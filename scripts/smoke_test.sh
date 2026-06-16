#!/usr/bin/env bash
# smoke_test.sh — offline single-turn smoke test
# Usage: ./scripts/smoke_test.sh
set -euo pipefail

echo "=== g10kz v5 smoke test ==="

# Offline once mode with mock provider
reply=$(LLM_PROVIDER=mock \
        CARGO_TARGET_DIR=/tmp/g10kz-target \
        cargo run -q -p g10kz-bot -- once "你好" 2>/dev/null)

echo "reply: $reply"

if [[ -z "$reply" ]]; then
    echo "FAIL: empty reply"
    exit 1
fi

echo "PASS"
