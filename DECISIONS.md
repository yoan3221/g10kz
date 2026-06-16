# g10kz v5 架構決策書

> 狀態：**已定稿，待開工**（在使用者明確說「開始寫」之前不動程式碼）
> 取代：本文件取代 `ARCHITECTURE.md`、`IMPLEMENTATION_PLAN.md`（兩者在宿主拓樸決策後已過時）
> 專案：傲嬌人設 Discord 機器人「小十」(g10kz)，第五代完整重構
> 前身：v3 = Python discord.py + LangGraph（`bot.py`，1763 行）；v4 = 全 Rust 12-crate，砍掉 LangGraph

---

## 0. 一句話定位

> **Rust 宿主進程編排這個 I/O 密集的 agent；借用 LangGraph 的圖設計思想用 Rust 重寫；一個純函式驗證核心守護所有安全攸關的決策；EverOS 以 HTTP sidecar 提供長期記憶。功能全面覆蓋並超越 v3。**

核心認知：這是**I/O 密集、低頻率、整合密集**的應用。每回合的時間全花在網路 I/O（LLM、EverOS、搜尋、Discord），CPU 工作趨近於零。因此 Rust 進來**不是為了快**（這個 workload 用不到），而是為了：型別安全的驗證核心、單一語言/單一 binary 的掌控度、以及使用者的明確偏好。

---

## 1. 核心決策（ADR）

### D1 — 宿主拓樸：(C) Rust 宿主，LangGraph 當藍本
**決策**：serenity（Rust）為唯一宿主進程；用 Rust 寫一個 LangGraph 風格的回合狀態機；kernel / Fusion / 媒體全原生 Rust；EverOS 用它自帶的 HTTP server 當 sidecar。

**理由**：最純 Rust、單一 binary、型別安全、可離線測試、單檔部署。修好 v4 唯一弱點（v4 也得幫 EverOS 開 sidecar，但其餘全自己重造輪子）。

**拒絕的方案**：
- (B) Rust 宿主 + PyO3 內嵌真 LangGraph：造成「Rust→Python→Rust 三明治」，Fusion 的 tokio↔asyncio 橋接是 GIL 死鎖溫床，個人維護的維護陷阱。
- (A) Python 宿主 + Rust wheel：閘道非 serenity，Rust 退為配角，與偏好不符。

**代價（已接受）**：不採用 PyO3/Maturin、不跑真 LangGraph、**沒有 LangGraph Studio**（用 tracing 補償，見 D8）。

### D2 — Rust 範圍：全應用 Rust，明確切出純函式 kernel
**決策**：整個應用都是 Rust（D1 推論）。在 crate 邊界上把「純函式、安全攸關、確定性」的邏輯隔離成 `g10kz-kernel`，作為窮舉測試的驗證核心。

**理由**：kernel 是「絕對不能有 bug」的程式碼（注入防禦、owner 防護、洩漏檢查），Rust 型別系統 + 完整測試給正確性保證；且純函式最易測、最穩定。

### D3 — 記憶：EverOS sidecar（長期）+ engine checkpointer（短期）
**決策**：長期語意記憶 = EverOS HTTP sidecar（`everos server start`）；短期對話狀態 = engine 回合後落地的 checkpointer。**移除 v4 規劃的 `g10kz-store` 整層**。

**理由**：EverOS 自帶 server，sidecar 比 PyO3 內嵌省事（避開 asyncio↔tokio）；localhost 一跳延遲可忽略。短期狀態由 engine 持有即可，不需獨立 store 層。

**拒絕**：PyO3 內嵌 EverOS（asyncio 內嵌痛）、自寫向量庫（重造 EverOS）。

### D4 — Fusion：原生 Rust，provider 無關
**決策**：在 `g10kz-llm` 原生實作 Fusion——並行 fan-out 到 N 個 OpenAI-相容後端 + judge 合成。第三方 API 透過 OpenAI-相容端點直接接入。

**理由**：使用者要求「Fusion 要能用第三方 API」。原生實作不綁 OpenRouter，任何相容端點都能用，且可離線 mock。

**強化**：timeout + quorum（不等最慢模型）、共識短路（drafts 雷同跳過 judge）、judge 匿名合成、panel 選不同家族模型。

**拒絕**：OpenRouter 官方 Fusion plugin（綁定 OpenRouter endpoint）。

### D5 — 路由：純函式條件邊 + 便宜路徑自我升級（移除 judge::route）
**決策**：**刪除每回合的 LLM mini 路由器**。明確情況用 kernel 純函式述詞決定（注入→refuse、媒體→media、搜尋關鍵字→search、指令→command）；模糊情況直接走 social 便宜模型，給它一個 `escalate(reason)` function-call 逃生口，需要才 re-route。

**理由**：預先分類是浪費；多數聊天 0 LLM 路由 + 1 次回覆呼叫搞定。

### D6 — 串流：social 串流 / reason 緩衝
**決策**：social 路徑逐句串流（~750ms 節流、token bucket 守 Discord 5/5s 限流）；reason 路徑緩衝（先 sanitize 完整答案再送）。

**理由**：串流與 sanitize 衝突（要完整文字才能檢洩漏）；reason 風險高且 Fusion 本就一次出完整文字，緩衝合理；social 風險低，串流改善體感。

### D7 — 搜尋失敗：預設 hedge，事實題硬拒
**決策**：搜尋失敗時預設帶 hedge 回答（「我查不到即時資料，不過就我所知……」，人設承接）；只有明確數值/事實題（價格、比分、日期）才硬拒答以免誤導。

**理由**：v3 一律硬拒，UX 差；分級處理兼顧 UX 與不亂答。

### D8 — 可觀測性：structured tracing 取代 Studio
**決策**：每回合一個 tracing span，記錄路徑、各 stage 耗時、token、快取命中、降級事件，輸出每回合 JSON log；加 `/trace` slash 指令重播上回合決策軌跡。

**理由**：選 (C) 失去 Studio 視覺 debug，必須有意識補上，否則 bot 行為變黑盒。

### D9 — embedding 與正規化
**決策**：字串正規化（去 mention/前綴）用純函式，**不用 embedding**；embedding 的語意能力交給 EverOS（語意召回）；語意去重快取為選用優化。

**理由**：embedding 不適合字串清洗（純字串操作即可）；語意層已在 EverOS。

---

## 2. 架構總覽

```
Discord
   ↓ serenity（Rust，唯一宿主進程）
┌──────────────────────────────────────────────────────────────┐
│ g10kz-engine：回合狀態機（型別化 Stage enum + 條件邊）          │
│   ingest → guard → gather → [route] → {refuse/media/search/    │
│              reason/social} → render+sanitize → persist → respond │
└───┬───────────────┬──────────────┬───────────────┬────────────┘
    │               │              │               │
 g10kz-kernel   g10kz-llm     g10kz-tools     g10kz-everos
 純驗證核心     供應層+Fusion  工具+媒體       記憶 sidecar client
                    │                              │
              OpenAI 相容後端                 EverOS HTTP server
              (任意第三方)                    (Python sidecar)
```

### Crate 分層（單一 Rust workspace，依賴由下往上）

| 層 | Crate | 職責 |
|---|---|---|
| L0 | `g10kz-config` | 型別化設定（env）：模型、Fusion、EverOS、ffmpeg 路徑、事件池、預算 |
| L1 | `g10kz-kernel` | **純函式驗證核心**：persona 卡、注入防禦、owner、sanitize、格式、反重複、黑名單策略、主動決策、route 述詞、成本判斷 |
| L1 | `g10kz-llm` | Provider trait、多模態 Message/Part、mock、OpenAI 相容 client（可取消）、per-path 參數、prefix 快取、Fusion |
| L2 | `g10kz-everos` | EverOS HTTP client、斷路器、寫入合併、search 快取、降級 |
| L2 | `g10kz-tools` | Tool trait + registry（時間/台股/網搜）、工具迴圈、媒體（ffmpeg/downscale/轉錄） |
| L3 | `g10kz-engine` | 回合狀態機、條件邊、自我升級、取消、串流策略、tracing span、checkpointer |
| L4 | `g10kz-discord` | serenity 閘道、應答閘、群組 ring buffer、串流編輯、長訊息分段、slash 指令、persona 熱重載 |
| L5 | `g10kz-bot` | binary：daemon/once/proactive；tracing 初始化；sidecar 健康檢查；graceful shutdown |

> 對比 v4 的 12 crate：移除 `g10kz-store`、`g10kz-vector`、`g10kz-memory`、`g10kz-web`、`g10kz-mcp`（工具併入 tools）。**未使用 PyO3/Maturin。**

---

## 3. 每回合資料流 + 成本階梯

```
Discord 訊息 → 應答閘（DM/@/reply；群組訊息進 ring buffer）→ 訊息 id 去重 → run_turn:

  guard      pre_guard（kernel，0 LLM）：注入關鍵字快擋 + owner 判定
  normalize  純函式：去 mention/前綴（NFKC/homoglyph 正規化）
  gather     本地近期歷史(免費) ∥ EverOS 語意搜尋(僅 reason 或回指時，800ms 超時降級) ∥ 群組窗口
  route      純函式條件邊：注入→refuse｜媒體→media｜搜尋詞→search｜指令→command｜else→social
   ├ refuse   kernel 拒絕池（0 LLM）
   ├ media    ffmpeg 抽幀(自適應) → 直接進 reason 的 vision 呼叫；記憶 caption 另開 async
   ├ search   原始 query 優先 → CF/Gemini → 失敗 hedge/事實題硬拒
   ├ reason   單模型跑工具迴圈(蒐集事實) → Fusion 只在最後合成(timeout/quorum/共識短路)
   └ social   單一便宜模型 1 次呼叫 + escalate() 逃生口（多數聊天止於此）
  render     sanitize 洩漏後檢(命中→重生1次→仍洩漏→拒答) + 格式機械正規化 + 空回覆保護
  respond    social 串流 / reason 緩衝；長訊息分段
  persist    先回覆，再背景 spawn：checkpointer + EverOS add_turn(批次) + 記憶 caption
```

**成本階梯**（優化總目標：把流量壓在便宜端）：

```
social   ≈ 1 次呼叫   ← 多數流量
search   ≈ 2 次
reason   ≈ 2–3 次（單模型 + 工具）
reason+Fusion ≈ N+1 次（僅難題/重要題開啟，且 Fusion 只合成一次）
```

---

## 4. 優化清單（定稿，依主題）

**LLM 成本**
- ⭐ Persona system prompt 標記為可快取**前綴**（人設永不變，命中後幾乎免費）——最大單筆省。
- Per-path `max_tokens`/`temperature`（social 短高溫、reason 長低溫、judge 低溫），取代 v3 一律 2048。
- Recall 按 token 預算截斷，非固定條數。
- 成本計量器 + 超預算降級（kernel 純函式）。

**Fusion**
- timeout + quorum（M-of-N 到齊或 T 毫秒即啟動 judge，丟落後者）。
- 共識短路（drafts 雷同→跳過 judge）。
- judge 匿名合成；panel 選不同家族模型。
- 單模型跑工具迴圈，Fusion 只用於最終合成（綁死一次）。

**取消 / 並行**
- ⭐ 取消能中斷在途 LLM 呼叫（`tokio::select!` 綁 reqwest future），新訊息殺舊回合不白燒 token。
- 逐用戶串行（新訊息取消舊）+ 全域 semaphore 背壓。

**記憶 / sidecar**
- 斷路器（EverOS/搜尋/各 backend 連續失敗→暫停呼叫 T 秒）。
- EverOS 寫入合併/批次 flush（降低它端抽取成本）。
- search 結果短 TTL 快取；工具結果快取（台股 60s）。
- EverOS 僅在 reason 路徑或訊息含回指（「還記得/之前/人名」）時才打。

**引擎**
- 型別化 Stage enum + match，非泛型 graph runtime（編譯期檢查、零 runtime 邊查找）。
- 跳過回合中途 checkpointing，只在回合完成後落短期狀態。

**多模態**
- 圖片送 vision 前 downscale ≤1024px；影片幀數隨時長自適應；靜態幀去重；音訊轉錄當文字。
- 媒體走 `tokio::process` 非阻塞；暫存檔 Drop guard 自動清。

**品質 / 人設**
- 格式機械正規化在 kernel 免費做（空白/換行/`( )`）；只有缺動作才軟處理，不為小問題重 roll。
- 反重複：注入 hint + kernel 偵測重複開頭片語。
- persona 卡熱重載（監看檔案，不重啟調語氣）。

**安全**
- 注入比對前狠正規化：NFKC、去組合字元/零寬、收合重複、全形→半形、homoglyph 折疊。
- owner 只認 user_id（名稱可仿冒，不採用）。
- 投資重心在 sanitize 後檢（有界、可靠）而非事前注入偵測（無界、打不贏）——**砍掉 v3 的逐則語意偵測呼叫**。

**可觀測性**
- 每回合 tracing span（路徑/耗時/token/快取/降級）+ `/trace` 重播。

**測試 / 部署**
- 注入 clock 與 RNG → proactive/拒絕池可決定性測試。
- 全 kernel 純函式 → proptest。
- mock provider 腳本化 → 引擎各分支離線測試。
- 訊息 id 冪等去重；graceful shutdown flush。

---

## 5. 對 v3 / v4 的取捨

| 處置 | 項目 |
|---|---|
| **移除** | judge::route LLM 呼叫、v3 逐則語意注入偵測、`g10kz-store`、`g10kz-web` 後台、Rust decorator 供應層的過度設計、多 binary 複雜度、PyO3 三明治 |
| **精簡** | 短期狀態歸 engine checkpointer、工具併入單一 tools crate、搜尋關鍵字萃取（原始 query 優先） |
| **新增（v3/v4 沒有）** | Fusion 多模型合成、prefix 快取、取消中斷在途呼叫、斷路器、串流回覆、tracing 可觀測性、語意去重快取、音訊多模態、persona 熱重載、成本計量、homoglyph 正規化、可決定性測試 |

---

## 6. v3 功能全覆蓋（確認）

v3 `bot.py` 每項功能在 v5 都有對應且強化：人設/嚴格格式、per-user 記憶+摘要、圖片記憶描述、兩層注入防禦（改為關鍵字+sanitize）、反重複、Bing 搜尋（→工具迴圈+網搜）、RAG 知識庫（→EverOS/CF）、群組上下文、主動發話+生活事件、影片 ffmpeg 抽幀、黑名單降級、長訊息分段、`!reset/!stop/!search/!memory`（→slash）、停止/取消機制、DM/@/reply 觸發、多模型分層。**詳細對照見先前盤點，全數收進上面 crate 與資料流。**

---

## 7. 設定與部署

- **單一 Rust binary** + EverOS sidecar，docker-compose 一起跑。
- 設定全走 env：`DISCORD_TOKEN`、`OWNER_USER_ID`、`LLM_BASE_URL`/`LLM_API_KEY`（任意第三方相容端點）、`MODEL_*`（角色→模型）、`FUSION_*`（enabled/panel/judge）、`EVEROS_URL`、`FFMPEG_PATH`/`FFPROBE_PATH`（修 v3 寫死路徑）、`PROACTIVE_*`、`DAILY_TOKEN_BUDGET`、`SEARCH_*`。
- `LLM_PROVIDER=mock` 全離線跑通（mock provider + NullMemory），CI 零網路。
- 三個入口：`g10kz-bot daemon`（常駐連 Discord）/ `once "訊息"`（離線煙霧測試）/ `proactive`（跑一次主動排程）。

---

## 8. 風險與未決

- **EverOS sidecar 依賴**：整合測試標 `#[ignore]`；斷路器 + 降級確保它掛了 bot 不崩。
- **serenity 編譯重**：可接受；非熱點，且單一宿主必需。
- **無 Studio**：以 tracing + `/trace` 補償（D8）。
- **Fusion 成本**：分級開啟 + 共識短路控制；預設只在難題開。
- **prefix 快取**依後端支援：不支援時退化為一般送出（仍正確，只是較貴）。

---

## 9. 開工順序（待「開始」指令）

M1 `config`+`kernel`（純函式，最高價值，proptest 全綠）→ M2 `llm`（Fusion/mock/per-path/prefix 快取）→ M3 `everos`（斷路器/降級）→ M4 `tools`（工具迴圈/媒體）→ M5 `engine`（狀態機/取消/串流/tracing）→ M6 `bot once`（離線跑通）→ M7 `discord`（serenity/串流/指令）→ M8 全工作區 `cargo test`/`clippy -D warnings`/`fmt --check` + docker-compose 接 EverOS。

每個 crate 先寫測試骨架再填實作，逐 crate `cargo test -p` 驗證後再進下一個。

---

> **本決策書已定稿。在你說「開始寫」之前，不會動任何程式碼。**
