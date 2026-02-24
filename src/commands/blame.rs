use crate::core::receipt::{CodeOrigin, CodeOriginStats};
use crate::git::notes;
use comfy_table::{Cell, Color, Table};
use std::collections::HashMap;

pub fn calculate_code_origin(file: &str) -> Option<CodeOriginStats> {
    let file_content = std::fs::read_to_string(file).ok()?;
    let total_lines = file_content.lines().count() as f64;
    if total_lines == 0.0 {
        return None;
    }

    // Run git blame to get commit SHAs per line
    let blame_output = std::process::Command::new("git")
        .args(["blame", "--porcelain", file])
        .output()
        .ok()?;

    if !blame_output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&blame_output.stdout);
    let mut line_commits: HashMap<u32, String> = HashMap::new();
    for line in stdout.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 3
            && parts[0].len() == 40
            && parts[0].chars().all(|c| c.is_ascii_hexdigit())
        {
            let sha = parts[0].to_string();
            if let Ok(final_line) = parts[2].parse::<u32>() {
                line_commits.insert(final_line, sha);
            }
        }
    }

    // Collect unique SHAs and fetch receipts
    let unique_shas: Vec<String> = {
        let mut shas: Vec<String> = line_commits.values().cloned().collect();
        shas.sort();
        shas.dedup();
        shas
    };

    let mut sha_receipts: HashMap<String, Vec<crate::core::receipt::Receipt>> = HashMap::new();
    for sha in &unique_shas {
        if let Some(payload) = notes::read_receipts_for_commit(sha) {
            // Check file_mappings first for finer granularity
            if let Some(ref mappings) = payload.file_mappings {
                for fm in mappings {
                    if fm.path == file || file.ends_with(&fm.path) || fm.path.ends_with(file) {
                        // Use hunks from file_mappings for precise attribution
                        // (stored in sha_receipts as regular receipts for now)
                    }
                }
            }
            sha_receipts.insert(sha.clone(), payload.receipts);
        }
    }

    let mut ai_lines = 0u32;
    let line_count = total_lines as u32;

    for line_num in 1..=line_count {
        if let Some(sha) = line_commits.get(&line_num) {
            if let Some(receipts) = sha_receipts.get(sha) {
                for r in receipts {
                    if (r.file_path == file
                        || file.ends_with(&r.file_path)
                        || r.file_path.ends_with(file))
                        && line_num >= r.line_range.0
                        && line_num <= r.line_range.1
                    {
                        ai_lines += 1;
                        break;
                    }
                }
            }
        }
    }

    let ai_pct = (ai_lines as f64 / total_lines) * 100.0;
    let human_pct = 100.0 - ai_pct;

    Some(CodeOriginStats {
        ai_generated_pct: ai_pct,
        human_edited_pct: 0.0, // Future: compare blob hashes to detect post-AI edits
        pure_human_pct: human_pct,
    })
}

pub fn run(file: &str) {
    // Verify file exists and is tracked
    let output = std::process::Command::new("git")
        .args(["ls-files", file])
        .output();

    match &output {
        Ok(o) if o.stdout.is_empty() => {
            eprintln!("Error: '{}' is not tracked by git", file);
            return;
        }
        Err(_) => {
            eprintln!("Error: Not in a git repository");
            return;
        }
        _ => {}
    }

    // Run git blame --porcelain
    let blame_output = match std::process::Command::new("git")
        .args(["blame", "--porcelain", file])
        .output()
    {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
        _ => {
            eprintln!("Error: git blame failed for '{}'", file);
            return;
        }
    };

    // Parse porcelain output: build line_number -> commit_sha map
    let mut line_commits: HashMap<u32, String> = HashMap::new();
    for line in blame_output.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 3
            && parts[0].len() == 40
            && parts[0].chars().all(|c| c.is_ascii_hexdigit())
        {
            let sha = parts[0].to_string();
            if let Ok(final_line) = parts[2].parse::<u32>() {
                line_commits.insert(final_line, sha);
            }
        }
    }

    // Collect unique SHAs and fetch receipts
    let unique_shas: Vec<String> = {
        let mut shas: Vec<String> = line_commits.values().cloned().collect();
        shas.sort();
        shas.dedup();
        shas
    };

    let mut sha_receipts: HashMap<String, Vec<crate::core::receipt::Receipt>> = HashMap::new();
    let mut sha_mappings: HashMap<String, Vec<crate::core::receipt::FileMapping>> = HashMap::new();

    for sha in &unique_shas {
        if let Some(payload) = notes::read_receipts_for_commit(sha) {
            sha_receipts.insert(sha.clone(), payload.receipts);
            if let Some(mappings) = payload.file_mappings {
                sha_mappings.insert(sha.clone(), mappings);
            }
        }
    }

    // Read file lines
    let file_content = match std::fs::read_to_string(file) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error reading file: {}", e);
            return;
        }
    };

    let lines: Vec<&str> = file_content.lines().collect();

    // Build table
    let mut table = Table::new();
    table.set_header(vec![
        "Line", "Code", "Source", "Provider", "Model", "Cost", "Prompt",
    ]);

    let mut ai_line_count = 0u32;
    let total_lines = lines.len() as u32;

    for (idx, line_content) in lines.iter().enumerate() {
        let line_num = (idx + 1) as u32;
        let code: String = line_content.chars().take(50).collect();

        let commit_sha = line_commits.get(&line_num);

        // Check if this line has an AI receipt
        let mut source = "Human";
        let mut provider = "".to_string();
        let mut model = "".to_string();
        let mut cost = "".to_string();
        let mut prompt = "".to_string();

        if let Some(sha) = commit_sha {
            // Check file_mappings first for finer granularity
            if let Some(mappings) = sha_mappings.get(sha) {
                for fm in mappings {
                    if fm.path == file || file.ends_with(&fm.path) || fm.path.ends_with(file) {
                        for h in &fm.hunks {
                            if line_num >= h.start_line && line_num <= h.end_line {
                                match h.origin {
                                    CodeOrigin::AiGenerated => {
                                        source = "AI";
                                        if let Some(ref m) = h.model {
                                            model = m.clone();
                                        }
                                    }
                                    CodeOrigin::HumanEdited => source = "Edited",
                                    CodeOrigin::PureHuman => source = "Human",
                                }
                                break;
                            }
                        }
                    }
                }
            }

            // Fall back to receipt-level matching
            if source == "Human" {
                if let Some(receipts) = sha_receipts.get(sha) {
                    for r in receipts {
                        if (r.file_path == file
                            || file.ends_with(&r.file_path)
                            || r.file_path.ends_with(file))
                            && line_num >= r.line_range.0
                            && line_num <= r.line_range.1
                        {
                            source = "AI";
                            provider = r.provider.clone();
                            model = r.model.clone();
                            cost = format!("${:.4}", r.cost_usd);
                            prompt = r.prompt_summary.chars().take(30).collect();
                            break;
                        }
                    }
                }
            }
        }

        if source == "AI" {
            ai_line_count += 1;
        }

        let source_color = match source {
            "AI" => Color::Yellow,
            "Edited" => Color::Cyan,
            _ => Color::Green,
        };

        table.add_row(vec![
            Cell::new(line_num),
            Cell::new(&code),
            Cell::new(source).fg(source_color),
            Cell::new(&provider),
            Cell::new(&model),
            Cell::new(&cost),
            Cell::new(&prompt),
        ]);
    }

    println!("{table}");

    // Show code origin summary
    if total_lines > 0 {
        let ai_pct = (ai_line_count as f64 / total_lines as f64) * 100.0;
        let human_pct = 100.0 - ai_pct;
        println!();
        println!(
            "Code Origin: {:.1}% AI-generated, {:.1}% human",
            ai_pct, human_pct
        );
    }
}
