//! The todo tab: what needs you (`crate::todo`), in order, and Claude's part in it.

use super::*;
use crate::todo::{self, Facts, Item};

impl List {
    /// What needs you, from what's known now, in Claude's order (where it ranked them).
    /// (The cursor stays on the card it's on, wherever the list puts it now: `keeping_sel`.)
    pub(super) fn find_todo(&mut self) {
        self.keeping_sel(Self::rebuild_todo);
    }

    /// The todo items, and what Claude would close, from the reviews and PRs as they are.
    fn rebuild_todo(&mut self) {
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
        for p in self.prs.iter().filter(|p| p.open()) {
            let file = dir.as_ref().map(|d| d.join(format!("pr-{}.json", p.number)));
            let review = file.as_ref().and_then(|f| crate::claude::load(f).ok().flatten());
            if let Some(r) = review.as_ref().filter(|r| !r.close.trim().is_empty()) {
                // (Reviewed at an older head: as the todo tab has it, its review's to update.)
                let head = self.extra.get(&p.number).map_or("", |e| e.head_oid.as_str());
                let older = !r.head.is_empty() && !head.is_empty() && r.head != head;
                closable.insert(p.number, format!("{}{}", r.close.trim(), if older { " (reviewed at an older head)" } else { "" }));
            }
            let looked = seen.as_ref().and_then(|s| std::fs::read_to_string(s.join(format!("pr-{}.looked", p.number))).ok()?.trim().parse().ok());
            let (f, c, head_at) = self.unpushed.get(&p.number).copied().unwrap_or((0, 0, None));
            let ci_note = ci.as_ref().and_then(|d| crate::claude::ci_note_at(&d.join(format!("pr-{}.json", p.number))));
            let facts = Facts { pr: p, extra: self.extra.get(&p.number), me: &self.me, review: review.as_ref(), looked, review_changed: file.and_then(secs), unpushed: (f, c), head_at, ci_note: ci_note.as_ref() };
            let (needs, claude) = todo::needs(&facts);
            if !needs.is_empty() {
                items.push(Item { rep: false, number: p.number, needs, claude, updated: p.updated.clone(), reason: None, changed: false, deferred: None, busy: None, waits: None });
            }
        }
        self.closable = closable;
        for r in self.rep_of.values() {
            let needs = todo::rep_needs(r);
            let updated = self.reps.iter().find(|c| c.number == r.id).map_or(String::new(), |c| c.updated.clone());
            if !needs.is_empty() {
                items.push(Item { rep: true, number: r.id, needs, claude: None, updated, reason: None, changed: false, deferred: None, busy: None, waits: None });
            }
        }
        let (items, since) = todo::order(items, self.ranked.as_ref());
        let now = crate::claude::now_secs();
        // What deferrals waited for that has happened: they're over (said so, once).
        let met: Vec<(String, String)> = items
            .iter()
            .filter_map(|it| Some((it, self.deferred.get(&it.key())?.cond.as_ref()?)))
            .filter(|(it, c)| self.happened(it, c))
            .map(|(it, c)| (it.key(), format!("#{} is back: {}", it.number, c.says())))
            .collect();
        for (key, said) in &met {
            self.deferred.remove(key);
            self.msg = said.clone();
        }
        if !met.is_empty() {
            _ = todo::save_deferred(self.deferred_file.as_deref(), &self.deferred);
        }
        let mut items = todo::defer(items, &self.deferred, now, |it, c| self.happened(it, c));
        // What a PR's conversation says it waits on (a trusted comment), not happened yet: to
        // the bottom too, for everyone.
        for it in items.iter_mut().filter(|it| !it.rep && it.deferred.is_none()) {
            let said = self.extra.get(&it.number).map(|e| e.said.as_slice()).unwrap_or(&[]);
            // A comment's word, else what Claude saw in the conversation when it ranked.
            let claude = self.ranked.as_ref().and_then(|r| r.waits.get(&it.key())).map(|c| (c.clone(), String::new()));
            it.waits = todo::waiting_on(said, now).or(claude).filter(|(c, _)| !self.happened(it, c));
        }
        let (mut waiting, mut rest): (Vec<Item>, Vec<Item>) = items.into_iter().partition(|it| it.waits.is_some());
        rest.append(&mut waiting);
        let items = rest;
        // Those Claude is at (here or in any dv; drafting their replacement too): at the bottom
        // till it's done.
        let doing = |it: &Item| match it.rep {
            // A replacement: Claude drafting it, or at it in its worktree.
            true => self.drafting_replacement(it.number).then(|| "drafting it".to_owned()).or_else(|| self.running_reps.get(&it.number).cloned()),
            false => self.claude_doing(it.number).or_else(|| self.drafting_replacement(it.number).then(|| "drafting a replacement".to_owned())),
        };
        let (mut busy, todo): (Vec<Item>, Vec<Item>) = items.into_iter().partition(|it| doing(it).is_some());
        busy.iter_mut().for_each(|it| it.busy = doing(it));
        // A replacement drafted, ready for you to review: first.
        let (mut ready, todo): (Vec<Item>, Vec<Item>) = todo.into_iter().partition(|it| it.rep && it.needs.contains(&todo::Need::Publish));
        // CI still running on it (a replacement's: on its PR): what it needs mostly waits on
        // that (merging, marking it ready, closing what it replaces), so after what you can do
        // now, before what you've put aside or wait on, and what Claude's at. Not with checks
        // failing already: those can be fixed now. Back up once CI's done.
        let ci_running = |it: &Item| {
            let n = if it.rep { self.rep_of.get(&it.number).and_then(|r| r.published) } else { Some(it.number) };
            n.and_then(|n| self.prs.iter().find(|p| p.number == n)).and_then(|p| p.checks).is_some_and(|c| c.2 > 0 && c.1 == 0)
        };
        let (mut later, todo): (Vec<Item>, Vec<Item>) = todo.into_iter().partition(|it| it.deferred.is_some() || it.waits.is_some());
        let (mut ci, mut todo): (Vec<Item>, Vec<Item>) = todo.into_iter().partition(ci_running);
        ready.append(&mut todo);
        ready.append(&mut ci);
        ready.append(&mut later);
        ready.append(&mut busy);
        (self.todo, self.since) = (ready, since);
        self.auto_run();
    }

    /// Auto (`C`): Claude does its part on each item, as `a a` would have it (fixing your CI,
    /// reviewing, updating its review, answering you), each once a head (an answer, once a
    /// change to its review file); green drafts of yours are marked ready for review; a
    /// published replacement, green, closes what it replaces; and what needs you is ranked again
    /// as it changes. Not what you've put aside or wait on, not where Claude's at it already.
    fn auto_run(&mut self) {
        if !self.auto {
            return;
        }
        let todo: Vec<Item> = self.todo.iter().filter(|it| it.busy.is_none() && it.deferred.is_none() && it.waits.is_none()).cloned().collect();
        // A published replacement, green, nothing of it unpushed: what it replaces is closed,
        // with its closing note (their items aren't Claude's to review any more, then).
        let mut close = vec![];
        for it in todo.iter().filter(|it| it.rep && it.needs.iter().any(|n| matches!(n, todo::Need::CloseOriginals(_)))) {
            let Some(r) = self.rep_of.get(&it.number) else { continue };
            let Some(p) = r.published.and_then(|n| self.prs.iter().find(|p| p.number == n)) else { continue };
            if !p.checks_head.is_empty() && todo::green(p.checks) && self.auto_done.insert((it.number, p.checks_head.clone(), "close")) {
                close.push(r.clone());
            }
        }
        let closing: HashSet<u64> = close.iter().flat_map(|r| r.replaces.iter().map(|o| o.number)).collect();
        let (mut run, mut ready) = (vec![], vec![]);
        // One batch of Claude's at a time, of at most as many as run at once: the cap holds, and
        // the rest wait for a later look (most pressing first, in the todo's order).
        let batch_busy = self.batch.values().any(batch::Status::running);
        for it in todo.iter().filter(|it| !it.rep && !closing.contains(&it.number)) {
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
        let mut said = vec![];
        if !run.is_empty() {
            said.push(format!("Claude is {}", self.start_batch(run, String::new())));
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
                Ok(done) => format!("closed {} (replaced by #{n}, its CI passed)", done.iter().map(|o| format!("#{o}")).collect::<Vec<_>>().join(", ")),
                Err(e) => format!("not closing what #{n} replaces: {e:#}"),
            });
            self.load_reps();
        }
        // Ranked again once something's new or changed, at most every quarter hour (a run each,
        // tried or not: a failing one isn't run over and over).
        let due = self.auto_ranked.is_none_or(|t| t.elapsed() > std::time::Duration::from_secs(15 * 60));
        let changed = self.ranked.is_none() || self.since.new + self.since.changed > 0;
        if due && changed && self.rank_rx.is_none() && !self.want_rank && !self.todo.is_empty() {
            self.auto_ranked = Some(std::time::Instant::now());
            self.rank_now();
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
        let dir = crate::worktree_dir_named(&self.root, &crate::replace::name(r.id))?;
        let d = dir.to_string_lossy().into_owned();
        let git = |args: &[&str]| crate::diff::git(&[&["-C", d.as_str()], args].concat()).unwrap_or_default();
        anyhow::ensure!(git(&["status", "--porcelain"]).trim().is_empty(), "its worktree has uncommitted changes");
        anyhow::ensure!(git(&["rev-list", "--count", "@{upstream}..HEAD"]).trim() == "0", "its worktree has commits not pushed");
        let n = r.published.unwrap_or(0);
        anyhow::ensure!(todo::green(Some(crate::gh::checks(n)?)), "#{n}'s CI isn't all green on GitHub now");
        Ok(())
    }

    /// The todo tab's items with their cards (a PR's, or a replacement's).
    pub(super) fn todo_shown(&self) -> Vec<(&Listed, &Item)> {
        self.todo
            .iter()
            .filter_map(|it| {
                let cards = if it.rep { &self.reps } else { &self.prs };
                Some((cards.iter().find(|c| c.number == it.number)?, it))
            })
            .collect()
    }

    /// Have Claude rank what needs you (`r r`: a Claude run, so only when you ask), once what's
    /// needed is in (GitHub's part).
    pub(super) fn rank_now(&mut self) {
        if self.rank_rx.is_some() || self.todo.is_empty() {
            return;
        }
        if self.extra_rx.is_some() || self.checks.is_some() {
            self.msg = "Claude ranks what needs you once GitHub's part is in…".into();
            return self.want_rank = true;
        }
        self.want_rank = false;
        let lines: Vec<String> = self
            .todo_shown()
            .into_iter()
            .filter(|(_, it)| it.deferred.is_none()) // not ranked while deferred
            .map(|(card, it)| {
                let summary = match it.rep {
                    true => self.rep_of.get(&it.number).map(|r| r.body.clone()).unwrap_or_default(),
                    false => self.reviews_dir.as_ref().and_then(|d| crate::claude::load(&d.join(format!("pr-{}.json", it.number))).ok().flatten()).map(|r| r.summary).unwrap_or_default(),
                };
                todo::about(it, card, &summary)
            })
            .collect();
        self.rank_rx = Some(todo::rank(lines, todo::fingerprint(&self.todo), self.root.clone()));
        self.msg = format!("Claude is ranking the {} PR(s) that need you…", self.todo.len());
    }

    /// What `r` says before `r` again ranks: what it'd cost, and what's changed since the last
    /// ranking, to judge whether it's worth it.
    pub(super) fn rank_offer(&self) -> String {
        let last = match &self.ranked {
            Some(r) => format!("the last, {} ago, {}", crate::gh::ago_secs(crate::claude::now_secs() - r.at), cost_text(r.cost)),
            None => "never ranked before".into(),
        };
        let since = self.since_text();
        format!("r again: Claude ranks the {} PR(s) that need you, one run ({last}{since}); nothing else starts", self.todo.len())
    }

    /// What's changed since the last ranking, for the header and `r`: ", since: 3 new, 1 changed, 2 seen to".
    pub(super) fn since_text(&self) -> String {
        let s = self.since;
        let parts: Vec<String> = [(s.new, "new"), (s.changed, "changed"), (s.gone, "seen to")].iter().filter(|p| p.0 > 0).map(|(n, w)| format!("{n} {w}")).collect();
        match (self.ranked.is_some(), parts.is_empty()) {
            (false, _) => String::new(),
            (true, true) => ", nothing's changed since".into(),
            (true, false) => format!(", since: {}", parts.join(", ")),
        }
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
                Err(e) => self.msg = format!("what needs you: {e:#}"),
            }
            self.find_todo();
        }
        if self.want_rank {
            self.rank_now();
        }
        let Some(Ok(res)) = self.rank_rx.as_ref().map(Receiver::try_recv) else { return };
        self.rank_rx = None;
        match res {
            Ok(r) => {
                self.ranked = Some(r);
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
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let found = local
                .into_iter()
                .filter_map(|(n, d)| {
                    let git = |args: &[&str]| crate::diff::git(&[&["-C", d.as_str()], args].concat()).ok();
                    let files = git(&["status", "--porcelain"])?.lines().count();
                    // Beyond the PR's head (as GitHub has it), if that's here; else its upstream's.
                    let head = heads.get(&n).cloned().filter(|h| git(&["cat-file", "-e", &format!("{h}^{{commit}}")]).is_some());
                    let theirs = head.unwrap_or_else(|| "@{upstream}".into());
                    // Still at what dv checked out, the PR moving on since (a force-push): behind,
                    // not work of yours.
                    let here = git(&["rev-parse", "HEAD"]).unwrap_or_default();
                    let stale = crate::last_checkout(n).is_some_and(|c| c == here.trim());
                    let commits = match stale {
                        true => 0,
                        false => git(&["rev-list", "--count", &format!("{theirs}..HEAD")]).and_then(|c| c.trim().parse().ok()).unwrap_or(0),
                    };
                    // When the PR's head was made: Claude's changes after it are what's waiting.
                    let at = git(&["log", "-1", "--format=%ct", &theirs]).and_then(|t| t.trim().parse().ok());
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

    /// Whether what deferral condition `c` waits for has happened, for item `it`.
    pub(super) fn happened(&self, it: &Item, c: &todo::Until) -> bool {
        let pr = self.prs.iter().find(|p| p.number == it.number);
        let state = |n: u64| self.prs.iter().find(|p| p.number == n).map(|p| p.state.clone()).or_else(|| self.others.get(&n).cloned());
        let reviews = self.extra.get(&it.number).map(|e| e.reviews.as_slice()).unwrap_or(&[]);
        match c {
            todo::Until::Time(t) => crate::claude::now_secs() >= *t,
            todo::Until::Merged(n) => state(*n).is_some_and(|s| s == "MERGED"),
            todo::Until::Closed(n) => state(*n).is_some_and(|s| s == "MERGED" || s == "CLOSED"),
            todo::Until::Green => todo::green(pr.and_then(|p| p.checks)),
            todo::Until::Approved(None) => pr.is_some_and(|p| p.decision == "APPROVED") || reviews.iter().any(|r| r.1 == "APPROVED"),
            todo::Until::Approved(Some(who)) => reviews.iter().any(|r| &r.0 == who && r.1 == "APPROVED"),
            todo::Until::Reviewed(who) => reviews.iter().any(|r| &r.0 == who),
        }
    }

    /// `s` then `u`: defer the item until what you write happens (`key`: the item's). dv reads
    /// the usual phrasings itself; the rest Claude puts in a form dv can watch for (one small
    /// run, in the background: `until_news`).
    pub(super) fn defer_until(&mut self, key: String, text: &str) {
        match todo::Until::parse(text, crate::claude::now_secs()) {
            Some(c) => self.defer_on(&key, c),
            None => {
                self.msg = "Claude is reading when you mean…".into();
                self.until_rx = Some((key, todo::interpret(text.trim().to_owned(), self.root.clone())));
            }
        }
    }

    fn defer_on(&mut self, key: &str, c: todo::Until) {
        let Some(it) = self.todo.iter().find(|it| it.key() == key).cloned() else { return };
        let (until, cond) = match c {
            todo::Until::Time(t) => (t, None),
            c => (i64::MAX, Some(c)),
        };
        let says = cond.as_ref().map_or_else(|| todo::in_secs(until - crate::claude::now_secs()), todo::Until::says);
        self.deferred.insert(key.to_owned(), todo::Deferral { until, needs: it.needs_key(), cond });
        self.msg = format!("deferred until {says} (s on it brings it back)");
        // A PR it waits on that isn't listed: its state, now (and on refresh).
        self.find_others();
        self.save_defer();
    }

    /// Claude's reading of a deferral's "until …", when it comes.
    pub(super) fn until_news(&mut self) {
        let Some(Ok(res)) = self.until_rx.as_ref().map(|(_, rx)| rx.try_recv()) else { return };
        let Some((key, _)) = self.until_rx.take() else { return };
        match res {
            Ok(c) => self.defer_on(&key, c),
            Err(e) => self.msg = format!("not deferred: {e:#}"),
        }
    }

    /// The state of PRs that deferrals wait on and that aren't listed (merged or closed ones
    /// aren't), from GitHub.
    pub(super) fn find_others(&mut self) {
        let waited: Vec<u64> = self
            .deferred
            .values()
            .filter_map(|d| match d.cond {
                Some(todo::Until::Merged(n) | todo::Until::Closed(n)) => Some(n),
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
        self.todo.iter().filter(|it| it.deferred.is_none() && it.busy.is_none() && it.waits.is_none()).count()
    }

    /// What Claude is doing on PR `n` now, if anything: a run from here (`a a`), or any dv's
    /// (its marker in dv's directory, `running`).
    /// Claude is drafting a replacement of PR `n` (here, or in another dv).
    pub(super) fn drafting_replacement(&self, n: u64) -> bool {
        self.rep_of.values().filter(|r| r.id == n || r.replaces.iter().any(|o| o.number == n)).any(|r| {
            self.drafting.get(&r.id).is_some_and(batch::Status::running) || self.drafting_elsewhere.contains(&r.id)
        })
    }

    pub(super) fn claude_doing(&self, n: u64) -> Option<String> {
        match self.batch.get(&n) {
            Some(s) if s.running() => Some(self.batch_task.get(&n).map_or("reviewing", batch::Task::doing).to_owned()),
            _ => self.running.get(&n).cloned(),
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
        // A run in a pane is at it while Claude Code works there (or asks you something), not
        // just while its pane is open: done, it waits there idle.
        let alive = |pid: Option<u32>, session: Option<&str>| match (pid, session) {
            (Some(pid), _) => std::process::Command::new("kill").args(["-0", &pid.to_string()]).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success()),
            (None, Some(id)) => panes.get_or_init(open_panes).iter().find(|p: &&Pane| p.1.contains(id)).is_some_and(|p| {
                let screen = crate::pane::screen(p.0);
                crate::pane::working(&screen) || crate::pane::asking(&screen)
            }),
            _ => false,
        };
        let mut now = crate::claude::running(&dir, alive);
        let mut reps = HashMap::new();
        // Claude Code in a pane in a PR's worktree, or a replacement's (one you started, from a
        // shell even: its screen says): at it there, or asking you something.
        let rep_dirs: Vec<(u64, std::path::PathBuf)> =
            self.rep_of.keys().filter_map(|&id| Some((id, crate::worktree_dir_named(&self.root, &crate::replace::name(id)).ok()?))).collect();
        if crate::view::in_zellij() && (!self.local.is_empty() || !rep_dirs.is_empty()) {
            for (id, _, cwd) in panes.get_or_init(open_panes) {
                let pr = self.local.iter().find(|(_, (d, _))| cwd.starts_with(d)).map(|(n, _)| *n).filter(|n| !now.contains_key(n));
                let rep = rep_dirs.iter().find(|(_, d)| cwd.starts_with(d)).map(|(r, _)| *r);
                if pr.is_none() && rep.is_none() {
                    continue;
                }
                let screen = crate::pane::screen(*id);
                let doing = match (crate::pane::asking(&screen), crate::pane::working(&screen)) {
                    (true, _) => "asking you (in its pane)",
                    (_, true) => "working (in its pane)",
                    _ => continue,
                };
                match (pr, rep) {
                    (Some(n), _) => now.insert(n, doing.into()),
                    (_, Some(r)) => reps.insert(r, doing.into()),
                    _ => None,
                };
            }
        }
        // A pane dv started, Claude done there: closed (`close_if_done`), its run over (its
        // session stays, for the next ask to resume). Every marker of a pane run is looked at
        // (a cheap read of its transcript's end), so one not closed at first (you were on it)
        // is later.
        for (n, session, since) in crate::claude::pane_runs(&dir).into_iter().filter(|r| !now.contains_key(&r.0)) {
            if let Some(p) = panes.get_or_init(open_panes).iter().find(|p| p.1.contains(&session))
                && crate::pane::close_if_done(p.0, &session, since)
            {
                crate::claude::clear_running(n);
            }
        }
        let changed = now != self.running || reps != self.running_reps;
        (self.running, self.running_reps) = (now, reps);
        changed
    }

    /// `s` on the selected todo item: bring it back if it's deferred, else ask for how long.
    pub(super) fn defer_key(&mut self) {
        let Some(it) = self.selected_item() else { return };
        let what = if it.rep { crate::replace::label(it.number) } else { format!("#{}", it.number) };
        if it.deferred.is_some() {
            self.deferred.remove(&it.key());
            self.msg = format!("{what} is back");
            return self.save_defer();
        }
        self.msg = format!("defer {what} for: 1 an hour, 4 four hours, d a day, w a week, or u until something happens (any other key: not); it's back sooner if what it needs changes");
        self.deferring = Some(it.key());
    }

    /// Defer the item `s` asked about (its key) for `c`'s time.
    pub(super) fn defer_for(&mut self, key: &str, c: char) {
        let Some(it) = self.todo.iter().find(|it| it.key() == key).cloned() else { return };
        let (secs, how) = match c {
            '1' => (3600, "an hour"),
            '4' => (4 * 3600, "four hours"),
            'd' => (86400, "a day"),
            _ => (7 * 86400, "a week"),
        };
        self.deferred.insert(it.key(), todo::Deferral { until: crate::claude::now_secs() + secs, needs: it.needs_key(), cond: None });
        self.msg = format!("deferred for {how} (s on it brings it back)");
        self.save_defer();
    }

    pub(super) fn save_defer(&mut self) {
        if let Err(e) = todo::save_deferred(self.deferred_file.as_deref(), &self.deferred) {
            self.msg = format!("couldn't save it: {e:#}");
        }
        // (You moved it: the cursor stays put, on what's next.)
        self.rebuild_todo();
    }

    /// What Claude would do on PR `n` from the todo tab (`a a`): what's due there, else review.
    pub(super) fn todo_task(&self, n: u64) -> batch::Task {
        let it = self.todo.iter().find(|it| !it.rep && it.number == n);
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

/// A ranking's cost: "cost $0.38" (one from before dv kept it: not recorded).
pub(super) fn cost_text(c: f64) -> String {
    if c > 0.0 { format!("cost ${c:.2}") } else { "cost not recorded".into() }
}

/// A zellij pane: its id, command and working directory.
type Pane = (u64, String, std::path::PathBuf);

/// zellij's open panes (none outside zellij).
fn open_panes() -> Vec<Pane> {
    if !crate::view::in_zellij() {
        return vec![];
    }
    crate::pane::list(&["-c"])
        .unwrap_or_default()
        .iter()
        .filter(|p| p["exited"] != true && p["is_plugin"] != true)
        .filter_map(|p| Some((p["id"].as_u64()?, p["pane_command"].as_str()?.to_owned(), std::path::PathBuf::from(p["pane_cwd"].as_str().unwrap_or("")))))
        .collect()
}
