# ── Stage 1: Build frontend WASM ─────────────────────────────────────────────
FROM rust:1.94-bookworm AS frontend-builder

RUN rustup target add wasm32-unknown-unknown

# Install pre-built trunk binary (avoids OOM from compiling trunk from source)
RUN set -eux; \
    ARCH=$(uname -m); \
    TRUNK_VERSION="0.21.14"; \
    curl -fsSL "https://github.com/trunk-rs/trunk/releases/download/v${TRUNK_VERSION}/trunk-${ARCH}-unknown-linux-gnu.tar.gz" \
      | tar xz -C /usr/local/bin; \
    chmod +x /usr/local/bin/trunk

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY backend/Cargo.toml ./backend/
COPY frontend/Cargo.toml ./frontend/

# Pre-fetch dependencies (cache layer)
RUN mkdir -p backend/src frontend/src && \
    echo 'fn main(){}' > backend/src/main.rs && \
    echo '' > frontend/src/lib.rs && \
    cargo fetch

COPY frontend/ ./frontend/
WORKDIR /app/frontend
RUN trunk build --release

# ── Stage 2: Build backend binary ────────────────────────────────────────────
FROM rust:1.94-bookworm AS backend-builder

RUN apt-get update && apt-get install -y pkg-config && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY backend/Cargo.toml ./backend/
COPY frontend/Cargo.toml ./frontend/

# Pre-fetch dependencies (cache layer)
RUN mkdir -p backend/src frontend/src && \
    echo 'fn main(){}' > backend/src/main.rs && \
    echo '' > frontend/src/lib.rs && \
    cargo fetch

COPY backend/ ./backend/
RUN cargo build --release --bin fetchly

# ── Stage 3: Final minimal image ─────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && \
    apt-get install -y ca-certificates && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=backend-builder /app/target/release/fetchly ./fetchly
COPY --from=frontend-builder /app/frontend/dist ./static

RUN mkdir -p /data && chmod 777 /data

EXPOSE 3001

ENV STATIC_DIR=/app/static
ENV DB_PATH=/data/fetchly.db

CMD ["/app/fetchly", "serve"]
