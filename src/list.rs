//! `dv list`: the repo's open PRs; `Enter` reviews one, and quitting that review comes back here.
use crate::gh::{self, Listed};
use crate::theme::{Theme, Ui};
use anyhow::Result;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Alignment, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use ratatui::{DefaultTerminal, Frame};
use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

type Checks = anyhow::Result<HashMap<u64, (usize, usize, usize)>>;

const HELP: &str = "\
  j k, Up Down         move
  gg ge                first / last PR
  Ctrl-d Ctrl-u        half a page down / up
  Enter                review the PR (with dv list's flags); q there comes back here
  /                    filter (Enter keeps it, Esc clears it); terms must all match:
                         text            number, title, author, branch, labels
                         author: label: base: branch:   part of that field
                         is:draft  review:approved|changes|required|none
                         ci:pass|fail|pending|none     -term excludes
  Esc                  clear the filter
  o O                  next sort order (updated, created, number, size) / reverse
  R                    refresh
  ?                    this help
  q, Ctrl-c            quit";

pub struct List {
    prs: Vec<Listed>,
    ui: Ui,
    /// Flags each PR opens with, for the status bar (e.g. "-wb").
    flags: String,
    sel: usize,
    top: usize,
    query: String,
    /// Typing a `/` filter (Enter keeps it, Esc clears it).
    filtering: bool,
    pending_g: bool,
    help: bool,
    order: Order,
    /// Oldest / smallest / lowest first instead.
    rev: bool,
    /// CI check counts, still on their way (they take a few seconds for a busy repo).
    checks: Option<Receiver<Checks>>,
    pub msg: String,
}

impl List {
    pub fn new(flags: String) -> List {
        let ui = Theme::load().ui();
        let (query, msg) = (String::new(), String::new());
        List { prs: vec![], ui, flags, sel: 0, top: 0, query, filtering: false, pending_g: false, help: false, order: Order::Updated, rev: false, checks: None, msg }
    }

    /// Show the list until a PR is picked (`Some`) or the user quits (`None`).
    pub fn run(&mut self, term: &mut DefaultTerminal) -> Result<Option<u64>> {
        if self.prs.is_empty() {
            self.refresh(term)?;
        }
        loop {
            term.draw(|f| self.draw(f))?;
            if let Some(Ok(res)) = self.checks.as_ref().map(Receiver::try_recv) {
                self.checks = None;
                match res {
                    Ok(by_pr) => self.prs.iter_mut().for_each(|p| p.checks = by_pr.get(&p.number).copied()),
                    Err(e) => self.msg = format!("checks: {e:#}"),
                }
            }
            if !event::poll(Duration::from_millis(100))? {
                continue;
            }
            let Event::Key(k) = event::read()? else { continue };
            if k.kind != KeyEventKind::Press {
                continue;
            }
            match self.key(k) {
                Some(Do::Quit) => return Ok(None),
                Some(Do::Open(n)) => return Ok(Some(n)),
                Some(Do::Refresh) => self.refresh(term)?,
                None => {}
            }
        }
    }

    fn refresh(&mut self, term: &mut DefaultTerminal) -> Result<()> {
        self.msg = "loading PRs…".into();
        term.draw(|f| self.draw(f))?;
        let keep = self.shown().get(self.sel).map(|p| p.number);
        match gh::list() {
            Ok(prs) => {
                let (tx, rx) = mpsc::channel();
                std::thread::spawn(move || tx.send(gh::list_checks()));
                self.checks = Some(rx);
                self.msg = if prs.is_empty() { "no open PRs".into() } else { String::new() };
                self.prs = prs;
                self.sel = keep.and_then(|n| self.shown().iter().position(|p| p.number == n)).unwrap_or(0);
            }
            Err(e) => self.msg = format!("{e:#}"),
        }
        Ok(())
    }

    /// PRs matching every filter term, in the chosen order.
    fn shown(&self) -> Vec<&Listed> {
        let mut prs: Vec<&Listed> = self.prs.iter().filter(|p| self.query.split_whitespace().all(|t| term(p, t))).collect();
        prs.sort_by(|a, b| {
            let o = match self.order {
                Order::Updated => b.updated.cmp(&a.updated), // ISO times sort as text
                Order::Created => b.created.cmp(&a.created),
                Order::Number => b.number.cmp(&a.number),
                Order::Size => (b.additions + b.deletions).cmp(&(a.additions + a.deletions)),
            };
            if self.rev { o.reverse() } else { o }
        });
        prs
    }

    /// Change the order, keeping the selected PR selected.
    fn sort(&mut self, order: Order, rev: bool) {
        let keep = self.shown().get(self.sel).map(|p| p.number);
        (self.order, self.rev) = (order, rev);
        self.sel = keep.and_then(|n| self.shown().iter().position(|p| p.number == n)).unwrap_or(0);
    }

    fn key(&mut self, k: KeyEvent) -> Option<Do> {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && k.code == KeyCode::Char('c') {
            return Some(Do::Quit);
        }
        let n = self.shown().len();
        self.msg.clear();
        if std::mem::take(&mut self.help) {
            return None; // any key closes the help
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
        let half = (self.height() / 2).max(1);
        let g = std::mem::take(&mut self.pending_g);
        match k.code {
            KeyCode::Enter => return self.shown().get(self.sel).map(|p| Do::Open(p.number)),
            KeyCode::Esc if !self.query.is_empty() => (self.query.clear(), self.sel = 0).1,
            KeyCode::Char('q') | KeyCode::Esc => return Some(Do::Quit),
            KeyCode::Char('R') => return Some(Do::Refresh),
            KeyCode::Char('?') => self.help = true,
            KeyCode::Char('/') => self.filtering = true, // edits the current filter
            KeyCode::Char('o') => self.sort(self.order.next(), self.rev),
            KeyCode::Char('O') => self.sort(self.order, !self.rev),
            KeyCode::Char('d') if ctrl => self.sel = (self.sel + half).min(n.saturating_sub(1)),
            KeyCode::Char('u') if ctrl => self.sel = self.sel.saturating_sub(half),
            KeyCode::Char('j') | KeyCode::Down => self.sel = (self.sel + 1).min(n.saturating_sub(1)),
            KeyCode::Char('k') | KeyCode::Up => self.sel = self.sel.saturating_sub(1),
            KeyCode::Char('g') if g => self.sel = 0,
            KeyCode::Char('e') if g => self.sel = n.saturating_sub(1),
            KeyCode::Char('g') => self.pending_g = true,
            _ => {}
        }
        None
    }

    /// Cards on screen (2 rows each, between the header and the status bar).
    fn height(&self) -> usize {
        ratatui::crossterm::terminal::size().map_or(10, |(_, h)| h.saturating_sub(2) as usize / 2)
    }

    fn draw(&mut self, f: &mut Frame) {
        let area = f.area();
        let h = area.height.saturating_sub(2) as usize / 2;
        let sel = self.sel.min(self.shown().len().saturating_sub(1));
        // Keep the selection on screen.
        (self.sel, self.top) = (sel, self.top.min(sel).max((sel + 1).saturating_sub(h)));
        let (ui, shown) = (&self.ui, self.shown());
        f.render_widget(Paragraph::new("").style(ui.base), area);

        let head = Rect { height: 1.min(area.height), ..area };
        let hs = ui.base.patch(ui.header);
        let arrow = if self.rev { '↑' } else { '↓' };
        f.render_widget(Paragraph::new(format!(" open pull requests · by {} {arrow}", self.order.name())).style(hs), head);
        let pos = if shown.is_empty() { "0 / 0 ".into() } else { format!("{} / {} ", sel + 1, shown.len()) };
        f.render_widget(Paragraph::new(pos).alignment(Alignment::Right).style(hs), head);

        let num_w = shown.iter().map(|p| p.number.to_string().len()).max().unwrap_or(1) + 1;
        let width = area.width as usize;
        let lines: Vec<Line> = shown
            .iter()
            .enumerate()
            .skip(self.top)
            .take(h)
            .flat_map(|(i, p)| card(p, ui, i == sel, num_w, width, self.checks.is_some()))
            .collect();
        f.render_widget(Paragraph::new(lines), Rect { y: area.y + 1, height: (h * 2) as u16, ..area });

        let status = Rect { y: area.bottom().saturating_sub(1), height: 1.min(area.height), ..area };
        let bar = ui.base.patch(ui.status);
        let left = match (self.filtering, self.query.is_empty(), self.msg.is_empty()) {
            (true, _, _) => format!(" /{}▏  author: label: base: branch: is:draft review: ci:  -x negates", self.query),
            (_, _, false) => format!(" {}", self.msg),
            (_, false, _) => format!(" filter: {}  (Esc clears)", self.query),
            _ => " Enter review · / filter · o O order · R refresh · ? help · q quit".into(),
        };
        let flags = if self.flags.is_empty() { String::new() } else { format!("opens with {}  ", self.flags) };
        f.render_widget(Paragraph::new(left).style(bar), status);
        f.render_widget(Paragraph::new(flags).alignment(Alignment::Right).style(bar), status);
        if self.help {
            // Like dv's popups: no border, the popup background, a header row.
            let lines: Vec<&str> = HELP.lines().collect();
            let w = (lines.iter().map(|l| l.chars().count()).max().unwrap_or(0) as u16 + 4).min(area.width);
            let h = (lines.len() as u16 + 3).min(area.height);
            let r = Rect { x: (area.width - w) / 2, y: (area.height - h) / 2, width: w, height: h };
            f.render_widget(Clear, r);
            f.render_widget(Block::new().style(ui.base.patch(ui.popup)), r);
            let hs = ui.base.patch(ui.header);
            f.render_widget(Paragraph::new(" dv list keys").style(hs), Rect { height: 1, ..r });
            f.render_widget(Paragraph::new("any key closes ").alignment(Alignment::Right).style(hs), Rect { height: 1, ..r });
            let body: Vec<Line> = lines
                .iter()
                .map(|l| {
                    let (k, d) = l.split_at(l.char_indices().nth(23).map_or(l.len(), |(i, _)| i));
                    Line::from(vec![Span::styled(k.to_owned(), ui.key), Span::raw(d.to_owned())])
                })
                .collect();
            f.render_widget(Paragraph::new(body), Rect { x: r.x + 1, y: r.y + 2, width: r.width.saturating_sub(2), height: h.saturating_sub(3) });
        }
    }
}

enum Do {
    Quit,
    Open(u64),
    Refresh,
}

/// A PR as two lines: number, title, draft badge and age; then who, CI, review, size, labels
/// and branches, most useful first so a narrow screen cuts the least useful.
fn card(p: &Listed, ui: &Ui, selected: bool, num_w: usize, width: usize, checks_loading: bool) -> [Line<'static>; 2] {
    let row = if selected { ui.base.patch(ui.menu_selected) } else { ui.base };
    let dim = row.patch(ui.stale);
    let sp = |t: String, st| Span::styled(t, st);
    let age = format!("{} ", gh::ago(&p.updated));
    let draft = if p.draft { "  DRAFT " } else { "" };
    let room = width.saturating_sub(num_w + 3 + draft.len() + age.len()).max(10);
    let mut top = vec![
        sp(format!(" {:>num_w$} ", format!("#{}", p.number)), row.patch(ui.key)),
        sp(cut(&p.title, room), row),
    ];
    if p.draft {
        top.extend([sp("  ".into(), row), sp(" DRAFT ".into(), crate::view::chip("6e7681"))]);
    }
    let used: usize = top.iter().map(Span::width).sum();
    top.push(sp(format!("{age:>w$}", w = width.saturating_sub(used)), dim));

    let sep = || sp(" · ".into(), dim);
    let mut bottom = vec![sp(" ".repeat(num_w + 2), row), sp(format!("@{}", p.author), dim)];
    match p.checks {
        Some((0, 0, 0)) => {}
        Some((pass, fail, pending)) => {
            let counts = [(pass, '✓', ui.plus), (fail, '✗', ui.error), (pending, '●', ui.warning)];
            for (k, &(n, mark, st)) in counts.iter().filter(|c| c.0 > 0).enumerate() {
                bottom.extend([sp(if k == 0 { " · " } else { " " }.into(), dim), sp(format!("{mark}{n}"), row.patch(st))]);
            }
        }
        None if checks_loading => bottom.extend([sep(), sp("checks…".into(), dim)]),
        None => {}
    }
    match p.decision.as_str() {
        "APPROVED" => bottom.extend([sep(), sp("✓ approved".into(), row.patch(ui.plus))]),
        "CHANGES_REQUESTED" => bottom.extend([sep(), sp("✗ changes requested".into(), row.patch(ui.error))]),
        _ => {}
    }
    bottom.extend([
        sep(),
        sp(format!("+{}", p.additions), row.patch(ui.plus)),
        sp(" ".into(), row),
        sp(format!("−{}", p.deletions), row.patch(ui.minus)),
        sp(format!(" in {} file{}", p.files, if p.files == 1 { "" } else { "s" }), dim),
    ]);
    for (name, color) in &p.labels {
        bottom.extend([sp(" ".into(), row), sp(format!(" {name} "), crate::view::chip(color))]);
    }
    bottom.extend([sep(), sp(format!("{} ← {}", p.base, cut(&p.branch, 32)), dim)]);
    [Line::from(top).style(row), Line::from(bottom).style(row)]
}

/// `s` cut to `n` chars, ending in `…` if it was longer.
fn cut(s: &str, n: usize) -> String {
    match s.chars().count() > n {
        true => format!("{}…", s.chars().take(n.saturating_sub(1)).collect::<String>()),
        false => s.to_owned(),
    }
}

#[derive(Clone, Copy)]
enum Order {
    Updated,
    Created,
    Number,
    Size,
}

impl Order {
    fn next(self) -> Order {
        match self {
            Order::Updated => Order::Created,
            Order::Created => Order::Number,
            Order::Number => Order::Size,
            Order::Size => Order::Updated,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Order::Updated => "updated",
            Order::Created => "created",
            Order::Number => "number",
            Order::Size => "size",
        }
    }
}

/// One filter term, GitHub-search style: `author:` `label:` `base:` `branch:` (substring),
/// `is:draft`, `review:approved|changes|required|none`, `ci:pass|fail|pending|none`, else text
/// in the number, title, author, branch or labels. `-` negates. Smart case.
fn term(p: &Listed, t: &str) -> bool {
    let (neg, t) = match t.strip_prefix('-') {
        Some(rest) if !rest.is_empty() => (true, rest),
        _ => (false, t),
    };
    let fold = |s: &str| if t.chars().any(char::is_uppercase) { s.to_owned() } else { s.to_lowercase() };
    let has = |field: &str, v: &str| fold(field).contains(v);
    let labels = || p.labels.iter().map(|l| l.0.as_str()).collect::<Vec<_>>().join(" ");
    let hit = match t.split_once(':') {
        Some(("author", v)) => has(&p.author, v),
        Some(("label", v)) => p.labels.iter().any(|l| has(&l.0, v)),
        Some(("base", v)) => has(&p.base, v),
        Some(("branch", v)) => has(&p.branch, v),
        Some(("is", "draft")) => p.draft,
        Some(("review", v)) => match p.decision.as_str() {
            "APPROVED" => "approved".starts_with(v),
            "CHANGES_REQUESTED" => "changes".starts_with(v),
            "REVIEW_REQUIRED" => "required".starts_with(v),
            _ => v == "none",
        },
        // Unknown until the checks arrive; then a PR can match several (fail and pending).
        Some(("ci", v)) => match p.checks {
            Some((0, 0, 0)) | None => v == "none",
            Some((pass, fail, pending)) => {
                ("pass".starts_with(v) && pass > 0 && fail == 0 && pending == 0)
                    || ("fail".starts_with(v) && fail > 0)
                    || ("pending".starts_with(v) && pending > 0)
            }
        },
        _ => has(&format!("#{} {} @{} {} {}", p.number, p.title, p.author, p.branch, labels()), t),
    };
    hit != neg
}

#[test]
fn filter_and_keys() {
    let pr = |number, title: &str, author: &str, updated: &str| Listed {
        number,
        title: title.into(),
        author: author.into(),
        base: "master".into(),
        branch: "b".into(),
        draft: false,
        created: String::new(),
        updated: updated.into(),
        additions: number,
        deletions: 0,
        files: 1,
        decision: String::new(),
        labels: vec![],
        checks: None,
    };
    let mut l = List::new("-wb".into());
    l.prs = vec![pr(1, "Fix CI", "ann", "2026-03"), pr(22, "Add docs", "bo", "2026-01"), pr(333, "fix fees", "cy", "2026-02")];
    (l.prs[1].draft, l.prs[1].decision, l.prs[1].labels) = (true, "APPROVED".into(), vec![("docs".into(), "0075ca".into())]);
    l.prs[2].checks = Some((3, 1, 0));
    let k = |l: &mut List, c| l.key(KeyEvent::new(c, KeyModifiers::NONE));
    let shown = |l: &List| l.shown().iter().map(|p| p.number).collect::<Vec<_>>();
    let filter = |l: &mut List, q: &str| (l.query = q.into(), shown(l)).1;
    assert_eq!(shown(&l), [1, 333, 22]); // most recently updated first
    assert_eq!(filter(&mut l, "fix"), [1, 333]); // smart case
    assert_eq!(filter(&mut l, "Fix"), [1]);
    assert_eq!(filter(&mut l, "is:draft"), [22]);
    assert_eq!(filter(&mut l, "-is:draft review:none"), [1, 333]);
    assert_eq!(filter(&mut l, "review:appr label:doc"), [22]);
    assert_eq!(filter(&mut l, "ci:fail"), [333]);
    assert_eq!(filter(&mut l, "ci:none -author:an"), [22]);
    assert_eq!(filter(&mut l, "docs"), [22]); // labels count as text too
    l.query.clear();
    // `/` types a filter; Enter keeps it, Esc (out of filter mode) clears it.
    k(&mut l, KeyCode::Char('/'));
    "fix".chars().for_each(|c| _ = k(&mut l, KeyCode::Char(c)));
    k(&mut l, KeyCode::Enter);
    assert_eq!(shown(&l), [1, 333]);
    k(&mut l, KeyCode::Char('j'));
    assert!(matches!(k(&mut l, KeyCode::Enter), Some(Do::Open(333))));
    k(&mut l, KeyCode::Esc); // clears the filter, doesn't quit
    assert_eq!(shown(&l), [1, 333, 22]);
    // Ordering keeps the selected PR selected.
    k(&mut l, KeyCode::Char('j')); // on 333
    k(&mut l, KeyCode::Char('o')); // created: all equal, so stable
    k(&mut l, KeyCode::Char('o')); // number
    assert_eq!(shown(&l), [333, 22, 1]);
    k(&mut l, KeyCode::Char('O'));
    assert_eq!(shown(&l), [1, 22, 333]);
    assert_eq!(l.shown()[l.sel].number, 333);
    // `?` shows the help; the next key only closes it.
    k(&mut l, KeyCode::Char('?'));
    assert!(l.help);
    assert!(k(&mut l, KeyCode::Char('q')).is_none() && !l.help);
    assert!(matches!(k(&mut l, KeyCode::Char('q')), Some(Do::Quit)));
}
