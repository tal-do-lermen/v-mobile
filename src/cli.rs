//! src/cli.rs
//!
//! AQUI VOCÊ APRENDE: structs, derive e argumentos de linha de comando.
//!
//! `clap` com a feature "derive" gera, em tempo de COMPILAÇÃO, todo o código
//! de parsing a partir da struct abaixo. O atributo `#[derive(Parser)]` implementa
//! o trait `clap::Parser` para `Cli` — de graça, sem escrever parser manual.

use crate::error::{DetectorError, Resultado};
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
    /// Opcional: com --stdin a entrada vem pelo pipe (PCM cru).
    /// Campo SEM atributo `#[arg]` = argumento posicional:
    /// `venda-detector caminho/audio.wav`
    pub audio: Option<PathBuf>,

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

    /// Lê PCM CRU (i16 little-endian, 16 kHz, mono) da entrada padrão.
    /// Fonte qualquer via pipe, ex.:
    ///   arecord -f S16_LE -r 16000 -c 1 | venda-detector --stdin --progressivo
    #[arg(long)]
    pub stdin: bool,

    /// Modo CLÁSSICO: carrega o áudio inteiro, transcreve uma vez e decide
    /// só no fim (o jeito antigo). Sem esta flag, arquivos usam o modo
    /// PROGRESSIVO por padrão (janelas com parada antecipada).
    #[arg(long)]
    pub completo: bool,

    /// ATENÇÃO: o progressivo agora é o PADRÃO — esta flag virou no-op e
    /// foi mantida só para não quebrar scripts antigos.
    #[arg(long)]
    pub progressivo: bool,

    /// EXPERIMENTAL — acelera o áudio antes da transcrição (atempo do ffmpeg).
    /// Menos amostras = análise mais rápida, MAS o modelo acústico é treinado
    /// com fala natural: acima de ~1.3x a transcrição degrada e VENDAS PODEM
    /// DEIXAR DE SER DETECTADAS (falso negativo). Só para experimentos.
    /// 1.0 = normal (padrão). Só se aplica a arquivos; --stdin ignora.
    #[arg(long, default_value_t = 1.0)]
    pub velocidade: f32,

    /// Janelas decodificadas EM PARALELO no modo progressivo de arquivos.
    /// 1 = sequencial (menos RAM, texto mais contínuo); 2+ = usa os núcleos
    /// (mais rápido, mas pode picar palavras na fronteira da janela).
    /// NÃO se aplica a --stdin (stream ao vivo é sempre sequencial).
    #[arg(long, default_value_t = 2)]
    pub threads: usize,

    /// Tamanho de cada janela do modo progressivo, em segundos.
    #[arg(long, default_value_t = 30.0)]
    pub janela: f32,

    /// Confiança mínima (0.0 a 1.0) para parar e declarar venda.
    /// Também é o limiar do veredito final no modo progressivo.
    #[arg(long, default_value_t = 0.70)]
    pub limiar_confianca: f32,

    /// URL da API que devolve as palavras-chave (texto no mesmo formato do
    /// arquivo: ("palavra", peso), uma por linha). Se a API falhar, cai
    /// automaticamente para --palavras e depois para a tabela embutida.
    /// Sem esta flag, a API nem é consultada.
    #[arg(long)]
    pub api_url: Option<String>,

    /// Caminho do arquivo local de palavras-chave (fallback da API).
    #[arg(long, default_value = "../palavras-chaves/palavras_chaves.txt")]
    pub palavras: PathBuf,

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

impl Cli {
    /// Regras que o clap sozinho não expressa: exatamente UMA fonte de áudio
    /// (arquivo OU --stdin) e valores dentro das faixas válidas.
    ///
    /// Truque de tabela-verdade: `stdin == audio.is_some()` só é `true` quando
    /// os dois estão presentes OU os dois ausentes — exatamente os dois erros.
    pub fn validar(&self) -> Resultado<()> {
        if self.stdin == self.audio.is_some() {
            return Err(DetectorError::Argumento(
                "informe UM caminho de .wav OU --stdin (não os dois, nem nenhum)".to_string(),
            ));
        }
        if self.janela <= 0.0 {
            return Err(DetectorError::Argumento(
                "--janela deve ser maior que 0".to_string(),
            ));
        }
        if !(0.0..=1.0).contains(&self.limiar_confianca) {
            return Err(DetectorError::Argumento(
                "--limiar-confianca deve estar entre 0.0 e 1.0".to_string(),
            ));
        }
        if self.threads == 0 {
            return Err(DetectorError::Argumento(
                "--threads deve ser pelo menos 1".to_string(),
            ));
        }
        if !(0.5..=4.0).contains(&self.velocidade) {
            return Err(DetectorError::Argumento(
                "--velocidade deve estar entre 0.5 e 4.0 (1.0 = normal)".to_string(),
            ));
        }
        Ok(())
    }

    /// Deriva a configuração do escaneador a partir dos argumentos.
    pub fn config_scan(&self) -> crate::scan::ConfigScan {
        crate::scan::ConfigScan {
            janela_seg: self.janela,
            limiar_confianca: self.limiar_confianca,
            limiar_vad: self.limiar,
        }
    }
}
