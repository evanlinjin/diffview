//! Drawing the frame: its layout (the files' list, the code, the pane), the pinned title and
//! its tabs, the status bar, pickers and popups. One row's drawing is `row.rs`'s; what the keys
//! say, `hints.rs`'s.
use super::*;
use row::{clip, fit, skip_cols};

/// Files bigger than this aren't highlighted.
pub(super) const HL_MAX_BYTES: usize = 1 << 20;

/// A GitHub-style badge: the given background (hex, no '#'), black or white text for contrast.
pub(crate) fn chip(hex: &str) -> Style {
    let n = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0x6e7681);
    let (r, g, b) = ((n >> 16) as u8, (n >> 8) as u8, n as u8);
    let light = 0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32 > 150.0;
    let fg = if light { Color::Black } else { Color::White };
    Style::new().bg(Color::Rgb(r, g, b)).fg(fg).add_modifier(ratatui::style::Modifier::BOLD)
}

/// The frame's columns for one draw (`frame_cols`): the whole window and the part beside the
/// files' list, the pinned title, the rows' height; the code's line-number width, gutter (a
/// split half's), relative numbers' column, its text width; the pane's width and
/// text width, whether it's shown and floating; the code beside it, a split half's text
/// widths; where the pane starts; the code's text width as drawn (`code_tw`); the cursor's
/// display row.
#[derive(Clone, Copy)]
struct Cols {
    full: Rect,
    area: Rect,
    sw: usize,
    pin: usize,
    h: usize,
    lw: usize,
    gw: usize,
    ghw: usize,
    relw: usize,
    tw: usize,
    mw: usize,
    margin: bool,
    float: bool,
    ctw: usize,
    htw_l: usize,
    htw_r: usize,
    mx: usize,
    code_tw: usize,
    hd: usize,
}

impl App {
    pub(super) fn draw(&mut self, f: &mut Frame) {
        let c = self.frame_cols(f.area());
        self.scroll_to_cursor(&c);
        self.draw_body(f, &c);
        let Cols { full, area, sw, pin, h, .. } = c;
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
        self.draw_title(f, &c);
        self.draw_link_tip(f, &c);
        self.draw_status(f, Rect { y: area.y + (h + pin) as u16, height: 1, ..full });

        self.draw_picker(f, full);

        // The key hints, without the contents list: off the cursor, where they cover the least.
        if let Some((title, keys)) = keys.filter(|_| sw == 0) {
            draw_hints(f, &self.ui, Rect { y: area.y + pin as u16, height: h as u16, ..area }, title, keys, Some(self.keep_clear(&c)));
        }

        self.draw_popup(f, full);
    }


    /// Laid out again for the window as last drawn (what's beside the cursor is as it'll be).
    pub(super) fn relayout(&mut self) {
        self.frame_cols(self.full);
    }

    /// The frame's columns (`Cols`) for this draw: the files' list's, the pinned title, the code's
    /// and its gutters, the pane's (shown or not, floating or a column), as the window allows and
    /// as set; the rows rebuilt when what they wrap to changes.
    fn frame_cols(&mut self, full: Rect) -> Cols {
        self.full = full;
        // Maximized till you leave it: the code (into the files' list, or onto a comment: `]c`,
        // say), the pane (back to the code).
        let left = match self.zoom {
            Some(false) => self.files_focus || self.rows[self.head.row].thread.is_some(),
            Some(true) => self.files_focus || !self.margin_side(),
            None => false,
        };
        if left {
            self.zoom = None;
        }
        // The changed files' list on the left (`Ctrl-w e`), the rest beside it.
        let sw = if self.zoom.is_some() { 0 } else { self.files_width(full.width as usize) };
        self.files_w = sw;
        let area = Rect { x: full.x + sw as u16, width: full.width - sw as u16, ..full };
        // The PR's title line pinned at the top, as a panel over the code and the pane.
        self.pin = self.rows.first().is_some_and(|r| r.file == INFO && r.line.kind == Kind::File)
            && matches!(self.drows.first(), Some(DRow::Full(0)))
            && area.height > 4;
        let pin = self.pin as usize;
        let h = (area.height as usize).saturating_sub(1 + pin);
        self.height = h.max(1);
        self.width = full.width; // pickers and popups: over the whole window, the contents too
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
        shown &= self.zoom != Some(false);
        // A very narrow window, or the pane maximized: shown, the pane floats over all of the
        // code beside it (else a comment the cursor goes to would be nowhere).
        let tiny = width <= mw + gw + 10 || self.zoom == Some(true);
        if tiny {
            mw = width.saturating_sub(gw + 1).max(12);
        }
        let margin = shown && width > gw + 12;
        let float = margin && (tiny || self.margin_float_pref.unwrap_or(!room));
        // The code's columns: the whole width, or what's left of the margin beside it; split
        // view's halves (one line-number column each, a 1-col divider between) within that.
        let code_w = if margin && !float { width.saturating_sub(1 + mw) } else { width };
        let ctw = code_w.saturating_sub(gw).max(1);
        let half = code_w.saturating_sub(1) / 2;
        // (Both halves as wide: the left one; a spare column, the right's, left blank. A line on
        // both sides then wraps the same on each.)
        let htw_l = half.saturating_sub(ghw).max(1);
        let htw_r = htw_l;
        // PR text wraps to the code column's width (narrower beside the margin).
        let ww = if margin { ctw } else { tw }.saturating_sub(1).clamp(40, 100);
        // What long code lines wrap to (`soft_wrap`): the code's text widths, as `render_row`
        // draws them (less the last column, the comment marks'); split halves alike. Changed,
        // laid out again (once: with a rebuild, by it).
        // Where the margin starts (floating: over the code's right end).
        let mx = width.saturating_sub(mw).saturating_sub(gw + 1);
        let changed = ww != self.wrap_w || margin != self.margin_active || (margin && mw != self.margin_w) || float != self.margin_float;
        (self.wrap_w, self.margin_active, self.margin_w, self.margin_float) = (ww, margin, mw, float);
        let w = Widths {
            full: if margin && !float { ctw } else { tw },
            half: htw_l,
            pane: mw.saturating_sub(self.pane_gw() + self.pane_pad()), // (`ptw` below)
            float_at: float.then_some(relw + gw + mx),
        };
        let rewrap = (w.full, w.half) != (self.frame_w.full, self.frame_w.half) && self.soft_wrap;
        self.frame_w = w;
        if changed {
            let keep = self.cursor_key();
            self.rebuild(keep);
        } else if rewrap {
            self.relayout_keeping_top();
        }
        if !self.margin_side() {
            self.code_head = self.head;
        }
        // The code's text width: beside the pane as a column, under it floating up to where
        // it starts; the whole width without it. (What the title, a pinned header, a row cut at
        // the pane, and a click go by.)
        let code_tw = if float { mx } else { ctw };
        self.geom = mouse::Geom { relw, gw, ghw, half, ctw: code_tw, picker: None };
        // The PR's counts on its pinned title, when they fit beside all of it.
        self.bar_counts = self.pin && self.title_fits(code_tw);
        let hd = self.hd();
        Cols { full, area, sw, pin, h, lw, gw, ghw, relw, tw, mw, margin, float, ctw, htw_l, htw_r, mx, code_tw, hd }
    }

    /// The code scrolled to the cursor, up and down, and sideways: in a comment's code block by
    /// that block only, in the diff by `left`.
    fn scroll_to_cursor(&mut self, c: &Cols) {
        let Cols { pin, h, hd, float, .. } = *c;
        let so = scrolloff(h);
        // Scroll the code to the cursor.
        if hd < self.top + so {
            self.top = hd.saturating_sub(so);
        }
        if hd + so + 1 > self.top + h {
            self.top = hd + so + 1 - h;
        }
        self.top = self.top.max(pin); // the title is above, not in, what scrolls
        // What the cursor's row shows of its text (`row_w`, as drawn); floating, a comment
        // beside it covers the code from `mx` on (`draw_body`), a split half's too.
        let cur_w = self.row_w(self.head.row);
        let cur_w = match self.drows[hd] {
            // (Over all of it: nowhere to keep the cursor; as without it, not pinned to it.)
            DRow::Margin(_, _, Some(_)) if float && !self.margin_side() => match self.float_cut(self.text_x(c)).unwrap_or(0) {
                0 => cur_w,
                seen => cur_w.min(seen),
            },
            _ => cur_w,
        };
        // The cursor kept in view: in a comment's code block by scrolling only that block (to
        // its box's edge, as `render_row` cuts it); elsewhere in a comment there's nothing to
        // scroll; in the diff by `left`.
        let scroller = self.scroller(self.head.row);
        let block = if let Some(Scroller::Block(b)) = scroller { Some(b) } else { None };
        if block != self.code_at {
            (self.code_at, self.code_left) = (block, 0);
        }
        // What the cursor's in scrolls sideways (the mouse's, by `nudge`; not past the line's
        // end), and the cursor kept on screen: one scrolled off stays at the edge it went off;
        // otherwise what scrolls follows the cursor.
        let nudge = std::mem::take(&mut self.hnudge);
        let len = self.len(self.head.row);
        let w = cur_w.max(1);
        // (Wrapped, nothing's off to the side: the code scrolled back as it comes onto it.)
        let Some(scroller) = scroller else {
            if self.wraps(self.head.row) {
                self.left = 0;
            }
            return;
        };
        // (In columns, as `render_row` draws it: a tab's 4, a wide character's 2.)
        let (row, mut left) = (self.head.row, self.hscroll(self.head.row));
        if nudge != 0 {
            left = (left as isize + nudge).clamp(0, self.col_of(row, len).saturating_sub(1) as isize) as usize;
            let first = self.char_from(row, left);
            let last = self.char_at(row, left + w).saturating_sub(1).max(first);
            self.head.col = self.head.col.clamp(first, last).min(len.saturating_sub(1));
            (self.anchor, self.want_x) = (self.head, None); // (the mouse: no selection)
        }
        let (start, end) = (self.col_of(row, self.head.col), self.col_of(row, self.head.col + 1));
        left = left.min(start);
        if end > left + w {
            left = end.saturating_sub(w).min(start);
        }
        *match scroller {
            Scroller::Block(_) => &mut self.code_left,
            Scroller::Diff => &mut self.left,
        } = left;
    }

    /// The rows on screen: the code (split view's halves), the pane beside it (a column) or over
    /// it (floating), the file the top is in pinned over its code; below the last, the line
    /// between them on to the bottom, and what an empty view says.
    fn draw_body(&mut self, f: &mut Frame, c: &Cols) {
        let Cols { full, area, pin, h, lw, gw, ghw, relw, tw, mw, margin, float, ctw, htw_l, htw_r, mx, code_tw, hd, .. } = *c;
        let (s, e) = self.ordered();
        let overlay = self.label_overlay();
        self.highlight_visible();
        let divider = Span::styled("│", self.ui.linenr);
        let lit_code = self.lit_code();
        let pair = |l: Option<usize>, rr: Option<usize>, cur: bool, seg: usize| {
            let half = |row: Option<usize>, right: bool, w: usize| match row {
                Some(row) => self.render_row(row, s, e, (lw, ghw, w), Some(right), cur, &overlay, seg),
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
        // (Which side has focus: the cursor line's tint is on that side only.)
        let sep = self.pane_sep(float);
        let pane_cur = self.margin_side();
        // The cursor in the code: the box about its line has its heading tinted as the code a
        // comment's about is the other way round (the same tint, not the cursor line's).
        let lit_box = (!pane_cur && self.settings.cursorline).then(|| self.lit_thread()).flatten();
        let head_tint = |r: usize, spans: Vec<Span<'static>>| match lit_box.is_some() && self.rows[r].thread == lit_box && (r == 0 || self.rows[r - 1].thread != lit_box) {
            true => self.related_tint(spans),
            false => spans,
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
        let ptw = self.frame_w.pane;
        // The file the top of the screen is in, its header scrolled off: pinned over its code.
        let sticky = self.sticky();
        let code_cols = relw + gw + code_tw;
        let rows_shown = (self.top + h).min(self.drows.len()).saturating_sub(self.top);
        let lines: Vec<Line> = (0..rows_shown)
            .map(|y| {
                let d = self.top + y;
                // (A wrapped line's part: drawn on from where the last left off. Its parts are
                // the cursor's line, all of them; its relative number is lines', on its first.)
                let seg = self.dseg[d];
                let line = |d: usize| d - self.dseg[d];
                let cur = line(d) == line(hd);
                // The cursor line's tint: on the side with focus only (none with the list's).
                let tint = cur && !self.files_focus;
                let rel = match (relw, cur) {
                    (0, _) => vec![],
                    // (Lit as the code's line is: when it has focus.)
                    (_, true) => vec![Span::styled(format!("{:>w$} ", "", w = relw - 1), if tint && !self.margin_side() { self.ui.linenr_selected } else { self.ui.linenr })],
                    // (Screen rows, as counts move: `5j` goes to the row marked 5.)
                    _ => vec![Span::styled(format!("{:>w$} ", d.abs_diff(hd), w = relw - 1), self.ui.linenr)],
                };
                let mut spans = rel;
                // Beside the pane (as a column), full-width rows are as wide as the code.
                let column = margin && !float;
                let fw = if column { ctw } else { tw };
                match self.drows[d] {
                    DRow::Full(r) => {
                        spans.extend(self.render_row(r, s, e, (lw, gw, fw), None, tint, &overlay, seg));
                        if column {
                            spans = clip(spans, code_cols);
                            spans.push(sep.clone());
                            spans.push(Span::raw(" ".repeat(mw)));
                        }
                    }
                    DRow::Pair(l, rr) => spans.extend(pair(l, rr, tint, seg)),
                    DRow::Margin(l, rr, comment) => {
                        // Code on the left (blank on a spacer row), a gap, then the comment
                        // column; floating, the code runs full width under it, and is covered
                        // only where there's a comment.
                        let lit = tint && !pane_cur;
                        let related = |r: usize| lit_code.is_some_and(|(a, b)| (a..=b).contains(&r));
                        let diff_line = |r: usize| self.rows[r].line.kind.is_code();
                        let cw = if float { tw } else { ctw };
                        let code = match (l, rr) {
                            (None, None) => vec![],
                            (l, rr) if self.split && l.or(rr).is_some_and(diff_line) => pair(l, rr, lit, seg),
                            (Some(r), _) | (None, Some(r)) => self.render_row(r, s, e, (lw, gw, cw), None, lit, &overlay, seg),
                        };
                        let code = match l.or(rr).is_some_and(related) && self.settings.cursorline {
                            true => self.related_tint(code),
                            false => code,
                        };
                        match (comment, float) {
                            (None, true) => spans.extend(code),
                            (c, _) => {
                                spans.extend(clip(code, gw + mx));
                                spans.push(sep.clone());
                                if let Some(r) = c {
                                // As a column, boxes are open on their left (`open_left`): a box's
                                // rules start on the line, `├─`; a row in none (a folded thread, a
                                // heading) has its mark where the space would be, so its text lines
                                // up with the boxes'. Floating: a space, then the row.
                                let (gutter, w) = match (float, self.box_part(r)) {
                                    (false, Some(p)) if p != BoxPart::Body => {
                                        spans.pop();
                                        spans.push(Span::styled(format!("├{}", "─".repeat(pad)), self.ui.linenr.bg(self.ui.base.bg.unwrap_or(Color::Reset))));
                                        (self.pane_gw(), mw - pad)
                                    }
                                    (false, None) => (pad + self.pane_gw(), mw),
                                    _ => {
                                        spans.push(Span::raw(" ".repeat(pad)));
                                        (self.pane_gw(), mw - pad)
                                    }
                                };
                                spans.extend(raise(clip(head_tint(r, self.render_row(r, s, e, (lw, gutter, ptw), None, d == hd && pane_cur && !self.files_focus, &overlay, 0)), w)));
                                } else {
                                    spans.push(Span::raw(" ".repeat(mw)));
                                }
                            }
                        }
                    }
                }
                // The top line: the pinned file header over the code (the pane's part kept).
                if let (0, Some(hr)) = (y, sticky) {
                    let mut head = vec![Span::styled(" ".repeat(relw), self.ui.header)];
                    head.extend(self.render_row(hr, s, e, (lw, gw, code_cols - relw - gw), None, false, &overlay, 0));
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
    }

    /// Whether the PR's counts fit on its pinned title, `w` wide, beside all of it: its text, its
    /// state, the checkout's badge (as `render_row` puts it).
    fn title_fits(&self, w: usize) -> bool {
        let line = self.rows.first().map_or(0, |r| super::row::cols(&r.line.text));
        line + super::row::chips_w(&self.chips(0, true, Style::new())) <= w
    }

    /// Where the pane meets the code: as a column, a dim line; floating, the pane's color.
    fn pane_sep(&self, float: bool) -> Span<'static> {
        match float {
            false => Span::styled("│", Style { fg: self.ui.linenr.fg, ..Style::new() }),
            true => Span::styled(" ", Style { bg: self.ui.popup.bg.or(self.ui.bar), ..Style::new() }),
        }
    }

    /// The PR's title pinned at the top, its counts beside it when they fit (`bar_counts`); with
    /// the pane, its tabs over it.
    fn draw_title(&mut self, f: &mut Frame, c: &Cols) {
        let Cols { area, lw, gw, relw, margin, float, code_tw, .. } = *c;
        if !self.pin {
            return;
        }
        let (s, e) = self.ordered();
        let overlay = self.label_overlay();
        let sep = self.pane_sep(float);
        let bar = Style { bg: self.ui.bar, ..self.ui.header };
        let mut spans = vec![Span::styled(" ".repeat(relw), bar)];
        // With the pane: the title over the code, and the pane's tabs over the pane.
        spans.extend(self.render_row(0, s, e, (lw, gw, code_tw), None, self.head.row == 0, &overlay, 0));
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
            let rest = (area.width as usize).saturating_sub(relw + gw + code_tw + used);
            spans.push(Span::styled(format!("{:>rest$}", "gn ", rest = rest), dim));
        }
        f.render_widget(Paragraph::new(Line::from(spans)), Rect { height: 1, ..area });
    }

    /// Where the cursor line's text starts on screen: in the pane, after the code column, the
    /// gap and its own gutter; in split view, at its half's.
    fn text_x(&self, c: &Cols) -> usize {
        match self.margin_side() {
            true => self.pane_x(),
            // (The cursor's line on the half it's drawn on: as `code_x` has it for any row.)
            false => self.code_x(c.hd, self.head_right()),
        }
    }

    /// While the cursor is on a link, its URL just under (or over) it.
    fn draw_link_tip(&self, f: &mut Frame, c: &Cols) {
        let Cols { area, pin, h, hd, .. } = *c;
        // Link tooltip: while the cursor is on a link, show its URL just under (or over) it.
        // Not on the PR header (where dv starts): the status hint says it links there.
        let header = self.rows[self.head.row].line.kind == Kind::File;
        if let (Some(url), None, None, false) = (self.link_at_cursor(), &self.picker, &self.popup, header) {
            let y = (hd - self.top) as u16 + pin as u16;
            let x = ((self.text_x(c) + self.rows[self.head.row].guide_w() + self.col_of(self.head.row, self.head.col).saturating_sub(self.text_left(self.head.row, self.head.col))) as u16).min(area.width.saturating_sub(20));
            // (Just where it goes: its keys are in the key hints.)
            let text = format!(" ↗ {url} ");
            let w = (text.chars().count() as u16).min(area.width - x);
            let ty = if y + 1 < (h + pin) as u16 { y + 1 } else { y.saturating_sub(1) };
            let r = Rect { x: area.x + x, y: area.y + ty, width: w, height: 1 };
            f.render_widget(Clear, r);
            f.render_widget(Paragraph::new(text).style(self.ui.base.patch(self.ui.popup)), r);
        }

    }

    /// The status bar: the mode, where you are, the message; on the right, what's going on.
    fn draw_status(&self, f: &mut Frame, status: Rect) {
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
    let extras = format!("{ra}{pr}");
    // What Claude's doing (a run of this dv's, asking you, at work in its pane): always shown, a
    // badge in its color.
    // (It gives way, cut, to what's said on the left: half the bar at least, as it needs.)
    let width = status.width as usize;
    let cols = |s: &str| Span::raw(s.to_owned()).width();
    let doing = self.claude_doing();
    let gap = if doing.is_some() { "  " } else { "" };
    let room = width.saturating_sub(cols(&essential) + cols(gap) + left.width().min(width / 2) + 1);
    let doing = doing.map(|d| format!(" {} ", fit_badge(&d, room.saturating_sub(2)))).unwrap_or_default();
    let essentials = cols(&doing) + cols(gap) + cols(&essential);
    let extras = if left.width() + cols(&extras) + essentials < width { extras } else { String::new() };
    let right = Line::from(vec![Span::raw(extras), Span::styled(doing, badge(&self.ui, self.ui.pending.fg)), Span::raw(format!("{gap}{essential}"))]);
    let left = fit(left, width.saturating_sub(right.width() + 1));
    let bar = self.ui.base.patch(self.ui.status);
    f.render_widget(Paragraph::new(left).style(bar), status);
    f.render_widget(Paragraph::new(right).alignment(Alignment::Right).style(bar), status);

    }

    /// The picker open, centered, as wide as its items (up to most of the window): its query,
    /// then its items by section, each under its heading (when there are two or more), an icon
    /// in its color before it (`dv.icons`), what's in its last parentheses dimmed, the words
    /// the filter matched lit; items with details as cards.
    fn draw_picker(&mut self, f: &mut Frame, full: Rect) {
        let Some(p) = &self.picker else { return };
        let m = self.matches();
        let sel = p.sel.min(m.len().saturating_sub(1));
        let ui = &self.ui;
        let icons = crate::theme::icons();
        // Items are cards when they have detail lines (comment pickers): all the same height.
        let per = 1 + p.items.iter().map(|i| i.detail.len()).max().unwrap_or(0);
        let headed = p.items.iter().map(|i| i.section).collect::<HashSet<_>>().len() > 1;
        let q = p.query.to_lowercase();
        let heading = ui.heading.add_modifier(ratatui::style::Modifier::BOLD);
        // Every line, and which match it's of (a heading's, a gap's: none).
        let mut lines: Vec<(Line<'static>, Option<usize>)> = vec![];
        for (i, &it) in m.iter().enumerate() {
            let item = &p.items[it];
            let first_of = i == 0 || p.items[m[i - 1]].section != item.section;
            if headed && first_of && !item.section.is_empty() {
                if i > 0 {
                    lines.push((Line::default(), None));
                }
                lines.push((Line::from(Span::styled(format!(" {}", item.section), heading)), None));
            } else if per > 1 && i > 0 {
                lines.push((Line::default(), None)); // a blank line between cards
            }
            let st = if i == sel { ui.menu_selected } else { Style::new() };
            let (icon, tone, _) = item.target.look();
            let color = match tone {
                Tone::Plain => ui.key.fg,
                Tone::Claude => ui.pending.fg,
                Tone::Good => ui.resolved.fg,
                Tone::Danger => ui.error.fg,
                Tone::Warn => ui.warning.fg,
            };
            let st = match item.target {
                Target::Warn => st.patch(ui.warning),
                _ => st,
            };
            let mut spans = vec![Span::styled(if i == sel { "▌" } else { " " }, st.patch(ui.key))];
            // (Chosen: in the row's colors, as its text, so nothing on it is lost in the tint.
            // A space after the icon: a Nerd Font's are drawn wider than their cell.)
            if icons {
                let icon_st = if i == sel { st } else { st.fg(color.unwrap_or(Color::Reset)) };
                spans.push(Span::styled(format!("{icon}  "), icon_st));
            }
            // What the filter matched lit (where `matches` finds it); what's in the label's last
            // parentheses dimmed: what the item is, said first.
            let label: Vec<char> = item.label.chars().collect();
            let lower: Vec<char> = item.label.to_lowercase().chars().collect();
            let mut hit = vec![false; label.len()];
            for w in q.split_whitespace().map(|w| w.chars().collect::<Vec<_>>()) {
                for k in (0..lower.len().saturating_sub(w.len() - 1)).filter(|&k| lower.len() == label.len() && lower[k..].starts_with(&w)) {
                    hit[k..k + w.len()].iter_mut().for_each(|h| *h = true);
                }
            }
            let note = aside(&item.label);
            spans.extend(label.iter().enumerate().map(|(k, c)| {
                let base = if k >= note && i != sel { st.patch(ui.stale) } else { st };
                Span::styled(c.to_string(), if hit[k] { base.patch(ui.key) } else { base })
            }));
            lines.push((Line::from(spans).style(st), Some(i)));
            for k in 0..per - 1 {
                let text = item.detail.get(k).map_or(String::new(), |d| format!("   {d}"));
                lines.push((Line::from(Span::styled(text, st.patch(ui.stale))).style(st), Some(i)));
            }
        }
        // As wide as its widest line (and its title), from 40 to most of the window.
        let widest = lines.iter().map(|(l, _)| l.width()).max().unwrap_or(0).max(p.title.trim().chars().count() + 12);
        let w = (widest as u16 + 5).clamp(40.min(full.width), (full.width * 3 / 4).max(40).min(full.width));
        // The border, the query and a rule under it, the lines.
        let ph = (lines.len() as u16 + 4).min(full.height.saturating_sub(4)).max(5);
        let r = centered(full, w, ph);
        let vis = (ph as usize).saturating_sub(4).max(1);
        // Scrolled to the chosen item, all its lines (and its heading, when they fit).
        let at = |i: usize| lines.iter().position(|l| l.1 == Some(i)).unwrap_or(0);
        let (top, end) = (at(sel), at(sel) + per);
        let top = if headed && top > 0 && lines[top - 1].1.is_none() { top - 1 } else { top };
        let off = end.saturating_sub(vis).min(top);
        let pos = if m.is_empty() { "0 / 0".into() } else { format!("{} / {}", sel + 1, m.len()) };
        let inner = self.popup_frame(f, r, p.title, &pos);
        let prompt = if icons { "\u{f422} " } else { "> " };
        let query = Line::from(vec![Span::styled(prompt, self.ui.key), Span::raw(p.query.clone()), Span::styled(" ", self.cursor_style())]);
        let rule = Line::from(Span::styled("─".repeat(inner.width as usize), self.ui.linenr));
        let shown: Vec<Line> = [query, rule].into_iter().chain(lines.iter().skip(off).take(vis).map(|l| l.0.clone())).collect();
        f.render_widget(Paragraph::new(shown), inner);
        // Items start below the query and its rule.
        let list = Rect { y: inner.y + 2, height: inner.height.saturating_sub(2), ..inner };
        self.scrollbar(f, list, lines.len(), vis, off);
        self.geom.picker = Some(mouse::PickerGeom { list, off });
        let lines = lines.into_iter().map(|l| l.1).collect();
        if let Some(p) = &mut self.picker {
            p.lines = lines;
        }
    }

    /// What the key hints keep clear of, as drawn (`c`): the cursor's cell; in the comments
    /// pane, all of it (the comment being read).
    fn keep_clear(&self, c: &Cols) -> Rect {
        if self.margin_side() {
            let x = c.relw + c.gw + c.code_tw;
            return Rect { x: c.area.x + x as u16, y: c.area.y + c.pin as u16, width: c.area.width.saturating_sub(x as u16), height: c.h as u16 };
        }
        let row = self.head.row;
        let x = self.text_x(c) + self.rows[row].guide_w() + self.col_of(row, self.head.col).saturating_sub(self.text_left(row, self.head.col));
        let y = c.pin + c.hd.saturating_sub(self.top);
        Rect { x: c.area.x + x as u16, y: c.area.y + y as u16, width: 1, height: 1 }
    }

    /// The popup open (help, hover, messages), centered, as wide as its text; scrolled.
    fn draw_popup(&mut self, f: &mut Frame, full: Rect) {
        let Some((title, text, scroll)) = self.popup.clone() else { return };
        // As wide as the text (hover is wrapped to fit 100; help may be two columns), and the title.
        let text_w = text.iter().map(Line::width).max().unwrap_or(0).max(title.chars().count() + 1);
        let w = (text_w as u16 + 5).max(30).min(full.width.saturating_sub(4));
        // ponytail: wrapped height estimated from line widths (char wrap, not word wrap).
        let cw = (w as usize).saturating_sub(5).max(1);
        let total: usize = text.iter().map(|l| l.width().div_ceil(cw).max(1)).sum();
        let h = (total as u16 + 2).min(full.height.saturating_sub(4)).max(3);
        let r = centered(full, w, h);
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
        let text = code_to_edge(text, text_area.width as usize, self.ui.md.code_bg);
        f.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }).scroll((scroll, 0)), text_area);
        self.scrollbar(f, inner, total, vis, scroll as usize);
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

    pub(super) fn popup_frame(&self, f: &mut Frame, r: Rect, title: &str, pos: &str) -> Rect {
        popup_frame(&self.ui, f, r, title, pos)
    }
}

/// Which keys do what (`title`, `keys`), as Helix shows them, in a corner of `above` (bottom
/// right first, then bottom left, top right, top left): never over the cursor (`avoid`; unless
/// it's under all of them), and of the rest, where they cover the least of what's drawn. (dv
/// list's too.)
pub(crate) fn draw_hints(f: &mut Frame, ui: &Ui, above: Rect, title: &str, keys: &[(&str, &str)], avoid: Option<Rect>) {
    let kw = keys.iter().map(|k| k.0.chars().count()).max().unwrap_or(1);
    let lines: Vec<Line> = keys.iter().map(|(k, d)| Line::from(vec![Span::styled(format!("{k:<kw$}  "), ui.key), Span::raw(d.to_string())])).collect();
    let w = (lines.iter().map(Line::width).max().unwrap_or(10) as u16 + 4).max(title.chars().count() as u16 + 6).min(above.width);
    let h = (lines.len() as u16 + 2).min(above.height);
    let (right, bottom) = (above.x + above.width - w, above.y + above.height - h);
    let corners = [(right, bottom), (above.x, bottom), (right, above.y), (above.x, above.y)].map(|(x, y)| Rect { x, y, width: w, height: h });
    let buf = f.buffer_mut();
    let covered = |r: &Rect| r.positions().filter(|&p| buf.cell(p).is_some_and(|c| !c.symbol().trim().is_empty())).count();
    let r = corners.into_iter().min_by_key(|r| (avoid.is_some_and(|a| r.intersects(a)), covered(r))).unwrap_or(corners[0]);
    let inner = popup_frame(ui, f, r, title, "");
    f.render_widget(Paragraph::new(lines), inner);
}

/// `d` (a badge's text) in `w` columns: its label cut, its last word (how long it's been at
/// it: "4m") kept, so it doesn't look stuck; with no room, just its mark.
fn fit_badge(d: &str, w: usize) -> String {
    let cols = |s: &str| Span::raw(s.to_owned()).width();
    if cols(d) <= w {
        return d.to_owned();
    }
    let (head, tail) = d.rsplit_once(' ').unwrap_or((d, ""));
    let mut out = String::new();
    for c in head.chars() {
        if cols(&out) + cols(&c.to_string()) + 2 + cols(tail) > w {
            break;
        }
        out.push(c);
    }
    match out.trim_end() {
        o if o.chars().count() > 1 => format!("{o}… {tail}"),
        _ => d.chars().next().map_or(String::new(), String::from), // (the mark, ◐)
    }
}

#[test]
fn a_badge_cut_keeps_its_time() {
    let d = "◐ claude: resolving the rebase's conflicts 4m";
    assert_eq!(fit_badge(d, 80), d);
    let cut = fit_badge(d, 24);
    assert!(cut.ends_with("… 4m") && Span::raw(cut.clone()).width() <= 24, "{cut}");
    assert_eq!(fit_badge(d, 3), "◐");
}

/// A badge (what Claude's doing, how the checkout stands): bold, in `color`, on the window's.
pub(super) fn badge(ui: &Ui, color: Option<Color>) -> Style {
    Style::new().fg(ui.base.bg.unwrap_or(Color::Black)).bg(color.unwrap_or(Color::Yellow)).add_modifier(ratatui::style::Modifier::BOLD)
}

/// A popup as zellij floats a pane: the window's background in a rounded border, the
/// title set in its top (`pos` at its right). Returns the area inside, a column in from each
/// side. (dv list's too.)
pub(crate) fn popup_frame(ui: &Ui, f: &mut Frame, r: Rect, title: &str, pos: &str) -> Rect {
    f.render_widget(Clear, r);
    // A long title gives way to the position on the right.
    let room = (r.width as usize).saturating_sub(pos.chars().count() + 8);
    let title = title.trim();
    let title = match title.chars().count() > room {
        true => format!("{}…", title.chars().take(room.saturating_sub(1)).collect::<String>()),
        false => title.to_owned(),
    };
    let edge = Style::new().fg(ui.key.fg.unwrap_or(Color::Reset));
    let mut block = Block::bordered()
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(edge)
        .style(ui.base)
        .title(Span::styled(format!(" {title} "), ui.key));
    if !pos.is_empty() {
        block = block.title(ratatui::text::Line::from(Span::styled(format!(" {pos} "), ui.linenr)).right_aligned());
    }
    let inner = block.inner(r);
    f.render_widget(block, r);
    Rect { x: inner.x + 1, width: inner.width.saturating_sub(2), ..inner }
}

/// A popup's code blocks' lines (`hover_lines` gives them `code_bg` as their line's style) with
/// their background to its edge, `w`, as in a comment's box.
pub(super) fn code_to_edge(text: Vec<Line<'static>>, w: usize, code_bg: Option<Color>) -> Vec<Line<'static>> {
    let Some(bg) = code_bg else { return text };
    text.into_iter()
        .map(|mut l| {
            if l.style.bg == Some(bg) {
                let pad = w.saturating_sub(l.width());
                l.spans.push(Span::styled(" ".repeat(pad), Style::new().bg(bg)));
            }
            l
        })
        .collect()
}

/// Where `label`'s last parenthetical starts (`…` after it too), what says more about the
/// item than what it is; else its end.
fn aside(label: &str) -> usize {
    let chars: Vec<char> = label.trim_end_matches('…').chars().collect();
    if chars.last() != Some(&')') {
        return label.chars().count();
    }
    let mut depth = 0;
    for (k, &c) in chars.iter().enumerate().rev() {
        depth += match c {
            ')' => 1,
            '(' => -1,
            _ => 0,
        };
        if depth == 0 {
            // (Not one that's all of it.)
            return if k > 0 { k } else { label.chars().count() };
        }
    }
    label.chars().count()
}

/// The text width a popup has in a window `width` wide: its margins (2 a side), border,
/// padding and scrollbar.
pub(super) fn popup_text_w(width: u16) -> usize {
    width.saturating_sub(9) as usize
}

/// A popup's place: centered in `full`, near its top, `w` × `h` (as much as fits).
fn centered(full: Rect, w: u16, h: u16) -> Rect {
    Rect { x: full.x + (full.width - w) / 2, y: full.y + 2.min(full.height), width: w, height: h }.intersection(full)
}

/// How many rows the cursor keeps from the top and bottom of `h` rows (Helix's scrolloff).
pub(super) fn scrolloff(h: usize) -> usize {
    3.min(h / 2)
}

impl App {
    /// What jump labels show (`gw`): each label's two chars at its word, or after the first key
    /// pressed, the remaining one.
    fn label_overlay(&self) -> HashMap<Pos, char> {
        self.labels
            .iter()
            .filter(|l| self.label_first.is_none_or(|f| l.2[0] == f))
            .flat_map(|&(p, _, [a, b])| match self.label_first {
                None => vec![(p, a), (Pos { col: p.col + 1, ..p }, b)],
                Some(_) => vec![(p, b)],
            })
            .collect()
    }

    /// The files of the rows on screen highlighted, those not yet (cached by revision and path).
    fn highlight_visible(&mut self) {
        for r in self.visible_rows() {
            if let Some(key) = self.hl_key(r).filter(|k| !self.hl_cache.contains_key(k)) {
                let (rev, path) = &key;
                let text = match rev.as_str() {
                    "" => std::fs::read_to_string(path).unwrap_or_default(),
                    rev => diff::git(&["show", &format!("{rev}:{path}")]).unwrap_or_default(),
                };
                // ponytail: whole file highlighted synchronously on first view; a thread if big PRs stutter.
                let colors = if text.len() > HL_MAX_BYTES { vec![] } else { self.hl.file(path, &text) };
                self.hl_cache.insert(key, colors);
            }
        }
    }

    /// The PR's counts: threads (and Claude's), files viewed, comments pending.
    pub(super) fn pr_counts(&self) -> String {
        let pending = match self.pending_count() {
            0 if self.review.is_none() => String::new(),
            n => format!(" · {n} pending (S submits)"),
        };
        let claude = self.threads.iter().filter(|t| t.by_claude()).count();
        let review_file = self.threads.iter().filter(|t| t.claude.is_some()).count();
        let seen = self.files.iter().filter(|f| self.viewed.get(&f.path).is_some_and(|s| s == "VIEWED")).count();
        let on_github = self.files.iter().filter(|f| f.meta != "untracked").count(); // what "viewed" can be of
        let threads = match (self.threads.len() - review_file, claude) {
            (n, 0) => format!("{n} threads"),
            (n, c) => format!("{n} threads + {c} Claude"),
        };
        // (The pane's tabs shown, on the pinned title: they count the threads, one way.)
        let tabs = self.pin && self.margin_active;
        match &self.pr {
            Some(_) if tabs => format!("{seen}/{on_github} viewed{pending}"),
            Some(_) => format!("{threads} · {seen}/{on_github} viewed{pending}"),
            None if claude > 0 && !tabs => threads,
            None => String::new(),
        }
    }

}

impl App {
    /// Rows on screen, each once (a context line shows on both sides but counts once).
    pub(super) fn visible_rows(&self) -> Vec<usize> {
        let mut out = vec![];
        for d in &self.drows[self.top.min(self.drows.len())..(self.top + self.height).min(self.drows.len())] {
            match *d {
                DRow::Full(r) => out.push(r),
                DRow::Pair(l, r) => out.extend(l.into_iter().chain(r.filter(|&r| Some(r) != l))),
                DRow::Margin(l, r, c) => out.extend(l.into_iter().chain(r.filter(|&r| Some(r) != l)).chain(c)),
            }
        }
        out.dedup(); // (a wrapped line's parts: once)
        out
    }

    /// Which highlighted file a code row's colors come from: the base for removed lines, else the working tree.
    pub(super) fn hl_key(&self, r: usize) -> Option<(String, String)> {
        let row = &self.rows[r];
        let f = self.files.get(row.file).filter(|f| f.meta != "binary")?;
        match row.line.kind {
            Kind::Del => Some((self.base.clone(), f.old_path.clone())),
            Kind::Add | Kind::Ctx => Some((self.to.clone().unwrap_or_default(), f.path.clone())),
            _ => None,
        }
    }

    /// The working tree's colors forgotten: its files may have changed (the base's and a
    /// commit's can't).
    pub(super) fn forget_tree_colors(&mut self) {
        // ("HEAD", the uncommitted view's base, moves with a commit or an amend too.)
        self.hl_cache.retain(|k, _| !k.0.is_empty() && k.0 != "HEAD");
    }
}
