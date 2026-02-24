use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum ModelLicense {
    OpenSource,
    ClosedSource,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum ModelDeployment {
    Local,
    Cloud,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelClassification {
    pub model_id: String,
    pub family: String,
    pub vendor: String,
    pub license: ModelLicense,
    pub deployment: ModelDeployment,
    pub display_name: String,
}

pub fn classify(model_id: &str) -> ModelClassification {
    let lower = model_id.to_lowercase();

    // Check local prefixes first
    if lower.starts_with("ollama:") || lower.starts_with("lmstudio:") || lower.starts_with("local:")
    {
        let inner = model_id.split_once(':').map(|x| x.1).unwrap_or(model_id);
        let inner_lower = inner.to_lowercase();
        let (family, _vendor) = classify_inner_model(&inner_lower);
        return ModelClassification {
            model_id: model_id.to_string(),
            family,
            vendor: "local".to_string(),
            license: ModelLicense::OpenSource,
            deployment: ModelDeployment::Local,
            display_name: format!("Local: {}", inner),
        };
    }

    let (family, vendor, license, display) = if lower.contains("claude") {
        let display = if lower.contains("opus-4-6") {
            "Claude Opus 4.6"
        } else if lower.contains("opus-4-5") {
            "Claude Opus 4.5"
        } else if lower.contains("opus-4-1") {
            "Claude Opus 4.1"
        } else if lower.contains("opus-4-0") || lower.contains("opus-4-20") {
            "Claude Opus 4.0"
        } else if lower.contains("sonnet-4-5") {
            "Claude Sonnet 4.5"
        } else if lower.contains("sonnet-4-0") || lower.contains("sonnet-4-20") {
            "Claude Sonnet 4.0"
        } else if lower.contains("haiku-4-5") {
            "Claude Haiku 4.5"
        } else if lower.contains("haiku-3-5") || lower.contains("3-5-haiku") {
            "Claude Haiku 3.5"
        } else if lower.contains("haiku-3") || lower.contains("3-haiku") {
            "Claude Haiku 3"
        } else {
            "Claude (unknown)"
        };
        ("claude", "anthropic", ModelLicense::ClosedSource, display)
    } else if lower.contains("gpt-4")
        || lower.contains("gpt-3.5")
        || lower.contains("o1")
        || lower.contains("o3")
    {
        let display = if lower.contains("gpt-4o") {
            "GPT-4o"
        } else if lower.contains("gpt-4") {
            "GPT-4"
        } else if lower.contains("gpt-3.5") {
            "GPT-3.5"
        } else {
            "OpenAI"
        };
        ("gpt", "openai", ModelLicense::ClosedSource, display)
    } else if lower.contains("gemma") {
        ("gemini", "google", ModelLicense::OpenSource, "Gemma")
    } else if lower.contains("gemini") {
        ("gemini", "google", ModelLicense::ClosedSource, "Gemini")
    } else if lower.contains("codellama") {
        ("llama", "meta", ModelLicense::OpenSource, "Code Llama")
    } else if lower.contains("llama") {
        ("llama", "meta", ModelLicense::OpenSource, "Llama")
    } else if lower.contains("mixtral") {
        ("mistral", "mistral_ai", ModelLicense::OpenSource, "Mixtral")
    } else if lower.contains("codestral") {
        (
            "mistral",
            "mistral_ai",
            ModelLicense::OpenSource,
            "Codestral",
        )
    } else if lower.contains("mistral") {
        ("mistral", "mistral_ai", ModelLicense::OpenSource, "Mistral")
    } else if lower.contains("deepseek") {
        (
            "deepseek",
            "deepseek_ai",
            ModelLicense::OpenSource,
            "DeepSeek",
        )
    } else if lower.contains("phi-") {
        ("phi", "microsoft", ModelLicense::OpenSource, "Phi")
    } else if lower.contains("qwen") || lower.contains("codeqwen") {
        ("qwen", "alibaba", ModelLicense::OpenSource, "Qwen")
    } else if lower.contains("command-r") {
        (
            "command_r",
            "cohere",
            ModelLicense::ClosedSource,
            "Command R",
        )
    } else if lower.contains("replit") {
        (
            "replit",
            "replit",
            ModelLicense::ClosedSource,
            "Replit Agent",
        )
    } else {
        (
            "unknown",
            "unknown",
            ModelLicense::ClosedSource,
            "Unknown Model",
        )
    };

    ModelClassification {
        model_id: model_id.to_string(),
        family: family.to_string(),
        vendor: vendor.to_string(),
        license,
        deployment: ModelDeployment::Cloud,
        display_name: display.to_string(),
    }
}

fn classify_inner_model(lower: &str) -> (String, String) {
    if lower.contains("llama") || lower.contains("codellama") {
        ("llama".to_string(), "meta".to_string())
    } else if lower.contains("mistral") || lower.contains("mixtral") {
        ("mistral".to_string(), "mistral_ai".to_string())
    } else if lower.contains("deepseek") {
        ("deepseek".to_string(), "deepseek_ai".to_string())
    } else if lower.contains("phi") {
        ("phi".to_string(), "microsoft".to_string())
    } else if lower.contains("qwen") {
        ("qwen".to_string(), "alibaba".to_string())
    } else if lower.contains("gemma") {
        ("gemini".to_string(), "google".to_string())
    } else {
        ("unknown".to_string(), "local".to_string())
    }
}

#[allow(dead_code)]
pub fn is_open_source(model_id: &str) -> bool {
    classify(model_id).license == ModelLicense::OpenSource
}

#[allow(dead_code)]
pub fn is_local(model_id: &str) -> bool {
    classify(model_id).deployment == ModelDeployment::Local
}

pub fn display_name(model_id: &str) -> String {
    classify(model_id).display_name
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_claude_classification() {
        let c = classify("claude-opus-4-6");
        assert_eq!(c.license, ModelLicense::ClosedSource);
        assert_eq!(c.vendor, "anthropic");
        assert_eq!(c.deployment, ModelDeployment::Cloud);
    }

    #[test]
    fn test_ollama_local() {
        let c = classify("ollama:llama3.2");
        assert_eq!(c.deployment, ModelDeployment::Local);
        assert_eq!(c.license, ModelLicense::OpenSource);
    }

    #[test]
    fn test_deepseek_cloud() {
        let c = classify("deepseek-coder-v2");
        assert_eq!(c.license, ModelLicense::OpenSource);
        assert_eq!(c.deployment, ModelDeployment::Cloud);
    }

    #[test]
    fn test_gpt_classification() {
        let c = classify("gpt-4o");
        assert_eq!(c.license, ModelLicense::ClosedSource);
        assert_eq!(c.vendor, "openai");
    }

    #[test]
    fn test_local_prefix() {
        let c = classify("local:mistral-7b");
        assert_eq!(c.deployment, ModelDeployment::Local);
    }

    #[test]
    fn test_replit() {
        let c = classify("replit-agent");
        assert_eq!(c.vendor, "replit");
        assert_eq!(c.license, ModelLicense::ClosedSource);
    }

    #[test]
    fn test_display_names() {
        assert_eq!(
            display_name("claude-sonnet-4-5-20250929"),
            "Claude Sonnet 4.5"
        );
        assert_eq!(display_name("claude-opus-4-6"), "Claude Opus 4.6");
    }
}
