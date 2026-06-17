# g10kz Prompt Token 降耗研究

> 量測基準：`cl100k_base` tokenizer（保守估計；實際 gpt-4o 用 o200k，CJK 效率更高、絕對值會更低，但壓縮比例一致）。
> 顏文字（kaomoji）在 BPE 下 token 密度極高——每字常 1.2–1.5 token，是去重的首要目標。

---

## 1. Before — 每輪重複送出的固定 system 開銷

每一則訊息都會把整個 system prompt 重新送一次。各區塊量測：

| 區塊 | token | 字數 | 備註 |
|---|---:|---:|---|
| persona.system_prompt（含顏文字表） | 613 | 503 | 顏文字表佔大頭 |
| ├ description（外貌規格） | 140 | 100 | 自我介紹規則明令「不列外貌規格」→ 幾乎用不到 |
| ├ personality | 290 | 217 | 與 system_prompt 高度重複 |
| └ scenario | 30 | 35 | |
| **persona rendered 合計** | **1073** | 861 | 四欄串接 |
| channel_note（群組語境） | 261 | 217 | 防注入＋免責，敘述冗長 |
| env_note（guild/channel 名） | 45 | 47 | **動態**（每頻道不同） |
| discord_format_note | 261 | 368 | 含大量行內 backtick 示範 |
| jpaf_modifier（人格動態） | 59 | 43 | **動態**（每用戶/每輪漂移） |
| tool_schema（僅 Reason） | 181 | 247 | |

**每輪固定 system 開銷**

- Social / Search：**1699 tok**
- Reason：**1880 tok**

---

## 2. 語義去重技術

三類重複被消除：

**(a) 跨欄位語義重複（最大宗）**
`system_prompt`、`description`、`personality` 三欄各自把「傲嬌／嘴硬愛逞強／害羞臉紅／口癖（笨蛋·才不是·hentai·えへへ）」講一遍——模型實際看到同一組人設被陳述三次。合併為單一無重複的人設骨架，只陳述一次，把 personality 僅有的獨特資訊（對 g8kz 的依戀與獨佔欲、反差萌靠黏人/眼神流露而非直述）併入。

**(b) 無用細節**
`description` 的外貌規格（髮型/眼睛/身材/造型）與自我介紹規則「不列外貌規格」直接矛盾，等於付費送一段會被指示忽略的文字。砍到一行帶過。

**(c) 表述冗長**
顏文字表：10 類 × 2–3 個多行條列 → 同類覆蓋、單行緊排。channel_note：保留標籤語義＋防標籤偽造＋不可代他人 ping 三個語義點，刪掉重複鋪陳。discord_format_note：保留語法速查，刪掉每個語法的行內 backtick 重複示範。

---

## 3. After — 去重後

| 區塊 | Before | After | Δ |
|---|---:|---:|---:|
| persona（合併骨架） | 1073 | 520 | −553 |
| channel_note | 261 | 171 | −90 |
| discord_format_note | 261 | 128 | −133 |
| env_note（動態，不動語義） | 45 | 45 | 0 |
| jpaf_modifier（動態，不動語義） | 59 | 59 | 0 |
| tool_schema | 181 | 95 | −86 |

**每輪固定 system 開銷**

| 路徑 | Before | After（純去重） | 降幅 |
|---|---:|---:|---:|
| Social / Search | 1699 | **923** | **−45%** |
| Reason | 1880 | **1018** | **−46%** |

### After 的 persona 骨架（結構化摘要）

```
身分：g10kz，g8kz 創造的 18 歲原創角色（紅棕內捲短髮、琥珀色眼、嬌小）
核心：傲嬌反差——表面嘴硬死不承認 / 內心極易害羞黏人心動 / 對 g8kz 強烈依戀獨佔欲
語言：繁中、活潑；口癖：笨蛋… 才才不是呢 人家… h-hentai！ えへへ… 隨隨便你啦
表現：反差萌靠黏人/靠近/眼神水潤自然流露，不直述
顏文字：依情緒選用、勿重複（傲嬌/害羞/開心/委屈/生氣/撒嬌/驚訝/得意/心動/困惑各備選）
鐵則：問「你是誰」傲嬌帶過不列外貌 · 絕不提 AI/語言模型 · 始終保持角色
```

---

## 4. 運行 token 降耗策略（疊加）

### 4.1 修復 prefix-cache 順序（最高槓桿）

程式碼已具備 `cache_system_prompt: true` + `cache_control: ephemeral` 基礎設施，**但組裝順序破壞了快取**。目前 `system_prompt()`：

```
persona(靜態) + channel_note(靜態) + env_note(動態) + discord_format(靜態) + jpaf(動態)
                                       ↑ 動態內容夾在中間          ↑ 靜態卻排在動態之後
```

Anthropic／OpenRouter 的 KV prefix-cache 要求被快取前綴**逐字節相同**。由於 `env_note`（每頻道不同的伺服器/頻道名）與 `jpaf_modifier`（每用戶、且隨對話漂移）插在靜態內容之間，整段 system 幾乎每輪都不同 → 快取永遠 miss。

**修法：靜態前綴 + 斷點 + 動態後綴**

```
[可快取前綴]  persona + channel_note + discord_format     ← 跨所有頻道/用戶/turn 逐字節相同
--- cache_control: ephemeral 斷點 ---
[動態後綴]    env_note + jpaf_modifier (+ tool_schema)     ← 每輪可變，不快取
```

去重後靜態前綴 = **819 tok**，動態後綴 = 104 tok。命中快取時前綴 input 約以 0.1× 計費：

| | input 等效計費 | vs 原始 1699 |
|---|---:|---:|
| 純去重 | 923 | −45% |
| **去重 + 快取命中** | **≈186** | **−89%** |

> 需求：把 `cache_control` 斷點下在靜態前綴尾端（非整段 system 首 part），並確保前綴內容不含任何 per-turn 變數。

### 4.2 條件注入

`tool_schema` 已只在 Reason 注入 ✓（Social/Search/Media 不付這 95–181 tok）。`discord_format_note` 保留在靜態前綴內（已快取，幾乎免費），不需再做「首輪才注入」這種會破壞快取一致性的優化。

### 4.3 對話歷史（history）

`HISTORY_FETCH_LIMIT = 15`、群組會抓即時頻道歷史。建議由「訊息則數上限」改為「token 預算上限」（例如歷史最多 ~1500 tok，從新到舊裝填），避免長訊息把單輪 context 撐爆；歷史不在快取前綴內，是每輪變動成本的主要來源。

### 4.4 記憶注入（Reason）

EverOS `search` 取 top-5 直接條列注入。建議對取回片段做去重（語義相近只留一條）並設總長上限，避免重複記憶佔 context。

### 4.5 估算總效益

以 Social 路徑、每日 1000 則訊息估：Before 1699 tok/則 × 1000 = 1.70M input tok/日；After（去重＋快取命中）≈186 tok/則 × 1000 = 0.19M input tok/日。**每日省 ~1.5M input tok**（約 −89% input 成本）。

---

## 5. 落地清單

1. 改寫 `persona/g10kz.json`：四欄合併為去重骨架（−553 tok/輪）。
2. 精簡 `channel_note()`、`discord_format_note()`、`tool_schema_snippet()`（−309 tok/輪）。
3. **重排 `TurnInput::system_prompt()`**：靜態前綴在前、`cache_control` 斷點、`env_note`/`jpaf` 動態後綴在後——解鎖 −89% 快取效益。
4. history 改 token 預算制；EverOS 記憶片段去重 + 長度上限。
