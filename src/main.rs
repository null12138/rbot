mod config;
mod init;
mod llm;
mod memory;
mod scheduler;
mod skills;
mod telegram;
mod tools;

use crate::config::Config;
use crate::llm::{LlmClient, OpenAIClient};
use crate::memory::MemoryStore;
use crate::scheduler::Scheduler;
use crate::skills::SkillManager;
use crate::tools::{DangerGuard, ToolRegistry};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;
use teloxide::net;
use teloxide::prelude::*;
use teloxide::RequestError;
use std::fs;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .init();

    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(|s| s == "init").unwrap_or(false) {
        return init::run();
    }

    let mut cfg = Config::load(None)?;
    cfg.resolve_paths(Path::new("."));

    let proxy_url = cfg
        .network
        .proxy_url
        .clone()
        .or_else(|| std::env::var("RBOT_PROXY").ok());

    let (bot, use_proxy) = build_bot_with_fallback(&cfg.telegram.token, proxy_url.as_deref()).await?;
    let proxy_for_clients = if use_proxy { proxy_url.clone() } else { None };

    let persona = load_persona("config/persona.md");
    let memory = MemoryStore::new(&cfg.memory.db_path, &cfg.memory.base_dir)?;
    let guard = DangerGuard::new(&cfg.security.danger_patterns)?;
    let tools = ToolRegistry::new(
        cfg.tools.shell.allowlist.clone(),
        cfg.tools.shell.blocklist.clone(),
        crate::tools::ShellMode::parse(&cfg.tools.shell.mode),
        cfg.tools.tmux.allowlist.clone(),
        cfg.tools.http.allowed_domains.clone(),
        proxy_for_clients.clone(),
        cfg.tools.search.api_key.clone(),
        cfg.tools.search.endpoint.clone(),
        cfg.tools.search.limit,
        cfg.tools.shell.allow_all,
        cfg.tools.shell.allow_meta,
        cfg.tools.shell.use_shell,
        cfg.tools.tmux.allow_all,
        cfg.tools.http.allow_all,
        guard,
        memory.clone(),
    )?;

    let llm: Option<Arc<dyn LlmClient>> = if cfg.llm.api_key.trim().is_empty() {
        None
    } else {
        let client = OpenAIClient::new(
            cfg.llm.base_url.clone(),
            cfg.llm.api_key.clone(),
            cfg.llm.model.clone(),
            cfg.llm.embed_model.clone(),
            cfg.llm.request_timeout_secs,
            proxy_for_clients.clone(),
        )?;
        Some(Arc::new(client))
    };

    let skills = SkillManager::load(&cfg.skills.dir)?;
    let tz: chrono_tz::Tz = cfg
        .memory
        .timezone
        .parse()
        .unwrap_or(chrono_tz::Asia::Shanghai);

    let scheduler = Arc::new(Scheduler::new(
        bot.clone(),
        memory.clone(),
        tools.clone(),
        llm.clone(),
        cfg.memory.sleep_time.clone(),
        tz,
        cfg.scheduler.heartbeat_interval_secs,
    ));
    scheduler.clone().start();

    let pending_tool_limit = Arc::new(Mutex::new(HashMap::new()));

    let state = telegram::AppState {
        cfg,
        memory,
        tools,
        scheduler,
        skills,
        llm,
        persona,
        pending_tool_limit,
    };

    telegram::run_bot(bot, state).await;
    Ok(())
}

async fn build_bot_with_fallback(
    token: &str,
    proxy_url: Option<&str>,
) -> anyhow::Result<(AutoSend<Bot>, bool)> {
    let bot = Bot::with_client(token.to_string(), build_bot_client(None)?).auto_send();
    match bot.get_me().await {
        Ok(_) => Ok((bot, false)),
        Err(err) => {
            if let Some(proxy) = proxy_url {
                if is_network_error(&err) {
                    tracing::warn!("telegram access failed without proxy, retrying with proxy: {}", err);
                    let bot_proxy = Bot::with_client(token.to_string(), build_bot_client(Some(proxy))?).auto_send();
                    bot_proxy
                        .get_me()
                        .await
                        .map_err(|e| anyhow::anyhow!("telegram access failed with proxy: {}", e))?;
                    return Ok((bot_proxy, true));
                }
            }
            Err(anyhow::anyhow!("telegram access failed: {}", err))
        }
    }
}

fn build_bot_client(proxy_url: Option<&str>) -> anyhow::Result<reqwest::Client> {
    let mut builder = net::default_reqwest_settings().no_proxy();
    if let Some(proxy) = proxy_url.map(|s| s.trim()).filter(|s| !s.is_empty()) {
        builder = builder.proxy(reqwest::Proxy::all(proxy)?);
    }
    Ok(builder
        .build()
        .map_err(|e| anyhow::anyhow!("failed to build telegram client: {}", e))?)
}

fn is_network_error(err: &RequestError) -> bool {
    matches!(err, RequestError::Network(_))
}

fn load_persona(path: &str) -> String {
    let path = Path::new(path);
    if path.exists() {
        if let Ok(text) = fs::read_to_string(path) {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
    }
    "You are a concise, pragmatic assistant running in the local environment. You can execute tools (shell/http/tmux) proactively for safe tasks. Track multi-turn goals, use memory, ask only when needed.".to_string()
}
