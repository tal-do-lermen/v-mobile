//! src/cli.rs
//!
//! AQUI VOCÊ APRENDE: structs, derive e argumentos de linha de comando.
//!
//! `clap` com a feature "derive" gera, em tempo de COMPILAÇÃO, todo o código
//! de parsing a partir da struct abaixo. O atributo `#[derive(Parser)]` implementa
//! o trait `clap::Parser` para `Cli` — de graça, sem escrever parser manual.

use clap::Parser;
use std::path::PathBuf; // PathBuf = caminho de arquivo dono da string (ownership)

/// Doc comment em cima da struct vira o texto de ajuda do `--help`.
#[derive(Parser, Debug)]
#[command(
    name = "venda-detector",
    version,
    about = "Detecta se um áudio .wav contém uma ligação de venda"
)]
pub struct Cli {
    /// Caminho do arquivo de áudio .wav (mono, 16 bits).
    /// Campo SEM atributo `#[arg]` = argumento posicional obrigatório:
    /// `venda-detector caminho/audio.wav`
    pub audio: PathBuf,

    /// Se informado, grava o relatório JSON neste caminho.
    /// `Option<PathBuf>` = "pode não vir nada" → vira None.
    /// `#[arg(short, long)]` = aceita `-s` e `--saida`.
    #[arg(short, long)]
    pub saida: Option<PathBuf>,

    /// Limiar de energia RMS (0.0 a 1.0) que o VAD usa para decidir
    /// se uma janela de 30 ms contém fala. Menor = mais sensível.
    /// `default_value_t` injeta o valor quando a flag não é passada.
    #[arg(short, long, default_value_t = 0.02)]
    pub limiar: f32,

    /// Usa o transcritor Vosk real (precisa compilar com --features vosk,
    /// rodar ./scripts/instalar_vosk.sh e ter a PASTA do modelo em --modelo).
    /// Sem isso, usa o mock.
    /// `bool` sem valor = flag booleana (`--vosk` presente = true).
    #[arg(long)]
    pub vosk: bool,

    /// Caminho da PASTA do modelo Vosk descompactado (só usado com --vosk).
    /// `default_value` aceita caminhos com valor padrão.
    #[arg(long, default_value = "models/asr/vosk-model-small-pt-0.3")]
    pub modelo: PathBuf,

    /// Ativa logs detalhados (nível debug) no terminal.
    #[arg(short, long)]
    pub verboso: bool,
}
