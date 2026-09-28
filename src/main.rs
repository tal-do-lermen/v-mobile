//! src/main.rs — o BINÁRIO.
//!
//! AQUI VOCÊ APRENDE: como um main fino organiza tudo.
//!
//! Fluxo: parsear CLI → configurar logs → rodar pipeline → apresentar
//! resultado (ou erro amigável). Toda a lógica mora na LIB (lib.rs).

use clap::Parser; // trait que habilita Cli::parse()
use tracing_subscriber::EnvFilter;
use venda_detector::cli::Cli;
use venda_detector::pipeline;

fn main() {
    // 1) Parseia os argumentos da linha de comando.
    //    Se algo estiver errado (falta arquivo, valor inválido), o clap
    //    imprime a ajuda e ENCERRA o processo automaticamente — por isso
    //    `.parse()` "nunca falha": ou devolve Cli ou já sai.
    let cli = Cli::parse();

    // 2) Configura o sistema de logs.
    //    - RUST_LOG=debug cargo run ... controla o nível por variável de ambiente
    //    - sem RUST_LOG: "debug" se --verboso, senão "info"
    let filtro_padrao = if cli.verboso { "debug" } else { "info" };
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new(filtro_padrao)),
        )
        .init();

    // 3) Roda o pipeline. `match` é a forma idiomática de tratar Result:
    //    um braço para Ok, outro para Err — o compilador OBRIGA a tratar os dois.
    match pipeline::executar(&cli) {
        Ok(relatorio) => {
            imprimir_resumo(&relatorio);

            // 4) Se o usuário pediu --saida, grava o JSON.
            //    `if let` desempacota a Option<PathBuf> SEM precisar de match:
            //    "se tem Some(saida), usa; se None, só ignora este bloco".
            if let Some(saida) = &cli.saida {
                // Aqui sim tratamos o Result com match: falhar ao gravar
                // precisa ser reportado.
                match relatorio.salvar(saida) {
                    Ok(()) => println!("Relatório salvo em: {}", saida.display()),
                    Err(e) => {
                        eprintln!("Erro ao salvar relatório: {e}");
                        // Código de saída != 0 sinaliza falha para scripts.
                        std::process::exit(1);
                    }
                }
            }
        }
        Err(e) => {
            // `{e}` usa o Display gerado pelo thiserror (mensagens legíveis).
            eprintln!("Erro: {e}");
            std::process::exit(1);
        }
    }
}

/// Imprime o resumo legível no terminal.
///
/// Recebe `&Relatorio` emprestado: só lê, não toma ownership — quem chama
/// (o main) ainda precisa do relatório para gravar o JSON.
fn imprimir_resumo(r: &venda_detector::report::Relatorio) {
    println!("\n========== RESULTADO ==========");
    println!("Arquivo:   {} ({:.1} s)", r.arquivo, r.duracao_segundos);
    println!("Transcritor: {}", r.transcritor);

    // Lista os segmentos encontrados pelo VAD.
    println!("Segmentos de fala: {}", r.segmentos.len());
    for s in &r.segmentos {
        // `{:.2}` = float com 2 casas; `–` entre tempos.
        println!("  [{:.2}s – {:.2}s]", s.inicio, s.fim);
    }

    println!("Transcrição: \"{}\"", r.transcricao);

    // Operador ternário não existe em Rust; use if/else como EXPRESSÃO
    // (que devolve um valor).
    let veredito = if r.decisao.e_venda { "VENDA ✔" } else { "não-venda ✘" };
    println!("Decisão:   {veredito} (confiança {:.0}%)", r.decisao.confianca * 100.0);
    println!("Palavras-chave: {}", r.decisao.palavras_chave.join(", "));
    println!("===============================\n");
}
