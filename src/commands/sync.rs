use std::process::Command;

pub fn push() {
    // Check if remote exists
    let remote_check = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .output();

    match remote_check {
        Ok(o) if o.status.success() => {}
        _ => {
            eprintln!("Error: No remote 'origin' configured.");
            eprintln!("  Add a remote first: git remote add origin <url>");
            return;
        }
    }

    println!("Pushing BlamePrompt notes to origin...");
    let output = Command::new("git")
        .args(["push", "origin", "refs/notes/blameprompt"])
        .output();

    match output {
        Ok(o) if o.status.success() => {
            println!("[BlamePrompt] Notes pushed to origin successfully.");
        }
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            if stderr.contains("does not match any") {
                println!("[BlamePrompt] No notes to push (refs/notes/blameprompt does not exist yet).");
                println!("  Create some commits with AI receipts first.");
            } else {
                eprintln!("Error pushing notes: {}", stderr);
            }
        }
        Err(e) => {
            eprintln!("Error: git push failed: {}", e);
        }
    }
}

pub fn pull() {
    // Check if remote exists
    let remote_check = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .output();

    match remote_check {
        Ok(o) if o.status.success() => {}
        _ => {
            eprintln!("Error: No remote 'origin' configured.");
            eprintln!("  Add a remote first: git remote add origin <url>");
            return;
        }
    }

    println!("Fetching BlamePrompt notes from origin...");
    let output = Command::new("git")
        .args(["fetch", "origin", "refs/notes/blameprompt:refs/notes/blameprompt"])
        .output();

    match output {
        Ok(o) if o.status.success() => {
            println!("[BlamePrompt] Notes fetched from origin successfully.");

            // Configure auto-fetch for future pulls
            let _ = Command::new("git")
                .args(["config", "--add", "remote.origin.fetch",
                       "+refs/notes/blameprompt:refs/notes/blameprompt"])
                .output();
        }
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            if stderr.contains("couldn't find remote ref") {
                println!("[BlamePrompt] No notes found on origin.");
                println!("  Someone needs to push notes first: blameprompt push");
            } else {
                eprintln!("Error fetching notes: {}", stderr);
            }
        }
        Err(e) => {
            eprintln!("Error: git fetch failed: {}", e);
        }
    }
}
