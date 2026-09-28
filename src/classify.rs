//! src/classify.rs
//!
//! AQUI VOCÊ APRENDE: `HashMap`/tabelas estáticas, `match`, enums COM DADOS
//! e o padrão "acumular durante iteração".
//!
//! ESTÁGIO 4 (último) do pipeline: olha o texto transcrito e decide se é
//! uma venda. Aqui usamos regras simples (palavras-chave com pesos) —
//! didático e sem IA. Numa versão futura, trocar por um modelo de
//! classificação é fácil: a "borda" já está isolada nesta função.

use serde::Serialize;

/// Resultado da classificação.
///
/// CONCEITO — struct com `Serialize`: quando o relatório JSON for gerado,
/// este struct vira:
///   "decisao": { "e_venda": true, "confianca": 0.8, "palavras_chave": [...] }
#[derive(Debug, Serialize)]
pub struct Decisao {
    /// true = parece uma venda
    pub e_venda: bool,
    /// 0.0 (nada) a 1.0 (máxima certeza)
    pub confianca: f32,
    /// Quais palavras-chave foram encontradas (transparência da decisão).
    pub palavras_chave: Vec<String>,
}

/// Tabela de palavras-chave e seus PESOS (quanto cada uma indica venda).
/// `&[(&str, f32)]` = slice (visão imutável) de tuplas texto+peso.
/// `const` + `&` = dado em memória estática: existe a vida toda do programa,
/// sem alocar em runtime.
const PALAVRAS_CHAVE: &[(&str, f32)] = &[
    // (palavra, peso)
    ("fechar o pedido", 3.0),
    ("preço", 2.0),
    ("preco", 2.0),          // sem acento, para pegar as duas grafias
    ("orçamento", 2.0),
    ("orcamento", 2.0),
    ("desconto", 2.0),
    ("proposta", 2.0),
    ("pagamento", 1.5),
    ("parcelas", 1.0),
    ("cartão", 1.0),
    ("cartao", 1.0),
    ("boleto", 1.0),
    ("à vista", 1.5),
    ("garantia", 1.0),
    ("contrato", 1.5),
];

/// Score de referência: as três palavras-chave mais fortes somadas
/// (3.0 + 2.0 + 2.0). Se o texto atinge esse score, confiança = 100%.
/// Normalizar por um "top 3" em vez da soma TOTAL é mais realista:
/// uma venda real não precisa citar TODAS as palavras da tabela.
const SCORE_REFERENCIA: f32 = 7.0;

/// Score (normalizado) a partir do qual consideramos "é venda".
const LIMIAR_DECISAO: f32 = 0.5;

/// Classifica a transcrição.
///
/// Assinatura emprestada: recebe `&str` (fatia de texto imutável — aceita
/// tanto `String` quanto `&str` quem chamar) e devolve uma `Decisao` nova.
pub fn classificar(transcricao: &str) -> Decisao {
    // `.to_lowercase()` normaliza para comparar sem se importar com maiúsculas.
    let texto = transcricao.to_lowercase();

    // Acumuladores que o loop abaixo preenche.
    let mut score = 0.0f32; // f32 com sufixo: tipo explícito no literal
    let mut encontradas: Vec<String> = Vec::new();

    // `for (palavra, peso) in ...` DESESTRUTURA cada tupla do slice
    // diretamente nas variáveis palavra/peso.
    for (palavra, peso) in PALAVRAS_CHAVE {
        if texto.contains(palavra) {
            score += peso;
            // `.to_string()` cria uma String própria (ownership nova) para
            // guardar no Vec — a `palavra` original é só um &str emprestado.
            encontradas.push((*palavra).to_string());
        }
    }

    // Normaliza para 0..1 e trava (clamp) contra passar de 1.0.
    let confianca = (score / SCORE_REFERENCIA).clamp(0.0, 1.0);

    let decisao = Decisao {
        e_venda: confianca >= LIMIAR_DECISAO,
        confianca,
        palavras_chave: encontradas,
    };

    tracing::debug!(?decisao, "classificação concluída"); // ?decisao = debug da struct no log
    decisao
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texto_de_venda_ponta_alta() {
        let d = classificar("Quero fechar o pedido hoje, qual o preço com desconto?");
        assert!(d.e_venda);
        assert!(d.confianca >= LIMIAR_DECISAO);
        assert!(d.palavras_chave.contains(&"fechar o pedido".to_string()));
    }

    #[test]
    fn conversa_comum_nao_e_venda() {
        let d = classificar("Oi, tudo bem? Só passando para desejar um bom dia!");
        assert!(!d.e_venda);
        assert!(d.palavras_chave.is_empty());
    }

    #[test]
    fn texto_vazio_nao_quebra() {
        let d = classificar("");
        assert!(!d.e_venda);
        assert_eq!(d.confianca, 0.0);
    }

    #[test]
    fn confianca_nunca_passa_de_um() {
        // Empilha MUITAS palavras-chave para testar o clamp.
        let d = classificar(
            "fechar o pedido preço orçamento desconto proposta pagamento parcelas \
             cartão boleto à vista garantia contrato",
        );
        assert!(d.confianca <= 1.0);
    }
}
