//! Reviews written by Claude Code (or anything else) as a JSON file that dv shows inline.
//!
//! The file lives at `<git common dir>/dv/reviews/<pr-N | branch-NAME>.json`, so it's never
//! committed and every worktree of the repo finds it. Format:
//!
//! ```json
//! {
//!   "head": "<commit the review was made against>",
//!   "created": "2026-10-16T12:00:00Z",
//!   "summary": "markdown",
//!   "comments": [
//!     { "path": "src/a.rs", "line": 42, "start_line": 40, "side": "new", "body": "markdown",
//!       "resolved": false,
//!       "replies": [ { "author": "user", "body": "why?" }, { "author": "claude", "body": "…" } ] }
//!   ]
//! }
//! ```
//! `line`/`start_line` are file line numbers on `side` ("new" = the working tree, default;
//! "old" = the base). `start_line` is optional (single-line comment). Replies are a
//! conversation with Claude: dv appends the user's (`r`), Claude appends its answers.
use crate::diff::{self, FileDiff, Kind};
use crate::gh::{Comment, Thread};
use anyhow::{Context, Result};
use serde_json::Value;
use std::path::PathBuf;

/// Author names in the file. The user's replies show as "you".
pub const AUTHOR: &str = "claude";
const USER: &str = "user";

pub struct Review {
    pub head: String,
    pub summary: String,
    pub threads: Vec<Thread>,
}

/// Where the review for PR `pr` (or, without one, the current branch) lives.
pub fn path(pr: Option<u64>) -> Result<PathBuf> {
    let common = diff::git(&["rev-parse", "--path-format=absolute", "--git-common-dir"])?;
    let name = match pr {
        Some(n) => format!("pr-{n}"),
        None => match diff::git(&["branch", "--show-current"])?.trim() {
            "" => "detached".into(),
            b => format!("branch-{}", b.replace('/', "-")),
        },
    };
    Ok(PathBuf::from(common.trim()).join("dv").join("reviews").join(format!("{name}.json")))
}

/// The review at `path`; `None` if there is no file.
pub fn load(path: &std::path::Path) -> Result<Option<Review>> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).context(format!("reading {}", path.display())),
    };
    parse(&text).map(Some).with_context(|| format!("in {}", path.display()))
}

fn parse(text: &str) -> Result<Review> {
    let v: Value = serde_json::from_str(text)?;
    let s = |v: &Value| v.as_str().unwrap_or("").to_owned();
    let created = s(&v["created"]);
    let comments = v["comments"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    let threads = comments
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let line = |k: &str| c[k].as_u64().map(|n| n as u32);
            let path = s(&c["path"]);
            anyhow::ensure!(!path.is_empty(), "comment {} has no \"path\"", i + 1);
            let end = line("line").with_context(|| format!("comment {} has no \"line\"", i + 1))?;
            Ok(Thread {
                id: String::new(),
                path,
                left: c["side"] == "old",
                line: Some(end),
                start_line: line("start_line").filter(|&s| s < end),
                original_line: Some(end),
                resolved: c["resolved"] == true,
                outdated: false,
                hunk: vec![],
                claude: Some(i),
                comments: std::iter::once((AUTHOR.to_owned(), s(&c["body"]), created.clone()))
                    .chain(c["replies"].as_array().into_iter().flatten().map(|r| {
                        let who = s(&r["author"]);
                        (if who == USER { "you".into() } else { who }, s(&r["body"]), s(&r["created"]))
                    }))
                    .enumerate()
                    .map(|(k, (author, body, created))| Comment {
                        // Stable across reloads (folding state is keyed by the root's), never a GitHub id.
                        id: u64::MAX - ((i as u64) << 16) - k as u64,
                        node_id: String::new(),
                        can_edit: false,
                        can_delete: false,
                        author,
                        body,
                        created,
                        pending: false,
                    })
                    .collect(),
            })
        })
        .collect::<Result<_>>()?;
    Ok(Review { head: s(&v["head"]), summary: s(&v["summary"]), threads })
}

/// Change the file's `comments` with `f`, keeping everything else in it as written.
fn update(path: &std::path::Path, i: usize, f: impl FnOnce(&mut Vec<Value>)) -> Result<()> {
    let mut v: Value = serde_json::from_str(&std::fs::read_to_string(path)?)?;
    let comments = v["comments"].as_array_mut().context("no comments")?;
    anyhow::ensure!(i < comments.len(), "that comment is no longer in the file");
    f(comments);
    std::fs::write(path, serde_json::to_string_pretty(&v)? + "\n")?;
    Ok(())
}

/// Remove comment `i`.
pub fn dismiss(path: &std::path::Path, i: usize) -> Result<()> {
    update(path, i, |c| _ = c.remove(i))
}

/// Add the user's reply to comment `i`, for Claude to answer.
pub fn reply(path: &std::path::Path, i: usize, body: &str) -> Result<()> {
    update(path, i, |c| {
        if !c[i]["replies"].is_array() {
            c[i]["replies"] = serde_json::json!([]);
        }
        let r = serde_json::json!({ "author": USER, "body": body, "created": now() });
        if let Some(a) = c[i]["replies"].as_array_mut() {
            a.push(r);
        }
    })
}

pub fn set_resolved(path: &std::path::Path, i: usize, on: bool) -> Result<()> {
    update(path, i, |c| c[i]["resolved"] = Value::Bool(on))
}

/// The time now as ISO 8601 (UTC), e.g. 2026-10-16T12:00:00Z.
fn now() -> String {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    let (days, secs) = (t.div_euclid(86400), t.rem_euclid(86400));
    // Civil date from days since 1970 (Howard Hinnant's algorithm).
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + (m <= 2) as i64;
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", secs / 3600, secs % 3600 / 60, secs % 60)
}

/// Problems with the review against the diff: errors (dv can't place the comment) and
/// warnings (dv can, but GitHub won't take it as a line comment).
pub fn check(r: &Review, files: &[FileDiff]) -> (Vec<String>, Vec<String>) {
    let (mut errors, mut warnings) = (vec![], vec![]);
    for (i, t) in r.threads.iter().enumerate() {
        let at = format!("comment {} ({}:{})", i + 1, t.path, t.line.unwrap_or(0));
        let Some(f) = files.iter().find(|f| f.path == t.path) else {
            errors.push(format!("{at}: {} isn't in the diff", t.path));
            continue;
        };
        let side = |l: &diff::Line| match t.left {
            true => matches!(l.kind, Kind::Ctx | Kind::Del),
            false => matches!(l.kind, Kind::Ctx | Kind::Add),
        };
        let no = |l: &diff::Line| if t.left { l.old } else { l.new };
        for n in t.start_line.into_iter().chain(t.line) {
            let Some(i) = f.full.iter().position(|l| side(l) && no(l) == Some(n)) else {
                errors.push(format!("{at}: line {n} doesn't exist on the {} side", if t.left { "old" } else { "new" }));
                continue;
            };
            // GitHub takes line comments within its hunks: 3 lines of context around a change.
            let near = &f.full[i.saturating_sub(3)..(i + 4).min(f.full.len())];
            if !near.iter().any(|l| matches!(l.kind, Kind::Add | Kind::Del)) {
                warnings.push(format!("{at}: line {n} is outside the diff's hunks, so it can't become a GitHub comment"));
            }
        }
    }
    (errors, warnings)
}

#[test]
fn parse_and_dismiss() {
    let text = r#"{ "head": "abc", "summary": "ok", "comments": [
        { "path": "src/a.rs", "line": 3, "start_line": 2, "body": "x" },
        { "path": "src/a.rs", "line": 1, "side": "old", "body": "y", "extra": true } ] }"#;
    let r = parse(text).unwrap();
    assert_eq!((r.head.as_str(), r.summary.as_str(), r.threads.len()), ("abc", "ok", 2));
    assert_eq!((r.threads[0].start_line, r.threads[0].line, r.threads[0].left), (Some(2), Some(3), false));
    assert!(r.threads[1].left);
    assert!(parse(r#"{ "comments": [ { "line": 1 } ] }"#).is_err()); // no path
    let p = std::env::temp_dir().join(format!("dv-claude-test-{}.json", std::process::id()));
    std::fs::write(&p, text).unwrap();
    reply(&p, 1, "why?").unwrap();
    set_resolved(&p, 1, true).unwrap();
    let r = load(&p).unwrap().unwrap();
    let t = &r.threads[1];
    assert!(t.resolved);
    assert_eq!((t.comments[1].author.as_str(), t.comments[1].body.as_str()), ("you", "why?"));
    assert_eq!(t.comments[1].created.len(), "2026-10-16T12:00:00Z".len());
    dismiss(&p, 0).unwrap();
    let left = std::fs::read_to_string(&p).unwrap();
    _ = std::fs::remove_file(&p);
    assert!(!left.contains("\"x\"") && left.contains("\"extra\": true"));
}
