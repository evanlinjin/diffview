# Changelog

All notable changes to dv. Versions follow [semver](https://semver.org); a release's notes
on GitHub are its section here.

## [Unreleased]

### Changed
- Replacements:
  - `A A` on several marked PRs has Claude suggest which go together (one replacement each
    group) and which stand alone; you edit that in your editor before anything's drafted.
  - Publishing pushes to your own remote, not origin: dv asks which the first time and keeps
    it in `git config dv.replaceRemote`. It rebases onto the base on origin first.
  - `S` can publish and then close the originals in one go (closing on its own stays too).
  - Uncommitted changes aren't published: `S` has you commit them first.
  - A closing note without `{replacement}` gets "Superseded by #N." added.
  - Drafts get your repo notes in their prompt, read-only, and can only write their record:
    a PR's text can't steer what Claude does in later reviews.
  - A draft under way is known everywhere (`dv list`, dv, `dv replace`), so none starts a
    second one; dv and `dv list` show drafts as they change.
  - PRs into different base branches can't share a replacement.

### Fixed
- `dv list`: a review (or replacement) that failed says why on its card, not just "failed",
  and that `a a` (`A A`) tries again.

## [0.4.1] - 2026-10-17

### Added
- `gf` on a `file:line` in a comment (`src/a.rs:12`, `a.rs:12-15`, `#L12`) goes there: in the
  diff if it's shown, else in hx (`Ctrl-o` comes back). The skill has Claude cite code as
  `path:line` from the repo root, so it always resolves.

### Fixed
- A comment on a range of lines (5-10) lights all of them beside its box in the comment
  margin, not just its last line; the cursor on any of them lights the box.

## [0.4.0] - 2026-10-16

### Added
- Replacements: Claude drafts the change a PR (or several attempts at the same thing) should
  have been, unpublished, for you to review first. `A A` in `dv list`, `Space a` in dv, or
  `dv replace N`; Claude works in `<repo>-replace-N` on `dv/replace-N`, commits (never pushes),
  and drafts the new PR's title and description and a note for closing the originals. `dv
  list`'s new replacements tab lists them, and Enter opens one in dv with what it replaces and
  the drafts on top. `S` there publishes it as a draft PR, then closes the originals with the
  note (each step after your editor).
- `S` → "Comment and close the PR": your comment as a review (pending comments too), then the
  PR closed; with nothing to say, it only closes.
- `dv list`: `[new]` on PRs with something you haven't seen: GitHub activity or a change to
  Claude's review since you last left the PR in dv (and, of PRs you've never opened, those
  Claude has reviewed).
- Your last instructions for Claude's reviews (`Space a` → review, `a a` in `dv list`) are
  remembered per repo and fill the editor next time; clear them to forget them.
- `dv list`: `a a` opens your editor first, for instructions to Claude for every marked PR's
  review (leave it empty for a plain review; `:cq` cancels).
- `Ctrl-w a`: the Claude view, just Claude's review: the PR's one-line summary, Claude's
  summary, and only the code it commented on (a few lines around each comment).

### Changed
- Minimum Rust version 1.89 (was 1.88), for current dependencies (uuid 1.28).
- `dv list`: Claude's part of a card (`claude ✓`, `claude: reviewing…`) sits at the right end
  of its second line, under the age; a long line is cut to keep it in view.
- The PR's state (DRAFT, OPEN, MERGED, CLOSED) sits at the right end of the PR's header line,
  where it stands out; a long title is cut to keep it in view.
- `[new]` also goes once the cursor has stayed on the thread for 3 seconds, not only once it
  leaves.

### Fixed
- dv no longer replaces an installed skill that a newer dv installed (with an older dv
  installed and a newer one built, they took turns). The skill notes the dv that installed it.
- A closed (or merged) draft PR shows as CLOSED (MERGED), not DRAFT.
- Claude's summary is Claude's, not PR text: `c` copies it into your review summary (`S`
  starts from it), `C` posts it as your own PR comment (not quoted), and its key popup says so.
- `?`: every section heading in the heading color (in two columns, the right column's had none
  and the left's colored its whole row).
- `dv list`: the selected PR is readable (its text takes the highlight's colors), and the
  highlight runs to the edge on both of its lines.

## [0.3.0] - 2026-10-16

### Added
- `[new]` on threads with comments you haven't seen (Claude's, or anyone's on GitHub; not
  your own), until the cursor has been on them; `]n` / `[n` step through those and `Space n`
  lists them. dv remembers what you've seen per PR (or branch).
- `Space g`: show only what isn't committed (to check Claude's changes), commit it (the
  message starts from Claude's "Changed: …" replies since the last commit), and push to the
  branch it tracks (a PR's, under its own name). The review loop no longer leaves dv.
- dv keeps an installed dv-review skill up to date with itself, and a PR's Claude session
  that started with another version of the skill isn't resumed (Claude Code reads a skill
  once per session). `Space a` can start a fresh session for the PR.
- `S` lists what's still open under its choices: Claude's open comments (and how many it
  answered, waiting for you), unresolved GitHub threads, pending comments on code that has
  changed since, a checkout that isn't the PR's head. Approving puts them in the editor too, as
  hint lines (not posted).
- `Space m` lists the messages so far, in full (the status bar cuts long ones), and where the
  last background Claude run's log is.
- Comments on files that are no longer in the diff (a change reverted, a file back to the
  base) are listed in the PR section, by file, instead of showing nowhere; `]c` / `]a` and
  the pickers get there.
- `c` / `C` / `s` / `Space v` on an untracked file say why they can't (GitHub hasn't got it)
  instead of failing there, and "viewed" counts only the PR's files.
- A background Claude run says what it took when it's done ("5 turns, $0.22"); `Space m`
  has the total so far.
- Batch reviews from `dv list`: `m` marks PRs, `a a` has Claude review them in the
  background, two at a time, each in its worktree (`<repo>-N`, made if need be) and its own
  session, so opening one later carries on from there. Cards show how each is going, and
  `claude ✓` once a PR has a review.
- Claude's comments show whose turn it is: `[waiting]` (you spoke last) and `[answered]`
  (Claude answered you); typing either in `Space A` lists just those.
- A `severity` for Claude's comments (`bug`, `nit`, `question`, …), shown as a tag and put
  before the text when you copy the comment to GitHub.
- "Update Claude's review" after new commits on top of the one it reviewed asks it to review
  just those (`git diff <its head>`).
- On someone else's PR, "make the changes" becomes "suggest the changes": GitHub suggestion
  blocks in Claude's comments for you to post, and no edits to the code.
- Untracked files show in the diff (marked untracked), and changing one reloads it.
- `dv list`: `D D` in the worktrees tab removes the PR's worktree (not with local changes in
  it; its branch stays); up to 1000 PRs (was 200), and Enter on a `#N` filter opens PR N even
  if it isn't listed.

### Fixed
- With dv in a floating zellij pane, the editor (and hx, and Claude's pane) opened nowhere:
  zellij can't split a floating pane, and said nothing. dv then took the comment as cancelled.
  They now open where zellij puts them.
- The key popup moves to the bottom left while the cursor is in the comment margin, so it no
  longer covers the comments there.
- On someone else's PR, Claude's answers to your replies suggest the changes you ask for
  instead of making them, as "suggest the changes" does.
- dv doesn't paste a request into a Claude pane that's asking whether you trust the folder
  (the first time Claude Code runs in one), as it already didn't for permission prompts.
- `C` beside a pending review says so up front: GitHub refuses single comments then.
- A refused review submission (e.g. approving your own PR) no longer leaves an empty pending
  review behind.
- GitHub's reasons for refusing something show in full, before where your text was saved.
- Claude finds `dv` (this one) when dv was started by its path or the Claude pane gets a
  different PATH (a zellij pane gets zellij's environment), so it can run `dv review-*`.
- Claude's reply times are right: the prompt tells it the time.
- dv no longer rewrites the review file to give comments ids while Claude may be editing it.

## [0.2.0] - 2026-10-16

### Added
- dv reloads the diff when the working tree changes under it (Claude making changes, a save in
  hx, anything else), about once a second, keeping the cursor where it is; and right after a
  background Claude run.
- Mouse support: the wheel scrolls (the view, a popup, a picker's selection), a click moves
  the cursor there (code, comment margin, either split side) or folds on a fold arrow, a drag
  selects; in pickers and `dv list` a click picks and a second click opens. `--no-mouse` leaves
  the mouse to the terminal (most terminals select text with Shift held either way).
- `Space u` updates the checkout: to the PR's head (fast-forward only, keeping local commits),
  or `git pull --ff-only` without a PR, then refreshes.
- Comments in the right margin: in single-pane view on a wide window (room for 80 columns of
  code beside the margin: a third of the window, 60–80 columns, or all beyond 200 columns of
  code side; thread tags on a line of their
  own under the header), review threads sit beside their lines instead of between
  them. A box that would overlap the one before goes just below it; code only gets blank
  rows where a file's comments need more room than its code. `Ctrl-w l` / `Ctrl-w h` move
  between code and comments; the box about the cursor's line (or the line a box is about) is
  lit. `Ctrl-w c` switches to inline comments and back.
- In the same layout, the PR's conversation (and Claude's summary) sits beside the
  description: whole comments, while there's description beside them; the rest follows full
  width below.
- One Claude Code session per PR, kept next to its review file: every run (background or in a
  pane, from any dv and any worktree) continues the same conversation about that PR.
- Claude's notes on the repo (`notes.md` in dv's directory, shared by worktrees): the skill reads them
  before each task and keeps them up to date (conventions, subtle code, your review
  preferences), so what it learns carries over to the next PR. `Space a` opens them in hx.
- Approving a PR (`S`, Approve) starts the summary as `ACK <the PR's head commit>`.
- `a` on code lines leaves Claude a comment there (a question or a request), in the review file;
  Claude answers right away, and `r` continues the thread. Nothing goes to GitHub.
- "Ask Claude to make the changes" acts on the review's open comments, Claude's and the ones
  you left it, and works with none too: then what you write is the request ("Ask Claude to
  make a change"). It's checked whether Claude can take it before the editor opens, and if
  it still can't, what you wrote is saved to a file.
- Background Claude runs may run `cargo check`, `cargo build`, `cargo test` and `cargo clippy`,
  to check a change they made.
- `Space a` → "Dismiss Claude's outdated comments": all of Claude's comments whose code has
  changed, after a confirmation (your own comments for Claude stay).
- `]p` / `[p` step through your pending review's comments (replies too), wrapping around.
- `Ctrl-d` in the pending comments picker (`Space p`) deletes the selected comment (after a
  confirmation); the review thread key popup lists `Space d`.

### Changed
- dv's files for a repo (Claude's reviews, sessions, notes) move from `.git/dv` to
  `~/.local/share/dv/<repo>/` (or `$XDG_DATA_HOME`), on first use: Claude Code refuses to edit
  anything inside `.git`, so background Claude runs couldn't write the review there. Claude
  runs get access to that directory (`--add-dir`), from worktrees too.
- When a Claude run is refused something, dv says what (in the status bar, and keeps one log
  per run) instead of "done"; replies made meanwhile wait rather than hit the same refusal.
- Approving pre-fills `ACK <PR head>` even when your checkout has diverged from it, with a hint
  above it (not posted) that it isn't what you have checked out.
- Inside zellij, `Enter` reuses dv's hx pane while it's open (`:open` + `:goto` there) instead of
  opening another, and dv reloads as you save in it, not only when hx exits.
- Claude resolves a thread once it's settled (it made the change asked for, or you agree it
  doesn't apply), after replying; answering a question leaves it open. Reopen with `Space r`.
- Inside zellij, a PR's Claude Code pane is reused: when it's open, `Space a` puts the request
  in its input (pasted, focused; you press Enter) instead of opening another pane, and leaves
  it alone while it's asking you something. Replies are answered in the background on a fork
  of its session, so nothing is typed into a pane behind your back.
- Claude Code sessions are kept per checkout directory (as Claude Code stores them), and one it
  doesn't have isn't resumed.
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

[0.4.1]: https://github.com/evanlinjin/diffview/releases/tag/v0.4.1
[0.4.0]: https://github.com/evanlinjin/diffview/releases/tag/v0.4.0
[0.3.0]: https://github.com/evanlinjin/diffview/releases/tag/v0.3.0
[0.2.0]: https://github.com/evanlinjin/diffview/releases/tag/v0.2.0
[0.1.0]: https://github.com/evanlinjin/diffview/releases/tag/v0.1.0
