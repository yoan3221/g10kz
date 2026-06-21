<a id="readme-top"></a>

[![Stars][stars-shield]][stars-url]
[![Forks][forks-shield]][forks-url]
[![Issues][issues-shield]][issues-url]
[![License][license-shield]][license-url]

<br />
<div align="center">
  <h2>g10kz</h2>
  <p>傲嬌 AI Discord 機器人 — 以 Rust 構建，具備長期記憶、多模型推理與人格自適應能力</p>
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

g10kz 是由 g8kz 創造的 18 歲原創角色，在 Discord 上以繁體中文與使用者自然互動。她表面嘴硬、愛逞強，內心其實超容易害羞、黏人 (//ω//)

**人格**
- SillyTavern V2 角色卡驅動，可熱抽換角色
- JPAF 人格自適應框架：追蹤 8 個榮格認知函式（Fe/Ti/Ne/Si…），per-user 建模，讓角色對不同人產生不同的互動風格
- 伺服器 / 頻道感知，角色知道自己在哪個 Discord 伺服器與頻道

**對話能力**
- 五路由引擎：Social / Search / Reason / Media / Command，依訊息自動分流
- Fusion 多模型（Reason 路徑）：多個 drafter 並行 → 共識過濾 → judge 合成，回覆品質優於單模型
- EverOS 語意記憶：向量化長期記憶 sidecar，每輪自動 add/flush/search，掛掉自動降級；embedding 後端為 Cloudflare Workers AI（Qwen3-Embedding-0.6B）
- 滑動視窗歷史：Social 路徑最多 14 條、Reason 路徑 12 條，防止 token 無限增長
- 內心獨白（Inner Monologue）：僅當情緒有起伏時才在 `<think>...</think>` 私下想一句真心話，輸出前自動剝離；平淡閒聊免 think 直接答，節省 output token

**輸出格式**
- 顏文字直接生成：LLM 直接在台詞中寫出顏文字（如 `(//ω//)` `(♡ω♡)` 等），移除 BM25 佔位符機制
- 動作描述自動轉換：`> 動作描述` 格式直接渲染成 Discord blockquote
- Prompt 語義去重 + prefix-cache 靜態/動態分離；SOCIAL_EXTRA_NOTE 積極壓縮 + BM25 範例 top-3→top-2 + 條件式 think，每輪 input token 較原始設計節省 ~45%，output 端 think token 浪費大幅減少

**工具**
- 網路搜索：Gemini 2.5 Flash Lite + Google Search grounding，即時新聞準確率高；Social 路徑偵測到時效性問題自動觸發 `[[SEARCH:]]` 哨符，禁止憑記憶幻覺回答
- 零幻覺鐵則：新聞 / 技術細節沒把握一律說不知道或觸發搜尋，不猜測、不編造
- 台股即時報價、台灣時間

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
- Discord Bot Token
- OpenAI 相容 LLM API（OpenRouter / new-api / 其他）

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

程式碼修改後，musl 靜態編譯 + binary swap，停機約 10 秒：

```sh
# 前置：裝一次即可
rustup target add x86_64-unknown-linux-musl
sudo apt-get install -y musl-tools

# 每次更新
./deploy.sh
```

### 本地測試（不需要 Discord）

```sh
LLM_PROVIDER=mock cargo run -p g10kz-bot -- once "你好，自我介紹一下"
```

<p align="right">(<a href="#readme-top">back to top</a>)</p>

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

<p align="right">(<a href="#readme-top">back to top</a>)</p>

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

<p align="right">(<a href="#readme-top">back to top</a>)</p>

---

## 角色卡

支援兩種格式，`PERSONA_CARD_PATH` 指向目錄時自動載入 OKF bundle（推薦）：

**OKF Bundle（推薦）**
```
persona/okf/
  index.md       # YAML frontmatter + 角色設定正文 + ## First Message
  examples.md    # 對話範例（BM25 top-k 每輪注入）
  lore/          # Lorebook（可選）
```

**SillyTavern V2 JSON（相容）**
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

> `once` 模式使用內建 stub persona，角色卡僅在 daemon 模式生效。OKF bundle 中 `## First Message` 之後的內容會被自動截斷，不會注入 system prompt。

<p align="right">(<a href="#readme-top">back to top</a>)</p>

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
L1  g10kz-kernel   路由 / guard / JPAF / sanitize（lone backtick / pipe fix）/ persona 載入（OKF bundle 或 JSON）
L0  g10kz-config   型別化設定，無任何外部依賴
```

依賴方向由下往上，無反向耦合。

### Discord 閘道過濾

```
Discord 事件
  ├─ DM（私訊）               → ✓ 進入管線
  ├─ @mention（群組 @bot）    → ✓ 進入管線
  ├─ reply to bot（回覆機器人）→ ✓ 進入管線
  └─ 其他群組訊息              → 存入 ring buffer（作為語境背景，不回應）
```

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
    ├─ Social  → [llm] 單次 social model，history 最多 14 條
    ├─ Search  → [tools] WebSearch → [llm] social model 整合
    ├─ Reason  → [llm] FusionProvider + 工具迴圈，history 最多 12 條
    ├─ Media   → 附件 URL → Reason 路徑
    └─ Command → 直接處理，0 LLM 呼叫
    │
    ▼
[kernel]  sanitize()
          strip_artefact → collapse_blank_lines
          → actions_to_blockquote（*動作* / _動作_ / > 動作 → Discord blockquote）
          → strip_lone_backtick（移除造成 Discord code span 爆版的孤立反引號）
          → pipe_to_blockquote（| 行首修正為 >）
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

### 路由決策

| 優先 | 觸發條件 | 路徑 | LLM 呼叫 |
|---|---|---|---|
| 1 | 已知指令前綴 | Command | 0 次 |
| 2 | 有附件 | Media | Reason 路徑 |
| 3 | 搜尋觸發詞 | Search | 1 次 + 工具 |
| 4 | 複雜度訊號（長文 / 多問號 / 程式碼 / 分析詞） | Reason | N drafter + judge |
| 5 | 其餘 | Social | 1 次 |

### Fusion 多模型（Reason 路徑）

```
[drafter A] ──┐
[drafter B] ──┼──→ Jaccard 共識過濾 ──→ judge 合成最終回覆
[drafter C] ──┘

drafter < 2 或全部失敗 → 自動退化為單模型
```

### EverOS 記憶整合

[EverOS](https://github.com/EverMind-AI/EverOS) HTTP API，embedding 後端為 `cf-embed`（Cloudflare Workers AI Proxy，Qwen3-Embedding-0.6B，1024-dim）：

| 端點 | 時機 |
|---|---|
| `POST /api/v1/memory/search` | 每輪開始前 |
| `POST /api/v1/memory/add` | 每輪結束後 |
| `POST /api/v1/memory/flush` | add 之後 |

<p align="right">(<a href="#readme-top">back to top</a>)</p>

---

## 部署拓樸

```
REDACTED (主機)
├─ g10kz-bot      Docker，host network  LLM 閘道 :3000 / EverOS :8000 / guard :8083
├─ new-api        Docker，:3000         OpenAI 相容 LLM 閘道
├─ everos         Docker，:8000         語意記憶 sidecar
├─ cf-embed       Docker，:8082         Cloudflare Workers AI embedding proxy
├─ prompt-guard   Docker，:8083         Llama Prompt Guard 2 22M（OpenVINO CPU）
├─ gemini-search  Docker，host :8090    Gemini 2.5 Flash Lite + Google grounding
├─ cloudflared    Docker，host network  CF Tunnel → api.g8kz.top → new-api:3000
├─ postgres       Docker，:5432         new-api 持久化
└─ redis          Docker，:6379         new-api 快取
```

`g10kz-bot` 以 `host` network 執行，直接用 `localhost:*` 存取所有服務。其餘服務透過 Docker bridge，`host-gateway` extra_hosts 讓容器回呼宿主機服務。

<p align="right">(<a href="#readme-top">back to top</a>)</p>

---

## 使用的開源技術

| 項目 | 用途 |
|---|---|
| [Serenity](https://github.com/serenity-rs/serenity) | Rust Discord 閘道 / 事件處理 |
| [EverOS](https://github.com/EverMind-AI/EverOS) | 向量化長期記憶 sidecar |
| [Cloudflare Workers AI](https://developers.cloudflare.com/workers-ai/) | 遠端 embedding 推理（Qwen3-Embedding-0.6B） |
| [SillyTavern](https://github.com/SillyTavern/SillyTavern) | V2 角色卡格式規範 |
| [new-api](https://github.com/Calcium-Ion/new-api) | OpenAI 相容 LLM 閘道 |
| [Llama Prompt Guard 2](https://huggingface.co/meta-llama/Prompt-Guard-2-22M) | 提示注入偵測（22M ONNX，OpenVINO CPU） |
| [Tokio](https://github.com/tokio-rs/tokio) | Rust 非同步運行時 |
| [reqwest](https://github.com/seanmonstar/reqwest) | HTTP 客戶端（LLM / EverOS / 搜索） |
| [serde / serde_json](https://github.com/serde-rs/serde) | JSON 序列化 / 反序列化 |
| [tracing](https://github.com/tokio-rs/tracing) | 結構化日誌 |
| [Gemini API](https://ai.google.dev/) | 網路搜索 + Google grounding（gemini-search 微服務） |

<p align="right">(<a href="#readme-top">back to top</a>)</p>

---

## 開發注意事項

**Windows 掛載 null byte 問題**：透過 Windows-mounted 路徑修改的 Rust / TOML 檔案可能附帶 trailing null bytes，導致 `cargo` 解析失敗。所有原始碼修改必須透過 paramiko SFTP，不可使用本地 Edit/Write 工具。

**Rust 字串**：中文直接用字面 UTF-8，不用 `\u{XXXX}`，避免手誤寫成 Python 風格的 `\uXXXX`。

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
[rust-shield]: https://img.shields.io/badge/Rust-000000?style=for-the-badge&logo=rust&logoColor=white
[rust-url]: https://www.rust-lang.org/
[tokio-shield]: https://img.shields.io/badge/Tokio-000000?style=for-the-badge&logo=tokio&logoColor=white
[tokio-url]: https://tokio.rs/
[docker-shield]: https://img.shields.io/badge/Docker-2496ED?style=for-the-badge&logo=docker&logoColor=white
[docker-url]: https://www.docker.com/
