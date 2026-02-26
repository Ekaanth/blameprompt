use crate::core::{receipt::NotePayload, util};
use crate::git::notes;
use std::collections::HashMap;
use std::io::BufRead;
use std::process::{Command, Stdio};

/// Process pairs from post-rewrite hook stdin and remap each note.
///
/// The post-rewrite hook writes "old-sha new-sha" pairs on stdin.
/// For each pair, we remap the BlamePrompt note from old to new, adjusting
/// line offsets in file_mappings when the commit diff shows context shifts.
pub fn run_from_stdin() {
    let stdin = std::io::stdin();
    for line in stdin.lock().lines().map_while(Result::ok) {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 {
            remap(parts[0], parts[1]);
        }
    }
}

/// Remap a BlamePrompt note from `old_sha` → `new_sha` with line-offset adjustment.
pub fn remap(old_sha: &str, new_sha: &str) {
    let payload = match notes::read_receipts_for_commit(old_sha) {
        Some(p) => p,
        None => return,
    };

    let remapped = remap_line_offsets(payload, old_sha, new_sha);

    if let Err(e) = write_note(new_sha, &remapped) {
        eprintln!(
            "[BlamePrompt] Failed to remap note {} → {}: {}",
            util::short_sha(old_sha),
            util::short_sha(new_sha),
            e
        );
        return;
    }

    // Remove the old note now that we've copied it to the new SHA
    let _ = Command::new("git")
        .args([
            "notes",
            "--ref",
            "refs/notes/blameprompt",
            "remove",
            old_sha,
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Adjust `file_mappings` hunk line numbers to account for context changes between
/// `old_sha` and `new_sha` (caused by rebase squash, fixup, or amend).
fn remap_line_offsets(mut payload: NotePayload, old_sha: &str, new_sha: &str) -> NotePayload {
    // Only remap if we have file_mappings with hunk data
    if payload.file_mappings.is_none() {
        return payload;
    }

    let diff = match get_diff_between_commits(old_sha, new_sha) {
        Some(d) if !d.trim().is_empty() => d,
        _ => return payload, // No diff — commits are identical content-wise
    };

    // Build per-file cumulative line offset table from the diff
    let offsets = build_offset_table(&diff);

    if offsets.is_empty() {
        return payload;
    }

    if let Some(ref mut mappings) = payload.file_mappings {
        for fm in mappings.iter_mut() {
            if let Some(file_offsets) = offsets.get(&fm.path) {
                for hunk in fm.hunks.iter_mut() {
                    // Find the cumulative offset that applies at this hunk's start line
                    let offset = file_offsets
                        .iter()
                        .filter(|(at_line, _)| *at_line <= hunk.start_line)
                        .map(|(_, delta)| delta)
                        .sum::<i64>();

                    if offset != 0 {
                        let new_start = (hunk.start_line as i64 + offset).max(1) as u32;
                        let span = hunk.end_line.saturating_sub(hunk.start_line);
                        hunk.start_line = new_start;
                        hunk.end_line = new_start + span;
                    }
                }
            }
        }
    }

    payload
}

/// Build a per-file table of (at_line, delta) pairs from a unified diff.
///
/// Each entry says: "starting at `at_line` in the new file, line numbers
/// are shifted by `delta` relative to the old file."
fn build_offset_table(diff: &str) -> HashMap<String, Vec<(u32, i64)>> {
    let mut table: HashMap<String, Vec<(u32, i64)>> = HashMap::new();
    let mut current_file: Option<String> = None;

    for line in diff.lines() {
        if let Some(path) = line.strip_prefix("+++ b/") {
            current_file = Some(path.to_string());
        } else if line.starts_with("@@ ") {
            if let Some(ref file) = current_file {
                let (_old_start, old_count, new_start, new_count) = parse_hunk_header(line);
                // Net change after this hunk: positive = lines added, negative = lines removed
                let net_delta = new_count as i64 - old_count as i64;
                let apply_at = new_start + new_count; // offset applies to lines after this hunk
                if net_delta != 0 {
                    table
                        .entry(file.clone())
                        .or_default()
                        .push((apply_at, net_delta));
                }
            }
        }
    }

    table
}

/// Parse `@@ -old_start[,old_count] +new_start[,new_count] @@`.
fn parse_hunk_header(line: &str) -> (u32, u32, u32, u32) {
    let mut old_start = 0u32;
    let mut old_count = 1u32;
    let mut new_start = 0u32;
    let mut new_count = 1u32;

    // Strip leading "@@" and trailing "@@" + context text
    let body = line.trim_start_matches('@').trim_start_matches(' ');
    for part in body.split_whitespace() {
        if let Some(nums) = part.strip_prefix('-') {
            let v: Vec<&str> = nums.split(',').collect();
            old_start = v.first().and_then(|s| s.parse().ok()).unwrap_or(0);
            old_count = v.get(1).and_then(|s| s.parse().ok()).unwrap_or(1);
        } else if let Some(nums) = part.strip_prefix('+') {
            let v: Vec<&str> = nums.split(',').collect();
            new_start = v.first().and_then(|s| s.parse().ok()).unwrap_or(0);
            new_count = v.get(1).and_then(|s| s.parse().ok()).unwrap_or(1);
        } else if part == "@@" {
            break; // End of coordinate section
        }
    }

    (old_start, old_count, new_start, new_count)
}

fn get_diff_between_commits(old_sha: &str, new_sha: &str) -> Option<String> {
    let output = Command::new("git")
        .args(["diff", "--unified=0", old_sha, new_sha])
        .output()
        .ok()?;

    if output.status.success() {
        String::from_utf8(output.stdout).ok()
    } else {
        None
    }
}

fn write_note(sha: &str, payload: &NotePayload) -> Result<(), String> {
    use std::io::Write;

    let json =
        serde_json::to_string_pretty(payload).map_err(|e| format!("Serialize error: {}", e))?;

    let mut child = Command::new("git")
        .args([
            "notes",
            "--ref",
            "refs/notes/blameprompt",
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hunk_header() {
        assert_eq!(parse_hunk_header("@@ -1,3 +1,5 @@"), (1, 3, 1, 5));
        assert_eq!(parse_hunk_header("@@ -10,2 +12,4 @@"), (10, 2, 12, 4));
        assert_eq!(parse_hunk_header("@@ -5 +5 @@"), (5, 1, 5, 1));
    }

    #[test]
    fn test_build_offset_table_additions() {
        let diff =
            "+++ b/src/main.rs\n@@ -10,2 +10,5 @@ fn foo() {\n+    line1\n+    line2\n+    line3\n";
        let table = build_offset_table(diff);
        assert!(table.contains_key("src/main.rs"));
        let offsets = &table["src/main.rs"];
        // net delta = 5 - 2 = 3 added lines, apply at line 15
        assert_eq!(offsets[0], (15, 3));
    }
}
