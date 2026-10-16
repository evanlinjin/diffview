//! Drawing: rows, split halves, hunk rules, popups, scrollbars, and the help/hover content.
use super::*;

impl App {
    /// The help text, styled: section headings, keys (separators dimmed), descriptions wrapped
    /// with a hanging indent under the description column, `(notes)` dimmed. With a `query`
    /// (smart case), only matching entries and their headings are kept, matches highlighted.
    pub(super) fn help_lines(&self, width: usize, query: &str) -> Vec<Line<'static>> {
        const KEY_COL: usize = 23; // entries are "  <key, padded to col 23><description>"
        let ui = &self.ui;
        let dim = Style { fg: ui.stale.fg, ..Style::new() };
        let smart = !query.chars().any(char::is_uppercase);
        let fold = |s: &str| if smart { s.to_lowercase() } else { s.to_owned() };
        let q = fold(query);
        // Char positions in `s` covered by a match of the query.
        let hits = |s: &str| -> Vec<bool> {
            let (chars, lower): (Vec<char>, Vec<char>) = (s.chars().collect(), fold(s).chars().collect());
            let qc: Vec<char> = q.chars().collect();
            let mut on = vec![false; chars.len()];
            if !qc.is_empty() && lower.len() == chars.len() {
                for i in 0..=lower.len().saturating_sub(qc.len()) {
                    if lower[i..].starts_with(&qc) {
                        on[i..i + qc.len()].iter_mut().for_each(|x| *x = true);
                    }
                }
            }
            on
        };
        // Sections: heading + entries, keeping only matching entries.
        let mut sections: Vec<(&str, Vec<&str>)> = vec![];
        for l in HELP.trim_matches('\n').lines() {
            match l {
                "" => {}
                h if !h.starts_with(' ') => sections.push((h, vec![])),
                e if q.is_empty() || fold(e).contains(&q) => {
                    if let Some(s) = sections.last_mut() {
                        s.1.push(e);
                    }
                }
                _ => {}
            }
        }
        sections.retain(|s| !s.1.is_empty());
        if sections.is_empty() {
            return vec![Line::styled(format!(" no keys match \"{query}\""), dim)];
        }
        // Two columns when the window fits two unwrapped ones (so there's no scrolling).
        const GAP: usize = 4;
        let natural = sections.iter().flat_map(|s| &s.1).map(|l| l.chars().count()).max().unwrap_or(0) + 1;
        let two = sections.len() > 1 && width >= 2 * natural + GAP;
        let width = if two { natural } else { width.min(97) };
        let mut blocks: Vec<Vec<Line<'static>>> = vec![];
        for (heading, entries) in sections {
            // Styled as a span, not a line: in two columns lines are joined span by span.
            let mut out = vec![Line::from(Span::styled(format!(" {heading}"), ui.heading.add_modifier(ratatui::style::Modifier::BOLD)))];
            for l in entries {
                let (key, desc) = l.split_at(KEY_COL.min(l.len()));
                let mut spans: Vec<Span> = vec![Span::raw("  ")];
                for (c, hit) in key[2..].chars().zip(hits(&key[2..])) {
                    let st = if c == ',' { dim } else { ui.key }; // `/` is a key itself (search)
                    spans.push(Span::styled(c.to_string(), if hit { st.patch(ui.selection) } else { st }));
                }
                let mut depth = 0; // inside "(...)", which may wrap
                for (i, line) in wrap(desc, width.saturating_sub(KEY_COL).max(10)).into_iter().enumerate() {
                    if i > 0 {
                        out.push(Line::from(std::mem::take(&mut spans)));
                        spans.push(Span::raw(" ".repeat(KEY_COL)));
                    }
                    // Dim parenthesised notes, e.g. "(pending review)".
                    for (c, hit) in line.chars().zip(hits(&line)) {
                        depth += (c == '(') as i32;
                        let st = if depth > 0 { dim } else { Style::new() };
                        spans.push(Span::styled(c.to_string(), if hit { st.patch(ui.selection) } else { st }));
                        depth -= (c == ')') as i32;
                    }
                }
                out.push(Line::from(spans));
            }
            blocks.push(out);
        }
        let join = |bs: &[Vec<Line<'static>>]| -> Vec<Line<'static>> {
            bs.iter().enumerate().flat_map(|(n, b)| (n > 0).then(Line::default).into_iter().chain(b.iter().cloned())).collect()
        };
        if !two {
            return join(&blocks);
        }
        // Split between sections where the columns come out most even.
        let len = |bs: &[Vec<Line>]| bs.iter().map(|b| b.len() + 1).sum::<usize>();
        let k = (1..blocks.len()).min_by_key(|&k| len(&blocks[..k]).abs_diff(len(&blocks[k..]))).unwrap_or(1);
        let (left, right) = (join(&blocks[..k]), join(&blocks[k..]));
        (0..left.len().max(right.len()))
            .map(|i| {
                let mut l = left.get(i).cloned().unwrap_or_default();
                let pad = (natural + GAP).saturating_sub(l.width());
                l.spans.push(Span::raw(" ".repeat(pad)));
                l.spans.extend(right.get(i).cloned().unwrap_or_default().spans);
                l
            })
            .collect()
    }

    /// Hover markdown, highlighted (code fences in their language), with the fence lines dropped.
    pub(super) fn hover_lines(&self, md: &str) -> Vec<Line<'static>> {
        let cw = (self.width.saturating_sub(4).min(100) as usize).saturating_sub(3);
        crate::md::render(md, cw, &self.ui.md, &self.hl, None)
            .into_iter()
            .map(|l| {
                let spans: Vec<Span> = l
                    .text
                    .chars()
                    .zip(l.styles.into_iter().chain(std::iter::repeat(None)))
                    .map(|(ch, st)| Span::styled(ch.to_string(), st.unwrap_or_default()))
                    .collect();
                Line::from(spans)
            })
            .collect()
    }

    /// What the overloaded keys do where the cursor is, for the bottom-right key popup: on a
    /// selection, a review thread, Claude's comment, the PR header or PR text. Nothing elsewhere,
    /// to keep it quiet.
    pub(super) fn context_keys(&self) -> Option<(&'static str, &'static [(&'static str, &'static str)])> {
        let row = &self.rows[self.head.row];
        if self.select || self.anchor.row != self.head.row {
            return Some(("selection", &[
                ("c", "comment (review)"),
                ("C", "comment now"),
                ("s", "suggest a change"),
                ("a", "a comment for Claude"),
                ("y", "copy"),
                ("Space c", "copy file:line ref"),
            ]));
        }
        if let Some(t) = row.thread {
            let t = &self.threads[t];
            return Some(match (t.claude.is_some(), agent::claude_wrote(t), t.outdated) {
                // Yours for Claude: private, so nothing here goes to GitHub.
                (true, false, _) => ("your comment for Claude", &[("r", "reply"), ("Space r", "resolve"), ("Space d", "remove"), ("Tab", "fold")]),
                (true, true, true) => ("Claude's comment (outdated)", &[("r", "reply to Claude"), ("Space d", "dismiss"), ("Tab", "fold")]),
                (true, true, false) => ("Claude's comment", &[
                    ("r", "reply to Claude"),
                    ("c", "copy to your review"),
                    ("C", "post it now"),
                    ("Space r", "resolve"),
                    ("Space d", "dismiss"),
                ]),
                (false, ..) => ("review thread", &[
                    ("r", "reply (review)"),
                    ("C", "reply now"),
                    ("Space r", "resolve"),
                    ("Space e", "edit your comment"),
                    ("Space d", "delete your comment"),
                    ("Space c", "copy link"),
                ]),
            });
        }
        if row.file == INFO && row.line.kind == Kind::File && !row.links.is_empty() {
            return Some(("PR", &[("gf", "open on GitHub"), ("Tab", "fold")]));
        }
        if row.info == Some(SUMMARY) {
            return Some(("Claude's review", &[("c", "copy to your review summary"), ("C", "post it as your PR comment")]));
        }
        (row.file == INFO && row.info.is_some())
            .then_some(("PR text", &[("C", "quote in a new conversation comment"), ("Space c", "copy link")]))
    }

    pub(super) fn draw(&mut self, f: &mut Frame) {
        let area = f.area();
        let h = area.height.saturating_sub(1) as usize;
        self.height = h.max(1);
        self.width = area.width;
        let so = 3.min(h / 2);
        let hd = self.hd();
        if hd < self.top + so {
            self.top = hd.saturating_sub(so);
        }
        if hd + so + 1 > self.top + h {
            self.top = hd + so + 1 - h;
        }

        let maxno = self.rows.iter().filter_map(|r| r.line.old.max(r.line.new)).max().unwrap_or(0);
        let lw = maxno.to_string().len().max(3);
        let gw = lw * 2 + 4;
        // Relative line numbers (Helix `line-number = "relative"`) get their own column on the left.
        let relw = if self.settings.relative { 4 } else { 0 };
        let width = (area.width as usize).saturating_sub(relw);
        let tw = width.saturating_sub(gw).max(1);
        // Split halves: one line-number column each, a 1-col divider between.
        let half = width.saturating_sub(1) / 2;
        let ghw = lw + 3;
        let (htw_l, htw_r) = (half.saturating_sub(ghw).max(1), width.saturating_sub(1 + half).saturating_sub(ghw).max(1));
        // Comments in the right margin when there's room for it beside enough code.
        // The margin: about a third of the window (60–80 columns), or all that's beyond 200 for
        // the code side (gutter included), beside at least 80 columns of code.
        let mw = (width / 3).clamp(MARGIN_MIN, MARGIN_MAX).max(width.saturating_sub(1 + MARGIN_CODE_MAX));
        let margin = self.margin_pref && !self.split && width >= gw + MARGIN_CODE_MIN + 1 + mw;
        let (ctw, mtw) = (width.saturating_sub(gw + 1 + mw).max(1), mw - 3);
        // PR text wraps to the code column's width (narrower beside the margin).
        let ww = if margin { ctw } else { tw }.saturating_sub(1).clamp(40, 100);
        if ww != self.wrap_w || margin != self.margin_active || (margin && mw != self.margin_w) {
            (self.wrap_w, self.margin_active, self.margin_w) = (ww, margin, mw);
            let keep = self.cursor_key();
            self.rebuild(keep);
        }
        self.geom = mouse::Geom { relw, gw, ghw, half, ctw, picker: None };
        if self.head.col < self.left {
            self.left = self.head.col;
        }
        let cur_w = match self.drows[self.hd()] {
            DRow::Pair(..) => htw_l.min(htw_r),
            DRow::Margin(..) if self.margin_side() => mtw,
            DRow::Margin(..) => ctw,
            DRow::Full(_) => tw,
        };
        if self.head.col >= self.left + cur_w {
            self.left = self.head.col + 1 - cur_w;
        }

        let (s, e) = self.ordered();
        let overlay: HashMap<Pos, char> = self
            .labels
            .iter()
            .filter(|l| self.label_first.is_none_or(|f| l.2[0] == f))
            .flat_map(|&(p, _, [a, b])| match self.label_first {
                None => vec![(p, a), (Pos { col: p.col + 1, ..p }, b)],
                Some(_) => vec![(p, b)], // after the first key, show the remaining char at the word start
            })
            .collect();
        for r in self.visible_rows() {
            if let Some(key) = self.hl_key(r).filter(|k| !self.hl_cache.contains_key(k)) {
                let text = match key.1 {
                    true => std::fs::read_to_string(&key.0).unwrap_or_default(),
                    false => diff::git(&["show", &format!("{}:{}", self.base, key.0)]).unwrap_or_default(),
                };
                // ponytail: whole file highlighted synchronously on first view; a thread if big PRs stutter.
                let colors = if text.len() > HL_MAX_BYTES { vec![] } else { self.hl.file(&key.0, &text) };
                self.hl_cache.insert(key, colors);
            }
        }
        let divider = Span::styled("│", self.ui.linenr);
        let hd = self.hd();
        let lit_code = self.lit_code();
        let lines: Vec<Line> = (self.top..(self.top + h).min(self.drows.len()))
            .map(|d| {
                let cur = d == hd;
                let rel = match (relw, cur) {
                    (0, _) => vec![],
                    (_, true) => vec![Span::styled(format!("{:>w$} ", "", w = relw - 1), self.ui.linenr_selected)],
                    _ => vec![Span::styled(format!("{:>w$} ", d.abs_diff(hd), w = relw - 1), self.ui.linenr)],
                };
                let mut spans = rel;
                match self.drows[d] {
                    DRow::Full(r) => spans.extend(self.render_row(r, s, e, (lw, gw, tw), None, cur, &overlay)),
                    DRow::Pair(l, rr) => {
                        let half = |row: Option<usize>, right: bool, w: usize| match row {
                            Some(row) => self.render_row(row, s, e, (lw, ghw, w), Some(right), cur, &overlay),
                            // Like delta: a quiet hatch where the other side has no line.
                            None => vec![Span::styled("╱".repeat(ghw + w), self.ui.linenr)],
                        };
                        spans.extend(half(l, false, htw_l));
                        spans.push(divider.clone());
                        spans.extend(half(rr, true, htw_r));
                    }
                    DRow::Margin(code, comment) => {
                        // Code on the left (blank on a spacer row), a gap, then the comment column.
                        match code {
                            Some(r) => {
                                let lit = lit_code.is_some_and(|(a, b)| (a..=b).contains(&r));
                                spans.extend(self.render_row(r, s, e, (lw, gw, ctw), None, cur || lit, &overlay))
                            }
                            None => spans.push(Span::raw(" ".repeat(gw + ctw))),
                        }
                        spans.push(Span::raw(" "));
                        match comment {
                            Some(r) => spans.extend(self.render_row(r, s, e, (lw, 3, mtw), None, cur, &overlay)),
                            None => spans.push(Span::raw(" ".repeat(mw))),
                        }
                    }
                }
                Line::from(spans)
            })
            .collect();
        f.render_widget(Block::new().style(self.ui.base), area);
        f.render_widget(Paragraph::new(lines), Rect { height: h as u16, ..area });

        // Link tooltip: while the cursor is on a link, show its URL just under (or over) it.
        // Not on the PR header (where dv starts): the status hint says it links there.
        let header = self.rows[self.head.row].line.kind == Kind::File;
        if let (Some(url), None, None, false) = (self.link_at_cursor(), &self.picker, &self.popup, header) {
            let y = (hd - self.top) as u16;
            // In the margin, the text starts after the code column, the gap and its own gutter.
            let start = if self.margin_side() && matches!(self.drows[hd], DRow::Margin(..)) { relw + gw + ctw + 1 + 3 } else { relw + gw };
            let x = ((start + self.head.col.saturating_sub(self.left)) as u16).min(area.width.saturating_sub(20));
            let preview = if gh::issue_ref(url).is_some() { "Space k preview · " } else { "" };
            let text = format!(" ↗ {url}  {preview}gf open ");
            let w = (text.chars().count() as u16).min(area.width - x);
            let ty = if y + 1 < h as u16 { y + 1 } else { y.saturating_sub(1) };
            let r = Rect { x: area.x + x, y: area.y + ty, width: w, height: 1 };
            f.render_widget(Clear, r);
            f.render_widget(Paragraph::new(text).style(self.ui.base.patch(self.ui.popup)), r);
        }

        let status = Rect { y: area.y + h as u16, height: 1, ..area };
        let left = match &self.prompt {
            Some(q) => Line::from(format!("/{q}")),
            None => {
                let l = &self.rows[self.head.row].line;
                let pos = match (l.new, l.old) {
                    (Some(n), _) => format!("{n}:{}", self.head.col + 1),
                    (None, Some(o)) => format!("-{o}:{}", self.head.col + 1),
                    _ => String::new(),
                };
                let (mode, mst) = if self.select { (" SEL ", self.ui.mode_select) } else { (" NOR ", self.ui.mode_normal) };
                Line::from(vec![Span::styled(mode, mst), Span::raw(format!("  {}  {pos}  {}", self.file_path().unwrap_or(""), self.msg))])
            }
        };
        let pend = match self.pending {
            Some(CTRL_W) => "C-w".into(),
            p => p.map(String::from).unwrap_or_default(),
        };
        let count = if self.count > 0 { self.count.to_string() } else { String::new() };
        let ra = match self.progress.values().next() {
            Some((t, m)) => format!("ra: {t} {m}  "),
            None if self.lsp.is_some() && !self.lsp_ready => "ra: starting  ".into(),
            None => String::new(),
        };
        let pending = match self.pending_count() {
            0 if self.review.is_none() => String::new(),
            n => format!(" · {n} pending (S submits)"),
        };
        let claude = self.threads.iter().filter(|t| agent::claude_wrote(t)).count();
        let review_file = self.threads.iter().filter(|t| t.claude.is_some()).count();
        let seen = self.files.iter().filter(|f| self.viewed.get(&f.path).is_some_and(|s| s == "VIEWED")).count();
        let on_github = self.files.iter().filter(|f| f.meta != "untracked").count(); // what "viewed" can be of
        let threads = match (self.threads.len() - review_file, claude) {
            (n, 0) => format!("{n} threads"),
            (n, c) => format!("{n} threads + {c} Claude"),
        };
        let pr = match &self.pr {
            Some(p) => format!("#{} · {threads} · {seen}/{on_github} viewed{pending}  ", p.number),
            None if claude > 0 => format!("{threads}  "),
            None => String::new(),
        };
        let working = if self.claude_working() { "Claude working… · " } else { "" };
        let view = format!(
            "{}{}{}",
            if self.claude_view { "claude view  " } else { "" },
            if self.uncommitted() { "uncommitted  " } else { "" },
            if self.split { "split  " } else { "" }
        );
        // The left side (what you're on, messages, hints) wins: when both don't fit, the right
        // keeps only the essentials, and the left is cut with an ellipsis.
        let essential = format!("{view}{count}{pend}  {}/{} ", self.head.row + 1, self.rows.len());
        let extras = format!("{ra}{working}{pr}");
        let width = area.width as usize;
        let right = match left.width() + extras.chars().count() + essential.chars().count() < width {
            true => format!("{extras}{essential}"),
            false => essential,
        };
        let left = fit(left, width.saturating_sub(right.chars().count() + 1));
        let bar = self.ui.base.patch(self.ui.status);
        f.render_widget(Paragraph::new(left).style(bar), status);
        f.render_widget(Paragraph::new(right).alignment(Alignment::Right).style(bar), status);

        if let Some(p) = &self.picker {
            let m = self.matches();
            let sel = p.sel.min(m.len().saturating_sub(1));
            // Items are cards when they have detail lines (comment pickers): all the same height.
            let per = 1 + p.items.iter().map(|i| i.detail.len()).max().unwrap_or(0);
            let gap = (per > 1) as usize; // a blank line between cards
            let item_h = per + gap;
            let w = (area.width * 3 / 4).max(20).min(area.width);
            let ph = ((m.len() * item_h) as u16 + 3).min(area.height.saturating_sub(4)).max(3);
            let r = Rect { x: (area.width - w) / 2, y: 2.min(area.height), width: w, height: ph.min(area.height) };
            let vis = ((ph as usize).saturating_sub(3) / item_h).max(1);
            let off = sel.saturating_sub(vis.saturating_sub(1));
            let mut lines = vec![Line::from(format!("> {}", p.query))];
            let q: Vec<char> = p.query.to_lowercase().chars().collect();
            for (i, &it) in m.iter().enumerate().skip(off).take(vis) {
                let st = if i == sel { self.ui.menu_selected } else { Style::new() };
                // Highlight the letters the filter matched (first fit, as `matches` checks).
                let mut j = 0;
                let spans: Vec<Span> = p.items[it]
                    .label
                    .chars()
                    .map(|c| {
                        let hit = j < q.len() && c.to_lowercase().eq(std::iter::once(q[j]));
                        j += hit as usize;
                        Span::styled(c.to_string(), if hit { st.patch(self.ui.key) } else { st })
                    })
                    .collect();
                lines.push(Line::from(spans).style(st));
                let detail = &p.items[it].detail;
                for k in 0..per - 1 {
                    let text = detail.get(k).map_or(String::new(), |d| format!("  {d}"));
                    lines.push(Line::from(Span::styled(text, st.patch(self.ui.stale))).style(st));
                }
                if gap > 0 {
                    lines.push(Line::default());
                }
            }
            let pos = if m.is_empty() { "0 / 0".into() } else { format!("{} / {}", sel + 1, m.len()) };
            let inner = self.popup_frame(f, r, p.title, &pos);
            f.render_widget(Paragraph::new(lines), inner);
            // Items start below the query line.
            let list = Rect { y: inner.y + 1, height: inner.height.saturating_sub(1), ..inner };
            self.scrollbar(f, list, m.len(), vis, off);
            self.geom.picker = Some(mouse::PickerGeom { list, item_h, off });
        }

        // Bottom right, like Helix's which-key: what can follow a pending prefix key, else what the
        // keys do where the cursor is (when no other popup is up). Bottom left while the cursor is
        // in the comment margin, so it doesn't cover the comments there.
        let keys = match self.pending {
            Some(p) => App::which(p),
            None if self.picker.is_none() && self.popup.is_none() && self.prompt.is_none() => self.context_keys(),
            None => None,
        };
        if let Some((title, keys)) = keys {
            let kw = keys.iter().map(|k| k.0.len()).max().unwrap_or(1);
            let lines: Vec<Line> = keys
                .iter()
                .map(|(k, d)| Line::from(vec![Span::styled(format!("{k:<kw$}  "), self.ui.key), Span::raw(*d)]))
                .collect();
            let w = (lines.iter().map(Line::width).max().unwrap_or(10) as u16 + 4).max(title.len() as u16 + 4).min(area.width);
            let ph = (lines.len() as u16 + 2).min(h as u16);
            let x = if self.margin_side() { 0 } else { area.width - w };
            let r = Rect { x, y: (h as u16).saturating_sub(ph), width: w, height: ph };
            let inner = self.popup_frame(f, r, title, "");
            f.render_widget(Paragraph::new(lines), inner);
        }

        if let Some((title, text, scroll)) = self.popup.clone() {
            // As wide as the text (hover is wrapped to fit 100; help may be two columns), and the title.
            let text_w = text.iter().map(Line::width).max().unwrap_or(0).max(title.chars().count() + 1);
            let w = (text_w as u16 + 3).max(30).min(area.width.saturating_sub(4));
            // ponytail: wrapped height estimated from line widths (char wrap, not word wrap).
            let cw = w.saturating_sub(3).max(1) as usize;
            let total: usize = text.iter().map(|l| l.width().div_ceil(cw).max(1)).sum();
            let h = (total as u16 + 2).min(area.height.saturating_sub(4)).max(3);
            let r = Rect { x: (area.width - w) / 2, y: 2.min(area.height), width: w, height: h.min(area.height) };
            let vis = h.saturating_sub(2) as usize;
            // Don't scroll past the end.
            let scroll = scroll.min(total.saturating_sub(vis) as u16);
            if let Some(p) = &mut self.popup {
                p.2 = scroll;
            }
            let first = scroll as usize + 1;
            let pos = if total > vis { format!("{first}–{} / {total}", (first + vis - 1).min(total)) } else { String::new() };
            let inner = self.popup_frame(f, r, &title, &pos);
            let text_area = Rect { width: inner.width.saturating_sub(1), ..inner }; // room for the scrollbar
            f.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }).scroll((scroll, 0)), text_area);
            self.scrollbar(f, inner, total, vis, scroll as usize);
        }
    }

    /// A thin scrollbar on the right edge of `area`, only when `total` items don't fit in `vis`.
    pub(super) fn scrollbar(&self, f: &mut Frame, area: Rect, total: usize, vis: usize, pos: usize) {
        if total <= vis {
            return;
        }
        let mut state = ScrollbarState::new(total.saturating_sub(vis)).viewport_content_length(vis).position(pos);
        let bar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .track_symbol(Some("│"))
            .track_style(self.ui.linenr)
            .thumb_symbol("┃")
            .thumb_style(self.ui.linenr_selected);
        f.render_stateful_widget(bar, area, &mut state);
    }

    /// Helix-style popup: no border, the theme's popup background, the title as a header row
    /// in the header color. Returns the padded content area.
    pub(super) fn popup_frame(&self, f: &mut Frame, r: Rect, title: &str, pos: &str) -> Rect {
        f.render_widget(Clear, r);
        f.render_widget(Block::new().style(self.ui.base.patch(self.ui.popup)), r);
        let head = Rect { height: 1.min(r.height), ..r };
        let head_style = self.ui.base.patch(self.ui.header);
        // A long title gives way to the position on the right.
        let room = (r.width as usize).saturating_sub(pos.chars().count() + 3);
        let title = title.trim();
        let title = match title.chars().count() > room {
            true => format!("{}…", title.chars().take(room.saturating_sub(1)).collect::<String>()),
            false => title.to_owned(),
        };
        f.render_widget(Paragraph::new(format!(" {title}")).style(head_style), head);
        // Where you are in the content, on the right of the header.
        f.render_widget(Paragraph::new(format!("{pos} ")).alignment(Alignment::Right).style(head_style), head);
        Rect { x: r.x + 1, y: r.y + 1, width: r.width.saturating_sub(2), height: r.height.saturating_sub(2) }
    }

    /// A hunk header drawn as a quiet rule: `── fn main ─────── L12–18`. The raw `@@` text stays
    /// the row's text (search, `Y`); only the drawing changes.
    pub(super) fn hunk_divider(&self, r: usize, gw: usize, tw: usize) -> Option<Vec<Span<'static>>> {
        let (start, count, ctx) = diff::hunk_label(&self.rows[r].line.text)?;
        let range = match count {
            0 | 1 => format!(" L{start} "),
            n => format!(" L{start}–{} ", start + n - 1),
        };
        let mut left = String::from("──");
        if !ctx.is_empty() {
            left += &format!(" {ctx} ");
        }
        let fill = tw.saturating_sub(left.chars().count() + range.chars().count() + 1);
        let text = format!("{left}{}{range}─", "─".repeat(fill));
        let mut spans = vec![Span::styled(" ".repeat(gw), self.ui.linenr)];
        let mut chars = text.chars();
        if self.head.row == r {
            let c = chars.next().unwrap_or(' ');
            spans.push(Span::styled(c.to_string(), if self.select { self.ui.cursor_select } else { self.ui.cursor }));
        }
        spans.push(Span::styled(chars.collect::<String>(), self.ui.hunk));
        Some(spans)
    }

    /// One row's spans, padded to the gutter + `tw` width. `side`: None = full width, else
    /// Some(false/true) = the old/new half of a split row (one line number, not two).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_row(
        &self, r: usize, s: Pos, e: Pos, (lw, gw, tw): (usize, usize, usize), side: Option<bool>, cursor_row: bool,
        overlay: &HashMap<Pos, char>,
    ) -> Vec<Span<'static>> {
        let row = &self.rows[r];
        let l = &row.line;
        let num = |n: Option<u32>| n.map(|n| n.to_string()).unwrap_or_default();
        let mut line_bg = None;
        let ui = &self.ui;
        let mut gstyle = ui.linenr;
        let mut part = None;
        let (gutter, style) = match l.kind {
            Kind::File => {
                let collapsed = match row.file {
                    INFO => self.info_collapsed,
                    f => self.files.get(f).is_some_and(|f| self.collapsed.contains(&f.path)),
                };
                line_bg = ui.header.bg;
                gstyle = ui.header;
                (format!(" {:<w$}", if collapsed { "▸" } else { "▾" }, w = gw - 1), ui.header)
            }
            Kind::Hunk => {
                if let Some(spans) = self.hunk_divider(r, gw, tw) {
                    return spans;
                }
                (" ".repeat(gw), ui.hunk)
            }
            Kind::Thread | Kind::Comment => {
                let resolved = row.thread.is_some_and(|t| self.threads[t].stale());
                let st = match (l.kind, resolved) {
                    (_, true) => ui.stale,
                    (Kind::Thread, _) if row.file == INFO => ui.heading,
                    (Kind::Thread, _) => ui.thread,
                    _ => Style::new(),
                };
                part = self.box_part(r);
                let first = row.thread.is_some() && (r == 0 || self.rows[r - 1].thread != row.thread);
                // Comments sit in a box with a subtle border; a folded thread is just its ▸ line.
                let mark = match part {
                    Some(BoxPart::Top) => "╭─ ",
                    Some(BoxPart::Divider) => "├─ ",
                    Some(BoxPart::Body) => "│  ",
                    Some(BoxPart::Bottom) => "╰──",
                    None if first => " ▸ ",
                    None if row.file == INFO => "   ", // PR facts: the boxes below set them apart
                    None => " ┃ ",
                };
                // No fill: the box's border sets comments apart. The gutter keeps the page background
                // (so the cursor line's tint stays in the text column, inside the box). A lit box
                // (margin mode: the one about the cursor's line) has its border in the accent.
                if part.is_some() && row.thread.is_some() && row.thread == self.lit_thread() {
                    gstyle = ui.key;
                }
                gstyle = gstyle.bg(ui.base.bg.unwrap_or(Color::Reset));
                (format!("{:>gw$}", mark), st)
            }
            k => {
                let m = match k {
                    Kind::Add => {
                        (line_bg, gstyle) = (Some(ui.add_bg), ui.plus);
                        '+'
                    }
                    Kind::Del => {
                        (line_bg, gstyle) = (Some(ui.del_bg), ui.minus);
                        '-'
                    }
                    _ => ' ',
                };
                let gutter = match side {
                    None => format!("{:>lw$} {:>lw$} {m} ", num(l.old), num(l.new)),
                    Some(right) => format!("{:>lw$} {m} ", num(if right { l.new } else { l.old })),
                };
                (gutter, Style::new())
            }
        };
        if cursor_row && self.settings.cursorline {
            if matches!(l.kind, Kind::Ctx | Kind::Add | Kind::Del) {
                gstyle = gstyle.patch(ui.linenr_selected);
            }
            // Like Helix: the cursor's line is tinted. Added/removed lines keep their color, lifted.
            line_bg = match l.kind {
                Kind::Add => Some(ui.add_bg_cur),
                Kind::Del => Some(ui.del_bg_cur),
                Kind::Ctx | Kind::Thread | Kind::Comment => ui.cursorline.or(line_bg),
                _ => line_bg,
            };
        }
        let emph_bg = if l.kind == Kind::Add { ui.emph_add } else { ui.emph_del };
        let emph = self.emph.get(&r);
        let syn = match row.syn.is_empty() {
            false => Some(&row.syn),
            true => self.hl_key(r).and_then(|k| {
                let n = if k.1 { l.new } else { l.old }?;
                self.hl_cache.get(&k)?.get(n as usize - 1)
            }),
        };
        let mut spans = vec![Span::styled(gutter, gstyle)];
        let mut n = 0;
        // A context line shows on both sides of a split row; the cursor goes on the current side.
        let this_side = side.is_none() || l.kind != Kind::Ctx || side == Some(self.right);
        let cursor_here = self.head.row == r && this_side;
        for (i, c) in l.text.chars().enumerate().skip(self.left).take(tw) {
            let p = Pos { row: r, col: i };
            let mut st = style;
            if let Some(c) = syn.and_then(|syn| syn.get(i).copied().flatten()) {
                st = st.patch(c);
            }
            if emph.is_some_and(|e| e.iter().any(|&(a, b)| a <= i && i < b)) {
                st = st.bg(emph_bg);
            }
            if s <= p && p <= e && this_side {
                st = st.patch(ui.selection);
            }
            if p == self.head && cursor_here {
                st = st.patch(if self.select { ui.cursor_select } else { ui.cursor });
            }
            if let Some(&lc) = overlay.get(&p) {
                spans.push(Span::styled(lc.to_string(), ui.jump_label));
                n = i + 1;
                continue;
            }
            // ponytail: tab drawn as 4 cols but scrolled as 1 char; fine unless tab-heavy files.
            spans.push(Span::styled(if c == '\t' { "    ".into() } else { c.to_string() }, st));
            n = i + 1;
        }
        if cursor_here && self.head.col >= n.max(self.left) {
            spans.push(Span::styled(" ", if self.select { ui.cursor_select } else { ui.cursor }));
        }
        // Pad to the full width (so split halves line up), and tint the whole row, so
        // added/removed lines read as bands. Spans with their own bg (the cursor) keep it.
        let used: usize = spans[1..].iter().map(|s| s.width()).sum();
        if let Some(p) = part {
            // The box's right side: header rows run a rule to the corner, body rows pad to the
            // edge. It's as wide as the comment text wraps (+ margin), not the whole screen.
            let bw = tw.min(self.wrap_w + 2);
            let (fill, lead, corner) = match p {
                BoxPart::Top => ('─', " ", '╮'),
                BoxPart::Divider => ('─', " ", '┤'),
                BoxPart::Body => (' ', "", '│'),
                BoxPart::Bottom => ('─', "", '╯'),
            };
            let lead = if used == 0 { "" } else { lead };
            let n = bw.saturating_sub(used + lead.len() + 1);
            let edge = if row.thread.is_some() && row.thread == self.lit_thread() { ui.key } else { ui.linenr };
            spans.push(Span::styled(format!("{lead}{}", fill.to_string().repeat(n)), style.patch(edge)));
            if let Some(bg) = line_bg {
                for sp in &mut spans {
                    sp.style = Style::new().bg(bg).patch(sp.style);
                }
            }
            let border = edge.bg(ui.base.bg.unwrap_or(Color::Reset));
            spans.push(Span::styled(corner.to_string(), border));
            let used: usize = spans[1..].iter().map(|s| s.width()).sum();
            spans.push(Span::raw(" ".repeat(tw.saturating_sub(used))));
            return spans;
        }
        let tag = if l.kind == Kind::File { self.header_tag(row.file) } else { None };
        // The PR's state always shows: a long title is cut to make room.
        let mut used = used;
        if let Some((t, _)) = &tag
            && row.file == INFO
        {
            while used + 1 + t.chars().count() > tw && spans.len() > 1 {
                used -= spans.pop().map_or(0, |s| s.width());
            }
        }
        match tag {
            // Right-aligned when it fits after the name; it isn't part of the row's text.
            Some((t, st)) if used + 1 + t.chars().count() <= tw => {
                let n = t.chars().count();
                spans.push(Span::raw(" ".repeat(tw - used - n)));
                spans.push(Span::styled(t, style.patch(st)));
            }
            _ => spans.push(Span::raw(" ".repeat(tw.saturating_sub(used)))),
        }
        if let Some(bg) = line_bg {
            for sp in &mut spans {
                sp.style = Style::new().bg(bg).patch(sp.style);
            }
        }
        spans
    }
}

/// `line` cut to `w` columns, ending in `…` if it was longer.
fn fit(line: Line<'static>, w: usize) -> Line<'static> {
    if line.width() <= w {
        return line;
    }
    let mut left = w.saturating_sub(1);
    let mut spans = vec![];
    for sp in line.spans {
        let n = sp.content.chars().count();
        if n <= left {
            left -= n;
            spans.push(sp);
        } else {
            spans.push(Span::styled(sp.content.chars().take(left).collect::<String>(), sp.style));
            break;
        }
    }
    spans.push(Span::raw("…"));
    Line::from(spans)
}
