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
        if self.hidden_here().is_some() {
            return Some(("hidden on GitHub", &[("Tab", "show it, or hide it again"), ("r", "reply")]));
        }
        // On a link: what can be done with it (before what the row it's in offers).
        if self.link_at_cursor().is_some() {
            return Some(("link", &[("Enter", "open it in the browser"), ("Space c", "copy it"), ("Space k", "preview it (an issue or PR)")]));
        }
        if let Some(t) = row.thread {
            let t = &self.threads[t];
            return Some(match (t.claude.is_some(), agent::claude_wrote(t), t.outdated) {
                // Yours for Claude: private, so nothing here goes to GitHub.
                (true, false, _) => ("your comment for Claude", &[("r", "reply"), ("Space r", "resolve"), ("Space d", "remove"), ("Tab", "fold")]),
                (true, true, true) => ("Claude's comment (outdated)", &[("r", "reply to Claude"), ("Space d", "dismiss"), ("Tab", "fold")]),
                (true, true, false) => ("Claude's comment", &[
                    ("r", "reply to Claude"),
                    ("gf", "go to a file:line or name in it"),
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
                    ("Space =", "react"),
                    ("Space c", "copy link"),
                ]),
            });
        }
        if row.file == INFO && row.line.kind == Kind::File && !row.links.is_empty() {
            return Some(("PR", &[("gf", "open on GitHub"), ("Tab", "fold")]));
        }
        if row.info == Some(MORE_CONVERSATION) {
            return Some(("conversation", &[("Enter", "all of it, or only the latest"), ("Space o", "the same")]));
        }
        if row.info == Some(SUMMARY) {
            return Some(("Claude's review", &[("r", "reply (Claude answers)"), ("c", "add to your review summary"), ("C", "post it as your PR comment")]));
        }
        if self.replacement_draft(self.head.row).is_some() {
            return Some(("Claude's draft", &[("a", "ask Claude to redraft this (quoted)")]));
        }
        if row.info == Some(FACTS) {
            return Some(("PR", &[("Enter", "open the link under the cursor (a check: its run)"), ("gf", "the same")]));
        }
        if row.info == Some(DRAFT) {
            return Some(("your review", &[("S", "edit it and submit"), ("C", "new conversation comment"), ("Space c", "copy link")]));
        }
        (row.file == INFO && row.info.is_some())
            .then_some(("PR text", &[("c", "quote in your review summary"), ("C", "quote in a new conversation comment"), ("Space =", "react"), ("Space c", "copy link")]))
    }

    /// The Claude or pending view with nothing in it: what the code side and the pane say.
    fn empty_note(&self) -> Option<(&'static str, &'static str)> {
        let any = self.any_in_view();
        match self.view {
            View::Claude if !any && self.claude.summary.is_empty() && !self.unpublished() => {
                Some(("Nothing here yet: Claude hasn't reviewed this (Space a asks it).", "No comments from Claude."))
            }
            View::Pending if !any && self.summary_draft.is_empty() => {
                Some(("Nothing here yet: your pending review is empty (c on a line queues a comment).", "No pending comments."))
            }
            _ => None,
        }
    }

    pub(super) fn draw(&mut self, f: &mut Frame) {
        // The changed files' list on the left (`Ctrl-w e`), the rest beside it.
        let full = f.area();
        let sw = self.files_width(full.width as usize);
        self.files_w = sw;
        let area = Rect { x: full.x + sw as u16, width: full.width - sw as u16, ..full };
        // The PR's title line pinned at the top, as a panel over the code and the pane.
        self.pin = self.rows.first().is_some_and(|r| r.file == INFO && r.line.kind == Kind::File)
            && matches!(self.drows.first(), Some(DRow::Full(0)))
            && area.height > 4;
        self.bar_counts = false; // until the title's drawn, with room for them
        let pin = self.pin as usize;
        let h = (area.height as usize).saturating_sub(1 + pin);
        self.height = h.max(1);
        self.width = full.width; // pickers and popups: over the whole window, the contents too
        let so = 3.min(h / 2);

        let maxno = self.rows.iter().filter_map(|r| r.line.old.max(r.line.new)).max().unwrap_or(0);
        let lw = maxno.to_string().len().max(3);
        let gw = lw * 2 + 4;
        // Relative line numbers (Helix `line-number = "relative"`) get their own column on the left.
        let relw = if self.settings.relative { 4 } else { 0 };
        let width = (area.width as usize).saturating_sub(relw);
        let tw = width.saturating_sub(gw).max(1);
        let ghw = lw + 3;
        // Comments in the right margin. It's about a third of the window (60–80 columns), or
        // all that's beyond 200 for the code (gutter included). Shown unless hidden (`Ctrl-w c`)
        // when there's room beside it for at least 80 columns of code (70 each side, in split
        // view); in a narrower window it's hidden unless shown (then it takes half the window,
        // floating over the code), except in the Claude and pending views, which are about the
        // comments. The cursor on a comment (gone to with `]c`, say) shows it.
        let mut mw = (width / 3).clamp(MARGIN_MIN, MARGIN_MAX).max(width.saturating_sub(1 + MARGIN_CODE_MAX));
        let room = match self.split {
            true => width >= 2 * (ghw + MARGIN_SPLIT_MIN) + 1 + 1 + mw,
            false => width >= gw + MARGIN_CODE_MIN + 1 + mw,
        };
        if !room {
            mw = (width / 2).clamp(MARGIN_NARROW, MARGIN_MIN);
        }
        // As set (`Ctrl-w ,` / `.`), leaving some code beside it.
        mw = self.margin_w_pref.unwrap_or(mw).min(width.saturating_sub(gw + 20)).max(MARGIN_NARROW);
        let mut shown = self.margin_pref.unwrap_or(room || self.view != View::All);
        if !shown && self.in_margin(self.head.row) && !self.rows[self.head.row].attached {
            (self.margin_pref, shown) = (Some(true), true);
        }
        // A very narrow window: shown, the pane floats over all of the code beside it (else a
        // comment the cursor goes to would be nowhere).
        let tiny = width <= mw + gw + 10;
        if tiny {
            mw = width.saturating_sub(gw + 1).max(12);
        }
        let margin = shown && width > gw + 12;
        let float = margin && (tiny || self.margin_float_pref.unwrap_or(!room));
        let mtw = mw - if float { 3 } else { 1 }; // (`pane_gw`, before it's set)
        // The code's columns: the whole width, or what's left of the margin beside it; split
        // view's halves (one line-number column each, a 1-col divider between) within that.
        let code_w = if margin && !float { width.saturating_sub(1 + mw) } else { width };
        let ctw = code_w.saturating_sub(gw).max(1);
        let half = code_w.saturating_sub(1) / 2;
        let (htw_l, htw_r) = (half.saturating_sub(ghw).max(1), code_w.saturating_sub(1 + half).saturating_sub(ghw).max(1));
        // PR text wraps to the code column's width (narrower beside the margin).
        let ww = if margin { ctw } else { tw }.saturating_sub(1).clamp(40, 100);
        if ww != self.wrap_w || margin != self.margin_active || (margin && mw != self.margin_w) || float != self.margin_float {
            (self.wrap_w, self.margin_active, self.margin_w, self.margin_float) = (ww, margin, mw, float);
            let keep = self.cursor_key();
            self.rebuild(keep);
        }
        if !self.margin_side() {
            self.code_head = self.head;
        }
        // Scroll the code to the cursor.
        let hd = self.hd();
        if hd < self.top + so {
            self.top = hd.saturating_sub(so);
        }
        if hd + so + 1 > self.top + h {
            self.top = hd + so + 1 - h;
        }
        self.top = self.top.max(pin); // the title is above, not in, what scrolls
        // Where the margin starts (floating: over the code's right end).
        let mx = width.saturating_sub(mw).saturating_sub(gw + 1);
        self.geom = mouse::Geom { relw, gw, ghw, half, ctw: if float { mx } else { ctw }, picker: None };
        let code_line = matches!(self.rows[self.head.row].line.kind, Kind::Ctx | Kind::Add | Kind::Del);
        let cur_w = match self.drows[self.hd()] {
            DRow::Pair(..) => htw_l.min(htw_r),
            DRow::Margin(..) if self.margin_side() => mtw - self.pane_pad(),
            // Split view beside the pane: a line is one half, paired or not.
            DRow::Margin(..) if self.split && code_line => htw_l.min(htw_r),
            DRow::Margin(..) => ctw,
            DRow::Full(_) => tw,
        };
        // A code line's last column is for comment marks.
        let cur_w = if code_line && !self.margin_side() { cur_w.saturating_sub(1) } else { cur_w };
        // The cursor kept in view: in a comment's code block by scrolling only that block (to
        // its box's edge, as `render_row` cuts it); elsewhere in a comment there's nothing to
        // scroll; in the diff by `left`.
        let block = self.code_block(self.head.row).filter(|_| self.box_part(self.head.row).is_some());
        if block != self.code_at {
            (self.code_at, self.code_left) = (block, 0);
        }
        let col = self.head.col;
        let follow = |left: &mut usize, w: usize| {
            *left = (*left).min(col);
            if col >= *left + w {
                *left = col + 1 - w;
            }
        };
        match (block, self.box_part(self.head.row)) {
            (Some(_), _) => {
                let w = self.box_w(self.head.row, cur_w).saturating_sub(1);
                follow(&mut self.code_left, w)
            }
            (None, Some(_)) => {}
            (None, None) => follow(&mut self.left, cur_w),
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
                    true => match &self.to {
                        Some(c) => diff::git(&["show", &format!("{c}:{}", key.0)]).unwrap_or_default(),
                        None => std::fs::read_to_string(&key.0).unwrap_or_default(),
                    },
                    false => diff::git(&["show", &format!("{}:{}", self.base, key.0)]).unwrap_or_default(),
                };
                // ponytail: whole file highlighted synchronously on first view; a thread if big PRs stutter.
                let colors = if text.len() > HL_MAX_BYTES { vec![] } else { self.hl.file(&key.0, &text) };
                self.hl_cache.insert(key, colors);
            }
        }
        let divider = Span::styled("│", self.ui.linenr);
        let lit_code = self.lit_code();
        let pair = |l: Option<usize>, rr: Option<usize>, cur: bool| {
            let half = |row: Option<usize>, right: bool, w: usize| match row {
                Some(row) => self.render_row(row, s, e, (lw, ghw, w), Some(right), cur, &overlay),
                // Like delta: a quiet hatch where the other side has no line.
                None => vec![Span::styled("╱".repeat(ghw + w), self.ui.linenr)],
            };
            let mut spans = half(l, false, htw_l);
            spans.push(divider.clone());
            spans.extend(half(rr, true, htw_r));
            spans
        };
        // Where the pane meets the code: as a column, a dim line between them; floating, the
        // pane in the popup color (on the rows it covers), with no line.
        let raised = self.ui.popup.bg.or(self.ui.bar);
        // In the key color while the cursor's in the pane: which side has focus.
        let sep = match (float, self.margin_side()) {
            (false, false) => Span::styled("│", Style { fg: self.ui.linenr.fg, ..Style::new() }),
            (false, true) => Span::styled("│", Style { fg: self.ui.key.fg, ..Style::new() }),
            (true, false) => Span::styled(" ", Style { bg: raised, ..Style::new() }),
            (true, true) => Span::styled("▎", Style { fg: self.ui.key.fg, bg: raised, ..Style::new() }),
        };
        let base_bg = self.ui.base.bg;
        let raise = |v: Vec<Span<'static>>| match (float, raised) {
            // Its own colors (the cursor line, a selection) stay; the window's give way.
            (true, Some(bg)) => v
                .into_iter()
                .map(|mut sp| {
                    if sp.style.bg.is_none() || sp.style.bg == base_bg {
                        sp.style.bg = Some(bg);
                    }
                    sp
                })
                .collect(),
            _ => v,
        };
        // As a column, a space between the line and the pane's boxes (not `││`).
        let pad = self.pane_pad();
        let ptw = mtw - pad;
        let pane_cell = |v: Vec<Span<'static>>| {
            let mut out = vec![Span::raw(" ".repeat(pad))];
            out.extend(clip(v, mw - pad));
            out
        };
        // The file the top of the screen is in, its header scrolled off: pinned over its code.
        let sticky = (|| {
            let d = self.top.min(self.drows.len().checked_sub(1)?);
            let r = match self.drows[d] {
                DRow::Full(r) => r,
                DRow::Pair(l, r) | DRow::Margin(l, r, _) => l.or(r)?,
            };
            let file = self.rows[r].file;
            (file < self.files.len()).then_some(())?;
            let head = self.rows.iter().position(|x| x.file == file && x.line.kind == Kind::File)?;
            (self.d_of[head] < self.top).then_some(head)
        })();
        let code_cols = relw + gw + if margin { mx } else { tw };
        let rows_shown = (self.top + h).min(self.drows.len()).saturating_sub(self.top);
        let lines: Vec<Line> = (0..rows_shown)
            .map(|y| {
                let d = self.top + y;
                let cur = d == hd;
                let rel = match (relw, cur) {
                    (0, _) => vec![],
                    (_, true) => vec![Span::styled(format!("{:>w$} ", "", w = relw - 1), self.ui.linenr_selected)],
                    _ => vec![Span::styled(format!("{:>w$} ", d.abs_diff(hd), w = relw - 1), self.ui.linenr)],
                };
                let mut spans = if d < self.drows.len() { rel } else { vec![] };
                // Beside the pane (as a column), full-width rows are as wide as the code.
                let column = margin && !float;
                let fw = if column { ctw } else { tw };
                match self.drows.get(d).copied().unwrap_or(DRow::Pair(None, None)) {
                    _ if d >= self.drows.len() => {}
                    DRow::Full(r) => {
                        spans.extend(self.render_row(r, s, e, (lw, gw, fw), None, cur, &overlay));
                        if column {
                            spans = clip(spans, relw + gw + mx);
                            spans.push(sep.clone());
                            spans.push(Span::raw(" ".repeat(mw)));
                        }
                    }
                    DRow::Pair(l, rr) => spans.extend(pair(l, rr, cur)),
                    DRow::Margin(l, rr, comment) => {
                        // Code on the left (blank on a spacer row), a gap, then the comment
                        // column; floating, the code runs full width under it, and is covered
                        // only where there's a comment.
                        let lit = |r: usize| cur || lit_code.is_some_and(|(a, b)| (a..=b).contains(&r));
                        let diff_line = |r: usize| matches!(self.rows[r].line.kind, Kind::Ctx | Kind::Add | Kind::Del);
                        let cw = if float { tw } else { ctw };
                        let code = match (l, rr) {
                            (None, None) => vec![],
                            (l, rr) if self.split && l.or(rr).is_some_and(diff_line) => pair(l, rr, l.or(rr).is_some_and(lit)),
                            (Some(r), _) | (None, Some(r)) => self.render_row(r, s, e, (lw, gw, cw), None, lit(r), &overlay),
                        };
                        match (comment, float) {
                            (None, true) => spans.extend(code),
                            (c, _) => {
                                spans.extend(clip(code, gw + mx));
                                spans.push(sep.clone());
                                match c {
                                    // A box open on its left: its rules start on the line, `├─`.
                                    Some(r) if self.open_left(r) && self.box_part(r).is_some_and(|p| p != BoxPart::Body) => {
                                        spans.pop();
                                        spans.push(Span::styled(format!("├{}", "─".repeat(pad)), self.box_edge(r).bg(self.ui.base.bg.unwrap_or(Color::Reset))));
                                        spans.extend(raise(clip(self.render_row(r, s, e, (lw, self.pane_gw(), ptw), None, cur, &overlay), mw - pad)));
                                    }
                                    // Not in a box (a folded thread, a heading): its mark where the
                                    // space would be, so its text lines up with the boxes'.
                                    Some(r) if self.open_left(r) && self.box_part(r).is_none() => {
                                        spans.extend(raise(clip(self.render_row(r, s, e, (lw, pad + self.pane_gw(), ptw), None, cur, &overlay), mw)));
                                    }
                                    Some(r) => spans.extend(raise(pane_cell(self.render_row(r, s, e, (lw, self.pane_gw(), ptw), None, cur, &overlay)))),
                                    None => spans.push(Span::raw(" ".repeat(mw))),
                                }
                            }
                        }
                    }
                }
                // The top line: the pinned file header over the code (the pane's part kept).
                if let (0, Some(hr)) = (y, sticky) {
                    let mut head = vec![Span::styled(" ".repeat(relw), self.ui.header)];
                    head.extend(self.render_row(hr, s, e, (lw, gw, code_cols - relw - gw), None, false, &overlay));
                    let mut out = clip(head, code_cols);
                    out.extend(skip_cols(spans, code_cols));
                    spans = out;
                }
                Line::from(spans)
            })
            .collect();
        f.render_widget(Block::new().style(self.ui.base), full);
        f.render_widget(Paragraph::new(lines), Rect { y: area.y + pin as u16, height: h as u16, ..area });
        // Below the last row: the line between the code and the pane goes on to the bottom; and
        // a view with nothing in it (yet) says so, on both sides.
        if rows_shown < h {
            let column = margin && !float;
            let code_w = if column { mx } else { tw };
            let note = self.empty_note();
            let filler: Vec<Line> = (rows_shown..h)
                .map(|y| {
                    let say = note.filter(|_| y == rows_shown + 1);
                    let code: String = say.map_or("", |n| n.0).chars().take(code_w).collect();
                    let mut spans = vec![Span::raw(" ".repeat(relw + gw)), Span::styled(format!("{code:<code_w$}"), self.ui.stale)];
                    if column {
                        spans.push(sep.clone());
                        let pane: String = say.map_or("", |n| n.1).chars().take(mw.saturating_sub(1)).collect();
                        spans.push(Span::styled(format!(" {pane}"), self.ui.stale));
                    }
                    Line::from(spans)
                })
                .collect();
            let at = area.y + (pin + rows_shown) as u16;
            f.render_widget(Paragraph::new(filler), Rect { y: at, height: (h - rows_shown) as u16, ..area });
        }
        // Like Helix's which-key: what can follow a pending prefix key, else what the keys do where
        // the cursor is (when no other popup is up). At the bottom of the contents list when it's
        // shown; else a popup (below).
        let keys = match self.pending {
            Some(p) => App::which(p),
            // The list has focus: its keys.
            None if self.files_focus && self.files_w > 0 && self.picker.is_none() && self.popup.is_none() => Some(self.list_keys()),
            None if self.picker.is_none() && self.popup.is_none() && self.prompt.is_none() => self.context_keys(),
            None => None,
        };
        if sw > 0 {
            self.draw_files(f, Rect { x: full.x, y: full.y, width: sw as u16, height: (h + pin) as u16 }, keys);
        }
        if self.pin {
            let bar = Style { bg: self.ui.bar, ..self.ui.header };
            let mut spans = vec![Span::styled(" ".repeat(relw), bar)];
            // With the pane: the title over the code, and the pane's tabs over the pane.
            let title_w = if margin { self.geom.ctw } else { tw };
            let state = self.header_tag(INFO).map_or(0, |t| t.0.chars().count() + 2);
            let line = self.rows[0].line.text.chars().count();
            self.bar_counts = line + 2 + self.pr_counts().chars().count() + state <= title_w;
            spans.extend(self.render_row(0, s, e, (lw, gw, title_w), None, self.head.row == 0, &overlay));
            if margin {
                let dim = bar.remove_modifier(ratatui::style::Modifier::BOLD).patch(Style { fg: self.ui.linenr.fg, ..Style::new() });
                let mut used = 0;
                let lit = self.bar();
                for (tab, label, cols) in self.pane_tabs() {
                    // The line between the code and the pane goes on up through the title.
                    if used == 0 && !float {
                        spans.push(Span::styled(sep.content.clone(), dim.patch(Style { fg: sep.style.fg, ..Style::new() })));
                        used = 1;
                    }
                    spans.push(Span::styled(" ".repeat(cols.start - used), dim));
                    let on = tab == lit;
                    spans.push(Span::styled(label, if on { self.ui.menu_selected } else { dim }));
                    used = cols.end;
                }
                let rest = (area.width as usize).saturating_sub(relw + gw + title_w + used);
                spans.push(Span::styled(format!("{:>rest$}", "gn ", rest = rest), dim));
            }
            f.render_widget(Paragraph::new(Line::from(spans)), Rect { height: 1, ..area });
        }

        // Link tooltip: while the cursor is on a link, show its URL just under (or over) it.
        // Not on the PR header (where dv starts): the status hint says it links there.
        let header = self.rows[self.head.row].line.kind == Kind::File;
        if let (Some(url), None, None, false) = (self.link_at_cursor(), &self.picker, &self.popup, header) {
            let y = (hd - self.top) as u16 + pin as u16;
            // In the margin, the text starts after the code column, the gap and its own gutter.
            let start = match self.drows[hd] {
                _ if self.margin_side() => relw + self.geom.gw + self.geom.ctw + 1 + self.pane_pad() + self.pane_gw(),
                DRow::Pair(..) | DRow::Margin(Some(_), Some(_), _) if self.right => relw + half + 1 + ghw,
                DRow::Pair(..) => relw + ghw,
                DRow::Margin(Some(_), Some(_), _) if self.split => relw + ghw,
                DRow::Margin(Some(l), None, _) if self.split && self.rows[l].line.kind == Kind::Del => relw + ghw,
                DRow::Margin(None, Some(_), _) if self.split => relw + half + 1 + ghw,
                _ => relw + gw,
            };
            let x = ((start + self.head.col.saturating_sub(self.hscroll(self.head.row))) as u16).min(area.width.saturating_sub(20));
            let preview = if gh::issue_ref(url).is_some() { "Space k preview · " } else { "" };
            let text = format!(" ↗ {url}  {preview}gf open · Space c copy ");
            let w = (text.chars().count() as u16).min(area.width - x);
            let ty = if y + 1 < (h + pin) as u16 { y + 1 } else { y.saturating_sub(1) };
            let r = Rect { x: area.x + x, y: area.y + ty, width: w, height: 1 };
            f.render_widget(Clear, r);
            f.render_widget(Paragraph::new(text).style(self.ui.base.patch(self.ui.popup)), r);
        }

        let status = Rect { y: area.y + (h + pin) as u16, height: 1, ..full };
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
        let pr = match self.pr_counts() {
            c if c.is_empty() || self.bar_counts => String::new(), // on the pinned title then
            c => format!("{c}  "),
        };
        let working = match &self.claude.job {
            Some(j) => format!("Claude working {}… · ", agent::ago(j.started)),
            None => String::new(),
        };
        let view = format!(
            "{}{}{}",
            match self.view {
                View::Claude => "claude view  ",
                View::Pending => "pending view  ",
                View::All => "",
            },
            if self.uncommitted() { "uncommitted  ".into() } else { self.commits_label().map(|l| format!("{l}  ")).unwrap_or_default() },
            if self.split { "split  " } else { "" }
        );
        // The left side (what you're on, messages, hints) wins: when both don't fit, the right
        // keeps only the essentials, and the left is cut with an ellipsis.
        let essential = format!("{view}{count}{pend}  {}/{} ", self.head.row + 1, self.rows.len());
        let extras = format!("{ra}{working}{pr}");
        let width = full.width as usize;
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
            let w = (full.width * 3 / 4).max(20).min(full.width);
            let ph = ((m.len() * item_h) as u16 + 3).min(full.height.saturating_sub(4)).max(3);
            let r = Rect { x: full.x + (full.width - w) / 2, y: 2.min(full.height), width: w, height: ph.min(full.height) };
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

        // The key hints, without the contents list: bottom right, as Helix has them; bottom left
        // while the cursor is in the comment margin, so they don't cover the comments there.
        if let Some((title, keys)) = keys.filter(|_| sw == 0) {
            let kw = keys.iter().map(|k| k.0.len()).max().unwrap_or(1);
            let lines: Vec<Line> = keys
                .iter()
                .map(|(k, d)| Line::from(vec![Span::styled(format!("{k:<kw$}  "), self.ui.key), Span::raw(*d)]))
                .collect();
            let w = (lines.iter().map(Line::width).max().unwrap_or(10) as u16 + 4).max(title.len() as u16 + 4).min(area.width);
            let ph = (lines.len() as u16 + 2).min(h as u16);
            let x = if self.margin_side() { area.x } else { area.x + area.width - w };
            let r = Rect { x, y: ((h + pin) as u16).saturating_sub(ph), width: w, height: ph }; // above the status bar
            let inner = self.popup_frame(f, r, title, "");
            f.render_widget(Paragraph::new(lines), inner);
        }

        if let Some((title, text, scroll)) = self.popup.clone() {
            // As wide as the text (hover is wrapped to fit 100; help may be two columns), and the title.
            let text_w = text.iter().map(Line::width).max().unwrap_or(0).max(title.chars().count() + 1);
            let w = (text_w as u16 + 3).max(30).min(full.width.saturating_sub(4));
            // ponytail: wrapped height estimated from line widths (char wrap, not word wrap).
            let cw = w.saturating_sub(3).max(1) as usize;
            let total: usize = text.iter().map(|l| l.width().div_ceil(cw).max(1)).sum();
            let h = (total as u16 + 2).min(full.height.saturating_sub(4)).max(3);
            let r = Rect { x: full.x + (full.width - w) / 2, y: 2.min(full.height), width: w, height: h.min(full.height) };
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

    /// The comment mark at the right edge of code row `r` (`side`: which half of a split row):
    /// `╎` over a comment's lines, a dot at its last, `●` others', `◆` Claude's, `○` resolved or
    /// outdated (`App::marks`).
    fn comment_mark(&self, r: usize, side: Option<bool>) -> Option<(char, Style)> {
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

    /// A box's border: the lit thread's in the accent; one from Claude's review file (Claude's,
    /// or yours for it: not on GitHub) in the hint color; GitHub's dim.
    fn box_edge(&self, r: usize) -> Style {
        let t = self.rows[r].thread;
        match t.is_some() && t == self.lit_thread() {
            true => self.ui.key,
            false if t.is_some_and(|t| self.threads[t].claude.is_some()) => Style { fg: self.ui.pending.fg, ..Style::new() },
            false => self.ui.linenr,
        }
    }

    /// Whether row `r`'s box is open on its left: in the pane, as a column (its rules come
    /// from the line between it and the code).
    fn open_left(&self, r: usize) -> bool {
        self.margin_row(r) && self.margin_active && !self.margin_float
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
                        gstyle = self.box_edge(r);
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
                // (so the cursor line's tint stays in the text column, inside the box). A lit box
                // (margin mode: the one about the cursor's line) has its border in the accent.
                if part.is_some() && row.thread.is_some() && row.thread == self.lit_thread() {
                    gstyle = ui.key;
                }
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
        // A code line keeps its last column for comment marks (`╎` over a comment's lines, a dot
        // at its last), at the edge beside the pane.
        let code_row = matches!(l.kind, Kind::Ctx | Kind::Add | Kind::Del);
        let mark = code_row.then(|| self.comment_mark(r, side)).flatten();
        let tw = if code_row { tw.saturating_sub(1) } else { tw };
        let mut spans = vec![Span::styled(gutter, gstyle)];
        let g = spans.len(); // the gutter's spans; the text's follow
        let mut n = 0;
        // A context line shows on both sides of a split row; the cursor goes on the current side.
        let this_side = side.is_none() || l.kind != Kind::Ctx || side == Some(self.right);
        let cursor_here = self.head.row == r && this_side;
        // In a box, what doesn't wrap (code) is cut at its edge, and scrolled on its own.
        let bw = self.box_w(r, tw);
        let text_w = if part.is_some() { bw.saturating_sub(1) } else { tw };
        let skip = self.hscroll(r);
        for (i, c) in l.text.chars().enumerate().skip(skip).take(text_w) {
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
        if cursor_here && self.head.col >= n.max(skip) {
            spans.push(Span::styled(" ", if self.select { ui.cursor_select } else { ui.cursor }));
        }
        // A code line cut by its box: `‹` / `›` where there's more (not over the cursor).
        if row.nowrap && part.is_some() && spans.len() > g {
            let more = Style::new().fg(ui.linenr.fg.unwrap_or(Color::DarkGray));
            let mark = |sp: &mut Span<'static>, c: &str, at: usize| {
                if !(cursor_here && self.head.col == at) {
                    (sp.content, sp.style) = (c.to_owned().into(), sp.style.patch(more));
                }
            };
            if skip > 0 {
                mark(&mut spans[g], "‹", skip);
            }
            if l.text.chars().count() > skip + text_w {
                let last = spans.len() - 1;
                mark(&mut spans[last], "›", skip + text_w - 1);
            }
        }
        // Pad to the full width (so split halves line up), and tint the whole row, so
        // added/removed lines read as bands. Spans with their own bg (the cursor) keep it.
        let used: usize = spans[g..].iter().map(|s| s.width()).sum();
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
            let edge = self.box_edge(r);
            spans.push(Span::styled(format!("{lead}{}", fill.to_string().repeat(n)), style.patch(edge)));
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
        // it (`bar_counts`; else they stay on the status bar); and, always, what Claude's doing
        // and how the checkout stands against the PR, as badges.
        let pinned = r == 0 && self.pin;
        let counts = match pinned && self.bar_counts {
            true => self.pr_counts(),
            false => String::new(),
        };
        let mut segs: Vec<(String, Style)> = vec![];
        if !counts.is_empty() {
            segs.push((counts, style.patch(ui.linenr)));
        }
        let badge = |c: Option<Color>| Style::new().fg(ui.base.bg.unwrap_or(Color::Black)).bg(c.unwrap_or(Color::Yellow)).add_modifier(ratatui::style::Modifier::BOLD);
        if pinned {
            if let Some(d) = self.claude_doing() {
                segs.push((format!(" {d} "), badge(ui.pending.fg)));
            }
            if !self.sync.is_empty() {
                segs.push((format!(" {} ", self.sync), badge(ui.warning.fg)));
            }
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
            _ => spans.push(Span::raw(" ".repeat(tw.saturating_sub(used)))),
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

/// `spans` cut to `w` columns, padded to them if shorter.
fn clip(spans: Vec<Span<'static>>, w: usize) -> Vec<Span<'static>> {
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
fn skip_cols(spans: Vec<Span<'static>>, n: usize) -> Vec<Span<'static>> {
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
    /// The PR's counts: threads (and Claude's), files viewed, comments pending.
    pub(super) fn pr_counts(&self) -> String {
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
        match &self.pr {
            Some(_) => format!("{threads} · {seen}/{on_github} viewed{pending}"),
            None if claude > 0 => threads,
            None => String::new(),
        }
    }

}
