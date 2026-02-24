use crate::core::{config, pricing, receipt::Receipt, redact, transcript};
use crate::commands::staging;
use chrono::Utc;
use sha2::{Digest, Sha256};
use std::io::Read;

#[derive(Debug)]
struct HookInput {
    transcript_path: Option<String>,
    cwd: Option<String>,
    hook_event_name: Option<String>,
    file_path: Option<String>,
}

fn parse_hook_input(json_str: &str) -> HookInput {
    let v: serde_json::Value = serde_json::from_str(json_str).unwrap_or(serde_json::Value::Null);
    HookInput {
        transcript_path: v.get("transcript_path").and_then(|v| v.as_str()).map(String::from),
        cwd: v.get("cwd").and_then(|v| v.as_str()).map(String::from),
        hook_event_name: v.get("hook_event_name").and_then(|v| v.as_str()).map(String::from),
        file_path: v
            .get("tool_input")
            .and_then(|ti| ti.get("file_path"))
            .and_then(|v| v.as_str())
            .map(String::from),
    }
}

fn get_changed_lines(cwd: &str, file_path: &str) -> (u32, u32) {
    let output = std::process::Command::new("git")
        .current_dir(cwd)
        .args(["diff", "--unified=0", file_path])
        .output();

    match output {
        Ok(o) => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            let mut start = 0u32;
            let mut end = 0u32;
            for line in stdout.lines() {
                if line.starts_with("@@") {
                    // Parse @@ -a,b +c,d @@
                    if let Some(plus_part) = line.split('+').nth(1) {
                        let nums: &str = plus_part.split(' ').next().unwrap_or("0");
                        let parts: Vec<&str> = nums.split(',').collect();
                        let line_start: u32 = parts.first().and_then(|s| s.parse().ok()).unwrap_or(0);
                        let count: u32 = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(1);
                        if start == 0 || line_start < start {
                            start = line_start;
                        }
                        let line_end = if count == 0 { line_start } else { line_start + count - 1 };
                        if line_end > end {
                            end = line_end;
                        }
                    }
                }
            }
            if start == 0 {
                (1, 1)
            } else {
                (start, end)
            }
        }
        Err(_) => (1, 1),
    }
}

fn get_git_user() -> String {
    let name = std::process::Command::new("git")
        .args(["config", "user.name"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let email = std::process::Command::new("git")
        .args(["config", "user.email"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown@unknown".to_string());

    format!("{} <{}>", name, email)
}

pub fn run(agent: &str, hook_input_source: &str) {
    // Read from stdin
    let json_str = if hook_input_source == "stdin" {
        let mut buf = String::new();
        if std::io::stdin().read_to_string(&mut buf).is_err() {
            return; // Silent failure
        }
        buf
    } else {
        hook_input_source.to_string()
    };

    let input = parse_hook_input(&json_str);

    // PreToolUse: return early
    if input.hook_event_name.as_deref() == Some("PreToolUse") {
        return;
    }

    // Need transcript path for PostToolUse
    let transcript_path = match input.transcript_path {
        Some(p) => p,
        None => return,
    };

    let cwd = input.cwd.unwrap_or_else(|| ".".to_string());

    // Parse JSONL transcript
    let parsed = match transcript::parse_claude_jsonl(&transcript_path) {
        Ok(p) => p,
        Err(_) => return, // Silent failure
    };

    let cfg = config::load_config();

    let model = parsed.model.unwrap_or_else(|| "unknown".to_string());

    let prompt_summary = if cfg.capture.store_full_conversation {
        let full = transcript::full_conversation_text(&parsed.transcript);
        let truncated: String = full.chars().take(cfg.capture.max_prompt_length).collect();
        redact::redact_secrets_with_config(&truncated, &cfg)
    } else {
        transcript::first_user_prompt(&parsed.transcript)
            .map(|p| {
                let truncated: String = p.chars().take(cfg.capture.max_prompt_length).collect();
                redact::redact_secrets_with_config(&truncated, &cfg)
            })
            .unwrap_or_default()
    };

    let full_text = transcript::full_conversation_text(&parsed.transcript);
    let mut hasher = Sha256::new();
    hasher.update(full_text.as_bytes());
    let prompt_hash = format!("sha256:{:x}", hasher.finalize());

    // Estimate cost
    let total_chars: usize = parsed.transcript.messages.iter().map(|m| match m {
        transcript::Message::User { text, .. } => text.len(),
        transcript::Message::Assistant { text, .. } => text.len(),
        transcript::Message::ToolUse { .. } => 0,
    }).sum();
    let estimated_tokens = pricing::estimate_tokens_from_chars(total_chars);
    let cost = pricing::estimate_cost(&model, estimated_tokens / 2, estimated_tokens / 2);

    // Extract conversation turns (the chain of thought)
    let conversation_turns = transcript::extract_conversation_turns(
        &parsed.transcript,
        cfg.capture.max_prompt_length,
        &|text| redact::redact_secrets_with_config(text, &cfg),
    );

    let user = get_git_user();
    let message_count = parsed.transcript.messages.len() as u32;

    // Determine which files to create receipts for
    let target_file = input.file_path.clone();
    let files: Vec<String> = if let Some(f) = target_file {
        vec![f]
    } else {
        parsed.files_modified.clone()
    };

    // Read existing staging to find parent receipts for chaining
    let existing_staging = staging::read_staging();

    for file_path in &files {
        let line_range = get_changed_lines(&cwd, file_path);

        // Find parent receipt: same file + same session in staging
        let parent_id = existing_staging
            .receipts
            .iter()
            .rev()
            .find(|r| r.file_path == *file_path && r.session_id == parsed.session_id)
            .map(|r| r.id.clone());

        let receipt = Receipt {
            id: Receipt::new_id(),
            provider: agent.to_string(),
            model: model.clone(),
            session_id: parsed.session_id.clone(),
            prompt_summary: prompt_summary.clone(),
            prompt_hash: prompt_hash.clone(),
            message_count,
            cost_usd: cost,
            timestamp: Utc::now(),
            session_start: parsed.session_start,
            session_end: parsed.session_end,
            session_duration_secs: parsed.session_duration_secs,
            ai_response_time_secs: parsed.avg_response_time_secs,
            user: user.clone(),
            file_path: file_path.clone(),
            line_range,
            parent_receipt_id: parent_id,
            conversation: if conversation_turns.is_empty() { None } else { Some(conversation_turns.clone()) },
        };

        staging::add_receipt(&receipt);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hook_input() {
        let json = r#"{"transcript_path":"/tmp/test.jsonl","cwd":"/tmp","hook_event_name":"PostToolUse","tool_input":{"file_path":"src/main.rs"}}"#;
        let input = parse_hook_input(json);
        assert_eq!(input.transcript_path.unwrap(), "/tmp/test.jsonl");
        assert_eq!(input.cwd.unwrap(), "/tmp");
        assert_eq!(input.hook_event_name.unwrap(), "PostToolUse");
        assert_eq!(input.file_path.unwrap(), "src/main.rs");
    }

    #[test]
    fn test_parse_hook_input_missing_fields() {
        let json = r#"{}"#;
        let input = parse_hook_input(json);
        assert!(input.transcript_path.is_none());
        assert!(input.file_path.is_none());
    }
}
