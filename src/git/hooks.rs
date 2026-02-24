use std::path::Path;

/// PATH augmentation added to every hook so blameprompt and git are found
/// even when VS Code SCM (or other GUI git clients) spawn git with a minimal PATH.
const PATH_PREAMBLE: &str = r#"# Augment PATH for VS Code SCM and GUI git clients
export PATH="$HOME/.cargo/bin:$HOME/.local/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"
"#;

/// Resolve the absolute path to the current blameprompt binary.
/// Falls back to "blameprompt" (rely on PATH) if resolution fails.
pub fn resolve_binary_path() -> String {
    std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "blameprompt".to_string())
}

fn pre_commit_hook(binary: &str) -> String {
    format!(r#"# BlamePrompt pre-commit hook (do not edit between markers)
{preamble}BLAMEPROMPT="{binary}"
if [ -x "$BLAMEPROMPT" ]; then
    COUNT=$("$BLAMEPROMPT" staging-count 2>/dev/null || echo "0")
    if [ "$COUNT" != "0" ]; then
        echo "[BlamePrompt] $COUNT receipt(s) will be attached to this commit"
    fi
fi
# /BlamePrompt
"#, preamble = PATH_PREAMBLE, binary = binary)
}

fn post_commit_hook(binary: &str) -> String {
    format!(r#"# BlamePrompt post-commit hook (do not edit between markers)
{preamble}BLAMEPROMPT="{binary}"
if [ -x "$BLAMEPROMPT" ]; then
    "$BLAMEPROMPT" attach 2>/dev/null || true
fi
# /BlamePrompt
"#, preamble = PATH_PREAMBLE, binary = binary)
}

fn post_checkout_hook(binary: &str) -> String {
    format!(r#"# BlamePrompt post-checkout hook (do not edit between markers)
{preamble}BLAMEPROMPT="{binary}"
# Auto-initialize BlamePrompt in new repos after git clone / git checkout
if [ -x "$BLAMEPROMPT" ]; then
    BP_DIR=".blameprompt"
    if [ ! -d "$BP_DIR" ]; then
        mkdir -p "$BP_DIR"
        echo '{{"receipts":[]}}' > "$BP_DIR/staging.json"
    fi
    # Fetch remote BlamePrompt notes if they exist
    git fetch origin refs/notes/blameprompt:refs/notes/blameprompt 2>/dev/null || true
fi
# /BlamePrompt
"#, preamble = PATH_PREAMBLE, binary = binary)
}

fn post_merge_hook(binary: &str) -> String {
    format!(r#"# BlamePrompt post-merge hook (do not edit between markers)
{preamble}BLAMEPROMPT="{binary}"
if [ -x "$BLAMEPROMPT" ]; then
    COUNT=$("$BLAMEPROMPT" staging-count 2>/dev/null || echo "0")
    if [ "$COUNT" != "0" ]; then
        echo "[BlamePrompt] $COUNT staged receipt(s) preserved after merge"
    fi
fi
# /BlamePrompt
"#, preamble = PATH_PREAMBLE, binary = binary)
}

fn post_rewrite_hook(binary: &str) -> String {
    format!(r#"# BlamePrompt post-rewrite hook (do not edit between markers)
{preamble}BLAMEPROMPT="{binary}"
# Remap BlamePrompt notes after rebase or amend
if [ -x "$BLAMEPROMPT" ]; then
    while read OLD_SHA NEW_SHA; do
        NOTE=$(git notes --ref refs/notes/blameprompt show "$OLD_SHA" 2>/dev/null) || continue
        git notes --ref refs/notes/blameprompt add -f -m "$NOTE" "$NEW_SHA" 2>/dev/null && \
        git notes --ref refs/notes/blameprompt remove "$OLD_SHA" 2>/dev/null || true
    done
fi
# /BlamePrompt
"#, preamble = PATH_PREAMBLE, binary = binary)
}

fn all_hooks(binary: &str) -> Vec<(&'static str, String)> {
    vec![
        ("pre-commit",    pre_commit_hook(binary)),
        ("post-commit",   post_commit_hook(binary)),
        ("post-checkout", post_checkout_hook(binary)),
        ("post-merge",    post_merge_hook(binary)),
        ("post-rewrite",  post_rewrite_hook(binary)),
    ]
}

fn git_hooks_dir() -> Result<std::path::PathBuf, String> {
    let repo = git2::Repository::discover(".").map_err(|_| "Not in a git repository. Run this from inside a git repository.".to_string())?;
    Ok(repo.path().join("hooks"))
}

pub fn install_hooks() -> Result<(), String> {
    let hooks_dir = git_hooks_dir()?;
    std::fs::create_dir_all(&hooks_dir)
        .map_err(|e| format!("Cannot create hooks dir: {}", e))?;

    let binary = resolve_binary_path();
    for (name, content) in all_hooks(&binary) {
        install_hook(&hooks_dir, name, &content)?;
    }

    println!("Installed git hooks in {}", hooks_dir.display());
    Ok(())
}

fn install_hook(hooks_dir: &Path, name: &str, content: &str) -> Result<(), String> {
    let hook_path = hooks_dir.join(name);

    if hook_path.exists() {
        let existing = std::fs::read_to_string(&hook_path)
            .map_err(|e| format!("Cannot read {}: {}", name, e))?;

        if existing.contains("BlamePrompt") {
            return Ok(());
        }

        // Append to existing hook
        let mut new_content = existing;
        new_content.push_str("\n\n");
        new_content.push_str(content);
        std::fs::write(&hook_path, new_content)
            .map_err(|e| format!("Cannot write {}: {}", name, e))?;
    } else {
        // Create new hook
        let full = format!("#!/bin/sh\n\n{}", content);
        std::fs::write(&hook_path, full)
            .map_err(|e| format!("Cannot write {}: {}", name, e))?;
    }

    // Make executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&hook_path, std::fs::Permissions::from_mode(0o755));
    }

    Ok(())
}

pub fn uninstall_hooks() -> Result<(), String> {
    let hooks_dir = match git_hooks_dir() {
        Ok(d) => d,
        Err(_) => {
            println!("  \x1b[2m[skip]\x1b[0m Not in a git repository");
            return Ok(());
        }
    };

    let hook_names = ["pre-commit", "post-commit", "post-checkout", "post-merge", "post-rewrite"];
    for hook_name in &hook_names {
        let hook_path = hooks_dir.join(hook_name);
        if !hook_path.exists() {
            continue;
        }

        let content = std::fs::read_to_string(&hook_path)
            .map_err(|e| format!("Cannot read hook: {}", e))?;

        if !content.contains("BlamePrompt") {
            continue;
        }

        let cleaned = remove_between_markers(&content, "# BlamePrompt", "# /BlamePrompt");

        if cleaned.trim().is_empty() || cleaned.trim() == "#!/bin/sh" {
            std::fs::remove_file(&hook_path)
                .map_err(|e| format!("Cannot delete hook: {}", e))?;
            println!("  \x1b[1;32m[done]\x1b[0m Removed \x1b[2m.git/hooks/{}\x1b[0m", hook_name);
        } else {
            std::fs::write(&hook_path, &cleaned)
                .map_err(|e| format!("Cannot write hook: {}", e))?;
            println!("  \x1b[1;32m[done]\x1b[0m Removed BlamePrompt section from \x1b[2m.git/hooks/{}\x1b[0m", hook_name);
        }
    }
    Ok(())
}

fn remove_between_markers(content: &str, start_marker: &str, end_marker: &str) -> String {
    let mut result = String::new();
    let mut skipping = false;
    for line in content.lines() {
        if line.contains(start_marker) && !line.contains(end_marker) {
            skipping = true;
            continue;
        }
        if line.contains(end_marker) {
            skipping = false;
            continue;
        }
        if !skipping {
            result.push_str(line);
            result.push('\n');
        }
    }
    result
}

pub fn all_hook_entries(binary: &str) -> Vec<(&'static str, String)> {
    all_hooks(binary)
}
