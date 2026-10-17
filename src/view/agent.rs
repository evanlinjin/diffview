//! `Space a`: run Claude Code on this review (the dv-review skill), without leaving dv. Inside
//! zellij it gets a pane on the right (an interactive session to watch and steer); elsewhere it runs
//! in the background. Either way its review shows up as it writes the file.
use super::*;

#[derive(Clone, Copy)]
pub(super) enum Ask {
    Review,
    Answer,
    /// Make the changes the review's open comments ask for (Claude's and yours), and/or what
    /// your instructions say.
    Fix,
}

/// What `claude -p` may do in the background: edit files (the review, and code when asked),
/// and run only the commands the skill needs.
const ALLOWED: &[&str] = &[
    "Bash(dv review-info:*)",
    "Bash(dv review-check:*)",
    "Bash(git diff:*)",
    "Bash(git show:*)",
    "Bash(git log:*)",
    "Bash(git rev-parse:*)",
    // To check a change it made still builds and passes.
    "Bash(cargo check:*)",
    "Bash(cargo build:*)",
    "Bash(cargo test:*)",
    "Bash(cargo clippy:*)",
];

/// What drafting a replacement needs on top: reading the PRs it replaces, and committing (to
/// its own local branch; dv never pushes it without you).
pub(crate) const REPLACE_TOOLS: &[&str] = &[
    "Bash(dv replace-info:*)",
    "Bash(gh pr view:*)",
    "Bash(gh pr diff:*)",
    "Bash(gh issue view:*)",
    "Bash(git status:*)",
    "Bash(git add:*)",
    "Bash(git commit:*)",
];

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

/// Whether thread `t` is Claude's own comment (from its review file), not one you left for it.
pub(super) fn claude_wrote(t: &Thread) -> bool {
    t.claude.is_some() && t.comments.first().is_some_and(|c| c.author == crate::claude::AUTHOR)
}

/// Whose turn it is on an open thread of the review file: Claude's (`[waiting]`: you had the
/// last word), or yours once Claude has answered you (`[answered]`).
pub(super) fn turn(t: &Thread) -> Option<&'static str> {
    let last = t.comments.last().filter(|_| t.claude.is_some() && !t.resolved)?;
    match last.author == "you" {
        true => Some("[waiting] "),
        false => t.comments.iter().any(|c| c.author == "you").then_some("[answered] "),
    }
}

impl App {
    /// Whether thread `t` shows: in the Claude view, only those of Claude's review; in the
    /// pending view, those with a comment of your pending review.
    pub(super) fn in_view(&self, t: &Thread) -> bool {
        match self.view {
            View::All => true,
            View::Claude => t.claude.is_some(),
            View::Pending => t.comments.iter().any(|c| c.pending),
        }
    }

    /// `Ctrl-w a` / `Ctrl-w p`: the Claude / pending view, or back to the whole diff.
    pub(super) fn toggle_view(&mut self, v: View) {
        self.view = if self.view == v { View::All } else { v };
        let keep = self.cursor_key();
        self.rebuild(keep);
        let any = self.threads.iter().any(|t| self.in_view(t));
        self.msg = match self.view {
            View::Claude if !any && self.claude.summary.is_empty() => "Claude view: Claude hasn't reviewed this yet (Space a asks it; Ctrl-w a: the whole diff)",
            View::Claude => "Claude view: its summary and the code it commented on (Ctrl-w a: the whole diff)",
            View::Pending if !any && self.summary_draft.is_empty() => "pending view: no pending review yet (c queues a comment; Ctrl-w p: the whole diff)",
            View::Pending => "pending view: your pending review and the code it's on (S submits it; Ctrl-w p: the whole diff)",
            View::All => "the whole diff",
        }
        .into();
    }

    /// Replies of yours that Claude hasn't answered yet.
    pub(super) fn waiting_replies(&self) -> usize {
        self.threads.iter().filter(|t| turn(t) == Some("[waiting] ")).count()
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
        self.threads.iter().filter(|t| t.claude.is_some() && !t.resolved && !t.outdated).count()
    }

    /// Why Claude can't take a request right now (checked before asking you for instructions,
    /// so nothing you write is lost to it): still busy, or the PR's pane is from an older dv
    /// that can't reach the review files (when the request needs them).
    fn claude_blocked(&self, needs_files: bool) -> Option<String> {
        let busy = self.claude.job.is_some();
        if busy {
            return Some("Claude is still working on the last request".into());
        }
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
        let has_review = self.threads.iter().any(claude_wrote) || !self.claude.summary.is_empty();
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
            Item::new(notes, Target::ClaudeNotes),
            Item::new("Check Claude's review now (dv watches it; this reads it again)", Target::ClaudeCheck),
        ];
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
                Target::ClaudeReplace(vec![p.number]),
            )),
            (None, Some(r)) if r.drafting_now() => items.push(Item::new("Claude is drafting this replacement (dv shows it as it goes)", Target::Nothing)),
            (None, Some(r)) => items.push(Item::new(
                "Redraft this replacement (Claude carries on, with your instructions)",
                Target::ClaudeReplace(r.replaces.iter().map(|o| o.number).collect()),
            )),
            _ => {}
        }
        self.picker = Some(Picker { title: " claude ", items, query: String::new(), sel: if n > 0 { 1 } else { 0 } });
    }

    /// `Space a` → check now: the review file read again (as if it had changed), and the
    /// background run's state, rather than waiting for dv to notice.
    pub(super) fn check_claude(&mut self) {
        self.claude.mtime = None; // read it, changed or not
        self.watch_claude();
        self.poll_claude();
        if let Some(j) = &self.claude.job {
            self.msg = format!("Claude is still {} ({} so far); Space a can stop it", j.label, ago(j.started));
        } else if !self.claude.path.as_ref().is_some_and(|p| p.exists()) {
            self.msg = "Claude hasn't written a review yet (Space a → review asks for one)".into();
        }
    }

    /// `Space a` → stop: end the background run (its log stays). The session is kept, so the
    /// next run resumes where it left off.
    pub(super) fn stop_claude(&mut self) {
        let Some(mut j) = self.claude.job.take() else { return self.msg = "Claude isn't working on anything".into() };
        _ = j.child.kill();
        _ = j.child.wait();
        self.claude.again = false;
        self.reload_tree(); // what it changed before it stopped
        self.msg = format!("stopped Claude ({} for {}); its log: {}", j.label, ago(j.started), j.log.display());
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
            true => "the next Claude run starts a new session; close the pane on the right, which has the old one".into(),
            false => "the next Claude run starts a new session".into(),
        };
    }

    /// The ids of Claude's own comments whose code has changed (not your comments for it).
    pub(super) fn outdated_claude_comments(&self) -> Vec<String> {
        self.threads.iter().filter(|t| claude_wrote(t) && t.outdated).filter_map(|t| t.claude.clone()).collect()
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
            return self.picker = Some(Picker { title: " dismiss outdated? ", items, query: String::new(), sel: 0 });
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
                Some(old) => format!(
                    "Use the dv-review skill to update your dv review of {what}: review only what changed since you \
                     reviewed {old} (`git diff {old}`), and keep the comments that still apply."
                ),
                None => review_prompt(&what),
            },
            Ask::Answer if self.theirs => format!(
                "Use the dv-review skill to answer my replies (and the comments I left for you) in the dv review of {what}. \
                 It's someone else's PR, so don't edit the code: where I ask for a change, suggest it instead (see \
                 \"Suggest the changes\")."
            ),
            Ask::Answer => format!(
                "Use the dv-review skill to answer my replies (and the comments I left for you) in the dv review of {what}."
            ),
            Ask::Fix if self.theirs && self.change_requests() == 0 => format!(
                "Use the dv-review skill to suggest the change the user asks for below, in the dv review of {what}: \
                 it's someone else's PR, so don't edit the code; add a comment of yours there with a suggestion block."
            ),
            Ask::Fix if self.theirs => format!(
                "Use the dv-review skill to suggest the changes the open comments in the dv review of {what} ask for \
                 (see \"Suggest the changes\"): it's someone else's PR, so don't edit the code."
            ),
            Ask::Fix if self.change_requests() == 0 => format!(
                "Make the change the user asks for below, in {what}. First run `dv review-info`: if the notes it lists \
                 exist, read them and follow them (they're the dv-review skill's notes on this repo)."
            ),
            Ask::Fix => format!(
                "Use the dv-review skill to make the changes the open comments in the dv review of {what} ask for \
                 (your own, and the user's: those with \"author\": \"user\"; every one that isn't resolved or outdated), \
                 replying on each with what you changed and resolving the ones you addressed."
            ),
        };
        // Claude Code knows the date but not the time, and makes one up for `created`.
        with_instructions(&format!("{prompt} (It's {} now.)", crate::claude::now()), extra)
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

    /// `a`: leave a comment for Claude on the selected lines (or the cursor's), in the review
    /// file; Claude answers right away, as it does replies. Nothing goes to GitHub.
    pub(super) fn start_ask_claude(&mut self) {
        if self.claude.path.is_none() {
            return self.msg = "no review file here (not in a git repo?)".into();
        }
        let anchor = match self.selection_anchor() {
            Ok(a) => a,
            Err(_) => return self.msg = "a on code lines leaves Claude a comment there".into(),
        };
        // Line numbers are per side; a range across removed and added lines has no one side.
        if anchor.start.is_some_and(|(_, left)| left != anchor.left) {
            return self.msg = "select lines on one side (removed, or added/unchanged) to leave Claude a comment".into();
        }
        // Their text, so the comment follows its code (as review-check records for Claude's).
        let first = anchor.start.map_or(anchor.line, |s| s.0);
        let code = self.files.iter().find(|f| f.path == anchor.path).and_then(|f| crate::claude::code_at(f, anchor.left, first, anchor.line));
        match code {
            Some(code) => self.action = Some(Action::AskClaude { anchor, code }),
            None => self.msg = "those lines aren't in this diff".into(),
        }
    }

    /// From the `Space a` menu. Reviewing and making the changes first ask for your own
    /// instructions (an editor; optional, except for a change with no open comments to act on,
    /// where they're the request); answering replies just goes. Anything that would stop Claude
    /// taking it is checked first, so what you write isn't lost.
    pub(super) fn pick_claude(&mut self, ask: Ask) {
        let needs_files = !matches!(ask, Ask::Fix) || self.change_requests() > 0;
        if let Some(why) = self.claude_blocked(needs_files) {
            return self.msg = why;
        }
        match ask {
            Ask::Review | Ask::Fix => self.action = Some(Action::ClaudeWith(ask)),
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
        if matches!(ask, Ask::Fix) && requests == 0 && extra.trim().is_empty() {
            self.msg = "nothing to change: no open comments, and no instructions written".into();
            return false;
        }
        if let Some(why) = self.claude_blocked(!matches!(ask, Ask::Fix) || requests > 0) {
            self.msg = why;
            return false;
        }
        let prompt = self.claude_prompt(ask, extra);
        let label = match ask {
            Ask::Review => "reviewing",
            Ask::Answer => "answering your replies",
            Ask::Fix if self.theirs => "suggesting the changes",
            Ask::Fix => "making the changes",
        };
        // Claude already open on this PR in a pane: put the request there, rather than open
        // another. Pasted, not sent: what the pane is showing (a permission prompt, your own
        // half-typed text) isn't dv's to answer, so you press Enter there.
        if let Some(pane) = self.claude_pane() {
            let (msg, asked) = match offer_to_pane(pane, &prompt) {
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
            true => match std::process::Command::new("zellij")
                .args(zellij_run())
                .args(["--name", &format!("claude: {label}"), "--cwd"])
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
                Ok(s) if s.success() => (format!("Claude is {label} in the pane on the right; its comments show up here as it writes them"), true),
                Ok(s) => (format!("zellij run failed ({s})"), false),
                Err(e) => (format!("running zellij: {e}"), false),
            },
            false => match self.spawn_claude(&prompt, label, false) {
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
        let prompt = self.claude_prompt(Ask::Answer, "");
        let forked = self.claude_pane().is_some();
        match self.spawn_claude(&prompt, "answering", forked) {
            Ok(job) => {
                self.claude.job = Some(job);
                "reply sent; Claude is answering…".into()
            }
            Err(e) => format!("reply saved, but starting claude failed: {e:#} (Space a to retry)"),
        }
    }

    /// The zellij pane running this PR's Claude Code session, if one is open: dv starts those
    /// with the session id on the command line, which no other pane has.
    fn claude_pane(&self) -> Option<u64> {
        self.claude_pane_cmd().map(|p| p.0)
    }

    /// The PR's Claude pane and its command line.
    fn claude_pane_cmd(&self) -> Option<(u64, String)> {
        let id = self.claude.session.as_ref().filter(|_| in_zellij())?;
        let out = std::process::Command::new("zellij").args(["action", "list-panes", "-j", "-c", "-s"]).output().ok()?;
        let panes: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
        panes.as_array()?.iter().find_map(|p| {
            let cmd = p["pane_command"].as_str().filter(|c| p["exited"] != true && c.contains(id.as_str()))?;
            Some((p["id"].as_u64()?, cmd.to_owned()))
        })
    }

    /// The PR's Claude Code session in this checkout: resume it if Claude Code has it, else
    /// start it under a new id. The id is saved next to the review file (per checkout, as Claude
    /// Code keeps sessions per directory), so the next run continues the same conversation. One
    /// that never started (a pane that failed) isn't resumed: Claude Code doesn't have it.
    fn session_args(&mut self) -> [String; 2] {
        match self.claude.session.clone().filter(|id| crate::claude::session_exists(id)) {
            Some(id) => ["--resume".into(), id],
            None => {
                let id = crate::claude::new_session_id();
                self.set_session(Some(id.clone()));
                ["--session-id".into(), id]
            }
        }
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
    fn spawn_claude(&mut self, prompt: &str, label: &'static str, forked: bool) -> Result<ClaudeJob> {
        // One log per run, so a follow-up run doesn't wipe the one a message points to.
        self.claude.runs += 1;
        let log = std::env::temp_dir().join(format!("dv-claude-{}-{}.log", std::process::id(), self.claude.runs));
        let err = log.with_extension("err");
        let mut session = self.session_args().to_vec();
        if forked {
            session.push("--fork-session".into());
        }
        let child = claude_command(prompt, &self.root, &session, &[], None)
            .stdin(std::process::Stdio::null())
            .stdout(std::fs::File::create(&log)?)
            .stderr(std::fs::File::create(&err)?)
            .spawn()
            .context("is Claude Code (`claude`) installed?")?;
        self.claude.log = Some(log.clone());
        Ok(ClaudeJob { child, log, label, forked, started: std::time::Instant::now() })
    }

    /// When a background run ends: keep its session for the next one, say how it went (its
    /// writes have already shown up via `watch_claude`), and answer replies made meanwhile.
    pub(super) fn poll_claude(&mut self) {
        let Some(job) = &mut self.claude.job else { return };
        let Ok(Some(st)) = job.child.try_wait() else { return };
        let Some(ClaudeJob { log, label, forked, .. }) = self.claude.job.take() else { return };
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
        // Replies made meanwhile: one more run, unless this one went wrong (then that's the news,
        // and retrying would hit the same wall).
        if std::mem::take(&mut self.claude.again) && self.waiting_replies() > 0 {
            match failed || refused.is_some() {
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

/// Put `prompt` in a Claude Code pane's input, as a paste (so a multi-line prompt arrives
/// whole), and focus the pane; you press Enter there. Not if the pane is asking you something
/// (a permission prompt): `Ok(false)`, and nothing is typed.
fn offer_to_pane(pane: u64, prompt: &str) -> Result<bool> {
    let id = format!("terminal_{pane}");
    let zellij = |args: &[&str]| -> Result<std::process::Output> {
        let out = std::process::Command::new("zellij").args(args).stdin(std::process::Stdio::null()).output()?;
        anyhow::ensure!(out.status.success(), "zellij {} failed ({})", args[1], out.status);
        Ok(out)
    };
    let screen = String::from_utf8_lossy(&zellij(&["action", "dump-screen", "-p", &id])?.stdout).into_owned();
    if asking(&screen) {
        return Ok(false);
    }
    zellij(&["action", "paste", "-p", &id, prompt])?;
    zellij(&["action", "focus-pane-id", &id])?;
    Ok(true)
}

/// Whether a Claude Code pane's screen shows it asking you something: a permission prompt, or
/// (in a folder it hasn't seen) whether you trust it. Typing in then would answer it.
pub(super) fn asking(screen: &str) -> bool {
    ["Do you want", "Would you like", "❯ 1.", "trust this folder"].iter().any(|p| screen.contains(p))
}

/// Have Claude draft a replacement for `prs` with a dv of its own (`dv replace`), in the
/// background: it carries on if this dv quits. Its output goes to a log in the temp dir.
pub(super) fn spawn_replace(prs: &[u64]) -> Result<()> {
    use std::os::unix::process::CommandExt;
    let log = std::env::temp_dir().join(format!("dv-replace-{}.log", prs[0]));
    std::process::Command::new(std::env::current_exe()?)
        .arg("replace")
        .args(prs.iter().map(u64::to_string))
        .stdin(std::process::Stdio::null())
        .stdout(std::fs::File::create(&log)?)
        .stderr(std::fs::File::create(log.with_extension("err"))?)
        .process_group(0) // not this terminal's: closing it doesn't stop the draft
        .spawn()?;
    Ok(())
}

/// `prompt`, with the user's own instructions (if any) after it.
pub(crate) fn with_instructions(prompt: &str, extra: &str) -> String {
    match extra.trim() {
        "" => prompt.to_owned(),
        e => format!("{prompt}\n\nThe user adds:\n{e}"),
    }
}

/// `--add-dir` for dv's directory (the review files and notes): it's outside the checkout
/// Claude runs in, which is all Claude Code lets it touch otherwise.
pub(super) fn claude_dirs() -> Vec<std::ffi::OsString> {
    match crate::claude::dv_dir() {
        Ok(dv) => vec!["--add-dir".into(), dv.into_os_string()],
        Err(_) => vec![],
    }
}

/// Asking Claude to review `what` ("PR #12").
pub(crate) fn review_prompt(what: &str) -> String {
    format!("Use the dv-review skill to review {what} for dv. If there's already a review, update it.")
}

/// `claude -p prompt` in `dir` as dv runs it in the background: its result as JSON on stdout,
/// edits allowed, in the session `session` names (`--resume <id>`, `--session-id <id>`, …),
/// with dv's directory reachable, and only the commands the skill needs.
/// `more`: tools allowed on top of the usual ones; `access`: the one directory outside `dir`
/// Claude may write to (`None`: all of dv's).
pub(crate) fn claude_command(prompt: &str, dir: &Path, session: &[String], more: &[&str], access: Option<&Path>) -> std::process::Command {
    let mut cmd = std::process::Command::new("claude");
    cmd.env("PATH", path_for_claude())
        .args(["-p", prompt, "--output-format", "json", "--permission-mode", "acceptEdits"])
        .args(session);
    match access {
        Some(d) => _ = cmd.arg("--add-dir").arg(d),
        None => _ = cmd.args(claude_dirs()),
    }
    // Last: it takes every argument after it.
    cmd.arg("--allowedTools").args(ALLOWED).args(more).current_dir(dir);
    cmd
}

/// PATH for Claude, with this dv's directory first: the skill runs `dv review-*`, which must
/// be found (dv may have been started by its full path, or from a shell whose PATH a zellij
/// pane doesn't get) and be this version.
fn path_for_claude() -> std::ffi::OsString {
    let own = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf));
    let rest = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect::<Vec<_>>()).unwrap_or_default();
    std::env::join_paths(own.into_iter().chain(rest)).unwrap_or_default()
}

/// How long since `t`, for the status bar: "40s", "3m".
/// `XDG_DATA_HOME=…` if dv has it set: where its review files are.
fn data_env() -> Option<std::ffi::OsString> {
    let mut v = std::ffi::OsString::from("XDG_DATA_HOME=");
    v.push(std::env::var_os("XDG_DATA_HOME")?);
    Some(v)
}

pub(super) fn ago(t: std::time::Instant) -> String {
    match t.elapsed().as_secs() {
        s if s < 60 => format!("{s}s"),
        s => format!("{}m", s / 60),
    }
}
