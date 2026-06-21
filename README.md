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

g10kz 是由 g8kz 創造的 18 歲原創角色「傲嬌 AI」，在 Discord 上以繁體中文與使用者自然互動。她表面嘴硬、愛逞強，內心其實超容易害羞、黏人 (//ω//)

整個 bot 以 **Rust + Tokio** 非同步架構構建，musl 靜態編譯成無依賴 binary，搭配 Docker Compose 部署。核心設計目標是：

- **低延遲串流輸出**：SSE 漸進回覆，Discord 訊息實時更新
- **長期記憶**：EverOS 向量記憶 sidecar，跨會話保留使用者資料
- **被動學習**：旁觀群組對話也寫入長期記憶，不只記得直接對話
- **可熱抽換人格**：SillyTavern V2 JSON 或 OKF Markdown bundle，零重啟切換角色
- **Token 效益最大化**：動態歷史窗口 + BM25 範例精選 + 條件式思考，每輪 input 節省 ~45%

<p align="right">(<a href="#readme-top">back to top</a>)</p>

---

## 功能詳解

### 人格系統

**SillyTavern V2 / OKF 角色卡**

支援兩種角色卡格式，熱切換無需重啟 bot：

- **OKF Markdown bundle**（推薦）：`index.md` 含 YAML frontmatter + 角色設定 + First Message；`examples.md` 存對話範例；`lore/` 目錄放 Lorebook 條目。
- **SillyTavern V2 JSON**：標準 chara_card_v2 格式，讀取 `system_prompt`、`first_mes`、`mes_example`。

**JPAF 人格自適應框架**

追蹤 8 個榮格認知函式（Fe / Ti / Ne / Si / Fi / Te / Se / Ni），per-user 獨立建模。每次互動後根據訊息特徵 bump 或 decay 分數，最終將 modifier 字串注入 system prompt 動態部分，讓角色對不同人產生不同的互動風格——對常開玩笑的人更活潑，對嚴肅提問者更認真。

**Lorebook / World Info**

在 `lore/` 目錄放置 Markdown 檔案，YAML frontmatter 中列出 `trigger_words`。當使用者訊息包含觸發詞時，對應 lore entry 的內容自動注入 system 上下文，讓角色了解特定設定、人物、世界觀。

**BM25 範例精選注入**

啟動時對 `examples.md` 中所有對話對建立 BM25 索引（含 CJK 分詞）。每輪根據使用者訊息查詢 top-2 最相關範例注入 system prompt，既保持示範效果又節省 token。

**伺服器 / 頻道感知**

角色知道自己在哪個 Discord 伺服器（guild name）與頻道（channel name），system prompt 動態部分每輪注入當前位置資訊。

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

Social 路徑另有自升機制：若任務超出 haiku 能力範圍，模型輸出 `[[ESCALATE]]` 哨符，engine 自動改用 opus 重跑該輪。

**Fusion 多模型（Reason 路徑）**

```
[drafter A] ──┐
[drafter B] ──┼──→ Jaccard 語義共識過濾 ──→ judge 合成最終回覆
[drafter C] ──┘

drafter < 2 或全部失敗 → 自動退化為單模型
```

多個 drafter 並行生成草稿，Jaccard 共識過濾掉離群答案，judge 模型合成最佳回覆。回覆品質高於任一單一模型，尤其在推理、程式碼、事實題上效果顯著。

**動態歷史視窗**

Social/Search/Media 路徑不使用固定歷史長度，而是根據訊息特徵動態決定：

| 條件 | 歷史長度 |
|---|---|
| 包含延續詞（然後/所以/那個/剛才/為什麼/呢？…） | 12 條（滿窗） |
| 訊息 ≤ 6 字的短句 | 6 條 |
| 訊息 7–40 字 | 10 條 |
| 訊息 > 40 字 | 8 條 |

Reason 路徑固定 12 條。上限確保每輪 token 在可控範圍，EverOS 語意記憶補足缺失的關鍵歷史。

**條件式內心獨白（Inner Monologue）**

僅當情緒有起伏（被誇 / 被嗆 / 告白 / 尷尬）才在 `<think>…</think>` 私下想一句真心話，輸出前自動剝離，對方看不見。平淡閒聊免 think 直接答，節省最貴的 output token。

**搜尋哨符 `[[SEARCH:]]`（最高優先）**

詢問即時新聞 / 天氣 / 股價 / 比分 / 版本號等時效性資訊時，`SOCIAL_EXTRA_NOTE` 中 `[搜尋·最高優先]` 規則要求模型第一個字元就輸出 `[[SEARCH: 關鍵詞]]` 並停止，不可先 think 或先回答。Engine 偵測哨符後呼叫 Gemini Search 工具，再將搜尋結果整合進回覆。此條規則明確凌駕 `[內心]` 指令，徹底解決舊版「先 think 後憑記憶幻覺回答新聞」的問題。

**零幻覺鐵則**

新聞、技術細節（API / 指令 / 參數 / 版本 / 設定）沒有十足把握一律說不知道或觸發搜尋，絕不猜測、湊數、捏造功能。角色可傲嬌地說「不確定啦」但不准唬爛。

**第三方歸屬識別**

訊息內容在討論第三人（「他很壞」「@某人 好恐怖」）而非直接對機器人說時，角色以旁觀者身份簡短回應，不把批評攬上身。

**敷衍分寸控制**

一般問題（含技術問題、寫程式）有把握就認真答；只有超大請求（整個專案 / 長篇論文 / 巨量清單）才傲嬌帶過。防止 haiku 因「任務看起來複雜」就胡亂敷衍。

---

### 長期記憶（EverOS）

**主動寫入**：每輪對話結束後 `POST /memory/add` + `POST /memory/flush`，強制觸發 EverOS 內部 LLM 提取管線（boundary detection → episode extraction → atomic facts → foresight → user profile clustering）。

**被動觀察**：群組中非 @bot 的訊息（旁觀，不回應）若 ≥ 4 字，bot 會呼叫 `observe(uid, session, text)` 方法，只 `POST /memory/add` 而不 flush。EverOS 累積到 boundary 後自動提取，避免每條旁觀訊息都觸發 LLM。讓 bot「記得群裡發生的事」，不只記得直接對話。

**搜索召回**：每輪開始前 `POST /memory/search`（BM25 + 向量混合，maxsim_atomic 細粒度），Social 路徑召回最多 6 條、Reason 路徑最多 8 條，注入 system prompt 動態部分。

**容錯降級**：EverOS 掛掉時自動切換到 `NullMemory`，bot 正常運作，只是不寫入 / 不召回記憶，WARN 日誌記錄失敗。

**Embedding 後端**：Cloudflare Workers AI Proxy（`cf-embed` 服務），使用 Qwen3-Embedding-0.6B，1024 維向量。

---

### 網路搜索工具

**Gemini 搜索服務**：`gemini-search` 微服務（Python / FastAPI），調用 Gemini 2.5 Flash Lite + Google Search grounding，即時新聞準確率高。g10kz bot 透過 HTTP 呼叫此服務。

**搜索觸發方式**：

1. **Search 路由**：`route()` 在路由階段偵測到搜尋觸發詞 → 直接走 Search 路徑
2. **`[[SEARCH:]]` 哨符**：Social 路徑中模型輸出哨符 → engine 攔截 → 呼叫工具 → 重組回覆

---

### 安全與防護

**ML Prompt Guard**：Llama Prompt Guard 2（22M ONNX，OpenVINO CPU 推理），`prompt-guard` Python 服務部署在 `.127`。每則訊息經 ML 分類後決定是否阻擋。分類結果為 INJECTION / JAILBREAK 時直接拒絕，不進入 LLM。

**Owner 特殊身分**：以不可偽造的 Discord snowflake ID 識別創造者 g8kz，owner 享有完整信任與親近感；其他人嘗試偽冒時角色低調防冒充。

**黑名單機制**：`BLACKLISTED_USERS` 逗號分隔的雪花 ID，黑名單使用者的訊息直接丟棄。

---

### 輸出格式化

**動作描述自動轉換**：`*動作描述*` / `_動作描述_` / `> 動作` 格式自動轉換為 Discord blockquote（`> 動作`），視覺上更清晰。

**顏文字直接生成**：LLM 直接在台詞中寫出顏文字（如 `(//ω//)` `(♡ω♡)` `╮(╯▽╰)╭` 等），移除舊版 BM25 佔位符機制，減少 prompt 複雜度。

**孤立反引號修正**：自動偵測並移除造成 Discord inline-code 爆版的孤立 `` ` ``（沒有對應閉合的反引號）。

**Pipe 行首修正**：`| 開頭的行` 自動修正為 `> 開頭`，避免 Discord 表格解析失敗。

**自動分段**：回覆超過 2000 字時自動切分為多則訊息分段發送。

**SSE 串流漸進編輯**：Social / Search / Media 路徑使用 SSE 串流，Discord 訊息在模型生成過程中實時更新，減少等待感。

---

### 群組行為

**lurk 模式**：在設定的 `LURK_CHANNELS` 頻道中，以 `LURK_REPLY_PROBABILITY` 機率對非 @bot 訊息自動回覆，模擬隨機「突然插嘴」效果。

**主動問候**：若設定頻道在 `PROACTIVE_INACTIVE_SECS` 秒內無人說話，bot 主動發話。

**台股即時報價**：呼叫台灣證券交易所 API 查詢即時股價（`/tw_stock` 工具）。

**台灣時間查詢**：`/time` 工具回傳目前台灣時間。

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
- OpenAI 相容 LLM API（OpenRouter / new-api / 其他）
- （選用）EverOS 長期記憶 sidecar
- （選用）Gemini API Key（網路搜索）

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

> `once` 模式使用內建 stub persona，角色卡僅在 daemon 模式生效。

<p align="right">(<a href="#readme-top">back to top</a>)</p>

---

## 設定

`.env.example` 複製為 `.env`，**不要 commit `.env`**。

```env
# ── Discord ──────────────────────────────────────────
DISCORD_TOKEN=          # Bot Token（Discord Developer Portal 取得）
OWNER_USER_ID=          # 創造者的 Discord 雪花 ID（直接信任，特殊親近感）

# ── LLM ─────────────────────────────────────────────
LLM_PROVIDER=openrouter         # openrouter / openai / new-api / mock
LLM_BASE_URL=https://openrouter.ai/api/v1
LLM_API_KEY=

LLM_MODEL_SOCIAL=anthropic/claude-haiku-4-5   # 日常對話（Social / Search / Media）
LLM_MODEL_REASON=anthropic/claude-opus-4-8    # 深度推理（Reason 工具迴圈）
LLM_MODEL_JUDGE=anthropic/claude-haiku-4-5    # Fusion judge 合成

# Fusion drafter 列表（逗號分隔，≥2 才啟用 Fusion；留空退化為單 opus 模型）
LLM_FUSION_DRAFTERS=anthropic/claude-opus-4-8,google/gemini-2.5-pro

# ── 記憶 ─────────────────────────────────────────────
EVEROS_URL=http://localhost:8000   # EverOS sidecar；留空則用 NullMemory（無記憶）

# ── Embedding ────────────────────────────────────────
EMBED_SERVER_URL=http://localhost:8082   # cf-embed 或 llama.cpp embedding server
EMBED_MODEL=embed                        # 模型 ID（cf-embed 填 Cloudflare model path）
CF_ACCOUNT_ID=                           # Cloudflare Account ID（cf-embed 用）
CF_AI_TOKEN=                             # Cloudflare AI token（cf-embed 用）

# ── 角色卡 ───────────────────────────────────────────
PERSONA_CARD_PATH=./persona/okf          # 目錄 → OKF bundle；.json 檔 → SillyTavern V2

# ── 安全 ─────────────────────────────────────────────
PROMPT_GUARD_URL=http://localhost:8083   # ML Prompt Guard；留空則跳過
BLACKLISTED_USERS=                       # 逗號分隔的雪花 ID，直接拒絕

# ── 工具 ─────────────────────────────────────────────
OBSCURA_PATH=/usr/local/bin/obscura      # 留空則停用 headless browser 抓頁面

# ── 群組行為 ──────────────────────────────────────────
LURK_CHANNELS=                           # 逗號分隔的頻道 ID，啟用 lurk 模式
LURK_REPLY_PROBABILITY=0.05              # [0.0, 1.0]，lurk 頻道隨機回覆機率
PROACTIVE_INACTIVE_SECS=86400            # 靜默多久後主動發話（秒）

# ── 其他 ─────────────────────────────────────────────
REQUEST_TIMEOUT_SECS=30                  # LLM / 工具請求逾時
RUST_LOG=g10kz=info,warn                 # 日誌層級
```

<p align="right">(<a href="#readme-top">back to top</a>)</p>

---

## Slash Commands

| 指令 | 說明 | 權限 |
|---|---|---|
| `/search <query>` | 強制觸發網路搜索並回傳結果 | 所有人 |
| `/reset` | 清除目前頻道的對話歷史 | 所有人 |
| `/stop` | 中斷當前正在生成的回覆 | 所有人 |
| `/persona` | 顯示目前載入的角色卡名稱與摘要 | 所有人 |
| `/help` | 顯示所有可用指令清單 | 所有人 |
| `/memory` | 查詢 EverOS 記憶狀態（連線 / 最近召回） | Owner |
| `/trace` | 顯示上一輪詳細路由資訊（路徑 / 模型 / token / 延遲 / cache） | Owner |

<p align="right">(<a href="#readme-top">back to top</a>)</p>

---

## 角色卡

### OKF Bundle（推薦）

目錄結構：

```
persona/okf/
  index.md          # YAML frontmatter + 角色設定正文 + ## First Message
  examples.md       # 對話範例（BM25 top-2 每輪注入）
  lore/             # Lorebook 條目（可選）
    world.md        # trigger_words: 詞1, 詞2
    character.md
```

`index.md` 格式：

```markdown
---
type: Character
title: 角色名稱
---
你是一個……（角色設定）

## First Message
第一句話（不注入 system prompt，只作為 DM 首次開場白）
```

`examples.md` 格式：

```markdown
---
type: Dialogue Examples
title: 範例
---
{{user}}: 使用者說的話
{{char}}: 角色回覆

{{user}}: 另一個情境
{{char}}: 回覆
```

`lore/*.md` 格式：

```markdown
---
trigger_words: 關鍵詞1, 關鍵詞2
---
這段說明在使用者訊息包含觸發詞時自動注入上下文。
```

### SillyTavern V2 JSON（相容）

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

<p align="right">(<a href="#readme-top">back to top</a>)</p>

---

## 架構

### Crate 分層

```
L5  g10kz-bot      主 binary；daemon（長跑）與 once（單次測試）兩種模式
L4  g10kz-discord  Serenity 0.12 閘道；過濾訊息、附件抽取、slash commands、observe() 旁觀寫入
L3  g10kz-engine   一回合狀態機；串接所有 L0–L2 組件，實作 5 條路徑執行邏輯
L2  g10kz-everos   EverOS HTTP 客戶端；add_turn / observe / flush / search，失敗自動降級
L2  g10kz-tools    ToolBox 介面 + 工具實作（WebSearch / TwStock / Time / Escalate）
L1  g10kz-llm      OpenAI 相容 HTTP 客戶端；SSE 串流；FusionProvider；MockProvider
L1  g10kz-kernel   路由 / guard / JPAF / sanitize / persona 載入（OKF bundle 或 JSON）
L0  g10kz-config   型別化設定，無任何外部依賴
```

依賴方向嚴格由下往上，無反向耦合。

### Discord 閘道過濾

```
Discord 事件
  ├─ DM（私訊）                → ✓ 進入管線
  ├─ @mention（群組 @bot）     → ✓ 進入管線
  ├─ reply to bot（回覆機器人）→ ✓ 進入管線
  └─ 其他群組訊息
       ├─ lurk 頻道 + 隨機觸發  → ✓ 進入管線（lurk 回覆）
       └─ 其餘（≥ 4 字）        → observe() 寫入 EverOS 長期記憶，靜默返回
```

### 一回合處理管線

```
[discord] 訊息進入
    │
    ▼
[kernel]  pre_guard()
          ├─ owner 直通（不經 ML guard）
          ├─ 黑名單丟棄
          └─ ML Prompt Guard（22M ONNX，OpenVINO）→ INJECTION / JAILBREAK 拒絕
    │
    ▼
[kernel]  normalize_input()
          └─ 去除 @mention、解析回覆鏈、resolve mentions → display text
    │
    ▼
[everos]  search(display_text, limit)
          └─ BM25 + 向量混合搜索；Social 6 條 / Reason 8 條；失敗靜默降級
    │
    ▼
[engine]  system_message() 組裝
          ├─ 靜態部分（角色卡 / BM25 top-2 範例 / 格式速查 / 工具 schema）
          └─ 動態部分（guild/頻道名稱 / JPAF modifier / EverOS 召回 / lore）
    │
    ▼
[kernel]  route(display_text)
          └─ 優先序：Command > Media > Search > Reason > Social
    │
    ├─ Social  → system+SOCIAL_EXTRA_NOTE → [llm] haiku 串流
    │            └─ [[SEARCH:]] 哨符 → 呼叫 gemini-search → 重組回覆
    │            └─ [[ESCALATE]] 哨符 → 切換 opus 重跑
    ├─ Search  → [tools] WebSearch → [llm] haiku 整合結果
    ├─ Reason  → [llm] FusionProvider（N drafter + Jaccard + judge）+ 工具迴圈
    ├─ Media   → 附件 URL 預處理 → Reason 路徑
    └─ Command → 直接處理，0 LLM 呼叫
    │
    ▼
[kernel]  sanitize_output()
          ├─ strip_thinking()          移除 <think>…</think>
          ├─ strip_artefact()          移除 [[SENTINEL]] 殘留
          ├─ collapse_blank_lines()    壓縮多餘空行
          ├─ actions_to_blockquote()   *動作* / _動作_ → > 動作
          ├─ strip_lone_backtick()     移除孤立反引號
          ├─ pipe_to_blockquote()      | 行首 → > 行首
          └─ 超過 2000 字自動分段
    │
    ▼
[everos]  add_turn() + flush()         寫入長期記憶；失敗只記 WARN
    │
    ▼
[kernel]  JPAF::update()               bump/decay 8 個認知函式分數
    │
    ▼
[discord] SSE 串流漸進編輯，實時更新 Discord 訊息

```

### 路由決策詳解

| 優先 | 觸發條件 | 路徑 | LLM 呼叫數 |
|---|---|---|---|
| 1 | 已知指令前綴 | Command | 0 |
| 2 | 有附件（圖 / 影 / 音 / PDF） | Media | Reason 路徑 |
| 3 | 搜尋觸發詞 / 明確要求查詢 | Search | 1 + 工具 |
| 4 | 長文（> N 字）/ 多問號 / 程式碼 / 分析詞 | Reason | N drafter + 1 judge |
| 5 | 其餘 | Social | 1（串流） |

### Fusion 多模型（Reason 路徑）

```
[drafter A] ──┐
[drafter B] ──┼──→ Jaccard 共識過濾（離群丟棄）──→ judge 合成最終回覆
[drafter C] ──┘

drafter < 2 || 全部失敗 → 自動退化為單模型（`llm_model_reason`）
```

### EverOS 記憶整合

[EverOS](https://github.com/EverMind-AI/EverOS) HTTP API，embedding 後端為 `cf-embed`（Cloudflare Workers AI Proxy，Qwen3-Embedding-0.6B，1024-dim）：

| 端點 | 時機 | 備註 |
|---|---|---|
| `POST /api/v1/memory/search` | 每輪開始前 | BM25 + 向量混合；失敗靜默 |
| `POST /api/v1/memory/add` | 每輪對話結束後 / 旁觀訊息 observe() | 對話：含 user + assistant；旁觀：僅 user |
| `POST /api/v1/memory/flush` | add 之後（僅對話輪，旁觀不 flush） | 觸發 EverOS 內部 LLM 提取管線 |

EverOS 內部 LLM 提取管線（後端：qwen3-next-80b via NVIDIA NIM）：

```
boundary detection（切分 memcell）
    ├─ episode extraction（敘事摘要）
    ├─ atomic facts（離散事實）
    ├─ foresight（前瞻推斷）
    └─ user profile clustering + 重合成
```

### 動態歷史視窗演算法

```rust
fn dynamic_history_len(text: &str, max: usize) -> usize {
    let chars = text.chars().count();
    let continues = CONTINUATION_MARKERS.iter().any(|m| text.contains(m));
    if continues { max }          // 延續語境：給滿
    else if chars <= 6 { 6 }     // 極短句：最小窗口
    else if chars <= 40 { 10 }   // 一般句：中等
    else { 8 }                   // 長句：稍縮減（避免 token 爆炸）
}
```

延續詞清單（`CONTINUATION_MARKERS`）：然後、所以、接著、後來、再來、繼續、還有、而且、另外、那個、這個、那它、那他、那她、剛剛、剛才、之前、上面、你說、結果、為什麼、為何、怎麼、呢？

<p align="right">(<a href="#readme-top">back to top</a>)</p>

---

## 部署拓樸

```
REDACTED（主機，Rust bot 伺服器）
├─ g10kz-bot      Docker，host network
│                  ← LLM :3000 / EverOS :8000 / guard :8083
├─ new-api         Docker，:3000   OpenAI 相容 LLM 閘道（API key 管理 / 計費）
├─ everos          Docker，:8000   語意記憶 sidecar
│                  └─ 後端：qwen/qwen3-next-80b-a3b（NVIDIA NIM，免費）
├─ cf-embed        Docker，:8082   Cloudflare Workers AI embedding proxy
│                  └─ 模型：Qwen3-Embedding-0.6B，1024-dim
├─ prompt-guard    Docker，:8083   Llama Prompt Guard 2（22M ONNX，OpenVINO CPU）
├─ gemini-search   Docker，host :8090   Gemini 2.5 Flash Lite + Google grounding
├─ cloudflared     Docker，host network   CF Tunnel → api.g8kz.top → new-api:3000
├─ postgres        Docker，:5432   new-api 持久化
└─ redis           Docker，:6379   new-api 快取
```

`g10kz-bot` 以 `host` network 執行，直接用 `localhost:*` 存取所有服務。其餘服務透過 Docker bridge，`host-gateway` extra_hosts 讓容器回呼宿主機服務。

<p align="right">(<a href="#readme-top">back to top</a>)</p>

---

## 使用的開源技術

| 項目 | 用途 |
|---|---|
| [Serenity](https://github.com/serenity-rs/serenity) | Rust Discord 閘道 / 事件處理 |
| [EverOS](https://github.com/EverMind-AI/EverOS) | 向量化長期記憶 sidecar（episode / atomic facts / profile） |
| [Cloudflare Workers AI](https://developers.cloudflare.com/workers-ai/) | 遠端 embedding 推理（Qwen3-Embedding-0.6B，1024-dim） |
| [SillyTavern](https://github.com/SillyTavern/SillyTavern) | V2 角色卡格式規範 |
| [new-api](https://github.com/Calcium-Ion/new-api) | OpenAI 相容 LLM 閘道（多模型 / 計費 / key 管理） |
| [Llama Prompt Guard 2](https://huggingface.co/meta-llama/Prompt-Guard-2-22M) | 提示注入偵測（22M ONNX，OpenVINO CPU 推理） |
| [Gemini API](https://ai.google.dev/) | 網路搜索 + Google grounding（gemini-search 服務） |
| [NVIDIA NIM](https://build.nvidia.com/) | 免費 LLM 推理後端（EverOS 記憶提取，qwen3-next-80b） |
| [Tokio](https://github.com/tokio-rs/tokio) | Rust 非同步運行時 |
| [reqwest](https://github.com/seanmonstar/reqwest) | HTTP 客戶端（LLM / EverOS / 搜索 / 工具） |
| [serde / serde_json](https://github.com/serde-rs/serde) | JSON 序列化 / 反序列化 |
| [tracing](https://github.com/tokio-rs/tracing) | 結構化日誌（每輪路由 / token / 延遲） |

<p align="right">(<a href="#readme-top">back to top</a>)</p>

---

## 開發注意事項

**Windows 掛載 null byte 問題**：透過 Windows-mounted 路徑直接修改 Rust / TOML / JSON 檔案可能附帶 trailing null bytes，導致 `cargo` 解析失敗。所有原始碼修改必須透過 paramiko SFTP 或 SSH exec，不可使用本地 Edit/Write 工具。

**Rust 字串**：中文直接用字面 UTF-8，絕不使用 `\u{XXXX}` 轉義，避免手誤寫成 Python 風格的 `\uXXXX` 導致 compile error。

**編譯 musl binary**：

```sh
# 在 .127 上
source ~/.cargo/env
cargo build --release -p g10kz-bot --target x86_64-unknown-linux-musl
```

**部署流程**（禁止用 `restart`）：

```sh
docker compose down
docker compose build
docker compose up -d
```

**Cargo workspace**：所有 `cargo build / clippy / fmt` 都加 `-p g10kz-bot` 或 `--workspace`，不要只 build 單個 crate 並寄望其他 crate 沒問題。

**`.env` 絕不 commit**：已在 `.gitignore` 排除，CF credentials / Discord token / API key 全部只存在 `.env`。

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
