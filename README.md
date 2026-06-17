# g10kz

> 傲嬌 AI Discord 機器人 — 以 Rust 構建，具備長期記憶、多模型推理與人格自適應能力。

g10kz 是由 g8kz 創造的 18 歲原創角色，在 Discord 上以繁體中文與使用者自然互動。她表面嘴硬、愛逞強，內心其實超容易害羞、黏人 (//ω//)

---

## 特色

**人格**
- SillyTavern V2 角色卡驅動，可熱抽換角色
- JPAF 人格自適應框架：追蹤 8 個榮格認知函式（Fe/Ti/Ne/Si…），per-user 建模，讓角色對不同人產生不同的互動風格
- 伺服器 / 頻道感知，角色知道自己在哪個 Discord 伺服器與頻道

**對話能力**
- 五路由引擎：Social / Search / Reason / Media / Command，依訊息自動分流
- Fusion 多模型（Reason 路徑）：多個 drafter 並行 → 共識過濾 → judge 合成，回覆品質優於單模型
- EverOS 語意記憶：向量化長期記憶 sidecar，每輪自動 add/flush/search，掛掉自動降級

**工具**
- 網路搜索：Obscura 防偵測瀏覽器 + DuckDuckGo Lite + BM25 相關段落萃取
- 台股即時報價、台灣時間

**效能**
- Prompt 語義去重 + prefix-cache 靜態/動態分離，每輪 system token −45%，快取命中時等效 −89%
- 輸出 sanitize：提示注入防禦、speaker 標籤自動剝除、反重複偵測

---

## 快速開始

### 需求

- Docker + Docker Compose
- Discord Bot Token
- OpenAI 相容 LLM API（OpenRouter / new-api / 其他）

### 啟動

```bash
cp .env.example .env
# 填入 DISCORD_TOKEN、LLM_API_KEY 等（見下方說明）

docker compose up -d --build
docker logs g10kz-bot -f
```

### 快速更新部署（首選）

程式碼修改後，用 `deploy.sh` — musl 靜態編譯 + binary swap，**停機約 10 秒**：

```bash
# 前置：裝一次即可
rustup target add x86_64-unknown-linux-musl
sudo apt-get install -y musl-tools

# 每次更新只要
./deploy.sh
```

`deploy.sh` 產生完全靜態連結 binary（無 glibc 相依），可直接注入任何 Linux container。

### 本地測試（不需要 Discord）

```bash
LLM_PROVIDER=mock cargo run -p g10kz-bot -- once "你好，自我介紹一下"
```

---

## 設定

`.env.example` 複製為 `.env`，**不要 commit `.env`**。

```env
# ── Discord ──────────────────────────────────────────
DISCORD_TOKEN=          # Bot Token
OWNER_USER_ID=          # 你的 Discord 雪花 ID

# ── LLM ─────────────────────────────────────────────
LLM_PROVIDER=openrouter
LLM_BASE_URL=https://openrouter.ai/api/v1
LLM_API_KEY=

LLM_MODEL_SOCIAL=               # 日常對話路徑（Social / Search）
LLM_MODEL_REASON=               # 深度推理路徑（Reason 工具迴圈）
LLM_MODEL_JUDGE=                # Fusion judge（合成多個 drafter 結果）

# Fusion drafter 列表（逗號分隔，≥2 個才啟用 Fusion；留空退化為單模型）
LLM_FUSION_DRAFTERS=

# ── 記憶 ─────────────────────────────────────────────
EVEROS_URL=http://localhost:8000   # 留空則用 NullMemory

# ── 角色卡 ───────────────────────────────────────────
PERSONA_CARD_PATH=./persona/g10kz.json

# ── 工具 ─────────────────────────────────────────────
OBSCURA_PATH=/usr/local/bin/obscura  # 留空則僅用 DDG snippet

# ── 其他 ─────────────────────────────────────────────
PROACTIVE_INACTIVE_SECS=86400  # 主動發話閾值（秒）
BLACKLISTED_USERS=             # 逗號分隔的雪花 ID
RUST_LOG=g10kz=info,warn
```

---

## Slash Commands

| 指令 | 說明 |
|---|---|
| `/search <query>` | 強制網路搜索 |
| `/reset` | 清除頻道對話記憶 |
| `/stop` | 中斷當前回覆 |
| `/persona` | 顯示目前角色卡 |
| `/memory` | EverOS 記憶狀態（Owner） |
| `/trace` | 上一輪路由資訊（Owner） |
| `/help` | 指令清單 |

---

## 角色卡

`persona/` 目錄放 SillyTavern V2 格式的 JSON，只需填 `system_prompt`：

```json
{
  "spec": "chara_card_v2",
  "spec_version": "2.0",
  "data": {
    "name": "角色名",
    "system_prompt": "你是...",
    "first_mes": "第一句話",
    "mes_example": "<START>\n{{user}}: ...\n{{char}}: ...\n<END>"
  }
}
```

> `once` 模式使用內建 stub persona，角色卡僅在 daemon 模式生效。

---

## 架構

```
g10kz-bot        主 binary（daemon / once）
  └─ g10kz-discord    Serenity 0.12 閘道
       └─ g10kz-engine     turn 狀態機
            ├─ g10kz-everos     EverOS 記憶客戶端
            ├─ g10kz-tools      工具迴圈（WebSearch / TwStock / Time）
            ├─ g10kz-llm        LLM 供應層（OpenRouter / Fusion / Mock）
            ├─ g10kz-kernel     路由 / guard / JPAF / persona 載入
            └─ g10kz-config     環境變數型別化設定
```

### 一輪的生命週期

```
Discord 訊息
  → guard（提示注入防禦，0 LLM）
  → EverOS search（語意記憶注入）
  → route → Social / Search / Reason / Media / Command
  → sanitize（標籤剝除 / 反重複）
  → EverOS add + flush
  → JPAF update
  → Discord 發送（>2000 字自動切割）
```

### 路由

| 優先 | 觸發條件 | 路徑 |
|---|---|---|
| 1 | 已知指令前綴 | Command |
| 2 | 有附件 | Media |
| 3 | 搜尋關鍵詞 | Search |
| 4 | 長文 / 分析詞 / 多問號 | Reason |
| 5 | 其餘 | Social |

---

## 部署拓樸

```
REDACTED
├─ new-api       :3000   LLM 閘道
├─ everos        :8000   記憶 sidecar  ─┐
├─ llama-embed   :8082   向量化引擎    ─┤ g10kz-memory stack
└─ g10kz-bot     host    network_mode: host
```

---

## License

MIT
