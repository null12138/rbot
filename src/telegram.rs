use crate::config::Config;
use crate::llm::{ChatOptions, LlmClient, LlmMessage, LlmToolCall};
use serde::Deserialize;
use crate::memory::{local_day_string, MemoryStore, StoredMessage};
use crate::scheduler::{ScheduledAction, Scheduler};
use crate::skills::SkillManager;
use crate::tools::{tmux::TmuxAction, ToolCall, ToolRegistry};
use chrono::Local;
use std::sync::Arc;
use teloxide::dispatching::dialogue::{Dialogue, InMemStorage};
use teloxide::prelude::*;
use teloxide::types::{KeyboardButton, KeyboardMarkup, ParseMode};

#[derive(Clone)]
pub struct AppState {
    pub cfg: Config,
    pub memory: MemoryStore,
    pub tools: ToolRegistry,
    pub scheduler: Arc<Scheduler>,
    pub skills: SkillManager,
    pub llm: Option<Arc<dyn LlmClient>>,
    pub persona: String,
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
    let handler = Update::filter_message()
        .enter_dialogue::<Message, InMemStorage<DialogueState>, DialogueState>()
        .branch(dptree::case![DialogueState::Idle].endpoint(handle_idle))
        .branch(dptree::case![DialogueState::AwaitingCommand].endpoint(handle_command))
        .branch(dptree::case![DialogueState::AwaitingHttp].endpoint(handle_http))
        .branch(dptree::case![DialogueState::AwaitingTmux].endpoint(handle_tmux))
        .branch(dptree::case![DialogueState::AwaitingSchedule].endpoint(handle_schedule))
        .branch(dptree::case![DialogueState::AwaitingWhitelist].endpoint(handle_whitelist));

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

    if text.eq_ignore_ascii_case("/start") || text.eq_ignore_ascii_case("/menu") {
        bot.send_message(msg.chat.id, escape_html("菜单"))
            .parse_mode(ParseMode::Html)
            .reply_markup(menu_keyboard())
            .await?;
        return Ok(());
    }

    if text.eq_ignore_ascii_case("命令") || text.eq_ignore_ascii_case("Run") {
        dialogue.update(DialogueState::AwaitingCommand).await?;
        bot.send_message(msg.chat.id, escape_html("发送命令（取消请输入 Cancel/取消）"))
            .parse_mode(ParseMode::Html)
            .await?;
        return Ok(());
    }

    if text.eq_ignore_ascii_case("对话") || text.eq_ignore_ascii_case("Chat") {
        bot.send_message(msg.chat.id, escape_html("请发送你的问题。"))
            .parse_mode(ParseMode::Html)
            .await?;
        return Ok(());
    }

    if text.eq_ignore_ascii_case("取消") || text.eq_ignore_ascii_case("Cancel") {
        bot.send_message(msg.chat.id, escape_html("已取消"))
            .parse_mode(ParseMode::Html)
            .reply_markup(menu_keyboard())
            .await?;
        return Ok(());
    }

    if text.eq_ignore_ascii_case("网络") || text.eq_ignore_ascii_case("HTTP") {
        dialogue.update(DialogueState::AwaitingHttp).await?;
        bot.send_message(msg.chat.id, escape_html("发送 HTTP：METHOD URL [BODY]"))
            .parse_mode(ParseMode::Html)
            .await?;
        return Ok(());
    }

    if text.eq_ignore_ascii_case("任务") || text.eq_ignore_ascii_case("Tmux") {
        dialogue.update(DialogueState::AwaitingTmux).await?;
        bot.send_message(
            msg.chat.id,
            escape_html("Tmux：start <name> <cmd> | stop <name> | logs <name> [lines] | list"),
        )
        .parse_mode(ParseMode::Html)
        .await?;
        return Ok(());
    }

    if text.eq_ignore_ascii_case("定时") || text.eq_ignore_ascii_case("Schedule") {
        dialogue.update(DialogueState::AwaitingSchedule).await?;
        bot.send_message(
            msg.chat.id,
            escape_html(
                "定时：<cron_with_prefix> | msg <text> 或 shell <cmd> 或 http <METHOD> <URL> [BODY]。cron 必须以 rbot_ 或 rbot_system_ 开头。",
            ),
        )
        .parse_mode(ParseMode::Html)
        .await?;
        return Ok(());
    }

    if text.eq_ignore_ascii_case("白名单") || text.eq_ignore_ascii_case("Whitelist") {
        dialogue.update(DialogueState::AwaitingWhitelist).await?;
        bot.send_message(msg.chat.id, escape_html("白名单：<tool> <command>（仅管理员）"))
            .parse_mode(ParseMode::Html)
            .await?;
        return Ok(());
    }

    if text.eq_ignore_ascii_case("技能") || text.eq_ignore_ascii_case("Skills") {
        let mut out = String::from("Skills:\n");
        for skill in state.skills.list() {
            out.push_str(&format!("- {}: {}\n", skill.name, skill.description));
        }
        out.push_str("Use /skill <name> or /skill_off");
        bot.send_message(msg.chat.id, escape_html(&out))
            .parse_mode(ParseMode::Html)
            .await?;
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
        bot.send_message(msg.chat.id, escape_html(&out))
            .parse_mode(ParseMode::Html)
            .await?;
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
                bot.send_message(msg.chat.id, escape_html(&format!("skill activated: {}", name)))
                    .parse_mode(ParseMode::Html)
                    .await?;
            }
            Err(_) => {
                bot.send_message(msg.chat.id, escape_html("skill not found"))
                    .parse_mode(ParseMode::Html)
                    .await?;
            }
        }
        return Ok(());
    }

    if text.eq_ignore_ascii_case("/skill_off") {
        state.skills.deactivate(msg.chat.id.0);
        bot.send_message(msg.chat.id, escape_html("skill cleared"))
            .parse_mode(ParseMode::Html)
            .await?;
        return Ok(());
    }

    if text.starts_with("!shell ") {
        let cmd = text.trim_start_matches("!shell ").to_string();
        let out = state.tools.execute(ToolCall::Shell { cmd }).await;
        send_tool_output(&bot, msg.chat.id, out).await?;
        return Ok(());
    }

    if text.starts_with("!http ") {
        let parts: Vec<&str> = text.trim_start_matches("!http ").splitn(3, ' ').collect();
        if parts.len() < 2 {
            bot.send_message(msg.chat.id, escape_html("format: !http METHOD URL [BODY]"))
                .parse_mode(ParseMode::Html)
                .await?;
            return Ok(());
        }
        let method = parts[0].to_string();
        let url = parts[1].to_string();
        let body = parts.get(2).map(|s| s.to_string());
        let out = state.tools.execute(ToolCall::Http { method, url, body }).await;
        send_tool_output(&bot, msg.chat.id, out).await?;
        return Ok(());
    }

    if text.starts_with("!tmux ") {
        let cmd = text.trim_start_matches("!tmux ");
        let action = parse_tmux_action(cmd)?;
        let out = state.tools.execute(ToolCall::Tmux { action }).await;
        send_tool_output(&bot, msg.chat.id, out).await?;
        return Ok(());
    }

    if let Some(skill) = state.skills.maybe_trigger(msg.chat.id.0, text) {
        let _ = bot
            .send_message(msg.chat.id, escape_html(&format!("skill activated: {}", skill.name)))
            .parse_mode(ParseMode::Html)
            .await;
    }

    // LLM chat
    state.memory.append_message(msg.chat.id.0, "user", text)?;
    let response = chat_with_llm(&state, msg.chat.id.0, text).await;
    let reply = match response {
        Ok(r) => r,
        Err(err) => format!("llm error: {}", err),
    };
    state.memory.append_message(msg.chat.id.0, "assistant", &reply)?;
    let send = bot
        .send_message(msg.chat.id, reply.clone())
        .parse_mode(ParseMode::Html)
        .reply_markup(menu_keyboard())
        .await;
    if let Err(_) = send {
        let safe = escape_html(&reply);
        bot.send_message(msg.chat.id, safe)
            .parse_mode(ParseMode::Html)
            .reply_markup(menu_keyboard())
            .await?;
    }
    Ok(())
}

async fn handle_command(bot: AutoSend<Bot>, msg: Message, dialogue: MyDialogue, state: AppState) -> HandlerResult {
    let text = msg.text().unwrap_or("").trim();
    if text.eq_ignore_ascii_case("cancel") {
        dialogue.update(DialogueState::Idle).await?;
        bot.send_message(msg.chat.id, escape_html("Cancelled"))
            .parse_mode(ParseMode::Html)
            .reply_markup(menu_keyboard())
            .await?;
        return Ok(());
    }
    let out = state.tools.execute(ToolCall::Shell { cmd: text.to_string() }).await;
    send_tool_output(&bot, msg.chat.id, out).await?;
    dialogue.update(DialogueState::Idle).await?;
    Ok(())
}

async fn handle_http(bot: AutoSend<Bot>, msg: Message, dialogue: MyDialogue, state: AppState) -> HandlerResult {
    let text = msg.text().unwrap_or("").trim();
    if text.eq_ignore_ascii_case("cancel") {
        dialogue.update(DialogueState::Idle).await?;
        bot.send_message(msg.chat.id, escape_html("Cancelled"))
            .parse_mode(ParseMode::Html)
            .reply_markup(menu_keyboard())
            .await?;
        return Ok(());
    }
    let parts: Vec<&str> = text.splitn(3, ' ').collect();
    if parts.len() < 2 {
        bot.send_message(msg.chat.id, escape_html("format: METHOD URL [BODY]"))
            .parse_mode(ParseMode::Html)
            .await?;
        return Ok(());
    }
    let method = parts[0].to_string();
    let url = parts[1].to_string();
    let body = parts.get(2).map(|s| s.to_string());
    let out = state.tools.execute(ToolCall::Http { method, url, body }).await;
    send_tool_output(&bot, msg.chat.id, out).await?;
    dialogue.update(DialogueState::Idle).await?;
    Ok(())
}

async fn handle_tmux(bot: AutoSend<Bot>, msg: Message, dialogue: MyDialogue, state: AppState) -> HandlerResult {
    let text = msg.text().unwrap_or("").trim();
    if text.eq_ignore_ascii_case("cancel") {
        dialogue.update(DialogueState::Idle).await?;
        bot.send_message(msg.chat.id, escape_html("Cancelled"))
            .parse_mode(ParseMode::Html)
            .reply_markup(menu_keyboard())
            .await?;
        return Ok(());
    }
    let action = parse_tmux_action(text)?;
    let out = state.tools.execute(ToolCall::Tmux { action }).await;
    send_tool_output(&bot, msg.chat.id, out).await?;
    dialogue.update(DialogueState::Idle).await?;
    Ok(())
}

async fn handle_schedule(bot: AutoSend<Bot>, msg: Message, dialogue: MyDialogue, state: AppState) -> HandlerResult {
    let text = msg.text().unwrap_or("").trim();
    if text.eq_ignore_ascii_case("cancel") {
        dialogue.update(DialogueState::Idle).await?;
        bot.send_message(msg.chat.id, escape_html("Cancelled"))
            .parse_mode(ParseMode::Html)
            .reply_markup(menu_keyboard())
            .await?;
        return Ok(());
    }
    let parts: Vec<&str> = text.splitn(2, '|').collect();
    if parts.len() < 2 {
        bot.send_message(
            msg.chat.id,
            escape_html(
                "format: <cron_with_prefix> | msg <text> OR shell <cmd> OR http <METHOD> <URL> [BODY]. cron must start with rbot_ or rbot_system_.",
            ),
        )
        .parse_mode(ParseMode::Html)
        .await?;
        return Ok(());
    }
    let cron = parts[0].trim();
    let action_str = parts[1].trim();
    let action = parse_schedule_action(action_str)?;
    let id = state.scheduler.add_schedule(msg.chat.id.0, cron, action).await?;
    bot.send_message(msg.chat.id, escape_html(&format!("scheduled id {}", id)))
        .parse_mode(ParseMode::Html)
        .await?;
    dialogue.update(DialogueState::Idle).await?;
    Ok(())
}

async fn handle_whitelist(bot: AutoSend<Bot>, msg: Message, dialogue: MyDialogue, state: AppState) -> HandlerResult {
    let text = msg.text().unwrap_or("").trim();
    if text.eq_ignore_ascii_case("cancel") {
        dialogue.update(DialogueState::Idle).await?;
        bot.send_message(msg.chat.id, escape_html("Cancelled"))
            .parse_mode(ParseMode::Html)
            .reply_markup(menu_keyboard())
            .await?;
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
        bot.send_message(msg.chat.id, escape_html("admin only"))
            .parse_mode(ParseMode::Html)
            .await?;
        return Ok(());
    }
    let mut parts = text.trim_start_matches("/allow").trim().splitn(2, ' ');
    let tool = parts.next().unwrap_or("").trim();
    let cmd = parts.next().unwrap_or("").trim();
    if tool.is_empty() || cmd.is_empty() {
        bot.send_message(msg.chat.id, escape_html("format: /allow <tool> <command>"))
            .parse_mode(ParseMode::Html)
            .await?;
        return Ok(());
    }
    state.tools.extend_allowlist(tool, cmd, user_id)?;
    bot.send_message(msg.chat.id, escape_html(&format!("allowlist updated: {} {}", tool, cmd)))
        .parse_mode(ParseMode::Html)
        .await?;
    Ok(())
}

async fn chat_with_llm(state: &AppState, chat_id: i64, input: &str) -> anyhow::Result<String> {
    let llm = match &state.llm {
        Some(llm) => llm.clone(),
        None => anyhow::bail!("llm not configured"),
    };
    let mut messages = Vec::new();
    messages.push(LlmMessage {
        role: "system".into(),
        content: format!(
            "Persona:\n{}\n\nRules:\n- You can use tools (shell/http/tmux) proactively to achieve the goal.\n- If the user asks about local state you can check (time/IP/files/processes/ports), use tools directly; do not ask the user to run commands you can run.\n- For safe read-only actions, assume permission unless the user forbids it.\n- Maintain multi-turn context: track the user's goal, use memory, ask only necessary clarifying questions, and propose a next step when helpful.\n- Use official tool calls when tools are needed.\n- Format responses for Telegram HTML parse mode; use only supported tags (e.g., <b>, <i>, <code>, <pre>, <a>) and escape <, >, & in text.\n- Otherwise, respond normally.\n- Be concise and practical.",
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
    let mut attempts = 0;
    loop {
        attempts += 1;
        let reply = llm
            .chat(
                messages.clone(),
                ChatOptions {
                    temperature: 0.2,
                    tools: true,
                },
            )
            .await?;
        if !reply.tool_calls.is_empty() {
            messages.push(LlmMessage {
                role: "assistant".into(),
                content: reply.content.clone(),
                tool_call_id: None,
                tool_calls: Some(reply.tool_calls.clone()),
            });
            for call in reply.tool_calls {
                let tool_result = match tool_call_from_llm(&call) {
                    Ok(tool_call) => match state.tools.execute(tool_call).await {
                        Ok(out) => format!(
                            "TOOL_RESULT stdout:\n{}\n\nstderr:\n{}\ncode:{}",
                            out.stdout, out.stderr, out.exit_code
                        ),
                        Err(err) => format!("TOOL_RESULT error: {}", err),
                    },
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
            if attempts >= 4 {
                return Ok("Tool call limit reached. Please refine your request.".to_string());
            }
            continue;
        }
        if let Some(tool_call) = parse_tool_call(&reply.content)? {
            let tool_result = match state.tools.execute(tool_call).await {
                Ok(out) => format!(
                    "TOOL_RESULT stdout:\n{}\n\nstderr:\n{}\ncode:{}",
                    out.stdout, out.stderr, out.exit_code
                ),
                Err(err) => format!("TOOL_RESULT error: {}", err),
            };
            state.memory.append_message(chat_id, "tool", &tool_result)?;
            messages.push(LlmMessage {
                role: "assistant".into(),
                content: reply.content,
                tool_call_id: None,
                tool_calls: None,
            });
            messages.push(LlmMessage {
                role: "system".into(),
                content: tool_result,
                tool_call_id: None,
                tool_calls: None,
            });
            if attempts >= 4 {
                return Ok("Tool call limit reached. Please refine your request.".to_string());
            }
            continue;
        }
        return Ok(reply.content);
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

fn menu_keyboard() -> KeyboardMarkup {
    KeyboardMarkup::new(vec![
        vec![KeyboardButton::new("对话"), KeyboardButton::new("命令"), KeyboardButton::new("网络")],
        vec![KeyboardButton::new("任务"), KeyboardButton::new("定时"), KeyboardButton::new("记忆")],
        vec![KeyboardButton::new("白名单"), KeyboardButton::new("技能"), KeyboardButton::new("取消")],
    ])
    .resize_keyboard(true)
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
) -> HandlerResult {
    match out {
        Ok(output) => {
            let text = format_tool_output_html(&output);
            bot.send_message(chat_id, text)
                .parse_mode(ParseMode::Html)
                .await?;
        }
        Err(err) => {
            bot.send_message(chat_id, escape_html(&format!("tool error: {}", err)))
                .parse_mode(ParseMode::Html)
                .await?;
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

fn parse_tool_call(text: &str) -> anyhow::Result<Option<ToolCall>> {
    let first = match text.lines().next() {
        Some(l) => l.trim(),
        None => return Ok(None),
    };
    if !first.starts_with("TOOL") {
        return Ok(None);
    }
    let rest = first
        .trim_start_matches("TOOL")
        .trim_start_matches(':')
        .trim();
    if rest.is_empty() {
        anyhow::bail!("tool call missing args");
    }
    let mut parts = rest.splitn(2, ' ');
    let kind = parts.next().unwrap_or("");
    let args = parts.next().unwrap_or("").trim();
    match kind {
        "shell" => {
            if args.is_empty() {
                anyhow::bail!("shell args missing");
            }
            Ok(Some(ToolCall::Shell {
                cmd: args.to_string(),
            }))
        }
        "http" => {
            let http_parts: Vec<&str> = args.splitn(3, ' ').collect();
            if http_parts.len() < 2 {
                anyhow::bail!("http args missing");
            }
            Ok(Some(ToolCall::Http {
                method: http_parts[0].to_string(),
                url: http_parts[1].to_string(),
                body: http_parts.get(2).map(|s| s.to_string()),
            }))
        }
        "tmux" => {
            let action = parse_tmux_action(args)?;
            Ok(Some(ToolCall::Tmux { action }))
        }
        _ => Ok(None),
    }
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
        "tmux" => {
            #[derive(Deserialize)]
            struct TmuxArgs {
                action: String,
            }
            let parsed: TmuxArgs = serde_json::from_str(args)?;
            let action = parse_tmux_action(&parsed.action)?;
            Ok(ToolCall::Tmux { action })
        }
        _ => anyhow::bail!("unknown tool: {}", name),
    }
}
