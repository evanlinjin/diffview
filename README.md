# dv

A terminal diff viewer for reading and reviewing pull requests, with Helix keys,
rust-analyzer, and GitHub review comments.

- **Diff**: unified or side-by-side (`Ctrl-w v`), with review comments in the right margin on
  wide windows (`Ctrl-w c` puts them inline), syntax highlighted, changed words
  emphasised, context expandable per hunk (`+`/`-`, `Z` for the whole file).
- **Helix-style**: the cursor/selection model, motions (`w b e`, `f t`, `gw`, `mm`, …), and
  your Helix theme, `cursorline`, `line-number`, and `jump-label-alphabet`.
- **rust-analyzer**: `gd` / `gr` / `gy` / `gi` and hover (`Space k`) from the diff.
- **GitHub**: the PR's description, conversation, checks and review threads inline, rendered
  like github.com. Comment on selected lines, suggest changes, reply, edit, delete, resolve,
  mark files viewed, and submit reviews.

## Install

```sh
cargo install dv-review   # installs the `dv` binary
```

Or download a binary for Linux or macOS from the
[releases](https://github.com/evanlinjin/diffview/releases), or build from a checkout with
`cargo install --path .`.

Needs `git`. For PRs: [`gh`](https://cli.github.com), logged in (`gh auth login`).
Optional: `rust-analyzer` (go to definition, references, hover), `hx` (opening files and
writing comments; `$VISUAL`/`$EDITOR` is used for comments if set), `xdg-open`/`open`
(opening links). Inside [zellij](https://zellij.dev), hx, editors and Claude Code open in a pane to
the right, and dv keeps working while hx is open.

## Usage

```sh
dv                # working tree vs the merge-base with the PR's base (or origin/HEAD)
dv main           # ... vs the merge-base with main
dv 2258           # review PR 2258 (gh pr checkout 2258 first)
dv 2258 -f        # same, resetting your local copy of the PR branch to the PR head
dv 2258 -w        # review it in a worktree next to the repo: ../<repo>-2258 (no branch)
dv 2258 -wb       # same, on the PR's branch, so you can push to the PR
dv 2258 --rm      # remove that worktree (-f to discard local changes)
dv list -w        # pick from the open PRs; Enter reviews one (here: in a worktree), q comes back;
                  # Tab lists the PRs you have worktrees for
```

On a branch with an open PR, plain `dv` loads it too.

The mouse works too: scroll, click to move, drag to select (`dv --no-mouse` leaves it to your
terminal; most select text with Shift held either way). Press `?` in dv for every key (`/` searches the list). [SPEC.md](SPEC.md) has the full key
table and design notes.

## Reviewing a PR

1. `dv 2258` (or `dv 2258 -w` to keep your checkout as it is).
2. Read: `]g`/`[g` hop between hunks, `]f`/`[f` between files, `Space f` picks a file,
   `Space t` lists the threads. `Space v` marks a file viewed and moves on.
3. Comment: select lines (`x`, `v`), then `c` to comment or `s` to suggest a change. On a
   thread, `r` replies (quoting the comment under the cursor). These go into your pending
   review; `Space p` lists them.
4. Submit: `S`, then Comment / Approve / Request changes.

`C` posts a comment immediately instead of adding it to the review. `Space e` / `Space d`
edit / delete a comment, `Space r` resolves a thread, `gf` opens a link, `R` refreshes.

## Reviewing with Claude Code

[Claude Code](https://claude.com/claude-code) writes a review, and dv shows it inline next to
the lines it's about. You reply to Claude from dv, Claude answers (or changes the code), and
you copy the comments you agree with into your GitHub review. The two talk through a review
file that dv reloads whenever it changes, so you can keep dv open while Claude works.

**Setup** (once): `dv install-skill` installs the `dv-review` skill to
`~/.claude/skills/dv-review/`. Restart any running Claude Code session so it picks it up.

1. **Get the PR**: `dv 2258 -w` (or `dv list -w`, then Enter).
2. **Ask for a review**: press `Space a` in dv and pick "Ask Claude to review this". `$EDITOR`
   opens for anything it should focus on; leave it empty to just go. (Or, in Claude Code in
   that checkout, ask it to "review this PR for dv".) Inside zellij, Claude
   Code opens in a pane on the right, where you can watch it and approve what it asks to do;
   if that PR's Claude pane is already open, dv asks there instead of opening another;
   elsewhere it runs in the background, allowed to edit files and run only the `dv review-*`
   and read-only `git` commands it needs, and the status bar says when it's done. It runs
   `dv review-info` (the review file's path, and the base commit: `git diff <base>` is the
   diff dv shows), writes its review, and runs `dv review-check`, which checks every line
   reference against the diff and records the commented code.
3. **Read it in dv**: Claude's summary sits in the top section; its comments are threads by
   **claude** under their lines (`]c` / `[c` to step through them, `Space A` lists them).
4. **Act on each comment**:

   | Key | |
   |---|---|
   | `c` | copy it into your pending GitHub review (edit it in `$EDITOR`, or keep it as is) |
   | `C` | same, but post it now |
   | `r` | reply to Claude: it answers right away, in the background (nothing goes to GitHub) |
   | `Space r` | resolve it, so Claude leaves it alone |
   | `Space d` | dismiss it |

   To ask Claude something yourself, select lines and press `a`: your comment goes into the
   review file and Claude answers it like a reply.

   A comment copied to GitHub is removed from the review file, so it doesn't show twice.
5. **Claude answers**: as soon as you reply, Claude Code answers in the background, resuming
   the same session each time so it keeps the review in mind; its answer shows up under your
   reply ("Claude is answering…" meanwhile). Replies you write while it's busy are answered
   in one more run. (`Space a` → "answer your replies", or asking Claude Code to "answer my dv
   replies", does the same by hand.) For each unresolved
   comment where you spoke last, it replies, or makes the change you asked for, says what it
   changed and resolves the thread. Its answers appear in dv as it writes them. Repeat 4–5 as
   needed.
6. **Have Claude make the changes** (optional): `Space a`, "Ask Claude to make the changes". You
   can add your own instructions in `$EDITOR` first (leave it empty to just go; with no open
   comments, they're the request: "Ask Claude to make a change"). Claude makes the change each
   open comment asks for, its own and the ones you left it (not the resolved, dismissed or
   outdated ones),
   replying on each with what it changed and resolving it. Resolved threads fold: unfold one
   (`Tab`) to check the change, and `Space r` reopens it if it isn't right.
7. **Submit**: `S` submits your pending review, holding whatever you queued with `c`. Claude
   never posts to GitHub; only you do, from dv.

**When the code changes** (Claude's edits, yours in `hx`, a new push), Claude's comments follow
their code: one whose lines moved moves with them; one whose code changed or is gone shows
`[outdated]`, folded and dimmed near its old line, and can't be copied to GitHub. Claude
re-anchors outdated comments that still apply when it next runs `dv review-check`.

**Good to know**

- The review file is `~/.local/share/dv/<repo>/reviews/pr-<N>.json` (`branch-<name>.json`
  without a PR; `$XDG_DATA_HOME` if set): outside the repo, so never committed, and shared by
  its worktrees. (Not in `.git`: Claude Code won't edit files there.) A new review replaces it (the skill keeps
  comments that still apply, with your replies).
- It works on a local branch before there's a PR, but `c` / `C` need one.
- Like dv's own comments, `c` / `C` need the checkout at the PR head with no local changes.
  While Claude's changes are uncommitted you can reply and resolve, but copy comments to
  GitHub only once they're pushed.

## Layout

| File | |
|---|---|
| `src/main.rs` | arguments, PR checkout, worktrees |
| `src/diff.rs` | `git diff` parsing, hunks, word diffs |
| `src/list.rs` | `dv list`: the open-PR picker |
| `src/claude.rs` | Claude review files: format, checks, replies |
| `skills/dv-review/` | the Claude Code skill (`dv install-skill`) |
| `src/gh.rs` | GitHub via `gh`: PR info, threads, review mutations |
| `src/lsp.rs` | a minimal rust-analyzer client |
| `src/highlight.rs` | syntect highlighting mapped to Helix scopes |
| `src/md.rs` | GitHub-flavoured markdown rendering |
| `src/theme.rs` | Helix theme and settings |
| `src/view/` | the TUI: state (`mod.rs`), keys, motions, rendering, review actions, rust-analyzer |

`cargo test` runs the tests.

## License

MIT or Apache-2.0, at your option ([LICENSE-MIT](LICENSE-MIT), [LICENSE-APACHE](LICENSE-APACHE)).
Releasing: [RELEASING.md](RELEASING.md); changes: [CHANGELOG.md](CHANGELOG.md).

Throwaway line.
