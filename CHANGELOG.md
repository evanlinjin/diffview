# Changelog

All notable changes to dv. Versions follow [semver](https://semver.org); a release's notes
on GitHub are its section here.

## [Unreleased]

### Added
- One Claude Code session per PR, kept next to its review file: every run (background or in a
  pane, from any dv and any worktree) continues the same conversation about that PR.
- Claude's notes on the repo (`.git/dv/notes.md`, shared by worktrees): the skill reads them
  before each task and keeps them up to date (conventions, subtle code, your review
  preferences), so what it learns carries over to the next PR. `Space a` opens them in hx.
- Approving a PR (`S`, Approve) starts the summary as `ACK <the PR's head commit>`.
- `a` on code lines leaves Claude a comment there (a question or a request), in the review file;
  Claude answers right away, and `r` continues the thread. Nothing goes to GitHub.
- `Space a` → "Dismiss Claude's outdated comments": all of Claude's comments whose code has
  changed, after a confirmation (your own comments for Claude stay).
- `]p` / `[p` step through your pending review's comments (replies too), wrapping around.
- `Ctrl-d` in the pending comments picker (`Space p`) deletes the selected comment (after a
  confirmation); the review thread key popup lists `Space d`.

### Changed
- Inside zellij, a PR's Claude Code pane is reused: when it's open, `Space a` puts the request
  in its input (pasted, focused; you press Enter) instead of opening another pane, and leaves
  it alone while it's asking you something. Replies are answered in the background on a fork
  of its session, so nothing is typed into a pane behind your back.
- Claude Code sessions are kept per checkout directory (as Claude Code stores them), and one it
  doesn't have isn't resumed.

### Changed
- Inside zellij, hx, the comment editor and Claude Code open in a pane to the right instead of
  a floating one.

### Fixed
- dv no longer freezes while hx is open beside it: hx runs in its own pane and dv reloads when
  it exits.
- `dv N` for a PR that has a `<repo>-N` worktree opens it there (its branch couldn't be checked
  out in the main checkout too).
- A PR branch with local commits that aren't on the PR (unpushed work, a local rebase) opens
  as it is, with a note, instead of failing to fast-forward; `-f` still resets it to the PR.

## [0.1.0] - 2026-10-16

The first release.

### Diff
- base → working tree diff, unified or side by side (`Ctrl-w v`), syntax highlighted, changed
  words emphasised, context expandable per hunk (`+` / `-`, `Z` for the whole file).
- Helix-style cursor and selections, motions (`w b e`, `f t`, `gw`, `mm`, …), search, jump
  list; your Helix theme, `cursorline`, `line-number` and `jump-label-alphabet`.
- rust-analyzer: go to definition / references / type / implementations, hover.
- Open `hx` at the cursor (in a floating pane inside zellij); copy text, patches, `path:line`
  refs and comment links.

### GitHub
- `dv N` checks out PR N (`-w` in a worktree next to the repo, `-b` on its branch, `--rm` to
  remove it). `dv list`: the open PRs as cards with CI, review state, size and labels, sorted
  and filtered GitHub-search style, plus a tab of the PRs you have worktrees for.
- The PR's description, conversation, checks and review threads inline, rendered like
  github.com, in boxes; outdated threads relocated to their code.
- Comment on selected lines, suggest changes, reply, edit, delete, resolve, quote into the
  conversation; a pending review you submit with `S`; mark files viewed.
- Issue / PR link previews (`Space k`).

### Claude Code
- `dv install-skill` installs the `dv-review` skill: Claude Code writes a review that dv shows
  inline (`dv review-info`, `dv review-check`), anchored to its code as the code changes.
- Reply to Claude from dv and it answers right away; ask it to review or to make the changes
  its open comments ask for (`Space a`, with your own instructions if you like); copy its
  comments into your GitHub review.

### Keys
- Which-key popups after prefix keys, and the keys for what's under the cursor in the same
  corner; `?` lists every key (two columns on wide windows, `/` to search).

[0.1.0]: https://github.com/evanlinjin/diffview/releases/tag/v0.1.0
