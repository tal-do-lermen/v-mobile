//! src/pipeline.rs
//!
//! AQUI VOCÊ APRENDE: composição — juntar os 4 estágios em um fluxo só.
//!
//! Este é o "maestro" do app. Cada linha chama um estágio e passa o resultado
//! para o próximo:
//!
//!   arquivo .wav ──▶ [audio]  ──▶ AudioData
//!                    [vad]    ──▶ Vec<Segmento>
//!                    [transcribe] ─▶ String
//!                    [classify] ──▶ Decisao
//!                                 ──▶ Relatorio (JSON)
//!
//! Repare: quase não há lógica AQUI. O valor de um pipeline bem dividido é
//! que cada estágio pode ser testado/trocado isoladamente.

use crate::audio::AudioData;
use crate::cli::Cli;
use crate::error::Resultado;
use crate::report::Relatorio;
use crate::{classify, transcribe, vad};

/// Executa o pipeline completo para os argumentos da CLI.
///
/// `Resultado<Relatorio>`: qualquer estágio pode falhar, e o `?` abaixo
/// desmonta a pilha de chamadas na hora, levando o erro até o main.
pub fn executar(cli: &Cli) -> Resultado<Relatorio> {
    // ---- ESTÁGIO 1: carregar áudio -------------------------------------
    tracing::info!(arquivo = %cli.audio.display(), "estágio 1/4: carregando áudio");
    let audio = AudioData::carregar(&cli.audio)?;
    tracing::info!(duracao_s = format!("{:.2}", audio.duracao_segundos()), "áudio carregado");

    // ---- ESTÁGIO 2: VAD (cortar silêncio) ------------------------------
    tracing::info!("estágio 2/4: detectando fala (VAD)");
    let segmentos = vad::detectar_fala(&audio, cli.limiar);
    tracing::info!(segmentos = segmentos.len(), "segmentos de fala encontrados");

    // ---- ESTÁGIO 3: transcrição ----------------------------------------
    tracing::info!("estágio 3/4: transcrevendo");
    // A factory decide mock vs vosk. O `Box<dyn Transcritor>` permite
    // guardar "qualquer transcritor" nesta variável — polimorfismo em ação.
    let mut transcritor = transcribe::criar_transcritor(cli.vosk, &cli.modelo)?;
    let transcricao = transcritor.transcrever(&audio, &segmentos)?;

    // ---- ESTÁGIO 4: classificação --------------------------------------
    tracing::info!("estágio 4/4: classificando");
    let decisao = classify::classificar(&transcricao);

    // ---- MONTAGEM DO RELATÓRIO -----------------------------------------
    // Sintaxe de struct literal: `campo: valor`. Valores movidos (ownership
    // transferida) para dentro do Relatorio — depois daqui, `segmentos` e
    // `transcricao` pertencem ao relatório.
    let relatorio = Relatorio {
        arquivo: cli.audio.display().to_string(),
        duracao_segundos: audio.duracao_segundos(),
        transcritor: transcritor.nome().to_string(),
        segmentos,
        transcricao,
        decisao,
    };

    Ok(relatorio)
}
