//! An issue in dv (Enter in `dv issues`): its text and conversation, Claude's triage under them,
//! and what to do about it: comment (`C`), apply Claude's labels (`L L`) or title (`T T`),
//! close it with Claude's comment (`X X`, after your editor), have Claude draft a PR fixing it
//! (`N N`), or triage it again (`a a`).
use super::*;
use serde_json::Value;

/// A line's links: their columns, and where they go.
type Links = Vec<(std::ops::Range<usize>, String)>;

/// `links`, `by` columns to the right.
fn shift(links: Links, by: usize) -> Links {
    links.into_iter().map(|(r, u)| (r.start + by..r.end + by, u)).collect()
}

/// What a key in the view came to, for the list to do: text to copy, a link to open.
pub enum Did {
    Nothing,
    Copy(String),
    Open(String),
    /// Not the view's key.
    Not,
}

/// An issue as dv shows it: as GitHub has it, and Claude's triage (all of it: the summary,
/// labels, title and closing comment too, not just what its card shows).
pub struct IssueView {
    pub number: u64,
    title: String,
    pub author: String,
    url: String,
    body: String,
    labels: Vec<(String, String)>,
    /// (who, when, what), oldest first.
    comments: Vec<(String, String, String)>,
    triage: Value,
    scroll: usize,
    /// The lines as last drawn, at what width, where the pane starts (side by side), the columns
    /// each line's text is in (the description's, the pane's: not margins, bubbles' sides), and
    /// how many lines show.
    lines: Vec<Line<'static>>,
    width: usize,
    pane_x: Option<usize>,
    text_at: Vec<[std::ops::Range<usize>; 2]>,
    links_at: Vec<Links>,
    shown: usize,
    /// The cursor (line, column), as Helix's: `v` selects from where it was (`sel` then
    /// follows it), `x` the line.
    cur: (usize, usize),
    select: bool,
    /// The selection is `x`'s, whole lines: `x` again takes the next.
    lines_sel: bool,
    /// How many lines the description takes (one column: the pane's after them).
    desc_rows: usize,
    /// Code blocks and tables scrolled sideways (the mouse's), the description's and the
    /// pane's, and how far each could go (as last drawn).
    code_left: [usize; 2],
    code_over: [usize; 2],
    /// Whether you may label it (triage access and up): Claude's labels are suggestions else.
    can_triage: bool,
    /// Text dragged over (line, column), from where the drag began to where it is.
    sel: Option<((usize, usize), (usize, usize))>,
    hl: crate::highlight::Highlighter,
}

impl IssueView {
    /// Issue `n` from GitHub, with its triage from `dir` (the reviews' directory).
    pub fn load(n: u64, dir: Option<&std::path::Path>, can_triage: bool) -> Result<IssueView> {
        let v: Value = gh::gh_json(&["issue", "view", &n.to_string(), "--json", "number,title,author,url,body,labels,comments"])?;
        let s = |v: &Value| v.as_str().unwrap_or("").to_owned();
        let triage = dir
            .and_then(|d| std::fs::read_to_string(d.join(format!("issue-{n}.json"))).ok())
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or(Value::Null);
        Ok(IssueView {
            number: n,
            title: s(&v["title"]),
            author: s(&v["author"]["login"]),
            url: s(&v["url"]),
            body: s(&v["body"]).replace("\r\n", "\n"),
            labels: v["labels"].as_array().into_iter().flatten().map(|l| (s(&l["name"]), s(&l["color"]))).collect(),
            comments: v["comments"].as_array().into_iter().flatten().map(|c| (s(&c["author"]["login"]), s(&c["createdAt"]), s(&c["body"]).replace("\r\n", "\n"))).collect(),
            triage,
            scroll: 0,
            lines: vec![],
            width: 0,
            pane_x: None,
            text_at: vec![],
            links_at: vec![],
            shown: 0,
            cur: (0, 2),
            select: false,
            lines_sel: false,
            desc_rows: 0,
            code_left: [0; 2],
            code_over: [0; 2],
            can_triage,
            sel: None,
            hl: crate::highlight::Highlighter::new(&Theme::load()),
        })
    }

    /// Its title bar, as the PR view's: `Issue #N · title`, who opened it, its labels.
    pub fn title_bar(&self, f: &mut Frame, ui: &Ui, r: Rect) {
        let hs = ui.base.patch(Style { bg: ui.bar, ..ui.header });
        let dim = hs.remove_modifier(ratatui::style::Modifier::BOLD).patch(Style { fg: ui.linenr.fg, ..Style::new() });
        let mut spans = vec![Span::styled(format!(" Issue #{} · {}", self.number, self.title), hs.add_modifier(ratatui::style::Modifier::BOLD)), Span::styled(format!("  @{}", self.author), dim)];
        for (name, color) in &self.labels {
            spans.extend([Span::styled(" ", hs), Span::styled(format!(" {name} "), crate::view::chip(color))]);
        }
        f.render_widget(Paragraph::new(Line::from(spans)).style(hs), r);
    }

    /// Claude's triage's `k`, trimmed (empty: not said).
    fn t(&self, k: &str) -> String {
        self.triage[k].as_str().unwrap_or("").trim().to_owned()
    }

    /// The labels Claude's triage would add (those it doesn't have yet).
    pub fn new_labels(&self) -> Vec<String> {
        let has = |l: &str| self.labels.iter().any(|(n, _)| n.eq_ignore_ascii_case(l));
        self.triage["labels"].as_array().into_iter().flatten().filter_map(Value::as_str).filter(|l| !has(l)).map(str::to_owned).collect()
    }

    /// Claude's better title, if it has one (and it isn't the title already).
    pub fn new_title(&self) -> Option<String> {
        Some(self.t("title")).filter(|t| !t.is_empty() && *t != self.title)
    }

    /// Claude's closing comment, if it would close it.
    pub fn closing(&self) -> Option<String> {
        (!self.t("close").is_empty()).then(|| self.t("closing"))
    }

    /// What a PR fixing it is for Claude to do (`N N`): the issue, and what its triage found.
    pub fn fix_task(&self) -> String {
        let summary = self.t("summary");
        let mut task = format!("Fix #{}: {}", self.number, self.title);
        if !summary.is_empty() {
            task += &format!("\n\nFrom triaging it:\n{summary}");
        }
        task
    }

    /// The issue as the PR view shows a PR: its description on the left (as the PR's text), and
    /// beside it a pane (a third of the window, 60–80 columns) with Claude's triage in its box,
    /// then the conversation, a bubble a comment. Too narrow for two: one column, the pane's
    /// boxes after the description.
    fn render(&mut self, ui: &Ui, width: usize) {
        let pane_w = (width / 3).clamp(60, 80);
        let two = width >= pane_w + 60;
        let (left_w, right_w) = match two {
            true => (width - pane_w - 1, pane_w),
            false => (width, width),
        };
        // (`#123` linked to the repo's, as the PR view's.)
        let repo = self.url.rsplit_once("/issues/").map(|(r, _)| r.to_owned());
        // (A code block's line in its background to the width given, as hover popups have it.)
        let code_bg = ui.md.code_bg;
        // (Code and tables scrolled sideways, as far as the side's are; how far they'd go.)
        let over = std::cell::Cell::new([0; 2]);
        // (`seen`: the columns shown, the description's 2 more than it's wrapped at.)
        let md = |text: &str, w: usize, side: usize| -> Vec<(Line<'static>, Links)> {
            let seen = if side == 0 { w + 2 } else { w };
            let lines = crate::md::render(text, w, &ui.md, &self.hl, repo.as_deref());
            // Each block (code, a table: its lines in a run) by as much of the side's scroll as
            // it's wider than shown: one that fits stays put.
            let mut by = vec![0; lines.len()];
            let mut i = 0;
            while i < lines.len() {
                let n = lines[i..].iter().take_while(|l| l.nowrap).count().max(1);
                let wide = lines[i..i + n].iter().filter(|l| l.nowrap).map(|l| Span::raw(l.text.as_str()).width()).max().unwrap_or(0);
                let o = wide.saturating_sub(seen);
                let mut all = over.get();
                all[side] = all[side].max(o);
                over.set(all);
                by[i..i + n].fill(self.code_left[side].min(o));
                i += n;
            }
            lines
                .into_iter()
                .zip(by)
                .map(|(l, by)| {
                    let code = code_bg.filter(|&bg| l.nowrap && l.styles.last().copied().flatten().and_then(|s| s.bg) == Some(bg));
                    // (Its links' columns, as drawn: chars to columns, less what's scrolled.)
                    let col = |c: usize| Span::raw(l.text.chars().take(c).collect::<String>()).width().saturating_sub(by);
                    let links: Links = l.links.iter().map(|(s, e, u)| (col(*s)..col(*e).min(seen), u.clone())).filter(|(r, _)| !r.is_empty()).collect();
                    let wide = Span::raw(l.text.as_str()).width();
                    let nowrap = l.nowrap;
                    let mut line = l.into_line();
                    line.spans = crate::view::skip_cols(line.spans, by);
                    // Cut where it's shown, `‹` / `›` where there's more to scroll to.
                    if nowrap && wide > seen {
                        let more = Style::new().fg(ui.linenr.fg.unwrap_or(ratatui::style::Color::DarkGray));
                        if wide.saturating_sub(by) > seen {
                            line.spans = super::card::cut_spans(std::mem::take(&mut line.spans), seen.saturating_sub(1));
                            line.spans.push(Span::styled("›", more));
                        }
                        if by > 0 && !line.spans.is_empty() {
                            // (Over a wide character: a space after, the rest where it was.)
                            let first = line.spans.remove(0);
                            let pad = " ".repeat(first.width().saturating_sub(1));
                            line.spans.splice(0..0, [Span::styled("‹", first.style.patch(more)), Span::styled(pad, first.style)]);
                        }
                    }
                    if let Some(bg) = code {
                        line.spans.push(Span::styled(" ".repeat(w.saturating_sub(line.width())), Style::new().bg(bg)));
                    }
                    (line, links)
                })
                .collect()
        };
        let dim = ui.stale;
        let border = ui.linenr;
        // A bubble: `╭─ title ──╮`, its lines between `│ … │`, `╰──╯`, `w` wide (each with the
        // columns of its text: none on its frame).
        let bubble = |title: Vec<Span<'static>>, body: Vec<(Line<'static>, Links)>, w: usize| -> Vec<(Line<'static>, std::ops::Range<usize>, Links)> {
            let tw: usize = title.iter().map(Span::width).sum();
            let mut top = vec![Span::styled("╭─ ", border)];
            top.extend(title);
            top.push(Span::styled(format!(" {}╮", "─".repeat(w.saturating_sub(tw + 5))), border));
            let mut out = vec![(Line::from(top), 0..0, vec![])];
            for (l, links) in body {
                // (A code block or table line wider than the bubble: cut at its edge.)
                let mut v = vec![Span::styled("│ ", border)];
                v.extend(super::card::fit(l.spans, w.saturating_sub(4), Style::new()));
                v.push(Span::styled(" │", border));
                out.push((Line::from(v), 2..w.saturating_sub(2), shift(links, 2)));
            }
            out.push((Line::from(Span::styled(format!("╰{}╯", "─".repeat(w.saturating_sub(2))), border)), 0..0, vec![]));
            out
        };
        // The description, as the PR view's text.
        let url_w = Span::raw(self.url.as_str()).width();
        let mut left: Vec<(Line, Links)> = vec![
            (Line::from(Span::styled(format!("  {}", self.url), dim)), vec![(2..2 + url_w, self.url.clone())]),
            (Line::default(), vec![]),
            (Line::from(Span::styled("  description", ui.heading.add_modifier(ratatui::style::Modifier::BOLD))), vec![]),
        ];
        let body = if self.body.trim().is_empty() { "_(no description)_" } else { &self.body };
        left.extend(md(body, left_w.saturating_sub(4).max(20), 0).into_iter().map(|(mut l, links)| {
            l.spans.insert(0, Span::raw("  "));
            (l, shift(links, 2))
        }));
        // The pane: Claude's triage, then the conversation.
        let inner = right_w.saturating_sub(4).max(20);
        let claude = |text: String| vec![Span::styled("◆ ", ui.key), Span::styled(text, ui.thread)];
        let mut right = vec![];
        // Triaged (its kind or summary said: a closing comment alone isn't), or not yet; what's
        // said of closing it shows either way.
        let triaged = !self.t("kind").is_empty() || !self.t("summary").is_empty();
        {
            let title = ["claude's triage".to_owned(), self.t("kind"), self.t("priority")].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
            let mut b = match triaged {
                true => md(&self.t("summary"), inner, 1),
                false => vec![(Line::from(Span::styled("none yet: a a triages it", dim)), vec![])],
            };
            {
                // (Wrapped as markdown: a long `next` stays in the bubble.)
                let mut said = |k: &str, v: String| {
                    if !v.is_empty() {
                        b.push((Line::default(), vec![]));
                        b.extend(md(&format!("**{k}:** {v}"), inner, 1));
                    }
                };
                said("next", self.t("next"));
                said(if self.can_triage { "labels to add (L L)" } else { "labels to suggest (labelling takes triage access)" }, self.new_labels().join(", "));
                said("title (T T)", self.new_title().unwrap_or_default());
                said("close (X X)", self.t("close"));
                if let Some(c) = self.closing().filter(|c| !c.is_empty()) {
                    b.extend([(Line::default(), vec![]), (Line::from(Span::styled("closing comment:", ui.key)), vec![])]);
                    b.extend(md(&c, inner, 1));
                }
            }
            right.extend(bubble(claude(title), b, right_w));
        }
        right.push((Line::default(), 0..0, vec![]));
        right.push((Line::from(Span::styled(format!("activity ({})", self.comments.len()), ui.heading)), 0..right_w, vec![]));
        for (who, when, body) in &self.comments {
            let mut title = vec![Span::styled(format!("@{who}"), ui.thread)];
            if !when.is_empty() {
                title.push(Span::styled(format!(" · {}", gh::ago(when)), dim));
            }
            right.extend(bubble(title, md(body, inner, 1), right_w));
        }
        // Side by side, a dim line between; or one after the other.
        // (The description's text: after its margin.)
        let desc = 2..left_w;
        self.desc_rows = left.len();
        // Each line, the columns of its text on each side, its links.
        type Row = (Line<'static>, [std::ops::Range<usize>; 2], Links);
        let rows: Vec<Row> = match two {
            true => (0..left.len().max(right.len()))
                .map(|k| {
                    // (The description cut at its column: a wide code line doesn't push the pane.)
                    let (dl, mut links) = left.get(k).cloned().unwrap_or_default();
                    links.retain(|(r, _)| r.start < left_w);
                    let mut l = Line::from(super::card::fit(dl.spans, left_w, Style::new()));
                    l.spans.push(Span::styled("│", border));
                    let (r, at, rl) = right.get(k).cloned().unwrap_or_default();
                    l.spans.extend(r.spans);
                    links.extend(shift(rl, left_w + 1));
                    (l, [if k < left.len() { desc.clone() } else { 0..0 }, at.start + left_w + 1..at.end + left_w + 1], links)
                })
                .collect(),
            false => left
                .into_iter()
                .map(|(l, links)| (l, [desc.clone(), 0..0], links))
                .chain([(Line::default(), [0..0, 0..0], vec![])])
                .chain(right.into_iter().map(|(l, at, links)| (l, [at, 0..0], links)))
                .collect(),
        };
        let (mut out, mut text_at, mut links_at) = (vec![], vec![], vec![]);
        for (l, t, k) in rows {
            out.push(l);
            text_at.push(t);
            links_at.push(k);
        }
        (self.lines, self.width, self.pane_x, self.text_at, self.links_at, self.sel) = (out, width, two.then_some(left_w + 1), text_at, links_at, None);
        (self.select, self.lines_sel) = (false, false); // (what it was over may have moved)
        self.code_over = over.get();
    }

    /// How it closes, as its triage says (`--reason` of `gh issue close`, and for a duplicate,
    /// of what); not said, from its words: "fixed …" completed, else not planned.
    pub fn close_reason(&self) -> (String, Option<u64>) {
        let dup = self.triage["duplicate_of"].as_u64();
        match self.t("reason").as_str() {
            r @ ("completed" | "not planned") => (r.to_owned(), None),
            "duplicate" if dup.is_some() => ("duplicate".into(), dup),
            _ if self.t("close").to_lowercase().starts_with("fixed") => ("completed".into(), None),
            _ => ("not planned".into(), None),
        }
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn scroll_at(&self) -> usize {
        self.scroll
    }

    pub fn set_scroll(&mut self, at: usize) {
        self.scroll = at;
    }

    pub fn scroll(&mut self, by: isize, height: usize) {
        let max = self.lines.len().saturating_sub(height);
        self.scroll = (self.scroll as isize + by).clamp(0, max as isize) as usize;
    }

    pub fn draw(&mut self, f: &mut Frame, ui: &Ui, area: Rect) {
        if self.width != area.width as usize || self.lines.is_empty() {
            self.render(ui, area.width as usize);
            // (Fewer lines now, a narrower window wider: not past the end.)
            self.scroll = self.scroll.min(self.lines.len().saturating_sub(area.height as usize));
        }
        self.shown = area.height as usize;
        let body: Vec<Line> = self.lines.iter().skip(self.scroll).take(area.height as usize).cloned().collect();
        f.render_widget(Paragraph::new(body).style(ui.base), area);
        // What's selected (dragged over, `v`, `x`), reversed; the cursor.
        for y in 0..area.height {
            for x in 0..area.width {
                if self.selected(self.scroll + y as usize, x as usize) {
                    f.buffer_mut()[(area.x + x, area.y + y)].modifier.insert(ratatui::style::Modifier::REVERSED);
                }
            }
        }
        let (row, col) = self.cur;
        if (self.scroll..self.scroll + area.height as usize).contains(&row) && col < area.width as usize {
            let style = if self.select { ui.cursor_select } else { ui.cursor };
            f.buffer_mut()[(area.x + col as u16, area.y + (row - self.scroll) as u16)].set_style(style);
        }
    }

    /// The side column `x` is on: 0 the description's, 1 the pane's (side by side).
    fn side_of(&self, x: usize) -> usize {
        self.pane_x.is_some_and(|p| x >= p) as usize
    }

    /// Line `row`'s text columns on the cursor's side.
    fn text(&self, row: usize) -> std::ops::Range<usize> {
        self.text_at.get(row).map_or(0..0, |t| t[self.side_of(self.cur.1)].clone())
    }

    /// Line `row`'s columns: what each shows (a wide character's second: empty).
    fn cells(&self, row: usize) -> Vec<String> {
        let w = self.width as u16;
        let mut buf = ratatui::buffer::Buffer::empty(Rect::new(0, 0, w, 1));
        if let Some(l) = self.lines.get(row) {
            buf.set_line(0, 0, l, w);
        }
        let mut covered = 0;
        (0..self.width)
            .map(|x| {
                let sym = buf[(x as u16, 0)].symbol().to_owned();
                if covered > 0 {
                    covered -= 1;
                    return String::new();
                }
                covered = Span::raw(sym.as_str()).width().saturating_sub(1);
                sym
            })
            .collect()
    }

    /// The cursor to (`row`, `col`), kept in that line's text on its side (or the side's
    /// start, a line with none there), in view; `v` on: the selection follows it.
    fn go(&mut self, row: usize, col: usize) {
        self.cur = self.clamp(row, col, self.side_of(self.cur.1));
        self.lines_sel = false;
        match (self.select, &mut self.sel) {
            (true, Some((_, b))) => *b = self.cur,
            (true, None) => self.sel = Some((self.cur, self.cur)),
            (false, _) => self.sel = None,
        }
        // (In view, three lines from its edges where there are.)
        let row = self.cur.0;
        let h = self.shown.max(1);
        let off = 3.min(h / 2);
        if row < self.scroll + off {
            self.scroll = row.saturating_sub(off);
        } else if row + off >= self.scroll + h {
            self.scroll = (row + off + 1).saturating_sub(h).min(self.lines.len().saturating_sub(h));
        }
    }

    /// (`row`, `col`) as a cursor can be there: in that line's text on `side`, or that side's
    /// columns on a line with none there.
    fn clamp(&self, row: usize, col: usize, side: usize) -> (usize, usize) {
        let row = row.min(self.lines.len().saturating_sub(1));
        let t = self.text_at.get(row).map_or(0..0, |t| t[side].clone());
        // (The side's own columns, text there or not: not past it, nor into the other.)
        let cols = match (side, self.pane_x) {
            (1, Some(p)) => p..self.width.max(p + 1),
            (_, Some(p)) => 0..p.saturating_sub(1).max(1),
            _ => 0..self.width.max(1),
        };
        let col = if t.is_empty() { col.clamp(cols.start, cols.end - 1) } else { col.clamp(t.start, t.end - 1) };
        (row, col)
    }

    /// The next (`fwd`) or previous word's start (`end`: its end) from the cursor, as Helix's
    /// `w`, `b`, `e`: on through the lines of the cursor's side.
    fn word(&self, fwd: bool, end: bool) -> (usize, usize) {
        let class = |s: &str| match s.chars().next() {
            None => 0,
            Some(c) if c.is_whitespace() => 0,
            Some(c) if c.is_alphanumeric() || c == '_' => 1,
            Some(_) => 2,
        };
        // Every column of the side's text, in reading order, with its class.
        let (row0, col0) = self.cur;
        let rows: Box<dyn Iterator<Item = usize>> = if fwd { Box::new(row0..self.lines.len()) } else { Box::new((0..=row0).rev()) };
        let mut seq = vec![];
        for row in rows.take(200) {
            let cells = self.cells(row);
            let t = self.text(row);
            let mut cols: Vec<(usize, usize)> = t.filter(|&x| !cells.get(x).is_some_and(String::is_empty)).map(|x| (row, x)).collect();
            if !fwd {
                cols.reverse();
            }
            let line: Vec<((usize, usize), u8)> = cols.into_iter().map(|p| (p, class(cells.get(p.1).map_or("", String::as_str)))).collect();
            seq.extend(line);
            seq.push(((row, usize::MAX), 0)); // (a line's end: a space)
        }
        let at = seq.iter().position(|(p, _)| *p == (row0, col0)).unwrap_or(0);
        let mut i = at + 1;
        let is_start = |i: usize| seq[i].1 != 0 && (i == 0 || seq[i - 1].1 != seq[i].1);
        let is_end = |i: usize| seq[i].1 != 0 && (i + 1 == seq.len() || seq[i + 1].1 != seq[i].1);
        while i < seq.len() {
            let hit = match (fwd, end) {
                (true, false) => is_start(i),
                (true, true) => is_end(i),
                // (Back: the start of a word is where the reversed run ends.)
                (false, _) => is_end(i),
            };
            if hit {
                return seq[i].0;
            }
            i += 1;
        }
        self.cur
    }

    /// A key in the view, Helix's: `h j k l` (and arrows), `w b e`, `gh gl`, `gg ge`,
    /// `Ctrl-d Ctrl-u`, `Tab` to the other side; `v` selects, `x` the line (again: the next
    /// too), `;` drops it, `y` copies it; Enter opens the link under the cursor.
    pub fn key(&mut self, pending: Option<char>, k: KeyCode, ctrl: bool) -> Did {
        let (row, col) = self.cur;
        let half = (self.shown / 2).max(1);
        match (pending, k) {
            (_, KeyCode::Char('d')) if ctrl => self.go(row + half, col),
            (_, KeyCode::Char('u')) if ctrl => self.go(row.saturating_sub(half), col),
            _ if ctrl => return Did::Not,
            (Some('g'), KeyCode::Char('g')) => self.go(0, col),
            (Some('g'), KeyCode::Char('e')) => self.go(usize::MAX, col),
            (Some('g'), KeyCode::Char('h')) => self.go(row, 0),
            (Some('g'), KeyCode::Char('l')) => {
                // (Its last character, not the space after.)
                let cells = self.cells(row);
                let last = self.text(row).rev().find(|&x| cells.get(x).is_some_and(|c| !c.trim().is_empty()));
                self.go(row, last.unwrap_or(col));
            }
            (_, KeyCode::Char('j') | KeyCode::Down) => self.go(row + 1, col),
            (_, KeyCode::Char('k') | KeyCode::Up) => self.go(row.saturating_sub(1), col),
            (_, KeyCode::Char('h') | KeyCode::Left) => self.go(row, col.saturating_sub(1)),
            (_, KeyCode::Char('l') | KeyCode::Right) => self.go(row, col + 1),
            (_, KeyCode::Char(c @ ('w' | 'b' | 'e'))) => {
                let (r, x) = self.word(c != 'b', c == 'e');
                self.go(r, x);
            }
            (_, KeyCode::Tab) => {
                // The other side, side by side: its text on this line (or the nearest above).
                let Some(p) = self.pane_x else { return Did::Nothing };
                let to = if self.side_of(col) == 0 { p } else { 0 };
                self.cur.1 = to;
                self.go(row, to);
            }
            (_, KeyCode::Char('v')) => {
                (self.select, self.lines_sel) = (!self.select, false);
                self.sel = self.select.then_some((self.cur, self.cur));
            }
            (_, KeyCode::Char('x')) => {
                // The line's text; again (`x`'s lines selected): on to the next with text.
                let (anchor, mut to) = match (self.lines_sel, self.sel) {
                    (true, Some((a, _))) => (a, row + 1),
                    _ => ((row, self.text(row).start), row),
                };
                while to + 1 < self.lines.len() && self.text(to).is_empty() {
                    to += 1;
                }
                let end = self.text(to).end.saturating_sub(1);
                self.select = false;
                self.go(to, end);
                (self.sel, self.lines_sel) = (Some((anchor, self.cur)), true);
            }
            (_, KeyCode::Char(';')) => (self.select, self.sel, self.lines_sel) = (false, None, false),
            (_, KeyCode::Char('y')) => {
                let text = self.selection();
                (self.select, self.sel) = (false, None);
                return match text.trim().is_empty() {
                    true => Did::Nothing,
                    false => Did::Copy(text),
                };
            }
            (_, KeyCode::Enter) => {
                let link = self.links_at.get(row).and_then(|l| l.iter().find(|(r, _)| r.contains(&col)));
                return link.map_or(Did::Nothing, |(_, u)| Did::Open(u.clone()));
            }
            _ => return Did::Not,
        }
        Did::Nothing
    }

    /// Whether there's a selection to drop (`Esc` drops it before leaving).
    pub fn selecting(&self) -> bool {
        self.select || self.sel.is_some()
    }

    pub fn drop_selection(&mut self) {
        (self.select, self.sel) = (false, None);
    }

    /// Whether (line, column) is dragged over: from where the drag began to where it is, as text
    /// runs (whole lines between), in the text of the side it began on.
    fn selected(&self, row: usize, x: usize) -> bool {
        let Some((from, to)) = self.sel else { return false };
        let (a, b) = if from <= to { (from, to) } else { (to, from) };
        let side = self.pane_x.is_some_and(|p| from.1 >= p) as usize;
        self.text_at.get(row).is_some_and(|t| t[side].contains(&x)) && (row, x) >= a && (row, x) <= b
    }

    /// The mouse, at (column, row) of the issue's area (rows past it: its last): a press begins a
    /// drag, a drag moves its end, and the release gives the text dragged over, to copy (a click:
    /// nothing, and nothing kept).
    pub fn mouse(&mut self, kind: ratatui::crossterm::event::MouseEventKind, x: usize, y: usize) -> Option<String> {
        use ratatui::crossterm::event::{MouseButton, MouseEventKind};
        let at = (self.scroll + y.min(self.shown.saturating_sub(1)), x);
        match kind {
            MouseEventKind::Down(MouseButton::Left) => {
                // The cursor there, kept on that side's columns (the view where it is: a drag
                // goes on from the row under the mouse); the drag from the very point.
                self.cur = self.clamp(at.0, at.1, self.side_of(at.1));
                (self.select, self.lines_sel, self.sel) = (false, false, Some((at, at)));
            }
            // Sideways: the code blocks and tables of the side it's over.
            MouseEventKind::ScrollLeft | MouseEventKind::ScrollRight => {
                let side = match self.pane_x {
                    Some(p) => (x >= p) as usize,
                    None => (at.0 >= self.desc_rows) as usize,
                };
                let by = if kind == MouseEventKind::ScrollRight { 8 } else { -8 };
                self.code_left[side] = (self.code_left[side] as isize + by).clamp(0, self.code_over[side] as isize) as usize;
                self.redraw();
            }
            MouseEventKind::Drag(MouseButton::Left) | MouseEventKind::Up(MouseButton::Left) if self.sel.is_some() => {
                let ax = self.sel.map_or(at.1, |((_, x), _)| x);
                if let Some((_, b)) = &mut self.sel {
                    *b = at;
                }
                // (On the side the drag began on, as what it selects is.)
                self.cur = self.clamp(at.0, at.1, self.side_of(ax));
                if kind == MouseEventKind::Drag(MouseButton::Left) {
                    return None;
                }
                self.sel = self.sel.filter(|(a, b)| a != b);
                return self.sel.map(|_| self.selection());
            }
            _ => {}
        }
        None
    }

    /// The text dragged over, line by line.
    fn selection(&self) -> String {
        let Some(((r0, _), (r1, _))) = self.sel.map(|(a, b)| if a <= b { (a, b) } else { (b, a) }) else { return String::new() };
        let out: Vec<String> = (r0..=r1.min(self.lines.len().saturating_sub(1)))
            .map(|row| {
                let text: String = self.cells(row).iter().enumerate().filter(|(x, _)| self.selected(row, *x)).map(|(_, c)| c.as_str()).collect();
                text.trim_end().to_owned()
            })
            .collect();
        out.join("\n").trim_matches('\n').to_owned()
    }

    /// Forget the lines drawn: the issue or its triage changed.
    pub fn redraw(&mut self) {
        self.lines.clear();
    }

    /// Its triage, read again (Claude's just written it).
    pub fn reload_triage(&mut self, dir: Option<&std::path::Path>) {
        if let Some(t) = dir.and_then(|d| std::fs::read_to_string(d.join(format!("issue-{}.json", self.number))).ok()).and_then(|t| serde_json::from_str(&t).ok()) {
            self.triage = t;
            self.redraw();
        }
    }
}

pub(super) const VIEW_HELP: &str = "
MOVE
  h j k l, arrows      move the cursor (Helix's); w b e by word; gh gl the line's start / end
  Ctrl-d Ctrl-u        half a page down / up
  gg ge                top / end
  Tab                  to the other side (the description, the pane)
  v, x                 select from here; the line (again: the next too); ; drops it
  y                    copy what's selected
  Enter                open the link under the cursor
  mouse                wheel scrolls (Shift or sideways: the code blocks); a drag copies
                       the text it's over (one side's)
  q, Esc               back to the issues
  ?                    this help

ON GITHUB
  C                    comment on it (your editor first)
  L L                  add the labels Claude's triage suggests
  T T                  give it the title Claude's triage suggests
  X X                  close it, with Claude's closing comment (your editor first; none
                       yet: Claude drafts one, then X X closes with it)
  o                    open it in the browser

CLAUDE
  a a                  triage it (again)
  N N                  Claude drafts a PR fixing it (unpublished, on dv list's drafts tab)";

#[test]
fn an_issue_and_its_triage() {
    use crate::view::tests::{screen, snapshot};
    let triage = serde_json::json!({"kind": "bug", "priority": "high", "summary": "Still on master: `src/a.rs:12` unwraps.",
        "labels": ["bug", "chain"], "title": "LocalChain panics on reorg past genesis", "next": "review #9",
        "close": "fixed by #9", "closing": "Fixed by #9, thanks!"});
    let mut v = IssueView {
        number: 7, title: "panic".into(), author: "amy".into(), url: "https://github.com/o/r/issues/7".into(),
        body: "It **panics**.".into(), labels: vec![("bug".into(), "d73a4a".into())],
        comments: vec![("bo".into(), String::new(), "Same here.".into())], triage, scroll: 0, lines: vec![], width: 0, pane_x: None, text_at: vec![], links_at: vec![], shown: 0, cur: (0, 2), select: false, lines_sel: false, desc_rows: 0, code_left: [0; 2], code_over: [0; 2], can_triage: true, sel: None,
        hl: crate::highlight::Highlighter::default(),
    };
    assert_eq!(v.new_labels(), ["chain"]); // bug it has
    assert_eq!(v.new_title().as_deref(), Some("LocalChain panics on reorg past genesis"));
    assert_eq!(v.closing().as_deref(), Some("Fixed by #9, thanks!"));
    assert_eq!(v.close_reason(), ("completed".into(), None)); // "fixed by …"
    v.triage["reason"] = "duplicate".into();
    v.triage["duplicate_of"] = 3.into();
    assert_eq!(v.close_reason(), ("duplicate".into(), Some(3)));
    assert!(v.fix_task().starts_with("Fix #7: panic\n\nFrom triaging it:\nStill on master"));
    let ui = Theme::load().ui();
    snapshot("issue_view", &screen(80, 22, |f| {
        let a = f.area();
        v.title_bar(f, &ui, Rect { height: 1, ..a });
        v.draw(f, &ui, Rect { y: 1, height: a.height - 1, ..a });
    }));
    // Wide: the description, and the pane beside it (triage, then the conversation in bubbles).
    v.redraw();
    snapshot("issue_view_wide", &screen(160, 22, |f| {
        let a = f.area();
        v.title_bar(f, &ui, Rect { height: 1, ..a });
        v.draw(f, &ui, Rect { y: 1, height: a.height - 1, ..a });
    }));
    // A drag in the pane copies the comment's text, not the description beside it nor the bubble.
    use ratatui::crossterm::event::{MouseButton, MouseEventKind};
    let row = v.lines.iter().position(|l| l.to_string().contains("Same here.")).unwrap();
    let p = v.pane_x.unwrap();
    v.shown = v.lines.len(); // (all of it on screen: rows are lines)
    v.mouse(MouseEventKind::Down(MouseButton::Left), p, row);
    v.mouse(MouseEventKind::Drag(MouseButton::Left), 159, row);
    assert_eq!(v.mouse(MouseEventKind::Up(MouseButton::Left), 159, row).as_deref(), Some("Same here."));
    // From above the bubble to below it: its text, not its frame.
    v.mouse(MouseEventKind::Down(MouseButton::Left), p, row - 1);
    assert_eq!(v.mouse(MouseEventKind::Up(MouseButton::Left), 159, row + 1).as_deref(), Some("Same here."));
    // The description, from its margin: its text.
    let row = v.lines.iter().position(|l| l.to_string().contains("It panics.")).unwrap();
    v.mouse(MouseEventKind::Down(MouseButton::Left), 0, row);
    assert_eq!(v.mouse(MouseEventKind::Up(MouseButton::Left), p - 2, row).as_deref(), Some("It panics."));
    // The cursor (Helix's): Enter on the link under it opens it; `x` then `y` copies its line;
    // `w` to the next word; `Tab` to the pane.
    v.cur = (0, 2);
    assert!(matches!(v.key(None, KeyCode::Enter, false), Did::Open(u) if u == "https://github.com/o/r/issues/7"));
    let row = v.lines.iter().position(|l| l.to_string().contains("It panics.")).unwrap();
    v.go(row, 0);
    assert_eq!(v.cur, (row, 2)); // (its text, past the margin)
    v.key(None, KeyCode::Char('w'), false);
    assert_eq!(v.cur, (row, 5)); // "panics"
    v.key(None, KeyCode::Char('x'), false);
    assert!(matches!(v.key(None, KeyCode::Char('y'), false), Did::Copy(t) if t == "It panics."));
    v.key(None, KeyCode::Tab, false);
    assert!(v.cur.1 >= v.pane_x.unwrap());
    // `x` again: on through lines with no text (a bubble's frame), not dropping what it had.
    let row = v.lines.iter().position(|l| l.to_string().contains("Same here.")).unwrap();
    v.go(row, 0);
    v.key(None, KeyCode::Char('x'), false);
    v.key(None, KeyCode::Char('x'), false);
    let (a, b) = v.sel.unwrap();
    assert!(a.0 == row && b.0 > row, "{a:?} {b:?}");
    // On a line with no text on its side, `l` stays on that side's columns.
    v.go(row + 1, 0); // (the bubble's frame)
    for _ in 0..300 {
        v.key(None, KeyCode::Char('l'), false);
    }
    assert!(v.cur.1 < v.width && v.side_of(v.cur.1) == 1);
    // Sideways over the description: its code, from its start.
    v.body = format!("```\n{}\n{}\n```\n\n| ab | cd |\n|---|---|\n| 1 | 2 |", "x".repeat(300), "y".repeat(110));
    v.redraw();
    let _ = screen(160, 22, |f| v.draw(f, &ui, f.area()));
    assert!(v.lines.iter().any(|l| l.to_string().contains("  xxx")));
    v.mouse(MouseEventKind::ScrollRight, 2, 0);
    let _ = screen(160, 22, |f| v.draw(f, &ui, f.area()));
    assert_eq!(v.code_left, [8, 0]);
    // (Cut both ways now: `‹` and `›` say there's more.)
    let code = v.lines.iter().map(|l| l.to_string()).find(|l| l.contains("xxx")).unwrap();
    assert!(code.contains("  ‹x") && code.contains("x›"), "{code}");
    // (The table beside it fits: it stays put.)
    assert!(v.lines.iter().any(|l| l.to_string().contains("ab │ cd")), "{:?}", v.lines.iter().map(|l| l.to_string()).collect::<Vec<_>>());
    // (Far right: a shorter line of the block scrolled past its end, no overflow.)
    for _ in 0..30 {
        v.mouse(MouseEventKind::ScrollRight, 2, 0);
        let _ = screen(160, 22, |f| v.draw(f, &ui, f.area()));
    }
    v.code_left = [8, 0];
    v.mouse(MouseEventKind::ScrollLeft, 2, 0);
    v.mouse(MouseEventKind::ScrollLeft, 2, 0);
    assert_eq!(v.code_left, [0, 0]);
    // A click in the margin: the cursor on the text beside it, nothing copied, the view still.
    (v.scroll, v.shown) = (0, 20);
    v.mouse(MouseEventKind::Down(MouseButton::Left), 0, 0); // (the issue's link line, "  https://…")
    assert_eq!((v.mouse(MouseEventKind::Up(MouseButton::Left), 0, 0), v.cur, v.scroll), (None, (0, 2), 0));
    v.mouse(MouseEventKind::Down(MouseButton::Left), 2, 19); // (near the bottom: no scroll)
    assert_eq!(v.scroll, 0);
    // A click: nothing, and nothing left selected.
    v.mouse(MouseEventKind::Down(MouseButton::Left), 2, 0);
    assert_eq!(v.mouse(MouseEventKind::Up(MouseButton::Left), 2, 0), None);
    assert!(v.sel.is_none());
}
