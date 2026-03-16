use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub telegram: TelegramConfig,
    pub llm: LlmConfig,
    pub memory: MemoryConfig,
    pub tools: ToolsConfig,
    pub security: SecurityConfig,
    pub scheduler: SchedulerConfig,
    pub skills: SkillsConfig,
    #[serde(default)]
    pub network: NetworkConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramConfig {
    pub token: String,
    pub admin_user_ids: Vec<i64>,
    #[serde(default)]
    pub admin_chat_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NetworkConfig {
    pub proxy_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub embed_model: Option<String>,
    pub request_timeout_secs: u64,
    #[serde(default)]
    pub overall_timeout_secs: Option<u64>,
    #[serde(default = "default_max_tool_calls")]
    pub max_tool_calls: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConfig {
    pub db_path: String,
    pub base_dir: String,
    pub short_term_limit: usize,
    pub sleep_time: String,
    pub timezone: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolsConfig {
    pub shell: ShellToolConfig,
    pub tmux: TmuxToolConfig,
    pub http: HttpToolConfig,
    #[serde(default)]
    pub search: SearchToolConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellToolConfig {
    pub allowlist: Vec<String>,
    #[serde(default)]
    pub blocklist: Vec<String>,
    #[serde(default = "default_shell_mode")]
    pub mode: String,
    #[serde(default)]
    pub allow_all: bool,
    #[serde(default)]
    pub allow_meta: bool,
    #[serde(default)]
    pub use_shell: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TmuxToolConfig {
    pub allowlist: Vec<String>,
    #[serde(default)]
    pub allow_all: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpToolConfig {
    pub allowed_domains: Vec<String>,
    #[serde(default)]
    pub allow_all: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SearchToolConfig {
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default = "default_search_limit")]
    pub limit: usize,
}

fn default_search_limit() -> usize {
    5
}

fn default_max_tool_calls() -> usize {
    16
}

fn default_shell_mode() -> String {
    "allowlist".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityConfig {
    pub danger_patterns: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchedulerConfig {
    pub heartbeat_interval_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillsConfig {
    pub dir: String,
}

impl Config {
    pub fn load(path: Option<PathBuf>) -> anyhow::Result<Self> {
        let path = path.unwrap_or_else(|| {
            std::env::var("RBOT_CONFIG")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("config/config.toml"))
        });
        let text = fs::read_to_string(&path)
            .map_err(|e| anyhow::anyhow!("failed to read config at {:?}: {}", path, e))?;
        let cfg: Config = toml::from_str(&text)
            .map_err(|e| anyhow::anyhow!("failed to parse config {:?}: {}", path, e))?;
        Ok(cfg)
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let text = toml::to_string_pretty(self)?;
        fs::write(path, text)?;
        Ok(())
    }

    pub fn resolve_paths(&mut self, base: &Path) {
        if !Path::new(&self.memory.db_path).is_absolute() {
            self.memory.db_path = base.join(&self.memory.db_path).to_string_lossy().to_string();
        }
        if !Path::new(&self.memory.base_dir).is_absolute() {
            self.memory.base_dir = base.join(&self.memory.base_dir).to_string_lossy().to_string();
        }
        if !Path::new(&self.skills.dir).is_absolute() {
            self.skills.dir = base.join(&self.skills.dir).to_string_lossy().to_string();
        }
    }
}
