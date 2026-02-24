# blameprompt

**Every AI-written line, accounted for.**

An open-source CLI that hooks into [Claude Code](https://claude.ai/claude-code) and records exactly which lines AI wrote, which prompt triggered them, and when. Stored as [Git Notes](https://git-scm.com/docs/git-notes) — nothing added to your repo, nothing leaves your machine.

```
$ blameprompt blame src/auth.rs

  src/auth.rs

   1 │ use jsonwebtoken::{encode, Header};    human
   2 │ use serde::{Deserialize, Serialize};   human
   3 │
   4 │ #[derive(Serialize, Deserialize)]       AI  Claude  a1b2c3d
   5 │ pub struct Claims {                     AI  Claude  a1b2c3d
   6 │     pub sub: String,                    AI  Claude  a1b2c3d
   7 │     pub exp: usize,                     AI  Claude  a1b2c3d
   8 │ }                                       AI  Claude  a1b2c3d
   9 │
  10 │ pub fn validate(token: &str) -> bool {  human
  11 │     // manual validation logic           human

  5/11 lines (45.5%) written by AI
```

## Install

```bash
git clone https://github.com/ekaanth/blameprompt.git
cd blameprompt
cargo install --path .
```

First run auto-configures everything globally — Claude Code hooks, Git template, the works. Every `git init` and `git clone` after that is tracked automatically.

Requires [Rust](https://rustup.rs/) and Git.

## How it works

```
Claude Code ──▶ hooks capture ──▶ staging.json ──▶ git commit ──▶ Git Notes
                (file, lines,       (local)        (post-commit    (refs/notes/
                 prompt, hash)                       hook)          blameprompt)
```

1. **You code with Claude** — hooks fire on `PreToolUse`, `PostToolUse`, `SessionStart`, `SessionEnd`
2. **Edits are staged** — file paths, line ranges, prompt text, content hash
3. **Receipts attach on commit** — `post-commit` hook writes everything as a Git Note
4. **Query anytime** — `blame`, `audit`, `search`, `analytics`

Receipts survive rebases, merges, and cherry-picks via the `post-rewrite` hook.

## Commands

```bash
# Attribution
blameprompt blame <file>            # line-by-line AI vs human
blameprompt show <commit>           # receipts for a specific commit
blameprompt search <query>          # full-text search across prompts

# Reporting
blameprompt audit                   # full audit trail (md, json, csv, table)
blameprompt analytics               # aggregated stats
blameprompt report                  # comprehensive markdown report

# Security
blameprompt vuln-scan               # 10 CWE patterns on AI-generated code
blameprompt prompt-injection        # detect backdoors and hidden instructions
blameprompt secret-rotation         # flag secrets exposed to AI
blameprompt supply-chain-risk       # risk score 0-10
blameprompt license-scan            # model license compliance

# Sharing
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
