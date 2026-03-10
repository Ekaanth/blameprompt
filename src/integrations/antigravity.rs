/// Antigravity integration for blameprompt.
///
/// Since Antigravity is a drop-in replacement or extension of Gemini/Claude patterns,
/// we reuse the transcript parsing logic but identify as Antigravity provider.
use crate::core::receipt::Receipt;
use crate::integrations::gemini;
use std::path::{Path, PathBuf};

/// Check if a model name looks like it could come from the Antigravity UI.
/// Accepts Antigravity-native models plus known models served through the Antigravity platform.
fn is_antigravity_model(model_lower: &str) -> bool {
    model_lower.contains("antigravity")
        || model_lower.contains("gpt-oss")
        // Gemini family (both 2.x and 3.x served through Antigravity)
        || model_lower.contains("gemini-2")
        || model_lower.contains("gemini-3")
        // Claude models — dot notation (sonnet-4.6) and hyphenated (sonnet-4-6)
        || model_lower.contains("sonnet-4.6")
        || model_lower.contains("sonnet-4-6")
        || model_lower.contains("opus-4.6")
        || model_lower.contains("opus-4-6")
}

/// Import a specific Antigravity session file (reuse Gemini format for now).
/// Parses the file once and builds the receipt directly to avoid double I/O.
pub fn import_session(path: &Path) -> Option<Receipt> {
    let session = gemini::parse_gemini_session(path)?;

    // Only proceed if it's actually an Antigravity model
    if !is_antigravity_model(&session.model.to_lowercase()) {
        return None;
    }

    // Build receipt directly from the parsed session (avoids double parse via gemini::import_session)
    let cfg = crate::core::config::load_config();
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let user = crate::core::util::git_user();

    let first_user_msg = session
        .messages
        .iter()
        .find(|m| m.role == "user")
        .map(|m| {
            m.text
                .chars()
                .take(cfg.capture.max_prompt_length)
                .collect::<String>()
        })
        .unwrap_or_default();

    let prompt_summary = crate::core::redact::redact_secrets_with_config(&first_user_msg, &cfg);

    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(prompt_summary.as_bytes());
    let prompt_hash = format!("sha256:{:x}", hasher.finalize());

    let response_summary = session
        .messages
        .iter()
        .rev()
        .find(|m| m.role == "assistant")
        .map(|m| m.text.chars().take(500).collect());

    let files_changed: Vec<crate::core::receipt::FileChange> = session
        .files_modified
        .iter()
        .map(|f| crate::core::receipt::FileChange {
            path: crate::core::util::make_relative(f, &cwd),
            line_range: (1, 1),
            blob_hash: None,
            additions: 0,
            deletions: 0,
        })
        .collect();

    let cost = if let (Some(it), Some(ot)) = (session.input_tokens, session.output_tokens) {
        crate::core::pricing::cost_from_usage(&session.model, it, ot, 0, 0)
    } else {
        0.0
    };

    let prompt_quality = Some(crate::core::prompt_eval::evaluate(&prompt_summary));

    Some(Receipt {
        id: Receipt::new_id(),
        provider: "antigravity".to_string(),
        model: session.model,
        session_id: session.session_id,
        prompt_summary,
        response_summary,
        prompt_hash,
        message_count: session.messages.len() as u32,
        cost_usd: cost,
        input_tokens: session.input_tokens,
        output_tokens: session.output_tokens,
        cache_read_tokens: None,
        cache_creation_tokens: None,
        timestamp: session.timestamp,
        session_start: None,
        session_end: None,
        session_duration_secs: None,
        ai_response_time_secs: None,
        user,
        file_path: files_changed
            .first()
            .map(|f| f.path.clone())
            .unwrap_or_default(),
        line_range: (0, 0),
        files_changed,
        parent_receipt_id: None,
        parent_session_id: None,
        is_continuation: None,
        continuation_depth: None,
        prompt_number: Some(1),
        total_additions: 0,
        total_deletions: 0,
        tools_used: vec![],
        mcp_servers: vec![],
        agents_spawned: vec![],
        subagent_activities: vec![],
        concurrent_tool_calls: None,
        user_decisions: vec![],
        conversation: None,
        prompt_submitted_at: Some(session.timestamp),
        prompt_duration_secs: None,
        accepted_lines: None,
        overridden_lines: None,
        prompt_quality,
    })
}

/// Install proper hooks for Antigravity IDE.
/// This creates .agent/rules/blameprompt.md and .agent/workflows/checkpoint.md
pub fn install() -> Result<(), String> {
    let agent_dir = Path::new(".agent");
    let rules_dir = agent_dir.join("rules");
    let workflows_dir = agent_dir.join("workflows");

    std::fs::create_dir_all(&rules_dir)
        .map_err(|e| format!("Failed to create .agent/rules: {}", e))?;
    std::fs::create_dir_all(&workflows_dir)
        .map_err(|e| format!("Failed to create .agent/workflows: {}", e))?;

    let binary = std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "blameprompt".to_string());

    // 1. Install Rule
    let rule_content = format!(
        "# BlamePrompt Integration\n\n\
        To ensure code provenance and cost tracking, you MUST follow these rules:\n\n\
        1. After every tool call that modifies a file (e.g., `write_to_file`, `replace_file_content`, `run_command` that edits code), you MUST call the `checkpoint` workflow.\n\
        2. Before you finish a task (before calling `notify_user` or finishing a mission), you MUST call `blameprompt checkpoint antigravity` to finalize the session recording.\n\n\
        Use the following command for manual checkpoints:\n\
        ```bash\n\
        {} checkpoint antigravity\n\
        ```\n",
        binary
    );
    std::fs::write(rules_dir.join("blameprompt.md"), rule_content)
        .map_err(|e| format!("Failed to write blameprompt.md rule: {}", e))?;

    // 2. Install Workflow
    let workflow_content = format!(
        "---\n\
        description: Record a work checkpoint with BlamePrompt\n\
        ---\n\n\
        1. Run the BlamePrompt checkpoint command to record the current state and cost.\n\n\
        // turbo\n\
        2. run_command: \"{} checkpoint antigravity\"\n",
        binary
    );
    std::fs::write(workflows_dir.join("checkpoint.md"), workflow_content)
        .map_err(|e| format!("Failed to write checkpoint.md workflow: {}", e))?;

    println!("Installed Antigravity hooks in .agent/");
    Ok(())
}

/// Locate the Antigravity sessions directory.
/// Checks Antigravity-specific paths first, then falls back to Gemini paths.
fn find_antigravity_sessions_dir() -> Option<PathBuf> {
    let home = dirs::home_dir()?;

    // Antigravity-specific locations
    let ag_primary = home.join(".antigravity").join("sessions");
    if ag_primary.exists() {
        return Some(ag_primary);
    }
    let ag_root = home.join(".antigravity");
    if ag_root.exists() {
        return Some(ag_root);
    }

    // XDG for Antigravity
    let ag_xdg = home.join(".config/antigravity/sessions");
    if ag_xdg.exists() {
        return Some(ag_xdg);
    }

    if let Ok(data_dir) = std::env::var("XDG_DATA_HOME") {
        let ag_custom = PathBuf::from(&data_dir).join("antigravity/sessions");
        if ag_custom.exists() {
            return Some(ag_custom);
        }
    }

    // Fall back to Gemini paths (Antigravity may store sessions there)
    gemini::find_sessions_dir()
}

/// Resolve session files from an optional path or the default Antigravity directory.
fn resolve_session_files(session_path: Option<&str>) -> Vec<PathBuf> {
    if let Some(path) = session_path {
        let p = PathBuf::from(path);
        if !p.exists() {
            eprintln!("File not found: {}", path);
            return vec![];
        }
        if p.is_dir() {
            gemini::list_session_files(&p)
        } else {
            vec![p]
        }
    } else {
        match find_antigravity_sessions_dir() {
            Some(dir) => {
                let files = gemini::list_session_files(&dir);
                if files.is_empty() {
                    eprintln!("No session files found in {}", dir.display());
                    return vec![];
                }
                files.into_iter().take(10).collect()
            }
            None => {
                eprintln!("Cannot find Antigravity sessions directory.");
                eprintln!("  Checked: ~/.antigravity/sessions/, ~/.config/antigravity/sessions/");
                eprintln!("  Pass --session <path> to specify a transcript file.");
                vec![]
            }
        }
    }
}

/// Record Antigravity sessions from the default directory.
pub fn run_record_antigravity(session_path: Option<&str>) {
    let files = resolve_session_files(session_path);
    let mut count = 0;
    for path in files {
        if let Some(receipt) = import_session(&path) {
            crate::commands::staging::upsert_receipt(&receipt);
            count += 1;
        }
    }

    if count > 0 {
        println!("[antigravity] Recorded {} Antigravity session(s)", count);
        println!("  Receipts staged. They will be attached on next git commit.");
    } else {
        eprintln!("[antigravity] No valid Antigravity sessions found.");
    }
}
