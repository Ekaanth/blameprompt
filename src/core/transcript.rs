use chrono::{DateTime, Utc};
use std::collections::HashSet;
use std::path::Path;

#[derive(Debug, Clone)]
pub enum Message {
    User {
        text: String,
    },
    Assistant {
        text: String,
        /// The model that generated this specific response (e.g. "claude-opus-4-6").
        /// Stored per-message so model switches mid-session are tracked correctly.
        model: Option<String>,
        /// Token usage from this specific API call.
        /// Set only on the first text message per JSONL assistant entry to avoid double-counting.
        usage: Option<TokenUsage>,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
}

#[derive(Debug)]
pub struct Transcript {
    pub messages: Vec<Message>,
}

/// Aggregated token usage from all assistant messages in the transcript.
#[derive(Debug, Clone, Default)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
}

#[derive(Debug)]
pub struct TranscriptParseResult {
    pub transcript: Transcript,
    pub model: Option<String>,
    pub session_id: String,
    pub files_modified: Vec<String>,
    pub session_start: Option<DateTime<Utc>>,
    pub session_end: Option<DateTime<Utc>>,
    pub session_duration_secs: Option<u64>,
    pub avg_response_time_secs: Option<f64>,
    /// Timestamps of each user prompt message (1-indexed: index 0 = prompt 1).
    /// Used for setting accurate per-receipt timestamps instead of Utc::now().
    pub user_prompt_timestamps: Vec<DateTime<Utc>>,
}

pub fn parse_claude_jsonl(transcript_path: &str) -> Result<TranscriptParseResult, String> {
    let path = Path::new(transcript_path);

    // Extract session UUID from filename
    let session_id = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();

    let content =
        std::fs::read_to_string(path).map_err(|e| format!("Cannot read transcript: {}", e))?;

    let mut messages = Vec::new();
    let mut model: Option<String> = None;
    let mut files_modified = Vec::new();
    // Track AskUserQuestion tool_use IDs so we can capture the user's selected answers.
    let mut ask_question_ids: HashSet<String> = HashSet::new();

    let mut first_timestamp: Option<DateTime<Utc>> = None;
    let mut last_timestamp: Option<DateTime<Utc>> = None;
    let mut response_times: Vec<f64> = Vec::new();
    let mut last_user_timestamp: Option<DateTime<Utc>> = None;
    let mut user_prompt_timestamps: Vec<DateTime<Utc>> = Vec::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let entry: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue, // Skip malformed lines
        };

        // Track timing
        if let Some(ts_str) = entry.get("timestamp").and_then(|v| v.as_str()) {
            if let Ok(ts) = ts_str.parse::<DateTime<Utc>>() {
                if first_timestamp.is_none() {
                    first_timestamp = Some(ts);
                }
                last_timestamp = Some(ts);

                match entry.get("type").and_then(|v| v.as_str()) {
                    Some("user") => {
                        last_user_timestamp = Some(ts);
                    }
                    Some("assistant") => {
                        if let Some(user_ts) = last_user_timestamp {
                            let delta = (ts - user_ts).num_milliseconds() as f64 / 1000.0;
                            if delta > 0.0 && delta < 600.0 {
                                response_times.push(delta);
                            }
                            last_user_timestamp = None;
                        }
                    }
                    _ => {}
                }
            }
        }

        match entry.get("type").and_then(|v| v.as_str()) {
            Some("user") => {
                let content_val = entry.get("message").and_then(|m| m.get("content"));
                // Claude Code transcripts use either a plain string or an array of content
                // blocks (e.g. [{"type":"text","text":"..."},{"type":"tool_result",...}]).
                // Only "text" blocks represent the human's actual typed message; tool_result
                // blocks are feedback from tool calls and should be ignored — EXCEPT for
                // tool_result blocks that are answers to AskUserQuestion, which we capture
                // with a `[choice] ` prefix so they appear in conversation turns.
                let mut text = if let Some(s) = content_val.and_then(|c| c.as_str()) {
                    s.to_string()
                } else if let Some(arr) = content_val.and_then(|c| c.as_array()) {
                    let parts: Vec<&str> = arr
                        .iter()
                        .filter_map(|item| {
                            if item.get("type").and_then(|t| t.as_str()) == Some("text") {
                                item.get("text").and_then(|t| t.as_str())
                            } else {
                                None
                            }
                        })
                        .collect();
                    parts.join("\n")
                } else {
                    String::new()
                };

                // Capture AskUserQuestion answers from tool_result blocks.
                if let Some(arr) = content_val.and_then(|c| c.as_array()) {
                    let answers: Vec<String> = arr
                        .iter()
                        .filter_map(|item| {
                            if item.get("type").and_then(|t| t.as_str()) != Some("tool_result") {
                                return None;
                            }
                            let tuid = item.get("tool_use_id").and_then(|v| v.as_str())?;
                            if !ask_question_ids.contains(tuid) {
                                return None;
                            }
                            item.get("content")
                                .map(extract_tool_result_text)
                                .filter(|s| !s.is_empty())
                        })
                        .collect();
                    if !answers.is_empty() {
                        let choice = format!("[choice] {}", answers.join("; "));
                        if text.is_empty() {
                            text = choice;
                        } else {
                            text.push('\n');
                            text.push_str(&choice);
                        }
                    }
                }

                // Capture per-prompt timestamp for real prompts (matches count_user_prompts logic).
                if is_real_prompt(&text) {
                    if let Some(ts_str) = entry.get("timestamp").and_then(|v| v.as_str()) {
                        if let Ok(ts) = ts_str.parse::<DateTime<Utc>>() {
                            user_prompt_timestamps.push(ts);
                        }
                    }
                }
                messages.push(Message::User { text });
            }
            Some("assistant") => {
                let msg = entry.get("message");

                // Extract model from this assistant message (always update — last seen wins,
                // so model switches during a session are reflected in the global fallback).
                let entry_model: Option<String> = msg
                    .and_then(|m| m.get("model"))
                    .and_then(|v| v.as_str())
                    .map(String::from);
                if entry_model.is_some() {
                    model = entry_model.clone();
                }

                // Extract actual token usage from the usage field.
                // Claude Code writes usage data on each assistant message with real API token counts.
                // Store per-entry usage so we can attribute costs to individual prompts later.
                let mut entry_usage: Option<TokenUsage> = None;
                if let Some(usage) = msg.and_then(|m| m.get("usage")) {
                    let it = usage
                        .get("input_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    let ot = usage
                        .get("output_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    let cr = usage
                        .get("cache_read_input_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    let cc = usage
                        .get("cache_creation_input_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    if it > 0 || ot > 0 {
                        entry_usage = Some(TokenUsage {
                            input_tokens: it,
                            output_tokens: ot,
                            cache_read_tokens: cr,
                            cache_creation_tokens: cc,
                        });
                    }
                }

                // Track whether usage has been assigned to the first text message in this entry.
                // Only the first Message::Assistant gets the usage to avoid double-counting.
                let mut usage_assigned = false;

                // Parse content array
                if let Some(content_arr) = msg
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.as_array())
                {
                    for item in content_arr {
                        match item.get("type").and_then(|v| v.as_str()) {
                            Some("text") => {
                                let text = item
                                    .get("text")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let msg_usage = if !usage_assigned {
                                    usage_assigned = true;
                                    entry_usage.clone()
                                } else {
                                    None
                                };
                                messages.push(Message::Assistant {
                                    text,
                                    model: entry_model.clone(),
                                    usage: msg_usage,
                                });
                            }
                            Some("tool_use") => {
                                let name = item
                                    .get("name")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let input = item
                                    .get("input")
                                    .cloned()
                                    .unwrap_or(serde_json::Value::Null);

                                // Track AskUserQuestion IDs so we can capture the user's
                                // selected answer from the following tool_result block.
                                if name == "AskUserQuestion" {
                                    if let Some(id) = item.get("id").and_then(|v| v.as_str()) {
                                        ask_question_ids.insert(id.to_string());
                                    }
                                }

                                // Track modified files — handle both single file_path (Write/Edit)
                                // and MultiEdit's edits array (edits[].file_path).
                                if let Some(fp) = input.get("file_path").and_then(|v| v.as_str()) {
                                    if !files_modified.contains(&fp.to_string()) {
                                        files_modified.push(fp.to_string());
                                    }
                                } else if let Some(edits) =
                                    input.get("edits").and_then(|e| e.as_array())
                                {
                                    for edit in edits {
                                        if let Some(fp) =
                                            edit.get("file_path").and_then(|v| v.as_str())
                                        {
                                            if !files_modified.contains(&fp.to_string()) {
                                                files_modified.push(fp.to_string());
                                            }
                                        }
                                    }
                                }

                                let id = item
                                    .get("id")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                messages.push(Message::ToolUse { id, name, input });
                            }
                            _ => {}
                        }
                    }
                } else if let Some(content_str) =
                    msg.and_then(|m| m.get("content")).and_then(|c| c.as_str())
                {
                    // Content is a plain string
                    messages.push(Message::Assistant {
                        text: content_str.to_string(),
                        model: entry_model,
                        usage: entry_usage,
                    });
                }
            }
            _ => {}
        }
    }

    let session_duration_secs = match (first_timestamp, last_timestamp) {
        (Some(start), Some(end)) => Some((end - start).num_seconds().max(0) as u64),
        _ => None,
    };

    let avg_response_time_secs = if !response_times.is_empty() {
        Some(response_times.iter().sum::<f64>() / response_times.len() as f64)
    } else {
        None
    };

    Ok(TranscriptParseResult {
        transcript: Transcript { messages },
        model,
        session_id,
        files_modified,
        session_start: first_timestamp,
        session_end: last_timestamp,
        session_duration_secs,
        avg_response_time_secs,
        user_prompt_timestamps,
    })
}

/// Returns true if a user message text is a real typed prompt (not a UI option selection).
/// AskUserQuestion answers are stored with a `[choice] ` prefix so they appear in
/// conversation turns but are not counted as new prompts.
fn is_real_prompt(text: &str) -> bool {
    !text.is_empty() && !text.starts_with("[choice] ")
}

/// Extract displayable text from a tool_result `content` field.
/// Handles plain strings, arrays of text blocks, and JSON objects.
fn extract_tool_result_text(content: &serde_json::Value) -> String {
    if let Some(s) = content.as_str() {
        return s.to_string();
    }
    if let Some(arr) = content.as_array() {
        let parts: Vec<&str> = arr
            .iter()
            .filter_map(|item| {
                if item.get("type").and_then(|t| t.as_str()) == Some("text") {
                    item.get("text").and_then(|v| v.as_str())
                } else {
                    None
                }
            })
            .collect();
        return parts.join("; ");
    }
    String::new()
}

pub fn first_user_prompt(transcript: &Transcript) -> Option<String> {
    for msg in &transcript.messages {
        if let Message::User { text, .. } = msg {
            if is_real_prompt(text) {
                let truncated: String = text.chars().take(200).collect();
                return Some(truncated);
            }
        }
    }
    None
}

/// Returns the Nth (1-based) non-empty user prompt in the transcript.
pub fn nth_user_prompt(transcript: &Transcript, n: u32) -> Option<String> {
    let mut count = 0u32;
    for msg in &transcript.messages {
        if let Message::User { text, .. } = msg {
            if is_real_prompt(text) {
                count += 1;
                if count == n {
                    let truncated: String = text.chars().take(200).collect();
                    return Some(truncated);
                }
            }
        }
    }
    None
}

/// Count the number of user prompts in the transcript.
pub fn count_user_prompts(transcript: &Transcript) -> u32 {
    transcript
        .messages
        .iter()
        .filter(|m| matches!(m, Message::User { text, .. } if is_real_prompt(text)))
        .count() as u32
}

/// Maximum conversation turns stored per receipt.
const MAX_CONVERSATION_TURNS: usize = 50;

/// Check if an assistant message has substance (not just a short transition).
fn is_substantive_message(text: &str) -> bool {
    if text.len() < 50 {
        let lower = text.to_lowercase();
        let transitional = [
            "let me ",
            "now let me",
            "now build",
            "now update",
            "now create",
            "now I need",
            "now add",
            "now fix",
            "now run",
            "now check",
            "now install",
            "now rebuild",
            "now rewrite",
            "now verify",
        ];
        if transitional.iter().any(|p| lower.starts_with(p)) {
            return false;
        }
        if lower.ends_with(':') {
            return false;
        }
    }
    true
}

/// Strip an absolute path to just the filename.
fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Produce a concise one-line summary of a tool invocation.
/// Examples: `Bash(command: "git status")`, `Write(file: "main.rs")`
fn tool_summary(name: &str, input: &serde_json::Value) -> String {
    let arg = match name {
        "AskUserQuestion" => input
            .get("questions")
            .and_then(|q| q.as_array())
            .and_then(|qs| qs.first())
            .and_then(|q| q.get("question"))
            .and_then(|v| v.as_str())
            .map(|q| {
                let truncated: String = q.chars().take(60).collect();
                format!("\"{}\"", truncated)
            }),
        "Bash" => input.get("command").and_then(|v| v.as_str()).map(|s| {
            let truncated: String = s.chars().take(80).collect();
            if s.chars().count() > 80 {
                format!("command: \"{}...\"", truncated)
            } else {
                format!("command: \"{}\"", truncated)
            }
        }),
        "Write" | "Edit" | "Read" => input
            .get("file_path")
            .and_then(|v| v.as_str())
            .map(|s| format!("file: \"{}\"", basename(s))),
        "MultiEdit" => {
            // MultiEdit has an edits array; show all affected filenames.
            if let Some(edits) = input.get("edits").and_then(|e| e.as_array()) {
                let names: Vec<&str> = edits
                    .iter()
                    .filter_map(|e| e.get("file_path").and_then(|v| v.as_str()))
                    .map(basename)
                    .collect();
                if !names.is_empty() {
                    return format!("MultiEdit(files: \"{}\")", names.join(", "));
                }
            }
            // Fallback to single file_path if edits is absent
            input
                .get("file_path")
                .and_then(|v| v.as_str())
                .map(|s| format!("file: \"{}\"", basename(s)))
        }
        "Grep" => input.get("pattern").and_then(|v| v.as_str()).map(|s| {
            let truncated: String = s.chars().take(60).collect();
            format!("pattern: \"{}\"", truncated)
        }),
        "Glob" => input
            .get("pattern")
            .and_then(|v| v.as_str())
            .map(|s| format!("pattern: \"{}\"", s)),
        _ => input
            .get("file_path")
            .and_then(|v| v.as_str())
            .map(|s| format!("file: \"{}\"", basename(s))),
    };

    match arg {
        Some(a) => format!("{}({})", name, a),
        None => format!("{}()", name),
    }
}

/// Extract structured conversation turns for storage in receipts.
/// Produces a concise history: keeps user prompts + substantive AI responses,
/// collapses consecutive tool calls into single summary turns.
pub fn extract_conversation_turns(
    transcript: &Transcript,
    max_turn_length: usize,
    redact_fn: &dyn Fn(&str) -> String,
) -> Vec<crate::core::receipt::ConversationTurn> {
    use crate::core::receipt::ConversationTurn;

    let mut turns = Vec::new();
    let mut turn_idx = 0u32;
    let messages = &transcript.messages;
    let mut i = 0;

    while i < messages.len() {
        match &messages[i] {
            Message::User { text, .. } => {
                if !text.is_empty() {
                    let truncated: String = text.chars().take(max_turn_length).collect();
                    // AskUserQuestion answers are prefixed with "[choice] " — show them
                    // as a distinct role so it's clear these are UI selections, not prompts.
                    let role = if text.starts_with("[choice] ") {
                        "choice".to_string()
                    } else {
                        "user".to_string()
                    };
                    turns.push(ConversationTurn {
                        turn: turn_idx,
                        role,
                        content: redact_fn(&truncated),
                        tool_name: None,
                        files_touched: None,
                    });
                    turn_idx += 1;
                }
                i += 1;
            }
            Message::Assistant { text, .. } => {
                if !text.is_empty() && is_substantive_message(text) {
                    let truncated: String = text.chars().take(max_turn_length).collect();
                    turns.push(ConversationTurn {
                        turn: turn_idx,
                        role: "assistant".to_string(),
                        content: redact_fn(&truncated),
                        tool_name: None,
                        files_touched: None,
                    });
                    turn_idx += 1;
                }
                i += 1;
            }
            Message::ToolUse { .. } => {
                // Collapse consecutive tool calls into one summary turn
                let mut tool_summaries: Vec<String> = Vec::new();
                let mut all_files: Vec<String> = Vec::new();
                let mut tool_names_seen: Vec<String> = Vec::new();

                while i < messages.len() {
                    if let Message::ToolUse { name, input, .. } = &messages[i] {
                        let summary = tool_summary(name, input);
                        if !tool_summaries.contains(&summary) {
                            tool_summaries.push(summary);
                        }
                        if !tool_names_seen.contains(name) {
                            tool_names_seen.push(name.clone());
                        }
                        if let Some(fp) = input.get("file_path").and_then(|v| v.as_str()) {
                            let display_path = fp.rsplit('/').next().unwrap_or(fp).to_string();
                            if !all_files.contains(&display_path) {
                                all_files.push(display_path);
                            }
                        } else if let Some(edits) = input.get("edits").and_then(|e| e.as_array()) {
                            for edit in edits {
                                if let Some(fp) = edit.get("file_path").and_then(|v| v.as_str()) {
                                    let display_path =
                                        fp.rsplit('/').next().unwrap_or(fp).to_string();
                                    if !all_files.contains(&display_path) {
                                        all_files.push(display_path);
                                    }
                                }
                            }
                        }
                        i += 1;
                    } else {
                        break;
                    }
                }

                let content = redact_fn(&tool_summaries.join(", "));

                turns.push(ConversationTurn {
                    turn: turn_idx,
                    role: "tool".to_string(),
                    content,
                    tool_name: if tool_names_seen.len() == 1 {
                        Some(tool_names_seen[0].clone())
                    } else {
                        None
                    },
                    files_touched: if all_files.is_empty() {
                        None
                    } else {
                        Some(all_files)
                    },
                });
                turn_idx += 1;
            }
        }
    }

    // Cap turns: keep first 5 (initial context) + last N (most recent work)
    if turns.len() > MAX_CONVERSATION_TURNS {
        let mut capped = turns[..5].to_vec();
        capped.push(ConversationTurn {
            turn: 5,
            role: "assistant".to_string(),
            content: format!(
                "... ({} turns omitted) ...",
                turns.len() - MAX_CONVERSATION_TURNS
            ),
            tool_name: None,
            files_touched: None,
        });
        capped.extend_from_slice(&turns[turns.len() - (MAX_CONVERSATION_TURNS - 6)..]);
        // Renumber
        for (idx, t) in capped.iter_mut().enumerate() {
            t.turn = idx as u32;
        }
        turns = capped;
    }

    turns
}

/// Extract unique tool names used in the transcript.
/// Returns sorted list of tool names like ["Bash", "Edit", "Grep", "Write"].
pub fn extract_tools_used(transcript: &Transcript) -> Vec<String> {
    let mut tools: Vec<String> = Vec::new();
    for msg in &transcript.messages {
        if let Message::ToolUse { name, .. } = msg {
            // Exclude MCP tools (they're tracked separately) and Task tool (tracked as agents)
            if !name.starts_with("mcp__") && name != "Task" && !tools.contains(name) {
                tools.push(name.clone());
            }
        }
    }
    tools.sort();
    tools
}

/// Extract MCP server names from tool calls matching the `mcp__<server>__<tool>` pattern.
/// Returns sorted unique server names like ["filesystem", "github"].
pub fn extract_mcp_servers(transcript: &Transcript) -> Vec<String> {
    let mut servers: Vec<String> = Vec::new();
    for msg in &transcript.messages {
        if let Message::ToolUse { name, .. } = msg {
            // MCP tools follow the pattern mcp__<server>__<tool>
            if let Some(rest) = name.strip_prefix("mcp__") {
                if let Some(server) = rest.split("__").next() {
                    if !server.is_empty() && !servers.contains(&server.to_string()) {
                        servers.push(server.to_string());
                    }
                }
            }
        }
    }
    servers.sort();
    servers
}

/// Extract sub-agent descriptions from Task tool calls.
/// Returns list of agent descriptions like ["Explore codebase", "Run tests"].
pub fn extract_agents_spawned(transcript: &Transcript) -> Vec<String> {
    let mut agents: Vec<String> = Vec::new();
    for msg in &transcript.messages {
        if let Message::ToolUse { name, input, .. } = msg {
            if name == "Task" {
                // Task tool has a "description" field and optionally "subagent_type"
                let desc = input
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown agent");
                let agent_type = input
                    .get("subagent_type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let label = if agent_type.is_empty() {
                    desc.to_string()
                } else {
                    format!("{} ({})", desc, agent_type)
                };
                if !agents.contains(&label) {
                    agents.push(label);
                }
            }
        }
    }
    agents
}

/// Extract tools/MCP servers/agents scoped to a specific prompt (1-indexed).
/// Uses `prompt_message_slice` to avoid attributing full-session data to a single prompt.
pub fn extract_tools_for_prompt(transcript: &Transcript, prompt_number: u32) -> Vec<String> {
    let slice = prompt_message_slice(&transcript.messages, prompt_number);
    if slice.is_empty() {
        return vec![];
    }
    let sub = Transcript {
        messages: slice.to_vec(),
    };
    extract_tools_used(&sub)
}

pub fn extract_mcps_for_prompt(transcript: &Transcript, prompt_number: u32) -> Vec<String> {
    let slice = prompt_message_slice(&transcript.messages, prompt_number);
    if slice.is_empty() {
        return vec![];
    }
    let sub = Transcript {
        messages: slice.to_vec(),
    };
    extract_mcp_servers(&sub)
}

/// Sum token usage from only the assistant messages within the Nth prompt's slice.
/// Returns None if no usage data exists in that slice (e.g. JSONL lacks token counts).
pub fn token_usage_for_prompt(transcript: &Transcript, prompt_number: u32) -> Option<TokenUsage> {
    let slice = prompt_message_slice(&transcript.messages, prompt_number);
    let mut total = TokenUsage::default();
    let mut found = false;
    for msg in slice {
        if let Message::Assistant { usage: Some(u), .. } = msg {
            total.input_tokens += u.input_tokens;
            total.output_tokens += u.output_tokens;
            total.cache_read_tokens += u.cache_read_tokens;
            total.cache_creation_tokens += u.cache_creation_tokens;
            found = true;
        }
    }
    if found {
        Some(total)
    } else {
        None
    }
}

pub fn extract_agents_for_prompt(transcript: &Transcript, prompt_number: u32) -> Vec<String> {
    let slice = prompt_message_slice(&transcript.messages, prompt_number);
    if slice.is_empty() {
        return vec![];
    }
    let sub = Transcript {
        messages: slice.to_vec(),
    };
    extract_agents_spawned(&sub)
}

/// Extract file paths modified by Write/Edit/MultiEdit/NotebookEdit tools in a specific prompt.
/// Lightweight alternative to full conversation extraction — only returns file paths.
pub fn files_for_prompt(transcript: &Transcript, prompt_number: u32) -> Vec<String> {
    let slice = prompt_message_slice(&transcript.messages, prompt_number);
    let mut files = Vec::new();
    for msg in slice {
        if let Message::ToolUse { input, name, .. } = msg {
            match name.as_str() {
                "Write" | "Edit" | "NotebookEdit" => {
                    if let Some(fp) = input.get("file_path").and_then(|v| v.as_str()) {
                        if !files.contains(&fp.to_string()) {
                            files.push(fp.to_string());
                        }
                    }
                }
                "MultiEdit" => {
                    if let Some(edits) = input.get("edits").and_then(|e| e.as_array()) {
                        for edit in edits {
                            if let Some(fp) = edit.get("file_path").and_then(|v| v.as_str()) {
                                if !files.contains(&fp.to_string()) {
                                    files.push(fp.to_string());
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    files
}

/// Count the maximum number of tool calls that appear consecutively in the transcript,
/// approximating the max parallel tool use within a single assistant turn.
pub fn count_concurrent_tools(transcript: &Transcript) -> u32 {
    let mut max_concurrent: u32 = 0;
    let mut current_streak: u32 = 0;

    for msg in &transcript.messages {
        match msg {
            Message::ToolUse { .. } => {
                current_streak += 1;
                max_concurrent = max_concurrent.max(current_streak);
            }
            _ => {
                current_streak = 0;
            }
        }
    }
    max_concurrent
}

/// Extract structured UserDecision data from AskUserQuestion tool_use/tool_result pairs.
///
/// Walks the transcript messages in order. When an AskUserQuestion ToolUse is found,
/// it captures the questions/options from the input along with the tool_use id.
/// Then matches answers from `[choice]`-prefixed User messages by option label matching.
pub fn extract_user_decisions(transcript: &Transcript) -> Vec<crate::core::receipt::UserDecision> {
    use crate::core::receipt::{DecisionOption, UserDecision};

    let mut decisions: Vec<UserDecision> = Vec::new();

    // Phase 1: Collect all AskUserQuestion tool_use entries with their questions.
    for msg in &transcript.messages {
        if let Message::ToolUse { id, name, input } = msg {
            if name != "AskUserQuestion" {
                continue;
            }
            if let Some(questions) = input.get("questions").and_then(|q| q.as_array()) {
                for q in questions {
                    let question = match q.get("question").and_then(|v| v.as_str()) {
                        Some(q) => q.to_string(),
                        None => continue,
                    };
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
                                        label: opt
                                            .get("label")
                                            .and_then(|v| v.as_str())?
                                            .to_string(),
                                        selected: false,
                                    })
                                })
                                .collect()
                        })
                        .unwrap_or_default();

                    decisions.push(UserDecision {
                        tool_use_id: id.clone(),
                        question,
                        header,
                        options,
                        multi_select,
                        answer: None,
                    });
                }
            }
        }
    }

    if decisions.is_empty() {
        return vec![];
    }

    // Phase 2: Find answers in [choice]-prefixed User messages.
    for msg in &transcript.messages {
        if let Message::User { text } = msg {
            if let Some(answer_text) = text.strip_prefix("[choice] ") {
                // Match answer to pending decisions by checking option labels.
                for decision in &mut decisions {
                    if decision.answer.is_some() {
                        continue;
                    }
                    let answer_parts: Vec<&str> = answer_text.split("; ").collect();
                    let mut matched = false;
                    for part in &answer_parts {
                        for opt in &mut decision.options {
                            if opt.label == *part {
                                opt.selected = true;
                                matched = true;
                            }
                        }
                    }
                    if matched {
                        decision.answer = Some(answer_text.to_string());
                    }
                }
            }
        }
    }

    decisions
}

/// Return the slice of messages that belong exclusively to the Nth user prompt (1-indexed).
///
/// Spans from the Nth non-empty user message up to (but not including) the (N+1)th
/// non-empty user message.  If N is out of range, returns an empty slice.
fn prompt_message_slice(messages: &[Message], prompt_number: u32) -> &[Message] {
    let mut count = 0u32;
    let mut start: Option<usize> = None;

    for (i, msg) in messages.iter().enumerate() {
        if let Message::User { text, .. } = msg {
            if is_real_prompt(text) {
                count += 1;
                if count == prompt_number {
                    start = Some(i);
                } else if count == prompt_number + 1 {
                    if let Some(s) = start {
                        return &messages[s..i];
                    }
                }
            }
        }
    }

    // Prompt N is the last one — return from its start to end of transcript
    if let Some(s) = start {
        return &messages[s..];
    }

    &[]
}

/// Return the timestamp of the Nth user prompt (1-indexed) from the JSONL.
/// Returns None if the prompt number is out of range.
pub fn timestamp_for_prompt(
    result: &TranscriptParseResult,
    prompt_number: u32,
) -> Option<DateTime<Utc>> {
    if prompt_number == 0 {
        return None;
    }
    result
        .user_prompt_timestamps
        .get((prompt_number - 1) as usize)
        .copied()
}

/// Return the model used for the Nth user prompt (1-indexed).
///
/// Scans the prompt's message slice for the last assistant message that carries a
/// model identifier — this is the model that actually handled this prompt.
/// Falls back to `None` if the prompt has no assistant reply yet (e.g. at UserPromptSubmit time).
pub fn model_for_prompt(transcript: &Transcript, prompt_number: u32) -> Option<String> {
    let slice = prompt_message_slice(&transcript.messages, prompt_number);
    slice.iter().rev().find_map(|msg| {
        if let Message::Assistant { model, .. } = msg {
            model.clone()
        } else {
            None
        }
    })
}

/// Extract conversation turns for a **specific prompt** (1-indexed) only.
///
/// Use this instead of `extract_conversation_turns` when you want a receipt to contain
/// only the messages that belong to a single user prompt, not the entire session history.
pub fn extract_conversation_for_prompt(
    transcript: &Transcript,
    prompt_number: u32,
    max_turn_length: usize,
    redact_fn: &dyn Fn(&str) -> String,
) -> Vec<crate::core::receipt::ConversationTurn> {
    let slice = prompt_message_slice(&transcript.messages, prompt_number);
    if slice.is_empty() {
        return vec![];
    }
    // Reuse the same turn-extraction logic on just this prompt's slice
    let sub = Transcript {
        messages: slice.to_vec(),
    };
    extract_conversation_turns(&sub, max_turn_length, redact_fn)
}

pub fn full_conversation_text(transcript: &Transcript) -> String {
    let mut text = String::new();
    for msg in &transcript.messages {
        match msg {
            Message::User { text: t, .. } => {
                text.push_str("USER: ");
                text.push_str(t);
                text.push('\n');
            }
            Message::Assistant { text: t, .. } => {
                text.push_str("ASSISTANT: ");
                text.push_str(t);
                text.push('\n');
            }
            Message::ToolUse { name, .. } => {
                text.push_str("TOOL: ");
                text.push_str(name);
                text.push('\n');
            }
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_claude_jsonl() {
        let jsonl = r#"{"type":"user","message":{"content":"write hello world"},"timestamp":"2026-01-01T00:00:00Z"}
{"type":"assistant","message":{"model":"claude-sonnet-4-5-20250929","content":[{"type":"text","text":"Here's hello world"}]},"timestamp":"2026-01-01T00:00:01Z"}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Write","input":{"file_path":"main.rs","content":"fn main() { println!(\"hello\"); }"}}]},"timestamp":"2026-01-01T00:00:02Z"}"#;

        let tmp = std::env::temp_dir().join("test_transcript.jsonl");
        std::fs::write(&tmp, jsonl).unwrap();
        let result = parse_claude_jsonl(tmp.to_str().unwrap()).unwrap();

        assert_eq!(result.model, Some("claude-sonnet-4-5-20250929".to_string()));
        assert_eq!(result.files_modified, vec!["main.rs"]);
        assert_eq!(result.transcript.messages.len(), 3);
        assert!(result.session_start.is_some());
        assert!(result.session_end.is_some());
        assert_eq!(result.session_duration_secs, Some(2));
        assert!(result.avg_response_time_secs.is_some());
        let avg = result.avg_response_time_secs.unwrap();
        assert!((avg - 1.0).abs() < 0.01);
        std::fs::remove_file(tmp).ok();
    }

    #[test]
    fn test_parse_user_message_array_content() {
        // Claude Code transcripts store user messages as arrays of content blocks.
        // Text blocks are the human's typed message; tool_result blocks are skipped.
        let jsonl = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"fix the bug"},{"type":"tool_result","tool_use_id":"t1","content":"ok"}]},"timestamp":"2026-01-01T00:00:00Z"}
{"type":"assistant","message":{"model":"claude-sonnet-4-6","content":[{"type":"text","text":"Sure"}]},"timestamp":"2026-01-01T00:00:01Z"}"#;

        let tmp = std::env::temp_dir().join("test_array_content.jsonl");
        std::fs::write(&tmp, jsonl).unwrap();
        let result = parse_claude_jsonl(tmp.to_str().unwrap()).unwrap();

        // The user message with array content should be parsed correctly
        assert_eq!(count_user_prompts(&result.transcript), 1);
        assert_eq!(
            nth_user_prompt(&result.transcript, 1),
            Some("fix the bug".to_string())
        );
        std::fs::remove_file(tmp).ok();
    }

    #[test]
    fn test_parse_empty_file() {
        let tmp = std::env::temp_dir().join("test_empty.jsonl");
        std::fs::write(&tmp, "").unwrap();
        let result = parse_claude_jsonl(tmp.to_str().unwrap()).unwrap();
        assert!(result.transcript.messages.is_empty());
        assert!(result.model.is_none());
        std::fs::remove_file(tmp).ok();
    }

    #[test]
    fn test_parse_malformed_lines() {
        let jsonl =
            "not json\n{\"type\":\"user\",\"message\":{\"content\":\"hello\"}}\nalso not json\n";
        let tmp = std::env::temp_dir().join("test_malformed.jsonl");
        std::fs::write(&tmp, jsonl).unwrap();
        let result = parse_claude_jsonl(tmp.to_str().unwrap()).unwrap();
        assert_eq!(result.transcript.messages.len(), 1);
        std::fs::remove_file(tmp).ok();
    }

    #[test]
    fn test_model_for_prompt_tracks_per_prompt_model() {
        // Simulates a session where the user switches from sonnet to opus after prompt 1.
        // prompt 1 → assistant replies with sonnet
        // prompt 2 → assistant replies with opus
        let transcript = Transcript {
            messages: vec![
                Message::User {
                    text: "first prompt".to_string(),
                },
                Message::Assistant {
                    text: "response 1".to_string(),
                    model: Some("claude-sonnet-4-6".to_string()),
                    usage: None,
                },
                Message::User {
                    text: "second prompt".to_string(),
                },
                Message::Assistant {
                    text: "response 2".to_string(),
                    model: Some("claude-opus-4-6".to_string()),
                    usage: None,
                },
            ],
        };

        assert_eq!(
            model_for_prompt(&transcript, 1),
            Some("claude-sonnet-4-6".to_string()),
            "prompt 1 should use sonnet"
        );
        assert_eq!(
            model_for_prompt(&transcript, 2),
            Some("claude-opus-4-6".to_string()),
            "prompt 2 should use opus after model switch"
        );
        // Out-of-range prompt → None
        assert_eq!(model_for_prompt(&transcript, 3), None);
    }

    #[test]
    fn test_token_usage_for_prompt() {
        let transcript = Transcript {
            messages: vec![
                Message::User {
                    text: "first prompt".to_string(),
                },
                Message::Assistant {
                    text: "response 1".to_string(),
                    model: None,
                    usage: Some(TokenUsage {
                        input_tokens: 1000,
                        output_tokens: 500,
                        cache_read_tokens: 200,
                        cache_creation_tokens: 50,
                    }),
                },
                Message::User {
                    text: "second prompt".to_string(),
                },
                Message::Assistant {
                    text: "response 2a".to_string(),
                    model: None,
                    usage: Some(TokenUsage {
                        input_tokens: 2000,
                        output_tokens: 800,
                        cache_read_tokens: 400,
                        cache_creation_tokens: 100,
                    }),
                },
                Message::Assistant {
                    text: "response 2b".to_string(),
                    model: None,
                    usage: Some(TokenUsage {
                        input_tokens: 500,
                        output_tokens: 200,
                        cache_read_tokens: 0,
                        cache_creation_tokens: 0,
                    }),
                },
            ],
        };

        // Prompt 1: only 1000/500 tokens
        let u1 = token_usage_for_prompt(&transcript, 1).unwrap();
        assert_eq!(u1.input_tokens, 1000);
        assert_eq!(u1.output_tokens, 500);
        assert_eq!(u1.cache_read_tokens, 200);

        // Prompt 2: sum of both assistant messages = 2500/1000
        let u2 = token_usage_for_prompt(&transcript, 2).unwrap();
        assert_eq!(u2.input_tokens, 2500);
        assert_eq!(u2.output_tokens, 1000);
        assert_eq!(u2.cache_read_tokens, 400);

        // Out-of-range → None
        assert!(token_usage_for_prompt(&transcript, 3).is_none());
    }

    #[test]
    fn test_model_from_jsonl_last_wins() {
        // Transcript where model changes between assistant messages.
        // The global `model` field on TranscriptParseResult should hold the LAST seen model.
        let jsonl = r#"{"type":"user","message":{"content":"prompt 1"},"timestamp":"2026-01-01T00:00:00Z"}
{"type":"assistant","message":{"model":"claude-sonnet-4-6","content":[{"type":"text","text":"response 1"}]},"timestamp":"2026-01-01T00:00:01Z"}
{"type":"user","message":{"content":"prompt 2"},"timestamp":"2026-01-01T00:00:02Z"}
{"type":"assistant","message":{"model":"claude-opus-4-6","content":[{"type":"text","text":"response 2"}]},"timestamp":"2026-01-01T00:00:03Z"}"#;

        let tmp = std::env::temp_dir().join("test_model_switch.jsonl");
        std::fs::write(&tmp, jsonl).unwrap();
        let result = parse_claude_jsonl(tmp.to_str().unwrap()).unwrap();
        std::fs::remove_file(tmp).ok();

        // Global model = last seen (opus)
        assert_eq!(result.model, Some("claude-opus-4-6".to_string()));
        // Per-prompt lookup is correct
        assert_eq!(
            model_for_prompt(&result.transcript, 1),
            Some("claude-sonnet-4-6".to_string())
        );
        assert_eq!(
            model_for_prompt(&result.transcript, 2),
            Some("claude-opus-4-6".to_string())
        );
    }

    #[test]
    fn test_ask_user_question_answer_captured() {
        // Simulate a session where Claude calls AskUserQuestion and the user picks an option.
        // The answer comes back as a tool_result block in the next user message.
        let jsonl = r#"{"type":"user","message":{"content":"implement dark mode"},"timestamp":"2026-01-01T00:00:00Z"}
{"type":"assistant","message":{"model":"claude-sonnet-4-6","content":[{"type":"text","text":"Which approach?"},{"type":"tool_use","id":"toolu_001","name":"AskUserQuestion","input":{"questions":[{"question":"Which approach?","header":"Approach","options":[{"label":"CSS variables"},{"label":"Theme context"}],"multiSelect":false}]}}]},"timestamp":"2026-01-01T00:00:01Z"}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_001","content":[{"type":"text","text":"CSS variables"}]}]},"timestamp":"2026-01-01T00:00:02Z"}
{"type":"assistant","message":{"model":"claude-sonnet-4-6","content":[{"type":"text","text":"I'll use CSS variables."}]},"timestamp":"2026-01-01T00:00:03Z"}"#;

        let tmp = std::env::temp_dir().join("test_ask_user.jsonl");
        std::fs::write(&tmp, jsonl).unwrap();
        let result = parse_claude_jsonl(tmp.to_str().unwrap()).unwrap();
        std::fs::remove_file(tmp).ok();

        // Only the real typed prompt counts — the [choice] answer does not
        assert_eq!(count_user_prompts(&result.transcript), 1);
        assert_eq!(
            first_user_prompt(&result.transcript),
            Some("implement dark mode".to_string())
        );

        // The [choice] answer IS present in messages
        let choice_msg = result
            .transcript
            .messages
            .iter()
            .find(|m| matches!(m, Message::User { text, .. } if text.starts_with("[choice]")));
        assert!(
            choice_msg.is_some(),
            "AskUserQuestion answer should be stored"
        );
        if let Some(Message::User { text, .. }) = choice_msg {
            assert!(
                text.contains("CSS variables"),
                "Answer text should be captured: {}",
                text
            );
        }

        // The choice should appear as a "choice" role in conversation turns
        let turns = extract_conversation_turns(&result.transcript, 1000, &|s| s.to_string());
        let choice_turn = turns.iter().find(|t| t.role == "choice");
        assert!(
            choice_turn.is_some(),
            "choice turn should appear in conversation"
        );
        assert!(choice_turn.unwrap().content.contains("CSS variables"));
    }

    #[test]
    fn test_extract_user_decisions() {
        let messages = vec![
            Message::User {
                text: "implement dark mode".to_string(),
            },
            Message::ToolUse {
                id: "toolu_001".to_string(),
                name: "AskUserQuestion".to_string(),
                input: serde_json::json!({
                    "questions": [{
                        "question": "Which approach?",
                        "header": "Approach",
                        "options": [{"label": "CSS variables"}, {"label": "Theme context"}],
                        "multiSelect": false
                    }]
                }),
            },
            Message::User {
                text: "[choice] CSS variables".to_string(),
            },
            Message::Assistant {
                text: "I'll use CSS variables.".to_string(),
                model: None,
                usage: None,
            },
        ];
        let transcript = Transcript { messages };
        let decisions = extract_user_decisions(&transcript);
        assert_eq!(decisions.len(), 1);
        assert_eq!(decisions[0].tool_use_id, "toolu_001");
        assert_eq!(decisions[0].question, "Which approach?");
        assert_eq!(decisions[0].header, Some("Approach".to_string()));
        assert_eq!(decisions[0].options.len(), 2);
        assert!(decisions[0].options[0].selected); // CSS variables
        assert!(!decisions[0].options[1].selected); // Theme context
        assert_eq!(decisions[0].answer, Some("CSS variables".to_string()));
        assert!(!decisions[0].multi_select);
    }

    #[test]
    fn test_first_user_prompt() {
        let transcript = Transcript {
            messages: vec![Message::User {
                text: "write a function".to_string(),
            }],
        };
        assert_eq!(
            first_user_prompt(&transcript),
            Some("write a function".to_string())
        );
    }

    #[test]
    fn test_tool_summary_bash() {
        let input = serde_json::json!({"command": "git status"});
        assert_eq!(
            tool_summary("Bash", &input),
            r#"Bash(command: "git status")"#
        );
    }

    #[test]
    fn test_tool_summary_bash_truncation() {
        let long_cmd = "a".repeat(100);
        let input = serde_json::json!({"command": long_cmd});
        let result = tool_summary("Bash", &input);
        assert!(
            result.contains("...\""),
            "Missing truncation marker: {}",
            result
        );
        // 80 char command + wrapper "Bash(command: \"...\")" ~ 100 chars total
        assert!(!result.contains(&"a".repeat(100)), "Command not truncated");
    }

    #[test]
    fn test_tool_summary_write() {
        let input = serde_json::json!({"file_path": "/Users/someone/project/src/main.rs", "content": "..."});
        assert_eq!(tool_summary("Write", &input), r#"Write(file: "main.rs")"#);
    }

    #[test]
    fn test_tool_summary_grep() {
        let input = serde_json::json!({"pattern": "fn main"});
        assert_eq!(tool_summary("Grep", &input), r#"Grep(pattern: "fn main")"#);
    }

    #[test]
    fn test_tool_summary_unknown_with_file() {
        let input = serde_json::json!({"file_path": "/home/user/test.py"});
        assert_eq!(
            tool_summary("CustomTool", &input),
            r#"CustomTool(file: "test.py")"#
        );
    }

    #[test]
    fn test_tool_summary_unknown_no_args() {
        let input = serde_json::json!({"some_key": "value"});
        assert_eq!(tool_summary("CustomTool", &input), "CustomTool()");
    }

    #[test]
    fn test_extract_turns_with_tool_summaries() {
        let transcript = Transcript {
            messages: vec![
                Message::User {
                    text: "fix the bug".to_string(),
                },
                Message::ToolUse {
                    id: String::new(),
                    name: "Bash".to_string(),
                    input: serde_json::json!({"command": "git diff"}),
                },
                Message::ToolUse {
                    id: String::new(),
                    name: "Write".to_string(),
                    input: serde_json::json!({"file_path": "/home/user/src/main.rs", "content": "..."}),
                },
                Message::Assistant {
                    text: "I fixed the bug by updating main.rs".to_string(),
                    model: None,
                    usage: None,
                },
            ],
        };

        let turns = extract_conversation_turns(&transcript, 1000, &|s| s.to_string());
        assert_eq!(turns.len(), 3);

        let tool_turn = &turns[1];
        assert_eq!(tool_turn.role, "tool");
        assert!(tool_turn.content.contains(r#"Bash(command: "git diff")"#));
        assert!(tool_turn.content.contains(r#"Write(file: "main.rs")"#));
        assert_eq!(tool_turn.files_touched, Some(vec!["main.rs".to_string()]));
    }
}
