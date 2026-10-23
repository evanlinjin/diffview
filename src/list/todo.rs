//! The todo tab: what needs you (`crate::todo`), in order, and Claude's part in it.

use super::*;
use crate::todo::{self, Facts, Item};

impl List {
    /// What needs you, from what's known now, in Claude's order (where it ranked them).
    /// (The cursor stays on its card through it as the event loop keeps it: `select_card`.)
    pub(super) fn find_todo(&mut self) {
        let dir = self.reviews_dir.clone();
        let seen = dir.as_ref().and_then(|d| d.parent()).map(|d| d.join("seen"));
        // Claude's CI notes (as `crate::claude::ci_note_path`, without a git run per PR).
        let ci = dir.as_ref().and_then(|d| d.parent()).map(|d| d.join("ci"));
        let secs = |p: std::path::PathBuf| -> Option<i64> {
            let t = std::fs::metadata(p).ok()?.modified().ok()?;
            Some(t.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs() as i64)
        };
        let mut items = vec![];
        // (Reading each review: which Claude would close too, the "can close" tab.)
        let mut closable = HashMap::new();
        // (And which Claude would replace with one of its own: auto drafts it.)
        let mut replaceable = HashMap::new();
        let mut claude_open = HashMap::new();
        for p in self.prs.iter().filter(|p| p.open()) {
            let file = dir.as_ref().map(|d| d.join(format!("pr-{}.json", p.number)));
            let (review, unreadable) = match file.as_ref().map(|f| crate::claude::load(f)) {
                Some(Ok(r)) => (r, false),
                Some(Err(_)) => (None, true),
                None => (None, false),
            };
            if let Some(r) = review.as_ref().filter(|r| !r.close.trim().is_empty()) {
                // (Reviewed at an older head: as the todo tab has it, its review's to update.)
                let head = self.extra.get(&p.number).map_or("", |e| e.head_oid.as_str());
                let older = !r.head.is_empty() && !head.is_empty() && r.head != head;
                closable.insert(p.number, format!("{}{}", r.close.trim(), if older { " (reviewed at an older head)" } else { "" }));
            }
            // Claude's comments not resolved; reviewed at an older head, some may be outdated
            // (the view would say: the list doesn't diff); a file that won't read, said so.
            match &review {
                Some(r) => {
                    let head = self.extra.get(&p.number).map_or("", |e| e.head_oid.as_str());
                    let older = !r.head.is_empty() && !head.is_empty() && r.head != head;
                    claude_open.insert(p.number, Some((r.threads.iter().filter(|t| t.by_claude() && !t.resolved).count(), older)));
                }
                None if unreadable => _ = claude_open.insert(p.number, None),
                None => {}
            }
            if let Some(r) = review.as_ref().filter(|r| !r.replace.trim().is_empty() && r.close.trim().is_empty()) {
                replaceable.insert(p.number, r.replace.trim().to_owned());
            }
            let looked = seen.as_ref().and_then(|s| std::fs::read_to_string(s.join(format!("pr-{}.looked", p.number))).ok()?.trim().parse().ok());
            let (f, c, head_at) = self.unpushed.get(&p.number).copied().unwrap_or((0, 0, None));
            let head = self.extra.get(&p.number).map_or("", |e| e.head_oid.as_str());
            let ci_note = ci.as_ref().and_then(|d| crate::claude::ci_note_at(&d.join(format!("pr-{}.json", p.number)), head));
            let facts = Facts { pr: p, extra: self.extra.get(&p.number), me: &self.me, review: review.as_ref(), looked, review_changed: file.and_then(secs), unpushed: (f, c), head_at, ci_note: ci_note.as_ref() };
            let (needs, claude) = todo::needs(&facts);
            if !needs.is_empty() {
                items.push(Item { kind: crate::todo::Kind::Pr, number: p.number, needs, claude, updated: p.updated.clone(), reason: None, changed: false, deferred: None, busy: None, waits: None });
            }
        }
        self.closable = closable;
        self.replaceable = replaceable;
        self.claude_open = claude_open;
        for r in self.rep_of.values() {
            let needs = todo::rep_needs(r);
            let updated = self.reps.iter().find(|c| c.number == r.id).map_or(String::new(), |c| c.updated.clone());
            if !needs.is_empty() {
                items.push(Item { kind: crate::todo::Kind::Rep, number: r.id, needs, claude: None, updated, reason: None, changed: false, deferred: None, busy: None, waits: None });
            }
        }
        let now = crate::claude::now_secs();
        let met = self.todos.met(&items, |it, c, at| self.happened(it, c, at));
        let (mut items, said) = self.todos.settle(items, met);
        if let Some(said) = said {
            self.msg = said;
        }
        // What a PR's conversation says it waits on (a trusted comment), not happened yet: to
        // the bottom too, for everyone.
        for it in items.iter_mut().filter(|it| !it.is_rep() && it.deferred.is_none()) {
            let said = self.extra.get(&it.number).map(|e| e.said.as_slice()).unwrap_or(&[]);
            // A comment's word, else what Claude saw in the conversation when it ranked.
            let claude = self.todos.ranked.as_ref().and_then(|r| r.waits.get(&it.key())).map(|c| (c.clone(), String::new()));
            // (A push from no known head: nothing to tell it by.)
            let claude = claude.filter(|(c, _)| !c.unwatchable());
            // (A review since it was said: the comment's, or since Claude ranked.)
            let (waits, since) = match todo::waiting_on(said, now) {
                Some((c, who, at)) => (Some((c, who)), at),
                None => (claude, self.todos.ranked.as_ref().map_or(0, |r| r.at)),
            };
            it.waits = waits.filter(|(c, _)| !self.happened(it, c, since));
        }
        let (mut waiting, mut rest): (Vec<Item>, Vec<Item>) = items.into_iter().partition(|it| it.waits.is_some());
        rest.append(&mut waiting);
        let items = rest;
        // Those Claude is at (here or in any dv; drafting their replacement too): at the bottom
        // till it's done.
        let doing = |it: &Item| match it.is_rep() {
            // A replacement: Claude drafting it, or at it in its worktree.
            true => self.drafting_replacement(it.number).then(|| "drafting it".to_owned()).or_else(|| self.running_reps.get(&it.number).cloned()),
            false => self.claude_doing(it.number),
        };
        let (mut busy, todo): (Vec<Item>, Vec<Item>) = items.into_iter().partition(|it| doing(it).is_some());
        busy.iter_mut().for_each(|it| it.busy = doing(it));
        // A replacement drafted, ready for you to review: first.
        let (mut ready, todo): (Vec<Item>, Vec<Item>) = todo.into_iter().partition(|it| it.is_rep() && it.needs.contains(&todo::Need::Publish));
        // CI still running on it (a replacement's: on its PR): what it needs mostly waits on
        // that (merging, marking it ready, closing what it replaces), so after what you can do
        // now, before what you've put aside or wait on, and what Claude's at. Not with checks
        // failing already: those can be fixed now. Back up once CI's done.
        let ci_running = |it: &Item| {
            let n = self.pr_of(it);
            n.and_then(|n| self.prs.iter().find(|p| p.number == n)).and_then(|p| p.checks).is_some_and(|c| c.2 > 0 && c.1 == 0)
        };
        let (mut later, todo): (Vec<Item>, Vec<Item>) = todo.into_iter().partition(|it| it.deferred.is_some() || it.waits.is_some());
        let (mut ci, mut todo): (Vec<Item>, Vec<Item>) = todo.into_iter().partition(ci_running);
        ready.append(&mut todo);
        ready.append(&mut ci);
        ready.append(&mut later);
        ready.append(&mut busy);
        self.todos.items = ready;
        self.auto_run();
    }

    /// Auto (`C`): Claude does its part on each item, as `a a` would have it (fixing your CI,
    /// reviewing, updating its review, answering you), each once a head (an answer, once a
    /// change to its review file); green drafts of yours are marked ready for review; a
    /// published replacement, green, closes what it replaces; and what needs you is ranked again
    /// as it changes. Not what you've put aside or wait on, not where Claude's at it already.
    fn auto_run(&mut self) {
        if !self.todos.auto {
            return;
        }
        let todo: Vec<Item> = self.todos.items.iter().filter(|it| it.busy.is_none() && it.deferred.is_none() && it.waits.is_none()).cloned().collect();
        // A published replacement, green, nothing of it unpushed: what it replaces is closed,
        // with its closing note (their items aren't Claude's to review any more, then).
        let mut close = vec![];
        for it in todo.iter().filter(|it| it.is_rep() && it.needs.iter().any(|n| matches!(n, todo::Need::CloseOriginals(_)))) {
            let Some(r) = self.rep_of.get(&it.number) else { continue };
            let Some(p) = r.published.and_then(|n| self.prs.iter().find(|p| p.number == n)) else { continue };
            // (Not while Claude's at one of them: closed under it.)
            let at_one = r.replaces.iter().any(|o| self.claude_doing(o.number).is_some());
            if !at_one && !p.checks_head.is_empty() && todo::green(p.checks) && self.auto_done.insert((it.number, p.checks_head.clone(), "close")) {
                close.push(r.clone());
            }
        }
        let closing: HashSet<u64> = close.iter().flat_map(|r| r.replaces.iter().map(|o| o.number)).collect();
        let (mut run, mut ready) = (vec![], vec![]);
        // One batch of Claude's at a time, of at most as many as run at once: the cap holds, and
        // the rest wait for a later look (most pressing first, in the todo's order).
        let batch_busy = self.batch.values().any(batch::Status::running);
        for it in todo.iter().filter(|it| !it.is_rep() && !closing.contains(&it.number)) {
            let Some(p) = self.prs.iter().find(|p| p.number == it.number) else { continue };
            let Some(head) = self.extra.get(&it.number).map(|e| e.head_oid.clone()).filter(|h| !h.is_empty()) else { continue };
            // CI's state is about the head the rest knows of (else it's another commit's).
            let ci_here = head == p.checks_head;
            match it.claude.clone() {
                Some(t) if !batch_busy && run.len() < batch::AT_ONCE && (t != batch::Task::FixChecks || ci_here) => {
                    // Answers: once each change to the review file (your new replies), not a head.
                    let at = match t {
                        batch::Task::Answer { .. } => format!("{head}@{}", self.review_mtime(it.number)),
                        _ => head,
                    };
                    if self.auto_done.insert((it.number, at, t.doing())) {
                        run.push((it.number, t));
                    }
                }
                _ if ci_here && it.needs.contains(&todo::Need::MarkReady) && self.auto_done.insert((it.number, head, "ready")) => ready.push(it.number),
                _ => {}
            }
        }
        // What else Claude does here passes over: a PR it's at, or about to be (once a PR), put
        // aside or waited on, or being closed; and yours (you'd close or change it yourself).
        let skip: HashSet<u64> = self
            .todos
            .items
            .iter()
            .filter(|it| it.busy.is_some() || it.deferred.is_some() || it.waits.is_some())
            .map(|it| it.number)
            // (Claude at it anywhere, a PR with no item even, a run held back too: `claude_doing`.)
            .chain(self.running.keys().chain(self.batch.keys()).copied().filter(|&n| self.claude_doing(n).is_some()))
            .chain(run.iter().map(|r| r.0))
            .chain(closing.iter().copied())
            .chain(self.prs.iter().filter(|p| p.author == self.me).map(|p| p.number))
            .collect();
        // PRs Claude's review would close with no closing comment drafted yet: drafted, once each
        // at a head (it's yours to close, after your editor, from the PR's view).
        for &n in self.closable.keys().filter(|&&n| !skip.contains(&n)) {
            if batch_busy || run.len() >= batch::AT_ONCE {
                break;
            }
            let Some(head) = self.extra.get(&n).map(|e| e.head_oid.clone()).filter(|h| !h.is_empty()) else { continue };
            let file = self.reviews_dir.as_ref().map(|d| d.join(format!("pr-{n}.json")));
            let drafted = file.and_then(|f| crate::claude::load(&f).ok().flatten()).is_some_and(|r| !r.closing.trim().is_empty());
            if !drafted && self.auto_done.insert((n, head, batch::Task::Close.doing())) {
                run.push((n, batch::Task::Close));
            }
        }
        // Your PRs changed since their descriptions were checked: Claude checks again, proposing
        // a new one if it no longer fits (Space a in the PR approves it). One first seen: its
        // head noted, nothing asked (nothing's changed that dv knows of).
        // (As saved now: another dv list may have noted some since.)
        if let Some(saved) = read_described() {
            self.described = saved;
        }
        let mut noted = vec![];
        // (Claude at it, `claude_doing`: a run held back too; or its item put aside.)
        let busy: HashSet<u64> =
            self.prs.iter().map(|p| p.number).filter(|&n| self.claude_doing(n).is_some() || self.todos.items.iter().any(|it| it.number == n && (it.busy.is_some() || it.deferred.is_some()))).collect();
        let busy = |n: u64| busy.contains(&n);
        for p in self.prs.iter().filter(|p| p.open() && p.author == self.me) {
            let Some(head) = self.extra.get(&p.number).map(|e| e.head_oid.clone()).filter(|h| !h.is_empty()) else { continue };
            match self.described.get(&p.number).cloned() {
                None => noted.push((p.number, head)),
                // (A draft: its description's still to come. One Claude hasn't reviewed: that's
                // its first run, not this: a description alone would stand for a review.)
                // (Once a head, as auto's every task: `auto_done`; noted checked once it is.)
                Some(old) if old != head && !p.draft && !batch_busy && run.len() < batch::AT_ONCE && !busy(p.number) && !run.iter().any(|r| r.0 == p.number) => {
                    let f = self.reviews_dir.as_ref().map(|d| d.join(format!("pr-{}.json", p.number)));
                    let review = f.and_then(|f| crate::claude::load(&f).ok().flatten());
                    // A proposal already waiting on you: that first, and the head not noted, so
                    // what's changed since is checked after. (A first review that proposes one
                    // holds this run off the same way, till you've dealt with it.)
                    if review.as_ref().is_some_and(|r| r.description.trim().is_empty()) && self.auto_done.insert((p.number, head.clone(), "checking its description")) {
                        run.push((p.number, batch::Task::Describe(old, head)));
                    }
                }
                _ => {}
            }
        }
        for (n, head) in noted {
            self.note_described(n, head);
        }
        // A PR Claude would replace (low effort, worth keeping) with no replacement of it yet:
        // drafted, one at a time, once each at a head (unpublished: the drafts tab has it).
        let drafting = self.drafting.values().any(batch::Status::running) || self.running_reps.keys().any(|&id| self.drafting_elsewhere(id));
        let replaced = |n: u64| self.rep_of.values().any(|r| r.replaces.iter().any(|o| o.number == n));
        let next = self.replaceable.iter().find_map(|(&n, why)| {
            let head = self.extra.get(&n).map(|e| e.head_oid.clone()).filter(|h| !h.is_empty())?;
            (!replaced(n) && !skip.contains(&n) && !self.auto_done.contains(&(n, head.clone(), "replace"))).then(|| (n, head, why.clone()))
        });
        let mut replacing = None;
        if let (false, Some((n, head, why))) = (drafting, next) {
            self.auto_done.insert((n, head, "replace"));
            // (Your instructions for replacements, as a first draft gets them, and why this one.)
            let saved = crate::claude::instructions("replace");
            let extra = format!("{}Claude's review of #{n} found it worth replacing: {why}", if saved.trim().is_empty() { String::new() } else { format!("{}\n\n", saved.trim()) });
            self.draft_under_way(n, batch::replace(vec![n], self.root.clone(), extra));
            replacing = Some(n);
        }
        let mut said = vec![];
        if let Some(n) = replacing {
            said.push(format!("Claude is drafting a replacement of #{n} (its review: low effort, worth keeping; the drafts tab has it)"));
        }
        if !run.is_empty() {
            said.push(format!("Claude is {}", self.start_batch(run, None)));
        }
        // `mark_ready` and the closing check CI again first. Said either way (not tried again
        // at this head: you'd be told each time).
        for n in ready {
            said.push(match crate::gh::mark_ready(n) {
                Ok(()) => format!("#{n} marked ready for review (CI passed)"),
                Err(e) => format!("#{n} not marked ready: {e:#}"),
            });
        }
        for mut r in close {
            let n = r.published.unwrap_or(0);
            let note = crate::replace::note(&r);
            let res = self.rep_pushed(&r).and_then(|()| crate::replace::close_originals(&mut r, &note));
            said.push(match res {
                Ok(said) => format!("{said} (replaced by #{n}, its CI passed)"),
                Err(e) => format!("not closing what #{n} replaces: {e:#}"),
            });
            self.load_reps();
        }
        // Ranked again once something's new or changed, at most every quarter hour (a run each,
        // tried or not: a failing one isn't run over and over).
        if self.want_rank.is_none() && self.todos.auto_rank_due() {
            self.rank_now(false);
        }
        if !said.is_empty() {
            self.msg = format!("auto: {}", said.join("; "));
        }
    }

    /// When Claude's review file for PR `n` last changed (seconds; 0: none).
    fn review_mtime(&self, n: u64) -> u64 {
        let f = self.reviews_dir.as_ref().map(|d| d.join(format!("pr-{n}.json")));
        let t = f.and_then(|f| std::fs::metadata(f).ok()?.modified().ok());
        t.and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs())
    }

    /// Replacement `r`'s branch as published: nothing in its worktree left out (uncommitted, or
    /// commits not pushed), and its PR's CI passing on GitHub now.
    fn rep_pushed(&self, r: &crate::replace::Record) -> anyhow::Result<()> {
        // (Its worktree as everywhere: made again if it's gone; git failing, said, not read as
        // nothing there.)
        let d = crate::replace::worktree(r, &self.root)?.to_string_lossy().into_owned();
        let git = |args: &[&str]| crate::diff::git(&[&["-C", d.as_str()], args].concat());
        anyhow::ensure!(git(&["status", "--porcelain"])?.trim().is_empty(), "its worktree has uncommitted changes");
        anyhow::ensure!(git(&["rev-list", "--count", "@{upstream}..HEAD"])?.trim() == "0", "its worktree has commits not pushed");
        let n = r.published.unwrap_or(0);
        anyhow::ensure!(todo::green(Some(crate::gh::checks(n)?)), "#{n}'s CI isn't all green on GitHub now");
        Ok(())
    }

    /// The todo tab's items with their cards (a PR's, or a replacement's).
    pub(super) fn todo_shown(&self) -> Vec<(&Listed, &Item)> {
        self.todos.items
            .iter()
            .filter_map(|it| {
                let cards = if it.is_rep() { &self.reps } else { &self.prs };
                Some((cards.iter().find(|c| c.number == it.number)?, it))
            })
            .collect()
    }

    /// Have Claude rank what needs you (`r r`: a Claude run, so only when you ask), once what's
    /// needed is in (GitHub's part).
    pub(super) fn rank_now(&mut self, asked: bool) {
        // (Not reading every review for it, if it can't; said only if you asked.)
        if let Err(why) = self.todos.can_rank() {
            self.want_rank = None;
            return if asked { self.msg = why.into() };
        }
        if self.extra_rx.is_some() || self.checks.is_some() {
            // (Said once, when you ask.)
            if asked && self.want_rank != Some(true) {
                self.msg = "Claude ranks what needs you once GitHub's part is in…".into();
            }
            return self.want_rank = Some(asked);
        }
        self.want_rank = None;
        let items: Vec<(todo::Entry, String)> = self
            .todo_shown()
            .into_iter()
            .filter(|(_, it)| it.deferred.is_none()) // not ranked while deferred
            .map(|(card, it)| {
                let summary = match it.is_rep() {
                    true => self.rep_of.get(&it.number).map(|r| r.body.clone()).unwrap_or_default(),
                    false => self.reviews_dir.as_ref().and_then(|d| crate::claude::load(&d.join(format!("pr-{}.json", it.number))).ok().flatten()).map(|r| r.summary).unwrap_or_default(),
                };
                (it.entry(), todo::about(it, card, &summary))
            })
            .collect();
        let n = items.len();
        self.msg = match self.todos.rank(items, &self.root, todo::Of::Prs) {
            Ok(()) => format!("Claude is ranking the {n} PR(s) that need you…"),
            Err(why) => why.into(),
        };
    }

    /// News from the background: GitHub's part (then what needs you again), and a ranking.
    pub(super) fn todo_news(&mut self) {
        if let Some(Ok(res)) = self.extra_rx.as_ref().map(Receiver::try_recv) {
            self.extra_rx = None;
            match res {
                Ok((me, extra)) => {
                    (self.me, self.extra) = (me, extra);
                    self.scan_unpushed(); // against the PRs' heads, now they're known
                }
                Err(e) => {
                    // (Not ranked on what didn't come: the error stays said.) GitHub out of reach
                    // (the network): nothing for you to do but wait; dv asks again shortly. Else
                    // (a login run out, gh missing): the whole of it, for you to see to.
                    let why = format!("{e:#}");
                    self.msg = match crate::gh::unreachable(&why) {
                        true => {
                            let (tx, rx) = mpsc::channel();
                            std::thread::spawn(move || {
                                std::thread::sleep(Duration::from_secs(30));
                                _ = tx.send(github::list_extra());
                            });
                            self.extra_rx = Some(rx);
                            let kept = if self.extra.is_empty() { "none yet" } else { "it's as it was" };
                            format!("GitHub didn't answer for the todo's details ({kept}); dv asks again in 30s, R now")
                        }
                        false => format!("the todo's details from GitHub: {why}"),
                    };
                    self.want_rank = None;
                }
            }
            self.find_todo();
        }
        if let Some(asked) = self.want_rank {
            self.rank_now(asked);
        }
        let Some(res) = self.todos.rank_news() else { return };
        match res {
            Ok(_) => {
                self.find_todo();
                self.msg = "Claude ranked what needs you: a a on one has it do its part there".into();
            }
            Err(e) => self.msg = format!("ranking: {e:#} (r r tries again)"),
        }
    }

    /// What's in each PR worktree that isn't on GitHub: (changed files, untracked ones too;
    /// commits beyond the PR's head). Looked at on refresh, back from a PR, and when Claude's
    /// done somewhere (two git calls a worktree: not every frame).
    pub(super) fn scan_unpushed(&mut self) {
        if self.unpushed_rx.is_some() {
            self.rescan = true; // after the one under way
            return;
        }
        let local: Vec<(u64, String)> = self.local.iter().map(|(&n, (dir, _))| (n, dir.to_string_lossy().into_owned())).collect();
        let heads: HashMap<u64, String> = self.extra.iter().map(|(&n, e)| (n, e.head_oid.clone())).collect();
        let bases: HashMap<u64, String> = self.prs.iter().map(|p| (p.number, p.base.clone())).collect();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let found = local
                .into_iter()
                .filter_map(|(n, d)| {
                    let git = |args: &[&str]| crate::diff::git(&[&["-C", d.as_str()], args].concat()).ok();
                    let files = git(&["status", "--porcelain"])?.lines().count();
                    // Beyond the PR's head as GitHub has it (fetched if it isn't here), as dv's
                    // one rule has it (`only_here`). Its head not known yet: asked again once it
                    // is.
                    let head = heads.get(&n).filter(|h| !h.is_empty()).filter(|h| {
                        git(&["cat-file", "-e", &format!("{h}^{{commit}}")]).is_some()
                            || git(&["fetch", "-q", "--no-write-fetch-head", &crate::diff::remote(), &format!("refs/pull/{n}/head")]).is_some()
                    });
                    let commits = head.map_or(0, |h| crate::only_here(std::path::Path::new(&d), n, h, bases.get(&n).map(String::as_str)));
                    // When the PR's head was made: Claude's changes after it are what's waiting.
                    let at = head.and_then(|h| git(&["log", "-1", "--format=%ct", h])).and_then(|t| t.trim().parse().ok());
                    (files + commits > 0).then_some((n, (files, commits, at)))
                })
                .collect();
            _ = tx.send(found);
        });
        self.unpushed_rx = Some(rx);
    }

    /// The worktrees' PRs looked up (`apply_list`), when they're in: on their cards.
    pub(super) fn gone_news(&mut self) {
        let mut found = vec![];
        self.gone_rx.retain(|rx| match rx.try_recv() {
            Ok(f) => {
                found.extend(f);
                false
            }
            Err(mpsc::TryRecvError::Empty) => true,
            Err(mpsc::TryRecvError::Disconnected) => false,
        });
        if self.gone_rx.is_empty() {
            self.gone_pending.clear(); // a lookup that died: asked again next time
        }
        if found.is_empty() {
            return;
        }
        for (n, p) in found {
            self.gone_pending.remove(&n);
            // Not wanted any more (its worktree removed, its mark gone): left out.
            let Some(o) = self.prs.iter_mut().find(|o| o.number == n) else { continue };
            match p {
                Some(p) => {
                    self.gone.insert(n, p.clone());
                    let (checks, head) = (o.checks, std::mem::take(&mut o.checks_head));
                    *o = Listed { checks, checks_head: head, ..p };
                }
                // Not to be had: said so, and asked again on the next refresh.
                None => o.title = "(couldn't load this PR)".into(),
            }
        }
        self.prune_marks();
        self.find_todo();
    }

    /// The scan's result, when it's in: on the todo tab then.
    pub(super) fn unpushed_news(&mut self) {
        let found = match self.unpushed_rx.as_ref().map(Receiver::try_recv) {
            Some(Ok(found)) => found,
            Some(Err(mpsc::TryRecvError::Disconnected)) => return self.unpushed_rx = None, // it died: the next asks again
            _ => return,
        };
        (self.unpushed, self.unpushed_rx) = (found, None);
        self.find_todo();
        if std::mem::take(&mut self.rescan) {
            self.scan_unpushed();
        }
    }

    /// Whether what deferral condition `c` waits for has happened, for item `it`: a review or an
    /// approval, one since `since` (when it was deferred: not one it had then; 0, any).
    pub(super) fn happened(&self, it: &Item, c: &todo::Until, since: i64) -> bool {
        let n = self.pr_of(it);
        let pr = self.prs.iter().find(|p| Some(p.number) == n);
        let state = |n: u64| self.prs.iter().find(|p| p.number == n).map(|p| p.state.clone()).or_else(|| self.others.get(&n).cloned());
        let extra = n.and_then(|n| self.extra.get(&n));
        let reviews: Vec<&(String, String, i64)> = extra.into_iter().flat_map(|e| &e.reviews).filter(|r| r.2 > since).collect();
        match c {
            todo::Until::Time(t) => crate::claude::now_secs() >= *t,
            todo::Until::Merged(n) => state(*n).is_some_and(|s| s == "MERGED"),
            todo::Until::Closed(n) => state(*n).is_some_and(|s| s == "MERGED" || s == "CLOSED"),
            todo::Until::Green => todo::green(pr.and_then(|p| p.checks)),
            // (Approved already when deferred: an approval since, not its standing decision.)
            todo::Until::Approved(None) => since == 0 && pr.is_some_and(|p| p.decision == "APPROVED") || reviews.iter().any(|r| r.1 == "APPROVED"),
            todo::Until::Approved(Some(who)) => reviews.iter().any(|r| &r.0 == who && r.1 == "APPROVED"),
            todo::Until::Reviewed(Some(who)) => reviews.iter().any(|r| &r.0 == who),
            // (Someone else: not you, nor its author answering in a review.)
            todo::Until::Reviewed(None) => reviews.iter().any(|r| r.0 != self.me && pr.is_some_and(|p| r.0 != p.author)),
            // (Or no more pushes to come: merged, closed.)
            todo::Until::PushedTo(n, head) => {
                self.extra.get(n).is_some_and(|e| todo::Until::moved_past(head, &e.head_oid)) || state(*n).is_some_and(|s| s != "OPEN")
            }
            todo::Until::Pushed(head) => extra.is_some_and(|e| todo::Until::moved_past(head, &e.head_oid)),
        }
    }

    /// `s` then `u`: defer the item until what you write happens (`key`: the item's). dv reads
    /// the usual phrasings itself; the rest Claude puts in a form dv can watch for (one small
    /// run, in the background: `until_news`).
    /// "…, or 2 weeks": whichever comes first.
    pub(super) fn defer_until(&mut self, key: String, text: &str) {
        match self.todos.until(key.clone(), text, || "a pull request".into(), &self.root) {
            Reading::Read(c, or) => self.defer_on(&key, c, or),
            Reading::Asked(said) => self.msg = said,
        }
    }

    /// The PR item `it` is about on GitHub: its own, or a replacement's once published.
    pub(super) fn pr_of(&self, it: &Item) -> Option<u64> {
        match it.is_rep() {
            true => self.rep_of.get(&it.number).and_then(|r| r.published),
            false => Some(it.number),
        }
    }

    fn defer_on(&mut self, key: &str, c: todo::Until, or: Option<i64>) {
        // (Seen to meanwhile, as Claude read when: said so.)
        let Some(it) = self.todos.items.iter().find(|it| it.key() == key).cloned() else { return self.msg = GONE.into() };
        // A push, a review: from where its PR is now (known once its details are in), so
        // what it had already doesn't count.
        let n = self.pr_of(&it);
        let extra = n.and_then(|n| self.extra.get(&n)).cloned();
        let c = match c {
            // (A draft not published: no PR to watch; publishing it brings it back anyway.)
            todo::Until::Green | todo::Until::Approved(_) | todo::Until::Reviewed(_) | todo::Until::Pushed(_) if n.is_none() => {
                return self.msg = "not deferred: it isn't published yet, so there's no PR to watch".into();
            }
            todo::Until::Pushed(_) => match extra.as_ref().map(|e| e.head_oid.clone()).filter(|h| !h.is_empty()) {
                Some(h) => todo::Until::Pushed(h),
                None => return self.msg = "not deferred: its head on GitHub isn't known yet, to see a push by (try again in a moment)".into(),
            },
            // Another PR's push: from its head now (a listed PR's).
            todo::Until::PushedTo(n, _) => match self.extra.get(&n).map(|e| e.head_oid.clone()).filter(|h| !h.is_empty()) {
                Some(h) => todo::Until::PushedTo(n, h),
                None => return self.msg = format!("not deferred: #{n}'s head isn't known (an open PR of this repo's?), to see a push by"),
            },
            todo::Until::Approved(_) | todo::Until::Reviewed(_) if extra.is_none() => {
                return self.msg = "not deferred: its reviews on GitHub aren't known yet, to tell a new one by (try again in a moment)".into();
            }
            c => c,
        };
        // (By GitHub's clock, and what dv had shown you: not one it hadn't, nor your clock's.)
        let at = extra.and_then(|e| e.reviews.iter().map(|r| r.2).max()).unwrap_or(0);
        self.msg = self.todos.defer_on(key, c, or, at);
        // A PR it waits on that isn't listed: its state, now (and on refresh).
        self.find_others();
        self.save_defer();
    }

    /// Claude's reading of a deferral's "until …", when it comes.
    /// Whether it deferred the item under the cursor (your own move: the cursor stays put, on
    /// what's next, rather than following it).
    pub(super) fn until_news(&mut self) -> bool {
        match self.todos.until_news() {
            Some(Ok((key, c, or))) => {
                let on_it = self.selected_item().is_some_and(|it| it.key() == key);
                self.defer_on(&key, c, or);
                on_it
            }
            Some(Err(said)) => (self.msg = said, false).1,
            None => false,
        }
    }

    /// The state of PRs that deferrals wait on and that aren't listed (merged or closed ones
    /// aren't), from GitHub.
    pub(super) fn find_others(&mut self) {
        let waited: Vec<u64> = self
            .todos
            .deferred
            .values()
            .filter_map(|d| match d.cond {
                Some(todo::Until::Merged(n) | todo::Until::Closed(n) | todo::Until::PushedTo(n, _)) => Some(n),
                _ => None,
            })
            .filter(|n| !self.prs.iter().any(|p| p.number == *n) && !self.others_pending.contains(n))
            .collect();
        if waited.is_empty() {
            return;
        }
        self.others_pending.extend(&waited);
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || _ = tx.send(waited.into_iter().filter_map(|n| Some((n, gh::pr_state(n).ok()??))).collect()));
        self.others_rx.push(rx);
    }

    /// Those PRs' states, when they're in: what deferrals wait for may have happened.
    pub(super) fn others_news(&mut self) {
        let mut found = vec![];
        self.others_rx.retain(|rx| match rx.try_recv() {
            Ok(f) => {
                found.extend(f);
                false
            }
            Err(mpsc::TryRecvError::Empty) => true,
            Err(mpsc::TryRecvError::Disconnected) => false,
        });
        if self.others_rx.is_empty() {
            self.others_pending.clear(); // done, or a lookup that died: asked again next time
        }
        if found.is_empty() {
            return;
        }
        self.others.extend(found);
        self.find_todo();
    }

    /// What needs you now (not deferred, Claude not at it), for the tab's count.
    pub(super) fn todo_now(&self) -> usize {
        self.todos.now()
    }

    /// Claude at work on replacement `id`'s checkout: drafting it (here, another dv), or in a pane
    /// there (dv's run, or one you started): nothing else starts there meanwhile.
    pub(super) fn rep_busy(&self, id: u64) -> bool {
        self.drafting.get(&id).is_some_and(batch::Status::running) || self.running_reps.contains_key(&id)
    }

    /// Replacement `id` being drafted by another dv (`dv replace`): its claim says so (not a
    /// run or pane in its worktree, making changes: those are `running_reps` too).
    pub(super) fn drafting_elsewhere(&self, id: u64) -> bool {
        self.running_reps.get(&id).is_some_and(|w| w == "drafting")
    }

    /// Claude is drafting a replacement of PR `n` (here, or in another dv).
    pub(super) fn drafting_replacement(&self, n: u64) -> bool {
        self.rep_of.values().filter(|r| r.id == n || r.replaces.iter().any(|o| o.number == n)).any(|r| {
            self.drafting.get(&r.id).is_some_and(batch::Status::running) || self.drafting_elsewhere(r.id)
        })
    }

    /// PR `n`'s description checked at `head`, kept (`described.json`): that one entry, into
    /// the file as it is now (another dv list's notes kept), the one place they're written.
    pub(super) fn note_described(&mut self, n: u64, head: String) {
        self.described.insert(n, head.clone());
        if let Some(f) = described_file() {
            let mut all = read_described().unwrap_or_default();
            all.insert(n, head);
            _ = super::marks::write_json(&f, &serde_json::json!(all));
        }
    }

    /// What Claude is doing on PR `n` now, if anything: a run from here (`a a`), or any dv's
    /// (its marker in dv's directory, `running`).
    pub(super) fn claude_doing(&self, n: u64) -> Option<String> {
        let doing = || self.batch_task.get(&n).map_or("reviewing", batch::Task::doing);
        match self.batch.get(&n) {
            Some(s) if s.running() => Some(doing().to_owned()),
            // Held back (Claude at it where it'd work): at it, as far as anything here goes.
            Some(batch::Status::Busy(..)) => Some(self.running.get(&n).cloned().unwrap_or_else(|| format!("held back from {}", doing()))),
            _ if self.running.contains_key(&n) => self.running.get(&n).cloned(),
            // Its replacement being drafted (from anywhere): nothing on it meanwhile.
            _ => self.drafting_replacement(n).then(|| "drafting a replacement".to_owned()),
        }
    }

    /// Claude's runs from any dv, from their markers, every couple of seconds: only those still
    /// at it (their process, or their zellij pane, still there). Returns whether that changed.
    pub(super) fn watch_running(&mut self) -> bool {
        if self.running_checked.is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(2)) {
            return false;
        }
        self.running_checked = Some(std::time::Instant::now());
        let Some(dir) = self.reviews_dir.as_ref().and_then(|d| d.parent()).map(|d| d.join("running")) else { return false };
        let panes = std::cell::OnceCell::new(); // zellij's panes, looked up only if a run is in one
        // (PRs' and issues' runs; replacements' and new PRs' drafts and runs: their claims.)
        let all = crate::claude::sweep_running(&dir, &panes);
        let of = |rep| all.iter().filter_map(move |(k, w)| Some((crate::claude::key_n(k, rep)?, w.clone())));
        let (mut now, mut reps): (HashMap<u64, String>, HashMap<u64, String>) = (of(false).collect(), of(true).collect());
        // Claude Code in a pane in a PR's worktree, or a replacement's (one you started, from a
        // shell even: its screen says): at it there, or asking you something.
        let rep_dirs: Vec<(u64, std::path::PathBuf)> =
            self.rep_of.keys().filter_map(|&id| Some((id, crate::worktree_dir_named(&self.root, &crate::replace::name(id)).ok()?))).collect();
        if crate::view::in_zellij() && (!self.local.is_empty() || !rep_dirs.is_empty()) {
            for p in panes.get_or_init(crate::pane::open) {
                let pr = self.local.iter().find(|(_, (d, _))| p.2.starts_with(d)).map(|(n, _)| *n).filter(|n| !now.contains_key(n));
                let rep = rep_dirs.iter().find(|(_, d)| p.2.starts_with(d)).map(|(r, _)| *r);
                if pr.is_none() && rep.is_none() {
                    continue;
                }
                let Some(doing) = crate::pane::claude_doing(p) else { continue };
                match (pr, rep) {
                    (Some(n), _) => now.insert(n, doing.into()),
                    (_, Some(r)) => reps.insert(r, doing.into()),
                    _ => None,
                };
            }
        }
        let changed = now != self.running || reps != self.running_reps;
        (self.running, self.running_reps) = (now, reps);
        // Runs that found Claude at it: once it isn't (on the PR, nor where it'd work), yours
        // asked again as you asked them; auto's, decided again by auto from how things are now
        // (off, or done meanwhile: not run): a change as any here.
        let free: Vec<u64> = self.batch.iter().filter(|(n, s)| matches!(s, batch::Status::Busy(_, d) if !batch::held(**n, d.as_deref()))).map(|(&n, _)| n).collect();
        for &n in &free {
            let task = self.batch_task.get(&n).cloned().unwrap_or(batch::Task::Review);
            match self.batch_extra.get(&n).cloned().flatten() {
                Some(extra) => _ = self.start_batch(vec![(n, task)], Some(extra)),
                None => {
                    self.batch.remove(&n);
                    self.auto_done.retain(|d| d.0 != n || d.2 != task.doing());
                }
            }
        }
        changed || !free.is_empty()
    }

    /// `s` on the selected todo item: bring it back if it's deferred, else ask for how long.
    pub(super) fn defer_key(&mut self) {
        let Some(it) = self.selected_item() else { return };
        let what = it.label();
        let (changed, said) = self.todos.defer_key(&it, &what);
        self.msg = said;
        if changed {
            self.save_defer();
        }
    }

    pub(super) fn save_defer(&mut self) {
        if let Some(e) = self.todos.save() {
            self.msg = e;
        }
        // (You moved it: the cursor stays put, on what's next.)
        self.find_todo();
    }

    /// What Claude would do on PR `n` from the todo tab (`a a`): what's due there, else review.
    pub(super) fn todo_task(&self, n: u64) -> batch::Task {
        let it = self.todos.items.iter().find(|it| !it.is_rep() && it.number == n);
        // Claude found its failing CI isn't its doing: `a a` has it look again all the same;
        // not over answering you or updating its review, nor over changes waiting to be pushed.
        let retry = it.is_some_and(|it| {
            it.needs.iter().any(|n| matches!(n, todo::Need::CiNotThisPr(..)))
                && !it.needs.iter().any(|n| matches!(n, todo::Need::Unpushed(..) | todo::Need::ClaudeChanged(..)))
                && !matches!(it.claude, Some(batch::Task::Answer { .. } | batch::Task::Update(_)))
        });
        if retry {
            return batch::Task::FixChecks;
        }
        it.and_then(|it| it.claude.clone()).unwrap_or(batch::Task::Review)
    }
}

/// Where the heads PRs' descriptions were checked at are kept (none from tests).
fn described_file() -> Option<std::path::PathBuf> {
    crate::claude::dv_dir().ok().map(|d| d.join("described.json")).filter(|_| !cfg!(test))
}

/// The heads PRs' descriptions were checked at, as saved (`described.json`), if it's there.
fn read_described() -> Option<HashMap<u64, String>> {
    serde_json::from_str(&std::fs::read_to_string(described_file()?).ok()?).ok()
}
