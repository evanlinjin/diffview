//! Getting about the review's threads: the threads and pending pickers, `]c` and the like,
//! and which comment of a thread the cursor is in.
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
            items.push(Item::new(none, Target::Nothing));
        }
        if tab == THREADS_PENDING && !matches!(items[0].target, Target::Nothing) {
            self.msg = "Enter jumps there · Ctrl-d deletes it".into();
        }
        self.picker = Some(Picker { title: THREAD_TABS[tab], items, query, sel: 0 });
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
            'a' => (each(&agent::claude_wrote), "no comments from Claude (Space a asks for a review)"),
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
        self.build();
        // Each comment starts with an author row; outdated threads' code rows aren't author rows.
        let row = self.rows.iter().enumerate().filter(|(_, r)| r.thread == Some(ti) && r.line.kind == Kind::Thread).nth(ci);
        match row {
            Some((r, _)) => self.jump(r, 0),
            None => self.msg = format!("{} isn't in this diff", self.threads[ti].path), // showing only what isn't committed
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
}
