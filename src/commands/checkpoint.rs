use crate::commands::staging;
use crate::core::{
    config, pricing,
    receipt::{FileChange, Receipt},
    redact, transcript,
};
use chrono::Utc;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;
use transcript::{extract_agents_spawned, extract_mcp_servers, extract_tools_used};

#[derive(Debug)]
struct HookInput {
    transcript_path: Option<String>,
    cwd: Option<String>,
    hook_event_name: Option<String>,
    tool_name: Option<String>,
    file_path: Option<String>,
}

fn parse_hook_input(json_str: &str) -> HookInput {
    let v: serde_json::Value = serde_json::from_str(json_str).unwrap_or(serde_json::Value::Null);
    HookInput {
        transcript_path: v
            .get("transcript_path")
            .and_then(|v| v.as_str())
            .map(String::from),
        cwd: v.get("cwd").and_then(|v| v.as_str()).map(String::from),
        hook_event_name: v
            .get("hook_event_name")
            .and_then(|v| v.as_str())
            .map(String::from),
        tool_name: v
            .get("tool_name")
            .and_then(|v| v.as_str())
            .map(String::from),
        file_path: v
            .get("tool_input")
            .and_then(|ti| ti.get("file_path"))
            .and_then(|v| v.as_str())
            .map(String::from),
    }
}

/// Convert an absolute path to a path relative to `base`.
/// If the path is already relative or doesn't start with base, return as-is.
fn make_relative(path: &str, base: &str) -> String {
    let path = path.trim();
    let base = base.trim_end_matches('/');
    if base.is_empty() || base == "." {
        return path.to_string();
    }
    if let Some(rel) = path.strip_prefix(base) {
        let rel = rel.strip_prefix('/').unwrap_or(rel);
        if rel.is_empty() {
            return path.to_string();
        }
        return rel.to_string();
    }
    path.to_string()
}

/// Parse diff hunk headers to extract changed line ranges.
fn parse_diff_hunks(diff_output: &str) -> (u32, u32) {
    let mut start = 0u32;
    let mut end = 0u32;
    for line in diff_output.lines() {
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
                let line_end = if count == 0 {
                    line_start
                } else {
                    line_start + count - 1
                };
                if line_end > end {
                    end = line_end;
                }
            }
        }
    }
    if start == 0 {
        (0, 0)
    } else {
        (start, end)
    }
}

/// Try to detect changed lines using multiple git diff strategies.
fn get_changed_lines(cwd: &str, file_path: &str) -> (u32, u32) {
    // Strategy 1: Unstaged changes (git diff)
    if let Ok(o) = std::process::Command::new("git")
        .current_dir(cwd)
        .args(["diff", "--unified=0", "--", file_path])
        .output()
    {
        let stdout = String::from_utf8_lossy(&o.stdout);
        let (start, end) = parse_diff_hunks(&stdout);
        if start > 0 {
            return (start, end);
        }
    }

    // Strategy 2: Staged changes (git diff --cached)
    if let Ok(o) = std::process::Command::new("git")
        .current_dir(cwd)
        .args(["diff", "--cached", "--unified=0", "--", file_path])
        .output()
    {
        let stdout = String::from_utf8_lossy(&o.stdout);
        let (start, end) = parse_diff_hunks(&stdout);
        if start > 0 {
            return (start, end);
        }
    }

    // Strategy 3: Diff against HEAD (catches both staged + unstaged)
    if let Ok(o) = std::process::Command::new("git")
        .current_dir(cwd)
        .args(["diff", "HEAD", "--unified=0", "--", file_path])
        .output()
    {
        let stdout = String::from_utf8_lossy(&o.stdout);
        let (start, end) = parse_diff_hunks(&stdout);
        if start > 0 {
            return (start, end);
        }
    }

    // Strategy 4: Count total lines in file as fallback
    let full_path = if std::path::Path::new(file_path).is_absolute() {
        file_path.to_string()
    } else {
        format!("{}/{}", cwd.trim_end_matches('/'), file_path)
    };
    if let Ok(content) = std::fs::read_to_string(&full_path) {
        let line_count = content.lines().count() as u32;
        if line_count > 0 {
            return (1, line_count);
        }
    }

    (1, 1)
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
        if let Err(e) = std::io::stdin().read_to_string(&mut buf) {
            eprintln!("[BlamePrompt] Failed to read hook input from stdin: {}", e);
            return;
        }
        buf
    } else {
        hook_input_source.to_string()
    };

    let input = parse_hook_input(&json_str);

    match input.hook_event_name.as_deref() {
        Some("PostToolUse") => {
            match input.tool_name.as_deref() {
                Some("Write" | "Edit" | "MultiEdit") => {
                    handle_file_change(agent, &input);
                }
                _ => {} // skip non-writing tools
            }
        }
        Some("Stop") => {
            // Create receipts for prompts that didn't modify any files.
            // Stop fires after the assistant finishes responding to each prompt,
            // so any PostToolUse receipts for file changes have already been written.
            handle_stop(agent, &input);
        }
        _ => {} // skip all other events
    }
}

/// Build common transcript context used by both file-change and stop handlers.
struct TranscriptContext {
    parsed: transcript::TranscriptParseResult,
    cwd: String,
    cfg: config::BlamePromptConfig,
    model: String,
    prompt_hash: String,
    cost: f64,
    user: String,
    message_count: u32,
}

fn build_context(input: &HookInput) -> Option<TranscriptContext> {
    let transcript_path = input.transcript_path.as_ref()?;
    let cwd = input.cwd.clone().unwrap_or_else(|| ".".to_string());

    let parsed = transcript::parse_claude_jsonl(transcript_path).ok()?;
    let cfg = config::load_config();
    let model = parsed
        .model
        .clone()
        .unwrap_or_else(|| "unknown".to_string());

    let full_text = transcript::full_conversation_text(&parsed.transcript);
    let mut hasher = Sha256::new();
    hasher.update(full_text.as_bytes());
    let prompt_hash = format!("sha256:{:x}", hasher.finalize());

    let total_chars: usize = parsed
        .transcript
        .messages
        .iter()
        .map(|m| match m {
            transcript::Message::User { text, .. } => text.len(),
            transcript::Message::Assistant { text, .. } => text.len(),
            transcript::Message::ToolUse { .. } => 0,
        })
        .sum();
    let estimated_tokens = pricing::estimate_tokens_from_chars(total_chars);
    let cost = pricing::estimate_cost(&model, estimated_tokens / 2, estimated_tokens / 2);
    let user = get_git_user();
    let message_count = parsed.transcript.messages.len() as u32;

    Some(TranscriptContext {
        parsed,
        cwd,
        cfg,
        model,
        prompt_hash,
        cost,
        user,
        message_count,
    })
}

/// Handle PostToolUse for Write/Edit/MultiEdit — creates a receipt with file changes.
fn handle_file_change(agent: &str, input: &HookInput) {
    let ctx = match build_context(input) {
        Some(c) => c,
        None => return,
    };

    let prompt_number = transcript::count_user_prompts(&ctx.parsed.transcript);

    let mut conversation_turns = transcript::extract_conversation_turns(
        &ctx.parsed.transcript,
        ctx.cfg.capture.max_prompt_length,
        &|text| redact::redact_secrets_with_config(text, &ctx.cfg),
    );
    for turn in &mut conversation_turns {
        if let Some(ref mut files) = turn.files_touched {
            *files = files.iter().map(|f| make_relative(f, &ctx.cwd)).collect();
        }
    }

    // Get the file being modified by this tool use
    let file = match input.file_path {
        Some(ref f) => {
            let rel = make_relative(f, &ctx.cwd);
            if rel.starts_with(".claude/") || rel.contains("/tool-results/") {
                return;
            }
            rel
        }
        None => return,
    };

    let files_changed = vec![FileChange {
        path: file.clone(),
        line_range: get_changed_lines(&ctx.cwd, &file),
    }];

    let prompt_summary = transcript::last_user_prompt(&ctx.parsed.transcript)
        .map(|p| {
            let truncated: String = p.chars().take(ctx.cfg.capture.max_prompt_length).collect();
            redact::redact_secrets_with_config(&truncated, &ctx.cfg)
        })
        .unwrap_or_default();

    let receipt = Receipt {
        id: Receipt::new_id(),
        provider: agent.to_string(),
        model: ctx.model,
        session_id: ctx.parsed.session_id,
        prompt_summary,
        prompt_hash: ctx.prompt_hash,
        message_count: ctx.message_count,
        cost_usd: ctx.cost,
        timestamp: Utc::now(),
        session_start: ctx.parsed.session_start,
        session_end: ctx.parsed.session_end,
        session_duration_secs: ctx.parsed.session_duration_secs,
        ai_response_time_secs: ctx.parsed.avg_response_time_secs,
        user: ctx.user,
        file_path: files_changed
            .first()
            .map(|f| f.path.clone())
            .unwrap_or_default(),
        line_range: files_changed
            .first()
            .map(|f| f.line_range)
            .unwrap_or((0, 0)),
        files_changed,
        parent_receipt_id: None,
        prompt_number: Some(prompt_number),
        tools_used: extract_tools_used(&ctx.parsed.transcript),
        mcp_servers: extract_mcp_servers(&ctx.parsed.transcript),
        agents_spawned: extract_agents_spawned(&ctx.parsed.transcript),
        conversation: if conversation_turns.is_empty() {
            None
        } else {
            Some(conversation_turns)
        },
    };

    staging::upsert_receipt_in(&receipt, &ctx.cwd);
}

/// Handle Stop event — ensure every user prompt in the session has a receipt,
/// even if no files were modified (e.g. a simple "hi" prompt).
fn handle_stop(agent: &str, input: &HookInput) {
    let ctx = match build_context(input) {
        Some(c) => c,
        None => return,
    };

    let total_prompts = transcript::count_user_prompts(&ctx.parsed.transcript);
    if total_prompts == 0 {
        return;
    }

    // Read existing staging to find which prompts already have receipts
    let existing = staging::read_staging_in(Path::new(&ctx.cwd));
    let existing_prompt_numbers: Vec<Option<u32>> = existing
        .receipts
        .iter()
        .filter(|r| r.session_id == ctx.parsed.session_id)
        .map(|r| r.prompt_number)
        .collect();

    // Create receipts for any prompts that don't have one yet
    for pn in 1..=total_prompts {
        if existing_prompt_numbers.contains(&Some(pn)) {
            continue; // Already has a receipt from PostToolUse
        }

        let prompt_summary = transcript::nth_user_prompt(&ctx.parsed.transcript, pn)
            .map(|p| {
                let truncated: String = p.chars().take(ctx.cfg.capture.max_prompt_length).collect();
                redact::redact_secrets_with_config(&truncated, &ctx.cfg)
            })
            .unwrap_or_default();

        let receipt = Receipt {
            id: Receipt::new_id(),
            provider: agent.to_string(),
            model: ctx.model.clone(),
            session_id: ctx.parsed.session_id.clone(),
            prompt_summary,
            prompt_hash: ctx.prompt_hash.clone(),
            message_count: ctx.message_count,
            cost_usd: ctx.cost,
            timestamp: Utc::now(),
            session_start: ctx.parsed.session_start,
            session_end: ctx.parsed.session_end,
            session_duration_secs: ctx.parsed.session_duration_secs,
            ai_response_time_secs: ctx.parsed.avg_response_time_secs,
            user: ctx.user.clone(),
            file_path: String::new(),
            line_range: (0, 0),
            files_changed: vec![],
            parent_receipt_id: None,
            prompt_number: Some(pn),
            tools_used: extract_tools_used(&ctx.parsed.transcript),
            mcp_servers: extract_mcp_servers(&ctx.parsed.transcript),
            agents_spawned: extract_agents_spawned(&ctx.parsed.transcript),
            conversation: None, // No conversation for non-file prompts to save space
        };

        staging::upsert_receipt_in(&receipt, &ctx.cwd);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hook_input() {
        let json = r#"{"transcript_path":"/tmp/test.jsonl","cwd":"/tmp","hook_event_name":"PostToolUse","tool_name":"Write","tool_input":{"file_path":"src/main.rs"}}"#;
        let input = parse_hook_input(json);
        assert_eq!(input.transcript_path.unwrap(), "/tmp/test.jsonl");
        assert_eq!(input.cwd.unwrap(), "/tmp");
        assert_eq!(input.hook_event_name.unwrap(), "PostToolUse");
        assert_eq!(input.tool_name.unwrap(), "Write");
        assert_eq!(input.file_path.unwrap(), "src/main.rs");
    }

    #[test]
    fn test_parse_hook_input_missing_fields() {
        let json = r#"{}"#;
        let input = parse_hook_input(json);
        assert!(input.transcript_path.is_none());
        assert!(input.file_path.is_none());
    }

    #[test]
    fn test_parse_diff_hunks() {
        let diff = "@@ -1,3 +1,5 @@\n some code\n@@ -10,2 +12,4 @@\n more code\n";
        let (start, end) = parse_diff_hunks(diff);
        assert_eq!(start, 1);
        assert_eq!(end, 15); // 12 + 4 - 1
    }

    #[test]
    fn test_parse_diff_hunks_empty() {
        let (start, end) = parse_diff_hunks("");
        assert_eq!(start, 0);
        assert_eq!(end, 0);
    }

    #[test]
    fn test_make_relative() {
        assert_eq!(
            make_relative("/home/user/project/src/main.rs", "/home/user/project"),
            "src/main.rs"
        );
        assert_eq!(
            make_relative("src/main.rs", "/home/user/project"),
            "src/main.rs"
        );
        assert_eq!(
            make_relative("/other/path/file.rs", "/home/user/project"),
            "/other/path/file.rs"
        );
    }
}
