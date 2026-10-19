//! What `dv list` asks GitHub: the open PRs (cards), what its todo tab needs of each, their
//! checks, and whether any of it changed (an ETag); and the list kept up with it, in the
//! background.
use crate::gh::{awaiting_approval, etag_of, gh_json, next_page, requires_signatures, tally, PAGE, SIGNATURE_RULES};
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
    /// (passed, failed, pending), filled in by `list_checks` (slow, so fetched separately).
    pub checks: Option<(usize, usize, usize)>,
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
        author: s(&p["author"]["login"]),
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
        checks_head: String::new(),
    }
}

/// How many open PRs `dv list` fetches (the most recently updated); `#N` opens any other.
pub const LIST_MAX: usize = 1000;

pub fn list() -> Result<Vec<Listed>> {
    let v: Value = gh_json(&["pr", "list", "--limit", &LIST_MAX.to_string(), "--json", LISTED])?;
    Ok(v.as_array().context("gh pr list")?.iter().map(listed).collect())
}

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
    /// Everyone's latest review: (login, state).
    pub reviews: Vec<(String, String)>,
    /// The conversation's last comments by people with write access (or you), oldest first:
    /// (login, body), for what the PR waits on.
    pub said: Vec<(String, String)>,
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
                 number headRefOid mergeable baseRefName {SIGNATURE_RULES}
                 commits(last: 100) {{ nodes {{ commit {{ signature {{ isValid }} }} }} }}
                 reviewRequests(first: 30) {{ nodes {{ requestedReviewer {{ ... on User {{ login }} }} }} }}
                 latestReviews(first: 50) {{ nodes {{ author {{ login }} state commit {{ oid }} }} }}
                 comments(last: 20) {{ nodes {{ author {{ login }} authorAssociation body }} }} }} }} }} }}"
        );
        let v: Value = gh_json(&["api", "graphql", "-F", "owner={owner}", "-F", "name={repo}", "-f", &format!("query={q}")])?;
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

fn extra(p: &Value, me: &str) -> Extra {
    let s = |v: &Value| v.as_str().unwrap_or("").to_owned();
    let nodes = |k: &str| p[k]["nodes"].as_array().cloned().unwrap_or_default();
    let my_review = nodes("latestReviews").into_iter().find(|r| r["author"]["login"] == me).map(|r| (s(&r["state"]), s(&r["commit"]["oid"])));
    Extra {
        my_review,
        reviews: nodes("latestReviews").iter().map(|r| (s(&r["author"]["login"]), s(&r["state"]))).collect(),
        said: nodes("comments")
            .iter()
            .filter(|c| c["author"]["login"] == me || matches!(c["authorAssociation"].as_str(), Some("OWNER" | "MEMBER" | "COLLABORATOR")))
            .map(|c| (s(&c["author"]["login"]), s(&c["body"])))
            .collect(),
        requested: nodes("reviewRequests").iter().any(|r| r["requestedReviewer"]["login"] == me),
        mergeable: s(&p["mergeable"]),
        head_oid: s(&p["headRefOid"]),
        base: s(&p["baseRefName"]),
        unsigned: nodes("commits").iter().filter(|c| c["commit"]["signature"]["isValid"] != true).count(),
        signatures_required: requires_signatures(p),
        awaiting: 0, // from the workflow runs, after (`list_extra`)
    }
}

pub type ListChecks = std::collections::HashMap<u64, ((usize, usize, usize), String)>;

/// CI check counts for the PRs `list` returns: number → ((passed, failed, pending), the head
/// they're on).
pub fn list_checks() -> Result<ListChecks> {
    let v: Value = gh_json(&["pr", "list", "--limit", &LIST_MAX.to_string(), "--json", "number,headRefOid,statusCheckRollup"])?;
    let prs = v.as_array().context("gh pr list")?;
    Ok(prs
        .iter()
        .map(|p| {
            let head = p["headRefOid"].as_str().unwrap_or("").to_owned();
            (p["number"].as_u64().unwrap_or(0), (tally(p["statusCheckRollup"].as_array().map(Vec::as_slice).unwrap_or(&[])), head))
        })
        .collect())
}

/// The open PRs' ETag (as `list` lists them: most recently updated first), if it has changed
/// from `etag`, as `pr_etag`: a push, comment, review, new or closed PR changes it; CI doesn't.
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
    /// background): no message, and the CI counts kept until the new ones come.
    pub(super) fn apply_list(&mut self, res: Result<Vec<Listed>>, quiet: bool) {
        let keep = self.shown().get(self.sel).map(|p| p.number);
        self.local = crate::pr_worktrees().into_iter().map(|(n, dir, b)| (n, (dir, b))).collect();
        self.scan_unpushed();
        self.find_others();
        self.reviewed = crate::batch::reviewed();
        self.load_reps();
        match res {
            Ok(mut prs) => {
                if quiet {
                    for p in &mut prs {
                        if let Some(o) = self.prs.iter().find(|o| o.number == p.number) {
                            (p.checks, p.checks_head) = (o.checks, o.checks_head.clone());
                        }
                    }
                }
                let (tx, rx) = mpsc::channel();
                std::thread::spawn(move || tx.send(github::list_checks()));
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
                self.prune_marks();
                self.find_new();
                self.find_todo();
                self.sel = keep.and_then(|n| self.shown().iter().position(|p| p.number == n)).unwrap_or(0);
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
            self.reviewed = crate::batch::reviewed_in(dir);
        }
    }

    /// The selected PR, looked up on GitHub in the background once it's been selected a moment
    /// (not each one `j` passes): opening it then needn't wait on that, nor, the first time,
    /// on its comments (most of what opening a PR takes).
    pub(super) fn prefetch(&mut self) {
        // Not a replacement (its number is its own, not a PR's).
        let n = self.selected().filter(|s| !s.1).map(|s| s.0);
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
