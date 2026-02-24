use crate::git::notes;
use comfy_table::Table;

pub fn run(query: &str, limit: usize) {
    let commits = notes::list_commits_with_notes();

    if commits.is_empty() {
        println!("No BlamePrompt notes found in this repository.");
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
        println!("No receipts matching \"{}\"", query);
        return;
    }

    println!("Search results for \"{}\": {} match(es)", query, matches.len());
    println!();

    let mut table = Table::new();
    table.set_header(vec![
        "Commit", "Provider", "Model", "File", "Lines", "Cost", "Prompt Summary",
    ]);

    for (sha, r) in &matches {
        let sha_short = if sha.len() >= 8 { &sha[..8] } else { sha };
        let prompt: String = r.prompt_summary.chars().take(50).collect();

        table.add_row(vec![
            sha_short,
            &r.provider,
            &r.model,
            &r.file_path,
            &format!("{}-{}", r.line_range.0, r.line_range.1),
            &format!("${:.4}", r.cost_usd),
            &prompt,
        ]);
    }

    println!("{table}");

    if matches.len() >= limit {
        println!("\n(showing first {} results, use --limit to see more)", limit);
    }
}
