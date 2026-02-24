use serde_json::json;
use std::path::PathBuf;

fn settings_path() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or("Cannot find home directory")?;
    Ok(home.join(".claude").join("settings.json"))
}

fn blameprompt_binary_path() -> String {
    std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "blameprompt".to_string())
}

pub fn install() -> Result<(), String> {
    let path = settings_path()?;

    // Create ~/.claude/ if it doesn't exist
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("Cannot create ~/.claude/: {}", e))?;
    }

    // Read existing settings or create empty object
    let mut settings: serde_json::Value = if path.exists() {
        let content =
            std::fs::read_to_string(&path).map_err(|e| format!("Cannot read settings: {}", e))?;
        serde_json::from_str(&content).unwrap_or_else(|_| json!({}))
    } else {
        json!({})
    };

    // Check if BlamePrompt hooks already installed
    let settings_str = serde_json::to_string(&settings).unwrap_or_default();
    if settings_str.contains("blameprompt") {
        println!("BlamePrompt hooks already installed in ~/.claude/settings.json");
        return Ok(());
    }

    let binary = blameprompt_binary_path();
    let command = format!("{} checkpoint claude --hook-input stdin", binary);

    let hook_cmd = json!([{
        "type": "command",
        "command": command
    }]);

    // All hook events and their matchers for comprehensive auditing
    let hook_configs: Vec<(&str, Option<&str>)> = vec![
        ("PreToolUse", Some("Write|Edit|MultiEdit|Bash")),
        (
            "PostToolUse",
            Some("Write|Edit|MultiEdit|Bash|Read|Glob|Grep|WebFetch|WebSearch|Task"),
        ),
        ("PostToolUseFailure", Some("Write|Edit|MultiEdit|Bash")),
        ("UserPromptSubmit", None),
        ("SessionStart", None),
        ("SessionEnd", None),
        ("Stop", None),
        ("SubagentStart", None),
        ("SubagentStop", None),
        ("Notification", None),
    ];

    // Ensure hooks object exists
    if settings.get("hooks").is_none() {
        settings["hooks"] = json!({});
    }

    let hooks = settings.get_mut("hooks").unwrap();

    for (event, matcher) in &hook_configs {
        let entry = if let Some(m) = matcher {
            json!({
                "matcher": m,
                "hooks": hook_cmd
            })
        } else {
            json!({
                "hooks": hook_cmd
            })
        };

        if hooks.get(*event).is_none() {
            hooks[*event] = json!([]);
        }
        if let Some(arr) = hooks.get_mut(*event).and_then(|v| v.as_array_mut()) {
            arr.push(entry);
        }
    }

    // Write back
    let json_str = serde_json::to_string_pretty(&settings)
        .map_err(|e| format!("Failed to serialize: {}", e))?;
    std::fs::write(&path, json_str).map_err(|e| format!("Failed to write settings: {}", e))?;

    println!("Installed Claude Code hooks in {}", path.display());
    Ok(())
}

pub fn uninstall() -> Result<(), String> {
    let path = settings_path()?;

    if !path.exists() {
        println!("  \x1b[2m[skip]\x1b[0m No ~/.claude/settings.json found");
        return Ok(());
    }

    let content =
        std::fs::read_to_string(&path).map_err(|e| format!("Cannot read settings: {}", e))?;
    let mut settings: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| format!("Invalid JSON: {}", e))?;

    if let Some(hooks) = settings.get_mut("hooks") {
        let all_events = [
            "PreToolUse",
            "PostToolUse",
            "PostToolUseFailure",
            "UserPromptSubmit",
            "SessionStart",
            "SessionEnd",
            "Stop",
            "SubagentStart",
            "SubagentStop",
            "Notification",
        ];
        for event in &all_events {
            if let Some(arr) = hooks.get_mut(*event).and_then(|v| v.as_array_mut()) {
                arr.retain(|entry| {
                    let json_str = serde_json::to_string(entry).unwrap_or_default();
                    !json_str.contains("blameprompt")
                });
            }
        }

        // Clean up empty arrays
        if let Some(hooks_obj) = hooks.as_object_mut() {
            hooks_obj.retain(|_, v| v.as_array().is_none_or(|a| !a.is_empty()));
        }
        if hooks.as_object().is_some_and(|o| o.is_empty()) {
            settings.as_object_mut().unwrap().remove("hooks");
        }
    }

    let json_str = serde_json::to_string_pretty(&settings)
        .map_err(|e| format!("Failed to serialize: {}", e))?;
    std::fs::write(&path, json_str).map_err(|e| format!("Failed to write: {}", e))?;

    println!("  \x1b[1;32m[done]\x1b[0m Removed Claude Code hooks \x1b[2m(~/.claude/settings.json)\x1b[0m");
    Ok(())
}
