# Pinned Rust builder base. Rustup keeps the exact project toolchain selected.
FROM rust:1.97.0-bookworm@sha256:8fa55b2f3ddf97471ab6a767bfa3f37e6bad0986ba823e75fea57e2a2a5c3073 AS builder

ARG RUST_VERSION=1.96.0

LABEL org.opencontainers.image.source="https://github.com/theopeuchlestrade/fiestaaa_back"

RUN rustup toolchain install "$RUST_VERSION" --profile minimal \
 && rustup default "$RUST_VERSION"

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

# Pinned Debian runtime image for deterministic production serving (bookworm-slim)
FROM debian:bookworm-slim@sha256:7b140f374b289a7c2befc338f42ebe6441b7ea838a042bbd5acbfca6ec875818 AS runtime

LABEL org.opencontainers.image.source="https://github.com/theopeuchlestrade/fiestaaa_back"

RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl libgnutls30 libssl3 \
 && rm -rf /var/lib/apt/lists/* \
 && groupadd --system --gid 10001 fiestaaa \
 && useradd --system --uid 10001 --gid fiestaaa --home-dir /app --shell /usr/sbin/nologin fiestaaa

WORKDIR /app

COPY --from=builder /app/target/release/fiestaaa_back /usr/local/bin/fiestaaa_back

ENV HOST=0.0.0.0 \
    PORT=8080

EXPOSE 8080

USER 10001:10001

CMD ["fiestaaa_back"]
