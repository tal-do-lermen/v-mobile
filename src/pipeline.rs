//! src/pipeline.rs
//!
//! AQUI VOCÊ APRENDE: composição — juntar os estágios em fluxos, escolher
//! fontes de dados (hound vs ffmpeg) e limitar subprocessos.
//!
//! Rotas de execução, todas terminando no mesmo `Relatorio`:
//!
//!   PADRÃO (arquivo): WAV 16 kHz → hound streaming ┐
//!                    outro formato → ffmpeg no pipe ┘→ [scan progressivo]
//!                                     (--threads ≥ 2 decodifica em paralelo)
//!   --stdin:          PCM cru do pipe ──────────────→ [scan sequencial]
//!   --completo:       WAV inteiro na RAM ───────────→ [4 estágios clássicos]
//!
//! O peso pesado (decode do Vosk) é a lib C — as vitórias aqui são
//! decodificar MENOS (VAD-gating, early exit) e menos RAM (streaming).

use crate::audio::AudioData;
use crate::cli::Cli;
use crate::error::{DetectorError, Resultado};
use crate::palavras_chave::{self, TabelaPalavras};
use crate::report::Relatorio;
use crate::scan::{self, FonteJanelas, FontePcm, FonteWav};
use crate::{classify, transcribe, vad};
use std::path::Path;
use std::process::{Child, Command, Stdio};

/// Executa o pipeline para os argumentos da CLI, escolhendo a rota.
pub fn executar(cli: &Cli) -> Resultado<Relatorio> {
    cli.validar()?;

    // A tabela de palavras-chave é carregada UMA vez (API → arquivo →
    // embutida) e emprestada para todas as rotas.
    let tabela = palavras_chave::carregar(cli.api_url.as_deref(), &cli.palavras);

    if cli.stdin {
        executar_stream(cli, &tabela)
    } else if cli.completo {
        executar_classico(cli, &tabela)
    } else {
        // Progressivo é o PADRÃO para arquivos (early exit é o maior
        // salvador em hardware fraco).
        executar_progressivo(cli, &tabela)
    }
}

/// ROTA CLÁSSICA (`--completo`) — o fluxo original de 4 estágios.
fn executar_classico(cli: &Cli, tabela: &TabelaPalavras) -> Resultado<Relatorio> {
    let caminho = cli.audio.as_ref().expect("validar() garante arquivo");
    tracing::info!(arquivo = %caminho.display(), "modo completo: carregando áudio inteiro");
    let audio = AudioData::carregar(caminho)?;
    tracing::info!(duracao_s = format!("{:.2}", audio.duracao_segundos()), "áudio carregado");

    tracing::info!("estágio 2/4: detectando fala (VAD)");
    let segmentos = vad::detectar_fala(&audio, cli.limiar);
    tracing::info!(segmentos = segmentos.len(), "segmentos de fala encontrados");

    tracing::info!("estágio 3/4: transcrevendo");
    let mut transcritor = transcribe::criar_transcritor(cli.vosk, &cli.modelo)?;
    let transcricao = transcritor.transcrever(&audio, &segmentos)?;

    tracing::info!("estágio 4/4: classificando");
    let decisao = classify::classificar_com_tabela(&transcricao, tabela, classify::LIMIAR_DECISAO);

    Ok(Relatorio {
        arquivo: caminho.display().to_string(),
        duracao_segundos: audio.duracao_segundos(),
        transcritor: transcritor.nome().to_string(),
        segmentos,
        transcricao,
        decisao,
        fonte_palavras: Some(descricao_da_tabela(tabela)),
        detectado_em_seg: None,
        janelas_analisadas: None,
        percentual_processado: None,
    })
}

/// ROTA PROGRESSIVA (padrão) — streaming + early exit + paralelismo opcional.
///
/// Decisão de FONTE: o cabeçalho do WAV é lido de graça (hound). Se o
/// arquivo já é 16 kHz mono PCM16, o hound streama direto (puro Rust);
/// senão, o ffmpeg reamostra no pipe (`-f s16le -ar 16000 -ac 1`), e o
/// mesmo scan consome o PCM cru — RAM constante nos dois casos.
fn executar_progressivo(cli: &Cli, tabela: &TabelaPalavras) -> Resultado<Relatorio> {
    let caminho = cli.audio.as_ref().expect("validar() garante arquivo");
    tracing::info!(
        arquivo = %caminho.display(),
        janela_s = cli.janela,
        threads = cli.threads,
        "modo progressivo (padrão)"
    );
    let cfg = cli.config_scan();
    let mut transcritor = transcribe::criar_transcritor(cli.vosk, &cli.modelo)?;
    let nome = transcritor.nome().to_string();

    // --velocidade > 1.0 força o caminho do ffmpeg MESMO em arquivos 16 kHz:
    // o atempo precisa passar pelo filtro antes de virar PCM pra gente.
    let usa_ffmpeg = cli.velocidade != 1.0 || !wav_16k_mono(caminho)?;
    let resultado = if !usa_ffmpeg {
        let mut fonte = FonteWav::abrir(caminho, cli.janela)?;
        escanear(cli, &mut fonte, &mut transcritor, &cfg, tabela)?
    } else {
        if cli.velocidade != 1.0 {
            tracing::warn!(
                velocidade = cli.velocidade,
                "EXPERIMENTAL: áudio acelerado — a transcrição degrada e vendas podem NÃO ser detectadas"
            );
        } else {
            tracing::info!("formato não é 16 kHz mono PCM16 — reamostrando via ffmpeg (pipe)");
        }
        let mut filho = spawn_ffmpeg(caminho, cli.velocidade)?;
        // `stdout.take()` move o pipe para nós; o Child continua dono do processo.
        let stdout = filho.stdout.take().expect("stdout configurado como piped");
        let mut fonte = FontePcm::novo(stdout, cli.janela, scan::TAXA_STREAM);
        let r = escanear(cli, &mut fonte, &mut transcritor, &cfg, tabela);
        // Se saímos cedo (early exit), o pipe fecha e o ffmpeg morre sozinho;
        // o kill/wait explícito não deixa zumbi em nenhum caso.
        let _ = filho.kill();
        let _ = filho.wait();
        r?
    };

    Ok(relatorio_de_scan(cli, &nome, resultado, tabela))
}

/// ROTA STREAM (--stdin) — PCM cru ao vivo, sempre sequencial (a stream
/// não tem "futuro" para decodificar em paralelo).
fn executar_stream(cli: &Cli, tabela: &TabelaPalavras) -> Resultado<Relatorio> {
    tracing::info!(
        taxa = scan::TAXA_STREAM,
        "modo stream: aguardando PCM i16 LE mono no stdin (Ctrl+D encerra)"
    );
    if cli.velocidade != 1.0 {
        tracing::warn!("--velocidade é IGNORADO em --stdin (a stream já chega pronta)");
    }
    let mut transcritor = transcribe::criar_transcritor(cli.vosk, &cli.modelo)?;
    let nome = transcritor.nome().to_string();

    let mut stdin = std::io::stdin().lock();
    let mut fonte = FontePcm::novo(&mut stdin, cli.janela, scan::TAXA_STREAM);
    let resultado = scan::escanear_sequencial(&mut fonte, transcritor.as_mut(), &cli.config_scan(), tabela)?;

    Ok(relatorio_de_scan(cli, &nome, resultado, tabela))
}

/// Sequential vs paralelo, conforme `--threads` (paralelo só faz sentido
/// para arquivos; a rota stream chama o sequencial direto).
fn escanear(
    cli: &Cli,
    fonte: &mut dyn FonteJanelas,
    transcritor: &mut Box<dyn transcribe::Transcritor>,
    cfg: &scan::ConfigScan,
    tabela: &TabelaPalavras,
) -> Resultado<scan::ResultadoScan> {
    if cli.threads > 1 {
        scan::escanear_paralelo(fonte, transcritor.as_ref(), cfg, tabela, cli.threads)
    } else {
        scan::escanear_sequencial(fonte, transcritor.as_mut(), cfg, tabela)
    }
}

/// O arquivo é WAV 16 kHz mono PCM16? (leitura de cabeçalho — barata)
fn wav_16k_mono(caminho: &Path) -> Resultado<bool> {
    let leitor = hound::WavReader::open(caminho)?;
    let spec = leitor.spec();
    Ok(spec.channels == 1
        && spec.bits_per_sample == 16
        && spec.sample_format == hound::SampleFormat::Int
        && spec.sample_rate == scan::TAXA_STREAM)
}

/// Sobe o ffmpeg convertendo para o formato canônico (PCM s16le 16 kHz mono
/// no stdout) e, se `velocidade != 1.0`, aplica o filtro `atempo` ANTES —
/// menos amostras chegam ao decoder (mais rápido, porém menos preciso).
fn spawn_ffmpeg(caminho: &Path, velocidade: f32) -> Resultado<Child> {
    let mut cmd = Command::new("ffmpeg");
    cmd.arg("-i").arg(caminho.display().to_string());
    // O filtro vem antes das opções de formato: ffmpeg aplica -filter:a
    // ao áudio decodificado e só então reamostra/empacota.
    if velocidade != 1.0 {
        cmd.args(["-filter:a", &format!("atempo={velocidade}")]);
    }
    cmd.args(["-f", "s16le", "-ac", "1", "-ar", &scan::TAXA_STREAM.to_string(), "-"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null()) // logs do ffmpeg poluiriam nossa saída
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                DetectorError::FerramentaExterna(
                    "ffmpeg não encontrado no PATH. Instale-o (ou converta o áudio para \
                     WAV 16 kHz mono: ffmpeg -i in.wav -ar 16000 -ac 1 out.wav)"
                        .to_string(),
                )
            } else {
                DetectorError::Io(e)
            }
        })
}

/// Texto curto que identifica a fonte da tabela no relatório.
fn descricao_da_tabela(tabela: &TabelaPalavras) -> String {
    format!("{} ({} termos)", tabela.fonte, tabela.entradas.len())
}

/// Converte o resultado do escaneador no relatório oficial (JSON/terminal).
/// Recebe o `ResultadoScan` por MOVEDORIA — zero clones no caminho quente.
fn relatorio_de_scan(
    cli: &Cli,
    nome_transcritor: &str,
    resultado: scan::ResultadoScan,
    tabela: &TabelaPalavras,
) -> Relatorio {
    // Cálculos feitos ANTES da movedoria (os campos mudam de dono depois).
    let percentual = resultado.percentual_processado();
    let detectado_em_seg = resultado.detectado_em_seg;
    let janelas = resultado.janelas_analisadas;
    let duracao = resultado.duracao_total_seg;
    let arquivo = if cli.stdin {
        "stdin".to_string()
    } else {
        cli.audio
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default()
    };

    Relatorio {
        arquivo,
        duracao_segundos: duracao,
        transcritor: nome_transcritor.to_string(),
        segmentos: resultado.segmentos,
        transcricao: resultado.transcricao,
        decisao: resultado.decisao,
        fonte_palavras: Some(descricao_da_tabela(tabela)),
        detectado_em_seg,
        janelas_analisadas: Some(janelas),
        percentual_processado: Some(percentual),
    }
}
