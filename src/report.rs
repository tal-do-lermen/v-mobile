//! src/report.rs
//!
//! AQUI VOCÊ APRENDE: serialização com serde — transformar structs em JSON.
//!
//! O relatório é a "saída oficial" do app: um JSON com tudo que o pipeline
//! descobriu. Repare que `Segmento` (vad.rs) e `Decisao` (classify.rs) já
//! derivam `Serialize`, então elas se encaixam aqui sem esforço — cada struct
//! vira um objeto JSON com os mesmos nomes dos campos.

use crate::classify::Decisao;
use crate::error::Resultado;
use crate::vad::Segmento;
use serde::Serialize;
use std::path::Path;

/// Relatório completo de uma análise.
///
/// CONCEITO — derivando `Serialize`: o serde gera, em tempo de compilação,
/// o código que percorre cada campo e escreve o JSON. Para os nomes ficarem
/// em snake_case no JSON (padrão), usamos o atributo `rename_all`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct Relatorio {
    /// Caminho do áudio analisado (como o usuário digitou).
    pub arquivo: String,
    /// Duração total em segundos.
    pub duracao_segundos: f32,
    /// Qual transcritor foi usado ("mock" ou "vosk").
    pub transcritor: String,
    /// Segmentos de fala detectados pelo VAD.
    pub segmentos: Vec<Segmento>,
    /// Texto transcrito.
    pub transcricao: String,
    /// Veredito: é venda? qual confiança?
    pub decisao: Decisao,
}

impl Relatorio {
    /// Serializa para JSON "bonito" (indentado) e grava no disco.
    ///
    /// CONCEITO — dois `?` de tipos diferentes: `to_string_pretty` retorna
    /// `serde_json::Error` e `fs::write` retorna `io::Error`. Os dois caem
    /// no nosso `DetectorError` porque error.rs tem `#[from]` para ambos —
    /// o `?` converte cada um automaticamente.
    pub fn salvar(&self, caminho: &Path) -> Resultado<()> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(caminho, json)?;
        tracing::info!(caminho = %caminho.display(), "relatório salvo");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializa_para_json_esperado() {
        let rel = Relatorio {
            arquivo: "exemplo.wav".into(),
            duracao_segundos: 2.0,
            transcritor: "mock".into(),
            segmentos: vec![Segmento { inicio: 0.5, fim: 1.5 }],
            transcricao: "fechar o pedido".into(),
            decisao: Decisao {
                e_venda: true,
                confianca: 0.9,
                palavras_chave: vec!["fechar o pedido".into()],
            },
        };
        let json = serde_json::to_string(&rel).unwrap();
        // Campos-chave presentes no JSON gerado:
        assert!(json.contains("\"arquivo\":\"exemplo.wav\""));
        assert!(json.contains("\"e_venda\":true"));
    }
}
