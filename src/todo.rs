//! `dv list`'s todo tab: the PRs (and replacements) that need you, ranked. dv finds what needs
//! you (`needs`, from what it and GitHub know), Claude puts them in order and says why and what
//! next (`rank`), and on the top ones does its own part first (`Item::claude`: a review, an
//! update to one, answers to your replies), so each is ready when you get to it.
use crate::batch::Task;
use crate::gh::{Extra, Listed};
use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::sync::mpsc::{self, Receiver};

/// Something on a PR (or replacement) that's waiting on you, most pressing first.
#[derive(Clone, Debug, PartialEq)]
pub enum Need {
    /// A replacement Claude has drafted: publish it (or not).
    Publish,
    /// A published replacement whose originals (this many) are still open: close them.
    CloseOriginals(usize),
    /// Work in its worktree that isn't on GitHub (yours, or Claude's): this many changed files,
    /// and commits beyond the PR's head. Check it, commit it, push it (`Space g`).
    Unpushed(usize, usize),
    /// You reviewed it, and its author has pushed since.
    PushedSinceReview,
    /// Claude has answered you on this many of its review's threads.
    ClaudeAnswered(usize),
    /// Your review is requested.
    ReviewRequested,
    /// Approved, mergeable, checks green: merge it.
    ReadyToMerge,
    /// It conflicts with its base: a rebase is needed (`true`: yours, so yours to do).
    Conflicts(bool),
    /// Claude's review has changed since you last looked.
    ClaudeReviewNew,
    /// Claude's comments (this many) you haven't answered or resolved.
    ClaudeComments(usize),
    /// Nothing from you on it yet.
    NoReview,
    /// New commits or comments since you last looked.
    NewActivity,
}

impl Need {
    /// How pressing it is, for the order before (and without) Claude's.
    pub fn weight(&self) -> u8 {
        match self {
            Need::Publish => 90,
            Need::Unpushed(..) => 85,
            Need::PushedSinceReview => 80,
            Need::ClaudeAnswered(_) => 75,
            Need::ReviewRequested => 70,
            Need::CloseOriginals(_) => 65,
            Need::ReadyToMerge => 60,
            Need::Conflicts(_) => 55,
            Need::ClaudeReviewNew => 50,
            Need::ClaudeComments(_) => 40,
            Need::NoReview => 30,
            Need::NewActivity => 20,
        }
    }

    /// In a few words, for the card (and for Claude, ranking).
    pub fn says(&self) -> String {
        let s = |n: usize| if n == 1 { "" } else { "s" };
        match self {
            Need::Publish => "replacement drafted: publish it".into(),
            Need::CloseOriginals(n) => format!("published: close the {n} PR{} it replaces", s(*n)),
            Need::Unpushed(files, commits) => {
                let what = match (files, commits) {
                    (0, c) => format!("{c} commit{} the PR doesn't have", s(*c)),
                    (f, 0) => format!("{f} uncommitted file{}", s(*f)),
                    (f, c) => format!("{f} uncommitted file{} and {c} commit{} the PR doesn't have", s(*f), s(*c)),
                };
                format!("in its worktree: {what} (Space g there)")
            }
            Need::PushedSinceReview => "pushed since your review".into(),
            Need::ClaudeAnswered(n) => format!("Claude answered you on {n} thread{}", s(*n)),
            Need::ReviewRequested => "your review is requested".into(),
            Need::ReadyToMerge => "approved, checks green: merge it".into(),
            Need::Conflicts(true) => "conflicts with its base: rebase it".into(),
            Need::Conflicts(false) => "conflicts with its base: ask for a rebase (b b)".into(),
            Need::ClaudeReviewNew => "Claude's review is new to you".into(),
            Need::ClaudeComments(n) => format!("{n} comment{} from Claude to go through", s(*n)),
            Need::NoReview => "no review from you yet".into(),
            Need::NewActivity => "new activity since you looked".into(),
        }
    }
}

/// A PR or replacement on the todo tab.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// A replacement (`number` is its own, its first original's).
    pub rep: bool,
    pub number: u64,
    pub needs: Vec<Need>,
    /// What Claude should do on it first, if anything.
    pub claude: Option<Task>,
    /// For the order among equals: the longest waiting first.
    pub updated: String,
    /// Claude's word on it, from the last ranking: why it's where it is, and what to do.
    pub reason: Option<String>,
    /// New, or its needs changed, since that ranking: placed by the rules, not Claude.
    pub changed: bool,
    /// Deferred until then (seconds; `s`): at the bottom till then, unless its needs change.
    pub deferred: Option<i64>,
    /// What Claude is doing on it now (from any dv): nothing for you till it's done, so it's at
    /// the bottom meanwhile.
    pub busy: Option<String>,
}

impl Item {
    pub fn key(&self) -> String {
        format!("{}-{}", if self.rep { "rep" } else { "pr" }, self.number)
    }

    /// Its needs, as deferring it records them (it comes back early when they change).
    pub fn needs_key(&self) -> String {
        self.needs.iter().map(|n| n.weight().to_string()).collect::<Vec<_>>().join(",")
    }

    fn top(&self) -> u8 {
        self.needs.iter().map(Need::weight).max().unwrap_or(0)
    }

    /// What the card says: Claude's word if it ranked it, else the needs.
    pub fn says(&self) -> String {
        self.reason.clone().unwrap_or_else(|| self.needs.iter().map(Need::says).collect::<Vec<_>>().join(" · "))
    }
}

/// What's known about a PR, to see what it needs.
pub struct Facts<'a> {
    pub pr: &'a Listed,
    pub extra: Option<&'a Extra>,
    /// Your GitHub login.
    pub me: &'a str,
    pub review: Option<&'a crate::claude::Review>,
    /// When you last left it in dv, and when Claude's review last changed (seconds).
    pub looked: Option<i64>,
    pub review_changed: Option<i64>,
    /// In its worktree, what isn't on GitHub: (changed files, commits beyond the PR's head).
    pub unpushed: (usize, usize),
}

/// What PR `f` needs from you, and what Claude should do on it first.
pub fn needs(f: &Facts) -> (Vec<Need>, Option<Task>) {
    let (pr, mut out) = (f.pr, vec![]);
    if f.unpushed != (0, 0) {
        out.push(Need::Unpushed(f.unpushed.0, f.unpushed.1));
    }
    let mine = !f.me.is_empty() && pr.author == f.me;
    let head = f.extra.map_or("", |e| e.head_oid.as_str());
    let my_review = f.extra.and_then(|e| e.my_review.as_ref());
    if let Some((_, commit)) = my_review
        && !mine
        && !commit.is_empty()
        && !head.is_empty()
        && commit != head
    {
        out.push(Need::PushedSinceReview);
    }
    let threads = f.review.map_or(&[][..], |r| r.threads.as_slice());
    let count = |p: &dyn Fn(&crate::gh::Thread) -> bool| threads.iter().filter(|t| p(t)).count();
    let answered = count(&|t| crate::view::turn(t) == Some("[answered] "));
    if answered > 0 {
        out.push(Need::ClaudeAnswered(answered));
    }
    let requested = f.extra.is_some_and(|e| e.requested);
    if requested && !mine {
        out.push(Need::ReviewRequested);
    }
    let green = pr.checks.is_some_and(|(_, fail, pending)| fail == 0 && pending == 0);
    if pr.decision == "APPROVED" && f.extra.is_some_and(|e| e.mergeable == "MERGEABLE") && green && !pr.draft {
        out.push(Need::ReadyToMerge);
    }
    if f.extra.is_some_and(|e| e.mergeable == "CONFLICTING") && !pr.draft {
        out.push(Need::Conflicts(mine));
    }
    if f.review.is_some() && f.review_changed.is_some_and(|c| c > f.looked.unwrap_or(0)) {
        out.push(Need::ClaudeReviewNew);
    }
    let open = count(&|t| crate::view::claude_wrote(t) && !t.resolved && !t.comments.iter().any(|c| c.author == "you"));
    if open > 0 {
        out.push(Need::ClaudeComments(open));
    }
    if f.extra.is_some() && my_review.is_none() && !requested && !mine && !pr.draft {
        out.push(Need::NoReview);
    }
    let updated = crate::gh::epoch_secs(&pr.updated);
    if out.is_empty() && f.looked.is_some_and(|l| updated.is_some_and(|u| u > l)) {
        out.push(Need::NewActivity);
    }
    // Claude's part: your replies waiting on it, a review behind the PR, or none yet.
    let waiting = count(&|t| crate::view::turn(t) == Some("[waiting] "));
    let claude = match f.review {
        _ if pr.draft => None,
        Some(_) if waiting > 0 => Some(Task::Answer { theirs: !mine }),
        Some(r) if !r.head.is_empty() && !head.is_empty() && r.head != head => Some(Task::Update(r.head.clone())),
        Some(_) => None,
        None => Some(Task::Review),
    };
    (out, claude)
}

/// What a replacement needs from you: publishing, then closing what it replaces.
pub fn rep_needs(r: &crate::replace::Record) -> Vec<Need> {
    match r.published {
        None if !r.title.is_empty() && !r.drafting_now() => vec![Need::Publish],
        Some(_) => match r.replaces.iter().filter(|o| !r.closed.contains(&o.number)).count() {
            0 => vec![],
            n => vec![Need::CloseOriginals(n)],
        },
        None => vec![],
    }
}

/// The order without Claude's: most pressing need first, then the longest waiting.
pub fn rule_order(items: &mut [Item]) {
    items.sort_by(|a, b| b.top().cmp(&a.top()).then_with(|| a.updated.cmp(&b.updated)));
}

/// Claude's last ranking: when, of what (`fingerprint`), and its order with a word on each.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ranked {
    pub at: i64,
    /// What it cost (dollars, as Claude Code reports it).
    pub cost: f64,
    pub fingerprint: String,
    /// (key, "reason — next"), in order.
    pub items: Vec<(String, String)>,
}

/// What a ranking was of: each item and its needs. When today's differs, the ranking is stale.
pub fn fingerprint(items: &[Item]) -> String {
    let mut keys: Vec<String> = items.iter().map(|i| format!("{}:{}", i.key(), i.needs.iter().map(Need::weight).map(|w| w.to_string()).collect::<Vec<_>>().join(","))).collect();
    keys.sort();
    keys.join(";")
}

/// What's changed since a ranking, for judging whether another is worth its cost: items new to
/// the list, items whose needs changed, items no longer on it (seen to).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Since {
    pub new: usize,
    pub changed: usize,
    pub gone: usize,
}

/// `items` in Claude's order (with its word on each) where it ranked them as they are; the
/// others (new, or changed since: `Item::changed`) in the rules' order, each placed before the
/// first ranked one that needs you less. Also what's changed since the ranking.
pub fn order(mut items: Vec<Item>, ranked: Option<&Ranked>) -> (Vec<Item>, Since) {
    // The ranking's items and their needs then, from its fingerprint.
    let fp = ranked.map_or("", |r| r.fingerprint.as_str());
    let then: std::collections::HashMap<&str, &str> = fp.split(';').filter_map(|e| e.split_once(':')).collect();
    let now = |it: &Item| it.needs_key();
    let mut since = Since { gone: then.keys().filter(|k| !items.iter().any(|it| it.key() == **k)).count(), ..Since::default() };
    for it in items.iter_mut().filter(|_| ranked.is_some()) {
        match then.get(it.key().as_str()) {
            None => (it.changed, since.new) = (true, since.new + 1),
            Some(n) if *n != now(it) => (it.changed, since.changed) = (true, since.changed + 1),
            Some(_) => {}
        }
    }
    let mut out = vec![];
    for (key, word) in ranked.map_or(&[][..], |r| r.items.as_slice()) {
        if let Some(i) = items.iter().position(|it| &it.key() == key) {
            let mut it = items.remove(i);
            it.reason = Some(word.clone());
            out.push(it);
        }
    }
    rule_order(&mut items);
    for it in items {
        let at = out.iter().position(|o: &Item| o.top() < it.top()).unwrap_or(out.len());
        out.insert(at, it);
    }
    (out, since)
}

fn path() -> Option<std::path::PathBuf> {
    crate::claude::dv_dir().ok().map(|d| d.join("todo.json"))
}

/// Items deferred (`s`): key → (until, its needs then).
pub type Deferred = std::collections::HashMap<String, (i64, String)>;

/// Where they're kept: `deferred.json` in dv's directory.
pub fn deferred_path() -> Option<std::path::PathBuf> {
    crate::claude::dv_dir().ok().map(|d| d.join("deferred.json"))
}

/// What's deferred (in file `p`), the expired ones gone.
pub fn load_deferred(p: Option<&std::path::Path>) -> Deferred {
    let v: Value = p.and_then(|p| serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()).unwrap_or_default();
    let now = crate::claude::now_secs();
    v.as_object()
        .into_iter()
        .flatten()
        .filter_map(|(k, e)| Some((k.clone(), (e["until"].as_i64()?, e["needs"].as_str().unwrap_or("").to_owned()))))
        .filter(|(_, (until, _))| *until > now)
        .collect()
}

pub fn save_deferred(p: Option<&std::path::Path>, d: &Deferred) -> Result<()> {
    let v: serde_json::Map<String, Value> = d.iter().map(|(k, (until, needs))| (k.clone(), json!({ "until": until, "needs": needs }))).collect();
    let p = p.context("no dv directory")?;
    std::fs::create_dir_all(p.parent().unwrap_or(p))?;
    Ok(std::fs::write(p, Value::Object(v).to_string())?)
}

/// `items` with what's deferred (and still needs what it did then) moved to the end, marked.
pub fn defer(items: Vec<Item>, deferred: &Deferred, now: i64) -> Vec<Item> {
    let (mut later, mut out): (Vec<Item>, Vec<Item>) = items.into_iter().partition(|it| {
        deferred.get(&it.key()).is_some_and(|(until, needs)| *until > now && *needs == it.needs_key())
    });
    later.iter_mut().for_each(|it| it.deferred = deferred.get(&it.key()).map(|d| d.0));
    out.append(&mut later);
    out
}

/// The last ranking (`todo.json` in dv's directory), if any.
pub fn load() -> Option<Ranked> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(path()?).ok()?).ok()?;
    let items = v["items"].as_array()?.iter().map(|i| (i["key"].as_str().unwrap_or("").to_owned(), i["says"].as_str().unwrap_or("").to_owned())).collect();
    Some(Ranked { at: v["at"].as_i64().unwrap_or(0), cost: v["cost"].as_f64().unwrap_or(0.0), fingerprint: v["fingerprint"].as_str().unwrap_or("").to_owned(), items })
}

fn save(r: &Ranked) -> Result<()> {
    let items: Vec<Value> = r.items.iter().map(|(k, s)| json!({ "key": k, "says": s })).collect();
    let p = path().context("no dv directory")?;
    std::fs::create_dir_all(p.parent().unwrap_or(&p))?;
    Ok(std::fs::write(p, json!({ "at": r.at, "cost": r.cost, "fingerprint": r.fingerprint, "items": items }).to_string())?)
}

/// A line about item `it` for Claude's ranking: what it is and what it needs.
pub fn about(it: &Item, pr: &Listed, summary: &str) -> String {
    let mut v = json!({
        "key": it.key(),
        "title": pr.title,
        "author": pr.author,
        "size": format!("+{} -{} in {} files", pr.additions, pr.deletions, pr.files),
        "opened": crate::gh::ago(&pr.created),
        "updated": crate::gh::ago(&pr.updated),
        "labels": pr.labels.iter().map(|l| l.0.clone()).collect::<Vec<_>>(),
        "needs": it.needs.iter().map(Need::says).collect::<Vec<_>>(),
    });
    if let Some((pass, fail, pending)) = pr.checks {
        v["checks"] = json!(format!("{pass} passed, {fail} failed, {pending} pending"));
    }
    if !summary.trim().is_empty() {
        v["claude_review"] = json!(summary.chars().take(400).collect::<String>());
    }
    v.to_string()
}

/// Claude puts `lines` (from `about`) in order, in the background, and the ranking is saved;
/// `fingerprint` is what it's of. It only reads (the PRs, on GitHub).
pub fn rank(lines: Vec<String>, fingerprint: String, root: std::path::PathBuf) -> Receiver<Result<Ranked>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let res = ask(&lines, &root).map(|(items, cost)| Ranked { at: crate::claude::now_secs(), cost, fingerprint, items });
        if let Ok(r) = &res {
            _ = save(r);
        }
        _ = tx.send(res);
    });
    rx
}

fn ask(lines: &[String], root: &std::path::Path) -> Result<(Vec<(String, String)>, f64)> {
    let notes = crate::claude::notes_path().ok().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
    let notes = match notes.trim() {
        "" => String::new(),
        n => format!("Your notes on this repo, from reviewing it (follow them):\n{n}\n\n"),
    };
    let prompt = format!(
        "{notes}I maintain this repo. These open PRs (and replacements: PRs you drafted to replace others) are \
         waiting on me, one JSON object each, with what each needs from me:\n{}\n\n\
         Put them in the order I should take them, most important first. Judge what matters: fixes for bugs and \
         security over refactors and features; contributors who've waited longest; quick ones first among equals; \
         a PR that blocks others before them. Read a PR (`gh pr view N`) only where its line isn't enough to judge. \
         Change nothing. Reply with only a JSON array, every key exactly once, no other text: \
         [{{\"key\": \"pr-12\", \"reason\": \"why it's here, a few words\", \"next\": \"what I should do, a few words\"}}]",
        lines.join("\n")
    );
    let tools = ["Bash(gh pr view:*)", "Bash(gh pr diff:*)"];
    let out = crate::view::claude_command(&prompt, root, &[], &tools, Some(root)).stdin(std::process::Stdio::null()).output()?;
    let v: Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
    anyhow::ensure!(v["is_error"] == false, "Claude couldn't rank them ({})", out.status);
    Ok((parse(v["result"].as_str().unwrap_or(""))?, v["total_cost_usd"].as_f64().unwrap_or(0.0)))
}

/// The order in Claude's answer (a JSON array, maybe with words around it): (key, "reason — next").
fn parse(answer: &str) -> Result<Vec<(String, String)>> {
    let json = answer.get(answer.find('[').context("no ranking in Claude's answer")?..=answer.rfind(']').context("no ranking")?).unwrap_or("");
    let v: Value = serde_json::from_str(json)?;
    let s = |v: &Value| v.as_str().unwrap_or("").trim().to_owned();
    Ok(v.as_array()
        .context("no ranking")?
        .iter()
        .map(|i| {
            let (reason, next) = (s(&i["reason"]), s(&i["next"]));
            let says = if next.is_empty() { reason } else { format!("{reason} — {next}") };
            (s(&i["key"]), says)
        })
        .filter(|(k, _)| !k.is_empty())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gh::{Comment, Thread};

    fn pr(n: u64, author: &str) -> Listed {
        Listed { number: n, author: author.into(), state: "OPEN".into(), updated: "2026-10-01T00:00:00Z".into(), ..Listed::default() }
    }

    fn thread(comments: &[&str], resolved: bool) -> Thread {
        let c = |a: &&str| Comment { author: (*a).into(), ..Comment::default() };
        Thread { claude: Some("c1".into()), comments: comments.iter().map(c).collect(), resolved, ..Thread::default() }
    }

    /// When you last looked: after the PRs' updates (2026-10-01), before 2026-12-01.
    const LOOKED: i64 = 1_795_000_000;

    fn leak<T>(x: T) -> &'static T {
        Box::leak(Box::new(x))
    }

    fn review(threads: Vec<Thread>, head: &str) -> crate::claude::Review {
        crate::claude::Review { head: head.into(), summary: String::new(), threads }
    }

    #[test]
    fn what_needs_you() {
        let extra = |mine: Option<(&str, &str)>, requested, head: &str| Extra {
            my_review: mine.map(|(s, c)| (s.into(), c.into())),
            requested,
            mergeable: "MERGEABLE".into(),
            head_oid: head.into(),
        };
        let facts = |p: &'static Listed, e: Option<&'static Extra>, r: Option<&'static crate::claude::Review>| Facts { pr: p, extra: e, me: "me", review: r, looked: Some(LOOKED), review_changed: Some(50), unpushed: (0, 0) };
        // Nothing from you, no Claude review: you, and Claude reviews it.
        let (n, c) = needs(&facts(leak(pr(1, "amy")), Some(leak(extra(None, false, "h1"))), None));
        assert_eq!((n, c), (vec![Need::NoReview], Some(Task::Review)));
        // You reviewed, then they pushed; Claude answered you on one, and has one untouched.
        let r = leak(review(vec![thread(&["claude", "you", "claude"], false), thread(&["claude"], false), thread(&["claude"], true)], "h2"));
        let (n, c) = needs(&facts(leak(pr(2, "amy")), Some(leak(extra(Some(("CHANGES_REQUESTED", "h1")), false, "h2"))), Some(r)));
        assert_eq!((n, c), (vec![Need::PushedSinceReview, Need::ClaudeAnswered(1), Need::ClaudeComments(1)], None));
        // Your reply waiting on Claude, its review behind: it answers first.
        let r = leak(review(vec![thread(&["claude", "you"], false)], "old"));
        let (_, c) = needs(&facts(leak(pr(3, "me")), Some(leak(extra(None, false, "new"))), Some(r)));
        assert_eq!(c, Some(Task::Answer { theirs: false }));
        let r = leak(review(vec![], "old"));
        let (_, c) = needs(&facts(leak(pr(3, "me")), Some(leak(extra(None, false, "new"))), Some(r)));
        assert_eq!(c, Some(Task::Update("old".into())));
        // Requested (not "no review" on top); approved + mergeable + green: merge it.
        let mut p = pr(4, "amy");
        (p.decision, p.checks) = ("APPROVED".into(), Some((3, 0, 0)));
        let (n, _) = needs(&facts(leak(p), Some(leak(extra(None, true, "h"))), None));
        assert_eq!(n, vec![Need::ReviewRequested, Need::ReadyToMerge]);
        // Your own PR, nothing else: nothing; with an update since you looked: new activity.
        let (n, _) = needs(&facts(leak(pr(5, "me")), Some(leak(extra(None, false, "h"))), None));
        assert_eq!(n, vec![]);
        let mut p = pr(5, "me");
        p.updated = "2026-12-01T00:00:00Z".into();
        let (n, _) = needs(&facts(leak(p), Some(leak(extra(None, false, "h"))), None));
        assert_eq!(n, vec![Need::NewActivity]);
        // Work in its worktree not pushed: first.
        let mut f = facts(leak(pr(8, "me")), Some(leak(extra(None, false, "h"))), None);
        f.unpushed = (2, 1);
        assert_eq!(needs(&f).0, vec![Need::Unpushed(2, 1)]);
        assert_eq!(Need::Unpushed(2, 1).says(), "in its worktree: 2 uncommitted files and 1 commit the PR doesn't have (Space g there)");
        // Conflicting with its base: ask for a rebase (or, yours, do it).
        let mut conflicting = extra(None, false, "h");
        conflicting.mergeable = "CONFLICTING".into();
        assert_eq!(needs(&facts(leak(pr(7, "amy")), Some(leak(conflicting.clone())), None)).0, vec![Need::Conflicts(false), Need::NoReview]);
        assert_eq!(needs(&facts(leak(pr(7, "me")), Some(leak(conflicting)), None)).0, vec![Need::Conflicts(true)]);
        // Claude's review changed since you looked.
        let mut f = facts(leak(pr(6, "me")), Some(leak(extra(None, false, "h"))), Some(leak(review(vec![], "h"))));
        f.review_changed = Some(LOOKED + 1);
        assert_eq!(needs(&f).0, vec![Need::ClaudeReviewNew]);
    }

    #[test]
    fn replacements() {
        let o = |n| crate::replace::Original { number: n, ..Default::default() };
        let r = crate::replace::Record { id: 12, replaces: vec![o(12), o(15)], title: "Fix".into(), ..Default::default() };
        assert_eq!(rep_needs(&r), vec![Need::Publish]);
        let r = crate::replace::Record { published: Some(20), closed: vec![12], ..r };
        assert_eq!(rep_needs(&r), vec![Need::CloseOriginals(1)]);
        assert_eq!(rep_needs(&crate::replace::Record { closed: vec![12, 15], ..r }), vec![]);
    }

    #[test]
    fn ordering() {
        let it = |n, needs: Vec<Need>, updated: &str| Item { rep: false, number: n, needs, claude: None, updated: updated.into(), reason: None, changed: false, deferred: None, busy: None };
        let items = vec![it(1, vec![Need::NoReview], "b"), it(2, vec![Need::ReviewRequested], "c"), it(3, vec![Need::NoReview], "a")];
        // Rules: the most pressing first, then the longest waiting.
        let (o, since) = order(items.clone(), None);
        assert_eq!(since, Since::default());
        assert!(o.iter().all(|i| !i.changed)); // nothing to have changed since
        assert_eq!(o.iter().map(|i| i.number).collect::<Vec<_>>(), [2, 3, 1]);
        // Claude's order, with its word; a new one goes before the first that needs you less.
        let ranked = Ranked { at: 1, cost: 0.1, fingerprint: fingerprint(&items), items: vec![("pr-1".into(), "small fix — merge".into()), ("pr-3".into(), "x".into())] };
        let (o, since) = order(items.clone(), Some(&ranked));
        assert_eq!(since, Since::default()); // it's of the same three (pr-2 left out of Claude's answer goes by the rules)
        assert_eq!(o.iter().map(|i| i.number).collect::<Vec<_>>(), [2, 1, 3]);
        assert_eq!(o[1].says(), "small fix — merge");
        assert_eq!(o[0].says(), "your review is requested");
        // Since: one new, one whose needs changed, one gone; those two marked.
        let mut now = vec![it(1, vec![Need::ReviewRequested], "b"), it(3, vec![Need::NoReview], "a"), it(9, vec![Need::NoReview], "d")];
        now[0].needs.push(Need::NoReview);
        let (o, since) = order(now, Some(&ranked));
        assert_eq!(since, Since { new: 1, changed: 1, gone: 1 });
        assert_eq!(o.iter().filter(|i| i.changed).map(|i| i.number).collect::<Vec<_>>().len(), 2);
        // Claude's answer, words around it.
        let p = parse("Here: [{\"key\": \"pr-2\", \"reason\": \"bug fix\", \"next\": \"review it\"}, {\"key\": \"rep-12\", \"reason\": \"ready\"}]").unwrap();
        assert_eq!(p, [("pr-2".to_owned(), "bug fix — review it".to_owned()), ("rep-12".to_owned(), "ready".to_owned())]);
    }

    #[test]
    fn deferring() {
        let it = |n, needs: Vec<Need>| Item { rep: false, number: n, needs, claude: None, updated: String::new(), reason: None, changed: false, deferred: None, busy: None };
        let items = vec![it(1, vec![Need::ReadyToMerge]), it(2, vec![Need::NoReview])];
        let d: Deferred = [("pr-1".to_owned(), (200, items[0].needs_key()))].into();
        // Deferred: to the end, marked, until then.
        let o = defer(items.clone(), &d, 100);
        assert_eq!((o[0].number, o[1].number, o[1].deferred), (2, 1, Some(200)));
        // Past it: back in place.
        assert_eq!(defer(items.clone(), &d, 300)[0].number, 1);
        // Its needs changed: back before then.
        let mut changed = items;
        changed[0].needs.push(Need::NewActivity);
        assert_eq!(defer(changed, &d, 100)[0].deferred, None);
    }
}
