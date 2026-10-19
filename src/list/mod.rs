//! `dv list`: what needs you (ranked, `todo.rs`), the repo's open PRs, (`gn` / `gp`) the ones with a
//! local worktree, and the replacements Claude has drafted (unpublished); `Enter` reviews one,
//! and quitting that review comes back here.
use crate::gh;
use crate::theme::{Theme, Ui};
use anyhow::{Context, Result};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Alignment, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::{DefaultTerminal, Frame};
use crate::batch;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

mod card;
use card::*;
mod todo;
pub(crate) mod github;
mod draw;
mod keys;
mod marks;
use {keys::*, marks::*};
pub(crate) use github::{Extra, Listed};
use crate::gh::ago_secs;
#[cfg(test)]
mod tests;

/// PRs looked up on GitHub, by number (`None`: not to be had).
type Lookups = Vec<(u64, Option<Listed>)>;
/// By PR, in its worktree: (changed files, commits beyond the PR's head, when that head was made).
type Unpushed = HashMap<u64, (usize, usize, Option<i64>)>;
type Checks = anyhow::Result<github::ListChecks>;
/// The open PRs, as fetched (in the background).
type Fetched = Result<Vec<Listed>>;

/// You, and what GitHub says of each PR beyond its card (`github::list_extra`).
type Extras = anyhow::Result<(String, HashMap<u64, Extra>)>;

#[derive(Default)]
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
    extra: HashMap<u64, Extra>,
    extra_rx: Option<Receiver<Extras>>,
    /// The selected PR, looked up on GitHub in the background for Enter (`prefetch`).
    pr_cache: PrCache,
    /// The PR selected, since when, and whether it's been looked up.
    prefetched: (Option<u64>, Option<std::time::Instant>, bool),
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
    running_checked: Option<std::time::Instant>,
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
    /// The cards' width when the todo tab's messages go in a column beside them (`msg_column`).
    msg_col: Option<usize>,
    /// PRs Claude has a review of, and when its reviews directory last changed: a review
    /// written from a PR's view (or still being, in Claude's pane) shows up as it lands.
    reviewed: HashSet<u64>,
    reviews_dir: Option<std::path::PathBuf>,
    reviews_mtime: Option<std::time::SystemTime>,
    /// PRs with something you haven't seen (`fresh`).
    new: HashSet<u64>,
    /// PRs Claude's review would close, and why: the "can close" tab.
    closable: HashMap<u64, String>,
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
        List {
            ui: Theme::load().ui(),
            flags,
            ranked: crate::todo::load(),
            auto: crate::diff::git(&["config", "--get", "dv.auto"]).is_ok_and(|v| v.trim() == "true"),
            deferred: crate::todo::load_deferred(crate::todo::deferred_path().as_deref()),
            deferred_file: crate::todo::deferred_path(),
            wide: true,
            msg_col: None,
            reviews_dir: crate::claude::dv_dir().ok().map(|d| d.join("reviews")),
            root: std::env::current_dir().unwrap_or_default(),
            ..Default::default()
        }
    }

    /// Show the list until a PR is picked (`Some`) or the user quits (`None`).
    pub fn run(&mut self, term: &mut DefaultTerminal) -> Result<Option<Pick>> {
        // As saved: another dv list of this repo may have changed them meanwhile.
        self.marked = load_marks();
        self.prefetched = (None, None, true); // what's selected is looked up again
        self.marked_saved = self.marked.clone();
        // From GitHub again in the background while it's open (and now, back from a PR: you may
        // have changed it).
        match &self.fresh {
            None => self.watch_github(),
            Some((tx, _)) => {
                let tx = tx.clone();
                std::thread::spawn(move || _ = tx.send(github::list()));
            }
        }
        if self.prs.is_empty() {
            self.refresh(term)?;
            if !self.marked.is_empty() && self.msg.is_empty() {
                self.msg = format!("{} marked from before: a a or A A acts on them; m unmarks", self.marked.len());
            }
        }
        // (The cursor on the card it was on, as through what comes in below.)
        let keep = self.selected();
        self.find_new(); // back from a PR: that one's seen now
        self.load_reps();
        self.scan_unpushed(); // and what you (or Claude) left there unpushed
        self.find_todo(); // and what you did there may have seen to it
        self.select_card(keep);
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
            // What comes in below may change the list under the cursor: it stays on its card
            // (`select_card`, after it all; your own keys, after, move it as they do).
            let mut keep = self.selected();
            // Your own deferral (Claude read when), first, while the cursor's still where it
            // was: on the card it moved, the cursor stays put, on what's next.
            if self.until_news() {
                keep = None;
            }
            self.batch_news();
            self.watch_reviews();
            self.todo_news();
            self.unpushed_news();
            self.gone_news();
            self.prefetch();
            self.others_news();
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
                    // (One GitHub said nothing of keeps what it had.)
                    Ok(mut by_pr) => self.prs.iter_mut().for_each(|p| {
                        if let Some((c, o, h)) = by_pr.remove(&p.number) {
                            (p.checks, p.optional_failing, p.checks_head) = (Some(c), o, h);
                        }
                    }),
                    Err(e) => self.msg = format!("checks: {e:#}"),
                }
                self.find_todo(); // green checks: ready to merge
            }
            self.select_card(keep);
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
                    let replacement = self.selected().is_some_and(|(m, rep)| m == n && rep);
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
        groups.retain(|g| !g.is_empty() && !self.rep_busy(g[0]));
        if groups.is_empty() {
            return self.msg = "nothing to draft".into();
        }
        let which: Vec<String> = groups.iter().map(|g| g.iter().map(|n| format!("#{n}")).collect::<Vec<_>>().join("+")).collect();
        let what = format!("instructions for Claude's replacement of {} (optional)", which.join(", "));
        // Drafted already: what else you want of it (asked of its drafts), not your instructions
        // for first drafts (nor saved as them); any first drafts among them get those.
        let extra = match groups.iter().any(|g| crate::replace::drafted(g[0])) {
            true => self.edit_text(term, &format!("what else Claude should do in its redraft of {} (optional; first drafts get your instructions for replacements)", which.join(", ")), "", true),
            false => self.instructions(term, "replace", &what),
        };
        let Some(extra) = extra else { return };
        self.msg = format!("Claude is drafting {} replacement(s): the replacements tab shows them", groups.len());
        self.marked.clear();
        for g in groups {
            // (A first draft with nothing given takes your saved instructions: `replace::draft`.)
            let extra = if crate::replace::drafted(g[0]) { extra.clone() } else { String::new() };
            self.draft_rx.push(batch::replace(g, self.root.clone(), extra));
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

    /// A key that asks to be pressed again (what it'd do, `ask`, said): whether it was (`armed`
    /// is what the last key armed), else it's armed now.
    fn again(&mut self, armed: Option<(char, u64)>, key: (char, u64), ask: impl Into<String>) -> bool {
        if armed == Some(key) {
            return true;
        }
        (self.msg, self.confirm) = (ask.into(), Some(key));
        false
    }

    /// The selected card's number, and whether it's a replacement's draft.
    fn selected(&self) -> Option<(u64, bool)> {
        self.card_key(self.sel)
    }

    /// Card `i`'s (as shown): its number, and whether it's a replacement's draft.
    fn card_key(&self, i: usize) -> Option<(u64, bool)> {
        self.shown_cards().get(i).map(|c| self.key_of(c))
    }

    fn key_of(&self, (card, it): &(&Listed, Option<&crate::todo::Item>)) -> (u64, bool) {
        (card.number, self.tab == Tab::Replacements || it.is_some_and(|it| it.rep))
    }

    /// `f`, the cursor kept on the card it was on, wherever it is after (as the event loop does
    /// for what comes in: a sort, here). Gone: it stays where it was.
    pub(super) fn keeping_sel(&mut self, f: impl FnOnce(&mut Self)) {
        let keep = self.selected();
        f(self);
        self.select_card(keep);
    }

    /// The cursor on card `k` (`selected`'s), where it's shown now; not shown, left as it is.
    pub(super) fn select_card(&mut self, k: Option<(u64, bool)>) {
        let Some(k) = k else { return };
        if let Some(i) = self.shown_cards().iter().position(|c| self.key_of(c) == k) {
            self.sel = i;
        }
    }

    /// The selected todo item (the todo tab's, as shown: filtered).
    pub(super) fn selected_item(&self) -> Option<crate::todo::Item> {
        self.shown_cards().get(self.sel).and_then(|(_, it)| it.cloned())
    }

    /// The selected card's PR, for what's done to a PR on GitHub (not a draft replacement's:
    /// said so).
    fn selected_pr(&mut self) -> Option<u64> {
        match self.selected()? {
            (_, true) => {
                self.msg = "a draft replacement, not on GitHub: Enter opens it (its own keys are on the drafts tab)".into();
                None
            }
            (n, false) => Some(n),
        }
    }

    /// The tab's PRs matching every filter term, in the chosen order.
    fn shown(&self) -> Vec<&Listed> {
        self.shown_cards().into_iter().map(|(card, _)| card).collect()
    }

    /// The cards shown, each with its todo item on the todo tab (which says whether it's a
    /// replacement's draft, whose number is the PR it replaces': not that PR, for what's done).
    fn shown_cards(&self) -> Vec<(&Listed, Option<&crate::todo::Item>)> {
        let in_tab = |p: &&Listed| match self.tab {
            Tab::Worktrees => self.local.contains_key(&p.number),
            Tab::CanClose => p.open() && self.closable.contains_key(&p.number),
            _ => p.open(),
        };
        let all: Vec<(&Listed, Option<&crate::todo::Item>)> = match self.tab {
            Tab::Replacements => self.reps.iter().map(|r| (r, None)).collect(),
            Tab::Todo => self.todo_shown().into_iter().map(|(card, it)| (card, Some(it))).collect(),
            _ => self.prs.iter().filter(in_tab).map(|p| (p, None)).collect(),
        };
        let mut prs: Vec<(&Listed, Option<&crate::todo::Item>)> = all.into_iter().filter(|p| self.query.split_whitespace().all(|t| term(p.0, t))).collect();
        if self.tab == Tab::Todo {
            return prs; // in the order of what needs you most
        }
        prs.sort_by(|(a, _), (b, _)| {
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
        self.keeping_sel(|l| (l.order, l.rev) = (order, rev));
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

#[derive(Clone, Copy, Debug, PartialEq, Default)]
enum Tab {
    /// What needs you, ranked.
    Todo,
    #[default]
    Open,
    /// What Claude's reviews would close: the premise is wrong, a competing PR is better.
    CanClose,
    Worktrees,
    Replacements,
}

impl Tab {
    fn next(self) -> Tab {
        match self {
            Tab::Todo => Tab::Open,
            Tab::Open => Tab::CanClose,
            Tab::CanClose => Tab::Worktrees,
            Tab::Worktrees => Tab::Replacements,
            Tab::Replacements => Tab::Todo,
        }
    }

    fn prev(self) -> Tab {
        self.next().next().next().next()
    }
}

#[derive(Clone, Copy, Default)]
enum Order {
    Updated,
    #[default]
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
