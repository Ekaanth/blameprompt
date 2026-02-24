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
    },

    /// Display all AI receipts attached to a specific commit
    Show {
        /// Commit SHA (full or short)
        commit: String,
    },

    /// Search across stored prompts
    Search {
        /// Search query
        query: String,
        /// Maximum number of results (default: 50)
        #[arg(long, default_value = "50")]
        limit: usize,
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
}

#[derive(Subcommand)]
enum CacheAction {
    /// Sync Git Notes into the local SQLite cache for fast queries
    Sync,
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

        Commands::Blame { file } => {
            commands::blame::run(&file);
        }

        Commands::Show { commit } => {
            commands::show::run(&commit);
        }

        Commands::Search { query, limit } => {
            commands::search::run(&query, limit);
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

        Commands::Push => {
            commands::sync::push();
        }

        Commands::Pull => {
            commands::sync::pull();
        }

        Commands::Redact { test } => {
            commands::redact_test::run(&test);
        }

        Commands::Record { session } => {
            commands::record::run(&session, None);
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

        Commands::Attach => {
            let data = commands::staging::read_staging();
            if data.receipts.is_empty() {
                return;
            }
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
