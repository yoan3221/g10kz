//! Built-in tools: time, Taiwan stock quote, web search, escalate.

use std::path::PathBuf;

use serde_json::{json, Value};
use tracing::warn;

use crate::tool::{BoxFuture, Tool, ToolCall, ToolResult};

fn ok(name: &str, content: String) -> ToolResult {
    ToolResult { name: name.into(), content, success: true }
}
fn err(name: &str, msg: String) -> ToolResult {
    ToolResult { name: name.into(), content: msg, success: false }
}

// ─── TimeTool ────────────────────────────────────────────────────────────────

pub struct TimeTool;

impl Tool for TimeTool {
    fn name(&self) -> &str { "current_time" }
    fn description(&self) -> &str { "回傳台灣當前日期與時間（UTC+8）。" }
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
            let text = format!("{year:04}-{month:02}-{day:02} {hh:02}:{mm:02}:{ss:02} (台灣時間 UTC+8)");
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

/// Web search: DuckDuckGo lite HTML for results + Obscura for full page content.
///
/// Search flow:
/// 1. POST to DDG lite → parse top hits (title, url, snippet)
/// 2. Fetch top 3 pages via Obscura (anti-detection, JS rendering)
/// 3. BM25-inspired keyword scoring → extract most relevant passages
/// 4. Return Discord Markdown formatted result
pub struct WebSearchTool {
    client: reqwest::Client,
    /// Path to the `obscura` binary. `None` → snippet-only fallback.
    pub obscura_path: Option<PathBuf>,
}

impl WebSearchTool {
    pub fn new(obscura_path: Option<PathBuf>) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(12))
            .user_agent(
                "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
                 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36",
            )
            .build()
            .unwrap();
        Self { client, obscura_path }
    }

    async fn ddg_search(&self, query: &str) -> Vec<SearchHit> {
        let body = format!("q={}", url_encode(query));
        let resp = match self.client
            .post("https://lite.duckduckgo.com/lite/")
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(body)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => { warn!("ddg_search: {e}"); return vec![]; }
        };
        match resp.text().await {
            Ok(html) => parse_ddg_lite(&html),
            Err(e) => { warn!("ddg_search read: {e}"); vec![] }
        }
    }

    async fn obscura_fetch(&self, url: &str) -> Option<String> {
        let path = self.obscura_path.as_ref()?;
        let out = tokio::process::Command::new(path)
            .args(["fetch", url, "--dump", "text", "--timeout", "8", "--quiet"])
            .output()
            .await
            .ok()?;
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        if text.trim().len() < 50 { None } else { Some(text) }
    }
}

impl Default for WebSearchTool {
    fn default() -> Self {
        let obscura_path = ["/usr/local/bin/obscura", "/usr/bin/obscura"]
            .iter()
            .find(|p| std::path::Path::new(p).exists())
            .map(PathBuf::from);
        Self::new(obscura_path)
    }
}

impl Tool for WebSearchTool {
    fn name(&self) -> &str { "web_search" }
    fn description(&self) -> &str {
        "搜尋網路最新資訊。自動取得頁面全文並提取最相關段落。參數：query（搜尋詞）。"
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "搜尋關鍵詞（繁體中文或英文）"
                },
                "fetch_pages": {
                    "type": "boolean",
                    "description": "是否抓取頁面全文（預設 true）"
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
            let do_fetch = call.arguments
                .get("fetch_pages")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);

            let hits = self.ddg_search(&query).await;
            if hits.is_empty() {
                return ok(&call.name, format!("找不到「{query}」的相關結果，請換個搜尋詞試試。"));
            }

            let query_terms: Vec<String> =
                query.split_whitespace().map(|s| s.to_lowercase()).collect();
            let term_refs: Vec<&str> = query_terms.iter().map(|s| s.as_str()).collect();
            let top_hits: Vec<&SearchHit> = hits.iter().take(3).collect();

            // Fetch pages concurrently
            let page_contents: Vec<Option<String>> =
                if do_fetch && self.obscura_path.is_some() {
                    let futs: Vec<_> =
                        top_hits.iter().map(|h| self.obscura_fetch(&h.url)).collect();
                    futures::future::join_all(futs).await
                } else {
                    vec![None; top_hits.len()]
                };

            // Build Discord-formatted output
            let mut output = format!("## 🔍 {query}\n\n");

            for (i, (hit, content)) in
                top_hits.iter().zip(page_contents.into_iter()).enumerate()
            {
                let num = i + 1;
                output.push_str(&format!("**[{num}] {}**\n", hit.title));
                output.push_str(&format!("-# <{}>\n", hit.url));

                let body = if let Some(text) = content {
                    extract_relevant(&text, &term_refs, 700)
                } else if !hit.snippet.is_empty() {
                    hit.snippet.chars().take(350).collect()
                } else {
                    String::new()
                };

                if !body.is_empty() {
                    // Quote-format for Discord
                    let quoted = body.lines()
                        .map(|l| format!("> {l}"))
                        .collect::<Vec<_>>()
                        .join("\n");
                    output.push_str(&quoted);
                    output.push('\n');
                }
                output.push('\n');
            }

            // Remaining results as small footnotes
            if hits.len() > 3 {
                output.push_str("-# 更多結果：");
                for hit in hits.iter().skip(3).take(3) {
                    output.push_str(&format!("[{}](<{}>) · ", hit.title, hit.url));
                }
                output.push('\n');
            }

            ok(&call.name, output.trim().to_string())
        })
    }
}

// ─── SearchHit ───────────────────────────────────────────────────────────────

struct SearchHit {
    title: String,
    url: String,
    snippet: String,
}

// ─── DDG lite HTML parser ────────────────────────────────────────────────────

fn parse_ddg_lite(html: &str) -> Vec<SearchHit> {
    let mut hits: Vec<SearchHit> = Vec::new();
    let mut pending_url: Option<String> = None;
    let mut pending_title: Option<String> = None;
    let mut in_snippet = false;
    let mut snippet_buf = String::new();

    for line in html.lines() {
        let t = line.trim();

        if t.contains("class='result-link'") || t.contains("class=\"result-link\"") {
            // Flush previous result
            if let (Some(url), Some(title)) = (pending_url.take(), pending_title.take()) {
                hits.push(SearchHit { url, title, snippet: snippet_buf.trim().to_string() });
                snippet_buf.clear();
                if hits.len() >= 6 { return hits; }
            }
            if let Some(url) = extract_href(t) {
                let title = extract_inner_text(t).unwrap_or_else(|| url.clone());
                pending_url = Some(url);
                pending_title = Some(title);
            }
            in_snippet = false;

        } else if t.contains("class='result-snippet'") || t.contains("class=\"result-snippet\"") {
            in_snippet = true;
            if let Some(pos) = t.rfind('>') {
                let after = t[pos + 1..].trim();
                if !after.is_empty() {
                    snippet_buf.push_str(&strip_html(after));
                    snippet_buf.push(' ');
                }
            }
        } else if in_snippet {
            if t.starts_with("</td>") || t.starts_with("</tr>") {
                in_snippet = false;
            } else if !t.is_empty() {
                snippet_buf.push_str(&strip_html(t));
                snippet_buf.push(' ');
            }
        }
    }

    if let (Some(url), Some(title)) = (pending_url, pending_title) {
        hits.push(SearchHit { url, title, snippet: snippet_buf.trim().to_string() });
    }
    hits
}

fn extract_href(line: &str) -> Option<String> {
    for (prefix, quote) in &[("href=\"", '"'), ("href='", '\'')] {
        if let Some(start) = line.find(prefix) {
            let rest = &line[start + prefix.len()..];
            if let Some(end) = rest.find(*quote) {
                let url = &rest[..end];
                if url.starts_with("http") {
                    return Some(url.to_string());
                }
            }
        }
    }
    None
}

fn extract_inner_text(line: &str) -> Option<String> {
    let start = line.rfind('>')?;
    let rest = &line[start + 1..];
    let end = rest.find('<').unwrap_or(rest.len());
    let text = strip_html(&rest[..end]).trim().to_string();
    if text.is_empty() { None } else { Some(text) }
}

fn strip_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&amp;", "&")
       .replace("&lt;", "<")
       .replace("&gt;", ">")
       .replace("&quot;", "\"")
       .replace("&#39;", "'")
       .replace("&apos;", "'")
       .replace("&nbsp;", " ")
}

// ─── BM25-inspired relevance extraction ──────────────────────────────────────

fn extract_relevant(content: &str, query_terms: &[&str], max_chars: usize) -> String {
    let paragraphs: Vec<&str> = content
        .split('\n')
        .map(str::trim)
        .filter(|s| s.len() > 20)
        .collect();

    if paragraphs.is_empty() {
        return content.chars().take(max_chars).collect();
    }

    let mut scored: Vec<(usize, usize, &str)> = paragraphs
        .iter()
        .enumerate()
        .map(|(idx, &p)| {
            let p_lower = p.to_lowercase();
            let score = query_terms.iter().filter(|&&t| p_lower.contains(t)).count();
            (score, idx, p)
        })
        .collect();

    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));

    let mut result = String::new();
    for (_, _, para) in &scored {
        if result.len() + para.len() + 2 > max_chars { break; }
        if !result.is_empty() { result.push('\n'); }
        result.push_str(para);
    }

    if result.is_empty() {
        content.chars().take(max_chars).collect()
    } else {
        result
    }
}

// ─── URL encoding ────────────────────────────────────────────────────────────

fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~') {
            out.push(c);
        } else if c == ' ' {
            out.push('+');
        } else {
            for b in c.to_string().bytes() {
                out.push_str(&format!("%{:02X}", b));
            }
        }
    }
    out
}

// ─── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn time_tool_returns_datetime() {
        let call = ToolCall { name: "current_time".into(), arguments: json!({}) };
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
        let call = ToolCall { name: "escalate".into(), arguments: json!({"reason": "complex"}) };
        let result = EscalateTool.call(call).await;
        assert!(result.success);
        assert_eq!(result.content, "ESCALATE");
    }

    #[tokio::test]
    async fn tw_stock_missing_arg() {
        let call = ToolCall { name: "tw_stock".into(), arguments: json!({}) };
        assert!(!TwStockTool::new().call(call).await.success);
    }

    #[tokio::test]
    async fn web_search_missing_query() {
        let call = ToolCall { name: "web_search".into(), arguments: json!({}) };
        assert!(!WebSearchTool::new(None).call(call).await.success);
    }

    #[test]
    fn strip_html_basic() {
        assert_eq!(strip_html("<b>hello</b> &amp; world"), "hello & world");
    }

    #[test]
    fn url_encode_basic() {
        assert_eq!(url_encode("hello world"), "hello+world");
    }

    #[test]
    fn parse_ddg_lite_extracts_hits() {
        let html = r#"
            <tr><td>
              <a rel="nofollow" href="https://example.com/" class='result-link'>Example Site</a>
            </td></tr>
            <tr><td class='result-snippet'>
              An <b>example</b> snippet about Rust.
            </td></tr>
            <tr><td>
              <a rel="nofollow" href="https://foo.org/bar" class='result-link'>Foo Bar</a>
            </td></tr>
            <tr><td class='result-snippet'>Another snippet.</td></tr>
        "#;
        let hits = parse_ddg_lite(html);
        assert!(!hits.is_empty(), "should parse hits");
        assert_eq!(hits[0].url, "https://example.com/");
        assert_eq!(hits[0].title, "Example Site");
        assert!(hits[0].snippet.contains("example") || hits[0].snippet.contains("Rust"),
            "snippet: '{}'", hits[0].snippet);
    }

    #[test]
    fn extract_relevant_prefers_query_terms() {
        let content = "Rust is fast.\nPython is slow.\nRust has memory safety.\nJava is verbose.";
        let result = extract_relevant(content, &["rust"], 200);
        assert!(result.contains("Rust"), "result: {result}");
    }
}
