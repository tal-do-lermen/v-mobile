//! src/palavras_chave.rs — FONTE DAS PALAVRAS-CHAVE
//!
//! AQUI VOCÊ APRENDE: estratégia de fallback em cadeia, parsing manual de
//! texto e client HTTP síncrono com timeout.
//!
//! A tabela de palavras-chave pode vir de TRÊS fontes, nesta ordem:
//!
//!   1. API (--api-url)     → corpo de texto no MESMO formato do arquivo;
//!   2. arquivo local (--palavras, padrão ../palavras-chaves/palavras_chaves.txt);
//!   3. tabela embutida     → compilada dentro do binário (nunca falha).
//!
//! Se uma fonte falha (API fora do ar, arquivo ausente), registramos um
//! warning no log e CAIMOS PARA A PRÓXIMA — o detector nunca fica sem
//! tabela, só fica com uma menos atualizada. A fonte usada vai para o
//! relatório (`fonte_palavras`), porque transparência importa.

use crate::error::{DetectorError, Resultado};
use std::path::Path;
use std::time::Duration;

/// Uma tabela carregada: pares (palavra, peso) + de onde ela veio.
#[derive(Debug, Clone)]
pub struct TabelaPalavras {
    /// Ordem do texto de origem preservada (o txt vem ordenado por peso).
    pub entradas: Vec<(String, f32)>,
    /// "api" | "arquivo" | "embutida" — vai para o relatório.
    pub fonte: &'static str,
}

impl TabelaPalavras {
    /// Soma dos 3 maiores pesos — o "quanto vale 100%" do classificador.
    ///
    /// Versão DINÂMICA do antigo `SCORE_REFERENCIA` (era 3.0 + 2.0 + 2.0
    /// da tabela embutida). Como a tabela agora vem de fora e pode mudar,
    /// a referência tem que acompanhar: com a tabela nova (vários 3.0),
    /// a referência vira 9.0 e as confianças continuam proporcionais.
    pub fn score_referencia(&self) -> f32 {
        let mut pesos: Vec<f32> = self.entradas.iter().map(|&(_, p)| p).collect();
        // Ordena do MAIOR para o menor. `partial_cmp` porque f32 não tem
        // ordem total (NaN!) — se aparecer NaN, tratamos como empate.
        pesos.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
        // `.take(3)` = só os 3 primeiros; `.max(1.0)` evita divisão por 0
        // no caso improvável de tabela com pesos todos zerados.
        pesos.iter().take(3).sum::<f32>().max(1.0)
    }

    /// Tabela embutida no binário — o último elo da cadeia de fallback.
    /// Mesmos pares do `PALAVRAS_CHAVE` que antes morava em classify.rs.
    pub fn embutida() -> Self {
        Self {
            entradas: TABELA_EMBUTIDA
                .iter()
                .map(|(p, w)| (p.to_string(), *w))
                .collect(),
            fonte: "embutida",
        }
    }
}

/// A cadeia de fallback completa: API → arquivo → embutida.
///
/// NUNCA retorna Err: pior caso, devolve a tabela embutida. O chamador
/// (pipeline) não precisa tratar erro nenhum — só usar.
pub fn carregar(api_url: Option<&str>, caminho_txt: &Path) -> TabelaPalavras {
    // Elo 1: API (só se o usuário passou --api-url).
    if let Some(url) = api_url {
        match baixar_api(url) {
            Ok(tabela) => {
                tracing::info!(url, entradas = tabela.entradas.len(), "palavras-chave via API");
                return tabela;
            }
            Err(e) => {
                tracing::warn!(url, erro = %e, "API falhou — caindo para o arquivo local");
            }
        }
    }

    // Elo 2: arquivo local.
    match ler_arquivo(caminho_txt) {
        Ok(tabela) => {
            tracing::info!(
                caminho = %caminho_txt.display(),
                entradas = tabela.entradas.len(),
                "palavras-chave via arquivo"
            );
            return tabela;
        }
        Err(e) => {
            tracing::warn!(
                caminho = %caminho_txt.display(),
                erro = %e,
                "arquivo indisponível — usando tabela embutida"
            );
        }
    }

    // Elo 3: embutida. Sem rede, sem disco — sempre funciona.
    TabelaPalavras::embutida()
}

/// Busca a tabela na API. O corpo deve ser TEXTO no mesmo formato do
/// arquivo: `("palavra", 3.0),` uma por linha. Timeout curto: a tabela é
/// só um insumo — não vale travar o programa 30 s esperando.
fn baixar_api(url: &str) -> Resultado<TabelaPalavras> {
    let agente: ureq::Agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(5))
        .build();
    let corpo = agente
        .get(url)
        .call()
        .map_err(|e| DetectorError::ApiPalavras(e.to_string()))? // HTTP/ rede
        .into_string()
        .map_err(|e| DetectorError::ApiPalavras(e.to_string()))?; // leitura do corpo
    parse_tabela(&corpo, "api")
}

/// Lê e parseia o arquivo local.
pub fn ler_arquivo(caminho: &Path) -> Resultado<TabelaPalavras> {
    let texto = std::fs::read_to_string(caminho)?;
    parse_tabela(&texto, "arquivo")
}

/// O PARSER — o mesmo para API e arquivo (por isso a API devolve o formato
/// idêntico ao txt: um parser só, dois consumidores).
///
/// Aceita linhas do tipo `("palavra", 3.0),` — espaços e vírgula final
/// opcionais — e ignora em silêncio linhas em branco ou malformadas
/// (tabela vem de gente humana; tolerância a falha local ajuda).
/// Se NENHUMA linha for válida, aí sim é erro: tabela vazia não tem valor.
pub fn parse_tabela(texto: &str, fonte: &'static str) -> Resultado<TabelaPalavras> {
    let mut entradas = Vec::new();

    for linha in texto.lines() {
        // `and_then` em Option = "map que pode falhar": encadeia conversões
        // que devolvem Option. Se qualquer passo falhar, a linha é pulada.
        let parse = linha
            .trim()
            .trim_end_matches(',')
            .strip_prefix('(')
            .and_then(|s| s.strip_suffix(')'))
            .and_then(|s| s.split_once(','))
            .and_then(|(p, w)| {
                let palavra = p.trim().trim_matches('"').to_string();
                w.trim().parse::<f32>().ok().map(|peso| (palavra, peso))
            });

        if let Some(par) = parse {
            if !par.0.is_empty() {
                entradas.push(par);
            }
        }
    }

    if entradas.is_empty() {
        return Err(DetectorError::TabelaInvalida(
            "nenhuma linha válida no formato (\"palavra\", peso)".to_string(),
        ));
    }

    Ok(TabelaPalavras { entradas, fonte })
}

/// Tabela EMBUTIDA no binário — espelho completo do arquivo
/// ../palavras-chaves/palavras_chaves.txt (mesmas palavras, mesmos pesos,
/// mesma ordem). Genéricas já rebaixadas para 0.5 (higiene anti-falso-
/// positivo). Se a API e o arquivo falharem, é ISTO que classifica.
const TABELA_EMBUTIDA: &[(&str, f32)] = &[
    // ---- peso 3.0 ----
    ("adquirir", 3.0),
    ("aquisicao", 3.0),
    ("aquisição", 3.0),
    ("atende", 3.0),
    ("atender", 3.0),
    ("atendeu", 3.0),
    ("atendimento", 3.0),
    ("boleto", 3.0),
    ("brinde", 3.0),
    ("carrinho", 3.0),
    ("cartao", 3.0),
    ("cartão", 3.0),
    ("cashback", 3.0),
    ("checkout", 3.0),
    // ---- peso 0.5 ----
    ("cliente", 0.5),
    ("clientes", 0.5),
    // ---- peso 3.0 ----
    ("compra", 3.0),
    ("comprador", 3.0),
    ("comprando", 3.0),
    ("comprar", 3.0),
    ("compras", 3.0),
    ("comprasse", 3.0),
    ("comprei", 3.0),
    ("comprou", 3.0),
    ("consumidor", 3.0),
    ("cupom", 3.0),
    ("custo", 3.0),
    ("debito", 3.0),
    ("desconto", 3.0),
    ("descontos", 3.0),
    ("dinheiro", 3.0),
    ("débito", 3.0),
    ("encomenda", 3.0),
    ("encomendas", 3.0),
    ("entrega", 3.0),
    ("entregar", 3.0),
    ("entregas", 3.0),
    ("entregue", 3.0),
    ("especie", 3.0),
    ("espécie", 3.0),
    ("estoque", 3.0),
    ("frete", 3.0),
    ("frete gratis", 3.0),
    ("frete grátis", 3.0),
    ("juros", 3.0),
    ("liquidacao", 3.0),
    ("liquidação", 3.0),
    ("oferta", 3.0),
    ("ofertas", 3.0),
    ("orcamento", 3.0),
    ("orçamento", 3.0),
    ("pagamento", 3.0),
    ("pagamentos", 3.0),
    ("pagar", 3.0),
    ("pago", 3.0),
    ("paguei", 3.0),
    ("parcela", 3.0),
    ("parcelado", 3.0),
    ("parcelamento", 3.0),
    ("parcelar", 3.0),
    ("parcelas", 3.0),
    ("pedido", 3.0),
    ("pedidos", 3.0),
    ("pix", 3.0),
    ("preco", 3.0),
    ("precos", 3.0),
    ("preço", 3.0),
    ("preços", 3.0),
    ("promocao", 3.0),
    ("promocoes", 3.0),
    ("promoção", 3.0),
    ("promoções", 3.0),
    ("proposta", 3.0),
    ("queima de estoque", 3.0),
    ("sem juros", 3.0),
    ("valor", 3.0),
    ("valores", 3.0),
    ("venda", 3.0),
    ("vendas", 3.0),
    ("vende", 3.0),
    ("vendedor", 3.0),
    ("vendedora", 3.0),
    ("vendedores", 3.0),
    ("vendendo", 3.0),
    ("vender", 3.0),
    ("vendeu", 3.0),
    ("vendi", 3.0),
    ("vendida", 3.0),
    ("vendido", 3.0),
    // ---- peso 2.5 ----
    ("5 vezes", 2.5),
    ("a vista", 2.5),
    ("aprovacao", 2.5),
    ("aprovado", 2.5),
    ("aprovação", 2.5),
    ("assinatura", 2.5),
    ("atacado", 2.5),
    ("balcao", 2.5),
    ("balcão", 2.5),
    ("barato", 2.5),
    ("baratos", 2.5),
    ("cadastrar", 2.5),
    ("cadastro", 2.5),
    ("cnpj", 2.5),
    ("codigo", 2.5),
    ("codigo de barras", 2.5),
    ("comissao", 2.5),
    ("comissão", 2.5),
    ("condicao", 2.5),
    ("condicoes", 2.5),
    ("condição", 2.5),
    ("condições", 2.5),
    ("confirmar", 2.5),
    ("cotacao", 2.5),
    ("cotação", 2.5),
    ("cpf", 2.5),
    ("crediario", 2.5),
    ("credito", 2.5),
    ("crediário", 2.5),
    ("crédito", 2.5),
    ("código", 2.5),
    ("código de barras", 2.5),
    ("devolucao", 2.5),
    ("devolução", 2.5),
    ("disponivel", 2.5),
    ("disponível", 2.5),
    ("entrada", 2.5),
    ("entradinha", 2.5),
    ("faturamento", 2.5),
    ("fechadinha", 2.5),
    ("fechamento", 2.5),
    ("fechar", 2.5),
    ("financiamento", 2.5),
    ("garantia", 2.5),
    ("garantias", 2.5),
    ("indisponivel", 2.5),
    ("indisponível", 2.5),
    ("investimento", 2.5),
    ("lead", 2.5),
    ("limite", 2.5),
    // ---- peso 0.5 ----
    ("loja", 0.5),
    ("lojas", 0.5),
    // ---- peso 2.5 ----
    ("mensalidade", 2.5),
    ("meta", 2.5),
    ("mostruario", 2.5),
    ("mostruário", 2.5),
    ("negociacao", 2.5),
    ("negociar", 2.5),
    ("negociação", 2.5),
    ("nota fiscal", 2.5),
    ("oportunidade", 2.5),
    ("pdv", 2.5),
    ("pos", 2.5),
    ("prazo", 2.5),
    ("pronta entrega", 2.5),
    ("pronta-entrega", 2.5),
    ("prospect", 2.5),
    ("recibo", 2.5),
    ("reposicao", 2.5),
    ("reposição", 2.5),
    ("reserva", 2.5),
    ("reservar", 2.5),
    ("retirada", 2.5),
    ("retirar", 2.5),
    ("revenda", 2.5),
    ("showroom", 2.5),
    ("simulacao", 2.5),
    ("simulação", 2.5),
    ("troca", 2.5),
    ("varejo", 2.5),
    ("à vista", 2.5),
    // ---- peso 2.0 ----
    ("adicionar", 2.0),
    ("amostra", 2.0),
    ("avaliacao", 2.0),
    ("avaliação", 2.0),
    // ---- peso 0.5 ----
    ("beneficio", 0.5),
    ("beneficios", 0.5),
    ("benefício", 0.5),
    ("benefícios", 0.5),
    // ---- peso 2.0 ----
    ("caixa", 2.0),
    ("catalogo", 2.0),
    ("catálogo", 2.0),
    ("confirma", 2.0),
    ("demonstracao", 2.0),
    ("demonstração", 2.0),
    ("diferenciais", 2.0),
    ("diferencial", 2.0),
    // ---- peso 0.5 ----
    ("disponibilidade", 0.5),
    // ---- peso 2.0 ----
    ("em conta", 2.0),
    // ---- peso 0.5 ----
    ("escolha", 0.5),
    ("escolher", 0.5),
    // ---- peso 2.0 ----
    ("experimentar", 2.0),
    ("fidelidade", 2.0),
    ("finalizar", 2.0),
    ("fornecedor", 2.0),
    ("gerente", 2.0),
    ("indicacao", 2.0),
    ("indicação", 2.0),
    // ---- peso 0.5 ----
    ("item", 0.5),
    ("itens", 0.5),
    // ---- peso 2.0 ----
    ("leva", 2.0),
    ("levar", 2.0),
    ("leve", 2.0),
    // ---- peso 0.5 ----
    ("marca", 0.5),
    ("marcas", 0.5),
    ("modelo", 0.5),
    ("modelos", 0.5),
    ("mostra", 0.5),
    ("mostrar", 0.5),
    // ---- peso 2.0 ----
    ("nota", 2.0),
    ("objecao", 2.0),
    ("objeção", 2.0),
    // ---- peso 0.5 ----
    ("plano", 0.5),
    ("preferencia", 0.5),
    ("preferência", 0.5),
    // ---- peso 2.0 ----
    ("produto", 2.0),
    ("produtos", 2.0),
    // ---- peso 0.5 ----
    ("qualidade", 0.5),
    ("quanto", 0.5),
    // ---- peso 2.0 ----
    ("recompra", 2.0),
    ("sugestao", 2.0),
    ("sugestoes", 2.0),
    ("sugestão", 2.0),
    ("sugestões", 2.0),
    ("trocar", 2.0),
    // ---- peso 0.5 ----
    ("vantagem", 0.5),
    ("vantagens", 0.5),
    // ---- peso 2.0 ----
    ("vezes", 2.0),
    ("vitrine", 2.0),
    // ---- peso 0.5 ----
    ("ajuda", 0.5),
    ("ajudar", 0.5),
    // ---- peso 1.5 ----
    ("atencao", 1.5),
    ("atencioso", 1.5),
    // ---- peso 0.5 ----
    ("atenção", 0.5),
    ("contato", 0.5),
    ("duvida", 0.5),
    ("duvidas", 0.5),
    ("dúvida", 0.5),
    ("dúvidas", 0.5),
    ("gostaria", 0.5),
    ("interessado", 0.5),
    ("interesse", 0.5),
    ("precisa", 0.5),
    // ---- peso 1.5 ----
    ("preferir", 1.5),
    // ---- peso 0.5 ----
    ("quer", 0.5),
    ("retorno", 0.5),
    // ---- peso 1.5 ----
    ("satisfacao", 1.5),
    ("satisfação", 1.5),
    // ---- peso 0.5 ----
    ("teste", 0.5),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parseia_formato_do_arquivo_txt() {
        let texto = "(\"adquirir\", 3.0),\n(\"frete gratis\", 3.0),\n(\"teste\", 1.0),";
        let t = parse_tabela(texto, "arquivo").unwrap();
        assert_eq!(t.entradas.len(), 3);
        assert_eq!(t.entradas[0], ("adquirir".to_string(), 3.0));
        // Frases com espaço também funcionam (split na VÍRGULA, não no espaço).
        assert_eq!(t.entradas[1], ("frete gratis".to_string(), 3.0));
        assert_eq!(t.entradas[2], ("teste".to_string(), 1.0));
    }

    #[test]
    fn tolera_linhas_malformadas_e_vazio() {
        let texto = "\nlixo qualquer\n(\"ok\", 2.5),\n(\"sem peso\", abc),\n  \n(\"boa\",1.0)";
        let t = parse_tabela(texto, "arquivo").unwrap();
        // Só as 2 linhas bem-formadas entram; o resto é ignorado.
        assert_eq!(t.entradas.len(), 2);
        assert_eq!(t.entradas[0], ("ok".to_string(), 2.5));
    }

    #[test]
    fn texto_sem_linhas_validas_da_erro() {
        let r = parse_tabela("nada aqui", "arquivo");
        assert!(matches!(r, Err(DetectorError::TabelaInvalida(_))));
    }

    #[test]
    fn score_referencia_e_a_soma_do_top3() {
        let t = parse_tabela("(\"a\", 1.0),\n(\"b\", 3.0),\n(\"c\", 2.0),\n(\"d\", 2.5),", "x").unwrap();
        // Top 3 pesos: 3.0 + 2.5 + 2.0 = 7.5 (a ordem das linhas não importa).
        assert!((t.score_referencia() - 7.5).abs() < 1e-5);
        // A embutida (espelho do txt) tem vários pesos 3.0 no topo → 9.0.
        assert!((TabelaPalavras::embutida().score_referencia() - 9.0).abs() < 1e-5);
        // E ela é BIG: 248 termos, o mesmo número do arquivo de origem.
        assert_eq!(TabelaPalavras::embutida().entradas.len(), 248);
    }
}
