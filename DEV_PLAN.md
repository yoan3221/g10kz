# g10kz v5 開發計畫書

> 配合 [DECISIONS.md](DECISIONS.md)。本文把架構拆成可逐步執行、每步有「完成定義(DoD)」的開發步驟。
> 狀態：**待「開始寫」指令**。

---

## 方法論

1. **走路骨架優先**：先讓最小路徑（config → kernel guard → mock llm → social → respond）離線端到端跑通，**驗證架構脊椎**，再逐 crate 長肉。比純由下而上更早暴露整合問題。
2. **每 crate TDD**：先寫測試骨架（mock/proptest）再填實作，`cargo test -p <crate>` 綠燈才進下一步。
3. **離線可測**：全程 `LLM_PROVIDER=mock` + `NullMemory` 可跑，外部依賴用 trait 隔離，整合測試標 `#[ignore]`，CI 零網路。
4. **每步測試 gate**：fmt + clippy(-D warnings) + test，沒綠不前進。

---

## 依賴與順序

```
P0 骨架
   ↓
P1 走路骨架（縱切最小路徑）── 驗證脊椎
   ↓
P2 kernel ──┐
P3 llm ─────┤  (P2/P3 可部分平行：型別先定，實作各自展開)
P4 everos ──┤
P5 tools ───┘
   ↓
P6 engine（串接 P2–P5）
   ↓
P7 discord + daemon
   ↓
P8 整合 + 硬化
```

---

## P0 — workspace 骨架

**目標**：可編譯的空殼，工具鏈與 CI 就緒。
**產出**：
- workspace `Cargo.toml`（8 crate 成員）、`rust-toolchain.toml`(stable 1.96)、`.gitignore`(target/、.env)
- 8 個 crate 骨架：`config / kernel / llm / everos / tools / engine / discord / bot`，各含 `lib.rs`(或 `main.rs`) stub
- 共用依賴版本對齊（workspace deps）、`tracing` 初始化骨架
- CI：fmt / clippy(-D warnings) / test 三道

**DoD**：`cargo build --workspace` 通過、`cargo clippy --workspace -- -D warnings` 綠、`cargo fmt --check` 綠。

---

## P1 — 走路骨架（離線端到端最小路徑）

**目標**：`once "你好"` 能離線印出一句 mock 社交回覆。**證明脊椎成立。**
**產出**（每個 crate 只做最小可用）：
- `config`：`Config` + `from_env` + `mock_default()`
- `kernel`：`TurnContext` 型別、`pre_guard`(關鍵字 stub)、`normalize`、`route` 述詞(最小)、persona 卡載入、reject 池；**注入 clock/RNG**
- `llm`：`Provider` trait、`Message`/`Part`、`MockProvider`(腳本化回應)
- `engine`：`Stage` enum + `run_turn` 最小流程(guard→normalize→route→social→render)
- `bot`：`once` 入口 + tracing init

**DoD**：`LLM_PROVIDER=mock cargo run -p g10kz-bot -- once "你好"` 印出 mock 社交回覆；該縱切路徑單元測試綠。

---

## P2 — kernel 完整化（純驗證核心，最高價值）

**目標**：所有安全/策略純函式齊備且窮舉測試。
**產出**：
- 注入防禦：完整關鍵字表 + 狠正規化(NFKC/去組合字元/去零寬/收合重複/全形→半形/homoglyph 折疊) + `keyword_injection_hit`
- owner guard（**僅 user_id**）+ `pre_guard` verdict 完整
- `sanitize` 洩漏後檢 + leak signals 表
- 格式機械正規化 + `validate` + 空回覆保護
- 反重複 hint + 重複開頭片語偵測
- 黑名單策略（純函式矩陣）
- 主動發話決策（注入 clock/RNG，可決定性）+ 生活事件挑選
- `route` 述詞完整(complexity / search-trigger / command signals) + 成本/預算判斷

**DoD**：`cargo test -p g10kz-kernel` ≥50 例全綠，含 proptest（正規化/格式）。

---

## P3 — llm 完整化 + Fusion

**目標**：統一供應層、可取消、Fusion 齊備。
**產出**：
- OpenAI 相容 client（reqwest，`tokio::select!` 可取消）
- per-path `max_tokens`/`temperature`；persona system prompt **prefix 快取標記**
- 簡單 retry + fallback + 斷路器掛勾
- Fusion：並行 fan-out + **timeout/quorum** + **共識短路**(drafts 雷同跳過 judge) + judge 匿名合成
- 成本計量整合

**DoD**：`cargo test -p g10kz-llm` 綠——Fusion 部分失敗降級、共識短路、取消中斷在途、多模態 message 序列化成 OpenAI content array、per-path 參數。

---

## P4 — everos 記憶 sidecar client

**目標**：長期記憶接入，掛了不崩。
**產出**：
- `Memory` trait + `NullMemory` + `EverosMemory`(search / add)
- 斷路器（連續失敗暫停 T 秒）+ 800ms 超時 + 失敗降級回空
- 寫入合併/批次 flush + search 結果短 TTL 快取

**DoD**：`cargo test -p g10kz-everos` 綠——降級路徑(連線失敗回空不 panic)、payload 組裝、斷路器狀態轉移；真 EverOS 整合測試標 `#[ignore]`。

---

## P5 — tools 工具迴圈 + 媒體

**目標**：可被模型多輪呼叫的工具 + 多模態前處理。
**產出**：
- `Tool` trait + `ToolBox` registry + function-calling schema
- 時間工具、台股(60s 快取)、網搜(原始 query 優先；失敗 hedge / 事實題硬拒)
- 工具迴圈（max 迭代上限）
- 媒體：`tokio::process` ffmpeg 抽幀(幀數隨時長自適應、靜態幀去重) + 圖片 downscale≤1024 + 音訊轉錄 + 暫存檔 Drop guard

**DoD**：`cargo test -p g10kz-tools` 綠——ToolBox 分派、搜尋安全閥、時間/台股工具、迭代上限；ffmpeg 測試標 `#[ignore]`。

---

## P6 — engine 完整化（串接全部）

**目標**：完整每回合狀態機。
**產出**：
- `gather` 三路並行 + EverOS 閘門(僅 reason 或訊息含回指才打)
- `route` 完整分支 + 自我升級逃生口(`escalate()` function-call)
- media / search / reason / social 各節點接真 crate
- reason：單模型工具迴圈 → Fusion 最終合成
- render：sanitize→重生1次→拒答 + 格式正規化
- `CancellationToken` 綁 in-flight 呼叫 + 逐用戶串行 + 全域 semaphore 背壓
- 串流策略(social 串/reason 緩) + persist 先回覆後背景 spawn + checkpointer
- per-turn tracing span

**DoD**：`cargo test -p g10kz-engine` 綠（全 mock 離線）——社交1呼叫、注入拒答、多模態強制升級、EverOS 降級不崩、洩漏轉拒答、取消中斷、黑名單跳過、自我升級 re-route。

---

## P7 — discord + bot daemon

**目標**：真連 Discord，常駐運行。
**產出**：
- serenity 閘道：應答閘(DM/@/reply) + 訊息 id 去重 + 最小 intents
- 群組 ring buffer + typing 自動續(~8s)
- 串流編輯批次(token bucket 守 5/5s 限流) + 長訊息分段(>1900 換行優先切)
- slash 指令：`/reset` `/stop` `/search` `/memory` `/persona`(切角色卡) `/trace`(重播上回合)
- persona 卡熱重載（檔案監看）
- `bot daemon`/`proactive` 入口 + sidecar 健康檢查 + 退避重連 + graceful shutdown flush

**DoD**：`cargo check -p g10kz-discord` 通過；本地連真 Discord 冒煙(需 token)——收發、串流、指令、取消可用。

---

## P8 — 整合 + 硬化

**目標**：端到端可用 + 全綠 + 對 v3 驗收。
**產出**：
- `docker-compose.yml`：bot + EverOS sidecar；`.env.example`
- 端到端：真 EverOS + 真 LLM 端點跑通(記憶召回/網搜/Fusion/多模態/主動)
- 全工作區 gate：`cargo test --workspace` + `clippy -D warnings` + `fmt --check`
- 文件：README、設定說明；(選用) v3 `memory.json` → EverOS 遷移腳本
- **功能對照 v3 逐項驗收**（見 DECISIONS §6）

**DoD**：全綠 + daemon 穩定運行 + v3 功能全項通過 + 至少一個多模型 Fusion 回合、一個多模態回合、一次主動發話實測。

---

## 平行與風險

- **可平行**：P2–P5 在型別先定後可各自展開（不同 crate、低耦合）；單人開發建議仍按 P2→P5 順序以控管心智負擔。
- **關鍵風險**：engine 串接(P6) 是整合熱點 → 走路骨架(P1) 已先驗證脊椎降低此風險。
- **外部依賴**：EverOS/Discord/LLM 端點全 trait+feature 隔離，P1–P6 全離線完成，P7/P8 才碰真服務。

---

## 進度追蹤

任務板已對齊 P0–P8。每階段 DoD 達標才標記 completed 並進下一階段。
