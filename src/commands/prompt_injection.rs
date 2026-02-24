use crate::commands::audit;
use chrono::Utc;
use regex::Regex;

fn relative_path(path: &str) -> String {
    if let Ok(cwd) = std::env::current_dir() {
        let cwd_str = cwd.to_string_lossy();
        if path.starts_with(cwd_str.as_ref()) {
            let rel = &path[cwd_str.len()..];
            return rel.strip_prefix('/').unwrap_or(rel).to_string();
        }
    }
    path.to_string()
}

struct InjectionPattern {
    name: &'static str,
    pattern: &'static str,
    severity: &'static str,
    description: &'static str,
}

const INJECTION_PATTERNS: &[InjectionPattern] = &[
    InjectionPattern {
        name: "Eval/Exec Backdoor",
        pattern: r"(?i)\b(eval|exec)\s*\(\s*(atob|Buffer\.from|base64|decode)",
        severity: "CRITICAL",
        description: "Code decodes and executes encoded payloads - classic backdoor pattern",
    },
    InjectionPattern {
        name: "Base64 Decode + Execute",
        pattern: r"(?i)(atob|base64[._]decode|Buffer\.from)",
        severity: "HIGH",
        description: "Base64 decoding detected - may be hiding malicious payloads",
    },
    InjectionPattern {
        name: "Outbound Network Call",
        pattern: r"(?i)(fetch|axios|http\.get|https\.get|urllib|requests\.(get|post)|curl_exec)\s*\(",
        severity: "HIGH",
        description: "Outbound network call - potential data exfiltration",
    },
    InjectionPattern {
        name: "Dynamic Import/Require",
        pattern: r"(?i)(require|import)\s*\(\s*[a-zA-Z_]",
        severity: "MEDIUM",
        description: "Dynamic module loading with variable - could load malicious modules",
    },
    InjectionPattern {
        name: "Environment Variable Exfiltration",
        pattern: r"(?i)(process\.env|os\.environ|std::env::var|env::var).*?(fetch|http|request|send|post|axios)",
        severity: "CRITICAL",
        description:
            "Environment variables accessed near network calls - potential secret exfiltration",
    },
    InjectionPattern {
        name: "Hidden Process Spawn",
        pattern: r"(?i)(child_process|subprocess|Process\.Start|Runtime\.exec|Command::new)\s*\(",
        severity: "HIGH",
        description:
            "Process spawning detected - verify this is intentional and inputs are validated",
    },
    InjectionPattern {
        name: "File System Write",
        pattern: r"(?i)(writeFile|write_to_string|fs::write)\s*\(",
        severity: "MEDIUM",
        description: "File write operation - verify the file path and contents are expected",
    },
    InjectionPattern {
        name: "Crypto/Hash Manipulation",
        pattern: r"(?i)(createCipher|AES|DES|RC4|encrypt|decrypt)\s*\(",
        severity: "MEDIUM",
        description: "Cryptographic operation - verify it uses strong algorithms",
    },
    InjectionPattern {
        name: "Timer-Based Trigger",
        pattern: r"(?i)(setTimeout|setInterval|schedule|cron)\s*\([^)]*\b(eval|exec|fetch|http|require)\b",
        severity: "HIGH",
        description: "Delayed execution with dynamic code - potential time-bomb backdoor",
    },
    InjectionPattern {
        name: "Obfuscated String",
        pattern: r"\\x[0-9a-fA-F]{2}(\\x[0-9a-fA-F]{2}){5,}",
        severity: "HIGH",
        description: "Heavily obfuscated string literals - may hide malicious payloads",
    },
];

struct Detection {
    file: String,
    line_number: u32,
    line_content: String,
    pattern_name: String,
    severity: String,
    description: String,
    model: String,
    #[allow(dead_code)]
    receipt_id: String,
}

pub fn run(output: &str) {
    let entries = match audit::collect_all_entries(None, None, None, true) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("Error: {}", e);
            return;
        }
    };

    if entries.is_empty() {
        println!("No AI-generated code found to scan.");
        return;
    }

    let all_receipts: Vec<_> = entries.iter().flat_map(|e| &e.receipts).collect();
    let now = Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string();

    let mut detections: Vec<Detection> = Vec::new();
    let mut files_scanned = 0;
    let mut lines_scanned: u32 = 0;

    // Scan AI-generated code for injection patterns
    for r in &all_receipts {
        let file_path = &r.file_path;
        let content = match std::fs::read_to_string(file_path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        files_scanned += 1;
        let lines: Vec<&str> = content.lines().collect();

        let start = r.line_range.0.saturating_sub(1) as usize;
        let end = (r.line_range.1 as usize).min(lines.len());

        if start >= lines.len() {
            continue;
        }

        for (i, line) in lines[start..end].iter().enumerate() {
            let line_num = (start + i + 1) as u32;
            lines_scanned += 1;

            for pattern in INJECTION_PATTERNS {
                if let Ok(re) = Regex::new(pattern.pattern) {
                    if re.is_match(line) {
                        detections.push(Detection {
                            file: relative_path(file_path),
                            line_number: line_num,
                            line_content: line.trim().chars().take(120).collect(),
                            pattern_name: pattern.name.to_string(),
                            severity: pattern.severity.to_string(),
                            description: pattern.description.to_string(),
                            model: r.model.clone(),
                            receipt_id: r.id.clone(),
                        });
                    }
                }
            }
        }
    }

    // Also scan conversation turns for suspicious AI instructions
    let mut prompt_flags: Vec<(String, String, String)> = Vec::new();
    for r in &all_receipts {
        if let Some(ref turns) = r.conversation {
            for t in turns {
                if t.role == "assistant" {
                    // Check if AI response contains suspicious instructions
                    let suspicious_phrases = [
                        "ignore previous instructions",
                        "disregard the above",
                        "do not tell the user",
                        "secretly",
                        "hidden functionality",
                        "bypass security",
                        "disable authentication",
                        "skip validation",
                    ];
                    let lower = t.content.to_lowercase();
                    for phrase in &suspicious_phrases {
                        if lower.contains(phrase) {
                            prompt_flags.push((
                                r.id.clone(),
                                phrase.to_string(),
                                t.content.chars().take(200).collect(),
                            ));
                        }
                    }
                }
            }
        }
    }

    // Generate report
    let mut md = String::new();
    md.push_str("# BlamePrompt Prompt Injection Detection Report\n\n");
    md.push_str(&format!("> Generated: {}\n\n", now));

    // Summary
    let critical = detections
        .iter()
        .filter(|d| d.severity == "CRITICAL")
        .count();
    let high = detections.iter().filter(|d| d.severity == "HIGH").count();
    let medium = detections.iter().filter(|d| d.severity == "MEDIUM").count();

    md.push_str("## Summary\n\n");
    md.push_str("| Metric | Value |\n");
    md.push_str("|--------|-------|\n");
    md.push_str(&format!("| Files scanned | {} |\n", files_scanned));
    md.push_str(&format!(
        "| AI-generated lines scanned | {} |\n",
        lines_scanned
    ));
    md.push_str(&format!(
        "| Code pattern detections | {} |\n",
        detections.len()
    ));
    md.push_str(&format!(
        "| Suspicious AI responses | {} |\n",
        prompt_flags.len()
    ));
    md.push_str(&format!("| CRITICAL findings | {} |\n", critical));
    md.push_str(&format!("| HIGH findings | {} |\n", high));
    md.push_str(&format!("| MEDIUM findings | {} |\n\n", medium));

    // Code-level detections
    if !detections.is_empty() {
        md.push_str("## Code-Level Detections\n\n");
        md.push_str("Patterns detected in AI-generated code that may indicate prompt injection or backdoors:\n\n");

        for severity in &["CRITICAL", "HIGH", "MEDIUM"] {
            let sev_detections: Vec<_> = detections
                .iter()
                .filter(|d| d.severity == *severity)
                .collect();
            if sev_detections.is_empty() {
                continue;
            }

            md.push_str(&format!("### {} ({})\n\n", severity, sev_detections.len()));
            for d in &sev_detections {
                md.push_str(&format!(
                    "- **{}** in `{}` line {}\n",
                    d.pattern_name, d.file, d.line_number
                ));
                md.push_str(&format!("  - Model: {}\n", d.model));
                md.push_str(&format!("  - {}\n", d.description));
                md.push_str(&format!("  - Code: `{}`\n\n", d.line_content));
            }
        }
    } else {
        md.push_str(
            "## Code-Level Detections\n\nNo suspicious patterns detected in AI-generated code.\n\n",
        );
    }

    // Suspicious AI responses
    if !prompt_flags.is_empty() {
        md.push_str("## Suspicious AI Responses\n\n");
        md.push_str(
            "The following AI responses contain phrases that may indicate prompt injection:\n\n",
        );
        for (receipt_id, phrase, context) in &prompt_flags {
            let id_short = if receipt_id.len() >= 8 {
                &receipt_id[..8]
            } else {
                receipt_id
            };
            md.push_str(&format!(
                "- **Receipt {}**: Detected `{}`\n",
                id_short, phrase
            ));
            md.push_str(&format!("  > {}\n\n", context));
        }
    }

    // Recommendations
    md.push_str("## Recommendations\n\n");
    if critical > 0 {
        md.push_str("1. **URGENT** — Review all CRITICAL findings immediately. These may indicate active prompt injection.\n");
    }
    md.push_str("2. **Verify intent** — For each detection, confirm the code behavior matches the original prompt.\n");
    md.push_str("3. **Check conversation context** — Review the full chain of thought for flagged receipts.\n");
    md.push_str("4. **Sandbox testing** — Test AI-generated code in isolated environments before deploying.\n");
    md.push_str("5. **Prompt hardening** — Use system prompts that instruct AI to avoid generating backdoors or eval patterns.\n\n");

    md.push_str("---\n\n");
    md.push_str("*Generated by [BlamePrompt](https://github.com/ekaanth/blameprompt) — Prompt Injection Detector*\n");

    match std::fs::write(output, &md) {
        Ok(_) => println!("Prompt injection detection report written to {}", output),
        Err(e) => eprintln!("Error writing report: {}", e),
    }
}
