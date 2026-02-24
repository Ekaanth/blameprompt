use crate::commands::audit;
use crate::core::model_classifier::{self, ModelDeployment, ModelLicense};
use crate::core::redact;
use chrono::Utc;
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

struct RiskFactor {
    name: String,
    score: f64,  // 0.0 - 10.0
    weight: f64, // how much this factor contributes
    detail: String,
}

fn calculate_risk_factors(entries: &[audit::AuditEntry]) -> Vec<RiskFactor> {
    let all_receipts: Vec<_> = entries.iter().flat_map(|e| &e.receipts).collect();
    let mut factors = Vec::new();

    if all_receipts.is_empty() {
        return factors;
    }

    // 1. Model diversity risk — more models = more supply chain surface
    let unique_models: std::collections::HashSet<_> =
        all_receipts.iter().map(|r| &r.model).collect();
    let model_count = unique_models.len();
    let model_score = (model_count as f64 * 2.0).min(10.0);
    factors.push(RiskFactor {
        name: "Model Diversity".to_string(),
        score: model_score,
        weight: 0.15,
        detail: format!(
            "{} unique models used — each is a separate supply chain dependency",
            model_count
        ),
    });

    // 2. Cloud vs local — cloud models have higher supply chain risk
    let cloud_count = all_receipts
        .iter()
        .filter(|r| model_classifier::classify(&r.model).deployment == ModelDeployment::Cloud)
        .count();
    let cloud_pct = cloud_count as f64 / all_receipts.len() as f64;
    let cloud_score = cloud_pct * 10.0;
    factors.push(RiskFactor {
        name: "Cloud Dependency".to_string(),
        score: cloud_score,
        weight: 0.20,
        detail: format!(
            "{:.0}% of AI code generated via cloud APIs ({} of {} receipts)",
            cloud_pct * 100.0,
            cloud_count,
            all_receipts.len()
        ),
    });

    // 3. Open-source model risk — licensing and supply chain integrity
    let oss_count = all_receipts
        .iter()
        .filter(|r| model_classifier::classify(&r.model).license == ModelLicense::OpenSource)
        .count();
    let oss_pct = oss_count as f64 / all_receipts.len() as f64;
    let oss_score = oss_pct * 6.0; // open source is less risky for supply chain
    factors.push(RiskFactor {
        name: "Open-Source Model Usage".to_string(),
        score: oss_score,
        weight: 0.10,
        detail: format!(
            "{:.0}% of AI code from open-source models (verify model integrity)",
            oss_pct * 100.0
        ),
    });

    // 4. Prompt sensitivity — check for secrets/sensitive data in prompts
    let mut prompts_with_secrets = 0;
    for r in &all_receipts {
        let report = redact::redact_with_report(&r.prompt_summary);
        if !report.detections.is_empty() {
            prompts_with_secrets += 1;
        }
    }
    let sensitivity_pct = prompts_with_secrets as f64 / all_receipts.len() as f64;
    let sensitivity_score = sensitivity_pct * 10.0;
    factors.push(RiskFactor {
        name: "Prompt Sensitivity".to_string(),
        score: sensitivity_score,
        weight: 0.20,
        detail: format!(
            "{} of {} prompts contain or reference sensitive data",
            prompts_with_secrets,
            all_receipts.len()
        ),
    });

    // 5. Critical file exposure — AI touching sensitive paths
    let sensitive_patterns = [
        "auth", "crypto", "security", "secret", "password", "key", "token", "payment", "billing",
        "admin", "config", "env", ".pem", ".key",
    ];
    let sensitive_files: Vec<_> = all_receipts
        .iter()
        .filter(|r| {
            r.all_file_paths().iter().any(|f| {
                let lower = f.to_lowercase();
                sensitive_patterns.iter().any(|p| lower.contains(p))
            })
        })
        .collect();
    let sensitive_pct = sensitive_files.len() as f64 / all_receipts.len() as f64;
    let file_score = sensitive_pct * 10.0;
    factors.push(RiskFactor {
        name: "Critical File Exposure".to_string(),
        score: file_score,
        weight: 0.20,
        detail: format!(
            "{} of {} AI-generated code changes touch security-sensitive files",
            sensitive_files.len(),
            all_receipts.len()
        ),
    });

    // 6. Human review coverage — commits with multiple receipts suggest less review
    let mut single_receipt_commits = 0;
    let mut multi_receipt_commits = 0;
    for entry in entries {
        if entry.receipts.len() == 1 {
            single_receipt_commits += 1;
        } else if entry.receipts.len() > 1 {
            multi_receipt_commits += 1;
        }
    }
    let total_commits = single_receipt_commits + multi_receipt_commits;
    let review_score = if total_commits > 0 {
        (multi_receipt_commits as f64 / total_commits as f64) * 8.0
    } else {
        5.0
    };
    factors.push(RiskFactor {
        name: "Human Review Gap".to_string(),
        score: review_score,
        weight: 0.15,
        detail: format!(
            "{} of {} commits have multiple AI receipts (may indicate less human review)",
            multi_receipt_commits, total_commits
        ),
    });

    factors
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
        println!("No AI-generated code found to assess.");
        return;
    }

    let now = Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string();
    let all_receipts: Vec<_> = entries.iter().flat_map(|e| &e.receipts).collect();
    let factors = calculate_risk_factors(&entries);

    // Calculate weighted overall score
    let overall_score: f64 = factors.iter().map(|f| f.score * f.weight).sum::<f64>()
        / factors.iter().map(|f| f.weight).sum::<f64>()
        * 10.0;
    let overall_score = overall_score.min(10.0);

    let risk_level = if overall_score >= 7.0 {
        "CRITICAL"
    } else if overall_score >= 5.0 {
        "HIGH"
    } else if overall_score >= 3.0 {
        "MEDIUM"
    } else {
        "LOW"
    };

    let mut md = String::new();

    md.push_str("# BlamePrompt Supply Chain Risk Assessment\n\n");
    md.push_str(&format!("> Generated: {}\n\n", now));

    // Overall score
    md.push_str("## Overall Risk Score\n\n");
    md.push_str(&format!(
        "**Score: {:.1} / 10.0** — **{}**\n\n",
        overall_score, risk_level
    ));
    md.push_str("```\n");
    let filled = (overall_score * 3.0) as usize;
    let empty = 30 - filled;
    md.push_str(&format!(
        "[{}{}] {:.1}/10\n",
        "#".repeat(filled),
        "-".repeat(empty),
        overall_score
    ));
    md.push_str("```\n\n");

    // Summary
    md.push_str("## Summary\n\n");
    md.push_str("| Metric | Value |\n");
    md.push_str("|--------|-------|\n");
    md.push_str(&format!("| Total AI receipts | {} |\n", all_receipts.len()));
    md.push_str(&format!("| Commits analyzed | {} |\n", entries.len()));
    let unique_providers: std::collections::HashSet<_> =
        all_receipts.iter().map(|r| &r.provider).collect();
    md.push_str(&format!("| AI providers | {} |\n", unique_providers.len()));
    let unique_models: std::collections::HashSet<_> =
        all_receipts.iter().map(|r| &r.model).collect();
    md.push_str(&format!("| Unique models | {} |\n", unique_models.len()));
    md.push_str(&format!("| Risk level | {} |\n\n", risk_level));

    // Risk factors breakdown
    md.push_str("## Risk Factor Breakdown\n\n");
    md.push_str("| Factor | Score | Weight | Detail |\n");
    md.push_str("|--------|-------|--------|--------|\n");
    for f in &factors {
        let level = if f.score >= 7.0 {
            "CRITICAL"
        } else if f.score >= 5.0 {
            "HIGH"
        } else if f.score >= 3.0 {
            "MEDIUM"
        } else {
            "LOW"
        };
        md.push_str(&format!(
            "| {} | {:.1} ({}) | {:.0}% | {} |\n",
            f.name,
            f.score,
            level,
            f.weight * 100.0,
            f.detail
        ));
    }
    md.push('\n');

    // Per-model risk
    md.push_str("## Per-Model Supply Chain Analysis\n\n");
    md.push_str("| Model | Vendor | Deployment | License | Receipts | Risk |\n");
    md.push_str("|-------|--------|------------|---------|----------|------|\n");
    let mut model_counts: HashMap<String, usize> = HashMap::new();
    for r in &all_receipts {
        *model_counts.entry(r.model.clone()).or_insert(0) += 1;
    }
    for (model_id, count) in &model_counts {
        let c = model_classifier::classify(model_id);
        let risk = match (&c.deployment, &c.license) {
            (ModelDeployment::Cloud, ModelLicense::ClosedSource) => "HIGH — cloud + proprietary",
            (ModelDeployment::Cloud, ModelLicense::OpenSource) => "MEDIUM — cloud + OSS",
            (ModelDeployment::Local, _) => "LOW — local deployment",
        };
        let license_str = match c.license {
            ModelLicense::OpenSource => "Open Source",
            ModelLicense::ClosedSource => "Closed Source",
        };
        md.push_str(&format!(
            "| {} | {} | {:?} | {} | {} | {} |\n",
            c.display_name, c.vendor, c.deployment, license_str, count, risk
        ));
    }
    md.push('\n');

    // High-risk files
    let sensitive_patterns = [
        "auth", "crypto", "security", "secret", "password", "key", "token", "payment", "billing",
        "admin", "config", "env", ".pem", ".key",
    ];
    let sensitive_receipts: Vec<_> = all_receipts
        .iter()
        .filter(|r| {
            r.all_file_paths().iter().any(|f| {
                let lower = f.to_lowercase();
                sensitive_patterns.iter().any(|p| lower.contains(p))
            })
        })
        .collect();
    if !sensitive_receipts.is_empty() {
        md.push_str("## High-Risk AI-Modified Files\n\n");
        md.push_str("These security-sensitive files were generated or modified by AI:\n\n");
        md.push_str("| File | Model | Provider | Lines |\n");
        md.push_str("|------|-------|----------|-------|\n");
        for r in &sensitive_receipts {
            for fc in r.all_file_changes() {
                md.push_str(&format!(
                    "| {} | {} | {} | {}-{} |\n",
                    relative_path(&fc.path),
                    r.model,
                    r.provider,
                    fc.line_range.0,
                    fc.line_range.1
                ));
            }
        }
        md.push('\n');
    }

    // Recommendations
    md.push_str("## Recommendations\n\n");
    if overall_score >= 5.0 {
        md.push_str("1. **Reduce cloud dependency** — Use local models (Ollama/LM Studio) for sensitive code.\n");
        md.push_str("2. **Pin model versions** — Lock to specific model versions to prevent supply chain drift.\n");
        md.push_str("3. **Mandatory code review** — Require human review for all AI-generated security-critical code.\n");
    }
    if overall_score >= 3.0 {
        md.push_str(
            "4. **Model integrity verification** — Verify checksums of local model weights.\n",
        );
        md.push_str("5. **Provider access audit** — Review which team members have access to each AI provider.\n");
    }
    md.push_str(
        "6. **Regular risk assessment** — Re-run this scan weekly or before each release.\n",
    );
    md.push_str("7. **SBOM inclusion** — Include AI model dependencies in your Software Bill of Materials.\n\n");

    md.push_str("---\n\n");
    md.push_str("*Generated by [BlamePrompt](https://github.com/ekaanth/blameprompt) — Supply Chain Risk Scanner*\n");

    match std::fs::write(output, &md) {
        Ok(_) => println!("Supply chain risk assessment written to {}", output),
        Err(e) => eprintln!("Error writing report: {}", e),
    }
}
