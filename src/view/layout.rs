//! Rows onto the screen: one per screen row, side by side in split view, or with comments
//! beside the code in the right margin.

use super::*;

impl App {
    /// Map rows to screen rows. In split view, each run of removed lines pairs with the added
    /// lines that follow it; unchanged lines show on both sides; everything else is full width.
    pub(super) fn layout(&mut self) {
        if self.margin_active && !self.split {
            return self.layout_margin();
        }
        let n = self.rows.len();
        let kind = |i: usize| self.rows[i].line.kind;
        let mut d = vec![];
        let mut i = 0;
        while i < n {
            match kind(i) {
                _ if !self.split => d.push(DRow::Full(i)),
                Kind::Ctx => d.push(DRow::Pair(Some(i), Some(i))),
                Kind::Del | Kind::Add => {
                    let dels = (i..n).take_while(|&j| kind(j) == Kind::Del).count();
                    let adds = (i + dels..n).take_while(|&j| kind(j) == Kind::Add).count();
                    for k in 0..dels.max(adds) {
                        d.push(DRow::Pair((k < dels).then_some(i + k), (k < adds).then_some(i + dels + k)));
                    }
                    i += dels + adds;
                    continue;
                }
                _ => d.push(DRow::Full(i)),
            }
            i += 1;
        }
        self.set_drows(d);
    }

    pub(super) fn set_drows(&mut self, d: Vec<DRow>) {
        let mut d_of = vec![0; self.rows.len()];
        for (di, dr) in d.iter().enumerate() {
            match *dr {
                DRow::Full(r) => d_of[r] = di,
                DRow::Pair(l, r) | DRow::Margin(l, r) => l.into_iter().chain(r).for_each(|x| d_of[x] = di),
            }
        }
        (self.drows, self.d_of) = (d, d_of);
    }

    /// The cursor is in the margin (on a comment), not the code. Worked out from where the
    /// cursor is, so every way of moving it (search, word motions, jumps) keeps it right.
    pub(super) fn margin_side(&self) -> bool {
        self.margin_row(self.head.row)
    }

    /// A review thread's row in a file (not the PR section's comments): in margin mode, it goes
    /// in the margin.
    pub(super) fn margin_row(&self, r: usize) -> bool {
        let row = &self.rows[r];
        self.margin_active && ((row.thread.is_some() && row.file != INFO) || row.aside)
    }

    /// Comments in the right margin: each thread's box starts beside its line, or just below
    /// the box before it if that's still running there; code only gets spacer rows when a
    /// file's comments need more room than its code (they never spill into the next file).
    pub(super) fn layout_margin(&mut self) {
        let n = self.rows.len();
        let mut d: Vec<DRow> = vec![];
        let mut pending: std::collections::VecDeque<usize> = Default::default();
        let mut free = 0; // the first screen row whose margin is free
        let mut anchors = HashMap::new();
        let mut last_code = None;
        let mut desc_first = None; // the description's first screen row: the conversation's top
        let flush = |d: &mut Vec<DRow>, pending: &mut std::collections::VecDeque<usize>| {
            d.extend(pending.drain(..).map(|c| DRow::Margin(None, Some(c))));
        };
        for i in 0..n {
            let row = &self.rows[i];
            if self.margin_row(i) {
                let first = i == 0 || self.rows[i - 1].thread != row.thread || (row.aside && !self.rows[i - 1].aside);
                if let (true, Some(t), Some(c)) = (first, row.thread, last_code) {
                    anchors.insert(t, c);
                }
                // A box starts beside its line (the code row just before it) when that's free;
                // the conversation, beside the top of the description.
                let at = match (first, row.aside) {
                    (true, true) => desc_first.unwrap_or(d.len()).max(free),
                    (true, false) => d.len().saturating_sub(1).max(free),
                    (false, _) => free,
                };
                if pending.is_empty() && matches!(d.get(at), Some(DRow::Margin(Some(_), None))) {
                    if let DRow::Margin(code, _) = d[at] {
                        d[at] = DRow::Margin(code, Some(i));
                    }
                    free = at + 1;
                } else {
                    pending.push_back(i);
                }
                continue;
            }
            let code = (matches!(row.line.kind, Kind::Ctx | Kind::Add | Kind::Del | Kind::Hunk) && row.file != INFO) || row.lead;
            if row.lead && desc_first.is_none() {
                desc_first = Some(d.len());
            }
            if code {
                last_code = Some(i);
                let c = pending.pop_front();
                d.push(DRow::Margin(Some(i), c));
                if c.is_some() {
                    free = d.len();
                }
            } else {
                // File headers and the PR section are full width: comments still waiting get
                // spacer rows first, so they stay with their file.
                flush(&mut d, &mut pending);
                d.push(DRow::Full(i));
                free = d.len();
            }
        }
        flush(&mut d, &mut pending);
        self.margin_anchor = anchors;
        self.set_drows(d);
    }

    /// In margin mode, the thread whose box is "lit": the one the cursor is in, or one about
    /// the code line it's on. So you can tell which line a box (pushed down) is about.
    pub(super) fn lit_thread(&self) -> Option<usize> {
        if !self.margin_active || self.split {
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
            DRow::Margin(l, r) => if self.margin_side() { r.or(l) } else { l.or(r) }.unwrap_or(0),
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
