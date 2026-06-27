//! Built-in tools: time, Taiwan stock quote, web search, fetch page, escalate.

use serde_json::{json, Value};
use tracing::warn;

use crate::tool::{BoxFuture, Tool, ToolCall, ToolResult};

fn ok(name: &str, content: String) -> ToolResult {
    ToolResult {
        name: name.into(),
        content,
        success: true,
    }
}
fn err(name: &str, msg: String) -> ToolResult {
    ToolResult {
        name: name.into(),
        content: msg,
        success: false,
    }
}

// ─── TimeTool ────────────────────────────────────────────────────────────────

pub struct TimeTool;

impl Tool for TimeTool {
    fn name(&self) -> &str {
        "current_time"
    }
    fn description(&self) -> &str {
        "回傳台灣當前日期與時間（UTC+8）。"
    }
    fn schema(&self) -> Value {
        json!({ "type": "object", "properties": {}, "required": [] })
    }
    fn call<'a>(&'a self, call: ToolCall) -> BoxFuture<'a, ToolResult> {
        Box::pin(async move {
            let now_utc = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let tw_secs = now_utc + 8 * 3600;
            let days = tw_secs / 86400;
            let time_of_day = tw_secs % 86400;
            let hh = time_of_day / 3600;
            let mm = (time_of_day % 3600) / 60;
            let ss = time_of_day % 60;
            let (year, month, day) = unix_days_to_ymd(days);
            let text =
                format!("{year:04}-{month:02}-{day:02} {hh:02}:{mm:02}:{ss:02} (台灣時間 UTC+8)");
            ok(call.name.as_str(), text)
        })
    }
}

fn unix_days_to_ymd(days: u64) -> (u64, u64, u64) {
    let z = days + 719468;
    let era = z / 146097;
    let doe = z % 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

// ─── EscalateTool ────────────────────────────────────────────────────────────

pub struct EscalateTool;

impl Tool for EscalateTool {
    fn name(&self) -> &str {
        "escalate"
    }
    fn description(&self) -> &str {
        "當問題超出社交對話範圍、需要深度分析時使用。觸發升級到推理路徑。"
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "reason": { "type": "string", "description": "為何需要升級" }
            },
            "required": ["reason"]
        })
    }
    fn call<'a>(&'a self, call: ToolCall) -> BoxFuture<'a, ToolResult> {
        Box::pin(async move { ok(call.name.as_str(), "ESCALATE".into()) })
    }
}

// ─── TwStockTool ─────────────────────────────────────────────────────────────

pub struct TwStockTool {
    client: reqwest::Client,
}

impl TwStockTool {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(5))
                .build()
                .unwrap(),
        }
    }
}

impl Default for TwStockTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for TwStockTool {
    fn name(&self) -> &str {
        "tw_stock"
    }
    fn description(&self) -> &str {
        "查詢台灣股票即時資訊（TWSE）。參數：stock_code（如 \"2330\" 代表台積電）。"
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "stock_code": { "type": "string", "description": "TWSE股票代號" }
            },
            "required": ["stock_code"]
        })
    }
    fn call<'a>(&'a self, call: ToolCall) -> BoxFuture<'a, ToolResult> {
        Box::pin(async move {
            let code = match call.arguments.get("stock_code").and_then(|v| v.as_str()) {
                Some(c) => c.to_owned(),
                None => return err(&call.name, "missing stock_code".into()),
            };
            let url = format!(
                "https://mis.twse.com.tw/stock/api/getStockInfo.jsp\
                 ?ex_ch=tse_{code}.tw&json=1&delay=0&_={ts}",
                code = code,
                ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis()
            );
            let resp = match self.client.get(&url).send().await {
                Ok(r) => r,
                Err(e) => {
                    warn!("tw_stock fetch error: {e}");
                    return err(&call.name, format!("無法取得股票資訊：{e}"));
                }
            };
            let json: Value = match resp.json().await {
                Ok(v) => v,
                Err(e) => return err(&call.name, format!("解析失敗：{e}")),
            };
            let info = json
                .get("msgArray")
                .and_then(|a| a.as_array())
                .and_then(|a| a.first());
            match info {
                Some(stock) => {
                    let name = stock.get("n").and_then(|v| v.as_str()).unwrap_or("N/A");
                    let price = stock.get("z").and_then(|v| v.as_str()).unwrap_or("-");
                    let open = stock.get("o").and_then(|v| v.as_str()).unwrap_or("-");
                    let high = stock.get("h").and_then(|v| v.as_str()).unwrap_or("-");
                    let low = stock.get("l").and_then(|v| v.as_str()).unwrap_or("-");
                    let vol = stock.get("v").and_then(|v| v.as_str()).unwrap_or("-");
                    ok(&call.name, format!(
                        "{code} {name}\n現價：{price}\n開盤：{open} 最高：{high} 最低：{low}\n成交量：{vol}張"
                    ))
                }
                None => err(&call.name, format!("找不到股票代號 {code}")),
            }
        })
    }
}

// ─── WebSearchTool ───────────────────────────────────────────────────────────

/// 網路搜尋：委託 gemini-search 微服務（Gemini API + Google Search grounding）
pub struct WebSearchTool {
    client: reqwest::Client,
    search_url: String,
}

impl WebSearchTool {
    pub fn new(search_url: Option<String>) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .unwrap();
        let search_url = search_url
            .or_else(|| std::env::var("GEMINI_SEARCH_URL").ok())
            .unwrap_or_else(|| "http://localhost:8090".into());
        Self { client, search_url }
    }
}

impl Default for WebSearchTool {
    fn default() -> Self {
        Self::new(None)
    }
}

impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }
    fn description(&self) -> &str {
        "搜尋網路最新資訊。由 Gemini Google Search grounding 驅動，結果附帶來源連結。參數：query（搜尋詞）。"
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "搜尋關鍵詞（繁體中文或英文）"
                }
            },
            "required": ["query"]
        })
    }

    fn call<'a>(&'a self, call: ToolCall) -> BoxFuture<'a, ToolResult> {
        Box::pin(async move {
            let query = match call.arguments.get("query").and_then(|v| v.as_str()) {
                Some(q) => q.to_owned(),
                None => return err(&call.name, "missing query".into()),
            };

            let endpoint = format!("{}/v1/search", self.search_url);
            let body = json!({ "query": query, "max_results": 5 });

            let resp = match self.client.post(&endpoint).json(&body).send().await {
                Ok(r) => r,
                Err(e) => {
                    warn!("web_search: gemini-search request failed: {e}");
                    return err(&call.name, format!("搜尋服務無法連線：{e}"));
                }
            };

            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                warn!("web_search: gemini-search HTTP {status}: {text}");
                return err(&call.name, format!("搜尋服務錯誤 {status}"));
            }

            let data: Value = match resp.json().await {
                Ok(v) => v,
                Err(e) => return err(&call.name, format!("搜尋結果解析失敗：{e}")),
            };

            if let Some(e) = data.get("error").and_then(|v| v.as_str()) {
                return err(&call.name, format!("搜尋錯誤：{e}"));
            }

            let summary = data
                .get("summary")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned();
            let results = data
                .get("results")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();

            if summary.is_empty() && results.is_empty() {
                return ok(
                    &call.name,
                    format!("找不到「{query}」的相關結果，請換個搜尋詞試試。"),
                );
            }

            let mut output = format!("## 搜尋：{query}\n\n");

            if !summary.is_empty() {
                let quoted = summary
                    .lines()
                    .map(|l| format!("> {l}"))
                    .collect::<Vec<_>>()
                    .join("\n");
                output.push_str(&quoted);
                output.push_str("\n\n");
            }

            if !results.is_empty() {
                output.push_str("**來源：**\n");
                for (i, r) in results.iter().enumerate().take(5) {
                    let title = r
                        .get("title")
                        .and_then(|v| v.as_str())
                        .unwrap_or("(無標題)");
                    let url = r.get("url").and_then(|v| v.as_str()).unwrap_or("");
                    output.push_str(&format!("-# [{i_n}] [{title}](<{url}>)\n", i_n = i + 1));
                }
            }

            ok(&call.name, output.trim().to_string())
        })
    }
}

// ─── FetchPageTool ───────────────────────────────────────────────────────────

/// 讀取網頁內容：優先用 stealth headless 瀏覽器（browser 微服務 /v1/render，
/// 真實 JS 渲染 + 反偵測），失敗時回退到 gemini-search /v1/fetch。
pub struct FetchPageTool {
    client: reqwest::Client,
    browser_url: String,
    search_url: String,
}

impl FetchPageTool {
    pub fn new(search_url: Option<String>) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(45))
            .build()
            .unwrap();
        let browser_url = std::env::var("BROWSER_URL")
            .unwrap_or_else(|_| "http://localhost:8091".into());
        let search_url = search_url
            .or_else(|| std::env::var("GEMINI_SEARCH_URL").ok())
            .unwrap_or_else(|| "http://localhost:8090".into());
        Self { client, browser_url, search_url }
    }

    /// Render a page via the stealth headless browser. Returns the cleaned
    /// article text (with a truncation marker) or `None` on any failure.
    async fn browser_render(&self, url: &str) -> Option<String> {
        let endpoint = format!("{}/v1/render", self.browser_url);
        let body = json!({ "url": url, "max_chars": 6000 });
        let resp = self.client.post(&endpoint).json(&body).send().await.ok()?;
        if !resp.status().is_success() {
            warn!("fetch_page: browser HTTP {}", resp.status());
            return None;
        }
        let data: Value = resp.json().await.ok()?;
        let content = data.get("content").and_then(|v| v.as_str()).unwrap_or("");
        if content.trim().len() < 50 {
            return None;
        }
        let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("");
        let truncated = data.get("truncated").and_then(|v| v.as_bool()).unwrap_or(false);
        let mut out = String::new();
        if !title.is_empty() {
            out.push_str(&format!("# {title}\n\n"));
        }
        out.push_str(content.trim());
        if truncated {
            out.push_str("\n\n[內容已截斷]");
        }
        Some(out)
    }

    /// Fallback: ask gemini-search to fetch + summarise the URL.
    async fn gemini_fetch(&self, url: &str) -> Option<String> {
        let endpoint = format!("{}/v1/fetch", self.search_url);
        let body = json!({ "url": url, "max_chars": 4000 });
        let resp = self.client.post(&endpoint).json(&body).send().await.ok()?;
        if !resp.status().is_success() {
            warn!("fetch_page: gemini-search HTTP {}", resp.status());
            return None;
        }
        let data: Value = resp.json().await.ok()?;
        let content = data.get("content").and_then(|v| v.as_str()).unwrap_or("");
        if content.trim().is_empty() {
            return None;
        }
        let truncated = data.get("truncated").and_then(|v| v.as_bool()).unwrap_or(false);
        let note = if truncated { "\n\n[內容已截斷]" } else { "" };
        Some(format!("{content}{note}"))
    }
}

impl Default for FetchPageTool {
    fn default() -> Self {
        Self::new(None)
    }
}

impl Tool for FetchPageTool {
    fn name(&self) -> &str {
        "fetch_page"
    }
    fn description(&self) -> &str {
        "讀取指定網頁的完整內容，回傳文字摘要。適合讀取文章、GitHub README、技術文件、新聞等。\
         參數：url（完整網址，須含 https:// 或 http://）。"
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "要讀取的網頁完整 URL（須含 https:// 或 http://）"
                }
            },
            "required": ["url"]
        })
    }
    fn call<'a>(&'a self, call: ToolCall) -> BoxFuture<'a, ToolResult> {
        Box::pin(async move {
            let url = match call.arguments.get("url").and_then(|v| v.as_str()) {
                Some(u) => u.to_owned(),
                None => return err(&call.name, "missing url".into()),
            };

            // 1. Stealth headless browser (real JS rendering + anti-detection).
            if let Some(text) = self.browser_render(&url).await {
                return ok(&call.name, text);
            }
            warn!("fetch_page: browser render failed, falling back to gemini-search");

            // 2. Fallback: gemini-search fetch + summarise.
            if let Some(text) = self.gemini_fetch(&url).await {
                return ok(&call.name, text);
            }

            err(&call.name, format!("無法讀取頁面：{url}"))
        })
    }
}

// ─── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn time_tool_returns_datetime() {
        let call = ToolCall {
            name: "current_time".into(),
            arguments: json!({}),
        };
        let result = TimeTool.call(call).await;
        assert!(result.success);
        assert!(result.content.contains("UTC+8"));
        assert!(result.content.contains("20"));
    }

    #[test]
    fn unix_days_to_ymd_epoch() {
        assert_eq!(unix_days_to_ymd(0), (1970, 1, 1));
    }

    #[test]
    fn unix_days_to_ymd_known() {
        assert_eq!(unix_days_to_ymd(19723), (2024, 1, 1));
    }

    #[tokio::test]
    async fn escalate_tool_returns_sentinel() {
        let call = ToolCall {
            name: "escalate".into(),
            arguments: json!({"reason": "complex"}),
        };
        let result = EscalateTool.call(call).await;
        assert!(result.success);
        assert_eq!(result.content, "ESCALATE");
    }

    #[tokio::test]
    async fn tw_stock_missing_arg() {
        let call = ToolCall {
            name: "tw_stock".into(),
            arguments: json!({}),
        };
        assert!(!TwStockTool::new().call(call).await.success);
    }

    #[tokio::test]
    async fn web_search_missing_query() {
        let call = ToolCall {
            name: "web_search".into(),
            arguments: json!({}),
        };
        assert!(!WebSearchTool::new(None).call(call).await.success);
    }

    #[tokio::test]
    async fn fetch_page_missing_url() {
        let call = ToolCall {
            name: "fetch_page".into(),
            arguments: json!({}),
        };
        assert!(!FetchPageTool::new(None).call(call).await.success);
    }
}
