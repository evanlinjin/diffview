//! `dv list`'s todo tab: the PRs (and replacements) that need you, ranked. dv finds what needs
//! you (`needs`, from what it and GitHub know), Claude puts them in order and says why and what
//! next (`rank`), and on the top ones does its own part first (`Item::claude`: a review, an
//! update to one, answers to your replies), so each is ready when you get to it.
use crate::batch::Task;
use crate::list::{Extra, Listed};
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
    /// The same, made by Claude (it asked to make changes, and has): check them, push them.
    ClaudeChanged(usize, usize),
    /// You reviewed it, and its author has pushed since.
    PushedSinceReview,
    /// Claude has answered you on this many of its review's threads.
    ClaudeAnswered(usize),
    /// Your review is requested.
    ReviewRequested,
    /// Approved, mergeable, checks green: merge it.
    ReadyToMerge,
    /// Your draft, its checks green and nothing in its worktree unpushed: mark it ready for review.
    MarkReady,
    /// It conflicts with its base: a rebase is needed (`true`: yours, so yours to do).
    Conflicts(bool),
    /// This many workflow runs on it wait for a maintainer to approve them: no CI until then.
    ApproveRuns(usize),
    /// This many of its checks fail (`true`: yours, so yours to fix).
    ChecksFailing(usize, bool),
    /// Your PR's checks fail, and Claude found it isn't the PR's doing (a flaky test, CI
    /// itself, the base): why, and whether a PR of its own would fix it (`N N` drafts it).
    CiNotThisPr(String, bool),
    /// This many of its commits aren't signed, and its base requires signed commits: GitHub
    /// won't take a merge of it (`true`: yours, so yours to sign).
    Unsigned(usize, bool),
    /// Claude's review has changed since you last looked.
    ClaudeReviewNew,
    /// Claude's comments (this many) you haven't answered or resolved.
    ClaudeComments(usize),
    /// Nothing from you on it yet.
    NoReview,
    /// New commits or comments since you last looked.
    NewActivity,
    /// An issue: a PR fixing it (this one) is merged, and it's still open: close it.
    FixMerged(u64),
    /// An issue Claude's triage would close: why.
    WouldClose(String),
    /// An issue assigned to you.
    Assigned,
    /// An issue a PR (this one) would fix: review that.
    FixOpen(u64),
    /// An issue changed since Claude triaged it: triage it again.
    Retriage,
    /// An issue Claude hasn't triaged.
    Untriaged,
}

impl Need {
    /// How pressing it is, for the order before (and without) Claude's.
    pub fn weight(&self) -> u8 {
        match self {
            Need::Publish => 90,
            Need::ClaudeChanged(..) => 88,
            Need::Unpushed(..) => 85,
            Need::PushedSinceReview => 80,
            Need::ClaudeAnswered(_) => 75,
            Need::ReviewRequested => 70,
            Need::CloseOriginals(_) => 65,
            Need::MarkReady => 63,
            Need::ReadyToMerge => 60,
            Need::Conflicts(_) => 55,
            Need::ApproveRuns(_) => 66,
            Need::Unsigned(..) => 54,
            Need::ChecksFailing(..) | Need::CiNotThisPr(..) => 52,
            Need::ClaudeReviewNew => 50,
            Need::ClaudeComments(_) => 40,
            Need::NoReview => 30,
            Need::NewActivity => 20,
            Need::FixMerged(_) => 78,
            Need::WouldClose(_) => 58,
            Need::Assigned => 56,
            Need::FixOpen(_) => 45,
            Need::Retriage => 36,
            Need::Untriaged => 31,
        }
    }

    /// What holds the PR up, in a word or two, beside Claude's reason on its card: the state
    /// of things on GitHub (none for what Claude's reason already covers).
    pub fn tag(&self) -> Option<String> {
        let s = |n: usize| if n == 1 { "" } else { "s" };
        Some(match self {
            Need::ApproveRuns(n) => format!("⏸ {n} run{} to approve (W W)", s(*n)),
            Need::ChecksFailing(n, _) => format!("✗ {n} check{} failing", s(*n)),
            Need::CiNotThisPr(..) => "✗ CI failing (not this PR)".into(),
            Need::Unsigned(n, _) => format!("{n} unsigned commit{}", s(*n)),
            Need::Conflicts(_) => "conflicts".into(),
            _ => return None,
        })
    }

    /// In a few words, for the card (and for Claude, ranking).
    pub fn says(&self) -> String {
        let s = |n: usize| if n == 1 { "" } else { "s" };
        match self {
            Need::Publish => "replacement drafted: publish it".into(),
            Need::CloseOriginals(n) => format!("published: close the {n} PR{} it replaces", s(*n)),
            Need::Unpushed(files, commits) | Need::ClaudeChanged(files, commits) => {
                let what = match (files, commits) {
                    (0, c) => format!("{c} commit{} the PR doesn't have", s(*c)),
                    (f, 0) => format!("{f} uncommitted file{}", s(*f)),
                    (f, c) => format!("{f} uncommitted file{} and {c} commit{} the PR doesn't have", s(*f), s(*c)),
                };
                match self {
                    Need::ClaudeChanged(..) => format!("Claude made changes, not pushed: {what}; check them, push them (Space g there)"),
                    _ => format!("in its worktree: {what} (Space g there)"),
                }
            }
            Need::PushedSinceReview => "pushed since your review".into(),
            Need::ClaudeAnswered(n) => format!("Claude answered you on {n} thread{}", s(*n)),
            Need::ReviewRequested => "your review is requested".into(),
            Need::ReadyToMerge => "approved, checks green: merge it".into(),
            Need::MarkReady => "your draft, checks green: mark it ready for review (Space g there)".into(),
            Need::Conflicts(true) => "conflicts with its base: rebase it".into(),
            Need::ApproveRuns(n) => format!("{n} workflow run{} awaiting approval: approve {} (W W)", s(*n), if *n == 1 { "it" } else { "them" }),
            Need::ChecksFailing(n, true) => format!("{n} check{} failing: fix {}", s(*n), if *n == 1 { "it" } else { "them" }),
            Need::ChecksFailing(n, false) => format!("{n} check{} failing: see why, tell the author", s(*n)),
            Need::CiNotThisPr(why, true) => format!("CI fails, not this PR's doing (Claude: {why}): N N drafts a PR fixing it, a a has it try again"),
            Need::CiNotThisPr(why, false) => format!("CI fails, not this PR's doing (Claude: {why}): rerun it, or a a has it try again"),
            Need::Unsigned(n, true) => format!("{n} of your commits unsigned, and its base requires signed ones: sign them, force-push"),
            Need::Unsigned(n, false) => format!("{n} commit{} unsigned, and its base requires signed ones: ask the author to sign them", s(*n)),
            Need::Conflicts(false) => "conflicts with its base: ask for a rebase (b b)".into(),
            Need::ClaudeReviewNew => "Claude's review is new to you".into(),
            Need::ClaudeComments(n) => format!("{n} comment{} from Claude to go through", s(*n)),
            Need::NoReview => "no review from you yet".into(),
            Need::NewActivity => "new activity since you looked".into(),
            Need::FixMerged(n) => format!("#{n}, which fixes it, is merged: close it"),
            Need::WouldClose(why) => format!("Claude would close it: {why}"),
            Need::Assigned => "assigned to you".into(),
            Need::FixOpen(n) => format!("#{n} would fix it: review that"),
            Need::Retriage => "changed since Claude triaged it (a a)".into(),
            Need::Untriaged => "not triaged yet (a a)".into(),
        }
    }
}

/// What a todo item is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Pr,
    /// A replacement (its number is its own, its first original's).
    Rep,
    Issue,
}

/// A PR, replacement or issue on a todo tab.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub kind: Kind,
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
    /// Deferred (`s`): at the bottom till then, or till what it waits for happens, unless its
    /// needs change first.
    pub deferred: Option<Deferral>,
    /// What Claude is doing on it now (from any dv): nothing for you till it's done, so it's at
    /// the bottom meanwhile.
    pub busy: Option<String>,
    /// What it waits on, as its conversation says (and who said it), not happened yet: at the
    /// bottom meanwhile, for everyone whose dv reads it.
    pub waits: Option<(Until, String)>,
}

impl Item {
    /// How it's kept (deferrals, rankings): `pr-12`, `rep-12`, `issue-12`.
    pub fn key(&self) -> String {
        let kind = match self.kind {
            Kind::Pr => "pr",
            Kind::Rep => "rep",
            Kind::Issue => "issue",
        };
        format!("{kind}-{}", self.number)
    }

    pub fn is_rep(&self) -> bool {
        self.kind == Kind::Rep
    }

    /// Its needs, as deferring it records them (it comes back early when they change).
    pub fn needs_key(&self) -> String {
        self.needs.iter().map(|n| n.weight().to_string()).collect::<Vec<_>>().join(",")
    }

    fn top(&self) -> u8 {
        self.needs.iter().map(Need::weight).max().unwrap_or(0)
    }

    /// Its needs, in a few words each (what the card says when Claude hasn't ranked it).
    pub fn says(&self) -> String {
        self.needs.iter().map(Need::says).collect::<Vec<_>>().join(" · ")
    }

    /// What holds the PR up, in short (runs to approve, checks failing, …): said beside Claude's
    /// word, as its ranking may be older than they are.
    pub fn tags(&self) -> Vec<String> {
        self.needs.iter().filter_map(Need::tag).collect()
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
    /// When the PR's head commit was made (seconds): Claude's changes after it aren't pushed.
    pub head_at: Option<i64>,
    /// Claude's word that its failing CI isn't its doing, if it's looked.
    pub ci_note: Option<&'a crate::claude::CiNote>,
}

/// Checks that have all passed: some, none failing or still running (none at all: CI hasn't
/// started).
pub fn green(checks: Option<(usize, usize, usize)>) -> bool {
    checks.is_some_and(|(pass, fail, pending)| pass > 0 && fail == 0 && pending == 0)
}

/// What PR `f` needs from you, and what Claude should do on it first.
pub fn needs(f: &Facts) -> (Vec<Need>, Option<Task>) {
    let (pr, mut out) = (f.pr, vec![]);
    if f.unpushed != (0, 0) {
        // Claude's doing (it says what it "Changed:" since the PR's head), or anyone's.
        let claude = f.review.is_some_and(|r| {
            r.threads.iter().flat_map(|t| &t.comments).any(|c| {
                c.author == crate::claude::AUTHOR && c.body.starts_with("Changed:") && crate::gh::epoch_secs(&c.created).is_some_and(|t| f.head_at.is_none_or(|h| t > h))
            })
        });
        out.push(match claude {
            true => Need::ClaudeChanged(f.unpushed.0, f.unpushed.1),
            false => Need::Unpushed(f.unpushed.0, f.unpushed.1),
        });
    }
    let mine = !f.me.is_empty() && pr.author == f.me;
    // Workflow runs waiting for a maintainer: CI doesn't run until then.
    if let Some(e) = f.extra.filter(|e| e.awaiting > 0) {
        out.push(Need::ApproveRuns(e.awaiting));
    }
    // CI failing: nothing merges until it passes. (Someone else's draft isn't up for it yet.)
    // On yours, Claude may have found it isn't the PR's doing (at its head now).
    let head = f.extra.map_or("", |e| e.head_oid.as_str());
    let not_this = f.ci_note.filter(|c| mine && !head.is_empty() && c.head == head);
    if let Some((_, fail, _)) = pr.checks.filter(|c| c.1 > 0 && (!pr.draft || mine)) {
        out.push(match not_this {
            Some(c) if c.rerun => Need::CiNotThisPr(format!("{}; it reran it once already", c.why), !c.task.is_empty()),
            Some(c) => Need::CiNotThisPr(c.why.clone(), !c.task.is_empty()),
            None => Need::ChecksFailing(fail, mine),
        });
    }
    // Commits GitHub would refuse on its base: nothing merges until they're signed. (Someone
    // else's draft isn't up for merging yet.)
    if let Some(e) = f.extra.filter(|e| e.signatures_required && e.unsigned > 0 && (!pr.draft || mine)) {
        out.push(Need::Unsigned(e.unsigned, mine));
    }
    // Your draft, done by the look of it: checks passed, nothing failing or still running (none
    // at all: CI hasn't started yet), nothing unpushed.
    if mine && pr.draft && f.unpushed == (0, 0) && green(pr.checks) {
        out.push(Need::MarkReady);
    }
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
    let answered = count(&|t| t.answered());
    if answered > 0 {
        out.push(Need::ClaudeAnswered(answered));
    }
    let requested = f.extra.is_some_and(|e| e.requested);
    if requested && !mine {
        out.push(Need::ReviewRequested);
    }
    let green = pr.checks.is_some_and(|(_, fail, pending)| fail == 0 && pending == 0);
    // Not with commits GitHub would refuse on its base: that's the need, merging isn't.
    let refused = f.extra.is_some_and(|e| e.signatures_required && e.unsigned > 0);
    if pr.decision == "APPROVED" && f.extra.is_some_and(|e| e.mergeable == "MERGEABLE") && green && !pr.draft && !refused {
        out.push(Need::ReadyToMerge);
    }
    if f.extra.is_some_and(|e| e.mergeable == "CONFLICTING") && !pr.draft {
        out.push(Need::Conflicts(mine));
    }
    if f.review.is_some() && f.review_changed.is_some_and(|c| c > f.looked.unwrap_or(0)) {
        out.push(Need::ClaudeReviewNew);
    }
    let open = count(&|t| t.by_claude() && !t.resolved && !t.comments.iter().any(|c| c.author == "you"));
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
    let waiting = count(&|t| t.waiting());
    let claude = match f.review {
        // Your PR's CI failing (a draft too: a published replacement's, say): Claude looks into
        // it and fixes it here, for you to push. Not again while its changes wait on you.
        _ if mine && f.unpushed == (0, 0) && not_this.is_none() && pr.checks.is_some_and(|c| c.1 > 0) => Some(Task::FixChecks),
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
    /// (key, "reason\nnext"), in order.
    pub items: Vec<(String, String)>,
    /// What Claude saw each waits on, in its conversation (by key).
    pub waits: std::collections::HashMap<String, Until>,
    /// Those it says to close, and why (issues: a duplicate, fixed by a merged PR, moot).
    pub closes: std::collections::HashMap<String, String>,
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

/// Which todo list: dv list's PRs (and replacements), or dv issues' issues. Each has its own
/// ranking.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Of {
    Prs,
    Issues,
}

fn path(of: Of) -> Option<std::path::PathBuf> {
    let name = match of {
        Of::Prs => "todo.json",
        Of::Issues => "todo-issues.json",
    };
    crate::claude::dv_dir().ok().map(|d| d.join(name))
}

/// `s` seconds from now, to the nearest unit: "40s", "3m", "4h", "2d" (four hours just set
/// reads 4h, not the 3h `ago_secs` would round it down to).
pub fn in_secs(s: i64) -> String {
    match s.max(0) {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m", (s + 30) / 60),
        s if s < 86400 => format!("{}h", (s + 1800) / 3600),
        s => format!("{}d", (s + 43200) / 86400),
    }
}

/// What a deferral waits for, besides its needs changing: a time (`s` then 1 / 4 / d / w), or
/// something dv can see happen (`s` then `u`, in your words: dv reads the usual phrasings, and
/// has Claude put the rest as one of these).
#[derive(Clone, Debug, PartialEq)]
pub enum Until {
    /// Seconds since the epoch.
    Time(i64),
    /// That PR merged; closed (merged or not).
    Merged(u64),
    Closed(u64),
    /// This PR's checks all passing.
    Green,
    /// An approval on this PR (from that login, or anyone); any review from that login.
    Approved(Option<String>),
    Reviewed(String),
    /// A push to this PR: its head no longer this commit (filled in as it's deferred).
    Pushed(String),
}

impl Until {
    pub fn says(&self) -> String {
        match self {
            Until::Time(t) => format!("in {}", in_secs(t - crate::claude::now_secs())),
            Until::Merged(n) => format!("#{n} is merged"),
            Until::Closed(n) => format!("#{n} is closed"),
            Until::Green => "its checks pass".into(),
            Until::Approved(Some(who)) => format!("@{who} approves it"),
            Until::Approved(None) => "it's approved".into(),
            Until::Pushed(h) if h.is_empty() => "the author pushes".into(),
            Until::Pushed(h) => format!("the author pushes (past {})", h.get(..7).unwrap_or(h)),
            Until::Reviewed(who) => format!("@{who} reviews it"),
        }
    }

    fn to_json(&self) -> Value {
        match self {
            Until::Time(t) => json!({ "at": t }),
            Until::Merged(n) => json!({ "merged": n }),
            Until::Closed(n) => json!({ "closed": n }),
            Until::Green => json!({ "green": true }),
            Until::Approved(who) => json!({ "approved": who }),
            Until::Reviewed(who) => json!({ "reviewed": who }),
            Until::Pushed(head) => json!({ "pushed": head }),
        }
    }

    /// From the JSON `to_json` writes (and Claude answers in).
    pub fn from_json(v: &Value) -> Option<Until> {
        let s = |k: &str| v[k].as_str().map(|s| s.trim_start_matches('@').to_owned());
        Some(if let Some(t) = v["at"].as_i64() {
            Until::Time(t)
        } else if let Some(t) = v["at"].as_str() {
            Until::Time(crate::gh::epoch_secs(t)?)
        } else if let Some(n) = v["merged"].as_u64() {
            Until::Merged(n)
        } else if let Some(n) = v["closed"].as_u64() {
            Until::Closed(n)
        } else if v["green"] == true {
            Until::Green
        } else if v.get("approved").is_some() {
            Until::Approved(s("approved"))
        } else if v.get("pushed").is_some() {
            Until::Pushed(s("pushed").unwrap_or_default()) // (Claude says `true`: filled in as it's deferred)
        } else {
            Until::Reviewed(s("reviewed")?)
        })
    }

    /// The usual phrasings, without Claude: "#2380 is merged", "2380 closed", "CI is green",
    /// "checks pass", "@amy approves", "it's approved", "@bo reviews", "3h", "2 days", "a week".
    /// In a sentence ("Don't merge until …"), only what follows its last "until": what comes
    /// before is what waits (but a PR named there counts: "#12, until it's merged").
    pub fn parse(text: &str, now: i64) -> Option<Until> {
        let t = text.trim().to_lowercase();
        match t.rsplit_once("until ") {
            Some((before, after)) => {
                let named = before.split(|c: char| c.is_whitespace() || c == ',').find_map(|w| w.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '#').strip_prefix('#')?.parse::<u64>().ok());
                Self::parse_one(after, named, now)
            }
            None => Self::parse_one(&t, None, now),
        }
    }

    fn parse_one(t: &str, named: Option<u64>, now: i64) -> Option<Until> {
        let t = t.trim();
        let words: Vec<&str> = t.split(|c: char| c.is_whitespace() || c == ',').filter(|w| !w.is_empty()).collect();
        let has = |w: &str| words.iter().any(|x| x.starts_with(w));
        let num = words.iter().find_map(|w| w.trim_start_matches('#').parse::<u64>().ok().filter(|_| w.starts_with('#') || words.len() <= 4)).or(named);
        let who = words.iter().find_map(|w| w.strip_prefix('@')).map(|w| w.trim_end_matches(|c: char| !c.is_alphanumeric() && c != '-').to_owned());
        // A duration: "3h", "3 hours", "2d", "a week".
        let unit = |w: &str| match w.trim_end_matches('s') {
            "h" | "hr" | "hour" => Some(3600),
            "d" | "day" => Some(86400),
            "w" | "wk" | "week" => Some(7 * 86400),
            _ => None,
        };
        let dur = words.windows(2).find_map(|p| Some(p[0].replace('a', "1").parse::<i64>().ok()? * unit(p[1])?)).or_else(|| {
            words.iter().find_map(|w| {
                let at = w.find(|c: char| !c.is_ascii_digit())?;
                Some(w[..at].parse::<i64>().ok()? * unit(&w[at..])?)
            })
        });
        Some(match () {
            _ if has("merge") => Until::Merged(num?),
            _ if has("close") => Until::Closed(num?),
            _ if (has("ci") || has("check") || has("test")) && (has("green") || has("pass")) => Until::Green,
            _ if has("approv") => Until::Approved(who),
            // What the author does next: "addresses the reviews", "pushes", "updates it" (no one
            // else named, no other PR, no time: those are someone else's doing, or a while). As
            // `p` posts it, past the head it was at: "the author pushes (past abc1234)".
            _ if who.is_none() && dur.is_none() && ["address", "push", "updat", "respond", "fixes", "rebase"].iter().any(|w| has(w)) => {
                let head = words.windows(2).find(|p| p[0].trim_start_matches('(') == "past").map(|p| p[1].trim_matches(|c: char| !c.is_ascii_alphanumeric())).filter(|w| (7..=40).contains(&w.len()) && w.chars().all(|c| c.is_ascii_hexdigit()));
                match (head, num) {
                    (Some(h), _) => Until::Pushed(h.to_owned()),
                    (None, None) => Until::Pushed(String::new()),
                    (None, Some(_)) => return None, // (another PR's: for Claude to read)
                }
            }
            _ if has("review") => Until::Reviewed(who?),
            _ => Until::Time(now + dur?),
        })
    }
}

/// What a PR's conversation says it waits on: the newest line of a trusted comment (`said`,
/// oldest first) that starts "Waiting on", "Blocked on/by", "Depends on" or "Deferred until",
/// and who said it; a later "Unblocked" (or "No longer waiting") ends it. `#N` alone means that
/// PR merged.
pub fn waiting_on(said: &[(String, String)], now: i64) -> Option<(Until, String)> {
    const LEADS: [&str; 8] = ["waiting on", "waiting for", "blocked on", "blocked by", "depends on", "deferred until", "defer until", "until"];
    for (who, body) in said.iter().rev() {
        for line in body.lines().rev() {
            let l = line.trim().trim_start_matches(['>', '*', '-', ' ']).to_lowercase();
            if l.starts_with("unblocked") || l.starts_with("no longer waiting") {
                return None;
            }
            let Some(rest) = LEADS.iter().find_map(|p| l.strip_prefix(p)) else { continue };
            let rest = rest.trim_start_matches([':', ' ']);
            let bare = rest.trim_end_matches(['.', ' ', ')']).trim_start_matches('#').parse::<u64>().ok();
            // (A push from no known head: nothing to tell it by.)
            if let Some(c) = bare.map(Until::Merged).or_else(|| Until::parse(rest, now)).filter(|c| *c != Until::Pushed(String::new())) {
                return Some((c, who.clone()));
            }
        }
    }
    None
}

/// A deferral (`s`): till when (`i64::MAX` for one waiting on `cond`), the item's needs then (it
/// comes back early when they change), and what it waits for, if not a time.
#[derive(Clone, Debug, PartialEq)]
pub struct Deferral {
    pub until: i64,
    pub needs: String,
    pub cond: Option<Until>,
}

/// Items deferred (`s`), by key.
pub type Deferred = std::collections::HashMap<String, Deferral>;

/// Where `of`'s deferrals are kept (each list its own: neither writes over the other's).
pub fn deferred_path_of(of: Of) -> Option<std::path::PathBuf> {
    let name = match of {
        Of::Prs => "deferred.json",
        Of::Issues => "deferred-issues.json",
    };
    crate::claude::dv_dir().ok().map(|d| d.join(name))
}

/// What's deferred (in file `p`), the expired ones gone.
pub fn load_deferred(p: Option<&std::path::Path>) -> Deferred {
    let v: Value = p.and_then(|p| serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()).unwrap_or_default();
    let now = crate::claude::now_secs();
    v.as_object()
        .into_iter()
        .flatten()
        .filter_map(|(k, e)| {
            let cond = e.get("cond").and_then(Until::from_json);
            Some((k.clone(), Deferral { until: e["until"].as_i64()?, needs: e["needs"].as_str().unwrap_or("").to_owned(), cond }))
        })
        .filter(|(_, d)| d.until > now)
        .collect()
}

pub fn save_deferred(p: Option<&std::path::Path>, d: &Deferred) -> Result<()> {
    let v: serde_json::Map<String, Value> = d
        .iter()
        .map(|(k, d)| {
            let mut e = json!({ "until": d.until, "needs": d.needs });
            if let Some(c) = &d.cond {
                e["cond"] = c.to_json();
            }
            (k.clone(), e)
        })
        .collect();
    let p = p.context("no dv directory")?;
    std::fs::create_dir_all(p.parent().unwrap_or(p))?;
    Ok(std::fs::write(p, Value::Object(v).to_string())?)
}

/// How long `s` then `c` defers an item for: (seconds, in words). `1` an hour, `4` four hours,
/// `d` a day, else a week.
pub fn defer_secs(c: char) -> (i64, &'static str) {
    match c {
        '1' => (3600, "an hour"),
        '4' => (4 * 3600, "four hours"),
        'd' => (86400, "a day"),
        _ => (7 * 86400, "a week"),
    }
}

/// `items` with what's deferred (and still needs what it did then, and isn't `met`) moved to the
/// end, marked.
pub fn defer(items: Vec<Item>, deferred: &Deferred, now: i64, met: impl Fn(&Item, &Until) -> bool) -> Vec<Item> {
    let waiting = |it: &Item| {
        deferred.get(&it.key()).filter(|d| d.until > now && d.needs == it.needs_key() && !d.cond.as_ref().is_some_and(|c| met(it, c)))
    };
    let (mut later, mut out): (Vec<Item>, Vec<Item>) = items.into_iter().partition(|it| waiting(it).is_some());
    later.iter_mut().for_each(|it| it.deferred = deferred.get(&it.key()).cloned());
    out.append(&mut later);
    out
}

/// Claude puts `text` (a deferral's "until …", in your words) as one of `Until`'s, in the
/// background: one small run, reading only.
pub fn interpret(text: String, root: std::path::PathBuf) -> Receiver<Result<Until>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let prompt = format!(
            "A maintainer defers a pull request until: \"{text}\". It's {} now (UTC). Reply with only one JSON object \
             saying when, one of: {{\"merged\": N}} (PR N merged), {{\"closed\": N}} (PR N closed), {{\"green\": true}} \
             (this PR's checks pass), {{\"approved\": \"login\"}} or {{\"approved\": null}} (an approval), \
             {{\"reviewed\": \"login\"}} (a review from them), {{\"pushed\": true}} (the author pushes to it: \
             addresses the reviews, updates it), {{\"at\": \"2026-10-20T09:00:00Z\"}} (a time). If it's \
             none of these, {{\"none\": \"why, in a few words\"}}. No other text.",
            crate::claude::now()
        );
        let res = (|| {
            let out = crate::claude::claude_reader(&prompt, &root, &[]).stdin(std::process::Stdio::null()).output()?;
            let v: Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
            anyhow::ensure!(v["is_error"] == false, "Claude couldn't read it ({})", out.status);
            let answer = v["result"].as_str().unwrap_or("");
            let w = crate::claude::json_in(answer, '{').filter(Value::is_object).context("no answer")?;
            if let Some(why) = w["none"].as_str() {
                anyhow::bail!("not something dv can watch for: {why}");
            }
            Until::from_json(&w).context("not something dv can watch for")
        })();
        _ = tx.send(res);
    });
    rx
}

/// The last ranking of `of` (`todo.json`, `todo-issues.json` in dv's directory), if any.
pub fn load_of(of: Of) -> Option<Ranked> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(path(of)?).ok()?).ok()?;
    let items = v["items"].as_array()?.iter().map(|i| (i["key"].as_str().unwrap_or("").to_owned(), i["says"].as_str().unwrap_or("").to_owned())).collect();
    let waits = v["waits"].as_object().into_iter().flatten().filter_map(|(k, w)| Some((k.clone(), Until::from_json(w)?))).collect();
    let closes = v["closes"].as_object().into_iter().flatten().filter_map(|(k, w)| Some((k.clone(), w.as_str()?.to_owned()))).collect();
    Some(Ranked { at: v["at"].as_i64().unwrap_or(0), cost: v["cost"].as_f64().unwrap_or(0.0), fingerprint: v["fingerprint"].as_str().unwrap_or("").to_owned(), items, waits, closes })
}

fn save(r: &Ranked, of: Of) -> Result<()> {
    let items: Vec<Value> = r.items.iter().map(|(k, s)| json!({ "key": k, "says": s })).collect();
    let p = path(of).context("no dv directory")?;
    std::fs::create_dir_all(p.parent().unwrap_or(&p))?;
    let waits: serde_json::Map<String, Value> = r.waits.iter().map(|(k, w)| (k.clone(), w.to_json())).collect();
    Ok(std::fs::write(p, json!({ "at": r.at, "cost": r.cost, "fingerprint": r.fingerprint, "items": items, "waits": waits, "closes": r.closes }).to_string())?)
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

/// Claude ranks `of`'s items (`lines`, as `about` / `about_issue` put each), in the background;
/// kept as `of`'s last ranking.
pub fn rank_of(lines: Vec<String>, fingerprint: String, root: std::path::PathBuf, of: Of) -> Receiver<Result<Ranked>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let res = ask(&lines, &root, of).map(|((items, waits, closes), cost)| Ranked { at: crate::claude::now_secs(), cost, fingerprint, items, waits, closes });
        if let Ok(r) = &res {
            _ = save(r, of);
        }
        _ = tx.send(res);
    });
    rx
}

type Order = (Vec<(String, String)>, std::collections::HashMap<String, Until>, std::collections::HashMap<String, String>);

/// Claude's order and the waits it saw, and what it cost.
fn ask(lines: &[String], root: &std::path::Path, of: Of) -> Result<(Order, f64)> {
    let notes = crate::claude::notes_preamble();
    let (what, judge, key) = match of {
        Of::Prs => (
            "These open PRs (and replacements: PRs you drafted to replace others) are waiting on me",
            "fixes for bugs and security over refactors and features; contributors who've waited longest; quick ones \
             first among equals; a PR that blocks others before them. Read a PR (`gh pr view N`) only where its line \
             isn't enough to judge. If a PR's conversation (`gh pr view N --comments`, where you read it)",
            "pr-12",
        ),
        Of::Issues => (
            "These open issues are waiting on me (to triage, decide on, close, or get fixed)",
            "bugs users hit and security over features and chores; what Claude's triage says (its priority, whether to \
             close it); one a merged PR already fixes first (quick to close); issues that block others before them. \
             Read an issue (`gh issue view N`) only where its line isn't enough to judge.",
            "issue-12",
        ),
    };
    // (What a PR waits on, as dv can watch for it: PRs only.)
    let waits = match of {
        Of::Prs => " says it's waiting on something dv can watch for, add \"waits_on\", one of: {\"merged\": N}, \
                    {\"closed\": N}, {\"green\": true}, {\"approved\": \"login\"}, {\"reviewed\": \"login\"}.",
        // (What to close, said apart from the words: dv's close tab lists them.)
        Of::Issues => " For one I should close (a duplicate, fixed by a merged PR, answered, moot), add \"close\": \"why, \
                       a few words\".",
    };
    let prompt = format!(
        "{notes}I maintain this repo. {what}, one JSON object each, with what each needs from me:\n{}\n\n\
         Put them in the order I should take them, most important first. Judge what matters: {judge}{waits} \
         Change nothing. Reply with only \
         a JSON array, every key exactly once, no other text: [{{\"key\": \"{key}\", \"reason\": \"why it's here, a \
         few words\", \"next\": \"what I should do, a few words\"}}]",
        lines.join("\n")
    );
    let tools = ["Bash(gh pr view:*)", "Bash(gh pr diff:*)", "Bash(gh issue view:*)"];
    let out = crate::claude::claude_reader(&prompt, root, &tools).stdin(std::process::Stdio::null()).output()?;
    let v: Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
    anyhow::ensure!(v["is_error"] == false, "Claude couldn't rank them ({})", out.status);
    Ok((parse(v["result"].as_str().unwrap_or(""))?, v["total_cost_usd"].as_f64().unwrap_or(0.0)))
}

/// The order in Claude's answer (a JSON array, maybe with words around it): (key, "reason\nnext"),
/// and what it saw any wait on.
fn parse(answer: &str) -> Result<Order> {
    let v = crate::claude::json_in(answer, '[').filter(Value::is_array).context("no ranking in Claude's answer")?;
    let s = |v: &Value| v.as_str().unwrap_or("").trim().to_owned();
    let all = v.as_array().context("no ranking")?;
    let order = all
        .iter()
        .map(|i| {
            let (reason, next) = (s(&i["reason"]), s(&i["next"]));
            // (Apart on a line each, so a dash in either can't be taken for where they meet.)
            let says = if next.is_empty() { reason } else { format!("{reason}\n{next}") };
            (s(&i["key"]), says)
        })
        .filter(|(k, _)| !k.is_empty())
        .collect();
    let waits = all.iter().filter_map(|i| Some((s(&i["key"]), Until::from_json(i.get("waits_on")?)?))).collect();
    let closes = all.iter().map(|i| (s(&i["key"]), s(&i["close"]))).filter(|(k, c)| !k.is_empty() && !c.is_empty()).collect();
    Ok((order, waits, closes))
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
        crate::claude::Review { head: head.into(), summary: String::new(), description: String::new(), closing: String::new(), replace: String::new(), close: String::new(), threads }
    }

    #[test]
    fn what_needs_you() {
        let extra = |mine: Option<(&str, &str)>, requested, head: &str| Extra {
            my_review: mine.map(|(s, c)| (s.into(), c.into())),
            reviews: vec![],
            said: vec![],
            requested,
            mergeable: "MERGEABLE".into(),
            head_oid: head.into(),
            base: "main".into(),
            unsigned: 0,
            signatures_required: false,
            awaiting: 0,
        };
        let facts = |p: &'static Listed, e: Option<&'static Extra>, r: Option<&'static crate::claude::Review>| Facts { pr: p, extra: e, me: "me", review: r, looked: Some(LOOKED), review_changed: Some(50), unpushed: (0, 0), head_at: None, ci_note: None };
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
        // Claude said what it "Changed:" after the PR's head was made: Claude's, to check and push.
        let mut t = Thread { claude: Some("c1".into()), ..Default::default() };
        t.comments = vec![crate::gh::Comment { author: crate::claude::AUTHOR.into(), body: "Changed: renamed it".into(), created: "2026-10-18T10:00:00Z".into(), ..Default::default() }];
        let r = leak(crate::claude::Review { head: String::new(), summary: String::new(), description: String::new(), closing: String::new(), replace: String::new(), close: String::new(), threads: vec![t] });
        let mut f = facts(leak(pr(8, "me")), Some(leak(extra(None, false, "h"))), Some(r));
        f.unpushed = (2, 0);
        f.head_at = crate::gh::epoch_secs("2026-10-18T09:00:00Z");
        assert_eq!(needs(&f).0[0], Need::ClaudeChanged(2, 0));
        assert!(Need::ClaudeChanged(2, 0).says().starts_with("Claude made changes, not pushed: 2 uncommitted files"));
        assert!(Need::ClaudeChanged(2, 0).weight() > Need::Unpushed(2, 0).weight());
        // Its changes from before that head: pushed already; what's here now is someone's.
        f.head_at = crate::gh::epoch_secs("2026-10-18T11:00:00Z");
        assert_eq!(needs(&f).0[0], Need::Unpushed(2, 0));
        // Your draft, green and pushed: mark it ready; red, still running, or unpushed: not yet.
        let draft = |checks| {
            let mut p = pr(9, "me");
            (p.draft, p.checks) = (true, checks);
            facts(leak(p), Some(leak(extra(None, false, "h"))), None)
        };
        assert!(needs(&draft(Some((4, 0, 0)))).0.contains(&Need::MarkReady));
        for checks in [Some((3, 1, 0)), Some((3, 0, 1)), Some((0, 0, 0)), None] {
            assert!(!needs(&draft(checks)).0.contains(&Need::MarkReady), "{checks:?}");
        }
        let mut f = draft(Some((4, 0, 0)));
        f.unpushed = (1, 0);
        assert!(!needs(&f).0.contains(&Need::MarkReady));
        // Unsigned commits where its base requires signed ones: nothing merges until they're
        // signed (yours to sign, or the author's); not required, not a need.
        let unsigned = |required| {
            let mut e = extra(None, false, "h");
            (e.unsigned, e.signatures_required) = (1, required);
            facts(leak(pr(10, "amy")), Some(leak(e)), None)
        };
        assert!(needs(&unsigned(true)).0.contains(&Need::Unsigned(1, false)));
        assert!(!needs(&unsigned(false)).0.iter().any(|n| matches!(n, Need::Unsigned(..))));
        // Approved, green, mergeable, but with commits GitHub would refuse: not "merge it".
        let mut p = pr(12, "amy");
        (p.decision, p.checks) = ("APPROVED".into(), Some((3, 0, 0)));
        let mut e = extra(None, false, "h");
        (e.unsigned, e.signatures_required) = (1, true);
        let n = needs(&facts(leak(p), Some(leak(e)), None)).0;
        assert!(n.contains(&Need::Unsigned(1, false)) && !n.contains(&Need::ReadyToMerge), "{n:?}");
        assert_eq!(Need::Unsigned(1, false).says(), "1 commit unsigned, and its base requires signed ones: ask the author to sign them");
        // Failing checks: a need (yours to fix, or the author's); not on someone else's draft.
        let ci = |author: &str, draft: bool| {
            let mut p = pr(11, author);
            (p.checks, p.draft) = (Some((5, 2, 0)), draft);
            needs(&facts(leak(p), Some(leak(extra(None, false, "h"))), None)).0
        };
        assert!(ci("amy", false).contains(&Need::ChecksFailing(2, false)));
        assert!(ci("me", true).contains(&Need::ChecksFailing(2, true)));
        assert!(!ci("amy", true).iter().any(|n| matches!(n, Need::ChecksFailing(..))));
        assert_eq!(Need::ChecksFailing(2, true).says(), "2 checks failing: fix them");
        // Claude's part on yours (a draft too): fix CI; not while changes wait to be pushed.
        let fix = |author: &str, unpushed| {
            let mut p = pr(11, author);
            (p.checks, p.draft) = (Some((5, 2, 0)), true);
            let mut f = facts(leak(p), Some(leak(extra(None, false, "h"))), None);
            f.unpushed = unpushed;
            needs(&f).1
        };
        assert_eq!(fix("me", (0, 0)), Some(Task::FixChecks));
        assert_eq!(fix("me", (1, 0)), None);
        assert_eq!(fix("amy", (0, 0)), None);
        // Claude found it isn't the PR's doing, at its head: no fixing it again; at an older
        // head, it looks again.
        let mut p = pr(11, "me");
        p.checks = Some((5, 2, 0));
        let note = |head: &str| crate::claude::CiNote { head: head.into(), why: "flaky".into(), task: "pin the toolchain".into(), rerun: false };
        let (here, old) = (note("h"), note("old"));
        let mut f = facts(leak(p), Some(leak(extra(None, false, "h"))), None);
        f.ci_note = Some(&here);
        let (n, c) = needs(&f);
        assert!(n.contains(&Need::CiNotThisPr("flaky".into(), true)) && c != Some(Task::FixChecks), "{n:?} {c:?}");
        assert!(Need::CiNotThisPr("flaky".into(), true).says().ends_with("N N drafts a PR fixing it, a a has it try again"));
        f.ci_note = Some(&old);
        assert_eq!(needs(&f).1, Some(Task::FixChecks));
        // Workflow runs waiting for a maintainer's approval: a need, yours to do (W W).
        let mut e = extra(None, false, "h");
        e.awaiting = 2;
        assert!(needs(&facts(leak(pr(13, "amy")), Some(leak(e)), None)).0.contains(&Need::ApproveRuns(2)));
        assert_eq!(Need::ApproveRuns(1).says(), "1 workflow run awaiting approval: approve it (W W)");
        // With Claude's reason on the card, what holds the PR up goes first, in short.
        let it = Item { kind: crate::todo::Kind::Pr, number: 1, needs: vec![Need::ApproveRuns(3), Need::ChecksFailing(2, false), Need::NoReview], claude: None, updated: String::new(), reason: Some("bug fix — review".into()), changed: false, deferred: None, busy: None, waits: None };
        assert_eq!(it.tags(), ["⏸ 3 runs to approve (W W)", "✗ 2 checks failing"]);
        assert_eq!(it.says().split(" · ").count(), 3);
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
        let it = |n, needs: Vec<Need>, updated: &str| Item { kind: crate::todo::Kind::Pr, number: n, needs, claude: None, updated: updated.into(), reason: None, changed: false, deferred: None, busy: None, waits: None };
        let items = vec![it(1, vec![Need::NoReview], "b"), it(2, vec![Need::ReviewRequested], "c"), it(3, vec![Need::NoReview], "a")];
        // Rules: the most pressing first, then the longest waiting.
        let (o, since) = order(items.clone(), None);
        assert_eq!(since, Since::default());
        assert!(o.iter().all(|i| !i.changed)); // nothing to have changed since
        assert_eq!(o.iter().map(|i| i.number).collect::<Vec<_>>(), [2, 3, 1]);
        // Claude's order, with its word; a new one goes before the first that needs you less.
        let ranked = Ranked { at: 1, cost: 0.1, fingerprint: fingerprint(&items), waits: Default::default(), closes: Default::default(), items: vec![("pr-1".into(), "small fix — merge".into()), ("pr-3".into(), "x".into())] };
        let (o, since) = order(items.clone(), Some(&ranked));
        assert_eq!(since, Since::default()); // it's of the same three (pr-2 left out of Claude's answer goes by the rules)
        assert_eq!(o.iter().map(|i| i.number).collect::<Vec<_>>(), [2, 1, 3]);
        assert_eq!(o[1].reason.as_deref(), Some("small fix — merge"));
        assert_eq!(o[0].says(), "your review is requested");
        // Since: one new, one whose needs changed, one gone; those two marked.
        let mut now = vec![it(1, vec![Need::ReviewRequested], "b"), it(3, vec![Need::NoReview], "a"), it(9, vec![Need::NoReview], "d")];
        now[0].needs.push(Need::NoReview);
        let (o, since) = order(now, Some(&ranked));
        assert_eq!(since, Since { new: 1, changed: 1, gone: 1 });
        assert_eq!(o.iter().filter(|i| i.changed).map(|i| i.number).collect::<Vec<_>>().len(), 2);
        // Claude's answer, words around it.
        let (p, waits, closes) = parse("Here: [{\"key\": \"pr-2\", \"reason\": \"bug fix\", \"next\": \"review it\"}, {\"key\": \"rep-12\", \"reason\": \"ready\", \"waits_on\": {\"merged\": 2380}}, {\"key\": \"issue-5\", \"reason\": \"dup\", \"close\": \"duplicate of #4\"}]").unwrap();
        assert_eq!(closes.get("issue-5").map(String::as_str), Some("duplicate of #4"));
        assert_eq!(p, [("pr-2".to_owned(), "bug fix\nreview it".to_owned()), ("rep-12".to_owned(), "ready".to_owned()), ("issue-5".to_owned(), "dup".to_owned())]);
        assert_eq!(waits.get("rep-12"), Some(&Until::Merged(2380))); // what it saw the conversation say
    }

    fn items_needs() -> String {
        Need::ReadyToMerge.weight().to_string()
    }

    #[test]
    fn waiting_on_from_comments() {
        let c = |who: &str, body: &str| (who.to_owned(), body.to_owned());
        let now = 0;
        assert_eq!(waiting_on(&[c("bo", "Looks good.\n\nBlocked on #2380.")], now), Some((Until::Merged(2380), "bo".into())));
        assert_eq!(waiting_on(&[c("me", "Waiting on: CI to pass")], now), Some((Until::Green, "me".into())));
        assert_eq!(waiting_on(&[c("me", "waiting on @amy approves it")], now), Some((Until::Approved(Some("amy".into())), "me".into())));
        assert_eq!(waiting_on(&[c("me", "Depends on #12 merged")], now), Some((Until::Merged(12), "me".into())));
        // The newest says; a later Unblocked ends it; other talk isn't a wait.
        assert_eq!(waiting_on(&[c("me", "Blocked on #1"), c("bo", "Blocked on #2")], now).map(|w| w.0), Some(Until::Merged(2)));
        assert_eq!(waiting_on(&[c("me", "Blocked on #1"), c("me", "Unblocked: #1 is in")], now), None);
        assert_eq!(waiting_on(&[c("me", "I'm waiting to see what amy thinks")], now), None);
        // What `p` posts reads back as itself.
        for u in [Until::Merged(2380), Until::Green, Until::Approved(Some("amy".into())), Until::Reviewed("bo".into()), Until::Closed(7), Until::Pushed("abc1234".into())] {
            assert_eq!(waiting_on(&[c("me", &format!("Waiting on: {}.", u.says()))], now).map(|w| w.0), Some(u));
        }
    }

    #[test]
    fn deferral_conditions() {
        let now = 1_000_000;
        let p = |t: &str| Until::parse(t, now);
        assert_eq!(p("#2380 is merged"), Some(Until::Merged(2380)));
        assert_eq!(p("until 2380 closed"), Some(Until::Closed(2380)));
        assert_eq!(p("CI is green"), Some(Until::Green));
        assert_eq!(p("checks pass"), Some(Until::Green));
        assert_eq!(p("@amy approves"), Some(Until::Approved(Some("amy".into()))));
        assert_eq!(p("it's approved"), Some(Until::Approved(None)));
        assert_eq!(p("@bo reviews it"), Some(Until::Reviewed("bo".into())));
        assert_eq!(p("3h"), Some(Until::Time(now + 3 * 3600)));
        assert_eq!(p("2 days"), Some(Until::Time(now + 2 * 86400)));
        assert_eq!(p("a week"), Some(Until::Time(now + 7 * 86400)));
        assert_eq!(p("the release is out"), None); // for Claude to read
        assert_eq!(p("Can't do anything until author addresses reviews"), Some(Until::Pushed(String::new())));
        assert_eq!(p("the author pushes"), Some(Until::Pushed(String::new())));
        assert_eq!(p("the author pushes (past abc1234)"), Some(Until::Pushed("abc1234".into())));
        assert_eq!(p("Blocked on #12 until it's merged"), Some(Until::Merged(12)));
        assert_eq!(p("#12, until it's merged"), Some(Until::Merged(12)));
        assert_eq!(p("Blocked (see #12) until it's merged"), Some(Until::Merged(12)));
        assert_eq!(p("Don't approve until the release"), None); // what's before "until" is what waits
        assert_eq!(p("won't review until @amy pushes"), None);
        assert_eq!(p("the author addresses the defaced docs"), Some(Until::Pushed(String::new())));
        assert_eq!(p("@amy reviews the fixes"), Some(Until::Reviewed("amy".into())));
        assert_eq!(p("3 days after the push"), Some(Until::Time(now + 3 * 86400)));
        // Round trip, as kept and as Claude answers.
        for u in [Until::Merged(1), Until::Green, Until::Approved(None), Until::Reviewed("x".into()), Until::Time(5), Until::Pushed("abc".into())] {
            assert_eq!(Until::from_json(&u.to_json()), Some(u));
        }
        assert_eq!(Until::from_json(&json!({ "at": "2026-10-20T09:00:00Z" })), crate::gh::epoch_secs("2026-10-20T09:00:00Z").map(Until::Time));
    }

    #[test]
    fn deferring() {
        let it = |n, needs: Vec<Need>| Item { kind: crate::todo::Kind::Pr, number: n, needs, claude: None, updated: String::new(), reason: None, changed: false, deferred: None, busy: None, waits: None };
        let items = vec![it(1, vec![Need::ReadyToMerge]), it(2, vec![Need::NoReview])];
        let d: Deferred = [("pr-1".to_owned(), Deferral { until: 200, needs: items[0].needs_key(), cond: None })].into();
        // Deferred: to the end, marked, until then.
        let never = |_: &Item, _: &Until| false;
        let o = defer(items.clone(), &d, 100, never);
        assert_eq!((o[0].number, o[1].number, o[1].deferred.as_ref().map(|d| d.until)), (2, 1, Some(200)));
        // Past it: back in place.
        assert_eq!(defer(items.clone(), &d, 300, never)[0].number, 1);
        // Its needs changed: back before then.
        let mut changed = items;
        changed[0].needs.push(Need::NewActivity);
        assert_eq!(defer(changed, &d, 100, never)[0].deferred, None);
        // Until something happens: deferred till it has.
        let d: Deferred = [("pr-1".to_owned(), Deferral { until: i64::MAX, needs: items_needs(), cond: Some(Until::Merged(2380)) })].into();
        let items = vec![it(1, vec![Need::ReadyToMerge]), it(2, vec![Need::NoReview])];
        assert_eq!(defer(items.clone(), &d, 100, never)[1].number, 1);
        assert_eq!(defer(items, &d, 100, |_, c| *c == Until::Merged(2380))[0].number, 1);
    }
}
