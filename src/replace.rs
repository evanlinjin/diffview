//! Replacements: PRs Claude drafts locally to replace others (low-effort ones, say), kept
//! unpublished until you've reviewed them. Each is a branch `dv/replace-N` (N: the first PR it
//! replaces) in a worktree `<repo>-replace-N`, and a record in dv's directory,
//! `replacements/N.json`: the PRs it replaces, its base, and Claude's drafts of its title and
//! description and of the note closing the originals.
use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

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
}

impl Record {
    /// Claude is drafting it now (from any dv): its process is still running.
    pub fn drafting_now(&self) -> bool {
        self.drafting.is_some_and(|pid| {
            let alive = std::process::Command::new("kill").args(["-0", &pid.to_string()]).stderr(std::process::Stdio::null()).status();
            alive.is_ok_and(|s| s.success())
        })
    }
}

pub fn branch(id: u64) -> String {
    format!("dv/replace-{id}")
}

/// The replacement checked out here, by its branch.
pub fn current() -> Option<u64> {
    crate::diff::git(&["branch", "--show-current"]).ok()?.trim().strip_prefix("dv/replace-")?.parse().ok()
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
    let replaces = v["replaces"].as_array().context("no \"replaces\"")?;
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
    })
}

fn to_json(r: &Record) -> String {
    let replaces: Vec<Value> =
        r.replaces.iter().map(|o| json!({ "number": o.number, "title": o.title, "author": o.author, "url": o.url })).collect();
    let v = json!({
        "replaces": replaces, "base": r.base, "title": r.title, "body": r.body,
        "closing_note": r.closing_note, "session": r.session, "published": r.published, "closed": r.closed,
        "drafting": r.drafting,
    });
    serde_json::to_string_pretty(&v).unwrap_or_default() + "\n"
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

/// Set up a replacement of `prs` (the first names it) from the repo at `root`: its record
/// (kept if it exists: drafting again carries on), and its branch, from the originals' base,
/// in its worktree. Returns the record and the worktree.
pub fn prepare(prs: &[u64], root: &Path) -> Result<(Record, PathBuf)> {
    let id = *prs.first().context("no PR to replace")?;
    let git = |args: &[&str]| crate::diff::git(&[&["-C", &root.to_string_lossy()], args].concat());
    let mut r = load(id).unwrap_or(Record { id, ..Record::default() });
    anyhow::ensure!(!r.drafting_now(), "Claude is already drafting the replacement of #{id}");
    let mut originals = vec![];
    let mut bases: Vec<(u64, String)> = vec![];
    for &n in prs {
        let (title, author, url, base) = crate::gh::brief(n)?;
        bases.push((n, base));
        originals.push(Original { number: n, title, author, url });
    }
    // One replacement can only target one branch.
    if let Some((n, b)) = bases.iter().find(|b| b.1 != bases[0].1) {
        anyhow::bail!("#{} targets {}, #{n} targets {b}: one replacement can't replace both", bases[0].0, bases[0].1);
    }
    r.base = bases.first().map(|b| b.1.clone()).unwrap_or_default();
    r.replaces = originals;
    if r.session.is_empty() {
        r.session = crate::claude::new_session_id();
    }
    let dir = crate::worktree_dir_named(root, &format!("replace-{id}"))?;
    if dir.exists() {
        crate::ensure_worktree_of(&dir, root)?;
        // Carrying on: from the base as it is now, if that goes cleanly (else Claude sees why).
        _ = rebase(&r, &dir);
    } else {
        _ = git(&["fetch", "-q", "origin", &r.base]);
        let (b, d) = (branch(id), dir.to_string_lossy().into_owned());
        match git(&["rev-parse", "--verify", "-q", &format!("refs/heads/{b}")]) {
            Ok(_) => git(&["worktree", "add", &d, &b])?,
            Err(_) => git(&["worktree", "add", "-b", &b, &d, &format!("origin/{}", r.base)])?,
        };
    }
    save(&r)?;
    Ok((r, dir))
}

/// Bring replacement `r`'s branch (in `dir`) up to its base on origin: fetch, then rebase onto
/// it. A rebase that conflicts is undone, and said so.
pub fn rebase(r: &Record, dir: &Path) -> Result<()> {
    let git = |args: &[&str]| crate::diff::git(&[&["-C", &dir.to_string_lossy()], args].concat());
    git(&["fetch", "-q", "origin", &r.base])?;
    if let Err(e) = git(&["rebase", "-q", &format!("origin/{}", r.base)]) {
        _ = git(&["rebase", "--abort"]);
        anyhow::bail!("it doesn't rebase cleanly onto origin/{} ({e:#}): ask Claude to redraft it on the new base", r.base);
    }
    Ok(())
}

/// What Claude is asked, to draft (or redraft) replacement `r`. Your notes on the repo come
/// with it, to read: a draft reads untrusted PRs, so it gets no access to them (nor to dv's
/// other files), only to its record.
pub fn prompt(r: &Record, extra: &str) -> String {
    let prs: Vec<String> = r.replaces.iter().map(|o| format!("#{}", o.number)).collect();
    let notes = crate::claude::notes_path().ok().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
    let notes = match notes.trim() {
        "" => String::new(),
        n => format!("\n\nThe user's notes on this repo (read-only here; follow them):\n<notes>\n{n}\n</notes>"),
    };
    let p = format!(
        "Use the dv-review skill to draft a replacement for PR {} (its section 6: \"Draft a replacement\"). \
         First run `dv replace-info`. (It's {} now.){notes}",
        prs.join(", "),
        crate::claude::now()
    );
    crate::view::with_instructions(&p, extra)
}

/// The directory a draft may write to (its record): `--add-dir`.
pub fn writable() -> Result<PathBuf> {
    dir()
}

/// Draft replacement `prs` with Claude, in its worktree, in the foreground of this thread:
/// what it took ("12 turns, $0.40").
pub fn draft(prs: &[u64], root: &Path, extra: &str, prepared: impl FnOnce()) -> Result<String> {
    let (mut r, dir) = prepare(prs, root)?;
    // Others (dv list, another dv) see it's under way, and don't start another.
    r.drafting = Some(std::process::id());
    save(&r)?;
    let res = run_claude(&r, &dir, extra, prepared);
    // Claude has written to the record meanwhile: only the mark comes off.
    if let Ok(mut now) = load(r.id) {
        now.drafting = None;
        _ = save(&now);
    }
    res
}

fn run_claude(r: &Record, dir: &Path, extra: &str, prepared: impl FnOnce()) -> Result<String> {
    prepared();
    // A record that exists from before has a session that saw it: carry on there.
    let fresh = r.title.is_empty() && r.body.is_empty();
    let session = match fresh {
        true => ["--session-id".to_owned(), r.session.clone()],
        false => ["--resume".to_owned(), r.session.clone()],
    };
    let log = std::env::temp_dir().join(format!("dv-claude-replace-{}-{}.log", std::process::id(), r.id));
    let st = crate::view::claude_command(&prompt(r, extra), dir, &session, crate::view::REPLACE_TOOLS, Some(&writable()?))
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
        .filter(|n| *n != "origin")
        .map(|n| (n.to_owned(), crate::diff::git(&["remote", "get-url", n]).unwrap_or_default().trim().to_owned()))
        .collect()
}

/// The GitHub owner of a remote's repo, from its URL (`git@github.com:owner/repo.git`,
/// `https://github.com/owner/repo`).
fn owner(url: &str) -> Option<&str> {
    let path = url.split_once("github.com")?.1.trim_start_matches([':', '/']);
    path.split('/').next().filter(|o| !o.is_empty())
}

/// Publish replacement `r` (checked out in `dir`) as a PR (a draft, unless `ready` for
/// review), with `title` and `body` (what
/// you made of Claude's drafts): rebase it onto its base as it is on origin now, push it to
/// your replacements remote, then open the PR (from there, into origin's repo). Its number.
pub fn publish(r: &mut Record, dir: &Path, title: &str, body: &str, ready: bool) -> Result<u64> {
    anyhow::ensure!(r.published.is_none(), "already published, as #{}", r.published.unwrap_or(0));
    let remote = remote().context("no remote for replacement branches (dv.replaceRemote)")?;
    rebase(r, dir)?;
    let b = branch(r.id);
    let url = crate::diff::git(&["remote", "get-url", &remote])?;
    let owner = owner(url.trim()).with_context(|| format!("{remote} ({}) isn't a GitHub repo", url.trim()))?;
    crate::diff::git(&["-C", &dir.to_string_lossy(), "push", "-q", "-f", "-u", &remote, &format!("{b}:refs/heads/{b}")])?;
    let n = crate::gh::create_pr(&r.base, &format!("{owner}:{b}"), title, body, ready)?;
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
        let res = crate::gh::close_with(n, note).with_context(|| format!("closing #{n}"));
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
    let dir = crate::worktree_dir_named(root, &format!("replace-{id}"))?;
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
    Ok(format!("removed the replacement of #{id}; kept branch {b} (`git branch -D {b}` deletes it)"))
}

/// `dv replace-info`: what Claude needs to draft the replacement checked out here.
pub fn info() -> Result<()> {
    let id = current().context("not on a replacement's branch (dv/replace-N)")?;
    let r = load(id)?;
    let base = crate::diff::merge_base(Some(&format!("origin/{}", r.base)))?;
    println!("record: {}    (write title, body and closing_note here)", path(id)?.display());
    for o in &r.replaces {
        println!("replaces: #{} {} (@{}) {}", o.number, o.title, o.author, o.url);
    }
    println!("branch: {}    (commit your work here; never push)", branch(id));
    println!("base: {base}    (origin/{}: `git diff {base}` is what dv shows)", r.base);
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
    };
    assert_eq!(parse(12, &to_json(&r)).unwrap(), r);
    // As Claude may leave it: fields missing are empty.
    let bare = parse(3, r#"{ "replaces": [{ "number": 3 }], "base": "main" }"#).unwrap();
    assert_eq!((bare.replaces[0].number, bare.title.as_str(), bare.base.as_str()), (3, "", "main"));
    assert!(prompt(&r, "keep the old API").contains("PR #12, #15") && prompt(&r, "keep the old API").ends_with("keep the old API"));
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
