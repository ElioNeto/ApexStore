# ApexStore - High-Performance LSM-Tree Key-Value Store
# Multi-stage Dockerfile for optimized production builds

FROM lukemathwalker/cargo-chef:latest-rust-1 AS chef
LABEL stage=builder
WORKDIR /app

# Stage 1: Prepare dependency recipe
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# Stage 2: Build dependencies (cached layer)
FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --features api --recipe-path recipe.json

# Build the application
COPY . .
RUN cargo build --release --bin apexstore-server --features api

# Stage 3: Runtime image
FROM debian:bookworm-slim AS runtime
LABEL org.opencontainers.image.authors="Elio Neto <netoo.elio@hotmail.com>"
LABEL org.opencontainers.image.description="ApexStore - High-performance LSM-Tree key-value store built with Rust"
LABEL org.opencontainers.image.source="https://github.com/elioneto/apexstore"
LABEL org.opencontainers.image.licenses="MIT"
LABEL org.opencontainers.image.version="2.2.7"

WORKDIR /app

# Install runtime dependencies
RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates curl && \
    rm -rf /var/lib/apt/lists/*

# Copy the compiled binary
COPY --from=builder /app/target/release/apexstore-server /app/apexstore-server

# Create data directory for persistence.
# NOTE: the server reads DATA_DIR (src/bin/server.rs), not DIR_PATH.
RUN mkdir -p /data
ENV DATA_DIR=/data
VOLUME ["/data"]

# Expose API port
EXPOSE 8080

# Health check
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD curl -fsS "http://localhost:${PORT:-8080}/health/liveness" || exit 1

# Run the server
CMD ["/app/apexstore-server"]
