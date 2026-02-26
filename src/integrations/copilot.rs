/// GitHub Copilot integration for blameprompt.
///
/// Reads AI chat sessions from VS Code's Copilot Chat extension storage (SQLite)
/// and converts them to blameprompt receipts staged for the next git commit.
///
/// Copilot Chat stores history in:
///   macOS: ~/Library/Application Support/Code/User/workspaceStorage/<hash>/state.vscdb
///   Linux: ~/.config/Code/User/workspaceStorage/<hash>/state.vscdb
use crate::commands::staging;
use crate::core::{config, receipt::Receipt, util};
use chrono::{DateTime, TimeZone, Utc};
use rusqlite::Connection;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug)]
pub struct CopilotChatSession {
    pub session_id: String,
    pub title: String,
    pub model: String,
    pub messages: Vec<CopilotMessage>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug)]
pub struct CopilotMessage {
    pub role: String,
    pub text: String,
    #[allow(dead_code)]
    pub timestamp: Option<DateTime<Utc>>,
}

// ── Flexible Copilot Chat JSON deserialization ───────────────────────────

#[derive(Deserialize, Debug)]
struct CopilotChatRoot {
    #[serde(default)]
    threads: Vec<CopilotThread>,
    // Fallback: some versions use a flat array of messages
    #[serde(default)]
    tabs: Vec<CopilotTab>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct CopilotThread {
    #[serde(default)]
    id: String,
    #[serde(default)]
    title: String,
    created_at: Option<i64>,
    #[serde(default)]
    turns: Vec<CopilotTurn>,
}

#[derive(Deserialize, Debug)]
struct CopilotTurn {
    #[allow(dead_code)]
    #[serde(default)]
    role: String,
    #[serde(default)]
    request: String,
    #[serde(default)]
    response: String,
    #[serde(default)]
    model: String,
    timestamp: Option<i64>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct CopilotTab {
    #[serde(default)]
    tab_id: String,
    #[serde(default)]
    chat_title: String,
    last_updated_at: Option<i64>,
    #[serde(default)]
    conversation: Vec<CopilotEntry>,
}

#[derive(Deserialize, Debug)]
struct CopilotEntry {
    #[serde(rename = "type", default)]
    entry_type: String,
    #[serde(default)]
    role: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    message: String,
    timestamp: Option<i64>,
    #[serde(default)]
    model: String,
}

impl CopilotEntry {
    fn effective_role(&self) -> String {
        let raw = if !self.role.is_empty() {
            self.role.as_str()
        } else {
            self.entry_type.as_str()
        };
        match raw.to_lowercase().as_str() {
            "human" | "user" => "user".to_string(),
            "ai" | "assistant" | "bot" | "copilot" => "assistant".to_string(),
            other => other.to_string(),
        }
    }

    fn effective_text(&self) -> &str {
        if !self.text.is_empty() {
            &self.text
        } else {
            &self.message
        }
    }

    fn effective_timestamp(&self) -> Option<DateTime<Utc>> {
        self.timestamp.map(|ms| {
            if ms > 1_000_000_000_000 {
                Utc.timestamp_millis_opt(ms)
                    .single()
                    .unwrap_or_else(Utc::now)
            } else {
                Utc.timestamp_opt(ms, 0)
                    .single()
                    .unwrap_or_else(Utc::now)
            }
        })
    }
}

/// Locate VS Code workspace storage directories.
pub fn find_workspace_storage_dirs() -> Vec<PathBuf> {
    let base = vscode_storage_base();
    match base {
        Some(b) if b.exists() => {
            let mut dirs: Vec<PathBuf> = std::fs::read_dir(&b)
                .ok()
                .into_iter()
                .flatten()
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.join("state.vscdb").exists())
                .collect();
            dirs.sort_by_key(|d| {
                std::fs::metadata(d.join("state.vscdb"))
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
            });
            dirs.reverse();
            dirs
        }
        _ => vec![],
    }
}

fn vscode_storage_base() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    // macOS
    let macos = home.join("Library/Application Support/Code/User/workspaceStorage");
    if macos.exists() {
        return Some(macos);
    }
    // Linux
    let linux = home.join(".config/Code/User/workspaceStorage");
    if linux.exists() {
        return Some(linux);
    }
    // Windows
    if let Ok(appdata) = std::env::var("APPDATA") {
        let win = PathBuf::from(appdata).join("Code/User/workspaceStorage");
        if win.exists() {
            return Some(win);
        }
    }
    None
}

/// Read all Copilot chat sessions from a workspace state.vscdb.
pub fn read_chat_sessions(db_path: &Path) -> Vec<CopilotChatSession> {
    let conn = match Connection::open(db_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[copilot] Cannot open {}: {}", db_path.display(), e);
            return vec![];
        }
    };

    let has_table: bool = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='ItemTable'",
            [],
            |r| r.get::<_, i32>(0),
        )
        .unwrap_or(0)
        > 0;

    if !has_table {
        return vec![];
    }

    // Known Copilot Chat storage keys
    let known_key_patterns = &[
        "github.copilot-chat.chatData",
        "github.copilot.chat.chatData",
        "copilot-chat.chatData",
        "interactiveSession.chatData",
    ];

    let mut sessions: Vec<CopilotChatSession> = Vec::new();
    let mut found_keys: Vec<String> = Vec::new();

    for &key in known_key_patterns {
        if let Ok(value) = conn.query_row(
            "SELECT value FROM ItemTable WHERE key = ?1",
            [key],
            |r| r.get::<_, String>(0),
        ) {
            found_keys.push(value);
        }
    }

    // Fallback: scan for any key with "copilot" in the name
    if found_keys.is_empty() {
        let mut stmt = match conn.prepare(
            "SELECT value FROM ItemTable WHERE key LIKE '%copilot%' OR key LIKE '%Copilot%'",
        ) {
            Ok(s) => s,
            Err(_) => return sessions,
        };
        let rows = stmt.query_map([], |r| r.get::<_, String>(0));
        if let Ok(rows) = rows {
            for row in rows.flatten() {
                found_keys.push(row);
            }
        }
    }

    for value in found_keys {
        if let Some(parsed) = parse_copilot_chat_json(&value) {
            sessions.extend(parsed);
        }
    }

    sessions
}

fn parse_copilot_chat_json(json: &str) -> Option<Vec<CopilotChatSession>> {
    let root: CopilotChatRoot = serde_json::from_str(json).ok()?;
    let mut sessions = Vec::new();

    // Try threads format first (newer Copilot Chat)
    for thread in root.threads {
        if thread.turns.is_empty() {
            continue;
        }

        let model = thread
            .turns
            .iter()
            .rev()
            .find(|t| !t.model.is_empty())
            .map(|t| t.model.clone())
            .unwrap_or_else(|| "gpt-4o".to_string());

        let timestamp = thread
            .created_at
            .and_then(|ms| {
                if ms > 1_000_000_000_000 {
                    Utc.timestamp_millis_opt(ms).single()
                } else {
                    Utc.timestamp_opt(ms, 0).single()
                }
            })
            .unwrap_or_else(Utc::now);

        let mut messages = Vec::new();
        for turn in thread.turns {
            let ts = turn.timestamp.and_then(|ms| {
                if ms > 1_000_000_000_000 {
                    Utc.timestamp_millis_opt(ms).single()
                } else {
                    Utc.timestamp_opt(ms, 0).single()
                }
            });
            if !turn.request.is_empty() {
                messages.push(CopilotMessage {
                    role: "user".to_string(),
                    text: turn.request,
                    timestamp: ts,
                });
            }
            if !turn.response.is_empty() {
                messages.push(CopilotMessage {
                    role: "assistant".to_string(),
                    text: turn.response,
                    timestamp: ts,
                });
            }
        }

        sessions.push(CopilotChatSession {
            session_id: thread.id,
            title: thread.title,
            model,
            messages,
            timestamp,
        });
    }

    // Fallback: tabs format (similar to Cursor)
    for tab in root.tabs {
        if tab.conversation.is_empty() {
            continue;
        }

        let model = tab
            .conversation
            .iter()
            .rev()
            .find(|e| e.effective_role() == "assistant")
            .map(|e| e.model.as_str())
            .filter(|m| !m.is_empty())
            .unwrap_or("gpt-4o")
            .to_string();

        let timestamp = tab
            .last_updated_at
            .and_then(|ms| {
                if ms > 1_000_000_000_000 {
                    Utc.timestamp_millis_opt(ms).single()
                } else {
                    Utc.timestamp_opt(ms, 0).single()
                }
            })
            .unwrap_or_else(Utc::now);

        let messages: Vec<CopilotMessage> = tab
            .conversation
            .into_iter()
            .filter(|e| !e.effective_text().trim().is_empty())
            .map(|e| CopilotMessage {
                role: e.effective_role(),
                text: e.effective_text().to_string(),
                timestamp: e.effective_timestamp(),
            })
            .collect();

        sessions.push(CopilotChatSession {
            session_id: tab.tab_id,
            title: tab.chat_title,
            model,
            messages,
            timestamp,
        });
    }

    if sessions.is_empty() {
        None
    } else {
        Some(sessions)
    }
}

/// Find the VS Code workspace storage directory for the current git repo.
pub fn find_db_for_current_workspace() -> Option<PathBuf> {
    let workspace_path = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| {
            std::env::current_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default()
        });

    let all = find_workspace_storage_dirs();
    for dir in &all {
        let db = dir.join("state.vscdb");
        if let Ok(conn) = Connection::open(&db) {
            let found: bool = conn
                .query_row(
                    "SELECT count(*) FROM ItemTable WHERE key = 'workspaceFolders' AND value LIKE ?1",
                    [format!("%{}%", workspace_path)],
                    |r| r.get::<_, i32>(0),
                )
                .unwrap_or(0)
                > 0;
            if found {
                return Some(db);
            }
        }
    }

    all.into_iter().next().map(|d| d.join("state.vscdb"))
}

/// Main entry point: scan VS Code workspace for Copilot Chat sessions and create receipts.
pub fn run_record_copilot(workspace: Option<&str>) {
    let db_path = if let Some(w) = workspace {
        let p = PathBuf::from(w);
        if p.extension().is_some_and(|e| e == "vscdb") {
            p
        } else {
            p.join("state.vscdb")
        }
    } else {
        match find_db_for_current_workspace() {
            Some(p) => p,
            None => {
                eprintln!("[copilot] Cannot find VS Code workspace storage.");
                eprintln!(
                    "  Pass --workspace <path/to/state.vscdb> to specify the database."
                );
                std::process::exit(1);
            }
        }
    };

    if !db_path.exists() {
        eprintln!("[copilot] Database not found: {}", db_path.display());
        std::process::exit(1);
    }

    let sessions = read_chat_sessions(&db_path);
    if sessions.is_empty() {
        eprintln!(
            "[copilot] No Copilot Chat sessions found in {}",
            db_path.display()
        );
        eprintln!("  Make sure you have used GitHub Copilot Chat in this workspace.");
        return;
    }

    let cfg = config::load_config();
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let user = util::git_user();
    let mut count = 0usize;

    let changed_files = get_recent_changed_files();

    for session in &sessions {
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
            .unwrap_or_else(|| session.title.clone());

        let prompt_summary =
            crate::core::redact::redact_secrets_with_config(&first_user_msg, &cfg);

        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(prompt_summary.as_bytes());
        let prompt_hash = format!("sha256:{:x}", hasher.finalize());

        let files_changed: Vec<crate::core::receipt::FileChange> = changed_files
            .iter()
            .map(|f| crate::core::receipt::FileChange {
                path: util::make_relative(f, &cwd),
                line_range: (1, 1),
                blob_hash: None,
                additions: 0,
                deletions: 0,
            })
            .collect();

        let receipt = Receipt {
            id: Receipt::new_id(),
            provider: "copilot".to_string(),
            model: session.model.clone(),
            session_id: session.session_id.clone(),
            prompt_summary,
            response_summary: session
                .messages
                .iter()
                .rev()
                .find(|m| m.role == "assistant")
                .map(|m| m.text.chars().take(500).collect()),
            prompt_hash,
            message_count: session.messages.len() as u32,
            cost_usd: 0.0,
            input_tokens: None,
            output_tokens: None,
            cache_read_tokens: None,
            cache_creation_tokens: None,
            timestamp: session.timestamp,
            session_start: None,
            session_end: None,
            session_duration_secs: None,
            ai_response_time_secs: None,
            user: user.clone(),
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
            prompt_number: Some((count as u32) + 1),
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
        };

        staging::upsert_receipt(&receipt);
        count += 1;
    }

    println!(
        "[copilot] Recorded {} Copilot Chat session(s) from {}",
        count,
        db_path.display()
    );
    println!("  Receipts staged. They will be attached on next git commit.");
}

/// Install blameprompt hooks for GitHub Copilot.
/// Creates ~/.github/hooks/blameprompt.json with PreToolUse/PostToolUse hooks.
pub fn install_hooks() -> Result<(), String> {
    let home = dirs::home_dir().ok_or("Cannot find home directory")?;
    let hooks_dir = home.join(".github").join("hooks");

    std::fs::create_dir_all(&hooks_dir)
        .map_err(|e| format!("Cannot create {}: {}", hooks_dir.display(), e))?;

    let hook_path = hooks_dir.join("blameprompt.json");

    // Check if already installed
    if hook_path.exists() {
        let content = std::fs::read_to_string(&hook_path).unwrap_or_default();
        if content.contains("blameprompt") {
            println!(
                "  BlamePrompt hooks already installed in {}",
                hook_path.display()
            );
            return Ok(());
        }
    }

    let binary = std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "blameprompt".to_string());

    let command = format!("{} checkpoint copilot --hook-input stdin", binary);

    let hook_config = serde_json::json!({
        "hooks": {
            "PreToolUse": [{
                "matcher": "Write|Edit|MultiEdit",
                "hooks": [{"type": "command", "command": command}]
            }],
            "PostToolUse": [{
                "matcher": "Write|Edit|MultiEdit|Bash",
                "hooks": [{"type": "command", "command": command}]
            }]
        }
    });

    let json_str = serde_json::to_string_pretty(&hook_config)
        .map_err(|e| format!("Failed to serialize: {}", e))?;
    std::fs::write(&hook_path, json_str)
        .map_err(|e| format!("Cannot write {}: {}", hook_path.display(), e))?;

    println!(
        "  Installed BlamePrompt hooks in {}",
        hook_path.display()
    );
    Ok(())
}

fn get_recent_changed_files() -> Vec<String> {
    let output = Command::new("git")
        .args(["diff", "--name-only", "HEAD"])
        .output()
        .ok();

    let mut files = Vec::new();
    if let Some(o) = output {
        if o.status.success() {
            for line in String::from_utf8_lossy(&o.stdout).lines() {
                let l = line.trim().to_string();
                if !l.is_empty() {
                    files.push(l);
                }
            }
        }
    }

    let staged = Command::new("git")
        .args(["diff", "--cached", "--name-only"])
        .output()
        .ok();
    if let Some(o) = staged {
        if o.status.success() {
            for line in String::from_utf8_lossy(&o.stdout).lines() {
                let l = line.trim().to_string();
                if !l.is_empty() && !files.contains(&l) {
                    files.push(l);
                }
            }
        }
    }

    files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_copilot_threads_format() {
        let json = r#"{
            "threads": [{
                "id": "thread-1",
                "title": "Fix login",
                "createdAt": 1700000000000,
                "turns": [
                    {"role": "user", "request": "Fix the login bug", "response": "", "model": "", "timestamp": 1700000000000},
                    {"role": "assistant", "request": "", "response": "I'll fix it", "model": "gpt-4o", "timestamp": 1700000001000}
                ]
            }],
            "tabs": []
        }"#;
        let result = parse_copilot_chat_json(json);
        assert!(result.is_some());
        let sessions = result.unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].title, "Fix login");
        assert_eq!(sessions[0].model, "gpt-4o");
        assert_eq!(sessions[0].messages.len(), 2);
    }

    #[test]
    fn test_parse_copilot_empty() {
        let json = r#"{"threads":[],"tabs":[]}"#;
        let result = parse_copilot_chat_json(json);
        assert!(result.is_none());
    }

    #[test]
    fn test_copilot_entry_effective_role() {
        let entry = CopilotEntry {
            entry_type: "copilot".to_string(),
            role: String::new(),
            text: "response".to_string(),
            message: String::new(),
            timestamp: None,
            model: "gpt-4o".to_string(),
        };
        assert_eq!(entry.effective_role(), "assistant");
    }
}
