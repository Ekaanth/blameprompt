/// Agent Trace v0.1.0 spec implementation.
///
/// Implements the open interoperability standard for AI code provenance.
/// Spec: https://github.com/cursor/agent-trace
///
/// Records are stored in `refs/notes/agent-trace` git notes, one per commit.
use crate::core::{receipt::Receipt, util};
use crate::git::notes::read_receipts_for_commit;
use serde::{Deserialize, Serialize};
use std::process::{Command, Stdio};

/// Agent Trace v0.1.0 record (one per commit).
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TraceRecord {
    pub version: String,
    pub id: String,
    pub timestamp: String,
    pub vcs: VcsInfo,
    pub tool: ToolInfo,
    pub files: Vec<TracedFile>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct VcsInfo {
    #[serde(rename = "type")]
    pub vcs_type: String,
    pub revision: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ToolInfo {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TracedFile {
    pub path: String,
    pub conversations: Vec<FileConversation>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FileConversation {
    pub contributor: String, // "ai", "human", "mixed"
    pub model_id: String,    // e.g. "anthropic/claude-sonnet-4-6"
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub ranges: Vec<LineRange>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LineRange {
    pub start_line: u32,
    pub end_line: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
}

/// Convert blameprompt receipts to an Agent Trace record.
pub fn to_agent_trace(receipts: &[Receipt], commit_sha: &str) -> TraceRecord {
    let timestamp = receipts
        .first()
        .map(|r| r.timestamp.to_rfc3339())
        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());

    let id = uuid::Uuid::new_v4().to_string();

    // Group files across all receipts
    let mut file_map: std::collections::HashMap<String, Vec<FileConversation>> =
        std::collections::HashMap::new();

    for receipt in receipts {
        for fc in receipt.all_file_changes() {
            // Convert provider/model to models.dev format: "provider/model-name"
            let model_id = normalize_model_id(&receipt.provider, &receipt.model);
            let range = LineRange {
                start_line: fc.line_range.0.max(1),
                end_line: fc.line_range.1.max(1),
                content_hash: fc.blob_hash.clone(),
            };
            let conv = FileConversation {
                contributor: "ai".to_string(),
                model_id,
                ranges: vec![range],
            };
            file_map.entry(fc.path.clone()).or_default().push(conv);
        }
    }

    let files: Vec<TracedFile> = file_map
        .into_iter()
        .map(|(path, conversations)| TracedFile { path, conversations })
        .collect();

    TraceRecord {
        version: "0.1.0".to_string(),
        id,
        timestamp,
        vcs: VcsInfo {
            vcs_type: "git".to_string(),
            revision: commit_sha.to_string(),
        },
        tool: ToolInfo {
            name: "blameprompt".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
        files,
    }
}

/// Convert provider + model name to models.dev format "provider/model-name".
fn normalize_model_id(provider: &str, model: &str) -> String {
    // If model already contains a slash (already namespaced), return as-is
    if model.contains('/') {
        return model.to_string();
    }
    let provider_lower = provider.to_lowercase();
    let p = match provider_lower.as_str() {
        "claude" | "anthropic" => "anthropic",
        "openai" | "gpt" | "codex" => "openai",
        "cursor" => "cursor",
        "copilot" | "github" => "github",
        "gemini" | "google" => "google",
        "windsurf" | "codeium" => "codeium",
        other => other,
    };
    format!("{}/{}", p, model)
}

/// Write a TraceRecord to `refs/notes/agent-trace` for the given commit SHA.
pub fn write_to_git_notes(sha: &str, record: &TraceRecord) -> Result<(), String> {
    use std::io::Write;

    let json = serde_json::to_string_pretty(record)
        .map_err(|e| format!("Serialize error: {}", e))?;

    let mut child = Command::new("git")
        .args([
            "notes",
            "--ref",
            "refs/notes/agent-trace",
            "add",
            "-f",
            "-F",
            "-",
            sha,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to spawn git: {}", e))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(json.as_bytes())
            .map_err(|e| format!("Write error: {}", e))?;
    }

    let status = child.wait().map_err(|e| format!("Wait error: {}", e))?;
    if !status.success() {
        return Err("git notes add failed".to_string());
    }
    Ok(())
}

/// Read a TraceRecord from `refs/notes/agent-trace` for a given commit SHA.
pub fn read_from_git_notes(sha: &str) -> Option<TraceRecord> {
    let output = Command::new("git")
        .args(["notes", "--ref", "refs/notes/agent-trace", "show", sha])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let content = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(&content).ok()
}

/// Export: convert blameprompt notes for a commit to agent-trace format and write.
pub fn run_export(commit_ref: Option<&str>) {
    let sha = resolve_sha(commit_ref);

    let payload = match read_receipts_for_commit(&sha) {
        Some(p) => p,
        None => {
            eprintln!("[agent-trace] No blameprompt note found for {}", util::short_sha(&sha));
            return;
        }
    };

    let record = to_agent_trace(&payload.receipts, &sha);
    match write_to_git_notes(&sha, &record) {
        Ok(()) => {
            println!(
                "[agent-trace] Exported {} file(s) to refs/notes/agent-trace for {}",
                record.files.len(),
                util::short_sha(&sha)
            );
        }
        Err(e) => eprintln!("[agent-trace] Export failed: {}", e),
    }
}

/// Import: read agent-trace note for a commit and display it.
pub fn run_import(commit_ref: Option<&str>) {
    let sha = resolve_sha(commit_ref);

    match read_from_git_notes(&sha) {
        Some(record) => {
            println!(
                "Agent Trace v{} — commit {}",
                record.version,
                util::short_sha(&sha)
            );
            println!("Tool: {}/{}", record.tool.name, record.tool.version);
            println!("Timestamp: {}", record.timestamp);
            println!();
            for f in &record.files {
                println!("  {}", f.path);
                for conv in &f.conversations {
                    println!(
                        "    contributor={} model={}",
                        conv.contributor, conv.model_id
                    );
                    for r in &conv.ranges {
                        println!("      lines {}-{}", r.start_line, r.end_line);
                    }
                }
            }
        }
        None => {
            eprintln!(
                "[agent-trace] No agent-trace note found for {}",
                util::short_sha(&sha)
            );
        }
    }
}

fn resolve_sha(commit_ref: Option<&str>) -> String {
    let r = commit_ref.unwrap_or("HEAD");
    Command::new("git")
        .args(["rev-parse", r])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| r.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_model_id() {
        assert_eq!(normalize_model_id("claude", "claude-sonnet-4-6"), "anthropic/claude-sonnet-4-6");
        assert_eq!(normalize_model_id("openai", "gpt-4o"), "openai/gpt-4o");
        assert_eq!(normalize_model_id("cursor", "claude-3-5-sonnet"), "cursor/claude-3-5-sonnet");
        assert_eq!(normalize_model_id("codex", "gpt-4.1"), "openai/gpt-4.1");
        assert_eq!(normalize_model_id("gemini", "gemini-2.5-pro"), "google/gemini-2.5-pro");
        assert_eq!(normalize_model_id("windsurf", "claude-3-5-sonnet"), "codeium/claude-3-5-sonnet");
        assert_eq!(normalize_model_id("copilot", "gpt-4o"), "github/gpt-4o");
        // Already namespaced — pass through
        assert_eq!(normalize_model_id("anthropic", "anthropic/claude-3"), "anthropic/claude-3");
    }

    #[test]
    fn test_to_agent_trace_empty() {
        let record = to_agent_trace(&[], "abc123");
        assert_eq!(record.version, "0.1.0");
        assert_eq!(record.vcs.vcs_type, "git");
        assert_eq!(record.vcs.revision, "abc123");
        assert!(record.files.is_empty());
    }
}
