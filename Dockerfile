# Multi-stage build — slim final image.
# Builder: bookworm-based Rust (glibc 2.36) matches runtime.
FROM rust:slim-bookworm AS builder

WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/

ENV CARGO_TARGET_DIR=/tmp/target
RUN cargo build --release -p g10kz-bot

FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y     ca-certificates     ffmpeg     && rm -rf /var/lib/apt/lists/*

COPY --from=builder /tmp/target/release/g10kz-bot /usr/local/bin/g10kz-bot

# Obscura headless browser for web search (pre-built binary)
COPY bin/obscura /usr/local/bin/obscura
COPY bin/obscura-worker /usr/local/bin/obscura-worker

ENTRYPOINT ["/usr/local/bin/g10kz-bot"]
CMD ["daemon"]
