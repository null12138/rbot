use crate::config::Config;
use crate::llm::{ChatOptions, LlmClient, LlmMessage, LlmToolCall, StreamEvent, LlmResponse};
use serde::{Deserialize, Serialize};
use crate::memory::{local_day_string, MemoryStore, StoredMessage};
use crate::scheduler::{ScheduledAction, Scheduler};
use crate::skills::SkillManager;
use crate::tools::{tmux::TmuxAction, ToolCall, ToolError, ToolRegistry};
use chrono::Local;
use std::collections::HashMap;
use std::time::Instant;
use std::sync::Arc;
use tracing::{info, warn};
use uuid::Uuid;
use teloxide::dispatching::dialogue::{Dialogue, InMemStorage};
use teloxide::prelude::*;
use teloxide::types::{CallbackQuery, ChatAction, InlineKeyboardButton, InlineKeyboardMarkup, MessageId, ParseMode};
use reqwest::{Client, Proxy};
use tokio::sync::{oneshot, Mutex};
use tokio::time::{self, Duration};

#[derive(Clone)]
pub struct AppState {
    pub cfg: Config,
    pub memory: MemoryStore,
    pub tools: ToolRegistry,
    pub scheduler: Arc<Scheduler>,
    pub skills: SkillManager,
    pub llm: Option<Arc<dyn LlmClient>>,
    pub persona: String,
    pub pending_tool_limit: PendingToolLimitMap,
}

#[derive(Clone, Debug)]
pub struct PendingToolLimit {
    pub input: String,
    pub max_tool_calls: usize,
}

pub type PendingToolLimitMap = Arc<Mutex<HashMap<i64, PendingToolLimit>>>;

#[derive(Debug)]
struct ProgressHandle {
    stop: oneshot::Sender<()>,
    join: tokio::task::JoinHandle<()>,
    message_id: MessageId,
}

#[derive(Debug)]
enum ChatResult {
    Reply(String),
    ToolLimit { max: usize, suggested: usize },
}

const CALLBACK_YES: &str = "rbot_yes";
const CALLBACK_NO: &str = "rbot_no";

#[derive(Clone)]
struct StreamContext {
    bot: AutoSend<Bot>,
    chat_id: ChatId,
    message_id: Option<MessageId>,
    progress: Arc<Mutex<Option<ProgressHandle>>>,
}

struct StreamEditor {
    bot: AutoSend<Bot>,
    chat_id: ChatId,
    message_id: MessageId,
    created_at: Instant,
    last_edit: Instant,
    last_len: usize,
    min_interval: Duration,
    min_chars: usize,
    last_typing: Instant,
    progress: Arc<Mutex<Option<ProgressHandle>>>,
    stopped: bool,
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum DialogueState {
    #[default]
    Idle,
    AwaitingCommand,
    AwaitingHttp,
    AwaitingTmux,
    AwaitingSchedule,
    AwaitingWhitelist,
}

type MyDialogue = Dialogue<DialogueState, InMemStorage<DialogueState>>;
type HandlerResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

pub async fn run_bot(bot: AutoSend<Bot>, state: AppState) {
    let storage = InMemStorage::<DialogueState>::new();
    let handler = dptree::entry()
        .branch(
            Update::filter_message()
                .enter_dialogue::<Message, InMemStorage<DialogueState>, DialogueState>()
                .branch(dptree::case![DialogueState::Idle].endpoint(handle_idle))
                .branch(dptree::case![DialogueState::AwaitingCommand].endpoint(handle_command))
                .branch(dptree::case![DialogueState::AwaitingHttp].endpoint(handle_http))
                .branch(dptree::case![DialogueState::AwaitingTmux].endpoint(handle_tmux))
                .branch(dptree::case![DialogueState::AwaitingSchedule].endpoint(handle_schedule))
                .branch(dptree::case![DialogueState::AwaitingWhitelist].endpoint(handle_whitelist)),
        )
        .branch(Update::filter_callback_query().endpoint(handle_callback));

    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![state, storage])
        .build()
        .dispatch()
        .await;
}

async fn handle_idle(bot: AutoSend<Bot>, msg: Message, dialogue: MyDialogue, state: AppState) -> HandlerResult {
    let text = match msg.text() {
        Some(t) => t.trim(),
        None => return Ok(()),
    };
    let chat_id = msg.chat.id;
    let chat_id_i64 = msg.chat.id.0;

    let _ = try_like_message(&state, &msg).await;

    let pending = {
        let map = state.pending_tool_limit.lock().await;
        map.get(&chat_id_i64).cloned()
    };
    if let Some(pending) = pending {
        if is_confirm(text) {
            {
                let mut map = state.pending_tool_limit.lock().await;
                map.remove(&chat_id_i64);
            }
            let progress = start_progress(&bot, chat_id).await;
            let message_id = progress.as_ref().map(|p| p.message_id);
            let progress_state = Arc::new(Mutex::new(progress));
            let stream_ctx = StreamContext {
                bot: bot.clone(),
                chat_id,
                message_id,
                progress: progress_state.clone(),
            };
            let response = chat_with_timeout(
                &state,
                chat_id_i64,
                &pending.input,
                Some(pending.max_tool_calls),
                Some(stream_ctx),
            )
            .await;
            match response {
                Ok(ChatResult::Reply(reply)) => {
                    state.memory.append_message(chat_id_i64, "assistant", &reply)?;
                    let progress_handle = progress_state.lock().await.take();
                    send_reply_with_progress(&bot, chat_id, &reply, progress_handle, message_id, false).await?;
                }
                Ok(ChatResult::ToolLimit { max, suggested }) => {
                    {
                        let mut map = state.pending_tool_limit.lock().await;
                        map.insert(
                            chat_id_i64,
                            PendingToolLimit {
                                input: pending.input,
                                max_tool_calls: suggested,
                            },
                        );
                    }
                    let prompt = format!(
                        "工具调用已达上限 {}。回复“继续”可临时提高到 {} 并继续本次请求。",
                        max, suggested
                    );
                    let progress_handle = progress_state.lock().await.take();
                    send_reply_with_progress(&bot, chat_id, &prompt, progress_handle, message_id, false).await?;
                }
                Err(err) => {
                    let reply = format!("llm error: {}", err);
                    state.memory.append_message(chat_id_i64, "assistant", &reply)?;
                    let progress_handle = progress_state.lock().await.take();
                    send_reply_with_progress(&bot, chat_id, &reply, progress_handle, message_id, false).await?;
                }
            }
            return Ok(());
        }
        if is_reject(text) {
            {
                let mut map = state.pending_tool_limit.lock().await;
                map.remove(&chat_id_i64);
            }
            send_text(&bot, chat_id, "已取消").await?;
            return Ok(());
        }
        {
            let mut map = state.pending_tool_limit.lock().await;
            map.remove(&chat_id_i64);
        }
    }

    if text.eq_ignore_ascii_case("/start") || text.eq_ignore_ascii_case("/menu") {
        send_text(&bot, msg.chat.id, "RBot 已就绪。直接提问或输入命令。").await?;
        return Ok(());
    }

    if text.eq_ignore_ascii_case("命令") || text.eq_ignore_ascii_case("Run") {
        dialogue.update(DialogueState::AwaitingCommand).await?;
        send_text(&bot, msg.chat.id, "发送命令（取消请输入 Cancel/取消）").await?;
        return Ok(());
    }

    if text.eq_ignore_ascii_case("对话") || text.eq_ignore_ascii_case("Chat") {
        send_text(&bot, msg.chat.id, "请发送你的问题。").await?;
        return Ok(());
    }

    if text.eq_ignore_ascii_case("取消") || text.eq_ignore_ascii_case("Cancel") {
        send_text(&bot, msg.chat.id, "已取消").await?;
        return Ok(());
    }

    if text.eq_ignore_ascii_case("网络") || text.eq_ignore_ascii_case("HTTP") {
        dialogue.update(DialogueState::AwaitingHttp).await?;
        send_text(&bot, msg.chat.id, "发送 HTTP：METHOD URL [BODY]").await?;
        return Ok(());
    }

    if text.eq_ignore_ascii_case("任务") || text.eq_ignore_ascii_case("Tmux") {
        dialogue.update(DialogueState::AwaitingTmux).await?;
        send_text(
            &bot,
            msg.chat.id,
            "Tmux：start <name> <cmd> | stop <name> | logs <name> [lines] | list",
        )
        .await?;
        return Ok(());
    }

    if text.eq_ignore_ascii_case("定时") || text.eq_ignore_ascii_case("Schedule") {
        dialogue.update(DialogueState::AwaitingSchedule).await?;
        send_text(
            &bot,
            msg.chat.id,
            "定时：<cron_with_prefix> | msg <text> 或 shell <cmd> 或 http <METHOD> <URL> [BODY]。cron 必须以 rbot_ 或 rbot_system_ 开头。",
        )
        .await?;
        return Ok(());
    }

    if text.eq_ignore_ascii_case("白名单") || text.eq_ignore_ascii_case("Whitelist") {
        dialogue.update(DialogueState::AwaitingWhitelist).await?;
        send_text(&bot, msg.chat.id, "白名单：<tool> <command>（仅管理员）").await?;
        return Ok(());
    }

    if text.eq_ignore_ascii_case("技能") || text.eq_ignore_ascii_case("Skills") {
        let mut out = String::from("Skills:\n");
        for skill in state.skills.list() {
            out.push_str(&format!("- {}: {}\n", skill.name, skill.description));
        }
        out.push_str("Use /skill <name> or /skill_off");
        send_text(&bot, msg.chat.id, out).await?;
        return Ok(());
    }

    if text.eq_ignore_ascii_case("记忆") || text.eq_ignore_ascii_case("Memory") {
        let day = local_day_string(Local::now());
        let summary = state.memory.get_summary(msg.chat.id.0, &day)?.unwrap_or_else(|| "(no summary)".into());
        let long = state.memory.search_long_memory(msg.chat.id.0, "", 5)?;
        let mut out = format!("Summary {}:\n{}\n\nLong memory:\n", day, summary);
        for item in long {
            out.push_str(&format!("- {}\n", item));
        }
        send_text(&bot, msg.chat.id, out).await?;
        return Ok(());
    }

    if text.starts_with("/allow ") {
        handle_allow_command(&bot, &msg, &state, text).await?;
        return Ok(());
    }

    if text.starts_with("/skill ") {
        let name = text.trim_start_matches("/skill ").trim();
        match state.skills.activate(msg.chat.id.0, name) {
            Ok(_) => {
                send_text(&bot, msg.chat.id, format!("skill activated: {}", name)).await?;
            }
            Err(_) => {
                send_text(&bot, msg.chat.id, "skill not found").await?;
            }
        }
        return Ok(());
    }

    if text.eq_ignore_ascii_case("/skill_off") {
        state.skills.deactivate(msg.chat.id.0);
        send_text(&bot, msg.chat.id, "skill cleared").await?;
        return Ok(());
    }

    if text.starts_with("!shell ") {
        let cmd = text.trim_start_matches("!shell ").to_string();
        let out = state.tools.execute(ToolCall::Shell { cmd }).await;
        send_tool_output(&bot, msg.chat.id, out, "shell").await?;
        return Ok(());
    }

    if text.starts_with("!http ") {
        let parts: Vec<&str> = text.trim_start_matches("!http ").splitn(3, ' ').collect();
        if parts.len() < 2 {
            send_text(&bot, msg.chat.id, "format: !http METHOD URL [BODY]").await?;
            return Ok(());
        }
        let method = parts[0].to_string();
        let url = parts[1].to_string();
        let body = parts.get(2).map(|s| s.to_string());
        let out = state.tools.execute(ToolCall::Http { method, url, body }).await;
        send_tool_output(&bot, msg.chat.id, out, "http").await?;
        return Ok(());
    }

    if text.starts_with("!tmux ") {
        let cmd = text.trim_start_matches("!tmux ");
        let action = parse_tmux_action(cmd)?;
        let out = state.tools.execute(ToolCall::Tmux { action }).await;
        send_tool_output(&bot, msg.chat.id, out, "tmux").await?;
        return Ok(());
    }

    if let Some(skill) = state.skills.maybe_trigger(msg.chat.id.0, text) {
        let _ = send_text(&bot, msg.chat.id, format!("skill activated: {}", skill.name)).await;
    }

    // LLM chat
    state.memory.append_message(chat_id_i64, "user", text)?;
    let progress = start_progress(&bot, chat_id).await;
    let message_id = progress.as_ref().map(|p| p.message_id);
    let progress_state = Arc::new(Mutex::new(progress));
    let stream_ctx = StreamContext {
        bot: bot.clone(),
        chat_id,
        message_id,
        progress: progress_state.clone(),
    };
    let response = chat_with_timeout(&state, chat_id_i64, text, None, Some(stream_ctx)).await;
    match response {
        Ok(ChatResult::Reply(reply)) => {
            state.memory.append_message(chat_id_i64, "assistant", &reply)?;
            let progress_handle = progress_state.lock().await.take();
            send_reply_with_progress(&bot, chat_id, &reply, progress_handle, message_id, false).await?;
        }
        Ok(ChatResult::ToolLimit { max, suggested }) => {
            {
                let mut map = state.pending_tool_limit.lock().await;
                map.insert(
                    chat_id_i64,
                    PendingToolLimit {
                        input: text.to_string(),
                        max_tool_calls: suggested,
                    },
                );
            }
            let prompt = format!(
                "工具调用已达上限 {}。回复“继续”可临时提高到 {} 并继续本次请求。",
                max, suggested
            );
            let progress_handle = progress_state.lock().await.take();
            send_reply_with_progress(&bot, chat_id, &prompt, progress_handle, message_id, false).await?;
        }
        Err(err) => {
            let reply = format!("llm error: {}", err);
            state.memory.append_message(chat_id_i64, "assistant", &reply)?;
            let progress_handle = progress_state.lock().await.take();
            send_reply_with_progress(&bot, chat_id, &reply, progress_handle, message_id, false).await?;
        }
    }
    Ok(())
}

async fn handle_callback(bot: AutoSend<Bot>, q: CallbackQuery, state: AppState) -> HandlerResult {
    let data = match q.data.as_deref() {
        Some(d) => d,
        None => return Ok(()),
    };
    if data != CALLBACK_YES && data != CALLBACK_NO {
        return Ok(());
    }
    let _ = bot.answer_callback_query(q.id).await;
    let (chat_id, message_id) = match &q.message {
        Some(msg) => (msg.chat.id, msg.id),
        None => return Ok(()),
    };
    let chat_id_i64 = chat_id.0;
    let _ = bot
        .edit_message_reply_markup(chat_id, message_id)
        .reply_markup(InlineKeyboardMarkup::default())
        .await;

    let text = if data == CALLBACK_YES { "是" } else { "否" };

    let pending = {
        let map = state.pending_tool_limit.lock().await;
        map.get(&chat_id_i64).cloned()
    };
    if let Some(pending) = pending {
        if is_confirm(text) {
            {
                let mut map = state.pending_tool_limit.lock().await;
                map.remove(&chat_id_i64);
            }
            let progress = start_progress(&bot, chat_id).await;
            let message_id = progress.as_ref().map(|p| p.message_id);
            let progress_state = Arc::new(Mutex::new(progress));
            let stream_ctx = StreamContext {
                bot: bot.clone(),
                chat_id,
                message_id,
                progress: progress_state.clone(),
            };
            let response = chat_with_timeout(
                &state,
                chat_id_i64,
                &pending.input,
                Some(pending.max_tool_calls),
                Some(stream_ctx),
            )
            .await;
            match response {
                Ok(ChatResult::Reply(reply)) => {
                    state.memory.append_message(chat_id_i64, "assistant", &reply)?;
                    let progress_handle = progress_state.lock().await.take();
                    send_reply_with_progress(&bot, chat_id, &reply, progress_handle, message_id, false)
                        .await?;
                }
                Ok(ChatResult::ToolLimit { max, suggested }) => {
                    {
                        let mut map = state.pending_tool_limit.lock().await;
                        map.insert(
                            chat_id_i64,
                            PendingToolLimit {
                                input: pending.input,
                                max_tool_calls: suggested,
                            },
                        );
                    }
                    let prompt = format!(
                        "工具调用已达上限 {}。回复“继续”可临时提高到 {} 并继续本次请求。",
                        max, suggested
                    );
                    let progress_handle = progress_state.lock().await.take();
                    send_reply_with_progress(&bot, chat_id, &prompt, progress_handle, message_id, false)
                        .await?;
                }
                Err(err) => {
                    let reply = format!("llm error: {}", err);
                    state.memory.append_message(chat_id_i64, "assistant", &reply)?;
                    let progress_handle = progress_state.lock().await.take();
                    send_reply_with_progress(&bot, chat_id, &reply, progress_handle, message_id, false)
                        .await?;
                }
            }
            return Ok(());
        }
        if is_reject(text) {
            {
                let mut map = state.pending_tool_limit.lock().await;
                map.remove(&chat_id_i64);
            }
            send_text(&bot, chat_id, "已取消").await?;
            return Ok(());
        }
        {
            let mut map = state.pending_tool_limit.lock().await;
            map.remove(&chat_id_i64);
        }
    }

    state.memory.append_message(chat_id_i64, "user", text)?;
    let progress = start_progress(&bot, chat_id).await;
    let message_id = progress.as_ref().map(|p| p.message_id);
    let progress_state = Arc::new(Mutex::new(progress));
    let stream_ctx = StreamContext {
        bot: bot.clone(),
        chat_id,
        message_id,
        progress: progress_state.clone(),
    };
    let response = chat_with_timeout(&state, chat_id_i64, text, None, Some(stream_ctx)).await;
    match response {
        Ok(ChatResult::Reply(reply)) => {
            state.memory.append_message(chat_id_i64, "assistant", &reply)?;
            let progress_handle = progress_state.lock().await.take();
            send_reply_with_progress(&bot, chat_id, &reply, progress_handle, message_id, false).await?;
        }
        Ok(ChatResult::ToolLimit { max, suggested }) => {
            {
                let mut map = state.pending_tool_limit.lock().await;
                map.insert(
                    chat_id_i64,
                    PendingToolLimit {
                        input: text.to_string(),
                        max_tool_calls: suggested,
                    },
                );
            }
            let prompt = format!(
                "工具调用已达上限 {}。回复“继续”可临时提高到 {} 并继续本次请求。",
                max, suggested
            );
            let progress_handle = progress_state.lock().await.take();
            send_reply_with_progress(&bot, chat_id, &prompt, progress_handle, message_id, false).await?;
        }
        Err(err) => {
            let reply = format!("llm error: {}", err);
            state.memory.append_message(chat_id_i64, "assistant", &reply)?;
            let progress_handle = progress_state.lock().await.take();
            send_reply_with_progress(&bot, chat_id, &reply, progress_handle, message_id, false).await?;
        }
    }
    Ok(())
}

async fn handle_command(bot: AutoSend<Bot>, msg: Message, dialogue: MyDialogue, state: AppState) -> HandlerResult {
    let text = msg.text().unwrap_or("").trim();
    if text.eq_ignore_ascii_case("cancel") {
        dialogue.update(DialogueState::Idle).await?;
        send_text(&bot, msg.chat.id, "Cancelled").await?;
        return Ok(());
    }
    let out = state.tools.execute(ToolCall::Shell { cmd: text.to_string() }).await;
    send_tool_output(&bot, msg.chat.id, out, "shell").await?;
    dialogue.update(DialogueState::Idle).await?;
    Ok(())
}

async fn handle_http(bot: AutoSend<Bot>, msg: Message, dialogue: MyDialogue, state: AppState) -> HandlerResult {
    let text = msg.text().unwrap_or("").trim();
    if text.eq_ignore_ascii_case("cancel") {
        dialogue.update(DialogueState::Idle).await?;
        send_text(&bot, msg.chat.id, "Cancelled").await?;
        return Ok(());
    }
    let parts: Vec<&str> = text.splitn(3, ' ').collect();
    if parts.len() < 2 {
        send_text(&bot, msg.chat.id, "format: METHOD URL [BODY]").await?;
        return Ok(());
    }
    let method = parts[0].to_string();
    let url = parts[1].to_string();
    let body = parts.get(2).map(|s| s.to_string());
    let out = state.tools.execute(ToolCall::Http { method, url, body }).await;
    send_tool_output(&bot, msg.chat.id, out, "http").await?;
    dialogue.update(DialogueState::Idle).await?;
    Ok(())
}

async fn handle_tmux(bot: AutoSend<Bot>, msg: Message, dialogue: MyDialogue, state: AppState) -> HandlerResult {
    let text = msg.text().unwrap_or("").trim();
    if text.eq_ignore_ascii_case("cancel") {
        dialogue.update(DialogueState::Idle).await?;
        send_text(&bot, msg.chat.id, "Cancelled").await?;
        return Ok(());
    }
    let action = parse_tmux_action(text)?;
    let out = state.tools.execute(ToolCall::Tmux { action }).await;
    send_tool_output(&bot, msg.chat.id, out, "tmux").await?;
    dialogue.update(DialogueState::Idle).await?;
    Ok(())
}

async fn handle_schedule(bot: AutoSend<Bot>, msg: Message, dialogue: MyDialogue, state: AppState) -> HandlerResult {
    let text = msg.text().unwrap_or("").trim();
    if text.eq_ignore_ascii_case("cancel") {
        dialogue.update(DialogueState::Idle).await?;
        send_text(&bot, msg.chat.id, "Cancelled").await?;
        return Ok(());
    }
    let parts: Vec<&str> = text.splitn(2, '|').collect();
    if parts.len() < 2 {
        send_text(
            &bot,
            msg.chat.id,
            "format: <cron_with_prefix> | msg <text> OR shell <cmd> OR http <METHOD> <URL> [BODY]. cron must start with rbot_ or rbot_system_.",
        )
        .await?;
        return Ok(());
    }
    let cron = parts[0].trim();
    let action_str = parts[1].trim();
    let action = parse_schedule_action(action_str)?;
    let id = state.scheduler.add_schedule(msg.chat.id.0, cron, action).await?;
    send_text(&bot, msg.chat.id, format!("scheduled id {}", id)).await?;
    dialogue.update(DialogueState::Idle).await?;
    Ok(())
}

async fn handle_whitelist(bot: AutoSend<Bot>, msg: Message, dialogue: MyDialogue, state: AppState) -> HandlerResult {
    let text = msg.text().unwrap_or("").trim();
    if text.eq_ignore_ascii_case("cancel") {
        dialogue.update(DialogueState::Idle).await?;
        send_text(&bot, msg.chat.id, "Cancelled").await?;
        return Ok(());
    }
    handle_allow_command(&bot, &msg, &state, text).await?;
    dialogue.update(DialogueState::Idle).await?;
    Ok(())
}

async fn handle_allow_command(bot: &AutoSend<Bot>, msg: &Message, state: &AppState, text: &str) -> HandlerResult {
    let user_id_u64 = msg.from().map(|u| u.id.0).unwrap_or(0);
    let user_id = i64::try_from(user_id_u64).unwrap_or(0);
    if !state.cfg.telegram.admin_user_ids.contains(&user_id) {
        send_text(bot, msg.chat.id, "admin only").await?;
        return Ok(());
    }
    let mut parts = text.trim_start_matches("/allow").trim().splitn(2, ' ');
    let tool = parts.next().unwrap_or("").trim();
    let cmd = parts.next().unwrap_or("").trim();
    if tool.is_empty() || cmd.is_empty() {
        send_text(bot, msg.chat.id, "format: /allow <tool> <command>").await?;
        return Ok(());
    }
    state.tools.extend_allowlist(tool, cmd, user_id)?;
    send_text(bot, msg.chat.id, format!("allowlist updated: {} {}", tool, cmd)).await?;
    Ok(())
}

#[derive(Serialize)]
struct ReactionType {
    #[serde(rename = "type")]
    kind: &'static str,
    emoji: String,
}

#[derive(Serialize)]
struct SetMessageReactionPayload {
    chat_id: i64,
    message_id: i32,
    reaction: Vec<ReactionType>,
}

async fn try_like_message(state: &AppState, msg: &Message) -> Option<()> {
    let token = state.cfg.telegram.token.trim();
    if token.is_empty() {
        return None;
    }
    let url = format!("https://api.telegram.org/bot{}/setMessageReaction", token);
    let mut builder = Client::builder().no_proxy();
    if let Some(proxy_url) = state
        .cfg
        .network
        .proxy_url
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        let proxy = Proxy::all(proxy_url).ok()?;
        builder = builder.proxy(proxy);
    }
    let client = builder.build().ok()?;
    let payload = SetMessageReactionPayload {
        chat_id: msg.chat.id.0,
        message_id: msg.id.0,
        reaction: vec![ReactionType {
            kind: "emoji",
            emoji: "👍".to_string(),
        }],
    };
    let _ = client.post(url).json(&payload).send().await;
    Some(())
}

async fn start_progress(bot: &AutoSend<Bot>, chat_id: ChatId) -> Option<ProgressHandle> {
    let message = bot
        .send_message(chat_id, "☁️☁️☁️～")
        .parse_mode(ParseMode::Html)
        .await
        .ok()?;
    let (stop, mut stop_rx) = oneshot::channel::<()>();
    let bot_clone = bot.clone();
    let message_id = message.id;
    let join = tokio::spawn(async move {
        let mut ticker = time::interval(Duration::from_secs(4));
        loop {
            tokio::select! {
                _ = &mut stop_rx => break,
                _ = ticker.tick() => {
                    let _ = bot_clone.send_chat_action(chat_id, ChatAction::Typing).await;
                }
            }
        }
    });
    let _ = bot.send_chat_action(chat_id, ChatAction::Typing).await;
    Some(ProgressHandle {
        stop,
        join,
        message_id,
    })
}

async fn send_reply_with_progress(
    bot: &AutoSend<Bot>,
    chat_id: ChatId,
    reply: &str,
    progress: Option<ProgressHandle>,
    message_id: Option<MessageId>,
    stream: bool,
) -> HandlerResult {
    let markup = yes_no_markup(reply);
    if let Some(handle) = progress {
        let ProgressHandle { stop, join, message_id } = handle;
        let _ = stop.send(());
        join.abort();
        let _ = time::timeout(Duration::from_millis(200), join).await;
        time::sleep(Duration::from_millis(40)).await;
        let mut delivered = false;
        if stream && should_stream(reply) && markup.is_none() {
            delivered = stream_edit_message(bot, chat_id, message_id, reply).await;
        }
        if !delivered {
            let mut req = bot
                .edit_message_text(chat_id, message_id, reply.to_string())
                .parse_mode(ParseMode::Html);
            if let Some(m) = markup.clone() {
                req = req.reply_markup(m);
            }
            if req.await.is_ok() {
                delivered = true;
            }
        }
        if !delivered {
            let mut req = bot
                .send_message(chat_id, reply.to_string())
                .parse_mode(ParseMode::Html);
            if let Some(m) = markup.clone() {
                req = req.reply_markup(m);
            }
            let send = req.await;
            if send.is_err() {
                let safe = escape_html(reply);
                bot.send_message(chat_id, safe)
                    .parse_mode(ParseMode::Html)
                    .await?;
            }
            let _ = bot.delete_message(chat_id, message_id).await;
        }
        return Ok(());
    }
    if let Some(message_id) = message_id {
        let mut req = bot
            .edit_message_text(chat_id, message_id, reply.to_string())
            .parse_mode(ParseMode::Html);
        if let Some(m) = markup.clone() {
            req = req.reply_markup(m);
        }
        if req.await.is_ok() {
            return Ok(());
        }
        let safe = escape_html(reply);
        let mut req = bot
            .edit_message_text(chat_id, message_id, safe)
            .parse_mode(ParseMode::Html);
        if let Some(m) = markup.clone() {
            req = req.reply_markup(m);
        }
        if req.await.is_ok() {
            return Ok(());
        }
    }
    let mut req = bot
        .send_message(chat_id, reply.to_string())
        .parse_mode(ParseMode::Html);
    if let Some(m) = markup {
        req = req.reply_markup(m);
    }
    let send = req.await;
    if send.is_err() {
        let safe = escape_html(reply);
        bot.send_message(chat_id, safe)
            .parse_mode(ParseMode::Html)
            .await?;
    }
    Ok(())
}

fn yes_no_markup(text: &str) -> Option<InlineKeyboardMarkup> {
    if !text.contains("如果你要") {
        return None;
    }
    let row = vec![
        InlineKeyboardButton::callback("是", CALLBACK_YES),
        InlineKeyboardButton::callback("否", CALLBACK_NO),
    ];
    Some(InlineKeyboardMarkup::new(vec![row]))
}

async fn stream_edit_message(
    bot: &AutoSend<Bot>,
    chat_id: ChatId,
    message_id: MessageId,
    text: &str,
) -> bool {
    let len = text.chars().count();
    let steps = stream_steps(len);
    if steps == 0 {
        return false;
    }
    let chars: Vec<char> = text.chars().collect();
    for step in 1..=steps {
        let end = (len * step) / (steps + 1);
        let partial: String = chars.iter().take(end).collect();
        let safe = escape_html(&partial);
        if bot
            .edit_message_text(chat_id, message_id, safe)
            .parse_mode(ParseMode::Html)
            .await
            .is_err()
        {
            return false;
        }
        time::sleep(Duration::from_millis(140)).await;
    }
    bot.edit_message_text(chat_id, message_id, text.to_string())
        .parse_mode(ParseMode::Html)
        .await
        .is_ok()
}

fn stream_steps(len: usize) -> usize {
    if len < 400 {
        0
    } else if len < 900 {
        2
    } else if len < 1800 {
        3
    } else {
        4
    }
}

fn should_stream(text: &str) -> bool {
    stream_steps(text.chars().count()) > 0
}

fn suggested_tool_limit(current: usize) -> usize {
    current.saturating_add(8)
}

fn is_confirm(text: &str) -> bool {
    let t = normalize_choice(text);
    matches!(
        t.as_str(),
        "继续"
            | "继续吧"
            | "确认"
            | "同意"
            | "允许"
            | "是"
            | "是的"
            | "好"
            | "好的"
            | "ok"
            | "okay"
            | "yes"
            | "y"
    )
}

fn is_reject(text: &str) -> bool {
    let t = normalize_choice(text);
    matches!(t.as_str(), "取消" | "算了" | "不用" | "不需要" | "no" | "n")
}

fn normalize_choice(text: &str) -> String {
    let mut t = text.trim().to_lowercase();
    for token in ["✅", "❌", "👍", "👎"] {
        t = t.replace(token, "");
    }
    t.replace(' ', "")
}

async fn chat_stream_with_updates(
    llm: Arc<dyn LlmClient>,
    messages: Vec<LlmMessage>,
    options: ChatOptions,
    ctx: &StreamContext,
    request_id: &str,
    round: usize,
) -> anyhow::Result<LlmResponse> {
    let started = Instant::now();
    info!(request_id, round, "llm.stream.start");
    let mut editor = StreamEditor::new(ctx).await?;
    let mut rx = llm.chat_stream(messages, options).await?;
    let mut content = String::new();
    let mut tool_calls = Vec::new();
    while let Some(event) = rx.recv().await {
        match event {
            StreamEvent::Delta(delta) => {
                content.push_str(&delta);
                editor.update(&content).await;
            }
            StreamEvent::Done(resp) => {
                content = resp.content;
                tool_calls = resp.tool_calls;
                break;
            }
            StreamEvent::Error(err) => return Err(anyhow::anyhow!(err)),
        }
    }
    info!(
        request_id,
        round,
        elapsed_ms = started.elapsed().as_millis(),
        tool_calls = tool_calls.len(),
        content_len = content.len(),
        "llm.stream.done"
    );
    Ok(LlmResponse { content, tool_calls })
}

impl StreamEditor {
    async fn new(ctx: &StreamContext) -> anyhow::Result<Self> {
        let message_id = match ctx.message_id {
            Some(id) => id,
            None => {
                let msg = ctx
                    .bot
                    .send_message(ctx.chat_id, "☁️☁️☁️～")
                    .parse_mode(ParseMode::Html)
                    .await?;
                msg.id
            }
        };
        let now = Instant::now();
        Ok(Self {
            bot: ctx.bot.clone(),
            chat_id: ctx.chat_id,
            message_id,
            created_at: now,
            // Allow the first partial update to land immediately.
            last_edit: now - Duration::from_millis(900),
            last_len: 0,
            min_interval: Duration::from_millis(350),
            min_chars: 24,
            last_typing: now,
            progress: ctx.progress.clone(),
            stopped: false,
        })
    }

    async fn stop_progress(&mut self) {
        if self.stopped {
            return;
        }
        if let Some(handle) = self.progress.lock().await.take() {
            let ProgressHandle { stop, join, .. } = handle;
            let _ = stop.send(());
            join.abort();
            let _ = time::timeout(Duration::from_millis(200), join).await;
        }
        self.stopped = true;
    }

    async fn update(&mut self, content: &str) {
        if content.is_empty() {
            return;
        }
        self.stop_progress().await;
        let now = Instant::now();
        if now.duration_since(self.last_typing) >= Duration::from_secs(4) {
            let _ = self.bot.send_chat_action(self.chat_id, ChatAction::Typing).await;
        self.last_typing = now;
        }
        let delta = content.len().saturating_sub(self.last_len);
        let elapsed = now.duration_since(self.created_at);
        let burst_interval = if elapsed < Duration::from_secs(2) {
            Duration::from_millis(200)
        } else if elapsed < Duration::from_secs(5) {
            Duration::from_millis(350)
        } else {
            self.min_interval
        };
        let interval = self.min_interval.max(burst_interval);
        let min_chars = if elapsed < Duration::from_secs(2) {
            8
        } else if elapsed < Duration::from_secs(5) {
            16
        } else if content.len() < 200 {
            16
        } else {
            self.min_chars
        };
        if now.duration_since(self.last_edit) < interval && delta < min_chars {
            return;
        }
        let mut safe = escape_html(content);
        if safe.len() > 3800 {
            safe.truncate(3800);
            safe.push_str("…");
        }
        let result = self
            .bot
            .edit_message_text(self.chat_id, self.message_id, safe)
            .parse_mode(ParseMode::Html)
            .await;
        match result {
            Ok(_) => {
                self.last_edit = now;
                self.last_len = content.len();
                self.min_interval = Duration::from_millis(450);
            }
            Err(_) => {
                self.min_interval = Duration::from_millis(1000);
            }
        }
    }
}

async fn chat_with_timeout(
    state: &AppState,
    chat_id: i64,
    input: &str,
    max_tool_calls_override: Option<usize>,
    stream: Option<StreamContext>,
) -> anyhow::Result<ChatResult> {
    let request_id = Uuid::new_v4().to_string();
    let timeout_secs = state
        .cfg
        .llm
        .overall_timeout_secs
        .unwrap_or_else(|| state.cfg.llm.request_timeout_secs.saturating_add(120));
    info!(
        request_id,
        chat_id,
        timeout_secs,
        streaming = stream.is_some(),
        "chat.start"
    );
    let started = Instant::now();
    let result = if timeout_secs == 0 {
        chat_with_llm(
            state,
            chat_id,
            input,
            max_tool_calls_override,
            stream,
            request_id.clone(),
        )
        .await
    } else {
        let timeout = Duration::from_secs(timeout_secs);
        match time::timeout(
            timeout,
            chat_with_llm(
                state,
                chat_id,
                input,
                max_tool_calls_override,
                stream,
                request_id.clone(),
            ),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => {
                warn!(
                    request_id,
                    elapsed_ms = started.elapsed().as_millis(),
                    timeout_secs,
                    "chat.timeout"
                );
                Ok(ChatResult::Reply(format!(
                    "处理超时（{} 秒）。可以回复“重试”或稍后再试。",
                    timeout_secs
                )))
            }
        }
    };
    match &result {
        Ok(ChatResult::Reply(text)) => info!(
            request_id,
            elapsed_ms = started.elapsed().as_millis(),
            reply_len = text.len(),
            "chat.done"
        ),
        Ok(ChatResult::ToolLimit { max, suggested }) => info!(
            request_id,
            elapsed_ms = started.elapsed().as_millis(),
            max,
            suggested,
            "chat.tool_limit"
        ),
        Err(err) => warn!(
            request_id,
            elapsed_ms = started.elapsed().as_millis(),
            error = %err,
            "chat.error"
        ),
    }
    result
}

async fn chat_with_llm(
    state: &AppState,
    chat_id: i64,
    input: &str,
    max_tool_calls_override: Option<usize>,
    stream: Option<StreamContext>,
    request_id: String,
) -> anyhow::Result<ChatResult> {
    let llm = match &state.llm {
        Some(llm) => llm.clone(),
        None => anyhow::bail!("llm not configured"),
    };
    let mut messages = Vec::new();
    messages.push(LlmMessage {
        role: "system".into(),
        content: format!(
            "Persona:\n{}\n\nRules:\n- Reply fast and directly.\n- Use only function tool calls when needed; never output manual tool instructions.\n- If you can check local state, do it via tools; don’t ask the user to run commands.\n- Don’t install dependencies unless explicitly asked; prefer built-in tools.\n- Avoid multiple-choice prompts; pick a reasonable default and proceed.\n- For data queries, return a single best answer. If it’s unavailable, silently try the next best source/variant and only ask if all options fail.\n- For real-time/web data, use search/http tools; if a tool fails, show the error and a concrete config fix.\n- Ask the minimum clarifying questions; keep context across turns.\n- Format for Telegram HTML (escape <, >, &; only supported tags).\n- Be concise.",
            state.persona
        ),
        tool_call_id: None,
        tool_calls: None,
    });
    if let Some(skill) = state.skills.active_skill(chat_id) {
        if let Some(prompt) = skill.system_prompt {
            messages.push(LlmMessage {
                role: "system".into(),
                content: prompt,
                tool_call_id: None,
                tool_calls: None,
            });
        }
    }
    let day = local_day_string(Local::now());
    if let Some(summary) = state.memory.get_summary(chat_id, &day)? {
        messages.push(LlmMessage {
            role: "system".into(),
            content: format!("Memory summary: {}", summary),
            tool_call_id: None,
            tool_calls: None,
        });
    }
    let long = state.memory.search_long_memory(chat_id, input, 3)?;
    if !long.is_empty() {
        let joined = long.join("; ");
        messages.push(LlmMessage {
            role: "system".into(),
            content: format!("Long memory: {}", joined),
            tool_call_id: None,
            tool_calls: None,
        });
    }
    let recent = state
        .memory
        .get_recent_messages(chat_id, state.cfg.memory.short_term_limit)?;
    let mut recent_msgs = to_llm_messages(recent);
    let add_input = match recent_msgs.last() {
        Some(last) => !(last.role == "user" && last.content == input),
        None => true,
    };
    messages.append(&mut recent_msgs);
    if add_input {
        messages.push(LlmMessage {
            role: "user".into(),
            content: input.to_string(),
            tool_call_id: None,
            tool_calls: None,
        });
    }
    let max_tool_calls = max_tool_calls_override
        .unwrap_or(state.cfg.llm.max_tool_calls)
        .max(1);
    let mut tool_calls_used = 0usize;
    let mut round = 0usize;
    loop {
        round += 1;
        info!(
            request_id = request_id.as_str(),
            round,
            messages = messages.len(),
            tool_calls_used,
            "llm.call.start"
        );
        let reply = if let Some(ctx) = &stream {
            match chat_stream_with_updates(
                llm.clone(),
                messages.clone(),
                ChatOptions {
                    temperature: 0.2,
                    tools: true,
                },
                ctx,
                request_id.as_str(),
                round,
            )
            .await
            {
                Ok(resp) => resp,
                Err(err) => {
                    warn!(
                        request_id = request_id.as_str(),
                        round,
                        error = %err,
                        "llm.stream.failed"
                    );
                    let started = Instant::now();
                    let resp = llm.chat(
                        messages.clone(),
                        ChatOptions {
                            temperature: 0.2,
                            tools: true,
                        },
                    )
                    .await?;
                    info!(
                        request_id = request_id.as_str(),
                        round,
                        elapsed_ms = started.elapsed().as_millis(),
                        tool_calls = resp.tool_calls.len(),
                        content_len = resp.content.len(),
                        "llm.chat.done"
                    );
                    resp
                }
            }
        } else {
            let started = Instant::now();
            let resp = llm.chat(
                messages.clone(),
                ChatOptions {
                    temperature: 0.2,
                    tools: true,
                },
            )
            .await?;
            info!(
                request_id = request_id.as_str(),
                round,
                elapsed_ms = started.elapsed().as_millis(),
                tool_calls = resp.tool_calls.len(),
                content_len = resp.content.len(),
                "llm.chat.done"
            );
            resp
        };
        info!(
            request_id = request_id.as_str(),
            round,
            tool_calls = reply.tool_calls.len(),
            content_len = reply.content.len(),
            "llm.call.done"
        );
        if !reply.tool_calls.is_empty() {
            if tool_calls_used + reply.tool_calls.len() > max_tool_calls {
                warn!(
                    request_id = request_id.as_str(),
                    round,
                    max_tool_calls,
                    used = tool_calls_used,
                    pending = reply.tool_calls.len(),
                    "tool.limit.hit"
                );
                return Ok(ChatResult::ToolLimit {
                    max: max_tool_calls,
                    suggested: suggested_tool_limit(max_tool_calls),
                });
            }
            tool_calls_used += reply.tool_calls.len();
            messages.push(LlmMessage {
                role: "assistant".into(),
                content: reply.content.clone(),
                tool_call_id: None,
                tool_calls: Some(reply.tool_calls.clone()),
            });
            for call in reply.tool_calls {
                let tool_result = match tool_call_from_llm(&call) {
                    Ok(tool_call) => {
                        let tool_name = tool_name(&tool_call);
                        let started = Instant::now();
                        info!(
                            request_id = request_id.as_str(),
                            round,
                            tool = tool_name,
                            "tool.start"
                        );
                        match state.tools.execute(tool_call).await {
                            Ok(out) => {
                                info!(
                                    request_id = request_id.as_str(),
                                    round,
                                    tool = tool_name,
                                    exit = out.exit_code,
                                    elapsed_ms = started.elapsed().as_millis(),
                                    "tool.done"
                                );
                                format!(
                                    "TOOL_RESULT stdout:\n{}\n\nstderr:\n{}\ncode:{}",
                                    out.stdout, out.stderr, out.exit_code
                                )
                            }
                            Err(err) => {
                                warn!(
                                    request_id = request_id.as_str(),
                                    round,
                                    tool = tool_name,
                                    elapsed_ms = started.elapsed().as_millis(),
                                    error = %err,
                                    "tool.error"
                                );
                                format_tool_error_plain(tool_name, &err)
                            }
                        }
                    }
                    Err(err) => format!("TOOL_RESULT error: {}", err),
                };
                state.memory.append_message(chat_id, "tool", &tool_result)?;
                messages.push(LlmMessage {
                    role: "tool".into(),
                    content: tool_result,
                    tool_call_id: Some(call.id),
                    tool_calls: None,
                });
            }
            continue;
        }
        return Ok(ChatResult::Reply(reply.content));
    }
}

fn to_llm_messages(messages: Vec<StoredMessage>) -> Vec<LlmMessage> {
    messages
        .into_iter()
        .map(|m| {
            if m.role == "tool" {
                return LlmMessage {
                    role: "system".to_string(),
                    content: format!("Tool result: {}", m.content),
                    tool_call_id: None,
                    tool_calls: None,
                };
            }
            LlmMessage {
                role: m.role,
                content: m.content,
                tool_call_id: None,
                tool_calls: None,
            }
        })
        .collect()
}

fn parse_tmux_action(text: &str) -> anyhow::Result<TmuxAction> {
    let parts: Vec<&str> = text.splitn(4, ' ').collect();
    let cmd = parts.get(0).map(|s| s.to_lowercase()).unwrap_or_default();
    match cmd.as_str() {
        "start" => {
            if parts.len() < 3 {
                anyhow::bail!("format: start <name> <cmd>");
            }
            let session = parts[1].to_string();
            let cmd = parts[2..].join(" ");
            Ok(TmuxAction::Start { session, cmd })
        }
        "stop" => {
            let session = parts.get(1).ok_or_else(|| anyhow::anyhow!("format: stop <name>"))?;
            Ok(TmuxAction::Stop {
                session: session.to_string(),
            })
        }
        "logs" => {
            let session = parts.get(1).ok_or_else(|| anyhow::anyhow!("format: logs <name> [lines]"))?;
            let lines = parts.get(2).and_then(|s| s.parse::<usize>().ok()).unwrap_or(200);
            Ok(TmuxAction::Logs {
                session: session.to_string(),
                lines,
            })
        }
        "list" => Ok(TmuxAction::List),
        _ => anyhow::bail!("unknown tmux action"),
    }
}

fn parse_schedule_action(text: &str) -> anyhow::Result<ScheduledAction> {
    let parts: Vec<&str> = text.splitn(2, ' ').collect();
    let kind = parts.get(0).map(|s| s.to_lowercase()).unwrap_or_default();
    let rest = parts.get(1).map(|s| s.trim()).unwrap_or("");
    match kind.as_str() {
        "msg" => Ok(ScheduledAction::Message {
            text: rest.to_string(),
        }),
        "shell" => Ok(ScheduledAction::Tool {
            tool: ToolCall::Shell {
                cmd: rest.to_string(),
            },
        }),
        "http" => {
            let http_parts: Vec<&str> = rest.splitn(3, ' ').collect();
            if http_parts.len() < 2 {
                anyhow::bail!("format: http METHOD URL [BODY]");
            }
            Ok(ScheduledAction::Tool {
                tool: ToolCall::Http {
                    method: http_parts[0].to_string(),
                    url: http_parts[1].to_string(),
                    body: http_parts.get(2).map(|s| s.to_string()),
                },
            })
        }
        "tmux" => {
            let action = parse_tmux_action(rest)?;
            Ok(ScheduledAction::Tool {
                tool: ToolCall::Tmux { action },
            })
        }
        _ => anyhow::bail!("unknown schedule action"),
    }
}

async fn send_tool_output(
    bot: &AutoSend<Bot>,
    chat_id: ChatId,
    out: Result<crate::tools::ToolOutput, crate::tools::ToolError>,
    tool_name: &str,
) -> HandlerResult {
    match out {
        Ok(output) => {
            let text = format_tool_output_html(&output);
            send_html(bot, chat_id, text).await?;
        }
        Err(err) => {
            let text = escape_html(&format_tool_error_plain(tool_name, &err));
            send_html(bot, chat_id, text).await?;
        }
    }
    Ok(())
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

async fn send_html(bot: &AutoSend<Bot>, chat_id: ChatId, text: impl Into<String>) -> HandlerResult {
    bot.send_message(chat_id, text.into())
        .parse_mode(ParseMode::Html)
        .await?;
    Ok(())
}

async fn send_text(bot: &AutoSend<Bot>, chat_id: ChatId, text: impl AsRef<str>) -> HandlerResult {
    let safe = escape_html(text.as_ref());
    send_html(bot, chat_id, safe).await
}

fn tool_name(call: &ToolCall) -> &'static str {
    match call {
        ToolCall::Shell { .. } => "shell",
        ToolCall::Http { .. } => "http",
        ToolCall::Search { .. } => "search",
        ToolCall::Pdf { .. } => "pdf",
        ToolCall::Tmux { .. } => "tmux",
    }
}

fn format_tool_error_plain(tool: &str, err: &ToolError) -> String {
    match err {
        ToolError::NotAllowed => format!(
            "TOOL_RESULT error: command not allowed. To enable, {}",
            tool_enable_hint(tool)
        ),
        ToolError::Dangerous => {
            "TOOL_RESULT error: dangerous command rejected. Check security.danger_patterns in config/config.toml."
                .to_string()
        }
        ToolError::InvalidInput(msg) => format!("TOOL_RESULT error: invalid tool input: {}", msg),
        ToolError::Execution(msg) => format!("TOOL_RESULT error: tool execution failed: {}", msg),
    }
}

fn tool_enable_hint(tool: &str) -> &'static str {
    match tool {
        "http" => "set tools.http.allow_all = true OR add domain to tools.http.allowed_domains in config/config.toml.",
        "shell" => "set tools.shell.mode = \"blocklist\" and update tools.shell.blocklist OR use /allow shell <command> with allowlist mode.",
        "tmux" => "use /allow tmux <command> OR set tools.tmux.allow_all = true OR add to tools.tmux.allowlist.",
        "search" => "set tools.search.api_key (Tavily) in config/config.toml; endpoint optional.",
        "pdf" => "ensure the PDF file path is accessible on disk.",
        _ => "update tool allowlist in config/config.toml.",
    }
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

fn tool_call_from_llm(call: &LlmToolCall) -> anyhow::Result<ToolCall> {
    let name = call.function.name.as_str();
    let args = call.function.arguments.as_str();
    match name {
        "shell" => {
            #[derive(Deserialize)]
            struct ShellArgs {
                cmd: String,
            }
            let parsed: ShellArgs = serde_json::from_str(args)?;
            Ok(ToolCall::Shell { cmd: parsed.cmd })
        }
        "http" => {
            #[derive(Deserialize)]
            struct HttpArgs {
                method: String,
                url: String,
                body: Option<serde_json::Value>,
            }
            let parsed: HttpArgs = serde_json::from_str(args)?;
            let body = parsed.body.map(|v| match v {
                serde_json::Value::String(s) => s,
                other => other.to_string(),
            });
            Ok(ToolCall::Http {
                method: parsed.method,
                url: parsed.url,
                body,
            })
        }
        "search" => {
            #[derive(Deserialize)]
            struct SearchArgs {
                query: String,
                count: Option<usize>,
            }
            let parsed: SearchArgs = serde_json::from_str(args)?;
            Ok(ToolCall::Search {
                query: parsed.query,
                count: parsed.count,
            })
        }
        "tmux" => {
            #[derive(Deserialize)]
            struct TmuxArgs {
                action: String,
            }
            let parsed: TmuxArgs = serde_json::from_str(args)?;
            let action = parse_tmux_action(&parsed.action)?;
            Ok(ToolCall::Tmux { action })
        }
        "pdf" => {
            #[derive(Deserialize)]
            struct PdfArgs {
                path: String,
                max_chars: Option<usize>,
            }
            let parsed: PdfArgs = serde_json::from_str(args)?;
            Ok(ToolCall::Pdf {
                path: parsed.path,
                max_chars: parsed.max_chars,
            })
        }
        _ => anyhow::bail!("unknown tool: {}", name),
    }
}
