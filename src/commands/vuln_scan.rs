use crate::commands::audit;
use chrono::Utc;
use regex::Regex;
use std::collections::HashMap;

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

struct VulnPattern {
    name: &'static str,
    pattern: &'static str,
    severity: &'static str,
    cwe: &'static str,
    description: &'static str,
    fix: &'static str,
    file_extensions: &'static [&'static str], // empty = all files
}

const VULN_PATTERNS: &[VulnPattern] = &[
    // Command Injection
    VulnPattern {
        name: "Command Injection",
        pattern: r"(?i)(exec|system|popen|subprocess\.call|subprocess\.run|os\.system|child_process\.exec)\s*\(",
        severity: "CRITICAL",
        cwe: "CWE-78",
        description: "Potential OS command injection via dynamic command execution",
        fix: "Use parameterized commands, avoid shell=True, validate and sanitize inputs",
        file_extensions: &[],
    },
    // SQL Injection
    VulnPattern {
        name: "SQL Injection",
        pattern: r#"(?i)(execute|query|raw)\s*\(\s*f["\']|format!\s*\(\s*"[^"]*(?:SELECT|INSERT|UPDATE|DELETE|DROP)"#,
        severity: "CRITICAL",
        cwe: "CWE-89",
        description: "Potential SQL injection via string concatenation/interpolation in queries",
        fix: "Use parameterized queries or prepared statements",
        file_extensions: &[],
    },
    // XSS
    VulnPattern {
        name: "Cross-Site Scripting (XSS)",
        pattern: r"(?i)(innerHTML|outerHTML|document\.write|v-html)\s*=",
        severity: "HIGH",
        cwe: "CWE-79",
        description: "Potential XSS via unsafe HTML injection",
        fix: "Use textContent instead of innerHTML, sanitize HTML before rendering",
        file_extensions: &["js", "ts", "jsx", "tsx", "vue", "html"],
    },
    // Path Traversal
    VulnPattern {
        name: "Path Traversal",
        pattern: r"(?i)(open|read_file|readFile|createReadStream)\s*\([^)]*\+",
        severity: "HIGH",
        cwe: "CWE-22",
        description: "Potential path traversal via unsanitized file path construction",
        fix: "Validate paths, use path.resolve() and check against allowed directories",
        file_extensions: &[],
    },
    // Hardcoded Credentials
    VulnPattern {
        name: "Hardcoded Credentials",
        pattern: r#"(?i)(password|passwd|secret|api_key|apikey)\s*=\s*["'][^"']{8,}["']"#,
        severity: "HIGH",
        cwe: "CWE-798",
        description: "Hardcoded credentials found in source code",
        fix: "Use environment variables or a secrets manager",
        file_extensions: &[],
    },
    // Insecure Deserialization
    VulnPattern {
        name: "Insecure Deserialization",
        pattern: r"(?i)(pickle\.loads?|yaml\.load\s*\((?!.*Loader)|eval\s*\(|unserialize\s*\()",
        severity: "HIGH",
        cwe: "CWE-502",
        description: "Potentially unsafe deserialization that could lead to RCE",
        fix: "Use safe deserialization (yaml.safe_load, json instead of pickle, etc.)",
        file_extensions: &[],
    },
    // Insecure Random
    VulnPattern {
        name: "Insecure Randomness",
        pattern: r"(?i)(Math\.random|random\.random|rand\(\)|srand\()",
        severity: "MEDIUM",
        cwe: "CWE-330",
        description: "Use of non-cryptographic random number generator for potentially security-sensitive operations",
        fix: "Use crypto.getRandomValues(), secrets module, or OS-level CSPRNG",
        file_extensions: &[],
    },
    // Unsafe Regex
    VulnPattern {
        name: "ReDoS (Regex Denial of Service)",
        pattern: r"(?i)(Regex::new|re\.compile|new RegExp)\s*\([^)]*(\.\*\+|\.\+\*|(\[.*\])\+\+)",
        severity: "MEDIUM",
        cwe: "CWE-1333",
        description: "Potentially catastrophic backtracking in regular expression",
        fix: "Simplify regex patterns, add timeouts, use atomic groups",
        file_extensions: &[],
    },
    // Unsafe eval/exec
    VulnPattern {
        name: "Dynamic Code Execution",
        pattern: r"\b(eval|exec)\s*\(",
        severity: "CRITICAL",
        cwe: "CWE-95",
        description: "Dynamic code execution that could allow arbitrary code injection",
        fix: "Avoid eval/exec; use safe alternatives like JSON.parse() or AST-based approaches",
        file_extensions: &[],
    },
    // Missing authentication check
    VulnPattern {
        name: "Potentially Unprotected Endpoint",
        pattern: r#"(?i)(app\.(get|post|put|delete|patch)|router\.(get|post|put|delete))\s*\(\s*['"][^'"]*['"]"#,
        severity: "LOW",
        cwe: "CWE-306",
        description: "HTTP endpoint defined — verify authentication middleware is applied",
        fix: "Ensure authentication/authorization middleware protects all sensitive endpoints",
        file_extensions: &["js", "ts", "py", "rs"],
    },
];

struct Finding {
    file: String,
    line_number: u32,
    line_content: String,
    vuln_name: String,
    severity: String,
    cwe: String,
    description: String,
    fix: String,
    model: String,
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

    let mut findings: Vec<Finding> = Vec::new();
    let mut files_scanned = 0;
    let mut lines_scanned: u32 = 0;

    // Scan each AI-generated file region
    for r in &all_receipts {
        for fc in r.all_file_changes() {
            let file_path = &fc.path;
            let content = match std::fs::read_to_string(file_path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            files_scanned += 1;
            let lines: Vec<&str> = content.lines().collect();

            // Only scan lines within the AI-generated range
            let start = fc.line_range.0.saturating_sub(1) as usize;
            let end = (fc.line_range.1 as usize).min(lines.len());

            if start >= lines.len() {
                continue;
            }

            let file_ext = file_path.rsplit('.').next().unwrap_or("");

            for (i, line) in lines[start..end].iter().enumerate() {
                let line_num = (start + i + 1) as u32;
                lines_scanned += 1;

                for vuln in VULN_PATTERNS {
                    // Check file extension filter
                    if !vuln.file_extensions.is_empty() && !vuln.file_extensions.contains(&file_ext)
                    {
                        continue;
                    }

                    if let Ok(re) = Regex::new(vuln.pattern) {
                        if re.is_match(line) {
                            findings.push(Finding {
                                file: relative_path(file_path),
                                line_number: line_num,
                                line_content: line.trim().chars().take(120).collect(),
                                vuln_name: vuln.name.to_string(),
                                severity: vuln.severity.to_string(),
                                cwe: vuln.cwe.to_string(),
                                description: vuln.description.to_string(),
                                fix: vuln.fix.to_string(),
                                model: r.model.clone(),
                            });
                        }
                    }
                }
            }
        } // for fc in all_file_changes
    }

    // Generate report
    let mut md = String::new();
    md.push_str("# BlamePrompt Vulnerability Scan — AI-Generated Code\n\n");
    md.push_str(&format!("> Generated: {}\n\n", now));

    // Summary
    let critical = findings.iter().filter(|f| f.severity == "CRITICAL").count();
    let high = findings.iter().filter(|f| f.severity == "HIGH").count();
    let medium = findings.iter().filter(|f| f.severity == "MEDIUM").count();
    let low = findings.iter().filter(|f| f.severity == "LOW").count();

    md.push_str("## Summary\n\n");
    md.push_str("| Metric | Value |\n");
    md.push_str("|--------|-------|\n");
    md.push_str(&format!("| Files scanned | {} |\n", files_scanned));
    md.push_str(&format!(
        "| AI-generated lines scanned | {} |\n",
        lines_scanned
    ));
    md.push_str(&format!("| Total findings | {} |\n", findings.len()));
    md.push_str(&format!("| CRITICAL | {} |\n", critical));
    md.push_str(&format!("| HIGH | {} |\n", high));
    md.push_str(&format!("| MEDIUM | {} |\n", medium));
    md.push_str(&format!("| LOW | {} |\n\n", low));

    // Severity breakdown
    if !findings.is_empty() {
        md.push_str("## Findings by Severity\n\n");

        // Group by severity
        for severity in &["CRITICAL", "HIGH", "MEDIUM", "LOW"] {
            let sev_findings: Vec<_> = findings
                .iter()
                .filter(|f| f.severity == *severity)
                .collect();
            if sev_findings.is_empty() {
                continue;
            }

            md.push_str(&format!("### {} ({})\n\n", severity, sev_findings.len()));

            for (i, f) in sev_findings.iter().enumerate() {
                md.push_str(&format!(
                    "#### {}.{} {} — {} ({})\n\n",
                    severity.chars().next().unwrap(),
                    i + 1,
                    f.vuln_name,
                    f.cwe,
                    f.file
                ));
                md.push_str(&format!(
                    "- **File**: `{}` (line {})\n",
                    f.file, f.line_number
                ));
                md.push_str(&format!("- **AI Model**: {}\n", f.model));
                md.push_str(&format!("- **Description**: {}\n", f.description));
                md.push_str(&format!("- **Fix**: {}\n", f.fix));
                md.push_str(&format!("- **Code**: `{}`\n\n", f.line_content));
            }
        }
    } else {
        md.push_str("## Findings\n\nNo vulnerabilities detected in AI-generated code.\n\n");
    }

    // CWE breakdown
    if !findings.is_empty() {
        md.push_str("## CWE Distribution\n\n");
        md.push_str("| CWE | Name | Count |\n");
        md.push_str("|-----|------|-------|\n");
        let mut cwe_counts: HashMap<String, (String, usize)> = HashMap::new();
        for f in &findings {
            let entry = cwe_counts
                .entry(f.cwe.clone())
                .or_insert((f.vuln_name.clone(), 0));
            entry.1 += 1;
        }
        let mut cwe_sorted: Vec<_> = cwe_counts.into_iter().collect();
        cwe_sorted.sort_by_key(|b| std::cmp::Reverse(b.1 .1));
        for (cwe, (name, count)) in &cwe_sorted {
            md.push_str(&format!("| {} | {} | {} |\n", cwe, name, count));
        }
        md.push('\n');
    }

    // Recommendations
    md.push_str("## Recommendations\n\n");
    if critical > 0 || high > 0 {
        md.push_str(
            "1. **Immediate action** — Address all CRITICAL and HIGH findings before deployment.\n",
        );
        md.push_str("2. **Code review** — All AI-generated code touching these patterns needs manual security review.\n");
    }
    md.push_str("3. **Integrate SAST** — Run Semgrep, CodeQL, or Snyk alongside BlamePrompt for deeper analysis.\n");
    md.push_str("4. **AI prompt guidance** — Include security requirements in AI prompts to reduce vulnerability introduction.\n");
    md.push_str("5. **Regular scanning** — Run vulnerability scans on every PR that includes AI-generated code.\n\n");

    md.push_str("---\n\n");
    md.push_str("*Generated by [BlamePrompt](https://github.com/ekaanth/blameprompt) — AI Code Vulnerability Scanner*\n");

    match std::fs::write(output, &md) {
        Ok(_) => println!("Vulnerability scan written to {}", output),
        Err(e) => eprintln!("Error writing report: {}", e),
    }
}
