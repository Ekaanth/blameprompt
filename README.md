# blameprompt

**Every AI-written line, accounted for.**

An open-source CLI that hooks into [Claude Code](https://claude.ai/claude-code) and records exactly which lines AI wrote, which prompt triggered them, who ran the session, and when. Stored as [Git Notes](https://git-scm.com/docs/git-notes) — nothing added to your repo, nothing leaves your machine.

```
$ blameprompt blame src/auth.rs

  src/auth.rs

   1 │ use jsonwebtoken::{encode, Header};    human
   2 │ use serde::{Deserialize, Serialize};   human
   3 │
   4 │ #[derive(Serialize, Deserialize)]       AI  Claude Sonnet 4.5  a1b2c3d  2026-02-24
   5 │ pub struct Claims {                     AI  Claude Sonnet 4.5  a1b2c3d  2026-02-24
   6 │     pub sub: String,                    AI  Claude Sonnet 4.5  a1b2c3d  2026-02-24
   7 │     pub exp: usize,                     AI  Claude Sonnet 4.5  a1b2c3d  2026-02-24
   8 │ }                                       AI  Claude Sonnet 4.5  a1b2c3d  2026-02-24
   9 │
  10 │ pub fn validate(token: &str) -> bool {  human
  11 │     // manual validation logic           human

  5/11 lines (45.5%) written by AI
```

## Install

```bash
curl -sSL https://blameprompt.com/install.sh | bash
```

Or build from source:

```bash
cargo install blameprompt
```

First run auto-configures everything globally — Claude Code hooks, Git template, the works. Every `git init` and `git clone` after that is tracked automatically.

## How it works

```
Claude Code ──▶ hooks capture ──▶ staging.json ──▶ git commit ──▶ Git Notes
                (file, lines,       (local)        (post-commit    (refs/notes/
                 prompt, model,                      hook)          blameprompt)
                 user, timestamp)
```

1. **You code with Claude** — hooks fire on `PreToolUse`, `PostToolUse`, `SessionStart`, `SessionEnd`
2. **Edits are staged** — file paths, line ranges, prompt text, model, user, timestamp, session ID, cost estimate
3. **Receipts attach on commit** — `post-commit` hook writes everything as a Git Note
4. **Query anytime** — `blame`, `audit`, `show`, `search`, `analytics`, `report`

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
| **Session duration** | `12m 34s` |
| **File** | `src/auth.rs` |
| **Line range** | `4-8` |
| **Prompt summary** | `"Add JWT claims struct with sub and exp fields"` |
| **Prompt hash** | `sha256:9f86d08...` |
| **Message count** | `5` |
| **Estimated cost** | `$0.0342` |

## Commands

### Attribution

```bash
$ blameprompt blame src/auth.rs       # line-by-line AI vs human

$ blameprompt show a1b2c3d            # all receipts for a commit
  Commit a1b2c3d — 2026-02-24 14:32 — Jane Doe <jane@example.com>
  ┌──────────┬──────────────────┬──────┬──────────┬────────┬────────────────┐
  │ Provider │ Model            │ Cost │ File     │ Lines  │ Prompt Summary │
  ├──────────┼──────────────────┼──────┼──────────┼────────┼────────────────┤
  │ claude   │ Claude Sonnet 4.5│$0.03 │src/auth.rs│ 4-8   │"Add JWT claims"│
  └──────────┴──────────────────┴──────┴──────────┴────────┴────────────────┘

$ blameprompt search "JWT"            # full-text search across prompts
```

### Audit & Reporting

```bash
$ blameprompt audit                           # full audit trail
$ blameprompt audit --from 2026-01-01 --to 2026-02-24 --author "Jane"
$ blameprompt audit --format json             # md, table, json, csv
$ blameprompt analytics                       # aggregated stats + cost breakdown
$ blameprompt report --output report.md       # comprehensive markdown report
```

The audit trail shows every AI-assisted commit with author, date, model, cost, and prompt context:

```
$ blameprompt audit --format table

  ┌──────────┬────────────┬──────────────────────┬──────────────────┬────────┬───────────────────────┐
  │ Commit   │ Date       │ Author               │ Model            │ Cost   │ Prompt                │
  ├──────────┼────────────┼──────────────────────┼──────────────────┼────────┼───────────────────────┤
  │ a1b2c3d  │ 2026-02-24 │ Jane <jane@acme.com> │ Claude Sonnet 4.5│ $0.034 │ "Add JWT claims..."   │
  │ f4e5d6c  │ 2026-02-23 │ Bob <bob@acme.com>   │ Claude Opus 4.6  │ $0.128 │ "Refactor auth flow..." │
  └──────────┴────────────┴──────────────────────┴──────────────────┴────────┴───────────────────────┘
```

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

## Git hooks

Five hooks cover the full lifecycle:

| Hook | What it does |
|------|-------------|
| `pre-commit` | Reports staged receipt count |
| `post-commit` | Writes receipts as Git Notes |
| `post-checkout` | Auto-initializes in new clones |
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
| Config | `.blamepromptrc` |

## Privacy

Zero network calls (except git push/pull). Zero API keys. Zero telemetry. Zero accounts. Built-in redaction strips secrets before storage.

## Uninstall

```bash
blameprompt uninstall           # remove hooks, keep receipt history
blameprompt uninstall --purge   # remove everything
```

## License

[MIT](LICENSE)
