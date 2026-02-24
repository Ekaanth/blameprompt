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
    if let Some(existing) = data.receipts.iter_mut().find(|r| {
        r.session_id == receipt.session_id && r.prompt_number == receipt.prompt_number
    }) {
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

        // Update the receipt in place
        *existing = receipt.clone();
        existing.id = original_id;
        existing.parent_receipt_id = original_parent;
        existing.files_changed = merged_files;

        // Keep legacy fields pointing at first file
        if let Some(first) = existing.files_changed.first() {
            existing.file_path = first.path.clone();
            existing.line_range = first.line_range;
        }
    } else {
        // New prompt — find parent (previous receipt in this session or different session)
        let mut new_receipt = receipt.clone();
        new_receipt.parent_receipt_id = data
            .receipts
            .last()
            .map(|r| r.id.clone());
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

    #[test]
    fn test_staging_roundtrip() {
        let data = StagingData::empty();
        let json = serde_json::to_string(&data).unwrap();
        let parsed: StagingData = serde_json::from_str(&json).unwrap();
        assert!(parsed.receipts.is_empty());
    }
}
