#!/usr/bin/env bash
# WSL 上執行：build g10kz-bot image，scp 到 .94，重啟 bot
set -e

SERVER=""
PASS=""
REPO="https://github.com/yoan3221/g10kz.git"
IMAGE="g10kz-bot:latest"
TAR="/tmp/g10kz-bot-latest.tar"
SSH_OPTS="-o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null"

echo "=== [1/5] 拉最新代碼 ==="
rm -rf /tmp/g10kz-build
git clone --depth=1 "$REPO" /tmp/g10kz-build
cd /tmp/g10kz-build

echo "=== [2/5] 建立 Docker image ==="
docker build -f Dockerfile -t "$IMAGE" .

echo "=== [3/5] 匯出成 tar ==="
docker save "$IMAGE" -o "$TAR"
echo "tar size: $(du -sh $TAR | cut -f1)"

echo "=== [4/5] scp 傳到 .94 ==="
sshpass -p "$PASS" scp $SSH_OPTS "$TAR" "${SERVER}:/tmp/g10kz-bot-latest.tar"

echo "=== [5/5] 伺服器 load + restart ==="
sshpass -p "$PASS" ssh $SSH_OPTS "$SERVER" '
  docker load -i /tmp/g10kz-bot-latest.tar &&
  cd ~/g10kz &&
  docker compose down 2>/dev/null || true &&
  docker compose up -d --no-build &&
  sleep 4 &&
  docker logs g10kz-bot --tail 10 2>&1
'

echo ""
echo "=== 完成 ==="
