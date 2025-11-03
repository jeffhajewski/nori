# Multi-stage build for NoriKV server
FROM rust:1.75-slim as builder

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Copy workspace files
COPY Cargo.toml Cargo.lock ./
COPY crates/ ./crates/
COPY apps/ ./apps/

# Build only the server binary (release mode for smaller image)
RUN cargo build --release -p norikv-server

# Runtime stage - minimal image
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Create norikv user
RUN useradd -m -u 1000 norikv

# Copy binary from builder
COPY --from=builder /build/target/release/norikv-server /usr/local/bin/

# Create data directory
RUN mkdir -p /var/lib/norikv && chown norikv:norikv /var/lib/norikv

# Switch to norikv user
USER norikv
WORKDIR /home/norikv

# Expose gRPC port
EXPOSE 7447

# Expose Prometheus metrics port
EXPOSE 9090

# Default command (config via environment or volume-mounted YAML)
CMD ["norikv-server"]
