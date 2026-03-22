/// Prompt quality evaluation engine.
///
/// Scores developer prompts across four weighted dimensions:
///   - Clarity  (30%): file refs, line refs, function refs, code blocks
///   - Actionability (25%): strong verbs, clear intent, CLI commands
///   - Context  (25%): error messages, expected behavior, stack traces, constraints
///   - Efficiency (20%): appropriate length, no vague language, no verbosity
///
///
/// Runs entirely locally — no network calls, no AI inference.
use serde::{Deserialize, Serialize};

// ── Struct ──────────────────────────────────────────────────────────────────

/// Quality assessment attached to each receipt.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PromptQuality {
    /// Overall score 0–100.
    pub score: u32,
    /// Human-readable rating derived from score.
    pub rating: String,
    /// Detected issues (e.g. "vague_language", "no_file_reference").
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub issues: Vec<String>,
    /// Prompt classification: code_generation, bug_fix, question, file_operation,
    /// command, clarification, refactor, or other.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
}

// ── Constants ───────────────────────────────────────────────────────────────

const STRONG_VERBS: &[&str] = &[
    "fix",
    "add",
    "create",
    "implement",
    "refactor",
    "update",
    "remove",
    "delete",
    "rename",
    "move",
    "extract",
    "replace",
    "convert",
    "migrate",
    "optimize",
    "debug",
    "test",
    "write",
    "build",
    "configure",
    "install",
    "integrate",
    "upgrade",
    "downgrade",
    "rewrite",
    "simplify",
    "split",
    "merge",
    "resolve",
    "handle",
    "deprecate",
    "scaffold",
    "stub",
    "mock",
    "validate",
    "serialize",
    "deserialize",
    "parse",
    "format",
    "lint",
    "benchmark",
    "profile",
    "deploy",
    "provision",
    "containerize",
    "dockerize",
];

const WEAK_VERBS: &[&str] = &[
    "make", "do", "change", "help", "try", "look", "check", "see", "get", "put",
];

const VAGUE_PHRASES: &[&str] = &[
    "fix it",
    "do it",
    "make it work",
    "it's broken",
    "it doesn't work",
    "something is wrong",
    "just do",
    "you know what i mean",
    "figure it out",
    "make it better",
    "clean it up",
    "do the thing",
    "do the same",
    "do what you think",
    "whatever you think",
    "just make",
    "try again",
    "do something",
    "i don't know",
    "help me",
    "please help",
    "not working",
    "doesn't work",
    "it broke",
    "same thing",
    "the usual",
];

/// File extensions that require a preceding word boundary (dot preceded by
/// an alphanumeric char) to avoid false positives like ".c" matching ".css".
/// We list extensions longest-first within each starting letter to avoid
/// prefix collisions.
const FILE_EXTENSIONS: &[&str] = &[
    ".tsx", ".ts", ".rs", ".py", ".rb", ".go", ".java", ".jsx", ".js", ".swift", ".kt", ".scala",
    ".cpp", ".hpp", ".css", ".csv", ".cs", ".html", ".yml", ".yaml", ".json", ".toml", ".sql",
    ".scss", ".svelte", ".vue", ".md", ".sh", ".bash", ".zsh", ".c", ".h", ".proto", ".graphql",
    ".gql", ".tf", ".hcl", ".ex", ".exs", ".clj", ".zig", ".nim", ".lua", ".r", ".jl", ".dart",
    ".php", ".xml", ".wasm", ".wat",
];

const CLI_COMMANDS: &[&str] = &[
    "git ",
    "cargo ",
    "npm ",
    "yarn ",
    "pnpm ",
    "bun ",
    "docker ",
    "kubectl ",
    "make ",
    "cmake ",
    "go ",
    "python ",
    "pip ",
    "ruby ",
    "gem ",
    "rustup ",
    "rustc ",
    "gcc ",
    "g++ ",
    "javac ",
    "gradle ",
    "mvn ",
    "dotnet ",
    "terraform ",
    "ansible ",
    "helm ",
    "curl ",
    "wget ",
    "ssh ",
    "scp ",
    "rsync ",
    "ls ",
    "cd ",
    "cat ",
    "grep ",
    "find ",
    "sed ",
    "awk ",
    "tar ",
    "zip ",
    "unzip ",
];

const ERROR_PATTERNS: &[&str] = &[
    "error",
    "exception",
    "panic",
    "traceback",
    "stack trace",
    // JS/TS
    "typeerror",
    "referenceerror",
    "syntaxerror",
    "rangeerror",
    // Python
    "nameerror",
    "valueerror",
    "indexerror",
    "keyerror",
    "attributeerror",
    "importerror",
    "ioerror",
    "runtimeerror",
    "zerodivisionerror",
    // Java/JVM
    "nullpointerexception",
    "classnotfound",
    "classcastexception",
    "outofmemoryerror",
    "stackoverflowerror",
    "illegalargument",
    "illegalstate",
    // Rust
    "segfault",
    "unwrap()",
    "expect()",
    // General
    "status 4",
    "status 5",
    "http 4",
    "http 5",
    "404",
    "500",
    "502",
    "503",
    "failed to",
    "cannot ",
    "could not",
    "unable to",
    "doesn't work",
    "crash",
    "failing",
    "assertion failed",
    "compilation error",
    "compile error",
    "build error",
    "lint error",
    "warning:",
    "stderr",
    "e0",
    "e1", // rust error codes like E0382
    "errno",
    "exitcode",
    "exit code",
    "return code",
    "aborted",
    "killed",
    "timeout",
    "timed out",
    "deprecated",
    "missing module",
    "module not found",
    "connection refused",
    "permission denied",
    "access denied",
];

const EXPECTED_BEHAVIOR_PATTERNS: &[&str] = &[
    "should ",
    "expect",
    "instead of",
    "rather than",
    "supposed to",
    "want it to",
    "need it to",
    "currently ",
    "right now ",
    "but instead",
    "the result should",
    "the output should",
    "i want",
    "desired behavior",
    "the goal is",
    "ideally ",
    "the expected",
    "it used to",
    "before it was",
    "after this change",
    "acceptance criteria",
    "given ",
    "when ",
    "then ",
];

const CONSTRAINT_PATTERNS: &[&str] = &[
    "must ",
    "must not",
    "without breaking",
    "backwards compatible",
    "backward compatible",
    "don't change",
    "do not change",
    "keep the",
    "preserve ",
    "maintain ",
    "no breaking change",
    "within ",
    "at most ",
    "at least ",
    "maximum ",
    "minimum ",
    "performance",
    "latency",
    "memory",
    "thread safe",
    "idempotent",
    "atomic",
    "transactional",
];

// ── Public API ──────────────────────────────────────────────────────────────

/// Evaluate a prompt and return a quality assessment.
pub fn evaluate(prompt: &str) -> PromptQuality {
    let trimmed = prompt.trim();
    if trimmed.is_empty() {
        return PromptQuality {
            score: 0,
            rating: "No prompt".to_string(),
            issues: vec!["empty_prompt".to_string()],
            category: None,
        };
    }

    let lower = trimmed.to_lowercase();

    // Skip grading for auto-generated continuation preambles (check FIRST).
    if lower.starts_with("this session is being continued") {
        return PromptQuality {
            score: 50,
            rating: "Continuation".to_string(),
            issues: vec![],
            category: Some("continuation".to_string()),
        };
    }

    let word_count = trimmed.split_whitespace().count();
    let category = classify_prompt(&lower);
    let is_cli = is_cli_command(&lower);

    // ── Standalone minimal prompts ─────────────────────────────────────
    // Single-word non-commands like "yes", "no", "ok" are inherently low quality.
    if word_count <= 2 && is_minimal_response(&lower) {
        return PromptQuality {
            score: 15,
            rating: "Poor".to_string(),
            issues: vec!["minimal_response".to_string()],
            category: Some(category),
        };
    }

    let mut issues: Vec<String> = Vec::new();

    // ── Clarity (30%) ──────────────────────────────────────────────────
    let mut clarity: i32 = 40;
    let has_file = has_file_reference(trimmed);
    let has_line = has_line_reference(trimmed);
    let has_func = has_function_reference(&lower);
    let has_code = has_code_block(trimmed);

    if has_file {
        clarity += 20;
    }
    if has_line {
        clarity += 15;
    }
    if has_func {
        clarity += 15;
    }
    if has_code {
        clarity += 10;
    }
    if !has_file && !has_code && !has_func {
        clarity -= 10;
        issues.push("no_file_reference".to_string());
    }

    // ── Actionability (25%) ────────────────────────────────────────────
    let mut actionability: i32 = 40;
    if is_cli {
        // CLI commands are inherently clear and actionable
        actionability += 30;
        // CLI commands don't need file references — they ARE the context
        clarity += 15;
    } else if has_leading_strong_verb(&lower) {
        actionability += 25;
    } else if has_strong_verb_anywhere(&lower) {
        actionability += 15;
    } else if has_weak_verb_only(&lower) {
        actionability -= 15;
        issues.push("weak_action_verb".to_string());
    } else if category == "question" {
        // Questions without action verbs are okay but not great
        actionability += 5;
    }

    // ── Context (25%) ──────────────────────────────────────────────────
    let mut context: i32 = 40;
    if has_error_context(&lower) {
        context += 20;
    }
    if has_expected_behavior(&lower) {
        context += 15;
    }
    if has_stack_trace(trimmed) {
        context += 15;
    }
    if has_constraints(&lower) {
        context += 10;
    }

    // ── Efficiency (20%) ───────────────────────────────────────────────
    let mut efficiency: i32 = 60;

    // Vague language
    let vague_count = count_vague_phrases(&lower);
    if vague_count > 0 {
        let penalty = (vague_count as i32 * 10).min(40);
        efficiency -= penalty;
        issues.push("vague_language".to_string());
    }

    // Length
    if word_count < 3 && !is_cli {
        efficiency -= 15;
        issues.push("too_short".to_string());
    } else if (8..=200).contains(&word_count) {
        efficiency += 10; // well-formed length
    } else if word_count > 500 {
        efficiency -= 10;
        issues.push("verbose".to_string());
    }

    // ── Weighted composite ─────────────────────────────────────────────
    let clarity = clarity.clamp(0, 100);
    let actionability = actionability.clamp(0, 100);
    let context = context.clamp(0, 100);
    let efficiency = efficiency.clamp(0, 100);

    let composite = (clarity as f64 * 0.30)
        + (actionability as f64 * 0.25)
        + (context as f64 * 0.25)
        + (efficiency as f64 * 0.20);

    let score = (composite.round() as i32).clamp(0, 100) as u32;

    let rating = match score {
        90..=100 => "Excellent",
        70..=89 => "Good",
        50..=69 => "Fair",
        _ => "Poor",
    };

    PromptQuality {
        score,
        rating: rating.to_string(),
        issues,
        category: Some(category),
    }
}

/// Convert a score to a short badge string for display.
pub fn score_badge(quality: &PromptQuality) -> String {
    let icon = match quality.score {
        90..=100 => "A+",
        70..=89 => "A",
        50..=69 => "B",
        25..=49 => "C",
        _ => "D",
    };
    format!("[{} {}]", icon, quality.score)
}

/// Classify a prompt into a category.
pub fn classify_prompt(lower: &str) -> String {
    let first_word = lower.split_whitespace().next().unwrap_or("");

    // Clarification / correction (highest priority)
    if lower.starts_with("no,")
        || lower.starts_with("not that")
        || lower.starts_with("i meant")
        || lower.starts_with("actually,")
        || lower.starts_with("actually ")
        || lower.starts_with("instead,")
        || lower.starts_with("wait,")
        || lower.starts_with("sorry,")
    {
        return "clarification".to_string();
    }

    // CLI command
    if is_cli_command(lower) {
        return "command".to_string();
    }

    // Bug fix
    if (lower.contains("fix ") || lower.contains("debug ") || lower.contains("resolve "))
        && (has_error_context(lower) || lower.contains("bug") || lower.contains("issue"))
    {
        return "bug_fix".to_string();
    }

    // Refactor
    for kw in &[
        "refactor",
        "restructure",
        "reorganize",
        "clean up",
        "simplify",
        "extract ",
        "inline ",
        "rename ",
    ] {
        if lower.contains(kw) {
            return "refactor".to_string();
        }
    }

    // Code generation
    for kw in &[
        "create ",
        "implement ",
        "add ",
        "build ",
        "write ",
        "scaffold ",
        "generate ",
        "set up ",
        "setup ",
    ] {
        if lower.contains(kw) {
            return "code_generation".to_string();
        }
    }

    // File operation
    for kw in &[
        "read ", "search ", "find ", "list ", "delete ", "move ", "copy ",
    ] {
        if first_word == kw.trim() {
            return "file_operation".to_string();
        }
    }

    // Question
    if first_word == "what"
        || first_word == "why"
        || first_word == "how"
        || first_word == "where"
        || first_word == "when"
        || first_word == "which"
        || first_word == "is"
        || first_word == "are"
        || first_word == "does"
        || first_word == "can"
        || first_word == "could"
        || first_word == "would"
        || lower.ends_with('?')
        || lower.contains("explain ")
        || lower.contains("describe ")
    {
        return "question".to_string();
    }

    "other".to_string()
}

// ── Clarity helpers ─────────────────────────────────────────────────────────

/// Check for file path patterns with proper boundary matching.
/// Avoids false positives like ".c" matching ".css" by checking extensions
/// longest-first and requiring a word-like character before the dot.
fn has_file_reference(text: &str) -> bool {
    // Explicit directory paths
    if text.contains("src/")
        || text.contains("lib/")
        || text.contains("test/")
        || text.contains("tests/")
        || text.contains("pkg/")
        || text.contains("cmd/")
        || text.contains("internal/")
        || text.contains("./")
    {
        return true;
    }

    // Check for file.ext patterns with boundary awareness.
    // Split on whitespace, backticks, quotes to get tokens.
    // Use lowercase comparison so "Main.RS" matches ".rs".
    for token in split_tokens(text) {
        let token_lower = token.to_lowercase();
        for ext in FILE_EXTENSIONS {
            if let Some(pos) = token_lower.rfind(ext) {
                // Must be at end of token or followed by `:` (file:line) or `)`
                let after = pos + ext.len();
                let at_end = after >= token_lower.len()
                    || token_lower.as_bytes().get(after).is_none_or(|&b| {
                        b == b':' || b == b')' || b == b',' || b == b'"' || b == b'\''
                    });
                // Must have at least one char before the dot (the filename)
                let has_name = pos > 0 && token_lower.as_bytes()[pos - 1].is_ascii_alphanumeric();
                // Ensure the extension isn't a prefix of a longer extension already in token
                // e.g., ".c" in "foo.css" — check there's no more alpha after ext
                let no_longer_ext = after >= token_lower.len()
                    || !token_lower.as_bytes()[after].is_ascii_alphabetic();
                if at_end && has_name && no_longer_ext {
                    return true;
                }
            }
        }
    }
    false
}

/// Check for line number references with reduced false positives.
fn has_line_reference(text: &str) -> bool {
    let lower = text.to_lowercase();

    // "line 42", "line 123"
    for (i, _) in lower.match_indices("line ") {
        let after = &lower[i + 5..];
        if after.starts_with(|c: char| c.is_ascii_digit()) {
            return true;
        }
    }

    // L42 as standalone token
    for token in split_tokens(text) {
        if token.len() > 1
            && token.starts_with('L')
            && token[1..].chars().all(|c| c.is_ascii_digit())
        {
            return true;
        }
    }

    // file.ext:42 pattern — require the part before `:` to look like a filename
    for token in split_tokens(text) {
        if let Some(colon_pos) = token.rfind(':') {
            let before = &token[..colon_pos];
            let after = &token[colon_pos + 1..];
            // After colon must be digits (the line number)
            if !after.is_empty()
                && after.chars().all(|c| c.is_ascii_digit())
                // Before colon must contain a dot (file extension) or slash (path)
                && (before.contains('.') || before.contains('/'))
                // Reject time patterns like "3:00" — before must have alpha chars
                && before.chars().any(|c| c.is_ascii_alphabetic())
            {
                return true;
            }
        }
    }

    false
}

/// Check for function/method references with reduced false positives.
fn has_function_reference(lower: &str) -> bool {
    // Explicit keyword + identifier patterns
    let keywords = [
        "function ",
        "method ",
        "fn ",
        "def ",
        "class ",
        "struct ",
        "enum ",
        "trait ",
        "interface ",
        "impl ",
        "module ",
        "package ",
        "type ",
    ];
    for kw in &keywords {
        if let Some(pos) = lower.find(kw) {
            let after = &lower[pos + kw.len()..];
            // Next word should look like an identifier (starts with alpha/underscore)
            if after.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') {
                return true;
            }
        }
    }

    // Backtick-quoted identifiers: `fetchUser`, `my_func`
    if lower.contains('`') {
        let parts: Vec<&str> = lower.split('`').collect();
        for i in (1..parts.len()).step_by(2) {
            let ident = parts[i];
            if ident.len() > 1
                && ident.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
                && ident.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            {
                return true;
            }
        }
    }

    // snake_case identifiers in tokens (must contain underscore between alphanumeric chars)
    for token in split_tokens(lower) {
        let clean = token.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '_');
        if clean.len() > 4 && clean.contains('_') {
            // Must have alpha on both sides of at least one underscore
            let has_valid_snake = clean.split('_').filter(|p| !p.is_empty()).count() >= 2
                && clean
                    .split('_')
                    .all(|p| p.is_empty() || p.chars().all(|c| c.is_ascii_alphanumeric()));
            if has_valid_snake {
                // Exclude common English phrases that use underscores
                if clean != "right_now" && clean != "set_up" && !clean.starts_with("__") {
                    return true;
                }
            }
        }
    }

    false
}

/// Check for fenced code blocks (``` or indented 4+ spaces code).
fn has_code_block(text: &str) -> bool {
    if text.contains("```") {
        return true;
    }
    // 3+ consecutive lines indented by 4+ spaces
    let mut indented_count = 0u32;
    for line in text.lines() {
        if line.starts_with("    ") && !line.trim().is_empty() {
            indented_count += 1;
            if indented_count >= 3 {
                return true;
            }
        } else {
            indented_count = 0;
        }
    }
    false
}

// ── Actionability helpers ───────────────────────────────────────────────────

/// Check if the prompt starts with a strong action verb.
fn has_leading_strong_verb(lower: &str) -> bool {
    let first_word = lower.split_whitespace().next().unwrap_or("");
    STRONG_VERBS.contains(&first_word)
}

/// Check if a strong action verb appears anywhere (not just first word).
/// Handles "can you fix", "please add", "I need to refactor", etc.
fn has_strong_verb_anywhere(lower: &str) -> bool {
    let words: Vec<&str> = lower.split_whitespace().collect();
    for (i, word) in words.iter().enumerate() {
        // Strip common suffixes: fixing→fix, creates→create, etc.
        let stem = stem_verb(word);
        if STRONG_VERBS.contains(&stem.as_str()) {
            // Skip if it's in a clearly non-imperative position (e.g., "the fix was")
            // But allow after common preambles: "can you", "please", "I need to"
            if i == 0 {
                return true;
            }
            let prev = words[i - 1];
            // Accept after: you, to, please, and, also, then, we, i, let's, now
            let preamble_words = [
                "you", "to", "please", "and", "also", "then", "we", "i", "let's", "now", "just",
                "can", "could", "should", "will", "need", "want", "try",
            ];
            if preamble_words.contains(&prev) || i <= 3 {
                return true;
            }
        }
    }
    false
}

/// Check if prompt starts with a weak verb.
fn has_weak_verb_only(lower: &str) -> bool {
    let first_word = lower.split_whitespace().next().unwrap_or("");
    WEAK_VERBS.contains(&first_word) && !has_strong_verb_anywhere(lower)
}

/// Check if this is a direct CLI command.
fn is_cli_command(lower: &str) -> bool {
    CLI_COMMANDS.iter().any(|cmd| lower.starts_with(cmd))
}

/// Check if this is a minimal response (yes/no/ok/sure/etc.).
fn is_minimal_response(lower: &str) -> bool {
    let minimal = [
        "yes",
        "no",
        "ok",
        "okay",
        "sure",
        "yep",
        "nope",
        "y",
        "n",
        "k",
        "thanks",
        "thank you",
        "thx",
        "ty",
        "done",
        "next",
        "continue",
        "go ahead",
        "go on",
        "proceed",
        "yes please",
        "no thanks",
    ];
    let trimmed = lower.trim_end_matches(['.', '!', '?']);
    minimal.contains(&trimmed)
}

// ── Context helpers ─────────────────────────────────────────────────────────

/// Check for error context: error messages, exception names, HTTP codes.
fn has_error_context(lower: &str) -> bool {
    ERROR_PATTERNS.iter().any(|p| lower.contains(p))
}

/// Check for expected behavior descriptions.
fn has_expected_behavior(lower: &str) -> bool {
    EXPECTED_BEHAVIOR_PATTERNS.iter().any(|p| lower.contains(p))
}

/// Check for stack trace patterns.
fn has_stack_trace(text: &str) -> bool {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() < 2 {
        return false;
    }
    let trace_lines = lines
        .iter()
        .filter(|l| {
            let t = l.trim();
            t.starts_with("at ")
                || t.starts_with("File \"")
                || t.starts_with("Caused by:")
                || t.starts_with("note:")
                || t.starts_with("-->")  // Rust compiler
                || (t.contains("    at ") && t.contains(':'))
                || (t.contains(".java:") || t.contains(".py:") || t.contains(".rs:"))
                    && t.chars().filter(|&c| c == ':').count() >= 2
        })
        .count();
    trace_lines >= 2
}

/// Check for constraint/requirement language.
fn has_constraints(lower: &str) -> bool {
    CONSTRAINT_PATTERNS.iter().any(|p| lower.contains(p))
}

// ── Efficiency helpers ──────────────────────────────────────────────────────

/// Count vague phrases.
fn count_vague_phrases(lower: &str) -> usize {
    VAGUE_PHRASES.iter().filter(|p| lower.contains(**p)).count()
}

// ── Utility ─────────────────────────────────────────────────────────────────

/// Split text into tokens on whitespace, backticks, and quotes.
fn split_tokens(text: &str) -> Vec<&str> {
    text.split(|c: char| c.is_whitespace() || c == '`' || c == '"' || c == '\'')
        .filter(|s| !s.is_empty())
        .collect()
}

/// Basic verb stemming: removes common suffixes (ing, ed, es, s).
fn stem_verb(word: &str) -> String {
    let w = word.trim_matches(|c: char| !c.is_ascii_alphanumeric());
    if w.len() > 4 && w.ends_with("ing") {
        // fixing → fix, creating → creat(e)
        let stem = &w[..w.len() - 3];
        // Check if adding 'e' gives a known verb
        let with_e = format!("{}e", stem);
        if STRONG_VERBS.contains(&with_e.as_str()) {
            return with_e;
        }
        // Also check raw stem (e.g., "debugging" → "debug" needs special case)
        if STRONG_VERBS.contains(&stem) {
            return stem.to_string();
        }
    }
    if w.len() > 3 && w.ends_with("ed") {
        let stem = &w[..w.len() - 2];
        if STRONG_VERBS.contains(&stem) {
            return stem.to_string();
        }
        let with_e = format!("{}e", stem);
        if STRONG_VERBS.contains(&with_e.as_str()) {
            return with_e;
        }
    }
    if w.len() > 2 && w.ends_with('s') && !w.ends_with("ss") {
        let stem = &w[..w.len() - 1];
        if STRONG_VERBS.contains(&stem) {
            return stem.to_string();
        }
        // "es" suffix
        if let Some(stem2) = stem.strip_suffix('e') {
            if STRONG_VERBS.contains(&stem2) {
                return stem2.to_string();
            }
        }
    }
    w.to_string()
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Scoring tests ───────────────────────────────────────────────────

    #[test]
    fn test_excellent_prompt() {
        let q = evaluate("Fix the TypeError in src/api/users.ts:42 where `fetchUser` returns undefined instead of a User object");
        assert!(q.score >= 75, "Score {} should be >= 75", q.score);
        assert!(
            q.rating == "Excellent" || q.rating == "Good",
            "Rating: {}",
            q.rating
        );
        assert!(!q.issues.contains(&"no_file_reference".to_string()));
    }

    #[test]
    fn test_good_prompt() {
        let q = evaluate("Add a retry mechanism to the HTTP client in lib/http.rs");
        assert!(q.score >= 55, "Score {} should be >= 55", q.score);
    }

    #[test]
    fn test_poor_prompt() {
        let q = evaluate("fix it");
        assert!(q.score < 50, "Score {} should be < 50", q.score);
        assert!(q.issues.contains(&"vague_language".to_string()));
        assert!(q.issues.contains(&"too_short".to_string()));
    }

    #[test]
    fn test_very_poor_prompt() {
        let q = evaluate("do it");
        assert!(q.score < 40, "Score {} should be < 40", q.score);
    }

    #[test]
    fn test_empty_prompt() {
        let q = evaluate("");
        assert_eq!(q.score, 0);
        assert!(q.issues.contains(&"empty_prompt".to_string()));
    }

    #[test]
    fn test_continuation_prompt() {
        let q = evaluate(
            "This session is being continued from a previous conversation that ran out of context.",
        );
        assert_eq!(q.rating, "Continuation");
        assert_eq!(q.score, 50);
    }

    #[test]
    fn test_prompt_with_error_context() {
        let q = evaluate("Fix the TypeError: Cannot read properties of undefined in `handleClick`");
        assert!(
            q.score >= 60,
            "Score {} should be >= 60 (has error context + strong verb + function ref)",
            q.score
        );
    }

    #[test]
    fn test_prompt_with_expected_behavior() {
        let q = evaluate("Update the login form — it should redirect to /dashboard after successful authentication instead of staying on /login");
        assert!(q.score >= 50, "Score {} should be >= 50", q.score);
        assert!(
            q.rating == "Fair" || q.rating == "Good",
            "Rating: {}",
            q.rating
        );
    }

    #[test]
    fn test_vague_prompt_penalty() {
        let q1 = evaluate("make it work somehow, just do whatever you think is best");
        assert!(
            q1.score < 50,
            "Score {} should be < 50 (multiple vague phrases)",
            q1.score
        );
        assert!(q1.issues.contains(&"vague_language".to_string()));
    }

    #[test]
    fn test_score_badge_display() {
        let q = PromptQuality {
            score: 85,
            rating: "Good".to_string(),
            issues: vec![],
            category: None,
        };
        assert_eq!(score_badge(&q), "[A 85]");
    }

    // ── File reference tests ────────────────────────────────────────────

    #[test]
    fn test_has_file_reference() {
        assert!(has_file_reference("fix src/main.rs"));
        assert!(has_file_reference("update ./config.toml"));
        assert!(has_file_reference("edit lib/utils.py"));
        assert!(has_file_reference("check app.tsx"));
        assert!(!has_file_reference("fix the bug"));
    }

    #[test]
    fn test_file_reference_no_false_positive_substrings() {
        // ".css" and ".cpp" ARE valid extensions in our list, so they SHOULD match
        assert!(has_file_reference("update styles.css to be responsive"));
        assert!(has_file_reference("the main.cpp file compiles"));
        // ".c" must NOT false-positive match when the actual extension is longer
        // (e.g., ".cs" in "foo.css" should match as .css, not .c)
        assert!(has_file_reference("fix buffer.c overflow"));
        assert!(has_file_reference("update header.h"));
        // Plain English words with dots should NOT match
        assert!(!has_file_reference("e.g. this is fine"));
        assert!(!has_file_reference("fix the broken auth flow"));
    }

    #[test]
    fn test_file_reference_with_path() {
        assert!(has_file_reference("look at src/commands/checkpoint.rs"));
        assert!(has_file_reference("tests/test_api.py has a failure"));
    }

    // ── Line reference tests ────────────────────────────────────────────

    #[test]
    fn test_has_line_reference() {
        assert!(has_line_reference("fix error at src/main.rs:42"));
        assert!(has_line_reference("check line 15"));
        assert!(has_line_reference("see L42"));
        assert!(!has_line_reference("fix the bug"));
    }

    #[test]
    fn test_line_reference_no_false_positive_time() {
        // "3:00 pm." should NOT match as a line reference
        assert!(!has_line_reference("let's meet at 3:00 pm to discuss"));
        // But file:line should still work
        assert!(has_line_reference("error in utils.py:15"));
    }

    // ── Function reference tests ────────────────────────────────────────

    #[test]
    fn test_function_reference_keywords() {
        assert!(has_function_reference("fix the function handleclick"));
        assert!(has_function_reference("the fn parse_input is broken"));
        assert!(has_function_reference("class userservice needs update"));
    }

    #[test]
    fn test_function_reference_backtick() {
        assert!(has_function_reference("the `fetch_data` function fails"));
        assert!(has_function_reference("call `processqueue` here"));
    }

    #[test]
    fn test_function_reference_snake_case() {
        assert!(has_function_reference("update the parse_config function"));
        assert!(!has_function_reference("fix the bug now"));
    }

    // ── Classification tests ────────────────────────────────────────────

    #[test]
    fn test_classify_bug_fix() {
        assert_eq!(
            classify_prompt("fix the undefined error in auth"),
            "bug_fix"
        );
    }

    #[test]
    fn test_classify_code_generation() {
        assert_eq!(
            classify_prompt("create a new user model"),
            "code_generation"
        );
        assert_eq!(
            classify_prompt("implement pagination for the api"),
            "code_generation"
        );
    }

    #[test]
    fn test_classify_question() {
        assert_eq!(
            classify_prompt("how does the auth middleware work?"),
            "question"
        );
        assert_eq!(
            classify_prompt("what is the purpose of this function?"),
            "question"
        );
    }

    #[test]
    fn test_classify_refactor() {
        assert_eq!(classify_prompt("refactor the database module"), "refactor");
        assert_eq!(
            classify_prompt("extract the validation logic into a helper"),
            "refactor"
        );
    }

    #[test]
    fn test_classify_clarification() {
        assert_eq!(
            classify_prompt("no, i meant the other file"),
            "clarification"
        );
        assert_eq!(
            classify_prompt("actually, use async instead"),
            "clarification"
        );
    }

    #[test]
    fn test_classify_command() {
        assert_eq!(classify_prompt("cargo test --release"), "command");
        assert_eq!(classify_prompt("git status"), "command");
    }

    // ── CLI command tests ───────────────────────────────────────────────

    #[test]
    fn test_cli_command_high_score() {
        let q = evaluate("cargo test --release");
        assert!(
            q.score >= 50,
            "CLI commands should score >= 50, got {}",
            q.score
        );
        assert_eq!(q.category.as_deref(), Some("command"));
    }

    // ── Minimal response tests ──────────────────────────────────────────

    #[test]
    fn test_minimal_response_low_score() {
        let q = evaluate("yes");
        assert!(q.score < 30, "Score {} should be < 30", q.score);
        assert!(q.issues.contains(&"minimal_response".to_string()));
    }

    #[test]
    fn test_ok_is_minimal() {
        let q = evaluate("ok");
        assert!(q.score < 30, "Score {} should be < 30", q.score);
    }

    // ── Verb stemming tests ─────────────────────────────────────────────

    #[test]
    fn test_stem_verb() {
        assert_eq!(stem_verb("fixing"), "fix");
        assert_eq!(stem_verb("creating"), "create");
        assert_eq!(stem_verb("updated"), "update");
        assert_eq!(stem_verb("adds"), "add");
        assert_eq!(stem_verb("replaces"), "replace");
    }

    // ── Verb anywhere tests ─────────────────────────────────────────────

    #[test]
    fn test_strong_verb_after_preamble() {
        assert!(has_strong_verb_anywhere("can you fix the auth bug"));
        assert!(has_strong_verb_anywhere("please add a test for this"));
        assert!(has_strong_verb_anywhere("i need to refactor the module"));
    }

    // ── Code block tests ────────────────────────────────────────────────

    #[test]
    fn test_code_block_fenced() {
        assert!(has_code_block(
            "here is the error:\n```\nTypeError: foo\n```"
        ));
    }

    #[test]
    fn test_code_block_indented() {
        let text = "The code is:\n    fn main() {\n        println!(\"hello\");\n    }";
        assert!(has_code_block(text));
    }

    #[test]
    fn test_no_code_block() {
        assert!(!has_code_block("fix the bug in the auth module"));
    }

    // ── Constraint tests ────────────────────────────────────────────────

    #[test]
    fn test_constraints_detected() {
        assert!(has_constraints("this must be backwards compatible"));
        assert!(has_constraints(
            "keep the existing api surface, don't change the return type"
        ));
        assert!(!has_constraints("add a new endpoint"));
    }

    // ── Verbose prompt test ─────────────────────────────────────────────

    #[test]
    fn test_verbose_prompt_penalized() {
        let long_prompt = (0..600)
            .map(|i| format!("word{}", i))
            .collect::<Vec<_>>()
            .join(" ");
        let q = evaluate(&long_prompt);
        assert!(q.issues.contains(&"verbose".to_string()));
    }

    // ── Integration scoring test ────────────────────────────────────────

    #[test]
    fn test_prompt_with_everything() {
        let q = evaluate(
            "Fix the TypeError in src/api/users.ts:42 where `fetch_user` returns undefined. \
             The function should return a User object instead of null. Error: \
             TypeError: Cannot read properties of undefined (reading 'id'). \
             This must be backwards compatible with the existing API.",
        );
        assert!(
            q.score >= 75,
            "Full-context prompt should score >= 75, got {}",
            q.score
        );
    }

    #[test]
    fn test_prompt_with_code_block_and_error() {
        let q = evaluate(
            "Fix this compilation error:\n```\nerror[E0382]: borrow of moved value\n  --> src/main.rs:42\n```\nThe variable should be cloned before the move."
        );
        assert!(
            q.score >= 70,
            "Error + code block prompt should score >= 70, got {}",
            q.score
        );
    }

    #[test]
    fn test_category_present() {
        let q = evaluate("fix the bug in auth.rs");
        assert!(q.category.is_some(), "Category should always be set");
    }

    // ── Backwards compat: old struct without category deserializes ─────

    #[test]
    fn test_backwards_compat_no_category() {
        let json = r#"{"score":75,"rating":"Good","issues":[]}"#;
        let q: PromptQuality = serde_json::from_str(json).unwrap();
        assert_eq!(q.score, 75);
        assert!(q.category.is_none());
    }
}
