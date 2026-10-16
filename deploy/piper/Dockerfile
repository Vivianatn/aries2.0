# Synthèse vocale Piper (piper1-gpl, GPL-3.0) : service séparé, jamais lié à ArIès.
# La voix (.onnx + .onnx.json) est montée depuis ./models (deploy/download-models.sh).
FROM python:3.12-slim
RUN pip install --no-cache-dir "piper-tts[http]==1.8.0" \
    && useradd --system --no-create-home piper
USER piper
WORKDIR /tmp
EXPOSE 5000
ENTRYPOINT ["python", "-m", "piper.http_server", "--host", "0.0.0.0", "--port", "5000"]
