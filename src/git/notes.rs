use crate::commands::staging::StagingData;
use crate::core::receipt::NotePayload;
use std::process::{Command, Stdio};

pub fn attach_receipts_to_head(staging: &StagingData) -> Result<(), String> {
    if staging.receipts.is_empty() {
        return Ok(());
    }

    // Merge with existing notes if present
    let mut receipts = if let Some(existing) = read_receipts_for_commit("HEAD") {
        existing.receipts
    } else {
        Vec::new()
    };

    // Add new receipts, avoiding duplicates by ID
    for r in &staging.receipts {
        if !receipts.iter().any(|existing| existing.id == r.id) {
            receipts.push(r.clone());
        }
    }

    let payload = NotePayload::new(receipts);
    let json = serde_json::to_string_pretty(&payload)
        .map_err(|e| format!("Failed to serialize: {}", e))?;

    let mut child = Command::new("git")
        .args([
            "notes",
            "--ref",
            "refs/notes/blameprompt",
            "add",
            "-f",
            "-F",
            "-",
            "HEAD",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to spawn git notes: {}", e))?;

    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        stdin
            .write_all(json.as_bytes())
            .map_err(|e| format!("Failed to write to stdin: {}", e))?;
    }

    let output = child
        .wait_with_output()
        .map_err(|e| format!("Failed to wait: {}", e))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git notes add failed: {}", stderr.trim()));
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
