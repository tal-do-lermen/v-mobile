#!/usr/bin/env bash
# scripts/gravar_e_detectar.sh
#
# Grava do MICROFONE e roda o detector completo com transcrição Vosk real.
#
# Uso:
#   ./scripts/gravar_e_detectar.sh                 # grava 10 s e analisa
#   ./scripts/gravar_e_detectar.sh 15              # grava 15 s
#   ./scripts/gravar_e_detectar.sh 10 minha.wav    # grava em outro caminho
#   ARECORD_DEV="hw:1,0" ./scripts/gravar_e_detectar.sh   # escolhe o dispositivo
#
# Requisitos: arecord (alsa-utils), feature vosk compilável
# (rode ./scripts/instalar_vosk.sh antes, na primeira vez).
set -euo pipefail

RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
cd "$RAIZ"

# 1º argumento: duração em segundos (padrão 10)
DURACAO="${1:-10}"
# 2º argumento: onde salvar o WAV (padrão dentro de datasets/voz_real/)
SAIDA="${2:-datasets/voz_real/minha_gravacao.wav}"
# Variável de ambiente opcional para escolher o microfone (ex.: hw:1,0).
# Se não for definida, detecta automaticamente o 1º cartão de captura.
DEV="${ARECORD_DEV:-}"
if [ -z "$DEV" ]; then
    # arecord -l lista cartões de captura; extraímos o número do 1º ("card N")
    CARTAO="$(arecord -l 2>/dev/null | grep -oE '^card [0-9]+' | head -n1 | grep -oE '[0-9]+$')"
    if [ -n "$CARTAO" ]; then
        # plughw (e não hw) = deixa o ALSA converter formato/taxa quando preciso
        DEV="plughw:${CARTAO},0"
        echo "Microfone detectado: $DEV (force outro com ARECORD_DEV=hw:X,0)"
    else
        echo "Nenhum microfone encontrado (arecord -l vazio)." >&2
        exit 1
    fi
fi

# Confirma que o modelo do Vosk está instalado antes de a pessoa falar à toa
if [ ! -d "models/asr/vosk-model-small-pt-0.3" ]; then
    echo "Modelo do Vosk não encontrado. Rode antes: ./scripts/instalar_vosk.sh" >&2
    exit 1
fi

mkdir -p "$(dirname "$SAIDA")"

# -f S16_LE -r 16000 -c 1 = PCM 16 bits, 16 kHz, mono — exatamente o formato
# que src/audio.rs espera. Mesma spec do arecord no README.
CMD=(arecord -f S16_LE -r 16000 -c 1 -d "$DURACAO")
if [ -n "$DEV" ]; then
    CMD+=(-D "$DEV")
fi
CMD+=("$SAIDA")

echo "=================================================================="
echo " GRAVANDO ${DURACAO}s → ${SAIDA}"
echo " FALE AGORA (ex.: 'quero fechar o pedido hoje, qual o preço?')"
echo "=================================================================="
"${CMD[@]}"
echo " Gravação concluída. Analisando..."
echo

# O pipeline completo: áudio → VAD → Vosk → classificador → relatório
cargo run --features vosk --quiet -- "$SAIDA" --vosk --saida relatorio.json
echo
echo "Relatório JSON em relatorio.json"
