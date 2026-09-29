//! src/lib.rs
//!
//! Um projeto Rust pode ter DOIS pontos de entrada:
//!   - `src/lib.rs`  → a BIBLIOTECA: funções/structs que outros códigos importam
//!   - `src/main.rs` → o BINÁRIO: o que roda quando você digita `cargo run`
//!
//! Separar assim é um padrão comum: a lógica fica na lib (testável por
//! `tests/pipeline.rs` e reutilizável), e o main fica fino, só "colando" tudo.
//!
//! AQUI VOCÊ APRENDE: módulos. Cada `pub mod X;` diz ao compilador:
//!   "existe um arquivo src/X.rs e ele faz parte deste crate".
//! `pub` = público, visível fora do crate (o teste de integração vai importá-los).

pub mod audio;
pub mod classify;
pub mod cli;
pub mod error;
pub mod palavras_chave;
pub mod pipeline;
pub mod report;
pub mod scan;
pub mod transcribe;
pub mod vad;
