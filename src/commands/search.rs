use crate::commands::audit;
use crate::git::notes;
use comfy_table::Table;
use serde::Serialize;

#[derive(Serialize)]
pub struct SearchResult {
    pub commit_sha: String,
    pub receipt_id: String,
    pub provider: String,
    pub model: String,
    pub file_path: String,
    pub line_range: (u32, u32),
    pub cost_usd: f64,
    pub prompt_summary: String,
    pub timestamp: String,
    pub user: String,
    pub session_id: String,
    pub message_count: u32,
    pub has_conversation: bool,
}

#[derive(Serialize)]
pub struct SearchOutput {
    pub query: String,
    pub total_matches: usize,
    pub results: Vec<SearchResult>,
}

pub fn run(query: &str, limit: usize, format: &str) {
    let commits = notes::list_commits_with_notes();

    if commits.is_empty() {
        if format == "json" {
            println!(
                "{{\"query\":\"{}\",\"total_matches\":0,\"results\":[]}}",
                query
            );
        } else {
            println!("No BlamePrompt notes found in this repository.");
        }
        return;
    }

    let query_lower = query.to_lowercase();
    let mut matches = Vec::new();

    for sha in &commits {
        if let Some(payload) = notes::read_receipts_for_commit(sha) {
            for r in &payload.receipts {
                if r.prompt_summary.to_lowercase().contains(&query_lower)
                    || r.file_path.to_lowercase().contains(&query_lower)
                    || r.model.to_lowercase().contains(&query_lower)
                    || r.provider.to_lowercase().contains(&query_lower)
                {
                    matches.push((sha.clone(), r.clone()));
                }
                if matches.len() >= limit {
                    break;
                }
            }
        }
        if matches.len() >= limit {
            break;
        }
    }

    if matches.is_empty() {
        if format == "json" {
            println!(
                "{{\"query\":\"{}\",\"total_matches\":0,\"results\":[]}}",
                query
            );
        } else {
            println!("No receipts matching \"{}\"", query);
        }
        return;
    }

    // JSON output
    if format == "json" {
        let output = SearchOutput {
            query: query.to_string(),
            total_matches: matches.len(),
            results: matches
                .iter()
                .map(|(sha, r)| SearchResult {
                    commit_sha: sha.clone(),
                    receipt_id: r.id.clone(),
                    provider: r.provider.clone(),
                    model: r.model.clone(),
                    file_path: audit::relative_path(&r.file_path),
                    line_range: r.line_range,
                    cost_usd: r.cost_usd,
                    prompt_summary: r.prompt_summary.clone(),
                    timestamp: r.timestamp.to_rfc3339(),
                    user: r.user.clone(),
                    session_id: r.session_id.clone(),
                    message_count: r.message_count,
                    has_conversation: r.conversation.is_some(),
                })
                .collect(),
        };
        println!("{}", serde_json::to_string_pretty(&output).unwrap());
        return;
    }

    // Table output (default)
    println!(
        "Search results for \"{}\": {} match(es)",
        query,
        matches.len()
    );
    println!();

    let mut table = Table::new();
    table.set_header(vec![
        "Commit",
        "Provider",
        "Model",
        "File",
        "Lines",
        "Cost",
        "Prompt Summary",
    ]);

    for (sha, r) in &matches {
        let sha_short = if sha.len() >= 8 { &sha[..8] } else { sha };
        let prompt: String = r.prompt_summary.chars().take(50).collect();
        let rel_file = audit::relative_path(&r.file_path);

        table.add_row(vec![
            sha_short,
            &r.provider,
            &r.model,
            &rel_file,
            &format!("{}-{}", r.line_range.0, r.line_range.1),
            &format!("${:.4}", r.cost_usd),
            &prompt,
        ]);
    }

    println!("{table}");

    if matches.len() >= limit {
        println!(
            "\n(showing first {} results, use --limit to see more)",
            limit
        );
    }
}
