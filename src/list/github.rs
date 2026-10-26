//! What `dv list` asks GitHub: the open PRs (cards), what its todo tab needs of each, their
//! checks, and whether any of it changed (an ETag); and the list kept up with it, in the
//! background.
use crate::gh::{awaiting_approval, etag_of, gh_json, next_page, requires_signatures, PAGE, SIGNATURE_RULES};
use super::*;
use anyhow::{Context, Result};
use serde_json::Value;

/// A PR for `dv list`.
#[derive(Clone, Default)]
pub struct Listed {
    pub number: u64,
    pub title: String,
    pub author: String,
    pub base: String,
    pub branch: String,
    pub draft: bool,
    /// OPEN, CLOSED or MERGED.
    pub state: String,
    pub created: String,
    pub updated: String,
    pub additions: u64,
    pub deletions: u64,
    pub files: u64,
    /// APPROVED, CHANGES_REQUESTED, REVIEW_REQUIRED or empty.
    pub decision: String,
    /// (name, hex color)
    pub labels: Vec<(String, String)>,
    /// (passed, failed, pending) of the checks that decide (its base's required ones, when it
    /// names any; else all), filled in by `list_checks` (slow, so fetched separately).
    pub checks: Option<(usize, usize, usize)>,
    /// The optional checks (not required) that fail: shown, but they don't hold it back.
    pub optional_failing: usize,
    /// The commit `checks` are on (the PR's head when they were fetched).
    pub checks_head: String,
}

impl Listed {
    /// Open, as far as the list knows (no state: one made up here, as in tests).
    pub fn open(&self) -> bool {
        self.state == "OPEN" || self.state.is_empty()
    }
}

/// The state of a PR the list hasn't looked up yet (`Listed::state`): not open, as far as it knows.
pub const LOOKING: &str = "…";

const LISTED: &str = "number,title,author,baseRefName,headRefName,isDraft,state,createdAt,updatedAt,additions,deletions,changedFiles,reviewDecision,labels";

fn listed(p: &Value) -> Listed {
    let s = |v: &Value| v.as_str().unwrap_or("").to_owned();
    let n = |v: &Value| v.as_u64().unwrap_or(0);
    Listed {
        number: n(&p["number"]),
        title: s(&p["title"]),
        author: crate::gh::login(&p["author"]),
        base: s(&p["baseRefName"]),
        branch: s(&p["headRefName"]),
        draft: p["isDraft"] == true,
        state: s(&p["state"]),
        created: s(&p["createdAt"]),
        updated: s(&p["updatedAt"]),
        additions: n(&p["additions"]),
        deletions: n(&p["deletions"]),
        files: n(&p["changedFiles"]),
        decision: s(&p["reviewDecision"]),
        labels: p["labels"].as_array().into_iter().flatten().map(|l| (s(&l["name"]), s(&l["color"]))).collect(),
        checks: None,
        optional_failing: 0,
        checks_head: String::new(),
    }
}

/// How many open PRs `dv list` fetches (the most recently updated); `#N` opens any other.
pub const LIST_MAX: usize = 1000;

pub fn list() -> Result<Vec<Listed>> {
    let v: Value = gh_json(&["pr", "list", "--limit", &LIST_MAX.to_string(), "--json", LISTED])?;
    Ok(v.as_array().context("gh pr list")?.iter().map(listed).collect())
}

/// The last `limit` merged and closed PRs (`dv list`'s done tab), most recently updated first
/// (GitHub's search order: a long-lived PR merged today among them).
pub fn done(limit: usize) -> Result<Vec<Listed>> {
    let v: Value = gh_json(&["pr", "list", "--state", "closed", "--search", "sort:updated-desc", "--limit", &limit.to_string(), "--json", LISTED])?;
    Ok(v.as_array().context("gh pr list")?.iter().map(listed).collect())
}

/// How many merged and closed PRs the done tab fetches at a time: more as you reach the end.
pub const DONE_MAX: usize = 100;

pub fn listed_one(n: u64) -> Result<Listed> {
    let v: Value = gh_json(&["pr", "view", &n.to_string(), "--json", LISTED])?;
    Ok(listed(&v))
}

/// What `dv list`'s todo tab needs to know about a PR beyond its card: your latest review
/// (its state and the commit it was on), whether your review is requested, whether it merges
/// cleanly, and its head commit.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Extra {
    pub my_review: Option<(String, String)>,
    /// The last reviews, each (not only everyone's latest: a comment's review after an approval
    /// would hide it): (login, state, when submitted).
    pub reviews: Vec<(String, String, i64)>,
    /// The conversation's last comments by people with write access (or you), oldest first:
    /// (login, body, when posted; 0 if unknown: any review counts), for what the PR waits on.
    pub said: Vec<(String, String, i64)>,
    pub requested: bool,
    /// MERGEABLE, CONFLICTING or UNKNOWN.
    pub mergeable: String,
    pub head_oid: String,
    pub base: String,
    /// Its commits without a signature GitHub verifies, and whether its base requires one.
    pub unsigned: usize,
    pub signatures_required: bool,
    /// Workflow runs on its head waiting for a maintainer's approval.
    pub awaiting: usize,
    /// What the author hasn't answered: the newest maintainer's review (not an approval) or
    /// comment since their last push, comment or review, (when, who, a review or not). None:
    /// they had the last word, or an approval since did.
    pub unanswered: Option<(i64, String, bool)>,
    /// The last sign of anyone at work on it: its author's last word (push, comment, review),
    /// or anyone's push (one taken over is not left); 0: none seen.
    pub active: i64,
}

/// You (your login), and `Extra` for the open PRs (the most recently updated, as `list` lists
/// them): number → extra. One GraphQL query (`gh pr list` leaves reviews' commits out).
pub fn list_extra() -> Result<(String, std::collections::HashMap<u64, Extra>)> {
    let (mut me, mut all, mut after) = (String::new(), std::collections::HashMap::new(), None::<String>);
    // A page at a time (100), as many as `list` lists.
    while all.len() < LIST_MAX {
        let from = after.as_ref().map_or(String::new(), |c| format!(", after: \"{c}\""));
        let q = format!(
            "query($owner: String!, $name: String!) {{ viewer {{ login }} repository(owner: $owner, name: $name) {{
               pullRequests(states: OPEN, first: 100{from}, orderBy: {{ field: UPDATED_AT, direction: DESC }}) {{ {PAGE} nodes {{
                 number headRefOid mergeable baseRefName author {{ login }} {SIGNATURE_RULES}
                 commits(last: 100) {{ nodes {{ commit {{ committedDate authoredDate committer {{ user {{ login }} }} author {{ user {{ login }} }} signature {{ isValid }} }} }} }}
                 reviewRequests(first: 30) {{ nodes {{ requestedReviewer {{ ... on User {{ login }} }} }} }}
                 latestReviews(first: 50) {{ nodes {{ author {{ login }} state commit {{ oid }} }} }}
                 reviews(last: 50) {{ nodes {{ author {{ login }} authorAssociation state submittedAt }} }}
                 comments(last: 20) {{ nodes {{ author {{ login }} authorAssociation body createdAt }} }} }} }} }} }}"
        );
        let v: Value = gh_json(&["api", "graphql", "-F", "owner={owner}", "-F", "name={repo}", "-f", &format!("query={q}")])?;
        crate::gh::refused(&v)?; // (only a base's rules may be: else a refused part reads as none)
        me = v["data"]["viewer"]["login"].as_str().unwrap_or("").to_owned();
        let list = &v["data"]["repository"]["pullRequests"];
        let prs = list["nodes"].as_array().context("gh api graphql: pull requests")?;
        all.extend(prs.iter().map(|p| (p["number"].as_u64().unwrap_or(0), extra(p, &me))));
        match next_page(list) {
            Some(c) => after = Some(c),
            None => break,
        }
    }
    // Workflow runs waiting for approval, by the PR whose head they're on.
    if let Ok(w) = awaiting_approval(None) {
        for e in all.values_mut() {
            e.awaiting = w.get(&e.head_oid).map_or(0, Vec::len);
        }
    }
    Ok((me, all))
}

pub(super) fn extra(p: &Value, me: &str) -> Extra {
    let s = |v: &Value| v.as_str().unwrap_or("").to_owned();
    let nodes = |k: &str| p[k]["nodes"].as_array().cloned().unwrap_or_default();
    let my_review = nodes("latestReviews").into_iter().find(|r| r["author"]["login"] == me).map(|r| (s(&r["state"]), s(&r["commit"]["oid"])));
    let at = |v: &Value| crate::gh::epoch_secs(&s(v)).unwrap_or(0);
    let author = crate::gh::login(&p["author"]);
    let trusted = |c: &Value| c["author"]["login"] == me || matches!(c["authorAssociation"].as_str(), Some("OWNER" | "MEMBER" | "COLLABORATOR"));
    // The author's last word (their push: not a maintainer's "Update branch"; a comment; a
    // review: a reply on a thread is one), and what maintainers said since, waiting on them
    // (bots' and drive-bys' aren't); an approval since: the ball's with the maintainers.
    // (Theirs, or no one's linked to it: a commit from an email not on their account. One
    // someone else committed, a maintainer's rebase: when they wrote it, not when it was
    // rebased. GitHub's web edits are web-flow's.)
    // (Its author, not `authors`, co-authors too: a connection per commit, of 100 commits of
    // 100 PRs, cost the query 100 of GitHub's points a page, not 6.)
    let theirs = |c: &Value| {
        let by = &c["commit"]["author"];
        by.is_object() && (by["user"]["login"] == author.as_str() || by["user"].is_null())
    };
    let when = |c: &Value| {
        let by = &c["commit"]["committer"]["user"]["login"];
        let other = by.as_str().is_some_and(|by| by != author && by != "web-flow");
        at(&c["commit"][if other { "authoredDate" } else { "committedDate" }])
    };
    let pushed = nodes("commits").iter().filter(|c| theirs(c)).map(when).max().unwrap_or(0);
    let by_author = |k: &str, t: &str| nodes(k).into_iter().filter(|c| c["author"]["login"] == author.as_str()).map(|c| at(&c[t])).max().unwrap_or(0);
    let last_word = pushed.max(by_author("comments", "createdAt")).max(by_author("reviews", "submittedAt"));
    let others = |c: &Value| c["author"]["login"] != author.as_str() && trusted(c);
    let reviews = nodes("reviews").into_iter().filter(|r| others(r) && matches!(r["state"].as_str(), Some("CHANGES_REQUESTED" | "COMMENTED")));
    let said = reviews
        .map(|r| (at(&r["submittedAt"]), crate::gh::login(&r["author"]), true))
        .chain(nodes("comments").into_iter().filter(others).map(|c| (at(&c["createdAt"]), crate::gh::login(&c["author"]), false)))
        .filter(|(t, ..)| *t > last_word)
        .max_by_key(|(t, ..)| *t);
    let approved_last = nodes("reviews").iter().filter(|r| others(r) && r["state"] == "APPROVED").map(|r| at(&r["submittedAt"])).max().unwrap_or(0);
    let unanswered = said.filter(|(t, ..)| *t > approved_last);
    Extra {
        my_review,
        reviews: nodes("reviews").iter().map(|r| (crate::gh::login(&r["author"]), s(&r["state"]), crate::gh::epoch_secs(&s(&r["submittedAt"])).unwrap_or(0))).collect(),
        said: nodes("comments")
            .iter()
            .filter(|c| trusted(c))
            .map(|c| (crate::gh::login(&c["author"]), s(&c["body"]), crate::gh::epoch_secs(&s(&c["createdAt"])).unwrap_or(0)))
            .collect(),
        requested: nodes("reviewRequests").iter().any(|r| r["requestedReviewer"]["login"] == me),
        mergeable: s(&p["mergeable"]),
        head_oid: s(&p["headRefOid"]),
        base: s(&p["baseRefName"]),
        unsigned: nodes("commits").iter().filter(|c| c["commit"]["signature"]["isValid"] != true).count(),
        signatures_required: requires_signatures(p),
        awaiting: 0, // from the workflow runs, after (`list_extra`)
        unanswered,
        active: last_word.max(nodes("commits").iter().map(|c| at(&c["commit"]["committedDate"])).max().unwrap_or(0)),
    }
}

pub type ListChecks = std::collections::HashMap<u64, ((usize, usize, usize), usize, String)>;

/// CI check counts for PRs `ns` (the list's): number → ((passed, failed, pending) of those
/// that decide (`gh::gate`), how many optional ones fail, the head they're on: as GitHub has
/// them, each check required or not (`gh::rollups`), checks and head of one answer.
pub fn list_checks(ns: Vec<u64>) -> Result<ListChecks> {
    Ok(crate::gh::rollups(&ns)?
        .into_iter()
        .map(|(n, r)| {
            let gated = crate::gh::gate(&r.checks);
            let optional_failing = r.checks.iter().filter(|c| crate::gh::optional(c, &r.checks) && crate::gh::verdict(c) == '✗').count();
            (n, (gated, optional_failing, r.head))
        })
        .collect())
}

/// The open PRs' ETag (as `list` lists them: most recently updated first), if it has changed
/// from `etag`, as a PR's (`gh::etag_of`): a push, comment, review, new or closed PR changes it; CI doesn't.
pub fn list_etag(etag: &str) -> Result<Option<String>> {
    etag_of("repos/{owner}/{repo}/pulls?state=open&sort=updated&direction=desc&per_page=100", etag)
}

impl List {
    pub(super) fn refresh(&mut self, term: &mut DefaultTerminal) -> Result<()> {
        self.msg = "loading PRs…".into();
        term.draw(|f| self.draw(f))?;
        self.apply_list(github::list(), false);
        Ok(())
    }

    /// Watch GitHub in the background for as long as dv runs: the open PRs' ETag every 5s, as
    /// the PR view does (a 304 doesn't count against the rate limit), fetching them again when it
    /// changes; and every 5 minutes regardless (CI doesn't change it). What's fetched shows as it
    /// comes (`apply_list`).
    pub(super) fn watch_github(&mut self) {
        let (tx, rx) = mpsc::channel();
        let sender = tx.clone();
        std::thread::spawn(move || {
            let mut etag = github::list_etag("").ok().flatten().unwrap_or_default();
            let mut fetched = std::time::Instant::now();
            loop {
                std::thread::sleep(Duration::from_secs(5));
                let changed = match github::list_etag(&etag) {
                    Ok(Some(e)) => (etag = e, true).1,
                    _ => false,
                };
                if changed || fetched.elapsed() >= Duration::from_secs(300) {
                    fetched = std::time::Instant::now();
                    if sender.send(github::list()).is_err() {
                        return; // dv list is done
                    }
                }
            }
        });
        self.fresh = Some((tx, rx));
    }

    /// The PRs as fetched: on the cards, keeping the selected one selected. `quiet` (in the
    /// background): no message. (The CI counts are kept till the new ones come, either way.)
    pub(super) fn apply_list(&mut self, res: Result<Vec<Listed>>, quiet: bool) {
        // (Before the PRs change: the cursor's card, as shown now.)
        let keep = self.selected();
        self.worktrees_changed();
        // (What was looked up for Enter may have moved on since: a push the list has now.)
        if let Ok(mut c) = self.pr_cache.lock() {
            let now = crate::claude::now_secs();
            let moved = |n: u64, at: &std::time::Instant| {
                let updated = res.as_ref().ok().and_then(|l| l.iter().find(|p| p.number == n)).and_then(|p| crate::gh::epoch_secs(&p.updated));
                updated.is_some_and(|u| u >= now - at.elapsed().as_secs() as i64)
            };
            c.retain(|n, (at, _)| !moved(*n, at));
        }
        self.reviewed = crate::batch::reviewed();
        self.load_reps();
        match res {
            Ok(mut prs) => {
                // CI as it was till the new counts come (shown loading meanwhile): unknown isn't
                // none, nothing failing nor ready all at once.
                for p in &mut prs {
                    if let Some(o) = self.prs.iter().find(|o| o.number == p.number) {
                        (p.checks, p.optional_failing, p.checks_head) = (o.checks, o.optional_failing, o.checks_head.clone());
                    }
                }
                let (tx, rx) = mpsc::channel();
                let ns: Vec<u64> = prs.iter().map(|p| p.number).collect();
                std::thread::spawn(move || tx.send(github::list_checks(ns)));
                self.checks = Some(rx);
                let (tx, rx) = mpsc::channel();
                std::thread::spawn(move || tx.send(github::list_extra()));
                self.extra_rx = Some(rx);
                if !quiet {
                    self.msg = if prs.is_empty() { "no open PRs".into() } else { String::new() };
                }
                // PRs of worktrees or marks that aren't in the list (not open any more, or beyond
                // it): closed ones as looked up before, else looked up in the background (a
                // GitHub call each, a second or so: the list doesn't wait on them).
                let mut wanted: Vec<u64> = self.local.keys().chain(&self.marked).filter(|n| !prs.iter().any(|p| p.number == **n)).copied().collect();
                wanted.sort();
                wanted.dedup();
                let mut missing = vec![];
                for n in wanted {
                    match self.gone.get(&n).filter(|p| !p.open()) {
                        Some(p) => prs.push(p.clone()),
                        None => {
                            if self.gone_pending.insert(n) {
                                missing.push(n);
                            }
                            prs.push(Listed { number: n, title: "(looking it up…)".into(), state: LOOKING.into(), ..Listed::default() });
                        }
                    }
                }
                if !missing.is_empty() {
                    let (tx, rx) = mpsc::channel();
                    std::thread::spawn(move || _ = tx.send(missing.into_iter().map(|n| (n, github::listed_one(n).ok())).collect()));
                    self.gone_rx.push(rx);
                }
                self.prs = prs;
                // (Deferrals kept while it's listed open, or still being looked up.)
                let open = |n: u64| self.prs.iter().any(|p| p.number == n && (p.open() || p.state == LOOKING));
                if let Some(e) = self.todos.prune(|k| match (k.strip_prefix("pr-"), k.strip_prefix("rep-")) {
                    (Some(n), _) => n.parse().is_ok_and(open),
                    (_, Some(id)) => id.parse().is_ok_and(|id: u64| self.rep_of.contains_key(&id)),
                    _ => true,
                }) {
                    self.msg = e;
                }
                // (Yours held for a PR no longer open: let go, as the issues list's; it'd only
                // find it gone.)
                self.batch.retain(|&n, s| !matches!(s, batch::Status::Busy(..)) || open(n));
                // (What's waited on, looked up again against this fetch: `find_others`, in
                // `find_todo` below; what's final kept.)
                self.others.retain(|_, s| s == "MERGED" || s == "CLOSED");
                self.prune_marks();
                self.find_new();
                self.find_todo();
                self.select_card(keep);
            }
            Err(e) => self.msg = format!("{e:#}"),
        }
    }

    /// Claude's reviews read again when one is added: started from a PR's view, it may finish
    /// after you're back here.
    pub(super) fn watch_reviews(&mut self) {
        let Some(dir) = &self.reviews_dir else { return };
        let mtime = std::fs::metadata(dir).and_then(|m| m.modified()).ok();
        if mtime != self.reviews_mtime {
            self.reviews_mtime = mtime;
            // Its review written since it failed (from its view, say): a run that failed is over.
            let since = |n: u64, at: &std::time::SystemTime| std::fs::metadata(dir.join(format!("pr-{n}.json"))).and_then(|m| m.modified()).is_ok_and(|m| m > *at);
            self.batch.retain(|&n, s| !matches!(s, crate::batch::Status::Failed(_, at) if since(n, at)));
            self.reviewed = crate::batch::reviewed_in(dir);
            // What's new, what needs you, what Claude would close: from the reviews as they are.
            self.find_new();
            self.find_todo();
        }
    }

    /// The selected PR, looked up on GitHub in the background once it's been selected a moment
    /// (not each one `j` passes): opening it then needn't wait on that, nor, the first time,
    /// on its comments (most of what opening a PR takes).
    pub(super) fn prefetch(&mut self) {
        // Its PR (a published replacement's card: its PR, as Enter opens it).
        let n = self.card_pr();
        if n != self.prefetched.0 {
            self.prefetched = (n, Some(std::time::Instant::now()), false);
        }
        if self.prefetched.2 || self.prefetched.1.is_some_and(|t| t.elapsed() < std::time::Duration::from_millis(300)) {
            return;
        }
        self.prefetched.2 = true;
        let Some(n) = n else { return };
        // Looked up a moment ago already (back on it).
        if self.pr_cache.lock().is_ok_and(|c| c.get(&n).is_some_and(|(t, _)| t.elapsed().as_secs() < 120)) {
            return;
        }
        let cache = self.pr_cache.clone();
        std::thread::spawn(move || {
            if let (Ok(p), Ok(mut c)) = (gh::pr(Some(n)), cache.lock()) {
                c.insert(n, (std::time::Instant::now(), p));
            }
            crate::view::prefetch_github(n);
        });
    }
}
