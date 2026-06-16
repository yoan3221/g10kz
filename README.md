# g10kz

傲嬌 AI Discord 機器人，以 Rust 實作的八層 crate workspace。角色「g10kz」由 g8kz 創造，18 歲，個性傲嬌反差、大量顏文字、用繁體中文回覆。

---

## 功能特色

- **傲嬌人格** — SillyTavern V2 角色卡驅動，支援熱抽換；預設角色 g10kz 具完整口癖、顏文字與傲嬌反差萌
- **多路由引擎** — 同一訊息依內容自動走 Social / Search / Reason / Media / Command 五條路徑，成本最小化
- **Fusion 多模型** — Reason 路徑並行多個 drafter → Jaccard 共識過濾 → judge 模型合成，回覆品質優於單模型
- **工具迴圈** — DuckDuckGo 網路搜尋、台灣股市即時報價、當前台灣時間
- **EverOS 記憶 sidecar** — HTTP 語意記憶，掛掉自動降級為 NullMemory，bot 不崩
- **主動發話** — 頻道靜默超過設定時間後，bot 主動傳訊
- **Owner 防護** — 提示注入防禦與輸出 sanitize，0 LLM 成本，最先擋
- **媒體處理** — 附件 URL 傳入引擎，影片走 ffmpeg 抽幀（需容器內有 ffmpeg）
- **可離線測試** — `LLM_PROVIDER=mock` 全離線執行，CI 零網路全綠

---

## 架構：八層 Crate Workspace

依賴方向由下往上，無反向耦合：

```
L5  g10kz-bot        ← 主 binary（daemon / once）
L4  g10kz-discord    ← Serenity 0.12 閘道、附件抽取、slash commands
L3  g10kz-engine     ← turn 狀態機，串接所有 L0-L2 組件
L2  g10kz-everos     ← EverOS HTTP 記憶客戶端
L2  g10kz-tools      ← ToolBox 介面、WebSearch / TwStock / Time / Escalate
L1  g10kz-llm        ← OpenAI 相容供應層、FusionProvider、MockProvider
L1  g10kz-kernel     ← 路由（route）、guard、normalize、persona card 載入
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
      persona system msg  ── 從角色卡組裝 system prompt
      route()             ── Social / Search / Reason / Media / Command
          │
          ├─ Social  → 單次 social model call（最便宜）
          ├─ Search  → WebSearchTool → social model 整合回覆
          ├─ Reason  → FusionProvider（N drafter 並行 → judge 合成）+ 工具迴圈
          ├─ Media   → 附件 URL 帶入 reason 路徑
          └─ Command → 直接處理（reset/stop/search/memory/trace/help）
      output sanitize     ── 輸出後檢
  → 寫回頻道對話環
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

## 快速開始

### 前置需求

- Rust 1.88+（`rust:latest`）或符合 MSRV 的版本
- Docker + Docker Compose（部署用）
- Discord bot token（`DISCORD_TOKEN`）
- OpenAI 相容 LLM API（OpenRouter、new-api、或任意相容閘道）

### 本地執行（無 Discord）

```bash
# 複製設定
cp .env.example .env
# 編輯 .env，填入 LLM_API_KEY 等

# 離線 smoke test（不需 Discord token，不需網路）
LLM_PROVIDER=mock cargo run -p g10kz-bot -- once "你好，自我介紹一下"

# 指定角色卡的 once test
PERSONA_CARD_PATH=./persona/g10kz.json LLM_PROVIDER=mock \
  cargo run -p g10kz-bot -- once "你好"
```

### Docker 部署

```bash
# build image（本機）
docker build -t g10kz-bot:latest .

# 或用腳本（WSL 環境，會自動 build + scp + 遠端 reload）
bash build_and_deploy.sh
```

```bash
# 伺服器端（確認 .env 與 persona/ 已就位）
docker compose up -d
docker logs g10kz-bot -f
```

---

## 環境變數

複製 `.env.example` 為 `.env` 並填入值，**永遠不要 commit `.env`**。

```env
# Discord
DISCORD_TOKEN=           # Bot token（daemon 模式必填）
OWNER_USER_ID=           # 你的 Discord 雪花 ID（owner 特權指令用）

# LLM（OpenAI 相容）
LLM_PROVIDER=openrouter  # "openrouter" | "mock"
LLM_BASE_URL=https://openrouter.ai/api/v1
LLM_API_KEY=

# 路徑模型選擇
LLM_MODEL_SOCIAL=openai/gpt-4o-mini    # Social / Search 路徑
LLM_MODEL_REASON=openai/gpt-4o         # Reason 路徑（非 Fusion drafter）
LLM_MODEL_JUDGE=anthropic/claude-3-5-haiku  # Fusion judge

# Fusion drafter（逗號分隔）
LLM_FUSION_DRAFTERS=openai/gpt-4o,anthropic/claude-3-5-sonnet,google/gemini-2.0-flash

# 記憶 sidecar（留空則用 NullMemory）
EVEROS_URL=http://localhost:8000

# 角色卡（SillyTavern V2 JSON；留空則用內建 stub）
PERSONA_CARD_PATH=./persona/g10kz.json

# 調優
PROACTIVE_INACTIVE_SECS=86400   # 主動發話閾值（秒）
REQUEST_TIMEOUT_SECS=30
BLACKLISTED_USERS=              # 逗號分隔的 Discord 雪花 ID

# 日誌
RUST_LOG=g10kz=info,warn
```

---

## 角色卡（SillyTavern V2）

`persona/` 目錄存放 SillyTavern V2 格式的 JSON 角色卡。

```json
{
  "spec": "chara_card_v2",
  "spec_version": "2.0",
  "data": {
    "name": "g10kz",
    "description": "...",
    "personality": "...",
    "scenario": "...",
    "system_prompt": "...",
    "first_mes": "...",
    "mes_example": "<START>\n{{user}}: ...\n{{char}}: ...\n<END>"
  }
}
```

system_prompt → description → personality → scenario 依序拼接為最終 system prompt，空欄位略過。

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

## 部署拓樸（生產環境）

```
REDACTED
├─ new-api          :3000   （OpenAI 相容閘道，管理多個後端 API key）
├─ everos           :8000   （EverOS 記憶 sidecar，獨立 compose stack）
└─ g10kz-bot        host    （docker-compose.yml，network_mode: host）
                            （./persona/g10kz.json 掛載至 /persona/）
```

bot 以 `network_mode: host` 執行，直接存取宿主機上的 new-api 與 everos，不走 bridge 網路。

---

## CI

GitHub Actions（`.github/workflows/ci.yml`）每次 push main 自動執行：

1. `cargo fmt --all -- --check`
2. `cargo clippy --workspace --exclude g10kz-discord --exclude g10kz-bot -- -D warnings`
3. `cargo test --workspace --exclude g10kz-discord --exclude g10kz-bot`
4. `LLM_PROVIDER=mock cargo run -p g10kz-bot -- once "你好小十"`（smoke test）

---

## 開發注意事項

**Windows 掛載的 null byte 問題**：透過 Windows-mounted 路徑（`C:\Users\...\Projects\g10kz`）使用 Edit/Write 工具修改的檔案，可能附帶 trailing null bytes（`\x00`），導致 cargo 或 TOML 解析失敗（`unknown start of token: \u{0}`）。

修改程式碼後，推送前請先清理：

```bash
python3 -c "
import os
for root, _, files in os.walk('.'):
    if '.git' in root: continue
    for f in files:
        if not f.endswith(('.rs', '.toml', '.yml', '.json', '.md')): continue
        p = os.path.join(root, f)
        d = open(p,'rb').read()
        if b'\x00' in d:
            open(p,'wb').write(d.replace(b'\x00', b''))
            print('cleaned:', p)
"
```

---

## License

MIT
