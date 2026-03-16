use crate::llm::{ChatOptions, LlmClient, LlmMessage};
use crate::memory::{local_day_string, MemoryStore};
use crate::tools::{ToolCall, ToolRegistry};
use chrono::{DateTime, Duration, TimeZone, Utc};
use chrono_tz::Tz;
use cron::Schedule;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::ParseMode;
use tokio::time::sleep;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ScheduledAction {
    Message { text: String },
    Tool { tool: ToolCall },
}

#[derive(Clone)]
pub struct Scheduler {
    bot: AutoSend<Bot>,
    memory: MemoryStore,
    tools: ToolRegistry,
    llm: Option<Arc<dyn LlmClient>>,
    sleep_time: String,
    timezone: Tz,
    heartbeat_interval_secs: u64,
}

impl Scheduler {
    pub fn new(
        bot: AutoSend<Bot>,
        memory: MemoryStore,
        tools: ToolRegistry,
        llm: Option<Arc<dyn LlmClient>>,
        sleep_time: String,
        timezone: Tz,
        heartbeat_interval_secs: u64,
    ) -> Self {
        Self {
            bot,
            memory,
            tools,
            llm,
            sleep_time,
            timezone,
            heartbeat_interval_secs,
        }
    }

    pub fn start(self: Arc<Self>) {
        let sleep_task = self.clone();
        tokio::spawn(async move {
            sleep_task.sleep_loop().await;
        });

        let heartbeat_task = self.clone();
        tokio::spawn(async move {
            heartbeat_task.heartbeat_loop().await;
        });

        let user_task = self.clone();
        tokio::spawn(async move {
            user_task.spawn_user_schedules().await;
        });
    }

    async fn heartbeat_loop(&self) {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(
            self.heartbeat_interval_secs.max(5),
        ));
        loop {
            interval.tick().await;
            let ts = self.memory.now_rfc3339();
            let _ = self.memory.write_heartbeat(&format!("heartbeat {}\n", ts));
            tracing::info!("heartbeat ok");
        }
    }

    async fn sleep_loop(&self) {
        loop {
            let next = next_sleep_time(&self.sleep_time, self.timezone);
            let now = Utc::now();
            let wait = next.signed_duration_since(now);
            if let Ok(dur) = wait.to_std() {
                sleep(dur).await;
            } else {
                sleep(std::time::Duration::from_secs(60)).await;
            }
            if let Err(err) = self.run_sleep().await {
                tracing::error!("sleep task failed: {}", err);
            }
        }
    }

    async fn run_sleep(&self) -> anyhow::Result<()> {
        let day = local_day_string(chrono::Local::now());
        let chat_ids = self.memory.list_chats_for_day(&day)?;
        for chat_id in chat_ids {
            let date = self.memory.parse_date(&day)?;
            let log = self.memory.read_daily_log(chat_id, date)?;
            if log.trim().is_empty() {
                continue;
            }
            let result = if let Some(llm) = &self.llm {
                sleep_compact_llm(llm.clone(), &log).await.unwrap_or_else(|_| sleep_compact_simple(&log))
            } else {
                sleep_compact_simple(&log)
            };
            for item in &result.retain {
                self.memory.append_long_memory_file(chat_id, item)?;
                self.memory.add_long_memory(chat_id, item, "sleep")?;
            }
            let summary = if result.retain.is_empty() {
                "(no retain items)".to_string()
            } else {
                result.retain.join("; ")
            };
            self.memory.set_summary(chat_id, &day, &summary)?;
            let archive = format!("# Sleep Archive {}\n\n{}\n", day, result.archive.trim());
            self.memory.write_sleep_archive(chat_id, date, &archive)?;
        }
        Ok(())
    }

    async fn spawn_user_schedules(&self) {
        let schedules = match self.memory.list_schedules() {
            Ok(list) => list,
            Err(err) => {
                tracing::error!("failed to load schedules: {}", err);
                return;
            }
        };
        for (id, chat_id, cron, action_type, action_payload) in schedules {
            if let Some(run_at) = parse_once_marker(&cron) {
                let action = match parse_action(&action_type, &action_payload) {
                    Ok(a) => a,
                    Err(err) => {
                        tracing::error!("invalid schedule action: {}", err);
                        continue;
                    }
                };
                let bot = self.bot.clone();
                let tools = self.tools.clone();
                let memory = self.memory.clone();
                tokio::spawn(async move {
                    run_once(bot, tools, memory, chat_id, id, run_at, action).await;
                });
                continue;
            }
            let action = match parse_action(&action_type, &action_payload) {
                Ok(a) => a,
                Err(err) => {
                    tracing::error!("invalid schedule action: {}", err);
                    continue;
                }
            };
            let (_, raw) = match split_cron_prefix(&cron) {
                Ok(v) => v,
                Err(err) => {
                    tracing::error!("invalid cron prefix: {}", err);
                    continue;
                }
            };
            let cron_norm = normalize_cron(&raw);
            let bot = self.bot.clone();
            let tools = self.tools.clone();
            tokio::spawn(async move {
                if let Err(err) = run_cron_loop(&cron_norm, move || {
                    let bot = bot.clone();
                    let tools = tools.clone();
                    let action = action.clone();
                    async move {
                        execute_action(bot, tools, chat_id, action).await;
                    }
                })
                .await
                {
                    tracing::error!("cron loop error: {}", err);
                }
            });
        }
    }

    pub async fn add_schedule(
        &self,
        chat_id: i64,
        cron: &str,
        action: ScheduledAction,
    ) -> anyhow::Result<i64> {
        let (scope, raw) = split_cron_prefix(cron)?;
        let cron_norm = normalize_cron(&raw);
        let cron_store = format!("{}{}", cron_prefix(scope), cron_norm);
        // Spawn immediately for runtime schedules.
        let bot = self.bot.clone();
        let tools = self.tools.clone();
        let action_clone = action.clone();
        let cron_expr = cron_norm.clone();
        tokio::spawn(async move {
            if let Err(err) = run_cron_loop(&cron_expr, move || {
                let bot = bot.clone();
                let tools = tools.clone();
                let action = action_clone.clone();
                async move {
                    execute_action(bot, tools, chat_id, action).await;
                }
            })
            .await
            {
                tracing::error!("cron loop error: {}", err);
            }
        });

        let action_type = match action {
            ScheduledAction::Message { .. } => "message",
            ScheduledAction::Tool { .. } => "tool",
        };
        let payload = serde_json::to_string(&action)?;
        self.memory.add_schedule(chat_id, &cron_store, action_type, &payload)
    }

    pub async fn add_once(
        &self,
        chat_id: i64,
        run_at: DateTime<Utc>,
        action: ScheduledAction,
    ) -> anyhow::Result<i64> {
        let cron_store = format!("once@{}", run_at.to_rfc3339());
        let action_type = match action {
            ScheduledAction::Message { .. } => "message",
            ScheduledAction::Tool { .. } => "tool",
        };
        let payload = serde_json::to_string(&action)?;
        let id = self.memory.add_schedule(chat_id, &cron_store, action_type, &payload)?;

        let bot = self.bot.clone();
        let tools = self.tools.clone();
        let memory = self.memory.clone();
        tokio::spawn(async move {
            run_once(bot, tools, memory, chat_id, id, run_at, action).await;
        });
        Ok(id)
    }
}

#[derive(Debug, Clone)]
struct SleepResult {
    retain: Vec<String>,
    archive: String,
}

async fn sleep_compact_llm(
    llm: Arc<dyn LlmClient>,
    log: &str,
) -> anyhow::Result<SleepResult> {
    let system = r#"Condense daily log into JSON {"retain":[],"archive":""}. Retain durable facts/preferences/decisions only."#;
    let messages = vec![
        LlmMessage {
            role: "system".to_string(),
            content: system.to_string(),
            tool_call_id: None,
            tool_calls: None,
        },
        LlmMessage {
            role: "user".to_string(),
            content: log.to_string(),
            tool_call_id: None,
            tool_calls: None,
        },
    ];
    let raw = llm
        .chat(
            messages,
            ChatOptions {
                temperature: 0.1,
                tools: false,
            },
        )
        .await?
        .content;
    let value: serde_json::Value = serde_json::from_str(&raw)?;
    let retain = value
        .get("retain")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let archive = value
        .get("archive")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    Ok(SleepResult { retain, archive })
}

fn sleep_compact_simple(log: &str) -> SleepResult {
    let mut retain = Vec::new();
    let mut archive_lines = Vec::new();
    for line in log.lines() {
        let lower = line.to_lowercase();
        if lower.contains("todo")
            || lower.contains("important")
            || lower.contains("decision")
            || lower.contains("preference")
            || lower.contains("like")
            || lower.contains("dislike")
        {
            retain.push(line.trim().to_string());
        } else if archive_lines.len() < 200 {
            archive_lines.push(line.trim().to_string());
        }
    }
    let archive = archive_lines.join("\n");
    SleepResult { retain, archive }
}

fn next_sleep_time(time_str: &str, tz: Tz) -> DateTime<Utc> {
    let now = Utc::now().with_timezone(&tz);
    let parts: Vec<&str> = time_str.split(':').collect();
    let hour: u32 = parts.get(0).and_then(|s| s.parse().ok()).unwrap_or(2);
    let minute: u32 = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(30);
    let today = now.date_naive();
    let mut target = tz
        .from_local_datetime(&today.and_hms_opt(hour, minute, 0).unwrap())
        .single()
        .unwrap_or_else(|| tz.from_local_datetime(&today.and_hms_opt(2, 30, 0).unwrap()).unwrap());
    if target <= now {
        target = target + Duration::days(1);
    }
    target.with_timezone(&Utc)
}

fn parse_action(action_type: &str, payload: &str) -> anyhow::Result<ScheduledAction> {
    if action_type == "message" || action_type == "tool" {
        let action: ScheduledAction = serde_json::from_str(payload)?;
        return Ok(action);
    }
    anyhow::bail!("unknown action")
}

#[derive(Debug, Clone, Copy)]
enum CronScope {
    User,
    System,
}

fn split_cron_prefix(expr: &str) -> anyhow::Result<(CronScope, String)> {
    let trimmed = expr.trim();
    if let Some(rest) = trimmed.strip_prefix("rbot_system_") {
        return Ok((CronScope::System, rest.trim().to_string()));
    }
    if let Some(rest) = trimmed.strip_prefix("rbot_") {
        return Ok((CronScope::User, rest.trim().to_string()));
    }
    anyhow::bail!("cron must start with rbot_ or rbot_system_");
}

fn cron_prefix(scope: CronScope) -> &'static str {
    match scope {
        CronScope::User => "rbot_",
        CronScope::System => "rbot_system_",
    }
}

fn normalize_cron(expr: &str) -> String {
    let parts: Vec<&str> = expr.split_whitespace().collect();
    if parts.len() == 5 {
        format!("0 {}", expr)
    } else {
        expr.to_string()
    }
}

fn parse_once_marker(expr: &str) -> Option<DateTime<Utc>> {
    let trimmed = expr.trim();
    let rest = trimmed.strip_prefix("once@")?;
    DateTime::parse_from_rfc3339(rest)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

async fn run_cron_loop<F, Fut>(cron_expr: &str, mut job: F) -> anyhow::Result<()>
where
    F: FnMut() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send,
{
    let schedule = Schedule::from_str(cron_expr)?;
    let mut upcoming = schedule.upcoming(Utc);
    loop {
        let next = match upcoming.next() {
            Some(n) => n,
            None => break,
        };
        let now = Utc::now();
        let wait = next.signed_duration_since(now);
        if let Ok(dur) = wait.to_std() {
            sleep(dur).await;
        } else {
            sleep(std::time::Duration::from_secs(1)).await;
        }
        job().await;
    }
    Ok(())
}

async fn execute_action(bot: AutoSend<Bot>, tools: ToolRegistry, chat_id: i64, action: ScheduledAction) {
    match action {
        ScheduledAction::Message { text } => {
            let send = bot
                .send_message(ChatId(chat_id), text.clone())
                .parse_mode(ParseMode::Html)
                .await;
            if send.is_err() {
                let safe = escape_html(&text);
                let _ = bot
                    .send_message(ChatId(chat_id), safe)
                    .parse_mode(ParseMode::Html)
                    .await;
            }
        }
        ScheduledAction::Tool { tool } => {
            match tools.execute(tool).await {
                Ok(out) => {
                    let text = format_tool_output_html(&out);
                    let _ = bot
                        .send_message(ChatId(chat_id), text)
                        .parse_mode(ParseMode::Html)
                        .await;
                }
                Err(err) => {
                    let _ = bot
                        .send_message(
                            ChatId(chat_id),
                            escape_html(&format!("tool error: {}", err)),
                        )
                        .parse_mode(ParseMode::Html)
                        .await;
                }
            }
        }
    }
}

async fn run_once(
    bot: AutoSend<Bot>,
    tools: ToolRegistry,
    memory: MemoryStore,
    chat_id: i64,
    schedule_id: i64,
    run_at: DateTime<Utc>,
    action: ScheduledAction,
) {
    let now = Utc::now();
    if run_at > now {
        let wait = run_at.signed_duration_since(now);
        if let Ok(dur) = wait.to_std() {
            sleep(dur).await;
        }
    }
    execute_action(bot, tools, chat_id, action).await;
    let _ = memory.disable_schedule(schedule_id);
}

fn format_tool_output_html(output: &crate::tools::ToolOutput) -> String {
    let mut parts = Vec::new();
    if !output.stdout.trim().is_empty() {
        parts.push(format!(
            "<b>stdout</b>\n<pre>{}</pre>",
            escape_html(&output.stdout)
        ));
    } else {
        parts.push("<b>stdout</b>\n<pre>(empty)</pre>".to_string());
    }
    if !output.stderr.trim().is_empty() {
        parts.push(format!(
            "<b>stderr</b>\n<pre>{}</pre>",
            escape_html(&output.stderr)
        ));
    }
    parts.push(format!("<b>exit</b> {}", output.exit_code));
    parts.join("\n")
}

fn escape_html(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
    out
}
