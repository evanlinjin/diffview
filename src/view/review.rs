//! GitHub review actions: comments, replies, suggestions, edits, deletes, reviews, resolving,
//! viewed marks, links, and the pending/threads pickers.
use super::*;

/// Shown while what's on screen is from the cache.
pub(super) const STALE: &str = "showing the PR as dv last saw it; refreshing from GitHub…";

/// How often dv looks at whether the PR changed on GitHub (a 304 costs no rate limit).
const GITHUB_POLL: std::time::Duration = std::time::Duration::from_secs(5);

/// The PR's review threads and its info, as `fetch_github` got them.
pub(super) type Fetched = (Result<gh::Review>, Result<gh::Info>);

/// Both at once (each is a ~1s GitHub round-trip), kept in `cache` when both came.
fn fetch_github(n: u64, cache: Option<&Path>) -> Fetched {
    let (threads, info) = std::thread::scope(|s| {
        let info = s.spawn(|| gh::info_json(n));
        (gh::threads_json(n), info.join().unwrap_or_else(|_| Err(anyhow!("panicked"))))
    });
    if let (Some(p), Ok(t), Ok(i)) = (cache, &threads, &info) {
        // Not keeping it only costs the next start its head start.
        _ = p.parent().map(std::fs::create_dir_all);
        _ = crate::claude::write(p, &json!({ "threads": t, "info": i }));
    }
    (threads.map(|t| gh::parse_threads(&t)), info.map(|i| gh::parse_info(&i)))
}

/// Where the PR's last answers from GitHub are kept, to show at once next time.
fn cache_path(n: u64) -> Option<PathBuf> {
    Some(crate::claude::dv_dir().ok()?.join("cache").join(format!("pr-{n}.json")))
}

/// For `dv list`: PR `n`'s comments and info from GitHub, kept for its first opening if
/// nothing is (else they show at once already, fetched afresh as they do).
pub(crate) fn prefetch_github(n: u64) {
    if let Some(p) = cache_path(n).filter(|p| !p.exists()) {
        _ = fetch_github(n, Some(&p));
    }
}

/// What `fetch_github` kept, if anything.
pub(super) fn cached_github(p: &Path) -> Option<Fetched> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()?;
    Some((Ok(gh::parse_threads(&v["threads"])), Ok(gh::parse_info(&v["info"]))))
}

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
                gh::close(&pr).context("the review went in, but closing the PR failed")?;
                Ok("PR closed")
            }
        }
    }

    /// `Space g` → update the checkout: on a PR, up to its head as `gh pr checkout` does (fast-forward only: a branch
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

    /// Whether file `path` has comments you haven't seen.
    fn new_on(&self, path: &str) -> bool {
        self.threads.iter().any(|t| t.path == path && self.is_new(t))
    }

    /// The files with comments you haven't seen.
    fn new_paths(&self) -> HashSet<String> {
        self.threads.iter().filter(|t| self.is_new(t)).map(|t| t.path.clone()).collect()
    }

    /// New comments came (GitHub's or Claude's): the viewed files they're on unfold, so
    /// they're seen; not those that had some already (`before`: folded since, by you).
    fn unfold_new(&mut self, before: &HashSet<String>) {
        let open: Vec<String> = self.collapsed.iter().filter(|p| self.viewed.get(*p).is_some_and(|s| s == "VIEWED") && self.new_on(p) && !before.contains(*p)).cloned().collect();
        for p in open {
            self.collapsed.remove(&p);
        }
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

    /// `S` on a replacement: publish it as a draft PR (ready for review once CI passes: `Space g`
    /// checks), then close what it replaces.
    fn replacement_menu(&mut self) {
        let Some(r) = &self.replacement else { return };
        let item = |label: String, target| Item::new(label, target);
        let open: Vec<String> = r.replaces.iter().filter(|o| !r.closed.contains(&o.number)).map(|o| format!("#{}", o.number)).collect();
        let dirty = !diff::git(&["-C", &self.root.to_string_lossy(), "status", "--porcelain"]).unwrap_or_default().trim().is_empty();
        let to = crate::replace::remote().map_or("your remote: dv asks which".to_owned(), |r| format!("to {r}"));
        let mut items = match r.published {
            _ if r.drafting_now() => vec![item("Claude is drafting it: wait for it to finish".into(), Target::Nothing)],
            None if r.title.is_empty() => vec![item("Claude hasn't drafted it yet (Space a → Redraft)".into(), Target::Nothing)],
            // Only commits are published: what isn't committed would be left out.
            None if dirty => vec![item("Commit the uncommitted changes first (they'd be left out)…".into(), Target::GitCommit)],
            None => {
                let mut v = vec![item(format!("Publish as a draft PR (rebased on origin/{}, pushed {to})…", r.base), Target::Publish(false))];
                if !open.is_empty() {
                    v.push(item(format!("Publish, then close {} with the closing note…", open.join(", ")), Target::Publish(true)));
                }
                v
            }
            Some(n) => vec![item(format!("Published as PR #{n}"), Target::Nothing)],
        };
        match (r.published, open.is_empty()) {
            (_, true) => items.push(item("  the PRs it replaces are closed".into(), Target::Nothing)),
            (Some(_), false) => items.push(item(format!("Close {} with the closing note (now)…", open.join(", ")), Target::CloseOriginals)),
            (None, false) => {}
        }
        self.picker = Some(Picker { title: " replacement ", items, query: String::new(), sel: 0 });
    }

    /// Which remote keeps replacement branches: asked once, then kept in `dv.replaceRemote`.
    pub(super) fn ask_replace_remote(&mut self, close: bool) -> String {
        let choices = crate::replace::remote_choices();
        if choices.is_empty() {
            return "replacement branches go to a remote of your own, not origin: add one (git remote add <name> <your fork's URL>), then S again".into();
        }
        let items = choices.into_iter().map(|(n, url)| Item::new(format!("{n}  {url}"), Target::ReplaceRemote(n, close))).collect();
        self.picker = Some(Picker { title: " which remote keeps replacement branches? ", items, query: String::new(), sel: 0 });
        "dv remembers it (git config dv.replaceRemote)".into()
    }

    /// The replacement here, shown again when its record changes (Claude redrafting it).
    pub(super) fn watch_replacement(&mut self) {
        let Some(id) = self.replacement.as_ref().map(|r| r.id) else { return };
        let mtime = crate::replace::path(id).ok().and_then(|p| std::fs::metadata(p).ok()?.modified().ok());
        if mtime == self.replacement_mtime {
            return;
        }
        let first = self.replacement_mtime.is_none();
        self.replacement_mtime = mtime;
        if let (false, Ok(r)) = (first, crate::replace::load(id)) {
            let keep = self.cursor_key();
            self.show_replacement(r);
            self.rebuild(keep);
        }
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
        let gist = gh::hidden_words(&c.hidden);
        let text = self.seen_body(&c).unwrap_or(&gist);
        let preview: String = text.lines().next().unwrap_or("").chars().take(40).collect();
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
            true if self.seen_body(c).is_none() => self.msg = "it's hidden on GitHub: Tab shows it first".into(),
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

    /// The replacement checked out here (no PR), as PR info: what it replaces, and Claude's
    /// drafts of its description and closing note.
    fn load_replacement(&mut self) {
        if let Some(r) = crate::replace::current().and_then(|id| crate::replace::load(id).ok()) {
            self.show_replacement(r);
        }
    }

    pub(super) fn show_replacement(&mut self, r: crate::replace::Record) {
        let line = |heading, md, text: &str| gh::InfoLine { heading, md, text: text.into(), marks: vec![], hidden: String::new(), url: String::new(), node: String::new() };
        let mut replaces = line(false, false, "replaces ");
        for (i, o) in r.replaces.iter().enumerate() {
            let sep = if i == 0 { "" } else { ", " };
            replaces.text.push_str(sep);
            let at = replaces.text.chars().count();
            let num = format!("#{}", o.number);
            replaces.text.push_str(&format!("{num} {} (@{})", o.title, o.author));
            replaces.marks.push((at, at + num.chars().count(), gh::Mark::Link(o.url.clone())));
        }
        let mut status = vec![];
        if let Some(n) = r.published {
            let url = r.replaces.first().and_then(|o| Some(format!("{}/{n}", o.url.rsplit_once('/')?.0))).unwrap_or_default();
            let mut l = line(false, false, &format!("published as PR #{n}"));
            l.marks.push((l.text.chars().count() - format!("#{n}").len(), l.text.chars().count(), gh::Mark::Link(url)));
            status.push(l);
        }
        if !r.closed.is_empty() {
            let closed: Vec<String> = r.closed.iter().map(|n| format!("#{n}")).collect();
            status.push(line(false, false, &format!("closed {}, with the note", closed.join(", "))));
        }
        // A new PR: what you asked for, in place of what it replaces (and no closing note).
        let new = r.replaces.is_empty();
        if new {
            replaces = line(false, false, &format!("asked: {}", r.task.lines().next().unwrap_or("")));
        }
        self.info = vec![replaces];
        self.info.extend(status);
        self.info.extend([line(true, false, "description (Claude's draft)"), line(false, true, &r.body)]);
        if !new {
            self.info.extend([line(true, false, "closing note, for the PRs it replaces (Claude's draft)"), line(false, true, &r.closing_note)]);
        }
        self.pr_state = Some(if r.published.is_some() { ("PUBLISHED", "238636") } else { ("UNPUBLISHED", "6e7681") });
        self.replacement = Some(r);
    }

    fn load_github(&mut self) {
        if self.pr.is_none() {
            return self.load_replacement();
        }
        let Some(n) = self.pr.as_ref().map(|p| p.number) else { return };
        self.github_gen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.github_stale = false;
        self.apply_github(fetch_github(n, cache_path(n).as_deref()), true);
    }

    /// At start: the PR as dv last saw it, if it kept that, while `start_github_watch` fetches
    /// it afresh. Until that's in, `ensure_fresh` holds back anything that changes GitHub.
    pub(super) fn show_cached(&mut self, n: u64) -> bool {
        let Some(f) = cache_path(n).as_deref().and_then(cached_github) else { return false };
        let head = self.pr.as_ref().map(|p| p.head_oid.clone()); // just looked up; the cache's may be old
        self.github_gen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.apply_github(f, false);
        if let (Some(pr), Some(h)) = (&mut self.pr, head) {
            pr.head_oid = h;
        }
        self.github_stale = true;
        self.msg = STALE.into();
        true
    }

    /// Before changing anything on GitHub: the PR as it is now, not as cached.
    fn ensure_fresh(&mut self) {
        if self.github_stale {
            let keep = self.cursor_key();
            self.load_threads(); // the background fetch, when it comes, is older: dropped
            self.rebuild(keep);
        }
    }

    /// Every few seconds, a look at the PR's ETag (free while nothing has changed); when it changes,
    /// its threads and info are fetched in the background, for `watch_github` to show.
    /// `fetch_now`: once at the start too (what's shown came from the cache).
    pub(super) fn start_github_watch(&mut self, n: u64, fetch_now: bool) {
        let (tx, rx) = std::sync::mpsc::channel();
        let loads = self.github_gen.clone();
        let (poke, poked) = std::sync::mpsc::channel();
        self.github_poke = Some(poke);
        let cache = cache_path(n);
        std::thread::spawn(move || {
            let mut etag = gh::pr_etag(n, "").ok().flatten().unwrap_or_default();
            let at = loads.load(std::sync::atomic::Ordering::SeqCst);
            if fetch_now && tx.send((at, fetch_github(n, cache.as_deref()))).is_err() {
                return;
            }
            loop {
                // Every so often, or at once when poked (CI rerun: its checks change, not the
                // PR's ETag: fetched whatever it says; the ETag, conditionally asked first, so a
                // change since doesn't fetch again).
                match poked.recv_timeout(GITHUB_POLL) {
                    Ok(()) => {
                        if let Ok(Some(e)) = gh::pr_etag(n, &etag) {
                            etag = e;
                        }
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        let Ok(Some(e)) = gh::pr_etag(n, &etag) else { continue };
                        etag = e;
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return, // dv is done
                }
                let at = loads.load(std::sync::atomic::Ordering::SeqCst);
                if tx.send((at, fetch_github(n, cache.as_deref()))).is_err() {
                    return; // dv is done
                }
            }
        });
        self.github = Some(rx);
    }

    /// Shows what the background look at GitHub fetched, once nothing is in the middle of
    /// something (a picker's or selection's rows would move under it).
    /// dv has just changed the PR on GitHub (pushed, posted its description, …): what a fetch
    /// under way brings back is from before, and would undo what dv shows now. The next one,
    /// after it, is current.
    pub(super) fn github_changed(&mut self) {
        self.github_gen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }

    pub(super) fn watch_github(&mut self) {
        let busy = self.action.is_some() || self.job.is_some() || self.picker.is_some() || self.prompt.is_some();
        if busy || self.pending.is_some() || self.select || !self.labels.is_empty() {
            return;
        }
        let Some((at, f)) = self.github.as_ref().and_then(|rx| rx.try_iter().last()) else { return };
        if at != self.github_gen.load(std::sync::atomic::Ordering::SeqCst) {
            return; // loaded since: that's newer
        }
        let count = |a: &App| a.threads.iter().filter(|t| t.claude.is_none()).map(|t| t.comments.len()).sum::<usize>();
        let before = count(self);
        let had_new = self.new_paths();
        let keep = self.cursor_key();
        let claude: Vec<Thread> = std::mem::take(&mut self.threads).into_iter().filter(|t| t.claude.is_some()).collect();
        self.apply_github(f, true);
        self.threads.extend(claude);
        self.shape();
        let new = count(self).saturating_sub(before);
        self.unfold_new(&had_new);
        self.rebuild(keep);
        if std::mem::take(&mut self.github_stale) && self.msg == STALE {
            self.msg.clear();
        }
        if new > 0 && self.msg.is_empty() {
            self.msg = format!("{new} new comment(s) on GitHub");
        }
    }

    /// (`fresh`: just fetched, not the cache's: only then does a retarget count.)
    fn apply_github(&mut self, (threads, info): Fetched, fresh: bool) {
        match threads {
            Ok(r) => {
                // A draft GitHub couldn't take (see `Post::Summary`) stays until it can.
                if r.pending.is_none() || !r.pending_body.is_empty() {
                    self.summary_draft = r.pending_body;
                }
                (self.threads, self.review) = (r.threads, r.pending);
                (self.theirs, self.me) = (!r.mine, r.me); // (before: your own comments aren't new)
                self.apply_viewed(r.viewed); // after the threads: not folded with new ones
                if let Some(pr) = &mut self.pr {
                    pr.node_id = r.pr_id;
                }
            }
            Err(e) => self.msg = format!("loading comments: {e:#}"),
        }
        match info {
            Ok(gh::Info { lines, title, head, url, state, unsigned, awaiting, failing, base }) => {
                (self.awaiting, self.failing) = (awaiting, failing);
                (self.info, self.pr_state) = (lines, Some(state));
                // GitHub's word on its commits' signatures, for the contents' commits (none this
                // time: what was known stands).
                if let Some(unsigned) = unsigned {
                    for c in &mut self.commits {
                        c.signed = !unsigned.contains(&c.sha);
                    }
                    self.unsigned = unsigned;
                }
                if let Some(pr) = &mut self.pr {
                    (pr.title, pr.head_oid, pr.url) = (title, head, url);
                    if fresh && !base.is_empty() {
                        self.github_base = Some(base);
                    }
                }
                self.refresh_sync(); // the PR's head may have moved
            }
            Err(e) => self.msg = format!("loading PR info: {e:#}"),
        }
        self.follow_github_base(true);
    }

    /// The PR's base on GitHub isn't the one here (retargeted): the diff starts from the new
    /// one, once fetched (a job: a frame says so first). Mid-rebase, a diff that follows the base
    /// waits till it's through (`say`: say so, as a fetch shows it); looked at again on each
    /// reload (the rebase's end is one). Cheap while the bases agree.
    pub(super) fn follow_github_base(&mut self, say: bool) {
        let Some(base) = self.github_base.clone().filter(|b| self.pr.as_ref().is_some_and(|p| p.base != *b)) else { return };
        if self.job.is_some() {
            return;
        }
        let follows = self.pr.as_ref().is_some_and(|p| self.follow_base.as_deref() == Some(format!("origin/{}", p.base).as_str()));
        if follows && super::rebase::rebasing(&self.root) {
            if say {
                self.base_note(&format!("the PR's base is {base} now: the diff follows once the rebase is through"));
            }
            return;
        }
        self.base_note(&format!("the PR's base is {base} now: fetching it…"));
        self.job = Some(Job::Retarget(base));
    }

    /// Say `note` about the PR's base after what's said, in place of an earlier one about it
    /// (and of the "as dv last saw it" notice: this is fresh): not piled up, not over a
    /// rebase's ending, say.
    fn base_note(&mut self, note: &str) {
        let last = std::mem::replace(&mut self.base_said, note.to_owned());
        self.msg = base_note_in(&self.msg, &last, note);
    }

    /// The PR's base changed on GitHub to `base`: a rebase goes onto it, and the diff starts
    /// from it (fetched), when dv chose the diff's base (no `--base` given).
    fn retarget(&mut self, base: String) {
        let Some(old) = self.pr.as_ref().map(|p| p.base.clone()) else { return };
        let follows = self.follow_base.as_deref() == Some(format!("origin/{old}").as_str());
        let onto = format!("origin/{base}");
        if follows {
            let fetched = diff::git_in(&self.root, &["fetch", "-q", "origin", &format!("+refs/heads/{base}:refs/remotes/{onto}")]);
            let Ok(mb) = fetched.and_then(|_| diff::git_in(&self.root, &["merge-base", &onto, "HEAD"])) else {
                self.github_base = None; // (asked again when GitHub next says)
                let said = format!("the PR's base is {base} now, but {onto} couldn't be fetched: the diff still starts from {old} (tried again on the next refresh)");
                return self.base_note(&said);
            };
            let mb = mb.trim().to_owned();
            if !self.uncommitted() {
                self.base = mb.clone();
            }
            (self.full_base, self.follow_base) = (mb, Some(onto));
            (self.to, self.commit_shown, self.commit_anchor) = (None, None, None);
            self.hl_cache.retain(|k, _| !k.1);
        }
        if let Some(pr) = &mut self.pr {
            pr.base = base.clone();
        }
        self.reload_tree();
        let said = match follows {
            true => format!("the PR's base is {base} now: the diff starts from there"),
            false => format!("the PR's base is {base} now (the diff keeps the base you gave)"),
        };
        // After what was said before (a rebase's ending, say), not over it.
        self.base_note(&said);
    }

    /// The link under the cursor, if any.
    pub(super) fn link_at_cursor(&self) -> Option<&str> {
        let col = self.head.col;
        self.rows[self.head.row].links.iter().find(|(a, b, _)| *a <= col && col < *b).map(|l| l.2.as_str())
    }

    /// `gf`: the link under the cursor in the browser, or the `file:line` under it (as comments
    /// cite code: `src/a.rs:12`, `a.rs:12-15`, `a.rs#L12`) in dv, or in hx if it isn't shown;
    /// or, on a name (`next_returns_none`), where it's defined.
    pub(super) fn open_link(&mut self) {
        // On a GitHub comment's author line (a review thread's, or the conversation's): that
        // comment on GitHub. In its text: what's under the cursor, else the comment too.
        let row = &self.rows[self.head.row];
        // (Not other headings there: Claude's review, your pending one: nothing on GitHub.)
        let has_link = row.thread.is_some_and(|t| self.threads[t].claude.is_none())
            || row.file == INFO && row.info.and_then(|i| self.info.get(i.checked_sub(1)?)).is_some_and(|h| h.heading && !h.url.is_empty());
        let comment = self.cursor_ref().filter(|u| has_link && u.starts_with("https://"));
        let on_author = row.line.kind == Kind::Thread;
        let url = self.link_at_cursor().map(str::to_owned).or_else(|| comment.clone().filter(|_| on_author));
        let Some(url) = url else {
            let text = &self.rows[self.head.row].line.text;
            // After `w` the cursor is on the space after the word: the selection's start is on it.
            let on_space = text.chars().nth(self.head.col).is_none_or(char::is_whitespace);
            let col = if on_space && self.anchor.row == self.head.row { self.anchor.col } else { self.head.col };
            if let Some((path, line)) = file_ref(text, col) {
                return self.goto_file_line(&path, line);
            }
            return match (self.name_at_cursor(), comment) {
                (Some(name), _) => self.goto_definition(&name),
                (None, Some(u)) => self.open_url(&u),
                (None, None) => self.msg = "no link, file:line or name under the cursor".into(),
            };
        };
        self.open_url(&url);
    }

    fn open_url(&mut self, url: &str) {
        if cfg!(test) {
            return self.msg = format!("opened {url}"); // (no browser from tests)
        }
        let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
        let spawned = std::process::Command::new(opener)
            .arg(url)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        self.msg = match spawned {
            Ok(_) => format!("opened {url}"),
            Err(e) => format!("{opener}: {e}"),
        };
    }

    /// Go to `line` of `path` (as a comment wrote it: from the repo root, or just its end, like
    /// `view.rs`): in the diff when it's shown there, else in hx.
    fn goto_file_line(&mut self, path: &str, line: u32) {
        let path = path.trim_start_matches("./");
        let ends = |p: &str| p == path || p.ends_with(&format!("/{path}"));
        let found = match self.root.join(path).is_file() {
            true => Some(path.to_owned()),
            false => self.files.iter().map(|f| f.path.clone()).find(|p| ends(p)).or_else(|| {
                let all = diff::git(&["ls-files"]).unwrap_or_default();
                let mut hits = all.lines().filter(|p| ends(p));
                // Only if it's unambiguous.
                match (hits.next(), hits.next()) {
                    (Some(p), None) => Some(p.to_owned()),
                    _ => None,
                }
            }),
        };
        let Some(rel) = found else { return self.msg = format!("no file {path} here") };
        // A folded file opens, so the line can show.
        if self.collapsed.remove(&rel) {
            let keep = self.cursor_key();
            self.rebuild(keep);
        }
        self.jump_loc(&crate::lsp::Loc { path: self.root.join(&rel), line: line.saturating_sub(1), character: 0 });
    }

    /// Where `name` is defined (by a grep for `fn name`, `class name` and the like, in the files
    /// on disk), the diff's files first: there, or a picker of them.
    /// ponytail: a grep, not the language server; good enough for names in comments.
    /// The name under the cursor in text (a comment, the PR's): in `code`, or one that looks
    /// like code (`snake_case`, `CamelCase`, `a::b`).
    pub(super) fn name_at_cursor(&self) -> Option<String> {
        let text = &self.rows[self.head.row].line.text;
        // After `w` the cursor is on the space after the word: the selection's start is on it.
        let on_space = text.chars().nth(self.head.col).is_none_or(char::is_whitespace);
        let col = if on_space && self.anchor.row == self.head.row { self.anchor.col } else { self.head.col };
        // Code's color (it may be bold too, in bold text or a heading).
        let raw = self.ui.md.raw.fg;
        let in_code = raw.is_some() && self.rows[self.head.row].syn.get(col).is_some_and(|s| s.and_then(|s| s.fg) == raw);
        ident_at(text, col, in_code)
    }

    pub(super) fn goto_definition(&mut self, name: &str) {
        // POSIX classes, not \b / \s: those are GNU-only.
        let pat = format!("(^|[^[:alnum:]_])({DEF_WORDS})[[:space:]]+{name}([^[:alnum:]_]|$)");
        let root = self.root.display().to_string();
        // -z: path NUL line NUL text, the path unquoted (it may hold `:` or non-ASCII).
        let out = diff::git(&["-C", &root, "grep", "-n", "-z", "-I", "--untracked", "-E", &pat]).unwrap_or_default();
        let mut locs: Vec<crate::lsp::Loc> = out
            .lines()
            .filter_map(|l| {
                let mut parts = l.splitn(3, '\0');
                let (path, line, text) = (parts.next()?, parts.next()?.parse::<u32>().ok()?, parts.next()?);
                // A UTF-16 column, as LSP's.
                let character = text.find(name).map_or(0, |i| text[..i].encode_utf16().count()) as u32;
                Some(crate::lsp::Loc { path: self.root.join(path), line: line - 1, character })
            })
            .collect();
        let in_diff: HashSet<PathBuf> = self.files.iter().map(|f| self.root.join(&f.path)).collect();
        locs.sort_by_key(|l| !in_diff.contains(&l.path));
        locs.truncate(50);
        if locs.is_empty() {
            return self.msg = format!("no definition of {name} found");
        }
        self.show_locations(locs);
    }

    /// The GitHub comment (or review) under the cursor, to react to: its node id. A review
    /// thread's comment, or one in the PR's conversation (its heading or text).
    fn reactable_at_cursor(&self) -> Option<String> {
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
    fn labels_menu(&mut self) {
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

    fn react_menu(&mut self, node: &str) {
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
    fn thread_items(&self, keep: impl Fn(&Thread) -> bool) -> Vec<Item> {
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
                Some(Item { detail: self.snippet(&c.gist(), 2), ..Item::new(label, Target::Comment(ti, 0)) })
            })
            .collect()
    }

    /// Each comment of your pending review (replies too).
    fn pending_items(&self) -> Vec<Item> {
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
                items.push(Item { detail: self.snippet(&c.gist(), 2), ..item });
            }
        }
        items
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

    /// `]c` / `[c`: the next / previous thread, into folded files too (a viewed file's comments).
    pub(super) fn next_thread(&mut self, fwd: bool) {
        let items = (0..self.threads.len()).filter(|&t| self.in_view(&self.threads[t])).map(|t| (t, 0)).collect();
        self.next_comment(fwd, items, "no comments");
    }

    /// `]u` / `[u`: the next / previous unresolved thread (GitHub's or Claude's; outdated ones too,
    /// which still want resolving).
    pub(super) fn next_unresolved(&mut self, fwd: bool) {
        let items = (0..self.threads.len()).filter(|&t| !self.threads[t].resolved).map(|t| (t, 0)).collect();
        self.next_comment(fwd, items, "no unresolved comments");
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
            (true, _) if review && self.hidden_line(self.head.row) => self.msg = "it's hidden on GitHub: Tab shows it first".into(),
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
        // (Not what's hidden on GitHub, unseen.)
        let text = row.info.and_then(|i| self.info.get(i)).map(|l| l.text.as_str()).filter(|_| !self.hidden_line(self.head.row));
        let quote = text.map_or(String::new(), quote);
        self.action = Some(Action::Converse { quote, as_is: false });
    }

    /// `c` in the PR section: the comment (or description) under the cursor, quoted at the end
    /// of your pending review's text.
    fn quote_into_summary(&mut self) {
        let row = &self.rows[self.head.row];
        match row.info.and_then(|i| self.info.get(i)).filter(|l| l.md && !l.text.trim().is_empty()) {
            Some(l) if self.pr.is_some() => {
                let q = quote(&l.text);
                self.add_to_summary(q.trim_end());
            }
            _ => self.msg = "c here: on a comment or the description, quotes it in your review summary".into(),
        }
    }

    /// Appends `text` to your pending review's text, on GitHub (starting the review if need be).
    fn add_to_summary(&mut self, text: &str) {
        let draft = self.summary_draft.trim_end();
        self.summary_draft = if draft.is_empty() { text.to_owned() } else { format!("{draft}\n\n{text}") };
        self.msg = self.post_later(Post::Summary(self.summary_draft.clone())).unwrap_or_else(|e| format!("{e:#}"));
    }

    /// `c` / `C` on Claude's review (its summary), as on its comments: yours to use, not to
    /// quote. `c` puts it in your review summary (`S` starts from it); `C` posts it as your PR
    /// comment, after the editor.
    fn promote_summary(&mut self, review: bool) {
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

/// How long the cursor stays on a thread before its comments count as seen.
const SEEN_AFTER: std::time::Duration = std::time::Duration::from_secs(3);

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
        self.seen.path = self.claude.path.as_ref().and_then(|p| Some(p.parent()?.parent()?.join("seen").join(p.file_name()?)));
        let text = self.seen.path.as_ref().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
        self.seen.keys = serde_json::from_str(&text).unwrap_or_default();
        self.seen.before = self.seen.keys.clone();
    }

    /// Thread `t` has comments you haven't seen, not counting your own. (Never, with nowhere
    /// to keep what you've seen.)
    pub(super) fn is_new(&self, t: &Thread) -> bool {
        self.seen.path.is_some()
            && t.comments.iter().enumerate().any(|(ci, c)| !self.wrote(c) && !self.seen.keys.contains(&comment_key(t, ci)))
    }

    fn wrote(&self, c: &gh::Comment) -> bool {
        c.pending || c.author == "you" || (!self.me.is_empty() && c.author == self.me)
    }

    /// A thread's comments are seen once the cursor leaves it (as they were when it got
    /// there), or has been on it for `SEEN_AFTER`.
    pub(super) fn watch_seen(&mut self) {
        if self.select {
            return; // the rebuild would drop the selection: later
        }
        let here = self.rows.get(self.head.row).and_then(|r| r.thread).map(|ti| {
            let t = &self.threads[ti];
            (0..t.comments.len()).map(|ci| comment_key(t, ci)).collect::<Vec<_>>()
        });
        // A conversation comment (its heading or text): its node id.
        let on_thread = here.is_some();
        let here = here.or_else(|| self.conversation_node().map(|n| vec![n])).unwrap_or_default();
        let (seen, conversation) = match here == self.seen.current {
            true if self.seen.since.elapsed() >= SEEN_AFTER => (self.seen.current.clone(), self.seen.conversation),
            true => return,
            false => {
                self.seen.since = std::time::Instant::now();
                let was = std::mem::replace(&mut self.seen.conversation, !on_thread);
                (std::mem::replace(&mut self.seen.current, here), was)
            }
        };
        let before = self.seen.keys.len();
        self.seen.keys.extend(seen.into_iter().filter(|k| !k.is_empty()));
        if self.seen.keys.len() != before
            && let Some(p) = &self.seen.path
        {
            let mut keys: Vec<&String> = self.seen.keys.iter().collect();
            keys.sort();
            // ponytail: rewrites the whole set; fine for a PR's worth of comments.
            _ = std::fs::create_dir_all(p.parent().unwrap_or(p)).and_then(|_| std::fs::write(p, serde_json::to_string(&keys).unwrap_or_default()));
            if !conversation {
                let keep = self.cursor_key();
                self.rebuild(keep); // without its [new] tag
            }
        }
    }
}

/// `S` → "Comment and close the PR": a comment review (if there's anything to say), then closing.
pub(super) const CLOSE: &str = "CLOSE";

/// The `path:line` reference in `text` around char `col`, if there is one: `src/a.rs:12`,
/// `a.rs:12-15`, `a.rs:12:5`, `a.rs#L12` (a trailing `.`/`,`/`:` is punctuation).
fn file_ref(text: &str, col: usize) -> Option<(String, u32)> {
    let chars: Vec<char> = text.chars().collect();
    let ok = |c: char| c.is_alphanumeric() || "_./-:#".contains(c);
    if !chars.get(col).is_some_and(|&c| ok(c)) {
        return None;
    }
    let a = (0..col).rev().take_while(|&i| ok(chars[i])).last().unwrap_or(col);
    let b = (col..chars.len()).take_while(|&i| ok(chars[i])).last()? + 1;
    let token: String = chars[a..b].iter().collect();
    let token = token.trim_end_matches(['.', ',', ':', '-']);
    let (path, rest) = token.split_once("#L").or_else(|| token.split_once(':'))?;
    let line: u32 = rest.split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()?;
    // A file: it has an extension or a directory (not "note:3" or a time "12:30").
    let file = path.rsplit('/').next().is_some_and(|f| f.contains('.') && !f.starts_with('.') || path.contains('/'));
    (file && line > 0 && !path.chars().all(|c| c.is_ascii_digit())).then(|| (path.to_owned(), line))
}

/// The words a definition starts with, across the usual languages.
const DEF_WORDS: &str = "fn|struct|enum|trait|type|union|mod|macro_rules!|const|static|def|class|function|func|interface";

/// The name under char `col` of `text`, if it's code: `in_code` (a `code` span), or a word
/// with `_`, a capital after its start, or a digit (`next_returns_none`, `parseArgs`), not a
/// plain English word.
fn ident_at(text: &str, col: usize, in_code: bool) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let ok = |c: char| c.is_alphanumeric() || c == '_';
    if !chars.get(col).is_some_and(|&c| ok(c)) {
        return None;
    }
    let a = (0..col).rev().take_while(|&i| ok(chars[i])).last().unwrap_or(col);
    let b = (col..chars.len()).take_while(|&i| ok(chars[i])).last()? + 1;
    let w: String = chars[a..b].iter().collect();
    let code = in_code || w.contains('_') || w.chars().skip(1).any(char::is_uppercase) || w.chars().any(|c| c.is_ascii_digit());
    (code && !w.starts_with(|c: char| c.is_ascii_digit())).then_some(w)
}

#[test]
fn idents() {
    let at = |text: &str, needle: &str| ident_at(text, text.find(needle).unwrap(), false);
    let t = "half of next_returns_none_after_error, so inline parseArgs or Foo::bar.";
    assert_eq!(at(t, "none").as_deref(), Some("next_returns_none_after_error")); // anywhere on it
    assert_eq!(at(t, "parse").as_deref(), Some("parseArgs"));
    assert_eq!(at(t, "half"), None); // a word
    assert_eq!(at(t, "Foo"), None); // could be anything: ask by its method
    assert_eq!(at(t, " so"), None);
    assert_eq!(ident_at(t, t.find("bar").unwrap(), true).as_deref(), Some("bar")); // in `code`: any name
}

#[test]
fn file_refs() {
    let at = |text: &str, needle: &str| file_ref(text, text.find(needle).unwrap());
    let t = "see src/view/mod.rs:1213 and a.rs:12-15, also b.rs#L7.";
    assert_eq!(at(t, "mod.rs"), Some(("src/view/mod.rs".into(), 1213)));
    assert_eq!(at(t, "1213"), Some(("src/view/mod.rs".into(), 1213))); // anywhere on it
    assert_eq!(at(t, "a.rs"), Some(("a.rs".into(), 12)));
    assert_eq!(at(t, "b.rs"), Some(("b.rs".into(), 7)));
    assert_eq!(at(t, "see"), None);
    assert_eq!(at("at 12:30 today", "12"), None); // a time
    assert_eq!(at("note: this", "note"), None);
    assert_eq!(at("x.rs:12:5,", "x.rs"), Some(("x.rs".into(), 12)));
}

/// `msg` with `note` said after it, in place of `last` (the note before) and of the "as dv
/// last saw it" notice.
fn base_note_in(msg: &str, last: &str, note: &str) -> String {
    let keep = msg.split(" · ").filter(|p| !p.is_empty() && *p != last && *p != STALE);
    keep.chain([note]).collect::<Vec<_>>().join(" · ")
}

#[test]
fn notes_on_the_base() {
    let a = base_note_in(&format!("rebased · {STALE}"), "", "the base is x now: fetching it…");
    assert_eq!(a, "rebased · the base is x now: fetching it…");
    assert_eq!(base_note_in(&a, "the base is x now: fetching it…", "the base is x now"), "rebased · the base is x now");
}
