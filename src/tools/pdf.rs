use crate::tools::{ToolError, ToolOutput};

pub async fn extract_pdf_text(path: String, max_chars: Option<usize>) -> Result<ToolOutput, ToolError> {
    let cap = max_chars.unwrap_or(12000).clamp(1000, 50000);
    let text = pdf_extract::extract_text(std::path::Path::new(&path))
        .map_err(|e| ToolError::Execution(e.to_string()))?;
    let out = truncate_safe(text, cap);
    Ok(ToolOutput {
        stdout: out,
        stderr: String::new(),
        exit_code: 0,
    })
}

fn truncate_safe(mut s: String, max: usize) -> String {
    if s.len() > max {
        let mut end = max.min(s.len());
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        s.truncate(end);
        s.push_str("\n...[truncated]");
    }
    s
}
