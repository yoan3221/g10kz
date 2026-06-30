<div align="center">

# g10kz

**Rust 實作的 AI Discord Bot — 傲嬌角色、長期記憶、多路由推理、本地優先**

[![Stars](https://img.shields.io/github/stars/yoan3221/g10kz.svg?style=for-the-badge)](https://github.com/yoan3221/g10kz/stargazers)
[![Forks](https://img.shields.io/github/forks/yoan3221/g10kz.svg?style=for-the-badge)](https://github.com/yoan3221/g10kz/network/members)
[![License](https://img.shields.io/github/license/yoan3221/g10kz.svg?style=for-the-badge)](LICENSE)
[![CI](https://img.shields.io/github/actions/workflow/status/yoan3221/g10kz/ci.yml?style=for-the-badge&label=CI)](https://github.com/yoan3221/g10kz/actions)

</div>

---

g10kz 是一個以 **Rust + Tokio** 非同步架構構建的 Discord AI bot，對外呈現的是一個 18 歲傲嬌少女的角色，對內是一套具備路由推理、向量記憶、人格自適應、即時搜索能力的推理引擎。

設計出發點只有一個：讓 bot 真的「記得你」、「會思考」、「懂時事」——而不只是每次對話都從零開始的問答機器。

---

## 目錄

- [它能做什麼](#它能做什麼)
- [它怎麼運作](#它怎麼運作)
  - [五路由引擎](#五路由引擎)
  - [長期記憶](#長期記憶)
  - [人格系統](#人格系統)
  - [網路搜索](#網路搜索)
  - [安全防護](#安全防護)
- [部署](#部署)
  - [需求](#需求)
  - [最簡部署（OpenRouter）](#最簡部署openrouter)
  - [全功能本地部署](#全功能本地部署)
  - [編譯 binary](#編譯-binary)
- [設定參考](#設定參考)
- [角色卡格式](#角色卡格式)
- [Slash Commands](#slash-commands)
- [Crate 架構](#crate-架構)
- [License](#license)

---

## 它能做什麼

- **記得你說過的事** — 跨會話保留使用者記憶，再次見面時知道你之前聊過什麼。
- **旁聽群組對話** — 沒被 @ 的訊息也悄悄記進長期記憶，像是真的在群裡生活。
- **知道今天發生了什麼** — 偵測到時效性問題自動搜索 DuckDuckGo，整合結果回答。
- **認真的問題認真回答** — 複雜推理走多 drafter + judge 的 Fusion 路徑，不用同一個模型包辦一切。
- **可換角色** — 支援 SillyTavern V2 JSON 與 OKF Markdown bundle，零重啟熱切換人格。
- **對不同的人有不同的態度** — JPAF 框架 per-user 追蹤 8 個榮格認知函式，互動風格隨使用者個性調整。
- **不會被 jailbreak** — ML Prompt Guard 2（22M ONNX）每則訊息分類，注入攻擊直接擋在入口。

---

## 它怎麼運作

### 五路由引擎

每則訊息進來，`route()` 函式依優先順序決定走哪條路徑：

```
訊息
 │
 ├─ 1. 斜線指令    → 0 次 LLM 呼叫，直接執行
 ├─ 2. 附件        → 抓取圖片/影片 URL → Reason 路徑處理
 ├─ 3. 搜索觸發    → 呼叫 stealth 瀏覽器 → 彙整結果回覆
 ├─ 4. 複雜推理    → Fusion：N 個 drafter 並行 → 共識過濾 → judge 合成
 └─ 5. 一般對話    → Social 串流，有需要時升級
```

**Fusion 多模型推理**（Reason 路徑）

```
[drafter A] ─┐
[drafter B] ─┼──→ Jaccard 語義共識 ──→ judge 合成最終回覆
[drafter C] ─┘

drafter 少於 2 個或全部失敗 → 自動退化為單模型
```

不同路徑可以配不同的模型，讓日常閒聊走便宜快速的模型，複雜問題走昂貴精準的模型。

**自動升級哨符**

- Social 路徑如果模型輸出 `[[SEARCH: 關鍵詞]]`，engine 攔截後呼叫搜尋工具再彙整回覆，不需要預先判斷要不要搜索。
- 模型輸出 `[[ESCALATE]]` 時，engine 自動切換更強的模型重跑當輪。

---

### 長期記憶

記憶系統使用 [EverOS](https://github.com/EverMind-AI/EverOS)，一個本地向量記憶 sidecar。

**寫入（三個時機）**

```
一般對話 ─→ 每輪 /add 累積訊息 ─→ 每 N 輪 /flush 觸發提取
                                        ↓
                             boundary detection → episode extraction
                             （「這段對話發生了什麼事」寫進向量庫）

旁觀模式 ─→ 群組中未 @ bot 的訊息 → 僅 /add，自然積累
                                        ↓
                             EverOS 依話題邊界自行切割

直接呼叫 ─→ observe() ────────────────────────────────────────▶
```

**讀取（每輪開始前）**

```
使用者訊息 ─→ BM25 + 向量混合搜索 ─→ 相關記憶 top-6 注入 system prompt
```

**容錯**：EverOS 掛掉時自動切換 `NullMemory`，bot 繼續正常回話，只記 WARN 日誌。

---

### 人格系統

**角色卡格式**

支援兩種格式，透過 `PERSONA_CARD_PATH` 指向目錄（OKF）或 `.json` 檔（SillyTavern）：

| 格式 | 說明 |
|---|---|
| OKF bundle（目錄） | `index.md` 角色設定、`examples.md` 對話範例、`lore/` Lorebook 條目 |
| SillyTavern V2 JSON | 標準 chara_card_v2，讀取 `system_prompt`、`first_mes`、`mes_example` |

**BM25 範例精選**

啟動時對 `examples.md` 建立 BM25 + CJK 分詞索引。每輪根據使用者訊息查詢 top-2 最相關範例注入 system prompt，保留示範效果又節省 token。

**Lorebook 觸發注入**

`lore/` 下的 Markdown 檔案在 YAML frontmatter 列出 `trigger_words`，使用者訊息命中觸發詞時，該條目自動插入 context。

**JPAF 人格自適應框架**

追蹤每個使用者的 8 個榮格認知函式（Fe / Ti / Ne / Si / Fi / Te / Se / Ni），每次互動後 bump 或 decay 分數，將 modifier 字串注入 system prompt，讓角色對不同人產生不同的互動風格——話少的人她會主動多說兩句，話嘮的人她會拿架子少回應。

**條件式內心獨白**

只有在情緒有明顯起伏（被誇、被嗆、告白、尷尬）時，才在 `<think>…</think>` 私下想一句真心話，輸出前自動剝離，使用者看不見。平淡對話直接答，不浪費 output token。

---

### 網路搜索

搜索後端是 Playwright stealth 瀏覽器微服務（`:8091`），爬取 DuckDuckGo 結果，完全不依賴任何外部搜索 API key。

觸發方式有兩條：

1. **路由層偵測**：`route()` 分析訊息包含搜索觸發詞（「最近」「今天」「誰是」「多少錢」等）→ 直接走 Search 路徑
2. **LLM 自發觸發**：Social 路徑中模型認為需要資料時，第一個輸出 `[[SEARCH: 關鍵詞]]` → engine 攔截 → 呼叫工具 → 重組回覆

---

### 安全防護

**ML Prompt Guard**：每則訊息先過 Llama Prompt Guard 2（22M 參數 ONNX，CPU 推理），INJECTION / JAILBREAK 分類直接拒絕，不進 LLM。

**Owner 認證**：以不可偽造的 Discord snowflake ID 識別 bot 創造者，owner 享有完整信任；其他人嘗試偽冒時 bot 低調防冒充。

**黑名單**：`BLACKLISTED_USERS` 設定的雪花 ID，訊息進來直接丟棄。

---

## 部署

### 需求

- Docker + Docker Compose
- Discord Bot（需啟用 `MESSAGE_CONTENT` 和 `GUILD_MEMBERS` Intent）
- 任意 OpenAI 相容 LLM API（OpenRouter / vLLM / new-api / ollama / 等等）

**選用服務**（提升功能，沒有也能跑）：

| 服務 | 功能 | 預設埠 |
|---|---|---|
| [EverOS](https://github.com/EverMind-AI/EverOS) | 長期記憶 | 8000 |
| 任意 OpenAI 相容 embedding server | 語意路由 + 記憶搜索 | 8082 |
| Prompt Guard 服務 | 注入偵測 | 8083 |
| Playwright stealth 瀏覽器服務 | 網路搜索 | 8091 |

---

### 最簡部署（OpenRouter）

這是最快能跑起來的方式，不需要任何本地 AI 服務。

**1. 準備 Discord Bot**

到 [Discord Developer Portal](https://discord.com/developers/applications) 建立 Application → Bot，開啟 `MESSAGE_CONTENT` 和 `GUILD_MEMBERS` Privileged Gateway Intents，複製 Bot Token。

**2. Clone 專案**

```bash
git clone https://github.com/yoan3221/g10kz.git
cd g10kz
```

**3. 建立 .env**

```bash
cp .env.example .env
```

編輯 `.env`，至少填入：

```env
DISCORD_TOKEN=你的 Bot Token
OWNER_USER_ID=你的 Discord 使用者 ID（右鍵複製 ID）

LLM_PROVIDER=openrouter
LLM_BASE_URL=https://openrouter.ai/api/v1
LLM_API_KEY=你的 OpenRouter API Key

LLM_MODEL_SOCIAL=google/gemma-3-27b-it
LLM_MODEL_REASON=google/gemma-3-27b-it
LLM_MODEL_JUDGE=google/gemma-3-27b-it
```

**4. 準備 binary + 啟動**

> bot 需要預先編譯好的 musl 靜態 binary（見[編譯 binary](#編譯-binary)），放到 `bin/g10kz-bot`。

```bash
mkdir -p bin
# 把編譯好的 binary 放到 bin/g10kz-bot
docker compose build
docker compose up -d
docker compose logs -f
```

到 Discord 邀請 bot 進伺服器後輸入任何訊息，正常應該能看到回覆。

---

### 全功能本地部署

想要長期記憶、搜索、prompt guard 全開，需要額外跑幾個服務：

**EverOS（長期記憶）**

```bash
git clone https://github.com/EverMind-AI/EverOS everos
cd everos
# 設定 EverOS 的 LLM 和 embedding 來源（參考 EverOS 文件）
docker compose up -d
```

在 g10kz 的 `.env` 加上：

```env
EVEROS_URL=http://localhost:8000
```

**Stealth 瀏覽器服務（網路搜索）**

```bash
# 搜尋自建 Playwright stealth 服務，或用任何實作了下列 API 的服務：
# POST /v1/search   body: {"query": "..."} → 回傳搜索摘要
# POST /v1/render   body: {"url": "..."}   → 回傳頁面 markdown
```

在 g10kz 的 `.env` 加上：

```env
BROWSER_URL=http://localhost:8091
```

**Embedding Server（語意路由）**

任意 OpenAI 相容 `/v1/embeddings` 端點，推薦用 [llama.cpp server](https://github.com/ggml-org/llama.cpp)、[ollama](https://ollama.com/) 或 vLLM 跑本地模型。

```env
EMBED_SERVER_URL=http://localhost:8082
EMBED_MODEL=你的模型 ID
```

---

### 編譯 binary

bot 使用 `x86_64-unknown-linux-musl` target 靜態編譯，不依賴 libc，可直接在任何 x86_64 Linux 上執行。

**安裝 musl target**

```bash
# 安裝 Rust（如果沒有）
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# 加入 musl target
rustup target add x86_64-unknown-linux-musl

# Debian/Ubuntu 需要 musl linker
sudo apt-get install musl-tools
```

**編譯**

```bash
cargo build -p g10kz-bot --release --target x86_64-unknown-linux-musl
cp target/x86_64-unknown-linux-musl/release/g10kz-bot bin/g10kz-bot
```

**一鍵編譯 + 部署**（在 bot 主機上執行）

```bash
bash build_and_deploy.sh
```

> **注意**：更新 `.env` 後必須 `docker compose down && docker compose up -d`，`docker compose restart` 不會重新讀取 env_file。

---

## 設定參考

完整設定透過 `.env` 注入，詳見 `.env.example`。

**必填**

| 變數 | 說明 |
|---|---|
| `DISCORD_TOKEN` | Discord Bot Token |
| `OWNER_USER_ID` | 你的 Discord 雪花 ID（決定 owner 特殊身分） |
| `LLM_BASE_URL` | OpenAI 相容 LLM API 端點 |
| `LLM_API_KEY` | LLM API Key |

**LLM 模型（可各別設定不同模型）**

| 變數 | 用途 | 建議 |
|---|---|---|
| `LLM_MODEL_SOCIAL` | 日常對話、串流回覆 | 快速便宜的模型 |
| `LLM_MODEL_REASON` | 深度推理、工具呼叫迴圈 | 強力模型 |
| `LLM_MODEL_JUDGE` | Fusion judge 合成最終回覆 | 邏輯強的模型 |
| `LLM_MODEL_MEDIA` | 圖片/附件處理 | 支援 multimodal 的模型 |
| `LLM_FUSION_DRAFTERS` | 逗號分隔的 drafter 列表 | 填 2 個以上才啟用 Fusion |

**人格**

| 變數 | 說明 |
|---|---|
| `PERSONA_CARD_PATH` | 指向 OKF bundle 目錄或 SillyTavern V2 `.json` 檔；留空用內建 stub |

**選用服務**

| 變數 | 說明 |
|---|---|
| `EVEROS_URL` | EverOS 記憶服務 URL；留空停用記憶（NullMemory） |
| `EMBED_SERVER_URL` | OpenAI 相容 embedding server URL |
| `EMBED_MODEL` | Embedding 模型 ID |
| `BROWSER_URL` | Stealth 瀏覽器服務 URL；留空停用搜索 |
| `PROMPT_GUARD_URL` | Prompt Guard 服務 URL；留空跳過 ML 防護 |

**群組行為**

| 變數 | 預設 | 說明 |
|---|---|---|
| `LURK_CHANNELS` | 空 | 逗號分隔的頻道 ID，啟用旁觀模式 |
| `LURK_REPLY_PROBABILITY` | `0.0` | 旁觀頻道隨機插嘴機率 `[0.0, 1.0]` |
| `PROACTIVE_INACTIVE_SECS` | `86400` | 靜默多久後主動發話（秒） |
| `BLACKLISTED_USERS` | 空 | 逗號分隔雪花 ID，直接封鎖 |

---

## 角色卡格式

### OKF Bundle（推薦）

建立一個目錄，`PERSONA_CARD_PATH` 指向它：

```
persona/my-character/
├── index.md        # 角色設定主體
├── examples.md     # 對話範例（BM25 索引，每輪注入 top-2）
└── lore/           # Lorebook（可選）
    └── world.md    # 包含 trigger_words 的世界觀條目
```

`index.md` 格式：

```markdown
---
type: Character
title: 角色名稱
---

你是……（角色設定正文）
```

`examples.md` 格式（`{{user}}` / `{{char}}` 為固定佔位符）：

```markdown
---
type: Dialogue Examples
---

{{user}}: 你好啊
{{char}}: 哼，誰跟你好了……（但臉有點紅）

{{user}}: 你在想什麼？
{{char}}: 才、才沒有在想什麼！你少多管閒事！
```

`lore/` 下的條目格式：

```markdown
---
trigger_words: 學校, 班級, 同學
---

她就讀某高中二年級，班上有……
```

### SillyTavern V2 JSON

`PERSONA_CARD_PATH` 指向 `.json` 檔：

```json
{
  "spec": "chara_card_v2",
  "data": {
    "name": "角色名",
    "system_prompt": "你是……",
    "first_mes": "第一句話",
    "mes_example": "<START>\n{{user}}: …\n{{char}}: …\n<END>"
  }
}
```

---

## Slash Commands

| 指令 | 說明 |
|---|---|
| `/search <query>` | 強制觸發網路搜索 |
| `/reset` | 清除此頻道對話歷史 |
| `/stop` | 中斷正在生成的回覆 |
| `/persona` | 顯示目前載入的角色卡 |
| `/help` | 顯示所有指令 |

---

## Crate 架構

專案以嚴格分層的 workspace 組織，依賴方向只從上往下：

```
g10kz-bot        主 binary（daemon / once 兩個執行模式）
    │
g10kz-discord    Serenity 閘道：過濾、附件抽取、slash commands、旁觀寫入
    │
g10kz-engine     一回合狀態機：串接所有組件，實作 5 路由邏輯
    │
g10kz-everos     EverOS HTTP 客戶端：add_turn / observe / flush / search
g10kz-tools      工具箱：WebSearch / FetchPage / TwStock / Time / Escalate
    │
g10kz-llm        OpenAI 相容 HTTP 客戶端：SSE 串流 / Fusion / Mock
g10kz-kernel     路由 / guard / JPAF / sanitize / 角色卡載入
    │
g10kz-config     型別化設定，無任何內部依賴
```

每個 crate 只知道它下面那層的存在，沒有反向依賴。`g10kz-llm` 設計為 provider-agnostic，`LLM_PROVIDER=mock` 時完全不發任何網路請求，方便本地測試。

**本地測試（不需要 Discord）**

```bash
cargo run -p g10kz-bot -- once "你好，自我介紹一下"
```

> `once` 模式使用內建 stub persona，`PERSONA_CARD_PATH` 設定在此模式下不生效。

---

## License

[AGPL-3.0](LICENSE) © 2026 g8kz

