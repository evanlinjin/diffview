//! Mouse: the wheel scrolls (the view, a popup, or a picker's selection), a click moves the
//! cursor there (or folds, on a fold arrow; or picks, in a picker), and a drag selects.
use super::*;
use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

/// Where the last frame put things, to map a click back to a row and column.
#[derive(Default, Clone, Copy)]
pub(super) struct Geom {
    /// Relative line numbers' column, the gutter, a split half's gutter and the half's width,
    /// and the code column's width beside the comment margin.
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
                    self.scroll(if down { 3 } else { -3 });
                }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if self.picker.is_some() {
                    return self.click_picker(y);
                }
                if self.popup.is_some() {
                    return (self.popup, self.help) = (None, None); // a click elsewhere closes it
                }
                let Some((row, col, gutter)) = self.at_screen(x, y) else { return };
                // On a fold arrow (a file's or a thread's first row, in the gutter): fold / unfold.
                let first_of_thread = self.rows[row].thread.is_some_and(|t| row == 0 || self.rows[row - 1].thread != Some(t));
                self.select = false;
                self.goto(row, col);
                match (gutter, self.rows[row].line.kind) {
                    (true, Kind::File) => self.toggle_collapse(),
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
    pub(super) fn at_screen(&self, x: usize, y: usize) -> Option<(usize, usize, bool)> {
        if y >= self.height {
            return None; // the status bar
        }
        let d = self.top + y;
        let g = self.geom;
        let (row, start) = match *self.drows.get(d)? {
            DRow::Full(r) => (r, g.relw + g.gw),
            // Split view: the old side, the divider, the new side.
            DRow::Pair(l, r) if x < g.relw + g.half => (l.or(r)?, g.relw + g.ghw),
            DRow::Pair(l, r) => (r.or(l)?, g.relw + g.half + 1 + g.ghw),
            // The comment margin: after the code column and a 1-column gap, behind its own gutter.
            DRow::Margin(code, note) if x > g.relw + g.gw + g.ctw => (note.or(code)?, g.relw + g.gw + g.ctw + 1 + 3),
            DRow::Margin(code, note) => (code.or(note)?, g.relw + g.gw),
        };
        Some((row, x.saturating_sub(start) + self.left, x < start))
    }
}
