use crate::core::receipt::Receipt;
use crate::git::notes;
use rusqlite::{params, Connection};
use std::path::PathBuf;

fn db_path() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let dir = home.join(".blameprompt");
    let _ = std::fs::create_dir_all(&dir);
    dir.join("prompts.db")
}

pub fn get_connection() -> Result<Connection, String> {
    let path = db_path();
    let conn = Connection::open(&path).map_err(|e| format!("Cannot open database: {}", e))?;

    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS receipts (
            id TEXT PRIMARY KEY,
            commit_sha TEXT,
            provider TEXT NOT NULL,
            model TEXT NOT NULL,
            session_id TEXT NOT NULL,
            prompt_summary TEXT,
            prompt_hash TEXT,
            message_count INTEGER,
            cost_usd REAL,
            timestamp TEXT NOT NULL,
            session_start TEXT,
            session_end TEXT,
            session_duration_secs INTEGER,
            ai_response_time_secs REAL,
            user TEXT,
            file_path TEXT,
            line_start INTEGER,
            line_end INTEGER,
            parent_receipt_id TEXT
        );",
    )
    .map_err(|e| format!("Cannot create table: {}", e))?;

    Ok(conn)
}

pub fn insert_receipt(conn: &Connection, commit_sha: &str, r: &Receipt) -> Result<(), String> {
    conn.execute(
        "INSERT OR REPLACE INTO receipts (id, commit_sha, provider, model, session_id, prompt_summary, prompt_hash, message_count, cost_usd, timestamp, session_start, session_end, session_duration_secs, ai_response_time_secs, user, file_path, line_start, line_end, parent_receipt_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
        params![
            r.id,
            commit_sha,
            r.provider,
            r.model,
            r.session_id,
            r.prompt_summary,
            r.prompt_hash,
            r.message_count,
            r.cost_usd,
            r.timestamp.to_rfc3339(),
            r.session_start.map(|t| t.to_rfc3339()),
            r.session_end.map(|t| t.to_rfc3339()),
            r.session_duration_secs,
            r.ai_response_time_secs,
            r.user,
            r.file_path,
            r.line_range.0,
            r.line_range.1,
            r.parent_receipt_id,
        ],
    ).map_err(|e| format!("Cannot insert receipt: {}", e))?;

    Ok(())
}

/// Sync all Git Notes into the SQLite cache.
pub fn sync_from_notes() -> Result<(), String> {
    let conn = get_connection()?;
    let commits = notes::list_commits_with_notes();

    if commits.is_empty() {
        println!("[BlamePrompt] No notes found to cache.");
        return Ok(());
    }

    let mut count = 0;
    for sha in &commits {
        if let Some(payload) = notes::read_receipts_for_commit(sha) {
            for receipt in &payload.receipts {
                insert_receipt(&conn, sha, receipt)?;
                count += 1;
            }
        }
    }

    println!(
        "[BlamePrompt] Cached {} receipt(s) from {} commit(s) into SQLite.",
        count,
        commits.len()
    );
    Ok(())
}

/// Search prompt summaries in the SQLite cache.
#[allow(dead_code)]
pub fn search_prompts(query: &str, limit: usize) -> Result<Vec<(String, Receipt)>, String> {
    let conn = get_connection()?;

    let mut stmt = conn.prepare(
        "SELECT commit_sha, id, provider, model, session_id, prompt_summary, prompt_hash, message_count, cost_usd, timestamp, session_start, session_end, session_duration_secs, ai_response_time_secs, user, file_path, line_start, line_end, parent_receipt_id FROM receipts WHERE prompt_summary LIKE ?1 OR file_path LIKE ?1 OR model LIKE ?1 ORDER BY timestamp DESC LIMIT ?2"
    ).map_err(|e| format!("Query error: {}", e))?;

    let pattern = format!("%{}%", query);
    let rows = stmt
        .query_map(params![pattern, limit as i64], |row| {
            let commit_sha: String = row.get(0)?;
            let timestamp_str: String = row.get(9)?;
            let session_start_str: Option<String> = row.get(10)?;
            let session_end_str: Option<String> = row.get(11)?;

            let timestamp = chrono::DateTime::parse_from_rfc3339(&timestamp_str)
                .map(|dt| dt.with_timezone(&chrono::Utc))
                .unwrap_or_else(|_| chrono::Utc::now());

            let session_start = session_start_str.and_then(|s| {
                chrono::DateTime::parse_from_rfc3339(&s)
                    .ok()
                    .map(|dt| dt.with_timezone(&chrono::Utc))
            });
            let session_end = session_end_str.and_then(|s| {
                chrono::DateTime::parse_from_rfc3339(&s)
                    .ok()
                    .map(|dt| dt.with_timezone(&chrono::Utc))
            });

            let line_start: u32 = row.get(16)?;
            let line_end: u32 = row.get(17)?;

            Ok((
                commit_sha,
                Receipt {
                    id: row.get(1)?,
                    provider: row.get(2)?,
                    model: row.get(3)?,
                    session_id: row.get(4)?,
                    prompt_summary: row.get(5)?,
                    prompt_hash: row.get(6)?,
                    message_count: row.get(7)?,
                    cost_usd: row.get(8)?,
                    timestamp,
                    session_start,
                    session_end,
                    session_duration_secs: row.get(12)?,
                    ai_response_time_secs: row.get(13)?,
                    user: row.get(14)?,
                    file_path: row.get(15)?,
                    line_range: (line_start, line_end),
                    parent_receipt_id: row.get(18)?,
                    prompt_number: None,
                    tools_used: vec![],
                    mcp_servers: vec![],
                    agents_spawned: vec![],
                    files_changed: vec![], // SQLite cache uses legacy file_path/line_range
                    conversation: None,    // SQLite cache doesn't store conversation turns
                },
            ))
        })
        .map_err(|e| format!("Query error: {}", e))?;

    let mut results = Vec::new();
    for r in rows.flatten() {
        results.push(r);
    }

    Ok(results)
}
