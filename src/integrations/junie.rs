/// JetBrains Junie AI coding assistant integration for blameprompt.
///
/// Imports Junie session transcripts and converts them to blameprompt receipts.
///
/// Junie stores session data in JetBrains IDE workspace:
///   ~/Library/Application Support/JetBrains/*/junie/sessions/ (macOS)
///   ~/.config/JetBrains/*/junie/sessions/ (Linux)
///
/// Files are JSON with `messages` array containing `role`, `content`, `toolUse` objects.
use crate::commands::staging;
use crate::core::{config, receipt::Receipt, util};
use chrono::{DateTime, Utc};
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct JunieSession {
    pub session_id: String,
    pub model: String,
    pub messages: Vec<JunieMessage>,
    pub files_modified: Vec<String>,
    pub tools_used: Vec<String>,
    pub timestamp: DateTime<Utc>,
    pub end_timestamp: Option<DateTime<Utc>>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

#[derive(Debug)]
pub struct JunieMessage {
    pub role: String,
    pub text: String,
}

/// Locate all Junie sessions directories across JetBrains IDE versions.
pub fn find_sessions_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => return dirs,
    };

    // macOS: ~/Library/Application Support/JetBrains/*/junie/sessions/
    let macos_base = home.join("Library/Application Support/JetBrains");
    if macos_base.exists() {
        if let Ok(entries) = std::fs::read_dir(&macos_base) {
            for entry in entries.flatten() {
                let sessions = entry.path().join("junie").join("sessions");
                if sessions.exists() {
                    dirs.push(sessions);
                }
            }
        }
    }

    // Linux: ~/.config/JetBrains/*/junie/sessions/
    let linux_base = home.join(".config/JetBrains");
    if linux_base.exists() {
        if let Ok(entries) = std::fs::read_dir(&linux_base) {
            for entry in entries.flatten() {
                let sessions = entry.path().join("junie").join("sessions");
                if sessions.exists() {
                    dirs.push(sessions);
                }
            }
        }
    }

    dirs
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

pub fn parse_junie_session(path: &Path) -> Option<JunieSession> {
    let content = std::fs::read_to_string(path).ok()?;

    let mut messages = Vec::new();
    let mut model = String::new();
    let mut files_modified = Vec::new();
    let mut tools_used = Vec::new();
    let mut first_ts: Option<DateTime<Utc>> = None;
    let mut last_ts: Option<DateTime<Utc>> = None;
    let mut input_tokens: u64 = 0;
    let mut output_tokens: u64 = 0;

    // Try as a single JSON document first (Junie uses JSON with messages array)
    if let Ok(doc) = serde_json::from_str::<serde_json::Value>(&content) {
        if let Some(m) = doc.get("model").and_then(|v| v.as_str()) {
            model = m.to_string();
        }

        if let Some(ts_str) = doc.get("timestamp").and_then(|v| v.as_str()) {
            first_ts = DateTime::parse_from_rfc3339(ts_str)
                .ok()
                .map(|dt| dt.with_timezone(&Utc));
        }

        if let Some(msgs) = doc.get("messages").and_then(|v| v.as_array()) {
            for msg in msgs {
                let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("");
                let text = msg.get("content").and_then(|v| v.as_str()).unwrap_or("");

                if let Some(ts_str) = msg.get("timestamp").and_then(|v| v.as_str()) {
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

                if !text.is_empty() && (role == "user" || role == "assistant") {
                    messages.push(JunieMessage {
                        role: role.to_string(),
                        text: text.to_string(),
                    });
                }

                // Extract toolUse
                if let Some(tool_use) = msg.get("toolUse") {
                    if let Some(name) = tool_use.get("name").and_then(|v| v.as_str()) {
                        if !tools_used.contains(&name.to_string()) {
                            tools_used.push(name.to_string());
                        }
                    }
                    if let Some(args) = tool_use.get("args").or_else(|| tool_use.get("arguments")) {
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

                // Extract toolUse from array too
                if let Some(tool_uses) = msg.get("toolUse").and_then(|v| v.as_array()) {
                    for tu in tool_uses {
                        if let Some(name) = tu.get("name").and_then(|v| v.as_str()) {
                            if !tools_used.contains(&name.to_string()) {
                                tools_used.push(name.to_string());
                            }
                        }
                    }
                }
            }
        }

        // Token usage from doc level
        if let Some(usage) = doc.get("usage") {
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

    // Fallback: try JSONL
    if messages.is_empty() {
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
                messages.push(JunieMessage {
                    role: role.to_string(),
                    text,
                });
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
    }

    if messages.is_empty() {
        return None;
    }

    if model.is_empty() {
        model = "junie".to_string();
    }

    let session_id = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    Some(JunieSession {
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
    let session = parse_junie_session(path)?;
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
        provider: "junie".to_string(),
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

pub fn run_record_junie(session_path: Option<&str>) {
    let files = if let Some(path) = session_path {
        let p = PathBuf::from(path);
        if !p.exists() {
            eprintln!("[junie] File not found: {}", path);
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
            eprintln!("[junie] Cannot find Junie sessions directory.");
            eprintln!("  Pass --session <path> to specify a transcript file.");
            return;
        }
        let mut all_files: Vec<PathBuf> = Vec::new();
        for dir in &dirs {
            all_files.extend(list_session_files(dir));
        }
        all_files.sort_by_key(|f| {
            std::fs::metadata(f)
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
        });
        all_files.reverse();
        if all_files.is_empty() {
            eprintln!(
                "[junie] No session files found in {}",
                dirs.iter()
                    .map(|d| d.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            return;
        }
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
        eprintln!("[junie] No valid sessions found in the provided file(s).");
    } else {
        println!("[junie] Recorded {} Junie session(s)", count);
        println!("  Receipts staged. They will be attached on next git commit.");
    }
}
