use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize, Clone, Default)]
pub struct BlamePromptConfig {
    #[serde(default)]
    pub redaction: RedactionConfig,
    #[serde(default)]
    pub capture: CaptureConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct CustomPattern {
    pub pattern: String,
    pub replacement: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct RedactionConfig {
    #[serde(default)]
    pub custom_patterns: Vec<CustomPattern>,
    #[serde(default)]
    pub disable_patterns: Vec<String>,
    #[serde(default = "default_redaction_mode")]
    pub mode: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct CaptureConfig {
    #[serde(default = "default_max_prompt_length")]
    pub max_prompt_length: usize,
    #[serde(default)]
    pub store_full_conversation: bool,
}

fn default_redaction_mode() -> String {
    "replace".to_string()
}

fn default_max_prompt_length() -> usize {
    2000
}

impl Default for RedactionConfig {
    fn default() -> Self {
        RedactionConfig {
            custom_patterns: Vec::new(),
            disable_patterns: Vec::new(),
            mode: default_redaction_mode(),
        }
    }
}

impl Default for CaptureConfig {
    fn default() -> Self {
        CaptureConfig {
            max_prompt_length: default_max_prompt_length(),
            store_full_conversation: false,
        }
    }
}

fn find_config_file() -> Option<PathBuf> {
    // Check repo root first
    let repo_config = Path::new(".blamepromptrc");
    if repo_config.exists() {
        return Some(repo_config.to_path_buf());
    }

    // Fall back to home directory
    if let Some(home) = dirs::home_dir() {
        let home_config = home.join(".blamepromptrc");
        if home_config.exists() {
            return Some(home_config);
        }
    }

    None
}

pub fn load_config() -> BlamePromptConfig {
    match find_config_file() {
        Some(path) => match std::fs::read_to_string(&path) {
            Ok(content) => toml::from_str(&content).unwrap_or_else(|e| {
                eprintln!(
                    "[BlamePrompt] Warning: Failed to parse {}: {}",
                    path.display(),
                    e
                );
                BlamePromptConfig::default()
            }),
            Err(e) => {
                eprintln!(
                    "[BlamePrompt] Warning: Failed to read {}: {}",
                    path.display(),
                    e
                );
                BlamePromptConfig::default()
            }
        },
        None => BlamePromptConfig::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = BlamePromptConfig::default();
        assert_eq!(config.capture.max_prompt_length, 2000);
        assert!(!config.capture.store_full_conversation);
        assert_eq!(config.redaction.mode, "replace");
        assert!(config.redaction.custom_patterns.is_empty());
        assert!(config.redaction.disable_patterns.is_empty());
    }

    #[test]
    fn test_parse_config() {
        let toml_str = r#"
[redaction]
mode = "hash"
custom_patterns = [
    { pattern = "internal\\.company\\.com/[\\w/]+", replacement = "[REDACTED_INTERNAL_URL]" },
]
disable_patterns = ["BEARER_TOKEN"]

[capture]
max_prompt_length = 5000
store_full_conversation = true
"#;
        let config: BlamePromptConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.redaction.mode, "hash");
        assert_eq!(config.redaction.custom_patterns.len(), 1);
        assert_eq!(
            config.redaction.custom_patterns[0].replacement,
            "[REDACTED_INTERNAL_URL]"
        );
        assert_eq!(config.redaction.disable_patterns, vec!["BEARER_TOKEN"]);
        assert_eq!(config.capture.max_prompt_length, 5000);
        assert!(config.capture.store_full_conversation);
    }

    #[test]
    fn test_partial_config() {
        let toml_str = r#"
[capture]
max_prompt_length = 1000
"#;
        let config: BlamePromptConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.capture.max_prompt_length, 1000);
        assert_eq!(config.redaction.mode, "replace");
    }
}
