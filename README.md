# g10kz

傲嬌 AI Discord 機器人，以 Rust 實作的八層 crate workspace。角色「g10kz」由 g8kz 創造，18 歲，個性傲嬌反差、大量顏文字、用繁體中文回覆。

---

## 功能特色

- **傲嬌人格** — SillyTavern V2 角色卡驅動，支援熱抽換；預設角色 g10kz 具完整口癖、顏文字與傲嬌反差萌
- **JPAF 人格適應框架** — 榮格認知功能（Fe/Ti/Ne/Si/Te/Fi/Se/Ni）per-user 建模，每輪對話自動 bump/decay，動態注入 system prompt modifier，讓角色對不同使用者產生差異化互動風格
- **多路由引擎** — 同一訊息依內容自動走 Social / Search / Reason / Media / Command 五條路徑，成本最小化
- **Fusion 多模型** — Reason 路徑並行多個 drafter → Jaccard 共識過濾 → judge 模型合成，回覆品質優於單模型
- **工具迴圈** — Obscura 防偵測瀏覽器 + DuckDuckGo Lite 網路搜尋（BM25 相關段落萃取）、台灣股市即時報價、當前台灣時間
- **EverOS 語意記憶** — EverOS 1.0 HTTP sidecar 深度整合；每輪對話 add → flush，回覆前 search 注入上下文；掛掉自動降級為 NullMemory，bot 不崩
- **Discord Markdown** — 每個 system prompt 注入格式指引，讓 LLM 懂得在 Discord 正確使用 **粗體**、*斜體*、`code`、`> 引用`、`-# 小字`、`||劇透||`、標題與超連結
- **伺服器 / 頻道感知** — `[伺服器環境]` 區塊注入 system prompt，包含 guild 名稱、頻道名稱；讓角色能依環境調整語氣與行為
- **Prompt Token 最佳化** — 語義去重 persona 骨架（1073→520 tok）+ prefix-cache 靜態/動態分離；每輪 system token −45%，快取命中時等效 −89%
- **輸出 sanitize** — 提示注入防禦、`[角色名]` 前綴標籤自動剝除、反重複偵測，0 LLM 成本
- **主動發話** — 頻道靜默超過設定時間後，bot 主動傳訊
- **媒體處理** — 附件 URL 傳入引擎，影片走 ffmpeg 抽幀（需容器內有 ffmpeg）
- **可離線測試** — `LLM_PROVIDER=mock` 全離線執行，CI 零網路全綠

---

## 架構：八層 Crate Workspace

依賴方向由下往上，無反向耦合：

```
L5  g10kz-bot        ← 主 binary（daemon / once）
L4  g10kz-discord    ← Serenity 0.12 閘道、附件抽取、slash commands
L3  g10kz-engine     ← turn 狀態機，串接所有 L0-L2 組件
L2  g10kz-everos     ← EverOS 1.0 HTTP 記憶客戶端
L2  g10kz-tools      ← ToolBox 介面、WebSearch / TwStock / Time / Escalate
L1  g10kz-llm        ← OpenAI 相容供應層、FusionProvider、MockProvider
L1  g10kz-kernel     ← 路由（route）、guard、normalize、persona card 載入、JPAF
L0  g10kz-config     ← 從環境變數載入的型別化設定，無任何依賴
```

### 一回合的資料流

```
Discord 訊息
  → 應答閘（DM / @mention / reply to bot）
  → 附件抽取（image/audio/video URL）
  → run_turn:
      guard::pre          ── 純函式提示注入防禦（0 LLM）
      normalize           ── 去 mention、前綴解析
      EverOS::search      ── 語意記憶注入（HTTP, 降級安全）
      system_message()    ── [靜態前綴 + cache_control] + [動態後綴]
                             靜態：persona + channel note + Discord 格式指引
                             動態：伺服器環境 + JPAF modifier（不破壞快取前綴）
      route()             ── Social / Search / Reason / Media / Command
          │
          ├─ Social  → 單次 social model call（最便宜）
          ├─ Search  → WebSearchTool（DDG+Obscura+BM25）→ social model 整合回覆
          ├─ Reason  → FusionProvider（N drafter 並行 → judge 合成）+ 工具迴圈
          ├─ Media   → 附件 URL 帶入 reason 路徑
          └─ Command → 直接處理（reset/stop/search/memory/trace/help）
      sanitize_output     ── 剝除 [標籤] 前綴、提示注入偵測、反重複
      EverOS::add + flush ── 本輪對話寫入記憶
      JPAF::update        ── 根據本輪訊號 bump/decay 認知函式分數
  → Discord 分段發送（>2000 字自動切割）
```

---

## 路由決策

`g10kz-kernel/src/route.rs` 的純函式決策梯（由上到下，優先順序依序）：

| 優先順序 | 條件 | 路徑 |
|---|---|---|
| 1 | `/cmd` 或 `!cmd` 已知指令 | Command |
| 2 | 有附件 | Media |
| 3 | 搜尋觸發詞（搜尋/幫我查/股價/天氣…） | Search |
| 4 | 複雜度訊號（長文 >250 字/多問號/程式碼區塊/分析關鍵詞） | Reason |
| 5 | 其餘 | Social |

---

## Fusion 多模型（Reason 路徑）

```
[drafter A] ─┐
[drafter B] ─┼─→ Jaccard 共識過濾 → judge 模型合成最終回覆
[drafter C] ─┘
```

- `LLM_FUSION_DRAFTERS`：逗號分隔的 drafter 模型清單
- `LLM_MODEL_JUDGE`：judge 模型（任意 OpenAI 相容模型）
- drafter 少於 2 個或全失敗時，自動降級為單模型直答

---

## 網路搜索（Obscura + DuckDuckGo Lite）

Search 路徑使用三層流水線：

```
用戶查詢
  → POST DuckDuckGo Lite HTML（https://lite.duckduckgo.com/lite/）
  → 解析 result-link / result-snippet，取前 5 筆
  → Obscura headless browser fetch 前 3 頁全文（防偵測，CDP）
  → BM25 相關段落萃取（按查詢詞命中率排序，取 top N 至 max_chars）
  → Discord Markdown 格式化輸出
```

Obscura 二進位放在 `bin/obscura`（gitignore），透過 Dockerfile `COPY` 打入容器。`OBSCURA_PATH` 未設或路徑不存在時，自動降級為僅回傳 DDG snippets。

---

## JPAF 人格適應框架

每位使用者維護 8 個榮格認知函式分數（Fe/Ti/Ne/Si/Te/Fi/Se/Ni），初始值均等：

- 每輪對話依訊息特徵（情緒詞、邏輯詞、創意詞、細節詞等）**bump** 對應函式
- 所有分數每輪輕微 **decay**，讓模型自然回歸基線
- 主導函式（最高分）注入 system prompt `[人格動態]` modifier，影響角色語氣與優先回應策略

---

## EverOS 記憶整合

使用 [EverOS 1.0](https://github.com/EverMind-AI/EverOS) HTTP API：

| 端點 | 時機 | 說明 |
|---|---|---|
| `POST /api/v1/memory/add` | 每輪對話後 | 寫入 user + assistant 訊息 |
| `POST /api/v1/memory/flush` | add 之後 | 觸發向量化與持久化 |
| `POST /api/v1/memory/search` | 每輪開始前 | 語意搜尋注入上下文 |

embedding 後端：`llama-embed`（llama.cpp server-cuda），`Qwen3-Embedding-0.6B-Q8_0.gguf`（1024-dim，~112ms/次）。

---

## Prompt Token 最佳化

詳見 [`docs/prompt-token-optimization.md`](docs/prompt-token-optimization.md)。

### Before / After（cl100k 量測）

| 路徑 | Before | After（純去重） | 快取命中等效 |
|---|---:|---:|---:|
| Social / Search | 1699 tok | **923 tok（−45%）** | **≈186 tok（−89%）** |
| Reason | 1880 tok | **1018 tok（−46%）** | — |

### 語義去重

`persona/g10kz.json` 四欄（system_prompt / description / personality / scenario）合併為單一無重複骨架，channel_note、discord_format_note、tool_schema 各自精簡。

### Prefix-Cache 靜態/動態分離

`system_message()` 回傳兩-part Message：

```
part 0  靜態前綴 ← cache_control 在這裡
        persona + channel_note + discord_format [+ tool_schema（Reason）]
        ↑ 跨所有頻道/用戶/turn 逐字節相同 → KV-cache 命中

part 1  動態後綴 ← 不快取
        env_note（guild/channel 名）+ JPAF modifier
```

---

## 快速開始

### 前置需求

- Rust 1.88+（`rust:slim-bookworm`）
- Docker + Docker Compose
- Discord bot token
- OpenAI 相容 LLM API（OpenRouter、new-api 等）

### 本地執行（無 Discord）

```bash
cp .env.example .env

# 離線 smoke test
LLM_PROVIDER=mock cargo run -p g10kz-bot -- once "你好，自我介紹一下"
```

### Docker 部署

```bash
docker build -t g10kz-bot:latest .
docker compose up -d
docker logs g10kz-bot -f
```

---

## 環境變數

複製 `.env.example` 為 `.env`，**永遠不要 commit `.env`**。

```env
# Discord
DISCORD_TOKEN=
OWNER_USER_ID=

# LLM
LLM_PROVIDER=openrouter
LLM_BASE_URL=https://openrouter.ai/api/v1
LLM_API_KEY=

LLM_MODEL_SOCIAL=openai/gpt-4o-mini
LLM_MODEL_REASON=openai/gpt-4o
LLM_MODEL_JUDGE=anthropic/claude-3-5-haiku
LLM_FUSION_DRAFTERS=openai/gpt-4o,anthropic/claude-3-5-sonnet,google/gemini-2.0-flash

# 記憶 sidecar（留空則用 NullMemory）
EVEROS_URL=http://localhost:8000

# 角色卡
PERSONA_CARD_PATH=./persona/g10kz.json

# 網路搜索（留空則僅 DDG snippet）
OBSCURA_PATH=/usr/local/bin/obscura

# 調優
PROACTIVE_INACTIVE_SECS=86400
REQUEST_TIMEOUT_SECS=30
BLACKLISTED_USERS=

RUST_LOG=g10kz=info,warn
```

---

## 角色卡（SillyTavern V2）

`persona/` 目錄存放 SillyTavern V2 格式的 JSON 角色卡。`system_prompt` 欄位為唯一必填，其餘欄位若有內容依序拼接。

```json
{
  "spec": "chara_card_v2",
  "spec_version": "2.0",
  "data": {
    "name": "g10kz",
    "system_prompt": "你是 g10kz，...",
    "first_mes": "...",
    "mes_example": "<START>\n{{user}}: ...\n{{char}}: ...\n<END>"
  }
}
```

> `g10kz-bot once` 模式使用內建 stub persona，角色卡僅在 daemon 模式下生效。

---

## Slash Commands

| 指令 | 說明 | 權限 |
|---|---|---|
| `/reset` | 清除當前頻道對話記憶 | 全員 |
| `/stop` | 取消正在進行的回覆 | 全員 |
| `/search <query>` | 強制走 Search 路徑查詢 | 全員 |
| `/memory` | 查詢 EverOS 記憶狀態 | Owner |
| `/persona` | 顯示當前角色卡名稱 | 全員 |
| `/trace` | 顯示上一次 turn 的路由資訊 | Owner |
| `/help` | 指令清單 | 全員 |

---

## 部署拓樸

```
REDACTED
├─ new-api        :3000   OpenAI 相容閘道
├─ everos         :8000   ┐
├─ llama-embed    :8082   ├─ g10kz-memory stack（GPU 推論）
└─ ollama         :11434  ┘
└─ g10kz-bot      host    network_mode: host
                          persona/ 掛載，Obscura 打入 image
```

---

## CI

每次 push main 自動執行 fmt → clippy → test → smoke test（`LLM_PROVIDER=mock once "你好小十"`）。

---

## License

MIT
