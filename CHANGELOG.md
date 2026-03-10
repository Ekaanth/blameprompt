# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.0] - 2026-03-10

### Added

- **Antigravity IDE integration** — New `blameprompt record-antigravity` and `blameprompt install-antigravity` commands. Full session import with auto-detection of Antigravity-native models, Gemini 2.x/3.x, and Claude models served through the Antigravity platform. Installs `.agent/rules/blameprompt.md` and `.agent/workflows/checkpoint.md` for automatic provenance tracking.
- **Prompt quality evaluation engine** 4-dimension weighted scoring system. Scores prompts on Clarity (30%), Actionability (25%), Context (25%), and Efficiency (20%). Classifies prompts into categories: bug_fix, code_generation, question, refactor, command, clarification, file_operation, continuation.
- **Prompt category distribution in reports** — `blameprompt report` now shows prompt category breakdown (bug fixes vs. code generation vs. refactoring, etc.) and anti-pattern analysis (verbose prompts, minimal responses, vague language).
- **Git notes merge on attach** — `blameprompt attach` now merges with existing notes on a commit instead of overwriting, preventing receipt loss on amend or multi-stage workflows.
- **Duplicate fetch-refspec prevention** — `blameprompt pull` no longer appends duplicate `remote.origin.fetch` entries for `refs/notes/blameprompt`.

### Fixed

- **Opus 4.5 / 4.6 pricing bug** — `opus-4-6` and `opus-4.6` (dot vs hyphen notation) were mapped to different price tiers ($5/$25 vs $15/$75). Both now correctly resolve to the same rate. Opus 4.5 was incorrectly priced at the 4.6 tier; now correctly set to $15/$75.
- **Missing model classifier entries** — Added `sonnet-4-6` ("Claude Sonnet 4.6") and `o4`/`o4-mini` display names. Previously fell through to "Claude (unknown)" and unclassified respectively.
- **Duplicated `is_blameprompt_ignored` function** — Removed ~55-line duplicate from `init_hooks.rs`; now delegates to the canonical implementation in `staging.rs`.
- **Prevent silent re-installation after uninstall** — `blameprompt uninstall` now writes a sentinel marker; `auto_setup()` respects it and will not silently reinstall hooks.
- **Duplicate `.blameprompt` gitignore entries** — Checks local `.gitignore`, global `core.excludesFile`, and `.git/info/exclude` before appending, preventing duplicate lines.
- **Self-updater version pinning** — `blameprompt update` now targets a specific release version instead of always pulling latest.

### Changed

- Prompt evaluation now runs at receipt creation time across all providers (Claude, Gemini, Cursor, Codex, Copilot, Windsurf, Antigravity), not just at report generation.
- Rating tiers: Excellent (90+), Good (70-89), Fair (50-69), Poor (<50).
- 37 new unit tests for prompt evaluation covering scoring, classification, false positives, edge cases, and backward compatibility.
- Total test count: 186 (up from 159 in v0.2.0).

### Technical

- `PromptQuality` struct extended with backward-compatible `category: Option<String>` field.
- `staging::is_blameprompt_ignored` promoted to `pub(crate)` to eliminate cross-module duplication.
- Verb stemming, CLI command detection, minimal response detection, and file extension boundary matching in the evaluation engine.
- Continuation prompts auto-detected and scored as "Good" (score 75) to avoid penalizing context-window resumptions.

## [0.2.0] - 2026-03-03

### Added

- **Multi-agent support** — Added session recording and attribution for Cursor, GitHub Copilot, OpenAI Codex, Google Gemini, and Windsurf (`blameprompt record-*`).
- **Subagent activity tracking** — Captures the full tree of sub-agents spawned by primary tools (Explore, Plan, etc.), including tool usage and duration.
- **Per-token cost isolation** — Granular tracking of input, output, cache read, and cache creation tokens for hyper-accurate spend reporting.
- **Cache-aware pricing** — Support for 30+ models with automatic 90% discounts for cached reads and 25% surcharges for cache creation.
- **`blameprompt github-comment`** — Automated PR attribution summaries for CI/CD, including model used, cost, and security scan results.
- **`blameprompt hackathon-report`** — Verifiable audit engine for AI vs. human contributions in coding competitions.
- **`blameprompt diff`** — Specialized diff viewer showing exactly which lines in a change were AI-generated vs. human-written.
- **Accepted vs. Overridden tracking** — Compares AI suggestions against final commits to track developer oversight and AI efficacy.
- **VS Code Extension** — Native editor integration with gutter markers, hover tooltips, prompt history sidebar, and interactive conversation graphs.
- **`blameprompt update`** — Built-in self-updater for seamless CLI evolution.
- **Refined Receipt Metadata** — Added response summaries, file change metadata, and full conversation turn history to the storage layer.

### Changed

- Updated pricing engine to support the latest 2026 model pricing (Claude 4.6, GPT-4.1, Gemini 2.5).
- Improved CLI output formatting for `blame` and `audit` commands.
- Enhanced redaction engine with broader pattern coverage for cloud provider secrets.

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

[0.3.0]: https://github.com/ekaanth/blameprompt/releases/tag/v0.3.0
[0.2.0]: https://github.com/ekaanth/blameprompt/releases/tag/v0.2.0
[0.1.0]: https://github.com/ekaanth/blameprompt/releases/tag/v0.1.0
