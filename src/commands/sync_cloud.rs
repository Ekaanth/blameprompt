use crate::core::api_client::ApiClient;
use crate::core::auth;
use crate::git::notes;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

// ── Sync state persistence ──────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Default)]
struct SyncState {
    last_sync: Option<String>,
}

fn sync_state_path() -> PathBuf {
    dirs::home_dir()
        .expect("Could not determine home directory")
        .join(".blameprompt")
        .join("sync_state")
}

fn save_sync_state(ts: &DateTime<Utc>) -> Result<(), String> {
    let path = sync_state_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("Failed to create dir: {}", e))?;
    }
    let state = SyncState {
        last_sync: Some(ts.to_rfc3339()),
    };
    let content = toml::to_string_pretty(&state).map_err(|e| format!("Serialize error: {}", e))?;
    std::fs::write(&path, content).map_err(|e| format!("Failed to write sync state: {}", e))?;
    Ok(())
}

// ── Project name detection ───────────────────────────────────────────────────

fn get_project_name() -> Option<String> {
    std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().rsplit('/').next().unwrap_or("unknown").to_string())
}

// ── API payload types ───────────────────────────────────────────────────────
// Only aggregated daily activity is sent to the cloud.
// No individual prompts, responses, or per-receipt data is transmitted.

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DailyActivity {
    date: String,
    prompt_count: u32,
    total_tokens_in: u64,
    total_tokens_out: u64,
    total_cost_usd: f64,
    models_used: HashMap<String, u32>,
    providers_used: HashMap<String, u32>,
    tools_used: HashMap<String, u32>,
    categories: HashMap<String, u32>,
    avg_quality_score: f64,
    total_files_changed: u32,
    total_additions: u32,
    total_deletions: u32,
    session_count: u32,
    total_session_duration_secs: u64,
    projects_used: HashMap<String, u32>,
    hourly_prompts: HashMap<String, u32>,
    languages_used: HashMap<String, u32>,
    editors_used: HashMap<String, u32>,
    total_accepted_lines: u32,
    total_overridden_lines: u32,
    project: String,
}

#[derive(Serialize)]
struct SyncPayload {
    activities: Vec<DailyActivity>,
}

#[derive(Deserialize)]
struct SyncResponse {
    #[serde(default)]
    _ok: Option<bool>,
}

// ── Main entry point ────────────────────────────────────────────────────────

pub fn run(quiet: bool) {
    // 1. Check if logged in
    if !auth::is_logged_in() {
        if !quiet {
            eprintln!("  \x1b[1;31mError:\x1b[0m Not logged in. Run `blameprompt login` first.");
            std::process::exit(1);
        }
        return;
    }

    let api = match ApiClient::from_credentials() {
        Ok(c) => c,
        Err(e) => {
            if !quiet {
                eprintln!("  \x1b[1;31mError:\x1b[0m {}", e);
                std::process::exit(1);
            }
            return;
        }
    };

    // 2. Detect project name from git repo
    let project_name = get_project_name();

    // 3. List all commits with notes
    let commits = notes::list_commits_with_notes();

    // 4. Collect ALL receipts (no last_sync filter — backend is idempotent with REPLACE)
    let mut all_receipts = Vec::new();
    let mut seen_ids = HashSet::new();
    for sha in &commits {
        if let Some(payload) = notes::read_receipts_for_commit(sha) {
            for receipt in payload.receipts {
                if seen_ids.insert(receipt.id.clone()) {
                    all_receipts.push(receipt);
                }
            }
        }
    }

    // Also include uncommitted/staged receipts from all subdirectories
    // (handles monorepos where frontend/, backend/, etc. each have their own staging)
    let staged = crate::commands::staging::read_all_staging();
    for receipt in staged.receipts {
        if seen_ids.insert(receipt.id.clone()) {
            all_receipts.push(receipt);
        }
    }

    if all_receipts.is_empty() {
        if !quiet {
            println!("No receipts found to sync.");
        }
        return;
    }

    // 5. Aggregate receipts by date into daily activity summaries
    let mut daily: HashMap<String, DailyActivityBuilder> = HashMap::new();

    for r in &all_receipts {
        let date_key = r.timestamp.format("%Y-%m-%d").to_string();
        let day = daily.entry(date_key).or_default();

        day.prompt_count += 1;
        day.total_tokens_in += r.input_tokens.unwrap_or(0);
        day.total_tokens_out += r.output_tokens.unwrap_or(0);
        day.total_cost_usd += r.cost_usd;

        *day.models_used.entry(r.model.clone()).or_insert(0) += 1;
        *day.providers_used.entry(r.provider.clone()).or_insert(0) += 1;

        for tool in &r.tools_used {
            *day.tools_used.entry(tool.clone()).or_insert(0) += 1;
        }

        if let Some(ref pq) = r.prompt_quality {
            if let Some(ref cat) = pq.category {
                *day.categories.entry(cat.clone()).or_insert(0) += 1;
            }
            day.quality_scores.push(pq.score);
        }

        day.total_files_changed += r.all_file_changes().len() as u32;
        day.total_additions += r.effective_total_additions();
        day.total_deletions += r.effective_total_deletions();

        let hour = r.timestamp.format("%H").to_string();
        *day.hourly_prompts.entry(hour).or_insert(0) += 1;

        if let Some(ref proj) = project_name {
            *day.projects_used.entry(proj.clone()).or_insert(0) += 1;
        }

        day.session_ids.insert(r.session_id.clone());
        if let Some(dur) = r.session_duration_secs {
            let entry = day
                .session_durations
                .entry(r.session_id.clone())
                .or_insert(0);
            if dur > *entry {
                *entry = dur;
            }
        }

        // Language detection from file extensions
        for fc in r.all_file_changes() {
            if let Some(ext) = std::path::Path::new(&fc.path)
                .extension()
                .and_then(|e| e.to_str())
            {
                let ext_lower = ext.to_lowercase();
                let lang = match ext_lower.as_str() {
                    "rs" => "Rust",
                    "ts" | "tsx" => "TypeScript",
                    "js" | "jsx" => "JavaScript",
                    "py" => "Python",
                    "go" => "Go",
                    "rb" => "Ruby",
                    "java" => "Java",
                    "cpp" | "cc" | "cxx" => "C++",
                    "c" | "h" => "C",
                    "cs" => "C#",
                    "swift" => "Swift",
                    "kt" | "kts" => "Kotlin",
                    "php" => "PHP",
                    "html" | "htm" => "HTML",
                    "css" | "scss" | "sass" => "CSS",
                    "sql" => "SQL",
                    "sh" | "bash" | "zsh" => "Shell",
                    "json" => "JSON",
                    "yaml" | "yml" => "YAML",
                    "toml" => "TOML",
                    "md" | "mdx" => "Markdown",
                    "vue" => "Vue",
                    "svelte" => "Svelte",
                    "dart" => "Dart",
                    "r" => "R",
                    "scala" => "Scala",
                    "zig" => "Zig",
                    "lua" => "Lua",
                    "ex" | "exs" => "Elixir",
                    other => other,
                };
                *day.languages_used.entry(lang.to_string()).or_insert(0) += 1;
            }
        }

        // Editor detection from provider
        let editor = match r.provider.as_str() {
            "claude" => "Claude Code",
            "cursor" => "Cursor",
            "copilot" => "GitHub Copilot",
            "windsurf" => "Windsurf",
            "codex" => "OpenAI Codex",
            other => other,
        };
        *day.editors_used.entry(editor.to_string()).or_insert(0) += 1;

        // AI vs Human tracking
        if let Some(accepted) = r.accepted_lines {
            day.total_accepted_lines += accepted;
        }
        if let Some(overridden) = r.overridden_lines {
            day.total_overridden_lines += overridden;
        }
    }

    let activities: Vec<DailyActivity> = {
        let mut items: Vec<_> = daily
            .into_iter()
            .map(|(date, b)| {
                let avg_quality_score = if b.quality_scores.is_empty() {
                    0.0
                } else {
                    b.quality_scores.iter().sum::<u32>() as f64 / b.quality_scores.len() as f64
                };
                let total_session_duration_secs: u64 = b.session_durations.values().sum();
                let project = b.projects_used.keys().next().cloned().unwrap_or_else(|| "default".to_string());
                DailyActivity {
                    date,
                    prompt_count: b.prompt_count,
                    total_tokens_in: b.total_tokens_in,
                    total_tokens_out: b.total_tokens_out,
                    total_cost_usd: b.total_cost_usd,
                    models_used: b.models_used,
                    providers_used: b.providers_used,
                    tools_used: b.tools_used,
                    categories: b.categories,
                    avg_quality_score,
                    total_files_changed: b.total_files_changed,
                    total_additions: b.total_additions,
                    total_deletions: b.total_deletions,
                    session_count: b.session_ids.len() as u32,
                    total_session_duration_secs,
                    projects_used: b.projects_used,
                    hourly_prompts: b.hourly_prompts,
                    languages_used: b.languages_used,
                    editors_used: b.editors_used,
                    total_accepted_lines: b.total_accepted_lines,
                    total_overridden_lines: b.total_overridden_lines,
                    project,
                }
            })
            .collect();
        items.sort_by(|a, b| a.date.cmp(&b.date));
        items
    };

    let day_count = activities.len();

    // 6. POST to /api/sync (only aggregated daily activities — no individual prompt data)
    let payload = SyncPayload { activities };

    match api.post::<SyncPayload, SyncResponse>("/api/sync", &payload) {
        Ok(_) => {
            let now = Utc::now();
            if let Err(e) = save_sync_state(&now) {
                eprintln!(
                    "  \x1b[1;33mWarning:\x1b[0m Could not save sync state: {}",
                    e
                );
            }

            if !quiet {
                println!(
                    "  \x1b[1;32m\u{2713}\x1b[0m Synced {} day(s) to BlamePrompt Cloud",
                    day_count
                );
            }
        }
        Err(e) => {
            if !quiet {
                eprintln!("  \x1b[1;31mError:\x1b[0m Sync failed: {}", e);
                std::process::exit(1);
            }
        }
    }
}

// ── Internal builder ────────────────────────────────────────────────────────

#[derive(Default)]
struct DailyActivityBuilder {
    prompt_count: u32,
    total_tokens_in: u64,
    total_tokens_out: u64,
    total_cost_usd: f64,
    models_used: HashMap<String, u32>,
    providers_used: HashMap<String, u32>,
    tools_used: HashMap<String, u32>,
    categories: HashMap<String, u32>,
    quality_scores: Vec<u32>,
    total_files_changed: u32,
    total_additions: u32,
    total_deletions: u32,
    session_ids: HashSet<String>,
    session_durations: HashMap<String, u64>,
    projects_used: HashMap<String, u32>,
    hourly_prompts: HashMap<String, u32>,
    languages_used: HashMap<String, u32>,
    editors_used: HashMap<String, u32>,
    total_accepted_lines: u32,
    total_overridden_lines: u32,
}
