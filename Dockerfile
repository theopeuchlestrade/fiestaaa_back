FROM rust:1.90.0-bookworm AS builder

LABEL org.opencontainers.image.source="https://github.com/theopeuchlestrade/fiestaaa_back"

# Build deps for sqlx/postgres and native-tls consumers such as reqwest.
RUN apt-get update \
 && apt-get install -y --no-install-recommends build-essential pkg-config libssl-dev ca-certificates \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Prime dependency compilation for faster rebuilds.
COPY Cargo.toml Cargo.lock build.rs ./
COPY migrations ./migrations
RUN mkdir -p src && echo "fn main(){}" > src/main.rs \
 && cargo build --release --locked || true \
 && rm -rf src

# Real source
COPY . .

RUN cargo build --release --locked

FROM debian:bookworm-slim AS runtime

LABEL org.opencontainers.image.source="https://github.com/theopeuchlestrade/fiestaaa_back"

RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl libssl3 \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /app/target/release/fiestaaa_back /usr/local/bin/fiestaaa_back

ENV HOST=0.0.0.0 \
    PORT=8080

EXPOSE 8080

CMD ["fiestaaa_back"]
