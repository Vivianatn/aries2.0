# Reconnaissance vocale (whisper.cpp, MIT), compilée pour le VPS : AVX2 sans AVX-512.
# L'image officielle `ghcr.io/ggml-org/whisper.cpp:main` utilise des instructions absentes
# de ce processeur (arrêt immédiat, code 132 « instruction illégale »).
FROM debian:bookworm-slim AS build
ARG WHISPER_VERSION=v1.9.4
RUN apt-get update \
    && apt-get install -y --no-install-recommends build-essential cmake git ca-certificates \
    && rm -rf /var/lib/apt/lists/*
RUN git clone --depth 1 --branch "$WHISPER_VERSION" https://github.com/ggml-org/whisper.cpp /src
WORKDIR /src
RUN cmake -B build -DCMAKE_BUILD_TYPE=Release -DBUILD_SHARED_LIBS=OFF \
        -DGGML_NATIVE=OFF -DGGML_AVX=ON -DGGML_AVX2=ON -DGGML_FMA=ON -DGGML_F16C=ON \
        -DGGML_AVX512=OFF -DWHISPER_BUILD_SERVER=ON -DWHISPER_BUILD_TESTS=OFF \
    && cmake --build build --target whisper-server -j "$(nproc)"

FROM debian:bookworm-slim
# ffmpeg : `--convert` lit le WebM/Opus ou le MP4 enregistré par le navigateur.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ffmpeg libgomp1 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --no-create-home whisper
COPY --from=build /src/build/bin/whisper-server /usr/local/bin/whisper-server
USER whisper
WORKDIR /tmp
EXPOSE 8080
ENTRYPOINT ["whisper-server"]
