/// Windsurf (Codeium) integration for blameprompt.
///
/// Reads AI chat sessions from Windsurf's workspace storage (SQLite)
/// and converts them to blameprompt receipts staged for the next git commit.
///
/// Windsurf stores chat history in:
///   macOS: ~/Library/Application Support/Windsurf/User/workspaceStorage/<hash>/state.vscdb
///   Linux: ~/.config/Windsurf/User/workspaceStorage/<hash>/state.vscdb
use crate::commands::staging;
use crate::core::{config, receipt::Receipt, util};
use chrono::{DateTime, TimeZone, Utc};
use rusqlite::Connection;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug)]
pub struct WindsurfChatSession {
    pub session_id: String,
    pub title: String,
    pub model: String,
    pub messages: Vec<WindsurfMessage>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug)]
pub struct WindsurfMessage {
    pub role: String,
    pub text: String,
    #[allow(dead_code)]
    pub timestamp: Option<DateTime<Utc>>,
}

// ── Flexible Windsurf JSON deserialization ────────────────────────────────

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct WindsurfChatRoot {
    #[serde(default)]
    tabs: Vec<WindsurfTab>,
    // Windsurf Cascade uses "conversations"
    #[serde(default)]
    conversations: Vec<WindsurfConversation>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct WindsurfTab {
    #[serde(default)]
    tab_id: String,
    #[serde(default)]
    chat_title: String,
    last_updated_at: Option<i64>,
    #[serde(default)]
    conversation: Vec<WindsurfEntry>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct WindsurfConversation {
    #[serde(default)]
    id: String,
    #[serde(default)]
    title: String,
    created_at: Option<i64>,
    #[serde(default)]
    messages: Vec<WindsurfEntry>,
}

#[derive(Deserialize, Debug)]
struct WindsurfEntry {
    #[serde(rename = "type", default)]
    entry_type: String,
    #[serde(default)]
    role: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    content: String,
    #[serde(default)]
    message: String,
    timestamp: Option<i64>,
    #[serde(default)]
    model: String,
}

impl WindsurfEntry {
    fn effective_role(&self) -> String {
        let raw = if !self.role.is_empty() {
            self.role.as_str()
        } else {
            self.entry_type.as_str()
        };
        match raw.to_lowercase().as_str() {
            "human" | "user" => "user".to_string(),
            "ai" | "assistant" | "bot" | "cascade" => "assistant".to_string(),
            other => other.to_string(),
        }
    }

    fn effective_text(&self) -> &str {
        if !self.text.is_empty() {
            &self.text
        } else if !self.content.is_empty() {
            &self.content
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
                Utc.timestamp_opt(ms, 0).single().unwrap_or_else(Utc::now)
            }
        })
    }
}

/// Locate Windsurf workspace storage directories.
pub fn find_workspace_storage_dirs() -> Vec<PathBuf> {
    let base = windsurf_storage_base();
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

fn windsurf_storage_base() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    // macOS
    let macos = home.join("Library/Application Support/Windsurf/User/workspaceStorage");
    if macos.exists() {
        return Some(macos);
    }
    // Also check Codeium path
    let codeium = home.join("Library/Application Support/Codeium/User/workspaceStorage");
    if codeium.exists() {
        return Some(codeium);
    }
    // Linux
    let linux = home.join(".config/Windsurf/User/workspaceStorage");
    if linux.exists() {
        return Some(linux);
    }
    let linux_codeium = home.join(".config/Codeium/User/workspaceStorage");
    if linux_codeium.exists() {
        return Some(linux_codeium);
    }
    // Windows
    if let Ok(appdata) = std::env::var("APPDATA") {
        let win = PathBuf::from(&appdata).join("Windsurf/User/workspaceStorage");
        if win.exists() {
            return Some(win);
        }
        let win_codeium = PathBuf::from(appdata).join("Codeium/User/workspaceStorage");
        if win_codeium.exists() {
            return Some(win_codeium);
        }
    }
    None
}

/// Read all Windsurf chat sessions from a workspace state.vscdb.
pub fn read_chat_sessions(db_path: &Path) -> Vec<WindsurfChatSession> {
    let conn = match Connection::open(db_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[windsurf] Cannot open {}: {}", db_path.display(), e);
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

    let known_key_patterns = &[
        "codeium.chatData",
        "windsurf.chatData",
        "cascade.chatData",
        "codeium.cascade.chatData",
        "workbench.panel.aichat.view.aichat.chatData",
        "aichat.chatData",
    ];

    let mut sessions: Vec<WindsurfChatSession> = Vec::new();
    let mut found_keys: Vec<String> = Vec::new();

    for &key in known_key_patterns {
        if let Ok(value) =
            conn.query_row("SELECT value FROM ItemTable WHERE key = ?1", [key], |r| {
                r.get::<_, String>(0)
            })
        {
            found_keys.push(value);
        }
    }

    // Fallback: scan for related keys
    if found_keys.is_empty() {
        let mut stmt = match conn.prepare(
            "SELECT value FROM ItemTable WHERE key LIKE '%codeium%' OR key LIKE '%windsurf%' OR key LIKE '%cascade%' OR key LIKE '%chat%'",
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
        if let Some(parsed) = parse_windsurf_chat_json(&value) {
            sessions.extend(parsed);
        }
    }

    sessions
}

fn parse_windsurf_chat_json(json: &str) -> Option<Vec<WindsurfChatSession>> {
    let root: WindsurfChatRoot = serde_json::from_str(json).ok()?;
    let mut sessions = Vec::new();

    // Tabs format
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
            .unwrap_or("windsurf")
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

        let messages: Vec<WindsurfMessage> = tab
            .conversation
            .into_iter()
            .filter(|e| !e.effective_text().trim().is_empty())
            .map(|e| WindsurfMessage {
                role: e.effective_role(),
                text: e.effective_text().to_string(),
                timestamp: e.effective_timestamp(),
            })
            .collect();

        sessions.push(WindsurfChatSession {
            session_id: tab.tab_id,
            title: tab.chat_title,
            model,
            messages,
            timestamp,
        });
    }

    // Conversations format (Cascade)
    for conv in root.conversations {
        if conv.messages.is_empty() {
            continue;
        }

        let model = conv
            .messages
            .iter()
            .rev()
            .find(|e| e.effective_role() == "assistant")
            .map(|e| e.model.as_str())
            .filter(|m| !m.is_empty())
            .unwrap_or("windsurf")
            .to_string();

        let timestamp = conv
            .created_at
            .and_then(|ms| {
                if ms > 1_000_000_000_000 {
                    Utc.timestamp_millis_opt(ms).single()
                } else {
                    Utc.timestamp_opt(ms, 0).single()
                }
            })
            .unwrap_or_else(Utc::now);

        let messages: Vec<WindsurfMessage> = conv
            .messages
            .into_iter()
            .filter(|e| !e.effective_text().trim().is_empty())
            .map(|e| WindsurfMessage {
                role: e.effective_role(),
                text: e.effective_text().to_string(),
                timestamp: e.effective_timestamp(),
            })
            .collect();

        sessions.push(WindsurfChatSession {
            session_id: conv.id,
            title: conv.title,
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

/// Find the Windsurf workspace storage directory for the current git repo.
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

/// Main entry point: scan Windsurf workspace and create receipts.
pub fn run_record_windsurf(workspace: Option<&str>) {
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
                eprintln!("[windsurf] Cannot find Windsurf workspace storage.");
                eprintln!("  Pass --workspace <path/to/state.vscdb> to specify the database.");
                std::process::exit(1);
            }
        }
    };

    if !db_path.exists() {
        eprintln!("[windsurf] Database not found: {}", db_path.display());
        std::process::exit(1);
    }

    let sessions = read_chat_sessions(&db_path);
    if sessions.is_empty() {
        eprintln!(
            "[windsurf] No AI chat sessions found in {}",
            db_path.display()
        );
        eprintln!("  Make sure you have used Windsurf's AI features in this workspace.");
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

        let prompt_summary = crate::core::redact::redact_secrets_with_config(&first_user_msg, &cfg);

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
            provider: "windsurf".to_string(),
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
        "[windsurf] Recorded {} Windsurf AI session(s) from {}",
        count,
        db_path.display()
    );
    println!("  Receipts staged. They will be attached on next git commit.");
}

fn get_recent_changed_files() -> Vec<String> {
    let output = std::process::Command::new("git")
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

    let staged = std::process::Command::new("git")
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

/// Install BlamePrompt hooks for Windsurf (Codeium).
/// Writes to ~/.windsurf/hooks.json or ~/.codeium/hooks.json.
pub fn install_hooks() -> Result<(), String> {
    let home = dirs::home_dir().ok_or("Cannot find home directory")?;

    // Try Windsurf first, then Codeium
    let target_dir = if home.join(".windsurf").exists() {
        home.join(".windsurf")
    } else if home.join(".codeium").exists() {
        home.join(".codeium")
    } else {
        return Err(
            "Windsurf/Codeium not found (~/.windsurf/ or ~/.codeium/ does not exist)".to_string(),
        );
    };

    let hook_path = target_dir.join("hooks.json");

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

    let command = format!("{} checkpoint windsurf --hook-input stdin", binary);

    let hook_config = serde_json::json!({
        "hooks": {
            "afterFileEdit": [{
                "command": command,
                "description": "BlamePrompt: record AI file edits"
            }],
            "afterCascadeResponse": [{
                "command": format!("{} checkpoint windsurf --hook-input stdin", binary),
                "description": "BlamePrompt: capture Cascade responses"
            }]
        }
    });

    let json_str = serde_json::to_string_pretty(&hook_config)
        .map_err(|e| format!("Failed to serialize: {}", e))?;
    std::fs::write(&hook_path, json_str)
        .map_err(|e| format!("Cannot write {}: {}", hook_path.display(), e))?;

    println!("  Installed BlamePrompt hooks in {}", hook_path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_windsurf_tabs_format() {
        let json = r#"{
            "tabs": [{
                "tabId": "tab1",
                "chatTitle": "Fix bug",
                "lastUpdatedAt": 1700000000000,
                "conversation": [
                    {"type": "human", "text": "Fix the login bug", "timestamp": 1700000000000},
                    {"type": "cascade", "text": "I'll fix it", "timestamp": 1700000001000, "model": "claude-3-5-sonnet"}
                ]
            }],
            "conversations": []
        }"#;
        let result = parse_windsurf_chat_json(json);
        assert!(result.is_some());
        let sessions = result.unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].title, "Fix bug");
        assert_eq!(sessions[0].messages[1].role, "assistant");
    }

    #[test]
    fn test_parse_windsurf_empty() {
        let json = r#"{"tabs":[],"conversations":[]}"#;
        let result = parse_windsurf_chat_json(json);
        assert!(result.is_none());
    }

    #[test]
    fn test_windsurf_entry_cascade_role() {
        let entry = WindsurfEntry {
            entry_type: "cascade".to_string(),
            role: String::new(),
            text: "response".to_string(),
            content: String::new(),
            message: String::new(),
            timestamp: None,
            model: "claude-3-5-sonnet".to_string(),
        };
        assert_eq!(entry.effective_role(), "assistant");
    }
}
