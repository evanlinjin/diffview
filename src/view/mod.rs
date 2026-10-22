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
mod ask;
mod amend;
pub(crate) use amend::fold_cli;
pub(crate) use hints::help_lines;
mod rebase;
mod replacement;
mod seen;
mod build;
mod text;
mod compose;
use compose::*;
use build::*;
mod files;
mod git;
mod github;
mod info;
mod links;
use info::*;
#[cfg(test)]
pub(crate) use info::info_lines_of;
pub(crate) use files::cut;
mod keys;
mod layout;
mod mouse;
mod motion;
mod ra;
mod render;
mod hints;
mod row;
mod review;
mod select;
mod term;
mod threads;
pub use term::{MOUSE, clipboard, init_terminal, restore_terminal};
pub(crate) use term::{compose_with, in_zellij};
pub(crate) use github::{prefetch_github, recent_github};
pub(crate) use render::{chip, draw_hints, popup_frame};
use {agent::ClaudeState, git::with_reload, keys::*, layout::*, motion::*, ra::*, seen::Seen, threads::*};
use term::*;
#[cfg(test)]
pub(crate) mod tests;

const DEFAULT_CONTEXT: u32 = 3;
/// Pending-key marker for Ctrl-w (window commands).
const CTRL_W: char = '\u{17}';
/// `Row::file` for the PR info section at the top.
const INFO: usize = usize::MAX - 1;
/// Comment marks at the code's right edge, by rank (the highest wins where they meet).
pub(super) const MARK_STALE_LINE: u8 = 1;
pub(super) const MARK_LINE: u8 = 2;
pub(super) const MARK_STALE_DOT: u8 = 3;
pub(super) const MARK_CLAUDE_DOT: u8 = 4;
pub(super) const MARK_DOT: u8 = 5;
/// A mark's rank is in its low 4 bits; above them (`<< BAR`), the strongest range passing too
/// (its comment's last line further on): `MARK_LINE` or `MARK_STALE_LINE`, or none.
pub(super) const BAR: u8 = 4;
/// A mark's own rank, its low bits.
pub(super) const RANK: u8 = (1 << BAR) - 1;

/// What a row scrolls sideways with (`App::scroller`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Scroller {
    Diff,
    /// A comment's code block or table, by its first row.
    Block(usize),
}

/// `Row::info` for Claude's review summary (it isn't one of `info`'s lines).
const SUMMARY: usize = usize::MAX;
/// `Row::info` for the text of your pending review, in the pending view.
const DRAFT: usize = usize::MAX - 2;
/// `Row::info` for the PR's facts (author, branches, size, reviews, checks), boxed together.
const FACTS: usize = usize::MAX - 3;
/// `Row::info` for the line saying how much more of the conversation there is (`Space o`).
const MORE_CONVERSATION: usize = usize::MAX - 4;
/// `Row::info` for Claude's latest doings, boxed together (the Claude tab's activity).
const CLAUDE_ACTIVITY: usize = usize::MAX - 5;
/// How many of Claude's latest doings its activity shows.
const CLAUDE_LATEST: usize = 5;
/// How many of the conversation's latest comments are attached to the description, at least.
const LATEST: usize = 3;
/// `Row::main` for the description's section (commit `n`'s message is `1 + n`).
const DESCRIPTION: usize = 0;
/// What a thread in Claude's review file that replies to its summary is on (`"conversation"`).
const ON_SUMMARY: &str = "summary";
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
    /// PR text attached to the description (its facts, conversation, your review; Claude's
    /// summary): beside it in the pane, or after it with the pane hidden.
    attached: bool,
    /// A line standing for a comment shown elsewhere (Claude's activity): (thread, comment),
    /// where Enter goes.
    goes: Option<(usize, usize)>,
    /// A reply under Claude's comment: drawn indented, behind a guide (`GUIDE`, not its text).
    guide: bool,
    /// The line standing for a resolved talk under Claude's comment (Tab shows it).
    talk: bool,
}

/// What sets a reply under Claude's comment in: drawn after the gutter, not part of its text.
pub(super) const GUIDE: &str = "  │ ";

impl Row {
    /// The columns its guide takes before its text.
    fn guide_w(&self) -> usize {
        if self.guide { GUIDE.chars().count() } else { 0 }
    }

    fn new(file: usize, line: DLine) -> Row {
        Row { file, line, thread: None, syn: vec![], links: vec![], info: None, edge: false, nowrap: false, main: None, attached: false, goes: None, guide: false, talk: false }
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
    /// A PR row outside its sections (facts, conversation, …: they move and re-wrap with the
    /// pane): what its `info` line is (`info_id`: not its place, which a refresh moves), how
    /// far into that line's rows, and which of all those rows.
    info: Option<(Option<String>, usize, usize)>,
    /// A section before the files (the description, a commit's message, Claude's summary): its
    /// `Row::main` and how far into it. (By index, the PR text before it growing would move it.)
    section: Option<(usize, usize)>,
    /// A line standing for a comment (`Row::goes`: Claude's activity): that comment's id (not
    /// its place in its thread, which a deleted one before it moves), how far into its lines.
    goes: Option<((u64, u64), usize)>,
    /// On a hunk's header (no line of its own): the header above `line`, its first.
    hunk: bool,
}

/// Where the cursor and a selection's anchor (and its column) are, taken before what a rebuild
/// follows changes the rows' threads.
struct Kept {
    head: CursorKey,
    anchor: Option<(CursorKey, usize)>,
}

/// Network work that doesn't need the terminal; run after a frame shows its "…" message.
enum Job {
    /// This repo's #N, from a link (its URL): an issue's in dv's issue view, a PR's in the browser.
    OpenIssue(u64, String),
    Refresh,
    /// Bring the checkout up to date: the PR's head, or `git pull --ff-only` without a PR.
    Pull,
    Resolve(String, bool),
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
    /// Rerun the head's checks: all of them (true), or the failed.
    Rerun(bool),
    /// Ask GitHub for the PR's labels, assignees and reviewers, and what it could have.
    Labels,
    /// Add (`true`) or remove a label, an assignee or a requested reviewer, by name.
    SetField(gh::Field, String, bool),
    /// Push HEAD to the PR's branch (`true`: force, with a lease on the PR's head).
    Push(bool),
    /// Send what was written in the editor.
    Post(Post),
}

impl Job {
    /// What it's doing, said on the frame before it's run.
    fn doing(&self) -> String {
        match self {
            Job::Refresh => "refreshing…".into(),
            Job::Pull => "updating the checkout…".into(),
            Job::Resolve(_, on) => if *on { "resolving…" } else { "unresolving…" }.into(),
            Job::React(..) => "reacting…".into(),
            Job::Discard(_) => "discarding…".into(),
            Job::DeleteComment(_) => "deleting…".into(),
            Job::Preview(url) => format!("loading {url}…"),
            Job::OpenIssue(n, _) => format!("opening #{n}…"),
            Job::Reactions(_) => "asking GitHub for its reactions…".into(),
            Job::Rebase => "rebasing…".into(),
            Job::Retarget(base) => format!("the PR's base is {base} now: fetching it…"),
            Job::Rerun(all) => if *all { "rerunning every check…" } else { "rerunning the failed checks…" }.into(),
            Job::Labels => "asking GitHub for labels, assignees, reviewers…".into(),
            Job::SetField(..) => "changing it on GitHub…".into(),
            Job::Push(force) => if *force { "force-pushing…" } else { "pushing…" }.into(),
            Job::Post(p) => match p {
                Post::Thread { review: true, .. } | Post::Reply { review: true, .. } => "adding to your pending review…",
                Post::Thread { .. } | Post::Reply { .. } | Post::Converse(_) => "posting…",
                Post::Summary(_) => "adding to your review summary…",
                Post::Edit { .. } => "updating the comment…",
                Post::Submit(..) => "submitting the review…",
            }
            .into(),
        }
    }
}

enum Target {
    /// Something done with the editor (`Action`), or after a frame saying so (`Job`).
    Do(Action),
    Run(Job),
    /// A line of the files' list (a file, a section): there.
    Spot(files::Spot),
    Loc(Loc),
    /// Asks for confirmation first.
    Discard,
    /// A comment: (thread index, comment index within it).
    Comment(usize, usize),
    Claude(agent::Ask),
    /// How "make the changes" lands them (`agent::LANDS`).
    Land(agent::Ask, &'static str),
    /// What you want of Claude on the comment or reply you're about to write ("ask", …).
    Want(&'static str),
    ClaudeNotes,
    /// Ask Claude a question about the PR (a thread under its summary, in the Claude tab).
    ClaudeQuestion,
    /// Keep replacement branches on this remote (`dv.replaceRemote`), then publish (as `Publish`).
    ReplaceRemote(String, bool),
    ClaudeFresh,
    /// The PR's replacement (its id), in its own view; back here after.
    OpenReplacement(u64),
    ClaudePane,
    ClaudeOpen,
    ClaudeCheck,
    ClaudeStop,
    GitView,
    /// A row of the commits list to show, as Enter on it there (`shown_rows`' terms).
    CommitRow(usize),
    /// The amend picker / amend what isn't committed into the PR's commits / ask Claude where
    /// what blame couldn't place goes.
    AmendMenu,
    Amend,
    AmendAsk,
    /// Claude places every uncommitted change, and dv amends them in once it has.
    AmendAll,
    /// Claude carrying a stopped rebase on / abort it.
    RebaseClaude,
    RebaseAbort,
    /// Take the branch as it is on GitHub (`true`: confirmed, or nothing here would be lost).
    GitTakeTheirs(bool),
    /// Drop what isn't committed (of that file; else all of it), stashed: after a yes.
    GitDrop(Option<String>, bool),
    /// End the review and merge the PR with github-merge.py.
    GitMerge,
    /// Dismiss all of Claude's outdated comments (`true`: confirmed).
    ClaudeDismissOutdated(bool),
    /// Mark the PR (a draft) ready for review.
    GitReady,
    /// Approve the workflow runs waiting on the PR's head.
    GitApproveRuns,
    /// Post Claude's proposed description to the PR (`true`: confirmed), or drop it.
    DescApprove(bool),
    DescDiscard,
    CloseDiscard,
    /// No, don't (a question's answer); or nothing to do.
    Nothing,
    /// Not to pick: what's so, under that section ("": the one before it's).
    Note(&'static str),
    /// Not to pick: what to see to before going on (`Tone::Warn`).
    Warn,
}

/// What a screen row shows of a line (`App::span`): its row; the chars of it there (a wrapped
/// line's part; else as far as it's scrolled and fits, short of a floating comment's box); and
/// the column the screen's text starts at (what a click counts from). Where the cursor may
/// land on it: `App::landing`.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Shown {
    pub row: usize,
    pub chars: std::ops::Range<usize>,
    pub left: usize,
}

/// The text widths rows are drawn at, as last drawn (0: nothing drawn yet): a full-width row's
/// (beside the pane as a column, as wide as the code; code's one less, for its marks' column),
/// a split half's (both alike), the pane's; and floating, the screen column the pane's boxes
/// start at (over the code's right end).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Widths {
    pub full: usize,
    pub half: usize,
    pub pane: usize,
    pub float_at: Option<usize>,
}

/// The issue a view is of (`App::open_issue`): no PR, no code; its text, its conversation, and
/// Claude's triage of it and talk about it.
#[derive(Clone, Debug, Default)]
pub(super) struct IssueRec {
    pub number: u64,
    pub title: String,
    pub url: String,
    /// When it last changed, and the PRs linked as fixing it: what a triage is as of.
    pub updated: String,
    pub fixes: Vec<(u64, String, String)>,
}

/// What a view hands over to show instead, and comes back to after.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HandOff {
    /// An issue of the PR's repo, in dv's issue view.
    Issue(u64),
    /// The PR's replacement (its id), in its own view.
    Replacement(u64),
    /// A PR a link goes to (`App::hands_prs`), reviewed as dv list does.
    Pr(u64),
}

/// What the diff shows: all of it, or only the threads of Claude's review / your pending one.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub(super) enum View {
    #[default]
    All,
    Claude,
    Pending,
}

struct Item {
    label: String,
    /// More lines under the label (dimmed), e.g. a comment's text; matched by the filter too.
    detail: Vec<String>,
    target: Target,
    /// The section it's listed under (its target's, else the one before it's: `Picker::new`).
    section: &'static str,
}

impl Item {
    fn new(label: impl Into<String>, target: Target) -> Item {
        Item { label: label.into(), detail: vec![], target, section: "" }
    }
}

/// An item's icon's color: what it does, at a glance.
#[derive(Clone, Copy)]
enum Tone {
    Plain,
    /// Claude does it.
    Claude,
    Good,
    /// It undoes, drops or closes something.
    Danger,
    Warn,
}

/// The pickers' sections, in the order they're listed.
const SECTIONS: [&str; 15] = [
    "Review", "Changes", "Description", "Closing", "Claude", "Replacement", "Working tree", "Where each change goes", "Branch", "CI",
    "Pull request", "Reviewers", "Labels", "Assignees", "Still open",
];

impl Target {
    /// It changes the checkout (its tree, its branch): not while Claude's at work there
    /// (`claude_busy`), one rule where it's picked.
    fn touches_checkout(&self) -> bool {
        matches!(self, Target::GitDrop(..) | Target::GitMerge | Target::GitTakeTheirs(_) | Target::RebaseAbort | Target::Run(Job::Pull | Job::Rebase) | Target::AmendAsk | Target::AmendAll)
    }

    /// How its item looks: its icon (a Nerd Font's; ' ': none), its color, its section ("": the
    /// one before it's).
    fn look(&self) -> (char, Tone, &'static str) {
        use {agent::Ask, Tone::*};
        match self {
            Target::Claude(Ask::Review | Ask::Triage) => ('\u{f441}', Claude, "Review"),
            Target::Claude(Ask::Answer) => ('\u{f4a8}', Claude, "Review"),
            Target::ClaudeQuestion => ('\u{f420}', Claude, "Review"),
            Target::ClaudeNotes => ('\u{f4f6}', Plain, "Claude"),
            Target::ClaudeDismissOutdated(_) => ('\u{f48e}', Danger, "Review"),
            Target::Claude(Ask::Fix | Ask::Edit) => ('\u{f448}', Claude, "Changes"),
            Target::Claude(Ask::FixChecks) => ('\u{f425}', Claude, "Changes"),
            Target::Do(Action::CiFixPr | Action::NewPr { .. }) => ('\u{f407}', Claude, "Changes"),
            Target::Claude(Ask::Describe) => ('\u{f4ed}', Claude, "Description"),
            Target::DescApprove(_) => ('\u{f42e}', Good, "Description"),
            Target::DescDiscard => ('\u{f48e}', Danger, "Description"),
            Target::Claude(Ask::Close) => ('\u{f4dc}', Claude, "Closing"),
            Target::Do(Action::ClosePr) => ('\u{f4dc}', Danger, "Closing"),
            Target::CloseDiscard => ('\u{f48e}', Danger, "Closing"),
            Target::ClaudeStop => ('\u{f46e}', Danger, "Claude"),
            Target::ClaudePane | Target::ClaudeOpen => ('\u{f489}', Plain, "Claude"),
            Target::ClaudeCheck => ('\u{f46a}', Plain, "Claude"),
            Target::ClaudeFresh => ('\u{f44d}', Plain, "Claude"),
            Target::OpenReplacement(_) => ('\u{f407}', Claude, "Claude"),
            Target::Do(Action::Replace(..)) => ('\u{f402}', Claude, "Replacement"),
            Target::GitView => ('\u{f440}', Plain, "Working tree"),
            Target::Do(Action::Commit) | Target::AmendMenu | Target::Amend => ('\u{f417}', Plain, "Working tree"),
            Target::AmendAsk | Target::AmendAll => ('\u{f417}', Claude, "Working tree"),
            Target::GitDrop(..) => ('\u{f48e}', Danger, "Working tree"),
            Target::Run(Job::Push(_)) => ('\u{f403}', Plain, "Branch"),
            Target::GitTakeTheirs(_) | Target::Run(Job::Pull) => ('\u{f409}', Plain, "Branch"),
            Target::Run(Job::Rebase) => ('\u{f418}', Claude, "Branch"),
            Target::Run(Job::Rerun(_)) => ('\u{f46a}', Plain, "CI"),
            Target::GitApproveRuns => ('\u{f52e}', Good, "CI"),
            Target::GitReady => ('\u{f407}', Good, "Pull request"),
            Target::GitMerge => ('\u{f419}', Good, "Pull request"),
            Target::Run(Job::SetField(gh::Field::Reviewer, ..)) => ('\u{f415}', Plain, "Reviewers"),
            Target::Run(Job::SetField(gh::Field::Label, ..)) => ('\u{f412}', Plain, "Labels"),
            Target::Run(Job::SetField(gh::Field::Assignee, ..)) => ('\u{f415}', Plain, "Assignees"),
            Target::RebaseClaude => ('\u{f51b}', Claude, ""),
            Target::RebaseAbort => ('\u{f467}', Danger, ""),
            Target::Do(Action::Submit("APPROVE")) => ('\u{f42e}', Good, ""),
            Target::Do(Action::Submit("REQUEST_CHANGES")) => ('\u{f421}', Plain, ""),
            Target::Do(Action::Submit(review::CLOSE) | Action::CloseOriginals) => ('\u{f4dc}', Danger, ""),
            Target::Do(Action::Submit(_)) => ('\u{f41f}', Plain, ""),
            Target::Do(Action::Publish(_)) => ('\u{f403}', Good, ""),
            Target::Discard | Target::Run(Job::Discard(_) | Job::DeleteComment(_)) => ('\u{f48e}', Danger, ""),
            Target::Want("ask") => ('\u{f420}', Claude, ""),
            Target::Want("suggest") => ('\u{f41f}', Claude, ""),
            Target::Want("change") => ('\u{f448}', Claude, ""),
            Target::Want(_) | Target::Land(..) => ('\u{f417}', Claude, ""),
            Target::Spot(_) => ('\u{f4a5}', Plain, ""),
            Target::CommitRow(_) => ('\u{f417}', Plain, ""),
            Target::Loc(_) => ('\u{f450}', Plain, ""),
            Target::Comment(..) => ('\u{f41f}', Plain, ""),
            Target::ReplaceRemote(..) => ('\u{f402}', Plain, ""),
            Target::Nothing => ('\u{f467}', Plain, ""),
            Target::Note(s) => (' ', Plain, s),
            Target::Warn => ('\u{f421}', Warn, "Still open"),
            _ => (' ', Plain, ""),
        }
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

struct Picker {
    title: &'static str,
    items: Vec<Item>,
    query: String,
    sel: usize,
    /// Its lines as last drawn, below the query: which match each is (`None`: a section's
    /// heading, a gap), for a click.
    lines: Vec<Option<usize>>,
}

impl Picker {
    /// `items` under `title`, by section (as `SECTIONS` has them, each as given; one without a
    /// section stays with the one before it), nothing typed, the first chosen.
    fn new(title: &'static str, mut items: Vec<Item>) -> Self {
        let mut last = "";
        for it in &mut items {
            last = match it.target.look().2 {
                "" => last,
                s => s,
            };
            it.section = last;
        }
        items.sort_by_key(|it| SECTIONS.iter().position(|&s| s == it.section));
        Picker { title, items, query: String::new(), sel: 0, lines: vec![] }
    }

    /// The item `want` picks chosen (else the first).
    fn select(self, want: impl Fn(&Target) -> bool) -> Self {
        let sel = self.items.iter().position(|i| want(&i.target)).unwrap_or(0);
        Picker { sel, ..self }
    }
}

#[derive(Default)]
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
    /// The first commit shown, by SHA (the last's is `to`): what the list's reloads follow.
    shown_first: Option<String>,
    /// The commits the diff's ends are (`base`, `to`), as last noted (`note_diff`).
    diff_at: (String, Option<String>),
    commit_cur: usize,
    commit_anchor: Option<usize>,
    in_commits: bool,
    files: Vec<FileDiff>,
    /// Extra context lines (can be negative), per hunk of the whole diff, or per comment's
    /// window in the Claude / pending views (`Wide`).
    extra: HashMap<Wide, i64>,
    /// Files shown whole (`Z`).
    whole: HashSet<String>,
    collapsed: HashSet<String>,
    rows: Vec<Row>,
    anchor: Pos,
    head: Pos,
    /// The column a vertical move keeps (into a wrapped line's part, or the line), while moves
    /// are vertical (`j`/`k`, the wheel); any other move lets it go.
    want_x: Option<usize>,
    /// The selection `x` made (anchor, head), while it stands: another `x` takes the next line
    /// too. (Its shape can't say: on an empty or one-char line, it's a cursor's.)
    lines: Option<(Pos, Pos)>,
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
    /// A sideways scroll of the mouse's, to make (columns): the next frame scrolls what the
    /// cursor's in by it, the cursor along.
    hnudge: isize,
    height: usize,
    width: u16,
    action: Option<Action>,
    /// Done on the next loop, so its "…" message is drawn first.
    job: Option<Job>,
    /// The viewer's pending review on the PR, if any.
    /// How lines at the PR's head moved in what's shown here (local edits, unpushed commits,
    /// a commit before it): per file, its hunks (old start, old count, new start, new count).
    /// GitHub's line numbers are the head's; `here` maps them.
    head_moves: diff::Moves,
    review: Option<String>,
    /// The commit the pending review is on (`review`'s).
    review_on: String,
    /// What to show instead for a while (an issue a link goes to, this PR's replacement): the
    /// view hands it over, and goes on as it was after.
    pub hand_off: Option<HandOff>,
    /// A PR a link goes to is handed off to be reviewed (an issue opened from dv's issues),
    /// rather than opened in the browser.
    pub hands_prs: bool,
    /// This PR's replacement (one replacing it, among others or not), as last read, and when
    /// that was looked for (every few seconds, as the records change).
    replaced_by: Option<crate::replace::Record>,
    /// The repo's page on GitHub, by its remote (`repo_url` when there's no PR).
    github_url: Option<String>,
    replaced_at: Option<(std::time::Instant, Option<std::time::SystemTime>)>,
    /// Your "Viewed" marks from GitHub: path → VIEWED / DISMISSED.
    viewed: HashMap<String, String>,
    /// Viewed marks on their way to GitHub (shown already): path → how it's going.
    marking: HashMap<String, review::Marking>,
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
    /// The pane you're in maximized (`Ctrl-w o`): the code (`false`; no files' list, no
    /// comments pane) or the comments pane (`true`; over all of the code), till you leave it.
    zoom: Option<bool>,
    /// The window as last drawn (to lay it out again unzoomed, `side`).
    full: Rect,
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
    margin_anchor: std::collections::BTreeMap<usize, usize>,
    /// The marks at the code's right edge, by (row, the old side), from `layout`: a comment's
    /// lines get `╎`, its last line (where it's anchored) a dot. Ranked, the highest shown:
    /// see `MARKS`.
    marks: HashMap<(usize, bool), u8>,
    /// Where things were drawn last, for mouse clicks.
    geom: mouse::Geom,
    /// Screen rows, and the screen row of each row (`top` and scrolling count screen rows).
    drows: Vec<DRow>,
    d_of: Vec<usize>,
    /// Each screen row's part of its line, wrapped (`soft_wrap`): 0, or the n-th continuation.
    dseg: Vec<usize>,
    /// Long code lines wrap (`Ctrl-w r`) instead of scrolling sideways; the code's text widths
    /// they wrap to (one column: the whole width or beside the pane; a split half: both halves
    /// alike); and, for this layout, where each row's parts after its first start (`wrap_rows`).
    soft_wrap: bool,
    wrap: Vec<Vec<usize>>,
    /// The text widths rows are drawn at, as last drawn: what wrapping, and what's on screen
    /// (`span`), go by.
    frame_w: Widths,
    /// Changed-word spans (char ranges) for paired removed/added rows.
    emph: HashMap<usize, Vec<(usize, usize)>>,
    /// Helix `cursorline` / relative `line-number`.
    settings: Settings,
    quit: bool,
    /// The window (or zellij pane) isn't focused: no cursor drawn.
    unfocused: bool,
    /// `Space g` → merge: the review ends, then dv merges this PR (`merge.rs`), the one the
    /// view had then (a replacement published in it too).
    pub merge: Option<u64>,
    pr: Option<Pr>,
    threads: Vec<Thread>,
    info: Vec<InfoLine>,
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
    /// Each file's syntax colors, by (its revision: the base, a commit, or empty for the
    /// working tree; its path): another base or commit is another key, not stale colors.
    hl_cache: HashMap<(String, String), Colors>,
    /// Highlighted markdown by wrapped text (filled while building rows, hence the RefCell).
    md_cache: std::cell::RefCell<HashMap<String, Vec<crate::md::MdLine>>>,
    /// Width comment/info text is wrapped to; follows the terminal.
    wrap_w: usize,
    /// Root comment ids of threads folded/unfolded against the default (resolved = folded).
    toggled: HashMap<u64, bool>,
    /// Claude's comments (their roots' ids) whose resolved talk is shown anyway (Tab on it):
    /// to read, not reopened.
    talk_shown: HashSet<u64>,
    /// Claude's review and its background runs.
    claude: ClaudeState,
    /// The PR is someone else's: Claude suggests changes there rather than making them.
    theirs: bool,
    /// The Claude view (the `Claude` tab): only Claude's review: the PR's one-line summary, Claude's
    /// summary, and its comments with a few lines of code around each. The pending view
    /// (the `Pending` tab): the same for your pending review.
    view: View,
    /// The PR's state and its color (DRAFT, OPEN, …), at the right end of its header line.
    pr_state: Option<State>,
    /// It conflicts with its base (GitHub's word): said on its state's chip.
    conflicts: bool,
    /// What `dv list`'s ranking said to do next about it, and when it ranked (the Claude tab
    /// says it).
    ranked_next: Option<(String, i64)>,
    /// The replacement checked out here, if it is one (`replace.rs`): shown in the PR section;
    /// and when its record last changed (Claude redrafting it: it's shown again).
    replacement: Option<crate::replace::Record>,
    replacement_mtime: Option<std::time::SystemTime>,
    /// The issue this view is of, if it is one (no PR: `pr` is none, and no code).
    issue: Option<IssueRec>,
    /// Your pending review's text so far (on GitHub; `c` in the PR section adds to it), for `S`.
    summary_draft: String,
    /// Your GitHub login: your comments are never new to you.
    me: String,
    /// The comments you've seen.
    seen: Seen,
    /// The `hx` pane dv opened (inside zellij), reused for the next file.
    hx: Option<HxPane>,
    /// What's being written in the editor's pane (inside zellij), dv going on meanwhile, and
    /// what asked for it: done again once it's written, given the text (`term::Writing`).
    writing: Option<(Action, term::Writing)>,
    /// Quitting while writing beside dv was warned of, by the last key: the next quit goes.
    quit_armed: Option<std::time::Instant>,
    /// When dv last looked at the working tree, and what it saw (`watch_tree`).
    tree_checked: Option<std::time::Instant>,
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
    jumps: Vec<(CursorKey, usize)>,
    alphabet: Vec<char>,
    /// `gw` jump labels: (word start, word end, label), plus the first label char typed.
    labels: Vec<(Pos, Pos, [char; 2])>,
    label_first: Option<char>,
    /// The background look at GitHub (`start_github_watch`): what it fetched, with `github_gen`
    /// as it was then (a load since is newer, so that one's dropped).
    github: Option<std::sync::mpsc::Receiver<(u64, github::Fetched)>>,
    github_gen: std::sync::Arc<std::sync::atomic::AtomicU64>,
    /// Tells the background watch to fetch now, whatever the PR's ETag says.
    github_poke: Option<std::sync::mpsc::Sender<()>>,
    /// The PR's whole conversation attached to the description, not only its latest (`Space o`).
    conversation_all: bool,
    /// The first row of the threads on files gone from the diff (rows' length if none).
    gone_at: usize,
    /// Comments hidden on GitHub that you've shown here (Tab), by node id.
    revealed: HashSet<String>,
    /// The PR's labels, assignees and reviewers, and what it could have (`Space l`), as GitHub
    /// said and as you've changed them since; and where you were in their picker, to come back.
    fields: Option<(gh::Choices, gh::Choices, gh::Choices)>,
    fields_at: Option<(String, gh::Field, String)>,
    /// Claude is placing every change for an amend (`AmendAll`): dv amends once it has.
    amend_then: bool,
    /// Shown, then hidden again with Tab on their thread's first line: the next Tab there folds
    /// the thread (Tab goes round: shown, hidden, folded).
    hid_again: HashSet<String>,
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
        // A replacement published as this PR: its draft's review is the PR's now.
        if let Some(p) = &pr {
            crate::replace::adopt_review(p.number);
            crate::progress(Some("loading comments and PR info"));
        }
        app.claude.path = crate::claude::path(pr.as_ref().map(|p| p.number)).ok();
        app.pr = pr;
        app.load_replacement();
        app.open_github();
        app.rekey_commits(); // (commits rewritten while dv was closed)
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

    /// Issue `n` of this repo, in the PR view: its text and conversation from GitHub, Claude's
    /// triage of it and talk about it; no code (no diff, commits, or language server).
    pub fn open_issue(n: u64) -> Result<Self> {
        crate::progress(Some("loading the issue"));
        let mut app = Self::with(String::new(), vec![]);
        app.issue = Some(IssueRec { number: n, ..IssueRec::default() });
        app.claude.path = crate::claude::Subject::Issue(n).path().ok();
        if let Some(p) = &app.claude.path {
            _ = crate::claude::adopt_issue_extras(p);
        }
        app.open_github();
        app.files_pref = Some(false); // (no files: its contents would be the description alone)
        app.build();
        Ok(app)
    }

    /// What a view of a PR or an issue starts with alike: the theme, the pane, what was seen,
    /// Claude's session, and what GitHub has of it (what dv kept, at once; then afresh, and
    /// watched).
    fn open_github(&mut self) {
        let theme = Theme::load();
        (self.hl, self.ui) = (Highlighter::new(&theme), theme.ui());
        self.settings = crate::theme::settings();
        self.github_url = diff::github_url();
        self.load_pane();
        self.load_seen();
        // Its Claude Code session, if an earlier run (any dv, any worktree) started one. One
        // that started with another version of the skill would keep following that: start afresh.
        let session = self.claude.path.as_ref().and_then(|p| crate::claude::load_session(p, &self.root));
        let other_skill = session.as_ref().is_some_and(|s| !s.1);
        self.claude.session = session.filter(|s| s.1).map(|s| s.0);
        let cached = self.show_cached();
        match cached {
            true => self.load_claude(),
            false => self.load_threads(),
        }
        self.start_github_watch(cached);
        if other_skill && (self.msg.is_empty() || self.github_stale) {
            self.msg = format!("dv's Claude skill has changed since {}'s Claude session began: the next Claude run starts a new one", self.what());
        }
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
            base,
            height: 1,
            width: 80,
            right: true,
            margin_w: MARGIN_MIN,
            wrap_w: 100,
            root: std::env::current_dir().unwrap_or_default(),
            alphabet: jump_alphabet(),
            ..Default::default()
        };
        app.set_files(files);
        app.shape();
        app.build();
        app
    }

    /// Merge PR `n`, asked in a view this one handed over to: this one ends too, as `Space g`
    /// here would end it (`hold_quit` first), for the caller to merge.
    pub fn merge_asked(&mut self, n: u64) {
        (self.merge, self.quit) = (Some(n), true);
    }

    /// Leaving dv while you write beside it (what's written would be lost), whatever asked to
    /// (`q`, Ctrl-c, merging): said first, and left only when asked again a moment later (not
    /// a held key's repeat, nor long after, the warning gone).
    pub(super) fn hold_quit(&mut self) {
        if !self.quit || self.writing.is_none() {
            return;
        }
        let again = self.quit_armed.take().is_some_and(|t| (QUIT_AFTER..QUIT_WITHIN).contains(&t.elapsed()));
        if !again {
            (self.quit, self.merge, self.quit_armed) = (false, None, Some(std::time::Instant::now()));
            self.msg = "you're writing in the editor's pane: save or :cq it first (again: leaves anyway)".into();
        }
    }

    /// A key pressed: what it does, and (another key than leaving) leaving asks again.
    pub(super) fn press(&mut self, k: KeyEvent) {
        self.key(k);
        if !self.quit {
            self.quit_armed = None;
        }
    }

    /// Something half done whose places are rows (a prefix, jump labels, a picker's items, a
    /// prompt): what changed elsewhere waits till it's done.
    pub(super) fn mid_sequence(&self) -> bool {
        self.pending.is_some() || !self.labels.is_empty() || self.picker.is_some() || self.prompt.is_some()
    }

    /// What changed elsewhere (the tree, Claude, GitHub, …) shown; not mid-sequence (the rows
    /// stay as it began on till it's done).
    pub(super) fn watch(&mut self) {
        if self.mid_sequence() {
            return;
        }
        self.lsp_poll();
        self.watch_seen();
        self.watch_tree();
        self.watch_claude();
        self.watch_github();
        self.watch_marks();
        if self.refresh_at.is_some_and(|t| t <= std::time::Instant::now()) {
            self.refresh_at = None;
            if let Some(p) = &self.github_poke {
                _ = p.send(());
            }
        }
        self.watch_replacement();
        self.watch_replaced_by();
        self.poll_claude();
        self.watch_claude_pane();
        self.watch_elsewhere();
    }

    pub fn run(&mut self, term: &mut DefaultTerminal) -> Result<()> {
        // (Till you quit, or an issue's to be shown: then back here, as it was.) Leaving goes
        // through `hold_quit`, whatever asked: a key here, a merge asked in a view this one
        // handed over to (`run_view`).
        loop {
            self.hold_quit();
            if self.quit || self.hand_off.is_some() {
                break;
            }
            self.log_msg();
            term.draw(|f| self.draw(f))?;
            if let Some(job) = self.job.take() {
                self.run_job(job);
                continue;
            }
            // ponytail: 100ms poll so LSP replies show up without a keypress; a waker channel if it matters.
            if event::poll(Duration::from_millis(100))? {
                match event::read()? {
                    Event::Key(k) if k.kind == KeyEventKind::Press => self.press(k),
                    Event::Mouse(m) => self.mouse(m),
                    Event::FocusLost => self.unfocused = true,
                    Event::FocusGained => self.unfocused = false,
                    _ => {}
                }
            }
            self.hold_quit();
            self.watch();
            self.act(term);
        }
        self.seen.current.push(String::new()); // what the cursor is on now counts as seen too
        self.watch_seen();
        if let Some(pr) = &self.pr {
            _ = crate::claude::set_looked(pr.number); // for `dv list`'s [new]
        }
        Ok(())
    }

    /// https://github.com/owner/repo, for `#123` links: the PR's, else the remote's (a
    /// replacement or new PR not published, a branch).
    fn repo_url<'a>(&'a self) -> Option<&'a str> {
        let of = |url: &'a str, at| url.rsplit_once(at).map(|(r, _)| r);
        self.pr.as_ref().and_then(|p| of(&p.url, "/pull/")).or(self.issue.as_ref().and_then(|i| of(&i.url, "/issues/"))).or(self.github_url.as_deref())
    }

    fn file_path(&self) -> Option<&str> {
        Some(&self.files.get(self.rows[self.head.row].file)?.path)
    }

    fn len(&self, r: usize) -> usize {
        self.rows[r].line.text.chars().count()
    }

    /// The column row `r`'s char `i` starts at on screen (past its end, a column each).
    pub(super) fn col_of(&self, r: usize, i: usize) -> usize {
        self.rows[r].line.text.chars().chain(std::iter::repeat(' ')).take(i).map(row::char_w).sum()
    }

    /// Row `r`'s char at column `c`, the one it's in (past its end, a column each).
    pub(super) fn char_at(&self, r: usize, c: usize) -> usize {
        let mut used = 0;
        self.rows[r].line.text.chars().chain(std::iter::repeat(' ')).take_while(|&ch| {
            used += row::char_w(ch);
            used <= c
        }).count()
    }

    /// Row `r`'s first char wholly at or after column `c` (one cut by it isn't shown).
    pub(super) fn char_from(&self, r: usize, c: usize) -> usize {
        let i = self.char_at(r, c);
        i + usize::from(self.col_of(r, i) < c)
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

}

/// Leaving while you write beside dv goes only this long after dv said so (`hold_quit`).
const QUIT_AFTER: std::time::Duration = std::time::Duration::from_millis(300);
const QUIT_WITHIN: std::time::Duration = std::time::Duration::from_secs(5);

/// Word-wrap long lines at `w` chars, keeping their indent; short lines are kept as they are.
pub(super) fn wrap(text: &str, w: usize) -> Vec<String> {
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

/// Add `x` to `s`, or remove it if it's there.
fn flip<T: std::hash::Hash + Eq>(s: &mut HashSet<T>, x: T) {
    if !s.remove(&x) {
        s.insert(x);
    }
}

/// `text` as a markdown quote (`> ` lines), followed by a blank line to write under.
fn quote(text: &str) -> String {
    if text.trim().is_empty() {
        return String::new(); // nothing to quote (an empty description, a review with no text)
    }
    let q: Vec<String> = text.lines().map(|l| format!("> {l}").trim_end().to_owned()).collect();
    format!("{}\n\n", q.join("\n"))
}

/// Line `n` (0-based) of a file on disk, reading each file once per `cache`.
fn disk_line(cache: &mut HashMap<PathBuf, String>, p: &Path, n: u32) -> String {
    let text = cache.entry(p.to_owned()).or_insert_with(|| std::fs::read_to_string(p).unwrap_or_default());
    text.lines().nth(n as usize).unwrap_or("").to_owned()
}

impl App {
    /// Whether thread `t` shows: in the Claude view, only those of Claude's review; in the
    /// pending view, those with a comment of your pending review.
    pub(super) fn in_view(&self, t: &Thread) -> bool {
        self.in_view_of(self.view, t)
    }

    /// `in_view`, for view `v` (the pane's tabs count what each would show).
    pub(super) fn in_view_of(&self, v: View, t: &Thread) -> bool {
        // Comments are on the PR's head: not on commits before it (but one about a commit is
        // on no lines: it shows whatever's shown).
        if self.before_head() && commit_of(&t.conversation).is_none() {
            return false;
        }
        Self::of_view(v, t)
    }

    /// Whether thread `t` is of view `v` (Claude's, your pending review's), shown or not.
    pub(super) fn of_view(v: View, t: &Thread) -> bool {
        match v {
            View::All => true,
            View::Claude => t.claude.is_some(),
            View::Pending => t.comments.iter().any(|c| c.pending),
        }
    }

    /// The Claude and Pending tabs: the Claude / pending view, or back to the
    /// whole diff.
    pub(super) fn toggle_view(&mut self, v: View) {
        self.view = if self.view == v { View::All } else { v };
        let keep = self.cursor_key();
        self.rebuild(keep);
        let any = self.any_in_view();
        self.msg = match self.view {
            View::Claude if !any && self.claude.summary.is_empty() && self.claude.broken.is_some() => "Claude view: its review file can't be read right now (see the note; it shows again once it's fixed)",
            View::Claude if !any && self.claude.summary.is_empty() => "Claude view: Claude hasn't reviewed this yet (Space a asks it; gn / gp: the other tabs)",
            View::Claude => "Claude view: its summary and the code it commented on (gn / gp: the other tabs)",
            View::Pending if !any && self.summary_draft.is_empty() => "pending view: no pending review yet (c queues a comment; gn / gp: the other tabs)",
            View::Pending => "pending view: your pending review and the code it's on (S submits it; gn / gp: the other tabs)",
            View::All => "the whole diff",
        }
        .into();
    }

    /// Whether the view has threads in it (on an earlier commit too, where they're hidden).
    pub(super) fn any_in_view(&self) -> bool {
        self.threads.iter().any(|t| Self::of_view(self.view, t))
    }
}
