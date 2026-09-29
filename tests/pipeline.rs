//! tests/pipeline.rs — TESTE DE INTEGRAÇÃO
//!
//! AQUI VOCÊ APRENDE: a diferença entre teste unitário e integração.
//!   - Unitário (dentro dos módulos): testa uma função isolada.
//!   - Integração (esta pasta): testa o app "de fora", como um usuário
//!     da biblioteca — usando só o que é `pub` em lib.rs.
//!
//! Fluxo testado: gera um WAV temporário → roda o pipeline completo →
//! confere o relatório → grava JSON. Tudo sem áudio real.

use hound::{SampleFormat, WavSpec, WavWriter};
use std::path::PathBuf;
use venda_detector::cli::Cli;
use venda_detector::pipeline;
/// Cria um WAV temporário igualzinho ao do exemplo golden.
fn criar_wav(caminho: &PathBuf) {
    let spec = WavSpec {
        channels: 1,
        sample_rate: 16000,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };
    let mut w = WavWriter::create(caminho, spec).unwrap();
    // silêncio 0,5 s | tom 1 s | silêncio 0,5 s → 1 segmento esperado
    for _ in 0..8000 {
        w.write_sample(0i16).unwrap();
    }
    for i in 0..16000 {
        let t = i as f32 / 16000.0;
        let v = (0.5 * (2.0 * std::f32::consts::PI * 440.0 * t).sin() * 32767.0) as i16;
        w.write_sample(v).unwrap();
    }
    for _ in 0..8000 {
        w.write_sample(0i16).unwrap();
    }
    w.finalize().unwrap();
}

#[test]
fn pipeline_completo_produz_relatorio_de_venda() {
    // Prepara o áudio de entrada num diretório temporário do sistema.
    let caminho = std::env::temp_dir().join("vd_pipeline_teste.wav");
    criar_wav(&caminho);

    // Monta a CLI "na mão" (como se o usuário tivesse digitado os args).
    let cli = Cli {
        audio: Some(caminho.clone()),
        saida: None,
        limiar: 0.02,
        stdin: false,
        progressivo: false,
        completo: true,
        threads: 1,
        velocidade: 1.0,
        janela: 30.0,
        limiar_confianca: 0.70,
        api_url: None,
        palavras: PathBuf::from("../palavras-chaves/palavras_chaves.txt"),
        vosk: false, // teste usa o mock: determinístico, sem IA
        modelo: PathBuf::from("models/asr/modelo.bin"),
        verboso: false,
    };

    // Roda o pipeline INTEIRO — o coração deste teste.
    let relatorio = pipeline::executar(&cli).expect("pipeline deveria ter sucesso");

    // O mock sempre responde com o texto de venda → decisões previsíveis:
    assert_eq!(relatorio.transcritor, "mock");
    assert!(!relatorio.segmentos.is_empty(), "VAD deveria achar a fala");
    assert!(relatorio.decisao.e_venda, "mock fala texto de venda");
    assert!(relatorio.decisao.confianca >= 0.5);

    // Bônus: valida também a serialização JSON do relatório.
    let json = serde_json::to_string(&relatorio).unwrap();
    assert!(json.contains("\"e_venda\":true"));
    // Modo clássico: campos progressivos nem existem no JSON.
    assert!(!json.contains("detectado_em_seg"));
}

#[test]
fn pipeline_progressivo_detecta_e_para_antecipado() {
    // Mesmo WAV do teste clássico (2 s), mas com janelas de 1 s:
    // o mock "ouve" o tom já na 1ª janela → para em 1 s, sem ver o resto.
    let caminho = std::env::temp_dir().join("vd_pipeline_progressivo.wav");
    criar_wav(&caminho);

    let cli = Cli {
        audio: Some(caminho),
        saida: None,
        limiar: 0.02,
        stdin: false,
        progressivo: true,
        completo: false,
        threads: 1,
        velocidade: 1.0,
        janela: 1.0,
        limiar_confianca: 0.70,
        api_url: None,
        palavras: PathBuf::from("../palavras-chaves/palavras_chaves.txt"),
        vosk: false,
        modelo: PathBuf::from("models/asr/modelo.bin"),
        verboso: false,
    };

    let relatorio = pipeline::executar(&cli).expect("pipeline deveria ter sucesso");

    assert!(relatorio.decisao.e_venda);
    assert_eq!(relatorio.detectado_em_seg, Some(1.0));
    assert_eq!(relatorio.janelas_analisadas, Some(1));
    // Processou 1 s dos 2 s do arquivo = 50%.
    assert_eq!(relatorio.percentual_processado, Some(50.0));

    let json = serde_json::to_string(&relatorio).unwrap();
    assert!(json.contains("\"detectado_em_seg\":1.0"));
}

#[test]
fn pipeline_rejeita_fonte_ambigua() {
    // Nem arquivo nem --stdin: validar() precisa barrar com erro claro.
    let cli = Cli {
        audio: None,
        saida: None,
        limiar: 0.02,
        stdin: false,
        progressivo: false,
        completo: false,
        threads: 1,
        velocidade: 1.0,
        janela: 30.0,
        limiar_confianca: 0.70,
        api_url: None,
        palavras: PathBuf::from("../palavras-chaves/palavras_chaves.txt"),
        vosk: false,
        modelo: PathBuf::from("models/asr/modelo.bin"),
        verboso: false,
    };
    let resultado = pipeline::executar(&cli);
    assert!(matches!(resultado, Err(venda_detector::error::DetectorError::Argumento(_))));
}

#[test]
fn progressivo_e_padrao_para_arquivos_sem_flags() {
    // SEM --progressivo e SEM --completo: o progressivo é o padrão
    // (com o padrão novo de --threads = 2 → decodificação por batch).
    let caminho = std::env::temp_dir().join("vd_pipeline_padrao.wav");
    criar_wav(&caminho);

    let cli = Cli {
        audio: Some(caminho),
        saida: None,
        limiar: 0.02,
        stdin: false,
        progressivo: false,
        completo: false,
        threads: 2,
        velocidade: 1.0,
        janela: 1.0,
        limiar_confianca: 0.70,
        api_url: None,
        palavras: PathBuf::from("../palavras-chaves/palavras_chaves.txt"),
        vosk: false,
        modelo: PathBuf::from("models/asr/modelo.bin"),
        verboso: false,
    };

    let relatorio = pipeline::executar(&cli).expect("pipeline deveria ter sucesso");

    assert!(relatorio.decisao.e_venda);
    // threads=2 → o 1º batch decodifica as 2 janelas do áudio de 2 s
    // e o checkpoint do batch detecta: granularidade = 2 janelas.
    assert_eq!(relatorio.detectado_em_seg, Some(2.0));
    assert_eq!(relatorio.janelas_analisadas, Some(2));
    assert_eq!(relatorio.percentual_processado, Some(100.0));
}
