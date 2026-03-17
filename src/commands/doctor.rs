use std::process::Command;

const GREEN: &str = "\x1b[1;32m";
const RED: &str = "\x1b[1;31m";
const CYAN: &str = "\x1b[1;36m";
const DIM: &str = "\x1b[2m";
const BOLD: &str = "\x1b[1m";
const RESET: &str = "\x1b[0m";

struct CheckResult {
    passed: bool,
    label: String,
}

impl CheckResult {
    fn pass(label: impl Into<String>) -> Self {
        Self {
            passed: true,
            label: label.into(),
        }
    }
    fn fail(label: impl Into<String>) -> Self {
        Self {
            passed: false,
            label: label.into(),
        }
    }
}

fn check_git_available() -> CheckResult {
    match Command::new("git").arg("--version").output() {
        Ok(output) if output.status.success() => {
            let version_str = String::from_utf8_lossy(&output.stdout);
            // Extract version number from "git version 2.43.0"
            let version = version_str
                .trim()
                .strip_prefix("git version ")
                .unwrap_or(version_str.trim());
            CheckResult::pass(format!("Git available ({})", version))
        }
        _ => CheckResult::fail("Git not found (install git first)"),
    }
}

fn check_inside_git_repo() -> CheckResult {
    match Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .output()
    {
        Ok(output) if output.status.success() => {
            CheckResult::pass("Inside git repository")
        }
        _ => CheckResult::fail("Not inside a git repository"),
    }
}

fn check_notes_namespace() -> CheckResult {
    // Check if the blameprompt notes ref exists
    match Command::new("git")
        .args(["notes", "--ref=refs/notes/blameprompt", "list"])
        .output()
    {
        Ok(output) if output.status.success() => {
            let note_count = String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter(|l| !l.trim().is_empty())
                .count();
            if note_count > 0 {
                CheckResult::pass(format!(
                    "BlamePrompt notes initialized ({} commit(s))",
                    note_count
                ))
            } else {
                CheckResult::pass("BlamePrompt notes initialized (empty)")
            }
        }
        _ => CheckResult::fail("BlamePrompt notes not initialized (run: blameprompt init)"),
    }
}

fn check_git_hooks_installed() -> CheckResult {
    // Check local .git/hooks/post-commit
    let local_hook = Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| {
            let git_dir = s.trim().to_string();
            let hook_path = std::path::Path::new(&git_dir).join("hooks").join("post-commit");
            if hook_path.exists() {
                std::fs::read_to_string(&hook_path)
                    .map(|c| c.contains("blameprompt"))
                    .unwrap_or(false)
            } else {
                false
            }
        })
        .unwrap_or(false);

    if local_hook {
        return CheckResult::pass("Git hooks installed (local)");
    }

    // Check global git template hooks
    let global_hook = Command::new("git")
        .args(["config", "--global", "--get", "init.templateDir"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| {
            let template_dir = s.trim().to_string();
            let hook_path = std::path::Path::new(&template_dir)
                .join("hooks")
                .join("post-commit");
            if hook_path.exists() {
                std::fs::read_to_string(&hook_path)
                    .map(|c| c.contains("blameprompt"))
                    .unwrap_or(false)
            } else {
                false
            }
        })
        .unwrap_or(false);

    if global_hook {
        return CheckResult::pass("Git hooks installed (global template)");
    }

    // Check if git wrapper is installed
    let home = dirs::home_dir();
    if let Some(ref h) = home {
        let wrapper = h.join(".blameprompt").join("bin").join("git");
        if wrapper.exists() {
            return CheckResult::pass("Git hooks installed (git wrapper)");
        }
    }

    CheckResult::fail("Git hooks not installed (run: blameprompt init)")
}

fn check_claude_hooks() -> CheckResult {
    let settings_path = match dirs::home_dir() {
        Some(h) => h.join(".claude").join("settings.json"),
        None => return CheckResult::fail("Claude Code hooks not configured (no home dir)"),
    };

    if !settings_path.exists() {
        return CheckResult::fail("Claude Code hooks not configured (no ~/.claude/settings.json)");
    }

    match std::fs::read_to_string(&settings_path) {
        Ok(content) if content.contains("blameprompt") => {
            CheckResult::pass("Claude Code hooks configured")
        }
        Ok(_) => CheckResult::fail(
            "Claude Code hooks not configured (run: blameprompt init)",
        ),
        Err(_) => CheckResult::fail("Claude Code hooks not configured (cannot read settings)"),
    }
}

fn check_sqlite_cache() -> CheckResult {
    let db_path = match dirs::home_dir() {
        Some(h) => h.join(".blameprompt").join("prompts.db"),
        None => return CheckResult::fail("SQLite cache not found (no home dir)"),
    };

    if !db_path.exists() {
        return CheckResult::fail("SQLite cache not found (run: blameprompt cache sync)");
    }

    // Try to count receipts
    match rusqlite::Connection::open(&db_path) {
        Ok(conn) => {
            let count: i64 = conn
                .query_row("SELECT COUNT(*) FROM receipts", [], |row| row.get(0))
                .unwrap_or(0);
            CheckResult::pass(format!("SQLite cache exists ({} receipts)", format_number(count)))
        }
        Err(_) => CheckResult::pass("SQLite cache exists (cannot read count)"),
    }
}

fn format_number(n: i64) -> String {
    let s = n.to_string();
    let bytes = s.as_bytes();
    let mut result = String::new();
    for (i, &b) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i) % 3 == 0 {
            result.push(',');
        }
        result.push(b as char);
    }
    result
}

fn check_logged_in() -> CheckResult {
    match crate::core::auth::load() {
        Some(creds) => CheckResult::pass(format!("Logged in as @{}", creds.username)),
        None => CheckResult::fail("Not logged in (run: blameprompt login)"),
    }
}

pub fn run() {
    let version = env!("CARGO_PKG_VERSION");

    println!();
    println!("  {BOLD}BlamePrompt Doctor{RESET} {DIM}v{version}{RESET}");
    println!();

    let checks = vec![
        check_git_available(),
        check_inside_git_repo(),
        check_notes_namespace(),
        check_git_hooks_installed(),
        check_claude_hooks(),
        check_sqlite_cache(),
        check_logged_in(),
    ];

    let mut passed = 0;
    let total = checks.len();

    for check in &checks {
        if check.passed {
            passed += 1;
            println!("  {GREEN}\u{2713}{RESET} {}", check.label);
        } else {
            println!("  {RED}\u{2717}{RESET} {}", check.label);
        }
    }

    println!();
    if passed == total {
        println!(
            "  {GREEN}{passed}/{total} checks passed{RESET} {DIM}\u{2014} all good!{RESET}"
        );
    } else {
        println!(
            "  {CYAN}{passed}/{total} checks passed{RESET}"
        );
    }
    println!();
}
