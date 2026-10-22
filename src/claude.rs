//! Reviews written by Claude Code (or anything else) as a JSON file that dv shows inline.
//!
//! The file lives in dv's directory for the repo (`dv_dir`: `~/.local/share/dv/<repo>/`), at
//! `reviews/<pr-N | branch-NAME>.json`: outside the repo, so it's never committed, and the same
//! for every worktree of it. (Not in `.git`: Claude Code won't edit anything in there.) Format:
//!
//! ```json
//! {
//!   "head": "<commit the review was made against>",
//!   "created": "2026-10-16T12:00:00Z",
//!   "summary": "markdown",
//!   "comments": [
//!     { "path": "src/a.rs", "line": 42, "start_line": 40, "side": "new", "severity": "nit", "body": "markdown",
//!       "resolved": false,
//!       "talk_resolved": false,
//!       "replies": [ { "author": "user", "body": "why?" }, { "author": "claude", "body": "…" } ] }
//!   ]
//! }
//! ```
//! `line`/`start_line` are file line numbers on `side` ("new" = the working tree, default;
//! "old" = the base). `start_line` is optional (single-line comment). Replies are a
//! conversation with Claude: dv appends the user's (`r`), Claude appends its answers. A comment
//! with `"author": "user"` is one the user left for Claude (`a` in dv), which Claude answers.
//! A top-level `"description"` is a new description Claude proposes for the PR (when asked):
//! dv shows it against the current one, for the user to approve (it's posted then) or not.
//! A top-level `"closing"` is a comment Claude drafts for closing the PR (when asked): the user
//! edits it and closes the PR with it in dv, or not.
//! One with `"conversation": "<url>"` instead of `path`/`line` asks about a comment in the PR's
//! conversation (that URL's): it's on no lines, and dv shows it under that comment; with
//! `"conversation": "summary"`, it replies to the review's summary, and shows under that.
//!
//! `id` names a comment for good (`c1`, `c2`, …), so replies and dismissals reach the right one
//! however the list is edited; dv and `dv review-check` give one to any comment without.
//! Both write the file whole, via a rename, so a reader never sees half of it.
//!
//! `code` is the text of the commented lines, recorded by `dv review-check`. It keeps comments
//! on their code as the code changes: dv finds those lines again where they moved, and shows a
//! comment as outdated when they're gone.
use crate::diff::{self, FileDiff, Kind};
use crate::gh::{Comment, Thread};
use anyhow::{Context, Result};
use serde_json::Value;
use std::path::PathBuf;

mod run;
pub(crate) use run::*;

impl Thread {
    /// Claude's own comment (from its review file), not one you left for it.
    pub fn by_claude(&self) -> bool {
        self.claude.is_some() && self.comments.first().is_some_and(|c| c.author == AUTHOR)
    }

    /// A thread of the review file still being talked over: not done, its talk not resolved
    /// (settled, Claude's comment as it now stands).
    fn talking(&self) -> bool {
        self.claude.is_some() && !self.resolved && !self.talk_resolved
    }

    /// One being talked over where you had the last word: Claude's turn.
    pub fn waiting(&self) -> bool {
        self.talking() && self.comments.last().is_some_and(|c| c.author == "you")
    }

    /// One being talked over where Claude has answered you: your turn.
    pub fn answered(&self) -> bool {
        self.talking() && !self.waiting() && self.comments.iter().any(|c| c.author == "you")
    }
}

/// Author names in the file. The user's replies show as "you".
pub const AUTHOR: &str = "claude";
const USER: &str = "user";

#[derive(Default)]
pub struct Review {
    pub head: String,
    /// When Claude wrote it (its file's `"created"`).
    pub created: String,
    pub summary: String,
    /// A new description Claude proposes for the PR (empty: none), for the user to approve.
    pub description: String,
    /// A comment Claude drafted for closing the PR (empty: none), for the user to close it with.
    pub closing: String,
    /// Why Claude would close the PR (its premise is wrong, a competing PR is better); empty:
    /// it wouldn't. `dv list`'s "can close" tab.
    pub close: String,
    /// Why Claude would replace it with a PR of its own (low effort, AI slop, but the idea's
    /// worth keeping); empty: it wouldn't. Auto drafts that replacement.
    pub replace: String,
    pub threads: Vec<Thread>,
}

/// Claude's review `r` as text for a prompt (a replacement's: what was found wrong): its
/// summary, then each thread with its replies (yours and Claude's), where it is: on code, on
/// the summary, on a conversation comment.
pub fn review_brief(r: &Review) -> String {
    let mut out = vec![r.summary.trim().to_owned()];
    for t in &r.threads {
        let at = match t.conversation.as_str() {
            "" => match t.own_lines() {
                Some((Some(s), l)) => format!("`{}:{s}-{l}`", t.path),
                Some((None, l)) => format!("`{}:{l}`", t.path),
                None => format!("`{}`", t.path),
            },
            "summary" => "on the summary".into(),
            url => format!("on {url}"),
        };
        let state = [(t.resolved, "resolved"), (t.outdated, "outdated")].iter().filter(|s| s.0).map(|s| s.1).collect::<Vec<_>>().join(", ");
        let tag = [t.severity.as_str(), state.as_str()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(", ");
        let tag = if tag.is_empty() { String::new() } else { format!(" ({tag})") };
        let talk: Vec<String> = t.comments.iter().map(|c| format!("  {}: {}", if c.author == AUTHOR { "Claude" } else { "the user" }, c.body.trim().replace('\n', "\n  "))).collect();
        out.push(format!("- {at}{tag}\n{}", talk.join("\n")));
    }
    out.retain(|s| !s.is_empty());
    out.join("\n\n")
}

/// dv's directory for this repo: `$XDG_DATA_HOME/dv/<repo>/` (`~/.local/share/dv/...`), named
/// after the main checkout's path so its worktrees share it. Moves an older `.git/dv` here the
/// first time.
pub fn dv_dir() -> Result<PathBuf> {
    // The repo's git dir, by working directory: asked of git once each (dv list wants this
    // directory many times a second, and a git run each time added up).
    static COMMON: std::sync::Mutex<Vec<(PathBuf, PathBuf)>> = std::sync::Mutex::new(Vec::new());
    let cwd = std::env::current_dir()?;
    let known = COMMON.lock().ok().and_then(|c| c.iter().find(|k| k.0 == cwd).map(|k| k.1.clone()));
    let common = match known {
        Some(c) => c,
        None => {
            let c = PathBuf::from(diff::git(&["rev-parse", "--path-format=absolute", "--git-common-dir"])?.trim());
            if let Ok(mut k) = COMMON.lock() {
                k.push((cwd, c.clone()));
            }
            c
        }
    };
    let dir = data_home()?.join(repo_key(&common));
    let old = common.join("dv");
    if old.is_dir() && !dir.exists() {
        std::fs::create_dir_all(dir.parent().unwrap_or(&dir))?;
        if std::fs::rename(&old, &dir).is_err() {
            copy_dir(&old, &dir)?; // another filesystem: copy, and leave the old one be
        }
    }
    Ok(dir)
}

/// `$XDG_DATA_HOME/dv` (`~/.local/share/dv`): every repo's directory, and what isn't any one's.
pub fn data_home() -> Result<PathBuf> {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .context("no $HOME")?;
    Ok(data.join("dv"))
}

/// A directory name for the repo whose git dir is `common`: its main checkout's path, with `/`
/// as `-` (as Claude Code names its project directories).
// ponytail: "/a-b/c" and "/a/b-c" would share a name; a hash suffix if that ever bites.
fn repo_key(common: &std::path::Path) -> String {
    let main = if common.file_name().is_some_and(|f| f == ".git") { common.parent().unwrap_or(common) } else { common };
    main.to_string_lossy().trim_start_matches('/').replace(['/', '\\', ':'], "-")
}

fn copy_dir(from: &std::path::Path, to: &std::path::Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        match e.file_type()?.is_dir() {
            true => copy_dir(&e.path(), &to.join(e.file_name()))?,
            false => _ = std::fs::copy(e.path(), to.join(e.file_name()))?,
        }
    }
    Ok(())
}

/// Where the review for PR `pr` (or, without one, the current branch) lives.
pub fn path(pr: Option<u64>) -> Result<PathBuf> {
    let name = match pr {
        Some(n) => return Subject::Pr(n).path(),
        None => match diff::git(&["branch", "--show-current"])?.trim() {
            "" => "detached".into(),
            b => format!("branch-{}", b.replace('/', "-")),
        },
    };
    Ok(dv_dir()?.join("reviews").join(format!("{name}.json")))
}

/// What a review file is of, on GitHub: a PR, or an issue (Claude's triage of it, and the
/// talk about it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Subject {
    Pr(u64),
    Issue(u64),
}

impl Subject {
    pub fn n(self) -> u64 {
        match self {
            Subject::Pr(n) | Subject::Issue(n) => n,
        }
    }

    /// "PR #7", "issue #7".
    pub fn what(self) -> String {
        match self {
            Subject::Pr(n) => format!("PR #{n}"),
            Subject::Issue(n) => format!("issue #{n}"),
        }
    }

    /// Its review file.
    pub fn path(self) -> Result<PathBuf> {
        let name = match self {
            Subject::Pr(n) => format!("pr-{n}"),
            Subject::Issue(n) => format!("issue-{n}"),
        };
        Ok(dv_dir()?.join("reviews").join(format!("{name}.json")))
    }

    /// In the environment of a Claude dv runs on it (`PR_ENV`, `ISSUE_ENV`).
    pub fn env(self) -> (&'static str, String) {
        match self {
            Subject::Pr(n) => (PR_ENV, n.to_string()),
            Subject::Issue(n) => (ISSUE_ENV, n.to_string()),
        }
    }
}

/// Claude's word that PR `n`'s failing CI isn't the PR's doing (a flaky test, CI itself, the
/// base): `ci/pr-N.json` in dv's directory, which Claude writes. At which head it looked, why,
/// and what a PR of its own fixing CI would do (empty: nothing in the repo would).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CiNote {
    pub head: String,
    pub why: String,
    pub task: String,
    /// Claude reran the failed jobs (once a head: a transient failure, it thought).
    pub rerun: bool,
}

pub fn ci_note_path(n: u64) -> Result<PathBuf> {
    Ok(dv_dir()?.join("ci").join(format!("pr-{n}.json")))
}

pub fn ci_note_at(p: &std::path::Path) -> Option<CiNote> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()?;
    let s = |k: &str| v[k].as_str().unwrap_or("").trim().to_owned();
    Some(CiNote { head: s("head"), why: s("why"), task: s("task"), rerun: v["rerun"] == true })
}

/// PR `n`'s note, if Claude wrote it at `head` (the PR's head now): older ones are about
/// other commits.
pub fn ci_note(n: u64, head: &str) -> Option<CiNote> {
    ci_note_at(&ci_note_path(n).ok()?).filter(|c| !head.is_empty() && c.head == head)
}

/// A PR fixing CI is drafted from PR `n`'s note: its task goes, so it isn't offered again.
pub fn ci_fix_drafted(n: u64) {
    let Ok(p) = ci_note_path(n) else { return };
    let Some(mut v) = std::fs::read_to_string(&p).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) else { return };
    v["task"] = "".into();
    _ = std::fs::write(&p, v.to_string());
}

impl CiNote {
    /// What to ask for in a new PR fixing CI, from this note on PR `n`.
    pub fn prefill(&self, n: u64) -> String {
        format!("Fix CI: it fails on #{n}, not through #{n}'s doing ({}).\n\n{}\n", self.why, self.task)
    }
}

/// Notes Claude keeps about the repo across reviews (conventions, recurring issues, the
/// user's review preferences): `notes.md` in dv's directory, shared by all worktrees.
pub fn notes_path() -> Result<PathBuf> {
    Ok(dv_dir()?.join("notes.md"))
}

/// How big the notes may grow: every run reads them all, so past this they're skimmed (about
/// 3k tokens). And a point's longest.
const NOTES_BUDGET: usize = 12_000;
const NOTE_POINT: usize = 240;

/// What `dv review-check` says when the notes have outgrown their budget, or have points
/// that run long: the run that grew them condenses them.
pub fn notes_over(notes: &str) -> Option<String> {
    let long = notes.lines().filter(|l| l.chars().count() > NOTE_POINT).count();
    let mut over = vec![];
    if notes.len() > NOTES_BUDGET {
        over.push(format!("{} KB, over their {} KB budget", notes.len().div_ceil(1000), NOTES_BUDGET / 1000));
    }
    if long > 0 {
        over.push(format!("{long} point(s) longer than {NOTE_POINT} characters"));
    }
    (!over.is_empty()).then(|| {
        format!(
            "your notes on the repo have {}: condense them before you finish (merge overlapping points, drop what's stale or about one PR)",
            over.join(" and ")
        )
    })
}

/// When you last left PR `n` in dv (seconds since the epoch), for `dv list`'s `[new]`:
/// `seen/pr-N.looked` in dv's directory.
pub fn looked(n: u64) -> Option<i64> {
    std::fs::read_to_string(dv_dir().ok()?.join(format!("seen/pr-{n}.looked"))).ok()?.trim().parse().ok()
}

/// Claude at work on PR `n`, for `dv list` (any dv's run): a marker in dv's directory,
/// `running/pr-N.json`, with what it's doing and how to tell it's still at it: its process, or
/// its zellij pane (by the session in its command line).
pub fn mark_running(n: u64, what: &str, pid: Option<u32>, session: Option<&str>) {
    let Ok(dir) = dv_dir().map(|d| d.join("running")) else { return };
    _ = std::fs::create_dir_all(&dir);
    let v = serde_json::json!({ "what": what, "pid": pid, "session": session, "started": now_secs() });
    _ = std::fs::write(dir.join(format!("pr-{n}.json")), v.to_string());
}

/// Claude's done with PR `n` (or stopped).
pub fn clear_running(n: u64) {
    if let Ok(d) = dv_dir() {
        _ = std::fs::remove_file(d.join("running").join(format!("pr-{n}.json")));
    }
}

/// What Claude is doing, by PR, from the markers in `dir` (`running/`): only those still at it
/// (`alive` says, for a process or a pane's session).
pub fn running(dir: &std::path::Path, alive: impl Fn(Option<u32>, Option<&str>) -> bool) -> std::collections::HashMap<u64, String> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Default::default() };
    entries
        .flatten()
        .filter_map(|e| {
            let n: u64 = e.file_name().to_str()?.strip_prefix("pr-")?.strip_suffix(".json")?.parse().ok()?;
            let v: Value = serde_json::from_str(&std::fs::read_to_string(e.path()).ok()?).ok()?;
            let pid = v["pid"].as_u64().map(|p| p as u32);
            alive(pid, v["session"].as_str()).then(|| (n, v["what"].as_str().unwrap_or("working").to_owned()))
        })
        .collect()
}

/// Seconds since the epoch, now.
pub fn now_secs() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// You're leaving PR `n`: what's on it now has been seen.
pub fn set_looked(n: u64) -> Result<()> {
    let p = dv_dir()?.join(format!("seen/pr-{n}.looked"));
    std::fs::create_dir_all(p.parent().unwrap_or(&p))?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    Ok(std::fs::write(p, now.to_string())?)
}

/// When Claude's review of PR `n` last changed (seconds since the epoch), if it has one.
pub fn review_changed(n: u64) -> Option<i64> {
    let t = std::fs::metadata(path(Some(n)).ok()?).ok()?.modified().ok()?;
    Some(t.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs() as i64)
}

/// Your last instructions to Claude for `kind` of task in this repo ("review", "replace"), to
/// start from next time (empty if none): `<kind>-instructions.md` in dv's directory.
pub fn instructions(kind: &str) -> String {
    dv_dir().and_then(|d| Ok(std::fs::read_to_string(d.join(format!("{kind}-instructions.md")))?)).unwrap_or_default()
}

/// Remember `text` as those instructions (empty forgets them).
pub fn save_instructions(kind: &str, text: &str) -> Result<()> {
    let p = dv_dir()?.join(format!("{kind}-instructions.md"));
    match text.trim() {
        "" => _ = std::fs::remove_file(&p),
        t => {
            std::fs::create_dir_all(p.parent().unwrap_or(&p))?;
            std::fs::write(&p, format!("{t}\n"))?;
        }
    }
    Ok(())
}

/// Where the Claude Code sessions for a review are kept: next to its file (`pr-N.session`),
/// one line per checkout directory (`<dir>\t<id>`), since Claude Code keeps sessions per
/// directory: each checkout of the PR continues its own conversation about it.
pub fn session_path(review: &std::path::Path) -> PathBuf {
    review.with_extension("session")
}

/// Claude's review at `from` (and its session) moved to `to`: a branch's, once it's a PR (a
/// replacement published). Not over one that's there.
pub fn move_review(from: &std::path::Path, to: &std::path::Path) -> Result<()> {
    if from.exists() && !to.exists() {
        std::fs::rename(from, to)?;
        if session_path(from).exists() {
            std::fs::rename(session_path(from), session_path(to))?;
        }
    }
    Ok(())
}

/// The session for `review` in checkout `dir`, if Claude Code still has it.
/// The dv-review skill this dv was built with (`dv install-skill` installs it).
pub const SKILL: &str = include_str!("../skills/dv-review/SKILL.md");

/// Where Claude Code finds the skill.
pub fn skill_path() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").context("no $HOME")?;
    Ok(PathBuf::from(home).join(".claude/skills/dv-review/SKILL.md"))
}

/// The skill as dv installs it: marked with the dv that did, so an older dv leaves it alone.
pub fn skill_file() -> String {
    format!("{SKILL}\n<!-- installed by dv {} -->\n", env!("CARGO_PKG_VERSION"))
}

/// Bring an installed skill up to this dv's (not installing one that isn't); whether it did.
/// Not one a newer dv installed: with two dvs about (an installed one and a build), they'd
/// take turns.
pub fn update_skill() -> bool {
    let Ok(p) = skill_path() else { return false };
    let Ok(old) = std::fs::read_to_string(&p) else { return false };
    old != skill_file() && !newer(installed_by(&old), env!("CARGO_PKG_VERSION")) && std::fs::write(&p, skill_file()).is_ok()
}

/// The version of dv that installed a skill (`0.0.0` if it doesn't say: older than any).
fn installed_by(skill: &str) -> &str {
    skill.lines().rev().find_map(|l| l.strip_prefix("<!-- installed by dv ")?.strip_suffix(" -->")).unwrap_or("0.0.0")
}

/// Whether version `a` is newer than `b` (x.y.z).
fn newer(a: &str, b: &str) -> bool {
    let v = |s: &str| s.split('.').map(|n| n.parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
    v(a) > v(b)
}

/// Which skill a session started with: Claude Code reads a skill once per session, so one
/// that started with another keeps following that.
fn skill_tag() -> String {
    // FNV-1a: stable across builds, unlike std's hasher.
    let h = SKILL.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
    format!("{h:016x}")
}

/// What resumes the PR's Claude session in checkout `dir` (`review` is the PR's review file):
/// `--resume <id>` when there's one, of this skill, that Claude Code still has; else a new
/// one's `--session-id`, saved as the PR's.
pub fn session_args(review: &std::path::Path, dir: &std::path::Path) -> Result<[String; 2]> {
    match load_session(review, dir) {
        Some((id, true)) if session_exists(&id) => Ok(["--resume".into(), id]),
        _ => {
            let id = new_session_id();
            save_session(review, dir, Some(&id))?;
            Ok(["--session-id".into(), id])
        }
    }
}

/// The session for `review` in checkout `dir`, and whether it started with this dv's skill.
pub fn load_session(review: &std::path::Path, dir: &std::path::Path) -> Option<(String, bool)> {
    let text = std::fs::read_to_string(session_path(review)).ok()?;
    let (id, same) = session_line(&text, &dir.to_string_lossy())?;
    session_exists(&id).then_some((id, same))
}

/// `dir`'s line of a session file: "<dir>\t<id>\t<skill tag>" (dv 0.2 wrote no tag).
fn session_line(text: &str, dir: &str) -> Option<(String, bool)> {
    text.lines().find_map(|l| {
        let mut f = l.split('\t');
        (f.next()? == dir).then(|| (f.next().unwrap_or("").trim().to_owned(), f.next().is_some_and(|t| t.trim() == skill_tag())))
    })
}

/// Remember (`Some`) or forget the session for `review` in checkout `dir`.
pub fn save_session(review: &std::path::Path, dir: &std::path::Path, id: Option<&str>) -> Result<()> {
    let p = session_path(review);
    let key = dir.to_string_lossy();
    let text = std::fs::read_to_string(&p).unwrap_or_default();
    let mut lines: Vec<String> = text.lines().filter(|l| l.split_once('\t').is_none_or(|(d, _)| d != key)).map(str::to_owned).collect();
    lines.extend(id.map(|id| format!("{key}\t{id}\t{}", skill_tag())));
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    match lines.is_empty() {
        true => _ = std::fs::remove_file(&p),
        false => std::fs::write(&p, lines.join("\n") + "\n")?,
    }
    Ok(())
}

/// Whether Claude Code has session `id` (it keeps each as `projects/<dir>/<id>.jsonl` under its
/// config dir), so dv doesn't try to resume one that never started or was cleaned up.
pub fn session_exists(id: &str) -> bool {
    transcript(id).is_some()
}

/// Claude Code's transcript of session `id` (`~/.claude/projects/<dir>/<id>.jsonl`).
fn transcript(id: &str) -> Option<PathBuf> {
    let config = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".claude")))?;
    let dirs = std::fs::read_dir(config.join("projects")).ok()?;
    dirs.flatten().map(|d| d.path().join(format!("{id}.jsonl"))).find(|p| p.exists())
}

/// `turn_ended`'s answers by session: (the transcript's length, since, the answer).
type Ended = std::collections::HashMap<String, (u64, i64, Option<String>)>;

/// Whether Claude has ended its turn in session `id`, after `since` (seconds), by its
/// transcript: its last message its answer (`end_turn`), not a tool call or a result it's
/// working from. Its last words then.
pub fn turn_ended(id: &str, since: i64) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    // As last read, while it hasn't grown (a pane left open, done, is looked at every few
    // seconds).
    static LAST: std::sync::Mutex<Option<Ended>> = std::sync::Mutex::new(None);
    let path = transcript(id)?;
    let len = std::fs::metadata(&path).ok()?.len();
    if let Ok(last) = LAST.lock()
        && let Some((l, s, r)) = last.as_ref().and_then(|m| m.get(id))
        && (*l, *s) == (len, since)
    {
        return r.clone();
    }
    let mut f = std::fs::File::open(&path).ok()?;
    f.seek(SeekFrom::Start(len.saturating_sub(512 * 1024))).ok()?;
    let mut tail = vec![];
    f.read_to_end(&mut tail).ok()?;
    // Bytes: the cut may land inside a character (its first line, cut, isn't JSON anyway).
    let r = turn_ended_in(&String::from_utf8_lossy(&tail), since);
    if let Ok(mut last) = LAST.lock() {
        last.get_or_insert_default().insert(id.to_owned(), (len, since, r.clone()));
    }
    r
}

fn turn_ended_in(tail: &str, since: i64) -> Option<String> {
    // Its messages (not the bookkeeping lines between, nor subagents'), the last ones.
    let msgs: Vec<Value> = tail
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|v| matches!(v["type"].as_str(), Some("user" | "assistant")) && v["isSidechain"] != true)
        .collect();
    let last = msgs.last()?;
    let ended = last["type"] == "assistant" && last["message"]["stop_reason"] == "end_turn";
    let at = crate::gh::epoch_secs(last["timestamp"].as_str().unwrap_or(""))?;
    if !ended || at <= since {
        return None;
    }
    // Its answer: the text of the turn's last messages.
    let text: Vec<&str> = msgs
        .iter()
        .rev()
        .take_while(|v| v["type"] == "assistant" && v["message"]["stop_reason"] == "end_turn")
        .flat_map(|v| v["message"]["content"].as_array().into_iter().flatten())
        .filter(|c| c["type"] == "text")
        .filter_map(|c| c["text"].as_str())
        .collect();
    Some(text.into_iter().rev().collect::<Vec<_>>().join("\n"))
}

/// PR `n`'s run in a pane, from its marker (`mark_running`): its session, and when it started.
pub fn pane_run(n: u64) -> Option<(String, i64)> {
    pane_run_in(&dv_dir().ok()?.join("running").join(format!("pr-{n}.json")))
}

fn pane_run_in(marker: &std::path::Path) -> Option<(String, i64)> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(marker).ok()?).ok()?;
    Some((v["session"].as_str()?.to_owned(), v["started"].as_i64().unwrap_or(0)))
}

/// Every run in a pane, from the markers in `dir` (`running/`): (PR, session, started).
pub fn pane_runs(dir: &std::path::Path) -> Vec<(u64, String, i64)> {
    let Ok(entries) = std::fs::read_dir(dir) else { return vec![] };
    entries
        .flatten()
        .filter_map(|e| {
            let n = e.file_name().to_str()?.strip_prefix("pr-")?.strip_suffix(".json")?.parse().ok()?;
            let (s, t) = pane_run_in(&e.path())?;
            Some((n, s, t))
        })
        .collect()
}

/// The text of lines `first..=last` on one side of `f`, or `None` if any of them doesn't exist
/// there. What a comment's `code` records, so it can follow those lines.
pub fn code_at(f: &FileDiff, left: bool, first: u32, last: u32) -> Option<Vec<String>> {
    let lines = side_lines(f, left);
    (first..=last).map(|n| lines.iter().find(|l| l.0 == n).map(|l| l.1.to_owned())).collect()
}

/// A fresh session id (a random UUID v4), for `claude --session-id`.
pub fn new_session_id() -> String {
    use std::io::Read;
    let mut b = [0u8; 16];
    if std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut b)).is_err() {
        // ponytail: no /dev/urandom (not Linux/macOS): time + pid, unique enough for one machine.
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        b = (t ^ ((std::process::id() as u128) << 64)).to_le_bytes();
    }
    (b[6], b[8]) = (b[6] & 0x0f | 0x40, b[8] & 0x3f | 0x80); // version 4, RFC 4122 variant
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &h[..8], &h[8..12], &h[12..16], &h[16..20], &h[20..])
}

/// The review at `path`, giving any comment without an id the one it'll get when it's next
/// written (so dv's folding and cursor, keyed by ids, don't jump then); `None` if there is no
/// file. Only in memory: writing here would race Claude, who is likely mid-edit right then.
pub fn load_with_ids(path: &std::path::Path) -> Result<Option<Review>> {
    let Some(text) = read(path)? else { return Ok(None) };
    let mut v: Value = serde_json::from_str(&text).with_context(|| format!("in {}", path.display()))?;
    if v["comments"].as_array_mut().is_some_and(|c| assign_ids(c)) {
        return parse(&v.to_string()).map(Some).with_context(|| format!("in {}", path.display()));
    }
    parse(&text).map(Some).with_context(|| format!("in {}", path.display()))
}

fn read(path: &std::path::Path) -> Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(t) => Ok(Some(t)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).context(format!("reading {}", path.display())),
    }
}

/// The review at `path`; `None` if there is no file.
pub fn load(path: &std::path::Path) -> Result<Option<Review>> {
    let Some(text) = read(path)? else { return Ok(None) };
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
            let conversation = s(&c["conversation"]);
            anyhow::ensure!(!path.is_empty() || !conversation.is_empty(), "comment {} has no \"path\"", i + 1);
            let end = match conversation.is_empty() {
                true => Some(line("line").with_context(|| format!("comment {} has no \"line\"", i + 1))?),
                false => None, // on no lines: under the conversation comment it's about
            };
            Ok(Thread {
                id: String::new(),
                path,
                left: c["side"] == "old",
                line: end,
                start_line: line("start_line").filter(|&s| end.is_some_and(|e| s < e)),
                start_left: None,
                original_line: end,
                conversation,
                resolved: c["resolved"] == true,
                talk_resolved: c["talk_resolved"] == true,
                outdated: false,
                // The recorded code, diff-style (+ new side, - old), as outdated threads show it.
                hunk: c["code"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|l| format!("{}{}", if c["side"] == "old" { '-' } else { '+' }, l.as_str().unwrap_or("")))
                    .collect(),
                // Without an id yet: its position, until dv or review-check gives it one.
                claude: Some(c["id"].as_str().map_or(format!("#{i}"), str::to_owned)),
                severity: s(&c["severity"]).to_lowercase(),
                about: about_id(&c["about"]),
                // The user's last message's: what they want from Claude now.
                want: std::iter::once(c)
                    .chain(c["replies"].as_array().into_iter().flatten())
                    .rfind(|m| m["author"] == USER && m["want"].as_str().is_some_and(|w| !w.is_empty()))
                    .map_or(String::new(), |m| s(&m["want"])),
                // A comment's own time (the user's have one), else the review's.
                comments: std::iter::once((root_author(c), s(&c["body"]), Some(s(&c["created"])).filter(|t| !t.is_empty()).unwrap_or(created.clone())))
                    .chain(c["replies"].as_array().into_iter().flatten().map(|r| {
                        let who = s(&r["author"]);
                        (if who == USER { "you".into() } else { who }, s(&r["body"]), s(&r["created"]))
                    }))
                    .enumerate()
                    .map(|(k, (author, body, created))| Comment {
                        // Stable across reloads (folding state is keyed by the root's).
                        id: comment_id(c["id"].as_str().unwrap_or(&format!("#{i}")), k),
                        node_id: String::new(),
                        can_edit: false,
                        can_delete: false,
                        author,
                        body,
                        created,
                        pending: false,
                        reactions: vec![], hidden: String::new(),
                    })
                    .collect(),
            })
        })
        .collect::<Result<_>>()?;
    Ok(Review { head: s(&v["head"]), created: created.clone(), summary: s(&v["summary"]), description: s(&v["description"]), closing: s(&v["closing"]), close: s(&v["close"]), replace: s(&v["replace"]), threads })
}

/// A GitHub review comment's id, from a review file's `"about"`: the id, or its link
/// (`…#discussion_r123`).
fn about_id(v: &Value) -> Option<u64> {
    v.as_u64().or_else(|| v.as_str()?.rsplit("discussion_r").next()?.trim().parse().ok())
}

/// A number for comment `k` of the thread `key`, as dv's folding state keys threads. The top bit
/// keeps it clear of GitHub's ids.
fn comment_id(key: &str, k: usize) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (key, k).hash(&mut h);
    h.finish() | 1 << 63
}

/// Give every comment without an `id` the next free `c<n>`. Whether any changed.
fn assign_ids(comments: &mut [Value]) -> bool {
    let num = |c: &Value| c["id"].as_str().and_then(|s| s.strip_prefix('c')?.parse::<u64>().ok());
    let mut next = comments.iter().filter_map(num).max().unwrap_or(0) + 1;
    let mut changed = false;
    for c in comments.iter_mut().filter(|c| !c["id"].is_string()) {
        c["id"] = format!("c{next}").into();
        (next, changed) = (next + 1, true);
    }
    changed
}

/// Replace the file in one step (write a temp file, rename it over), so dv and Claude never
/// read a half-written review.
pub(crate) fn write(path: &std::path::Path, v: &Value) -> Result<()> {
    let tmp = path.with_extension(format!("json.{}.tmp", std::process::id()));
    std::fs::write(&tmp, serde_json::to_string_pretty(v)? + "\n")?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Change comment `key` with `f` (given the comments and its index), re-reading the file
/// first so a change Claude just made isn't lost. Everything else stays as written.
fn update(path: &std::path::Path, key: &str, f: impl FnOnce(&mut Vec<Value>, usize)) -> Result<()> {
    let mut v: Value = serde_json::from_str(&std::fs::read_to_string(path)?)?;
    let comments = v["comments"].as_array_mut().context("no comments")?;
    let i = match key.strip_prefix('#') {
        // A position, from before the comment had an id: still that comment only if it has none.
        Some(n) => n.parse::<usize>().ok().filter(|&i| comments.get(i).is_some_and(|c| !c["id"].is_string())),
        // An id dv gave it in memory (`load_with_ids`), if the file has none yet.
        None => {
            let mut ids = comments.clone();
            assign_ids(&mut ids);
            ids.iter().position(|c| c["id"] == key)
        }
    };
    let i = i.context("that comment is no longer in the file (Claude may have just changed it): try again")?;
    assign_ids(comments);
    f(comments, i);
    write(path, &v)
}

/// Who started a comment: Claude, unless it's one the user left for Claude (shown as "you").
fn root_author(c: &Value) -> String {
    if c["author"] == USER { "you".into() } else { AUTHOR.into() }
}

/// A comment the user leaves for Claude on lines of the diff (`a` in dv), for Claude to answer
/// or act on. Starts the review file if there isn't one. Returns its id.
pub fn add_comment(path: &std::path::Path, a: &crate::gh::Anchor, code: Vec<String>, body: &str, about: Option<u64>, want: &str) -> Result<String> {
    let mut c = serde_json::json!({
        "path": a.path, "line": a.line, "side": if a.left { "old" } else { "new" },
        "author": USER, "body": body, "created": now(), "code": code,
    });
    if !want.is_empty() {
        c["want"] = want.into();
    }
    if let Some((s, _)) = a.start {
        c["start_line"] = s.into();
    }
    if let Some(id) = about {
        c["about"] = id.into(); // a GitHub review comment it asks Claude about
    }
    push_comment(path, c)
}

/// A question the user asks Claude about the PR conversation's comment at `url` (`a` on it).
/// (Or about a commit, its URL: `want` then says what of it, as on a comment.)
pub fn add_conversation(path: &std::path::Path, url: &str, body: &str, want: &str) -> Result<String> {
    let mut c = serde_json::json!({ "conversation": url, "author": USER, "body": body, "created": now() });
    if !want.is_empty() {
        c["want"] = want.into();
    }
    push_comment(path, c)
}

/// The questions about each conversation item (a commit) `from` are about `to` now: (from,
/// to) pairs, in one write.
pub fn rename_conversations(path: &std::path::Path, moves: &[(String, String)]) -> Result<()> {
    let mut v: Value = serde_json::from_str(&std::fs::read_to_string(path)?)?;
    for c in v["comments"].as_array_mut().into_iter().flatten() {
        if let Some((_, to)) = moves.iter().find(|(from, _)| c["conversation"] == from.as_str()) {
            c["conversation"] = to.as_str().into();
        }
    }
    write(path, &v)
}

/// Add comment `c` to the review file at `path` (starting it if there isn't one); its id.
fn push_comment(path: &std::path::Path, c: Value) -> Result<String> {
    let mut v: Value = match std::fs::read_to_string(path) {
        Ok(t) => serde_json::from_str(&t)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => serde_json::json!({ "comments": [] }),
        Err(e) => return Err(e.into()),
    };
    if !v["comments"].is_array() {
        v["comments"] = serde_json::json!([]);
    }
    let comments = v["comments"].as_array_mut().context("no comments")?;
    comments.push(c);
    assign_ids(comments);
    let id = comments.last().and_then(|c| c["id"].as_str()).unwrap_or_default().to_owned();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    write(path, &v)?;
    Ok(id)
}

/// Drop the review file's top-level draft `key`: Claude's proposed `"description"`, or its
/// `"closing"` comment (used, or discarded).
pub fn clear(path: &std::path::Path, key: &str) -> Result<()> {
    let mut v: Value = serde_json::from_str(&std::fs::read_to_string(path)?)?;
    if let Some(o) = v.as_object_mut() {
        o.remove(key);
    }
    write(path, &v)
}

/// Remove comment `key`.
pub fn dismiss(path: &std::path::Path, key: &str) -> Result<()> {
    update(path, key, |c, i| _ = c.remove(i))
}

/// Remove the comments with these ids (in one write). How many went.
pub fn dismiss_all(path: &std::path::Path, keys: &[String]) -> Result<usize> {
    let mut v: Value = serde_json::from_str(&std::fs::read_to_string(path)?)?;
    let comments = v["comments"].as_array_mut().context("no comments")?;
    let before = comments.len();
    comments.retain(|c| !c["id"].as_str().is_some_and(|id| keys.iter().any(|k| k == id)));
    let gone = before - comments.len();
    write(path, &v)?;
    Ok(gone)
}

/// Add the user's reply to comment `key`, for Claude to answer (`want`: "ask", "suggest" or
/// "change"; empty: unsaid).
pub fn reply(path: &std::path::Path, key: &str, body: &str, want: &str) -> Result<()> {
    update(path, key, |c, i| {
        if !c[i]["replies"].is_array() {
            c[i]["replies"] = serde_json::json!([]);
        }
        let mut r = serde_json::json!({ "author": USER, "body": body, "created": now() });
        if !want.is_empty() {
            r["want"] = want.into();
        }
        if let Some(a) = c[i]["replies"].as_array_mut() {
            a.push(r);
        }
        // (Talk again: the conversation's open again.)
        c[i]["talk_resolved"] = false.into();
    })
}

pub fn set_resolved(path: &std::path::Path, key: &str, on: bool) -> Result<()> {
    update(path, key, |c, i| c[i]["resolved"] = Value::Bool(on))
}

/// The conversation under Claude's comment `key` resolved (`on`) or reopened: the comment
/// itself stays as it is.
pub fn set_talk_resolved(path: &std::path::Path, key: &str, on: bool) -> Result<()> {
    update(path, key, |c, i| c[i]["talk_resolved"] = Value::Bool(on))
}

/// The time now as ISO 8601 (UTC), e.g. 2026-10-16T12:00:00Z.
pub fn now() -> String {
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

/// Where a comment's recorded code is now.
#[derive(Debug, PartialEq)]
pub enum Found {
    /// No code recorded: nothing to check against.
    Unknown,
    Here,
    /// Lines moved: the new (start_line, line).
    Moved(Option<u32>, u32),
    /// Its lines are gone or changed.
    Gone,
}

/// The file's lines on one side: (line number, text).
fn side_lines(f: &FileDiff, left: bool) -> Vec<(u32, &str)> {
    f.full
        .iter()
        .filter_map(|l| match (left, l.kind) {
            (true, Kind::Ctx | Kind::Del) => Some((l.old?, l.text.as_str())),
            (false, Kind::Ctx | Kind::Add) => Some((l.new?, l.text.as_str())),
            _ => None,
        })
        .collect()
}

/// Find thread `t`'s recorded code in `f`: where it was, else the nearest place the same
/// lines appear (trailing whitespace ignored).
pub fn locate(t: &Thread, f: &FileDiff) -> Found {
    let code: Vec<&str> = t.hunk.iter().map(|l| l.get(1..).unwrap_or("").trim_end()).collect();
    let Some(line) = t.line.or(t.original_line) else { return Found::Gone };
    if code.is_empty() {
        return Found::Unknown;
    }
    let start = t.start_line.unwrap_or(line);
    let lines = side_lines(f, t.left);
    let at = |i: usize| lines[i..].iter().take(code.len()).map(|l| l.1.trim_end()).eq(code.iter().copied());
    let hits = (0..lines.len()).filter(|&i| at(i));
    match hits.min_by_key(|&i| (lines[i].0 as i64 - start as i64).abs()) {
        Some(i) if lines[i].0 == start => Found::Here,
        Some(i) => {
            let s = lines[i].0;
            Found::Moved(t.start_line.map(|_| s), s + (line - start))
        }
        None => Found::Gone,
    }
}

/// Keep Claude's comments on their code: move the ones whose lines moved, and mark the ones
/// whose lines are gone outdated (or that aren't in the diff any more, which dv can't show).
pub fn reanchor(threads: &mut [Thread], files: &[FileDiff]) {
    for t in threads.iter_mut().filter(|t| t.claude.is_some() && t.conversation.is_empty()) {
        let found = match files.iter().find(|f| f.path == t.path) {
            Some(f) => locate(t, f),
            None => Found::Gone,
        };
        match found {
            // (Back on its code, one marked outdated by an earlier pass isn't any more.)
            Found::Unknown => {}
            Found::Here => (t.outdated, t.line) = (false, t.line.or(t.original_line)),
            // (Its new place is where it was last seen, should its code go and come back.)
            Found::Moved(s, l) => (t.start_line, t.line, t.original_line, t.outdated) = (s, Some(l), Some(l), false),
            Found::Gone => (t.outdated, t.line) = (true, None),
        }
    }
}

pub struct Report {
    /// dv can't place these comments.
    pub errors: Vec<String>,
    /// dv shows these, but GitHub won't take them, or they're outdated.
    pub warnings: Vec<String>,
    /// What `check` fixed in the file.
    pub notes: Vec<String>,
    pub comments: usize,
}

/// How long Claude's prose may run (characters, code blocks not counted) before
/// `review-check` asks for it shorter: about the longest tenth of what it wrote before. A
/// review comment; a reply; the summary.
const LONG: (usize, usize, usize) = (900, 1500, 4000);

/// `text`'s prose: its length without fenced code (a suggestion's lines aren't padding).
fn prose_len(text: &str) -> usize {
    let mut fenced = false;
    text.lines()
        .filter(|l| {
            let fence = l.trim_start().starts_with("```");
            fenced ^= fence;
            !fence && !fenced
        })
        .map(|l| l.chars().count() + 1)
        .sum()
}

/// Warnings for what Claude has just written that runs long: its summary, its comments new
/// to this check (no `code` yet), and its replies of the last hour. (Not older text: that
/// would have each run trim what's long done with.)
fn long_texts(v: &Value, rep: &mut Report) {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    let fresh = |r: &Value| r["created"].as_str().and_then(crate::gh::epoch_secs).is_some_and(|t| now - t < 3600);
    let mut long = |what: String, text: &str, max: usize| {
        let n = prose_len(text);
        if n > max {
            rep.warnings.push(format!(
                "{what}: {n} characters of prose, past {max}: cut what pads it (restating, recaps, hedges, offers), not the claim, the failure or the fix; fine if the user asked for detail"
            ));
        }
    };
    long("the summary".into(), v["summary"].as_str().unwrap_or(""), LONG.2);
    for (i, c) in v["comments"].as_array().into_iter().flatten().enumerate() {
        let at = format!("comment {} ({}:{})", i + 1, c["path"].as_str().unwrap_or(""), c["line"].as_u64().unwrap_or(0));
        if c["author"] != "user" && c.get("code").is_none() {
            long(at.clone(), c["body"].as_str().unwrap_or(""), LONG.0);
        }
        for r in c["replies"].as_array().into_iter().flatten().filter(|r| r["author"] == "claude" && fresh(r)) {
            long(format!("{at}, your reply"), r["body"].as_str().unwrap_or(""), LONG.1);
        }
    }
}

/// Check the review at `path` against the diff, and keep it anchored: record each comment's
/// code (if it isn't yet), and move comments whose code moved. Writes the file if it changed.
pub fn check(path: &std::path::Path, files: &[FileDiff]) -> Result<Report> {
    let r = load(path)?.with_context(|| format!("no review at {}", path.display()))?;
    let mut v: Value = serde_json::from_str(&std::fs::read_to_string(path)?)?;
    let mut rep = Report { errors: vec![], warnings: vec![], notes: vec![], comments: r.threads.len() };
    long_texts(&v, &mut rep);
    let mut changed = v["comments"].as_array_mut().is_some_and(|c| assign_ids(c));
    for (i, t) in r.threads.iter().enumerate() {
        let c = &mut v["comments"][i];
        if !t.conversation.is_empty() {
            continue; // about a conversation comment: on no lines to check
        }
        let at = format!("comment {} ({}:{})", i + 1, t.path, t.line.unwrap_or(0));
        let Some(f) = files.iter().find(|f| f.path == t.path) else {
            rep.errors.push(format!("{at}: {} isn't in the diff", t.path));
            continue;
        };
        let (mut start, mut line) = (t.start_line, t.line.unwrap_or(0));
        match locate(t, f) {
            Found::Here => {}
            Found::Moved(s, l) => {
                rep.notes.push(format!("{at}: its code moved, now at line {l}"));
                (start, line) = (s, l);
                c["line"] = l.into();
                if let Some(s) = s {
                    c["start_line"] = s.into();
                }
                changed = true;
            }
            Found::Gone => {
                rep.warnings.push(format!(
                    "{at}: the code it was on has changed, so dv shows it as outdated; to re-anchor it, set its \"line\" (and \"start_line\") and delete its \"code\""
                ));
                continue;
            }
            Found::Unknown => {
                match code_at(f, t.left, start.unwrap_or(line), line) {
                    Some(code) => {
                        c["code"] = serde_json::json!(code);
                        changed = true;
                    }
                    None => {
                        let side = if t.left { "old" } else { "new" };
                        rep.errors.push(format!("{at}: lines {}-{line} don't all exist on the {side} side", start.unwrap_or(line)));
                        continue;
                    }
                }
            }
        }
        // GitHub takes line comments within its hunks: 3 lines of context around a change.
        let full_at = |n: u32| f.full.iter().position(|l| if t.left { l.old == Some(n) && l.kind != Kind::Add } else { l.new == Some(n) && l.kind != Kind::Del });
        for n in start.into_iter().chain([line]) {
            let Some(i) = full_at(n) else { continue };
            let near = &f.full[i.saturating_sub(3)..(i + 4).min(f.full.len())];
            if !near.iter().any(|l| matches!(l.kind, Kind::Add | Kind::Del)) {
                rep.warnings.push(format!("{at}: line {n} is outside the diff's hunks, so it can't become a GitHub comment"));
            }
        }
    }
    if changed {
        write(path, &v)?;
    }
    Ok(rep)
}

#[test]
fn questions_about_the_conversation() {
    // `a` on a conversation comment: a question on no lines, by the comment's URL; check()
    // leaves it be (nothing to anchor), and it loads with its URL and no path.
    let p = std::env::temp_dir().join(format!("dv-claude-conv-{}.json", std::process::id()));
    _ = std::fs::remove_file(&p);
    let url = "https://github.com/o/r/pull/7#issuecomment-5";
    let id = add_conversation(&p, url, "what do they mean?", "").unwrap();
    let rep = check(&p, &[]).unwrap();
    assert!(rep.errors.is_empty() && rep.warnings.is_empty(), "{:?}", rep.errors);
    let t = &load(&p).unwrap().unwrap().threads[0];
    assert_eq!((t.conversation.as_str(), t.path.as_str(), t.line, t.claude.as_deref()), (url, "", None, Some(id.as_str())));
    assert_eq!(t.comments[0].author, "you");
    _ = std::fs::remove_file(&p);
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
    reply(&p, "#1", "why?", "").unwrap(); // by position: it has no id yet…
    assert!(set_resolved(&p, "#1", true).is_err()); // …and now it has one, so positions are out
    set_resolved(&p, "c2", true).unwrap();
    let r = load(&p).unwrap().unwrap();
    let t = &r.threads[1];
    assert!(t.resolved);
    assert_eq!((t.comments[1].author.as_str(), t.comments[1].body.as_str()), ("you", "why?"));
    assert_eq!(t.comments[1].created.len(), "2026-10-16T12:00:00Z".len());
    assert_eq!(r.threads[0].claude.as_deref(), Some("c1"));
    dismiss(&p, "c1").unwrap();
    assert_eq!(load(&p).unwrap().unwrap().threads[0].claude.as_deref(), Some("c2")); // ids survive
    let left = std::fs::read_to_string(&p).unwrap();
    _ = std::fs::remove_file(&p);
    assert!(!left.contains("\"x\"") && left.contains("\"extra\": true"));
}

#[test]
fn comments_for_claude() {
    let p = std::env::temp_dir().join(format!("dv-claude-ask-{}", std::process::id())).join("pr-1.json");
    let a = crate::gh::Anchor { path: "src/a.rs".into(), line: 3, left: false, start: Some((2, false)) };
    let id = add_comment(&p, &a, vec!["x".into(), "y".into()], "why?", None, "").unwrap(); // no file yet: started
    let r = load(&p).unwrap().unwrap();
    _ = std::fs::remove_dir_all(p.parent().unwrap());
    let t = &r.threads[0];
    assert_eq!((t.claude.as_deref(), t.start_line, t.line), (Some(id.as_str()), Some(2), Some(3)));
    assert_eq!((t.comments[0].author.as_str(), t.comments[0].body.as_str()), ("you", "why?"));
    assert_eq!(t.hunk, ["+x", "+y"]); // its code, so it follows it as the code changes
}

#[test]
fn ids_given_on_load_stay_in_memory() {
    let p = std::env::temp_dir().join(format!("dv-claude-ids-{}.json", std::process::id()));
    let text = r#"{"comments": [{"path": "a", "line": 1, "body": "x", "id": "c4"}, {"path": "a", "line": 2, "body": "y"}]}"#;
    std::fs::write(&p, text).unwrap();
    let r = load_with_ids(&p).unwrap().unwrap();
    assert_eq!(r.threads[1].claude.as_deref(), Some("c5"));
    assert_eq!(std::fs::read_to_string(&p).unwrap(), text); // not written: Claude may be mid-edit
    reply(&p, "c5", "ok", "").unwrap(); // that id finds it, and is written with dv's change
    let r = load(&p).unwrap().unwrap();
    _ = std::fs::remove_file(&p);
    assert_eq!((r.threads[1].claude.as_deref(), r.threads[1].comments.len()), (Some("c5"), 2));
}

#[test]
fn session_ids() {
    let (a, b) = (new_session_id(), new_session_id());
    assert_ne!(a, b);
    assert_eq!(a.len(), 36);
    assert_eq!(&a[14..15], "4"); // version 4
    assert_eq!(session_path(std::path::Path::new("/d/dv/r/reviews/pr-7.json")), std::path::Path::new("/d/dv/r/reviews/pr-7.session"));
    assert_eq!(repo_key(std::path::Path::new("/home/me/Git/bdk/.git")), "home-me-Git-bdk"); // worktrees share it
    // One per checkout dir; one Claude Code doesn't have isn't resumed.
    let home = std::env::temp_dir().join(format!("dv-sessions-{}", std::process::id()));
    let review = home.join("pr-7.json");
    save_session(&review, std::path::Path::new("/a"), Some("id-a")).unwrap();
    save_session(&review, std::path::Path::new("/b"), Some("id-b")).unwrap();
    let text = std::fs::read_to_string(session_path(&review)).unwrap();
    let tag = skill_tag();
    assert_eq!(text, format!("/a\tid-a\t{tag}\n/b\tid-b\t{tag}\n"));
    assert_eq!(load_session(&review, std::path::Path::new("/a")), None); // not in Claude Code's store
    assert_eq!(session_line(&text, "/b"), Some(("id-b".into(), true)));
    assert_eq!(session_line("/a\tid-a\n", "/a"), Some(("id-a".into(), false))); // dv 0.2's: another skill
    assert_eq!(session_line("/a\tid-a\tffff\n", "/a"), Some(("id-a".into(), false)));
    save_session(&review, std::path::Path::new("/a"), None).unwrap();
    assert_eq!(std::fs::read_to_string(session_path(&review)).unwrap(), format!("/b\tid-b\t{tag}\n"));
    _ = std::fs::remove_dir_all(&home);
}

#[test]
fn anchoring() {
    let f = diff::parse(diff::SAMPLE).remove(0);
    // New side: 1 `fn main() {`, 2 `    new_call(x);`, 3 `}`. Old side: 2 `    old();`.
    let p = std::env::temp_dir().join(format!("dv-claude-anchor-{}.json", std::process::id()));
    std::fs::write(&p, r#"{ "comments": [
        { "path": "src/a.rs", "line": 2, "body": "a" },
        { "path": "src/a.rs", "line": 2, "side": "old", "body": "b" },
        { "path": "src/a.rs", "line": 9, "body": "c" } ] }"#).unwrap();
    let rep = check(&p, std::slice::from_ref(&f)).unwrap();
    assert_eq!(rep.errors.len(), 1); // line 9 doesn't exist
    let mut r = load(&p).unwrap().unwrap();
    assert_eq!(r.threads[0].hunk, ["+    new_call(x);"]); // code recorded by check
    assert_eq!(r.threads[1].hunk, ["-    old();"]);
    assert_eq!(locate(&r.threads[0], &f), Found::Here);
    // The code moved down a line: found there. Changed: outdated.
    (r.threads[0].line, r.threads[0].original_line) = (Some(1), Some(1)); // as parse has a comment on line 1
    assert_eq!(locate(&r.threads[0], &f), Found::Moved(None, 2));
    r.threads[1].hunk = vec!["-    gone();".into()];
    reanchor(&mut r.threads, std::slice::from_ref(&f));
    assert_eq!((r.threads[0].line, r.threads[0].outdated), (Some(2), false));
    assert_eq!((r.threads[1].line, r.threads[1].outdated), (None, true));
    // Its code gone, then back (kept in memory while the file can't be read): where it moved to.
    r.threads[0].hunk = vec!["+    gone();".into()];
    reanchor(&mut r.threads, std::slice::from_ref(&f));
    assert_eq!((r.threads[0].line, r.threads[0].original_line, r.threads[0].outdated), (None, Some(2), true)); // last seen on 2
    r.threads[0].hunk = vec!["+    new_call(x);".into()];
    reanchor(&mut r.threads, std::slice::from_ref(&f));
    assert_eq!((r.threads[0].line, r.threads[0].outdated), (Some(2), false));
    // check moves a moved comment in the file too.
    let mut v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
    v["comments"][0]["line"] = 1.into();
    std::fs::write(&p, v.to_string()).unwrap();
    let rep = check(&p, std::slice::from_ref(&f)).unwrap();
    assert_eq!(rep.notes.len(), 1);
    assert_eq!(load(&p).unwrap().unwrap().threads[0].line, Some(2));
    _ = std::fs::remove_file(&p);
}

#[test]
fn skill_updates_only_forward() {
    assert_eq!(installed_by(&skill_file()), env!("CARGO_PKG_VERSION"));
    assert_eq!(installed_by("---\nname: dv-review\n---\n"), "0.0.0"); // from before the mark
    assert!(newer("0.10.0", "0.9.9") && newer("1.0.0", "0.99.0") && !newer("0.3.0", "0.3.0") && !newer("0.2.9", "0.3.0"));
}

#[test]
fn running_markers() {
    let dir = std::env::temp_dir().join(format!("dv-running-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("pr-7.json"), r#"{"what": "making the changes", "pid": 42, "session": null}"#).unwrap();
    std::fs::write(dir.join("pr-8.json"), r#"{"what": "reviewing (in its pane)", "pid": null, "session": "abc"}"#).unwrap();
    std::fs::write(dir.join("pr-9.json"), r#"{"what": "answering", "pid": 43, "session": null}"#).unwrap();
    // Only the ones still at it: process 42 is, 43 isn't; the pane with session abc is open.
    let alive = |pid: Option<u32>, session: Option<&str>| pid == Some(42) || session == Some("abc");
    let r = running(&dir, alive);
    assert_eq!(r.len(), 2);
    assert_eq!((r[&7].as_str(), r[&8].as_str()), ("making the changes", "reviewing (in its pane)"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn turn_ended_from_the_transcript() {
    // Its last message an answer (`end_turn`) after the run started: ended, with its words; a
    // tool call or a result it works from: still at it; an old answer (before the run): not yet.
    let line = |t: &str, ts: &str, stop: &str, text: &str| {
        serde_json::json!({ "type": t, "timestamp": ts, "message": { "stop_reason": stop, "content": [{ "type": "text", "text": text }] } }).to_string()
    };
    let since = crate::gh::epoch_secs("2026-10-18T10:00:00Z").unwrap();
    let done = [line("user", "2026-10-18T10:00:01Z", "", "go"), line("assistant", "2026-10-18T10:01:00Z", "end_turn", "All done."), r#"{"type":"queue-operation"}"#.into()].join("\n");
    assert_eq!(turn_ended_in(&done, since).as_deref(), Some("All done."));
    let working = [line("assistant", "2026-10-18T10:01:00Z", "tool_use", ""), line("user", "2026-10-18T10:01:02Z", "", "")].join("\n");
    assert_eq!(turn_ended_in(&working, since), None);
    let old = line("assistant", "2026-10-18T09:00:00Z", "end_turn", "Earlier.");
    assert_eq!(turn_ended_in(&old, since), None);
}

#[test]
fn a_branch_review_moves_to_its_pr() {
    let d = std::env::temp_dir().join(format!("dv-move-review-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    let (from, to) = (d.join("branch-dv-replace-12.json"), d.join("pr-40.json"));
    std::fs::write(&from, "{}").unwrap();
    std::fs::write(session_path(&from), "s").unwrap();
    move_review(&from, &to).unwrap();
    assert!(!from.exists() && to.exists() && session_path(&to).exists());
    // Not over a review the PR has.
    std::fs::write(&from, "{\"summary\": \"other\"}").unwrap();
    move_review(&from, &to).unwrap();
    assert_eq!(std::fs::read_to_string(&to).unwrap(), "{}");
    _ = std::fs::remove_dir_all(&d);
}

#[test]
fn notes_keep_to_their_budget() {
    assert_eq!(notes_over("- a point\n"), None);
    let long = format!("- {}\n", "x".repeat(NOTE_POINT));
    assert!(notes_over(&long).is_some_and(|w| w.contains("1 point(s) longer") && !w.contains("KB")));
    let big = "- a short point\n".repeat(NOTES_BUDGET / 10);
    assert!(notes_over(&big).is_some_and(|w| w.contains("KB, over their 12 KB budget") && !w.contains("longer")));
}

#[test]
fn long_prose_is_flagged_when_new() {
    assert_eq!(prose_len("ab\n```suggestion\nlet x = 1;\n```\ncd"), 6); // the code's not counted
    let long = "x".repeat(LONG.0 + 1);
    let v = serde_json::json!({ "summary": "ok", "comments": [
        { "path": "a.rs", "line": 1, "body": long },                    // new: flagged
        { "path": "a.rs", "line": 2, "body": long, "code": "x" },       // checked before: not
        { "path": "a.rs", "line": 3, "body": long, "author": "user" },  // the user's: not
        { "path": "a.rs", "line": 4, "body": "short", "code": "x", "replies": [
            { "author": "claude", "body": "y".repeat(LONG.1 + 1), "created": now() },                    // just now: flagged
            { "author": "claude", "body": "y".repeat(LONG.1 + 1), "created": "2020-01-01T00:00:00Z" }, // long ago: not
        ] },
    ] });
    let mut rep = Report { errors: vec![], warnings: vec![], notes: vec![], comments: 0 };
    long_texts(&v, &mut rep);
    assert_eq!(rep.warnings.len(), 2, "{:?}", rep.warnings);
    assert!(rep.warnings[0].starts_with("comment 1 (a.rs:1): 902 characters"), "{}", rep.warnings[0]);
    assert!(rep.warnings[1].starts_with("comment 4 (a.rs:4), your reply: "));
}

#[test]
fn a_review_as_a_replacement_reads_it() {
    let c = |who: &str, body: &str| crate::gh::Comment { author: who.into(), body: body.into(), ..Default::default() };
    let t = |path: &str, line, sev: &str, comments| crate::gh::Thread { path: path.into(), line: Some(line), severity: sev.into(), comments, ..Default::default() };
    let r = Review {
        summary: "Fixes the symptom.".into(),
        threads: vec![
            t("src/a.rs", 12, "bug", vec![c(AUTHOR, "This drops the error."), c("you", "Agreed:\nmake it return it")]),
            crate::gh::Thread { resolved: true, ..t("src/b.rs", 3, "", vec![c("you", "rename it")]) },
            crate::gh::Thread { conversation: "summary".into(), ..t("", 0, "", vec![c("you", "move the check into parse()")]) },
        ],
        ..Default::default()
    };
    assert_eq!(
        review_brief(&r),
        "Fixes the symptom.\n\n- `src/a.rs:12` (bug)\n  Claude: This drops the error.\n  the user: Agreed:\n  make it return it\n\n- `src/b.rs:3` (resolved)\n  the user: rename it\n\n- on the summary\n  the user: move the check into parse()"
    );
}
