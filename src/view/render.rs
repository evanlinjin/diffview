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
    mtw: usize,
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

        // The key hints, without the contents list: bottom right, as Helix has them; bottom left
        // while the cursor is in the comment margin, so they don't cover the comments there.
        if let Some((title, keys)) = keys.filter(|_| sw == 0) {
            self.draw_key_popup(f, Rect { y: area.y + pin as u16, height: h as u16, ..area }, title, keys);
        }

        self.draw_popup(f, full);
    }


    /// The frame's columns (`Cols`) for this draw: the files' list's, the pinned title, the code's
    /// and its gutters, the pane's (shown or not, floating or a column), as the window allows and
    /// as set; the rows rebuilt when what they wrap to changes.
    fn frame_cols(&mut self, full: Rect) -> Cols {
        // The changed files' list on the left (`Ctrl-w e`), the rest beside it.
        let sw = self.files_width(full.width as usize);
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
        // A very narrow window: shown, the pane floats over all of the code beside it (else a
        // comment the cursor goes to would be nowhere).
        let tiny = width <= mw + gw + 10;
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
        let (htw_l, htw_r) = (half.saturating_sub(ghw).max(1), code_w.saturating_sub(1 + half).saturating_sub(ghw).max(1));
        // PR text wraps to the code column's width (narrower beside the margin).
        let ww = if margin { ctw } else { tw }.saturating_sub(1).clamp(40, 100);
        if ww != self.wrap_w || margin != self.margin_active || (margin && mw != self.margin_w) || float != self.margin_float {
            (self.wrap_w, self.margin_active, self.margin_w, self.margin_float) = (ww, margin, mw, float);
            let keep = self.cursor_key();
            self.rebuild(keep);
        }
        let mtw = mw - self.pane_gw();
        if !self.margin_side() {
            self.code_head = self.head;
        }
        // Where the margin starts (floating: over the code's right end).
        let mx = width.saturating_sub(mw).saturating_sub(gw + 1);
        // The code's text width: beside the pane as a column, under it floating up to where
        // it starts; the whole width without it. (What the title, a pinned header, a row cut at
        // the pane, and a click go by.)
        let code_tw = if float { mx } else { ctw };
        self.geom = mouse::Geom { relw, gw, ghw, half, ctw: code_tw, picker: None };
        // The PR's counts on its pinned title, when they fit beside all of it.
        self.bar_counts = self.pin && self.title_fits(code_tw);
        let hd = self.hd();
        Cols { full, area, sw, pin, h, lw, gw, ghw, relw, tw, mw, mtw, margin, float, ctw, htw_l, htw_r, mx, code_tw, hd }
    }

    /// The code scrolled to the cursor, up and down, and sideways: in a comment's code block by
    /// that block only, in the diff by `left`.
    fn scroll_to_cursor(&mut self, c: &Cols) {
        let Cols { pin, h, tw, mtw, ctw, htw_l, htw_r, hd, float, mx, .. } = *c;
        let so = scrolloff(h);
        // Scroll the code to the cursor.
        if hd < self.top + so {
            self.top = hd.saturating_sub(so);
        }
        if hd + so + 1 > self.top + h {
            self.top = hd + so + 1 - h;
        }
        self.top = self.top.max(pin); // the title is above, not in, what scrolls
        let code_line = self.rows[self.head.row].line.kind.is_code();
        let cur_w = match self.drows[hd] {
            DRow::Pair(..) => htw_l.min(htw_r),
            DRow::Margin(..) if self.margin_side() => mtw - self.pane_pad(),
            // Split view beside the pane: a line is one half, paired or not.
            DRow::Margin(..) if self.split && code_line => htw_l.min(htw_r),
            // Floating, a comment beside it covers the code from `mx` on (`draw_body`).
            DRow::Margin(_, _, Some(_)) if float => mx,
            DRow::Margin(..) => ctw,
            DRow::Full(_) => tw,
        };
        // A code line's last column is for comment marks.
        let cur_w = if code_line && !self.margin_side() { cur_w.saturating_sub(1) } else { cur_w };
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
        let w = self.text_w(self.head.row, cur_w).max(1);
        let left = match scroller {
            Some(Scroller::Block(_)) => &mut self.code_left,
            Some(Scroller::Diff) => &mut self.left,
            None => return,
        };
        if nudge != 0 {
            *left = (*left as isize + nudge).clamp(0, len.saturating_sub(1) as isize) as usize;
            self.head.col = self.head.col.clamp(*left, *left + w - 1).min(len.saturating_sub(1));
            (self.anchor, self.want_col) = (self.head, self.head.col); // (the mouse: no selection)
        }
        *left = (*left).min(self.head.col);
        if self.head.col >= *left + w {
            *left = self.head.col + 1 - w;
        }
    }

    /// The rows on screen: the code (split view's halves), the pane beside it (a column) or over
    /// it (floating), the file the top is in pinned over its code; below the last, the line
    /// between them on to the bottom, and what an empty view says.
    fn draw_body(&mut self, f: &mut Frame, c: &Cols) {
        let Cols { full, area, pin, h, lw, gw, ghw, relw, tw, mw, mtw, margin, float, ctw, htw_l, htw_r, mx, code_tw, hd, .. } = *c;
        let (s, e) = self.ordered();
        let overlay = self.label_overlay();
        self.highlight_visible();
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
        let ptw = mtw - pad;
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
        let code_cols = relw + gw + code_tw;
        let rows_shown = (self.top + h).min(self.drows.len()).saturating_sub(self.top);
        let lines: Vec<Line> = (0..rows_shown)
            .map(|y| {
                let d = self.top + y;
                let cur = d == hd;
                // The cursor line's tint: on the side with focus only (none with the list's).
                let tint = cur && !self.files_focus;
                let rel = match (relw, cur) {
                    (0, _) => vec![],
                    // (Lit as the code's line is: when it has focus.)
                    (_, true) => vec![Span::styled(format!("{:>w$} ", "", w = relw - 1), if tint && !self.margin_side() { self.ui.linenr_selected } else { self.ui.linenr })],
                    _ => vec![Span::styled(format!("{:>w$} ", d.abs_diff(hd), w = relw - 1), self.ui.linenr)],
                };
                let mut spans = rel;
                // Beside the pane (as a column), full-width rows are as wide as the code.
                let column = margin && !float;
                let fw = if column { ctw } else { tw };
                match self.drows[d] {
                    DRow::Full(r) => {
                        spans.extend(self.render_row(r, s, e, (lw, gw, fw), None, tint, &overlay));
                        if column {
                            spans = clip(spans, code_cols);
                            spans.push(sep.clone());
                            spans.push(Span::raw(" ".repeat(mw)));
                        }
                    }
                    DRow::Pair(l, rr) => spans.extend(pair(l, rr, tint)),
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
                            (l, rr) if self.split && l.or(rr).is_some_and(diff_line) => pair(l, rr, lit),
                            (Some(r), _) | (None, Some(r)) => self.render_row(r, s, e, (lw, gw, cw), None, lit, &overlay),
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
                                spans.extend(raise(clip(head_tint(r, self.render_row(r, s, e, (lw, gutter, ptw), None, tint && pane_cur, &overlay)), w)));
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
    }

    /// Whether the PR's counts fit on its pinned title, `w` wide, beside all of it: its text, its
    /// state, Claude's and the checkout's badges (as `render_row` puts them).
    fn title_fits(&self, w: usize) -> bool {
        let badges = self.claude_doing().map(|d| d.chars().count() + 2).into_iter().chain((!self.sync.is_empty()).then(|| self.sync.chars().count() + 2));
        let state = self.header_tag(INFO).map(|t| t.0.chars().count()).into_iter().chain(badges).map(|n| n + 2).sum::<usize>();
        let line = self.rows.first().map_or(0, |r| r.line.text.chars().count());
        line + 2 + self.pr_counts().chars().count() + state <= w
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
        spans.extend(self.render_row(0, s, e, (lw, gw, code_tw), None, self.head.row == 0, &overlay));
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

    /// While the cursor is on a link, its URL just under (or over) it.
    fn draw_link_tip(&self, f: &mut Frame, c: &Cols) {
        let Cols { area, pin, h, gw, ghw, relw, hd, .. } = *c;
        // Link tooltip: while the cursor is on a link, show its URL just under (or over) it.
        // Not on the PR header (where dv starts): the status hint says it links there.
        let header = self.rows[self.head.row].line.kind == Kind::File;
        if let (Some(url), None, None, false) = (self.link_at_cursor(), &self.picker, &self.popup, header) {
            let y = (hd - self.top) as u16 + pin as u16;
            // In the margin, the text starts after the code column, the gap and its own gutter.
            let start = match self.drows[hd] {
                _ if self.margin_side() => self.pane_x(),
                DRow::Pair(..) | DRow::Margin(Some(_), Some(_), _) if self.right => self.right_x(),
                DRow::Pair(..) => relw + ghw,
                DRow::Margin(Some(_), Some(_), _) if self.split => relw + ghw,
                DRow::Margin(Some(l), None, _) if self.split && self.rows[l].line.kind == Kind::Del => relw + ghw,
                DRow::Margin(None, Some(_), _) if self.split => self.right_x(),
                _ => relw + gw,
            };
            let x = ((start + self.rows[self.head.row].guide_w() + self.head.col.saturating_sub(self.hscroll(self.head.row))) as u16).min(area.width.saturating_sub(20));
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
    let width = status.width as usize;
    let right = match left.width() + extras.chars().count() + essential.chars().count() < width {
        true => format!("{extras}{essential}"),
        false => essential,
    };
    let left = fit(left, width.saturating_sub(right.chars().count() + 1));
    let bar = self.ui.base.patch(self.ui.status);
    f.render_widget(Paragraph::new(left).style(bar), status);
    f.render_widget(Paragraph::new(right).alignment(Alignment::Right).style(bar), status);

    }

    /// The picker open, centered: its query, then its items (cards, with details), the words the
    /// filter matched lit.
    fn draw_picker(&mut self, f: &mut Frame, full: Rect) {
        let Some(p) = &self.picker else { return };
        let m = self.matches();
        let sel = p.sel.min(m.len().saturating_sub(1));
        // Items are cards when they have detail lines (comment pickers): all the same height.
        let per = 1 + p.items.iter().map(|i| i.detail.len()).max().unwrap_or(0);
        let gap = (per > 1) as usize; // a blank line between cards
        let item_h = per + gap;
        let w = (full.width * 3 / 4).max(20).min(full.width);
        let ph = ((m.len() * item_h) as u16 + 3).min(full.height.saturating_sub(4)).max(3);
        let r = centered(full, w, ph);
        let vis = ((ph as usize).saturating_sub(3) / item_h).max(1);
        let off = sel.saturating_sub(vis.saturating_sub(1));
        let mut lines = vec![Line::from(format!("> {}", p.query))];
        let q = p.query.to_lowercase();
        for (i, &it) in m.iter().enumerate().skip(off).take(vis) {
            let st = if i == sel { self.ui.menu_selected } else { Style::new() };
            // Highlight the words the filter matched, where they are (as `matches` checks).
            let label: Vec<char> = p.items[it].label.chars().collect();
            let lower: Vec<char> = p.items[it].label.to_lowercase().chars().collect();
            let mut hit = vec![false; label.len()];
            for w in q.split_whitespace().map(|w| w.chars().collect::<Vec<_>>()) {
                for k in (0..lower.len().saturating_sub(w.len() - 1)).filter(|&k| lower.len() == label.len() && lower[k..].starts_with(&w)) {
                    hit[k..k + w.len()].iter_mut().for_each(|h| *h = true);
                }
            }
            let spans: Vec<Span> = label.iter().zip(&hit).map(|(c, &h)| Span::styled(c.to_string(), if h { st.patch(self.ui.key) } else { st })).collect();
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

    /// Which keys do what here (`title`, `keys`): bottom right of `above` (the code, above the
    /// status bar), as Helix has them; bottom left while the cursor is in the comment margin, so
    /// they don't cover the comments there.
    fn draw_key_popup(&self, f: &mut Frame, above: Rect, title: &str, keys: &[(&str, &str)]) {
        let kw = keys.iter().map(|k| k.0.len()).max().unwrap_or(1);
        let lines: Vec<Line> = keys
            .iter()
            .map(|(k, d)| Line::from(vec![Span::styled(format!("{k:<kw$}  "), self.ui.key), Span::raw(*d)]))
            .collect();
        let w = (lines.iter().map(Line::width).max().unwrap_or(10) as u16 + 4).max(title.len() as u16 + 4).min(above.width);
        let ph = (lines.len() as u16 + 2).min(above.height);
        let x = if self.margin_side() { above.x } else { above.x + above.width - w };
        let r = Rect { x, y: above.y + above.height.saturating_sub(ph), width: w, height: ph };
        let inner = self.popup_frame(f, r, title, "");
        f.render_widget(Paragraph::new(lines), inner);
    }

    /// The popup open (help, hover, messages), centered, as wide as its text; scrolled.
    fn draw_popup(&mut self, f: &mut Frame, full: Rect) {
        let Some((title, text, scroll)) = self.popup.clone() else { return };
        // As wide as the text (hover is wrapped to fit 100; help may be two columns), and the title.
        let text_w = text.iter().map(Line::width).max().unwrap_or(0).max(title.chars().count() + 1);
        let w = (text_w as u16 + 3).max(30).min(full.width.saturating_sub(4));
        // ponytail: wrapped height estimated from line widths (char wrap, not word wrap).
        let cw = w.saturating_sub(3).max(1) as usize;
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

/// Helix-style popup: no border, the theme's popup background, the title as a header row in the
/// header color (`pos` at its right). Returns the padded content area. (dv list's too.)
pub(crate) fn popup_frame(ui: &Ui, f: &mut Frame, r: Rect, title: &str, pos: &str) -> Rect {
    f.render_widget(Clear, r);
    f.render_widget(Block::new().style(ui.base.patch(ui.popup)), r);
    let head = Rect { height: 1.min(r.height), ..r };
    let head_style = ui.base.patch(ui.header);
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

/// A popup's place: centered in `full`, near its top, `w` × `h` (as much as fits).
fn centered(full: Rect, w: u16, h: u16) -> Rect {
    Rect { x: full.x + (full.width - w) / 2, y: 2.min(full.height), width: w, height: h.min(full.height) }
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
