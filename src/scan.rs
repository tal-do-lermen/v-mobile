//! src/scan.rs — ESCANEAMENTO PROGRESSIVO (feito para hardware fraco)
//!
//! AQUI VOCÊ APRENDE: traits como abstração de FONTE de dados, `thread::scope`
//! para paralelismo seguro e a regra de ouro de performance: decodificar MENOS.
//!
//! Fluxo do modo progressivo (padrão para arquivos):
//!
//!   FonteJanelas ──▶ janelas de 30 s (RAM ~2 MB, nunca o arquivo inteiro)
//!        │              ├─ WAV 16 kHz mono  → hound, puro Rust
//!        │              └─ qualquer outro    → PCM cru de um Read (ffmpeg/pipe)
//!        ▼
//!   VAD da janela ──▶ recortar_fala (só a FALA vai pro Vosk — silêncio não)
//!        ▼
//!   sessão.alimentar ──▶ texto parcial acumulado ──▶ classificar
//!        │
//!        └─ confiança ≥ limiar? PARA já (early exit)
//!
//! Com `--threads ≥ 2`, um BATCH de janelas é decodificado em paralelo
//! (thread::scope, 1 sessão por janela, textos juntados em ordem).

use crate::audio::AudioData;
use crate::classify::{self, Decisao};
use crate::error::{DetectorError, Resultado};
use crate::palavras_chave::TabelaPalavras;
use crate::transcribe::{SessaoTranscricao, Transcritor};
use crate::vad::{self, Segmento};
use std::io::Read;
use std::path::Path;
use std::time::Instant;

/// Taxa NATIVA do modelo Vosk (small-pt). Arquivos nesta taxa seguem o
/// caminho puro Rust (hound); nas outras, o pipeline reamostra via ffmpeg.
pub const TAXA_STREAM: u32 = 16000;

/// Falas separadas por menos que isto são juntas numa única alimentação
/// (evita "metralhar" o recognizer com micro-trechos de fala).
const MERGE_GAP_S: f32 = 0.3;

/// Parâmetros da varredura. `f32` em segundos fica legível na CLI.
#[derive(Debug, Clone, Copy)]
pub struct ConfigScan {
    /// Tamanho de cada janela de análise (padrão: 30 s).
    pub janela_seg: f32,
    /// Confiança mínima para PARAR e declarar venda (padrão: 0.70).
    pub limiar_confianca: f32,
    /// Sensibilidade do VAD (o mesmo `--limiar` do modo clássico).
    pub limiar_vad: f32,
}

/// Resultado da varredura — o pipeline transforma isto num `Relatorio`.
#[derive(Debug)]
pub struct ResultadoScan {
    /// Veredito final (com o limiar do progressivo aplicado).
    pub decisao: Decisao,
    /// Texto transcrito até o momento da parada (ou do fim do áudio).
    pub transcricao: String,
    /// Segmentos de fala acumulados de todas as janelas analisadas.
    pub segmentos: Vec<Segmento>,
    /// Some(seg) = parada ANTECIPADA: venda confirmada neste instante.
    pub detectado_em_seg: Option<f32>,
    /// Quantas janelas foram analisadas (checkpoints feitos).
    pub janelas_analisadas: usize,
    /// Amostras que realmente passaram pelo pipeline até a parada.
    pub amostras_processadas: usize,
    /// Amostras totais conhecidas (arquivo) ou recebidas (stream).
    pub total_amostras: usize,
    pub taxa_amostragem: u32,
    /// Duração de áudio total conhecida (arquivo) ou recebida (stream).
    pub duracao_total_seg: f32,
}

impl ResultadoScan {
    /// Fração do áudio que foi efetivamente processada, em 0..100.
    pub fn percentual_processado(&self) -> f32 {
        if self.total_amostras == 0 {
            return 0.0;
        }
        ((self.amostras_processadas as f32 / self.total_amostras as f32) * 100.0).min(100.0)
    }
}

// ------------------------------------------------------------------
// FONTES DE JANELAS — abstração de "de onde vêm os pedaços de áudio".
// Duas implementações, um só consumidor (os escaneadores).
// ------------------------------------------------------------------

/// Uma janela de áudio já materializada em RAM (~2 MB para 30 s @16 kHz).
#[derive(Debug)]
pub struct Janela {
    /// Índice sequencial (0, 1, 2...).
    pub indice: usize,
    /// Começo desta janela no áudio TOTAL, em segundos.
    pub offset_seg: f32,
    /// Amostras f32 em [-1, 1].
    pub amostras: Vec<f32>,
}

/// O "contrato" de qualquer fonte de janelas.
pub trait FonteJanelas {
    /// Próxima janela (None = fonte esgotada). A última pode ser menor.
    fn proxima(&mut self) -> Resultado<Option<Janela>>;

    /// Taxa de amostragem desta fonte.
    fn taxa(&self) -> u32;

    /// Total de amostras SE for conhecível de antemão (WAV tem cabeçalho;
    /// uma stream/pipe não — ninguém sabe o que ainda vai chegar).
    fn total_conhecido(&self) -> Option<usize>;
}

/// FONTE 1: arquivo WAV 16 kHz mono PCM16 — streaming puro Rust.
/// O hound entrega amostras sob demanda; mantemos só uma janela em RAM.
pub struct FonteWav {
    iter: hound::WavIntoSamples<std::io::BufReader<std::fs::File>, i16>,
    janela_amostras: usize,
    taxa: u32,
    total: usize,
    indice: usize,
}

impl FonteWav {
    /// Abre o arquivo e valida o formato (mono, PCM 16 bits).
    pub fn abrir(caminho: &Path, janela_seg: f32) -> Resultado<Self> {
        let leitor = hound::WavReader::open(caminho)?;
        let spec = leitor.spec();
        if spec.channels != 1 {
            return Err(DetectorError::FormatoNaoSuportado {
                detalhe: format!("esperado 1 canal (mono), encontrado {}", spec.channels),
            });
        }
        if spec.bits_per_sample != 16 || spec.sample_format != hound::SampleFormat::Int {
            return Err(DetectorError::FormatoNaoSuportado {
                detalhe: format!(
                    "esperado PCM 16 bits, encontrado {} bits ({:?})",
                    spec.bits_per_sample, spec.sample_format
                ),
            });
        }
        let taxa = spec.sample_rate;
        let janela_amostras = (janela_seg * taxa as f32).max(1.0) as usize;
        // `duration()` = nº de frames do cabeçalho (= amostras no mono).
        let total = leitor.duration() as usize;
        let iter = leitor.into_samples::<i16>();
        Ok(Self { iter, janela_amostras, taxa, total, indice: 0 })
    }
}

impl FonteJanelas for FonteWav {
    fn proxima(&mut self) -> Resultado<Option<Janela>> {
        let offset_seg = (self.indice * self.janela_amostras) as f32 / self.taxa as f32;
        // Puxa até UMA janela de amostras do iterador; para no EOF (None).
        let mut fatia: Vec<f32> = Vec::with_capacity(self.janela_amostras);
        for _ in 0..self.janela_amostras {
            match self.iter.next() {
                Some(Ok(s)) => fatia.push(s as f32 / 32768.0),
                Some(Err(e)) => return Err(e.into()),
                None => break,
            }
        }
        if fatia.is_empty() {
            return Ok(None);
        }
        self.indice += 1;
        Ok(Some(Janela { indice: self.indice - 1, offset_seg, amostras: fatia }))
    }

    fn taxa(&self) -> u32 {
        self.taxa
    }

    fn total_conhecido(&self) -> Option<usize> {
        Some(self.total)
    }
}

/// FONTE 2: PCM cru (i16 LE) de QUALQUER `Read` — pipe do ffmpeg, stdin...
/// Genérico sobre `R: Read`: nos testes, um simples `&[u8]` faz o papel
/// de pipe (sem processo nenhum).
pub struct FontePcm<R: Read> {
    fonte: R,
    janela_amostras: usize,
    taxa: u32,
    indice: usize,
}

impl<R: Read> FontePcm<R> {
    pub fn novo(fonte: R, janela_seg: f32, taxa: u32) -> Self {
        Self {
            fonte,
            janela_amostras: (janela_seg * taxa as f32).max(1.0) as usize,
            taxa,
            indice: 0,
        }
    }
}

impl<R: Read> FonteJanelas for FontePcm<R> {
    fn proxima(&mut self) -> Resultado<Option<Janela>> {
        let offset_seg = (self.indice * self.janela_amostras) as f32 / self.taxa as f32;
        let brutos = ler_pcm_i16(&mut self.fonte, self.janela_amostras)?;
        if brutos.is_empty() {
            return Ok(None);
        }
        self.indice += 1;
        Ok(Some(Janela {
            indice: self.indice - 1,
            offset_seg,
            amostras: brutos.iter().map(|&s| s as f32 / 32768.0).collect(),
        }))
    }

    fn taxa(&self) -> u32 {
        self.taxa
    }

    fn total_conhecido(&self) -> Option<usize> {
        None // pipe: o futuro é desconhecido
    }
}

// ------------------------------------------------------------------
// VAD-GATING — a otimização nº 1 para hardware fraco: o silêncio
// NUNCA vai para o decoder pesado.
// ------------------------------------------------------------------

/// Recorta da fatia apenas os trechos COM fala (segmentos do VAD, que já
/// vêm com padding de 0,1 s), juntando falas vizinhas (gap < 0,3 s).
/// Numa ligação típica isso corta ~40-60% das amostras decodificadas.
fn recortar_fala(fatia: &[f32], segmentos_rel: &[Segmento], taxa: u32) -> Vec<f32> {
    if segmentos_rel.is_empty() {
        return Vec::new();
    }
    // Junta segmentos vizinhos: se o gap entre o fim de um e o início do
    // outro é menor que MERGE_GAP_S, viram um trecho só.
    let mut trechos: Vec<(f32, f32)> = Vec::new();
    for s in segmentos_rel {
        match trechos.last_mut() {
            Some(ultimo) if s.inicio - ultimo.1 < MERGE_GAP_S => ultimo.1 = s.fim,
            _ => trechos.push((s.inicio, s.fim)),
        }
    }
    // Converte segundos → índices (com trava nos limites) e copia.
    let mut saida = Vec::new();
    for (ini, fim) in trechos {
        let a = ((ini * taxa as f32) as usize).min(fatia.len());
        let b = ((fim * taxa as f32) as usize).min(fatia.len());
        saida.extend_from_slice(&fatia[a..b]);
    }
    saida
}

// ------------------------------------------------------------------
// ESCANEADORES — sequential (padrão) e paralelo (--threads ≥ 2)
// ------------------------------------------------------------------

/// Um CHECKPOINT da varredura sequencial: VAD + recorte + alimentar +
/// classificar. Devolve `Some(decisao)` quando já pode PARAR.
#[allow(clippy::too_many_arguments)]
fn checkpoint(
    sessao: &mut (dyn SessaoTranscricao + Send + '_),
    cfg: &ConfigScan,
    tabela: &TabelaPalavras,
    fatia: &[f32],
    taxa: u32,
    offset_seg: f32,
    segmentos: &mut Vec<Segmento>,
    janelas: &mut usize,
) -> Resultado<Option<Decisao>> {
    *janelas += 1;

    // VAD na fatia nova (barato) + registro com offset do áudio completo.
    let segs_rel = vad::detectar_fala_slice(fatia, taxa, cfg.limiar_vad);
    let fala = recortar_fala(fatia, &segs_rel, taxa);
    for mut s in segs_rel {
        s.inicio += offset_seg;
        s.fim += offset_seg;
        segmentos.push(s);
    }

    // Só a FALA alimenta o decoder pesado (silêncio de graça).
    sessao.alimentar(&fala)?;

    // Classifica o texto ACUMULADO — a janela 2 já vê o texto da 1.
    let texto = sessao.texto_parcial();
    let decisao = classify::classificar_com_tabela(&texto, tabela, cfg.limiar_confianca);
    tracing::debug!(
        janela = *janelas,
        confianca = format!("{:.2}", decisao.confianca),
        e_venda = decisao.e_venda,
        "checkpoint do modo progressivo"
    );

    if decisao.e_venda {
        Ok(Some(decisao)) // ✋ PARADA ANTECIPADA
    } else {
        Ok(None) // ainda não: expande a janela e continua
    }
}

/// Log de progresso/ETA — essencial quando o hardware é mais lento que
/// o tempo real (sem isto, parece travado).
fn log_progresso(janelas: usize, processadas: usize, taxa: u32, total: Option<usize>, inicio: Instant) {
    let audio_s = processadas as f32 / taxa as f32;
    let decorrido = inicio.elapsed().as_secs_f32();
    let vel = if decorrido > 0.001 { audio_s / decorrido } else { 0.0 };
    match total {
        Some(t) if t > processadas => {
            let eta_s = ((t - processadas) as f32 / taxa as f32) / vel.max(0.001);
            tracing::info!(
                janelas,
                audio = format!("{:.0}s", audio_s),
                velocidade = format!("{:.1}x realtime", vel),
                eta = format!("{:.0}s", eta_s),
                "progresso"
            );
        }
        _ => tracing::info!(
            janelas,
            audio = format!("{:.0}s", audio_s),
            velocidade = format!("{:.1}x realtime", vel),
            "progresso"
        ),
    }
}

/// Varredura SEQUENCIAL — uma única sessão de transcrição acumulada.
/// Menor RAM, texto com continuidade total, ideal para streams ao vivo
/// e para hardware de 1-2 núcleos.
pub fn escanear_sequencial(
    fonte: &mut dyn FonteJanelas,
    transcritor: &mut dyn Transcritor,
    cfg: &ConfigScan,
    tabela: &TabelaPalavras,
) -> Resultado<ResultadoScan> {
    let taxa = fonte.taxa();
    let total = fonte.total_conhecido();
    let mut sessao = transcritor.iniciar_sessao(taxa)?;
    let mut segmentos = Vec::new();
    let mut janelas = 0usize;
    let mut processadas = 0usize;
    let inicio = Instant::now();

    while let Some(janela) = fonte.proxima()? {
        if let Some(decisao) = checkpoint(
            sessao.as_mut(),
            cfg,
            tabela,
            &janela.amostras,
            taxa,
            janela.offset_seg,
            &mut segmentos,
            &mut janelas,
        )? {
            let detectado_em = janela.offset_seg + janela.amostras.len() as f32 / taxa as f32;
            tracing::info!(detectado_em_s = detectado_em, janelas = janelas, "venda detectada");
            return Ok(monta(
                decisao,
                sessao.texto_parcial(),
                segmentos,
                Some(detectado_em),
                janelas,
                processadas + janela.amostras.len(),
                total.unwrap_or(processadas + janela.amostras.len()),
                taxa,
            ));
        }
        processadas += janela.amostras.len();
        log_progresso(janelas, processadas, taxa, total, inicio);
    }

    // Áudio acabou sem confirmar venda: fecha a sessão e decide com tudo.
    let transcricao = sessao.finalizar()?;
    let decisao = classify::classificar_com_tabela(&transcricao, tabela, cfg.limiar_confianca);
    Ok(monta(
        decisao,
        transcricao,
        segmentos,
        None,
        janelas,
        processadas,
        total.unwrap_or(processadas),
        taxa,
    ))
}

/// Junta os textos das janelas EM ORDEM, sem deixar espaços órfãos quando
/// uma janela não produziu texto nenhum (silêncio puro).
fn junta_textos(textos: &[String]) -> String {
    let partes: Vec<&str> = textos.iter().map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
    partes.join(" ")
}

/// Varredura PARALELA (--threads ≥ 2): decodifica um BATCH de janelas ao
/// mesmo tempo, cada uma com a PRÓPRIA sessão (o modelo é compartilhado —
/// `Model: Sync` sancionado pelo crate), junta os textos EM ORDEM e faz o
/// checkpoint a cada batch. Granularidade do early exit = tamanho do batch.
pub fn escanear_paralelo(
    fonte: &mut dyn FonteJanelas,
    transcritor: &dyn Transcritor,
    cfg: &ConfigScan,
    tabela: &TabelaPalavras,
    threads: usize,
) -> Resultado<ResultadoScan> {
    let taxa = fonte.taxa();
    let total = fonte.total_conhecido();
    let mut textos: Vec<String> = Vec::new();
    let mut segmentos = Vec::new();
    let mut janelas = 0usize;
    let mut processadas = 0usize;
    let inicio = Instant::now();

    loop {
        // 1) Coleta um batch de até `threads` janelas da fonte.
        let mut batch: Vec<Janela> = Vec::with_capacity(threads);
        for _ in 0..threads {
            match fonte.proxima()? {
                Some(j) => batch.push(j),
                None => break,
            }
        }
        if batch.is_empty() {
            break;
        }

        // 2) VAD + recorte no thread PRINCIPAL (barato — o thread-pool só
        //    recebe a fala útil, e os segmentos já saem com offset).
        let preparo: Vec<(Vec<f32>, Vec<Segmento>)> = batch
            .iter()
            .map(|j| {
                let segs_rel = vad::detectar_fala_slice(&j.amostras, taxa, cfg.limiar_vad);
                let fala = recortar_fala(&j.amostras, &segs_rel, taxa);
                let segs_com_offset: Vec<Segmento> = segs_rel
                    .iter()
                    .map(|s| Segmento { inicio: s.inicio + j.offset_seg, fim: s.fim + j.offset_seg })
                    .collect();
                (fala, segs_com_offset)
            })
            .collect();
        for (_, segs) in &preparo {
            segmentos.extend(segs.iter().cloned());
        }

        // 3) Decode PARALELO: `thread::scope` empresta o transcritor para
        //    as threads sem 'static — escopo acaba, threads acabam.
        let textos_batch: Vec<String> = std::thread::scope(|escopo| {
            let alcas: Vec<_> = preparo
                .into_iter()
                .map(|(fala, _)| {
                    // `move` leva a fala (dona) e o &transcritor (cópia da
                    // referência — é Send porque Transcritor: Sync).
                    let tr = transcritor;
                    escopo.spawn(move || {
                        let mut sessao = tr.iniciar_sessao_enviada(taxa)?;
                        sessao.alimentar(&fala)?;
                        sessao.finalizar()
                    })
                })
                .collect();
            alcas
                .into_iter()
                .map(|a| {
                    a.join().unwrap_or_else(|_| {
                        Err(DetectorError::Transcricao("thread de decodificação quebrou".into()))
                    })
                })
                .collect::<Resultado<Vec<String>>>()
        })?;
        textos.extend(textos_batch);

        // 4) Checkpoint do batch: junta os textos EM ORDEM e classifica.
        janelas += batch.len();
        processadas += batch.iter().map(|j| j.amostras.len()).sum::<usize>();
        let texto_acumulado = junta_textos(&textos);
        let decisao =
            classify::classificar_com_tabela(&texto_acumulado, tabela, cfg.limiar_confianca);
        log_progresso(janelas, processadas, taxa, total, inicio);

        let ultima = batch.last().expect("batch não vazio");
        let fim_ultimo = ultima.offset_seg + ultima.amostras.len() as f32 / taxa as f32;
        if decisao.e_venda {
            tracing::info!(detectado_em_s = fim_ultimo, janelas = janelas, "venda detectada");
            return Ok(monta(
                decisao,
                texto_acumulado,
                segmentos,
                Some(fim_ultimo),
                janelas,
                processadas,
                total.unwrap_or(processadas),
                taxa,
            ));
        }
    }

    let texto = junta_textos(&textos);
    let decisao = classify::classificar_com_tabela(&texto, tabela, cfg.limiar_confianca);
    Ok(monta(
        decisao,
        texto,
        segmentos,
        None,
        janelas,
        processadas,
        total.unwrap_or(processadas),
        taxa,
    ))
}

/// Monta o `ResultadoScan` calculando a duração total conforme a fonte
/// (arquivo sabe a duração de antemão; stream, só o que já chegou).
#[allow(clippy::too_many_arguments)]
fn monta(
    decisao: Decisao,
    transcricao: String,
    segmentos: Vec<Segmento>,
    detectado_em_seg: Option<f32>,
    janelas: usize,
    processadas: usize,
    total: usize,
    taxa: u32,
) -> ResultadoScan {
    let duracao_total_seg = total as f32 / taxa as f32;
    ResultadoScan {
        decisao,
        transcricao,
        segmentos,
        detectado_em_seg,
        janelas_analisadas: janelas,
        amostras_processadas: processadas,
        total_amostras: total,
        taxa_amostragem: taxa,
        duracao_total_seg,
    }
}

/// Lê ATÉ `n` amostras i16 little-endian do leitor (menos só no EOF).
fn ler_pcm_i16<R: Read>(fonte: &mut R, n: usize) -> Resultado<Vec<i16>> {
    let mut brutos = vec![0u8; n * 2];
    let mut lidos = 0usize;
    while lidos < brutos.len() {
        match fonte.read(&mut brutos[lidos..]) {
            Ok(0) => break, // EOF
            Ok(m) => lidos += m,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.into()),
        }
    }
    let inteiras = lidos - (lidos % 2);
    Ok(brutos[..inteiras]
        .chunks(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]))
        .collect())
}

/// Mantida para o modo CLÁSSICO (`--completo`), que opera sobre o áudio
/// inteiro já carregado em RAM. O progressivo usa as fontes streaming.
pub fn escanear_audio_memoria(
    audio: &AudioData,
    transcritor: &mut dyn Transcritor,
    cfg: &ConfigScan,
    tabela: &TabelaPalavras,
) -> Resultado<ResultadoScan> {
    let mut fonte = AudioMemoria { audio, pos: 0, janela: (cfg.janela_seg * audio.taxa_amostragem as f32).max(1.0) as usize };
    escanear_sequencial(&mut fonte, transcritor, cfg, tabela)
}

/// Adaptador: um `AudioData` já carregado disfarçado de `FonteJanelas`.
struct AudioMemoria<'a> {
    audio: &'a AudioData,
    pos: usize,
    janela: usize,
}

impl FonteJanelas for AudioMemoria<'_> {
    fn proxima(&mut self) -> Resultado<Option<Janela>> {
        if self.pos >= self.audio.amostras.len() {
            return Ok(None);
        }
        let fim = (self.pos + self.janela).min(self.audio.amostras.len());
        let offset_seg = self.pos as f32 / self.audio.taxa_amostragem as f32;
        let janela = Janela {
            indice: self.indice(),
            offset_seg,
            amostras: self.audio.amostras[self.pos..fim].to_vec(),
        };
        self.pos = fim;
        Ok(Some(janela))
    }
    fn taxa(&self) -> u32 {
        self.audio.taxa_amostragem
    }
    fn total_conhecido(&self) -> Option<usize> {
        Some(self.audio.amostras.len())
    }
}

impl AudioMemoria<'_> {
    fn indice(&self) -> usize {
        self.pos / self.janela
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcribe::MockTranscritor;

    /// Config de teste: janelas de 1 s (rápido de rodar).
    fn cfg() -> ConfigScan {
        ConfigScan { janela_seg: 1.0, limiar_confianca: 0.70, limiar_vad: 0.02 }
    }

    fn tabela() -> TabelaPalavras {
        TabelaPalavras::embutida()
    }

    /// PCM i16 LE de n_seg segundos de tom 440 Hz ("fala") ou silêncio.
    fn pcm(n_seg: f32, com_tom: bool) -> Vec<u8> {
        let n = (n_seg * TAXA_STREAM as f32) as usize;
        let mut v = Vec::with_capacity(n * 2);
        for i in 0..n {
            let t = i as f32 / TAXA_STREAM as f32;
            let s = if com_tom { (0.5 * (2.0 * std::f32::consts::PI * 440.0 * t).sin() * 32767.0) as i16 } else { 0 };
            v.extend_from_slice(&s.to_le_bytes());
        }
        v
    }

    #[test]
    fn pcm_detecta_na_primeira_janela_e_para() {
        let bytes = pcm(3.0, true);
        let mut fonte = FontePcm::novo(bytes.as_slice(), 1.0, TAXA_STREAM);
        let mut t = MockTranscritor;

        let r = escanear_sequencial(&mut fonte, &mut t, &cfg(), &tabela()).unwrap();

        assert!(r.decisao.e_venda);
        assert_eq!(r.detectado_em_seg, Some(1.0)); // parou em 1 s
        assert_eq!(r.janelas_analisadas, 1);
        // Parou de ler: só a 1ª janela das 3 disponíveis foi consumida.
        assert_eq!(r.amostras_processadas, TAXA_STREAM as usize);
    }

    #[test]
    fn pcm_silencio_varre_ate_o_fim_sem_venda() {
        let bytes = pcm(4.0, false);
        let mut fonte = FontePcm::novo(bytes.as_slice(), 1.0, TAXA_STREAM);
        let mut t = MockTranscritor;

        let r = escanear_sequencial(&mut fonte, &mut t, &cfg(), &tabela()).unwrap();

        assert!(!r.decisao.e_venda);
        assert_eq!(r.detectado_em_seg, None);
        assert_eq!(r.janelas_analisadas, 4);
        assert_eq!(r.transcricao, "");
        assert_eq!(r.segmentos.len(), 0);
    }

    #[test]
    fn fonte_wav_janelas_e_janela_final_parcial() {
        // WAV de 2,5 s em arquivo temporário → janelas [1s, 1s, 0.5s].
        let caminho = std::env::temp_dir().join("vd_scan_fonte_wav.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: TAXA_STREAM,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&caminho, spec).unwrap();
        for _ in 0..TAXA_STREAM as usize * 5 / 2 {
            w.write_sample(0i16).unwrap();
        }
        w.finalize().unwrap();

        let mut fonte = FonteWav::abrir(&caminho, 1.0).unwrap();
        assert_eq!(fonte.taxa(), TAXA_STREAM);
        assert_eq!(fonte.total_conhecido(), Some(TAXA_STREAM as usize * 5 / 2));

        let mut tamanhos = Vec::new();
        while let Some(j) = fonte.proxima().unwrap() {
            tamanhos.push(j.amostras.len());
        }
        assert_eq!(tamanhos, vec![TAXA_STREAM as usize, TAXA_STREAM as usize, TAXA_STREAM as usize / 2]);
    }

    #[test]
    fn wav_silencio_varre_tudo_com_percentual_real() {
        let caminho = std::env::temp_dir().join("vd_scan_wav_silencio.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: TAXA_STREAM,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&caminho, spec).unwrap();
        for _ in 0..TAXA_STREAM as usize * 4 {
            w.write_sample(0i16).unwrap();
        }
        w.finalize().unwrap();

        let mut fonte = FonteWav::abrir(&caminho, 1.0).unwrap();
        let mut t = MockTranscritor;
        let r = escanear_sequencial(&mut fonte, &mut t, &cfg(), &tabela()).unwrap();

        assert!(!r.decisao.e_venda);
        assert_eq!(r.janelas_analisadas, 4);
        // Fonte WAV conhece o total → percentual e duração são REAIS.
        assert_eq!(r.total_amostras, TAXA_STREAM as usize * 4);
        assert_eq!(r.duracao_total_seg, 4.0);
        assert_eq!(r.percentual_processado(), 100.0);
    }

    #[test]
    fn recortar_fala_corta_o_silencio() {
        // 0,5 s de silêncio + 1 s de tom: o recorte deve devolver bem menos
        // que a fatia inteira, mas preservar a fala inteira (com padding).
        let taxa = TAXA_STREAM;
        let mut fatia = vec![0.0; taxa as usize / 2];
        for i in 0..taxa as usize {
            let t = i as f32 / taxa as f32;
            fatia.push(0.5 * (2.0 * std::f32::consts::PI * 440.0 * t).sin());
        }
        let segs = vad::detectar_fala_slice(&fatia, taxa, 0.02);
        let recorte = recortar_fala(&fatia, &segs, taxa);

        assert!(!recorte.is_empty());
        assert!(recorte.len() < fatia.len(), "silêncio inicial deveria ser cortado");
        assert!(recorte.len() > taxa as usize, "fala inteira (com padding) deveria ficar");
        // E o recorte tem energia: é fala, não silêncio.
        assert!(recorte.iter().any(|&a| a.abs() > 0.1));
    }

    #[test]
    fn paralelo_detecta_e_para_no_batch() {
        // threads=2 → o 1º batch decodifica as janelas 1 E 2; o texto
        // acumulado cruza o limiar → para APÓS 2 janelas (2 s), não na 1ª.
        let bytes = pcm(3.0, true);
        let mut fonte = FontePcm::novo(bytes.as_slice(), 1.0, TAXA_STREAM);
        let t = MockTranscritor;

        let r = escanear_paralelo(&mut fonte, &t, &cfg(), &tabela(), 2).unwrap();

        assert!(r.decisao.e_venda);
        assert_eq!(r.janelas_analisadas, 2);
        assert_eq!(r.detectado_em_seg, Some(2.0));
        assert_eq!(r.amostras_processadas, TAXA_STREAM as usize * 2);
        // Textos das janelas juntados EM ORDEM (o mock repete o texto fixo).
        assert_eq!(r.transcricao.matches("fechar o pedido").count(), 2);
    }

    #[test]
    fn paralelo_sem_venda_varre_tudo() {
        // 5 janelas de silêncio com threads=2 → batches [2, 2, 1] = 5.
        let bytes = pcm(5.0, false);
        let mut fonte = FontePcm::novo(bytes.as_slice(), 1.0, TAXA_STREAM);
        let t = MockTranscritor;

        let r = escanear_paralelo(&mut fonte, &t, &cfg(), &tabela(), 2).unwrap();

        assert!(!r.decisao.e_venda);
        assert_eq!(r.janelas_analisadas, 5);
        assert_eq!(r.detectado_em_seg, None);
        assert_eq!(r.transcricao, "");
    }
}
