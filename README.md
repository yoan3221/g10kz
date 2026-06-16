# g10kz

> 傲嬌 AI Discord bot，以 Rust 構建。

小十（g10kz）是一個具備多模型推理、長期記憶、工具迴圈與主動發言能力的 Discord AI bot，採用 8-crate Rust workspace 架構，接入 [OpenRouter](https://openrouter.ai/) 相容 API。

---

## 特色

- **Fusion 多模型合成** — 多個 drafter 並行出稿，Jaccard 共識判斷是否需要 judge 仲裁
- **EverOS 記憶 sidecar** — 向量語意記憶，跨對話持久化
- **工具迴圈** — 時間、台股即時、Cloudflare AI Search 網路搜尋、人工升級
- **主動發言** — 頻道閒置超過閾值後自動打招呼
- **per-channel 滑動視窗** — 30 條訊息的對話 ring buffer
- **Slash commands** — `/reset` `/stop` `/memory` `/persona` `/trace`
- **傲嬌人格** — 透過 YAML persona card 定義角色

---

## 架構

```
g10kz-config   (L0)  env / config 載入
├── g10kz-kernel  (L1)  normalize · guard · route · persona
├── g10kz-llm     (L1)  OpenRouter provider · Fusion
│   ├── g10kz-everos  (L2)  EverOS memory sidecar client
│   ├── g10kz-tools   (L2)  time · stock · web-search · escalate
│   └── g10kz-engine  (L3)  turn orchestrator
│       └── g10kz-discord (L4)  serenity gateway · slash commands
│           └── g10kz-bot (L5)  daemon / once entry point
```

---

## 快速開始

### 需求

- Rust 1.82+
- Docker（部署用）
- Discord bot token（需開啟 `MESSAGE_CONTENT` Privileged Intent）
- OpenRouter 相容 API key

### 環境變數

複製 `.env.example`（需自行建立）或直接設定：

```env
DISCORD_TOKEN=your_discord_bot_token
OWNER_USER_ID=your_discord_user_id

LLM_PROVIDER=openrouter
LLM_BASE_URL=https://openrouter.ai/api/v1
LLM_API_KEY=your_api_key

LLM_MODEL_SOCIAL=openai/gpt-4o-mini
LLM_MODEL_REASON=openai/gpt-4o
LLM_MODEL_JUDGE=openai/gpt-4o-mini
LLM_FUSION_DRAFTERS=openai/gpt-4o,openai/gpt-4o-mini

# 選填
EVEROS_URL=http://localhost:8000
CF_ACCOUNT_ID=
CF_API_TOKEN=
PERSONA_CARD_PATH=
PROACTIVE_INACTIVE_SECS=86400
RUST_LOG=g10kz=info,warn
```

### 本地開發

```bash
# 離線單輪測試（mock provider）
LLM_PROVIDER=mock cargo run -p g10kz-bot -- once 你好

# 執行完整測試
cargo test --workspace
```

### Docker 部署

```bash
# 建置 image
docker compose build

# 啟動
docker compose up -d

# 查看 log
docker compose logs -f bot
```

`docker-compose.yml` 預設使用 `network_mode: host`，方便 bot 存取同機的 EverOS（`localhost:8000`）。

---

## Slash Commands

| 指令 | 說明 |
|---|---|
| `/reset` | 清除當前頻道的對話記憶 |
| `/stop` | 中止正在處理的回覆 |
| `/memory <query>` | 搜尋長期記憶 |
| `/persona` | 重新載入 persona card |
| `/trace` | 切換 debug trace 模式 |

---

## 觸發條件

Bot 只在以下情況回應（不監聽所有頻道訊息）：

- **私訊（DM）**
- **@mention** bot
- **回覆** bot 的訊息

> ⚠️ 需在 [Discord Developer Portal](https://discord.com/developers/applications) 開啟 **MESSAGE CONTENT** Privileged Intent

---

## 授權

MIT
