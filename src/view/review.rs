//! GitHub review actions: comments, replies, suggestions, edits, deletes, reviews, resolving,
//! viewed marks, links, and the pending/threads pickers.
use super::*;
use std::time::Instant;

/// Tries after the first for a viewed mark, waiting 1s, 2s, 4s… between them (about 2 minutes).
const MARK_TRIES: u32 = 7;

/// A viewed mark on its way to GitHub: what it'll be (`on`), what GitHub had before (to put
/// back if it never lands), and when it's tried again, after a miss.
pub(super) struct Marking {
    pub(super) on: bool,
    was: Option<String>,
    rx: std::sync::mpsc::Receiver<Mark>,
    pub(super) retry: Option<Instant>,
}

enum Mark {
    Trying,
    Retry(String, Instant),
    Done,
    GaveUp(String),
}

impl App {
    /// The pending review to add to, starting one on `commit` (a comment's lines' own; else the
    /// PR's head) if there isn't one yet, with `body` (your review text, as GitHub can't add any
    /// later to one started without). The caller's, as when posted: a refresh meanwhile can't
    /// swap them out.
    /// One on another commit (started before a pull) can't take lines read on this one (they'd
    /// land on its), said.
    pub(super) fn ensure_review(&mut self, body: &str, commit: Option<&str>) -> Result<String> {
        if let Some(r) = &self.review {
            self.pending_on(commit)?;
            return Ok(r.clone());
        }
        let pr = self.pr.as_ref().context("no PR")?;
        let on = commit.unwrap_or(&pr.head_oid).to_owned();
        let id = gh::start_review(&pr.node_id, &on, body)?;
        (self.review, self.review_on) = (Some(id.clone()), on);
        Ok(id)
    }

    /// A pending review (if there is one) is on `commit` (none: any), or it's said what to do.
    pub(super) fn pending_on(&self, commit: Option<&str>) -> Result<()> {
        match commit.filter(|c| self.review.is_some() && !self.review_on.is_empty() && *c != self.review_on) {
            Some(_) => bail!(
                "your pending review is on {}, before the head moved: S → Comment submits it, or S → Discard pending review drops it",
                &self.review_on[..self.review_on.len().min(8)]
            ),
            None => Ok(()),
        }
    }

    pub(super) fn run_job(&mut self, job: Job) {
        // What changes GitHub works on the PR as it is now, not as cached (not what only reads
        // it, nor git's own work).
        if !matches!(job, Job::Preview(_) | Job::OpenIssue(..) | Job::Reactions(_) | Job::Labels | Job::Retarget(_) | Job::Rebase | Job::Pull | Job::Push(_)) {
            self.ensure_fresh();
        }
        match job {
            Job::Pull => {
                let res = self.pull();
                self.finish(res);
            }
            Job::Push(force) => {
                let res = self.push(force);
                if res.is_ok() {
                    self.github_changed();
                }
                self.finish(res); // the PR's head moved
            }
            // Reruns: GitHub sets them going in a moment, and CI doesn't change the PR's ETag:
            // dv looks again by itself, shortly.
            Job::Rerun(all) => {
                let head = self.pr.as_ref().map(|p| p.head_oid.clone()).unwrap_or_default();
                self.msg = match gh::rerun(&head, all) {
                    Ok(said) => {
                        // What a fetch under way brings back is from before (failing still);
                        // the watch fetches again by itself, shortly (through `watch_github`).
                        self.github_changed();
                        self.refresh_at = Some(std::time::Instant::now() + std::time::Duration::from_secs(10));
                        said
                    }
                    Err(e) => format!("{e:#}"),
                };
            }
            Job::Rebase => self.start_rebase(),
            Job::Retarget(base) => self.retarget(base),
            Job::Labels => self.labels_menu(),
            Job::Reactions(node) => self.react_menu(&node),
            Job::Preview(url) => match gh::issue_preview(&url) {
                Ok((title, md)) => (self.msg.clear(), self.popup = Some((title, self.hover_lines(&md), 0))).1,
                Err(e) => self.msg = format!("{e:#}"),
            },
            // (GitHub's links say /issues/ for PRs too: it's asked which.)
            Job::OpenIssue(n, url) => match gh::is_pr(n) {
                true if self.hands_prs => (self.hand_off, self.msg) = (Some(HandOff::Pr(n)), String::new()),
                true => self.open_url(&url),
                false => (self.hand_off, self.msg) = (Some(HandOff::Issue(n)), String::new()),
            },
            Job::Refresh => self.finish(Ok("refreshed")),
            Job::Resolve(t, on) => self.finish(gh::set_resolved(&t, on).map(|_| if on { "thread resolved" } else { "thread unresolved" })),
            Job::Discard(r) => {
                let res = gh::delete_review(&r);
                if res.is_ok() {
                    self.review = None;
                }
                self.finish(res.map(|_| "pending review discarded"));
            }
            Job::DeleteComment(c) => self.finish(gh::delete_comment(&c).map(|_| "comment deleted")),
            Job::SetField(field, name, on) => {
                let n = self.subject().map_or(0, |s| s.n());
                let res = gh::set_field(n, field, &name, on);
                self.msg = match &res {
                    Err(e) => format!("{e:#}"),
                    Ok(()) => match (field, on) {
                        (gh::Field::Label, true) => format!("label {name} added"),
                        (gh::Field::Label, false) => format!("label {name} removed"),
                        (gh::Field::Assignee, true) => format!("@{name} assigned"),
                        (gh::Field::Assignee, false) => format!("@{name} unassigned"),
                        (gh::Field::Reviewer, true) => format!("review requested from @{name}"),
                        (gh::Field::Reviewer, false) => format!("review request to @{name} withdrawn"),
                    },
                };
                // The picker again, as you left it (what's on it as now); the PR's facts, at
                // GitHub's next look (a moment: no reload of the diff for it).
                if res.is_ok()
                    && let Some(f) = &mut self.fields
                {
                    let list = match field {
                        gh::Field::Label => &mut f.0,
                        gh::Field::Assignee => &mut f.1,
                        gh::Field::Reviewer => &mut f.2,
                    };
                    list.iter_mut().filter(|x| x.0 == name).for_each(|x| x.1 = on);
                    self.github_changed();
                }
                if let Some((query, field, name)) = self.fields_at.take() {
                    self.fields_menu(query, Some((field, name)));
                }
            }
            Job::React(node, what, add) => self.finish(gh::react(&node, what, add).map(|_| if add { "reacted" } else { "reaction taken back" })),
            Job::Post(p) => {
                let res = self.post(p);
                // (A change dv made, as any other: what's fetched from before it doesn't undo it.)
                if res.is_ok() {
                    self.github_changed();
                }
                self.finish(res);
            }
        }
    }

    /// After a job that changed something: the diff reloaded (the working tree, or the PR's
    /// head, may have changed), and what came of it said.
    fn finish(&mut self, res: Result<impl Into<String>>) {
        self.reloaded(res, true);
    }

    /// The diff reloaded (`threads`: GitHub's too), and what came of `res` said: its error, or
    /// the reload's, or what it did.
    pub(super) fn reloaded(&mut self, res: Result<impl Into<String>>, threads: bool) {
        let reload = self.reload_all(threads);
        self.msg = match res {
            Ok(m) => with_reload(m.into(), reload),
            Err(e) => format!("{e:#}"),
        };
    }

    /// Queue `p` to send after the next frame, which shows what's being sent.
    pub(super) fn post_later(&mut self, p: Post) -> Result<String> {
        self.start(Job::Post(p));
        Ok(std::mem::take(&mut self.msg))
    }

    /// `job`, after the next frame, which says what it's doing.
    pub(super) fn start(&mut self, job: Job) {
        self.msg = job.doing();
        self.job = Some(job);
    }

    /// Send `p`; if that fails, its text is saved to a file so it isn't lost.
    pub(super) fn post(&mut self, p: Post) -> Result<&'static str> {
        // (On the conversation, an edit: a PR's or an issue's alike; the rest, a PR's.)
        let pr = || self.pr.clone().context("no PR");
        match p {
            Post::Thread { anchor, review, body, claude } => {
                let res = match review {
                    true => {
                        let draft = self.summary_draft.clone();
                        self.ensure_review(&draft, Some(&anchor.commit)).and_then(|r| gh::add_thread(&r, &anchor, &body))
                    }
                    false => pr().and_then(|pr| gh::comment(&pr, &anchor, &body)),
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
                    true => {
                        let draft = self.summary_draft.clone();
                        self.ensure_review(&draft, None).and_then(|r| gh::add_reply(&r, &thread, &body))
                    }
                    false => pr().and_then(|pr| gh::reply(&pr, comment, &body)),
                };
                keep_on_error(res, &body).map(|_| posted_msg(review))
            }
            Post::Converse(body) => {
                let n = self.subject().context("no PR or issue")?.n();
                keep_on_error(gh::conversation_comment(n, &body), &body).map(|_| "comment posted to the conversation")
            }
            Post::Summary(body) => {
                // A new review starts with it; one started without text can't take any yet, so
                // it stays here (S submits it with the review).
                let fresh = self.review.is_none();
                let res = self.ensure_review(&body, None).and_then(|r| if fresh { Ok(()) } else { gh::set_review_body(&r, &body) });
                match res {
                    Err(e) if format!("{e:#}").contains("missing body") => Ok(
                        "in your review summary till you quit (GitHub won't hold it yet): S submits it",
                    ),
                    res => keep_on_error(res, &body).map(|_| "in your review summary: S submits it (editing it first)"),
                }
            }
            Post::Edit { comment, body } => keep_on_error(gh::edit_comment(&comment, &body), &body).map(|_| "comment updated"),
            Post::Submit(event, body, on) => {
                // Closing comments; with nothing to say (and no pending comments) it only closes.
                let pr = pr()?;
                let close = event == CLOSE;
                let event = if close { "COMMENT" } else { event };
                if !close || self.review.is_some() || !body.trim().is_empty() {
                    // (An approval's ACK names a head: its review on another, refused.)
                    let res = match &self.review {
                        Some(r) => self.pending_on((event == "APPROVE").then_some(on.as_str())).and_then(|_| gh::submit_review(r, event, &body)),
                        // (On the head its summary was written on: an ACK names it.)
                        None => gh::review_now(&pr.node_id, &on, event, &body),
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
    pub(super) fn apply_viewed(&mut self, mut viewed: HashMap<String, String>) {
        // (Marks on their way stand: GitHub hasn't heard them yet.)
        for (path, m) in &self.marking {
            match m.on {
                true => viewed.insert(path.clone(), "VIEWED".into()),
                false => viewed.remove(path),
            };
        }
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
        // Shown (and folded) at once; GitHub's told in the background.
        // (Marked again before the last landed: what GitHub had is still from before that.)
        let was = self.marking.remove(&path).map_or_else(|| self.viewed.get(&path).cloned(), |m| m.was);
        match on {
            true => _ = (self.viewed.insert(path.clone(), "VIEWED".into()), self.collapsed.insert(path.clone())),
            false => _ = (self.viewed.remove(&path), self.collapsed.remove(&path)),
        }
        let pr = self.pr.as_ref().map(|p| p.node_id.clone()).unwrap_or_default();
        let (tx, rx) = std::sync::mpsc::channel();
        let p = path.clone();
        std::thread::spawn(move || {
            for n in 0.. {
                let res = if cfg!(test) { Ok(()) } else { gh::set_viewed(&pr, &p, on) };
                let wait = Duration::from_secs(1 << n);
                let next = match res {
                    Ok(()) => return drop(tx.send(Mark::Done)),
                    Err(e) if n == MARK_TRIES => return drop(tx.send(Mark::GaveUp(format!("{e:#}")))),
                    Err(e) => Mark::Retry(format!("{e:#}"), Instant::now() + wait),
                };
                // (Not heard: marked again since, or dv closed. Then it's let go.)
                if tx.send(next).is_err() {
                    return;
                }
                std::thread::sleep(wait);
                if tx.send(Mark::Trying).is_err() {
                    return;
                }
            }
        });
        // ponytail: a mark sent while an older one's in flight may land first; one queue per file if it bites.
        self.marking.insert(path.clone(), Marking { on, was, rx, retry: None });
        self.msg = if on { "marked as viewed" } else { "unmarked as viewed" }.into();
        // To the file's header, or when marked, on to the next file (like on GitHub).
        let keep = self.cursor_key();
        self.rebuild(keep);
        if let Some(fi) = self.files.iter().position(|f| f.path == path)
            && let Some(own) = self.rows.iter().position(|r| r.file == fi && r.line.kind == Kind::File)
        {
            let later = self.rows[own + 1..].iter().position(|r| r.line.kind == Kind::File).map(|i| own + 1 + i);
            self.select = false;
            self.goto(if on { later.unwrap_or(own) } else { own }, 0);
        }
    }

    /// How the viewed marks on their way are going: one GitHub wouldn't take, after its
    /// retries, is as GitHub has it again.
    pub(super) fn watch_marks(&mut self) {
        let mut over = vec![];
        for (path, m) in &mut self.marking {
            for got in m.rx.try_iter() {
                match got {
                    Mark::Trying => m.retry = None,
                    Mark::Retry(why, at) => {
                        m.retry = Some(at);
                        let secs = at.saturating_duration_since(Instant::now()).as_secs() + 1;
                        self.msg = format!("GitHub didn't take {path}'s viewed mark ({why}); dv tries again in {secs}s");
                    }
                    Mark::Done => over.push((path.clone(), None)),
                    Mark::GaveUp(why) => over.push((path.clone(), Some(why))),
                }
            }
        }
        let mut undone = false;
        for (path, failed) in over {
            let Some(m) = self.marking.remove(&path) else { continue };
            // A fetch under way is from before: the next one's GitHub's word on it.
            self.github_changed();
            let Some(why) = failed else { continue };
            // (Its fold stays: you may have opened it since.)
            match m.was {
                Some(s) => self.viewed.insert(path.clone(), s),
                None => self.viewed.remove(&path),
            };
            let what = if m.on { "marked viewed" } else { "unmarked" };
            self.msg = format!("{path} isn't {what} on GitHub after {} tries ({why}): it's as it was", MARK_TRIES + 1);
            undone = true;
        }
        if undone {
            let keep = self.cursor_key();
            self.rebuild(keep);
        }
    }

    pub(super) fn pending_count(&self) -> usize {
        self.threads.iter().flat_map(|t| &t.comments).filter(|c| c.pending).count()
    }

    pub(super) fn review_menu(&mut self) {
        if !self.has_code() {
            return self.msg = super::git::NO_CODE.into();
        }
        if self.pr.is_none() && self.replacement.is_some() {
            return self.replacement_menu();
        }
        if self.pr.is_none() {
            self.msg = "no PR for this branch".into();
            return;
        }
        let item = |label: &str, target| Item::new(label, target);
        let mut items = vec![
            item("Comment", Target::Do(Action::Submit("COMMENT"))),
            item("Approve", Target::Do(Action::Submit("APPROVE"))),
            item("Request changes", Target::Do(Action::Submit("REQUEST_CHANGES"))),
            item("Close the PR (with a comment, optional)", Target::Do(Action::Submit(CLOSE))),
        ];
        if self.review.is_some() {
            let n = self.pending_count();
            items.push(item(&format!("Discard pending review ({n} comment(s))"), Target::Discard));
        }
        // A replacement, published: closing what it replaces is still to do.
        if let Some(r) = &self.replacement {
            let open: Vec<String> = r.replaces.iter().filter(|o| !r.closed.contains(&o.number)).map(|o| format!("#{}", o.number)).collect();
            if !open.is_empty() {
                items.push(item(&format!("Close {} (what it replaces) with the closing note…", open.join(", ")), Target::Do(Action::CloseOriginals)));
            }
        }
        // What's still open, to see before you say you're done (approving most of all).
        items.extend(self.submit_notes().iter().map(|n| item(n, Target::Warn)));
        self.picker = Some(Picker::new(" submit review ", items));
    }

    /// What's still open in the review: threads waiting on someone, pending comments whose
    /// code has changed, and a checkout that isn't the PR's head.
    pub(super) fn submit_notes(&self) -> Vec<String> {
        let count = |f: &dyn Fn(&Thread) -> bool| self.threads.iter().filter(|t| f(t)).count();
        let open = count(&|t| t.claude.is_some() && !t.stale());
        let answered = count(&|t| t.answered());
        let github = count(&|t| t.claude.is_none() && !t.stale() && !t.comments.iter().all(|c| c.pending));
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
            Item::new(format!("Yes, delete @{}'s comment: {preview}", c.author), Target::Run(Job::DeleteComment(c.node_id.clone()))),
        ];
        self.picker = Some(Picker::new(" delete comment? ", items));
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
            let th = &self.threads[t];
            // Your note for Claude: resolved or reopened.
            if !th.by_claude() {
                return match th.resolved {
                    true => self.edit_claude(&k, |p, k| crate::claude::set_resolved(p, k, false), "unresolved (your note for Claude)"),
                    false => self.edit_claude(&k, |p, k| crate::claude::set_resolved(p, k, true), "resolved (your note for Claude)"),
                };
            }
            // Claude's comment is what's kept (and posted): Space r resolves the talk under it,
            // not it (Space d deletes it). One Claude marked done (its change made): reopened.
            return match (th.resolved, th.talk_resolved, th.comments.len() > 1) {
                (true, ..) => self.edit_claude(&k, |p, k| crate::claude::set_resolved(p, k, false), "reopened (Claude's comment)"),
                (_, true, _) => self.edit_claude(&k, |p, k| crate::claude::set_talk_resolved(p, k, false), "the conversation under Claude's comment reopened"),
                (_, false, true) => self.edit_claude(&k, |p, k| crate::claude::set_talk_resolved(p, k, true), "the conversation under Claude's comment resolved (the comment stays)"),
                (_, false, false) => self.msg = "Claude's comment has no conversation to resolve (Space d deletes the comment)".into(),
            };
        }
        match self.rows[self.head.row].thread.map(|t| &self.threads[t]) {
            Some(t) => {
                let job = Job::Resolve(t.id.clone(), !t.resolved);
                self.start(job);
            }
            None => self.msg = "move onto a comment thread".into(),
        }
    }

    /// Claude's review file, replacing its threads (GitHub's stay).
    pub(super) fn load_claude(&mut self) {
        let had_new = self.new_paths();
        // (Its time before it's read: a write landing meanwhile is read next time round.)
        self.claude.mtime = self.claude.path.as_ref().and_then(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());
        let loaded = self.claude.path.as_ref().map(|p| crate::claude::load_with_ids(p));
        // Unreadable: what was read of it last stays, not "Claude hasn't reviewed this".
        if let Some(Err(e)) = &loaded {
            let why = format!("{e:#}");
            self.msg = format!("Claude's review file can't be read (shown as it last was): {why}");
            self.claude.broken = Some(why);
            // (Kept, over a diff that may have changed since: following their code.)
            if self.whole_pr() {
                crate::claude::reanchor(&mut self.threads, &self.files);
            }
            return;
        }
        self.claude.broken = None;
        // (`dv list`'s ranking: its word on this PR, or this replacement.)
        // (Its PR's item, then a replacement's own: closing what it replaces, say.)
        let keys: Vec<String> = self.pr.iter().map(|p| format!("pr-{}", p.number)).chain(self.replacement.iter().map(|r| format!("rep-{}", r.id))).collect();
        let ranked = crate::todo::load_of(crate::todo::Of::Prs);
        self.ranked_next = ranked.and_then(|r| keys.iter().find_map(|k| r.items.iter().find(|i| &i.0 == k && !i.1.trim().is_empty())).map(|i| (i.1.clone(), r.at)));
        self.threads.retain(|t| t.claude.is_none());
        self.claude.summary.clear();
        self.claude.head.clear();
        self.claude.created.clear();
        self.claude.description.clear();
        self.claude.closing.clear();
        (self.claude.triage, self.claude.close) = Default::default();
        if let Some(Ok(Some(mut r))) = loaded {
            // Comments with their code recorded follow it; others can only trust their lines.
            let head = diff::git(&["rev-parse", "HEAD"]).unwrap_or_default();
            let moved = !r.head.is_empty() && !head.trim().starts_with(&r.head) && !r.head.starts_with(head.trim());
            if moved && r.threads.iter().any(|t| t.hunk.is_empty()) {
                self.msg = format!(
                    "Claude reviewed {}; HEAD has moved since, so its line numbers may be off (`dv review-check` re-anchors)",
                    r.head.get(..8).unwrap_or(&r.head)
                );
            }
            // Not against HEAD: files without uncommitted changes aren't in that diff, and
            // their comments would look outdated (and could be dismissed as such).
            if self.whole_pr() {
                crate::claude::reanchor(&mut r.threads, &self.files);
            }
            (self.claude.summary, self.claude.head, self.claude.description, self.claude.closing) = (r.summary, r.head, r.description, r.closing);
            (self.claude.created, self.claude.triage, self.claude.close) = (r.created, r.triage, r.close);
            self.threads.extend(r.threads);
            // (Tab's shown talks: while resolved only, so one resolved again, by you or Claude,
            // folds.)
            let resolved: HashSet<u64> = (0..self.threads.len()).filter(|&t| self.threads[t].talk_resolved).filter_map(|t| self.root_id(t)).collect();
            self.talk_shown.retain(|id| resolved.contains(id));
            self.unfold_new(&had_new);
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
                let n = self.threads.iter().filter(|t| t.by_claude()).count();
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
        if !th.by_claude() {
            return self.msg = "this is your comment for Claude (private): r replies, Space d removes it".into();
        }
        if th.outdated {
            return self.msg = "this comment is outdated (its code changed): ask Claude to update it, or Space d".into();
        }
        if !th.conversation.is_empty() {
            return self.msg = "Claude's answer here is for you, on no lines: C on the comment above quotes it in a new conversation comment".into();
        }
        let anchor = self.claude_anchor(t);
        // Claude's comment, wherever in its thread the cursor is: the talk under it is about it,
        // not what's posted.
        let mut body = th.comments.first().map(|c| c.body.clone()).unwrap_or_default();
        // On GitHub the severity leads the text, as reviewers write it: "**nit:** …".
        if !th.severity.is_empty() && !body.starts_with("**") {
            body = format!("**{}:** {body}", th.severity);
        }
        match self.line_comment(Ok(anchor)) {
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
        // (Its text's heading's: `text_at`, as c and C go by.)
        let head = self.info.get(self.text_at()?.checked_sub(1)?).filter(|h| h.heading)?;
        Some(head.node.clone()).filter(|n| !n.is_empty())
    }

    /// `Space =`: react to the comment under the cursor, or take your reaction back: what's
    /// on it now is asked of GitHub first (a job, so a frame says so), so yours show.
    pub(super) fn start_react(&mut self) {
        match self.reactable_at_cursor() {
            Some(node) => self.start(Job::Reactions(node)),
            None => self.msg = "move onto a GitHub comment (or the description) to react to it".into(),
        }
    }

    /// `Space l`: the PR's labels, assignees and requested reviewers, and what it could have
    /// (✓: on it); Enter adds or removes one. Type to find one (`review`, `@` for people).
    pub(super) fn labels_menu(&mut self) {
        self.msg.clear();
        // (An issue's: no reviewers.)
        let fields = match self.subject() {
            Some(crate::claude::Subject::Pr(n)) => gh::pr_fields(n),
            Some(crate::claude::Subject::Issue(n)) => gh::issue_fields(n).map(|(l, p)| (l, p, vec![])),
            None => return,
        };
        match fields {
            Ok(x) => self.fields = Some(x),
            Err(e) => return self.msg = format!("{e:#}"),
        };
        self.fields_menu(String::new(), None);
    }

    /// The labels picker, of what GitHub said (`fields`, kept up as you change it): what was
    /// typed, and where you were, as given. It stays as you pick (one after another).
    /// (`on`: the one to be on, as picked: its place moves as what's on sorts first.)
    pub(super) fn fields_menu(&mut self, query: String, on: Option<(gh::Field, String)>) {
        let Some((labels, people, reviewers)) = &self.fields else { return };
        // (Under their sections: Reviewers, Labels, Assignees.)
        // (The labels Claude's triage suggests, not on it yet: marked, first.)
        let suggested = |f: gh::Field, x: &str, on: bool| f == gh::Field::Label && !on && self.claude.triage.labels.iter().any(|l| l.eq_ignore_ascii_case(x));
        let item = |field: gh::Field, name: &str, on: bool| {
            let at = if matches!(field, gh::Field::Label) { "" } else { "@" };
            let tick = match (on, suggested(field, name, on)) {
                (true, _) => "✓ ",
                (_, true) => "◆ ",
                _ => "  ",
            };
            Item::new(format!("{tick}{at}{name}"), Target::Run(Job::SetField(field, name.to_owned(), !on)))
        };
        let items: Vec<Item> = [(gh::Field::Reviewer, &reviewers), (gh::Field::Label, &labels), (gh::Field::Assignee, &people)]
            .into_iter()
            .flat_map(|(f, xs)| {
                let mut xs: Vec<&(String, bool)> = xs.iter().collect();
                xs.sort_by_key(|(x, on)| !(*on || suggested(f, x, *on)));
                xs.into_iter().map(move |(x, on)| item(f, x, *on))
            })
            .collect();
        if items.is_empty() {
            return self.msg = "no labels in this repo, nor anyone to assign or ask".into();
        }
        let p = Picker::new(" reviewers, labels, assignees (✓ on it, ◆ Claude suggests it: Enter adds or takes it off) ", items);
        self.picker = Some(Picker { query, ..p });
        let picked = |t: &Target| matches!((t, &on), (Target::Run(Job::SetField(f, n, _)), Some((g, m))) if f == g && n == m);
        let sel = self.matches().into_iter().position(|i| self.picker.as_ref().is_some_and(|p| picked(&p.items[i].target))).unwrap_or(0);
        if let Some(p) = &mut self.picker {
            p.sel = sel;
        }
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
                Item::new(label, Target::Run(Job::React(node.to_owned(), what, !mine)))
            })
            .collect();
        self.picker = Some(Picker::new(" react ", items));
    }

    /// GitHub anchor for the selected code lines (comment rows inside the selection are ignored).
    pub(super) fn selection_anchor(&self) -> Result<Anchor> {
        let (s, e) = self.ordered();
        let code: Vec<usize> = (s.row..=e.row).filter(|&r| self.in_selection(r) && self.rows[r].line.kind.is_code()).collect();
        let (Some(&a), Some(&b)) = (code.first(), code.last()) else { bail!("select code lines to comment on") };
        if self.rows[a].file != self.rows[b].file {
            bail!("a comment can't span files");
        }
        // (A removed line's the old side; so is any on a split view's left half.)
        let side = |r: usize| {
            let l = &self.rows[r].line;
            if l.kind == Kind::Del || (self.split && !self.head_right()) { (l.old, true) } else { (l.new, false) }
        };
        let ((Some(line), left), (Some(sl), sleft)) = (side(b), side(a)) else { bail!("no line number") };
        Ok(Anchor {
            path: self.files[self.rows[b].file].path.clone(),
            line,
            left,
            start: (a != b).then_some((sl, sleft)),
            ..Default::default()
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
        let pr_text = row.file == INFO && row.thread.is_none();
        if let (false, Err(e)) = (review, self.posts_now()) {
            return self.msg = format!("{e:#}");
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

    /// Whether `C` can post right away here: GitHub allows one pending review per PR and
    /// refuses single line comments beside it (not on the PR's text: a conversation comment).
    /// The one rule `C` and its key hint go by.
    pub(super) fn posts_now(&self) -> Result<()> {
        let row = &self.rows[self.head.row];
        let pr_text = row.file == INFO && row.thread.is_none();
        if self.review.is_some() && !pr_text && row.info != Some(SUMMARY) {
            // What queues here says so: a GitHub thread, r (a reply); lines (Claude's comment's
            // too), c, unless the review's on a head moved past (c can't add them then: said).
            let n = self.pending_count();
            if row.thread.is_some_and(|t| self.threads[t].claude.is_none()) {
                bail!("you have a pending review ({n}): r adds a reply to it, S submits it, then C posts");
            }
            self.pending_on(self.pr.as_ref().map(|p| p.head_oid.as_str()))?;
            bail!("you have a pending review ({n}): c adds to it, S submits it, then C posts");
        }
        Ok(())
    }

    /// The PR line comments go on, at its head as shown (a draft has none yet; nor before its
    /// last commit). Without asking git: the key hints go by it too, with `on_github`.
    pub(super) fn on_head(&self) -> Result<&Pr> {
        if self.unpublished() {
            bail!("it's a draft: nothing on GitHub to comment on yet (a comments for Claude; S publishes it)");
        }
        let pr = self.pr.as_ref().context("no PR for this branch")?;
        if self.before_head() {
            bail!("comments go on the PR's head: show commits up to the last one (or all) to comment");
        }
        // (Lines read on this head can't go in a pending review on another: refused before
        // the editor, and hinted so.)
        self.pending_on(Some(&pr.head_oid))?;
        Ok(pr)
    }

    /// What GitHub takes a line comment on: the PR at its head (`on_head`), then the `lines`,
    /// then a file it has (`on_github`). Without git: the key hints' rule too.
    pub(super) fn github_anchor(&self, lines: Result<Anchor>) -> Result<Anchor> {
        self.on_head()?;
        self.on_github(lines?)
    }

    /// A line comment on `lines` (`github_anchor`), as it's posted from here: the checkout at
    /// the PR's head, its tree clean; else the lines would shift.
    pub(super) fn line_comment(&self, lines: Result<Anchor>) -> Result<Anchor> {
        let a = self.github_anchor(lines)?;
        let pr = self.on_head()?;
        if diff::git(&["rev-parse", "HEAD"])?.trim() != pr.head_oid {
            bail!("HEAD isn't the PR head (pull, or `gh pr checkout {}`)", pr.number);
        }
        if !diff::git(&["status", "--porcelain", "--untracked-files=no"])?.trim().is_empty() {
            bail!("working tree has local changes; stash them to comment");
        }
        // (Posted on this head, the lines' own, however the PR moves meanwhile.)
        Ok(Anchor { commit: pr.head_oid.clone(), ..a })
    }

    /// Where Claude's comment `t` is, for posting it.
    pub(super) fn claude_anchor(&self, t: usize) -> Anchor {
        let th = &self.threads[t];
        Anchor { path: th.path.clone(), line: th.line.unwrap_or(0), left: th.left, start: th.range_start(), ..Default::default() }
    }

    /// GitHub only takes comments on files the PR has: not on one that's untracked here.
    pub(super) fn on_github(&self, a: Anchor) -> Result<Anchor> {
        match self.files.iter().any(|f| f.path == a.path && f.meta == "untracked") {
            true => bail!("{} is untracked: GitHub hasn't got it (commit and push it: Space g), or `a` asks Claude", a.path),
            false => Ok(a),
        }
    }

    pub(super) fn start_comment(&mut self, review: bool) {
        match self.line_comment(self.selection_anchor()) {
            Ok(anchor) => self.action = Some(Action::Comment { anchor, review, prefill: String::new(), claude: None }),
            Err(e) => self.msg = format!("{e:#}"),
        }
    }

    /// `s`: a GitHub suggestion for the selected new-side lines, pre-filled with their code.
    pub(super) fn start_suggest(&mut self) {
        let s = self.suggestion();
        match self.line_comment(s.as_ref().map(|s| s.0.clone()).map_err(|e| anyhow!("{e:#}"))) {
            Ok(anchor) => self.action = Some(Action::Comment { anchor, review: true, prefill: s.map(|s| s.1).unwrap_or_default(), claude: None }),
            Err(e) => self.msg = format!("{e:#}"),
        }
    }

    pub(super) fn suggestion(&self) -> Result<(Anchor, String)> {
        let (s, e) = self.ordered();
        let rows: Vec<&Row> = (s.row..=e.row).filter(|&r| self.in_selection(r)).map(|r| &self.rows[r]).filter(|r| r.line.kind.is_code()).collect();
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
            Anchor { path: self.files[first.file].path.clone(), line: b, left: false, start: (a != b).then_some((a, false)), ..Default::default() };
        Ok((anchor, format!("\n{fence}suggestion\n{code}\n{fence}\n")))
    }

    /// `C` in the PR section: a new PR conversation comment quoting the comment (or description)
    /// under the cursor, like GitHub's "Quote reply"; on other PR info lines, an empty one.
    pub(super) fn start_conversation_comment(&mut self) {
        if self.subject().is_none() {
            return self.msg = "no PR for this branch".into();
        }
        // (Hidden on GitHub, unseen: not quoted, and the editor says why; an event's line, none.)
        let quote = match self.quotable() {
            Some(None) => format!("{HINT} not quoted: it's hidden on GitHub (Tab on it shows it) -->\n"),
            i => i.flatten().map_or(String::new(), |l| quote(&l.text)),
        };
        self.action = Some(Action::Converse { quote, as_is: false });
    }

    /// `c` in the PR section: the comment (or description) under the cursor, quoted at the end
    /// of your pending review's text.
    pub(super) fn quote_into_summary(&mut self) {
        if !self.has_code() {
            return self.msg = NO_REVIEW.into();
        }
        match self.quotable() {
            Some(None) => self.msg = HIDDEN_FIRST.into(),
            Some(Some(l)) if !l.text.trim().is_empty() && self.pr.is_some() => {
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
        if review && !self.has_code() {
            return self.msg = NO_REVIEW.into();
        }
        if self.subject().is_none() {
            return self.msg = "no PR for this branch".into();
        }
        let text = self.claude.summary.trim().to_owned();
        match review {
            true => self.add_to_summary(&text),
            false => self.action = Some(Action::Converse { quote: format!("{text}\n"), as_is: true }),
        }
    }

    pub(super) fn start_reply(&mut self, review: bool) {
        // Several of Claude's threads selected: one reply to them all.
        if let Some(keys) = self.selected_claude_threads() {
            return self.pick_want(Action::ClaudeReplyAll(keys, ""), None);
        }
        // What's selected, quoted in the reply: what it's about.
        let sel = self.selected_quote();
        // On Claude's review summary: go on in the thread replying to it, or start one.
        if self.rows[self.head.row].info == Some(SUMMARY) && self.rows[self.head.row].thread.is_none() {
            if self.claude.path.is_none() {
                return self.msg = "no review file here (not in a git repo?)".into();
            }
            let prefill = sel.unwrap_or_default();
            return self.action = Some(match self.threads.iter().find(|t| t.conversation == ON_SUMMARY).and_then(|t| t.claude.clone()) {
                Some(k) => Action::ClaudeReply(k, "", prefill),
                None => Action::AskConversation { url: ON_SUMMARY.into(), prefill, want: "" },
            });
        }
        // On lines, say what you want of Claude; under a conversation comment it's a question.
        if let Some((t, k)) = self.claude_at_cursor() {
            let reply = Action::ClaudeReply(k, "", sel.unwrap_or_default());
            return match self.threads[t].conversation.is_empty() {
                true => self.pick_want(reply, Some(t)),
                false => self.action = Some(reply),
            };
        }
        let t = self.rows[self.head.row].thread;
        match (t, t.and_then(|t| self.root_id(t))) {
            (Some(t), Some(comment)) if self.pr.is_some() => {
                let thread = self.threads[t].id.clone();
                // (Nothing selected: the comment under the cursor, whole.)
                let quote = sel.or_else(|| self.comment_under_cursor(t).and_then(|c| self.seen_body(c)).map(quote)).unwrap_or_default();
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

/// `S` → "Close the PR": a comment review (if there's anything to say), then closing.
pub(super) const CLOSE: &str = "CLOSE";

/// What `c` says on an issue: there's no review of yours to queue it into.
const NO_REVIEW: &str = "an issue has no review to add to: C comments (quoting what's under the cursor)";
