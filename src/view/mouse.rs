//! Mouse: the wheel scrolls (the view, a popup, or a picker's selection), a click moves the
//! cursor there (or folds, on a fold arrow; or picks, in a picker), and a drag selects.
use super::*;
use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

/// Where the last frame put things, to map a click back to a row and column.
#[derive(Default, Clone, Copy)]
pub(super) struct Geom {
    /// Relative line numbers' column, the gutter, a split half's gutter and the half's width,
    /// and the code column's width beside the comment margin (or under it, floating: up to it).
    pub(super) relw: usize,
    pub(super) gw: usize,
    pub(super) ghw: usize,
    pub(super) half: usize,
    pub(super) ctw: usize,
    pub(super) picker: Option<PickerGeom>,
}

#[derive(Clone, Copy)]
pub(super) struct PickerGeom {
    /// The items' area, and the first of its lines shown (`Picker::lines`).
    pub(super) list: Rect,
    pub(super) off: usize,
}

/// The wheel as it scrolls: sideways with Shift, Alt or Ctrl held (for a wheel that only goes
/// up and down; kitty keeps Shift and the mouse for its own selection).
pub(crate) fn sideways(m: &MouseEvent) -> MouseEventKind {
    let held = m.modifiers.intersects(KeyModifiers::SHIFT | KeyModifiers::ALT | KeyModifiers::CONTROL);
    match m.kind {
        MouseEventKind::ScrollDown if held => MouseEventKind::ScrollRight,
        MouseEventKind::ScrollUp if held => MouseEventKind::ScrollLeft,
        k => k,
    }
}

impl App {
    pub(super) fn mouse(&mut self, m: MouseEvent) {
        let (x, y) = (m.column as usize, m.row as usize);
        // The files' list on the left: a click goes to a file, the wheel scrolls it; the rest is
        // laid out beside it.
        if x < self.files_w && self.picker.is_none() && self.popup.is_none() {
            match m.kind {
                MouseEventKind::Down(MouseButton::Left) => self.click_file(y, m.modifiers.contains(KeyModifiers::SHIFT)),
                MouseEventKind::ScrollDown => self.scroll_files(3),
                MouseEventKind::ScrollUp => self.scroll_files(-3),
                _ => {}
            }
            return;
        }
        let x = if self.picker.is_some() || self.popup.is_some() { x } else { x.saturating_sub(self.files_w) };
        // Sideways over the code (a popup or a picker scrolls as with the wheel).
        let code = self.picker.is_none() && self.popup.is_none();
        let kind = if code { sideways(&m) } else { m.kind };
        match kind {
            // Sideways: what's under it (a comment's code block or table, or the diff), the
            // cursor to it first, as a click. Not what doesn't scroll (a comment's prose, the
            // pinned title: the cursor stays), nor a line too short to show where it's scrolled
            // to (the cursor there would take it back).
            MouseEventKind::ScrollLeft | MouseEventKind::ScrollRight if code => {
                let Some((row, col, _)) = self.at_screen(x, y) else { return };
                if self.scroller(row).is_none() || kind == MouseEventKind::ScrollRight && self.col_of(row, self.len(row)) <= self.hscroll(row) + 1 && self.hscroll(row) > 0 {
                    return;
                }
                self.select = false;
                self.goto(row, col);
                self.hnudge = if kind == MouseEventKind::ScrollRight { 8 } else { -8 };
            }
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                let down = m.kind == MouseEventKind::ScrollDown;
                if let Some((_, _, scroll)) = &mut self.popup {
                    *scroll = if down { scroll.saturating_add(3) } else { scroll.saturating_sub(3) };
                } else if self.picker.is_some() {
                    let n = self.matches().len();
                    if let Some(p) = &mut self.picker {
                        p.sel = if down { (p.sel + 1).min(n.saturating_sub(1)) } else { p.sel.saturating_sub(1) };
                    }
                } else {
                    // What's under the mouse, wherever the cursor is, and only that: each stops at
                    // its own end. The comments with the code (they're beside their lines), while
                    // there are comments that way.
                    let d = if down { 3 } else { -3 };
                    let over_pane = self.margin_active && x > self.geom.relw + self.geom.gw + self.geom.ctw;
                    if !over_pane || self.comments_beyond(down) {
                        _ = self.scroll_code(d);
                    }
                }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if self.picker.is_some() {
                    return self.click_picker(y);
                }
                if self.popup.is_some() {
                    return (self.popup, self.help) = (None, None); // a click elsewhere closes it
                }
                // On the pinned title, over the pane: its tabs.
                let pane = self.geom.relw + self.geom.gw + self.geom.ctw;
                if self.pin && y == 0 && self.margin_active && x >= pane {
                    if let Some((b, ..)) = self.pane_tabs().into_iter().find(|t| t.2.contains(&(x - pane))) {
                        self.bar_to(b);
                    }
                    return;
                }
                let Some((row, col, gutter)) = self.clicked_at(x, y) else { return };
                // On a fold arrow (a file's or a thread's first row, in the gutter): fold / unfold.
                let first_of_thread = self.rows[row].thread.is_some_and(|t| row == 0 || self.rows[row - 1].thread != Some(t));
                self.select = false;
                self.goto(row, col);
                match (gutter, self.rows[row].line.kind) {
                    (true, Kind::File) if !(row == 0 && self.pin) => self.toggle_collapse(), // pinned: no fold
                    (true, Kind::Thread) if first_of_thread => self.toggle_thread(),
                    _ => {}
                }
            }
            // Dragging selects, from where the button went down (the anchor stays there).
            MouseEventKind::Drag(MouseButton::Left) if self.picker.is_none() && self.popup.is_none() => {
                if let Some((row, col, _)) = self.at_screen(x, y) {
                    self.set_head(Pos { row, col: col.min(self.len(row).saturating_sub(1)) });
                }
            }
            _ => {}
        }
    }

    /// In a picker: a click picks that item; on the one already picked, opens it (as Enter).
    fn click_picker(&mut self, y: usize) {
        let Some(g) = self.geom.picker else { return };
        let (top, h) = (g.list.y as usize, g.list.height as usize);
        if y < top || y >= top + h {
            return self.picker = None; // outside its items: close it, as Esc
        }
        let Some(p) = &mut self.picker else { return };
        // (A section's heading, a gap: nothing.)
        let Some(i) = p.lines.get(g.off + y - top).copied().flatten() else { return };
        match p.sel == i {
            true => self.picker_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            false => p.sel = i,
        }
    }

    /// Where the comment pane's text starts on screen: after the code column and a 1-column
    /// gap, behind its own gutter (as the last frame drew it).
    pub(super) fn pane_x(&self) -> usize {
        let g = self.geom;
        g.relw + g.gw + g.ctw + 1 + self.pane_pad() + self.pane_gw()
    }

    /// Where a split view's right (new) side's text starts on screen.
    pub(super) fn right_x(&self) -> usize {
        let g = self.geom;
        g.relw + g.half + 1 + g.ghw
    }

    /// What a click at screen cell (`x`, `y`) goes to: as `at_screen`, but on the file header
    /// pinned over the top, the header (not what it covers: a drag or the wheel there is still
    /// about the code under it).
    pub(super) fn clicked_at(&self, x: usize, y: usize) -> Option<(usize, usize, bool)> {
        let g = self.geom;
        let pinned = (y == self.pin as usize && x < g.relw + g.gw + g.ctw).then(|| self.sticky()).flatten();
        pinned.map(|h| (h, 0, x < g.relw + g.gw)).or_else(|| self.at_screen(x, y))
    }

    /// The row and column under screen cell (`x`, `y`), and whether it's in the gutter (line
    /// numbers, fold arrows), as the last frame drew it.
    pub(super) fn at_screen(&self, x: usize, y: usize) -> Option<(usize, usize, bool)> {
        let g = self.geom;
        if self.pin && y == 0 {
            let start = g.relw + g.gw; // the pinned title (and the pane's tabs, `click_tab`)
            return Some((0, x.saturating_sub(start), x < start));
        }
        let y = y - self.pin as usize;
        if y >= self.height {
            return None; // the status bar
        }
        let d = self.top + y;
        // In the pane: the comment there.
        if let Some(note) = self.note_at(d).filter(|_| x > g.relw + g.gw + g.ctw) {
            let s = self.span_of(note, 0);
            let at = x.saturating_sub(self.pane_x() + self.rows[note].guide_w());
            return Some((note, self.clicked(note, s.as_ref(), at), x < self.pane_x()));
        }
        // The code: the side clicked (split view; a hatch, no line there: the other's), what
        // that screen row shows of it (`span`), the click kept within it. A spacer: the comment
        // beside it.
        let paired = matches!(self.drows.get(d)?, &(DRow::Pair(l, r) | DRow::Margin(l, r, _)) if self.paired(l, r));
        let side = paired && x >= g.relw + g.half;
        let side = if self.side_row(d, side).is_some() { side } else { !side };
        let Some(row) = self.side_row(d, side) else {
            let note = self.note_at(d)?;
            return Some((note, 0, false));
        };
        let start = self.code_x(d, side) + self.rows[row].guide_w();
        let s = self.span(d, side);
        Some((row, self.clicked(row, s.as_ref(), x.saturating_sub(start)), x < self.code_x(d, side)))
    }

}
