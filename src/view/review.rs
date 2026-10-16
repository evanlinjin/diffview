//! GitHub review actions: comments, replies, suggestions, edits, deletes, reviews, resolving,
//! viewed marks, links, and the pending/threads pickers.
use super::*;

impl App {
    /// The pending review to add to, starting one on the PR head if there isn't one yet.
    pub(super) fn ensure_review(&mut self) -> Result<String> {
        if let Some(r) = &self.review {
            return Ok(r.clone());
        }
        let pr = self.pr.as_ref().context("no PR")?;
        let id = gh::start_review(&pr.node_id, &pr.head_oid)?;
        self.review = Some(id.clone());
        Ok(id)
    }

    pub(super) fn run_job(&mut self, job: Job) {
        // Read-only: no reload after.
        if let Job::Preview(url) = job {
            match gh::issue_preview(&url) {
                Ok((title, md)) => (self.msg.clear(), self.popup = Some((title, self.hover_lines(&md), 0))).1,
                Err(e) => self.msg = format!("{e:#}"),
            }
            return;
        }
        let mut viewed = None;
        let res = match job {
            Job::Refresh => Ok("refreshed"),
            Job::Resolve(t, on) => gh::set_resolved(&t, on).map(|_| if on { "thread resolved" } else { "thread unresolved" }),
            Job::Discard(r) => gh::delete_review(&r).map(|_| {
                self.review = None;
                "pending review discarded"
            }),
            Job::DeleteComment(c) => gh::delete_comment(&c).map(|_| "comment deleted"),
            Job::Preview(_) => unreachable!("handled above"),
            Job::Viewed(path, on) => {
                let pr_id = self.pr.as_ref().map(|p| p.node_id.clone()).unwrap_or_default();
                let res = gh::set_viewed(&pr_id, &path, on);
                if res.is_ok() && !on {
                    self.collapsed.remove(&path);
                }
                viewed = Some((path, on));
                res.map(|_| if on { "marked as viewed" } else { "unmarked as viewed" })
            }
        };
        self.hl_cache.retain(|k, _| !k.1);
        let reload = self.reload(true);
        // Viewed: to the file's header, or when marked, on to the next file (like on GitHub).
        if let Some((path, on)) = viewed
            && let Some(fi) = self.files.iter().position(|f| f.path == path)
            && let Some(own) = self.rows.iter().position(|r| r.file == fi && r.line.kind == Kind::File)
        {
            let later = self.rows[own + 1..].iter().position(|r| r.line.kind == Kind::File).map(|i| own + 1 + i);
            self.select = false;
            self.goto(if on { later.unwrap_or(own) } else { own }, 0);
        }
        self.msg = match (res, reload) {
            (Err(e), _) | (_, Err(e)) => format!("{e:#}"),
            (Ok(m), Ok(())) => m.into(),
        };
    }

    /// Viewed files fold, like on GitHub: on first load, and when newly marked. Files you
    /// unfold yourself stay unfolded across refreshes.
    pub(super) fn apply_viewed(&mut self, viewed: HashMap<String, String>) {
        for (path, state) in &viewed {
            if state == "VIEWED" && self.viewed.get(path).is_none_or(|old| old != "VIEWED") {
                self.collapsed.insert(path.clone());
            }
        }
        self.viewed = viewed;
    }

    /// `Space v`: toggle GitHub's "Viewed" mark for the file under the cursor.
    pub(super) fn toggle_viewed(&mut self) {
        let Some(path) = self.file_path().map(str::to_owned) else {
            return self.msg = "not in a file".into();
        };
        if self.pr.is_none() {
            return self.msg = "no PR for this branch".into();
        }
        let on = self.viewed.get(&path).is_none_or(|s| s != "VIEWED");
        self.msg = if on { "marking as viewed…" } else { "unmarking…" }.into();
        self.job = Some(Job::Viewed(path, on));
    }

    pub(super) fn pending_count(&self) -> usize {
        self.threads.iter().flat_map(|t| &t.comments).filter(|c| c.pending).count()
    }

    pub(super) fn review_menu(&mut self) {
        if self.pr.is_none() {
            self.msg = "no PR for this branch".into();
            return;
        }
        let item = |label: &str, target| Item { label: label.into(), target };
        let mut items = vec![
            item("Comment", Target::Submit("COMMENT")),
            item("Approve", Target::Submit("APPROVE")),
            item("Request changes", Target::Submit("REQUEST_CHANGES")),
        ];
        if self.review.is_some() {
            let n = self.pending_count();
            items.push(item(&format!("Discard pending review ({n} comment(s))"), Target::Discard));
        }
        self.picker = Some(Picker { title: " submit review ", items, query: String::new(), sel: 0 });
    }

    /// `Space d`: delete the comment under the cursor, after a confirmation picker.
    pub(super) fn start_delete(&mut self) {
        if let Some((_, k)) = self.claude_at_cursor() {
            return self.edit_claude(&k, crate::claude::dismiss, "dismissed Claude's comment");
        }
        let Some(c) = self.rows[self.head.row].thread.and_then(|t| self.comment_under_cursor(t)) else {
            self.msg = "move onto a comment to delete it".into();
            return;
        };
        if !c.can_delete {
            self.msg = format!("you can't delete @{}'s comment", c.author);
            return;
        }
        let preview: String = c.body.lines().next().unwrap_or("").chars().take(40).collect();
        let items = vec![
            Item { label: "No, keep it".into(), target: Target::Nothing },
            Item { label: format!("Yes, delete @{}'s comment: \"{preview}\"", c.author), target: Target::DeleteComment(c.node_id.clone()) },
        ];
        self.picker = Some(Picker { title: " delete comment? ", items, query: String::new(), sel: 0 });
    }

    pub(super) fn start_edit(&mut self) {
        if self.claude_at_cursor().is_some() {
            return self.msg = "c copies Claude's comment into your review, to edit there; r replies to Claude".into();
        }
        let Some(c) = self.rows[self.head.row].thread.and_then(|t| self.comment_under_cursor(t)) else {
            self.msg = "move onto a comment to edit it".into();
            return;
        };
        match c.can_edit {
            true => {
                let (comment, body, author) = (c.node_id.clone(), c.body.clone(), c.author.clone());
                self.action = Some(Action::Edit { comment, body, author });
            }
            false => self.msg = format!("you can't edit @{}'s comment", c.author),
        }
    }

    pub(super) fn toggle_resolved(&mut self) {
        if let Some((t, k)) = self.claude_at_cursor() {
            return match self.threads[t].resolved {
                true => self.edit_claude(&k, |p, k| crate::claude::set_resolved(p, k, false), "unresolved (Claude's comment)"),
                false => self.edit_claude(&k, |p, k| crate::claude::set_resolved(p, k, true), "resolved (Claude's comment)"),
            };
        }
        match self.rows[self.head.row].thread.map(|t| &self.threads[t]) {
            Some(t) => {
                self.msg = if t.resolved { "unresolving…" } else { "resolving…" }.into();
                self.job = Some(Job::Resolve(t.id.clone(), !t.resolved));
            }
            None => self.msg = "move onto a comment thread".into(),
        }
    }

    /// Fetch review threads and PR info (in parallel: each is a ~1s GitHub round-trip).
    pub(super) fn load_threads(&mut self) {
        self.load_github();
        self.load_claude();
    }

    /// Claude's review file, replacing its threads (GitHub's stay).
    pub(super) fn load_claude(&mut self) {
        self.threads.retain(|t| t.claude.is_none());
        self.claude_summary.clear();
        let Some(p) = &self.claude_path else { return };
        let loaded = crate::claude::load_with_ids(p);
        self.claude_mtime = std::fs::metadata(p).and_then(|m| m.modified()).ok(); // after any id write
        match loaded {
            Ok(Some(mut r)) => {
                // Comments with their code recorded follow it; others can only trust their lines.
                let head = diff::git(&["rev-parse", "HEAD"]).unwrap_or_default();
                let moved = !r.head.is_empty() && !head.trim().starts_with(&r.head) && !r.head.starts_with(head.trim());
                if moved && r.threads.iter().any(|t| t.hunk.is_empty()) {
                    self.msg = format!(
                        "Claude reviewed {}; HEAD has moved since, so its line numbers may be off (`dv review-check` re-anchors)",
                        &r.head[..r.head.len().min(8)]
                    );
                }
                crate::claude::reanchor(&mut r.threads, &self.files);
                self.claude_summary = r.summary;
                self.threads.extend(r.threads);
            }
            Ok(None) => {}
            Err(e) => self.msg = format!("Claude's review: {e:#}"),
        }
    }

    /// Reload Claude's review when its file changes (Claude writing it, or answering replies).
    pub(super) fn watch_claude(&mut self) {
        let Some(p) = &self.claude_path else { return };
        let mtime = std::fs::metadata(p).and_then(|m| m.modified()).ok();
        if mtime != self.claude_mtime {
            let keep = self.cursor_key();
            self.msg.clear(); // a warning from loading takes its place
            self.load_claude();
            self.rebuild(keep);
            if self.msg.is_empty() {
                let n = self.threads.iter().filter(|t| t.claude.is_some()).count();
                self.msg = format!("Claude's review updated: {n} comment(s)");
            }
        }
    }

    /// Run a change to Claude's review file on comment `i`, then show the result.
    pub(super) fn edit_claude(&mut self, key: &str, f: fn(&std::path::Path, &str) -> Result<()>, done: &str) {
        let Some(p) = self.claude_path.clone() else { return };
        let (keep, res) = (self.cursor_key(), f(&p, key));
        self.load_claude();
        self.rebuild(keep);
        self.msg = match res {
            Ok(()) => done.into(),
            Err(e) => format!("{e:#}"),
        };
    }

    /// The thread under the cursor and its id in Claude's review file, if it's from there.
    pub(super) fn claude_at_cursor(&self) -> Option<(usize, String)> {
        let t = self.rows[self.head.row].thread?;
        Some((t, self.threads[t].claude.clone()?))
    }

    /// `c`/`C` on Claude's comment: copy it (edited in `$EDITOR`) into your pending review, or
    /// post it now. Claude's copy is dismissed once it's on GitHub.
    pub(super) fn start_promote(&mut self, t: usize, key: String, review: bool) {
        let th = &self.threads[t];
        if th.outdated {
            return self.msg = "this comment is outdated (its code changed): ask Claude to update it, or Space d".into();
        }
        let anchor = Anchor { path: th.path.clone(), line: th.line.unwrap_or(0), left: th.left, start: th.start_line.map(|s| (s, th.left)) };
        let body = self.comment_under_cursor(t).map(|c| c.body.clone()).unwrap_or_default();
        match self.check_synced() {
            Ok(()) => self.action = Some(Action::Comment { anchor, review, prefill: body, claude: Some(key) }),
            Err(e) => self.msg = format!("{e:#}"),
        }
    }

    fn load_github(&mut self) {
        let Some(n) = self.pr.as_ref().map(|p| p.number) else { return };
        let (threads, info) = std::thread::scope(|s| {
            let info = s.spawn(|| gh::info(n));
            (gh::threads(n), info.join())
        });
        match threads {
            Ok(r) => {
                self.apply_viewed(r.viewed);
                (self.threads, self.review) = (r.threads, r.pending);
                if let Some(pr) = &mut self.pr {
                    pr.node_id = r.pr_id;
                }
            }
            Err(e) => self.msg = format!("loading comments: {e:#}"),
        }
        match info {
            Ok(Ok((lines, title, head, url))) => {
                self.info = lines;
                if let Some(pr) = &mut self.pr {
                    (pr.title, pr.head_oid, pr.url) = (title, head, url);
                }
            }
            Ok(Err(e)) => self.msg = format!("loading PR info: {e:#}"),
            Err(_) => self.msg = "loading PR info panicked".into(),
        }
    }

    /// The link under the cursor, if any.
    pub(super) fn link_at_cursor(&self) -> Option<&str> {
        let col = self.head.col;
        self.rows[self.head.row].links.iter().find(|(a, b, _)| *a <= col && col < *b).map(|l| l.2.as_str())
    }

    pub(super) fn open_link(&mut self) {
        let Some(url) = self.link_at_cursor().map(str::to_owned) else {
            return self.msg = "no link under the cursor".into();
        };
        let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
        let spawned = std::process::Command::new(opener)
            .arg(&url)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        self.msg = match spawned {
            Ok(_) => format!("opened {url}"),
            Err(e) => format!("{opener}: {e}"),
        };
    }

    /// `Space p`: a picker of the comments in your pending review.
    /// `Space t`: a picker of every review thread, with its state, author, gist and replies.
    pub(super) fn threads_picker(&mut self) {
        let items: Vec<Item> = self
            .threads
            .iter()
            .enumerate()
            .filter_map(|(ti, t)| {
                let c = t.comments.first()?;
                let line = t.line.or(t.original_line).map_or(String::new(), |n| format!(":{n}"));
                let tags: String = [
                    (t.comments.iter().any(|c| c.pending), "[pending] "),
                    (t.resolved, "[resolved] "),
                    (t.outdated, "[outdated] "),
                ]
                .iter()
                .filter_map(|&(on, s)| on.then_some(s))
                .collect();
                let replies = match t.comments.len() - 1 {
                    0 => String::new(),
                    1 => "  · 1 reply".into(),
                    n => format!("  · {n} replies"),
                };
                let label = format!("{}{line}  {tags}@{}: {}{replies}", t.path, c.author, gist(&c.body));
                Some(Item { label, target: Target::Comment(ti, 0) })
            })
            .collect();
        match items.is_empty() {
            true => self.msg = "no review threads".into(),
            false => self.picker = Some(Picker { title: " threads ", items, query: String::new(), sel: 0 }),
        }
    }

    pub(super) fn pending_picker(&mut self) {
        let mut items = vec![];
        for (ti, t) in self.threads.iter().enumerate() {
            for (ci, c) in t.comments.iter().enumerate().filter(|(_, c)| c.pending) {
                let line = t.line.or(t.original_line).map_or(String::new(), |n| format!(":{n}"));
                let kind = match (ci, c.body.contains("```suggestion")) {
                    (_, true) => "suggestion",
                    (0, _) => "comment",
                    _ => "reply",
                };
                items.push(Item { label: format!("{}{line}  {kind}: {}", t.path, gist(&c.body)), target: Target::Comment(ti, ci) });
            }
        }
        match items.is_empty() {
            true => self.msg = "no pending comments".into(),
            false => self.picker = Some(Picker { title: " pending comments ", items, query: String::new(), sel: 0 }),
        }
    }

    /// Jump to comment `ci` of thread `ti`, unfolding its file and thread if needed.
    pub(super) fn goto_comment(&mut self, ti: usize, ci: usize) {
        let Some(t) = self.threads.get(ti) else { return };
        if !self.files.iter().any(|f| f.path == t.path) {
            return self.msg = format!("{} isn't in this diff", t.path);
        }
        self.collapsed.remove(&t.path.clone());
        if !self.thread_open(ti)
            && let Some(id) = self.root_id(ti)
        {
            flip(&mut self.toggled, id);
        }
        self.build();
        // Each comment starts with an author row; outdated threads' code rows aren't author rows.
        let row = self.rows.iter().enumerate().filter(|(_, r)| r.thread == Some(ti) && r.line.kind == Kind::Thread).nth(ci);
        if let Some((r, _)) = row {
            self.jump(r, 0);
        }
    }

    /// GitHub anchor for the selected code lines (comment rows inside the selection are ignored).
    pub(super) fn selection_anchor(&self) -> Result<Anchor> {
        let (s, e) = self.ordered();
        let code: Vec<usize> = (s.row..=e.row)
            .filter(|&r| matches!(self.rows[r].line.kind, Kind::Ctx | Kind::Add | Kind::Del))
            .collect();
        let (Some(&a), Some(&b)) = (code.first(), code.last()) else { bail!("select code lines to comment on") };
        if self.rows[a].file != self.rows[b].file {
            bail!("a comment can't span files");
        }
        let side = |r: usize| {
            let l = &self.rows[r].line;
            if l.kind == Kind::Del { (l.old, true) } else { (l.new, false) }
        };
        let ((Some(line), left), (Some(sl), sleft)) = (side(b), side(a)) else { bail!("no line number") };
        Ok(Anchor {
            path: self.files[self.rows[b].file].path.clone(),
            line,
            left,
            start: (a != b).then_some((sl, sleft)),
        })
    }

    /// `c`/`C`: reply when on a thread, else comment on the selected lines. In the PR section
    /// `C` quotes into a new conversation comment; `c` has nothing to queue there (yet).
    pub(super) fn comment_or_reply(&mut self, review: bool) {
        if let Some((t, k)) = self.claude_at_cursor() {
            return self.start_promote(t, k, review);
        }
        let row = &self.rows[self.head.row];
        match (row.file == INFO, row.thread) {
            (true, _) if review => self.msg = "select code lines to comment on".into(),
            (true, _) => self.start_conversation_comment(),
            (false, Some(_)) => self.start_reply(review),
            (false, None) => self.start_comment(review),
        }
    }

    /// Line comments are posted against the PR head; local edits would shift the lines.
    pub(super) fn check_synced(&self) -> Result<()> {
        let pr = self.pr.as_ref().context("no PR for this branch")?;
        if diff::git(&["rev-parse", "HEAD"])?.trim() != pr.head_oid {
            bail!("HEAD isn't the PR head (pull, or `gh pr checkout {}`)", pr.number);
        }
        if !diff::git(&["status", "--porcelain", "--untracked-files=no"])?.trim().is_empty() {
            bail!("working tree has local changes; stash them to comment");
        }
        Ok(())
    }

    pub(super) fn start_comment(&mut self, review: bool) {
        match self.check_synced().and_then(|_| self.selection_anchor()) {
            Ok(anchor) => self.action = Some(Action::Comment { anchor, review, prefill: String::new(), claude: None }),
            Err(e) => self.msg = format!("{e:#}"),
        }
    }

    /// `s`: a GitHub suggestion for the selected new-side lines, pre-filled with their code.
    pub(super) fn start_suggest(&mut self) {
        match self.check_synced().and_then(|_| self.suggestion()) {
            Ok((anchor, prefill)) => self.action = Some(Action::Comment { anchor, review: true, prefill, claude: None }),
            Err(e) => self.msg = format!("{e:#}"),
        }
    }

    pub(super) fn suggestion(&self) -> Result<(Anchor, String)> {
        let (s, e) = self.ordered();
        let rows: Vec<&Row> =
            self.rows[s.row..=e.row].iter().filter(|r| matches!(r.line.kind, Kind::Ctx | Kind::Add | Kind::Del)).collect();
        let (Some(first), Some(last)) = (rows.first(), rows.last()) else { bail!("select the lines to suggest a change for") };
        if rows.iter().any(|r| r.line.kind == Kind::Del) {
            bail!("suggestions replace lines in the new version; leave removed lines out of the selection");
        }
        if first.file != last.file {
            bail!("a suggestion can't span files");
        }
        let (Some(a), Some(b)) = (first.line.new, last.line.new) else { bail!("no line number") };
        let code: Vec<&str> = rows.iter().map(|r| r.line.text.as_str()).collect();
        let code = code.join("\n");
        // A fence longer than any backtick run in the code, so the block can't end early.
        let mut fence = String::from("```");
        while code.contains(&fence) {
            fence.push('`');
        }
        let anchor =
            Anchor { path: self.files[first.file].path.clone(), line: b, left: false, start: (a != b).then_some((a, false)) };
        Ok((anchor, format!("\n{fence}suggestion\n{code}\n{fence}\n")))
    }

    /// `C` in the PR section: a new PR conversation comment quoting the comment (or description)
    /// under the cursor, like GitHub's "Quote reply"; on other PR info lines, an empty one.
    pub(super) fn start_conversation_comment(&mut self) {
        if self.pr.is_none() {
            return self.msg = "no PR for this branch".into();
        }
        let row = &self.rows[self.head.row];
        let quote = row.info.and_then(|i| self.info.get(i)).map_or(String::new(), |l| quote(&l.text));
        self.action = Some(Action::Converse { quote });
    }

    pub(super) fn start_reply(&mut self, review: bool) {
        if let Some((_, k)) = self.claude_at_cursor() {
            return self.action = Some(Action::ClaudeReply(k));
        }
        let t = self.rows[self.head.row].thread;
        match (t, t.and_then(|t| self.root_id(t))) {
            (Some(t), Some(comment)) if self.pr.is_some() => {
                let thread = self.threads[t].id.clone();
                let quote = self.comment_under_cursor(t).map_or(String::new(), |c| quote(&c.body));
                self.action = Some(Action::Reply { thread, comment, review, quote });
            }
            _ => self.msg = "move onto a comment thread to reply".into(),
        }
    }

    /// Which comment of thread `t` the cursor is in: each comment starts with an author row.
    pub(super) fn comment_under_cursor(&self, t: usize) -> Option<&gh::Comment> {
        let start = self.rows[..=self.head.row].iter().rposition(|r| r.thread != Some(t)).map_or(0, |i| i + 1);
        let heads = self.rows[start..=self.head.row].iter().filter(|r| r.line.kind == Kind::Thread).count();
        self.threads[t].comments.get(heads.saturating_sub(1))
    }
}
