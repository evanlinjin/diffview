//! CI on GitHub: a PR's checks, its failed runs rerun, runs waiting for a maintainer's
//! approval, and marking it ready for review.
use super::{gh, gh_json};
use anyhow::{bail, Result};
use serde_json::Value;

/// PR `n`'s CI checks on GitHub now that decide (`gate`): (passed, failed, pending).
pub fn checks(n: u64) -> Result<(usize, usize, usize)> {
    let mut r = rollups(&[n])?;
    Ok(gate(&r.remove(&n).unwrap_or_default().1))
}

/// PRs `ns`' checks, as GitHub has them now: each its base and its checks (a check run's
/// `name`/`status`/`conclusion`/`detailsUrl`, a status's `context`/`state`/`targetUrl`, as
/// `gh pr view --json statusCheckRollup` has them), with `isRequired`: GitHub's word on whether
/// the PR's base requires it (its rulesets', its branch protection's: read access does). A
/// check its base's rulesets require that hasn't reported yet is there too, expected (pending).
pub fn rollups(ns: &[u64]) -> Result<std::collections::HashMap<u64, (String, Vec<Value>)>> {
    let mut out = std::collections::HashMap::new();
    let mut bases: std::collections::HashMap<String, Vec<String>> = Default::default();
    for chunk in ns.chunks(40) {
        let prs: String = chunk
            .iter()
            .map(|n| {
                format!(
                    "p{n}: pullRequest(number: {n}) {{ baseRefName commits(last: 1) {{ nodes {{ commit {{ statusCheckRollup {{ contexts(first: 100) {{ nodes {{ \
                     ... on CheckRun {{ name status conclusion detailsUrl isRequired(pullRequestNumber: {n}) }} \
                     ... on StatusContext {{ context state targetUrl isRequired(pullRequestNumber: {n}) }} }} }} }} }} }} }} }}"
                )
            })
            .collect::<Vec<_>>()
            .join(" ");
        let q = format!("query($owner: String!, $name: String!) {{ repository(owner: $owner, name: $name) {{ {prs} }} }}");
        let v = gh_json(&["api", "graphql", "-F", "owner={owner}", "-F", "name={repo}", "-f", &format!("query={q}")])?;
        if let Some(e) = v["errors"].get(0) {
            bail!("GitHub: {}", e["message"].as_str().unwrap_or("GraphQL error"));
        }
        for &n in chunk {
            let pr = &v["data"]["repository"][format!("p{n}")];
            let base = pr["baseRefName"].as_str().unwrap_or("").to_owned();
            let mut checks = pr["commits"]["nodes"][0]["commit"]["statusCheckRollup"]["contexts"]["nodes"].as_array().cloned().unwrap_or_default();
            // Required by its rulesets, and not reported (yet): expected, as GitHub shows it.
            let want = bases.entry(base.clone()).or_insert_with(|| ruleset_checks(&base));
            let missing: Vec<&String> = want.iter().filter(|w| !checks.iter().any(|c| check_name(c) == w.as_str())).collect();
            checks.extend(missing.into_iter().map(|name| serde_json::json!({ "name": name, "status": "EXPECTED", "isRequired": true })));
            out.insert(n, (base, checks));
        }
    }
    Ok(out)
}

/// The checks branch `base`'s rulesets require (read access does); none if it can't say.
fn ruleset_checks(base: &str) -> Vec<String> {
    let Ok(v) = gh_json(&["api", &format!("repos/{{owner}}/{{repo}}/rules/branches/{base}")]) else { return vec![] };
    let rules = v.as_array().into_iter().flatten().filter(|r| r["type"] == "required_status_checks");
    rules.flat_map(|r| r["parameters"]["required_status_checks"].as_array().cloned().unwrap_or_default()).filter_map(|c| c["context"].as_str().map(str::to_owned)).collect()
}

/// A check's name (a check run's) or context (a status's).
pub(crate) fn check_name(c: &Value) -> &str {
    c["name"].as_str().or(c["context"].as_str()).unwrap_or("?")
}

/// Whether check `c` is optional: its PR's base requires other checks, not it (`isRequired`).
pub(crate) fn optional(c: &Value, checks: &[Value]) -> bool {
    c["isRequired"] != true && checks.iter().any(|c| c["isRequired"] == true)
}

/// The checks that decide whether CI passes, tallied: the required ones when the base requires
/// any (an optional one failing, codecov's say, doesn't hold the PR back), else all of them.
pub(crate) fn gate(checks: &[Value]) -> (usize, usize, usize) {
    let req: Vec<Value> = checks.iter().filter(|c| !optional(c, checks)).cloned().collect();
    tally(&req)
}

/// Mark draft PR `n` ready for review (`gh pr ready`): only once CI passes, nothing failing
/// or still running (a PR with no checks has nothing to wait for).
pub fn mark_ready(n: u64) -> Result<()> {
    let (_, fail, pending) = checks(n)?;
    match (fail, pending) {
        // (Required ones that haven't reported are pending: `rollups`.)
        (0, 0) => gh(&["pr", "ready", &n.to_string()]).map(drop),
        (0, k) => bail!("CI is still running ({k} check{} pending): ready once it passes", if k == 1 { "" } else { "s" }),
        (k, _) => bail!("CI fails ({k} required check{} failed): fix it first (dv list: a a has Claude look)", if k == 1 { "" } else { "s" }),
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
