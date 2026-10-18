//! The changed files down the left (`Ctrl-w e`): each under its folder, with what's open on it
//! (comments) and whether it's viewed; the one you're in lit. A click goes to a file.
use super::*;

/// Windows this wide show the list unless you've hidden it (`Ctrl-w e`, kept like the pane's).
const FILES_AUTO: usize = 150;

/// One line of the list: a folder (`None`), or file `i`.
pub(super) struct Entry {
    pub(super) file: Option<usize>,
    text: String,
}

impl App {
    /// The list's width (its divider included), or 0 when it's hidden.
    pub(super) fn files_width(&self, full: usize) -> usize {
        let shown = self.files_pref.unwrap_or(full >= FILES_AUTO) && full >= 100 && !self.files.is_empty();
        if !shown {
            return 0;
        }
        let longest = self.files.iter().map(|f| f.path.rsplit('/').next().unwrap_or(&f.path).chars().count()).max().unwrap_or(10);
        (longest + 9).clamp(24, 36) + 1
    }

    /// The list's lines: each folder once, its files under it.
    pub(super) fn file_entries(&self) -> Vec<Entry> {
        let mut out = vec![];
        let mut dir = None;
        for (i, f) in self.files.iter().enumerate() {
            let (d, name) = f.path.rsplit_once('/').map_or(("", f.path.as_str()), |(d, n)| (d, n));
            if dir != Some(d) {
                if !d.is_empty() {
                    out.push(Entry { file: None, text: format!("{d}/") });
                }
                dir = Some(d);
            }
            out.push(Entry { file: Some(i), text: name.to_owned() });
        }
        out
    }

    /// The file the cursor is in (or, on the PR's text, the top of the screen is).
    pub(super) fn current_file(&self) -> Option<usize> {
        let in_file = |r: usize| Some(self.rows.get(r)?.file).filter(|&f| f < self.files.len());
        in_file(self.head.row).or_else(|| {
            let r = match *self.drows.get(self.top)? {
                DRow::Full(r) => r,
                DRow::Pair(l, r) | DRow::Margin(l, r, _) => l.or(r)?,
            };
            in_file(r)
        })
    }

    pub(super) fn draw_files(&mut self, f: &mut Frame, area: Rect) {
        let ui = &self.ui;
        let w = area.width as usize - 1; // the divider
        let entries = self.file_entries();
        let cur = self.current_file();
        let h = (area.height as usize).saturating_sub(1);
        // The current file in view, when it's another (the wheel scrolls the list meanwhile).
        if let Some(i) = cur.filter(|_| cur != self.files_cur).and_then(|c| entries.iter().position(|e| e.file == Some(c))) {
            if i < self.files_top {
                self.files_top = i;
            }
            if i >= self.files_top + h {
                self.files_top = i + 1 - h;
            }
        }
        self.files_cur = cur;
        self.files_top = self.files_top.min(entries.len().saturating_sub(h));
        let bar = Style { bg: ui.bar, ..ui.header };
        let mut lines = vec![Line::from(Span::styled(format!("{:<w$}", format!(" FILES {}", self.files.len())), bar))];
        let dim = Style { fg: ui.stale.fg, ..Style::new() };
        for e in entries.iter().skip(self.files_top).take(h) {
            let line = match e.file {
                // A folder: dimmed, its start cut when it's long (the end says the most).
                None => {
                    let n = e.text.chars().count();
                    let t = if n + 1 > w { format!("…{}", e.text.chars().skip(n + 2 - w).collect::<String>()) } else { e.text.clone() };
                    Line::from(Span::styled(format!(" {t:<width$}", width = w - 1), dim))
                }
                Some(i) => {
                    let file = &self.files[i];
                    let on = cur == Some(i);
                    let row = if on { ui.base.bg(ui.cursorline.or(ui.menu_selected.bg).unwrap_or(Color::DarkGray)) } else { ui.base };
                    let open = self.threads.iter().filter(|t| t.path == file.path && !t.stale() && self.in_view(t)).count();
                    let viewed = self.viewed.get(&file.path).is_some_and(|s| s == "VIEWED");
                    let right = match open {
                        0 => String::new(),
                        n => format!(" ·{n}"),
                    };
                    let mark = if viewed { "✓ " } else { "  " };
                    let room = w.saturating_sub(1 + mark.len() + right.chars().count());
                    let name = cut(&e.text, room);
                    let pad = room.saturating_sub(name.chars().count());
                    Line::from(vec![
                        Span::styled(if on { "▌" } else { " " }, row.patch(Style { fg: ui.key.fg, ..Style::new() })),
                        Span::styled(mark, row.patch(Style { fg: ui.plus.fg, ..Style::new() })),
                        Span::styled(name, if on { row.add_modifier(ratatui::style::Modifier::BOLD) } else { row }),
                        Span::styled(" ".repeat(pad), row),
                        Span::styled(right, row.patch(Style { fg: ui.thread.fg, ..Style::new() })),
                    ])
                }
            };
            lines.push(line);
        }
        f.render_widget(Paragraph::new(lines), Rect { width: area.width - 1, ..area });
        let divider: Vec<Line> = (0..area.height).map(|_| Line::from(Span::styled("│", ui.linenr))).collect();
        f.render_widget(Paragraph::new(divider), Rect { x: area.x + area.width - 1, width: 1, ..area });
        self.files_entries = entries.into_iter().map(|e| e.file).collect();
    }

    /// A click on the list's line `y` (0 is its heading): that file, unfolded, at the top.
    pub(super) fn click_file(&mut self, y: usize) {
        let Some(Some(i)) = y.checked_sub(1).and_then(|y| self.files_entries.get(self.files_top + y)).copied() else { return };
        let path = self.files[i].path.clone();
        if self.collapsed.remove(&path) {
            self.build();
        }
        if let Some(r) = self.rows.iter().position(|r| r.file == i && r.line.kind == Kind::File) {
            self.goto(r, 0);
            self.top = self.d_of[r].max(self.pin as usize);
        }
    }

    /// `Ctrl-w e`: the files' list on / off, kept for next time.
    pub(super) fn toggle_files(&mut self) {
        let on = self.files_w == 0;
        self.files_pref = Some(on);
        self.save_pane("files", on.into());
        self.msg = if on { "the files' list (Ctrl-w e hides it)" } else { "files' list hidden (Ctrl-w e shows it)" }.into();
    }
}

/// `s` in at most `n` chars, ending in `…` if it was longer.
fn cut(s: &str, n: usize) -> String {
    match s.chars().count() > n {
        true => format!("{}…", s.chars().take(n.saturating_sub(1)).collect::<String>()),
        false => s.to_owned(),
    }
}
