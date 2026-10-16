//! `Space a`: run Claude Code on this review (the dv-review skill), without leaving dv. Inside
//! zellij it gets a pane on the right (an interactive session to watch and steer); elsewhere it runs
//! in the background. Either way its review shows up as it writes the file.
use super::*;

#[derive(Clone, Copy)]
pub(super) enum Ask {
    Review,
    Answer,
    /// Make the changes Claude's open comments ask for.
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
];

impl App {
    /// Replies of yours that Claude hasn't answered yet.
    pub(super) fn waiting_replies(&self) -> usize {
        let yours = |t: &Thread| t.comments.last().is_some_and(|c| c.author == "you");
        self.threads.iter().filter(|t| t.claude.is_some() && !t.resolved && yours(t)).count()
    }

    /// Claude's comments still to act on: not dismissed (still in the file), not resolved, and
    /// not outdated (their code has already changed).
    pub(super) fn open_claude_comments(&self) -> usize {
        self.threads.iter().filter(|t| t.claude.is_some() && !t.resolved && !t.outdated).count()
    }

    pub(super) fn claude_menu(&mut self) {
        if self.claude_path.is_none() {
            return self.msg = "no review file here (not in a git repo?)".into();
        }
        let has_review = self.threads.iter().any(|t| t.claude.is_some()) || !self.claude_summary.is_empty();
        let review = match has_review {
            true => "Update Claude's review (keeps comments that still apply, and your replies)",
            false => "Ask Claude to review this",
        };
        let n = self.waiting_replies();
        let open = self.open_claude_comments();
        let notes = match crate::claude::notes_path().is_ok_and(|p| p.exists()) {
            true => "Edit Claude's notes on this repo (it reads them before each task)",
            false => "Write notes for Claude on this repo (it reads them before each task)",
        };
        let items = vec![
            Item::new(review, Target::Claude(Ask::Review)),
            Item::new(format!("Ask Claude to answer your replies ({n} waiting)"), Target::Claude(Ask::Answer)),
            Item::new(format!("Ask Claude to make the changes ({open} open comments)"), Target::Claude(Ask::Fix)),
            Item::new(notes, Target::ClaudeNotes),
        ];
        self.picker = Some(Picker { title: " claude ", items, query: String::new(), sel: if n > 0 { 1 } else { 0 } });
    }

    /// `extra`: the user's own instructions for this run (may be empty).
    pub(super) fn claude_prompt(&self, ask: Ask, extra: &str) -> String {
        let what = match &self.pr {
            Some(p) => format!("PR #{}", p.number),
            None => "the current branch".into(),
        };
        let prompt = match ask {
            Ask::Review => format!("Use the dv-review skill to review {what} for dv. If there's already a review, update it."),
            Ask::Answer => format!(
                "Use the dv-review skill to answer my replies (and the comments I left for you) in the dv review of {what}."
            ),
            Ask::Fix => format!(
                "Use the dv-review skill to make the changes your open comments ask for in the dv review of {what} \
                 (every comment that isn't resolved or outdated), replying on each with what you changed."
            ),
        };
        match extra.trim() {
            "" => prompt,
            e => format!("{prompt}\n\nThe user adds:\n{e}"),
        }
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
        if self.claude_path.is_none() {
            return self.msg = "no review file here (not in a git repo?)".into();
        }
        let anchor = match self.selection_anchor() {
            Ok(a) => a,
            Err(_) => return self.msg = "a on code lines leaves Claude a comment there".into(),
        };
        // Their text, so the comment follows its code (as review-check records for Claude's).
        let code = self.files.iter().find(|f| f.path == anchor.path).map_or(vec![], |f| {
            let lines = crate::claude::side_lines(f, anchor.left);
            let first = anchor.start.map_or(anchor.line, |s| s.0);
            lines.iter().filter(|l| (first..=anchor.line).contains(&l.0)).map(|l| l.1.to_owned()).collect()
        });
        self.action = Some(Action::AskClaude { anchor, code });
    }

    /// From the `Space a` menu. Reviewing and making the changes first ask for your own
    /// instructions (an editor; empty is fine); answering replies just goes.
    pub(super) fn pick_claude(&mut self, ask: Ask) {
        match ask {
            Ask::Review => self.action = Some(Action::ClaudeWith(Ask::Review)),
            Ask::Fix if self.open_claude_comments() > 0 => self.action = Some(Action::ClaudeWith(Ask::Fix)),
            ask => self.run_claude(ask, ""),
        }
    }

    pub(super) fn run_claude(&mut self, ask: Ask, extra: &str) {
        if matches!(ask, Ask::Answer) && self.waiting_replies() == 0 {
            return self.msg = "no replies waiting for Claude (r on a Claude comment replies)".into();
        }
        if matches!(ask, Ask::Fix) && self.open_claude_comments() == 0 {
            return self.msg = "no open Claude comments to act on (resolved, outdated and dismissed ones are skipped)".into();
        }
        if let Some((child, ..)) = &mut self.claude_job
            && child.try_wait().ok().flatten().is_none()
        {
            return self.msg = "Claude is still working on the last request".into();
        }
        let prompt = self.claude_prompt(ask, extra);
        let label = match ask {
            Ask::Review => "reviewing",
            Ask::Answer => "answering your replies",
            Ask::Fix => "making the changes",
        };
        // Claude already open on this PR in a pane: ask there, rather than open another.
        if let Some(pane) = self.claude_pane() {
            return self.msg = match send_to_pane(pane, &prompt) {
                Ok(()) => format!("asked Claude in its pane (it's on this PR): {label}"),
                Err(e) => format!("typing into Claude's pane: {e:#}"),
            };
        }
        self.msg = match in_zellij() {
            // A real session in a pane (the PR's, as the background runs use): watch it, approve
            // what it asks, talk to it.
            true => match std::process::Command::new("zellij")
                .args(["run", "--direction", "right", "--name", &format!("claude: {label}"), "--cwd"])
                .arg(&self.root)
                .args(["--", "claude", &prompt])
                .args(self.session_args())
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
            {
                Ok(s) if s.success() => format!("Claude is {label} in the pane on the right; its comments show up here as it writes them"),
                Ok(s) => format!("zellij run failed ({s})"),
                Err(e) => format!("running zellij: {e}"),
            },
            false => match self.spawn_claude(&prompt) {
                Ok(job) => {
                    self.claude_job = Some((job.0, job.1, label));
                    format!("Claude is {label} in the background…")
                }
                Err(e) => format!("starting claude: {e:#}"),
            },
        };
    }

    /// After `r` on Claude's comment: have Claude answer right away, in the background (also in
    /// zellij: a pane per reply would get in the way). Replies made while it's working are
    /// picked up by one more run when it finishes. Returns what to tell the user.
    pub(super) fn answer_now(&mut self) -> String {
        // The PR's session is open in a pane: answer there (two processes on one session would
        // trip over each other).
        if let Some(pane) = self.claude_pane() {
            return match send_to_pane(pane, &self.claude_prompt(Ask::Answer, "")) {
                Ok(()) => "reply sent to Claude's pane; it answers there".into(),
                Err(e) => format!("reply saved, but typing into Claude's pane failed: {e:#}"),
            };
        }
        if self.claude_job.is_some() {
            self.claude_again = true;
            return "reply saved; Claude answers it when it's done with what it's doing".into();
        }
        let prompt = self.claude_prompt(Ask::Answer, "");
        match self.spawn_claude(&prompt) {
            Ok((child, log)) => {
                self.claude_job = Some((child, log, "answering"));
                "reply sent; Claude is answering…".into()
            }
            Err(e) => format!("reply saved, but starting claude failed: {e:#} (Space a to retry)"),
        }
    }

    /// The zellij pane running this PR's Claude Code session, if one is open: dv starts those
    /// with the session id on the command line, which no other pane has.
    fn claude_pane(&self) -> Option<u64> {
        let id = self.claude_session.as_ref().filter(|_| in_zellij())?;
        let out = std::process::Command::new("zellij").args(["action", "list-panes", "-j", "-c", "-s"]).output().ok()?;
        let panes: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
        panes.as_array()?.iter().find_map(|p| {
            let live = p["exited"] != true && p["pane_command"].as_str().is_some_and(|c| c.contains(id.as_str()));
            live.then(|| p["id"].as_u64()).flatten()
        })
    }

    /// The PR's Claude Code session: resume it, or start it under a new id. Either way the id
    /// is saved next to the review file, so the next run (from any dv, any worktree) continues
    /// the same conversation about this PR.
    fn session_args(&mut self) -> [String; 2] {
        match self.claude_session.clone() {
            Some(id) => ["--resume".into(), id],
            None => {
                let id = crate::claude::new_session_id();
                self.set_session(Some(id.clone()));
                ["--session-id".into(), id]
            }
        }
    }

    /// Remember (or, with `None`, forget) the PR's session, on disk too.
    pub(super) fn set_session(&mut self, id: Option<String>) {
        if let Some(p) = self.claude_path.as_ref().map(|p| crate::claude::session_path(p)) {
            _ = match &id {
                Some(id) => std::fs::create_dir_all(p.parent().unwrap_or(&p)).and_then(|_| std::fs::write(&p, id)),
                None => std::fs::remove_file(&p),
            };
        }
        self.claude_session = id;
    }

    /// `claude -p` in the background: its result (JSON) in a log file, its errors next to it.
    /// It runs in the PR's session, so Claude keeps the review's context across runs.
    fn spawn_claude(&mut self, prompt: &str) -> Result<(std::process::Child, PathBuf)> {
        let log = std::env::temp_dir().join(format!("dv-claude-{}.log", std::process::id()));
        let err = log.with_extension("err");
        let mut cmd = std::process::Command::new("claude");
        cmd.args(["-p", prompt, "--output-format", "json", "--permission-mode", "acceptEdits"]);
        cmd.args(self.session_args());
        let child = cmd
            .arg("--allowedTools")
            .args(ALLOWED)
            .current_dir(&self.root)
            .stdin(std::process::Stdio::null())
            .stdout(std::fs::File::create(&log)?)
            .stderr(std::fs::File::create(&err)?)
            .spawn()
            .context("is Claude Code (`claude`) installed?")?;
        Ok((child, log))
    }

    /// When a background run ends: keep its session for the next one, say how it went (its
    /// writes have already shown up via `watch_claude`), and answer replies made meanwhile.
    pub(super) fn poll_claude(&mut self) {
        let Some((child, log, label)) = &mut self.claude_job else { return };
        let Ok(Some(st)) = child.try_wait() else { return };
        let (log, label) = (log.clone(), *label);
        self.claude_job = None;
        let out: Option<serde_json::Value> = std::fs::read_to_string(&log).ok().and_then(|t| serde_json::from_str(&t).ok());
        let failed = !st.success() || out.as_ref().is_none_or(|v| v["is_error"] == true);
        match out.as_ref().and_then(|v| v["session_id"].as_str()) {
            Some(id) if !failed => self.set_session(Some(id.to_owned())),
            _ => self.set_session(None), // e.g. the session is gone: start afresh next time
        }
        self.msg = match failed {
            false => format!("Claude is done {label}"),
            true => format!("Claude stopped ({st}) while {label}: see {} (and .err)", log.display()),
        };
        if std::mem::take(&mut self.claude_again) && !failed && self.waiting_replies() > 0 {
            self.msg = self.answer_now();
        }
    }

    /// For the status bar.
    pub(super) fn claude_working(&self) -> bool {
        self.claude_job.is_some()
    }
}

/// Type `prompt` into a Claude Code pane and send it: as a paste (so a multi-line prompt
/// arrives whole), then Enter. If Claude is busy, it queues it.
fn send_to_pane(pane: u64, prompt: &str) -> Result<()> {
    let id = format!("terminal_{pane}");
    for chars in [format!("\x1b[200~{prompt}\x1b[201~"), "\r".into()] {
        let st = std::process::Command::new("zellij")
            .args(["action", "write-chars", "-p", &id, &chars])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()?;
        anyhow::ensure!(st.success(), "zellij action write-chars failed ({st})");
    }
    Ok(())
}
