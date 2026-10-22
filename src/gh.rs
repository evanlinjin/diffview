//! GitHub via the `gh` CLI: PR lookup, review threads, posting comments.
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::io::Write;
use std::process::{Command, Stdio};

mod ci;
pub use ci::*;

#[derive(Clone, Default)]
pub struct Pr {
    pub number: u64,
    /// GraphQL node id; filled in by `threads`.
    pub node_id: String,
    pub title: String,
    pub base: String,
    pub head_oid: String,
    /// e.g. https://github.com/o/r/pull/1.
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
    /// The side its range starts on, when GitHub says (it can start on the old side and end
    /// on the new); else its own (`left`).
    pub start_left: Option<bool>,
    pub original_line: Option<u32>,
    pub resolved: bool,
    /// Claude's (from its review file): the conversation under its comment resolved, the
    /// comment itself still open (what's kept, and posted).
    pub talk_resolved: bool,
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

/// Who `v` (GitHub's author, actor: `{login}`) is: a deleted account's, GitHub's "ghost".
pub(crate) fn login(v: &Value) -> String {
    v["login"].as_str().unwrap_or("ghost").to_owned()
}

/// Whether `err` (a gh run's, as said) is GitHub out of reach (the network: a timeout, no
/// connection, no DNS), not something to see to (a login run out, a refusal).
pub(crate) fn unreachable(err: &str) -> bool {
    let e = err.to_lowercase();
    ["timeout", "timed out", "error connecting", "connection refused", "connection reset", "no such host", "name resolution", "network is unreachable", "eof"]
        .iter()
        .any(|w| e.contains(w))
}

#[test]
fn github_out_of_reach_or_not() {
    assert!(unreachable("gh api: Post \"https://api.github.com/graphql\": net/http: TLS handshake timeout"));
    assert!(unreachable("error connecting to api.github.com"));
    assert!(!unreachable("gh api: HTTP 401: Bad credentials (https://api.github.com/graphql)\nTry authenticating with:  gh auth login"));
    assert!(!unreachable("running gh: No such file or directory (os error 2)"));
}

/// `dv repo-file`: a file of GitHub repo `repo` ("owner/name", "owner/name@ref": else its
/// default branch), as it is (text, up to 1MB); a directory, its entries ("name", "dir/").
/// Read only (GET), only a repo's contents, and only a public repo's or this one's (text
/// Claude reads, a PR's, could ask it for a private one's), for Claude to read other repos as it
/// can't `gh api`.
pub(crate) fn repo_file(repo: &str, path: &str) -> Result<String> {
    let (repo, rf) = repo.split_once('@').unwrap_or((repo, ""));
    // (Nothing that could reach past the repo's contents: no `..`, each part encoded.)
    let part = |p: &str| !p.is_empty() && p != "." && p != "..";
    let ok = repo.split_once('/').is_some_and(|(o, r)| part(o) && part(r) && !r.contains('/'));
    anyhow::ensure!(ok, "a repo is owner/name (or owner/name@ref), not {repo:?}");
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty() && *p != ".").collect();
    anyhow::ensure!(parts.iter().all(|p| part(p)), "a path within the repo, not {path:?}");
    let enc = |s: &str| s.split('/').map(percent).collect::<Vec<_>>().join("/");
    let meta = gh_json(&["api", "-X", "GET", &format!("repos/{}", enc(repo))])?;
    if meta["private"] != false {
        let here = gh_json(&["repo", "view", "--json", "nameWithOwner"]).ok().and_then(|v| v["nameWithOwner"].as_str().map(str::to_lowercase));
        anyhow::ensure!(here.as_deref() == Some(repo.to_lowercase().as_str()), "{repo} isn't public (only a public repo's files, or this repo's)");
    }
    let at = if rf.is_empty() { String::new() } else { format!("?ref={}", percent(rf)) };
    let v = gh_json(&["api", "-X", "GET", &format!("repos/{}/contents/{}{at}", enc(repo), enc(&parts.join("/")))])?;
    if let Some(entries) = v.as_array() {
        let mut names: Vec<String> = entries.iter().map(|e| format!("{}{}", e["name"].as_str().unwrap_or(""), if e["type"] == "dir" { "/" } else { "" })).collect();
        // (GitHub lists a directory's first 1000 only.)
        if entries.len() >= 1000 {
            names.push("… (GitHub lists only the first 1000)".into());
        }
        return Ok(names.join("\n") + "\n");
    }
    anyhow::ensure!(v["type"] == "file", "{path} in {repo} is a {}, not a file or a directory", v["type"].as_str().unwrap_or("?"));
    // Up to 1MB, the file is in the answer (that very blob); more, not read.
    let size = v["size"].as_u64().unwrap_or(0);
    anyhow::ensure!(v["encoding"] == "base64", "{path} is {size} bytes: too big to read here (1MB at most)");
    let bytes = base64(v["content"].as_str().unwrap_or("")).context("its content didn't decode")?;
    String::from_utf8(bytes).map_err(|_| anyhow::anyhow!("{path} isn't text ({size} bytes)"))
}

/// Standard base64 (`=` padded; whitespace, as GitHub wraps it, skipped), decoded.
fn base64(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    };
    let digits: Vec<u8> = s.bytes().filter(|c| !c.is_ascii_whitespace() && *c != b'=').map(val).collect::<Option<_>>()?;
    let mut out = Vec::with_capacity(digits.len() * 3 / 4);
    for chunk in digits.chunks(4) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, &d)| n | (d as u32) << (18 - 6 * i));
        out.extend(&n.to_be_bytes()[1..chunk.len()]);
    }
    Some(out)
}

#[test]
fn base64_as_github_gives_it() {
    assert_eq!(base64("aGk=\n").as_deref(), Some(&b"hi"[..]));
    assert_eq!(base64("W3Rvb2xj\naGFpbl0K").as_deref(), Some(&b"[toolchain]\n"[..]));
    assert_eq!(base64("YQ==").as_deref(), Some(&b"a"[..]));
    assert!(base64("a!b").is_none());
}

#[test]
fn repo_file_stays_in_a_repos_contents() {
    // (Refused before GitHub's asked: nothing reaches past the contents.)
    for (repo, path) in [("a/..", "x"), ("/x", "y"), ("a/", "y"), ("a/b/c", "y"), ("a", "y"), ("o/r", "../../user"), ("o/r", "a/../../b"), ("o/r", "./x")] {
        assert!(repo_file(repo, path).is_err(), "{repo} {path}");
    }
}

/// `gh args`, its output as JSON.
pub(crate) fn gh_json(args: &[&str]) -> Result<Value> {
    Ok(serde_json::from_str(&gh(args)?)?)
}

pub(crate) fn gh(args: &[&str]) -> Result<String> {
    gh_input(args, None)
}

/// PR (or issue) `n`'s title and description as they are on GitHub now (the issues API has
/// both's).
pub fn text(n: u64) -> Result<(String, String)> {
    let v: Value = gh_json(&["api", &format!("repos/{{owner}}/{{repo}}/issues/{n}")])?;
    let s = |k: &str| v[k].as_str().unwrap_or("").replace("\r\n", "\n");
    Ok((s("title"), s("body")))
}

/// PR (or issue) `n`'s description `body`, and its `title` if given, on GitHub.
pub fn set_text(n: u64, title: Option<&str>, body: &str) -> Result<()> {
    let mut v = json!({ "body": body });
    if let Some(t) = title {
        v["title"] = t.into();
    }
    gh_input(&["api", "-X", "PATCH", &format!("repos/{{owner}}/{{repo}}/issues/{n}"), "--input", "-"], Some(&v.to_string())).map(drop)
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
    // A GraphQL answer with some of it refused (a field the viewer can't read: a base's branch
    // protection, say): gh exits non-zero, but what came back is the answer, its `errors` with
    // it, for the caller to weigh. Only none at all is gh failing.
    let answered = || serde_json::from_slice::<Value>(&out.stdout).is_ok_and(|v| !v["data"].is_null());
    if !out.status.success() && !answered() {
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
    let mut args = vec!["pr", "view", "--json", PR_FIELDS];
    args.extend(n.as_deref());
    let v: Value = gh_json(&args)?;
    pr_of(&v)
}

/// How long what dv last fetched of a PR stands in for asking GitHub again, in dv's commands
/// Claude runs (`dv review-check` after each edit of a review, say).
pub(crate) const RECENT: std::time::Duration = std::time::Duration::from_secs(15 * 60);

/// Whether `p` was written in the last `RECENT`.
pub(crate) fn recent(p: &std::path::Path) -> bool {
    std::fs::metadata(p).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age < RECENT)
}

/// What `pr_of` reads.
const PR_FIELDS: &str = "number,title,url,baseRefName,headRefOid,baseRefOid,headRefName,headRepositoryOwner,author,isCrossRepository,headRepository,maintainerCanModify";

#[test]
fn a_pr_from_its_info() {
    // dv's cache of a PR's info stands in for `pr` (`view::recent_github`): it has all `pr_of` reads.
    for f in PR_FIELDS.split(',') {
        assert!(INFO_FIELDS.split(',').any(|i| i == f), "{f}");
    }
    let p = pr_of(&serde_json::json!({ "number": 3, "baseRefOid": "b", "headRefName": "x", "author": { "login": "a" } })).unwrap();
    assert_eq!((p.number, p.base_oid.as_str(), p.head_branch.as_str(), p.author.as_str()), (3, "b", "x", "a"));
}

/// The PR in `v`: `gh pr view`'s JSON, `pr`'s or `info_json`'s (`INFO_FIELDS` has all it reads).
pub(crate) fn pr_of(v: &Value) -> Result<Pr> {
    Ok(Pr {
        number: v["number"].as_u64().context("pr number")?,
        node_id: String::new(),
        title: v["title"].as_str().unwrap_or("").to_owned(),
        base: v["baseRefName"].as_str().unwrap_or("main").to_owned(),
        head_oid: v["headRefOid"].as_str().unwrap_or("").to_owned(),
        url: v["url"].as_str().unwrap_or("").to_owned(),
        base_oid: v["baseRefOid"].as_str().unwrap_or("").to_owned(),
        head_branch: v["headRefName"].as_str().unwrap_or("").to_owned(),
        head_owner: v["headRepositoryOwner"]["login"].as_str().unwrap_or("").to_owned(),
        author: login(&v["author"]),
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
    let v: Value = gh_json(&["api", &format!("repos/{owner}/{repo}/issues/{n}")])?;
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

/// PR `n`'s state on GitHub now: OPEN, CLOSED or MERGED (`None` if gh doesn't say).
pub fn pr_state(n: u64) -> Result<Option<String>> {
    let v: Value = gh_json(&["pr", "view", &n.to_string(), "--json", "state"])?;
    Ok(v["state"].as_str().map(str::to_owned))
}

/// This repo on GitHub, as `owner/name`.
pub fn repo_name() -> Result<String> {
    Ok(gh(&["repo", "view", "--json", "nameWithOwner", "-q", ".nameWithOwner"])?.trim().to_owned())
}

const COMMENT: &str = "id databaseId author { login } body createdAt diffHunk state viewerCanUpdate viewerCanDelete \
reactionGroups { content viewerHasReacted reactors { totalCount } } isMinimized minimizedReason";
const THREAD: &str = "id path line startLine startDiffSide originalLine diffSide isResolved isOutdated";
pub(crate) const PAGE: &str = "pageInfo { hasNextPage endCursor }";

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
        let v: Value = gh_json(&args)?;
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
pub(crate) fn next_page(list: &Value) -> Option<String> {
    let p = &list["pageInfo"];
    (p["hasNextPage"] == true).then(|| p["endCursor"].as_str().map(str::to_owned)).flatten()
}

/// API `path`'s ETag, if it has changed from `etag` (empty: the first look); `None` if GitHub
/// says 304 Not Modified, which doesn't count against the rate limit. A PR's (or an issue's):
/// new comments, replies, reviews and pushes change it; edits and resolving a thread don't.
pub(crate) fn etag_of(path: &str, etag: &str) -> Result<Option<String>> {
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

/// A reaction on a comment: GitHub's name for it (e.g. THUMBS_UP), how many, one is yours.
pub type Reaction = (String, u64, bool);

/// A comment's `reactionGroups` (`reactors`, `viewerHasReacted`), those with any.
pub(crate) fn reactions(v: &Value) -> Vec<Reaction> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|g| {
            let n = g["reactors"]["totalCount"].as_u64()?;
            (n > 0).then(|| (g["content"].as_str().unwrap_or("").to_owned(), n, g["viewerHasReacted"] == true))
        })
        .collect()
}

/// The reactions on comment (or review) `node` now: each kind's (name, count, yours).
pub fn reactions_of(node: &str) -> Result<Vec<Reaction>> {
    let d = graphql(&format!("query($id: ID!) {{ node(id: $id) {{ ... on Reactable {{ {REACTED} }} }} }}"), json!({ "id": node }))?;
    Ok(reactions(&d["node"]["reactionGroups"]))
}

/// React to comment (or review) `node` with `content` (`add`), or take yours back.
pub fn react(node: &str, content: &str, add: bool) -> Result<()> {
    let op = if add { "addReaction" } else { "removeReaction" };
    let q = format!("mutation($id: ID!, $c: ReactionContent!) {{ {op}(input: {{ subjectId: $id, content: $c }}) {{ clientMutationId }} }}");
    graphql(&q, json!({ "id": node, "c": content })).map(drop)
}

/// Run a GraphQL query or mutation; variables go in a JSON body so strings need no escaping.
pub(crate) fn graphql(query: &str, vars: Value) -> Result<Value> {
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
    let d = graphql(
        "mutation($pr: ID!, $commit: GitObjectID!, $body: String) {
          addPullRequestReview(input: { pullRequestId: $pr, commitOID: $commit, body: $body }) { pullRequestReview { id } } }",
        json!({ "pr": pr_id, "commit": commit, "body": (!body.is_empty()).then_some(body) }),
    )?;
    d["addPullRequestReview"]["pullRequestReview"]["id"].as_str().map(str::to_owned).context("no review id")
}

/// A review with no comments, created and submitted at once (so a refusal leaves nothing behind).
pub fn review_now(pr_id: &str, commit: &str, event: &str, body: &str) -> Result<()> {
    graphql(
        "mutation($pr: ID!, $commit: GitObjectID!, $event: PullRequestReviewEvent!, $body: String) {
          addPullRequestReview(input: { pullRequestId: $pr, commitOID: $commit, event: $event, body: $body }) {
            pullRequestReview { id } } }",
        json!({ "pr": pr_id, "commit": commit, "event": event, "body": body }),
    )
    .map(drop)
}

pub fn add_thread(review: &str, a: &Anchor, body: &str) -> Result<()> {
    let (start_line, start_side) = a.start.map_or((None, None), |(l, left)| (Some(l), Some(side(left))));
    graphql(
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
    graphql(
        "mutation($review: ID!, $thread: ID!, $body: String!) {
          addPullRequestReviewThreadReply(input: { pullRequestReviewId: $review,
            pullRequestReviewThreadId: $thread, body: $body }) { comment { id } } }",
        json!({ "review": review, "thread": thread, "body": body }),
    )
    .map(drop)
}

/// `event`: COMMENT, APPROVE or REQUEST_CHANGES.
pub fn submit_review(review: &str, event: &str, body: &str) -> Result<()> {
    graphql(
        "mutation($review: ID!, $event: PullRequestReviewEvent!, $body: String) {
          submitPullRequestReview(input: { pullRequestReviewId: $review, event: $event, body: $body }) {
            pullRequestReview { id } } }",
        json!({ "review": review, "event": event, "body": body }),
    )
    .map(drop)
}

/// Replace a pending review's text (its summary).
pub fn set_review_body(review: &str, body: &str) -> Result<()> {
    graphql(
        "mutation($review: ID!, $body: String!) {
          updatePullRequestReview(input: { pullRequestReviewId: $review, body: $body }) { pullRequestReview { id } } }",
        json!({ "review": review, "body": body }),
    )
    .map(drop)
}

pub fn delete_review(review: &str) -> Result<()> {
    graphql(
        "mutation($review: ID!) { deletePullRequestReview(input: { pullRequestReviewId: $review }) {
          pullRequestReview { id } } }",
        json!({ "review": review }),
    )
    .map(drop)
}

pub fn delete_comment(comment: &str) -> Result<()> {
    graphql(
        "mutation($c: ID!) { deletePullRequestReviewComment(input: { id: $c }) { clientMutationId } }",
        json!({ "c": comment }),
    )
    .map(drop)
}

pub fn edit_comment(comment: &str, body: &str) -> Result<()> {
    graphql(
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
    graphql(q, json!({ "pr": pr_id, "path": path })).map(drop)
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
    let v: Value = gh_json(&["pr", "view", &n.to_string(), "--json", "labels,assignees,reviewRequests,author"])?;
    let on = |k: &str, f: &str| -> Vec<String> { v[k].as_array().into_iter().flatten().filter_map(|x| x[f].as_str().map(str::to_owned)).collect() };
    let (labels, assigned) = (on("labels", "name"), on("assignees", "login"));
    // Requested of people (their login), or of teams (gh gives their slug as `org/team`).
    let asked: Vec<String> = v["reviewRequests"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| r["login"].as_str().or_else(|| r["slug"].as_str()).map(str::to_owned))
        .collect();
    let (all, people) = repo_choices()?;
    let author = &*login(&v["author"]);
    let reviewers = people.iter().filter(|p| *p != author).cloned().collect();
    Ok((mark(all, &labels), mark(people, &assigned), mark(reviewers, &asked)))
}

/// Issue `n`'s labels and assignees, and what it could have, as `pr_fields`.
pub fn issue_fields(n: u64) -> Result<(Choices, Choices)> {
    let v: Value = gh_json(&["issue", "view", &n.to_string(), "--json", "labels,assignees"])?;
    let on = |k: &str, f: &str| -> Vec<String> { v[k].as_array().into_iter().flatten().filter_map(|x| x[f].as_str().map(str::to_owned)).collect() };
    let (all, people) = repo_choices()?;
    Ok((mark(all, &on("labels", "name")), mark(people, &on("assignees", "login"))))
}

/// The repo's labels, and the people its issues and PRs can be assigned to.
fn repo_choices() -> Result<(Vec<String>, Vec<String>)> {
    let all: Value = gh_json(&["label", "list", "--limit", "500", "--json", "name"])?;
    let all: Vec<String> = all.as_array().into_iter().flatten().filter_map(|x| x["name"].as_str().map(str::to_owned)).collect();
    let people = gh(&["api", "--paginate", "repos/{owner}/{repo}/assignees", "-q", ".[].login"]).unwrap_or_default();
    Ok((all, people.lines().map(str::to_owned).collect()))
}

/// `all`, each with whether it's `on`, those on first, then by name (one each).
fn mark(all: Vec<String>, on: &[String]) -> Choices {
    let mut out: Vec<(String, bool)> = all.into_iter().chain(on.iter().cloned()).map(|x| (x.clone(), on.contains(&x))).collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.to_lowercase().cmp(&b.0.to_lowercase())));
    out.dedup();
    out
}

/// Names, each with whether it's on the PR.
pub type Choices = Vec<(String, bool)>;

/// Add (`on`) or remove a label, an assignee (on PR or issue `n`: the issues API has both's),
/// or a requested reviewer (a PR's).
pub fn set_field(n: u64, field: Field, name: &str, on: bool) -> Result<()> {
    let ep = format!("repos/{{owner}}/{{repo}}/issues/{n}");
    let method = if on { "POST" } else { "DELETE" };
    match field {
        // (A label's name may have a comma in it, which `gh pr edit` splits on: the API takes it whole.)
        Field::Label if on => gh(&["api", "-X", "POST", &format!("{ep}/labels"), "-f", &format!("labels[]={name}")]).map(drop),
        Field::Label => gh(&["api", "-X", "DELETE", &format!("{ep}/labels/{}", percent(name))]).map(drop),
        Field::Assignee => gh(&["api", "-X", method, &format!("{ep}/assignees"), "-f", &format!("assignees[]={name}")]).map(drop),
        Field::Reviewer => gh(&["pr", "edit", &n.to_string(), if on { "--add-reviewer" } else { "--remove-reviewer" }, name]).map(drop),
    }
}

pub fn set_resolved(thread: &str, resolved: bool) -> Result<()> {
    let q = match resolved {
        true => "mutation($t: ID!) { resolveReviewThread(input: { threadId: $t }) { thread { isResolved } } }",
        false => "mutation($t: ID!) { unresolveReviewThread(input: { threadId: $t }) { thread { isResolved } } }",
    };
    graphql(q, json!({ "t": thread })).map(drop)
}

impl Thread {
    /// Resolved or outdated: shown dimmed and folded until expanded.
    pub fn stale(&self) -> bool {
        self.resolved || self.outdated
    }

    /// Where its range starts, a range comment's: its first line and the side it's on (before
    /// its line, on its side; or on the other, the old side's). None when it's on one line, or
    /// its line's gone (outdated: its start is of code since changed).
    pub fn range_start(&self) -> Option<(u32, bool)> {
        let (s, l, left) = (self.start_line?, self.line?, self.start_left.unwrap_or(self.left));
        (left != self.left || s < l).then_some((s, left))
    }

    /// A Claude thread's lines, on its one side, outdated or not: its range's start (a range's)
    /// and its line.
    pub fn own_lines(&self) -> Option<(Option<u32>, u32)> {
        let line = self.line.or(self.original_line)?;
        Some((self.start_line.filter(|&s| s < line), line))
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
        start_left: t["startDiffSide"].as_str().map(|s| s == "LEFT"),
        original_line: line("originalLine"),
        resolved: t["isResolved"].as_bool().unwrap_or(false),
        talk_resolved: false,
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
                author: login(&c["author"]),
                body: c["body"].as_str().unwrap_or("").replace("\r\n", "\n"),
                created: c["createdAt"].as_str().unwrap_or("").to_owned(),
                pending: c["state"] == "PENDING",
                reactions: reactions(&c["reactionGroups"]),
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

/// A comment on PR (or issue) `n`'s conversation (not tied to a line or a review).
pub fn conversation_comment(n: u64, body: &str) -> Result<()> {
    let ep = format!("repos/{{owner}}/{{repo}}/issues/{n}/comments");
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
/// Close issue `n` with `comment`, as `reason` says (`completed`, `not planned`, `duplicate`
/// of `dup`).
pub fn close_issue(n: u64, comment: &str, reason: &str, dup: Option<u64>) -> Result<()> {
    let (n, dup) = (n.to_string(), dup.map(|d| d.to_string()));
    let mut args = vec!["issue", "close", &n, "--comment", comment];
    match (reason, &dup) {
        ("duplicate", Some(d)) => args.extend(["--duplicate-of", d]),
        ("not planned", _) => args.extend(["--reason", "not planned"]),
        _ => args.extend(["--reason", "completed"]),
    }
    gh(&args).map(drop)
}

pub fn close(n: u64, comment: Option<&str>) -> Result<()> {
    let n = n.to_string();
    let mut args = vec!["pr", "close", &n];
    args.extend(comment.map(|c| ["--comment", c]).into_iter().flatten());
    gh(&args).map(drop)
}

/// A single reply, posted immediately.
pub fn reply(pr: &Pr, comment_id: u64, body: &str) -> Result<()> {
    let ep = format!("repos/{{owner}}/{{repo}}/pulls/{}/comments/{comment_id}/replies", pr.number);
    post(&ep, &json!({ "body": body }))
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
pub(crate) const SIGNATURE_RULES: &str = "baseRef { branchProtectionRule { requiresCommitSignatures } rules(first: 100) { nodes { type } } }";

/// Whether a PR's base (its node, with `SIGNATURE_RULES`) requires signed commits.
pub(crate) fn requires_signatures(p: &Value) -> bool {
    let b = &p["baseRef"];
    b["branchProtectionRule"]["requiresCommitSignatures"] == true || b["rules"]["nodes"].as_array().is_some_and(|r| r.iter().any(|r| r["type"] == "REQUIRED_SIGNATURES"))
}

/// PR `n`'s commits' signatures, and whether its base requires them: one GraphQL query.
pub fn signatures(n: u64) -> Result<Signatures> {
    signatures_of(&pr_query(n)?)
}

/// The first line of a message (GitHub's can run on), for a line of dv's.
fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").to_owned()
}

/// A GraphQL answer about PRs (`v`), weighed: what it says of a base's rules may be refused (a
/// ruleset or branch protection you can't read): taken as not requiring anything. Anything else
/// refused, none of it: a commit GitHub had trouble with isn't one that isn't signed, a review
/// it didn't give isn't none. GitHub's first word for why.
pub(crate) fn refused(v: &Value) -> Result<()> {
    let base = |e: &&Value| e["path"].as_array().is_some_and(|p| p.iter().any(|k| k == "baseRef"));
    match v["errors"].as_array().into_iter().flatten().find(|e| !base(e)) {
        Some(e) => bail!("GitHub: {}", e["message"].as_str().unwrap_or("refused")),
        None => Ok(()),
    }
}

/// The pull request's node in GitHub's answer to `pr_query`, weighed (`refused`).
fn pr_node(v: &Value) -> Result<Value> {
    refused(v)?;
    v["data"]["repository"]["pullRequest"].as_object().map(|o| Value::Object(o.clone())).context("GitHub: no pull request in its answer")
}

/// PR `n`'s commits' signatures and its base's rules: one GraphQL query (`pr_node`; a part
/// refused still answers: `gh_input`).
fn pr_query(n: u64) -> Result<Value> {
    let q = format!(
        "query($owner: String!, $name: String!, $n: Int!) {{ repository(owner: $owner, name: $name) {{ pullRequest(number: $n) {{
           {SIGNATURE_RULES} commits(last: 100) {{ totalCount nodes {{ commit {{ oid signature {{ isValid }} }} }} }} }} }} }}"
    );
    let v = gh_json(&["api", "graphql", "-F", "owner={owner}", "-F", "name={repo}", "-F", &format!("n={n}"), "-f", &format!("query={q}")])?;
    pr_node(&v)
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

const INFO_FIELDS: &str = "number,baseRefOid,headRepositoryOwner,isCrossRepository,headRepository,maintainerCanModify,headRefOid,title,author,state,isDraft,url,baseRefName,headRefName,additions,deletions,\
changedFiles,labels,assignees,reviewDecision,reviewRequests,latestReviews,statusCheckRollup,body,id,mergeable";

/// What GitHub couldn't say this time (`new`) but did before (`old`, as dv kept it), kept: its
/// commits' signatures, for the same head (a commit's signature is part of it, so it still
/// holds; only the base's rule may have changed since), as of then; and its activity (what
/// was, still was; only what's new since is missing).
pub fn keep_known(new: &mut Value, old: &Value) {
    if !new["dvCommitsUnknown"].is_null() && old["dvCommits"].as_array().is_some_and(|c| !c.is_empty()) && new["headRefOid"] == old["headRefOid"] {
        for k in ["dvCommits", "dvCommitsTotal", "dvSignaturesRequired", "dvCommitsAt"] {
            new[k] = old[k].clone();
        }
        new["dvCommitsKept"] = true.into();
        if let Some(o) = new.as_object_mut() {
            o.remove("dvCommitsUnknown");
        }
    }
    if !new["dvActivity"]["error"].is_null() && old["dvActivity"]["items"].is_array() {
        new["dvActivity"] = old["dvActivity"].clone();
        new["dvActivity"]["kept"] = true.into();
    }
}

/// The PR's (or `issue`'s) activity, as its page on GitHub lists it: comments, reviews,
/// commits pushed, and what happened to it (labels, reviewers, force-pushes, renames, merged
/// or closed, …), oldest first; and its own reactions. One query for the last 100 (fewer,
/// asked again, if GitHub runs out of time), then those before, 100 at a time. `id`: its node
/// id.
pub fn activity(id: &str, issue: bool) -> Result<Value> {
    // (An issue's timeline has none of a PR's own: asking for them is an error.)
    let (on, types, own) = match issue {
        false => ("PullRequest", PR_TYPES, PR_ACTIVITY),
        true => ("Issue", ISSUE_TYPES, ISSUE_ACTIVITY),
    };
    let q = format!(
        "query($id: ID!, $b: String, $n: Int!) {{ node(id: $id) {{ ... on {on} {{ {REACTED} timelineItems(last: $n, before: $b, itemTypes: [{ACTIVITY_TYPES}, {types}]) {{ pageInfo {{ hasPreviousPage startCursor }} nodes {{ {ACTIVITY} {own} }} }} }} }} }}"
    );
    let page = |b: Option<&str>, n: u32| graphql(&q, json!({ "id": id, "b": b, "n": n }));
    let first = page(None, 100).or_else(|_| page(None, 25))?;
    let pr = &first["node"];
    let mut items = pr["timelineItems"]["nodes"].as_array().cloned().unwrap_or_default();
    let mut at = pr["timelineItems"]["pageInfo"].clone();
    // ponytail: 10 pages (1,000) before the last at most; older than that, left out.
    for _ in 0..10 {
        let Some(before) = at["startCursor"].as_str().filter(|_| at["hasPreviousPage"] == true).map(str::to_owned) else { break };
        let Ok(d) = page(Some(&before), 100) else { break };
        let mut older = d["node"]["timelineItems"]["nodes"].as_array().cloned().unwrap_or_default();
        older.append(&mut items);
        items = older;
        at = d["node"]["timelineItems"]["pageInfo"].clone();
    }
    Ok(json!({ "reactions": pr["reactionGroups"], "items": items }))
}

/// A node's reactions, with which are yours.
const REACTED: &str = "reactionGroups { content viewerHasReacted reactors { totalCount } }";

/// What GitHub's page shows of a PR's or an issue's timeline (not who subscribed or was
/// mentioned, say): what both have, then each one's own.
const ACTIVITY_TYPES: &str = "ISSUE_COMMENT, LABELED_EVENT, UNLABELED_EVENT, ASSIGNED_EVENT, UNASSIGNED_EVENT, \
RENAMED_TITLE_EVENT, CLOSED_EVENT, REOPENED_EVENT, CROSS_REFERENCED_EVENT";
const PR_TYPES: &str = "PULL_REQUEST_REVIEW, PULL_REQUEST_COMMIT, HEAD_REF_FORCE_PUSHED_EVENT, REVIEW_REQUESTED_EVENT, \
REVIEW_REQUEST_REMOVED_EVENT, MERGED_EVENT, READY_FOR_REVIEW_EVENT, CONVERT_TO_DRAFT_EVENT, BASE_REF_CHANGED_EVENT, \
REVIEW_DISMISSED_EVENT";
const ISSUE_TYPES: &str = "MARKED_AS_DUPLICATE_EVENT, CONNECTED_EVENT";

/// Each of those, with what `view::info` says of it.
const ACTIVITY: &str = "__typename ... on Node { id } \
... on IssueComment { id url body createdAt author { login } isMinimized minimizedReason reactionGroups { content viewerHasReacted reactors { totalCount } } } \
... on LabeledEvent { createdAt actor { login } label { name color } } \
... on UnlabeledEvent { createdAt actor { login } label { name color } } \
... on AssignedEvent { createdAt actor { login } assignee { ... on User { login } ... on Bot { login } ... on Mannequin { login } ... on Organization { login } } } \
... on UnassignedEvent { createdAt actor { login } assignee { ... on User { login } ... on Bot { login } ... on Mannequin { login } ... on Organization { login } } } \
... on RenamedTitleEvent { createdAt actor { login } previousTitle currentTitle } \
... on ClosedEvent { createdAt actor { login } stateReason closer { ... on PullRequest { number url } ... on Commit { abbreviatedOid url } } } \
... on ReopenedEvent { createdAt actor { login } } \
... on CrossReferencedEvent { createdAt actor { login } source { ... on Issue { number title url repository { nameWithOwner } } ... on PullRequest { number title url repository { nameWithOwner } } } }";
const PR_ACTIVITY: &str = "\
... on PullRequestReview { id body state submittedAt createdAt author { login } comments { totalCount } commit { oid abbreviatedOid } reactionGroups { content viewerHasReacted reactors { totalCount } } } \
... on PullRequestCommit { commit { abbreviatedOid messageHeadline committedDate author { user { login } name } } } \
... on HeadRefForcePushedEvent { createdAt actor { login } beforeCommit { abbreviatedOid } afterCommit { abbreviatedOid } } \
... on ReviewRequestedEvent { createdAt actor { login } requestedReviewer { ... on User { login } ... on Team { name } } } \
... on ReviewRequestRemovedEvent { createdAt actor { login } requestedReviewer { ... on User { login } ... on Team { name } } } \
... on MergedEvent { createdAt actor { login } commit { abbreviatedOid } mergeRefName } \
... on ReadyForReviewEvent { createdAt actor { login } } \
... on ConvertToDraftEvent { createdAt actor { login } } \
... on BaseRefChangedEvent { createdAt actor { login } previousRefName currentRefName } \
... on ReviewDismissedEvent { createdAt actor { login } }";
const ISSUE_ACTIVITY: &str = "\
... on MarkedAsDuplicateEvent { createdAt actor { login } canonical { ... on Issue { number url repository { nameWithOwner } } ... on PullRequest { number url repository { nameWithOwner } } } } \
... on ConnectedEvent { createdAt actor { login } subject { ... on Issue { number title url repository { nameWithOwner } } ... on PullRequest { number title url repository { nameWithOwner } } } }";

/// Issue `n` as `info_json` has a PR, for the same view to read (`view::info`): its text,
/// state, labels, who has it, the PRs that would close it (`dvFixes`), and its activity.
pub fn issue_json(n: u64) -> Result<Value> {
    let q = "query($owner: String!, $name: String!, $n: Int!) { viewer { login } repository(owner: $owner, name: $name) { issue(number: $n) { \
             __typename id number title url body state stateReason createdAt updatedAt author { login } viewerDidAuthor \
             labels(first: 100) { nodes { name color } } assignees(first: 50) { nodes { login } } \
             closedByPullRequestsReferences(first: 20, includeClosedPrs: true) { nodes { number title state url headRefOid } } } } }";
    let t = gh_json(&["api", "graphql", "-F", "owner={owner}", "-F", "name={repo}", "-F", &format!("n={n}"), "-f", &format!("query={q}")])?;
    let mut v = t["data"]["repository"]["issue"].clone();
    // (GitHub's reason, if it gave one: a PR's number, no access to the repo.)
    anyhow::ensure!(v.is_object(), "#{n}: {}", t["errors"][0]["message"].as_str().unwrap_or("no such issue here"));
    // (Lists as `gh pr view` has them.)
    for k in ["labels", "assignees"] {
        v[k] = v[k]["nodes"].clone();
    }
    v["dvFixes"] = v["closedByPullRequestsReferences"]["nodes"].clone();
    v["dvMe"] = t["data"]["viewer"]["login"].clone();
    let id = v["id"].as_str().unwrap_or("").to_owned();
    v["dvActivity"] = activity(&id, true).unwrap_or_else(|e| json!({ "error": first_line(&format!("{e:#}")) }));
    Ok(v)
}

/// PR info lines, plus the current (title, head commit, url) so a refresh picks up new pushes.
/// GitHub's answer, as JSON (dv caches it; `parse_info` reads it).
pub fn info_json(n: u64) -> Result<Value> {
    let s = |v: &Value| v.as_str().unwrap_or("").to_owned();
    let mut v: Value = gh_json(&["pr", "view", &n.to_string(), "--json", INFO_FIELDS])?;
    // Whether its commits are signed, and whether its base wants them to be (`gh pr view` says
    // neither): with the rest, so the cache has them too.
    // Workflow runs on its head waiting for approval (`gh pr view` doesn't say).
    if let Some(sha) = v["headRefOid"].as_str().map(str::to_owned)
        && let Ok(w) = awaiting_approval(Some(&sha))
    {
        v["dvAwaiting"] = w.get(&sha).map_or(0, Vec::len).into();
    }
    // Its activity (comments, reviews, commits, events), and its commits' signatures, at once.
    let id = s(&v["id"]);
    let (activity, signatures) = std::thread::scope(|sc| {
        let activity = sc.spawn(|| activity(&id, false));
        let signatures = pr_query(n).and_then(|p| signatures_of(&p)).map_err(|e| format!("{e:#}"));
        (activity.join().unwrap_or_else(|p| Err(anyhow::anyhow!(crate::batch::panicked(&p)))), signatures.map_err(|w| first_line(&w)))
    });
    v["dvActivity"] = activity.unwrap_or_else(|e| json!({ "error": first_line(&format!("{e:#}")) }));
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
    // Its checks as GitHub has them, each required or not (`ci::rollups`); else as gh said.
    if let Some(r) = ci::rollups(&[n]).ok().and_then(|mut r| r.remove(&n)) {
        v["statusCheckRollup"] = r.checks.into();
        v["dvRequiredUnknown"] = r.unknown.into();
    }
    Ok(v)
}

/// `s` seconds as "40s", "3m", "2h", "5d".
pub fn ago_secs(s: i64) -> String {
    match s.max(0) {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m", s / 60),
        s if s < 86400 => format!("{}h", s / 3600),
        s => format!("{}d", s / 86400),
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
