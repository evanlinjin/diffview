//! Render GitHub-flavoured markdown (PR text, comments, hover) to styled, wrapped lines, roughly
//! as github.com shows it: link text without URLs, tables as grids, no syntax markers. Links are
//! kept as char ranges so they can be opened from the keyboard.
use crate::highlight::Highlighter;
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Color, Modifier, Style};

/// Styles for rendered markdown, from the Helix theme's `markup.*` scopes.
#[derive(Clone, Default)]
pub struct Styles {
    pub heading: Style,
    pub link: Style,
    pub raw: Style,
    pub quote: Style,
    pub list: Style,
    pub dim: Style,
    pub code_bg: Option<Color>,
}

/// A rendered line: text, a style per char, and links as (start, end, url) char ranges.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MdLine {
    pub text: String,
    pub styles: Vec<Option<Style>>,
    pub links: Vec<(usize, usize, String)>,
    /// A code block's or a table's line: it doesn't wrap (shown scrolled sideways where it's
    /// too wide).
    pub nowrap: bool,
}

#[derive(Clone)]
struct Cell {
    ch: char,
    st: Option<Style>,
    link: Option<usize>,
}

struct Table {
    rows: Vec<Vec<Vec<Cell>>>,
    head: usize,
}

struct R<'a> {
    width: usize,
    s: &'a Styles,
    hl: &'a Highlighter,
    repo: Option<&'a str>,
    out: Vec<MdLine>,
    urls: Vec<String>,
    cur: Vec<Cell>,
    style: Vec<Style>,
    link: Vec<usize>,
    lists: Vec<Option<u64>>,
    bullet: Option<String>,
    quote: usize,
    code: Option<(String, String)>,
    table: Option<Table>,
    heading: Option<HeadingLevel>,
}

pub fn render(text: &str, width: usize, s: &Styles, hl: &Highlighter, repo: Option<&str>) -> Vec<MdLine> {
    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS | Options::ENABLE_GFM;
    let mut r = R {
        width: width.max(20),
        s,
        hl,
        repo,
        out: vec![],
        urls: vec![],
        cur: vec![],
        style: vec![],
        link: vec![],
        lists: vec![],
        bullet: None,
        quote: 0,
        code: None,
        table: None,
        heading: None,
    };
    for ev in Parser::new_ext(text, opts) {
        r.event(ev);
    }
    r.flush();
    while r.out.last().is_some_and(|l| l.text.is_empty()) {
        r.out.pop();
    }
    if r.out.is_empty() {
        r.out.push(MdLine::default());
    }
    r.out
}

impl R<'_> {
    fn event(&mut self, ev: Event) {
        match ev {
            Event::Start(t) => self.start(t),
            Event::End(t) => self.end(t),
            Event::Text(t) => match &mut self.code {
                Some((_, buf)) => buf.push_str(&t),
                None => self.text(&t),
            },
            Event::Code(t) => self.push(&t, Some(self.s.raw), false),
            // GitHub keeps line breaks in comments and PR bodies (unlike plain CommonMark).
            Event::SoftBreak => self.flush(),
            Event::HardBreak => self.flush(),
            Event::Rule => {
                self.flush();
                let w = self.width.saturating_sub(self.prefix(true).len());
                self.line(self.prefix(true), "─".repeat(w).chars().map(|ch| Cell { ch, st: Some(self.s.dim), link: None }).collect());
                self.blank();
            }
            Event::TaskListMarker(done) => self.bullet = Some(if done { "☑ " } else { "☐ " }.into()),
            Event::Html(h) | Event::InlineHtml(h) => self.html(&h),
            _ => {}
        }
    }

    fn start(&mut self, t: Tag) {
        match t {
            Tag::Heading { level, .. } => {
                self.flush();
                self.heading = Some(level);
                self.style.push(self.s.heading.add_modifier(Modifier::BOLD));
            }
            Tag::BlockQuote(_) => {
                self.flush();
                self.quote += 1;
            }
            Tag::CodeBlock(kind) => {
                self.flush();
                let lang = match kind {
                    CodeBlockKind::Fenced(l) => l.split([' ', ',']).next().unwrap_or("").to_owned(),
                    CodeBlockKind::Indented => String::new(),
                };
                self.code = Some((lang, String::new()));
            }
            Tag::List(start) => {
                self.flush();
                self.lists.push(start);
            }
            Tag::Item => {
                self.flush();
                let b = match self.lists.last_mut() {
                    Some(Some(n)) => {
                        *n += 1;
                        format!("{}. ", *n - 1)
                    }
                    _ => "• ".into(),
                };
                self.bullet = Some(b);
            }
            Tag::Emphasis => self.style.push(Style::new().add_modifier(Modifier::ITALIC)),
            Tag::Strong => self.style.push(Style::new().add_modifier(Modifier::BOLD)),
            Tag::Strikethrough => self.style.push(Style::new().add_modifier(Modifier::CROSSED_OUT)),
            Tag::Link { dest_url, .. } => self.open_link(dest_url.to_string()),
            Tag::Image { dest_url, .. } => {
                self.open_link(dest_url.to_string());
                self.push("🖼 ", None, false);
            }
            Tag::Table(_) => {
                self.flush();
                self.table = Some(Table { rows: vec![], head: 0 });
            }
            Tag::TableHead | Tag::TableRow => {
                if let Some(t) = &mut self.table {
                    t.rows.push(vec![]);
                }
            }
            Tag::TableCell => {
                if let Some(row) = self.table.as_mut().and_then(|t| t.rows.last_mut()) {
                    row.push(vec![]);
                }
            }
            _ => {}
        }
    }

    fn end(&mut self, t: TagEnd) {
        match t {
            TagEnd::Paragraph => {
                self.flush();
                if self.lists.is_empty() {
                    self.blank();
                }
            }
            TagEnd::Heading(level) => {
                self.style.pop();
                let len = self.cur.len();
                self.flush();
                self.heading = None;
                // GitHub underlines the two biggest heading levels.
                if matches!(level, HeadingLevel::H1 | HeadingLevel::H2) {
                    let rule: Vec<Cell> = "─".repeat(len.min(self.width)).chars().map(|ch| Cell { ch, st: Some(self.s.dim), link: None }).collect();
                    self.line(self.prefix(true), rule);
                }
                self.blank();
            }
            TagEnd::BlockQuote(_) => {
                self.flush();
                self.quote -= 1;
                self.blank();
            }
            TagEnd::CodeBlock => {
                let (lang, buf) = self.code.take().unwrap_or_default();
                let colors = self.hl.lang(&lang, buf.trim_end_matches('\n'));
                for (i, l) in buf.trim_end_matches('\n').lines().enumerate() {
                    let bg = self.s.code_bg.map(|c| Style::new().bg(c)).unwrap_or_default();
                    let cells: Vec<Cell> = format!(" {l} ").chars().enumerate().map(|(j, ch)| {
                        let syn = j.checked_sub(1).and_then(|j| colors.get(i)?.get(j).copied().flatten());
                        Cell { ch, st: Some(bg.patch(syn.unwrap_or_default())), link: None }
                    }).collect();
                    self.line(self.prefix(true), cells);
                    if let Some(l) = self.out.last_mut() {
                        l.nowrap = true;
                    }
                }
                self.blank();
            }
            TagEnd::List(_) => {
                self.flush();
                self.lists.pop();
                if self.lists.is_empty() {
                    self.blank();
                }
            }
            TagEnd::Item => self.flush(),
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => _ = self.style.pop(),
            TagEnd::Link | TagEnd::Image => {
                self.style.pop();
                self.link.pop();
            }
            TagEnd::TableHead => {
                if let Some(t) = &mut self.table {
                    t.head = t.rows.len();
                }
            }
            TagEnd::Table => {
                if let Some(t) = self.table.take() {
                    self.table(t);
                }
                self.blank();
            }
            _ => {}
        }
    }

    fn open_link(&mut self, url: String) {
        self.urls.push(url);
        self.link.push(self.urls.len() - 1);
        self.style.push(self.s.link);
    }

    /// GitHub HTML in markdown: `<summary>` becomes a "▸" heading line, `<br>` a line break;
    /// other tags are dropped and their text kept.
    fn html(&mut self, h: &str) {
        let text = strip_tags(h);
        if h.contains("<summary") {
            self.flush();
            self.push(&format!("▸ {}", text.trim()), Some(Style::new().add_modifier(Modifier::BOLD)), false);
            self.flush();
        } else if h.contains("<br") {
            self.flush();
        } else if !text.trim().is_empty() {
            self.text(text.trim());
        }
    }

    /// Plain text: emoji shortcodes, and (outside links) bare URLs, `#123` and `@user` as links.
    fn text(&mut self, t: &str) {
        let t = emoji(t);
        if !self.link.is_empty() {
            return self.push(&t, None, false);
        }
        let c: Vec<char> = t.chars().collect();
        let (mut i, mut plain) = (0, String::new());
        while i < c.len() {
            let rest: String = c[i..].iter().collect();
            let word_start = i == 0 || !(c[i - 1].is_alphanumeric() || c[i - 1] == '/');
            let auto = if rest.starts_with("https://") || rest.starts_with("http://") {
                let n = rest.find(|ch: char| ch.is_whitespace() || "<>\"')]".contains(ch)).unwrap_or(rest.len());
                let url = rest[..n].trim_end_matches(['.', ',', ';', ':', '!', '?']);
                Some((url.chars().count(), url.to_owned()))
            } else if word_start && c[i] == '#' && self.repo.is_some() {
                let n = c[i + 1..].iter().take_while(|ch| ch.is_ascii_digit()).count();
                (n > 0).then(|| (n + 1, format!("{}/issues/{}", self.repo.unwrap_or(""), &rest[1..=n])))
            } else if word_start && c[i] == '@' {
                let n = c[i + 1..].iter().take_while(|ch| ch.is_ascii_alphanumeric() || **ch == '-').count();
                (n > 0).then(|| (n + 1, format!("https://github.com/{}", &rest[1..=n])))
            } else {
                None
            };
            match auto {
                Some((n, url)) => {
                    self.push(&std::mem::take(&mut plain), None, false);
                    self.open_link(url);
                    self.push(&c[i..i + n].iter().collect::<String>(), None, false);
                    self.style.pop();
                    self.link.pop();
                    i += n;
                }
                None => {
                    plain.push(c[i]);
                    i += 1;
                }
            }
        }
        self.push(&plain, None, false);
    }

    /// Append inline text with the current style stack (plus `extra`), into a table cell if in one.
    fn push(&mut self, t: &str, extra: Option<Style>, _: bool) {
        let mut st = self.style.iter().fold(Style::new(), |a, b| a.patch(*b));
        if let Some(e) = extra {
            st = st.patch(e);
        }
        let st = (st != Style::new()).then_some(st);
        let link = self.link.last().copied();
        let cells = t.chars().map(|ch| Cell { ch, st, link });
        match self.table.as_mut().and_then(|t| t.rows.last_mut()).and_then(|r| r.last_mut()) {
            Some(cell) => cell.extend(cells),
            None => self.cur.extend(cells),
        }
    }

    /// Indent for the current container: quote bars, list nesting, and the item's bullet on
    /// its first line (spaces of the same width after).
    fn prefix(&self, first: bool) -> Vec<Cell> {
        let mut p: Vec<Cell> = vec![];
        let add = |p: &mut Vec<Cell>, s: &str, st: Style| p.extend(s.chars().map(|ch| Cell { ch, st: Some(st), link: None }));
        for _ in 0..self.quote {
            add(&mut p, "▎ ", self.s.quote);
        }
        let depth = self.lists.len().saturating_sub(1);
        add(&mut p, &"  ".repeat(depth), Style::new());
        if let Some(b) = &self.bullet {
            match first {
                true => add(&mut p, b, self.s.list),
                false => add(&mut p, &" ".repeat(b.chars().count()), Style::new()),
            }
        } else if !self.lists.is_empty() {
            add(&mut p, "  ", Style::new());
        }
        p
    }

    /// Word-wrap the current inline content under the container prefix.
    fn flush(&mut self) {
        if self.cur.is_empty() {
            return;
        }
        let cells = std::mem::take(&mut self.cur);
        let (first, rest) = (self.prefix(true), self.prefix(false));
        self.bullet = None;
        let quoted = self.quote > 0;
        let avail = self.width.saturating_sub(first.len()).max(10);
        let mut lines: Vec<Vec<Cell>> = vec![vec![]];
        for word in cells.split(|c| c.ch == ' ') {
            let line = lines.last_mut().unwrap();
            let need = width(word) + (!line.is_empty()) as usize;
            if !line.is_empty() && width(line) + need > avail {
                lines.push(vec![]);
            }
            let line = lines.last_mut().unwrap();
            if !line.is_empty() {
                let sep = line.last().map(|c| (c.st, c.link)).filter(|_| !word.is_empty());
                let (st, link) = sep.unwrap_or((None, None));
                line.push(Cell { ch: ' ', st, link: link.filter(|l| word.first().is_some_and(|w| w.link == Some(*l))) });
            }
            for chunk in word.chunks(avail) {
                let line = lines.last_mut().unwrap();
                if !line.is_empty() && width(line) + width(chunk) > avail {
                    lines.push(vec![]);
                }
                lines.last_mut().unwrap().extend_from_slice(chunk);
            }
        }
        for (i, mut l) in lines.into_iter().enumerate() {
            if quoted {
                l.iter_mut().for_each(|c| c.st = Some(self.s.quote.patch(c.st.unwrap_or_default())));
            }
            self.line(if i == 0 { first.clone() } else { rest.clone() }, l);
        }
    }

    fn line(&mut self, prefix: Vec<Cell>, cells: Vec<Cell>) {
        let all: Vec<Cell> = prefix.into_iter().chain(cells).collect();
        let mut links: Vec<(usize, usize, String)> = vec![];
        for (i, c) in all.iter().enumerate() {
            let Some(id) = c.link else { continue };
            match links.last_mut() {
                Some(l) if l.1 == i && l.2 == self.urls[id] => l.1 = i + 1,
                _ => links.push((i, i + 1, self.urls[id].clone())),
            }
        }
        self.out.push(MdLine {
            text: all.iter().map(|c| c.ch).collect(),
            styles: all.iter().map(|c| c.st).collect(),
            links,
            nowrap: false,
        });
    }

    fn blank(&mut self) {
        if self.out.last().is_some_and(|l| !l.text.is_empty()) {
            self.out.push(MdLine::default());
        }
    }

    /// A grid sized to fit: columns shrink (widest first) and long cells end in "…".
    fn table(&mut self, t: Table) {
        let cols = t.rows.iter().map(Vec::len).max().unwrap_or(0);
        if cols == 0 {
            return;
        }
        let mut w: Vec<usize> = (0..cols).map(|c| t.rows.iter().filter_map(|r| r.get(c)).map(|c| width(c)).max().unwrap_or(0).max(1)).collect();
        // Columns narrowed to fit, but not past where their text stops reading: a table wider
        // than that is scrolled sideways instead.
        let avail = self.width.saturating_sub(self.prefix(true).len() + 3 * cols + 1);
        while w.iter().sum::<usize>() > avail {
            let (i, &m) = w.iter().enumerate().max_by_key(|(_, w)| **w).unwrap();
            if m <= TABLE_COL_MIN {
                break;
            }
            w[i] -= 1;
        }
        let dim = |s: &str| s.chars().map(|ch| Cell { ch, st: Some(self.s.dim), link: None }).collect::<Vec<_>>();
        let rule = |l: &str, m: &str, r: &str| {
            let mid: Vec<String> = w.iter().map(|w| "─".repeat(w + 2)).collect();
            dim(&format!("{l}{}{r}", mid.join(m)))
        };
        let start = self.out.len();
        self.line(self.prefix(true), rule("┌", "┬", "┐"));
        for (ri, row) in t.rows.iter().enumerate() {
            // Each cell wrapped to its column, as GitHub does; the row as tall as its tallest.
            let wrapped: Vec<Vec<Vec<Cell>>> = w
                .iter()
                .enumerate()
                .map(|(c, &width)| {
                    let mut cell: Vec<Cell> = row.get(c).cloned().unwrap_or_default();
                    if ri < t.head {
                        cell.iter_mut().for_each(|x| x.st = Some(x.st.unwrap_or_default().add_modifier(Modifier::BOLD)));
                    }
                    wrap_cells(cell, width)
                })
                .collect();
            for k in 0..wrapped.iter().map(Vec::len).max().unwrap_or(1) {
                let mut cells = dim("│");
                for (lines, width) in wrapped.iter().zip(&w) {
                    let cell = lines.get(k).cloned().unwrap_or_default();
                    let pad = width.saturating_sub(self::width(&cell));
                    cells.push(Cell { ch: ' ', st: None, link: None });
                    cells.extend(cell);
                    cells.extend(std::iter::repeat_n(Cell { ch: ' ', st: None, link: None }, pad + 1));
                    cells.extend(dim("│"));
                }
                self.line(self.prefix(true), cells);
            }
            if ri + 1 == t.head {
                self.line(self.prefix(true), rule("├", "┼", "┤"));
            }
        }
        self.line(self.prefix(true), rule("└", "┴", "┘"));
        self.out[start..].iter_mut().for_each(|l| l.nowrap = true);
    }
}

/// `cell` in lines `w` wide, broken after a space where it can be, else mid-word.
fn wrap_cells(cell: Vec<Cell>, w: usize) -> Vec<Vec<Cell>> {
    let mut out = vec![];
    let mut line: Vec<Cell> = vec![];
    for c in cell {
        line.push(c);
        if width(&line) > w {
            let at = match line.iter().rposition(|c| c.ch == ' ') {
                Some(i) if i > 0 => i + 1,
                _ => line.len() - 1,
            };
            let rest = line.split_off(at);
            while line.last().is_some_and(|c| c.ch == ' ') {
                line.pop();
            }
            out.push(std::mem::replace(&mut line, rest));
        }
    }
    out.push(line);
    out
}

/// The narrowest a table's column is made to fit the width (narrower ones keep theirs).
const TABLE_COL_MIN: usize = 20;

/// Display width in terminal columns (emoji are 2 wide).
fn width(cells: &[Cell]) -> usize {
    cells.iter().map(|c| ratatui::text::Span::raw(c.ch.to_string()).width()).sum()
}

fn strip_tags(h: &str) -> String {
    let mut out = String::new();
    let mut tag = false;
    for c in h.chars() {
        match c {
            '<' => tag = true,
            '>' => tag = false,
            _ if !tag => out.push(c),
            _ => {}
        }
    }
    out
}

/// GitHub `:shortcode:` emoji, for the ones bots and people use most; others stay as text.
fn emoji(t: &str) -> String {
    const MAP: &[(&str, &str)] = &[
        ("x", "❌"), ("white_check_mark", "✅"), ("heavy_check_mark", "✔"), ("warning", "⚠"),
        ("rocket", "🚀"), ("tada", "🎉"), ("+1", "👍"), ("thumbsup", "👍"), ("-1", "👎"),
        ("thumbsdown", "👎"), ("eyes", "👀"), ("bug", "🐛"), ("memo", "📝"), ("sparkles", "✨"),
        ("fire", "🔥"), ("heart", "❤"), ("construction", "🚧"), ("information_source", "ℹ"),
        ("question", "❓"), ("exclamation", "❗"), ("red_circle", "🔴"), ("green_circle", "🟢"),
        ("yellow_circle", "🟡"), ("recycle", "♻"), ("zap", "⚡"), ("lock", "🔒"), ("bulb", "💡"),
        ("wrench", "🔧"), ("hammer", "🔨"), ("package", "📦"), ("umbrella", "☂"),
        ("chart_with_upwards_trend", "📈"), ("chart_with_downwards_trend", "📉"), ("robot", "🤖"),
        ("thinking", "🤔"), ("smile", "😄"), ("sweat_smile", "😅"), ("pray", "🙏"), ("ok_hand", "👌"),
        ("muscle", "💪"), ("rotating_light", "🚨"), ("no_entry", "⛔"), ("arrow_up", "⬆"),
        ("arrow_down", "⬇"), ("arrow_right", "➡"), ("star", "⭐"), ("100", "💯"), ("clap", "👏"),
    ];
    let mut out = String::new();
    let mut rest = t;
    while let Some(a) = rest.find(':') {
        let after = &rest[a + 1..];
        match after.find(':').map(|b| &after[..b]).and_then(|name| MAP.iter().find(|(k, _)| *k == name)) {
            Some((k, e)) => {
                out.push_str(&rest[..a]);
                out.push_str(e);
                rest = &after[k.len() + 1..];
            }
            None => {
                out.push_str(&rest[..=a]);
                rest = after;
            }
        }
    }
    out + rest
}

#[test]
fn renders_like_github() {
    use crate::theme::Theme;
    let hl = Highlighter::new(&Theme::ansi());
    let s = Styles::default();
    let r = |t: &str| render(t, 60, &s, &hl, Some("https://github.com/o/r"));
    let text = |t: &str| r(t).into_iter().map(|l| l.text).collect::<Vec<_>>();

    let l = r("See [Codecov](https://codecov.io/x?y=1) :x: and file\\_store.");
    assert_eq!(l[0].text, "See Codecov ❌ and file_store.");
    assert_eq!(l[0].links, [(4, 11, "https://codecov.io/x?y=1".into())]);

    assert_eq!(text("## Changed\n\n- one\n- two\n  - nested"), ["Changed", "───────", "", "• one", "• two", "  • nested"]);
    assert_eq!(text("1. a\n2. b"), ["1. a", "2. b"]);
    assert_eq!(text("one\ntwo"), ["one", "two"]); // line breaks kept, as on GitHub
    assert_eq!(text("- [x] done\n- [ ] todo"), ["☑ done", "☐ todo"]);
    assert_eq!(text("```rust\nfn x() {}\n```"), [" fn x() {} "]);
    assert_eq!(text("<details><summary>More</summary>\n\nbody\n</details>"), ["▸ More", "body"]);

    let t = text("| File | % |\n|---|---|\n| [a.rs](http://x) | 98 |");
    assert_eq!(t, ["┌──────┬────┐", "│ File │ %  │", "├──────┼────┤", "│ a.rs │ 98 │", "└──────┴────┘"]);
    // Emoji are two columns wide: the grid still lines up.
    let e = text("| a | b |\n|---|---|\n| ok :x: | 1 |");
    assert_eq!(e[3], "│ ok ❌ │ 1 │");
    assert_eq!(e[1], "│ a     │ b │");
    // Too wide to fit: narrowed only so far, then left wider than the box (scrolled sideways).
    let long = "x".repeat(40);
    let w = r(&format!("| a | b | c |\n|---|---|---|\n| {long} | {long} | {long} |"));
    assert!(w.iter().all(|l| l.nowrap) && w[3].text.chars().count() > 60 && w[3].text.contains(&"x".repeat(20)));
    // Cells wrap within their column, at spaces where they can.
    let t = text(&format!("| a | b |\n|---|---|\n| {} | ok |", "word ".repeat(12).trim_end()));
    assert_eq!(t.len(), 6, "{t:#?}");
    assert!(t[3].starts_with("│ word word") && t[3].ends_with("│ ok │") && t[4].ends_with("│    │"), "{t:#?}");
    assert!(r("```\nx\n```\n\nplain").iter().map(|l| l.nowrap).eq([true, false, false]));

    let auto = r("Fixes #12, by @amy: https://x.y/z.");
    assert_eq!(auto[0].text, "Fixes #12, by @amy: https://x.y/z.");
    let urls: Vec<&str> = auto[0].links.iter().map(|l| l.2.as_str()).collect();
    assert_eq!(urls, ["https://github.com/o/r/issues/12", "https://github.com/amy", "https://x.y/z"]);
}
