# Mondoshawan Blockchain Node - Dockerfile
# Base images are pinned to specific versions for reproducible builds:
# - rust:1.92.0-slim-bookworm (Rust 1.92.0 on Debian Bookworm)
# - debian:bookworm-20240904-slim (Debian Bookworm dated image)
# To update: check https://hub.docker.com/_/rust and https://hub.docker.com/_/debian for new tags
FROM rust:1.92.0-slim-bookworm AS builder

WORKDIR /app

# Install build dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    libssl-dev \
    pkg-config \
    protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*

# Copy source code, Cargo.toml, and Cargo.lock
COPY mondoshawan-blockchain/Cargo.toml mondoshawan-blockchain/Cargo.toml
COPY mondoshawan-blockchain/Cargo.lock mondoshawan-blockchain/Cargo.lock
COPY mondoshawan-blockchain/src mondoshawan-blockchain/src
COPY mondoshawan-blockchain/benches mondoshawan-blockchain/benches
COPY mondoshawan-blockchain/examples mondoshawan-blockchain/examples
COPY mondoshawan-blockchain/build.rs mondoshawan-blockchain/build.rs
COPY mondoshawan-blockchain/proto mondoshawan-blockchain/proto

# Build the node
WORKDIR /app/mondoshawan-blockchain
RUN cargo build --release --features kyber

# Runtime stage - using dated Debian tag for reproducibility
FROM debian:bookworm-20240904-slim

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy binary from builder
COPY --from=builder /app/mondoshawan-blockchain/target/release/node /app/node

# Create data directory
RUN mkdir -p /data

# Expose ports
EXPOSE 8080 8545 9090

# Create non-root user for security
RUN addgroup --system mondoshawan && adduser --system --ingroup mondoshawan mondoshawan
RUN chown -R mondoshawan:mondoshawan /app /data
USER mondoshawan

# Default command
CMD ["./node"]
