//! The PR section's lines from GitHub's answer (`gh::info_json`): its facts, description and
//! activity, as text with marks for the view to color.
use crate::gh::{ago, check_name, gate, hidden, optional, reactions, tally, verdict, Reaction, REACTIONS};
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
    /// Reactions, yours in the accent color (as GitHub lights them).
    fn put_reactions(&mut self, r: &[Reaction]) {
        for (i, (text, mine)) in reaction_parts(r).into_iter().enumerate() {
            self.put(if i == 0 { "" } else { "  " }, None).put(&text, mine.then_some(Mark::Mine));
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
    /// In the PR's activity, what happened that isn't a comment or a review (a push, a label,
    /// a rename, …): a line of its own, no box (the commits of a push, under it).
    pub event: bool,
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
    /// Its base branch, and whether it conflicts with it.
    pub base: String,
    pub conflicts: bool,
    /// An issue's: when it last changed, and the PRs linked as fixing it (number, state, head).
    pub updated: String,
    pub fixes: Vec<(u64, String, String)>,
}

/// The PR section's lines, its title, head, URL and state, its commits GitHub has no valid
/// signature on, (workflow runs awaiting approval, checks failing), and its base branch.
pub fn parse_info(v: &Value) -> Info {
    let s = |k: &str| v[k].as_str().unwrap_or("").to_owned();
    let unsigned = v["dvCommits"].as_array().map(|cs| cs.iter().filter(|c| c["signed"] != true).filter_map(|c| c["oid"].as_str().map(str::to_owned)).collect());
    // (What fails that decides: required checks, when its base names any.)
    let failing = gate(v["statusCheckRollup"].as_array().map(Vec::as_slice).unwrap_or(&[])).1;
    let awaiting = v["dvAwaiting"].as_u64().unwrap_or(0) as usize;
    let fixes = v["dvFixes"].as_array().into_iter().flatten();
    let fixes = fixes.map(|p| (p["number"].as_u64().unwrap_or(0), p["state"].as_str().unwrap_or("").to_owned(), p["headRefOid"].as_str().unwrap_or("").to_owned())).collect();
    Info {
        lines: info_lines(v),
        title: s("title"),
        head: s("headRefOid"),
        url: s("url"),
        state: state(v),
        unsigned,
        awaiting,
        failing,
        base: s("baseRefName"),
        conflicts: s("mergeable") == "CONFLICTING",
        updated: s("updatedAt"),
        fixes,
    }
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
    // Each line with its key (`InfoLine::key`): what it is, whatever it says.
    let mut push = |key: String, mut l: InfoLine| {
        l.key = key;
        out.push(l);
    };
    let plain = |text: String| {
        let mut l = InfoLine::new(false, false);
        l.put(&text, None);
        l
    };

    // An issue's (`gh::issue_json`): what a PR's are, but for its code (branches, size,
    // reviews, checks, commits); and the PRs that would close it.
    let issue = v["__typename"] == "Issue";
    // Author, branches, size (the state is on the PR's header line); an issue's, when.
    let author = crate::gh::login(&v["author"]);
    let mut l = InfoLine::new(false, false);
    l.put(&format!("@{author}"), Some(Mark::Link(crate::gh::page(&v["author"]))));
    match issue {
        true => _ = l.put(&format!(" · opened {}", ago(&s(&v["createdAt"]))), None),
        false => {
            l.put(&format!(" · {} ← {} · ", s(&v["baseRefName"]), s(&v["headRefName"])), None)
                .put(&format!("+{}", v["additions"]), Some(Mark::Pass))
                .put(" ", None)
                .put(&format!("−{}", v["deletions"]), Some(Mark::Fail))
                .put(&format!(" in {} files", v["changedFiles"]), None);
            if v["mergeable"] == "CONFLICTING" {
                l.put(" · ", None).put(&format!("conflicts with {}", s(&v["baseRefName"])), Some(Mark::Fail));
            }
        }
    }
    push("author".into(), l);
    let fixes = list(&v["dvFixes"]);
    if !fixes.is_empty() {
        let mut l = InfoLine::new(false, false);
        l.put("fixed by", Some(Mark::Dim));
        for p in &fixes {
            l.put(" ", None).put(&format!("#{}", p["number"]), Some(Mark::Link(s(&p["url"])))).put(&format!(" ({})", s(&p["state"]).to_lowercase()), Some(Mark::Dim));
        }
        push("fixes".into(), l);
    }

    let labels = list(&v["labels"]);
    if !labels.is_empty() {
        let mut l = InfoLine::new(false, false);
        l.put("labels", Some(Mark::Dim));
        for lb in &labels {
            l.put(" ", None).put(&format!(" {} ", s(&lb["name"])), Some(Mark::Chip(s(&lb["color"]))));
        }
        push("labels".into(), l);
    }
    let assigned = list(&v["assignees"]);
    if !assigned.is_empty() {
        let mut l = InfoLine::new(false, false);
        l.put("assigned", Some(Mark::Dim));
        for a in &assigned {
            l.put(&format!(" @{}", crate::gh::login(a)), None);
        }
        push("assigned".into(), l);
    }
    // The PR's own reactions (Space = on its description reacts), yours lit.
    let activity = &v["dvActivity"];
    let own = reactions(&activity["reactions"]);
    if !own.is_empty() {
        let mut l = InfoLine::new(false, false);
        l.put("reactions ", Some(Mark::Dim)).put_reactions(&own);
        push("reactions".into(), l);
    }

    if !issue {
        // Who did what, one reviewer a line (no scrolling sideways for them all): "✗ @bo requested
        // changes", "✓ @amy approved", "● @cy requested" (waiting on them); in their colors.
        let reviews = list(&v["latestReviews"]);
        let by = |state: &str| -> Vec<String> { reviews.iter().filter(|r| r["state"] == state).map(|r| crate::gh::login(&r["author"])).collect() };
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
        push("reviews".into(), l);
        for (sign, what, who, mark) in groups {
            for name in who {
                let mut l = InfoLine::new(false, false);
                l.put(&format!("{sign} @{name}"), Some(mark.clone())).put(&format!(" {what}"), None);
                push(format!("reviewer/{name}"), l);
            }
        }

        let checks = list(&v["statusCheckRollup"]);
        if checks.is_empty() {
            push("checks".into(), plain("checks: none".into()));
        } else {
            let optional = |c: &&Value| optional(c, &checks);
            let any_required = checks.iter().any(|c| c["isRequired"] == true);
            let mut l = InfoLine::new(false, false);
            l.put("checks", Some(Mark::Link(format!("{}/checks", s(&v["url"])))));
            l.put(": ", None);
            // Only the counts that aren't zero; the base's required checks apart from the others,
            // when it names any.
            let counts = |l: &mut InfoLine, of: &[Value]| {
                let (pass, fail, pending) = tally(of);
                let parts = [(pass, "✓", "passed", Mark::Pass), (fail, "✗", "failed", Mark::Fail), (pending, "●", "pending", Mark::Pending)];
                for (k, (n, sym, what, m)) in parts.into_iter().filter(|p| p.0 > 0).enumerate() {
                    if k > 0 {
                        l.put(" · ", None);
                    }
                    l.put(&format!("{sym} {n} {what}"), Some(m));
                }
            };
            match !any_required {
                true => counts(&mut l, &checks),
                false => {
                    let (opt, req): (Vec<Value>, Vec<Value>) = checks.iter().cloned().partition(|c| optional(&c));
                    l.put("required ", None);
                    counts(&mut l, &req);
                    if !opt.is_empty() {
                        l.put(" · optional ", None);
                        counts(&mut l, &opt);
                    }
                }
            }
            // Its base's required checks unreadable: one not reported yet would be missing here.
            if v["dvRequiredUnknown"] == true {
                l.put(" · can't read which its base requires", Some(Mark::Dim));
            }
            push("checks".into(), l);
            for c in checks.iter().filter(|c| verdict(c) != '✓') {
                let name = check_name(c);
                let m = if verdict(c) == '✗' { Mark::Fail } else { Mark::Pending };
                let mut l = InfoLine::new(false, false);
                l.put(&format!("{} ", verdict(c)), Some(m.clone()));
                // Its run on GitHub (a check run's details, a status's target).
                match c["detailsUrl"].as_str().or(c["targetUrl"].as_str()).filter(|u| !u.is_empty()) {
                    Some(u) => l.put(name, Some(Mark::Link(u.to_owned()))),
                    None => l.put(name, Some(m)),
                };
                if optional(&c) {
                    l.put(" (optional)", Some(Mark::Dim));
                }
                push(format!("check/{name}"), l);
            }
        }

        // Workflow runs that won't run until a maintainer approves them.
        match v["dvAwaiting"].as_u64().unwrap_or(0) {
            0 => {}
            k => {
                let mut l = InfoLine::new(false, false);
                let s = if k == 1 { "" } else { "s" };
                l.put(&format!("⏸ {k} workflow run{s} awaiting approval"), Some(Mark::Pending)).put(" (Space g approves)", Some(Mark::Dim));
                push("awaiting".into(), l);
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
            // GitHub couldn't say this time: what it said last (`keep_known`).
            if v["dvCommitsKept"] == true {
                let when = Some(s(&v["dvCommitsAt"])).filter(|t| !t.is_empty()).map_or("an earlier look".to_owned(), |t| ago(&t));
                l.put(&format!(" (as of {when})"), Some(Mark::Dim));
            }
            push("commits".into(), l);
        } else if let Some(why) = v["dvCommitsUnknown"].as_str() {
            let mut l = InfoLine::new(false, false);
            l.put("commits: ", None).put(&format!("signatures unknown ({why})"), Some(Mark::Dim));
            push("commits".into(), l);
        }
    }

    let heading = |t: String| {
        let mut l = InfoLine::new(true, false);
        l.put(&t, None);
        l
    };
    // The PR itself takes reactions: its node id on the description's heading.
    push("description".into(), InfoLine { url: s(&v["url"]), node: s(&v["id"]), ..heading("description".into()) });
    let body = s(&v["body"]).replace("\r\n", "\n");
    push("description/text".into(), markdown(body)); // empty: the view shows a placeholder

    // Its activity, as GitHub's page has it (`gh::activity`).
    if let Some(why) = activity["error"].as_str() {
        push("activity".into(), plain(format!("activity: not known ({why})")));
        return out;
    }
    let items = activity_items(v, &list(&activity["items"]));
    // (GitHub didn't answer this time: what it said before, `gh::keep_known`.)
    let kept = if activity["kept"] == true { " (as last fetched)" } else { "" };
    push("activity".into(), heading(format!("activity ({}){kept}", items.len())));
    for item in items {
        for (key, l) in item {
            push(key, l);
        }
    }
    out
}

/// `owner/name` of a GitHub URL (`https://github.com/o/r/issues/7`).
fn url_repo(url: &str) -> &str {
    let rest = url.strip_prefix("https://github.com/").unwrap_or("");
    rest.match_indices('/').nth(1).map_or(rest, |(i, _)| &rest[..i])
}

/// The PR section's lines' text, for a live test to look at.
#[cfg(test)]
pub(crate) fn info_lines_of(v: &Value) -> Vec<String> {
    info_lines(v).into_iter().map(|l| l.text).collect()
}

/// A comment's or a review's text, rendered as markdown.
fn markdown(t: String) -> InfoLine {
    let mut l = InfoLine::new(false, true);
    l.put(&t, None);
    l
}

/// Each of the PR's activity `items` (`gh::activity`, oldest first), as its lines with their
/// keys: a comment or a review, its heading (who, what, when, its reactions) then its text; the
/// rest, a line (`event`) each, and a push its commits under it.
fn activity_items(v: &Value, items: &[Value]) -> Vec<Vec<(String, InfoLine)>> {
    let s = |v: &Value| v.as_str().unwrap_or("").to_owned();
    let mut out: Vec<Vec<(String, InfoLine)>> = vec![];
    // A review's comments on the code, and the commit it's of (said if not the PR's head now).
    let review_of = |r: &Value| -> Vec<String> {
        let n = r["comments"]["totalCount"].as_u64().unwrap_or(0);
        let comments = (n > 0).then(|| format!("{n} comment{}", if n == 1 { "" } else { "s" }));
        let older = r["commit"]["oid"].as_str().filter(|o| v["headRefOid"].as_str().is_some_and(|h| h != *o));
        let on = older.map(|_| format!("on {} (an older commit)", s(&r["commit"]["abbreviatedOid"])));
        comments.into_iter().chain(on).collect()
    };
    // A comment, or a review with something to read: (who, what they did, its verdict's color).
    let said = |c: &Value, url: String, what: &str, mark: Option<Mark>, hidden: String| {
        let node = s(&c["id"]);
        let at = c["submittedAt"].as_str().or(c["createdAt"].as_str()).unwrap_or("").to_owned();
        let mut h = InfoLine { url, node: node.clone(), hidden, ..InfoLine::new(true, false) };
        h.put(&format!("@{} · ", crate::gh::login(&c["author"])), None);
        if !what.is_empty() {
            h.put(what, mark).put(" · ", None);
        }
        for m in review_of(c) {
            h.put(&m, Some(Mark::Dim)).put(" · ", None);
        }
        h.put(&ago(&at), None);
        let r = reactions(&c["reactionGroups"]);
        if !r.is_empty() {
            h.put(" · ", None).put_reactions(&r);
        }
        vec![(node.clone(), h), (format!("{node}/text"), markdown(s(&c["body"]).replace("\r\n", "\n")))]
    };
    // What else happened: a dim line, `@who did what · when`.
    let event = |key: String, who: &str, parts: &[(String, Option<Mark>)], at: &str| {
        let mut l = InfoLine { event: true, ..InfoLine::new(true, false) };
        l.put(&format!("● @{who} "), Some(Mark::Dim));
        for (t, m) in parts {
            l.put(t, m.clone().or(Some(Mark::Dim)));
        }
        l.put(&format!(" · {}", ago(at)), Some(Mark::Dim));
        vec![(key, l)]
    };
    let t = |t: &str| (t.to_owned(), None);
    // Another issue or PR, linked: "#9 title" (another repo's, with its name).
    let here = url_repo(v["url"].as_str().unwrap_or(""));
    let other = |x: &Value| -> (String, Option<Mark>) {
        let repo = x["repository"]["nameWithOwner"].as_str().filter(|r| !r.eq_ignore_ascii_case(here)).unwrap_or("");
        let what = x["number"].as_u64().map_or("a deleted or private one".to_owned(), |n| format!("{repo}#{n} {}", s(&x["title"])));
        (what.trim_end().to_owned(), Some(s(&x["url"])).filter(|u| !u.is_empty()).map(Mark::Link))
    };
    for it in items {
        let at = s(&it["createdAt"]);
        let who = &*crate::gh::login(&it["actor"]);
        // Its id (two at once, two labels added, happen at the same second).
        let key = s(&it["id"]);
        let item = match it["__typename"].as_str().unwrap_or("") {
            "IssueComment" => said(it, s(&it["url"]), "", None, hidden(it)),
            "PullRequestReview" => {
                // Its own link: questions about it (`a`) are kept by it. (The fragment is the
                // review's id: the link opens the PR.)
                let url = format!("{}#review-{}", s(&v["url"]), s(&it["id"]));
                let reviewer = &*crate::gh::login(&it["author"]);
                let at = it["submittedAt"].as_str().unwrap_or(&at);
                match s(&it["state"]).as_str() {
                    "APPROVED" => said(it, url, "✓ approved", Some(Mark::Pass), String::new()),
                    "CHANGES_REQUESTED" => said(it, url, "✗ requested changes", Some(Mark::Fail), String::new()),
                    "DISMISSED" => said(it, url, "reviewed (dismissed)", None, String::new()),
                    "COMMENTED" if !s(&it["body"]).trim().is_empty() => said(it, url, "reviewed", None, String::new()),
                    // Only comments on the code: they're threads there.
                    "COMMENTED" => {
                        let more = review_of(it).iter().map(|m| format!(" · {m}")).collect::<String>();
                        event(key, reviewer, &[t(&format!("reviewed{more}"))], at)
                    }
                    _ => continue, // pending: not on GitHub yet
                }
            }
            "PullRequestCommit" => {
                // Pushed: one line for a run of them by the same author, each commit under it.
                let c = &it["commit"];
                let by = c["author"]["user"]["login"].as_str().or(c["author"]["name"].as_str()).unwrap_or("ghost").to_owned();
                let oid = s(&c["abbreviatedOid"]);
                let mut line = InfoLine { event: true, ..InfoLine::new(false, false) };
                line.put("  ", None).put(&oid, Some(Mark::Dim)).put(&format!(" {}", s(&c["messageHeadline"])), None);
                match out.last_mut().filter(|i| i.first().is_some_and(|f| f.0.starts_with(&format!("push/{by}/")))) {
                    Some(push) => {
                        push.push((format!("commit/{oid}"), line));
                        let n = push.len() - 1;
                        let head = &mut push[0].1;
                        let when = head.text.rsplit(" · ").next().unwrap_or("").to_owned();
                        *head = InfoLine { event: true, ..InfoLine::new(true, false) };
                        head.put(&format!("○ @{by} pushed {n} commits · {when}"), Some(Mark::Dim));
                    }
                    None => {
                        let mut head = InfoLine { event: true, ..InfoLine::new(true, false) };
                        head.put(&format!("○ @{by} pushed 1 commit · {}", ago(&s(&c["committedDate"]))), Some(Mark::Dim));
                        out.push(vec![(format!("push/{by}/{oid}"), head), (format!("commit/{oid}"), line)]);
                    }
                }
                continue;
            }
            "HeadRefForcePushedEvent" => {
                let (a, b) = (s(&it["beforeCommit"]["abbreviatedOid"]), s(&it["afterCommit"]["abbreviatedOid"]));
                event(key, who, &[t(&format!("force-pushed {a} → {b}"))], &at)
            }
            k @ ("LabeledEvent" | "UnlabeledEvent") => {
                let what = if k == "LabeledEvent" { "added" } else { "removed" };
                let l = &it["label"];
                event(key, who, &[t(&format!("{what} ")), (format!(" {} ", s(&l["name"])), Some(Mark::Chip(s(&l["color"]))))], &at)
            }
            k @ ("ReviewRequestedEvent" | "ReviewRequestRemovedEvent") => {
                let r = &it["requestedReviewer"];
                let whom = r["login"].as_str().map(|l| format!("@{l}")).or(r["name"].as_str().map(str::to_owned)).unwrap_or("?".into());
                let what = if k == "ReviewRequestedEvent" { "requested a review from" } else { "removed the review request for" };
                event(key, who, &[t(&format!("{what} {whom}"))], &at)
            }
            k @ ("AssignedEvent" | "UnassignedEvent") => {
                let what = if k == "AssignedEvent" { "assigned" } else { "unassigned" };
                event(key, who, &[t(&format!("{what} @{}", crate::gh::login(&it["assignee"])))], &at)
            }
            "RenamedTitleEvent" => event(key, who, &[t(&format!("changed the title: {} → {}", s(&it["previousTitle"]), s(&it["currentTitle"])))], &at),
            "MergedEvent" => {
                let what = format!("merged {} into {}", s(&it["commit"]["abbreviatedOid"]), s(&it["mergeRefName"]));
                event(key, who, &[(what, Some(Mark::Chip("8957e5".into())))], &at)
            }
            "ClosedEvent" => {
                // With what (a PR, a commit), and why (completed, not planned, a duplicate).
                let c = &it["closer"];
                let by = c["number"].as_u64().map(|n| format!("#{n}")).or(c["abbreviatedOid"].as_str().map(str::to_owned));
                let why = s(&it["stateReason"]).to_lowercase().replace('_', " ");
                let mut parts = vec![("closed it".to_owned(), Some(Mark::Fail))];
                if let Some(by) = by {
                    parts.extend([t(" with "), (by, Some(s(&c["url"])).filter(|u| !u.is_empty()).map(Mark::Link))]);
                }
                if !why.is_empty() {
                    parts.push(t(&format!(" ({why})")));
                }
                event(key, who, &parts, &at)
            }
            "ReopenedEvent" => event(key, who, &[("reopened it".into(), Some(Mark::Pass))], &at),
            "ReadyForReviewEvent" => event(key, who, &[t("marked it ready for review")], &at),
            "ConvertToDraftEvent" => event(key, who, &[t("marked it as a draft")], &at),
            "BaseRefChangedEvent" => event(key, who, &[t(&format!("changed the base from {} to {}", s(&it["previousRefName"]), s(&it["currentRefName"])))], &at),
            "CrossReferencedEvent" => event(key, who, &[t("mentioned this in "), other(&it["source"])], &at),
            "MarkedAsDuplicateEvent" => event(key, who, &[t("marked this a duplicate of "), other(&it["canonical"])], &at),
            "ConnectedEvent" => event(key, who, &[t("linked "), other(&it["subject"])], &at),
            "ReviewDismissedEvent" => event(key, who, &[t("dismissed a review")], &at),
            _ => continue,
        };
        out.push(item);
    }
    out
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::gh::*;
    use serde_json::json;

    #[test]
    fn an_issues_facts_and_timeline() {
        // As `gh::issue_json` has it: no branches, reviews, checks or commits; who opened it
        // when, and the PRs that would close it.
        let item = |t: &str, at: &str, rest: serde_json::Value| {
            let mut v = json!({ "__typename": t, "id": format!("{t}{at}"), "createdAt": format!("2026-01-0{at}T00:00:00Z"), "actor": { "login": "bo" } });
            v.as_object_mut().unwrap().extend(rest.as_object().unwrap().clone());
            v
        };
        let v = json!({
            "__typename": "Issue", "url": "https://github.com/o/r/issues/7", "author": { "login": "amy" }, "createdAt": "2026-01-01T00:00:00Z",
            "state": "OPEN", "body": "It panics.", "labels": [{ "name": "bug", "color": "d73a4a" }], "assignees": [],
            "dvFixes": [{ "number": 9, "state": "OPEN", "url": "https://github.com/o/r/pull/9" }],
            "dvActivity": { "items": [
                item("CrossReferencedEvent", "3", json!({ "source": { "number": 9, "title": "Fix the panic", "url": "https://github.com/o/r/pull/9", "repository": { "nameWithOwner": "o/r" } } })),
                item("ClosedEvent", "4", json!({ "stateReason": "COMPLETED", "closer": { "number": 9, "url": "https://github.com/o/r/pull/9" } })),
                item("CrossReferencedEvent", "5", json!({ "source": { "number": 4, "title": "Elsewhere", "url": "https://github.com/o/w/issues/4", "repository": { "nameWithOwner": "o/w" } } })),
                item("MarkedAsDuplicateEvent", "6", json!({ "canonical": { "number": 2, "url": "https://github.com/o/r/issues/2" } })),
                item("ConnectedEvent", "7", json!({ "subject": { "number": 9, "title": "Fix the panic", "url": "https://github.com/o/r/pull/9" } })),
            ] },
        });
        let t = info_lines_of(&v);
        assert!(t[0].starts_with("@amy · opened ") && t[1] == "fixed by #9 (open)" && t[2] == "labels  bug ", "{t:#?}");
        assert!(!t.iter().any(|l| l.starts_with("reviews:") || l.starts_with("checks")), "{t:#?}");
        let events: Vec<&str> = t.iter().filter(|l| l.starts_with("● ")).map(|l| l.rsplit_once(" · ").unwrap().0).collect();
        assert_eq!(events, [
            "● @bo mentioned this in #9 Fix the panic",
            "● @bo closed it with #9 (completed)",
            "● @bo mentioned this in o/w#4 Elsewhere", // (another repo's, named so)
            "● @bo marked this a duplicate of #2",
            "● @bo linked #9 Fix the panic",
        ]);
    }

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
        let mut c = v.clone();
        c["mergeable"] = "CONFLICTING".into();
        assert_eq!(info_lines(&c)[0].text, "@amy · main ← feat · +3 −1 in 2 files · conflicts with main");
        assert_eq!(&t[2..4], ["reviews:", "✗ @bo requested changes"]); // one a line
        assert_eq!(&t[4..7], ["checks: ✓ 1 passed · ✗ 1 failed · ● 1 pending", "✗ test", "● ci/legacy"]);
        let v = serde_json::json!({ "statusCheckRollup": [{ "name": "build", "status": "COMPLETED", "conclusion": "SUCCESS" }] });
        let t: Vec<String> = info_lines(&v).into_iter().map(|l| l.text).collect();
        assert!(t.contains(&"checks: ✓ 1 passed".to_owned())); // no "✗ 0 failed · ● 0 pending"
        // Its base requiring some: those apart from the optional ones, which say so; only what's
        // required decides (an optional one failing isn't the PR failing).
        let v = serde_json::json!({ "statusCheckRollup": [
            { "name": "build", "status": "COMPLETED", "conclusion": "SUCCESS", "isRequired": true },
            { "name": "codecov/patch", "status": "COMPLETED", "conclusion": "FAILURE", "isRequired": false },
        ] });
        let t: Vec<String> = info_lines(&v).into_iter().map(|l| l.text).collect();
        assert!(t.contains(&"checks: required ✓ 1 passed · optional ✗ 1 failed".to_owned()) && t.contains(&"✗ codecov/patch (optional)".to_owned()), "{t:?}");
        assert_eq!(parse_info(&v).failing, 0);
        assert!(t.contains(&"reviews: none yet".to_owned()));
    }

    #[test]
    fn activity_as_github_has_it() {
        let item = |t: &str, more: Value| {
            let mut v = json!({ "__typename": t, "createdAt": "2026-01-01T00:00:00Z", "actor": { "login": "amy" } });
            v.as_object_mut().unwrap().extend(more.as_object().unwrap().clone());
            v
        };
        let commit = |oid: &str, by: &str| item("PullRequestCommit", json!({ "commit": { "abbreviatedOid": oid, "messageHeadline": format!("subject {oid}"), "committedDate": "2026-01-01T00:00:00Z", "author": { "user": { "login": by } } } }));
        let v = json!({
            "url": "https://github.com/o/r/pull/1", "body": "", "headRefOid": "head2",
            "dvActivity": { "reactions": [], "items": [
                item("IssueComment", json!({ "id": "IC_1", "author": { "login": "amy" }, "body": "first", "url": "c1" })),
                commit("aaa1", "bo"), commit("aaa2", "bo"),
                item("LabeledEvent", json!({ "label": { "name": "bug", "color": "d73a4a" } })),
                item("PullRequestReview", json!({ "id": "R2", "author": { "login": "bo" }, "state": "APPROVED", "body": "", "submittedAt": "2026-01-02T00:00:00Z", "comments": { "totalCount": 3 }, "commit": { "oid": "head1", "abbreviatedOid": "head1" } })),
                item("PullRequestReview", json!({ "id": "R3", "author": { "login": "cy" }, "state": "COMMENTED", "body": "", "submittedAt": "2026-01-04T00:00:00Z", "comments": { "totalCount": 1 }, "commit": { "oid": "head2", "abbreviatedOid": "head2" } })),
                item("PullRequestReview", json!({ "author": { "login": "me" }, "state": "PENDING", "body": "draft" })),
                item("SubscribedEvent", json!({})),
            ] },
        });
        let lines = info_lines(&v);
        let text: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
        let at = text.iter().position(|t| t.starts_with("activity")).unwrap();
        // A comment, a push (its two commits under it), a label, an approval, a plain review: not
        // the pending one, nor what GitHub's page doesn't show.
        assert_eq!(text[at], "activity (5)");
        let rest = &text[at + 1..];
        assert!(rest[0].starts_with("@amy · ") && rest[1] == "first");
        assert!(rest[2].starts_with("○ @bo pushed 2 commits · ") && rest[3] == "  aaa1 subject aaa1" && rest[4] == "  aaa2 subject aaa2", "{rest:?}");
        assert!(rest[5].starts_with("● @amy added  bug "));
        // A review's comments on the code, and its commit if not the head now.
        assert!(rest[6].starts_with("@bo · ✓ approved · 3 comments · on head1 (an older commit) · ") && rest[7].is_empty(), "{}", rest[6]);
        assert!(rest[8].starts_with("● @cy reviewed · 1 comment · ") && rest.len() == 9, "{}", rest[8]);
        // Events are lines of their own (no box); each comment and review its own link.
        assert!(lines[at + 3].event && lines[at + 6].event && !lines[at + 1].event);
        assert_eq!(lines[at + 7].url, "https://github.com/o/r/pull/1#review-R2");
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
        // Those with any; yours as GitHub says.
        let g = json!([{ "content": "THUMBS_UP", "reactors": { "totalCount": 2 } }, { "content": "EYES", "reactors": { "totalCount": 0 } }, { "content": "HOORAY", "viewerHasReacted": true, "reactors": { "totalCount": 1 } }]);
        assert_eq!(reactions(&g), [("THUMBS_UP".to_owned(), 2, false), ("HOORAY".to_owned(), 1, true)]);
        assert_eq!(reactions_text(&reactions(&g)), "👍 2  🎉 1");
    }

    #[test]
    fn info_lines_keep_their_keys() {
        let keys = |v: Value| info_lines(&v).into_iter().map(|l| l.key).collect::<Vec<_>>();
        let comment = |i: usize| json!({ "__typename": "IssueComment", "id": format!("IC_{i}"), "author": { "login": "amy" }, "body": "hi", "createdAt": "2026-10-01T00:00:00Z" });
        let pr = |n: usize| json!({ "id": "PR_1", "statusCheckRollup": [], "dvActivity": { "items": (0..n).map(comment).collect::<Vec<_>>() } });
        let (one, two) = (keys(pr(1)), keys(pr(2)));
        // The same lines keep their keys though their text (counts) changed; a comment's text is
        // keyed by its comment.
        for k in ["activity", "IC_0", "IC_0/text", "checks", "description"] {
            assert!(one.iter().any(|x| x == k) && two.iter().any(|x| x == k), "{k}: {one:?} {two:?}");
        }
    }

    #[test]
    fn signatures_kept_when_github_cant_say() {
        let old = json!({ "headRefOid": "abc", "dvCommits": [{ "oid": "abc12345", "signed": true }], "dvCommitsTotal": 1, "dvSignaturesRequired": false, "dvCommitsAt": "2026-10-19T07:00:00Z" });
        let mut new = json!({ "headRefOid": "abc", "dvCommitsUnknown": "timed out" });
        keep_known(&mut new, &old);
        assert!(new["dvCommitsUnknown"].is_null() && new["dvCommitsKept"] == true);
        let line = info_lines(&new).into_iter().find(|l| l.text.starts_with("commits:")).unwrap().text;
        assert!(line.contains("✓ 1 signed") && line.contains("(as of"), "{line}");
        // Pushed since: not kept (other commits); and nothing to keep when GitHub said.
        let mut moved = json!({ "headRefOid": "def", "dvCommitsUnknown": "timed out" });
        keep_known(&mut moved, &old);
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

    /// Live: an issue as the PR view reads it (its query, its activity's, valid on GitHub).
    /// `DV_LIVE_REPO=<a checkout> DV_LIVE_ISSUE=<n> cargo test live_issue -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_issue() {
        std::env::set_current_dir(std::env::var("DV_LIVE_REPO").expect("DV_LIVE_REPO")).unwrap();
        let n: u64 = std::env::var("DV_LIVE_ISSUE").expect("DV_LIVE_ISSUE").parse().unwrap();
        let v = issue_json(n).unwrap();
        for l in info_lines_of(&v) {
            println!("{l}");
        }
        assert!(v["dvActivity"]["error"].is_null(), "{}", v["dvActivity"]["error"]);
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

#[cfg(test)]
#[test]
fn activity_kept_when_github_cant_say() {
    let old = serde_json::json!({ "dvActivity": { "items": [{ "__typename": "IssueComment", "id": "IC_1", "author": { "login": "a" }, "body": "hi" }] } });
    let mut new = serde_json::json!({ "dvActivity": { "error": "timed out" } });
    crate::gh::keep_known(&mut new, &old);
    assert!(info_lines(&new).iter().any(|l| l.text == "activity (1) (as last fetched)"));
}
