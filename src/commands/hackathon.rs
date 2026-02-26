use crate::commands::audit;
use crate::core::{model_classifier, receipt::Receipt, session_stats, util};
use chrono::{DateTime, Utc};
use std::collections::{HashMap, HashSet};
use std::fmt::Write;

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Severity {
    Info,
    Warning,
    Critical,
}

impl Severity {
    fn label(&self) -> &'static str {
        match self {
            Severity::Info => "INFO",
            Severity::Warning => "WARNING",
            Severity::Critical => "CRITICAL",
        }
    }

    fn weight(&self) -> f64 {
        match self {
            Severity::Info => 2.0,
            Severity::Warning => 8.0,
            Severity::Critical => 20.0,
        }
    }
}

struct AnomalyFlag {
    severity: Severity,
    category: String,
    description: String,
    evidence: String,
}

struct FileAttribution {
    path: String,
    total_lines: u32,
    ai_lines: u32,
    receipt_count: u32,
    first_touched: Option<DateTime<Utc>>,
}

struct TimelineEntry {
    timestamp: DateTime<Utc>,
    prompt_summary: String,
    duration_secs: Option<u64>,
    model: String,
    files_touched: Vec<String>,
    additions: u32,
    deletions: u32,
    within_window: bool,
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub fn generate_hackathon_report(
    start_str: &str,
    end_str: &str,
    output_path: &str,
    author: Option<&str>,
    include_uncommitted: bool,
) -> Result<(), String> {
    let hackathon_start = parse_datetime(start_str).ok_or_else(|| {
        format!(
            "Invalid --start: \"{}\". Use ISO 8601 (e.g. 2026-02-26T09:00:00Z).",
            start_str
        )
    })?;
    let hackathon_end = parse_datetime(end_str).ok_or_else(|| {
        format!(
            "Invalid --end: \"{}\". Use ISO 8601 (e.g. 2026-02-26T21:00:00Z).",
            end_str
        )
    })?;

    if hackathon_end <= hackathon_start {
        return Err("--end must be after --start".to_string());
    }

    // Collect ALL entries (not time-filtered) — we need out-of-window activity for anomaly detection.
    let mut entries = audit::collect_all_entries(None, None, author, include_uncommitted)?;
    if entries.is_empty() && !include_uncommitted {
        let staged = audit::collect_staged_entries();
        if !staged.is_empty() {
            entries = staged;
        }
    }

    let all_receipts: Vec<&Receipt> = entries.iter().flat_map(|e| &e.receipts).collect();

    if all_receipts.is_empty() {
        return Err(
            "No AI receipts found. Is BlamePrompt installed and have you used AI coding tools?"
                .to_string(),
        );
    }

    let timeline = build_timeline(&all_receipts, hackathon_start, hackathon_end);
    let anomalies = detect_anomalies(
        &all_receipts,
        &entries,
        &timeline,
        hackathon_start,
        hackathon_end,
    );
    let integrity_score = calculate_integrity_score(&anomalies);
    let file_attribution = build_file_attribution(&all_receipts);

    let mut md = String::with_capacity(8192);
    write_header(&mut md, hackathon_start, hackathon_end, author);
    write_summary(
        &mut md,
        &all_receipts,
        &timeline,
        &anomalies,
        integrity_score,
    );
    write_timeline(&mut md, &timeline);
    write_code_attribution(&mut md, &all_receipts, &file_attribution);
    write_anomaly_flags(&mut md, &anomalies);
    write_integrity_assessment(&mut md, &anomalies, integrity_score);
    write_footer(&mut md);

    std::fs::write(output_path, &md).map_err(|e| format!("Cannot write report: {}", e))?;
    println!("Hackathon report written to {}", output_path);
    println!(
        "  Integrity score: {}/100 ({})",
        integrity_score,
        score_label(integrity_score)
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Timeline builder
// ---------------------------------------------------------------------------

fn build_timeline(
    receipts: &[&Receipt],
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Vec<TimelineEntry> {
    let mut timeline: Vec<TimelineEntry> = receipts
        .iter()
        .map(|r| {
            let ts = r.prompt_submitted_at.unwrap_or(r.timestamp);
            let files: Vec<String> = r.all_file_paths().iter().map(|f| make_rel(f)).collect();
            TimelineEntry {
                timestamp: ts,
                prompt_summary: r.prompt_summary.chars().take(200).collect(),
                duration_secs: r.prompt_duration_secs,
                model: model_classifier::display_name(&r.model),
                files_touched: files,
                additions: r.effective_total_additions(),
                deletions: r.effective_total_deletions(),
                within_window: ts >= start && ts <= end,
            }
        })
        .collect();

    timeline.sort_by_key(|t| t.timestamp);
    timeline
}

// ---------------------------------------------------------------------------
// Anomaly detection — orchestrator + 7 detectors
// ---------------------------------------------------------------------------

fn detect_anomalies(
    receipts: &[&Receipt],
    entries: &[audit::AuditEntry],
    timeline: &[TimelineEntry],
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Vec<AnomalyFlag> {
    let mut flags = Vec::new();
    flags.extend(detect_time_window_violations(timeline, start, end));
    flags.extend(detect_pre_written_code(receipts));
    flags.extend(detect_untracked_files(receipts));
    flags.extend(detect_duplicate_prompt_hashes(receipts));
    flags.extend(detect_batch_commits(entries, receipts));
    flags.extend(detect_unusual_session_patterns(receipts, timeline));
    flags.extend(detect_time_gaps(timeline, start));
    flags
}

/// Detector 1: Prompts submitted outside the hackathon time window.
fn detect_time_window_violations(
    timeline: &[TimelineEntry],
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Vec<AnomalyFlag> {
    let outside: Vec<&TimelineEntry> = timeline.iter().filter(|t| !t.within_window).collect();
    if outside.is_empty() {
        return vec![];
    }

    let before_count = outside.iter().filter(|t| t.timestamp < start).count();
    let after_count = outside.iter().filter(|t| t.timestamp > end).count();
    let additions_outside: u32 = outside.iter().map(|t| t.additions).sum();

    let severity = if additions_outside > 100 {
        Severity::Critical
    } else {
        Severity::Warning
    };

    vec![AnomalyFlag {
        severity,
        category: "Time Window Violation".into(),
        description: format!(
            "{} prompt(s) submitted outside the hackathon window ({} before start, {} after end)",
            outside.len(),
            before_count,
            after_count,
        ),
        evidence: format!(
            "{} lines added outside window. Earliest: {}. Latest: {}",
            additions_outside,
            outside
                .first()
                .map(|t| t.timestamp.to_rfc3339())
                .unwrap_or_default(),
            outside
                .last()
                .map(|t| t.timestamp.to_rfc3339())
                .unwrap_or_default(),
        ),
    }]
}

/// Detector 2: File appears fully-formed in a single prompt with no iterative history.
fn detect_pre_written_code(receipts: &[&Receipt]) -> Vec<AnomalyFlag> {
    let mut flags = Vec::new();

    // Build per-file history: (timestamp, additions, prompt_summary)
    let mut file_history: HashMap<String, Vec<(DateTime<Utc>, u32, String)>> = HashMap::new();
    for r in receipts {
        let ts = r.prompt_submitted_at.unwrap_or(r.timestamp);
        for fc in r.all_file_changes() {
            file_history.entry(make_rel(&fc.path)).or_default().push((
                ts,
                fc.additions,
                r.prompt_summary.clone(),
            ));
        }
    }

    for (file, mut history) in file_history {
        history.sort_by_key(|(ts, _, _)| *ts);
        if let Some((_, first_additions, prompt_summary)) = history.first() {
            let total_touches = history.len();
            let total_additions: u32 = history.iter().map(|(_, a, _)| a).sum();

            // Heuristic: first touch >80 lines, <=2 total touches, >70% of all additions
            if *first_additions > 80
                && total_touches <= 2
                && (*first_additions as f64 / total_additions.max(1) as f64) > 0.7
            {
                let severity = if *first_additions > 200 {
                    Severity::Critical
                } else {
                    Severity::Warning
                };

                flags.push(AnomalyFlag {
                    severity,
                    category: "Pre-written Code Suspected".into(),
                    description: format!(
                        "`{}` appeared with {} lines in a single prompt ({} total touches)",
                        file, first_additions, total_touches,
                    ),
                    evidence: format!(
                        "Prompt: \"{}\". {} of {} total lines in first touch ({:.0}%)",
                        truncate(prompt_summary, 100),
                        first_additions,
                        total_additions,
                        (*first_additions as f64 / total_additions.max(1) as f64) * 100.0,
                    ),
                });
            }
        }
    }

    flags
}

/// Detector 3: Source files committed with zero receipt trail.
fn detect_untracked_files(receipts: &[&Receipt]) -> Vec<AnomalyFlag> {
    let receipted_files: HashSet<String> = receipts
        .iter()
        .flat_map(|r| r.all_file_paths())
        .map(|f| make_rel(&f))
        .collect();

    let recently_added = get_recently_added_files();
    let untracked: Vec<String> = recently_added
        .into_iter()
        .filter(|f| !receipted_files.contains(f) && is_source_file(f))
        .collect();

    if untracked.is_empty() {
        return vec![];
    }

    let severity = if untracked.len() > 5 {
        Severity::Critical
    } else {
        Severity::Warning
    };

    vec![AnomalyFlag {
        severity,
        category: "Files Without Receipt Trail".into(),
        description: format!(
            "{} source file(s) were added with no AI receipt or iterative history",
            untracked.len(),
        ),
        evidence: format!(
            "Files: {}",
            untracked
                .iter()
                .take(10)
                .cloned()
                .collect::<Vec<_>>()
                .join(", "),
        ),
    }]
}

/// Detector 4: Same prompt_hash submitted multiple times (rehearsed prompts).
fn detect_duplicate_prompt_hashes(receipts: &[&Receipt]) -> Vec<AnomalyFlag> {
    let mut hash_counts: HashMap<&str, usize> = HashMap::new();
    for r in receipts {
        if !r.prompt_hash.is_empty() {
            *hash_counts.entry(r.prompt_hash.as_str()).or_insert(0) += 1;
        }
    }

    let duplicates: Vec<(&str, usize)> = hash_counts
        .into_iter()
        .filter(|(_, count)| *count > 1)
        .collect();

    if duplicates.is_empty() {
        return vec![];
    }

    let total_dupes: usize = duplicates.iter().map(|(_, c)| c).sum();
    let severity = if total_dupes > 10 {
        Severity::Critical
    } else if total_dupes > 3 {
        Severity::Warning
    } else {
        Severity::Info
    };

    vec![AnomalyFlag {
        severity,
        category: "Duplicate Prompt Hashes".into(),
        description: format!(
            "{} unique prompt(s) submitted more than once ({} total duplicated)",
            duplicates.len(),
            total_dupes,
        ),
        evidence: format!(
            "Hashes: {}",
            duplicates
                .iter()
                .take(5)
                .map(|(h, c)| format!("{}.. ({}x)", util::short_sha(h), c))
                .collect::<Vec<_>>()
                .join(", "),
        ),
    }]
}

/// Detector 5: Commit touches many files but few have receipt coverage.
fn detect_batch_commits(entries: &[audit::AuditEntry], receipts: &[&Receipt]) -> Vec<AnomalyFlag> {
    let mut flags = Vec::new();
    let _ = receipts; // receipts available if needed in future

    for entry in entries {
        if entry.commit_sha == "uncommitted" {
            continue;
        }

        let files_in_commit = count_files_in_commit(&entry.commit_sha);
        let receipted_files: HashSet<String> = entry
            .receipts
            .iter()
            .flat_map(|r| r.all_file_paths())
            .map(|f| make_rel(&f))
            .collect();

        let coverage = if files_in_commit > 0 {
            receipted_files.len() as f64 / files_in_commit as f64
        } else {
            1.0
        };

        if files_in_commit > 5 && coverage < 0.30 {
            let severity = if files_in_commit > 15 && coverage < 0.10 {
                Severity::Critical
            } else {
                Severity::Warning
            };

            flags.push(AnomalyFlag {
                severity,
                category: "Batch Commit".into(),
                description: format!(
                    "Commit {} changed {} files but only {} ({:.0}%) have receipt coverage",
                    util::short_sha(&entry.commit_sha),
                    files_in_commit,
                    receipted_files.len(),
                    coverage * 100.0,
                ),
                evidence: format!(
                    "Commit: \"{}\". Date: {}",
                    entry.commit_message, entry.commit_date,
                ),
            });
        }
    }

    flags
}

/// Detector 6: Very short prompt producing disproportionately large output.
fn detect_unusual_session_patterns(
    receipts: &[&Receipt],
    timeline: &[TimelineEntry],
) -> Vec<AnomalyFlag> {
    let mut flags = Vec::new();

    // Per-receipt: <60s duration producing >50 lines at >2 lines/sec
    for r in receipts {
        let duration = match r.prompt_duration_secs {
            Some(d) if d > 0 => d,
            _ => continue,
        };
        let additions = r.effective_total_additions();

        if duration < 60 && additions > 50 {
            let lps = additions as f64 / duration as f64;
            if lps > 2.0 {
                flags.push(AnomalyFlag {
                    severity: Severity::Warning,
                    category: "Unusual Session Pattern".into(),
                    description: format!(
                        "Prompt produced {} lines in {}s ({:.1} lines/sec)",
                        additions, duration, lps,
                    ),
                    evidence: format!(
                        "Prompt: \"{}\". Model: {}",
                        truncate(&r.prompt_summary, 100),
                        model_classifier::display_name(&r.model),
                    ),
                });
            }
        }
    }

    // Aggregate: very few prompts with very large total output
    let total_prompts = timeline.len();
    let total_additions: u32 = timeline.iter().map(|t| t.additions).sum();
    if total_prompts > 0 && total_prompts <= 3 && total_additions > 500 {
        flags.push(AnomalyFlag {
            severity: Severity::Warning,
            category: "Unusual Session Pattern".into(),
            description: format!(
                "Only {} prompt(s) produced {} total lines",
                total_prompts, total_additions,
            ),
            evidence: format!(
                "Average: {:.0} lines per prompt",
                total_additions as f64 / total_prompts as f64,
            ),
        });
    }

    flags
}

/// Detector 7: Suspiciously long gaps between consecutive prompts during the hackathon.
fn detect_time_gaps(timeline: &[TimelineEntry], start: DateTime<Utc>) -> Vec<AnomalyFlag> {
    let mut flags = Vec::new();

    let within: Vec<&TimelineEntry> = timeline.iter().filter(|t| t.within_window).collect();
    if within.len() < 2 {
        return flags;
    }

    // Gap between hackathon start and first prompt
    if let Some(first) = within.first() {
        let gap = (first.timestamp - start).num_seconds().max(0) as u64;
        if gap > 7200 {
            flags.push(AnomalyFlag {
                severity: Severity::Info,
                category: "Late Start".into(),
                description: format!(
                    "First AI prompt was {} after hackathon start",
                    session_stats::format_duration(gap),
                ),
                evidence: format!(
                    "Hackathon start: {}. First prompt: {}",
                    start.format("%H:%M:%S UTC"),
                    first.timestamp.format("%H:%M:%S UTC"),
                ),
            });
        }
    }

    // Inter-prompt gaps
    for window in within.windows(2) {
        let gap = (window[1].timestamp - window[0].timestamp)
            .num_seconds()
            .max(0) as u64;

        if gap > 5400 {
            let severity = if gap > 10800 {
                Severity::Warning
            } else {
                Severity::Info
            };

            flags.push(AnomalyFlag {
                severity,
                category: "Activity Gap".into(),
                description: format!(
                    "{} gap between prompts during hackathon window",
                    session_stats::format_duration(gap),
                ),
                evidence: format!(
                    "From {} to {}. Prompt before gap: \"{}\"",
                    window[0].timestamp.format("%H:%M:%S"),
                    window[1].timestamp.format("%H:%M:%S"),
                    truncate(&window[0].prompt_summary, 60),
                ),
            });
        }
    }

    flags
}

// ---------------------------------------------------------------------------
// Integrity score
// ---------------------------------------------------------------------------

fn calculate_integrity_score(anomalies: &[AnomalyFlag]) -> u32 {
    let total_deduction: f64 = anomalies.iter().map(|a| a.severity.weight()).sum();
    // Exponential decay: score = 100 * e^(-deduction/50)
    let raw = 100.0 * (-total_deduction / 50.0_f64).exp();
    raw.round().clamp(0.0, 100.0) as u32
}

fn score_label(score: u32) -> &'static str {
    if score >= 80 {
        "PASS"
    } else if score >= 50 {
        "REVIEW"
    } else {
        "FAIL"
    }
}

// ---------------------------------------------------------------------------
// File attribution builder
// ---------------------------------------------------------------------------

fn build_file_attribution(receipts: &[&Receipt]) -> Vec<FileAttribution> {
    let mut by_file: HashMap<String, FileAttribution> = HashMap::new();

    for r in receipts {
        let ts = r.prompt_submitted_at.unwrap_or(r.timestamp);
        for fc in r.all_file_changes() {
            let rel = make_rel(&fc.path);
            let entry = by_file.entry(rel.clone()).or_insert(FileAttribution {
                path: rel,
                total_lines: 0,
                ai_lines: 0,
                receipt_count: 0,
                first_touched: None,
            });
            entry.ai_lines += fc.additions;
            entry.receipt_count += 1;
            if entry.first_touched.is_none() || ts < entry.first_touched.unwrap() {
                entry.first_touched = Some(ts);
            }
        }
    }

    // Enrich with total line counts from actual files on disk
    for attr in by_file.values_mut() {
        if let Ok(content) = std::fs::read_to_string(&attr.path) {
            attr.total_lines = content.lines().count() as u32;
        }
    }

    let mut result: Vec<FileAttribution> = by_file.into_values().collect();
    result.sort_by_key(|a| std::cmp::Reverse(a.ai_lines));
    result
}

// ---------------------------------------------------------------------------
// Markdown section writers
// ---------------------------------------------------------------------------

fn write_header(md: &mut String, start: DateTime<Utc>, end: DateTime<Utc>, author: Option<&str>) {
    let _ = writeln!(md, "# Hackathon Fairness Report");
    let _ = writeln!(
        md,
        "> Generated by BlamePrompt v{}",
        env!("CARGO_PKG_VERSION")
    );
    let _ = writeln!(
        md,
        "> Hackathon window: {} -- {}",
        start.format("%Y-%m-%d %H:%M UTC"),
        end.format("%Y-%m-%d %H:%M UTC"),
    );
    if let Some(a) = author {
        let _ = writeln!(md, "> Participant: {}", a);
    }
    let _ = writeln!(
        md,
        "> Report date: {}\n",
        Utc::now().format("%Y-%m-%d %H:%M UTC")
    );
}

fn write_summary(
    md: &mut String,
    receipts: &[&Receipt],
    timeline: &[TimelineEntry],
    anomalies: &[AnomalyFlag],
    integrity_score: u32,
) {
    let _ = writeln!(md, "## 1. Summary\n");

    let within_count = timeline.iter().filter(|t| t.within_window).count();
    let outside_count = timeline.len() - within_count;
    let total_ai_lines: u32 = receipts.iter().map(|r| r.effective_total_additions()).sum();
    let total_accepted: u32 = receipts.iter().filter_map(|r| r.accepted_lines).sum();
    let total_overridden: u32 = receipts.iter().filter_map(|r| r.overridden_lines).sum();
    let total_cost: f64 = receipts.iter().map(|r| r.cost_usd).sum();
    let unique_files: HashSet<String> = receipts
        .iter()
        .flat_map(|r| r.all_file_paths().into_iter().map(|f| make_rel(&f)))
        .collect();

    // Session timing
    let stats = session_stats::calculate(receipts);

    let _ = writeln!(md, "| Metric | Value |");
    let _ = writeln!(md, "|--------|-------|");
    let _ = writeln!(
        md,
        "| **Integrity Score** | **{}/100 ({})** |",
        integrity_score,
        score_label(integrity_score),
    );
    let _ = writeln!(md, "| Total AI prompts | {} |", timeline.len());
    let _ = writeln!(md, "| Prompts within window | {} |", within_count);
    if outside_count > 0 {
        let _ = writeln!(md, "| Prompts outside window | **{}** |", outside_count);
    }
    let _ = writeln!(md, "| AI-generated lines | {} |", total_ai_lines);
    if total_accepted + total_overridden > 0 {
        let rate = total_accepted as f64 / (total_accepted + total_overridden) as f64 * 100.0;
        let _ = writeln!(
            md,
            "| Acceptance rate | {:.0}% ({} accepted, {} overridden) |",
            rate, total_accepted, total_overridden,
        );
    }
    let _ = writeln!(md, "| Files modified | {} |", unique_files.len());
    if stats.wall_clock_secs > 0 {
        let _ = writeln!(
            md,
            "| Active coding time | {} |",
            session_stats::format_duration(stats.wall_clock_secs),
        );
    }
    let _ = writeln!(md, "| Estimated AI cost | ${:.2} |", total_cost);
    let _ = writeln!(
        md,
        "| Anomalies detected | {} ({} critical, {} warning, {} info) |",
        anomalies.len(),
        anomalies
            .iter()
            .filter(|a| a.severity == Severity::Critical)
            .count(),
        anomalies
            .iter()
            .filter(|a| a.severity == Severity::Warning)
            .count(),
        anomalies
            .iter()
            .filter(|a| a.severity == Severity::Info)
            .count(),
    );
    let _ = writeln!(md);
}

fn write_timeline(md: &mut String, timeline: &[TimelineEntry]) {
    let _ = writeln!(md, "## 2. Timeline\n");

    if timeline.is_empty() {
        let _ = writeln!(md, "No prompts recorded.\n");
        return;
    }

    let _ = writeln!(
        md,
        "| # | Time | In Window | Duration | Model | Lines +/- | Files | Prompt |"
    );
    let _ = writeln!(
        md,
        "|---|------|-----------|----------|-------|-----------|-------|--------|"
    );

    for (i, entry) in timeline.iter().enumerate() {
        let window = if entry.within_window { "Yes" } else { "**NO**" };
        let duration = entry
            .duration_secs
            .map(session_stats::format_duration)
            .unwrap_or_else(|| "-".into());
        let files = if entry.files_touched.len() <= 2 {
            entry.files_touched.join(", ")
        } else {
            format!("{} files", entry.files_touched.len())
        };
        let prompt: String = entry.prompt_summary.chars().take(60).collect();

        let _ = writeln!(
            md,
            "| {} | {} | {} | {} | {} | +{} -{} | {} | {} |",
            i + 1,
            entry.timestamp.format("%H:%M:%S"),
            window,
            duration,
            entry.model,
            entry.additions,
            entry.deletions,
            files,
            prompt,
        );
    }
    let _ = writeln!(md);
}

fn write_code_attribution(
    md: &mut String,
    receipts: &[&Receipt],
    file_attribution: &[FileAttribution],
) {
    let _ = writeln!(md, "## 3. Code Attribution\n");

    let total_ai: u32 = receipts.iter().map(|r| r.effective_total_additions()).sum();
    let total_accepted: u32 = receipts.iter().filter_map(|r| r.accepted_lines).sum();
    let total_overridden: u32 = receipts.iter().filter_map(|r| r.overridden_lines).sum();

    let _ = writeln!(md, "### Overall\n");
    let _ = writeln!(md, "- **AI-generated lines**: {}", total_ai);
    if total_accepted + total_overridden > 0 {
        let rate = total_accepted as f64 / (total_accepted + total_overridden) as f64 * 100.0;
        let _ = writeln!(
            md,
            "- **Accepted unchanged**: {} ({:.0}%)",
            total_accepted, rate,
        );
        let _ = writeln!(
            md,
            "- **Human-edited after AI**: {} ({:.0}%)",
            total_overridden,
            100.0 - rate,
        );
    }
    let _ = writeln!(md);

    if !file_attribution.is_empty() {
        let _ = writeln!(md, "### Per-File Breakdown\n");
        let _ = writeln!(
            md,
            "| File | Total Lines | AI Lines | AI % | Receipts | First Touched |"
        );
        let _ = writeln!(
            md,
            "|------|------------|----------|------|----------|--------------|"
        );

        for attr in file_attribution.iter().take(30) {
            let ai_pct = if attr.total_lines > 0 {
                (attr.ai_lines as f64 / attr.total_lines as f64) * 100.0
            } else if attr.ai_lines > 0 {
                100.0
            } else {
                0.0
            };
            let first = attr
                .first_touched
                .map(|t| t.format("%H:%M:%S").to_string())
                .unwrap_or_else(|| "-".into());

            let _ = writeln!(
                md,
                "| {} | {} | {} | {:.0}% | {} | {} |",
                attr.path, attr.total_lines, attr.ai_lines, ai_pct, attr.receipt_count, first,
            );
        }
        let _ = writeln!(md);
    }
}

fn write_anomaly_flags(md: &mut String, anomalies: &[AnomalyFlag]) {
    let _ = writeln!(md, "## 4. Anomaly Flags\n");

    if anomalies.is_empty() {
        let _ = writeln!(
            md,
            "No anomalies detected. All activity appears consistent with genuine hackathon work.\n"
        );
        return;
    }

    // Sort: CRITICAL first, then WARNING, then INFO
    let mut sorted: Vec<&AnomalyFlag> = anomalies.iter().collect();
    sorted.sort_by_key(|a| match a.severity {
        Severity::Critical => 0,
        Severity::Warning => 1,
        Severity::Info => 2,
    });

    for (i, flag) in sorted.iter().enumerate() {
        let _ = writeln!(
            md,
            "### Flag {}: [{}] {}\n",
            i + 1,
            flag.severity.label(),
            flag.category,
        );
        let _ = writeln!(md, "**{}**\n", flag.description);
        let _ = writeln!(md, "> Evidence: {}\n", flag.evidence);
    }
}

fn write_integrity_assessment(md: &mut String, anomalies: &[AnomalyFlag], score: u32) {
    let _ = writeln!(md, "## 5. Integrity Assessment\n");
    let _ = writeln!(md, "### Score Breakdown\n");
    let _ = writeln!(md, "| Factor | Count | Deduction |");
    let _ = writeln!(md, "|--------|-------|-----------|");

    let critical = anomalies
        .iter()
        .filter(|a| a.severity == Severity::Critical)
        .count();
    let warning = anomalies
        .iter()
        .filter(|a| a.severity == Severity::Warning)
        .count();
    let info = anomalies
        .iter()
        .filter(|a| a.severity == Severity::Info)
        .count();

    if critical > 0 {
        let _ = writeln!(
            md,
            "| CRITICAL anomalies | {} | -{:.0} pts ({} each) |",
            critical,
            critical as f64 * Severity::Critical.weight(),
            Severity::Critical.weight(),
        );
    }
    if warning > 0 {
        let _ = writeln!(
            md,
            "| WARNING anomalies | {} | -{:.0} pts ({} each) |",
            warning,
            warning as f64 * Severity::Warning.weight(),
            Severity::Warning.weight(),
        );
    }
    if info > 0 {
        let _ = writeln!(
            md,
            "| INFO anomalies | {} | -{:.0} pts ({} each) |",
            info,
            info as f64 * Severity::Info.weight(),
            Severity::Info.weight(),
        );
    }
    let _ = writeln!(md);

    let total_deduction: f64 = anomalies.iter().map(|a| a.severity.weight()).sum();
    let _ = writeln!(md, "**Total deduction weight**: {:.0}", total_deduction);
    let _ = writeln!(md, "**Formula**: `score = 100 * e^(-deduction/50)`");
    let _ = writeln!(md, "**Final Score**: **{}/100**\n", score);

    let _ = writeln!(md, "### Conclusion\n");
    if score >= 80 {
        let _ = writeln!(
            md,
            "The coding activity during this hackathon appears **consistent with genuine work**. \
             Prompt progression shows iterative development, and AI tool usage aligns with the hackathon timeframe.\n"
        );
    } else if score >= 50 {
        let _ = writeln!(
            md,
            "The coding activity shows **some irregularities** that warrant review by an organizer. \
             The flagged anomalies above should be examined manually before confirming results.\n"
        );
    } else {
        let _ = writeln!(
            md,
            "The coding activity shows **significant irregularities** that suggest the code may not have been \
             entirely produced during the hackathon window. \
             Organizers should review the flagged anomalies carefully.\n"
        );
    }
}

fn write_footer(md: &mut String) {
    let _ = writeln!(md, "---");
    let _ = writeln!(
        md,
        "*Generated by BlamePrompt v{} -- Hackathon Fair Play Verification*",
        env!("CARGO_PKG_VERSION"),
    );
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn parse_datetime(s: &str) -> Option<DateTime<Utc>> {
    // Try RFC 3339 first, then fallback to date-only with midnight
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }
    if let Ok(d) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return d
            .and_hms_opt(0, 0, 0)
            .map(|dt| DateTime::<Utc>::from_naive_utc_and_offset(dt, Utc));
    }
    None
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max.saturating_sub(3)).collect();
        format!("{}...", truncated)
    }
}

fn make_rel(path: &str) -> String {
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    util::make_relative(path, &cwd)
}

fn is_source_file(path: &str) -> bool {
    let extensions = [
        ".rs", ".py", ".js", ".ts", ".tsx", ".jsx", ".go", ".java", ".c", ".cpp", ".h", ".hpp",
        ".rb", ".swift", ".kt", ".cs", ".php", ".scala", ".ex", ".exs", ".zig", ".vue", ".svelte",
    ];
    extensions.iter().any(|ext| path.ends_with(ext))
}

fn get_recently_added_files() -> Vec<String> {
    std::process::Command::new("git")
        .args([
            "log",
            "--diff-filter=A",
            "--name-only",
            "--pretty=format:",
            "-50",
        ])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| {
            s.lines()
                .filter(|l| !l.is_empty())
                .map(String::from)
                .collect::<HashSet<_>>()
                .into_iter()
                .collect()
        })
        .unwrap_or_default()
}

fn count_files_in_commit(sha: &str) -> usize {
    std::process::Command::new("git")
        .args(["diff-tree", "--no-commit-id", "--name-only", "-r", sha])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.lines().filter(|l| !l.is_empty()).count())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_anomaly(severity: Severity) -> AnomalyFlag {
        AnomalyFlag {
            severity,
            category: "Test".into(),
            description: "test".into(),
            evidence: "test".into(),
        }
    }

    #[test]
    fn test_integrity_score_no_anomalies() {
        assert_eq!(calculate_integrity_score(&[]), 100);
    }

    #[test]
    fn test_integrity_score_one_warning() {
        let anomalies = vec![make_anomaly(Severity::Warning)];
        let score = calculate_integrity_score(&anomalies);
        assert!(score >= 80 && score <= 90, "score={}", score);
    }

    #[test]
    fn test_integrity_score_one_critical() {
        let anomalies = vec![make_anomaly(Severity::Critical)];
        let score = calculate_integrity_score(&anomalies);
        assert!(score >= 60 && score <= 75, "score={}", score);
    }

    #[test]
    fn test_integrity_score_multiple_criticals() {
        let anomalies = vec![
            make_anomaly(Severity::Critical),
            make_anomaly(Severity::Critical),
        ];
        let score = calculate_integrity_score(&anomalies);
        assert!(score < 50, "score={}", score);
    }

    #[test]
    fn test_time_window_violations() {
        let start = Utc::now() - chrono::Duration::hours(2);
        let end = Utc::now() - chrono::Duration::hours(1);

        let timeline = vec![TimelineEntry {
            timestamp: Utc::now(), // AFTER the window
            prompt_summary: "test".into(),
            duration_secs: None,
            model: "test".into(),
            files_touched: vec![],
            additions: 50,
            deletions: 0,
            within_window: false,
        }];

        let flags = detect_time_window_violations(&timeline, start, end);
        assert_eq!(flags.len(), 1);
        assert_eq!(flags[0].severity, Severity::Warning);
    }

    #[test]
    fn test_pre_written_code_detection() {
        let now = Utc::now();
        let r = Receipt {
            id: "test".into(),
            provider: "claude".into(),
            model: "opus".into(),
            session_id: "s1".into(),
            prompt_summary: "Create the entire app".into(),
            response_summary: None,
            prompt_hash: "hash1".into(),
            message_count: 2,
            cost_usd: 0.1,
            input_tokens: None,
            output_tokens: None,
            cache_read_tokens: None,
            cache_creation_tokens: None,
            timestamp: now,
            session_start: None,
            session_end: None,
            session_duration_secs: None,
            ai_response_time_secs: None,
            prompt_submitted_at: Some(now),
            prompt_duration_secs: Some(120),
            accepted_lines: None,
            overridden_lines: None,
            user: "test".into(),
            file_path: String::new(),
            line_range: (0, 0),
            files_changed: vec![crate::core::receipt::FileChange {
                path: "src/big_file.rs".into(),
                line_range: (1, 250),
                blob_hash: None,
                additions: 250,
                deletions: 0,
            }],
            parent_receipt_id: None,
            parent_session_id: None,
            is_continuation: None,
            continuation_depth: None,
            prompt_number: Some(1),
            total_additions: 250,
            total_deletions: 0,
            tools_used: vec![],
            mcp_servers: vec![],
            agents_spawned: vec![],
            subagent_activities: vec![],
            concurrent_tool_calls: None,
            user_decisions: vec![],
            conversation: None,
        };

        let receipts = vec![&r];
        let flags = detect_pre_written_code(&receipts);
        assert!(!flags.is_empty(), "Should flag 250-line single-touch file");
        assert_eq!(flags[0].severity, Severity::Critical);
    }

    #[test]
    fn test_no_false_positive_on_small_file() {
        let now = Utc::now();
        let r = Receipt {
            id: "test".into(),
            provider: "claude".into(),
            model: "opus".into(),
            session_id: "s1".into(),
            prompt_summary: "Add config".into(),
            response_summary: None,
            prompt_hash: "hash1".into(),
            message_count: 2,
            cost_usd: 0.01,
            input_tokens: None,
            output_tokens: None,
            cache_read_tokens: None,
            cache_creation_tokens: None,
            timestamp: now,
            session_start: None,
            session_end: None,
            session_duration_secs: None,
            ai_response_time_secs: None,
            prompt_submitted_at: Some(now),
            prompt_duration_secs: Some(60),
            accepted_lines: None,
            overridden_lines: None,
            user: "test".into(),
            file_path: String::new(),
            line_range: (0, 0),
            files_changed: vec![crate::core::receipt::FileChange {
                path: "src/config.rs".into(),
                line_range: (1, 30),
                blob_hash: None,
                additions: 30,
                deletions: 0,
            }],
            parent_receipt_id: None,
            parent_session_id: None,
            is_continuation: None,
            continuation_depth: None,
            prompt_number: Some(1),
            total_additions: 30,
            total_deletions: 0,
            tools_used: vec![],
            mcp_servers: vec![],
            agents_spawned: vec![],
            subagent_activities: vec![],
            concurrent_tool_calls: None,
            user_decisions: vec![],
            conversation: None,
        };

        let receipts = vec![&r];
        let flags = detect_pre_written_code(&receipts);
        assert!(flags.is_empty(), "30-line file should NOT be flagged");
    }

    #[test]
    fn test_duplicate_prompt_hashes() {
        let now = Utc::now();
        let make_receipt = |hash: &str| Receipt {
            id: Receipt::new_id(),
            provider: "claude".into(),
            model: "opus".into(),
            session_id: "s1".into(),
            prompt_summary: "test".into(),
            response_summary: None,
            prompt_hash: hash.into(),
            message_count: 1,
            cost_usd: 0.0,
            input_tokens: None,
            output_tokens: None,
            cache_read_tokens: None,
            cache_creation_tokens: None,
            timestamp: now,
            session_start: None,
            session_end: None,
            session_duration_secs: None,
            ai_response_time_secs: None,
            prompt_submitted_at: None,
            prompt_duration_secs: None,
            accepted_lines: None,
            overridden_lines: None,
            user: "test".into(),
            file_path: String::new(),
            line_range: (0, 0),
            files_changed: vec![],
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
        };

        let r1 = make_receipt("sha256:abc123");
        let r2 = make_receipt("sha256:abc123");
        let r3 = make_receipt("sha256:abc123");
        let r4 = make_receipt("sha256:different");

        let receipts = vec![&r1, &r2, &r3, &r4];
        let flags = detect_duplicate_prompt_hashes(&receipts);
        assert_eq!(flags.len(), 1);
        assert_eq!(flags[0].severity, Severity::Info); // 3 dupes → INFO (threshold: >3 for WARNING)
    }

    #[test]
    fn test_unusual_session_pattern() {
        let now = Utc::now();
        let r = Receipt {
            id: "test".into(),
            provider: "claude".into(),
            model: "opus".into(),
            session_id: "s1".into(),
            prompt_summary: "Do everything".into(),
            response_summary: None,
            prompt_hash: "hash".into(),
            message_count: 2,
            cost_usd: 0.0,
            input_tokens: None,
            output_tokens: None,
            cache_read_tokens: None,
            cache_creation_tokens: None,
            timestamp: now,
            session_start: None,
            session_end: None,
            session_duration_secs: None,
            ai_response_time_secs: None,
            prompt_submitted_at: Some(now),
            prompt_duration_secs: Some(30), // 30 seconds
            accepted_lines: None,
            overridden_lines: None,
            user: "test".into(),
            file_path: String::new(),
            line_range: (0, 0),
            files_changed: vec![],
            parent_receipt_id: None,
            parent_session_id: None,
            is_continuation: None,
            continuation_depth: None,
            prompt_number: Some(1),
            total_additions: 100, // 100 lines in 30s = 3.3 lines/sec
            total_deletions: 0,
            tools_used: vec![],
            mcp_servers: vec![],
            agents_spawned: vec![],
            subagent_activities: vec![],
            concurrent_tool_calls: None,
            user_decisions: vec![],
            conversation: None,
        };

        let timeline = vec![]; // empty timeline so aggregate check doesn't fire
        let flags = detect_unusual_session_patterns(&[&r], &timeline);
        assert!(!flags.is_empty(), "100 lines in 30s should be flagged");
    }

    #[test]
    fn test_timeline_ordering() {
        let now = Utc::now();
        let start = now - chrono::Duration::hours(2);
        let end = now + chrono::Duration::hours(2);

        let make_receipt = |mins_ago: i64| Receipt {
            id: Receipt::new_id(),
            provider: "claude".into(),
            model: "opus".into(),
            session_id: "s1".into(),
            prompt_summary: format!("prompt at -{}", mins_ago),
            response_summary: None,
            prompt_hash: "h".into(),
            message_count: 1,
            cost_usd: 0.0,
            input_tokens: None,
            output_tokens: None,
            cache_read_tokens: None,
            cache_creation_tokens: None,
            timestamp: now - chrono::Duration::minutes(mins_ago),
            session_start: None,
            session_end: None,
            session_duration_secs: None,
            ai_response_time_secs: None,
            prompt_submitted_at: Some(now - chrono::Duration::minutes(mins_ago)),
            prompt_duration_secs: None,
            accepted_lines: None,
            overridden_lines: None,
            user: "test".into(),
            file_path: String::new(),
            line_range: (0, 0),
            files_changed: vec![],
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
        };

        let r1 = make_receipt(30); // 30 min ago
        let r2 = make_receipt(60); // 60 min ago
        let r3 = make_receipt(10); // 10 min ago

        let receipts = vec![&r1, &r2, &r3];
        let timeline = build_timeline(&receipts, start, end);

        // Should be sorted chronologically (oldest first)
        assert!(timeline[0].timestamp <= timeline[1].timestamp);
        assert!(timeline[1].timestamp <= timeline[2].timestamp);
    }

    #[test]
    fn test_is_source_file() {
        assert!(is_source_file("src/main.rs"));
        assert!(is_source_file("app.py"));
        assert!(is_source_file("index.tsx"));
        assert!(!is_source_file("README.md"));
        assert!(!is_source_file("config.json"));
        assert!(!is_source_file("image.png"));
    }

    #[test]
    fn test_time_gap_detection() {
        let now = Utc::now();
        let start = now - chrono::Duration::hours(4);

        let timeline = vec![
            TimelineEntry {
                timestamp: now - chrono::Duration::hours(3),
                prompt_summary: "first".into(),
                duration_secs: None,
                model: "test".into(),
                files_touched: vec![],
                additions: 10,
                deletions: 0,
                within_window: true,
            },
            TimelineEntry {
                timestamp: now, // 3 hour gap
                prompt_summary: "second".into(),
                duration_secs: None,
                model: "test".into(),
                files_touched: vec![],
                additions: 10,
                deletions: 0,
                within_window: true,
            },
        ];

        let flags = detect_time_gaps(&timeline, start);
        assert!(!flags.is_empty(), "3-hour gap should be flagged");
    }

    #[test]
    fn test_parse_datetime_rfc3339() {
        use chrono::Timelike;
        let dt = parse_datetime("2026-02-26T09:00:00Z");
        assert!(dt.is_some());
        assert_eq!(dt.unwrap().hour(), 9);
    }

    #[test]
    fn test_parse_datetime_date_only() {
        use chrono::Timelike;
        let dt = parse_datetime("2026-02-26");
        assert!(dt.is_some());
        assert_eq!(dt.unwrap().hour(), 0);
    }
}
