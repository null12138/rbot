use crate::config::{
    Config, HttpToolConfig, LlmConfig, MemoryConfig, NetworkConfig, SchedulerConfig,
    SearchToolConfig, SecurityConfig, ShellToolConfig, SkillsConfig, TelegramConfig, ToolsConfig,
    TmuxToolConfig,
};
use dialoguer::{Confirm, Input, Password};
use std::io::{self, Write};
use std::path::Path;

pub fn run() -> anyhow::Result<()> {
    let prompter = Prompter::new();

    let token = prompter.prompt_secret("Telegram token", None, false)?;
    let admin_user_ids =
        prompter.prompt("Admin user IDs (comma separated)", Some("123456789"), false)?;

    let base_url = prompter.prompt("LLM base_url", Some("https://api.openai.com"), false)?;
    let api_key = prompter.prompt_secret("LLM api_key", Some(""), true)?;
    let model = prompter.prompt("LLM model", Some("gpt-4o-mini"), false)?;
    let use_defaults = prompter.confirm("Use defaults for advanced settings?", true)?;

    let (embed_model, sleep_time, timezone, heartbeat, search_api_key, search_endpoint, search_limit) =
        if use_defaults {
            (
                "text-embedding-3-small".to_string(),
                "02:30".to_string(),
                "Asia/Shanghai".to_string(),
                "60".to_string(),
                "".to_string(),
                "".to_string(),
                "5".to_string(),
            )
        } else {
            let embed_model =
                prompter.prompt("Embedding model (optional)", Some("text-embedding-3-small"), true)?;
            let sleep_time = prompter.prompt("Sleep time (HH:MM)", Some("02:30"), false)?;
            let timezone = prompter.prompt("Timezone", Some("Asia/Shanghai"), false)?;
            let heartbeat = prompter.prompt("Heartbeat interval secs", Some("60"), false)?;
            let search_api_key = prompter.prompt("Tavily api_key (optional)", Some(""), true)?;
            let search_endpoint = prompter.prompt("Tavily endpoint (optional)", Some(""), true)?;
            let search_limit = prompter.prompt("Search result limit", Some("5"), false)?;
            (
                embed_model,
                sleep_time,
                timezone,
                heartbeat,
                search_api_key,
                search_endpoint,
                search_limit,
            )
        };

    let allowlist_shell = vec!["ls", "rg", "git", "cargo", "cat", "pwd", "whoami"]
        .into_iter()
        .map(|s| s.to_string())
        .collect();
    let blocklist_shell = vec!["rm", "sudo", "shutdown", "reboot", "mkfs", "dd"]
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
            overall_timeout_secs: Some(600),
            max_tool_calls: 16,
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
                blocklist: blocklist_shell,
                mode: "blocklist".into(),
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
                allow_all: true,
            },
            search: SearchToolConfig {
                api_key: if search_api_key.trim().is_empty() {
                    None
                } else {
                    Some(search_api_key)
                },
                endpoint: if search_endpoint.trim().is_empty() {
                    None
                } else {
                    Some(search_endpoint)
                },
                limit: search_limit.parse().unwrap_or(5),
            },
        },
        security: SecurityConfig { danger_patterns },
        scheduler: SchedulerConfig {
            heartbeat_interval_secs: heartbeat.parse().unwrap_or(60),
        },
        skills: SkillsConfig { dir: "skills".into() },
        network: NetworkConfig::default(),
    };

    let path = Path::new("config/config.local.toml");
    cfg.save(path)?;
    println!("Wrote config to {}", path.display());
    Ok(())
}

struct Prompter {
    use_tui: bool,
}

impl Prompter {
    fn new() -> Self {
        let use_tui = atty::is(atty::Stream::Stdin) && atty::is(atty::Stream::Stdout);
        Self { use_tui }
    }

    fn prompt(&self, label: &str, default: Option<&str>, allow_empty: bool) -> anyhow::Result<String> {
        if self.use_tui {
            prompt_tui(label, default, allow_empty)
        } else {
            prompt_plain(label, default, allow_empty)
        }
    }

    fn prompt_secret(
        &self,
        label: &str,
        default: Option<&str>,
        allow_empty: bool,
    ) -> anyhow::Result<String> {
        if self.use_tui {
            prompt_secret_tui(label, default, allow_empty)
        } else {
            prompt_plain(label, default, allow_empty)
        }
    }

    fn confirm(&self, label: &str, default: bool) -> anyhow::Result<bool> {
        if self.use_tui {
            let value = Confirm::new()
                .with_prompt(label)
                .default(default)
                .interact()?;
            Ok(value)
        } else {
            Ok(default)
        }
    }
}

fn prompt_tui(label: &str, default: Option<&str>, allow_empty: bool) -> anyhow::Result<String> {
    let mut input = Input::<String>::new().with_prompt(label);
    if let Some(d) = default {
        input = input.default(d.to_string());
    }
    if allow_empty {
        input = input.allow_empty(true);
    }
    let value = input.interact_text()?;
    Ok(value.trim().to_string())
}

fn prompt_secret_tui(label: &str, default: Option<&str>, allow_empty: bool) -> anyhow::Result<String> {
    let mut input = Password::new().with_prompt(label);
    if allow_empty {
        input = input.allow_empty_password(true);
    }
    let value = input.interact()?;
    if value.trim().is_empty() {
        if let Some(d) = default {
            return Ok(d.to_string());
        }
        if allow_empty {
            return Ok(String::new());
        }
    }
    Ok(value.trim().to_string())
}

fn prompt_plain(label: &str, default: Option<&str>, allow_empty: bool) -> anyhow::Result<String> {
    let mut stdout = io::stdout();
    if let Some(d) = default {
        write!(stdout, "{} [{}]: ", label, d)?;
    } else {
        write!(stdout, "{}: ", label)?;
    }
    stdout.flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let trimmed = input.trim();
    if trimmed.is_empty() {
        if let Some(d) = default {
            return Ok(d.to_string());
        }
        if allow_empty {
            return Ok(String::new());
        }
    }
    Ok(trimmed.to_string())
}

fn parse_id_list(input: &str) -> Vec<i64> {
    input
        .split(',')
        .filter_map(|s| s.trim().parse::<i64>().ok())
        .collect()
}
