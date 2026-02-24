use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ConversationTurn {
    pub turn: u32,
    pub role: String,          // "user", "assistant", "tool"
    pub content: String,       // redacted message text
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files_touched: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Receipt {
    pub id: String,
    pub provider: String,
    pub model: String,
    pub session_id: String,
    pub prompt_summary: String,
    pub prompt_hash: String,
    pub message_count: u32,
    pub cost_usd: f64,
    pub timestamp: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_start: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_end: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_duration_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ai_response_time_secs: Option<f64>,
    pub user: String,
    pub file_path: String,
    pub line_range: (u32, u32),
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_receipt_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation: Option<Vec<ConversationTurn>>,
}

impl Receipt {
    pub fn new_id() -> String {
        Uuid::new_v4().to_string()
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub enum CodeOrigin {
    #[serde(rename = "ai_generated")]
    AiGenerated,
    #[serde(rename = "human_edited")]
    HumanEdited,
    #[serde(rename = "pure_human")]
    PureHuman,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Hunk {
    pub start_line: u32,
    pub end_line: u32,
    pub origin: CodeOrigin,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_turn: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FileMapping {
    pub path: String,
    pub blob_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_blob_hash: Option<String>,
    pub hunks: Vec<Hunk>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CodeOriginStats {
    pub ai_generated_pct: f64,
    pub human_edited_pct: f64,
    pub pure_human_pct: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct NotePayload {
    pub blameprompt_version: String,
    pub receipts: Vec<Receipt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_mappings: Option<Vec<FileMapping>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code_origin: Option<CodeOriginStats>,
}

impl NotePayload {
    pub fn new(receipts: Vec<Receipt>) -> Self {
        NotePayload {
            blameprompt_version: env!("CARGO_PKG_VERSION").to_string(),
            receipts,
            file_mappings: None,
            code_origin: None,
        }
    }

    #[allow(dead_code)]
    pub fn with_file_mappings(receipts: Vec<Receipt>, file_mappings: Vec<FileMapping>) -> Self {
        NotePayload {
            blameprompt_version: env!("CARGO_PKG_VERSION").to_string(),
            receipts,
            file_mappings: if file_mappings.is_empty() { None } else { Some(file_mappings) },
            code_origin: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_receipt_roundtrip() {
        let receipt = Receipt {
            id: Receipt::new_id(),
            provider: "claude".to_string(),
            model: "claude-sonnet-4-5-20250929".to_string(),
            session_id: "test-session".to_string(),
            prompt_summary: "test prompt".to_string(),
            prompt_hash: "sha256:abc123".to_string(),
            message_count: 5,
            cost_usd: 0.05,
            timestamp: Utc::now(),
            session_start: Some(Utc::now()),
            session_end: Some(Utc::now()),
            session_duration_secs: Some(120),
            ai_response_time_secs: Some(3.5),
            user: "Test <test@example.com>".to_string(),
            file_path: "src/main.rs".to_string(),
            line_range: (1, 10),
            parent_receipt_id: None,
            conversation: None,
        };

        let json = serde_json::to_string_pretty(&receipt).unwrap();
        let deserialized: Receipt = serde_json::from_str(&json).unwrap();
        assert_eq!(receipt.id, deserialized.id);
        assert_eq!(receipt.model, deserialized.model);
        assert_eq!(receipt.cost_usd, deserialized.cost_usd);
    }

    #[test]
    fn test_optional_fields_omitted() {
        let receipt = Receipt {
            id: Receipt::new_id(),
            provider: "claude".to_string(),
            model: "claude-sonnet-4-5-20250929".to_string(),
            session_id: "test".to_string(),
            prompt_summary: "test".to_string(),
            prompt_hash: "sha256:abc".to_string(),
            message_count: 1,
            cost_usd: 0.0,
            timestamp: Utc::now(),
            session_start: None,
            session_end: None,
            session_duration_secs: None,
            ai_response_time_secs: None,
            user: "Test <test@example.com>".to_string(),
            file_path: "test.rs".to_string(),
            line_range: (1, 1),
            parent_receipt_id: None,
            conversation: None,
        };

        let json = serde_json::to_string(&receipt).unwrap();
        assert!(!json.contains("session_start"));
        assert!(!json.contains("session_end"));
        assert!(!json.contains("session_duration_secs"));
        assert!(!json.contains("ai_response_time_secs"));
        assert!(!json.contains("parent_receipt_id"));
        assert!(!json.contains("conversation"));
    }

    #[test]
    fn test_note_payload() {
        let payload = NotePayload::new(vec![]);
        let json = serde_json::to_string_pretty(&payload).unwrap();
        assert!(json.contains("blameprompt_version"));
        assert!(json.contains(env!("CARGO_PKG_VERSION")));
    }
}
