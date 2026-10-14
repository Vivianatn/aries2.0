#!/usr/bin/env bash
# Télécharge les modèles GGUF dans ./models (reprend un téléchargement interrompu).
#   bash deploy/download-models.sh
# Autre LLM : MODEL_REPO=... MODEL_FILE=... bash deploy/download-models.sh
set -euo pipefail

MODEL_REPO="${MODEL_REPO:-Qwen/Qwen3-8B-GGUF}"
MODEL_FILE="${MODEL_FILE:-Qwen3-8B-Q4_K_M.gguf}"
EMBEDDING_REPO="${EMBEDDING_REPO:-ggml-org/bge-m3-Q8_0-GGUF}"
EMBEDDING_FILE="${EMBEDDING_FILE:-bge-m3-q8_0.gguf}"
DEST="$(cd "$(dirname "$0")/.." && pwd)/models"

download() {
  local repo="$1" file="$2"
  if [[ -f "$DEST/$file" ]]; then
    echo "$file est déjà présent."
    return
  fi
  curl -fL --retry 3 -C - -o "$DEST/$file.part" "https://huggingface.co/$repo/resolve/main/$file"
  mv "$DEST/$file.part" "$DEST/$file"
  echo "Modèle prêt : $DEST/$file"
}

mkdir -p "$DEST"
download "$MODEL_REPO" "$MODEL_FILE"
download "$EMBEDDING_REPO" "$EMBEDDING_FILE"
