//! GitHub via the `gh` CLI: PR lookup, review threads, posting comments.
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::io::Write;
use std::process::{Command, Stdio};

#[derive(Clone)]
pub struct Pr {
    pub number: u64,
    /// GraphQL node id; filled in by `threads`.
    pub node_id: String,
    pub title: String,
    pub base: String,
    pub head_oid: String,
    /// e.g. https://github.com/o/r/pull/1; filled in by `info`.
    pub url: String,
}

/// Review threads plus the viewer's pending review on the PR, if any.
pub struct Review {
    pub pr_id: String,
    pub pending: Option<String>,
    pub threads: Vec<Thread>,
    /// Your "Viewed" marks: path → VIEWED, or DISMISSED (the file changed after you marked it).
    pub viewed: std::collections::HashMap<String, String>,
}

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
}

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
}

/// Where a new comment goes. Lines are file line numbers on the given side.
pub struct Anchor {
    pub path: String,
    pub line: u32,
    pub left: bool,
    pub start: Option<(u32, bool)>,
}

fn gh(args: &[&str]) -> Result<String> {
    gh_input(args, None)
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
        bail!("gh {}: {}", args.first().unwrap_or(&""), String::from_utf8_lossy(&out.stderr).trim());
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
    let mut args = vec!["pr", "view", "--json", "number,title,baseRefName,headRefOid"];
    args.extend(n.as_deref());
    let v: Value = serde_json::from_str(&gh(&args)?)?;
    Ok(Pr {
        number: v["number"].as_u64().context("pr number")?,
        node_id: String::new(),
        title: v["title"].as_str().unwrap_or("").to_owned(),
        base: v["baseRefName"].as_str().unwrap_or("main").to_owned(),
        head_oid: v["headRefOid"].as_str().unwrap_or("").to_owned(),
        url: String::new(),
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

/// An open PR, for `dv list`.
pub struct Listed {
    pub number: u64,
    pub title: String,
    pub author: String,
    pub base: String,
    pub branch: String,
    pub draft: bool,
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
}

// ponytail: the 200 most recently updated open PRs; a search box if a repo has more worth scanning.
pub fn list() -> Result<Vec<Listed>> {
    const FIELDS: &str = "number,title,author,baseRefName,headRefName,isDraft,createdAt,updatedAt,additions,deletions,changedFiles,reviewDecision,labels";
    let v: Value = serde_json::from_str(&gh(&["pr", "list", "--limit", "200", "--json", FIELDS])?)?;
    let s = |v: &Value| v.as_str().unwrap_or("").to_owned();
    let n = |v: &Value| v.as_u64().unwrap_or(0);
    let prs = v
        .as_array()
        .context("gh pr list")?
        .iter()
        .map(|p| Listed {
            number: n(&p["number"]),
            title: s(&p["title"]),
            author: s(&p["author"]["login"]),
            base: s(&p["baseRefName"]),
            branch: s(&p["headRefName"]),
            draft: p["isDraft"] == true,
            created: s(&p["createdAt"]),
            updated: s(&p["updatedAt"]),
            additions: n(&p["additions"]),
            deletions: n(&p["deletions"]),
            files: n(&p["changedFiles"]),
            decision: s(&p["reviewDecision"]),
            labels: p["labels"].as_array().into_iter().flatten().map(|l| (s(&l["name"]), s(&l["color"]))).collect(),
            checks: None,
        })
        .collect();
    Ok(prs)
}

/// CI check counts for the PRs `list` returns: number → (passed, failed, pending).
pub fn list_checks() -> Result<std::collections::HashMap<u64, (usize, usize, usize)>> {
    let v: Value = serde_json::from_str(&gh(&["pr", "list", "--limit", "200", "--json", "number,statusCheckRollup"])?)?;
    let prs = v.as_array().context("gh pr list")?;
    Ok(prs
        .iter()
        .map(|p| {
            let checks = p["statusCheckRollup"].as_array().map(Vec::as_slice).unwrap_or(&[]);
            let count = |m| checks.iter().filter(|c| verdict(c) == m).count();
            (p["number"].as_u64().unwrap_or(0), (count('✓'), count('✗'), count('●')))
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

// ponytail: first 100 threads × 100 comments; paginate if a PR ever outgrows that.
const THREADS: &str = "query($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) { pullRequest(number: $number) {
    id
    reviews(states: PENDING, first: 10) { nodes { id viewerDidAuthor } }
    files(first: 100) { nodes { path viewerViewedState } }
    reviewThreads(first: 100) { nodes {
      id path line startLine originalLine diffSide isResolved isOutdated
      comments(first: 100) { nodes { id databaseId author { login } body createdAt diffHunk state viewerCanUpdate viewerCanDelete } }
    } }
  } }
}";

pub fn threads(n: u64) -> Result<Review> {
    let num = format!("number={n}");
    let q = format!("query={THREADS}");
    let v: Value = serde_json::from_str(&gh(&[
        "api", "graphql", "-F", "owner={owner}", "-F", "name={repo}", "-F", &num, "-f", &q,
    ])?)?;
    Ok(parse_review(&v["data"]["repository"]["pullRequest"]))
}

fn parse_review(pr: &Value) -> Review {
    let nodes = |v: &Value| v["nodes"].as_array().cloned().unwrap_or_default();
    Review {
        pr_id: pr["id"].as_str().unwrap_or("").to_owned(),
        pending: nodes(&pr["reviews"])
            .iter()
            .find(|r| r["viewerDidAuthor"] == true)
            .and_then(|r| r["id"].as_str().map(str::to_owned)),
        threads: nodes(&pr["reviewThreads"]).iter().map(parse_thread).collect(),
        viewed: nodes(&pr["files"])
            .iter()
            .filter(|f| f["viewerViewedState"] != "UNVIEWED")
            .filter_map(|f| Some((f["path"].as_str()?.to_owned(), f["viewerViewedState"].as_str()?.to_owned())))
            .collect(),
    }
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

/// Start a pending review on `commit`; returns its id.
pub fn start_review(pr_id: &str, commit: &str) -> Result<String> {
    let d = mutate(
        "mutation($pr: ID!, $commit: GitObjectID!) {
          addPullRequestReview(input: { pullRequestId: $pr, commitOID: $commit }) { pullRequestReview { id } } }",
        json!({ "pr": pr_id, "commit": commit }),
    )?;
    d["addPullRequestReview"]["pullRequestReview"]["id"].as_str().map(str::to_owned).context("no review id")
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
            })
            .collect(),
        claude: None,
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
    Link(String),
}

/// A line of the PR info section. `md` lines (description, comments) are rendered as markdown;
/// the rest are colored by `marks` (char ranges).
pub struct InfoLine {
    pub heading: bool,
    pub md: bool,
    pub text: String,
    pub marks: Vec<(usize, usize, Mark)>,
    /// On a comment's (or the description's) heading: its link on GitHub, for `Space c`.
    pub url: String,
}

impl InfoLine {
    fn new(heading: bool, md: bool) -> Self {
        InfoLine { heading, md, text: String::new(), marks: vec![], url: String::new() }
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

const INFO_FIELDS: &str = "headRefOid,title,author,state,isDraft,url,baseRefName,headRefName,additions,deletions,\
changedFiles,labels,reviewDecision,reviewRequests,latestReviews,statusCheckRollup,body,comments";

/// PR info lines, plus the current (title, head commit, url) so a refresh picks up new pushes.
pub fn info(n: u64) -> Result<(Vec<InfoLine>, String, String, String)> {
    let v: Value = serde_json::from_str(&gh(&["pr", "view", &n.to_string(), "--json", INFO_FIELDS])?)?;
    let s = |k: &str| v[k].as_str().unwrap_or("").to_owned();
    Ok((info_lines(&v), s("title"), s("headRefOid"), s("url")))
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

    // State badge in GitHub's colors, then author, branches, size.
    let (state, color) = match (v["isDraft"] == true, s(&v["state"]).as_str()) {
        (true, _) => ("DRAFT", "6e7681"),
        (_, "MERGED") => ("MERGED", "8957e5"),
        (_, "CLOSED") => ("CLOSED", "da3633"),
        _ => ("OPEN", "238636"),
    };
    let author = s(&v["author"]["login"]);
    let mut l = InfoLine::new(false, false);
    l.put(&format!(" {state} "), Some(Mark::Chip(color.into())))
        .put(" ", None)
        .put(&format!("@{author}"), Some(Mark::Link(format!("https://github.com/{author}"))))
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

    // Who did what, once: "✗ changes requested by @bo · ✓ approved by @amy · waiting on @cy".
    let by = |state: &str| -> Vec<String> {
        let reviews = list(&v["latestReviews"]);
        reviews.iter().filter(|r| r["state"] == state).map(|r| format!("@{}", s(&r["author"]["login"]))).collect()
    };
    let requested: Vec<String> = list(&v["reviewRequests"])
        .iter()
        .map(|r| format!("@{}", r["login"].as_str().or(r["name"].as_str()).unwrap_or("?")))
        .collect();
    let groups = [
        ("✗ changes requested by ", by("CHANGES_REQUESTED"), Some(Mark::Fail)),
        ("✓ approved by ", by("APPROVED"), Some(Mark::Pass)),
        ("waiting on ", requested, Some(Mark::Pending)),
    ];
    let mut l = InfoLine::new(false, false);
    l.put("review: ", None);
    let mut any = false;
    for (what, who, mark) in groups.into_iter().filter(|g| !g.1.is_empty()) {
        if any {
            l.put(" · ", None);
        }
        l.put(&format!("{what}{}", who.join(", ")), mark);
        any = true;
    }
    if !any {
        let decision = s(&v["reviewDecision"]).to_lowercase().replace('_', " ");
        l.put(if decision.is_empty() { "none yet" } else { &decision }, Some(Mark::Dim));
    }
    out.push(l);

    let checks = list(&v["statusCheckRollup"]);
    if checks.is_empty() {
        out.push(plain("checks: none".into()));
    } else {
        let count = |m| checks.iter().filter(|c| verdict(c) == m).count();
        let (pass, fail, pending) = (count('✓'), count('✗'), count('●'));
        let mut l = InfoLine::new(false, false);
        l.put("checks: ", None);
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
            l.put(&format!("{} {name}", verdict(c)), Some(m));
            out.push(l);
        }
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
    out.push(InfoLine { url: s(&v["url"]), ..heading("description".into()) });
    let body = s(&v["body"]).replace("\r\n", "\n");
    out.push(markdown(body)); // empty: the view shows a placeholder

    let comments = list(&v["comments"]);
    out.push(heading(format!("conversation ({})", comments.len())));
    for c in &comments {
        let h = heading(format!("@{} · {}", s(&c["author"]["login"]), ago(&s(&c["createdAt"]))));
        out.push(InfoLine { url: s(&c["url"]), ..h });
        out.push(markdown(s(&c["body"]).replace("\r\n", "\n")));
    }
    out
}

#[test]
fn info_summarises_checks() {
    let v = serde_json::json!({
        "title": "t", "author": { "login": "amy" }, "state": "OPEN", "isDraft": true,
        "baseRefName": "main", "headRefName": "feat", "additions": 3, "deletions": 1, "changedFiles": 2,
        "labels": [{ "name": "bug", "color": "d73a4a" }], "reviewDecision": "CHANGES_REQUESTED",
        "latestReviews": [{ "author": { "login": "bo" }, "state": "CHANGES_REQUESTED" }], "reviewRequests": [],
        "statusCheckRollup": [
            { "name": "build", "status": "COMPLETED", "conclusion": "SUCCESS" },
            { "name": "test", "status": "COMPLETED", "conclusion": "FAILURE" },
            { "context": "ci/legacy", "state": "PENDING" },
        ],
        "body": "", "comments": [],
    });
    let lines = info_lines(&v);
    assert_eq!(lines[0].marks[0], (0, 7, Mark::Chip("6e7681".into()))); // " DRAFT " badge
    assert_eq!(lines[1].marks[1], (7, 12, Mark::Chip("d73a4a".into()))); // " bug " in its label color
    let t: Vec<String> = lines.into_iter().map(|l| l.text).collect();
    assert_eq!(t[0], " DRAFT  @amy · main ← feat · +3 −1 in 2 files");
    assert_eq!(t[2], "review: ✗ changes requested by @bo");
    assert_eq!(&t[3..6], ["checks: ✓ 1 passed · ✗ 1 failed · ● 1 pending", "✗ test", "● ci/legacy"]);
    let v = serde_json::json!({ "statusCheckRollup": [{ "name": "build", "status": "COMPLETED", "conclusion": "SUCCESS" }] });
    let t: Vec<String> = info_lines(&v).into_iter().map(|l| l.text).collect();
    assert!(t.contains(&"checks: ✓ 1 passed".to_owned())); // no "✗ 0 failed · ● 0 pending"
    assert!(t.contains(&"review: none yet".to_owned()));
}

/// "3h ago", "2d ago", "5mo ago", like GitHub. Falls back to the date if it can't be parsed.
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
fn epoch_secs(iso: &str) -> Option<i64> {
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

#[test]
fn issue_links() {
    assert_eq!(issue_ref("https://github.com/o/r/issues/12"), Some(("o", "r", 12)));
    assert_eq!(issue_ref("https://github.com/o/r/pull/7#discussion_r1"), Some(("o", "r", 7)));
    assert_eq!(issue_ref("https://github.com/o/r/pull/7/files?w=1"), Some(("o", "r", 7)));
    assert_eq!(issue_ref("https://github.com/amy"), None);
    assert_eq!(issue_ref("https://github.com/o/r/blob/main/x.rs"), None);
}

#[test]
fn relative_time() {
    assert_eq!(epoch_secs("1970-01-02T00:00:00Z"), Some(86400));
    assert_eq!(epoch_secs("2000-03-01"), Some(951868800));
    let now = epoch_secs("2026-10-16T12:00:00Z").unwrap();
    assert_eq!(ago_at("2026-10-16T11:59:30Z", now), "just now");
    assert_eq!(ago_at("2026-10-16T09:00:00Z", now), "3h ago");
    assert_eq!(ago_at("2026-10-14T12:00:00Z", now), "2d ago");
    assert_eq!(ago_at("2026-08-14T12:00:00Z", now), "2mo ago");
    assert_eq!(ago_at("garbage", now), "garbage");
}

#[test]
fn parses_review() {
    let r = parse_review(&json!({
        "id": "PR_1",
        "reviews": { "nodes": [{ "id": "R_other", "viewerDidAuthor": false }, { "id": "R_mine", "viewerDidAuthor": true }] },
        "files": { "nodes": [{ "path": "a.rs", "viewerViewedState": "VIEWED" }, { "path": "b.rs", "viewerViewedState": "UNVIEWED" }] },
        "reviewThreads": { "nodes": [{ "id": "T_1", "path": "a.rs", "comments": { "nodes": [{ "id": "C_1", "databaseId": 1, "state": "PENDING", "viewerCanUpdate": true }] } }] },
    }));
    assert_eq!((r.pr_id.as_str(), r.pending.as_deref()), ("PR_1", Some("R_mine")));
    assert_eq!(r.viewed.len(), 1);
    assert_eq!(r.viewed["a.rs"], "VIEWED");
    assert_eq!((r.threads[0].id.as_str(), r.threads[0].comments[0].pending), ("T_1", true));
    assert_eq!((r.threads[0].comments[0].node_id.as_str(), r.threads[0].comments[0].can_edit), ("C_1", true));
}

#[test]
fn parses_thread() {
    let t = parse_thread(&serde_json::json!({
        "path": "src/a.rs", "line": 12, "startLine": 10, "originalLine": 12, "diffSide": "RIGHT",
        "isResolved": false, "isOutdated": false,
        "comments": { "nodes": [{ "databaseId": 7, "author": { "login": "amy" }, "body": "a\r\nb", "createdAt": "2026-10-01T00:00:00Z",
            "diffHunk": "@@ -1,2 +1,2 @@\n fn a() {\n+    let x = 1;" }] }
    }));
    assert_eq!((t.left, t.line, t.start_line), (false, Some(12), Some(10)));
    assert_eq!(t.hunk, [" fn a() {", "+    let x = 1;"]);
    assert_eq!((t.comments[0].id, t.comments[0].body.as_str(), t.comments[0].created.as_str()), (7, "a\nb", "2026-10-01T00:00:00Z"));
}
