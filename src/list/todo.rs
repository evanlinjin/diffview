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
            let facts = Facts { pr: p, extra: self.extra.get(&p.number), me: &self.me, review: review.as_ref(), looked, review_changed: file.and_then(secs) };
            let (needs, claude) = todo::needs(&facts);
            if !needs.is_empty() {
                items.push(Item { rep: false, number: p.number, needs, claude, updated: p.updated.clone(), reason: None, changed: false, deferred: None });
            }
        }
        for r in self.rep_of.values() {
            let needs = todo::rep_needs(r);
            let updated = self.reps.iter().find(|c| c.number == r.id).map_or(String::new(), |c| c.updated.clone());
            if !needs.is_empty() {
                items.push(Item { rep: true, number: r.id, needs, claude: None, updated, reason: None, changed: false, deferred: None });
            }
        }
        let (items, since) = todo::order(items, self.ranked.as_ref());
        (self.todo, self.since) = (todo::defer(items, &self.deferred, crate::claude::now_secs()), since);
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
                Ok((me, extra)) => (self.me, self.extra) = (me, extra),
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

    /// What needs you now (not deferred), for the tab's count.
    pub(super) fn todo_now(&self) -> usize {
        self.todo.iter().filter(|it| it.deferred.is_none()).count()
    }

    /// `s` on the selected todo item: bring it back if it's deferred, else ask for how long.
    pub(super) fn defer_key(&mut self) {
        let Some(it) = self.todo_shown().get(self.sel).map(|(_, it)| (*it).clone()) else { return };
        let what = if it.rep { format!("the replacement of #{}", it.number) } else { format!("#{}", it.number) };
        if it.deferred.is_some() {
            self.deferred.remove(&it.key());
            self.msg = format!("{what} is back");
            return self.save_defer();
        }
        self.msg = format!("defer {what} for: 1 an hour, 4 four hours, d a day, w a week (any other key: not); it's back sooner if what it needs changes");
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
        self.deferred.insert(it.key(), (crate::claude::now_secs() + secs, it.needs_key()));
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

/// `s` seconds from now, to the nearest unit: "40s", "3m", "4h", "2d" (four hours just set
/// reads 4h, not the 3h `ago_secs` would round it down to).
pub(super) fn in_secs(s: i64) -> String {
    match s.max(0) {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m", (s + 30) / 60),
        s if s < 86400 => format!("{}h", (s + 1800) / 3600),
        s => format!("{}d", (s + 43200) / 86400),
    }
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
