use std::io::Write;
use std::path::Path;

pub fn run(keep_notes: bool, purge: bool) -> Result<(), String> {
    // ANSI color shortcuts
    let bg = "\x1b[1;32m";  // bold green
    let br = "\x1b[1;31m";  // bold red
    let by = "\x1b[1;33m";  // bold yellow
    let bc = "\x1b[1;36m";  // bold cyan
    let bw = "\x1b[1;37m";  // bold white
    let b  = "\x1b[1m";     // bold
    let d  = "\x1b[2m";     // dim
    let r  = "\x1b[0m";     // reset

    println!();
    println!("  {bw}Uninstalling BlamePrompt...{r}");
    println!();

    // Purge confirmation
    if purge {
        let note_count = count_git_notes();
        println!("  {br}WARNING:{r} {b}This will permanently delete:{r}");
        println!("    {d}-{r} All {by}{}{r} Git Note(s) {d}(receipt history){r}", note_count);
        println!("    {d}-{r} SQLite database {d}(~/.blameprompt/prompts.db){r}");
        println!("    {d}-{r} All hooks {d}(Claude Code + git, globally and in this repo){r}");
        println!("    {d}-{r} Git template directory");
        println!();
        print!("  {b}Continue?{r} {d}[y/N]{r} ");
        std::io::stdout().flush().ok();
        let mut input = String::new();
        std::io::stdin().read_line(&mut input).unwrap_or(0);
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("  {d}Aborted.{r}");
            return Ok(());
        }
        println!();
    }

    // 1. Remove Claude Code hooks from ~/.claude/settings.json
    crate::integrations::claude_hooks::uninstall()?;

    // 2. Remove git hooks from current repo
    crate::git::hooks::uninstall_hooks()?;

    // 3. Remove staging directory from current repo
    remove_staging_dir()?;

    // 4. Remove .gitignore entry from current repo
    remove_gitignore_entry()?;

    // 5. Reset git init.templateDir (stops future repos from getting hooks)
    remove_git_template()?;

    // 6. Remove ~/.blameprompt/ (SQLite cache, git template, setup marker)
    remove_global_data()?;

    // 7. Remove Git Notes (only with --purge)
    if purge && !keep_notes {
        remove_git_notes()?;
    } else {
        println!("  {by}[kept]{r} Git Notes {d}(refs/notes/blameprompt){r}");
        println!("         {d}→ To remove:{r} {bc}blameprompt uninstall --purge{r}");
    }

    // 8. Show binary removal instructions (only with --purge)
    if purge {
        remove_binary()?;
    }

    println!();
    println!("{bw}BlamePrompt uninstalled.{r}");
    println!("  {bg}✓{r} Global hooks removed");
    println!("  {bg}✓{r} Git template removed");
    if !purge {
        println!("  {bg}✓{r} Git Notes preserved {d}(your receipt history is still in the repo){r}");
    }
    println!();

    Ok(())
}

fn remove_staging_dir() -> Result<(), String> {
    let staging = Path::new(".blameprompt");
    if staging.exists() {
        std::fs::remove_dir_all(staging)
            .map_err(|e| format!("Cannot remove .blameprompt/: {}", e))?;
        println!("  \x1b[1;32m[done]\x1b[0m Removed .blameprompt/ directory");
    }
    Ok(())
}

fn remove_gitignore_entry() -> Result<(), String> {
    let gitignore = Path::new(".gitignore");
    if !gitignore.exists() {
        return Ok(());
    }

    let content = std::fs::read_to_string(gitignore)
        .map_err(|e| format!("Cannot read .gitignore: {}", e))?;

    let cleaned: Vec<&str> = content
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            trimmed != ".blameprompt/" && trimmed != ".blameprompt"
                && !trimmed.contains("# BlamePrompt staging")
        })
        .collect();

    if cleaned.len() < content.lines().count() {
        let mut result = cleaned.join("\n");
        while result.ends_with("\n\n") {
            result.pop();
        }
        if !result.ends_with('\n') {
            result.push('\n');
        }
        std::fs::write(gitignore, result)
            .map_err(|e| format!("Cannot write .gitignore: {}", e))?;
        println!("  \x1b[1;32m[done]\x1b[0m Cleaned .gitignore");
    }
    Ok(())
}

fn remove_global_data() -> Result<(), String> {
    let home = dirs::home_dir().ok_or("Cannot find home directory")?;
    let global_dir = home.join(".blameprompt");
    if global_dir.exists() {
        std::fs::remove_dir_all(&global_dir)
            .map_err(|e| format!("Cannot remove ~/.blameprompt/: {}", e))?;
        println!("  \x1b[1;32m[done]\x1b[0m Removed ~/.blameprompt/");
    }
    Ok(())
}

fn remove_git_template() -> Result<(), String> {
    let output = std::process::Command::new("git")
        .args(["config", "--global", "--get", "init.templateDir"])
        .output();

    if let Ok(out) = output {
        let current = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if current.contains(".blameprompt") {
            let _ = std::process::Command::new("git")
                .args(["config", "--global", "--unset", "init.templateDir"])
                .status();
            println!("  \x1b[1;32m[done]\x1b[0m Reset git init.templateDir");
        }
    }
    Ok(())
}

fn remove_git_notes() -> Result<(), String> {
    let list = std::process::Command::new("git")
        .args(["notes", "--ref", "refs/notes/blameprompt", "list"])
        .output();

    if let Ok(output) = list {
        if output.status.success() {
            let notes = String::from_utf8_lossy(&output.stdout);
            let count = notes.lines().count();

            let _ = std::process::Command::new("git")
                .args(["update-ref", "-d", "refs/notes/blameprompt"])
                .output();

            println!("  \x1b[1;32m[done]\x1b[0m Removed {} Git Note(s)", count);

            let _ = std::process::Command::new("git")
                .args([
                    "config", "--unset", "remote.origin.fetch",
                    "+refs/notes/blameprompt:refs/notes/blameprompt",
                ])
                .output();
        }
    }
    Ok(())
}

fn remove_binary() -> Result<(), String> {
    if let Ok(_exe_path) = std::env::current_exe() {
        println!("  \x1b[1;36m[info]\x1b[0m To remove the binary:");
        println!("         \x1b[36mcargo uninstall blameprompt\x1b[0m");
    }
    Ok(())
}

fn count_git_notes() -> usize {
    let output = std::process::Command::new("git")
        .args(["notes", "--ref", "refs/notes/blameprompt", "list"])
        .output();

    match output {
        Ok(o) if o.status.success() => {
            String::from_utf8_lossy(&o.stdout).lines().count()
        }
        _ => 0,
    }
}
