use crate::commands::staging;
use crate::core::{receipt::Receipt, util};
use crate::git::notes;

// ANSI color codes
const RESET: &str = "\x1b[0m";
const RED: &str = "\x1b[31m";
const GREEN: &str = "\x1b[32m";
const CYAN: &str = "\x1b[36m";
const BOLD_YELLOW: &str = "\x1b[1;33m";
const DIM: &str = "\x1b[2m";
const BOLD_CYAN: &str = "\x1b[1;36m";

/// Show an annotated diff with AI/human attribution markers.
///
/// If `commit_ref` is provided, shows that commit's diff with notes-based attribution.
/// Otherwise shows staged/unstaged changes with staging.json attribution.
pub fn run(commit_ref: Option<&str>) {
    if let Some(commit) = commit_ref {
        show_commit_diff(commit);
    } else {
        show_working_diff();
    }
}

fn show_commit_diff(commit: &str) {
    let sha = match resolve_sha(commit) {
        Some(s) => s,
        None => {
            eprintln!("Error: Cannot resolve commit '{}'", commit);
            return;
        }
    };

    let diff_output = match run_git(&["show", "--unified=3", &sha]) {
        Some(s) => s,
        None => {
            eprintln!("Error: git show failed for '{}'", commit);
            return;
        }
    };

    if diff_output.trim().is_empty() {
        println!("No changes in commit {}.", commit);
        return;
    }

    let receipts = notes::read_receipts_for_commit(&sha)
        .map(|p| p.receipts)
        .unwrap_or_default();

    if receipts.is_empty() {
        // Print undecorated diff if no notes exist
        println!(
            "{}[BlamePrompt] No AI receipts found for {}{}",
            DIM, commit, RESET
        );
        print_plain_diff(&diff_output);
        return;
    }

    println!(
        "{}[BlamePrompt] {} receipt(s) attached to {}{}",
        BOLD_CYAN,
        receipts.len(),
        &util::short_sha(&sha),
        RESET
    );
    print_annotated_diff(&diff_output, &receipts);
}

fn show_working_diff() {
    // Try unstaged first, then staged
    let diff_output = run_git(&["diff", "--unified=3", "HEAD"])
        .filter(|s| !s.trim().is_empty())
        .or_else(|| run_git(&["diff", "--cached", "--unified=3"]).filter(|s| !s.trim().is_empty()))
        .unwrap_or_default();

    if diff_output.trim().is_empty() {
        println!("No changes to display. Use 'blameprompt diff <commit>' to view a commit.");
        return;
    }

    let staging_data = staging::read_staging();
    let receipts = staging_data.receipts;

    if receipts.is_empty() {
        println!(
            "{}[BlamePrompt] No staged AI receipts — all changes appear human-written.{}",
            DIM, RESET
        );
    } else {
        println!(
            "{}[BlamePrompt] {} staged AI receipt(s){}",
            BOLD_CYAN,
            receipts.len(),
            RESET
        );
    }

    print_annotated_diff(&diff_output, &receipts);
}

/// Print a unified diff with AI-origin annotation on each `@@ ... @@` hunk header.
fn print_annotated_diff(diff: &str, receipts: &[Receipt]) {
    let mut current_file: Option<String> = None;

    for line in diff.lines() {
        if line.starts_with("diff --git ") {
            // Extract "b/<path>" from "diff --git a/path b/path"
            if let Some(b_part) = line.split(" b/").last() {
                current_file = Some(b_part.to_string());
            }
            println!("{}{}{}", BOLD_CYAN, line, RESET);
        } else if line.starts_with("--- ") || line.starts_with("+++ ") {
            println!("{}{}{}", BOLD_CYAN, line, RESET);
        } else if line.starts_with("index ")
            || line.starts_with("new file")
            || line.starts_with("deleted file")
        {
            println!("{}{}{}", DIM, line, RESET);
        } else if line.starts_with("@@ ") {
            let annotation = if let Some(ref file) = current_file {
                let (hunk_start, hunk_end) = util::parse_hunk_range(line);
                get_hunk_annotation(file, hunk_start, hunk_end, receipts)
            } else {
                String::new()
            };

            if annotation.is_empty() {
                // Human-written hunk
                println!(
                    "{}{}{}  {}[\u{1f465} Human]{}",
                    CYAN, line, RESET, DIM, RESET
                );
            } else {
                println!("{}{}{}  {}", CYAN, line, RESET, annotation);
            }
        } else if line.starts_with('+') && !line.starts_with("+++") {
            println!("{}{}{}", GREEN, line, RESET);
        } else if line.starts_with('-') && !line.starts_with("---") {
            println!("{}{}{}", RED, line, RESET);
        } else {
            println!("{}", line);
        }
    }
}

fn print_plain_diff(diff: &str) {
    for line in diff.lines() {
        if line.starts_with('+') && !line.starts_with("+++") {
            println!("{}{}{}", GREEN, line, RESET);
        } else if line.starts_with('-') && !line.starts_with("---") {
            println!("{}{}{}", RED, line, RESET);
        } else {
            println!("{}", line);
        }
    }
}

/// Find the best AI attribution for a hunk in the given file.
fn get_hunk_annotation(file: &str, hunk_start: u32, hunk_end: u32, receipts: &[Receipt]) -> String {
    for receipt in receipts {
        for fc in receipt.all_file_changes() {
            if util::paths_match(file, &fc.path)
                && hunk_end >= fc.line_range.0
                && hunk_start <= fc.line_range.1.max(fc.line_range.0)
            {
                let summary: String = receipt.prompt_summary.chars().take(50).collect();
                let model_short = shorten_model(&receipt.model);
                return format!(
                    "{}[🤖 {} | \"{}\"]{}",
                    BOLD_YELLOW, model_short, summary, RESET
                );
            }
        }
    }
    String::new()
}

/// Shorten a model name for display (e.g. "claude-sonnet-4-6" → "sonnet-4.6").
fn shorten_model(model: &str) -> String {
    let s = model
        .trim_start_matches("claude-")
        .replace("-20250", " '25/")
        .replace("-2026", " '26/");
    // Truncate long model names
    let truncated: String = s.chars().take(24).collect();
    truncated
}

fn run_git(args: &[&str]) -> Option<String> {
    std::process::Command::new("git")
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
}

fn resolve_sha(reference: &str) -> Option<String> {
    run_git(&["rev-parse", reference])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shorten_model() {
        assert_eq!(shorten_model("claude-sonnet-4-6"), "sonnet-4-6");
    }
}
