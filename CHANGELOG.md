# Changelog

All notable changes to dv. Versions follow [semver](https://semver.org); a release's notes
on GitHub are its section here.

## [Unreleased]

## [0.74.4] - 2026-10-20

### Fixed
- A deleted reviewer or assignee shows as ghost too (the PR's facts and activity, dv list's
  reviews), so deferrals waiting on "a review from anyone" judge them alike.
- A deleted author's PR isn't on the stalled tab: there's no one to nudge.

## [0.74.3] - 2026-10-20

### Fixed
- A deleted account shows as ghost everywhere (PR and issue lists, a PR's author, its
  comments), not as a blank.

### Changed
- dv takes the terminal itself, with one panic hook of its own, set once.

## [0.74.2] - 2026-10-20

### Fixed
- A comment under no code line shown (its line hidden by less context) leaves the cursor on its
  own line when it goes.
- An issue that won't open says GitHub's reason (a PR's number, no access); a deleted author
  shows as ghost.
- The panic log's place is found when dv takes the terminal, not inside the panic hook.

## [0.74.1] - 2026-10-20

### Fixed
- In zellij, the PR view's asks (review, change, describe, fix CI, close) take their task's model
  too; Claude's pane takes `dv.model`.
- A comment that goes (posted, deleted) leaves the cursor on the line it was under, outdated or
  not.
- In an issue, a drag scrolls only once past the view's edge, not on its first line.
- The panic log is in dv's directory (`panic.log`), the one before kept as `panic.log.old`.

### Changed
- An issue opens with one GitHub query, not two.

## [0.74.0] - 2026-10-20

### Added
- In an issue, a drag at the view's top or bottom edge scrolls it on.

### Fixed
- A sideways scroll in an issue no longer sends the cursor to its first line.
- A talk resolved again (by you or by Claude) folds, even if `Tab` had it shown.
- The cursor is kept clear of a floating comment beside its line.
- Sideways over a comment's prose leaves the cursor where it is.
- `dv-panic.log` starts over once past a MiB.

## [0.73.0] - 2026-10-20

### Added
- A model per Claude task: `git config dv.model.<task>` (`review`, `answer`, `change`, `rank`,
  `triage`, …; see the README), else `dv.model`, else Claude Code's default. Claude's pane
  (`Space a` → open) keeps Claude Code's own.

## [0.72.5] - 2026-10-20

### Fixed
- Posting Claude's comment (`c`/`C`) leaves the cursor on its line, not the file's top; so does
  any thread that goes (deleted, dismissed).

## [0.72.4] - 2026-10-20

### Fixed
- Scrolling sideways with the mouse keeps the cursor on screen: scrolled off, it stays at the
  first (or last) column shown, in the PR view and the issue view.
- The wheel with Alt or Ctrl scrolls sideways too: kitty keeps Shift and the mouse for itself,
  so Shift and the wheel did nothing there.

## [0.72.3] - 2026-10-20

### Fixed
- Under Claude's comment, replies' code boxes and the cursor stay inside the screen.
- A resolved talk no longer counts as Claude's turn, and resolving it again folds it.

## [0.72.2] - 2026-10-20

### Fixed
- Claude's comment and the talk under it:
  - `c`/`C` posts the comment, wherever the cursor is in its thread (a reply was posted, and
    the comment dropped);
  - `Space r` on your own note for Claude resolves it as before;
  - a resolved talk no longer counts as "answered, your turn";
  - `Tab` shows a resolved talk to read, without reopening it.
- Replies' guide is drawn beside their text, not in it: motions, copying and `Space N` see the
  text as it is.

## [0.72.1] - 2026-10-20

### Fixed
- `Space N` in a PR hands your editor the terminal properly, and the draft runs on its own,
  carrying on if dv quits. Left untouched in the editor, nothing is asked.
- Auto description checks:
  - only on PRs Claude has reviewed (one never reviewed was kept from its review);
  - changes made while a proposal waits are checked after.

## [0.72.0] - 2026-10-20

### Changed
- Claude's comment on the code is what's kept:
  - replies under it are indented, as talk about it;
  - `Space r` resolves that talk (it folds to a line) and leaves the comment open; the comment
    itself is only deleted (`Space d`);
  - "suggest" has Claude revise its comment and resolve the talk; a change made marks the
    comment done.

## [0.71.0] - 2026-10-20

### Added
- Auto mode: once one of your PRs changes, Claude checks its description. If it no longer
  fits, Claude proposes a new one, which you approve in the PR's view (`Space a`).

## [0.70.0] - 2026-10-20

### Added
- `Space N` in a PR or an issue: Claude drafts a new PR from what's selected (or the cursor's
  line). You say what it should do in your editor first. The draft shows in dv list's drafts
  tab.

## [0.69.10] - 2026-10-20

### Fixed
- Two panics at once don't mix their lines in the panic log.
- A CI rerun that hits a bug says which run, and that it was dv's bug.

## [0.69.9] - 2026-10-20

### Fixed
- After a crash, the mouse is given back to the shell too.
- A draft always ends, even when reading its record hits a bug.
- The panic log keeps every panic.

## [0.69.8] - 2026-10-20

### Fixed
- A bug in a background run (a review, a triage, a draft, a ranking) no longer gives the
  terminal back mid-session or leaves its work marked as under way. It fails, says why, and
  the rest goes on.

## [0.69.7] - 2026-10-20

### Fixed
- A draft or a ranking whose run hits a bug ends as failed, rather than counting as under way
  for the rest of the session.

## [0.69.6] - 2026-10-20

### Fixed
- Quitting dv list or dv issues asks first while Claude drafts a replacement or a new PR, or
  ranks, not only while it reviews.

## [0.69.5] - 2026-10-20

### Fixed
- dv list: Esc while Claude's reviews run says how to quit (q, then q again) rather than
  arming a confirm the next Esc calls off.

## [0.69.4] - 2026-10-20

### Fixed
- dv list: Esc after a key waiting for its second (`g`, `M`, `D`, `s`, …) calls it off
  instead of quitting. In dv issues' list, it no longer clears the filter too.

## [0.69.3] - 2026-10-20

### Fixed
- In an issue, Esc after any pending key (`L`, `T`, `X`, `N`, `a`, `g`, Space) calls it off
  instead of leaving the issue.

## [0.69.2] - 2026-10-20

### Fixed
- In an issue, `x` again on the last line keeps the whole line selected.
- `Space` then `Esc` calls Space off rather than leaving the issue.

## [0.69.1] - 2026-10-20

### Fixed
- An issue's timeline is read whole, not just its last 100 entries. Other repos' issues and
  PRs are named owner/repo#N, bots show when assigned, and deleted accounts read @ghost.
- A draft fix just started holds its issue off the todo right away.
- With the `Space l` picker open, the mouse doesn't reach the issue behind it.
- The cursor stays out of the blank lines at an issue's end.

## [0.69.0] - 2026-10-20

### Added
- An issue's activity shows its timeline:
  - comments;
  - labels added or removed, assigned, retitled;
  - closed (and with what), reopened;
  - mentioned elsewhere, marked a duplicate, linked PRs.
- What Claude's triage suggests doing (labels, title, closing) is a box of its own.

### Fixed
- dv issues:
  - a failed draft doesn't hide its issue;
  - drafts' state updates as they start and finish;
  - the `Space l` picker gets the wheel, says it's loading, and doesn't panic narrow;
  - the keys' box no longer hides an issue's last lines.
- Stalled: an author's push from an email not on their account counts as their answer.
- The keys' box pads to its longest key.

## [0.68.0] - 2026-10-20

### Added
- In an issue:
  - `Space l` toggles its labels and assignees, as in a PR;
  - a key box shows the most used keys;
  - its view lists the drafts of Claude's that fix it, published or not.
- `dv issues`: an issue Claude is drafting a fix for, or has an unpublished draft fix, leaves the
  todo; its card says so.

### Fixed
- No "✓" that reads as approval: "claude: drafted", "claude: triaged".
- Stalled PRs:
  - only maintainers' reviews and comments count;
  - an approval since clears them;
  - a maintainer's "Update branch" isn't the author's word;
  - drafts and bots' PRs are left out.
- An issue view's cursor stays on its side across a resize.

## [0.67.0] - 2026-10-20

### Added
- dv list's "stalled" tab lists PRs whose author has left a review or a maintainer's comment
  unanswered for `dv.stalledDays` (git config; 14 days by default). A PR waiting on reviews
  doesn't count. `H` drafts a "do you need help with this?" comment in your editor and posts
  it.

## [0.66.1] - 2026-10-20

### Fixed
- In an issue, switching sides with a selection drops it instead of breaking `y`, and the
  cursor stays where it can be after a resize.

## [0.66.0] - 2026-10-20

### Added
- An issue's description and the pane beside it scroll separately. The wheel scrolls the
  side it's over.

### Fixed
- A `#N` link: "opening #N…" shows while it's looked up, and a link to the open issue goes to
  the browser.
- Backspace (Ctrl-h in many terminals) goes to the description.

## [0.65.0] - 2026-10-20

### Added
- In an issue's view:
  - `Ctrl-h` / `Ctrl-l` move the cursor to the description / the pane;
  - Enter on a `#N` link opens that PR's review (back to the issues after) or that issue in dv.

## [0.64.3] - 2026-10-20

### Fixed
- The record of dropped commits is only added to, once each, and written whole. One worktree
  can no longer undo another's drop, and another dv never reads it half written.

## [0.64.2] - 2026-10-20

### Fixed
- A dropped commit is recorded for every worktree of the repo, and forgotten once it's back.
- `Space C` picks a commit by its sha (it may have moved since the picker opened), and doesn't
  mark "all commits" while only uncommitted changes show.

## [0.64.1] - 2026-10-20

### Fixed
- A commit an amend drops (via `f`, `dv amend` or Claude) is recorded, so its questions don't
  move onto another commit; `f` and `dv amend` say "dropped".
- dv list: Claude's count says "on an older head" when it reviewed one (some may be
  outdated), and says when a review file won't read.
- A published replacement's Claude tab falls back to its own next step.

## [0.64.0] - 2026-10-20

### Added
- `Space C`: a commit picker. It shows one commit's diff, or all of them.

## [0.63.1] - 2026-10-20

### Fixed
- Dropping a commit an amend empties works whatever your rebase settings. With
  `rebase.abbreviateCommands`, it used to keep the commit and lose the changes.
- The amend no longer runs your sequence editor.
- A dropped commit's questions don't move onto another commit.
- A published replacement's Claude tab shows its PR's next step.

## [0.63.0] - 2026-10-20

### Changed
- dv list: a reviewed PR's card says how many of Claude's comments are left to address
  ("claude: 3 to address", "claude: all addressed"), not "claude ✓", which read as approval.

## [0.62.0] - 2026-10-20

### Added
- The Claude tab of a PR opens with what dv list's ranking says to do next about it, and how
  long ago it ranked.

## [0.61.1] - 2026-10-20

### Fixed
- Claude's commit rewords:
  - the amend picker shows them first;
  - the count is right;
  - questions on a reworded commit stay with it;
  - a message that would itself be autosquashed is ignored.
- Builds on Rust 1.89 again.

## [0.61.0] - 2026-10-20

### Added
- An amend whose changes undo all a commit did drops that commit, where before the amend failed.
  dv says which commits it dropped, and the backup branch keeps them. Claude, placing changes,
  puts undoing changes in the commit they undo.

## [0.60.0] - 2026-10-20

### Added
- Amending with Claude ("Amend it all…", or its answers in the amend picker): when a commit's
  message wouldn't describe it once the changes are in, Claude writes a new one and dv rewords
  the commit in the same amend.

### Changed
- Checking whether anything's uncommitted no longer reads every changed file.

## [0.59.1] - 2026-10-20

### Fixed
- Whether anything is uncommitted is judged by the diff dv shows, not `git status`, so a
  submodule's changes or a staged-then-undone change can't leave an empty view.
- In an issue, a drag into the other side keeps the cursor on the side it selects.

## [0.59.0] - 2026-10-20

### Added
- In a PR, the state chip says when it conflicts with its base: " OPEN · conflicts " in red.

### Fixed
- In an issue, a click in the margin doesn't copy a character, and a click near the edge
  doesn't scroll.
- Back to the whole diff once nothing's uncommitted: the cursor keeps its place, and Space g
  says so rather than flipping back a second later.

## [0.58.2] - 2026-10-20

### Fixed
- Issue view cursor:
  - it stays on its side's columns;
  - `x` again goes past a bubble's frame;
  - Enter opens only links that are on screen;
  - after `L` or `a`, a cursor key moves it.
- A scrolled code block in an issue no longer crashes debug builds.
- `‹` and `›` don't cover a jump label, or shift a line with wide characters.
- Dropping one file's changes takes its name literally.
- Only http(s) links are opened.

## [0.58.1] - 2026-10-20

### Fixed
- When only uncommitted changes are shown, a commit, an amend or a drop that leaves nothing
  uncommitted shows the whole diff again, not an empty view.

## [0.58.0] - 2026-10-20

### Added
- `Space g` → drop what isn't committed, all of it or the cursor's file. It's stashed first,
  so `git stash pop` brings it back.
- `‹` and `›` mark a line cut short with more to scroll to: diff lines in a PR, code blocks
  and tables in an issue.

## [0.57.0] - 2026-10-20

### Added
- An issue in dv has a cursor, as in Helix:
  - `h j k l`, `w b e`, `gh gl`, `gg ge`, and `Tab` to the other side;
  - `v` and `x` select, `y` copies;
  - Enter opens the link under it.

### Fixed
- A deferral until another PR gets a push also ends when that PR is merged or closed.
- An issue can only wait on its own fix PRs.
- "…, or 2 weeks" can't crash on non-ASCII text.

## [0.56.0] - 2026-10-20

### Added
- `dv issues`: `s` then `u` defers an issue until a fix PR merges, closes or gets a push
  ("the author makes progress": its open fix PR's push), or a time.
- Either list: "…, or 2 weeks" ends the deferral at whichever comes first. A PR can wait for
  another PR's push ("#2258 gets a push").

### Fixed
- Reading waits from comments:
  - "Waiting on CI, then review" and "Waiting for me to review" aren't someone else's review;
  - bold markers read wherever the bold is;
  - "#12 until it's approved" is #12's.

## [0.55.1] - 2026-10-20

### Fixed
- Reading waits from comments:
  - "Waiting for the review" (or your, code, maintainer review) waits for someone else's
    review again;
  - "**Blocked on** #12" reads;
  - "Until it's updated, LGTM" doesn't end an older wait.

## [0.55.0] - 2026-10-20

### Added
- In a PR's activity, a review says how many comments it left on the code, and, if it was of
  an older commit, which one.

### Fixed
- Reading waits from comments:
  - "Until then, LGTM" doesn't hide an older "Blocked on #1";
  - "waiting only for CI" isn't "waiting on";
  - "the review comments to be fixed" is the author's to do;
  - "@bo's review" is bo's.

## [0.54.2] - 2026-10-20

### Fixed
- A PR comment saying "Waiting for reviews" waits for someone else's review again. The newest
  "waiting on" line decides, even when dv can't read it; it no longer falls back to an older one.

## [0.54.1] - 2026-10-20

### Fixed
- Deferring "until … review …" means someone else's review only when it says so
  ("someone else", "anyone"). "review it in 2 days" is a time, and the rest go to Claude.

## [0.54.0] - 2026-10-19

### Added
- Defer until someone else reviews it: a new review from anyone but you and the PR's author
  brings it back.

## [0.53.4] - 2026-10-19

### Fixed
- An unpublished replacement can't be deferred until something of its PR (green, a
  review, a push): there's none yet. dv now says so up front.

## [0.53.3] - 2026-10-19

### Fixed
- Waits and deferrals until a review count reviews from when they were said: a comment's
  wait from the comment; one Claude found from when it was asked to rank. A deferral waits
  for the PR's reviews to be known first.
- A published replacement's item waits on its own PR (reviews, pushes, checks), not on the
  PR it replaces.
- A sideways scroll left over a short line stops at the line's end instead of jumping to
  column 0.

## [0.53.2] - 2026-10-19

### Fixed
- A deferral until a review or approval reads each review, not just everyone's latest (a
  comment after an approval no longer hides it). It counts reviews newer than what dv had
  shown you, by GitHub's clock.
- A sideways scroll left works over a short line.
- A published replacement's card shows its PR's conflicts.

## [0.53.1] - 2026-10-19

### Fixed
- Sideways mouse scroll:
  - in a PR, it doesn't flip direction over a short line or jump to the top from the pinned
    title, and Shift+wheel scrolls popups and pickers again;
  - in an issue, only the blocks wider than shown move.
- `dv scratch`:
  - replacement drafts may edit their scratch checkouts;
  - the Claude pane may make one without asking;
  - `--rm` removes only a whole checkout;
  - git's records of your other worktrees are left alone.

## [0.53.0] - 2026-10-19

### Added
- A PR that conflicts with its base says so on its card in every tab (drafts too) and in the
  PR view ("conflicts with main"), not just as a need on the todo tab.

### Fixed
- A deferral until someone reviews (or approves) a PR counts only reviews submitted after it
  was deferred, not ones the PR already had.

## [0.52.0] - 2026-10-19

### Added
- `dv scratch <rev>`: Claude gets a throwaway detached checkout to test in (does a test fail
  without the fix? does the base build?), so it can find out instead of asking. Old checkouts
  are removed after a day; `dv scratch --rm <dir>` removes one now.

## [0.51.0] - 2026-10-19

### Added
- The mouse scrolls code sideways (a sideways wheel, or Shift and the wheel). In a PR, it
  scrolls a comment's code block or table, or the diff, under the pointer. In an issue, it
  scrolls the code blocks and tables on that side.

## [0.50.1] - 2026-10-19

### Fixed
- An issue in dv: a drag copies the text as drawn (tables keep their borders, code its
  indentation; no bubble frames), a click leaves nothing selected, and a drag past the
  view stops at its last line.
- `dv issues`: `T T` and `X X` on someone else's issue say they take triage access, as `L L` does.
- Every replacement draft counts as under way from its start, so auto doesn't start a second one.

## [0.50.0] - 2026-10-19

### Added
- An issue in dv: a drag copies the text it's over (the description's or the pane's,
  without the bubbles' sides).

### Fixed
- `dv issues`: no labelling without triage access on the repo. Claude's labels show as
  suggestions and `L L` says why, instead of failing on GitHub.
- Auto: closing comments and replacements skip a PR that Claude is busy with, one put aside,
  waited on, or being closed, and your own PRs. One replacement at a time, including other
  dvs' drafts. Issues' closing comments skip deferred issues.

## [0.49.1] - 2026-10-19

### Fixed
- An issue in dv: a code block's background runs its full width, not just under its text.

## [0.49.0] - 2026-10-19

### Added
- Claude's review of a PR may say to replace it (`"replace"`: low effort, AI slop, but what it's
  after is worth having), and auto then drafts that replacement: as `A A` does, unpublished, on
  the drafts tab, one at a time, once a head, with your instructions for replacements and why.

## [0.48.0] - 2026-10-19

### Added
- Auto drafts closing comments: for a PR Claude's review would close (`dv list`), and an issue
  its triage or ranking would close (`dv issues`), Claude drafts the comment it closes with,
  once each, ready for you: the PR's view (`Space a` → close with Claude's comment) or the
  issue's (`X X`) closes with it, after your editor.

## [0.47.1] - 2026-10-19

### Fixed
- `dv issues` with auto on counted every issue as not triaged as it started (the triages
  weren't read yet).
- The issue view: a long line (code, a table, Claude's next step) stays in its bubble, and a
  wide code line in the description doesn't push the pane aside; a comment with no time says
  none; a closing comment drafted for an issue not triaged shows as that, not as its triage.
- An issue on the close tab for its ranking's say-so shows why on its card. A re-triage saying
  to keep an issue open no longer gets an older closing comment back.

## [0.47.0] - 2026-10-19

### Changed
- An issue in dv is laid out as a PR is: its description on the left, and a pane beside it with
  Claude's triage in its `◆ claude` box, then the conversation, a bubble a comment (one column
  when the window's too narrow for both).

### Fixed
- The close tab of `dv issues` was empty while Claude's ranking said to close issues it hadn't
  triaged: the ranking now says which to close (`r r` again), and the tab lists them too.

## [0.46.1] - 2026-10-19

### Fixed
- `dv issues` with auto on (`dv.auto`, set from dv list too) says as it starts how many issues
  Claude's about to triage, and what it costs; `C` turns it off.
- A closing comment Claude drafted on an issue not triaged doesn't count as its triage (auto
  triages it still), and a triage landing after it keeps it. A draft that comes back empty, or
  whose run dies, says so (it said "drafting" for good); `q` asks again while one's under way.

## [0.46.0] - 2026-10-19

### Added
- An issue in dv: `X X` with no closing comment of Claude's yet has Claude draft one (why, how it
  closes: completed, not planned, a duplicate), then `X X` closes with it, your editor first.

### Changed
- An issue in dv has its own title bar (`Issue #N · title`, who, labels), not the list's tabs,
  and shows Claude's triage first, before its text and conversation.

## [0.45.0] - 2026-10-19

### Changed
- `dv list` and `dv issues` draw the same top and status bars (which list, tabs, the other one,
  where you are; what's said, the order or ranking, auto on / off) and the same keys popup.
- Auto in `dv issues` triages every issue not triaged, the backlog too (turning it on says how
  many, about $0.30 each), and those changed since their triage.

### Fixed
- `dv issues`: the cursor stays on its issue as the todo reorders (a ranking, a triage landing);
  no ranking in the same moment as a triage, nor as it starts; a click below the last card drawn
  does nothing; Claude's news doesn't talk over a question or what you just did.

## [0.44.1] - 2026-10-19

### Added
- The mouse in `dv issues`, as in `dv list`: a click on a tab shows it, on a card selects it, a
  second click opens it; the wheel moves the cursor (scrolls an open issue).

## [0.44.0] - 2026-10-19

### Added
- Auto mode in `dv issues` (`C`, the same `dv.auto` as dv list's): Claude triages issues that
  are new or changed since dv started, and those changed since their triage, two at a time, and
  ranks the todo again once it's changed (at most every quarter hour). The backlog already
  there untriaged is left to you (`a a`), so turning it on isn't a bill for all of it.

## [0.43.1] - 2026-10-19

### Fixed
- The issue view: `o` opens its own link (not an empty one once it's left the list); it doesn't
  scroll past its end after a resize or a reload; `#123` links as elsewhere; it says it's
  loading. Closing goes by how Claude's triage says (completed, not planned, or a duplicate of
  #N, as GitHub marks it), not by its wording. Labels are added a flag each.
- `q` in dv issues asks again while Claude drafts a fix too (it was lost).

## [0.43.0] - 2026-10-19

### Added
- `dv issues`: Enter opens the issue in dv: its text and comments, then Claude's triage (its
  summary, what next, the labels it would add, a better title, why to close it and how). From
  there: `C` comments, `L L` adds Claude's labels, `T T` gives it Claude's title, `X X` closes it
  with Claude's closing comment (your editor first), `N N` has Claude draft a PR fixing it (on
  dv list's drafts tab), `a a` triages it again, `o` opens it in the browser.
- Claude's triage may suggest a clearer title.

## [0.42.1] - 2026-10-19

### Fixed
- `dv issues`' todo: deferrals are kept (they were dropped as it started, and lost once
  another was saved), and come back when their time's up without waiting for a refresh.
  `r r` doesn't start a second ranking while one's under way, and ranks only what isn't
  deferred. "Changed since Claude triaged it" goes by the issue's own update time when it was
  triaged, not when the triage was written.
- The issues ranking no longer asks Claude what an issue waits on (dv watches that for PRs).

## [0.42.0] - 2026-10-19

### Added
- `dv issues`: a todo tab, first. What needs you of an issue: the PR fixing it merged (close
  it), Claude would close it, assigned to you, a PR would fix it (review that), changed since
  Claude triaged it, not triaged. `r r` has Claude rank them (one reading-only run: why and
  what next, in the message column as on dv list's todo), `s` defers one (an hour, four, a day,
  a week; back sooner if what it needs changes).
- Each list's top bar says which it is (`PRs`, `Issues`), and `I I` the other.

## [0.41.4] - 2026-10-19

### Fixed
- An issue's triage follows your notes on the repo again (in its prompt: a reading-only run
  can't reach dv's directory to read them).
- Ranking, grouping and reading a deferral find Claude's JSON as the triage does (text after
  it, like "(see [1])", no longer breaks them).
- The filter's smart case goes by what you're looking for, not the key: `Label:bug` matches
  `Bug`.

## [0.41.3] - 2026-10-19

### Fixed
- Claude's runs that only read (triaging an issue, ranking the todo list, grouping PRs for a
  replacement, reading a deferral's "until") refuse anything not allowed. They took your Claude
  Code default mode, which under `auto` let them edit files; ranking, grouping and deferrals
  were allowed edits outright.
- An issue's triage is found in Claude's answer however it wraps it; when a run fails, its own
  word for why is on the card. The base it reads is the repo's default branch as dv finds it
  everywhere else.
- Filter keys in any case (`Label:bug`).

## [0.41.2] - 2026-10-19

### Changed
- `dv list` and `dv issues` share their filter (smart case in both now), editor, help popup
  and how Claude's runs are queued, so they behave alike.

## [0.41.1] - 2026-10-19

### Fixed
- `dv issues`: Claude's triage only reads (no edits allowed: its answer is the triage, which
  dv keeps), and reads the code as it is on the base branch. A run that answers no triage fails,
  said on its card; a finished one says what it took.
- `q` there asks again while Claude's triaging, and doesn't quit with Claude still at work on
  PRs from `dv list` (back there instead). `a a` skips issues Claude's at already.
- The wheel moves the cursor; a lone `-` in the filter is text; Esc puts the cursor at the top.

## [0.41.0] - 2026-10-19

### Added
- `dv issues` (or `I I` in `dv list`, and back): the repo's open issues as cards (title, labels,
  comments and 👍, who opened it, who has it, the PRs that would close it), tabs (open, mine,
  can close), `/` filters (`label:`, `author:`, `kind:`, `priority:`, `is:triaged`). `a a` has
  Claude triage the marked issues (or this one), reading only: what kind, how pressing, whether
  it still holds (with `path:line`), the labels that fit, what next, and whether to close it
  (with a closing comment to edit), shown on its card. Enter opens it on GitHub for now; dv's
  own view of an issue, with replying, labelling and closing, comes next.

## [0.40.1] - 2026-10-19

### Fixed
- Committing never runs while Claude is at work in the checkout, however it's reached (an editor
  pane closing late, a reply sent meanwhile: that one waits for Space a).
- Claude's commit message is kept when your editor can't open with it then (a menu open): `Space g`
  → commit opens with it, no second run. It's written on a fork of the PR's session, so the
  review's conversation doesn't carry it.

## [0.40.0] - 2026-10-19

### Added
- `Space g` → "Commit everything that isn't": Claude writes the commit message first (in the
  background, from the changes, in the repo's style as `git log` shows it), and your editor
  opens with it. Without a review file, or if it couldn't, the message starts from Claude's
  "Changed: …" replies as before.

## [0.39.3] - 2026-10-19

### Fixed
- Claude's review read before answering is shown at once (its rows rebuilt with it; read once,
  not three times). Enter on "(no changes)" says there's nothing there.

## [0.39.2] - 2026-10-19

### Fixed
- Enter on a file's header opens it in your editor again, and on a comment with no link,
  `file:line` or name under the cursor, the code it's on.
- A reply asking for an amend or a commit while Claude's pane is at work waits for it (it went
  ahead: the reply wasn't read yet). One reply to several comments says why one failed.
- Searching the help keeps an entry's lines together.

## [0.39.1] - 2026-10-19

### Changed
- `dv list`'s help (`?`): in sections (move, find, todo, Claude, PR), a line a key, styled as
  dv's (headings, keys in their color, notes dim), in two columns when the window fits them.

## [0.39.0] - 2026-10-19

### Changed
- Enter does what `gf` did, off code: the link, `file:line` or name under the cursor (on code,
  as before: the file in your editor). `gf` is gone; the hints say Enter.

### Fixed
- A new comment for Claude left empty is cancelled again (only a reply's empty editor means
  "what Claude's comment says", and the editor says so).
- One reply to several comments: a comment gone meanwhile doesn't stop the others being
  answered (said which).
- Making the changes: a comment's own `want` (amend, commit) wins over how the run lands them;
  on someone else's PR, made here, they land as commits by default (what `Space g` pushes).
- An amend or commit asked for while Claude's pane is at work in the checkout waits for it.

## [0.38.1] - 2026-10-19

### Changed
- Keys are said in one place, the key hints: a link's tooltip shows only where it goes, and the
  hints don't list `gf` where Enter does the same.

## [0.38.0] - 2026-10-19

### Added
- Replying to Claude's comment (`r`, or `a`), two more things to want: `Amend` (Claude makes
  the change and amends it into the branch's commit it belongs in, through `dv amend`) and
  `Commit` (the change as a new commit). For those and `Change`, the editor may be left empty:
  "what your comment says".
- With a selection over several of Claude's threads, `r` (or `a`) is one reply to them all: one
  picker, one editor, one run.
- `Space a` → make the changes asks how they land: left uncommitted, each amended into the commit
  it belongs in, or as new commits (the last answer first).
- Code lines' key hints, `Enter` first (opens the file in your editor at that line).
- A PR named in brackets before "until" counts for a deferral ("Blocked (see #12) until it's merged").

## [0.37.2] - 2026-10-19

### Fixed
- A deferral's sentence: only what follows "until" says when ("Don't approve until the release"
  isn't taken for an approval: Claude reads it), though a PR named before it counts ("#12, until
  it's merged"). A push's head is only the word after "past", as `p` writes it (a word like
  "defaced" brought the item straight back).

## [0.37.1] - 2026-10-19

### Fixed
- Deferring until the author pushes: `p` posts the head it waits past ("the author pushes (past
  abc1234)"), so a comment saying so clears once they push (it held the PR back for good).
  Not deferred while the PR's head on GitHub isn't known yet.
- "Blocked on #12 until it's merged", "CI to pass until Friday" read as before 0.37.0 (what
  follows "until" is tried first, then the whole); "@amy reviews the fixes" is her review, and
  "3 days after the push" a time, not a push.

## [0.37.0] - 2026-10-19

### Added
- Defer a todo until the author pushes: "until the author addresses the reviews", "pushes",
  "updates it". It's back once the PR's head moves on. A sentence ("Can't do anything until
  …") is read from its last "until"; Claude, reading the rest, knows this one too. It was
  refused: dv had no way to watch for it.

## [0.36.2] - 2026-10-19

### Fixed
- The todo column: a why with a dash in it and no next step isn't split at the dash any more
  (one ranked before 0.36.1 shows whole, till ranked again). A narrow window's cards keep their
  number too, however many badges.

## [0.36.1] - 2026-10-19

### Fixed
- The todo column: a dash in Claude's why no longer turns its end into the `→` next step
  (rankings from now on; re-rank with `r r` for those before). A title with many tags keeps
  its PR number.
- A base whose protection can't be read is asked again after 30 seconds, not once for each of
  its PRs on every refresh.

## [0.36.0] - 2026-10-19

### Changed
- `dv list`'s todo tab, in a wide window (150 columns or so): Claude's word on each card goes in
  a column of its own beside it, over four rows, not cut off: why it's there, wrapped, then
  `→ what to do` in the accent colour. The card's place in the order sits under its number, what
  holds the PR up (`[✗ 1 check failing · 1 unsigned commit]`) on its title line. The keys popup
  stays out of the column's way (`?` shows the keys). Narrower, as before: a line under the title.

## [0.35.1] - 2026-10-19

### Fixed
- `dv cargo` runs where it's run (a crate of its own in a subdirectory), not at the repo's root.
- "can't read which its base requires": not on closed PRs, nor for minutes after one failed
  read (it's asked again), nor for a base whose name has `#`, `?` or a space.
- A PR whose checks GitHub refused says GitHub's reason again, not "said nothing".

## [0.35.0] - 2026-10-19

### Added
- `dv cargo [+toolchain] …`: cargo `check`, `build`, `test`, `clippy`, `doc` or `fmt --check`
  on any toolchain, nothing else. Claude may run it, so it can check a PR as CI does on a
  pinned toolchain (`dv cargo +1.85.0 clippy`), which a permission rule can't name.

### Fixed
- The checks a PR's base requires by its branch protection: read over REST, which answers
  anyone who can read the repo (GitHub's GraphQL gives them only to its admins), so one not
  reported yet shows as expected there too. Neither readable: the checks line ends
  `· can't read which its base requires`.
- A PR GitHub refuses in a batch of checks (`dv list` asks 40 at once) leaves the other 39's
  checks; only that one's are missing.

## [0.34.99] - 2026-10-19

### Fixed
- Writing in the editor's pane: its pane closed from zellij is noticed whatever zellij can
  list (the shell running the editor notes its pid; gone, the pane's gone), so dv no longer
  waits on it forever when zellij couldn't say. What you'd written is kept, as before.

## [0.34.98] - 2026-10-19

### Fixed
- A Claude comment that moved with its code, then went outdated, then came back: on its lines
  as they are now (0.34.97 could give it the moved start and the old end).

## [0.34.97] - 2026-10-19

### Fixed
- A Claude comment whose code came back (an amend undone, while its review file couldn't be
  read) shows on it again, not still outdated.

## [0.34.96] - 2026-10-19

### Fixed
- Claude's review read while it was half written could stay "can't be read" after Claude
  finished; and what's kept shown meanwhile follows its code as the diff changes.
- Amending refuses conflicted files when dv runs in a subdirectory too.

## [0.34.95] - 2026-10-19

### Fixed
- Claude's review vanishing ("Claude hasn't reviewed this") while its file couldn't be read
  (Claude mid-edit, a slip in its JSON): what was last read of it stays shown, and dv says
  why it can't read the file; it updates once the file is fixed.
- Amending with conflicted files is refused before anything changes (files marked with
  `git add -N` came back added); git's reason is its last line when it says it on stdout.

## [0.34.94] - 2026-10-19

### Fixed
- An amend that put nothing in leaves new files marked with `git add -N` as they were (they came
  back fully added), and says why when git's reason was on its standard output.

## [0.34.93] - 2026-10-19

### Fixed
- `dv amend -F` with an empty file is refused (it left the commit with no message).
- An amend whose changes came to nothing is reported as such again, not as amended.
- Claude may run `cargo fmt -- --check` (and `+nightly`), as CI often spells it.

## [0.34.92] - 2026-10-19

### Added
- `dv amend <commit> -F <file>`: that commit's message replaced by the file's (with files too,
  to fold them in the same go). Claude rewords a commit with it, rather than asking you to.

### Fixed
- Claude may run the fmt check and clippy as CI does (`cargo +nightly fmt --check`, `cargo
  +nightly clippy`): they were refused. The skill says to run one plain command per call, as
  `a && b`, loops and `git -C` need each part allowed.
- `-b`: the PR's branch is recognised by where `Space g` pushes it (its push remote; a URL, as
  `gh pr checkout` sets one for a fork), and dv says when it has diverged from the PR's head.
- Amending: files marked with `git add -N` are found when git would take them for a rename,
  and added by their literal names (a `*` in one added others).

## [0.34.91] - 2026-10-19

### Fixed
- `-b` (as `dv list` opens PRs): a PR whose branch is checked out in another worktree with
  commits not pushed yet (a rebase, an amend: a replacement's, say) opens there, as it does
  when it's only ahead; it failed to open. The branch is the PR's when it pushes to the PR's.
- Amending: new files only marked (`git add -N`, as Claude does) are added for real first:
  they stopped the amend (git can't stash them), and came back untracked. A commit whose
  changes were left out is named by its subject (its sha may have changed meanwhile).

## [0.34.90] - 2026-10-19

### Fixed
- Amending: a commit whose changes conflict no longer untracks staged new files a later commit
  takes (that left a rebase half begun, refusing every amend after); its changes stay staged.
  Why one was left out says so plainly ("a later commit changes those lines", else git's error).
- `dv amend`'s files with `diff.relative` set.

## [0.34.89] - 2026-10-19

### Changed
- The pane's tabs count an outdated thread as done, as a resolved one is (both are shown
  folded): `Claude 7/8` with one thread left you can see open, not `6/8`.

## [0.34.88] - 2026-10-19

### Fixed
- Amending into the PR's commits: one commit's changes conflicting (a later commit changes those
  lines) no longer undoes them all. Each commit is amended on its own, newest first; one that
  conflicts keeps its changes uncommitted, and dv says which (`f` on a later commit folds them).
- Amending no longer needs nothing staged, and never unstages: the fixup commits are made in an
  index of dv's own. (0.34.87's `dv amend` unstaged everything first, a conflicted file too.)
- `dv amend`'s files are resolved by git: `../a`, a directory, a glob.

## [0.34.87] - 2026-10-19

### Fixed
- `dv amend`: only into the branch's own commits (not the base's, which it would have rewritten),
  with changes staged and new files in, and file names as given from a subdirectory.
- "Open Claude in a pane" while it was at work: that copy is no longer taken for the PR's pane
  (asks pasted into it, closed while you talked to it, later runs forked).
- dv's own reads (`git status` every few seconds) don't take git's index lock, so a commit or
  rebase at work in the same checkout doesn't fail on it.
- The ref a new PR worktree is made from is dropped once it's made.

## [0.34.86] - 2026-10-19

### Added
- `Space a` → "Open Claude in a pane": the PR's Claude session in a floating pane, focused, to
  talk to it when none is open; while a background run is at work on it, a fork of it.
- `dv amend <commit> [file…]`: what isn't committed (in those files) folded into that commit of
  the branch, as `f` does, a backup branch first. Claude amends through it.

### Fixed
- "Amend it all into the commits Claude finds it belongs in" did nothing when Claude was refused
  a command on the way (a `jq` to read its answers): its placements are amended now.
- Claude commits only in runs that answer you or make changes (not in reviews or CI fixes), and
  amends through `dv amend` (merges kept, other branches untouched) rather than a hand rebase;
  dv's amend won't run over a rebase stopped part way.
- A new PR worktree is made from a ref of its own (two prepared at once could swap commits),
  and recorded as dv's checkout (a PR force-pushed after a failed checkout wasn't taken as work).

## [0.34.85] - 2026-10-19

### Fixed
- "Do it, amend it into the relevant commits": Claude couldn't (background runs weren't allowed
  `git add`/`git commit`), and left the change uncommitted. It may now commit, and amend a
  change into the commit it belongs in (`git commit --fixup`, `git rebase --autosquash`); never push.
- A worktree dv creates for a PR starts at the PR's head, not the main checkout's commit: one
  left half set up (the checkout failed, dv was killed) is never taken for work done there.

## [0.34.84] - 2026-10-19

### Fixed
- A PR whose checkout failed (its branch checked out in another worktree, say) no longer leaves
  a worktree at the main checkout's commit: opened again, dv took that commit's history for
  work done there ("kept: Space g pushes them"), and pushing it emptied the PR, which GitHub
  closed. Work done here is now only commits on neither the PR nor its base; and dv won't push
  a head with no commits over the PR's base (GitHub would close it).

## [0.34.83] - 2026-10-19

### Fixed
- 0.34.82 as it should have been: it didn't build.

## [0.34.82] - 2026-10-19

### Fixed
- Claude's activity: an item ending in blank lines isn't marked `…`; the cursor stays on its
  comment when one before it in its thread is deleted.

## [0.34.81] - 2026-10-19

### Fixed
- Claude's activity: a long word (a path, a URL) doesn't run past the box; `…` says when Claude
  said more than its first paragraph; the cursor stays on its comment as newer ones come in
  above it (it kept its place in the box, on another).

## [0.34.80] - 2026-10-19

### Changed
- Claude's activity is readable: each item says when and where (the file's name and line), dim,
  then what Claude said, wrapped over two lines (it was one line, the path taking most of it).

## [0.34.79] - 2026-10-19

### Fixed
- Claude's review, the PR's facts and activity sit in the pane again when there's no
  description beside them (a branch Claude reviewed; a PR whose text hasn't loaded): 0.34.78
  put them in the code's column.
- Right after publishing a replacement, its closing note shows once (it could show twice: the
  draft's and the PR's).

## [0.34.78] - 2026-10-19

### Fixed
- `+` / `-` in the Claude or pending view on a file shown whole (`Z`) go by what's on screen.
- A Claude review whose summary is only whitespace doesn't title the view "Claude's review"
  over nothing.

## [0.34.77] - 2026-10-19

### Fixed
- Enter (or `gf`) on a line of Claude's activity goes to that comment, wherever it is (a thread
  on code, on its review, on a commit; outdated) and wherever the cursor is on the line (`gf`
  read the text under the cursor: only its `path:line` worked, and an outdated one's line
  was the wrong one).

## [0.34.76] - 2026-10-19

### Fixed
- Claude's activity: its lines' keys are its own (`gf` goes to it), not the PR text's; a thread
  on code that's changed says the line it was on; it shows when Claude's review has no summary.

## [0.34.75] - 2026-10-19

### Added
- The Claude tab has Claude's activity, as the Comments tab has the PR's: the last 5 things it
  wrote (comments, answers, changes it made), a line each: when, where, its first line.

## [0.34.74] - 2026-10-19

### Fixed
- A PR's checks are weighed as its other facts are: GitHub refusing part of them fails the
  fetch, rather than reading as no checks (which marking it ready took for passing).

## [0.34.73] - 2026-10-19

### Fixed
- dv list's facts on each PR (signatures, reviews, review requests) are weighed as a PR's are:
  only a base's rules may be refused; anything else refused fails the fetch, rather than a
  refused signature reading as unsigned or a refused review as none (since 0.34.72).

## [0.34.72] - 2026-10-19

### Fixed
- A GitHub answer with part of it refused (a base's branch protection you can't read, one PR
  not found) is taken for what it has: gh's failing exit failed it all, so CI checks (in the
  PR, on dv list's cards, marking ready) and dv list's signatures failed outright for some
  viewers. A batch of checks GitHub fails on leaves the others'; a required check named twice
  (a ruleset and branch protection) is expected once.

## [0.34.71] - 2026-10-19

### Fixed
- A PR's checks, its head and what its base requires (rulesets and branch protection) come in
  one GitHub answer: a required check past GitHub's first 100 isn't taken for one not yet
  reported, nor is one on a closed PR; branch protection's not yet reported count as pending
  too; a card's counts are on the head they say; one PR GitHub fails on keeps its old counts
  rather than blanking every card's; dv list's checks are fetched a few batches at once.

## [0.34.70] - 2026-10-19

### Fixed
- Which checks are required is GitHub's word for each (`isRequired`): branch protection's count
  too without admin rights, and nothing kept from a failed look. A required check that hasn't
  reported yet counts as pending, so a draft isn't marked ready (nor shown green) before it runs.
- "Draft a replacement" from a PR's view isn't refused while Claude works on that PR there.
- A pane's "Claude working" doesn't linger once its session is gone (it held back commits).

## [0.34.69] - 2026-10-19

### Fixed
- Claude at work in a replacement's pane (zellij: dv's, or one you started there) holds back a
  redraft from dv list or from dv, and dv's commit, amend, rebase and publish (only its
  background runs did); asking Claude more still goes to its pane.
- A redraft marks done what you asked that it was given, by text, whatever Claude did to the
  record's order.
- Menus say "Claude is at work on it", not "drafting", while dv's own run holds a replacement.

## [0.34.68] - 2026-10-19

### Changed
- CI is judged by the checks its base requires (its rulesets, its branch protection), when it
  names any: an optional one failing (codecov's, say) no longer stops marking a draft ready,
  dv list's "checks green", or sends Claude to fix CI. The PR's status counts required and
  optional checks apart (`required ✓ 7 passed · optional ✗ 2 failed`; a failing optional check
  says `(optional)`); dv list's cards count the required ones, `· ✗2 optional` beside them.

## [0.34.67] - 2026-10-19

### Changed
- The pane's tabs count resolved threads only: an outdated one isn't done (its code changed;
  its point may stand).
- Showing only what isn't committed: no PR text before the code either, as with some commits.

### Fixed
- dv's own Claude run in a replacement (answering you, a change) is marked in its record as a
  redraft is: dv list, or a redraft from elsewhere, won't start one in its checkout meanwhile.
- A change asked of a replacement's drafts that a finished redraft took in without saying so is
  done (it was asked again each redraft); dv's note for it shows as dv's, not as Claude's answer.
- A deferral Claude reads the time of is checked against the cursor before other news moves it.

## [0.34.66] - 2026-10-19

### Fixed
- `dv list`: a deferral Claude reads the time of leaves the cursor on the card it's on when
  you've moved on meanwhile (0.34.65 let the list shift under it); on the deferred card, it stays
  put, on what's next.

## [0.34.65] - 2026-10-19

### Fixed
- `dv list`: back from a PR, the cursor is on the card it was on (the list may have changed
  meanwhile); a deferral Claude reads the time of ("after the release") leaves it on what's next,
  as one you type.

## [0.34.64] - 2026-10-19

### Fixed
- `dv list` keeps the cursor on its card through whatever comes in (a draft changing, a PR
  coming back, back from a PR), not only some of it: it's taken before any of it changes the
  list; and finding the card again is one pass (it was quadratic, worse on the todo tab).

## [0.34.63] - 2026-10-19

### Fixed
- `dv list`: deferring the todo item under the cursor leaves the cursor on what's next (0.34.62
  took it along); one rule keeps the cursor on its card through a refresh, a sort and the todo
  list changing, a replacement's draft told apart from the PR it replaces.

## [0.34.62] - 2026-10-19

### Fixed
- `dv list`'s todo tab keeps the cursor on the PR it's on when the list changes under it (a
  review landing, CI finishing, Claude done), so the next key acts on that PR, not another.

## [0.34.61] - 2026-10-19

### Fixed
- `dv list` takes in a review Claude writes while you're on it (from a PR's view, another dv):
  what's new, the todo tab and the can close tab follow at once, not at GitHub's next refresh.

## [0.34.60] - 2026-10-19

### Fixed
- `dv list`'s can close tab: its cards' height counts the line saying why (the last was cut
  off, clicks picked the wrong card); it's up to date as soon as Claude's review is (it waited
  for GitHub); a reason from a review of an older head says so; the tab is "close N", so the
  top bar keeps every tab's count.

## [0.34.59] - 2026-10-19

### Added
- `dv list`'s "can close" tab (after open): the PRs Claude's review would close, and why, under
  each card's title: its premise is wrong, or a competing PR does it better. Claude says so in
  the review file's new top-level `"close"` (the skill asks it to when it would).

### Fixed
- The title's thread counts give way to the pane's tabs only where the tabs are drawn (on a
  pinned title), not when there's none.

## [0.34.58] - 2026-10-19

### Changed
- One count per thing: the pane's `Pending` tab counts your pending comments as the title does
  (not the threads they're in), and with the pane's tabs shown, the title doesn't count the
  threads again.

### Fixed
- The pinned title, with its counts, Claude's badge and the checkout's beside it, could lose
  its text: whether the counts fit now counts the badges too.

## [0.34.57] - 2026-10-19

### Changed
- The pane's tabs count a thread on code that's changed since (outdated) as settled, as a
  resolved one: as dv folds them, and the files' open counts leave them out.

## [0.34.56] - 2026-10-19

### Changed
- The pane's tabs count resolved threads, of all (`Comments 1/3`, `3/3` when all are), as the
  title counts files viewed (0.34.55 counted open ones).

## [0.34.55] - 2026-10-19

### Changed
- The pane's tabs say how many of their threads are open when some are resolved
  (`Comments 2/3`).

### Fixed
- The key hints' popup stays below the pinned title in a short window.

## [0.34.54] - 2026-10-19

### Changed
- `dv list`: `gn` / `gp` go to the next / previous tab, as in dv (it was `Tab` / `Shift-Tab`).

## [0.34.53] - 2026-10-19

### Fixed
- Popups shade only code blocks to their edge: not a line ending in inline code, in a theme
  giving inline code the code blocks' background.

## [0.34.52] - 2026-10-19

### Fixed
- Hover docs' code blocks: shaded to the popup's edge as it's drawn, so a popup stays as wide
  as its text (0.34.51 made any with code as wide as the window), and blocks in a list or a
  quote are shaded too.

## [0.34.51] - 2026-10-19

### Fixed
- Code blocks in hover docs (`Space k`) have their background to the popup's edge, as in a
  comment's box, not only behind their text.

## [0.34.50] - 2026-10-19

### Fixed
- `a` on an unpublished draft quotes only its text: not the line saying what it replaces, nor
  "(not drafted yet)" in place of one.

## [0.34.49] - 2026-10-19

### Fixed
- `a` on the line saying what an unpublished replacement replaces (or what a new PR does)
  redrafts its description again (0.34.48 said "no PR here"): a draft's row is its section's.
- Inline code at the start of a Markdown line (```` ```foo``` bar ````) isn't taken for a fence.

## [0.34.48] - 2026-10-19

### Fixed
- Code blocks in files: an empty one no longer puts the rest of the file's out of step, and one
  closes only on a fence like its own (a ```` ```` ```` block can hold ```` ``` ````), as
  CommonMark has it; a fence's language is read one way in files and in comments alike.
- What you asked of a replacement's drafts: a question says "Claude is answering it" / "not
  answered yet" while it waits; `a` on the line saying what it replaces doesn't quote it as the
  draft's text.

## [0.34.47] - 2026-10-19

### Added
- Code blocks in Markdown files and in Rust doc comments (`///`, `//!`) are highlighted in
  their language: a fence's first word (`rust,no_run` is Rust), and in doc comments, as rustdoc
  has it, Rust when it names none (or only `ignore`, `no_run`, ...).

### Fixed
- What you asked of a replacement's drafts: each in a box of its own (several on one draft ran
  together); `a` on one doesn't quote it as the draft's text; Tab on one beside the closing note
  folds the closing note (not the description); an answered question says "answered".
- `a` on a published replacement's closing note says `S` closes what it replaces with it.

## [0.34.46] - 2026-10-19

### Added
- `a` on a replacement's drafts (or on what you asked of them) asks what you want first, as on
  a PR's comments: `Ask` (Claude answers; it changes nothing) or `Change` (it redrafts).

### Fixed
- `a` on what you asked of a replacement's drafts, or Claude's answer, asks about that draft
  (it said to select code lines).
- A request of the drafts is done once Claude has answered it, so one Claude's redraft skipped
  or reordered is asked again (it was marked by its place in the list).
- Folding a replacement's closing note folds what you asked of it too.
- A published replacement's PR shows its closing note (what `S` closes the PRs it replaces with)
  while any of them is open.

## [0.34.45] - 2026-10-19

(0.34.44 again, with its tests passing on CI: it isn't on crates.io.)

## [0.34.44] - 2026-10-19

### Changed
- What you ask of a replacement's drafts is in the pane, beside the draft it's about (as a PR's
  comments are), not a section in the code's column.

### Fixed
- A question asked of a replacement's drafts gets an answer: the redraft writes what it did,
  or its answer, on each request, shown under it (it went only to Claude's log).
- Publishing a replacement shows the PR at once (GitHub's text, comments and CI), not the draft
  until something changed on GitHub; what you've seen is the PR's.
- Opening a published replacement again keeps its record (closing what it replaces, Claude's
  redraft counted as at work), not only the first time.
- Claude's review of a draft moves to the PR only once GitHub has answered (offline, it stays
  where dv finds it), and for any dv on that PR, `dv` in its worktree too.
- Publishing waits for Claude at work in the checkout (it rebases and pushes the branch).
- A PR checked out on a `dv/replace-N` branch takes no local record that isn't that PR's.

## [0.34.43] - 2026-10-19

### Changed
- Showing some of the commits: only their messages come before the code (and in the contents);
  the PR's description, what's attached to it and Claude's drafts are about all of them, and
  come back with the whole diff.

### Fixed
- A replacement's diff follows its base as a PR's does: after a redraft or publishing rebases
  it, the diff is its own commits again, `Space g` offers rebasing it, and amending never folds
  a change into the base branch's commits.
- While Claude redrafts a replacement (from any dv), dv doesn't commit, amend or rebase under
  it, nor run another Claude there.
- Amending is offered only when tracked files changed (new files alone can't be amended).
- An unpublished draft doesn't offer pushing or pulling (`Space g` says `S` publishes it), its
  messages after a commit, amend or rebase say `S` publishes it, and Claude's comments and
  review there don't offer copying to a review or posting (there's no PR).
- A published replacement is its PR in dv: on it once published, and when opened again (it
  stayed a PR-less draft: no comments, CI or `Space g` → Mark ready, as publishing promised).
  Claude's review of the draft moves to the PR's; `S` there closes what it replaces.

## [0.34.42] - 2026-10-19

### Added
- `S` on a replacement with uncommitted changes offers amending them into its commits (you
  place each change, or Claude finds where they go), besides committing them.

### Fixed
- Tab on an unpublished draft's title says there's nothing to fold (as in the Claude view),
  rather than flipping its arrow and folding nothing.

## [0.34.41] - 2026-10-19

### Changed
- A replacement's (or new PR's) view shows Claude's review beside its description, in the
  pane, as a PR's Claude view does, not as a section in the code's column.

### Fixed
- A replacement's first draft that failed can be drafted again: each first try gets a Claude
  session of its own (Claude Code wouldn't start the failed one's again).
- A replacement that couldn't be set up leaves no empty record (the cleanup saved it back).

## [0.34.40] - 2026-10-19

### Fixed
- A replacement's first draft that Claude stops partway (an error, its turn limit) keeps its
  record (its drafts so far, its session) again: only one that couldn't be set up is removed.
- `dv list`'s `A` on replacements, some drafted and some not: what you write goes to the
  drafted ones' redrafts; the first drafts get your instructions for replacements.

## [0.34.39] - 2026-10-19

### Changed
- Comment boxes from Claude's review file have the same dim border as GitHub's (not the hint
  color): the `◆ claude` heading and the thread's tags say whose they are.

### Fixed
- `dv list`'s `A` on replacements drafted already opens an empty editor for what else you
  want (asked of their drafts), rather than your instructions for first drafts, which it no
  longer adds as a request every time, nor overwrites with a redraft's request.
- A replacement whose drafting fails before it starts (PRs on different bases, one that isn't
  there) leaves no empty record on the replacements tab.

## [0.34.38] - 2026-10-19

### Fixed
- Redrafting a replacement no longer resends the last instructions you wrote for any
  replacement: what you write for a redraft is asked of that one's drafts (shown with them),
  and the saved instructions are for first drafts only. A new PR's redraft gets what you wrote
  (it was dropped).
- A request made of a replacement's drafts while its redraft is being set up is no longer
  lost, and two redrafts can't start at once.
- Only what the redraft under way was given shows as "Claude is redrafting with it".

## [0.34.37] - 2026-10-19

### Fixed
- `a` on a replacement's drafts: what you ask shows after them (`what you asked of the drafts`,
  marked until a redraft takes it in), instead of vanishing into the background run; it no
  longer becomes the instructions every later redraft starts from, and asked while Claude
  drafts, it's kept for the next one rather than refused.
- `dv list` no longer counts a zellij pane that has exited as one of Claude's.

## [0.34.36] - 2026-10-19

### Fixed
- A failed reload after an amend, a rebase, taking GitHub's branch or stopping Claude is said
  after what was done (the backup branch's name too), not in its place.
- `dv list`'s todo tab, filtered: `s` (defer) and `p` (post what it waits on) act on the item
  under the cursor, not another.
- `dv list`'s reviews resume Claude's session only when Claude Code still has it (as dv does).
- `dv list`'s status bar measures its message by its width on screen.

## [0.34.35] - 2026-10-19

### Changed
- Inside dv, no change you'd see: `list/mod.rs` is split as `view/` is (drawing, keys, marks,
  the background), its fields default, one helper for the keys you press twice, and its
  popups drawn as the view's are.

## [0.34.34] - 2026-10-19

### Changed
- Pickers match the words you type (each as it is, case-insensitive, in any order), not your
  letters scattered through an entry: "close" finds what says close.
- `S`'s close entry is "Close the PR (with a comment, optional)", and `?` says `S` closes.

### Fixed
- `dv list`: on a draft replacement's row, `M M`, `b b`, `W W`, `m` and `a a` no longer act on
  the PR it replaces (they say it's a draft).
- After an amend or taking GitHub's branch, a failed reload is said.
- The latest activity keeps the last three comments however many events follow them.
- Syntax colors in the uncommitted view follow a commit or amend.
- `dv list`'s message (and press-again prompts) has the status bar first.
- Opening `Space g` no longer works out a commit message you may not use.

## [0.34.33] - 2026-10-19

### Changed
- The labels / assignees / reviewers picker (`Space l`) stays open as you pick, as you left
  it (what you'd typed, where you were), the change ticked: one after another, `Esc` when done.

## [0.34.32] - 2026-10-19

### Fixed
- Every GitHub `:shortcode:` shows as its emoji (`:loudspeaker:`, `:snowflake:`, …: dv knew
  some fifty), in a `<details>` summary's text too.

## [0.34.31] - 2026-10-19

### Changed
- Inside dv, no change you'd see: `view/mod.rs` keeps App and its loop (what belongs to other
  modules moved there, what's done with the editor to `compose.rs`); App's fields default,
  pickers name the action or job they do, and each job says what it's doing.

## [0.34.30] - 2026-10-19

### Added
- `Space g` → "Amend it all into the commits Claude finds it belongs in": Claude places every
  uncommitted change (git blame's place a hint), and dv amends them in when it's done, a
  backup branch first; what it can't place stays uncommitted.

### Changed
- `dv list`: the top bar is the tabs (and where you are); the status bar has the message on
  its left and the list's state on its right: how it's ordered (or Claude's ranking: how long
  ago, its cost, what's changed since), auto on (in the warning color) or off, and the flags
  PRs open with. No badge that looks like a tab, nothing said twice.

### Fixed
- Removed lines take the base's syntax colors after the base changes (the uncommitted view, a
  rebase, a retargeted PR), not the old base's.
- After an editor action, a reload's error is said.
- The latest activity beside the description is bounded when comments are few (at most nine
  items), and each event keeps the cursor through a refresh (keyed by GitHub's id).

## [0.34.29] - 2026-10-19

### Fixed
- The cursor line's tint shows on a code block's line in a comment (the block's background
  covered it).

## [0.34.28] - 2026-10-19

### Changed
- Inside dv, no change you'd see: `view/review.rs` is split by what each part does (the PR's
  sync with GitHub, links and definitions, what you've seen, replacements, getting about the
  threads), and duplicated code in it folded.

## [0.34.27] - 2026-10-19

### Changed
- Inside zellij, the editor's pane opens below dv (hx on Enter, and what you write), not on
  its right: dv keeps its width, its comments' pane on its right.

### Fixed
- A background refresh that fails (a rate limit, a blip) no longer empties the PR of its
  GitHub comments.
- The PR's activity GitHub can't give this time is kept as last fetched, not replaced by the
  error.
- The latest activity beside the description is the last three comments and reviews (and
  what happened among them): three events after a comment no longer leave it out.
- Two events at once (two labels added) keep the cursor on the one it was on; `Space o` says
  why there's no activity when GitHub's answer was an error.

## [0.34.26] - 2026-10-19

### Changed
- The pane's conversation is the PR's activity, as GitHub's PR page lists it: comments and
  reviews in their boxes, and what else happened a dim line each: pushes (each commit under
  them), force-pushes, labels, review requests, assignments, renames, base changes, mentions
  from other issues and PRs, drafts, merged or closed. One GraphQL query (it also says which
  reactions are yours), where the comments were fetched twice before.

### Fixed
- The cursor keeps its place on any line of the PR's text through a refresh (each line has a
  key made with it, not guessed from its text).
- dv looks the remote and its default branch up once (it ran git each time when none was
  found, and asked the remote for its HEAD twice).

## [0.34.25] - 2026-10-19

### Added
- When a refresh can't get the commits' signatures from GitHub (a timeout, a rate limit),
  the last answer for the same head is kept, marked "(as of 5m ago)", rather than shown as
  unknown: a commit's signature is part of it.

## [0.34.24] - 2026-10-19

### Fixed
- Answering your replies isn't refused because an older dv's Claude pane is open (it never
  goes to the pane); what goes to the pane is still checked.
- Where zellij doesn't say a new pane's id, the hx pane is found by its name, so Enter reuses
  it rather than opening another.
- Leaving while you write beside dv: another key between the warning and the next quit means
  it's asked again.
- One rule for the default branch (a new PR's base, the diff's without a PR), and for which
  threads are about the code shown (where they're listed, and the tabs' counts).
- The cursor stays on any line of the PR's text through a refresh, not only on comments.
- Reactions in dv's cache from before 0.34.23 aren't shown as unknown.

## [0.34.23] - 2026-10-19

### Fixed
- A new PR drafted for CI is based on the remote's default branch, asked of the remote if git
  doesn't know it (it was "master" on a remote added by hand).
- The cursor on a comment in the PR's text stays on it when a refresh adds a line before it.
- A pane closed from zellij while you write beside dv is noticed even when zellij doesn't say
  the pane's id.
- The tabs' counts are what each tab's view shows, questions about a commit too.

### Changed
- Leaving dv while you write beside it is held wherever it's asked from (one place), not
  only by `q`, Ctrl-c and merging.
- Which reactions are yours is fetched for older comments and reviews at once.

## [0.34.22] - 2026-10-19

### Fixed
- Code blocks in comments have a background of their own (the theme's `markup.raw.block`,
  else the background a shade darker), not the cursor line's, and it runs to the box's edge.

## [0.34.21] - 2026-10-19

### Fixed
- The files' list's cursor has the cursor line's tint, as the diff's and the pane's do (not
  the menu's selection colour); what the diff shows, the list unfocused, the related tint.

## [0.34.20] - 2026-10-19

### Added
- With the cursor on code, the pane's box about that line has its heading tinted as the code a
  comment is about is (the same tint, not the cursor line's).
- Reactions on PRs with more than 100 comments or reviews: which are yours is asked for the
  older ones too (100 at a time); where it isn't known, they're dimmed, not shown as not yours.

### Changed
- A stopped rebase holds back only what would change the code (making changes, fixing CI,
  amending, a review, answering a reply that wants a change); questions, a description, a
  closing comment go ahead.
- Tab on a thread's first line whose comment is hidden on GitHub goes round: shown, hidden
  again, the thread folded.
- Claude's `"ask"` doesn't stop it editing its own comments when you ask it to (the want is
  about the code and GitHub, not its drafts in the review file).
- Choosing the remote: `upstream` is the default offered (a fork's usual setup), and gh is
  set to the same repo (`gh repo set-default`).

### Fixed
- The remote's default branch is asked of it when git doesn't know it (a remote added by
  hand); the drafts tab's sizes, and messages naming the base, use the chosen remote.
- `dv rerun-failed` is allowed Claude for the PR it's fixing only.
- A gateway's timeout page that isn't JSON at all is asked again smaller too; a hidden
  comment with no time has no dangling separator.

## [0.34.19] - 2026-10-19

### Added
- The remote of the GitHub repo (for fetching bases, PR heads, rebasing) needn't be `origin`:
  dv takes `dv.remote` (git config), else the only remote, else the one gh resolved as the
  base; with several and nothing to say, it asks once as it starts and keeps the answer in the
  repo's git config.
- `dv rerun-failed`: reruns the failed jobs on the PR's head. Claude fixing CI is allowed this,
  not `gh run rerun` (which could rerun whole runs, or any).

### Fixed
- A comment hidden on GitHub says so right after who wrote it, so a long heading cut at the
  pane's edge still shows it.
- GitHub out of time before it could answer (a 502 / 504 page, not GraphQL's error) is asked
  again smaller, as its own timeouts are.

## [0.34.18] - 2026-10-19

### Fixed
- Quitting while you write beside dv: the second quit counts only a moment after the first
  said so (not a held key's repeat, nor long after); merging (`Space g`) waits too.

## [0.34.17] - 2026-10-19

### Fixed
- Quitting while you write beside dv: `q` (or Ctrl-c) says so once, and the next quits; a
  second `q` used to only say it again, and Ctrl-c didn't ask.
- A pane closed from zellij keeps what's in it only when it's yours, not just what it started
  with; an empty text that couldn't be sent leaves no empty file behind.

## [0.34.16] - 2026-10-19

### Fixed
- Writing beside dv (zellij): a save as dv looks for the editor's pane is never taken for a
  cancel; a pane zellij can't be asked about counts as open; closed from zellij after a save,
  what it had is kept in a file (dv says where). `q` while you write says so first (`q`
  again quits anyway). Each unsent text is kept in a file of its own.
- CI: the list's lit-tab test works with no theme (it had failed on CI since 0.28).

## [0.34.15] - 2026-10-19

### Fixed
- Writing beside dv (zellij): what you wrote only ever goes to what asked for it, made as the
  editor said (empty allowed or not, as it was then); if that gave up meanwhile, the text is
  saved to a file, not lost or sent elsewhere. Closing the editor's pane from zellij cancels
  (it no longer blocks writing anything else); a save is never read half-written; something
  you did as the editor closed isn't dropped; `c` can't add to the review's summary while you
  write it (it'd be lost on submitting).

## [0.34.14] - 2026-10-19

### Changed
- Inside zellij, writing a comment (a reply, a commit message, …) no longer freezes dv: the
  editor's pane opens beside it and dv stays usable; what you wrote goes once you save and
  quit (`:cq` cancels). One at a time: starting another says to finish the first.

### Fixed
- `C` on a hidden conversation comment says in the editor why it isn't quoted; the pending
  picker doesn't tell a hidden comment's kind from its text; the hidden-comment rule is in
  one place for the PR's text too.

## [0.34.13] - 2026-10-19

### Fixed
- A hidden comment's text is read through one rule everywhere, the PR conversation's too:
  pickers show a comment once you've shown it; `c` can't quote one unseen from any path.
- `Space d` on a hidden comment marks it as hidden, not as its quoted text; `a` on one reads
  "— hidden on GitHub (spam), not shown here", with its id only where there's no link; `a`
  where no comment's found says so.

## [0.34.12] - 2026-10-19

### Fixed
- A comment hidden on GitHub isn't shown unseen by `Space d` (its preview says it's hidden)
  or `Space e` (Tab shows it first); every quote, preview and edit reads a comment's text
  through one rule.
- `a` on a hidden comment says it's hidden (and gives the comment's id), so Claude knows to
  read it on GitHub.

## [0.34.11] - 2026-10-19

### Fixed
- `a` (ask Claude) on a comment hidden on GitHub no longer quotes its text unseen (Claude
  reads it at the link); `C` on a hidden conversation comment opens a new comment, unquoted.
- The hint on a hidden comment offers what works there (`r` on a thread, `C` in the
  conversation); once shown, its text has the usual hints.
- Tab showing or hiding a comment keeps the cursor's column.

## [0.34.10] - 2026-10-19

### Fixed
- `c` / `C` on a conversation comment hidden on GitHub, and `r` on a hidden review comment,
  no longer quote its text unseen (Tab shows it first).
- The tabs' counts include threads on files gone from the diff, where they're listed.
- With the files' list focused, the code a comment is about isn't tinted; the relative
  line number of the code's line is lit only when the code has focus.
- On a shown hidden comment, the hint says Tab hides it again; Tab showing or hiding one
  keeps the cursor where it was if its heading can't be found.

## [0.34.9] - 2026-10-19

### Changed
- With the cursor on a comment in the pane, the code it's about has a tint of its own, not
  the cursor line's: the theme's `ui.cursorline.secondary`, else half the cursor line's.

## [0.34.8] - 2026-10-19

### Changed
- Focus is shown by the cursor line's tint alone, on the side that has it: in the comments
  pane, the code beside isn't tinted (nor, in the code, the comment beside); with the files'
  list focused, only its selection. Borders no longer change color: not the pane's separator,
  the list's divider, or the box about the cursor's line.

### Fixed
- On a conversation comment hidden on GitHub, `Space c`, `gf` and `C` work on that comment.
- Tab hiding a comment again leaves the cursor on its heading.
- A folded thread whose first comment is hidden has that note dimmed.
- The tabs' counts leave out threads on files that aren't shown.
- `Space o` from another tab says it's on the whole diff now.
- The hint on a hidden comment is just Tab; pickers say "hidden on GitHub" when GitHub gives
  no reason.

## [0.34.7] - 2026-10-19

### Changed
- A comment hidden on GitHub is a line, as a folded thread is: `▸ @who · 3d ago · hidden on
  GitHub (spam)` (no box, dimmed); in a thread, just its heading says so. `Tab` shows it
  (on a thread's first line, Tab then folds the thread, as usual); the key hints say so.
- In the pane, as a column, comment text starts nearer the line beside the code, and has
  two more columns to wrap in; folded threads' `▸` sits where the box's side would be.

## [0.34.6] - 2026-10-19

### Changed
- In the comments pane, comment boxes are open on their left instead: their rules come out
  of the line between the code and the pane (`├───`), and their right side is closed again.

### Fixed
- The line between the code and the pane goes on up through the pinned title.
- Tab on a folded thread whose first comment is hidden on GitHub unfolds it (it showed the
  hidden text instead); hidden comments' text no longer shows in the comment pickers; the
  hidden line wraps, and says just "hidden on GitHub" when GitHub gives no reason.
- `Space o` with no conversation says so; from another tab it builds once.
- The cursor on the PR's text stays on the same comment when the pane re-wraps it.
- In the pending view, Tab on the files-gone heading no longer folds your review's text.
- The tabs' counts are of threads on the code that are shown.

## [0.34.5] - 2026-10-19

### Added
- Comments hidden on GitHub (minimized as off topic, outdated, …) show folded: one dim line,
  `hidden on GitHub (off topic): Tab shows it`; `Tab` shows the text, and again hides it.
- The pane's tabs say how many threads each has: `Comments 4  Claude 1  Pending 1`.

### Fixed
- The cursor on the PR's text stays put when `Ctrl-w c` shows or hides the pane.
- `Tab` on comments on files gone from the diff, on a branch's questions, or on the title in
  the Claude view no longer folds something else (or nothing visible).
- `Space o` from the Claude or pending tab goes to the whole diff with all the conversation
  (it no longer flips back to the latest); with no conversation it says so.
- "All of it: Space o shows only the latest" shows only when the latest would show less; the
  key hints on that line are right.

## [0.34.4] - 2026-10-19

### Changed
- In the comments pane, comment boxes are open on their right: no side border, their rules
  run to the pane's edge, and their text uses the width that frees.

## [0.34.3] - 2026-10-19

### Changed
- The line saying how much more of the conversation there is comes first, under its heading
  (what's left out is older: newest last), and is short enough not to be cut.

## [0.34.2] - 2026-10-19

### Changed
- `Space o` (or Enter on the "N more" line) shows the whole conversation right there, in the
  pane beside the description, and again only its latest: no separate view to go back from.
- The PR's facts list its reviewers one a line (`✓ @amy approved`, `● @cy requested`): no
  scrolling sideways to see them all.

### Fixed
- With the pane shown: the pending view's review text and comments on files gone from the
  diff are in the pane, at its width; Tab on Claude's review on a branch (no description)
  doesn't hide it for good; folding the PR's text leaves the Claude tab's summary alone;
  hiding the pane keeps the cursor on the PR text it's on; word motions go through it, inline;
  questions to Claude on a branch before any review show.

## [0.34.1] - 2026-10-19

### Fixed
- 0.34.0: with the comments pane hidden, the PR's facts, its conversation's latest, your
  pending review's text (and Claude's summary in the Claude tab) showed nowhere; they're after
  the description again. Comments on files gone from the diff are back in the diff's column.
- `[new]` tags go again when you leave a thread; Esc from the whole conversation goes back to
  the tab it was opened from; Tab on what's attached to the description folds it with the
  description (not the whole PR section).

## [0.34.0] - 2026-10-19

### Changed
- The `PR` tab is gone: the PR's text is attached to its description, beside it in the
  comments pane (as comments are beside their lines; after it, the pane hidden): its facts,
  then the conversation's latest (new since you looked, the last three at least, any with a
  question to Claude), then your pending review's text. `Space o` (or Enter on the line saying
  how many more) shows the whole conversation on its own, where `a`, `C`, `Space =` and `gf`
  work as on the latest; Esc goes back. The comments tab no longer starts with your review's
  text (it's attached to the description).
- The Claude tab shows the PR's description too, Claude's summary attached beside it.

### Fixed
- Signatures asked again on their own only for GitHub's own kinds of refusal for a query's size
  or time (not an error with no kind); the reason given is the one that explains why not, and
  when asking again fails too, its reason as well.

## [0.33.3] - 2026-10-19

### Fixed
- 0.33.2 showed the last known signatures (and your reactions) when GitHub didn't say: from
  the cache, as if current, even of an older head, hiding "signatures unknown". It says
  unknown again. Asked again alone only when refused for its size or time (not a rate limit,
  a permission, a PR not found).

## [0.33.2] - 2026-10-19

### Fixed
- After rerunning failed checks, dv looks at them again through its background watch (a picker
  open isn't disturbed, nothing said is overwritten), and a fetch from before the rerun isn't
  shown as current.
- Notes on a retargeted PR's base replace one another rather than pile up, and the "as dv last
  saw it" notice goes once GitHub's fresh answer is in.
- Signatures: GitHub refusing only a PR's reactions keeps them; refusing its commits, or an
  error that doesn't say where, leaves them unknown: `commits: signatures unknown (why)`, and
  the commits' marks as they were (not all taken as signed). Refused whole (a big PR, with its
  reactions; not when rate-limited), they're asked on their own. What GitHub last said of
  them, and of your reactions, stands (in what dv keeps, too) when it doesn't say.

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

[0.34.4]: https://github.com/evanlinjin/diffview/releases/tag/v0.34.4
[0.34.3]: https://github.com/evanlinjin/diffview/releases/tag/v0.34.3
[0.34.2]: https://github.com/evanlinjin/diffview/releases/tag/v0.34.2
[0.34.1]: https://github.com/evanlinjin/diffview/releases/tag/v0.34.1
[0.34.0]: https://github.com/evanlinjin/diffview/releases/tag/v0.34.0
[0.33.3]: https://github.com/evanlinjin/diffview/releases/tag/v0.33.3
[0.33.2]: https://github.com/evanlinjin/diffview/releases/tag/v0.33.2
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
