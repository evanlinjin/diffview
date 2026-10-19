//! `Space a`: run Claude Code on this review (the dv-review skill), without leaving dv. Inside
//! zellij it gets a floating pane (an interactive session to watch and steer); elsewhere it runs
//! in the background. Either way its review shows up as it writes the file.
use super::*;
use crate::claude::{answer_prompt, claude_command, claude_dirs, data_env, fix_checks_prompt, path_for_claude, review_prompt, tools_for, update_prompt, with_instructions};

#[derive(Clone, Copy)]
pub(super) enum Ask {
    Review,
    Answer,
    /// Make the changes the review's open comments ask for (Claude's and yours), and/or what
    /// your instructions say.
    Fix,
    /// Propose a new description for the PR (in the review file), for you to approve.
    Describe,
    /// On someone else's PR: make the changes (as `Fix` does on yours), here, for you to push
    /// to their branch, rather than suggest them.
    Edit,
    /// Your PR's CI fails: find out why from the failed runs' logs, and fix it here.
    FixChecks,
    /// Draft a comment closing the PR (in the review file), for you to close it with.
    Close,
}

/// A background `claude -p` run.
pub(super) struct ClaudeJob {
    pub(super) child: std::process::Child,
    /// Its result (JSON); errors go next to it (`.err`).
    pub(super) log: PathBuf,
    /// What it's doing, for the status bar.
    pub(super) label: &'static str,
    /// It ran on a fork of the PR's session (the session itself is open in a pane), so its
    /// session id isn't the one to keep.
    pub(super) forked: bool,
    pub(super) started: std::time::Instant,
}

/// On a thread waiting for Claude: what you want of it (`[change]` …), when you said.
pub(crate) fn want_tag(t: &Thread) -> Option<&'static str> {
    let tag = ask::WANTS.iter().find(|w| w.0 == t.want)?.1;
    t.waiting().then_some(tag)
}

/// Whose turn it is on an open thread of the review file, as its tag: `[waiting]` (Claude's),
/// `[answered]` (yours).
pub(super) fn turn(t: &Thread) -> Option<&'static str> {
    match (t.waiting(), t.answered()) {
        (true, _) => Some("[waiting] "),
        (_, true) => Some("[answered] "),
        _ => None,
    }
}

/// Claude's review for this PR/branch, and the background `claude -p` runs that write it.
#[derive(Default)]
pub(super) struct ClaudeState {
    /// Its review file (see `claude.rs`), and when that last changed.
    pub(super) path: Option<PathBuf>,
    pub(super) mtime: Option<std::time::SystemTime>,
    pub(super) summary: String,
    /// The commit the review was made against (its `head`).
    pub(super) head: String,
    /// A new description for the PR that Claude proposes (empty: none).
    pub(super) description: String,
    /// A comment Claude drafted for closing the PR (empty: none).
    pub(super) closing: String,
    /// A background `claude -p` run: (process, log file, what it's doing).
    pub(super) job: Option<agent::ClaudeJob>,
    /// The Claude Code session of the last background run, resumed by the next.
    pub(super) session: Option<String>,
    /// A reply came in while Claude was busy: answer again when it's done.
    pub(super) again: bool,
    /// Background runs so far (each gets its own log).
    pub(super) runs: u32,
    /// The last background run's log (its result, as JSON).
    pub(super) log: Option<PathBuf>,
    /// What background runs have cost so far (as Claude Code reports it, in USD).
    pub(super) spent: f64,
    /// When Claude's pane was last looked at, and whether it was asking you something then.
    pub(super) pane_check: Option<std::time::Instant>,
    pub(super) asking: bool,
    /// Its pane shows it working (Claude Code's "esc to interrupt").
    pub(super) pane_busy: bool,
}

impl App {
    /// Whether `ask` may change the code (or reads it whole: a review): answering does when a
    /// reply waiting wants a change.
    pub(super) fn edits(&self, ask: Ask) -> bool {
        match ask {
            Ask::Describe | Ask::Close => false,
            Ask::Answer => self.threads.iter().any(|t| t.waiting() && t.want == "change"),
            _ => true,
        }
    }

    /// Replies of yours that Claude hasn't answered yet.
    pub(super) fn waiting_replies(&self) -> usize {
        self.threads.iter().filter(|t| t.waiting()).count()
    }

    /// The commit Claude last reviewed, if there's been more since that build on it (a push,
    /// your commits): then an update need only look at those. Not after a rebase or force-push:
    /// that's all new.
    pub(super) fn reviewed_before(&self) -> Option<&str> {
        let old = self.claude.head.as_str();
        let head = diff::git(&["rev-parse", "HEAD"]).ok()?;
        let newer = !old.is_empty() && !head.trim().starts_with(old);
        (newer && diff::git(&["merge-base", "--is-ancestor", old, "HEAD"]).is_ok()).then_some(old)
    }

    /// What "make the changes" acts on: the review file's open comments, Claude's and the ones
    /// you left it (`a`), not resolved or outdated. The one place that's decided.
    pub(super) fn change_requests(&self) -> usize {
        self.threads.iter().filter(|t| t.claude.is_some() && !t.stale() && (t.by_claude() || !matches!(t.want.as_str(), "ask" | "suggest"))).count()
    }

    /// Why Claude can't take a request right now (checked before asking you for instructions,
    /// so nothing you write is lost to it): still busy, or a rebase stopped (`pane_cant`: where
    /// it goes to the pane). `ask`: what's asked (`None`: amending, which changes the code but needs no review file).
    /// What changes the code (or reads it whole: a review) waits for a stopped rebase, whose
    /// conflicts are in the way; a question, a description, a closing comment don't.
    pub(super) fn claude_blocked(&self, ask: Option<Ask>) -> Option<String> {
        let edits = ask.is_none_or(|a| self.edits(a));
        if self.claude_busy() {
            return Some("Claude is still working on the last request".into());
        }
        // A rebase stopped part way: the tree has its conflicts, HEAD is detached.
        if edits && super::rebase::rebasing(&self.root) {
            return Some(super::rebase::stopped_says(&self.root).into());
        }
        None
    }

    /// Claude is at work in this checkout: a run of this dv's, or (a replacement) its redraft,
    /// from any dv. Nothing here rewrites the branch or commits under it meanwhile.
    pub(super) fn claude_busy(&self) -> bool {
        self.claude.job.is_some() || self.replacement.as_ref().is_some_and(|r| r.drafting_now())
    }

    /// Why `ask` can't go to the PR's Claude pane, where it would (one is open): the pane is
    /// from an older dv that can't reach the review files, and `ask` needs them. (A run of its
    /// own, as answering is, reaches them whatever the pane.)
    fn pane_cant(&self, ask: Ask) -> Option<String> {
        let needs_files = !matches!(ask, Ask::Fix | Ask::Edit) || self.change_requests() > 0;
        let (_, cmd) = self.claude_pane_cmd()?;
        let dv = crate::claude::dv_dir().map(|d| d.display().to_string()).unwrap_or_default();
        (needs_files && !dv.is_empty() && !cmd.contains(&dv)).then(|| {
            "Claude's pane for this PR can't reach dv's review files (it's from an older dv): close it, and Space a starts a new one".into()
        })
    }

    pub(super) fn claude_menu(&mut self) {
        if self.claude.path.is_none() {
            return self.msg = "no review file here (not in a git repo?)".into();
        }
        let has_review = self.threads.iter().any(Thread::by_claude) || !self.claude.summary.is_empty();
        let review = match (has_review, self.reviewed_before()) {
            (true, Some(old)) => format!("Update Claude's review: what changed since it reviewed {}", &old[..old.len().min(8)]),
            (true, None) => "Update Claude's review (keeps comments that still apply, and your replies)".into(),
            (false, _) => "Ask Claude to review this".into(),
        };
        let n = self.waiting_replies();
        let open = self.change_requests();
        let notes = match crate::claude::notes_path().is_ok_and(|p| p.exists()) {
            true => "Edit Claude's notes on this repo (it reads them before each task)",
            false => "Write notes for Claude on this repo (it reads them before each task)",
        };
        let mut items = vec![
            Item::new(review, Target::Claude(Ask::Review)),
            Item::new(format!("Ask Claude to answer your replies ({n} waiting)"), Target::Claude(Ask::Answer)),
            Item::new(
                match (open, self.theirs) {
                    (0, false) => "Ask Claude to make a change (no open comments: you say what)".to_owned(),
                    (n, false) => format!("Ask Claude to make the changes ({n} open comments)"),
                    // Someone else's PR: as suggestions you can post, not edits.
                    (0, true) => "Ask Claude to suggest a change (you say what; their PR: no edits)".to_owned(),
                    (n, true) => format!("Ask Claude to suggest the changes ({n} open comments; their PR: no edits)"),
                },
                Target::Claude(Ask::Fix),
            ),
        ];
        // Someone else's PR: Claude can make the changes all the same, here, for you to push to
        // their branch (when they let maintainers: else they stay here).
        if self.theirs {
            let push = match self.pr.as_ref().is_some_and(|p| p.maintainer_can_modify || !p.cross_repo) {
                true => "Space g pushes them to their branch",
                false => "they don't allow maintainer edits: the commits stay here",
            };
            let what = if open == 0 { "a change (you say what)".to_owned() } else { format!("the changes ({open} open comments)") };
            items.push(Item::new(format!("Ask Claude to make {what} here, as commits ({push})"), Target::Claude(Ask::Edit)));
        }
        // Your PR's CI failing: Claude finds out why; or found it isn't the PR's doing, and had
        // a fix of its own in mind, a new PR's.
        if let Some(n) = self.pr.as_ref().map(|p| p.number).filter(|_| self.failing > 0 && !self.theirs) {
            let s = if self.failing == 1 { "" } else { "s" };
            items.push(Item::new(format!("Ask Claude to fix the failing CI ({} check{s}: it reads the failed runs' logs; you push)", self.failing), Target::Claude(Ask::FixChecks)));
            if let Some(c) = self.ci_note(n).filter(|c| !c.task.is_empty()) {
                items.insert(0, Item::new(format!("Have Claude draft a PR fixing CI (not this PR's doing: {})…", c.why), Target::Do(Action::CiFixPr)));
            }
        }
        items.push(Item::new("Ask Claude a question about this PR… (it answers in the Claude tab)", Target::ClaudeQuestion));
        items.extend([
            Item::new(notes, Target::ClaudeNotes),
            Item::new("Check Claude's review now (dv watches it; this reads it again)", Target::ClaudeCheck),
        ]);
        if self.claude_pane().is_some() {
            items.insert(0, Item::new("Show Claude's pane (its session on this PR)", Target::ClaudePane));
        }
        // A description Claude proposed: yours to approve (posted then), take further, or drop.
        if !self.claude.description.is_empty() {
            items.insert(0, Item::new("Discard Claude's proposed description", Target::DescDiscard));
            items.insert(0, Item::new("Ask Claude for more changes to the description…", Target::Claude(Ask::Describe)));
            items.insert(0, Item::new("Approve Claude's description: it replaces the PR's on GitHub…", Target::DescApprove(false)));
        } else if self.pr.is_some() {
            items.push(Item::new("Ask Claude to change the PR's description (you approve it)…", Target::Claude(Ask::Describe)));
        }
        // Closing it: Claude drafts the comment; you edit it, then close with it (or don't).
        // (Closed or merged meanwhile: the draft can only go.)
        let open = self.pr_state.is_some_and(|s| matches!(s.0, "OPEN" | "DRAFT"));
        let drafted = !self.claude.closing.trim().is_empty();
        if drafted {
            items.insert(0, Item::new("Discard Claude's closing comment", Target::CloseDiscard));
        }
        if drafted && open {
            items.insert(0, Item::new("Ask Claude for changes to the closing comment…", Target::Claude(Ask::Close)));
            items.insert(0, Item::new("Close the PR with Claude's comment (your editor first)…", Target::Do(Action::ClosePr)));
        } else if self.pr.is_some() && open {
            items.push(Item::new("Ask Claude to draft a comment closing the PR (you edit it, then close)…", Target::Claude(Ask::Close)));
        }
        // One that seems stuck: it can be stopped (the session stays, so the next run resumes it).
        if let Some(j) = &self.claude.job {
            items.insert(0, Item::new(format!("Stop Claude ({} for {})", j.label, ago(j.started)), Target::ClaudeStop));
        }
        let outdated = self.outdated_claude_comments().len();
        if outdated > 0 {
            items.push(Item::new(format!("Dismiss Claude's outdated comments ({outdated})"), Target::ClaudeDismissOutdated(false)));
        }
        if self.claude.session.is_some() {
            items.push(Item::new("Start a fresh Claude session for this PR (the review stays)", Target::ClaudeFresh));
        }
        // A replacement: Claude writes the change itself, unpublished, for you to review here.
        match (&self.pr, &self.replacement) {
            (Some(p), _) => items.push(Item::new(
                format!("Draft a replacement for #{} (Claude writes it properly, unpublished, in the background)", p.number),
                Target::Do(Action::Replace(vec![p.number], None)),
            )),
            (None, Some(r)) if r.drafting_now() => items.push(Item::new("Claude is drafting this replacement (dv shows it as it goes)", Target::Nothing)),
            (None, Some(r)) => items.push(Item::new(
                "Redraft this (Claude carries on, with your instructions)",
                Target::Do(Action::Replace(r.ids(), None)),
            )),
            _ => {}
        }
        // Preselected: answering your replies if any wait, else the review (never Stop: Enter
        // out of habit mustn't end a run).
        let want = |t: &Target| matches!(t, Target::Claude(a) if matches!((a, n > 0), (Ask::Answer, true) | (Ask::Review, false)));
        let sel = items.iter().position(|i| want(&i.target)).unwrap_or(0);
        self.picker = Some(Picker { sel, ..Picker::new(" claude ", items) });
    }

    /// `Space a` → check now: the review file read again (as if it had changed), and the
    /// background run's state, rather than waiting for dv to notice.
    pub(super) fn check_claude(&mut self) {
        // Read it, changed or not (or gone: `None` would look unchanged then).
        self.claude.mtime = Some(std::time::SystemTime::UNIX_EPOCH);
        self.watch_claude();
        let ran = self.claude.job.is_some();
        self.poll_claude(); // a run that's ended says how (that's the news, then)
        if let Some(j) = &self.claude.job {
            self.msg = format!("Claude is still {} ({} so far); Space a can stop it", j.label, ago(j.started));
        } else if !ran && !self.claude.path.as_ref().is_some_and(|p| p.exists()) {
            self.msg = "Claude hasn't written a review yet (Space a → review asks for one)".into();
        }
    }

    /// `Space a` → stop: end the background run (its log stays). The session is kept, so the
    /// next run resumes where it left off.
    pub(super) fn stop_claude(&mut self) {
        let Some(mut j) = self.claude.job.take() else { return self.msg = "Claude isn't working on anything".into() };
        if let Some(p) = self.pr.as_ref().filter(|_| !j.forked) {
            crate::claude::clear_running(p.number);
        }
        // Its process group: Claude and the commands it's running (a build, the tests).
        let group = format!("-{}", j.child.id());
        _ = std::process::Command::new("kill").args(["-KILL", "--", &group]).stderr(std::process::Stdio::null()).status();
        _ = j.child.kill();
        _ = j.child.wait();
        self.claude.again = false;
        self.msg = format!("stopped Claude ({} for {}); its log: {} (and .err)", j.label, ago(j.started), j.log.display());
        self.reload_tree(); // what it changed before it stopped
    }

    /// Forget the PR's Claude session, so the next run starts a new one: when it has got
    /// something wrong into its head, or grown long. The review file is what carries over.
    pub(super) fn fresh_claude_session(&mut self) {
        if self.claude.job.is_some() {
            return self.msg = "Claude is still working on the last request; once it's done".into();
        }
        let pane = self.claude_pane().is_some();
        self.set_session(None);
        self.msg = match pane {
            true => "the next Claude run starts a new session; close Claude's pane, which has the old one".into(),
            false => "the next Claude run starts a new session".into(),
        };
    }

    /// `Space a` → approve Claude's description: after a confirmation (`sure`), it replaces
    /// the PR's on GitHub, and the proposal goes.
    pub(super) fn approve_description(&mut self, sure: bool) {
        let (Some(pr), Some(path)) = (self.pr.as_ref().map(|p| p.number), self.claude.path.clone()) else { return };
        if !sure {
            let items = vec![
                Item::new("No, not yet", Target::Nothing),
                Item::new(format!("Yes, replace #{pr}'s description on GitHub with Claude's"), Target::DescApprove(true)),
            ];
            return self.picker = Some(Picker::new(" description ", items));
        }
        let text = self.claude.description.clone();
        let body = (0..self.info.len()).find(|&i| self.is_description(i)).map(|h| h + 1).filter(|&b| self.info.get(b).is_some_and(|l| l.md));
        let shown = body.map_or(String::new(), |b| self.info[b].text.clone());
        // Edited on GitHub since dv showed it: the proposal's diff was against an older text.
        let now = match crate::gh::body(pr) {
            Ok(t) => t,
            Err(e) => return self.msg = format!("description not changed: {e:#}"),
        };
        if now.trim() != shown.trim() {
            if let Some(b) = body {
                self.info[b].text = now;
            }
            let keep = self.cursor_key();
            self.rebuild(keep);
            return self.msg = "description not changed: it was edited on GitHub since; dv shows it now against Claude's: approve again if it's still right".into();
        }
        if let Err(e) = crate::gh::set_body(pr, &text) {
            return self.msg = format!("description not changed: {e:#}");
        }
        self.github_changed();
        // What's on GitHub now, without waiting for the next fetch.
        if let Some(b) = body {
            self.info[b].text = text;
        }
        let cleared = crate::claude::clear(&path, "description");
        let keep = self.cursor_key();
        self.load_claude();
        self.rebuild(keep);
        self.msg = match cleared {
            Ok(()) => format!("#{pr}'s description is Claude's now"),
            Err(e) => format!("#{pr}'s description is Claude's now, but its proposal is still in the review file: {e:#} (Space a discards it)"),
        };
    }

    /// `Space a` → close the PR with Claude's comment: in your editor first (an empty one, or
    /// `:cq`, closes nothing), then posted, and the PR closed.
    pub(super) fn close_pr(&mut self) -> Result<String> {
        let (Some(n), Some(path)) = (self.pr.as_ref().map(|p| p.number), self.claude.path.clone()) else { return Ok(String::new()) };
        let what = format!("comment closing #{n} (posted on its conversation, then it's closed; empty closes nothing)");
        let Some(text) = compose_with(&what, false, self.claude.closing.trim(), true)?.filter(|t| !t.trim().is_empty()) else {
            return Ok(format!("#{n} not closed"));
        };
        keep_on_error(crate::gh::close(n, Some(text.trim())), &text)?;
        _ = crate::claude::clear(&path, "closing");
        self.pr_state = Some(("CLOSED", "da3633"));
        self.github_changed();
        let keep = self.cursor_key();
        self.load_claude();
        self.rebuild(keep);
        Ok(format!("#{n} closed, with your comment"))
    }

    /// `Space a` → discard Claude's draft `key` (`"description"`, `"closing"`), `what` it is.
    pub(super) fn discard(&mut self, key: &str, what: &str) {
        let Some(path) = self.claude.path.clone() else { return };
        self.msg = match crate::claude::clear(&path, key) {
            Ok(()) => {
                let keep = self.cursor_key();
                self.load_claude();
                self.rebuild(keep);
                format!("{what} discarded")
            }
            Err(e) => format!("{e:#}"),
        };
    }

    /// The ids of Claude's own comments whose code has changed (not your comments for it).
    pub(super) fn outdated_claude_comments(&self) -> Vec<String> {
        self.threads.iter().filter(|t| t.by_claude() && t.outdated).filter_map(|t| t.claude.clone()).collect()
    }

    /// From the `Space a` menu: dismiss every outdated comment of Claude's, after a confirmation
    /// (`sure`: the "Yes" in it).
    pub(super) fn dismiss_outdated(&mut self, sure: bool) {
        let keys = self.outdated_claude_comments();
        if keys.is_empty() {
            return self.msg = "no outdated comments from Claude".into();
        }
        if !sure {
            let items = vec![
                Item::new("No, keep them", Target::Nothing),
                Item::new(format!("Yes, dismiss Claude's {} outdated comment(s)", keys.len()), Target::ClaudeDismissOutdated(true)),
            ];
            return self.picker = Some(Picker::new(" dismiss outdated? ", items));
        }
        let Some(p) = self.claude.path.clone() else { return };
        let keep = self.cursor_key();
        let res = crate::claude::dismiss_all(&p, &keys);
        self.load_claude();
        self.rebuild(keep);
        self.msg = match res {
            Ok(n) => format!("dismissed {n} outdated comment(s) from Claude"),
            Err(e) => format!("{e:#}"),
        };
    }

    /// `extra`: the user's own instructions for this run (may be empty).
    pub(super) fn claude_prompt(&self, ask: Ask, extra: &str) -> String {
        let what = match &self.pr {
            Some(p) => format!("PR #{}", p.number),
            None => "the current branch".into(),
        };
        let prompt = match ask {
            Ask::Review => match self.reviewed_before() {
                Some(old) => update_prompt(&what, old),
                None => review_prompt(&what),
            },
            Ask::Answer => answer_prompt(&what, self.theirs),
            Ask::FixChecks => fix_checks_prompt(self.pr.as_ref().map_or(0, |p| p.number)),
            Ask::Close => format!(
                "Use the dv-review skill to draft a comment closing {what}, as the user asks below, in the review file's \
                 top-level \"closing\" field (see \"Draft a closing comment\"). Start from the draft already there, if there \
                 is one. Don't close the PR or comment on GitHub: the user closes it in dv."
            ),
            Ask::Describe => format!(
                "Use the dv-review skill to propose a new description for {what}, as the user asks below: the whole new text, \
                 in the review file's top-level \"description\" field (see \"Propose a description\"). Start from the proposal \
                 already there, if there is one, else from the PR's current description. Don't change the PR on GitHub: the user \
                 approves it in dv."
            ),
            Ask::Fix if self.theirs && self.change_requests() == 0 => format!(
                "Use the dv-review skill to suggest the change the user asks for below, in the dv review of {what}: \
                 it's someone else's PR, so don't edit the code; add a comment of yours there with a suggestion block."
            ),
            Ask::Fix if self.theirs => format!(
                "Use the dv-review skill to suggest the changes the open comments in the dv review of {what} ask for \
                 (see \"Suggest the changes\"): it's someone else's PR, so don't edit the code."
            ),
            // Making them (yours, or someone else's when you ask Claude to all the same).
            Ask::Fix | Ask::Edit if self.change_requests() == 0 => format!(
                "Make the change the user asks for below, in {what}. First run `dv review-info`: if the notes it lists \
                 exist, read them and follow them (they're the dv-review skill's notes on this repo)."
            ),
            Ask::Fix | Ask::Edit => format!(
                "Use the dv-review skill to make the changes the open comments in the dv review of {what} ask for \
                 (your own, and the user's: those with \"author\": \"user\"; every one that isn't resolved or outdated), \
                 replying on each with what you changed and resolving the ones you addressed."
            ),
        };
        // Claude Code knows the date but not the time, and makes one up for `created`.
        with_instructions(&format!("{prompt} (It's {} now.)", crate::claude::now()), extra)
    }

    /// Claude's note that PR `n`'s failing CI isn't its doing, written at its head now.
    pub(super) fn ci_note(&self, n: u64) -> Option<crate::claude::CiNote> {
        crate::claude::ci_note(n, &self.pr.as_ref()?.head_oid)
    }

    /// Open Claude's notes on the repo in hx (they're plain markdown; you can write them too).
    pub(super) fn edit_claude_notes(&mut self) {
        match crate::claude::notes_path() {
            Ok(p) => {
                _ = std::fs::create_dir_all(p.parent().unwrap_or(&p));
                self.action = Some(Action::Open(p.display().to_string()));
            }
            Err(e) => self.msg = format!("{e:#}"),
        }
    }

    /// From the `Space a` menu. Reviewing and making the changes first ask for your own
    /// instructions (an editor; optional, except for a change with no open comments to act on,
    /// where they're the request); answering replies just goes. Anything that would stop Claude
    /// taking it is checked first, so what you write isn't lost.
    pub(super) fn pick_claude(&mut self, ask: Ask) {
        // (Before the editor: nothing you write is lost to it.)
        if let Some(why) = self.claude_blocked(Some(ask)).or_else(|| self.pane_cant(ask)) {
            return self.msg = why;
        }
        match ask {
            Ask::Describe | Ask::Close if self.pr.is_none() => self.msg = "no PR here".into(),
            Ask::Review | Ask::Fix | Ask::Edit | Ask::Describe | Ask::Close => self.action = Some(Action::ClaudeWith(ask)),
            ask => _ = self.run_claude(ask, ""),
        }
    }

    /// Ask Claude (`extra`: your instructions, may be empty). Whether it was asked: if not,
    /// `msg` says why.
    pub(super) fn run_claude(&mut self, ask: Ask, extra: &str) -> bool {
        if matches!(ask, Ask::Answer) && self.waiting_replies() == 0 {
            self.msg = "no replies waiting for Claude (r on a Claude comment replies)".into();
            return false;
        }
        let requests = self.change_requests();
        if matches!(ask, Ask::Fix | Ask::Edit) && requests == 0 && extra.trim().is_empty() {
            self.msg = "nothing to change: no open comments, and no instructions written".into();
            return false;
        }
        if let Some(why) = self.claude_blocked(Some(ask)) {
            self.msg = why;
            return false;
        }
        // Fixing CI afresh: Claude's last word that it isn't the PR's doing goes (it says so
        // again, if it still finds that).
        if let (Ask::FixChecks, Some(p)) = (ask, &self.pr) {
            crate::claude::ci_note_path(p.number).map(std::fs::remove_file).ok();
        }
        let prompt = self.claude_prompt(ask, extra);
        let label = match ask {
            Ask::Review => "reviewing",
            Ask::Answer => "answering your replies",
            Ask::Describe => "proposing a description",
            Ask::Fix if self.theirs => "suggesting the changes",
            Ask::Edit => "making the changes",
            Ask::Close => "drafting a closing comment",
            Ask::Fix => "making the changes",
            Ask::FixChecks => "fixing CI",
        };
        // Claude already open on this PR in a pane: put the request there, rather than open
        // another. Pasted, not sent: what the pane is showing (a permission prompt, your own
        // half-typed text) isn't dv's to answer, so you press Enter there.
        if let Some(pane) = self.claude_pane() {
            if let Some(why) = self.pane_cant(ask) {
                self.msg = why;
                return false;
            }
            let (msg, asked) = match crate::pane::offer(pane, &prompt) {
                Ok(true) => (format!("{label}: the request is in Claude's pane, press Enter there to send it"), true),
                Ok(false) => ("Claude's pane is waiting for you to answer it; ask again after".into(), false),
                Err(e) => (format!("Claude's pane: {e:#}"), false),
            };
            self.msg = msg;
            return asked;
        }
        let mut path_env = std::ffi::OsString::from("PATH=");
        path_env.push(path_for_claude());
        let (msg, asked) = match in_zellij() {
            // A real session in a pane (the PR's, as the background runs use): watch it, approve
            // what it asks, talk to it.
            // Floating and unfocused: hidden while zellij's floating panes are (`Space a` →
            // show it, or zellij's own toggle); dv says when it's asking you something, and
            // closes it once Claude's done there (`close_if_done`).
            true => match std::process::Command::new("zellij")
                .args(["run", "--floating", "--no-focus", "--name", &format!("claude: {label}"), "--cwd"])
                .arg(&self.root)
                .args(["--", "env"])
                .arg(path_env) // the pane gets zellij's environment, not dv's
                .args(data_env()) // and so the same dv directory as this one, which it watches
                .args(["claude", &prompt])
                .args(self.session_args())
                .args(claude_dirs())
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
            {
                Ok(s) if s.success() => {
                    // For dv list: at it while its pane is open (a session: when it's done
                    // working there, only the pane knows).
                    if let (Some(p), Some(id)) = (&self.pr, &self.claude.session) {
                        crate::claude::mark_running(p.number, &format!("{label} (in its pane)"), None, Some(id));
                    }
                    (format!("Claude is {label} in a hidden pane (Space a shows it; it closes when Claude's done); its comments show up here as it writes them"), true)
                }
                Ok(s) => (format!("zellij run failed ({s})"), false),
                Err(e) => (format!("running zellij: {e}"), false),
            },
            false => match self.spawn_claude(&prompt, label, false, tools_for(self.pr.as_ref().filter(|_| matches!(ask, Ask::FixChecks)).map(|p| p.number)).as_deref().as_slice()) {
                Ok(job) => {
                    self.claude.job = Some(job);
                    (format!("Claude is {label} in the background…"), true)
                }
                Err(e) => (format!("starting claude: {e:#}"), false),
            },
        };
        self.msg = msg;
        asked
    }

    /// After `r` on Claude's comment (or `a`): have Claude answer right away, in the
    /// background (also in zellij: a pane per reply would get in the way). If the PR's session
    /// is open in a pane, this runs on a fork of it: the same conversation so far, without two
    /// processes on one session. Replies made while it's working get one more run when it
    /// finishes. Returns what to tell the user.
    pub(super) fn answer_now(&mut self) -> String {
        if self.claude.job.is_some() {
            self.claude.again = true;
            return "reply saved; Claude answers it when it's done with what it's doing".into();
        }
        if let Some(why) = self.claude_blocked(Some(Ask::Answer)) {
            return format!("reply saved; {why} (then Space a has Claude answer)");
        }
        let prompt = self.claude_prompt(Ask::Answer, "");
        let forked = self.claude_pane().is_some();
        match self.spawn_claude(&prompt, "answering", forked, &[]) {
            Ok(job) => {
                self.claude.job = Some(job);
                "reply sent; Claude is answering…".into()
            }
            Err(e) => format!("reply saved, but starting claude failed: {e:#} (Space a to retry)"),
        }
    }

    /// The zellij pane running this PR's Claude Code session, if one is open: dv starts those
    /// with the session id on the command line, which no other pane has.
    pub(super) fn claude_pane(&self) -> Option<u64> {
        self.claude_pane_cmd().map(|p| p.0)
    }

    /// `Space a` → show Claude's pane (`show_pane`).
    pub(super) fn show_claude_pane(&mut self) {
        let Some((pane, ..)) = self.claude_pane_cmd() else { return self.msg = "Claude's pane is closed".into() };
        if crate::pane::show(pane).is_err() {
            self.msg = "zellij couldn't show Claude's pane".into();
        }
    }

    /// Every few seconds: whether Claude's pane (hidden, maybe) is asking you something, a
    /// permission prompt it won't get past alone; said once each time it starts asking.
    pub(super) fn watch_claude_pane(&mut self) {
        if self.claude.session.is_none() || !in_zellij() || self.claude.pane_check.is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(3)) {
            return;
        }
        self.claude.pane_check = Some(std::time::Instant::now());
        let pane = self.claude_pane();
        let screen = pane.map(crate::pane::screen);
        let asking_now = screen.as_deref().is_some_and(crate::pane::asking);
        if asking_now && !self.claude.asking {
            self.msg = "Claude is asking you something in its pane: Space a → Show Claude's pane".into();
        }
        self.claude.asking = asking_now;
        // Claude Code says how to stop it while it's working.
        self.claude.pane_busy = screen.as_deref().is_some_and(crate::pane::working);
        // Done there: its pane closes (`close_if_done`); the session stays, for the next ask.
        // (The PR's marker may be another checkout's run: only this one's session counts.)
        let (Some(pr), Some(pane)) = (self.pr.as_ref().map(|p| p.number), pane) else { return };
        let Some((session, since)) = crate::claude::pane_run(pr).filter(|r| Some(&r.0) == self.claude.session.as_ref()) else { return };
        if crate::pane::close_if_done(pane, &session, since) {
            crate::claude::clear_running(pr);
            self.msg = "Claude's done: its pane closed (Space a asks it more, in the same session)".into();
        }
    }

    /// What Claude is doing here, for the pinned title: a run in the background, or its pane
    /// busy (or waiting on you). `None` when it's idle.
    pub(super) fn claude_doing(&self) -> Option<String> {
        match &self.claude.job {
            Some(j) => Some(format!("◐ claude: {} {}", j.label, ago(j.started))),
            None if self.claude.asking => Some("◐ claude is asking you (Space a)".into()),
            None if self.claude.pane_busy => Some("◐ claude working (its pane)".into()),
            None => None,
        }
    }

    /// The PR's Claude pane and its command line.
    fn claude_pane_cmd(&self) -> Option<(u64, String)> {
        let id = self.claude.session.as_ref().filter(|_| in_zellij())?;
        crate::pane::list(&["-c"]).unwrap_or_default().iter().find_map(|p| {
            let cmd = p["pane_command"].as_str().filter(|c| p["exited"] != true && c.contains(id.as_str()))?;
            Some((p["id"].as_u64()?, cmd.to_owned()))
        })
    }

    /// The PR's Claude Code session in this checkout: resume it if Claude Code has it, else
    /// start it under a new id. The id is saved next to the review file (per checkout, as Claude
    /// Code keeps sessions per directory), so the next run continues the same conversation. One
    /// that never started (a pane that failed) isn't resumed: Claude Code doesn't have it.
    fn session_args(&mut self) -> [String; 2] {
        let args = match &self.claude.path {
            Some(p) => crate::claude::session_args(p, &self.root).ok(),
            None => None,
        };
        let args = args.unwrap_or_else(|| ["--session-id".into(), crate::claude::new_session_id()]);
        self.claude.session = Some(args[1].clone());
        args
    }

    /// Remember (or, with `None`, forget) the PR's session in this checkout, on disk too.
    pub(super) fn set_session(&mut self, id: Option<String>) {
        if let Some(p) = &self.claude.path {
            _ = crate::claude::save_session(p, &self.root, id.as_deref());
        }
        self.claude.session = id;
    }

    /// `claude -p` in the background: its result (JSON) in a log file, its errors next to it.
    /// It runs in the PR's session (or, `forked`, a fork of it), so Claude keeps the review's
    /// context across runs.
    /// `more`: tools it may use on top of `ALLOWED`.
    pub(super) fn spawn_claude(&mut self, prompt: &str, label: &'static str, forked: bool, more: &[&str]) -> Result<ClaudeJob> {
        use std::os::unix::process::CommandExt;
        // One log per run, so a follow-up run doesn't wipe the one a message points to.
        self.claude.runs += 1;
        let log = std::env::temp_dir().join(format!("dv-claude-{}-{}.log", std::process::id(), self.claude.runs));
        let err = log.with_extension("err");
        let mut session = self.session_args().to_vec();
        if forked {
            session.push("--fork-session".into());
        }
        let child = claude_command(prompt, &self.root, &session, more, None)
            .stdin(std::process::Stdio::null())
            .stdout(std::fs::File::create(&log)?)
            .stderr(std::fs::File::create(&err)?)
            .process_group(0) // so stopping it stops what it's running too
            .spawn()
            .context("is Claude Code (`claude`) installed?")?;
        self.claude.log = Some(log.clone());
        // For dv list (a fork, its pane open: that pane's marker stays, for closing it later).
        if let Some(p) = self.pr.as_ref().filter(|_| !forked) {
            crate::claude::mark_running(p.number, label, Some(child.id()), None);
        }
        Ok(ClaudeJob { child, log, label, forked, started: std::time::Instant::now() })
    }

    /// When a background run ends: keep its session for the next one, say how it went (its
    /// writes have already shown up via `watch_claude`), and answer replies made meanwhile.
    pub(super) fn poll_claude(&mut self) {
        let Some(job) = &mut self.claude.job else { return };
        let Ok(Some(st)) = job.child.try_wait() else { return };
        let Some(ClaudeJob { log, label, forked, .. }) = self.claude.job.take() else { return };
        if let Some(p) = self.pr.as_ref().filter(|_| !forked) {
            crate::claude::clear_running(p.number);
        }
        self.reload_tree(); // it may have changed code (making changes, or answering a request)
        let out: Option<serde_json::Value> = std::fs::read_to_string(&log).ok().and_then(|t| serde_json::from_str(&t).ok());
        let failed = !st.success() || out.as_ref().is_none_or(|v| v["is_error"] == true);
        // A fork's id isn't the PR's session (that one's open in a pane): keep the PR's.
        match out.as_ref().and_then(|v| v["session_id"].as_str()) {
            _ if forked => {}
            Some(id) if !failed => self.set_session(Some(id.to_owned())),
            _ => self.set_session(None), // e.g. the session is gone: start afresh next time
        }
        // A run can end "successfully" having been refused what it needed: say what.
        let refused = out.as_ref().and_then(refusal);
        let cost = out.as_ref().and_then(|v| v["total_cost_usd"].as_f64());
        self.claude.spent += cost.unwrap_or(0.0);
        let took = match (out.as_ref().and_then(|v| v["num_turns"].as_u64()), cost) {
            (Some(n), Some(c)) => format!(" ({n} turns, ${c:.2})"),
            _ => String::new(),
        };
        self.msg = match (failed, &refused) {
            (false, None) => format!("Claude is done {label}{took}"),
            (false, Some(r)) => format!("Claude finished {label}, but was refused {r}: see {}", log.display()),
            (true, Some(r)) => format!("Claude stopped while {label}, refused {r}: see {}", log.display()),
            (true, None) => format!("Claude stopped ({st}) while {label}: see {} (and .err)", log.display()),
        };
        // A rebase Claude carried on: how it stands now.
        if label == super::rebase::REBASE_LABEL {
            self.rebase_ended();
        }
        // Placing changes for an amend: its answers, in the picker.
        // (Not over what you're at: a picker, a prompt, an action on its way.)
        // (Asked to place it all: amended, now.)
        let then = std::mem::take(&mut self.amend_then);
        if label == super::amend::AMEND_LABEL && !failed && refused.is_none() && then {
            self.amend_now();
        } else if label == super::amend::AMEND_LABEL && !failed && refused.is_none() {
            match self.picker.is_none() && self.prompt.is_none() && self.action.is_none() {
                true => self.amend_menu(),
                false => self.msg += " (Space g → amend shows where it put them)",
            }
        }
        // Replies made meanwhile: one more run, unless this one went wrong (then that's the news,
        // and retrying would hit the same wall).
        if std::mem::take(&mut self.claude.again) && self.waiting_replies() > 0 {
            match failed || refused.is_some() || super::rebase::rebasing(&self.root) {
                true => self.msg += " · your newer replies wait (Space a to retry)",
                false => self.msg = self.answer_now(),
            }
        }
    }

}

/// What a run's result says it was refused, in a few words: the first refusal's tool and its
/// target (first line, kept short for the status bar), "and N more" if there were others.
pub(super) fn refusal(out: &serde_json::Value) -> Option<String> {
    let denied = out["permission_denials"].as_array().filter(|d| !d.is_empty())?;
    let d = &denied[0];
    let input = &d["tool_input"];
    let target = ["file_path", "path", "command", "pattern", "url"].iter().find_map(|k| input[*k].as_str()).unwrap_or("");
    let target: String = target.lines().next().unwrap_or("").chars().take(60).collect();
    let more = match denied.len() {
        1 => String::new(),
        n => format!(" (and {} more)", n - 1),
    };
    Some(format!("{} {target}{more}", d["tool_name"].as_str().unwrap_or("a tool")).replace("  ", " ").trim().to_owned())
}

/// How long since `t`, for the status bar: "40s", "3m".
pub(super) fn ago(t: std::time::Instant) -> String {
    crate::gh::ago_secs(t.elapsed().as_secs() as i64)
}
