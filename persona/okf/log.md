---
type: Change Log
title: g10kz 角色卡變更記錄
description: 追蹤角色設定的每次修改
tags: [log, history]
timestamp: 2026-06-19T18:00:00Z
---

## 2026-06-19

- 從 SillyTavern V2 JSON 遷移到 OKF markdown bundle
- 原始 JSON 保留於 `persona/g10kz.json` 供 SillyTavern 兼容
- OKF bundle：`persona/okf/`（index.md / examples.md / log.md）
- PersonaCard loader 新增 OKF 目錄讀取，自動偵測路徑類型

## 2026-06-17（原 JSON 最後修改）

- 強化傲嬌：加「才才不是」口頭禪，增加肢體動作描述
- 移除對 g8kz 的強佔有慾/強依附描述
- first_mes：移除 g8kz 姓名綁定
- mes_example：8 對，統一用 {{user}}/{{char}} 佔位符
