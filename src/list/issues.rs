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
    pub url: String,
    pub updated: String,
    /// (name, hex color)
    pub labels: Vec<(String, String)>,
    pub comments: u64,
    pub thumbs: u64,
    pub assignees: Vec<String>,
    /// PRs that would close it (they say "fixes #N"): (number, state).
    pub fixed_by: Vec<(u64, String)>,
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

impl ITab {
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

/// You, and the open issues (`fetch`).
type Fetch = Result<(String, Vec<Issue>)>;

/// How many open issues are fetched (the most recently updated).
pub const ISSUES_MAX: usize = 500;

/// You (your login), and the open issues, most recently updated first.
pub fn fetch() -> Result<(String, Vec<Issue>)> {
    let (mut me, mut all, mut after) = (String::new(), vec![], None::<String>);
    while all.len() < ISSUES_MAX {
        let from = after.as_ref().map_or(String::new(), |c| format!(", after: \"{c}\""));
        let q = format!(
            "query($owner: String!, $name: String!) {{ viewer {{ login }} repository(owner: $owner, name: $name) {{
               issues(states: OPEN, first: 100{from}, orderBy: {{ field: UPDATED_AT, direction: DESC }}) {{ {PAGE} nodes {{
                 number title url updatedAt author {{ login }}
                 labels(first: 10) {{ nodes {{ name color }} }} comments {{ totalCount }}
                 reactions(content: THUMBS_UP) {{ totalCount }} assignees(first: 5) {{ nodes {{ login }} }}
                 closedByPullRequestsReferences(first: 5, includeClosedPrs: true) {{ nodes {{ number state }} }} }} }} }} }}"
        );
        let v: Value = gh_json(&["api", "graphql", "-F", "owner={owner}", "-F", "name={repo}", "-f", &format!("query={q}")])?;
        crate::gh::refused(&v)?;
        me = v["data"]["viewer"]["login"].as_str().unwrap_or("").to_owned();
        let list = &v["data"]["repository"]["issues"];
        all.extend(list["nodes"].as_array().context("gh api graphql: issues")?.iter().map(issue));
        match next_page(list) {
            Some(c) => after = Some(c),
            None => break,
        }
    }
    Ok((me, all))
}

fn issue(i: &Value) -> Issue {
    let s = |v: &Value| v.as_str().unwrap_or("").to_owned();
    let nodes = |k: &str| i[k]["nodes"].as_array().cloned().unwrap_or_default();
    Issue {
        number: i["number"].as_u64().unwrap_or(0),
        title: s(&i["title"]),
        author: s(&i["author"]["login"]),
        url: s(&i["url"]),
        updated: s(&i["updatedAt"]),
        labels: nodes("labels").iter().map(|l| (s(&l["name"]), s(&l["color"]))).collect(),
        comments: i["comments"]["totalCount"].as_u64().unwrap_or(0),
        thumbs: i["reactions"]["totalCount"].as_u64().unwrap_or(0),
        assignees: nodes("assignees").iter().map(|a| s(&a["login"])).collect(),
        fixed_by: nodes("closedByPullRequestsReferences").iter().map(|p| (p["number"].as_u64().unwrap_or(0), s(&p["state"]))).collect(),
    }
}

/// Claude's triage of issue `n`, if it has one (`dir`: the reviews' directory).
pub fn triage_of(dir: &std::path::Path, n: u64) -> Option<Triage> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(dir.join(format!("issue-{n}.json"))).ok()?).ok()?;
    let s = |k: &str| v[k].as_str().unwrap_or("").trim().to_owned();
    Some(Triage { kind: s("kind"), priority: s("priority"), next: s("next"), close: s("close"), updated: s("updated") })
}

/// What issue `i` needs of you (`t`: Claude's triage of it, written at `t_at`, seconds):
/// the PR fixing it merged (close it), Claude would close it, assigned to you, a PR would fix it
/// (review that), changed since Claude triaged it, not triaged.
pub fn issue_needs(i: &Issue, t: Option<&Triage>, t_at: Option<i64>, me: &str) -> Vec<crate::todo::Need> {
    use crate::todo::Need;
    let mut out = vec![];
    if let Some(&(n, _)) = i.fixed_by.iter().find(|p| p.1 == "MERGED") {
        out.push(Need::FixMerged(n));
    }
    if let Some(t) = t.filter(|t| !t.close.is_empty()) {
        out.push(Need::WouldClose(t.close.clone()));
    }
    if !me.is_empty() && i.assignees.iter().any(|a| a == me) {
        out.push(Need::Assigned);
    }
    if let Some(&(n, _)) = i.fixed_by.iter().find(|p| p.1 == "OPEN") {
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
fn triage_run(n: u64, updated: &str, root: &std::path::Path, extra: &str) -> Result<batch::Status> {
    let file = crate::claude::dv_dir()?.join("reviews").join(format!("issue-{n}.json"));
    let base = crate::diff::remote_branch(&crate::diff::default_branch());
    let prompt = format!(
        "{}Use the dv-review skill to triage issue #{n} (see \"Triage an issue\"); the code as it is on the base branch \
         is `{base}` (`git show {base}:<path>`, `git log {base}`): this checkout may be on another branch. Reply with \
         only the JSON object. (It's {} now.)",
        crate::claude::notes_preamble(),
        crate::claude::now()
    );
    let prompt = crate::claude::with_instructions(&prompt, extra);
    let tools = ["Bash(gh issue view:*)", "Bash(gh issue list:*)", "Bash(gh search:*)", "Bash(gh label list:*)"];
    let out = crate::claude::claude_reader(&prompt, root, &tools).stdin(std::process::Stdio::null()).output().context("is Claude Code (`claude`) installed?")?;
    let v: Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
    let answer = v["result"].as_str().unwrap_or("");
    // (Its own word for what went wrong, where it said one.)
    let why = if answer.is_empty() { String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("").to_owned() } else { answer.lines().next().unwrap_or("").to_owned() };
    anyhow::ensure!(out.status.success() && v["is_error"] == false, "Claude stopped ({}): {why}", out.status);
    let mut t = crate::claude::json_in(answer, '{').filter(Value::is_object).context("no triage in Claude's answer")?;
    (t["issue"], t["updated"]) = (n.into(), updated.into());
    std::fs::create_dir_all(file.parent().context("no dir")?)?;
    crate::claude::write(&file, &t)?;
    Ok(batch::Status::Done(format!("{} turns, ${:.2}", v["num_turns"].as_u64().unwrap_or(0), v["total_cost_usd"].as_f64().unwrap_or(0.0))))
}

/// Claude triages each of `ns`, a couple at a time, in the background: its news on the receiver.
/// (Each with its `updatedAt` as listed: what it's triaged as of.)
fn triage_all(ns: Vec<(u64, String)>, root: PathBuf, extra: String) -> Receiver<(u64, batch::Status)> {
    batch::pool(ns, move |(n, updated), tx| {
        _ = tx.send((*n, batch::Status::Reviewing));
        (*n, triage_run(*n, updated, &root, &extra).unwrap_or_else(|e| batch::Status::Failed(format!("{e:#}"))))
    })
}

/// What the issues list asks of its caller.
pub enum IssuesDo {
    Quit,
    /// Back to the PRs (`I I`).
    Prs,
}

#[derive(Default)]
pub struct Issues {
    ui: Ui,
    items: Vec<Issue>,
    me: String,
    tab: ITab,
    sel: usize,
    top: usize,
    query: String,
    filtering: bool,
    help: bool,
    /// The first of a two-key command (`g`, `a`, `I`, `U`), waiting for the second.
    pending: Option<char>,
    marked: HashSet<u64>,
    /// Claude's triage of each issue (from its review file), and the runs under way.
    triage: HashMap<u64, Triage>,
    /// When each triage was written (seconds): an issue changed since is to triage again.
    triage_at: HashMap<u64, i64>,
    /// The todo tab: what needs you, in order; Claude's last ranking of it, and the next on its
    /// way; deferred items (`s`, kept in `deferred-issues.json`), and the one `s` is asking about.
    todo: Vec<crate::todo::Item>,
    ranked: Option<crate::todo::Ranked>,
    rank_rx: Option<Receiver<Result<crate::todo::Ranked>>>,
    deferred: crate::todo::Deferred,
    deferring: Option<String>,
    running: HashMap<u64, batch::Status>,
    running_rx: Vec<Receiver<(u64, batch::Status)>>,
    reviews_dir: Option<PathBuf>,
    reviews_mtime: Option<std::time::SystemTime>,
    fetched: Option<Receiver<Fetch>>,
    root: PathBuf,
    pub msg: String,
}

impl Issues {
    pub fn new() -> Issues {
        Issues {
            ui: Theme::load().ui(),
            ranked: crate::todo::load_of(crate::todo::Of::Issues),
            deferred: crate::todo::load_deferred(crate::todo::deferred_path_of(crate::todo::Of::Issues).as_deref()),
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
        let items: Vec<Item> = self
            .items
            .iter()
            .filter_map(|i| {
                let needs = issue_needs(i, self.triage.get(&i.number), self.triage_at.get(&i.number).copied(), &self.me);
                (!needs.is_empty()).then(|| Item { kind: Kind::Issue, number: i.number, needs, claude: None, updated: i.updated.clone(), reason: None, changed: false, deferred: None, busy: None, waits: None })
            })
            .collect();
        let (items, _) = crate::todo::order(items, self.ranked.as_ref());
        // Deferred: at the bottom till then, or till what it needs changes (as dv list's).
        self.todo = crate::todo::defer(items, &self.deferred, crate::claude::now_secs(), |_, _| false);
    }

    /// Those not deferred: what needs you now.
    fn todo_now(&self) -> usize {
        self.todo.iter().filter(|it| it.deferred.is_none()).count()
    }

    /// The issues on the tab shown, filtered.
    pub fn shown(&self) -> Vec<&Issue> {
        if self.tab == ITab::Todo {
            return self.todo.iter().filter_map(|it| self.items.iter().find(|i| i.number == it.number)).filter(|i| self.matches(i)).collect();
        }
        self.items
            .iter()
            .filter(|i| match self.tab {
                ITab::Todo => true,
                ITab::Open => true,
                ITab::Mine => i.author == self.me || i.assignees.contains(&self.me),
                ITab::CanClose => self.triage.get(&i.number).is_some_and(|t| !t.close.is_empty()),
            })
            .filter(|i| self.matches(i))
            .collect()
    }

    /// Whether `i` matches the filter: every term (`label:`, `author:`, `kind:`, `is:triaged`,
    /// `-term` excludes; else in its number, title, author or labels).
    fn matches(&self, i: &Issue) -> bool {
        let t = self.triage.get(&i.number);
        self.query.split_whitespace().all(|term| {
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
                    Ok((me, items)) => {
                        let keep = self.shown().get(self.sel).map(|i| i.number);
                        (self.me, self.items) = (me, items);
                        self.reviews_mtime = None; // read the triages for these
                        self.watch_reviews();
                        self.sel = keep.and_then(|n| self.shown().iter().position(|i| i.number == n)).unwrap_or(0);
                        if self.msg.starts_with("fetching") {
                            self.msg.clear();
                        }
                    }
                    Err(e) => self.msg = format!("issues: {e:#}"),
                }
            }
            self.watch_reviews();
            // A deferral whose time's come: back.
            let now = crate::claude::now_secs();
            if self.todo.iter().any(|it| it.deferred.as_ref().is_some_and(|d| d.until <= now)) {
                self.find_todo();
            }
            if let Some(Ok(res)) = self.rank_rx.as_ref().map(Receiver::try_recv) {
                self.rank_rx = None;
                match res {
                    Ok(r) => {
                        self.msg = format!("Claude ranked what needs you (${:.2})", r.cost);
                        self.ranked = Some(r);
                        self.find_todo();
                    }
                    Err(e) => self.msg = format!("not ranked: {e:#}"),
                }
            }
            for rx in &self.running_rx {
                while let Ok((n, st)) = rx.try_recv() {
                    match &st {
                        batch::Status::Failed(e) => self.msg = format!("#{n}: {}", e.lines().next().unwrap_or("")),
                        batch::Status::Done(took) => {
                            self.msg = format!("triaged #{n} ({took})");
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
                // The wheel moves the cursor, as j / k.
                Event::Mouse(m) => match m.kind {
                    event::MouseEventKind::ScrollDown => KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
                    event::MouseEventKind::ScrollUp => KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE),
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
        if std::mem::take(&mut self.help) {
            return None; // any key closes it
        }
        if self.filtering {
            match k.code {
                KeyCode::Esc => (self.query.clear(), self.filtering = false).1,
                KeyCode::Enter => self.filtering = false,
                KeyCode::Backspace => _ = self.query.pop(),
                KeyCode::Char(c) => self.query.push(c),
                _ => {}
            }
            self.sel = 0;
            return None;
        }
        let n = self.shown().len();
        let half = (self.height() / 2).max(1);
        let pending = self.pending.take();
        let this = self.shown().get(self.sel).map(|i| i.number);
        self.msg.clear();
        match (pending, k.code) {
            (_, KeyCode::Char('d')) if ctrl => self.sel = (self.sel + half).min(n.saturating_sub(1)),
            (_, KeyCode::Char('u')) if ctrl => self.sel = self.sel.saturating_sub(half),
            (Some('g'), KeyCode::Char('g')) => self.sel = 0,
            (Some('g'), KeyCode::Char('e')) => self.sel = n.saturating_sub(1),
            (Some('g'), KeyCode::Char('n')) => (self.tab, self.sel) = (self.tab.next(), 0),
            (Some('g'), KeyCode::Char('p')) => (self.tab, self.sel) = (self.tab.prev(), 0),
            (Some('I'), KeyCode::Char('I')) => return Some(IssuesDo::Prs),
            (Some('U'), KeyCode::Char('U')) => {
                self.msg = format!("unmarked {}", self.marked.len());
                self.marked.clear();
            }
            (Some('a'), KeyCode::Char('a')) => self.triage_marked(term),
            (Some('r'), KeyCode::Char('r')) => self.rank(),
            // `s` asked how long: 1 an hour, 4 four hours, d a day, w a week.
            (Some('s'), KeyCode::Char(c @ ('1' | '4' | 'd' | 'w'))) => self.defer_for(c),
            (Some('s'), _) => self.msg = "not deferred".into(),
            (_, KeyCode::Char('s')) if self.tab == ITab::Todo => self.defer_key(),
            (_, KeyCode::Char('r')) if self.tab == ITab::Todo => {
                self.pending = Some('r');
                self.msg = format!("r again: Claude ranks the {} issue(s) that need you (one run; {})", self.todo_now(), match &self.ranked {
                    Some(r) => format!("ranked {} ago, cost ${:.2}", gh::ago_secs(crate::claude::now_secs() - r.at), r.cost),
                    None => "never ranked before".into(),
                });
            }
            (_, KeyCode::Char(c @ ('g' | 'I' | 'U' | 'a'))) => {
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
            (_, KeyCode::Char('j') | KeyCode::Down) => self.sel = (self.sel + 1).min(n.saturating_sub(1)),
            (_, KeyCode::Char('k') | KeyCode::Up) => self.sel = self.sel.saturating_sub(1),
            (_, KeyCode::Char('/')) => self.filtering = true,
            (_, KeyCode::Esc) => (self.query.clear(), self.sel = 0).1,
            (_, KeyCode::Char('?')) => self.help = true,
            (Some('q'), KeyCode::Char('q')) => return Some(IssuesDo::Quit),
            (_, KeyCode::Char('q')) if self.running.values().any(batch::Status::running) => {
                self.pending = Some('q');
                self.msg = "Claude is still triaging: q again quits (runs under way finish, but their triage is lost)".into();
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
                    self.sel = (self.sel + 1).min(self.shown().len().saturating_sub(1));
                }
            }
            // On GitHub, for now (dv's own view of an issue comes later).
            (_, KeyCode::Enter) => {
                if let Some(url) = self.shown().get(self.sel).map(|i| i.url.clone()) {
                    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
                    self.msg = match std::process::Command::new(opener).arg(&url).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn() {
                        Ok(_) => format!("opened {url}"),
                        Err(e) => format!("{opener}: {e}"),
                    };
                }
            }
            _ => {}
        }
        self.sel = self.sel.min(self.shown().len().saturating_sub(1));
        None
    }

    /// `s` on a todo item: defer it (the next key says how long), or bring a deferred one back.
    fn defer_key(&mut self) {
        let Some(n) = self.shown().get(self.sel).map(|i| i.number) else { return };
        let Some(it) = self.todo.iter().find(|it| it.number == n) else { return };
        if it.deferred.is_some() {
            self.deferred.remove(&it.key());
            self.msg = format!("#{n} is back");
            return self.save_defer();
        }
        self.msg = "defer it for: 1 an hour, 4 four hours, d a day, w a week (any other key: not); back sooner if what it needs changes".into();
        (self.pending, self.deferring) = (Some('s'), Some(it.key()));
    }

    fn defer_for(&mut self, c: char) {
        let Some(key) = self.deferring.take() else { return };
        let Some(it) = self.todo.iter().find(|it| it.key() == key) else { return };
        let (secs, how) = crate::todo::defer_secs(c);
        self.deferred.insert(key, crate::todo::Deferral { until: crate::claude::now_secs() + secs, needs: it.needs_key(), cond: None });
        self.msg = format!("deferred for {how} (s on it brings it back)");
        self.save_defer();
    }

    fn save_defer(&mut self) {
        if let Err(e) = crate::todo::save_deferred(crate::todo::deferred_path_of(crate::todo::Of::Issues).as_deref(), &self.deferred) {
            self.msg = format!("couldn't save it: {e:#}");
        }
        self.find_todo();
    }

    /// `r r`: Claude ranks the todo tab (one run, in the background).
    fn rank(&mut self) {
        if self.rank_rx.is_some() {
            return self.msg = "Claude is ranking them already".into();
        }
        if self.todo_now() == 0 {
            return self.msg = "nothing on the todo tab to rank".into();
        }
        let now: Vec<crate::todo::Item> = self.todo.iter().filter(|it| it.deferred.is_none()).cloned().collect();
        let lines: Vec<String> = now
            .iter()
            .filter_map(|it| Some(about_issue(it, self.items.iter().find(|i| i.number == it.number)?, self.triage.get(&it.number))))
            .collect();
        self.rank_rx = Some(crate::todo::rank_of(lines, crate::todo::fingerprint(&now), self.root.clone(), crate::todo::Of::Issues));
        self.msg = "Claude is ranking what needs you…".into();
    }

    /// `a a`: Claude triages the marked issues (or this one), with your instructions if any.
    fn triage_marked(&mut self, term: &mut DefaultTerminal) {
        let ns: Vec<u64> = match self.marked.is_empty() {
            true => self.shown().get(self.sel).map(|i| i.number).into_iter().collect(),
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
        let ns = ns.into_iter().map(|n| (n, self.items.iter().find(|i| i.number == n).map_or(String::new(), |i| i.updated.clone()))).collect();
        self.running_rx.push(triage_all(ns, self.root.clone(), extra));
    }

    /// A card's rows, and the blank after it: on the todo tab, one more (Claude's word, beside
    /// it or under its title).
    fn card_rows(&self) -> usize {
        if self.tab == ITab::Todo { 5 } else { 4 }
    }

    fn height(&self) -> usize {
        ratatui::crossterm::terminal::size().map_or(10, |(_, h)| h.saturating_sub(2) as usize / self.card_rows())
    }

    fn draw(&mut self, f: &mut Frame) {
        let area = f.area();
        let ui = &self.ui;
        f.render_widget(Paragraph::new("").style(ui.base), area);
        // The top bar: which list this is, its tabs and counts, where you are.
        let hs = ui.base.patch(Style { bg: ui.bar, ..ui.header });
        let dim = hs.remove_modifier(ratatui::style::Modifier::BOLD).patch(Style { fg: ui.linenr.fg, ..Style::new() });
        let mine = self.items.iter().filter(|i| i.author == self.me || i.assignees.contains(&self.me)).count();
        let close = self.items.iter().filter(|i| self.triage.get(&i.number).is_some_and(|t| !t.close.is_empty())).count();
        let tabs = [
            (ITab::Todo, format!("todo {}", self.todo_now())),
            (ITab::Open, format!("open {}", self.items.len())),
            (ITab::Mine, format!("mine {mine}")),
            (ITab::CanClose, format!("close {close}")),
        ];
        let mut spans = vec![Span::styled(" Issues ", hs.add_modifier(ratatui::style::Modifier::BOLD)), Span::styled(" ", hs)];
        for (t, name) in tabs {
            spans.extend([Span::styled(format!(" {name} "), if self.tab == t { ui.menu_selected } else { dim }), Span::styled(" ", hs)]);
        }
        spans.push(Span::styled("   I I the PRs", dim));
        f.render_widget(Paragraph::new(Line::from(spans)).style(hs), Rect { height: 1.min(area.height), ..area });
        let shown = self.shown();
        let rows = self.card_rows();
        // The todo tab: Claude's word beside each card when the window's wide enough, else
        // under its title (as dv list's).
        let column = (self.tab == ITab::Todo).then(|| msg_column(area.width as usize)).flatten();
        let h = area.height.saturating_sub(2) as usize / rows;
        let sel = self.sel.min(shown.len().saturating_sub(1));
        let top = self.top.min(sel).max((sel + 1).saturating_sub(h));
        let num_w = shown.iter().map(|i| format!("#{}", i.number).len()).max().unwrap_or(2);
        let width = area.width as usize;
        let lines: Vec<Line> = shown
            .iter()
            .enumerate()
            .skip(top)
            .take(h)
            .flat_map(|(k, i)| {
                let item = (self.tab == ITab::Todo).then(|| self.todo.iter().find(|it| it.number == i.number)).flatten();
                let card_w = column.unwrap_or(width);
                let rank = item.filter(|_| column.is_some()).map(|_| k + 1); // (narrow: on the line under the title)
                let mut l = issue_card(i, ui, k == sel, self.marked.contains(&i.number), num_w, card_w, self.triage.get(&i.number), self.running.get(&i.number), rank);
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
            let none = match (self.fetched.is_some(), self.tab) {
                (true, _) => " fetching…",
                (_, ITab::CanClose) => " nothing Claude would close (a a triages issues)",
                (_, ITab::Mine) => " no open issues assigned to you or opened by you",
                _ => " no open issues",
            };
            f.render_widget(Paragraph::new(none).style(ui.base.patch(ui.stale)), Rect { y: area.y + 2, height: 1, ..area });
        }
        f.render_widget(Paragraph::new(lines), Rect { y: area.y + 1, height: (h * rows) as u16, ..area });
        // The status bar: the filter or the message, and the keys to know.
        let status = Rect { y: area.bottom().saturating_sub(1), height: 1.min(area.height), ..area };
        let bar = ui.base.patch(ui.status);
        let left = match (self.filtering, self.query.is_empty(), self.msg.is_empty()) {
            (true, ..) => format!(" /{}▏  label: author: kind: priority: is:triaged  -x negates", self.query),
            (_, _, false) => format!(" {}", self.msg),
            (_, false, _) => format!(" filter: {}  (Esc clears)", self.query),
            _ => " ? keys · I I the PRs".into(),
        };
        let pos = if shown.is_empty() { String::new() } else { format!("{} / {} ", sel + 1, shown.len()) };
        f.render_widget(Paragraph::new(left).style(bar), status);
        let w = (pos.len() as u16).min(status.width);
        f.render_widget(Paragraph::new(pos).style(bar), Rect { x: status.right() - w, width: w, ..status });
        if self.help {
            draw_help(f, ui, ISSUES_HELP, "dv issues keys");
        }
        self.top = top;
    }
}

/// An issue's card, three lines: number, title and labels, its comments, 👍 and age; who opened
/// it and who has it, and Claude's triage (what kind, how pressing); the PRs that would fix it,
/// and what Claude says is next (or that it's at it).
#[allow(clippy::too_many_arguments)]
fn issue_card(i: &Issue, ui: &Ui, selected: bool, marked: bool, num_w: usize, width: usize, t: Option<&Triage>, run: Option<&batch::Status>, rank: Option<usize>) -> Vec<Line<'static>> {
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
        (_, Some(t)) if !t.close.is_empty() => vec![sp(format!("✕ close: {}", t.close), tint(ui.warning))],
        (_, Some(t)) => vec![sp([t.kind.as_str(), t.priority.as_str()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" · "), tint(ui.key))],
        _ => vec![],
    };
    // What would fix it; what's next.
    let mut fix = vec![indent()];
    if !i.fixed_by.is_empty() {
        let prs: Vec<String> = i.fixed_by.iter().map(|(n, s)| format!("#{n} ({})", s.to_lowercase())).collect();
        fix.push(sp(format!("fixed by {}   ", prs.join(", ")), tint(ui.plus)));
    }
    if let Some(t) = t.filter(|t| !t.next.is_empty()) {
        fix.push(sp(format!("→ {}", t.next), tint(ui.key)));
    }
    let claude = match t {
        Some(_) => vec![sp("claude ✓".into(), tint(ui.key))],
        None => vec![],
    };
    vec![zones(top, talk), zones(who, triage), zones(fix, claude)]
}

pub(super) const ISSUES_HELP: &str = "
MOVE
  j k, Up Down         move
  gg ge                first / last issue
  Ctrl-d Ctrl-u        half a page down / up
  gn gp                next / previous tab (todo, open, mine, can close)
  Enter                open it on GitHub
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
  s                    defer it: 1 an hour, 4 four hours, d a day, w a week (again: back)

CLAUDE
  m                    mark the issue (again: unmark)
  U U                  unmark all
  a a                  Claude triages the marked issues (or this one), two at a time";

#[test]
fn issues_listed_triaged_filtered() {
    use crate::view::tests::{screen, snapshot};
    let v: Value = serde_json::from_str(r#"{"number": 7, "title": "Panics on reorg", "url": "https://github.com/o/r/issues/7",
        "updatedAt": "2026-01-01T00:00:00Z", "author": {"login": "amy"}, "labels": {"nodes": [{"name": "bug", "color": "d73a4a"}]},
        "comments": {"totalCount": 3}, "reactions": {"totalCount": 2}, "assignees": {"nodes": [{"login": "me"}]},
        "closedByPullRequestsReferences": {"nodes": [{"number": 9, "state": "OPEN"}]}}"#).unwrap();
    let a = issue(&v);
    assert_eq!((a.number, a.author.as_str(), a.comments, a.thumbs, a.fixed_by.clone()), (7, "amy", 3, 2, vec![(9, "OPEN".to_owned())]));
    let dir = std::env::temp_dir().join(format!("dv-issues-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("issue-8.json"), r#"{"kind": "question", "priority": "low", "next": "answer it", "close": "answered in #3"}"#).unwrap();
    let mut l = Issues { me: "me".into(), reviews_dir: Some(dir.clone()), ..Default::default() };
    l.items = vec![a, Issue { number: 8, title: "How do I sync?".into(), author: "bo".into(), updated: "2026-01-02T00:00:00Z".into(), ..Default::default() }];
    l.watch_reviews();
    l.tab = ITab::Open;
    assert_eq!(l.triage.get(&8).map(|t| t.close.as_str()), Some("answered in #3"));
    let shown = |l: &Issues| l.shown().iter().map(|i| i.number).collect::<Vec<_>>();
    assert_eq!(shown(&l), [7, 8]);
    l.tab = ITab::Mine;
    assert_eq!(shown(&l), [7]); // assigned to you
    l.tab = ITab::CanClose;
    assert_eq!(shown(&l), [8]);
    l.tab = ITab::Open;
    l.query = "label:bug".into();
    assert_eq!(shown(&l), [7]);
    l.query = "-is:triaged".into();
    assert_eq!(shown(&l), [7]);
    l.query = "kind:question".into();
    assert_eq!(shown(&l), [8]);
    l.query.clear();
    l.items.iter_mut().for_each(|i| i.updated.clear()); // (no age: the snapshot stays as it is)
    snapshot("issues", &screen(140, 12, |f| l.draw(f)));
    // The todo tab: #7 assigned to you, a PR would fix it; #8 Claude would close.
    l.find_todo();
    assert_eq!(l.todo.iter().map(|it| (it.number, it.needs.clone())).collect::<Vec<_>>(), [
        (8, vec![crate::todo::Need::WouldClose("answered in #3".into())]),
        (7, vec![crate::todo::Need::Assigned, crate::todo::Need::FixOpen(9), crate::todo::Need::Untriaged]),
    ]);
    l.tab = ITab::Todo;
    snapshot("issues_todo", &screen(140, 14, |f| l.draw(f)));
    snapshot("issues_todo_wide", &screen(200, 14, |f| l.draw(f)));
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
    println!("{:?}", triage_run(n, "", &repo, ""));
    let dir = crate::claude::dv_dir().unwrap().join("reviews");
    println!("{:?}", triage_of(&dir, n));
    assert!(triage_of(&dir, n).is_some());
}
