use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ConversationTurn {
    pub turn: u32,
    pub role: String,    // "user", "assistant", "tool"
    pub content: String, // redacted message text
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files_touched: Option<Vec<String>>,
}

/// Detailed tracking of a subagent (Task tool) spawned during a prompt.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SubagentActivity {
    /// Unique agent ID from SubagentStart/Stop hook payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Agent type: "Explore", "Plan", "general-purpose", etc.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
    /// Task description provided when spawning the agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Lifecycle status: "started", "completed", "active".
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
    /// Tools used by this subagent (e.g., "Glob", "Grep", "Read").
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools_used: Vec<String>,
}

/// A single option presented in an AskUserQuestion prompt.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DecisionOption {
    /// The option text shown to the user.
    pub label: String,
    /// Whether this option was selected by the user.
    #[serde(default, skip_serializing_if = "is_false")]
    pub selected: bool,
}

/// A structured decision point where the AI asked the user a question
/// and the user selected from presented options (via AskUserQuestion tool).
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UserDecision {
    /// Unique tool_use ID from Claude Code (e.g., "toolu_001").
    pub tool_use_id: String,
    /// The question text shown to the user.
    pub question: String,
    /// Optional header/category label (e.g., "Approach", "Library").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header: Option<String>,
    /// All options that were presented to the user.
    pub options: Vec<DecisionOption>,
    /// Whether the user could select multiple options.
    #[serde(default, skip_serializing_if = "is_false")]
    pub multi_select: bool,
    /// The user's selected answer text. None at PostToolUse time; filled at Stop time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<String>,
}

/// A single file change within a prompt-centric receipt.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FileChange {
    pub path: String,
    pub line_range: (u32, u32),
    /// Git blob SHA of the file after this change.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blob_hash: Option<String>,
    /// Lines added in this file.
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub additions: u32,
    /// Lines deleted from this file.
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub deletions: u32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Receipt {
    pub id: String,
    pub provider: String,
    pub model: String,
    pub session_id: String,
    pub prompt_summary: String,
    /// Summary of the AI's response — what it actually did (from Stop hook's last_assistant_message).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_summary: Option<String>,
    pub prompt_hash: String,
    pub message_count: u32,
    pub cost_usd: f64,
    /// Actual token usage from the API (parsed from JSONL transcript).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_creation_tokens: Option<u64>,
    pub timestamp: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_start: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_end: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_duration_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ai_response_time_secs: Option<f64>,
    /// When the user submitted THIS specific prompt (set at UserPromptSubmit hook time).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_submitted_at: Option<DateTime<Utc>>,
    /// How long THIS prompt took to complete (from submission to Stop event), in seconds.
    /// Unlike session_duration_secs (whole session), this measures only this prompt's time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_duration_secs: Option<u64>,
    /// Lines the AI wrote that were kept unchanged in the final commit (accepted by human).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepted_lines: Option<u32>,
    /// Lines the AI wrote that were subsequently modified by the human before committing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overridden_lines: Option<u32>,
    pub user: String,
    /// Deprecated: use files_changed instead. Kept for backwards compat with old git notes.
    #[serde(default)]
    pub file_path: String,
    /// Deprecated: use files_changed instead. Kept for backwards compat with old git notes.
    #[serde(default = "default_line_range")]
    pub line_range: (u32, u32),
    /// All files changed by this prompt (prompt-centric model).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files_changed: Vec<FileChange>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_receipt_id: Option<String>,
    /// Session ID of the parent session this continues from (context exhaustion → new session).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    /// Whether this receipt is the first prompt in a continuation session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_continuation: Option<bool>,
    /// Number of continuation hops in the session chain (0 = original, 1 = first continuation).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continuation_depth: Option<u32>,
    /// Which user prompt (1-based) in the session this receipt corresponds to.
    /// Used to create separate receipts per prompt within the same session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_number: Option<u32>,
    /// Total lines added across all files in this prompt.
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub total_additions: u32,
    /// Total lines deleted across all files in this prompt.
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub total_deletions: u32,
    /// Tools used during this prompt session (e.g., "Bash", "Write", "Edit", "Grep").
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools_used: Vec<String>,
    /// MCP servers called during this session (extracted from mcp__<server>__<tool> pattern).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mcp_servers: Vec<String>,
    /// Sub-agents spawned via the Task tool during this session.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agents_spawned: Vec<String>,
    /// Detailed subagent activity tracking (lifecycle, tools used per agent).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subagent_activities: Vec<SubagentActivity>,
    /// Maximum number of parallel tool calls observed during this prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub concurrent_tool_calls: Option<u32>,
    /// Structured user decisions from AskUserQuestion tool interactions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub user_decisions: Vec<UserDecision>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation: Option<Vec<ConversationTurn>>,
}

fn default_line_range() -> (u32, u32) {
    (1, 1)
}

fn is_zero_u32(v: &u32) -> bool {
    *v == 0
}

fn is_false(v: &bool) -> bool {
    !v
}

impl Receipt {
    pub fn new_id() -> String {
        Uuid::new_v4().to_string()
    }

    /// Returns all file changes. Uses `files_changed` if present,
    /// otherwise falls back to the legacy `file_path`/`line_range` fields.
    pub fn all_file_changes(&self) -> Vec<FileChange> {
        if !self.files_changed.is_empty() {
            self.files_changed.clone()
        } else if !self.file_path.is_empty() {
            vec![FileChange {
                path: self.file_path.clone(),
                line_range: self.line_range,
                blob_hash: None,
                additions: 0,
                deletions: 0,
            }]
        } else {
            vec![]
        }
    }

    /// Returns all unique file paths from this receipt.
    pub fn all_file_paths(&self) -> Vec<String> {
        self.all_file_changes()
            .iter()
            .map(|fc| fc.path.clone())
            .collect()
    }

    /// Total lines changed across all files (line range span, legacy metric).
    pub fn total_lines_changed(&self) -> u32 {
        self.all_file_changes()
            .iter()
            .map(|fc| {
                if fc.line_range.1 >= fc.line_range.0 {
                    fc.line_range.1 - fc.line_range.0 + 1
                } else {
                    0
                }
            })
            .sum()
    }

    /// Sum additions and deletions from file-level stats when receipt-level totals are absent.
    pub fn effective_total_additions(&self) -> u32 {
        if self.total_additions > 0 {
            return self.total_additions;
        }
        self.all_file_changes().iter().map(|fc| fc.additions).sum()
    }

    #[allow(dead_code)]
    pub fn effective_total_deletions(&self) -> u32 {
        if self.total_deletions > 0 {
            return self.total_deletions;
        }
        self.all_file_changes().iter().map(|fc| fc.deletions).sum()
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
            file_mappings: if file_mappings.is_empty() {
                None
            } else {
                Some(file_mappings)
            },
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
            response_summary: Some("I fixed the bug".to_string()),
            prompt_hash: "sha256:abc123".to_string(),
            message_count: 5,
            cost_usd: 0.05,
            input_tokens: Some(1000),
            output_tokens: Some(500),
            cache_read_tokens: Some(200),
            cache_creation_tokens: Some(100),
            timestamp: Utc::now(),
            session_start: Some(Utc::now()),
            session_end: Some(Utc::now()),
            session_duration_secs: Some(120),
            ai_response_time_secs: Some(3.5),
            user: "Test <test@example.com>".to_string(),
            file_path: String::new(),
            line_range: (0, 0),
            files_changed: vec![
                FileChange {
                    path: "src/main.rs".to_string(),
                    line_range: (1, 10),
                    blob_hash: None,
                    additions: 10,
                    deletions: 0,
                },
                FileChange {
                    path: "src/lib.rs".to_string(),
                    line_range: (5, 20),
                    blob_hash: None,
                    additions: 16,
                    deletions: 2,
                },
            ],
            parent_receipt_id: None,
            parent_session_id: None,
            is_continuation: None,
            continuation_depth: None,
            prompt_number: Some(1),
            total_additions: 26,
            total_deletions: 2,
            tools_used: vec!["Write".to_string(), "Bash".to_string()],
            mcp_servers: vec![],
            agents_spawned: vec![],
            subagent_activities: vec![],
            concurrent_tool_calls: None,
            user_decisions: vec![],
            conversation: None,
            prompt_submitted_at: None,
            prompt_duration_secs: None,
            accepted_lines: None,
            overridden_lines: None,
        };

        let json = serde_json::to_string_pretty(&receipt).unwrap();
        let deserialized: Receipt = serde_json::from_str(&json).unwrap();
        assert_eq!(receipt.id, deserialized.id);
        assert_eq!(receipt.model, deserialized.model);
        assert_eq!(receipt.cost_usd, deserialized.cost_usd);
        assert_eq!(deserialized.files_changed.len(), 2);
        assert_eq!(deserialized.files_changed[0].path, "src/main.rs");
        assert_eq!(deserialized.tools_used, vec!["Write", "Bash"]);
    }

    #[test]
    fn test_optional_fields_omitted() {
        let receipt = Receipt {
            id: Receipt::new_id(),
            provider: "claude".to_string(),
            model: "claude-sonnet-4-5-20250929".to_string(),
            session_id: "test".to_string(),
            prompt_summary: "test".to_string(),
            response_summary: None,
            prompt_hash: "sha256:abc".to_string(),
            message_count: 1,
            cost_usd: 0.0,
            input_tokens: None,
            output_tokens: None,
            cache_read_tokens: None,
            cache_creation_tokens: None,
            timestamp: Utc::now(),
            session_start: None,
            session_end: None,
            session_duration_secs: None,
            ai_response_time_secs: None,
            user: "Test <test@example.com>".to_string(),
            file_path: String::new(),
            line_range: (0, 0),
            files_changed: vec![],
            parent_receipt_id: None,
            parent_session_id: None,
            is_continuation: None,
            continuation_depth: None,
            prompt_number: None,
            total_additions: 0,
            total_deletions: 0,
            tools_used: vec![],
            mcp_servers: vec![],
            agents_spawned: vec![],
            subagent_activities: vec![],
            concurrent_tool_calls: None,
            user_decisions: vec![],
            conversation: None,
            prompt_submitted_at: None,
            prompt_duration_secs: None,
            accepted_lines: None,
            overridden_lines: None,
        };

        let json = serde_json::to_string(&receipt).unwrap();
        assert!(!json.contains("session_start"));
        assert!(!json.contains("session_end"));
        assert!(!json.contains("session_duration_secs"));
        assert!(!json.contains("ai_response_time_secs"));
        assert!(!json.contains("parent_receipt_id"));
        assert!(!json.contains("conversation"));
        assert!(!json.contains("prompt_number"));
        assert!(!json.contains("files_changed"));
        assert!(!json.contains("tools_used"));
        assert!(!json.contains("mcp_servers"));
        assert!(!json.contains("agents_spawned"));
        assert!(!json.contains("prompt_submitted_at"));
        assert!(!json.contains("prompt_duration_secs"));
        assert!(!json.contains("accepted_lines"));
        assert!(!json.contains("response_summary"));
        assert!(!json.contains("input_tokens"));
        assert!(!json.contains("output_tokens"));
        assert!(!json.contains("cache_read_tokens"));
        assert!(!json.contains("cache_creation_tokens"));
        assert!(!json.contains("overridden_lines"));
        assert!(!json.contains("parent_session_id"));
        assert!(!json.contains("is_continuation"));
        assert!(!json.contains("continuation_depth"));
        assert!(!json.contains("subagent_activities"));
        assert!(!json.contains("concurrent_tool_calls"));
        assert!(!json.contains("user_decisions"));
    }

    #[test]
    fn test_all_file_changes_new_format() {
        let receipt = Receipt {
            id: "test".to_string(),
            provider: "claude".to_string(),
            model: "opus".to_string(),
            session_id: "s1".to_string(),
            prompt_summary: "test".to_string(),
            response_summary: None,
            prompt_hash: "h".to_string(),
            message_count: 1,
            cost_usd: 0.0,
            input_tokens: None,
            output_tokens: None,
            cache_read_tokens: None,
            cache_creation_tokens: None,
            timestamp: Utc::now(),
            session_start: None,
            session_end: None,
            session_duration_secs: None,
            ai_response_time_secs: None,
            user: "u".to_string(),
            file_path: String::new(),
            line_range: (0, 0),
            files_changed: vec![
                FileChange {
                    path: "a.rs".to_string(),
                    line_range: (1, 10),
                    blob_hash: None,
                    additions: 0,
                    deletions: 0,
                },
                FileChange {
                    path: "b.rs".to_string(),
                    line_range: (5, 15),
                    blob_hash: None,
                    additions: 0,
                    deletions: 0,
                },
            ],
            parent_receipt_id: None,
            parent_session_id: None,
            is_continuation: None,
            continuation_depth: None,
            prompt_number: None,
            total_additions: 0,
            total_deletions: 0,
            tools_used: vec![],
            mcp_servers: vec![],
            agents_spawned: vec![],
            subagent_activities: vec![],
            concurrent_tool_calls: None,
            user_decisions: vec![],
            conversation: None,
            prompt_submitted_at: None,
            prompt_duration_secs: None,
            accepted_lines: None,
            overridden_lines: None,
        };
        let changes = receipt.all_file_changes();
        assert_eq!(changes.len(), 2);
        assert_eq!(receipt.total_lines_changed(), 21); // 10 + 11
    }

    #[test]
    fn test_all_file_changes_legacy_format() {
        let receipt = Receipt {
            id: "test".to_string(),
            provider: "claude".to_string(),
            model: "opus".to_string(),
            session_id: "s1".to_string(),
            prompt_summary: "test".to_string(),
            response_summary: None,
            prompt_hash: "h".to_string(),
            message_count: 1,
            cost_usd: 0.0,
            input_tokens: None,
            output_tokens: None,
            cache_read_tokens: None,
            cache_creation_tokens: None,
            timestamp: Utc::now(),
            session_start: None,
            session_end: None,
            session_duration_secs: None,
            ai_response_time_secs: None,
            user: "u".to_string(),
            file_path: "old_file.rs".to_string(),
            line_range: (1, 50),
            files_changed: vec![],
            parent_receipt_id: None,
            parent_session_id: None,
            is_continuation: None,
            continuation_depth: None,
            prompt_number: None,
            total_additions: 0,
            total_deletions: 0,
            tools_used: vec![],
            mcp_servers: vec![],
            agents_spawned: vec![],
            subagent_activities: vec![],
            concurrent_tool_calls: None,
            user_decisions: vec![],
            conversation: None,
            prompt_submitted_at: None,
            prompt_duration_secs: None,
            accepted_lines: None,
            overridden_lines: None,
        };
        let changes = receipt.all_file_changes();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].path, "old_file.rs");
        assert_eq!(receipt.total_lines_changed(), 50);
    }

    #[test]
    fn test_backwards_compat_deserialization() {
        // Simulate old JSON without files_changed field
        let json = r#"{
            "id": "test",
            "provider": "claude",
            "model": "opus",
            "session_id": "s1",
            "prompt_summary": "test",
            "prompt_hash": "h",
            "message_count": 1,
            "cost_usd": 0.0,
            "timestamp": "2026-01-01T00:00:00Z",
            "user": "u",
            "file_path": "legacy.rs",
            "line_range": [1, 30]
        }"#;
        let receipt: Receipt = serde_json::from_str(json).unwrap();
        assert_eq!(receipt.file_path, "legacy.rs");
        assert!(receipt.files_changed.is_empty());
        let changes = receipt.all_file_changes();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].path, "legacy.rs");
        assert_eq!(changes[0].line_range, (1, 30));
    }

    #[test]
    fn test_user_decision_serialization() {
        let decision = UserDecision {
            tool_use_id: "toolu_001".to_string(),
            question: "Which approach?".to_string(),
            header: Some("Approach".to_string()),
            options: vec![
                DecisionOption {
                    label: "CSS variables".to_string(),
                    selected: true,
                },
                DecisionOption {
                    label: "Theme context".to_string(),
                    selected: false,
                },
            ],
            multi_select: false,
            answer: Some("CSS variables".to_string()),
        };
        let json = serde_json::to_string(&decision).unwrap();
        assert!(json.contains("CSS variables"));
        // multi_select: false should be omitted
        assert!(!json.contains("multi_select"));
        // selected: false should be omitted on "Theme context"
        let deser: UserDecision = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.tool_use_id, "toolu_001");
        assert_eq!(deser.answer, Some("CSS variables".to_string()));
        assert!(deser.options[0].selected);
        assert!(!deser.options[1].selected);
    }

    #[test]
    fn test_note_payload() {
        let payload = NotePayload::new(vec![]);
        let json = serde_json::to_string_pretty(&payload).unwrap();
        assert!(json.contains("blameprompt_version"));
        assert!(json.contains(env!("CARGO_PKG_VERSION")));
    }
}
