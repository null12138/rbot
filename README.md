# rbot

Minimal Rust Telegram bot with LLM tool-calling, scheduler, tmux task management, streaming replies, and three-layer memory.

**Features**
- Telegram bot with fast streaming replies.
- Tool system: shell, HTTP, tmux, Tavily search, PDF text extraction.
- Scheduler: cron tasks + nightly sleep compaction.
- OpenAI-compatible LLM API client (streaming supported).
- Three-layer memory: short-term, daily summaries, long-term.
- Skills: load `skills/*.toml` and activate via `/skill <name>`.

**Quick Start (one-liner)**
macOS / Linux / FreeBSD:
```sh
curl -fsSL https://raw.githubusercontent.com/null12138/rbot/main/scripts/rbot.sh | sh -s -- install
```
Update:
```sh
curl -fsSL https://raw.githubusercontent.com/null12138/rbot/main/scripts/rbot.sh | sh -s -- update
```
Uninstall:
```sh
curl -fsSL https://raw.githubusercontent.com/null12138/rbot/main/scripts/rbot.sh | sh -s -- uninstall
```
Start in tmux:
```sh
curl -fsSL https://raw.githubusercontent.com/null12138/rbot/main/scripts/rbot.sh | sh -s -- start
```
Stop:
```sh
curl -fsSL https://raw.githubusercontent.com/null12138/rbot/main/scripts/rbot.sh | sh -s -- stop
```
Logs:
```sh
curl -fsSL https://raw.githubusercontent.com/null12138/rbot/main/scripts/rbot.sh | sh -s -- logs 200
```

Windows (PowerShell):
```powershell
irm https://raw.githubusercontent.com/null12138/rbot/main/scripts/rbot.ps1 | iex
```
Update:
```powershell
irm https://raw.githubusercontent.com/null12138/rbot/main/scripts/rbot.ps1 -OutFile $env:TEMP\rbot.ps1; & $env:TEMP\rbot.ps1 update
```
Uninstall:
```powershell
irm https://raw.githubusercontent.com/null12138/rbot/main/scripts/rbot.ps1 -OutFile $env:TEMP\rbot.ps1; & $env:TEMP\rbot.ps1 uninstall
```

The installer downloads the latest **tagged** release for your OS/arch, installs to `~/.rbot`, and runs `rbot init` (TUI).
Run the bot with:
```sh
rbot
```

**Environment Overrides**
- `RBOT_VERSION` (default: `latest` tag)
- `RBOT_REPO` (default: `null12138/rbot`)
- `RBOT_HOME` (default: `~/.rbot`)
- `RBOT_BIN_DIR` (default: `~/.local/bin` on Unix, `%LOCALAPPDATA%\\rbot\\bin` on Windows)
- `RBOT_TMUX_SESSION` (default: `rbot`)
- `RBOT_KEEP_CONFIG` (set to keep config/data when uninstalling)

**Config**
Path: `~/.rbot/config/config.toml`  
Run the TUI config wizard any time with `rbot init`.

**Tools & Security**
- Shell supports `allowlist` or `blocklist` mode:
  - `tools.shell.mode = "blocklist"` (default)
  - `tools.shell.blocklist` is always enforced
  - `tools.shell.allow_meta` defaults to `false`
  - `tools.shell.use_shell` defaults to `false`
- `/allow shell <cmd>` only applies in allowlist mode.

**Web Search Tool (Tavily)**
```
[tools.search]
api_key = ""         # Tavily API key
endpoint = ""        # optional override (default: https://api.tavily.com/search)
limit = 5
```

**Build From Source**
1. `cargo run -- init`
2. (Optional) copy `config/config.example.toml` -> `config/config.toml`
3. `cargo run`

**Troubleshooting**
- `command not found`: add `~/.local/bin` to your PATH or run `/home/<user>/.local/bin/rbot`.
- `Not Found` from Telegram: bot token invalid or whitespace in token.
- `TerminatedByOtherGetUpdates`: more than one bot instance is running.
