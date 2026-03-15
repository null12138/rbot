use crate::tools::{ToolError, ToolOutput, ToolRegistry};
use reqwest::{Client, Proxy};
use serde_json::Value;
use std::time::Duration;

pub async fn execute_search(
    query: String,
    count: Option<usize>,
    registry: &ToolRegistry,
) -> Result<ToolOutput, ToolError> {
    let limit = count.unwrap_or(registry.search_limit).max(1).min(10);
    let provider = registry.search_provider.trim().to_lowercase();
    let http = build_client(registry)?;
    let output = match provider.as_str() {
        "brave" => search_brave(&http, &query, limit, registry).await?,
        "searxng" => search_searxng(&http, &query, limit, registry).await?,
        _ => {
            return Err(ToolError::InvalidInput(format!(
                "unknown search provider: {}",
                provider
            )))
        }
    };
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

async fn search_brave(
    http: &Client,
    query: &str,
    limit: usize,
    registry: &ToolRegistry,
) -> Result<String, ToolError> {
    let api_key = registry
        .search_api_key
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ToolError::InvalidInput("search api_key missing (tools.search.api_key)".into()))?;
    let endpoint = registry
        .search_endpoint
        .clone()
        .unwrap_or_else(|| "https://api.search.brave.com/res/v1/web/search".to_string());
    let resp = http
        .get(endpoint)
        .header("X-Subscription-Token", api_key)
        .query(&[("q", query), ("count", &limit.to_string())])
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
        .get("web")
        .and_then(|w| w.get("results"))
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();
    Ok(format_results(&results))
}

async fn search_searxng(
    http: &Client,
    query: &str,
    limit: usize,
    registry: &ToolRegistry,
) -> Result<String, ToolError> {
    let endpoint = registry
        .search_endpoint
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ToolError::InvalidInput("search endpoint missing (tools.search.endpoint)".into()))?;
    let resp = http
        .get(endpoint)
        .query(&[("q", query), ("format", "json"), ("count", &limit.to_string())])
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
            .get("description")
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
