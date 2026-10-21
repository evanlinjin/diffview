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

/// Claude triages issue `n` (`extra`: your instructions), in the main checkout, reading only
/// (it can't edit: its answer is the triage, which dv writes to `issue-N.json` beside the PRs'
/// reviews).
fn triage_run(n: u64, updated: &str, fixes: &[(u64, String, String)], root: &std::path::Path, extra: &str) -> Result<batch::Status> {
    let file = crate::claude::dv_dir()?.join("reviews").join(format!("issue-{n}.json"));
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
    let out = crate::claude::claude_reader(crate::claude::Work::Triage, &prompt, root, &tools).stdin(std::process::Stdio::null()).output().context("is Claude Code (`claude`) installed?")?;
    let v: Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
    let answer = v["result"].as_str().unwrap_or("");
    // (Its own word for what went wrong, where it said one.)
    let why = if answer.is_empty() { String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("").to_owned() } else { answer.lines().next().unwrap_or("").to_owned() };
    anyhow::ensure!(out.status.success() && v["is_error"] == false, "Claude stopped ({}): {why}", out.status);
    let mut t = crate::claude::json_in(answer, '{').filter(Value::is_object).context("no triage in Claude's answer")?;
    (t["issue"], t["updated"]) = (n.into(), updated.into());
    // (The merged fixes it judged: one merging after has it triaged again.)
    t["fixes_seen"] = fixes.iter().filter(|p| p.1 == "MERGED").map(|p| p.0).collect::<Vec<_>>().into();
    // A closing comment drafted for it (`X X`), kept when this triage has none of its own.
    // (Only while this triage still says to close it: one that says keep it open, keeps it.)
    let old: Value = std::fs::read_to_string(&file).ok().and_then(|o| serde_json::from_str(&o).ok()).unwrap_or_default();
    let said = |v: &Value, k: &str| v[k].as_str().is_some_and(|c| !c.trim().is_empty());
    if said(&t, "close") && !said(&t, "closing") && said(&old, "closing") {
        for k in ["closing", "reason", "duplicate_of"] {
            t[k] = old[k].clone();
        }
    }
    std::fs::create_dir_all(file.parent().context("no dir")?)?;
    crate::claude::write(&file, &t)?;
    Ok(batch::Status::Done(format!("{} turns, ${:.2}", v["num_turns"].as_u64().unwrap_or(0), v["total_cost_usd"].as_f64().unwrap_or(0.0))))
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
        (a.n, triage_run(a.n, &a.updated, &a.fixes, &root, &extra).unwrap_or_else(|e| batch::Status::Failed(format!("{e:#}"))))
    })
}

/// The PRs linked as fixing an issue, said: "#9 (merged), #12 (open)".
fn fixes_text(fixes: &[(u64, String, String)]) -> String {
    fixes.iter().map(|(n, s, _)| format!("#{n} ({})", s.to_lowercase())).collect::<Vec<_>>().join(", ")
}

/// Claude drafts a comment closing issue `n` (reading only), in the background: kept in its
/// triage (`close`, `closing`, `reason`, `duplicate_of`), for `X X` to close with.
fn draft_closing(n: u64, root: PathBuf) -> Receiver<Result<()>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let res = crate::batch::guarded(|| -> Result<()> {
            let file = crate::claude::dv_dir()?.join("reviews").join(format!("issue-{n}.json"));
            let prompt = format!(
                "{}The maintainer is closing issue #{n}. Read it (`gh issue view {n} --comments`, its triage in {} if \
                 there is one, related issues and PRs) and draft the comment it closes with: short, kind, saying why \
                 (fixed by which PR, a duplicate of which issue, answered, out of scope, stale), thanking the author \
                 where it fits. Reply with only a JSON object: {{\"close\": \"why, a few words\", \"closing\": \
                 \"the comment, markdown\", \"reason\": \"completed\" | \"not planned\" | \"duplicate\", \
                 \"duplicate_of\": N or null}}.",
                crate::claude::notes_preamble(),
                file.display()
            );
            let tools = ["Bash(gh issue view:*)", "Bash(gh issue list:*)", "Bash(gh search:*)"];
            let out = crate::claude::claude_reader(crate::claude::Work::Close, &prompt, &root, &tools).stdin(std::process::Stdio::null()).output()?;
            let v: Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
            anyhow::ensure!(v["is_error"] == false, "Claude couldn't draft it ({})", out.status);
            let d = crate::claude::json_in(v["result"].as_str().unwrap_or(""), '{').filter(Value::is_object).context("no draft in Claude's answer")?;
            let said = |k: &str| d[k].as_str().is_some_and(|s| !s.trim().is_empty());
            anyhow::ensure!(said("close") && said("closing"), "Claude's draft said no why or no comment");
            let mut t: Value = std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_else(|| serde_json::json!({ "issue": n }));
            for k in ["close", "closing", "reason", "duplicate_of"] {
                t[k] = d[k].clone();
            }
            std::fs::create_dir_all(file.parent().context("no dir")?)?;
            crate::claude::write(&file, &t)
        });
        _ = tx.send(res);
    });
    rx
}

/// What the issues list asks of its caller.
pub enum IssuesDo {
    Quit,
    /// Back to the PRs (`I I`).
    Prs,
    /// Review PR N (Enter on a link to it), then back here.
    Pr(u64),
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
    /// `Space l` in an issue: its labels and assignees to toggle (what, name, on it), and the
    /// one selected.
    meta: Option<(Vec<Meta>, usize, bool)>,
    /// Issues a draft of Claude's (unpublished: `N N`'s) is to fix: its label, whether Claude's
    /// at it now. Not on the todo till it's done: nothing for you there meanwhile.
    draft_fix: HashMap<u64, (String, bool)>,
    running: HashMap<u64, batch::Status>,
    running_rx: Vec<Receiver<(u64, batch::Status)>>,
    reviews_dir: Option<PathBuf>,
    reviews_mtime: Option<std::time::SystemTime>,
    fetched: Option<Receiver<Fetch>>,
    /// The issue open in dv (Enter), and Claude's drafts of PRs fixing issues, under way.
    view: Option<super::issue_view::IssueView>,
    drafts: Vec<Receiver<(u64, batch::Status)>>,
    /// Those drafts not done yet (by id).
    drafting: HashSet<u64>,
    /// A closing comment Claude's drafting (for which issue), on its way.
    closing_rx: Option<(u64, Receiver<Result<()>>)>,
    /// Auto (`C`, git config `dv.auto`, as dv list's): Claude triages what isn't triaged, and
    /// what's changed since, and ranks the todo again. Whether it's started (the first fetch
    /// in), what it's done (once each), and when it last ranked.
    started: bool,
    auto_done: HashSet<(u64, String)>,
    root: PathBuf,
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
        self.draft_fix = draft_fix(&records(), &self.drafting);
        let items: Vec<Item> = items.into_iter().filter(|it| !self.draft_fix.contains_key(&it.number)).collect();

        // (Lent out: whether what a deferral waits for has happened is read from the rest.)
        let mut todos = std::mem::take(&mut self.todos);
        let (items, said) = todos.settle(items, |_, c, _| self.happened(c));
        self.todos = todos;
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
            self.triage(run, String::new());
        }
        // An issue Claude would close with no closing comment drafted yet: drafted, one at a time,
        // once each (X X then closes with it, your editor first).
        if self.closing_rx.is_none() && !started {
            let dir = self.reviews_dir.clone();
            let drafted = |n: u64| dir.as_ref().and_then(|d| std::fs::read_to_string(d.join(format!("issue-{n}.json"))).ok()).is_some_and(|t| serde_json::from_str::<Value>(&t).is_ok_and(|v| v["closing"].as_str().is_some_and(|c| !c.trim().is_empty())));
            // (Not what you've put aside.)
            let deferred: HashSet<u64> = self.todos.items.iter().filter(|it| it.deferred.is_some()).map(|it| it.number).collect();
            let next = self.items.iter().map(|i| (i.number, i.updated.clone())).find(|(n, u)| !deferred.contains(n) && self.close_why(*n).is_some() && !drafted(*n) && !self.auto_done.contains(&(*n, format!("closing@{u}"))));
            if let Some((n, u)) = next {
                self.auto_done.insert((n, format!("closing@{u}")));
                self.closing_rx = Some((n, draft_closing(n, self.root.clone())));
                self.say(format!("auto: Claude is drafting a comment closing #{n}"));
            }
        }
        if !busy && !started && self.todos.auto_rank_due() {
            self.rank();
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
            if due && run.len() < batch::AT_ONCE && self.auto_done.insert((i.number, format!("{}{merged:?}", i.updated))) {
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

    /// Claude at work from here: triaging, drafting a fix or a closing comment, ranking.
    pub fn busy(&self) -> bool {
        self.running.values().any(batch::Status::running) || !self.drafting.is_empty() || self.closing_rx.is_some() || self.todos.rank_rx.is_some()
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
            self.auto_run();
            // Claude's reading of a deferral's "until …".
            if let Some((key, res, or)) = self.todos.until_news() {
                match res {
                    Ok(c) => self.defer_on(&key, c, or),
                    Err(e) => self.msg = format!("not deferred: {e:#}"),
                }
            }
            // A deferral whose time's come: back.
            let now = crate::claude::now_secs();
            if self.todos.items.iter().any(|it| it.deferred.as_ref().is_some_and(|d| d.until <= now)) {
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
            for (id, st) in self.drafts.iter().flat_map(|rx| rx.try_iter()).collect::<Vec<_>>() {
                if !st.running() {
                    self.drafting.remove(&id);
                }
                // (Under way, or done: its record says so now.)
                if !matches!(st, batch::Status::Queued | batch::Status::Preparing) {
                    self.drafts_changed();
                }
                match st {
                    batch::Status::Done(took) => self.msg = format!("Claude drafted {} ({took}): dv list's drafts tab has it (I I there)", crate::replace::label(id)),
                    batch::Status::Failed(e) => self.msg = format!("Claude's draft of {} failed: {}", crate::replace::label(id), e.lines().next().unwrap_or("")),
                    _ => {}
                }
            }
            // (Its thread gone without a word: as a failure, not drafting forever.)
            let news = self.closing_rx.as_ref().map(|(_, rx)| rx.try_recv());
            if let Some(Ok(_) | Err(mpsc::TryRecvError::Disconnected)) = &news {
                let n = self.closing_rx.take().map_or(0, |c| c.0);
                let res = match news {
                    Some(Ok(r)) => r,
                    _ => Err(anyhow::anyhow!("its run stopped")),
                };
                match res {
                    Ok(()) => {
                        self.msg = format!("Claude drafted a comment closing #{n}: X X closes with it (your editor first)");
                        self.reviews_mtime = None;
                        let dir = self.reviews_dir.clone();
                        if let Some(v) = self.view.as_mut().filter(|v| v.number == n) {
                            v.reload_triage(dir.as_deref());
                        }
                    }
                    Err(e) => self.msg = format!("no closing comment: {e:#}"),
                }
            }
            for (n, st) in self.running_rx.iter().flat_map(|rx| rx.try_iter()).collect::<Vec<_>>() {
                {
                    if matches!(st, batch::Status::Done(_)) && self.view.as_ref().is_some_and(|v| v.number == n) {
                        let dir = self.reviews_dir.clone();
                        if let Some(v) = self.view.as_mut() {
                            v.reload_triage(dir.as_deref());
                        }
                    }
                    match &st {
                        batch::Status::Failed(e) => self.msg = format!("#{n}: {}", e.lines().next().unwrap_or("")),
                        batch::Status::Done(took) => {
                            self.say(format!("triaged #{n} ({took})"));
                            self.reviews_mtime = None; // (read again: written over, the dir may not say)
                        }
                        _ => {}
                    }
                    self.running.insert(n, st);
                }
            }
            if !event::poll(Duration::from_millis(100))? {
                continue;
            }
            let k = match event::read()? {
                Event::Key(k) if k.kind == KeyEventKind::Press => k,
                // The wheel moves the cursor (scrolls the issue), as j / k; a click picks a tab or a
                // card, twice opens it (as dv list's).
                // In an issue, a drag copies what it's over, and sideways scrolls its code blocks.
                Event::Mouse(m) if self.view.is_some() && !self.nav.help && self.meta.is_none() && {
                    use event::MouseEventKind::*;
                    matches!(crate::view::sideways(&m), Down(_) | Drag(_) | Up(_) | ScrollLeft | ScrollRight)
                } =>
                {
                    let v = self.view.as_mut().expect("an issue open");
                    let kind = crate::view::sideways(&m);
                    // (Row 0's the title bar: the view's from row 1.)
                    match v.mouse(kind, m.column as usize, m.row as isize - 1) {
                        Some(t) if t.trim().is_empty() => self.msg = "nothing to copy".into(),
                        Some(t) => {
                            crate::view::clipboard(&t);
                            self.msg = format!("copied {} line{}", t.lines().count(), if t.lines().count() == 1 { "" } else { "s" });
                        }
                        None => {}
                    }
                    continue;
                }
                // In an issue the wheel scrolls it, the cursor where it is.
                // (The picker open: the wheel moves it.)
                Event::Mouse(m) if self.meta.is_some() => match m.kind {
                    event::MouseEventKind::ScrollDown => KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
                    event::MouseEventKind::ScrollUp => KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE),
                    _ => continue,
                },
                Event::Mouse(m) if matches!(m.kind, event::MouseEventKind::ScrollDown | event::MouseEventKind::ScrollUp) && !self.nav.help => {
                    if let Some(v) = self.view.as_mut() {
                        v.scroll(m.column as usize, if m.kind == event::MouseEventKind::ScrollDown { 3 } else { -3 });
                        continue;
                    }
                    KeyEvent::new(KeyCode::Char(if m.kind == event::MouseEventKind::ScrollDown { 'j' } else { 'k' }), KeyModifiers::NONE)
                }
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

    fn key(&mut self, term: &mut DefaultTerminal, k: KeyEvent) -> Option<IssuesDo> {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && k.code == KeyCode::Char('c') {
            return Some(IssuesDo::Quit);
        }
        if std::mem::take(&mut self.nav.help) {
            return None; // any key closes it
        }
        if self.meta.is_some() {
            self.meta_key(k);
            return None;
        }
        if self.view.is_some() {
            return self.view_key(term, k);
        }
        if self.nav.filter_key(k) {
            return None;
        }
        let n = self.shown().len();
        let pending = self.pending.take();
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
        if moves(pending, k.code) && self.nav.key(k, n, self.height()) {
            return None;
        }
        match (pending, k.code) {
            (Some('I'), KeyCode::Char('I')) => return Some(IssuesDo::Prs),
            (Some('U'), KeyCode::Char('U')) => {
                self.msg = format!("unmarked {}", self.marked.len());
                self.marked.clear();
            }
            (Some('a'), KeyCode::Char('a')) => self.triage_marked(term),
            (Some('r'), KeyCode::Char('r')) => self.rank(),
            (_, KeyCode::Char('s')) if self.nav.tab == ITab::Todo => self.defer_key(),
            (_, KeyCode::Char('r')) if self.nav.tab == ITab::Todo => {
                self.pending = Some('r');
                self.msg = self.todos.rank_offer("issue(s)");
            }
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
            (Some('q'), KeyCode::Char('q')) => return Some(IssuesDo::Quit),
            (_, KeyCode::Char('q')) if self.busy() => {
                self.pending = Some('q');
                self.msg = "Claude is still at work (triaging, drafting, ranking): q again quits (what's under way is lost)".into();
            }
            (_, KeyCode::Char('q')) => return Some(IssuesDo::Quit),
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
            // The issue, in dv.
            (_, KeyCode::Enter) => {
                if let Some(n) = this {
                    self.msg = format!("#{n}…");
                    _ = term.draw(|f| self.draw(f)); // (said while GitHub's asked)
                    self.msg.clear();
                    match self.load_issue(n) {
                        Ok(v) => self.view = Some(v),
                        Err(e) => self.msg = format!("#{n}: {e:#}"),
                    }
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
        if m.kind != MouseEventKind::Down(MouseButton::Left) || self.nav.help || self.view.is_some() {
            return false;
        }
        let (n, rows, h) = (self.shown().len(), self.card_rows(), self.height());
        self.nav.click(m.column, m.row, n, rows, h)
    }

    /// A key in the issue's view.
    fn view_key(&mut self, term: &mut DefaultTerminal, k: KeyEvent) -> Option<IssuesDo> {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let pending = self.pending.take();
        // (Scrolling, the wheel too, leaves what was said.)
        if !matches!(k.code, KeyCode::Char('j' | 'k') | KeyCode::Up | KeyCode::Down) && !ctrl {
            self.msg.clear();
        }
        let v = self.view.as_mut()?;
        let n = v.number;
        // The cursor's keys (Helix's), but not the second of another pair (`L L`).
        // (A pair's second key isn't the cursor's: `L j` moves it, `L` called off.)
        {
            // (`Space l`'s l isn't the cursor's either.)
            let key = if pending == Some(' ') { super::issue_view::Did::Not } else { v.key(pending.filter(|&p| p == 'g'), k.code, ctrl) };
            match key {
                super::issue_view::Did::Not => {}
                super::issue_view::Did::Nothing => return None,
                super::issue_view::Did::Copy(t) => {
                    crate::view::clipboard(&t);
                    self.msg = format!("copied {} line{}", t.lines().count(), if t.lines().count() == 1 { "" } else { "s" });
                    return None;
                }
                super::issue_view::Did::Open(u) => {
                    // This repo's #N: a PR's view, an issue's, in dv; anything else, the browser.
                    let Some((n, pr)) = v.number_in(&u) else { return (self.msg = crate::view::open_url(&u), None).1 };
                    // (GitHub asked which it is, unless the link says: said meanwhile.)
                    self.msg = format!("opening #{n}…");
                    _ = term.draw(|f| self.draw(f));
                    self.msg.clear();
                    if pr || crate::gh::gh(&["pr", "view", &n.to_string(), "--json", "number"]).is_ok() {
                        return Some(IssuesDo::Pr(n));
                    }
                    match self.load_issue(n) {
                        Ok(v) => self.view = Some(v),
                        Err(e) => self.msg = format!("#{n}: {e:#}"),
                    }
                    return None;
                }
            }
        }
        match (pending, k.code) {
            (Some(_), KeyCode::Esc) => {} // (a pending key called off, as Helix's: L, g, Space…)
            // Esc drops a selection first, as Helix's.
            (_, KeyCode::Esc) if v.selecting() => v.drop_selection(),
            (_, KeyCode::Char('q') | KeyCode::Esc) => self.view = None,
            (_, KeyCode::Char('?')) => self.nav.help = true,
            (_, KeyCode::Char('o')) => self.msg = crate::view::open_url(v.url()),
            (_, KeyCode::Char('C')) => {
                let body = said(edit(term, &format!("a comment on issue #{n} (posted when you save)"), "", false, true), &mut self.msg)?;
                self.msg = match crate::gh::gh(&["issue", "comment", &n.to_string(), "--body", &body]) {
                    Ok(_) => format!("commented on #{n}"),
                    Err(e) => format!("not posted: {e:#}"),
                };
                self.reload_view(n);
            }
            // Labelling, and retitling or closing someone else's issue, take triage access.
            (Some(c @ ('L' | 'T' | 'X')), KeyCode::Char(k)) if k == c && !self.can_triage && (c == 'L' || v.author != self.me) => {
                let what = match c {
                    'L' => "labelling issues",
                    'T' => "retitling others' issues",
                    _ => "closing others' issues",
                };
                self.msg = format!("{what} here takes triage access, which you don't have");
            }
            (Some('L'), KeyCode::Char('L')) => {
                let labels = v.new_labels();
                // (A flag each: a label may have a comma in it.)
                let mut args = vec!["issue".to_owned(), "edit".into(), n.to_string()];
                labels.iter().for_each(|l| args.extend(["--add-label".into(), l.clone()]));
                self.msg = match crate::gh::gh(&args.iter().map(String::as_str).collect::<Vec<_>>()) {
                    Ok(_) => format!("#{n}: labelled {}", labels.join(", ")),
                    Err(e) => format!("not labelled: {e:#}"),
                };
                self.reload_view(n);
            }
            (Some('T'), KeyCode::Char('T')) => {
                let title = v.new_title()?;
                self.msg = match crate::gh::gh(&["issue", "edit", &n.to_string(), "--title", &title]) {
                    Ok(_) => format!("#{n} is now \"{title}\""),
                    Err(e) => format!("not retitled: {e:#}"),
                };
                self.reload_view(n);
            }
            // No closing comment of Claude's yet: it drafts one first (X X then closes with it).
            (Some('X'), KeyCode::Char('X')) if v.closing().is_none_or(|c| c.is_empty()) => {
                if self.closing_rx.is_some() {
                    self.msg = "Claude's drafting a closing comment already".into();
                } else {
                    self.closing_rx = Some((n, draft_closing(n, self.root.clone())));
                    self.msg = format!("Claude is drafting a comment closing #{n} (X X then closes with it)…");
                }
            }
            (Some('X'), KeyCode::Char('X')) => {
                let closing = v.closing().unwrap_or_default();
                // As its triage says: completed, not planned, or a duplicate (of what).
                let (reason, dup) = v.close_reason();
                let body = said(edit(term, &format!("closing issue #{n} ({reason}): the comment it closes with (empty: none)"), &closing, true, true), &mut self.msg)?;
                let mut args = vec!["issue".to_owned(), "close".into(), n.to_string()];
                match dup {
                    Some(d) => args.extend(["--duplicate-of".into(), d.to_string()]),
                    None => args.extend(["--reason".into(), reason.clone()]),
                }
                if !body.trim().is_empty() {
                    args.extend(["--comment".into(), body]);
                }
                self.msg = match crate::gh::gh(&args.iter().map(String::as_str).collect::<Vec<_>>()) {
                    Ok(_) => {
                        self.view = None;
                        self.refresh();
                        format!("closed #{n} ({reason})")
                    }
                    Err(e) => format!("not closed: {e:#}"),
                };
            }
            (Some('N'), KeyCode::Char('N')) => {
                let task = v.fix_task();
                let what = format!("a PR fixing #{n}, for Claude to draft (what it should do; unpublished until you publish it)");
                let task = said(edit(term, &what, &task, false, true), &mut self.msg)?;
                match start_new_pr(&task, &self.root) {
                    Ok((id, rx)) => {
                        self.drafts.push(rx);
                        self.drafting.insert(id);
                        self.msg = format!("Claude is drafting {} (fixes #{n}): dv list's drafts tab shows it", crate::replace::label(id));
                        self.drafts_changed();
                    }
                    Err(e) => self.msg = format!("couldn't start it: {e:#}"),
                }
            }
            (Some('a'), KeyCode::Char('a')) => {
                if self.running.get(&n).is_some_and(batch::Status::running) {
                    self.msg = "Claude's triaging it already".into();
                } else {
                    self.triage(vec![n], String::new());
                    self.msg = format!("Claude is triaging #{n}…");
                }
            }
            (Some(' '), KeyCode::Char('l')) => {
                // (Three asks of GitHub: said meanwhile.)
                self.msg = format!("asking GitHub for #{n}'s labels and assignees…");
                _ = term.draw(|f| self.draw(f));
                self.msg.clear();
                self.open_meta(n);
            }
            // A PR from what's selected (none: the cursor's line), of this issue's but not fixing
            // it whole (N N does that).
            (Some(' '), KeyCode::Char('N')) => {
                let draft = format!("From #{n} ({}):\n\n> {}\n\nWhat a PR should do about it: ", v.title(), v.picked_text().trim().replace('\n', "\n> "));
                let what = format!("a new PR from #{n}, for Claude to draft (what it should do; unpublished until you publish it)");
                // (Saved as it was: nothing said of what it should do. Not asked.)
                let task = said(edit(term, &what, &draft, false, false), &mut self.msg)?;
                match start_new_pr(&task, &self.root) {
                    Ok((id, rx)) => {
                        self.drafts.push(rx);
                        self.drafting.insert(id);
                        self.msg = format!("Claude is drafting {} (from #{n}): dv list's drafts tab shows it", crate::replace::label(id));
                    }
                    Err(e) => self.msg = format!("couldn't start it: {e:#}"),
                }
            }
            (_, KeyCode::Char(' ')) => (self.pending, self.msg) = (Some(' '), "Space: l labels, assignees · N a PR from the selection".into()),
            (Some(' '), _) => self.msg.clear(), // (Space, then not l: called off)
            (_, KeyCode::Char(c @ ('g' | 'L' | 'T' | 'X' | 'N' | 'a'))) => {
                self.pending = Some(c);
                self.msg = match c {
                    'L' => match v.new_labels() {
                        l if l.is_empty() => (self.pending = None, "Claude's triage has no labels to add".to_owned()).1,
                        l => format!("L again: add {} to #{n}", l.join(", ")),
                    },
                    'T' => match v.new_title() {
                        None => (self.pending = None, "Claude's triage has no new title".to_owned()).1,
                        Some(t) => format!("T again: retitle #{n} \"{t}\""),
                    },
                    'X' => match v.closing().is_some_and(|c| !c.is_empty()) {
                        true => format!("X again: close #{n} (your editor first, with Claude's closing comment)"),
                        false => format!("X again: Claude drafts a comment closing #{n} (then X X closes with it)"),
                    },
                    'N' => format!("N again: Claude drafts a PR fixing #{n} (your editor first)"),
                    'a' => format!("a again: Claude triages #{n} again"),
                    _ => String::new(),
                };
            }
            _ => {}
        }
        None
    }

    /// The issue open, from GitHub again (after a comment, a label, a title).
    fn reload_view(&mut self, n: u64) {
        let keep = self.view.as_ref().map_or([0; 2], |v| v.scroll_at());
        match self.load_issue(n) {
            Ok(mut v) => {
                v.set_scroll(keep);
                self.view = Some(v);
            }
            Err(e) => self.msg += &format!(" (reading it again: {e:#})"),
        }
        self.refresh();
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
            Until::Merged(_) => p.1 == "MERGED",
            Until::Closed(_) => p.1 != "OPEN",
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
            Reading::Asked => self.msg = "Claude is reading when you mean…".into(),
            Reading::Busy => self.msg = "not deferred: Claude is still reading the last \"until\" (try again in a moment)".into(),
        }
    }

    /// Claude triages `ns` (`extra`: your instructions), each as listed now (`Ask`).
    fn triage(&mut self, ns: Vec<u64>, extra: String) {
        let asks = ns
            .into_iter()
            .map(|n| match self.items.iter().find(|i| i.number == n) {
                Some(i) => Ask { n, updated: i.updated.clone(), fixes: i.fixed_by.clone() },
                None => Ask { n, updated: String::new(), fixes: vec![] },
            })
            .collect();
        self.running_rx.push(triage_all(asks, self.root.clone(), extra));
    }

    /// Issue `n`'s fix PRs: (number, state, head).
    fn fixes(&self, n: u64) -> Vec<(u64, String, String)> {
        self.items.iter().find(|i| i.number == n).map(|i| i.fixed_by.clone()).unwrap_or_default()
    }

    /// Defer issue item `key` until `c` (or time `or`, first): what an issue can wait for, its
    /// fix PRs' merging or pushes ("the author pushes": its one open fix's), or a time.
    fn defer_on(&mut self, key: &str, c: crate::todo::Until, or: Option<i64>) {
        use crate::todo::Until;
        let Some(it) = self.todos.items.iter().find(|it| it.key() == key).cloned() else { return };
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

    /// A draft started or finished: the todo, and the open issue's list of them, again.
    fn drafts_changed(&mut self) {
        self.find_todo();
        // (Drawn again only if they changed: a redraw drops a selection being made.)
        if let Some(v) = &mut self.view {
            let d = drafts_for(v.number, &self.drafting);
            if v.drafts != d {
                v.drafts = d;
                v.redraw();
            }
        }
    }

    /// Issue `n`'s view, from GitHub, with the drafts of Claude's that fix it.
    fn load_issue(&self, n: u64) -> Result<super::issue_view::IssueView> {
        let mut v = super::issue_view::IssueView::load(n, self.reviews_dir.as_deref(), self.can_triage)?;
        v.drafts = drafts_for(n, &self.drafting);
        Ok(v)
    }

    /// `Space l` in an issue: its labels and assignees, from GitHub, to toggle (it's said while
    /// it's asked). Changing them takes triage access.
    fn open_meta(&mut self, n: u64) {
        if !self.can_triage {
            return self.msg = "labels and assignees here take triage access, which you don't have".into();
        }
        use crate::gh::Field;
        self.meta = match crate::gh::issue_fields(n) {
            Ok((labels, people)) => {
                let items = people.into_iter().map(|(p, on)| (Field::Assignee, p, on)).chain(labels.into_iter().map(|(l, on)| (Field::Label, l, on))).collect();
                Some((items, 0, false))
            }
            Err(e) => return self.msg = format!("#{n}: {e:#}"),
        };
    }

    /// A key in `Space l`'s picker: j / k move, Enter or Space puts it on or takes it off (on
    /// GitHub, now), q or Esc closes it (the issue read again).
    fn meta_key(&mut self, k: KeyEvent) {
        let Some((items, sel, changed)) = &mut self.meta else { return };
        let n = self.view.as_ref().map_or(0, |v| v.number);
        match k.code {
            KeyCode::Char('j') | KeyCode::Down => *sel = (*sel + 1).min(items.len().saturating_sub(1)),
            KeyCode::Char('k') | KeyCode::Up => *sel = sel.saturating_sub(1),
            KeyCode::Enter | KeyCode::Char(' ') => {
                let Some((field, name, on)) = items.get_mut(*sel) else { return };
                let to = !*on;
                self.msg = match crate::gh::set_issue_field(n, *field, name, to) {
                    Ok(()) => {
                        (*on, *changed) = (to, true);
                        format!("#{n}: {} {name}", if to { "added" } else { "removed" })
                    }
                    Err(e) => format!("not changed: {e:#}"),
                };
            }
            // (Read again only if something changed.)
            KeyCode::Char('q') | KeyCode::Esc if self.meta.take().is_some_and(|m| m.2) => self.reload_view(n),
            KeyCode::Char('q') | KeyCode::Esc => {}
            _ => {}
        }
    }

    fn save_defer(&mut self) {
        if let Some(e) = self.todos.save() {
            self.msg = e;
        }
        self.find_todo();
    }

    /// `r r`: Claude ranks the todo tab (one run, in the background).
    fn rank(&mut self) {
        if self.todos.rank_rx.is_some() {
            return self.msg = "Claude is ranking them already".into();
        }
        if self.todos.to_rank() == 0 {
            return self.msg = "nothing on the todo tab to rank".into();
        }
        let lines: Vec<String> = self
            .todos
            .items
            .iter()
            .filter(|it| it.deferred.is_none())
            .filter_map(|it| Some(about_issue(it, self.items.iter().find(|i| i.number == it.number)?, self.triage.get(&it.number))))
            .collect();
        if self.todos.rank(lines, &self.root, crate::todo::Of::Issues) {
            self.msg = "Claude is ranking what needs you…".into();
        }
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
        self.triage(ns, extra);
    }

    /// A card's rows, and the blank after it: on the todo tab, one more (Claude's word, beside
    /// it or under its title).
    fn card_rows(&self) -> usize {
        if self.nav.tab == ITab::Todo { 5 } else { 4 }
    }

    fn height(&self) -> usize {
        cards_fit(self.card_rows())
    }

    fn draw(&mut self, f: &mut Frame) {
        let area = f.area();
        let ui = &self.ui;
        f.render_widget(Paragraph::new("").style(ui.base), area);
        // An issue open: its own title bar (no tabs: they're the list's), it, and the status bar.
        if let Some(v) = self.view.as_mut() {
            v.title_bar(f, ui, Rect { height: 1.min(area.height), ..area });
            let cursor = v.draw(f, ui, Rect { y: area.y + 1, height: area.height.saturating_sub(2), ..area });
            let left = if self.msg.is_empty() { " ? keys · q back to the issues".to_owned() } else { format!(" {}", self.msg) };
            status_bar(f, ui, &left, &format!("#{}", v.number), self.todos.auto, "");
            if self.nav.help {
                draw_help(f, ui, super::issue_view::VIEW_HELP, "an issue's keys");
            } else if let Some((items, sel, _)) = &self.meta {
                draw_meta(f, ui, items, *sel, v.number);
            } else {
                // The keys most often wanted here, bottom right, as the PR view shows them.
                let keys = [("C", "comment"), ("Space l", "labels, assignees"), ("X X", "close it"), ("a a", "triage again"), ("Enter", "open link"), ("o", "in the browser"), ("?", "all keys")];
                draw_keys(f, ui, &keys, cursor);
            }
            return;
        }
        let shown_n = self.shown().len();
        let rows = self.card_rows();
        let h = area.height.saturating_sub(2) as usize / rows;
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
                let mut l = issue_card(i, ui, k == sel, self.marked.contains(&i.number), num_w, card_w, self.triage.get(&i.number), self.running.get(&i.number), rank, &close, self.draft_fix.get(&i.number));
                if let Some(it) = item {
                    let cx = Cards { ui, num_w: num_w.max(3), width: card_w, checks_loading: false, usual: "" };
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
fn issue_card(i: &Issue, ui: &Ui, selected: bool, marked: bool, num_w: usize, width: usize, t: Option<&Triage>, run: Option<&batch::Status>, rank: Option<usize>, close: &str, draft: Option<&(String, bool)>) -> Vec<Line<'static>> {
    let row = card_row(ui, selected);
    let tint = |st: Style| row.patch(Style { bg: None, ..st });
    let dim = tint(ui.stale);
    let sp = |s: String, st| Span::styled(s, st);
    let bar = || sp(if selected { "▌".into() } else { " ".into() }, tint(ui.key));
    let indent = || sp(" ".repeat(MARK_W + num_w + 1), row);
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
        sp(format!("{:>w$} ", format!("#{}", i.number), w = num_w), tint(ui.key)),
        sp(i.title.clone(), row.add_modifier(ratatui::style::Modifier::BOLD)),
    ];
    for (name, color) in &i.labels {
        top.extend([sp(" ".into(), row), sp(format!(" {name} "), crate::view::chip(color))]);
    }
    let talk = vec![sp(format!("💬 {} · 👍 {}  ", i.comments, i.thumbs), dim), sp(gh::ago(&i.updated), dim)];
    // Who; Claude's triage.
    // (Its place on the todo tab, under its number.)
    let first = match rank {
        Some(r) => sp(format!("{}{:>w$} ", " ".repeat(MARK_W), format!("{r}."), w = num_w), tint(ui.key)),
        None => indent(),
    };
    let mut who = vec![first, sp(format!("@{}", i.author), dim)];
    if !i.assignees.is_empty() {
        who.push(sp(format!("  → {}", i.assignees.iter().map(|a| format!("@{a}")).collect::<Vec<_>>().join(" ")), dim));
    }
    let triage = match (run, t) {
        (Some(s), _) if s.running() => vec![sp("claude: triaging…".into(), tint(ui.key))],
        (Some(batch::Status::Failed(e)), _) => vec![sp(format!("claude failed: {}", e.lines().next().unwrap_or("")), tint(ui.error))],
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
        (Some((d, true)), _) => fix.push(sp(format!("Claude is drafting a fix: {d}"), tint(ui.key))),
        (Some((d, false)), _) => fix.push(sp(format!("a draft fix: {d} (dv list's drafts tab: review it, S publishes it)"), tint(ui.plus))),
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

/// Unpublished drafts fixing an issue: the issue → the draft's label, whether Claude's at it
/// (its record says so, or this dv just started it: its record may not say yet). One whose
/// run failed or stopped before drafting doesn't count: nothing to wait on.
fn draft_fix(records: &[crate::replace::Record], here: &HashSet<u64>) -> HashMap<u64, (String, bool)> {
    records
        .iter()
        .filter(|r| r.published.is_none())
        .filter_map(|r| {
            let n = fixes(&r.task)?;
            let at_it = here.contains(&r.id) || r.drafting_now();
            (at_it || !r.title.is_empty()).then(|| (n, (crate::replace::label(r.id), at_it)))
        })
        .collect()
}

/// The drafts of Claude's that fix issue `n`, and how each stands (`here`: those this dv
/// started, under way).
fn drafts_for(n: u64, here: &HashSet<u64>) -> Vec<String> {
    records()
        .into_iter()
        .filter(|r| fixes(&r.task) == Some(n))
        .map(|r| {
            let label = crate::replace::label(r.id);
            match (r.published, here.contains(&r.id) || r.drafting_now()) {
                (Some(p), _) => format!("{label}: published, #{p}"),
                (None, true) => format!("{label}: Claude is drafting it"),
                // (Its run failed, or stopped, before a draft: nothing to publish.)
                (None, false) if r.title.is_empty() => format!("{label}: no draft (its run failed or stopped: dv list's drafts tab)"),
                (None, false) => format!("{label}: drafted, unpublished (dv list's drafts tab: review it, S publishes it)"),
            }
        })
        .collect()
}

/// The issue a draft's task is to fix: its first line's "Fix #N" (as `N N` writes it).
fn fixes(task: &str) -> Option<u64> {
    let first = task.lines().next()?.trim();
    let rest = first.get(..5).filter(|p| p.eq_ignore_ascii_case("fix #")).map(|_| &first[5..])?;
    rest.split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()
}

/// A label or an assignee of `Space l`'s: which, its name, whether it's on the issue.
type Meta = (crate::gh::Field, String, bool);

/// `Space l`'s picker over the issue: each label and assignee, on it or not.
fn draw_meta(f: &mut Frame, ui: &Ui, items: &[Meta], sel: usize, n: u64) {
    let area = f.area();
    let lines: Vec<Line> = items
        .iter()
        .enumerate()
        .map(|(i, (field, name, on))| {
            let what = if *field == crate::gh::Field::Label { "label" } else { "assignee" };
            let st = if i == sel { ui.selection } else { Style::new() };
            Line::from(vec![Span::styled(format!(" {} ", if *on { "●" } else { "○" }), st.patch(ui.key)), Span::styled(format!("{what:<9}"), st.patch(ui.stale)), Span::styled(name.clone(), st)])
        })
        .collect();
    let w = (lines.iter().map(Line::width).max().unwrap_or(20) as u16 + 4).max(48).min(area.width);
    let h = (lines.len() as u16 + 2).min(area.height.saturating_sub(2));
    let r = Rect { x: (area.width - w) / 2, y: (area.height - h) / 2, width: w, height: h };
    let inner = crate::view::popup_frame(ui, f, r, &format!("#{n}: labels, assignees (Enter toggles, q done)"), "");
    let top = sel.saturating_sub(inner.height.saturating_sub(1) as usize);
    f.render_widget(Paragraph::new(lines.into_iter().skip(top).collect::<Vec<_>>()), inner);
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
  Esc                  clear the filter
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
    // Space l in an issue: without triage access, said; its picker moves with j / k.
    l.can_triage = false;
    l.open_meta(7);
    assert!(l.msg.starts_with("labels and assignees here take triage access"), "{}", l.msg);
    l.meta = Some((vec![(crate::gh::Field::Assignee, "amy".into(), false), (crate::gh::Field::Label, "bug".into(), true)], 0, false));
    l.meta_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
    assert_eq!(l.meta.as_ref().map(|m| m.1), Some(1));
    l.meta = None;
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
fn a_drafts_task_names_the_issue_it_fixes() {
    assert_eq!(fixes("Fix #2217: KeychainTxOutIndex returns stale entries\n\nFrom triaging it: …"), Some(2217));
    assert_eq!(fixes("fix #12"), Some(12));
    assert_eq!(fixes("Refactor the index (see #12)"), None);
    assert_eq!(fixes(""), None);
}

#[test]
fn a_draft_holds_its_issue_while_it_can_come() {
    use crate::replace::Record;
    let rec = |id: u64, task: &str, title: &str| Record { id, task: task.into(), title: title.into(), ..Default::default() };
    let records = [rec(1, "Fix #7: it panics", "Fix the panic"), rec(2, "Fix #8: slow", ""), rec(3, "Fix #9: odd", ""), rec(4, "Refactor", "x")];
    // #7 drafted; #8 started here (its record doesn't say yet); #9's run failed before drafting.
    let fix = draft_fix(&records, &HashSet::from([2]));
    assert_eq!(fix.get(&7).map(|f| f.1), Some(false));
    assert_eq!(fix.get(&8).map(|f| f.1), Some(true));
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
