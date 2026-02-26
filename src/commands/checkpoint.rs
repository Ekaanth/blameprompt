use crate::commands::staging;
use crate::core::{
    config, pricing,
    receipt::{DecisionOption, FileChange, Receipt, SubagentActivity, UserDecision},
    redact, transcript, util,
};
use crate::git::notes;
use chrono::Utc;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;
use transcript::{
    count_concurrent_tools, extract_agents_for_prompt, extract_mcps_for_prompt,
    extract_tools_for_prompt, token_usage_for_prompt,
};

#[derive(Debug)]
struct HookInput {
    /// Session ID from the hook payload (present in all events).
    session_id: Option<String>,
    /// Parent session ID if Claude Code provides it (future-proof for continuations).
    parent_session_id: Option<String>,
    /// Agent ID from SubagentStart/SubagentStop events.
    agent_id: Option<String>,
    /// Agent type from SubagentStart/SubagentStop events (e.g., "Explore", "Plan").
    agent_type: Option<String>,
    /// Path to the subagent's transcript (SubagentStop event).
    agent_transcript_path: Option<String>,
    transcript_path: Option<String>,
    cwd: Option<String>,
    hook_event_name: Option<String>,
    tool_name: Option<String>,
    /// Prompt text sent directly in the hook payload (UserPromptSubmit event).
    prompt: Option<String>,
    /// All file paths touched by this tool call.
    /// Write/Edit produce one entry; MultiEdit produces one per edit in the edits array.
    file_paths: Vec<String>,
    /// The AI's final response text (Stop and SubagentStop events).
    last_assistant_message: Option<String>,
    /// Tool execution result (PostToolUse event). Reserved for future use.
    #[allow(dead_code)]
    tool_response: Option<serde_json::Value>,
    /// Raw tool_input JSON from PostToolUse. Used to parse AskUserQuestion questions.
    tool_input: Option<serde_json::Value>,
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
        session_id: v
            .get("session_id")
            .and_then(|v| v.as_str())
            .map(String::from),
        parent_session_id: v
            .get("parent_session_id")
            .and_then(|v| v.as_str())
            .map(String::from),
        agent_id: v.get("agent_id").and_then(|v| v.as_str()).map(String::from),
        agent_type: v
            .get("agent_type")
            .and_then(|v| v.as_str())
            .map(String::from),
        agent_transcript_path: v
            .get("agent_transcript_path")
            .and_then(|v| v.as_str())
            .map(String::from),
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
        prompt: v.get("prompt").and_then(|v| v.as_str()).map(String::from),
        file_paths,
        last_assistant_message: v
            .get("last_assistant_message")
            .and_then(|v| v.as_str())
            .map(String::from),
        tool_response: v.get("tool_response").cloned(),
        tool_input: tool_input.cloned(),
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
        Some("PostToolUse") => match input.tool_name.as_deref() {
            Some("Write" | "Edit" | "MultiEdit") => {
                handle_file_change(agent, &input);
            }
            Some("AskUserQuestion") => {
                handle_ask_user_question(&input);
            }
            _ => {}
        },
        Some("Stop") => {
            // Finalizes the current prompt's receipt with conversation, tools, and cost.
            // Also creates receipts for any older prompts still missing one.
            handle_stop(agent, &input);
        }
        Some("SubagentStart") => {
            handle_subagent_start(&input);
        }
        Some("SubagentStop") => {
            handle_subagent_stop(&input);
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

    let user = util::git_user();
    let message_count = parsed.transcript.messages.len() as u32;

    Some(TranscriptContext {
        parsed,
        cwd,
        cfg,
        model,
        prompt_hash,
        user,
        message_count,
    })
}

/// Compute per-prompt cost and token usage.
///
/// Uses `token_usage_for_prompt()` to sum only the assistant messages within the prompt's
/// message slice, avoiding the cumulative full-session totals that inflate costs.
/// Falls back to the full-session context values if per-prompt data isn't available.
fn prompt_cost_and_tokens(
    ctx: &TranscriptContext,
    prompt_number: u32,
) -> (f64, Option<transcript::TokenUsage>) {
    if let Some(usage) = token_usage_for_prompt(&ctx.parsed.transcript, prompt_number) {
        let cost = pricing::cost_from_usage(
            &ctx.model,
            usage.input_tokens,
            usage.output_tokens,
            usage.cache_read_tokens,
            usage.cache_creation_tokens,
        );
        (cost, Some(usage))
    } else {
        // No per-prompt usage data — fall back to char-based estimate for this prompt's slice
        (0.0, None)
    }
}

/// Prefix that Claude Code uses when continuing from a prior session that exhausted its context.
const CONTINUATION_MARKER: &str = "This session is being continued from a previous conversation";

/// Detect whether a prompt is a session continuation and resolve the parent session ID.
///
/// Resolution priority:
/// 1. `parent_session_id` from hook payload (future Claude Code feature)
/// 2. Prompt text starts with the continuation marker — look up the most recent session
///    in staging.json, then fall back to scanning recent git notes.
///
/// Returns `(parent_session_id, continuation_depth)`.
fn detect_continuation(input: &HookInput, cwd: &str) -> (Option<String>, u32) {
    // Priority 1: Claude Code provides parent_session_id directly in hook payload
    if let Some(ref psid) = input.parent_session_id {
        let depth = find_depth_for_session(psid, cwd) + 1;
        return (Some(psid.clone()), depth);
    }

    // Priority 2: Detect from prompt text
    let prompt = input.prompt.as_deref().unwrap_or("");
    if !prompt.starts_with(CONTINUATION_MARKER) {
        return (None, 0);
    }

    // Find the most recent session in staging.json
    let staging_data = staging::read_staging_in(Path::new(cwd));
    if let Some(last_receipt) = staging_data.receipts.last() {
        // Only link if the last receipt is from a different session (not the same one)
        let current_sid = input.session_id.as_deref().unwrap_or("");
        if last_receipt.session_id != current_sid {
            let parent_sid = last_receipt.session_id.clone();
            let depth = last_receipt.continuation_depth.unwrap_or(0) + 1;
            return (Some(parent_sid), depth);
        }
    }

    // Staging is empty or same session — scan recent git notes for the parent
    if let Some((parent_sid, parent_depth)) =
        find_most_recent_session_from_notes(input.session_id.as_deref().unwrap_or(""))
    {
        return (Some(parent_sid), parent_depth + 1);
    }

    // We know it's a continuation but can't find the parent
    (None, 1)
}

/// Walk staging to find the continuation depth of a given session ID.
fn find_depth_for_session(session_id: &str, cwd: &str) -> u32 {
    let staging_data = staging::read_staging_in(Path::new(cwd));
    for r in staging_data.receipts.iter().rev() {
        if r.session_id == session_id {
            return r.continuation_depth.unwrap_or(0);
        }
    }
    0
}

/// Scan recent git commits for the most recent session_id stored in blameprompt notes.
/// Skips receipts matching `current_session_id` to avoid self-linking.
fn find_most_recent_session_from_notes(current_session_id: &str) -> Option<(String, u32)> {
    let commits = notes::list_commits_with_notes();
    for sha in commits.iter().take(10) {
        if let Some(payload) = notes::read_receipts_for_commit(sha) {
            // Walk receipts in reverse to find the most recent one from a different session
            for r in payload.receipts.iter().rev() {
                if r.session_id != current_session_id {
                    return Some((r.session_id.clone(), r.continuation_depth.unwrap_or(0)));
                }
            }
        }
    }
    None
}

/// Handle UserPromptSubmit — fires the instant the user submits a prompt, before Claude responds.
///
/// Creates an initial "in-progress" receipt immediately so the prompt is visible in
/// staging.json in real-time. PostToolUse and Stop events will upsert-merge into this
/// receipt, progressively adding file changes, tools, conversation, and cost.
///
/// **Race condition fix**: If the transcript hasn't been updated yet (common when Claude Code
/// fires this hook before flushing the JSONL), we fall back to creating the receipt from the
/// hook payload alone (session_id, prompt text, cwd).
fn handle_user_prompt_submit(agent: &str, input: &HookInput) {
    let cwd = input.cwd.clone().unwrap_or_else(|| ".".to_string());
    let cfg = config::load_config();

    // Try to build full transcript context; if it fails or has 0 prompts, fall back.
    let (prompt_number, model, session_id, prompt_hash, session_start, message_count) =
        if let Some(ctx) = build_context(input) {
            let count = transcript::count_user_prompts(&ctx.parsed.transcript);
            if count == 0 {
                // Transcript doesn't have the user message yet — fall back to payload-only.
                let sid = input
                    .session_id
                    .clone()
                    .unwrap_or_else(|| ctx.parsed.session_id.clone());
                (
                    1u32,
                    ctx.model,
                    sid,
                    ctx.prompt_hash,
                    ctx.parsed.session_start,
                    ctx.message_count,
                )
            } else {
                // UserPromptSubmit fires BEFORE Claude Code writes the user message to
                // the JSONL transcript. So count_user_prompts returns the count of PREVIOUS
                // prompts, not including this one. We need count + 1.
                //
                // Safety check: if the transcript's last user message matches the hook's
                // prompt text, the JSONL was already updated (rare) and count is correct.
                let hook_prompt = input.prompt.as_deref().unwrap_or("");
                let pn = if !hook_prompt.is_empty() {
                    let last = transcript::nth_user_prompt(&ctx.parsed.transcript, count)
                        .unwrap_or_default();
                    let prefix_len = 80.min(hook_prompt.len()).min(last.len());
                    if prefix_len > 0
                        && hook_prompt.chars().take(80).collect::<String>()
                            == last.chars().take(80).collect::<String>()
                    {
                        count // Transcript already has this prompt (rare)
                    } else {
                        count + 1 // Normal: transcript not yet updated
                    }
                } else {
                    count + 1
                };
                let m = transcript::model_for_prompt(&ctx.parsed.transcript, pn)
                    .unwrap_or(ctx.model.clone());
                (
                    pn,
                    m,
                    ctx.parsed.session_id,
                    ctx.prompt_hash,
                    ctx.parsed.session_start,
                    ctx.message_count,
                )
            }
        } else {
            // Transcript file not readable — create receipt from hook payload alone.
            let sid = input
                .session_id
                .clone()
                .unwrap_or_else(|| "unknown".to_string());
            let hash = {
                let mut hasher = Sha256::new();
                hasher.update(input.prompt.as_deref().unwrap_or("").as_bytes());
                format!("sha256:{:x}", hasher.finalize())
            };
            (1u32, "unknown".to_string(), sid, hash, None, 0u32)
        };

    // Use the prompt text from the hook payload when available (cleaner, no truncation),
    // otherwise fall back to parsing the transcript.
    let prompt_summary = input
        .prompt
        .as_deref()
        .filter(|p| !p.is_empty())
        .map(|p| {
            let truncated: String = p.chars().take(cfg.capture.max_prompt_length).collect();
            redact::redact_secrets_with_config(&truncated, &cfg)
        })
        .unwrap_or_default();

    if prompt_summary.is_empty() {
        return;
    }

    let user = util::git_user();

    // Detect session continuation (context exhaustion → new session)
    let (parent_session_id, continuation_depth) = detect_continuation(input, &cwd);
    let is_continuation = if parent_session_id.is_some() || continuation_depth > 0 {
        Some(true)
    } else {
        None
    };

    let receipt = Receipt {
        id: Receipt::new_id(),
        provider: agent.to_string(),
        model,
        session_id,
        prompt_summary,
        response_summary: None, // Populated at Stop time from last_assistant_message
        prompt_hash,
        message_count,
        cost_usd: 0.0, // Not known yet; Stop will fill this in
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        cache_creation_tokens: None,
        timestamp: Utc::now(),
        session_start,
        session_end: None, // Session not finished yet
        session_duration_secs: None,
        ai_response_time_secs: None,
        prompt_submitted_at: Some(Utc::now()), // Record exact submission time for per-prompt duration
        prompt_duration_secs: None,            // Computed at Stop time
        accepted_lines: None,
        overridden_lines: None,
        user,
        file_path: String::new(),
        line_range: (0, 0),
        files_changed: vec![],
        parent_receipt_id: None,
        parent_session_id,
        is_continuation,
        continuation_depth: if continuation_depth > 0 {
            Some(continuation_depth)
        } else {
            None
        },
        prompt_number: Some(prompt_number),
        total_additions: 0,
        total_deletions: 0,
        tools_used: vec![],
        mcp_servers: vec![],
        agents_spawned: vec![],
        subagent_activities: vec![],
        concurrent_tool_calls: None,
        user_decisions: vec![],
        conversation: None, // Conversation populated at Stop time
    };

    staging::upsert_receipt_in(&receipt, &cwd);
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
            *files = files
                .iter()
                .map(|f| util::make_relative(f, &ctx.cwd))
                .collect();
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

    // Use nth_user_prompt (not last_user_prompt) so the summary matches THIS prompt,
    // even if the transcript already contains a newer prompt by the time PostToolUse fires.
    let prompt_summary = transcript::nth_user_prompt(&ctx.parsed.transcript, prompt_number)
        .map(|p| {
            let truncated: String = p.chars().take(ctx.cfg.capture.max_prompt_length).collect();
            redact::redact_secrets_with_config(&truncated, &ctx.cfg)
        })
        .unwrap_or_default();

    // Per-prompt cost/tokens — avoids the cumulative full-session totals
    let (prompt_cost, prompt_tokens) = prompt_cost_and_tokens(&ctx, prompt_number);

    let receipt = Receipt {
        id: Receipt::new_id(),
        provider: agent.to_string(),
        model,
        session_id: ctx.parsed.session_id,
        prompt_summary,
        response_summary: None, // Populated at Stop time
        prompt_hash: ctx.prompt_hash,
        message_count: ctx.message_count,
        cost_usd: prompt_cost,
        input_tokens: prompt_tokens.as_ref().map(|u| u.input_tokens),
        output_tokens: prompt_tokens.as_ref().map(|u| u.output_tokens),
        cache_read_tokens: prompt_tokens.as_ref().map(|u| u.cache_read_tokens),
        cache_creation_tokens: prompt_tokens.as_ref().map(|u| u.cache_creation_tokens),
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
        parent_session_id: None,
        is_continuation: None,
        continuation_depth: None,
        prompt_number: Some(prompt_number),
        tools_used: extract_tools_for_prompt(&ctx.parsed.transcript, prompt_number),
        mcp_servers: extract_mcps_for_prompt(&ctx.parsed.transcript, prompt_number),
        agents_spawned: extract_agents_for_prompt(&ctx.parsed.transcript, prompt_number),
        subagent_activities: vec![],
        concurrent_tool_calls: None,
        user_decisions: vec![],
        conversation: if conversation_turns.is_empty() {
            None
        } else {
            Some(conversation_turns)
        },
    };

    staging::upsert_receipt_in(&receipt, &ctx.cwd);
}

/// Handle PostToolUse for AskUserQuestion — captures questions and options in real-time.
/// The user's answer will be enriched later at Stop time from the transcript.
fn handle_ask_user_question(input: &HookInput) {
    let cwd = input.cwd.clone().unwrap_or_else(|| ".".to_string());
    let session_id = match input.session_id.as_ref() {
        Some(s) => s.clone(),
        None => return,
    };
    let tool_input = match input.tool_input.as_ref() {
        Some(ti) => ti,
        None => return,
    };

    let questions = match tool_input.get("questions").and_then(|q| q.as_array()) {
        Some(qs) => qs,
        None => return,
    };

    let decisions: Vec<UserDecision> = questions
        .iter()
        .enumerate()
        .filter_map(|(i, q)| {
            let question = q.get("question").and_then(|v| v.as_str())?.to_string();
            let header = q.get("header").and_then(|v| v.as_str()).map(String::from);
            let multi_select = q
                .get("multiSelect")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let options: Vec<DecisionOption> = q
                .get("options")
                .and_then(|o| o.as_array())
                .map(|opts| {
                    opts.iter()
                        .filter_map(|opt| {
                            Some(DecisionOption {
                                label: opt.get("label").and_then(|v| v.as_str())?.to_string(),
                                selected: false,
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            // Use a temporary ID; the real tool_use_id is set at Stop time
            // by matching on question text from the transcript.
            Some(UserDecision {
                tool_use_id: format!("pending_{}", i),
                question,
                header,
                options,
                multi_select,
                answer: None,
            })
        })
        .collect();

    if decisions.is_empty() {
        return;
    }

    // Directly update the staging receipt for this session
    let mut data = staging::read_staging_in(Path::new(&cwd));
    let last_pn = data
        .receipts
        .iter()
        .filter(|r| r.session_id == session_id)
        .filter_map(|r| r.prompt_number)
        .max();

    if let Some(pn) = last_pn {
        if let Some(receipt) = data
            .receipts
            .iter_mut()
            .find(|r| r.session_id == session_id && r.prompt_number == Some(pn))
        {
            for decision in decisions {
                // Skip if a decision with same question text already tracked
                if !receipt
                    .user_decisions
                    .iter()
                    .any(|d| d.question == decision.question)
                {
                    receipt.user_decisions.push(decision);
                }
            }
            staging::write_staging_data_in(&data, &cwd);
        }
    }
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
    //
    // IMPORTANT: Only sweep if the target prompt actually used Bash (or similar tools that
    // modify files without firing PostToolUse). Without this guard, prompts like "hello"
    // would incorrectly get ALL uncommitted files attributed to them.
    let git_modified = get_all_git_modified_files(&ctx.cwd);
    if !git_modified.is_empty() {
        if let Some(last_pn) = existing_prompt_numbers.iter().flatten().copied().max() {
            // Only sweep if the target prompt used Bash — the only tool that can modify
            // files without firing PostToolUse. Skip for prompts with no file-modifying tools.
            let target_tools =
                transcript::extract_tools_for_prompt(&ctx.parsed.transcript, last_pn);
            let has_bash = target_tools.iter().any(|t| t == "Bash");

            if has_bash {
                // Build already_tracked from BOTH staging data AND transcript.
                // This prevents files from earlier prompts leaking onto the current prompt
                // even when staging.json was deleted/reset (the transcript is always available).
                let mut already_tracked: Vec<String> = existing
                    .receipts
                    .iter()
                    .filter(|r| r.session_id == ctx.parsed.session_id)
                    .flat_map(|r| r.files_changed.iter().map(|fc| fc.path.clone()))
                    .collect();

                // Also extract files from ALL other prompts' tool calls in the transcript.
                // This is the fallback that catches files when staging was deleted.
                for pn in 1..=total_prompts {
                    if pn == last_pn {
                        continue; // Don't exclude the target prompt's own files
                    }
                    let pn_files = transcript::files_for_prompt(&ctx.parsed.transcript, pn);
                    for f in pn_files {
                        let rel = util::make_relative(&f, &ctx.cwd);
                        if !already_tracked.contains(&rel) {
                            already_tracked.push(rel);
                        }
                    }
                }

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
                        response_summary: None,
                        prompt_hash: ctx.prompt_hash.clone(),
                        message_count: ctx.message_count,
                        cost_usd: 0.0,
                        input_tokens: None,
                        output_tokens: None,
                        cache_read_tokens: None,
                        cache_creation_tokens: None,
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
                        line_range: missing_files
                            .first()
                            .map(|f| f.line_range)
                            .unwrap_or((0, 0)),
                        total_additions: missing_files.iter().map(|f| f.additions).sum(),
                        total_deletions: missing_files.iter().map(|f| f.deletions).sum(),
                        files_changed: missing_files,
                        parent_receipt_id: None,
                        parent_session_id: None,
                        is_continuation: None,
                        continuation_depth: None,
                        prompt_number: Some(last_pn),
                        tools_used: vec![],
                        mcp_servers: vec![],
                        agents_spawned: vec![],
                        subagent_activities: vec![],
                        concurrent_tool_calls: None,
                        user_decisions: vec![],
                        conversation: None,
                    };
                    staging::upsert_receipt_in(&patch, &ctx.cwd);
                }
            } // if has_bash
        }
    }

    // Stop fires after each prompt completes, so total_prompts IS the current prompt number.
    // Always finalize the current prompt's receipt — this updates any preliminary receipt
    // created by UserPromptSubmit with full conversation, tools, cost, and session timing.
    // The upsert merge logic preserves any file changes already written by PostToolUse.
    let current_pn = total_prompts;

    // Extract tools/MCP/agents scoped to THIS prompt only (not the full session).
    // Using full-session extraction would attribute every tool from every prompt to each receipt.
    let tools = extract_tools_for_prompt(&ctx.parsed.transcript, current_pn);
    let mcps = extract_mcps_for_prompt(&ctx.parsed.transcript, current_pn);
    let agents = extract_agents_for_prompt(&ctx.parsed.transcript, current_pn);

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
    let prompt_duration_secs =
        prompt_submitted_at.map(|start| (Utc::now() - start).num_seconds().max(0) as u64);

    // Use nth_user_prompt (not last_user_prompt) to get the EXACT prompt for this receipt.
    // last_user_prompt is wrong here because by the time Stop fires, the transcript JSONL
    // may already contain a newer prompt, causing the summary to bleed into the wrong receipt.
    let current_summary = transcript::nth_user_prompt(&ctx.parsed.transcript, current_pn)
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
            *files = files
                .iter()
                .map(|f| util::make_relative(f, &ctx.cwd))
                .collect();
        }
    }

    // Capture the AI's response summary from the Stop hook's last_assistant_message.
    // Truncate to a reasonable length for storage.
    let response_summary = input
        .last_assistant_message
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(|s| {
            let truncated: String = s.chars().take(ctx.cfg.capture.max_prompt_length).collect();
            redact::redact_secrets_with_config(&truncated, &ctx.cfg)
        });

    // Per-prompt cost/tokens — avoids the cumulative full-session totals that inflate costs.
    let (prompt_cost, prompt_tokens) = prompt_cost_and_tokens(&ctx, current_pn);

    // Use the actual prompt timestamp from the JSONL instead of Utc::now().
    // This ensures the receipt shows when the prompt was submitted, not when Stop fired.
    let prompt_ts =
        transcript::timestamp_for_prompt(&ctx.parsed, current_pn).unwrap_or_else(Utc::now);

    let current_receipt = Receipt {
        id: Receipt::new_id(),
        provider: agent.to_string(),
        model: current_model.clone(),
        session_id: ctx.parsed.session_id.clone(),
        prompt_summary: current_summary,
        response_summary,
        prompt_hash: ctx.prompt_hash.clone(),
        message_count: ctx.message_count,
        cost_usd: prompt_cost,
        input_tokens: prompt_tokens.as_ref().map(|u| u.input_tokens),
        output_tokens: prompt_tokens.as_ref().map(|u| u.output_tokens),
        cache_read_tokens: prompt_tokens.as_ref().map(|u| u.cache_read_tokens),
        cache_creation_tokens: prompt_tokens.as_ref().map(|u| u.cache_creation_tokens),
        timestamp: prompt_ts,
        session_start: ctx.parsed.session_start,
        session_end: ctx.parsed.session_end,
        session_duration_secs: ctx.parsed.session_duration_secs,
        ai_response_time_secs: ctx.parsed.avg_response_time_secs,
        prompt_submitted_at,  // Preserved from UserPromptSubmit via upsert merge
        prompt_duration_secs, // Wall-clock time for this specific prompt only
        accepted_lines: None,
        overridden_lines: None,
        user: ctx.user.clone(),
        file_path: String::new(),
        line_range: (0, 0),
        files_changed: vec![], // files_changed is merged by upsert; don't overwrite
        parent_receipt_id: None,
        parent_session_id: None,
        is_continuation: None,
        continuation_depth: None,
        prompt_number: Some(current_pn),
        total_additions: 0,
        total_deletions: 0,
        tools_used: tools.clone(),
        mcp_servers: mcps.clone(),
        agents_spawned: agents.clone(),
        subagent_activities: vec![],
        concurrent_tool_calls: {
            let c = count_concurrent_tools(&ctx.parsed.transcript);
            if c > 1 {
                Some(c)
            } else {
                None
            }
        },
        user_decisions: transcript::extract_user_decisions(&ctx.parsed.transcript),
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
    //
    // Skip prompts that were already committed to git notes (e.g. after `git commit`
    // cleared staging.json). Without this check, the backfill loop would recreate
    // receipts for all earlier prompts in the session every time Stop fires.
    let committed_max = staging::committed_max_prompt(&ctx.parsed.session_id, &ctx.cwd);

    for pn in 1..current_pn {
        if existing_prompt_numbers.contains(&Some(pn)) || pn <= committed_max {
            continue;
        }

        let prompt_summary = transcript::nth_user_prompt(&ctx.parsed.transcript, pn)
            .map(|p| {
                let truncated: String = p.chars().take(ctx.cfg.capture.max_prompt_length).collect();
                redact::redact_secrets_with_config(&truncated, &ctx.cfg)
            })
            .unwrap_or_default();

        // Each retrospective prompt may have been answered by a different model.
        let pn_model =
            transcript::model_for_prompt(&ctx.parsed.transcript, pn).unwrap_or(ctx.model.clone());

        // Each retrospective receipt gets only its own prompt's conversation.
        let mut pn_turns = transcript::extract_conversation_for_prompt(
            &ctx.parsed.transcript,
            pn,
            ctx.cfg.capture.max_prompt_length,
            &|text| redact::redact_secrets_with_config(text, &ctx.cfg),
        );
        for turn in &mut pn_turns {
            if let Some(ref mut files) = turn.files_touched {
                *files = files
                    .iter()
                    .map(|f| util::make_relative(f, &ctx.cwd))
                    .collect();
            }
        }

        // Per-prompt cost/tokens for retrospective receipts too
        let (pn_cost, pn_tokens) = prompt_cost_and_tokens(&ctx, pn);

        // Use the actual prompt timestamp from the JSONL for backfilled receipts.
        let pn_ts = transcript::timestamp_for_prompt(&ctx.parsed, pn).unwrap_or_else(Utc::now);

        let receipt = Receipt {
            id: Receipt::new_id(),
            provider: agent.to_string(),
            model: pn_model,
            session_id: ctx.parsed.session_id.clone(),
            prompt_summary,
            response_summary: None, // Not available for retrospective receipts
            prompt_hash: ctx.prompt_hash.clone(),
            message_count: ctx.message_count,
            cost_usd: pn_cost,
            input_tokens: pn_tokens.as_ref().map(|u| u.input_tokens),
            output_tokens: pn_tokens.as_ref().map(|u| u.output_tokens),
            cache_read_tokens: pn_tokens.as_ref().map(|u| u.cache_read_tokens),
            cache_creation_tokens: pn_tokens.as_ref().map(|u| u.cache_creation_tokens),
            timestamp: pn_ts,
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
            parent_session_id: None,
            is_continuation: None,
            continuation_depth: None,
            prompt_number: Some(pn),
            total_additions: 0,
            total_deletions: 0,
            tools_used: extract_tools_for_prompt(&ctx.parsed.transcript, pn),
            mcp_servers: extract_mcps_for_prompt(&ctx.parsed.transcript, pn),
            agents_spawned: extract_agents_for_prompt(&ctx.parsed.transcript, pn),
            subagent_activities: vec![],
            concurrent_tool_calls: None,
            user_decisions: vec![],
            conversation: if pn_turns.is_empty() {
                None
            } else {
                Some(pn_turns)
            },
        };

        staging::upsert_receipt_in(&receipt, &ctx.cwd);
    }
}

/// Handle SubagentStart — a Task tool subagent has been spawned.
/// Creates a SubagentActivity entry on the current prompt's receipt.
fn handle_subagent_start(input: &HookInput) {
    let cwd = input.cwd.clone().unwrap_or_else(|| ".".to_string());
    let session_id = match input.session_id.as_ref() {
        Some(s) => s.clone(),
        None => return,
    };

    let existing = staging::read_staging_in(Path::new(&cwd));
    // Find the most recent receipt for this session
    let last_pn = existing
        .receipts
        .iter()
        .filter(|r| r.session_id == session_id)
        .filter_map(|r| r.prompt_number)
        .max();

    let pn = match last_pn {
        Some(pn) => pn,
        None => return, // No receipt for this session yet
    };

    let activity = SubagentActivity {
        agent_id: input.agent_id.clone(),
        agent_type: input.agent_type.clone(),
        description: None, // Not provided in SubagentStart payload
        status: "started".to_string(),
        started_at: Some(Utc::now()),
        completed_at: None,
        tools_used: vec![],
    };

    // Read current receipt and add the activity
    let mut data = staging::read_staging_in(Path::new(&cwd));
    if let Some(receipt) = data
        .receipts
        .iter_mut()
        .find(|r| r.session_id == session_id && r.prompt_number == Some(pn))
    {
        // Don't add duplicate entries for the same agent_id
        if let Some(ref aid) = activity.agent_id {
            if receipt
                .subagent_activities
                .iter()
                .any(|a| a.agent_id.as_deref() == Some(aid))
            {
                return;
            }
        }
        receipt.subagent_activities.push(activity);
        staging::write_staging_data_in(&data, &cwd);
    }
}

/// Handle SubagentStop — a Task tool subagent has completed.
/// Updates the matching SubagentActivity to "completed" and extracts tools used.
fn handle_subagent_stop(input: &HookInput) {
    let cwd = input.cwd.clone().unwrap_or_else(|| ".".to_string());
    let session_id = match input.session_id.as_ref() {
        Some(s) => s.clone(),
        None => return,
    };

    // Parse the subagent's transcript if available to extract tools used
    let subagent_tools: Vec<String> = input
        .agent_transcript_path
        .as_ref()
        .and_then(|path| transcript::parse_claude_jsonl(path).ok())
        .map(|parsed| transcript::extract_tools_used(&parsed.transcript))
        .unwrap_or_default();

    let mut data = staging::read_staging_in(Path::new(&cwd));
    // Find the receipt for this session and update the matching activity
    for receipt in data
        .receipts
        .iter_mut()
        .filter(|r| r.session_id == session_id)
    {
        let found = if let Some(ref aid) = input.agent_id {
            receipt
                .subagent_activities
                .iter_mut()
                .find(|a| a.agent_id.as_deref() == Some(aid))
        } else {
            // No agent_id — update the last "started" activity
            receipt
                .subagent_activities
                .iter_mut()
                .rev()
                .find(|a| a.status == "started")
        };

        if let Some(activity) = found {
            activity.status = "completed".to_string();
            activity.completed_at = Some(Utc::now());
            if !subagent_tools.is_empty() {
                activity.tools_used = subagent_tools;
            }
            staging::write_staging_data_in(&data, &cwd);
            return;
        }
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
        assert!(input.last_assistant_message.is_none());
        assert!(input.session_id.is_none());
    }

    #[test]
    fn test_continuation_marker_detection() {
        let prompt = "This session is being continued from a previous conversation that ran out of context. The summary below covers the earlier portion.";
        assert!(prompt.starts_with(CONTINUATION_MARKER));
    }

    #[test]
    fn test_normal_prompt_not_continuation() {
        let prompt = "Fix the failing tests in checkpoint.rs";
        assert!(!prompt.starts_with(CONTINUATION_MARKER));
    }

    #[test]
    fn test_parse_hook_input_subagent_start() {
        let json = r#"{"session_id":"abc-123","hook_event_name":"SubagentStart","cwd":"/proj","agent_id":"agent-1","agent_type":"Explore"}"#;
        let input = parse_hook_input(json);
        assert_eq!(input.hook_event_name.as_deref(), Some("SubagentStart"));
        assert_eq!(input.agent_id.as_deref(), Some("agent-1"));
        assert_eq!(input.agent_type.as_deref(), Some("Explore"));
    }

    #[test]
    fn test_parse_hook_input_parent_session_id() {
        let json = r#"{"session_id":"new-session","parent_session_id":"old-session","hook_event_name":"UserPromptSubmit","prompt":"continue..."}"#;
        let input = parse_hook_input(json);
        assert_eq!(input.parent_session_id.as_deref(), Some("old-session"));
    }

    #[test]
    fn test_parse_hook_input_ask_user_question() {
        let json = r#"{"session_id":"s1","hook_event_name":"PostToolUse","tool_name":"AskUserQuestion","tool_input":{"questions":[{"question":"Which approach?","header":"Approach","options":[{"label":"CSS variables"},{"label":"Theme context"}],"multiSelect":false}]}}"#;
        let input = parse_hook_input(json);
        assert_eq!(input.tool_name.as_deref(), Some("AskUserQuestion"));
        assert!(input.tool_input.is_some());
        let ti = input.tool_input.unwrap();
        let qs = ti.get("questions").unwrap().as_array().unwrap();
        assert_eq!(qs.len(), 1);
        assert_eq!(
            qs[0].get("question").unwrap().as_str().unwrap(),
            "Which approach?"
        );
    }

    #[test]
    fn test_parse_hook_input_stop_event() {
        let json = r#"{"session_id":"abc-123","hook_event_name":"Stop","transcript_path":"/tmp/t.jsonl","cwd":"/proj","last_assistant_message":"I fixed the bug by updating the parser to handle edge cases."}"#;
        let input = parse_hook_input(json);
        assert_eq!(input.hook_event_name.as_deref(), Some("Stop"));
        assert_eq!(input.session_id.as_deref(), Some("abc-123"));
        assert_eq!(
            input.last_assistant_message.as_deref(),
            Some("I fixed the bug by updating the parser to handle edge cases.")
        );
    }
}
