# dv — PR diff viewer with Helix keys + rust-analyzer

```
dv              # working tree vs merge-base(origin/HEAD)
dv <base>       # working tree vs merge-base(<base>)
dv --pr 123     # gh pr checkout 123 first (-f: reset the local PR branch to the PR head)
dv --pr 123 -w  # check it out in a worktree next to the repo (<repo>-123), detached; reused later
dv --pr 123 -w -b  # same, on the PR's branch (tracking it, for pushing)
dv 123 -wb         # shorthand: a bare number is a PR; short flags combine (-p N = --pr N)
dv 123 --rm        # remove that worktree (-f if it has local changes; a -b branch is kept)
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
   File headers: dir dimmed, +/− colored. Split view hatches the empty side; pickers
   highlight matched letters.
4. **Later**:
   - **`c` in the PR section → review summary.** `c` on a conversation comment (or the
     description) appends it as a `> ` quote to the pending review's summary (body), saved on
     GitHub (`updatePullRequestReview` on the pending review, or `addPullRequestReview` with a
     body if there is none); `S` then opens the summary editor pre-filled with that draft.
     Keeps `c` = queue / `C` = post now (`C` there already quotes into a new conversation
     comment). Until then `c` in the PR section only says "select code lines".
   - **`dv list`**: a picker of the repo's open PRs (`gh pr list`); `Enter` opens the chosen
     one as `dv <N>` would, with `dv list`'s own flags: `dv list -wbf` enters each PR in a
     worktree, on its branch, force-reset to the PR head.
   - Multiple cursors.

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
| `n N`, `]g [g` | next/prev change (hunk) |
| `]f [f`, `Space f` | next/prev file, file picker |
| `/`, `Alt-n Alt-N`, `*` | search (smart case), next/prev match, search for the selection |
| `y` / `Space y`, `Y` | copy text, copy as patch (via OSC 52) |
| `Space c` | copy a `path:line` / `path:start-end` ref for the selection |
| `+`/`=`, `-`, `Z` | context ±10 lines around the hunk under the cursor (hunks merge as they meet), whole file |
| `Tab` | collapse/expand file; on a comment, fold/unfold its thread |
| `Shift-Tab` | fold all files, or unfold all if all are folded |
| `Ctrl-w v` | toggle split view: old ‖ new side by side (removed runs pair with the added lines after them) |
| `Ctrl-h Ctrl-l` | split view: move to the old / new side |
| `Enter` | open `hx file:line:col` (in a floating pane inside zellij), reload diff afterwards; on a link, open the link |
| `gf` | open the link under the cursor in the browser (`xdg-open` / `open`); a tooltip shows the URL while the cursor is on a link |
| `gd gr gy gi`, `Space k` | definition / references / type / impl, hover |
| `Ctrl-o` | jump back |
| `c` | comment on the selection, or reply on a thread, in your pending review (`r` = reply) |
| `C` | same, but post immediately as a single comment; in the PR description/conversation: a new conversation comment quoting the comment under the cursor |
| `s` | suggest a change: `$EDITOR` opens with a ```suggestion block of the selected new-side lines (pending review) |
| `S` | submit the pending review (comment / approve / request changes) or discard it |
| `Space r` | resolve / unresolve the thread under the cursor |
| `Space e` | edit your comment under the cursor (opens its text in `$EDITOR`) |
| `Space d` | delete the comment under the cursor (confirmation picker, "No" preselected) |
| `Space p` | picker of your pending review's comments; jumps there (unfolding as needed) |
| `Space t` | picker of all review threads (state tags, author, gist, reply count); type to filter, e.g. `resolved` |
| `Space v` | mark / unmark the file as viewed on GitHub (folds it and moves to the next file); viewed files start folded, headers show `✓ Viewed` / `↻ changed since viewed` |
| `]c [c` | next/prev comment thread |
| `R` | refresh: re-run `git diff`, re-fetch threads + PR info |
| `?` | key help overlay; `/` inside it filters entries (smart case), Enter keeps, Esc clears |
| `q` | quit |

## Design
One crate: `main.rs` (args), `diff.rs` (git + parser), `view.rs` (state, keys,
render), later `lsp.rs`, `gh.rs`. Deps: `ratatui`, `anyhow` (+ `serde_json` for
phases 2–3). Shells out to `git`/`gh`; no git2, no GitHub crate.

Not planned: editing in dv, forges other than GitHub, other LSPs.
