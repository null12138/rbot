use crate::config::{Config, HttpToolConfig, LlmConfig, MemoryConfig, NetworkConfig, SchedulerConfig, SecurityConfig, ShellToolConfig, SkillsConfig, TelegramConfig, ToolsConfig, TmuxToolConfig};
use std::io::{self, Write};
use std::path::Path;

pub fn run() -> anyhow::Result<()> {
    let token = prompt("Telegram token", None)?;
    let admin_user_ids = prompt("Admin user IDs (comma separated)", Some("123456789".into()))?;

    let base_url = prompt("LLM base_url", Some("https://api.openai.com".into()))?;
    let api_key = prompt("LLM api_key", Some("".into()))?;
    let model = prompt("LLM model", Some("gpt-4o-mini".into()))?;
    let embed_model = prompt("Embedding model (optional)", Some("text-embedding-3-small".into()))?;

    let sleep_time = prompt("Sleep time (HH:MM)", Some("02:30".into()))?;
    let timezone = prompt("Timezone", Some("Asia/Shanghai".into()))?;
    let heartbeat = prompt("Heartbeat interval secs", Some("60".into()))?;

    let allowlist_shell = vec!["ls", "rg", "git", "cargo", "cat", "pwd", "whoami"]
        .into_iter()
        .map(|s| s.to_string())
        .collect();
    let allowlist_tmux = vec![
        "new-session",
        "list-sessions",
        "kill-session",
        "capture-pane",
        "send-keys",
    ]
    .into_iter()
    .map(|s| s.to_string())
    .collect();
    let allowed_domains = vec!["api.github.com", "api.openai.com"]
        .into_iter()
        .map(|s| s.to_string())
        .collect();

    let danger_patterns = vec![
        "rm\\s+-rf",
        "mkfs",
        "dd\\s+if=",
        ":\\(\\)\\{\\s*:\\|:\\s*&\\};:",
        "shutdown",
        "reboot",
    ]
    .into_iter()
    .map(|s| s.to_string())
    .collect();

    let cfg = Config {
        telegram: TelegramConfig {
            token,
            admin_user_ids: parse_id_list(&admin_user_ids),
            admin_chat_id: None,
        },
        llm: LlmConfig {
            base_url,
            api_key,
            model,
            embed_model: if embed_model.trim().is_empty() {
                None
            } else {
                Some(embed_model)
            },
            request_timeout_secs: 60,
        },
        memory: MemoryConfig {
            db_path: "data/agent.db".into(),
            base_dir: "memory".into(),
            short_term_limit: 8,
            sleep_time,
            timezone,
        },
        tools: ToolsConfig {
            shell: ShellToolConfig {
                allowlist: allowlist_shell,
                allow_all: false,
                allow_meta: false,
                use_shell: false,
            },
            tmux: TmuxToolConfig {
                allowlist: allowlist_tmux,
                allow_all: false,
            },
            http: HttpToolConfig {
                allowed_domains,
                allow_all: false,
            },
        },
        security: SecurityConfig { danger_patterns },
        scheduler: SchedulerConfig {
            heartbeat_interval_secs: heartbeat.parse().unwrap_or(60),
        },
        skills: SkillsConfig { dir: "skills".into() },
        network: NetworkConfig::default(),
    };

    let path = Path::new("config/config.toml");
    cfg.save(path)?;
    println!("Wrote config to {}", path.display());
    Ok(())
}

fn prompt(label: &str, default: Option<String>) -> anyhow::Result<String> {
    let mut stdout = io::stdout();
    if let Some(d) = &default {
        write!(stdout, "{} [{}]: ", label, d)?;
    } else {
        write!(stdout, "{}: ", label)?;
    }
    stdout.flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let trimmed = input.trim();
    if trimmed.is_empty() {
        Ok(default.unwrap_or_default())
    } else {
        Ok(trimmed.to_string())
    }
}

fn parse_id_list(input: &str) -> Vec<i64> {
    input
        .split(',')
        .filter_map(|s| s.trim().parse::<i64>().ok())
        .collect()
}
