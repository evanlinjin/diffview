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
    /// The items' area, how many lines each takes, and the first one shown.
    pub(super) list: Rect,
    pub(super) item_h: usize,
    pub(super) off: usize,
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
        match m.kind {
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
                let Some((row, col, gutter)) = self.at_screen(x, y) else { return };
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
                    self.head = Pos { row, col: col.min(self.len(row).saturating_sub(1)) };
                    self.want_col = self.head.col;
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
        let i = g.off + (y - top) / g.item_h.max(1);
        let n = self.matches().len();
        let Some(p) = &mut self.picker else { return };
        if i >= n {
            return;
        }
        match p.sel == i {
            true => self.picker_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            false => p.sel = i,
        }
    }

    /// The row and column under screen cell (`x`, `y`), and whether it's in the gutter (line
    /// numbers, fold arrows), as the last frame drew it.
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
        let (row, start) = match *self.drows.get(d)? {
            DRow::Full(r) => (r, g.relw + g.gw),
            // Split view: the old side, the divider, the new side.
            DRow::Pair(l, r) if x < g.relw + g.half => (l.or(r)?, g.relw + g.ghw),
            DRow::Pair(l, r) => (r.or(l)?, self.right_x()),
            DRow::Margin(_, _, Some(note)) if x > g.relw + g.gw + g.ctw => (note, self.pane_x()),
            // Split view's code beside it: as a pair, but a hunk header spans both sides.
            DRow::Margin(l, r, _) if self.split && l.or(r).is_some_and(|c| self.rows[c].line.kind.is_code()) => {
                match x < g.relw + g.half {
                    true => (l.or(r)?, g.relw + g.ghw),
                    false => (r.or(l)?, self.right_x()),
                }
            }
            DRow::Margin(l, r, note) => (l.or(r).or(note)?, g.relw + g.gw),
        };
        Some((row, x.saturating_sub(start) + self.hscroll(row), x < start))
    }

}
