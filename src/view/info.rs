//! The PR section's lines from GitHub's answer (`gh::info_json`): its facts, description and
//! conversation, as text with marks for the view to color.
use crate::gh::{ago, hidden, reactions, verdict, Reaction, REACTIONS};
use serde_json::Value;

/// What a comment hidden on GitHub (`why`, from `hidden`) says it is: "hidden on GitHub
/// (spam)", or (GitHub gave no reason) just "hidden on GitHub".
pub fn hidden_words(why: &str) -> String {
    match why {
        "hidden" => "hidden on GitHub".into(),
        why => format!("hidden on GitHub ({why})"),
    }
}

/// Reactions as they show (`👍 2`, `🎉 1`), each with whether one's yours.
pub fn reaction_parts(r: &[Reaction]) -> Vec<(String, bool)> {
    let emoji = |c: &str| REACTIONS.iter().find(|e| e.0 == c).map_or("?", |e| e.1);
    r.iter().map(|(c, n, mine)| (format!("{} {n}", emoji(c)), *mine)).collect()
}

/// Reactions as text: `👍 2  🎉 1`.
#[cfg(test)]
fn reactions_text(r: &[Reaction]) -> String {
    reaction_parts(r).into_iter().map(|p| p.0).collect::<Vec<_>>().join("  ")
}

impl InfoLine {
    /// Reactions, yours in the accent color (as GitHub lights them); all dimmed when which
    /// are yours isn't `known`.
    fn put_reactions(&mut self, r: &[Reaction], known: bool) {
        for (i, (text, mine)) in reaction_parts(r).into_iter().enumerate() {
            let mark = match known {
                true => mine.then_some(Mark::Mine),
                false => Some(Mark::Dim),
            };
            self.put(if i == 0 { "" } else { "  " }, None).put(&text, mark);
        }
    }
}

/// How to color part of a PR info line.
#[derive(Clone, Debug, PartialEq)]
pub enum Mark {
    /// A badge with this background (hex, no '#'), e.g. a label or the PR state.
    Chip(String),
    Pass,
    Fail,
    Pending,
    Dim,
    /// A reaction of yours.
    Mine,
    Link(String),
}

/// A line of the PR info section. `md` lines (description, comments) are rendered as markdown;
/// the rest are colored by `marks` (char ranges).
#[derive(Default)]
pub struct InfoLine {
    pub heading: bool,
    pub md: bool,
    pub text: String,
    pub marks: Vec<(usize, usize, Mark)>,
    /// On a comment's (or the description's) heading: its link on GitHub, for `Space c`.
    pub url: String,
    /// On a conversation comment's (or review's) heading: its node id, to react to it.
    pub node: String,
    /// On a conversation comment's heading: hidden on GitHub, and why; empty if it isn't.
    pub hidden: String,
    /// What the line is, the same across refreshes (its text may not be: counts, times): a
    /// comment's node id (its text: `<node>/text`), else what the line is about (`checks`,
    /// `conversation`, …). For the cursor to stay on it.
    pub key: String,
}

impl InfoLine {
    fn new(heading: bool, md: bool) -> Self {
        InfoLine { heading, md, ..Default::default() }
    }

    fn put(&mut self, s: &str, mark: Option<Mark>) -> &mut Self {
        let a = self.text.chars().count();
        self.text.push_str(s);
        if let Some(m) = mark {
            self.marks.push((a, a + s.chars().count(), m));
        }
        self
    }
}

/// What PR info says, beyond its section's lines.
pub struct Info {
    pub lines: Vec<InfoLine>,
    pub title: String,
    pub head: String,
    pub url: String,
    pub state: State,
    /// Its commits GitHub has no valid signature on.
    /// (`None`: GitHub didn't say this time: what was known stands.)
    pub unsigned: Option<Vec<String>>,
    /// Workflow runs awaiting approval, checks failing.
    pub awaiting: usize,
    pub failing: usize,
    /// Its base branch.
    pub base: String,
}

/// The PR section's lines, its title, head, URL and state, its commits GitHub has no valid
/// signature on, (workflow runs awaiting approval, checks failing), and its base branch.
pub fn parse_info(v: &Value) -> Info {
    let s = |k: &str| v[k].as_str().unwrap_or("").to_owned();
    let unsigned = v["dvCommits"].as_array().map(|cs| cs.iter().filter(|c| c["signed"] != true).filter_map(|c| c["oid"].as_str().map(str::to_owned)).collect());
    let failing = v["statusCheckRollup"].as_array().into_iter().flatten().filter(|c| verdict(c) == '✗').count();
    let awaiting = v["dvAwaiting"].as_u64().unwrap_or(0) as usize;
    Info { lines: info_lines(v), title: s("title"), head: s("headRefOid"), url: s("url"), state: state(v), unsigned, awaiting, failing, base: s("baseRefName") }
}

/// A PR's state as GitHub shows it, with its color: DRAFT, OPEN, MERGED or CLOSED.
pub type State = (&'static str, &'static str);

pub fn state(v: &Value) -> State {
    // A closed draft is closed.
    match (v["state"].as_str().unwrap_or(""), v["isDraft"] == true) {
        ("MERGED", _) => ("MERGED", "8957e5"),
        ("CLOSED", _) => ("CLOSED", "da3633"),
        (_, true) => ("DRAFT", "6e7681"),
        _ => ("OPEN", "238636"),
    }
}

fn info_lines(v: &Value) -> Vec<InfoLine> {
    let s = |v: &Value| v.as_str().unwrap_or("").to_owned();
    let list = |v: &Value| v.as_array().map(Vec::as_slice).unwrap_or(&[]).to_vec();
    let mut out = vec![];
    let plain = |text: String| {
        let mut l = InfoLine::new(false, false);
        l.put(&text, None);
        l
    };

    // Author, branches, size (the state is on the PR's header line).
    let author = s(&v["author"]["login"]);
    let mut l = InfoLine::new(false, false);
    l.put(&format!("@{author}"), Some(Mark::Link(format!("https://github.com/{author}"))))
        .put(&format!(" · {} ← {} · ", s(&v["baseRefName"]), s(&v["headRefName"])), None)
        .put(&format!("+{}", v["additions"]), Some(Mark::Pass))
        .put(" ", None)
        .put(&format!("−{}", v["deletions"]), Some(Mark::Fail))
        .put(&format!(" in {} files", v["changedFiles"]), None);
    out.push(l);

    let labels = list(&v["labels"]);
    if !labels.is_empty() {
        let mut l = InfoLine::new(false, false);
        l.put("labels", Some(Mark::Dim));
        for lb in &labels {
            l.put(" ", None).put(&format!(" {} ", s(&lb["name"])), Some(Mark::Chip(s(&lb["color"]))));
        }
        out.push(l);
    }
    let assigned = list(&v["assignees"]);
    if !assigned.is_empty() {
        let mut l = InfoLine::new(false, false);
        l.put("assigned", Some(Mark::Dim));
        for a in &assigned {
            l.put(&format!(" @{}", s(&a["login"])), None);
        }
        out.push(l);
    }
    // The PR's own reactions (Space = on its description reacts).
    // Yours, by node id (`mine_json`).
    let mine = |id: &Value| -> Vec<&str> { id.as_str().and_then(|id| v["dvMine"][id].as_array()).into_iter().flatten().filter_map(Value::as_str).collect() };
    // Whether which of a node's reactions are yours is known (asked of GitHub): else they're
    // dimmed, not shown as not yours.
    let known = |id: &Value| id.as_str().is_some_and(|id| v["dvMine"].get(id).is_some());
    let own = reactions(&v["reactionGroups"], &mine(&v["id"]));
    if !own.is_empty() {
        let mut l = InfoLine::new(false, false);
        l.put("reactions ", Some(Mark::Dim)).put_reactions(&own, known(&v["id"]));
        out.push(l);
    }

    // Who did what, one reviewer a line (no scrolling sideways for them all): "✗ @bo requested
    // changes", "✓ @amy approved", "● @cy requested" (waiting on them); in their colors.
    let reviews = list(&v["latestReviews"]);
    let by = |state: &str| -> Vec<String> { reviews.iter().filter(|r| r["state"] == state).map(|r| s(&r["author"]["login"])).collect() };
    let requested: Vec<String> = list(&v["reviewRequests"]).iter().map(|r| r["login"].as_str().or(r["slug"].as_str()).or(r["name"].as_str()).unwrap_or("?").to_owned()).collect();
    let groups = [
        ("✗", "requested changes", by("CHANGES_REQUESTED"), Mark::Fail),
        ("✓", "approved", by("APPROVED"), Mark::Pass),
        ("●", "requested", requested, Mark::Pending),
    ];
    let mut l = InfoLine::new(false, false);
    l.put("reviews:", None);
    if groups.iter().all(|g| g.2.is_empty()) {
        let decision = s(&v["reviewDecision"]).to_lowercase().replace('_', " ");
        l.put(" ", None).put(if decision.is_empty() { "none yet" } else { &decision }, Some(Mark::Dim));
    }
    out.push(l);
    for (sign, what, who, mark) in groups {
        for name in who {
            let mut l = InfoLine::new(false, false);
            l.put(&format!("{sign} @{name}"), Some(mark.clone())).put(&format!(" {what}"), None);
            out.push(l);
        }
    }

    let checks = list(&v["statusCheckRollup"]);
    if checks.is_empty() {
        out.push(plain("checks: none".into()));
    } else {
        let count = |m| checks.iter().filter(|c| verdict(c) == m).count();
        let (pass, fail, pending) = (count('✓'), count('✗'), count('●'));
        let mut l = InfoLine::new(false, false);
        l.put("checks", Some(Mark::Link(format!("{}/checks", s(&v["url"])))));
        l.put(": ", None);
        // Only the counts that aren't zero.
        let parts = [(pass, "✓", "passed", Mark::Pass), (fail, "✗", "failed", Mark::Fail), (pending, "●", "pending", Mark::Pending)];
        for (k, (n, sym, what, m)) in parts.into_iter().filter(|p| p.0 > 0).enumerate() {
            if k > 0 {
                l.put(" · ", None);
            }
            l.put(&format!("{sym} {n} {what}"), Some(m));
        }
        out.push(l);
        for c in checks.iter().filter(|c| verdict(c) != '✓') {
            let name = c["name"].as_str().or(c["context"].as_str()).unwrap_or("?");
            let m = if verdict(c) == '✗' { Mark::Fail } else { Mark::Pending };
            let mut l = InfoLine::new(false, false);
            l.put(&format!("{} ", verdict(c)), Some(m.clone()));
            // Its run on GitHub (a check run's details, a status's target).
            match c["detailsUrl"].as_str().or(c["targetUrl"].as_str()).filter(|u| !u.is_empty()) {
                Some(u) => l.put(name, Some(Mark::Link(u.to_owned()))),
                None => l.put(name, Some(m)),
            };
            out.push(l);
        }
    }

    // Workflow runs that won't run until a maintainer approves them.
    match v["dvAwaiting"].as_u64().unwrap_or(0) {
        0 => {}
        k => {
            let mut l = InfoLine::new(false, false);
            let s = if k == 1 { "" } else { "s" };
            l.put(&format!("⏸ {k} workflow run{s} awaiting approval"), Some(Mark::Pending)).put(" (Space g approves)", Some(Mark::Dim));
            out.push(l);
        }
    }
    // Its commits signed, or which aren't (and whether its base requires them to be).
    if let Some(cs) = v["dvCommits"].as_array().filter(|c| !c.is_empty()) {
        let unsigned: Vec<String> = cs.iter().filter(|c| c["signed"] != true).map(|c| s(&c["oid"]).chars().take(8).collect()).collect();
        let required = v["dvSignaturesRequired"] == true;
        let mut l = InfoLine::new(false, false);
        l.put("commits: ", None);
        match unsigned.len() {
            0 => l.put(&format!("✓ {} signed", if cs.len() == 1 { "1".to_owned() } else { format!("all {}", cs.len()) }), Some(Mark::Pass)),
            k => l.put(&format!("✗ {k} of {} unsigned: {}", cs.len(), unsigned.join(", ")), Some(if required { Mark::Fail } else { Mark::Pending })),
        };
        // More than GitHub's last 100: the older ones aren't checked.
        let total = v["dvCommitsTotal"].as_u64().unwrap_or(0) as usize;
        if total > cs.len() {
            l.put(&format!(" (the last {} of {total})", cs.len()), Some(Mark::Dim));
        }
        if required {
            let st = if unsigned.is_empty() { Mark::Dim } else { Mark::Fail };
            l.put(" · ", None).put(&format!("{} requires signed commits", s(&v["baseRefName"])), Some(st));
        }
        // GitHub couldn't say this time: what it said last (`keep_signatures`).
        if v["dvCommitsKept"] == true {
            let when = Some(s(&v["dvCommitsAt"])).filter(|t| !t.is_empty()).map_or("an earlier look".to_owned(), |t| ago(&t));
            l.put(&format!(" (as of {when})"), Some(Mark::Dim));
        }
        out.push(l);
    } else if let Some(why) = v["dvCommitsUnknown"].as_str() {
        let mut l = InfoLine::new(false, false);
        l.put("commits: ", None).put(&format!("signatures unknown ({why})"), Some(Mark::Dim));
        out.push(l);
    }

    let heading = |t: String| {
        let mut l = InfoLine::new(true, false);
        l.put(&t, None);
        l
    };
    let markdown = |t: String| {
        let mut l = InfoLine::new(false, true);
        l.put(&t, None);
        l
    };
    // The PR itself takes reactions: its node id on the description's heading.
    out.push(InfoLine { url: s(&v["url"]), node: s(&v["id"]), ..heading("description".into()) });
    let body = s(&v["body"]).replace("\r\n", "\n");
    out.push(markdown(body)); // empty: the view shows a placeholder

    // The conversation: comments, and reviews with something to read (their text, or a
    // verdict: a plain review's inline comments are threads on the code), in time order:
    // (when, who, what they did, link, text).
    // (when, who, what they did, link, text, node id, reactions, why it's hidden).
    type Said<'a> = (String, String, &'a str, String, String, String, Vec<Reaction>, String);
    let mut said: Vec<Said> = list(&v["comments"])
        .iter()
        .map(|c| (s(&c["createdAt"]), s(&c["author"]["login"]), "", s(&c["url"]), s(&c["body"]), s(&c["id"]), reactions(&c["reactionGroups"], &mine(&c["id"])), hidden(c)))
        .collect();
    for r in list(&v["reviews"]) {
        let what = match s(&r["state"]).as_str() {
            "APPROVED" => "✓ approved · ",
            "CHANGES_REQUESTED" => "✗ requested changes · ",
            "DISMISSED" => "reviewed (dismissed) · ",
            "COMMENTED" if !s(&r["body"]).trim().is_empty() => "reviewed · ",
            _ => continue, // pending, or nothing to read here
        };
        // Its own: questions about it (`a`) are kept by it. (The fragment is the review's id:
        // the link opens the PR.)
        let url = format!("{}#review-{}", s(&v["url"]), s(&r["id"]));
        said.push((s(&r["submittedAt"]), s(&r["author"]["login"]), what, url, s(&r["body"]), s(&r["id"]), reactions(&r["reactionGroups"], &mine(&r["id"])), String::new()));
    }
    said.sort_by(|a, b| a.0.cmp(&b.0)); // ISO times sort as text
    out.push(heading(format!("conversation ({})", said.len())));
    for (at, who, what, url, body, node, r, hidden) in said {
        let sure = known(&Value::from(node.as_str()));
        // A verdict in its color, so an approval doesn't read as just another comment.
        let mut h = InfoLine { url, node, hidden, ..InfoLine::new(true, false) };
        let mark = match what {
            "✓ approved · " => Some(Mark::Pass),
            "✗ requested changes · " => Some(Mark::Fail),
            _ => None,
        };
        h.put(&format!("@{who} · "), None).put(what.trim_end_matches(" · "), mark).put(if what.is_empty() { "" } else { " · " }, None);
        h.put(&ago(&at), None);
        if !r.is_empty() {
            h.put(" · ", None).put_reactions(&r, sure);
        }
        out.push(h);
        out.push(markdown(body.replace("\r\n", "\n")));
    }
    for i in 0..out.len() {
        out[i].key = line_key(&out[..=i]);
    }
    out
}

/// What the last of `lines` is (`InfoLine::key`): its node; else the node of the heading it's
/// the text of; else its label, what it says up to its first `:`, `(` or digit (`checks`,
/// `conversation`, `reactions`).
fn line_key(lines: &[InfoLine]) -> String {
    let (l, before) = lines.split_last().unwrap_or_else(|| unreachable!());
    match before.last().filter(|h| h.heading && !l.heading && !h.node.is_empty()) {
        _ if !l.node.is_empty() => l.node.clone(),
        Some(h) => format!("{}/text", h.node),
        None => l.text.split([':', '(']).next().unwrap_or("").split(|c: char| c.is_ascii_digit()).next().unwrap_or("").trim().to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gh::*;
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
}
