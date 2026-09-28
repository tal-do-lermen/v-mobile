# venda-detector

App CLI em Rust que analisa um áudio `.wav` e decide se contém uma **ligação de venda**.

```
.wav ──▶ [audio] ──▶ [vad] ──▶ [transcribe] ──▶ [classify] ──▶ relatorio.json
         decodifica   corta      fala → texto    venda? confiança?
```

Repositório: `git@github.com:tal-do-lermen/v-mobile.git`

## Requisitos

| Ferramenta | Para quê | Obrigatório? |
|---|---|---|
| Rust 1.75+ (cargo) | compilar o app | sim |
| `curl` + `unzip` | script de instalação do Vosk | só para a feature `vosk` |
| `arecord` (alsa-utils) | gravar do microfone | só para o script de gravação |

## Como usar (modo estudo — sem IA)

```bash
# 1. gera um áudio de exemplo (sintético, sem precisar de gravação real)
cargo run --example gerar_exemplo

# 2. roda o detector com o transcritor mock (padrão, sem IA)
cargo run -- datasets/golden/exemplo.wav --saida relatorio.json

# 3. logs detalhados de cada estágio
cargo run -- --verboso datasets/golden/exemplo.wav
```

## Transcrição real com Vosk (offline, gratuito, Apache 2.0)

O Vosk é opcional e fica atrás de uma **feature flag** — o build padrão nem
o contém. Tudo fica LOCAL no projeto (nada é instalado no sistema).

```bash
# 1. baixa libvosk.so (7 MB) + modelo pt-BR (31 MB) para models/asr/
./scripts/instalar_vosk.sh

# 2. compila e roda com a transcrição real
cargo build --features vosk
cargo run --features vosk -- audio.wav --vosk
```

**Áudio sintético não transcreve nada** (tons não são fala — a transcrição
vem vazia). Para testar com fala de verdade, duas opções:

### Opção A — gravar pelo microfone (script automático)

```bash
./scripts/gravar_e_detectar.sh          # grava 10 s e analisa na hora
./scripts/gravar_e_detectar.sh 15       # grava 15 s
```

O script detecta o microfone automaticamente, grava no formato certo
(16 kHz/mono/16 bits) e roda o pipeline completo com `--vosk`.

### Opção B — amostras de fala real já incluídas

`datasets/voz_real/` tem 3 gravações reais de um falante brasileiro
(fonte: [VoxForge PT](http://www.repository.voxforge1.org/downloads/pt/),
licença aberta):

```bash
cargo run --features vosk -- datasets/voz_real/161.wav --vosk
# Transcrição: "eu sou do brasil"
```

Limitação conhecida: o modelo `small-pt` (31 MB) tem acurácia modesta em
áudio telefônico brasileiro [WER 68,9% no CORAA]. Para produção, considere
o modelo grande pt (1,6 GB) ou um serviço de nuvem.

## Estrutura do projeto

```
venda-detector/
├── src/                     ← o app (leia nesta ordem!)
│   ├── lib.rs               entrada da biblioteca (pub mod)
│   ├── error.rs             enum de erros (thiserror, Result, ?)
│   ├── cli.rs               argumentos (clap derive)
│   ├── audio.rs             estágio 1: carrega WAV (hound)
│   ├── vad.rs               estágio 2: detecta fala (RMS, 30 ms)
│   ├── transcribe.rs        estágio 3: trait Transcritor + mock + Vosk
│   ├── classify.rs          estágio 4: palavras-chave + confiança
│   ├── report.rs            relatório JSON (serde)
│   ├── pipeline.rs          orquestra os 4 estágios (tracing)
│   └── main.rs              binário: CLI + saída amigável
├── tests/pipeline.rs        teste de integração (fluxo completo)
├── examples/gerar_exemplo.rs  gera datasets/golden/exemplo.wav
├── scripts/
│   ├── instalar_vosk.sh     baixa libvosk.so + modelo pt-BR (local)
│   └── gravar_e_detectar.sh grava do microfone e analisa
├── datasets/
│   ├── golden/              áudio sintético de referência
│   └── voz_real/            3 amostras de fala PT-BR real (VoxForge)
├── models/asr/              libvosk.so + modelo (gitignored — baixe com o script)
├── docs/guia-interativo.html  simulador do pipeline + guia de Rust (abra no navegador)
└── .cargo/config.toml       linker/rpath da lib nativa do Vosk
```

## Ordem de estudo recomendada

Leia os arquivos nesta ordem — cada um ensina conceitos novos, comentados linha a linha:

| # | Arquivo | Conceitos |
|---|---------|-----------|
| 1 | `src/error.rs` | `Result`, `?`, `thiserror`, `#[from]` |
| 2 | `src/cli.rs` | structs, `#[derive]`, clap, `Option` |
| 3 | `src/audio.rs` | `Vec`, slices, ownership, iteradores, casts |
| 4 | `src/vad.rs` | `chunks`, `filter_map`, `match` com `Option`, RMS |
| 5 | `src/transcribe.rs` | **traits**, `Box<dyn>`, `#[cfg(feature)]`, `map_err` |
| 6 | `src/classify.rs` | tabelas estáticas, desestruturação, `clamp` |
| 7 | `src/report.rs` | serde, JSON, conversão de múltiplos erros |
| 8 | `src/pipeline.rs` | composição, `tracing`, fluxo de dados |
| 9 | `src/main.rs` | `match` em `Result`, `if let`, saída amigável |

Extras: `src/lib.rs` (o que é lib vs bin), `examples/gerar_exemplo.rs` (binários de exemplo),
`tests/pipeline.rs` (teste de integração), `scripts/instalar_vosk.sh` (bash didático),
`.cargo/config.toml` (linker e rpath para a lib nativa).

## Estudar com o guia interativo

Abra `docs/guia-interativo.html` no navegador (não precisa de servidor nem do Rust):
simulador animado do pipeline + guia de conceitos com quizzes.

## Comandos úteis

```bash
cargo test                              # roda os 13 testes (mock, sem vosk)
cargo test --features vosk              # 14 testes (inclui o teste real do vosk)
cargo clippy                            # linter
RUST_LOG=debug cargo run -- ...         # logs via env var
```

## Licenças e créditos

| Componente | Licença |
|---|---|
| Código deste projeto | uso de estudo |
| [Vosk](https://github.com/alphacep/vosk-api) / `libvosk.so` | Apache 2.0 |
| Modelo `vosk-model-small-pt-0.3` | Apache 2.0 ([alphacephei.com/vosk/models](https://alphacephei.com/vosk/models)) |
| Amostras `datasets/voz_real/` | [VoxForge PT](http://www.repository.voxforge1.org/downloads/pt/) |
| Crates: clap, serde, thiserror, tracing, hound, vosk | MIT/Apache 2.0 |
