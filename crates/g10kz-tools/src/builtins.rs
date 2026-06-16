//! Built-in tools: time, Taiwan stock quote, web search, escalate.

use serde_json::{json, Value};
use tracing::warn;

use crate::tool::{BoxFuture, Tool, ToolCall, ToolResult};

// ─── helpers ─────────────────────────────────────────────────────────────────

fn ok(name: &str, content: String) -> ToolResult {
    ToolResult { name: name.into(), content, success: true }
}
fn err(name: &str, msg: String) -> ToolResult {
    ToolResult { name: name.into(), content: msg, success: false }
}

// ─── TimeTool ────────────────────────────────────────────────────────────────

/// Returns current date-time in Taiwan (UTC+8).
pub struct TimeTool;

impl Tool for TimeTool {
    fn name(&self) -> &str { "current_time" }
    fn description(&self) -> &str { "回傳台灣當前日期與時間（UTC+8）。" }
    fn schema(&self) -> Value {
        json!({ "type": "object", "properties": {}, "required": [] })
    }
    fn call<'a>(&'a self, call: ToolCall) -> BoxFuture<'a, ToolResult> {
        Box::pin(async move {
            // UTC + 8 hours, computed without external crate dependency
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

            // Gregorian calendar calculation (Tomohiko Sakamoto algorithm inspired)
            let (year, month, day) = unix_days_to_ymd(days);

            let text = format!("{year:04}-{month:02}-{day:02} {hh:02}:{mm:02}:{ss:02} (台灣時間 UTC+8)");
            ok(call.name.as_str(), text)
        })
    }
}

/// Convert unix days to (year, month, day).
fn unix_days_to_ymd(days: u64) -> (u64, u64, u64) {
    // Days since 1970-01-01
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

/// Sentinel: signals the engine to escalate from social → reason path.
/// The tool loop recognises the "escalate" name and returns `ESCALATE`.
pub struct EscalateTool;

impl Tool for EscalateTool {
    fn name(&self) -> &str { "escalate" }
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
        // The loop intercepts "escalate" before calling this, but provide a
        // safe impl in case it's invoked directly.
        Box::pin(async move { ok(call.name.as_str(), "ESCALATE".into()) })
    }
}

// ─── TwStockTool ─────────────────────────────────────────────────────────────

/// Fetches real-time Taiwan Stock Exchange (TWSE) data.
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
    fn default() -> Self { Self::new() }
}

impl Tool for TwStockTool {
    fn name(&self) -> &str { "tw_stock" }
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

            let info = json.get("msgArray")
                .and_then(|a| a.as_array())
                .and_then(|a| a.first());

            match info {
                Some(stock) => {
                    let name  = stock.get("n").and_then(|v| v.as_str()).unwrap_or("N/A");
                    let price = stock.get("z").and_then(|v| v.as_str()).unwrap_or("-");
                    let open  = stock.get("o").and_then(|v| v.as_str()).unwrap_or("-");
                    let high  = stock.get("h").and_then(|v| v.as_str()).unwrap_or("-");
                    let low   = stock.get("l").and_then(|v| v.as_str()).unwrap_or("-");
                    let vol   = stock.get("v").and_then(|v| v.as_str()).unwrap_or("-");
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

/// Cloudflare AI Search — or DuckDuckGo fallback when not configured.
pub struct WebSearchTool {
    client: reqwest::Client,
    account_id: String,
    api_token: String,
}

impl WebSearchTool {
    pub fn new(account_id: impl Into<String>, api_token: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(8))
                .build()
                .unwrap(),
            account_id: account_id.into(),
            api_token: api_token.into(),
        }
    }

    pub fn from_config(config: &g10kz_config::Config) -> Self {
        Self::new(&config.cf_account_id, &config.cf_api_token)
    }

    fn has_cloudflare(&self) -> bool {
        !self.account_id.is_empty() && !self.api_token.is_empty()
    }
}

impl Default for WebSearchTool {
    fn default() -> Self { Self::new("", "") }
}

impl Tool for WebSearchTool {
    fn name(&self) -> &str { "web_search" }
    fn description(&self) -> &str { "搜尋網路上的最新資訊。參數：query（搜尋詞）。" }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "搜尋關鍵詞" }
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

            // Cloudflare AI Search path
            if self.has_cloudflare() {
                let url = format!(
                    "https://api.cloudflare.com/client/v4/accounts/{}/ai/run/@cf/cloudflare/ai-search",
                    self.account_id
                );
                match self.client
                    .post(&url)
                    .bearer_auth(&self.api_token)
                    .json(&json!({ "query": query }))
                    .send()
                    .await
                {
                    Ok(r) if r.status().is_success() => {
                        match r.json::<Value>().await {
                            Ok(v) => {
                                let result = serde_json::to_string_pretty(&v)
                                    .unwrap_or_else(|_| v.to_string());
                                return ok(&call.name, result);
                            }
                            Err(e) => warn!("cf search parse error: {e}"),
                        }
                    }
                    Ok(r) => warn!("cf search HTTP {}", r.status()),
                    Err(e) => warn!("cf search error: {e}"),
                }
            }

            // DuckDuckGo fallback (Instant Answer API)
            let ddg_url = format!(
                "https://api.duckduckgo.com/?q={}&format=json&no_html=1&skip_disambig=1",
                urlencoding(&query)
            );
            match self.client.get(&ddg_url).send().await {
                Ok(r) if r.status().is_success() => {
                    match r.json::<Value>().await {
                        Ok(v) => {
                            let abstract_text = v.get("AbstractText")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            let related: Vec<&str> = v.get("RelatedTopics")
                                .and_then(|a| a.as_array())
                                .map(|a| a.iter()
                                    .take(3)
                                    .filter_map(|t| t.get("Text").and_then(|v| v.as_str()))
                                    .collect())
                                .unwrap_or_default();

                            let mut out = if abstract_text.is_empty() {
                                String::new()
                            } else {
                                format!("{abstract_text}\n")
                            };
                            for r in related { out.push_str(&format!("• {r}\n")); }
                            if out.trim().is_empty() {
                                ok(&call.name, format!("查不到「{query}」的相關資訊"))
                            } else {
                                ok(&call.name, out.trim().to_string())
                            }
                        }
                        Err(e) => err(&call.name, format!("搜尋結果解析失敗：{e}")),
                    }
                }
                Ok(r) => err(&call.name, format!("搜尋 HTTP {}",  r.status())),
                Err(e) => err(&call.name, format!("搜尋失敗：{e}")),
            }
        })
    }
}

fn urlencoding(s: &str) -> String {
    s.chars().map(|c| {
        if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~') {
            c.to_string()
        } else {
            format!("%{:02X}", c as u32)
        }
    }).collect()
}

// ─── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn time_tool_returns_datetime() {
        let call = ToolCall {
            name: "current_time".into(),
            arguments: serde_json::json!({}),
        };
        let result = TimeTool.call(call).await;
        assert!(result.success, "TimeTool should succeed");
        assert!(result.content.contains("UTC+8"), "got: {}", result.content);
        assert!(result.content.contains("20"), "should contain year 20xx: {}", result.content);
    }

    #[test]
    fn unix_days_to_ymd_epoch() {
        let (y, m, d) = unix_days_to_ymd(0);
        assert_eq!((y, m, d), (1970, 1, 1));
    }

    #[test]
    fn unix_days_to_ymd_known() {
        // 2024-01-01 = 19723 days since epoch
        let (y, m, d) = unix_days_to_ymd(19723);
        assert_eq!((y, m, d), (2024, 1, 1));
    }

    #[tokio::test]
    async fn escalate_tool_returns_sentinel() {
        let call = ToolCall { name: "escalate".into(), arguments: json!({"reason": "complex"}) };
        let result = EscalateTool.call(call).await;
        assert!(result.success);
        assert_eq!(result.content, "ESCALATE");
    }

    #[tokio::test]
    async fn tw_stock_missing_arg_returns_err() {
        let call = ToolCall { name: "tw_stock".into(), arguments: json!({}) };
        let result = TwStockTool::new().call(call).await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn web_search_missing_query_returns_err() {
        let call = ToolCall { name: "web_search".into(), arguments: json!({}) };
        let result = WebSearchTool::default().call(call).await;
        assert!(!result.success);
    }

    #[test]
    fn urlencoding_ascii() {
        assert_eq!(urlencoding("hello world"), "hello%20world");
    }
}
