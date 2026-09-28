#!/usr/bin/env bash
# scripts/instalar_vosk.sh
#
# Baixa e instala, LOCALMENTE no projeto, tudo que a feature "vosk" precisa:
#   1. libvosk.so  → biblioteca nativa C do Vosk (linux-x86_64), vai para
#                    models/asr/lib/ e é usada no LINK do build (--features vosk)
#   2. modelo pt-BR vosk-model-small-pt-0.3 (31 MB, Apache 2.0) → models/asr/
#
# Uso:  ./scripts/instalar_vosk.sh
# Fontes (oficiais):
#   libvosk: https://github.com/alphacep/vosk-api/releases (tag v0.3.45)
#   modelo:  https://alphacephei.com/vosk/models
set -euo pipefail

# resolve o caminho do projeto a partir de onde o script está (não do cwd)
RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
DIR_LIB="$RAIZ/models/asr/lib"
DIR_MODELO="$RAIZ/models/asr"
MODELO="vosk-model-small-pt-0.3"
VERSAO="0.3.45"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT   # limpa a pasta temporária em QUALQUER saída

echo "==> Projeto: $RAIZ"

# ---------------------------------------------------------------- 1. libvosk
if [ -f "$DIR_LIB/libvosk.so" ]; then
    echo "==> libvosk.so já existe em models/asr/lib/ (pulando download)"
else
    URL_LIB="https://github.com/alphacep/vosk-api/releases/download/v${VERSAO}/vosk-linux-x86_64-${VERSAO}.zip"
    echo "==> Baixando libvosk.so (~7 MB): $URL_LIB"
    mkdir -p "$DIR_LIB"
    curl -fL --retry 3 "$URL_LIB" -o "$TMP/libvosk.zip"
    unzip -q -o "$TMP/libvosk.zip" -d "$TMP/libvosk"
    # o .so pode vir na raiz do zip ou dentro de uma subpasta — acha os dois casos
    SO="$(find "$TMP/libvosk" -name 'libvosk.so' -type f | head -n1)"
    if [ -z "$SO" ]; then
        echo "ERRO: libvosk.so não encontrado dentro do zip!" >&2
        exit 1
    fi
    cp "$SO" "$DIR_LIB/libvosk.so"
    echo "    instalado em models/asr/lib/libvosk.so"
fi

# ---------------------------------------------------------------- 2. modelo
if [ -d "$DIR_MODELO/$MODELO" ]; then
    echo "==> Modelo já existe em models/asr/$MODELO (pulando download)"
else
    URL_MODELO="https://alphacephei.com/vosk/models/${MODELO}.zip"
    echo "==> Baixando modelo pt-BR (~31 MB): $URL_MODELO"
    curl -fL --retry 3 "$URL_MODELO" -o "$TMP/modelo.zip"
    unzip -q -o "$TMP/modelo.zip" -d "$DIR_MODELO"
    echo "    instalado em models/asr/$MODELO/"
fi

echo
echo "==> Concluído! Agora teste com:"
echo "      cargo build --features vosk"
echo "      cargo run --features vosk -- datasets/golden/exemplo.wav --vosk"
