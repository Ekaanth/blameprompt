/// Google Gemini CLI integration for blameprompt.
///
/// Imports Gemini CLI session transcripts and converts them to blameprompt receipts.
/// Also supports installing hooks into Gemini CLI's settings.json.
///
/// Gemini CLI stores session data in:
///   ~/.gemini/sessions/ (JSON/JSONL files)
///   ~/.config/gemini/sessions/ (Linux XDG)
///
/// Hook integration:
///   Modifies ~/.gemini/settings.json to add BeforeTool/AfterTool hooks.
use crate::commands::staging;
use crate::core::{config, receipt::Receipt, util};
use chrono::{DateTime, Utc};
use std::path::{Path, PathBuf};

/// A parsed Gemini CLI session.
#[derive(Debug)]
pub struct GeminiSession {
    pub session_id: String,
    pub model: String,
    pub messages: Vec<GeminiMessage>,
    pub files_modified: Vec<String>,
    pub timestamp: DateTime<Utc>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

#[derive(Debug)]
pub struct GeminiMessage {
    pub role: String,
    pub text: String,
}

/// Locate the Gemini CLI sessions directory.
pub fn find_sessions_dir() -> Option<PathBuf> {
    let home = dirs::home_dir()?;

    // Primary: ~/.gemini/sessions/
    let primary = home.join(".gemini").join("sessions");
    if primary.exists() {
        return Some(primary);
    }

    // Also check ~/.gemini/ root for session files
    let root = home.join(".gemini");
    if root.exists() {
        return Some(root);
    }

    // Linux XDG: ~/.config/gemini/sessions/
    let xdg = home.join(".config/gemini/sessions");
    if xdg.exists() {
        return Some(xdg);
    }

    if let Ok(config_dir) = std::env::var("XDG_DATA_HOME") {
        let xdg_custom = PathBuf::from(config_dir).join("gemini/sessions");
        if xdg_custom.exists() {
            return Some(xdg_custom);
        }
    }

    None
}

/// List session files, sorted by most recent.
pub fn list_session_files(sessions_dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(sessions_dir)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "jsonl" || e == "json"))
        .collect();
    files.sort_by_key(|f| {
        std::fs::metadata(f)
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
    });
    files.reverse();
    files
}

/// Try to parse the content as a single JSON document (Gemini API response format).
fn try_parse_single_json(path: &Path, content: &str) -> Option<GeminiSession> {
    let doc: serde_json::Value = serde_json::from_str(content).ok()?;
    let contents = doc.get("contents").and_then(|v| v.as_array())?;

    let mut messages = Vec::new();
    for msg in contents {
        let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("");
        let text = if let Some(parts) = msg.get("parts").and_then(|v| v.as_array()) {
            parts
                .iter()
                .filter_map(|p| p.get("text").and_then(|v| v.as_str()))
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            String::new()
        };
        if !text.is_empty() {
            messages.push(GeminiMessage {
                role: if role == "model" {
                    "assistant".to_string()
                } else {
                    role.to_string()
                },
                text,
            });
        }
    }

    if messages.is_empty() {
        return None;
    }

    let model = doc
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("gemini-2.5-flash")
        .to_string();

    let session_id = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    Some(GeminiSession {
        session_id,
        model,
        messages,
        files_modified: vec![],
        timestamp: Utc::now(),
        input_tokens: None,
        output_tokens: None,
    })
}

/// Parse a Gemini CLI session file.
pub fn parse_gemini_session(path: &Path) -> Option<GeminiSession> {
    let content = std::fs::read_to_string(path).ok()?;

    // Try as a single JSON document first (Gemini API response format)
    if let Some(session) = try_parse_single_json(path, &content) {
        return Some(session);
    }

    let mut messages = Vec::new();
    let mut model = String::new();
    let mut files_modified = Vec::new();
    let mut first_ts: Option<DateTime<Utc>> = None;
    let mut input_tokens: u64 = 0;
    let mut output_tokens: u64 = 0;

    // Try JSONL format
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let entry: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        if first_ts.is_none() {
            if let Some(ts_str) = entry.get("timestamp").and_then(|v| v.as_str()) {
                first_ts = DateTime::parse_from_rfc3339(ts_str)
                    .ok()
                    .map(|dt| dt.with_timezone(&Utc));
            }
        }

        if model.is_empty() {
            if let Some(m) = entry.get("model").and_then(|v| v.as_str()) {
                model = m.to_string();
            }
        }

        let role = entry
            .get("role")
            .or_else(|| entry.get("type"))
            .and_then(|v| v.as_str())
            .unwrap_or("");

        // Gemini uses "parts" array with "text" field
        let text = if let Some(parts) = entry.get("parts").and_then(|v| v.as_array()) {
            parts
                .iter()
                .filter_map(|p| p.get("text").and_then(|v| v.as_str()))
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            entry
                .get("content")
                .or_else(|| entry.get("text"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        };

        // Accept "gemini" as an alias for "model"/"assistant" (Gemini CLI uses this)
        if !text.is_empty()
            && (role == "user" || role == "model" || role == "assistant" || role == "gemini")
        {
            messages.push(GeminiMessage {
                role: if role == "model" || role == "gemini" {
                    "assistant".to_string()
                } else {
                    role.to_string()
                },
                text,
            });
        }

        // Extract file modifications from function calls
        if let Some(function_call) = entry.get("functionCall").or_else(|| entry.get("tool_call")) {
            if let Some(args) = function_call
                .get("args")
                .or_else(|| function_call.get("arguments"))
            {
                if let Some(fp) = args
                    .get("file_path")
                    .or_else(|| args.get("path"))
                    .and_then(|v| v.as_str())
                {
                    if !files_modified.contains(&fp.to_string()) {
                        files_modified.push(fp.to_string());
                    }
                }
            }
        }

        // Extract tool calls from toolCalls array
        if let Some(tool_calls) = entry.get("toolCalls").and_then(|v| v.as_array()) {
            for tc in tool_calls {
                if let Some(name) = tc.get("name").and_then(|v| v.as_str()) {
                    if let Some(args) = tc.get("args") {
                        if let Some(fp) = args
                            .get("file_path")
                            .or_else(|| args.get("path"))
                            .and_then(|v| v.as_str())
                        {
                            if !files_modified.contains(&fp.to_string()) {
                                files_modified.push(fp.to_string());
                            }
                        }
                    }
                    let _ = name; // tool name tracked for future use
                }
            }
        }

        // Extract token usage
        if let Some(usage) = entry.get("usageMetadata").or_else(|| entry.get("usage")) {
            if let Some(it) = usage
                .get("promptTokenCount")
                .or_else(|| usage.get("input_tokens"))
                .and_then(|v| v.as_u64())
            {
                input_tokens = input_tokens.max(it); // Gemini reports cumulative
            }
            if let Some(ot) = usage
                .get("candidatesTokenCount")
                .or_else(|| usage.get("output_tokens"))
                .and_then(|v| v.as_u64())
            {
                output_tokens = output_tokens.max(ot);
            }
        }
    }

    if messages.is_empty() {
        return None;
    }

    if model.is_empty() {
        model = "gemini-2.5-flash".to_string();
    }

    let session_id = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    Some(GeminiSession {
        session_id,
        model,
        messages,
        files_modified,
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

/// Import a specific Gemini session file.
pub fn import_session(path: &Path) -> Option<Receipt> {
    let session = parse_gemini_session(path)?;
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
        provider: "gemini".to_string(),
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
        tools_used: vec![],
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

/// Main entry point: scan Gemini CLI sessions and create receipts.
pub fn run_record_gemini(session_path: Option<&str>) {
    let files = if let Some(path) = session_path {
        let p = PathBuf::from(path);
        if !p.exists() {
            eprintln!("[gemini] File not found: {}", path);
            std::process::exit(1);
        }
        if p.is_dir() {
            list_session_files(&p)
        } else {
            vec![p]
        }
    } else {
        match find_sessions_dir() {
            Some(dir) => {
                let files = list_session_files(&dir);
                if files.is_empty() {
                    eprintln!("[gemini] No session files found in {}", dir.display());
                    return;
                }
                files.into_iter().take(10).collect()
            }
            None => {
                eprintln!("[gemini] Cannot find Gemini CLI sessions directory.");
                eprintln!("  Pass --session <path> to specify a transcript file.");
                std::process::exit(1);
            }
        }
    };

    let mut count = 0usize;
    for file in &files {
        if let Some(receipt) = import_session(file) {
            staging::upsert_receipt(&receipt);
            count += 1;
        }
    }

    if count == 0 {
        eprintln!("[gemini] No valid sessions found in the provided file(s).");
    } else {
        println!("[gemini] Recorded {} Gemini CLI session(s)", count);
        println!("  Receipts staged. They will be attached on next git commit.");
    }
}

/// Install blameprompt hooks into Gemini CLI's settings (~/.gemini/settings.json).
/// Adds BeforeTool/AfterTool hooks with write_file|replace matcher.
pub fn install_hooks() -> Result<(), String> {
    let home = dirs::home_dir().ok_or("Cannot find home directory")?;
    let gemini_dir = home.join(".gemini");
    if !gemini_dir.exists() {
        return Err("Gemini CLI not found (~/.gemini/ does not exist)".to_string());
    }

    let settings_path = gemini_dir.join("settings.json");
    let binary = std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "blameprompt".to_string());

    let mut settings: serde_json::Value = if settings_path.exists() {
        let content = std::fs::read_to_string(&settings_path)
            .map_err(|e| format!("Cannot read {}: {}", settings_path.display(), e))?;
        serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    // Check if already installed
    let settings_str = serde_json::to_string(&settings).unwrap_or_default();
    if settings_str.contains("blameprompt") {
        println!(
            "  BlamePrompt hooks already installed in {}",
            settings_path.display()
        );
        return Ok(());
    }

    let command = format!("{} checkpoint gemini --hook-input stdin", binary);

    // Enable tool hooks
    if settings.get("tools").is_none() {
        settings["tools"] = serde_json::json!({});
    }
    settings["tools"]["enableHooks"] = serde_json::json!(true);

    // Add hooks
    if settings.get("hooks").is_none() {
        settings["hooks"] = serde_json::json!({});
    }
    let hooks = settings.get_mut("hooks").unwrap();

    let hook_cmd = serde_json::json!([{
        "type": "command",
        "command": command
    }]);

    for event in &["BeforeTool", "AfterTool"] {
        let entry = serde_json::json!({
            "matcher": "write_file|replace",
            "hooks": hook_cmd
        });
        if hooks.get(*event).is_none() {
            hooks[*event] = serde_json::json!([]);
        }
        if let Some(arr) = hooks.get_mut(*event).and_then(|v| v.as_array_mut()) {
            arr.push(entry);
        }
    }

    let json_str = serde_json::to_string_pretty(&settings)
        .map_err(|e| format!("Failed to serialize: {}", e))?;
    std::fs::write(&settings_path, json_str)
        .map_err(|e| format!("Cannot write {}: {}", settings_path.display(), e))?;

    println!(
        "  Installed BlamePrompt hooks in {}",
        settings_path.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_gemini_session_jsonl() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let content = r#"{"role":"user","parts":[{"text":"Fix the bug"}],"timestamp":"2026-02-01T10:00:00Z","model":"gemini-2.5-pro"}
{"role":"model","parts":[{"text":"I'll fix it"}],"usageMetadata":{"promptTokenCount":100,"candidatesTokenCount":50}}
"#;
        std::fs::write(tmp.path(), content).unwrap();

        let session = parse_gemini_session(tmp.path());
        assert!(session.is_some());
        let s = session.unwrap();
        assert_eq!(s.model, "gemini-2.5-pro");
        assert_eq!(s.messages.len(), 2);
        assert_eq!(s.messages[1].role, "assistant"); // "model" → "assistant"
        assert_eq!(s.input_tokens, Some(100));
        assert_eq!(s.output_tokens, Some(50));
    }

    #[test]
    fn test_parse_gemini_session_empty() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), "").unwrap();

        let session = parse_gemini_session(tmp.path());
        assert!(session.is_none());
    }

    #[test]
    fn test_parse_gemini_contents_format() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let content = r#"{
            "model": "gemini-2.5-flash",
            "contents": [
                {"role": "user", "parts": [{"text": "hello"}]},
                {"role": "model", "parts": [{"text": "hi there"}]}
            ]
        }"#;
        std::fs::write(tmp.path(), content).unwrap();

        let session = parse_gemini_session(tmp.path());
        assert!(session.is_some());
        let s = session.unwrap();
        assert_eq!(s.model, "gemini-2.5-flash");
        assert_eq!(s.messages.len(), 2);
    }
}
