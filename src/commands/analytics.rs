use crate::commands::audit;
use serde::Serialize;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Serialize)]
pub struct AnalyticsReport {
    pub total_commits_scanned: u32,
    pub commits_with_ai: u32,
    pub ai_commit_percentage: f64,
    pub total_receipts: u32,
    pub total_sessions: u32,
    pub total_estimated_cost_usd: f64,
    pub total_ai_lines: u32,
    pub by_provider: HashMap<String, ProviderStats>,
    pub by_model: HashMap<String, ModelStats>,
    pub by_user: HashMap<String, UserStats>,
}

#[derive(Debug, Serialize, Default)]
pub struct ProviderStats {
    pub sessions: u32,
    pub files_modified: u32,
    pub total_cost: f64,
}

#[derive(Debug, Serialize, Default)]
pub struct ModelStats {
    pub sessions: u32,
    pub files_modified: u32,
    pub total_cost: f64,
}

#[derive(Debug, Serialize, Default)]
pub struct UserStats {
    pub sessions: u32,
    pub lines_generated: u32,
    pub total_cost: f64,
}

pub fn generate_report(
    from: Option<&str>,
    to: Option<&str>,
) -> Result<AnalyticsReport, String> {
    // Get total commits
    let total_commits = count_total_commits()?;

    // Get audit entries (commits with AI)
    let entries = audit::collect_audit_entries(from, to, None)?;

    let commits_with_ai = entries.len() as u32;
    let ai_commit_percentage = if total_commits > 0 {
        (commits_with_ai as f64 / total_commits as f64) * 100.0
    } else {
        0.0
    };

    let mut total_receipts = 0u32;
    let mut total_cost = 0.0f64;
    let mut total_lines = 0u32;
    let mut session_ids: HashSet<String> = HashSet::new();
    let mut by_provider: HashMap<String, ProviderStats> = HashMap::new();
    let mut by_model: HashMap<String, ModelStats> = HashMap::new();
    let mut by_user: HashMap<String, UserStats> = HashMap::new();

    for entry in &entries {
        for r in &entry.receipts {
            total_receipts += 1;
            total_cost += r.cost_usd;
            let lines = if r.line_range.1 >= r.line_range.0 {
                r.line_range.1 - r.line_range.0 + 1
            } else {
                0
            };
            total_lines += lines;
            session_ids.insert(r.session_id.clone());

            // By provider
            let ps = by_provider.entry(r.provider.clone()).or_default();
            ps.sessions += 1;
            ps.files_modified += 1;
            ps.total_cost += r.cost_usd;

            // By model
            let ms = by_model.entry(r.model.clone()).or_default();
            ms.sessions += 1;
            ms.files_modified += 1;
            ms.total_cost += r.cost_usd;

            // By user
            let us = by_user.entry(r.user.clone()).or_default();
            us.sessions += 1;
            us.lines_generated += lines;
            us.total_cost += r.cost_usd;
        }
    }

    Ok(AnalyticsReport {
        total_commits_scanned: total_commits,
        commits_with_ai,
        ai_commit_percentage,
        total_receipts,
        total_sessions: session_ids.len() as u32,
        total_estimated_cost_usd: total_cost,
        total_ai_lines: total_lines,
        by_provider,
        by_model,
        by_user,
    })
}

fn count_total_commits() -> Result<u32, String> {
    let output = std::process::Command::new("git")
        .args(["rev-list", "--count", "HEAD"])
        .output()
        .map_err(|e| format!("git rev-list failed: {}", e))?;

    if !output.status.success() {
        return Ok(0);
    }

    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .map_err(|e| format!("Parse error: {}", e))
}

pub fn run(export_format: Option<&str>) {
    let report = match generate_report(None, None) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Error: {}", e);
            return;
        }
    };

    match export_format {
        Some("json") => {
            println!("{}", serde_json::to_string_pretty(&report).unwrap_or_default());
        }
        Some("csv") => {
            println!("metric,value");
            println!("total_commits_scanned,{}", report.total_commits_scanned);
            println!("commits_with_ai,{}", report.commits_with_ai);
            println!("ai_commit_percentage,{:.1}", report.ai_commit_percentage);
            println!("total_receipts,{}", report.total_receipts);
            println!("total_sessions,{}", report.total_sessions);
            println!("total_estimated_cost_usd,{:.2}", report.total_estimated_cost_usd);
            println!("total_ai_lines,{}", report.total_ai_lines);
            println!();
            println!("model,sessions,files_modified,total_cost");
            for (model, stats) in &report.by_model {
                println!("{},{},{},{:.4}", model, stats.sessions, stats.files_modified, stats.total_cost);
            }
        }
        _ => {
            println!("OVERVIEW");
            println!("========");
            println!("Total commits scanned: {}", report.total_commits_scanned);
            println!("Commits with AI: {} ({:.1}%)", report.commits_with_ai, report.ai_commit_percentage);
            println!("Total sessions: {}", report.total_sessions);
            println!("Total AI lines: {}", report.total_ai_lines);
            println!();

            println!("COST");
            println!("====");
            println!("Total estimated cost: ${:.2}", report.total_estimated_cost_usd);
            if report.total_sessions > 0 {
                println!(
                    "Avg cost per session: ${:.3}",
                    report.total_estimated_cost_usd / report.total_sessions as f64
                );
            }
            println!();

            println!("BY MODEL");
            println!("========");
            let mut table = comfy_table::Table::new();
            table.set_header(vec!["Model", "Sessions", "Files", "Est. Cost"]);
            for (model, stats) in &report.by_model {
                table.add_row(vec![
                    model.as_str(),
                    &stats.sessions.to_string(),
                    &stats.files_modified.to_string(),
                    &format!("${:.4}", stats.total_cost),
                ]);
            }
            println!("{table}");
            println!();

            println!("BY USER");
            println!("=======");
            let mut table = comfy_table::Table::new();
            table.set_header(vec!["User", "Sessions", "AI Lines", "Est. Cost"]);
            for (user, stats) in &report.by_user {
                table.add_row(vec![
                    user.as_str(),
                    &stats.sessions.to_string(),
                    &stats.lines_generated.to_string(),
                    &format!("${:.4}", stats.total_cost),
                ]);
            }
            println!("{table}");
            println!();

            // Collect unique files from audit entries to calculate code origin
            let all_files: std::collections::HashSet<String> = if let Ok(entries) = audit::collect_audit_entries(None, None, None) {
                entries.iter().flat_map(|e| e.receipts.iter().map(|r| r.file_path.clone())).collect()
            } else {
                std::collections::HashSet::new()
            };

            if !all_files.is_empty() {
                let mut total_ai_pct = 0.0f64;
                let mut files_counted = 0u32;
                for file in &all_files {
                    if let Some(origin) = crate::commands::blame::calculate_code_origin(file) {
                        total_ai_pct += origin.ai_generated_pct;
                        files_counted += 1;
                    }
                }
                if files_counted > 0 {
                    let avg_ai_pct = total_ai_pct / files_counted as f64;
                    println!("CODE ORIGIN");
                    println!("===========");
                    println!("Files with AI code: {}", files_counted);
                    println!("Avg AI-generated: {:.1}%", avg_ai_pct);
                    println!("Avg pure human: {:.1}%", 100.0 - avg_ai_pct);
                }
            }
        }
    }
}
