# rbot

Minimal Rust Telegram bot with LLM tool-calling, scheduler, tmux task management, streaming replies, and three-layer memory.

> 中文说明见下方 “中文使用说明”

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

---

# 中文使用说明

一个极简 Rust Telegram 机器人，支持 LLM 工具调用、计划任务、tmux 管理、流式回复和三层记忆。

**主要功能**
- Telegram 机器人 + 快速流式回复
- 工具：shell / HTTP / tmux / Tavily 搜索 / PDF 解析
- 定时任务：cron + 每日睡眠压缩
- OpenAI 兼容 LLM API（支持流式）
- 三层记忆：短期 / 日总结 / 长期
- 技能：`skills/*.toml`，可用 `/skill <name>` 激活

**一行安装（推荐）**
macOS / Linux / FreeBSD：
```sh
curl -fsSL https://raw.githubusercontent.com/null12138/rbot/main/scripts/rbot.sh | sh -s -- install
```
更新：
```sh
curl -fsSL https://raw.githubusercontent.com/null12138/rbot/main/scripts/rbot.sh | sh -s -- update
```
卸载：
```sh
curl -fsSL https://raw.githubusercontent.com/null12138/rbot/main/scripts/rbot.sh | sh -s -- uninstall
```
tmux 启动：
```sh
curl -fsSL https://raw.githubusercontent.com/null12138/rbot/main/scripts/rbot.sh | sh -s -- start
```
停止：
```sh
curl -fsSL https://raw.githubusercontent.com/null12138/rbot/main/scripts/rbot.sh | sh -s -- stop
```
查看日志：
```sh
curl -fsSL https://raw.githubusercontent.com/null12138/rbot/main/scripts/rbot.sh | sh -s -- logs 200
```

Windows（PowerShell）：
```powershell
irm https://raw.githubusercontent.com/null12138/rbot/main/scripts/rbot.ps1 | iex
```
更新：
```powershell
irm https://raw.githubusercontent.com/null12138/rbot/main/scripts/rbot.ps1 -OutFile $env:TEMP\rbot.ps1; & $env:TEMP\rbot.ps1 update
```
卸载：
```powershell
irm https://raw.githubusercontent.com/null12138/rbot/main/scripts/rbot.ps1 -OutFile $env:TEMP\rbot.ps1; & $env:TEMP\rbot.ps1 uninstall
```

安装脚本会下载 **固定 latest tag** 的发行包，安装到 `~/.rbot`，并启动 `rbot init`（TUI）。
运行：
```sh
rbot
```

**环境变量**
- `RBOT_VERSION`（默认 `latest` tag）
- `RBOT_REPO`（默认 `null12138/rbot`）
- `RBOT_HOME`（默认 `~/.rbot`）
- `RBOT_BIN_DIR`（默认 `~/.local/bin`；Windows 为 `%LOCALAPPDATA%\\rbot\\bin`）
- `RBOT_TMUX_SESSION`（默认 `rbot`）
- `RBOT_KEEP_CONFIG`（卸载时保留配置/数据）

**配置**
路径：`~/.rbot/config/config.toml`  
可随时执行 `rbot init` 重新生成/编辑。

**工具与安全**
Shell 支持黑名单或白名单：
- `tools.shell.mode = "blocklist"`（默认）
- `tools.shell.blocklist` 始终生效
- `tools.shell.allow_meta` 默认 `false`
- `tools.shell.use_shell` 默认 `false`
- `/allow shell <cmd>` 仅在 allowlist 模式有效

**Tavily 搜索配置**
```
[tools.search]
api_key = ""         # Tavily API key
endpoint = ""        # 可选覆盖（默认 https://api.tavily.com/search）
limit = 5
```

**源码构建**
1. `cargo run -- init`
2. （可选）复制 `config/config.example.toml` -> `config/config.toml`
3. `cargo run`

**常见问题**
- `command not found`：将 `~/.local/bin` 加入 PATH，或直接运行 `/home/<user>/.local/bin/rbot`。
- Telegram 报 `Not Found`：token 无效或有空格/换行。
- `TerminatedByOtherGetUpdates`：同一 bot 同时启动了多个实例。
