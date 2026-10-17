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
    let st = crate::view::claude_command(&prompt, &dir, &session, &[], None)
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

/// Claude drafts one replacement for `prs` (see `replace.rs`) in the background: its news
/// comes back on the receiver, under the replacement's number (the first PR's).
pub fn replace(prs: Vec<u64>, root: PathBuf, extra: String) -> Receiver<(u64, Status)> {
    let (tx, rx) = mpsc::channel();
    let id = prs[0];
    _ = tx.send((id, Status::Preparing));
    std::thread::spawn(move || {
        let ready = tx.clone();
        let st = crate::replace::draft(&prs, &root, &extra, move || _ = ready.send((id, Status::Reviewing)));
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
    std::thread::spawn(move || _ = tx.send(suggest_groups(&prs, &root)));
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
    let out = crate::view::claude_command(&prompt, root, &[], &tools, Some(root)).stdin(std::process::Stdio::null()).output()?;
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
    anyhow::ensure!(v["is_error"] == false, "Claude couldn't group them ({})", out.status);
    parse_groups(v["result"].as_str().unwrap_or(""))
}

/// The groups in Claude's answer (a JSON array, maybe with words around it).
fn parse_groups(answer: &str) -> anyhow::Result<Groups> {
    use anyhow::Context;
    let json = answer.get(answer.find('[').context("no groups in Claude's answer")?..=answer.rfind(']').context("no groups")?).unwrap_or("");
    let v: serde_json::Value = serde_json::from_str(json)?;
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
    let Ok(entries) = std::fs::read_dir(dir.join("reviews")) else { return Default::default() };
    entries
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.strip_prefix("pr-")?.strip_suffix(".json")?.parse().ok())
        .collect()
}
