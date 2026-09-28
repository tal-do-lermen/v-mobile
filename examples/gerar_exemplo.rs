//! examples/gerar_exemplo.rs
//!
//! Gera um WAV sintético em datasets/golden/exemplo.wav para você testar
//! o app sem precisar de nenhum áudio real.
//!
//! Rode com:  cargo run --example gerar_exemplo
//!
//! AQUI VOCÊ APRENDE: exemplos (`examples/`) são binários extras do projeto —
//! ótimos para demos e scripts utilitários. `Box<dyn Error>` no main é o
//! "erro genérico" rápido para código de exemplo.

use hound::{SampleFormat, WavSpec, WavWriter};
use std::f32::consts::PI;

const TAXA: u32 = 16000;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Especificação do WAV: mono, 16 bits, 16 kHz (o formato que o app aceita).
    let spec = WavSpec {
        channels: 1,
        sample_rate: TAXA,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };

    let caminho = "datasets/golden/exemplo.wav";
    let mut w = WavWriter::create(caminho, spec)?;

    // "Conversa" simulada: silêncio → fala 1 → pausa → fala 2 → silêncio.
    // (o padrão que o VAD deve encontrar: 2 segmentos)
    escrever_silencio(&mut w, 500)?; // 0,5 s de silêncio inicial
    escrever_fala(&mut w, 440.0, 2500)?; // "fala" 1: tom modulado de 2,5 s
    escrever_silencio(&mut w, 400)?; // pausa de 0,4 s
    escrever_fala(&mut w, 620.0, 3000)?; // "fala" 2: tom modulado de 3 s
    escrever_silencio(&mut w, 600)?; // silêncio final

    // `finalize` atualiza os cabeçalhos do WAV com o tamanho real.
    w.finalize()?;
    println!("Gerado: {caminho} (~7,0 s)");
    Ok(())
}

/// Escreve `dur_ms` milissegundos de silêncio (amostras zeradas).
fn escrever_silencio(w: &mut WavWriter<std::io::BufWriter<std::fs::File>>, dur_ms: u32) -> hound::Result<()> {
    for _ in 0..TAXA * dur_ms / 1000 {
        w.write_sample(0i16)?;
    }
    Ok(())
}

/// Escreve `dur_ms` ms de um tom senoidal com envelope de amplitude
/// (a "voz" sobe e desce de volume, imitando o padrão da fala).
fn escrever_fala(w: &mut WavWriter<std::io::BufWriter<std::fs::File>>, freq: f32, dur_ms: u32) -> hound::Result<()> {
    let n = (TAXA * dur_ms / 1000) as usize;
    for i in 0..n {
        let t = i as f32 / TAXA as f32;
        // Envelope: 0,55 + 0,45 * seno lento (2,5 Hz) → volume variando.
        let envelope = 0.55 + 0.45 * (2.0 * PI * 2.5 * t).sin();
        // Senoide pura na frequência dada, multiplicada pelo envelope.
        let valor = envelope * (2.0 * PI * freq * t).sin() * 0.8;
        // Volta para i16 (o formato do WAV de 16 bits).
        w.write_sample((valor * 32767.0) as i16)?;
    }
    Ok(())
}
