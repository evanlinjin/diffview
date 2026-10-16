# dv — PR diff viewer with Helix keys + rust-analyzer

```
dv              # working tree vs merge-base(origin/HEAD)
dv <base>       # working tree vs merge-base(<base>)
dv --pr 123     # gh pr checkout 123 first (-f: reset the local PR branch to the PR head)
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
   PR description, conversation and (unresolved) review comments are highlighted as
   markdown, code fences included. TOML uses a small built-in grammar (syntect has
   none). File headers are a full-width bar with +/− line counts.
4. **Later**: side-by-side, batched reviews
   (approve/request changes), multiple cursors.

## Keys
| Key | Action |
|---|---|
| `h j k l`, arrows | move (counts work: `5j`) |
| `w b e`, `W B E` | word motions (they select, like Helix) |
| `gh gl gs` | line start / end / first non-blank |
| `f t F T <c>` | find char |
| `gw` | jump labels on visible words (uses your `jump-label-alphabet`) |
| `gg ge`, `Ctrl-d Ctrl-u`, `Ctrl-e Ctrl-y` | top / end, half page, scroll 5 lines |
| `zz zt zb` | put the cursor line at center / top / bottom |
| `x` | select line (repeat to extend) |
| `v`, `;`, `Esc` | select mode / collapse selection / exit select mode |
| `n N`, `]g [g` | next/prev change (hunk) |
| `]f [f`, `Space f` | next/prev file, file picker |
| `/`, `Alt-n Alt-N`, `*` | search (smart case), next/prev match, search for the selection |
| `y` / `Space y`, `Y` | copy text, copy as patch (via OSC 52) |
| `Space c` | copy a `path:line` / `path:start-end` ref for the selection |
| `+`/`=`, `-`, `Z` | file context ±10 lines, whole file |
| `Tab` | collapse/expand file; on a comment, fold/unfold its thread |
| `Shift-Tab` | fold all files, or unfold all if all are folded |
| `Enter` | open `hx file:line:col`, reload diff afterwards |
| `gd gr gy gi`, `Space k` | definition / references / type / impl, hover |
| `Ctrl-o` | jump back |
| `c`, `r` | comment on the selected lines / reply to the thread under the cursor |
| `]c [c` | next/prev comment thread |
| `?` | key help overlay |
| `q` | quit |

## Design
One crate: `main.rs` (args), `diff.rs` (git + parser), `view.rs` (state, keys,
render), later `lsp.rs`, `gh.rs`. Deps: `ratatui`, `anyhow` (+ `serde_json` for
phases 2–3). Shells out to `git`/`gh`; no git2, no GitHub crate.

Not planned: editing in dv, forges other than GitHub, other LSPs.
