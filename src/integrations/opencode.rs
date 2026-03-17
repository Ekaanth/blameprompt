/// OpenCode terminal AI coding tool integration for blameprompt.
///
/// Imports OpenCode session transcripts and converts them to blameprompt receipts.
///
/// OpenCode stores session data in:
///   ~/.opencode/sessions/ (JSONL files)
///
/// Each line has `role`, `content`, `model`, `timestamp`, `tool_calls` array.
/// Config at ~/.opencode/config.json
use crate::commands::staging;
use crate::core::{config, receipt::Receipt, util};
use chrono::{DateTime, Utc};
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct OpenCodeSession {
    pub session_id: String,
    pub model: String,
    pub messages: Vec<OpenCodeMessage>,
    pub files_modified: Vec<String>,
    pub tools_used: Vec<String>,
    pub timestamp: DateTime<Utc>,
    pub end_timestamp: Option<DateTime<Utc>>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

#[derive(Debug)]
pub struct OpenCodeMessage {
    pub role: String,
    pub text: String,
}

pub fn find_sessions_dir() -> Option<PathBuf> {
    let home = dirs::home_dir()?;

    let primary = home.join(".opencode").join("sessions");
    if primary.exists() {
        return Some(primary);
    }

    let root = home.join(".opencode");
    if root.exists() {
        return Some(root);
    }

    None
}

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

pub fn parse_opencode_session(path: &Path) -> Option<OpenCodeSession> {
    let content = std::fs::read_to_string(path).ok()?;

    let mut messages = Vec::new();
    let mut model = String::new();
    let mut files_modified = Vec::new();
    let mut tools_used = Vec::new();
    let mut first_ts: Option<DateTime<Utc>> = None;
    let mut last_ts: Option<DateTime<Utc>> = None;
    let mut input_tokens: u64 = 0;
    let mut output_tokens: u64 = 0;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let entry: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        if let Some(ts_str) = entry.get("timestamp").and_then(|v| v.as_str()) {
            let ts = DateTime::parse_from_rfc3339(ts_str)
                .ok()
                .map(|dt| dt.with_timezone(&Utc));
            if first_ts.is_none() {
                first_ts = ts;
            }
            if ts.is_some() {
                last_ts = ts;
            }
        }

        if model.is_empty() {
            if let Some(m) = entry.get("model").and_then(|v| v.as_str()) {
                model = m.to_string();
            }
        }

        let role = entry.get("role").and_then(|v| v.as_str()).unwrap_or("");
        let text = entry
            .get("content")
            .or_else(|| entry.get("text"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if !text.is_empty() && (role == "user" || role == "assistant") {
            messages.push(OpenCodeMessage {
                role: role.to_string(),
                text,
            });
        }

        // Extract tool_calls array
        if let Some(tool_calls) = entry.get("tool_calls").and_then(|v| v.as_array()) {
            for tc in tool_calls {
                let name = tc
                    .get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|v| v.as_str())
                    .or_else(|| tc.get("name").and_then(|v| v.as_str()));
                if let Some(name) = name {
                    if !tools_used.contains(&name.to_string()) {
                        tools_used.push(name.to_string());
                    }
                }
                // Extract file paths from arguments
                let args = tc
                    .get("function")
                    .and_then(|f| f.get("arguments"))
                    .or_else(|| tc.get("args"))
                    .or_else(|| tc.get("arguments"));
                if let Some(args) = args {
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
                            .and_then(|v| v.as_str())
                        {
                            if !files_modified.contains(&fp.to_string()) {
                                files_modified.push(fp.to_string());
                            }
                        }
                    }
                }
            }
        }

        if let Some(usage) = entry.get("usage") {
            if let Some(it) = usage
                .get("input_tokens")
                .or_else(|| usage.get("prompt_tokens"))
                .and_then(|v| v.as_u64())
            {
                input_tokens += it;
            }
            if let Some(ot) = usage
                .get("output_tokens")
                .or_else(|| usage.get("completion_tokens"))
                .and_then(|v| v.as_u64())
            {
                output_tokens += ot;
            }
        }
    }

    // Fallback: try as single JSON
    if messages.is_empty() {
        if let Ok(doc) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(msgs) = doc.get("messages").and_then(|v| v.as_array()) {
                for msg in msgs {
                    let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("");
                    let text = msg.get("content").and_then(|v| v.as_str()).unwrap_or("");
                    if !text.is_empty() {
                        messages.push(OpenCodeMessage {
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
        model = "opencode".to_string();
    }

    let session_id = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    Some(OpenCodeSession {
        session_id,
        model,
        messages,
        files_modified,
        tools_used,
        timestamp: first_ts.unwrap_or_else(Utc::now),
        end_timestamp: last_ts,
        input_tokens: if input_tokens > 0 { Some(input_tokens) } else { None },
        output_tokens: if output_tokens > 0 { Some(output_tokens) } else { None },
    })
}

pub fn import_session(path: &Path) -> Option<Receipt> {
    let session = parse_opencode_session(path)?;
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

    let (final_input, final_output) = match (session.input_tokens, session.output_tokens) {
        (Some(it), Some(ot)) => (it, ot),
        _ => {
            let est_in = crate::core::pricing::estimate_tokens_from_chars(
                session.messages.iter().filter(|m| m.role == "user").map(|m| m.text.len()).sum(),
            );
            let est_out = crate::core::pricing::estimate_tokens_from_chars(
                session.messages.iter().filter(|m| m.role == "assistant").map(|m| m.text.len()).sum(),
            );
            (
                session.input_tokens.unwrap_or(est_in),
                session.output_tokens.unwrap_or(est_out),
            )
        }
    };
    let cost = crate::core::pricing::estimate_cost(&session.model, final_input, final_output);

    let session_duration_secs = session.end_timestamp.map(|end| {
        let dur = (end - session.timestamp).num_seconds();
        if dur > 0 { dur as u64 } else { 0 }
    });

    let conversation: Vec<crate::core::receipt::ConversationTurn> = session
        .messages
        .iter()
        .enumerate()
        .map(|(i, m)| crate::core::receipt::ConversationTurn {
            turn: (i as u32) + 1,
            role: m.role.clone(),
            content: crate::core::redact::redact_secrets_with_config(
                &m.text.chars().take(cfg.capture.max_prompt_length).collect::<String>(),
                &cfg,
            ),
            tool_name: None,
            files_touched: None,
        })
        .collect();

    let prompt_quality = Some(crate::core::prompt_eval::evaluate(&prompt_summary));

    Some(Receipt {
        id: Receipt::new_id(),
        provider: "opencode".to_string(),
        model: session.model,
        session_id: session.session_id,
        prompt_summary,
        response_summary,
        prompt_hash,
        message_count: session.messages.len() as u32,
        cost_usd: cost,
        input_tokens: Some(final_input),
        output_tokens: Some(final_output),
        cache_read_tokens: None,
        cache_creation_tokens: None,
        timestamp: session.timestamp,
        session_start: Some(session.timestamp),
        session_end: session.end_timestamp,
        session_duration_secs,
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
        conversation: if conversation.is_empty() { None } else { Some(conversation) },
        prompt_submitted_at: Some(session.timestamp),
        prompt_duration_secs: None,
        accepted_lines: None,
        overridden_lines: None,
        prompt_quality,
    })
}

pub fn run_record_opencode(session_path: Option<&str>) {
    let files = if let Some(path) = session_path {
        let p = PathBuf::from(path);
        if !p.exists() {
            eprintln!("[opencode] File not found: {}", path);
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
                    eprintln!("[opencode] No session files found in {}", dir.display());
                    return;
                }
                files.into_iter().take(10).collect()
            }
            None => {
                eprintln!("[opencode] Cannot find OpenCode sessions directory.");
                eprintln!("  Pass --session <path> to specify a transcript file.");
                return;
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
        eprintln!("[opencode] No valid sessions found in the provided file(s).");
    } else {
        println!("[opencode] Recorded {} OpenCode session(s)", count);
        println!("  Receipts staged. They will be attached on next git commit.");
    }
}
