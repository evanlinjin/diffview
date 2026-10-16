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
        if let Job::Pull = job {
            let res = self.pull();
            self.hl_cache.retain(|k, _| !k.1); // the working tree changed
            let reload = self.reload(true);
            self.msg = match (res, reload) {
                (Err(e), _) | (_, Err(e)) => format!("{e:#}"),
                (Ok(m), Ok(())) => m,
            };
            return;
        }
        if let Job::Push = job {
            let res = self.push();
            let reload = self.reload(true); // the PR's head moved
            self.msg = match (res, reload) {
                (Err(e), _) | (_, Err(e)) => format!("{e:#}"),
                (Ok(m), Ok(())) => m,
            };
            return;
        }
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
            Job::Preview(_) | Job::Pull | Job::Push => unreachable!("handled above"),
            Job::Post(p) => self.post(p),
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

    /// Queue `p` to send after the next frame, which shows what's being sent.
    pub(super) fn post_later(&mut self, p: Post) -> Result<String> {
        let msg = match &p {
            Post::Thread { review: true, .. } | Post::Reply { review: true, .. } => "adding to your pending review…",
            Post::Thread { .. } | Post::Reply { .. } | Post::Converse(_) => "posting…",
            Post::Edit { .. } => "updating the comment…",
            Post::Submit(..) => "submitting the review…",
        };
        self.job = Some(Job::Post(p));
        Ok(msg.into())
    }

    /// Send `p`; if that fails, its text is saved to a file so it isn't lost.
    fn post(&mut self, p: Post) -> Result<&'static str> {
        let pr = self.pr.clone().context("no PR")?;
        match p {
            Post::Thread { anchor, review, body, claude } => {
                let res = match review {
                    true => self.ensure_review().and_then(|r| gh::add_thread(&r, &anchor, &body)),
                    false => gh::comment(&pr, &anchor, &body),
                };
                keep_on_error(res, &body)?;
                // It's on GitHub now; drop Claude's copy so it doesn't show twice.
                if let (Some(k), Some(p)) = (claude, &self.claude_path) {
                    crate::claude::dismiss(p, &k)?;
                }
                Ok(posted_msg(review))
            }
            Post::Reply { thread, comment, review, body } => {
                let res = match review {
                    true => self.ensure_review().and_then(|r| gh::add_reply(&r, &thread, &body)),
                    false => gh::reply(&pr, comment, &body),
                };
                keep_on_error(res, &body).map(|_| posted_msg(review))
            }
            Post::Converse(body) => {
                keep_on_error(gh::conversation_comment(&pr, &body), &body).map(|_| "comment posted to the conversation")
            }
            Post::Edit { comment, body } => keep_on_error(gh::edit_comment(&comment, &body), &body).map(|_| "comment updated"),
            Post::Submit(event, body) => {
                let res = match &self.review {
                    Some(r) => gh::submit_review(r, event, &body),
                    None => gh::review_now(&pr.node_id, &pr.head_oid, event, &body),
                };
                keep_on_error(res, &body)?;
                self.review = None;
                Ok("review submitted")
            }
        }
    }

    /// `Space u`: on a PR, up to its head as `gh pr checkout` does (fast-forward only: a branch
    /// with commits that aren't on the PR is left as it is, and said so); else `git pull
    /// --ff-only`. Local changes in the way are git's to refuse, so nothing is overwritten.
    fn pull(&self) -> Result<String> {
        match &self.pr {
            Some(p) => {
                let detached = diff::git(&["branch", "--show-current"])?.trim().is_empty();
                let note = crate::checkout_or_keep(p.number, false, detached)?;
                Ok(note.unwrap_or_else(|| format!("up to date with PR #{}", p.number)))
            }
            None => {
                diff::git(&["pull", "--ff-only"])?;
                Ok("pulled".into())
            }
        }
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
        if self.files.iter().any(|f| f.path == path && f.meta == "untracked") {
            return self.msg = format!("{path} is untracked: not on GitHub, so it can't be marked viewed there");
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
        let item = |label: &str, target| Item::new(label, target);
        let mut items = vec![
            item("Comment", Target::Submit("COMMENT")),
            item("Approve", Target::Submit("APPROVE")),
            item("Request changes", Target::Submit("REQUEST_CHANGES")),
        ];
        if self.review.is_some() {
            let n = self.pending_count();
            items.push(item(&format!("Discard pending review ({n} comment(s))"), Target::Discard));
        }
        // What's still open, to see before you say you're done (approving most of all).
        items.extend(self.submit_notes().iter().map(|n| item(&format!("  ! {n}"), Target::Nothing)));
        self.picker = Some(Picker { title: " submit review ", items, query: String::new(), sel: 0 });
    }

    /// What's still open in the review: threads waiting on someone, pending comments whose
    /// code has changed, and a checkout that isn't the PR's head.
    pub(super) fn submit_notes(&self) -> Vec<String> {
        let count = |f: &dyn Fn(&Thread) -> bool| self.threads.iter().filter(|t| f(t)).count();
        let open = count(&|t| t.claude.is_some() && !t.resolved && !t.outdated);
        let answered = count(&|t| agent::turn(t) == Some("[answered] "));
        let github = count(&|t| t.claude.is_none() && !t.resolved && !t.outdated && !t.comments.iter().all(|c| c.pending));
        let stale = count(&|t| t.outdated && t.comments.iter().any(|c| c.pending));
        let head = diff::git(&["rev-parse", "HEAD"]).unwrap_or_default();
        let behind = self.pr.as_ref().is_some_and(|p| !p.head_oid.is_empty() && p.head_oid != head.trim());
        [
            (open > 0, format!("{open} open comment(s) in Claude's review ({answered} answered, your turn)")),
            (github > 0, format!("{github} unresolved thread(s) on GitHub")),
            (stale > 0, format!("{stale} of your pending comments are on code that has changed since")),
            (behind, "your checkout isn't the PR's head: what you reviewed isn't what you'd approve".into()),
        ]
        .into_iter()
        .filter_map(|(on, s)| on.then_some(s))
        .collect()
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
        self.confirm_delete(c.clone());
    }

    /// The "delete this comment?" picker, "No" preselected.
    pub(super) fn confirm_delete(&mut self, c: gh::Comment) {
        if !c.can_delete {
            self.msg = format!("you can't delete @{}'s comment", c.author);
            return;
        }
        let preview: String = c.body.lines().next().unwrap_or("").chars().take(40).collect();
        let items = vec![
            Item::new("No, keep it", Target::Nothing),
            Item::new(format!("Yes, delete @{}'s comment: \"{preview}\"", c.author), Target::DeleteComment(c.node_id.clone())),
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
        self.claude_head.clear();
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
                // Not against HEAD: files without uncommitted changes aren't in that diff, and
                // their comments would look outdated (and could be dismissed as such).
                if !self.uncommitted() {
                    crate::claude::reanchor(&mut r.threads, &self.files);
                }
                (self.claude_summary, self.claude_head) = (r.summary, r.head);
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
                let n = self.threads.iter().filter(|t| agent::claude_wrote(t)).count();
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
        if !agent::claude_wrote(th) {
            return self.msg = "this is your comment for Claude (private): r replies, Space d removes it".into();
        }
        if th.outdated {
            return self.msg = "this comment is outdated (its code changed): ask Claude to update it, or Space d".into();
        }
        let anchor = Anchor { path: th.path.clone(), line: th.line.unwrap_or(0), left: th.left, start: th.start_line.map(|s| (s, th.left)) };
        let mut body = self.comment_under_cursor(t).map(|c| c.body.clone()).unwrap_or_default();
        // On GitHub the severity leads the text, as reviewers write it: "**nit:** …".
        if !th.severity.is_empty() && !body.starts_with("**") && self.comment_index_under_cursor(t) == 0 {
            body = format!("**{}:** {body}", th.severity);
        }
        match self.check_synced().and_then(|_| self.on_github(anchor)) {
            Ok(anchor) => self.action = Some(Action::Comment { anchor, review, prefill: body, claude: Some(key) }),
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
                (self.theirs, self.me) = (!r.mine, r.me);
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
        self.threads_picker_of(" threads ", |_, _| true, "no review threads");
    }

    /// `Space A`: Claude's comments, `[waiting]` where your reply hasn't been answered,
    /// `[answered]` where Claude has answered you (type either to pick from just those).
    pub(super) fn claude_picker(&mut self) {
        self.threads_picker_of(" Claude's comments ", |_, t| t.claude.is_some(), "no comments from Claude (Space a asks for a review)");
    }

    fn threads_picker_of(&mut self, title: &'static str, keep: impl Fn(usize, &Thread) -> bool, none: &str) {
        let items: Vec<Item> = self
            .threads
            .iter()
            .enumerate()
            .filter(|&(ti, t)| keep(ti, t))
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
                .collect();
                // Before the text, so a long first line can't push it out of sight.
                let replies = match t.comments.len() - 1 {
                    0 => String::new(),
                    1 => "1 reply · ".into(),
                    n => format!("{n} replies · "),
                };
                // A card: where, what state, who; then the start of what was said.
                let label = format!("{}{line}  {replies}{tags}@{}", t.path, c.author);
                Some(Item { detail: self.snippet(&c.body, 2), ..Item::new(label, Target::Comment(ti, 0)) })
            })
            .collect();
        match items.is_empty() {
            true => self.msg = none.into(),
            false => self.picker = Some(Picker { title, items, query: String::new(), sel: 0 }),
        }
    }

    /// `Space n`: the threads with comments you haven't seen.
    pub(super) fn new_picker(&mut self) {
        let new: HashSet<usize> = (0..self.threads.len()).filter(|&t| self.is_new(&self.threads[t])).collect();
        self.threads_picker_of(" new comments ", |ti, _| new.contains(&ti), "nothing new: you've seen every comment");
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
                let item = Item::new(format!("{}{line}  {kind}", t.path), Target::Comment(ti, ci));
                items.push(Item { detail: self.snippet(&c.body, 2), ..item });
            }
        }
        match items.is_empty() {
            true => self.msg = "no pending comments".into(),
            false => self.picker = Some(Picker { title: " pending comments ", items, query: String::new(), sel: 0 }),
        }
        if self.picker.is_some() {
            self.msg = "Enter jumps there · Ctrl-d deletes it".into();
        }
    }

    /// `]a` / `[a`: the next / previous Claude comment.
    pub(super) fn next_claude(&mut self, fwd: bool) {
        let items = (0..self.threads.len()).filter(|&t| agent::claude_wrote(&self.threads[t])).map(|t| (t, 0)).collect();
        self.next_comment(fwd, items, "no comments from Claude (Space a asks for a review)");
    }

    /// `]n` / `[n`: the next / previous thread with comments you haven't seen.
    pub(super) fn next_new(&mut self, fwd: bool) {
        let items = (0..self.threads.len()).filter(|&t| self.is_new(&self.threads[t])).map(|t| (t, 0)).collect();
        self.next_comment(fwd, items, "nothing new: you've seen every comment");
    }

    /// `]p` / `[p`: the next / previous comment of your pending review (replies too).
    pub(super) fn next_pending(&mut self, fwd: bool) {
        let threads = &self.threads;
        let items = (0..threads.len())
            .flat_map(|t| (0..threads[t].comments.len()).filter(move |&c| threads[t].comments[c].pending).map(move |c| (t, c)))
            .collect();
        self.next_comment(fwd, items, "no pending comments");
    }

    /// Go to the next / previous of `items` (thread, comment) after the cursor, by file and
    /// line, wrapping around; folded files and threads open on the way (`goto_comment`).
    fn next_comment(&mut self, fwd: bool, items: Vec<(usize, usize)>, none: &str) {
        // Threads on files no longer in the diff are listed above the files, so they come first.
        let file = |path: &str| self.files.iter().position(|f| f.path == path).map_or(Some(0), |i| Some(i + 1));
        // (file, line, thread, comment); a code line counts as thread -1, before the threads
        // shown under it.
        let key = |t: usize, c: usize| -> Option<(usize, u32, i64, usize)> {
            let th = &self.threads[t];
            Some((file(&th.path)?, th.line.or(th.original_line).unwrap_or(0), t as i64, c))
        };
        let mut order: Vec<_> = items.into_iter().filter(|&(t, _)| self.in_view(&self.threads[t])).filter_map(|(t, c)| key(t, c)).collect();
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
            self.claude_view = false; // it's not Claude's: back to the whole diff, to show it
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
        // GitHub allows one pending review per PR and refuses single line comments beside it.
        let row = &self.rows[self.head.row];
        // PR text (not a thread there: those are on files no longer in the diff).
        let pr_text = row.file == INFO && row.thread.is_none();
        if !review && self.review.is_some() && !pr_text {
            let n = self.pending_count();
            return self.msg = format!("you have a pending review ({n}): c adds to it, S submits it, then C posts");
        }
        if let Some((t, k)) = self.claude_at_cursor() {
            return self.start_promote(t, k, review);
        }
        match (pr_text, self.rows[self.head.row].thread) {
            (true, _) if review => self.msg = "select code lines to comment on".into(),
            (true, _) => self.start_conversation_comment(),
            // `r` replies (in the review); `C` here replies right away, as `C` posts what `c` queues.
            (false, Some(_)) if review => self.msg = "r replies to this thread (c comments on selected code lines)".into(),
            (false, Some(_)) => self.start_reply(false),
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

    /// GitHub only takes comments on files the PR has: not on one that's untracked here.
    pub(super) fn on_github(&self, a: Anchor) -> Result<Anchor> {
        match self.files.iter().any(|f| f.path == a.path && f.meta == "untracked") {
            true => bail!("{} is untracked: GitHub hasn't got it (commit and push it: Space g), or `a` asks Claude", a.path),
            false => Ok(a),
        }
    }

    pub(super) fn start_comment(&mut self, review: bool) {
        match self.check_synced().and_then(|_| self.selection_anchor()).and_then(|a| self.on_github(a)) {
            Ok(anchor) => self.action = Some(Action::Comment { anchor, review, prefill: String::new(), claude: None }),
            Err(e) => self.msg = format!("{e:#}"),
        }
    }

    /// `s`: a GitHub suggestion for the selected new-side lines, pre-filled with their code.
    pub(super) fn start_suggest(&mut self) {
        match self.check_synced().and_then(|_| self.suggestion()).and_then(|(a, p)| Ok((self.on_github(a)?, p))) {
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
        let text = match row.info {
            Some(SUMMARY) => Some(self.claude_summary.as_str()),
            i => i.and_then(|i| self.info.get(i)).map(|l| l.text.as_str()),
        };
        let quote = text.map_or(String::new(), quote);
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
        self.threads[t].comments.get(self.comment_index_under_cursor(t))
    }

    /// Which comment of thread `t` the cursor is in, by index.
    fn comment_index_under_cursor(&self, t: usize) -> usize {
        let start = self.rows[..=self.head.row].iter().rposition(|r| r.thread != Some(t)).map_or(0, |i| i + 1);
        let heads = self.rows[start..=self.head.row].iter().filter(|r| r.line.kind == Kind::Thread).count();
        heads.saturating_sub(1)
    }
}

/// A comment's key in what you've seen: its GitHub id, or its place in Claude's review file.
fn comment_key(t: &Thread, ci: usize) -> String {
    match &t.claude {
        Some(id) => format!("claude:{id}:{ci}"),
        None => t.comments[ci].node_id.clone(),
    }
}

impl App {
    /// What you've seen of this PR's (or branch's) threads, in dv's directory: `seen/` beside
    /// `reviews/`, by the same name.
    pub(super) fn load_seen(&mut self) {
        self.seen_path = self.claude_path.as_ref().and_then(|p| Some(p.parent()?.parent()?.join("seen").join(p.file_name()?)));
        let text = self.seen_path.as_ref().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
        self.seen = serde_json::from_str(&text).unwrap_or_default();
    }

    /// Thread `t` has comments you haven't seen, not counting your own. (Never, with nowhere
    /// to keep what you've seen.)
    pub(super) fn is_new(&self, t: &Thread) -> bool {
        self.seen_path.is_some()
            && t.comments.iter().enumerate().any(|(ci, c)| !self.wrote(c) && !self.seen.contains(&comment_key(t, ci)))
    }

    fn wrote(&self, c: &gh::Comment) -> bool {
        c.pending || c.author == "you" || (!self.me.is_empty() && c.author == self.me)
    }

    /// Once the cursor leaves a thread, its comments (as they were when it got there) are seen.
    pub(super) fn watch_seen(&mut self) {
        if self.select {
            return; // the rebuild would drop the selection: later
        }
        let here = self.rows.get(self.head.row).and_then(|r| r.thread).map(|ti| {
            let t = &self.threads[ti];
            (0..t.comments.len()).map(|ci| comment_key(t, ci)).collect::<Vec<_>>()
        });
        let here = here.unwrap_or_default();
        if here == self.seeing {
            return;
        }
        let left = std::mem::replace(&mut self.seeing, here);
        let before = self.seen.len();
        self.seen.extend(left.into_iter().filter(|k| !k.is_empty()));
        if self.seen.len() != before
            && let Some(p) = &self.seen_path
        {
            let mut keys: Vec<&String> = self.seen.iter().collect();
            keys.sort();
            // ponytail: rewrites the whole set; fine for a PR's worth of comments.
            _ = std::fs::create_dir_all(p.parent().unwrap_or(p)).and_then(|_| std::fs::write(p, serde_json::to_string(&keys).unwrap_or_default()));
            let keep = self.cursor_key();
            self.rebuild(keep); // without its [new] tag
        }
    }
}
