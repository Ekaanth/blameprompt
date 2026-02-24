use crate::core::receipt::Receipt;
use serde::{Deserialize, Serialize};
use std::path::Path;

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

fn staging_dir() -> &'static str {
    ".blameprompt"
}

fn staging_path() -> String {
    format!("{}/staging.json", staging_dir())
}

fn ensure_staging_dir() {
    let dir = Path::new(staging_dir());
    if !dir.exists() {
        let _ = std::fs::create_dir_all(dir);
    }
    // Add to .gitignore if not present
    let gitignore = Path::new(".gitignore");
    let needs_entry = if gitignore.exists() {
        let content = std::fs::read_to_string(gitignore).unwrap_or_default();
        !content.lines().any(|l| l.trim() == ".blameprompt/" || l.trim() == ".blameprompt")
    } else {
        true
    };
    if needs_entry {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(gitignore)
            .ok();
        if let Some(ref mut f) = file {
            use std::io::Write;
            let _ = writeln!(f, "\n# BlamePrompt staging (auto-generated)\n.blameprompt/");
        }
    }
}

pub fn add_receipt(receipt: &Receipt) {
    ensure_staging_dir();
    let path = staging_path();
    let tmp_path = format!("{}/staging.json.tmp", staging_dir());

    let mut data = read_staging();
    data.receipts.push(receipt.clone());

    if let Ok(json) = serde_json::to_string_pretty(&data) {
        let _ = std::fs::write(&tmp_path, &json);
        let _ = std::fs::rename(&tmp_path, &path);
    }
}

pub fn read_staging() -> StagingData {
    let path = staging_path();
    match std::fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_else(|_| StagingData::empty()),
        Err(_) => StagingData::empty(),
    }
}

pub fn clear_staging() {
    ensure_staging_dir();
    let path = staging_path();
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
