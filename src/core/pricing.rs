/// Estimate cost based on model, input tokens, and output tokens.
/// Pricing verified from platform.claude.com/docs/en/about-claude/pricing (February 2026).
pub fn estimate_cost(model: &str, input_tokens: u64, output_tokens: u64) -> f64 {
    let model_lower = model.to_lowercase();
    let (input_rate, output_rate) = get_rates(&model_lower);
    (input_tokens as f64 / 1_000_000.0) * input_rate
        + (output_tokens as f64 / 1_000_000.0) * output_rate
}

/// Compute cost from actual token usage data (including cache pricing).
/// Cache reads are 90% cheaper than regular input tokens.
/// Cache creation tokens are 25% more expensive than regular input tokens.
pub fn cost_from_usage(
    model: &str,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cache_creation_tokens: u64,
) -> f64 {
    let model_lower = model.to_lowercase();
    let (input_rate, output_rate) = get_rates(&model_lower);
    let cache_read_rate = input_rate * 0.1; // 90% discount
    let cache_creation_rate = input_rate * 1.25; // 25% surcharge

    (input_tokens as f64 / 1_000_000.0) * input_rate
        + (output_tokens as f64 / 1_000_000.0) * output_rate
        + (cache_read_tokens as f64 / 1_000_000.0) * cache_read_rate
        + (cache_creation_tokens as f64 / 1_000_000.0) * cache_creation_rate
}

/// Estimate tokens from character count.
/// Fallback when JSONL doesn't expose token counts: 1 token ~ 4 characters.
pub fn estimate_tokens_from_chars(char_count: usize) -> u64 {
    (char_count / 4) as u64
}

fn get_rates(model_lower: &str) -> (f64, f64) {
    // ── Anthropic (Claude) ──────────────────────────────────────────────
    if model_lower.contains("opus-4-6") || model_lower.contains("opus-4-5") {
        (5.00, 25.00)
    } else if model_lower.contains("opus-4-1")
        || model_lower.contains("opus-4-0")
        || model_lower.contains("opus-4-20")
    {
        (15.00, 75.00)
    } else if model_lower.contains("sonnet") {
        (3.00, 15.00)
    } else if model_lower.contains("haiku-4-5") || model_lower.contains("haiku-4-") {
        (1.00, 5.00)
    } else if model_lower.contains("haiku-3-5") || model_lower.contains("3-5-haiku") {
        (0.80, 4.00)
    } else if model_lower.contains("haiku-3") || model_lower.contains("3-haiku") {
        (0.25, 1.25)
    }
    // ── OpenAI / Codex ──────────────────────────────────────────────────
    else if model_lower.contains("o3-pro") {
        (60.00, 240.00)
    } else if model_lower.contains("o3-mini") {
        (1.10, 4.40)
    } else if model_lower.contains("o3") {
        (10.00, 40.00)
    } else if model_lower.contains("o4-mini") {
        (1.10, 4.40)
    } else if model_lower.contains("o1-pro") {
        (150.00, 600.00)
    } else if model_lower.contains("o1-mini") {
        (3.00, 12.00)
    } else if model_lower.contains("o1") {
        (15.00, 60.00)
    } else if model_lower.contains("gpt-4.1") || model_lower.contains("gpt-4-1") {
        (2.00, 8.00)
    } else if model_lower.contains("gpt-4.1-mini") || model_lower.contains("gpt-4-1-mini") {
        (0.40, 1.60)
    } else if model_lower.contains("gpt-4.1-nano") || model_lower.contains("gpt-4-1-nano") {
        (0.10, 0.40)
    } else if model_lower.contains("gpt-4o-mini") {
        (0.15, 0.60)
    } else if model_lower.contains("gpt-4o") {
        (2.50, 10.00)
    } else if model_lower.contains("gpt-4-turbo") {
        (10.00, 30.00)
    } else if model_lower.contains("gpt-4") {
        (30.00, 60.00)
    } else if model_lower.contains("gpt-3.5") || model_lower.contains("gpt-3-5") {
        (0.50, 1.50)
    } else if model_lower.contains("codex") {
        // OpenAI Codex CLI — uses GPT-4.1 pricing
        (2.00, 8.00)
    }
    // ── Google (Gemini) ─────────────────────────────────────────────────
    else if model_lower.contains("gemini-2.5-pro") || model_lower.contains("gemini-2-5-pro") {
        (1.25, 10.00) // $1.25/M input (<200k), $10/M output
    } else if model_lower.contains("gemini-2.5-flash") || model_lower.contains("gemini-2-5-flash") {
        (0.15, 0.60)
    } else if model_lower.contains("gemini-2.0-flash") || model_lower.contains("gemini-2-0-flash") {
        (0.10, 0.40)
    } else if model_lower.contains("gemini-1.5-pro") || model_lower.contains("gemini-1-5-pro") {
        (1.25, 5.00)
    } else if model_lower.contains("gemini-1.5-flash") || model_lower.contains("gemini-1-5-flash") {
        (0.075, 0.30)
    } else if model_lower.contains("gemini") {
        // Generic Gemini — default to Flash pricing
        (0.15, 0.60)
    }
    // ── Default ─────────────────────────────────────────────────────────
    else {
        // Default to Sonnet pricing for unknown models
        (3.00, 15.00)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sonnet_pricing() {
        let cost = estimate_cost("claude-sonnet-4-5-20250929", 1250, 890);
        let expected = (1250.0 / 1_000_000.0) * 3.0 + (890.0 / 1_000_000.0) * 15.0;
        assert!(
            (cost - expected).abs() < 0.0001,
            "Sonnet cost: {} vs expected: {}",
            cost,
            expected
        );
    }

    #[test]
    fn test_opus_4_6_pricing() {
        let cost = estimate_cost("claude-opus-4-6", 1000, 500);
        let expected = (1000.0 / 1_000_000.0) * 5.0 + (500.0 / 1_000_000.0) * 25.0;
        assert!((cost - expected).abs() < 0.0001);
    }

    #[test]
    fn test_opus_4_1_pricing() {
        let cost = estimate_cost("claude-opus-4-1-20250805", 1000, 500);
        let expected = (1000.0 / 1_000_000.0) * 15.0 + (500.0 / 1_000_000.0) * 75.0;
        assert!((cost - expected).abs() < 0.0001);
    }

    #[test]
    fn test_haiku_4_5_pricing() {
        let cost = estimate_cost("claude-haiku-4-5-20251001", 1000, 500);
        let expected = (1000.0 / 1_000_000.0) * 1.0 + (500.0 / 1_000_000.0) * 5.0;
        assert!((cost - expected).abs() < 0.0001);
    }

    #[test]
    fn test_unknown_model_defaults_to_sonnet() {
        let cost = estimate_cost("some-unknown-model", 1000, 500);
        let expected = (1000.0 / 1_000_000.0) * 3.0 + (500.0 / 1_000_000.0) * 15.0;
        assert!((cost - expected).abs() < 0.0001);
    }

    #[test]
    fn test_gpt4o_pricing() {
        let cost = estimate_cost("gpt-4o-2024-08-06", 1000, 500);
        let expected = (1000.0 / 1_000_000.0) * 2.50 + (500.0 / 1_000_000.0) * 10.00;
        assert!((cost - expected).abs() < 0.0001);
    }

    #[test]
    fn test_gpt4o_mini_pricing() {
        let cost = estimate_cost("gpt-4o-mini", 1000, 500);
        let expected = (1000.0 / 1_000_000.0) * 0.15 + (500.0 / 1_000_000.0) * 0.60;
        assert!((cost - expected).abs() < 0.0001);
    }

    #[test]
    fn test_o3_pricing() {
        let cost = estimate_cost("o3-2025-04-16", 1000, 500);
        let expected = (1000.0 / 1_000_000.0) * 10.0 + (500.0 / 1_000_000.0) * 40.0;
        assert!((cost - expected).abs() < 0.0001);
    }

    #[test]
    fn test_gemini_pro_pricing() {
        let cost = estimate_cost("gemini-2.5-pro-preview-05-06", 1000, 500);
        let expected = (1000.0 / 1_000_000.0) * 1.25 + (500.0 / 1_000_000.0) * 10.00;
        assert!((cost - expected).abs() < 0.0001);
    }

    #[test]
    fn test_gemini_flash_pricing() {
        let cost = estimate_cost("gemini-2.5-flash", 1000, 500);
        let expected = (1000.0 / 1_000_000.0) * 0.15 + (500.0 / 1_000_000.0) * 0.60;
        assert!((cost - expected).abs() < 0.0001);
    }

    #[test]
    fn test_codex_pricing() {
        let cost = estimate_cost("codex-mini", 1000, 500);
        let expected = (1000.0 / 1_000_000.0) * 2.00 + (500.0 / 1_000_000.0) * 8.00;
        assert!((cost - expected).abs() < 0.0001);
    }

    #[test]
    fn test_estimate_tokens_from_chars() {
        assert_eq!(estimate_tokens_from_chars(400), 100);
        assert_eq!(estimate_tokens_from_chars(0), 0);
        assert_eq!(estimate_tokens_from_chars(3), 0); // rounds down
    }
}
