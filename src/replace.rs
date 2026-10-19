//! Replacements: PRs Claude drafts locally to replace others (low-effort ones, say), kept
//! unpublished until you've reviewed them. Each is a branch `dv/replace-N` (N: the first PR it
//! replaces) in a worktree `<repo>-replace-N`, and a record in dv's directory,
//! `replacements/N.json`: the PRs it replaces, its base, and Claude's drafts of its title and
//! description and of the note closing the originals. A new PR (replacing nothing: what you
//! asked for) is the same, as `dv/new-K` in `<repo>-new-K`, its record keeping your request.
use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// Something you asked of a replacement's drafts (`a` on one, or for a redraft).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Request {
    pub text: String,
    /// What you want of Claude: `"ask"` (an answer; nothing changed), `"change"` (made in its
    /// drafts or code); empty: as it reads.
    pub want: String,
    /// The draft it's about: `"description"`, `"closing note"`; empty: the whole.
    pub on: String,
    /// Claude's answer: what it did about it, or (a question) its answer. Claude writes it.
    pub answer: String,
}

/// What dv puts for an answer when a redraft took a request in without one (Claude didn't
/// say): done, not to ask again. Shown as dv's note, not as Claude's.
pub const NO_ANSWER: &str = "_(redrafted with it)_";

impl Request {
    /// A redraft has taken it in: Claude has answered it (one rule, whatever Claude did to the
    /// record's order).
    pub fn done(&self) -> bool {
        !self.answer.trim().is_empty()
    }
}

/// A PR a replacement replaces, as it was when the replacement was drafted.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Original {
    pub number: u64,
    pub title: String,
    pub author: String,
    pub url: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Record {
    /// The first PR it replaces: its name (`dv/replace-N`, `<repo>-replace-N`, `N.json`).
    pub id: u64,
    pub replaces: Vec<Original>,
    /// The branch it's based on (the originals'), on origin.
    pub base: String,
    /// Claude's drafts: the PR's title and description, and the note for closing the originals.
    pub title: String,
    pub body: String,
    pub closing_note: String,
    /// The Claude Code session that drafted it, to carry on in.
    pub session: String,
    /// Its PR, once published (a draft).
    pub published: Option<u64>,
    /// The originals closed (with the note) since.
    pub closed: Vec<u64>,
    /// The process drafting it now, if one is (`drafting_now` checks it's still running).
    pub drafting: Option<u32>,
    /// A new PR's: what you asked it to do (a replacement's is what it replaces).
    pub task: String,
    /// What you've asked of its drafts, oldest first.
    pub asks: Vec<Request>,
    /// How many of `asks` the draft under way was given (the rest wait for the next).
    pub taking: usize,
}

impl Record {
    /// What to draft it again from: the PRs it replaces, or (a new PR) itself.
    pub fn ids(&self) -> Vec<u64> {
        match self.replaces.is_empty() {
            true => vec![self.id],
            false => self.replaces.iter().map(|o| o.number).collect(),
        }
    }

    /// Claude has drafted it (so drafting it again carries on, in the same session).
    pub fn drafted(&self) -> bool {
        !self.title.is_empty() || !self.body.is_empty()
    }

    /// Claude is drafting it now (from any dv): its process is still running.
    pub fn drafting_now(&self) -> bool {
        self.drafting.is_some_and(|pid| {
            let alive = std::process::Command::new("kill").args(["-0", &pid.to_string()]).stderr(std::process::Stdio::null()).status();
            alive.is_ok_and(|s| s.success())
        })
    }
}

/// New PRs' ids start here (`new_id`), above any PR number: new PR K is `NEW + K`.
const NEW: u64 = 1 << 40;

/// Whether `id` is a new PR's (not a replacement's).
pub fn is_new(id: u64) -> bool {
    id >= NEW
}

/// Its name: its branch's (`dv/<name>`) and its worktree's (`<repo>-<name>`).
pub fn name(id: u64) -> String {
    match is_new(id) {
        true => format!("new-{}", id - NEW),
        false => format!("replace-{id}"),
    }
}

/// How it's said: "#N" (the PR a replacement replaces first) or "new K".
pub fn shown(id: u64) -> String {
    match is_new(id) {
        true => format!("new {}", id - NEW),
        false => format!("#{id}"),
    }
}

/// What it is, in words: "the replacement of #N" or "new PR K".
pub fn label(id: u64) -> String {
    match is_new(id) {
        true => format!("new PR {}", id - NEW),
        false => format!("the replacement of #{id}"),
    }
}

pub fn branch(id: u64) -> String {
    format!("dv/{}", name(id))
}

/// The replacement (or new PR) checked out here, by its branch.
pub fn current() -> Option<u64> {
    let b = crate::diff::git(&["branch", "--show-current"]).ok()?;
    let b = b.trim();
    match b.strip_prefix("dv/new-") {
        Some(k) => k.parse::<u64>().ok().map(|k| NEW + k),
        None => b.strip_prefix("dv/replace-")?.parse().ok(),
    }
}

/// The replacement checked out here, if it's PR `pr`'s: the PR it was published as (not one
/// someone else's dv published from a record of its own), or no PR (GitHub not asked, or not
/// answering).
pub fn here(pr: Option<u64>) -> Option<Record> {
    load(current()?).ok().filter(|r| pr.is_none() || r.published == pr)
}

/// Published as PR `n`: Claude's review of the draft (and its session) moved to the PR's, for
/// dv on the PR to carry on with. Once: not over one the PR has.
pub fn adopt_review(n: u64) {
    if here(Some(n)).is_some()
        && let (Ok(from), Ok(to)) = (crate::claude::path(None), crate::claude::path(Some(n)))
    {
        _ = crate::claude::move_review(&from, &to);
    }
}

/// The next new PR's id: one past the last there's a record of.
pub fn new_id() -> u64 {
    all().iter().map(|r| r.id).filter(|&id| is_new(id)).max().map_or(NEW + 1, |m| m + 1)
}

fn dir() -> Result<PathBuf> {
    Ok(crate::claude::dv_dir()?.join("replacements"))
}

pub fn path(id: u64) -> Result<PathBuf> {
    Ok(dir()?.join(format!("{id}.json")))
}

pub fn load(id: u64) -> Result<Record> {
    let p = path(id)?;
    parse(id, &std::fs::read_to_string(&p).with_context(|| format!("reading {}", p.display()))?)
}

fn parse(id: u64, text: &str) -> Result<Record> {
    let v: Value = serde_json::from_str(text)?;
    let s = |v: &Value| v.as_str().unwrap_or("").to_owned();
    let replaces = v["replaces"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    Ok(Record {
        id,
        replaces: replaces
            .iter()
            .map(|o| Original { number: o["number"].as_u64().unwrap_or(0), title: s(&o["title"]), author: s(&o["author"]), url: s(&o["url"]) })
            .collect(),
        base: s(&v["base"]),
        title: s(&v["title"]),
        body: s(&v["body"]),
        closing_note: s(&v["closing_note"]),
        session: s(&v["session"]),
        published: v["published"].as_u64(),
        closed: v["closed"].as_array().into_iter().flatten().filter_map(Value::as_u64).collect(),
        drafting: v["drafting"].as_u64().map(|p| p as u32),
        task: s(&v["task"]),
        asks: v["asks"].as_array().into_iter().flatten().map(|a| Request {
                    text: s(&a["text"]),
                    want: s(&a["want"]),
                    on: s(&a["on"]),
                    // (Records from before answers: taken in, unanswered.)
                    answer: match (s(&a["answer"]), a["done"] == true) {
                        (ans, true) if ans.trim().is_empty() => NO_ANSWER.into(),
                        (ans, _) => ans,
                    },
                }).collect(),
        taking: v["taking"].as_u64().unwrap_or(0) as usize,
    })
}

fn to_json(r: &Record) -> String {
    let replaces: Vec<Value> =
        r.replaces.iter().map(|o| json!({ "number": o.number, "title": o.title, "author": o.author, "url": o.url })).collect();
    let v = json!({
        "replaces": replaces, "base": r.base, "title": r.title, "body": r.body,
        "closing_note": r.closing_note, "session": r.session, "published": r.published, "closed": r.closed,
        "drafting": r.drafting, "task": r.task,
        "asks": r.asks.iter().map(|a| json!({ "text": a.text, "want": a.want, "on": a.on, "answer": a.answer })).collect::<Vec<_>>(), "taking": r.taking,
    });
    serde_json::to_string_pretty(&v).unwrap_or_default() + "\n"
}

/// Change replacement `id`'s record as it is now (Claude may have written to it, another dv
/// asked for something): load, `f`, save, quickly, so nothing written meanwhile is lost.
/// `f`'s answer, and the record as saved.
fn update<T>(id: u64, f: impl FnOnce(&mut Record) -> Result<T>) -> Result<(T, Record)> {
    let mut r = match path(id)?.exists() {
        true => load(id)?,
        false => Record { id, ..Record::default() },
    };
    let out = f(&mut r)?;
    save(&r)?;
    Ok((out, r))
}

pub fn save(r: &Record) -> Result<()> {
    let p = path(r.id)?;
    std::fs::create_dir_all(p.parent().context("no dir")?)?;
    Ok(std::fs::write(p, to_json(r))?)
}

/// When any replacement's record last changed (or one came or went), to notice it.
pub fn changed() -> Option<std::time::SystemTime> {
    let d = dir().ok()?;
    let entries = std::fs::read_dir(&d).ok()?.flatten().filter_map(|e| e.metadata().ok()?.modified().ok());
    entries.chain(std::fs::metadata(&d).ok()?.modified().ok()).max()
}

/// Every replacement in this repo, by number.
pub fn all() -> Vec<Record> {
    let Ok(entries) = dir().and_then(|d| Ok(std::fs::read_dir(d)?)) else { return vec![] };
    let mut out: Vec<Record> = entries
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.strip_suffix(".json")?.parse().ok())
        .filter_map(|id| load(id).ok())
        .collect();
    out.sort_by_key(|r| r.id);
    out
}

/// Set up a replacement of `prs` (the first names it) from the repo at `root`: what it replaces
/// and its base in its record (`draft` has marked it, and sets its session), and its branch,
/// from the originals' base, in its worktree. Returns the worktree.
fn prepare(prs: &[u64], root: &Path) -> Result<PathBuf> {
    let id = *prs.first().context("no PR to replace")?;
    let git = |args: &[&str]| crate::diff::git(&[&["-C", &root.to_string_lossy()], args].concat());
    let mut originals = vec![];
    let mut bases: Vec<(u64, String)> = vec![];
    for &n in prs {
        let p = crate::gh::pr(Some(n))?;
        bases.push((n, p.base));
        originals.push(Original { number: n, title: p.title, author: p.author, url: p.url });
    }
    // One replacement can only target one branch.
    if let Some((n, b)) = bases.iter().find(|b| b.1 != bases[0].1) {
        anyhow::bail!("#{} targets {}, #{n} targets {b}: one replacement can't replace both", bases[0].0, bases[0].1);
    }
    let base = bases.first().map(|b| b.1.clone()).unwrap_or_default();
    let ((), r) = update(id, |r| {
        (r.base, r.replaces) = (base, originals);
        Ok(())
    })?;
    let dir = crate::worktree_dir_named(root, &name(id))?;
    if dir.exists() {
        crate::ensure_worktree_of(&dir, root)?;
        // Carrying on: from the base as it is now, if that goes cleanly (else Claude sees why).
        _ = rebase(&r, &dir);
    } else {
        _ = git(&["fetch", "-q", &crate::diff::remote(), &r.base]);
        let (b, d) = (branch(id), dir.to_string_lossy().into_owned());
        match git(&["rev-parse", "--verify", "-q", &format!("refs/heads/{b}")]) {
            Ok(_) => git(&["worktree", "add", &d, &b])?,
            Err(_) => git(&["worktree", "add", "-b", &b, &d, &crate::diff::remote_branch(&r.base)])?,
        };
    }
    Ok(dir)
}

/// Set up a new PR doing `task` from the repo at `root`: its record, and its branch from the
/// repo's default branch (on its remote), in its worktree. Its id.
pub fn create_new(task: &str, root: &Path) -> Result<u64> {
    let git = |args: &[&str]| crate::diff::git(&[&["-C", &root.to_string_lossy()], args].concat());
    let base = crate::diff::default_branch();
    let id = new_id();
    let r = Record { id, base: base.clone(), task: task.trim().to_owned(), ..Record::default() }; // its session: `draft`
    let dir = crate::worktree_dir_named(root, &name(id))?;
    _ = git(&["fetch", "-q", &crate::diff::remote(), &base]);
    git(&["worktree", "add", "-b", &branch(id), &dir.to_string_lossy(), &crate::diff::remote_branch(&base)])?;
    save(&r)?;
    Ok(id)
}

/// A new PR's worktree, to draft it (again): its branch brought up to its base.
fn prepare_new(id: u64, root: &Path) -> Result<PathBuf> {
    let r = load(id)?;
    let dir = crate::worktree_dir_named(root, &name(id))?;
    anyhow::ensure!(dir.exists(), "its worktree {} is gone", dir.display());
    crate::ensure_worktree_of(&dir, root)?;
    _ = rebase(&r, &dir); // carrying on: from its base as it is now, if that goes cleanly
    Ok(dir)
}

/// Bring replacement `r`'s branch (in `dir`) up to its base on origin: fetch, then rebase onto
/// it. A rebase that conflicts is undone, and said so.
pub fn rebase(r: &Record, dir: &Path) -> Result<()> {
    let git = |args: &[&str]| crate::diff::git(&[&["-C", &dir.to_string_lossy()], args].concat());
    git(&["fetch", "-q", &crate::diff::remote(), &r.base])?;
    if let Err(e) = git(&["rebase", "-q", &crate::diff::remote_branch(&r.base)]) {
        _ = git(&["rebase", "--abort"]);
        anyhow::bail!("it doesn't rebase cleanly onto {} ({e:#}): ask Claude to redraft it on the new base", crate::diff::remote_branch(&r.base));
    }
    Ok(())
}

/// What Claude is asked, to draft (or redraft) replacement `r`. Your notes on the repo come
/// with it, to read: a draft reads untrusted PRs, so it gets no access to them (nor to dv's
/// other files), only to its record.
pub fn prompt(r: &Record, extra: &str) -> String {
    let prs: Vec<String> = r.replaces.iter().map(|o| format!("#{}", o.number)).collect();
    let now = crate::claude::now();
    let notes = crate::claude::notes_path().ok().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
    let notes = match notes.trim() {
        "" => String::new(),
        n => format!("\n\nThe user's notes on this repo (read-only here; follow them):\n<notes>\n{n}\n</notes>"),
    };
    let p = match is_new(r.id) {
        // What it's to do is the user's request (in `dv replace-info` too); `extra`, if any, is
        // more, for carrying on.
        true => format!(
            "Use the dv-review skill to draft a new PR doing what the user asks (its section 7: \"Draft a new PR\"). \
             First run `dv replace-info`. (It's {now} now.){notes}\n\nThe user asks:\n{}",
            r.task
        ),
        false => format!(
            "Use the dv-review skill to draft a replacement for PR {} (its section 6: \"Draft a replacement\"). \
             First run `dv replace-info`. (It's {now} now.){notes}",
            prs.join(", ")
        ),
    };
    // What you asked of its drafts that no redraft has taken in yet: in the record, to answer.
    let want = |w: &str| match w {
        "ask" => " (ask: answer it; change nothing)",
        "change" => " (change: make it)",
        _ => "",
    };
    let asks: Vec<String> = r.asks.iter().enumerate().filter(|(_, a)| !a.done()).map(|(i, a)| format!("`asks[{i}]`{}: {}", want(&a.want), a.text.trim())).collect();
    let p = match asks.is_empty() {
        true => p,
        false => format!(
            "{p}\n\nThe user asks of your drafts (the record's `asks`): do what each says it wants (ask: only answer it, \
             changing nothing; change: make the change; neither said: as it reads), and write what you did, or your answer, \
             in a sentence or few, in its `\"answer\"` (in the record; the user reads it beside your drafts).\n\n{}",
            asks.join("\n\n")
        ),
    };
    crate::claude::with_instructions(&p, extra)
}

/// Whether replacement `id` has drafts (`Record::drafted`).
pub fn drafted(id: u64) -> bool {
    load(id).is_ok_and(|r| r.drafted())
}

/// Claude at work on replacement `id` in dv (`pid`: a run of dv's, answering you, making a
/// change), marked as a redraft is, so nothing else starts one there meanwhile; `None`: done
/// (its own mark only comes off: not a redraft's that started after).
pub fn mark_at_work(id: u64, pid: Option<u32>, was: u32) {
    _ = update(id, |r| {
        match pid {
            Some(p) if !r.drafting_now() => r.drafting = Some(p),
            None if r.drafting == Some(was) => r.drafting = None,
            _ => {}
        }
        Ok(())
    });
}

/// Ask Claude about replacement `id`'s drafts (`on`: the one it's about, or ""; `want`: see
/// `Request`): kept in its record (shown beside it), for its next redraft to take in.
pub fn ask(id: u64, text: &str, on: &str, want: &str) -> Result<()> {
    update(id, |r| {
        r.asks.push(Request { text: text.trim().to_owned(), on: on.to_owned(), want: want.to_owned(), ..Request::default() });
        Ok(())
    })
    .map(drop)
}

/// Have Claude draft a replacement for `prs` with a dv of its own (`dv replace`), in the
/// background: it carries on if this dv quits. Its output goes to a log in the temp dir.
pub fn spawn(prs: &[u64]) -> Result<()> {
    use std::os::unix::process::CommandExt;
    let log = std::env::temp_dir().join(format!("dv-replace-{}.log", prs[0]));
    std::process::Command::new(crate::self_exe())
        .arg("replace")
        .args(prs.iter().map(u64::to_string))
        .stdin(std::process::Stdio::null())
        .stdout(std::fs::File::create(&log)?)
        .stderr(std::fs::File::create(log.with_extension("err"))?)
        .process_group(0) // not this terminal's: closing it doesn't stop the draft
        .spawn()?;
    Ok(())
}

/// The directory a draft may write to (its record): `--add-dir`.
pub fn writable() -> Result<PathBuf> {
    dir()
}

/// Draft replacement `prs` with Claude, in its worktree, in the foreground of this thread:
/// what it took ("12 turns, $0.40"). `extra`: what you wrote for it just now: your
/// instructions for a first draft (none: your last ones for replacements); for one drafted
/// already, asked of its drafts (as `ask`).
pub fn draft(prs: &[u64], root: &Path, extra: &str, prepared: impl FnOnce()) -> Result<String> {
    let id = *prs.first().context("no PR to replace")?;
    // Under way first, before the slow setting up: others (dv list, another dv) see it, and
    // don't start another.
    let existed = path(id)?.exists();
    anyhow::ensure!(existed || !is_new(id), "no new PR {}", label(id));
    let (first, _) = update(id, |r| {
        anyhow::ensure!(!r.drafting_now(), "Claude is already drafting {}", label(id));
        r.drafting = Some(std::process::id());
        if r.drafted() && !extra.trim().is_empty() {
            r.asks.push(Request { text: extra.trim().to_owned(), ..Request::default() });
        }
        // A first draft: a session of its own (one that failed before may have made the last
        // one, which Claude Code won't start again).
        let first = !r.drafted();
        if first {
            r.session = crate::claude::new_session_id();
        }
        Ok(first)
    })?;
    let saved;
    let extra = match extra.trim() {
        "" if first && !is_new(id) => {
            saved = crate::claude::instructions("replace");
            saved.as_str()
        }
        e => e,
    };
    let mut set_up = false;
    let res = (|| {
        let dir = match prs {
            [id] if is_new(*id) => prepare_new(*id, root)?,
            _ => prepare(prs, root)?,
        };
        set_up = true;
        // What you've asked of it so far goes in its prompt: those are what it takes in.
        let (r, _) = update(id, |r| {
            r.taking = r.asks.len();
            Ok(r.clone())
        })?;
        run_claude(&r, &dir, first, if first { extra } else { "" }, prepared)
    })();
    // Claude has written to the record meanwhile (its answers): the mark comes off; a change
    // it was given and made (it finished) without saying so is done all the same, not asked
    // again and again.
    _ = update(id, |now| {
        let taken = std::mem::take(&mut now.taking);
        now.drafting = None;
        for a in now.asks.iter_mut().take(taken).filter(|a| res.is_ok() && a.want != "ask" && !a.done()) {
            a.answer = NO_ANSWER.into();
        }
        Ok(())
    });
    // Not set up (PRs on different bases, one that isn't there): no record left of it.
    if !set_up && !existed {
        _ = std::fs::remove_file(path(id)?);
    }
    res
}

/// `first`: its first draft (a new session; else it carries on in its own).
fn run_claude(r: &Record, dir: &Path, first: bool, extra: &str, prepared: impl FnOnce()) -> Result<String> {
    prepared();
    let session = match first {
        true => ["--session-id".to_owned(), r.session.clone()],
        false => ["--resume".to_owned(), r.session.clone()],
    };
    let log = std::env::temp_dir().join(format!("dv-claude-replace-{}-{}.log", std::process::id(), r.id));
    let st = crate::claude::claude_command(&prompt(r, extra), dir, &session, crate::claude::REPLACE_TOOLS, Some(&writable()?))
        .stdin(std::process::Stdio::null())
        .stdout(std::fs::File::create(&log)?)
        .stderr(std::fs::File::create(log.with_extension("err"))?)
        .status()
        .context("is Claude Code (`claude`) installed?")?;
    let v: Value = std::fs::read_to_string(&log).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    if !st.success() || v["is_error"] != false {
        anyhow::bail!("Claude stopped ({st}): see {}", log.display());
    }
    Ok(format!("{} turns, ${:.2}", v["num_turns"].as_u64().unwrap_or(0), v["total_cost_usd"].as_f64().unwrap_or(0.0)))
}

/// The remote replacement branches are pushed to (`git config dv.replaceRemote`): not origin,
/// but yours (a fork). `None` until it's set.
pub fn remote() -> Option<String> {
    crate::diff::git(&["config", "--get", "dv.replaceRemote"]).ok().map(|r| r.trim().to_owned()).filter(|r| !r.is_empty())
}

pub fn set_remote(name: &str) -> Result<()> {
    crate::diff::git(&["config", "dv.replaceRemote", name]).map(drop)
}

/// The remotes it could be (all but origin).
pub fn remote_choices() -> Vec<(String, String)> {
    let names = crate::diff::git(&["remote"]).unwrap_or_default();
    names
        .lines()
        .filter(|n| *n != crate::diff::remote())
        .map(|n| (n.to_owned(), crate::diff::git(&["remote", "get-url", n]).unwrap_or_default().trim().to_owned()))
        .collect()
}

/// The GitHub owner of a remote's repo, from its URL (`git@github.com:owner/repo.git`,
/// `https://github.com/owner/repo`).
fn owner(url: &str) -> Option<&str> {
    let path = url.split_once("github.com")?.1.trim_start_matches([':', '/']);
    path.split('/').next().filter(|o| !o.is_empty())
}

/// Publish replacement `r` (checked out in `dir`) as a draft PR (ready for review once CI
/// passes), with `title` and `body` (what
/// you made of Claude's drafts): rebase it onto its base as it is on origin now, push it to
/// your replacements remote, then open the PR (from there, into origin's repo). Its number.
pub fn publish(r: &mut Record, dir: &Path, title: &str, body: &str) -> Result<u64> {
    anyhow::ensure!(r.published.is_none(), "already published, as #{}", r.published.unwrap_or(0));
    let remote = remote().context("no remote for replacement branches (dv.replaceRemote)")?;
    rebase(r, dir)?;
    let b = branch(r.id);
    let url = crate::diff::git(&["remote", "get-url", &remote])?;
    let owner = owner(url.trim()).with_context(|| format!("{remote} ({}) isn't a GitHub repo", url.trim()))?;
    crate::diff::git(&["-C", &dir.to_string_lossy(), "push", "-q", "-f", "-u", &remote, &format!("{b}:refs/heads/{b}")])?;
    let n = crate::gh::create_pr(&r.base, &format!("{owner}:{b}"), title, body)?;
    (r.published, r.title, r.body) = (Some(n), title.to_owned(), body.to_owned());
    save(r)?;
    Ok(n)
}

/// The closing note, with the replacement's PR where Claude left `{replacement}` (or, if it
/// didn't, said at the end).
pub fn note(r: &Record) -> String {
    let link = r.published.map_or("the replacement".to_owned(), |n| format!("#{n}"));
    match r.closing_note.contains("{replacement}") {
        true => r.closing_note.replace("{replacement}", &link),
        false => format!("{}\n\nSuperseded by {link}.", r.closing_note.trim_end()).trim_start().to_owned(),
    }
}

/// Close the originals not closed yet, each with `note`: what was closed (an error stops at
/// the PR it's about, keeping those before).
pub fn close_originals(r: &mut Record, note: &str) -> Result<Vec<u64>> {
    let mut done = vec![];
    for n in r.replaces.iter().map(|o| o.number).filter(|n| !r.closed.contains(n)).collect::<Vec<_>>() {
        let res = crate::gh::close(n, Some(note)).with_context(|| format!("closing #{n}"));
        if res.is_ok() {
            r.closed.push(n);
            done.push(n);
        }
        save(r)?;
        res?;
    }
    Ok(done)
}

/// Remove replacement `id`: its worktree (not with local changes) and its record. Its branch
/// stays, in case there's something on it you want.
pub fn remove(id: u64, root: &Path) -> Result<String> {
    let dir = crate::worktree_dir_named(root, &name(id))?;
    if dir.exists() {
        crate::ensure_worktree_of(&dir, root)?;
        crate::diff::git(&["-C", &root.to_string_lossy(), "worktree", "remove", &dir.to_string_lossy()]).map_err(|e| {
            match format!("{e:#}").contains("modified or untracked") {
                true => e.context(format!("{} has local changes: commit or discard them first", dir.display())),
                false => e,
            }
        })?;
    }
    _ = std::fs::remove_file(path(id)?);
    let b = branch(id);
    Ok(format!("removed {}; kept branch {b} (`git branch -D {b}` deletes it)", label(id)))
}

/// `dv replace-info`: what Claude needs to draft the replacement checked out here.
pub fn info() -> Result<()> {
    let id = current().context("not on a replacement's branch (dv/replace-N) or a new PR's (dv/new-K)")?;
    let r = load(id)?;
    let base = crate::diff::merge_base(Some(&crate::diff::remote_branch(&r.base)))?;
    match is_new(id) {
        true => {
            println!("record: {}    (write title and body here)", path(id)?.display());
            println!("task: {}", r.task.replace('\n', "\n      "));
        }
        false => println!("record: {}    (write title, body and closing_note here)", path(id)?.display()),
    }
    for o in &r.replaces {
        println!("replaces: #{} {} (@{}) {}", o.number, o.title, o.author, o.url);
    }
    println!("branch: {}    (commit your work here; never push)", branch(id));
    println!("base: {base}    ({}: `git diff {base}` is what dv shows)", crate::diff::remote_branch(&r.base));
    println!("notes: the user's notes on this repo are in your prompt (read-only for a replacement)");
    Ok(())
}

#[test]
fn records_round_trip() {
    let r = Record {
        id: 12,
        replaces: vec![Original { number: 12, title: "fix it".into(), author: "bot".into(), url: "u".into() }, Original { number: 15, ..Original::default() }],
        base: "master".into(),
        title: "Fix it properly".into(),
        body: "Replaces #12 and #15.\n\nFixes #9.".into(),
        closing_note: "Thanks! Handled in the replacement.".into(),
        session: "s".into(),
        published: Some(20),
        closed: vec![12],
        drafting: Some(7),
        task: String::new(),
        asks: vec![
            Request { text: "shorter title".into(), want: "change".into(), on: "description".into(), answer: "Done.".into() },
            Request { text: "mention #9".into(), ..Request::default() },
        ],
        taking: 1,
    };
    assert_eq!(parse(12, &to_json(&r)).unwrap(), r);
    // Only what no redraft has taken in yet is asked again.
    assert!(prompt(&r, "").ends_with("`asks[1]`: mention #9") && !prompt(&r, "").contains("asks[0]"));
    // As Claude may leave it: fields missing are empty.
    let bare = parse(3, r#"{ "replaces": [{ "number": 3 }], "base": "main" }"#).unwrap();
    assert_eq!((bare.replaces[0].number, bare.title.as_str(), bare.base.as_str()), (3, "", "main"));
    assert!(prompt(&r, "keep the old API").contains("PR #12, #15") && prompt(&r, "keep the old API").ends_with("The user adds:\nkeep the old API"));
    let r = Record { closing_note: "Thanks! See {replacement}.".into(), ..r };
    assert_eq!(note(&r), "Thanks! See #20.");
    assert_eq!(note(&Record { published: None, ..r.clone() }), "Thanks! See the replacement.");
    // Without the placeholder, the link goes at the end.
    assert_eq!(note(&Record { closing_note: "Thanks!".into(), ..r.clone() }), "Thanks!\n\nSuperseded by #20.");
    assert_eq!(owner("git@github.com:evan/diffview.git"), Some("evan"));
    assert_eq!(owner("https://github.com/evan/diffview"), Some("evan"));
    assert_eq!(owner("/srv/git/x.git"), None);
    // A draft from a process that's gone isn't under way.
    assert!(Record { drafting: Some(std::process::id()), ..r.clone() }.drafting_now());
    assert!(!Record { drafting: None, ..r }.drafting_now());
}

#[test]
fn new_prs() {
    let id = NEW + 3;
    assert!(is_new(id) && !is_new(2390));
    assert_eq!((name(id), branch(id), shown(id), label(id)), ("new-3".into(), "dv/new-3".into(), "new 3".into(), "new PR 3".into()));
    assert_eq!((name(12), branch(12), shown(12)), ("replace-12".into(), "dv/replace-12".into(), "#12".into()));
    // Its record: no PRs it replaces, its task kept; the prompt asks for a new PR doing it.
    let r = parse(id, r#"{ "base": "master", "task": "Fix #9: the panic on empty input" }"#).unwrap();
    assert!(r.replaces.is_empty() && r.task.starts_with("Fix #9"));
    assert_eq!(parse(id, &to_json(&r)).unwrap(), r);
    let p = prompt(&r, "");
    assert!(p.contains("draft a new PR") && p.ends_with("Fix #9: the panic on empty input"), "{p}");
}
