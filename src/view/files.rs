//! What the diff shows, down the left (`Ctrl-w e`), as contents: the PR's description and the
//! messages of the commits shown, then the changed files, each under its folder, with what's
//! open on it (comments) and whether it's viewed; the one you're in lit. A click goes there.
//! Under them, the PR's commits: one, or a range (`v`), shown instead of the whole diff.
use super::*;

/// Windows this wide show the list unless you've hidden it (`Ctrl-w e`, kept like the pane's).
const FILES_AUTO: usize = 150;
/// The narrowest it goes (`Ctrl-w ,`).
const FILES_MIN: usize = 16;

/// What isn't committed, by kind: staged (in the index), changed (not staged), new (untracked).
#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub(super) struct Dirty {
    pub(super) staged: usize,
    pub(super) changed: usize,
    pub(super) new: usize,
}

impl Dirty {
    /// From `git status --porcelain` (a file can be staged and changed after).
    pub(super) fn of(status: &str) -> Dirty {
        let mut d = Dirty::default();
        for l in status.lines() {
            let (x, y) = (l.chars().next().unwrap_or(' '), l.chars().nth(1).unwrap_or(' '));
            match (x, y) {
                ('?', _) => d.new += 1,
                _ => {
                    d.staged += usize::from(x != ' ');
                    d.changed += usize::from(y != ' ');
                }
            }
        }
        d
    }

    pub(super) fn has(&self) -> bool {
        *self != Dirty::default()
    }

    /// `2 staged, 3 changed, 1 new` (those there are).
    pub(super) fn says(&self) -> String {
        [(self.staged, "staged"), (self.changed, "changed"), (self.new, "new")].iter().filter(|k| k.0 > 0).map(|(n, k)| format!("{n} {k}")).collect::<Vec<_>>().join(", ")
    }
}

/// One of the PR's commits.
#[derive(Clone, PartialEq, Debug)]
pub(super) struct Commit {
    pub(super) sha: String,
    pub(super) short: String,
    pub(super) subject: String,
    /// When it was authored (seconds), which a rebase or an amend keeps.
    pub(super) at: String,
    /// GitHub verifies a signature on it (as the PR's info says; no PR: taken as signed).
    pub(super) signed: bool,
}

/// What a line of the scrolling list is, for clicks: an entry (a file or a section), commit row
/// `r` (0: "all commits"), or nothing to click (a folder, a heading, a blank).
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Hit {
    Spot(Spot),
    Commit(usize),
    Nothing,
}

/// What a line of the list is: a folder, file `i`, or a section (`Row::main`).
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Spot {
    Folder,
    File(usize),
    Section(usize),
}

pub(super) struct Entry {
    pub(super) spot: Spot,
    pub(super) text: String,
}

impl App {
    /// The list's tints: its cursor's (the cursor line's, as in the diff and the pane), and
    /// what's shown in the diff when the list hasn't focus (the related tint: not the cursor).
    fn list_tints(&self) -> (Style, Style) {
        let ui = &self.ui;
        let line = |c: Option<Color>| ui.base.bg(c.or(ui.menu_selected.bg).unwrap_or(Color::DarkGray));
        (line(ui.cursorline), line(ui.related.or(ui.cursorline)))
    }

    /// The list's width (its divider included), or 0 when it's hidden.
    pub(super) fn files_width(&self, full: usize) -> usize {
        // (However many files: none, its heading says so.)
        let shown = self.files_pref.unwrap_or(full >= FILES_AUTO) && full >= 100;
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

    /// The list's lines: the sections (as their headers say), then each folder once, its files
    /// under it.
    pub(super) fn file_entries(&self) -> Vec<Entry> {
        let mut out: Vec<Entry> = self
            .rows
            .iter()
            .filter(|r| r.line.kind == Kind::File)
            .filter_map(|r| {
                let id = r.main?;
                let text = match self.commits.get(id.wrapping_sub(1)) {
                    _ if id == DESCRIPTION => "description".to_owned(),
                    _ if id == CLOSING_NOTE => "closing note".to_owned(),
                    _ if id == CLOSING_PR => "closing comment (Claude's draft)".to_owned(),
                    Some(c) => format!("{} {}", c.short, c.subject),
                    None => r.line.text.clone(),
                };
                let text = if id == DESCRIPTION && self.has_proposal() { format!("{text} (Claude's proposal)") } else { text };
                Some(Entry { spot: Spot::Section(id), text })
            })
            .collect();
        let mut dir = None;
        for (i, f) in self.files.iter().enumerate() {
            let (d, name) = f.path.rsplit_once('/').map_or(("", f.path.as_str()), |(d, n)| (d, n));
            if dir != Some(d) {
                if !d.is_empty() {
                    out.push(Entry { spot: Spot::Folder, text: format!("{d}/") });
                }
                dir = Some(d);
            }
            out.push(Entry { spot: Spot::File(i), text: name.to_owned() });
        }
        out
    }

    /// Where the cursor is, as the list has it (or, on the PR's text, the top of the screen).
    pub(super) fn current_spot(&self) -> Option<Spot> {
        let at = |r: usize| {
            let row = self.rows.get(r)?;
            row.main.map(Spot::Section).or_else(|| (row.file < self.files.len()).then_some(Spot::File(row.file)))
        };
        at(self.head.row).or_else(|| {
            // The top of the screen, under the pinned title.
            let r = match *self.drows.get(self.top.max(self.pin as usize))? {
                DRow::Full(r) => r,
                DRow::Pair(l, r) | DRow::Margin(l, r, _) => l.or(r)?,
            };
            at(r)
        })
    }

    /// The list: its contents, the commits right after them, and `keys` (the key hints: what
    /// can follow a prefix key, or what the keys do where the cursor is) pinned at its bottom.
    pub(super) fn draw_files(&mut self, f: &mut Frame, area: Rect, keys: Option<(&str, &[(&str, &str)])>) {
        let ui = &self.ui;
        let w = area.width as usize - 1; // the divider
        let entries = self.file_entries();
        let cur = self.current_spot();
        // The hints: their heading and a line each, cut to the width; at most half the height.
        let hints: Vec<Line<'static>> = keys.map_or(vec![], |(title, keys)| {
            let kw = keys.iter().map(|k| k.0.chars().count()).max().unwrap_or(1);
            let head = Line::from(Span::styled(format!("{:<w$}", format!(" KEYS: {title}")), Style { bg: ui.bar, ..ui.header }));
            let rows = keys.iter().map(|(k, d)| {
                Line::from(vec![Span::styled(format!(" {k:<kw$}  "), ui.key), Span::raw(cut(d, w.saturating_sub(kw + 3)))])
            });
            std::iter::once(head).chain(rows).take(area.height as usize / 2).collect()
        });
        let avail = (area.height as usize).saturating_sub(hints.len());
        // Under the heading, one list that scrolls: the contents, a blank line, the commits.
        let room = avail.saturating_sub(1);
        let sel = self.files_sel.filter(|_| self.files_focus && !self.in_commits);
        let (cursor, here) = self.list_tints();
        let bar = Style { bg: ui.bar, ..ui.header };
        let count = match self.files.len() {
            0 => self.no_files().to_owned(),
            n => format!("{n} files"),
        };
        let heading = Line::from(Span::styled(format!("{:<w$}", format!(" CONTENTS · {count}")), bar));
        let dim = Style { fg: ui.stale.fg, ..Style::new() };
        let mut rows: Vec<(Line<'static>, Hit)> = vec![];
        for e in &entries {
            // The list's cursor (while it has focus), else where the diff is.
            let lit = |on: bool| match (sel == Some(e.spot), on) {
                (true, _) => cursor,
                (_, true) => here,
                _ => ui.base,
            };
            let line = match e.spot {
                // A section: its name, in the headings' color.
                Spot::Section(_) => {
                    let on = cur == Some(e.spot);
                    let row = lit(on);
                    let name = cut(&e.text, w.saturating_sub(3));
                    let pad = w.saturating_sub(3 + name.chars().count());
                    Line::from(vec![
                        Span::styled(if on { "▌" } else { " " }, row.patch(Style { fg: ui.key.fg, ..Style::new() })),
                        Span::styled("≡ ", row.patch(Style { fg: ui.stale.fg, ..Style::new() })),
                        Span::styled(name, row.patch(Style { fg: ui.heading.fg, ..Style::new() })),
                        Span::styled(" ".repeat(pad), row),
                    ])
                }
                // A folder: dimmed, its start cut when it's long (the end says the most).
                Spot::Folder => {
                    let n = e.text.chars().count();
                    let t = if n + 1 > w { format!("…{}", e.text.chars().skip(n + 2 - w).collect::<String>()) } else { e.text.clone() };
                    Line::from(Span::styled(format!(" {t:<width$}", width = w - 1), dim))
                }
                Spot::File(i) => {
                    let file = &self.files[i];
                    let on = cur == Some(e.spot);
                    let row = lit(on);
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
            rows.push((line, if e.spot == Spot::Folder { Hit::Nothing } else { Hit::Spot(e.spot) }));
        }
        if !self.commits.is_empty() {
            rows.push((Line::default(), Hit::Nothing));
            rows.extend(self.commit_lines(w));
        }
        // In view: the list's cursor while it has focus (on a commit, or an entry), else where the
        // diff is when that changes (the wheel scrolls the list meanwhile).
        let at = |h: Hit| rows.iter().position(|r| r.1 == h);
        let want = match (self.files_focus && self.in_commits, sel) {
            (true, _) => at(Hit::Commit(self.commit_cur)),
            (false, Some(s)) => at(Hit::Spot(s)),
            _ => cur.filter(|_| cur != self.files_cur).and_then(|c| at(Hit::Spot(c))),
        };
        if let Some(i) = want {
            self.files_top = self.files_top.min(i).max((i + 1).saturating_sub(room));
        }
        self.files_cur = cur;
        self.files_top = self.files_top.min(rows.len().saturating_sub(room));
        let mut lines = vec![heading];
        self.list_hits = rows.iter().skip(self.files_top).take(room).map(|r| r.1).collect();
        lines.extend(rows.into_iter().skip(self.files_top).take(room).map(|r| r.0));
        // The hints, pinned at the bottom.
        lines.resize(avail, Line::default());
        lines.extend(hints);
        f.render_widget(Paragraph::new(lines), Rect { width: area.width - 1, ..area });
        // (Dim, focused or not: focus shows by the cursor line's tint, its selection here.)
        let edge = self.ui.linenr;
        let divider: Vec<Line> = (0..area.height).map(|_| Line::from(Span::styled("│", edge))).collect();
        f.render_widget(Paragraph::new(divider), Rect { x: area.x + area.width - 1, width: 1, ..area });
    }

    /// The commits' part: its heading, then "all commits" and each commit.
    fn commit_lines(&self, w: usize) -> Vec<(Line<'static>, Hit)> {
        let ui = &self.ui;
        let n = self.commits.len();
        let title = match self.commit_shown {
            Some((a, b)) if a == b => format!(" COMMITS {} of {n}", a + 1),
            Some((a, b)) => format!(" COMMITS {}–{} of {n}", a + 1, b + 1),
            None => format!(" COMMITS {n}"),
        };
        let mut lines = vec![(Line::from(Span::styled(format!("{title:<w$}"), Style { bg: ui.bar, ..ui.header })), Hit::Nothing)];
        // Rows: 0 "all commits", r the commit r - 1; then, if anything isn't, "uncommitted".
        let last = n + usize::from(self.dirty.has());
        let shown = |r: usize| match (self.uncommitted(), self.commit_shown) {
            (true, _) => r == n + 1,
            (_, None) => r == 0,
            (_, Some((a, b))) => r > a && r <= b + 1,
        };
        let picking = self.commit_anchor.map(|a| (a.min(self.commit_cur), a.max(self.commit_cur)));
        let focus = self.files_focus && self.in_commits;
        let (cursor, here) = self.list_tints();
        for r in 0..=last {
            let on = focus && r == self.commit_cur;
            let row = match (on, picking.is_some_and(|(a, b)| r >= a && r <= b), shown(r)) {
                (true, _, _) => cursor,
                (_, true, _) => ui.menu_selected, // (a range being picked)
                (_, _, true) => here,
                _ => ui.base,
            };
            let (hash, text) = match r {
                0 => (String::new(), "all commits".to_owned()),
                r if r > n => (String::new(), format!("uncommitted · {}", self.dirty.says())),
                r => (format!("{} ", self.commits[r - 1].short), self.commits[r - 1].subject.clone()),
            };
            // An unsigned commit: marked (a repo may refuse it on its base).
            let unsigned = if r > 0 && r <= n && !self.commits[r - 1].signed { "✗ " } else { "" };
            let text = cut(&text, w.saturating_sub(2 + hash.chars().count() + unsigned.chars().count()));
            let pad = w.saturating_sub(2 + hash.chars().count() + unsigned.chars().count() + text.chars().count());
            let line = Line::from(vec![
                Span::styled(if on || shown(r) { "▌ " } else { "  " }, row.patch(Style { fg: ui.key.fg, ..Style::new() })),
                Span::styled(hash, row.patch(Style { fg: ui.stale.fg, ..Style::new() })),
                Span::styled(unsigned, row.patch(Style { fg: ui.error.fg, ..Style::new() })),
                Span::styled(text, if shown(r) { row.add_modifier(ratatui::style::Modifier::BOLD) } else { row }),
                Span::styled(" ".repeat(pad), row),
            ]);
            lines.push((line, Hit::Commit(r)));
        }
        lines
    }

    /// A click on the list's line `y` (0 is its heading): that file or section, or that commit
    /// (with Shift: the commits from the one shown to it).
    pub(super) fn click_file(&mut self, y: usize, shift: bool) {
        match y.checked_sub(1).and_then(|y| self.list_hits.get(y)).copied() {
            Some(Hit::Spot(s)) => self.goto_spot(s),
            Some(Hit::Commit(r)) => {
                self.commit_cur = r;
                self.commit_anchor = self.commit_shown.map(|(a, _)| a + 1).filter(|_| shift && r > 0);
                self.pick_commits();
            }
            _ => {} // a folder, a heading, the key hints
        }
    }

    /// The wheel over the list: it scrolls (the key hints stay where they are).
    pub(super) fn scroll_files(&mut self, by: isize) {
        self.files_top = self.files_top.saturating_add_signed(by);
    }

    /// The PR's commits as checked out (first parents: the branch's own line), oldest first.
    pub(super) fn load_commits(&mut self) {
        let range = format!("{}..HEAD", self.full_base);
        let Ok(out) = diff::git(&["log", "--reverse", "--first-parent", "--format=%H%x09%h%x09%at%x09%s", &range]) else { return };
        let was: Vec<String> = self.commits.iter().map(|c| c.sha.clone()).collect();
        self.commits = out
            .lines()
            .filter_map(|l| {
                let mut p = l.splitn(4, '\t');
                let (sha, short, at) = (p.next()?, p.next()?, p.next()?);
                let signed = !self.unsigned.iter().any(|u| u == sha);
                Some(Commit { sha: sha.into(), short: short.into(), at: at.into(), signed, subject: p.next().unwrap_or("").into() })
            })
            .collect();
        self.dirty = Dirty::of(&diff::git(&["status", "--porcelain"]).unwrap_or_default());
        let n = self.commits.len();
        // What's kept by a commit (the commits shown, the list's cursor and range's start) follows
        // it by what it was (`now_as`: amended, rebased, other SHAs), not where it sat (one dropped
        // before it moves the rest); gone (dropped), the whole diff again.
        let rewritten = std::cell::OnceCell::new();
        let at = |sha: &str| self.now_as(sha, &rewritten).and_then(|s| self.commits.iter().position(|c| c.sha == s));
        // (The list's spots: 0 the whole diff, then each commit, then what isn't committed.)
        let spot = |k: usize| match k {
            0 => Some(0),
            k if k <= was.len() => at(&was[k - 1]).map(|i| i + 1),
            _ => Some(n + 1),
        };
        let (cur, anchor) = (spot(self.commit_cur).unwrap_or(self.commit_cur), self.commit_anchor.and_then(spot));
        let shown = |sha: &Option<String>| sha.as_deref().and_then(at);
        let (first, last) = (shown(&self.shown_first), shown(&self.to));
        self.commit_cur = cur.min(n + usize::from(self.dirty.has()));
        self.commit_anchor = anchor;
        if self.commit_shown.is_some() {
            match (first, last) {
                (Some(a), Some(b)) if a <= b => {
                    self.commit_shown = Some((a, b));
                    self.commits_as_now((a, b));
                }
                _ => self.show_whole(self.full_base.clone()),
            }
        }
    }

    /// The keys while the list has focus, for its hints: the commits', or the contents'.
    pub(super) fn list_keys(&self) -> (&'static str, &'static [(&'static str, &'static str)]) {
        match self.in_commits {
            true => ("commits", &[
                ("Enter", "show it (with v: the range)"),
                ("v", "start a range"),
                ("m", "its whole message"),
                ("y", "copy its hash"),
                ("a", "ask Claude about it / for a change to it"),
                ("f", "fold what isn't committed into it"),
                ("Tab", "to the contents"),
                ("Ctrl-l", "back to the code"),
            ]),
            false => ("contents", &[("j k", "move"), ("Enter", "go there"), ("Tab", "to the commits"), ("Ctrl-l", "back to the code")]),
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
        // "uncommitted": what isn't committed (as Space g shows it). (Picking commits leaves it:
        // show_commits sets the base.)
        if self.commit_cur > self.commits.len() {
            self.commit_anchor = None;
            if !self.uncommitted() {
                self.toggle_uncommitted();
            }
            return;
        }
        // (A range starts on a commit, not on "uncommitted".)
        self.commit_anchor = self.commit_anchor.filter(|&a| a <= self.commits.len());
        let (a, b) = match self.commit_anchor.take() {
            Some(a) => (a.min(self.commit_cur), a.max(self.commit_cur)),
            None => (self.commit_cur, self.commit_cur),
        };
        self.show_commits((a > 0).then(|| (a - 1, b - 1)));
    }

    /// `Space C`: the PR's commits to pick one of (its diff alone), or all of them (newest first;
    /// the one shown marked).
    pub(super) fn commit_picker(&mut self) {
        if self.commits.is_empty() {
            return self.msg = "no commits here to pick from".into();
        }
        let all = self.commit_shown.is_none() && !self.uncommitted();
        let mut items = vec![Item::new(format!("{} all commits: the whole diff", if all { "●" } else { " " }), Target::Commit(None))];
        items.extend(self.commits.iter().enumerate().rev().map(|(i, c)| {
            let on = if self.commit_shown == Some((i, i)) { "●" } else { " " };
            Item::new(format!("{on} {} {}", c.short, c.subject), Target::Commit(Some(c.sha.clone())))
        }));
        self.picker = Some(Picker::new(" commits ", items));
    }

    /// The diff and messages of commits `a..=b` of the list as it is: from the one before (or
    /// the base) to the last; their messages, each a section before the files (one `git log`).
    fn commits_as_now(&mut self, (a, b): (usize, usize)) {
        let first = Some(self.commits[a].sha.clone());
        let to = Some(self.commits[b].sha.clone());
        let base = if a == 0 { self.full_base.clone() } else { self.commits[a - 1].sha.clone() };
        if (&first, &to, &base) == (&self.shown_first, &self.to, &self.base) {
            return; // (the same commits: as they were, their messages too)
        }
        self.set_diff(base, to);
        self.shown_first = first;
        let mut args = vec!["log", "--no-walk=unsorted", "--format=%B%x00"];
        args.extend(self.commits[a..=b].iter().map(|c| c.sha.as_str()));
        let out = diff::git(&args).unwrap_or_default();
        self.commit_msgs = out.split('\0').map(|m| m.trim_matches('\n').to_owned()).take(b + 1 - a).collect();
    }

    /// Show commits `first..=last` (`None`: all of them, the whole diff).
    pub(super) fn show_commits(&mut self, sel: Option<(usize, usize)>) {
        let sel = sel.filter(|&(a, b)| a <= b && b < self.commits.len());
        match sel {
            Some(sel) => {
                self.commit_shown = Some(sel);
                self.commits_as_now(sel);
            }
            None => self.show_whole(self.full_base.clone()),
        }
        self.msg = match (self.reload(false), self.commits_label()) {
            (Err(e), _) => format!("{e:#}"),
            (Ok(()), None) => "all commits: the whole diff".into(),
            (Ok(()), Some(l)) if self.before_head() => format!("{l} (comments are on the PR's head: they show with the last commit)"),
            (Ok(()), Some(l)) => l,
        };
    }

    /// A key while the list has focus: j / k move its cursor (the diff stays put), Enter goes
    /// there and back to the code, Ctrl-l, l or Esc back without going. Others leave the list
    /// too, and act as usual (`false`).
    pub(super) fn files_key(&mut self, k: KeyEvent) -> bool {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        // Through what the list has (sections, then files), in its order.
        let spots: Vec<Spot> = self.file_entries().into_iter().map(|e| e.spot).filter(|&s| s != Spot::Folder).collect();
        // The list's cursor moves; the diff stays where it is until Enter.
        let at = self.files_sel.or(self.current_spot()).and_then(|c| spots.iter().position(|&s| s == c));
        let step = |app: &mut App, d: isize| {
            let i = at.map_or(0, |i| (i as isize + d).clamp(0, spots.len() as isize - 1) as usize);
            app.files_sel = spots.get(i).copied();
        };
        let last_file = at.is_some_and(|i| i + 1 == spots.len()) || spots.is_empty();
        match k.code {
            // In the commits: j / k move, v marks a range's start, Enter shows it.
            KeyCode::Char('j') | KeyCode::Down if !ctrl && self.in_commits => self.commit_cur = (self.commit_cur + 1).min(self.commits.len() + usize::from(self.dirty.has())),
            KeyCode::Char('k') | KeyCode::Up if !ctrl && self.in_commits && self.commit_cur == 0 => self.in_commits = false,
            KeyCode::Char('k') | KeyCode::Up if !ctrl && self.in_commits => self.commit_cur -= 1,
            KeyCode::Char('v') if self.in_commits && self.commit_cur > self.commits.len() && self.commit_anchor.is_none() => {} // (no range from "uncommitted")
            KeyCode::Char('v') if self.in_commits => self.commit_anchor = if self.commit_anchor.is_some() { None } else { Some(self.commit_cur) },
            KeyCode::Enter if self.in_commits => self.pick_commits(),
            // Between the files and the commits, without going through every file.
            KeyCode::Tab | KeyCode::BackTab => self.focus_list(!self.in_commits),
            KeyCode::Char('m') if self.in_commits => self.commit_popup(),
            KeyCode::Char('a') if self.in_commits => match self.commit_cur.checked_sub(1) {
                Some(i) => self.ask_commit(i),
                None => self.msg = "a on a commit asks Claude about it, or for a change to it".into(),
            },
            KeyCode::Char('f') if self.in_commits => match self.commit_cur.checked_sub(1).and_then(|i| self.commits.get(i)).map(|c| c.sha.clone()) {
                Some(sha) => self.amend_into(&sha),
                None => self.msg = "f on a commit folds what isn't committed into it (amends it)".into(),
            },
            KeyCode::Char('y') if self.in_commits => match self.commit_cur.checked_sub(1).and_then(|i| self.commits.get(i)).map(|c| c.sha.clone()) {
                Some(sha) => {
                    clipboard(&sha);
                    self.msg = format!("copied {sha}");
                }
                None => self.msg = "y on a commit copies its hash".into(),
            },
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
            KeyCode::Enter => {
                match self.files_sel {
                    Some(s) => self.goto_spot(s),
                    None => (self.files_focus, self.commit_anchor) = (false, None),
                }
            }
            KeyCode::Char('l') | KeyCode::Right | KeyCode::Esc => (self.files_focus, self.commit_anchor) = (false, None),
            _ => {
                self.files_focus = false;
                return false;
            }
        }
        true
    }

    /// File `path` shown, for what's wanted of it (`shown`): unfolded, and, not shown in this
    /// view (Claude's, pending), the whole diff. Rebuilt keeping the cursor.
    pub(super) fn show_file(&mut self, path: &str, shown: impl Fn(&Self) -> bool) {
        if self.collapsed.remove(path) {
            let keep = self.cursor_key();
            self.rebuild(keep);
        }
        if !shown(self) && self.view != View::All {
            self.view = View::All;
            let keep = self.cursor_key();
            self.rebuild(keep);
        }
    }

    /// To a line of the list (from it, or `Space f`): a file (shown) or a section, its header
    /// at the top; a jump (Ctrl-o comes back), and back to the code from the list.
    pub(super) fn goto_spot(&mut self, s: Spot) {
        (self.files_focus, self.commit_anchor) = (false, None);
        if let Spot::File(i) = s
            && let Some(path) = self.files.get(i).map(|f| f.path.clone())
        {
            self.show_file(&path, |a| a.rows.iter().any(|r| r.file == i));
        }
        let header = |r: &Row| {
            r.line.kind == Kind::File
                && match s {
                    Spot::File(i) => r.file == i,
                    Spot::Section(id) => r.main == Some(id),
                    Spot::Folder => false,
                }
        };
        if let Some(r) = self.rows.iter().position(header) {
            self.jump(r, 0);
            self.top = self.d_of[r].max(self.pin as usize);
        }
    }

    /// Into the list: its commits (`Ctrl-w j`, at the one shown), or its files (`Ctrl-w k`).
    /// A diff with no files, said: a draft's are to come; else there are none.
    pub(super) fn no_files(&self) -> &'static str {
        if self.unpublished() { "no files yet" } else { "no changes" }
    }

    /// Whether the list has anything to go to (a section, a file, a commit).
    pub(super) fn list_has_spots(&self) -> bool {
        !self.file_entries().is_empty() || !self.commits.is_empty()
    }

    pub(super) fn focus_list(&mut self, commits: bool) {
        if self.files_w == 0 {
            return self.msg = "the files' list is hidden (Ctrl-w e shows it)".into();
        }
        if !self.list_has_spots() {
            return self.msg = format!("the list has nothing to go to ({})", self.no_files());
        }
        (self.files_focus, self.files_sel) = (true, self.current_spot());
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

/// `s` in at most `n` chars, ending in `…` if it was longer (dv list's too).
pub(crate) fn cut(s: &str, n: usize) -> String {
    match s.chars().count() > n {
        true => format!("{}…", s.chars().take(n.saturating_sub(1)).collect::<String>()),
        false => s.to_owned(),
    }
}

#[test]
fn what_isnt_committed() {
    let d = Dirty::of("M  a.rs\nMM b.rs\n M c.rs\n?? d.rs\nA  e.rs\n");
    assert_eq!((d.staged, d.changed, d.new), (3, 2, 1));
    assert_eq!(d.says(), "3 staged, 2 changed, 1 new");
    assert!(!Dirty::of("").has());
}
