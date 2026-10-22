//! One row of the diff on screen: its gutter, its text (code, a comment box, a section), its marks.
use super::*;

impl App {
    /// How the cursor's drawn: selecting or not; nothing while dv isn't focused (every place
    /// that draws it, by this).
    pub(super) fn cursor_style(&self) -> Style {
        match (self.unfocused, self.select) {
            (true, _) => Style::new(),
            (_, true) => self.ui.cursor_select,
            _ => self.ui.cursor,
        }
    }
}

/// `line` cut to `w` columns, ending in `…` if it was longer.
pub(super) fn fit(line: Line<'static>, w: usize) -> Line<'static> {
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

/// The columns `c` takes on screen (a tab, 4: as `render_row` draws it).
pub(super) fn char_w(c: char) -> usize {
    if c == '\t' { 4 } else { Span::raw(&*c.encode_utf8(&mut [0; 4])).width() }
}

/// `spans` cut to `w` columns, padded to them if shorter.
pub(super) fn clip(spans: Vec<Span<'static>>, w: usize) -> Vec<Span<'static>> {
    let mut out = vec![];
    let mut used = 0;
    for sp in spans {
        let sw = sp.width();
        if used + sw <= w {
            used += sw;
            out.push(sp);
            continue;
        }
        let mut text = String::new();
        for c in sp.content.chars() {
            let cw = Span::raw(c.to_string()).width();
            if used + cw > w {
                break;
            }
            (used, _) = (used + cw, text.push(c));
        }
        out.push(Span::styled(text, sp.style));
        break;
    }
    out.push(Span::raw(" ".repeat(w - used)));
    out
}

/// `spans` without their first `n` columns.
pub(crate) fn skip_cols(spans: Vec<Span<'static>>, n: usize) -> Vec<Span<'static>> {
    let mut left = n;
    let mut out = vec![];
    for sp in spans {
        if left == 0 {
            out.push(sp);
            continue;
        }
        let mut text = String::new();
        for c in sp.content.chars() {
            let cw = Span::raw(c.to_string()).width();
            if left >= cw && left > 0 {
                left -= cw;
            } else {
                text.push(c);
            }
        }
        if !text.is_empty() {
            out.push(Span::styled(text, sp.style));
        }
    }
    out
}

impl App {
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
            spans.push(Span::styled(c.to_string(), self.ui.hunk.patch(self.cursor_style())));
        }
        spans.push(Span::styled(chars.collect::<String>(), self.ui.hunk));
        Some(spans)
    }

    /// The comment mark at the right edge of code row `r` (`side`: which half of a split row):
    /// `╎` over a comment's lines, a dot at its last, `●` others', `◆` Claude's, `○` resolved or
    /// outdated (`App::marks`).
    pub(super) fn comment_mark(&self, r: usize, side: Option<bool>) -> Option<(char, Style)> {
        let rank = match side {
            Some(right) => self.marks.get(&(r, !right)).copied(),
            None => self.marks.get(&(r, false)).copied().max(self.marks.get(&(r, true)).copied()),
        }?;
        Some(match rank {
            MARK_DOT => ('●', self.ui.thread),
            MARK_CLAUDE_DOT => ('◆', self.ui.thread),
            MARK_STALE_DOT => ('○', self.ui.stale),
            MARK_LINE => ('╎', self.ui.thread),
            _ => ('╎', self.ui.stale),
        })
    }

    /// The code a comment in the pane is about (the cursor there): its own tint, not the
    /// cursor line's, over the background and the added/removed lines' tints.
    pub(super) fn related_tint(&self, spans: Vec<Span<'static>>) -> Vec<Span<'static>> {
        let ui = &self.ui;
        let Some(rel) = ui.related else { return spans };
        spans
            .into_iter()
            .map(|mut sp| {
                sp.style.bg = match sp.style.bg {
                    None => Some(rel),
                    b if b == ui.base.bg => Some(rel),
                    Some(b) if b == ui.add_bg => Some(ui.add_bg_rel),
                    Some(b) if b == ui.del_bg => Some(ui.del_bg_rel),
                    b => b,
                };
                sp
            })
            .collect()
    }

    /// Whether row `r`'s box is open on its left: in the pane, as a column (its rules come
    /// from the line between it and the code).
    pub(super) fn open_left(&self, r: usize) -> bool {
        self.margin_row(r) && self.margin_active && !self.margin_float
    }

    /// One row's spans, padded to the gutter + `tw` width. `side`: None = full width, else
    /// Some(false/true) = the old/new half of a split row (one line number, not two).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_row(
        &self, r: usize, s: Pos, e: Pos, (lw, gw, tw): (usize, usize, usize), side: Option<bool>, cursor_row: bool,
        overlay: &HashMap<Pos, char>, seg: usize,
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
                    INFO if row.main.is_some() => self.section_of(r).is_some_and(|s| self.sections_folded.contains(&s)),
                    INFO => self.info_collapsed,
                    f => self.files.get(f).is_some_and(|f| self.collapsed.contains(&f.path)),
                };
                // The pinned title: as dark as the status bar.
                let header = match r == 0 && self.pin {
                    true => Style { bg: ui.bar.or(ui.header.bg), ..ui.header },
                    false => ui.header,
                };
                (line_bg, gstyle) = (header.bg, header);
                let arrow = match collapsed {
                    _ if r == 0 && self.pin => " ", // a panel, not something to fold
                    true => "▸",
                    false => "▾",
                };
                (format!(" {arrow:<w$}", w = gw - 1), header)
            }
            Kind::Hunk => {
                if let Some(spans) = self.hunk_divider(r, gw, tw) {
                    return spans;
                }
                (" ".repeat(gw), ui.hunk)
            }
            Kind::Thread | Kind::Comment => {
                let resolved = row.thread.is_some_and(|t| self.threads[t].stale());
                let hidden = self.hidden_line(r);
                let st = match (l.kind, resolved) {
                    (_, true) => ui.stale,
                    _ if hidden => ui.stale,
                    (Kind::Thread, _) if row.file == INFO => ui.heading,
                    (Kind::Thread, _) => ui.thread,
                    _ => Style::new(),
                };
                part = self.box_part(r);
                let first = row.thread.is_some() && (r == 0 || self.rows[r - 1].thread != row.thread);
                // Comments sit in a box with a subtle border; a folded thread is just its ▸ line.
                // In the pane (as a column), a box is open on its left: its rules come from the
                // line beside the code.
                let mark = match part {
                    Some(p) if self.open_left(r) => {
                        gstyle = self.ui.linenr;
                        if p == BoxPart::Bottom { "─" } else { " " }
                    }
                    Some(BoxPart::Top) => "╭─ ",
                    Some(BoxPart::Divider) => "├─ ",
                    Some(BoxPart::Body) => "│  ",
                    Some(BoxPart::Bottom) => "╰──",
                    None if first || hidden => " ▸ ",
                    None if row.file == INFO => "   ", // PR facts: the boxes below set them apart
                    None => " ┃ ",
                };
                // No fill: the box's border sets comments apart. The gutter keeps the page background
                // (so the cursor line's tint stays in the text column, inside the box).
                gstyle = gstyle.bg(ui.base.bg.unwrap_or(Color::Reset));
                // (A narrower gutter, in the pane: the mark's end, ` ▸ ` → `▸ `.)
                let mark: String = mark.chars().skip(mark.chars().count().saturating_sub(gw)).collect();
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
            if l.kind.is_code() {
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
                let n = if l.kind == Kind::Del { l.old } else { l.new }?;
                self.hl_cache.get(&k)?.get(n as usize - 1)
            }),
        };
        // A code line keeps its last column for comment marks (`╎` over a comment's lines, a dot
        // at its last), at the edge beside the pane.
        let code_row = l.kind.is_code();
        let mark = code_row.then(|| self.comment_mark(r, side)).flatten();
        let tw = if code_row { tw.saturating_sub(1) } else { tw };
        let mut spans = vec![Span::styled(gutter, gstyle)];
        // A reply under Claude's comment: its guide, then its text (narrower by it).
        if row.guide {
            spans.push(Span::styled(GUIDE, self.ui.stale));
        }
        let g = spans.len(); // the gutter's spans (and a reply's guide); the text's follow
        let mut n = 0;
        // A context line shows on both sides of a split row; the cursor goes on the current side.
        let this_side = side.is_none() || l.kind != Kind::Ctx || side == Some(self.right);
        let cursor_here = self.head.row == r && this_side;
        // In a box, what doesn't wrap (code) is cut at its edge, and scrolled on its own.
        // (Its box as wide as the comment's, measured from after a reply's guide.)
        let (text_w, bw) = (self.text_w(r, tw), self.box_w(r, tw).saturating_sub(row.guide_w()));
        let tw = tw.saturating_sub(row.guide_w());
        // Scrolled by columns, every row alike (a tab's 4, a wide character's 2): the chars
        // wholly in `left..left + text_w`, after the part of one cut at the left.
        // Wrapped (`soft_wrap`), a part of the line (`parts`: whole chars each); past its last
        // (a pair's shorter side), nothing; and after its first, no numbers: `↪` for its mark.
        let wrapped = self.wraps(r);
        let shown = self.span_of(r, if wrapped { seg } else { 0 });
        if wrapped && seg > 0 {
            let w = spans[0].width();
            let mark = if shown.is_some() { "↪" } else { "" };
            spans[0].content = format!("{mark:>w$} ", w = w.saturating_sub(1)).into();
        }
        let left = shown.as_ref().map_or(0, |s| s.left);
        let (skip, end) = match (&shown, wrapped) {
            (Some(s), true) => (s.chars.start, s.chars.end),
            (None, _) => (self.len(r), self.len(r)), // (a short side's row past its end)
            (Some(_), false) => (self.char_from(r, left), self.char_at(r, left + text_w)),
        };
        let cut = self.col_of(r, skip) - left;
        if cut > 0 {
            spans.push(Span::styled(" ".repeat(cut), style));
        }
        for (i, c) in l.text.chars().enumerate().take(end).skip(skip) {
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
                st = st.patch(self.cursor_style());
            }
            if let Some(&lc) = overlay.get(&p) {
                spans.push(Span::styled(lc.to_string(), ui.jump_label));
                n = i + 1;
                continue;
            }
            spans.push(Span::styled(if c == '\t' { "    ".into() } else { c.to_string() }, st));
            n = i + 1;
        }
        // (Past the line's end: on its last part, wrapped.)
        let last_part = !wrapped || self.part(r, seg + 1).is_none();
        if cursor_here && self.head.col >= n.max(skip) && last_part {
            spans.push(Span::styled(" ", self.cursor_style()));
        }
        // A line cut, a diff's by the window or code by its box: `‹` / `›` where there's more,
        // to scroll to (not over the cursor).
        if self.scroller(r).is_some() && spans.len() > g && !wrapped {
            let more = Style::new().fg(ui.linenr.fg.unwrap_or(Color::DarkGray));
            // (Not over a jump label; over a wide character, a space after it: the rest stays put.)
            let mark = |sp: &mut Span<'static>, c: &str, at: usize| {
                if !(cursor_here && self.head.col == at) && !overlay.contains_key(&Pos { row: r, col: at }) {
                    let pad = " ".repeat(sp.width().saturating_sub(1));
                    (sp.content, sp.style) = (format!("{c}{pad}").into(), sp.style.patch(more));
                }
            };
            if left > 0 {
                mark(&mut spans[g], "‹", skip);
            }
            if l.text.chars().count() > end && end > skip {
                let last = spans.len() - 1;
                mark(&mut spans[last], "›", end - 1);
            }
        }
        // Pad to the full width (so split halves line up), and tint the whole row, so
        // added/removed lines read as bands. Spans with their own bg (the cursor) keep it.
        let used: usize = spans[g..].iter().map(|s| s.width()).sum();
        // A code block's line: its background on to the row's end (its box's edge).
        let code_bg = row.nowrap.then(|| row.syn.last().copied().flatten()?.bg).flatten().filter(|&c| Some(c) == ui.md.code_bg);
        // On the cursor's line, the cursor line's tint over it (the code's own colors kept).
        let code_bg = match (code_bg, ui.cursorline) {
            (Some(c), Some(cur)) if cursor_row && self.settings.cursorline => {
                for sp in &mut spans[g..] {
                    if sp.style.bg == Some(c) {
                        sp.style.bg = Some(cur);
                    }
                }
                Some(cur)
            }
            (c, _) => c,
        };
        let pad_style = |st: Style| match code_bg {
            Some(c) => st.bg(c),
            None => st,
        };
        if let Some(p) = part {
            // The box's right side: header rows run a rule to the corner, body rows pad to the edge.
            let (fill, lead, corner) = match p {
                BoxPart::Top => ('─', " ", '╮'),
                BoxPart::Divider => ('─', " ", '┤'),
                BoxPart::Body => (' ', "", '│'),
                BoxPart::Bottom => ('─', "", '╯'),
            };
            let lead = if used == 0 { "" } else { lead };
            let n = bw.saturating_sub(used + lead.len() + 1);
            let edge = self.ui.linenr;
            spans.push(Span::styled(format!("{lead}{}", fill.to_string().repeat(n)), pad_style(style.patch(edge))));
            if let Some(bg) = line_bg {
                for sp in &mut spans {
                    sp.style = Style::new().bg(bg).patch(sp.style);
                }
            }
            let border = edge.bg(ui.base.bg.unwrap_or(Color::Reset));
            spans.push(Span::styled(corner.to_string(), border));
            let used: usize = spans[g..].iter().map(|s| s.width()).sum();
            spans.push(Span::raw(" ".repeat(tw.saturating_sub(used))));
            return spans;
        }
        let tag = if l.kind == Kind::File && row.main.is_none() { self.header_tag(row.file) } else { None };
        // Pinned, the PR's title has its counts before its state, when they fit beside all of
        // it (`bar_counts`; else they stay on the status bar); and, always, how the checkout
        // stands against the PR, a badge (what Claude's doing: the status bar's).
        let pinned = r == 0 && self.pin;
        let counts = match pinned && self.bar_counts {
            true => self.pr_counts(),
            false => String::new(),
        };
        let mut segs: Vec<(String, Style)> = vec![];
        if !counts.is_empty() {
            segs.push((counts, style.patch(ui.linenr)));
        }
        // (What Claude's doing: the status bar's.)
        if pinned && !self.sync.is_empty() {
            segs.push((format!(" {} ", self.sync), super::render::badge(ui, ui.warning.fg)));
        }
        if let Some((t, st)) = tag {
            segs.push((t, style.patch(st)));
        }
        let width = |segs: &[(String, Style)]| segs.iter().map(|(t, _)| t.chars().count()).sum::<usize>() + 2 * segs.len().saturating_sub(1);
        // These always show: a long title is cut to make room (the counts go first).
        let mut used = used;
        if !segs.is_empty() && row.file == INFO {
            while used + 1 + width(&segs) > tw && spans.len() > 1 {
                used -= spans.pop().map_or(0, |s| s.width());
            }
        }
        match width(&segs) {
            // Right-aligned when it fits after the name; it isn't part of the row's text.
            n if n > 0 && used + 1 + n <= tw => {
                spans.push(Span::raw(" ".repeat(tw - used - n)));
                for (k, (t, st)) in segs.into_iter().enumerate() {
                    if k > 0 {
                        spans.push(Span::raw("  "));
                    }
                    spans.push(Span::styled(t, st));
                }
            }
            _ => spans.push(Span::styled(" ".repeat(tw.saturating_sub(used)), pad_style(Style::new()))),
        }
        if code_row {
            spans.push(match mark {
                Some((c, st)) => Span::styled(c.to_string(), st),
                None => Span::raw(" "),
            });
        }
        if let Some(bg) = line_bg {
            for sp in &mut spans {
                sp.style = Style::new().bg(bg).patch(sp.style);
            }
        }
        spans
    }
}
