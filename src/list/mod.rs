//! `dv list`: what needs you (ranked, `todo.rs`), the repo's open PRs, (`Tab`) the ones with a
//! local worktree, and the replacements Claude has drafted (unpublished); `Enter` reviews one,
//! and quitting that review comes back here.
use crate::gh::{self, Listed};
use crate::theme::{Theme, Ui};
use anyhow::{Context, Result};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Alignment, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use ratatui::{DefaultTerminal, Frame};
use crate::batch;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

mod card;
use card::*;
mod todo;
use todo::ago_secs;
#[cfg(test)]
mod tests;

/// PRs looked up on GitHub, by number (`None`: not to be had).
type Lookups = Vec<(u64, Option<Listed>)>;
/// By PR, in its worktree: (changed files, commits beyond the PR's head, when that head was made).
type Unpushed = HashMap<u64, (usize, usize, Option<i64>)>;
type Checks = anyhow::Result<gh::ListChecks>;
/// The open PRs, as fetched (in the background).
type Fetched = Result<Vec<Listed>>;

/// You, and what GitHub says of each PR beyond its card (`gh::list_extra`).
type Extras = anyhow::Result<(String, HashMap<u64, gh::Extra>)>;

const HELP: &str = "\
  j k, Up Down         move
  gg ge                first / last PR
  Ctrl-d Ctrl-u        half a page down / up
  Tab                  todo (what needs you, ranked) / open PRs / PRs with a local worktree
                       (<repo>-N, as dv N -w makes) / replacements (Claude's drafts, unpublished)
                       On todo: a PR or replacement waiting on you (Claude's review or answers,
                       your review requested or none yet, pushed since you reviewed, ready to
                       merge, a replacement to publish or close for), in the order Claude puts
                       them (once you have it rank them: r r), with why and what next; a a
                       there has Claude do its part on one (answer your replies, update its
                       review, or review it)
  Enter                review the PR (with dv list's flags; from the worktrees tab, in its
                       worktree as it was made); q there comes back here
  /                    filter (Enter keeps it, Esc clears it); terms must all match:
                         text            number, title, author, branch, labels
                         author: label: base: branch:   part of that field
                         is:draft  review:approved|changes|required|none
                         ci:pass|fail|pending|none     -term excludes
  Esc                  clear the filter
  o O                  next sort order (created, the default; number, size, updated) / reverse
  m                    mark the PR (again: unmark), for Claude to review; marks are kept, for
                       going through them a PR at a time (quitting dv list too)
  U U                  unmark all
  C                    auto on / off (top bar; git config dv.auto): Claude does its part as
                       things come in, CI-green drafts go ready, replacements close what they
                       replace, the todo list is ranked again
  a a                  Claude reviews the marked PRs (or this one) in the background, two at a
                       time, each in its worktree (<repo>-N, made if need be); cards show how
                       it's going, and claude ✓ once a PR has a review
  A A                  Claude drafts a replacement for this PR, on a branch of its own in
                       <repo>-replace-N, unpublished, for you to review first (replacements
                       tab); for marked PRs, it first suggests which go together (one
                       replacement each group), which you edit; then your instructions
  b b                  ask the PR's author for a rebase: a comment saying it needs one (in your
                       editor first); on the todo tab it's then deferred until it changes
                       (their push brings it back)
  M M                  merge the PR with github-merge.py (Bitcoin Core's script, built into dv;
                       git config dv.mergescript <path> runs your own copy), in the main
                       checkout: its worktree goes first, unless something in it is unpushed;
                       in a zellij pane of its own, or in this terminal
  N N                  Claude drafts a new PR: your editor opens for what it should do (issues
                       by #N, which Claude reads); on a branch of its own (dv/new-K, in
                       <repo>-new-K), unpublished, on the drafts tab: Enter reviews it, S publishes
  W W                  approve the workflow runs waiting on the PR (GitHub holds a first-time
                       contributor's until a maintainer does): its card says '⏸ N to approve'
  D D                  worktrees tab: remove the PR's worktree (not with local changes in it;
                       its branch stays); replacements tab: remove the replacement (the same)
  R                    refresh (from GitHub; no Claude)
  s                    todo: defer the item (then 1 an hour, 4 four hours, d a day, w a week,
                       or u until something happens: #N merged, CI green, an approval, ...):
                       to the bottom till then, or till what it needs changes; s on a
                       deferred one brings it back. p posts what it waits on as a comment
                       (Waiting on: #N is merged.), which dv reads back for everyone: a
                       comment like that (by you or anyone with write access) defers the PR
                       till then, until a later Unblocked
  r r                  todo: Claude ranks what needs you (one Claude run: the header says what
                       the last cost and what's changed since, to judge if it's worth it)
  ?                    this help
  q, Ctrl-c            quit";

pub struct List {
    /// The tabs' columns on the header (for clicks), and the last click on a card (when, which:
    /// a second soon after on the same one opens it).
    tab_hits: Vec<(std::ops::Range<u16>, Tab)>,
    last_click: Option<(std::time::Instant, usize)>,
    prs: Vec<Listed>,
    ui: Ui,
    /// Flags each PR opens with, for the status bar (e.g. "-wb").
    flags: String,
    sel: usize,
    top: usize,
    query: String,
    /// Typing a `/` filter (Enter keeps it, Esc clears it).
    filtering: bool,
    pending_g: bool,
    help: bool,
    order: Order,
    /// Oldest / smallest / lowest first instead.
    rev: bool,
    /// CI check counts, still on their way (they take a few seconds for a busy repo).
    checks: Option<Receiver<Checks>>,
    /// The PRs fetched again in the background (`watch_github`), to show without a wait.
    fresh: Option<(mpsc::Sender<Fetched>, Receiver<Fetched>)>,
    /// PR worktrees: number → (dir, branch or `None` when detached).
    local: HashMap<u64, (PathBuf, Option<String>)>,
    /// The tab showing.
    tab: Tab,
    /// Replacements (`replace.rs`), as cards (`number`: the replacement's), and their records.
    reps: Vec<Listed>,
    rep_of: HashMap<u64, crate::replace::Record>,
    /// Replacements Claude is drafting: how each is going, and the news on its way.
    drafting: HashMap<u64, batch::Status>,
    draft_rx: Vec<Receiver<(u64, batch::Status)>>,
    /// Replacements being drafted by another dv (`dv replace`, dv's `Space a`), from their
    /// records; and when the records last changed, to see that.
    drafting_elsewhere: HashSet<u64>,
    reps_changed: Option<std::time::SystemTime>,
    /// Claude's suggestion of how to group marked PRs into replacements, on its way.
    group_rx: Option<Receiver<anyhow::Result<batch::Groups>>>,
    /// A key that asks to be pressed again (`D` on this PR's worktree, `a`, `q`): any other
    /// key calls it off.
    confirm: Option<(char, u64)>,
    /// PRs marked (`m`) for Claude to review: kept in dv's directory (`marked.json`), so they
    /// outlast a `dv list`; and what was saved last.
    marked: HashSet<u64>,
    marked_saved: HashSet<u64>,
    /// Claude's reviews from here (`a`): how each is going, and the news on its way.
    batch: HashMap<u64, batch::Status>,
    batch_rx: Vec<Receiver<(u64, batch::Status)>>,
    /// What each of those is (a review, an update to one, answers), for the cards.
    batch_task: HashMap<u64, batch::Task>,
    /// The todo tab: you (your login) and what GitHub says beyond the cards (on its way, like
    /// `checks`); what needs you, in order, and whether Claude's ranking of it is stale; the
    /// last ranking, and the next on its way (`R` there, or by itself when it's old).
    me: String,
    extra: HashMap<u64, gh::Extra>,
    extra_rx: Option<Receiver<Extras>>,
    /// The selected PR, looked up on GitHub in the background for Enter (`prefetch`).
    pr_cache: PrCache,
    /// The PR selected, since when, and whether it's been looked up.
    prefetched: (Option<u64>, std::time::Instant, bool),
    /// PRs not in the list that it needs (worktrees', marked ones): as looked up (`gone_news`;
    /// ones not open kept, as they stay so), the lookups under way, and for which.
    gone: HashMap<u64, Listed>,
    gone_rx: Vec<Receiver<Lookups>>,
    gone_pending: HashSet<u64>,
    todo: Vec<crate::todo::Item>,
    since: crate::todo::Since,
    ranked: Option<crate::todo::Ranked>,
    rank_rx: Option<Receiver<Result<crate::todo::Ranked>>>,
    /// Rank (again) once what's needed is in (`R` on the todo tab).
    want_rank: bool,
    /// What Claude is doing, by PR, in any dv (its markers: `watch_running`), and when they
    /// were last looked at.
    running: HashMap<u64, String>,
    /// The same for replacements (and new PRs), by id: Claude at it in a pane in its worktree.
    running_reps: HashMap<u64, String>,
    /// Runs in panes dv started that stopped at the last look: done if still so at the next (one
    /// look can miss Claude's spinner), their panes closed then.
    stopped: HashSet<u64>,
    running_checked: std::time::Instant,
    /// Auto (`C`, kept in git config `dv.auto`): Claude does its part on what needs you, green
    /// drafts are marked ready, and the list is ranked again (`auto_run`). What it's done, once
    /// each: (PR, head, what).
    auto: bool,
    auto_done: HashSet<(u64, String, &'static str)>,
    /// When auto last had Claude rank the list.
    auto_ranked: Option<std::time::Instant>,
    /// What's not pushed in each PR worktree (`scan_unpushed`): changed files, commits beyond
    /// the PR's head, and when that head was made.
    unpushed: Unpushed,
    /// That scan, under way (in the background: a few git runs a worktree add up), and whether
    /// to scan again when it's done (asked for meanwhile).
    unpushed_rx: Option<Receiver<Unpushed>>,
    rescan: bool,
    /// Todo items deferred (`s`), kept in dv's directory; the one `s` is asking how long for.
    deferred: crate::todo::Deferred,
    deferring: Option<String>,
    /// A deferral's "until …" Claude is reading (the item's key), and the states of PRs that
    /// deferrals wait on and that aren't listed.
    until_rx: Option<(String, Receiver<Result<crate::todo::Until>>)>,
    others: HashMap<u64, String>,
    /// Those being looked up (in the background: a GitHub call each).
    others_rx: Vec<Receiver<Vec<(u64, String)>>>,
    others_pending: HashSet<u64>,
    deferred_file: Option<PathBuf>,
    /// The window is wide enough for cards in two zones (set by `draw`; cards are 3 lines then).
    wide: bool,
    /// PRs Claude has a review of, and when its reviews directory last changed: a review
    /// written from a PR's view (or still being, in Claude's pane) shows up as it lands.
    reviewed: HashSet<u64>,
    reviews_dir: Option<std::path::PathBuf>,
    reviews_mtime: Option<std::time::SystemTime>,
    /// PRs with something you haven't seen (`fresh`).
    new: HashSet<u64>,
    /// The repo, for the reviews' worktrees.
    root: PathBuf,
    pub msg: String,
}

/// What was picked: the PR, and for one from the worktrees tab, whether its worktree is on
/// its branch (it opens there, as `dv N -w` / `-wb`).
pub struct Pick {
    pub number: u64,
    pub worktree: Option<bool>,
    /// It's a replacement (`number` is its own): it opens in its worktree.
    pub replacement: bool,
    /// Merge it (`M M`) instead.
    pub merge: bool,
    /// The PR as GitHub has it, if the list looked it up just now (while it was selected): the
    /// review needn't wait on that.
    pub pr: Option<gh::Pr>,
}

/// PRs looked up while selected (`prefetch`), and when.
type PrCache = std::sync::Arc<std::sync::Mutex<HashMap<u64, (std::time::Instant, gh::Pr)>>>;

impl List {
    pub fn new(flags: String) -> List {
        let ui = Theme::load().ui();
        let (query, msg) = (String::new(), String::new());
        List {
            tab_hits: vec![],
            last_click: None,
            prs: vec![],
            ui,
            flags,
            sel: 0,
            top: 0,
            query,
            filtering: false,
            pending_g: false,
            help: false,
            order: Order::Created,
            rev: false,
            checks: None,
            fresh: None,
            local: HashMap::new(),
            tab: Tab::Open,
            reps: vec![],
            rep_of: HashMap::new(),
            drafting: HashMap::new(),
            draft_rx: vec![],
            drafting_elsewhere: HashSet::new(),
            reps_changed: None,
            group_rx: None,
            confirm: None,
            marked: HashSet::new(),
            marked_saved: HashSet::new(),
            batch: HashMap::new(),
            batch_rx: vec![],
            batch_task: HashMap::new(),
            me: String::new(),
            extra: HashMap::new(),
            extra_rx: None,
            gone: HashMap::new(),
            gone_rx: vec![],
            gone_pending: HashSet::new(),
            pr_cache: Default::default(),
            prefetched: (None, std::time::Instant::now(), true),
            todo: vec![],
            since: Default::default(),
            ranked: crate::todo::load(),
            rank_rx: None,
            want_rank: false,
            running: HashMap::new(),
            running_reps: HashMap::new(),
            stopped: HashSet::new(),
            unpushed: HashMap::new(),
            unpushed_rx: None,
            rescan: false,
            running_checked: std::time::Instant::now() - std::time::Duration::from_secs(60),
            auto: crate::diff::git(&["config", "--get", "dv.auto"]).is_ok_and(|v| v.trim() == "true"),
            auto_done: HashSet::new(),
            auto_ranked: None,
            deferred: crate::todo::load_deferred(crate::todo::deferred_path().as_deref()),
            deferring: None,
            until_rx: None,
            others: HashMap::new(),
            others_rx: vec![],
            others_pending: HashSet::new(),
            deferred_file: crate::todo::deferred_path(),
            wide: true,
            reviewed: HashSet::new(),
            reviews_dir: crate::claude::dv_dir().ok().map(|d| d.join("reviews")),
            reviews_mtime: None,
            new: HashSet::new(),
            root: std::env::current_dir().unwrap_or_default(),
            msg,
        }
    }

    /// Show the list until a PR is picked (`Some`) or the user quits (`None`).
    pub fn run(&mut self, term: &mut DefaultTerminal) -> Result<Option<Pick>> {
        // As saved: another dv list of this repo may have changed them meanwhile.
        self.marked = load_marks();
        self.prefetched = (None, std::time::Instant::now(), true); // what's selected is looked up again
        self.marked_saved = self.marked.clone();
        // From GitHub again in the background while it's open (and now, back from a PR: you may
        // have changed it).
        match &self.fresh {
            None => self.watch_github(),
            Some((tx, _)) => {
                let tx = tx.clone();
                std::thread::spawn(move || _ = tx.send(gh::list()));
            }
        }
        if self.prs.is_empty() {
            self.refresh(term)?;
            if !self.marked.is_empty() && self.msg.is_empty() {
                self.msg = format!("{} marked from before: a a or A A acts on them; m unmarks", self.marked.len());
            }
        }
        self.find_new(); // back from a PR: that one's seen now
        self.load_reps();
        self.scan_unpushed(); // and what you (or Claude) left there unpushed
        self.find_todo(); // and what you did there may have seen to it
        loop {
            if self.marked != self.marked_saved {
                // On what's saved now: another dv list of this repo may have marked some too.
                match save_marks(&self.marked_saved, &self.marked) {
                    Ok(now) => self.marked = now,
                    Err(e) => self.msg = format!("couldn't save the marks: {e}"),
                }
                self.marked_saved = self.marked.clone();
            }
            term.draw(|f| self.draw(f))?;
            self.batch_news();
            self.watch_reviews();
            self.todo_news();
            self.unpushed_news();
            self.gone_news();
            self.prefetch();
            self.others_news();
            self.until_news();
            if self.watch_running() {
                self.scan_unpushed(); // what it finished with may be waiting to be pushed
                self.find_todo(); // Claude started or finished somewhere: its item moves
            }
            // A replacement drafted, or its draft changed, from anywhere: show it.
            let changed = crate::replace::changed();
            if changed != self.reps_changed {
                self.reps_changed = changed;
                self.load_reps();
                self.find_todo(); // drafting (in any dv), or drafted: where they go on it
            }
            if let Some(Ok(res)) = self.group_rx.as_ref().map(Receiver::try_recv) {
                self.group_rx = None;
                self.draft_groups(term, res);
            }
            // Quietly: what you're reading stays (an error just waits for the next try).
            if let Some(Ok(Ok(prs))) = self.fresh.as_ref().map(|(_, rx)| rx.try_recv()) {
                self.apply_list(Ok(prs), true);
            }
            if let Some(Ok(res)) = self.checks.as_ref().map(Receiver::try_recv) {
                self.checks = None;
                match res {
                    Ok(mut by_pr) => self.prs.iter_mut().for_each(|p| (p.checks, p.checks_head) = by_pr.remove(&p.number).map_or((None, String::new()), |(c, h)| (Some(c), h))),
                    Err(e) => self.msg = format!("checks: {e:#}"),
                }
                self.find_todo(); // green checks: ready to merge
            }
            if !event::poll(Duration::from_millis(100))? {
                continue;
            }
            let k = match event::read()? {
                Event::Key(k) if k.kind == KeyEventKind::Press => k,
                Event::Mouse(m) => match self.mouse(m) {
                    // A double-click on a card opens it, as Enter.
                    true => KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                    false => continue,
                },
                _ => continue,
            };
            match self.key(k) {
                Some(Do::Quit) => return Ok(None),
                Some(Do::Open(n)) => {
                    let todo_rep = self.tab == Tab::Todo && self.todo_shown().iter().any(|(c, it)| c.number == n && it.rep);
                    let replacement = self.tab == Tab::Replacements || todo_rep;
                    let worktree = (self.tab == Tab::Worktrees).then(|| self.local.get(&n).is_some_and(|w| w.1.is_some()));
                    // Used once: after the review it may have moved on (a push), so it's looked up again.
                    let pr = self.pr_cache.lock().ok().and_then(|mut c| c.remove(&n)).filter(|(t, _)| t.elapsed().as_secs() < 120).map(|(_, p)| p);
                    return Ok(Some(Pick { number: n, worktree, replacement, merge: false, pr }));
                }
                Some(Do::Merge(n)) => {
                    return Ok(Some(Pick { number: n, worktree: None, replacement: false, merge: true, pr: None }));
                }
                Some(Do::Refresh) => {
                    self.refresh(term)?;
                }
                Some(Do::Review(prs)) => {
                    // Instructions for every one of these reviews, if you have any. In zellij the
                    // editor gets a pane beside the list; elsewhere it takes the terminal.
                    let which = prs.iter().map(|n| format!("#{n}")).collect::<Vec<_>>().join(" ");
                    let what = format!("instructions for Claude's reviews of {which} (optional)");
                    let Some(extra) = self.instructions(term, "review", &what) else { continue };
                    // From the todo tab, what's due on each (answers, an update, a review).
                    let prs: Vec<(u64, batch::Task)> = prs.into_iter().map(|n| (n, if self.tab == Tab::Todo { self.todo_task(n) } else { batch::Task::Review })).collect();
                    let with = if extra.is_empty() { "" } else { ", with your instructions" };
                    self.msg = format!("Claude is {}{with}, {} at a time", self.start_batch(prs, extra), batch::AT_ONCE);
                    self.marked.clear();
                }
                // One PR: its replacement. Several: Claude suggests how to group them first.
                Some(Do::Replace(prs)) if prs.len() == 1 => self.draft_groups(term, Ok(vec![(prs, String::new())])),
                Some(Do::Replace(prs)) => {
                    let about = |n: u64| self.prs.iter().find(|p| p.number == n).map(|p| (n, p.title.clone(), p.author.clone(), p.base.clone()));
                    let prs = prs.into_iter().map(|n| about(n).unwrap_or((n, String::new(), String::new(), String::new()))).collect();
                    self.msg = "Claude is looking at how to group them (one replacement per group)…".into();
                    self.group_rx = Some(batch::group(prs, self.root.clone()));
                }
                Some(Do::AskRebase(n)) => self.ask_rebase(term, n),
                Some(Do::PostWait(n, text)) => {
                    let what = format!("comment on #{n} saying what it waits on (dv reads it back, yours and others')");
                    if let Some(body) = self.edit_text(term, &what, &text, false) {
                        self.msg = match gh::conversation_comment(&gh::Pr { number: n, ..Default::default() }, &body) {
                            Ok(()) => format!("posted on #{n}: {}", body.lines().next().unwrap_or("")),
                            Err(e) => format!("not posted: {e:#}"),
                        };
                    }
                }
                Some(Do::DeferUntil(key)) => {
                    let what = "defer it until… (e.g. #2380 is merged, CI is green, @amy approves, 3 days, monday)";
                    if let Some(text) = self.edit_text(term, what, "", false) {
                        self.defer_until(key, &text);
                    }
                }
                Some(Do::NewPr) => self.new_pr(term),
                Some(Do::RemoveReplacement(id)) => {
                    self.msg = crate::replace::remove(id, &self.root).unwrap_or_else(|e| format!("{e:#}"));
                    self.load_reps();
                    self.sel = self.sel.min(self.shown().len().saturating_sub(1));
                }
                Some(Do::Remove(n)) => {
                    self.msg = match crate::remove_worktree(n, false) {
                        Ok(m) => {
                            self.local.remove(&n);
                            // Listed only for its worktree (closed, merged): gone with it.
                            self.prs.retain(|p| p.number != n || p.open());
                            self.sel = self.sel.min(self.shown().len().saturating_sub(1));
                            m
                        }
                        Err(e) => format!("{e:#}"),
                    }
                }
                None => {}
            }
        }
    }

    /// Your instructions to Claude for `kind` of task, in your editor, starting from the last
    /// ones (kept if left as they are). In zellij the editor gets a pane beside the list;
    /// elsewhere it takes the terminal. `None`: cancelled (`:cq`), or it failed (said so).
    fn instructions(&mut self, term: &mut DefaultTerminal, kind: &str, what: &str) -> Option<String> {
        let last = crate::claude::instructions(kind);
        let extra = self.edit_text(term, what, last.trim(), true)?;
        _ = crate::claude::save_instructions(kind, &extra);
        Some(extra)
    }

    /// Your editor on `prefill` (kept as is if you leave it: `untouched_ok`), as `instructions`
    /// opens it. `None`: cancelled, or it failed (said so).
    fn edit_text(&mut self, term: &mut DefaultTerminal, what: &str, prefill: &str, allow_empty: bool) -> Option<String> {
        let in_pane = crate::view::in_zellij();
        if !in_pane {
            crate::view::restore_terminal();
        }
        let text = crate::view::compose_with(what, allow_empty, prefill, true);
        if !in_pane {
            *term = crate::view::init_terminal();
        }
        match text {
            Ok(Some(t)) => Some(t),
            Ok(None) => {
                self.msg = "cancelled".into();
                None
            }
            Err(e) => {
                self.msg = format!("{e:#}");
                None
            }
        }
    }

    /// Marks on PRs closed or merged go (ones not looked up yet stay).
    pub(super) fn prune_marks(&mut self) {
        self.marked.retain(|n| self.prs.iter().find(|p| p.number == *n).is_none_or(|p| !matches!(p.state.as_str(), "CLOSED" | "MERGED")));
    }

    /// The selected PR, looked up on GitHub in the background once it's been selected a moment
    /// (not each one `j` passes): opening it then needn't wait on that, nor, the first time,
    /// on its comments (most of what opening a PR takes).
    fn prefetch(&mut self) {
        // Not a replacement (its number is its own, not a PR's).
        let rep = |n: u64| self.tab == Tab::Todo && self.todo_shown().iter().any(|(c, it)| c.number == n && it.rep);
        let n = self.shown().get(self.sel).map(|p| p.number).filter(|&n| self.tab != Tab::Replacements && !rep(n));
        if n != self.prefetched.0 {
            self.prefetched = (n, std::time::Instant::now(), false);
        }
        if self.prefetched.2 || self.prefetched.1.elapsed() < std::time::Duration::from_millis(300) {
            return;
        }
        self.prefetched.2 = true;
        let Some(n) = n else { return };
        // Looked up a moment ago already (back on it).
        if self.pr_cache.lock().is_ok_and(|c| c.get(&n).is_some_and(|(t, _)| t.elapsed().as_secs() < 120)) {
            return;
        }
        let cache = self.pr_cache.clone();
        std::thread::spawn(move || {
            if let (Ok(p), Ok(mut c)) = (gh::pr(Some(n)), cache.lock()) {
                c.insert(n, (std::time::Instant::now(), p));
            }
            crate::view::prefetch_github(n);
        });
    }

    /// Claude does each task on its PR in the background (`a a`, auto): what it's doing, said.
    pub(super) fn start_batch(&mut self, prs: Vec<(u64, batch::Task)>, extra: String) -> String {
        // A worktree as it was made; a new one as dv list's flags say (-b: on the PR's branch).
        // Fixing CI makes changes to push: on the PR's branch, always.
        let on_branch = |n| self.local.get(&n).map_or(self.flags.contains('b'), |w| w.1.is_some());
        let prs: Vec<(u64, bool, batch::Task)> = prs.into_iter().map(|(n, t)| (n, on_branch(n) || t == batch::Task::FixChecks, t)).collect();
        let doing: Vec<String> = prs.iter().map(|(n, _, t)| format!("{} #{n}", t.doing())).collect();
        for (n, _, t) in &prs {
            self.batch_task.insert(*n, t.clone());
            // Queued now, not once its news comes: what looks at it next sees it under way.
            self.batch.insert(*n, batch::Status::Queued);
            // Fixing CI afresh: its last word that it isn't the PR's doing goes (it says so
            // again, if it still finds that).
            if *t == batch::Task::FixChecks {
                crate::claude::ci_note_path(*n).map(std::fs::remove_file).ok();
            }
        }
        self.batch_rx.push(batch::start(prs, self.root.clone(), extra));
        doing.join(", ")
    }

    /// `N N`: Claude drafts a new PR doing what you write (in your editor), unpublished: on its
    /// own branch in its own worktree, on the drafts tab as it goes.
    fn new_pr(&mut self, term: &mut DefaultTerminal) {
        let what = "a new PR for Claude to draft: what it should do (issues by #N: Claude reads them); unpublished until you publish it";
        // On a todo item whose CI fails through no fault of its own: the fix Claude had in mind.
        let n = self.shown().get(self.sel).map(|p| p.number).filter(|_| self.tab == Tab::Todo);
        let head = |n: u64| self.extra.get(&n).map(|e| e.head_oid.clone());
        let note = n.and_then(|n| Some((n, crate::claude::ci_note(n, &head(n)?)?))).filter(|(_, c)| !c.task.is_empty());
        let prefill = note.as_ref().map_or(String::new(), |(n, c)| c.prefill(*n));
        let Some(task) = self.edit_text(term, what, &prefill, false).filter(|t| !t.trim().is_empty()) else { return };
        match crate::replace::create_new(&task, &self.root) {
            Ok(id) => {
                if let Some((n, _)) = note {
                    crate::claude::ci_fix_drafted(n); // not offered again
                }
                self.draft_rx.push(batch::replace(vec![id], self.root.clone(), String::new()));
                self.load_reps();
                self.msg = format!("Claude is drafting {}: the drafts tab shows it (Enter there reviews it, S publishes it)", crate::replace::label(id));
            }
            Err(e) => self.msg = format!("couldn't start a new PR: {e:#}"),
        }
    }

    /// `b b`: ask PR `n`'s author for a rebase (a comment, after your editor), then defer it
    /// on the todo tab until it changes (a push brings it back).
    fn ask_rebase(&mut self, term: &mut DefaultTerminal, n: u64) {
        let base = self.prs.iter().find(|p| p.number == n).map_or("its base".to_owned(), |p| format!("`{}`", p.base));
        let what = format!("comment asking for a rebase of #{n} (posted on its conversation)");
        let Some(body) = self.edit_text(term, &what, &format!("Needs rebase: this conflicts with {base} now."), false) else { return };
        let pr = gh::Pr { number: n, ..Default::default() };
        if let Err(e) = gh::conversation_comment(&pr, &body) {
            return self.msg = format!("not posted: {e:#}");
        }
        self.msg = format!("asked for a rebase of #{n}");
        if let Some(it) = self.todo.iter().find(|it| !it.rep && it.number == n).cloned() {
            self.deferred.insert(it.key(), crate::todo::Deferral { until: crate::claude::now_secs() + 7 * 86400, needs: it.needs_key(), cond: None });
            self.msg += "; deferred on the todo tab until it changes";
            self.save_defer();
        }
    }

    /// Draft a replacement for each group, after your editor: the groups (Claude's suggestion,
    /// for several PRs, a line each), then instructions for all of them.
    fn draft_groups(&mut self, term: &mut DefaultTerminal, groups: Result<batch::Groups>) {
        let (groups, why) = match groups {
            Ok(g) => (g, String::new()),
            Err(e) => (vec![], format!("{e:#}")),
        };
        let suggested = batch::groups_text(&self.group_notes(&groups));
        let mut groups: Vec<Vec<u64>> = groups.iter().map(|g| g.0.clone()).collect();
        if groups.len() > 1 || groups.first().is_some_and(|g| g.len() > 1) || !why.is_empty() {
            let what = match why.is_empty() {
                true => "replacements to draft: one per line, the PRs it replaces (Claude's suggestion; change it as you like, # notes are ignored)".to_owned(),
                false => format!("replacements to draft, one per line (Claude couldn't suggest any: {why})"),
            };
            let in_pane = crate::view::in_zellij();
            if !in_pane {
                crate::view::restore_terminal();
            }
            let edited = crate::view::compose_with(&what, false, &suggested, true);
            if !in_pane {
                *term = crate::view::init_terminal();
            }
            match edited {
                Ok(Some(text)) => groups = batch::parse_groups_text(&text),
                Ok(None) => return self.msg = "cancelled".into(),
                Err(e) => return self.msg = format!("{e:#}"),
            }
        }
        groups.retain(|g| !g.is_empty() && !self.drafting_elsewhere.contains(&g[0]) && !self.drafting.get(&g[0]).is_some_and(batch::Status::running));
        if groups.is_empty() {
            return self.msg = "nothing to draft".into();
        }
        let which: Vec<String> = groups.iter().map(|g| g.iter().map(|n| format!("#{n}")).collect::<Vec<_>>().join("+")).collect();
        let what = format!("instructions for Claude's replacement of {} (optional)", which.join(", "));
        let Some(extra) = self.instructions(term, "replace", &what) else { return };
        self.msg = format!("Claude is drafting {} replacement(s): the replacements tab shows them", groups.len());
        self.marked.clear();
        for g in groups {
            self.draft_rx.push(batch::replace(g, self.root.clone(), extra.clone()));
        }
    }

    /// Groups with Claude's reason for each, if it gave one (else the PRs' titles).
    fn group_notes(&self, groups: &batch::Groups) -> batch::Groups {
        groups
            .iter()
            .map(|(g, why)| {
                let titles: Vec<String> = g.iter().filter_map(|n| self.prs.iter().find(|p| p.number == *n).map(|p| p.title.clone())).collect();
                (g.clone(), if why.is_empty() { titles.join(" / ") } else { why.clone() })
            })
            .collect()
    }

    /// The replacements, as cards: what they replace, and their size against their base.
    fn load_reps(&mut self) {
        let records = crate::replace::all();
        self.drafting_elsewhere = records.iter().filter(|r| r.drafting_now()).map(|r| r.id).collect();
        self.reps = records.iter().map(|r| rep_card(r, &self.root)).collect();
        self.rep_of = records.into_iter().map(|r| (r.id, r)).collect();
    }

    /// How Claude's drafts of replacements are going.
    fn draft_news(&mut self) {
        let news: Vec<(u64, batch::Status)> = self.draft_rx.iter().flat_map(|rx| rx.try_iter()).collect();
        for (id, s) in news {
            match &s {
                batch::Status::Done(took) => self.msg = format!("Claude drafted a replacement for #{id} ({took}): it's in the replacements tab"),
                batch::Status::Failed(e) => self.msg = format!("Claude's draft of {} failed: {e}", crate::replace::label(id)),
                _ => {}
            }
            let done = !s.running();
            self.drafting.insert(id, s);
            if done || !self.rep_of.contains_key(&id) {
                self.load_reps(); // its record exists from Preparing on; its drafts once done
            }
            self.find_todo(); // drafting: its PRs to the bottom; drafted: the replacement on top
        }
    }

    /// How a replacement's draft stands, for its card, or (`of`) for those of the PRs it
    /// replaces: "claude ✓" there reads "replacement ✓".
    fn draft_state(&self, id: u64, of: &str) -> Option<(String, bool)> {
        let (what, ok) = match self.drafting.get(&id) {
            _ if self.drafting_elsewhere.contains(&id) => ("drafting…", true),
            Some(batch::Status::Failed(e)) => return Some((format!("{of}: draft failed: {}", cut(e.lines().next().unwrap_or(""), 60)), false)),
            Some(s) if s.running() => ("drafting…", true),
            _ => match self.rep_of.get(&id)? {
                r if r.published.is_some() => return Some((format!("{of}: published as #{}", r.published.unwrap_or(0)), true)),
                r if r.title.is_empty() => ("no draft yet", false),
                _ => ("✓", true),
            },
        };
        Some((format!("{of}{}{what}", if what == "✓" { " " } else { ": " }), ok))
    }

    /// How Claude's reviews are going: each card shows it; one finished counts as reviewed.
    fn batch_news(&mut self) {
        self.draft_news();
        let news: Vec<(u64, batch::Status)> = self.batch_rx.iter().flat_map(|rx| rx.try_iter()).collect();
        let mut worktrees = false;
        // Any news (started, done, failed) moves its item on the todo tab.
        let done = !news.is_empty();
        for (n, s) in news {
            let doing = self.batch_task.get(&n).map_or("reviewing", batch::Task::doing);
            match &s {
                batch::Status::Done(took) => {
                    self.reviewed.insert(n);
                    self.new.insert(n);
                    self.msg = match doing {
                        "reviewing" => format!("Claude reviewed #{n} ({took}): Enter opens it, in its worktree from the worktrees tab"),
                        d => format!("Claude is done {d} on #{n} ({took})"),
                    };
                }
                batch::Status::Failed(e) => self.msg = format!("Claude's {doing} on #{n} failed: {e} (a a asks again)"),
                batch::Status::Reviewing => worktrees = true, // its worktree is ready
                _ => {}
            }
            self.batch.insert(n, s);
        }
        if worktrees {
            self.local = crate::pr_worktrees().into_iter().map(|(n, dir, b)| (n, (dir, b))).collect();
        }
        if done {
            self.scan_unpushed();
            self.find_todo(); // Claude at it, or done (its review, or answers, changed what's waiting)
        }
    }

    /// The PRs with something you haven't seen: anything on GitHub (its update time) or in
    /// Claude's review since you last left it in dv. Of those you've never opened, only the ones
    /// Claude has reviewed (else every PR would be new).
    fn find_new(&mut self) {
        self.new = self
            .prs
            .iter()
            .filter(|p| {
                let claude = crate::claude::review_changed(p.number);
                match crate::claude::looked(p.number) {
                    Some(t) => gh::epoch_secs(&p.updated).is_some_and(|u| u > t) || claude.is_some_and(|c| c > t),
                    None => claude.is_some(),
                }
            })
            .map(|p| p.number)
            .collect();
    }

    /// Claude's reviews read again when one is added: started from a PR's view, it may finish
    /// after you're back here.
    fn watch_reviews(&mut self) {
        let Some(dir) = &self.reviews_dir else { return };
        let mtime = std::fs::metadata(dir).and_then(|m| m.modified()).ok();
        if mtime != self.reviews_mtime {
            self.reviews_mtime = mtime;
            self.reviewed = crate::batch::reviewed_in(dir);
        }
    }

    fn refresh(&mut self, term: &mut DefaultTerminal) -> Result<()> {
        self.msg = "loading PRs…".into();
        term.draw(|f| self.draw(f))?;
        self.apply_list(gh::list(), false);
        Ok(())
    }

    /// Watch GitHub in the background for as long as dv runs: the open PRs' ETag every 5s, as
    /// the PR view does (a 304 doesn't count against the rate limit), fetching them again when it
    /// changes; and every 5 minutes regardless (CI doesn't change it). What's fetched shows as it
    /// comes (`apply_list`).
    fn watch_github(&mut self) {
        let (tx, rx) = mpsc::channel();
        let sender = tx.clone();
        std::thread::spawn(move || {
            let mut etag = gh::list_etag("").ok().flatten().unwrap_or_default();
            let mut fetched = std::time::Instant::now();
            loop {
                std::thread::sleep(Duration::from_secs(5));
                let changed = match gh::list_etag(&etag) {
                    Ok(Some(e)) => (etag = e, true).1,
                    _ => false,
                };
                if changed || fetched.elapsed() >= Duration::from_secs(300) {
                    fetched = std::time::Instant::now();
                    if sender.send(gh::list()).is_err() {
                        return; // dv list is done
                    }
                }
            }
        });
        self.fresh = Some((tx, rx));
    }

    /// The PRs as fetched: on the cards, keeping the selected one selected. `quiet` (in the
    /// background): no message, and the CI counts kept until the new ones come.
    fn apply_list(&mut self, res: Result<Vec<Listed>>, quiet: bool) {
        let keep = self.shown().get(self.sel).map(|p| p.number);
        self.local = crate::pr_worktrees().into_iter().map(|(n, dir, b)| (n, (dir, b))).collect();
        self.scan_unpushed();
        self.find_others();
        self.reviewed = crate::batch::reviewed();
        self.load_reps();
        match res {
            Ok(mut prs) => {
                if quiet {
                    for p in &mut prs {
                        if let Some(o) = self.prs.iter().find(|o| o.number == p.number) {
                            (p.checks, p.checks_head) = (o.checks, o.checks_head.clone());
                        }
                    }
                }
                let (tx, rx) = mpsc::channel();
                std::thread::spawn(move || tx.send(gh::list_checks()));
                self.checks = Some(rx);
                let (tx, rx) = mpsc::channel();
                std::thread::spawn(move || tx.send(gh::list_extra()));
                self.extra_rx = Some(rx);
                if !quiet {
                    self.msg = if prs.is_empty() { "no open PRs".into() } else { String::new() };
                }
                // PRs of worktrees or marks that aren't in the list (not open any more, or beyond
                // it): closed ones as looked up before, else looked up in the background (a
                // GitHub call each, a second or so: the list doesn't wait on them).
                let mut wanted: Vec<u64> = self.local.keys().chain(&self.marked).filter(|n| !prs.iter().any(|p| p.number == **n)).copied().collect();
                wanted.sort();
                wanted.dedup();
                let mut missing = vec![];
                for n in wanted {
                    match self.gone.get(&n).filter(|p| !p.open()) {
                        Some(p) => prs.push(p.clone()),
                        None => {
                            if self.gone_pending.insert(n) {
                                missing.push(n);
                            }
                            prs.push(Listed { number: n, title: "(looking it up…)".into(), state: gh::LOOKING.into(), ..Listed::default() });
                        }
                    }
                }
                if !missing.is_empty() {
                    let (tx, rx) = mpsc::channel();
                    std::thread::spawn(move || _ = tx.send(missing.into_iter().map(|n| (n, gh::listed_one(n).ok())).collect()));
                    self.gone_rx.push(rx);
                }
                self.prs = prs;
                self.prune_marks();
                self.find_new();
                self.find_todo();
                self.sel = keep.and_then(|n| self.shown().iter().position(|p| p.number == n)).unwrap_or(0);
            }
            Err(e) => self.msg = format!("{e:#}"),
        }
    }

    /// The tab's PRs matching every filter term, in the chosen order.
    fn shown(&self) -> Vec<&Listed> {
        let in_tab = |p: &&Listed| match self.tab {
            Tab::Worktrees => self.local.contains_key(&p.number),
            _ => p.open(),
        };
        let all: Vec<&Listed> = match self.tab {
            Tab::Replacements => self.reps.iter().collect(),
            Tab::Todo => self.todo_shown().into_iter().map(|(card, _)| card).collect(),
            _ => self.prs.iter().filter(in_tab).collect(),
        };
        let mut prs: Vec<&Listed> = all.into_iter().filter(|p| self.query.split_whitespace().all(|t| term(p, t))).collect();
        if self.tab == Tab::Todo {
            return prs; // in the order of what needs you most
        }
        prs.sort_by(|a, b| {
            let o = match self.order {
                Order::Updated => b.updated.cmp(&a.updated), // ISO times sort as text
                Order::Created => b.created.cmp(&a.created),
                Order::Number => b.number.cmp(&a.number),
                Order::Size => (b.additions + b.deletions).cmp(&(a.additions + a.deletions)),
            };
            if self.rev { o.reverse() } else { o }
        });
        prs
    }

    /// Change the order, keeping the selected PR selected.
    fn sort(&mut self, order: Order, rev: bool) {
        let keep = self.shown().get(self.sel).map(|p| p.number);
        (self.order, self.rev) = (order, rev);
        self.sel = keep.and_then(|n| self.shown().iter().position(|p| p.number == n)).unwrap_or(0);
    }

    fn key(&mut self, k: KeyEvent) -> Option<Do> {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && k.code == KeyCode::Char('c') {
            return Some(Do::Quit);
        }
        let n = self.shown().len();
        self.msg.clear();
        let armed = self.confirm.take();
        if std::mem::take(&mut self.help) {
            return None; // any key closes the help
        }
        if self.filtering {
            match k.code {
                KeyCode::Esc => (self.query.clear(), self.filtering = false).1,
                KeyCode::Enter => self.filtering = false,
                KeyCode::Backspace => _ = self.query.pop(),
                KeyCode::Char(c) => self.query.push(c),
                _ => {}
            }
            self.sel = 0;
            return None;
        }
        let half = (self.height() / 2).max(1);
        let g = std::mem::take(&mut self.pending_g);
        // After `s`: how long to defer it for, or until what (any other key: not).
        if let Some(key) = self.deferring.take() {
            match k.code {
                KeyCode::Char(c @ ('1' | '4' | 'd' | 'w')) => self.defer_for(&key, c),
                KeyCode::Char('u') => return Some(Do::DeferUntil(key)),
                _ => self.msg = "not deferred".into(),
            }
            return None;
        }
        match k.code {
            // A filter that's just a number opens that PR, listed or not (older, or closed).
            KeyCode::Enter => {
                let typed = self.query.trim().trim_start_matches('#').parse().ok();
                return self.shown().get(self.sel).map(|p| p.number).or(typed).map(Do::Open);
            }
            KeyCode::Esc if !self.query.is_empty() => (self.query.clear(), self.sel = 0).1,
            // Reviews running: they'd stop with dv (the ones started finish; queued ones don't start).
            KeyCode::Char('q') | KeyCode::Esc if self.batch.values().any(batch::Status::running) && armed != Some(('q', 0)) => {
                self.msg = "Claude is still reviewing: q again quits (reviews under way finish; queued ones don't start)".into();
                self.confirm = Some(('q', 0));
            }
            KeyCode::Char('q') | KeyCode::Esc => return Some(Do::Quit),
            KeyCode::Char('R') => return Some(Do::Refresh),
            KeyCode::Char('D') if self.tab == Tab::Replacements => {
                let id = self.shown().get(self.sel).map(|p| p.number)?;
                if armed == Some(('D', id)) {
                    return Some(Do::RemoveReplacement(id));
                }
                self.msg = format!("D again removes {} (its worktree and record; its branch stays); any other key keeps it", crate::replace::label(id));
                self.confirm = Some(('D', id));
            }
            KeyCode::Char('m' | 'a' | 'A') if self.tab == Tab::Replacements => {
                self.msg = "a draft: Enter opens it in dv, D D removes it (N N drafts a new PR)".into();
            }
            KeyCode::Char('D') if self.tab == Tab::Worktrees => {
                let p = self.shown().get(self.sel).map(|p| p.number)?;
                let (dir, _) = self.local.get(&p)?;
                if armed == Some(('D', p)) {
                    return Some(Do::Remove(p));
                }
                self.msg = format!("D again removes {} (its branch stays); any other key keeps it", dir.display());
                self.confirm = Some(('D', p));
            }
            KeyCode::Char('s') if self.tab == Tab::Todo => self.defer_key(),
            KeyCode::Char('p') if self.tab == Tab::Todo => {
                let it = self.todo_shown().get(self.sel).map(|(_, it)| (*it).clone())?;
                match it.deferred.and_then(|d| d.cond).filter(|_| !it.rep) {
                    Some(c) => return Some(Do::PostWait(it.number, format!("Waiting on: {}.", c.says()))),
                    None => self.msg = "p posts what an item is deferred until (s then u) on its PR: this one isn't".into(),
                }
            }
            KeyCode::Char('r') if self.tab == Tab::Todo => {
                if self.rank_rx.is_some() {
                    return (self.msg = "Claude is ranking already".into(), None).1;
                }
                if armed == Some(('r', 0)) {
                    self.rank_now();
                    return None;
                }
                self.msg = self.rank_offer();
                self.confirm = Some(('r', 0));
            }
            KeyCode::Char('b') if self.tab != Tab::Replacements => {
                let p = self.shown().get(self.sel).map(|p| p.number)?;
                if armed == Some(('b', p)) {
                    return Some(Do::AskRebase(p));
                }
                self.msg = format!("b again: ask #{p}'s author for a rebase (a comment; your editor opens first); any other key doesn't");
                self.confirm = Some(('b', p));
            }
            KeyCode::Char('N') => {
                if armed == Some(('N', 0)) {
                    return Some(Do::NewPr);
                }
                self.msg = "N again: Claude drafts a new PR, unpublished (your editor opens first: what it should do; issues by #N); any other key doesn't".into();
                self.confirm = Some(('N', 0));
            }
            KeyCode::Char('W') if self.tab != Tab::Replacements => {
                let p = self.shown().get(self.sel).map(|p| p.number)?;
                let Some(head) = self.extra.get(&p).filter(|e| e.awaiting > 0).map(|e| e.head_oid.clone()) else {
                    self.msg = format!("#{p} has no workflow runs waiting for approval");
                    return None;
                };
                if armed == Some(('W', p)) {
                    self.msg = match crate::gh::approve_runs(&head) {
                        Ok(k) => {
                            if let Some(e) = self.extra.get_mut(&p) {
                                e.awaiting = 0;
                            }
                            self.find_todo();
                            format!("approved {k} workflow run{} on #{p}: they run now", if k == 1 { "" } else { "s" })
                        }
                        Err(e) => format!("not approved: {e:#}"),
                    };
                    return None;
                }
                self.msg = format!("W again: approve the workflow runs waiting on #{p} (they run then); any other key doesn't");
                self.confirm = Some(('W', p));
            }
            KeyCode::Char('M') if self.tab != Tab::Replacements => {
                let p = self.shown().get(self.sel).map(|p| p.number)?;
                if armed == Some(('M', p)) {
                    return Some(Do::Merge(p));
                }
                let wt = if self.local.contains_key(&p) { " (its worktree goes first, if nothing in it is unpushed)" } else { "" };
                self.msg = format!("M again: merge #{p} with github-merge.py{wt}; any other key doesn't");
                self.confirm = Some(('M', p));
            }
            KeyCode::Char('m') => {
                let (p, state) = self.shown().get(self.sel).map(|p| (p.number, p.state.clone()))?;
                if !matches!(state.as_str(), "OPEN" | "") && !self.marked.contains(&p) {
                    return (self.msg = format!("#{p} isn't open: nothing to mark it for"), None).1;
                }
                if !self.marked.remove(&p) {
                    self.marked.insert(p);
                }
                self.sel = (self.sel + 1).min(n.saturating_sub(1)); // on to the next, to mark a run
                self.msg = format!("{} marked: a asks Claude to review them", self.marked.len());
            }
            // Auto on / off: Claude runs, drafts marked ready, rankings, without you.
            KeyCode::Char('C') => {
                self.auto = !self.auto;
                _ = crate::diff::git(&["config", "dv.auto", if self.auto { "true" } else { "false" }]);
                self.msg = match self.auto {
                    true => "auto on: Claude does its part as things come in (fixes your CI, reviews, answers you; Claude runs, at your cost), green drafts are marked ready, and it ranks the list again as it changes".into(),
                    false => "auto off: a a has Claude do its part, r r ranks, Space g in a PR marks it ready".into(),
                };
                self.find_todo();
            }
            // All the marks off (kept between runs, they pile up): asked first.
            KeyCode::Char('U') => {
                if self.marked.is_empty() {
                    return (self.msg = "nothing marked".into(), None).1;
                }
                if armed == Some(('U', 0)) {
                    self.msg = format!("unmarked {}", self.marked.len());
                    self.marked.clear();
                    return None;
                }
                self.msg = format!("U again unmarks all {} marked PRs; any other key doesn't", self.marked.len());
                self.confirm = Some(('U', 0));
            }
            KeyCode::Char('a') => {
                let mut prs: Vec<u64> = match self.marked.is_empty() {
                    true => self.shown().get(self.sel).map(|p| vec![p.number]).unwrap_or_default(),
                    false => self.marked.iter().copied().collect(),
                };
                prs.retain(|n| !self.batch.get(n).is_some_and(batch::Status::running));
                prs.sort();
                if prs.is_empty() {
                    return None;
                }
                if armed == Some(('a', 0)) {
                    return Some(Do::Review(prs));
                }
                let which = match self.tab {
                    Tab::Todo => prs.iter().map(|&n| format!("{} #{n}", self.todo_task(n).doing())).collect::<Vec<_>>().join(", "),
                    _ => format!("reviews {}", prs.iter().map(|n| format!("#{n}")).collect::<Vec<_>>().join(" ")),
                };
                self.msg = format!("a again: Claude {which} in the background, each in its worktree (<repo>-N); your editor opens first, for any instructions");
                self.confirm = Some(('a', 0));
            }
            KeyCode::Char('A') => {
                let mut prs: Vec<u64> = match self.marked.is_empty() {
                    true => self.shown().get(self.sel).map(|p| vec![p.number]).unwrap_or_default(),
                    false => self.marked.iter().copied().collect(),
                };
                prs.sort();
                let id = *prs.first()?;
                if self.drafting.get(&id).is_some_and(batch::Status::running) || self.drafting_elsewhere.contains(&id) {
                    return (self.msg = format!("Claude is already drafting {}", crate::replace::label(id)), None).1;
                }
                if armed == Some(('A', 0)) {
                    return Some(Do::Replace(prs));
                }
                let which = prs.iter().map(|n| format!("#{n}")).collect::<Vec<_>>().join(" ");
                let again = if self.rep_of.contains_key(&id) { " (again: it has one, which Claude carries on from)" } else { "" };
                self.msg = match prs.len() {
                    1 => format!("A again: Claude drafts a replacement for {which}{again}, unpublished; your editor opens first, for instructions"),
                    _ => format!("A again: Claude suggests how to group {which} into replacements (you decide, in your editor), then drafts them, unpublished"),
                };
                self.confirm = Some(('A', 0));
            }
            KeyCode::Char('?') => self.help = true,
            KeyCode::Tab | KeyCode::BackTab => {
                let tab = if k.code == KeyCode::Tab { self.tab.next() } else { self.tab.prev() };
                (self.tab, self.sel, self.top) = (tab, 0, 0);

            }
            KeyCode::Char('/') => self.filtering = true, // edits the current filter
            KeyCode::Char('o') => self.sort(self.order.next(), self.rev),
            KeyCode::Char('O') => self.sort(self.order, !self.rev),
            KeyCode::Char('d') if ctrl => self.sel = (self.sel + half).min(n.saturating_sub(1)),
            KeyCode::Char('u') if ctrl => self.sel = self.sel.saturating_sub(half),
            KeyCode::Char('j') | KeyCode::Down => self.sel = (self.sel + 1).min(n.saturating_sub(1)),
            KeyCode::Char('k') | KeyCode::Up => self.sel = self.sel.saturating_sub(1),
            KeyCode::Char('g') if g => self.sel = 0,
            KeyCode::Char('e') if g => self.sel = n.saturating_sub(1),
            KeyCode::Char('g') => self.pending_g = true,
            _ => {}
        }
        None
    }

    /// The wheel moves the selection; a click selects a card, and on the selected one opens it
    /// (`true`: open).
    /// A click selects a card, a double-click opens it (`true`: as Enter); a click on a tab
    /// switches to it; the wheel moves the selection.
    fn mouse(&mut self, m: event::MouseEvent) -> bool {
        use event::{MouseButton, MouseEventKind};
        let n = self.shown().len();
        match m.kind {
            MouseEventKind::ScrollDown => self.sel = (self.sel + 1).min(n.saturating_sub(1)),
            MouseEventKind::ScrollUp => self.sel = self.sel.saturating_sub(1),
            MouseEventKind::Down(MouseButton::Left) if !self.help && m.row == 0 => {
                if let Some(&(_, tab)) = self.tab_hits.iter().find(|(x, _)| x.contains(&m.column)) {
                    (self.tab, self.sel, self.top) = (tab, 0, 0);
                }
            }
            MouseEventKind::Down(MouseButton::Left) if !self.help => {
                let i = self.top + (m.row as usize - 1) / self.card_rows(); // cards start under the header
                if i >= n {
                    return false;
                }
                let double = self.last_click.is_some_and(|(at, j)| j == i && at.elapsed() < Duration::from_millis(400));
                self.last_click = Some((std::time::Instant::now(), i));
                self.sel = i;
                if double {
                    self.last_click = None;
                    return true;
                }
            }
            _ => {}
        }
        false
    }

    /// Cards on screen (between the header and the status bar).
    fn height(&self) -> usize {
        ratatui::crossterm::terminal::size().map_or(10, |(_, h)| h.saturating_sub(2) as usize / self.card_rows())
    }

    /// A card's rows: 3 in a wide window (two zones), else 4, and a blank one between cards; on
    /// the todo tab, one more, for what needs you.
    fn card_rows(&self) -> usize {
        let lines = if self.wide { 3 } else { 4 };
        lines + usize::from(self.tab == Tab::Todo) + 1
    }

    fn draw(&mut self, f: &mut Frame) {
        let area = f.area();
        self.wide = area.width as usize >= TWO_ZONES;
        let rows = self.card_rows();
        let h = area.height.saturating_sub(2) as usize / rows;
        let sel = self.sel.min(self.shown().len().saturating_sub(1));
        // Keep the selection on screen.
        (self.sel, self.top) = (sel, self.top.min(sel).max((sel + 1).saturating_sub(h)));
        let (ui, shown) = (&self.ui, self.shown());
        f.render_widget(Paragraph::new("").style(ui.base), area);

        let head = Rect { height: 1.min(area.height), ..area };
        // As the PR view's title bar: the darker of the status bar's and the headers' background.
        let hs = ui.base.patch(Style { bg: ui.bar, ..ui.header });
        let arrow = if self.rev { '↑' } else { '↓' };
        // Tabs: the current one bold, with counts.
        let open = self.prs.iter().filter(|p| p.open()).count();
        // As the PR view's tabs: the one shown lit, the others dim.
        let dim = hs.remove_modifier(ratatui::style::Modifier::BOLD).patch(Style { fg: ui.linenr.fg, ..Style::new() });
        let tab = |name: String, on: bool| Span::styled(format!(" {name} "), if on { ui.menu_selected } else { dim });
        // What you'd judge a ranking by: how old the last is, what it cost, what's changed since.
        let ranking = match (&self.rank_rx, &self.ranked) {
            (Some(_), _) => "Claude is ranking…".to_owned(),
            (None, Some(r)) => format!("ranked by Claude {} ago ({}){}  · r r ranks again", ago_secs(crate::claude::now_secs() - r.at), todo::cost_text(r.cost), self.since_text()),
            (None, None) => "in the rules' order  · r r has Claude rank it (one run)".to_owned(),
        };
        let rest = match self.tab {
            Tab::Todo => format!("   {ranking}"),
            _ => format!("   by {} {arrow}  (Tab switches)", self.order.name()),
        };
        let names = [
            (Tab::Todo, format!("todo {}", self.todo_now())),
            (Tab::Open, format!("open {open}{}", if self.prs.len() >= gh::LIST_MAX { "+" } else { "" })),
            (Tab::Worktrees, format!("worktrees {}", self.local.len())),
            (Tab::Replacements, format!("drafts {}", self.reps.len())),
        ];
        let mut spans = vec![Span::styled(" ", hs)];
        let mut hits = vec![];
        let mut x = 1u16;
        for (t, name) in names {
            let s = tab(name, self.tab == t);
            let w = s.width() as u16;
            hits.push((x..x + w, t));
            spans.extend([s, Span::styled(" ", hs)]);
            x += w + 1;
        }
        spans.push(Span::styled(rest, dim));
        // Auto, lit when on: it starts Claude runs, and changes PRs on GitHub.
        let auto = match self.auto {
            true => Span::styled(" auto: CI · review · rank ", ui.menu_selected),
            false => Span::styled("auto off (C)", dim),
        };
        spans.extend([Span::styled("   ", dim), auto]);
        let tabs = Line::from(spans);
        f.render_widget(Paragraph::new(tabs).style(hs), head);
        let pos = if shown.is_empty() { "0 / 0 ".into() } else { format!("{} / {} ", sel + 1, shown.len()) };
        // Only where it goes (at the right end): over the whole line, its style would take the
        // lit tab's.
        let w = (pos.chars().count() as u16).min(head.width);
        f.render_widget(Paragraph::new(pos).style(hs), Rect { x: head.right() - w, width: w, ..head });

        // The base most PRs target goes without saying; others show as "base ← branch".
        let mut bases: HashMap<&str, usize> = HashMap::new();
        self.prs.iter().for_each(|p| *bases.entry(p.base.as_str()).or_default() += 1);
        let usual = bases.into_iter().max_by_key(|b| b.1).map(|b| b.0.to_owned()).unwrap_or_default();
        let cx = Cards {
            ui,
            num_w: shown.iter().map(|p| crate::replace::shown(p.number).chars().count()).max().unwrap_or(2),
            width: area.width as usize,
            checks_loading: self.checks.is_some(),
            usual: &usual,
        };
        let lines: Vec<Line> = shown
            .iter()
            .enumerate()
            .skip(self.top)
            .take(h)
            .flat_map(|(i, p)| {
                let item = (self.tab == Tab::Todo).then(|| self.todo_shown().into_iter().find(|(c, _)| std::ptr::eq(*c, *p)).map(|x| x.1)).flatten();
                let rep_card = self.tab == Tab::Replacements || item.is_some_and(|it| it.rep);
                let mark = match rep_card {
                    true => {
                        let replaces = self.rep_of.get(&p.number).map(|r| match r.replaces.is_empty() {
                            // A new PR: what you asked of it.
                            true => format!("asked: {}", r.task.lines().next().unwrap_or("")),
                            false => {
                                let each: Vec<String> = r.replaces.iter().map(|o| format!("#{} (@{})", o.number, o.author)).collect();
                                format!("replaces {}", each.join(", "))
                            }
                        });
                        Mark { marked: false, claude: self.draft_state(p.number, "claude"), new: false, replaces, awaiting: 0 }
                    }
                    false => {
                        // On the todo tab, what Claude could do there (`a a`), when it isn't at it.
                        let could = item.and_then(|it| it.claude.as_ref()).filter(|_| !self.batch.get(&p.number).is_some_and(batch::Status::running)).map(batch::Task::could);
                        let claude = match (self.claude_state(p.number), could) {
                            (Some((s, ok)), Some(c)) => Some((format!("{s} · {c}"), ok)),
                            (None, Some(c)) => Some((format!("claude {c}"), true)),
                            (s, None) => s,
                        };
                        let awaiting = self.extra.get(&p.number).map_or(0, |e| e.awaiting);
                        Mark { marked: self.marked.contains(&p.number), claude, new: self.new.contains(&p.number), replaces: None, awaiting }
                    }
                };
                let local = if rep_card { None } else { self.local.get(&p.number) };
                let mut lines = card(p, &cx, i == sel, local, mark);
                if let Some(it) = item {
                    lines.insert(1, todo_line(i + 1, it, &cx, i == sel)); // under the title
                }
                lines.push(Line::raw("")); // room between cards
                lines
            })
            .collect();
        let typed = self.query.trim().trim_start_matches('#').parse::<u64>().ok();
        let none = match (self.tab, self.query.is_empty(), typed) {
            (Tab::Worktrees, true, _) => " no PR worktrees here yet: `dv N -w` (or dv list -w, then Enter) makes one".into(),
            (Tab::Replacements, true, _) => " no replacements yet: A A on a PR (or marked ones) has Claude draft one".into(),
            (_, _, Some(n)) => format!(" #{n} isn't listed: Enter opens it anyway"),
            _ => String::new(),
        };
        if shown.is_empty() {
            f.render_widget(Paragraph::new(none).style(ui.base.patch(ui.stale)), Rect { y: area.y + 2, height: 1, ..area });
        }
        f.render_widget(Paragraph::new(lines), Rect { y: area.y + 1, height: (h * rows) as u16, ..area });

        let status = Rect { y: area.bottom().saturating_sub(1), height: 1.min(area.height), ..area };
        let bar = ui.base.patch(ui.status);
        let left = match (self.filtering, self.query.is_empty(), self.msg.is_empty()) {
            (true, _, _) => format!(" /{}▏  author: label: base: branch: is:draft review: ci:  -x negates", self.query),
            (_, _, false) => format!(" {}", self.msg),
            (_, false, _) => format!(" filter: {}  (Esc clears)", self.query),
            _ => String::new(), // the keys are in the bottom-right popup
        };
        let flags = if self.flags.is_empty() { String::new() } else { format!("opens with {}  ", self.flags) };
        f.render_widget(Paragraph::new(left).style(bar), status);
        f.render_widget(Paragraph::new(flags).alignment(Alignment::Right).style(bar), status);
        if !self.help && !self.filtering {
            // The keys for this tab, bottom right, as dv's diff view shows them.
            let enter = match self.tab {
                Tab::Todo => "see to it (opens the PR or replacement)".to_owned(),
                Tab::Worktrees => "review, in its worktree".to_owned(),
                Tab::Replacements => "review it, in its worktree".to_owned(),
                Tab::Open if self.flags.is_empty() => "review".to_owned(),
                Tab::Open => format!("review (dv N {})", self.flags),
            };
            let tab = match self.tab {
                Tab::Todo => "open PRs",
                Tab::Open => "PRs with a worktree",
                Tab::Worktrees => "replacements",
                Tab::Replacements => "what needs you",
            };
            let remove = if self.tab == Tab::Replacements { "remove replacement" } else { "remove worktree" };
            let keys = [
                ("Enter", enter.as_str()),
                ("Tab", tab),
                ("/", "filter"),
                ("o O", "sort / reverse"),
                ("R", "refresh (from GitHub)"),
                ("r r", "Claude ranks what needs you (a run)"),
                ("s", "defer it (or bring it back)"),
                ("p", "post what it waits on (a comment)"),
                ("m", "mark"),
                ("a a", if self.tab == Tab::Todo { "Claude does its part (marked)" } else { "Claude reviews marked" }),
                ("A A", "Claude drafts a replacement"),
                ("M M", "merge (github-merge.py)"),
                ("W W", "approve its workflow runs"),
                ("U U", "unmark all"),
                ("N N", "Claude drafts a new PR (unpublished)"),
                ("C", "auto on / off (CI, reviews, ranking)"),
                ("b b", "ask for a rebase (a comment)"),
                ("D D", remove),
                ("?", "all keys"),
                ("q", "quit"),
            ];
            let reps = self.tab == Tab::Replacements;
            let lines: Vec<Line> = keys
                .iter()
                .filter(|(k, _)| match *k {
                    "D D" => !matches!(self.tab, Tab::Open | Tab::Todo),
                    "o O" => self.tab != Tab::Todo, // in the order of what needs you
                    "r r" | "s" | "p" => self.tab == Tab::Todo,
                    "m" | "a a" | "A A" | "M M" | "b b" | "W W" | "U U" => !reps,
                    _ => true,
                })
                .map(|(k, d)| Line::from(vec![Span::styled(format!("{k:<6}"), ui.key), Span::raw(d.to_string())]))
                .collect();
            let w = (lines.iter().map(Line::width).max().unwrap_or(10) as u16 + 4).min(area.width);
            let h = (lines.len() as u16 + 2).min(area.height.saturating_sub(1));
            let r = Rect { x: area.width - w, y: area.height.saturating_sub(1 + h), width: w, height: h };
            f.render_widget(Clear, r);
            f.render_widget(Block::new().style(ui.base.patch(ui.popup)), r);
            f.render_widget(Paragraph::new(" keys").style(ui.base.patch(ui.header)), Rect { height: 1, ..r });
            f.render_widget(Paragraph::new(lines), Rect { x: r.x + 1, y: r.y + 1, width: w.saturating_sub(2), height: h.saturating_sub(1) });
        }
        if self.help {
            // Like dv's popups: no border, the popup background, a header row.
            let lines: Vec<&str> = HELP.lines().collect();
            let w = (lines.iter().map(|l| l.chars().count()).max().unwrap_or(0) as u16 + 4).min(area.width);
            let h = (lines.len() as u16 + 3).min(area.height);
            let r = Rect { x: (area.width - w) / 2, y: (area.height - h) / 2, width: w, height: h };
            f.render_widget(Clear, r);
            f.render_widget(Block::new().style(ui.base.patch(ui.popup)), r);
            let hs = ui.base.patch(ui.header);
            f.render_widget(Paragraph::new(" dv list keys").style(hs), Rect { height: 1, ..r });
            f.render_widget(Paragraph::new("any key closes ").alignment(Alignment::Right).style(hs), Rect { height: 1, ..r });
            let body: Vec<Line> = lines
                .iter()
                .map(|l| {
                    let (k, d) = l.split_at(l.char_indices().nth(23).map_or(l.len(), |(i, _)| i));
                    Line::from(vec![Span::styled(k.to_owned(), ui.key), Span::raw(d.to_owned())])
                })
                .collect();
            f.render_widget(Paragraph::new(body), Rect { x: r.x + 1, y: r.y + 2, width: r.width.saturating_sub(2), height: h.saturating_sub(3) });
        }
        self.tab_hits = hits; // where the tabs are, for clicks
    }
}

enum Do {
    Quit,
    /// `N N`: Claude drafts a new PR.
    NewPr,
    Open(u64),
    Refresh,
    /// Remove this PR's worktree.
    Remove(u64),
    /// Have Claude review these PRs, in the background.
    Review(Vec<u64>),
    /// Have Claude draft one replacement for these PRs, in the background.
    Replace(Vec<u64>),
    /// Remove this replacement (its worktree and record).
    RemoveReplacement(u64),
    /// Merge this PR with github-merge.py.
    Merge(u64),
    /// Ask this PR's author for a rebase.
    AskRebase(u64),
    /// Defer this todo item (its key) until something happens, written in your editor.
    DeferUntil(String),
    /// Post on this PR what it's deferred until, for everyone (and their dv) to see.
    PostWait(u64, String),
}

fn marks_path() -> Option<PathBuf> {
    crate::claude::dv_dir().ok().map(|d| d.join("marked.json"))
}

fn load_marks() -> HashSet<u64> {
    marks_path().map(|p| read_marks(&p)).unwrap_or_default()
}

/// Save the change from `was` to `marked` onto the marks saved now, and return those.
fn save_marks(was: &HashSet<u64>, marked: &HashSet<u64>) -> Result<HashSet<u64>> {
    merge_marks(&marks_path().context("no dv directory for this repo")?, was, marked)
}

fn merge_marks(p: &std::path::Path, was: &HashSet<u64>, marked: &HashSet<u64>) -> Result<HashSet<u64>> {
    let mut now = read_marks(p);
    now.retain(|n| marked.contains(n) || !was.contains(n));
    now.extend(marked.difference(was));
    write_marks(p, &now)?;
    Ok(now)
}

fn read_marks(p: &std::path::Path) -> HashSet<u64> {
    let text = std::fs::read_to_string(p).unwrap_or_default();
    serde_json::from_str::<Vec<u64>>(&text).unwrap_or_default().into_iter().collect()
}

fn write_marks(p: &std::path::Path, marked: &HashSet<u64>) -> Result<()> {
    let mut v: Vec<u64> = marked.iter().copied().collect();
    v.sort();
    std::fs::create_dir_all(p.parent().unwrap_or(p))?;
    // In one step (temp file, renamed over), so another dv list never reads half of it.
    let tmp = p.with_extension(format!("json.{}.tmp", std::process::id()));
    std::fs::write(&tmp, serde_json::to_string(&v)?)?;
    Ok(std::fs::rename(&tmp, p)?)
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Tab {
    /// What needs you, ranked.
    Todo,
    Open,
    Worktrees,
    Replacements,
}

impl Tab {
    fn next(self) -> Tab {
        match self {
            Tab::Todo => Tab::Open,
            Tab::Open => Tab::Worktrees,
            Tab::Worktrees => Tab::Replacements,
            Tab::Replacements => Tab::Todo,
        }
    }

    fn prev(self) -> Tab {
        self.next().next().next()
    }
}

#[derive(Clone, Copy)]
enum Order {
    Updated,
    Created,
    Number,
    Size,
}

impl Order {
    fn next(self) -> Order {
        match self {
            Order::Updated => Order::Created,
            Order::Created => Order::Number,
            Order::Number => Order::Size,
            Order::Size => Order::Updated,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Order::Updated => "updated",
            Order::Created => "created",
            Order::Number => "number",
            Order::Size => "size",
        }
    }
}

/// One filter term, GitHub-search style: `author:` `label:` `base:` `branch:` (substring),
/// `is:draft`, `review:approved|changes|required|none`, `ci:pass|fail|pending|none`, else text
/// in the number, title, author, branch or labels. `-` negates. Smart case.
fn term(p: &Listed, t: &str) -> bool {
    let (neg, t) = match t.strip_prefix('-') {
        Some(rest) if !rest.is_empty() => (true, rest),
        _ => (false, t),
    };
    let fold = |s: &str| if t.chars().any(char::is_uppercase) { s.to_owned() } else { s.to_lowercase() };
    let has = |field: &str, v: &str| fold(field).contains(v);
    let labels = || p.labels.iter().map(|l| l.0.as_str()).collect::<Vec<_>>().join(" ");
    let hit = match t.split_once(':') {
        Some(("author", v)) => has(&p.author, v),
        Some(("label", v)) => p.labels.iter().any(|l| has(&l.0, v)),
        Some(("base", v)) => has(&p.base, v),
        Some(("branch", v)) => has(&p.branch, v),
        Some(("is", "draft")) => p.draft,
        Some(("review", v)) => match p.decision.as_str() {
            "APPROVED" => "approved".starts_with(v),
            "CHANGES_REQUESTED" => "changes".starts_with(v),
            "REVIEW_REQUIRED" => "required".starts_with(v),
            _ => v == "none",
        },
        // Unknown until the checks arrive; then a PR can match several (fail and pending).
        Some(("ci", v)) => match p.checks {
            Some((0, 0, 0)) | None => v == "none",
            Some((pass, fail, pending)) => {
                ("pass".starts_with(v) && pass > 0 && fail == 0 && pending == 0)
                    || ("fail".starts_with(v) && fail > 0)
                    || ("pending".starts_with(v) && pending > 0)
            }
        },
        _ => has(&format!("#{} {} @{} {} {}", p.number, p.title, p.author, p.branch, labels()), t),
    };
    hit != neg
}
