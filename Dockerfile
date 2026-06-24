# IronDAG Blockchain Node - Dockerfile
FROM rust:slim-bookworm AS builder

WORKDIR /app

RUN apt-get update && apt-get install -y --no-install-recommends \
    libssl-dev \
    pkg-config \
    protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*

COPY irondag-blockchain/Cargo.toml irondag-blockchain/Cargo.toml
COPY irondag-blockchain/Cargo.lock irondag-blockchain/Cargo.lock
COPY irondag-blockchain/src irondag-blockchain/src
COPY irondag-blockchain/benches irondag-blockchain/benches
COPY irondag-blockchain/examples irondag-blockchain/examples
COPY irondag-blockchain/build.rs irondag-blockchain/build.rs
COPY irondag-blockchain/proto irondag-blockchain/proto

WORKDIR /app/irondag-blockchain
RUN cargo build --release --bin irondagd

FROM debian:bookworm-slim

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /app/irondag-blockchain/target/release/irondagd /app/irondagd

RUN mkdir -p /data

EXPOSE 8080 8546 9090

RUN addgroup --system irondag && adduser --system --ingroup irondag irondag
RUN chown -R irondag:irondag /app /data
USER irondag

ENTRYPOINT ["/app/irondagd"]
