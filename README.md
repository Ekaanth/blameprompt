# blameprompt

**git blame, but for AI prompts.**

An open-source CLI that hooks into [Claude Code](https://claude.ai/claude-code) and records exactly which AI prompt generated which code — every file, every line range, every model, every dollar spent. Stored as [Git Notes](https://git-scm.com/docs/git-notes) — nothing added to your working tree, nothing leaves your machine.

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
Claude Code ──> hooks capture ──> staging.json ──> git commit ──> Git Notes
                 (files, lines,      (local,        (post-commit    (refs/notes/
                  prompt, model,      gitignored)     hook)          blameprompt)
                  tools, agents,
                  cost, session)
```

1. **You code with Claude** — hooks fire on every tool use (`Write`, `Edit`, `Bash`, etc.) and session lifecycle events
2. **One receipt per prompt** — all files changed in a session are grouped into a single receipt with the prompt that triggered them
3. **Receipts attach on commit** — `post-commit` hook writes everything as a Git Note on the commit
4. **Query anytime** — `blame`, `show`, `search`, `audit`, `analytics`, `report`

Receipts survive rebases, merges, and cherry-picks via the `post-rewrite` hook.

## What gets captured

Every AI receipt includes:

| Field | Example |
|-------|---------|
| **Provider** | `claude` |
| **Model** | `claude-sonnet-4-5-20250929` |
| **User** | `Jane Doe <jane@example.com>` |
| **Timestamp** | `2026-02-24T14:32:00Z` |
| **Session ID** | `a1b2c3d4-e5f6-7890-abcd-ef1234567890` |
| **Session start/end** | `2026-02-24T14:20:00Z` — `2026-02-24T14:32:00Z` |
| **Session duration** | `12m 34s` |
| **Files changed** | `src/auth.rs` (L4-8), `src/middleware.rs` (L15-30) |
| **Prompt summary** | `"Add JWT claims struct with sub and exp fields"` |
| **Prompt hash** | `sha256:9f86d08...` |
| **Message count** | `5` |
| **Estimated cost** | `$0.0342` |
| **Tools used** | `Bash`, `Write`, `Edit`, `Grep` |
| **MCP servers** | `filesystem`, `github` |
| **Agents spawned** | `"Explore codebase (Explore)"`, `"Run tests (Bash)"` |
| **Chain of thought** | Full conversation turns (user, AI, tool calls) |
| **Parent receipt** | Links to previous receipt in the session chain |

### Prompt-centric model

One prompt = one receipt = all files changed. This matches how AI coding actually works — a single prompt often touches multiple files across a codebase.

```json
{
  "prompt_summary": "Add JWT validation middleware",
  "files_changed": [
    { "path": "src/auth.rs", "line_range": [4, 8] },
    { "path": "src/middleware.rs", "line_range": [15, 30] },
    { "path": "tests/auth_test.rs", "line_range": [1, 45] }
  ],
  "tools_used": ["Write", "Edit", "Bash"],
  "agents_spawned": ["Run tests (Bash)"],
  "cost_usd": 0.0342
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
```

### Audit & reporting

```bash
blameprompt audit                           # full audit trail (md, table, json, csv)
blameprompt audit --from 2026-01-01 --author "Jane" --format json
blameprompt analytics                       # aggregated stats + cost breakdown
blameprompt report --output report.md       # comprehensive 11-section markdown report
```

The report includes: executive summary, AI vs human attribution, cost analysis, user contributions, time analysis (with merged wall-clock time), security audit, model comparison (open-source vs closed), file heatmap, session deep dive, prompt details, and recommendations.

### Security

```bash
blameprompt vuln-scan               # 10 CWE patterns on AI-generated code
blameprompt prompt-injection        # detect backdoors and hidden instructions
blameprompt secret-rotation         # flag secrets exposed to AI
blameprompt supply-chain-risk       # risk score 0-10
blameprompt license-scan            # model license compliance
```

### Sharing

```bash
blameprompt push                    # push notes to remote
blameprompt pull                    # fetch notes from remote
blameprompt cache sync              # build local SQLite cache
blameprompt record --session <file> # import Claude Code transcript
```

All reporting commands support `--from`, `--to`, `--author`, and `--include-uncommitted`.

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

Five hooks cover the full lifecycle:

| Hook | What it does |
|------|-------------|
| `pre-commit` | Reports staged receipt count |
| `post-commit` | Writes receipts as Git Notes |
| `post-checkout` | Auto-initializes `.blameprompt/` in new clones, adds to `.gitignore`, fetches remote notes |
| `post-merge` | Preserves staged receipts |
| `post-rewrite` | Remaps notes after rebase/amend |

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
| Staging | `.blameprompt/staging.json` (gitignored) |
| Cache | `~/.blameprompt/prompts.db` |
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
