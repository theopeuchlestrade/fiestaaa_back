# Pinned Rust builder base. Rustup keeps the exact project toolchain selected.
FROM rust:1.97.1-bookworm@sha256:14bc9c5966e7b3a385794b3d5389a8765668342025fbcc7b2e3d2866ac4bd8c3 AS builder

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
FROM debian:bookworm-slim@sha256:abd67ffcfa541b485a3dff59865ab629aa048a6c613e639d36e7456b0b229241 AS runtime

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
