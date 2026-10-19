//! Rows onto the screen: one per screen row, side by side in split view, or with comments
//! beside the code in the right margin.

use super::*;

impl App {
    /// The width of the comment box row `r` is in, drawn `tw` wide: as wide as the comment text
    /// wraps (+ margin), not the whole screen; in the comment pane, the pane's width.
    pub(super) fn box_w(&self, r: usize, tw: usize) -> usize {
        if self.margin_row(r) { tw } else { tw.min(self.wrap_w + 2) }
    }

    /// The rows the pane has, in order, when the cursor's in it (the comments); nothing when
    /// it's in the code.
    pub(super) fn pane_rows(&self) -> Vec<usize> {
        match self.margin_side() {
            true => (0..self.rows.len()).filter(|&r| self.in_margin(r)).collect(),
            false => vec![],
        }
    }

    /// Comments further down (`down`) or up than what's on screen, beside the code.
    pub(super) fn comments_beyond(&self, down: bool) -> bool {
        let has = |d: &DRow| matches!(d, DRow::Margin(_, _, Some(_)));
        let end = (self.top + self.height).min(self.drows.len());
        match down {
            true => self.drows[end..].iter().any(has),
            false => self.drows[..self.top.min(self.drows.len())].iter().any(has),
        }
    }

    /// The code line the comment under the cursor is beside.
    pub(super) fn line_beside(&self) -> usize {
        (0..self.head.row).rev().find(|&r| !self.in_margin(r)).unwrap_or(0)
    }

    /// The pane shown, if it wasn't (`gn` / `gp`, a click on a tab).
    pub(super) fn show_pane(&mut self) {
        self.margin_pref = Some(true);
        self.layout();
    }

    /// The pane's tabs, on the pinned title's right end, over the pane (`x` from where it
    /// starts): each one's label, with how many threads it shows, and its columns.
    pub(super) fn pane_tabs(&self) -> Vec<(Bar, String, std::ops::Range<usize>)> {
        let mut x = 1;
        // A draft no one else has seen: only Claude's comments and yours, so one tab for both;
        // and no PR yet (what it replaces is atop its description).
        let tabs: &[_] = match self.unpublished() {
            true => &[(Bar::Claude, "Claude")],
            false => &[(Bar::Comments, "Comments"), (Bar::Claude, "Claude"), (Bar::Pending, "Pending")],
        };
        // The threads about the code each tab's view shows (as the view decides it).
        let about_code: Vec<&Thread> = self.threads.iter().filter(|t| self.on_code(t)).collect();
        tabs.iter()
            .map(|&(b, name)| {
                let v = match b {
                    Bar::Claude if !self.unpublished() => View::Claude,
                    Bar::Pending => View::Pending,
                    _ => View::All,
                };
                let n = about_code.iter().filter(|t| self.in_view_of(v, t)).count();
                let l = match n {
                    0 => format!(" {name} "),
                    n => format!(" {name} {n} "),
                };
                let w = l.chars().count();
                x += w;
                (b, l, x - w..x)
            })
            .collect()
    }

    /// The tab that's lit: Claude's review (the Claude view), your pending one (the pending
    /// view), or the comments (the whole diff).
    pub(super) fn bar(&self) -> Bar {
        match self.view {
            _ if self.unpublished() => Bar::Claude,
            View::Claude => Bar::Claude,
            View::Pending => Bar::Pending,
            View::All => Bar::Comments,
        }
    }

    /// To tab `b` (a click on it, `gn` / `gp`): each tab its view: Claude's, the Claude view
    /// beside its comments; Pending, the pending view; the comments', the whole diff.
    pub(super) fn bar_to(&mut self, b: Bar) {
        // In a draft, Claude's tab is the whole diff: every comment there is Claude's or yours.
        let v = match b {
            Bar::Claude if !self.unpublished() => View::Claude,
            Bar::Pending => View::Pending,
            _ => View::All,
        };
        if v != self.view {
            self.toggle_view(if v == View::All { self.view } else { v });
        }
        self.show_pane();
    }

    /// `gn` / `gp`: the next / previous tab, round.
    pub(super) fn cycle_bar(&mut self, fwd: bool) {
        let tabs: Vec<Bar> = self.pane_tabs().into_iter().map(|t| t.0).collect();
        let i = tabs.iter().position(|&b| b == self.bar()).unwrap_or(0);
        self.bar_to(tabs[(i + if fwd { 1 } else { tabs.len() - 1 }) % tabs.len()]);
    }

    /// A replacement (or new PR) not published yet: Claude's draft, seen by no one else.
    pub(super) fn unpublished(&self) -> bool {
        self.replacement.as_ref().is_some_and(|r| r.published.is_none())
    }

    /// The first row of the code block or table (in a comment) that row `r` is a line of.
    pub(super) fn code_block(&self, r: usize) -> Option<usize> {
        let row = self.rows.get(r).filter(|x| x.nowrap)?;
        let same = |x: &Row| x.nowrap && x.thread == row.thread && x.info == row.info;
        Some((0..r).rev().take_while(|&i| same(&self.rows[i])).last().unwrap_or(r))
    }

    /// How far row `r`'s text is scrolled sideways: the diff by `left`; in a comment's box,
    /// not at all, but for the cursor's code block, by its own offset.
    pub(super) fn hscroll(&self, r: usize) -> usize {
        match self.box_part(r) {
            // The pinned title isn't the diff's.
            None if r == 0 && self.pin => 0,
            None => self.left,
            Some(_) if self.code_at.is_some() && self.code_block(r) == self.code_at => self.code_left,
            Some(_) => 0,
        }
    }

    /// The pane as you last left it (`Ctrl-w c`, `f`, `,` / `.`), and relative line numbers
    /// (`Ctrl-w n`), in any repo.
    pub(super) fn load_pane(&mut self) {
        self.pane_file = crate::claude::data_home().ok().map(|d| d.join("pane.json"));
        self.apply_pane();
    }

    /// What `pane_file` has, applied.
    pub(super) fn apply_pane(&mut self) {
        let v = self.pane_json();
        self.margin_pref = v["shown"].as_bool();
        self.margin_float_pref = v["float"].as_bool();
        self.margin_w_pref = v["width"].as_u64().map(|w| w as usize);
        // Relative line numbers as you last set them (`Ctrl-w n`), over Helix's setting.
        if let Some(r) = v["relative"].as_bool() {
            self.settings.relative = r;
        }
        self.files_pref = v["files"].as_bool();
        self.files_w_pref = v["files_width"].as_u64().map(|w| w as usize);
    }

    pub(super) fn pane_json(&self) -> Value {
        let text = self.pane_file.as_ref().and_then(|p| std::fs::read_to_string(p).ok());
        text.and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_else(|| serde_json::json!({}))
    }

    /// Keeps `key` for the next run (`null` forgets it). Only what its key set: the pane shown
    /// for a comment under the cursor isn't a choice to remember.
    pub(super) fn save_pane(&self, key: &str, val: Value) {
        let Some(p) = &self.pane_file else { return };
        let mut v = self.pane_json();
        v[key] = val;
        let _ = std::fs::create_dir_all(p.parent().unwrap_or(p));
        let _ = std::fs::write(p, v.to_string());
    }

    /// Map rows to screen rows. Code runs down the left: one column, or in split view two,
    /// where each run of removed lines pairs with the added lines that follow it and unchanged
    /// lines show on both sides. Comments on files go in the margin on the right, each box
    /// beside its line (or just below the box before it, if that's still running there); code
    /// only gets spacer rows when a file's comments need more room than its code has (they
    /// never spill into the next file). With the margin hidden, they aren't shown, and the
    /// lines they're on have a mark in the gutter.
    pub(super) fn layout(&mut self) {
        let n = self.rows.len();
        let kind = |i: usize| self.rows[i].line.kind;
        // Code that a comment can sit beside: a diff line or a hunk header.
        let code = |i: usize| {
            let row = &self.rows[i];
            matches!(row.line.kind, Kind::Ctx | Kind::Add | Kind::Del | Kind::Hunk) && row.file != INFO
        };
        let margin = |i: usize| self.in_margin(i);
        // The main column's rows (full width, or code: one row, or in split view a pair), each
        // with the comment rows that follow its rows; and the code row each thread is about.
        let mut units: Vec<(DRow, Vec<usize>)> = vec![];
        let mut unit_of = vec![usize::MAX; n];
        let mut anchors = HashMap::new();
        let mut last: Option<usize> = None; // the last row in the main column, and the last code row
        let mut last_code: Option<usize> = None;
        let mut i = 0;
        while i < n {
            if margin(i) {
                let first = i == 0 || self.rows[i - 1].thread != self.rows[i].thread;
                if let (true, Some(t), Some(c)) = (first, self.rows[i].thread, last_code) {
                    anchors.insert(t, c);
                }
                match last {
                    Some(r) => units[unit_of[r]].1.push(i),
                    None => units.push((DRow::Margin(None, None, None), vec![i])),
                }
                i += 1;
                continue;
            }
            let mut push = |u: DRow, rows: &[usize], units: &mut Vec<(DRow, Vec<usize>)>| {
                rows.iter().for_each(|&r| unit_of[r] = units.len());
                units.push((u, vec![]));
            };
            match kind(i) {
                Kind::Del | Kind::Add if self.split => {
                    // The run, its comments aside: the k-th removed line beside the k-th added.
                    let run: Vec<usize> = (i..n).take_while(|&j| margin(j) || matches!(kind(j), Kind::Del | Kind::Add)).collect();
                    let dels: Vec<usize> = run.iter().copied().take_while(|&j| margin(j) || kind(j) == Kind::Del).filter(|&j| !margin(j)).collect();
                    let rest = run.iter().copied().skip_while(|&j| margin(j) || kind(j) == Kind::Del);
                    let adds: Vec<usize> = rest.take_while(|&j| margin(j) || kind(j) == Kind::Add).filter(|&j| !margin(j)).collect();
                    let end = adds.last().or(dels.last()).copied().unwrap_or(i);
                    for k in 0..dels.len().max(adds.len()) {
                        let (l, r) = (dels.get(k).copied(), adds.get(k).copied());
                        let rows: Vec<usize> = l.into_iter().chain(r).collect();
                        push(DRow::Pair(l, r), &rows, &mut units);
                    }
                    // Its comments go with the line before them.
                    for j in i..=end {
                        if margin(j) {
                            let before = (i..j).rev().find(|&b| !margin(b));
                            let first = self.rows[j - 1].thread != self.rows[j].thread;
                            if let (true, Some(t), Some(b)) = (first, self.rows[j].thread, before) {
                                anchors.insert(t, b);
                            }
                            match before {
                                Some(b) => units[unit_of[b]].1.push(j),
                                None => units.last_mut().expect("a unit before").1.push(j),
                            }
                        } else {
                            last_code = Some(j);
                        }
                    }
                    last = Some(end);
                    i = end + 1;
                    continue;
                }
                Kind::Ctx if self.split => push(DRow::Pair(Some(i), Some(i)), &[i], &mut units),
                _ => push(DRow::Full(i), &[i], &mut units),
            }
            if code(i) {
                last_code = Some(i);
            }
            last = Some(i);
            i += 1;
        }
        self.margin_anchor = anchors;
        self.marks = self.find_marks();
        let mut d: Vec<DRow> = vec![];
        if !self.comments_in_pane() {
            for (u, _) in units {
                if u != DRow::Margin(None, None, None) {
                    d.push(u);
                }
            }
            return self.set_drows(d);
        }
        // The margin: comments wait in `pending` for a free row beside code.
        let mut pending: std::collections::VecDeque<usize> = Default::default();
        // The first screen row whose margin is free: at the top, below the pinned title.
        let mut free = usize::from(self.rows.first().is_some_and(|r| r.file == INFO && r.line.kind == Kind::File));
        let flush = |d: &mut Vec<DRow>, pending: &mut std::collections::VecDeque<usize>| {
            d.extend(pending.drain(..).map(|c| DRow::Margin(None, None, Some(c))));
        };
        for (u, comments) in units {
            let (l, r) = match u {
                DRow::Full(r) if code(r) => (Some(r), None),
                // A section's text (the description, say): what's attached to it sits beside it.
                DRow::Full(r) if self.rows[r].main.is_some() && self.rows[r].line.kind != Kind::File => (Some(r), None),
                DRow::Pair(l, r) => (l, r),
                DRow::Margin(..) => (None, None),
                DRow::Full(r) => {
                    // File headers and the PR section are full width: comments still waiting
                    // get spacer rows first, so they stay with their file.
                    flush(&mut d, &mut pending);
                    d.push(DRow::Full(r));
                    free = free.max(d.len()); // (not back up under the summaries, at the top)
                    (None, None)
                }
            };
            if l.is_some() || r.is_some() {
                let c = if d.len() >= free { pending.pop_front() } else { None };
                d.push(DRow::Margin(l, r, c));
                if c.is_some() {
                    free = d.len();
                }
            }
            for i in comments {
                // A box starts beside its line (the row just laid out) when that's free.
                let first = i == 0 || self.rows[i - 1].thread != self.rows[i].thread;
                let at = if first { d.len().saturating_sub(1).max(free) } else { free };
                match d.get(at) {
                    Some(&DRow::Margin(l, r, None)) if pending.is_empty() && (l.is_some() || r.is_some()) => {
                        d[at] = DRow::Margin(l, r, Some(i));
                        free = at + 1;
                    }
                    _ => pending.push_back(i),
                }
            }
        }
        flush(&mut d, &mut pending);
        self.set_drows(d);
    }

    /// Screen rows, and each row's screen row (a hidden comment's: the line it's on).
    pub(super) fn set_drows(&mut self, d: Vec<DRow>) {
        let mut d_of = vec![usize::MAX; self.rows.len()];
        for (di, dr) in d.iter().enumerate() {
            match *dr {
                DRow::Full(r) => d_of[r] = di,
                DRow::Pair(l, r) => l.into_iter().chain(r).for_each(|x| d_of[x] = di),
                DRow::Margin(l, r, c) => l.into_iter().chain(r).chain(c).for_each(|x| d_of[x] = di),
            }
        }
        let mut prev = 0;
        for x in &mut d_of {
            if *x == usize::MAX {
                *x = prev;
            }
            prev = *x;
        }
        (self.drows, self.d_of) = (d, d_of);
    }

    /// A comment on a file, or PR text attached to the description (its facts, conversation,
    /// your review; Claude's summary): it goes in the margin, beside what it's about; a file's
    /// isn't shown while that's hidden.
    pub(super) fn in_margin(&self, r: usize) -> bool {
        let row = &self.rows[r];
        match row.file == INFO {
            false => row.thread.is_some(),
            // (The pane hidden: inline, after the description.)
            true => row.attached && self.comments_in_pane(),
        }
    }

    /// Comments on files are in the pane: it's shown, on that tab.
    pub(super) fn comments_in_pane(&self) -> bool {
        self.margin_active
    }

    /// Row `r` is in the pane now: a comment on a file, the pane shown.
    pub(super) fn in_pane(&self, r: usize) -> bool {
        self.comments_in_pane() && self.in_margin(r)
    }

    /// The cursor is in the margin (on a comment), not the code. Worked out from where the
    /// cursor is, so every way of moving it (search, word motions, jumps) keeps it right.
    pub(super) fn margin_side(&self) -> bool {
        self.margin_row(self.head.row)
    }

    /// In the margin, as laid out now.
    pub(super) fn margin_row(&self, r: usize) -> bool {
        self.in_pane(r)
    }

    /// In margin mode, the thread whose box is "lit": the one the cursor is in, or one about
    /// the code line it's on. So you can tell which line a box (pushed down) is about.
    pub(super) fn lit_thread(&self) -> Option<usize> {
        if !self.comments_in_pane() {
            return None;
        }
        match self.margin_side() {
            true => self.rows[self.head.row].thread,
            // On its last line first, then anywhere in its range.
            false => self.margin_anchor.iter().find(|&(_, &c)| c == self.head.row).map(|(&t, _)| t).or_else(|| {
                self.margin_anchor.keys().copied().find(|&t| self.code_span(t).is_some_and(|(a, b)| (a..=b).contains(&self.head.row)))
            }),
        }
    }

    /// With the cursor in a margin comment: the code lines it's about (tinted like the cursor's).
    pub(super) fn lit_code(&self) -> Option<(usize, usize)> {
        // (Not with the files' list focused: only its selection's lit then.)
        (self.margin_side() && !self.files_focus).then(|| self.code_span(self.rows[self.head.row].thread?)).flatten()
    }

    /// The rows of the code thread `t` is about, first to last (its anchor row): a range
    /// comment (`start_line`) covers the lines from there, as far as they're shown.
    pub(super) fn code_span(&self, t: usize) -> Option<(usize, usize)> {
        let last = *self.margin_anchor.get(&t)?;
        let th = &self.threads[t];
        let Some(start) = th.start_line.filter(|&s| th.line.is_some_and(|l| s < l)) else { return Some((last, last)) };
        let no = |r: &Row| if th.left { r.line.old } else { r.line.new };
        let file = self.rows[last].file;
        let first = (0..last)
            .rev()
            .take_while(|&r| self.rows[r].file == file)
            .filter(|&r| self.rows[r].thread.is_none())
            .take_while(|&r| no(&self.rows[r]).is_none_or(|n| n >= start))
            .last()
            .unwrap_or(last);
        // From its first line itself (not a removed line or a hunk header just above it).
        let first = (first..=last).find(|&r| self.rows[r].thread.is_none() && no(&self.rows[r]).is_some()).unwrap_or(last);
        Some((first, last))
    }

    /// The comment marks for the code's right edge (see `App::marks`): over a comment's lines
    /// `╎`, at its last a dot; open comments over resolved / outdated ones, a dot over a line.
    fn find_marks(&self) -> HashMap<(usize, bool), u8> {
        let mut marks: HashMap<(usize, bool), u8> = HashMap::new();
        for &t in self.margin_anchor.keys() {
            let Some((first, last)) = self.code_span(t) else { continue };
            let th = &self.threads[t];
            let open = !th.stale();
            let dot = match (open, agent::claude_wrote(th)) {
                (false, _) => MARK_STALE_DOT,
                (true, true) => MARK_CLAUDE_DOT,
                (true, false) => MARK_DOT,
            };
            let no = |r: usize| if th.left { self.rows[r].line.old } else { self.rows[r].line.new };
            for r in (first..=last).filter(|&r| self.rows[r].thread.is_none() && no(r).is_some()) {
                let m = if r == last { dot } else if open { MARK_LINE } else { MARK_STALE_LINE };
                let e = marks.entry((r, th.left)).or_default();
                *e = (*e).max(m);
            }
        }
        marks
    }

    /// The row to put the cursor on for screen row `d`, preferring the current side.
    pub(super) fn pick(&self, d: usize) -> usize {
        match self.drows[d] {
            DRow::Full(r) => r,
            DRow::Pair(l, r) => if self.right { r.or(l) } else { l.or(r) }.unwrap_or(0),
            DRow::Margin(l, r, c) => {
                let code = if self.right { r.or(l) } else { l.or(r) };
                if self.margin_side() { c.or(code) } else { code.or(c) }.unwrap_or(0)
            }
        }
    }

    pub(super) fn toggle_split(&mut self) {
        self.split = !self.split;
        self.layout();
        self.msg = if self.split { "split view (Ctrl-h / Ctrl-l switch sides)" } else { "unified view" }.into();
    }

    /// Screen row of the cursor.
    pub(super) fn hd(&self) -> usize {
        self.d_of[self.head.row]
    }
}
