//! GitHub review actions: comments, replies, suggestions, edits, deletes, reviews, resolving,
//! viewed marks, links, and the pending/threads pickers.
use super::*;

impl App {
    /// The pending review to add to, starting one on the PR head if there isn't one yet (with
    /// your review text so far, as GitHub can't add any later to one started without).
    pub(super) fn ensure_review(&mut self) -> Result<String> {
        if let Some(r) = &self.review {
            return Ok(r.clone());
        }
        let pr = self.pr.as_ref().context("no PR")?;
        let id = gh::start_review(&pr.node_id, &pr.head_oid, &self.summary_draft)?;
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
        if let Job::Push(force) = job {
            let res = self.push(force);
            if res.is_ok() {
                self.github_changed();
            }
            let reload = self.reload(true); // the PR's head moved
            self.msg = match (res, reload) {
                (Err(e), _) | (_, Err(e)) => format!("{e:#}"),
                (Ok(m), Ok(())) => m,
            };
            return;
        }
        // Read-only: no reload after.
        if !matches!(job, Job::Preview(_) | Job::Reactions(_) | Job::Labels | Job::Retarget(_) | Job::Rebase) {
            self.ensure_fresh();
        }
        // Reruns: GitHub sets them going in a moment, and CI doesn't change the PR's ETag: dv
        // looks again by itself, shortly.
        if let Job::Rerun = job {
            let head = self.pr.as_ref().map(|p| p.head_oid.clone()).unwrap_or_default();
            self.msg = match gh::rerun_failed(&head) {
                Ok(said) => {
                    // What a fetch under way brings back is from before (failing still); the
                    // watch fetches again by itself, shortly (through `watch_github`).
                    self.github_changed();
                    self.refresh_at = Some(std::time::Instant::now() + std::time::Duration::from_secs(10));
                    said
                }
                Err(e) => format!("{e:#}"),
            };
            return;
        }
        if let Job::Rebase = job {
            return self.start_rebase();
        }

        if let Job::Retarget(base) = job {
            return self.retarget(base);
        }
        if let Job::Labels = job {
            return self.labels_menu();
        }
        if let Job::Reactions(node) = job {
            return self.react_menu(&node);
        }
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
            Job::SetField(field, name, on) => {
                let n = self.pr.as_ref().map_or(0, |p| p.number);
                gh::set_field(n, field, &name, on).map(|_| match (field, on) {
                    (gh::Field::Label, true) => "label added",
                    (gh::Field::Label, false) => "label removed",
                    (gh::Field::Assignee, true) => "assigned",
                    (gh::Field::Assignee, false) => "unassigned",
                    (gh::Field::Reviewer, true) => "review requested",
                    (gh::Field::Reviewer, false) => "review request withdrawn",
                })
            }
            Job::React(node, what, add) => gh::react(&node, what, add).map(|_| if add { "reacted" } else { "reaction taken back" }),
            Job::Preview(_) | Job::Reactions(_) | Job::Labels | Job::Retarget(_) | Job::Rerun | Job::Rebase | Job::Pull | Job::Push(_) => unreachable!("handled above"),
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
            Post::Summary(_) => "adding to your review summary…",
            Post::Edit { .. } => "updating the comment…",
            Post::Submit(..) => "submitting the review…",
        };
        self.job = Some(Job::Post(p));
        Ok(msg.into())
    }

    /// Send `p`; if that fails, its text is saved to a file so it isn't lost.
    pub(super) fn post(&mut self, p: Post) -> Result<&'static str> {
        let pr = self.pr.clone().context("no PR")?;
        match p {
            Post::Thread { anchor, review, body, claude } => {
                let res = match review {
                    true => self.ensure_review().and_then(|r| gh::add_thread(&r, &anchor, &body)),
                    false => gh::comment(&pr, &anchor, &body),
                };
                keep_on_error(res, &body)?;
                // It's on GitHub now; drop Claude's copy so it doesn't show twice.
                if let (Some(k), Some(p)) = (claude, &self.claude.path) {
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
            Post::Summary(body) => {
                // A new review starts with it; one started without text can't take any yet, so
                // it stays here (S submits it with the review).
                let fresh = self.review.is_none();
                let res = self.ensure_review().and_then(|r| if fresh { Ok(()) } else { gh::set_review_body(&r, &body) });
                match res {
                    Err(e) if format!("{e:#}").contains("missing body") => Ok(
                        "in your review summary till you quit (GitHub won't hold it yet): S submits it",
                    ),
                    res => keep_on_error(res, &body).map(|_| "in your review summary: S submits it (editing it first)"),
                }
            }
            Post::Edit { comment, body } => keep_on_error(gh::edit_comment(&comment, &body), &body).map(|_| "comment updated"),
            Post::Submit(event, body) => {
                // Closing comments; with nothing to say (and no pending comments) it only closes.
                let close = event == CLOSE;
                let event = if close { "COMMENT" } else { event };
                if !close || self.review.is_some() || !body.trim().is_empty() {
                    let res = match &self.review {
                        Some(r) => gh::submit_review(r, event, &body),
                        None => gh::review_now(&pr.node_id, &pr.head_oid, event, &body),
                    };
                    keep_on_error(res, &body)?;
                    self.review = None;
                }
                if !close {
                    return Ok("review submitted");
                }
                gh::close(pr.number, None).context("the review went in, but closing the PR failed")?;
                Ok("PR closed")
            }
        }
    }

    /// Viewed files fold, like on GitHub: on first load, and when newly marked; not with
    /// comments you haven't seen on them. Files you unfold yourself stay unfolded across
    /// refreshes.
    pub(super) fn apply_viewed(&mut self, viewed: HashMap<String, String>) {
        for (path, state) in &viewed {
            if state == "VIEWED" && self.viewed.get(path).is_none_or(|old| old != "VIEWED") && !self.new_on(path) {
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
        if self.pr.is_none() && self.replacement.is_some() {
            return self.replacement_menu();
        }
        if self.pr.is_none() {
            self.msg = "no PR for this branch".into();
            return;
        }
        let item = |label: &str, target| Item::new(label, target);
        let mut items = vec![
            item("Comment", Target::Submit("COMMENT")),
            item("Approve", Target::Submit("APPROVE")),
            item("Request changes", Target::Submit("REQUEST_CHANGES")),
            item("Comment and close the PR", Target::Submit(CLOSE)),
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
        let preview = match self.seen_body(&c) {
            Some(body) => format!("\"{}\"", body.lines().next().unwrap_or("").chars().take(40).collect::<String>()),
            None => format!("({})", hidden_words(&c.hidden)),
        };
        let items = vec![
            Item::new("No, keep it", Target::Nothing),
            Item::new(format!("Yes, delete @{}'s comment: {preview}", c.author), Target::DeleteComment(c.node_id.clone())),
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
            true if self.seen_body(c).is_none() => self.msg = HIDDEN_FIRST.into(),
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

    /// Claude's review file, replacing its threads (GitHub's stay).
    pub(super) fn load_claude(&mut self) {
        let had_new = self.new_paths();
        self.threads.retain(|t| t.claude.is_none());
        self.claude.summary.clear();
        self.claude.head.clear();
        self.claude.description.clear();
        self.claude.closing.clear();
        let Some(p) = &self.claude.path else { return };
        let loaded = crate::claude::load_with_ids(p);
        self.claude.mtime = std::fs::metadata(p).and_then(|m| m.modified()).ok(); // after any id write
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
                if !self.uncommitted() && self.commit_shown.is_none() {
                    crate::claude::reanchor(&mut r.threads, &self.files);
                }
                (self.claude.summary, self.claude.head, self.claude.description, self.claude.closing) = (r.summary, r.head, r.description, r.closing);
                self.threads.extend(r.threads);
                self.unfold_new(&had_new);
            }
            Ok(None) => {}
            Err(e) => self.msg = format!("Claude's review: {e:#}"),
        }
    }

    /// Reload Claude's review when its file changes (Claude writing it, or answering replies).
    pub(super) fn watch_claude(&mut self) {
        let Some(p) = &self.claude.path else { return };
        let mtime = std::fs::metadata(p).and_then(|m| m.modified()).ok();
        if mtime != self.claude.mtime {
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
        let Some(p) = self.claude.path.clone() else { return };
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
        if !th.conversation.is_empty() {
            return self.msg = "Claude's answer here is for you, on no lines: C on the comment above quotes it in a new conversation comment".into();
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

    /// The GitHub comment (or review) under the cursor, to react to: its node id. A review
    /// thread's comment, or one in the PR's conversation (its heading or text).
    pub(super) fn reactable_at_cursor(&self) -> Option<String> {
        let row = &self.rows[self.head.row];
        if let Some(t) = row.thread {
            let c = self.comment_under_cursor(t).filter(|c| self.threads[t].claude.is_none() && !c.pending)?;
            return Some(c.node_id.clone()).filter(|n| !n.is_empty());
        }
        self.conversation_node()
    }

    /// The PR conversation's comment (or review) under the cursor, or the description (the PR
    /// itself): its node id.
    pub(super) fn conversation_node(&self) -> Option<String> {
        let row = &self.rows[self.head.row];
        let i = row.info.filter(|_| row.file == INFO && row.thread.is_none() && matches!(row.main, None | Some(DESCRIPTION)))?;
        let head = match self.info.get(i)? {
            l if l.heading => l,
            l if l.md => self.info.get(i.checked_sub(1)?).filter(|h| h.heading)?,
            _ => return None,
        };
        Some(head.node.clone()).filter(|n| !n.is_empty())
    }

    /// `Space =`: react to the comment under the cursor, or take your reaction back: what's
    /// on it now is asked of GitHub first (a job, so a frame says so), so yours show.
    pub(super) fn start_react(&mut self) {
        match self.reactable_at_cursor() {
            Some(node) => (self.msg, self.job) = ("asking GitHub for its reactions…".into(), Some(Job::Reactions(node))),
            None => self.msg = "move onto a GitHub comment (or the description) to react to it".into(),
        }
    }

    /// `Space l`: the PR's labels, assignees and requested reviewers, and what it could have
    /// (✓: on it); Enter adds or removes one. Type to find one (`review`, `@` for people).
    pub(super) fn labels_menu(&mut self) {
        self.msg.clear();
        let Some(n) = self.pr.as_ref().map(|p| p.number) else { return };
        let (labels, people, reviewers) = match gh::pr_fields(n) {
            Ok(x) => x,
            Err(e) => return self.msg = format!("{e:#}"),
        };
        let item = |field: gh::Field, name: &str, on: bool| {
            let shown = match field {
                gh::Field::Label => format!("label {name}"),
                gh::Field::Assignee => format!("assignee @{name}"),
                gh::Field::Reviewer => format!("review from @{name}"),
            };
            let tick = if on { "✓ " } else { "  " };
            Item::new(format!("{tick}{shown}"), Target::SetField(field, name.to_owned(), !on))
        };
        let items: Vec<Item> = [(gh::Field::Reviewer, &reviewers), (gh::Field::Label, &labels), (gh::Field::Assignee, &people)]
            .into_iter()
            .flat_map(|(f, xs)| xs.iter().map(move |(x, on)| item(f, x, *on)))
            .collect();
        if items.is_empty() {
            return self.msg = "no labels in this repo, nor anyone to assign or ask".into();
        }
        self.picker = Some(Picker { title: " labels, assignees, reviewers (✓: on it; Enter adds or removes) ", items, query: String::new(), sel: 0 });
    }

    pub(super) fn react_menu(&mut self, node: &str) {
        self.msg.clear();
        let now = match gh::reactions_of(node) {
            Ok(r) => r,
            Err(e) => return self.msg = format!("{e:#}"),
        };
        let items = gh::REACTIONS
            .iter()
            .map(|&(what, emoji)| {
                let (n, mine) = now.iter().find(|r| r.0 == what).map_or((0, false), |r| (r.1, r.2));
                let count = if n > 0 { format!(" {n}") } else { String::new() };
                let label = match mine {
                    true => format!("{emoji}{count}  ✓ yours: Enter takes it back"),
                    false => format!("{emoji}{count}"),
                };
                Item::new(label, Target::React(node.to_owned(), what, !mine))
            })
            .collect();
        self.picker = Some(Picker { title: " react ", items, query: String::new(), sel: 0 });
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
        if row.info == Some(SUMMARY) {
            return self.promote_summary(review);
        }
        if review && row.info == Some(DRAFT) {
            return self.msg = "this is your pending review's text: S submits it, editing it first".into();
        }
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
            // (Not quoting what's hidden on GitHub, unseen.)
            (true, _) if review => self.quote_into_summary(),
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
        if self.before_head() {
            bail!("comments go on the PR's head: show commits up to the last one (or all) to comment");
        }
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
        let i = row.info.filter(|&i| i < self.info.len());
        // (Hidden on GitHub, unseen: not quoted, and the editor says why.)
        let quote = match i.map(|i| self.seen_info(i)) {
            Some(None) => format!("{HINT} not quoted: it's hidden on GitHub (Tab on it shows it) -->\n"),
            i => i.flatten().map_or(String::new(), |l| quote(&l.text)),
        };
        self.action = Some(Action::Converse { quote, as_is: false });
    }

    /// `c` in the PR section: the comment (or description) under the cursor, quoted at the end
    /// of your pending review's text.
    pub(super) fn quote_into_summary(&mut self) {
        let i = self.rows[self.head.row].info.filter(|&i| i < self.info.len());
        match i.map(|i| self.seen_info(i)) {
            Some(None) => self.msg = HIDDEN_FIRST.into(),
            Some(Some(l)) if l.md && !l.text.trim().is_empty() && self.pr.is_some() => {
                let q = quote(&l.text);
                self.add_to_summary(q.trim_end());
            }
            _ => self.msg = "c here: on a comment or the description, quotes it in your review summary".into(),
        }
    }

    /// Appends `text` to your pending review's text, on GitHub (starting the review if need be).
    pub(super) fn add_to_summary(&mut self, text: &str) {
        // (Submitting goes with what's in the editor: added here, it'd be lost.)
        if matches!(self.writing, Some((Action::Submit(_), _))) {
            return self.msg = "you're writing what goes with your review in the editor's pane: add it there".into();
        }
        let draft = self.summary_draft.trim_end();
        self.summary_draft = if draft.is_empty() { text.to_owned() } else { format!("{draft}\n\n{text}") };
        self.msg = self.post_later(Post::Summary(self.summary_draft.clone())).unwrap_or_else(|e| format!("{e:#}"));
    }

    /// `c` / `C` on Claude's review (its summary), as on its comments: yours to use, not to
    /// quote. `c` puts it in your review summary (`S` starts from it); `C` posts it as your PR
    /// comment, after the editor.
    pub(super) fn promote_summary(&mut self, review: bool) {
        if self.pr.is_none() {
            return self.msg = "no PR for this branch".into();
        }
        let text = self.claude.summary.trim().to_owned();
        match review {
            true => self.add_to_summary(&text),
            false => self.action = Some(Action::Converse { quote: format!("{text}\n"), as_is: true }),
        }
    }

    pub(super) fn start_reply(&mut self, review: bool) {
        // On Claude's review summary: go on in the thread replying to it, or start one.
        if self.rows[self.head.row].info == Some(SUMMARY) && self.rows[self.head.row].thread.is_none() {
            if self.claude.path.is_none() {
                return self.msg = "no review file here (not in a git repo?)".into();
            }
            return self.action = Some(match self.threads.iter().find(|t| t.conversation == ON_SUMMARY).and_then(|t| t.claude.clone()) {
                Some(k) => Action::ClaudeReply(k, "", String::new()),
                None => Action::AskConversation { url: ON_SUMMARY.into(), prefill: String::new(), want: "" },
            });
        }
        // On lines, say what you want of Claude; under a conversation comment it's a question.
        if let Some((t, k)) = self.claude_at_cursor() {
            return match self.threads[t].conversation.is_empty() {
                true => self.pick_want(Action::ClaudeReply(k, "", String::new()), Some(t)),
                false => self.action = Some(Action::ClaudeReply(k, "", String::new())),
            };
        }
        let t = self.rows[self.head.row].thread;
        match (t, t.and_then(|t| self.root_id(t))) {
            (Some(t), Some(comment)) if self.pr.is_some() => {
                let thread = self.threads[t].id.clone();
                let quote = self.comment_under_cursor(t).and_then(|c| self.seen_body(c)).map_or(String::new(), quote);
                self.action = Some(Action::Reply { thread, comment, review, quote });
            }
            _ => self.msg = "move onto a comment thread to reply".into(),
        }
    }

}

/// What `c`, `Space e` and the like say on a comment hidden on GitHub, not shown here.
const HIDDEN_FIRST: &str = "it's hidden on GitHub: Tab on it shows it first";

impl App {
}

/// `S` → "Comment and close the PR": a comment review (if there's anything to say), then closing.
pub(super) const CLOSE: &str = "CLOSE";
