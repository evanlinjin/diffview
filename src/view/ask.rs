//! `a`: writing to Claude: on what (code lines, a thread, a commit, the conversation, a draft),
//! and what you want of it (an answer, a suggestion for GitHub, the change made here).
use super::*;

/// What the editor says Claude will do with what you write, for `want`.
pub(super) fn want_hint(want: &str) -> &'static str {
    WANTS.iter().find(|w| w.0 == want).map_or(" (it answers right away; nothing goes to GitHub)", |w| w.2)
}

/// Whether wanting `w` has Claude change the code: here, amended into a commit, or as a new one.
pub(super) fn edits_code(w: &str) -> bool {
    matches!(w, "change" | "amend" | "commit")
}

/// What you can want of Claude on a comment or reply (its `"want"` in the review file): the
/// key, the thread's tag while it waits, what the editor says, the picker's item.
pub(super) const WANTS: [(&str, &str, &str, &str); 5] = [
    ("ask", "[ask] ", " (Claude answers; it changes nothing)", "Ask: Claude answers (it changes nothing)"),
    (
        "suggest",
        "[suggest] ",
        " (Claude drafts a suggestion for you to post; it doesn't edit the code)",
        "Suggest: Claude drafts a suggestion for you to post on GitHub (no code changes)",
    ),
    ("change", "[change] ", " (Claude makes the change in the code here, uncommitted; nothing goes to GitHub)", "Change: Claude edits the code here (left uncommitted)"),
    (
        "amend",
        "[amend] ",
        " (Claude makes the change and amends it into the commit it belongs in; nothing goes to GitHub)",
        "Amend: Claude makes the change and amends it into the commit it belongs in",
    ),
    (
        "commit",
        "[commit] ",
        " (Claude makes the change as a new commit; nothing goes to GitHub)",
        "Commit: Claude makes the change as a new commit",
    ),
];

impl App {
    /// `a`: leave a comment for Claude on the selected lines (or the cursor's), in the review
    /// file; Claude answers right away, as it does replies. Nothing goes to GitHub.
    pub(super) fn start_ask_claude(&mut self) {
        if self.claude.path.is_none() {
            return self.msg = "no review file here (not in a git repo?)".into();
        }
        // On text (a description, a draft's closing note, Claude's closing comment): a thread
        // about it, with what it's about quoted.
        if let Some(on) = self.talk_text_at(self.head.row) {
            return self.ask_text(on);
        }
        if self.on_summary() {
            return self.ask_summary();
        }
        // A published replacement's closing note: yours to change as it goes out.
        if self.section_of(self.head.row) == Some(CLOSING_NOTE) {
            return self.msg = "S closes what it replaces with it, after your editor has it to change".into();
        }
        // On a comment in the PR's conversation: a question about it, under it.
        if let Some((url, prefill)) = self.conversation_at_cursor() {
            return self.action = Some(Action::AskConversation { url, prefill, want: "" });
        }
        // Several of Claude's threads selected: one reply to them all.
        if let Some(keys) = self.selected_claude_threads() {
            return self.pick_want(Action::ClaudeReplyAll(keys, ""), None);
        }
        // On a thread of Claude's review (its comment, or your question and its answer): a reply
        // there, quoting what's selected of it.
        if let Some((t, k)) = self.claude_at_cursor() {
            let quote = self.selected_quote().map_or(String::new(), |q| format!("About this:\n{q}\n"));
            let reply = Action::ClaudeReply(k, "", quote);
            return match self.threads[t].conversation.is_empty() {
                true => self.pick_want(reply, Some(t)),
                false => self.action = Some(reply),
            };
        }
        // On a GitHub review thread: ask Claude about the comment under the cursor.
        if let Some(t) = self.rows[self.head.row].thread.filter(|&t| self.threads[t].claude.is_none()) {
            return self.ask_about(t);
        }
        let anchor = match self.selection_anchor() {
            Ok(a) => a,
            Err(_) => return self.msg = "a on code lines leaves Claude a comment there".into(),
        };
        let anchor = self.one_sided(anchor);
        // Their text, so the comment follows its code (as review-check records for Claude's).
        let first = anchor.start.map_or(anchor.line, |s| s.0);
        let code = self.files.iter().find(|f| f.path == anchor.path).and_then(|f| crate::claude::code_at(f, anchor.left, first, anchor.line));
        match code {
            Some(code) => self.pick_want(Action::AskClaude { anchor, code, about: None, want: "" }, None),
            None => self.msg = "those lines aren't in this diff".into(),
        }
    }

    /// Before writing a comment or reply for Claude: what you want of it (an answer, a
    /// suggestion for GitHub, or the change made here). Preselected: what you last wanted on
    /// thread `t`, else the change on your PR, a suggestion on theirs.
    pub(super) fn pick_want(&mut self, action: Action, t: Option<usize>) {
        if self.claude.path.is_none() {
            return self.msg = "no review file here (not in a git repo?)".into();
        }
        // About text (a description, a closing comment): an answer, or the change to it (no
        // code, nor anything to post). Else an issue: no code to change or suggest for; asked,
        // Claude answers.
        let text = matches!(&action, Action::AskConversation { url, .. } if on_text(url));
        if !self.has_code() && !text {
            self.wanting = Some(action);
            return self.want("ask");
        }
        // About a commit (on no lines): no suggestion to post; an answer, unless you said.
        let commit = matches!(&action, Action::AskConversation { url, .. } if commit_of(url).is_some());
        let wants: Vec<_> = WANTS.iter().filter(|w| !((commit || text) && w.0 == "suggest") && !(text && matches!(w.0, "amend" | "commit"))).collect();
        let last = t.and_then(|t| wants.iter().position(|w| w.0 == self.threads[t].want));
        let usual = match (commit, text, self.theirs) {
            (true, ..) => "ask",
            (_, true, _) | (false, false, false) => "change",
            (false, false, true) => "suggest",
        };
        // (A draft's: redrafted; else Claude proposes it, for you to approve or use.)
        let draft = self.unpublished();
        let sel = last.or_else(|| wants.iter().position(|w| w.0 == usual)).unwrap_or(0);
        let items = wants
            .iter()
            .map(|&&(key, _, _, label)| match (key, text, self.theirs) {
                ("change", true, _) if text && draft => Item::new("Change: Claude redrafts it (its drafts, and the code)", Target::Want(key)),
                ("change", true, _) if text => Item::new("Change: Claude proposes it anew (you approve it, or use it)", Target::Want(key)),
                ("change", false, true) => Item::new(format!("{label} (their PR: Space g pushes what you commit)"), Target::Want(key)),
                ("amend", false, true) => Item::new(format!("{label} (their PR: Space g force-pushes it)"), Target::Want(key)),
                ("commit", false, true) => Item::new(format!("{label} (their PR: Space g pushes it)"), Target::Want(key)),
                _ => Item::new(label, Target::Want(key)),
            })
            .collect();
        self.wanting = Some(action);
        self.picker = Some(Picker { sel, ..Picker::new(" what do you want of Claude? ", items) });
    }

    /// The want picked: on to the editor.
    pub(super) fn want(&mut self, w: &'static str) {
        self.action = match self.wanting.take() {
            Some(Action::AskClaude { anchor, code, about, .. }) => Some(Action::AskClaude { anchor, code, about, want: w }),
            Some(Action::ClaudeReply(k, _, q)) => Some(Action::ClaudeReply(k, w, q)),
            Some(Action::ClaudeReplyAll(ks, _)) => Some(Action::ClaudeReplyAll(ks, w)),
            Some(Action::AskConversation { url, prefill, .. }) => Some(Action::AskConversation { url, prefill, want: w }),
            other => other,
        };
    }

    /// Claude's threads on lines in the selection, when it spans more than one (`r`/`a` reply to
    /// them all, once): their keys, in order.
    pub(super) fn selected_claude_threads(&self) -> Option<Vec<String>> {
        if !self.selecting() {
            return None;
        }
        let (s, e) = self.ordered();
        let mut keys: Vec<String> = vec![];
        for r in s.row..=e.row {
            let Some(t) = self.rows[r].thread else { continue };
            let th = &self.threads[t];
            if let Some(k) = th.claude.as_ref().filter(|_| th.conversation.is_empty())
                && !keys.contains(k)
            {
                keys.push(k.clone());
            }
        }
        (keys.len() > 1).then_some(keys)
    }

    /// `a` on a commit in the list: a question about it, or a change to it, for Claude (on no
    /// lines: a thread of its own, under "about commit …").
    pub(super) fn ask_commit(&mut self, i: usize) {
        let Some(c) = self.commits.get(i) else { return };
        let url = match &self.pr {
            Some(p) => format!("{}/commits/{}", p.url, c.sha),
            None => format!("commit:{}", c.sha),
        };
        let prefill = format!("About commit {} ({}):\n", c.short, c.subject);
        // A thread about it already: what you last wanted there.
        let t = self.threads.iter().rposition(|t| t.conversation == url);
        self.pick_want(Action::AskConversation { url, prefill, want: "" }, t);
    }

    /// Anchor `a` on its end's side's lines, what Claude's review file and the code quoted read: a
    /// range from the other side (removed to added, GitHub's too) from the first line of this
    /// side there.
    fn one_sided(&self, mut a: Anchor) -> Anchor {
        let file = self.files.iter().find(|f| f.path == a.path);
        a.start = a.start.and_then(|s| start_on_side(file?, s, a.left)).filter(|&s| s < a.line).map(|s| (s, a.left));
        a
    }

    /// The text row `r` is on, to talk about (its `"conversation"`): a draft's description or
    /// closing note (`replacement_draft`), a PR's or issue's description, Claude's closing
    /// comment. Not a thread beside it (`r` there goes on in it).
    pub(super) fn talk_text_at(&self, r: usize) -> Option<&'static str> {
        if self.rows[r].thread.is_some() {
            return None;
        }
        if let Some(which) = self.replacement_draft(r) {
            return Some(which);
        }
        match self.section_of(r) {
            Some(DESCRIPTION) => Some("description"),
            Some(CLOSING_PR) => Some(ON_CLOSING),
            _ => None,
        }
    }

    /// `a` on text (`talk_text_at`): a thread about it, its selected lines (the cursor's) quoted;
    /// what you want of Claude first: an answer, or the change (proposed, redrafted).
    pub(super) fn ask_text(&mut self, on: &'static str) {
        let (s, e) = self.ordered();
        // Its own text: its section's lines of markdown (not its header, the line saying what
        // a draft replaces, nor another's).
        let text = |r: usize| {
            let row = &self.rows[r];
            self.talk_text_at(r) == Some(on) && row.main.is_some() && row.line.kind != Kind::File && row.info.is_some_and(|i| self.info.get(i).is_some_and(|l| l.md && !l.text.trim().is_empty()))
        };
        let quote: String = (s.row..=e.row).filter(|&r| text(r)).map(|r| format!("> {}\n", self.rows[r].line.text.trim_end())).collect();
        let name = if on == ON_CLOSING { "closing comment" } else { on };
        let t = self.threads.iter().rposition(|t| t.conversation == on);
        self.pick_want(Action::AskConversation { url: on.into(), prefill: format!("About the {name}:\n{quote}\n"), want: "" }, t);
    }

    /// On Claude's review summary (not a thread under it).
    pub(super) fn on_summary(&self) -> bool {
        let row = &self.rows[self.head.row];
        row.info == Some(SUMMARY) && row.thread.is_none()
    }

    /// `a` or `r` on Claude's review summary: a question of its own, about what's selected
    /// there (quoted). (A thread under it goes on with `r` on it.)
    pub(super) fn ask_summary(&mut self) {
        let prefill = match self.selected_quote() {
            Some(q) => format!("About this part of your review:\n{q}"),
            None => String::new(),
        };
        self.action = Some(Action::AskConversation { url: ON_SUMMARY.into(), prefill, want: "" });
    }

    /// `a` on a GitHub review thread: a comment for Claude on its lines, asking about the comment
    /// under the cursor (quoted unless it's hidden on GitHub, with its link, else its id, so
    /// Claude reads it and the thread).
    pub(super) fn ask_about(&mut self, t: usize) {
        // (Its lines as here, not the PR head's: Claude reads the code here.)
        let th = self.here(&self.threads[t]).into_owned();
        let Some(line) = th.line else { return self.msg = "this thread is outdated: its lines aren't in the diff to ask about".into() };
        let Some(c) = self.comment_under_cursor(t) else { return self.msg = "move onto a comment to ask about it".into() };
        let anchor = self.one_sided(Anchor { path: th.path.clone(), line, left: th.left, start: th.range_start(), ..Default::default() });
        let first_line = anchor.start.map_or(line, |s| s.0);
        let Some(code) = self.files.iter().find(|f| f.path == anchor.path).and_then(|f| crate::claude::code_at(f, anchor.left, first_line, line)) else {
            return self.msg = "those lines aren't in this diff".into();
        };
        let link = self.pr.as_ref().map(|p| format!(" ({}#discussion_r{})", p.url, c.id)).unwrap_or_default();
        // (Hidden on GitHub, unseen: not quoted; said so, for Claude to read it there.)
        let quote: String = match self.seen_body(c) {
            Some(body) => body.lines().map(|l| format!("> {l}\n")).collect(),
            None => format!("— {}, not shown here\n", hidden_words(&c.hidden)),
        };
        // (Its id, where there's no link with it.)
        let link = if link.is_empty() { format!(" (id {})", c.id) } else { link };
        let prefill = format!("About @{}'s comment{link}:\n{quote}\n", c.author);
        // (A question: answered, not a change request left open for "make the changes".)
        self.action = Some(Action::AskClaude { anchor, code, about: Some((c.id, prefill)), want: "ask" });
    }

    /// The conversation comment under the cursor (its author line or text): its URL, and the
    /// start of a question about it (who, the link, the comment quoted).
    pub(super) fn conversation_at_cursor(&self) -> Option<(String, String)> {
        let row = &self.rows[self.head.row];
        let body = row.info.filter(|_| row.file == INFO && row.thread.is_none() && row.main.is_none())?;
        let text = self.info.get(body).filter(|l| l.md)?;
        let head = self.info.get(body.checked_sub(1)?).filter(|h| h.heading && !h.url.is_empty() && !h.event)?;
        let who = head.text.split(" · ").next().unwrap_or("");
        // What's selected of it, else all of it.
        // (Hidden on GitHub, unseen: not quoted; Claude reads it at the link.)
        let quote = match self.seen_info(body) {
            None => format!("— {}, not shown here", hidden_words(&head.hidden)),
            Some(_) => self.selected_quote().unwrap_or_else(|| super::quote(&text.text)),
        };
        Some((head.url.clone(), format!("About {who}'s comment ({}):\n{quote}", head.url)))
    }

    /// The selected text, quoted (`> `), when there's a selection.
    pub(super) fn selected_quote(&self) -> Option<String> {
        self.selecting().then(|| super::quote(self.selection_text(false).trim_end())).filter(|q| !q.is_empty())
    }

    /// Which of a replacement's drafts row `r` is a line of: "description" or "closing note".
    pub(super) fn replacement_draft(&self, r: usize) -> Option<&'static str> {
        // (Published, it's a PR: its description is GitHub's, proposed anew as any PR's.)
        self.replacement.as_ref().filter(|_| self.unpublished())?;
        let row = &self.rows[r];
        // Its section's; beside one (what you asked of it), the draft its text is.
        match row.main {
            Some(DESCRIPTION) => return Some("description"),
            Some(CLOSING_NOTE) => return Some("closing note"),
            _ => {}
        }
        let i = row.info.filter(|_| row.file == INFO)?;
        let heading = self.info.get(i.checked_sub(1)?).filter(|h| h.heading && self.info[i].md)?;
        [("description", "description"), ("closing note", "closing note")].into_iter().find(|(h, _)| heading.text.starts_with(h)).map(|x| x.1)
    }
}
