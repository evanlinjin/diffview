//! The diff view: state, loading and laying out rows. Keys, motions, rendering, GitHub review
//! actions and rust-analyzer live in the submodules.
use crate::diff::{self, FileDiff, Kind, Line as DLine};
use crate::gh::{self, Anchor, Pr, Thread};
use crate::highlight::{Colors, Highlighter};
use crate::theme::{Settings, Theme, Ui};
use crate::lsp::{self, Loc, Lsp};
use anyhow::{anyhow, bail, Context, Result};
use ratatui::{
    crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    layout::{Alignment, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap},
    DefaultTerminal, Frame,
};
use std::collections::{HashMap, HashSet};
use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

mod agent;
mod amend;
mod rebase;
pub(crate) use agent::{REPLACE_TOOLS, tools_for, answer_prompt, close_if_done, pane_screen, asking, working, claude_command, claude_wrote, fix_checks_prompt, review_prompt, turn, update_prompt, with_instructions};
mod build;
use build::*;
mod files;
mod git;
mod keys;
mod layout;
mod mouse;
mod motion;
mod ra;
mod render;
mod review;
mod select;
mod term;
pub use term::{MOUSE, init_terminal, restore_terminal};
pub(crate) use term::{compose_with, in_zellij};
pub(crate) use review::prefetch_github;
use term::*;
#[cfg(test)]
pub(crate) mod tests;

const HELP: &str = "
MOVE
  h j k l, arrows      move (counts: 5j)
  w b e  W B E         word motions (select)
  gh gl gs             line start / end / first non-blank
  f t F T <c>          find char
  gw                   jump labels
  mm                   matching bracket
  gg ge                top / end (in the comments pane: its first / last)
  Ctrl-d Ctrl-u        half page down / up
  Ctrl-e Ctrl-y        scroll 5 lines
  zz zt zb             cursor line to center / top / bottom
  Ctrl-o               jump back

DIFF
  ]g [g                next / prev hunk (change)
  ]f [f, Space f       next / prev file, file picker
  ]c [c                next / prev comment thread
  ]u [u                next / prev unresolved one (wraps around)
  ]a [a                next / prev Claude comment (wraps around)
  ]p [p                next / prev comment of your pending review (wraps around)
  /  n N  *            search, next / prev match, search selection

SELECT & COPY
  x                    select line (repeat to extend)
  v  ;  Esc            select mode / collapse / exit select mode
  y  (or Space y)      copy text
  Y                    copy as patch
  Space c              copy: a link's URL, file:line, or a comment's / the PR's link

VIEW
  + =  -               hunk context +10 / -10 lines
  Z                    whole file
  Tab                  fold file / PR info / thread
  gn  gp               comments pane: next / previous tab (comments, Claude's, the PR's)
  Shift-Tab            fold / unfold all files
  Ctrl-w v             toggle split (side-by-side) view
  Ctrl-w n             relative line numbers on / off (kept for next time)
  Ctrl-w e             the contents on the left (description, messages, files), on / off
                       and the PR's commits under it: Enter shows one, v … Enter a range,
                       m its whole message, a asks Claude about it (or for a change),
                       f folds what isn't committed into it;
                       Ctrl-w j / Ctrl-w k (or Tab there) go to the commits / the files
  Ctrl-w h  Ctrl-w l   pane left / right: files, old / new side, comments (or Ctrl-h Ctrl-l)
  Ctrl-w c             show / hide the comments pane (╎ ● mark their lines)
  Ctrl-w f             comments pane: a column beside the code / floating over it
  Ctrl-w ,  Ctrl-w .   the pane you're in narrower / wider (Ctrl-w = usual width)
  Ctrl-w q             hide the pane you're in (files' list or comments)
  Enter                open hx at cursor (on a link: open it)
  gf                   open the link under the cursor, or go to the file:line under it
                       (as comments cite code) or to where the function / type named
                       under it is defined (Ctrl-o comes back)

RUST-ANALYZER
  gd gr gy gi          definition / references / type / impl (on a name in a comment too)
  Space k              hover; on an issue/PR link: preview it

ON SELECTED LINES
  c                    comment, in your pending review (C: post it now)
  s                    suggest a change, in your pending review
  a                    a comment for Claude: an answer, a suggestion, or the change you want
                       (it acts right away; not on GitHub); on a
                       replacement's description or closing note: ask Claude to redraft it

ON A REVIEW THREAD
  r                    reply, in your pending review (C: post it now)
  Space r              resolve / unresolve
  Space =              react (👍 🎉 …), or take your reaction back (PR comments too)
  Space l              the PR's labels, assignees, reviewers: add or remove (request reviews)
  Space e  Space d     edit / delete your comment (delete asks first)

ON CLAUDE'S COMMENT
  r                    reply to Claude, saying what you want (it acts right away, in the
                       background)
  c                    copy it into your pending review (C: post it now)
  Space r  Space d     resolve (Claude skips it) / dismiss

ON PR TEXT
  c                    quote it in your review summary (S starts from it)
  C                    quote it in a new conversation comment

ON CLAUDE'S REVIEW (its summary)
  c                    add it to your review summary (S starts from it)
  C                    post it now, as your PR comment

REVIEW
  S                    submit (comment / approve / request changes), or discard
  Space g              git: show only what isn't committed (Claude's changes), commit, amend
                       it into the commits it belongs in, push, update the checkout, rebase
                       onto the base (Claude resolves conflicts)
  Space p  Space t     pick from your pending comments (Ctrl-d deletes) / all threads
  Space v              mark the file viewed (folds it, moves on)
  Space a              ask Claude Code to review, answer your replies, or make the changes
                       (or check on its run now, or stop one that's stuck)
  Space A              pick from Claude's comments ([waiting]: its turn, [answered]: yours)
  ]n [n  Space n       next / prev [new] thread (ones you haven't seen) / pick from them

GENERAL
  ?                    this help (/ to search it)
  R                    refresh (git diff + GitHub)
  Space m              messages so far, in full (the status bar cuts long ones)
  Space s              a shell in the checkout's directory (zellij: a floating pane)
  Space o              the PR's whole conversation on its own (Esc: back)
  mouse                wheel scrolls, click moves (or folds, on an arrow), drag selects
  q, Ctrl-c            quit
";

const DEFAULT_CONTEXT: u32 = 3;
/// Pending-key marker for Ctrl-w (window commands).
const CTRL_W: char = '\u{17}';
/// Files bigger than this aren't highlighted.
const HL_MAX_BYTES: usize = 1 << 20;
/// `Row::file` for the PR info section at the top.
const INFO: usize = usize::MAX - 1;
/// Comment marks at the code's right edge, by rank (the highest wins where they meet).
pub(super) const MARK_STALE_LINE: u8 = 1;
pub(super) const MARK_LINE: u8 = 2;
pub(super) const MARK_STALE_DOT: u8 = 3;
pub(super) const MARK_CLAUDE_DOT: u8 = 4;
pub(super) const MARK_DOT: u8 = 5;

/// `Row::info` for Claude's review summary (it isn't one of `info`'s lines).
const SUMMARY: usize = usize::MAX;
/// `Row::info` for the text of your pending review, in the pending view.
const DRAFT: usize = usize::MAX - 2;
/// `Row::info` for the PR's facts (author, branches, size, reviews, checks), boxed together.
const FACTS: usize = usize::MAX - 3;
/// `Row::info` for the line saying how much more of the conversation there is (`Space o`).
const MORE_CONVERSATION: usize = usize::MAX - 4;
/// How many of the conversation's latest comments are attached to the description, at least.
const LATEST: usize = 3;
/// `Row::main` for the description's section (commit `n`'s message is `1 + n`).
const DESCRIPTION: usize = 0;
/// What a thread in Claude's review file that replies to its summary is on (`"conversation"`).
const ON_SUMMARY: &str = "summary";
/// `Row::main` for Claude's review summary, a section of its own in the Claude view.
const CLAUDE_REVIEW: usize = usize::MAX;
/// `Row::main` for a replacement's closing note (Claude's draft), a section like its description.
const CLOSING_NOTE: usize = usize::MAX - 1;
/// `Row::main` for the comment Claude drafted for closing the PR, a section before the rest.
const CLOSING_PR: usize = usize::MAX - 2;

/// Position in the view: row index into `rows`, char index into the row's text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Pos {
    row: usize,
    col: usize,
}

struct Row {
    file: usize,
    line: DLine,
    /// Index into `threads` for comment rows.
    thread: Option<usize>,
    /// Per-char styles for markdown rows; code rows use `hl_cache` instead.
    syn: Vec<Option<Style>>,
    /// Links in rendered markdown: (start, end, url) char ranges.
    links: Vec<(usize, usize, String)>,
    /// For PR info rows: the index in `info` of the markdown body this row belongs to.
    info: Option<usize>,
    /// The bottom edge of a comment's box (an empty row, drawn as a border).
    edge: bool,
    /// A line of a code block or table in a comment: cut at the box's edge, scrolled sideways.
    nowrap: bool,
    /// PR text as a section of its own (folding as one, in the contents list): which
    /// (`DESCRIPTION`, or `1 + n`: commit `n`'s message).
    main: Option<usize>,
}

impl Row {
    fn new(file: usize, line: DLine) -> Row {
        Row { file, line, thread: None, syn: vec![], links: vec![], info: None, edge: false, nowrap: false, main: None }
    }

    /// A row of plain text (headers, comments, PR info).
    fn text(file: usize, kind: Kind, text: String) -> Row {
        Row::new(file, DLine { kind, old: None, new: None, text })
    }
}

/// What the cursor is on, independent of row indices.
struct CursorKey {
    /// (file, old, new) for a diff row.
    line: Option<(String, Option<u32>, Option<u32>)>,
    /// Root comment id for a thread row, and how far into the thread it is.
    thread: Option<(u64, usize)>,
    /// Row index for a PR info row (the section is at the top and only grows at its end).
    info: Option<usize>,
    /// A section before the files (the description, a commit's message, Claude's summary): its
    /// `Row::main` and how far into it. (By index, the PR text before it growing would move it.)
    section: Option<(usize, usize)>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum BoxPart {
    Top,
    Divider,
    Body,
    Bottom,
}

/// A screen row: full width, a side-by-side pair (old line, new line) in split view, or code
/// (one row, or a pair in split view) with a comment row beside it in the right margin.
#[derive(Clone, Copy, Debug, PartialEq)]
enum DRow {
    Full(usize),
    Pair(Option<usize>, Option<usize>),
    /// (code row or old side, new side in split view, comment row in the margin); a code-less
    /// one is a spacer, where a file's comments need more room than its code has.
    Margin(Option<usize>, Option<usize>, Option<usize>),
}

/// The pane's tabs as they're shown (`pane_tabs`): its comments (the whole diff), Claude's
/// (the Claude view, its comments beside its code), your pending review's.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Bar {
    Comments,
    Claude,
    Pending,
}

/// The comment margin's width: about a third of the window, within these bounds, or everything
/// beyond the code side's most (it doesn't read better any wider); and the least code text
/// width beside it (narrower windows show comments inline).
const MARGIN_MIN: usize = 60;
/// The least code text width beside it for each side of split view.
const MARGIN_SPLIT_MIN: usize = 70;
/// How much `Ctrl-w ,` / `.` change its width by.
const MARGIN_STEP: isize = 6;
/// The least it takes in a narrow window, when it's shown there.
const MARGIN_NARROW: usize = 36;
const MARGIN_MAX: usize = 80;
const MARGIN_CODE_MIN: usize = 80;
const MARGIN_CODE_MAX: usize = 200;

/// Work that needs the terminal back (an editor), run between frames.
enum Action {
    /// A shell in the checkout's directory: a floating pane in zellij, else this terminal.
    Shell,
    Open(String),
    /// `review`: add to the pending review instead of posting immediately.
    /// `prefill`: starting text in the editor (a ```suggestion block for `s`).
    /// `claude`: copying Claude's comment (its id in the review file) into the review; the
    /// prefill is its text, posted as is if left unchanged, and the comment is dismissed after.
    Comment { anchor: Anchor, review: bool, prefill: String, claude: Option<String> },
    /// `quote`: the comment under the cursor as a `> ` quote, pre-filled in the editor.
    Reply { thread: String, comment: u64, review: bool, quote: String },
    /// A reply to Claude on its comment (by id), saved to the review file for Claude to answer,
    /// with what you want of it ("ask", "suggest", "change"; empty: unsaid).
    /// (And what the editor starts from: a quote of what's selected, say.)
    ClaudeReply(String, &'static str, String),
    /// A comment for Claude on these lines (with their text), then Claude answers it; `about`:
    /// the GitHub review comment it asks about (its id, and a quote of it for the editor).
    AskClaude { anchor: Anchor, code: Vec<String>, about: Option<(u64, String)>, want: &'static str },
    /// `a` on a PR conversation comment: a question about it (its URL), starting from `prefill`.
    /// On a commit (its URL): a question about it, or a change to it (`want`).
    AskConversation { url: String, prefill: String, want: &'static str },
    /// Run Claude (review, or make the changes) after your own instructions for it.
    ClaudeWith(agent::Ask),
    /// Have Claude draft a replacement for these PRs, after your instructions for it (starting
    /// from these, else your last ones: `a` on a draft quotes what it's about).
    Replace(Vec<u64>, Option<String>),
    /// Publish the replacement here as a draft PR, after the editor (title, description): (then
    /// close what it replaces, next).
    Publish(bool),
    /// A new PR fixing CI for Claude to draft, after the editor (from Claude's note on this PR).
    CiFixPr,
    /// Close the PR with Claude's draft comment, after the editor.
    ClosePr,
    /// Close the PRs the replacement here replaces, after the editor (the note).
    CloseOriginals,
    /// A new PR conversation comment, starting with `quote` (may be empty); `as_is`: posted
    /// even if left untouched (Claude's review, yours to post).
    Converse { quote: String, as_is: bool },
    /// Rewrite one of your comments (node id, current body).
    Edit { comment: String, body: String, author: String },
    /// Submit the pending review: COMMENT, APPROVE or REQUEST_CHANGES.
    Submit(&'static str),
    /// Commit everything uncommitted, starting from this message.
    Commit(String),
}

/// Network work that doesn't need the terminal; run after a frame shows its "…" message.
enum Job {
    Refresh,
    /// Bring the checkout up to date: the PR's head, or `git pull --ff-only` without a PR.
    Pull,
    Resolve(String, bool),
    /// Mark (true) or unmark a file as viewed.
    Viewed(String, bool),
    /// React to a comment (its node id) with this reaction, or (`false`) take yours back.
    React(String, &'static str, bool),
    Discard(String),
    DeleteComment(String),
    /// Fetch an issue/PR link's preview into a popup.
    Preview(String),
    /// Ask GitHub what reactions a comment (its node id) has, for the react picker.
    Reactions(String),
    /// Fetch the base branch and rebase onto it (Claude resolving conflicts).
    Rebase,
    /// The PR's base changed on GitHub to this one: fetch it, and the diff follows.
    Retarget(String),
    /// Rerun the failed checks' jobs on the PR's head.
    Rerun,
    /// Ask GitHub for the PR's labels, assignees and reviewers, and what it could have.
    Labels,
    /// Add (`true`) or remove a label, an assignee or a requested reviewer, by name.
    SetField(gh::Field, String, bool),
    /// Push HEAD to the PR's branch (`true`: force, with a lease on the PR's head).
    Push(bool),
    /// Send what was written in the editor.
    Post(Post),
}

/// Something written in the editor, to send to GitHub (as a job, so "posting…" shows first).
enum Post {
    /// A new comment on lines (`claude`: copied from Claude's comment, dismissed once sent).
    Thread { anchor: Anchor, review: bool, body: String, claude: Option<String> },
    Reply { thread: String, comment: u64, review: bool, body: String },
    Converse(String),
    /// Your pending review's new text (`c` in the PR section, or on Claude's summary).
    Summary(String),
    Edit { comment: String, body: String },
    Submit(&'static str, String),
}

enum Target {
    Row(usize),
    Loc(Loc),
    Submit(&'static str),
    /// Asks for confirmation first; `DiscardYes` does it.
    Discard,
    DiscardYes,
    DeleteComment(String),
    /// A comment: (thread index, comment index within it).
    Comment(usize, usize),
    Claude(agent::Ask),
    /// What you want of Claude on the comment or reply you're about to write ("ask", …).
    Want(&'static str),
    ClaudeNotes,
    /// Ask Claude a question about the PR (a thread under its summary, in the Claude tab).
    ClaudeQuestion,
    /// Have Claude draft (or redraft) a replacement for these PRs, in the background.
    ClaudeReplace(Vec<u64>),
    /// Publish the replacement here as a draft PR (then close the PRs it replaces) / close them.
    Publish(bool),
    CloseOriginals,
    /// Keep replacement branches on this remote (`dv.replaceRemote`), then publish (as `Publish`).
    ReplaceRemote(String, bool),
    ClaudeFresh,
    ClaudePane,
    ClaudeCheck,
    ClaudeStop,
    GitView,
    GitCommit,
    /// Add (`true`) or remove a label, an assignee or a requested reviewer, by name.
    SetField(gh::Field, String, bool),
    /// React to a comment (node id, reaction), or take yours back (`false`).
    React(String, &'static str, bool),
    /// Update the checkout: to the PR's head (fast-forward only), or `git pull --ff-only`.
    GitUpdate,
    /// The amend picker / amend what isn't committed into the PR's commits / ask Claude where
    /// what blame couldn't place goes.
    AmendMenu,
    Amend,
    AmendAsk,
    /// Rerun the failed checks (their failed jobs).
    GitRerun,
    /// Rebase onto the base branch (Claude resolving conflicts) / Claude carrying a stopped
    /// one on / abort it.
    Rebase,
    RebaseClaude,
    RebaseAbort,
    GitPush,
    /// Force-push (with a lease): HEAD doesn't contain the PR's head (a rebase or amend here).
    GitForcePush,
    /// Take the branch as it is on GitHub (`true`: confirmed, or nothing here would be lost).
    GitTakeTheirs(bool),
    /// End the review and merge the PR with github-merge.py.
    GitMerge,
    /// Dismiss all of Claude's outdated comments (`true`: confirmed).
    ClaudeDismissOutdated(bool),
    /// Mark the PR (a draft) ready for review.
    GitReady,
    /// Have Claude draft a new PR fixing CI, from its note on this PR (after your editor).
    CiFixPr,
    /// Approve the workflow runs waiting on the PR's head.
    GitApproveRuns,
    /// Post Claude's proposed description to the PR (`true`: confirmed), or drop it.
    DescApprove(bool),
    DescDiscard,
    /// Close the PR with Claude's draft comment (after your editor) / drop the draft.
    ClosePr,
    CloseDiscard,
    Nothing,
}

/// What the diff shows: all of it, or only the threads of Claude's review / your pending one.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum View {
    All,
    Claude,
    Pending,
    /// The PR's whole conversation on its own (`Space o`).
    Conversation,
}

struct Item {
    label: String,
    /// More lines under the label (dimmed), e.g. a comment's text; matched by the filter too.
    detail: Vec<String>,
    target: Target,
}

impl Item {
    fn new(label: impl Into<String>, target: Target) -> Item {
        Item { label: label.into(), detail: vec![], target }
    }
}

/// The commit a question (its `"conversation"`) is about: a commit's URL on GitHub
/// (`…/commits/<sha>`), or `commit:<sha>` without a PR.
pub(super) fn commit_of(url: &str) -> Option<&str> {
    url.rsplit_once("/commits/").map(|(_, s)| s).or_else(|| url.strip_prefix("commit:"))
}

impl App {
    /// A commit's short sha, as the commits list has it (else its first 7 characters).
    pub(super) fn commit_short(&self, sha: &str) -> String {
        self.commits.iter().find(|c| c.sha == sha).map_or_else(|| sha.chars().take(7).collect(), |c| c.short.clone())
    }
}

/// The threads picker's titles, one per filter (Tab goes round them): its title says which.
const THREAD_TABS: [&str; 5] = [
    " threads: [all] unresolved pending Claude new ",
    " threads: all [unresolved] pending Claude new ",
    " threads: all unresolved [pending] Claude new ",
    " threads: all unresolved pending [Claude] new ",
    " threads: all unresolved pending Claude [new] ",
];
const THREADS_ALL: usize = 0;
const THREADS_UNRESOLVED: usize = 1;
const THREADS_PENDING: usize = 2;
const THREADS_CLAUDE: usize = 3;
const THREADS_NEW: usize = 4;

struct Picker {
    title: &'static str,
    items: Vec<Item>,
    query: String,
    sel: usize,
}

/// What an in-flight LSP request was for.
enum Req {
    Init,
    Goto,
    Hover,
    /// A name from text (a comment, the PR's): where it's defined, by rust-analyzer's symbol
    /// search; then `then` (references, implementations) from there, if that's what was asked.
    Symbol { name: String, then: Option<&'static str> },
}

/// Claude's review for this PR/branch, and the background `claude -p` runs that write it.
#[derive(Default)]
struct ClaudeState {
    /// Its review file (see `claude.rs`), and when that last changed.
    path: Option<PathBuf>,
    mtime: Option<std::time::SystemTime>,
    summary: String,
    /// The commit the review was made against (its `head`).
    head: String,
    /// A new description for the PR that Claude proposes (empty: none).
    description: String,
    /// A comment Claude drafted for closing the PR (empty: none).
    closing: String,
    /// A background `claude -p` run: (process, log file, what it's doing).
    job: Option<agent::ClaudeJob>,
    /// The Claude Code session of the last background run, resumed by the next.
    session: Option<String>,
    /// A reply came in while Claude was busy: answer again when it's done.
    again: bool,
    /// Background runs so far (each gets its own log).
    runs: u32,
    /// The last background run's log (its result, as JSON).
    log: Option<PathBuf>,
    /// What background runs have cost so far (as Claude Code reports it, in USD).
    spent: f64,
    /// When Claude's pane was last looked at, and whether it was asking you something then.
    pane_check: Option<std::time::Instant>,
    asking: bool,
    /// Its pane shows it working (Claude Code's "esc to interrupt").
    pane_busy: bool,
}

/// The comments you've seen (`comment_key`), kept in `path`; and those of the thread the
/// cursor is on, seen once it leaves.
struct Seen {
    keys: HashSet<String>,
    /// What you'd seen when the PR was loaded: what's new against it stays shown (attached to
    /// the description) as you read it.
    before: HashSet<String>,
    path: Option<PathBuf>,
    current: Vec<String>,
    since: std::time::Instant,
}

pub struct App {
    base: String,
    /// The base dv started with (`base` is HEAD while showing only what isn't committed).
    full_base: String,
    /// The branch `full_base` comes from (`origin/<the PR's base>`), when dv chose it: after a
    /// rebase onto it, the diff starts from where the branch is now (`reload`).
    pub follow_base: Option<String>,
    /// The commit the diff goes up to while showing some of the PR's commits, else the
    /// working tree.
    to: Option<String>,
    /// The PR's commits, oldest first, under the files' list; what's shown of them (first,
    /// last; `None`: all, the whole diff), the cursor there (0: "all commits", then each),
    /// a range being selected (`v`), and whether the list's cursor is there.
    commits: Vec<files::Commit>,
    /// The PR's commits GitHub has no valid signature on (from its info).
    unsigned: Vec<String>,
    /// Workflow runs on its head waiting for a maintainer's approval (from its info).
    awaiting: usize,
    /// Its checks failing, as GitHub said last.
    failing: usize,
    commit_shown: Option<(usize, usize)>,
    commit_cur: usize,
    commit_anchor: Option<usize>,
    in_commits: bool,
    files: Vec<FileDiff>,
    /// Extra context lines per hunk: (path, hunk's first full index) → lines (can be negative).
    extra: HashMap<(String, usize), i64>,
    /// The same in the Claude / pending view, per comment: (path, its line's full index).
    view_extra: HashMap<(String, usize), i64>,
    /// Files shown whole (`Z`).
    whole: HashSet<String>,
    collapsed: HashSet<String>,
    rows: Vec<Row>,
    anchor: Pos,
    head: Pos,
    want_col: usize,
    select: bool,
    count: usize,
    pending: Option<char>,
    prompt: Option<String>,
    picker: Option<Picker>,
    /// A comment or reply for Claude, waiting on the picker for what you want of it.
    wanting: Option<Action>,
    /// The PR's base as GitHub last said (freshly fetched): when it isn't the PR's here, it was
    /// retargeted, and the diff follows (`follow_github_base`).
    github_base: Option<String>,
    /// When to look at GitHub again by itself (CI rerun: its checks change, not the PR's ETag).
    refresh_at: Option<std::time::Instant>,
    /// What isn't committed (the commits list's last row), as of the last reload.
    dirty: files::Dirty,
    /// The commits (shas) questions about them were last matched to (`rekey_commits`).
    rekeyed: Vec<String>,
    search: String,
    msg: String,
    top: usize,
    left: usize,
    /// The PR's title line is pinned above the code, and has room for the PR's counts (else
    /// they're on the status bar); set by `draw`.
    pin: bool,
    bar_counts: bool,
    /// The code block in a comment that the cursor is in (its first row), and how far it's
    /// scrolled sideways: only it, so the code beside it and the comment's text stay put.
    code_at: Option<usize>,
    code_left: usize,
    height: usize,
    width: u16,
    action: Option<Action>,
    /// Done on the next loop, so its "…" message is drawn first.
    job: Option<Job>,
    /// The viewer's pending review on the PR, if any.
    review: Option<String>,
    /// Your "Viewed" marks from GitHub: path → VIEWED / DISMISSED.
    viewed: HashMap<String, String>,
    /// Side-by-side view; `right` is which side the cursor prefers on lines shown on both.
    split: bool,
    right: bool,
    /// Comments in the right margin: shown or hidden by `Ctrl-w c`; until then (`None`), shown
    /// when the window is wide enough for it.
    margin_pref: Option<bool>,
    /// The layout has them there now (set by `draw` from the width), and the margin's width.
    margin_active: bool,
    margin_w: usize,
    /// Its width (`Ctrl-w ,` / `.`) and whether it floats over the code rather than beside it
    /// (`Ctrl-w f`), as set; until then (`None`), from the window (narrow ones: floating).
    margin_w_pref: Option<usize>,
    margin_float_pref: Option<bool>,
    /// The files' list (`Ctrl-w e`): shown or not as you set it (else by the window's width),
    /// its width and first line as drawn, and the file of each of its lines (for clicks).
    files_pref: Option<bool>,
    /// Its width, when you've set it (`Ctrl-w ,` / `.` in it).
    files_w_pref: Option<usize>,
    files_w: usize,
    /// How far the list under its heading (the contents, then the commits) is scrolled.
    files_top: usize,
    /// What each line of the list under its heading is (for clicks).
    list_hits: Vec<files::Hit>,
    files_cur: Option<files::Spot>,
    /// `Ctrl-h` from the code's left edge: keys go to the files' list (j / k a file).
    files_focus: bool,
    /// The list's own cursor while it has focus (`j` / `k` move it; `Enter` goes there).
    files_sel: Option<files::Spot>,
    /// Where those three are kept between runs (`pane.json`, any repo), once set by their keys.
    pane_file: Option<PathBuf>,
    /// It floats now (set by `draw`).
    margin_float: bool,
    /// Where the cursor was last in the code.
    code_head: Pos,
    /// In margin mode: each thread's code line (the one it's about), as laid out.
    margin_anchor: HashMap<usize, usize>,
    /// The marks at the code's right edge, by (row, the old side), from `layout`: a comment's
    /// lines get `╎`, its last line (where it's anchored) a dot. Ranked, the highest shown:
    /// see `MARKS`.
    marks: HashMap<(usize, bool), u8>,
    /// Where things were drawn last, for mouse clicks.
    geom: mouse::Geom,
    /// Screen rows, and the screen row of each row (`top` and scrolling count screen rows).
    drows: Vec<DRow>,
    d_of: Vec<usize>,
    /// Changed-word spans (char ranges) for paired removed/added rows.
    emph: HashMap<usize, Vec<(usize, usize)>>,
    /// Helix `cursorline` / relative `line-number`.
    settings: Settings,
    quit: bool,
    /// `Space g` → merge: the review ends, then dv merges the PR (`merge.rs`).
    pub merge: bool,
    pr: Option<Pr>,
    threads: Vec<Thread>,
    info: Vec<gh::InfoLine>,
    info_collapsed: bool,
    /// How the checkout stands against the PR (`refresh_sync`): empty when in step.
    sync: String,
    /// The description's and the commit messages' sections, folded (by `Row::main`).
    sections_folded: HashSet<usize>,
    /// The messages of the commits shown (`commit_shown`), each its section's text.
    commit_msgs: Vec<String>,
    hl: Highlighter,
    ui: Ui,
    /// Highlighted files by (path, new side?), filled lazily as rows are drawn.
    hl_cache: HashMap<(String, bool), Colors>,
    /// Highlighted markdown by wrapped text (filled while building rows, hence the RefCell).
    md_cache: std::cell::RefCell<HashMap<String, Vec<crate::md::MdLine>>>,
    /// Width comment/info text is wrapped to; follows the terminal.
    wrap_w: usize,
    /// Root comment ids of threads folded/unfolded against the default (resolved = folded).
    toggled: HashSet<u64>,
    /// Claude's review and its background runs.
    claude: ClaudeState,
    /// The PR is someone else's: Claude suggests changes there rather than making them.
    theirs: bool,
    /// The Claude view (the `Claude` tab): only Claude's review: the PR's one-line summary, Claude's
    /// summary, and its comments with a few lines of code around each. The pending view
    /// (the `Pending` tab): the same for your pending review.
    view: View,
    /// The PR's state and its color (DRAFT, OPEN, …), at the right end of its header line.
    pr_state: Option<gh::State>,
    /// The replacement checked out here, if it is one (`replace.rs`): shown in the PR section;
    /// and when its record last changed (Claude redrafting it: it's shown again).
    replacement: Option<crate::replace::Record>,
    replacement_mtime: Option<std::time::SystemTime>,
    /// Your pending review's text so far (on GitHub; `c` in the PR section adds to it), for `S`.
    summary_draft: String,
    /// Your GitHub login: your comments are never new to you.
    me: String,
    /// The comments you've seen.
    seen: Seen,
    /// The `hx` pane dv opened (inside zellij), reused for the next file.
    hx: Option<HxPane>,
    /// When dv last looked at the working tree, and what it saw (`watch_tree`).
    tree_checked: std::time::Instant,
    tree_seen: Option<(Option<std::time::SystemTime>, u64)>,
    /// Messages shown in the status bar, with when (`Space m` lists them in full).
    msgs: Vec<(String, String)>,
    root: PathBuf,
    lsp: Option<Lsp>,
    lsp_ready: bool,
    inflight: HashMap<u64, Req>,
    progress: HashMap<String, (String, String)>,
    /// Scrollable overlay: (title, text, scroll).
    popup: Option<(String, Vec<Line<'static>>, u16)>,
    /// Help overlay search: (query, typing it).
    help: Option<(String, bool)>,
    jumps: Vec<Pos>,
    alphabet: Vec<char>,
    /// `gw` jump labels: (word start, word end, label), plus the first label char typed.
    labels: Vec<(Pos, Pos, [char; 2])>,
    label_first: Option<char>,
    /// The background look at GitHub (`start_github_watch`): what it fetched, with `github_gen`
    /// as it was then (a load since is newer, so that one's dropped).
    github: Option<std::sync::mpsc::Receiver<(u64, review::Fetched)>>,
    github_gen: std::sync::Arc<std::sync::atomic::AtomicU64>,
    /// Tells the background watch to fetch now, whatever the PR's ETag says.
    github_poke: Option<std::sync::mpsc::Sender<()>>,
    /// Where the cursor was when the whole conversation was opened (`Space o`), to go back to.
    conversation_back: Option<CursorKey>,
    /// The last note said about the PR's base (`base_note`), to say the next in its place.
    base_said: String,
    /// What's shown came from the cache, and the fresh fetch isn't in yet.
    github_stale: bool,
}

impl App {
    pub fn new(base: String, pr: Option<Pr>) -> Result<Self> {
        crate::progress(Some("loading the diff"));
        let files = diff::load(&base, None, DEFAULT_CONTEXT)?;
        let mut app = Self::with(base, files);
        app.load_commits();
        let theme = Theme::load();
        (app.hl, app.ui) = (Highlighter::new(&theme), theme.ui());
        app.settings = crate::theme::settings();
        app.load_pane();
        app.claude.path = crate::claude::path(pr.as_ref().map(|p| p.number)).ok();
        app.load_seen();
        // The PR's Claude Code session, if an earlier run (any dv, any worktree) started one.
        // One that started with another version of the skill would keep following that: start afresh.
        let session = app.claude.path.as_ref().and_then(|p| crate::claude::load_session(p, &app.root));
        let other_skill = session.as_ref().is_some_and(|s| !s.1);
        app.claude.session = session.filter(|s| s.1).map(|s| s.0);
        if pr.is_some() {
            crate::progress(Some("loading comments and PR info"));
        }
        app.pr = pr;
        // What dv saw last time, if it kept it, while it fetches the PR afresh.
        let n = app.pr.as_ref().map(|p| p.number);
        let cached = n.is_some_and(|n| app.show_cached(n));
        match cached {
            true => app.load_claude(),
            false => app.load_threads(),
        }
        app.rekey_commits(); // (commits rewritten while dv was closed)
        if let Some(n) = n {
            app.start_github_watch(n, cached);
        }
        if other_skill && (app.msg.is_empty() || app.github_stale) {
            app.msg = "dv's Claude skill has changed since this PR's Claude session began: the next Claude run starts a new one".into();
        }
        app.refresh_sync();
        app.build();
        match Lsp::start(&app.root) {
            Ok((l, id)) => {
                app.lsp = Some(l);
                app.inflight.insert(id, Req::Init);
            }
            Err(e) => app.msg = format!("{e:#}"),
        }
        Ok(app)
    }

    /// Keep each new status-bar message, so a long one cut off there can be read in full.
    fn log_msg(&mut self) {
        let m = self.msg.trim();
        if m.is_empty() || m.ends_with('…') || self.msgs.last().is_some_and(|l| l.1 == m) {
            return; // "posting…" and such: the outcome that follows is the news
        }
        let at = crate::claude::now().get(11..19).unwrap_or("").to_owned();
        self.msgs.push((at, m.to_owned()));
        if self.msgs.len() > 100 {
            self.msgs.remove(0);
        }
    }

    /// `Space m`: the messages so far, newest first, in full.
    fn messages(&mut self) {
        self.log_msg();
        let mut md: String = self.msgs.iter().rev().map(|(at, m)| format!("- `{at}` {m}\n")).collect();
        if md.is_empty() {
            md = "_no messages yet_\n".into();
        }
        if let Some(log) = &self.claude.log {
            md += &format!("\nThe last background Claude run's log: `{}` (its errors: `.err`)\n", log.display());
            md += &format!("\nBackground Claude runs so far: {}, ${:.2} (runs in a pane aren't counted)\n", self.claude.runs, self.claude.spent);
        }
        self.popup = Some((" messages (UTC) · j/k scroll ".into(), self.hover_lines(&md), 0));
    }

    /// A message to show when dv opens (e.g. how the checkout went).
    pub fn notice(&mut self, msg: String) {
        self.msg = msg;
    }

    fn with(base: String, files: Vec<FileDiff>) -> Self {
        let mut app = App {
            full_base: base.clone(),
            follow_base: None,
            base,
            to: None,
            commits: vec![],
            unsigned: vec![],
            awaiting: 0,
            failing: 0,
            commit_shown: None,
            commit_cur: 0,
            commit_anchor: None,
            in_commits: false,
            files,
            extra: HashMap::new(),
            view_extra: HashMap::new(),
            whole: HashSet::new(),
            collapsed: HashSet::new(),
            rows: vec![],
            anchor: Pos::default(),
            head: Pos::default(),
            want_col: 0,
            select: false,
            count: 0,
            pending: None,
            prompt: None,
            picker: None,
            wanting: None,
            rekeyed: vec![],
            dirty: Default::default(),
            github_base: None,
            refresh_at: None,
            search: String::new(),
            msg: String::new(),
            top: 0,
            left: 0,
            pin: false,
            bar_counts: false,
            code_at: None,
            code_left: 0,
            height: 1,
            width: 80,
            action: None,
            job: None,
            review: None,
            viewed: HashMap::new(),
            split: false,
            right: true,
            margin_pref: None,
            margin_active: false,
            margin_w: MARGIN_MIN,
            margin_w_pref: None,
            margin_float_pref: None,
            pane_file: None,
            files_pref: None,
            files_w_pref: None,
            files_w: 0,
            files_top: 0,
            list_hits: vec![],
            files_cur: None,
            files_focus: false,
            files_sel: None,
            margin_float: false,
            code_head: Pos { row: 0, col: 0 },
            margin_anchor: HashMap::new(),
            marks: HashMap::new(),
            geom: Default::default(),
            drows: vec![],
            d_of: vec![],
            emph: HashMap::new(),
            settings: Settings::default(),
            quit: false,
            merge: false,
            pr: None,
            threads: vec![],
            info: vec![],
            info_collapsed: false,
            sync: String::new(),
            sections_folded: HashSet::new(),
            commit_msgs: vec![],
            wrap_w: 100,
            hl: Highlighter::new(&Theme::ansi()),
            ui: Theme::ansi().ui(),
            hl_cache: HashMap::new(),
            md_cache: Default::default(),
            toggled: HashSet::new(),
            theirs: false,
            view: View::All,
            pr_state: None,
            summary_draft: String::new(),
            replacement: None,
            replacement_mtime: None,
            me: String::new(),
            seen: Seen { keys: HashSet::new(), before: HashSet::new(), path: None, current: vec![], since: std::time::Instant::now() },
            claude: ClaudeState::default(),
            hx: None,
            tree_checked: std::time::Instant::now(),
            tree_seen: None,
            msgs: vec![],
            root: std::env::current_dir().unwrap_or_default(),
            lsp: None,
            lsp_ready: false,
            inflight: HashMap::new(),
            progress: HashMap::new(),
            popup: None,
            help: None,
            jumps: vec![],
            alphabet: jump_alphabet(),
            labels: vec![],
            label_first: None,
            github: None,
            github_gen: Default::default(),
            github_poke: None,
            base_said: String::new(),
            conversation_back: None,
            github_stale: false,
        };
        app.shape();
        app.build();
        app
    }

    pub fn run(&mut self, term: &mut DefaultTerminal) -> Result<()> {
        while !self.quit {
            self.log_msg();
            self.watch_seen();
            term.draw(|f| self.draw(f))?;
            if let Some(job) = self.job.take() {
                self.run_job(job);
                continue;
            }
            // ponytail: 100ms poll so LSP replies show up without a keypress; a waker channel if it matters.
            if event::poll(Duration::from_millis(100))? {
                match event::read()? {
                    Event::Key(k) if k.kind == KeyEventKind::Press => self.key(k),
                    Event::Mouse(m) => self.mouse(m),
                    _ => {}
                }
            }
            self.lsp_poll();
            self.watch_tree();
            self.watch_claude();
            self.watch_github();
            if self.refresh_at.is_some_and(|t| t <= std::time::Instant::now()) {
                self.refresh_at = None;
                if let Some(p) = &self.github_poke {
                    _ = p.send(());
                }
            }
            self.watch_replacement();
            self.poll_claude();
            self.watch_claude_pane();
            if let Some(a) = self.action.take() {
                // In zellij the editor gets a pane on the right and dv stays on screen beside it.
                let in_pane = in_zellij();
                if !in_pane {
                    restore_terminal();
                }
                // Anything to post is queued as a job (it runs after a "posting…" frame, then
                // refetches); here only the diff reloads, as hx may have changed files.
                let res = self.perform(a);
                self.hl_cache.retain(|k, _| !k.1); // working-tree files may have changed; the base can't
                if !in_pane {
                    *term = init_terminal();
                }
                if let Err(e) = self.reload(false) {
                    self.msg = e.to_string();
                }
                match res {
                    Ok(m) => self.msg = m,
                    Err(e) => self.msg = format!("{e:#}"),
                }
            }
        }
        self.seen.current.push(String::new()); // what the cursor is on now counts as seen too
        self.watch_seen();
        if let Some(pr) = &self.pr {
            _ = crate::claude::set_looked(pr.number); // for `dv list`'s [new]
        }
        Ok(())
    }

    fn perform(&mut self, a: Action) -> Result<String> {
        match a {
            Action::Shell => {
                let shell = std::env::var("SHELL").unwrap_or_else(|_| "sh".into());
                let root = self.root.display().to_string();
                if in_zellij() {
                    let name = format!("shell: {}", self.root.file_name().map_or(root.clone(), |n| n.to_string_lossy().into_owned()));
                    let st = std::process::Command::new("zellij")
                        .args(["run", "--floating", "--close-on-exit", "--name", &name, "--cwd", &root, "--", &shell])
                        .stdin(std::process::Stdio::null())
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .status()?;
                    anyhow::ensure!(st.success(), "zellij run failed");
                    return Ok(format!("a shell in {root}, floating (exit closes it)"));
                }
                std::process::Command::new(&shell).current_dir(&self.root).status().context("running your shell")?;
                Ok(String::new())
            }
            Action::Open(target) if in_zellij() => self.open_in_hx_pane(&target),
            Action::Open(target) => {
                interactive("hx", std::slice::from_ref(&target), &format!("hx {target}")).context("running hx")?;
                Ok(String::new())
            }
            Action::Comment { anchor, review, prefill, claude } => {
                let range = match anchor.start {
                    Some((s, _)) => format!("{s}-{}", anchor.line),
                    None => anchor.line.to_string(),
                };
                let kind = match (&claude, prefill.is_empty()) {
                    (Some(_), _) => "Claude's comment",
                    (None, true) => "comment",
                    (None, false) => "suggestion",
                };
                let what = format!("{kind} on {}:{range}", anchor.path);
                self.pr.as_ref().context("no PR")?; // before the editor, so nothing written is lost
                let Some(body) = compose_with(&what, false, &prefill, claude.is_some())? else {
                    return Ok(format!("{kind} cancelled"));
                };
                self.post_later(Post::Thread { anchor, review, body, claude })
            }
            Action::Reply { thread, comment, review, quote } => {
                self.pr.as_ref().context("no PR")?;
                let Some(body) = compose_with("reply", false, &quote, false)? else { return Ok("reply cancelled".into()) };
                self.post_later(Post::Reply { thread, comment, review, body })
            }
            Action::ClaudeReply(k, want, prefill) => {
                let p = self.claude.path.clone().context("no review file")?;
                let what = format!("reply to Claude{}", agent::want_hint(want));
                let Some(body) = compose_with(&what, false, &prefill, false)? else { return Ok("reply cancelled".into()) };
                keep_on_error(crate::claude::reply(&p, &k, &body, want), &body)?;
                Ok(self.answer_now())
            }
            Action::AskClaude { anchor, code, about, want } => {
                let p = self.claude.path.clone().context("no review file")?;
                let range = anchor.start.map_or(anchor.line.to_string(), |(s, _)| format!("{s}-{}", anchor.line));
                let what = match about {
                    Some(_) => format!("ask Claude about this review comment on {}:{range} (it answers right away; nothing goes to GitHub)", anchor.path),
                    None => format!("a comment for Claude on {}:{range}{}", anchor.path, agent::want_hint(want)),
                };
                let prefill = about.as_ref().map_or(String::new(), |(_, q)| q.clone());
                let Some(body) = compose_with(&what, false, &prefill, false)? else { return Ok("comment cancelled".into()) };
                keep_on_error(crate::claude::add_comment(&p, &anchor, code, &body, about.map(|a| a.0), want).map(drop), &body)?;
                self.load_claude();
                Ok(self.answer_now())
            }
            Action::AskConversation { url, prefill, want } => {
                let p = self.claude.path.clone().context("no review file")?;
                let what = match (url == ON_SUMMARY, commit_of(&url)) {
                    (true, _) => "a question for Claude about the PR, or a reply to its review (it answers in the Claude tab; nothing goes to GitHub)".to_owned(),
                    (_, Some(sha)) => format!("for Claude, about commit {}{}", self.commit_short(sha), agent::want_hint(want)),
                    _ => "ask Claude about this conversation comment (it answers under it; nothing goes to GitHub)".to_owned(),
                };
                let Some(body) = compose_with(&what, false, &prefill, false)? else { return Ok("question cancelled".into()) };
                keep_on_error(crate::claude::add_conversation(&p, &url, &body, want).map(drop), &body)?;
                self.load_claude();
                Ok(self.answer_now())
            }
            Action::ClaudeWith(ask) => {
                // Optional (empty goes ahead) unless they're the whole request: a change with no
                // open comments. Quitting the editor with an error (`:cq`) cancels.
                let alone = matches!(ask, agent::Ask::Fix | agent::Ask::Edit) && self.change_requests() == 0 || matches!(ask, agent::Ask::Describe);
                let what = match (ask, alone) {
                    (agent::Ask::Review, _) => "anything for Claude to look at or keep in mind in its review (optional)",
                    (agent::Ask::Describe, _) if !self.claude.description.is_empty() => "what else to change in Claude's proposed description",
                    (agent::Ask::Describe, _) => "what to change in the PR's description (Claude proposes a new one; you approve it)",
                    (agent::Ask::Close, _) if !self.claude.closing.is_empty() => "what to change in Claude's closing comment",
                    (agent::Ask::Close, _) => "why close it, and anything the comment should say (optional: Claude drafts it; you edit it before it goes)",
                    (_, true) => "what Claude should change (there are no open comments to act on)",
                    _ => "instructions for Claude, on top of the open comments (optional)",
                };
                // Review instructions start from the last ones (kept if left as they are).
                let review = matches!(ask, agent::Ask::Review);
                let last = if review { crate::claude::instructions("review") } else { String::new() };
                let Some(extra) = compose_with(what, !alone, last.trim(), review)? else { return Ok("Claude not asked".into()) };
                if review {
                    _ = crate::claude::save_instructions("review", &extra);
                }
                if self.run_claude(ask, &extra) || extra.trim().is_empty() {
                    return Ok(std::mem::take(&mut self.msg));
                }
                // It couldn't be asked after all: keep what you wrote.
                let why = std::mem::take(&mut self.msg);
                Ok(keep_on_error(Err(anyhow::anyhow!(why)), &extra).err().map_or(String::new(), |e| format!("{e:#}")))
            }
            Action::Replace(prs, about) => {
                if crate::replace::load(prs[0]).is_ok_and(|r| r.drafting_now()) {
                    return Ok(format!("Claude is already drafting the replacement of #{}", prs[0]));
                }
                let which = prs.iter().map(|n| format!("#{n}")).collect::<Vec<_>>().join(", ");
                let (what, prefill) = match about {
                    Some(q) => (format!("ask Claude to redraft the replacement of {which}: what to change, under the quote"), q),
                    None => (format!("instructions for Claude's replacement of {which} (optional)"), crate::claude::instructions("replace")),
                };
                let Some(extra) = compose_with(&what, true, prefill.trim_end(), true)? else { return Ok("not asked".into()) };
                crate::claude::save_instructions("replace", &extra)?; // `dv replace` reads them
                agent::spawn_replace(&prs)?;
                Ok(format!("Claude is drafting a replacement for {which} in the background: dv list's replacements tab shows it"))
            }
            Action::Publish(close) => {
                let mut r = self.replacement.clone().context("not a replacement")?;
                if crate::replace::remote().is_none() {
                    return Ok(self.ask_replace_remote(close));
                }
                let prefill = format!("{}\n\n{}", r.title.trim(), r.body.trim());
                let kind = "draft PR";
                let what = format!("{kind} for {}: its title (first line), then its description", crate::replace::branch(r.id));
                let Some(text) = compose_with(&what, false, &prefill, true)? else { return Ok("not published".into()) };
                let (title, body) = text.split_once('\n').map_or((text.as_str(), ""), |(t, b)| (t, b));
                let (title, body) = (title.trim(), body.trim());
                if title.is_empty() {
                    return Ok("not published: it needs a title (the first line)".into());
                }
                let res = crate::replace::publish(&mut r, &self.root, title, body);
                keep_on_error(res.as_ref().map(drop).map_err(|e| anyhow!("{e:#}")), &text)?;
                let originals = !r.replaces.is_empty(); // a new PR replaces nothing
                self.show_replacement(r);
                if close {
                    self.action = Some(Action::CloseOriginals); // its own editor, next
                }
                let then = if originals { ": S closes what it replaces, with the note" } else { "" };
                // Ready for review only once CI passes: `Space g` there, which checks.
                Ok(format!("published as {kind} #{}{then} (ready for review once CI passes: Space g)", res.unwrap_or(0)))
            }
            Action::ClosePr => self.close_pr(),
            Action::CiFixPr => {
                let n = self.pr.as_ref().map_or(0, |p| p.number);
                let c = self.ci_note(n).context("no fix in mind for CI any more")?;
                let prefill = c.prefill(n);
                let what = "a new PR fixing CI, for Claude to draft: what it should do; unpublished until you publish it";
                let Some(task) = compose_with(what, false, &prefill, true)?.filter(|t| !t.trim().is_empty()) else { return Ok("not asked".into()) };
                let id = crate::replace::create_new(&task, &self.root)?;
                crate::claude::ci_fix_drafted(n); // not offered again
                agent::spawn_replace(&[id])?;
                Ok(format!("Claude is drafting {} in the background: dv list's drafts tab shows it", crate::replace::label(id)))
            }
            Action::CloseOriginals => {
                let mut r = self.replacement.clone().context("not a replacement")?;
                let which: Vec<String> = r.replaces.iter().filter(|o| !r.closed.contains(&o.number)).map(|o| format!("#{}", o.number)).collect();
                let what = format!("note closing {} (each gets it as a comment, then is closed)", which.join(", "));
                let Some(note) = compose_with(&what, false, &crate::replace::note(&r), true)? else { return Ok("nothing closed".into()) };
                let res = crate::replace::close_originals(&mut r, &note);
                self.show_replacement(r);
                let done = keep_on_error(res.as_ref().map(drop).map_err(|e| anyhow!("{e:#}")), &note).map(|_| res.unwrap_or_default())?;
                let done: Vec<String> = done.iter().map(|n| format!("#{n}")).collect();
                Ok(format!("closed {}, with the note", done.join(", ")))
            }
            Action::Converse { quote, as_is } => {
                self.pr.as_ref().context("no PR")?;
                let Some(body) = compose_with("comment on the PR conversation", false, &quote, as_is)? else {
                    return Ok("comment cancelled".into());
                };
                self.post_later(Post::Converse(body))
            }
            Action::Edit { comment, body: old, author } => {
                // Unchanged or emptied counts as cancelled; deleting a comment is a separate thing.
                // Maintainers can edit others' comments too, so the hint names whose this is.
                let what = format!("editing @{author}'s comment");
                let Some(body) = compose_with(&what, false, &old, false)? else { return Ok("edit cancelled".into()) };
                self.post_later(Post::Edit { comment, body })
            }
            Action::Submit(event) => {
                // An empty summary is fine; quitting the editor with an error (`:cq`) cancels.
                // Approving starts from "ACK <the PR's head commit>", posted as is if kept.
                let what = match event {
                    review::CLOSE => "closing comment (empty: just close the PR)".to_owned(),
                    e => format!("review summary ({})", e.to_lowercase().replace('_', " ")),
                };
                let head = diff::git(&["rev-parse", "HEAD"]).unwrap_or_default();
                let mut prefill = ack(event, self.pr.as_ref(), head.trim());
                if !self.summary_draft.is_empty() {
                    prefill = format!("{}\n\n{prefill}", self.summary_draft.trim_end());
                }
                if event == "APPROVE" {
                    // (A checkout that isn't the PR's head: the ACK's own hint says so.)
                    let notes = self.submit_notes().into_iter().filter(|n| !n.starts_with("your checkout"));
                    let notes: String = notes.map(|n| format!("{HINT} still open: {n} -->\n")).collect();
                    prefill = notes + &prefill;
                }
                let Some(body) = compose_with(&what, true, &prefill, true)? else {
                    return Ok("review not submitted".into());
                };
                self.summary_draft.clear();
                self.post_later(Post::Submit(event, body))
            }
            Action::Commit(draft) => self.commit(&draft),
        }
    }

    /// `Enter` inside zellij: open the file in dv's hx pane, reusing it if it's still open
    /// (`:open` + `:goto` typed into it, after `Esc` for normal mode), else in a new pane on the
    /// right. dv keeps working either way.
    fn open_in_hx_pane(&mut self, target: &str) -> Result<String> {
        let zellij = |args: &[&str]| -> Result<String> {
            let out = std::process::Command::new("zellij").args(args).stdin(std::process::Stdio::null()).output()?;
            anyhow::ensure!(out.status.success(), "zellij {} failed", args.get(1).unwrap_or(&""));
            Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
        };
        if let Some(id) = self.hx.as_ref().map(|h| h.pane.clone()).filter(|id| pane_alive(id)) {
            // `target` is path:line:col.
            let mut parts = target.rsplitn(3, ':');
            let (_col, line, path) = (parts.next(), parts.next().unwrap_or("1"), parts.next().unwrap_or(target));
            zellij(&["action", "write-chars", "-p", &id, "\x1b"])?;
            zellij(&["action", "write-chars", "-p", &id, &format!(":open {path}\r:goto {line}\r")])?;
            zellij(&["action", "focus-pane-id", &id])?;
            return Ok("opened in hx (its pane on the right); dv reloads as you save".into());
        }
        let root = self.root.display().to_string();
        let id = zellij(&[zellij_run().as_slice(), &["--close-on-exit", "--name", "hx", "--cwd", &root, "--", "hx", target]].concat())?;
        self.hx = Some(HxPane { pane: id });
        Ok("hx is open on the right; dv reloads as you save, and Enter opens the next file there".into())
    }

    /// What the working tree looks like, cheaply: the latest change time of the diff's files,
    /// and `git status` of tracked files (so a file Claude starts changing counts too).
    fn tree_stamp(&self) -> (Option<std::time::SystemTime>, u64) {
        use std::hash::{Hash, Hasher};
        let newest = self.files.iter().filter_map(|f| std::fs::metadata(self.root.join(&f.path)).and_then(|m| m.modified()).ok()).max();
        let mut h = std::collections::hash_map::DefaultHasher::new();
        diff::git(&["status", "--porcelain"]).unwrap_or_default().hash(&mut h);
        (newest, h.finish())
    }

    /// About once a second: reload the diff when the working tree changed (Claude's edits, a
    /// save in hx, anything else), keeping the cursor where it is; and notice dv's hx pane
    /// closing.
    // ponytail: polls git status each second; a file watcher if that's ever slow on a big repo.
    fn watch_tree(&mut self) {
        if self.tree_checked.elapsed() < std::time::Duration::from_secs(1) {
            return;
        }
        self.tree_checked = std::time::Instant::now();
        if self.hx.as_ref().is_some_and(|h| !pane_alive(&h.pane)) {
            self.hx = None;
        }
        let now = self.tree_stamp();
        if self.tree_seen.replace(now).is_none_or(|before| before == now) {
            return;
        }
        self.reload_tree();
    }

    /// Reload the diff after the working tree changed under dv.
    pub(super) fn reload_tree(&mut self) {
        self.hl_cache.retain(|k, _| !k.1); // working-tree files may have changed; the base can't
        if let Err(e) = self.reload(false) {
            self.msg = format!("{e:#}");
        }
        self.rekey_commits(); // (commits rewritten here or elsewhere)
        self.follow_github_base(false); // (a retarget that waited on a rebase now through)
        self.tree_seen = Some(self.tree_stamp());
    }

    fn reload(&mut self, threads: bool) -> Result<()> {
        let keep = self.cursor_key();
        self.follow_rebase();
        self.load_commits();
        self.files = diff::load(&self.base, self.to.as_deref(), DEFAULT_CONTEXT)?;
        // After the diff: Claude's comments follow their code, which may have just been edited.
        match threads {
            true => self.load_threads(),
            false => self.load_claude(),
        }
        self.shape();
        self.rebuild(keep);
        self.refresh_sync();
        Ok(())
    }

    /// HEAD rebased onto a newer base (its branch has moved on, and the PR with it): the diff
    /// starts there, else it'd show the base's own commits between as the PR's. Only forward:
    /// a base ahead of the old one, on HEAD's history.
    pub(super) fn follow_rebase(&mut self) {
        let Some(branch) = &self.follow_base else { return };
        // Not mid-rebase: it may yet be aborted, and the base only ever moves forward.
        if rebase::rebasing(&self.root) {
            return;
        }
        let git = |args: &[&str]| diff::git_in(&self.root, args);
        let Ok(now) = git(&["merge-base", branch, "HEAD"]).map(|s| s.trim().to_owned()) else { return };
        if now == self.full_base || git(&["merge-base", "--is-ancestor", &self.full_base, &now]).is_err() {
            return;
        }
        if !self.uncommitted() {
            self.base = now.clone(); // the whole diff (a commit shown, or a range, goes)
        }
        self.full_base = now;
        (self.to, self.commit_shown, self.commit_anchor) = (None, None, None);
        self.hl_cache.retain(|k, _| !k.1);
    }

    /// https://github.com/owner/repo, for `#123` links.
    fn repo_url(&self) -> Option<&str> {
        self.pr.as_ref().and_then(|p| p.url.rsplit_once("/pull/")).map(|(r, _)| r)
    }

    fn file_path(&self) -> Option<&str> {
        Some(&self.files.get(self.rows[self.head.row].file)?.path)
    }

    fn len(&self, r: usize) -> usize {
        self.rows[r].line.text.chars().count()
    }

    fn chars(&self, r: usize) -> Vec<char> {
        self.rows[r].line.text.chars().collect()
    }

    fn open_target(&self) -> Option<String> {
        let row = &self.rows[self.head.row];
        let f = self.files.get(row.file)?;
        // Deleted lines have no new-side position: use the next new-side line in the file.
        let line = self.rows[self.head.row..]
            .iter()
            .take_while(|r| r.file == row.file)
            .find_map(|r| r.line.new)
            .unwrap_or(1);
        let col = if row.line.new.is_some() { self.head.col + 1 } else { 1 };
        Some(format!("{}:{line}:{col}", f.path))
    }

    /// Rows on screen, each once (a context line shows on both sides but counts once).
    fn visible_rows(&self) -> Vec<usize> {
        let mut out = vec![];
        for d in &self.drows[self.top.min(self.drows.len())..(self.top + self.height).min(self.drows.len())] {
            match *d {
                DRow::Full(r) => out.push(r),
                DRow::Pair(l, r) => out.extend(l.into_iter().chain(r.filter(|&r| Some(r) != l))),
                DRow::Margin(l, r, c) => out.extend(l.into_iter().chain(r.filter(|&r| Some(r) != l)).chain(c)),
            }
        }
        out
    }

    /// Which highlighted file a code row's colors come from: the base for removed lines, else the working tree.
    fn hl_key(&self, r: usize) -> Option<(String, bool)> {
        let row = &self.rows[r];
        let f = self.files.get(row.file).filter(|f| f.meta != "binary")?;
        match row.line.kind {
            Kind::Del => Some((f.old_path.clone(), false)),
            Kind::Add | Kind::Ctx => Some((f.path.clone(), true)),
            _ => None,
        }
    }
}

/// How dv's hint lines in the editor start; they're dropped from what's posted.
const HINT: &str = "<!-- dv:";

/// Drop dv's hint line and trim; untouched prefill alone counts as nothing written.
fn clean_body(text: &str, mark: &str, prefill: &str) -> String {
    let body: Vec<&str> = text.lines().filter(|l| !l.starts_with(mark)).collect();
    let body = body.join("\n").trim().to_owned();
    if !prefill.is_empty() && body == prefill.trim() { String::new() } else { body }
}

/// If posting failed, save what was written so it isn't lost, and say where.
fn keep_on_error(res: Result<()>, body: &str) -> Result<()> {
    res.map_err(|e| {
        let saved = std::env::temp_dir().join(format!("dv-unsent-{}.md", std::process::id()));
        match std::fs::write(&saved, body) {
            // The reason first: a narrow status bar cuts the end off.
            Ok(()) if !body.is_empty() => anyhow!("{e:#} (text saved to {})", saved.display()),
            _ => e,
        }
    })
}

/// The summary an approval starts from: `ACK <the PR's head commit>`, as Bitcoin-style projects
/// write it. If what you reviewed (`local`, your HEAD) isn't that commit (your branch has
/// diverged from the PR), a hint line above it says so; hint lines aren't posted. Other
/// reviews start empty.
fn ack(event: &str, pr: Option<&Pr>, local: &str) -> String {
    let Some(p) = pr.filter(|p| event == "APPROVE" && !p.head_oid.is_empty()) else { return String::new() };
    let short = |h: &str| h.chars().take(8).collect::<String>();
    match p.head_oid == local || local.is_empty() {
        true => format!("ACK {}", p.head_oid),
        false => format!(
            "{HINT} your checkout is at {}, not the PR's head {}: check this ACK names what you reviewed -->\nACK {}",
            short(local),
            short(&p.head_oid),
            p.head_oid
        ),
    }
}

fn posted_msg(review: bool) -> &'static str {
    if review { "added to pending review (S to submit)" } else { "posted" }
}

/// Word-wrap long lines at `w` chars, keeping their indent; short lines are kept as they are.
fn wrap(text: &str, w: usize) -> Vec<String> {
    let mut out = vec![];
    for line in text.lines() {
        if line.chars().count() <= w {
            out.push(line.to_owned());
            continue;
        }
        // Cut at spaces; the lines it's cut into keep its indent.
        let body = line.trim_start();
        let indent = &line[..line.len() - body.len()];
        let mut cur = indent.to_owned();
        for word in body.split(' ') {
            if cur.len() > indent.len() && cur.chars().count() + 1 + word.chars().count() > w {
                out.push(std::mem::replace(&mut cur, indent.to_owned()));
            } else if cur.len() > indent.len() {
                cur.push(' ');
            }
            cur.push_str(word);
        }
        out.push(cur);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

/// `text` as a markdown quote (`> ` lines), followed by a blank line to write under.
/// Add `x` to `s`, or remove it if it's there.
fn flip<T: std::hash::Hash + Eq>(s: &mut HashSet<T>, x: T) {
    if !s.remove(&x) {
        s.insert(x);
    }
}

fn quote(text: &str) -> String {
    if text.trim().is_empty() {
        return String::new(); // nothing to quote (an empty description, a review with no text)
    }
    let q: Vec<String> = text.lines().map(|l| format!("> {l}").trim_end().to_owned()).collect();
    format!("{}\n\n", q.join("\n"))
}

/// A GitHub-style badge: the given background (hex, no '#'), black or white text for contrast.
pub(crate) fn chip(hex: &str) -> Style {
    let n = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0x6e7681);
    let (r, g, b) = ((n >> 16) as u8, (n >> 8) as u8, n as u8);
    let light = 0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32 > 150.0;
    let fg = if light { Color::Black } else { Color::White };
    Style::new().bg(Color::Rgb(r, g, b)).fg(fg).add_modifier(ratatui::style::Modifier::BOLD)
}

/// `editor.jump-label-alphabet` from the Helix config, else Helix's default.
fn jump_alphabet() -> Vec<char> {
    let cfg = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|_| std::env::var("HOME").map(|h| Path::new(&h).join(".config")))
        .map(|d| std::fs::read_to_string(d.join("helix/config.toml")).unwrap_or_default())
        .unwrap_or_default();
    // ponytail: line scan instead of a TOML parser; fine for `key = "value"` on one line.
    cfg.lines()
        .filter_map(|l| l.trim().strip_prefix("jump-label-alphabet")?.trim().strip_prefix('=')?.trim().strip_prefix('"')?.strip_suffix('"').map(str::to_owned))
        .next()
        .filter(|a| !a.is_empty())
        .unwrap_or_else(|| "abcdefghijklmnopqrstuvwxyz".into())
        .chars()
        .collect()
}

/// Line `n` (0-based) of a file on disk, reading each file once per `cache`.
fn disk_line(cache: &mut HashMap<PathBuf, String>, p: &Path, n: u32) -> String {
    let text = cache.entry(p.to_owned()).or_insert_with(|| std::fs::read_to_string(p).unwrap_or_default());
    text.lines().nth(n as usize).unwrap_or("").to_owned()
}
