use crate::tools::{ToolError, ToolOutput, ToolRegistry};
use reqwest::{Client, Method, Proxy};
use url::Url;

pub async fn execute_http(
    method: &str,
    url: &str,
    body: Option<String>,
    registry: &ToolRegistry,
) -> Result<ToolOutput, ToolError> {
    registry.guard.check(url)?;
    let parsed = Url::parse(url).map_err(|e| ToolError::InvalidInput(e.to_string()))?;
    let host = parsed.host_str().ok_or_else(|| ToolError::InvalidInput("missing host".into()))?;
    if !registry.http_allow_all {
        let allowed = registry
            .http_allowed_domains
            .read()
            .unwrap()
            .iter()
            .any(|d| host == d || host.ends_with(&format!(".{}", d)));
        if !allowed {
            return Err(ToolError::NotAllowed);
        }
    }

    let mut builder = Client::builder().no_proxy();
    if let Some(proxy_url) = registry.http_proxy.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
        builder = builder
            .proxy(Proxy::all(proxy_url).map_err(|e| ToolError::InvalidInput(e.to_string()))?);
    }
    let client = builder
        .build()
        .map_err(|e| ToolError::Execution(e.to_string()))?;
    let parsed = Method::from_bytes(method.as_bytes())
        .map_err(|e| ToolError::InvalidInput(e.to_string()))?;
    let mut req = client.request(parsed, url);
    if let Some(b) = body {
        req = req.body(b);
    }
    let resp = req.send().await.map_err(|e| ToolError::Execution(e.to_string()))?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| ToolError::Execution(e.to_string()))?;
    Ok(ToolOutput {
        stdout: truncate(text, 4000),
        stderr: String::new(),
        exit_code: status.as_u16() as i32,
    })
}

fn truncate(mut s: String, max: usize) -> String {
    if s.len() > max {
        s.truncate(max);
        s.push_str("\n...[truncated]");
    }
    s
}
