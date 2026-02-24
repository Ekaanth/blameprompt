use crate::core::{config, pricing, receipt::Receipt, redact, transcript};
use crate::commands::staging;
use chrono::Utc;
use sha2::{Digest, Sha256};

pub fn run(session_path: &str, provider: Option<&str>) {
    let provider = provider.unwrap_or("claude");

    let parsed = match transcript::parse_claude_jsonl(session_path) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Error parsing transcript: {}", e);
            return;
        }
    };

    if parsed.files_modified.is_empty() {
        println!("No file modifications found in transcript.");
        return;
    }

    let cfg = config::load_config();
    let model = parsed.model.unwrap_or_else(|| "unknown".to_string());

    let prompt_summary = if cfg.capture.store_full_conversation {
        let full = transcript::full_conversation_text(&parsed.transcript);
        let truncated: String = full.chars().take(cfg.capture.max_prompt_length).collect();
        redact::redact_secrets_with_config(&truncated, &cfg)
    } else {
        transcript::first_user_prompt(&parsed.transcript)
            .map(|p| {
                let truncated: String = p.chars().take(cfg.capture.max_prompt_length).collect();
                redact::redact_secrets_with_config(&truncated, &cfg)
            })
            .unwrap_or_default()
    };

    let full_text = transcript::full_conversation_text(&parsed.transcript);
    let mut hasher = Sha256::new();
    hasher.update(full_text.as_bytes());
    let prompt_hash = format!("sha256:{:x}", hasher.finalize());

    let total_chars: usize = parsed.transcript.messages.iter().map(|m| match m {
        transcript::Message::User { text, .. } => text.len(),
        transcript::Message::Assistant { text, .. } => text.len(),
        transcript::Message::ToolUse { .. } => 0,
    }).sum();
    let estimated_tokens = pricing::estimate_tokens_from_chars(total_chars);
    let cost = pricing::estimate_cost(&model, estimated_tokens / 2, estimated_tokens / 2);

    let conversation_turns = transcript::extract_conversation_turns(
        &parsed.transcript,
        cfg.capture.max_prompt_length,
        &|text| redact::redact_secrets_with_config(text, &cfg),
    );

    let user = get_git_user();
    let message_count = parsed.transcript.messages.len() as u32;

    let mut receipt_count = 0;
    for file_path in &parsed.files_modified {
        let receipt = Receipt {
            id: Receipt::new_id(),
            provider: provider.to_string(),
            model: model.clone(),
            session_id: parsed.session_id.clone(),
            prompt_summary: prompt_summary.clone(),
            prompt_hash: prompt_hash.clone(),
            message_count,
            cost_usd: cost,
            timestamp: Utc::now(),
            session_start: parsed.session_start,
            session_end: parsed.session_end,
            session_duration_secs: parsed.session_duration_secs,
            ai_response_time_secs: parsed.avg_response_time_secs,
            user: user.clone(),
            file_path: file_path.clone(),
            line_range: (1, 1), // Unknown without diff context
            parent_receipt_id: None,
            conversation: if conversation_turns.is_empty() { None } else { Some(conversation_turns.clone()) },
        };

        staging::add_receipt(&receipt);
        receipt_count += 1;
    }

    println!("[BlamePrompt] Recorded {} receipt(s) from session {}", receipt_count, parsed.session_id);
    println!("  Provider: {}", provider);
    println!("  Model: {}", model);
    println!("  Messages: {}", message_count);
    println!("  Files: {}", parsed.files_modified.join(", "));
    println!("  Est. cost: ${:.4}", cost);
    println!("\nReceipts added to staging. They will be attached on next commit.");
}

fn get_git_user() -> String {
    let name = std::process::Command::new("git")
        .args(["config", "user.name"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let email = std::process::Command::new("git")
        .args(["config", "user.email"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown@unknown".to_string());

    format!("{} <{}>", name, email)
}
