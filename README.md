<a id="readme-top"></a>

[![Stars][stars-shield]][stars-url]
[![Forks][forks-shield]][forks-url]
[![Issues][issues-shield]][issues-url]
[![License][license-shield]][license-url]
[![CI][ci-shield]][ci-url]

<br />
<div align="center">
  <h2>g10kz</h2>
  <p>傲嬌 AI Discord 機器人 — 以 Rust 構建，具備長期記憶、多模型推理、人格自適應、被動觀察與即時搜索能力</p>
  <p>
    <a href="https://github.com/yoan3221/g10kz/issues/new?labels=bug">Report Bug</a>
    &middot;
    <a href="https://github.com/yoan3221/g10kz/issues/new?labels=enhancement">Request Feature</a>
  </p>
</div>

---

<details>
  <summary>目錄</summary>
  <ol>
    <li><a href="#關於此專案">關於此專案</a></li>
    <li><a href="#功能詳解">功能詳解</a></li>
    <li><a href="#技術棧">技術棧</a></li>
    <li><a href="#快速開始">快速開始</a></li>
    <li><a href="#設定">設定</a></li>
    <li><a href="#slash-commands">Slash Commands</a></li>
    <li><a href="#角色卡">角色卡</a></li>
    <li><a href="#架構">架構</a></li>
    <li><a href="#部署拓樸">部署拓樸</a></li>
    <li><a href="#使用的開源技術">使用的開源技術</a></li>
    <li><a href="#開發注意事項">開發注意事項</a></li>
    <li><a href="#license">License</a></li>
  </ol>
</details>

---

## 關於此專案

g10kz 是由 g8kz 創造的 18 歲原創角色「傲嬌 AI」，在 Discord 上以繁體中文與使用者自然互動。她表面嘴硬、愛逞強，內心其實超容易害羞、黏人。

整個 bot 以 **Rust + Tokio** 非同步架構構建，musl 靜態編譯成無依賴 binary，搭配 Docker Compose 部署。核心設計目標：

- **低延遲串流輸出**：SSE 漸進回覆，Discord 訊息實時更新
- **長期記憶**：EverOS 向量記憶 sidecar，跨會話保留使用者資料
- **被動學習**：旁觀群組對話也寫入長期記憶，不只記得直接對話
- **可熱抽換人格**：SillyTavern V2 JSON 或 OKF Markdown bundle，零重啟切換角色
- **完全本地推理**：bot、embedding、記憶提取全走本地 vLLM，零外部 LLM API 費用

<p align="right">(<a href="#readme-top">back to top</a>)</p>

---

## 功能詳解

### 人格系統

**SillyTavern V2 / OKF 角色卡**

支援兩種角色卡格式，熱切換無需重啟 bot：

- **OKF Markdown bundle**（推薦）：`index.md` 含 YAML frontmatter + 角色設定；`examples.md` 存對話範例；`lore/` 目錄放 Lorebook 條目
- **SillyTavern V2 JSON**：標準 chara_card_v2 格式，讀取 `system_prompt`、`first_mes`、`mes_example`

**JPAF 人格自適應框架**

追蹤 8 個榮格認知函式（Fe / Ti / Ne / Si / Fi / Te / Se / Ni），per-user 獨立建模。每次互動後根據訊息特徵 bump 或 decay 分數，最終將 modifier 字串注入 system prompt，讓角色對不同人產生不同的互動風格。

**Lorebook / World Info**

在 `lore/` 目錄放置 Markdown 檔案，YAML frontmatter 中列出 `trigger_words`。當使用者訊息包含觸發詞時，對應 lore entry 的內容自動注入 system 上下文。

**BM25 範例精選注入**

啟動時對 `examples.md` 中所有對話對建立 BM25 索引（含 CJK 分詞）。每輪根據使用者訊息查詢 top-2 最相關範例注入 system prompt，既保持示範效果又節省 token。

---

### 對話能力

**五路由引擎**

每則訊息通過 `route()` 函式自動分流到最適合的路徑：

| 優先 | 觸發條件 | 路徑 | LLM 呼叫 |
|---|---|---|---|
| 1 | 已知指令前綴 | Command | 0 次 |
| 2 | 有附件（圖片 / 影片 / 音訊 / PDF） | Media | Reason 路徑 |
| 3 | 搜尋觸發詞或時效性問題 | Search | 1 次 + 工具 |
| 4 | 複雜度訊號（長文 / 多問號 / 程式碼 / 分析詞） | Reason | N drafter + judge |
| 5 | 其餘閒聊 | Social | 1 次（串流） |

Social 路徑另有語意升級機制：embedding router 對訊息做語意相似度比較，必要時自動升級為 Search 或 Reason 路徑。若模型輸出 `[[ESCALATE]]` 哨符，engine 自動以更強模型重跑該輪。

**Fusion 多模型（Reason 路徑）**

```
[drafter A] ──┐
[drafter B] ──┼──→ Jaccard 語義共識過濾 ──→ judge 合成最終回覆
[drafter C] ──┘

drafter < 2 或全部失敗 → 自動退化為單模型
```

**動態歷史視窗**

根據訊息特徵動態決定歷史長度：

| 條件 | 歷史長度 |
|---|---|
| 包含延續詞（然後 / 所以 / 那個 / 剛才…） | 滿窗 |
| 訊息 ≤ 6 字的短句 | 6 條 |
| 訊息 7–40 字 | 10 條 |
| 訊息 > 40 字 | 8 條 |

**條件式內心獨白**

僅當情緒有起伏（被誇 / 被嗆 / 告白 / 尷尬）才在 `<think>…</think>` 私下想一句真心話，輸出前自動剝離，對方看不見。平淡閒聊直接答，節省 output token。

**搜尋哨符 `[[SEARCH:]]`（最高優先）**

詢問即時資訊時，模型第一個字元輸出 `[[SEARCH: 關鍵詞]]`，engine 偵測後呼叫 stealth 瀏覽器搜尋 DuckDuckGo，結果整合進回覆。此條規則凌駕 `[內心]` 指令，解決憑記憶幻覺回答新聞的問題。

---

### 長期記憶（EverOS）

**主動寫入**：每 10 輪對話呼叫一次 `POST /memory/flush`，觸發 EverOS 內部提取管線（boundary detection → episode extraction）。期間每輪仍呼叫 `POST /memory/add` 累積訊息。

**被動觀察**：群組中非 @bot 的訊息（≥ 4 字）呼叫 `observe()`，只 `POST /memory/add` 不 flush，讓 EverOS 累積至自然邊界後提取。

**搜索召回**：每輪開始前 `POST /memory/search`（BM25 + 向量混合），Social 路徑召回最多 6 條、Reason 路徑最多 8 條，注入 system prompt。

**容錯降級**：EverOS 掛掉時自動切換到 `NullMemory`，bot 正常運作，只記 WARN 日誌。

**Embedding 後端**：本地 llama-embed 服務（`:8082`），OpenAI 相容 `/v1/embeddings`，1024 維向量。

---

### 網路工具

**Stealth 瀏覽器服務**：Playwright 微服務（`:8091`），路由 `/v1/search`（DuckDuckGo 爬取）與 `/v1/render`（任意頁面渲染）。完全不依賴外部搜尋 API。

**搜索觸發方式**：

1. **Search 路由**：`route()` 偵測搜尋觸發詞 → 直接走 Search 路徑
2. **`[[SEARCH:]]` 哨符**：Social 路徑中模型輸出哨符 → engine 攔截 → 呼叫工具 → 重組回覆

---

### 安全與防護

**ML Prompt Guard**：Llama Prompt Guard 2（22M ONNX，CPU 推理，`:8083`）。每則訊息經 ML 分類後決定是否阻擋，INJECTION / JAILBREAK 直接拒絕，不進入 LLM。

**Owner 特殊身分**：以不可偽造的 Discord snowflake ID 識別創造者，owner 享有完整信任與親近感；其他人嘗試偽冒時角色低調防冒充。

**黑名單機制**：`BLACKLISTED_USERS` 逗號分隔的雪花 ID，黑名單使用者的訊息直接丟棄。

---

### 輸出格式化

**動作描述自動轉換**：`*動作*` / `_動作_` 格式自動轉換為 Discord blockquote（`> 動作`）

**顏文字自由生成**：LLM 根據情緒自由創作顏文字，每次不重複，直接融入台詞

**孤立反引號修正**：自動移除造成 Discord inline-code 爆版的孤立反引號

**自動分段**：回覆超過 2000 字時自動切分為多則訊息

**SSE 串流漸進編輯**：Social / Search / Media 路徑使用 SSE 串流，Discord 訊息實時更新

<p align="right">(<a href="#readme-top">back to top</a>)</p>

---

## 技術棧

[![Rust][rust-shield]][rust-url]
[![Tokio][tokio-shield]][tokio-url]
[![Docker][docker-shield]][docker-url]

<p align="right">(<a href="#readme-top">back to top</a>)</p>

---

## 快速開始

### 需求

- Docker + Docker Compose
- Discord Bot Token（需啟用 `MESSAGE_CONTENT` 與 `GUILD_MEMBERS` Intent）
- OpenAI 相容 LLM API（OpenRouter / vLLM / new-api / 其他）
- （選用）EverOS 長期記憶 sidecar + embedding server

### 安裝

1. Clone 專案
   ```sh
   git clone https://github.com/yoan3221/g10kz.git
   cd g10kz
   ```

2. 複製環境變數範本
   ```sh
   cp .env.example .env
   ```

3. 填入設定（見[設定](#設定)）並啟動
   ```sh
   docker compose up -d --build
   docker logs g10kz-bot -f
   ```

### 快速更新部署

```sh
# 在 bot 主機上（x86_64-unknown-linux-musl target 需已裝好）
source ~/.cargo/env
cargo build -p g10kz-bot --release --target x86_64-unknown-linux-musl
cp target/x86_64-unknown-linux-musl/release/g10kz-bot ~/g10kz/bin/g10kz-bot
cd ~/g10kz && docker compose down && docker compose build && docker compose up -d
```

### 本地測試（不需要 Discord）

```sh
LLM_PROVIDER=mock cargo run -p g10kz-bot -- once "你好，自我介紹一下"
```

> `once` 模式使用內建 stub persona，角色卡僅在 daemon 模式生效。

<p align="right">(<a href="#readme-top">back to top</a>)</p>

---

## 設定

`.env.example` 複製為 `.env`，**不要 commit `.env`**。

```env
# ── Discord ──────────────────────────────────────────
DISCORD_TOKEN=          # Bot Token（Discord Developer Portal 取得）
OWNER_USER_ID=          # 創造者的 Discord 雪花 ID

# ── LLM ─────────────────────────────────────────────
LLM_PROVIDER=openrouter
LLM_BASE_URL=https://openrouter.ai/api/v1   # 或本地 vLLM / new-api URL
LLM_API_KEY=

LLM_MODEL_SOCIAL=google/gemma-3-27b-it      # 日常對話 / Social / Search / Media
LLM_MODEL_REASON=google/gemma-3-27b-it      # 深度推理（Reason 工具迴圈）
LLM_MODEL_JUDGE=google/gemma-3-27b-it       # Fusion judge 合成
LLM_MODEL_MEDIA=google/gemma-3-27b-it       # 附件 / 圖片視覺處理（需支援 multimodal）

# Fusion drafter 列表（逗號分隔，≥2 才啟用 Fusion；留空退化為單模型）
LLM_FUSION_DRAFTERS=

# ── 記憶 ─────────────────────────────────────────────
EVEROS_URL=http://localhost:8000   # EverOS sidecar；留空則用 NullMemory

# ── Embedding（語意路由 embed_router 用）────────────
EMBED_SERVER_URL=http://localhost:8082   # OpenAI 相容 embedding server
EMBED_MODEL=embed                        # 模型 ID

# ── 瀏覽器服務 ──────────────────────────────────────
BROWSER_URL=http://localhost:8091        # Playwright stealth 服務；留空則停用

# ── 角色卡 ───────────────────────────────────────────
PERSONA_CARD_PATH=./persona/okf          # 目錄 → OKF bundle；.json 檔 → SillyTavern V2

# ── 安全 ─────────────────────────────────────────────
PROMPT_GUARD_URL=http://localhost:8083   # ML Prompt Guard；留空則跳過
BLACKLISTED_USERS=                       # 逗號分隔的雪花 ID

# ── 群組行為 ──────────────────────────────────────────
LURK_CHANNELS=                           # 逗號分隔的頻道 ID，啟用 lurk 模式
LURK_REPLY_PROBABILITY=0.05              # [0.0, 1.0]，lurk 頻道隨機回覆機率
PROACTIVE_INACTIVE_SECS=86400            # 靜默多久後主動發話（秒）

# ── 其他 ─────────────────────────────────────────────
REQUEST_TIMEOUT_SECS=120
RUST_LOG=g10kz=info,warn
```

<p align="right">(<a href="#readme-top">back to top</a>)</p>

---

## Slash Commands

| 指令 | 說明 |
|---|---|
| `/search <query>` | 強制觸發網路搜索並回傳結果 |
| `/reset` | 清除目前頻道的對話歷史 |
| `/stop` | 中斷當前正在生成的回覆 |
| `/persona` | 顯示目前載入的角色卡名稱與摘要 |
| `/help` | 顯示所有可用指令清單 |

<p align="right">(<a href="#readme-top">back to top</a>)</p>

---

## 角色卡

### OKF Bundle（推薦）

```
persona/okf/
  index.md          # YAML frontmatter + 角色設定正文
  examples.md       # 對話範例（BM25 top-2 每輪注入）
  lore/             # Lorebook 條目（可選）
    world.md        # trigger_words: 詞1, 詞2
```

`index.md` 格式：

```markdown
---
type: Character
title: 角色名稱
---
你是一個……（角色設定）
```

`examples.md` 格式：

```markdown
---
type: Dialogue Examples
---
{{user}}: 使用者說的話
{{char}}: 角色回覆

{{user}}: 另一個情境
{{char}}: 回覆
```

### SillyTavern V2 JSON（相容）

```json
{
  "spec": "chara_card_v2",
  "data": {
    "name": "角色名",
    "system_prompt": "你是...",
    "first_mes": "第一句話",
    "mes_example": "<START>\n{{user}}: ...\n{{char}}: ...\n<END>"
  }
}
```

<p align="right">(<a href="#readme-top">back to top</a>)</p>

---

## 架構

### Crate 分層

```
L5  g10kz-bot      主 binary；daemon（長跑）與 once（單次測試）兩種模式
L4  g10kz-discord  Serenity 0.12 閘道；過濾訊息、附件抽取、slash commands、observe() 旁觀寫入
L3  g10kz-engine   一回合狀態機；串接所有 L0–L2 組件，實作 5 條路徑執行邏輯
L2  g10kz-everos   EverOS HTTP 客戶端；add_turn / observe / flush / search，失敗自動降級
L2  g10kz-tools    ToolBox 介面 + 工具實作（WebSearch / FetchPage / TwStock / Time / Escalate）
L1  g10kz-llm      OpenAI 相容 HTTP 客戶端；SSE 串流；FusionProvider；MockProvider
L1  g10kz-kernel   路由 / guard / JPAF / sanitize / persona 載入（OKF bundle 或 JSON）
L0  g10kz-config   型別化設定，無任何外部依賴
```

依賴方向嚴格由下往上，無反向耦合。

### 一回合處理管線

```
[discord] 訊息進入
    │
    ▼
[kernel]  pre_guard()
          ├─ owner 直通
          ├─ 黑名單丟棄
          └─ ML Prompt Guard（ONNX CPU）→ INJECTION 拒絕
    │
    ▼
[kernel]  normalize_input() → display text
    │
    ▼
[everos]  search(display_text)   BM25+向量混合；失敗靜默
    │
    ▼
[engine]  system_message() 組裝
          ├─ 靜態部分（角色卡 / BM25 範例 / 格式說明 / 工具 schema）
          └─ 動態部分（guild 名稱 / JPAF modifier / EverOS 召回 / lore）
    │
    ▼
[engine]  embed_router.refine()  語意升級 Social → Search/Reason（可選）
    │
    ▼
[kernel]  route()
    │
    ├─ Social  → haiku 串流 → [[SEARCH:]] 哨符→工具 / [[ESCALATE]]→升級
    ├─ Search  → WebSearchTool → haiku 整合結果
    ├─ Reason  → FusionProvider（N drafter + Jaccard + judge）+ 工具迴圈
    ├─ Media   → 附件 URL 抓取 → Reason 路徑
    └─ Command → 0 LLM 呼叫
    │
    ▼
[kernel]  sanitize_output()
          strip_thinking / strip_artefact / actions_to_blockquote / 分段
    │
    ▼
[everos]  add_turn()   累積訊息（每 10 輪 flush 一次）
    │
    ▼
[kernel]  JPAF::update()   bump/decay 認知函式分數
```

### EverOS 記憶整合

| 端點 | 時機 | 備註 |
|---|---|---|
| `POST /api/v1/memory/search` | 每輪開始前 | BM25+向量；失敗靜默 |
| `POST /api/v1/memory/add` | 每輪（add_turn） / observe() | 旁觀：僅 user 角色 |
| `POST /api/v1/memory/flush` | 每 10 輪 add_turn | 觸發 boundary detection + episode extraction |

EverOS 啟用策略（其他全部停用以節省 LLM 消耗）：

```
boundary detection  →  episode extraction
                              ↓
                    (atomic_facts / foresight / profile 已停用)
```

<p align="right">(<a href="#readme-top">back to top</a>)</p>

---

## 部署拓樸

```
<BOT-SERVER-IP>（主機）
├─ g10kz-bot       Docker host network，:—
│                   ← LLM 直連 vLLM :8000 on <GPU-SERVER-IP>
│                   ← EverOS :8000 / embed :8082 / guard :8083 / browser :8091
├─ everos           Docker，:8000   語意記憶 sidecar
│                   └─ LLM：本地 gemma4 via OmniRoute :20128
│                   └─ Embedding：llama-embed :8082
├─ llama-embed      Docker，:8082   本地 embedding server（OpenAI 相容）
├─ prompt-guard     Docker，:8083   Llama Prompt Guard 2（ONNX CPU）
├─ browser          Docker，:8091   Playwright stealth 瀏覽器服務
│                   └─ /v1/search（DuckDuckGo）/ /v1/render（任意頁面）
├─ new-api          Docker，:20128  OpenAI 相容 LLM 閘道（OmniRoute）
│                   └─ 路由 → <GPU-SERVER-IP>:8000（本地 vLLM，gemma4）
├─ cloudflared      Docker，CF Tunnel → api.g8kz.top → new-api:20128
├─ postgres         Docker，:5432   new-api 持久化
└─ redis            Docker，:6379   new-api 快取

<GPU-SERVER-IP>（GPU 主機）
└─ vLLM             :8000   gemma-4 abliterated AWQ（本地推理）
                            model ID：gemma4
```

g10kz-bot 以 host network 執行，直接用 `localhost` 存取所有服務；vLLM 以直連 IP 存取。

<p align="right">(<a href="#readme-top">back to top</a>)</p>

---

## 使用的開源技術

| 項目 | 用途 |
|---|---|
| [Serenity](https://github.com/serenity-rs/serenity) | Rust Discord 閘道 / 事件處理 |
| [EverOS](https://github.com/EverMind-AI/EverOS) | 向量化長期記憶 sidecar（episode / atomic facts / profile） |
| [SillyTavern](https://github.com/SillyTavern/SillyTavern) | V2 角色卡格式規範 |
| [discord.js](https://discord.js.org/) | Discord API 參考（Serenity 補充） |
| [Playwright](https://playwright.dev/) | Stealth 瀏覽器服務（搜尋 / 頁面渲染） |
| [Llama Prompt Guard 2](https://huggingface.co/meta-llama/Prompt-Guard-2-22M) | 提示注入偵測（22M ONNX，CPU 推理） |
| [Tokio](https://github.com/tokio-rs/tokio) | Rust 非同步運行時 |
| [reqwest](https://github.com/seanmonstar/reqwest) | HTTP 客戶端 |
| [serde / serde_json](https://github.com/serde-rs/serde) | JSON 序列化 |
| [tracing](https://github.com/tokio-rs/tracing) | 結構化日誌 |

<p align="right">(<a href="#readme-top">back to top</a>)</p>

---

## 開發注意事項

**Rust 字串**：中文直接用字面 UTF-8，絕不使用 `\u{XXXX}` 轉義。

**編譯 musl binary**：

```sh
source ~/.cargo/env
cargo build -p g10kz-bot --release --target x86_64-unknown-linux-musl
```

**部署流程**（禁止用 `restart`，不會讀取新 env_file）：

```sh
cp target/.../g10kz-bot ~/g10kz/bin/g10kz-bot
cd ~/g10kz
docker compose down
docker compose build
docker compose up -d
```

**`.env` 絕不 commit**：已在 `.gitignore` 排除。所有 credentials 只存在 `.env`。

**EverOS 設定熱載入**：`~/.everos/ome.toml` 修改後約 2 秒自動生效，無需重啟。

<p align="right">(<a href="#readme-top">back to top</a>)</p>

---

## License

[AGPL-3.0](LICENSE) © 2026 g8kz

<p align="right">(<a href="#readme-top">back to top</a>)</p>

---

<!-- SHIELDS -->
[stars-shield]: https://img.shields.io/github/stars/yoan3221/g10kz.svg?style=for-the-badge
[stars-url]: https://github.com/yoan3221/g10kz/stargazers
[forks-shield]: https://img.shields.io/github/forks/yoan3221/g10kz.svg?style=for-the-badge
[forks-url]: https://github.com/yoan3221/g10kz/network/members
[issues-shield]: https://img.shields.io/github/issues/yoan3221/g10kz.svg?style=for-the-badge
[issues-url]: https://github.com/yoan3221/g10kz/issues
[license-shield]: https://img.shields.io/github/license/yoan3221/g10kz.svg?style=for-the-badge
[license-url]: https://github.com/yoan3221/g10kz/blob/main/LICENSE
[ci-shield]: https://img.shields.io/github/actions/workflow/status/yoan3221/g10kz/ci.yml?style=for-the-badge&label=CI
[ci-url]: https://github.com/yoan3221/g10kz/actions/workflows/ci.yml
[rust-shield]: https://img.shields.io/badge/Rust-000000?style=for-the-badge&logo=rust&logoColor=white
[rust-url]: https://www.rust-lang.org/
[tokio-shield]: https://img.shields.io/badge/Tokio-000000?style=for-the-badge&logo=tokio&logoColor=white
[tokio-url]: https://tokio.rs/
[docker-shield]: https://img.shields.io/badge/Docker-2496ED?style=for-the-badge&logo=docker&logoColor=white
[docker-url]: https://www.docker.com/
