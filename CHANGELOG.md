# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-02-24

### Added

- **Core receipt system** — Captures AI code provenance as structured JSON receipts attached to Git commits via Git Notes (`refs/notes/blameprompt`).
- **Claude Code integration** — 10 lifecycle hooks (PreToolUse, PostToolUse, PostToolUseFailure, UserPromptSubmit, SessionStart, SessionEnd, Stop, SubagentStart, SubagentStop, Notification) auto-installed in `~/.claude/settings.json`.
- **Git hooks** — pre-commit, post-commit, post-checkout, post-merge, post-rewrite hooks installed globally via git template and per-repo.
- **Auto-setup** — First run silently installs global hooks and git template. No manual config needed.
- **`blameprompt init`** — Initialize per-repo or globally (`--global`).
- **`blameprompt blame <file>`** — Line-by-line AI vs human attribution with colored terminal output.
- **`blameprompt show <commit>`** — Display all AI receipts attached to a specific commit.
- **`blameprompt search <query>`** — Full-text search across stored prompts.
- **`blameprompt audit`** — Complete audit trail with date/author/format filters. Supports md, table, json, csv output.
- **`blameprompt analytics`** — Aggregated AI usage statistics with model breakdown, cost estimates, and session analysis. Export to json/csv.
- **`blameprompt report`** — Comprehensive markdown report: executive summary, AI vs human attribution, cost analysis, user contributions, time analysis, security audit, model comparison, file heatmap, session deep dive, and recommendations.
- **`blameprompt vuln-scan`** — Static analysis (SAST) on AI-generated code regions. Detects command injection, SQL injection, XSS, path traversal, hardcoded credentials, insecure deserialization, insecure randomness, ReDoS, dynamic code execution, and unprotected endpoints. Outputs severity-ranked findings with CWE references.
- **`blameprompt license-scan`** — Scans AI model licenses for open-source vs closed-source compliance.
- **`blameprompt supply-chain-risk`** — Assesses AI supply chain risk score across model diversity, vendor concentration, and deployment patterns.
- **`blameprompt prompt-injection`** — Detects prompt injection patterns in AI-generated code.
- **`blameprompt secret-rotation`** — Alerts on secrets that may need rotation after AI exposure.
- **`blameprompt push` / `pull`** — Share receipts with your team via Git Notes remotes.
- **`blameprompt record --session <file>`** — Import Claude Code JSONL session transcripts.
- **`blameprompt cache sync`** — Sync Git Notes into local SQLite cache for fast queries.
- **`blameprompt redact --test <file>`** — Dry-run the redaction engine on a file.
- **`blameprompt uninstall`** — Clean removal of all hooks and data. `--purge` removes Git Notes too.
- **Automatic redaction** — Secrets (API keys, passwords, tokens, AWS keys, private keys, bearer tokens, high-entropy strings) are auto-redacted before storage. Configurable via `.blameprompt/config.toml`.
- **Model classification** — Automatic detection of AI model vendor, license (open/closed), and deployment (local/cloud) for 20+ models.
- **Pricing engine** — Cost estimation for Claude, GPT, Gemini, DeepSeek, Llama, and other models.
- **Session statistics** — Duration tracking, response time analysis, and dev-hours-saved estimation.

### Technical

- Pure Rust, single binary, no runtime dependencies.
- Git Notes for storage — zero impact on working tree, fully compatible with existing workflows.
- SQLite cache for fast local queries.
- 42 unit tests covering receipts, redaction, pricing, model classification, sessions, transcripts, staging, and config.
- Zero compiler warnings, zero clippy warnings.

[0.1.0]: https://github.com/ekaanth/blameprompt/releases/tag/v0.1.0
