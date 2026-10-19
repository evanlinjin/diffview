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
        // Some of the commits: their messages, then their code; not the PR's text (it's about
        // all of them).
        let commits = self.commit_shown.is_some();
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

    /// Your replies to Claude's review summary (and its answers), under it.
    fn summary_threads(&self) -> Vec<Row> {
        (0..self.threads.len())
            .filter(|&t| self.threads[t].conversation == ON_SUMMARY && self.in_view(&self.threads[t]))
            .flat_map(|t| self.thread_rows(t, INFO))
            .collect()
    }

    /// The questions for Claude about conversation comment `info[body]` (and its answers),
    /// under it.
    fn conversation_rows(&self, body: usize) -> Vec<Row> {
        let Some(url) = body.checked_sub(1).and_then(|h| self.info.get(h)).filter(|h| h.heading && !h.url.is_empty()).map(|h| &h.url) else { return vec![] };
        (0..self.threads.len())
            .filter(|&t| self.threads[t].conversation == *url && self.in_view(&self.threads[t]))
            .flat_map(|t| self.thread_rows(t, INFO))
            .collect()
    }

    /// The conversation comment with this URL: its body's index in `info`.
    pub(super) fn conversation_item(&self, url: &str) -> Option<usize> {
        (0..self.info.len()).find(|&i| self.info[i].heading && self.info[i].url == url).map(|h| h + 1).filter(|&b| self.info.get(b).is_some_and(|l| l.md))
    }

    /// `info[i]` is the description's heading.
    pub(super) fn is_description(&self, i: usize) -> bool {
        // A replacement's: "description (Claude's draft)".
        self.info.get(i).is_some_and(|l| l.heading && (l.text == "description" || self.replacement.is_some() && l.text.starts_with("description")))
    }

    /// `info[i]` is a replacement's closing note's heading.
    pub(super) fn is_closing_note(&self, i: usize) -> bool {
        self.replacement.is_some() && self.info.get(i).is_some_and(|l| l.heading && l.text.starts_with("closing note"))
    }

    /// `Space o` (or Enter on the line saying there's more): the PR's whole conversation beside
    /// the description, or (again) only its latest. In the whole diff (where it is), the cursor
    /// on the conversation's heading.
    pub(super) fn toggle_conversation(&mut self) {
        if self.pr.is_none() {
            return self.msg = "no PR here".into();
        }
        let start = self.info.iter().position(|l| l.key == "activity");
        // (Not GitHub's answer, what went wrong: its line says.)
        if let Some(s) = start.filter(|&s| !self.info[s].heading) {
            return self.msg = self.info[s].text.clone();
        }
        if !start.is_some_and(|s| self.info[s + 1..].iter().any(|l| l.heading)) {
            return self.msg = "no activity on this PR yet".into();
        }
        // From another tab: to the whole diff, with all of it.
        self.conversation_all = self.view != View::All || !self.conversation_all;
        if self.view != View::All {
            (self.view, self.msg) = (View::All, "the whole diff, with all of the PR's activity".into());
        }
        self.sections_folded.remove(&DESCRIPTION);
        self.info_collapsed = false;
        self.rebuild_at(|r| r.attached && r.line.text.starts_with("activity ("));
    }

    /// A section of PR text in the diff's column: a header like a file's (`name` folds it),
    /// then `text` rendered to the code's width (`info`: the `info` line it is, for `C`).
    fn section(&self, id: usize, title: String, text: &str, info: Option<usize>) -> Vec<Row> {
        let mk = |kind, text| Row { main: Some(id), info, ..Row::text(INFO, kind, text) };
        let mut rows = vec![mk(Kind::File, title)];
        if !self.sections_folded.contains(&id) {
            rows.extend(self.md_rows_at(text, self.wrap_w, mk).into_iter().map(|r| Row { main: Some(id), info, ..r }));
        }
        rows
    }

    /// The PR's facts (author, branches, size, reviews, checks), in a box whose title opens
    /// the PR on GitHub.
    fn pr_facts(&self) -> Vec<Row> {
        let mk = |kind, text| Row::text(INFO, kind, text);
        let facts = self.info.iter().take_while(|l| !l.heading && !l.md).count();
        if facts == 0 || self.unpublished() {
            return vec![];
        }
        let url = self.pr.as_ref().map(|p| p.url.clone()).filter(|u| !u.is_empty());
        let fact = |r: Row| Row { info: Some(FACTS), ..r };
        let title = if url.is_some() { "status · on GitHub ↗" } else { "status" };
        let links = url.iter().map(|u| (9, title.chars().count(), u.clone())).collect();
        let mut rows = vec![fact(Row { links, ..mk(Kind::Thread, title.into()) })];
        // Not wrapped: cut at the box's edge and scrolled sideways, as code is.
        rows.extend(self.info[..facts].iter().map(|l| fact(Row { nowrap: true, ..self.info_row(l, mk(Kind::Comment, l.text.clone())) })));
        rows.push(Row { edge: true, ..fact(mk(Kind::Comment, String::new())) });
        rows
    }

    /// The PR's conversation (its comments and reviews, and the questions to Claude under
    /// them): all of it (`None`), or (`Some(n)`) those new since you looked (after your own last
    /// comment or review, not seen in dv), the last `n` at least, and any with a question under
    /// it; a line first saying how many older ones there are.
    fn conversation(&self, latest: Option<usize>) -> Vec<Row> {
        let mk = |kind, text| Row::text(INFO, kind, text);
        let width = self.info_w();
        // Its items, after its own heading: a comment's or a review's heading then its text, or
        // what else happened (`event`) and the lines under it.
        let Some(start) = self.info.iter().position(|l| l.key == "activity") else { return vec![] };
        let items: Vec<usize> = (start + 1..self.info.len()).filter(|&i| self.info[i].heading).collect();
        // What the latest shows (`LATEST`, or `n`), all of it or not.
        let latest_shown: Vec<usize> = {
            let n = latest.unwrap_or(LATEST);
                let asked = |h: usize| !self.info[h].url.is_empty() && self.threads.iter().any(|t| t.conversation == self.info[h].url && self.in_view(t));
                // New: after your own last word there (you'd read up to it, as GitHub has it),
                // and not seen in dv since.
                let me = format!("@{} ·", self.me);
                let yours = |h: usize| !self.me.is_empty() && self.info[h].text.starts_with(&me);
                let after = items.iter().rposition(|&h| yours(h)).map_or(0, |k| k + 1);
                // (Nowhere to keep what you've seen: nothing's new, as for threads; nor your own.)
                let new = |k: usize, h: usize| {
                    self.seen.path.is_some() && k >= after && !yours(h) && !self.info[h].node.is_empty() && !self.seen.before.contains(&self.info[h].node)
                };
                // From the `n`th last comment or review (events don't count: three of those would
                // leave out the comment before them), the first if there are fewer; of the events
                // since, the last `3n` (many after a comment: the latest of them).
                let said: Vec<usize> = (0..items.len()).filter(|&k| !self.info[items[k]].event).collect();
                let from = said.len().checked_sub(n).map_or(said.first().copied().unwrap_or(0), |k| said[k]);
                let recent = |k: usize| k >= from && (!self.info[items[k]].event || k >= items.len().saturating_sub(3 * n));
                items.iter().enumerate().filter(|&(k, &h)| recent(k) || new(k, h) || asked(h)).map(|(_, &h)| h).collect()
        };
        let shown = if latest.is_some() { &latest_shown } else { &items };
        let mut rows = vec![];
        // (Not GitHub's answer: what went wrong, in its place.)
        if !items.is_empty() || !self.info[start].heading {
            rows.push(Row { info: None, ..mk(Kind::Thread, self.info[start].text.clone()) });
        }
        // What's left out is older (newest last): said first, under its heading.
        let more = items.len() - shown.len();
        if more > 0 {
            let what = if more == 1 { "1 earlier".to_owned() } else { format!("{more} earlier") };
            rows.push(Row { info: Some(MORE_CONVERSATION), ..mk(Kind::Comment, format!("… {what}: Space o shows them")) });
        } else if latest.is_none() && latest_shown.len() < items.len() {
            rows.push(Row { info: Some(MORE_CONVERSATION), ..mk(Kind::Comment, "… all of it: Space o shows only the latest".into()) });
        }
        for h in shown.iter().copied() {
            let body = h + 1;
            // What else happened: its line, and those under it (a push's commits), no box.
            if self.info[h].event {
                let under = self.info[body..].iter().take_while(|l| l.event && !l.heading).count();
                rows.extend((h..body + under).map(|i| Row { info: Some(i), ..self.info_row(&self.info[i], mk(Kind::Comment, self.info[i].text.clone())) }));
                continue;
            }
            // Hidden on GitHub: a line, as a folded thread is (Tab shows it), and what's under it.
            if let Some(note) = self.hidden_note(&self.info[h].hidden, &self.info[h].node) {
                // (The note right after who: a long line cut at the box's edge still says so.)
                let l = &self.info[h];
                let at = l.text.find(" · ").map_or(l.text.chars().count(), |i| l.text[..i].chars().count());
                let n = note.chars().count();
                let rest = l.text.chars().skip(at).collect::<String>();
                let rest = if rest.trim() == "·" { String::new() } else { rest }; // (no time to say)
                let text: String = l.text.chars().take(at).chain(note.chars()).chain(rest.chars()).collect();
                let mut row = self.info_row(l, mk(Kind::Thread, text.clone()));
                for link in &mut row.links {
                    if link.0 >= at {
                        (link.0, link.1) = (link.0 + n, link.1 + n);
                    }
                }
                rows.push(Row { info: self.info.get(body).filter(|n| n.md).map(|_| body), syn: vec![Some(self.ui.stale); text.chars().count()], ..row });
                rows.extend(self.conversation_rows(body));
                continue;
            }
            rows.push(Row { info: self.info.get(body).filter(|n| n.md).map(|_| body), ..self.info_row(&self.info[h], mk(Kind::Thread, self.info[h].text.clone())) });
            let text = self.info.get(body).map_or("", |l| l.text.as_str());
            // A review with no text, just its verdict: GitHub's placeholder, in its box.
            let text = if text.trim().is_empty() { "_(no comment)_" } else { text };
            rows.extend(self.md_rows_at(text, width, mk).into_iter().map(|r| Row { info: Some(body), ..r }));
            rows.push(Row { info: Some(body), edge: true, ..mk(Kind::Comment, String::new()) });
            rows.extend(self.conversation_rows(body));
        }
        rows
    }

    /// The Claude view's, attached to the description: Claude's summary (or, with no review
    /// yet, your questions about the PR), and what's said under it.
    fn claude_attached(&self) -> Vec<Row> {
        let questions = self.threads.iter().any(|t| t.conversation == ON_SUMMARY && self.in_view(t));
        let summary = !self.claude.summary.trim().is_empty();
        if !summary && !questions {
            return vec![];
        }
        let mk = |kind, text| Row::text(INFO, kind, text);
        let mut rows = match summary {
            // (No summary: nothing of it to quote or post, so not the summary's rows.)
            false => vec![mk(Kind::Thread, "questions for Claude".into())],
            true => {
                let summary = |r: Row| Row { info: Some(SUMMARY), ..r };
                let mut rows = vec![summary(mk(Kind::Thread, "claude's review".into()))];
                rows.extend(self.md_rows_at(&self.claude.summary, self.info_w(), mk).into_iter().map(summary));
                rows.push(Row { edge: true, ..summary(mk(Kind::Comment, String::new())) });
                rows
            }
        };
        rows.extend(self.summary_threads());
        rows
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

    /// A comment hidden on GitHub (`why`), not shown here (Tab): what its heading says after
    /// who and when, in place of its text.
    fn hidden_note(&self, why: &str, node: &str) -> Option<String> {
        self.unseen(why, node).then(|| format!(" · {}", hidden_words(why)))
    }

    /// Whether a comment (`hidden` on GitHub, why; its node id) is hidden here still: not to be
    /// quoted (`c`, `C`, `r`, `a`) before you've seen it.
    pub(super) fn unseen(&self, hidden: &str, node: &str) -> bool {
        !hidden.is_empty() && !self.revealed.contains(node)
    }

    /// A review comment's text, as far as you've seen it: none while it's hidden on GitHub
    /// (and not shown here). What quotes, previews or edits it reads it through this.
    pub(super) fn seen_body<'a>(&self, c: &'a gh::Comment) -> Option<&'a str> {
        (!self.unseen(&c.hidden, &c.node_id)).then_some(c.body.as_str())
    }

    /// The same for the PR's text: `info` line `i`, unless it's a conversation comment's,
    /// hidden on GitHub and not shown here.
    pub(super) fn seen_info(&self, i: usize) -> Option<&InfoLine> {
        let head = i.checked_sub(1).and_then(|h| self.info.get(h)).filter(|h| h.heading);
        self.info.get(i).filter(|_| !head.is_some_and(|h| self.unseen(&h.hidden, &h.node)))
    }

    /// A review comment's text for a list, or (hidden on GitHub, unseen) that it's hidden.
    pub(super) fn gist<'a>(&self, c: &'a gh::Comment) -> std::borrow::Cow<'a, str> {
        self.seen_body(c).map_or_else(|| format!("_({})_", hidden_words(&c.hidden)).into(), Into::into)
    }

    /// Whether row `r` is the folded line of a conversation comment hidden on GitHub.
    pub(super) fn hidden_line(&self, r: usize) -> bool {
        self.hidden_row(&self.rows[r])
    }

    fn hidden_row(&self, row: &Row) -> bool {
        row.file == INFO && row.thread.is_none() && row.line.kind == Kind::Thread && row.info.is_some_and(|i| i < self.info.len() && self.seen_info(i).is_none())
    }

    /// The comment hidden on GitHub under the cursor (its node id), for Tab to show or hide:
    /// not on an open thread's first line once it's shown (Tab there folds the thread).
    pub(super) fn hidden_here(&self) -> Option<String> {
        let r = self.head.row;
        match self.rows[r].thread {
            Some(t) if !self.thread_open(t) => None,
            Some(t) => {
                let c = self.comment_under_cursor(t).filter(|c| !c.hidden.is_empty())?;
                let first = r == 0 || self.rows[r - 1].thread != Some(t);
                // On the thread's first line, Tab goes round: shown, hidden again, the thread folded.
                (!(first && self.hid_again.contains(&c.node_id))).then(|| c.node_id.clone())
            }
            None => self.conversation_node().filter(|n| self.info.iter().any(|l| l.node == *n && !l.hidden.is_empty())),
        }
    }

    /// Tab on a comment hidden on GitHub: its text shown, or hidden again. Whether it was one.
    pub(super) fn toggle_hidden(&mut self) -> bool {
        let r = self.head.row;
        let first = self.rows[r].thread.filter(|&t| r == 0 || self.rows[r - 1].thread != Some(t)).and_then(|t| self.comment_under_cursor(t)).map(|c| c.node_id.clone());
        let Some(node) = self.hidden_here() else {
            // Hidden again there: this Tab folds the thread, and unfolded, it starts over.
            if let Some(n) = first {
                self.hid_again.remove(&n);
            }
            return false;
        };
        if first.as_ref() == Some(&node) && self.revealed.contains(&node) {
            self.hid_again.insert(node.clone());
        }
        flip(&mut self.revealed, node.clone());
        // The cursor on its heading, whether its text went or came (else where it was).
        let at = self.rows[self.head.row].thread.map(|t| (self.root_id(t), self.comment_index_under_cursor(t)));
        let (keep, col) = (self.cursor_key(), self.head.col);
        self.rebuild(keep);
        let row = match at {
            Some((id, k)) => {
                let t = (0..self.threads.len()).find(|&t| self.root_id(t) == id);
                self.rows.iter().enumerate().filter(|(_, r)| r.thread.is_some() && r.thread == t && r.line.kind == Kind::Thread).nth(k).map(|(i, _)| i)
            }
            None => self.rows.iter().position(|r| {
                let head = r.info.and_then(|b| self.info.get(b.checked_sub(1)?));
                r.file == INFO && r.line.kind == Kind::Thread && r.thread.is_none() && head.is_some_and(|l| l.heading && l.node == node)
            }),
        };
        if let Some(row) = row {
            self.goto(row, col);
        }
        true
    }

    /// With no PR (a branch Claude reviewed), Claude's review is all the PR section has.
    fn no_pr_summary(&self) -> Vec<Row> {
        match self.pr.is_none() && self.replacement.is_none() {
            true => self.claude_attached(),
            false => vec![],
        }
    }

    /// A section with what's `attached` to it: beside it from its header on (the pane), or
    /// after it.
    fn attach(&self, mut s: Vec<Row>, attached: &mut Vec<Row>) -> Vec<Row> {
        let at = if self.margin_active { 1 } else { s.len() };
        s.splice(at..at, std::mem::take(attached));
        s
    }

    /// What you asked of an unpublished draft's drafts `on` (`replace::Request::on`), a box
    /// each, oldest first, saying whether a redraft has taken it in: beside the draft it's about.
    /// (Each row is its draft's, `info`: `a` there asks about that draft too.)
    fn request_rows(&self, on: &[&str], draft: usize) -> Vec<Row> {
        let Some(r) = self.replacement.as_ref().filter(|_| self.unpublished()) else { return vec![] };
        let mk = |kind, text| Row { info: Some(draft), ..Row::text(INFO, kind, text) };
        let drafting = r.drafting_now();
        let mut rows = vec![];
        for (i, a) in r.asks.iter().enumerate().filter(|(_, a)| on.contains(&a.on.as_str())) {
            // (A question is answered; anything else, redrafted with.)
            let [done, doing, todo] = match a.want.as_str() {
                "ask" => ["answered", "Claude is answering it", "not answered yet"],
                _ => ["redrafted with it", "Claude is redrafting with it", "not redrafted with it yet"],
            };
            let state = match (a.done(), drafting && i < r.taking) {
                (true, _) => done,
                (false, true) => doing,
                (false, false) => todo,
            };
            let want = match a.want.as_str() {
                "ask" => " [ask]",
                "change" => " [change]",
                _ => "",
            };
            rows.push(mk(Kind::Thread, format!("you · asked of the drafts{want} · {state}")));
            rows.extend(self.md_rows_at(&a.text, self.info_w(), mk));
            if !a.answer.trim().is_empty() {
                rows.push(mk(Kind::Thread, "◆ claude".into()));
                rows.extend(self.md_rows_at(&a.answer, self.info_w(), mk));
            }
            rows.push(Row { edge: true, ..mk(Kind::Comment, String::new()) });
        }
        rows
    }

    /// Your pending review's text, in full, as it goes to GitHub with the review (`S`).
    fn draft_rows(&self) -> Vec<Row> {
        if self.summary_draft.trim().is_empty() {
            return vec![];
        }
        let mk = |kind, text| Row::text(INFO, kind, text);
        let draft = |r: Row| Row { info: Some(DRAFT), ..r };
        let mut rows = vec![draft(mk(Kind::Thread, "your review (pending)".into()))];
        rows.extend(self.md_rows_at(&self.summary_draft, self.info_w(), mk).into_iter().map(draft));
        rows.push(Row { edge: true, ..draft(mk(Kind::Comment, String::new())) });
        rows
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
        pane_gw(self.margin_float)
    }

    /// Color a PR info line by its marks: badges, pass/fail colors, links.
    pub(super) fn info_row(&self, l: &InfoLine, mut row: Row) -> Row {
        let ui = &self.ui;
        row.syn = vec![None; l.text.chars().count()];
        for (a, b, m) in &l.marks {
            let st = match m {
                Mark::Chip(hex) => chip(hex),
                Mark::Pass => ui.plus,
                Mark::Fail => ui.error,
                Mark::Pending => ui.warning,
                Mark::Dim => ui.stale,
                Mark::Mine => ui.key,
                Mark::Link(url) => {
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
        let want = agent::want_tag(t);
        let tags: Vec<(&str, Style)> = [
            (self.is_new(t), "[new] ", self.ui.key),
            (first_pending, "[pending] ", self.ui.pending),
            (t.outdated, "[outdated] ", self.ui.warning),
            (t.resolved, "[resolved] ", self.ui.resolved),
            (!t.severity.is_empty(), severity.as_str(), if serious { self.ui.warning } else { self.ui.resolved }),
            (turn.is_some(), turn.unwrap_or(""), self.ui.pending),
            (want.is_some(), want.unwrap_or(""), self.ui.pending),
        ]
            .into_iter()
            .filter_map(|(on, s, c)| on.then_some((s, c)))
            .collect();
        let tag_text: String = tags.iter().map(|t| t.0).collect();
        let tag_syn: Vec<Option<Style>> =
            tags.iter().flat_map(|&(s, st)| s.chars().map(move |_| Some(st))).collect();
        let mk = |kind, text| Row { thread: Some(ti), ..Row::text(file, kind, text) };
        // Claude's comments (from its review file) carry its name in the accent color, after a `◆`
        // (as the gutter marks them), so they don't pass for GitHub's.
        let by_claude = |c: &gh::Comment| t.claude.is_some() && c.author == crate::claude::AUTHOR;
        let name = |c: &gh::Comment| if by_claude(c) { format!("◆ {}", c.author) } else { c.author.clone() };
        let author_syn = |c: &gh::Comment| vec![by_claude(c).then_some(self.ui.key); name(c).chars().count()];
        // One answering a GitHub review comment says whose.
        let re = t
            .about
            .and_then(|id| self.threads.iter().flat_map(|t| &t.comments).find(|c| c.id == id))
            .map_or(String::new(), |c| format!(" · re @{}", c.author));
        let when = |c: &gh::Comment| match gh::ago(&c.created) {
            a if a.is_empty() => String::new(),
            a => format!(" · {a}"),
        };
        // Its reactions, after who, when and where; yours in the accent color.
        let reacts = |c: &gh::Comment| -> (String, Vec<Option<Style>>) {
            let (mut text, mut syn) = (String::new(), vec![]);
            for (i, (t, mine)) in reaction_parts(&c.reactions).into_iter().enumerate() {
                let gap = if i == 0 { " · " } else { "  " };
                syn.extend(vec![None; gap.chars().count()]);
                syn.extend(vec![mine.then_some(self.ui.key); t.chars().count()]);
                text += gap;
                text += &t;
            }
            (text, syn)
        };
        // A line's syntax: `syn` up to `at` chars, then the reactions'.
        let with = |mut syn: Vec<Option<Style>>, at: usize, r: Vec<Option<Style>>| {
            syn.resize(at, None);
            syn.extend(r);
            syn
        };
        let mut out = vec![];
        // A hidden comment's note (columns `a..b` of its heading), dimmed. (Right after who
        // wrote it: a long heading cut at the box's edge still says so.)
        let dim_note = |mut syn: Vec<Option<Style>>, a: usize, b: usize| {
            if syn.len() < b {
                syn.resize(b, None);
            }
            syn[a..b].fill(Some(self.ui.stale));
            syn
        };
        for (k, c) in t.comments.iter().enumerate() {
            let note = self.hidden_note(&c.hidden, &c.node_id);
            if k == 0 {
                // In the margin the tags get a line of their own, under the header, so the header
                // keeps room for who, when and where.
                let tags_below = self.margin_active && !tag_text.is_empty();
                let lead = if tags_below { String::new() } else { tag_text.clone() };
                let (rt, rs) = reacts(c);
                let mut head = format!("{lead}{}", name(c));
                let ns = head.chars().count();
                head += note.as_deref().unwrap_or("");
                let ne = head.chars().count();
                head += &format!("{re}{}{range}", when(c));
                let at = head.chars().count();
                head += &rt;
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
                let syn = if rs.is_empty() { syn } else { with(syn, at, rs) };
                out.push(Row { syn: dim_note(syn, ns, ne), ..mk(Kind::Thread, head) });
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
                let (rt, rs) = reacts(c);
                let who = format!("{tag}{}", name(c));
                let note = note.as_deref().unwrap_or("");
                let (ns, ne) = (who.chars().count(), who.chars().count() + note.chars().count());
                let head = format!("{who}{note}{}", when(c));
                let syn = if rs.is_empty() { syn } else { with(syn, head.chars().count(), rs) };
                out.push(Row { syn: dim_note(syn, ns, ne), ..mk(Kind::Thread, format!("{head}{rt}")) });
            }
            // Hidden on GitHub: its heading says so, and that's all (Tab shows it).
            if note.is_some() {
                continue;
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
            // (A section's text: plain, as code; the line saying there's more conversation, too.)
            (None, Some(i)) if r.file == INFO && r.main.is_none() && i != MORE_CONVERSATION && !self.hidden_row(r) && !self.info.get(i).is_some_and(|l| l.event) => Some((None, Some(i))),
            _ => None,
        };
        let row = &self.rows[r];
        let k = key(row)?;
        if row.thread.is_some_and(|t| !self.thread_open(t)) {
            return None; // a folded thread stays a single line
        }
        // (After a box's bottom edge, another box: boxes in a row on the same text, as what
        // you asked of a draft.)
        let first = r == 0 || self.rows[r - 1].edge || key(&self.rows[r - 1]) != Some(k);
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

/// The pane's gutter, floating (`float`) or as a column: see `App::pane_gw`.
pub(super) fn pane_gw(float: bool) -> usize {
    if float { 3 } else { 1 }
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
