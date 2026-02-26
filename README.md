# blameprompt

**git blame, but for AI prompts.**

An open-source CLI that records exactly which AI prompt generated which code — every file, every line range, every model, every dollar spent. Works with **Claude Code, GitHub Copilot, OpenAI Codex CLI, Google Gemini CLI, Cursor, and Windsurf**. Stored as [Git Notes](https://git-scm.com/docs/git-notes) — nothing added to your working tree, nothing leaves your machine.

```
$ blameprompt blame src/auth.rs

  src/auth.rs

   1 | use jsonwebtoken::{encode, Header};    human
   2 | use serde::{Deserialize, Serialize};   human
   3 |
   4 | #[derive(Serialize, Deserialize)]       AI  Claude Sonnet 4.5  a1b2c3d  2026-02-24
   5 | pub struct Claims {                     AI  Claude Sonnet 4.5  a1b2c3d  2026-02-24
   6 |     pub sub: String,                    AI  Claude Sonnet 4.5  a1b2c3d  2026-02-24
   7 |     pub exp: usize,                     AI  Claude Sonnet 4.5  a1b2c3d  2026-02-24
   8 | }                                       AI  Claude Sonnet 4.5  a1b2c3d  2026-02-24
   9 |
  10 | pub fn validate(token: &str) -> bool {  human
  11 |     // manual validation logic           human

  5/11 lines (45.5%) written by AI
```

## Supported agents

| Agent | Integration | Import command |
|-------|------------|----------------|
| **Claude Code** | Real-time hooks (10 events) | Automatic via `~/.claude/settings.json` |
| **GitHub Copilot** | VS Code workspace SQLite | `blameprompt record-copilot` |
| **OpenAI Codex CLI** | JSONL transcript parser | `blameprompt record-codex` |
| **Google Gemini CLI** | JSONL/JSON session parser | `blameprompt record-gemini` |
| **Cursor** | Workspace SQLite | `blameprompt record-cursor` |
| **Windsurf** | Workspace SQLite | `blameprompt record-windsurf` |
| **Any provider** | Manual JSONL import | `blameprompt record --session <file> --provider <name>` |

## Install

```bash
cargo install --path .
```

First run auto-configures everything globally — Claude Code hooks, Git template, the works. Every `git init` and `git clone` after that is tracked automatically.

```bash
blameprompt init --global    # explicit global setup
blameprompt init             # setup in current repo only
```

## How it works

```
AI Agent ──> hooks/import ──> staging.json ──> git commit ──> Git Notes
              (files, lines,     (local,        (post-commit    (refs/notes/
               prompt, model,     gitignored)     hook)          blameprompt)
               tools, agents,
               cost, tokens)
```

1. **You code with AI** — hooks fire in real time (Claude Code) or you import sessions from other agents
2. **One receipt per prompt** — all files changed are grouped into a single receipt with the prompt that triggered them
3. **Receipts attach on commit** — `post-commit` hook writes everything as a Git Note on the commit
4. **Query anytime** — `blame`, `show`, `search`, `audit`, `analytics`, `report`, `diff`

Receipts survive rebases, merges, and cherry-picks via the `post-rewrite` hook.

## What gets captured

Every AI receipt includes:

| Field | Example |
|-------|---------|
| **Provider** | `claude`, `copilot`, `codex`, `gemini`, `cursor`, `windsurf` |
| **Model** | `claude-opus-4-6`, `gpt-4o`, `gemini-2.5-pro`, `codex-mini` |
| **User** | `Jane Doe <jane@example.com>` |
| **Timestamp** | `2026-02-24T14:32:00Z` |
| **Session ID** | `a1b2c3d4-e5f6-7890-abcd-ef1234567890` |
| **Prompt duration** | `45s` (wall-clock time for this prompt) |
| **Session duration** | `12m 34s` (total session time) |
| **Files changed** | `src/auth.rs` (L4-8, +5-0), `src/middleware.rs` (L15-30, +16-2) |
| **Prompt summary** | `"Add JWT claims struct with sub and exp fields"` |
| **Response summary** | `"Added Claims struct with sub/exp fields and JWT validation"` |
| **Prompt hash** | `sha256:9f86d08...` |
| **Token usage** | input: 12,450 / output: 3,200 / cache_read: 8,100 / cache_creation: 1,500 |
| **Cost** | `$0.0342` (real token-based, not estimated) |
| **Tools used** | `Bash`, `Write`, `Edit`, `Grep` |
| **MCP servers** | `filesystem`, `github` |
| **Agents spawned** | `"Explore codebase (Explore)"`, `"Run tests (Bash)"` |
| **Chain of thought** | Full conversation turns (user, AI, tool calls) |
| **Acceptance rate** | `92% (46 accepted, 4 overridden)` |
| **Parent receipt** | Links to previous receipt in the session chain |

### Real token-based cost tracking

BlamePrompt parses actual API token usage data (input, output, cache reads, cache creation) from session transcripts. Cache reads are priced at 90% discount, cache creation at 25% surcharge. No more guessing based on character counts.

Pricing supported for: Claude (Opus, Sonnet, Haiku), GPT-4o/4.1/o1/o3, Gemini 2.5 Pro/Flash, Codex, and more.

### Prompt-centric model

One prompt = one receipt = all files changed. This matches how AI coding actually works — a single prompt often touches multiple files across a codebase.

```json
{
  "prompt_summary": "Add JWT validation middleware",
  "response_summary": "Added Claims struct, JWT middleware, and tests",
  "files_changed": [
    { "path": "src/auth.rs", "line_range": [4, 8], "additions": 5, "deletions": 0 },
    { "path": "src/middleware.rs", "line_range": [15, 30], "additions": 16, "deletions": 2 },
    { "path": "tests/auth_test.rs", "line_range": [1, 45], "additions": 45, "deletions": 0 }
  ],
  "tools_used": ["Write", "Edit", "Bash"],
  "agents_spawned": ["Run tests (Bash)"],
  "cost_usd": 0.0342,
  "input_tokens": 12450,
  "output_tokens": 3200
}
```

### Time tracking with parallel agent merging

When Claude Code spawns sub-agents (Task tool), they run in parallel with different session IDs. BlamePrompt merges overlapping time intervals so parallel work isn't double-counted:

```
Main agent:  |████████████████████|  10:00 - 10:10  (600s)
Sub-agent A:     |████████|           10:02 - 10:06  (240s)
Sub-agent B:        |██████████|      10:04 - 10:08  (240s)
                                      ────────────────────
Wall-clock (merged):                  600s (not 1080s)
```

## Commands

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
blameprompt report --output report.md       # comprehensive 11-section markdown report
```

The report includes: executive summary, AI vs human attribution, cost analysis, user contributions, time analysis (with merged wall-clock time), security audit, model comparison (open-source vs closed), file heatmap, session deep dive, prompt details, and recommendations.

### Import from other agents

```bash
blameprompt record-copilot                  # import GitHub Copilot Chat sessions
blameprompt record-codex                    # import OpenAI Codex CLI transcripts
blameprompt record-gemini                   # import Google Gemini CLI sessions
blameprompt record-cursor                   # import Cursor IDE sessions
blameprompt record-windsurf                 # import Windsurf/Codeium sessions
blameprompt record --session <file>         # import any JSONL transcript
blameprompt record --session <file> --provider openai
```

Each import command auto-discovers session data from default locations, or accepts explicit paths:

```bash
blameprompt record-copilot --workspace ~/path/to/state.vscdb
blameprompt record-codex --session ~/.codex/sessions/
blameprompt record-gemini --session ~/session.jsonl
blameprompt record-windsurf --workspace ~/path/to/state.vscdb
```

### Security

```bash
blameprompt vuln-scan               # 10 CWE patterns on AI-generated code
blameprompt prompt-injection        # detect backdoors and hidden instructions
blameprompt secret-rotation         # flag secrets exposed to AI
blameprompt supply-chain-risk       # risk score 0-10
blameprompt license-scan            # model license compliance
```

### Hackathon fairness verification

Generate a report proving code was built during the hackathon — not pre-written and pasted in.

```bash
blameprompt hackathon-report                    # last 24h, all participants
blameprompt hackathon-report --start "2026-02-26T09:00:00Z" --end "2026-02-26T21:00:00Z"
blameprompt hackathon-report --author "John" --include-uncommitted
```

The report includes:

1. **Summary** — integrity score (0-100: PASS/REVIEW/FAIL), prompt count, AI lines, cost
2. **Timeline** — every prompt chronologically with timestamp, duration, model, files touched
3. **Code attribution** — AI vs human % overall and per-file
4. **Anomaly detection** — 7 detectors flag suspicious patterns:
   - Prompts outside the hackathon time window
   - Pre-written code (file appears fully-formed with no iteration)
   - Source files with no receipt trail (manual paste-ins)
   - Duplicate prompt hashes (rehearsed prompts)
   - Batch commits with low receipt coverage
   - Unusually fast output (short session, many lines)
   - Long activity gaps during the hackathon
5. **Integrity assessment** — weighted score breakdown with conclusion

### Sharing & interop

```bash
blameprompt push                    # push notes to remote
blameprompt pull                    # fetch notes from remote
blameprompt cache sync              # build local SQLite cache
blameprompt export-agent-trace      # export to Agent Trace v0.1.0 format
blameprompt import-agent-trace      # display Agent Trace record
blameprompt github-comment          # post AI attribution as PR comment
```

All reporting commands support `--from`, `--to`, `--author`, and `--include-uncommitted`.

### Transparent git wrapper

Auto-attaches receipts on every `git commit` and auto-pushes notes on `git push`, with zero workflow changes:

```bash
blameprompt install-git-wrap        # install ~/.blameprompt/bin/git shim
```

## VS Code extension

The companion [blameprompt-vscode](https://github.com/ekaanth/blameprompt-vscode) extension provides a rich sidebar with three views:

**Prompt Receipts** — tree view structured as:
```
commit (subject · author · sha · time ago)
  └─ "Add JWT validation..." (sonnet-4-5 · 3 files · $0.03 · 2m ago)
       ├─ src/auth.rs:4-8           (5L)      <- click to open file
       ├─ src/middleware.rs:15-30   (16L)
       ├─ Write                     tool      <- tools used
       ├─ Edit                      tool
       ├─ github                    MCP server <- MCP servers called
       └─ "Run tests (Bash)"       sub-agent  <- agents spawned
```

**Prompt History** — git-log-style visual timeline with:
- Commit rows showing author, SHA, prompt count, cost
- Collapsible file lists per prompt
- Model-colored labels (Claude=orange, OpenAI=green, Google=blue)
- Tool/MCP/agent chips on each prompt
- Real-time filter search

**File History** — shows all prompts that modified the currently open file, auto-updates as you switch tabs.

Click any prompt to open a detailed receipt view with the full chain of thought (conversation turns, tool calls, files touched).

## Git hooks

Seven hooks cover the full lifecycle:

| Hook | What it does |
|------|-------------|
| `pre-commit` | Reports staged receipt count |
| `prepare-commit-msg` | Annotates commit editor with AI receipt info |
| `post-commit` | Writes receipts as Git Notes |
| `post-checkout` | Auto-initializes `.blameprompt/` in new clones, adds to `.gitignore`, fetches remote notes |
| `post-merge` | Preserves staged receipts |
| `pre-push` | Auto-pushes `refs/notes/blameprompt` to remote |
| `post-rewrite` | Remaps notes after rebase/amend with line-offset adjustment |

Installed globally via `init.templateDir`. Every new repo gets them automatically.

## Configuration

Optional. Create `.blamepromptrc` in your repo or `~/.blamepromptrc`:

```toml
[redaction]
mode = "replace"    # or "hash"

[[redaction.custom_patterns]]
pattern = "INTERNAL-\\d{6}"
replacement = "[REDACTED_INTERNAL_ID]"

[capture]
max_prompt_length = 2000
store_full_conversation = true
```

## Data

Everything local. Nothing leaves your machine unless you `push`.

| What | Where |
|------|-------|
| AI receipts | `refs/notes/blameprompt` (inside `.git`) |
| Agent Trace | `refs/notes/agent-trace` (inside `.git`) |
| Staging | `.blameprompt/staging.json` (gitignored) |
| Cache | `~/.blameprompt/prompts.db` |
| Git wrapper | `~/.blameprompt/bin/git` |
| Template | `~/.blameprompt/git-template/` |
| Config | `.blamepromptrc` or `~/.blamepromptrc` |

## Privacy

Zero network calls (except `git push`/`pull`). Zero API keys. Zero telemetry. Zero accounts. Built-in redaction engine strips secrets (API keys, passwords, AWS credentials, bearer tokens, high-entropy strings) before storage. Configurable patterns for org-specific secrets.

## Uninstall

```bash
blameprompt uninstall               # remove hooks, keep receipt history
blameprompt uninstall --purge       # remove everything including Git Notes
```

## License

[MIT](LICENSE)
