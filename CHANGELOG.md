# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.0.1] - 2026-03-22

### Fixed

- **Token count 773x undercount in cloud sync** — `total_tokens_in` now includes `cache_read_tokens` and `cache_creation_tokens`. Previously only counted non-cached `input_tokens`, massively undercounting actual consumption since prompt caching serves the vast majority of input tokens.
- **File additions/deletions overwritten per prompt** — Each PostToolUse was overwriting `total_additions`/`total_deletions` instead of accumulating. Now recomputed from the merged `files_changed` vector on every upsert, so a prompt editing 11 files correctly reflects all 11.
- **Untracked files reported as 0 additions** — `get_diff_stats()` now falls back to counting lines via `std::fs::read_to_string` for new/untracked files that `git diff` can't see. Previously returned `(0, 0)` for all files in untracked directories (e.g. `docs/`).
- **Timestamp misalignment in transcript parsing** — `user_prompt_timestamps` vector could fall out of sync with `count_user_prompts()` if a timestamp failed to parse, causing all subsequent `timestamp_for_prompt()` calls to return the wrong prompt's timestamp. Now always pushes an entry (falls back to `Utc::now()`).
- **`paths_match()` false positives** — Suffix matching like `"ob.rs"` matching `"a/bob.rs"` is now prevented by requiring a `/` separator before the matched suffix.
- **File extension matching case-sensitive** — Prompt quality scoring now lowercases tokens before matching extensions, so `Main.RS` correctly detects a file reference.
- **`sync_cloud.rs` panic on missing home dir** — Replaced `.expect()` with `Result` return in `sync_state_path()`.
- **`show.rs` inconsistent SHA display** — Replaced inline `&sha[..8]` with `util::short_sha()`.
- **`handle_ask_user_question` race condition** — Replaced direct staging mutation with `upsert_receipt_in()` to avoid data loss from concurrent hook events.
- **`get_changed_lines()` misleading default** — Returns `(0, 0)` instead of `(1, 1)` for deleted/nonexistent files.
- **API client hangs indefinitely** — Added 30s request timeout and 10s connect timeout to `reqwest` client.
- **API error messages vague** — Error responses now include the response body (first 200 chars) instead of just the HTTP status code.
- **Login device flow hangs forever** — Added 10-minute polling timeout (120 attempts at 5s intervals).
- **CSV audit export breaks on special chars** — Added proper `csv_escape()` that wraps fields containing commas, quotes, or newlines in double-quotes with escaped inner quotes.
- **Supply chain risk score inflated 10x** — Formula had an extra `* 10.0` multiplier after weighted average. Risk scores now correctly stay in the 0–10 range.
- **Session timing fields lost on upsert** — `session_start`, `session_duration_secs`, and `ai_response_time_secs` are now preserved through staging merge (previously overwritten by `*existing = receipt.clone()`).
- **Corrupted config silently replaced** — `claude_hooks.rs` install now returns an error if the settings JSON can't be parsed, instead of silently creating an empty object (which would destroy the user's other hooks).
- **`blame.rs` fuzzy path matching** — Replaced inline triple-OR condition with `util::paths_match()` to prevent false positives across different directories.
- **Language stats inflated in sync** — Language detection now deduplicates by unique file per day, so editing `main.rs` ten times counts as 1 Rust file, not 10.
- **Agent double-counting in sync** — `agents_spawned` list is now only counted when `subagent_activities` is empty, preventing the same agent from being counted twice.
- **GitHub PR number extraction edge case** — `extract_first_pr_number` now handles PR number at the end of a JSON string.
- **Rebase line offset defaults** — Hunk header parse defaults changed from `0` to `1` to match 1-based diff line numbering.

### Added

- **Agent & skill tracking in cloud sync** — DailyActivity now includes `agents_spawned`, `agent_types_used`, `total_agent_count`, `total_agent_duration_secs`, `skills_used`, and `total_user_decisions` for full agent/skill analytics on the dashboard.

### Changed

- **Install scripts use redirect-based version detection** — `install.sh` and `install.ps1` now extract the latest version from the GitHub releases redirect URL instead of the JSON API, avoiding 403 rate-limit errors on unauthenticated requests.
- **Messaging updated to portfolio/hiring angle** — All user-facing text (installer, CLI about, banner, social posts, docs) updated from "Track AI-generated code" to "Your AI skills deserve a portfolio".

## [1.0.0] - 2026-03-20

### Added

- **6 new agent integrations** — Continue (`record-continue`), Droid (`record-droid`), JetBrains Junie (`record-junie`), Atlassian Rovo Dev (`record-rovo-dev`), Sourcegraph Amp (`record-amp`), and OpenCode (`record-opencode`). All auto-configured by `blameprompt init --global`. Total supported agents: 13.
- **BlamePrompt Cloud** — New `login`, `logout`, `dash`, `profile`, and `sync` commands. Authenticate via GitHub Device Flow or API token. `blameprompt dash` opens your dashboard; `blameprompt profile --edit` opens profile settings. `blameprompt sync` uploads aggregated daily metrics (not individual prompts) to the cloud.
- **Browser login fallback** — When the device flow API is unavailable, `blameprompt login` gracefully falls back to opening `blameprompt.com/login` in the browser with instructions to use `--token`.
- **`blameprompt doctor`** — Diagnostic command that checks hook installation, binary paths, staging state, and git notes health.
- **One-line installers** — `curl -sSL blameprompt.com/install.sh | bash` (macOS/Linux) and `irm blameprompt.com/install.ps1 | iex` (Windows). Both auto-run `blameprompt init --global` after install.

### Fixed

- **Post-commit hook silently swallowing output** — The hook redirected all stderr to `/dev/null`, hiding both success confirmations and error messages. Now logs errors to `.git/blameprompt-hook.log` and prints success to stdout.
- **Attach success message invisible** — `blameprompt attach` printed its `[BlamePrompt] N receipt(s) attached` message to stderr, which the hook suppressed. Changed to stdout so it appears during `git commit`.
- **Git notes errors silently discarded** — `git notes add` stderr was piped to `Stdio::null()`, hiding permission errors, ref corruption, and other failures. Now captured and included in the error message.
- **Clippy `comparison_chain` warning** — Replaced `if/else if` chain in supply chain risk scoring with idiomatic `match` on `cmp()`.
- **Clippy `manual_is_multiple_of` lint** — Added `#[allow(unknown_lints, clippy::manual_is_multiple_of)]` for cross-version compatibility (lint exists on nightly/1.94+ but not on older stable).
- **Git push slowness** — Multiple fixes to pre-push hook and git wrap shim for note pushing.
- **Monorepo subfolder staging** — Fixed staging.json discovery when blameprompt is run from a subfolder.
- **Global install reliability** — Fixed `blameprompt init --global` edge cases.

### Changed

- **CI workflow optimized** — Removed redundant `cargo build` step (clippy + test already compile). Added `restore-keys` for partial cache hits.
- **Release workflow hardened** — macOS x86 now builds on Intel runners (`macos-13`) instead of cross-compiling on ARM. Added cargo caching. Fixed `workflow_dispatch` to require a tag input. Release job now runs on both tag push and manual dispatch.
- **README rewritten** — Streamlined from ~350 lines to ~170 lines. Added install scripts, quick start with login flow, all 13 agents, account commands, and diagnostics section. Removed verbose tables and diagrams.
- **Domain URLs consolidated** — All source references point to `blameprompt.com` (`api.blameprompt.com` for API).
- Total test count: 191 (up from 186 in v0.3.0).

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

[1.0.1]: https://github.com/ekaanth/blameprompt/releases/tag/v1.0.1
[1.0.0]: https://github.com/ekaanth/blameprompt/releases/tag/v1.0.0
[0.3.0]: https://github.com/ekaanth/blameprompt/releases/tag/v0.3.0
[0.2.0]: https://github.com/ekaanth/blameprompt/releases/tag/v0.2.0
[0.1.0]: https://github.com/ekaanth/blameprompt/releases/tag/v0.1.0
