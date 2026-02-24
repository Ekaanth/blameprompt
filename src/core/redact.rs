use regex::Regex;
use sha2::{Digest, Sha256};
use std::collections::HashMap;

use crate::core::config::{BlamePromptConfig, RedactionConfig};

pub struct RedactionResult {
    pub redacted_text: String,
    pub detections: Vec<SecretDetection>,
}

pub struct SecretDetection {
    pub secret_type: String,
    pub severity: String,
}

/// Redact secrets from text, returning the cleaned string.
#[allow(dead_code)]
pub fn redact_secrets(text: &str) -> String {
    redact_with_report(text).redacted_text
}

/// Redact secrets using a specific config.
pub fn redact_secrets_with_config(text: &str, config: &BlamePromptConfig) -> String {
    redact_with_report_and_config(text, &config.redaction).redacted_text
}

/// Redact secrets and return detection metadata.
pub fn redact_with_report(text: &str) -> RedactionResult {
    let config = RedactionConfig::default();
    redact_with_report_and_config(text, &config)
}

/// Redact secrets with config and return detection metadata.
pub fn redact_with_report_and_config(text: &str, config: &RedactionConfig) -> RedactionResult {
    let mut result = text.to_string();
    let mut detections = Vec::new();

    let builtin_patterns: Vec<(&str, &str, &str, &str)> = vec![
        // (pattern, replacement, secret_type, severity)
        (
            r"sk-[A-Za-z0-9_-]{20,}",
            "[REDACTED_API_KEY]",
            "API_KEY",
            "HIGH",
        ),
        (
            r"key-[A-Za-z0-9_-]{20,}",
            "[REDACTED_API_KEY]",
            "API_KEY",
            "HIGH",
        ),
        (
            r"AKIA[A-Z0-9]{16}",
            "[REDACTED_AWS_KEY]",
            "AWS_KEY",
            "CRITICAL",
        ),
        (
            r#"(?i)(password|secret)\s*=\s*"[^"]*""#,
            "[REDACTED_SECRET]",
            "PASSWORD",
            "HIGH",
        ),
        (
            r"(?i)Bearer\s+[A-Za-z0-9_.~+/=-]{10,}",
            "Bearer [REDACTED]",
            "BEARER_TOKEN",
            "HIGH",
        ),
        (
            r"(?i)(token|auth)\s*=\s*[A-Za-z0-9_.~+/=-]{40,}",
            "[REDACTED_TOKEN]",
            "TOKEN",
            "MEDIUM",
        ),
        (
            r"[a-zA-Z0-9_.-]+@[a-zA-Z0-9._-]{2,}",
            "[REDACTED_HOST]",
            "SHELL_PROMPT",
            "MEDIUM",
        ),
        (
            r"(?:/Users/|/home/)[a-zA-Z0-9_.-]+",
            "[REDACTED_HOME]",
            "HOME_PATH",
            "LOW",
        ),
    ];

    // Apply built-in patterns (skip disabled ones)
    for (pattern, replacement, secret_type, severity) in &builtin_patterns {
        if config.disable_patterns.iter().any(|d| d == *secret_type) {
            continue;
        }
        let re = Regex::new(pattern).unwrap();
        let count = re.find_iter(&result.clone()).count();
        for _ in 0..count {
            detections.push(SecretDetection {
                secret_type: secret_type.to_string(),
                severity: severity.to_string(),
            });
        }
        if config.mode == "hash" {
            // Replace with SHA-256 prefix instead of placeholder
            let result_clone = result.clone();
            let mut new_result = result_clone.clone();
            for mat in re.find_iter(&result_clone) {
                let hash = sha256_prefix(mat.as_str());
                new_result = new_result.replacen(mat.as_str(), &hash, 1);
            }
            result = new_result;
        } else {
            result = re.replace_all(&result, *replacement).to_string();
        }
    }

    // Apply custom patterns from config
    for cp in &config.custom_patterns {
        if let Ok(re) = Regex::new(&cp.pattern) {
            let count = re.find_iter(&result.clone()).count();
            for _ in 0..count {
                detections.push(SecretDetection {
                    secret_type: "CUSTOM".to_string(),
                    severity: "MEDIUM".to_string(),
                });
            }
            result = re.replace_all(&result, cp.replacement.as_str()).to_string();
        }
    }

    // Entropy-based detection
    let entropy_detections = detect_high_entropy_strings(&result);
    for (start, end) in entropy_detections.iter().rev() {
        let token = &result[*start..*end];
        detections.push(SecretDetection {
            secret_type: "HIGH_ENTROPY".to_string(),
            severity: "MEDIUM".to_string(),
        });
        if config.mode == "hash" {
            let hash = sha256_prefix(token);
            result.replace_range(*start..*end, &hash);
        } else {
            result.replace_range(*start..*end, "[REDACTED_HIGH_ENTROPY]");
        }
    }

    RedactionResult {
        redacted_text: result,
        detections,
    }
}

fn sha256_prefix(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    let hash = format!("{:x}", hasher.finalize());
    format!("[SHA256:{}...]", &hash[..12])
}

/// Calculate Shannon entropy of a string.
fn shannon_entropy(s: &str) -> f64 {
    if s.is_empty() {
        return 0.0;
    }
    let mut freq: HashMap<char, usize> = HashMap::new();
    for c in s.chars() {
        *freq.entry(c).or_insert(0) += 1;
    }
    let len = s.len() as f64;
    freq.values()
        .map(|&count| {
            let p = count as f64 / len;
            -p * p.log2()
        })
        .sum()
}

/// Detect high-entropy strings that may be secrets.
/// Returns byte-offset ranges (start, end) of suspicious tokens.
fn detect_high_entropy_strings(text: &str) -> Vec<(usize, usize)> {
    let mut results = Vec::new();
    // Split on whitespace and common delimiters, track byte offsets
    let token_re = Regex::new(r"[A-Za-z0-9+/=_\-]{20,}").unwrap();
    for mat in token_re.find_iter(text) {
        let token = mat.as_str();
        // Skip tokens that are already redacted
        if token.contains("REDACTED") || token.contains("SHA256") {
            continue;
        }
        // Skip tokens that look like normal words (low entropy)
        let entropy = shannon_entropy(token);
        if entropy > 4.5 && token.len() >= 20 {
            results.push((mat.start(), mat.end()));
        }
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_redact_api_key() {
        let text = "my key is sk-ant-api03-abcdefghijklmnopqrstuvwxyz";
        let redacted = redact_secrets(text);
        assert!(redacted.contains("[REDACTED_API_KEY]"));
        assert!(!redacted.contains("sk-ant"));
    }

    #[test]
    fn test_redact_aws_key() {
        let text = "aws key: AKIAIOSFODNN7EXAMPLE";
        let redacted = redact_secrets(text);
        assert!(redacted.contains("[REDACTED_AWS_KEY]"));
        assert!(!redacted.contains("AKIA"));
    }

    #[test]
    fn test_redact_password() {
        let text = r#"password = "hunter2""#;
        let redacted = redact_secrets(text);
        assert!(redacted.contains("[REDACTED_SECRET]"));
        assert!(!redacted.contains("hunter2"));
    }

    #[test]
    fn test_redact_bearer() {
        let text = "Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9";
        let redacted = redact_secrets(text);
        assert!(redacted.contains("Bearer [REDACTED]"));
        assert!(!redacted.contains("eyJh"));
    }

    #[test]
    fn test_normal_text_unchanged() {
        let text = "This is a normal prompt about writing a function";
        let redacted = redact_secrets(text);
        assert_eq!(text, redacted);
    }

    #[test]
    fn test_redact_with_report_counts() {
        let text = "key is sk-ant-api03-abcdefghijklmnopqrstuvwxyz and AKIAIOSFODNN7EXAMPLE";
        let result = redact_with_report(text);
        assert_eq!(result.detections.len(), 2);
        assert!(result.detections.iter().any(|d| d.secret_type == "API_KEY"));
        assert!(result.detections.iter().any(|d| d.secret_type == "AWS_KEY"));
    }

    #[test]
    fn test_shannon_entropy() {
        // Low entropy - repeated chars
        let low = shannon_entropy("aaaaaaaaaa");
        assert!(low < 1.0);

        // High entropy - random-looking
        let high = shannon_entropy("aB3cD4eF5gH6iJ7kL8mN9oP0qR");
        assert!(high > 4.0);
    }

    #[test]
    fn test_entropy_detection() {
        // A high-entropy string that looks like a secret
        let text = "token=aB3cD4eF5gH6iJ7kL8mN9oP0qRsTuVwXyZ1234567890abcdef";
        let result = redact_with_report(text);
        // Should be caught by either pattern or entropy
        assert!(!result.detections.is_empty());
    }

    #[test]
    fn test_custom_pattern() {
        let config = RedactionConfig {
            custom_patterns: vec![crate::core::config::CustomPattern {
                pattern: r"CUST-\d{6,}".to_string(),
                replacement: "[REDACTED_CUSTOMER_ID]".to_string(),
            }],
            disable_patterns: Vec::new(),
            mode: "replace".to_string(),
        };
        let result = redact_with_report_and_config("Customer CUST-123456 filed a ticket", &config);
        assert!(result.redacted_text.contains("[REDACTED_CUSTOMER_ID]"));
        assert!(!result.redacted_text.contains("CUST-123456"));
    }

    #[test]
    fn test_disabled_pattern() {
        let config = RedactionConfig {
            custom_patterns: Vec::new(),
            disable_patterns: vec!["BEARER_TOKEN".to_string()],
            mode: "replace".to_string(),
        };
        let text = "Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9";
        let result = redact_with_report_and_config(text, &config);
        // Bearer token pattern should be skipped (but entropy might still catch it)
        assert!(!result
            .detections
            .iter()
            .any(|d| d.secret_type == "BEARER_TOKEN"));
    }

    #[test]
    fn test_hash_mode() {
        let config = RedactionConfig {
            custom_patterns: Vec::new(),
            disable_patterns: Vec::new(),
            mode: "hash".to_string(),
        };
        let text = "my key is sk-ant-api03-abcdefghijklmnopqrstuvwxyz";
        let result = redact_with_report_and_config(text, &config);
        assert!(result.redacted_text.contains("[SHA256:"));
        assert!(!result.redacted_text.contains("sk-ant"));
    }

    #[test]
    fn test_redact_shell_prompt() {
        let text = "(base) metaquity@Abhisheks-MacBook-Pro-2 blameprompt % blameprompt pull";
        let redacted = redact_secrets(text);
        assert!(!redacted.contains("metaquity@"), "Username leaked: {}", redacted);
        assert!(redacted.contains("[REDACTED_HOST]"), "Host not redacted: {}", redacted);
        // Rest of prompt preserved
        assert!(redacted.contains("(base)"), "Env prefix lost: {}", redacted);
        assert!(redacted.contains("blameprompt pull"), "Command lost: {}", redacted);
    }

    #[test]
    fn test_redact_simple_shell_prompt() {
        let text = "user@hostname $ ls -la";
        let redacted = redact_secrets(text);
        assert!(redacted.contains("[REDACTED_HOST]"));
        assert!(!redacted.contains("user@hostname"));
        // Command preserved
        assert!(redacted.contains("$ ls -la"));
    }
}
