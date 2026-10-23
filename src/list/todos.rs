//! What needs you, as a list keeps it (dv list's PRs, the issues): the items in order, Claude's
//! ranking of them and the next on its way, what you've deferred (`s`) and Claude's reading of an
//! "until …", and when auto last ranked. What's the same in both lists; what an item waits for
//! and whether it's happened is each list's own.
use super::*;
use crate::todo::{self, Item, Of, Ranked, Until};

#[derive(Default)]
pub(super) struct Todos {
    /// What needs you, in order, and what's changed since Claude last ranked it.
    pub(super) items: Vec<Item>,
    pub(super) since: todo::Since,
    pub(super) ranked: Option<Ranked>,
    pub(super) rank_rx: Option<Receiver<Result<Ranked>>>,
    /// Items deferred (`s`), kept in dv's directory (`file`); the one `s` is asking how long for.
    pub(super) deferred: todo::Deferred,
    /// The deferrals as last read or saved: what's changed here since, put into the file as it
    /// is then (another dv list's kept).
    saved: todo::Deferred,
    pub(super) deferred_file: Option<PathBuf>,
    /// What items need is complete (all its sources in): what's deferred records it.
    pub(super) ready: bool,
    pub(super) deferring: Option<String>,
    /// Claude reading a deferral's "until …": the item's key, the "or" time, its answer.
    pub(super) until_rx: Option<(String, Option<i64>, Receiver<Result<Until>>)>,
    /// Auto (`C`, kept in git config `dv.auto`, one setting for both lists): Claude does its
    /// part as things come in, and ranks them again; when it last ranked.
    pub(super) auto: bool,
    pub(super) auto_ranked: Option<std::time::Instant>,
}

/// What's said of an item deferred once it's no longer on the list (seen to meanwhile).
pub(super) const GONE: &str = "not deferred: it's no longer on the list";

/// What the key after `s` did: deferred it (said so: to save), asked until what (the item's
/// key), not (said why), or called it off (Esc).
pub(super) enum After {
    Deferred(String),
    Until(String),
    Not(String),
    Off,
}

/// Claude's reading of an item's "until …": its key, the condition, the "or" time.
pub(super) type Read = (String, Until, Option<i64>);

/// What `Todos::until` made of an "until …": the condition to defer on, or Claude reading it
/// (what's said).
pub(super) enum Reading {
    Read(Until, Option<i64>),
    Asked(String),
}

impl Todos {
    /// The list of `of`'s: its last ranking and deferrals, as kept.
    pub(super) fn of(of: Of) -> Todos {
        let file = todo::deferred_path_of(of);
        let auto = crate::diff::git(&["config", "--get", "dv.auto"]).is_ok_and(|v| v.trim() == "true");
        let deferred = todo::load_deferred(file.as_deref());
        Todos { ranked: todo::load_of(of), saved: deferred.clone(), deferred, deferred_file: file, auto, ..Default::default() }
    }

    /// `C`: auto on / off, kept (for both lists).
    pub(super) fn toggle_auto(&mut self) {
        self.auto = !self.auto;
        _ = crate::diff::git(&["config", "dv.auto", if self.auto { "true" } else { "false" }]);
    }

    /// What's changed since the last ranking: ", since: 3 new, 1 changed, 2 seen to".
    pub(super) fn since_text(&self) -> String {
        let s = self.since;
        let parts: Vec<String> = [(s.new, "new"), (s.changed, "changed"), (s.gone, "seen to")].iter().filter(|p| p.0 > 0).map(|(n, w)| format!("{n} {w}")).collect();
        match (self.ranked.is_some(), parts.is_empty()) {
            (false, _) => String::new(),
            (true, true) => ", nothing's changed since".into(),
            (true, false) => format!(", since: {}", parts.join(", ")),
        }
    }

    /// How the todo tab is ordered, for the status bar: Claude ranking it, its last ranking (how
    /// old, what it cost, what's changed since), or the rules'.
    pub(super) fn order_text(&self) -> String {
        match (&self.rank_rx, &self.ranked) {
            (Some(_), _) => "Claude is ranking…".into(),
            (None, Some(r)) => format!("ranked {} ago, {}{}", crate::gh::ago_secs(crate::claude::now_secs() - r.at), cost_text(r.cost), self.since_text()),
            (None, None) => "in the rules' order".into(),
        }
    }

    /// What `r` says before `r` again ranks `what` ("PR(s)"): what it'd cost, and what's changed
    /// since the last ranking, to judge whether it's worth it; if it can't, why not.
    pub(super) fn rank_offer(&self, what: &str) -> Result<String, &'static str> {
        self.can_rank()?;
        let last = match &self.ranked {
            Some(r) => format!("the last, {} ago, {}", crate::gh::ago_secs(crate::claude::now_secs() - r.at), cost_text(r.cost)),
            None => "never ranked before".into(),
        };
        Ok(format!("r again: Claude ranks the {} {what} that need you, one run ({last}{}); nothing else starts", self.to_rank(), self.since_text()))
    }

    /// What needs you now: not deferred, Claude not at it, not waiting on something.
    pub(super) fn now(&self) -> usize {
        self.items.iter().filter(|it| it.deferred.is_none() && it.busy.is_none() && it.waits.is_none()).count()
    }

    /// `s` on `it` (`what` names it): brought back if deferred (`true`: changed, to save), else
    /// asked how long for.
    pub(super) fn defer_key(&mut self, it: &Item, what: &str) -> (bool, String) {
        if it.deferred.is_some() {
            self.forget_reading(&it.key());
            self.deferred.remove(&it.key());
            return (true, format!("{what} is back"));
        }
        // (What it needs not all in yet: deferred on part of it, it'd be back once the rest is.)
        if !self.ready {
            return (false, "once GitHub's part is in (a moment): s again".into());
        }
        self.deferring = Some(it.key());
        (false, format!("defer {what} for: 1 an hour, 4 four hours, d a day, w a week, or u until something happens (any other key: not); it's back sooner if what it needs changes"))
    }

    /// The key after `s` (`None`: `s` wasn't pressed): `1` / `4` / `d` / `w` defers it that
    /// long; `u` asks until what; any other, not.
    pub(super) fn deferring_key(&mut self, c: KeyCode) -> Option<After> {
        let key = self.deferring.take()?;
        Some(match c {
            KeyCode::Char(c @ ('1' | '4' | 'd' | 'w')) => {
                let (secs, how) = todo::defer_secs(c);
                match self.defer(&key, crate::claude::now_secs() + secs, None, 0) {
                    true => After::Deferred(format!("deferred for {how} (s on it brings it back)")),
                    false => After::Not(GONE.into()),
                }
            }
            KeyCode::Char('u') => After::Until(key),
            KeyCode::Esc => After::Off,
            _ => After::Not("not deferred".into()),
        })
    }

    /// Item `key` deferred till `until` (or `cond` happens: since `at`, by GitHub's clock);
    /// `false`: it's no longer listed (`GONE`).
    pub(super) fn defer(&mut self, key: &str, until: i64, cond: Option<Until>, at: i64) -> bool {
        let Some(needs) = self.items.iter().find(|it| it.key() == key).map(Item::needs_key) else { return false };
        self.forget_reading(key);
        self.deferred.insert(key.to_owned(), todo::Deferral { until, needs, cond, at });
        true
    }

    /// Claude's reading of an "until …" for `key`, if one's under way: dropped (what you've done
    /// since stands; its answer would undo it).
    fn forget_reading(&mut self, key: &str) {
        if self.until_rx.as_ref().is_some_and(|r| r.0 == key) {
            self.until_rx = None;
        }
    }

    /// Item `key` as you know it (`Item::label`).
    fn label(&self, key: &str) -> String {
        self.items.iter().find(|it| it.key() == key).map_or_else(|| key.to_owned(), Item::label)
    }

    /// Item `key` deferred until `c` (or the time `or`, whichever first; `at`: since when, by
    /// GitHub's clock): what's said, to save.
    pub(super) fn defer_on(&mut self, key: &str, c: Until, or: Option<i64>, at: i64) -> String {
        let (until, cond, says) = todo::until_or(c, or);
        match self.defer(key, until, cond, at) {
            true => format!("deferred until {says} (s on it brings it back)"),
            false => GONE.into(),
        }
    }

    /// What needs you, `items`, settled: in Claude's order where it ranked them; deferrals whose
    /// condition has happened (`met`) over (kept so; what to say of the last), the rest at the
    /// bottom till then; and what's changed since the ranking (`since`).
    /// Deferrals whose time is up are dropped too. (One whose item's needs changed is back
    /// while they differ: needs read before all of what they come from is in aren't a change.)
    pub(super) fn settle(&mut self, items: Vec<Item>, met: Vec<(String, String)>) -> (Vec<Item>, Option<String>) {
        let items = todo::order(items, self.ranked.as_ref());
        let before = self.deferred.len();
        met.iter().for_each(|(key, _)| _ = self.deferred.remove(key));
        let now = crate::claude::now_secs();
        self.deferred.retain(|_, d| d.until > now);
        if self.deferred.len() != before {
            _ = self.save();
        }
        // (What's met is over: what's deferred still waits.)
        let items = todo::defer(items, &self.deferred, crate::claude::now_secs());
        self.since = todo::since_of(&items, self.ranked.as_ref());
        (items, met.into_iter().last().map(|m| m.1))
    }

    /// The deferrals of `items` whose condition has happened (`happened`: for an item, what it
    /// waits for, since when), for `settle`: their keys, and what to say ("#12 is back: …").
    pub(super) fn met(&self, items: &[Item], happened: impl Fn(&Item, &Until, i64) -> bool) -> Vec<(String, String)> {
        items
            .iter()
            .filter_map(|it| Some((it, self.deferred.get(&it.key())?)))
            .filter_map(|(it, d)| Some((it, d.cond.as_ref()?, d.at)))
            .filter(|(it, c, at)| happened(it, c, *at))
            .map(|(it, c, _)| (it.key(), format!("{} is back: {}", it.label(), c.says())))
            .collect()
    }

    /// Whether a deferral's time is up (`now`): the items to settle again.
    pub(super) fn due(&self, now: i64) -> bool {
        self.deferred.values().any(|d| d.until <= now) || self.items.iter().any(|it| it.waits.as_ref().is_some_and(|w| matches!(w.0, Until::Time(t) if t <= now)))
    }

    /// The deferrals, kept (an error: what's said).
    /// Deferrals of what's gone (merged, closed; a draft removed) dropped, not kept for good:
    /// those `keep` says no to. Saved if any were (a failure said).
    pub(super) fn prune(&mut self, keep: impl Fn(&str) -> bool) -> Option<String> {
        let before = self.deferred.len();
        self.deferred.retain(|k, _| keep(k));
        (self.deferred.len() != before).then(|| self.save()).flatten()
    }

    pub(super) fn save(&mut self) -> Option<String> {
        // What's changed here since, into the file as it is now (as marks are merged); as
        // saved only once it is (failed: still to save, next time).
        match todo::save_deferred(self.deferred_file.as_deref(), &self.saved, &self.deferred) {
            Ok(all) => {
                (self.deferred, self.saved) = (all.clone(), all);
                None
            }
            Err(e) => Some(format!("couldn't save it: {e:#}")),
        }
    }

    /// "until `text`" for item `key` ("…, or 2 weeks": whichever first): read here, or Claude
    /// reads it (`about`: what the item is, and what it can wait for), in the background
    /// (`until_news`).
    pub(super) fn until(&mut self, key: String, text: &str, about: impl FnOnce() -> String, root: &std::path::Path) -> Reading {
        let now = crate::claude::now_secs();
        let (text, or) = todo::or_time(text, now);
        match Until::parse(&text, now) {
            Some(c) => Reading::Read(c, or),
            None => {
                // (One read meanwhile: this one instead, said so; a reading that hangs isn't
                // in the way of the next.)
                let instead = self.until_rx.as_ref().map_or(String::new(), |r| format!(" (instead of the \"until\" for {})", self.label(&r.0)));
                self.until_rx = Some((key, or, todo::interpret(text, about(), root.to_path_buf())));
                Reading::Asked(format!("Claude is reading when you mean…{instead}"))
            }
        }
    }

    /// Claude's reading of an "until …", once it's in: (the item's key, it, the "or" time), or
    /// what's said of why not.
    pub(super) fn until_news(&mut self) -> Option<std::result::Result<Read, String>> {
        let res = self.until_rx.as_ref()?.2.try_recv().ok()?;
        let (key, or, _) = self.until_rx.take()?;
        Some(res.map(|c| (key, c, or)).map_err(|e| format!("not deferred: {e:#}")))
    }

    /// What there is to rank: what needs you, but what's deferred.
    pub(super) fn to_rank(&self) -> usize {
        self.items.iter().filter(|it| it.deferred.is_none()).count()
    }

    /// Whether Claude can rank them now: not while it is, nor with nothing to rank (why not).
    pub(super) fn can_rank(&self) -> Result<(), &'static str> {
        match (self.rank_rx.is_some(), self.to_rank(), self.items.is_empty()) {
            (true, ..) => Err("Claude is ranking already"),
            (_, 0, true) => Err("nothing on the todo tab to rank"),
            (_, 0, false) => Err("nothing on the todo tab to rank (what's there is deferred)"),
            _ => Ok(()),
        }
    }

    /// Claude ranks `items`, each with what it's told of it (what needs you, but what's
    /// deferred), in the background, if it can (`can_rank`). The ranking is of those, as they
    /// are (its fingerprint): one left out (deferred, say) is new to it when it's back.
    /// (Each as its `Item::entry` and the line Claude's told.)
    pub(super) fn rank(&mut self, items: Vec<(todo::Entry, String)>, root: &std::path::Path, of: Of) -> Result<(), &'static str> {
        self.can_rank()?;
        // (Each item told of: one gone from the cards meanwhile isn't.)
        if items.is_empty() {
            return Err("nothing on the todo tab to rank");
        }
        let (entries, lines): (Vec<todo::Entry>, Vec<String>) = items.into_iter().unzip();
        let fingerprint = todo::fingerprint_of(entries);
        self.rank_rx = Some(todo::rank_of(lines, fingerprint, root.to_path_buf(), of));
        Ok(())
    }

    /// A ranking, once it's in (kept: `ranked`): what it cost.
    pub(super) fn rank_news(&mut self) -> Option<Result<f64>> {
        let res = self.rank_rx.as_ref()?.try_recv().ok()?;
        self.rank_rx = None;
        Some(res.map(|r| (r.cost, self.ranked = Some(r)).0))
    }

    /// Auto doesn't rank for a quarter hour from now (a list just open: what's to triage first).
    pub(super) fn hold_auto_rank(&mut self) {
        self.auto_ranked = Some(std::time::Instant::now());
    }

    /// Whether auto ranks them again now: something new or changed since it last did, at most
    /// every quarter hour (a run each, tried or not: one failing isn't run over and over); noted.
    pub(super) fn auto_rank_due(&mut self) -> bool {
        let due = self.auto_ranked.is_none_or(|t| t.elapsed() > std::time::Duration::from_secs(15 * 60));
        let changed = self.ranked.is_none() || self.since.new + self.since.changed > 0;
        let due = due && changed && self.can_rank().is_ok();
        if due {
            self.auto_ranked = Some(std::time::Instant::now());
        }
        due
    }
}

/// A ranking's cost: "cost $0.38" (one from before dv kept it: not recorded).
pub(super) fn cost_text(c: f64) -> String {
    if c > 0.0 { format!("cost ${c:.2}") } else { "cost not recorded".into() }
}

#[test]
fn deferring_an_item() {
    use todo::{Kind, Need};
    let it = Item { kind: Kind::Issue, number: 7, needs: vec![Need::Untriaged], claude: None, updated: String::new(), reason: None, changed: false, deferred: None, busy: None, waits: None };
    let file = std::env::temp_dir().join(format!("dv-todos-{}.json", std::process::id()));
    let mut t = Todos { items: vec![it.clone()], deferred_file: Some(file.clone()), ..Default::default() };
    // Not before what it needs is all in: said so.
    assert!(t.defer_key(&it, "#7").1.starts_with("once GitHub's part is in"));
    t.ready = true;
    // s asks how long; d: a day, kept.
    let (changed, said) = t.defer_key(&it, "#7");
    assert!(!changed && said.starts_with("defer #7 for:"));
    assert!(matches!(t.deferring_key(KeyCode::Char('d')), Some(After::Deferred(s)) if s.starts_with("deferred for a day")));
    assert!(t.deferred.contains_key("issue-7") && t.save().is_none() && std::fs::read_to_string(&file).unwrap().contains("issue-7"));
    // No s pending: not its key. s, then another: not deferred; s, then u: until what.
    assert!(t.deferring_key(KeyCode::Char('d')).is_none());
    t.deferring = Some("issue-7".into());
    assert!(matches!(t.deferring_key(KeyCode::Char('x')), Some(After::Not(_))));
    t.deferring = Some("issue-7".into());
    assert!(matches!(t.deferring_key(KeyCode::Esc), Some(After::Off)));
    // One no longer listed: not deferred, said so.
    t.deferring = Some("issue-9".into());
    assert!(matches!(t.deferring_key(KeyCode::Char('d')), Some(After::Not(s)) if s == GONE));
    t.deferring = Some("issue-7".into());
    assert!(matches!(t.deferring_key(KeyCode::Char('u')), Some(After::Until(k)) if k == "issue-7"));
    // Until #2380 merges: met once it has, named as you know it.
    t.deferred.insert("issue-7".into(), todo::Deferral { until: i64::MAX, needs: it.needs_key(), cond: Some(Until::Merged(2380)), at: 0 });
    assert!(t.met(&t.items, |_, _, _| false).is_empty());
    assert_eq!(t.met(&t.items, |_, c, _| *c == Until::Merged(2380)), [("issue-7".to_owned(), "#7 is back: #2380 is merged (or closed)".to_owned())]);
    t.deferred.remove("issue-7");
    // A reading under way for it, then a deferral of yours: the reading's dropped.
    let (_tx, rx) = std::sync::mpsc::channel();
    t.until_rx = Some(("issue-7".into(), None, rx));
    assert!(t.defer("issue-7", 1, None, 0) && t.until_rx.is_none());
    // "until 3 days" read here, no Claude.
    assert!(matches!(t.until("issue-7".into(), "3 days", String::new, &file), Reading::Read(Until::Time(_), None)));
    // s on one deferred: back.
    let deferred = Item { deferred: t.deferred.get("issue-7").cloned(), ..it };
    assert!(t.defer_key(&deferred, "#7") == (true, "#7 is back".into()) && t.deferred.is_empty());
    // All deferred: nothing to rank (r says why; no run).
    let all_deferred = Todos { items: vec![deferred.clone()], ..Default::default() };
    assert!(all_deferred.rank_offer("issue(s)").is_err() && all_deferred.can_rank().is_err());
    assert_eq!(Todos::default().can_rank(), Err("nothing on the todo tab to rank")); // (none deferred: none at all)
    // Never ranked, something not deferred: auto ranks now, then not for a quarter hour.
    assert!(t.auto_rank_due() && !t.auto_rank_due());
    // Ranked with #8 deferred: nothing's new or changed while it is; back, it's new (not ranked).
    let eight = Item { number: 8, ..t.items[0].clone() };
    t.items.push(Item { deferred: Some(todo::Deferral { until: i64::MAX, needs: String::new(), cond: None, at: 0 }), ..eight.clone() });
    let ranked = Ranked { at: 1, cost: 0.0, fingerprint: todo::fingerprint_of(t.items.iter().filter(|it| it.deferred.is_none()).map(Item::entry)), items: vec![], waits: Default::default(), closes: Default::default() };
    let since = |items: &[Item]| todo::since_of(items, Some(&ranked));
    assert_eq!((since(&t.items).new, since(&t.items).changed), (0, 0));
    assert_eq!(since(&[t.items[0].clone(), eight]).new, 1);
    _ = std::fs::remove_file(file);
}

#[test]
fn two_lists_keep_each_others_deferrals() {
    let file = std::env::temp_dir().join(format!("dv-todos-two-{}.json", std::process::id()));
    _ = std::fs::remove_file(&file);
    let d = || todo::Deferral { until: i64::MAX, needs: String::new(), cond: None, at: 0 };
    let (mut a, mut b) = (Todos { deferred_file: Some(file.clone()), ..Default::default() }, Todos { deferred_file: Some(file.clone()), ..Default::default() });
    a.deferred.insert("pr-1".into(), d());
    assert!(a.save().is_none());
    b.deferred.insert("pr-2".into(), d());
    assert!(b.save().is_none());
    // Each kept, and b has a's now; a's removal of its own leaves b's.
    assert!(b.deferred.contains_key("pr-1") && b.deferred.contains_key("pr-2"));
    a.deferred.remove("pr-1");
    assert!(a.save().is_none());
    assert_eq!(todo::load_deferred(Some(&file)).keys().collect::<Vec<_>>(), ["pr-2"]);
    _ = std::fs::remove_file(&file);
}
