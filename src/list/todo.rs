//! The todo tab: what needs you (`crate::todo`), in order, and Claude's part in it.

use super::*;
use crate::todo::{self, Facts, Item};

impl List {
    /// What needs you, from what's known now, in Claude's order (where it ranked them).
    pub(super) fn find_todo(&mut self) {
        let dir = self.reviews_dir.clone();
        let seen = dir.as_ref().and_then(|d| d.parent()).map(|d| d.join("seen"));
        let secs = |p: std::path::PathBuf| -> Option<i64> {
            let t = std::fs::metadata(p).ok()?.modified().ok()?;
            Some(t.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs() as i64)
        };
        let mut items = vec![];
        for p in self.prs.iter().filter(|p| p.state == "OPEN" || p.state.is_empty()) {
            let file = dir.as_ref().map(|d| d.join(format!("pr-{}.json", p.number)));
            let review = file.as_ref().and_then(|f| crate::claude::load(f).ok().flatten());
            let looked = seen.as_ref().and_then(|s| std::fs::read_to_string(s.join(format!("pr-{}.looked", p.number))).ok()?.trim().parse().ok());
            let (f, c, head_at) = self.unpushed.get(&p.number).copied().unwrap_or((0, 0, None));
            let facts = Facts { pr: p, extra: self.extra.get(&p.number), me: &self.me, review: review.as_ref(), looked, review_changed: file.and_then(secs), unpushed: (f, c), head_at };
            let (needs, claude) = todo::needs(&facts);
            if !needs.is_empty() {
                items.push(Item { rep: false, number: p.number, needs, claude, updated: p.updated.clone(), reason: None, changed: false, deferred: None, busy: None, waits: None });
            }
        }
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
        // Those Claude is at (here or in any dv): at the bottom till it's done.
        let (mut busy, mut todo): (Vec<Item>, Vec<Item>) = items.into_iter().partition(|it| !it.rep && self.claude_doing(it.number).is_some());
        busy.iter_mut().for_each(|it| it.busy = self.claude_doing(it.number));
        todo.append(&mut busy);
        (self.todo, self.since) = (todo, since);
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
            Some(r) => format!("the last, {} ago, {}", ago_secs(crate::claude::now_secs() - r.at), cost_text(r.cost)),
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
            (true, true) => "; nothing's changed since".into(),
            (true, false) => format!("; since: {}", parts.join(", ")),
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
                let cost = r.cost;
                self.ranked = Some(r);
                self.find_todo();
                self.msg = format!("Claude ranked what needs you (${cost:.2}); a a on one has Claude do its part there");
            }
            Err(e) => self.msg = format!("ranking: {e:#} (r r tries again)"),
        }
    }

    /// What's in each PR worktree that isn't on GitHub: (changed files, untracked ones too;
    /// commits beyond the PR's head). Looked at on refresh, back from a PR, and when Claude's
    /// done somewhere (two git calls a worktree: not every frame).
    pub(super) fn scan_unpushed(&mut self) {
        self.unpushed = self
            .local
            .iter()
            .filter_map(|(&n, (dir, _))| {
                let d = dir.to_string_lossy().into_owned();
                let git = |args: &[&str]| crate::diff::git(&[&["-C", d.as_str()], args].concat()).ok();
                let files = git(&["status", "--porcelain"])?.lines().count();
                // Beyond the PR's head (as GitHub has it), if that's here; else its upstream's.
                let head = self.extra.get(&n).map(|e| e.head_oid.clone()).filter(|h| git(&["cat-file", "-e", &format!("{h}^{{commit}}")]).is_some());
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
            todo::Until::Green => pr.and_then(|p| p.checks).is_some_and(|(pass, fail, pending)| pass > 0 && fail == 0 && pending == 0),
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
            .filter(|n| !self.prs.iter().any(|p| p.number == *n))
            .collect();
        for n in waited {
            if let Ok(Some(s)) = gh::pr_state(n) {
                self.others.insert(n, s);
            }
        }
    }

    /// What needs you now (not deferred, Claude not at it), for the tab's count.
    pub(super) fn todo_now(&self) -> usize {
        self.todo.iter().filter(|it| it.deferred.is_none() && it.busy.is_none() && it.waits.is_none()).count()
    }

    /// What Claude is doing on PR `n` now, if anything: a run from here (`a a`), or any dv's
    /// (its marker in dv's directory, `running`).
    pub(super) fn claude_doing(&self, n: u64) -> Option<String> {
        match self.batch.get(&n) {
            Some(s) if s.running() => Some(self.batch_task.get(&n).map_or("reviewing", batch::Task::doing).to_owned()),
            _ => self.running.get(&n).cloned(),
        }
    }

    /// Claude's runs from any dv, from their markers, every couple of seconds: only those still
    /// at it (their process, or their zellij pane, still there). Returns whether that changed.
    pub(super) fn watch_running(&mut self) -> bool {
        if self.running_checked.elapsed() < std::time::Duration::from_secs(2) {
            return false;
        }
        self.running_checked = std::time::Instant::now();
        let Some(dir) = self.reviews_dir.as_ref().and_then(|d| d.parent()).map(|d| d.join("running")) else { return false };
        let panes = std::cell::OnceCell::new(); // zellij's panes, looked up only if a run is in one
        // A run in a pane is at it while Claude Code works there (or asks you something), not
        // just while its pane is open: done, it waits there idle.
        let alive = |pid: Option<u32>, session: Option<&str>| match (pid, session) {
            (Some(pid), _) => std::process::Command::new("kill").args(["-0", &pid.to_string()]).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success()),
            (None, Some(id)) => panes.get_or_init(open_panes).iter().find(|p: &&Pane| p.1.contains(id)).is_some_and(|p| {
                let screen = pane_screen(p.0);
                crate::view::working(&screen) || crate::view::asking(&screen)
            }),
            _ => false,
        };
        let mut now = crate::claude::running(&dir, alive);
        // Claude Code in a pane of its own in a PR's worktree (one you started, say): at it
        // there, or asking you something.
        if crate::view::in_zellij() && !self.local.is_empty() {
            for (id, cmd, cwd) in panes.get_or_init(open_panes) {
                let Some(n) = cmd.starts_with("claude").then(|| self.local.iter().find(|(_, (d, _))| cwd.starts_with(d)).map(|(n, _)| *n)).flatten() else { continue };
                if now.contains_key(&n) {
                    continue;
                }
                let screen = pane_screen(*id);
                if crate::view::asking(&screen) {
                    now.insert(n, "asking you (in its pane)".into());
                } else if crate::view::working(&screen) {
                    now.insert(n, "working (in its pane)".into());
                }
            }
        }
        let changed = now != self.running;
        self.running = now;
        changed
    }

    /// `s` on the selected todo item: bring it back if it's deferred, else ask for how long.
    pub(super) fn defer_key(&mut self) {
        let Some(it) = self.todo_shown().get(self.sel).map(|(_, it)| (*it).clone()) else { return };
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
        self.find_todo();
    }

    /// What Claude would do on PR `n` from the todo tab (`a a`): what's due there, else review.
    pub(super) fn todo_task(&self, n: u64) -> batch::Task {
        self.todo.iter().find(|it| !it.rep && it.number == n).and_then(|it| it.claude.clone()).unwrap_or(batch::Task::Review)
    }
}

/// A ranking's cost: "cost $0.38" (one from before dv kept it: not recorded).
pub(super) fn cost_text(c: f64) -> String {
    if c > 0.0 { format!("cost ${c:.2}") } else { "cost not recorded".into() }
}

/// `s` seconds as "40s", "3m", "2h", "5d".
pub(super) fn ago_secs(s: i64) -> String {
    match s.max(0) {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m", s / 60),
        s if s < 86400 => format!("{}h", s / 3600),
        s => format!("{}d", s / 86400),
    }
}

/// A zellij pane: its id, command and working directory.
type Pane = (u64, String, std::path::PathBuf);

/// zellij's open panes (none outside zellij).
fn open_panes() -> Vec<Pane> {
    if !crate::view::in_zellij() {
        return vec![];
    }
    let out = std::process::Command::new("zellij").args(["action", "list-panes", "-j", "-c"]).stdin(std::process::Stdio::null()).output();
    let panes: serde_json::Value = out.ok().and_then(|o| serde_json::from_slice(&o.stdout).ok()).unwrap_or_default();
    panes
        .as_array()
        .into_iter()
        .flatten()
        .filter(|p| p["exited"] != true && p["is_plugin"] != true)
        .filter_map(|p| Some((p["id"].as_u64()?, p["pane_command"].as_str()?.to_owned(), std::path::PathBuf::from(p["pane_cwd"].as_str().unwrap_or("")))))
        .collect()
}

/// What zellij pane `id` shows.
fn pane_screen(id: u64) -> String {
    let out = std::process::Command::new("zellij").args(["action", "dump-screen", "-p", &format!("terminal_{id}")]).stdin(std::process::Stdio::null()).output();
    out.map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default()
}
