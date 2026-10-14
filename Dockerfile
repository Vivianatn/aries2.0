FROM rust:1-bookworm AS builder
WORKDIR /src
COPY . .
RUN cargo build --release --bin aries

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates tzdata \
    && rm -rf /var/lib/apt/lists/*
ENV TZ=Europe/Paris
WORKDIR /app
COPY --from=builder /src/target/release/aries /usr/local/bin/aries
COPY config ./config
COPY web ./web
VOLUME /app/data
EXPOSE 3050
CMD ["aries"]
