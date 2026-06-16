# Multi-stage build — slim final image.
FROM rust:1.82-slim AS builder

WORKDIR /build
# Cache dependencies
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/

# Build release binary
ENV CARGO_TARGET_DIR=/tmp/target
RUN cargo build --release -p g10kz-bot

# ── Runtime image ─────────────────────────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y \
    ca-certificates \
    ffmpeg \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /tmp/target/release/g10kz-bot /usr/local/bin/g10kz-bot

ENTRYPOINT ["/usr/local/bin/g10kz-bot"]
CMD ["daemon"]
