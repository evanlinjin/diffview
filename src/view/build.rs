//! The rows dv shows: the diff's lines, comment threads and the PR section, built from the
//! files and threads (and rebuilt keeping the cursor where it was); folding and context.

use super::*;

/// How far (in lines) an outdated thread may land from its original line number.
pub(super) const OUTDATED_SLACK: i64 = 10;

/// The Claude / pending view's windows of `f` (as `diff::windows`): each of the shown threads
/// with its lines and a few around them (± its `extra`, by its line's index), merged where they
/// meet, each with the comment lines it covers. `whole` shows the entire file.
pub(super) fn view_windows(threads: &[&Thread], f: &FileDiff, extra: impl Fn(usize) -> i64, whole: bool) -> Vec<(usize, usize, Vec<usize>)> {
    let mut spans: Vec<(usize, usize, usize)> = threads
        .iter()
        .filter(|t| t.path == f.path)
        .filter_map(|t| {
            let at = anchor_line(t, &f.full)?;
            let more = match (t.start_line, t.line) {
                (Some(s), Some(e)) if e > s => (e - s) as usize,
                _ => 0,
            };
            let x = (DEFAULT_CONTEXT as i64 + extra(at)).max(0) as usize;
            Some((at.saturating_sub(more + x), (at + x).min(f.full.len() - 1), at))
        })
        .collect();
    if whole && !f.full.is_empty() {
        return vec![(0, f.full.len() - 1, spans.into_iter().map(|s| s.2).collect())];
    }
    spans.sort();
    let mut ws: Vec<(usize, usize, Vec<usize>)> = vec![];
    for (lo, hi, at) in spans {
        match ws.last_mut() {
            Some(w) if lo <= w.1 + 1 => {
                w.1 = w.1.max(hi);
                w.2.push(at);
            }
            _ => ws.push((lo, hi, vec![at])),
        }
    }
    ws
}

/// The diff line a thread belongs under. Current threads use their line. Outdated ones are found
/// by the commented line's text when it's distinctive, else by their original line number.
pub(super) fn anchor_line(t: &Thread, lines: &[DLine]) -> Option<usize> {
    let side = |l: &DLine| match t.left {
        true => matches!(l.kind, Kind::Ctx | Kind::Del),
        false => matches!(l.kind, Kind::Ctx | Kind::Add),
    };
    let no = |l: &DLine| if t.left { l.old } else { l.new };
    if !t.outdated {
        let n = t.line?;
        return lines.iter().position(|l| side(l) && no(l) == Some(n));
    }
    let want = t.original_line? as i64;
    let dist = |i: usize| (no(&lines[i]).unwrap_or(0) as i64 - want).abs();
    let target = t.hunk.last().and_then(|l| l.get(1..)).unwrap_or("").trim();
    let distinctive = target.chars().filter(|c| c.is_alphanumeric()).count() >= 3;
    let by_text = (0..lines.len())
        .filter(|&i| distinctive && side(&lines[i]) && lines[i].text.trim() == target)
        .min_by_key(|&i| dist(i));
    by_text.or_else(|| (0..lines.len()).filter(|&i| side(&lines[i])).min_by_key(|&i| dist(i)).filter(|&i| dist(i) <= OUTDATED_SLACK))
}

/// First row of a run of added/removed lines.
pub(super) fn change_start(rows: &[Row], r: usize) -> bool {
    let is = |r: usize| matches!(rows[r].line.kind, Kind::Add | Kind::Del);
    is(r) && (r == 0 || !is(r - 1))
}

impl App {
    /// Recompute each file's shown lines from its full diff and the per-hunk context.
    pub(super) fn shape(&mut self) {
        let (extra, whole) = (&self.extra, &self.whole);
        for f in &mut self.files {
            if f.full.is_empty() {
                f.prepare(); // parsed directly (tests)
            }
            let e = |a| *extra.get(&(f.path.clone(), a)).unwrap_or(&0);
            let w = diff::windows(&f.full, DEFAULT_CONTEXT as usize, e, whole.contains(&f.path));
            f.lines = diff::shown(&f.full, &w, &f.headers);
        }
    }

    pub(super) fn cursor_key(&self) -> CursorKey {
        self.key_at(self.head.row)
    }

    fn key_at(&self, row: usize) -> CursorKey {
        let Some(r) = self.rows.get(row) else { return CursorKey { line: None, thread: None, info: None } };
        CursorKey {
            line: self.files.get(r.file).map(|f| (f.path.clone(), r.line.old, r.line.new)),
            thread: r.thread.and_then(|t| {
                let first = self.rows.iter().position(|r| r.thread == Some(t))?;
                Some((self.root_id(t)?, row - first))
            }),
            info: (r.file == INFO).then_some(row),
        }
    }

    /// The row `key` is on now.
    fn find_key(&self, key: CursorKey) -> Option<usize> {
        key.thread
            .and_then(|(id, off)| {
                let first = self.rows.iter().position(|r| r.thread.and_then(|t| self.root_id(t)) == Some(id))?;
                let t = self.rows[first].thread;
                Some((first..=first + off).take_while(|&r| self.rows.get(r).is_some_and(|r| r.thread == t)).last().unwrap_or(first))
            })
            .or_else(|| key.line.map(|(p, old, new)| self.locate(&p, old, new)))
            .or(key.info.filter(|&r| self.rows.get(r).is_some_and(|r| r.file == INFO)))
    }

    /// Rebuild rows, keeping the cursor on the same code line, thread or PR info row.
    pub(super) fn rebuild(&mut self, keep: CursorKey) {
        self.build();
        let row = self.find_key(keep).unwrap_or(0);
        self.select = false;
        self.goto(row, self.head.col);
    }

    pub(super) fn build(&mut self) {
        // Where the cursor was in the code and on the PR tab (`Ctrl-h` / `Ctrl-l` go back
        // there), kept on the same lines in the new rows.
        let (code, pr) = (self.key_at(self.code_head.row), self.pr_head.map(|r| self.key_at(r)));
        self.jumps.clear();
        let mut rows = vec![];
        let title = match (&self.pr, &self.replacement) {
            (Some(pr), _) => Some(format!("PR #{} · {}", pr.number, pr.title)),
            (None, Some(r)) => {
                let title = if r.title.is_empty() { "(not drafted yet)" } else { r.title.as_str() };
                Some(format!("Replacement of #{} · {title}", r.id))
            }
            (None, None) => (!self.claude.summary.is_empty()).then(|| "Claude's review".to_owned()),
        };
        if let Some(title) = title {
            let mk = |kind, text| Row::text(INFO, kind, text);
            // The header links to the PR (gf / Enter), instead of a URL line of its own.
            let url = self.pr.as_ref().map(|p| p.url.clone()).filter(|u| !u.is_empty());
            let links = url.map(|u| vec![(0, title.chars().count(), u)]).unwrap_or_default();
            // Who, and from which branch into which, after the title (dim; cut first when narrow).
            let by = self.pr.as_ref().filter(|p| !p.author.is_empty()).map(|p| {
                let head = if p.cross_repo { format!("{}:{}", p.head_owner, p.head_branch) } else { p.head_branch.clone() };
                format!("  @{} · {} ← {head}", p.author, p.base)
            });
            let n = title.chars().count();
            let by = by.unwrap_or_default();
            let syn = [vec![None; n], vec![Some(self.ui.linenr); by.chars().count()]].concat();
            rows.push(Row { links, syn, ..mk(Kind::File, title + &by) });
            if self.view != View::All && !self.info_collapsed {
                // The PR's state, author, branches and size; then Claude's summary, or the text
                // of your pending review.
                if let Some(l) = self.info.first().filter(|l| !l.heading && !l.md) {
                    rows.push(self.info_row(l, mk(Kind::Comment, l.text.clone())));
                }
                let (label, text, info) = match self.view {
                    View::Claude => ("claude's review", &self.claude.summary, Some(SUMMARY)),
                    _ => ("your review (pending)", &self.summary_draft, Some(DRAFT)),
                };
                if !text.is_empty() {
                    let summary = |r: Row| Row { info, ..r };
                    rows.push(summary(mk(Kind::Thread, label.into())));
                    rows.extend(self.md_rows_at(text, self.info_w(), mk).into_iter().map(summary));
                    rows.push(Row { edge: true, ..summary(mk(Kind::Comment, String::new())) });
                }
            } else if !self.info_collapsed {
                // In the pane (when it's shown), wrapped to its width.
                let width = self.info_w();
                for (ii, l) in self.info.iter().enumerate() {
                    match (l.heading, l.md) {
                        // An author line belongs to the body after it, so `C` there quotes it.
                        (true, _) => {
                            let body = self.info.get(ii + 1).filter(|n| n.md).map(|_| ii + 1);
                            rows.push(Row { info: body, ..mk(Kind::Thread, l.text.clone()) });
                        }
                        // GitHub shows a placeholder for an empty description; there's nothing to quote.
                        (false, true) if l.text.trim().is_empty() => rows.extend(self.md_rows_at("_(no description)_", width, mk)),
                        (false, true) => {
                            rows.extend(self.md_rows_at(&l.text, width, mk).into_iter().map(|r| Row { info: Some(ii), ..r }));
                            rows.push(Row { info: Some(ii), edge: true, ..mk(Kind::Comment, String::new()) });
                        }
                        (false, false) => rows.push(self.info_row(l, mk(Kind::Comment, l.text.clone()))),
                    }
                }
                if !self.claude.summary.is_empty() {
                    let summary = |r: Row| Row { info: Some(SUMMARY), ..r };
                    rows.push(summary(mk(Kind::Thread, "claude's review".into())));
                    rows.extend(self.md_rows_at(&self.claude.summary, width, mk).into_iter().map(summary));
                    rows.push(Row { edge: true, ..summary(mk(Kind::Comment, String::new())) });
                }
            }
        }
        // Threads on files that have left the diff (a change reverted, a file now matching the
        // base) would show nowhere: list them here, by file. Not when showing only what isn't
        // committed, where most files are out of the diff on purpose.
        let mut gone: Vec<usize> = match self.uncommitted() {
            true => vec![],
            false => (0..self.threads.len())
                .filter(|&t| self.in_view(&self.threads[t]) && !self.files.iter().any(|f| f.path == self.threads[t].path))
                .collect(),
        };
        gone.sort_by(|&a, &b| self.threads[a].path.cmp(&self.threads[b].path)); // stable: in order within a file
        if !gone.is_empty() {
            let heading = |text: String| Row::text(INFO, Kind::Thread, text);
            rows.push(heading(format!("comments on files no longer in the diff ({})", gone.len())));
            let mut last = "";
            for &t in &gone {
                if self.threads[t].path != last {
                    last = &self.threads[t].path;
                    rows.push(heading(last.to_owned()));
                }
                rows.extend(self.thread_rows(t, INFO));
            }
        }
        let shown: Vec<&Thread> = self.threads.iter().filter(|t| self.in_view(t)).collect();
        for (i, f) in self.files.iter().enumerate() {
            // The Claude / pending view: only files with its comments, and only around them.
            let around;
            let lines = match self.view != View::All {
                true if !shown.iter().any(|t| t.path == f.path) => continue,
                true => {
                    let e = |a| *self.view_extra.get(&(f.path.clone(), a)).unwrap_or(&0);
                    around = diff::shown(&f.full, &view_windows(&shown, f, e, self.whole.contains(&f.path)), &f.headers);
                    &around
                }
                false => &f.lines,
            };
            let count = |k| f.lines.iter().filter(|l| l.kind == k).count();
            // "dir/" dimmed, the file name in the header color, "(new)" etc. dimmed, +/- colored.
            let (dir, name) = f.path.rsplit_once('/').map_or(("", f.path.as_str()), |(d, n)| (d, n));
            let dir = if dir.is_empty() { String::new() } else { format!("{dir}/") };
            let meta = if f.meta.is_empty() { String::new() } else { format!(" ({})", f.meta) };
            let dim = Some(self.ui.stale);
            let parts = [
                (dir, dim),
                (name.to_owned(), None),
                (meta, dim),
                ("  ".into(), None),
                (format!("+{}", count(Kind::Add)), Some(self.ui.plus)),
                (" ".into(), None),
                (format!("−{}", count(Kind::Del)), Some(self.ui.minus)),
            ];
            let text: String = parts.iter().map(|p| p.0.as_str()).collect();
            let syn = parts.iter().flat_map(|(t, st)| t.chars().map(move |_| *st)).collect();
            rows.push(Row { syn, ..Row::text(i, Kind::File, text) });
            if self.collapsed.contains(&f.path) {
                continue;
            }
            // (thread, index into f.lines it sits under); unplaceable ones go under the file header.
            let placed: Vec<(usize, Option<usize>)> = (0..self.threads.len())
                .filter(|&t| self.threads[t].path == f.path && self.in_view(&self.threads[t]))
                .map(|t| (t, anchor_line(&self.threads[t], lines)))
                .collect();
            for &(t, _) in placed.iter().filter(|p| p.1.is_none()) {
                rows.extend(self.thread_rows(t, i));
            }
            for (li, l) in lines.iter().enumerate() {
                rows.push(Row::new(i, l.clone()));
                for &(t, _) in placed.iter().filter(|p| p.1 == Some(li)) {
                    rows.extend(self.thread_rows(t, i));
                }
            }
        }
        if rows.is_empty() {
            rows.push(Row::text(usize::MAX, Kind::Hunk, "(no changes)".into()));
        }
        self.rows = rows;
        self.code_head.row = self.find_key(code).unwrap_or(0);
        self.pr_head = pr.and_then(|k| self.find_key(k));
        self.emph = self.word_spans();
        self.layout();
    }

    /// Pair each run of removed rows with the added rows after it (as split view does) and
    /// find the changed words in each pair.
    pub(super) fn word_spans(&self) -> HashMap<usize, Vec<(usize, usize)>> {
        let (n, kind) = (self.rows.len(), |i: usize| self.rows[i].line.kind);
        let mut out = HashMap::new();
        let mut i = 0;
        while i < n {
            if kind(i) != Kind::Del {
                i += 1;
                continue;
            }
            let dels = (i..n).take_while(|&j| kind(j) == Kind::Del).count();
            let adds = (i + dels..n).take_while(|&j| kind(j) == Kind::Add).count();
            for k in 0..dels.min(adds) {
                let (d, a) = (i + k, i + dels + k);
                if let Some((x, y)) = diff::word_diff(&self.rows[d].line.text, &self.rows[a].line.text) {
                    out.insert(d, x);
                    out.insert(a, y);
                }
            }
            i += dels + adds;
        }
        out
    }

    /// Wrap markdown into comment rows, highlighted.
    pub(super) fn md_rows_at(&self, text: &str, width: usize, mk: impl Fn(Kind, String) -> Row) -> Vec<Row> {
        let key = format!("{width}\0{text}");
        let lines = self
            .md_cache
            .borrow_mut()
            .entry(key)
            .or_insert_with(|| crate::md::render(text, width, &self.ui.md, &self.hl, self.repo_url()))
            .clone();
        lines.into_iter().map(|l| Row { syn: l.styles, links: l.links, nowrap: l.nowrap, ..mk(Kind::Comment, l.text) }).collect()
    }

    /// The width comments and PR text wrap to: the pane's, when it's shown (they're there).
    pub(super) fn info_w(&self) -> usize {
        if self.margin_active { self.margin_w - 3 - 2 - self.pane_pad() } else { self.wrap_w }
    }

    /// The space inside the line between the code and the pane, as a column.
    pub(super) fn pane_pad(&self) -> usize {
        usize::from(!self.margin_float)
    }

    /// Color a PR info line by its marks: badges, pass/fail colors, links.
    pub(super) fn info_row(&self, l: &gh::InfoLine, mut row: Row) -> Row {
        let ui = &self.ui;
        row.syn = vec![None; l.text.chars().count()];
        for (a, b, m) in &l.marks {
            let st = match m {
                gh::Mark::Chip(hex) => chip(hex),
                gh::Mark::Pass => ui.plus,
                gh::Mark::Fail => ui.error,
                gh::Mark::Pending => ui.warning,
                gh::Mark::Dim => ui.stale,
                gh::Mark::Link(url) => {
                    row.links.push((*a, *b, url.clone()));
                    ui.md.link
                }
            };
            for s in row.syn.iter_mut().take(*b).skip(*a) {
                *s = Some(st);
            }
        }
        row
    }

    /// Drawn at the right end of a header line: the PR's state on the PR's, the file's GitHub
    /// "Viewed" state on a file's.
    pub(super) fn header_tag(&self, file: usize) -> Option<(String, Style)> {
        if file == INFO {
            return self.pr_state.map(|(s, color)| (format!(" {s} "), chip(color)));
        }
        let (t, st) = match self.viewed.get(&self.files.get(file)?.path)?.as_str() {
            "VIEWED" => ("✓ Viewed ", self.ui.plus),
            _ => ("↻ changed since viewed ", self.ui.warning),
        };
        Some((t.into(), st))
    }

    /// A comment's first `n` lines as they read rendered (quotes skipped), for picker cards.
    pub(super) fn snippet(&self, body: &str, n: usize) -> Vec<String> {
        // Wrapped to the picker's text width (3/4 of the window, less padding and indent).
        let w = (self.width as usize * 3 / 4).saturating_sub(6).max(20);
        let lines = crate::md::render(body, w, &self.ui.md, &self.hl, None);
        let text = lines.into_iter().map(|l| l.text.trim_end().to_owned());
        text.skip_while(|l| l.is_empty() || l.starts_with('▎')).filter(|l| !l.is_empty()).take(n).collect()
    }

    pub(super) fn root_id(&self, t: usize) -> Option<u64> {
        Some(self.threads.get(t)?.comments.first()?.id)
    }

    pub(super) fn thread_rows(&self, ti: usize, file: usize) -> Vec<Row> {
        let t = &self.threads[ti];
        let open = self.thread_open(ti);
        let range = match (t.start_line, t.line.or(t.original_line)) {
            (Some(s), Some(e)) if s != e => format!(" · L{s}-{e}"),
            (_, Some(e)) if t.outdated => format!(" · was L{e}"),
            (_, Some(e)) => format!(" · L{e}"),
            _ => String::new(),
        };
        // Tags lead the header so stale threads are easy to spot when scanning.
        let first_pending = t.comments.first().is_some_and(|c| c.pending);
        let severity = format!("[{}] ", t.severity);
        let serious = matches!(t.severity.as_str(), "bug" | "security" | "major" | "blocker");
        let turn = agent::turn(t);
        let tags: Vec<(&str, Style)> = [
            (self.is_new(t), "[new] ", self.ui.key),
            (first_pending, "[pending] ", self.ui.pending),
            (t.outdated, "[outdated] ", self.ui.warning),
            (t.resolved, "[resolved] ", self.ui.resolved),
            (!t.severity.is_empty(), severity.as_str(), if serious { self.ui.warning } else { self.ui.resolved }),
            (turn.is_some(), turn.unwrap_or(""), self.ui.pending),
        ]
            .into_iter()
            .filter_map(|(on, s, c)| on.then_some((s, c)))
            .collect();
        let tag_text: String = tags.iter().map(|t| t.0).collect();
        let tag_syn: Vec<Option<Style>> =
            tags.iter().flat_map(|&(s, st)| s.chars().map(move |_| Some(st))).collect();
        let mk = |kind, text| Row { thread: Some(ti), ..Row::text(file, kind, text) };
        // Claude's comments (from its review file) carry its name in the accent color.
        let by_claude = |c: &gh::Comment| t.claude.is_some() && c.author == crate::claude::AUTHOR;
        let author_syn = |c: &gh::Comment| vec![by_claude(c).then_some(self.ui.key); c.author.chars().count()];
        let when = |c: &gh::Comment| match gh::ago(&c.created) {
            a if a.is_empty() => String::new(),
            a => format!(" · {a}"),
        };
        let mut out = vec![];
        for (k, c) in t.comments.iter().enumerate() {
            if k == 0 {
                // In the margin the tags get a line of their own, under the header, so the header
                // keeps room for who, when and where.
                let tags_below = self.margin_active && !tag_text.is_empty();
                let lead = if tags_below { String::new() } else { tag_text.clone() };
                let mut head = format!("{lead}{}{}{range}", c.author, when(c));
                if !open {
                    head += &match t.comments.len() {
                        1 => " · 1 comment".to_owned(),
                        n => format!(" · {n} comments"),
                    };
                }
                let syn = match tags_below {
                    true => author_syn(c),
                    false => [tag_syn.clone(), author_syn(c)].concat(),
                };
                out.push(Row { syn, ..mk(Kind::Thread, head) });
                if tags_below {
                    out.push(Row { syn: tag_syn.clone(), ..mk(Kind::Comment, tag_text.trim_end().to_owned()) });
                }
                if !open {
                    break;
                }
                // Outdated: the code may have moved or changed, so show what was commented on.
                if t.outdated {
                    let dim = Some(self.ui.stale);
                    out.extend(t.hunk.iter().map(|h| Row { syn: vec![dim; h.chars().count()], ..mk(Kind::Comment, h.clone()) }));
                }
            } else {
                let tag = if c.pending { "[pending] " } else { "" };
                let syn = [vec![Some(self.ui.pending); tag.chars().count()], author_syn(c)].concat();
                out.push(Row { syn, ..mk(Kind::Thread, format!("{tag}{}{}", c.author, when(c))) });
            }
            // In the margin, comments wrap to its width (less the gutter mark and box sides).
            let w = self.info_w();
            let rows = self.md_rows_at(&c.body, w, mk);
            match t.stale() {
                // Resolved/outdated: rendered the same, but dimmed throughout (links still open).
                true => out.extend(rows.into_iter().map(|mut r| {
                    r.syn = vec![Some(self.ui.stale); r.line.text.chars().count()];
                    r
                })),
                false => out.extend(rows),
            }
        }
        if open {
            out.push(Row { edge: true, ..mk(Kind::Comment, String::new()) });
        }
        out
    }

    /// Which comment box row `r` is part of, and its place in it; `None` outside one. Boxes are
    /// open review threads and the PR description / conversation comments.
    pub(super) fn box_part(&self, r: usize) -> Option<BoxPart> {
        let key = |r: &Row| match (r.thread, r.info) {
            (Some(t), _) => Some((Some(t), None)),
            (None, Some(i)) if r.file == INFO => Some((None, Some(i))),
            _ => None,
        };
        let row = &self.rows[r];
        let k = key(row)?;
        if row.thread.is_some_and(|t| !self.thread_open(t)) {
            return None; // a folded thread stays a single line
        }
        let first = r == 0 || key(&self.rows[r - 1]) != Some(k);
        Some(match (row.edge, row.line.kind, first) {
            (true, ..) => BoxPart::Bottom,
            (_, _, true) => BoxPart::Top,
            (_, Kind::Thread, false) => BoxPart::Divider, // the next comment's author line
            _ => BoxPart::Body,
        })
    }

    /// Stale threads start folded; `Tab` flips a thread from its default.
    pub(super) fn thread_open(&self, ti: usize) -> bool {
        self.threads[ti].stale() == self.root_id(ti).is_some_and(|id| self.toggled.contains(&id))
    }

    pub(super) fn toggle_thread(&mut self) {
        let Some(ti) = self.rows[self.head.row].thread else { return };
        let Some(id) = self.root_id(ti) else { return };
        flip(&mut self.toggled, id);
        self.rebuild_at(|r| r.thread == Some(ti));
    }

    /// Rebuild rows and put the cursor on the first row matching `at` (else the top).
    pub(super) fn rebuild_at(&mut self, at: impl Fn(&Row) -> bool) {
        self.build();
        self.select = false;
        let r = self.rows.iter().position(at).unwrap_or(0);
        self.goto(r, 0);
    }

    /// Row in file `path` at (or just after) the given line, else the file header.
    pub(super) fn locate(&self, path: &str, old: Option<u32>, new: Option<u32>) -> usize {
        let Some(fi) = self.files.iter().position(|f| f.path == path) else { return 0 };
        let in_file = |r: &&Row| r.file == fi;
        let hit = match (new, old) {
            (Some(n), _) => self.rows.iter().position(|r| in_file(&r) && r.line.new.is_some_and(|x| x >= n)),
            (None, Some(o)) => self.rows.iter().position(|r| in_file(&r) && r.line.old.is_some_and(|x| x >= o)),
            _ => None,
        };
        hit.or_else(|| self.rows.iter().position(|r| r.file == fi)).unwrap_or(0)
    }

    /// `+`/`-`: more or less context around the hunk under the cursor (on a header: the next one).
    pub(super) fn expand(&mut self, d: i64) {
        let row = &self.rows[self.head.row];
        let Some(f) = self.files.get(row.file) else { return };
        let path = f.path.clone();
        if d < 0 && self.whole.remove(&path) {
            return self.reshape();
        }
        let code = self.rows[self.head.row..]
            .iter()
            .take_while(|r| r.file == row.file)
            .find(|r| matches!(r.line.kind, Kind::Ctx | Kind::Add | Kind::Del));
        let Some(idx) = code.and_then(|c| f.full.iter().position(|x| x.old == c.line.old && x.new == c.line.new)) else {
            return;
        };
        // The Claude / pending view keeps its own, per comment.
        let view = self.view != View::All;
        let extra = if view { &self.view_extra } else { &self.extra };
        let e = |a| *extra.get(&(path.clone(), a)).unwrap_or(&0);
        let ws = match view {
            true => view_windows(&self.threads.iter().filter(|t| self.in_view(t)).collect::<Vec<_>>(), f, e, false),
            false => diff::windows(&f.full, DEFAULT_CONTEXT as usize, e, self.whole.contains(&path)),
        };
        let Some((_, _, hunks)) = ws.into_iter().find(|w| w.0 <= idx && idx <= w.1) else { return };
        let extra = if view { &mut self.view_extra } else { &mut self.extra };
        for a in hunks {
            let x = extra.entry((path.clone(), a)).or_insert(0);
            *x = (*x + d).max(-(DEFAULT_CONTEXT as i64));
        }
        self.reshape();
    }

    pub(super) fn toggle_whole(&mut self) {
        let Some(p) = self.file_path().map(str::to_owned) else { return };
        flip(&mut self.whole, p);
        self.reshape();
    }

    pub(super) fn reshape(&mut self) {
        let keep = self.cursor_key();
        self.shape();
        self.rebuild(keep);
    }

    pub(super) fn toggle_collapse(&mut self) {
        let fi = self.rows[self.head.row].file;
        if fi == INFO {
            self.info_collapsed = !self.info_collapsed;
            return self.rebuild_at(|_| true);
        }
        let Some(p) = self.files.get(fi).map(|f| f.path.clone()) else { return };
        flip(&mut self.collapsed, p);
        self.rebuild_at(|r| r.file == fi);
    }

    /// Fold every file if any is unfolded, else unfold them all. The cursor goes to its file's header.
    pub(super) fn toggle_collapse_all(&mut self) {
        let fi = self.rows[self.head.row].file;
        if self.files.iter().any(|f| !self.collapsed.contains(&f.path)) {
            self.collapsed = self.files.iter().map(|f| f.path.clone()).collect();
        } else {
            self.collapsed.clear();
        }
        self.rebuild_at(|r| r.file == fi);
    }
}
