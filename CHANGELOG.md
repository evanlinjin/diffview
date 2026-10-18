# Changelog

All notable changes to dv. Versions follow [semver](https://semver.org); a release's notes
on GitHub are its section here.

## [Unreleased]

### Added
- `dv list` shows Claude at work on a PR from any dv (a review, answers, making the changes,
  started from the PR's view too): on its card, and on the todo tab it goes to the bottom, out of
  the count, until Claude's done or has failed, as there's nothing for you till then.

## [0.8.1] - 2026-10-18

### Added
- `]u` / `[u`: the next / previous unresolved comment thread (wrapping around).
- `b b` in `dv list` asks a PR's author for a rebase: a comment (in your editor first), then the
  item is deferred on the todo tab until it changes. The todo tab lists PRs that conflict with
  their base.

### Changed
- Comment marks are at the code's right edge, beside the pane, shown or not: `╎` down the lines a
  comment is about and a dot at its last (`●`, `◆` Claude's, `○` resolved or outdated), so a
  comment on several lines shows them all. In split view, on its side.

## [0.8.0] - 2026-10-18

### Added
- `Ctrl-w n` turns relative line numbers on or off, and dv keeps that for next time, over
  Helix's `line-number` setting.
- `s` on the todo tab defers an item (then `1` an hour, `4` four hours, `d` a day, `w` a week):
  it goes to the bottom, out of the count, until then, or until what it needs changes. `s` on it
  brings it back. For what you can't act on yet (a merge you lack permission for, say).

### Changed
- The todo tab spends nothing without asking: Claude ranks only on `r r` (`R` refreshes from
  GitHub alone; opening the tab doesn't rank), and starts nothing after a ranking. To judge whether
  a ranking is worth it, the header and the first `r` say how old the last one is, what it cost,
  and what's changed since (new, changed, seen to); cards placed since say so. A card says what
  Claude could do there (answer you, update its review, review it), and `a a` does that.

## [0.7.2] - 2026-10-18

### Changed
- `dv list` cards in a wide window (110+ columns) have two zones: what the PR is on the left
  (title, who and how big, branch), how it stands on the right (CI and its age, review, Claude), a
  column that lines up from card to card, within 140 columns; three lines each. Narrower, the
  four stacked lines as before.

### Fixed
- Reviews show in the PR's conversation (in time order with its comments): their text, and
  approvals and change requests even without text. A plain review's text was nowhere before.
- `gg` / `ge` in the comments pane go to its own top / end (the first / last comment, or the PR
  tab's), not the diff's.

## [0.7.1] - 2026-10-18

### Changed
- `dv list` cards are easier to read: four lines and a gap between cards, related things
  together (title; who and how big; CI, review and Claude, in columns; branch and worktree),
  fewer separators. The selected card keeps its colors, on a quiet background (the theme's cursor
  line) with a bar down its left edge. On the todo tab, the reason is under the title.

### Fixed
- The mouse wheel scrolls what's under it, wherever the cursor is: the code, with the cursor in
  the PR tab's text (it didn't move), or the PR tab. At the end of one, beside the PR tab, it
  scrolls the other (the PR text stops at the bottom of the pane, not scrolling up past it).

## [0.7.0] - 2026-10-18

### Added
- Merging with Bitcoin Core's `github-merge.py`: `M M` on a PR in `dv list`, or `Space g` →
  merge in its review. The script is built into dv (pinned to an upstream commit, its SHA-256
  checked by a test; `vendor/github-merge/`), so nothing is downloaded to merge; `git config
  dv.mergescript <path>` runs your own copy instead. It runs in the main checkout, in a zellij
  pane of its own or in the terminal; the PR's worktree goes first, and only if nothing in it is
  unpushed (no local changes, no commits beyond the PR's head). `githubmerge.repository` is
  passed for that run if you haven't set it.
- `dv list` has a **todo** tab: the PRs and replacements waiting on you, in the order to take
  them. dv finds what's waiting (a replacement to publish or close for, a push since your
  review, Claude answering you, your review requested, ready to merge, Claude's review new to
  you or its comments untouched, no review from you, activity since you looked); Claude puts it
  in order with a few words on each (why, and what next), on `R` there or by itself when its
  last ranking is old; and does its own part first on the top three (answering your replies,
  updating a review that's behind, or reviewing).

## [0.6.2] - 2026-10-18

### Added
- `a` on a replacement's description or closing note asks Claude to redraft it: the
  instructions start from a quote of the lines (and which draft they're from), and you say what
  to change.

## [0.6.1] - 2026-10-18

### Changed
- The comments pane is set off from the code: as a column, by a dim line (file headers stop
  at it); floating, by the theme's popup color.
- The review's summaries, yours (pending) and Claude's, sit at the top of the comments tab, two
  lines of each, beside the comments they sum up; the comments start below them, and scroll
  under them. A click on them goes to them in full, on the PR tab (where yours now is too).

## [0.6.0] - 2026-10-18

### Added
- `dv version` (and `--version`, `-V`) prints dv's version.
- `Space a` can check on Claude now (reads its review again, and its run's state) and, while a
  run is going, stop it (the session stays, so the next run resumes it); the status bar says
  how long it's been working.
- `c` on a conversation comment or the description quotes it in your review summary, saved on
  your pending review on GitHub (and loaded from it, so it's still there next time); `S` starts
  from it. `c` on Claude's summary adds to it the same way.
- New comments, replies and reviews on GitHub show up while dv is open: every 5 seconds it asks
  whether the PR changed (an ETag check, free against the rate limit while nothing has), and
  fetches it in the background when it has. Edits and resolved threads wait for the next fetch.
- Opening a PR shows its comments and info as dv last saw them (kept in dv's data directory)
  at once, while it fetches them afresh; anything that changes GitHub waits for that.

### Changed
- The PR's title line is pinned at the top, across the code and the pane, like the status bar
  (and in its color: the darker of the theme's status line and menu): it doesn't scroll away
  or fold. It has who opened the PR and from which branch into which (`owner:branch` from a
  fork), and the PR's counts (threads, files viewed, pending comments) when they fit (else
  they stay on the status bar); over the pane, its tabs.
- Comments are never inline any more: they're in the pane on the right, beside their lines, in
  split view too. It's shown when the window has room for it, else hidden, with the lines that
  have comments marked in the gutter (`●`, `◆` Claude's, `○` resolved); `Ctrl-w c` shows or
  hides it, and going to a comment shows it. `Ctrl-w f` has it float over the code (covering
  it only where there are comments) rather than beside it, as it does in a narrow window, and
  `Ctrl-w ,` / `.` resize it (`Ctrl-w =`: back to its usual width). dv remembers what you
  set with these keys for the next run, in any repo.
- The pane has a second tab, the PR's text (`gn` / `gp`, as Helix goes between buffers, or a
  click on the tabs over the pane, on the pinned title): its description, conversation, checks
  and Claude's summary, beside the code and scrolled on their own (`Ctrl-l` goes in, `Ctrl-h`
  back to the code). While the pane is shown the PR section is there, not above the diff (its
  header line stays); hidden, it's above the diff as before. The conversation no longer sits
  beside the description.
- Opening a PR skips `gh pr checkout` when the checkout is already at the PR's head (detached,
  or on its branch, as asked), and the base fetch when the PR's base commit is already here.

### Fixed
- In a window too narrow for the pane beside the code, going to a comment floats it over the
  code (it was nowhere); the PR tab's text and the pinned title don't scroll sideways with the
  diff; your own comments for Claude are marked `●`, not `◆`, in the gutter.
- `dv list` shows `claude ✓` for a review started from a PR's view, once it's written (also
  after you're back in the list, while Claude finishes it in its pane).
- `Space a` while Claude is working preselects what it did before, not "Stop Claude": Enter out
  of habit no longer ends the run. Stopping it also stops the commands it was running.
- Switching tabs from a comment puts the cursor on the line it's on, not the top of the diff; and
  where the cursor was in the code (`Ctrl-h` from the PR tab) survives the review reloading.
- Typing in a picker or menu lists what has the text as typed first: `Space a`, `check` picks
  "Check Claude's review now", not "Edit Claude's notes" (whose letters come in that order).
- A long code line in a comment (a suggestion) no longer runs over the right edge of its box:
  it's cut there, with `›` (and `‹`) where there's more, and the cursor moving along it
  scrolls that code block sideways, on its own: the code beside it and the comment's text
  stay put.
- Tables in comments wrap their cells' text, as GitHub does, rather than cutting it to `…`;
  one too wide even so (columns 20 wide) scrolls sideways like a code block.
- Claude's zellij pane gets dv's `XDG_DATA_HOME`, so it writes its review where dv looks for it.
- PRs with more than 100 review threads, 100 comments in a thread, or 100 files: dv loads them
  all (it stopped at 100, so later threads were missing and their files' "Viewed" marks too).
- `gf` right after `w` (which leaves the cursor on the space after the word) goes by the word.
- The pending view boxes your review's text, as the Claude view does Claude's summary; `c`
  there points to `S`.
- Help: `gf` mentions names; the `]n [n  Space n` line is aligned.

## [0.5.1] - 2026-10-17

### Added
- `gf` on a name in a comment (`torn_prefix_file`, or anything in `code`) goes to where it's
  defined: found by a grep for `fn name`, `class name` and the like, the diff's files first; a
  picker when there are several.
- `Ctrl-w p`: the pending view, as the Claude view for your pending review: its text so far and
  the comments in it, each with a few lines of code around it.
- In the Claude view (and the pending one), `+` / `-` show more or less code around a comment,
  and `Z` the whole file.

### Changed
- `dv list` keeps your marks (`m`) when you quit it, so you can go through marked PRs a few at
  a time; marks on PRs that have closed go.

## [0.5.0] - 2026-10-17

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

[0.8.1]: https://github.com/evanlinjin/diffview/releases/tag/v0.8.1
[0.8.0]: https://github.com/evanlinjin/diffview/releases/tag/v0.8.0
[0.7.2]: https://github.com/evanlinjin/diffview/releases/tag/v0.7.2
[0.7.1]: https://github.com/evanlinjin/diffview/releases/tag/v0.7.1
[0.7.0]: https://github.com/evanlinjin/diffview/releases/tag/v0.7.0
[0.6.2]: https://github.com/evanlinjin/diffview/releases/tag/v0.6.2
[0.6.1]: https://github.com/evanlinjin/diffview/releases/tag/v0.6.1
[0.6.0]: https://github.com/evanlinjin/diffview/releases/tag/v0.6.0
[0.5.1]: https://github.com/evanlinjin/diffview/releases/tag/v0.5.1
[0.5.0]: https://github.com/evanlinjin/diffview/releases/tag/v0.5.0
[0.4.1]: https://github.com/evanlinjin/diffview/releases/tag/v0.4.1
[0.4.0]: https://github.com/evanlinjin/diffview/releases/tag/v0.4.0
[0.3.0]: https://github.com/evanlinjin/diffview/releases/tag/v0.3.0
[0.2.0]: https://github.com/evanlinjin/diffview/releases/tag/v0.2.0
[0.1.0]: https://github.com/evanlinjin/diffview/releases/tag/v0.1.0
