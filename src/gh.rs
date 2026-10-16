//! GitHub via the `gh` CLI: PR lookup, review threads, posting comments.
use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::process::Command;

pub struct Pr {
    pub number: u64,
    pub title: String,
    pub base: String,
    pub head_oid: String,
}

pub struct Thread {
    pub path: String,
    pub left: bool,
    /// Current line in the PR diff; `None` when outdated.
    pub line: Option<u32>,
    pub start_line: Option<u32>,
    pub original_line: Option<u32>,
    pub resolved: bool,
    pub outdated: bool,
    pub comments: Vec<Comment>,
}

pub struct Comment {
    pub id: u64,
    pub author: String,
    pub body: String,
    pub created: String,
}

/// Where a new comment goes. Lines are file line numbers on the given side.
pub struct Anchor {
    pub path: String,
    pub line: u32,
    pub left: bool,
    pub start: Option<(u32, bool)>,
}

fn gh(args: &[&str]) -> Result<String> {
    let out = Command::new("gh").args(args).output().context("running gh")?;
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
        title: v["title"].as_str().unwrap_or("").to_owned(),
        base: v["baseRefName"].as_str().unwrap_or("main").to_owned(),
        head_oid: v["headRefOid"].as_str().unwrap_or("").to_owned(),
    })
}

// ponytail: first 100 threads × 100 comments; paginate if a PR ever outgrows that.
const THREADS: &str = "query($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) { pullRequest(number: $number) {
    reviewThreads(first: 100) { nodes {
      path line startLine originalLine diffSide isResolved isOutdated
      comments(first: 100) { nodes { databaseId author { login } body createdAt } }
    } }
  } }
}";

pub fn threads(n: u64) -> Result<Vec<Thread>> {
    let num = format!("number={n}");
    let q = format!("query={THREADS}");
    let v: Value = serde_json::from_str(&gh(&[
        "api", "graphql", "-F", "owner={owner}", "-F", "name={repo}", "-F", &num, "-f", &q,
    ])?)?;
    let nodes = v["data"]["repository"]["pullRequest"]["reviewThreads"]["nodes"].as_array().cloned();
    Ok(nodes.unwrap_or_default().iter().map(parse_thread).collect())
}

fn parse_thread(t: &Value) -> Thread {
    let line = |k: &str| t[k].as_u64().map(|n| n as u32);
    let comments = t["comments"]["nodes"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    Thread {
        path: t["path"].as_str().unwrap_or("").to_owned(),
        left: t["diffSide"] == "LEFT",
        line: line("line"),
        start_line: line("startLine"),
        original_line: line("originalLine"),
        resolved: t["isResolved"].as_bool().unwrap_or(false),
        outdated: t["isOutdated"].as_bool().unwrap_or(false),
        comments: comments
            .iter()
            .map(|c| Comment {
                id: c["databaseId"].as_u64().unwrap_or(0),
                author: c["author"]["login"].as_str().unwrap_or("ghost").to_owned(),
                body: c["body"].as_str().unwrap_or("").replace("\r\n", "\n"),
                created: c["createdAt"].as_str().unwrap_or("").get(..10).unwrap_or("").to_owned(),
            })
            .collect(),
    }
}

pub fn comment(pr: &Pr, a: &Anchor, body_file: &str) -> Result<()> {
    let side = |left: bool| if left { "LEFT" } else { "RIGHT" };
    let ep = format!("repos/{{owner}}/{{repo}}/pulls/{}/comments", pr.number);
    let mut f = vec![
        format!("body=@{body_file}"),
        format!("commit_id={}", pr.head_oid),
        format!("path={}", a.path),
        format!("line={}", a.line),
        format!("side={}", side(a.left)),
    ];
    if let Some((l, left)) = a.start {
        f.push(format!("start_line={l}"));
        f.push(format!("start_side={}", side(left)));
    }
    let mut args = vec!["api", "-X", "POST", ep.as_str()];
    for x in &f {
        // -F reads @file and sends numbers as numbers; -f keeps strings verbatim.
        let flag = if x.starts_with("body=") || x.starts_with("line=") || x.starts_with("start_line=") { "-F" } else { "-f" };
        args.extend([flag, x.as_str()]);
    }
    gh(&args).map(drop)
}

pub fn reply(pr: &Pr, comment_id: u64, body_file: &str) -> Result<()> {
    let ep = format!("repos/{{owner}}/{{repo}}/pulls/{}/comments/{comment_id}/replies", pr.number);
    gh(&["api", "-X", "POST", &ep, "-F", &format!("body=@{body_file}")]).map(drop)
}

/// A line of the PR info section; headings render bold.
pub struct InfoLine {
    pub heading: bool,
    pub text: String,
}

const INFO_FIELDS: &str = "title,author,state,isDraft,url,baseRefName,headRefName,additions,deletions,\
changedFiles,labels,reviewDecision,reviewRequests,latestReviews,statusCheckRollup,body,comments";

pub fn info(n: u64) -> Result<Vec<InfoLine>> {
    let v: Value = serde_json::from_str(&gh(&["pr", "view", &n.to_string(), "--json", INFO_FIELDS])?)?;
    Ok(info_lines(&v))
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
fn parses_thread() {
    let t = parse_thread(&serde_json::json!({
        "path": "src/a.rs", "line": 12, "startLine": 10, "originalLine": 12, "diffSide": "RIGHT",
        "isResolved": false, "isOutdated": false,
        "comments": { "nodes": [{ "databaseId": 7, "author": { "login": "amy" }, "body": "a\r\nb", "createdAt": "2026-10-01T00:00:00Z" }] }
    }));
    assert_eq!((t.left, t.line, t.start_line), (false, Some(12), Some(10)));
    assert_eq!((t.comments[0].id, t.comments[0].body.as_str(), t.comments[0].created.as_str()), (7, "a\nb", "2026-10-01"));
}
