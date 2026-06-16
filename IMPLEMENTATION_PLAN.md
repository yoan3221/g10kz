# g10kz v5 實作計畫（可開工級）

> 配合 [ARCHITECTURE.md](ARCHITECTURE.md)。本文把每個 crate 展開到「拿著就能寫」的程度：模組、關鍵型別/trait 簽名、依賴選型、測試清單、以及對 v3 的超越點。
>
> 語言 Rust（edition 2021，stable 1.96+）。原則：**領域邏輯純無 I/O、可離線 mock、降級永不崩**。

---

## 0. 依賴選型（刻意精簡，利於離線編譯與測試）

| 用途 | crate | 備註 |
|---|---|---|
| 序列化 | `serde` / `serde_json` | 全工作區 |
| async runtime | `tokio`（features: rt-multi-thread, macros, time, sync） | llm/pipeline/tools/everos/bot |
| async trait | `async-trait` | Provider / Tool / Memory |
| 錯誤 | `thiserror`（庫）/ `anyhow`（binary） | |
| HTTP | `reqwest`（rustls-tls, json） | **feature-gated `http`**；mock 測試不需要 |
| 並行合成 | `futures`（join_all） | FusionProvider |
| 隨機 | `fastrand` | 拒絕池/主動事件，極輕量、無 syn 依賴 |
| 設定 | `serde` + 手寫 env 讀取 | 不引 figment，降依賴；可日後換 |
| Discord | `serenity`（client, gateway, model） | **feature-gated `discord`**，僅 daemon 用 |
| 時間 | `time` 或 `chrono` | 用 `time`（較輕） |
| 日誌 | `tracing` + `tracing-subscriber` | binary 初始化 |

> **關鍵決策**：短期記憶/狀態用 **trait + in-memory + JSON 持久化**，不綁原生 SQLite（rusqlite/sqlx 編譯重、需系統庫）。`Store` trait 留 SQLite drop-in 空間，先以 JSON 實作（對齊 v3 的 `memory.json` 行為，但抽象化）。

---

## 1. 依賴圖（編譯順序）

```
g10kz-config        （無依賴）
   ↑
g10kz-core          （config）            ← 純領域，最先寫最先測
   ↑
g10kz-llm           （core）              ← Provider + Fusion + mock
g10kz-store         （core）              ← 短期狀態
   ↑
g10kz-everos        （core, llm types）   ← 長期記憶 HTTP
g10kz-tools         （core, llm types）   ← 工具迴圈
   ↑
g10kz-pipeline      （以上全部）          ← 每回合狀態機
   ↑
g10kz-discord       （pipeline）[feature] g10kz-bot（pipeline, discord）
```

---

## 2. `g10kz-config`（L0）

**職責**：型別化設定，從 env 載入，提供預設值與驗證。無業務依賴。

**模組**：`lib.rs`、`models.rs`（角色→模型對應）、`load.rs`（env 解析）。

**關鍵型別**：

```rust
pub struct Config {
    pub discord_token: Option<String>,
    pub owner_user_id: u64,
    pub provider: ProviderKind,           // Mock | OpenAiCompat
    pub llm_base_url: String,
    pub llm_api_key: String,
    pub models: RoleModels,               // reasoning/background/emotion/render
    pub fusion: FusionConfig,             // enabled, panel, judge
    pub everos: EverosConfig,             // url, user_id, agent_id, enabled
    pub limits: Limits,                   // daily_token_budget, max_turns, summary_threshold
    pub proactive: ProactiveConfig,       // inactive_hours, channels, event_pools
    pub ffmpeg: FfmpegConfig,             // ffmpeg_path, ffprobe_path, frames（取代 v3 寫死路徑）
    pub search: SearchConfig,             // backend(CF/Gemini/None), endpoint, token
}
pub enum ProviderKind { Mock, OpenAiCompat }
pub struct RoleModels { pub reasoning: String, pub background: String, pub emotion: String, pub render: String, pub blacklist: String }

impl Config {
    pub fn from_env() -> Result<Self, ConfigError>;     // 缺關鍵值給預設 + 警告
    pub fn mock_default() -> Self;                       // 測試用，全離線
}
```

**對 v3 超越**：ffmpeg 路徑、事件池、模型全部設定化（v3 寫死 `C:\Users\HQP\...`）。

**測試**：`from_env` 缺值降級、`mock_default` 正確、模型對應解析。

---

## 3. `g10kz-core`（L1）— 純領域（重頭戲）

**職責**：所有「不打網路就能判斷」的邏輯。這是「比 v3 更優秀」的核心，全部純函式、全部單測。

**模組與內容**：

### `persona.rs` — SillyTavern 角色卡
```rust
pub struct CharacterCard {                 // 對應 Character Card V2 data 區
    pub name: String,
    pub description: String,
    pub personality: String,
    pub scenario: String,
    pub system_prompt: Option<String>,
    pub first_mes: Option<String>,         // 每日開場白
}
impl CharacterCard {
    pub fn from_json(s: &str) -> Result<Self, CardError>;
    pub fn render_system(&self, ctx: &TurnContext) -> String;   // 組 system prompt
}
```
> 超越 v3：人設資料化、可載多張卡熱切換（v3 硬編碼阿爾丹）。

### `guard.rs` — 防禦（純函式，0 LLM）
```rust
pub fn is_owner(user_id: u64, cfg: &Config) -> bool;
pub fn keyword_injection_hit(text: &str) -> Option<&'static str>;  // 回命中的關鍵字
pub fn normalize_for_detect(text: &str) -> String;                 // 去零寬字元/拆字混淆
pub enum Verdict { Allow, BlockInjection(&'static str), OwnerProtected }
pub fn pre_guard(text: &str, user_id: u64, cfg: &Config) -> Verdict;
```
關鍵字表（EN/中/羅馬拼音/拆字）整理為 `const &[&str]`，直接移植 v3 的 `_INJECTION_KEYWORDS` 並擴充。

### `sanitize.rs` — 輸出洩漏後檢
```rust
pub fn sanitize_output(answer: &str) -> Option<String>;  // None = 偵測洩漏，觸發拒絕
const LEAK_SIGNALS: &[&str];                              // 移植 v3 leak_signals
```

### `reject.rs` — 拒絕回應池
```rust
pub fn reject_response(rng: &mut impl FnMut() -> usize) -> &'static str;  // 可注入 rng 以測試
```
> 決定性測試：把隨機源抽象成參數（v3 直接 `random.choice` 不可測）。

### `repetition.rs` — 反重複
```rust
pub fn anti_repetition_hint(recent_assistant: &[String], n: usize) -> Option<String>;
```

### `blacklist.rs` — 黑名單策略（純函式）
```rust
pub struct Policy { pub model_role: Role, pub skip_rag: bool, pub skip_search: bool, pub skip_media: bool }
pub fn policy_for(user_id: u64, blacklist: &HashSet<u64>) -> Policy;
```

### `proactive.rs` — 主動發話決策（純函式）
```rust
pub struct ProactiveDecision { pub should_send: bool, pub delay_secs: Option<u64> }
pub fn decide_proactive(now: OffsetDateTime, last: Option<OffsetDateTime>,
                        sent_today: bool, cfg: &ProactiveConfig,
                        rng: &mut impl FnMut() -> f64) -> ProactiveDecision;
pub fn pick_event(pools: &EventPools, rng: ...) -> &str;
```
> 超越 v3：決策與 I/O 分離，可決定性測試（v3 決策埋在 async loop 裡無法測）。

### `format.rs` — 嚴格輸出格式
```rust
pub fn validate_format(answer: &str) -> FormatReport;   // 對話/動作分行、( )包裹、至少一個動作
pub fn fallback_silence() -> &'static str;              // 空回覆保護
```

### `route.rs` — 路由型別 + 純判斷
```rust
pub enum Intent { Social, InfoQuery, Factual, Command }
pub enum Tone { Casual, Serious }
pub struct RouteDecision { pub intent: Intent, pub escalate: bool, pub draft: Option<String> }
pub fn should_escalate(intent: Intent, tone: Tone, has_media: bool) -> bool;  // 多模態強制升級
```

### `group.rs` — 群組上下文組裝（純函式）
```rust
pub struct GroupMsg { pub author: String, pub content: String, pub time: String }
pub fn build_group_prompt(msgs: &[GroupMsg], inject_count: usize) -> String;
```

**core 測試清單（≥40 例）**：注入關鍵字命中/漏網、拆字混淆正規化、owner 判定、洩漏後檢命中、拒絕池決定性、反重複 hint 生成、黑名單策略矩陣、主動決策（剛互動/超時/今日已發/隨機延遲邊界）、格式校驗、escalate 矩陣（含多模態）、群組注入截斷。

---

## 4. `g10kz-llm`（L1）— 供應層 + Fusion

**職責**：統一 LLM 介面、多模態訊息、decorator 疊層、原生 Fusion、mock。

**模組**：`message.rs`、`provider.rs`、`mock.rs`、`openai.rs`（feature `http`）、`decorators/{budget,retry,loadbalance}.rs`、`fusion.rs`、`role.rs`。

```rust
// message.rs — 多模態
pub enum Part { Text(String), Image { url: String }, Audio { data: String, format: String } }
pub struct Message { pub role: MsgRole, pub parts: Vec<Part> }
pub enum MsgRole { System, User, Assistant }
impl Message { pub fn text(role: MsgRole, s: impl Into<String>) -> Self; }

// provider.rs
pub struct ChatRequest { pub model: String, pub messages: Vec<Message>, pub temperature: f32, pub max_tokens: u32 }
pub struct ChatResponse { pub text: String, pub tokens: u32 }
#[async_trait] pub trait Provider: Send + Sync {
    async fn complete(&self, req: ChatRequest) -> Result<ChatResponse, LlmError>;
}

// fusion.rs
pub struct FusionProvider<P> { inner: P, panel: Vec<String>, judge: String, enabled: bool }
// complete(): 並行 join_all → 收集成功 drafts → 組 judge prompt → inner.complete(judge)
// 全失敗/單模型 → 降級 inner.complete(req)
```

**decorator 疊法**：`Budgeted<Fusion<Retry<LoadBalanced<Backend>>>>`。每層獨立測試（Retry：指數退避 + temperature fallback；LoadBalance：先換 member 再退避；Budget：超預算拒絕）。

**mock.rs**：可程式化回應（依 model 名/輸入回固定值），讓 Fusion 與 pipeline 可決定性測試。

**測試**：FusionProvider 三模型→judge 合成、部分失敗仍合成、全失敗降級、Budget 擋量、Retry 退避次數、LoadBalance failover、多模態 message 序列化成 OpenAI content array。

> 超越 v3：v3 只有單一 `AsyncOpenAI` 直呼；v5 有預算/重試/負載均衡/多模型合成且可測。

---

## 5. `g10kz-store`（L1）— 短期狀態

**職責**：近期對話、群組滾動窗口、黑名單、最後互動時間。trait + in-memory + JSON。

```rust
#[async_trait] pub trait Store: Send + Sync {
    async fn recent_turns(&self, user: u64, n: usize) -> Vec<Message>;
    async fn push_turn(&self, user: u64, msg: Message);
    async fn record_group(&self, channel: u64, m: GroupMsg);
    async fn group_window(&self, channel: u64, n: usize) -> Vec<GroupMsg>;
    async fn blacklist(&self) -> HashSet<u64>;
    async fn last_interaction(&self, user: u64) -> Option<OffsetDateTime>;
    async fn touch(&self, user: u64);
}
pub struct MemStore { /* RwLock<HashMap> */ }       // 測試用
pub struct JsonStore { /* MemStore + 落盤 */ }       // 對齊 v3 行為
```

**測試**：滾動窗口上限丟棄、recent 截斷、黑名單讀寫、JSON round-trip。

---

## 6. `g10kz-everos`（L2）— 長期記憶

**職責**：EverOS HTTP 記憶（語意召回 + 推送），**失敗則靜默降級**回 `Store` 關鍵字。

```rust
#[async_trait] pub trait Memory: Send + Sync {
    async fn search(&self, query: &str, limit: u32) -> Vec<Recall>;   // 失敗回 vec![]
    async fn add_turn(&self, msgs: &[Message]);                        // 失敗只記 log
}
pub struct EverosMemory { http: Client, base_url: String, user_id: String, agent_id: String }
pub struct NullMemory;            // everos 未啟用時用，永遠空
pub struct Recall { pub text: String, pub score: f32, pub kind: RecallKind }
```
HTTP 形狀（依 ARCHITECTURE）：`POST /api/v1/memory/search`、`/add`。**所有錯誤吞掉回空**，bot 不崩。

**測試**：降級路徑（連線失敗回空、不 panic）；payload 組裝。網路整合測試標 `#[ignore]`。

---

## 7. `g10kz-tools`（L2）— 工具迴圈

**職責**：可被模型多輪呼叫的工具；多模態影片抽幀。

```rust
#[async_trait] pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn schema(&self) -> serde_json::Value;        // function-calling schema
    async fn call(&self, args: serde_json::Value) -> Result<String, ToolError>;
}
pub struct ToolBox { tools: Vec<Box<dyn Tool>> }
// 內建：TimeTool（即時時間）、StockTool（台股 TWSE，stub→實作）、WebSearchTool（CF AI Search/Gemini）
// 影片：fn extract_frames(path, n, &FfmpegConfig) -> Vec<PathBuf>  （包 ffmpeg/ffprobe，路徑來自 config）
```

**測試**：TimeTool 純邏輯、ToolBox 註冊/分派、ffmpeg 抽幀（標 `#[ignore]` 需真檔）、search 失敗安全閥回哨兵。

> 超越 v3：v3 只有單次 Bing；v5 是工具迴圈（模型可多輪 call），且 ffmpeg 路徑設定化。

---

## 8. `g10kz-pipeline`（L3）— 每回合狀態機

**職責**：串接全部，取代 LangGraph。純編排，依賴注入 trait（Provider/Memory/Store/ToolBox）。

```rust
pub struct Pipeline<P: Provider, M: Memory, S: Store> {
    pub llm: P, pub memory: M, pub store: S, pub tools: ToolBox,
    pub card: CharacterCard, pub cfg: Config,
}
pub struct TurnInput {
    pub user_id: u64, pub channel_id: Option<u64>, pub is_dm: bool,
    pub text: String, pub attachments: Vec<Attachment>,   // image/audio/video
}
pub struct TurnOutput { pub reply: String, pub used_escalation: bool }

impl Pipeline {
    pub async fn run_turn(&self, input: TurnInput, cancel: CancellationToken) -> TurnOutput;
}
```

**run_turn 流程**（對照 ARCHITECTURE 資料流，逐步可被 cancel 檢查）：
1. `pre_guard` → 注入/owner 防護（0 LLM）；命中 → `reject_response`
2. `normalize` 純函式
3. `policy_for`（黑名單）決定模型與是否跳過 RAG/搜尋/媒體
4. `gather`：`tokio::join!`(store.recent, memory.search, store.group_window)；EverOS 失敗降級
5. 多模態：有 image/video/audio → 強制 escalate；影片先抽幀→vision caption
6. `route`（mini）：分類 + escalate +（社交）草擬
7. 分支：guard_block→refuse｜escalate→think(Fusion + 工具迴圈)→render｜draft→direct｜else→emotion
8. `sanitize_output` → 洩漏則 refuse；`validate_format`；空回覆保護
9. `persist`：store.push_turn + 非同步 memory.add_turn（含附件）

**取消機制**：`tokio_util::sync::CancellationToken`，每階段 `if cancel.is_cancelled() { return }`（取代 v3 的 stop_flags + task.cancel）。

**測試（mock 全離線）**：社交快路徑只 1 次 mini call、注入直接 refuse、多模態強制 escalate、EverOS 掛降級不崩、洩漏輸出轉 refuse、取消中斷、黑名單跳過 RAG/搜尋。

---

## 9. `g10kz-discord`（L4，feature `discord`）+ `g10kz-bot`（L5）

**g10kz-discord**：serenity `EventHandler`。
- 應答閘：DM / @mention / reply-to-bot（其餘僅 `record_group`）
- 抽附件 → `Attachment{ kind, url }`
- `send_long_message`：>1900 字換行優先切，首段 reply 餘段 send（移植 v3）
- slash commands：`/reset` `/stop` `/search` `/memory` `/persona`（切換角色卡，新功能）
- typing indicator、錯誤回覆

**g10kz-bot**：
```
g10kz-bot once "你好"          # 離線（mock provider + NullMemory），印回覆 → CI/煙霧測試
g10kz-bot daemon               # 連 Discord（需 feature discord + token）
g10kz-bot proactive            # 跑主動發話排程一次
```
`once` 模式不需 serenity，保證離線可跑、可測。

---

## 10. 建置與驗證里程碑

| 里程碑 | 驗收 |
|---|---|
| M1 config + core | `cargo test -p g10kz-core` 全綠（≥40 例） |
| M2 llm | Fusion/decorator/mock 測試綠；多模態序列化 |
| M3 store + everos | 降級測試綠；JSON round-trip |
| M4 tools | ToolBox 分派、搜尋安全閥 |
| M5 pipeline | mock 全離線 run_turn 各分支測試綠 |
| M6 bot once | `LLM_PROVIDER=mock g10kz-bot once "你好"` 印出格式正確回覆 |
| M7 discord | `cargo check -p g10kz-discord --features discord` 通過 |
| M8 全工作區 | `cargo test --workspace`、`cargo clippy -- -D warnings`、`cargo fmt --check` |

---

## 11. 風險與取捨

- **serenity 編譯重** → feature-gate，預設工作區測試不拉；`once` 模式驗證核心。
- **原生 SQLite 依賴重/需系統庫** → 先用 JSON `Store`，trait 留 SQLite drop-in。
- **EverOS 是 Python sidecar** → 整合測試標 `#[ignore]`，單元測試只測降級與 payload。
- **網路受限環境** → 所有外部呼叫 feature/trait 隔離，CI 零網路全綠。
- **影片抽幀依賴 ffmpeg** → 路徑設定化；無 ffmpeg 時該功能回明確錯誤，主流程不受影響。

---

## 12. 下一步

依里程碑 M1→M8 逐 crate 實作，每個 crate 先寫測試骨架再填實作，每完成一個 `cargo test -p` 驗證後再進下一個。先從 `g10kz-config` + `g10kz-core` 開工（純函式、最高價值、最易驗證）。
