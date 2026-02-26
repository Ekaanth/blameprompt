/// OpenAI Codex CLI integration for blameprompt.
///
/// Imports Codex CLI session transcripts and converts them to blameprompt receipts.
/// Also supports installing hooks into Codex CLI's config.toml.
///
/// Codex CLI stores session data in:
///   ~/.codex/sessions/ (JSONL rollout files)
///   ~/.codex/archived_sessions/ (archived JSONL)
///   ~/.local/share/codex/sessions/ (Linux XDG)
///
/// Hook integration:
///   Modifies ~/.codex/config.toml to add a `notify` entry.
use crate::commands::staging;
use crate::core::{config, receipt::Receipt, util};
use chrono::{DateTime, Utc};
use std::path::{Path, PathBuf};

/// A parsed Codex CLI session.
#[derive(Debug)]
pub struct CodexSession {
    pub session_id: String,
    pub model: String,
    pub messages: Vec<CodexMessage>,
    pub files_modified: Vec<String>,
    pub tools_used: Vec<String>,
    pub timestamp: DateTime<Utc>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

#[derive(Debug)]
pub struct CodexMessage {
    pub role: String,
    pub text: String,
}

/// Locate the Codex CLI home directory.
fn codex_home() -> Option<PathBuf> {
    // Respect CODEX_HOME env var (used by Codex CLI)
    if let Ok(home) = std::env::var("CODEX_HOME") {
        let p = PathBuf::from(home);
        if p.exists() {
            return Some(p);
        }
    }
    let home = dirs::home_dir()?;
    let primary = home.join(".codex");
    if primary.exists() {
        return Some(primary);
    }
    // Linux XDG
    let xdg = home.join(".local/share/codex");
    if xdg.exists() {
        return Some(xdg);
    }
    if let Ok(data_home) = std::env::var("XDG_DATA_HOME") {
        let xdg_custom = PathBuf::from(data_home).join("codex");
        if xdg_custom.exists() {
            return Some(xdg_custom);
        }
    }
    None
}

/// Locate all Codex CLI session directories (active + archived).
pub fn find_sessions_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(codex) = codex_home() {
        let sessions = codex.join("sessions");
        if sessions.exists() {
            dirs.push(sessions);
        }
        let archived = codex.join("archived_sessions");
        if archived.exists() {
            dirs.push(archived);
        }
    }
    dirs
}

/// List session files across all session directories, sorted by most recent.
/// Follows git-ai's pattern of looking for rollout-*.jsonl files.
pub fn list_session_files(sessions_dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = Vec::new();
    collect_session_files_recursive(sessions_dir, &mut files);
    files.sort_by_key(|f| {
        std::fs::metadata(f)
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
    });
    files.reverse();
    files
}

fn collect_session_files_recursive(dir: &Path, files: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_session_files_recursive(&path, files);
        } else if path.extension().is_some_and(|e| e == "jsonl" || e == "json") {
            files.push(path);
        }
    }
}

/// Parse a Codex CLI session transcript file.
/// Handles multiple JSONL entry types:
///   - `turn_context`: model metadata and context
///   - `response_item`: messages and tool calls
///   - `event_msg`: legacy event format
///   - Standard `role`/`content` entries
pub fn parse_codex_session(path: &Path) -> Option<CodexSession> {
    let content = std::fs::read_to_string(path).ok()?;
    let mut messages = Vec::new();
    let mut model = String::new();
    let mut files_modified = Vec::new();
    let mut tools_used = Vec::new();
    let mut first_ts: Option<DateTime<Utc>> = None;
    let mut input_tokens: u64 = 0;
    let mut output_tokens: u64 = 0;

    // Try JSONL format (one JSON object per line)
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let entry: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        // Extract timestamp
        if first_ts.is_none() {
            if let Some(ts_str) = entry.get("timestamp").and_then(|v| v.as_str()) {
                first_ts = DateTime::parse_from_rfc3339(ts_str)
                    .ok()
                    .map(|dt| dt.with_timezone(&Utc));
            }
        }

        let entry_type = entry.get("type").and_then(|v| v.as_str()).unwrap_or("");

        // Handle Codex-specific entry types (from git-ai's CodexPreset)
        match entry_type {
            // turn_context contains model metadata
            "turn_context" => {
                if model.is_empty() {
                    if let Some(m) = entry.get("model").and_then(|v| v.as_str()) {
                        model = m.to_string();
                    }
                }
            }
            // response_item contains messages and tool calls
            "response_item" => {
                let role = entry.get("role").and_then(|v| v.as_str()).unwrap_or("");
                // Content can be a string or array of text blocks
                let text = extract_codex_content(&entry);
                if !text.is_empty() && (role == "user" || role == "assistant") {
                    messages.push(CodexMessage {
                        role: role.to_string(),
                        text,
                    });
                }
                // Extract tool calls from response items
                extract_codex_tool_calls(&entry, &mut files_modified, &mut tools_used);
            }
            // Legacy event_msg format
            "event_msg" => {
                let role = entry.get("role").and_then(|v| v.as_str()).unwrap_or("");
                let text = entry
                    .get("content")
                    .or_else(|| entry.get("text"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if !text.is_empty() && (role == "user" || role == "assistant") {
                    messages.push(CodexMessage {
                        role: role.to_string(),
                        text: text.to_string(),
                    });
                }
            }
            // Standard or unknown entry
            _ => {
                if model.is_empty() {
                    if let Some(m) = entry.get("model").and_then(|v| v.as_str()) {
                        model = m.to_string();
                    }
                }
                let role = entry
                    .get("role")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let text = extract_codex_content(&entry);
                if !text.is_empty()
                    && (role == "user" || role == "assistant" || role == "system")
                {
                    messages.push(CodexMessage {
                        role: role.to_string(),
                        text,
                    });
                }
                // Extract file paths from tool_input
                if let Some(tool_input) = entry.get("tool_input").or_else(|| entry.get("input")) {
                    if let Some(fp) = tool_input
                        .get("file_path")
                        .or_else(|| tool_input.get("path"))
                        .and_then(|v| v.as_str())
                    {
                        if !files_modified.contains(&fp.to_string()) {
                            files_modified.push(fp.to_string());
                        }
                    }
                }
            }
        }

        // Extract token usage from any entry
        if let Some(usage) = entry.get("usage") {
            if let Some(it) = usage.get("input_tokens").and_then(|v| v.as_u64()) {
                input_tokens += it;
            }
            if let Some(ot) = usage.get("output_tokens").and_then(|v| v.as_u64()) {
                output_tokens += ot;
            }
            if let Some(pt) = usage.get("prompt_tokens").and_then(|v| v.as_u64()) {
                input_tokens += pt;
            }
            if let Some(ct) = usage.get("completion_tokens").and_then(|v| v.as_u64()) {
                output_tokens += ct;
            }
        }
    }

    // If JSONL parsing didn't produce messages, try as a single JSON document
    if messages.is_empty() {
        if let Ok(doc) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(msgs) = doc.get("messages").and_then(|v| v.as_array()) {
                for msg in msgs {
                    let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("");
                    let text = msg.get("content").and_then(|v| v.as_str()).unwrap_or("");
                    if !text.is_empty() {
                        messages.push(CodexMessage {
                            role: role.to_string(),
                            text: text.to_string(),
                        });
                    }
                }
            }
            if model.is_empty() {
                if let Some(m) = doc.get("model").and_then(|v| v.as_str()) {
                    model = m.to_string();
                }
            }
        }
    }

    if messages.is_empty() {
        return None;
    }

    if model.is_empty() {
        model = "codex".to_string();
    }

    let session_id = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    Some(CodexSession {
        session_id,
        model,
        messages,
        files_modified,
        tools_used,
        timestamp: first_ts.unwrap_or_else(Utc::now),
        input_tokens: if input_tokens > 0 {
            Some(input_tokens)
        } else {
            None
        },
        output_tokens: if output_tokens > 0 {
            Some(output_tokens)
        } else {
            None
        },
    })
}

/// Extract content from a Codex entry. Handles both string and array-of-objects content.
fn extract_codex_content(entry: &serde_json::Value) -> String {
    // Try string content first
    if let Some(text) = entry
        .get("content")
        .or_else(|| entry.get("text"))
        .or_else(|| entry.get("message"))
    {
        if let Some(s) = text.as_str() {
            return s.to_string();
        }
        // Content can be an array of text blocks (Codex response_item format)
        if let Some(arr) = text.as_array() {
            let parts: Vec<&str> = arr
                .iter()
                .filter_map(|item| {
                    item.get("text")
                        .and_then(|v| v.as_str())
                        .or_else(|| item.get("content").and_then(|v| v.as_str()))
                })
                .collect();
            if !parts.is_empty() {
                return parts.join("\n");
            }
        }
    }
    String::new()
}

/// Extract tool calls from a Codex response_item entry.
/// Codex supports: function_call, custom_tool_call, local_shell_call, web_search_call.
fn extract_codex_tool_calls(
    entry: &serde_json::Value,
    files: &mut Vec<String>,
    tools: &mut Vec<String>,
) {
    // Check for function_call or tool_call entries
    for key in &[
        "function_call",
        "custom_tool_call",
        "local_shell_call",
        "web_search_call",
    ] {
        if let Some(call) = entry.get(*key) {
            let tool_name = call
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or(*key);
            if !tools.contains(&tool_name.to_string()) {
                tools.push(tool_name.to_string());
            }
            // Extract file path from arguments
            if let Some(args) = call
                .get("arguments")
                .or_else(|| call.get("args"))
                .or_else(|| call.get("input"))
            {
                // args might be a string (JSON-encoded) or an object
                let args_obj = if let Some(s) = args.as_str() {
                    serde_json::from_str::<serde_json::Value>(s).ok()
                } else {
                    Some(args.clone())
                };
                if let Some(obj) = args_obj {
                    if let Some(fp) = obj
                        .get("file_path")
                        .or_else(|| obj.get("path"))
                        .or_else(|| obj.get("filename"))
                        .and_then(|v| v.as_str())
                    {
                        if !files.contains(&fp.to_string()) {
                            files.push(fp.to_string());
                        }
                    }
                }
            }
        }
    }

    // Also check for a "tool_calls" array
    if let Some(calls) = entry.get("tool_calls").and_then(|v| v.as_array()) {
        for call in calls {
            if let Some(func) = call.get("function") {
                let name = func
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                if !tools.contains(&name.to_string()) {
                    tools.push(name.to_string());
                }
            }
        }
    }
}

/// Install blameprompt hooks into Codex CLI's config.toml (~/.codex/config.toml).
pub fn install_hooks() -> Result<(), String> {
    let codex_dir = codex_home().ok_or("Codex CLI not found (~/.codex/ does not exist)")?;
    let config_path = codex_dir.join("config.toml");

    let binary = std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "blameprompt".to_string());

    let content = if config_path.exists() {
        std::fs::read_to_string(&config_path)
            .map_err(|e| format!("Cannot read {}: {}", config_path.display(), e))?
    } else {
        String::new()
    };

    // Check if already installed
    if content.contains("blameprompt") {
        println!("  BlamePrompt hooks already installed in {}", config_path.display());
        return Ok(());
    }

    // Append notify configuration to TOML
    let hook_config = format!(
        r#"
# BlamePrompt hook — records AI coding receipts
[[notify]]
args = ["{}", "checkpoint", "codex", "--hook-input", "stdin"]
"#,
        binary
    );

    let new_content = format!("{}{}", content, hook_config);
    std::fs::write(&config_path, new_content)
        .map_err(|e| format!("Cannot write {}: {}", config_path.display(), e))?;

    println!("  Installed BlamePrompt hooks in {}", config_path.display());
    Ok(())
}

/// Import a specific Codex CLI transcript file.
pub fn import_session(path: &Path) -> Option<Receipt> {
    let session = parse_codex_session(path)?;
    let cfg = config::load_config();
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let user = util::git_user();

    let first_user_msg = session
        .messages
        .iter()
        .find(|m| m.role == "user")
        .map(|m| {
            m.text
                .chars()
                .take(cfg.capture.max_prompt_length)
                .collect::<String>()
        })
        .unwrap_or_default();

    let prompt_summary = crate::core::redact::redact_secrets_with_config(&first_user_msg, &cfg);

    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(prompt_summary.as_bytes());
    let prompt_hash = format!("sha256:{:x}", hasher.finalize());

    let response_summary = session
        .messages
        .iter()
        .rev()
        .find(|m| m.role == "assistant")
        .map(|m| m.text.chars().take(500).collect());

    let files_changed: Vec<crate::core::receipt::FileChange> = session
        .files_modified
        .iter()
        .map(|f| crate::core::receipt::FileChange {
            path: util::make_relative(f, &cwd),
            line_range: (1, 1),
            blob_hash: None,
            additions: 0,
            deletions: 0,
        })
        .collect();

    let cost = if let (Some(it), Some(ot)) = (session.input_tokens, session.output_tokens) {
        crate::core::pricing::estimate_cost(&session.model, it, ot)
    } else {
        0.0
    };

    Some(Receipt {
        id: Receipt::new_id(),
        provider: "codex".to_string(),
        model: session.model,
        session_id: session.session_id,
        prompt_summary,
        response_summary,
        prompt_hash,
        message_count: session.messages.len() as u32,
        cost_usd: cost,
        input_tokens: session.input_tokens,
        output_tokens: session.output_tokens,
        cache_read_tokens: None,
        cache_creation_tokens: None,
        timestamp: session.timestamp,
        session_start: None,
        session_end: None,
        session_duration_secs: None,
        ai_response_time_secs: None,
        user,
        file_path: files_changed
            .first()
            .map(|f| f.path.clone())
            .unwrap_or_default(),
        line_range: (0, 0),
        files_changed,
        parent_receipt_id: None,
        parent_session_id: None,
        is_continuation: None,
        continuation_depth: None,
        prompt_number: Some(1),
        total_additions: 0,
        total_deletions: 0,
        tools_used: session.tools_used,
        mcp_servers: vec![],
        agents_spawned: vec![],
        subagent_activities: vec![],
        concurrent_tool_calls: None,
        user_decisions: vec![],
        conversation: None,
        prompt_submitted_at: Some(session.timestamp),
        prompt_duration_secs: None,
        accepted_lines: None,
        overridden_lines: None,
    })
}

/// Main entry point: scan Codex CLI sessions and create receipts.
pub fn run_record_codex(session_path: Option<&str>) {
    let files = if let Some(path) = session_path {
        let p = PathBuf::from(path);
        if !p.exists() {
            eprintln!("[codex] File not found: {}", path);
            std::process::exit(1);
        }
        if p.is_dir() {
            list_session_files(&p)
        } else {
            vec![p]
        }
    } else {
        let dirs = find_sessions_dirs();
        if dirs.is_empty() {
            eprintln!("[codex] Cannot find Codex CLI sessions directory.");
            eprintln!("  Pass --session <path> to specify a transcript file.");
            std::process::exit(1);
        }
        let mut all_files: Vec<PathBuf> = Vec::new();
        for dir in &dirs {
            all_files.extend(list_session_files(dir));
        }
        // Sort all files by modification time (newest first)
        all_files.sort_by_key(|f| {
            std::fs::metadata(f)
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
        });
        all_files.reverse();
        if all_files.is_empty() {
            eprintln!(
                "[codex] No session files found in {}",
                dirs.iter()
                    .map(|d| d.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            return;
        }
        // Import only the most recent 10 sessions
        all_files.into_iter().take(10).collect()
    };

    let mut count = 0usize;
    for file in &files {
        if let Some(receipt) = import_session(file) {
            staging::upsert_receipt(&receipt);
            count += 1;
        }
    }

    if count == 0 {
        eprintln!("[codex] No valid sessions found in the provided file(s).");
    } else {
        println!(
            "[codex] Recorded {} Codex CLI session(s)",
            count
        );
        println!("  Receipts staged. They will be attached on next git commit.");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_codex_session_jsonl() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let content = r#"{"role":"user","content":"Fix the bug","timestamp":"2026-02-01T10:00:00Z","model":"gpt-4.1"}
{"role":"assistant","content":"I'll fix it","usage":{"prompt_tokens":100,"completion_tokens":50}}
"#;
        std::fs::write(tmp.path(), content).unwrap();

        let session = parse_codex_session(tmp.path());
        assert!(session.is_some());
        let s = session.unwrap();
        assert_eq!(s.model, "gpt-4.1");
        assert_eq!(s.messages.len(), 2);
        assert_eq!(s.input_tokens, Some(100));
        assert_eq!(s.output_tokens, Some(50));
    }

    #[test]
    fn test_parse_codex_session_empty() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), "").unwrap();

        let session = parse_codex_session(tmp.path());
        assert!(session.is_none());
    }

    #[test]
    fn test_parse_codex_session_single_json() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let content = r#"{"model":"codex-mini","messages":[{"role":"user","content":"hello"},{"role":"assistant","content":"hi"}]}"#;
        std::fs::write(tmp.path(), content).unwrap();

        let session = parse_codex_session(tmp.path());
        assert!(session.is_some());
        let s = session.unwrap();
        assert_eq!(s.model, "codex-mini");
        assert_eq!(s.messages.len(), 2);
    }

    #[test]
    fn test_parse_codex_turn_context_and_response_item() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let content = r#"{"type":"turn_context","model":"gpt-4.1","timestamp":"2026-02-01T10:00:00Z"}
{"type":"response_item","role":"user","content":"Fix the auth module"}
{"type":"response_item","role":"assistant","content":"I'll update the authentication","function_call":{"name":"write_file","arguments":"{\"file_path\":\"src/auth.rs\"}"}}
"#;
        std::fs::write(tmp.path(), content).unwrap();

        let session = parse_codex_session(tmp.path());
        assert!(session.is_some());
        let s = session.unwrap();
        assert_eq!(s.model, "gpt-4.1");
        assert_eq!(s.messages.len(), 2);
        assert!(s.files_modified.contains(&"src/auth.rs".to_string()));
        assert!(s.tools_used.contains(&"write_file".to_string()));
    }
}
