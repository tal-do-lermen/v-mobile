//! src/audio.rs
//!
//! AQUI VOCÊ APRENDE: structs com dados, `Vec`, slices, ownership e
//! conversão de tipos numéricos.
//!
//! Este módulo é o ESTÁGIO 1 do pipeline: lê o arquivo .wav do disco e
//! transforma em números que o resto do código consegue processar.

use crate::error::{DetectorError, Resultado};
use std::path::Path;

/// Estrutura que representa o áudio JÁ decodificado, pronto para processar.
///
/// `#[derive(Debug)]` gera a implementação do trait `Debug`, que permite
/// imprimir a struct com `{:?}` (útil em logs e testes).
#[derive(Debug)]
pub struct AudioData {
    /// Amostras do áudio normalizadas para o intervalo [-1.0, 1.0].
    ///
    /// CONCEITO — `Vec<f32>`: um vetor de floats 32 bits que "possui" os dados
    /// (está no heap, cresce/diminui, e é liberado automaticamente quando a
    /// struct sai de escopo — isso é o ownership agindo).
    ///
    /// O WAV guarda cada amostra como `i16` (inteiro de -32768 a 32767).
    /// Dividimos por 32768.0 para virar float entre -1.0 e 1.0, que é o
    /// formato que o VAD e o Vosk esperam.
    pub amostras: Vec<f32>,

    /// Quantas amostras por segundo (ex.: 16000 = 16 kHz).
    /// `u32` = inteiro sem sinal de 32 bits.
    pub taxa_amostragem: u32,
}

impl AudioData {
    /// Duração total do áudio em segundos.
    ///
    /// Note: `&self` = método que só LÊ a struct (empréstimo imutável).
    /// A duração é amostras ÷ taxa — regra de três básica.
    pub fn duracao_segundos(&self) -> f32 {
        self.amostras.len() as f32 / self.taxa_amostragem as f32
    }

    /// Lê o arquivo .wav do disco e devolve um `AudioData`.
    ///
    /// CONCEITO — assinatura: `fn carregar(caminho: &Path) -> Resultado<Self>`
    ///   - `&Path` = empréstimo imutável do caminho (NÃO tomamos ownership;
    ///     quem chamou continua podendo usar o Path depois)
    ///   - `Resultado<Self>` = `Result<AudioData, DetectorError>` — pode falhar!
    pub fn carregar(caminho: &Path) -> Resultado<Self> {
        // `Path::exists` pergunta ao sistema de arquivos. Se não existe,
        // retornamos cedo o nosso erro (construído com o caminho dentro).
        if !caminho.exists() {
            return Err(DetectorError::ArquivoNaoEncontrado(
                // `display()` converte Path para algo imprimível
                caminho.display().to_string(),
            ));
        }

        // Tenta abrir o WAV. O `?` faz duas coisas:
        //   1. se der `Err`, retorna da função já convertendo hound::Error
        //      para DetectorError (graças ao `#[from]` em error.rs)
        //   2. se der `Ok`, desempacota o valor
        let leitor = hound::WavReader::open(caminho)?;

        // Metadados do arquivo: canais, taxa, bits...
        let spec = leitor.spec();


        // Agora ACEITAMOS qualquer nº de canais (mono, estéreo...) e
        // convertemos para mono mais abaixo (downmix). Só rejeitamos
        // 0 canais: não existe em WAV válido e a média dividiria por 0.
        if spec.channels == 0 {
            return Err(DetectorError::FormatoNaoSuportado {
                detalhe: "número de canais inválido: 0".to_string(),
            });
        }

        // Quantos canais o arquivo tem (1 = mono, 2 = estéreo...).
        // `usize` é o tipo que o Rust usa para índices e tamanhos de Vec.
        let canais = spec.channels as usize;
        // E apenas PCM 16 bits (o formato mais comum para voz).
        if spec.bits_per_sample != 16 || spec.sample_format != hound::SampleFormat::Int {
            return Err(DetectorError::FormatoNaoSuportado {
                detalhe: format!(
                    "esperado PCM 16 bits, encontrado {} bits ({:?})",
                    spec.bits_per_sample, spec.sample_format
                ),
            });
        }

        // `into_samples::<i16>()` devolve um ITERADOR de Result<i16> —
        // um por amostra, sem carregar tudo de uma vez.
        //
        // ATENÇÃO: num WAV com vários canais as amostras vêm INTERCALADAS.
        // Um estéreo guarda [L, R, L, R, L, R...] — cada PAR é um "frame",
        // ou seja, o mesmo instante sonoro gravado nos dois canais.
        let brutos: Vec<i16> = leitor
            .into_samples::<i16>()
            .collect::<hound::Result<Vec<i16>>>()?;

        // CONCEITO — downmix (conversão p/ mono) com `chunks`:
        //
        //   .chunks(n)    → fatia o Vec em pedaços de `n` elementos.
        //     Em estéreo cada chunk é 1 frame: [L, R]. Em mono,
        //     chunks(1) gera chunks de 1 elemento — o MESMO código
        //     serve para os dois casos, sem nenhum `if` extra.
        //   .filter(...)  → descarta um possível frame truncado no fim
        //     (um arquivo corrompido poderia ter só o L do último frame).
        //   .map(...)     → condensa o frame em UMA amostra mono: (L+R)/2.
        //     A soma é feita em f32 porque em i16 daria OVERFLOW:
        //     32767 + 32767 = 65534, maior que o teto do i16 (32767).
        //     Somar em float e só depois dividir evita isso.
        //   .collect()    → monta o Vec<f32> final, já em mono.
        let amostras: Vec<f32> = brutos
            .chunks(canais)
            .filter(|frame| frame.len() == canais)
            .map(|frame| {
                let soma: f32 = frame.iter().map(|&s| s as f32 / 32768.0).sum();
                soma / canais as f32
            })
            .collect();

        // Áudio vazio não faz sentido para o pipeline — erro explícito.
        if amostras.is_empty() {
            return Err(DetectorError::AudioVazio);
        }

        tracing::debug!(
            amostras = amostras.len(),
            taxa = spec.sample_rate,
            "WAV carregado"
        );

        // Sem `;` na última linha = RETURN implícito: a expressão É o valor
        // de retorno (Rust é orientado a expressões).
        Ok(Self {
            amostras,
            taxa_amostragem: spec.sample_rate,
        })
    }
}

/// TESTES UNITÁRIOS — AQUI VOCÊ APRENDE: `#[test]`.
/// Rodam com `cargo test`. Cada `fn` marcada é um caso de teste independente.
#[cfg(test)]
mod tests {
    use super::*; // importa tudo do módulo de cima (AudioData etc.)
    use hound::{SampleFormat, WavSpec, WavWriter};

    /// Helper: cria um WAV temporário com N amostras de um tom senoidal.
    /// Devolve o caminho para os testes usarem.
    fn criar_wav_teste(caminho: &Path, n_amostras: usize, frequencia: f32) {
        let spec = WavSpec {
            channels: 1,
            sample_rate: 16000,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        let mut w = WavWriter::create(caminho, spec).unwrap();
        for i in 0..n_amostras {
            let t = i as f32 / 16000.0;
            let valor = (0.5 * (2.0 * std::f32::consts::PI * frequencia * t).sin() * 32767.0) as i16;
            w.write_sample(valor).unwrap();
        }
        w.finalize().unwrap();
    }

    /// Helper: cria um WAV ESTÉREO (2 canais) com um tom diferente em cada
    /// canal — serve para testar o downmix para mono.
    fn criar_wav_estereo_teste(caminho: &Path, n_frames: usize, freq_esq: f32, freq_dir: f32) {
        let spec = WavSpec {
            channels: 2,
            sample_rate: 16000,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        let mut w = WavWriter::create(caminho, spec).unwrap();
        for i in 0..n_frames {
            let t = i as f32 / 16000.0;
            let esq = (0.5 * (2.0 * std::f32::consts::PI * freq_esq * t).sin() * 32767.0) as i16;
            let dir = (0.5 * (2.0 * std::f32::consts::PI * freq_dir * t).sin() * 32767.0) as i16;
            // Num estéreo escrevemos o FRAME inteiro: primeiro L, depois R.
            w.write_sample(esq).unwrap();
            w.write_sample(dir).unwrap();
        }
        w.finalize().unwrap();
    }

    #[test]
    fn carrega_e_calcula_duracao() {
        // Cada teste usa um arquivo único para não brigar com outros testes.
        let caminho = std::env::temp_dir().join("vd_audio_teste.wav");
        criar_wav_teste(&caminho, 16000, 440.0); // 1 segundo a 16 kHz

        let audio = AudioData::carregar(&caminho).unwrap();

        assert_eq!(audio.amostras.len(), 16000);
        assert_eq!(audio.taxa_amostragem, 16000);
        // `assert!` + comparação com margem: floats raramente são exatos.
        assert!((audio.duracao_segundos() - 1.0).abs() < 1e-4);

        // Normalização: valores devem caber em [-1.0, 1.0].
        assert!(audio.amostras.iter().all(|&a| a.abs() <= 1.0));
    }

    #[test]
    fn arquivo_inexistente_da_erro_especifico() {
        let resultado = AudioData::carregar(Path::new("/tmp/nao_existe_12345.wav"));
        // Pattern matching no Result: checa qual variante do erro veio.
        assert!(matches!(
            resultado,
            Err(DetectorError::ArquivoNaoEncontrado(_))
        ));
    }

    #[test]
    fn converte_estereo_para_mono() {
        let caminho = std::env::temp_dir().join("vd_audio_estereo_teste.wav");
        criar_wav_estereo_teste(&caminho, 1000, 440.0, 0.0); // 1000 frames; R nulo

        let audio = AudioData::carregar(&caminho).unwrap();

        // 2000 amostras intercaladas viram 1000 mono (1 amostra por frame).
        assert_eq!(audio.amostras.len(), 1000);
        assert_eq!(audio.taxa_amostragem, 16000);

        // Canal direito é nulo (freq 0 → seno sempre 0), então cada amostra
        // mono deve ser EXATAMENTE a metade da esquerda — prova a média.
        let t = 10.0 / 16000.0;
        let esperado = 0.5 * (2.0 * std::f32::consts::PI * 440.0 * t).sin();
        assert!((audio.amostras[10] - esperado / 2.0).abs() < 1e-4);
    }
}
