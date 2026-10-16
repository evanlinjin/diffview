# dv

A terminal diff viewer for reading and reviewing pull requests, with Helix keys,
rust-analyzer, and GitHub review comments.

- **Diff**: unified or side-by-side (`Ctrl-w v`), syntax highlighted, changed words
  emphasised, context expandable per hunk (`+`/`-`, `Z` for the whole file).
- **Helix-style**: the cursor/selection model, motions (`w b e`, `f t`, `gw`, `mm`, …), and
  your Helix theme, `cursorline`, `line-number`, and `jump-label-alphabet`.
- **rust-analyzer**: `gd` / `gr` / `gy` / `gi` and hover (`Space k`) from the diff.
- **GitHub**: the PR's description, conversation, checks and review threads inline, rendered
  like github.com. Comment on selected lines, suggest changes, reply, edit, delete, resolve,
  mark files viewed, and submit reviews.

## Install

```sh
cargo install --path .
```

Needs `git`. For PRs: [`gh`](https://cli.github.com), logged in (`gh auth login`).
Optional: `rust-analyzer` (go to definition, references, hover), `hx` (opening files and
writing comments; `$VISUAL`/`$EDITOR` is used for comments if set), `xdg-open`/`open`
(opening links). Inside [zellij](https://zellij.dev), editors open in floating panes.

## Usage

```sh
dv                # working tree vs the merge-base with the PR's base (or origin/HEAD)
dv main           # ... vs the merge-base with main
dv 2258           # review PR 2258 (gh pr checkout 2258 first)
dv 2258 -f        # same, resetting your local copy of the PR branch to the PR head
dv 2258 -w        # review it in a worktree next to the repo: ../<repo>-2258 (no branch)
dv 2258 -wb       # same, on the PR's branch, so you can push to the PR
dv 2258 --rm      # remove that worktree (-f to discard local changes)
```

On a branch with an open PR, plain `dv` loads it too.

Press `?` in dv for every key (`/` searches the list). [SPEC.md](SPEC.md) has the full key
table and design notes.

## Reviewing a PR

1. `dv 2258` (or `dv 2258 -w` to keep your checkout as it is).
2. Read: `n`/`N` hop between hunks, `]f`/`[f` between files, `Space f` picks a file,
   `Space t` lists the threads. `Space v` marks a file viewed and moves on.
3. Comment: select lines (`x`, `v`), then `c` to comment or `s` to suggest a change. On a
   thread, `c` replies (quoting the comment under the cursor). These go into your pending
   review; `Space p` lists them.
4. Submit: `S`, then Comment / Approve / Request changes.

`C` posts a comment immediately instead of adding it to the review. `Space e` / `Space d`
edit / delete a comment, `Space r` resolves a thread, `gf` opens a link, `R` refreshes.

## Layout

| File | |
|---|---|
| `src/main.rs` | arguments, PR checkout, worktrees |
| `src/diff.rs` | `git diff` parsing, hunks, word diffs |
| `src/gh.rs` | GitHub via `gh`: PR info, threads, review mutations |
| `src/lsp.rs` | a minimal rust-analyzer client |
| `src/highlight.rs` | syntect highlighting mapped to Helix scopes |
| `src/md.rs` | GitHub-flavoured markdown rendering |
| `src/theme.rs` | Helix theme and settings |
| `src/view/` | the TUI: state (`mod.rs`), keys, motions, rendering, review actions, rust-analyzer |

`cargo test` runs the tests.
