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
        audio: caminho.clone(),
        saida: None,
        limiar: 0.02,
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
}
