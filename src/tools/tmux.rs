use crate::tools::{shell, ToolError, ToolOutput, ToolRegistry};
use serde::{Deserialize, Serialize};
use shell_words::split;
use tokio::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TmuxAction {
    Start { session: String, cmd: String },
    List,
    Stop { session: String },
    Logs { session: String, lines: usize },
}

pub async fn execute_tmux(action: TmuxAction, registry: &ToolRegistry) -> Result<ToolOutput, ToolError> {
    match action {
        TmuxAction::Start { session, cmd } => {
            registry.guard.check(&cmd)?;
            ensure_allowed_subcommand("new-session", registry)?;
            validate_session(&session)?;
            let parts = split(&cmd).map_err(|e| ToolError::InvalidInput(e.to_string()))?;
            let (program, args) = parts.split_first().ok_or_else(|| ToolError::InvalidInput("empty command".into()))?;
            let allowed = registry.shell_allowlist.read().unwrap().contains(program);
            if !allowed {
                return Err(ToolError::NotAllowed);
            }
            let mut command = Command::new("tmux");
            command.args(["new-session", "-d", "-s", &session, program]);
            command.args(args);
            run_tmux(command).await
        }
        TmuxAction::List => {
            ensure_allowed_subcommand("list-sessions", registry)?;
            let mut command = Command::new("tmux");
            command.args(["list-sessions"]);
            run_tmux(command).await
        }
        TmuxAction::Stop { session } => {
            ensure_allowed_subcommand("kill-session", registry)?;
            validate_session(&session)?;
            let mut command = Command::new("tmux");
            command.args(["kill-session", "-t", &session]);
            run_tmux(command).await
        }
        TmuxAction::Logs { session, lines } => {
            ensure_allowed_subcommand("capture-pane", registry)?;
            validate_session(&session)?;
            let start = format!("-{}", lines.max(10));
            let mut command = Command::new("tmux");
            command.args(["capture-pane", "-p", "-t", &session, "-S", &start]);
            run_tmux(command).await
        }
    }
}

fn ensure_allowed_subcommand(sub: &str, registry: &ToolRegistry) -> Result<(), ToolError> {
    if registry.tmux_allow_all {
        return Ok(());
    }
    let allowed = registry.tmux_allowlist.read().unwrap().contains(sub);
    if !allowed {
        return Err(ToolError::NotAllowed);
    }
    Ok(())
}

fn validate_session(session: &str) -> Result<(), ToolError> {
    if session.is_empty() || session.len() > 64 {
        return Err(ToolError::InvalidInput("invalid session".into()));
    }
    if !session.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(ToolError::InvalidInput("invalid session".into()));
    }
    Ok(())
}

async fn run_tmux(mut command: Command) -> Result<ToolOutput, ToolError> {
    let output = command.output().await.map_err(|e| ToolError::Execution(e.to_string()))?;
    let stdout = shell::truncate_bytes(&output.stdout, 4000);
    let stderr = shell::truncate_bytes(&output.stderr, 2000);
    Ok(ToolOutput {
        stdout,
        stderr,
        exit_code: output.status.code().unwrap_or(-1),
    })
}
