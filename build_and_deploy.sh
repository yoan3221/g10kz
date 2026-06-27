#!/usr/bin/env bash
# 在 .127 本地建置並部署 g10kz-bot
# 用法：ssh g8kz@REDACTED bash build_and_deploy.sh
set -e

SRC_DIR="$HOME/g10kz-src"
DEPLOY_DIR="$HOME/g10kz"
TARGET="x86_64-unknown-linux-musl"

echo "=== [1/4] cargo build (musl) ==="
source ~/.cargo/env
cd "$SRC_DIR"
cargo build -p g10kz-bot --release --target "$TARGET"

echo "=== [2/4] 複製 binary 到 bin/ ==="
cp "target/$TARGET/release/g10kz-bot" "$DEPLOY_DIR/bin/g10kz-bot"
ls -lh "$DEPLOY_DIR/bin/g10kz-bot"

echo "=== [3/4] docker compose build ==="
cd "$DEPLOY_DIR"
docker compose build

echo "=== [4/4] 重啟 bot ==="
docker compose down
docker compose up -d
sleep 3
docker compose logs --tail=10

echo ""
echo "=== 部署完成 ==="
