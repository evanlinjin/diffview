# dv — PR diff viewer with Helix keys + rust-analyzer

```
dv              # working tree vs merge-base(origin/HEAD)
dv <base>       # working tree vs merge-base(<base>)
dv --pr 123     # gh pr checkout 123 first (-f: reset the local PR branch to the PR head)
dv --pr 123 -w  # check it out in a worktree next to the repo (<repo>-123), detached; reused later
dv --pr 123 -w -b  # same, on the PR's branch (tracking it, for pushing)
dv 123 -wb         # shorthand: a bare number is a PR; short flags combine (-p N = --pr N)
dv 123 --rm        # remove that worktree (-f if it has local changes; a -b branch is kept)
dv list -wb        # pick from the open PRs; Enter reviews one with these flags, q comes back
# If the branch has a PR, dv uses its base and loads its review threads.
```

The diff is always base → **working tree**, so the new side is real code on
disk: rust-analyzer can index it and `hx` can open it.

## Phases
1. **Diff view**: parse `git diff`, one scrolling unified view (file header →
   hunks), Helix-style cursor + selection, copy, context expansion, open in hx.
2. **rust-analyzer**: JSON-RPC over stdio. `gd gr gy gi`, `Space k` on the
   symbol under the cursor (new-side lines only). Results go into a picker; jump
   within the diff, or open hx if the location is outside it.
3. **PR comments**: `gh api .../pulls/N/comments`, in the comment pane on the right
   beside their lines (never inline: hidden, the lines are marked in the gutter). A new comment attaches to the **selected lines** (GitHub multi-line
   comment: `start_line`/`line` + sides; the selection must sit within one file).
   The comment body is written in `$EDITOR` and posted via `gh api`. Threads load via
   GraphQL (resolved/outdated state included); resolved threads start folded, outdated
   ones sit under the file header. Posting requires HEAD == PR head and no local
   changes to tracked files, so line numbers match what GitHub has.
   **PR info** (title, state/draft, base ← head, labels, review decision + reviewers,
   CI checks with failing/pending ones listed, description, conversation) is a
   section at the top of the view from `gh pr view --json`; `Tab` folds it.
   **Syntax highlighting**: syntect (pure-Rust regex), whole file per side (working
   tree / `git show base:path`) so multi-line constructs parse right. syntect scopes map
   to Helix scopes, colored by the user's **Helix theme** (`theme` in config.toml, with
   `inherits` + palettes, from `~/.config/helix/themes` then the runtime dirs `hx --health`
   reports); the UI (selection, cursor, statusline, popups, diff colors, jump labels)
   uses the theme's `ui.*`/`diff.*` keys. No theme → named ANSI colors. Added/removed
   lines get a tint blended from `diff.plus`/`diff.minus` into `ui.background`.
   Paired removed/added lines also get the changed words emphasised (token LCS; skipped
   when the lines share less than they differ). Hunk headers draw as a quiet rule
   (`── fn main ─── L12–18`). Helix's `cursorline` and `line-number = "relative"` are
   mirrored: cursor row tint + highlighted line number, and a relative column counting
   screen rows (so `5j` lands where it says).
   PR description, conversation and (unresolved) review comments are highlighted as
   markdown, code fences included. TOML uses a small built-in grammar (syntect has
   none). File headers are a full-width bar with +/− line counts.
   **Reviews** use GitHub's own pending review (GraphQL): `c` adds to it (starting one on
   the PR head if needed, or continuing one started on the web), `S` submits with a summary
   from `$EDITOR` (`:cq` cancels), or comments and closes the PR (an empty comment only closes), discard asks for confirmation. Pending comments are
   tagged `[pending]` and counted on the PR's title line (pinned at the top, over the code and
   the pane, in the status bar's color: it has the threads, files viewed and pending counts). GitHub's answers are kept in
   `cache/pr-N.json` in dv's data directory: opening the PR shows them at once and fetches
   afresh in the background; a job that changes GitHub first waits for the fresh state
   (fetching it then if it isn't in yet). dv checks the PR's ETag every 5 seconds
   (a `304` costs no rate limit) and refetches in the background when it changed (new
   comments, replies, reviews, pushes; not edits or resolving), shown once no picker,
   prompt, selection or pending key is open. `c` on a conversation comment (or the
   description) quotes it at the end of the review's text (its summary), saved on GitHub and
   loaded with the PR; `S` starts from it. GitHub can't add text to a pending review started
   without any (`c` on code lines first), so then the text stays in dv until it's submitted
   (dv starts reviews with the text, when there is some). If posting fails, the text is saved
   to a temp file and the error says where.
   **PR text renders like github.com** (pulldown-cmark, GFM): link text without URLs (bare
   URLs, `#123`, `@user` become links), tables as grids sized to fit (display width, so emoji
   align), headings/emphasis without markers, `•`/`1.`/`☐☑` lists, `▎` quotes, fenced code
   highlighted in its language, `<details><summary>` as `▸ …`, common `:emoji:`, and line
   breaks kept as GitHub does in comments. PR info: state badge and label chips in GitHub's
   colors, colored review decision and check counts. Comment times are relative ("2d ago").
   Comments (open review threads, the description, conversation comments, Claude's summary) sit
   in boxes with a subtle border in the line-number color (no fill): `╭─ author ─╮`, a `├─` divider per reply, `╰──╯`
   (an empty row of its own); a folded thread stays one `▸` line.
   File headers: dir dimmed, +/− colored. Split view hatches the empty side; pickers
   highlight matched letters.
   **Claude reviews**: a review file at `~/.local/share/dv/<repo>/reviews/pr-<N>.json` (dv's
   data directory, `$XDG_DATA_HOME` if set, named after the main checkout's path; not `.git`,
   which Claude Code won't edit in; an older `.git/dv` moves there) (or
   `branch-<name>.json`): `{head, created, summary, comments: [{path, line, start_line?, side,
   body, resolved?, replies?: [{author: user|claude, body, created}]}]}`, line numbers being file
   lines on `side` (new = working tree, old = base). Outside the repo's history and shared by its
   worktrees. Claude Code writes it with the `dv-review` skill (`skills/dv-review/SKILL.md`,
   installed by `dv install-skill`): `dv review-info` gives it the file path and the base/head of
   the diff dv shows; `dv review-check` checks every comment against the diff (errors: dv can't
   place it; warnings: GitHub won't take it, more than 3 lines from a change, or it's outdated).
   `review-check` also records each comment's `code` (the commented lines' text) and moves
   comments whose code moved. dv does the same on every load, keeping comments on their code
   through new commits and edits (Claude's or yours); a comment whose code is gone is
   `[outdated]`: folded, dimmed, showing that code, placed near its old line (or under the file
   header), and `c`/`C` refuse it. dv shows the
   comments as threads by `claude` (accent color) and the summary in the top section (shown
   without a PR too), and reloads when the file changes. `r` stages a reply to Claude in the
   file (the skill answers replies, changing code if asked), `c`/`C` copy Claude's comment into
   your pending review / post it (editable in `$EDITOR`; posting as is is fine), then dismiss it
   from the file; on Claude's summary, `c` adds it to your review summary (what `S` starts
   from) and `C` posts it as your PR comment, as your words (not a quote); `Space r` resolves it in the file, `Space d` dismisses it. `Space a` runs
   Claude Code from dv (review / update the review, answer your waiting replies, or make the
   changes its open comments ask for, after optional instructions of yours in `$EDITOR`): in a
   zellij pane on the right as an interactive session, else `claude -p` in the background
   (`acceptEdits`, Bash limited to `dv review-*` and read-only git), with "Claude working…"
   in the status bar and its log path if it fails. Comments get stable `id`s (assigned by dv on
   load and by review-check); both write the file atomically and re-read it before a change.
   Each PR has one Claude Code session per checkout (`pr-N.session` next to its review file,
   `<dir>\t<uuid>` lines, as Claude Code keeps sessions per directory; dv picks the UUID with
   `--session-id`, then `--resume`s it while Claude Code has it), so every run continues one
   conversation. When that session is open in a zellij pane, `Space a` pastes its request into
   the pane (you press Enter; not while the pane is asking you something), and replies run on a
   `--fork-session` in the background. Your own comments for Claude (`a`) stay private: `c`/`C`
   refuse them, and they don't count as Claude's. Repo-wide
   notes for Claude live in `notes.md` in that directory (`dv review-info` prints the path; the
   skill reads them first and keeps them current; `Space a` opens them).
   **Replacements** (`replace.rs`): for PRs not worth reviewing into shape. A replacement is
   a branch `dv/replace-N` (N: the first PR it replaces) in a worktree `<repo>-replace-N`,
   from the originals' base on origin, and a record `replacements/N.json` in dv's directory:
   `replaces` (number, title, author, URL of each), `base`, Claude's drafts `title`, `body`,
   `closing_note` (`{replacement}` where the new PR's link goes), and `session`. `dv replace
   N [M…]` (and `A A` in `dv list`, `Space a` in dv, which run it in the background) prepares
   them and runs Claude (`claude -p`, the skill's section 6, with `dv replace-info`; allowed
   to read PRs and issues with `gh` and to `git add` / `git commit`, never to push). Drafting
   again resumes the session. dv on a replacement's branch shows, in the PR section, what it
   replaces (linked), the drafts, and `UNPUBLISHED` on the header line, and shows it again
   when the record changes. `S` there publishes it, or publishes then closes the originals
   (editor: title on the first line, then the description; not with uncommitted changes:
   committing is offered instead; fetch and rebase onto `origin/<base>`, aborted on conflict;
   push to `dv.replaceRemote` (git config; asked for from the remotes other than origin when
   unset); `gh pr create --draft --head <owner>:dv/replace-N`; `published` saved). Closing
   (editor: the note, `{replacement}` filled in, or "Superseded by #N." appended; `gh pr close
   N --comment`) records each in `closed` as it goes. A draft under way has its process in the
   record (`drafting`), so `dv list`, dv and `dv replace` don't start another; `dv list`
   watches the records. Drafts get the notes in the prompt and write access only to
   `replacements/` (not the notes, which every review reads). PRs into different bases can't
   share a replacement. For several marked PRs, `A A` first has Claude suggest groups (a
   `claude -p` that only reads them), edited in `$EDITOR`, a line each.
4. **Later**:
   - **Reproducible release builds**, so a binary can be rebuilt bit for bit and signed ahead
     of time (signatures, or the signed hashes, in the release notes or beside the archives).
     Needs a pinned toolchain, `--remap-path-prefix` for absolute paths, `SOURCE_DATE_EPOCH`,
     and archives made the same way every time (sorted entries, fixed mtimes and owners).

## Keys
| Key | Action |
|---|---|
| `h j k l`, arrows | move (counts work: `5j`) |
| `w b e`, `W B E` | word motions (they select, like Helix) |
| `gh gl gs` | line start / end / first non-blank |
| `f t F T <c>` | find char |
| `gw` | jump labels on visible words (uses your `jump-label-alphabet`) |
| `mm` | matching bracket (counted through one version of the whole file; grows the hunk if the match is hidden) |
| `gg ge`, `Ctrl-d Ctrl-u`, `Ctrl-e Ctrl-y` | top / end, half page, scroll 5 lines |
| `zz zt zb` | put the cursor line at center / top / bottom |
| `x` | select line (repeat to extend) |
| `v`, `;`, `Esc` | select mode / collapse selection / exit select mode |
| `]g [g` | next/prev change (hunk) |
| `]f [f`, `Space f` | next/prev file, file picker |
| `/`, `n N`, `*` | search (smart case), next/prev match (as in Helix), search for the selection |
| `y` / `Space y`, `Y` | copy text, copy as patch (via OSC 52) |
| `Space c` | copy a ref to paste to Claude: `path:line` / `path:start-end` for the selection; on a GitHub comment its link (`…#discussion_r…`, `…#issuecomment-…`), on the PR header / description the PR's, on a Claude comment its lines |
| `+`/`=`, `-`, `Z` | context ±10 lines around the hunk under the cursor (hunks merge as they meet), whole file |
| `Tab` | collapse/expand file; on a comment, fold/unfold its thread |
| `Shift-Tab` | fold all files, or unfold all if all are folded |
| `Ctrl-w c` | show / hide the comment pane on the right: comments beside their lines, in unified and split view (thread tags on their own line there), and on its PR tab (`gn` / `gp`, or a click on the tabs: they're on the pinned title, over the pane) the PR section: description, conversation, checks, Claude's summary, comments on files no longer in the diff, scrolled on its own (`Ctrl-l` into it, `Ctrl-h` back to the code line the cursor left); the diff keeps only its header line then. The cursor onto a comment or PR text switches to its tab. Shown by default when the window fits 80 code columns (70 each side in split view) + the pane (a third of the window, 60–80 columns, or all beyond a 200-column code side); otherwise hidden (except in the Claude and pending views), and shown it takes half the window, floating over the code: covering its right end only on rows with a comment. `Ctrl-w f` switches column / floating, `Ctrl-w ,` / `.` resize it by 6 columns, `Ctrl-w =` its usual width. What these keys set (shown or hidden, floating, width) is kept for the next run, in any repo (`$XDG_DATA_HOME/dv/pane.json`). Hidden, a line with comments has `●` in its gutter (`◆` Claude's, `○` all resolved or outdated); moving onto a comment (`]c`, a picker) shows the pane. The comments tab starts with the review's summaries (yours, pending, and Claude's: two lines of each, `…` if there's more), over its top rows: the comments start below them and scroll under them, and a click on them shows them in full on the PR tab. As a column, a dim line between the code and the pane; floating, the pane in the popup color. Code blocks in comments don't wrap: cut at the box's edge with `‹` / `›` where there's more, scrolled sideways (that block alone) as the cursor moves along them. Tables wrap their cells within columns narrowed to fit (to 20 columns at least); wider still, they scroll the same way |
| `Ctrl-w v` | toggle split view: old ‖ new side by side (removed runs pair with the added lines after them) |
| `Ctrl-w h` `Ctrl-w l` (or `Ctrl-h` `Ctrl-l`) | split view: move to the old / new side |
| `Enter` | open `hx file:line:col` (inside zellij: in a pane on the right, reused for the next file while it is open; dv stays usable); dv reloads whenever the working tree changes; on a link, open the link |
| `gf` | open the link under the cursor in the browser (`xdg-open` / `open`); a tooltip shows the URL while the cursor is on a link. On a `path:line` in text (`src/a.rs:12`, `:12-15`, `:12:5`, `#L12`; the path from the repo root, or the end of one if that's unambiguous): go there, in the diff if it's shown (unfolding its file), else in hx; `Ctrl-o` comes back. The skill has Claude cite code that way. On a name (in a `code` span, or one that looks like code: `snake_case`, `camelCase`): where it's defined, by `git grep` for `fn`/`struct`/`class`/`def`/`function`/… name, the diff's files first; a picker of several |
| `gd gr gy gi`, `Space k` | definition / references / type / impl, hover |
| `Space k` on an issue/PR link | preview it in a popup: state, author, age, comments, labels, body (the link tooltip says so) |
| `Ctrl-o` | jump back |
| `c` | comment on the selection, in your pending review |
| `r` | reply on a thread, in your pending review (on a Claude comment: reply to Claude) |
| `C` | `c` / `r`, but post immediately (not beside a pending review: GitHub refuses that, so dv says so first); in the PR description/conversation: a new conversation comment quoting the comment under the cursor (`c` there: quote it in your review summary) |
| `s` | suggest a change: `$EDITOR` opens with a ```suggestion block of the selected new-side lines (pending review) |
| `S` | submit the pending review (comment / approve / request changes) or discard it; under the choices, what's still open (Claude's open comments and how many it answered, unresolved GitHub threads, pending comments on changed code, a checkout that isn't the PR head), also as hint lines when approving |
| `Space r` | resolve / unresolve the thread under the cursor |
| `Space e` | edit your comment under the cursor (opens its text in `$EDITOR`) |
| `Space d` | delete the comment under the cursor (confirmation picker, "No" preselected) |
| `Space p` | picker of your pending review's comments; jumps there (unfolding as needed); `Ctrl-d` deletes the selected one (asks first) |
| `Space a` | run Claude Code: review (or update the review: after new commits on top of the reviewed one, just those), answer your replies, or make the changes the open comments ask for (Claude's and yours; your instructions optional, or the whole request when there are none; on someone else's PR: suggest them, as suggestion blocks, without editing). Also: Claude's notes, dismiss its outdated comments, start a fresh session for the PR, check on Claude now (its review file read again, its run's state), and while a run is going, stop it (kept: its log, and the session, which the next run resumes; the status bar shows how long it's run). Background runs may also run `cargo check/build/test/clippy`, and say what they cost |
| `Space A` | picker of Claude's comments (`[waiting]`: your reply is unanswered, `[answered]`: Claude answered you; severity tags like `[bug]`); jumps there |
| `Space t` | picker of all review threads (state tags, author, gist, reply count); type to filter, e.g. `resolved` |
| `Space v` | mark / unmark the file as viewed on GitHub (folds it and moves to the next file); viewed files start folded, headers show `✓ Viewed` / `↻ changed since viewed` at the right edge |
| `]c [c` | next/prev comment thread |
| `Ctrl-w a` | Claude view: only Claude's review: the PR's first info line (state, author, branches, size), Claude's summary, and the files it commented on, each comment with its lines ± 3 (other threads hidden; `+`/`-` show more or less around the comment under the cursor, kept apart from the whole diff's, and `Z` the whole file; going to a thread that isn't Claude's leaves it). Again: the whole diff |
| `Ctrl-w p` | Pending view: as the Claude view, for your pending review: the PR's first info line, the review's text so far, and the threads with a pending comment, each with its lines ± 3. Again: the whole diff |
| `]n [n`, `Space n` | next/prev thread tagged `[new]` (comments you haven't seen, not your own: seen once the cursor leaves the thread or has stayed on it for 3 s, remembered in dv's directory, `seen/`), and a picker of them |
| `]p [p` | next/prev comment of your pending review (replies too), wrapping around; opens folded files and threads |
| `]a [a` | next/prev Claude comment, wrapping around; folded files and threads are opened on the way |
| `a` | on code lines: a comment for Claude (in the review file, `"author": "user"`), which Claude answers right away; `r` continues the thread. On a replacement's description or closing note: a redraft, its instructions starting from a quote of those lines ("About the closing note: > …") |
| `r` on a Claude comment | reply to Claude: saved in the review file, then Claude answers right away in the background (`claude -p`, resuming the last run's session; replies made meanwhile get one more run) |
| `c` / `C` on a Claude comment | copy it into your pending review / post it now; dismissed from the file once posted |
| `R` | refresh: re-run `git diff`, re-fetch threads + PR info |
| `Space g` | git: show only what isn't committed (diff against HEAD, to check Claude's changes) / the whole diff again; commit everything (new files too; the message starts from Claude's "Changed: …" replies since the last commit); push to the branch it tracks (a PR's, under its own name) |
| `Space m` | the status bar's messages so far, in full, and the last background Claude run's log and the runs' cost |
| `Space u` | update the checkout: to the PR's head as `gh pr checkout` does (fast-forward only; local commits are kept), or `git pull --ff-only` without a PR; then refresh |
| `?` | key help overlay (two columns when the window fits them); `/` inside it filters entries (smart case), Enter keeps, Esc clears |
| mouse | wheel scrolls (view, popup, picker); click moves the cursor (or folds, on a fold arrow; picks in a picker, opens on a second click); drag selects. `--no-mouse` leaves it to the terminal |
| `q` | quit |

`dv list`'s first tab, **todo**, is what needs you, in the order to take it (`todo.rs`,
`list/todo.rs`). dv finds what's waiting on you, from its own files and one GraphQL query (your
latest review and the commit it was on, review requests, mergeability, the head commit): a
replacement to publish or to close the originals for, a push since your review, Claude answering
you, your review requested, approved + mergeable + checks green, Claude's review new to you (since
you last left the PR in dv), Claude's comments you haven't touched, no review from you (not your
PRs, not drafts), else activity since you looked. It orders them by the most pressing of those,
then the longest waiting; then Claude (one read-only `claude -p`, `gh pr view/diff` only, the repo's
notes in its prompt) puts them in order with a few words on each ("why — what next", the card's
third line), saved in dv's directory (`todo.json`) with what it ranked. Items it didn't rank (new,
or changed) go before the first ranked one that needs you less; the header says when the ranking
is stale. Claude ranks on `R` there, and by itself when you open the tab and its last ranking is
missing or over 12 hours old. After a ranking, Claude does its own part on the top three that have
one, as `a a` runs it (worktree, session): answers your replies waiting on it, updates a review
behind the PR's head, or reviews one that has none. Enter opens the PR (a replacement, in its
worktree); back in the list, what you saw to drops off.

`dv list` (the 1000 most recently updated open PRs; a `#N` filter's Enter opens PR N even if it
isn't listed; `Tab` switches to the PRs with a local
`<repo>-N` worktree, open or not, which `Enter` opens in that worktree as it was made:
detached or on its branch) shows each PR as a two-line card: number,
title, draft badge, age; then author, CI (✓/✗/● counts, fetched in the background since they're
slow), review decision, size, labels, base ← branch. `j k`, `gg ge`, `Ctrl-d Ctrl-u` move;
`o` cycles the order (updated / created / number / size, newest or biggest first), `O`
reverses it. `/` edits the filter (Enter keeps, Esc clears): space-separated terms that must
all match, GitHub-search style, `-` negating one: `author:` `label:` `base:` `branch:`
(substring), `is:draft`, `review:approved|changes|required|none`, `ci:pass|fail|pending|none`,
anything else is text in the number, title, author, branch or labels (smart case). E.g.
`ci:fail -is:draft -author:dependabot`. `Enter` reviews the PR as `dv <N> <flags>` would, `R`
refreshes, `q` quits. `m` marks PRs (kept in dv's directory, `marked.json`, so they outlast opening a PR and `dv list`
itself; closed PRs' marks go) and `a a` has Claude review them (or the selected one) in
the background (after your editor, for optional instructions to every one of those reviews, starting
from your last review instructions in this repo, as `Space a` → review does; `:cq` cancels), two at a time, each in its `<repo>-N` worktree (made if need be; one on the
PR's branch stays on it) and its own session; cards show `claude: reviewing…` and then
`claude ✓`. `A A` has Claude draft one replacement for the marked PRs (or the selected
one): see Replacements below. Tab cycles open / worktrees / replacements. `[new]` (before the age) marks a PR with something you haven't seen: GitHub updated
it, or Claude's review changed, since you last left it in dv (`seen/pr-N.looked`); of PRs
you've never opened, only those Claude has reviewed. `D D` in the worktrees tab removes the worktree (not with local changes; the branch
stays). Quitting the review returns to the list; if opening a PR fails, the
error shows in the list's status bar.

Discoverability: after a prefix key (`Space`, `g`, `z`, `]`, `[`, `Ctrl-w`, `m`) a which-key
popup lists what can follow, as in Helix. The same bottom-right popup otherwise says what the
keys do where the cursor is: on a selection, a review thread, Claude's comment, the PR header
or PR text (nothing on plain code). With the cursor in the comment margin it sits bottom
left instead, off the comments. The status bar's left
side (what you're on, messages, hints) wins over its right: when both don't fit, the right
keeps only the essentials.

## Design
One crate:
- `main.rs`: args, checkouts and worktrees, the `review-*` / `install-skill` commands
- `diff.rs`: `git diff` (untracked files too) and its parser
- `gh.rs`: GitHub through `gh` (REST and GraphQL)
- `claude.rs`: dv's data directory, review files, sessions, the skill
- `list/`, `batch.rs`, `todo.rs`: `dv list`, its batch reviews, and what needs you (ranked)
- `lsp.rs`, `highlight.rs`, `md.rs`, `theme.rs`: rust-analyzer, syntax, markdown, Helix themes
- `view/`: the diff viewer (`mod.rs` state and layout; `keys`, `motion`, `select`, `mouse`,
  `render`; `review` for GitHub, `agent` for Claude Code, `git` for `Space g`, `ra`; `tests`)

Deps: `ratatui`, `anyhow`, `serde_json`, `syntect`, `toml`, `pulldown-cmark`. Shells out to
`git`/`gh`/`claude`/`zellij`; no git2, no GitHub crate.

Not planned: editing in dv, forges other than GitHub, other LSPs.
