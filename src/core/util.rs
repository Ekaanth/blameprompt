/// Shared utility functions used across multiple commands.
///
/// Centralising these prevents duplicates in checkpoint.rs, record.rs, cursor.rs, etc.
use std::process::Command;

/// Convert an absolute path to one relative to `base`.
/// Returns the path unchanged if it doesn't start with `base` or is already relative.
pub fn make_relative(path: &str, base: &str) -> String {
    let path = path.trim();
    let base = base.trim_end_matches('/');
    if base.is_empty() || base == "." {
        return path.to_string();
    }
    if let Some(rel) = path.strip_prefix(base) {
        let rel = rel.strip_prefix('/').unwrap_or(rel);
        if rel.is_empty() {
            return path.to_string();
        }
        return rel.to_string();
    }
    path.to_string()
}

/// Shorten a full git SHA to 8 characters for display.
pub fn short_sha(sha: &str) -> String {
    sha.chars().take(8).collect()
}

/// Check whether two file paths refer to the same file.
/// Handles relative/absolute mismatches by checking suffix containment.
pub fn paths_match(a: &str, b: &str) -> bool {
    a == b || a.ends_with(b) || b.ends_with(a)
}

/// Return `git config user.name <user.email>` for the current repo.
pub fn git_user() -> String {
    let name = Command::new("git")
        .args(["config", "user.name"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let email = Command::new("git")
        .args(["config", "user.email"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown@unknown".to_string());

    format!("{} <{}>", name, email)
}

/// Parse a single unified-diff hunk header `@@ -old +new_start[,new_count] @@`
/// and return `(new_start, new_end)`.
pub fn parse_hunk_range(line: &str) -> (u32, u32) {
    if let Some(plus_part) = line.split('+').nth(1) {
        let nums = plus_part.split_whitespace().next().unwrap_or("0,0");
        let parts: Vec<&str> = nums.split(',').collect();
        let start: u32 = parts.first().and_then(|s| s.parse().ok()).unwrap_or(0);
        let count: u32 = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(1);
        let end = if count == 0 { start } else { start + count - 1 };
        return (start, end);
    }
    (0, 0)
}

/// Scan a full unified diff string and return the overall (min_start, max_end) line range
/// across all `@@` hunks. Used to determine the line range a tool call affected.
pub fn diff_line_range(diff_output: &str) -> (u32, u32) {
    let mut start = 0u32;
    let mut end = 0u32;
    for line in diff_output.lines() {
        if line.starts_with("@@") {
            let (hunk_start, hunk_end) = parse_hunk_range(line);
            if hunk_start > 0 {
                if start == 0 || hunk_start < start {
                    start = hunk_start;
                }
                if hunk_end > end {
                    end = hunk_end;
                }
            }
        }
    }
    if start == 0 {
        (0, 0)
    } else {
        (start, end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_make_relative_absolute() {
        assert_eq!(
            make_relative("/home/user/project/src/main.rs", "/home/user/project"),
            "src/main.rs"
        );
    }

    #[test]
    fn test_make_relative_already_relative() {
        assert_eq!(
            make_relative("src/main.rs", "/home/user/project"),
            "src/main.rs"
        );
    }

    #[test]
    fn test_make_relative_different_root() {
        assert_eq!(
            make_relative("/other/path/file.rs", "/home/user/project"),
            "/other/path/file.rs"
        );
    }

    #[test]
    fn test_short_sha() {
        assert_eq!(short_sha("abc1234567890abcdef"), "abc12345");
        assert_eq!(short_sha("abc"), "abc"); // shorter than 8 — no panic
    }

    #[test]
    fn test_paths_match() {
        assert!(paths_match("src/main.rs", "src/main.rs"));
        assert!(paths_match("/home/user/project/src/main.rs", "src/main.rs"));
        assert!(paths_match("src/main.rs", "/abs/src/main.rs"));
        assert!(!paths_match("src/lib.rs", "src/main.rs"));
    }

    #[test]
    fn test_parse_hunk_range() {
        assert_eq!(parse_hunk_range("@@ -1,3 +1,5 @@"), (1, 5));
        assert_eq!(parse_hunk_range("@@ -10,2 +12,4 @@"), (12, 15));
        assert_eq!(parse_hunk_range("@@ -1 +1,0 @@"), (1, 1));
    }

    #[test]
    fn test_diff_line_range() {
        let diff = "@@ -1,3 +1,5 @@\n some code\n@@ -10,2 +12,4 @@\n more code\n";
        let (start, end) = diff_line_range(diff);
        assert_eq!(start, 1);
        assert_eq!(end, 15);
    }
}
