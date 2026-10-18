//! The todo tab: what needs you (`crate::todo`), in order, and Claude's part in it.

use super::*;
use crate::todo::{self, Facts, Item};

/// How many of the top items Claude does its part on (each a Claude run), after a ranking.
const TODO_CLAUDE: usize = 3;

/// A ranking older than this is redone when you open the tab.
const RANK_EVERY: i64 = 12 * 3600;

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
                items.push(Item { rep: false, number: p.number, needs, claude, updated: p.updated.clone(), reason: None });
            }
        }
        for r in self.rep_of.values() {
            let needs = todo::rep_needs(r);
            let updated = self.reps.iter().find(|c| c.number == r.id).map_or(String::new(), |c| c.updated.clone());
            if !needs.is_empty() {
                items.push(Item { rep: true, number: r.id, needs, claude: None, updated, reason: None });
            }
        }
        (self.todo, self.todo_stale) = todo::order(items, self.ranked.as_ref());
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

    /// Have Claude rank what needs you, if it's due (`R` on the tab, or the ranking is old or
    /// missing when you open it) and what's needed is in (GitHub's part).
    pub(super) fn maybe_rank(&mut self, force: bool) {
        let old = self.ranked.as_ref().is_none_or(|r| crate::claude::now_secs() - r.at > RANK_EVERY);
        if self.rank_rx.is_some() || self.todo.is_empty() || !(force || old) {
            return;
        }
        if self.extra_rx.is_some() || self.checks.is_some() {
            return self.want_rank = true; // once they're in
        }
        self.want_rank = false;
        let lines: Vec<String> = self
            .todo_shown()
            .into_iter()
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

    /// News from the background: GitHub's part (then what needs you again), and a ranking
    /// (then Claude starts on its part of the top items).
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
            self.maybe_rank(true);
        }
        let Some(Ok(res)) = self.rank_rx.as_ref().map(Receiver::try_recv) else { return };
        self.rank_rx = None;
        match res {
            Ok(r) => {
                self.ranked = Some(r);
                self.find_todo();
                self.msg = format!("Claude ranked what needs you{}", self.queue_claude());
            }
            Err(e) => self.msg = format!("ranking: {e:#} (R tries again)"),
        }
    }

    /// Claude's part on the top items: a review, an update to one, answers to your replies.
    /// Returns what to add to the message.
    fn queue_claude(&mut self) -> String {
        let busy = |n: &u64| self.batch.get(n).is_some_and(batch::Status::running);
        let jobs: Vec<(u64, bool, batch::Task)> = self
            .todo
            .iter()
            .filter(|it| !it.rep && !busy(&it.number))
            .filter_map(|it| Some((it.number, self.local.get(&it.number).is_some_and(|w| w.1.is_some()), it.claude.clone()?)))
            .take(TODO_CLAUDE)
            .collect();
        if jobs.is_empty() {
            return String::new();
        }
        let what: Vec<String> = jobs.iter().map(|(n, _, t)| format!("{} #{n}", t.doing())).collect();
        jobs.iter().for_each(|(n, _, t)| _ = self.batch_task.insert(*n, t.clone()));
        self.batch_rx.push(batch::start(jobs, self.root.clone(), crate::claude::instructions("review")));
        format!("; Claude is {} first", what.join(", "))
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
