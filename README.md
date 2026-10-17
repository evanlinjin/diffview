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
  like github.com, with new comments showing up as they come. Comment on selected lines,
  suggest changes, reply, edit, delete, resolve, mark files viewed, and submit reviews.

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
                  # Tab lists the PRs you have worktrees for (D D removes one);
                  # m marks PRs and a a has Claude review them in the background
                  # (your editor opens first, for instructions to all of them; your last
                  # review instructions are there to start from);
                  # [new]: something on GitHub or from Claude since you last left it;
                  # A A has Claude draft a replacement (below); filter by #N to open a PR
                  # that isn't listed
dv replace 2258   # Claude drafts a replacement for PR 2258, unpublished (below)
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
4. Submit: `S`, then Comment / Approve / Request changes, or Comment and close the PR.

`C` posts a comment immediately instead of adding it to the review. `Space e` / `Space d`
edit / delete a comment, `Space r` resolves a thread, `gf` opens a link (or goes to a `file:line` or function a comment cites), `R` refreshes.

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
   **claude** under their lines (`]c` / `[c` to step through them, `Space A` lists them),
   tagged with how much each matters (`[bug]`, `[nit]`, …). `[waiting]` marks a thread where
   you spoke last (Claude's turn), `[answered]` one where Claude has answered you (yours);
   type either in `Space A` to list just those. `[new]` marks a thread with comments you
   haven't seen yet (Claude's or anyone's on GitHub; seen once your cursor has been on it for 3 seconds, or has moved on):
   `]n` / `[n` step through those, `Space n` lists them. `Ctrl-w a` switches to the
   Claude view: just the PR's one-line summary, Claude's summary, and its comments with a few
   lines of code around each (and back). `Ctrl-w p` does the same for your pending review: its
   text and the comments in it, before you submit.
4. **Act on each comment**:

   | Key | |
   |---|---|
   | `c` | copy it into your pending GitHub review (edit it in `$EDITOR`, or keep it as is) |
   | `C` | same, but post it now |
   | `r` | reply to Claude: it answers right away, in the background (nothing goes to GitHub) |
   | `Space r` | resolve it, so Claude leaves it alone |
   | `Space d` | dismiss it |

   On Claude's summary, `c` puts it in your review summary (`S` starts from it), and `C`
   posts it as your own PR comment (after the editor).

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
   (`Tab`) to check the change, and `Space r` reopens it if it isn't right. On someone else's
   PR this is "suggest the changes": Claude doesn't touch the code, it puts GitHub suggestion
   blocks in its comments for you to post with `c`.

   Then `Space g`: "Show only what isn't committed" to check just Claude's changes, "Commit"
   (the message starts from what Claude said it changed), and "Push" to the PR's branch.
   After a push, "Update Claude's review" looks at just what's new.
7. **Submit**: `S` submits your pending review, holding whatever you queued with `c`. In the
   PR section, `c` on a conversation comment (or the description) quotes it in the review's
   summary, which `S` starts from. Claude
   never posts to GitHub; only you do, from dv.

**When the code changes** (Claude's edits, yours in `hx`, a new push), Claude's comments follow
their code: one whose lines moved moves with them; one whose code changed or is gone shows
`[outdated]`, folded and dimmed near its old line, and can't be copied to GitHub. Claude
re-anchors outdated comments that still apply when it next runs `dv review-check`. After new
commits on top of the one Claude reviewed, "Update Claude's review" asks it to review just
those. New files show in the diff before they're committed (marked untracked).

**Good to know**

- The review file is `~/.local/share/dv/<repo>/reviews/pr-<N>.json` (`branch-<name>.json`
  without a PR; `$XDG_DATA_HOME` if set): outside the repo, so never committed, and shared by
  its worktrees. (Not in `.git`: Claude Code won't edit files there.) A new review replaces it (the skill keeps
  comments that still apply, with your replies).
- It works on a local branch before there's a PR, but `c` / `C` need one.
- Like dv's own comments, `c` / `C` need the checkout at the PR head with no local changes.
  While Claude's changes are uncommitted you can reply and resolve, but copy comments to
  GitHub only once they're pushed.

### Replacing a PR

Some PRs aren't worth reviewing into shape (low-effort, often AI-generated ones). Have Claude
write the change properly instead, as a **replacement** you review before anything is
published:

1. In `dv list`, `A A` on the PR, or `Space a` → "Draft a replacement" in dv. With several
   marked PRs, Claude first suggests which belong together (attempts at the same change: one
   replacement covers them) and which stand alone; your editor shows that, a line per
   replacement, to change as you like. Then your editor opens for instructions (your last ones
   are there to start from).
2. Claude works in the background, in a worktree of its own, `<repo>-replace-N` on branch
   `dv/replace-N`, from the originals' base: it reads them (and the issue they claim to fix),
   works out what they got wrong, writes the change properly with tests, and commits it
   (never pushing). Your notes on the repo come in its prompt, read-only: it reads untrusted
   PRs, so it can't touch dv's other files. It drafts the new PR's title and description ("Replaces #N", with any
   "Fixes #X" carried over) and a short, civil note for closing the originals.
3. `dv list`'s **replacements** tab lists them (cards say what they replace; a replaced PR's
   card says `replacement ✓`). Enter opens one in dv: its diff against the base, and above it
   what it replaces and Claude's drafts. Everything else works as on a branch: comments for
   Claude, `Space a` → "make the changes", or "Redraft this replacement" with new
   instructions (Claude carries on in the same session). `D D` removes one (its branch stays).

   It updates by itself as Claude redrafts.
4. When it's right, `S` there: **Publish as a draft PR** (your editor has the title, first line,
   and description to adjust), or **Publish, then close** the originals too. dv rebases it onto
   its base as it is on origin now (stopping if that conflicts), pushes `dv/replace-N` to your
   own remote (not origin: the first time, dv asks which, and keeps it in `git config
   dv.replaceRemote`), and opens the draft PR from there. What isn't committed isn't
   published, so dv has you commit it first. **Close #N with the closing note** (your editor
   has Claude's note, with the new PR's number in it, or "Superseded by #N." added if it left
   that out) comments on each original, then closes it; it's also on `S` later. Several PRs
   with different base branches can't share a replacement.

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
