# dv

A terminal code-review tool for GitHub PRs, with Claude Code built in. Read the diff with
Helix keys and the comments beside the code; review, reply and merge without leaving the
terminal; have Claude review, answer you, suggest fixes, or rewrite a low-effort PR as a proper
one. A todo list ranks what needs you next.

- **Diff**: unified or side-by-side (`Ctrl-w v`), syntax highlighted, changed words emphasised,
  context expandable per hunk (`+` / `-`, `Z`). Helix's cursor, motions and selections, your
  Helix theme and settings, and rust-analyzer (`gd`, `gr`, `Space k`).
- **Drafts**: Claude drafts replacements for PRs (`A A`) or new PRs from what you ask (`N N`),
  unpublished until you've reviewed them in dv.
- **Files list** on the left (`Ctrl-w e`): the changed files, viewed and comment counts, a click
  to jump; the file you're in keeps its header pinned. Under it, the PR's commits: view one, or
  a range.
- **Comments pane** on the right (`Ctrl-w c`): review threads beside their lines, and a second
  tab (`gn`) with the PR's description, conversation and checks, scrolled on its own.
- **GitHub**: comment on lines, suggest changes, reply, resolve, mark files viewed, submit
  reviews; new comments show up as they come.
- **Claude Code**: reviews (one PR, or a batch in the background), answers your replies,
  makes or suggests the changes, drafts replacement PRs. It never posts or pushes: you decide
  what goes to GitHub.

## Install

```sh
cargo install dv-review   # installs `dv`
```

Or a binary for Linux or macOS from the [releases](https://github.com/evanlinjin/diffview/releases).

Needs `git`, and for PRs [`gh`](https://cli.github.com) (logged in). Optional: `rust-analyzer`,
`hx` (or `$VISUAL` / `$EDITOR`) for writing comments, [Claude Code](https://claude.com/claude-code)
(then `dv install-skill`, once), `python3` for merging. Inside [zellij](https://zellij.dev),
editors, Claude and the merge script get panes of their own beside dv.

## Usage

```sh
dv                # the working tree vs the merge-base with the PR's base (or origin/HEAD)
dv main           # ... vs main
dv 2258           # review PR 2258 (checks it out)
dv 2258 -w        # ... in a worktree next to the repo (../<repo>-2258); -wb on its branch
dv 2258 --rm      # remove that worktree
dv list -w        # the PRs: what needs you, open ones, worktrees, replacements
```

`?` in dv (and `dv list`) lists every key; [SPEC.md](SPEC.md) has the full key table and
design notes. The mouse works: scroll, click, drag to select.

## Reviewing a PR

1. **Read**: `]g` / `[g` hop between hunks, `]f` / `[f` between files, `Space f` picks a file,
   `]c` / `[c` step through comments, `Space v` marks a file viewed.
2. **Comment**: select lines (`x`, `v`), then `c` to comment or `s` to suggest a change; `r`
   replies on a thread. These go into your pending review (`Space p` lists them); `C` posts
   right away instead.
3. **Submit**: `S`, then comment, approve or request changes.

## With Claude

- **Review**: `Space a` → review. Claude's comments appear in the pane, tagged by how much they
  matter (`[bug]`, `[nit]`, …), and its summary sits at the top of the comments tab. In
  `dv list`, `m` marks PRs and `a a` has Claude review them in the background.
- **Talk to it**: `r` on Claude's comment replies, and Claude answers right away; `a` on
  selected lines leaves it a comment of your own. `Space r` resolves a thread, `Space d`
  dismisses one.
- **Keep what you agree with**: `c` copies Claude's comment into your pending review (edit it
  first, or keep it as is). Nothing goes to GitHub otherwise.
- **Changes**: `Space a` → make the changes (on someone else's PR: suggest them, as suggestion
  blocks to post). Then `Space g` shows just what Claude changed, commits and pushes.

Claude and dv share a review file in dv's data directory (`~/.local/share/dv/`), which dv
reloads as Claude writes it; comments follow their code as it changes, and show `[outdated]`
when it's gone.

Each task can run on its own model: `git config dv.model.<task> <model>` (`review`, `answer`,
`change`, `describe`, `fix-ci`, `close`, `commit`, `amend`, `rebase`, `replace`, `triage`,
`rank`, `interpret`, `group`), else `git config dv.model <model>`, else Claude Code's default.
For example, `git config --global dv.model.rank haiku`. (`close`: closing comments, PRs' and
issues'. Claude's pane, `Space a` → open, takes `dv.model`.) A PR's tasks share its Claude
session: one switching models reads that session again, uncached.

## Replacing a low-effort PR

For a PR not worth coaching into shape (often AI-generated): `A A` on it in `dv list` (or
several marked ones that attempt the same change). Claude writes the change properly on a
branch of its own (`dv/replace-N`, in `<repo>-replace-N`), drafts the new PR's description and
a short, civil note for closing the originals, and publishes nothing. The **replacements** tab
lists them; Enter opens one in dv to review like any branch (`a` on a draft asks for a redraft).
`S` there publishes it as a draft PR from your own remote, and closes the originals with the
note.

## What needs you

`dv list`'s **todo** tab lists the PRs and replacements waiting on you: approved and green,
pushed since your review, Claude answered you, your review requested, conflicts with its base
(`b b` asks for a rebase), no review from you yet, a replacement to publish, and so on. `r r` has Claude rank them, with a word on why and what next
(one Claude run, so only when you ask: the header says what the last ranking cost and what's
changed since). `a a` has Claude do its part on one (answer, update its review, review it);
`s` defers one you can't act on yet; Enter opens it.

**Merging**: `M M` on a PR in `dv list`, or `Space g` → merge in its review, runs Bitcoin Core's
[`github-merge.py`](vendor/github-merge/) (built into dv, pinned and checked; `git config
dv.mergescript <path>` uses your own copy) in the main checkout. The PR's worktree is removed
first, only if nothing in it is unpushed.

## Layout

| Path | |
|---|---|
| `src/main.rs` | arguments, checkouts and worktrees, the `review-*` commands |
| `src/diff.rs` | `git diff` and its parser |
| `src/gh.rs` | GitHub through `gh` (REST and GraphQL) |
| `src/claude.rs` | dv's data directory, review files, Claude sessions |
| `src/list/`, `src/batch.rs` | `dv list`, and Claude's runs in the background |
| `src/todo.rs` | what needs you, and Claude's ranking |
| `src/replace.rs` | replacement PRs |
| `src/merge.rs`, `vendor/github-merge/` | merging with `github-merge.py` |
| `src/view/` | the diff viewer (state, keys, motions, rendering, GitHub and Claude actions) |
| `src/lsp.rs`, `src/highlight.rs`, `src/md.rs`, `src/theme.rs` | rust-analyzer, syntax, markdown, Helix themes |
| `skills/dv-review/` | the Claude Code skill (`dv install-skill`) |

`cargo test` runs the tests.

## License

MIT or Apache-2.0, at your option ([LICENSE-MIT](LICENSE-MIT), [LICENSE-APACHE](LICENSE-APACHE)).
Releasing: [RELEASING.md](RELEASING.md); changes: [CHANGELOG.md](CHANGELOG.md).
