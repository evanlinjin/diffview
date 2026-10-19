use super::*;
use serde_json::json;

#[test]
fn info_summarises_checks() {
    let v = serde_json::json!({
        "title": "t", "author": { "login": "amy" }, "state": "OPEN", "isDraft": true,
        "baseRefName": "main", "headRefName": "feat", "additions": 3, "deletions": 1, "changedFiles": 2,
        "labels": [{ "name": "bug", "color": "d73a4a" }], "reviewDecision": "CHANGES_REQUESTED",
        "latestReviews": [{ "author": { "login": "bo" }, "state": "CHANGES_REQUESTED" }], "reviewRequests": [],
        "statusCheckRollup": [
            { "name": "build", "status": "COMPLETED", "conclusion": "SUCCESS" },
            { "name": "test", "status": "COMPLETED", "conclusion": "FAILURE" },
            { "context": "ci/legacy", "state": "PENDING" },
        ],
        "body": "", "comments": [],
    });
    let lines = info_lines(&v);
    assert_eq!(state(&v), ("DRAFT", "6e7681")); // on the header line, in GitHub's color
    assert_eq!(state(&serde_json::json!({ "state": "CLOSED", "isDraft": true })).0, "CLOSED");
    assert_eq!(lines[1].marks[1], (7, 12, Mark::Chip("d73a4a".into()))); // " bug " in its label color
    let t: Vec<String> = lines.into_iter().map(|l| l.text).collect();
    assert_eq!(t[0], "@amy · main ← feat · +3 −1 in 2 files");
    assert_eq!(&t[2..4], ["reviews:", "✗ @bo requested changes"]); // one a line
    assert_eq!(&t[4..7], ["checks: ✓ 1 passed · ✗ 1 failed · ● 1 pending", "✗ test", "● ci/legacy"]);
    let v = serde_json::json!({ "statusCheckRollup": [{ "name": "build", "status": "COMPLETED", "conclusion": "SUCCESS" }] });
    let t: Vec<String> = info_lines(&v).into_iter().map(|l| l.text).collect();
    assert!(t.contains(&"checks: ✓ 1 passed".to_owned())); // no "✗ 0 failed · ● 0 pending"
    assert!(t.contains(&"reviews: none yet".to_owned()));
}

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

#[test]
fn reviews_in_the_conversation() {
    let v = serde_json::json!({
        "url": "https://github.com/o/r/pull/1", "body": "",
        "comments": [{ "author": { "login": "amy" }, "body": "first", "createdAt": "2026-01-01T00:00:00Z", "url": "c1" }],
        "reviews": [
            { "id": "R3", "author": { "login": "me" }, "state": "COMMENTED", "body": "looks close", "submittedAt": "2026-01-03T00:00:00Z" },
            { "id": "R2", "author": { "login": "bo" }, "state": "APPROVED", "body": "", "submittedAt": "2026-01-02T00:00:00Z" },
            { "author": { "login": "cy" }, "state": "COMMENTED", "body": "", "submittedAt": "2026-01-04T00:00:00Z" },
            { "author": { "login": "me" }, "state": "PENDING", "body": "draft", "submittedAt": "" },
        ],
    });
    let lines = info_lines(&v);
    let text: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
    let at = text.iter().position(|t| t.starts_with("conversation")).unwrap();
    assert_eq!(text[at], "conversation (3)"); // the empty plain review and the pending one aren't
    let rest = &text[at + 1..];
    assert!(rest[0].starts_with("@amy · ") && rest[1] == "first");
    assert!(rest[2].starts_with("@bo · ✓ approved · ") && rest[3].is_empty());
    assert!(rest[4].starts_with("@me · reviewed · ") && rest[5] == "looks close");
    // Each its own link (what `a` keeps a question about it by): not the PR's, nor another's.
    let urls: Vec<&str> = lines[lines.len() - 6..].iter().step_by(2).map(|l| l.url.as_str()).collect();
    assert_eq!(urls, ["c1", "https://github.com/o/r/pull/1#review-R2", "https://github.com/o/r/pull/1#review-R3"]);
}

#[test]
fn signed_commits_in_the_pr_info() {
    let v = |commits: Value, required: bool| {
        let v = json!({ "baseRefName": "master", "dvCommits": commits, "dvSignaturesRequired": required });
        info_lines(&v).into_iter().map(|l| l.text).find(|t| t.starts_with("commits: "))
    };
    let both = json!([{ "oid": "aaaaaaaa11", "signed": true }, { "oid": "eb35a46622", "signed": false }]);
    assert_eq!(v(both.clone(), true).unwrap(), "commits: ✗ 1 of 2 unsigned: eb35a466 · master requires signed commits");
    assert_eq!(v(both, false).unwrap(), "commits: ✗ 1 of 2 unsigned: eb35a466");
    assert_eq!(v(json!([{ "oid": "a", "signed": true }, { "oid": "b", "signed": true }]), false).unwrap(), "commits: ✓ all 2 signed");
    assert_eq!(v(json!([]), false), None); // not known (an older cache): nothing said
    // More commits than GitHub gave: the older ones aren't checked, and it says so.
    let w = json!({ "baseRefName": "master", "dvCommits": [{ "oid": "a", "signed": true }], "dvCommitsTotal": 150 });
    assert!(info_lines(&w).iter().any(|l| l.text == "commits: ✓ 1 signed (the last 1 of 150)"));
    // Required by a ruleset, or by classic branch protection.
    let rules = |r: Value| requires_signatures(&json!({ "baseRef": r }));
    assert!(rules(json!({ "rules": { "nodes": [{ "type": "PULL_REQUEST" }, { "type": "REQUIRED_SIGNATURES" }] } })));
    assert!(rules(json!({ "branchProtectionRule": { "requiresCommitSignatures": true }, "rules": { "nodes": [] } })));
    assert!(!rules(json!({ "branchProtectionRule": null, "rules": { "nodes": [{ "type": "DELETION" }] } })));
    // The unsigned commits, for the contents' commits.
    let x = json!({ "dvCommits": [{ "oid": "aa", "signed": true }, { "oid": "bb", "signed": false }] });
    assert_eq!(parse_info(&x).unsigned, Some(vec!["bb".to_owned()]));
    assert_eq!(parse_info(&json!({})).unsigned, None); // unknown: not "all signed"
}

#[test]
fn workflow_runs_awaiting_approval() {
    let v = json!({ "dvAwaiting": 2, "headRefOid": "abc" });
    assert!(info_lines(&v).iter().any(|l| l.text == "⏸ 2 workflow runs awaiting approval (Space g approves)"));
    assert_eq!((parse_info(&v).awaiting, parse_info(&v).failing), (2, 0));
    assert!(!info_lines(&json!({})).iter().any(|l| l.text.contains("awaiting approval")));
}

#[test]
fn reactions_both_ways() {
    // GraphQL's reactors, `gh pr view`'s users; none-left groups dropped.
    let g = json!([{ "content": "THUMBS_UP", "reactors": { "totalCount": 2 } }, { "content": "EYES", "reactors": { "totalCount": 0 } }]);
    let v = json!([{ "content": "HOORAY", "users": { "totalCount": 1 } }]);
    assert_eq!(reactions(&g, &[]), [("THUMBS_UP".to_owned(), 2, false)]);
    assert_eq!(reactions_text(&[reactions(&g, &[]), reactions(&v, &[])].concat()), "👍 2  🎉 1");
    // Yours: as GraphQL says, or (gh pr view's) as dv asked.
    let g = json!([{ "content": "EYES", "viewerHasReacted": true, "reactors": { "totalCount": 1 } }]);
    assert_eq!(reactions(&g, &[]), [("EYES".to_owned(), 1, true)]);
    assert_eq!(reactions(&v, &["HOORAY"]), [("HOORAY".to_owned(), 1, true)]);
}

/// Live: react to comments (and a PR) on GitHub and take it back. Changes GitHub: give it a
/// throwaway PR. `DV_LIVE_PR=<n> DV_LIVE_NODES=<PR's id>,<a comment's id> cargo test
/// live_reactions -- --ignored --nocapture`
#[test]
#[ignore]
fn live_reactions() {
    let nodes = std::env::var("DV_LIVE_NODES").expect("DV_LIVE_NODES");
    let pr: u64 = std::env::var("DV_LIVE_PR").expect("DV_LIVE_PR").parse().unwrap();
    for node in nodes.split(',') {
        let mine = |n: &str| reactions_of(n).unwrap().into_iter().find(|r| r.0 == "EYES").map(|r| (r.1, r.2));
        react(node, "EYES", true).unwrap();
        assert_eq!(mine(node), Some((1, true)), "{node}: added");
        if node.starts_with("PR_") {
            // Shown among the PR's facts; and its comments' on their headings.
            let lines = info_lines(&info_json(pr).unwrap());
            assert!(lines.iter().any(|l| l.text == "reactions 👀 1"), "PR's reactions not shown");
        } else {
            let lines = info_lines(&info_json(pr).unwrap());
            assert!(lines.iter().any(|l| l.node == node && l.text.ends_with("· 👀 1")), "comment's reactions not shown");
        }
        react(node, "EYES", false).unwrap();
        assert_eq!(mine(node), Some((0, false)), "{node}: taken back");
    }
}

/// Live: label and assign a PR, then take both back. Changes GitHub: give it a throwaway PR.
/// `DV_LIVE_PR=<n> cargo test live_labels -- --ignored --nocapture`
#[test]
#[ignore]
fn live_labels() {
    let n: u64 = std::env::var("DV_LIVE_PR").expect("DV_LIVE_PR").parse().unwrap();
    let (labels, people, _) = pr_fields(n).unwrap();
    println!("labels: {labels:?}\npeople: {people:?}");
    let (label, me) = (labels[0].0.clone(), people[0].0.clone());
    set_field(n, Field::Label, &label, true).unwrap();
    set_field(n, Field::Assignee, &me, true).unwrap();
    let (labels, people, _) = pr_fields(n).unwrap();
    assert!(labels.contains(&(label.clone(), true)) && people.contains(&(me.clone(), true)));
    let lines = info_lines(&info_json(n).unwrap());
    assert!(lines.iter().any(|l| l.text == format!("assigned @{me}")), "assignee not among the facts");
    set_field(n, Field::Label, &label, false).unwrap();
    set_field(n, Field::Assignee, &me, false).unwrap();
    let (labels, people, _) = pr_fields(n).unwrap();
    assert!(labels.iter().all(|l| !l.1) && people.iter().all(|p| !p.1));
    // A review asked of someone (not the author): `DV_LIVE_REVIEWER`, if given.
    if let Ok(who) = std::env::var("DV_LIVE_REVIEWER") {
        set_field(n, Field::Reviewer, &who, true).unwrap();
        assert!(pr_fields(n).unwrap().2.contains(&(who.clone(), true)));
        set_field(n, Field::Reviewer, &who, false).unwrap();
        assert!(!pr_fields(n).unwrap().2.contains(&(who, true)));
    }
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

#[test]
fn info_lines_keep_their_keys() {
    let keys = |v: Value| info_lines(&v).into_iter().map(|l| l.key).collect::<Vec<_>>();
    let pr = |n: usize| json!({ "id": "PR_1", "statusCheckRollup": [], "comments": (0..n).map(|i| json!({ "id": format!("IC_{i}"), "author": { "login": "amy" }, "body": "hi", "createdAt": "2026-10-01T00:00:00Z" })).collect::<Vec<_>>() });
    let (one, two) = (keys(pr(1)), keys(pr(2)));
    // The same lines keep their keys though their text (counts) changed; a comment's text is
    // keyed by its comment.
    for k in ["conversation", "IC_0", "IC_0/text"] {
        assert!(one.iter().any(|x| x == k) && two.iter().any(|x| x == k), "{k}: {one:?} {two:?}");
    }
}

#[test]
fn signatures_kept_when_github_cant_say() {
    let old = json!({ "headRefOid": "abc", "dvCommits": [{ "oid": "abc12345", "signed": true }], "dvCommitsTotal": 1, "dvSignaturesRequired": false, "dvCommitsAt": "2026-10-19T07:00:00Z" });
    let mut new = json!({ "headRefOid": "abc", "dvCommitsUnknown": "timed out" });
    keep_signatures(&mut new, &old);
    assert!(new["dvCommitsUnknown"].is_null() && new["dvCommitsKept"] == true);
    let line = info_lines(&new).into_iter().find(|l| l.text.starts_with("commits:")).unwrap().text;
    assert!(line.contains("✓ 1 signed") && line.contains("(as of"), "{line}");
    // Pushed since: not kept (other commits); and nothing to keep when GitHub said.
    let mut moved = json!({ "headRefOid": "def", "dvCommitsUnknown": "timed out" });
    keep_signatures(&mut moved, &old);
    assert!(moved["dvCommitsKept"].is_null() && !moved["dvCommitsUnknown"].is_null());
}
