use crate::core::receipt::{CodeOrigin, CodeOriginStats};
use crate::git::notes;
use comfy_table::{Cell, Color, Table};
use serde::Serialize;
use std::collections::HashMap;

#[derive(Serialize)]
pub struct BlameLineOutput {
    pub line: u32,
    pub code: String,
    pub source: String,
    pub provider: String,
    pub model: String,
    pub cost_usd: f64,
    pub prompt_summary: String,
    pub receipt_id: String,
    pub commit_sha: String,
}

#[derive(Serialize)]
pub struct BlameOutput {
    pub file: String,
    pub total_lines: u32,
    pub ai_lines: u32,
    pub ai_pct: f64,
    pub human_pct: f64,
    pub lines: Vec<BlameLineOutput>,
}

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
                'line: for r in receipts {
                    for fc in r.all_file_changes() {
                        if (fc.path == file
                            || file.ends_with(&fc.path)
                            || fc.path.ends_with(file))
                            && line_num >= fc.line_range.0
                            && line_num <= fc.line_range.1
                        {
                            ai_lines += 1;
                            break 'line;
                        }
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

struct LineAttribution {
    source: String,
    provider: String,
    model: String,
    cost_usd: f64,
    prompt_summary: String,
    receipt_id: String,
}

#[allow(clippy::type_complexity)]
fn compute_blame(file: &str) -> Option<(Vec<String>, HashMap<u32, String>, Vec<LineAttribution>)> {
    // Verify file exists and is tracked
    let output = std::process::Command::new("git")
        .args(["ls-files", file])
        .output();

    match &output {
        Ok(o) if o.stdout.is_empty() => {
            eprintln!("Error: '{}' is not tracked by git", file);
            return None;
        }
        Err(_) => {
            eprintln!("Error: Not in a git repository");
            return None;
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
            return None;
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
            return None;
        }
    };

    let lines: Vec<String> = file_content.lines().map(|s| s.to_string()).collect();
    let mut attributions = Vec::new();

    for (idx, _) in lines.iter().enumerate() {
        let line_num = (idx + 1) as u32;
        let commit_sha = line_commits.get(&line_num);

        let mut source = "human".to_string();
        let mut provider = String::new();
        let mut model = String::new();
        let mut cost_usd = 0.0;
        let mut prompt_summary = String::new();
        let mut receipt_id = String::new();

        if let Some(sha) = commit_sha {
            // Check file_mappings first for finer granularity
            if let Some(mappings) = sha_mappings.get(sha) {
                for fm in mappings {
                    if fm.path == file || file.ends_with(&fm.path) || fm.path.ends_with(file) {
                        for h in &fm.hunks {
                            if line_num >= h.start_line && line_num <= h.end_line {
                                match h.origin {
                                    CodeOrigin::AiGenerated => {
                                        source = "ai".to_string();
                                        if let Some(ref m) = h.model {
                                            model = m.clone();
                                        }
                                    }
                                    CodeOrigin::HumanEdited => source = "edited".to_string(),
                                    CodeOrigin::PureHuman => source = "human".to_string(),
                                }
                                break;
                            }
                        }
                    }
                }
            }

            // Fall back to receipt-level matching
            if source == "human" {
                if let Some(receipts) = sha_receipts.get(sha) {
                    'receipt: for r in receipts {
                        for fc in r.all_file_changes() {
                            if (fc.path == file
                                || file.ends_with(&fc.path)
                                || fc.path.ends_with(file))
                                && line_num >= fc.line_range.0
                                && line_num <= fc.line_range.1
                            {
                                source = "ai".to_string();
                                provider = r.provider.clone();
                                model = r.model.clone();
                                cost_usd = r.cost_usd;
                                prompt_summary = r.prompt_summary.clone();
                                receipt_id = r.id.clone();
                                break 'receipt;
                            }
                        }
                    }
                }
            }
        }

        attributions.push(LineAttribution {
            source,
            provider,
            model,
            cost_usd,
            prompt_summary,
            receipt_id,
        });
    }

    Some((lines, line_commits, attributions))
}

pub fn run(file: &str, format: &str) {
    let (lines, line_commits, attributions) = match compute_blame(file) {
        Some(data) => data,
        None => return,
    };

    let total_lines = lines.len() as u32;
    let ai_line_count = attributions.iter().filter(|a| a.source == "ai").count() as u32;

    if format == "json" {
        let output = BlameOutput {
            file: file.to_string(),
            total_lines,
            ai_lines: ai_line_count,
            ai_pct: if total_lines > 0 {
                (ai_line_count as f64 / total_lines as f64) * 100.0
            } else {
                0.0
            },
            human_pct: if total_lines > 0 {
                100.0 - (ai_line_count as f64 / total_lines as f64) * 100.0
            } else {
                100.0
            },
            lines: lines
                .iter()
                .enumerate()
                .map(|(idx, code)| {
                    let line_num = (idx + 1) as u32;
                    let attr = &attributions[idx];
                    BlameLineOutput {
                        line: line_num,
                        code: code.clone(),
                        source: attr.source.clone(),
                        provider: attr.provider.clone(),
                        model: attr.model.clone(),
                        cost_usd: attr.cost_usd,
                        prompt_summary: attr.prompt_summary.clone(),
                        receipt_id: attr.receipt_id.clone(),
                        commit_sha: line_commits.get(&line_num).cloned().unwrap_or_default(),
                    }
                })
                .collect(),
        };
        println!("{}", serde_json::to_string_pretty(&output).unwrap());
        return;
    }

    // Table output (default)
    let mut table = Table::new();
    table.set_header(vec![
        "Line", "Code", "Source", "Provider", "Model", "Cost", "Prompt",
    ]);

    for (idx, line_content) in lines.iter().enumerate() {
        let line_num = (idx + 1) as u32;
        let code: String = line_content.chars().take(50).collect();
        let attr = &attributions[idx];

        let source_display = match attr.source.as_str() {
            "ai" => "AI",
            "edited" => "Edited",
            _ => "Human",
        };

        let source_color = match attr.source.as_str() {
            "ai" => Color::Yellow,
            "edited" => Color::Cyan,
            _ => Color::Green,
        };

        let cost_display = if attr.cost_usd > 0.0 {
            format!("${:.4}", attr.cost_usd)
        } else {
            String::new()
        };

        let prompt_display: String = attr.prompt_summary.chars().take(30).collect();

        table.add_row(vec![
            Cell::new(line_num),
            Cell::new(&code),
            Cell::new(source_display).fg(source_color),
            Cell::new(&attr.provider),
            Cell::new(&attr.model),
            Cell::new(&cost_display),
            Cell::new(&prompt_display),
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
