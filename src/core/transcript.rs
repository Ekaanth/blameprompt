use chrono::{DateTime, Utc};
use std::path::Path;

#[derive(Debug, Clone)]
pub enum Message {
    User {
        text: String,
        #[allow(dead_code)]
        timestamp: Option<String>,
    },
    Assistant {
        text: String,
        #[allow(dead_code)]
        timestamp: Option<String>,
    },
    ToolUse {
        name: String,
        #[allow(dead_code)]
        input: serde_json::Value,
        #[allow(dead_code)]
        timestamp: Option<String>,
    },
}

#[derive(Debug)]
pub struct Transcript {
    pub messages: Vec<Message>,
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
}

pub fn parse_claude_jsonl(transcript_path: &str) -> Result<TranscriptParseResult, String> {
    let path = Path::new(transcript_path);

    // Extract session UUID from filename
    let session_id = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();

    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Cannot read transcript: {}", e))?;

    let mut messages = Vec::new();
    let mut model: Option<String> = None;
    let mut files_modified = Vec::new();

    let mut first_timestamp: Option<DateTime<Utc>> = None;
    let mut last_timestamp: Option<DateTime<Utc>> = None;
    let mut response_times: Vec<f64> = Vec::new();
    let mut last_user_timestamp: Option<DateTime<Utc>> = None;

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

        let ts_str = entry.get("timestamp").and_then(|v| v.as_str()).map(String::from);

        match entry.get("type").and_then(|v| v.as_str()) {
            Some("user") => {
                let text = entry
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.as_str())
                    .unwrap_or("")
                    .to_string();
                messages.push(Message::User { text, timestamp: ts_str });
            }
            Some("assistant") => {
                let msg = entry.get("message");

                // Extract model from first assistant message that has it
                if model.is_none() {
                    if let Some(m) = msg.and_then(|m| m.get("model")).and_then(|v| v.as_str()) {
                        model = Some(m.to_string());
                    }
                }

                // Parse content array
                if let Some(content_arr) = msg.and_then(|m| m.get("content")).and_then(|c| c.as_array()) {
                    for item in content_arr {
                        match item.get("type").and_then(|v| v.as_str()) {
                            Some("text") => {
                                let text = item.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                messages.push(Message::Assistant { text, timestamp: ts_str.clone() });
                            }
                            Some("tool_use") => {
                                let name = item.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                let input = item.get("input").cloned().unwrap_or(serde_json::Value::Null);

                                // Track modified files
                                if let Some(fp) = input.get("file_path").and_then(|v| v.as_str()) {
                                    if !files_modified.contains(&fp.to_string()) {
                                        files_modified.push(fp.to_string());
                                    }
                                }

                                messages.push(Message::ToolUse { name, input, timestamp: ts_str.clone() });
                            }
                            _ => {}
                        }
                    }
                } else if let Some(content_str) = msg.and_then(|m| m.get("content")).and_then(|c| c.as_str()) {
                    // Content is a plain string
                    messages.push(Message::Assistant { text: content_str.to_string(), timestamp: ts_str });
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
    })
}

pub fn first_user_prompt(transcript: &Transcript) -> Option<String> {
    for msg in &transcript.messages {
        if let Message::User { text, .. } = msg {
            if !text.is_empty() {
                let truncated: String = text.chars().take(200).collect();
                return Some(truncated);
            }
        }
    }
    None
}

/// Extract structured conversation turns for storage in receipts.
/// Each turn has a role, content (truncated), and optional tool/file info.
pub fn extract_conversation_turns(
    transcript: &Transcript,
    max_turn_length: usize,
    redact_fn: &dyn Fn(&str) -> String,
) -> Vec<crate::core::receipt::ConversationTurn> {
    use crate::core::receipt::ConversationTurn;

    let mut turns = Vec::new();
    let mut turn_idx = 0u32;

    for msg in &transcript.messages {
        match msg {
            Message::User { text, .. } => {
                if !text.is_empty() {
                    let truncated: String = text.chars().take(max_turn_length).collect();
                    turns.push(ConversationTurn {
                        turn: turn_idx,
                        role: "user".to_string(),
                        content: redact_fn(&truncated),
                        tool_name: None,
                        files_touched: None,
                    });
                    turn_idx += 1;
                }
            }
            Message::Assistant { text, .. } => {
                if !text.is_empty() {
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
            }
            Message::ToolUse { name, input, .. } => {
                let files: Option<Vec<String>> = input
                    .get("file_path")
                    .and_then(|v| v.as_str())
                    .map(|fp| vec![fp.to_string()]);

                turns.push(ConversationTurn {
                    turn: turn_idx,
                    role: "tool".to_string(),
                    content: format!("{}()", name),
                    tool_name: Some(name.clone()),
                    files_touched: files,
                });
                turn_idx += 1;
            }
        }
    }

    turns
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
        let jsonl = "not json\n{\"type\":\"user\",\"message\":{\"content\":\"hello\"}}\nalso not json\n";
        let tmp = std::env::temp_dir().join("test_malformed.jsonl");
        std::fs::write(&tmp, jsonl).unwrap();
        let result = parse_claude_jsonl(tmp.to_str().unwrap()).unwrap();
        assert_eq!(result.transcript.messages.len(), 1);
        std::fs::remove_file(tmp).ok();
    }

    #[test]
    fn test_first_user_prompt() {
        let transcript = Transcript {
            messages: vec![
                Message::User { text: "write a function".to_string(), timestamp: None },
            ],
        };
        assert_eq!(first_user_prompt(&transcript), Some("write a function".to_string()));
    }
}
