use crate::memory::MemoryStore;
use serde::{Deserialize, Serialize};
use regex::Regex;
use std::collections::HashSet;
use std::sync::{Arc, RwLock};

pub mod http;
pub mod pdf;
pub mod search;
pub mod shell;
pub mod tmux;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToolCall {
    Shell { cmd: String },
    Http { method: String, url: String, body: Option<String> },
    Search { query: String, count: Option<usize> },
    Pdf { path: String, max_chars: Option<usize> },
    Tmux { action: tmux::TmuxAction },
}

#[derive(Debug, Clone)]
pub struct ToolOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("command not allowed")]
    NotAllowed,
    #[error("dangerous command rejected")]
    Dangerous,
    #[error("invalid tool input: {0}")]
    InvalidInput(String),
    #[error("tool execution failed: {0}")]
    Execution(String),
}

#[derive(Clone)]
pub struct DangerGuard {
    patterns: Vec<Regex>,
}

impl DangerGuard {
    pub fn new(patterns: &[String]) -> anyhow::Result<Self> {
        let mut compiled = Vec::new();
        for p in patterns {
            compiled.push(Regex::new(p)?);
        }
        Ok(Self { patterns: compiled })
    }

    pub fn check(&self, input: &str) -> Result<(), ToolError> {
        for p in &self.patterns {
            if p.is_match(input) {
                return Err(ToolError::Dangerous);
            }
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct ToolRegistry {
    pub shell_allowlist: Arc<RwLock<HashSet<String>>>,
    pub tmux_allowlist: Arc<RwLock<HashSet<String>>>,
    pub http_allowed_domains: Arc<RwLock<HashSet<String>>>,
    pub http_proxy: Option<String>,
    pub search_api_key: Option<String>,
    pub search_endpoint: Option<String>,
    pub search_limit: usize,
    pub shell_allow_all: bool,
    pub shell_allow_meta: bool,
    pub shell_use_shell: bool,
    pub tmux_allow_all: bool,
    pub http_allow_all: bool,
    pub guard: DangerGuard,
    pub memory: MemoryStore,
}

impl ToolRegistry {
    pub fn new(
        shell_allowlist: Vec<String>,
        tmux_allowlist: Vec<String>,
        http_allowed_domains: Vec<String>,
        http_proxy: Option<String>,
        search_api_key: Option<String>,
        search_endpoint: Option<String>,
        search_limit: usize,
        shell_allow_all: bool,
        shell_allow_meta: bool,
        shell_use_shell: bool,
        tmux_allow_all: bool,
        http_allow_all: bool,
        guard: DangerGuard,
        memory: MemoryStore,
    ) -> anyhow::Result<Self> {
        let mut shell: HashSet<String> = shell_allowlist.into_iter().collect();
        let mut tmux: HashSet<String> = tmux_allowlist.into_iter().collect();
        let http: HashSet<String> = http_allowed_domains.into_iter().collect();

        for cmd in memory.load_allowlist("shell")? {
            shell.insert(cmd);
        }
        for cmd in memory.load_allowlist("tmux")? {
            tmux.insert(cmd);
        }

        Ok(Self {
            shell_allowlist: Arc::new(RwLock::new(shell)),
            tmux_allowlist: Arc::new(RwLock::new(tmux)),
            http_allowed_domains: Arc::new(RwLock::new(http)),
            http_proxy,
            search_api_key,
            search_endpoint,
            search_limit,
            shell_allow_all,
            shell_allow_meta,
            shell_use_shell,
            tmux_allow_all,
            http_allow_all,
            guard,
            memory,
        })
    }

    pub fn extend_allowlist(&self, tool: &str, command: &str, added_by: i64) -> anyhow::Result<()> {
        match tool {
            "shell" => {
                self.shell_allowlist.write().unwrap().insert(command.to_string());
            }
            "tmux" => {
                self.tmux_allowlist.write().unwrap().insert(command.to_string());
            }
            _ => {}
        }
        self.memory.add_allowlist(tool, command, added_by)?;
        Ok(())
    }

    pub async fn execute(&self, call: ToolCall) -> Result<ToolOutput, ToolError> {
        match call {
            ToolCall::Shell { cmd } => shell::execute_shell(cmd, self).await,
            ToolCall::Http { method, url, body } => {
                http::execute_http(&method, &url, body, self).await
            }
            ToolCall::Search { query, count } => {
                search::execute_search(query, count, self).await
            }
            ToolCall::Pdf { path, max_chars } => pdf::extract_pdf_text(path, max_chars).await,
            ToolCall::Tmux { action } => tmux::execute_tmux(action, self).await,
        }
    }
}
