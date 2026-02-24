/// Estimate cost based on model, input tokens, and output tokens.
/// Pricing verified from platform.claude.com/docs/en/about-claude/pricing (February 2026).
pub fn estimate_cost(model: &str, input_tokens: u64, output_tokens: u64) -> f64 {
    let model_lower = model.to_lowercase();
    let (input_rate, output_rate) = get_rates(&model_lower);
    (input_tokens as f64 / 1_000_000.0) * input_rate
        + (output_tokens as f64 / 1_000_000.0) * output_rate
}

/// Estimate tokens from character count.
/// JSONL doesn't expose token counts, so we approximate: 1 token ~ 4 characters.
pub fn estimate_tokens_from_chars(char_count: usize) -> u64 {
    (char_count / 4) as u64
}

fn get_rates(model_lower: &str) -> (f64, f64) {
    if model_lower.contains("opus-4-6") || model_lower.contains("opus-4-5") {
        (5.00, 25.00)
    } else if model_lower.contains("opus-4-1") || model_lower.contains("opus-4-0")
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
    } else {
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
        assert!((cost - expected).abs() < 0.0001, "Sonnet cost: {} vs expected: {}", cost, expected);
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
    fn test_estimate_tokens_from_chars() {
        assert_eq!(estimate_tokens_from_chars(400), 100);
        assert_eq!(estimate_tokens_from_chars(0), 0);
        assert_eq!(estimate_tokens_from_chars(3), 0); // rounds down
    }
}
