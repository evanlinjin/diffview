//! `dv issues` (or `I I` in `dv list`): the repo's open issues, as dv list has its PRs. Claude
//! triages them (`a a`: what kind, how pressing, whether it still holds, what next, whether to
//! close it), a review file each (`reviews/issue-N.json`, as `dv-review`'s "Triage an issue"
//! writes it); tabs: open, yours, and those Claude would close.
use super::*;
use crate::gh::{gh_json, next_page, PAGE};
use serde_json::Value;

/// An open issue, as its card shows it.
#[derive(Clone, Debug, Default)]
pub struct Issue {
    pub number: u64,
    pub title: String,
    pub author: String,
    pub updated: String,
    /// (name, hex color)
    pub labels: Vec<(String, String)>,
    pub comments: u64,
    pub thumbs: u64,
    pub assignees: Vec<String>,
    /// PRs that would close it (they say "fixes #N"): (number, state).
    pub fixed_by: Vec<(u64, String, String)>,
}

/// Claude's triage of an issue (`reviews/issue-N.json`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Triage {
    /// bug, feature, question, docs, chore.
    pub kind: String,
    /// high, medium, low.
    pub priority: String,
    /// What next, in a few words.
    pub next: String,
    /// Why to close it (a duplicate, fixed by #N, …), if it should be.
    pub close: String,
    /// The issue's `updatedAt` when it was triaged (empty: not said).
    pub updated: String,
    /// The PRs linked as fixing it that had merged when it was triaged (Claude judged them): one
    /// merging since has it triaged again.
    pub fixes_seen: Vec<u64>,
}

/// The issues' tabs.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum ITab {
    /// What needs you, in order (Claude's, once it's ranked them).
    #[default]
    Todo,
    Open,
    /// Assigned to you, or opened by you.
    Mine,
    /// Claude's triage would close.
    CanClose,
}

impl Tabs for ITab {
    fn next(self) -> ITab {
        match self {
            ITab::Todo => ITab::Open,
            ITab::Open => ITab::Mine,
            ITab::Mine => ITab::CanClose,
            ITab::CanClose => ITab::Todo,
        }
    }

    fn prev(self) -> ITab {
        self.next().next().next()
    }
}

/// You, whether you may label issues here, and the open issues (`fetch`).
type Fetch = Result<(String, bool, Vec<Issue>)>;

/// How many open issues are fetched (the most recently updated).
pub const ISSUES_MAX: usize = 500;

/// You (your login), whether you may label issues here (GitHub gives that to triage access and
/// up), and the open issues, most recently updated first.
pub fn fetch() -> Fetch {
    let (mut me, mut can_triage, mut all, mut after) = (String::new(), true, vec![], None::<String>);
    while all.len() < ISSUES_MAX {
        let from = after.as_ref().map_or(String::new(), |c| format!(", after: \"{c}\""));
        let q = format!(
            "query($owner: String!, $name: String!) {{ viewer {{ login }} repository(owner: $owner, name: $name) {{ viewerPermission
               issues(states: OPEN, first: 100{from}, orderBy: {{ field: UPDATED_AT, direction: DESC }}) {{ {PAGE} nodes {{
                 number title updatedAt author {{ login }}
                 labels(first: 10) {{ nodes {{ name color }} }} comments {{ totalCount }}
                 reactions(content: THUMBS_UP) {{ totalCount }} assignees(first: 5) {{ nodes {{ login }} }}
                 closedByPullRequestsReferences(first: 20, includeClosedPrs: true) {{ nodes {{ number state headRefOid }} }} }} }} }} }}"
        );
        let v: Value = gh_json(&["api", "graphql", "-F", "owner={owner}", "-F", "name={repo}", "-f", &format!("query={q}")])?;
        crate::gh::refused(&v)?;
        me = v["data"]["viewer"]["login"].as_str().unwrap_or("").to_owned();
        can_triage = !matches!(v["data"]["repository"]["viewerPermission"].as_str(), Some("READ") | None);
        let list = &v["data"]["repository"]["issues"];
        all.extend(list["nodes"].as_array().context("gh api graphql: issues")?.iter().map(issue));
        match next_page(list) {
            Some(c) => after = Some(c),
            None => break,
        }
    }
    Ok((me, can_triage, all))
}

fn issue(i: &Value) -> Issue {
    let s = |v: &Value| v.as_str().unwrap_or("").to_owned();
    let nodes = |k: &str| i[k]["nodes"].as_array().cloned().unwrap_or_default();
    Issue {
        number: i["number"].as_u64().unwrap_or(0),
        title: s(&i["title"]),
        author: crate::gh::login(&i["author"]),
        updated: s(&i["updatedAt"]),
        labels: nodes("labels").iter().map(|l| (s(&l["name"]), s(&l["color"]))).collect(),
        comments: i["comments"]["totalCount"].as_u64().unwrap_or(0),
        thumbs: i["reactions"]["totalCount"].as_u64().unwrap_or(0),
        assignees: nodes("assignees").iter().map(crate::gh::login).collect(),
        fixed_by: nodes("closedByPullRequestsReferences").iter().map(|p| (p["number"].as_u64().unwrap_or(0), s(&p["state"]), s(&p["headRefOid"]))).collect(),
    }
}

/// Claude's triage of issue `n`, if it has one (`dir`: the reviews' directory).
pub fn triage_of(dir: &std::path::Path, n: u64) -> Option<Triage> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(dir.join(format!("issue-{n}.json"))).ok()?).ok()?;
    let s = |k: &str| v[k].as_str().unwrap_or("").trim().to_owned();
    // (Only a closing comment drafted, no triage: not triaged.)
    if s("kind").is_empty() && s("summary").is_empty() {
        return None;
    }
    let fixes_seen = v["fixes_seen"].as_array().map(|a| a.iter().filter_map(Value::as_u64).collect()).unwrap_or_default();
    Some(Triage { kind: s("kind"), priority: s("priority"), next: s("next"), close: s("close"), updated: s("updated"), fixes_seen })
}

/// What issue `i` needs of you (`t`: Claude's triage of it, written at `t_at`, seconds):
/// a PR linked as fixing it merged since Claude looked (whether it does is to check: auto
/// triages it again, as for a change), Claude would close it, assigned to you, a PR would fix it (review that),
/// changed since Claude triaged it, not triaged.
pub fn issue_needs(i: &Issue, t: Option<&Triage>, t_at: Option<i64>, me: &str) -> Vec<crate::todo::Need> {
    use crate::todo::Need;
    let mut out = vec![];
    let merged = i.fixed_by.iter().find(|p| p.1 == "MERGED" && !t.is_some_and(|t| t.fixes_seen.contains(&p.0)));
    if let Some(&(n, ..)) = merged {
        out.push(Need::FixMerged(n));
    }
    if let Some(t) = t.filter(|t| !t.close.is_empty()) {
        out.push(Need::WouldClose(t.close.clone()));
    }
    if !me.is_empty() && i.assignees.iter().any(|a| a == me) {
        out.push(Need::Assigned);
    }
    if let Some(&(n, ..)) = i.fixed_by.iter().find(|p| p.1 == "OPEN") {
        out.push(Need::FixOpen(n));
    }
    // Changed since: by the issue's own time when it was triaged (the file's, for one from before
    // that was kept).
    match (t, t_at) {
        (None, _) => out.push(Need::Untriaged),
        (Some(t), _) if !t.updated.is_empty() => {
            if crate::gh::epoch_secs(&i.updated) > crate::gh::epoch_secs(&t.updated) {
                out.push(Need::Retriage);
            }
        }
        (Some(_), Some(at)) if crate::gh::epoch_secs(&i.updated).is_some_and(|u| u > at) => out.push(Need::Retriage),
        _ => {}
    }
    out
}

/// Issue `i`, a line for Claude ranking the todo list: what it is, what it needs, its triage.
pub fn about_issue(it: &crate::todo::Item, i: &Issue, t: Option<&Triage>) -> String {
    let mut v = serde_json::json!({
        "key": it.key(),
        "title": i.title,
        "author": i.author,
        "updated": gh::ago(&i.updated),
        "labels": i.labels.iter().map(|l| l.0.clone()).collect::<Vec<_>>(),
        "comments": i.comments,
        "thumbs_up": i.thumbs,
        "needs": it.needs.iter().map(crate::todo::Need::says).collect::<Vec<_>>(),
    });
    if let Some(t) = t {
        v["triage"] = serde_json::json!({ "kind": t.kind, "priority": t.priority, "next": t.next, "close": t.close });
    }
    v.to_string()
}

/// The command Claude triages issue `n` with (`fixes`: the PRs linked as fixing it; `extra`:
/// your instructions), in the checkout at `root`, reading only: its answer is the triage, as
/// JSON, for `save_triage` (dv list's and the issue's view alike).
pub(crate) fn triage_command(n: u64, fixes: &[(u64, String, String)], root: &std::path::Path, extra: &str) -> std::process::Command {
    let base = crate::diff::remote_branch(&crate::diff::default_branch());
    let prompt = format!(
        "{}Use the dv-review skill to triage issue #{n} (see \"Triage an issue\"); the code as it is on the base branch \
         is `{base}` (`git show {base}:<path>`, `git log {base}`): this checkout may be on another branch. Reply with \
         only the JSON object.{} (It's {} now.)",
        crate::claude::notes_preamble(),
        match fixes {
            [] => String::new(),
            f => format!(
                " PRs linked as fixing it: {}. A merged one may not fix it (only part of it, or on another branch): check \
                 the base branch before saying to close it.",
                fixes_text(f)
            ),
        },
        crate::claude::now()
    );
    let prompt = crate::claude::with_instructions(&prompt, extra);
    let tools = ["Bash(gh issue view:*)", "Bash(gh issue list:*)", "Bash(gh search:*)", "Bash(gh label list:*)"];
    let mut cmd = crate::claude::claude_reader(crate::claude::Work::Triage, &prompt, root, &tools);
    cmd.stdin(std::process::Stdio::null());
    cmd
}

/// What a triage run of issue `n` said (`claude -p`'s JSON, `v`), merged into its file (as of
/// `updated`, the issue's; `fixes`, as asked): how many turns it took, what it cost.
pub(crate) fn save_triage(n: u64, updated: &str, fixes: &[(u64, String, String)], v: &Value) -> Result<String> {
    let file = crate::claude::Subject::Issue(n).path()?;
    let answer = v["result"].as_str().unwrap_or("");
    let t = crate::claude::json_in(answer, '{').filter(Value::is_object).context("no triage in Claude's answer")?;
    let old: Value = std::fs::read_to_string(&file).ok().and_then(|o| serde_json::from_str(&o).ok()).unwrap_or_default();
    let t = merge_triage(&old, t, n, updated, fixes);
    std::fs::create_dir_all(file.parent().context("no dir")?)?;
    crate::claude::write(&file, &t)?;
    Ok(format!("{} turns, ${:.2}", v["num_turns"].as_u64().unwrap_or(0), v["total_cost_usd"].as_f64().unwrap_or(0.0)))
}

/// Claude triages issue `n` here (dv issues'): its hold on it meanwhile (`claude::claim`), for
/// the issue's view (and any dv) to see.
fn triage_run(n: u64, updated: &str, fixes: &[(u64, String, String)], root: &std::path::Path, extra: &str) -> Result<batch::Status> {
    let mut claim = match crate::claude::claim(&crate::claude::pr_key(n), "triaging", None)? {
        Ok(c) => c,
        Err(w) => return Ok(batch::Status::Busy(format!("Claude is {w} on it"), None)),
    };
    let mut cmd = triage_command(n, fixes, root, extra);
    let child = cmd.stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().context("is Claude Code (`claude`) installed?")?;
    claim.set_pid(child.id());
    // (No checkout of its own: it only reads, from git's objects; a merge can switch it.)
    let out = child.wait_with_output()?;
    drop(claim);
    let v: Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
    let answer = v["result"].as_str().unwrap_or("");
    // (Its own word for what went wrong, where it said one.)
    let why = if answer.is_empty() { String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("").to_owned() } else { answer.lines().next().unwrap_or("").to_owned() };
    anyhow::ensure!(out.status.success() && v["is_error"] == false, "Claude stopped ({}): {why}", out.status);
    save_triage(n, updated, fixes, &v).map(batch::Status::Done)
}

/// What an issue's triage says (Claude's word on it, each triage anew).
const TRIAGE_KEYS: [&str; 10] = ["kind", "priority", "summary", "next", "labels", "title", "close", "closing", "reason", "duplicate_of"];

/// A new triage `t` of issue `n` (as of `updated`, with the PRs linked as fixing it) in its
/// file as it was (`old`): what the last triage said goes, the rest stays (the talk with
/// Claude, its proposals). A closing comment drafted for it stays too while this triage still
/// says to close it and has none of its own; and a title proposed with a new text you asked
/// for (`"title"` with `"description"`), till you approve or drop it.
fn merge_triage(old: &Value, t: Value, n: u64, updated: &str, fixes: &[(u64, String, String)]) -> Value {
    let said = |v: &Value, k: &str| v[k].as_str().is_some_and(|c| !c.trim().is_empty());
    let keep_closing = said(&t, "close") && !said(&t, "closing") && said(old, "closing");
    let proposing = said(old, "description");
    let mut m = old.as_object().cloned().unwrap_or_default();
    let mut t = t.as_object().cloned().unwrap_or_default();
    for k in TRIAGE_KEYS.into_iter().filter(|&k| !(proposing && k == "title")) {
        m.remove(k);
    }
    if proposing {
        t.remove("title");
    }
    m.extend(t);
    if keep_closing {
        for k in ["closing", "reason", "duplicate_of"] {
            m.insert(k.into(), old[k].clone());
        }
    }
    m.insert("issue".into(), n.into());
    m.insert("updated".into(), updated.into());
    // (When Claude wrote it, as a PR review's: its age, its place among what Claude said.)
    m.insert("created".into(), crate::claude::now().into());
    // (The merged fixes it judged: one merging after has it triaged again.)
    m.insert("fixes_seen".into(), fixes.iter().filter(|p| p.1 == "MERGED").map(|p| p.0).collect::<Vec<_>>().into());
    Value::Object(m)
}

#[test]
fn a_triage_keeps_the_rest_of_the_file() {
    let old = serde_json::json!({ "summary": "old", "close": "fixed", "closing": "Thanks!", "reason": "completed", "next": "stale",
        "comments": [{ "conversation": "summary", "body": "why?" }], "description": "proposed" });
    // A new word: the old one's keys go (`next`), the talk and proposals stay.
    let t = merge_triage(&old, serde_json::json!({ "summary": "new", "close": "fixed by #9" }), 7, "u", &[(9, "MERGED".into(), String::new())]);
    assert_eq!((t["summary"].as_str(), t["next"].as_str(), t["description"].as_str()), (Some("new"), None, Some("proposed")));
    assert_eq!(t["comments"][0]["body"], "why?");
    // Still to close, with no comment of its own: the drafted one kept.
    assert_eq!((t["closing"].as_str(), t["reason"].as_str(), t["fixes_seen"][0].as_u64()), (Some("Thanks!"), Some("completed"), Some(9)));
    // Not to close now: no closing comment.
    assert!(merge_triage(&old, serde_json::json!({ "summary": "keep it" }), 7, "u", &[])["closing"].is_null());
    // A title proposed with a new text you asked for stays (the triage's own one waits).
    let old = serde_json::json!({ "title": "Asked for", "description": "proposed" });
    assert_eq!(merge_triage(&old, serde_json::json!({ "title": "Triage's" }), 7, "u", &[])["title"], "Asked for");
}

/// Whether `k` is for moving about, the tabs, the filter, the help (`Nav`) while `pending`
/// waits for its second key: so it calls that off; Esc only that.
fn moves(pending: Option<char>, k: KeyCode) -> bool {
    !(pending.is_some() && k == KeyCode::Esc)
}

#[test]
fn a_waiting_key_called_off_by_moving() {
    let j = KeyCode::Char('j');
    assert!(moves(None, j) && moves(Some('a'), j) && moves(None, KeyCode::Esc));
    assert!(!moves(Some('a'), KeyCode::Esc));
}

/// An issue to triage, as listed: its `updatedAt` (what it's triaged as of), and the PRs linked
/// as fixing it.
struct Ask {
    n: u64,
    updated: String,
    fixes: Vec<(u64, String, String)>,
}

impl batch::Numbered for Ask {
    fn number(&self) -> u64 {
        self.n
    }
}

/// Claude triages each of `asks`, a couple at a time, in the background: its news on the receiver.
fn triage_all(asks: Vec<Ask>, root: PathBuf, extra: String) -> Receiver<(u64, batch::Status)> {
    batch::pool(asks, move |a, tx| {
        _ = tx.send((a.n, batch::Status::Reviewing));
        (a.n, triage_run(a.n, &a.updated, &a.fixes, &root, &extra).unwrap_or_else(|e| batch::Status::failed(format!("{e:#}"))))
    })
}

/// The PRs linked as fixing an issue, said: "#9 (merged), #12 (open)".
fn fixes_text(fixes: &[(u64, String, String)]) -> String {
    fixes.iter().map(|(n, s, _)| format!("#{n} ({})", s.to_lowercase())).collect::<Vec<_>>().join(", ")
}

/// What the issues list asks of its caller.
pub enum IssuesDo {
    Quit,
    /// Back to the PRs (`I I`).
    Prs,
    /// Issue N, in the PR view (Enter on its card), then back here.
    Issue(u64),
}

#[derive(Default)]
pub struct Issues {
    ui: Ui,
    items: Vec<Issue>,
    me: String,
    /// Whether you may label issues here (triage access and up).
    can_triage: bool,
    /// The card selected, the filter, the tab (`nav.rs`).
    nav: Nav<ITab>,
    /// The first of a two-key command (`g`, `a`, `I`, `U`), waiting for the second.
    pending: Option<char>,
    marked: HashSet<u64>,
    /// Claude's triage of each issue (from its review file), and the runs under way.
    triage: HashMap<u64, Triage>,
    /// When each triage was written (seconds): an issue changed since is to triage again.
    triage_at: HashMap<u64, i64>,
    /// The todo tab: what needs you, Claude's ranking, what's deferred (`todos.rs`).
    todos: Todos,
    /// Issues a draft of Claude's (unpublished: `N N`'s) is to fix: its label, whether Claude's
    /// at it now. Not on the todo till it's done: nothing for you there meanwhile.
    draft_fix: HashMap<u64, (String, Option<String>)>,
    running: HashMap<u64, batch::Status>,
    /// Who asked each issue's last triage, as dv list's `batch_extra` (each overwrites it): you (`a a`, your instructions) or
    /// auto (`None`). Yours that Claude was at already is held (`Busy`) and asked again once it
    /// isn't; counted as dv's work (`held`).
    asked: HashMap<u64, Option<String>>,
    held: Option<batch::Work>,
    running_rx: Vec<Receiver<(u64, batch::Status)>>,
    reviews_dir: Option<PathBuf>,
    reviews_mtime: Option<std::time::SystemTime>,
    /// When the drafts fixing issues were last looked at (`watch_running`), and what Claude's
    /// doing, by key, then (`claude::sweep_running`).
    drafts_checked: Option<std::time::Instant>,
    at_work: HashMap<String, String>,
    fetched: Option<Receiver<Fetch>>,
    /// A closing comment Claude's drafting (`batch`'s, as for a PR's), on its way; one at a time.
    claude_rx: Option<Receiver<(u64, batch::Status)>>,
    /// The issues with a closing comment drafted (read as the triages are, as they change).
    closing: HashSet<u64>,
    /// Auto (`C`, git config `dv.auto`, as dv list's): Claude triages what isn't triaged, and
    /// what's changed since, and ranks the todo again. Whether it's started (the first fetch
    /// in), what it's done (once each), and when it last ranked.
    started: bool,
    auto_done: HashSet<(u64, String)>,
    root: PathBuf,
    /// The window (or zellij pane) isn't focused: an issue's cursor isn't drawn.
    unfocused: bool,
    pub msg: String,
}

impl Issues {
    pub fn new() -> Issues {
        Issues {
            ui: Theme::load().ui(),
            todos: Todos::of(crate::todo::Of::Issues),
            reviews_dir: crate::claude::dv_dir().ok().map(|d| d.join("reviews")),
            root: std::env::current_dir().unwrap_or_default(),
            ..Default::default()
        }
    }

    /// Fetch the issues in the background (the first time: the list says it's loading).
    fn refresh(&mut self) {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || _ = tx.send(fetch()));
        self.fetched = Some(rx);
    }

    /// What Claude's doing, every couple of seconds (`claude::sweep_running`, as dv list's): a
    /// draft fixing an issue started, redrafted or done from anywhere, or Claude at it there or
    /// not, moves its issue; auto's picks go by it too.
    fn watch_running(&mut self) {
        if self.drafts_checked.is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(2)) {
            return;
        }
        self.drafts_checked = Some(std::time::Instant::now());
        let Some(dir) = self.reviews_dir.as_ref().and_then(|d| d.parent()).map(|d| d.join("running")) else { return };
        // (Claude at an issue, or a draft fixing one, started or done: the todo again.)
        let was = std::mem::take(&mut self.at_work);
        self.at_work = crate::claude::sweep_running(&dir, &std::cell::OnceCell::new());
        if self.at_work != was || draft_fix(&records(), &self.at_work) != self.draft_fix {
            self.find_todo();
        }
        // Yours held back: asked again, as you asked, once Claude's done there (`batch::held`).
        let free: Vec<(u64, String)> = self.running.keys().filter(|&&n| self.held_yours(n) && !batch::held(n, None)).filter_map(|&n| Some((n, self.asked.get(&n).cloned().flatten()?))).collect();
        for (n, extra) in free {
            self.triage(vec![n], Some(extra));
        }
    }

    /// Claude's triages, read again when the reviews' directory changes.
    fn watch_reviews(&mut self) {
        let Some(dir) = &self.reviews_dir else { return };
        let mtime = std::fs::metadata(dir).and_then(|m| m.modified()).ok();
        if mtime == self.reviews_mtime {
            return;
        }
        self.reviews_mtime = mtime;
        self.triage = self.items.iter().filter_map(|i| Some((i.number, triage_of(dir, i.number)?))).collect();
        let at = |n: u64| std::fs::metadata(dir.join(format!("issue-{n}.json"))).ok()?.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_secs() as i64);
        self.triage_at = self.triage.keys().filter_map(|&n| Some((n, at(n)?))).collect();
        // (A closing comment can be drafted with no triage: Claude's ranking said close.)
        let closing = |n: u64| std::fs::read_to_string(dir.join(format!("issue-{n}.json"))).ok().is_some_and(|t| serde_json::from_str::<Value>(&t).is_ok_and(|v| v["closing"].as_str().is_some_and(|c| !c.trim().is_empty())));
        self.closing = self.items.iter().map(|i| i.number).filter(|&n| closing(n)).collect();
        self.find_todo();
    }

    /// The todo tab's items: each issue's needs, in Claude's order where it's ranked them (the
    /// rules' otherwise), deferred ones at the bottom till then (or till what they need changes).
    fn find_todo(&mut self) {
        use crate::todo::{Item, Kind};
        // (The cursor stays on its issue as they reorder: a ranking, a triage landing.)
        let keep = self.shown().get(self.nav.sel).map(|i| i.number);
        let items: Vec<Item> = self
            .items
            .iter()
            .filter_map(|i| {
                let needs = issue_needs(i, self.triage.get(&i.number), self.triage_at.get(&i.number).copied(), &self.me);
                (!needs.is_empty()).then(|| Item { kind: Kind::Issue, number: i.number, needs, claude: None, updated: i.updated.clone(), reason: None, changed: false, deferred: None, busy: None, waits: None })
            })
            .collect();
        // (A draft fixing it, unpublished: its record's task starts "Fix #N", as `N N` has it.)
        self.draft_fix = draft_fix(&records(), &self.at_work);
        let items: Vec<Item> = items.into_iter().filter(|it| !self.draft_fix.contains_key(&it.number)).collect();

        let met = self.todos.met(&items, |_, c, _| self.happened(c));
        self.todos.ready = !self.items.is_empty();
        let (items, said) = self.todos.settle(items, met);
        // Those Claude is at (here or in any dv): at the bottom till it's done, as dv list's.
        let doing = |it: &Item| self.at_work.get(&crate::claude::pr_key(it.number)).cloned();
        let (mut busy, mut items): (Vec<Item>, Vec<Item>) = items.into_iter().partition(|it| doing(it).is_some());
        busy.iter_mut().for_each(|it| it.busy = doing(it));
        items.append(&mut busy);
        self.todos.items = items;
        if let Some(said) = said {
            self.msg = said;
        }
        if let Some(i) = keep.and_then(|n| self.shown().iter().position(|i| i.number == n)) {
            self.nav.sel = i;
        }
    }

    /// Auto: Claude triages what's new or changed since dv started (and what changed since its
    /// triage), `AT_ONCE` at a time, once each; and ranks the todo again once something's new or
    /// changed, at most every quarter hour.
    fn auto_run(&mut self) {
        if !self.todos.auto || !self.started || self.fetched.is_some() {
            return;
        }
        let busy = self.running.values().any(batch::Status::running);
        let run = if busy { vec![] } else { self.auto_picks() };
        let started = !run.is_empty();
        if started {
            self.say(format!("auto: Claude is triaging {}", run.iter().map(|n| format!("#{n}")).collect::<Vec<_>>().join(", ")));
            self.triage(run, None);
        }
        // An issue Claude would close with no closing comment drafted yet: drafted, one at a time,
        // once each (its view closes with it, your editor first).
        if self.claude_rx.is_none() && !started {
            let drafted = |n: u64| self.closing.contains(&n);
            // (Not what you've put aside.)
            let deferred: HashSet<u64> = self.todos.items.iter().filter(|it| it.deferred.is_some()).map(|it| it.number).collect();
            let next = self.items.iter().map(|i| (i.number, i.updated.clone())).find(|(n, u)| {
                !deferred.contains(n) && self.close_why(*n).is_some() && !drafted(*n) && !self.auto_done.contains(&(*n, format!("closing@{u}"))) && !self.at_work.contains_key(&crate::claude::pr_key(*n))
            });
            if let Some((n, u)) = next {
                self.auto_done.insert((n, format!("closing@{u}")));
                self.claude_rx = Some(batch::start_issue(n, batch::Task::Close, self.root.clone()));
                self.say(format!("auto: Claude is drafting a comment closing #{n}"));
            }
        }
        if !busy && !started && self.todos.auto_rank_due() {
            self.rank(false);
        }
    }

    /// What auto triages next (`AT_ONCE` at most, each once at an `updatedAt`): what isn't
    /// triaged, and what's changed since its triage, in the todo's order.
    fn auto_picks(&mut self) -> Vec<u64> {
        let mut run = vec![];
        for it in self.todos.items.iter().filter(|it| it.deferred.is_none()) {
            let Some(i) = self.items.iter().find(|i| i.number == it.number) else { continue };
            use crate::todo::Need::*;
            let due = it.needs.iter().any(|n| matches!(n, Retriage | Untriaged | FixMerged(_)));
            // Once each as it'd be triaged: its updatedAt, and the fixes merged (a merge doesn't
            // move the issue's time).
            let merged: Vec<u64> = i.fixed_by.iter().filter(|p| p.1 == "MERGED").map(|p| p.0).collect();
            // (Not one Claude's at elsewhere just now: on a later look, once it's done.)
            // (Not one Claude's at (`watch_running`'s), nor one asked and not done: under way, or
            // yours held, asked again by `watch_running` alone.)
            let free = || !self.at_work.contains_key(&crate::claude::pr_key(i.number)) && !self.running.get(&i.number).is_some_and(|s| s.running() || matches!(s, batch::Status::Busy(..)));
            if due && run.len() < batch::AT_ONCE && free() && self.auto_done.insert((i.number, format!("{}{merged:?}", i.updated))) {
                run.push(i.number);
            }
        }
        run
    }

    /// News of Claude's own (auto, a triage or ranking done), said unless something's being said
    /// or asked already (a `q again`, what you just did).
    fn say(&mut self, m: String) {
        if self.pending.is_none() && self.msg.is_empty() {
            self.msg = m;
        }
    }

    /// Why Claude would close issue `n`: its triage says, or its ranking of the todo did.
    fn close_why(&self, n: u64) -> Option<String> {
        let triage = self.triage.get(&n).map(|t| t.close.clone()).filter(|c| !c.is_empty());
        triage.or_else(|| self.todos.ranked.as_ref()?.closes.get(&format!("issue-{n}")).cloned())
    }

    /// Issues auto would triage: not triaged, not deferred.
    fn untriaged(&self) -> usize {
        self.todos.items.iter().filter(|it| it.deferred.is_none() && it.needs.contains(&crate::todo::Need::Untriaged)).count()
    }

    /// Whether you asked issue `n`'s last triage (`a a`), not auto.
    fn yours(&self, n: u64) -> bool {
        self.asked.get(&n).is_some_and(Option::is_some)
    }

    /// Yours held back: Claude was at it, asked again once it isn't.
    fn held_yours(&self, n: u64) -> bool {
        matches!(self.running.get(&n), Some(batch::Status::Busy(..))) && self.yours(n)
    }

    /// Claude at work from here: triaging, drafting a fix or a closing comment, ranking.
    pub fn busy(&self) -> bool {
        // (All of this dv's, dv list's and a view's left at work too: `batch::at_work`.)
        batch::at_work()
    }

    /// The issues on the tab shown, filtered.
    pub fn shown(&self) -> Vec<&Issue> {
        if self.nav.tab == ITab::Todo {
            return self.todos.items.iter().filter_map(|it| self.items.iter().find(|i| i.number == it.number)).filter(|i| self.matches(i)).collect();
        }
        self.items
            .iter()
            .filter(|i| match self.nav.tab {
                ITab::Todo => true,
                ITab::Open => true,
                ITab::Mine => i.author == self.me || i.assignees.contains(&self.me),
                ITab::CanClose => self.close_why(i.number).is_some(),
            })
            .filter(|i| self.matches(i))
            .collect()
    }

    /// Whether `i` matches the filter: every term (`label:`, `author:`, `kind:`, `is:triaged`,
    /// `-term` excludes; else in its number, title, author or labels).
    fn matches(&self, i: &Issue) -> bool {
        let t = self.triage.get(&i.number);
        self.nav.query.split_whitespace().all(|term| {
            filter_term(
                term,
                |k, v, has| {
                    Some(match k {
                        "label" => i.labels.iter().any(|l| has(&l.0, v)),
                        "author" => has(&i.author, v),
                        "kind" => t.is_some_and(|t| has(&t.kind, v)),
                        "priority" => t.is_some_and(|t| has(&t.priority, v)),
                        "is" if v == "triaged" => t.is_some(),
                        _ => return None,
                    })
                },
                || format!("#{} {} @{} {}", i.number, i.title, i.author, i.labels.iter().map(|l| l.0.as_str()).collect::<Vec<_>>().join(" ")),
            )
        })
    }

    /// Show the issues until you quit or go back to the PRs.
    pub fn run(&mut self, term: &mut DefaultTerminal) -> Result<IssuesDo> {
        // (As kept: `C` in dv list may have changed it.)
        self.todos.auto = Todos::auto_setting();
        if self.items.is_empty() {
            self.msg = "fetching the open issues…".into();
        }
        self.refresh();
        loop {
            term.draw(|f| self.draw(f))?;
            if let Some(Ok(res)) = self.fetched.as_ref().map(Receiver::try_recv) {
                self.fetched = None;
                match res {
                    Ok((me, can_triage, items)) => {
                        let keep = self.shown().get(self.nav.sel).map(|i| i.number);
                        (self.me, self.can_triage, self.items) = (me, can_triage, items);
                        // (Deferrals of issues no longer open: unless past the cap, unlisted.)
                        let all_in = self.items.len() < ISSUES_MAX;
                        let open = |n: u64| self.items.iter().any(|i| i.number == n);
                        if all_in
                            && let Some(e) = self.todos.prune(|k| k.strip_prefix("issue-").and_then(|n| n.parse().ok()).is_none_or(open))
                        {
                            self.msg = e;
                        }
                        let first = !self.started;
                        // (Not a ranking straight away: once the triages are in, a quarter hour on.)
                        if first {
                            self.started = true;
                            self.todos.hold_auto_rank();
                        }
                        self.reviews_mtime = None; // read the triages for these
                        self.watch_reviews();
                        // Auto on (from dv list too: one setting): what it's about to cost, said (once
                        // the triages there are read).
                        if first && self.todos.auto && self.untriaged() > 0 {
                            self.msg = format!("auto is on: Claude triages the {} issue(s) not triaged, two at a time (~$0.30 each; C turns it off)", self.untriaged());
                        }
                        self.nav.sel = keep.and_then(|n| self.shown().iter().position(|i| i.number == n)).unwrap_or(0);
                        if self.msg.starts_with("fetching") {
                            self.msg.clear();
                        }
                    }
                    Err(e) => self.msg = format!("issues: {e:#}"),
                }
            }
            self.watch_reviews();
            self.watch_running();
            self.auto_run();
            // Claude's reading of a deferral's "until …".
            match self.todos.until_news() {
                Some(Ok((key, c, or))) => self.defer_on(&key, c, or),
                Some(Err(said)) => self.msg = said,
                None => {}
            }
            // A deferral (or a wait) whose time's come: back.
            if self.todos.due(crate::claude::now_secs()) {
                self.find_todo();
            }
            if let Some(res) = self.todos.rank_news() {
                match res {
                    Ok(cost) => {
                        self.say(format!("Claude ranked what needs you (${cost:.2})"));
                        self.find_todo();
                    }
                    Err(e) => self.msg = format!("not ranked: {e:#}"),
                }
            }
            for (n, st) in self.claude_rx.iter().flat_map(|rx| rx.try_iter()).collect::<Vec<_>>() {
                match st {
                    batch::Status::Done(took) => {
                        (self.msg, self.claude_rx, self.reviews_mtime) = (format!("Claude drafted a comment closing #{n} ({took}): Enter opens it, Space a closes it with it"), None, None);
                    }
                    batch::Status::Failed(e, _) => (self.msg, self.claude_rx) = (format!("#{n}: no closing comment: {}", e.lines().next().unwrap_or("")), None),
                    // Claude at it elsewhere: drafted on a later look.
                    batch::Status::Busy(w, _) => {
                        (self.claude_rx, self.msg) = (None, format!("#{n}: no closing comment drafted now: {w} (on a later look)"));
                        self.auto_done.retain(|d| d.0 != n || !d.1.starts_with("closing@"));
                    }
                    _ => {}
                }
            }
            for (n, st) in self.running_rx.iter().flat_map(|rx| rx.try_iter()).collect::<Vec<_>>() {
                {
                    match &st {
                        // Claude at it elsewhere: not triaged now, and not done for auto either
                        // (asked again on a later look).
                        batch::Status::Busy(w, _) if self.yours(n) => self.msg = format!("#{n}: {w}: triaged as you asked once it's done there"),
                        batch::Status::Busy(w, _) => {
                            self.msg = format!("#{n}: not triaged now: {w}");
                            self.running.remove(&n);
                            self.auto_done.retain(|d| d.0 != n || d.1.starts_with("closing@"));
                            continue;
                        }
                        batch::Status::Failed(e, _) => self.msg = format!("#{n}: {}", e.lines().next().unwrap_or("")),
                        batch::Status::Done(took) => {
                            self.say(format!("triaged #{n} ({took})"));
                            self.reviews_mtime = None; // (read again: written over, the dir may not say)
                        }
                        _ => {}
                    }
                    self.running.insert(n, st);
                }
            }
            // (Counted as dv's work while any is held: after the news, before a key can leave.)
            let holds = self.running.keys().any(|&n| self.held_yours(n));
            if holds != self.held.is_some() {
                self.held = holds.then(batch::Work::new);
            }
            if !event::poll(Duration::from_millis(100))? {
                continue;
            }
            let k = match event::read()? {
                Event::Key(k) if k.kind == KeyEventKind::Press => k,
                Event::FocusLost => {
                    self.unfocused = true;
                    continue;
                }
                Event::FocusGained => {
                    self.unfocused = false;
                    continue;
                }
                // The wheel moves the cursor, as j / k; a click picks a tab or a card, twice opens
                // it (as dv list's).
                Event::Mouse(m) => match m.kind {
                    event::MouseEventKind::ScrollDown => KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
                    event::MouseEventKind::ScrollUp => KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE),
                    _ if self.click(m) => KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                    _ => continue,
                },
                _ => continue,
            };
            if let Some(d) = self.key(term, k) {
                return Ok(d);
            }
        }
    }

    /// `q` (and Ctrl-c): asked again first while Claude's at work here.
    fn quit(&mut self, pending: Option<char>) -> Option<IssuesDo> {
        if !self.busy() || pending == Some('q') {
            return Some(IssuesDo::Quit);
        }
        (self.pending, self.msg) = (Some('q'), "Claude is still at work in dv (triaging, ranking; here or on PRs in dv list): q again quits (what's under way is lost)".into());
        None
    }

    fn key(&mut self, term: &mut DefaultTerminal, k: KeyEvent) -> Option<IssuesDo> {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let pending = self.pending.take();
        // (Ctrl-c is q, wherever: asked first while Claude's at work here, as dv list's.)
        if ctrl && k.code == KeyCode::Char('c') {
            return self.quit(pending);
        }
        // Ctrl keys: only Ctrl-d and Ctrl-u (half a page); no Ctrl+letter is the letter, as in
        // dv list. (It calls off what waits for a key, as any other.)
        if ctrl {
            self.todos.deferring = None;
            self.nav.g = false;
            if matches!(k.code, KeyCode::Char('d' | 'u')) {
                self.nav.key(k, self.shown().len());
            }
            return None;
        }
        if std::mem::take(&mut self.nav.help) {
            return None; // any key closes it
        }
        if self.nav.filter_key(k) {
            return None;
        }
        let n = self.shown().len();
        let this = self.shown().get(self.nav.sel).map(|i| i.number);
        self.msg.clear();
        // After `s`: how long to defer it for, or until what (any other key: not).
        match self.todos.deferring_key(k.code) {
            Some(After::Deferred(said)) => {
                self.msg = said;
                self.save_defer();
                return None;
            }
            Some(After::Until(key)) => {
                let what = "defer it until… (e.g. #2258 gets a push, the author pushes, #2258 is merged, 3 days; \"…, or 2 weeks\": whichever first)";
                if let Some(text) = said(edit(term, what, "", false, true), &mut self.msg) {
                    self.defer_until(key, &text);
                }
                return None;
            }
            Some(After::Not(why)) => return (self.msg = why, None).1,
            Some(After::Off) => return None,
            None => {}
        }
        if moves(pending, k.code) && self.nav.key(k, n) {
            return None;
        }
        match (pending, k.code) {
            (Some('I'), KeyCode::Char('I')) => return Some(IssuesDo::Prs),
            (Some('U'), KeyCode::Char('U')) => {
                self.msg = format!("unmarked {}", self.marked.len());
                self.marked.clear();
            }
            (Some('a'), KeyCode::Char('a')) => self.triage_marked(term),
            (Some('r'), KeyCode::Char('r')) => self.rank(true),
            (_, KeyCode::Char('s')) if self.nav.tab == ITab::Todo => self.defer_key(),
            (_, KeyCode::Char('r')) if self.nav.tab == ITab::Todo => match self.todos.rank_offer("issue(s)") {
                Ok(offer) => (self.pending, self.msg) = (Some('r'), offer),
                Err(why) => self.msg = why.into(),
            },
            (_, KeyCode::Char(c @ ('I' | 'U' | 'a'))) => {
                self.pending = Some(c);
                self.msg = match c {
                    'I' => "I again: back to the PRs".into(),
                    'U' => format!("U again: unmark all {}", self.marked.len()),
                    'a' => match self.marked.len() {
                        0 => "a again: Claude triages this issue".into(),
                        m => format!("a again: Claude triages the {m} marked"),
                    },
                    _ => String::new(),
                };
            }
            // (The same setting as dv list's: one auto for both.)
            (_, KeyCode::Char('C')) => {
                self.todos.toggle_auto();
                self.msg = match self.todos.auto {
                    true => format!(
                        "auto on: Claude triages every issue not triaged ({} now, two at a time, ~$0.30 each, at your cost) and those that change, and ranks the todo again",
                        self.untriaged()
                    ),
                    false => "auto off: a a triages, r r ranks".into(),
                };
            }
            (Some(_), KeyCode::Esc) => {} // (a pending key called off, as Helix's)
            // (No filter to clear: quits, as dv list's; Claude at work, says how (q, q again:
            // a second Esc calls things off, it doesn't quit).)
            (None, KeyCode::Esc) if !self.busy() => return Some(IssuesDo::Quit),
            (None, KeyCode::Esc) => self.msg = "Claude is still at work in dv: q, then q again, quits".into(),
            (_, KeyCode::Char('q')) => return self.quit(pending),
            (_, KeyCode::Char('R')) => {
                self.msg = "fetching the open issues…".into();
                self.refresh();
            }
            (_, KeyCode::Char('m')) => {
                if let Some(n) = this
                    && !self.marked.remove(&n)
                {
                    self.marked.insert(n);
                    self.nav.step(true, self.shown().len());
                }
            }
            // The issue, in dv. (A filter that's just a number: that issue, listed or not, as
            // dv list's.)
            (_, KeyCode::Enter) => {
                let typed = self.nav.query.trim().trim_start_matches('#').parse().ok();
                if let Some(n) = this.or(typed) {
                    self.msg = format!("#{n}…");
                    _ = term.draw(|f| self.draw(f)); // (said while GitHub's asked)
                    self.msg.clear();
                    return Some(IssuesDo::Issue(n));
                }
            }
            _ => {}
        }
        self.nav.clamp(self.shown().len());
        None
    }

    /// A click: on a tab, that tab; on a card, it selected (`true`: the second on it, to open
    /// it). Not in an issue's view, nor over the help.
    fn click(&mut self, m: event::MouseEvent) -> bool {
        use event::{MouseButton, MouseEventKind};
        if m.kind != MouseEventKind::Down(MouseButton::Left) || self.nav.help {
            return false;
        }
        let (n, rows) = (self.shown().len(), self.card_rows());
        self.nav.click(m.column, m.row, n, rows)
    }

    /// `s` on a todo item: defer it (the next key says how long), or bring a deferred one back.
    fn defer_key(&mut self) {
        let Some(n) = self.shown().get(self.nav.sel).map(|i| i.number) else { return };
        let Some(it) = self.todos.items.iter().find(|it| it.number == n).cloned() else { return };
        let (changed, said) = self.todos.defer_key(&it, &format!("#{n}"));
        self.msg = said;
        if changed {
            self.save_defer();
        }
    }

    /// Whether what an issue's deferral waits for has happened: its fix PR merged, closed,
    /// or pushed to past the commit it was at (a push: or no more to come, merged or closed).
    /// One no listed issue has as a fix any more (its issue closed with it): over.
    fn happened(&self, c: &crate::todo::Until) -> bool {
        use crate::todo::Until;
        let Some(n) = (match c {
            Until::Merged(n) | Until::Closed(n) | Until::PushedTo(n, _) => Some(*n),
            _ => None,
        }) else {
            return false;
        };
        let Some(p) = self.items.iter().flat_map(|i| &i.fixed_by).find(|p| p.0 == n) else { return self.started };
        match c {
            Until::Merged(_) | Until::Closed(_) => p.1 != "OPEN",
            Until::PushedTo(_, head) => Until::moved_past(head, &p.2) || p.1 != "OPEN",
            _ => false,
        }
    }

    /// `s` then `u`: defer the issue until what you write happens ("…, or 2 weeks": whichever
    /// first). dv reads the usual phrasings; the rest, Claude (`until_rx`).
    fn defer_until(&mut self, key: String, text: &str) {
        let n = self.todos.items.iter().find(|it| it.key() == key).map_or(0, |it| it.number);
        let fixes = self.fixes(n);
        let about = || {
            let about = match fixes.is_empty() {
                true => format!("issue #{n} (no PR fixes it yet)"),
                false => format!("issue #{n} (the PRs fixing it: {}; \"the author\" is theirs)", fixes_text(&fixes)),
            };
            format!("{about}; for an issue only a fix PR merged, closed or pushed to, or a time, apply")
        };
        match self.todos.until(key.clone(), text, about, &self.root) {
            Reading::Read(c, or) => self.defer_on(&key, c, or),
            Reading::Asked(said) => self.msg = said,
        }
    }

    /// Claude triages `ns` (`extra`: your instructions), each as listed now (`Ask`).
    /// (`extra`: your instructions, `a a`; `None`, auto's.)
    fn triage(&mut self, ns: Vec<u64>, extra: Option<String>) {
        // Queued now, not once its news comes: what looks at it next sees it under way (as dv
        // list's); who asked, kept.
        for &n in &ns {
            self.running.insert(n, batch::Status::Queued);
            self.asked.insert(n, extra.clone());
        }
        // (One Claude's at already, from an issue's view, say: its claim says so, as its run's
        // failure.)
        let asks = ns
            .into_iter()
            .map(|n| match self.items.iter().find(|i| i.number == n) {
                Some(i) => Ask { n, updated: i.updated.clone(), fixes: i.fixed_by.clone() },
                None => Ask { n, updated: String::new(), fixes: vec![] },
            })
            .collect();
        self.running_rx.push(triage_all(asks, self.root.clone(), extra.unwrap_or_default()));
    }

    /// Issue `n`'s fix PRs: (number, state, head).
    fn fixes(&self, n: u64) -> Vec<(u64, String, String)> {
        self.items.iter().find(|i| i.number == n).map(|i| i.fixed_by.clone()).unwrap_or_default()
    }

    /// Defer issue item `key` until `c` (or time `or`, first): what an issue can wait for, its
    /// fix PRs' merging or pushes ("the author pushes": its one open fix's), or a time.
    fn defer_on(&mut self, key: &str, c: crate::todo::Until, or: Option<i64>) {
        use crate::todo::Until;
        // (Seen to meanwhile, as Claude read when: said so.)
        let Some(it) = self.todos.items.iter().find(|it| it.key() == key).cloned() else { return self.msg = GONE.into() };
        let open: Vec<(u64, String, String)> = self.fixes(it.number).into_iter().filter(|p| p.1 == "OPEN").collect();
        let c = match c {
            Until::Pushed(_) if open.len() == 1 => Until::PushedTo(open[0].0, open[0].2.clone()),
            Until::Pushed(_) => return self.msg = format!("not deferred: whose push? #{} has {} open fix PRs (say which: \"#N gets a push\")", it.number, open.len()),
            // (Its own fix PRs': what dv sees of a PR here.)
            Until::PushedTo(n, _) | Until::Merged(n) | Until::Closed(n) if !self.fixes(it.number).iter().any(|p| p.0 == n) => {
                return self.msg = format!("not deferred: #{n} isn't a PR fixing #{}, which dv could watch", it.number);
            }
            Until::PushedTo(n, _) => match self.fixes(it.number).into_iter().find(|p| p.0 == n && !p.2.is_empty()) {
                Some(p) => Until::PushedTo(n, p.2),
                None => return self.msg = format!("not deferred: #{n}'s head isn't known, to see a push by"),
            },
            c @ (Until::Merged(_) | Until::Closed(_) | Until::Time(_)) => c,
            c => return self.msg = format!("not deferred: an issue can't wait until {} (a fix PR merging, closing or getting a push, or a time)", c.says()),
        };
        self.msg = self.todos.defer_on(key, c, or, 0);
        self.save_defer();
    }

    fn save_defer(&mut self) {
        if let Some(e) = self.todos.save() {
            self.msg = e;
        }
        self.find_todo();
    }

    /// `r r`: Claude ranks the todo tab (one run, in the background).
    fn rank(&mut self, asked: bool) {
        // (Said only if you asked.)
        if let Err(why) = self.todos.can_rank() {
            return if asked { self.msg = why.into() };
        }
        let items: Vec<(crate::todo::Entry, String)> = self
            .todos
            .items
            .iter()
            .filter(|it| it.deferred.is_none())
            .filter_map(|it| Some((it.entry(), about_issue(it, self.items.iter().find(|i| i.number == it.number)?, self.triage.get(&it.number)))))
            .collect();
        self.msg = match self.todos.rank(items, &self.root, crate::todo::Of::Issues) {
            Ok(()) => "Claude is ranking what needs you…".into(),
            Err(why) => why.into(),
        };
    }

    /// `a a`: Claude triages the marked issues (or this one), with your instructions if any.
    fn triage_marked(&mut self, term: &mut DefaultTerminal) {
        let ns: Vec<u64> = match self.marked.is_empty() {
            true => self.shown().get(self.nav.sel).map(|i| i.number).into_iter().collect(),
            false => self.items.iter().map(|i| i.number).filter(|n| self.marked.contains(n)).collect(),
        };
        // (Not one Claude's at already.)
        let ns: Vec<u64> = ns.into_iter().filter(|n| !self.running.get(n).is_some_and(batch::Status::running)).collect();
        if ns.is_empty() {
            return self.msg = "nothing to triage (Claude's at it already?)".into();
        }
        let Some(extra) = said(instructions(term, "triage", "instructions for Claude's triage (optional)"), &mut self.msg) else { return };
        self.msg = format!("Claude is triaging {} issue(s), {} at a time", ns.len(), batch::AT_ONCE);
        self.marked.clear();
        self.triage(ns, Some(extra));
    }

    /// A card's rows, and the blank after it: on the todo tab, one more (Claude's word, beside
    /// it or under its title).
    fn card_rows(&self) -> usize {
        if self.nav.tab == ITab::Todo { 5 } else { 4 }
    }

    fn draw(&mut self, f: &mut Frame) {
        let area = f.area();
        let ui = &self.ui;
        f.render_widget(Paragraph::new("").style(ui.base), area);
        let shown_n = self.shown().len();
        let rows = self.card_rows();
        let h = cards_fit(rows, area.height);
        self.nav.keep(shown_n, h);
        let (sel, top) = (self.nav.sel, self.nav.top);
        let mine = self.items.iter().filter(|i| i.author == self.me || i.assignees.contains(&self.me)).count();
        let close = self.items.iter().filter(|i| self.close_why(i.number).is_some()).count();
        let names = [
            (ITab::Todo, format!("todo {}", self.todos.now())),
            (ITab::Open, format!("open {}", self.items.len())),
            (ITab::Mine, format!("mine {mine}")),
            (ITab::CanClose, format!("close {close}")),
        ];
        let tabs: Vec<(String, bool)> = names.iter().map(|(t, n)| (n.clone(), self.nav.tab == *t)).collect();
        let hits = top_bar(f, ui, "Issues", &tabs, "I I the PRs", sel, shown_n);
        self.nav.tab_hits = hits.into_iter().zip(names.iter().map(|n| n.0)).collect();
        let shown = self.shown();
        // The todo tab: Claude's word beside each card when the window's wide enough, else
        // under its title (as dv list's).
        let column = (self.nav.tab == ITab::Todo).then(|| msg_column(area.width as usize)).flatten();
        let num_w = shown.iter().map(|i| format!("#{}", i.number).len()).max().unwrap_or(2);
        let width = area.width as usize;
        let lines: Vec<Line> = shown
            .iter()
            .enumerate()
            .skip(top)
            .take(h)
            .flat_map(|(k, i)| {
                let item = (self.nav.tab == ITab::Todo).then(|| self.todos.items.iter().find(|it| it.number == i.number)).flatten();
                let card_w = column.unwrap_or(width);
                let rank = item.filter(|_| column.is_some()).map(|_| k + 1); // (narrow: on the line under the title)
                let close = self.close_why(i.number).unwrap_or_default();
                let mut l = issue_card(i, ui, k == sel, self.marked.contains(&i.number), num_w, card_w, self.triage.get(&i.number), self.running.get(&i.number), self.at_work.get(&crate::claude::pr_key(i.number)).map(String::as_str), rank, &close, self.draft_fix.get(&i.number));
                if let Some(it) = item {
                    let cx = Cards { ui, num_w, width: card_w, checks_loading: false, usual: "" };
                    match column {
                        Some(w) => l = with_message(l, &todo_says(it).1, &cx, w, width, k == sel),
                        None => l.insert(1, todo_line(k + 1, it, &cx, k == sel)),
                    }
                }
                l.push(Line::raw(""));
                l
            })
            .collect();
        if shown.is_empty() {
            let none = match (self.fetched.is_some(), self.nav.tab) {
                (true, _) => " fetching…",
                (_, ITab::CanClose) => " nothing Claude would close (its triages say, a a; its ranking too, r r)",
                (_, ITab::Mine) => " no open issues assigned to you or opened by you",
                _ => " no open issues",
            };
            f.render_widget(Paragraph::new(none).style(ui.base.patch(ui.stale)), Rect { y: area.y + 2, height: 1, ..area });
        }
        f.render_widget(Paragraph::new(lines), Rect { y: area.y + 1, height: (h * rows) as u16, ..area });
        let idle = if column.is_some() { " ? keys" } else { "" };
        let left = self.nav.status(&self.msg, "label: author: kind: priority: is:triaged", idle);
        let order = match self.nav.tab {
            ITab::Todo => self.todos.order_text(),
            _ => "by updated ↓".to_owned(),
        };
        status_bar(f, ui, &left, &order, self.todos.auto, "");
        // The keys for this tab, bottom right (not over the todo tab's message column).
        if !self.nav.help && !self.nav.filtering && column.is_none() {
            let mut keys = vec![("Enter", "open it"), ("h l", "tabs"), ("/", "filter"), ("R", "refresh (from GitHub)")];
            if self.nav.tab == ITab::Todo {
                keys.extend([("r r", "Claude ranks what needs you (a run)"), ("s", "defer it (or bring it back)")]);
            }
            keys.extend([("m", "mark"), ("a a", "Claude triages (marked)"), ("U U", "unmark all"), ("C", "auto on / off (triage, ranking)"), ("I I", "the PRs"), ("?", "all keys"), ("q", "quit")]);
            // (Off the selected card: the list's cursor.)
            draw_keys(f, ui, &keys, Some(self.nav.card(rows, area)));
        }
        if self.nav.help {
            draw_help(f, ui, ISSUES_HELP, "dv issues keys");
        }
    }
}

/// An issue's card, three lines: number, title and labels, its comments, 👍 and age; who opened
/// it and who has it, and Claude's triage (what kind, how pressing); the PRs that would fix it,
/// and what Claude says is next (or that it's at it).
#[allow(clippy::too_many_arguments)]
fn issue_card(i: &Issue, ui: &Ui, selected: bool, marked: bool, num_w: usize, width: usize, t: Option<&Triage>, run: Option<&batch::Status>, at_work: Option<&str>, rank: Option<usize>, close: &str, draft: Option<&(String, Option<String>)>) -> Vec<Line<'static>> {
    let row = card_row(ui, selected);
    let tint = |st: Style| row.patch(Style { bg: None, ..st });
    let dim = tint(ui.stale);
    let sp = |s: String, st| Span::styled(s, st);
    let bar = || sp(if selected { "▌".into() } else { " ".into() }, tint(ui.key));
    let indent = || sp(" ".repeat(MARK_W + num_w + 2), row);
    let content = width.min(CARD_MAX);
    let left_w = content.saturating_sub(1 + RIGHT);
    let full = |mut spans: Vec<Span<'static>>| {
        let used: usize = spans.iter().map(Span::width).sum();
        spans.push(sp(" ".repeat(width.saturating_sub(used)), row));
        Line::from(spans).style(row)
    };
    let zones = |left: Vec<Span<'static>>, right: Vec<Span<'static>>| {
        let mut v = vec![bar()];
        v.extend(fit(left, left_w, row));
        v.extend(fit(right, RIGHT, row));
        full(v)
    };
    // What it is.
    let mut top = vec![
        sp(if marked { " ● ".into() } else { "   ".into() }, tint(ui.key).add_modifier(ratatui::style::Modifier::BOLD)),
        sp(format!("{:>w$}  ", format!("#{}", i.number), w = num_w), tint(ui.key)),
        sp(i.title.clone(), row.add_modifier(ratatui::style::Modifier::BOLD)),
    ];
    for (name, color) in &i.labels {
        top.extend([sp(" ".into(), row), sp(format!(" {name} "), crate::view::chip(color))]);
    }
    let talk = vec![sp(format!("💬 {} · 👍 {}  ", i.comments, i.thumbs), dim), sp(gh::ago(&i.updated), dim)];
    // Who; Claude's triage.
    // (Its place on the todo tab, under its number.)
    let first = match rank {
        Some(r) => sp(format!("{}{:>w$}  ", " ".repeat(MARK_W), format!("{r}."), w = num_w), tint(ui.key)),
        None => indent(),
    };
    let mut who = vec![first, sp(format!("@{}", i.author), dim)];
    if !i.assignees.is_empty() {
        who.push(sp(format!("  → {}", i.assignees.iter().map(|a| format!("@{a}")).collect::<Vec<_>>().join(" ")), dim));
    }
    let triage = match (run, t) {
        (Some(s), _) if s.running() => vec![sp("claude: triaging…".into(), tint(ui.key))],
        // (Claude at it from anywhere else: a view, another dv.)
        _ if let Some(w) = at_work => vec![sp(format!("claude: {w}…"), tint(ui.key))],
        (Some(batch::Status::Failed(e, _)), _) => vec![sp(format!("claude failed: {}", e.lines().next().unwrap_or("")), tint(ui.error))],
        _ if !close.is_empty() => vec![sp(format!("✕ close: {close}"), tint(ui.warning))],
        (_, Some(t)) => vec![sp([t.kind.as_str(), t.priority.as_str()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" · "), tint(ui.key))],
        _ => vec![],
    };
    // What would fix it; what's next.
    let mut fix = vec![indent()];
    if !i.fixed_by.is_empty() {
        fix.push(sp(format!("fixed by {}   ", fixes_text(&i.fixed_by)), tint(ui.plus)));
    }
    match (draft, t.filter(|t| !t.next.is_empty())) {
        // Claude's draft fixing it: that, not what's next (it is).
        (Some((d, Some(w))), _) => fix.push(sp(format!("Claude is {w} on its fix: {d}"), tint(ui.key))),
        (Some((d, None)), _) => fix.push(sp(format!("a draft fix: {d} (dv list's drafts tab: review it, S publishes it)"), tint(ui.plus))),
        (None, Some(t)) => fix.push(sp(format!("→ {}", t.next), tint(ui.key))),
        (None, None) => {}
    }
    let claude = match t {
        Some(_) => vec![sp("claude: triaged".into(), tint(ui.key))],
        None => vec![],
    };
    vec![zones(top, talk), zones(who, triage), zones(fix, claude)]
}

/// The drafts' records (none from tests: not the user's own).
fn records() -> Vec<crate::replace::Record> {
    if cfg!(test) { vec![] } else { crate::replace::all() }
}

/// Unpublished drafts fixing an issue: the issue → the draft's label, what Claude's doing on it
/// (its claims say). One whose run
/// failed or stopped before drafting doesn't count: nothing to wait on.
fn draft_fix(records: &[crate::replace::Record], at_work: &HashMap<String, String>) -> HashMap<u64, (String, Option<String>)> {
    records
        .iter()
        .filter(|r| r.published.is_none())
        .filter_map(|r| {
            let n = crate::replace::fixes(&r.task)?;
            let at_it = at_work.get(&crate::claude::rep_key(r.id)).cloned();
            (at_it.is_some() || r.drafted()).then(|| (n, (crate::replace::label(r.id), at_it)))
        })
        .collect()
}

pub(super) const ISSUES_HELP: &str = "
MOVE
  j k, Up Down         move
  gg ge                first / last issue
  Ctrl-d Ctrl-u        half a page down / up
  l h, gn gp           next / previous tab (todo, open, mine, can close)
  Enter                open it (its text, comments, Claude's triage; what to do about it)
  I I                  back to the PRs
  ?                    this help
  q, Ctrl-c            quit

FIND
  /                    filter: all terms match (Enter keeps it, Esc clears it)
                       text: number, title, author, labels
                       label: author: kind: priority:  is:triaged  (-term excludes)
  Esc                  clear the filter (none: quit)
  R                    refresh from GitHub (no Claude)

TODO
  r r                  Claude ranks what needs you (one run; the bar says its cost)
  s                    defer it: 1 an hour, 4 four hours, d a day, w a week, u until… (a
                       fix PR's push or merge, …, or 2 weeks; again: back)

CLAUDE
  m                    mark the issue (again: unmark)
  U U                  unmark all
  a a                  Claude triages the marked issues (or this one), two at a time
  C                    auto: Claude triages what isn't triaged (and changed), ranks again (git config dv.auto)";

#[test]
fn issues_listed_triaged_filtered() {
    use crate::view::tests::{screen, snapshot};
    let v: Value = serde_json::from_str(r#"{"number": 7, "title": "Panics on reorg",
        "updatedAt": "2026-01-01T00:00:00Z", "author": {"login": "amy"}, "labels": {"nodes": [{"name": "bug", "color": "d73a4a"}]},
        "comments": {"totalCount": 3}, "reactions": {"totalCount": 2}, "assignees": {"nodes": [{"login": "me"}]},
        "closedByPullRequestsReferences": {"nodes": [{"number": 9, "state": "OPEN", "headRefOid": "h9"}]}}"#).unwrap();
    let a = issue(&v);
    assert_eq!((a.number, a.author.as_str(), a.comments, a.thumbs, a.fixed_by.clone()), (7, "amy", 3, 2, vec![(9, "OPEN".to_owned(), "h9".to_owned())]));
    let dir = std::env::temp_dir().join(format!("dv-issues-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("issue-8.json"), r#"{"kind": "question", "priority": "low", "next": "answer it", "close": "answered in #3"}"#).unwrap();
    let mut l = Issues { me: "me".into(), reviews_dir: Some(dir.clone()), ..Default::default() };
    l.items = vec![a, Issue { number: 8, title: "How do I sync?".into(), author: "bo".into(), updated: "2026-01-02T00:00:00Z".into(), ..Default::default() }];
    l.watch_reviews();
    l.nav.tab = ITab::Open;
    assert_eq!(l.triage.get(&8).map(|t| t.close.as_str()), Some("answered in #3"));
    let shown = |l: &Issues| l.shown().iter().map(|i| i.number).collect::<Vec<_>>();
    assert_eq!(shown(&l), [7, 8]);
    l.nav.tab = ITab::Mine;
    assert_eq!(shown(&l), [7]); // assigned to you
    l.nav.tab = ITab::CanClose;
    assert_eq!(shown(&l), [8]);
    // The PR linked as fixing #7 merged: not closable for that alone (it may not fix it all);
    // to check, and triaged again once triaged before it merged, not once Claude's seen it.
    l.items[0].fixed_by[0].1 = "MERGED".into();
    assert_eq!(shown(&l), [8]);
    let t = |seen: Vec<u64>| Triage { updated: "2026-01-01T00:00:00Z".into(), fixes_seen: seen, ..Triage::default() };
    let needs = |t: Triage| issue_needs(&l.items[0], Some(&t), None, "");
    assert!(needs(t(vec![])).contains(&crate::todo::Need::FixMerged(9)));
    assert!(!needs(t(vec![9])).iter().any(|n| matches!(n, crate::todo::Need::FixMerged(_))));
    l.items[0].fixed_by[0].1 = "OPEN".into();
    l.nav.tab = ITab::Open;
    l.nav.query = "label:bug".into();
    assert_eq!(shown(&l), [7]);
    l.nav.query = "-is:triaged".into();
    assert_eq!(shown(&l), [7]);
    l.nav.query = "kind:question".into();
    assert_eq!(shown(&l), [8]);
    l.nav.query.clear();
    l.items.iter_mut().for_each(|i| i.updated.clear()); // (no age: the snapshot stays as it is)
    snapshot("issues", &screen(140, 12, |f| l.draw(f)));
    // The todo tab: #7 assigned to you, a PR would fix it; #8 Claude would close.
    l.find_todo();
    assert_eq!(l.todos.items.iter().map(|it| (it.number, it.needs.clone())).collect::<Vec<_>>(), [
        (8, vec![crate::todo::Need::WouldClose("answered in #3".into())]),
        (7, vec![crate::todo::Need::Assigned, crate::todo::Need::FixOpen(9), crate::todo::Need::Untriaged]),
    ]);
    l.nav.tab = ITab::Todo;
    snapshot("issues_todo", &screen(140, 14, |f| l.draw(f)));
    snapshot("issues_todo_wide", &screen(200, 14, |f| l.draw(f)));
    // The mouse: a tab clicked; a card, twice opens it.
    let click = |l: &mut Issues, column, row| {
        l.click(event::MouseEvent { kind: event::MouseEventKind::Down(event::MouseButton::Left), column, row, modifiers: KeyModifiers::NONE })
    };
    let (open_tab, _) = l.nav.tab_hits.iter().find(|h| h.1 == ITab::Open).cloned().unwrap();
    assert!(!click(&mut l, open_tab.start, 0) && l.nav.tab == ITab::Open);
    assert!(!click(&mut l, 10, 5) && l.nav.sel == 1); // the second card (four rows each, after the bar)
    assert!(click(&mut l, 10, 5)); // again: open it
    l.nav.tab = ITab::Todo;
    // Auto: what isn't triaged (#7), once at an update; again once it's changed.
    assert_eq!(l.auto_picks(), [7]);
    assert!(l.auto_picks().is_empty());
    l.items[0].updated = "2026-02-01T00:00:00Z".into();
    l.find_todo();
    assert_eq!(l.auto_picks(), [7]);
    // Until its fix PR gets a push, or 2 weeks: from #9's head now; back once it moves on.
    l.todos.deferred_file = Some(dir.join("deferred-issues.json"));
    let key = l.todos.items.iter().find(|it| it.number == 7).unwrap().key();
    l.defer_until(key.clone(), "the author makes progress, or 2 weeks");
    assert!(l.msg.starts_with("deferred until #9 gets a push (past h9), or in 1"), "{}", l.msg);
    assert!(l.todos.items.iter().find(|it| it.number == 7).unwrap().deferred.as_ref().is_some_and(|d| d.until < i64::MAX));
    l.items[0].fixed_by[0].2 = "h10".into();
    l.find_todo();
    assert!(l.todos.items.iter().find(|it| it.number == 7).unwrap().deferred.is_none() && l.msg.starts_with("#7 is back"), "{}", l.msg);
    // A PR that doesn't fix it: dv wouldn't see it.
    l.defer_until(key.clone(), "#99 is merged");
    assert!(l.msg.starts_with("not deferred: #99 isn't a PR fixing #7"), "{}", l.msg);
    // What an issue can't wait for: said so.
    l.defer_until(key, "@bo approves");
    assert!(l.msg.starts_with("not deferred: an issue can't wait until @bo approves it"), "{}", l.msg);
    _ = std::fs::remove_dir_all(&dir);
}

/// Live: `DV_LIVE_REPO=<a checkout> DV_LIVE_ISSUE=N cargo test live_triage -- --ignored --nocapture`
/// (with `XDG_DATA_HOME` somewhere to try it).
#[test]
#[ignore]
fn live_triage() {
    let repo = std::path::PathBuf::from(std::env::var("DV_LIVE_REPO").expect("DV_LIVE_REPO"));
    std::env::set_current_dir(&repo).unwrap();
    let n: u64 = std::env::var("DV_LIVE_ISSUE").expect("DV_LIVE_ISSUE").parse().unwrap();
    println!("{:?}", triage_run(n, "", &[], &repo, ""));
    let dir = crate::claude::dv_dir().unwrap().join("reviews");
    println!("{:?}", triage_of(&dir, n));
    assert!(triage_of(&dir, n).is_some());
}


#[test]
fn a_draft_holds_its_issue_while_it_can_come() {
    use crate::replace::Record;
    let rec = |id: u64, task: &str, title: &str| Record { id, task: task.into(), title: title.into(), ..Default::default() };
    let records = [rec(1, "Fix #7: it panics", "Fix the panic"), rec(2, "Fix #8: slow", ""), rec(3, "Fix #9: odd", ""), rec(4, "Refactor", "x")];
    // #7 drafted; #8 being drafted (its record doesn't say yet); #9's run failed before drafting.
    let fix = draft_fix(&records, &HashMap::from([(crate::claude::rep_key(2), "drafting".to_owned())]));
    assert_eq!(fix.get(&7).map(|f| f.1.as_deref()), Some(None));
    assert_eq!(fix.get(&8).map(|f| f.1.as_deref()), Some(Some("drafting")));
    assert!(!fix.contains_key(&9) && fix.len() == 2);
}

#[test]
fn auto_triages_each_issue_once_at_a_time() {
    use crate::todo::{Item, Kind, Need};
    let it = |n, needs: Vec<Need>| Item { kind: Kind::Issue, number: n, needs, claude: None, updated: String::new(), reason: None, changed: false, deferred: None, busy: None, waits: None };
    let mut l = Issues { items: (1..=5).map(|n| Issue { number: n, updated: "t1".into(), ..Default::default() }).collect(), ..Default::default() };
    let mut put_aside = it(1, vec![Need::Untriaged]);
    put_aside.deferred = Some(crate::todo::Deferral { until: i64::MAX, needs: String::new(), cond: None, at: 0 });
    l.todos.items = vec![put_aside, it(2, vec![Need::Untriaged]), it(3, vec![]), it(4, vec![Need::Retriage]), it(5, vec![Need::Untriaged])];
    // Not what's deferred, nor what needs no triage; at most AT_ONCE (the rest next time).
    assert_eq!(batch::AT_ONCE, 2);
    assert_eq!(l.auto_picks(), [2, 4]);
    assert_eq!(l.auto_picks(), [5]);
    // Each once at its updatedAt: again only once it changes.
    assert!(l.auto_picks().is_empty());
    l.items[1].updated = "t2".into();
    assert_eq!(l.auto_picks(), [2]);
    // Or once a PR linked as fixing it merges (the issue's time stays as it was).
    l.items[1].fixed_by = vec![(9, "MERGED".into(), String::new())];
    l.todos.items[1].needs = vec![Need::FixMerged(9)];
    assert_eq!(l.auto_picks(), [2]);
}
