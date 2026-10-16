//! GitHub via the `gh` CLI: PR lookup, review threads, posting comments.
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::io::Write;
use std::process::{Command, Stdio};

pub struct Pr {
    pub number: u64,
    /// GraphQL node id; filled in by `threads`.
    pub node_id: String,
    pub title: String,
    pub base: String,
    pub head_oid: String,
}

/// Review threads plus the viewer's pending review on the PR, if any.
pub struct Review {
    pub pr_id: String,
    pub pending: Option<String>,
    pub threads: Vec<Thread>,
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
}

pub struct Comment {
    pub id: u64,
    pub author: String,
    pub body: String,
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
pub fn checkout(n: u64, force: bool) -> Result<()> {
    let n = n.to_string();
    let mut args = vec!["pr", "checkout", &n];
    if force {
        args.push("--force");
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
    })
}

// ponytail: first 100 threads × 100 comments; paginate if a PR ever outgrows that.
const THREADS: &str = "query($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) { pullRequest(number: $number) {
    id
    reviews(states: PENDING, first: 10) { nodes { id viewerDidAuthor } }
    reviewThreads(first: 100) { nodes {
      id path line startLine originalLine diffSide isResolved isOutdated
      comments(first: 100) { nodes { databaseId author { login } body createdAt diffHunk state } }
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
                author: c["author"]["login"].as_str().unwrap_or("ghost").to_owned(),
                body: c["body"].as_str().unwrap_or("").replace("\r\n", "\n"),
                created: c["createdAt"].as_str().unwrap_or("").get(..10).unwrap_or("").to_owned(),
                pending: c["state"] == "PENDING",
            })
            .collect(),
    }
}

/// A single comment, posted immediately (outside any review).
pub fn comment(pr: &Pr, a: &Anchor, body: &str) -> Result<()> {
    let ep = format!("repos/{{owner}}/{{repo}}/pulls/{}/comments", pr.number);
    let mut v = json!({ "body": body, "commit_id": pr.head_oid, "path": a.path, "line": a.line, "side": side(a.left) });
    if let Some((l, left)) = a.start {
        (v["start_line"], v["start_side"]) = (json!(l), json!(side(left)));
    }
    gh_input(&["api", "-X", "POST", &ep, "--input", "-"], Some(&v.to_string())).map(drop)
}

/// A single reply, posted immediately.
pub fn reply(pr: &Pr, comment_id: u64, body: &str) -> Result<()> {
    let ep = format!("repos/{{owner}}/{{repo}}/pulls/{}/comments/{comment_id}/replies", pr.number);
    gh_input(&["api", "-X", "POST", &ep, "--input", "-"], Some(&json!({ "body": body }).to_string())).map(drop)
}

/// A line of the PR info section; headings render bold.
pub struct InfoLine {
    pub heading: bool,
    pub text: String,
}

const INFO_FIELDS: &str = "headRefOid,title,author,state,isDraft,url,baseRefName,headRefName,additions,deletions,\
changedFiles,labels,reviewDecision,reviewRequests,latestReviews,statusCheckRollup,body,comments";

/// PR info lines, plus the current (title, head commit) so a refresh picks up new pushes.
pub fn info(n: u64) -> Result<(Vec<InfoLine>, String, String)> {
    let v: Value = serde_json::from_str(&gh(&["pr", "view", &n.to_string(), "--json", INFO_FIELDS])?)?;
    let s = |k: &str| v[k].as_str().unwrap_or("").to_owned();
    Ok((info_lines(&v), s("title"), s("headRefOid")))
}

fn info_lines(v: &Value) -> Vec<InfoLine> {
    let s = |v: &Value| v.as_str().unwrap_or("").to_owned();
    let list = |v: &Value| v.as_array().map(Vec::as_slice).unwrap_or(&[]).to_vec();
    let mut out = vec![];
    let mut line = |heading, text: String| out.push(InfoLine { heading, text });

    let state = if v["isDraft"] == true { "draft".into() } else { s(&v["state"]).to_lowercase() };
    line(false, format!(
        "@{} · {state} · {} ← {} · +{} −{} in {} files",
        s(&v["author"]["login"]), s(&v["baseRefName"]), s(&v["headRefName"]),
        v["additions"], v["deletions"], v["changedFiles"],
    ));
    line(false, s(&v["url"]));
    let labels: Vec<String> = list(&v["labels"]).iter().map(|l| s(&l["name"])).collect();
    if !labels.is_empty() {
        line(false, format!("labels: {}", labels.join(", ")));
    }

    let decision = s(&v["reviewDecision"]).to_lowercase().replace('_', " ");
    let reviews: Vec<String> = list(&v["latestReviews"])
        .iter()
        .filter(|r| r["state"] == "APPROVED" || r["state"] == "CHANGES_REQUESTED")
        .map(|r| format!("@{} {}", s(&r["author"]["login"]), s(&r["state"]).to_lowercase().replace('_', " ")))
        .collect();
    let requested: Vec<String> = list(&v["reviewRequests"])
        .iter()
        .map(|r| format!("@{}", r["login"].as_str().or(r["name"].as_str()).unwrap_or("?")))
        .collect();
    let mut review = format!("review: {}", if decision.is_empty() { "none" } else { &decision });
    if !reviews.is_empty() {
        review += &format!(" · {}", reviews.join(", "));
    }
    if !requested.is_empty() {
        review += &format!(" · requested {}", requested.join(", "));
    }
    line(false, review);

    // CheckRun has status/conclusion; StatusContext has state.
    let checks = list(&v["statusCheckRollup"]);
    let verdict = |c: &Value| match (c["status"].as_str(), c["conclusion"].as_str().or(c["state"].as_str())) {
        (Some(st), _) if st != "COMPLETED" => '●',
        (_, Some("SUCCESS" | "NEUTRAL" | "SKIPPED")) => '✓',
        (_, Some("PENDING" | "EXPECTED")) | (_, None) => '●',
        _ => '✗',
    };
    if checks.is_empty() {
        line(false, "checks: none".into());
    } else {
        let count = |m| checks.iter().filter(|c| verdict(c) == m).count();
        line(false, format!("checks: ✓ {} passed · ✗ {} failed · ● {} pending", count('✓'), count('✗'), count('●')));
        for c in checks.iter().filter(|c| verdict(c) != '✓') {
            let name = c["name"].as_str().or(c["context"].as_str()).unwrap_or("?");
            line(false, format!("{} {name}", verdict(c)));
        }
    }

    line(true, "description".into());
    let body = s(&v["body"]).replace("\r\n", "\n");
    line(false, if body.trim().is_empty() { "(no description)".into() } else { body });

    let comments = list(&v["comments"]);
    line(true, format!("conversation ({})", comments.len()));
    for c in &comments {
        line(true, format!("@{} · {}", s(&c["author"]["login"]), s(&c["createdAt"]).get(..10).unwrap_or("")));
        line(false, s(&c["body"]).replace("\r\n", "\n"));
    }
    out
}

#[test]
fn info_summarises_checks() {
    let v = serde_json::json!({
        "title": "t", "author": { "login": "amy" }, "state": "OPEN", "isDraft": true,
        "baseRefName": "main", "headRefName": "feat", "additions": 3, "deletions": 1, "changedFiles": 2,
        "labels": [{ "name": "bug" }], "reviewDecision": "CHANGES_REQUESTED",
        "latestReviews": [{ "author": { "login": "bo" }, "state": "CHANGES_REQUESTED" }], "reviewRequests": [],
        "statusCheckRollup": [
            { "name": "build", "status": "COMPLETED", "conclusion": "SUCCESS" },
            { "name": "test", "status": "COMPLETED", "conclusion": "FAILURE" },
            { "context": "ci/legacy", "state": "PENDING" },
        ],
        "body": "", "comments": [],
    });
    let t: Vec<String> = info_lines(&v).into_iter().map(|l| l.text).collect();
    assert_eq!(t[0], "@amy · draft · main ← feat · +3 −1 in 2 files");
    assert_eq!(t[3], "review: changes requested · @bo changes requested");
    assert_eq!(&t[4..7], ["checks: ✓ 1 passed · ✗ 1 failed · ● 1 pending", "✗ test", "● ci/legacy"]);
}

#[test]
fn parses_review() {
    let r = parse_review(&json!({
        "id": "PR_1",
        "reviews": { "nodes": [{ "id": "R_other", "viewerDidAuthor": false }, { "id": "R_mine", "viewerDidAuthor": true }] },
        "reviewThreads": { "nodes": [{ "id": "T_1", "path": "a.rs", "comments": { "nodes": [{ "databaseId": 1, "state": "PENDING" }] } }] },
    }));
    assert_eq!((r.pr_id.as_str(), r.pending.as_deref()), ("PR_1", Some("R_mine")));
    assert_eq!((r.threads[0].id.as_str(), r.threads[0].comments[0].pending), ("T_1", true));
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
    assert_eq!((t.comments[0].id, t.comments[0].body.as_str(), t.comments[0].created.as_str()), (7, "a\nb", "2026-10-01"));
}
