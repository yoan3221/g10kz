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
# 填入 DISCORD_TOKEN、LLM_API_KEY 等（見下方設定說明）

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

### Crate 分層

依賴方向由下往上，無反向耦合。每層只能依賴自己以下的層：

```
L5  g10kz-bot      主 binary，daemon（長跑）與 once（單次測試）兩種模式
L4  g10kz-discord  Serenity 0.12 閘道；判斷哪些訊息需要回應、附件抽取、slash commands 註冊
L3  g10kz-engine   一回合的狀態機；按順序串接所有 L0–L2 組件，實作各路徑執行邏輯
L2  g10kz-everos   EverOS 1.0 HTTP 客戶端；add_turn / flush / search，失敗自動降級
L2  g10kz-tools    ToolBox 介面 + 工具實作（WebSearch / TwStock / Time / Escalate）
L1  g10kz-llm      OpenAI 相容 HTTP 客戶端；FusionProvider（多 drafter + judge）；MockProvider
L1  g10kz-kernel   路由決策 / guard 防禦 / JPAF 人格 / normalize / sanitize / persona 載入
L0  g10kz-config   從環境變數載入的型別化設定，無任何外部依賴
```

---

### Discord 閘道過濾

`g10kz-discord` 收到所有 Discord 事件，但只有下列三種情況才進入處理管線，其餘靜默：

```
Discord 事件
  ├─ DM（私訊）               → ✓ 進入管線
  ├─ @mention（群組 @bot）    → ✓ 進入管線
  ├─ reply to bot（回覆機器人）→ ✓ 進入管線
  └─ 其他群組訊息              → 存入 ring buffer（作為語境背景，不回應）
```

附件（圖片 / 音訊 / 影片）在這層抽出 URL，隨訊息一起往下傳。

---

### 一回合處理管線

訊息進入管線後，依序通過以下階段。每個階段標註負責的 crate 與失敗行為：

```
[discord] 訊息進入
    │
    ▼
[kernel]  guard::pre_guard()
          純函式，0 LLM 成本
          owner → 直接通過
          黑名單用戶 → 丟棄
          注入關鍵詞偵測（normalize 後掃描，防全形/同形字繞過）→ 丟棄
    │
    ▼
[kernel]  normalize()
          去除 @mention 標記、解析回覆鏈、前綴解析
    │
    ▼
[everos]  search()                         ← 失敗時：靜默降級為 NullMemory，繼續
          語意搜尋歷史記憶，結果注入 system prompt
    │
    ▼
[engine]  system_message() 組裝
          ┌─ part 0（靜態，可 prefix-cache）─────────────────────────────┐
          │  角色卡 system_prompt                                         │
          │  [頻道語境] 說明（群組模式下：發話者標籤規則、注入防禦提醒）  │
          │  [Discord Markdown 速查]（格式語法參考）                      │
          │  [工具 schema]（僅 Reason 路徑）                              │
          └──────────────────────────────────────────────────────────────┘
          ┌─ part 1（動態，每輪變動，不快取）────────────────────────────┐
          │  [伺服器環境]（guild 名稱、頻道名稱）                        │
          │  [人格適應]（JPAF modifier，依用戶互動歷史動態生成）         │
          └──────────────────────────────────────────────────────────────┘
    │
    ▼
[kernel]  route()  →  決定路徑（見下方路由說明）
    │
    ├─ Social  → [llm] 單次 social model 呼叫
    ├─ Search  → [tools] WebSearchTool → [llm] social model 整合回覆
    ├─ Reason  → [llm] FusionProvider（N drafter 並行 → Jaccard 共識 → judge 合成）
    │            + 工具迴圈（tool_call → tool_result → 繼續，直到完成）
    ├─ Media   → 附件 URL 帶入 → Reason 路徑（ffmpeg 抽幀處理影片）
    └─ Command → 直接處理，0 LLM 呼叫
    │
    ▼
[kernel]  sanitize()
          剝除 LLM 可能輸出的發話者標籤（[名字]: 格式）
          超過 2000 字自動切割分段
    │
    ▼
[everos]  add_turn() + flush()             ← 失敗時：WARN log，不影響已發出的回覆
          本輪對話寫入向量記憶庫
    │
    ▼
[kernel]  JPAF::update()
          依本輪訊息特徵 bump/decay 8 個認知函式分數
          更新該用戶下一輪的人格 modifier
    │
    ▼
[discord] 發送回覆
```

---

### 路由決策

`g10kz-kernel/src/route.rs` 純函式，優先順序由上至下，第一個命中即走該路徑：

| 優先 | 觸發條件 | 路徑 | LLM 呼叫 | 設計動機 |
|---|---|---|---|---|
| 1 | 已知指令前綴（`/cmd` `!cmd`） | Command | 0 次 | 直接處理，免費 |
| 2 | 有附件 | Media | Reason 路徑 | 多模態輸入 |
| 3 | 搜尋觸發詞（搜尋/幫我查/股價…） | Search | 1 次 + 工具 | 需要即時資料 |
| 4 | 複雜度訊號（長文 >250 字 / 多問號 / 程式碼區塊 / 分析關鍵詞） | Reason | N drafter + judge | 品質優先 |
| 5 | 其餘 | Social | 1 次 | 最便宜，日常閒聊 |

---

### Fusion 多模型（Reason 路徑）

```
[drafter A] ──┐
[drafter B] ──┼──→ Jaccard 相似度共識過濾 ──→ judge model 合成最終回覆
[drafter C] ──┘

drafter < 2 或全部失敗 → 自動退化為單模型直答
```

- `LLM_FUSION_DRAFTERS`：逗號分隔的 drafter 模型清單
- `LLM_MODEL_JUDGE`：合成用的 judge 模型

---

### 網路搜索（Search 路徑）

```
用戶查詢
  → POST DuckDuckGo Lite HTML（https://lite.duckduckgo.com/lite/）
  → 解析前 5 筆 result-link / result-snippet
  → Obscura headless browser fetch 前 3 頁全文（防偵測，CDP 協定）
  → BM25 相關段落萃取（按查詢詞命中率排序）
  → 傳入 social model 整合成回覆
```

`OBSCURA_PATH` 未設或不存在時，自動降級為僅用 DDG snippets（無全文）。

---

### JPAF 人格適應

每位用戶維護 8 個榮格認知函式分數（Fe/Ti/Ne/Si/Te/Fi/Se/Ni），初始均等：

- 每輪依訊息特徵 **bump** 對應函式（情緒詞→Fe、邏輯詞→Ti、創意詞→Ne…）
- 所有分數每輪輕微 **decay**，自然回歸基線
- 主導函式（最高分）注入 `[人格適應]` modifier 至 system prompt 動態部分

---

### EverOS 記憶整合

使用 [EverOS 1.0](https://github.com/EverMind-AI/EverOS) HTTP API，embedding 後端為 `llama-embed`（llama.cpp server-cuda，Qwen3-Embedding-0.6B，1024-dim，~112ms/次）：

| 端點 | 時機 | 說明 |
|---|---|---|
| `POST /api/v1/memory/search` | 每輪開始前 | 語意搜尋，注入相關歷史 |
| `POST /api/v1/memory/add` | 每輪結束後 | 寫入本輪 user + assistant 訊息 |
| `POST /api/v1/memory/flush` | add 之後 | 觸發向量化與持久化 |

任一端點失敗只記 WARN，bot 繼續正常運作。

---

## 部署拓樸

```
REDACTED
│
├─ g10kz-bot        (network_mode: host)
│    直接用 localhost 存取同機服務：
│    → new-api  localhost:3000   LLM 閘道（OpenAI 相容，管理多後端 key）
│    → everos   localhost:8000   記憶 sidecar
│
├─ everos           :8000  ─┐
└─ llama-embed      :8082  ─┘  g10kz-memory stack（同一 bridge 網路）
                               everos 透過容器名 llama-embed:8082 存取 embedding
│
├─ new-api          :3000   LLM 閘道（含 Gemini / Anthropic / OpenRouter 後端）
├─ cloudflared              api.g8kz.top → new-api:3000 的外部隧道
└─ geminicli2api    :8888   new-api 的 Gemini CLI 後端（接入 new-api bridge 網路）
```

**為什麼用 `network_mode: host`**：new-api 與 everos 發布在宿主機 port，bridge 網路的容器無法直接用 `localhost` 存取它們；host 模式讓 bot 直接共享宿主機網路介面，`localhost:3000` 與 `localhost:8000` 即可連通。

---

## CI

GitHub Actions 每次 push main 自動執行：

1. `cargo fmt --all -- --check`
2. `cargo clippy --workspace --exclude g10kz-discord --exclude g10kz-bot -- -D warnings`
3. `cargo test --workspace --exclude g10kz-discord --exclude g10kz-bot`
4. `LLM_PROVIDER=mock cargo run -p g10kz-bot -- once "你好小十"`（smoke test）

---

## 開發注意事項

**Windows 掛載的 null byte 問題**：透過 Windows-mounted 路徑修改的 Rust / TOML 檔案可能附帶 trailing null bytes，導致 `cargo` 解析失敗。所有 Rust / TOML / 設定檔修改必須透過 paramiko SFTP 或 SSH exec_command，不可使用本地 Edit/Write 工具。

**Rust 字串不用 `\u` escape**：Rust 字串中的中文直接用字面 UTF-8，不用 `\u{XXXX}`，避免手誤寫成無大括號的 Python 風格 `\uXXXX` 而導致編譯錯誤。

---

## License

MIT
