use crate::commands::audit;
use crate::core::model_classifier::{self, ModelLicense};
use chrono::Utc;
use std::collections::HashMap;

/// Model license restrictions for commercial use.
struct LicenseRestriction {
    model_pattern: &'static str,
    license_name: &'static str,
    restriction: &'static str,
    severity: &'static str,
}

const LICENSE_RESTRICTIONS: &[LicenseRestriction] = &[
    LicenseRestriction {
        model_pattern: "llama",
        license_name: "Llama Community License",
        restriction: "Commercial use restricted above 700M monthly active users. Must include 'Built with Llama' attribution.",
        severity: "HIGH",
    },
    LicenseRestriction {
        model_pattern: "mistral",
        license_name: "Apache 2.0",
        restriction: "Permissive. Must include license notice and copyright attribution.",
        severity: "LOW",
    },
    LicenseRestriction {
        model_pattern: "mixtral",
        license_name: "Apache 2.0",
        restriction: "Permissive. Must include license notice and copyright attribution.",
        severity: "LOW",
    },
    LicenseRestriction {
        model_pattern: "codestral",
        license_name: "Mistral AI Non-Production License",
        restriction: "Non-production use only. Commercial deployment requires separate license.",
        severity: "CRITICAL",
    },
    LicenseRestriction {
        model_pattern: "deepseek",
        license_name: "DeepSeek License",
        restriction: "Open-weight model. Check specific version license for commercial use terms.",
        severity: "MEDIUM",
    },
    LicenseRestriction {
        model_pattern: "phi-",
        license_name: "MIT License",
        restriction: "Permissive. No significant commercial restrictions.",
        severity: "LOW",
    },
    LicenseRestriction {
        model_pattern: "qwen",
        license_name: "Tongyi Qianwen License",
        restriction: "Free for commercial use with registration. Must not use for illegal purposes.",
        severity: "MEDIUM",
    },
    LicenseRestriction {
        model_pattern: "gemma",
        license_name: "Gemma Terms of Use",
        restriction: "Free for commercial use. Must not redistribute model weights without permission.",
        severity: "LOW",
    },
    LicenseRestriction {
        model_pattern: "codellama",
        license_name: "Llama Community License",
        restriction: "Same as Llama: commercial use restricted above 700M MAU.",
        severity: "HIGH",
    },
];

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

    // Classify all models
    let mut model_issues: Vec<(String, String, String, String, String, Vec<String>)> = Vec::new();
    let mut models_seen: HashMap<String, Vec<String>> = HashMap::new();

    for r in &all_receipts {
        for f in r.all_file_paths() {
            models_seen
                .entry(r.model.clone())
                .or_default()
                .push(relative_path(&f));
        }
    }

    let mut flagged_count = 0;
    let mut warnings = Vec::new();

    for (model_id, files) in &models_seen {
        let classification = model_classifier::classify(model_id);

        // Check license restrictions
        let model_lower = model_id.to_lowercase();
        for restriction in LICENSE_RESTRICTIONS {
            if model_lower.contains(restriction.model_pattern) {
                flagged_count += 1;
                let unique_files: Vec<String> = {
                    let mut f = files.clone();
                    f.sort();
                    f.dedup();
                    f
                };
                model_issues.push((
                    model_id.clone(),
                    classification.display_name.clone(),
                    restriction.license_name.to_string(),
                    restriction.restriction.to_string(),
                    restriction.severity.to_string(),
                    unique_files.clone(),
                ));
                break;
            }
        }

        // Flag open-source models in potentially proprietary codebases
        if classification.license == ModelLicense::OpenSource {
            let unique_files: Vec<String> = {
                let mut f = files.clone();
                f.sort();
                f.dedup();
                f
            };
            warnings.push(format!(
                "Open-source model '{}' ({}) used in {} file(s): {}",
                classification.display_name,
                model_id,
                unique_files.len(),
                unique_files
                    .iter()
                    .take(5)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }

    // Generate markdown report
    let now = Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string();
    let mut md = String::new();

    md.push_str("# BlamePrompt License Compliance Scan\n\n");
    md.push_str(&format!("> Generated: {}\n\n", now));

    md.push_str("## Summary\n\n");
    md.push_str("| Metric | Value |\n");
    md.push_str("|--------|-------|\n");
    md.push_str(&format!(
        "| Total receipts scanned | {} |\n",
        all_receipts.len()
    ));
    md.push_str(&format!("| Unique models used | {} |\n", models_seen.len()));
    md.push_str(&format!("| License issues flagged | {} |\n", flagged_count));
    md.push_str(&format!(
        "| Open-source model warnings | {} |\n\n",
        warnings.len()
    ));

    // License issues
    if !model_issues.is_empty() {
        md.push_str("## License Issues\n\n");
        for (model_id, display, license, restriction, severity, files) in &model_issues {
            md.push_str(&format!("### {} (`{}`)\n\n", display, model_id));
            md.push_str(&format!("- **License**: {}\n", license));
            md.push_str(&format!("- **Severity**: {}\n", severity));
            md.push_str(&format!("- **Restriction**: {}\n", restriction));
            md.push_str(&format!("- **Files affected** ({}):\n", files.len()));
            for f in files {
                md.push_str(&format!("  - `{}`\n", f));
            }
            md.push('\n');
        }
    } else {
        md.push_str("## License Issues\n\nNo license issues found.\n\n");
    }

    // Model inventory
    md.push_str("## Model Inventory\n\n");
    md.push_str("| Model | Vendor | License | Deployment | Files |\n");
    md.push_str("|-------|--------|---------|------------|-------|\n");
    for (model_id, files) in &models_seen {
        let c = model_classifier::classify(model_id);
        let license_str = match c.license {
            ModelLicense::OpenSource => "Open Source",
            ModelLicense::ClosedSource => "Closed Source",
        };
        let mut unique: Vec<_> = files.clone();
        unique.sort();
        unique.dedup();
        md.push_str(&format!(
            "| {} | {} | {} | {:?} | {} |\n",
            c.display_name,
            c.vendor,
            license_str,
            c.deployment,
            unique.len()
        ));
    }
    md.push('\n');

    // Warnings
    if !warnings.is_empty() {
        md.push_str("## Open-Source Model Warnings\n\n");
        md.push_str("These models have open-source licenses. Verify compatibility with your codebase license:\n\n");
        for w in &warnings {
            md.push_str(&format!("- {}\n", w));
        }
        md.push('\n');
    }

    // Recommendations
    md.push_str("## Recommendations\n\n");
    if flagged_count > 0 {
        md.push_str("1. **Review flagged licenses** — Ensure your commercial use complies with model license terms.\n");
        md.push_str(
            "2. **Add attribution** — Some licenses (Llama, Apache 2.0) require attribution.\n",
        );
        md.push_str("3. **Check MAU thresholds** — Llama-family models restrict commercial use above 700M MAU.\n");
    }
    md.push_str(
        "4. **Document model usage** — Keep this scan as part of your compliance evidence.\n",
    );
    md.push_str("5. **Review periodically** — Model licenses can change between versions.\n\n");

    md.push_str("---\n\n*Generated by [BlamePrompt](https://github.com/ekaanth/blameprompt) License Compliance Scanner*\n");

    match std::fs::write(output, &md) {
        Ok(_) => println!("License compliance scan written to {}", output),
        Err(e) => eprintln!("Error writing report: {}", e),
    }
}
