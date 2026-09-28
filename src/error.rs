//! src/error.rs
//!
//! AQUI VOCÊ APRENDE: tratamento de erros idiomático em Rust.
//!
//! Em Rust NÃO existe exceção (try/catch). Uma função que pode falhar retorna
//! `Result<T, E>`, onde:
//!   - `Ok(valor)`  → deu certo
//!   - `Err(erro)`  → deu errado
//!
//! O operador `?` usado dentro de funções que retornam `Result` faz o seguinte:
//!   - se o resultado é `Ok(v)`, desempacota e entrega `v`;
//!   - se é `Err(e)`, retorna `Err(e)` IMEDIATAMENTE da função atual.
//!
//! É isso que permite "empilhar" erros de baixo nível até o main sem if's.

use thiserror::Error;

/// O atributo `#[derive(Error, Debug)]` (do crate `thiserror`) gera
/// automaticamente a implementação do trait `std::error::Error` para este enum.
/// Só enums/structs podem usar derive; trait é implementado PARA você.
#[derive(Error, Debug)]
pub enum DetectorError {
    /// `#[error("...")]` define a mensagem exibida com `{}` (via Display).
    /// `{0}` interpola o primeiro campo da variante, como no `format!`.
    #[error("arquivo de áudio não encontrado: {0}")]
    ArquivoNaoEncontrado(String),

    /// `#[from]` gera AUTOMATICAMENTE `impl From<hound::Error> for DetectorError`.
    /// É isso que permite escrever `hound::WavReader::open(...)?` dentro de
    /// funções que retornam o NOSSO tipo de erro: o `?` converte sozinho.
    #[error("arquivo WAV inválido ou corrompido: {0}")]
    WavInvalido(#[from] hound::Error),

    /// Erros de disco (gravar o relatório, por exemplo) também são convertidos.
    #[error("erro de I/O: {0}")]
    Io(#[from] std::io::Error),

    /// Erro ao gerar o JSON do relatório (também via `#[from]`).
    #[error("erro ao gerar JSON: {0}")]
    Json(#[from] serde_json::Error),

    /// Variante com campos nomeados: `{canaais}` pega o campo de mesmo nome.
    #[error("formato de áudio não suportado: {detalhe}")]
    FormatoNaoSuportado { detalhe: String },

    #[error("o arquivo de áudio está vazio")]
    AudioVazio,

    #[error("falha na transcrição: {0}")]
    Transcricao(String),

    #[error("modelo do Vosk não encontrado: {0}")]
    ModeloNaoEncontrado(String),

    /// Variante sem dados: só sinaliza a condição. Usada quando o usuário pede
    /// `--vosk` mas o binário foi compilado sem a feature.
    #[error("Vosk não incluído neste binário. Compile com: cargo build --features vosk")]
    VoskNaoCompilado,
}

/// Apelido de tipo (type alias). `Resultado<T>` é a mesma coisa que
/// `std::result::Result<T, DetectorError>`, só que mais curto de ler.
/// Convenção comum em projetos Rust para não repetir o tipo de erro toda hora.
pub type Resultado<T> = std::result::Result<T, DetectorError>;
