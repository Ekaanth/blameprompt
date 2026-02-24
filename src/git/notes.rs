use crate::core::receipt::NotePayload;
use crate::commands::staging::StagingData;
use std::process::{Command, Stdio};

pub fn attach_receipts_to_head(staging: &StagingData) -> Result<(), String> {
    if staging.receipts.is_empty() {
        return Ok(());
    }

    let payload = NotePayload::new(staging.receipts.clone());
    let json = serde_json::to_string_pretty(&payload)
        .map_err(|e| format!("Failed to serialize: {}", e))?;

    let mut child = Command::new("git")
        .args(["notes", "--ref", "refs/notes/blameprompt", "add", "-f", "-F", "-", "HEAD"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to spawn git notes: {}", e))?;

    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        stdin
            .write_all(json.as_bytes())
            .map_err(|e| format!("Failed to write to stdin: {}", e))?;
    }

    let status = child.wait().map_err(|e| format!("Failed to wait: {}", e))?;
    if !status.success() {
        return Err("git notes add failed".to_string());
    }

    Ok(())
}

pub fn read_receipts_for_commit(sha: &str) -> Option<NotePayload> {
    let output = Command::new("git")
        .args(["notes", "--ref", "refs/notes/blameprompt", "show", sha])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let content = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(&content).ok()
}

#[allow(dead_code)]
pub fn list_commits_with_notes() -> Vec<String> {
    let output = Command::new("git")
        .args(["notes", "--ref", "refs/notes/blameprompt", "list"])
        .output();

    match output {
        Ok(o) if o.status.success() => {
            let text = String::from_utf8_lossy(&o.stdout);
            text.lines()
                .filter_map(|line| {
                    // Format: <note-object-sha> <commit-sha>
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    parts.get(1).map(|s| s.to_string())
                })
                .collect()
        }
        _ => Vec::new(),
    }
}
