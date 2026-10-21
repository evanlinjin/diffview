use super::*;
use serde_json::json;

/// "3h ago", "2d ago", "5mo ago", like GitHub. Falls back to the date if it can't be parsed.

#[test]
fn issue_links() {
    assert_eq!(issue_ref("https://github.com/o/r/issues/12"), Some(("o", "r", 12)));
    assert_eq!(issue_ref("https://github.com/o/r/pull/7#discussion_r1"), Some(("o", "r", 7)));
    assert_eq!(issue_ref("https://github.com/o/r/pull/7/files?w=1"), Some(("o", "r", 7)));
    assert_eq!(issue_ref("https://github.com/amy"), None);
    assert_eq!(issue_ref("https://github.com/o/r/blob/main/x.rs"), None);
}

#[test]
fn relative_time() {
    assert_eq!(epoch_secs("1970-01-02T00:00:00Z"), Some(86400));
    assert_eq!(epoch_secs("2000-03-01"), Some(951868800));
    let now = epoch_secs("2026-10-16T12:00:00Z").unwrap();
    assert_eq!(ago_at("2026-10-16T11:59:30Z", now), "just now");
    assert_eq!(ago_at("2026-10-16T09:00:00Z", now), "3h ago");
    assert_eq!(ago_at("2026-10-14T12:00:00Z", now), "2d ago");
    assert_eq!(ago_at("2026-08-14T12:00:00Z", now), "2mo ago");
    assert_eq!(ago_at("garbage", now), "garbage");
}

#[test]
fn parses_review() {
    let r = parse_review(&json!({
        "id": "PR_1",
        "reviews": { "nodes": [{ "id": "R_other", "viewerDidAuthor": false }, { "id": "R_mine", "viewerDidAuthor": true }] },
        "files": { "nodes": [{ "path": "a.rs", "viewerViewedState": "VIEWED" }, { "path": "b.rs", "viewerViewedState": "UNVIEWED" }] },
        "reviewThreads": { "nodes": [{ "id": "T_1", "path": "a.rs", "comments": { "nodes": [{ "id": "C_1", "databaseId": 1, "state": "PENDING", "viewerCanUpdate": true }] } }] },
    }));
    assert_eq!((r.pr_id.as_str(), r.pending.as_deref()), ("PR_1", Some("R_mine")));
    assert_eq!(r.viewed.len(), 1);
    assert_eq!(r.viewed["a.rs"], "VIEWED");
    assert_eq!((r.threads[0].id.as_str(), r.threads[0].comments[0].pending), ("T_1", true));
    assert_eq!((r.threads[0].comments[0].node_id.as_str(), r.threads[0].comments[0].can_edit), ("C_1", true));
}

#[test]
fn parses_thread() {
    let t = parse_thread(&serde_json::json!({
        "path": "src/a.rs", "line": 12, "startLine": 10, "originalLine": 12, "diffSide": "RIGHT",
        "isResolved": false, "isOutdated": false,
        "comments": { "nodes": [{ "databaseId": 7, "author": { "login": "amy" }, "body": "a\r\nb", "createdAt": "2026-10-01T00:00:00Z",
            "diffHunk": "@@ -1,2 +1,2 @@\n fn a() {\n+    let x = 1;" }] }
    }));
    assert_eq!((t.left, t.line, t.start_line), (false, Some(12), Some(10)));
    assert_eq!(t.hunk, [" fn a() {", "+    let x = 1;"]);
    assert_eq!((t.comments[0].id, t.comments[0].body.as_str(), t.comments[0].created.as_str()), (7, "a\nb", "2026-10-01T00:00:00Z"));
}

#[test]
fn review_pages() {
    // Two pages of threads (the first thread's comments on two pages too) and two of files.
    let page = |next: Option<&str>, nodes: Value| json!({ "pageInfo": { "hasNextPage": next.is_some(), "endCursor": next }, "nodes": nodes });
    let comment = |id: u64| json!({ "id": format!("C{id}"), "databaseId": id, "body": "", "createdAt": "" });
    let thread = |id: &str, comments: Value| json!({ "id": id, "path": "a.rs", "line": 1, "comments": comments });
    let pr = |k: &str, v: Value| json!({ "repository": { "pullRequest": { k: v } } });
    let mut calls = vec![];
    let d = fetch_review(|q, vars| {
        calls.push(vars.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(" "));
        Ok(if q.contains("viewer { login }") {
            let mut d = pr("reviewThreads", page(Some("t1"), json!([thread("T1", page(Some("c1"), json!([comment(1)])))])));
            d["repository"]["pullRequest"]["files"] = page(Some("f1"), json!([{ "path": "a.rs", "viewerViewedState": "VIEWED" }]));
            d["viewer"] = json!({ "login": "me" });
            d
        } else if q.contains("node(id") {
            json!({ "node": { "comments": page(None, json!([comment(2)])) } })
        } else if q.contains("reviewThreads") {
            pr("reviewThreads", page(None, json!([thread("T2", page(None, json!([comment(3)])))])))
        } else {
            pr("files", page(None, json!([{ "path": "b.rs", "viewerViewedState": "DISMISSED" }])))
        })
    })
    .unwrap();
    assert_eq!(calls, ["", "after=t1", "after=f1", "id=T1 after=c1"]);
    let r = parse_review(&d["repository"]["pullRequest"]);
    let ids: Vec<Vec<u64>> = r.threads.iter().map(|t| t.comments.iter().map(|c| c.id).collect()).collect();
    assert_eq!(ids, [vec![1, 2], vec![3]]);
    assert_eq!(r.viewed.len(), 2);
}

/// Live: PR info's one extra query (signatures, yours) answers. `DV_LIVE_PR=<n> cargo test
/// live_info -- --ignored`
#[test]
#[ignore]
fn live_info() {
    let n: u64 = std::env::var("DV_LIVE_PR").expect("DV_LIVE_PR").parse().unwrap();
    let v = info_json(n).unwrap();
    assert!(v["dvCommits"].is_array() && v["dvActivity"]["items"].is_array(), "{} {}", v["dvCommits"], v["dvActivity"]);
    // Its activity, as dv shows it.
    for l in crate::view::info_lines_of(&v) {
        eprintln!("{l}");
    }
}

#[test]
fn rerun_summary() {
    assert_eq!(rerun_says(3, &[], "failed ").unwrap(), "3 failed run(s) rerunning");
    assert_eq!(rerun_says(3, &["run 7: too old".into()], "failed ").unwrap(), "2 of 3 failed runs rerunning (1 couldn't be: run 7: too old)");
    assert!(rerun_says(1, &["run 7: too old".into()], "failed ").is_err());
    assert_eq!(rerun_says(5, &[], "").unwrap(), "5 run(s) rerunning");
}

#[test]
fn github_refusals() {
    let commits = json!({ "totalCount": 1, "nodes": [{ "commit": { "oid": "a", "signature": null } }] });
    let answer = |errors: Value| json!({ "data": { "repository": { "pullRequest": { "commits": commits, "baseRef": null } } }, "errors": errors });
    // Its base's rules refused: the signatures stand (not required).
    let p = pr_node(&answer(json!([{ "path": ["repository", "pullRequest", "baseRef", "rules"] }]))).unwrap();
    assert!(signatures_of(&p).is_ok_and(|s| !s.required));
    // Anything else refused, or an error that doesn't say where: none of it, GitHub's word why.
    for e in [json!([{ "path": ["repository", "pullRequest", "commits", "nodes", 0], "message": "timed out" }]), json!([{ "message": "timed out" }])] {
        assert_eq!(format!("{:#}", pr_node(&answer(e)).unwrap_err()), "GitHub: timed out");
    }
    // dv list's (many PRs): a base's rules refused deeper down, fine; a signature, not.
    let at = |path: Value| json!({ "data": {}, "errors": [{ "path": path, "message": "refused" }] });
    assert!(refused(&at(json!(["repository", "pullRequests", "nodes", 3, "baseRef", "branchProtectionRule"]))).is_ok());
    assert!(refused(&at(json!(["repository", "pullRequests", "nodes", 3, "commits", "nodes", 0, "commit", "signature"]))).is_err());
    // No pull request in it: said.
    assert!(pr_node(&json!({ "data": { "repository": { "pullRequest": null } } })).is_err());
}


#[test]
fn only_required_checks_decide() {
    use serde_json::json;
    let run = |name: &str, conclusion: &str, required: bool| json!({ "name": name, "status": "COMPLETED", "conclusion": conclusion, "isRequired": required });
    // An optional one failing: CI passes.
    let checks = [run("build", "SUCCESS", true), run("codecov/patch", "FAILURE", false)];
    assert_eq!(gate(&checks), (1, 0, 0));
    assert!(optional(&checks[1], &checks) && !optional(&checks[0], &checks));
    // A required one not reported yet (expected): pending, not passed.
    let checks = [run("build", "SUCCESS", true), json!({ "name": "e2e", "status": "EXPECTED", "isRequired": true })];
    assert_eq!(gate(&checks), (1, 0, 1));
    // None required: every check counts.
    let checks = [run("build", "SUCCESS", false), run("lint", "FAILURE", false)];
    assert_eq!(gate(&checks), (1, 1, 0));
}

/// Live: `DV_LIVE_REPO=<a checkout> cargo test live_rollups -- --ignored --nocapture`.
#[test]
#[ignore]
fn live_rollups() {
    std::env::set_current_dir(std::env::var("DV_LIVE_REPO").expect("DV_LIVE_REPO")).unwrap();
    let ns: Vec<u64> = std::env::var("DV_LIVE_PRS").unwrap_or_default().split(',').filter_map(|n| n.trim().parse().ok()).collect();
    for (n, r) in rollups(&ns).unwrap() {
        let req: Vec<&str> = r.checks.iter().filter(|c| c["isRequired"] == true).map(check_name).collect();
        println!("#{n} {} gate {:?} required {req:?}", &r.head[..7.min(r.head.len())], gate(&r.checks));
    }
}
