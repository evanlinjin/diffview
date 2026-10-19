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
        let Some(r) = self.rows.get(row) else { return CursorKey { line: None, thread: None, info: None, section: None } };
        CursorKey {
            line: self.files.get(r.file).map(|f| (f.path.clone(), r.line.old, r.line.new)),
            thread: r.thread.and_then(|t| {
                let first = self.rows.iter().position(|r| r.thread == Some(t))?;
                Some((self.root_id(t)?, row - first))
            }),
            info: (r.file == INFO && r.main.is_none()).then(|| {
                let pr = |x: &&Row| x.file == INFO && x.main.is_none();
                let before = || self.rows[..row].iter().filter(pr);
                let id = self.info_id(r.info);
                (id.clone(), before().filter(|x| id.is_some() && self.info_id(x.info) == id).count(), before().count())
            }),
            section: r.main.and_then(|m| Some((m, row - self.rows.iter().position(|x| x.main == Some(m))?))),
        }
    }

    /// What `info` line `i` is, whatever its place among them (a refresh adds lines before
    /// it): its `key`; one of `Row::info`'s markers (`FACTS`, …) as itself.
    fn info_id(&self, i: Option<usize>) -> Option<String> {
        let i = i?;
        // (A line with no key, not from GitHub's answer: its text.)
        Some(self.info.get(i).map_or_else(|| format!("#{i}"), |l| if l.key.is_empty() { format!("~{}", l.text) } else { l.key.clone() }))
    }

    /// The row `key` is on now.
    fn find_key(&self, key: CursorKey) -> Option<usize> {
        key.thread
            .and_then(|(id, off)| {
                let first = self.rows.iter().position(|r| r.thread.and_then(|t| self.root_id(t)) == Some(id))?;
                let t = self.rows[first].thread;
                Some((first..=first + off).take_while(|&r| self.rows.get(r).is_some_and(|r| r.thread == t)).last().unwrap_or(first))
            })
            .or_else(|| {
                let (m, off) = key.section?;
                let first = self.rows.iter().position(|r| r.main == Some(m))?;
                Some((first..=first + off).take_while(|&r| self.rows.get(r).is_some_and(|r| r.main == Some(m))).last().unwrap_or(first))
            })
            .or_else(|| key.line.map(|(p, old, new)| self.locate(&p, old, new)))
            .or_else(|| {
                let (info, off, k) = key.info?;
                let pr: Vec<usize> = (0..self.rows.len()).filter(|&r| self.rows[r].file == INFO && self.rows[r].main.is_none()).collect();
                // Its info's rows (re-wrapped: as far in as there is), else where it was among them.
                let same: Vec<usize> = pr.iter().copied().filter(|&r| info.is_some() && self.info_id(self.rows[r].info) == info).collect();
                same.get(off).or(same.last()).or(pr.get(k)).copied()
            })
    }

    /// Rebuild rows, keeping the cursor on the same code line, thread or PR info row.
    pub(super) fn rebuild(&mut self, keep: CursorKey) {
        self.build();
        let row = self.find_key(keep).unwrap_or(0);
        self.select = false;
        self.goto(row, self.head.col);
    }

    pub(super) fn build(&mut self) {
        // Where the cursor was in the code, kept on the same line in the new rows.
        let code = self.key_at(self.code_head.row);
        self.jumps.clear();
        let mut rows = vec![];
        let title = match (&self.pr, &self.replacement) {
            (Some(pr), _) => Some(format!("PR #{} · {}", pr.number, pr.title)),
            (None, Some(r)) => {
                let title = if r.title.is_empty() { "(not drafted yet)" } else { r.title.as_str() };
                Some(match crate::replace::is_new(r.id) {
                    true => format!("{} · {title}", crate::replace::label(r.id).replacen("new PR", "New PR", 1)),
                    false => format!("Replacement of #{} · {title}", r.id),
                })
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
            if self.view == View::Pending && !self.info_collapsed {
                // The PR's state, author, branches and size; then the text of your pending review
                // (in the pane, as the PR's text is).
                if let Some(l) = self.info.first().filter(|l| !l.heading && !l.md) {
                    rows.push(Row { attached: true, ..self.info_row(l, mk(Kind::Comment, l.text.clone())) });
                }
                rows.extend(self.draft_rows().into_iter().map(|r| Row { attached: true, ..r }));
            }
        }
        // Attached to the description: beside it in the pane, as comments are beside their
        // lines (inline after it, the pane hidden). The whole diff's: the PR's facts, its
        // conversation's latest (`Space o`: all of it), your pending review's text. The Claude
        // view's: Claude's summary (or your questions about the PR), and what's said under it.
        // (Folded with it: Tab on it, or on them.)
        // Some of the commits (their messages, then their code), or only what isn't committed:
        // not the PR's text (it's about all of them).
        let commits = self.commit_shown.is_some() || self.uncommitted();
        let mut attached: Vec<Row> = match self.view {
            _ if commits || self.sections_folded.contains(&DESCRIPTION) => vec![],
            // (A draft's: what you asked of it, beside it too.)
            _ if self.claude_side() => {
                let desc = (0..self.info.len()).find(|&i| self.is_description(i)).map_or(0, |h| h + 1);
                [self.claude_attached(), self.request_rows(&["description", ""], desc)].into_iter().flatten().collect()
            }
            View::All if self.info_collapsed => vec![],
            View::All => {
                let latest = (!self.conversation_all).then_some(LATEST);
                [self.pr_facts(), self.conversation(latest), self.no_pr_summary(), self.draft_rows()].into_iter().flatten().collect()
            }
            _ => vec![],
        };
        attached.iter_mut().for_each(|r| r.attached = true);
        // Claude's draft comment for closing the PR: first, as what you'd do next.
        if self.view == View::All && !commits && !self.claude.closing.trim().is_empty() {
            let what = match self.pr_state.is_some_and(|s| matches!(s.0, "OPEN" | "DRAFT")) {
                true => "closing comment · Claude's draft (Space a: close the PR with it, more changes, discard)",
                false => "closing comment · Claude's draft (the PR isn't open: Space a discards it)",
            };
            rows.extend(self.section(CLOSING_PR, what.into(), &self.claude.closing, None));
        }
        // Before the files, like one: the PR's description (in the Claude view too, Claude's
        // review attached to it), then the messages of the commits shown.
        if matches!(self.view, View::All | View::Claude)
            && !commits
            && let Some(h) = (0..self.info.len()).find(|&i| self.is_description(i))
        {
            let text = self.info.get(h + 1).filter(|l| l.md).map_or("", |l| l.text.as_str());
            match self.claude.description.is_empty() || self.pr.is_none() {
                true => {
                    let text = match (text.trim().is_empty(), self.replacement.is_some()) {
                        (true, true) => "_(not drafted yet)_",
                        (true, false) => "_(no description)_",
                        _ => text,
                    };
                    let mut s = self.section(DESCRIPTION, self.info[h].text.clone(), text, Some(h + 1));
                    // An unpublished draft: what it replaces (or what you asked for) first, as
                    // it has no facts box.
                    if let Some(l) = self.info.first().filter(|_| self.unpublished() && s.len() > 1) {
                        // (Its info line's, not the draft's text: `a` there quotes none of it.)
                        let mk = |text| Row { main: Some(DESCRIPTION), info: Some(0), ..Row::text(INFO, Kind::Comment, text) };
                        s.splice(1..1, [self.info_row(l, mk(l.text.clone())), mk(String::new())]);
                    }
                    rows.extend(self.attach(s, &mut attached));
                }
                // Claude's proposal: what it changes, line by line (as markdown source).
                false => {
                    let mk = |kind, text| Row { main: Some(DESCRIPTION), info: Some(h + 1), ..Row::text(INFO, kind, text) };
                    let mut s = vec![mk(Kind::File, "description · Claude's proposal (Space a: approve, more changes, discard)".into())];
                    if !self.sections_folded.contains(&DESCRIPTION) {
                        // By source line (a reworded paragraph is one changed line), each then
                        // wrapped to the code's width.
                        let w = self.wrap_w.saturating_sub(2).max(20);
                        let lines = |t: &str| t.replace("\r\n", "\n").lines().map(str::to_owned).collect::<Vec<_>>();
                        for (k, line) in line_diff(&lines(text), &lines(&self.claude.description)) {
                            s.extend(wrap(&line, w).into_iter().map(|t| mk(k, t)));
                        }
                    }
                    rows.extend(self.attach(s, &mut attached));
                }
            }
        }
        // No description to attach to (a branch): after the title.
        rows.append(&mut attached);
        if self.view == View::All {
            // A replacement's closing note (for the PRs it replaces), after its description.
            // Published, on its PR: still what closes what it replaces (S), while any is open.
            let open = |r: &&crate::replace::Record| r.replaces.iter().any(|o| !r.closed.contains(&o.number));
            if let Some(r) = self.replacement.as_ref().filter(|_| self.pr.is_some() && !commits).filter(open) {
                rows.extend(self.section(CLOSING_NOTE, "closing note, for the PRs it replaces (S closes them with it)".into(), &r.closing_note, None));
            }
            if let Some(h) = (0..self.info.len()).find(|&i| self.is_closing_note(i)).filter(|_| !commits) {
                let text = self.info.get(h + 1).filter(|l| l.md).map_or("", |l| l.text.as_str());
                let text = if text.trim().is_empty() { "_(not drafted yet)_" } else { text };
                let s = self.section(CLOSING_NOTE, self.info[h].text.clone(), text, Some(h + 1));
                let mut asked: Vec<Row> = match self.sections_folded.contains(&CLOSING_NOTE) {
                    true => vec![],
                    false => self.request_rows(&["closing note"], h + 1).into_iter().map(|r| Row { attached: true, ..r }).collect(),
                };
                rows.extend(self.attach(s, &mut asked));
            }
            if let Some((a, _)) = self.commit_shown {
                for (k, msg) in self.commit_msgs.iter().enumerate() {
                    let Some(c) = self.commits.get(a + k) else { break };
                    rows.extend(self.section(1 + a + k, format!("commit {}", c.short), msg, None));
                }
            }
        }
        // Threads on files that have left the diff (a change reverted, a file now matching the
        // base) would show nowhere: list them here, by file. Not when showing only what isn't
        // committed, where most files are out of the diff on purpose.
        // (Questions about a commit are on no lines: listed whatever's shown.)
        // Questions about the conversation are under their comment, in the whole diff (else
        // here), and those about the summary under it.
        let unplaced = |th: &Thread| th.conversation != ON_SUMMARY && (self.view != View::All || self.conversation_item(&th.conversation).is_none());
        let mut gone: Vec<usize> = (0..self.threads.len())
            .filter(|&t| {
                let th = &self.threads[t];
                self.in_view(th) && !self.has_file(&th.path) && (self.on_code(th) || self.gone_listed() && unplaced(th))
            })
            .collect();
        // By file (stable: in order within one); questions about the conversation, on no file, last.
        // (Questions on no lines: about a commit, under it; else about the conversation.)
        let label = |t: usize| {
            let th = &self.threads[t];
            match (th.path.is_empty(), commit_of(&th.conversation)) {
                (false, _) => th.path.clone(),
                (true, Some(sha)) => format!("about commit {}", self.commit_short(sha)),
                (true, None) => "questions about the PR's conversation".into(),
            }
        };
        gone.sort_by_cached_key(|&t| (self.threads[t].path.is_empty(), label(t)));
        let gone_at = rows.len();
        if !gone.is_empty() {
            // (In the pane, as the PR's text is; inline with it hidden.)
            let heading = |text: String| Row { attached: true, ..Row::text(INFO, Kind::Thread, text) };
            let on_files = gone.iter().filter(|&&t| !self.threads[t].path.is_empty()).count();
            if on_files > 0 {
                rows.push(heading(format!("comments on files no longer in the diff ({on_files})")));
            }
            let mut last = None;
            for &t in &gone {
                let l = label(t);
                if last.as_ref() != Some(&l) {
                    rows.push(heading(l.clone()));
                    last = Some(l);
                }
                rows.extend(self.thread_rows(t, INFO).into_iter().map(|r| Row { attached: true, ..r }));
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
            // Its open comments, said even when it's folded (a viewed file is), so none go unseen.
            let open = self.threads.iter().filter(|t| t.path == f.path && !t.stale() && self.in_view(t)).count();
            let comments = match open {
                0 => String::new(),
                1 => "  · 1 comment".into(),
                n => format!("  · {n} comments"),
            };
            let parts = [
                (dir, dim),
                (name.to_owned(), None),
                (meta, dim),
                ("  ".into(), None),
                (format!("+{}", count(Kind::Add)), Some(self.ui.plus)),
                (" ".into(), None),
                (format!("−{}", count(Kind::Del)), Some(self.ui.minus)),
                (comments, Some(self.ui.thread)),
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
        self.gone_at = gone_at;
        self.code_head.row = self.find_key(code).unwrap_or(0);
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

    /// Whether thread `t` is about the code shown: on one of its files, or on one gone from the
    /// diff (listed after the PR's text: `gone_listed`), or a question about a commit. The one
    /// rule for where a thread goes, and what the pane's tabs count.
    pub(super) fn on_code(&self, t: &Thread) -> bool {
        commit_of(&t.conversation).is_some() || !t.path.is_empty() && (self.gone_listed() || self.has_file(&t.path))
    }

    /// Whether threads on files gone from the diff are listed (not when showing only what isn't
    /// committed, or a commit: most files are out of the diff on purpose then).
    fn gone_listed(&self) -> bool {
        !(self.uncommitted() || self.commit_shown.is_some())
    }

    fn has_file(&self, path: &str) -> bool {
        self.files.iter().any(|f| f.path == path)
    }

    /// Which section row `r` is in (`Row::main`), if it's one.
    pub(super) fn section_of(&self, r: usize) -> Option<usize> {
        self.rows.get(r)?.main
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
            return self.pr_state.map(|(s, color)| (format!(" {s} "), chip(color)));
        }
        let (t, st) = match self.viewed.get(&self.files.get(file)?.path)?.as_str() {
            "VIEWED" => ("✓ Viewed ", self.ui.plus),
            _ => ("↻ changed since viewed ", self.ui.warning),
        };
        Some((t.into(), st))
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
            .find(|r| r.line.kind.is_code());
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
        // On what's attached to the description: it folds with it (if there's one); else
        // (a branch's, threads on files gone from the diff), nothing to fold.
        // (Your pending review's text, in the pending view: it folds, as the PR's text.)
        let attached = self.rows[self.head.row].attached;
        if attached && self.head.row >= self.gone_at {
            return self.msg = "nothing to fold here".into();
        }
        if attached && self.view != View::Pending {
            // The section it's beside: its text's (what you asked of a draft), else the description.
            let info = self.rows[self.head.row].info;
            let id = self.rows.iter().find(|r| r.main.is_some() && info.is_some() && r.info == info).and_then(|r| r.main).unwrap_or(DESCRIPTION);
            if !self.rows.iter().any(|r| r.main == Some(id) && r.line.kind == Kind::File) {
                return self.msg = "nothing to fold here".into();
            }
            flip(&mut self.sections_folded, id);
            return self.rebuild_at(move |r| r.main == Some(id) && r.line.kind == Kind::File);
        }
        if let Some(id) = self.section_of(self.head.row) {
            if !self.sections_folded.remove(&id) {
                self.sections_folded.insert(id);
            }
            return self.rebuild_at(move |r| r.main == Some(id) && r.line.kind == Kind::File);
        }
        if fi == INFO && self.claude_side() {
            return self.msg = "nothing to fold here".into();
        }
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

/// A line diff of `old` → `new`: (Ctx | Del | Add, line), from their longest common subsequence.
// ponytail: O(n·m) table, capped as `word_diff` is (beyond it: all old out, all new in).
fn line_diff(old: &[String], new: &[String]) -> Vec<(Kind, String)> {
    let (n, m) = (old.len(), new.len());
    if n * m > 250_000 {
        return old.iter().map(|l| (Kind::Del, l.clone())).chain(new.iter().map(|l| (Kind::Add, l.clone()))).collect();
    }
    let mut lcs = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if old[i] == new[j] { lcs[i + 1][j + 1] + 1 } else { lcs[i + 1][j].max(lcs[i][j + 1]) };
        }
    }
    let (mut i, mut j, mut out) = (0, 0, vec![]);
    while i < n || j < m {
        if i < n && j < m && old[i] == new[j] {
            out.push((Kind::Ctx, old[i].clone()));
            (i, j) = (i + 1, j + 1);
        } else if i < n && (j == m || lcs[i + 1][j] >= lcs[i][j + 1]) {
            out.push((Kind::Del, old[i].clone())); // what goes, before what comes instead
            i += 1;
        } else {
            out.push((Kind::Add, new[j].clone()));
            j += 1;
        }
    }
    out
}

#[test]
fn description_diff() {
    let l = |s: &str| s.lines().map(str::to_owned).collect::<Vec<_>>();
    let d = line_diff(&l("a\nb\nc"), &l("a\nB\nc\nd"));
    let k: Vec<Kind> = d.iter().map(|x| x.0).collect();
    assert_eq!(k, [Kind::Ctx, Kind::Del, Kind::Add, Kind::Ctx, Kind::Add]);
    assert_eq!(super::wrap("  - one two", 8), ["  - one", "  two"]);
}
