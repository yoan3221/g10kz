# g10kz v5 架構（完整重構）

> 傲嬌人設 Discord 機器人「小十」—— 在 v4 的 12-crate Rust workspace 基礎上完整重構。
>
> 本代重點：**EverOS 接管記憶層**、**原生 FusionProvider 多模型推理（不綁 OpenRouter）**、**全管線多模態**。

---

## 一句話定位

保留 v4 證明有效的骨架（分層 workspace、decorator LLM 供應層、四路上下文、mini 路由器、owner 防護、可離線 mock 測試），但把三件事換掉/補上：

1. **記憶層** v4 的 `g10kz-memory`（L1/L2/L3）+ `g10kz-vector`（Cloudflare Vectorize）→ **EverOS sidecar**（Python 服務，HTTP 通訊，Markdown 真相源 + 雙軌記憶 + 自演化）。Bot 本體仍 100% Rust。
2. **多模型推理** 在 `g10kz-llm` 新增 **原生 `FusionProvider` decorator**：自己對 N 個 OpenAI-相容後端並行 fan-out + judge 合成。**provider-agnostic，任何第三方 API 都能用**，可離線 mock。
3. **多模態** 從 `g10kz-discord` 抽附件 → `g10kz-llm` 訊息支援 content parts（text/image/audio）→ pipeline 走 vision 模型；附件同步攝取進 EverOS。

CF AI Search 定位為**選用的網頁搜尋工具**，放在 `g10kz-mcp`，與既有 Gemini grounded 搜尋並列（沒有也能跑）。

---

## 組件對應（你列的五個 → v5 落點）

| 列出的組件 | v5 落點 | 說明 |
|---|---|---|
| **EverOS** | `g10kz-everos`（新 crate，HTTP client） | 記憶大腦，取代 v4 的 memory + vector |
| **OpenRouter Fusion** | `g10kz-llm` 的 `FusionProvider` | 原生實作 fusion 模式，不綁 OpenRouter |
| **Cloudflare AI Search** | `g10kz-mcp` 的 web-search 工具 | 選用；網頁/知識檢索 |
| **SillyTavern** | `g10kz-core` 人設 | 讀取 Character Card V2 JSON 當人設來源 |
| **discord.js** | `g10kz-discord` | 已用 serenity 取代，本代延續 |

---

## Crate 分層（v5）

依賴方向由下往上，無反向耦合：

```
┌─ L5  g10kz-bot ──────────── 組裝所有 crate 的 binary（daemon / once / proactive）
├─ L4  g10kz-discord ──────── serenity 閘道 + 附件抽取        g10kz-web ── axum 後台
├─ L3  g10kz-pipeline ─────── 每回合狀態機（多模態）
├─ L2  g10kz-everos ── EverOS HTTP 記憶  g10kz-tools ──▶ g10kz-mcp（時間/台股/CF 網搜/影片抽幀）
├─ L1  g10kz-core ── 純領域（人設/guard/反重複/黑名單策略） g10kz-llm ── 供應層 + FusionProvider  g10kz-db ── SQLite 短期快取（群組窗口/黑名單/互動時間）
└─ L0  g10kz-config ───────── 型別化設定（無依賴，所有人的根）
```

與 v4 的差異：
- **移除** `g10kz-memory`、`g10kz-vector`（職責移交 EverOS）
- **新增** `g10kz-everos`（EverOS HTTP 客戶端，含優雅降級）
- `g10kz-db` 瘦身為**短期回合快取**（近期 history + owner/狀態），長期語意記憶全交 EverOS
- `g10kz-llm` 增 `FusionProvider`，`g10kz-mcp` 增 CF 網搜工具

---

## 一回合的資料流（v5）

```
Discord 訊息 → 應答閘（DM 或 @mention）→ 抽附件 → run_turn:

  guard::pre ──────── owner 防護 + 提示注入防禦（純函式，0 LLM，最先擋）
        │
  normalize ───────── 字串正規化：去 mention、解析前綴（純函式，不用 embedding）
        │
  context::gather ── 三路並行（tokio::join!）：
        │               ① 近期 history（SQLite 短期快取，快、離線）
        │               ② EverOS 語意召回（/memory/search，user episodes + agent skills）
        │               ③ 環境（誰/在哪，SQLite）
        │            → 去重合併（語意 > 近期 > 環境）
        │            → EverOS 失敗則靜默降級為純 SQLite，bot 不崩
        │
  judge::route ───── ★單一 mini 路由器：分類 + escalate +（純社交時）直接草擬回覆
        │             多模態輸入（有圖/音）→ 強制 escalate 到 vision 模型
        │
   ┌────┴───────────────────────────────────┐
   guard_block?  → render::refuse            （owner 攻擊，0 LLM）
   escalate?     → think → render::emotion   （查資料/嚴謹題/多模態：FusionProvider + 工具迴圈）
   有 reply?     → render::direct            （社交快路徑，全程 1 次 mini call）
   else          → render::emotion           （安全網）
        │
  persist ───────── 短期回合落 SQLite + 非同步推送對話到 EverOS /memory/add
                    （附件一併攝取，EverOS 多模態抽取成可搜尋記憶）
```

**成本控制原則不變**：社交對話只打 1 次 mini call；Fusion 只在 escalate 路徑啟用，不浪費在閒聊。

---

## 需求一：原生 FusionProvider（要能用第三方 API）

OpenRouter 官方 Fusion 是綁定 OpenRouter endpoint 的 plugin。要「能用第三方 API」，本代在 `g10kz-llm` **原生實作 fusion 模式**，沿用 v4 的 decorator 疊法：

```
BudgetedProvider → FusionProvider → RetryProvider → LoadBalancedProvider → OpenAI 相容後端
   額度守門          多模型 fan-out    退避重試          池內 failover         （ai.kot.gg / 任何第三方）
                     + judge 合成
```

`FusionProvider` 行為：
- 對設定的 N 個模型**並行**呼叫（`tokio::join!`），每路各自走內側的 Retry/LoadBalance
- 收集所有回覆，交給一個 **judge 角色**（可指定任意第三方模型）合成最終答案
- 任一路失敗 → 仍用成功的子集合成；全失敗 → 降級為單模型直答
- `mock` provider 下可決定性測試（固定多路輸出 → 固定 judge 輸出）

```rust
// crates/g10kz-llm/src/fusion.rs
pub struct FusionProvider<P> {
    inner:        P,                 // 內側 provider（Retry → LoadBalance → backend）
    panel:        Vec<String>,       // 並行模型清單，可由 env 覆寫
    judge_model:  String,            // 合成模型（第三方任選）
    enabled:      bool,              // config 開關，可整個關掉退回單模型
}

#[async_trait]
impl<P: Provider> Provider for FusionProvider<P> {
    async fn complete(&self, req: ChatRequest) -> Result<ChatResponse> {
        if !self.enabled || self.panel.len() < 2 {
            return self.inner.complete(req).await;          // 降級：單模型
        }
        // 1. 並行 fan-out
        let futures = self.panel.iter().map(|m| {
            let mut r = req.clone();
            r.model = m.clone();
            self.inner.complete(r)
        });
        let results = futures::future::join_all(futures).await;
        let drafts: Vec<String> = results.into_iter()
            .filter_map(|r| r.ok().map(|x| x.text))
            .collect();
        if drafts.is_empty() {
            return self.inner.complete(req).await;          // 全失敗 → 降級
        }
        // 2. judge 合成
        let judge_req = build_judge_request(&self.judge_model, &req, &drafts);
        self.inner.complete(judge_req).await
    }
}
```

設定（`.env`，沿用 v4 風格）：

```
FUSION_ENABLED=true
FUSION_PANEL=claude-opus-4-8,gpt-4o,gemini-2.0-flash      # 第三方任選
FUSION_JUDGE=claude-opus-4-8
LLM_BASE_URL=https://ai.kot.gg/v1                          # 第三方相容閘道
LLM_API_KEY=...
```

> 想用 OpenRouter 官方 Fusion 也行：把 OpenRouter 設為 LoadBalanced 的一個 member，請求帶 `plugins:[{id:"fusion"}]`，第三方 provider 透過 OpenRouter BYOK 接入（前 1M 請求/月免費，之後 5%）。但原生 FusionProvider 不綁任何家，較貼合你的需求。

---

## 需求二：embedding 與「正規化」的正確位置

直說結論：**embedding 不適合做字串正規化**（去 mention、解析前綴那種），那是純字串操作，留在 `g10kz-core` 的 `normalize`（純函式、0 成本、可測）即可，別讓它打網路。

embedding 真正該出力的地方是**語意層**，而本代這部分已經由 EverOS 內建（它自帶 embedding + LanceDB 向量檢索）：
- **語意召回**：`context::gather` 的第②路就是 EverOS 向量搜尋，換句話問也召回得到。
- **語意去重（選用、收益最大）**：mini 路由器前，可用 EverOS 搜尋「最近是否有近乎相同的問題」，命中就直接複用上一條答案，省一次 LLM。

```
（選用）dedup 快取：
  query → EverOS /memory/search（top1, 同 user, 近 N 分鐘）
        → 相似度 > 0.95 且時間近 → 複用上次回覆，跳過 LLM
        → 否則正常進 router
```

但你已有便宜的 mini 路由器一次完成分類，額外加「embedding 正規化層」收益邊際。**建議：字串正規化保持純函式；語意去重作為選用快取掛在 EverOS 上，不另立 embedding crate。**

---

## 需求三：全管線多模態

v4 是純文字。本代讓圖片/音檔/PDF/文檔貫穿整條管線。

涉及的 crate 改動：

**`g10kz-discord`** — 從 `message.attachments` 抽出附件，分類（image/audio/document），取 URL 或下載 bytes，包進 turn 輸入。

**`g10kz-llm`** — 訊息內容從 `String` 擴展為 content parts 陣列（OpenAI-相容多模態格式）：

```rust
// crates/g10kz-llm/src/message.rs
pub enum Part {
    Text  { text: String },
    Image { url: String },              // data: URL 或遠端
    Audio { data: String, format: String },
}
pub struct Message {
    pub role:  Role,
    pub parts: Vec<Part>,               // 文字訊息 = 單一 Text part
}
```

**`g10kz-core` / `g10kz-pipeline`** — owner 防護仍在最前；router 偵測到有非文字 part → 強制 `escalate` 到 vision 模型（不讓純文字 mini 模型瞎猜）。

**角色 → 模型**（多模態調整）：

| Role | 用途 | 模型要求 |
|---|---|---|
| `Reasoning` | 深度推理 + 多模態 + 工具迴圈 | **vision-capable**（如 claude / gemini vision） |
| `Background` | 路由器 / 背景學習 | 文字 mini 即可 |
| `Emotion` / `Render` | 回覆渲染 | 文字即可 |

**EverOS 多模態攝取** — 附件隨對話推送到 EverOS `/memory/add`（需 `everos[multimodal]` extra），它把圖/PDF/音檔統一抽取成可搜尋記憶。office 文檔需主機裝 LibreOffice，否則該類型回 415（其餘不受影響）。

**降級安全網** — 後端不支援 vision 時：先用一個多模態模型把附件轉成文字 caption，再餵回既有純文字管線，主流程不變。

---

## EverOS 整合細節

EverOS 是 **Python sidecar**，與 bot 一起跑，HTTP 通訊。Bot 本體維持純 Rust（你的語言偏好），EverOS 只是外掛記憶服務，記憶層升級不動 Rust 碼。

```rust
// crates/g10kz-everos/src/lib.rs
pub struct EverOsClient {
    http:     reqwest::Client,
    base_url: String,            // http://everos:8000
    user_id:  String,
    agent_id: String,            // "g10kz"
}

impl EverOsClient {
    /// 語意召回：user episodes + agent skills
    pub async fn search(&self, query: &str, limit: u32) -> Result<Vec<Recall>> {
        // POST /api/v1/memory/search  { user_id, agent_id, query, limit }
    }
    /// 推送一輪對話（含多模態附件）進記憶
    pub async fn add_turn(&self, msgs: &[Message]) -> Result<()> {
        // POST /api/v1/memory/add  { messages, user_id, agent_id }
    }
}
```

EverOS 取代了 v4 的 L1/L2/L3 手寫壓縮——它內建自演化（從真實使用萃取 episodes/profile 與 cases/skills），且記憶以 `.md` 落盤，可直接用 Obsidian 開、Git 版控。

---

## 部署：docker-compose

```yaml
services:
  everos:
    image: python:3.12-slim
    command: sh -c "pip install 'everos[multimodal]' && everos server start"
    environment:
      - EVEROS_LLM__BASE_URL=https://ai.kot.gg/v1     # 與 bot 共用第三方閘道
      - EVEROS_LLM__API_KEY=${LLM_API_KEY}
    volumes:
      - everos_data:/root/.everos                      # Markdown 記憶持久化
    ports: ["8000:8000"]
    # office 文檔支援需在映像內裝 libreoffice

  bot:
    build: .
    command: g10kz-bot daemon
    environment:
      - DISCORD_TOKEN=${DISCORD_TOKEN}
      - LLM_BASE_URL=https://ai.kot.gg/v1
      - LLM_API_KEY=${LLM_API_KEY}
      - FUSION_ENABLED=true
      - FUSION_PANEL=claude-opus-4-8,gpt-4o,gemini-2.0-flash
      - FUSION_JUDGE=claude-opus-4-8
      - EVEROS_URL=http://everos:8000
      - OWNER_USER_ID=${OWNER_USER_ID}
      - CF_TOKEN=${CF_TOKEN}                            # 選用：CF 網搜工具
    depends_on: [everos]

volumes:
  everos_data:
```

`LLM_PROVIDER=mock` 仍可全離線試跑（mock provider + 不連 EverOS 時降級 SQLite）。

---

## 功能全清單（對照 v3 `bot.py`，全覆蓋 + 超越）

v3 `bot.py`（目白阿爾丹）的全部功能盤點，每項都對應到 v5 落點。**v5 全覆蓋，且每項都有強化。**

| v3 `bot.py` 功能 | v5 落點 | 強化 |
|---|---|---|
| 角色人設 system prompt（單一硬編碼） | `g10kz-core` 讀 SillyTavern 卡 | 卡片化，可**熱切換多角色**（阿爾丹/小十/任意） |
| 嚴格輸出格式（對話＋`( )`動作分行） | `g10kz-core` finalize | 格式校驗為**純函式可單測** |
| 禁止重複/禁止說教/留白等行為規則 | 卡片 + finalize 規則 | 同左，規則資料化 |
| per-user JSON 記憶 + 摘要壓縮（20/30 閾值） | **EverOS** | 自演化 episodes/profile，Markdown 落盤，**免手寫閾值** |
| 記憶摘要回顧（阿爾丹視角） | EverOS profile + `g10kz-web` 後台 | 可視化、可編輯（Obsidian） |
| 圖片記憶描述（vision 生 50 字摘要） | 多模態管線 + EverOS 攝取 | EverOS **自動**多模態抽取入記憶 |
| 兩層提示注入防禦（關鍵字快篩 + 小模型語意） | `g10kz-core` guard（純函式關鍵字）+ mini 語意 | 關鍵字層 **0 LLM 成本**、在所有呼叫最前 |
| 輸出洩漏後檢（`_sanitize_response`） | `g10kz-core` 輸出 sanitize | 保留並純函式化 |
| 阿爾丹風格拒絕回應池 | `g10kz-core` refuse 渲染 | 同左 |
| 反重複提示（注入近 4 則回覆） | pipeline 注入 hint + EverOS 語意去重 | 加**語意層**去重，不只字面 |
| Bing 搜尋（判斷→萃取關鍵字→查詢→失敗中止） | `g10kz-mcp` 網搜工具（CF AI Search／Gemini grounded） | **工具迴圈**取代單次；失敗安全閥保留（不亂答） |
| `is_search_needed`（關鍵字 + 語意判斷） | mini 路由器的 escalate 判斷 | 併入單一 router call，省一次呼叫 |
| RAG 知識庫（chunk+embed+cosine+快取） | CF AI Search（托管）或 EverOS 向量 | **免手寫 cosine/分塊**，托管 RAG |
| `!reload_rag` 重建知識庫 | slash command + CF 重新索引 | 同左 |
| 群組對話滾動窗口（每頻道 30 則，注入 15 則） | `g10kz-db` 短期快取 + pipeline 注入 | 多人對話上下文，純函式組裝 |
| 主動發言（>24h 未互動，隨機時間） | pipeline proactive 決策（純函式）+ 排程 | **決策可決定性測試** |
| 生活事件池（訓練/日常兩池隨機） | 可設定事件池（config） | 資料化、可擴充 |
| 影片處理（ffmpeg 抽幀 4 張 + vision 分析） | `g10kz-tools` ffmpeg 橋 + 多模態 | 抽幀工具化，vision 走 Fusion |
| 圖片處理（下載/編碼/vision） | 多模態 Part::Image | 同左，整合進統一管線 |
| 黑名單（降級小模型、跳過 RAG/搜尋/圖片） | `g10kz-core` 策略（純函式）+ `g10kz-db` | 策略可測 |
| 長訊息 >1900 字分段（換行優先切） | `g10kz-discord` formatter | 同左 |
| `!reset` / `!stop` / `!search` / `!memory_status` | `g10kz-discord` slash commands | slash 化、權限化 |
| 停止/取消機制（stop_flags + task.cancel + 檢查點） | pipeline `CancellationToken` | tokio 原生取消，更乾淨 |
| 觸發條件（DM / @mention / reply） | `g10kz-discord` 應答閘 | 同左 |
| 最後互動時間追蹤 | `g10kz-db` | 同左 |
| typing indicator / 空回覆保護 / 錯誤處理 | `g10kz-discord` + pipeline | 同左 |
| 多模型分層（CHAT/EMBED/SMALL/BLACKLIST） | `g10kz-llm` 角色→模型 | 同左 + **Fusion 多模型合成** |

### v5 額外超越項（v3 `bot.py` 沒有的）

- **多模型 Fusion** — 並行多模型 + judge 合成，回覆品質高於單模型。
- **owner 防護 0 成本** — v3 注入偵測要打小模型；v5 owner/guard 是純函式，0 LLM、最先擋。
- **跨平台/跨 agent 記憶** — EverOS 記憶可被其他 agent（Claude Code 等）共用，不綁單一 bot。
- **音訊多模態** — v3 只有圖/影片；v5 加 audio part（語音訊息）。
- **可離線 mock 測試 + CI** — 全領域邏輯純無 I/O，零網路跑全測試（v3 無測試）。
- **decorator 供應層** — Budget（每日 token 預算）/ Retry（退避）/ LoadBalance（池內 failover），v3 無。
- **in-process 工具迴圈** — 台股即時報價、即時時間等，模型可多輪呼叫工具（v3 只有單次 Bing）。
- **axum 管理後台** — 檢視對話與記憶（v3 無）。
- **語意去重快取** — 近乎重複的問題直接複用答案，省 LLM。
- **SLSA3 供應鏈簽章 / Docker / 多 binary 模式**（daemon/once/proactive）。

> 注意：v3 人設是「目白阿爾丹」，v4 是「小十」。v5 用 SillyTavern 卡片承載人設，**同一引擎可掛任意角色甚至多角色並存**——這本身就是超越 v3 的功能。

---

## 重構落地順序

1. **骨架** — 重建 workspace，移除 memory/vector，建 `g10kz-everos`；保留 config/core/llm/db/discord/pipeline 骨幹，先全 mock 跑通文字快路徑。
2. **核心防禦 + 人設** — SillyTavern 卡載入、owner/注入防禦（關鍵字純函式 + mini 語意）、輸出 sanitize、拒絕池、嚴格輸出格式 finalize。全純函式單測。
3. **EverOS 記憶** — 接 sidecar，gather 改 EverOS，persist 推送 EverOS；驗證降級為 SQLite。
4. **FusionProvider** — 加進 decorator 鏈，escalate 路徑啟用，mock 下寫決定性測試。
5. **多模態** — Message content parts、附件抽取、圖片 vision、**影片 ffmpeg 抽幀**、音訊、EverOS 多模態攝取、caption 降級。
6. **群組 + 主動 + 黑名單** — 群組滾動窗口注入、主動發話（純函式決策 + 生活事件池 + 排程）、黑名單降級策略。
7. **工具 + 網搜** — `g10kz-mcp` 工具迴圈（時間/台股），CF AI Search／Gemini grounded 網搜（判斷→查詢→失敗安全閥）。
8. **指令 + 後台** — slash commands（reset/stop/search/memory）、取消機制、`g10kz-web` 後台、長訊息分段。

---

## 設計原則（延續 v4 + 本代新增）

- **Bot 全 Rust，外部能力走 HTTP sidecar/API** — EverOS（Python）、CF 都不侵入 Rust 碼，可獨立升級替換。
- **decorator 可組合、可單測** — FusionProvider 是又一層 decorator，疊在既有 Budget/Retry/LoadBalance 上，各層獨立測試。
- **降級永不崩** — EverOS 掛 → SQLite 關鍵字；Fusion 全失敗 → 單模型；vision 後端缺 → caption 轉文字。
- **成本優先** — 社交 1 次 mini call；Fusion / vision / 工具只在 escalate 啟用。
- **owner 防護恆在最前** — 純函式 guard 在任何 LLM 與 sidecar 呼叫之前先擋。
- **可離線測試** — mock provider + 跳過 EverOS，CI 零網路全綠。
