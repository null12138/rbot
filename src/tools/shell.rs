use crate::tools::{ToolError, ToolOutput, ToolRegistry};
use shell_words::split;
use tokio::process::Command;

fn reject_meta(cmd: &str) -> Result<(), ToolError> {
    let meta = [";", "&&", "||", "|", "`", "$(", "${", "<", ">"];
    for m in meta {
        if cmd.contains(m) {
            return Err(ToolError::Dangerous);
        }
    }
    Ok(())
}

pub async fn execute_shell(cmd: String, registry: &ToolRegistry) -> Result<ToolOutput, ToolError> {
    registry.guard.check(&cmd)?;
    if !registry.shell_allow_meta {
        reject_meta(&cmd)?;
    }

    let use_shell = registry.shell_use_shell;
    let mut program = String::new();
    let mut args: Vec<String> = Vec::new();

    if !use_shell {
        let parts = split(&cmd).map_err(|e| ToolError::InvalidInput(e.to_string()))?;
        let (prog, rest) = parts
            .split_first()
            .ok_or_else(|| ToolError::InvalidInput("empty command".into()))?;
        program = prog.to_string();
        args = rest.iter().map(|s| s.to_string()).collect();
    }

    if !registry.shell_allow_all {
        if use_shell {
            let parts = split(&cmd).map_err(|e| ToolError::InvalidInput(e.to_string()))?;
            let (prog, _) = parts
                .split_first()
                .ok_or_else(|| ToolError::InvalidInput("empty command".into()))?;
            let allowed = registry.shell_allowlist.read().unwrap().contains(prog);
            if !allowed {
                return Err(ToolError::NotAllowed);
            }
        } else {
            let allowed = registry.shell_allowlist.read().unwrap().contains(&program);
            if !allowed {
                return Err(ToolError::NotAllowed);
            }
        }
    }

    let output = if use_shell {
        Command::new("sh")
            .arg("-lc")
            .arg(cmd)
            .output()
            .await
            .map_err(|e| ToolError::Execution(e.to_string()))?
    } else {
        Command::new(program)
            .args(args)
            .output()
            .await
            .map_err(|e| ToolError::Execution(e.to_string()))?
    };

    let stdout = truncate_bytes(&output.stdout, 4000);
    let stderr = truncate_bytes(&output.stderr, 2000);
    Ok(ToolOutput {
        stdout,
        stderr,
        exit_code: output.status.code().unwrap_or(-1),
    })
}

pub(crate) fn truncate_bytes(data: &[u8], max: usize) -> String {
    let mut s = String::from_utf8_lossy(data).to_string();
    if s.len() > max {
        s.truncate(max);
        s.push_str("\n...[truncated]");
    }
    s
}
