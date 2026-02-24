use crate::core::receipt::Receipt;
use std::collections::HashMap;

/// Aggregated session statistics, deduplicated by session_id.
pub struct SessionStats {
    /// Number of unique sessions
    pub unique_sessions: usize,
    /// Total duration across unique sessions (seconds)
    pub total_duration_secs: u64,
    /// Average duration per unique session (seconds)
    pub avg_duration_secs: u64,
    /// Per-session durations keyed by session_id
    #[allow(dead_code)]
    pub per_session: HashMap<String, u64>,
}

/// Calculate session stats from a slice of receipts, deduplicating by session_id.
///
/// Multiple receipts from the same session all carry the same `session_duration_secs`.
/// This function ensures each session is counted only once.
pub fn calculate(receipts: &[&Receipt]) -> SessionStats {
    let mut per_session: HashMap<String, u64> = HashMap::new();

    for r in receipts {
        if let Some(dur) = r.session_duration_secs {
            let entry = per_session.entry(r.session_id.clone()).or_insert(0);
            // Keep the max duration per session (in case of slight variations
            // between receipts created at different points during the session)
            if dur > *entry {
                *entry = dur;
            }
        }
    }

    let unique_sessions = per_session.len();
    let total_duration_secs: u64 = per_session.values().sum();
    let avg_duration_secs = if unique_sessions > 0 {
        total_duration_secs / unique_sessions as u64
    } else {
        0
    };

    SessionStats {
        unique_sessions,
        total_duration_secs,
        avg_duration_secs,
        per_session,
    }
}

/// Format a duration in seconds as "Xh Ym" or "Xm Ys".
pub fn format_duration(secs: u64) -> String {
    if secs >= 3600 {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    } else {
        format!("{}m {}s", secs / 60, secs % 60)
    }
}

/// Estimate developer hours saved from AI-generated lines.
///
/// Uses 30s/line as a conservative estimate. Industry data suggests developers
/// write ~25 lines/hour for complex code, and AI removes significant manual effort.
/// 30s/line accounts for the fact that AI output still needs review.
pub fn estimate_dev_hours_saved(ai_lines: u32) -> f64 {
    (ai_lines as f64 * 30.0) / 3600.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn make_receipt(session_id: &str, duration: Option<u64>) -> Receipt {
        Receipt {
            id: "test".to_string(),
            provider: "claude".to_string(),
            model: "claude-sonnet-4-5-20250929".to_string(),
            session_id: session_id.to_string(),
            prompt_summary: "test".to_string(),
            prompt_hash: "sha256:abc".to_string(),
            message_count: 1,
            cost_usd: 0.01,
            timestamp: Utc::now(),
            session_start: None,
            session_end: None,
            session_duration_secs: duration,
            ai_response_time_secs: None,
            user: "test".to_string(),
            file_path: "test.rs".to_string(),
            line_range: (1, 10),
            parent_receipt_id: None,
            conversation: None,
        }
    }

    #[test]
    fn test_deduplicates_sessions() {
        let r1 = make_receipt("session-a", Some(600));
        let r2 = make_receipt("session-a", Some(600)); // same session
        let r3 = make_receipt("session-b", Some(300));
        let receipts: Vec<&Receipt> = vec![&r1, &r2, &r3];

        let stats = calculate(&receipts);
        assert_eq!(stats.unique_sessions, 2);
        assert_eq!(stats.total_duration_secs, 900); // 600 + 300, not 600 + 600 + 300
    }

    #[test]
    fn test_keeps_max_duration_per_session() {
        let r1 = make_receipt("session-a", Some(500));
        let r2 = make_receipt("session-a", Some(600)); // later receipt has longer duration
        let receipts: Vec<&Receipt> = vec![&r1, &r2];

        let stats = calculate(&receipts);
        assert_eq!(stats.unique_sessions, 1);
        assert_eq!(stats.total_duration_secs, 600); // max, not sum
    }

    #[test]
    fn test_skips_none_durations() {
        let r1 = make_receipt("session-a", None);
        let r2 = make_receipt("session-b", Some(300));
        let receipts: Vec<&Receipt> = vec![&r1, &r2];

        let stats = calculate(&receipts);
        assert_eq!(stats.unique_sessions, 1); // only session-b counted
        assert_eq!(stats.total_duration_secs, 300);
    }

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(90), "1m 30s");
        assert_eq!(format_duration(3661), "1h 1m");
        assert_eq!(format_duration(0), "0m 0s");
    }

    #[test]
    fn test_estimate_dev_hours() {
        let hours = estimate_dev_hours_saved(120); // 120 lines * 30s = 3600s = 1h
        assert!((hours - 1.0).abs() < 0.01);
    }
}
