FROM rust:1-slim as dev

LABEL org.opencontainers.image.source="https://github.com/theopeuchlestrade/fiestaaa_back"

# Install build deps for sqlx/postgres (OpenSSL)
RUN apt-get update \
 && apt-get install -y --no-install-recommends build-essential pkg-config libssl-dev \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Cache deps
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src && echo "fn main(){}" > src/main.rs \
 && cargo build --release || true

# Real source
COPY . .

# Default port
EXPOSE 8080

# Run the dev server (migrations run at startup)
CMD ["cargo", "run"]
