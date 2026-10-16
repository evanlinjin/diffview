//! `dv list`: the repo's open PRs; `Enter` reviews one, and quitting that review comes back here.
use crate::gh::{self, Listed};
use crate::theme::{Theme, Ui};
use anyhow::Result;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Alignment, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::{DefaultTerminal, Frame};

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
    pub msg: String,
}

impl List {
    pub fn new(flags: String) -> List {
        let ui = Theme::load().ui();
        List { prs: vec![], ui, flags, sel: 0, top: 0, query: String::new(), filtering: false, pending_g: false, msg: String::new() }
    }

    /// Show the list until a PR is picked (`Some`) or the user quits (`None`).
    pub fn run(&mut self, term: &mut DefaultTerminal) -> Result<Option<u64>> {
        if self.prs.is_empty() {
            self.refresh(term)?;
        }
        loop {
            term.draw(|f| self.draw(f))?;
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
                self.msg = if prs.is_empty() { "no open PRs".into() } else { String::new() };
                self.prs = prs;
                self.sel = keep.and_then(|n| self.shown().iter().position(|p| p.number == n)).unwrap_or(0);
            }
            Err(e) => self.msg = format!("{e:#}"),
        }
        Ok(())
    }

    /// PRs matching the filter (smart case) over number, title, author and branch.
    fn shown(&self) -> Vec<&Listed> {
        let smart = !self.query.chars().any(char::is_uppercase);
        let fold = |s: String| if smart { s.to_lowercase() } else { s };
        let q = fold(self.query.clone());
        self.prs.iter().filter(|p| fold(label(p)).contains(&q)).collect()
    }

    fn key(&mut self, k: KeyEvent) -> Option<Do> {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && k.code == KeyCode::Char('c') {
            return Some(Do::Quit);
        }
        let n = self.shown().len();
        self.msg.clear();
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
            KeyCode::Char('/') => (self.query.clear(), self.filtering = true).1,
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

    /// List rows on screen (between the header and the status bar), from the last draw.
    fn height(&self) -> usize {
        ratatui::crossterm::terminal::size().map_or(20, |(_, h)| h.saturating_sub(2) as usize)
    }

    fn draw(&mut self, f: &mut Frame) {
        let area = f.area();
        let h = area.height.saturating_sub(2) as usize;
        let sel = self.sel.min(self.shown().len().saturating_sub(1));
        // Keep the selection on screen.
        (self.sel, self.top) = (sel, self.top.min(sel).max((sel + 1).saturating_sub(h)));
        let (ui, shown) = (&self.ui, self.shown());
        f.render_widget(Paragraph::new("").style(ui.base), area);

        let head = Rect { height: 1.min(area.height), ..area };
        let hs = ui.base.patch(ui.header);
        f.render_widget(Paragraph::new(" open pull requests").style(hs), head);
        let pos = if shown.is_empty() { "0 / 0 ".into() } else { format!("{} / {} ", sel + 1, shown.len()) };
        f.render_widget(Paragraph::new(pos).alignment(Alignment::Right).style(hs), head);

        let num_w = shown.iter().map(|p| p.number.to_string().len()).max().unwrap_or(1) + 1;
        let lines: Vec<Line> = shown
            .iter()
            .enumerate()
            .skip(self.top)
            .take(h)
            .map(|(i, p)| {
                let row = if i == sel { ui.base.patch(ui.menu_selected) } else { ui.base };
                let dim = row.patch(ui.stale);
                let meta = format!("  @{} · {} · {}", p.author, cut(&p.branch, 24), gh::ago(&p.updated));
                let draft = if p.draft { " draft" } else { "" };
                // Long titles give way so the author and age stay visible.
                let room = (area.width as usize).saturating_sub(num_w + 2 + meta.chars().count() + draft.len()).max(10);
                let title = cut(&p.title, room);
                Line::from(vec![
                    Span::styled(format!(" {:>num_w$} ", format!("#{}", p.number)), row.patch(ui.key)),
                    Span::styled(title, row),
                    Span::styled(draft, dim),
                    Span::styled(meta, dim),
                ])
                .style(row)
            })
            .collect();
        f.render_widget(Paragraph::new(lines), Rect { y: area.y + 1, height: h as u16, ..area });

        let status = Rect { y: area.bottom().saturating_sub(1), height: 1.min(area.height), ..area };
        let bar = ui.base.patch(ui.status);
        let left = match (self.filtering, self.query.is_empty(), self.msg.is_empty()) {
            (true, _, _) => format!(" /{}", self.query),
            (_, _, false) => format!(" {}", self.msg),
            (_, false, _) => format!(" filter: {}  (Esc clears)", self.query),
            _ => " Enter review · / filter · R refresh · q quit".into(),
        };
        let flags = if self.flags.is_empty() { String::new() } else { format!("opens with {}  ", self.flags) };
        f.render_widget(Paragraph::new(left).style(bar), status);
        f.render_widget(Paragraph::new(flags).alignment(Alignment::Right).style(bar), status);
    }
}

enum Do {
    Quit,
    Open(u64),
    Refresh,
}

/// `s` cut to `n` chars, ending in `…` if it was longer.
fn cut(s: &str, n: usize) -> String {
    match s.chars().count() > n {
        true => format!("{}…", s.chars().take(n.saturating_sub(1)).collect::<String>()),
        false => s.to_owned(),
    }
}

fn label(p: &Listed) -> String {
    format!("#{} {} @{} {}", p.number, p.title, p.author, p.branch)
}

#[test]
fn filter_and_keys() {
    let pr = |number, title: &str, author: &str| Listed {
        number,
        title: title.into(),
        author: author.into(),
        branch: "b".into(),
        draft: false,
        updated: String::new(),
    };
    let mut l = List::new("-wb".into());
    l.prs = vec![pr(1, "Fix CI", "ann"), pr(22, "Add docs", "bo"), pr(333, "fix fees", "cy")];
    let k = |l: &mut List, c| l.key(KeyEvent::new(c, KeyModifiers::NONE));
    k(&mut l, KeyCode::Char('/'));
    "fix".chars().for_each(|c| _ = k(&mut l, KeyCode::Char(c)));
    k(&mut l, KeyCode::Enter); // keeps the filter
    assert_eq!(l.shown().iter().map(|p| p.number).collect::<Vec<_>>(), [1, 333]); // smart case
    k(&mut l, KeyCode::Char('j'));
    assert!(matches!(k(&mut l, KeyCode::Enter), Some(Do::Open(333))));
    k(&mut l, KeyCode::Esc); // clears the filter, doesn't quit
    assert_eq!(l.shown().len(), 3);
    assert!(matches!(k(&mut l, KeyCode::Char('q')), Some(Do::Quit)));
}
