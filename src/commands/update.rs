use std::cmp::Ordering;
use std::env;
use std::fs;
use std::process::Command;

const REPO: &str = "ekaanth/blameprompt";
const BINARY_NAME: &str = "blameprompt";

// ANSI colors (matching install.sh style)
const GREEN: &str = "\x1b[1;32m";
const CYAN: &str = "\x1b[1;36m";
const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";

fn info(msg: &str) {
    eprintln!("  {GREEN}[info]{RESET} {msg}");
}

/// Detect the release target triple for the current platform.
fn detect_target() -> Result<String, String> {
    let os = env::consts::OS;
    let arch = env::consts::ARCH;

    let target = match (os, arch) {
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        _ => return Err(format!("Unsupported platform: {os}/{arch}")),
    };
    Ok(target.to_string())
}

/// Fetch latest release version from GitHub API.
fn fetch_latest_version() -> Result<String, String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let output = Command::new("curl")
        .args([
            "-fsSL",
            "-H",
            "Accept: application/vnd.github.v3+json",
            &url,
        ])
        .output()
        .map_err(|e| format!("Failed to run curl: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "Failed to fetch latest release. Check https://github.com/{REPO}/releases"
        ));
    }

    let body = String::from_utf8_lossy(&output.stdout);
    extract_tag_name(&body)
}

/// Verify a specific version tag exists on GitHub.
fn fetch_specific_version(version: &str) -> Result<String, String> {
    let tag = normalize_tag(version);
    let url = format!("https://api.github.com/repos/{REPO}/releases/tags/{tag}");
    let output = Command::new("curl")
        .args([
            "-fsSL",
            "-H",
            "Accept: application/vnd.github.v3+json",
            &url,
        ])
        .output()
        .map_err(|e| format!("Failed to run curl: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "Release {tag} not found. Check https://github.com/{REPO}/releases"
        ));
    }

    let body = String::from_utf8_lossy(&output.stdout);
    extract_tag_name(&body)
}

/// Extract "tag_name" value from GitHub API JSON response.
fn extract_tag_name(json: &str) -> Result<String, String> {
    let v: serde_json::Value =
        serde_json::from_str(json).map_err(|_| "Invalid JSON from GitHub API")?;
    v.get("tag_name")
        .and_then(|v| v.as_str())
        .map(String::from)
        .ok_or_else(|| "No tag_name in response".to_string())
}

/// Normalize a version string to a tag (e.g. "0.2.0" -> "v0.2.0", "v0.2.0" -> "v0.2.0").
fn normalize_tag(version: &str) -> String {
    let v = version.trim();
    if v.starts_with('v') {
        v.to_string()
    } else {
        format!("v{v}")
    }
}

/// Strip the leading 'v' from a tag to get a bare version string.
fn strip_v(tag: &str) -> &str {
    tag.strip_prefix('v').unwrap_or(tag)
}

/// Compare two semver version strings (e.g. "0.1.0" vs "0.2.0").
fn compare_versions(a: &str, b: &str) -> Ordering {
    let parse = |s: &str| -> Vec<u64> {
        s.split('.')
            .map(|part| part.parse::<u64>().unwrap_or(0))
            .collect()
    };
    let va = parse(a);
    let vb = parse(b);

    for i in 0..va.len().max(vb.len()) {
        let pa = va.get(i).copied().unwrap_or(0);
        let pb = vb.get(i).copied().unwrap_or(0);
        match pa.cmp(&pb) {
            Ordering::Equal => continue,
            other => return other,
        }
    }
    Ordering::Equal
}

/// Download, extract, and replace the current binary.
fn download_and_install(tag: &str, target: &str) -> Result<(), String> {
    let tarball = format!("{BINARY_NAME}-{tag}-{target}.tar.gz");
    let url = format!("https://github.com/{REPO}/releases/download/{tag}/{tarball}");

    // Create temp directory
    let tmpdir = env::temp_dir().join(format!("blameprompt-update-{}", std::process::id()));
    fs::create_dir_all(&tmpdir).map_err(|e| format!("Failed to create temp dir: {e}"))?;

    let tarball_path = tmpdir.join(&tarball);

    // Download
    info(&format!("Downloading {tarball}..."));
    let status = Command::new("curl")
        .args(["-fsSL", &url, "-o", tarball_path.to_str().unwrap_or("")])
        .status()
        .map_err(|e| format!("Failed to run curl: {e}"))?;

    if !status.success() {
        let _ = fs::remove_dir_all(&tmpdir);
        return Err(format!(
            "Download failed. Check https://github.com/{REPO}/releases for available builds."
        ));
    }

    // Extract
    info("Extracting...");
    let status = Command::new("tar")
        .args(["xzf", tarball_path.to_str().unwrap_or("")])
        .current_dir(&tmpdir)
        .status()
        .map_err(|e| format!("Failed to extract: {e}"))?;

    if !status.success() {
        let _ = fs::remove_dir_all(&tmpdir);
        return Err("Failed to extract tarball".to_string());
    }

    // Find current binary location
    let current_exe =
        env::current_exe().map_err(|e| format!("Cannot determine current binary path: {e}"))?;

    let ext = if cfg!(windows) { ".exe" } else { "" };
    let new_binary = tmpdir.join(format!("{BINARY_NAME}{ext}"));

    if !new_binary.exists() {
        let _ = fs::remove_dir_all(&tmpdir);
        return Err("Downloaded archive does not contain the expected binary".to_string());
    }

    // Replace: move old to .bak, move new in, delete .bak
    let backup = current_exe.with_extension("bak");

    info("Installing...");
    fs::rename(&current_exe, &backup)
        .map_err(|e| format!("Failed to back up current binary (try running with sudo): {e}"))?;

    if let Err(e) = fs::copy(&new_binary, &current_exe) {
        // Restore backup on failure
        let _ = fs::rename(&backup, &current_exe);
        let _ = fs::remove_dir_all(&tmpdir);
        return Err(format!("Failed to install new binary: {e}"));
    }

    // Set executable permissions on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&current_exe, fs::Permissions::from_mode(0o755));
    }

    // Cleanup
    let _ = fs::remove_file(&backup);
    let _ = fs::remove_dir_all(&tmpdir);

    Ok(())
}

pub fn run(check: bool, force: bool, version: Option<String>) -> Result<(), String> {
    let current = env!("CARGO_PKG_VERSION");

    eprintln!();
    eprintln!("  {CYAN}BlamePrompt Update{RESET}");
    eprintln!("  {DIM}Current version: v{current}{RESET}");
    eprintln!();

    // Resolve target version
    let (tag, target_version) = if let Some(ref v) = version {
        let tag = fetch_specific_version(v)?;
        let ver = strip_v(&tag).to_string();
        (tag, ver)
    } else {
        info("Checking for updates...");
        let tag = fetch_latest_version()?;
        let ver = strip_v(&tag).to_string();
        (tag, ver)
    };

    info(&format!("Target version:  v{target_version}"));

    // Compare
    let cmp = compare_versions(current, &target_version);

    if check {
        match cmp {
            Ordering::Less => {
                eprintln!();
                eprintln!("  {GREEN}Update available:{RESET} v{current} → v{target_version}");
                eprintln!("  Run {CYAN}blameprompt update{RESET} to install.");
            }
            Ordering::Equal => {
                eprintln!();
                eprintln!("  {GREEN}Already up to date.{RESET}");
            }
            Ordering::Greater => {
                eprintln!();
                eprintln!(
                    "  {DIM}Current version (v{current}) is newer than v{target_version}.{RESET}"
                );
            }
        }
        eprintln!();
        return Ok(());
    }

    if cmp == Ordering::Equal && !force && version.is_none() {
        eprintln!();
        eprintln!("  {GREEN}Already up to date.{RESET}");
        eprintln!();
        return Ok(());
    }

    if cmp == Ordering::Greater && !force && version.is_none() {
        eprintln!();
        eprintln!(
            "  {DIM}Current version (v{current}) is newer than latest release (v{target_version}).{RESET}"
        );
        eprintln!(
            "  Use {CYAN}--force{RESET} or {CYAN}--version {target_version}{RESET} to downgrade."
        );
        eprintln!();
        return Ok(());
    }

    let target = detect_target()?;
    download_and_install(&tag, &target)?;

    eprintln!();
    if version.is_some() {
        eprintln!("  {GREEN}Installed v{target_version}.{RESET}");
    } else {
        eprintln!("  {GREEN}Updated to v{target_version}!{RESET}");
    }
    eprintln!();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compare_versions_equal() {
        assert_eq!(compare_versions("0.1.0", "0.1.0"), Ordering::Equal);
    }

    #[test]
    fn test_compare_versions_less() {
        assert_eq!(compare_versions("0.1.0", "0.2.0"), Ordering::Less);
        assert_eq!(compare_versions("0.1.9", "0.2.0"), Ordering::Less);
        assert_eq!(compare_versions("1.0.0", "2.0.0"), Ordering::Less);
    }

    #[test]
    fn test_compare_versions_greater() {
        assert_eq!(compare_versions("0.2.0", "0.1.0"), Ordering::Greater);
        assert_eq!(compare_versions("1.0.0", "0.9.9"), Ordering::Greater);
    }

    #[test]
    fn test_normalize_tag() {
        assert_eq!(normalize_tag("0.2.0"), "v0.2.0");
        assert_eq!(normalize_tag("v0.2.0"), "v0.2.0");
        assert_eq!(normalize_tag(" v0.1.0 "), "v0.1.0");
    }

    #[test]
    fn test_strip_v() {
        assert_eq!(strip_v("v0.2.0"), "0.2.0");
        assert_eq!(strip_v("0.2.0"), "0.2.0");
    }

    #[test]
    fn test_extract_tag_name() {
        let json = r#"{
  "tag_name": "v0.2.0",
  "name": "v0.2.0"
}"#;
        assert_eq!(extract_tag_name(json).unwrap(), "v0.2.0");
    }

    #[test]
    fn test_detect_target() {
        // Should succeed on any supported CI/dev platform
        assert!(detect_target().is_ok());
    }
}
