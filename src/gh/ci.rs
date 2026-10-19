//! CI on GitHub: a PR's checks, its failed runs rerun, runs waiting for a maintainer's
//! approval, and marking it ready for review.
use super::{gh, gh_json};
use anyhow::{bail, Result};
use serde_json::Value;

/// PR `n`'s CI checks on GitHub now: (passed, failed, pending).
pub fn checks(n: u64) -> Result<(usize, usize, usize)> {
    let v: Value = gh_json(&["pr", "view", &n.to_string(), "--json", "statusCheckRollup"])?;
    Ok(tally(v["statusCheckRollup"].as_array().map(Vec::as_slice).unwrap_or(&[])))
}

/// Mark draft PR `n` ready for review (`gh pr ready`): only once CI passes, nothing failing
/// or still running (a PR with no checks has nothing to wait for).
pub fn mark_ready(n: u64) -> Result<()> {
    let (_, fail, pending) = checks(n)?;
    match (fail, pending) {
        (0, 0) => gh(&["pr", "ready", &n.to_string()]).map(drop),
        (0, k) => bail!("CI is still running ({k} check{} pending): ready once it passes", if k == 1 { "" } else { "s" }),
        (k, _) => bail!("CI fails ({k} check{} failed): fix it first (dv list: a a has Claude look)", if k == 1 { "" } else { "s" }),
    }
}

/// How many of `checks` passed, failed, are pending.
pub(crate) fn tally(checks: &[Value]) -> (usize, usize, usize) {
    let count = |m| checks.iter().filter(|c| verdict(c) == m).count();
    (count('✓'), count('✗'), count('●'))
}

/// A CI check's outcome: ✓ passed, ✗ failed, ● pending. CheckRun has status/conclusion;
/// StatusContext has state.
pub(crate) fn verdict(c: &Value) -> char {
    match (c["status"].as_str(), c["conclusion"].as_str().or(c["state"].as_str())) {
        (Some(st), _) if st != "COMPLETED" => '●',
        (_, Some("SUCCESS" | "NEUTRAL" | "SKIPPED")) => '✓',
        (_, Some("PENDING" | "EXPECTED")) | (_, None) => '●',
        _ => '✗',
    }
}

/// Rerun the failed jobs of the workflow runs on commit `head` that failed, four at a time:
/// what happened, in a few words (an error if none could be).
pub fn rerun_failed(head: &str) -> Result<String> {
    let q = ".[] | select(.conclusion == \"failure\" or .conclusion == \"timed_out\" or .conclusion == \"cancelled\") | .databaseId";
    let out = gh(&["run", "list", "--commit", head, "--limit", "100", "--json", "databaseId,conclusion", "-q", q])?;
    let ids: Vec<&str> = out.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    if ids.is_empty() {
        return Ok("no failed runs to rerun (they may be running already)".into());
    }
    // A few at a time (not GitHub's limit on many writes at once); one that can't be (too
    // old, say) doesn't keep the others from it.
    let mut errors = vec![];
    for batch in ids.chunks(4) {
        std::thread::scope(|s| {
            let runs: Vec<_> = batch.iter().map(|id| s.spawn(move || gh(&["run", "rerun", id, "--failed"]).err().map(|e| format!("run {id}: {e:#}")))).collect();
            errors.extend(runs.into_iter().filter_map(|r| r.join().unwrap_or_else(|_| Some("a rerun went wrong".into()))));
        });
    }
    rerun_says(ids.len(), &errors)
}

/// What rerunning `total` runs came to, `errors` the ones that couldn't be.
pub(crate) fn rerun_says(total: usize, errors: &[String]) -> Result<String> {
    let rerun = total - errors.len();
    match (rerun, errors.first()) {
        (0, Some(e)) => anyhow::bail!("no failed run could be rerun: {e}"),
        (_, Some(e)) => Ok(format!("{rerun} of {total} failed runs rerunning ({} couldn't be: {e})", errors.len())),
        _ => Ok(format!("{rerun} failed run(s) rerunning")),
    }
}

/// Workflow runs waiting for a maintainer to approve them (a first-time contributor's PR, say):
/// head commit → their run ids. Only those for `sha`, if given.
pub fn awaiting_approval(sha: Option<&str>) -> Result<std::collections::HashMap<String, Vec<u64>>> {
    let mut ep = "repos/{owner}/{repo}/actions/runs?status=action_required&per_page=100".to_owned();
    if let Some(s) = sha {
        ep += &format!("&head_sha={s}");
    }
    let v: Value = gh_json(&["api", &ep])?;
    let mut out: std::collections::HashMap<String, Vec<u64>> = Default::default();
    for r in v["workflow_runs"].as_array().into_iter().flatten() {
        if let (Some(sha), Some(id)) = (r["head_sha"].as_str(), r["id"].as_u64()) {
            out.entry(sha.to_owned()).or_default().push(id);
        }
    }
    Ok(out)
}

/// Approve the workflow runs waiting on PR head `sha`, so they run: how many were.
pub fn approve_runs(sha: &str) -> Result<usize> {
    let ids = awaiting_approval(Some(sha))?.remove(sha).unwrap_or_default();
    for id in &ids {
        gh(&["api", "-X", "POST", &format!("repos/{{owner}}/{{repo}}/actions/runs/{id}/approve")])?;
    }
    Ok(ids.len())
}
