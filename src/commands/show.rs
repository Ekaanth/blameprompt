use crate::commands::audit;
use crate::git::notes;
use comfy_table::Table;

fn resolve_sha(input: &str) -> Result<String, String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", input])
        .output()
        .map_err(|e| format!("git rev-parse failed: {}", e))?;

    if !output.status.success() {
        return Err(format!("Cannot resolve commit: {}", input));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub fn run(commit: &str, format: &str) {
    let sha = match resolve_sha(commit) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Error: {}", e);
            return;
        }
    };

    let payload = match notes::read_receipts_for_commit(&sha) {
        Some(p) => p,
        None => {
            if format == "json" {
                println!(
                    "{{\"error\":\"no_receipts\",\"commit\":\"{}\"}}",
                    &sha[..8.min(sha.len())]
                );
            } else {
                println!(
                    "No BlamePrompt receipts found for commit {}",
                    &sha[..8.min(sha.len())]
                );
            }
            return;
        }
    };

    if payload.receipts.is_empty() {
        if format == "json" {
            println!(
                "{{\"error\":\"empty_receipts\",\"commit\":\"{}\"}}",
                &sha[..8.min(sha.len())]
            );
        } else {
            println!(
                "No AI receipts attached to commit {}",
                &sha[..8.min(sha.len())]
            );
        }
        return;
    }

    // JSON output — NotePayload is already Serialize
    if format == "json" {
        println!("{}", serde_json::to_string_pretty(&payload).unwrap());
        return;
    }

    // Table output (default)
    let sha_short = &sha[..8.min(sha.len())];
    println!("BlamePrompt receipts for commit {}", sha_short);
    println!("Schema version: {}", payload.blameprompt_version);
    println!("Total receipts: {}", payload.receipts.len());
    println!();

    let mut table = Table::new();
    table.set_header(vec![
        "ID",
        "Provider",
        "Model",
        "Session",
        "Messages",
        "Cost",
        "File",
        "Lines",
        "Timestamp",
        "Prompt Summary",
    ]);

    for r in &payload.receipts {
        let id_short = if r.id.len() >= 8 { &r.id[..8] } else { &r.id };
        let session_short = if r.session_id.len() >= 8 {
            &r.session_id[..8]
        } else {
            &r.session_id
        };
        let ts = r.timestamp.format("%Y-%m-%d %H:%M").to_string();
        let prompt: String = r.prompt_summary.chars().take(40).collect();

        let file_changes = r.all_file_changes();
        let files_display = if file_changes.len() == 1 {
            audit::relative_path(&file_changes[0].path)
        } else {
            format!("{} files", file_changes.len())
        };
        table.add_row(vec![
            id_short,
            &r.provider,
            &r.model,
            session_short,
            &r.message_count.to_string(),
            &format!("${:.4}", r.cost_usd),
            &files_display,
            &r.total_lines_changed().to_string(),
            &ts,
            &prompt,
        ]);
    }

    println!("{table}");

    // Show file mappings if present
    if let Some(ref mappings) = payload.file_mappings {
        println!("\nFile Mappings:");
        for fm in mappings {
            println!(
                "  {} (blob: {})",
                audit::relative_path(&fm.path),
                &fm.blob_hash[..8.min(fm.blob_hash.len())]
            );
            for h in &fm.hunks {
                println!(
                    "    Lines {}-{}: {:?}{}",
                    h.start_line,
                    h.end_line,
                    h.origin,
                    h.model
                        .as_ref()
                        .map(|m| format!(" ({})", m))
                        .unwrap_or_default()
                );
            }
        }
    }

    // Show code origin stats if present
    if let Some(ref origin) = payload.code_origin {
        println!("\nCode Origin:");
        println!("  AI Generated: {:.1}%", origin.ai_generated_pct);
        println!("  Human Edited: {:.1}%", origin.human_edited_pct);
        println!("  Pure Human:   {:.1}%", origin.pure_human_pct);
    }

    // Show parent receipt chains
    let chained: Vec<_> = payload
        .receipts
        .iter()
        .filter(|r| r.parent_receipt_id.is_some())
        .collect();
    if !chained.is_empty() {
        println!("\nReceipt Chains:");
        for r in &chained {
            let parent = r.parent_receipt_id.as_ref().unwrap();
            let parent_short = if parent.len() >= 8 {
                &parent[..8]
            } else {
                parent
            };
            let id_short = if r.id.len() >= 8 { &r.id[..8] } else { &r.id };
            println!("  {} -> {} (refinement)", parent_short, id_short);
        }
    }

    // Show conversation chain of thought
    for r in &payload.receipts {
        if let Some(ref turns) = r.conversation {
            let id_short = if r.id.len() >= 8 { &r.id[..8] } else { &r.id };
            let files_display: Vec<String> = r.all_file_paths().iter().map(|f| audit::relative_path(f)).collect();
            println!(
                "\nChain of Thought for receipt {} ({}):",
                id_short,
                files_display.join(", ")
            );
            println!("{}", "-".repeat(60));
            for t in turns {
                let prefix = match t.role.as_str() {
                    "user" => "  [USER]",
                    "assistant" => "  [AI]  ",
                    "tool" => "  [TOOL]",
                    _ => "  [???] ",
                };
                let content_preview: String = t.content.chars().take(120).collect();
                println!("{} Turn {}: {}", prefix, t.turn, content_preview);
                if let Some(ref files) = t.files_touched {
                    let rel_files: Vec<String> =
                        files.iter().map(|f| audit::relative_path(f)).collect();
                    println!("         Files: {}", rel_files.join(", "));
                }
            }
        }
    }
}
