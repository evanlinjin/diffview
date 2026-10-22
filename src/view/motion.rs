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
        // The code's left edge: on into the files' list.
        if !right && self.files_w > 0 && self.list_has_spots() && !self.margin_side() && (!self.split || !self.right) {
            (self.files_focus, self.files_sel) = (true, self.current_spot());
            return self.msg = "contents: j / k move, Enter goes there, Ctrl-l back".into();
        }
        let hd = self.hd();
        let into_margin = self.margin_active && right && (!self.split || self.right) && !self.margin_side();
        // By what a comment is about, not where its box happens to sit (boxes stack down the
        // pane): from a comment to its line, and from a line to the comment about it.
        if self.margin_side() && !right
            && let Some(&line) = self.rows[self.head.row].thread.and_then(|t| self.margin_anchor.get(&t))
        {
            return self.goto(line, 0);
        }
        if into_margin
            && let Some(t) = self.lit_thread()
            && let Some(r) = self.rows.iter().position(|r| r.thread == Some(t))
        {
            return self.goto(r, 0);
        }
        if into_margin || self.margin_side() && !right {
            let at = |d: usize| match self.drows[d] {
                DRow::Margin(_, _, c) if right => c,
                DRow::Margin(l, r, _) => if self.right { r.or(l) } else { l.or(r) },
                _ => None,
            };
            let near = at(hd)
                .or_else(|| (hd..self.drows.len()).find_map(|d| right.then(|| at(d)).flatten()))
                .or_else(|| (0..hd).rev().find_map(at));
            return match near {
                Some(r) => self.goto(r, 0),
                None => self.msg = if right { "no comments here" } else { "no code here" }.into(),
            };
        }
        if self.margin_side() {
            return; // already as far right as it goes
        }
        // To the other side's line on this screen row (its part, wrapped), as far into it.
        let x = self.cursor_x();
        self.right = right;
        if let Some(r) = self.side_row(hd, right)
            && let Some(s) = self.span_of(r, self.dseg[hd])
        {
            let col = self.char_at(r, self.col_of(r, s.chars.start) + x).min(s.chars.end.saturating_sub(1)).max(s.chars.start);
            self.goto(r, col);
        }
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
        match self.rows[row].line.kind {
            Kind::Del => self.right = false,
            Kind::Add => self.right = true,
            _ => {}
        }
        self.head = Pos { row, col: col.min(self.len(row).saturating_sub(1)) };
        if !self.select {
            self.anchor = self.head;
        }
        self.want_x = None;
    }

    /// Set a selection (motions like `w` select); in select mode only the head moves.
    pub(super) fn sel(&mut self, anchor: Pos, head: Pos) {
        self.head = head;
        if !self.select {
            self.anchor = anchor;
        }
        self.want_x = None;
    }

    pub(super) fn vmove(&mut self, d: isize) {
        // By screen rows (a wrapped line's parts too, as Helix moves), over those the cursor
        // can't stop on (`stop_at`), keeping its column.
        // (Overshooting into rows it can't stop on, as far as it can.)
        let x = self.cursor_x();
        let last = self.drows.len() as isize - 1;
        let hd = self.hd();
        let t = (hd as isize + d).clamp(0, last) as usize;
        let Some(p) = self.stop_near(t, if d < 0 { -1 } else { 1 }, hd, x) else { return };
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
            self.head = Pos { row: r, col: self.len(r).saturating_sub(1) };
        } else {
            self.anchor = Pos { row: s.row, col: 0 };
            self.head = Pos { row: e.row, col: end };
        }
        self.want_x = None;
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
                self.head = Pos { row: r, col: c + qlen - 1 };
                self.want_x = None;
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
            // Onto the screen: the nearest row it can stop on, toward the screen's middle; with
            // no code on screen (the pane's boxes running on past it), a comment there.
            let x = self.cursor_x();
            let step: isize = if hd < lo { 1 } else { -1 };
            let on_screen = |p: &Pos| (lo..=hi.max(lo)).contains(&self.d_at(*p));
            let comment = || (lo..=hi.max(lo)).find_map(|d| match self.drows.get(d) {
                Some(&DRow::Margin(_, _, Some(c))) => Some(Pos { row: c, col: 0 }),
                _ => None,
            });
            if let Some(p) = self.stop_near(into(hd), step, hd, x).filter(on_screen).or_else(comment) {
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
        // comment beside it.
        let mut shown: Vec<Shown> = vec![];
        for d in self.top..(self.top + self.height).min(self.drows.len()) {
            let note = match self.drows[d] {
                DRow::Margin(_, _, Some(c)) => self.span_of(c, 0),
                _ => None,
            };
            for s in [self.span(d, false), self.span(d, true), note].into_iter().flatten() {
                if !shown.contains(&s) {
                    shown.push(s);
                }
            }
        }
        for Shown { row, chars: on, .. } in shown {
            if matches!(self.rows[row].line.kind, Kind::File | Kind::Hunk) {
                continue;
            }
            let chars = self.chars(row);
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
        words.sort_by_key(|w| w.0);
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
