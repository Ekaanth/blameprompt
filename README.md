# blameprompt

**git blame, but for AI prompts.**

An open-source CLI that records exactly which AI prompt generated which code — every file, every line range, every model, every dollar spent. Stored as [Git Notes](https://git-scm.com/docs/git-notes) — nothing added to your working tree, nothing leaves your machine.

```
$ blameprompt blame src/auth.rs

 Line  Code                                        Source  Provider  Model              Cost     Prompt
 ──────────────────────────────────────────────────────────────────────────────────────────────────────────
    1  use jsonwebtoken::{encode, Header};          Human
    2  use serde::{Deserialize, Serialize};         Human
    3                                               Human
    4  #[derive(Serialize, Deserialize)]            AI      claude    claude-sonnet-4-5  $0.0112  Add JWT claims struct
    5  pub struct Claims {                          AI      claude    claude-sonnet-4-5  $0.0112  Add JWT claims struct
    6      pub sub: String,                         AI      claude    claude-sonnet-4-5  $0.0112  Add JWT claims struct
    7      pub exp: usize,                          AI      claude    claude-sonnet-4-5  $0.0112  Add JWT claims struct
    8  }                                            AI      claude    claude-sonnet-4-5  $0.0112  Add JWT claims struct
    9                                               Human
   10  pub fn validate(token: &str) -> bool {       Human
   11      // manual validation logic               Human

Code Origin: 45.5% AI-generated, 54.5% human
```

## Install

```bash
# macOS / Linux
curl -sSL https://blameprompt.com/install.sh | bash

# Windows (PowerShell)
irm https://blameprompt.com/install.ps1 | iex

# Or build from source
cargo install --path .
```

The installer automatically runs `blameprompt init --global` — setting up Claude Code hooks, Git template, and all detected agent hooks in one step. Every `git init` and `git clone` after that is tracked automatically.

## Quick start

After installing, log in to connect your dashboard:

```bash
# Log in via GitHub
blameprompt login

# Open your dashboard
blameprompt dash
```

Or sign in directly at [blameprompt.com/login](https://blameprompt.com/login) to connect your GitHub account and access your dashboard.

## How it works

```
AI Agent ──> hooks/import ──> staging.json ──> git commit ──> Git Notes
              (files, lines,     (local,        (post-commit    (refs/notes/
               prompt, model,     gitignored)     hook)          blameprompt)
               tools, agents,
               cost, tokens)
```

1. **You code with AI** — hooks fire in real time or you import sessions from other agents
2. **One receipt per prompt** — all files changed are grouped with the prompt that triggered them
3. **Receipts attach on commit** — `post-commit` hook writes everything as a Git Note
4. **Query anytime** — `blame`, `show`, `search`, `audit`, `analytics`, `report`, `diff`

Receipts survive rebases, merges, and cherry-picks via the `post-rewrite` hook.

## Supported agents

All detected agents are auto-configured by `blameprompt init --global`. If an agent isn't installed, it's silently skipped.

| Agent | Hook config | Import historical sessions |
|-------|------------|---------------------------|
| **Claude Code** | `~/.claude/settings.json` | Automatic |
| **GitHub Copilot** | `~/.github/hooks/blameprompt.json` | `blameprompt record-copilot` |
| **OpenAI Codex CLI** | `~/.codex/config.toml` | `blameprompt record-codex` |
| **Google Gemini CLI** | `~/.gemini/settings.json` | `blameprompt record-gemini` |
| **Cursor** | `~/.cursor/hooks.json` | `blameprompt record-cursor` |
| **Windsurf (Codeium)** | `~/.windsurf/hooks.json` | `blameprompt record-windsurf` |
| **Antigravity IDE** | `~/.antigravity/settings.json` | `blameprompt record-antigravity` |
| **Continue** | `~/.continue/hooks.json` | `blameprompt record-continue` |
| **Droid** | `~/.droid/hooks.json` | `blameprompt record-droid` |
| **JetBrains Junie** | `~/.junie/hooks.json` | `blameprompt record-junie` |
| **Atlassian Rovo Dev** | `~/.rovo-dev/hooks.json` | `blameprompt record-rovo-dev` |
| **Sourcegraph Amp** | `~/.amp/hooks.json` | `blameprompt record-amp` |
| **OpenCode** | `~/.opencode/hooks.json` | `blameprompt record-opencode` |
| **Any provider** | — | `blameprompt record --session <file> --provider <name>` |

The `record-*` commands import **historical sessions** from before blameprompt was installed. Once hooks are active, all new AI activity is tracked automatically.

## VS Code extension

Install from [VS Code Marketplace](https://marketplace.visualstudio.com/items?itemName=Blameprompt.blameprompt).

The companion extension provides three sidebar views:

- **Prompt Receipts** — tree view of commits > prompts > files/tools/agents
- **Prompt History** — git-log-style timeline with model-colored labels and filter search
- **File History** — all prompts that modified the currently open file, auto-updates on tab switch

Click any prompt to open a detailed receipt view with conversation turns, tool calls, and files touched.

## Commands

### Account

```bash
blameprompt login                   # authenticate via GitHub (opens browser)
blameprompt login --token <key>     # authenticate with API token (CI/headless)
blameprompt logout                  # clear stored credentials
blameprompt dash                    # open dashboard in browser
blameprompt profile                 # show your profile
blameprompt profile --edit          # edit profile in browser
blameprompt sync                    # upload aggregated metrics to BlamePrompt Cloud
```

### Attribution

```bash
blameprompt blame src/auth.rs       # line-by-line AI vs human
blameprompt show a1b2c3d            # all receipts for a commit
blameprompt search "JWT"            # full-text search across prompts
blameprompt diff                    # annotated working-tree diff
blameprompt diff a1b2c3d            # annotated commit diff
blameprompt check-provenance src/auth.rs          # AI vs human lines
blameprompt check-provenance src/auth.rs --line 5 # specific line
```

### Audit & reporting

```bash
blameprompt audit                           # full audit trail (md, table, json, csv)
blameprompt audit --from 2026-01-01 --author "Jane" --format json
blameprompt analytics                       # aggregated stats + cost breakdown
blameprompt report --output report.md       # comprehensive markdown report
```

### Security

```bash
blameprompt vuln-scan               # CWE pattern scanning on AI-generated code
blameprompt prompt-injection        # detect backdoors and hidden instructions
blameprompt secret-rotation         # flag secrets exposed to AI
blameprompt supply-chain-risk       # risk score 0-10
blameprompt license-scan            # model license compliance
```

### Hackathon fairness

```bash
blameprompt hackathon-report                    # last 24h, all participants
blameprompt hackathon-report --start "2026-02-26T09:00:00Z" --end "2026-02-26T21:00:00Z"
```

Generates an integrity report with timeline, code attribution, and anomaly detection (pre-written code, rehearsed prompts, activity gaps, etc.).

### Sharing & interop

```bash
blameprompt push                    # push notes to remote
blameprompt pull                    # fetch notes from remote
blameprompt cache sync              # build local SQLite cache
blameprompt export-agent-trace      # export to Agent Trace v0.1.0 format
blameprompt import-agent-trace      # display Agent Trace record
blameprompt github-comment          # post AI attribution as PR comment
```

### Setup & diagnostics

```bash
blameprompt init --global           # global setup (hooks, git template, agents)
blameprompt init                    # setup in current repo only
blameprompt install-git-wrap        # transparent git wrapper (auto-attach on commit)
blameprompt doctor                  # diagnose installation issues
blameprompt update                  # self-update
blameprompt uninstall               # remove hooks, keep receipt history
blameprompt uninstall --purge       # remove everything including Git Notes
```

## What gets captured

Every AI receipt includes: provider, model, user, timestamp, session ID, prompt & response summaries, files changed (with line ranges, additions, deletions), token usage (input, output, cache read, cache creation), real token-based cost, tools used, MCP servers called, agents spawned, conversation chain of thought, prompt quality score, prompt category, acceptance rate, and parent receipt links for session continuations.

Cost tracking uses actual API token data — cache reads priced at 90% discount, cache creation at 25% surcharge. Pricing supported for Claude, GPT-4o/4.1/o1/o3, Gemini 2.5, Codex, and more.

## Enterprise

BlamePrompt Enterprise provides organizational-level AI code observability:

- Aggregate AI code composition metrics across teams and repositories
- Agent and model effectiveness comparison
- Secure prompt storage with redaction and PII filtering
- Cross-repository dashboards and analytics

To learn more, visit [blameprompt.com](https://blameprompt.com).

## Data & privacy

Everything local by default. Nothing leaves your machine unless you `push` or `sync`.

| What | Where |
|------|-------|
| AI receipts | `refs/notes/blameprompt` (inside `.git`) |
| Staging | `.blameprompt/staging.json` (gitignored) |
| Credentials | `~/.blameprompt/credentials` |
| Cache | `~/.blameprompt/prompts.db` |
| Config | `.blamepromptrc` or `~/.blamepromptrc` |

Zero telemetry. Zero tracking. Built-in redaction engine strips secrets (API keys, passwords, AWS credentials, bearer tokens) before storage.

## License

[MIT](LICENSE)
