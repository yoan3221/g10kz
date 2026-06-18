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
- 滑動視窗歷史：Social 路徑最多 20 條、Reason 路徑 12 條，防止 token 無限增長

**輸出格式**
- 顏文字 BM25 替換：模型輸出 `[kaomoji:關鍵字]` 佔位符，Rust BM25 引擎從 705 個顏文字資料庫選出最匹配的（支援繁→簡轉換）
- 動作描述自動轉換：`> 動作描述` 格式直接渲染成 Discord blockquote
- Prompt 語義去重 + prefix-cache 靜態/動態分離，每輪 system token −45%

**工具**
- 網路搜索：Obscura 防偵測瀏覽器 + DuckDuckGo Lite + BM25 相關段落萃取
- 台股即時報價、台灣時間

---

## 快速開始

### 需求

- Docker + Docker Compose
- Discord Bot Token
- OpenAI 相容 LLM API（OpenRouter / new-api / 其他）

### 啟動

```bash
cp .env.example .env
# 填入 DISCORD_TOKEN、LLM_API_KEY 等

docker compose up -d --build
docker logs g10kz-bot -f
```

### 快速更新部署

程式碼修改後：

```bash
# 前置：裝一次即可
rustup target add x86_64-unknown-linux-musl
sudo apt-get install -y musl-tools

# 每次更新
./deploy.sh
```

musl 靜態編譯 + binary swap，停機約 10 秒。

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

LLM_MODEL_SOCIAL=               # 日常對話（Social / Search）
LLM_MODEL_REASON=               # 深度推理（Reason 工具迴圈）
LLM_MODEL_JUDGE=                # Fusion judge

# Fusion drafter 列表（逗號分隔，≥2 才啟用；留空退化為單模型）
LLM_FUSION_DRAFTERS=

# ── 記憶 ─────────────────────────────────────────────
EVEROS_URL=http://localhost:8000   # 留空則用 NullMemory
EMBED_SERVER_URL=http://localhost:8082  # llama.cpp embedding server

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

### Crate 分層

```
L5  g10kz-bot      主 binary，daemon（長跑）與 once（單次測試）兩種模式
L4  g10kz-discord  Serenity 0.12 閘道；判斷哪些訊息需要回應、附件抽取、slash commands
L3  g10kz-engine   一回合狀態機；串接所有 L0–L2 組件，實作各路徑執行邏輯
L2  g10kz-everos   EverOS HTTP 客戶端；add_turn / flush / search，失敗自動降級
L2  g10kz-tools    ToolBox 介面 + 工具實作（WebSearch / TwStock / Time / Escalate）
L1  g10kz-llm      OpenAI 相容 HTTP 客戶端；FusionProvider；MockProvider
L1  g10kz-kernel   路由 / guard / JPAF / kaomoji BM25 / sanitize / persona 載入
L0  g10kz-config   型別化設定，無任何外部依賴
```

依賴方向由下往上，無反向耦合。

---

### Discord 閘道過濾

```
Discord 事件
  ├─ DM（私訊）               → ✓ 進入管線
  ├─ @mention（群組 @bot）    → ✓ 進入管線
  ├─ reply to bot（回覆機器人）→ ✓ 進入管線
  └─ 其他群組訊息              → 存入 ring buffer（作為語境背景，不回應）
```

---

### 一回合處理管線

```
[discord] 訊息進入
    │
    ▼
[kernel]  guard::pre_guard()          owner 直通 / 黑名單丟棄 / 注入偵測
    │
    ▼
[kernel]  normalize()                 去除 @mention、解析回覆鏈
    │
    ▼
[everos]  search()                    語意搜尋歷史，失敗靜默降級
    │
    ▼
[engine]  system_message() 組裝
          ├─ 靜態部分（角色卡 / 頻道說明 / 格式速查 / 工具 schema）
          └─ 動態部分（guild/頻道名稱 / JPAF modifier）
    │
    ▼
[kernel]  route()                     決定路徑（見路由表）
    │
    ├─ Social  → [llm] 單次 social model，history 最多 20 條
    ├─ Search  → [tools] WebSearch → [llm] social model 整合
    ├─ Reason  → [llm] FusionProvider + 工具迴圈，history 最多 12 條
    ├─ Media   → 附件 URL → Reason 路徑
    └─ Command → 直接處理，0 LLM 呼叫
    │
    ▼
[kernel]  sanitize()
          strip_artefact → collapse_blank_lines
          → actions_to_blockquote（*動作* / _動作_ / > 動作 → Discord blockquote）
          → replace_kaomoji_markers（BM25 解析 [kaomoji:關鍵字]）
          → 超過 2000 字自動分段
    │
    ▼
[everos]  add_turn() + flush()        失敗只記 WARN
    │
    ▼
[kernel]  JPAF::update()              bump/decay 8 個認知函式分數
    │
    ▼
[discord] SSE 串流漸進編輯發送
```

---

### 路由決策

| 優先 | 觸發條件 | 路徑 | LLM 呼叫 |
|---|---|---|---|
| 1 | 已知指令前綴 | Command | 0 次 |
| 2 | 有附件 | Media | Reason 路徑 |
| 3 | 搜尋觸發詞 | Search | 1 次 + 工具 |
| 4 | 複雜度訊號（長文 / 多問號 / 程式碼 / 分析詞） | Reason | N drafter + judge |
| 5 | 其餘 | Social | 1 次 |

---

### Kaomoji BM25 系統

模型輸出 `[kaomoji:害羞,臉紅]` 格式的佔位符，sanitize pipeline 在最後階段用 BM25 從 705 個顏文字的資料庫中選出最匹配的：

```
[kaomoji:害羞,臉紅]  →  (//ω//)
[kaomoji:開心]       →  (≧▽≦)
```

- 關鍵字支援繁→簡轉換（`trad_to_simp()`），確保匹配資料庫的簡體標籤
- BM25 TF-IDF 加權，多關鍵字時取最高分的顏文字

---

### Fusion 多模型（Reason 路徑）

```
[drafter A] ──┐
[drafter B] ──┼──→ Jaccard 共識過濾 ──→ judge 合成最終回覆
[drafter C] ──┘

drafter < 2 或全部失敗 → 自動退化為單模型
```

---

### EverOS 記憶整合

[EverOS](https://github.com/EverMind-AI/EverOS) HTTP API，embedding 後端為 `llama-embed`（llama.cpp，Qwen3-Embedding-0.6B，1024-dim）：

| 端點 | 時機 |
|---|---|
| `POST /api/v1/memory/search` | 每輪開始前 |
| `POST /api/v1/memory/add` | 每輪結束後 |
| `POST /api/v1/memory/flush` | add 之後 |

任一端點失敗只記 WARN，bot 繼續正常運作。

---

## 部署拓樸

```
REDACTED
├─ g10kz-bot        (network_mode: host)
│    → new-api   localhost:3000   LLM 閘道（OpenAI 相容）
│    → everos    localhost:8000   記憶 sidecar
│    → llama-embed localhost:8082 embedding（語意路由）
│
├─ everos / llama-embed  :8000/:8082  g10kz-memory stack
├─ new-api               :3000        LLM 閘道（Gemini / Anthropic / OpenRouter）
├─ cloudflared                        api.g8kz.top → new-api:3000
└─ geminicli2api         :8080/:8888  new-api 的 Gemini CLI 後端
```

`network_mode: host`：bot 直接共享宿主機網路介面，`localhost:3000/8000/8082` 即可連通。

---

## CI

GitHub Actions 每次 push main 自動執行：

1. `cargo fmt --all -- --check`
2. `cargo clippy --workspace --exclude g10kz-discord --exclude g10kz-bot -- -D warnings`
3. `cargo test --workspace --exclude g10kz-discord --exclude g10kz-bot`
4. `LLM_PROVIDER=mock cargo run -p g10kz-bot -- once "你好小十"`（smoke test）

---

## 開發注意事項

**Windows 掛載 null byte 問題**：透過 Windows-mounted 路徑修改的 Rust / TOML 檔案可能附帶 trailing null bytes，導致 `cargo` 解析失敗。所有原始碼修改必須透過 paramiko SFTP，不可使用本地 Edit/Write 工具。

**Rust 字串**：中文直接用字面 UTF-8，不用 `\u{XXXX}`，避免手誤寫成 Python 風格的 `\uXXXX`。

---

## License

MIT
