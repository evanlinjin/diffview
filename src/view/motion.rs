//! Cursor movement: motions, word/bracket/char jumps, search, scrolling, jump labels.
use super::*;

/// `editor.jump-label-alphabet` from the Helix config, else Helix's default.
pub(super) fn jump_alphabet() -> Vec<char> {
    let cfg = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|_| std::env::var("HOME").map(|h| Path::new(&h).join(".config")))
        .map(|d| std::fs::read_to_string(d.join("helix/config.toml")).unwrap_or_default())
        .unwrap_or_default();
    // ponytail: line scan instead of a TOML parser; fine for `key = "value"` on one line.
    cfg.lines()
        .filter_map(|l| l.trim().strip_prefix("jump-label-alphabet")?.trim().strip_prefix('=')?.trim().strip_prefix('"')?.strip_suffix('"').map(str::to_owned))
        .next()
        .filter(|a| !a.is_empty())
        .unwrap_or_else(|| "abcdefghijklmnopqrstuvwxyz".into())
        .chars()
        .collect()
}

impl App {
    /// Ctrl-h / Ctrl-l: move to the old / new side of the current split row; with comments in
    /// the margin, from the code (its new side) into the margin (the comment beside it, else
    /// the nearest one) and back (the code line beside it, else the nearest above).
    pub(super) fn side(&mut self, right: bool) {
        // Out of a maximized pane: laid out as before first, to go into what's beside it; still
        // in it (as far as it goes, or the other half of the code), maximized still.
        if let Some(pane) = self.zoom.take() {
            self.relayout();
            self.go_side(right);
            if !self.files_focus && self.margin_side() == pane {
                self.zoom = Some(pane);
            }
            return;
        }
        self.go_side(right);
    }

    fn go_side(&mut self, right: bool) {
        if self.files_focus {
            self.files_focus = !right;
            return;
        }
        let hd = self.hd();
        let at_edge = self.at_edge(hd, right);
        // The code's left edge: on into the files' list.
        if !right && self.files_w > 0 && self.list_has_spots() && !self.margin_side() && at_edge {
            (self.files_focus, self.files_sel) = (true, self.current_spot());
            return self.msg = "contents: j / k move, Enter goes there, Ctrl-l back".into();
        }
        // The right edge: on into the pane.
        let into_margin = self.margin_active && right && at_edge && !self.margin_side();
        // Out of it (however you got in), onto the code beside it; your side the one it lands
        // on (a comment's line: the side it's about).
        let leaving = self.margin_side() && !right;
        // By what a comment is about, not where its box happens to sit (boxes stack down the
        // pane): from a comment to its line, and from a line to the comment about it.
        if leaving
            && let Some(t) = self.rows[self.head.row].thread
            && let Some(&line) = self.margin_anchor.get(&t)
        {
            self.right = !self.threads[t].left;
            return self.goto(line, 0);
        }
        if into_margin
            && let Some(t) = self.lit_thread()
            && let Some(r) = self.rows.iter().position(|r| r.thread == Some(t))
        {
            return self.goto(r, 0);
        }
        if into_margin || leaving {
            // (Into the pane: a comment; out of it: the code on that screen row, the side with
            // a part of it there, the right first.)
            // (Into the pane: a comment, here or below, else above; out of it: the code on that
            // screen row, a header too, the side with a part of it there, the right first, here
            // or above.)
            let at = |d: usize| match right {
                true => self.note_at(d).map(|c| Pos { row: c, col: 0 }),
                false => self.span(d, true).or_else(|| self.span(d, false)).map(|s| Pos { row: s.row, col: s.chars.start }),
            };
            let n = self.drows.len();
            let near = match right {
                true => Self::nearest(hd, 1, 0..n).find_map(at),
                false => Self::nearest(hd, -1, 0..hd + 1).find_map(at),
            };
            return match near {
                Some(p) => {
                    if leaving {
                        self.right = self.rows[p.row].line.kind != Kind::Del;
                    }
                    self.goto(p.row, p.col)
                }
                None => self.msg = if right { "no comments here" } else { "no code here" }.into(),
            };
        }
        if self.margin_side() {
            return; // already as far right as it goes
        }
        // To the other side's line on this screen row (its part, wrapped), as far into it; a
        // short one already ended above: its end. (A hatch, no line there: staying.)
        let Some(r) = self.side_row(hd, right) else { return };
        let x = self.cursor_x();
        let p = self.stop_on(hd, right, x, false).unwrap_or(Pos { row: r, col: self.len(r).saturating_sub(1) });
        self.right = right;
        self.goto(p.row, p.col);
    }

    /// The next (`fwd`) or previous row after `row` on the same side as it: the code or the
    /// comments pane (they're interleaved in `rows`, but not on screen), for word motions to
    /// carry on along a line of text rather than jump across.
    fn next_row_here(&self, row: usize, fwd: bool) -> Option<usize> {
        // (in the pane); a file's comments with the pane hidden aren't on any side (PR text is:
        // after the description).
        let side = |r: usize| (!(self.rows[r].thread.is_some() && !self.rows[r].attached && !self.in_pane(r))).then(|| self.in_pane(r));
        let here = side(row);
        let mut rs: Box<dyn Iterator<Item = usize>> = if fwd { Box::new(row + 1..self.rows.len()) } else { Box::new((0..row).rev()) };
        rs.find(|&r| side(r).is_some() && side(r) == here)
    }

    /// Move the head; outside select mode the anchor follows.
    pub(super) fn goto(&mut self, row: usize, col: usize) {
        let row = row.min(self.rows.len() - 1);
        self.set_head(Pos { row, col: col.min(self.len(row).saturating_sub(1)) });
        if !self.select {
            self.anchor = self.head;
        }
    }

    /// The cursor to `p`, every move of it through here: a vertical move's kept column let go.
    /// (The side you chose, `right`, stays: the side it's drawn on comes from the line,
    /// `head_right`.)
    pub(super) fn set_head(&mut self, p: Pos) {
        self.head = p;
        self.want_x = None;
    }

    /// Whether the cursor's at the code's edge toward `right` (the pane's side, else the files'
    /// list's): not in split view, or on a full-width row (a header); on that half already; or
    /// with nothing on it (a hatch) to go to.
    fn at_edge(&self, d: usize, right: bool) -> bool {
        !self.paired_at(d) || self.head_right() == right || self.side_row(d, right).is_none()
    }

    /// Which half the cursor's drawn on in split view: a removed line's the left, an added
    /// line's the right, one on both (unchanged) the side you chose (`right`: Ctrl-h / Ctrl-l,
    /// a click on a half).
    pub(super) fn head_right(&self) -> bool {
        self.row_right(self.head.row)
    }

    /// Which half row `r` is drawn on in split view (for the cursor, on one on both: yours).
    pub(super) fn row_right(&self, r: usize) -> bool {
        match self.rows[r].line.kind {
            Kind::Del => false,
            Kind::Add => true,
            _ => self.right,
        }
    }

    /// Set a selection (motions like `w` select); in select mode only the head moves.
    pub(super) fn sel(&mut self, anchor: Pos, head: Pos) {
        self.set_head(head);
        if !self.select {
            self.anchor = anchor;
        }
    }

    pub(super) fn vmove(&mut self, d: isize) {
        // By screen rows (a wrapped line's parts too, as Helix moves), over those the cursor
        // can't stop on (`stop_at`), keeping its column.
        // (Overshooting into rows it can't stop on, as far as it can.)
        let x = self.cursor_x();
        let (hd, n) = (self.hd(), self.drows.len());
        let t = (hd as isize + d).clamp(0, n.saturating_sub(1) as isize) as usize;
        // (On past where it is only: nowhere to go, nothing.)
        let within = if d < 0 { 0..hd } else { hd + 1..n };
        let Some(p) = Self::nearest(t, if d < 0 { -1 } else { 1 }, within).find_map(|t| self.stop_at(t, x)) else { return };
        self.goto(p.row, p.col);
        self.want_x = Some(x);
    }

    pub(super) fn ordered(&self) -> (Pos, Pos) {
        (self.anchor.min(self.head), self.anchor.max(self.head))
    }

    pub(super) fn word(&mut self, m: char) {
        let big = m.is_ascii_uppercase();
        let class = |c: char| if c.is_whitespace() { 0 } else if big || c.is_alphanumeric() || c == '_' { 1 } else { 2 };
        let Pos { row, col: i } = self.head;
        let chars = self.chars(row);
        let n = chars.len();
        let cl = |j: usize| class(chars[j]);
        if m.eq_ignore_ascii_case(&'b') {
            if i == 0 || n == 0 {
                if let Some(r) = self.next_row_here(row, false) {
                    let p = Pos { row: r, col: self.len(r).saturating_sub(1) };
                    self.sel(p, p);
                }
                return;
            }
            let start = if cl(i) != cl(i - 1) { i - 1 } else { i };
            let mut j = start;
            while j > 0 && cl(j) == 0 {
                j -= 1;
            }
            let c = cl(j);
            while j > 0 && cl(j - 1) == c {
                j -= 1;
            }
            self.sel(Pos { row, col: start }, Pos { row, col: j });
            return;
        }
        if i + 1 >= n {
            if let Some(r) = self.next_row_here(row, true) {
                let p = Pos { row: r, col: 0 };
                self.sel(p, p);
            }
            return;
        }
        let start = if cl(i) != cl(i + 1) { i + 1 } else { i };
        let mut j = start;
        if m.eq_ignore_ascii_case(&'w') {
            let c = cl(j);
            while j < n && cl(j) == c {
                j += 1;
            }
            while j < n && cl(j) == 0 {
                j += 1;
            }
            j -= 1;
        } else {
            while j + 1 < n && cl(j) == 0 {
                j += 1;
            }
            let c = cl(j);
            while j + 1 < n && cl(j + 1) == c {
                j += 1;
            }
        }
        self.sel(Pos { row, col: start }, Pos { row, col: j });
    }

    /// `mm`: jump to the bracket matching the one under the cursor. Brackets are counted
    /// through one version of the whole file (new side from an added/unchanged line, old side
    /// from a removed one); if the match is hidden between hunks, the hunk grows to show it.
    pub(super) fn match_bracket(&mut self) {
        let Pos { row, col } = self.head;
        let (fi, l) = (self.rows[row].file, self.rows[row].line.clone());
        let Some(f) = self.files.get(fi).filter(|_| l.kind.is_code()) else {
            return self.msg = "not on a bracket".into();
        };
        let Some(start) = f.full.iter().position(|x| x.old == l.old && x.new == l.new) else { return };
        let old = l.kind == Kind::Del;
        let side = |x: &DLine| match x.kind {
            Kind::Ctx => true,
            Kind::Add => !old,
            Kind::Del => old,
            _ => false,
        };
        let (open, close, fwd) = match l.text.chars().nth(col) {
            Some('(') => ('(', ')', true),
            Some('[') => ('[', ']', true),
            Some('{') => ('{', '}', true),
            Some(')') => ('(', ')', false),
            Some(']') => ('[', ']', false),
            Some('}') => ('{', '}', false),
            _ => return self.msg = "not on a bracket".into(),
        };
        let lines: Vec<usize> = match fwd {
            true => (start..f.full.len()).filter(|&j| side(&f.full[j])).collect(),
            false => (0..=start).rev().filter(|&j| side(&f.full[j])).collect(),
        };
        let mut depth = 0;
        let mut found = None;
        'scan: for j in lines {
            let text: Vec<char> = f.full[j].text.chars().collect();
            let cols: Vec<usize> = match (j == start, fwd) {
                (true, true) => (col..text.len()).collect(),
                (true, false) => (0..=col.min(text.len().saturating_sub(1))).rev().collect(),
                (false, true) => (0..text.len()).collect(),
                (false, false) => (0..text.len()).rev().collect(),
            };
            for i in cols {
                depth += match (text[i] == open, text[i] == close, fwd) {
                    (true, _, true) | (_, true, false) => 1,
                    (true, _, false) | (_, true, true) => -1,
                    _ => 0,
                };
                if depth == 0 {
                    found = Some((j, i));
                    break 'scan;
                }
            }
        }
        let Some((j, i)) = found else { return self.msg = "no matching bracket".into() };
        let (path, target) = (f.path.clone(), (f.full[j].old, f.full[j].new));
        let find = |app: &App| {
            app.rows.iter().position(|r| r.file == fi && r.thread.is_none() && (r.line.old, r.line.new) == target)
        };
        if find(self).is_none() {
            // Hidden in a gap: grow the cursor's hunk until it reaches the match.
            let e = |a| *self.extra.get(&(path.clone(), a)).unwrap_or(&0);
            let f = &self.files[fi];
            let ws = diff::windows(&f.full, DEFAULT_CONTEXT as usize, e, self.whole.contains(&path));
            if let Some((lo, hi, hunks)) = ws.into_iter().find(|w| w.0 <= start && start <= w.1) {
                let grow = if j > hi { j - hi } else { lo.saturating_sub(j) } as i64;
                for a in hunks {
                    *self.extra.entry((path.clone(), a)).or_insert(0) += grow;
                }
                self.reshape();
                self.msg = "expanded the hunk to reach the match".into();
            }
        }
        if let Some(r) = find(self) {
            self.goto(r, i);
        }
    }

    pub(super) fn find_char(&mut self, p: char, ch: char) {
        let chars = self.chars(self.head.row);
        let (i, n) = (self.head.col, chars.len());
        let hit = match p {
            'f' => (i + 1..n).find(|&j| chars[j] == ch),
            't' => (i + 2..n).find(|&j| chars[j] == ch).map(|j| j - 1),
            'F' => (0..i).rev().find(|&j| chars[j] == ch),
            _ => (0..i.saturating_sub(1)).rev().find(|&j| chars[j] == ch).map(|j| j + 1),
        };
        if let Some(col) = hit {
            self.sel(self.head, Pos { row: self.head.row, col });
        }
    }

    pub(super) fn line_select(&mut self) {
        let (s, e) = self.ordered();
        let end = self.len(e.row).saturating_sub(1);
        if s.col == 0 && e.col >= end && self.head == e && s != e {
            let r = (e.row + 1).min(self.rows.len() - 1);
            self.anchor = s;
            self.set_head(Pos { row: r, col: self.len(r).saturating_sub(1) });
        } else {
            self.anchor = Pos { row: s.row, col: 0 };
            self.set_head(Pos { row: e.row, col: end });
        }
    }

    pub(super) fn next_kind(&mut self, fwd: bool, is_start: impl Fn(&[Row], usize) -> bool) {
        let rows = &self.rows;
        let hit = if fwd {
            (self.head.row + 1..rows.len()).find(|&r| is_start(rows, r))
        } else {
            (0..self.head.row).rev().find(|&r| is_start(rows, r))
        };
        if let Some(r) = hit {
            self.goto(r, 0);
        }
    }

    pub(super) fn find(&mut self, fwd: bool) {
        if self.search.is_empty() {
            return;
        }
        let smart = !self.search.chars().any(char::is_uppercase);
        let q = if smart { self.search.to_ascii_lowercase() } else { self.search.clone() };
        let qlen = q.chars().count();
        let n = self.rows.len();
        let (s, _) = self.ordered();
        for k in 0..=n {
            let r = if fwd { (self.head.row + k) % n } else { (self.head.row + n - k) % n };
            let raw = &self.rows[r].line.text;
            let t = if smart { raw.to_ascii_lowercase() } else { raw.clone() };
            let mut hits = t.match_indices(q.as_str()).map(|(b, _)| t[..b].chars().count());
            let hit = if fwd {
                hits.find(|&c| k > 0 || c > self.head.col)
            } else {
                hits.filter(|&c| k > 0 || c < s.col).last()
            };
            if let Some(c) = hit {
                self.anchor = Pos { row: r, col: c };
                self.set_head(Pos { row: r, col: c + qlen - 1 });
                return;
            }
        }
        self.msg = format!("no match: {}", self.search);
    }

    pub(super) fn scroll(&mut self, d: isize) {
        self.scroll_code(d);
    }

    /// Scroll the code (with the comments beside it), whichever side the cursor is on: it moves
    /// along only if it would leave the screen. Returns whether it moved (not at its end
    /// already).
    pub(super) fn scroll_code(&mut self, d: isize) -> bool {
        let last = self.drows.len() as isize - 1;
        let was = self.top;
        self.top = (self.top as isize + d).clamp(0, last) as usize;
        if self.top == was {
            return false;
        }
        let so = super::render::scrolloff(self.height);
        let (lo, hi) = (self.top + so, (self.top + self.height).saturating_sub(so + 1));
        let into = |hd: usize| hd.clamp(lo, hi.max(lo)).min(last as usize);
        let hd = self.hd();
        if hd < lo || hd > hi {
            // Onto the screen, the nearest row first (toward the screen's middle): where it
            // can stop on its side; else (a tall pair's short side ended above, the pane's
            // boxes run out) the code on either side, or a comment.
            let x = self.cursor_x();
            let step: isize = if hd < lo { 1 } else { -1 };
            let screen = lo.min(last as usize)..hi.max(lo).min(last as usize) + 1;
            let order: Vec<usize> = Self::nearest(into(hd), step, screen).collect();
            let (right, pane) = (self.right, self.margin_side());
            let tries: &[(bool, bool)] = match pane {
                true => &[(right, true), (right, false), (!right, false)],
                false => &[(right, false), (!right, false), (right, true)],
            };
            let found = tries.iter().find_map(|&(side, pane)| order.iter().find_map(|&t| self.stop_on(t, side, x, pane)));
            if let Some(p) = found {
                self.goto(p.row, p.col);
                self.want_x = Some(x);
            }
        }
        true
    }

    /// Label every visible word (2+ chars), nearest to the cursor first, alternating forward/back.
    pub(super) fn show_labels(&mut self) {
        let mut words = vec![];
        // What's on screen, each part once: each screen row's code, a side each, and the
        // comment beside it (not under the file header pinned over the top).
        let mut shown: Vec<Shown> = vec![];
        let pinned = self.sticky().is_some();
        for d in self.top..(self.top + self.height).min(self.drows.len()) {
            let note = self.note_at(d).and_then(|c| self.span_of(c, 0));
            // (Under the pinned header, the code's covered; the pane beside it isn't.)
            let code = if pinned && d == self.top { [None, None] } else { [self.span(d, false), self.span(d, true)] };
            for s in code.into_iter().chain([note]).flatten() {
                if !shown.contains(&s) {
                    shown.push(s);
                }
            }
        }
        let mut texts: HashMap<usize, Vec<char>> = HashMap::new(); // (a row's chars, once)
        for Shown { row, chars: on, .. } in shown {
            if matches!(self.rows[row].line.kind, Kind::File | Kind::Hunk) {
                continue;
            }
            let chars = texts.entry(row).or_insert_with(|| self.chars(row));
            let class = |c: char| if c.is_whitespace() { 0 } else if c.is_alphanumeric() || c == '_' { 1 } else { 2 };
            let mut i = on.start;
            while i < on.end.min(chars.len()) {
                let (c, start) = (class(chars[i]), i);
                while i < on.end.min(chars.len()) && class(chars[i]) == c {
                    i += 1;
                }
                if c != 0 && i - start >= 2 {
                    words.push((Pos { row, col: start }, Pos { row, col: i - 1 }));
                }
            }
        }
        // (A word on both sides of a split line, cut short on one by a floating comment: once,
        // whole.)
        words.sort_by_key(|w| (w.0, std::cmp::Reverse(w.1)));
        words.dedup_by_key(|w| w.0);
        let split = words.partition_point(|w| w.0 <= self.head);
        let (back, fwd) = words.split_at(split);
        let (mut fwd, mut back) = (fwd.iter(), back.iter().rev());
        let n = self.alphabet.len();
        let mut order = vec![];
        loop {
            let (f, b) = (fwd.next(), back.next());
            if f.is_none() && b.is_none() {
                break;
            }
            order.extend(f.into_iter().chain(b).copied());
        }
        self.labels = order
            .into_iter()
            .take(n * n)
            .enumerate()
            .map(|(i, (s, e))| (s, e, [self.alphabet[i / n], self.alphabet[i % n]]))
            .collect();
        self.label_first = None;
        if self.labels.is_empty() {
            self.msg = "no words to label".into();
        }
    }

    /// Jump within the diff, remembering where we came from (Ctrl-o goes back).
    pub(super) fn jump(&mut self, row: usize, col: usize) {
        self.jumps.push(self.head);
        self.select = false;
        self.goto(row, col);
    }
}
