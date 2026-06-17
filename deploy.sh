#!/bin/bash
# g10kz fast deploy: musl build (static, no glibc dep) + docker cp swap
# Usage: ./deploy.sh
set -e

source ~/.cargo/env

echo "=== building (musl static) ==="
cd ~/g10kz
cargo build --release --target x86_64-unknown-linux-musl -p g10kz-bot
BINARY=target/x86_64-unknown-linux-musl/release/g10kz-bot

echo "=== swapping binary (bot down) ==="
docker stop g10kz-bot
docker cp "$BINARY" g10kz-bot:/usr/local/bin/g10kz-bot
docker start g10kz-bot

echo "=== done. logs ==="
sleep 3
docker logs g10kz-bot --tail 8
