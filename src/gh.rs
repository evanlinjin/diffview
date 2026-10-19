//! GitHub via the `gh` CLI: PR lookup, review threads, posting comments.
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::io::Write;
use std::process::{Command, Stdio};

#[derive(Clone, Default)]
pub struct Pr {
    pub number: u64,
    /// GraphQL node id; filled in by `threads`.
    pub node_id: String,
    pub title: String,
    pub base: String,
    pub head_oid: String,
    /// e.g. https://github.com/o/r/pull/1; filled in by `info`.
    pub url: String,
    /// The base as of the PR's last push (not the base branch's tip now).
    pub base_oid: String,
    /// Its branch, and whose (a fork's owner, or the repo's).
    pub head_branch: String,
    pub head_owner: String,
    /// Who opened it, and whether its branch is on a fork (shown as `owner:branch` then).
    pub author: String,
    pub cross_repo: bool,
    /// The repo its branch is in, `owner/name` (a fork's, or this one).
    pub head_repo: String,
    /// Its author lets maintainers push to its branch (on a fork: GitHub's checkbox).
    pub maintainer_can_modify: bool,
}

/// Review threads plus the viewer's pending review on the PR, if any.
pub struct Review {
    pub pr_id: String,
    /// Whether you opened the PR.
    pub mine: bool,
    /// Your login.
    pub me: String,
    pub pending: Option<String>,
    /// Its text so far (the review summary, submitted with it).
    pub pending_body: String,
    pub threads: Vec<Thread>,
    /// Your "Viewed" marks: path → VIEWED, or DISMISSED (the file changed after you marked it).
    pub viewed: std::collections::HashMap<String, String>,
}

#[derive(Clone, Default)]
pub struct Thread {
    /// GraphQL node id (for replies and resolving).
    pub id: String,
    pub path: String,
    pub left: bool,
    /// Current line in the PR diff; `None` when outdated.
    pub line: Option<u32>,
    pub start_line: Option<u32>,
    pub original_line: Option<u32>,
    pub resolved: bool,
    pub outdated: bool,
    /// Tail of the first comment's diff hunk (with +/-/space prefixes), ending at the commented
    /// line. Shown for outdated threads and used to relocate them.
    pub hunk: Vec<String>,
    pub comments: Vec<Comment>,
    /// A comment from a local review file (see `claude.rs`): its id there. Not on GitHub.
    pub claude: Option<String>,
    /// From the review file: how much it matters ("bug", "nit", "question", …); empty if unsaid.
    pub severity: String,
    /// From the review file: the GitHub review comment it's about (its id), if it answers one.
    pub about: Option<u64>,
    /// From the review file: the PR conversation comment (or review) it asks Claude about, by
    /// its URL. Such a thread is on no file's lines (`path` empty): it's under that comment.
    pub conversation: String,
    /// From the review file: what you last asked Claude for on it ("ask", "suggest",
    /// "change"); empty if unsaid.
    pub want: String,
}

#[derive(Clone, Default)]
pub struct Comment {
    pub id: u64,
    /// GraphQL node id (for editing).
    pub node_id: String,
    pub can_edit: bool,
    pub can_delete: bool,
    pub author: String,
    pub body: String,
    /// ISO 8601 timestamp, e.g. 2026-10-01T12:00:00Z.
    pub created: String,
    /// Part of the viewer's unsubmitted review.
    pub pending: bool,
    /// Its reactions: (GitHub's name for one, e.g. THUMBS_UP; how many; one is yours).
    pub reactions: Vec<Reaction>,
    /// Hidden on GitHub, and why ("off topic", "outdated", …); empty if it isn't.
    pub hidden: String,
}


/// Where a new comment goes. Lines are file line numbers on the given side.
#[derive(Clone)]
pub struct Anchor {
    pub path: String,
    pub line: u32,
    pub left: bool,
    pub start: Option<(u32, bool)>,
}

fn gh(args: &[&str]) -> Result<String> {
    gh_input(args, None)
}

/// PR `n`'s description as it is on GitHub now.
pub fn body(n: u64) -> Result<String> {
    let v: Value = serde_json::from_str(&gh(&["pr", "view", &n.to_string(), "--json", "body"])?)?;
    Ok(v["body"].as_str().unwrap_or("").replace("\r\n", "\n"))
}

/// PR `n`'s CI checks on GitHub now: (passed, failed, pending).
pub fn checks(n: u64) -> Result<(usize, usize, usize)> {
    let v: Value = serde_json::from_str(&gh(&["pr", "view", &n.to_string(), "--json", "statusCheckRollup"])?)?;
    let checks = v["statusCheckRollup"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    let count = |m| checks.iter().filter(|c| verdict(c) == m).count();
    Ok((count('✓'), count('✗'), count('●')))
}

/// Mark draft PR `n` ready for review (`gh pr ready`): only once CI passes, nothing failing
/// or still running (a PR with no checks has nothing to wait for).
pub fn mark_ready(n: u64) -> Result<()> {
    let (_, fail, pending) = checks(n)?;
    match (fail, pending) {
        (0, 0) => gh(&["pr", "ready", &n.to_string()]).map(drop),
        (0, k) => bail!("CI is still running ({k} check{} pending): ready once it passes", if k == 1 { "" } else { "s" }),
        (k, _) => bail!("CI fails ({k} check{} failed): fix it first (dv list: a a has Claude look)", if k == 1 { "" } else { "s" }),
    }
}

/// Replace PR `n`'s description with `body` (`gh pr edit`, the text on stdin).
pub fn set_body(n: u64, body: &str) -> Result<()> {
    gh_input(&["pr", "edit", &n.to_string(), "--body-file", "-"], Some(body)).map(drop)
}

/// Run gh, optionally feeding `input` on stdin (for `--input -` JSON bodies).
fn gh_input(args: &[&str], input: Option<&str>) -> Result<String> {
    let mut child = Command::new("gh")
        .args(args)
        .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("running gh")?;
    if let (Some(i), Some(mut stdin)) = (input, child.stdin.take()) {
        stdin.write_all(i.as_bytes())?;
    }
    let out = child.wait_with_output()?;
    if !out.status.success() {
        // A REST error's reasons are in the body; stderr only has its title ("Validation Failed").
        let body: Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
        let err = String::from_utf8_lossy(&out.stderr);
        let why = body["errors"].as_array().into_iter().flatten().filter_map(|e| e["message"].as_str());
        let why: Vec<&str> = why.filter(|w| !err.contains(w)).collect(); // GraphQL's are on stderr already
        match why.is_empty() {
            true => bail!("gh {}: {}", args.first().unwrap_or(&""), err.trim()),
            false => bail!("gh {}: {}: {}", args.first().unwrap_or(&""), err.trim(), why.join("; ")),
        }
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// `force` resets an existing local branch for the PR to the PR head, dropping its local commits.
/// `detach` checks out the PR head without a local branch (for worktrees: no branch clashes).
pub fn checkout(n: u64, force: bool, detach: bool) -> Result<()> {
    let n = n.to_string();
    let mut args = vec!["pr", "checkout", &n];
    if force {
        args.push("--force");
    }
    if detach {
        args.push("--detach");
    }
    gh(&args).map(drop)
}

/// The PR `n`, or the PR for the current branch.
pub fn pr(n: Option<u64>) -> Result<Pr> {
    let n = n.map(|n| n.to_string());
    let mut args = vec!["pr", "view", "--json", "number,title,baseRefName,headRefOid,baseRefOid,headRefName,headRepositoryOwner,author,isCrossRepository,headRepository,maintainerCanModify"];
    args.extend(n.as_deref());
    let v: Value = serde_json::from_str(&gh(&args)?)?;
    Ok(Pr {
        number: v["number"].as_u64().context("pr number")?,
        node_id: String::new(),
        title: v["title"].as_str().unwrap_or("").to_owned(),
        base: v["baseRefName"].as_str().unwrap_or("main").to_owned(),
        head_oid: v["headRefOid"].as_str().unwrap_or("").to_owned(),
        url: String::new(),
        base_oid: v["baseRefOid"].as_str().unwrap_or("").to_owned(),
        head_branch: v["headRefName"].as_str().unwrap_or("").to_owned(),
        head_owner: v["headRepositoryOwner"]["login"].as_str().unwrap_or("").to_owned(),
        author: v["author"]["login"].as_str().unwrap_or("").to_owned(),
        cross_repo: v["isCrossRepository"].as_bool().unwrap_or(false),
        head_repo: v["headRepository"]["nameWithOwner"].as_str().unwrap_or("").to_owned(),
        maintainer_can_modify: v["maintainerCanModify"] == true,
    })
}

/// (owner, repo, number) of a github.com issue or PR link, e.g. .../o/r/issues/12#issuecomment-1.
pub fn issue_ref(url: &str) -> Option<(&str, &str, u64)> {
    let rest = url.strip_prefix("https://github.com/")?;
    let mut parts = rest.split(['/', '#', '?']);
    let (owner, repo, kind, n) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
    matches!(kind, "issues" | "pull").then_some(())?;
    Some((owner, repo, n.parse().ok()?))
}

/// An issue or PR as (popup title, markdown): its state, author, age, labels, then its body.
pub fn issue_preview(url: &str) -> Result<(String, String)> {
    let (owner, repo, n) = issue_ref(url).context("not a GitHub issue or PR link")?;
    let v: Value = serde_json::from_str(&gh(&["api", &format!("repos/{owner}/{repo}/issues/{n}")])?)?;
    let s = |v: &Value| v.as_str().unwrap_or("").to_owned();
    let pr = v["pull_request"].is_object();
    let state = match (s(&v["state"]).as_str(), pr) {
        (_, true) if v["pull_request"]["merged_at"].is_string() => "Merged",
        ("open", true) if v["draft"] == true => "Draft",
        ("open", _) => "Open",
        _ if s(&v["state_reason"]) == "not_planned" => "Closed (not planned)",
        _ => "Closed",
    };
    let kind = if pr { "Pull request" } else { "Issue" };
    let comments = match v["comments"].as_u64().unwrap_or(0) {
        1 => " · 1 comment".into(),
        c => format!(" · {c} comments"),
    };
    let labels: Vec<String> = v["labels"].as_array().into_iter().flatten().map(|l| format!("`{}`", s(&l["name"]))).collect();
    let body = s(&v["body"]).replace("\r\n", "\n");
    let body = if body.trim().is_empty() { "_(no description)_".into() } else { body };
    let md = format!(
        "**{kind} · {state}** · @{} · opened {}{comments}\n{}\n\n{body}",
        s(&v["user"]["login"]),
        ago(&s(&v["created_at"])),
        labels.join(" "),
    );
    let title = format!(" {owner}/{repo}#{n} · {}  j/k scroll ", s(&v["title"]));
    Ok((title, md))
}

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
    let v: Value = serde_json::from_str(&gh(&["pr", "list", "--limit", &LIST_MAX.to_string(), "--json", LISTED])?)?;
    Ok(v.as_array().context("gh pr list")?.iter().map(listed).collect())
}

/// The PR's head branch and the login of the repo it's in, e.g. (`add_persist_test_utils`,
/// `110CodingP`).
pub fn head_branch(n: u64) -> Result<(String, String)> {
    let v: Value = serde_json::from_str(&gh(&["pr", "view", &n.to_string(), "--json", "headRefName,headRepositoryOwner"])?)?;
    Ok((v["headRefName"].as_str().unwrap_or("").to_owned(), v["headRepositoryOwner"]["login"].as_str().unwrap_or("").to_owned()))
}

/// One PR, open or not (for a worktree whose PR has since closed or merged).
/// PR `n`'s title, author, URL and base branch.
pub fn brief(n: u64) -> Result<(String, String, String, String)> {
    let v: Value = serde_json::from_str(&gh(&["pr", "view", &n.to_string(), "--json", "title,author,url,baseRefName"])?)?;
    let s = |v: &Value| v.as_str().unwrap_or("").to_owned();
    Ok((s(&v["title"]), s(&v["author"]["login"]), s(&v["url"]), s(&v["baseRefName"])))
}

pub fn listed_one(n: u64) -> Result<Listed> {
    let v: Value = serde_json::from_str(&gh(&["pr", "view", &n.to_string(), "--json", LISTED])?)?;
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
        let v: Value = serde_json::from_str(&gh(&["api", "graphql", "-F", "owner={owner}", "-F", "name={repo}", "-f", &format!("query={q}")])?)?;
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

/// PR `n`'s state on GitHub now: OPEN, CLOSED or MERGED (`None` if gh doesn't say).
pub fn pr_state(n: u64) -> Result<Option<String>> {
    let v: Value = serde_json::from_str(&gh(&["pr", "view", &n.to_string(), "--json", "state"])?)?;
    Ok(v["state"].as_str().map(str::to_owned))
}

/// This repo on GitHub, as `owner/name`.
pub fn repo_name() -> Result<String> {
    Ok(gh(&["repo", "view", "--json", "nameWithOwner", "-q", ".nameWithOwner"])?.trim().to_owned())
}

pub type ListChecks = std::collections::HashMap<u64, ((usize, usize, usize), String)>;

/// CI check counts for the PRs `list` returns: number → ((passed, failed, pending), the head
/// they're on).
pub fn list_checks() -> Result<ListChecks> {
    let v: Value = serde_json::from_str(&gh(&["pr", "list", "--limit", &LIST_MAX.to_string(), "--json", "number,headRefOid,statusCheckRollup"])?)?;
    let prs = v.as_array().context("gh pr list")?;
    Ok(prs
        .iter()
        .map(|p| {
            let checks = p["statusCheckRollup"].as_array().map(Vec::as_slice).unwrap_or(&[]);
            let count = |m| checks.iter().filter(|c| verdict(c) == m).count();
            let head = p["headRefOid"].as_str().unwrap_or("").to_owned();
            (p["number"].as_u64().unwrap_or(0), ((count('✓'), count('✗'), count('●')), head))
        })
        .collect())
}

/// A CI check's outcome: ✓ passed, ✗ failed, ● pending. CheckRun has status/conclusion;
/// StatusContext has state.
fn verdict(c: &Value) -> char {
    match (c["status"].as_str(), c["conclusion"].as_str().or(c["state"].as_str())) {
        (Some(st), _) if st != "COMPLETED" => '●',
        (_, Some("SUCCESS" | "NEUTRAL" | "SKIPPED")) => '✓',
        (_, Some("PENDING" | "EXPECTED")) | (_, None) => '●',
        _ => '✗',
    }
}

const COMMENT: &str = "id databaseId author { login } body createdAt diffHunk state viewerCanUpdate viewerCanDelete \
reactionGroups { content viewerHasReacted reactors { totalCount } } isMinimized minimizedReason";
const THREAD: &str = "id path line startLine originalLine diffSide isResolved isOutdated";
const PAGE: &str = "pageInfo { hasNextPage endCursor }";

/// The PR's review threads, viewed files and your pending review: the first 100 of each
/// (threads, their comments, files), then the rest a page at a time.
/// GitHub's answer, as JSON (dv caches it; `parse_threads` reads it).
pub fn threads_json(n: u64) -> Result<Value> {
    let num = format!("number={n}");
    let run = |q: &str, vars: &[(&str, &str)]| -> Result<Value> {
        let q = format!("query={q}");
        let vars: Vec<String> = vars.iter().map(|(k, v)| format!("{k}={v}")).collect();
        let mut args = vec!["api", "graphql", "-F", "owner={owner}", "-F", "name={repo}", "-F", &num, "-f", &q];
        for v in &vars {
            args.extend(["-f", v]);
        }
        let v: Value = serde_json::from_str(&gh(&args)?)?;
        if let Some(e) = v["errors"].get(0) {
            bail!("GitHub: {}", e["message"].as_str().unwrap_or("GraphQL error"));
        }
        Ok(v["data"].clone())
    };
    fetch_review(run)
}

pub fn parse_threads(d: &Value) -> Review {
    let mut r = parse_review(&d["repository"]["pullRequest"]);
    r.me = d["viewer"]["login"].as_str().unwrap_or("").to_owned();
    r
}

/// The cursor of a list's next page, if it has one.
fn next_page(list: &Value) -> Option<String> {
    let p = &list["pageInfo"];
    (p["hasNextPage"] == true).then(|| p["endCursor"].as_str().map(str::to_owned)).flatten()
}

/// The PR's ETag, if it has changed from `etag` (empty: the first look); `None` if GitHub says
/// 304 Not Modified, which doesn't count against the rate limit. New comments, replies, reviews
/// and pushes change it; edits and resolving a thread don't.
pub fn pr_etag(n: u64, etag: &str) -> Result<Option<String>> {
    etag_of(&format!("repos/{{owner}}/{{repo}}/pulls/{n}"), etag)
}

/// The open PRs' ETag (as `list` lists them: most recently updated first), if it has changed
/// from `etag`, as `pr_etag`: a push, comment, review, new or closed PR changes it; CI doesn't.
pub fn list_etag(etag: &str) -> Result<Option<String>> {
    etag_of("repos/{owner}/{repo}/pulls?state=open&sort=updated&direction=desc&per_page=100", etag)
}

/// API `path`'s ETag, if it has changed from `etag` (`None`: 304 Not Modified, free).
fn etag_of(path: &str, etag: &str) -> Result<Option<String>> {
    let header = format!("If-None-Match: {etag}");
    let mut args = vec!["api", "-i", path];
    if !etag.is_empty() {
        args.extend(["-H", &header]);
    }
    let out = Command::new("gh").args(&args).stdin(Stdio::null()).output().context("running gh")?;
    let text = String::from_utf8_lossy(&out.stdout);
    // gh exits 1 on a 304, so the status line decides.
    if text.lines().next().is_some_and(|l| l.contains(" 304")) {
        return Ok(None);
    }
    if !out.status.success() {
        bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    let tag = text.lines().take_while(|l| !l.trim().is_empty()).find_map(|l| {
        let (k, v) = l.split_once(':')?;
        k.eq_ignore_ascii_case("etag").then(|| v.trim().to_owned())
    });
    Ok(Some(tag.unwrap_or_default()))
}

/// The paging behind `threads`, with the GraphQL call passed in (so tests can fake it):
/// the first page, with later pages' nodes appended to its lists.
fn fetch_review(mut run: impl FnMut(&str, &[(&str, &str)]) -> Result<Value>) -> Result<Value> {
    let pr = "repository(owner: $owner, name: $name) { pullRequest(number: $number)";
    let vars = "$owner: String!, $name: String!, $number: Int!";
    let threads = |after: &str| format!("reviewThreads(first: 100{after}) {{ {PAGE} nodes {{ {THREAD} comments(first: 100) {{ {PAGE} nodes {{ {COMMENT} }} }} }} }}");
    let files = |after: &str| format!("files(first: 100{after}) {{ {PAGE} nodes {{ path viewerViewedState }} }}");
    let mut d = run(
        &format!(
            "query({vars}) {{ viewer {{ login }} {pr} {{ id viewerDidAuthor
               reviews(states: PENDING, first: 10) {{ nodes {{ id viewerDidAuthor body }} }} {} {} }} }} }}",
            files(""),
            threads("")
        ),
        &[],
    )?;
    let at = |d: &Value, k: &str| d["repository"]["pullRequest"][k].clone();
    // Appends a page's nodes to `list`; the cursor of the next page, if there is one.
    let append = |list: &mut Value, page: &Value| -> Option<String> {
        if let (Some(l), Some(p)) = (list["nodes"].as_array_mut(), page["nodes"].as_array()) {
            l.extend(p.iter().cloned());
        }
        next_page(page)
    };
    let p = &mut d["repository"]["pullRequest"];
    for (key, list) in [("reviewThreads", threads(", after: $after")), ("files", files(", after: $after"))] {
        let mut next = next_page(&p[key]);
        while let Some(after) = next {
            let page = run(&format!("query({vars}, $after: String!) {{ {pr} {{ {list} }} }} }}"), &[("after", &after)])?;
            next = append(&mut p[key], &at(&page, key));
        }
    }
    // A thread with more than 100 comments: the rest of them.
    if let Some(ts) = p["reviewThreads"]["nodes"].as_array_mut() {
        for t in ts {
            let mut next = next_page(&t["comments"]);
            let id = t["id"].as_str().unwrap_or("").to_owned();
            while let Some(after) = next {
                let q = format!(
                    "query($id: ID!, $after: String!) {{ node(id: $id) {{ ... on PullRequestReviewThread {{
                       comments(first: 100, after: $after) {{ {PAGE} nodes {{ {COMMENT} }} }} }} }} }}"
                );
                let page = run(&q, &[("id", &id), ("after", &after)])?;
                next = append(&mut t["comments"], &page["node"]["comments"]);
            }
        }
    }
    Ok(d)
}

fn parse_review(pr: &Value) -> Review {
    let nodes = |v: &Value| v["nodes"].as_array().cloned().unwrap_or_default();
    let pending = nodes(&pr["reviews"]).into_iter().find(|r| r["viewerDidAuthor"] == true);
    Review {
        pr_id: pr["id"].as_str().unwrap_or("").to_owned(),
        mine: pr["viewerDidAuthor"] != false,
        me: String::new(),
        pending: pending.as_ref().and_then(|r| r["id"].as_str().map(str::to_owned)),
        pending_body: pending.as_ref().and_then(|r| r["body"].as_str()).unwrap_or("").to_owned(),
        threads: nodes(&pr["reviewThreads"]).iter().map(parse_thread).collect(),
        viewed: nodes(&pr["files"])
            .iter()
            .filter(|f| f["viewerViewedState"] != "UNVIEWED")
            .filter_map(|f| Some((f["path"].as_str()?.to_owned(), f["viewerViewedState"].as_str()?.to_owned())))
            .collect(),
    }
}

/// GitHub's reactions, by its name for each, and how they show.
pub const REACTIONS: [(&str, &str); 8] = [
    ("THUMBS_UP", "👍"),
    ("THUMBS_DOWN", "👎"),
    ("LAUGH", "😄"),
    ("HOORAY", "🎉"),
    ("CONFUSED", "😕"),
    // Not ❤️: its variation selector makes it 2 columns in some terminals, 1 in width counts.
    ("HEART", "💖"),
    ("ROCKET", "🚀"),
    ("EYES", "👀"),
];

/// Why a comment is hidden on GitHub (its `isMinimized`, `minimizedReason`: "off-topic",
/// "OUTDATED", …), in words; empty if it isn't.
pub fn hidden(c: &Value) -> String {
    match c["isMinimized"] == true {
        true => Some(c["minimizedReason"].as_str().unwrap_or("").to_lowercase().replace(['-', '_'], " ")).filter(|r| !r.is_empty()).unwrap_or_else(|| "hidden".into()),
        false => String::new(),
    }
}

/// What a comment hidden on GitHub (`why`, from `hidden`) says it is: "hidden on GitHub
/// (spam)", or (GitHub gave no reason) just "hidden on GitHub".
pub fn hidden_words(why: &str) -> String {
    match why {
        "hidden" => "hidden on GitHub".into(),
        why => format!("hidden on GitHub ({why})"),
    }
}

/// A reaction on a comment: GitHub's name for it (e.g. THUMBS_UP), how many, one is yours.
pub type Reaction = (String, u64, bool);

/// A comment's `reactionGroups` (from GraphQL: `reactors`, `viewerHasReacted`; from `gh pr
/// view`: `users`, and whose isn't said: `mine` has yours, by node id), those with any.
fn reactions(v: &Value, mine: &[&str]) -> Vec<Reaction> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|g| {
            let n = g["reactors"]["totalCount"].as_u64().or_else(|| g["users"]["totalCount"].as_u64())?;
            let what = g["content"].as_str().unwrap_or("");
            (n > 0).then(|| (what.to_owned(), n, g["viewerHasReacted"] == true || mine.contains(&what)))
        })
        .collect()
}

/// Reactions as they show (`👍 2`, `🎉 1`), each with whether one's yours.
pub fn reaction_parts(r: &[Reaction]) -> Vec<(String, bool)> {
    let emoji = |c: &str| REACTIONS.iter().find(|e| e.0 == c).map_or("?", |e| e.1);
    r.iter().map(|(c, n, mine)| (format!("{} {n}", emoji(c)), *mine)).collect()
}

/// Reactions as text: `👍 2  🎉 1`.
#[cfg(test)]
fn reactions_text(r: &[Reaction]) -> String {
    reaction_parts(r).into_iter().map(|p| p.0).collect::<Vec<_>>().join("  ")
}

impl InfoLine {
    /// Reactions, yours in the accent color (as GitHub lights them); all dimmed when which
    /// are yours isn't `known`.
    fn put_reactions(&mut self, r: &[Reaction], known: bool) {
        for (i, (text, mine)) in reaction_parts(r).into_iter().enumerate() {
            let mark = match known {
                true => mine.then_some(Mark::Mine),
                false => Some(Mark::Dim),
            };
            self.put(if i == 0 { "" } else { "  " }, None).put(&text, mark);
        }
    }
}

/// The `pr_query` fields saying which reactions on a PR's conversation (itself, its
/// comments, its reviews) are yours: `gh pr view` doesn't say.
const MINE: &str = "id reactionGroups { content viewerHasReacted } comments(last: 100) { pageInfo { hasPreviousPage startCursor } nodes { id reactionGroups { content viewerHasReacted } } } reviews(last: 100) { pageInfo { hasPreviousPage startCursor } nodes { id reactionGroups { content viewerHasReacted } } }";

/// Yours, from a pull request's node with `MINE` (or a page of its comments or reviews), into
/// `mine`: every node looked at, by id, with the reactions on it that are yours (none: empty).
/// A node not in it: which are yours isn't known.
fn mine_of(p: &Value, mine: &mut serde_json::Map<String, Value>) {
    let nodes = std::iter::once(p).chain(["comments", "reviews"].iter().flat_map(|k| p[k]["nodes"].as_array().into_iter().flatten()));
    for node in nodes {
        let Some(id) = node["id"].as_str() else { continue };
        let yours: Vec<Value> = node["reactionGroups"].as_array().into_iter().flatten().filter(|g| g["viewerHasReacted"] == true).map(|g| g["content"].clone()).collect();
        mine.insert(id.to_owned(), yours.into());
    }
}

/// Which reactions on PR node `p` (with `MINE`) and its whole conversation are yours (as
/// `mine_of`): its comments and reviews before the last 100 asked for too, 100 at a time (a
/// PR that has so many), the two at once.
fn mine_all(p: &Value) -> Value {
    let mut mine = serde_json::Map::new();
    mine_of(p, &mut mine);
    let older = |kind: &str| {
        let mut out = serde_json::Map::new();
        let mut page = p[kind]["pageInfo"].clone();
        // ponytail: 20 pages (2,000 more) at most; beyond, not known (dimmed).
        for _ in 0..20 {
            let Some(before) = page["startCursor"].as_str().filter(|_| page["hasPreviousPage"] == true) else { break };
            let q = format!(
                "query($id: ID!, $b: String) {{ node(id: $id) {{ ... on PullRequest {{ {kind}(last: 100, before: $b) {{ pageInfo {{ hasPreviousPage startCursor }} nodes {{ id reactionGroups {{ content viewerHasReacted }} }} }} }} }} }}"
            );
            let Ok(d) = mutate(&q, json!({ "id": p["id"], "b": before })) else { break };
            mine_of(&json!({ kind: d["node"][kind] }), &mut out);
            page = d["node"][kind]["pageInfo"].clone();
        }
        out
    };
    std::thread::scope(|s| {
        let pages: Vec<_> = ["comments", "reviews"].into_iter().map(|k| s.spawn(move || older(k))).collect();
        for p in pages {
            mine.extend(p.join().unwrap_or_default());
        }
    });
    mine.into()
}

/// The reactions on comment (or review) `node` now: each kind's (name, count, yours).
pub fn reactions_of(node: &str) -> Result<Vec<(String, u64, bool)>> {
    let q = "query($id: ID!) { node(id: $id) { ... on Reactable { reactionGroups { content viewerHasReacted reactors { totalCount } } } } }";
    let d = mutate(q, json!({ "id": node }))?;
    Ok(d["node"]["reactionGroups"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|g| (g["content"].as_str().unwrap_or("").to_owned(), g["reactors"]["totalCount"].as_u64().unwrap_or(0), g["viewerHasReacted"] == true))
        .collect())
}

/// React to comment (or review) `node` with `content` (`add`), or take yours back.
pub fn react(node: &str, content: &str, add: bool) -> Result<()> {
    let op = if add { "addReaction" } else { "removeReaction" };
    let q = format!("mutation($id: ID!, $c: ReactionContent!) {{ {op}(input: {{ subjectId: $id, content: $c }}) {{ clientMutationId }} }}");
    mutate(&q, json!({ "id": node, "c": content })).map(drop)
}

/// Run a GraphQL mutation; variables go in a JSON body so strings need no escaping.
fn mutate(query: &str, vars: Value) -> Result<Value> {
    let body = json!({ "query": query, "variables": vars }).to_string();
    let v: Value = serde_json::from_str(&gh_input(&["api", "graphql", "--input", "-"], Some(&body))?)?;
    if let Some(e) = v["errors"].get(0) {
        bail!("GitHub: {}", e["message"].as_str().unwrap_or("GraphQL error"));
    }
    Ok(v["data"].clone())
}

fn side(left: bool) -> &'static str {
    if left { "LEFT" } else { "RIGHT" }
}

/// Start a pending review on `commit`, with `body` as its text if there is one; returns its id.
/// (GitHub can't give a review started without text any until it's submitted.)
pub fn start_review(pr_id: &str, commit: &str, body: &str) -> Result<String> {
    let d = mutate(
        "mutation($pr: ID!, $commit: GitObjectID!, $body: String) {
          addPullRequestReview(input: { pullRequestId: $pr, commitOID: $commit, body: $body }) { pullRequestReview { id } } }",
        json!({ "pr": pr_id, "commit": commit, "body": (!body.is_empty()).then_some(body) }),
    )?;
    d["addPullRequestReview"]["pullRequestReview"]["id"].as_str().map(str::to_owned).context("no review id")
}

/// A review with no comments, created and submitted at once (so a refusal leaves nothing behind).
pub fn review_now(pr_id: &str, commit: &str, event: &str, body: &str) -> Result<()> {
    mutate(
        "mutation($pr: ID!, $commit: GitObjectID!, $event: PullRequestReviewEvent!, $body: String) {
          addPullRequestReview(input: { pullRequestId: $pr, commitOID: $commit, event: $event, body: $body }) {
            pullRequestReview { id } } }",
        json!({ "pr": pr_id, "commit": commit, "event": event, "body": body }),
    )
    .map(drop)
}

pub fn add_thread(review: &str, a: &Anchor, body: &str) -> Result<()> {
    let (start_line, start_side) = a.start.map_or((None, None), |(l, left)| (Some(l), Some(side(left))));
    mutate(
        "mutation($review: ID!, $path: String!, $body: String!, $line: Int!, $side: DiffSide!,
                  $startLine: Int, $startSide: DiffSide) {
          addPullRequestReviewThread(input: { pullRequestReviewId: $review, path: $path, body: $body,
            line: $line, side: $side, startLine: $startLine, startSide: $startSide }) { thread { id } } }",
        json!({ "review": review, "path": a.path, "body": body, "line": a.line, "side": side(a.left),
                "startLine": start_line, "startSide": start_side }),
    )
    .map(drop)
}

pub fn add_reply(review: &str, thread: &str, body: &str) -> Result<()> {
    mutate(
        "mutation($review: ID!, $thread: ID!, $body: String!) {
          addPullRequestReviewThreadReply(input: { pullRequestReviewId: $review,
            pullRequestReviewThreadId: $thread, body: $body }) { comment { id } } }",
        json!({ "review": review, "thread": thread, "body": body }),
    )
    .map(drop)
}

/// `event`: COMMENT, APPROVE or REQUEST_CHANGES.
pub fn submit_review(review: &str, event: &str, body: &str) -> Result<()> {
    mutate(
        "mutation($review: ID!, $event: PullRequestReviewEvent!, $body: String) {
          submitPullRequestReview(input: { pullRequestReviewId: $review, event: $event, body: $body }) {
            pullRequestReview { id } } }",
        json!({ "review": review, "event": event, "body": body }),
    )
    .map(drop)
}

/// Replace a pending review's text (its summary).
pub fn set_review_body(review: &str, body: &str) -> Result<()> {
    mutate(
        "mutation($review: ID!, $body: String!) {
          updatePullRequestReview(input: { pullRequestReviewId: $review, body: $body }) { pullRequestReview { id } } }",
        json!({ "review": review, "body": body }),
    )
    .map(drop)
}

pub fn delete_review(review: &str) -> Result<()> {
    mutate(
        "mutation($review: ID!) { deletePullRequestReview(input: { pullRequestReviewId: $review }) {
          pullRequestReview { id } } }",
        json!({ "review": review }),
    )
    .map(drop)
}

pub fn delete_comment(comment: &str) -> Result<()> {
    mutate(
        "mutation($c: ID!) { deletePullRequestReviewComment(input: { id: $c }) { clientMutationId } }",
        json!({ "c": comment }),
    )
    .map(drop)
}

pub fn edit_comment(comment: &str, body: &str) -> Result<()> {
    mutate(
        "mutation($c: ID!, $body: String!) {
          updatePullRequestReviewComment(input: { pullRequestReviewCommentId: $c, body: $body }) {
            pullRequestReviewComment { id } } }",
        json!({ "c": comment, "body": body }),
    )
    .map(drop)
}

/// Toggle the "Viewed" checkbox for a file, as on github.com's Files tab.
pub fn set_viewed(pr_id: &str, path: &str, viewed: bool) -> Result<()> {
    let q = match viewed {
        true => "mutation($pr: ID!, $path: String!) { markFileAsViewed(input: { pullRequestId: $pr, path: $path }) { clientMutationId } }",
        false => "mutation($pr: ID!, $path: String!) { unmarkFileAsViewed(input: { pullRequestId: $pr, path: $path }) { clientMutationId } }",
    };
    mutate(q, json!({ "pr": pr_id, "path": path })).map(drop)
}

/// Rerun the failed jobs of the workflow runs on commit `head` that failed, four at a time:
/// what happened, in a few words (an error if none could be).
pub fn rerun_failed(head: &str) -> Result<String> {
    let q = ".[] | select(.conclusion == \"failure\" or .conclusion == \"timed_out\" or .conclusion == \"cancelled\") | .databaseId";
    let out = gh(&["run", "list", "--commit", head, "--limit", "100", "--json", "databaseId,conclusion", "-q", q])?;
    let ids: Vec<&str> = out.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    if ids.is_empty() {
        return Ok("no failed runs to rerun (they may be running already)".into());
    }
    // A few at a time (not GitHub's limit on many writes at once); one that can't be (too
    // old, say) doesn't keep the others from it.
    let mut errors = vec![];
    for batch in ids.chunks(4) {
        std::thread::scope(|s| {
            let runs: Vec<_> = batch.iter().map(|id| s.spawn(move || gh(&["run", "rerun", id, "--failed"]).err().map(|e| format!("run {id}: {e:#}")))).collect();
            errors.extend(runs.into_iter().filter_map(|r| r.join().unwrap_or_else(|_| Some("a rerun went wrong".into()))));
        });
    }
    rerun_says(ids.len(), &errors)
}

/// What rerunning `total` runs came to, `errors` the ones that couldn't be.
fn rerun_says(total: usize, errors: &[String]) -> Result<String> {
    let rerun = total - errors.len();
    match (rerun, errors.first()) {
        (0, Some(e)) => anyhow::bail!("no failed run could be rerun: {e}"),
        (_, Some(e)) => Ok(format!("{rerun} of {total} failed runs rerunning ({} couldn't be: {e})", errors.len())),
        _ => Ok(format!("{rerun} failed run(s) rerunning")),
    }
}

/// `s` for a URL path segment: all but unreserved characters percent-encoded.
fn percent(s: &str) -> String {
    s.bytes().map(|b| match b {
        b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (b as char).to_string(),
        b => format!("%{b:02X}"),
    }).collect()
}

/// What `Space l` sets on a PR.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Field {
    Label,
    Assignee,
    /// A review requested of someone.
    Reviewer,
}

/// PR `n`'s labels, assignees and requested reviewers, and what it could have: the repo's
/// labels, the people it can be assigned to (or asked to review: not its author). Each
/// (name, on it), sorted, those on it first.
pub fn pr_fields(n: u64) -> Result<(Choices, Choices, Choices)> {
    let v: Value = serde_json::from_str(&gh(&["pr", "view", &n.to_string(), "--json", "labels,assignees,reviewRequests,author"])?)?;
    let on = |k: &str, f: &str| -> Vec<String> { v[k].as_array().into_iter().flatten().filter_map(|x| x[f].as_str().map(str::to_owned)).collect() };
    let (labels, assigned) = (on("labels", "name"), on("assignees", "login"));
    // Requested of people (their login), or of teams (gh gives their slug as `org/team`).
    let asked: Vec<String> = v["reviewRequests"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| r["login"].as_str().or_else(|| r["slug"].as_str()).map(str::to_owned))
        .collect();
    let all: Value = serde_json::from_str(&gh(&["label", "list", "--limit", "500", "--json", "name"])?)?;
    let all: Vec<String> = all.as_array().into_iter().flatten().filter_map(|x| x["name"].as_str().map(str::to_owned)).collect();
    let people = gh(&["api", "--paginate", "repos/{owner}/{repo}/assignees", "-q", ".[].login"]).unwrap_or_default();
    let mark = |all: Vec<String>, on: &[String]| {
        let mut out: Vec<(String, bool)> = all.into_iter().chain(on.iter().cloned()).map(|x| (x.clone(), on.contains(&x))).collect();
        out.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.to_lowercase().cmp(&b.0.to_lowercase())));
        out.dedup();
        out
    };
    let people: Vec<String> = people.lines().map(str::to_owned).collect();
    let author = v["author"]["login"].as_str().unwrap_or("");
    let reviewers = people.iter().filter(|p| *p != author).cloned().collect();
    Ok((mark(all, &labels), mark(people, &assigned), mark(reviewers, &asked)))
}

/// Names, each with whether it's on the PR.
pub type Choices = Vec<(String, bool)>;

/// Add (`on`) or remove a label, an assignee or a requested reviewer on PR `n`.
pub fn set_field(n: u64, field: Field, name: &str, on: bool) -> Result<()> {
    // A label's name may have a comma in it, which `gh pr edit` splits on: the API takes it whole.
    if field == Field::Label {
        let ep = format!("repos/{{owner}}/{{repo}}/issues/{n}/labels");
        return match on {
            true => gh(&["api", "-X", "POST", &ep, "-f", &format!("labels[]={name}")]).map(drop),
            false => gh(&["api", "-X", "DELETE", &format!("{ep}/{}", percent(name))]).map(drop),
        };
    }
    let flag = match (field, on) {
        (Field::Label, _) => unreachable!("labels go through the API, above"),
        (Field::Assignee, true) => "--add-assignee",
        (Field::Assignee, false) => "--remove-assignee",
        (Field::Reviewer, true) => "--add-reviewer",
        (Field::Reviewer, false) => "--remove-reviewer",
    };
    gh(&["pr", "edit", &n.to_string(), flag, name]).map(drop)
}

pub fn set_resolved(thread: &str, resolved: bool) -> Result<()> {
    let q = match resolved {
        true => "mutation($t: ID!) { resolveReviewThread(input: { threadId: $t }) { thread { isResolved } } }",
        false => "mutation($t: ID!) { unresolveReviewThread(input: { threadId: $t }) { thread { isResolved } } }",
    };
    mutate(q, json!({ "t": thread })).map(drop)
}

impl Thread {
    /// Resolved or outdated: shown dimmed and folded until expanded.
    pub fn stale(&self) -> bool {
        self.resolved || self.outdated
    }
}

fn parse_thread(t: &Value) -> Thread {
    let line = |k: &str| t[k].as_u64().map(|n| n as u32);
    let comments = t["comments"]["nodes"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    Thread {
        id: t["id"].as_str().unwrap_or("").to_owned(),
        path: t["path"].as_str().unwrap_or("").to_owned(),
        left: t["diffSide"] == "LEFT",
        line: line("line"),
        start_line: line("startLine"),
        original_line: line("originalLine"),
        resolved: t["isResolved"].as_bool().unwrap_or(false),
        outdated: t["isOutdated"].as_bool().unwrap_or(false),
        hunk: comments
            .first()
            .and_then(|c| c["diffHunk"].as_str())
            .map(|h| {
                let lines: Vec<&str> = h.lines().filter(|l| !l.starts_with("@@")).collect();
                lines[lines.len().saturating_sub(4)..].iter().map(|l| l.to_string()).collect()
            })
            .unwrap_or_default(),
        comments: comments
            .iter()
            .map(|c| Comment {
                id: c["databaseId"].as_u64().unwrap_or(0),
                node_id: c["id"].as_str().unwrap_or("").to_owned(),
                can_edit: c["viewerCanUpdate"] == true,
                can_delete: c["viewerCanDelete"] == true,
                author: c["author"]["login"].as_str().unwrap_or("ghost").to_owned(),
                body: c["body"].as_str().unwrap_or("").replace("\r\n", "\n"),
                created: c["createdAt"].as_str().unwrap_or("").to_owned(),
                pending: c["state"] == "PENDING",
                reactions: reactions(&c["reactionGroups"], &[]),
                hidden: hidden(c),
            })
            .collect(),
        claude: None,
        severity: String::new(),
        about: None,
        conversation: String::new(),
        want: String::new(),
    }
}

/// POST `body` to a REST endpoint.
fn post(ep: &str, body: &Value) -> Result<()> {
    gh_input(&["api", "-X", "POST", ep, "--input", "-"], Some(&body.to_string())).map(drop)
}

/// A single comment, posted immediately (outside any review).
pub fn comment(pr: &Pr, a: &Anchor, body: &str) -> Result<()> {
    let ep = format!("repos/{{owner}}/{{repo}}/pulls/{}/comments", pr.number);
    let mut v = json!({ "body": body, "commit_id": pr.head_oid, "path": a.path, "line": a.line, "side": side(a.left) });
    if let Some((l, left)) = a.start {
        (v["start_line"], v["start_side"]) = (json!(l), json!(side(left)));
    }
    post(&ep, &v)
}

/// A comment on the PR conversation (not tied to a line or a review).
pub fn conversation_comment(pr: &Pr, body: &str) -> Result<()> {
    let ep = format!("repos/{{owner}}/{{repo}}/issues/{}/comments", pr.number);
    post(&ep, &json!({ "body": body }))
}

/// Open a draft PR from `head` into `base`: its number.
pub fn create_pr(base: &str, head: &str, title: &str, body: &str) -> Result<u64> {
    let args = ["pr", "create", "--draft", "--base", base, "--head", head, "--title", title, "--body-file", "-"];
    let out = gh_input(&args, Some(body))?;
    // It prints the new PR's URL: …/pull/N
    let url = out.lines().rev().find(|l| l.contains("/pull/")).context("gh pr create printed no PR URL")?;
    url.trim().rsplit('/').next().and_then(|n| n.parse().ok()).with_context(|| format!("no PR number in {url}"))
}

/// Close PR `n` with a comment saying why.
pub fn close_with(n: u64, comment: &str) -> Result<()> {
    gh(&["pr", "close", &n.to_string(), "--comment", comment]).map(drop)
}

/// Close the PR (it can be reopened on GitHub).
pub fn close(pr: &Pr) -> Result<()> {
    gh(&["pr", "close", &pr.number.to_string()]).map(drop)
}

/// A single reply, posted immediately.
pub fn reply(pr: &Pr, comment_id: u64, body: &str) -> Result<()> {
    let ep = format!("repos/{{owner}}/{{repo}}/pulls/{}/comments/{comment_id}/replies", pr.number);
    post(&ep, &json!({ "body": body }))
}

/// How to color part of a PR info line.
#[derive(Clone, Debug, PartialEq)]
pub enum Mark {
    /// A badge with this background (hex, no '#'), e.g. a label or the PR state.
    Chip(String),
    Pass,
    Fail,
    Pending,
    Dim,
    /// A reaction of yours.
    Mine,
    Link(String),
}

/// A line of the PR info section. `md` lines (description, comments) are rendered as markdown;
/// the rest are colored by `marks` (char ranges).
#[derive(Default)]
pub struct InfoLine {
    pub heading: bool,
    pub md: bool,
    pub text: String,
    pub marks: Vec<(usize, usize, Mark)>,
    /// On a comment's (or the description's) heading: its link on GitHub, for `Space c`.
    pub url: String,
    /// On a conversation comment's (or review's) heading: its node id, to react to it.
    pub node: String,
    /// On a conversation comment's heading: hidden on GitHub, and why; empty if it isn't.
    pub hidden: String,
    /// What the line is, the same across refreshes (its text may not be: counts, times): a
    /// comment's node id (its text: `<node>/text`), else what the line is about (`checks`,
    /// `conversation`, …). For the cursor to stay on it.
    pub key: String,
}

impl InfoLine {
    fn new(heading: bool, md: bool) -> Self {
        InfoLine { heading, md, ..Default::default() }
    }

    fn put(&mut self, s: &str, mark: Option<Mark>) -> &mut Self {
        let a = self.text.chars().count();
        self.text.push_str(s);
        if let Some(m) = mark {
            self.marks.push((a, a + s.chars().count(), m));
        }
        self
    }
}

/// Workflow runs waiting for a maintainer to approve them (a first-time contributor's PR, say):
/// head commit → their run ids. Only those for `sha`, if given.
pub fn awaiting_approval(sha: Option<&str>) -> Result<std::collections::HashMap<String, Vec<u64>>> {
    let mut ep = "repos/{owner}/{repo}/actions/runs?status=action_required&per_page=100".to_owned();
    if let Some(s) = sha {
        ep += &format!("&head_sha={s}");
    }
    let v: Value = serde_json::from_str(&gh(&["api", &ep])?)?;
    let mut out: std::collections::HashMap<String, Vec<u64>> = Default::default();
    for r in v["workflow_runs"].as_array().into_iter().flatten() {
        if let (Some(sha), Some(id)) = (r["head_sha"].as_str(), r["id"].as_u64()) {
            out.entry(sha.to_owned()).or_default().push(id);
        }
    }
    Ok(out)
}

/// Approve the workflow runs waiting on PR head `sha`, so they run: how many were.
pub fn approve_runs(sha: &str) -> Result<usize> {
    let ids = awaiting_approval(Some(sha))?.remove(sha).unwrap_or_default();
    for id in &ids {
        gh(&["api", "-X", "POST", &format!("repos/{{owner}}/{{repo}}/actions/runs/{id}/approve")])?;
    }
    Ok(ids.len())
}

/// The signatures on a PR's commits, as GitHub has them.
#[derive(Debug)]
pub struct Signatures {
    /// Its last 100 commits (oldest first): (oid, signed: a signature GitHub verifies).
    pub commits: Vec<(String, bool)>,
    /// How many commits it has in all (more than `commits` holds, the older aren't checked).
    pub total: usize,
    /// Whether its base requires signed commits (a ruleset, or classic branch protection):
    /// GitHub refuses a push of unsigned ones there.
    pub required: bool,
}

/// What GraphQL says, for a PR's node, of its base's rules on signatures.
const SIGNATURE_RULES: &str = "baseRef { branchProtectionRule { requiresCommitSignatures } rules(first: 100) { nodes { type } } }";

/// Whether a PR's base (its node, with `SIGNATURE_RULES`) requires signed commits.
fn requires_signatures(p: &Value) -> bool {
    let b = &p["baseRef"];
    b["branchProtectionRule"]["requiresCommitSignatures"] == true || b["rules"]["nodes"].as_array().is_some_and(|r| r.iter().any(|r| r["type"] == "REQUIRED_SIGNATURES"))
}

/// PR `n`'s commits' signatures, and whether its base requires them: one GraphQL query.
pub fn signatures(n: u64) -> Result<Signatures> {
    signatures_whole(&pr_query(n, "")?).map_err(anyhow::Error::msg)
}

/// The signatures in a `pr_query` answer, whole or not at all: a commit GitHub had trouble
/// with isn't one that isn't signed (else why not: GitHub's word). Its base's rules refused:
/// taken as not requiring them.
fn signatures_whole(a: &PrAnswer) -> std::result::Result<Signatures, String> {
    if let Some((_, why)) = a.refused.iter().find(|r| r.0 == "commits" || r.0 == "?") {
        return Err(why.clone());
    }
    signatures_of(&a.node).map_err(|e| format!("{e:#}"))
}

/// The first line of a message (GitHub's can run on), for a line of dv's.
fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").to_owned()
}

/// GitHub's kinds of refusal for a query's size or time: a smaller one may go through.
const TOO_BIG: [&str; 3] = ["MAX_NODE_LIMIT_EXCEEDED", "RESOURCE_LIMITS_EXCEEDED", "TIMEOUT"];

/// Whether GitHub refused a query for its size or time (asked again smaller), every error
/// saying so: not anything that would refuse it again (a rate limit, a permission, a field
/// that isn't there, an error that says no kind).
fn too_big(r: &Refused) -> bool {
    r.kinds.iter().all(|k| TOO_BIG.contains(&k.as_str())) // (never empty: see `pr_answer`)
}

/// What PR info adds from one `pr_query` with `MINE` (`first`): which reactions are yours (of
/// what GitHub answered: none of them asked again alone), and its commits' signatures (else
/// why not). Refused for its size, the signatures are asked on their own (`alone`).
fn extras(first: Result<PrAnswer>, alone: impl FnOnce() -> Result<PrAnswer>) -> (Value, std::result::Result<Signatures, String>) {
    let answer = match first {
        // (Too big, and then the smaller one failed too: both said, the second's reason the one
        // to act on.)
        Err(e) if e.downcast_ref::<Refused>().is_some_and(too_big) => alone().map_err(|e2| anyhow::anyhow!("{e:#}; and asked alone: {e2:#}")),
        r => r,
    };
    match answer {
        Ok(a) => (a.node.clone(), signatures_whole(&a).map_err(|w| first_line(&w))),
        Err(e) => (Value::Null, Err(first_line(&format!("{e:#}")))),
    }
}

/// What GitHub answered to a `pr_query`: the pull request's node, and the parts of it refused
/// (null there), with GitHub's word: its fields (`commits`, `comments`, …), or `?` for an
/// error that doesn't say.
#[derive(Debug)]
struct PrAnswer {
    node: Value,
    refused: Vec<(String, String)>,
}

/// GitHub refused a GraphQL query whole (its errors, no answer): each error's kind
/// (`RATE_LIMITED`, …), and the reason to give. Not the same as not being reached, nor as a bad
/// token.
#[derive(Debug)]
struct Refused {
    /// Each error's kind (empty when it gave none).
    kinds: Vec<String>,
    why: String,
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "GitHub: {}", self.why)
    }
}

impl std::error::Error for Refused {}

/// GitHub's answer (`gh api graphql`'s output, and what gh said, `stderr`): refused whole (no
/// node: `Refused`, with GitHub's reason) is an error.
fn pr_answer(v: &Value, stderr: &str) -> Result<PrAnswer> {
    let node = &v["data"]["repository"]["pullRequest"];
    let errors = v["errors"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    let why_of = |e: &Value| e["message"].as_str().unwrap_or("refused").to_owned();
    if !node.is_object() {
        if let Some(first) = errors.first() {
            let kinds: Vec<String> = errors.iter().map(|e| e["type"].as_str().unwrap_or("").to_owned()).collect();
            // The reason: a kind that isn't about size, with words, if any (that's why it isn't
            // asked again); else the first.
            let telling = |i: &usize| !kinds[*i].is_empty() && !TOO_BIG.contains(&kinds[*i].as_str()) && errors[*i]["message"].is_string();
            let why = (0..errors.len()).find(telling).map_or(first, |i| &errors[i]);
            return Err(Refused { why: why_of(why), kinds }.into());
        }
        // Not GraphQL's answer: the body's word (`Bad credentials`), else gh's.
        let why = v["message"].as_str().unwrap_or(stderr.trim());
        // GitHub out of time before it could answer (a gateway's 502 / 504, "We couldn't
        // respond to your request in time"): as its own TIMEOUT, so asked again, smaller.
        let late = ["HTTP 502", "HTTP 504", "in time"].iter().any(|w| why.contains(w) || stderr.contains(w));
        if late {
            return Err(Refused { kinds: vec!["TIMEOUT".into()], why: why.trim().to_owned() }.into());
        }
        anyhow::bail!("GitHub: {}", if why.is_empty() { "no answer" } else { why });
    }
    // repository.pullRequest.<field>: an error elsewhere, or with no path, is about who knows.
    let refused = errors
        .iter()
        .map(|e| (e["path"].get(2).and_then(Value::as_str).unwrap_or("?").to_owned(), why_of(e)))
        .collect();
    Ok(PrAnswer { node: node.clone(), refused })
}

/// PR `n`'s commits' signatures and its base's rules, and `more` of it: one GraphQL query. A
/// part GitHub refuses (some reactions, say) comes with the rest (gh exits non-zero then, so
/// its output is read either way).
fn pr_query(n: u64, more: &str) -> Result<PrAnswer> {
    let q = format!(
        "query($owner: String!, $name: String!, $n: Int!) {{ repository(owner: $owner, name: $name) {{ pullRequest(number: $n) {{
           {SIGNATURE_RULES} commits(last: 100) {{ totalCount nodes {{ commit {{ oid signature {{ isValid }} }} }} }} {more} }} }} }}"
    );
    let out = Command::new("gh")
        .args(["api", "graphql", "-F", "owner={owner}", "-F", "name={repo}", "-F", &format!("n={n}"), "-f", &format!("query={q}")])
        .stdin(Stdio::null())
        .output()
        .context("running gh")?;
    // (Not JSON at all: a gateway's page, or nothing; what gh said is all there is.)
    let v = serde_json::from_slice::<Value>(&out.stdout).unwrap_or_default();
    pr_answer(&v, &String::from_utf8_lossy(&out.stderr))
}

fn signatures_of(p: &Value) -> Result<Signatures> {
    let nodes = p["commits"]["nodes"].as_array().context("gh api graphql: commits")?;
    Ok(Signatures {
        commits: nodes.iter().map(|c| (c["commit"]["oid"].as_str().unwrap_or("").to_owned(), c["commit"]["signature"]["isValid"] == true)).collect(),
        total: p["commits"]["totalCount"].as_u64().unwrap_or(0) as usize,
        required: requires_signatures(p),
    })
}

/// Why PR `n` can't be merged as it is, if it can't: commits GitHub would refuse, unsigned
/// where its base requires signatures.
pub fn unsigned_blocking(n: u64) -> Result<Option<String>> {
    let s = signatures(n)?;
    let unsigned: Vec<String> = s.commits.into_iter().filter(|c| !c.1).map(|c| c.0.chars().take(8).collect()).collect();
    Ok((s.required && !unsigned.is_empty()).then(|| {
        format!("{} of #{n}'s commits aren't signed ({}), and its base requires signed commits: GitHub would refuse the push; ask the author to sign them", unsigned.len(), unsigned.join(", "))
    }))
}

const INFO_FIELDS: &str = "headRefOid,title,author,state,isDraft,url,baseRefName,headRefName,additions,deletions,\
changedFiles,labels,assignees,reviewDecision,reviewRequests,latestReviews,statusCheckRollup,body,comments,reviews,id,reactionGroups";

/// PR info lines, plus the current (title, head commit, url) so a refresh picks up new pushes.
/// GitHub's answer, as JSON (dv caches it; `parse_info` reads it).
/// GitHub couldn't say whose commits are signed this time (`new`), but did before (`old`, as
/// dv kept it) for the same head: that answer, as of then. A commit's signature is part of it,
/// so it still holds (only the base's rule may have changed since).
pub fn keep_signatures(new: &mut Value, old: &Value) {
    if new["dvCommitsUnknown"].is_null() || old["dvCommits"].as_array().is_none_or(|c| c.is_empty()) || new["headRefOid"] != old["headRefOid"] {
        return;
    }
    for k in ["dvCommits", "dvCommitsTotal", "dvSignaturesRequired", "dvCommitsAt"] {
        new[k] = old[k].clone();
    }
    new["dvCommitsKept"] = true.into();
    if let Some(o) = new.as_object_mut() {
        o.remove("dvCommitsUnknown");
    }
}

pub fn info_json(n: u64) -> Result<Value> {
    let mut v: Value = serde_json::from_str(&gh(&["pr", "view", &n.to_string(), "--json", INFO_FIELDS])?)?;
    // Whether its commits are signed, and whether its base wants them to be (`gh pr view` says
    // neither): with the rest, so the cache has them too.
    // Workflow runs on its head waiting for approval (`gh pr view` doesn't say).
    if let Some(sha) = v["headRefOid"].as_str().map(str::to_owned)
        && let Ok(w) = awaiting_approval(Some(&sha))
    {
        v["dvAwaiting"] = w.get(&sha).map_or(0, Vec::len).into();
    }
    // One query for both: its commits' signatures, and which reactions are yours (`extras`).
    let (node, signatures) = extras(pr_query(n, MINE), || pr_query(n, ""));
    v["dvMine"] = mine_all(&node);
    match signatures {
        Ok(s) => {
            v["dvCommits"] = s.commits.into_iter().map(|(oid, signed)| json!({ "oid": oid, "signed": signed })).collect();
            v["dvCommitsTotal"] = s.total.into();
            v["dvSignaturesRequired"] = s.required.into();
            v["dvCommitsAt"] = crate::claude::now().into();
        }
        // Unknown, and why: said among its facts.
        Err(why) => v["dvCommitsUnknown"] = why.into(),
    }
    Ok(v)
}

/// What PR info says, beyond its section's lines.
pub struct Info {
    pub lines: Vec<InfoLine>,
    pub title: String,
    pub head: String,
    pub url: String,
    pub state: State,
    /// Its commits GitHub has no valid signature on.
    /// (`None`: GitHub didn't say this time: what was known stands.)
    pub unsigned: Option<Vec<String>>,
    /// Workflow runs awaiting approval, checks failing.
    pub awaiting: usize,
    pub failing: usize,
    /// Its base branch.
    pub base: String,
}

/// The PR section's lines, its title, head, URL and state, its commits GitHub has no valid
/// signature on, (workflow runs awaiting approval, checks failing), and its base branch.
pub fn parse_info(v: &Value) -> Info {
    let s = |k: &str| v[k].as_str().unwrap_or("").to_owned();
    let unsigned = v["dvCommits"].as_array().map(|cs| cs.iter().filter(|c| c["signed"] != true).filter_map(|c| c["oid"].as_str().map(str::to_owned)).collect());
    let failing = v["statusCheckRollup"].as_array().into_iter().flatten().filter(|c| verdict(c) == '✗').count();
    let awaiting = v["dvAwaiting"].as_u64().unwrap_or(0) as usize;
    Info { lines: info_lines(v), title: s("title"), head: s("headRefOid"), url: s("url"), state: state(v), unsigned, awaiting, failing, base: s("baseRefName") }
}

/// A PR's state as GitHub shows it, with its color: DRAFT, OPEN, MERGED or CLOSED.
pub type State = (&'static str, &'static str);

pub fn state(v: &Value) -> State {
    // A closed draft is closed.
    match (v["state"].as_str().unwrap_or(""), v["isDraft"] == true) {
        ("MERGED", _) => ("MERGED", "8957e5"),
        ("CLOSED", _) => ("CLOSED", "da3633"),
        (_, true) => ("DRAFT", "6e7681"),
        _ => ("OPEN", "238636"),
    }
}

fn info_lines(v: &Value) -> Vec<InfoLine> {
    let s = |v: &Value| v.as_str().unwrap_or("").to_owned();
    let list = |v: &Value| v.as_array().map(Vec::as_slice).unwrap_or(&[]).to_vec();
    let mut out = vec![];
    let plain = |text: String| {
        let mut l = InfoLine::new(false, false);
        l.put(&text, None);
        l
    };

    // Author, branches, size (the state is on the PR's header line).
    let author = s(&v["author"]["login"]);
    let mut l = InfoLine::new(false, false);
    l.put(&format!("@{author}"), Some(Mark::Link(format!("https://github.com/{author}"))))
        .put(&format!(" · {} ← {} · ", s(&v["baseRefName"]), s(&v["headRefName"])), None)
        .put(&format!("+{}", v["additions"]), Some(Mark::Pass))
        .put(" ", None)
        .put(&format!("−{}", v["deletions"]), Some(Mark::Fail))
        .put(&format!(" in {} files", v["changedFiles"]), None);
    out.push(l);

    let labels = list(&v["labels"]);
    if !labels.is_empty() {
        let mut l = InfoLine::new(false, false);
        l.put("labels", Some(Mark::Dim));
        for lb in &labels {
            l.put(" ", None).put(&format!(" {} ", s(&lb["name"])), Some(Mark::Chip(s(&lb["color"]))));
        }
        out.push(l);
    }
    let assigned = list(&v["assignees"]);
    if !assigned.is_empty() {
        let mut l = InfoLine::new(false, false);
        l.put("assigned", Some(Mark::Dim));
        for a in &assigned {
            l.put(&format!(" @{}", s(&a["login"])), None);
        }
        out.push(l);
    }
    // The PR's own reactions (Space = on its description reacts).
    // Yours, by node id (`mine_json`).
    let mine = |id: &Value| -> Vec<&str> { id.as_str().and_then(|id| v["dvMine"][id].as_array()).into_iter().flatten().filter_map(Value::as_str).collect() };
    // Whether which of a node's reactions are yours is known (asked of GitHub): else they're
    // dimmed, not shown as not yours.
    let known = |id: &Value| id.as_str().is_some_and(|id| v["dvMine"].get(id).is_some());
    let own = reactions(&v["reactionGroups"], &mine(&v["id"]));
    if !own.is_empty() {
        let mut l = InfoLine::new(false, false);
        l.put("reactions ", Some(Mark::Dim)).put_reactions(&own, known(&v["id"]));
        out.push(l);
    }

    // Who did what, one reviewer a line (no scrolling sideways for them all): "✗ @bo requested
    // changes", "✓ @amy approved", "● @cy requested" (waiting on them); in their colors.
    let reviews = list(&v["latestReviews"]);
    let by = |state: &str| -> Vec<String> { reviews.iter().filter(|r| r["state"] == state).map(|r| s(&r["author"]["login"])).collect() };
    let requested: Vec<String> = list(&v["reviewRequests"]).iter().map(|r| r["login"].as_str().or(r["slug"].as_str()).or(r["name"].as_str()).unwrap_or("?").to_owned()).collect();
    let groups = [
        ("✗", "requested changes", by("CHANGES_REQUESTED"), Mark::Fail),
        ("✓", "approved", by("APPROVED"), Mark::Pass),
        ("●", "requested", requested, Mark::Pending),
    ];
    let mut l = InfoLine::new(false, false);
    l.put("reviews:", None);
    if groups.iter().all(|g| g.2.is_empty()) {
        let decision = s(&v["reviewDecision"]).to_lowercase().replace('_', " ");
        l.put(" ", None).put(if decision.is_empty() { "none yet" } else { &decision }, Some(Mark::Dim));
    }
    out.push(l);
    for (sign, what, who, mark) in groups {
        for name in who {
            let mut l = InfoLine::new(false, false);
            l.put(&format!("{sign} @{name}"), Some(mark.clone())).put(&format!(" {what}"), None);
            out.push(l);
        }
    }

    let checks = list(&v["statusCheckRollup"]);
    if checks.is_empty() {
        out.push(plain("checks: none".into()));
    } else {
        let count = |m| checks.iter().filter(|c| verdict(c) == m).count();
        let (pass, fail, pending) = (count('✓'), count('✗'), count('●'));
        let mut l = InfoLine::new(false, false);
        l.put("checks", Some(Mark::Link(format!("{}/checks", s(&v["url"])))));
        l.put(": ", None);
        // Only the counts that aren't zero.
        let parts = [(pass, "✓", "passed", Mark::Pass), (fail, "✗", "failed", Mark::Fail), (pending, "●", "pending", Mark::Pending)];
        for (k, (n, sym, what, m)) in parts.into_iter().filter(|p| p.0 > 0).enumerate() {
            if k > 0 {
                l.put(" · ", None);
            }
            l.put(&format!("{sym} {n} {what}"), Some(m));
        }
        out.push(l);
        for c in checks.iter().filter(|c| verdict(c) != '✓') {
            let name = c["name"].as_str().or(c["context"].as_str()).unwrap_or("?");
            let m = if verdict(c) == '✗' { Mark::Fail } else { Mark::Pending };
            let mut l = InfoLine::new(false, false);
            l.put(&format!("{} ", verdict(c)), Some(m.clone()));
            // Its run on GitHub (a check run's details, a status's target).
            match c["detailsUrl"].as_str().or(c["targetUrl"].as_str()).filter(|u| !u.is_empty()) {
                Some(u) => l.put(name, Some(Mark::Link(u.to_owned()))),
                None => l.put(name, Some(m)),
            };
            out.push(l);
        }
    }

    // Workflow runs that won't run until a maintainer approves them.
    match v["dvAwaiting"].as_u64().unwrap_or(0) {
        0 => {}
        k => {
            let mut l = InfoLine::new(false, false);
            let s = if k == 1 { "" } else { "s" };
            l.put(&format!("⏸ {k} workflow run{s} awaiting approval"), Some(Mark::Pending)).put(" (Space g approves)", Some(Mark::Dim));
            out.push(l);
        }
    }
    // Its commits signed, or which aren't (and whether its base requires them to be).
    if let Some(cs) = v["dvCommits"].as_array().filter(|c| !c.is_empty()) {
        let unsigned: Vec<String> = cs.iter().filter(|c| c["signed"] != true).map(|c| s(&c["oid"]).chars().take(8).collect()).collect();
        let required = v["dvSignaturesRequired"] == true;
        let mut l = InfoLine::new(false, false);
        l.put("commits: ", None);
        match unsigned.len() {
            0 => l.put(&format!("✓ {} signed", if cs.len() == 1 { "1".to_owned() } else { format!("all {}", cs.len()) }), Some(Mark::Pass)),
            k => l.put(&format!("✗ {k} of {} unsigned: {}", cs.len(), unsigned.join(", ")), Some(if required { Mark::Fail } else { Mark::Pending })),
        };
        // More than GitHub's last 100: the older ones aren't checked.
        let total = v["dvCommitsTotal"].as_u64().unwrap_or(0) as usize;
        if total > cs.len() {
            l.put(&format!(" (the last {} of {total})", cs.len()), Some(Mark::Dim));
        }
        if required {
            let st = if unsigned.is_empty() { Mark::Dim } else { Mark::Fail };
            l.put(" · ", None).put(&format!("{} requires signed commits", s(&v["baseRefName"])), Some(st));
        }
        // GitHub couldn't say this time: what it said last (`keep_signatures`).
        if v["dvCommitsKept"] == true {
            let when = Some(s(&v["dvCommitsAt"])).filter(|t| !t.is_empty()).map_or("an earlier look".to_owned(), |t| ago(&t));
            l.put(&format!(" (as of {when})"), Some(Mark::Dim));
        }
        out.push(l);
    } else if let Some(why) = v["dvCommitsUnknown"].as_str() {
        let mut l = InfoLine::new(false, false);
        l.put("commits: ", None).put(&format!("signatures unknown ({why})"), Some(Mark::Dim));
        out.push(l);
    }

    let heading = |t: String| {
        let mut l = InfoLine::new(true, false);
        l.put(&t, None);
        l
    };
    let markdown = |t: String| {
        let mut l = InfoLine::new(false, true);
        l.put(&t, None);
        l
    };
    // The PR itself takes reactions: its node id on the description's heading.
    out.push(InfoLine { url: s(&v["url"]), node: s(&v["id"]), ..heading("description".into()) });
    let body = s(&v["body"]).replace("\r\n", "\n");
    out.push(markdown(body)); // empty: the view shows a placeholder

    // The conversation: comments, and reviews with something to read (their text, or a
    // verdict: a plain review's inline comments are threads on the code), in time order:
    // (when, who, what they did, link, text).
    // (when, who, what they did, link, text, node id, reactions, why it's hidden).
    type Said<'a> = (String, String, &'a str, String, String, String, Vec<Reaction>, String);
    let mut said: Vec<Said> = list(&v["comments"])
        .iter()
        .map(|c| (s(&c["createdAt"]), s(&c["author"]["login"]), "", s(&c["url"]), s(&c["body"]), s(&c["id"]), reactions(&c["reactionGroups"], &mine(&c["id"])), hidden(c)))
        .collect();
    for r in list(&v["reviews"]) {
        let what = match s(&r["state"]).as_str() {
            "APPROVED" => "✓ approved · ",
            "CHANGES_REQUESTED" => "✗ requested changes · ",
            "DISMISSED" => "reviewed (dismissed) · ",
            "COMMENTED" if !s(&r["body"]).trim().is_empty() => "reviewed · ",
            _ => continue, // pending, or nothing to read here
        };
        // Its own: questions about it (`a`) are kept by it. (The fragment is the review's id:
        // the link opens the PR.)
        let url = format!("{}#review-{}", s(&v["url"]), s(&r["id"]));
        said.push((s(&r["submittedAt"]), s(&r["author"]["login"]), what, url, s(&r["body"]), s(&r["id"]), reactions(&r["reactionGroups"], &mine(&r["id"])), String::new()));
    }
    said.sort_by(|a, b| a.0.cmp(&b.0)); // ISO times sort as text
    out.push(heading(format!("conversation ({})", said.len())));
    for (at, who, what, url, body, node, r, hidden) in said {
        let sure = known(&Value::from(node.as_str()));
        // A verdict in its color, so an approval doesn't read as just another comment.
        let mut h = InfoLine { url, node, hidden, ..InfoLine::new(true, false) };
        let mark = match what {
            "✓ approved · " => Some(Mark::Pass),
            "✗ requested changes · " => Some(Mark::Fail),
            _ => None,
        };
        h.put(&format!("@{who} · "), None).put(what.trim_end_matches(" · "), mark).put(if what.is_empty() { "" } else { " · " }, None);
        h.put(&ago(&at), None);
        if !r.is_empty() {
            h.put(" · ", None).put_reactions(&r, sure);
        }
        out.push(h);
        out.push(markdown(body.replace("\r\n", "\n")));
    }
    for i in 0..out.len() {
        out[i].key = line_key(&out[..=i]);
    }
    out
}

/// What the last of `lines` is (`InfoLine::key`): its node; else the node of the heading it's
/// the text of; else its label, what it says up to its first `:`, `(` or digit (`checks`,
/// `conversation`, `reactions`).
fn line_key(lines: &[InfoLine]) -> String {
    let (l, before) = lines.split_last().unwrap_or_else(|| unreachable!());
    match before.last().filter(|h| h.heading && !l.heading && !h.node.is_empty()) {
        _ if !l.node.is_empty() => l.node.clone(),
        Some(h) => format!("{}/text", h.node),
        None => l.text.split([':', '(']).next().unwrap_or("").split(|c: char| c.is_ascii_digit()).next().unwrap_or("").trim().to_owned(),
    }
}

pub fn ago(iso: &str) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    ago_at(iso, now)
}

fn ago_at(iso: &str, now: i64) -> String {
    let Some(t) = epoch_secs(iso) else { return iso.get(..10).unwrap_or(iso).to_owned() };
    let d = (now - t).max(0);
    match d {
        0..60 => "just now".into(),
        60..3600 => format!("{}m ago", d / 60),
        3600..86400 => format!("{}h ago", d / 3600),
        86400..2592000 => format!("{}d ago", d / 86400),
        2592000..31536000 => format!("{}mo ago", d / 2592000),
        _ => format!("{}y ago", d / 31536000),
    }
}

/// Seconds since 1970 for "YYYY-MM-DD[THH:MM:SS…]" (UTC).
pub(crate) fn epoch_secs(iso: &str) -> Option<i64> {
    let n = |r: std::ops::Range<usize>| iso.get(r)?.parse::<i64>().ok();
    let (y, m, d) = (n(0..4)?, n(5..7)?, n(8..10)?);
    let (h, mi, s) = (n(11..13).unwrap_or(0), n(14..16).unwrap_or(0), n(17..19).unwrap_or(0));
    // Days from civil date (Howard Hinnant's algorithm).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some((era * 146097 + doe - 719468) * 86400 + h * 3600 + mi * 60 + s)
}

#[cfg(test)]
mod tests;
