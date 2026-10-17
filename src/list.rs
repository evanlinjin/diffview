//! `dv list`: the repo's open PRs, (`Tab`) the ones with a local worktree, and the replacements
//! Claude has drafted (unpublished); `Enter` reviews one, and quitting that review comes back
//! here.
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

type Checks = anyhow::Result<HashMap<u64, (usize, usize, usize)>>;

const HELP: &str = "\
  j k, Up Down         move
  gg ge                first / last PR
  Ctrl-d Ctrl-u        half a page down / up
  Tab                  open PRs / PRs with a local worktree (<repo>-N, as dv N -w makes) /
                       replacements (Claude's drafts, unpublished)
  Enter                review the PR (with dv list's flags; from the worktrees tab, in its
                       worktree as it was made); q there comes back here
  /                    filter (Enter keeps it, Esc clears it); terms must all match:
                         text            number, title, author, branch, labels
                         author: label: base: branch:   part of that field
                         is:draft  review:approved|changes|required|none
                         ci:pass|fail|pending|none     -term excludes
  Esc                  clear the filter
  o O                  next sort order (updated, created, number, size) / reverse
  m                    mark the PR (again: unmark), for Claude to review; marks are kept, for
                       going through them a PR at a time (quitting dv list too)
  a a                  Claude reviews the marked PRs (or this one) in the background, two at a
                       time, each in its worktree (<repo>-N, made if need be); cards show how
                       it's going, and claude ✓ once a PR has a review
  A A                  Claude drafts a replacement for this PR, on a branch of its own in
                       <repo>-replace-N, unpublished, for you to review first (replacements
                       tab); for marked PRs, it first suggests which go together (one
                       replacement each group), which you edit; then your instructions
  D D                  worktrees tab: remove the PR's worktree (not with local changes in it;
                       its branch stays); replacements tab: remove the replacement (the same)
  R                    refresh
  ?                    this help
  q, Ctrl-c            quit";

pub struct List {
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
    batch_rx: Option<Receiver<(u64, batch::Status)>>,
    /// PRs Claude has a review of.
    reviewed: HashSet<u64>,
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
}

impl List {
    pub fn new(flags: String) -> List {
        let ui = Theme::load().ui();
        let (query, msg) = (String::new(), String::new());
        List {
            prs: vec![],
            ui,
            flags,
            sel: 0,
            top: 0,
            query,
            filtering: false,
            pending_g: false,
            help: false,
            order: Order::Updated,
            rev: false,
            checks: None,
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
            batch_rx: None,
            reviewed: HashSet::new(),
            new: HashSet::new(),
            root: std::env::current_dir().unwrap_or_default(),
            msg,
        }
    }

    /// Show the list until a PR is picked (`Some`) or the user quits (`None`).
    pub fn run(&mut self, term: &mut DefaultTerminal) -> Result<Option<Pick>> {
        // As saved: another dv list of this repo may have changed them meanwhile.
        self.marked = load_marks();
        self.marked_saved = self.marked.clone();
        if self.prs.is_empty() {
            self.refresh(term)?;
            if !self.marked.is_empty() && self.msg.is_empty() {
                self.msg = format!("{} marked from before: a a or A A acts on them; m unmarks", self.marked.len());
            }
        }
        self.find_new(); // back from a PR: that one's seen now
        self.load_reps();
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
            // A replacement drafted, or its draft changed, from anywhere: show it.
            let changed = crate::replace::changed();
            if changed != self.reps_changed {
                self.reps_changed = changed;
                self.load_reps();
            }
            if let Some(Ok(res)) = self.group_rx.as_ref().map(Receiver::try_recv) {
                self.group_rx = None;
                self.draft_groups(term, res);
            }
            if let Some(Ok(res)) = self.checks.as_ref().map(Receiver::try_recv) {
                self.checks = None;
                match res {
                    Ok(by_pr) => self.prs.iter_mut().for_each(|p| p.checks = by_pr.get(&p.number).copied()),
                    Err(e) => self.msg = format!("checks: {e:#}"),
                }
            }
            if !event::poll(Duration::from_millis(100))? {
                continue;
            }
            let k = match event::read()? {
                Event::Key(k) if k.kind == KeyEventKind::Press => k,
                Event::Mouse(m) => match self.mouse(m) {
                    // A click on the selected card opens it, as Enter.
                    true => KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                    false => continue,
                },
                _ => continue,
            };
            match self.key(k) {
                Some(Do::Quit) => return Ok(None),
                Some(Do::Open(n)) => {
                    let replacement = self.tab == Tab::Replacements;
                    let worktree = (self.tab == Tab::Worktrees).then(|| self.local.get(&n).is_some_and(|w| w.1.is_some()));
                    return Ok(Some(Pick { number: n, worktree, replacement }));
                }
                Some(Do::Refresh) => self.refresh(term)?,
                Some(Do::Review(prs)) => {
                    // Instructions for every one of these reviews, if you have any. In zellij the
                    // editor gets a pane beside the list; elsewhere it takes the terminal.
                    let which = prs.iter().map(|n| format!("#{n}")).collect::<Vec<_>>().join(" ");
                    let what = format!("instructions for Claude's reviews of {which} (optional)");
                    let Some(extra) = self.instructions(term, "review", &what) else { continue };
                    let on_branch = |n| self.local.get(&n).is_some_and(|w| w.1.is_some());
                    let prs: Vec<(u64, bool)> = prs.into_iter().map(|n| (n, on_branch(n))).collect();
                    let with = if extra.is_empty() { "" } else { ", with your instructions" };
                    self.msg = format!("Claude is reviewing {} PR(s){with}, {} at a time", prs.len(), batch::AT_ONCE);
                    self.marked.clear();
                    self.batch_rx = Some(batch::start(prs, self.root.clone(), extra));
                }
                // One PR: its replacement. Several: Claude suggests how to group them first.
                Some(Do::Replace(prs)) if prs.len() == 1 => self.draft_groups(term, Ok(vec![(prs, String::new())])),
                Some(Do::Replace(prs)) => {
                    let about = |n: u64| self.prs.iter().find(|p| p.number == n).map(|p| (n, p.title.clone(), p.author.clone(), p.base.clone()));
                    let prs = prs.into_iter().map(|n| about(n).unwrap_or((n, String::new(), String::new(), String::new()))).collect();
                    self.msg = "Claude is looking at how to group them (one replacement per group)…".into();
                    self.group_rx = Some(batch::group(prs, self.root.clone()));
                }
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
                            self.prs.retain(|p| p.number != n || p.state == "OPEN" || p.state.is_empty());
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
        let in_pane = crate::view::in_zellij();
        if !in_pane {
            crate::view::restore_terminal();
        }
        let last = crate::claude::instructions(kind);
        let extra = crate::view::compose_with(what, true, last.trim(), true);
        if !in_pane {
            *term = crate::view::init_terminal();
        }
        match extra {
            Ok(Some(e)) => {
                _ = crate::claude::save_instructions(kind, &e);
                Some(e)
            }
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
                batch::Status::Failed(e) => self.msg = format!("Claude's replacement of #{id} failed: {e} (A A tries again)"),
                _ => {}
            }
            let done = !s.running();
            self.drafting.insert(id, s);
            if done || !self.rep_of.contains_key(&id) {
                self.load_reps(); // its record exists from Preparing on; its drafts once done
            }
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
        let Some(rx) = &self.batch_rx else { return };
        let news: Vec<(u64, batch::Status)> = rx.try_iter().collect();
        let mut worktrees = false;
        for (n, s) in news {
            match &s {
                batch::Status::Done(took) => {
                    self.reviewed.insert(n);
                    self.new.insert(n);
                    self.msg = format!("Claude reviewed #{n} ({took}): Enter opens it, in its worktree from the worktrees tab");
                }
                batch::Status::Failed(e) => self.msg = format!("Claude's review of #{n} failed: {e} (a a asks again)"),
                batch::Status::Reviewing => worktrees = true, // its worktree is ready
                _ => {}
            }
            self.batch.insert(n, s);
        }
        if worktrees {
            self.local = crate::pr_worktrees().into_iter().map(|(n, dir, b)| (n, (dir, b))).collect();
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

    fn refresh(&mut self, term: &mut DefaultTerminal) -> Result<()> {
        self.msg = "loading PRs…".into();
        term.draw(|f| self.draw(f))?;
        let keep = self.shown().get(self.sel).map(|p| p.number);
        self.local = crate::pr_worktrees().into_iter().map(|(n, dir, b)| (n, (dir, b))).collect();
        self.reviewed = crate::batch::reviewed();
        self.load_reps();
        match gh::list() {
            Ok(mut prs) => {
                let (tx, rx) = mpsc::channel();
                std::thread::spawn(move || tx.send(gh::list_checks()));
                self.checks = Some(rx);
                self.msg = if prs.is_empty() { "no open PRs".into() } else { String::new() };
                // Worktrees of PRs that aren't open any more (or beyond the list): fetch those few.
                let mut gone: Vec<u64> = self.local.keys().filter(|n| !prs.iter().any(|p| p.number == **n)).copied().collect();
                gone.sort();
                for n in gone {
                    let title = "(couldn't load this PR)".into();
                    prs.push(gh::listed_one(n).unwrap_or(Listed { number: n, title, ..Listed::default() }));
                }
                self.prs = prs;
                // Marks on PRs that aren't open any more go.
                // Ones not listed (closed, or beyond the list) are looked up: only a few are marked.
                let open = |p: &Listed| p.state == "OPEN" || p.state.is_empty();
                self.marked.retain(|n| match self.prs.iter().find(|p| p.number == *n) {
                    Some(p) => open(p),
                    None => gh::listed_one(*n).map_or(true, |p| open(&p)),
                });
                self.find_new();
                self.sel = keep.and_then(|n| self.shown().iter().position(|p| p.number == n)).unwrap_or(0);
            }
            Err(e) => self.msg = format!("{e:#}"),
        }
        Ok(())
    }

    /// The tab's PRs matching every filter term, in the chosen order.
    fn shown(&self) -> Vec<&Listed> {
        let in_tab = |p: &&Listed| match self.tab {
            Tab::Worktrees => self.local.contains_key(&p.number),
            _ => p.state == "OPEN" || p.state.is_empty(),
        };
        let all: Vec<&Listed> = match self.tab {
            Tab::Replacements => self.reps.iter().collect(),
            _ => self.prs.iter().filter(in_tab).collect(),
        };
        let mut prs: Vec<&Listed> = all.into_iter().filter(|p| self.query.split_whitespace().all(|t| term(p, t))).collect();
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
                self.msg = format!("D again removes the replacement of #{id} (its worktree and record; its branch stays); any other key keeps it");
                self.confirm = Some(('D', id));
            }
            KeyCode::Char('m' | 'a' | 'A') if self.tab == Tab::Replacements => {
                self.msg = "a replacement: Enter opens it in dv, D D removes it".into();
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
                let which = prs.iter().map(|n| format!("#{n}")).collect::<Vec<_>>().join(" ");
                self.msg = format!("a again: Claude reviews {which} in the background, each in its worktree (<repo>-N); your editor opens first, for any instructions");
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
                    return (self.msg = format!("Claude is already drafting the replacement of #{id}"), None).1;
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
            KeyCode::Tab => (self.tab, self.sel, self.top) = (self.tab.next(), 0, 0),
            KeyCode::BackTab => (self.tab, self.sel, self.top) = (self.tab.next().next(), 0, 0),
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
    fn mouse(&mut self, m: event::MouseEvent) -> bool {
        use event::{MouseButton, MouseEventKind};
        let n = self.shown().len();
        match m.kind {
            MouseEventKind::ScrollDown => self.sel = (self.sel + 1).min(n.saturating_sub(1)),
            MouseEventKind::ScrollUp => self.sel = self.sel.saturating_sub(1),
            MouseEventKind::Down(MouseButton::Left) if !self.help && m.row >= 1 => {
                let i = self.top + (m.row as usize - 1) / 2; // cards start under the header, 2 rows each
                if i < n {
                    if i == self.sel {
                        return true;
                    }
                    self.sel = i;
                }
            }
            _ => {}
        }
        false
    }

    /// Cards on screen (2 rows each, between the header and the status bar).
    fn height(&self) -> usize {
        ratatui::crossterm::terminal::size().map_or(10, |(_, h)| h.saturating_sub(2) as usize / 2)
    }

    fn draw(&mut self, f: &mut Frame) {
        let area = f.area();
        let h = area.height.saturating_sub(2) as usize / 2;
        let sel = self.sel.min(self.shown().len().saturating_sub(1));
        // Keep the selection on screen.
        (self.sel, self.top) = (sel, self.top.min(sel).max((sel + 1).saturating_sub(h)));
        let (ui, shown) = (&self.ui, self.shown());
        f.render_widget(Paragraph::new("").style(ui.base), area);

        let head = Rect { height: 1.min(area.height), ..area };
        let hs = ui.base.patch(ui.header);
        let arrow = if self.rev { '↑' } else { '↓' };
        // Tabs: the current one bold, with counts.
        let open = self.prs.iter().filter(|p| p.state == "OPEN" || p.state.is_empty()).count();
        let tab = |name: String, on: bool| Span::styled(name, if on { hs.add_modifier(ratatui::style::Modifier::BOLD | ratatui::style::Modifier::UNDERLINED) } else { hs.patch(ui.stale) });
        let tabs = Line::from(vec![
            Span::styled(" ", hs),
            tab(format!("open {open}{}", if self.prs.len() >= gh::LIST_MAX { "+" } else { "" }), self.tab == Tab::Open),
            Span::styled("  ", hs),
            tab(format!("worktrees {}", self.local.len()), self.tab == Tab::Worktrees),
            Span::styled("  ", hs),
            tab(format!("replacements {}", self.reps.len()), self.tab == Tab::Replacements),
            Span::styled(format!("   by {} {arrow}  (Tab switches)", self.order.name()), hs.patch(ui.stale)),
        ]);
        f.render_widget(Paragraph::new(tabs).style(hs), head);
        let pos = if shown.is_empty() { "0 / 0 ".into() } else { format!("{} / {} ", sel + 1, shown.len()) };
        f.render_widget(Paragraph::new(pos).alignment(Alignment::Right).style(hs), head);

        // The base most PRs target goes without saying; others show as "base ← branch".
        let mut bases: HashMap<&str, usize> = HashMap::new();
        self.prs.iter().for_each(|p| *bases.entry(p.base.as_str()).or_default() += 1);
        let usual = bases.into_iter().max_by_key(|b| b.1).map(|b| b.0.to_owned()).unwrap_or_default();
        let cx = Cards {
            ui,
            num_w: shown.iter().map(|p| p.number.to_string().len()).max().unwrap_or(1) + 1,
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
                let mark = match self.tab {
                    Tab::Replacements => {
                        let replaces = self.rep_of.get(&p.number).map(|r| {
                            let each: Vec<String> = r.replaces.iter().map(|o| format!("#{} (@{})", o.number, o.author)).collect();
                            format!("replaces {}", each.join(", "))
                        });
                        Mark { marked: false, claude: self.draft_state(p.number, "claude"), new: false, replaces }
                    }
                    _ => Mark { marked: self.marked.contains(&p.number), claude: self.claude_state(p.number), new: self.new.contains(&p.number), replaces: None },
                };
                let local = if self.tab == Tab::Replacements { None } else { self.local.get(&p.number) };
                card(p, &cx, i == sel, local, mark)
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
        f.render_widget(Paragraph::new(lines), Rect { y: area.y + 1, height: (h * 2) as u16, ..area });

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
                Tab::Worktrees => "review, in its worktree".to_owned(),
                Tab::Replacements => "review it, in its worktree".to_owned(),
                Tab::Open if self.flags.is_empty() => "review".to_owned(),
                Tab::Open => format!("review (dv N {})", self.flags),
            };
            let tab = match self.tab {
                Tab::Open => "PRs with a worktree",
                Tab::Worktrees => "replacements",
                Tab::Replacements => "open PRs",
            };
            let remove = if self.tab == Tab::Replacements { "remove replacement" } else { "remove worktree" };
            let keys = [
                ("Enter", enter.as_str()),
                ("Tab", tab),
                ("/", "filter"),
                ("o O", "sort / reverse"),
                ("R", "refresh"),
                ("m", "mark"),
                ("a a", "Claude reviews marked"),
                ("A A", "Claude drafts a replacement"),
                ("D D", remove),
                ("?", "all keys"),
                ("q", "quit"),
            ];
            let reps = self.tab == Tab::Replacements;
            let lines: Vec<Line> = keys
                .iter()
                .filter(|(k, _)| match *k {
                    "D D" => self.tab != Tab::Open,
                    "m" | "a a" | "A A" => !reps,
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
    }
}

enum Do {
    Quit,
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
    Open,
    Worktrees,
    Replacements,
}

impl Tab {
    fn next(self) -> Tab {
        match self {
            Tab::Open => Tab::Worktrees,
            Tab::Worktrees => Tab::Replacements,
            Tab::Replacements => Tab::Open,
        }
    }
}

/// A replacement's card: its drafted title, its branch, and its size against its base (from
/// git: nothing's on GitHub yet).
fn rep_card(r: &crate::replace::Record, root: &std::path::Path) -> Listed {
    let branch = crate::replace::branch(r.id);
    let git = |args: &[&str]| crate::diff::git(&[&["-C", &root.to_string_lossy()], args].concat()).unwrap_or_default();
    // " 3 files changed, 10 insertions(+), 2 deletions(-)"
    let stat = git(&["diff", "--shortstat", &format!("origin/{}...{branch}", r.base)]);
    let count = |what: &str| -> u64 {
        stat.split(',').find(|p| p.contains(what)).and_then(|p| p.split_whitespace().next()?.parse().ok()).unwrap_or(0)
    };
    let title = match r.title.is_empty() {
        true => format!("(replacement of #{}, not drafted yet)", r.id),
        false => r.title.clone(),
    };
    Listed {
        number: r.id,
        title,
        base: r.base.clone(),
        branch,
        state: "OPEN".into(),
        updated: git(&["log", "-1", "--format=%cI", &crate::replace::branch(r.id)]).trim().to_owned(),
        files: count("file"),
        additions: count("insertion"),
        deletions: count("deletion"),
        ..Listed::default()
    }
}

/// A card's own marks: picked for review (`m`), and how Claude's review of it stands.
struct Mark {
    /// Something on it you haven't seen.
    new: bool,
    /// A replacement's card: what it replaces (said instead of an author).
    replaces: Option<String>,
    marked: bool,
    claude: Option<(String, bool)>,
}

impl List {
    /// What a card says about Claude's review: ("claude ✓", good) or ("claude: reviewing…", …).
    fn claude_state(&self, n: u64) -> Option<(String, bool)> {
        // A PR being replaced says so too.
        let id = self.rep_of.values().find(|r| r.replaces.iter().any(|o| o.number == n)).map(|r| r.id);
        let replaced = id.or(self.drafting.contains_key(&n).then_some(n)).and_then(|id| self.draft_state(id, "replacement"));
        match (self.review_state(n), replaced) {
            (Some((a, x)), Some((b, y))) => Some((format!("{a} · {b}"), x && y)),
            (a, b) => a.or(b),
        }
    }

    fn review_state(&self, n: u64) -> Option<(String, bool)> {
        match self.batch.get(&n) {
            Some(batch::Status::Queued) => Some(("claude: queued".into(), true)),
            Some(batch::Status::Preparing) => Some(("claude: checking out…".into(), true)),
            Some(batch::Status::Reviewing) => Some(("claude: reviewing…".into(), true)),
            // Why, in short (the status bar said it in full): `a a` asks again.
            Some(batch::Status::Failed(e)) => Some((format!("claude failed: {}", cut(e.lines().next().unwrap_or(""), 60)), false)),
            _ if self.reviewed.contains(&n) => Some(("claude ✓".into(), true)),
            _ => None,
        }
    }
}

/// What every card on screen shares.
struct Cards<'a> {
    ui: &'a Ui,
    num_w: usize,
    width: usize,
    checks_loading: bool,
    /// The base most PRs target, left unsaid.
    usual: &'a str,
}

/// A PR as two lines: number, title, state / draft badge and age; then who, CI, review, size,
/// labels, branches and its worktree, most useful first so a narrow screen cuts the least useful.
fn card(p: &Listed, cx: &Cards, selected: bool, local: Option<&(PathBuf, Option<String>)>, mark: Mark) -> [Line<'static>; 2] {
    let (ui, num_w, width, usual) = (cx.ui, cx.num_w, cx.width, cx.usual);
    let row = if selected { ui.base.patch(ui.menu_selected) } else { ui.base };
    // On the selected card the text takes the highlight's colors, to stay readable (labels
    // keep theirs).
    let tint = |st: Style| if selected { row } else { row.patch(st) };
    let dim = tint(ui.stale);
    let sp = |t: String, st| Span::styled(t, st);
    let age = format!("{} ", gh::ago(&p.updated));
    // Closed / merged (only in the worktrees tab), else draft, as GitHub colors them.
    let badge = match (p.state.as_str(), p.draft) {
        ("MERGED", _) => Some(("MERGED", "8957e5")),
        ("CLOSED", _) => Some(("CLOSED", "da3633")),
        (_, true) => Some(("DRAFT", "6e7681")),
        _ => None,
    };
    let new = if mark.new { "[new] " } else { "" };
    let room = width.saturating_sub(num_w + 3 + badge.map_or(0, |b| b.0.len() + 4) + new.len() + age.len()).max(10);
    let mut top = vec![
        sp(format!("{}{:>num_w$} ", if mark.marked { "●" } else { " " }, format!("#{}", p.number)), tint(ui.key)),
        sp(cut(&p.title, room), row),
    ];
    if let Some((text, color)) = badge {
        top.extend([sp("  ".into(), row), sp(format!(" {text} "), crate::view::chip(color))]);
    }
    // At the right end, before the age.
    let used: usize = top.iter().map(Span::width).sum();
    top.push(sp(" ".repeat(width.saturating_sub(used + new.len() + age.len())), row));
    top.extend([sp(new.into(), tint(ui.key).add_modifier(ratatui::style::Modifier::BOLD)), sp(age, dim)]);

    let sep = || sp(" · ".into(), dim);
    let who = mark.replaces.clone().unwrap_or_else(|| format!("@{}", p.author));
    let mut bottom = vec![sp(" ".repeat(num_w + 2), row), sp(who, dim)];
    match p.checks {
        Some((0, 0, 0)) => {}
        Some((pass, fail, pending)) => {
            let counts = [(pass, '✓', ui.plus), (fail, '✗', ui.error), (pending, '●', ui.warning)];
            for (k, &(n, mark, st)) in counts.iter().filter(|c| c.0 > 0).enumerate() {
                bottom.extend([sp(if k == 0 { " · " } else { " " }.into(), dim), sp(format!("{mark}{n}"), tint(st))]);
            }
        }
        None if cx.checks_loading && mark.replaces.is_none() => bottom.extend([sep(), sp("checks…".into(), dim)]),
        None => {}
    }
    match p.decision.as_str() {
        "APPROVED" => bottom.extend([sep(), sp("✓ approved".into(), tint(ui.plus))]),
        "CHANGES_REQUESTED" => bottom.extend([sep(), sp("✗ changes requested".into(), tint(ui.error))]),
        _ => {}
    }
    bottom.extend([
        sep(),
        sp(format!("+{}", p.additions), tint(ui.plus)),
        sp(" ".into(), row),
        sp(format!("−{}", p.deletions), tint(ui.minus)),
        sp(format!(" in {} file{}", p.files, if p.files == 1 { "" } else { "s" }), dim),
    ]);
    for (name, color) in &p.labels {
        bottom.extend([sp(" ".into(), row), sp(format!(" {name} "), crate::view::chip(color))]);
    }
    let branch = match p.base == usual {
        true => cut(&p.branch, 32),
        false => format!("{} ← {}", p.base, cut(&p.branch, 32)),
    };
    bottom.extend([sep(), sp(branch, dim)]);
    if let Some((dir, branch)) = local {
        let name = dir.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
        let on = branch.as_deref().map_or("detached".to_owned(), |b| format!("on {}", cut(b, 24)));
        bottom.extend([sep(), sp(format!("⌂ {name} ({on})"), tint(ui.key))]);
    }
    // Claude's part at the right end, where it stands out (under the age), the rest cut to
    // make room; the highlight runs to the edge.
    let claude = mark.claude.map(|(text, ok)| sp(format!("{text} "), tint(if ok { ui.key } else { ui.error })));
    let room = width.saturating_sub(claude.as_ref().map_or(0, |c| c.width() + 1));
    let mut used: usize = bottom.iter().map(Span::width).sum();
    while (used > room || bottom.last().is_some_and(|s| s.content == " · ")) && bottom.len() > 1 {
        used -= bottom.pop().map_or(0, |s| s.width());
    }
    bottom.push(sp(" ".repeat(width.saturating_sub(used + claude.as_ref().map_or(0, Span::width))), row));
    bottom.extend(claude);
    [Line::from(top).style(row), Line::from(bottom).style(row)]
}

/// `s` cut to `n` chars, ending in `…` if it was longer.
fn cut(s: &str, n: usize) -> String {
    match s.chars().count() > n {
        true => format!("{}…", s.chars().take(n.saturating_sub(1)).collect::<String>()),
        false => s.to_owned(),
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

#[test]
fn filter_and_keys() {
    let pr = |number, title: &str, author: &str, updated: &str| Listed {
        number,
        title: title.into(),
        author: author.into(),
        base: "master".into(),
        branch: "b".into(),
        state: "OPEN".into(),
        updated: updated.into(),
        additions: number,
        files: 1,
        ..Listed::default()
    };
    let mut l = List::new("-wb".into());
    l.prs = vec![pr(1, "Fix CI", "ann", "2026-03"), pr(22, "Add docs", "bo", "2026-01"), pr(333, "fix fees", "cy", "2026-02")];
    (l.prs[1].draft, l.prs[1].decision, l.prs[1].labels) = (true, "APPROVED".into(), vec![("docs".into(), "0075ca".into())]);
    l.prs[2].checks = Some((3, 1, 0));
    let k = |l: &mut List, c| l.key(KeyEvent::new(c, KeyModifiers::NONE));
    let shown = |l: &List| l.shown().iter().map(|p| p.number).collect::<Vec<_>>();
    let filter = |l: &mut List, q: &str| (l.query = q.into(), shown(l)).1;
    assert_eq!(shown(&l), [1, 333, 22]); // most recently updated first
    assert_eq!(filter(&mut l, "fix"), [1, 333]); // smart case
    assert_eq!(filter(&mut l, "Fix"), [1]);
    assert_eq!(filter(&mut l, "is:draft"), [22]);
    assert_eq!(filter(&mut l, "-is:draft review:none"), [1, 333]);
    assert_eq!(filter(&mut l, "review:appr label:doc"), [22]);
    assert_eq!(filter(&mut l, "ci:fail"), [333]);
    assert_eq!(filter(&mut l, "ci:none -author:an"), [22]);
    assert_eq!(filter(&mut l, "docs"), [22]); // labels count as text too
    l.query.clear();
    // `/` types a filter; Enter keeps it, Esc (out of filter mode) clears it.
    k(&mut l, KeyCode::Char('/'));
    "fix".chars().for_each(|c| _ = k(&mut l, KeyCode::Char(c)));
    k(&mut l, KeyCode::Enter);
    assert_eq!(shown(&l), [1, 333]);
    k(&mut l, KeyCode::Char('j'));
    assert!(matches!(k(&mut l, KeyCode::Enter), Some(Do::Open(333))));
    k(&mut l, KeyCode::Esc); // clears the filter, doesn't quit
    assert_eq!(shown(&l), [1, 333, 22]);
    assert_eq!(filter(&mut l, "#4567"), [] as [u64; 0]); // not listed (older, or closed): Enter opens it anyway
    assert!(matches!(k(&mut l, KeyCode::Enter), Some(Do::Open(4567))));
    l.query.clear();
    // Ordering keeps the selected PR selected.
    k(&mut l, KeyCode::Char('j')); // on 333
    k(&mut l, KeyCode::Char('o')); // created: all equal, so stable
    k(&mut l, KeyCode::Char('o')); // number
    assert_eq!(shown(&l), [333, 22, 1]);
    k(&mut l, KeyCode::Char('O'));
    assert_eq!(shown(&l), [1, 22, 333]);
    assert_eq!(l.shown()[l.sel].number, 333);
    // `?` shows the help; the next key only closes it.
    k(&mut l, KeyCode::Char('?'));
    assert!(l.help);
    assert!(k(&mut l, KeyCode::Char('q')).is_none() && !l.help);
    // Tab: the PRs with a worktree, including one that's merged (not in the open tab).
    l.prs.push(Listed { number: 9, state: "MERGED".into(), ..pr(9, "old", "dee", "2025-01") });
    l.local.insert(9, ("/x/r-9".into(), None));
    l.local.insert(22, ("/x/r-22".into(), Some("feat".into())));
    assert_eq!(shown(&l), [1, 22, 333]); // open tab: not the merged one
    k(&mut l, KeyCode::Tab);
    assert_eq!(shown(&l), [9, 22]); // (still by number, ascending)
    k(&mut l, KeyCode::Char('j'));
    assert!(matches!(k(&mut l, KeyCode::Enter), Some(Do::Open(22))));
    // D asks first; D again removes it; anything else in between keeps it.
    assert!(k(&mut l, KeyCode::Char('D')).is_none() && l.msg.starts_with("D again removes /x/r-22"));
    k(&mut l, KeyCode::Char('o'));
    assert!(k(&mut l, KeyCode::Char('D')).is_none());
    assert!(matches!(k(&mut l, KeyCode::Char('D')), Some(Do::Remove(22))));
    // m marks (and moves on); a asks, a again has Claude review the marked ones.
    k(&mut l, KeyCode::BackTab); // open PRs: 1, 22, 333
    k(&mut l, KeyCode::Char('g'));
    k(&mut l, KeyCode::Char('g'));
    k(&mut l, KeyCode::Char('m'));
    k(&mut l, KeyCode::Char('j')); // past 22
    k(&mut l, KeyCode::Char('m'));
    assert_eq!(l.marked, HashSet::from([1, 333]));
    assert!(k(&mut l, KeyCode::Char('a')).is_none() && l.msg.starts_with("a again: Claude reviews #1 #333"));
    assert!(matches!(k(&mut l, KeyCode::Char('a')), Some(Do::Review(ref p)) if p == &[1, 333]));
    // A failed review can be asked for again.
    l.batch.insert(1, batch::Status::Failed("boom".into()));
    l.marked = HashSet::from([1]);
    assert!(k(&mut l, KeyCode::Char('a')).is_none() && l.msg.starts_with("a again: Claude reviews #1"), "{}", l.msg);
    assert!(matches!(k(&mut l, KeyCode::Char('a')), Some(Do::Review(ref p)) if p == &[1]));
    assert_eq!(l.claude_state(1), Some(("claude failed: boom".into(), false))); // why, on its card
    (l.marked, l.batch) = (HashSet::new(), HashMap::new());
    // While reviews run, cards say so, and q asks before quitting.
    l.batch.insert(333, batch::Status::Reviewing);
    l.reviewed.insert(1);
    assert_eq!(l.claude_state(333), Some(("claude: reviewing…".into(), true)));
    assert_eq!(l.claude_state(1), Some(("claude ✓".into(), true)));
    assert!(k(&mut l, KeyCode::Char('q')).is_none() && l.msg.starts_with("Claude is still reviewing"));
    assert!(matches!(k(&mut l, KeyCode::Char('q')), Some(Do::Quit)));
    // A A drafts one replacement for the marked PRs (or this one).
    l.marked = HashSet::from([22, 1]);
    assert!(k(&mut l, KeyCode::Char('A')).is_none() && l.msg.starts_with("A again: Claude suggests how to group #1 #22"));
    assert!(matches!(k(&mut l, KeyCode::Char('A')), Some(Do::Replace(ref p)) if p == &[1, 22]));
    // The replacements tab: its cards, and the PRs it replaces say so.
    let o = |number| crate::replace::Original { number, author: "bot".into(), ..Default::default() };
    let r = crate::replace::Record { id: 1, replaces: vec![o(1), o(22)], title: "Do it properly".into(), ..Default::default() };
    l.reps = vec![Listed { number: 1, title: r.title.clone(), ..Listed::default() }];
    l.rep_of.insert(1, r);
    assert_eq!(l.claude_state(22), Some(("replacement ✓".into(), true)));
    l.drafting.insert(1, batch::Status::Reviewing);
    assert_eq!(l.draft_state(1, "claude"), Some(("claude: drafting…".into(), true)));
    k(&mut l, KeyCode::Tab);
    k(&mut l, KeyCode::Tab);
    assert_eq!(l.tab, Tab::Replacements);
    assert_eq!(shown(&l), [1]);
    assert!(k(&mut l, KeyCode::Char('a')).is_none() && l.msg.starts_with("a replacement: Enter opens it"));
    assert!(k(&mut l, KeyCode::Char('D')).is_none());
    assert!(matches!(k(&mut l, KeyCode::Char('D')), Some(Do::RemoveReplacement(1))));
}

#[test]
fn selected_card_readable() {
    use ratatui::style::Color;
    let mut ui = crate::theme::Theme::load().ui();
    (ui.menu_selected, ui.stale) = (Style::new().fg(Color::Black).bg(Color::Cyan), Style::new().fg(Color::Gray));
    let cx = Cards { ui: &ui, num_w: 5, width: 120, checks_loading: false, usual: "main" };
    let p = Listed { number: 1, title: "t".into(), author: "amy".into(), labels: vec![("bug".into(), "d73a4a".into())], ..Listed::default() };
    let mark = || Mark { marked: false, claude: Some(("claude ✓".into(), true)), new: true, replaces: None };
    let [top, bottom] = card(&p, &cx, true, None, mark());
    for line in [&top, &bottom] {
        assert_eq!(line.width(), 120); // the highlight runs to the edge
        // The highlight's own text color throughout, but for the label chip.
        assert!(line.spans.iter().filter(|s| s.content != " bug ").all(|s| s.style.fg == Some(Color::Black)), "{line:?}");
    }
    let [_, bottom] = card(&p, &cx, false, None, mark());
    assert!(bottom.spans.iter().any(|s| s.style.fg == Some(Color::Gray))); // unselected: its colors
    // [new] at the right end of the top line, before the age.
    let [top, _] = card(&p, &cx, false, None, mark());
    assert_eq!(top.width(), 120);
    assert_eq!(top.spans[top.spans.len() - 2].content, "[new] ");
    let [top, _] = card(&p, &cx, false, None, Mark { new: false, ..mark() });
    assert!(!top.spans.iter().any(|s| s.content.contains("[new]")));
    // Claude's part at the right end, even when the rest has to be cut for it.
    assert_eq!(bottom.spans.last().unwrap().content, "claude ✓ ");
    let narrow = Cards { width: 30, ..cx };
    let p = Listed { branch: "a-very-long-branch-name-indeed".into(), ..p };
    let [_, bottom] = card(&p, &narrow, false, None, mark());
    assert_eq!(bottom.width(), 30);
    assert_eq!(bottom.spans.last().unwrap().content, "claude ✓ ");
    assert!(!bottom.spans.iter().rev().nth(2).is_some_and(|s| s.content == " · ")); // no separator left dangling
}

#[test]
fn marks_saved() {
    let dir = std::env::temp_dir().join(format!("dv-marks-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("sub/marked.json");
    assert!(read_marks(&p).is_empty()); // none yet
    write_marks(&p, &HashSet::from([22, 3])).unwrap();
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "[3,22]");
    assert_eq!(read_marks(&p), HashSet::from([3, 22]));
    // Another dv list marked 7 meanwhile; this one, which had 3 and 22, unmarks 3 and marks 9.
    write_marks(&p, &HashSet::from([3, 22, 7])).unwrap();
    let now = merge_marks(&p, &HashSet::from([3, 22]), &HashSet::from([22, 9])).unwrap();
    assert_eq!(now, HashSet::from([22, 7, 9]));
    _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn screens() {
    use crate::view::tests::{screen, snapshot};
    let pr = |number, title: &str, author: &str, updated: &str| Listed {
        number,
        title: title.into(),
        author: author.into(),
        base: "master".into(),
        branch: "b".into(),
        state: "OPEN".into(),
        updated: updated.into(), // not a time: shown as it is, so the snapshot doesn't age
        additions: number,
        files: 1,
        ..Listed::default()
    };
    let mut l = List::new("".into());
    l.prs = vec![pr(1, "Fix CI", "ann", "2026-03"), pr(22, "Add docs", "bo", "2026-01"), pr(333, "fix fees", "cy", "2026-02")];
    (l.prs[1].draft, l.prs[1].decision, l.prs[1].labels) = (true, "APPROVED".into(), vec![("docs".into(), "0075ca".into())]);
    l.prs[2].checks = Some((3, 1, 0));
    l.marked = HashSet::from([333]);
    l.new = HashSet::from([22]);
    l.reviewed = HashSet::from([1]);
    let k = |l: &mut List, c| l.key(KeyEvent::new(c, KeyModifiers::NONE));
    k(&mut l, KeyCode::Char('j'));
    snapshot("list", &screen(100, 12, |f| l.draw(f)));
    l.query = "fix".into();
    snapshot("list_filtered", &screen(100, 12, |f| l.draw(f)));
    l.query.clear();
    k(&mut l, KeyCode::Char('?'));
    snapshot("list_help", &screen(100, 60, |f| l.draw(f)));
    k(&mut l, KeyCode::Esc);
    k(&mut l, KeyCode::Tab);
    snapshot("list_worktrees", &screen(100, 8, |f| l.draw(f)));
    k(&mut l, KeyCode::Tab);
    snapshot("list_replacements", &screen(100, 8, |f| l.draw(f)));
}
