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

impl Status {
    pub fn running(&self) -> bool {
        matches!(self, Status::Queued | Status::Preparing | Status::Reviewing)
    }
}

/// Review `prs` (PR number, and whether its worktree is on the PR's branch) from the repo at
/// `root`, with the user's `extra` instructions (if any); progress comes back on the receiver.
pub fn start(prs: Vec<(u64, bool)>, root: PathBuf, extra: String) -> Receiver<(u64, Status)> {
    let (tx, rx) = mpsc::channel();
    for &(n, _) in &prs {
        _ = tx.send((n, Status::Queued));
    }
    let queue = Arc::new(Mutex::new(VecDeque::from(prs)));
    for _ in 0..AT_ONCE {
        let (queue, tx, root, extra) = (queue.clone(), tx.clone(), root.clone(), extra.clone());
        std::thread::spawn(move || {
            while let Some((n, branch)) = queue.lock().ok().and_then(|mut q| q.pop_front()) {
                let status = review(n, branch, &root, &extra, &tx).unwrap_or_else(|e| Status::Failed(format!("{e:#}")));
                _ = tx.send((n, status));
            }
        });
    }
    rx
}

fn review(n: u64, branch: bool, root: &Path, extra: &str, tx: &Sender<(u64, Status)>) -> anyhow::Result<Status> {
    use anyhow::Context;
    _ = tx.send((n, Status::Preparing));
    // The worktree, checked out at the PR (by a dv of its own: that changes directory).
    let mut prep = std::process::Command::new(std::env::current_exe()?);
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
    let session = match crate::claude::load_session(&review, &dir) {
        Some((id, true)) => ["--resume".to_owned(), id],
        _ => {
            let id = crate::claude::new_session_id();
            crate::claude::save_session(&review, &dir, Some(&id))?;
            ["--session-id".to_owned(), id]
        }
    };
    _ = tx.send((n, Status::Reviewing));
    let prompt = format!("{} (It's {} now.)", crate::view::review_prompt(&format!("PR #{n}")), crate::claude::now());
    let prompt = crate::view::with_instructions(&prompt, extra);
    let log = std::env::temp_dir().join(format!("dv-claude-batch-{}-{n}.log", std::process::id()));
    let st = crate::view::claude_command(&prompt, &dir, &session)
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

/// The PRs that have a review from Claude (a review file in dv's directory).
pub fn reviewed() -> std::collections::HashSet<u64> {
    let Ok(dir) = crate::claude::dv_dir() else { return Default::default() };
    let Ok(entries) = std::fs::read_dir(dir.join("reviews")) else { return Default::default() };
    entries
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.strip_prefix("pr-")?.strip_suffix(".json")?.parse().ok())
        .collect()
}
