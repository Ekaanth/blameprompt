use crate::core::receipt::Receipt;
use chrono::{DateTime, Utc};
use std::collections::HashMap;

/// Aggregated session statistics, deduplicated by session_id.
/// Uses interval merging to avoid double-counting time from parallel sub-agents.
pub struct SessionStats {
    /// Number of unique sessions
    pub unique_sessions: usize,
    /// Total duration across unique sessions (seconds) — raw sum, may double-count parallel agents
    pub total_duration_secs: u64,
    /// Wall-clock time with overlapping intervals merged (seconds) — no double-counting
    pub wall_clock_secs: u64,
    /// Average duration per unique session (seconds)
    pub avg_duration_secs: u64,
    /// Earliest session start time across all receipts
    pub earliest_start: Option<DateTime<Utc>>,
    /// Latest session end time across all receipts
    pub latest_end: Option<DateTime<Utc>>,
    /// Per-session durations keyed by session_id
    #[allow(dead_code)]
    pub per_session: HashMap<String, u64>,
}

/// Calculate session stats from a slice of receipts, deduplicating by session_id.
///
/// Multiple receipts from the same session all carry the same `session_duration_secs`.
/// This function ensures each session is counted only once and merges overlapping
/// time intervals from parallel sub-agents to avoid double-counting wall-clock time.
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

    // Collect time intervals from session_start/session_end for interval merging
    // Dedup by session_id: keep the widest interval per session
    let mut session_intervals: HashMap<String, (DateTime<Utc>, DateTime<Utc>)> = HashMap::new();
    for r in receipts {
        if let (Some(start), Some(end)) = (r.session_start, r.session_end) {
            let entry = session_intervals.entry(r.session_id.clone()).or_insert((start, end));
            if start < entry.0 {
                entry.0 = start;
            }
            if end > entry.1 {
                entry.1 = end;
            }
        }
    }

    let intervals: Vec<(DateTime<Utc>, DateTime<Utc>)> = session_intervals.into_values().collect();
    let wall_clock_secs = merge_intervals_duration(&intervals);

    // Find earliest start and latest end
    let earliest_start = receipts.iter().filter_map(|r| r.session_start).min();
    let latest_end = receipts.iter().filter_map(|r| r.session_end).max();

    SessionStats {
        unique_sessions,
        total_duration_secs,
        wall_clock_secs,
        avg_duration_secs,
        earliest_start,
        latest_end,
        per_session,
    }
}

/// Merge overlapping time intervals and return total wall-clock seconds.
///
/// Sub-agents spawned by Claude Code run in parallel with different session_ids.
/// Without merging, summing their durations double-counts wall-clock time.
/// This merges overlapping/adjacent intervals to produce the actual time spent.
fn merge_intervals_duration(intervals: &[(DateTime<Utc>, DateTime<Utc>)]) -> u64 {
    if intervals.is_empty() {
        return 0;
    }

    let mut sorted: Vec<(DateTime<Utc>, DateTime<Utc>)> = intervals.to_vec();
    sorted.sort_by_key(|&(start, _)| start);

    let mut merged: Vec<(DateTime<Utc>, DateTime<Utc>)> = Vec::new();
    merged.push(sorted[0]);

    for &(start, end) in &sorted[1..] {
        let last = merged.last_mut().unwrap();
        if start <= last.1 {
            // Overlapping or adjacent — extend the current interval
            if end > last.1 {
                last.1 = end;
            }
        } else {
            // Gap — start a new interval
            merged.push((start, end));
        }
    }

    merged
        .iter()
        .map(|(start, end)| {
            let diff = end.signed_duration_since(*start);
            diff.num_seconds().max(0) as u64
        })
        .sum()
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
    use chrono::{Duration, Utc};

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
            files_changed: vec![],
            parent_receipt_id: None,
            prompt_number: None,
            tools_used: vec![],
            mcp_servers: vec![],
            agents_spawned: vec![],
            conversation: None,
        }
    }

    fn make_receipt_with_times(
        session_id: &str,
        duration: Option<u64>,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Receipt {
        let mut r = make_receipt(session_id, duration);
        r.session_start = Some(start);
        r.session_end = Some(end);
        r
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
    fn test_merges_overlapping_intervals() {
        // Main agent: 10:00 - 10:10 (600s)
        // Sub-agent:  10:02 - 10:08 (360s) — fully within main
        let base = Utc::now();
        let r1 = make_receipt_with_times(
            "main-agent",
            Some(600),
            base,
            base + Duration::seconds(600),
        );
        let r2 = make_receipt_with_times(
            "sub-agent",
            Some(360),
            base + Duration::seconds(120),
            base + Duration::seconds(480),
        );
        let receipts: Vec<&Receipt> = vec![&r1, &r2];

        let stats = calculate(&receipts);
        assert_eq!(stats.unique_sessions, 2);
        assert_eq!(stats.total_duration_secs, 960); // raw sum: 600 + 360
        assert_eq!(stats.wall_clock_secs, 600); // merged: only 600s of wall clock
    }

    #[test]
    fn test_merges_partially_overlapping_intervals() {
        // Agent A: 10:00 - 10:10 (600s)
        // Agent B: 10:05 - 10:15 (600s) — partially overlapping
        let base = Utc::now();
        let r1 = make_receipt_with_times(
            "agent-a",
            Some(600),
            base,
            base + Duration::seconds(600),
        );
        let r2 = make_receipt_with_times(
            "agent-b",
            Some(600),
            base + Duration::seconds(300),
            base + Duration::seconds(900),
        );
        let receipts: Vec<&Receipt> = vec![&r1, &r2];

        let stats = calculate(&receipts);
        assert_eq!(stats.total_duration_secs, 1200); // raw: 600 + 600
        assert_eq!(stats.wall_clock_secs, 900); // merged: 10:00 - 10:15
    }

    #[test]
    fn test_non_overlapping_intervals_sum_normally() {
        // Agent A: 10:00 - 10:10 (600s)
        // Agent B: 11:00 - 11:05 (300s) — no overlap
        let base = Utc::now();
        let r1 = make_receipt_with_times(
            "agent-a",
            Some(600),
            base,
            base + Duration::seconds(600),
        );
        let r2 = make_receipt_with_times(
            "agent-b",
            Some(300),
            base + Duration::seconds(3600),
            base + Duration::seconds(3900),
        );
        let receipts: Vec<&Receipt> = vec![&r1, &r2];

        let stats = calculate(&receipts);
        assert_eq!(stats.total_duration_secs, 900);
        assert_eq!(stats.wall_clock_secs, 900); // no overlap, same as raw
    }

    #[test]
    fn test_earliest_start_and_latest_end() {
        let base = Utc::now();
        let r1 = make_receipt_with_times(
            "agent-a",
            Some(600),
            base + Duration::seconds(100),
            base + Duration::seconds(700),
        );
        let r2 = make_receipt_with_times(
            "agent-b",
            Some(300),
            base,
            base + Duration::seconds(900),
        );
        let receipts: Vec<&Receipt> = vec![&r1, &r2];

        let stats = calculate(&receipts);
        assert_eq!(stats.earliest_start, Some(base));
        assert_eq!(stats.latest_end, Some(base + Duration::seconds(900)));
    }

    #[test]
    fn test_no_time_data_yields_zero_wall_clock() {
        let r1 = make_receipt("session-a", Some(600));
        let r2 = make_receipt("session-b", Some(300));
        let receipts: Vec<&Receipt> = vec![&r1, &r2];

        let stats = calculate(&receipts);
        assert_eq!(stats.wall_clock_secs, 0); // no start/end data
        assert_eq!(stats.total_duration_secs, 900); // raw sum still works
        assert!(stats.earliest_start.is_none());
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
