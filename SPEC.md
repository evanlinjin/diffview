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
3. **PR comments**: `gh api .../pulls/N/comments`, threaded inline under their
   lines. A new comment attaches to the **selected lines** (GitHub multi-line
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
   from `$EDITOR` (`:cq` cancels), discard asks for confirmation. Pending comments are
   tagged `[pending]` and counted in the status bar. If posting fails, the text is saved
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
   **Claude reviews**: a review file at `<git common dir>/dv/reviews/pr-<N>.json` (or
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
   from the file; `Space r` resolves it in the file, `Space d` dismisses it. `Space a` runs
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
   notes for Claude live in `<git common dir>/dv/notes.md` (`dv review-info` prints the path; the
   skill reads them first and keeps them current; `Space a` opens them).
4. **Later**:
   - **`c` in the PR section → review summary.** `c` on a conversation comment (or the
     description) appends it as a `> ` quote to the pending review's summary (body), saved on
     GitHub (`updatePullRequestReview` on the pending review, or `addPullRequestReview` with a
     body if there is none); `S` then opens the summary editor pre-filled with that draft.
     Keeps `c` = queue / `C` = post now (`C` there already quotes into a new conversation
     comment). Until then `c` in the PR section only says "select code lines".

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
| `Ctrl-w c` | comments in the right margin (default when the window fits 80 code columns + a 60-column margin; single-pane view) / inline |
| `Ctrl-w v` | toggle split view: old ‖ new side by side (removed runs pair with the added lines after them) |
| `Ctrl-w h` `Ctrl-w l` (or `Ctrl-h` `Ctrl-l`) | split view: move to the old / new side |
| `Enter` | open `hx file:line:col` (inside zellij: in a pane on the right, reused for the next file while it is open; dv stays usable and reloads as you save); on a link, open the link |
| `gf` | open the link under the cursor in the browser (`xdg-open` / `open`); a tooltip shows the URL while the cursor is on a link |
| `gd gr gy gi`, `Space k` | definition / references / type / impl, hover |
| `Space k` on an issue/PR link | preview it in a popup: state, author, age, comments, labels, body (the link tooltip says so) |
| `Ctrl-o` | jump back |
| `c` | comment on the selection, in your pending review |
| `r` | reply on a thread, in your pending review (on a Claude comment: reply to Claude) |
| `C` | `c` / `r`, but post immediately; in the PR description/conversation: a new conversation comment quoting the comment under the cursor |
| `s` | suggest a change: `$EDITOR` opens with a ```suggestion block of the selected new-side lines (pending review) |
| `S` | submit the pending review (comment / approve / request changes) or discard it |
| `Space r` | resolve / unresolve the thread under the cursor |
| `Space e` | edit your comment under the cursor (opens its text in `$EDITOR`) |
| `Space d` | delete the comment under the cursor (confirmation picker, "No" preselected) |
| `Space p` | picker of your pending review's comments; jumps there (unfolding as needed); `Ctrl-d` deletes the selected one (asks first) |
| `Space a` | run Claude Code: review (or update the review), answer your replies, or make the changes its open comments ask for (with your own instructions, optional) |
| `Space A` | picker of Claude's comments (`[waiting]` = your reply is unanswered); jumps there |
| `Space t` | picker of all review threads (state tags, author, gist, reply count); type to filter, e.g. `resolved` |
| `Space v` | mark / unmark the file as viewed on GitHub (folds it and moves to the next file); viewed files start folded, headers show `✓ Viewed` / `↻ changed since viewed` at the right edge |
| `]c [c` | next/prev comment thread |
| `]p [p` | next/prev comment of your pending review (replies too), wrapping around; opens folded files and threads |
| `]a [a` | next/prev Claude comment, wrapping around; folded files and threads are opened on the way |
| `a` | on code lines: a comment for Claude (in the review file, `"author": "user"`), which Claude answers right away; `r` continues the thread |
| `r` on a Claude comment | reply to Claude: saved in the review file, then Claude answers right away in the background (`claude -p`, resuming the last run's session; replies made meanwhile get one more run) |
| `c` / `C` on a Claude comment | copy it into your pending review / post it now; dismissed from the file once posted |
| `R` | refresh: re-run `git diff`, re-fetch threads + PR info |
| `?` | key help overlay (two columns when the window fits them); `/` inside it filters entries (smart case), Enter keeps, Esc clears |
| `q` | quit |

`dv list` (the 200 most recently updated open PRs; `Tab` switches to the PRs with a local
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
refreshes, `q` quits. Quitting the review returns to the list; if opening a PR fails, the
error shows in the list's status bar.

Discoverability: after a prefix key (`Space`, `g`, `z`, `]`, `[`, `Ctrl-w`, `m`) a which-key
popup lists what can follow, as in Helix. The same bottom-right popup otherwise says what the
keys do where the cursor is: on a selection, a review thread, Claude's comment, the PR header
or PR text (nothing on plain code). The status bar's left
side (what you're on, messages, hints) wins over its right: when both don't fit, the right
keeps only the essentials.

## Design
One crate: `main.rs` (args), `diff.rs` (git + parser), `view.rs` (state, keys,
render), later `lsp.rs`, `gh.rs`. Deps: `ratatui`, `anyhow` (+ `serde_json` for
phases 2–3). Shells out to `git`/`gh`; no git2, no GitHub crate.

Not planned: editing in dv, forges other than GitHub, other LSPs.
