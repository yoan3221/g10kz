# g10kz-bot production image
# Uses pre-built musl static binary -- no Rust toolchain needed at image build time.
# To update: build the musl binary locally, copy to bin/g10kz-bot, then:
#   docker compose build && docker compose down && docker compose up -d
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Pre-built musl static binary (x86_64-unknown-linux-musl)
COPY bin/g10kz-bot /usr/local/bin/g10kz-bot

# Obscura headless browser for web search (pre-built binary)
COPY bin/obscura /usr/local/bin/obscura
COPY bin/obscura-worker /usr/local/bin/obscura-worker

ENTRYPOINT ["/usr/local/bin/g10kz-bot"]
CMD ["daemon"]
