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

pub fn run(commit: &str) {
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
            println!("No BlamePrompt receipts found for commit {}", &sha[..8.min(sha.len())]);
            return;
        }
    };

    if payload.receipts.is_empty() {
        println!("No AI receipts attached to commit {}", &sha[..8.min(sha.len())]);
        return;
    }

    let sha_short = &sha[..8.min(sha.len())];
    println!("BlamePrompt receipts for commit {}", sha_short);
    println!("Schema version: {}", payload.blameprompt_version);
    println!("Total receipts: {}", payload.receipts.len());
    println!();

    let mut table = Table::new();
    table.set_header(vec![
        "ID", "Provider", "Model", "Session", "Messages", "Cost",
        "File", "Lines", "Timestamp", "Prompt Summary",
    ]);

    for r in &payload.receipts {
        let id_short = if r.id.len() >= 8 { &r.id[..8] } else { &r.id };
        let session_short = if r.session_id.len() >= 8 { &r.session_id[..8] } else { &r.session_id };
        let ts = r.timestamp.format("%Y-%m-%d %H:%M").to_string();
        let prompt: String = r.prompt_summary.chars().take(40).collect();

        table.add_row(vec![
            id_short,
            &r.provider,
            &r.model,
            session_short,
            &r.message_count.to_string(),
            &format!("${:.4}", r.cost_usd),
            &r.file_path,
            &format!("{}-{}", r.line_range.0, r.line_range.1),
            &ts,
            &prompt,
        ]);
    }

    println!("{table}");

    // Show file mappings if present
    if let Some(ref mappings) = payload.file_mappings {
        println!("\nFile Mappings:");
        for fm in mappings {
            println!("  {} (blob: {})", fm.path, &fm.blob_hash[..8.min(fm.blob_hash.len())]);
            for h in &fm.hunks {
                println!("    Lines {}-{}: {:?}{}", h.start_line, h.end_line, h.origin,
                    h.model.as_ref().map(|m| format!(" ({})", m)).unwrap_or_default());
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
    let chained: Vec<_> = payload.receipts.iter()
        .filter(|r| r.parent_receipt_id.is_some())
        .collect();
    if !chained.is_empty() {
        println!("\nReceipt Chains:");
        for r in &chained {
            let parent = r.parent_receipt_id.as_ref().unwrap();
            let parent_short = if parent.len() >= 8 { &parent[..8] } else { parent };
            let id_short = if r.id.len() >= 8 { &r.id[..8] } else { &r.id };
            println!("  {} -> {} (refinement)", parent_short, id_short);
        }
    }

    // Show conversation chain of thought
    for r in &payload.receipts {
        if let Some(ref turns) = r.conversation {
            let id_short = if r.id.len() >= 8 { &r.id[..8] } else { &r.id };
            println!("\nChain of Thought for receipt {} ({}):", id_short, r.file_path);
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
                    println!("         Files: {}", files.join(", "));
                }
            }
        }
    }
}
