#!/usr/bin/env bash
# Télécharge les modèles GGUF dans ./models (reprend un téléchargement interrompu).
#   bash deploy/download-models.sh
# Autre LLM : MODEL_REPO=... MODEL_FILE=... bash deploy/download-models.sh
set -euo pipefail

MODEL_REPO="${MODEL_REPO:-Qwen/Qwen3-8B-GGUF}"
MODEL_FILE="${MODEL_FILE:-Qwen3-8B-Q4_K_M.gguf}"
EMBEDDING_REPO="${EMBEDDING_REPO:-ggml-org/bge-m3-Q8_0-GGUF}"
EMBEDDING_FILE="${EMBEDDING_FILE:-bge-m3-q8_0.gguf}"
WHISPER_MODEL_FILE="${WHISPER_MODEL_FILE:-ggml-small-q5_1.bin}"
# Voix Piper : <langue>-<nom>-<qualité>, dans rhasspy/piper-voices/fr/fr_FR/<nom>/<qualité>/.
PIPER_VOICE="${PIPER_VOICE:-fr_FR-siwis-medium}"
DEST="$(cd "$(dirname "$0")/.." && pwd)/models"

# download <dépôt> <chemin dans le dépôt> : enregistré sous models/<nom du fichier>.
download() {
  local repo="$1" path="$2" file
  file="$(basename "$path")"
  if [[ -f "$DEST/$file" ]]; then
    echo "$file est déjà présent."
    return
  fi
  curl -fL --retry 3 -C - -o "$DEST/$file.part" "https://huggingface.co/$repo/resolve/main/$path"
  mv "$DEST/$file.part" "$DEST/$file"
  echo "Modèle prêt : $DEST/$file"
}

mkdir -p "$DEST"
download "$MODEL_REPO" "$MODEL_FILE"
download "$EMBEDDING_REPO" "$EMBEDDING_FILE"
download ggerganov/whisper.cpp "$WHISPER_MODEL_FILE"
IFS=- read -r _ voice_name voice_quality <<< "$PIPER_VOICE"
voice_dir="fr/fr_FR/$voice_name/$voice_quality"
download rhasspy/piper-voices "$voice_dir/$PIPER_VOICE.onnx"
download rhasspy/piper-voices "$voice_dir/$PIPER_VOICE.onnx.json"
