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

impl MdLine {
    /// As a line to draw: a span a char, each in its style.
    pub fn into_line(self) -> ratatui::text::Line<'static> {
        let mut spans: Vec<ratatui::text::Span<'static>> = vec![];
        for (ch, st) in self.text.chars().zip(self.styles.into_iter().chain(std::iter::repeat(None))) {
            // (A combining mark with the char it's on: a span of its own, zero wide, isn't drawn.)
            if ratatui::text::Span::raw(ch.to_string()).width() == 0
                && let Some(l) = spans.last_mut()
            {
                l.content.to_mut().push(ch);
                continue;
            }
            spans.push(ratatui::text::Span::styled(ch.to_string(), st.unwrap_or_default()));
        }
        ratatui::text::Line::from(spans)
    }
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
    /// An HTML block's tag or comment not closed on its line (`<img` then `src=…` on the
    /// next; a PR template's `<!--`), for the next line's event.
    carry: String,
    /// Links HTML opened (`<a href>`), for its `</a>` to close: no others.
    html_links: usize,
    /// A `<summary>`'s ▸ is on the line: its title is the line's first (bold, when it's drawn).
    summary: bool,
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
        carry: String::new(),
        html_links: 0,
        summary: false,
    };
    // (Tabs as the diff draws them, four wide: measured as drawn.)
    let text = text.replace('\t', "    ");
    for ev in Parser::new_ext(&text, opts) {
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
            Event::Code(t) => self.push(&t, Some(self.s.raw)),
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
            // (Block HTML comes a line an event: each its own line.)
            // (Only a ▸ still waiting for its title holds the line.)
            Event::Html(h) => {
                self.html(&h);
                if h.ends_with('\n') && !self.title_to_come() {
                    self.flush();
                }
            }
            Event::InlineHtml(h) => self.html(&h),
            _ => {}
        }
    }

    fn start(&mut self, t: Tag) {
        match t {
            Tag::Heading { level, .. } => {
                // (A summary's title as a heading: on its ▸'s line.)
                if !self.title_to_come() {
                    self.flush();
                }
                self.heading = Some(level);
                self.style.push(self.s.heading.add_modifier(Modifier::BOLD));
            }
            Tag::BlockQuote(kind) => {
                self.flush();
                self.quote += 1;
                // GitHub's alerts (`> [!NOTE]`): their label, as it shows it.
                if let Some(k) = kind {
                    self.push(&format!("{k:?}"), Some(Style::new().add_modifier(Modifier::BOLD)));
                    self.flush();
                }
            }
            Tag::CodeBlock(kind) => {
                self.flush();
                let lang = match kind {
                    CodeBlockKind::Fenced(l) => crate::highlight::fence_lang(&l).to_owned(),
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
                self.push("🖼 ", None);
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
                let len = width(&self.cur);
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
            // (What's unclosed ends with its block: a tag, a comment.)
            TagEnd::HtmlBlock => self.carry.clear(),
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

    /// A `<summary>` open with only its ▸ so far: its title still to come.
    fn title_to_come(&self) -> bool {
        self.summary && self.cur.len() <= "▸ ".chars().count()
    }

    fn open_link(&mut self, url: String) {
        self.urls.push(url);
        self.link.push(self.urls.len() - 1);
        self.style.push(self.s.link);
    }

    /// GitHub HTML in markdown: `<summary>` becomes a "▸" heading line, `<br>` a line break,
    /// `<img>` and `<a href>` links as markdown's are, comments dropped (across lines too);
    /// other tags are dropped and their text kept.
    fn html(&mut self, h: &str) {
        // (A block comes a line an event: a tag or comment not closed on its line waits for
        // the next, `carry`; the block's end lets it go.)
        let h = std::mem::take(&mut self.carry) + h;
        let (mut rest, mut lead) = (h.as_str(), true);
        // (A `<` starts a tag only before a letter, `/` or `!`, as HTML reads it: `a < b` is
        // text.)
        let tag_at = |s: &str| s.match_indices('<').map(|(i, _)| i).find(|&i| s[i + 1..].starts_with(|c: char| c.is_ascii_alphabetic() || c == '/' || c == '!'));
        while let Some(a) = tag_at(rest) {
            self.html_text(&rest[..a], lead);
            lead = false;
            let close = if rest[a..].starts_with("<!--") { "-->" } else { ">" };
            let Some(e) = rest[a + 1..].find(close) else {
                self.carry = rest[a..].to_owned();
                return;
            };
            let end = a + 1 + e + close.len();
            if close == ">" {
                self.html_tag(&rest[a..end]);
            }
            rest = &rest[end..];
        }
        self.html_text(rest, lead);
    }

    /// Text between HTML tags: a line's indentation and end dropped (`lead`: what starts it),
    /// a space between two tags kept.
    fn html_text(&mut self, t: &str, lead: bool) {
        let t = t.trim_matches('\n');
        let t = if lead { t.trim_start() } else { t };
        if !t.is_empty() {
            self.text(t);
        }
    }

    /// One HTML tag: `<img>` a link to the image, `<a href>`…`</a>` a link (only one opened so:
    /// an anchor's `</a>` closes nothing of markdown's), `<summary>`…`</summary>` a "▸" heading
    /// line, `<br>` a line break; others dropped.
    fn html_tag(&mut self, tag: &str) {
        let name = tag.trim_start_matches('<').split(|c: char| c.is_whitespace() || c == '>' || c == '/').find(|s| !s.is_empty()).unwrap_or("").to_lowercase();
        let closing = tag.starts_with("</");
        // (An attribute by its whole name, after any space: `data-src=` isn't `src=`; its value
        // in either quotes, or none.)
        let attr = |k: &str| {
            let at = tag.match_indices(&format!("{k}=")).find(|(i, _)| tag[..*i].ends_with(char::is_whitespace))?.0 + k.len() + 1;
            let v = &tag[at..];
            match v.chars().next()? {
                q @ ('"' | '\'') => v[1..].split(q).next(),
                _ => v.split(|c: char| c.is_whitespace() || c == '>').next(),
            }
            .map(str::to_owned)
        };
        match (name.as_str(), closing) {
            ("img", false) => {
                if let Some(src) = attr("src") {
                    self.open_link(src);
                    self.push(&format!("🖼 {}", attr("alt").unwrap_or_default()), None);
                    self.style.pop();
                    self.link.pop();
                }
            }
            ("a", false) => {
                if let Some(u) = attr("href") {
                    self.open_link(u);
                    self.html_links += 1;
                }
            }
            ("a", true) if self.html_links > 0 => {
                self.html_links -= 1;
                self.style.pop();
                self.link.pop();
            }
            ("summary", false) if !self.summary => {
                self.flush();
                self.push("▸ ", None);
                self.summary = true;
            }
            // (Empty, `<summary></summary>`: its ▸ still.)
            ("summary", true) if self.summary => self.flush(),
            // (In a table's cell, which a line break can't split: a space.)
            ("br", _) if self.table.is_some() => self.push(" ", None),
            ("br", _) => self.flush(),
            _ => {}
        }
    }

    /// Plain text: emoji shortcodes, and (outside links) bare URLs, `#123` and `@user` as links.
    fn text(&mut self, t: &str) {
        let t = emoji(t);
        if !self.link.is_empty() {
            return self.push(&t, None);
        }
        let (mut plain, mut skip, mut prev) = (String::new(), 0, None::<char>);
        for (b, ch) in t.char_indices() {
            if b < skip {
                prev = Some(ch);
                continue;
            }
            let rest = &t[b..];
            let word_start = prev.is_none_or(|p| !(p.is_alphanumeric() || p == '/'));
            let auto = if rest.starts_with("https://") || rest.starts_with("http://") {
                let n = rest.find(|ch: char| ch.is_whitespace() || "<>\"')]".contains(ch)).unwrap_or(rest.len());
                let url = rest[..n].trim_end_matches(['.', ',', ';', ':', '!', '?']);
                Some((url.len(), url.to_owned()))
            } else if word_start && ch == '#' && self.repo.is_some() {
                let n = rest[1..].bytes().take_while(u8::is_ascii_digit).count();
                (n > 0).then(|| (n + 1, format!("{}/issues/{}", self.repo.unwrap_or(""), &rest[1..=n])))
            } else if word_start && ch == '@' {
                let n = rest[1..].bytes().take_while(|b| b.is_ascii_alphanumeric() || *b == b'-').count();
                (n > 0).then(|| (n + 1, format!("https://github.com/{}", &rest[1..=n])))
            } else {
                None
            };
            match auto {
                // (`n`: its length in bytes, from here.)
                Some((n, url)) => {
                    self.push(&std::mem::take(&mut plain), None);
                    self.open_link(url);
                    self.push(&rest[..n], None);
                    self.style.pop();
                    self.link.pop();
                    skip = b + n;
                }
                None => plain.push(ch),
            }
            prev = Some(ch);
        }
        self.push(&plain, None);
    }

    /// Append inline text with the current style stack (plus `extra`), into a table cell if in one.
    fn push(&mut self, t: &str, extra: Option<Style>) {
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
        let mut cells = std::mem::take(&mut self.cur);
        // (A summary's title: the line its ▸ is on, bold; whatever it was written as.)
        if std::mem::take(&mut self.summary) {
            cells.iter_mut().skip("▸ ".chars().count()).for_each(|c| c.st = Some(c.st.unwrap_or_default().add_modifier(Modifier::BOLD)));
        }
        let (first, rest) = (self.prefix(true), self.prefix(false));
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
            // (A word wider than a line: broken by width, not chars: a wide char is two.)
            for chunk in wrap_cells(word.to_vec(), avail).iter() {
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
        // (An item's bullet goes on its first line, whatever's drawn first: text, code, a rule.)
        self.bullet = None;
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

    /// A grid sized to fit: columns shrink (widest first, to `TABLE_COL_MIN`) and cells wrap.
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


/// GitHub's `:shortcode:` emoji (`:rocket:`, `:loudspeaker:`) as the emoji; others stay as text.
fn emoji(t: &str) -> String {
    let mut out = String::new();
    let mut rest = t;
    while let Some(a) = rest.find(':') {
        let after = &rest[a + 1..];
        match after.find(':').map(|b| &after[..b]).and_then(|name| Some((name, emojis::get_by_shortcode(name)?))) {
            Some((name, e)) => {
                out.push_str(&rest[..a]);
                // (Not its variation selector: it's 2 columns in some terminals, 1 in width counts.)
                out.push_str(&e.as_str().replace('\u{fe0f}', ""));
                rest = &after[name.len() + 1..];
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

#[test]
fn github_emoji_shortcodes() {
    let s = Styles::default();
    let text = |md: &str| render(md, 80, &s, &Highlighter::default(), None).into_iter().map(|l| l.text).collect::<Vec<_>>().join("\n");
    assert_eq!(text(":loudspeaker: Have feedback on :snowflake: tests"), "📢 Have feedback on ❄ tests");
    // In a `<details>` summary too; not what isn't one.
    assert!(text("<details><summary>:rocket: New features</summary>\n\nx\n</details>").contains("▸ 🚀 New features"));
    assert_eq!(text("at 12:30:00, :notanemoji:"), "at 12:30:00, :notanemoji:");
}

#[test]
fn github_html_alerts_and_wide_text() {
    let (s, hl) = (Styles::default(), Highlighter::new(&crate::theme::Theme::ansi()));
    let r = |t: &str, w: usize| render(t, w, &s, &hl, None);
    let texts = |l: Vec<MdLine>| l.into_iter().map(|l| l.text).collect::<Vec<_>>();
    // A template's comment, across lines: gone; HTML block lines each their own.
    assert_eq!(texts(r("<!--\nPlease describe\n-->\n\nReal text", 60)), ["Real text"]);
    assert_eq!(texts(r("<p>one</p>\n<p>two</p>", 60)), ["one", "two"]);
    // A pasted screenshot, and an HTML link: links still.
    let img = r(r#"<img width="800" alt="Image" src="https://x.y/i.png">"#, 60);
    assert_eq!((img[0].text.as_str(), img[0].links[0].2.as_str()), ("🖼 Image", "https://x.y/i.png"));
    assert_eq!(r(r#"See <a href="https://x.y">docs</a>"#, 60)[0].links[0].2, "https://x.y");
    // A CJK word wider than the line: broken by width.
    assert!(r(&"中文".repeat(40), 30).iter().all(|l| ratatui::text::Span::raw(l.text.clone()).width() <= 30));
    // A code block as an item's first content: its bullet once.
    assert_eq!(texts(r("- ```\n  a\n  b\n  ```", 60)).iter().filter(|l| l.contains('•')).count(), 1);
    // An alert's label.
    assert!(r("> [!NOTE]\n> Useful", 60)[0].text.contains("Note"));
    // A line's link around an image: both, and closed (not the rest of the text as a link).
    let l = r("<p align=\"center\">\n<a href=\"https://a.b\"><img src=\"https://i.png\" alt=\"logo\"></a>\n</p>\n\nAfter #1", 60);
    assert_eq!(l[0].text, "🖼 logo");
    assert!(l.iter().any(|l| l.text == "After #1" && l.links.is_empty()));
    // An anchor's </a> closes nothing of markdown's link; an attribute by its whole name.
    let l = r(r#"[x <a name="q"></a> y](https://l.k) after"#, 60);
    assert!(l[0].links.iter().all(|k| k.2 == "https://l.k" && k.1 <= 4) && l[0].text.ends_with("y after"));
    assert_eq!(r(r#"<img data-src="bad" src="https://good">"#, 60)[0].links[0].2, "https://good");
    // Spaces between tags; a tag over lines; bare and single-quoted values; a summary's
    // title on its own line.
    assert_eq!(texts(r("<p><b>foo</b> <i>bar</i></p>", 60)), ["foo bar"]);
    assert_eq!(texts(r("<p align=\"center\">\n  <img\n    src=\"https://i\"\n    alt=\"logo\">\n</p>", 60)), ["🖼 logo"]);
    assert_eq!(r("See <a href=https://x>docs</a> here", 60)[0].links[0].2, "https://x");
    assert_eq!(r("See <a href='https://y'>docs</a>", 60)[0].links[0].2, "https://y");
    assert_eq!(texts(r("<details>\n<summary>\nTitle\n</summary>\n\nBody\n</details>", 60))[0], "▸ Title");
    // A `<` that starts no tag is text; a summary left open: its title alone bold (not what
    // follows, nor a summary after it unseen); a title over lines, its first.
    assert_eq!(texts(r("<details>\n<summary>\n\nBody\n</details>\n\nafter\n\n<details><summary>Two</summary>\n\nB2\n</details>", 60)), ["▸ Body", "", "after", "", "▸ Two", "B2"]);
    let l = r("<details>\n<summary>\n\nBody\n</details>\n\nafter", 60);
    assert!(l.iter().find(|l| l.text == "after").is_some_and(|l| l.styles.iter().all(Option::is_none)));
    assert_eq!(texts(r("<details>\n<summary>\nTitle\nmore\n</summary>", 60)), ["▸ Title", "more"]);
    assert_eq!(texts(r("<div>\na < b\nmore text\n</div>", 60)), ["a < b", "more text"]);
    assert_eq!(texts(r("<details><summary>Title\n\nBody\n</details>\n\nafter", 60)), ["▸ Title", "Body", "", "after"]);
    // GitHub's markdown in a summary: a blank line, then its title (bold, a heading).
    assert_eq!(texts(r("<details>\n<summary>\n\n**Bold title**\n\n</summary>\n\nBody\n</details>", 60))[0], "▸ Bold title");
    assert_eq!(texts(r("<details>\n<summary>\n\n### Title\n\n</summary>\n\nBody\n</details>", 60))[0], "▸ Title");
}
