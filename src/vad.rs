//! src/vad.rs
//!
//! AQUI VOCÊ APRENDE: iteradores, slices, `Option` e algoritmo sem depender
//! de biblioteca nenhuma — Rust puro.
//!
//! VAD = Voice Activity Detection (detecção de atividade de voz).
//! ESTÁGIO 2 do pipeline: olha o áudio em janelas de 30 ms, mede a ENERGIA
//! de cada janela (RMS) e marca como "fala" as que passam do limiar.
//! Resultado: cortamos o silêncio antes de transcrever (mais rápido e mais barato).

use crate::audio::AudioData;
use serde::Serialize; // permite transformar Segmento em JSON (usado no relatório)

/// Um trecho contínuo de fala, em segundos: [inicio, fim].
///
/// `Serialize` (do serde) vai gerar código para virar JSON — a mesma struct
/// que o pipeline usa internamente aparece no relatório final.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Segmento {
    pub inicio: f32,
    pub fim: f32,
}

impl Segmento {
    pub fn duracao(&self) -> f32 {
        self.fim - self.inicio
    }
}

/// Duração de cada janela de análise, em milissegundos.
/// 30 ms é o padrão clássico de VADs reais (WebRTC usa isso).
const DURACAO_JANELA_MS: usize = 30;

/// Margem (em segundos) acrescentada ao redor de cada segmento, para não
/// cortar o começo/fim das palavras.
const PADDING_S: f32 = 0.1;

/// Calcula a energia RMS (root mean square) de uma janela de amostras.
///
/// CONCEITO — slice: `&[f32]` é uma "visão" emprestada de um `Vec<f32>`.
/// Não copia nada e não pode ser modificado — só leitura, sem custo.
///
/// RMS = raiz da média dos quadrados. Silêncio → perto de 0; voz → valor alto.
fn rms(janela: &[f32]) -> f32 {
    if janela.is_empty() {
        return 0.0;
    }
    // .iter()     → pega um iterador de &f32 (referências)
    // .map(|&a|…) → o padrão `&a` DESESTRUTURA a referência e copia o f32
    // .sum()      → soma tudo
    let soma_quadrados: f32 = janela.iter().map(|&a| a * a).sum();
    (soma_quadrados / janela.len() as f32).sqrt()
}

/// Percorre o áudio inteiro e devolve os segmentos de fala encontrados.
///
/// Repare na assinatura: recebe `&AudioData` (empréstimo — quem chama continua
/// dono do áudio) e DEVOLVE um `Vec<Segmento>` novo (ownership transferido
/// para quem chamou). Este é o fluxo típico de dados em Rust: funções recebem
/// empréstimos e retornam donos.
pub fn detectar_fala(audio: &AudioData, limiar: f32) -> Vec<Segmento> {
    // Refactor SEM mudança de comportamento: o trabalho real mora na versão
    // "slice" abaixo; aqui só extraímos os dados que ela precisa do struct.
    detectar_fala_slice(&audio.amostras, audio.taxa_amostragem, limiar)
}

/// Analisa UMA FATIA de amostras — é a versão que o escaneamento progressivo
/// (scan.rs) usa a cada janela de 30 s, sem reprocessar o áudio já visto.
///
/// Os tempos dos segmentos são RELATIVOS ao começo da fatia: quem quiser a
/// posição no áudio completo soma o offset (ex.: começo da janela em segundos).
pub fn detectar_fala_slice(amostras: &[f32], taxa_amostragem: u32, limiar: f32) -> Vec<Segmento> {
    // Amostras por janela: 30 ms em amostras. Ex.: 16000 Hz * 0.030 s = 480.
    // `.max(1)` evita divisão por zero se a taxa fosse absurda.
    let taxa = taxa_amostragem as usize;
    let amostras_por_janela = (taxa * DURACAO_JANELA_MS / 1000).max(1);
    let duracao_janela_s = DURACAO_JANELA_MS as f32 / 1000.0;

    // PASSO 1: marca o índice de cada janela cuja energia passou do limiar.
    //
    // CONCEITO — `chunks`: divide o slice em fatias consecutivas de N itens
    // (a última pode ser menor). `.enumerate()` num iterador entrega
    // (índice, elemento) — como um for com contador.
    let janelas_com_voz: Vec<usize> = amostras
        .chunks(amostras_por_janela)
        .enumerate()
        // `.filter_map` mantém só o que devolve Some — aqui transformamos
        // (i, janela) em Some(i) se a janela tem voz, e descartamos o resto.
        .filter_map(|(i, janela)| {
            if rms(janela) > limiar {
                Some(i)
            } else {
                None
            }
        })
        .collect();

    // PASSO 2: junta janelas CONSECUTIVAS em grupos → cada grupo vira um segmento.
    // Ex.: janelas com voz [3, 4, 5, 9, 10] → grupos [[3,4,5], [9,10]].
    let mut grupos: Vec<Vec<usize>> = Vec::new();
    for &i in &janelas_com_voz {
        // `match` desestrutura a Option devolvida por `grupos.last_mut()`
        // (Some se já existe um grupo, None se a lista está vazia).
        match grupos.last_mut() {
            // `Some(g)` dá &mut Vec<usize> do último grupo; `g.last_mut()`
            // é Option<&mut usize> — se o último índice é exatamente i-1,
            // as janelas são vizinhas → adiciona i no mesmo grupo.
            Some(g) if g.last().copied() == Some(i - 1) => g.push(i),
            // Caso contrário (lista vazia, ou buraco de silêncio entre
            // janelas): começa um grupo novo.
            _ => grupos.push(vec![i]),
        }
    }

    // PASSO 3: converte índices de janela em segundos, aplica padding
    // e recorta para dentro da duração da fatia.
    let duracao = amostras.len() as f32 / taxa as f32;
    let mut segmentos = Vec::new();
    for grupo in &grupos {
        // grupo[0] e grupo[len-1]: primeiro e último índice do grupo.
        // `as f32` converte o índice para float antes da matemática.
        let mut inicio = grupo[0] as f32 * duracao_janela_s - PADDING_S;
        let mut fim = (grupo[grupo.len() - 1] as f32 + 1.0) * duracao_janela_s + PADDING_S;
        // `.max()/.min()` em f32 travam o segmento dentro dos limites [0, duração].
        inicio = inicio.max(0.0);
        fim = fim.min(duracao);
        segmentos.push(Segmento { inicio, fim });
    }

    tracing::debug!(segmentos = segmentos.len(), "VAD concluído");
    segmentos
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: monta um AudioData com o padrão silêncio|tom|silêncio.
    fn audio_padrao() -> AudioData {
        let taxa = 16000u32;
        let mut amostras = Vec::new();
        // 0,5 s de silêncio (zeros) — repeat_n gera N cópias do valor
        amostras.extend(std::iter::repeat_n(0.0, taxa as usize / 2));
        // 1 s de tom senoidal a 440 Hz (energia alta)
        for i in 0..taxa as usize {
            let t = i as f32 / taxa as f32;
            amostras.push(0.5 * (2.0 * std::f32::consts::PI * 440.0 * t).sin());
        }
        // 0,5 s de silêncio
        amostras.extend(std::iter::repeat_n(0.0, taxa as usize / 2));
        AudioData { amostras, taxa_amostragem: taxa }
    }

    #[test]
    fn silencio_puro_nao_gera_segmento() {
        let audio = AudioData {
            amostras: vec![0.0; 16000],
            taxa_amostragem: 16000,
        };
        assert!(detectar_fala(&audio, 0.02).is_empty());
    }

    #[test]
    fn tom_no_meio_gera_um_segmento() {
        let segmentos = detectar_fala(&audio_padrao(), 0.02);
        assert_eq!(segmentos.len(), 1);
        // O segmento deve começar perto de 0,5 s e terminar perto de 1,5 s
        // (com o padding de 0,1 s, um pouco antes/depois disso).
        assert!(segmentos[0].inicio < 0.6);
        assert!(segmentos[0].fim > 1.4);
        assert!(segmentos[0].duracao() > 0.8);
    }
}
