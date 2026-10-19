# dv — PR diff viewer with Helix keys + rust-analyzer

```
dv              # working tree vs merge-base(origin/HEAD)
dv <base>       # working tree vs merge-base(<base>)
dv --pr 123     # gh pr checkout 123 first (-f: reset the local PR branch to the PR head)
dv --pr 123 -w  # check it out in a worktree next to the repo (<repo>-123), detached; reused later
dv --pr 123 -w -b  # same, on the PR's branch (tracking it, for pushing); a branch already
                   # checked out in another worktree (a replacement's own): opened there
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
   beside their lines (never inline; their lines are marked at the code's right edge, beside the pane). A new comment attaches to the **selected lines** (GitHub multi-line
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
   the pane, in the status bar's color: it has the threads, files viewed and pending counts; and, as badges before the PR's state that always show (a long title is cut for them), what Claude is doing here (`◐ claude: <what> <for how long>` for a run in the background; `◐ claude working (its pane)` while its pane shows Claude Code's "esc to interrupt"; `◐ claude is asking you` at a prompt there) and how the checkout stands against the PR (`↑N unpushed`, `⇅ diverged: force-push` after an amend or rebase, `↓ behind the PR`, `✎ uncommitted`; worked out when the tree changes or the PR's head moves). GitHub's answers are kept in
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
   zellij floating pane as an interactive session, opened without focus, so hidden while
   zellij's floating panes are (`Space a` → "Show Claude's pane" brings it up), and closed once
   Claude's done there: its transcript (Claude Code's, by the session's id) ends on its answer
   (`end_turn`) since the run started, not a question for you (ending in `?`), nor asking you
   something on screen, with its input box empty (a request waiting for Enter, your words), and
   you not looking at it (shown and focused); by dv or `dv list`, whichever sees it first. Its
   session is kept for the next ask to resume; dv says when it's
   asking you something, a permission prompt, checking its screen every 3s), else `claude -p` in the background
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
   replaces (linked) and `UNPUBLISHED` on the header line, its drafts of the description and the
   closing note as sections before the files (as a PR's description is; `a` there redrafts), and shows it again
   when the record changes. `S` there publishes it, or publishes then closes the originals
   (editor: title on the first line, then the description; not with uncommitted changes:
   committing is offered instead; fetch and rebase onto `origin/<base>`, aborted on conflict;
   push to `dv.replaceRemote` (git config; asked for from the remotes other than origin when
   unset); `gh pr create --draft --head <owner>:dv/replace-N`, always a draft: it's marked ready for
   review once CI passes; `published` saved). Closing
   (editor: the note, `{replacement}` filled in, or "Superseded by #N." appended; `gh pr close
   N --comment`) records each in `closed` as it goes. A draft under way has its process in the
   record (`drafting`), so `dv list`, dv and `dv replace` don't start another; `dv list`
   watches the records. Drafts get the notes in the prompt and write access only to
   `replacements/` (not the notes, which every review reads). PRs into different bases can't
   share a replacement. **New PRs** are the same, replacing nothing: `N N` in `dv list` opens
   your editor for what it should do (issues by `#N`, which Claude reads), then makes `dv/new-K`
   in `<repo>-new-K` from the default branch on origin, with a record keeping your request
   (`task`; `dv replace-info` prints it), and Claude drafts it in the background (the skill's
   "Draft a new PR": code, commits, `title`, `body`; never pushed); it's on the drafts tab (the
   replacements tab, renamed) as `new K`, reviewed in dv as a replacement is, and `S` publishes
   it. For several marked PRs, `A A` first has Claude suggest groups (a
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
| `gg ge`, `Ctrl-d Ctrl-u`, `Ctrl-e Ctrl-y` | top / end (in the comments pane, its own: the first / last comment), half page, scroll 5 lines |
| `zz zt zb` | put the cursor line at center / top / bottom |
| `x` | select line (repeat to extend) |
| `v`, `;`, `Esc` | select mode / collapse selection / exit select mode |
| `]g [g` | next/prev change (hunk) |
| `]f [f`, `Space f` | next/prev file, file picker |
| `/`, `n N`, `*` | search (smart case), next/prev match (as in Helix), search for the selection |
| `y` / `Space y`, `Y` | copy text, copy as patch (via OSC 52) |
| `Space c` | on a link: copy its URL (what Enter opens in the browser; its tooltip and the key hints say so). Else copy a ref to paste to Claude: `path:line` / `path:start-end` for the selection; on a GitHub comment its link (`…#discussion_r…`, `…#issuecomment-…`), on the PR header / description the PR's, on a Claude comment its lines |
| `+`/`=`, `-`, `Z` | context ±10 lines around the hunk under the cursor (hunks merge as they meet), whole file |
| `Tab` | collapse/expand file; on a comment, fold/unfold its thread; on a comment hidden on GitHub (shown as a line, as a folded thread is: `▸ @who · 3d ago · hidden on GitHub (off topic)`, dimmed; in a thread, its heading says so and its text is left out), show its text, and again hide it (on an open thread's first line, once shown, Tab folds the thread) |
| `Shift-Tab` | fold all files, or unfold all if all are folded |
| `Ctrl-w c` | show / hide the comment pane on the right: comments beside their lines, in unified and split view (thread tags on their own line there). Its tabs (`gn` / `gp` go round them, or a click on one: they're on the pinned title, over the pane): `Comments` (the whole diff), `Claude` (the Claude view), `Pending` (the pending view), each with how many threads it has (none: no number); an unpublished replacement or new PR has only `Claude`: every comment there is Claude's or yours, so it's the whole diff, with Claude's summary before the files (what it replaces, or what you asked for, heads its description's section). The PR's text is attached to its description: beside it in the pane (as comments are beside their lines; with the pane hidden, after it): its facts in a box (`status · on GitHub ↗`: Enter there opens the PR; each failed or pending check links to its run, `checks` to the PR's checks; `⏸ N workflow runs awaiting approval` when GitHub holds them for a maintainer (a first-time contributor's PR: `Space g` approves them, `W W` in `dv list`); `commits: ✓ all N signed`, or which aren't signed, and whether its base requires signed commits, from GitHub: a commit's signature it verifies, and the base's rules; a long line cut at the box's edge and scrolled sideways, as code in comments); (its reviewers one a line: `✓ @amy approved`, `✗ @bo requested changes`, `● @cy requested`) then the conversation's latest: the comments and reviews new since you looked (after your own last comment or review, and not seen in dv as of when the PR was loaded: one you read stays), the last three at least, and any with a question to Claude under it, each with those questions; first, under its heading, a line says how many older ones there are (newest last), and Enter on it (or `Space o`) shows them all there, in the pane (and again, only the latest); then your pending review's text (with no PR, a branch Claude reviewed, Claude's review); comments on files no longer in the diff come after. In the Claude view, the description, with Claude's summary (or your questions about the PR) attached beside it the same way. Shown by default when the window fits 80 code columns (70 each side in split view) + the pane (a third of the window, 60–80 columns, or all beyond a 200-column code side); otherwise hidden (except in the Claude and pending views), and shown it takes half the window, floating over the code: covering its right end only on rows with a comment. `Ctrl-w f` switches column / floating, `Ctrl-w ,` / `.` resize it by 6 columns, `Ctrl-w =` its usual width. What these keys set (shown or hidden, floating, width) is kept for the next run, in any repo (`$XDG_DATA_HOME/dv/pane.json`), as is `Ctrl-w n`: relative line numbers on / off (over Helix's `line-number` setting once you've set it). Comments from Claude's review file (Claude's, and yours for it: not on GitHub) have a box border in the hint color (in the pane, as a column, boxes are open on their left: their rules come out of the line between the code and the pane, `├─`), and Claude's a `◆` before its name; one answering a GitHub review comment (`"about"`) says `re @who`. Reviewing, Claude reads the PR's review comments (`dv review-comments`, read only) and answers those it has something to add to. The code's right edge marks what comments are about, shown or hidden: `╎` down a comment's lines, a dot at its last (where it's anchored): `●` others', `◆` Claude's, `○` resolved or outdated (in split view, on the side it's on); moving onto a comment (`]c`, a picker) shows the pane. As a column, a dim line between the code and the pane; floating, the pane in the popup color. Code blocks in comments don't wrap: cut at the box's edge with `‹` / `›` where there's more, scrolled sideways (that block alone) as the cursor moves along them. Tables wrap their cells within columns narrowed to fit (to 20 columns at least); wider still, they scroll the same way |
| `Ctrl-w e` | show / hide the changed files' list on the left: files under their folders, the one the top of the code is in marked (`▌`), `✓` when viewed, `·N` its open comments; a click goes to one (unfolding it), the wheel scrolls it. Shown by default from 150 columns, never under 100; what you set is kept (`pane.json`). Scrolled into a file, its header stays pinned on the code's top line. Under the files, the PR's commits (`COMMITS`, oldest first, first parents: `git log full_base..HEAD`): `all commits` (the whole diff), then each. In the list, `j` past the last file goes into them, `Tab` goes between the files and the commits, and `Ctrl-w j` / `Ctrl-w k` go straight to the commits / the files from anywhere; there `j` / `k` move, `Enter` shows the one under the cursor alone (the diff from the commit before it to it), `v` marks a range's start and `Enter` shows the range, `m` the commit's whole message (author, date) in a popup, `y` copies its hash (as `Space c` does on a commit's message in the diff); while the list has focus, its key hints are its own; a click shows one, Shift-click the range from the one shown to it. Shown, it's in the heading (`COMMITS 2–3 of 3`) and the status bar (`commits a..b`); highlighting reads the files at that commit. Comments are on the PR's head: hidden (and none can be made) on commits before the last. Before the files, each like one (a header, `Tab` folds it): the PR's description (its facts, latest conversation and your pending review's text attached), then the message of each commit shown, wrapped to the code's width. The list is their contents (`CONTENTS · N files`): those sections first (`≡ description`, `≡ <hash> <subject>`), then the files; a click goes to either, and with the list focused `j` / `k` move its own cursor (the diff stays where it is) and `Enter` goes there (`l`, `Ctrl-l`, `Esc`: back without going) |
| `Ctrl-w v` | toggle split view: old ‖ new side by side (removed runs pair with the added lines after them) |
| `Ctrl-w h` `Ctrl-w l` (or `Ctrl-h` `Ctrl-l`) | the pane to the left / right, through all of them: the files' list (`Ctrl-w e`), the old and new sides in split view, the comments pane. In the files' list `j` / `k` go to the next / previous file (its header at the top), `Ctrl-l`, `l`, `Enter` or `Esc` back to the code; other keys leave it and act as usual. The pane that has focus has the cursor line's tint, and only it: the code, the comments pane (beside a comment the code isn't tinted as the cursor's line, nor the comment beside the code's line; the code that comment is about has a tint of its own: Helix's `ui.cursorline.secondary`, else half the cursor line's), or the files' list (its selection; the code's line then untinted). Borders don't change color with focus |
| `Ctrl-w ,` `Ctrl-w .` `Ctrl-w =` `Ctrl-w q` | the pane you're in (the files' list or the comments pane; in the code, the comments pane) narrower / wider (the list by 4 columns, the pane by 6), back to its usual width, hidden (`q`, as `Ctrl-w e` / `Ctrl-w c` do; the code stays). Widths are kept for the next run (`pane.json`) |
| `Enter` | open `hx file:line:col` (inside zellij: in a pane on the right, reused for the next file while it is open; dv stays usable); dv reloads whenever the working tree changes; on a link, open the link |
| `gf` | open the link under the cursor in the browser (`xdg-open` / `open`); a tooltip shows the URL while the cursor is on a link. On a `path:line` in text (`src/a.rs:12`, `:12-15`, `:12:5`, `#L12`; the path from the repo root, or the end of one if that's unambiguous): go there, in the diff if it's shown (unfolding its file), else in hx; `Ctrl-o` comes back. The skill has Claude cite code that way. On a name (in a `code` span, or one that looks like code: `snake_case`, `camelCase`): where it's defined, by `git grep` for `fn`/`struct`/`class`/`def`/`function`/… name, the diff's files first; a picker of several |
| `gd gr gy gi`, `Space k` | definition / references / type / impl, hover. On a name in a comment or the PR's text: found by rust-analyzer's `workspace/symbol` (exact matches), then references / implementations from that definition; without rust-analyzer, `gd` searches by text as `gf` does |
| `Space k` on an issue/PR link | preview it in a popup: state, author, age, comments, labels, body (the link tooltip says so) |
| `Ctrl-o` | jump back |
| `c` | comment on the selection, in your pending review |
| `r` | reply on a thread, in your pending review (on a Claude comment: reply to Claude; on Claude's review summary: reply to it, a thread under the summary, `"conversation": "summary"` in the review file, which Claude answers as it does replies) |
| `C` | `c` / `r`, but post immediately (not beside a pending review: GitHub refuses that, so dv says so first); in the PR description/conversation: a new conversation comment quoting the comment under the cursor (`c` there: quote it in your review summary) |
| `s` | suggest a change: `$EDITOR` opens with a ```suggestion block of the selected new-side lines (pending review) |
| `S` | submit the pending review (comment / approve / request changes) or discard it; under the choices, what's still open (Claude's open comments and how many it answered, unresolved GitHub threads, pending comments on changed code, a checkout that isn't the PR head), also as hint lines when approving |
| `Space r` | resolve / unresolve the thread under the cursor |
| `Space e` | edit your comment under the cursor (opens its text in `$EDITOR`) |
| `Space d` | delete the comment under the cursor (confirmation picker, "No" preselected) |
| `Space s` | a shell (`$SHELL`) in the checkout's directory (a PR's worktree): in zellij a floating pane (`zellij run --floating --close-on-exit`), dv working on beside it; otherwise in dv's terminal, dv back when it exits |
| `Space l` | the PR's requested reviewers, labels and assignees: a picker of who can review it (the repo's assignees, not its author), the repo's labels (`gh label list`) and who it can be assigned to, `✓` on those it has (asked of GitHub as it opens); Enter adds or removes one (`gh pr edit --add-reviewer` / `--remove-reviewer` / `--add-assignee` / `--remove-assignee`; a label through the API, which takes a name with a comma whole), then dv refreshes. Teams aren't offered as reviewers, but a review requested of one (`org/team`) is listed, and can be withdrawn. Its assignees are among its facts: `assigned @amy` |
| `Space =` | react to the GitHub comment under the cursor (a review thread's, the PR conversation's, or the PR itself on its description: its heading or text): a picker of GitHub's eight (👍 👎 😄 🎉 😕 💖 🚀 👀; the heart as 💖, which every terminal draws 2 columns wide), each with its count and `✓ yours` on those you've given (asked of GitHub as it opens); Enter adds it, or takes yours back. Reactions show on each comment's author line, after when (and where): `· 👍 2  🎉 1` |
| `Space p` | the threads picker on `pending`: your pending review's comments, each one (replies too); jumps there (unfolding as needed); `Ctrl-d` deletes the selected one (asks first) |
| `Space a` | run Claude Code: review (or update the review: after new commits on top of the reviewed one, just those), answer your replies, or make the changes the open comments ask for (Claude's and yours; your instructions optional, or the whole request when there are none; on someone else's PR: suggest them, as suggestion blocks, without editing; or make them all the same, as commits here, for you to push to their branch with `Space g`, which works when they allow maintainer edits, as the menu says). Also: Claude's notes, dismiss its outdated comments, start a fresh session for the PR, check on Claude now (its review file read again, its run's state), and while a run is going, stop it (kept: its log, and the session, which the next run resumes; the status bar shows how long it's run). Background runs may also run `cargo check/build/test/clippy`, and say what they cost |
| `Space A` | the threads picker on `Claude`: Claude's comments (`[waiting]`: your reply is unanswered, `[answered]`: Claude answered you; severity tags like `[bug]`); jumps there |
| `Space t` | the threads picker: all review threads (state tags, author, gist, reply count); type to filter, e.g. `resolved`. `Tab` / `Shift-Tab` go round its filters, named in its title with the one shown in brackets: `all`, `unresolved`, `pending`, `Claude`, `new` (what's typed stays); `Space p` / `Space A` / `Space n` open it on theirs |
| `Space v` | mark / unmark the file as viewed on GitHub (folds it and moves to the next file); viewed files start folded, headers show `✓ Viewed` / `↻ changed since viewed` at the right edge |
| `]c [c` | next/prev comment thread |
| `]u [u` | next/prev unresolved thread (GitHub's or Claude's, outdated ones too), wrapping around |
| `Claude` tab | Claude view (the pane's `Claude` tab): only Claude's review: the PR's first info line (state, author, branches, size), Claude's summary in full (a section before the files, like the description; not in the pane's summary box then), and the files it commented on, each comment with its lines ± 3 (other threads hidden; `+`/`-` show more or less around the comment under the cursor, kept apart from the whole diff's, and `Z` the whole file; going to a thread that isn't Claude's leaves it). Again: the whole diff |
| `Pending` tab | Pending view (the comments pane's `Pending` tab): as the Claude view, for your pending review: the PR's first info line, the review's text so far, and the threads with a pending comment, each with its lines ± 3. Again: the whole diff |
| `]n [n`, `Space n` | next/prev thread tagged `[new]` (comments you haven't seen, not your own: seen once the cursor leaves the thread or has stayed on it for 3 s, remembered in dv's directory, `seen/`), and the threads picker on `new` |
| `]p [p` | next/prev comment of your pending review (replies too), wrapping around; opens folded files and threads |
| `]a [a` | next/prev Claude comment, wrapping around; folded files and threads are opened on the way |
| `a` | on code lines: a comment for Claude (in the review file, `"author": "user"`), which Claude answers right away; `r` continues the thread. On a GitHub review thread: ask Claude about the comment under the cursor (your editor has it quoted, with its link; the comment for Claude goes on that thread's lines with `"about": <its id>`, and Claude reads it with `dv review-comments`). On Claude's review summary: a reply to it (as `r` there), quoting what's selected of it. On a thread of Claude's review (its comment, or a question and its answer): a reply there (as `r`), quoting what's selected of it. On a commit in `Ctrl-w e`'s list: a question about it, or a change to it (what you want first, as on lines), on no lines: `"conversation": "<the commit's URL>"` (`commit:<sha>` with no PR), listed under `about commit <sha>`; Claude makes a change to it in the code, uncommitted, for `f` there to fold in. A question about the PR in general: `Space a` → "Ask Claude a question about this PR" (a thread like a reply to the summary, `"conversation": "summary"`; with no review yet, the Claude tab shows it under `questions for Claude`), answered at once. On a comment in the PR's conversation (attached to the description, or `Space o`): ask Claude about it, the same way (quoting what's selected of it, else all of it); the question is on no lines (`"conversation": "<its URL>"` instead of `path`/`line`), and it and Claude's answer show as a thread under that comment. On the PR's description (its section): Claude proposes a new one, as your instructions say (`"description"` at the top of the review file); the section shows it against the current text, line by line, and `Space a` approves it (after a confirmation it replaces the PR's on GitHub, `gh pr edit`), asks for more changes (`a` there too), or discards it. On Claude's closing comment (its section): changes to it. Closing a PR: `Space a` → "Ask Claude to draft a comment closing the PR" (after your editor, for why; on an open PR) has Claude write one, `"closing"` at the top of the review file, shown as a section before the rest, `closing comment · Claude's draft`; `Space a` then closes the PR with it, after your editor has it to change (empty, or `:cq`, closes nothing; `gh pr close --comment`), asks for changes, or discards it. On a replacement's description or closing note: a redraft, its instructions starting from a quote of those lines ("About the closing note: > …") |
| `a` / `r` for Claude, first | a picker of what you want of Claude: `Ask` (it answers, changes nothing), `Suggest` (a suggestion block for you to post, no code changes), `Change` (it edits the code here; on someone else's PR you push it with `Space g`). Saved as `"want"` on your comment or reply in the review file; the thread shows `[ask]`/`[suggest]`/`[change]` while it waits for Claude, and Claude does that, whoever's PR it is. Preselected: what you last wanted on the thread, else `Change` on your PR, `Suggest` on theirs. Not on questions about the PR, a conversation comment or a GitHub comment (`a` there): those go straight to the editor, with no `"want"`. Your comments marked `ask` or `suggest` aren't changes for "make the changes" |
| `r` on a Claude comment | reply to Claude: saved in the review file, then Claude answers right away in the background (`claude -p`, resuming the last run's session; replies made meanwhile get one more run) |
| `c` / `C` on a Claude comment | copy it into your pending review / post it now; dismissed from the file once posted |
| `R` | refresh: re-run `git diff`, re-fetch threads + PR info |
| `Space g` | git: show only what isn't committed (diff against HEAD, to check Claude's changes) / the whole diff again; commit everything (new files too; the message starts from Claude's "Changed: …" replies since the last commit); amend it into the PR's commits it belongs in: a picker shows where each change (hunk) goes first: the PR's commit that wrote all the lines it changes (`git blame`), else `✗` (new lines, or lines from two commits: it stays uncommitted); "Ask Claude where the other N belong" has Claude answer for those (in `amend/` beside `reviews/`, for that HEAD; the picker opens again with them, `(Claude: why)`), and "Amend" does it: HEAD kept on `dv-backup/pr-N-<sha>`, a fixup commit per commit, `rebase --autosquash` from the merge-base (a conflict: all back as it was), the rest left uncommitted, then force-push with a lease (this item); the commits list's last row, while anything isn't committed, is `uncommitted · N staged, N changed, N new` (`git status`: in the index, changed and not staged, untracked; those there are): Enter there shows only what isn't committed (as this does); `f` on a commit in `Ctrl-w e`'s list folds everything uncommitted (new lines too) into that one (amends it); rebase onto the base branch (`origin/<the PR's base>`; how many commits it's behind, as last fetched): fetches it, keeps HEAD on `dv-backup/pr-N-<sha>`, and rebases (not with uncommitted changes); stopped on conflicts, Claude resolves them in the background (`claude -p`, allowed `git status/add/rm` and `git rebase --continue/--skip/--abort`; no editor) and carries the rebase on, then checks it builds; dv says how it ended. While a rebase is stopped, `Space g` offers "Ask Claude to finish the rebase" and "Abort the rebase"; push to the branch it tracks (a PR's, under its own name), or from a detached `dv N -w` worktree to the PR's branch in its repo (a fork's too, as `origin` is reached, SSH or HTTPS); after a rebase or amend (HEAD doesn't contain the PR's head), force-push with a lease on the PR's head as dv knows it; or take the branch as it is on GitHub instead (behind: just update; diverged: after a confirmation, the commits only here kept on `dv-backup/pr-N-<sha>` first, then `reset --hard` to GitHub's; not with uncommitted changes); mark a draft PR ready for review (`gh pr ready`, only once its checks pass: none failing or still running); merge (ends the review) |
| `Space m` | the status bar's messages so far, in full, and the last background Claude run's log and the runs' cost |
| `Space g` → Update the checkout | update the checkout: to the PR's head as `gh pr checkout` does (fast-forward only; local commits are kept), or `git pull --ff-only` without a PR; then refresh |
| `?` | key help overlay (two columns when the window fits them); `/` inside it filters entries (smart case), Enter keeps, Esc clears |
| mouse | wheel scrolls what's under it (view, popup, picker), each stopping at its own end: the comments tab the code with it, while there are comments that way; click moves the cursor (or folds, on a fold arrow; picks in a picker, opens on a second click); drag selects. `--no-mouse` leaves it to the terminal |
| `q` | quit |

`dv list`'s first tab, **todo**, is what needs you, in the order to take it (`todo.rs`,
`list/todo.rs`). dv finds what's waiting on you, from its own files and one GraphQL query (your
latest review and the commit it was on, review requests, mergeability, the head commit): a
replacement to publish or to close the originals for, work in its worktree that isn't on GitHub
(uncommitted files, untracked too, and commits the PR doesn't have, but not a checkout the PR has
merely moved past: looked at on refresh, back from a PR and when Claude's done; when Claude made
it, saying what it `Changed:` in its review file after the PR's head commit, it's its own need,
first: "Claude made changes, not pushed … check them, push them"; a run in a zellij pane counts as
done once Claude Code there stops working, its pane still open: working is its spinner line, a
word then how long, `· Moseying… (59s · …)`, whatever the word), a push since your
review, Claude answering
you, your review requested, approved + mergeable + checks green, your draft with its checks green
and nothing unpushed (mark it ready for review: `Space g` in it), unsigned commits where its base
requires signed ones (not someone else's draft), failing checks (yours to fix, or the author's;
not someone else's draft; on yours, a draft too, Claude's part is to fix them: `a a` has it read
the failed runs' logs, `gh run view --log-failed`, and fix the cause in the PR's worktree on its
branch, uncommitted, for you to check and push; not while changes wait to be pushed; `Space a` in
the PR does the same; when it finds it isn't the PR's doing, a flaky test, CI itself, the base, it
changes nothing and writes `ci/pr-N.json` in dv's directory, `{"head", "why", "task"}`: then the
need says so, Claude's part is done at that head, and `N N` on it, or `Space a` in the PR, starts
a new PR fixing CI from its task, after your editor, and `a a` has Claude try again all the same
(the note goes; it writes it again if it still finds so); `C` in `dv list` turns auto on, kept in
git config `dv.auto` and shown on the top bar, ` auto: CI · review · rank `: then Claude does its
part on each item by itself, as `a a` would have it (fixes your failing CI, reviews, updates its
review, answers you; one batch at a time), a draft of yours with its checks green is marked ready
for review, each once a head, a published replacement with its checks green closes what it
replaces, each with its closing note (as `S` → close would), and the list is ranked again when something's new or changed, at
most every 15 minutes; not items deferred or waiting, nor ones Claude's at already), workflow runs awaiting a maintainer's approval (`W W`; its card says
`⏸ N to approve`), conflicts with its base (ask for
a rebase: `b b` in `dv list` posts a comment saying so, after your editor, then defers the item
until it changes), Claude's review new to you (since
you last left the PR in dv), Claude's comments you haven't touched, no review from you (not your
PRs, not drafts), else activity since you looked. It orders them by the most pressing of those,
then the longest waiting; then Claude (one read-only `claude -p`, `gh pr view/diff` only, the repo's
notes in its prompt) puts them in order with a few words on each ("why — what next", the card's
third line; what holds the PR up on GitHub before them, in short, as its ranking may be older:
`[⏸ 3 runs to approve (W W) · ✗ 2 checks failing · 1 unsigned commit · conflicts]`), saved in dv's directory (`todo.json`) with what it ranked. Items it didn't rank (new,
or changed) go before the first ranked one that needs you less, their cards saying "since Claude
ranked". Whatever the order, a PR with CI still running (a replacement's: on its PR) goes
after the rest, as what it needs mostly waits on CI, before what Claude's at, and back up once
CI's done. Claude ranks only when you ask, as it costs a run: `r r` there (`R` refreshes from GitHub
alone). To judge whether that's worth it, the header says how old the last ranking is, what it
cost (Claude Code's reported cost, kept with it), and what's changed since (new, changed, seen to),
and the first `r` says the same. Nothing else starts by itself: a card says what Claude could do
there (answer your replies waiting on it, update a review behind the PR's head, or review one
that has none), and `a a` on it (or marked ones) does that, as it runs reviews (worktree,
session). Enter opens the PR (a replacement, in its worktree); back in the list, what you saw to
drops off. A PR Claude is at (from any dv: a PR's view writes a marker, `running/pr-N.json` in
dv's directory, with its process or its zellij pane's session, read every couple of seconds while
that's alive) says so on its card everywhere ("claude: making the changes…"), and on the todo tab
goes to the bottom, out of the count, until Claude's done: there's nothing for you till then. So
does a PR whose replacement Claude is drafting ("drafting a replacement"), and a replacement Claude
is at (drafting it, or working in a pane in its worktree); once drafted and Claude done, the
replacement, ready for you to review, goes first. A Claude Code pane counts by its folder (a PR's
worktree or a replacement's) and what its screen shows, whatever started it (a shell, too). `s` defers an item, then `1` an hour, `4` four hours, `d` a day or `w` a week, or `u` until
something happens, written in your editor: another PR merged or closed, its checks green, an
approval (from someone, or anyone), someone's review, a time. dv reads the usual phrasings ("#2380
is merged", "CI is green", "@amy approves", "3 days") itself, and has Claude put the rest as one of
those (one small run with no tools; if it can't, nothing's deferred); dv then watches for it from
what it fetches anyway, and the item's back once it's happened, the status bar saying so. `p` posts
what an item waits on as a comment on its PR ("Waiting on: #2380 is merged."), and such a comment,
by you or anyone with write access, defers the PR on everyone's todo tab ("Waiting on", "Blocked
on/by", "Depends on", "Deferred until"; `#N` alone means merged; a later "Unblocked" ends it), as
does a wait Claude saw in a PR's conversation when it ranked (`waits_on`). Any other key after `s`:
not deferred. A time deferral: it goes to the bottom, out of the count ("deferred, back in 4h"), until then, or
until what it needs changes (its needs are kept with it: new activity is new information); `s` on
it brings it back. Deferrals are kept in dv's directory (`deferred.json`). Deferred items aren't
ranked.

Opening a PR that's checked out already (its `<repo>-N` worktree, or its branch here) updates it
to the PR's head, except over work done there since: commits the PR doesn't have (yours, or
Claude's: a commit, an amend, a rebase) are kept, `-f` or not, and dv says how many and how to
push or drop them. dv records the commit it last checked out (`checkouts/pr-N` in its
directory), so a PR its author force-pushed still updates. Worktrees made for Claude's runs in
`dv list` follow its `-b`.

Merging: `M M` on a PR in `dv list`, or `Space g` → merge in its review (which ends the review
first), runs Bitcoin Core's `github-merge.py` (`merge.rs`). Not when GitHub would refuse the push:
unsigned commits where the base requires signed ones (a ruleset, or classic branch protection),
as one GraphQL query tells (if it can't, the script runs as it would). dv builds the script in
(`vendor/github-merge/`, pinned to an upstream commit; a test checks its SHA-256) and writes it to
its data directory when it isn't exactly that, so a merge downloads nothing; `git config
dv.mergescript <path>` runs yours instead. It runs in the main checkout, as `python3 <script> N`, with git config for that run only (`GIT_CONFIG_*`) for what you haven't set: `githubmerge.repository`, and `user.ghtoken` from `gh auth token` (fetched as it starts, not written in its command), so its GitHub requests are authenticated (60 an hour without);
with `githubmerge.repository` set for that run (`GIT_CONFIG_*`) if you haven't set it. The script
checks out the base branch, which the PR's worktree would be in the way of, so that goes first:
only if it has no local changes (untracked files count) and no commits beyond the PR's head on
GitHub; else dv stops and says what's there. In zellij the script gets a pane of its own, which
closes when it succeeds (failing, or rejected, it says so and waits for Enter, for you to read why);
a merge that succeeds deletes the script's branches for the PR (`pull/N/base`, `pull/N/head`, …: a
failed one keeps them, to look into); elsewhere the terminal, and dv waits for Enter after it.

`dv list` (the 1000 most recently updated open PRs; a `#N` filter's Enter opens PR N even if it
isn't listed; `Tab` switches to the PRs with a local
`<repo>-N` worktree, open or not, which `Enter` opens in that worktree as it was made:
detached or on its branch) shows each PR as a card, a blank line between cards. It keeps up
with GitHub in the background: the open PRs' ETag every 5s, as a PR does (a 304 is free), fetching them
again when it changes (a push, comment, review, new or closed PR) and every 5 minutes regardless
(for CI), and again on coming back from a PR; the cards change in place (the selected one stays
selected, CI counts kept until the new ones come). `R` fetches now. In a window 110 columns or wider, three
lines in two zones (within 140 columns): on the left the mark (`m`: a `●` in a column of its own,
before the number, so marked cards line up), the number, title (bold), badges and `[new]`;
author, size, labels; base ← branch and its worktree. On the right, a 30-column zone lining up
from card to card: CI and the age; the review decision; Claude's part (those two may run on to
the window's edge: why Claude failed, in full). Narrower, four lines:
number, title (bold), draft badge, `[new]`, age; author, size, labels; CI (✓/✗/● counts, fetched
in the background since they're slow), review decision and Claude's part, in columns; base ←
branch and its worktree. The selected card keeps its colors on a quiet background (the theme's
cursor line), with a bar down its left edge. `j k`, `gg ge`, `Ctrl-d Ctrl-u` move;
`o` cycles the order (created, the default / number / size / updated, newest or biggest first), `O`
reverses it. `/` edits the filter (Enter keeps, Esc clears): space-separated terms that must
all match, GitHub-search style, `-` negating one: `author:` `label:` `base:` `branch:`
(substring), `is:draft`, `review:approved|changes|required|none`, `ci:pass|fail|pending|none`,
anything else is text in the number, title, author, branch or labels (smart case). E.g.
`ci:fail -is:draft -author:dependabot`. `Enter` reviews the PR as `dv <N> <flags>` would, `R`
refreshes, `q` quits. The mouse: a click selects a card, a double-click opens it, a click on a
tab switches to it, the wheel moves the selection; the tabs as the PR view's (the one shown lit).
`m` marks PRs (`U U` unmarks them all; kept in dv's directory, `marked.json`, so they outlast opening a PR and `dv list`
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
left instead, off the comments. With the contents list shown, these hints are pinned at its
bottom instead (`KEYS: <what>`, a line each, cut to its width; at most half its height), not
over the code; its commits follow its files right after (a blank line between), and under its
heading the two scroll as one list, following the list's cursor (or where the diff is), the
wheel scrolling it; the hints stay where they are. The status bar's left
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
