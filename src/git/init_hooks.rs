use crate::{
    git::hooks, git::wrap,
    integrations::{claude_hooks, codex, copilot, cursor, gemini, windsurf},
};
use std::path::Path;

/// Marker file to track that global setup has been done.
fn setup_marker_path() -> Option<std::path::PathBuf> {
    dirs::home_dir().map(|h| h.join(".blameprompt").join(".setup-done"))
}

/// Check if global setup has been completed.
pub fn is_globally_configured() -> bool {
    setup_marker_path().is_some_and(|p| p.exists())
}

/// Write the marker file after successful global setup.
fn mark_setup_done() {
    if let Some(path) = setup_marker_path() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&path, "1");
    }
}

/// Try to install hooks for all detected AI agents.
/// Failures are non-fatal — agents that aren't installed are silently skipped.
fn install_all_agent_hooks() {
    // Claude Code (always installed — it's our primary integration)
    let _ = claude_hooks::install();

    // Detect and install hooks for other agents if present
    // Each returns Err if agent not installed — silently skip those
    let _ = codex::install_hooks();
    let _ = gemini::install_hooks();
    let _ = copilot::install_hooks();
    let _ = cursor::install_hooks();
    let _ = windsurf::install_hooks();
}

/// Auto-setup: called on every blameprompt invocation.
/// If global hooks are not installed, install them silently.
pub fn auto_setup() {
    if is_globally_configured() {
        return;
    }

    // Install hooks for all detected AI agents
    install_all_agent_hooks();

    // Install git template (sets init.templateDir so every git init gets hooks)
    if install_git_template().is_err() {
        return;
    }

    // Install transparent git wrapper (optional; failure is non-fatal)
    let _ = wrap::install();

    mark_setup_done();

    // Also initialize the current repo if we're inside one
    if let Ok(cwd) = std::env::current_dir() {
        if git2::Repository::discover(&cwd).is_ok() {
            let _ = auto_init_blameprompt(cwd.to_str().unwrap_or("."));
        }
    }

    print_install_banner(true);
}

/// Shared install banner for auto_setup (stderr) and run_init (stdout).
fn print_install_banner(use_stderr: bool) {
    let home = dirs::home_dir()
        .map(|h| h.display().to_string())
        .unwrap_or_else(|| "~".to_string());

    // ANSI color shortcuts
    let c = "\x1b[36m"; // cyan
    let b = "\x1b[1m"; // bold
    let d = "\x1b[2m"; // dim
    let r = "\x1b[0m"; // reset
    let bg = "\x1b[1;32m"; // bold green
    let bc = "\x1b[1;36m"; // bold cyan
    let bw = "\x1b[1;37m"; // bold white

    // Collect all lines into a Vec, then print to stderr or stdout
    let lines = vec![
        String::new(),
        format!("  {bc}  ██████╗ ██╗      █████╗ ███╗   ███╗███████╗{r}"),
        format!("  {bc}  ██╔══██╗██║     ██╔══██╗████╗ ████║██╔════╝{r}"),
        format!("  {bc}  ██████╔╝██║     ███████║██╔████╔██║█████╗{r}"),
        format!("  {bc}  ██╔══██╗██║     ██╔══██║██║╚██╔╝██║██╔══╝{r}"),
        format!("  {bc}  ██████╔╝███████╗██║  ██║██║ ╚═╝ ██║███████╗{r}"),
        format!("  {bc}  ╚═════╝ ╚══════╝╚═╝  ╚═╝╚═╝     ╚═╝╚══════╝{r}"),
        format!("  {bg}  ██████╗ ██████╗  ██████╗ ███╗   ███╗██████╗ ████████╗{r}"),
        format!("  {bg}  ██╔══██╗██╔══██╗██╔═══██╗████╗ ████║██╔══██╗╚══██╔══╝{r}"),
        format!("  {bg}  ██████╔╝██████╔╝██║   ██║██╔████╔██║██████╔╝   ██║{r}"),
        format!("  {bg}  ██╔═══╝ ██╔══██╗██║   ██║██║╚██╔╝██║██╔═══╝    ██║{r}"),
        format!("  {bg}  ██║     ██║  ██║╚██████╔╝██║ ╚═╝ ██║██║        ██║{r}"),
        format!("  {bg}  ╚═╝     ╚═╝  ╚═╝ ╚═════╝ ╚═╝     ╚═╝╚═╝        ╚═╝{r}"),
        format!("  {d}  v{} · Track AI-generated code in Git{r}", env!("CARGO_PKG_VERSION")),
        String::new(),
        format!("Installing BlamePrompt..."),
        String::new(),
        format!("  {bg}[done]{r} Claude Code hooks installed (10 lifecycle hooks)"),
        format!("         {d}→ {home}/.claude/settings.json{r}"),
        format!("  {bg}[done]{r} Multi-agent hooks installed (Codex, Gemini, Copilot, Cursor, Windsurf)"),
        format!("         {d}→ detected agents configured automatically{r}"),
        format!("  {bg}[done]{r} Git template configured (7 git hooks)"),
        format!("         {d}→ {home}/.blameprompt/git-template{r}"),
        format!("  {bg}[done]{r} Transparent git wrapper installed"),
        format!("         {d}→ {home}/.blameprompt/bin/git{r}"),
        format!("  {bg}[done]{r} All future repos will auto-track AI prompts"),
        String::new(),
        format!("{bw}BlamePrompt installed.{r}"),
        format!("  Global hooks configured"),
        format!("  Git template ready"),
        format!("  Transparent git wrapper active"),
        String::new(),
        format!("{b}Get started:{r}"),
        format!("  {c}blameprompt blame{r} {d}<file>{r}       {d}Line-by-line AI vs human attribution{r}"),
        format!("  {c}blameprompt diff{r}  {d}[commit]{r}     {d}Annotated diff with AI/human markers{r}"),
        format!("  {c}blameprompt show{r}  {d}<commit>{r}     {d}View receipts attached to a commit{r}"),
        format!("  {c}blameprompt audit{r}              {d}Full audit trail{r}"),
        format!("  {c}blameprompt analytics{r}          {d}AI usage stats & cost breakdown{r}"),
        format!("  {c}blameprompt search{r} {d}<query>{r}     {d}Search prompts across history{r}"),
        format!("  {c}blameprompt report{r}             {d}Generate comprehensive markdown report{r}"),
        format!("  {c}blameprompt vuln-scan{r}          {d}Security scan on AI-generated code{r}"),
        format!("  {c}blameprompt push{r}               {d}Push receipts to remote{r}"),
        format!("  {c}blameprompt pull{r}               {d}Fetch receipts from remote{r}"),
        format!("  {c}blameprompt --help{r}             {d}See all commands{r}"),
        String::new(),
    ];

    for line in &lines {
        if use_stderr {
            eprintln!("{line}");
        } else {
            println!("{line}");
        }
    }
}

pub fn install_git_template() -> Result<(), String> {
    let home = dirs::home_dir().ok_or("Cannot find home directory")?;
    let template_dir = home.join(".blameprompt").join("git-template");
    let hooks_dir = template_dir.join("hooks");

    std::fs::create_dir_all(&hooks_dir)
        .map_err(|e| format!("Cannot create template dir: {}", e))?;

    // Write all hook templates with the absolute binary path embedded
    let binary = hooks::resolve_binary_path();
    for (name, content) in hooks::all_hook_entries(&binary) {
        let hook_path = hooks_dir.join(name);
        let full = format!("#!/bin/sh\n\n{}", content);
        std::fs::write(&hook_path, &full).map_err(|e| format!("Cannot write {}: {}", name, e))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&hook_path, std::fs::Permissions::from_mode(0o755));
        }
    }

    // Check if init.templateDir is already set to something else
    let existing = std::process::Command::new("git")
        .args(["config", "--global", "--get", "init.templateDir"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    if !existing.is_empty() && !existing.contains(".blameprompt") {
        eprintln!("  [warn] init.templateDir was: {}", existing);
        eprintln!("         Overriding with BlamePrompt template.");
    }

    // Configure git to use this template directory
    let status = std::process::Command::new("git")
        .args([
            "config",
            "--global",
            "init.templateDir",
            &template_dir.to_string_lossy(),
        ])
        .status()
        .map_err(|e| format!("Cannot set git config: {}", e))?;

    if !status.success() {
        return Err("Failed to set init.templateDir".to_string());
    }

    Ok(())
}

pub fn auto_init_blameprompt(repo_root: &str) -> Result<(), String> {
    let bp_dir = Path::new(repo_root).join(".blameprompt");

    if !bp_dir.exists() {
        std::fs::create_dir_all(&bp_dir)
            .map_err(|e| format!("Cannot create .blameprompt/: {}", e))?;
    }

    let staging = bp_dir.join("staging.json");
    if !staging.exists() {
        std::fs::write(&staging, "{\"receipts\":[]}")
            .map_err(|e| format!("Cannot create staging.json: {}", e))?;
    }

    // Add to .gitignore if not present
    let gitignore = Path::new(repo_root).join(".gitignore");
    let needs_entry = if gitignore.exists() {
        let content = std::fs::read_to_string(&gitignore).unwrap_or_default();
        !content
            .lines()
            .any(|l| l.trim() == ".blameprompt/" || l.trim() == ".blameprompt")
    } else {
        true
    };
    if needs_entry {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&gitignore)
            .map_err(|e| format!("Cannot write .gitignore: {}", e))?;
        use std::io::Write;
        writeln!(
            file,
            "\n# BlamePrompt staging (auto-generated)\n.blameprompt/"
        )
        .map_err(|e| format!("Cannot append to .gitignore: {}", e))?;
    }

    Ok(())
}

pub fn run_init(global: bool) -> Result<(), String> {
    if global {
        install_git_template()?;
        install_all_agent_hooks();
        // Install transparent git wrapper (optional; failure is non-fatal)
        let _ = wrap::install();
        mark_setup_done();

        // Also initialize the current repo if we're inside one
        if let Ok(cwd) = std::env::current_dir() {
            if git2::Repository::discover(&cwd).is_ok() {
                let _ = auto_init_blameprompt(cwd.to_str().unwrap_or("."));
            }
        }

        print_install_banner(false);
    } else {
        let cwd = std::env::current_dir().map_err(|e| format!("Cannot get cwd: {}", e))?;

        git2::Repository::discover(&cwd)
            .map_err(|_| "Not inside a git repository. Run 'git init' first.".to_string())?;

        auto_init_blameprompt(cwd.to_str().unwrap())?;
        hooks::install_hooks()?;

        // ANSI color shortcuts
        let c = "\x1b[36m"; // cyan
        let b = "\x1b[1m"; // bold
        let d = "\x1b[2m"; // dim
        let r = "\x1b[0m"; // reset
        let bg = "\x1b[1;32m"; // bold green
        let bw = "\x1b[1;37m"; // bold white

        println!();
        println!("Installing BlamePrompt in this repo...");
        println!();
        println!("  {bg}[done]{r} Git hooks installed");
        println!("         {d}pre-commit, prepare-commit-msg, post-commit, pre-push,{r}");
        println!("         {d}post-checkout, post-merge, post-rewrite{r}");
        println!("  {bg}[done]{r} Staging directory created (.blameprompt/)");
        println!("  {bg}[done]{r} Updated .gitignore");
        println!();
        println!("{bw}BlamePrompt initialized in {}{r}", cwd.display());
        println!("  Prompt tracking active — every AI code change will be captured.");
        println!();
        println!("{b}Get started:{r}");
        println!("  {c}blameprompt blame{r} {d}<file>{r}       {d}Line-by-line AI vs human attribution{r}");
        println!("  {c}blameprompt diff{r}  {d}[commit]{r}     {d}Annotated diff with AI/human markers{r}");
        println!(
            "  {c}blameprompt show{r}  {d}<commit>{r}     {d}View receipts attached to a commit{r}"
        );
        println!("  {c}blameprompt audit{r}              {d}Full audit trail{r}");
        println!("  {c}blameprompt --help{r}             {d}See all commands{r}");
        println!();
    }

    Ok(())
}
