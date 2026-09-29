//! src/transcribe.rs
//!
//! AQUI VOCÊ APRENDE: TRAITS — o "coração" do polimorfismo em Rust.
//!
//! ESTÁGIO 3 do pipeline: transforma os segmentos de fala em TEXTO.
//!
//! A ideia: o pipeline NÃO sabe (nem quer saber) se a transcrição vem de um
//! mock ou de uma rede neural. Ele só exige "algo que saiba transcrever".
//! Essa exigência formal é a TRAIT — equivalente a uma interface em Java/TS,
//! mas implementável em tipos que você nem escreveu.

use crate::audio::AudioData;
use crate::error::{DetectorError, Resultado};
use crate::vad::Segmento;
use std::path::Path;

/// O "contrato" de qualquer transcritor.
///
/// - `&mut self`: transcrever pode alterar o estado interno do transcritor
///   (o Vosk mantém contexto de decodificação entre chunks), então precisa
///   de empréstimo MUTÁVEL — quem chama precisa ter a variável como `mut`.
/// - `dyn Transcritor` (visto no pipeline) = "qualquer tipo que implemente
///   esta trait", resolvido em tempo de execução (despacho dinâmico).
pub trait Transcritor: Sync {
    /// Recebe o áudio emprestado + os segmentos do VAD, devolve o texto ou erro.
    fn transcrever(&mut self, audio: &AudioData, segmentos: &[Segmento]) -> Resultado<String>;

    /// Nome curto para logs/relatório ("mock", "vosk", ...).
    fn nome(&self) -> &'static str;

    /// Abre uma sessão de transcrição streaming LOCALL.
    ///
    /// Implementação DEFAULT que delega para `iniciar_sessao_enviada` —
    /// um `Box<dyn SessaoTranscricao + Send>` coerce de graça para
    /// `Box<dyn SessaoTranscricao + '_>` (coerções podem "relaxar" auto
    /// traits e encurtar lifetimes). Ou seja: UMA implementação só serve
    /// os dois usos (local e multi-thread).
    fn iniciar_sessao(
        &mut self,
        taxa_amostragem: u32,
    ) -> Resultado<Box<dyn SessaoTranscricao + Send + '_>> {
        self.iniciar_sessao_enviada(taxa_amostragem)
    }

    /// Abre uma sessão que PODE VIAJAR para outra thread.
    ///
    /// CONCEITO — `Sync`/`Send` na prática: a trait agora exige
    /// `Transcritor: Sync` (o transcritor pode ser COMPARTILHADO por
    /// referência entre threads). O Vosk sanciona isso oficialmente: o
    /// crate faz `unsafe impl Sync for Model` — a doc diz que o mesmo
    /// modelo serve vários recognizers, até em threads diferentes — e
    /// `unsafe impl Send for Recognizer` (cada thread cria o seu). O mock,
    /// sem estado compartilhável, é Sync de graça.
    fn iniciar_sessao_enviada(
        &self,
        taxa_amostragem: u32,
    ) -> Resultado<Box<dyn SessaoTranscricao + Send>>;
}

/// O "contrato" de uma sessão de transcrição streaming.
///
/// Três operações: ALIMENTAR com mais amostras, consultas ao TEXTO PARCIAL
/// (o que foi transcrito até agora) e FINALIZAR (encerra o decode e devolve
/// o texto completo). O texto parcial é ACUMULADO: a janela 3 já contém o
/// texto das janelas 1 e 2 — por isso o classificador consegue decidir
/// "os 60s" sem reprocessar nada.
pub trait SessaoTranscricao {
    /// Entrega mais um pedaço de áudio (amostras f32 em [-1, 1]).
    fn alimentar(&mut self, amostras: &[f32]) -> Resultado<()>;

    /// Tudo o que foi transcrito até este momento.
    fn texto_parcial(&mut self) -> String;

    /// Encerra a sessão e devolve o texto final.
    fn finalizar(&mut self) -> Resultado<String>;
}

/// Texto fixo que o mock "transcreve" ao perceber qualquer fala.
/// Constante única: o `transcrever` antigo e a sessão nova usam a MESMA
/// string, então os dois modos têm comportamento idêntico.
const TEXTO_MOCK_VENDA: &str = "Bom dia! Gostaria de fechar o pedido hoje. Qual o preço com \
desconto? Pode ser no cartão em três parcelas?";

/// ------------------------------------------------------------------
/// TRANSCRITOR MOCK — versão de estudo: não usa IA, devolve um texto
/// fixo de uma ligação de venda típica. Permite o pipeline funcionar
/// de ponta a ponta sem baixar modelo nenhum.
/// ------------------------------------------------------------------
#[derive(Debug, Default)]
pub struct MockTranscritor;

impl Transcritor for MockTranscritor {
    fn transcrever(&mut self, _audio: &AudioData, segmentos: &[Segmento]) -> Resultado<String> {
        // Sem segmentos de fala → não há o que transcrever.
        if segmentos.is_empty() {
            return Ok(String::new());
        }
        // Texto fixo, cheio de palavras-chave de venda (o classificador
        // do estágio 4 vai pontuar alto nele).
        Ok(TEXTO_MOCK_VENDA.to_string())
    }

    fn nome(&self) -> &'static str {
        "mock"
    }

    fn iniciar_sessao_enviada(
        &self,
        _taxa_amostragem: u32,
    ) -> Resultado<Box<dyn SessaoTranscricao + Send>> {
        // `_taxa` com underline: o mock não se importa com a taxa, mas a
        // assinatura da trait exige o parâmetro (todos os transcritores
        // abrem sessão do mesmo jeito). MockSessao é `Send` de graça
        // (só guarda um bool).
        Ok(Box::new(MockSessao { tem_fala: false }))
    }
}

/// SESSÃO do mock — lembra de uma coisa só: já vimos energia de voz?
///
/// A checagem de pico (> 0.015) é um "mini-VAD" simplificado: nos testes,
/// silêncio puro não vira texto e qualquer tom vira o texto fixo de venda.
#[derive(Debug, Default)]
struct MockSessao {
    tem_fala: bool,
}

impl SessaoTranscricao for MockSessao {
    fn alimentar(&mut self, amostras: &[f32]) -> Resultado<()> {
        // `.any` para no primeiro elemento que satisfaça — não varre o resto.
        if amostras.iter().any(|&a| a.abs() > 0.015) {
            self.tem_fala = true;
        }
        Ok(())
    }

    fn texto_parcial(&mut self) -> String {
        if self.tem_fala {
            TEXTO_MOCK_VENDA.to_string()
        } else {
            String::new()
        }
    }

    fn finalizar(&mut self) -> Resultado<String> {
        Ok(self.texto_parcial())
    }
}

/// ------------------------------------------------------------------
/// FÁBRICA (factory): decide QUAL transcritor instanciar.
///
/// CONCEITO — objeto de trait: `Box<dyn Transcritor>` é um ponteiro no heap
/// para QUALQUER tipo que implemente a trait. Todas as variantes de retorno
/// têm tipos diferentes (MockTranscritor vs VoskTranscritor), mas o Box
/// apaga essa diferença — por isso a função consegue devolver `Ok(...)` nos
/// dois casos.
pub fn criar_transcritor(usar_vosk: bool, modelo: &Path) -> Resultado<Box<dyn Transcritor>> {
    // Sem a feature "vosk", o parâmetro `modelo` não é usado — o `let _ =`
    // informa isso ao compilador explicitamente (e ensina o truque).
    #[cfg(not(feature = "vosk"))]
    let _ = modelo;

    // #[cfg(...)] = compilação condicional. O código do if abaixo SÓ EXISTE
    // no binário se a feature "vosk" foi ativada no Cargo.toml.
    #[cfg(feature = "vosk")]
    if usar_vosk {
        return Ok(Box::new(VoskTranscritor::novo(modelo)?));
    }

    // Compilado SEM a feature, pedir --vosk é erro explícito e amigável.
    // (Este bloco é o que existe quando a feature está DESLIGADA.)
    #[cfg(not(feature = "vosk"))]
    if usar_vosk {
        return Err(DetectorError::VoskNaoCompilado);
    }

    tracing::debug!("usando transcritor mock");
    Ok(Box::new(MockTranscritor))
}

/// ------------------------------------------------------------------
/// TRANSCRITOR VOSK — transcrição REAL e offline com o modelo pt-BR,
/// via crate `vosk` (bindings seguros da biblioteca C libvosk.so).
/// Só compila com `--features vosk`.
///
/// Como funciona o Vosk, em uma frase: você vai "alimentando" (feeding)
/// o Recognizer com fatias de áudio PCM e ele devolve o texto acumulado.
/// É um decodificador STREAMING — pensado para rodar enquanto a pessoa fala.
/// ------------------------------------------------------------------
#[cfg(feature = "vosk")]
pub struct VoskTranscritor {
    /// O modelo carregado na memória (~300 MB de RAM com o small-pt).
    /// O Recognizer é criado por chamada a partir deste modelo.
    modelo: vosk::Model,
}

#[cfg(feature = "vosk")]
impl VoskTranscritor {
    pub fn novo(caminho: &Path) -> Resultado<Self> {
        // O modelo do Vosk é uma PASTA descompactada (am/, conf/, graph/...),
        // baixada por scripts/instalar_vosk.sh — diferente de um .bin único.
        if !caminho.exists() {
            return Err(DetectorError::ModeloNaoEncontrado(
                caminho.display().to_string(),
            ));
        }
        // Silencia os logs internos do Kaldi (o motor por trás do Vosk),
        // senão ele imprime direto no nosso terminal.
        vosk::set_log_level(vosk::LogLevel::Error);

        // Model::new recebe algo conversível em String (impl Into<String>)
        // e devolve Option — None se falhar. `ok_or_else` converte
        // Option em Result usando uma closure que cria nosso erro.
        let modelo = vosk::Model::new(caminho.display().to_string()).ok_or_else(|| {
            DetectorError::Transcricao(format!(
                "não foi possível carregar o modelo em {}",
                caminho.display()
            ))
        })?;
        Ok(Self { modelo })
    }
}

#[cfg(feature = "vosk")]
impl Transcritor for VoskTranscritor {
    fn transcrever(&mut self, audio: &AudioData, segmentos: &[Segmento]) -> Resultado<String> {
        if segmentos.is_empty() {
            return Ok(String::new());
        }

        // Um Recognizer por chamada: recebe o modelo EMPRESTADO (&) + a taxa
        // de amostragem do nosso áudio (16000.0).
        let mut rec = vosk::Recognizer::new(&self.modelo, audio.taxa_amostragem as f32)
            .ok_or_else(|| DetectorError::Transcricao("falha ao criar Recognizer".to_string()))?;
        rec.set_words(false); // não precisamos dos metadados por palavra

        // Vosk consome PCM de 16 bits (&[i16]), mas nosso áudio interno é
        // f32 em [-1, 1] — o caminho de VOLTA da conversão feita em audio.rs.
        // Alimentamos em blocos de 8000 amostras (0,5 s a 16 kHz), simulando
        // o streaming de um microfone.
        for janela in audio.amostras.chunks(8000) {
            let pcm: Vec<i16> = janela.iter().map(|&a| (a * 32767.0) as i16).collect();
            // `.map_err` + `?`: accept_waveform devolve Result<_, AcceptWaveformError>
            // e nosso erro é DetectorError — convertemos manualmente.
            rec.accept_waveform(&pcm)
                .map_err(|e| DetectorError::Transcricao(e.to_string()))?;
        }

        // final_result() encerra o decode: internamente ele "flusha" o áudio
        // restante e devolve o texto completo. É um enum com 2 formas
        // (uma alternativa ou várias) — o match escolhe a certa.
        let texto = match rec.final_result() {
            vosk::CompleteResult::Single(unica) => unica.text.to_string(),
            vosk::CompleteResult::Multiple(multiplas) => multiplas
                .alternatives
                .first()                       // Option<&Alternative>
                .map(|alt| alt.text.to_string()) // se existe, pega o texto
                .unwrap_or_default(),          // se não, string vazia
        };

        Ok(texto.trim().to_string())
    }

    fn nome(&self) -> &'static str {
        "vosk"
    }

    fn iniciar_sessao_enviada(
        &self,
        taxa_amostragem: u32,
    ) -> Resultado<Box<dyn SessaoTranscricao + Send>> {
        // Cada sessão cria seu PRÓPRIO Recognizer a partir do modelo
        // compartilhado (&self — threads diferentes, mesmo Model). O
        // Recognizer é `Send` (unsafe impl do crate), então a caixa pode
        // viajar para a thread que for decodificar.
        let mut rec = vosk::Recognizer::new(&self.modelo, taxa_amostragem as f32)
            .ok_or_else(|| DetectorError::Transcricao("falha ao criar Recognizer".to_string()))?;
        rec.set_words(false); // não precisamos dos metadados por palavra
        Ok(Box::new(VoskSessao { rec }))
    }
}

/// SESSÃO do Vosk — o Recognizer acumula o contexto de decodificação entre
/// chamadas, então alimentar aos poucos é exatamente como ouvir ao vivo.
///
/// CONCEITO — por que SEM lifetime? O `vosk::Recognizer` guarda apenas um
/// ponteiro para o reconhecedor C (`NonNull<VoskRecognizer>`); o borrow do
/// modelo morre dentro de `Recognizer::new` e a biblioteca C resolve o
/// compartilhamento por conta própria (o doc do crate diz que o mesmo
/// Model pode servir vários recognizers). No nosso desenho isso é seguro
/// do mesmo jeito: a sessão vive presa ao `&mut` do transcritor, então o
/// `Model` dentro do `VoskTranscritor` sempre sobrevive à sessão.
#[cfg(feature = "vosk")]
struct VoskSessao {
    rec: vosk::Recognizer,
}

#[cfg(feature = "vosk")]
impl SessaoTranscricao for VoskSessao {
    fn alimentar(&mut self, amostras: &[f32]) -> Resultado<()> {
        // Vosk consome PCM de 16 bits (&[i16]); convertemos de f32 [-1, 1]
        // e alimentamos em blocos de 8000 amostras (0,5 s a 16 kHz).
        for janela in amostras.chunks(8000) {
            let pcm: Vec<i16> = janela.iter().map(|&a| (a * 32767.0) as i16).collect();
            self.rec
                .accept_waveform(&pcm)
                .map_err(|e| DetectorError::Transcricao(e.to_string()))?;
        }
        Ok(())
    }

    fn texto_parcial(&mut self) -> String {
        // partial_result() NÃO encerra o decode: devolve o texto acumulado
        // até agora e a sessão continua aceitando mais áudio depois.
        self.rec.partial_result().partial.trim().to_string()
    }

    fn finalizar(&mut self) -> Resultado<String> {
        // final_result() encerra o decode: internamente ele "flusha" o
        // áudio restante e devolve o texto completo. É um enum com 2 formas
        // (uma alternativa ou várias) — o match escolhe a certa.
        let texto = match self.rec.final_result() {
            vosk::CompleteResult::Single(unica) => unica.text.to_string(),
            vosk::CompleteResult::Multiple(multiplas) => multiplas
                .alternatives
                .first()                       // Option<&Alternative>
                .map(|alt| alt.text.to_string()) // se existe, pega o texto
                .unwrap_or_default(),          // se não, string vazia
        };
        Ok(texto.trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_transcreve_texto_de_venda() {
        let mut t = MockTranscritor;
        let audio = AudioData { amostras: vec![0.0; 1600], taxa_amostragem: 16000 };
        let segs = vec![Segmento { inicio: 0.0, fim: 0.1 }];
        let texto = t.transcrever(&audio, &segs).unwrap();
        assert!(texto.contains("fechar o pedido"));
    }

    #[test]
    fn mock_sem_segmentos_devolve_vazio() {
        let mut t = MockTranscritor;
        let audio = AudioData { amostras: vec![0.0; 1600], taxa_amostragem: 16000 };
        assert_eq!(t.transcrever(&audio, &[]).unwrap(), "");
    }

    #[test]
    fn sessao_mock_só_fala_apos_energia() {
        let mut t = MockTranscritor;
        // A sessão começa muda: silêncio alimentado NÃO vira texto...
        let mut s = t.iniciar_sessao(16000).unwrap();
        assert_eq!(s.texto_parcial(), "");
        s.alimentar(&[0.0; 1600]).unwrap();
        assert_eq!(s.texto_parcial(), "");
        // ...mas a primeira fatia com energia destrava o texto de venda.
        s.alimentar(&[0.5; 1600]).unwrap();
        assert!(s.texto_parcial().contains("fechar o pedido"));
        // E o finalizador devolve o texto completo acumulado.
        assert!(s.finalizar().unwrap().contains("fechar o pedido"));
    }

    #[test]
    fn factory_sem_feature_erro_claro() {
        // Caminho que NÃO existe — assim este teste nunca carrega o modelo
        // real (300 MB) quando roda com --features vosk.
        let r = criar_transcritor(true, Path::new("models/asr/nao-existe"));
        #[cfg(not(feature = "vosk"))]
        assert!(matches!(r, Err(DetectorError::VoskNaoCompilado)));
        // Com a feature ligada, o erro muda: a pasta do modelo não existe.
        #[cfg(feature = "vosk")]
        assert!(matches!(r, Err(DetectorError::ModeloNaoEncontrado(_))));
    }

    /// Teste REAL do Vosk (só roda com --features vosk e modelo instalado):
    /// silêncio puro não deve gerar texto nenhum. Se o modelo não estiver
    /// baixado, o teste apenas passa sem testar nada (guarda do início).
    #[cfg(feature = "vosk")]
    #[test]
    fn vosk_silencio_transcreve_vazio() {
        let caminho = Path::new("models/asr/vosk-model-small-pt-0.3");
        if !caminho.exists() {
            return; // modelo não baixado: nada a testar aqui
        }
        let mut t = VoskTranscritor::novo(caminho).unwrap();
        let audio = AudioData { amostras: vec![0.0; 16000], taxa_amostragem: 16000 };
        let segs = vec![Segmento { inicio: 0.0, fim: 1.0 }];
        assert_eq!(t.transcrever(&audio, &segs).unwrap(), "");
    }
}
