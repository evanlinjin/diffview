# Changelog

All notable changes to dv. Versions follow [semver](https://semver.org); a release's notes
on GitHub are its section here.

## [Unreleased]

### Fixed
- After rerunning failed checks, dv looks at them again through its background watch (a picker
  open isn't disturbed, nothing said is overwritten), and a fetch from before the rerun isn't
  shown as current.
- Notes on a retargeted PR's base replace one another rather than pile up, and the "as dv last
  saw it" notice goes once GitHub's fresh answer is in.
- Signatures: GitHub refusing only a PR's reactions keeps them; refusing its commits, or
  saying an error without where, leaves them unknown (not unsigned); refusing the whole query
  says GitHub's reason.

## [0.33.1] - 2026-10-19

### Fixed
- Only Claude's CI-fixing runs may rerun workflows (no other background run), and its "reran
  it once" note stays, so it doesn't rerun the same head again.
- "Rerun the failed checks" reruns them a few at a time; one that can't be (too old) doesn't
  keep the others from it, dv says how many went, and looks at the checks again shortly after.
- `Space l` lists a review requested of a team (`org/team`), and withdraws it.
- A PR retargeted during a stopped rebase: the diff follows its new base once the rebase is
  through (it was forgotten).
- A GitHub hiccup over one commit's signature no longer marks it unsigned (which could refuse
  a merge); with `log.showSignature` set, dv's reading of `git log` isn't thrown off.

## [0.33.0] - 2026-10-19

### Added
- `Space s`: a shell in the checkout's directory (a PR's worktree): in zellij, a floating pane
  (`exit` closes it); otherwise in this terminal, back to dv when it exits.
- `Space g` → "Rerun the failed checks" when the PR's checks fail: their failed jobs, on
  GitHub (a flaky test, a network blip).
- Fixing CI, Claude reruns the failed jobs itself, once a head, when the failure looks
  transient (a network error, a timeout, a runner problem, a test known to be flaky); its note
  says so, and the todo then says it reran it once already.
- `Space l` requests reviews too, and withdraws them: `review from @amy` (✓ when requested),
  among the labels and assignees.

### Fixed
- A long line of the PR's facts (its branches, say) ran past the box, hiding its edge: it's cut
  there with `›` now, and scrolls sideways as the cursor moves along it, as code in comments
  does.
- A retargeted PR: its new base is fetched without holding up the screen, Claude's threads
  aren't shown twice, and a failed fetch is tried again. Label names with commas work. A
  range of commits picked from the "uncommitted" view is kept.

## [0.32.0] - 2026-10-19

### Added
- `Space l`: the PR's labels and assignees, and what it could have (the repo's labels, who it
  can be assigned to), ✓ on those it has; Enter adds or removes one. Its assignees show among
  its facts (`assigned @amy`).
- The commits list ends with `uncommitted · 2 staged, 3 changed, 1 new` while anything isn't
  committed: Enter there shows only that (as `Space g` does).
- Your own reactions are lit (in the accent color), as on GitHub: on review comments, the
  conversation's, and the PR's.

### Fixed
- A question about a commit shows while that commit (or only what isn't committed) is shown;
  it offers Ask or Change (no suggestion: it's on no lines), Ask preselected.
- The PR's base changed on GitHub while dv is open (retargeted): the diff starts from the new
  base (fetched), and a rebase goes onto it (0.31.0 said so, but dv didn't re-read the base);
  not from a stale cache at startup.
- Questions about a commit follow it however it's rewritten (dv's amend or rebase, a
  terminal, GitHub's version taken), to the commit with the same subject and author date.
- The Claude or Pending tab with nothing in it says so on an earlier commit too; a note of
  dv's rebase that ended in a terminal is forgotten.

## [0.31.0] - 2026-10-19

### Added
- `a` on a commit in the commits list (`Ctrl-w e`): ask Claude about it, or for a change to it
  (Ask / Suggest / Change first, as on lines). Its thread is under "about commit …"; a change
  is made uncommitted, for `f` on the commit to fold in.

### Changed
- In the commits list, amending what isn't committed into a commit is `f` (fold), not `a`.
- `Ctrl-w a` is gone: the Claude view is the comments pane's `Claude` tab (`gn` / `gp`, or a
  click), as the pending view is the `Pending` tab.

### Fixed
- Rebasing with Claude: it couldn't carry the rebase on (its `git` commands came prefixed or
  chained, and were refused); it now runs them plainly, and fixes what the rebase broke
  before the last commit. A rebase finished on a detached worktree is no longer taken for an
  abort; an up-to-date branch says so; a rebase you started yourself isn't taken for dv's;
  the base is fetched whatever the remote's refspec.
- The summaries' strip in the pane isn't cut where a short diff ends; an earlier commit shown
  doesn't make the Claude or Pending tab say it's empty.
- A stale note of dv's rebase no longer claims one you started; an up-to-date branch leaves no
  backup branch; the rebase goes onto the PR's current base.

## [0.30.2] - 2026-10-19

### Fixed
- The line between the code and the comments pane goes on to the bottom of the window (it
  stopped where the rows did). The Claude and Pending tabs, with nothing in them, say so on
  both sides.
- Rebasing: no aborting a rebase Claude is at; while one is stopped, Claude takes no other
  requests (nor answers replies); a rebase Claude gave up on is told apart from one it
  finished; a stopped `git am` isn't taken for a rebase.
- `gf` opens only GitHub comments, not the PR from other headings there.

## [0.30.1] - 2026-10-19

### Added
- Rebase onto the base branch with Claude's help: `Space g` → "Rebase onto origin/<base>"
  fetches it and rebases (HEAD kept on a backup branch first); if it stops on conflicts,
  Claude resolves them and carries it on, in the background. Stopped part way, `Space g`
  asks Claude to finish it, or aborts it.
- The PR's own reactions, among its facts (`reactions 👍 2`).

### Fixed
- `a` with text selected in a Claude thread (its comment, or its answer to your question)
  replies there, quoting the selection, instead of saying it only works on code lines.
- `gf` on a GitHub comment (its author line; or its text, with nothing else under the cursor)
  opens it on GitHub.

## [0.30.0] - 2026-10-19

### Added
- Emoji reactions: a GitHub comment's (review thread or conversation) show after who and when
  (`👍 2  🎉 1`); `Space =` on one (or on the description, for the PR's) reacts, or takes
  yours back (✓ marks yours).
- Amend what isn't committed (Claude's changes, say) into the PR's commits it belongs in:
  `Space g` → "Amend it into the PR's commits…" shows where each change goes (the commit that
  wrote the lines it changes, by `git blame`) before anything happens; Claude can be asked
  about the ones blame can't place (new lines). A backup branch first; the rest stays
  uncommitted. `a` on a commit in `Ctrl-w e`'s list amends it all into that one.

### Changed
- In the PR's conversation, a review's verdict stands out from comments: `✓ approved` in
  green, `✗ requested changes` in red (a review with only comments says `reviewed`).
- A file marked viewed doesn't fold while it has comments you haven't seen, and one folded
  already opens when new comments come in on it.
- One threads picker: `Space t`, with filters `Tab` / `Shift-Tab` go round (all, unresolved,
  pending, Claude, new); `Space p`, `Space A` and `Space n` open it on theirs.
- The pending view is the comments pane's `Pending` tab (between `Claude` and `PR`), not
  `Ctrl-w p`.
- Updating the checkout (`Space u`) is in `Space g` now: "Update the checkout".
- Todo: a PR with CI still running goes after what you can act on now (before what Claude's
  at), and back up once CI is done.
- A comment or reply for Claude (`a` on lines, `r` on its comment) first asks what you want of
  it: an answer (it changes nothing), a suggestion for you to post on GitHub, or the change made
  in the code here. Saved as `"want"` on it, shown as `[ask]`/`[suggest]`/`[change]` while it
  waits, and Claude does exactly that: on someone else's PR too (it used to only suggest
  there, whatever you asked). Preselected: what you last wanted on that thread, else the change
  on your PR, a suggestion on theirs.

## [0.29.0] - 2026-10-19

### Added
- `Space g` can take the PR as it is on GitHub when your checkout has diverged from it (or is
  behind): your commits that GitHub doesn't have are kept on a `dv-backup/pr-N-<sha>` branch first.
- Ask Claude a question about the PR: `Space a` → "Ask Claude a question about this PR…". It
  answers right away, in the Claude tab (under its review, or under "questions for Claude" before
  there's one); `r` there carries the conversation on.

### Changed
- Claude's panes float in dv's tab again (hidden until you show them), and close once Claude's
  done there: no pile of them. The session stays, so the next ask resumes it.

### Fixed
- zellij crashing after Claude opened in its own tab: dv focused Claude's pane there, which
  zellij (0.45) can panic on ("Cannot focus pane … as it is not selectable"), taking the session
  down. That tab is gone (Claude's panes float in dv's tab again).

## [0.28.1] - 2026-10-18

### Fixed
- `dv list` freezing for seconds after you come back from a PR: it looked up every worktree's
  closed PR on GitHub, one by one, at each refresh. Those are kept now, and new ones (and the PRs
  deferrals wait on) are looked up in the background.
- Opening a PR from `dv list` is quicker: the selected PR is looked up on GitHub while you're on
  its card (its comments too, the first time), so Enter doesn't wait on that.

## [0.28.0] - 2026-10-18

### Fixed
- A PR rebased onto a newer base while open in dv (by Claude, say) no longer shows the base's own
  commits and files as the PR's: the diff starts from the new fork point.

### Changed
- Auto CI is now auto (`C` in `dv list`, git config `dv.auto`; turn it on again): Claude also
  reviews, updates its reviews and answers you as things come in, and ranks the todo list again as
  it changes (at most every 15 minutes), on top of fixing your CI and marking green drafts ready;
  and once a published replacement's CI passes, it closes the PRs it replaces, with the closing note.

### Added
- Closing a PR with a comment Claude drafts: `Space a` → "Ask Claude to draft a comment closing the
  PR" (say why, if you like). The draft shows first in the diff; `Space a` then closes the PR with
  it, after your editor has it to change, asks for changes (`a` on it too), or discards it.

## [0.27.1] - 2026-10-18

### Fixed
- `dv list` lagging: it ran git for dv's directory each time it wanted it (hundreds of times a
  refresh), and looked for unpushed work in every worktree while you waited; now git's asked once,
  and the worktrees are looked at in the background.

## [0.27.0] - 2026-10-18

### Added
- Auto CI, `C` in `dv list` (shown on its top bar, kept in git config `dv.autoCI`): on your PRs,
  Claude starts fixing CI as soon as it fails, and a draft is marked ready for review once CI passes.
- `Space a` in a PR of yours with failing CI: Claude fixes it, as `a a` does from the todo tab.
- When Claude finds failing CI isn't the PR's doing (a flaky test, CI itself, the base), it says so
  on the todo card and offers a new PR fixing CI (`N N` on it, or `Space a` in the PR); `a a` there
  has it try again all the same.

### Changed
- `dv list` sorts by created by default (newest first), not updated: `o` still cycles.
- An unpublished replacement or new PR has no PR tab: what it replaces (or what you asked for)
  heads its description, in the diff. Published, the tab is back.
- Claude's panes open in a zellij tab of their own per repo, `claude: <repo>`, stacked there, not
  floating over dv: they no longer pile up over the code as you open PRs. `Space a` → "Show
  Claude's pane" goes there.

### Fixed
- `dv list`'s top bar is as dark as the PR view's, its other tabs dimmed the same way.

## [0.26.0] - 2026-10-18

### Changed
- Replacements and new PRs publish as drafts only; a draft is marked ready for review (`Space g`)
  only once its CI passes: dv says what's failing or still running otherwise.
- An unpublished replacement or new PR has one tab for the comments, `Claude` (the whole diff, with
  Claude's summary): every comment there is Claude's or yours.

### Added
- Todo: Claude fixes your PR's failing CI (a published draft's too): `a a` on it has Claude read
  the failed runs' logs and fix the cause in its worktree, for you to check and push.

## [0.25.0] - 2026-10-18

### Changed
- Todo: anything Claude is at goes to the bottom, replacements too: one Claude is working on in a
  pane in its worktree (making changes there, say) no longer stays on top as ready to publish.
  And a Claude Code pane counts by its folder and screen, so one started from a shell counts too.
- A replacement's (or new PR's) drafted description, and its closing note, are sections before the
  files in the diff, as a PR's description is, not on the pane's PR tab; in the contents too.

## [0.24.3] - 2026-10-18

### Changed
- Todo: a PR whose replacement Claude is drafting goes to the bottom ("drafting a replacement"),
  as a PR Claude's reviewing does; once drafted, the replacement, ready for you to review, goes
  first.

### Fixed
- `dv list`: the tab shown wasn't lit (the position at the header's right end, drawn over the whole
  line, took its colors).
- In a dv that had been running across an install (a new dv in its place), drafting a
  replacement, or a review's worktree for `a a`, failed with "No such file or directory": dv ran
  another of itself by its old path, gone. It runs the new one now.

## [0.24.2] - 2026-10-18

### Changed
- `dv list`'s mouse: a click selects a card and a double-click opens it (a click on the selected
  one opened it, which a wheel turn could make a surprise); a click on a tab switches to it. Its
  tabs look as the PR view's do: the one shown lit, the others dim.

## [0.24.1] - 2026-10-18

### Changed
- `dv list` checks GitHub every 5 seconds, not 30 (as the PR view does): a check that finds
  nothing new is free.

## [0.24.0] - 2026-10-18

### Added
- `dv list` keeps up with GitHub by itself: it checks the open PRs' ETag every 30 seconds (free
  when nothing changed) and fetches them again when it changes, every 5 minutes regardless (for
  CI), and on coming back from a PR; in the background, the cards updating in place.

## [0.23.3] - 2026-10-18

### Fixed
- github-merge.py's GitHub requests went unauthenticated (60 an hour, then "rate limit
  exceeded"): dv gives it your `gh` token (`user.ghtoken`, from `gh auth token`) when you haven't
  set one, for that run only, fetched as it starts rather than written into its command.

## [0.23.2] - 2026-10-18

### Fixed
- On the PR tab, after the PR's text grew (posting a comment there with `S`, or a refresh while a
  preview was up), `Ctrl-h` and `gn` went to PR text (a comment) instead of back to the code, so
  the cursor seemed stuck in the pane: where you were in the description (or a commit's message)
  was kept by row number, which the PR text before it moved. It's kept by section now, and going
  back never lands on PR text or a comment.
- In zellij, dv takes the focus back when your editor's pane closes (and the merge pane gives it
  back when it closes), as zellij may focus a pane beside it instead.

## [0.23.1] - 2026-10-18

### Added
- `y` on a commit in the contents' commits copies its hash (`Space c` on a commit's message in the
  diff too); while the list has focus, the key hints are its own (the commits' or the contents').

## [0.23.0] - 2026-10-18

### Added
- `U U` in `dv list` unmarks every marked PR.
- `Space c` on a link copies its URL (the link's tooltip says so, and the key hints have a
  "link" set: open, copy, preview).

### Fixed
- `dv list` missed Claude Code working in a pane: it looked for "esc to interrupt", which Claude
  Code no longer shows. It reads the spinner line now (`· Moseying… (59s · …)`, whatever the
  word). And a Claude Code pane in a PR's worktree counts too, one dv didn't start ("claude:
  working (in its pane)…", or asking you something).

### Changed
- A merge that succeeds deletes github-merge.py's branches for the PR (`pull/N/base`,
  `pull/N/head`), which piled up a pair a merge; a failed one keeps them.

## [0.22.0] - 2026-10-18

### Changed
- github-merge.py's zellij pane closes by itself when the merge succeeds; when it fails (or you
  reject it) it says so and waits for Enter, so you can read why.
- Claude's review summary (and your replies to it) is the `Claude` tab's alone: no longer on the
  PR tab, nor in the summary box over the comments (which has your pending review's text). With
  no PR (a branch Claude reviewed), it's still at the top, as before.

## [0.21.1] - 2026-10-18

### Fixed
- Right after dv pushed, posted Claude's description, marked the PR ready or approved its runs,
  a GitHub fetch already under way could bring back the PR as it was before for a moment (the
  old description, an `↑1 unpushed` badge): it's dropped now.
- Publishing a new PR said "S closes what it replaces": it replaces nothing.

## [0.21.0] - 2026-10-18

### Added
- On someone else's PR, `Space a` can have Claude make the changes too (not only suggest them):
  as commits in your checkout, for you to push to their branch with `Space g`. The menu says
  whether they allow maintainer edits (else the commits stay with you).

## [0.20.0] - 2026-10-18

### Added
- `N N` in `dv list`: Claude drafts a new PR from what you write (issues by `#N`), unpublished, on
  a branch of its own (`dv/new-K`, in `<repo>-new-K`). It's on the drafts tab (the replacements
  tab, renamed) as `new K`: Enter reviews it in dv, `Space a` redrafts it, `S` publishes it.

## [0.19.1] - 2026-10-18

### Changed
- Todo: a card Claude has ranked says, before Claude's words, what holds the PR up on GitHub:
  `[⏸ 3 runs to approve (W W) · ✗ 2 checks failing · 1 unsigned commit · conflicts]` (Claude's
  ranking may be older than they are, and only the rule order said them before).

## [0.19.0] - 2026-10-18

### Added
- Workflow runs GitHub holds for a maintainer's approval (a first-time contributor's PR): the
  PR's status box says so (`⏸ 2 workflow runs awaiting approval`), its `dv list` card too
  (`⏸ 3 to approve`), and the todo tab has it as a need. `Space g` → approve in the PR, or `W W`
  in `dv list`, approves them, so they run.

## [0.18.1] - 2026-10-18

### Fixed
- A PR whose branch is checked out in another worktree already (a published replacement's, in
  its own worktree) couldn't be opened on its branch (`-b`, `dv list -b`): git won't check a
  branch out twice. It opens in that worktree now, as it is.

## [0.18.0] - 2026-10-18

### Added
- Todo: failing CI is a need ("2 checks failing: fix them" on yours; "see why, tell the author"
  on others', not on their drafts).
- Whether a PR's commits are signed: the PR tab's status box says so (`commits: ✓ all 3 signed`,
  or which aren't, and whether the base requires signed commits); unsigned commits are marked
  `✗` in the contents' commits; and on the todo tab, unsigned commits where the base requires
  signed ones are a need ("ask the author to sign them").
- Merging (`M M`, `Space g`) checks first: unsigned commits where the base requires signed ones
  (a ruleset, or classic branch protection) stop it before github-merge.py runs, saying why; and
  such a PR isn't "approved, checks green: merge it" on the todo tab.

### Fixed
- The pane's `PR` tab kept the Claude view (the diff only Claude's review): it shows the whole
  diff now, as `Comments` does; a click on the summaries goes there too, and the Claude view has
  nothing on the PR tab.

## [0.17.1] - 2026-10-18

### Fixed
- `dv list`: why Claude failed was cut to 60 characters (and the card's 30-column zone): its
  card says all of it now, the review and Claude lines running on to the window's edge.

### Changed
- `dv list`: a marked PR's `●` is in a column of its own before its number, not squeezed against
  it, so marked cards stand out and line up.

## [0.17.0] - 2026-10-18

### Fixed
- `a` on Claude's review summary (a selection in it, say) said it only works on code lines: it
  replies to the summary now, quoting what's selected. On a conversation comment, `a` quotes
  what's selected of it too (else all of it).

### Changed
- With the contents list shown, the key hints (after `Space`, `g`, `Ctrl-w`…, and what the keys do
  where the cursor is) are pinned at its bottom instead of over the code, and its commits follow
  its files right after, a blank line between. Under its heading, the contents and the commits
  scroll as one list (following the cursor, or the wheel), the hints pinned below.

## [0.16.0] - 2026-10-18

### Added
- `r` on Claude's review summary replies to it: your reply and Claude's answer are a thread under
  the summary (on the PR tab, and in the Claude view), and `r` there goes on with it.

## [0.15.0] - 2026-10-18

### Added
- `Space g` on a draft PR: mark it ready for review.
- Todo: your draft with its checks green and nothing unpushed is a need, "mark it ready for review".
- Publishing a replacement (`S`): "Publish ready for review (not a draft)", with or without closing
  what it replaces.

## [0.14.0] - 2026-10-18

### Added
- Todo: "Claude made changes, not pushed: … check them, push them" is a need of its own, at the
  top: work in a PR's worktree that Claude made (it said what it `Changed:`) since the PR's head.
  A Claude run in a zellij pane counts as done once Claude stops working there, not when its pane
  closes.
- The comments pane has a `Claude` tab, between `Comments` and `PR`: the Claude view (`Ctrl-w a`)
  as a tab you can see. `gn` / `gp` go round the three; `Comments` is the whole diff again.
- In the Claude view, Claude's review summary is in full, before the files (`Tab` folds it).

## [0.13.0] - 2026-10-18

### Added
- The pinned PR title says, as badges that always show, when Claude is working here (in the
  background, or in its pane, or asking you something there) and when the checkout is out of
  step with the PR: `↑2 unpushed`, `⇅ diverged: force-push`, `↓ behind the PR`, `✎ uncommitted`.
- `a` on a comment in the PR's conversation asks Claude about it: your question (the comment
  quoted) and Claude's answer are a thread under it, on the PR tab (`r` to go on).
- Commits shown: each one's message is a section before the files (one for every commit in a
  range), after the description. The files' list is now the diff's contents (`CONTENTS`): the
  description and the messages first, then the files; `j` / `k` and clicks go to them too.
- `Ctrl-w j` / `Ctrl-w k` go straight to the commits / the files in the files' list, and `Tab`
  there goes between them (no more stepping through every file to reach the commits).
- `a` on the PR's description: Claude proposes a new one, as you ask. dv shows it against the
  current text; `Space a` approves it (it replaces the description on GitHub), asks for more
  changes, or discards it.
- The PR's facts on the PR tab are in a box whose title opens the PR on GitHub; each failed or
  pending check links to its run, and `checks` to the PR's checks page.

### Fixed
- Moving in the contents list (`j` / `k`) no longer moves the diff: it has its own cursor,
  and `Enter` goes there.
- Pickers and popups (`?`, hover, a commit's message, the menus) are over the whole window
  again, the contents list too: they were as narrow as the code beside it.

## [0.12.0] - 2026-10-18

### Added
- The PR's commits, under the files' list: `Enter` on one shows just that commit, `v` then
  `Enter` a range of them, `all commits` the whole diff again (a click, or Shift-click for a
  range, too). Comments show when what's shown ends at the PR's last commit. `m` there shows
  a commit's whole message in a popup.
- The PR's description is a section before the files, like a file (`Tab` folds it), no longer on
  the pane's PR tab; one commit shown, its message is another, above it.

### Fixed
- `j` / `k` in the comments pane stay there, past the first and last comments (they went into the
  code).

## [0.11.0] - 2026-10-18

### Added
- `Ctrl-h` / `Ctrl-l` go through all the panes: the files' list, the old and new sides, the
  comments. In the files' list, `j` / `k` go file to file.
- The pane that has focus shows it: its edge in the key color.
- `Ctrl-w ,` / `.` / `=` resize the pane you're in (the files' list too), and `Ctrl-w q` hides it.

## [0.10.0] - 2026-10-18

### Added
- The changed files' list on the left (`Ctrl-w e`, kept for the next run): files under their
  folders, the one you're in marked, `✓` when viewed, `·N` its open comments; a click goes to
  one. Shown by default from 150 columns.
- Claude's pane (`Space a` inside zellij) opens hidden: a floating pane that doesn't take
  focus. `Space a` → "Show Claude's pane" brings it up, and dv says when Claude is asking you
  something there (a permission prompt).
- The file you're in keeps its header pinned at the top of the code as you scroll through it.

## [0.9.1] - 2026-10-18

### Fixed
- Comments in a folded file (a viewed one) no longer go unseen: its header counts them
  ("+8 −1 · 1 comment"), and `]c` / `[c` go into it, unfolding it, as `]u` and `]n` do.
- A review with no text in the conversation (an approval or change request alone) has its box
  closed, saying "(no comment)", instead of running into the next.

## [0.9.0] - 2026-10-18

### Added
- `a` on a GitHub review comment asks Claude about it ("what do you think about this?"): your
  editor has it quoted, and Claude answers on that thread's lines.
- Reviewing, Claude reads the PR's review comments (`dv review-comments`, read only) and answers
  those it has something to add to, on their lines; dv shows its comment as `re @who`.

### Changed
- Claude's comments stand apart from GitHub's: a `◆` before "claude", and the boxes of comments
  in Claude's review file (its, and yours for it) in the hint color.

### Added
- Deferring until something happens: `s` then `u` on the todo tab, in your words ("#2380 is
  merged", "CI is green", "@amy approves", "Monday"). dv reads the usual phrasings itself, and
  Claude the rest (one small run); then dv watches for it, and the item's back when it's
  happened.
- A PR comment saying what it waits on defers it on everyone's todo tab: "Waiting on #2380",
  "Blocked on #2380", "Depends on: CI to pass", by you or anyone with write access (a later
  "Unblocked" ends it). `p` posts yours from a deferral, and Claude notes such waits it reads in
  the conversation when it ranks.

### Added
- The todo tab lists a PR whose worktree has work that isn't on GitHub (uncommitted files, or
  commits the PR doesn't have: yours, or Claude's after making changes), near the top, with
  `Space g` there to push it. Before, it could drop off the list while the work sat there.

## [0.8.6] - 2026-10-18

### Fixed
- The mouse wheel over the comments pane stops at its end: on the comments tab once the last
  comment is on screen (it scrolled on through the code), on the PR tab at the end of its text
  (it went on to scroll the code).

## [0.8.5] - 2026-10-18

### Added
- `gd` / `gr` / `gy` / `gi` on a name in a comment or the PR's text (`_reveal_next_spk`): found by
  rust-analyzer's symbol search, then its references or implementations from there; without
  rust-analyzer, `gd` finds the definition by text, as `gf` does.

### Fixed
- `gw` labels the PR tab's text too.
- Between the code and the comments pane: `Ctrl-l` from a line goes to the comment about it, and
  `Ctrl-h` from a comment to the line it's about (not whatever happens to sit beside its box);
  word motions (`w`, `e`, `b`) carry on along the code or the pane, not across into the other.

## [0.8.4] - 2026-10-18

### Fixed
- Merging (`M M`, `Space g` → merge) checks the PR is still open on GitHub first: merged or
  closed (the list can be older than that), it says so rather than running `github-merge.py`, which
  failed on the merge ref GitHub drops.

## [0.8.3] - 2026-10-18

### Fixed
- Opening a PR that's checked out already (its worktree, or its branch here) never drops
  commits made there since (yours, or Claude's: a commit, an amend, a rebase), `-f` or not: it's
  kept as it is, and dv says how many commits the PR doesn't have yet. Before, dv updated it to
  the PR's head, leaving them only in the reflog. dv records the commit it last checked out, so a
  PR its author force-pushed still updates.
- Worktrees made for Claude's runs in `dv list` (`a a`) follow the list's `-b` (on the PR's
  branch); they were always detached.
- `Space g` pushes from a detached `dv N -w` worktree too (to the PR's branch, a fork's
  included), and, after a rebase or amend there, offers a force-push with a lease (only over the
  PR's head as dv knows it).

## [0.8.2] - 2026-10-18

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

[0.33.1]: https://github.com/evanlinjin/diffview/releases/tag/v0.33.1
[0.33.0]: https://github.com/evanlinjin/diffview/releases/tag/v0.33.0
[0.32.0]: https://github.com/evanlinjin/diffview/releases/tag/v0.32.0
[0.31.0]: https://github.com/evanlinjin/diffview/releases/tag/v0.31.0
[0.30.2]: https://github.com/evanlinjin/diffview/releases/tag/v0.30.2
[0.30.1]: https://github.com/evanlinjin/diffview/releases/tag/v0.30.1
[0.30.0]: https://github.com/evanlinjin/diffview/releases/tag/v0.30.0
[0.29.0]: https://github.com/evanlinjin/diffview/releases/tag/v0.29.0
[0.28.1]: https://github.com/evanlinjin/diffview/releases/tag/v0.28.1
[0.28.0]: https://github.com/evanlinjin/diffview/releases/tag/v0.28.0
[0.27.1]: https://github.com/evanlinjin/diffview/releases/tag/v0.27.1
[0.27.0]: https://github.com/evanlinjin/diffview/releases/tag/v0.27.0
[0.26.0]: https://github.com/evanlinjin/diffview/releases/tag/v0.26.0
[0.25.0]: https://github.com/evanlinjin/diffview/releases/tag/v0.25.0
[0.24.3]: https://github.com/evanlinjin/diffview/releases/tag/v0.24.3
[0.24.2]: https://github.com/evanlinjin/diffview/releases/tag/v0.24.2
[0.24.1]: https://github.com/evanlinjin/diffview/releases/tag/v0.24.1
[0.24.0]: https://github.com/evanlinjin/diffview/releases/tag/v0.24.0
[0.23.3]: https://github.com/evanlinjin/diffview/releases/tag/v0.23.3
[0.23.2]: https://github.com/evanlinjin/diffview/releases/tag/v0.23.2
[0.23.1]: https://github.com/evanlinjin/diffview/releases/tag/v0.23.1
[0.23.0]: https://github.com/evanlinjin/diffview/releases/tag/v0.23.0
[0.22.0]: https://github.com/evanlinjin/diffview/releases/tag/v0.22.0
[0.21.1]: https://github.com/evanlinjin/diffview/releases/tag/v0.21.1
[0.21.0]: https://github.com/evanlinjin/diffview/releases/tag/v0.21.0
[0.20.0]: https://github.com/evanlinjin/diffview/releases/tag/v0.20.0
[0.19.1]: https://github.com/evanlinjin/diffview/releases/tag/v0.19.1
[0.19.0]: https://github.com/evanlinjin/diffview/releases/tag/v0.19.0
[0.18.1]: https://github.com/evanlinjin/diffview/releases/tag/v0.18.1
[0.18.0]: https://github.com/evanlinjin/diffview/releases/tag/v0.18.0
[0.17.1]: https://github.com/evanlinjin/diffview/releases/tag/v0.17.1
[0.17.0]: https://github.com/evanlinjin/diffview/releases/tag/v0.17.0
[0.16.0]: https://github.com/evanlinjin/diffview/releases/tag/v0.16.0
[0.15.0]: https://github.com/evanlinjin/diffview/releases/tag/v0.15.0
[0.14.0]: https://github.com/evanlinjin/diffview/releases/tag/v0.14.0
[0.13.0]: https://github.com/evanlinjin/diffview/releases/tag/v0.13.0
[0.12.0]: https://github.com/evanlinjin/diffview/releases/tag/v0.12.0
[0.11.0]: https://github.com/evanlinjin/diffview/releases/tag/v0.11.0
[0.10.0]: https://github.com/evanlinjin/diffview/releases/tag/v0.10.0
[0.9.1]: https://github.com/evanlinjin/diffview/releases/tag/v0.9.1
[0.9.0]: https://github.com/evanlinjin/diffview/releases/tag/v0.9.0
[0.8.6]: https://github.com/evanlinjin/diffview/releases/tag/v0.8.6
[0.8.5]: https://github.com/evanlinjin/diffview/releases/tag/v0.8.5
[0.8.4]: https://github.com/evanlinjin/diffview/releases/tag/v0.8.4
[0.8.3]: https://github.com/evanlinjin/diffview/releases/tag/v0.8.3
[0.8.2]: https://github.com/evanlinjin/diffview/releases/tag/v0.8.2
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
