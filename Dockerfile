# Pinned Rust builder base. Rustup keeps the exact project toolchain selected.
FROM rust:1.99.1-bookworm@sha256:48b7f435b8fc6378ce43b44f15e3a47fd8919b2825f835c39bd4bd3da50af5b8 AS builder

ARG RUST_VERSION=1.99.1

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
COPY vendor ./vendor
COPY migrations ./migrations
RUN mkdir -p src && echo "fn main(){}" > src/main.rs \
 && cargo build --release --locked || true \
 && rm -rf src

# Real source
COPY . .

RUN cargo build --release --locked

# Pinned Debian runtime image for deterministic production serving (bookworm-slim)
FROM debian:bookworm-slim@sha256:7ff6ba31527b9e336162fa061746c366c4b2032f4d2cff2078bea43f78efe3f8 AS runtime

LABEL org.opencontainers.image.source="https://github.com/theopeuchlestrade/fiestaaa_back"

RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl libgnutls30 libpcre2-8-0=10.42-1+deb12u2 libssl3 perl-base=5.36.0-7+deb12u4 \
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
