//! The rows dv shows: the diff's lines, comment threads and the PR section, built from the
//! files and threads (and rebuilt keeping the cursor where it was); folding and context.

use super::*;

/// How far (in lines) an outdated thread may land from its original line number.
pub(super) const OUTDATED_SLACK: i64 = 10;

/// The Claude / pending view's windows of `f` (as `diff::windows`): each of `threads` with its
/// lines and a few around them (± its `extra`), merged where they meet, each with the comments
/// it covers. `whole` shows the entire file.
pub(super) fn view_windows(threads: &[&Thread], f: &FileDiff, extra: impl Fn(&Wide) -> i64, whole: bool) -> Vec<(usize, usize, Vec<Wide>)> {
    let spans = threads.iter().filter(|t| t.path == f.path).filter_map(|t| {
        let k = comment_wide(t);
        let at = anchor_line(t, &f.full)?;
        // (A range: from its first line, wherever that is in the diff; removed lines between
        // don't count toward it.)
        let first = t.range_start().and_then(|(s, left)| line_on(left, &f.full, s)).filter(|&s| s <= at).unwrap_or(at);
        let x = (DEFAULT_CONTEXT as i64 + extra(&k)).max(0) as usize;
        Some((first.saturating_sub(x), at + x, k))
    });
    diff::merge_windows(spans.collect(), f.full.len(), whole)
}

/// What widened context (`+`/`-`) is kept by: a hunk of the whole diff, by its file and where
/// it is in the base (its first line's old number, or for an added line, the one before it:
/// the same hunk across reloads, as the working tree changes); or a comment's window in the
/// Claude / pending views, by the comment.
#[derive(Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Wide {
    Hunk(String, u32),
    Comment(String, String),
}

/// The hunk of `f` at index `a` of its whole diff.
fn hunk_wide(f: &FileDiff, a: usize) -> Wide {
    let old = f.full[..=a].iter().rev().find_map(|l| l.old).or_else(|| f.full[a..].iter().find_map(|l| l.old));
    Wide::Hunk(f.path.clone(), old.unwrap_or(0))
}

/// Thread `t`'s window (its Claude key, else its GitHub id).
fn comment_wide(t: &Thread) -> Wide {
    Wide::Comment(t.path.clone(), t.claude.clone().unwrap_or_else(|| t.id.clone()))
}

/// The whole diff's windows of `f`: its hunks, widened as `extra` keeps them, each with what
/// widens it.
fn hunk_windows(f: &FileDiff, extra: &HashMap<Wide, i64>, whole: bool) -> Vec<(usize, usize, Vec<Wide>)> {
    let ctx = DEFAULT_CONTEXT as usize;
    let spans = diff::hunks(&f.full, ctx).into_iter().map(|(a, b)| {
        let k = hunk_wide(f, a);
        let x = (ctx as i64 + *extra.get(&k).unwrap_or(&0)).max(0) as usize;
        (a.saturating_sub(x), b + x, k)
    });
    diff::merge_windows(spans.collect(), f.full.len(), whole)
}

/// Range start `start` (a line and its side) as a line on side `left`: itself on that side;
/// from the other, the first line of `f`'s diff on `left`'s side from there on (what reads only
/// one side, the review file and the code quoted, takes a range as one side's).
pub(super) fn start_on_side(f: &FileDiff, (s, from): (u32, bool), left: bool) -> Option<u32> {
    if from == left {
        return Some(s);
    }
    let i = line_on(from, &f.full, s)?;
    f.full[i..].iter().find_map(|l| if left { l.old } else { l.new })
}

/// The diff line of line `n` on a side (`left`: the old one).
fn line_on(left: bool, lines: &[DLine], n: u32) -> Option<usize> {
    let side = |l: &DLine| match left {
        true => matches!(l.kind, Kind::Ctx | Kind::Del),
        false => matches!(l.kind, Kind::Ctx | Kind::Add),
    };
    lines.iter().position(|l| side(l) && if left { l.old } else { l.new } == Some(n))
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
        return line_on(t.left, lines, t.line?);
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
            let w = hunk_windows(f, extra, whole.contains(&f.path));
            f.lines = diff::shown(&f.full, &w, &f.headers);
        }
    }

    /// File `f`'s windows in this view (the whole diff's hunks, or the Claude / pending view's
    /// comments), each with what widens it: what `+`/`-` and showing a hidden line go by.
    pub(super) fn windows_of(&self, f: &FileDiff) -> Vec<(usize, usize, Vec<Wide>)> {
        let whole = self.whole.contains(&f.path);
        if self.view == View::All {
            return hunk_windows(f, &self.extra, whole);
        }
        let ts: Vec<&Thread> = self.threads.iter().filter(|t| self.in_view(t)).collect();
        view_windows(&ts, f, |k| *self.extra.get(k).unwrap_or(&0), whole)
    }

    pub(super) fn cursor_key(&self) -> CursorKey {
        self.key_at(self.head.row)
    }

    fn key_at(&self, row: usize) -> CursorKey {
        let Some(r) = self.rows.get(row) else { return CursorKey { line: None, thread: None, info: None, section: None, goes: None } };
        CursorKey {
            // (The code line it's on, or under: a comment's, should its thread go, as one posted
            // or deleted does, outdated or not; under none shown, its own line.)
            line: self.files.get(r.file).map(|f| {
                let at = match r.thread {
                    Some(_) => self.rows[..=row].iter().rev().take_while(|x| x.file == r.file).find(|x| x.line.kind.is_code()),
                    None => Some(r),
                };
                let own = || r.thread.and_then(|t| self.threads[t].line.map(|l| if self.threads[t].left { (Some(l), None) } else { (None, Some(l)) }));
                let (old, new) = at.map(|x| (x.line.old, x.line.new)).or_else(own).unwrap_or_default();
                (f.path.clone(), old, new)
            }),
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
            goes: r.goes.and_then(|(t, c)| Some(((self.root_id(t)?, self.threads[t].comments.get(c)?.id), row - self.rows.iter().position(|x| x.goes == r.goes)?))),
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
        // (A line standing for a comment: that comment's, wherever the list put it.)
        let goes = key.goes.and_then(|((root, id), off)| {
            let first = self.rows.iter().position(|r| r.goes.is_some_and(|(t, k)| self.root_id(t) == Some(root) && self.threads[t].comments.get(k).is_some_and(|c| c.id == id)))?;
            let g = self.rows[first].goes;
            Some((first..=first + off).take_while(|&r| self.rows.get(r).is_some_and(|r| r.goes == g)).last().unwrap_or(first))
        });
        goes.or_else(|| self.find_key_rest(key))
    }

    /// (`find_key`, but a line standing for a comment.)
    fn find_key_rest(&self, key: CursorKey) -> Option<usize> {
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
            .or_else(|| key.line.as_ref().and_then(|(p, old, new)| self.locate(p, *old, *new)))
            // The comment the cursor was on gone, with its file from the view (Claude's last
            // there posted): where the file would be. (Not so for a line: a view without it
            // starts at its top.)
            .or_else(|| key.thread.and(key.line.as_ref()).and_then(|(p, ..)| self.near_file(p)))
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
        let mut rows = self.title_rows();
        // Attached to the description (beside it in the pane, as comments are beside their
        // lines; inline after it, the pane hidden): with no description (a branch), after the
        // title.
        let mut attached = self.attached_rows();
        if self.whole_pr() {
            rows.extend(self.closing_comment());
            rows.extend(self.description_rows(&mut attached));
        }
        rows.extend(self.attach(vec![], &mut attached)); // (marked as attached all the same)
        rows.extend(self.closing_note_rows());
        rows.extend(self.commit_sections());
        let gone_at = rows.len();
        rows.extend(self.gone_rows());
        let shown: Vec<&Thread> = self.threads.iter().filter(|t| self.in_view(t)).collect();
        for (i, f) in self.files.iter().enumerate() {
            rows.extend(self.file_rows(i, f, &shown));
        }
        if rows.is_empty() {
            rows.push(Row::text(usize::MAX, Kind::Hunk, format!("({})", self.no_files())));
        } else if self.view == View::Claude && !self.files.is_empty() && !rows.iter().any(|r| r.file < self.files.len() || r.thread.is_some_and(|t| self.threads[t].claude.is_some())) {
            // (Its code is the files Claude commented on: none, it says so, not an empty page.)
            rows.push(Row::text(usize::MAX, Kind::Hunk, "(no comments from Claude on the code: gn / gp, the whole diff)".into()));
        }
        self.rows = rows;
        self.gone_at = gone_at;
        self.code_head.row = self.find_key(code).unwrap_or(0);
        self.emph = self.word_spans();
        self.layout();
    }

    /// The pinned title's text: the PR's number and title, a replacement's or new PR's, or (a
    /// branch Claude reviewed) "Claude's review"; none otherwise.
    fn title(&self) -> Option<String> {
        match (&self.pr, &self.replacement) {
            (Some(pr), _) => Some(format!("PR #{} · {}", pr.number, pr.title)),
            (None, Some(r)) => {
                let title = if r.title.is_empty() { "(not drafted yet)" } else { r.title.as_str() };
                Some(match crate::replace::is_new(r.id) {
                    true => format!("{} · {title}", crate::replace::label(r.id).replacen("new PR", "New PR", 1)),
                    false => format!("Replacement of #{} · {title}", r.id),
                })
            }
            (None, None) => (!self.claude.summary.trim().is_empty()).then(|| "Claude's review".to_owned()),
        }
    }

    /// The title row (it links to the PR; who, and from which branch into which, after it, dim);
    /// in the pending view, the PR's facts line and your pending review's text attached to it.
    fn title_rows(&self) -> Vec<Row> {
        let Some(title) = self.title() else { return vec![] };
        let mk = |kind, text| Row::text(INFO, kind, text);
        let url = self.pr.as_ref().map(|p| p.url.clone()).filter(|u| !u.is_empty());
        let links = url.map(|u| vec![(0, title.chars().count(), u)]).unwrap_or_default();
        let by = self.pr.as_ref().filter(|p| !p.author.is_empty()).map(|p| {
            let head = if p.cross_repo { format!("{}:{}", p.head_owner, p.head_branch) } else { p.head_branch.clone() };
            format!("  @{} · {} ← {head}", p.author, p.base)
        });
        let n = title.chars().count();
        let by = by.unwrap_or_default();
        let syn = [vec![None; n], vec![Some(self.ui.linenr); by.chars().count()]].concat();
        let rows = vec![Row { links, syn, ..mk(Kind::File, title + &by) }];
        if self.view != View::Pending || self.info_collapsed {
            return rows;
        }
        let facts = self.info.first().filter(|l| !l.heading && !l.md).map(|l| self.info_row(l, mk(Kind::Comment, l.text.clone())));
        let mut attached: Vec<Row> = facts.into_iter().chain(self.draft_rows()).collect();
        self.attach(rows, &mut attached)
    }

    /// What's attached to the description: the whole diff's, the PR's facts, its activity's
    /// latest (`Space o`: all of it), your pending review's text; Claude's side's, its summary
    /// (or your questions about the PR), what's said under it, its activity (a draft's: what
    /// you asked of it too). Not with some of the commits, nor the description folded.
    fn attached_rows(&self) -> Vec<Row> {
        match self.view {
            _ if !self.whole_pr() || self.sections_folded.contains(&DESCRIPTION) => vec![],
            _ if self.claude_side() => {
                let desc = self.heading(Self::is_description).map_or(0, |h| h + 1);
                [self.claude_attached(), self.request_rows(&["description", ""], desc)].into_iter().flatten().collect()
            }
            View::All if self.info_collapsed => vec![],
            View::All => {
                let latest = (!self.conversation_all).then_some(LATEST);
                [self.pr_facts(), self.conversation(latest), self.no_pr_summary(), self.draft_rows()].into_iter().flatten().collect()
            }
            View::Pending | View::Claude => vec![],
        }
    }

    /// Claude's draft comment for closing the PR: first, as what you'd do next.
    fn closing_comment(&self) -> Vec<Row> {
        if self.view != View::All || self.claude.closing.trim().is_empty() {
            return vec![];
        }
        let what = match self.pr_state.is_some_and(|s| matches!(s.0, "OPEN" | "DRAFT")) {
            true => "closing comment · Claude's draft (Space a: close the PR with it, more changes, discard)",
            false => "closing comment · Claude's draft (the PR isn't open: Space a discards it)",
        };
        self.section(CLOSING_PR, what.into(), &self.claude.closing, None)
    }

    /// Before the files, like one: the PR's description (in the Claude view too), what's
    /// `attached` to it; or Claude's proposal for it, what it changes, line by line.
    fn description_rows(&self, attached: &mut Vec<Row>) -> Vec<Row> {
        let Some(h) = self.heading(Self::is_description).filter(|_| matches!(self.view, View::All | View::Claude)) else { return vec![] };
        let text = self.info_body(h);
        let s = match self.claude.description.is_empty() || self.pr.is_none() {
            true => {
                let text = match (text.trim().is_empty(), self.replacement.is_some()) {
                    (true, true) => "_(not drafted yet)_",
                    (true, false) => "_(no description)_",
                    _ => text,
                };
                let mut s = self.section(DESCRIPTION, self.info[h].text.clone(), text, Some(h + 1));
                // An unpublished draft: what it replaces (or what you asked for) first, as it
                // has no facts box. (Its info line's, not the draft's text: `a` there quotes none.)
                if let Some(l) = self.info.first().filter(|_| self.unpublished() && s.len() > 1) {
                    let mk = |text| Row { main: Some(DESCRIPTION), info: Some(0), ..Row::text(INFO, Kind::Comment, text) };
                    s.splice(1..1, [self.info_row(l, mk(l.text.clone())), mk(String::new())]);
                }
                s
            }
            // By source line (a reworded paragraph is one changed line), each then wrapped to
            // the code's width (as markdown source).
            false => self.section_with(DESCRIPTION, "description · Claude's proposal (Space a: approve, more changes, discard)".into(), Some(h + 1), |mk| {
                let w = self.wrap_w.saturating_sub(2).max(20);
                let lines = |t: &str| t.replace("\r\n", "\n").lines().map(str::to_owned).collect::<Vec<_>>();
                line_diff(&lines(text), &lines(&self.claude.description)).into_iter().flat_map(|(k, line)| wrap(&line, w).into_iter().map(move |t| mk(k, t))).collect()
            }),
        };
        self.attach(s, attached)
    }

    /// A replacement's closing note (for the PRs it replaces), after its description: Claude's
    /// draft (unpublished), what you asked of it beside it; published, on its PR, still what
    /// closes what it replaces (`S`), while any is open.
    fn closing_note_rows(&self) -> Vec<Row> {
        if self.view != View::All || !self.whole_pr() {
            return vec![];
        }
        let open = |r: &&crate::replace::Record| r.replaces.iter().any(|o| !r.closed.contains(&o.number));
        if let Some(r) = self.replacement.as_ref().filter(|_| self.pr.is_some()).filter(open) {
            return self.section(CLOSING_NOTE, "closing note, for the PRs it replaces (S closes them with it)".into(), &r.closing_note, None);
        }
        let Some(h) = self.heading(Self::is_closing_note) else { return vec![] };
        let text = match self.info_body(h) {
            t if t.trim().is_empty() => "_(not drafted yet)_",
            t => t,
        };
        let s = self.section(CLOSING_NOTE, self.info[h].text.clone(), text, Some(h + 1));
        let mut asked = match self.sections_folded.contains(&CLOSING_NOTE) {
            true => vec![],
            false => self.request_rows(&["closing note"], h + 1),
        };
        self.attach(s, &mut asked)
    }

    /// Some of the commits shown: each one's message, a section.
    fn commit_sections(&self) -> Vec<Row> {
        let Some((a, _)) = self.commit_shown.filter(|_| self.view == View::All) else { return vec![] };
        let commits = self.commit_msgs.iter().enumerate().map_while(|(k, msg)| Some((self.commits.get(a + k)?, k, msg)));
        commits.flat_map(|(c, k, msg)| self.section(1 + a + k, format!("commit {}", c.short), msg, None)).collect()
    }

    /// Threads that would show nowhere, listed (in the pane, as the PR's text is; inline with
    /// it hidden), by file, then on no file: on files that have left the diff (a change
    /// reverted, a file now matching the base); questions about a commit (on no lines, under
    /// "about commit …", whatever's shown); questions about the conversation not under their
    /// comment (it isn't shown, or not in the whole diff). Those about the summary are under
    /// it. Not with only some of the PR shown (`whole_pr`): most files are out of the diff on
    /// purpose then.
    fn gone_rows(&self) -> Vec<Row> {
        let unplaced = |th: &Thread| th.conversation != ON_SUMMARY && (self.view != View::All || self.conversation_item(&th.conversation).is_none());
        let mut gone: Vec<usize> = (0..self.threads.len())
            .filter(|&t| {
                let th = &self.threads[t];
                self.in_view(th) && !self.has_file(&th.path) && (self.on_code(th) || self.whole_pr() && unplaced(th))
            })
            .collect();
        let label = |t: usize| {
            let th = &self.threads[t];
            match (th.path.is_empty(), commit_of(&th.conversation)) {
                (false, _) => th.path.clone(),
                (true, Some(sha)) => format!("about commit {}", self.commit_short(sha)),
                (true, None) => "questions about the PR's conversation".into(),
            }
        };
        gone.sort_by_cached_key(|&t| (self.threads[t].path.is_empty(), label(t)));
        let mut rows = vec![];
        let heading = |text: String| Row::text(INFO, Kind::Thread, text);
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
            rows.extend(self.thread_rows(t, INFO));
        }
        self.attach(vec![], &mut rows)
    }

    /// File `i`'s rows: its header, then its lines with the threads `shown` on them (the Claude
    /// and pending views: only a file with some, and only around them); folded, its header.
    fn file_rows(&self, i: usize, f: &FileDiff, shown: &[&Thread]) -> Vec<Row> {
        let around;
        let lines = match self.view != View::All {
            true if !shown.iter().any(|t| t.path == f.path) => return vec![],
            true => {
                around = diff::shown(&f.full, &self.windows_of(f), &f.headers);
                &around
            }
            false => &f.lines,
        };
        let mut rows = vec![self.file_header(i, f, shown)];
        let theirs = (0..self.threads.len()).filter(|&t| self.threads[t].path == f.path && self.in_view(&self.threads[t]));
        if self.collapsed.contains(&f.path) {
            // Folded, its comments are in the pane all the same, by their headings, beside the
            // file's (Tab on one unfolds the file there).
            if self.comments_in_pane() {
                rows.extend(theirs.flat_map(|t| self.thread_rows_as(t, i, false)));
            }
            return rows;
        }
        // (thread, index into its lines it sits under); unplaceable ones under the header.
        // Found in the whole diff (as the views' windows are), not just what's shown: an
        // outdated comment's line is then the same however much context shows. Under it if it's
        // shown; else (hidden context) under the header.
        // (The shown lines by their numbers, in a file with comments only.)
        let theirs: Vec<usize> = theirs.collect();
        let shown_at: HashMap<(Option<u32>, Option<u32>), usize> = match theirs.is_empty() {
            true => HashMap::new(),
            false => lines.iter().enumerate().filter(|(_, l)| l.kind != Kind::Hunk).map(|(i, l)| ((l.old, l.new), i)).collect(),
        };
        let at = |t: usize| {
            let k = anchor_line(&self.threads[t], &f.full)?;
            shown_at.get(&(f.full[k].old, f.full[k].new)).copied()
        };
        let placed: Vec<(usize, Option<usize>)> = theirs.into_iter().map(|t| (t, at(t))).collect();
        for &(t, _) in placed.iter().filter(|p| p.1.is_none()) {
            rows.extend(self.thread_rows(t, i));
        }
        for (li, l) in lines.iter().enumerate() {
            rows.push(Row::new(i, l.clone()));
            for &(t, _) in placed.iter().filter(|p| p.1 == Some(li)) {
                rows.extend(self.thread_rows(t, i));
            }
        }
        rows
    }

    /// A file's header: "dir/" dimmed, its name in the header color, "(new)" and such dimmed,
    /// +/- colored, and its open comments when they're inline (said folded too, so none go
    /// unseen; in the pane, they're there, folded or not).
    fn file_header(&self, i: usize, f: &FileDiff, shown: &[&Thread]) -> Row {
        let count = |k| f.lines.iter().filter(|l| l.kind == k).count();
        let (dir, name) = f.path.rsplit_once('/').map_or(("", f.path.as_str()), |(d, n)| (d, n));
        let dir = if dir.is_empty() { String::new() } else { format!("{dir}/") };
        let meta = if f.meta.is_empty() { String::new() } else { format!(" ({})", f.meta) };
        let dim = Some(self.ui.stale);
        let comments = match shown.iter().filter(|t| t.path == f.path && !t.stale() && !self.comments_in_pane()).count() {
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
        Row { syn, ..Row::text(i, Kind::File, text) }
    }

    /// The `info` heading that `is` (the description's, a closing note's), if there's one.
    fn heading(&self, is: fn(&Self, usize) -> bool) -> Option<usize> {
        (0..self.info.len()).find(|&i| is(self, i))
    }

    /// The text under `info` heading `h` (its markdown line), or "".
    fn info_body(&self, h: usize) -> &str {
        self.info.get(h + 1).filter(|l| l.md).map_or("", |l| l.text.as_str())
    }

    /// Pair each run of removed rows with the added rows after it (as split view does) and
    /// find the changed words in each pair.
    pub(super) fn word_spans(&self) -> HashMap<usize, Vec<(usize, usize)>> {
        // (Over the code alone, comments between its lines aside: as split view pairs them.)
        let code: Vec<usize> = (0..self.rows.len()).filter(|&i| !self.in_margin(i)).collect();
        let kind = |k: usize| self.rows[code[k]].line.kind;
        let (n, mut out, mut i) = (code.len(), HashMap::new(), 0);
        while i < n {
            if kind(i) != Kind::Del {
                i += 1;
                continue;
            }
            let dels = (i..n).take_while(|&j| kind(j) == Kind::Del).count();
            let adds = (i + dels..n).take_while(|&j| kind(j) == Kind::Add).count();
            for k in 0..dels.min(adds) {
                let (d, a) = (code[i + k], code[i + dels + k]);
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
        commit_of(&t.conversation).is_some() || !t.path.is_empty() && (self.whole_pr() || self.has_file(&t.path))
    }

    /// The whole PR shown: not some of its commits, nor only what isn't committed (then its
    /// text, about all of it, isn't; nor what's on files out of the diff, most on purpose).
    pub(super) fn whole_pr(&self) -> bool {
        !(self.uncommitted() || self.commit_shown.is_some())
    }

    fn has_file(&self, path: &str) -> bool {
        self.files.iter().any(|f| f.path == path)
    }

    /// Which section row `r` is in (`Row::main`), if it's one.
    pub(super) fn section_of(&self, r: usize) -> Option<usize> {
        self.rows.get(r)?.main
    }

    /// Rebuild rows and put the cursor on the first row matching `at` (else the top).
    pub(super) fn rebuild_at(&mut self, at: impl Fn(&Row) -> bool) {
        self.build();
        self.select = false;
        let r = self.rows.iter().position(at).unwrap_or(0);
        self.goto(r, 0);
    }

    /// Row in file `path` at (or just after) the given line, else the file header; the file not
    /// shown, none.
    pub(super) fn locate(&self, path: &str, old: Option<u32>, new: Option<u32>) -> Option<usize> {
        let fi = self.files.iter().position(|f| f.path == path)?;
        let in_file = |r: &&Row| r.file == fi;
        let hit = match (new, old) {
            (Some(n), _) => self.rows.iter().position(|r| in_file(&r) && r.line.new.is_some_and(|x| x >= n)),
            (None, Some(o)) => self.rows.iter().position(|r| in_file(&r) && r.line.old.is_some_and(|x| x >= o)),
            _ => None,
        };
        hit.or_else(|| self.rows.iter().position(|r| r.file == fi))
    }

    /// Where file `path` would be, it not shown: the next file's first row, else the last before.
    fn near_file(&self, path: &str) -> Option<usize> {
        let fi = self.files.iter().position(|f| f.path == path)?;
        let file = |r: &Row| r.file < self.files.len();
        self.rows.iter().position(|r| file(r) && r.file > fi).or_else(|| self.rows.iter().rposition(|r| file(r) && r.file < fi))
    }

    /// `+`/`-`: more or less context around the hunk under the cursor (on a header: the next one).
    pub(super) fn expand(&mut self, d: i64) {
        let row = &self.rows[self.head.row];
        let fi = row.file;
        let Some(f) = self.files.get(fi) else { return };
        let path = f.path.clone();
        if d < 0 && self.whole.remove(&path) {
            return self.reshape();
        }
        // The line the cursor's at in the whole diff: a comment's, its own (shown or not);
        // else the next code line's.
        let idx = match row.thread {
            Some(t) => anchor_line(&self.threads[t], &f.full),
            None => self.rows[self.head.row..]
                .iter()
                .take_while(|r| r.file == fi)
                .find(|r| r.line.kind.is_code())
                .and_then(|c| f.full.iter().position(|x| x.old == c.line.old && x.new == c.line.new)),
        };
        let Some(idx) = idx else { return };
        let Some((_, _, keys)) = self.windows_of(f).into_iter().find(|w| w.0 <= idx && idx <= w.1) else {
            // Its line in hidden context (a comment under the header): the window nearest it, on
            // to it.
            if d > 0 {
                self.grow_to(fi, None, idx);
            }
            return;
        };
        for k in keys {
            let x = self.extra.entry(k).or_insert(0);
            *x = (*x + d).max(-(DEFAULT_CONTEXT as i64));
        }
        self.reshape();
    }

    /// Widen file `fi`'s window (in this view) at index `from` of its whole diff (none: the one
    /// nearest `to`) until it shows index `to`. Whether one did.
    pub(super) fn grow_to(&mut self, fi: usize, from: Option<usize>, to: usize) -> bool {
        let ws = self.windows_of(&self.files[fi]);
        let gap = |w: &(usize, usize, Vec<Wide>)| if to > w.1 { to - w.1 } else { w.0.saturating_sub(to) };
        let w = match from {
            Some(i) => ws.into_iter().find(|w| w.0 <= i && i <= w.1),
            None => ws.into_iter().min_by_key(gap),
        };
        let Some(w) = w else { return false };
        let grow = gap(&w) as i64;
        for k in w.2 {
            *self.extra.entry(k).or_insert(0) += grow;
        }
        self.reshape();
        true
    }

    /// The whole diff again, from its base as now (another base, a commit or range let go):
    /// its hunks other ones, their context afresh. Every such change goes through here.
    pub(super) fn whole_diff_again(&mut self) {
        (self.to, self.commit_shown, self.commit_anchor, self.shown_first) = (None, None, None, None);
        self.hunks_afresh();
    }

    /// The diff from `base` now: another one, its hunks are other ones (their widened context
    /// let go). Every change of the base goes through here.
    pub(super) fn set_base(&mut self, base: String) {
        if base != self.base {
            self.base = base;
            self.hunks_afresh();
        }
    }

    /// The diff's hunks are other ones (another base, a commit's or a range's): their widened
    /// context, the whole diff's and the views', let go.
    pub(super) fn hunks_afresh(&mut self) {
        self.extra.clear();
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
            flip(&mut self.sections_folded, id);
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
        // (From a comment of a folded file's, in the pane: on it, unfolded.)
        match self.rows[self.head.row].thread {
            Some(t) => self.rebuild_at(|r| r.thread == Some(t)),
            None => self.rebuild_at(|r| r.file == fi),
        }
    }

    /// On a comment of a folded file's (its heading, in the pane beside the file's).
    pub(super) fn on_folded_file_comment(&self) -> bool {
        let r = &self.rows[self.head.row];
        r.thread.is_some() && self.files.get(r.file).is_some_and(|f| self.collapsed.contains(&f.path))
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
// ponytail: O(n·m) (`diff::lcs`), capped as `word_diff` is (beyond it: all old out, all new in).
fn line_diff(old: &[String], new: &[String]) -> Vec<(Kind, String)> {
    let (n, m) = (old.len(), new.len());
    let (ko, kn) = match n * m > 250_000 {
        true => (vec![false; n], vec![false; m]),
        false => diff::lcs(old, new, |a, b| a == b),
    };
    // Kept lines in step; between them, what goes before what comes instead.
    let (mut i, mut j, mut out) = (0, 0, vec![]);
    while i < n || j < m {
        if i < n && !ko[i] {
            out.push((Kind::Del, old[i].clone()));
            i += 1;
        } else if j < m && !kn[j] {
            out.push((Kind::Add, new[j].clone()));
            j += 1;
        } else {
            out.push((Kind::Ctx, old[i].clone()));
            (i, j) = (i + 1, j + 1);
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
