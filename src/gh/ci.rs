//! CI on GitHub: a PR's checks, its failed runs rerun, runs waiting for a maintainer's
//! approval, and marking it ready for review.
use super::{gh, gh_json};
use anyhow::{bail, Context, Result};
use serde_json::Value;

/// PR `n`'s CI checks on GitHub now that decide (`gate`): (passed, failed, pending).
pub fn checks(n: u64) -> Result<(usize, usize, usize)> {
    let r = rollups(&[n])?.remove(&n).context("GitHub said nothing of its checks")?;
    Ok(gate(&r.checks))
}

/// A PR's checks as GitHub has them now, on its head.
#[derive(Clone, Debug, Default)]
pub struct Rollup {
    pub head: String,
    /// A check run's `name`/`status`/`conclusion`/`detailsUrl`, a status's
    /// `context`/`state`/`targetUrl` (as `gh pr view --json statusCheckRollup` has them), each
    /// with `isRequired`: GitHub's word on whether its base requires it.
    pub checks: Vec<Value>,
    /// Its base's required checks couldn't be read (its rules, its branch protection): those
    /// not reported yet may be missing from `checks`.
    pub unknown: bool,
}

/// PRs `ns`' checks (`Rollup`), from one GraphQL query a few dozen PRs (a few at once). An open
/// PR's checks its base requires (its rulesets', its branch protection's) that haven't reported
/// yet are there, expected (pending), when GitHub listed them all. A PR GitHub had nothing on
/// (it failed there) isn't in the answer.
pub fn rollups(ns: &[u64]) -> Result<std::collections::HashMap<u64, Rollup>> {
    let chunks: Vec<&[u64]> = ns.chunks(40).collect();
    let mut out = std::collections::HashMap::new();
    for group in chunks.chunks(4) {
        let got: Vec<Result<Vec<(u64, Rollup)>>> = std::thread::scope(|s| {
            let handles: Vec<_> = group.iter().map(|c| s.spawn(move || rollup_chunk(c))).collect();
            handles.into_iter().map(|h| h.join().unwrap_or_else(|_| Err(anyhow::anyhow!("panicked")))).collect()
        });
        // (A batch GitHub failed on leaves the others': only all failing is a failure.)
        for r in got {
            match r {
                Ok(r) => out.extend(r),
                Err(e) if chunks.len() == 1 => return Err(e),
                Err(_) => {}
            }
        }
    }
    anyhow::ensure!(!out.is_empty() || ns.is_empty() || chunks.len() == 1, "GitHub answered none of its batches of checks");
    Ok(out)
}

/// The PRs (by their `p<N>` alias) a batch's errors are about: refused, and (no matter but
/// for what's required) its base's rules unreadable. An error about no one PR: the batch's.
fn refused_prs(v: &serde_json::Value) -> Result<(Vec<u64>, Vec<u64>)> {
    let (mut bad, mut unread) = (vec![], vec![]);
    for e in v["errors"].as_array().into_iter().flatten() {
        let path = e["path"].as_array().cloned().unwrap_or_default();
        let pr = path.iter().filter_map(|k| k.as_str()?.strip_prefix('p')?.parse::<u64>().ok()).next();
        match pr {
            Some(n) if path.iter().any(|k| k == "baseRef") => unread.push(n),
            Some(n) => bad.push(n),
            None => anyhow::bail!("GitHub: {}", e["message"].as_str().unwrap_or("refused")),
        }
    }
    Ok((bad, unread))
}

/// The checks branch `base`'s protection requires, from REST: GraphQL's `branchProtectionRule`
/// is null to anyone who can't administer the repo, while this answers anyone who can read it.
/// `None`: it couldn't be read. Kept a few minutes (a base's rules rarely change); a failure, 30s.
fn protected(base: &str) -> Option<Vec<String>> {
    type Cache = std::collections::HashMap<String, (std::time::Instant, Option<Vec<String>>)>;
    static CACHE: std::sync::LazyLock<std::sync::Mutex<Cache>> = std::sync::LazyLock::new(Default::default);
    // (A failure kept briefly: asked again soon, not once for each of its PRs.)
    let fresh = |c: &&(std::time::Instant, Option<Vec<String>>)| c.0.elapsed() < std::time::Duration::from_secs(if c.1.is_some() { 600 } else { 30 });
    if let Some(v) = CACHE.lock().ok()?.get(base).filter(fresh) {
        return v.1.clone();
    }
    // (A branch name may have `#`, `?`, `%` or a space: escaped in the path.)
    let path: String = base.bytes().map(|b| match b.is_ascii_alphanumeric() || b"/._-".contains(&b) {
        true => (b as char).to_string(),
        false => format!("%{b:02X}"),
    }).collect();
    let got: Option<Vec<String>> = (!base.is_empty())
        .then(|| gh_json(&["api", &format!("repos/{{owner}}/{{repo}}/branches/{path}")]).ok())
        .flatten()
        .map(|b| b["protection"]["required_status_checks"]["contexts"].as_array().into_iter().flatten().filter_map(|c| c.as_str().map(str::to_owned)).collect());
    if let Ok(mut c) = CACHE.lock() {
        c.insert(base.to_owned(), (std::time::Instant::now(), got.clone()));
    }
    got
}

fn rollup_chunk(ns: &[u64]) -> Result<Vec<(u64, Rollup)>> {
    let prs: String = ns
        .iter()
        .map(|n| {
            format!(
                "p{n}: pullRequest(number: {n}) {{ state baseRefName \
                 baseRef {{ rules(first: 50) {{ nodes {{ type parameters {{ ... on RequiredStatusChecksParameters {{ requiredStatusChecks {{ context }} }} }} }} }} }} \
                 commits(last: 1) {{ nodes {{ commit {{ oid statusCheckRollup {{ contexts(first: 100) {{ pageInfo {{ hasNextPage }} nodes {{ \
                   ... on CheckRun {{ name status conclusion detailsUrl isRequired(pullRequestNumber: {n}) }} \
                   ... on StatusContext {{ context state targetUrl isRequired(pullRequestNumber: {n}) }} }} }} }} }} }} }} }}"
            )
        })
        .collect::<Vec<_>>()
        .join(" ");
    let q = format!("query($owner: String!, $name: String!) {{ repository(owner: $owner, name: $name) {{ {prs} }} }}");
    let v = gh_json(&["api", "graphql", "-F", "owner={owner}", "-F", "name={repo}", "-f", &format!("query={q}")])?;
    // As a PR's other facts are weighed (`gh::refused`): its base's rules unreadable, its checks
    // stand (only none expected of those not reported yet: what's reported says if it's
    // required); anything else refused, none of that PR's (a check GitHub didn't give isn't
    // none: `checks` would say passing), the others' kept. Refused about no one PR: none.
    let (bad, unread) = refused_prs(&v)?;
    // All of them refused: GitHub's word for it, not an answer of nothing.
    if !ns.is_empty() && ns.iter().all(|n| bad.contains(n)) {
        super::refused(&v)?;
    }
    let repo = &v["data"]["repository"];
    anyhow::ensure!(repo.is_object(), "GitHub: no repository in its answer");
    Ok(ns
        .iter()
        .filter(|n| !bad.contains(n))
        .filter_map(|&n| {
            let pr = repo.get(format!("p{n}")).filter(|p| !p.is_null())?;
            let commit = &pr["commits"]["nodes"][0]["commit"];
            let contexts = &commit["statusCheckRollup"]["contexts"];
            let mut checks = contexts["nodes"].as_array().cloned().unwrap_or_default();
            // Required and not reported (yet): expected, as GitHub shows it. Not when it listed
            // only some (one past its first 100 isn't missing), nor once it's closed.
            let rules = pr["baseRef"]["rules"]["nodes"].as_array().into_iter().flatten().filter(|r| r["type"] == "REQUIRED_STATUS_CHECKS");
            let mut want: Vec<String> = rules.flat_map(|r| r["parameters"]["requiredStatusChecks"].as_array().cloned().unwrap_or_default()).filter_map(|c| c["context"].as_str().map(str::to_owned)).collect();
            // (Only an open one's: nothing more is expected of a closed one.)
            let open = pr["state"] == "OPEN";
            let protection = open.then(|| protected(pr["baseRefName"].as_str().unwrap_or(""))).flatten();
            let unknown = open && (unread.contains(&n) || protection.is_none());
            want.extend(protection.unwrap_or_default());
            want.sort();
            want.dedup(); // (a ruleset and branch protection may both name it)
            if pr["state"] == "OPEN" && contexts["pageInfo"]["hasNextPage"] != true {
                let missing: Vec<String> = want.into_iter().filter(|w| !checks.iter().any(|c| check_name(c) == w.as_str())).collect();
                checks.extend(missing.into_iter().map(|name| serde_json::json!({ "name": name, "status": "EXPECTED", "isRequired": true })));
            }
            Some((n, Rollup { head: commit["oid"].as_str().unwrap_or("").to_owned(), checks, unknown }))
        })
        .collect())
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

#[test]
fn one_refused_pr_leaves_the_others() {
    let v = serde_json::json!({ "errors": [
        { "path": ["repository", "p12", "commits", "nodes", 0], "message": "no" },
        { "path": ["repository", "p13", "baseRef", "rules"], "message": "can't read rules" } ] });
    assert_eq!(refused_prs(&v).unwrap(), (vec![12], vec![13]));
    let whole = serde_json::json!({ "errors": [{ "message": "rate limited" }] });
    assert!(refused_prs(&whole).is_err());
}
