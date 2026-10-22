//! The review's threads: as rows (their boxes, folded or open), the threads and pending
//! pickers, `]c` and the like, and which comment of a thread the cursor is in.
use super::*;

/// The threads picker's titles, one per filter (Tab goes round them): its title says which.
pub(super) const THREAD_TABS: [&str; 5] = [
    " threads: [all] unresolved pending Claude new ",
    " threads: all [unresolved] pending Claude new ",
    " threads: all unresolved [pending] Claude new ",
    " threads: all unresolved pending [Claude] new ",
    " threads: all unresolved pending Claude [new] ",
];

pub(super) const THREADS_ALL: usize = 0;

pub(super) const THREADS_UNRESOLVED: usize = 1;

pub(super) const THREADS_PENDING: usize = 2;

pub(super) const THREADS_CLAUDE: usize = 3;

pub(super) const THREADS_NEW: usize = 4;

impl App {
    /// `Space t` (and `Space p` / `A` / `n` on their filter): the review threads, with their
    /// state, author, gist and replies; Tab / Shift-Tab go round the filters (`THREAD_TABS`):
    /// all, unresolved, your pending comments (each one: `Ctrl-d` deletes it), Claude's, the
    /// new ones. What's typed stays across them.
    pub(super) fn threads_picker(&mut self, tab: usize) {
        let query = self.picker.as_ref().filter(|p| THREAD_TABS.contains(&p.title)).map(|p| p.query.clone()).unwrap_or_default();
        let (mut items, none) = match tab {
            THREADS_UNRESOLVED => (self.thread_items(|t| !t.resolved), "no unresolved threads"),
            THREADS_PENDING => (self.pending_items(), "no pending comments (c queues one)"),
            THREADS_CLAUDE => (self.thread_items(|t| t.claude.is_some()), "no comments from Claude (Space a asks for a review)"),
            THREADS_NEW => (self.thread_items(|t| self.is_new(t)), "nothing new: you've seen every comment"),
            _ => (self.thread_items(|_| true), "no review threads"),
        };
        if items.is_empty() {
            items.push(Item::new(none, Target::Note("")));
        }
        if tab == THREADS_PENDING && !matches!(items[0].target, Target::Note(_)) {
            self.msg = "Enter jumps there · Ctrl-d deletes it".into();
        }
        self.picker = Some(Picker { query, ..Picker::new(THREAD_TABS[tab], items) });
    }

    /// A card per thread that `keep`s: where, what state, who; then the start of what was said.
    pub(super) fn thread_items(&self, keep: impl Fn(&Thread) -> bool) -> Vec<Item> {
        self.threads
            .iter()
            .enumerate()
            .filter(|(_, t)| keep(t))
            .filter_map(|(ti, t)| {
                let c = t.comments.first()?;
                let line = t.line.or(t.original_line).map_or(String::new(), |n| format!(":{n}"));
                let severity = (!t.severity.is_empty()).then(|| format!("[{}] ", t.severity));
                let tags: String = [
                    (self.is_new(t), "[new] "),
                    (t.comments.iter().any(|c| c.pending), "[pending] "),
                    (t.resolved, "[resolved] "),
                    (t.outdated, "[outdated] "),
                ]
                .iter()
                .filter_map(|&(on, s)| on.then_some(s))
                .chain(severity.as_deref())
                .chain(agent::turn(t))
                .chain(agent::want_tag(t))
                .collect();
                // Before the text, so a long first line can't push it out of sight.
                let replies = match t.comments.len() - 1 {
                    0 => String::new(),
                    1 => "1 reply · ".into(),
                    n => format!("{n} replies · "),
                };
                let label = format!("{}{line}  {replies}{tags}@{}", t.path, c.author);
                Some(Item { detail: self.snippet(&self.gist(c), 2), ..Item::new(label, Target::Comment(ti, 0)) })
            })
            .collect()
    }

    /// Each comment of your pending review (replies too).
    pub(super) fn pending_items(&self) -> Vec<Item> {
        let mut items = vec![];
        for (ti, t) in self.threads.iter().enumerate() {
            for (ci, c) in t.comments.iter().enumerate().filter(|(_, c)| c.pending) {
                let line = t.line.or(t.original_line).map_or(String::new(), |n| format!(":{n}"));
                let kind = match (ci, self.seen_body(c).is_some_and(|b| b.contains("```suggestion"))) {
                    (_, true) => "suggestion",
                    (0, _) => "comment",
                    _ => "reply",
                };
                let item = Item::new(format!("{}{line}  {kind}", t.path), Target::Comment(ti, ci));
                items.push(Item { detail: self.snippet(&self.gist(c), 2), ..item });
            }
        }
        items
    }

    /// `]` / `[` and `what`: to the next / previous thread (`c`, into folded files too), of
    /// Claude's (`a`), with comments you haven't seen (`n`), unresolved (`u`: GitHub's or
    /// Claude's, outdated ones too, which still want resolving), or comment of your pending
    /// review (`p`, replies too).
    pub(super) fn next_of(&mut self, what: char, fwd: bool) {
        let threads = &self.threads;
        let each = |keep: &dyn Fn(&Thread) -> bool| (0..threads.len()).filter(|&t| keep(&threads[t])).map(|t| (t, 0)).collect();
        let (items, none) = match what {
            'a' => (each(&Thread::by_claude), "no comments from Claude (Space a asks for a review)"),
            'n' => (each(&|t| self.is_new(t)), "nothing new: you've seen every comment"),
            'u' => (each(&|t| !t.resolved), "no unresolved comments"),
            'p' => {
                let pending = (0..threads.len()).flat_map(|t| (0..threads[t].comments.len()).filter(move |&c| threads[t].comments[c].pending).map(move |c| (t, c)));
                (pending.collect(), "no pending comments")
            }
            _ => (each(&|t| self.in_view(t)), "no comments"),
        };
        self.next_comment(fwd, items, none);
    }

    /// Go to the next / previous of `items` (thread, comment) after the cursor, by file and
    /// line, wrapping around; folded files and threads open on the way (`goto_comment`).
    pub(super) fn next_comment(&mut self, fwd: bool, items: Vec<(usize, usize)>, none: &str) {
        // Threads on files no longer in the diff are listed above the files, so they come first.
        let file = |path: &str| self.files.iter().position(|f| f.path == path).map_or(Some(0), |i| Some(i + 1));
        // (file, line, thread, comment); a code line counts as thread -1, before the threads
        // shown under it.
        let key = |t: usize, c: usize| -> Option<(usize, u32, i64, usize)> {
            let th = &self.threads[t];
            Some((file(&th.path)?, th.line.or(th.original_line).unwrap_or(0), t as i64, c))
        };
        // In the Claude / pending view, those in it; if none are, all (going to one leaves it).
        let any_here = items.iter().any(|&(t, _)| self.in_view(&self.threads[t]));
        let mut order: Vec<_> = items.into_iter().filter(|&(t, _)| !any_here || self.in_view(&self.threads[t])).filter_map(|(t, c)| key(t, c)).collect();
        if order.is_empty() {
            return self.msg = none.into();
        }
        order.sort();
        // Where the cursor is, in the same terms (the PR section comes before every file).
        let row = &self.rows[self.head.row];
        let here = match row.thread {
            Some(t) => key(t, self.comment_index_under_cursor(t)),
            None if row.file == INFO => None,
            None => self.files.get(row.file).map(|_| (row.file + 1, row.line.new.or(row.line.old).unwrap_or(0), -1, 0)),
        };
        let pick = match (fwd, here) {
            (true, Some(h)) => order.iter().find(|&&o| o > h).or(order.first()),
            (false, Some(h)) => order.iter().rev().find(|&&o| o < h).or(order.last()),
            (true, None) => order.first(),
            (false, None) => order.last(),
        };
        if let Some(&(_, _, t, c)) = pick {
            self.goto_comment(t as usize, c);
        }
    }

    /// Jump to comment `ci` of thread `ti`, unfolding its file and thread if needed.
    pub(super) fn goto_comment(&mut self, ti: usize, ci: usize) {
        let Some(t) = self.threads.get(ti) else { return };
        let keep = self.cursor_key();
        if !self.in_view(t) {
            self.view = View::All; // it's not in this view: back to the whole diff, to show it
        }
        let t = &self.threads[ti];
        self.collapsed.remove(&t.path.clone());
        if !self.thread_open(ti)
            && let Some(id) = self.root_id(ti)
        {
            flip(&mut self.toggled, id);
        }
        self.rebuild(keep);
        // Each comment starts with an author row; outdated threads' code rows aren't author rows.
        let row = self.rows.iter().enumerate().filter(|(_, r)| r.thread == Some(ti) && r.line.kind == Kind::Thread).nth(ci);
        match row {
            Some((r, _)) => self.jump(r, 0),
            // (Showing only what isn't committed; a question on no file, where nothing shows it.)
            None => self.msg = match self.threads[ti].path.as_str() {
                "" => "that comment isn't shown here".into(),
                p => format!("{p} isn't in this diff"),
            },
        }
    }

    /// Which comment of thread `t` the cursor is in: each comment starts with an author row.
    pub(super) fn comment_under_cursor(&self, t: usize) -> Option<&gh::Comment> {
        self.threads[t].comments.get(self.comment_index_under_cursor(t))
    }

    /// Which comment of thread `t` the cursor is in, by index.
    pub(super) fn comment_index_under_cursor(&self, t: usize) -> usize {
        let start = self.rows[..=self.head.row].iter().rposition(|r| r.thread != Some(t)).map_or(0, |i| i + 1);
        let heads = self.rows[start..=self.head.row].iter().filter(|r| r.line.kind == Kind::Thread).count();
        heads.saturating_sub(1)
    }
    pub(super) fn thread_rows(&self, ti: usize, file: usize) -> Vec<Row> {
        self.thread_rows_as(ti, file, self.thread_open(ti))
    }

    /// A thread's rows, `open` or folded to its heading.
    pub(super) fn thread_rows_as(&self, ti: usize, file: usize, open: bool) -> Vec<Row> {
        let t = &self.threads[ti];
        // Its lines: a range (from the other side, each with its side: `−` the old, `+` the
        // new); outdated, where it was.
        let side = |left: bool| if left { '−' } else { '+' };
        let range = match (t.outdated, t.range_start(), t.line) {
            // (Outdated: where it was, as GitHub has it; Claude's range is its own, one side.)
            (true, ..) => match if t.claude.is_some() { t.own_lines() } else { t.original_line.or(t.line).map(|e| (None, e)) } {
                Some((Some(s), e)) => format!(" · was L{s}-{e}"),
                Some((None, e)) => format!(" · was L{e}"),
                None => String::new(),
            },
            (_, Some((s, from)), Some(e)) if from != t.left => format!(" · L{}{s}–{}{e}", side(from), side(t.left)),
            (_, Some((s, _)), Some(e)) => format!(" · L{s}-{e}"),
            (_, None, Some(e)) => format!(" · L{e}"),
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
        // Claude's comment is what's kept; replies under it are talk about it: indented, behind
        // a dim guide, and once that's resolved, one line.
        let talk = t.claude.is_some();
        let indent = |r: Row| Row { guide: true, ..r };
        for (k, c) in t.comments.iter().enumerate() {
            let shown = self.root_id(ti).is_some_and(|id| self.talk_shown.contains(&id));
            if talk && k == 1 && t.talk_resolved && !shown {
                let n = t.comments.len() - 1;
                let text = format!("  ▸ {n} repl{} under it, resolved (Tab shows them; Space r reopens them)", if n == 1 { "y" } else { "ies" });
                out.push(Row { syn: vec![Some(self.ui.stale); text.chars().count()], talk: true, ..mk(Kind::Comment, text) });
                break;
            }
            let start = out.len();
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
            let w = self.info_w().saturating_sub(if talk && k > 0 { GUIDE.chars().count() } else { 0 });
            let rows = self.md_rows_at(&c.body, w, mk);
            match t.stale() {
                // Resolved/outdated: rendered the same, but dimmed throughout (links still open).
                true => out.extend(rows.into_iter().map(|mut r| {
                    r.syn = vec![Some(self.ui.stale); r.line.text.chars().count()];
                    r
                })),
                false => out.extend(rows),
            }
            if talk && k > 0 {
                let mine: Vec<Row> = out.drain(start..).map(indent).collect();
                out.extend(mine);
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

    /// Tab on a resolved talk under Claude's comment (its line, or its replies shown): shown, or
    /// folded again; only seen, not reopened (Space r does that). Whether it was one.
    pub(super) fn toggle_talk(&mut self) -> bool {
        let r = &self.rows[self.head.row];
        let Some(ti) = r.thread.filter(|&t| self.threads[t].talk_resolved && (r.talk || r.guide)) else { return false };
        let Some(id) = self.root_id(ti) else { return false };
        flip(&mut self.talk_shown, id);
        self.rebuild_at(|r| r.thread == Some(ti));
        true
    }

    pub(super) fn toggle_thread(&mut self) {
        let Some(ti) = self.rows[self.head.row].thread else { return };
        let Some(id) = self.root_id(ti) else { return };
        flip(&mut self.toggled, id);
        self.rebuild_at(|r| r.thread == Some(ti));
    }

    pub(super) fn root_id(&self, t: usize) -> Option<u64> {
        Some(self.threads.get(t)?.comments.first()?.id)
    }
    /// A comment's first `n` lines as they read rendered (quotes skipped), for picker cards.
    pub(super) fn snippet(&self, body: &str, n: usize) -> Vec<String> {
        // Wrapped to the picker's text width (3/4 of the window, less padding and indent).
        let w = (self.width as usize * 3 / 4).saturating_sub(6).max(20);
        let lines = crate::md::render(body, w, &self.ui.md, &self.hl, None);
        let text = lines.into_iter().map(|l| l.text.trim_end().to_owned());
        text.skip_while(|l| l.is_empty() || l.starts_with('▎')).filter(|l| !l.is_empty()).take(n).collect()
    }
}
