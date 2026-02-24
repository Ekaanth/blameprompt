use crate::git::notes;
use crate::core::receipt::Receipt;
use chrono::Utc;
use comfy_table::Table;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct AuditEntry {
    pub commit_sha: String,
    pub commit_date: String,
    pub commit_author: String,
    pub commit_message: String,
    pub receipts: Vec<Receipt>,
    pub total_ai_lines: u32,
    pub total_cost_usd: f64,
}

pub fn collect_audit_entries(
    from: Option<&str>,
    to: Option<&str>,
    author: Option<&str>,
) -> Result<Vec<AuditEntry>, String> {
    let mut args = vec![
        "log".to_string(),
        "--format=%H|%aI|%an <%ae>|%s".to_string(),
    ];
    if let Some(f) = from {
        args.push(format!("--since={}", f));
    }
    if let Some(t) = to {
        args.push(format!("--until={}", t));
    }
    if let Some(a) = author {
        args.push(format!("--author={}", a));
    }

    let output = std::process::Command::new("git")
        .args(&args)
        .output()
        .map_err(|e| format!("git log failed: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Handle repos with no commits yet
        if stderr.contains("does not have any commits") || stderr.contains("bad default revision") {
            return Ok(Vec::new());
        }
        return Err("git log failed".to_string());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut entries = Vec::new();

    for line in stdout.lines() {
        let parts: Vec<&str> = line.splitn(4, '|').collect();
        if parts.len() < 4 {
            continue;
        }

        let sha = parts[0].to_string();
        let date = parts[1].to_string();
        let author_str = parts[2].to_string();
        let message = parts[3].to_string();

        if let Some(payload) = notes::read_receipts_for_commit(&sha) {
            if payload.receipts.is_empty() {
                continue;
            }

            let total_ai_lines: u32 = payload
                .receipts
                .iter()
                .map(|r| {
                    if r.line_range.1 >= r.line_range.0 {
                        r.line_range.1 - r.line_range.0 + 1
                    } else {
                        0
                    }
                })
                .sum();
            let total_cost_usd: f64 = payload.receipts.iter().map(|r| r.cost_usd).sum();

            entries.push(AuditEntry {
                commit_sha: sha,
                commit_date: date,
                commit_author: author_str,
                commit_message: message,
                receipts: payload.receipts,
                total_ai_lines,
                total_cost_usd,
            });
        }
    }

    Ok(entries)
}

/// Collect receipts from the local staging area (uncommitted/staged).
pub fn collect_staged_entries() -> Vec<AuditEntry> {
    let staging = crate::commands::staging::read_staging();
    if staging.receipts.is_empty() {
        return Vec::new();
    }
    // Group staged receipts into a single AuditEntry with commit_sha="uncommitted"
    let total_ai_lines: u32 = staging.receipts.iter().map(|r| {
        if r.line_range.1 >= r.line_range.0 { r.line_range.1 - r.line_range.0 + 1 } else { 0 }
    }).sum();
    let total_cost_usd: f64 = staging.receipts.iter().map(|r| r.cost_usd).sum();

    vec![AuditEntry {
        commit_sha: "uncommitted".to_string(),
        commit_date: Utc::now().format("%Y-%m-%dT%H:%M:%S%z").to_string(),
        commit_author: "staging".to_string(),
        commit_message: "(uncommitted changes)".to_string(),
        receipts: staging.receipts,
        total_ai_lines,
        total_cost_usd,
    }]
}

/// Collect both committed and (optionally) staged/uncommitted entries.
pub fn collect_all_entries(
    from: Option<&str>,
    to: Option<&str>,
    author: Option<&str>,
    include_uncommitted: bool,
) -> Result<Vec<AuditEntry>, String> {
    let mut entries = collect_audit_entries(from, to, author)?;
    if include_uncommitted {
        entries.extend(collect_staged_entries());
    }
    Ok(entries)
}

fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    }
}

/// Convert absolute file paths to repo-relative paths for cleaner display.
pub fn relative_path(path: &str) -> String {
    if let Ok(cwd) = std::env::current_dir() {
        let cwd_str = cwd.to_string_lossy();
        if path.starts_with(cwd_str.as_ref()) {
            let rel = &path[cwd_str.len()..];
            return rel.strip_prefix('/').unwrap_or(rel).to_string();
        }
    }
    // Fall back: strip everything before the last common path segment
    if let Some(idx) = path.rfind("/src/") {
        return path[idx + 1..].to_string();
    }
    path.to_string()
}

fn write_receipt_md(md: &mut String, r: &Receipt) {
    let rel_file = relative_path(&r.file_path);
    md.push_str(&format!("#### Receipt: {}\n", r.id));
    md.push_str("| Field | Value |\n");
    md.push_str("|-------|-------|\n");
    md.push_str(&format!("| Provider | {} |\n", r.provider));
    md.push_str(&format!("| Model | {} |\n", r.model));
    md.push_str(&format!("| Session | {} |\n", r.session_id));
    md.push_str(&format!("| Messages | {} |\n", r.message_count));
    md.push_str(&format!("| Cost | ${:.4} |\n", r.cost_usd));
    md.push_str(&format!("| File | {} |\n", rel_file));
    md.push_str(&format!("| Lines | {}-{} |\n\n", r.line_range.0, r.line_range.1));
    md.push_str("**Prompt Summary:**\n");
    md.push_str(&format!("> {}\n\n", r.prompt_summary));
    md.push_str(&format!("**Prompt Hash:** `{}`\n\n", r.prompt_hash));

    // Chain of Thought: conversation turns
    if let Some(ref turns) = r.conversation {
        md.push_str("**Chain of Thought:**\n\n");
        for t in turns {
            let role_label = match t.role.as_str() {
                "user" => "**User**",
                "assistant" => "**AI**",
                "tool" => "**Tool**",
                _ => "**???**",
            };
            let content_preview: String = t.content.chars().take(500).collect();
            md.push_str(&format!("- `Turn {}` {}: {}\n", t.turn, role_label, content_preview));
            if let Some(ref files) = t.files_touched {
                md.push_str(&format!("  - Files: {}\n", files.iter().map(|f| relative_path(f)).collect::<Vec<_>>().join(", ")));
            }
        }
        md.push('\n');
    }

    md.push_str("---\n\n");
}

fn generate_markdown(entries: &[AuditEntry]) -> String {
    let now = Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string();

    let committed: Vec<&AuditEntry> = entries.iter().filter(|e| e.commit_sha != "uncommitted").collect();
    let uncommitted: Vec<&AuditEntry> = entries.iter().filter(|e| e.commit_sha == "uncommitted").collect();

    let total_receipts: usize = entries.iter().map(|e| e.receipts.len()).sum();
    let total_lines: u32 = entries.iter().map(|e| e.total_ai_lines).sum();
    let total_cost: f64 = entries.iter().map(|e| e.total_cost_usd).sum();
    let uncommitted_receipts: usize = uncommitted.iter().map(|e| e.receipts.len()).sum();

    let mut md = String::new();
    md.push_str("# BlamePrompt Audit Trail\n");
    md.push_str(&format!("> Generated: {}\n\n", now));

    md.push_str("## Summary\n");
    md.push_str("| Metric | Value |\n");
    md.push_str("|--------|-------|\n");
    md.push_str(&format!("| Commits with AI code | {} |\n", committed.len()));
    md.push_str(&format!("| Total receipts | {} |\n", total_receipts));
    md.push_str(&format!("| Total AI lines | {} |\n", total_lines));
    md.push_str(&format!("| Estimated cost | ${:.2} |\n", total_cost));
    md.push_str(&format!("| Uncommitted receipts | {} |\n\n", uncommitted_receipts));

    if !committed.is_empty() {
        md.push_str("## Committed Changes\n");
        for entry in &committed {
            let sha_display = if entry.commit_sha.len() >= 8 {
                &entry.commit_sha[..8]
            } else {
                &entry.commit_sha
            };
            md.push_str(&format!("### Commit: {} - {}\n", sha_display, entry.commit_message));
            md.push_str(&format!("- **Date**: {}\n", entry.commit_date));
            md.push_str(&format!("- **Author**: {}\n\n", entry.commit_author));

            for r in &entry.receipts {
                write_receipt_md(&mut md, r);
            }
        }
    }

    if !uncommitted.is_empty() {
        md.push_str("## Uncommitted Changes (Staging)\n");
        for entry in &uncommitted {
            for r in &entry.receipts {
                write_receipt_md(&mut md, r);
            }
        }
    }

    md
}

pub fn run(
    from: Option<&str>,
    to: Option<&str>,
    author: Option<&str>,
    format: &str,
    include_uncommitted: bool,
) {
    let mut entries = match collect_all_entries(from, to, author, include_uncommitted) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("Error: {}", e);
            return;
        }
    };

    // Auto-include staging data if no committed entries found
    if entries.is_empty() && !include_uncommitted {
        let staged = collect_staged_entries();
        if !staged.is_empty() {
            entries = staged;
        }
    }

    if entries.is_empty() {
        println!("No AI-generated code found in this repository.");
        return;
    }

    match format {
        "json" => {
            println!("{}", serde_json::to_string_pretty(&entries).unwrap_or_default());
        }
        "csv" => {
            println!("commit_sha,date,author,message,provider,model,session_id,message_count,cost_usd,file,line_range,prompt_summary,prompt_hash");
            for entry in &entries {
                for r in &entry.receipts {
                    let sha_display = if entry.commit_sha.len() >= 8 {
                        &entry.commit_sha[..8]
                    } else {
                        &entry.commit_sha
                    };
                    println!(
                        "{},{},{},{},{},{},{},{},{:.4},{},{}-{},{},{}",
                        sha_display,
                        entry.commit_date,
                        entry.commit_author,
                        entry.commit_message.replace(',', ";"),
                        r.provider,
                        r.model,
                        r.session_id,
                        r.message_count,
                        r.cost_usd,
                        relative_path(&r.file_path),
                        r.line_range.0,
                        r.line_range.1,
                        r.prompt_summary.replace(',', ";"),
                        r.prompt_hash,
                    );
                }
            }
        }
        "md" => {
            let markdown = generate_markdown(&entries);
            let output_path = "blameprompt-audit.md";
            match std::fs::write(output_path, &markdown) {
                Ok(_) => println!("Audit markdown written to {}", output_path),
                Err(e) => eprintln!("Error writing markdown file: {}", e),
            }
        }
        _ => {
            // Table format
            let total_receipts: usize = entries.iter().map(|e| e.receipts.len()).sum();
            let total_cost: f64 = entries.iter().map(|e| e.total_cost_usd).sum();
            let total_lines: u32 = entries.iter().map(|e| e.total_ai_lines).sum();

            println!("AI Audit Trail");
            println!("==============");
            println!("Commits with AI code: {}", entries.len());
            println!("Total receipts: {}", total_receipts);
            println!("Total AI lines: {}", total_lines);
            println!("Total estimated cost: ${:.2}", total_cost);
            println!();

            let mut table = Table::new();
            table.set_header(vec![
                "Commit", "Date", "Author", "Provider", "Model", "Messages", "Cost", "File", "Lines", "Prompt Summary",
            ]);

            for entry in &entries {
                for r in &entry.receipts {
                    let sha_display = if entry.commit_sha.len() >= 8 {
                        &entry.commit_sha[..8]
                    } else {
                        &entry.commit_sha
                    };
                    let date_display = if entry.commit_date.len() >= 10 {
                        &entry.commit_date[..10]
                    } else {
                        &entry.commit_date
                    };
                    let rel_file = relative_path(&r.file_path);
                    table.add_row(vec![
                        sha_display,
                        date_display,
                        &entry.commit_author,
                        &r.provider,
                        &r.model,
                        &r.message_count.to_string(),
                        &format!("${:.4}", r.cost_usd),
                        &rel_file,
                        &format!("{}-{}", r.line_range.0, r.line_range.1),
                        &truncate_str(&r.prompt_summary, 40),
                    ]);
                }
            }

            println!("{table}");
        }
    }
}
