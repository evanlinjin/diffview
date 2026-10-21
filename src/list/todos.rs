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
    pub(super) deferred_file: Option<PathBuf>,
    pub(super) deferring: Option<String>,
    /// Claude reading a deferral's "until …": the item's key, the "or" time, its answer.
    pub(super) until_rx: Option<(String, Option<i64>, Receiver<Result<Until>>)>,
    /// When auto last had Claude rank them.
    pub(super) auto_ranked: Option<std::time::Instant>,
}

/// What the key after `s` did: deferred it (said so: to save), asked until what (the item's
/// key), or not.
pub(super) enum After {
    Deferred(String),
    Until(String),
    Not,
}

/// What `Todos::until` made of an "until …": the condition to defer on, or Claude reading it.
pub(super) enum Reading {
    Read(Until, Option<i64>),
    Asked,
}

impl Todos {
    /// The list of `of`'s: its last ranking and deferrals, as kept.
    pub(super) fn of(of: Of) -> Todos {
        let file = todo::deferred_path_of(of);
        Todos { ranked: todo::load_of(of), deferred: todo::load_deferred(file.as_deref()), deferred_file: file, ..Default::default() }
    }

    /// What needs you now: not deferred, Claude not at it, not waiting on something.
    pub(super) fn now(&self) -> usize {
        self.items.iter().filter(|it| it.deferred.is_none() && it.busy.is_none() && it.waits.is_none()).count()
    }

    /// `s` on `it` (`what` names it): brought back if deferred (`true`: changed, to save), else
    /// asked how long for.
    pub(super) fn defer_key(&mut self, it: &Item, what: &str) -> (bool, String) {
        if it.deferred.is_some() {
            self.deferred.remove(&it.key());
            return (true, format!("{what} is back"));
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
                self.defer(&key, crate::claude::now_secs() + secs, None, 0);
                After::Deferred(format!("deferred for {how} (s on it brings it back)"))
            }
            KeyCode::Char('u') => After::Until(key),
            _ => After::Not,
        })
    }

    /// Item `key` deferred till `until` (or `cond` happens: since `at`, by GitHub's clock).
    pub(super) fn defer(&mut self, key: &str, until: i64, cond: Option<Until>, at: i64) {
        let Some(needs) = self.items.iter().find(|it| it.key() == key).map(Item::needs_key) else { return };
        self.deferred.insert(key.to_owned(), todo::Deferral { until, needs, cond, at });
    }

    /// The deferrals, kept (an error: what's said).
    pub(super) fn save(&self) -> Option<String> {
        todo::save_deferred(self.deferred_file.as_deref(), &self.deferred).err().map(|e| format!("couldn't save it: {e:#}"))
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
                self.until_rx = Some((key, or, todo::interpret(text, about(), root.to_path_buf())));
                Reading::Asked
            }
        }
    }

    /// Claude's reading of an "until …", once it's in: (the item's key, it, the "or" time).
    pub(super) fn until_news(&mut self) -> Option<(String, Result<Until>, Option<i64>)> {
        let res = self.until_rx.as_ref()?.2.try_recv().ok()?;
        let (key, or, _) = self.until_rx.take()?;
        Some((key, res, or))
    }

    /// Claude ranks `lines` (what needs you, but what's deferred), in the background.
    pub(super) fn rank(&mut self, lines: Vec<String>, root: &std::path::Path, of: Of) {
        let now: Vec<Item> = self.items.iter().filter(|it| it.deferred.is_none()).cloned().collect();
        self.rank_rx = Some(todo::rank_of(lines, todo::fingerprint(&now), root.to_path_buf(), of));
    }

    /// A ranking, once it's in (kept: `ranked`).
    pub(super) fn rank_news(&mut self) -> Option<Result<()>> {
        let res = self.rank_rx.as_ref()?.try_recv().ok()?;
        self.rank_rx = None;
        Some(res.map(|r| self.ranked = Some(r)))
    }

    /// Whether auto ranks them again now: something new or changed since it last did, at most
    /// every quarter hour (a run each, tried or not: one failing isn't run over and over); noted.
    pub(super) fn auto_rank_due(&mut self) -> bool {
        let due = self.auto_ranked.is_none_or(|t| t.elapsed() > std::time::Duration::from_secs(15 * 60));
        let changed = self.ranked.is_none() || self.since.new + self.since.changed > 0;
        let due = due && changed && self.rank_rx.is_none() && self.items.iter().any(|it| it.deferred.is_none());
        if due {
            self.auto_ranked = Some(std::time::Instant::now());
        }
        due
    }
}

#[test]
fn deferring_an_item() {
    use todo::{Kind, Need};
    let it = Item { kind: Kind::Issue, number: 7, needs: vec![Need::Untriaged], claude: None, updated: String::new(), reason: None, changed: false, deferred: None, busy: None, waits: None };
    let file = std::env::temp_dir().join(format!("dv-todos-{}.json", std::process::id()));
    let mut t = Todos { items: vec![it.clone()], deferred_file: Some(file.clone()), ..Default::default() };
    // s asks how long; d: a day, kept.
    let (changed, said) = t.defer_key(&it, "#7");
    assert!(!changed && said.starts_with("defer #7 for:"));
    assert!(matches!(t.deferring_key(KeyCode::Char('d')), Some(After::Deferred(s)) if s.starts_with("deferred for a day")));
    assert!(t.deferred.contains_key("issue-7") && t.save().is_none() && std::fs::read_to_string(&file).unwrap().contains("issue-7"));
    // No s pending: not its key. s, then another: not deferred; s, then u: until what.
    assert!(t.deferring_key(KeyCode::Char('d')).is_none());
    t.deferring = Some("issue-7".into());
    assert!(matches!(t.deferring_key(KeyCode::Char('x')), Some(After::Not)));
    t.deferring = Some("issue-7".into());
    assert!(matches!(t.deferring_key(KeyCode::Char('u')), Some(After::Until(k)) if k == "issue-7"));
    // "until 3 days" read here, no Claude.
    assert!(matches!(t.until("issue-7".into(), "3 days", String::new, &file), Reading::Read(Until::Time(_), None)));
    // s on one deferred: back.
    let deferred = Item { deferred: t.deferred.get("issue-7").cloned(), ..it };
    assert!(t.defer_key(&deferred, "#7") == (true, "#7 is back".into()) && t.deferred.is_empty());
    // Never ranked, something not deferred: auto ranks now, then not for a quarter hour.
    assert!(t.auto_rank_due() && !t.auto_rank_due());
    _ = std::fs::remove_file(file);
}
