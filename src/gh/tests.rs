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
    assert!(v["dvCommits"].is_array() && v["dvMine"].is_object(), "{}", v["dvCommits"]);
}

#[test]
fn rerun_summary() {
    assert_eq!(rerun_says(3, &[]).unwrap(), "3 failed run(s) rerunning");
    assert_eq!(rerun_says(3, &["run 7: too old".into()]).unwrap(), "2 of 3 failed runs rerunning (1 couldn't be: run 7: too old)");
    assert!(rerun_says(1, &["run 7: too old".into()]).is_err());
}

#[test]
fn github_refusals() {
    let commits = json!({ "totalCount": 1, "nodes": [{ "commit": { "oid": "a", "signature": null } }] });
    let answer = |errors: Value| json!({ "data": { "repository": { "pullRequest": { "commits": commits, "baseRef": null } } }, "errors": errors });
    // Only its reactions (comments) refused: the signatures stand.
    let a = pr_answer(&answer(json!([{ "path": ["repository", "pullRequest", "comments"] }])), "").unwrap();
    assert!(signatures_whole(&a).is_ok());
    // Its commits refused, or an error that doesn't say where: not taken as unsigned.
    for e in [json!([{ "path": ["repository", "pullRequest", "commits", "nodes", 0] }]), json!([{ "message": "timeout" }])] {
        assert!(signatures_whole(&pr_answer(&answer(e), "").unwrap()).is_err());
    }
    // Refused whole: GitHub's reason.
    let e = pr_answer(&json!({ "data": null, "errors": [{ "type": "RATE_LIMITED", "message": "API rate limit exceeded" }] }), "").unwrap_err();
    assert_eq!(format!("{e:#}"), "GitHub: API rate limit exceeded");
    assert!(e.is::<Refused>());
    // Every error's kind kept; the reason, the one that isn't about size.
    let both = json!({ "data": null, "errors": [{ "type": "MAX_NODE_LIMIT_EXCEEDED", "message": "too many nodes" }, { "type": "RATE_LIMITED", "message": "API rate limit exceeded" }] });
    let e = pr_answer(&both, "").unwrap_err();
    let r = e.downcast_ref::<Refused>().unwrap();
    assert_eq!((r.kinds.len(), too_big(r), format!("{e:#}").as_str()), (2, false, "GitHub: API rate limit exceeded"));
    // A body with only a message (a bad token): that, and not a refusal (asked again for).
    let e = pr_answer(&json!({ "message": "Bad credentials" }), "gh: Bad credentials (HTTP 401)").unwrap_err();
    assert_eq!(format!("{e:#}"), "GitHub: Bad credentials");
    assert!(!e.is::<Refused>());
    // Its commits refused: GitHub's word for why, its first line.
    let a = pr_answer(&answer(json!([{ "path": ["repository", "pullRequest", "commits"], "message": "timed out\nmore" }])), "").unwrap();
    assert_eq!(extras(Ok(a), || unreachable!()).1.unwrap_err(), "timed out");
    // Refused whole for its size or time: asked again alone (none of the reactions then);
    // anything that would refuse it again, among its errors: not.
    let refused = |kinds: &[&str]| -> Result<PrAnswer> { Err(Refused { kinds: kinds.iter().map(|k| k.to_string()).collect(), why: "too big".into() }.into()) };
    let alone = || pr_answer(&answer(json!([])), "");
    for kinds in [&["MAX_NODE_LIMIT_EXCEEDED"][..], &["TIMEOUT"], &["RESOURCE_LIMITS_EXCEEDED", "TIMEOUT"]] {
        let (node, sig) = extras(refused(kinds), alone);
        let mut mine = serde_json::Map::new();
        mine_of(&node, &mut mine);
        assert!(sig.is_ok() && mine.is_empty(), "{kinds:?}"); // none known
    }
    let never: [&[&str]; 7] = [&[""], &["RATE_LIMITED"], &["SECONDARY_RATE_LIMIT"], &["FORBIDDEN"], &["NOT_FOUND"], &["EXCESSIVE_PAGINATION"], &["MAX_NODE_LIMIT_EXCEEDED", "RATE_LIMITED"]];
    for kinds in never {
        let (_, sig) = extras(refused(kinds), || unreachable!());
        assert_eq!(sig.unwrap_err(), "GitHub: too big");
    }
}

#[test]
fn gateway_timeout_asked_again_smaller() {
    let refused = |v: Value, stderr: &str| pr_answer(&v, stderr).err().and_then(|e| e.downcast::<Refused>().ok());
    let late = refused(serde_json::json!({ "message": "We couldn't respond to your request in time. Sorry about that." }), "gh: HTTP 504");
    assert!(late.is_some_and(|r| too_big(&r)));
    assert!(refused(serde_json::json!(null), "HTTP 502: Bad Gateway").is_some_and(|r| too_big(&r)));
    // Not about time: not asked again.
    assert!(refused(serde_json::json!({ "message": "Bad credentials" }), "HTTP 401").is_none());
}

