use crate::core::receipt::Receipt;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Deserialize)]
pub struct StagingData {
    pub receipts: Vec<Receipt>,
}

impl StagingData {
    pub fn empty() -> Self {
        StagingData {
            receipts: Vec::new(),
        }
    }
}

fn staging_dir_in(base: &Path) -> PathBuf {
    base.join(".blameprompt")
}

fn staging_path_in(base: &Path) -> PathBuf {
    staging_dir_in(base).join("staging.json")
}

fn ensure_staging_dir_in(base: &Path) {
    let dir = staging_dir_in(base);
    if !dir.exists() {
        let _ = std::fs::create_dir_all(&dir);
    }
    // Add to .gitignore if not present
    let gitignore = base.join(".gitignore");
    let needs_entry = if gitignore.exists() {
        let content = std::fs::read_to_string(&gitignore).unwrap_or_default();
        !content
            .lines()
            .any(|l| l.trim() == ".blameprompt/" || l.trim() == ".blameprompt")
    } else {
        true
    };
    if needs_entry {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&gitignore)
            .ok();
        if let Some(ref mut f) = file {
            use std::io::Write;
            let _ = writeln!(f, "\n# BlamePrompt staging (auto-generated)\n.blameprompt/");
        }
    }
}

/// Insert or update a receipt in the staging file at `base_dir`.
/// Deduplicates by (session_id, prompt_number) so each user prompt in a session
/// creates a separate receipt. Multiple tool uses within the same prompt merge
/// their files_changed.
pub fn upsert_receipt_in(receipt: &Receipt, base_dir: &str) {
    let base = Path::new(base_dir);
    ensure_staging_dir_in(base);
    let path = staging_path_in(base);
    let tmp_path = staging_dir_in(base).join("staging.json.tmp");

    let mut data = read_staging_in(base);

    // Look for an existing receipt with same (session_id, prompt_number)
    if let Some(existing) = data
        .receipts
        .iter_mut()
        .find(|r| r.session_id == receipt.session_id && r.prompt_number == receipt.prompt_number)
    {
        let original_id = existing.id.clone();
        let original_parent = existing.parent_receipt_id.clone();

        // Merge files_changed: add any new files from the incoming receipt
        let mut merged_files = existing.files_changed.clone();
        for fc in &receipt.files_changed {
            if let Some(pos) = merged_files.iter().position(|f| f.path == fc.path) {
                // Update existing file's line range
                merged_files[pos] = fc.clone();
            } else {
                merged_files.push(fc.clone());
            }
        }

        // Preserve fields from the existing receipt that the incoming update leaves blank.
        // This lets intermediate patches (e.g. git-sweep, UserPromptSubmit) refine
        // only the fields they know about without erasing prior richer values.
        let keep_summary = if receipt.prompt_summary.is_empty() {
            existing.prompt_summary.clone()
        } else {
            receipt.prompt_summary.clone()
        };
        let keep_conversation = if receipt.conversation.is_none() {
            existing.conversation.clone()
        } else {
            receipt.conversation.clone()
        };
        let keep_tools = if receipt.tools_used.is_empty() {
            existing.tools_used.clone()
        } else {
            receipt.tools_used.clone()
        };
        let keep_mcps = if receipt.mcp_servers.is_empty() {
            existing.mcp_servers.clone()
        } else {
            receipt.mcp_servers.clone()
        };
        let keep_agents = if receipt.agents_spawned.is_empty() {
            existing.agents_spawned.clone()
        } else {
            receipt.agents_spawned.clone()
        };
        let keep_cost = if receipt.cost_usd == 0.0 {
            existing.cost_usd
        } else {
            receipt.cost_usd
        };
        let keep_session_end = receipt.session_end.or(existing.session_end);
        // Preserve prompt_submitted_at: set once at UserPromptSubmit, never overwritten.
        let keep_prompt_submitted_at = existing
            .prompt_submitted_at
            .or(receipt.prompt_submitted_at);
        // Preserve prompt_duration_secs: set at Stop, keep if already computed.
        let keep_prompt_duration_secs = receipt.prompt_duration_secs.or(existing.prompt_duration_secs);
        // Preserve diff totals: use the incoming value unless it is zero, in which case
        // keep whatever was previously recorded (e.g. from PostToolUse).
        let keep_total_additions = if receipt.total_additions > 0 {
            receipt.total_additions
        } else {
            existing.total_additions
        };
        let keep_total_deletions = if receipt.total_deletions > 0 {
            receipt.total_deletions
        } else {
            existing.total_deletions
        };

        // Update the receipt in place
        *existing = receipt.clone();
        existing.id = original_id;
        existing.parent_receipt_id = original_parent;
        existing.files_changed = merged_files;
        existing.prompt_summary = keep_summary;
        existing.conversation = keep_conversation;
        existing.tools_used = keep_tools;
        existing.mcp_servers = keep_mcps;
        existing.agents_spawned = keep_agents;
        existing.cost_usd = keep_cost;
        existing.session_end = keep_session_end;
        existing.prompt_submitted_at = keep_prompt_submitted_at;
        existing.prompt_duration_secs = keep_prompt_duration_secs;
        existing.total_additions = keep_total_additions;
        existing.total_deletions = keep_total_deletions;

        // Keep legacy fields pointing at first file
        if let Some(first) = existing.files_changed.first() {
            existing.file_path = first.path.clone();
            existing.line_range = first.line_range;
        }
    } else {
        // New prompt — find parent (previous receipt in this session or different session)
        let mut new_receipt = receipt.clone();
        new_receipt.parent_receipt_id = data.receipts.last().map(|r| r.id.clone());
        data.receipts.push(new_receipt);
    }

    write_staging_data(&data, &path, &tmp_path);
}

/// Insert or update a receipt using the current working directory.
pub fn upsert_receipt(receipt: &Receipt) {
    upsert_receipt_in(receipt, ".");
}

fn write_staging_data(data: &StagingData, path: &Path, tmp_path: &Path) {
    match serde_json::to_string_pretty(data) {
        Ok(json) => {
            if let Err(e) = std::fs::write(tmp_path, &json) {
                eprintln!("[blameprompt] Failed to write staging file: {}", e);
                return;
            }
            if let Err(e) = std::fs::rename(tmp_path, path) {
                eprintln!("[blameprompt] Failed to rename staging file: {}", e);
            }
        }
        Err(e) => {
            eprintln!("[blameprompt] Failed to serialize staging data: {}", e);
        }
    }
}

pub fn read_staging_in(base: &Path) -> StagingData {
    let path = staging_path_in(base);
    match std::fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_else(|_| StagingData::empty()),
        Err(_) => StagingData::empty(),
    }
}

pub fn read_staging() -> StagingData {
    read_staging_in(Path::new("."))
}

pub fn clear_staging() {
    let base = Path::new(".");
    ensure_staging_dir_in(base);
    let path = staging_path_in(base);
    let data = StagingData::empty();
    if let Ok(json) = serde_json::to_string_pretty(&data) {
        let _ = std::fs::write(&path, json);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::receipt::FileChange;
    use chrono::Utc;

    fn make_receipt(session_id: &str, pn: u32) -> Receipt {
        Receipt {
            id: Receipt::new_id(),
            provider: "claude".to_string(),
            model: "test".to_string(),
            session_id: session_id.to_string(),
            prompt_summary: "original".to_string(),
            prompt_hash: "h".to_string(),
            message_count: 1,
            cost_usd: 0.0,
            timestamp: Utc::now(),
            session_start: None,
            session_end: None,
            session_duration_secs: None,
            ai_response_time_secs: None,
            user: "u".to_string(),
            file_path: String::new(),
            line_range: (0, 0),
            files_changed: vec![],
            parent_receipt_id: None,
            prompt_number: Some(pn),
            total_additions: 0,
            total_deletions: 0,
            tools_used: vec![],
            mcp_servers: vec![],
            agents_spawned: vec![],
            conversation: None,
            prompt_submitted_at: None,
            prompt_duration_secs: None,
        }
    }

    #[test]
    fn test_staging_roundtrip() {
        let data = StagingData::empty();
        let json = serde_json::to_string(&data).unwrap();
        let parsed: StagingData = serde_json::from_str(&json).unwrap();
        assert!(parsed.receipts.is_empty());
    }

    #[test]
    fn test_upsert_preserves_prompt_summary_on_empty_update() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_str().unwrap();

        // First upsert: initial receipt with a real summary (from UserPromptSubmit)
        let mut r = make_receipt("s1", 1);
        r.prompt_summary = "fix the tests".to_string();
        upsert_receipt_in(&r, dir);

        // Second upsert: patch with empty summary (e.g. git-sweep patch)
        let mut patch = make_receipt("s1", 1);
        patch.prompt_summary = String::new();
        patch.files_changed = vec![FileChange {
            path: "src/lib.rs".to_string(),
            line_range: (1, 5),
            blob_hash: None,
            additions: 5,
            deletions: 0,
        }];
        upsert_receipt_in(&patch, dir);

        let data = read_staging_in(tmp.path());
        let receipt = &data.receipts[0];
        // Summary should be preserved from the first upsert
        assert_eq!(receipt.prompt_summary, "fix the tests");
        // File should be merged in
        assert_eq!(receipt.files_changed.len(), 1);
    }

    #[test]
    fn test_upsert_preserves_files_on_stop_finalize() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_str().unwrap();

        // PostToolUse creates receipt with file change
        let mut r = make_receipt("s1", 1);
        r.prompt_summary = "add feature".to_string();
        r.files_changed = vec![FileChange {
            path: "src/main.rs".to_string(),
            line_range: (1, 10),
            blob_hash: None,
            additions: 10,
            deletions: 0,
        }];
        r.total_additions = 10;
        upsert_receipt_in(&r, dir);

        // Stop finalizes with conversation but empty files_changed
        let mut stop = make_receipt("s1", 1);
        stop.prompt_summary = "add feature".to_string();
        stop.cost_usd = 0.05;
        stop.files_changed = vec![];
        upsert_receipt_in(&stop, dir);

        let data = read_staging_in(tmp.path());
        let receipt = &data.receipts[0];
        // Files from PostToolUse should still be there
        assert_eq!(receipt.files_changed.len(), 1);
        assert_eq!(receipt.files_changed[0].path, "src/main.rs");
        // Cost from Stop should be applied
        assert!((receipt.cost_usd - 0.05).abs() < 0.001);
        // total_additions should be preserved
        assert_eq!(receipt.total_additions, 10);
    }
}
