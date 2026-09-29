//! src/classify.rs
//!
//! AQUI VOCÊ APRENDE: `HashMap`/tabelas estáticas, `match`, enums COM DADOS
//! e o padrão "acumular durante iteração".
//!
//! ESTÁGIO 4 (último) do pipeline: olha o texto transcrito e decide se é
//! uma venda. Aqui usamos regras simples (palavras-chave com pesos) —
//! didático e sem IA. Numa versão futura, trocar por um modelo de
//! classificação é fácil: a "borda" já está isolada nesta função.

use crate::palavras_chave::TabelaPalavras;
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

// A tabela de palavras que antes morava AQUI (`PALAVRAS_CHAVE`) mudou de
// endereço: virou `TABELA_EMBUTIDA` em src/palavras_chave.rs — o módulo que
// também sabe buscá-la na API (--api-url) ou no arquivo local (--palavras).

/// Score (normalizado) a partir do qual consideramos "é venda".
/// Alinhado com o --limiar-confianca do progressivo (0.70): conversa normal
/// precisa ficar BEM abaixo disso; venda real passa com folga.
/// `pub` para o pipeline usar o MESMO valor (nada de número mágico solto).
pub const LIMIAR_DECISAO: f32 = 0.70;

/// REGRA DE EVIDÊNCIA MÍNIMA: além do score, a decisão exige pelo menos
/// N palavras-chave DISTINTAS. Impede que uma palavra forte sozinha (ou
/// duas fracas) declare venda — falso positivo clássico de keyword matcher.
const MIN_DISTINTAS: usize = 3;

/// Quebra o texto em tokens minúsculos (letras/números, acentos preservados).
/// Ex.: "Qual o PREÇO, Dr.?" → ["qual", "o", "preço", "dr"]
///
/// CONCEITO — `split` com closure: dividimos em QUALQUER caractere que não
/// seja alfanumérico (espaço, vírgula, ponto...). Os `.filter/.map` limpos
/// os vazios e normalizam a caixa de cada pedaço.
fn tokenizar(texto: &str) -> Vec<String> {
    texto
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_lowercase())
        .collect()
}

/// Classifica a transcrição com o limiar PADRÃO do modo clássico (0.5)
/// e a tabela EMBUTIDA. Wrapper fino — mantém a assinatura antiga
/// funcionando para quem não precisa de tabela externa.
pub fn classificar(transcricao: &str) -> Decisao {
    classificar_com_limiar(transcricao, LIMIAR_DECISAO)
}

/// Versão parametrizada: o MODO PROGRESSIVO chama aqui com o limiar da CLI
/// (padrão 0.70) tanto nos checkpoints quanto no veredito final, usando a
/// tabela embutida.
pub fn classificar_com_limiar(transcricao: &str, limiar: f32) -> Decisao {
    classificar_com_tabela(transcricao, &TabelaPalavras::embutida(), limiar)
}

/// VERSÃO COMPLETA: recebe a tabela carregada (API/arquivo/embutida) e o
/// limiar. É por aqui que o pipeline inteiro passa — a tabela dinâmica
/// substitui a const antiga, e `score_referencia()` normaliza pelo
/// top-3 DA TABELA EM USO (não de uma constante fixa).
pub fn classificar_com_tabela(
    transcricao: &str,
    tabela: &TabelaPalavras,
    limiar: f32,
) -> Decisao {
    // `.to_lowercase()` normaliza para comparar sem se importar com maiúsculas.
    let texto = transcricao.to_lowercase();
    // Tokens para o casamento POR PALAVRA INTEIRA (ver abaixo).
    let tokens = tokenizar(transcricao);

    // Acumuladores que o loop abaixo preenche.
    let mut score = 0.0f32; // f32 com sufixo: tipo explícito no literal
    let mut encontradas: Vec<String> = Vec::new();

    // `for (palavra, peso) in ...` DESESTRUTURA cada tupla do Vec
    // diretamente nas variáveis palavra/peso.
    for (palavra, peso) in &tabela.entradas {
        // CASAMENTO EM DOIS MODOS:
        //
        //   PALAVRA simples (só letras/números) → casa com TOKEN EXATO.
        //     "comprar" não conta "compra"; "posso" não conta "pos";
        //     "mostrar" não conta "mostra" — sem dupla contagem de variantes.
        //   FRASE (qualquer entry com separador: espaço, hífen...) →
        //     substring no texto normalizado, pois frases já são específicas
        //     ("frete gratis", "à vista", "pronta-entrega", "fechar o pedido").
        let casou = if palavra.chars().any(|c| !c.is_alphanumeric()) {
            texto.contains(palavra.as_str())
        } else {
            tokens.iter().any(|t| t == palavra)
        };

        if casou {
            score += peso;
            // `.clone()` cria uma String própria (ownership nova) para
            // guardar no Vec — a `palavra` original continua na tabela.
            encontradas.push(palavra.clone());
        }
    }

    // Normaliza pelo top-3 da tabela e trava (clamp) contra passar de 1.0.
    let confianca = (score / tabela.score_referencia()).clamp(0.0, 1.0);

    // EVIDÊNCIA MÍNIMA: score alto E palavras distintas suficientes.
    // `encontradas` já é uma lista sem repetições (cada entrada da tabela
    // conta no máximo uma vez), então `.len()` é o nº de distintas.
    let evidencia_suficiente = encontradas.len() >= MIN_DISTINTAS;

    let decisao = Decisao {
        e_venda: confianca >= limiar && evidencia_suficiente,
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
        // Na tabela (espelho do txt) as entradas são palavras: "fechar" e
        // "pedido" batem por palavra inteira, junto com "preço" e "desconto"
        // → 4 distintas, confiança 11.5/9 = 100%.
        assert!(d.palavras_chave.contains(&"fechar".to_string()));
        assert!(d.palavras_chave.contains(&"pedido".to_string()));
        assert!(d.palavras_chave.contains(&"preço".to_string()));
    }

    #[test]
    fn conversa_comum_nao_e_venda() {
        let d = classificar("Oi, tudo bem? Só passando para desejar um bom dia!");
        assert!(!d.e_venda);
        assert!(d.palavras_chave.is_empty());
    }

    #[test]
    fn conversa_real_nao_e_venda() {
        // REGRESSÃO DO CASO REAL: transcrição (Vosk) do
        // datasets/golden/conversa-normal.wav — explicação sobre controlador
        // de medicação. Antes dava "VENDA 100%" por substring + genéricas.
        let conversa = "uma coisa assim o controle usei o caso da mãe toma médico a pressão \
            tomar médico para que isso tem umas quantidade tantas horas que toma uma caixinha \
            dura trinta dias nesses trinta dias quando chegar ali no entanto uma semana \
            determinado sistema tem que mostrar a lei está terminando para ser providenciado \
            mudar e que entre a questão do disparar pois a pessoa que responsável pelo idoso \
            também para comprar providenciar e para elas tem o controle que está terminando \
            uma semana que vem com eles controlam a pessoa tomar medicação medo dia caixinha \
            tem trinta anos sessenta vai durar tanto tempo margens de controle da medicação \
            é a não precisa fazer nada assim muito complicado pelas em simples mas que eu \
            botei aquelas coisas das cores quando tá tudo normal vizinho quando falta e \
            vamos e quinze na amarelinho quando falta uma semana ele passa para ver vermelho \
            é só pra chamar a atenção é";
        let d = classificar(conversa);
        assert!(
            !d.e_venda,
            "conversa comum não pode virar venda; palavras: {:?}, confiança: {}",
            d.palavras_chave, d.confianca
        );
    }

    #[test]
    fn casa_por_palavra_inteira_nao_substring() {
        // "pos" (peso alto!) NÃO pode ser achado dentro de "posso";
        // "meta" NÃO dentro de "metade". Só o token EXATO conta.
        let d = classificar("eu posso te ajudar depois da metade do mês");
        assert!(d.palavras_chave.iter().all(|p| p != "pos" && p != "meta"));
        // E a variante não conta a raiz: "comprar" ≠ "compra".
        let d2 = classificar("vamos comprar pão");
        assert!(!d2.palavras_chave.contains(&"compra".to_string()));
        assert!(d2.palavras_chave.contains(&"comprar".to_string()));
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

    #[test]
    fn limiar_customizado_muda_o_veredito() {
        // Tabela própria: 3 palavras fortes (ref = 3+3+3 = 9) e 1 fraca.
        // Texto com a,b,d → score 7/9 ≈ 0.778 e EXATAMENTE 3 distintas
        // (passa na regra de evidência mínima por pouco!).
        let tabela = TabelaPalavras {
            entradas: vec![
                ("a".to_string(), 3.0),
                ("b".to_string(), 3.0),
                ("c".to_string(), 3.0),
                ("d".to_string(), 1.0),
            ],
            fonte: "teste",
        };
        let texto = "tem a, b e d aqui";
        let d1 = classificar_com_tabela(texto, &tabela, 0.70);
        assert!(d1.e_venda, "0.778 >= 0.70 com 3 distintas");
        let d2 = classificar_com_tabela(texto, &tabela, 0.80);
        assert!(!d2.e_venda, "0.778 < 0.80");
        // A confiança em si não depende do limiar — só a decisão.
        assert!((d1.confianca - d2.confianca).abs() < 1e-6);
    }

    #[test]
    fn evidencia_minima_bloqueia_palavra_so() {
        // Score alto com UMA palavra só: confiança 100%, mas 1 < MIN_DISTINTAS
        // → NÃO é venda. É exatamente o caso que gerava falso positivo.
        let tabela = TabelaPalavras {
            entradas: vec![("desconto".to_string(), 3.0)],
            fonte: "teste",
        };
        let d = classificar_com_tabela("tem desconto sim", &tabela, 0.5);
        assert!((d.confianca - 1.0).abs() < 1e-6, "score bateu 100%");
        assert!(!d.e_venda, "mas 1 palavra distinta não basta");
    }

    #[test]
    fn tabela_dinamica_normaliza_pela_propria_tabela() {
        // Tabela mínima com 3 palavras de peso 3.0: top-3 = 9.0 e o texto
        // cita as três → confiança 100% — a normalização segue a tabela
        // em uso, não uma constante fixa.
        let tabela = TabelaPalavras {
            entradas: vec![
                ("preço".to_string(), 3.0),
                ("desconto".to_string(), 3.0),
                ("parcela".to_string(), 3.0),
            ],
            fonte: "teste",
        };
        let d = classificar_com_tabela("qual o preço, com desconto e parcela?", &tabela, 0.70);
        assert!(d.e_venda);
        assert!((d.confianca - 1.0).abs() < 1e-6);

        // Texto sem NENHUMA palavra da tabela → confiança zero.
        let d2 = classificar_com_tabela("bom dia tudo bem", &tabela, 0.70);
        assert!(!d2.e_venda);
        assert_eq!(d2.confianca, 0.0);
    }
}
