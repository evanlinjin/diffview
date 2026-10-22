//! Rows onto the screen: one per screen row, side by side in split view, or with comments
//! beside the code in the right margin.

use super::*;

#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum BoxPart {
    Top,
    Divider,
    Body,
    Bottom,
}

/// A screen row: full width, a side-by-side pair (old line, new line) in split view, or code
/// (one row, or a pair in split view) with a comment row beside it in the right margin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum DRow {
    Full(usize),
    Pair(Option<usize>, Option<usize>),
    /// (code row or old side, new side in split view, comment row in the margin); a code-less
    /// one is a spacer, where a file's comments need more room than its code has.
    Margin(Option<usize>, Option<usize>, Option<usize>),
}

/// The pane's tabs as they're shown (`pane_tabs`): its comments (the whole diff), Claude's
/// (the Claude view, its comments beside its code), your pending review's.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Bar {
    Comments,
    Claude,
    Pending,
}

/// The comment margin's width: about a third of the window, within these bounds, or everything
/// beyond the code side's most (it doesn't read better any wider); and the least code text
/// width beside it (narrower windows show comments inline).
pub(super) const MARGIN_MIN: usize = 60;

/// The least code text width beside it for each side of split view.
pub(super) const MARGIN_SPLIT_MIN: usize = 70;

/// How much `Ctrl-w ,` / `.` change its width by.
pub(super) const MARGIN_STEP: isize = 6;

/// The least it takes in a narrow window, when it's shown there.
pub(super) const MARGIN_NARROW: usize = 36;

pub(super) const MARGIN_MAX: usize = 80;

pub(super) const MARGIN_CODE_MIN: usize = 80;

pub(super) const MARGIN_CODE_MAX: usize = 200;

impl App {
    /// The width of the comment box row `r` is in, drawn `tw` wide: as wide as the comment text
    /// wraps (+ margin), not the whole screen; in the comment pane, the pane's width.
    pub(super) fn box_w(&self, r: usize, tw: usize) -> usize {
        if self.margin_row(r) { tw } else { tw.min(self.wrap_w + 2) }
    }

    /// The columns row `r`'s text has, of `tw` (its column's, less a code row's mark column):
    /// in a box, up to its right side (the box as wide as the comment it's in, a reply's guide
    /// inside it); else all of it, less a guide. The one rule for drawing a row and keeping the
    /// cursor in view on it.
    pub(super) fn text_w(&self, r: usize, tw: usize) -> usize {
        let guide = self.rows.get(r).map_or(0, Row::guide_w);
        match self.box_part(r) {
            Some(_) => self.box_w(r, tw).saturating_sub(1 + guide),
            None => tw.saturating_sub(guide),
        }
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
                let shown: Vec<&&Thread> = about_code.iter().filter(|t| self.in_view_of(v, t)).collect();
                // How many are done of them, resolved or outdated (its code changed: shown folded,
                // as a resolved one is), as the title's files viewed: "1/3", "3/3" when all are;
                // your pending comments, a count (nothing of them to resolve).
                let done = shown.iter().filter(|t| t.stale()).count();
                let l = match (shown.len(), b) {
                    // (As the title counts them: your pending comments, wherever they are.)
                    (_, Bar::Pending) => match self.pending_count() {
                        0 => format!(" {name} "),
                        n => format!(" {name} {n} "),
                    },
                    (0, _) => format!(" {name} "),
                    (n, _) => format!(" {name} {done}/{n} "),
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

    /// Claude's review is what's beside the description: the Claude view, or an unpublished
    /// draft (whose one tab is Claude's).
    pub(super) fn claude_side(&self) -> bool {
        self.view == View::Claude || self.unpublished()
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

    /// What row `r` scrolls sideways with: the diff, by `left`; a comment's code block or
    /// table (from its first row), by its own offset; nothing (`None`): a comment's prose
    /// wraps, and the pinned title isn't the diff's.
    pub(super) fn scroller(&self, r: usize) -> Option<Scroller> {
        match self.box_part(r) {
            None if r == 0 && self.pin => None,
            None => Some(Scroller::Diff),
            Some(_) => self.code_block(r).map(Scroller::Block),
        }
    }

    /// How far row `r`'s text is scrolled sideways: a code block only while the cursor's in
    /// it (`code_at`).
    pub(super) fn hscroll(&self, r: usize) -> usize {
        match self.scroller(r) {
            Some(Scroller::Diff) if self.soft_wrap => 0,
            Some(Scroller::Diff) => self.left,
            Some(Scroller::Block(b)) if self.code_at == Some(b) => self.code_left,
            _ => 0,
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
        self.soft_wrap = v["wrap"].as_bool().unwrap_or(false);
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
        // Screen rows, and each one's part of its line (a long line wrapped: one each).
        let (mut d, mut ds): (Vec<DRow>, Vec<usize>) = (vec![], vec![]);
        if !self.comments_in_pane() {
            for (u, _) in units {
                if u != DRow::Margin(None, None, None) {
                    let n = match u {
                        DRow::Full(r) => self.segs(Some(r), None),
                        DRow::Pair(l, r) | DRow::Margin(l, r, _) => self.segs(l, r),
                    };
                    d.extend(std::iter::repeat_n(u, n));
                    ds.extend(0..n);
                }
            }
            return self.set_drows(d, ds);
        }
        // The margin: comments wait in `pending` for a free row beside code.
        let mut pending: std::collections::VecDeque<usize> = Default::default();
        // The first screen row whose margin is free: at the top, below the pinned title.
        let mut free = usize::from(self.rows.first().is_some_and(|r| r.file == INFO && r.line.kind == Kind::File));
        let flush = |d: &mut Vec<DRow>, ds: &mut Vec<usize>, pending: &mut std::collections::VecDeque<usize>| {
            ds.extend(pending.iter().map(|_| 0));
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
                    flush(&mut d, &mut ds, &mut pending);
                    d.push(DRow::Full(r));
                    ds.push(0);
                    free = free.max(d.len()); // (not back up under the summaries, at the top)
                    (None, None)
                }
            };
            // (A long line wrapped: a row for each part, comments beside them as beside any.)
            let line_at = d.len();
            for k in 0..if l.is_some() || r.is_some() { self.segs(l, r) } else { 0 } {
                let c = if d.len() >= free { pending.pop_front() } else { None };
                d.push(DRow::Margin(l, r, c));
                ds.push(k);
                if c.is_some() {
                    free = d.len();
                }
            }
            for i in comments {
                // A box starts beside its line (the row just laid out) when that's free.
                let first = i == 0 || self.rows[i - 1].thread != self.rows[i].thread;
                let at = if first { line_at.min(d.len().saturating_sub(1)).max(free) } else { free };
                match d.get(at) {
                    Some(&DRow::Margin(l, r, None)) if pending.is_empty() && (l.is_some() || r.is_some()) => {
                        d[at] = DRow::Margin(l, r, Some(i));
                        free = at + 1;
                    }
                    _ => pending.push_back(i),
                }
            }
        }
        flush(&mut d, &mut ds, &mut pending);
        self.set_drows(d, ds);
    }

    /// Whether row `r` wraps (`soft_wrap`): code does.
    pub(super) fn wraps(&self, r: usize) -> bool {
        self.soft_wrap && self.rows[r].line.kind.is_code()
    }

    /// Which side row `r` is drawn on in split view: removed lines left, added right, an
    /// unchanged line (both) the cursor's.
    pub(super) fn side_of(&self, r: usize) -> bool {
        match self.rows[r].line.kind {
            Kind::Del => false,
            Kind::Add => true,
            _ => self.right,
        }
    }

    /// The text width row `r` wraps to, on `right`'s side (one column: the only width).
    fn wrap_w(&self, r: usize, right: bool) -> usize {
        let (one, left, rw) = self.wrap_cols;
        match self.split && self.rows[r].line.kind.is_code() {
            true if right => rw,
            true => left,
            false => one,
        }
    }

    /// Where row `r`'s parts start, wrapped at text width `w` (char indices): each as much as
    /// fits, never a char cut in two (a tab, a wide character, goes on whole to the next).
    /// One part, not wrapping, or the width not known yet (nothing drawn).
    pub(super) fn parts(&self, r: usize, w: usize) -> Vec<usize> {
        let mut starts = vec![0];
        if !self.wraps(r) || w == 0 {
            return starts;
        }
        let mut used = 0;
        for (i, c) in self.rows[r].line.text.chars().enumerate() {
            let cw = super::row::char_w(c);
            if used > 0 && used + cw > w {
                starts.push(i);
                used = 0;
            }
            used += cw;
        }
        starts
    }

    /// The part of row `r` char `col` is in (past the end: the last), on the side it's drawn,
    /// and where that part starts.
    pub(super) fn part_of(&self, r: usize, col: usize) -> (usize, usize) {
        let starts = self.parts(r, self.wrap_w(r, self.side_of(r)));
        let k = starts.partition_point(|&s| s <= col).saturating_sub(1);
        (k, starts[k])
    }

    /// The column row `r`'s text is drawn from where `col` is: its part's start, wrapped; else
    /// as it's scrolled sideways.
    pub(super) fn text_left(&self, r: usize, col: usize) -> usize {
        match self.wraps(r) {
            true => self.col_of(r, self.part_of(r, col).1),
            false => self.hscroll(r),
        }
    }

    /// How many screen rows a line takes, wrapped, on its side or sides (a pair: the longer).
    pub(super) fn segs(&self, l: Option<usize>, r: Option<usize>) -> usize {
        let paired = self.split && l.or(r).is_some_and(|x| self.rows[x].line.kind.is_code());
        let n = |x: Option<usize>, right: bool| x.map_or(1, |x| self.parts(x, self.wrap_w(x, right)).len());
        match paired {
            true => n(l, false).max(n(r, true)),
            false => n(l.or(r), false),
        }
    }

    /// Screen rows and each one's part of its line (wrapped: 0, 1, …); and each row's screen
    /// row (its first; a hidden comment's: the line it's on).
    pub(super) fn set_drows(&mut self, d: Vec<DRow>, dseg: Vec<usize>) {
        let mut d_of = vec![usize::MAX; self.rows.len()];
        for (di, dr) in d.iter().enumerate().filter(|&(di, _)| dseg[di] == 0) {
            match *dr {
                DRow::Full(r) => d_of[r] = di,
                DRow::Pair(l, r) => l.into_iter().chain(r).for_each(|x| d_of[x] = di),
                DRow::Margin(l, r, c) => l.into_iter().chain(r).chain(c).for_each(|x| d_of[x] = di),
            }
        }
        // (A comment beside a line's later part: there.)
        for (di, dr) in d.iter().enumerate() {
            if let DRow::Margin(_, _, Some(c)) = *dr {
                d_of[c] = di;
            }
        }
        self.dseg = dseg;
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
            let dot = match (open, th.by_claude()) {
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

    /// `Ctrl-w r`: long code lines wrap, or scroll sideways (again).
    pub(super) fn toggle_wrap(&mut self) {
        self.soft_wrap = !self.soft_wrap;
        self.left = 0;
        self.save_pane("wrap", self.soft_wrap.into());
        self.relayout_keeping_top();
        self.msg = if self.soft_wrap { "long lines wrap (Ctrl-w r: scroll sideways)" } else { "long lines scroll sideways (Ctrl-w r: wrap)" }.into();
    }

    /// Lay the rows out again (wrapped otherwise), the line at the top of the screen kept there.
    pub(super) fn relayout_keeping_top(&mut self) {
        let top = (self.top < self.drows.len()).then(|| self.pick(self.top));
        self.layout();
        if let Some(r) = top {
            self.top = self.d_of[r];
        }
    }

    pub(super) fn toggle_split(&mut self) {
        self.split = !self.split;
        self.layout();
        self.msg = if self.split { "split view (Ctrl-h / Ctrl-l switch sides)" } else { "unified view" }.into();
    }

    /// Screen row of the cursor.
    pub(super) fn hd(&self) -> usize {
        // (Wrapped: the part of its line the cursor's in.)
        self.d_of[self.head.row] + self.part_of(self.head.row, self.head.col).0
    }

    /// The cursor's place for screen row `d`, keeping its column within a wrapped line's part
    /// (`within`: how far into its part it is): the row, and the column there.
    pub(super) fn pick_at(&self, d: usize, within: usize) -> (usize, usize) {
        let r = self.pick(d);
        let k = self.dseg.get(d).copied().unwrap_or(0);
        if !self.wraps(r) {
            return (r, within);
        }
        let starts = self.parts(r, self.wrap_w(r, self.side_of(r)));
        match starts.get(k) {
            // (A pair's shorter side, past its last part: its end.)
            None => (r, self.len(r)),
            Some(&s) => (r, (s + within).min(starts.get(k + 1).map_or(usize::MAX, |&e| e.saturating_sub(1)))),
        }
    }
    /// The width comments and PR text wrap to: the pane's, when it's shown (they're there).
    pub(super) fn info_w(&self) -> usize {
        if self.margin_active { self.margin_w - self.pane_gw() - 2 - self.pane_pad() } else { self.wrap_w }
    }

    /// The space inside the line between the code and the pane, as a column.
    pub(super) fn pane_pad(&self) -> usize {
        usize::from(!self.margin_float)
    }

    /// The pane's gutter: a box's left side and a space (`│  `); as a column, where boxes are
    /// open on their left, just a space (their rules come from the line beside the code).
    pub(super) fn pane_gw(&self) -> usize {
        if self.margin_float { 3 } else { 1 }
    }

    /// Drawn at the right end of a header line: the PR's state on the PR's, the file's GitHub
    /// "Viewed" state on a file's.
    pub(super) fn header_tag(&self, file: usize) -> Option<(String, Style)> {
        if file == INFO {
            // (Open and conflicting with its base: said there, in red, wherever you are.)
            return self.pr_state.map(|(s, color)| match self.conflicts && matches!(s, "OPEN" | "DRAFT") {
                true => (format!(" {s} · conflicts "), chip("da3633")),
                false => (format!(" {s} "), chip(color)),
            });
        }
        let path = &self.files.get(file)?.path;
        if let Some(m) = self.marking.get(path) {
            let state = if m.on { "✓ Viewed ·" } else { "unmarking ·" };
            return Some(match m.retry {
                Some(at) => (format!("{state} retry in {}s ", at.saturating_duration_since(std::time::Instant::now()).as_secs() + 1), self.ui.warning),
                None => (format!("{state} saving… "), self.ui.stale),
            });
        }
        let (t, st) = match self.viewed.get(path)?.as_str() {
            "VIEWED" => ("✓ Viewed ", self.ui.plus),
            _ => ("↻ changed since viewed ", self.ui.warning),
        };
        Some((t.into(), st))
    }
}
