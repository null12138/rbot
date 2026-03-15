# rbot

Minimal Rust bot with Telegram UI, tool calling, scheduling, LLM, tmux task management, and three-layer memory.

## Features
- Telegram bot with menu and simple dialogues.
- Tool system: shell, HTTP, tmux. All gated by allowlist + danger regex.
- Scheduler: cron tasks + nightly sleep compaction.
- OpenAI-compatible LLM API client.
- Three-layer memory: short-term (recent turns), mid-term (daily summaries), long-term (MEMORY.md + SQLite).
- Skills: load `skills/*.toml` and activate via `/skill <name>`.

## Quick Start
1. Run config wizard:
   - `cargo run -- init`
1. Copy config:
   - `config/config.example.toml` -> `config/config.toml`
2. Fill in tokens and allowlists.
3. Run:
   - `cargo run`

## Proxy Fallback
By default rbot runs without proxy. If Telegram API is unreachable and `network.proxy_url`
is set (or `RBOT_PROXY` is provided), it will retry with the proxy automatically.

## Persona
Edit `config/persona.md` to adjust the assistant's tone and constraints.

## Telegram Commands
- `/start` or `/menu`: show menu
- `/skill <name>`: activate skill
- `/skill_off`: deactivate
- `/allow <tool> <command>`: extend allowlist (admin only)

## Menu
- Chat
- Run (shell)
- HTTP
- Tmux
- Schedule
- Memory
- Whitelist
- Skills

## Scheduling
Input format (cron must start with `rbot_` or `rbot_system_`):
```
<cron_with_prefix> | msg <text>
<cron_with_prefix> | shell <cmd>
<cron_with_prefix> | http <METHOD> <URL> [BODY]
```
Cron uses `cron` crate (seconds supported). Example:
```
rbot_0 */5 * * * * | msg heartbeat
```

## Sleep Compaction
Nightly task (`memory.sleep_time`) condenses the day log:
- Retained items -> `memory/<chat_id>/MEMORY.md`
- Other items -> `memory/<chat_id>/sleep/YYYY-MM-DD.md`

## Heartbeat
Writes `memory/heartbeat.txt` every `scheduler.heartbeat_interval_secs`.
