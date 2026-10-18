//! The changed files down the left (`Ctrl-w e`): each under its folder, with what's open on it
//! (comments) and whether it's viewed; the one you're in lit. A click goes to a file. Under
//! them, the PR's commits: one, or a range (`v`), shown instead of the whole diff.
use super::*;

/// Windows this wide show the list unless you've hidden it (`Ctrl-w e`, kept like the pane's).
const FILES_AUTO: usize = 150;
/// The narrowest it goes (`Ctrl-w ,`).
const FILES_MIN: usize = 16;

/// One of the PR's commits.
#[derive(Clone, PartialEq, Debug)]
pub(super) struct Commit {
    pub(super) sha: String,
    pub(super) short: String,
    pub(super) subject: String,
}

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
        self.files_w_pref.unwrap_or((longest + 9).clamp(24, 36)).clamp(FILES_MIN, full / 3) + 1
    }

    /// `Ctrl-w ,` / `.` in the list: narrower / wider (`by`), `Ctrl-w =` (`0`) its usual width.
    pub(super) fn resize_files(&mut self, by: isize) {
        let w = (by != 0).then(|| (self.files_w as isize - 1 + by).max(FILES_MIN as isize) as usize);
        self.files_w_pref = w;
        self.save_pane("files_width", w.map_or(Value::Null, Value::from));
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
        // The commits under the files: up to a third of the height (all of them when few).
        let ch = match self.commits.len() {
            0 => 0,
            n => (n + 2).min((area.height as usize / 3).max(4)),
        };
        let h = (area.height as usize).saturating_sub(1 + ch);
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
                    let row = if on && self.files_focus {
                        ui.menu_selected
                    } else if on { ui.base.bg(ui.cursorline.or(ui.menu_selected.bg).unwrap_or(Color::DarkGray)) } else { ui.base };
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
        if ch > 0 {
            lines.resize(1 + h, Line::default());
            let y = area.y + 1 + h as u16;
            self.commits_y = y;
            lines.extend(self.commit_lines(w, ch - 1));
        }
        f.render_widget(Paragraph::new(lines), Rect { width: area.width - 1, ..area });
        // In the key color while it has focus (as the comments pane's edge is).
        let edge = if self.files_focus { Style { fg: self.ui.key.fg, ..Style::new() } } else { self.ui.linenr };
        let divider: Vec<Line> = (0..area.height).map(|_| Line::from(Span::styled("│", edge))).collect();
        f.render_widget(Paragraph::new(divider), Rect { x: area.x + area.width - 1, width: 1, ..area });
        self.files_entries = entries.into_iter().map(|e| e.file).collect();
    }

    /// The commits' part: its heading, then "all commits" and each commit (`h` lines of them).
    fn commit_lines(&mut self, w: usize, h: usize) -> Vec<Line<'static>> {
        let ui = &self.ui;
        let n = self.commits.len();
        // The cursor in view while it's there.
        if self.in_commits {
            self.commits_top = self.commits_top.min(self.commit_cur).max((self.commit_cur + 1).saturating_sub(h));
        }
        self.commits_top = self.commits_top.min((n + 1).saturating_sub(h));
        let title = match self.commit_shown {
            Some((a, b)) if a == b => format!(" COMMITS {} of {n}", a + 1),
            Some((a, b)) => format!(" COMMITS {}–{} of {n}", a + 1, b + 1),
            None => format!(" COMMITS {n}"),
        };
        let mut lines = vec![Line::from(Span::styled(format!("{title:<w$}"), Style { bg: ui.bar, ..ui.header }))];
        // Rows: 0 "all commits", r the commit r - 1.
        let shown = |r: usize| match self.commit_shown {
            None => r == 0,
            Some((a, b)) => r > a && r <= b + 1,
        };
        let picking = self.commit_anchor.map(|a| (a.min(self.commit_cur), a.max(self.commit_cur)));
        let focus = self.files_focus && self.in_commits;
        let lit = ui.base.bg(ui.cursorline.or(ui.menu_selected.bg).unwrap_or(Color::DarkGray));
        for r in (self.commits_top..=n).take(h) {
            let on = focus && r == self.commit_cur;
            let row = match (on, picking.is_some_and(|(a, b)| r >= a && r <= b), shown(r)) {
                (true, _, _) | (_, true, _) => ui.menu_selected,
                (_, _, true) => lit,
                _ => ui.base,
            };
            let (hash, text) = match r {
                0 => (String::new(), "all commits".to_owned()),
                r => (format!("{} ", self.commits[r - 1].short), self.commits[r - 1].subject.clone()),
            };
            let text = cut(&text, w.saturating_sub(2 + hash.chars().count()));
            let pad = w.saturating_sub(2 + hash.chars().count() + text.chars().count());
            lines.push(Line::from(vec![
                Span::styled(if on || shown(r) { "▌ " } else { "  " }, row.patch(Style { fg: ui.key.fg, ..Style::new() })),
                Span::styled(hash, row.patch(Style { fg: ui.stale.fg, ..Style::new() })),
                Span::styled(text, if shown(r) { row.add_modifier(ratatui::style::Modifier::BOLD) } else { row }),
                Span::styled(" ".repeat(pad), row),
            ]));
        }
        lines
    }

    /// A click on the list's line `y` (0 is its heading): that file, or that commit (with
    /// Shift: the commits from the one shown to it).
    pub(super) fn click_file(&mut self, y: usize, shift: bool) {
        let top = self.commits_y as usize;
        if !self.commits.is_empty() && y > top {
            let r = self.commits_top + y - top - 1;
            if r <= self.commits.len() {
                self.commit_cur = r;
                let from = self.commit_shown.map(|(a, _)| a + 1).filter(|_| shift && r > 0);
                self.commit_anchor = from;
                self.pick_commits();
            }
            return;
        }
        if let Some(Some(i)) = y.checked_sub(1).and_then(|y| self.files_entries.get(self.files_top + y)).copied() {
            self.goto_file(i);
        }
    }

    /// The wheel over the list's line `y`: its files or its commits scroll.
    pub(super) fn scroll_files(&mut self, y: usize, by: isize) {
        let top = self.commits_y as usize;
        let t = if !self.commits.is_empty() && y >= top { &mut self.commits_top } else { &mut self.files_top };
        *t = t.saturating_add_signed(by);
    }

    /// The PR's commits as checked out (first parents: the branch's own line), oldest first.
    pub(super) fn load_commits(&mut self) {
        let range = format!("{}..HEAD", self.full_base);
        let Ok(out) = diff::git(&["log", "--reverse", "--first-parent", "--format=%H%x09%h%x09%s", &range]) else { return };
        self.commits = out
            .lines()
            .filter_map(|l| {
                let mut p = l.splitn(3, '\t');
                Some(Commit { sha: p.next()?.into(), short: p.next()?.into(), subject: p.next().unwrap_or("").into() })
            })
            .collect();
        let n = self.commits.len();
        self.commit_cur = self.commit_cur.min(n);
        if self.commit_shown.is_some_and(|(_, b)| b >= n) {
            (self.commit_shown, self.to) = (None, None);
            self.base = self.full_base.clone();
        }
    }

    /// `m` in the commits: the one under the cursor's whole message (and who, when), in a
    /// popup.
    fn commit_popup(&mut self) {
        let Some(c) = self.commit_cur.checked_sub(1).and_then(|i| self.commits.get(i)) else {
            return self.msg = "m on a commit shows its message".into();
        };
        let text = diff::git(&["log", "-1", "--format=%an <%ae> · %ad%n%n%B", "--date=format:%Y-%m-%d %H:%M", &c.sha]).unwrap_or_else(|e| format!("{e:#}"));
        let mut lines: Vec<Line<'static>> = text.trim_end().lines().map(|l| Line::raw(l.to_owned())).collect();
        if let Some(l) = lines.first_mut() {
            *l = Line::styled(l.to_string(), self.ui.linenr);
        }
        if let Some(l) = lines.get_mut(2) {
            *l = Line::styled(l.to_string(), Style::new().add_modifier(ratatui::style::Modifier::BOLD));
        }
        self.popup = Some((format!(" commit {}  j/k scroll ", c.short), lines, 0));
    }

    /// Showing commits before the PR's last: its comments (on the head's lines) don't fit them.
    pub(super) fn before_head(&self) -> bool {
        self.commit_shown.is_some_and(|(_, b)| b + 1 < self.commits.len())
    }

    /// What's shown of the commits, for the status bar: `None` for all of them.
    pub(super) fn commits_label(&self) -> Option<String> {
        let (a, b) = self.commit_shown?;
        Some(match a == b {
            true => format!("commit {}", self.commits[a].short),
            false => format!("commits {}..{}", self.commits[a].short, self.commits[b].short),
        })
    }

    /// `Enter` in the commits: the one under the cursor, or the range from `v`'s anchor to it
    /// ("all commits" in it: the whole diff).
    fn pick_commits(&mut self) {
        let (a, b) = match self.commit_anchor.take() {
            Some(a) => (a.min(self.commit_cur), a.max(self.commit_cur)),
            None => (self.commit_cur, self.commit_cur),
        };
        self.show_commits((a > 0).then(|| (a - 1, b - 1)));
    }

    /// Show commits `first..=last` (`None`: all of them, the whole diff).
    pub(super) fn show_commits(&mut self, sel: Option<(usize, usize)>) {
        let sel = sel.filter(|&(a, b)| a <= b && b < self.commits.len());
        let (base, to) = match sel {
            None => (self.full_base.clone(), None),
            Some((a, b)) => (if a == 0 { self.full_base.clone() } else { self.commits[a - 1].sha.clone() }, Some(self.commits[b].sha.clone())),
        };
        (self.base, self.to, self.commit_shown) = (base, to, sel);
        // One alone: its message, a section before the files.
        self.commit_msg = sel.filter(|(a, b)| a == b).and_then(|(a, _)| diff::git(&["log", "-1", "--format=%B", &self.commits[a].sha]).ok());
        self.extra.clear(); // per-hunk context: the hunks are different ones now
        self.hl_cache.clear(); // both sides may be other versions
        self.msg = match (self.reload(false), self.commits_label()) {
            (Err(e), _) => format!("{e:#}"),
            (Ok(()), None) => "all commits: the whole diff".into(),
            (Ok(()), Some(l)) if self.before_head() => format!("{l} (comments are on the PR's head: they show with the last commit)"),
            (Ok(()), Some(l)) => l,
        };
    }

    /// A key while the list has focus: j / k go to the next / previous file, Ctrl-l, l,
    /// Enter or Esc back to the code. Others leave the list too, and act as usual (`false`).
    pub(super) fn files_key(&mut self, k: KeyEvent) -> bool {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let step = |app: &mut App, d: isize| {
            let n = app.files.len() as isize;
            let i = app.current_file().map_or(0, |c| (c as isize + d).clamp(0, n - 1));
            app.goto_file(i as usize);
        };
        let last_file = self.current_file().is_some_and(|c| c + 1 == self.files.len()) || self.files.is_empty();
        match k.code {
            // In the commits: j / k move, v marks a range's start, Enter shows it.
            KeyCode::Char('j') | KeyCode::Down if !ctrl && self.in_commits => self.commit_cur = (self.commit_cur + 1).min(self.commits.len()),
            KeyCode::Char('k') | KeyCode::Up if !ctrl && self.in_commits && self.commit_cur == 0 => self.in_commits = false,
            KeyCode::Char('k') | KeyCode::Up if !ctrl && self.in_commits => self.commit_cur -= 1,
            KeyCode::Char('v') if self.in_commits => self.commit_anchor = if self.commit_anchor.is_some() { None } else { Some(self.commit_cur) },
            KeyCode::Enter if self.in_commits => self.pick_commits(),
            // Between the files and the commits, without going through every file.
            KeyCode::Tab | KeyCode::BackTab => self.focus_list(!self.in_commits),
            KeyCode::Char('m') if self.in_commits => self.commit_popup(),
            KeyCode::Esc if self.commit_anchor.is_some() => self.commit_anchor = None,
            // Past the last file: on into the commits, at the one shown.
            KeyCode::Char('j') | KeyCode::Down if !ctrl && last_file && !self.commits.is_empty() => {
                self.in_commits = true;
                self.commit_cur = self.commit_shown.map_or(0, |(a, _)| a + 1);
            }
            KeyCode::Char('j') | KeyCode::Down if !ctrl => step(self, 1),
            KeyCode::Char('k') | KeyCode::Up if !ctrl => step(self, -1),
            KeyCode::Char('w') if ctrl => return false, // Ctrl-w keys act on the list (resize, hide)
            KeyCode::Char('h') | KeyCode::Left | KeyCode::Backspace => {}
            KeyCode::Char('l') | KeyCode::Right | KeyCode::Enter | KeyCode::Esc => (self.files_focus, self.commit_anchor) = (false, None),
            _ => {
                self.files_focus = false;
                return false;
            }
        }
        true
    }

    /// File `i`, unfolded, its header at the top.
    fn goto_file(&mut self, i: usize) {
        let path = self.files[i].path.clone();
        if self.collapsed.remove(&path) {
            self.build();
        }
        if let Some(r) = self.rows.iter().position(|r| r.file == i && r.line.kind == Kind::File) {
            self.goto(r, 0);
            self.top = self.d_of[r].max(self.pin as usize);
        }
    }

    /// Into the list: its commits (`Ctrl-w j`, at the one shown), or its files (`Ctrl-w k`).
    pub(super) fn focus_list(&mut self, commits: bool) {
        if self.files_w == 0 {
            return self.msg = "the files' list is hidden (Ctrl-w e shows it)".into();
        }
        self.files_focus = true;
        self.in_commits = commits && !self.commits.is_empty();
        if self.in_commits {
            self.commit_cur = self.commit_shown.map_or(0, |(a, _)| a + 1);
        }
    }

    /// `Ctrl-w e`: the files' list on / off, kept for next time.
    pub(super) fn toggle_files(&mut self) {
        let on = self.files_w == 0;
        self.files_pref = Some(on);
        self.files_focus &= on;
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
