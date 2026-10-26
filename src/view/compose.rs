//! What's done with what you write in the editor (`Action`): a comment, a reply, a question for
//! Claude, a commit message, …: `perform`, and what it posts (`Post`).
use super::*;

/// Work that needs the terminal back (an editor), run between frames.
#[derive(Clone)]
pub(super) enum Action {
    /// A shell in the checkout's directory: a floating pane in zellij, else this terminal.
    Shell,
    Open(String),
    /// `review`: add to the pending review instead of posting immediately.
    /// `prefill`: starting text in the editor (a ```suggestion block for `s`).
    /// `claude`: copying Claude's comment (its id in the review file) into the review; the
    /// prefill is its text, posted as is if left unchanged, and the comment is dismissed after.
    Comment { anchor: Anchor, review: bool, prefill: String, claude: Option<String> },
    /// `quote`: the comment under the cursor as a `> ` quote, pre-filled in the editor.
    Reply { thread: String, comment: u64, review: bool, quote: String },
    /// A reply to Claude on its comment (by id), saved to the review file for Claude to answer,
    /// with what you want of it ("ask", "suggest", "change"; empty: unsaid).
    /// (And what the editor starts from: a quote of what's selected, say.)
    ClaudeReply(String, &'static str, String),
    /// One reply to several of Claude's threads (their keys), with what you want of it.
    ClaudeReplyAll(Vec<String>, &'static str),
    /// A comment for Claude on these lines (with their text), then Claude answers it; `about`:
    /// the GitHub review comment it asks about (its id, and a quote of it for the editor).
    AskClaude { anchor: Anchor, code: Vec<String>, about: Option<(u64, String)>, want: &'static str },
    /// `a` on a PR conversation comment: a question about it (its URL), starting from `prefill`.
    /// On a commit (its URL): a question about it, or a change to it (`want`).
    AskConversation { url: String, prefill: String, want: &'static str },
    /// Run Claude (review, or make the changes) after your own instructions for it.
    ClaudeWith(agent::Ask),
    /// Have Claude draft a replacement for these PRs, after your instructions for it (a first
    /// draft's start from your last ones; a redraft's are a thread about the whole draft).
    Replace(Vec<u64>),
    /// Publish the replacement here as a draft PR, after the editor (title, description): (then
    /// close what it replaces, next).
    Publish(bool),
    /// A new PR fixing CI for Claude to draft, after the editor (from Claude's note on this PR).
    CiFixPr,
    /// A new PR for Claude to draft, from this prefill (`Space N`: what's selected).
    /// A new PR for Claude to draft, what it should do (`as_is`: the prefill alone will do).
    NewPr { prefill: String, as_is: bool },
    /// Close the PR with Claude's draft comment, after the editor.
    ClosePr,
    /// Close what closing it closes with it, #N, with Claude's comment for it (`closing_with`),
    /// after the editor; then the next.
    CloseWith(u64),
    /// Close the PRs the replacement here replaces, after the editor (the note).
    CloseOriginals,
    /// A new PR conversation comment, starting with `quote` (may be empty); `as_is`: posted
    /// even if left untouched (Claude's review, yours to post).
    Converse { quote: String, as_is: bool },
    /// Rewrite one of your comments (node id, current body).
    Edit { comment: String, body: String, author: String },
    /// Submit the pending review: COMMENT, APPROVE or REQUEST_CHANGES.
    Submit(&'static str),
    /// Commit everything uncommitted: Claude writes the message first (`commit_ask`).
    Commit,
    /// Commit everything uncommitted, the editor starting from this message.
    CommitWith(String),
}

/// Something written in the editor, to send to GitHub (as a job, so "posting…" shows first).
pub(super) enum Post {
    /// A new comment on lines (`claude`: copied from Claude's comment, dismissed once sent).
    Thread { anchor: Anchor, review: bool, body: String, claude: Option<String> },
    Reply { thread: String, comment: u64, review: bool, body: String },
    Converse(String),
    /// Your pending review's new text (`c` in the PR section, or on Claude's summary).
    Summary(String),
    Edit { comment: String, body: String },
    /// (The event, the summary, the PR's head it was written on.)
    Submit(&'static str, String, String),
}

/// How dv's hint lines in the editor start; they're dropped from what's posted.
pub(super) const HINT: &str = "<!-- dv:";

/// Drop dv's hint line and trim; untouched prefill alone counts as nothing written.
pub(super) fn clean_body(text: &str, mark: &str, prefill: &str) -> String {
    let body: Vec<&str> = text.lines().filter(|l| !l.starts_with(mark)).collect();
    let body = body.join("\n").trim().to_owned();
    if !prefill.is_empty() && body == prefill.trim() { String::new() } else { body }
}

/// If posting failed, save what was written so it isn't lost, and say where.
pub(super) fn keep_on_error<T>(res: Result<T>, body: &str) -> Result<T> {
    res.map_err(|e| {
        let saved = term::unsent_path();
        match !body.is_empty() && std::fs::write(&saved, body).is_ok() {
            // The reason first: a narrow status bar cuts the end off.
            true => anyhow!("{e:#} (text saved to {})", saved.display()),
            _ => e,
        }
    })
}

/// The summary an approval starts from: `ACK <the PR's head commit>`, as Bitcoin-style projects
/// write it. If what you reviewed (`local`, your HEAD) isn't that commit (your branch has
/// diverged from the PR), a hint line above it says so; hint lines aren't posted. Other
/// reviews start empty.
pub(super) fn ack(event: &str, pr: Option<&Pr>, local: &str) -> String {
    let Some(p) = pr.filter(|p| event == "APPROVE" && !p.head_oid.is_empty()) else { return String::new() };
    let short = |h: &str| h.chars().take(8).collect::<String>();
    match p.head_oid == local || local.is_empty() {
        true => format!("ACK {}", p.head_oid),
        false => format!(
            "{HINT} your checkout is at {}, not the PR's head {}: check this ACK names what you reviewed -->\nACK {}",
            short(local),
            short(&p.head_oid),
            p.head_oid
        ),
    }
}

pub(super) fn posted_msg(review: bool) -> &'static str {
    if review { "added to pending review (S to submit)" } else { "posted" }
}

impl App {
    /// Your editor on `prefill` (what a new PR should do: `untouched_ok`, as it is), then Claude
    /// drafting it in a process of its own (`dv replace`: it carries on if dv quits),
    /// unpublished: dv list's drafts tab has it. Its id (None: not asked).
    fn new_pr(&self, what: &str, prefill: &str, untouched_ok: bool) -> Result<Option<u64>> {
        let Some(task) = compose_with(what, false, prefill, untouched_ok)?.filter(|t| !t.trim().is_empty()) else { return Ok(None) };
        let id = crate::replace::create_new(&task, &self.root)?;
        crate::replace::spawn(&[id])?;
        Ok(Some(id))
    }

    pub(super) fn perform(&mut self, a: Action) -> Result<String> {
        match a {
            Action::Shell => {
                let shell = std::env::var("SHELL").unwrap_or_else(|_| "sh".into());
                let root = self.root.display().to_string();
                if in_zellij() {
                    let name = format!("shell: {}", self.root.file_name().map_or(root.clone(), |n| n.to_string_lossy().into_owned()));
                    let st = std::process::Command::new("zellij")
                        .args(["run", "--floating", "--close-on-exit", "--name", &name, "--cwd", &root, "--", &shell])
                        .stdin(std::process::Stdio::null())
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .status()?;
                    anyhow::ensure!(st.success(), "zellij run failed");
                    return Ok(format!("a shell in {root}, floating (exit closes it)"));
                }
                std::process::Command::new(&shell).current_dir(&self.root).status().context("running your shell")?;
                Ok(String::new())
            }
            Action::Open(target) if in_zellij() => self.open_in_hx_pane(&target),
            Action::Open(target) => {
                interactive("hx", std::slice::from_ref(&target), &format!("hx {target}")).context("running hx")?;
                Ok(String::new())
            }
            Action::Comment { anchor, review, prefill, claude } => {
                let range = match anchor.start {
                    Some((s, _)) => format!("{s}-{}", anchor.line),
                    None => anchor.line.to_string(),
                };
                let kind = match (&claude, prefill.is_empty()) {
                    (Some(_), _) => "Claude's comment",
                    (None, true) => "comment",
                    (None, false) => "suggestion",
                };
                let what = format!("{kind} on {}:{range}", anchor.path);
                self.pr.as_ref().context("no PR")?; // before the editor, so nothing written is lost
                let Some(body) = compose_with(&what, false, &prefill, claude.is_some())? else {
                    return Ok(format!("{kind} cancelled"));
                };
                self.post_later(Post::Thread { anchor, review, body, claude })
            }
            Action::Reply { thread, comment, review, quote } => {
                self.pr.as_ref().context("no PR")?;
                let Some(body) = compose_with("reply", false, &quote, false)? else { return Ok("reply cancelled".into()) };
                self.post_later(Post::Reply { thread, comment, review, body })
            }
            Action::ClaudeReply(k, want, prefill) => {
                let p = self.claude.path.clone().context("no review file")?;
                let what = format!("reply to Claude{}", ask::want_hint(want));
                let Some(body) = compose_change(&what, want, &prefill)? else { return Ok("reply cancelled".into()) };
                keep_on_error(crate::claude::reply(&p, &k, &body, want), &body)?;
                Ok(self.answer_now())
            }
            Action::ClaudeReplyAll(keys, want) => {
                let p = self.claude.path.clone().context("no review file")?;
                let what = format!("one reply to Claude's {} comments{}", keys.len(), ask::want_hint(want));
                let Some(body) = compose_change(&what, want, "")? else { return Ok("reply cancelled".into()) };
                // Each written that can be (one gone meanwhile: said), and those answered.
                let mut errs: Vec<(String, anyhow::Error)> = keys.iter().filter_map(|k| crate::claude::reply(&p, k, &body, want).err().map(|e| (k.clone(), e))).collect();
                if errs.len() == keys.len() {
                    keep_on_error(Err::<(), _>(errs.remove(0).1), &body)?;
                }
                let failed: Vec<String> = errs.into_iter().map(|(k, e)| format!("{k}: {e:#}")).collect();
                let said = self.answer_now();
                Ok(match failed.is_empty() {
                    true => said,
                    false => format!("{said} (not replied to: {})", failed.join("; ")),
                })
            }
            Action::AskClaude { anchor, code, about, want } => {
                let p = self.claude.path.clone().context("no review file")?;
                let range = anchor.start.map_or(anchor.line.to_string(), |(s, _)| format!("{s}-{}", anchor.line));
                let what = match about {
                    Some(_) => format!("ask Claude about this review comment on {}:{range} (it answers right away; nothing goes to GitHub)", anchor.path),
                    None => format!("a comment for Claude on {}:{range}{}", anchor.path, ask::want_hint(want)),
                };
                let prefill = about.as_ref().map_or(String::new(), |(_, q)| q.clone());
                let Some(body) = compose_with(&what, false, &prefill, false)? else { return Ok("comment cancelled".into()) };
                keep_on_error(crate::claude::add_comment(&p, &anchor, code, &body, about.map(|a| a.0), want).map(drop), &body)?;
                Ok(self.answer_now())
            }
            Action::AskConversation { url, prefill, want } => {
                let p = self.claude.path.clone().context("no review file")?;
                // About text: what Claude does with a change to it.
                let text = match (url.as_str(), self.unpublished()) {
                    (_, true) => "Claude redrafts it",
                    ("description", _) => "Claude proposes a new one; you approve it",
                    _ => "Claude drafts it; you edit it before it goes",
                };
                let what = match (url == ON_SUMMARY, commit_of(&url)) {
                    _ if on_text(&url) && want == "change" => format!("what to change in the {} ({text}; nothing goes to GitHub)", text_name(&url)),
                    _ if on_text(&url) => format!("for Claude, about the {}{}", text_name(&url), ask::want_hint(want)),
                    (true, _) => "a question for Claude about the PR, or a reply to its review (it answers in the Claude tab; nothing goes to GitHub)".to_owned(),
                    (_, Some(sha)) => format!("for Claude, about commit {}{}", self.commit_short(sha), ask::want_hint(want)),
                    _ => "ask Claude about this conversation comment (it answers under it; nothing goes to GitHub)".to_owned(),
                };
                // (A closing comment drafted anew needs nothing said: Claude says why from the PR.)
                let fresh = url == ON_CLOSING && self.claude.closing.trim().is_empty();
                let Some(body) = compose_with(&what, fresh, &prefill, fresh)? else { return Ok("question cancelled".into()) };
                let body = match body.trim() {
                    "" => "Draft one.".to_owned(),
                    _ => body,
                };
                // (A question about the PR or a conversation comment: an ask, not a change
                // request left open once answered; a commit's, its picked want.)
                let want = if want.is_empty() && commit_of(&url).is_none() { "ask" } else { want };
                keep_on_error(crate::claude::add_conversation(&p, &url, &body, want).map(drop), &body)?;
                Ok(self.answer_now())
            }
            Action::ClaudeWith(ask) => {
                // Optional (empty goes ahead) unless they're the whole request: a change with no
                // open comments. Quitting the editor with an error (`:cq`) cancels.
                let alone = matches!(ask, agent::Ask::Fix | agent::Ask::Edit) && self.change_requests() == 0;
                let what = match (ask, alone) {
                    (agent::Ask::Review, _) => "anything for Claude to look at or keep in mind in its review (optional)",
                    (agent::Ask::Triage, _) => "instructions for Claude's triage (optional)",
                    (_, true) => "what Claude should change (there are no open comments to act on)",
                    _ => "instructions for Claude, on top of the open comments (optional)",
                };
                // Review (and triage) instructions start from the last ones (kept if left as they
                // are), as dv list's.
                let kept = match ask {
                    agent::Ask::Review => Some("review"),
                    agent::Ask::Triage => Some("triage"),
                    _ => None,
                };
                let last = kept.map_or(String::new(), crate::claude::instructions);
                let Some(extra) = compose_with(what, !alone, last.trim(), kept.is_some())? else { return Ok("Claude not asked".into()) };
                if let Some(k) = kept {
                    _ = crate::claude::save_instructions(k, &extra);
                }
                // A change you say: a thread of its own on the PR (as a question about it is),
                // wanting it as it lands, kept with Claude's answer; with the open comments, if
                // any (else answered as any thread).
                if matches!(ask, agent::Ask::Fix | agent::Ask::Edit) && !extra.trim().is_empty() {
                    let want = match (self.theirs && matches!(ask, agent::Ask::Fix), self.claude.land) {
                        (true, _) => agent::SUGGEST,
                        (_, "amend") => "amend",
                        (_, "commit") => "commit",
                        _ => "change",
                    };
                    let p = self.claude.path.clone().context("no review file")?;
                    let others = self.change_requests();
                    keep_on_error(crate::claude::add_conversation(&p, ON_SUMMARY, &extra, want).map(drop), &extra)?;
                    if others == 0 {
                        return Ok(self.answer_now());
                    }
                    self.load_claude();
                    // (A suggestion isn't among the changes it makes: answered after, as any
                    // thread waiting.)
                    let asked = self.run_claude(ask, "");
                    self.claude.again |= asked && want == agent::SUGGEST;
                    return Ok(std::mem::take(&mut self.msg));
                }
                if self.run_claude(ask, &extra) || extra.trim().is_empty() {
                    return Ok(std::mem::take(&mut self.msg));
                }
                // It couldn't be asked after all: keep what you wrote.
                let why = std::mem::take(&mut self.msg);
                Ok(keep_on_error::<()>(Err(anyhow::anyhow!(why)), &extra).err().map_or(String::new(), |e| format!("{e:#}")))
            }
            Action::Replace(prs) => {
                let prs = crate::replace::target(&prs);
                // Claude at work on it, or its pane open here (`redraft_waits`).
                let waits = || self.redraft_waits(prs[0]);
                if let Some(why) = crate::replace::refused(&prs) {
                    return Ok(why);
                }
                // (What it is, said: a new PR's id is no PR number.)
                let it = match crate::replace::is_new(prs[0]) {
                    true => crate::replace::label(prs[0]),
                    false => format!("the replacement of {}", prs.iter().map(|n| format!("#{n}")).collect::<Vec<_>>().join(", ")),
                };
                // Drafted already: what you want is a thread about it, which its redraft answers
                // (one under way, as it ends).
                if crate::replace::redrafts(prs[0]) {
                    let what = format!("what else Claude should do in its redraft of {it} (optional)");
                    let Some(ask) = compose_with(&what, true, "", true)? else { return Ok("not asked".into()) };
                    if !ask.trim().is_empty() {
                        keep_on_error(crate::replace::ask(prs[0], &ask, "", ""), &ask)?;
                        let keep = self.cursor_key();
                        self.load_claude();
                        self.rebuild(keep);
                        return Ok(self.redraft_to_answer(&prs));
                    }
                    if let Some(why) = waits() {
                        return Ok(why);
                    }
                } else {
                    if let Some(why) = waits() {
                        return Ok(why);
                    }
                    let what = format!("instructions for Claude's draft of {it} (optional)");
                    let Some(extra) = compose_with(&what, true, crate::claude::instructions("replace").trim_end(), true)? else { return Ok("not asked".into()) };
                    crate::claude::save_instructions("replace", &extra)?; // `dv replace` reads them
                    if let Some(why) = waits() {
                        return Ok(format!("not drafted: {why}"));
                    }
                }
                crate::replace::spawn(&prs)?;
                Ok(format!("Claude is drafting {it} in the background: Space a opens it (the status bar says while it's at it)"))
            }
            Action::Publish(close) => {
                // (It rebases and pushes the branch.)
                anyhow::ensure!(!self.claude_busy(), "{}", App::busy_says("publish"));
                let mut r = self.replacement.clone().context("not a replacement")?;
                if crate::replace::remote().is_none() {
                    return Ok(self.ask_replace_remote(close));
                }
                let prefill = format!("{}\n\n{}", r.title.trim(), r.body.trim());
                let kind = "draft PR";
                let what = format!("{kind} for {}: its title (first line), then its description", crate::replace::branch(r.id));
                let Some(text) = compose_with(&what, false, &prefill, true)? else { return Ok("not published".into()) };
                let (title, body) = text.split_once('\n').map_or((text.as_str(), ""), |(t, b)| (t, b));
                let (title, body) = (title.trim(), body.trim());
                if title.is_empty() {
                    return Ok("not published: it needs a title (the first line)".into());
                }
                let res = crate::replace::publish(&mut r, &self.root, title, body, self.claude.session.as_deref());
                keep_on_error(res.as_ref().map(drop).map_err(|e| anyhow!("{e:#}")), &text)?;
                let originals = !r.replaces.is_empty(); // a new PR replaces nothing
                self.show_replacement(r);
                if let Ok(n) = res {
                    self.become_pr(n);
                }
                if close {
                    self.action = Some(Action::CloseOriginals); // its own editor, next
                }
                let then = if originals { ": S closes what it replaces, with the note" } else { "" };
                // Ready for review only once CI passes: `Space g` there, which checks.
                Ok(format!("published as {kind} #{}{then} (ready for review once CI passes: Space g)", res.unwrap_or(0)))
            }
            Action::ClosePr => self.close_pr(),
            Action::CloseWith(m) => self.close_with(m),
            Action::CiFixPr => {
                let n = self.pr.as_ref().map_or(0, |p| p.number);
                let c = self.ci_note(n).context("no fix in mind for CI any more")?;
                let id = self.new_pr("a new PR fixing CI, for Claude to draft: what it should do; unpublished until you publish it", &c.prefill(n), true)?;
                if id.is_some() {
                    crate::claude::ci_fix_drafted(n); // not offered again
                }
                Ok(drafting(id))
            }
            Action::NewPr { prefill, as_is } => Ok(drafting(self.new_pr("a new PR for Claude to draft: what it should do; unpublished until you publish it", &prefill, as_is)?)),
            Action::CloseOriginals => {
                let mut r = self.replacement.clone().context("not a replacement")?;
                let which: Vec<String> = r.replaces.iter().filter(|o| !r.closed.contains(&o.number)).map(|o| format!("#{}", o.number)).collect();
                let what = format!("note closing {} (each gets it as a comment, then is closed)", which.join(", "));
                let Some(note) = compose_with(&what, false, &crate::replace::note(&r), true)? else { return Ok("nothing closed".into()) };
                let res = crate::replace::close_originals(&mut r, &note);
                self.show_replacement(r);
                keep_on_error(res.as_ref().map(drop).map_err(|e| anyhow!("{e:#}")), &note).map(|_| res.unwrap_or_default())
            }
            Action::Converse { quote, as_is } => {
                let what = format!("comment on {}'s conversation", self.subject().context("no PR or issue")?.what());
                let Some(body) = compose_with(&what, false, &quote, as_is)? else {
                    return Ok("comment cancelled".into());
                };
                self.post_later(Post::Converse(body))
            }
            Action::Edit { comment, body: old, author } => {
                // Unchanged or emptied counts as cancelled; deleting a comment is a separate thing.
                // Maintainers can edit others' comments too, so the hint names whose this is.
                let what = format!("editing @{author}'s comment");
                let Some(body) = compose_with(&what, false, &old, false)? else { return Ok("edit cancelled".into()) };
                self.post_later(Post::Edit { comment, body })
            }
            Action::Submit(event) => {
                // An empty summary is fine; quitting the editor with an error (`:cq`) cancels.
                // Approving starts from "ACK <the PR's head commit>", posted as is if kept.
                let what = match event {
                    review::CLOSE => "closing comment (empty: just close the PR)".to_owned(),
                    e => format!("review summary ({})", e.to_lowercase().replace('_', " ")),
                };
                let head = diff::git(&["rev-parse", "HEAD"]).unwrap_or_default();
                let on = self.pr.as_ref().map(|p| p.head_oid.clone()).unwrap_or_default();
                if event == "APPROVE" {
                    self.pending_on(Some(&on))?; // (before the editor: its ACK names this head)
                }
                let mut prefill = ack(event, self.pr.as_ref(), head.trim());
                if !self.summary_draft.is_empty() {
                    prefill = format!("{}\n\n{prefill}", self.summary_draft.trim_end());
                }
                if event == "APPROVE" {
                    // (A checkout that isn't the PR's head: the ACK's own hint says so.)
                    let notes = self.submit_notes().into_iter().filter(|n| !n.starts_with("your checkout"));
                    let notes: String = notes.map(|n| format!("{HINT} still open: {n} -->\n")).collect();
                    prefill = notes + &prefill;
                }
                let Some(body) = compose_with(&what, true, &prefill, true)? else {
                    return Ok("review not submitted".into());
                };
                self.summary_draft.clear();
                self.post_later(Post::Submit(event, body, on))
            }
            Action::Commit => {
                anyhow::ensure!(!self.claude_busy(), "{}", App::busy_edit("commit"));
                // Claude writes the message (then your editor has it); no review file to keep
                // it beside: the draft from its replies, as before.
                match self.claude.path.is_some() {
                    true => self.commit_ask(),
                    false => {
                        let draft = self.commit_draft();
                        self.commit(&draft)
                    }
                }
            }
            Action::CommitWith(msg) => self.commit(&msg),
        }
    }

    /// `Enter` inside zellij: open the file in dv's hx pane, reusing it if it's still open
    /// (`:open` + `:goto` typed into it, after `Esc` for normal mode), else in a new pane below
    /// dv. dv keeps working either way.
    pub(super) fn open_in_hx_pane(&mut self, target: &str) -> Result<String> {
        if let Some(id) = self.hx.as_ref().map(|h| h.pane.clone()).filter(|id| crate::pane::alive(id)) {
            // `target` is path:line:col.
            let mut parts = target.rsplitn(3, ':');
            let (_col, line, path) = (parts.next(), parts.next().unwrap_or("1"), parts.next().unwrap_or(target));
            crate::pane::action(&["write-chars", "-p", &id, "\x1b"])?;
            crate::pane::action(&["write-chars", "-p", &id, &format!(":open {path}\r:goto {line}\r")])?;
            crate::pane::action(&["focus-pane-id", &id])?;
            return Ok("opened in hx (its pane below); dv reloads as you save".into());
        }
        let (st, id) = term::zellij_pane("hx", &[target.to_owned()], "hx", &self.root, false)?;
        anyhow::ensure!(st.success(), "zellij run failed ({st})");
        self.hx = id.map(|pane| HxPane { pane });
        Ok("hx is open below; dv reloads as you save, and Enter opens the next file there".into())
    }
}

impl App {
    /// Between frames: what was asked that takes the editor (or its text, written beside dv
    /// once its pane is done) done, and the diff reloaded after.
    pub(super) fn act(&mut self, term: &mut DefaultTerminal) {
        // Written (the editor's pane closed): what asked for it goes on, given the text
        // (once nothing else is to be done first).
        let (mut given, mut kept) = (None, None); // (set below, once the pane's done)
        if self.action.is_none()
            && !self.mid_sequence()
            && let Some(text) = self.writing.as_mut().and_then(|w| w.1.done())
        {
            let (a, w) = self.writing.take().unwrap();
            self.quit_armed = None; // (what held leaving is over: a merge held with it too)
            (self.action, given, kept) = (Some(a), Some(text), w.kept);
            focus_dv();
        }
        if let Some(a) = self.action.take() {
            // What was selected for it: used (the action has its lines). (Resumed, its text
            // written: ended when it first ran; what's selected now is another's.)
            if given.is_none() {
                (self.select, self.anchor) = (false, self.head);
            }
            // In zellij the editor gets a pane below and dv stays on screen above it.
            let in_pane = in_zellij();
            if !in_pane {
                restore_terminal();
            }
            // Anything to post is queued as a job (it runs after a "posting…" frame, then
            // refetches); here only the diff reloads, as hx may have changed files.
            let again = a.clone();
            let (res, left) = term::writing_beside(self.writing.is_some(), given.take(), || self.perform(a));
            // The editor in its pane, dv going on: the rest once it's written.
            if res.as_ref().is_err_and(|e| e.is::<term::Writing>()) {
                let w = res.unwrap_err().downcast().unwrap();
                self.writing = Some((again, w));
                let before = self.chained.as_ref().map_or(String::new(), |c| format!("{c}; "));
                self.msg = format!("{before}writing in the editor's pane: dv's yours meanwhile (save and quit there to send; :cq cancels)");
                return;
            }
            // Written, but what asked gave up before its editor: kept, not lost.
            let res = match left.flatten().filter(|b| !b.trim().is_empty()) {
                Some(body) => {
                    let why = res.as_ref().map_or_else(|e| format!("{e:#}"), String::clone);
                    Err(keep_on_error::<()>(Err(anyhow!("{why}: what you wrote wasn't sent")), &body).unwrap_err())
                }
                None => res,
            };
            if !in_pane {
                *term = init_terminal();
            }
            self.reloaded(res, false);
            if let Some(k) = kept.take() {
                self.msg = format!("{}: its pane closed; what it had saved is kept in {}", self.msg, k.display());
            }
            // One leading to another (closing, then what closes with it): what each came to,
            // said together at the end, not each over the one before (a failure, where its text
            // was kept).
            let said = std::mem::take(&mut self.msg);
            let all = [self.chained.take(), Some(said)].into_iter().flatten().filter(|m| !m.is_empty()).collect::<Vec<_>>().join("; ");
            match self.action.is_some() {
                true => self.chained = Some(all),
                false => self.msg = all,
            }
        }
    }
}

/// What you write to Claude (`compose_with`): wanting the code changed, left empty (or only
/// the quote it started with) it's "what your comment says".
fn compose_change(what: &str, want: &str, prefill: &str) -> Result<Option<String>> {
    if !ask::edits_code(want) {
        return compose_with(what, false, prefill, false);
    }
    let what = format!("{what}; left empty: what Claude's comment says");
    Ok(compose_with(&what, true, prefill, true)?.map(|b| match b.trim().is_empty() || b.trim() == prefill.trim() {
        true => format!("{prefill}Make the change your comment describes.").trim_start().to_owned(),
        false => b,
    }))
}

/// What's said of a new PR started (`App::new_pr`), or not.
fn drafting(id: Option<u64>) -> String {
    match id {
        Some(id) => format!("Claude is drafting {} in the background: dv list's drafts tab shows it (Enter there reviews it, S publishes it)", crate::replace::label(id)),
        None => "not asked".into(),
    }
}
