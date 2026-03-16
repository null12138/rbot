use crate::tools::{ToolError, ToolOutput, ToolRegistry};
use reqwest::{Client, Proxy};
use serde_json::{json, Value};
use std::time::Duration;

pub async fn execute_search(
    query: String,
    count: Option<usize>,
    registry: &ToolRegistry,
) -> Result<ToolOutput, ToolError> {
    let limit = count.unwrap_or(registry.search_limit).max(1).min(10);
    let http = build_client(registry)?;
    let output = search_tavily(&http, &query, limit, registry).await?;
    Ok(ToolOutput {
        stdout: output,
        stderr: String::new(),
        exit_code: 0,
    })
}

fn build_client(registry: &ToolRegistry) -> Result<Client, ToolError> {
    let mut builder = Client::builder().no_proxy().timeout(Duration::from_secs(20));
    if let Some(proxy_url) = registry
        .http_proxy
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        builder = builder
            .proxy(Proxy::all(proxy_url).map_err(|e| ToolError::InvalidInput(e.to_string()))?);
    }
    builder.build().map_err(|e| ToolError::Execution(e.to_string()))
}

async fn search_tavily(
    http: &Client,
    query: &str,
    limit: usize,
    registry: &ToolRegistry,
) -> Result<String, ToolError> {
    let endpoint = registry
        .search_endpoint
        .clone()
        .unwrap_or_else(|| "https://api.tavily.com/search".to_string());
    let api_key = registry
        .search_api_key
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ToolError::InvalidInput("search api_key missing (tools.search.api_key)".into()))?;
    let payload = json!({
        "api_key": api_key,
        "query": query,
        "max_results": limit,
        "search_depth": "basic",
        "include_answer": false,
        "include_raw_content": false
    });
    let resp = http
        .post(endpoint)
        .json(&payload)
        .send()
        .await
        .map_err(|e| ToolError::Execution(e.to_string()))?
        .error_for_status()
        .map_err(|e| ToolError::Execution(e.to_string()))?;
    let value: Value = resp
        .json()
        .await
        .map_err(|e| ToolError::Execution(e.to_string()))?;
    let results = value
        .get("results")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();
    Ok(format_results(&results))
}

fn format_results(results: &[Value]) -> String {
    if results.is_empty() {
        return "No results".to_string();
    }
    let mut out = String::new();
    for (idx, item) in results.iter().take(10).enumerate() {
        let title = item
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("(no title)");
        let url = item
            .get("url")
            .or_else(|| item.get("link"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let snippet = item
            .get("content")
            .or_else(|| item.get("description"))
            .or_else(|| item.get("snippet"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        out.push_str(&format!("{}. {}\n", idx + 1, title));
        if !url.is_empty() {
            out.push_str(&format!("   {}\n", url));
        }
        if !snippet.is_empty() {
            out.push_str(&format!("   {}\n", truncate(snippet, 300)));
        }
    }
    out.trim_end().to_string()
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut out = s.to_string();
    out.truncate(max);
    out.push_str("...");
    out
}
