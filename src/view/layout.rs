//! Rows onto the screen: one per screen row, side by side in split view, or with comments
//! beside the code in the right margin.

use super::*;

impl App {
    /// Map rows to screen rows. Code runs down the left: one column, or in split view two,
    /// where each run of removed lines pairs with the added lines that follow it and unchanged
    /// lines show on both sides. Comments on files go in the margin on the right, each box
    /// beside its line (or just below the box before it, if that's still running there); code
    /// only gets spacer rows when a file's comments need more room than its code has (they
    /// never spill into the next file). With the margin hidden, or on its PR tab, they aren't
    /// shown, and the lines they're on have a mark in the gutter. With it shown, the PR section
    /// (but its header line) is on its PR tab, not here.
    pub(super) fn layout(&mut self) {
        let n = self.rows.len();
        let kind = |i: usize| self.rows[i].line.kind;
        // Code that a comment can sit beside: a diff line or a hunk header.
        let code = |i: usize| {
            let row = &self.rows[i];
            matches!(row.line.kind, Kind::Ctx | Kind::Add | Kind::Del | Kind::Hunk) && row.file != INFO
        };
        let margin = |i: usize| self.in_margin(i);
        let pr_tab: Vec<usize> = (0..n).filter(|&i| self.pr_row(i)).collect();
        // The main column's rows (full width, or code: one row, or in split view a pair), each
        // with the comment rows that follow its rows; and the code row each thread is about.
        let mut units: Vec<(DRow, Vec<usize>)> = vec![];
        let mut unit_of = vec![usize::MAX; n];
        let mut anchors = HashMap::new();
        let mut last: Option<usize> = None; // the last row in the main column, and the last code row
        let mut last_code: Option<usize> = None;
        let mut i = 0;
        while i < n {
            if self.pr_row(i) {
                i += 1;
                continue;
            }
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
        self.pr_rows = pr_tab;
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
        let mut free = 0; // the first screen row whose margin is free
        let flush = |d: &mut Vec<DRow>, pending: &mut std::collections::VecDeque<usize>| {
            d.extend(pending.drain(..).map(|c| DRow::Margin(None, None, Some(c))));
        };
        for (u, comments) in units {
            let (l, r) = match u {
                DRow::Full(r) if code(r) => (Some(r), None),
                DRow::Pair(l, r) => (l, r),
                DRow::Margin(..) => (None, None),
                DRow::Full(r) => {
                    // File headers and the PR section are full width: comments still waiting
                    // get spacer rows first, so they stay with their file.
                    flush(&mut d, &mut pending);
                    d.push(DRow::Full(r));
                    free = d.len();
                    (None, None)
                }
            };
            if l.is_some() || r.is_some() {
                let c = pending.pop_front();
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

    /// A comment on a file (not the PR section's): it goes in the margin, or isn't shown while
    /// that's hidden.
    pub(super) fn in_margin(&self, r: usize) -> bool {
        let row = &self.rows[r];
        row.thread.is_some() && row.file != INFO
    }

    /// PR text (the PR section but its header line), on the pane's PR tab while it's shown.
    pub(super) fn pr_row(&self, r: usize) -> bool {
        let row = &self.rows[r];
        self.margin_active && row.file == INFO && row.line.kind != Kind::File
    }

    /// Comments on files are in the pane: it's shown, on that tab.
    pub(super) fn comments_in_pane(&self) -> bool {
        self.margin_active && self.pane_tab == Tab::Comments
    }

    /// Row `r` is in the pane now: a comment (on that tab) or PR text (on the PR tab).
    pub(super) fn in_pane(&self, r: usize) -> bool {
        match self.pane_tab {
            Tab::Comments => self.comments_in_pane() && self.in_margin(r),
            Tab::Pr => self.pr_row(r),
        }
    }

    /// The cursor is on the pane's PR tab.
    pub(super) fn on_pr_tab(&self) -> bool {
        self.pane_tab == Tab::Pr && self.pr_row(self.head.row)
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
        self.margin_side().then(|| self.code_span(self.rows[self.head.row].thread?)).flatten()
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
