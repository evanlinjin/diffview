//! Batch reviews from `dv list`: Claude reviews each marked PR in the background, in the PR's
//! worktree (`<repo>-N`, as `dv N -w` makes it), a couple at a time. Each gets the PR's own
//! session and review file, so opening the PR in dv later picks up where it left off.
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

/// How many PRs Claude reviews at once.
pub const AT_ONCE: usize = 2;

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Queued,
    Preparing,
    Reviewing,
    /// What it took ("5 turns, $0.22").
    Done(String),
    Failed(String),
}

/// What Claude does on a PR: review it (or update the review it has: the prompt says to), update
/// the review from the commit it reviewed, or answer your replies in it (`theirs`: someone else's
/// PR, so suggestions, not edits).
#[derive(Clone, Debug, PartialEq)]
pub enum Task {
    Review,
    Update(String),
    Answer { theirs: bool },
    /// Your PR's CI fails: find out why, fix it in the checkout (you push).
    FixChecks,
    /// Claude's review would close it: draft the comment it closes with (you close it, in dv).
    Close,
    /// Your PR changed since this commit: propose a new description if it no longer fits
    /// (you approve it, in dv).
    Describe(String),
}

impl Task {
    /// Its `Work` (its model: `dv.model.<key>`), as the PR view's same asks have.
    pub fn work(&self) -> crate::claude::Work {
        use crate::claude::Work;
        match self {
            Task::Review | Task::Update(_) => Work::Review,
            Task::Answer { .. } => Work::Answer,
            Task::FixChecks => Work::FixCi,
            Task::Close => Work::Close,
            Task::Describe(_) => Work::Describe,
        }
    }

    /// For the cards and messages: "reviewing", "updating its review", "answering".
    pub fn doing(&self) -> &'static str {
        match self {
            Task::Review => "reviewing",
            Task::Update(_) => "updating its review",
            Task::Answer { .. } => "answering",
            Task::FixChecks => "fixing CI",
            Task::Close => "drafting a closing comment",
            Task::Describe(_) => "checking its description",
        }
    }

    /// For a card, before it's asked: "could review", "could update its review", "could answer you".
    pub fn could(&self) -> &'static str {
        match self {
            Task::Review => "could review",
            Task::Update(_) => "could update its review",
            Task::Answer { .. } => "could answer you",
            Task::FixChecks => "could fix CI",
            Task::Close => "could draft a closing comment",
            Task::Describe(_) => "could check its description",
        }
    }

    fn prompt(&self, n: u64) -> String {
        let what = format!("PR #{n}");
        match self {
            Task::Review => crate::claude::review_prompt(&what),
            Task::Update(old) => crate::claude::update_prompt(&what, old),
            Task::Answer { theirs } => crate::claude::answer_prompt(&what, *theirs),
            Task::FixChecks => crate::claude::fix_checks_prompt(n),
            Task::Close => format!(
                "Use the dv-review skill to draft a comment closing {what}, in the review file's top-level \"closing\" \
                 field (see \"Draft a closing comment\"): your review says it should be closed. Don't close the PR or \
                 comment on GitHub: the user closes it in dv."
            ),
            Task::Describe(old) => format!(
                "{what} has changed since {old} (`git diff {old} HEAD`; that commit gone, a force-push: the whole PR, \
                 `gh pr diff {n}`). Read its description (`gh pr view {n} --json body`). \
                 If it no longer describes the PR (what it changes now, why, how it's tested, anything in it now wrong \
                 or missing), use the dv-review skill to propose a new one in the review file's top-level \
                 \"description\" field (see \"Propose a description\"), changing only what needs to, keeping the rest. \
                 If it still fits, change nothing. Don't edit the PR on GitHub: the user approves it in dv."
            ),
        }
    }
}

impl Status {
    pub fn running(&self) -> bool {
        matches!(self, Status::Queued | Status::Preparing | Status::Reviewing)
    }
}

/// Claude does `task` on each of `prs` (PR number, whether its worktree is on the PR's branch,
/// the task) from the repo at `root`, with the user's `extra` instructions (if any); progress
/// comes back on the receiver.
pub fn start(prs: Vec<(u64, bool, Task)>, root: PathBuf, extra: String) -> Receiver<(u64, Status)> {
    pool(prs, move |(n, branch, task), tx| (*n, run(*n, *branch, task, &root, &extra, tx).unwrap_or_else(|e| Status::Failed(format!("{e:#}")))))
}

/// Something to do (a PR to review, an issue to triage): its number first.
pub trait Numbered {
    fn number(&self) -> u64;
}

impl Numbered for u64 {
    fn number(&self) -> u64 {
        *self
    }
}

impl Numbered for (u64, String) {
    fn number(&self) -> u64 {
        self.0
    }
}

impl Numbered for (u64, bool, Task) {
    fn number(&self) -> u64 {
        self.0
    }
}

/// `items` done by `work` (which may say how it's going on the sender), `AT_ONCE` at a time,
/// in the background: each queued first, then its news and how it ended, by number.
pub fn pool<T: Numbered + Send + 'static>(items: Vec<T>, work: impl Fn(&T, &Sender<(u64, Status)>) -> (u64, Status) + Send + Sync + 'static) -> Receiver<(u64, Status)> {
    let (tx, rx) = mpsc::channel();
    items.iter().for_each(|i| _ = tx.send((i.number(), Status::Queued)));
    let (queue, work) = (Arc::new(Mutex::new(VecDeque::from(items))), Arc::new(work));
    for _ in 0..AT_ONCE {
        let (queue, tx, work) = (queue.clone(), tx.clone(), work.clone());
        std::thread::spawn(move || {
            while let Some(item) = queue.lock().ok().and_then(|mut q| q.pop_front()) {
                // (A panic in one: it failed, and the worker goes on to the next.)
                let st = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&item, &tx)));
                _ = tx.send(st.unwrap_or_else(|p| (item.number(), Status::Failed(stopped(&p)))));
            }
        });
    }
    rx
}

fn run(n: u64, branch: bool, task: &Task, root: &Path, extra: &str, tx: &Sender<(u64, Status)>) -> anyhow::Result<Status> {
    use anyhow::Context;
    _ = tx.send((n, Status::Preparing));
    // The worktree, checked out at the PR (by a dv of its own: that changes directory).
    let mut prep = std::process::Command::new(crate::self_exe());
    prep.args(["prepare-worktree", &n.to_string()]).current_dir(root);
    if branch {
        prep.arg("-b");
    }
    let out = prep.stdin(std::process::Stdio::null()).output()?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("{}", err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("preparing the worktree failed").trim());
    }
    let dir = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    // The PR's session in that checkout, as dv there would resume it (a new one, if it has
    // none, or one from another version of the skill).
    let review = crate::claude::path(Some(n))?;
    let session = crate::claude::session_args(&review, &dir)?;
    _ = tx.send((n, Status::Reviewing));
    let prompt = format!("{} (It's {} now.)", task.prompt(n), crate::claude::now());
    let prompt = crate::claude::with_instructions(&prompt, extra);
    let log = std::env::temp_dir().join(format!("dv-claude-batch-{}-{n}.log", std::process::id()));
    let more = crate::claude::tools_for(matches!(task, Task::FixChecks).then_some(n), matches!(task, Task::Answer { .. }));
    let st = crate::claude::claude_command(task.work(), &prompt, &dir, &session, &more.iter().map(String::as_str).collect::<Vec<_>>(), None)
        .env(crate::claude::PR_ENV, n.to_string())
        .stdin(std::process::Stdio::null())
        .stdout(std::fs::File::create(&log)?)
        .stderr(std::fs::File::create(log.with_extension("err"))?)
        .status()
        .context("is Claude Code (`claude`) installed?")?;
    let v: serde_json::Value = std::fs::read_to_string(&log).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    if !st.success() || v["is_error"] != false {
        _ = crate::claude::save_session(&review, &dir, None); // start afresh next time
        anyhow::bail!("Claude stopped ({st}): see {}", log.display());
    }
    Ok(Status::Done(format!(
        "{} turns, ${:.2}",
        v["num_turns"].as_u64().unwrap_or(0),
        v["total_cost_usd"].as_f64().unwrap_or(0.0)
    )))
}

/// `f`'s result, or a failure if it panics: a background run always says how it ended (its
/// view counts it as under way till then: dv would ask before quitting forever).
pub fn guarded<T>(f: impl FnOnce() -> anyhow::Result<T>) -> anyhow::Result<T> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or_else(|p| Err(anyhow::anyhow!(stopped(&p))))
}

/// What a panic said: "dv hit a bug: …".
pub(crate) fn panicked(p: &Box<dyn std::any::Any + Send>) -> String {
    let said = p.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| p.downcast_ref::<String>().cloned()).unwrap_or_default();
    format!("dv hit a bug: {said}")
}

/// A background run's failure from its panic.
fn stopped(p: &Box<dyn std::any::Any + Send>) -> String {
    format!("its run stopped ({})", panicked(p))
}

/// Claude drafts one replacement for `prs` (see `replace.rs`) in the background: its news
/// comes back on the receiver, under the replacement's number (the first PR's).
pub fn replace(prs: Vec<u64>, root: PathBuf, extra: String) -> Receiver<(u64, Status)> {
    let (tx, rx) = mpsc::channel();
    let id = prs[0];
    _ = tx.send((id, Status::Preparing));
    std::thread::spawn(move || {
        let ready = tx.clone();
        // (Its run guarded inside, the record's mark coming off; and all of it, here.)
        let st = guarded(|| crate::replace::draft(&prs, &root, &extra, move || _ = ready.send((id, Status::Reviewing))));
        _ = tx.send((id, st.map_or_else(|e| Status::Failed(format!("{e:#}")), Status::Done)));
    });
    rx
}

/// A replacement each: the PRs it replaces, and why they go together (Claude's word).
pub type Groups = Vec<(Vec<u64>, String)>;

/// Claude suggests which of `prs` (number, title, author, base) one replacement should cover
/// together (attempts at the same change) and which stand alone, in the background. It only
/// reads them.
pub fn group(prs: Vec<(u64, String, String, String)>, root: PathBuf) -> Receiver<anyhow::Result<Groups>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || _ = tx.send(guarded(|| suggest_groups(&prs, &root))));
    rx
}

fn suggest_groups(prs: &[(u64, String, String, String)], root: &Path) -> anyhow::Result<Groups> {
    let list: Vec<String> = prs.iter().map(|(n, t, a, b)| format!("- #{n} {t} (@{a}, into {b})")).collect();
    let prompt = format!(
        "These PRs are to be replaced by better ones:\n{}\n\nRead each (`gh pr view N`, `gh pr diff N`) and group \
         them: PRs that try to make the same change (the same issue, the same code) go in one group, to be replaced \
         by one PR; the others each alone. PRs into different base branches never share a group. Change nothing. \
         Reply with only a JSON array, no other text: [{{\"prs\": [12, 15], \"why\": \"one short line\"}}], \
         every PR in exactly one group.",
        list.join("\n")
    );
    let tools = ["Bash(gh pr view:*)", "Bash(gh pr diff:*)"];
    let out = crate::claude::claude_reader(crate::claude::Work::Group, &prompt, root, &tools).stdin(std::process::Stdio::null()).output()?;
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
    anyhow::ensure!(v["is_error"] == false, "Claude couldn't group them ({})", out.status);
    parse_groups(v["result"].as_str().unwrap_or(""))
}

/// The groups in Claude's answer (a JSON array, maybe with words around it).
fn parse_groups(answer: &str) -> anyhow::Result<Groups> {
    use anyhow::Context;
    let v = crate::claude::json_in(answer, '[').filter(serde_json::Value::is_array).context("no groups in Claude's answer")?;
    Ok(v.as_array()
        .context("no groups")?
        .iter()
        .map(|g| (g["prs"].as_array().into_iter().flatten().filter_map(serde_json::Value::as_u64).collect(), g["why"].as_str().unwrap_or("").to_owned()))
        .filter(|g: &(Vec<u64>, String)| !g.0.is_empty())
        .collect())
}

/// Groups as you edit them: a line each, the PRs' numbers, `#` and after it a note.
pub fn groups_text(groups: &Groups) -> String {
    groups.iter().map(|(prs, why)| {
        let nums: Vec<String> = prs.iter().map(u64::to_string).collect();
        format!("{:<14} # {why}\n", nums.join(" "))
    }).collect()
}

pub fn parse_groups_text(text: &str) -> Vec<Vec<u64>> {
    text.lines()
        .map(|l| l.split('#').next().unwrap_or("").split_whitespace().filter_map(|n| n.trim_start_matches('#').parse().ok()).collect::<Vec<u64>>())
        .filter(|g| !g.is_empty())
        .collect()
}

#[test]
fn groups() {
    let g = parse_groups("Here: [{\"prs\": [12, 15], \"why\": \"same fix\"}, {\"prs\": [20], \"why\": \"alone\"}]").unwrap();
    assert_eq!(g, vec![(vec![12, 15], "same fix".to_owned()), (vec![20], "alone".to_owned())]);
    let text = groups_text(&g);
    assert_eq!(text, "12 15          # same fix\n20             # alone\n");
    assert_eq!(parse_groups_text(&format!("{text}\n21 22 # mine")), [vec![12, 15], vec![20], vec![21, 22]]);
}

/// The PRs that have a review from Claude (a review file in dv's directory).
pub fn reviewed() -> std::collections::HashSet<u64> {
    let Ok(dir) = crate::claude::dv_dir() else { return Default::default() };
    reviewed_in(&dir.join("reviews"))
}

/// The PRs with a review in `dir`.
pub fn reviewed_in(dir: &std::path::Path) -> std::collections::HashSet<u64> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Default::default() };
    entries
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.strip_prefix("pr-")?.strip_suffix(".json")?.parse().ok())
        .collect()
}

#[test]
fn a_run_that_panics_says_so() {
    assert_eq!(guarded(|| Ok(1)).unwrap(), 1);
    let e = guarded::<()>(|| panic!("boom")).unwrap_err();
    assert!(format!("{e}").contains("its run stopped (dv hit a bug: boom)"));
    // A worker's panic: that one failed, the next still runs.
    let rx = pool(vec![1u64, 2], |&n, _| if n == 1 { panic!("one") } else { (n, Status::Done("ok".into())) });
    let ends: Vec<(u64, Status)> = rx.iter().filter(|(_, s)| !matches!(s, Status::Queued)).collect();
    assert!(ends.iter().any(|(n, s)| *n == 1 && matches!(s, Status::Failed(e) if e.contains("one"))));
    assert!(ends.iter().any(|(n, s)| *n == 2 && matches!(s, Status::Done(_))));
}
