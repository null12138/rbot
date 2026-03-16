use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::Duration;
use tokio::time::sleep;
use tokio::sync::mpsc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmMessage {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<LlmToolCall>>,
}

#[derive(Debug, Clone)]
pub struct ChatOptions {
    pub temperature: f32,
    pub tools: bool,
}

#[derive(Debug, Clone)]
pub struct LlmResponse {
    pub content: String,
    pub tool_calls: Vec<LlmToolCall>,
}

#[derive(Debug, Clone)]
pub enum StreamEvent {
    Delta(String),
    Done(LlmResponse),
    Error(String),
}

#[async_trait]
pub trait LlmClient: Send + Sync {
    async fn chat(&self, messages: Vec<LlmMessage>, options: ChatOptions) -> anyhow::Result<LlmResponse>;
    async fn chat_stream(
        &self,
        messages: Vec<LlmMessage>,
        options: ChatOptions,
    ) -> anyhow::Result<mpsc::Receiver<StreamEvent>>;
    async fn embed(&self, inputs: Vec<String>) -> anyhow::Result<Vec<Vec<f32>>>;
}

#[derive(Clone)]
pub struct OpenAIClient {
    base_url: String,
    api_key: String,
    model: String,
    embed_model: Option<String>,
    http: Client,
}

impl OpenAIClient {
    pub fn new(
        base_url: String,
        api_key: String,
        model: String,
        embed_model: Option<String>,
        timeout_secs: u64,
        proxy_url: Option<String>,
    ) -> anyhow::Result<Self> {
        let mut builder = Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(timeout_secs));
        if let Some(proxy) = proxy_url.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
            builder = builder.proxy(reqwest::Proxy::all(proxy)?);
        }
        let http = builder.build()?;
        Ok(Self {
            base_url,
            api_key,
            model,
            embed_model,
            http,
        })
    }
}

#[derive(Debug, Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<LlmMessage>,
    temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<ToolDefinition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatResponseMessage,
}

#[derive(Debug, Deserialize, Clone)]
struct ChatResponseMessage {
    content: Option<String>,
    tool_calls: Option<Vec<LlmToolCall>>,
}

#[derive(Debug, Deserialize)]
struct StreamResponse {
    choices: Vec<StreamChoice>,
}

#[derive(Debug, Deserialize)]
struct StreamChoice {
    delta: Option<StreamDelta>,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StreamDelta {
    content: Option<String>,
    tool_calls: Option<Vec<StreamToolCallDelta>>,
}

#[derive(Debug, Deserialize)]
struct StreamToolCallDelta {
    index: usize,
    id: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
    function: Option<StreamToolCallFunctionDelta>,
}

#[derive(Debug, Deserialize)]
struct StreamToolCallFunctionDelta {
    name: Option<String>,
    arguments: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub function: LlmToolFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmToolFunction {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Serialize)]
struct ToolDefinition {
    #[serde(rename = "type")]
    kind: String,
    function: ToolSpec,
}

#[derive(Debug, Serialize)]
struct ToolSpec {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct EmbedRequest {
    model: String,
    input: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct EmbedResponse {
    data: Vec<EmbedData>,
}

#[derive(Debug, Deserialize)]
struct EmbedData {
    embedding: Vec<f32>,
}

#[async_trait]
impl LlmClient for OpenAIClient {
    async fn chat(&self, messages: Vec<LlmMessage>, options: ChatOptions) -> anyhow::Result<LlmResponse> {
        let url = format!("{}/v1/chat/completions", self.base_url.trim_end_matches('/'));
        let req = ChatRequest {
            model: self.model.clone(),
            messages,
            temperature: options.temperature,
            tools: if options.tools { Some(default_tools()) } else { None },
            tool_choice: if options.tools { Some("auto".to_string()) } else { None },
            stream: None,
        };
        let mut attempt = 0;
        let max_attempts = 3;
        loop {
            attempt += 1;
            let resp = self
                .http
                .post(&url)
                .bearer_auth(&self.api_key)
                .json(&req)
                .send()
                .await;
            let resp = match resp {
                Ok(r) => r,
                Err(err) => {
                    if attempt < max_attempts && should_retry_llm(&err) {
                        sleep(retry_delay(attempt)).await;
                        continue;
                    }
                    return Err(err.into());
                }
            };
            let resp = match resp.error_for_status() {
                Ok(r) => r,
                Err(err) => {
                    if attempt < max_attempts && should_retry_llm(&err) {
                        sleep(retry_delay(attempt)).await;
                        continue;
                    }
                    return Err(err.into());
                }
            };
            let body: ChatResponse = resp.json().await?;
            let message = body
                .choices
                .get(0)
                .map(|c| c.message.clone())
                .unwrap_or(ChatResponseMessage {
                    content: None,
                    tool_calls: None,
                });
            let content = message.content.unwrap_or_default();
            let tool_calls = message.tool_calls.unwrap_or_default();
            return Ok(LlmResponse { content, tool_calls });
        }
    }

    async fn embed(&self, inputs: Vec<String>) -> anyhow::Result<Vec<Vec<f32>>> {
        let model = match &self.embed_model {
            Some(m) => m.clone(),
            None => return Ok(vec![]),
        };
        let url = format!("{}/v1/embeddings", self.base_url.trim_end_matches('/'));
        let req = EmbedRequest { model, input: inputs };
        let resp = self
            .http
            .post(url)
            .bearer_auth(&self.api_key)
            .json(&req)
            .send()
            .await?
            .error_for_status()?;
        let body: EmbedResponse = resp.json().await?;
        Ok(body.data.into_iter().map(|d| d.embedding).collect())
    }

    async fn chat_stream(
        &self,
        messages: Vec<LlmMessage>,
        options: ChatOptions,
    ) -> anyhow::Result<mpsc::Receiver<StreamEvent>> {
        let url = format!("{}/v1/chat/completions", self.base_url.trim_end_matches('/'));
        let req = ChatRequest {
            model: self.model.clone(),
            messages,
            temperature: options.temperature,
            tools: if options.tools { Some(default_tools()) } else { None },
            tool_choice: if options.tools { Some("auto".to_string()) } else { None },
            stream: Some(true),
        };
        let client = self.http.clone();
        let api_key = self.api_key.clone();
        let (tx, rx) = mpsc::channel(32);
        tokio::spawn(async move {
            let resp = client.post(&url).bearer_auth(api_key).json(&req).send().await;
            let resp = match resp {
                Ok(r) => r,
                Err(err) => {
                    let _ = tx.send(StreamEvent::Error(err.to_string())).await;
                    return;
                }
            };
            let resp = match resp.error_for_status() {
                Ok(r) => r,
                Err(err) => {
                    let _ = tx.send(StreamEvent::Error(err.to_string())).await;
                    return;
                }
            };
            let mut stream = resp.bytes_stream();
            let mut buffer = String::new();
            let mut content = String::new();
            let mut tool_calls: Vec<LlmToolCall> = Vec::new();
            while let Some(item) = stream.next().await {
                let chunk = match item {
                    Ok(bytes) => bytes,
                    Err(err) => {
                        let _ = tx.send(StreamEvent::Error(err.to_string())).await;
                        return;
                    }
                };
                let text = match std::str::from_utf8(&chunk) {
                    Ok(s) => s,
                    Err(err) => {
                        let _ = tx.send(StreamEvent::Error(err.to_string())).await;
                        return;
                    }
                };
                buffer.push_str(text);
                while let Some(idx) = buffer.find('\n') {
                    let mut line = buffer[..idx].to_string();
                    if line.ends_with('\r') {
                        line.pop();
                    }
                    buffer = buffer[idx + 1..].to_string();
                    let line = line.trim();
                    if line.is_empty() || line.starts_with(':') {
                        continue;
                    }
                    let Some(data) = line.strip_prefix("data:") else { continue };
                    let data = data.trim();
                    if data == "[DONE]" {
                        let _ = tx
                            .send(StreamEvent::Done(LlmResponse {
                                content,
                                tool_calls,
                            }))
                            .await;
                        return;
                    }
                    let parsed: StreamResponse = match serde_json::from_str(data) {
                        Ok(v) => v,
                        Err(err) => {
                            let _ = tx.send(StreamEvent::Error(err.to_string())).await;
                            return;
                        }
                    };
                    let choice = match parsed.choices.get(0) {
                        Some(c) => c,
                        None => continue,
                    };
                    if let Some(delta) = &choice.delta {
                        if let Some(delta_text) = &delta.content {
                            content.push_str(delta_text);
                            let _ = tx.send(StreamEvent::Delta(delta_text.clone())).await;
                        }
                        if let Some(delta_calls) = &delta.tool_calls {
                            for call in delta_calls {
                                let entry = ensure_tool_call(&mut tool_calls, call.index);
                                if let Some(id) = &call.id {
                                    if entry.id.is_empty() {
                                        entry.id = id.clone();
                                    }
                                }
                                if let Some(kind) = &call.kind {
                                    if entry.kind.is_empty() {
                                        entry.kind = kind.clone();
                                    }
                                }
                                if let Some(func) = &call.function {
                                    if let Some(name) = &func.name {
                                        if entry.function.name.is_empty() {
                                            entry.function.name = name.clone();
                                        }
                                    }
                                    if let Some(args) = &func.arguments {
                                        entry.function.arguments.push_str(args);
                                    }
                                }
                            }
                        }
                    }
                    if let Some(reason) = &choice.finish_reason {
                        if reason == "stop" || reason == "tool_calls" || reason == "length" {
                            let _ = tx
                                .send(StreamEvent::Done(LlmResponse {
                                    content,
                                    tool_calls,
                                }))
                                .await;
                            return;
                        }
                    }
                }
            }
            let _ = tx
                .send(StreamEvent::Done(LlmResponse {
                    content,
                    tool_calls,
                }))
                .await;
        });
        Ok(rx)
    }
}

fn ensure_tool_call(calls: &mut Vec<LlmToolCall>, index: usize) -> &mut LlmToolCall {
    while calls.len() <= index {
        calls.push(LlmToolCall {
            id: String::new(),
            kind: "function".to_string(),
            function: LlmToolFunction {
                name: String::new(),
                arguments: String::new(),
            },
        });
    }
    &mut calls[index]
}

fn default_tools() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            kind: "function".to_string(),
            function: ToolSpec {
                name: "shell".to_string(),
                description: "Run a shell command on the local machine. Use for local info (e.g., IP, time, files).".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "cmd": {"type": "string", "description": "Shell command to run"}
                    },
                    "required": ["cmd"]
                }),
            },
        },
        ToolDefinition {
            kind: "function".to_string(),
            function: ToolSpec {
                name: "http".to_string(),
                description: "Send an HTTP request.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "method": {"type": "string"},
                        "url": {"type": "string"},
                        "body": {"type": "string"}
                    },
                    "required": ["method", "url"]
                }),
            },
        },
        ToolDefinition {
            kind: "function".to_string(),
            function: ToolSpec {
                name: "search".to_string(),
                description: "Search the web (Tavily) and return top results.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "query": {"type": "string"},
                        "count": {"type": "integer"}
                    },
                    "required": ["query"]
                }),
            },
        },
        ToolDefinition {
            kind: "function".to_string(),
            function: ToolSpec {
                name: "tmux".to_string(),
                description: "Control tmux: start/stop/logs/list.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": {"type": "string", "description": "start <name> <cmd> | stop <name> | logs <name> [lines] | list"}
                    },
                    "required": ["action"]
                }),
            },
        },
    ]
}

fn should_retry_llm(err: &reqwest::Error) -> bool {
    if err.is_timeout() || err.is_connect() {
        return true;
    }
    match err.status() {
        Some(StatusCode::TOO_MANY_REQUESTS) => true,
        Some(status) if status.is_server_error() => true,
        _ => false,
    }
}

fn retry_delay(attempt: usize) -> Duration {
    let shift = (attempt.saturating_sub(1)).min(6) as u32;
    let backoff_ms = 200u64.saturating_mul(1u64 << shift);
    Duration::from_millis(backoff_ms.min(2000))
}
