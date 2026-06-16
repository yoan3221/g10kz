# Multi-stage build — slim final image.
FROM rust:latest AS builder

WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/

ENV CARGO_TARGET_DIR=/tmp/target
RUN cargo build --release -p g10kz-bot

FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y \
    ca-certificates \
    ffmpeg \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /tmp/target/release/g10kz-bot /usr/local/bin/g10kz-bot

ENTRYPOINT ["/usr/local/bin/g10kz-bot"]
CMD ["daemon"]
