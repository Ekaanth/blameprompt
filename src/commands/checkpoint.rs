use crate::commands::staging;
use crate::core::{
    config, pricing,
    receipt::{FileChange, Receipt},
    redact, transcript, util,
};
use chrono::Utc;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;
use transcript::{extract_agents_spawned, extract_mcp_servers, extract_tools_used};

#[derive(Debug)]
struct HookInput {
    transcript_path: Option<String>,
    cwd: Option<String>,
    hook_event_name: Option<String>,
    tool_name: Option<String>,
    /// Prompt text sent directly in the hook payload (UserPromptSubmit event).
    prompt: Option<String>,
    /// All file paths touched by this tool call.
    /// Write/Edit produce one entry; MultiEdit produces one per edit in the edits array.
    file_paths: Vec<String>,
}

fn parse_hook_input(json_str: &str) -> HookInput {
    let v: serde_json::Value = serde_json::from_str(json_str).unwrap_or(serde_json::Value::Null);
    let tool_input = v.get("tool_input");

    // Collect all file paths from this tool invocation.
    // Write/Edit/Read: top-level file_path field.
    // MultiEdit: edits[].file_path array.
    let file_paths = if let Some(fp) = tool_input
        .and_then(|ti| ti.get("file_path"))
        .and_then(|v| v.as_str())
    {
        vec![fp.to_string()]
    } else if let Some(edits) = tool_input
        .and_then(|ti| ti.get("edits"))
        .and_then(|e| e.as_array())
    {
        edits
            .iter()
            .filter_map(|edit| {
                edit.get("file_path")
                    .and_then(|v| v.as_str())
                    .map(String::from)
            })
            .collect()
    } else {
        vec![]
    };

    HookInput {
        transcript_path: v
            .get("transcript_path")
            .and_then(|v| v.as_str())
            .map(String::from),
        cwd: v.get("cwd").and_then(|v| v.as_str()).map(String::from),
        hook_event_name: v
            .get("hook_event_name")
            .and_then(|v| v.as_str())
            .map(String::from),
        tool_name: v
            .get("tool_name")
            .and_then(|v| v.as_str())
            .map(String::from),
        prompt: v
            .get("prompt")
            .and_then(|v| v.as_str())
            .map(String::from),
        file_paths,
    }
}


/// Return all files currently modified (unstaged + staged) relative to HEAD.
/// Uses `git diff --name-only HEAD` to capture changes from any tool (Bash, Write, etc.).
fn get_all_git_modified_files(cwd: &str) -> Vec<String> {
    // Combine unstaged and staged changes relative to HEAD
    let mut files: Vec<String> = Vec::new();
    for args in &[
        &["diff", "--name-only", "HEAD"][..],
        &["diff", "--name-only", "--cached"][..],
        &["diff", "--name-only"][..],
    ] {
        if let Ok(o) = std::process::Command::new("git")
            .current_dir(cwd)
            .args(*args)
            .output()
        {
            for line in String::from_utf8_lossy(&o.stdout).lines() {
                let p = line.trim().to_string();
                if !p.is_empty() && !files.contains(&p) {
                    files.push(p);
                }
            }
        }
    }
    files
}

/// Get the git blob SHA of the current file contents.
fn get_blob_hash(cwd: &str, file_path: &str) -> Option<String> {
    let full_path = if std::path::Path::new(file_path).is_absolute() {
        file_path.to_string()
    } else {
        format!("{}/{}", cwd.trim_end_matches('/'), file_path)
    };
    let output = std::process::Command::new("git")
        .current_dir(cwd)
        .args(["hash-object", &full_path])
        .output()
        .ok()?;
    if output.status.success() {
        let hash = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !hash.is_empty() {
            return Some(hash);
        }
    }
    None
}

/// Get additions and deletions for a file using git diff --numstat.
/// Returns (additions, deletions). Tries unstaged, staged, then HEAD diffs.
fn get_diff_stats(cwd: &str, file_path: &str) -> (u32, u32) {
    let strategies: &[&[&str]] = &[
        &["diff", "--numstat", "--", file_path],
        &["diff", "--cached", "--numstat", "--", file_path],
        &["diff", "HEAD", "--numstat", "--", file_path],
    ];
    for args in strategies {
        if let Ok(o) = std::process::Command::new("git")
            .current_dir(cwd)
            .args(*args)
            .output()
        {
            let stdout = String::from_utf8_lossy(&o.stdout);
            for line in stdout.lines() {
                // numstat output: "<additions>\t<deletions>\t<file>"
                let parts: Vec<&str> = line.splitn(3, '\t').collect();
                if parts.len() >= 2 {
                    let additions = parts[0].parse::<u32>().unwrap_or(0);
                    let deletions = parts[1].parse::<u32>().unwrap_or(0);
                    if additions > 0 || deletions > 0 {
                        return (additions, deletions);
                    }
                }
            }
        }
    }
    (0, 0)
}

/// Try to detect changed lines using multiple git diff strategies.
fn get_changed_lines(cwd: &str, file_path: &str) -> (u32, u32) {
    // Strategy 1: Unstaged changes (git diff)
    if let Ok(o) = std::process::Command::new("git")
        .current_dir(cwd)
        .args(["diff", "--unified=0", "--", file_path])
        .output()
    {
        let stdout = String::from_utf8_lossy(&o.stdout);
        let (start, end) = util::diff_line_range(&stdout);
        if start > 0 {
            return (start, end);
        }
    }

    // Strategy 2: Staged changes (git diff --cached)
    if let Ok(o) = std::process::Command::new("git")
        .current_dir(cwd)
        .args(["diff", "--cached", "--unified=0", "--", file_path])
        .output()
    {
        let stdout = String::from_utf8_lossy(&o.stdout);
        let (start, end) = util::diff_line_range(&stdout);
        if start > 0 {
            return (start, end);
        }
    }

    // Strategy 3: Diff against HEAD (catches both staged + unstaged)
    if let Ok(o) = std::process::Command::new("git")
        .current_dir(cwd)
        .args(["diff", "HEAD", "--unified=0", "--", file_path])
        .output()
    {
        let stdout = String::from_utf8_lossy(&o.stdout);
        let (start, end) = util::diff_line_range(&stdout);
        if start > 0 {
            return (start, end);
        }
    }

    // Strategy 4: Count total lines in file as fallback
    let full_path = if std::path::Path::new(file_path).is_absolute() {
        file_path.to_string()
    } else {
        format!("{}/{}", cwd.trim_end_matches('/'), file_path)
    };
    if let Ok(content) = std::fs::read_to_string(&full_path) {
        let line_count = content.lines().count() as u32;
        if line_count > 0 {
            return (1, line_count);
        }
    }

    (1, 1)
}


pub fn run(agent: &str, hook_input_source: &str) {
    // Read from stdin
    let json_str = if hook_input_source == "stdin" {
        let mut buf = String::new();
        if let Err(e) = std::io::stdin().read_to_string(&mut buf) {
            eprintln!("[BlamePrompt] Failed to read hook input from stdin: {}", e);
            return;
        }
        buf
    } else {
        hook_input_source.to_string()
    };

    let input = parse_hook_input(&json_str);

    match input.hook_event_name.as_deref() {
        Some("UserPromptSubmit") => {
            // Fires the moment the user submits a prompt, before Claude responds.
            // Creates an initial receipt immediately so the prompt appears in staging
            // in real-time, parallel to the session in progress.
            handle_user_prompt_submit(agent, &input);
        }
        Some("PostToolUse") => {
            if let Some("Write" | "Edit" | "MultiEdit") = input.tool_name.as_deref() {
                handle_file_change(agent, &input);
            }
        }
        Some("Stop") => {
            // Finalizes the current prompt's receipt with conversation, tools, and cost.
            // Also creates receipts for any older prompts still missing one.
            handle_stop(agent, &input);
        }
        _ => {} // skip all other events
    }
}

/// Build common transcript context used by both file-change and stop handlers.
struct TranscriptContext {
    parsed: transcript::TranscriptParseResult,
    cwd: String,
    cfg: config::BlamePromptConfig,
    model: String,
    prompt_hash: String,
    cost: f64,
    user: String,
    message_count: u32,
}

fn build_context(input: &HookInput) -> Option<TranscriptContext> {
    let transcript_path = input.transcript_path.as_ref()?;
    let cwd = input.cwd.clone().unwrap_or_else(|| ".".to_string());

    let parsed = transcript::parse_claude_jsonl(transcript_path).ok()?;
    let cfg = config::load_config();
    let model = parsed
        .model
        .clone()
        .unwrap_or_else(|| "unknown".to_string());

    let full_text = transcript::full_conversation_text(&parsed.transcript);
    let mut hasher = Sha256::new();
    hasher.update(full_text.as_bytes());
    let prompt_hash = format!("sha256:{:x}", hasher.finalize());

    let total_chars: usize = parsed
        .transcript
        .messages
        .iter()
        .map(|m| match m {
            transcript::Message::User { text, .. } => text.len(),
            transcript::Message::Assistant { text, .. } => text.len(),
            transcript::Message::ToolUse { .. } => 0,
        })
        .sum();
    let estimated_tokens = pricing::estimate_tokens_from_chars(total_chars);
    let cost = pricing::estimate_cost(&model, estimated_tokens / 2, estimated_tokens / 2);
    let user = util::git_user();
    let message_count = parsed.transcript.messages.len() as u32;

    Some(TranscriptContext {
        parsed,
        cwd,
        cfg,
        model,
        prompt_hash,
        cost,
        user,
        message_count,
    })
}

/// Handle UserPromptSubmit — fires the instant the user submits a prompt, before Claude responds.
///
/// Creates an initial "in-progress" receipt immediately so the prompt is visible in
/// staging.json in real-time. PostToolUse and Stop events will upsert-merge into this
/// receipt, progressively adding file changes, tools, conversation, and cost.
fn handle_user_prompt_submit(agent: &str, input: &HookInput) {
    let ctx = match build_context(input) {
        Some(c) => c,
        None => return,
    };

    // At UserPromptSubmit time the transcript already contains the new user message.
    let prompt_number = transcript::count_user_prompts(&ctx.parsed.transcript);
    if prompt_number == 0 {
        return;
    }

    // At submit time the assistant hasn't replied yet, so model_for_prompt returns None.
    // Use the most recent model seen anywhere in the transcript as the best available estimate.
    // The Stop handler will overwrite with the confirmed model once the response is complete.
    let model = transcript::model_for_prompt(&ctx.parsed.transcript, prompt_number)
        .unwrap_or(ctx.model.clone());

    // Use the prompt text from the hook payload when available (cleaner, no truncation),
    // otherwise fall back to parsing the transcript.
    let prompt_summary = input
        .prompt
        .as_deref()
        .map(|p| {
            let truncated: String = p.chars().take(ctx.cfg.capture.max_prompt_length).collect();
            redact::redact_secrets_with_config(&truncated, &ctx.cfg)
        })
        .or_else(|| {
            transcript::last_user_prompt(&ctx.parsed.transcript).map(|p| {
                let truncated: String =
                    p.chars().take(ctx.cfg.capture.max_prompt_length).collect();
                redact::redact_secrets_with_config(&truncated, &ctx.cfg)
            })
        })
        .unwrap_or_default();

    if prompt_summary.is_empty() {
        return;
    }

    let receipt = Receipt {
        id: Receipt::new_id(),
        provider: agent.to_string(),
        model,
        session_id: ctx.parsed.session_id,
        prompt_summary,
        prompt_hash: ctx.prompt_hash,
        message_count: ctx.message_count,
        cost_usd: 0.0, // Not known yet; Stop will fill this in
        timestamp: Utc::now(),
        session_start: ctx.parsed.session_start,
        session_end: None, // Session not finished yet
        session_duration_secs: None,
        ai_response_time_secs: None,
        prompt_submitted_at: Some(Utc::now()), // Record exact submission time for per-prompt duration
        prompt_duration_secs: None,            // Computed at Stop time
        accepted_lines: None,
        overridden_lines: None,
        user: ctx.user,
        file_path: String::new(),
        line_range: (0, 0),
        files_changed: vec![],
        parent_receipt_id: None,
        prompt_number: Some(prompt_number),
        total_additions: 0,
        total_deletions: 0,
        tools_used: vec![],
        mcp_servers: vec![],
        agents_spawned: vec![],
        conversation: None, // Conversation populated at Stop time
    };

    staging::upsert_receipt_in(&receipt, &ctx.cwd);
}

/// Handle PostToolUse for Write/Edit/MultiEdit — creates a receipt with file changes.
fn handle_file_change(agent: &str, input: &HookInput) {
    let ctx = match build_context(input) {
        Some(c) => c,
        None => return,
    };

    let prompt_number = transcript::count_user_prompts(&ctx.parsed.transcript);

    // Use the model that actually responded to this specific prompt.
    let model = transcript::model_for_prompt(&ctx.parsed.transcript, prompt_number)
        .unwrap_or(ctx.model.clone());

    // Only extract conversation turns for THIS prompt — not the whole session history.
    let mut conversation_turns = transcript::extract_conversation_for_prompt(
        &ctx.parsed.transcript,
        prompt_number,
        ctx.cfg.capture.max_prompt_length,
        &|text| redact::redact_secrets_with_config(text, &ctx.cfg),
    );
    for turn in &mut conversation_turns {
        if let Some(ref mut files) = turn.files_touched {
            *files = files.iter().map(|f| util::make_relative(f, &ctx.cwd)).collect();
        }
    }

    // Build file change entries for every file touched by this tool call.
    // Write/Edit produce one path; MultiEdit may produce several.
    if input.file_paths.is_empty() {
        return;
    }

    let files_changed: Vec<FileChange> = input
        .file_paths
        .iter()
        .filter_map(|f| {
            let rel = util::make_relative(f, &ctx.cwd);
            // Skip internal Claude scratch files
            if rel.starts_with(".claude/") || rel.contains("/tool-results/") {
                return None;
            }
            let line_range = get_changed_lines(&ctx.cwd, &rel);
            let (additions, deletions) = get_diff_stats(&ctx.cwd, &rel);
            let blob_hash = get_blob_hash(&ctx.cwd, &rel);
            Some(FileChange {
                path: rel,
                line_range,
                blob_hash,
                additions,
                deletions,
            })
        })
        .collect();

    if files_changed.is_empty() {
        return;
    }

    let prompt_summary = transcript::last_user_prompt(&ctx.parsed.transcript)
        .map(|p| {
            let truncated: String = p.chars().take(ctx.cfg.capture.max_prompt_length).collect();
            redact::redact_secrets_with_config(&truncated, &ctx.cfg)
        })
        .unwrap_or_default();

    let receipt = Receipt {
        id: Receipt::new_id(),
        provider: agent.to_string(),
        model,
        session_id: ctx.parsed.session_id,
        prompt_summary,
        prompt_hash: ctx.prompt_hash,
        message_count: ctx.message_count,
        cost_usd: ctx.cost,
        timestamp: Utc::now(),
        session_start: ctx.parsed.session_start,
        session_end: ctx.parsed.session_end,
        session_duration_secs: ctx.parsed.session_duration_secs,
        ai_response_time_secs: ctx.parsed.avg_response_time_secs,
        prompt_submitted_at: None, // Preserved from UserPromptSubmit via upsert merge
        prompt_duration_secs: None, // Computed at Stop time
        accepted_lines: None,
        overridden_lines: None,
        user: ctx.user,
        file_path: files_changed
            .first()
            .map(|f| f.path.clone())
            .unwrap_or_default(),
        line_range: files_changed
            .first()
            .map(|f| f.line_range)
            .unwrap_or((0, 0)),
        total_additions: files_changed.iter().map(|f| f.additions).sum(),
        total_deletions: files_changed.iter().map(|f| f.deletions).sum(),
        files_changed,
        parent_receipt_id: None,
        prompt_number: Some(prompt_number),
        tools_used: extract_tools_used(&ctx.parsed.transcript),
        mcp_servers: extract_mcp_servers(&ctx.parsed.transcript),
        agents_spawned: extract_agents_spawned(&ctx.parsed.transcript),
        conversation: if conversation_turns.is_empty() {
            None
        } else {
            Some(conversation_turns)
        },
    };

    staging::upsert_receipt_in(&receipt, &ctx.cwd);
}

/// Handle Stop event — ensure every user prompt in the session has a receipt,
/// even if no files were modified (e.g. a simple "hi" prompt).
fn handle_stop(agent: &str, input: &HookInput) {
    let ctx = match build_context(input) {
        Some(c) => c,
        None => return,
    };

    let total_prompts = transcript::count_user_prompts(&ctx.parsed.transcript);
    if total_prompts == 0 {
        return;
    }

    // Read existing staging to find which prompts already have receipts
    let existing = staging::read_staging_in(Path::new(&ctx.cwd));
    let existing_prompt_numbers: Vec<Option<u32>> = existing
        .receipts
        .iter()
        .filter(|r| r.session_id == ctx.parsed.session_id)
        .map(|r| r.prompt_number)
        .collect();

    // Sweep for any git-modified files not yet tracked in the current prompt's receipt.
    // This catches files changed by Bash commands or other tools that bypass PostToolUse tracking.
    let git_modified = get_all_git_modified_files(&ctx.cwd);
    if !git_modified.is_empty() {
        // Find the last prompt in this session that already has a receipt with file changes.
        // Missing files (e.g. from Bash) are attributed to that prompt since we can't
        // determine which specific prompt caused a Bash-based file change.
        if let Some(last_pn) = existing_prompt_numbers.iter().flatten().copied().max() {
            // Build FileChanges for any git-modified file not already in the receipt
            let already_tracked: Vec<String> = existing
                .receipts
                .iter()
                .filter(|r| r.session_id == ctx.parsed.session_id && r.prompt_number == Some(last_pn))
                .flat_map(|r| r.files_changed.iter().map(|fc| fc.path.clone()))
                .collect();

            let missing_files: Vec<FileChange> = git_modified
                .iter()
                .filter(|p| {
                    !already_tracked.contains(p)
                        && !p.starts_with(".claude/")
                        && !p.contains("/tool-results/")
                        && !p.starts_with(".blameprompt")
                })
                .map(|p| {
                    let line_range = get_changed_lines(&ctx.cwd, p);
                    let (additions, deletions) = get_diff_stats(&ctx.cwd, p);
                    let blob_hash = get_blob_hash(&ctx.cwd, p);
                    FileChange {
                        path: p.clone(),
                        line_range,
                        blob_hash,
                        additions,
                        deletions,
                    }
                })
                .collect();

            if !missing_files.is_empty() {
                // Synthesise a minimal receipt that the upsert merge logic will fold into
                // the existing one for (session_id, last_pn).
                let patch_model = transcript::model_for_prompt(&ctx.parsed.transcript, last_pn)
                    .unwrap_or(ctx.model.clone());
                let patch = Receipt {
                    id: Receipt::new_id(),
                    provider: agent.to_string(),
                    model: patch_model,
                    session_id: ctx.parsed.session_id.clone(),
                    prompt_summary: String::new(),
                    prompt_hash: ctx.prompt_hash.clone(),
                    message_count: ctx.message_count,
                    cost_usd: 0.0,
                    timestamp: Utc::now(),
                    session_start: ctx.parsed.session_start,
                    session_end: ctx.parsed.session_end,
                    session_duration_secs: ctx.parsed.session_duration_secs,
                    ai_response_time_secs: ctx.parsed.avg_response_time_secs,
                    prompt_submitted_at: None, // Preserved from existing receipt via upsert merge
                    prompt_duration_secs: None,
                    accepted_lines: None,
                    overridden_lines: None,
                    user: ctx.user.clone(),
                    file_path: missing_files
                        .first()
                        .map(|f| f.path.clone())
                        .unwrap_or_default(),
                    line_range: missing_files.first().map(|f| f.line_range).unwrap_or((0, 0)),
                    total_additions: missing_files.iter().map(|f| f.additions).sum(),
                    total_deletions: missing_files.iter().map(|f| f.deletions).sum(),
                    files_changed: missing_files,
                    parent_receipt_id: None,
                    prompt_number: Some(last_pn),
                    tools_used: vec![],
                    mcp_servers: vec![],
                    agents_spawned: vec![],
                    conversation: None,
                };
                staging::upsert_receipt_in(&patch, &ctx.cwd);
            }
        }
    }

    let tools = extract_tools_used(&ctx.parsed.transcript);
    let mcps = extract_mcp_servers(&ctx.parsed.transcript);
    let agents = extract_agents_spawned(&ctx.parsed.transcript);

    // Stop fires after each prompt completes, so total_prompts IS the current prompt number.
    // Always finalize the current prompt's receipt — this updates any preliminary receipt
    // created by UserPromptSubmit with full conversation, tools, cost, and session timing.
    // The upsert merge logic preserves any file changes already written by PostToolUse.
    let current_pn = total_prompts;

    // Use the model from the assistant messages that actually responded to this prompt.
    // This correctly handles model switches mid-session (e.g. sonnet → opus).
    let current_model = transcript::model_for_prompt(&ctx.parsed.transcript, current_pn)
        .unwrap_or(ctx.model.clone());

    // Compute per-prompt duration from the submission timestamp stored at UserPromptSubmit.
    // This avoids the inflated session_duration_secs which spans the entire session JSONL
    // (growing across multiple prompts and including parallel sub-agent session time).
    let prompt_submitted_at = existing
        .receipts
        .iter()
        .find(|r| r.session_id == ctx.parsed.session_id && r.prompt_number == Some(current_pn))
        .and_then(|r| r.prompt_submitted_at);
    let prompt_duration_secs = prompt_submitted_at
        .map(|start| (Utc::now() - start).num_seconds().max(0) as u64);

    let current_summary = transcript::last_user_prompt(&ctx.parsed.transcript)
        .map(|p| {
            let truncated: String = p.chars().take(ctx.cfg.capture.max_prompt_length).collect();
            redact::redact_secrets_with_config(&truncated, &ctx.cfg)
        })
        .unwrap_or_default();

    // Extract conversation scoped to ONLY this prompt — prevents previous-session/prompt bleed.
    let mut current_turns = transcript::extract_conversation_for_prompt(
        &ctx.parsed.transcript,
        current_pn,
        ctx.cfg.capture.max_prompt_length,
        &|text| redact::redact_secrets_with_config(text, &ctx.cfg),
    );
    for turn in &mut current_turns {
        if let Some(ref mut files) = turn.files_touched {
            *files = files.iter().map(|f| util::make_relative(f, &ctx.cwd)).collect();
        }
    }

    let current_receipt = Receipt {
        id: Receipt::new_id(),
        provider: agent.to_string(),
        model: current_model.clone(),
        session_id: ctx.parsed.session_id.clone(),
        prompt_summary: current_summary,
        prompt_hash: ctx.prompt_hash.clone(),
        message_count: ctx.message_count,
        cost_usd: ctx.cost,
        timestamp: Utc::now(),
        session_start: ctx.parsed.session_start,
        session_end: ctx.parsed.session_end,
        session_duration_secs: ctx.parsed.session_duration_secs,
        ai_response_time_secs: ctx.parsed.avg_response_time_secs,
        prompt_submitted_at, // Preserved from UserPromptSubmit via upsert merge
        prompt_duration_secs, // Wall-clock time for this specific prompt only
        accepted_lines: None,
        overridden_lines: None,
        user: ctx.user.clone(),
        file_path: String::new(),
        line_range: (0, 0),
        files_changed: vec![], // files_changed is merged by upsert; don't overwrite
        parent_receipt_id: None,
        prompt_number: Some(current_pn),
        total_additions: 0,
        total_deletions: 0,
        tools_used: tools.clone(),
        mcp_servers: mcps.clone(),
        agents_spawned: agents.clone(),
        conversation: if current_turns.is_empty() {
            None
        } else {
            Some(current_turns)
        },
    };
    staging::upsert_receipt_in(&current_receipt, &ctx.cwd);

    // For any older prompts that somehow lack a receipt (edge case), create one now.
    // We only CREATE here — we do not update existing receipts for earlier prompts,
    // as they were already finalized by their own Stop event.
    for pn in 1..current_pn {
        if existing_prompt_numbers.contains(&Some(pn)) {
            continue;
        }

        let prompt_summary = transcript::nth_user_prompt(&ctx.parsed.transcript, pn)
            .map(|p| {
                let truncated: String = p.chars().take(ctx.cfg.capture.max_prompt_length).collect();
                redact::redact_secrets_with_config(&truncated, &ctx.cfg)
            })
            .unwrap_or_default();

        // Each retrospective prompt may have been answered by a different model.
        let pn_model = transcript::model_for_prompt(&ctx.parsed.transcript, pn)
            .unwrap_or(ctx.model.clone());

        // Each retrospective receipt gets only its own prompt's conversation.
        let mut pn_turns = transcript::extract_conversation_for_prompt(
            &ctx.parsed.transcript,
            pn,
            ctx.cfg.capture.max_prompt_length,
            &|text| redact::redact_secrets_with_config(text, &ctx.cfg),
        );
        for turn in &mut pn_turns {
            if let Some(ref mut files) = turn.files_touched {
                *files = files.iter().map(|f| util::make_relative(f, &ctx.cwd)).collect();
            }
        }

        let receipt = Receipt {
            id: Receipt::new_id(),
            provider: agent.to_string(),
            model: pn_model,
            session_id: ctx.parsed.session_id.clone(),
            prompt_summary,
            prompt_hash: ctx.prompt_hash.clone(),
            message_count: ctx.message_count,
            cost_usd: ctx.cost,
            timestamp: Utc::now(),
            session_start: ctx.parsed.session_start,
            session_end: ctx.parsed.session_end,
            session_duration_secs: ctx.parsed.session_duration_secs,
            ai_response_time_secs: ctx.parsed.avg_response_time_secs,
            prompt_submitted_at: None, // Not tracked for retrospectively-created receipts
            prompt_duration_secs: None,
            accepted_lines: None,
            overridden_lines: None,
            user: ctx.user.clone(),
            file_path: String::new(),
            line_range: (0, 0),
            files_changed: vec![],
            parent_receipt_id: None,
            prompt_number: Some(pn),
            total_additions: 0,
            total_deletions: 0,
            tools_used: tools.clone(),
            mcp_servers: mcps.clone(),
            agents_spawned: agents.clone(),
            conversation: if pn_turns.is_empty() {
                None
            } else {
                Some(pn_turns)
            },
        };

        staging::upsert_receipt_in(&receipt, &ctx.cwd);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hook_input() {
        let json = r#"{"transcript_path":"/tmp/test.jsonl","cwd":"/tmp","hook_event_name":"PostToolUse","tool_name":"Write","tool_input":{"file_path":"src/main.rs"}}"#;
        let input = parse_hook_input(json);
        assert_eq!(input.transcript_path.unwrap(), "/tmp/test.jsonl");
        assert_eq!(input.cwd.unwrap(), "/tmp");
        assert_eq!(input.hook_event_name.unwrap(), "PostToolUse");
        assert_eq!(input.tool_name.unwrap(), "Write");
        assert_eq!(input.file_paths, vec!["src/main.rs"]);
    }

    #[test]
    fn test_parse_hook_input_multiedit() {
        let json = r#"{"hook_event_name":"PostToolUse","tool_name":"MultiEdit","tool_input":{"edits":[{"file_path":"src/main.rs","old_string":"a","new_string":"b"},{"file_path":"src/lib.rs","old_string":"c","new_string":"d"}]}}"#;
        let input = parse_hook_input(json);
        assert_eq!(input.file_paths, vec!["src/main.rs", "src/lib.rs"]);
    }

    #[test]
    fn test_parse_hook_input_user_prompt_submit() {
        let json = r#"{"hook_event_name":"UserPromptSubmit","cwd":"/proj","transcript_path":"/tmp/s.jsonl","prompt":"fix the tests"}"#;
        let input = parse_hook_input(json);
        assert_eq!(input.hook_event_name.as_deref(), Some("UserPromptSubmit"));
        assert_eq!(input.prompt.as_deref(), Some("fix the tests"));
        assert!(input.tool_name.is_none());
        assert!(input.file_paths.is_empty());
    }

    #[test]
    fn test_parse_hook_input_missing_fields() {
        let json = r#"{}"#;
        let input = parse_hook_input(json);
        assert!(input.transcript_path.is_none());
        assert!(input.file_paths.is_empty());
        assert!(input.prompt.is_none());
    }

}
