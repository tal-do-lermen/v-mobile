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
pub trait Transcritor {
    /// Recebe o áudio emprestado + os segmentos do VAD, devolve o texto ou erro.
    fn transcrever(&mut self, audio: &AudioData, segmentos: &[Segmento]) -> Resultado<String>;

    /// Nome curto para logs/relatório ("mock", "vosk", ...).
    fn nome(&self) -> &'static str;
}

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
        Ok(
            "Bom dia! Gostaria de fechar o pedido hoje. Qual o preço com desconto? \
             Pode ser no cartão em três parcelas?"
                .to_string(),
        )
    }

    fn nome(&self) -> &'static str {
        "mock"
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
