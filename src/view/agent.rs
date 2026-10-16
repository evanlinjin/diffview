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
}

/// Whether thread `t` is Claude's own comment (from its review file), not one you left for it.
pub(super) fn claude_wrote(t: &Thread) -> bool {
    t.claude.is_some() && t.comments.first().is_some_and(|c| c.author == crate::claude::AUTHOR)
}

impl App {
    /// Replies of yours that Claude hasn't answered yet.
    pub(super) fn waiting_replies(&self) -> usize {
        let yours = |t: &Thread| t.comments.last().is_some_and(|c| c.author == "you");
        self.threads.iter().filter(|t| t.claude.is_some() && !t.resolved && yours(t)).count()
    }

    /// Claude's comments still to act on: not dismissed (still in the file), not resolved, and
    /// not outdated (their code has already changed).
    pub(super) fn open_claude_comments(&self) -> usize {
        self.threads.iter().filter(|t| claude_wrote(t) && !t.resolved && !t.outdated).count()
    }

    pub(super) fn claude_menu(&mut self) {
        if self.claude_path.is_none() {
            return self.msg = "no review file here (not in a git repo?)".into();
        }
        let has_review = self.threads.iter().any(claude_wrote) || !self.claude_summary.is_empty();
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
        let mut items = vec![
            Item::new(review, Target::Claude(Ask::Review)),
            Item::new(format!("Ask Claude to answer your replies ({n} waiting)"), Target::Claude(Ask::Answer)),
            Item::new(format!("Ask Claude to make the changes ({open} open comments)"), Target::Claude(Ask::Fix)),
            Item::new(notes, Target::ClaudeNotes),
        ];
        let outdated = self.outdated_claude_comments().len();
        if outdated > 0 {
            items.push(Item::new(format!("Dismiss Claude's outdated comments ({outdated})"), Target::ClaudeDismissOutdated(false)));
        }
        self.picker = Some(Picker { title: " claude ", items, query: String::new(), sel: if n > 0 { 1 } else { 0 } });
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
        let Some(p) = self.claude_path.clone() else { return };
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
            Ask::Review => format!("Use the dv-review skill to review {what} for dv. If there's already a review, update it."),
            Ask::Answer => format!(
                "Use the dv-review skill to answer my replies (and the comments I left for you) in the dv review of {what}."
            ),
            Ask::Fix if self.open_claude_comments() == 0 => format!(
                "Make the change the user asks for below, in {what} (there are no open dv review comments to act on). \
                 Use the dv-review skill's notes on the repo, if it has any."
            ),
            Ask::Fix => format!(
                "Use the dv-review skill to make the changes your open comments ask for in the dv review of {what} \
                 (every comment that isn't resolved or outdated), replying on each with what you changed and resolving \
                 the ones you addressed."
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
    /// instructions (an editor; empty is fine); answering replies just goes.
    pub(super) fn pick_claude(&mut self, ask: Ask) {
        match ask {
            Ask::Review => self.action = Some(Action::ClaudeWith(Ask::Review)),
            // Always to the editor: instructions alone are enough, even without open comments.
            Ask::Fix => self.action = Some(Action::ClaudeWith(Ask::Fix)),
            ask => self.run_claude(ask, ""),
        }
    }

    pub(super) fn run_claude(&mut self, ask: Ask, extra: &str) {
        if matches!(ask, Ask::Answer) && self.waiting_replies() == 0 {
            return self.msg = "no replies waiting for Claude (r on a Claude comment replies)".into();
        }
        if matches!(ask, Ask::Fix) && self.open_claude_comments() == 0 && extra.trim().is_empty() {
            return self.msg = "nothing to change: no open Claude comments, and no instructions written".into();
        }
        if let Some(job) = &mut self.claude_job
            && job.child.try_wait().ok().flatten().is_none()
        {
            return self.msg = "Claude is still working on the last request".into();
        }
        let prompt = self.claude_prompt(ask, extra);
        let label = match ask {
            Ask::Review => "reviewing",
            Ask::Answer => "answering your replies",
            Ask::Fix => "making the changes",
        };
        // Claude already open on this PR in a pane: put the request there, rather than open
        // another. Pasted, not sent: what the pane is showing (a permission prompt, your own
        // half-typed text) isn't dv's to answer, so you press Enter there.
        if let Some((pane, cmd)) = self.claude_pane_cmd() {
            // A pane from before dv gave Claude its directory can't write the review there.
            let dv = crate::claude::dv_dir().map(|d| d.display().to_string()).unwrap_or_default();
            if !dv.is_empty() && !cmd.contains(&dv) {
                return self.msg = "Claude's pane for this PR can't reach dv's review files (it's from an older dv): close it, and Space a starts a new one".into();
            }
            return self.msg = match offer_to_pane(pane, &prompt) {
                Ok(true) => format!("{label}: the request is in Claude's pane, press Enter there to send it"),
                Ok(false) => "Claude's pane is waiting for you to answer it; ask again after".into(),
                Err(e) => format!("Claude's pane: {e:#}"),
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
                .args(self.claude_dirs())
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
            {
                Ok(s) if s.success() => format!("Claude is {label} in the pane on the right; its comments show up here as it writes them"),
                Ok(s) => format!("zellij run failed ({s})"),
                Err(e) => format!("running zellij: {e}"),
            },
            false => match self.spawn_claude(&prompt, label, false) {
                Ok(job) => {
                    self.claude_job = Some(job);
                    format!("Claude is {label} in the background…")
                }
                Err(e) => format!("starting claude: {e:#}"),
            },
        };
    }

    /// After `r` on Claude's comment (or `a`): have Claude answer right away, in the
    /// background (also in zellij: a pane per reply would get in the way). If the PR's session
    /// is open in a pane, this runs on a fork of it: the same conversation so far, without two
    /// processes on one session. Replies made while it's working get one more run when it
    /// finishes. Returns what to tell the user.
    pub(super) fn answer_now(&mut self) -> String {
        if self.claude_job.is_some() {
            self.claude_again = true;
            return "reply saved; Claude answers it when it's done with what it's doing".into();
        }
        let prompt = self.claude_prompt(Ask::Answer, "");
        let forked = self.claude_pane().is_some();
        match self.spawn_claude(&prompt, "answering", forked) {
            Ok(job) => {
                self.claude_job = Some(job);
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
        let id = self.claude_session.as_ref().filter(|_| in_zellij())?;
        let out = std::process::Command::new("zellij").args(["action", "list-panes", "-j", "-c", "-s"]).output().ok()?;
        let panes: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
        panes.as_array()?.iter().find_map(|p| {
            let cmd = p["pane_command"].as_str().filter(|c| p["exited"] != true && c.contains(id.as_str()))?;
            Some((p["id"].as_u64()?, cmd.to_owned()))
        })
    }

    /// `--add-dir` for dv's directory (the review files and notes): it's outside the checkout
    /// Claude runs in, which is all Claude Code lets it touch otherwise.
    pub(super) fn claude_dirs(&self) -> Vec<std::ffi::OsString> {
        match crate::claude::dv_dir() {
            Ok(dv) => vec!["--add-dir".into(), dv.into_os_string()],
            Err(_) => vec![],
        }
    }

    /// The PR's Claude Code session in this checkout: resume it if Claude Code has it, else
    /// start it under a new id. The id is saved next to the review file (per checkout, as Claude
    /// Code keeps sessions per directory), so the next run continues the same conversation. One
    /// that never started (a pane that failed) isn't resumed: Claude Code doesn't have it.
    fn session_args(&mut self) -> [String; 2] {
        match self.claude_session.clone().filter(|id| crate::claude::session_exists(id)) {
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
        if let Some(p) = &self.claude_path {
            _ = crate::claude::save_session(p, &self.root, id.as_deref());
        }
        self.claude_session = id;
    }

    /// `claude -p` in the background: its result (JSON) in a log file, its errors next to it.
    /// It runs in the PR's session (or, `forked`, a fork of it), so Claude keeps the review's
    /// context across runs.
    fn spawn_claude(&mut self, prompt: &str, label: &'static str, forked: bool) -> Result<ClaudeJob> {
        // One log per run, so a follow-up run doesn't wipe the one a message points to.
        self.claude_runs += 1;
        let log = std::env::temp_dir().join(format!("dv-claude-{}-{}.log", std::process::id(), self.claude_runs));
        let err = log.with_extension("err");
        let mut cmd = std::process::Command::new("claude");
        cmd.args(["-p", prompt, "--output-format", "json", "--permission-mode", "acceptEdits"]);
        cmd.args(self.session_args());
        if forked {
            cmd.arg("--fork-session");
        }
        let child = cmd
            .args(self.claude_dirs())
            .arg("--allowedTools")
            .args(ALLOWED)
            .current_dir(&self.root)
            .stdin(std::process::Stdio::null())
            .stdout(std::fs::File::create(&log)?)
            .stderr(std::fs::File::create(&err)?)
            .spawn()
            .context("is Claude Code (`claude`) installed?")?;
        Ok(ClaudeJob { child, log, label, forked })
    }

    /// When a background run ends: keep its session for the next one, say how it went (its
    /// writes have already shown up via `watch_claude`), and answer replies made meanwhile.
    pub(super) fn poll_claude(&mut self) {
        let Some(job) = &mut self.claude_job else { return };
        let Ok(Some(st)) = job.child.try_wait() else { return };
        let Some(ClaudeJob { log, label, forked, .. }) = self.claude_job.take() else { return };
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
        self.msg = match (failed, &refused) {
            (false, None) => format!("Claude is done {label}"),
            (false, Some(r)) => format!("Claude finished {label}, but was refused {r}: see {}", log.display()),
            (true, Some(r)) => format!("Claude stopped while {label}, refused {r}: see {}", log.display()),
            (true, None) => format!("Claude stopped ({st}) while {label}: see {} (and .err)", log.display()),
        };
        // Replies made meanwhile: one more run, unless this one went wrong (then that's the news,
        // and retrying would hit the same wall).
        if std::mem::take(&mut self.claude_again) && self.waiting_replies() > 0 {
            match failed || refused.is_some() {
                true => self.msg += " · your newer replies wait (Space a to retry)",
                false => self.msg = self.answer_now(),
            }
        }
    }

    /// For the status bar.
    pub(super) fn claude_working(&self) -> bool {
        self.claude_job.is_some()
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
    if ["Do you want", "Would you like", "❯ 1."].iter().any(|p| screen.contains(p)) {
        return Ok(false);
    }
    zellij(&["action", "paste", "-p", &id, prompt])?;
    zellij(&["action", "focus-pane-id", &id])?;
    Ok(true)
}
