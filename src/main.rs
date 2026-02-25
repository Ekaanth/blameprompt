mod commands;
mod core;
mod git;
mod integrations;

use clap::{Parser, Subcommand};

/// BlamePrompt: Track AI-generated code provenance via Git Notes.
/// No API key needed — hooks into Claude Code's native session data.
#[derive(Parser)]
#[command(name = "blameprompt", version = env!("CARGO_PKG_VERSION"), about = "Track AI-generated code in git")]
struct Cli {
    /// Enable verbose debug output
    #[arg(long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Called by Claude Code hooks to capture AI receipts (internal)
    Checkpoint {
        /// Agent name (e.g., "claude")
        agent: String,
        /// Read hook input from stdin
        #[arg(long, default_value = "stdin")]
        hook_input: String,
    },

    /// Initialize BlamePrompt in the current repo or globally
    Init {
        /// Configure git template for all future repos
        #[arg(long)]
        global: bool,
    },

    /// Install Claude Code + git hooks (legacy, same as 'init')
    InstallHooks,

    /// Remove all BlamePrompt hooks and data
    Uninstall {
        /// Keep Git Notes (receipt history) when uninstalling
        #[arg(long)]
        keep_notes: bool,
        /// Remove everything including Git Notes and binary info
        #[arg(long)]
        purge: bool,
    },

    /// Show line-by-line AI/human attribution for a file
    Blame {
        /// File to analyze
        file: String,
        /// Output format: table, json
        #[arg(long, default_value = "table")]
        format: String,
    },

    /// Display all AI receipts attached to a specific commit
    Show {
        /// Commit SHA (full or short)
        commit: String,
        /// Output format: table, json
        #[arg(long, default_value = "table")]
        format: String,
    },

    /// Search across stored prompts
    Search {
        /// Search query
        query: String,
        /// Maximum number of results (default: 50)
        #[arg(long, default_value = "50")]
        limit: usize,
        /// Output format: table, json
        #[arg(long, default_value = "table")]
        format: String,
    },

    /// Show complete AI audit trail with filters
    Audit {
        /// Start date filter (e.g., 2026-01-01)
        #[arg(long)]
        from: Option<String>,
        /// End date filter (e.g., 2026-02-09)
        #[arg(long)]
        to: Option<String>,
        /// Filter by author name
        #[arg(long)]
        author: Option<String>,
        /// Output format: md, table, json, csv
        #[arg(long, default_value = "md")]
        format: String,
        /// Include uncommitted/staged receipts
        #[arg(long)]
        include_uncommitted: bool,
    },

    /// Show aggregated AI usage statistics
    Analytics {
        /// Export format: json, csv
        #[arg(long)]
        export: Option<String>,
    },

    /// Generate comprehensive markdown report
    Report {
        /// Output file path
        #[arg(long, default_value = "./blameprompt-report.md")]
        output: String,
        /// Start date filter
        #[arg(long)]
        from: Option<String>,
        /// End date filter
        #[arg(long)]
        to: Option<String>,
        /// Filter by author name
        #[arg(long)]
        author: Option<String>,
        /// Include uncommitted/staged receipts
        #[arg(long)]
        include_uncommitted: bool,
    },

    /// Show annotated diff with AI/human attribution
    Diff {
        /// Commit reference to annotate (default: working tree diff)
        commit: Option<String>,
    },

    /// Install transparent git wrapper (auto-attaches receipts on every commit)
    InstallGitWrap,

    /// Remap BlamePrompt notes after rebase/amend (called by post-rewrite hook, internal)
    RebaseNotes,

    /// Push BlamePrompt notes to origin
    Push,

    /// Fetch BlamePrompt notes from origin
    Pull,

    /// Dry-run the redaction engine on a file
    Redact {
        /// File to test redaction on
        #[arg(long)]
        test: String,
    },

    /// Import a Claude Code JSONL transcript
    Record {
        /// Path to the JSONL session transcript
        #[arg(long)]
        session: String,
        /// AI provider name (claude, cursor, copilot, openai …)
        #[arg(long)]
        provider: Option<String>,
    },

    /// Import recent AI chat sessions from Cursor IDE
    RecordCursor {
        /// Path to a specific Cursor workspace storage directory or state.vscdb
        #[arg(long)]
        workspace: Option<String>,
    },

    /// Manage the local SQLite cache
    Cache {
        #[command(subcommand)]
        action: CacheAction,
    },

    /// Scan AI model licenses for compliance issues
    LicenseScan {
        /// Output file path
        #[arg(long, default_value = "./blameprompt-license-scan.md")]
        output: String,
    },

    /// Assess AI supply chain risk score
    SupplyChainRisk {
        /// Output file path
        #[arg(long, default_value = "./blameprompt-supply-chain-risk.md")]
        output: String,
    },

    /// Scan AI-generated code for vulnerabilities (SAST)
    VulnScan {
        /// Output file path
        #[arg(long, default_value = "./blameprompt-vuln-scan.md")]
        output: String,
    },

    /// Detect prompt injection patterns in AI-generated code
    PromptInjection {
        /// Output file path
        #[arg(long, default_value = "./blameprompt-prompt-injection.md")]
        output: String,
    },

    /// Alert on secrets that may need rotation after AI exposure
    SecretRotation {
        /// Output file path
        #[arg(long, default_value = "./blameprompt-secret-rotation.md")]
        output: String,
    },

    /// Print count of staged receipts (used by git hooks, internal)
    StagingCount,

    /// Attach staged receipts to HEAD as git notes and clear staging (used by git hooks, internal)
    Attach,

    /// Export blameprompt notes for a commit to Agent Trace v0.1.0 format
    ExportAgentTrace {
        /// Commit reference (default: HEAD)
        commit: Option<String>,
    },

    /// Display Agent Trace v0.1.0 record for a commit
    ImportAgentTrace {
        /// Commit reference (default: HEAD)
        commit: Option<String>,
    },

    /// Post AI attribution summary as a GitHub PR comment
    GithubComment {
        /// PR number to comment on (auto-detected from current branch if omitted)
        #[arg(long)]
        pr: Option<u32>,
        /// Repository slug (owner/repo, auto-detected from remote if omitted)
        #[arg(long)]
        repo: Option<String>,
    },

    /// Show line-by-line AI provenance for a file
    CheckProvenance {
        /// File to check
        file: String,
        /// Show provenance for a specific line number
        #[arg(long)]
        line: Option<u32>,
    },
}

#[derive(Subcommand)]
enum CacheAction {
    /// Sync Git Notes into the local SQLite cache for fast queries
    Sync,
}

/// Get the blob SHA stored in HEAD for a given file path.
fn get_head_blob(file_path: &str) -> Option<String> {
    let spec = format!("HEAD:{}", file_path);
    std::process::Command::new("git")
        .args(["rev-parse", &spec])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Retrieve lines from a git blob by SHA.
fn get_blob_lines(blob_sha: &str) -> Vec<String> {
    std::process::Command::new("git")
        .args(["cat-file", "-p", blob_sha])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.lines().map(String::from).collect())
        .unwrap_or_default()
}

/// Count how many lines from `staging_blob` are present in `head_blob` (accepted)
/// vs absent (overridden by human edits).
fn count_accepted_overridden(staging_blob: &str, head_blob: &str) -> (u32, u32) {
    let staging_lines = get_blob_lines(staging_blob);
    let head_lines = get_blob_lines(head_blob);

    let head_set: std::collections::HashSet<&str> =
        head_lines.iter().map(|s| s.as_str()).collect();

    let mut accepted = 0u32;
    let mut overridden = 0u32;
    for line in &staging_lines {
        if head_set.contains(line.as_str()) {
            accepted += 1;
        } else {
            overridden += 1;
        }
    }
    (accepted, overridden)
}

/// Enrich receipts with `accepted_lines` / `overridden_lines` by comparing the
/// blob hashes captured at PostToolUse time against the blobs actually committed to HEAD.
fn compute_acceptance_stats(receipts: &mut [core::receipt::Receipt]) {
    for receipt in receipts.iter_mut() {
        let mut total_accepted = 0u32;
        let mut total_overridden = 0u32;
        let mut has_data = false;

        for fc in &receipt.files_changed {
            if let Some(ref staging_blob) = fc.blob_hash {
                if let Some(head_blob) = get_head_blob(&fc.path) {
                    has_data = true;
                    if head_blob == *staging_blob {
                        // File unchanged between AI write and commit — all additions accepted
                        total_accepted += fc.additions;
                    } else {
                        let (accepted, overridden) =
                            count_accepted_overridden(staging_blob, &head_blob);
                        total_accepted += accepted;
                        total_overridden += overridden;
                    }
                }
            }
        }

        if has_data {
            receipt.accepted_lines = Some(total_accepted);
            receipt.overridden_lines = Some(total_overridden);
        }
    }
}

fn main() {
    let cli = Cli::parse();

    // Auto-setup global hooks on first run after install
    // Skip auto-setup for uninstall (would re-create what we're removing)
    if !matches!(cli.command, Commands::Uninstall { .. }) {
        git::init_hooks::auto_setup();
    }

    match cli.command {
        Commands::Checkpoint { agent, hook_input } => {
            commands::checkpoint::run(&agent, &hook_input);
        }

        Commands::Init { global } => {
            if let Err(e) = git::init_hooks::run_init(global) {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }

        Commands::InstallHooks => {
            if let Err(e) = integrations::claude_hooks::install() {
                eprintln!("Error installing Claude Code hooks: {}", e);
                std::process::exit(1);
            }
            if let Err(e) = git::hooks::install_hooks() {
                eprintln!("Error installing git hooks: {}", e);
                std::process::exit(1);
            }
            println!();
            println!("  \x1b[1;32m✓\x1b[0m Claude Code hooks installed");
            println!("  \x1b[1;32m✓\x1b[0m Git hooks installed");
            println!();
            println!("  \x1b[2m───────────────────────────────────────────────\x1b[0m");
            println!();
            println!("  \x1b[1mShare receipts with your team:\x1b[0m");
            println!("    \x1b[36mblameprompt push\x1b[0m     Push receipts to remote");
            println!("    \x1b[36mblameprompt pull\x1b[0m     Fetch receipts from remote");
            println!();
        }

        Commands::Uninstall { keep_notes, purge } => {
            if let Err(e) = commands::uninstall::run(keep_notes, purge) {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }

        Commands::Blame { file, format } => {
            commands::blame::run(&file, &format);
        }

        Commands::Show { commit, format } => {
            commands::show::run(&commit, &format);
        }

        Commands::Search {
            query,
            limit,
            format,
        } => {
            commands::search::run(&query, limit, &format);
        }

        Commands::Audit {
            from,
            to,
            author,
            format,
            include_uncommitted,
        } => {
            commands::audit::run(
                from.as_deref(),
                to.as_deref(),
                author.as_deref(),
                &format,
                include_uncommitted,
            );
        }

        Commands::Analytics { export } => {
            commands::analytics::run(export.as_deref());
        }

        Commands::Report {
            output,
            from,
            to,
            author,
            include_uncommitted,
        } => {
            if let Err(e) = commands::report::generate_report(
                &output,
                from.as_deref(),
                to.as_deref(),
                author.as_deref(),
                include_uncommitted,
            ) {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }

        Commands::Diff { commit } => {
            commands::diff::run(commit.as_deref());
        }

        Commands::InstallGitWrap => {
            match git::wrap::install() {
                Ok(path) => {
                    let home = dirs::home_dir()
                        .map(|h| h.display().to_string())
                        .unwrap_or_else(|| "~".to_string());
                    println!();
                    println!(
                        "  \x1b[1;32m[done]\x1b[0m Git wrapper installed"
                    );
                    println!(
                        "         \x1b[2m→ {}\x1b[0m",
                        path.display()
                    );
                    println!(
                        "  \x1b[1;32m[done]\x1b[0m PATH export added to shell RC"
                    );
                    println!(
                        "         \x1b[2m→ {}/.blameprompt/bin:$PATH\x1b[0m",
                        home
                    );
                    println!();
                    println!(
                        "\x1b[1mReload your shell to activate:\x1b[0m  \x1b[36msource ~/.zshrc\x1b[0m"
                    );
                    println!(
                        "Every \x1b[36mgit commit\x1b[0m will now auto-attach AI receipts."
                    );
                    println!();
                }
                Err(e) => {
                    eprintln!("Error installing git wrapper: {}", e);
                    std::process::exit(1);
                }
            }
        }

        Commands::RebaseNotes => {
            commands::rebase_notes::run_from_stdin();
        }

        Commands::Push => {
            commands::sync::push();
        }

        Commands::Pull => {
            commands::sync::pull();
        }

        Commands::Redact { test } => {
            commands::redact_test::run(&test);
        }

        Commands::Record { session, provider } => {
            commands::record::run(&session, provider.as_deref());
        }

        Commands::RecordCursor { workspace } => {
            integrations::cursor::run_record_cursor(workspace.as_deref());
        }

        Commands::Cache { action } => match action {
            CacheAction::Sync => {
                if let Err(e) = core::db::sync_from_notes() {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            }
        },

        Commands::LicenseScan { output } => {
            commands::license_scan::run(&output);
        }

        Commands::SupplyChainRisk { output } => {
            commands::supply_chain::run(&output);
        }

        Commands::VulnScan { output } => {
            commands::vuln_scan::run(&output);
        }

        Commands::PromptInjection { output } => {
            commands::prompt_injection::run(&output);
        }

        Commands::SecretRotation { output } => {
            commands::secret_rotation::run(&output);
        }

        Commands::StagingCount => {
            let data = commands::staging::read_staging();
            println!("{}", data.receipts.len());
        }

        Commands::ExportAgentTrace { commit } => {
            integrations::agent_trace::run_export(commit.as_deref());
        }

        Commands::ImportAgentTrace { commit } => {
            integrations::agent_trace::run_import(commit.as_deref());
        }

        Commands::GithubComment { pr, repo } => {
            commands::github::run(pr, repo.as_deref());
        }

        Commands::CheckProvenance { file, line } => {
            commands::check_provenance::run(&file, line);
        }

        Commands::Attach => {
            let mut data = commands::staging::read_staging();
            if data.receipts.is_empty() {
                return;
            }
            // Compute accepted/overridden lines by comparing AI-written blobs against HEAD
            compute_acceptance_stats(&mut data.receipts);
            match git::notes::attach_receipts_to_head(&data) {
                Ok(()) => {
                    commands::staging::clear_staging();
                    let head_short = std::process::Command::new("git")
                        .args(["rev-parse", "--short", "HEAD"])
                        .output()
                        .ok()
                        .and_then(|o| String::from_utf8(o.stdout).ok())
                        .map(|s| s.trim().to_string())
                        .unwrap_or_else(|| "HEAD".to_string());
                    eprintln!(
                        "[BlamePrompt] {} receipt(s) attached to {}",
                        data.receipts.len(),
                        head_short
                    );
                }
                Err(e) => {
                    eprintln!("[BlamePrompt] Failed to attach receipts: {}", e);
                }
            }
        }
    }
}
