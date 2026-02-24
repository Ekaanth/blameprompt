use crate::commands::audit;
use crate::core::model_classifier::{self, ModelDeployment};
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

struct SecretExposure {
    secret_type: String,
    severity: String,
    provider: String,
    model: String,
    user: String,
    file: String,
    timestamp: String,
    deployment: ModelDeployment,
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

    let mut exposures: Vec<SecretExposure> = Vec::new();

    // Check all prompts for secrets that may have been sent to AI providers
    for r in &all_receipts {
        let classification = model_classifier::classify(&r.model);

        // Check prompt summary
        let report = redact::redact_with_report(&r.prompt_summary);
        for detection in &report.detections {
            exposures.push(SecretExposure {
                secret_type: detection.secret_type.clone(),
                severity: detection.severity.clone(),
                provider: r.provider.clone(),
                model: r.model.clone(),
                user: r.user.clone(),
                file: r
                    .all_file_paths()
                    .first()
                    .map(|f| relative_path(f))
                    .unwrap_or_default(),
                timestamp: r.timestamp.format("%Y-%m-%d %H:%M:%S").to_string(),
                deployment: classification.deployment.clone(),
                receipt_id: r.id.clone(),
            });
        }

        // Check conversation turns
        if let Some(ref turns) = r.conversation {
            for t in turns {
                if t.role == "user" {
                    let turn_report = redact::redact_with_report(&t.content);
                    for detection in &turn_report.detections {
                        exposures.push(SecretExposure {
                            secret_type: detection.secret_type.clone(),
                            severity: detection.severity.clone(),
                            provider: r.provider.clone(),
                            model: r.model.clone(),
                            user: r.user.clone(),
                            file: r
                                .all_file_paths()
                                .first()
                                .map(|f| relative_path(f))
                                .unwrap_or_default(),
                            timestamp: r.timestamp.format("%Y-%m-%d %H:%M:%S").to_string(),
                            deployment: classification.deployment.clone(),
                            receipt_id: r.id.clone(),
                        });
                    }
                }
            }
        }
    }

    // Separate cloud vs local exposures
    let cloud_exposures: Vec<_> = exposures
        .iter()
        .filter(|e| e.deployment == ModelDeployment::Cloud)
        .collect();
    let local_exposures: Vec<_> = exposures
        .iter()
        .filter(|e| e.deployment == ModelDeployment::Local)
        .collect();

    // Generate report
    let mut md = String::new();
    md.push_str("# BlamePrompt Secret Rotation Alert Report\n\n");
    md.push_str(&format!("> Generated: {}\n\n", now));

    // Summary
    md.push_str("## Summary\n\n");
    md.push_str("| Metric | Value |\n");
    md.push_str("|--------|-------|\n");
    md.push_str(&format!(
        "| Total receipts scanned | {} |\n",
        all_receipts.len()
    ));
    md.push_str(&format!(
        "| Secrets detected in prompts | {} |\n",
        exposures.len()
    ));
    md.push_str(&format!(
        "| Secrets sent to cloud APIs | {} |\n",
        cloud_exposures.len()
    ));
    md.push_str(&format!(
        "| Secrets processed locally | {} |\n\n",
        local_exposures.len()
    ));

    if cloud_exposures.is_empty() && local_exposures.is_empty() {
        md.push_str(
            "**No secrets detected in stored prompts.** The redaction engine appears to be\n",
        );
        md.push_str("effectively filtering sensitive data. No rotation needed.\n\n");
    }

    // Cloud exposures — HIGH PRIORITY
    if !cloud_exposures.is_empty() {
        md.push_str("## ROTATION REQUIRED — Secrets Sent to Cloud APIs\n\n");
        md.push_str(
            "The following secrets were detected in prompts sent to **external AI providers**.\n",
        );
        md.push_str("These secrets should be **rotated immediately** as they may have been logged by the provider.\n\n");

        md.push_str("| Secret Type | Severity | Provider | Model | User | File | When |\n");
        md.push_str("|-------------|----------|----------|-------|------|------|------|\n");
        for e in &cloud_exposures {
            md.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} | {} |\n",
                e.secret_type, e.severity, e.provider, e.model, e.user, e.file, e.timestamp
            ));
        }
        md.push('\n');

        // Group by secret type for rotation instructions
        let mut rotation_types: HashMap<String, usize> = HashMap::new();
        for e in &cloud_exposures {
            *rotation_types.entry(e.secret_type.clone()).or_insert(0) += 1;
        }

        md.push_str("### Rotation Instructions\n\n");
        for (secret_type, count) in &rotation_types {
            let instructions = match secret_type.as_str() {
                "API_KEY" => "1. Generate a new API key from your provider dashboard\n2. Update all references in environment variables and secret managers\n3. Revoke the old key\n4. Verify services still work with the new key",
                "AWS_KEY" => "1. Go to AWS IAM console\n2. Create new access key for the affected user\n3. Update AWS credentials in all environments\n4. Deactivate and delete the old key\n5. Review CloudTrail for unauthorized usage",
                "BEARER_TOKEN" => "1. Invalidate/revoke the exposed token\n2. Generate a new token from the authentication provider\n3. Update all services using this token\n4. Review access logs for unauthorized usage",
                "PASSWORD" => "1. Change the password immediately\n2. Update all references in secret managers\n3. Review access logs for the affected account\n4. Enable MFA if not already enabled",
                "TOKEN" => "1. Revoke the exposed token\n2. Generate a new token\n3. Update all services and configurations\n4. Review for unauthorized access",
                "HIGH_ENTROPY" => "1. Identify what this secret is (API key, token, password, etc.)\n2. Rotate it following the appropriate procedure above\n3. Add a custom redaction pattern to `.blamepromptrc` to catch similar secrets",
                _ => "1. Identify the secret type and its origin\n2. Rotate/regenerate the credential\n3. Update all references\n4. Add custom redaction patterns to prevent future exposure",
            };
            md.push_str(&format!(
                "#### {} ({} occurrence(s))\n\n{}\n\n",
                secret_type, count, instructions
            ));
        }
    }

    // Local exposures — lower priority
    if !local_exposures.is_empty() {
        md.push_str("## Advisory — Secrets Processed Locally\n\n");
        md.push_str(
            "These secrets were processed by **local AI models**. While they were not sent to\n",
        );
        md.push_str("external services, they are stored in Git Notes and should still be handled carefully.\n\n");

        md.push_str("| Secret Type | Severity | Model | User | File |\n");
        md.push_str("|-------------|----------|-------|------|------|\n");
        for e in &local_exposures {
            md.push_str(&format!(
                "| {} | {} | {} | {} | {} |\n",
                e.secret_type, e.severity, e.model, e.user, e.file
            ));
        }
        md.push('\n');
    }

    // Prevention recommendations
    md.push_str("## Prevention\n\n");
    md.push_str("To prevent future secret exposure to AI providers:\n\n");
    md.push_str("1. **Enhance redaction patterns** — Add custom patterns to `.blamepromptrc`:\n");
    md.push_str("   ```toml\n");
    md.push_str("   [[redaction.custom_patterns]]\n");
    md.push_str("   pattern = \"YOUR_CUSTOM_PATTERN\"\n");
    md.push_str("   replacement = \"[REDACTED_CUSTOM]\"\n");
    md.push_str("   ```\n");
    md.push_str("2. **Use local models** — Route sensitive work through Ollama or LM Studio.\n");
    md.push_str(
        "3. **Pre-commit checks** — Run `blameprompt secret-rotation` before each release.\n",
    );
    md.push_str("4. **Environment isolation** — Never store secrets in source code; use environment variables.\n");
    md.push_str("5. **Provider data policies** — Review AI provider data retention policies and opt out of training.\n\n");

    md.push_str("---\n\n");
    md.push_str("*Generated by [BlamePrompt](https://github.com/ekaanth/blameprompt) — Secret Rotation Alert System*\n");

    match std::fs::write(output, &md) {
        Ok(_) => println!("Secret rotation alert report written to {}", output),
        Err(e) => eprintln!("Error writing report: {}", e),
    }
}
