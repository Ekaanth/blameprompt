use crate::core::config;
use crate::core::redact;

pub fn run(file: &str) {
    let content = match std::fs::read_to_string(file) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error reading {}: {}", file, e);
            return;
        }
    };

    let cfg = config::load_config();
    let result = redact::redact_with_report_and_config(&content, &cfg.redaction);

    println!("Redaction Dry Run: {}", file);
    println!("{}", "=".repeat(40 + file.len()));
    println!();

    if result.detections.is_empty() {
        println!("No secrets detected.");
        return;
    }

    println!("Detections: {} secret(s) found", result.detections.len());
    println!();

    // Count by type
    let mut counts: std::collections::HashMap<String, (usize, String)> = std::collections::HashMap::new();
    for d in &result.detections {
        let entry = counts.entry(d.secret_type.clone()).or_insert((0, d.severity.clone()));
        entry.0 += 1;
    }

    let mut table = comfy_table::Table::new();
    table.set_header(vec!["Secret Type", "Count", "Severity"]);
    for (secret_type, (count, severity)) in &counts {
        table.add_row(vec![
            secret_type.as_str(),
            &count.to_string(),
            severity.as_str(),
        ]);
    }
    println!("{table}");

    println!("\n--- Redacted Output ---");
    println!("{}", result.redacted_text);
}
