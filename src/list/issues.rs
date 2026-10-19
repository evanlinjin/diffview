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
}

/// The issues' tabs.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum ITab {
    #[default]
    Open,
    /// Assigned to you, or opened by you.
    Mine,
    /// Claude's triage would close.
    CanClose,
}

impl ITab {
    fn next(self) -> ITab {
        match self {
            ITab::Open => ITab::Mine,
            ITab::Mine => ITab::CanClose,
            ITab::CanClose => ITab::Open,
        }
    }

    fn prev(self) -> ITab {
        self.next().next()
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
    Some(Triage { kind: s("kind"), priority: s("priority"), next: s("next"), close: s("close") })
}

/// Claude triages issue `n` (`extra`: your instructions), in the main checkout, reading only
/// (it can't edit: its answer is the triage, which dv writes to `issue-N.json` beside the PRs'
/// reviews).
fn triage_run(n: u64, root: &std::path::Path, extra: &str) -> Result<batch::Status> {
    let file = crate::claude::dv_dir()?.join("reviews").join(format!("issue-{n}.json"));
    let base = crate::diff::remote_branch(&crate::list::issues::default_branch());
    let prompt = format!(
        "Use the dv-review skill to triage issue #{n} (see \"Triage an issue\"); the code as it is on the base branch \
         is `{base}` (`git show {base}:<path>`, `git log {base}`): this checkout may be on another branch. Reply with \
         only the JSON object. (It's {} now.)",
        crate::claude::now()
    );
    let prompt = crate::claude::with_instructions(&prompt, extra);
    let tools = ["Bash(gh issue view:*)", "Bash(gh issue list:*)", "Bash(gh search:*)", "Bash(gh label list:*)"];
    let out = crate::claude::claude_reader(&prompt, root, &tools).stdin(std::process::Stdio::null()).output().context("is Claude Code (`claude`) installed?")?;
    let v: Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
    anyhow::ensure!(out.status.success() && v["is_error"] == false, "Claude stopped ({}): {}", out.status, String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or(""));
    let answer = v["result"].as_str().unwrap_or("");
    let json = answer.get(answer.find('{').context("no triage in Claude's answer")?..=answer.rfind('}').context("no triage")?).unwrap_or("");
    let mut t: Value = serde_json::from_str(json).context("Claude's triage isn't JSON")?;
    t["issue"] = n.into();
    std::fs::create_dir_all(file.parent().context("no dir")?)?;
    crate::claude::write(&file, &t)?;
    Ok(batch::Status::Done(format!("{} turns, ${:.2}", v["num_turns"].as_u64().unwrap_or(0), v["total_cost_usd"].as_f64().unwrap_or(0.0))))
}

/// The repo's default branch (the base issues are about), as `origin/HEAD` names it.
pub fn default_branch() -> String {
    crate::diff::git(&["rev-parse", "--abbrev-ref", &format!("{}/HEAD", crate::diff::remote())])
        .ok()
        .and_then(|b| b.trim().split_once('/').map(|(_, b)| b.to_owned()))
        .unwrap_or_else(|| "master".into())
}

/// Claude triages each of `ns`, a couple at a time, in the background: its news on the receiver.
fn triage_all(ns: Vec<u64>, root: PathBuf, extra: String) -> Receiver<(u64, batch::Status)> {
    let (tx, rx) = mpsc::channel();
    ns.iter().for_each(|&n| _ = tx.send((n, batch::Status::Queued)));
    let queue = std::sync::Arc::new(std::sync::Mutex::new(std::collections::VecDeque::from(ns)));
    for _ in 0..batch::AT_ONCE {
        let (queue, tx, root, extra) = (queue.clone(), tx.clone(), root.clone(), extra.clone());
        std::thread::spawn(move || {
            while let Some(n) = queue.lock().ok().and_then(|mut q| q.pop_front()) {
                _ = tx.send((n, batch::Status::Reviewing));
                let st = triage_run(n, &root, &extra).unwrap_or_else(|e| batch::Status::Failed(format!("{e:#}")));
                _ = tx.send((n, st));
            }
        });
    }
    rx
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
    }

    /// The issues on the tab shown, filtered.
    pub fn shown(&self) -> Vec<&Issue> {
        self.items
            .iter()
            .filter(|i| match self.tab {
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
            let (neg, term) = match term.strip_prefix('-').filter(|t| !t.is_empty()) {
                Some(t) => (true, t),
                None => (false, term),
            };
            let low = |s: &str| s.to_lowercase();
            let term = low(term);
            let hit = match term.split_once(':') {
                Some(("label", v)) => i.labels.iter().any(|l| low(&l.0).contains(v)),
                Some(("author", v)) => low(&i.author).contains(v),
                Some(("kind", v)) => t.is_some_and(|t| low(&t.kind).contains(v)),
                Some(("priority", v)) => t.is_some_and(|t| low(&t.priority).contains(v)),
                Some(("is", "triaged")) => t.is_some(),
                _ => {
                    let hay = format!("#{} {} {} {}", i.number, low(&i.title), low(&i.author), i.labels.iter().map(|l| low(&l.0)).collect::<Vec<_>>().join(" "));
                    hay.contains(&term)
                }
            };
            hit != neg
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
                self.msg = "Claude is still triaging: q again quits (its runs stop with dv)".into();
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
        let in_pane = crate::view::in_zellij();
        if !in_pane {
            crate::view::restore_terminal();
        }
        let last = crate::claude::instructions("triage");
        let extra = crate::view::compose_with("instructions for Claude's triage (optional)", true, last.trim(), true);
        if !in_pane {
            *term = crate::view::init_terminal();
        }
        let extra = match extra {
            Ok(Some(e)) => e,
            Ok(None) => return self.msg = "cancelled".into(),
            Err(e) => return self.msg = format!("{e:#}"),
        };
        _ = crate::claude::save_instructions("triage", &extra);
        self.msg = format!("Claude is triaging {} issue(s), {} at a time", ns.len(), batch::AT_ONCE);
        self.marked.clear();
        self.running_rx.push(triage_all(ns, self.root.clone(), extra));
    }

    fn height(&self) -> usize {
        ratatui::crossterm::terminal::size().map_or(10, |(_, h)| h.saturating_sub(2) as usize / 4)
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
        let tabs = [(ITab::Open, format!("open {}", self.items.len())), (ITab::Mine, format!("mine {mine}")), (ITab::CanClose, format!("close {close}"))];
        let mut spans = vec![Span::styled(" ISSUES ", hs.add_modifier(ratatui::style::Modifier::BOLD)), Span::styled(" ", hs)];
        for (t, name) in tabs {
            spans.extend([Span::styled(format!(" {name} "), if self.tab == t { ui.menu_selected } else { dim }), Span::styled(" ", hs)]);
        }
        f.render_widget(Paragraph::new(Line::from(spans)).style(hs), Rect { height: 1.min(area.height), ..area });
        let shown = self.shown();
        let rows = 4;
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
                let mut l = issue_card(i, ui, k == sel, self.marked.contains(&i.number), num_w, width, self.triage.get(&i.number), self.running.get(&i.number));
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
            let body = crate::view::help_lines(ISSUES_HELP, ui, area.width.saturating_sub(4) as usize, "");
            let w = (body.iter().map(Line::width).max().unwrap_or(0) as u16 + 4).min(area.width);
            let h = (body.len() as u16 + 3).min(area.height);
            let r = Rect { x: (area.width - w) / 2, y: (area.height - h) / 2, width: w, height: h };
            let inner = crate::view::popup_frame(ui, f, r, "dv issues keys", "any key closes");
            f.render_widget(Paragraph::new(body), Rect { y: inner.y + 1, height: inner.height.saturating_sub(1), ..inner });
        }
        self.top = top;
    }
}

/// An issue's card, three lines: number, title and labels, its comments, 👍 and age; who opened
/// it and who has it, and Claude's triage (what kind, how pressing); the PRs that would fix it,
/// and what Claude says is next (or that it's at it).
#[allow(clippy::too_many_arguments)]
fn issue_card(i: &Issue, ui: &Ui, selected: bool, marked: bool, num_w: usize, width: usize, t: Option<&Triage>, run: Option<&batch::Status>) -> Vec<Line<'static>> {
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
    let mut who = vec![indent(), sp(format!("@{}", i.author), dim)];
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
  gn gp                next / previous tab (open, mine, can close)
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
    println!("{:?}", triage_run(n, &repo, ""));
    let dir = crate::claude::dv_dir().unwrap().join("reviews");
    println!("{:?}", triage_of(&dir, n));
    assert!(triage_of(&dir, n).is_some());
}
