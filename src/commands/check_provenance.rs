/// Check AI provenance for a file or specific line.
///
/// Cross-references `git blame` (line → commit SHA) with blameprompt git notes
/// (commit SHA → receipts) to show which lines are AI-generated, by which model,
/// and which receipt they belong to.
use crate::core::util;
use crate::git::notes::read_receipts_for_commit;
use std::process::Command;

#[derive(Debug)]
pub struct LineProvenance {
    pub line_number: u32,
    pub content: String,
    pub commit_sha: String,
    pub author: String,
    pub is_ai: bool,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub receipt_id: Option<String>,
    pub prompt_summary: Option<String>,
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct ProvenanceRange {
    pub start_line: u32,
    pub end_line: u32,
    pub is_ai: bool,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub receipt_id: Option<String>,
    pub prompt_summary: Option<String>,
    pub commit_sha: String,
    pub author: String,
}

/// Run provenance check for a file, optionally filtered to a single line.
pub fn run(file: &str, line_number: Option<u32>) {
    let provenance = match compute_provenance(file) {
        Some(p) => p,
        None => {
            eprintln!("[blameprompt] Cannot compute provenance for '{}'. Is this a tracked file in a git repository?", file);
            std::process::exit(1);
        }
    };

    if let Some(ln) = line_number {
        // Show single line
        let entry = provenance.iter().find(|p| p.line_number == ln);
        match entry {
            Some(p) => print_single_line(p),
            None => eprintln!("[blameprompt] Line {} not found in '{}'", ln, file),
        }
    } else {
        // Show range summary
        let ranges = collapse_to_ranges(&provenance);
        print_ranges(file, &ranges);
    }
}

/// Parse `git blame --porcelain` for the file and cross-reference with blameprompt notes.
pub fn compute_provenance(file: &str) -> Option<Vec<LineProvenance>> {
    let output = Command::new("git")
        .args(["blame", "--porcelain", file])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let blame_text = String::from_utf8_lossy(&output.stdout);
    let blame_entries = parse_blame_porcelain(&blame_text);

    // Cache: commit_sha → Option<receipts payload>
    let mut note_cache: std::collections::HashMap<String, bool> = std::collections::HashMap::new();
    // Cache: commit_sha → Vec<(file, start_line, end_line, model, provider, receipt_id, summary)>
    #[allow(clippy::type_complexity)]
    let mut receipt_cache: std::collections::HashMap<
        String,
        Vec<(String, u32, u32, String, String, String, String)>,
    > = std::collections::HashMap::new();

    let mut result = Vec::new();

    for entry in blame_entries {
        let sha = entry.commit_sha.clone();

        // Populate cache for this commit if not done yet
        if !note_cache.contains_key(&sha) {
            match read_receipts_for_commit(&sha) {
                Some(payload) => {
                    note_cache.insert(sha.clone(), true);
                    let mut hits = Vec::new();
                    for receipt in &payload.receipts {
                        for fc in receipt.all_file_changes() {
                            hits.push((
                                fc.path.clone(),
                                fc.line_range.0,
                                fc.line_range.1,
                                receipt.model.clone(),
                                receipt.provider.clone(),
                                receipt.id.clone(),
                                receipt.prompt_summary.clone(),
                            ));
                        }
                    }
                    receipt_cache.insert(sha.clone(), hits);
                }
                None => {
                    note_cache.insert(sha.clone(), false);
                }
            }
        }

        let (is_ai, model, provider, receipt_id, prompt_summary) =
            if note_cache.get(&sha).copied().unwrap_or(false) {
                if let Some(hits) = receipt_cache.get(&sha) {
                    // Normalize file path for comparison
                    let norm_file = normalize_file_path(file);
                    find_matching_receipt(hits, &norm_file, entry.line_number)
                } else {
                    (false, None, None, None, None)
                }
            } else {
                (false, None, None, None, None)
            };

        result.push(LineProvenance {
            line_number: entry.line_number,
            content: entry.content.clone(),
            commit_sha: sha,
            author: entry.author.clone(),
            is_ai,
            model,
            provider,
            receipt_id,
            prompt_summary,
        });
    }

    Some(result)
}

/// Check if a receipt range covers the given line in the given file.
fn find_matching_receipt(
    hits: &[(String, u32, u32, String, String, String, String)],
    file: &str,
    line: u32,
) -> (
    bool,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
) {
    for (path, start, end, model, provider, receipt_id, summary) in hits {
        // Match on file path (basename or full)
        let hit_norm = normalize_file_path(path);
        let file_match = hit_norm == file || path.ends_with(file) || file.ends_with(path.as_str());

        // If receipt has line range 1..1 treat as "whole file" (no range info)
        let range_match = (*start == 1 && *end <= 1) || (line >= *start && line <= *end);

        if file_match && range_match {
            return (
                true,
                Some(model.clone()),
                Some(provider.clone()),
                Some(receipt_id.clone()),
                Some(summary.clone()),
            );
        }
    }
    (false, None, None, None, None)
}

fn normalize_file_path(path: &str) -> String {
    // Strip leading ./ for comparison
    path.trim_start_matches("./").to_string()
}

/// Collapse line-by-line provenance into contiguous ranges with the same attribution.
pub fn collapse_to_ranges(lines: &[LineProvenance]) -> Vec<ProvenanceRange> {
    let mut ranges: Vec<ProvenanceRange> = Vec::new();

    for lp in lines {
        let same_range = ranges.last_mut().filter(|r| {
            r.is_ai == lp.is_ai
                && r.model == lp.model
                && r.receipt_id == lp.receipt_id
                && r.commit_sha == lp.commit_sha
        });

        if let Some(range) = same_range {
            range.end_line = lp.line_number;
        } else {
            ranges.push(ProvenanceRange {
                start_line: lp.line_number,
                end_line: lp.line_number,
                is_ai: lp.is_ai,
                model: lp.model.clone(),
                provider: lp.provider.clone(),
                receipt_id: lp.receipt_id.clone(),
                prompt_summary: lp.prompt_summary.clone(),
                commit_sha: lp.commit_sha.clone(),
                author: lp.author.clone(),
            });
        }
    }

    ranges
}

fn print_ranges(file: &str, ranges: &[ProvenanceRange]) {
    let ai_lines: u32 = ranges
        .iter()
        .filter(|r| r.is_ai)
        .map(|r| r.end_line - r.start_line + 1)
        .sum();
    let total_lines: u32 = ranges.iter().map(|r| r.end_line - r.start_line + 1).sum();
    let human_lines = total_lines.saturating_sub(ai_lines);
    let ai_pct = if total_lines > 0 {
        (ai_lines as f64 / total_lines as f64) * 100.0
    } else {
        0.0
    };

    println!();
    println!("  Provenance: {}", file);
    println!("  ─────────────────────────────────────────────");
    println!(
        "  AI: {} lines ({:.0}%)   Human: {} lines ({:.0}%)",
        ai_lines,
        ai_pct,
        human_lines,
        100.0 - ai_pct
    );
    println!();

    for range in ranges {
        let sha_short = util::short_sha(&range.commit_sha);
        if range.is_ai {
            let model = range.model.as_deref().unwrap_or("unknown");
            let summary = range
                .prompt_summary
                .as_deref()
                .unwrap_or("")
                .chars()
                .take(50)
                .collect::<String>();
            println!(
                "  \x1b[33m[AI]\x1b[0m  lines {:>5}-{:<5}  {} ({}) — {}",
                range.start_line, range.end_line, model, sha_short, summary
            );
        } else {
            println!(
                "  \x1b[34m[HU]\x1b[0m  lines {:>5}-{:<5}  {} ({})",
                range.start_line, range.end_line, range.author, sha_short
            );
        }
    }
    println!();
}

fn print_single_line(lp: &LineProvenance) {
    let sha_short = util::short_sha(&lp.commit_sha);
    println!();
    println!("  Line {}: {}", lp.line_number, lp.content.trim_end());
    println!("  Commit: {}", sha_short);
    println!("  Author: {}", lp.author);
    if lp.is_ai {
        println!("  Origin: \x1b[33mAI-generated\x1b[0m");
        if let Some(ref m) = lp.model {
            println!("  Model:  {}", m);
        }
        if let Some(ref s) = lp.prompt_summary {
            println!("  Prompt: {}", s);
        }
        if let Some(ref id) = lp.receipt_id {
            println!("  Receipt: {}", id);
        }
    } else {
        println!("  Origin: \x1b[34mHuman-written\x1b[0m");
    }
    println!();
}

// ── git blame --porcelain parser ──────────────────────────────────────────

struct BlameEntry {
    commit_sha: String,
    line_number: u32,
    author: String,
    content: String,
}

fn parse_blame_porcelain(text: &str) -> Vec<BlameEntry> {
    let mut entries: Vec<BlameEntry> = Vec::new();
    let mut current_sha = String::new();
    let mut current_line = 0u32;
    let mut current_author = String::new();

    for line in text.lines() {
        if let Some(stripped) = line.strip_prefix('\t') {
            // Tab-prefixed lines are the actual source content
            entries.push(BlameEntry {
                commit_sha: current_sha.clone(),
                line_number: current_line,
                author: current_author.clone(),
                content: stripped.to_string(),
            });
        } else if let Some(rest) = line.strip_prefix("author ") {
            current_author = rest.to_string();
        } else {
            // Lines like "<sha> <orig_line> <final_line> [<count>]"
            let parts: Vec<&str> = line.splitn(4, ' ').collect();
            if parts.len() >= 3 && parts[0].len() == 40 {
                current_sha = parts[0].to_string();
                current_line = parts[2].parse().unwrap_or(0);
            }
        }
    }

    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_blame_porcelain() {
        let input = "abc1234500000000000000000000000000000000 1 1 1\nauthor Alice\n\tlet x = 1;\n";
        let entries = parse_blame_porcelain(input);
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].commit_sha,
            "abc1234500000000000000000000000000000000"
        );
        assert_eq!(entries[0].line_number, 1);
        assert_eq!(entries[0].author, "Alice");
        assert_eq!(entries[0].content, "let x = 1;");
    }

    #[test]
    fn test_collapse_ranges_same_attribution() {
        let lines: Vec<LineProvenance> = (1u32..=5)
            .map(|i| LineProvenance {
                line_number: i,
                content: format!("line {}", i),
                commit_sha: "abc".to_string(),
                author: "Alice".to_string(),
                is_ai: true,
                model: Some("claude-sonnet".to_string()),
                provider: Some("claude".to_string()),
                receipt_id: Some("r1".to_string()),
                prompt_summary: Some("test".to_string()),
            })
            .collect();

        let ranges = collapse_to_ranges(&lines);
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0].start_line, 1);
        assert_eq!(ranges[0].end_line, 5);
    }

    #[test]
    fn test_collapse_ranges_mixed() {
        let mut lines: Vec<LineProvenance> = (1u32..=3)
            .map(|i| LineProvenance {
                line_number: i,
                content: format!("line {}", i),
                commit_sha: "abc".to_string(),
                author: "Alice".to_string(),
                is_ai: true,
                model: Some("claude".to_string()),
                provider: Some("anthropic".to_string()),
                receipt_id: Some("r1".to_string()),
                prompt_summary: None,
            })
            .collect();
        lines.push(LineProvenance {
            line_number: 4,
            content: "line 4".to_string(),
            commit_sha: "def".to_string(),
            author: "Bob".to_string(),
            is_ai: false,
            model: None,
            provider: None,
            receipt_id: None,
            prompt_summary: None,
        });

        let ranges = collapse_to_ranges(&lines);
        assert_eq!(ranges.len(), 2);
        assert_eq!(ranges[0].end_line, 3);
        assert!(!ranges[1].is_ai);
    }
}
