//! `a`: writing to Claude: on what (code lines, a thread, a commit, the conversation, a draft),
//! and what you want of it (an answer, a suggestion for GitHub, the change made here).
use super::*;
use agent::Ask;

/// What the editor says Claude will do with what you write, for `want`.
pub(super) fn want_hint(want: &str) -> &'static str {
    WANTS.iter().find(|w| w.0 == want).map_or(" (it answers right away; nothing goes to GitHub)", |w| w.2)
}

/// What you can want of Claude on a comment or reply (its `"want"` in the review file): the
/// key, the thread's tag while it waits, what the editor says, the picker's item.
pub(super) const WANTS: [(&str, &str, &str, &str); 3] = [
    ("ask", "[ask] ", " (Claude answers; it changes nothing)", "Ask: Claude answers (it changes nothing)"),
    (
        "suggest",
        "[suggest] ",
        " (Claude drafts a suggestion for you to post; it doesn't edit the code)",
        "Suggest: Claude drafts a suggestion for you to post on GitHub (no code changes)",
    ),
    ("change", "[change] ", " (Claude makes the change in the code here; nothing goes to GitHub)", "Change: Claude edits the code here"),
];

impl App {
    /// `a`: leave a comment for Claude on the selected lines (or the cursor's), in the review
    /// file; Claude answers right away, as it does replies. Nothing goes to GitHub.
    pub(super) fn start_ask_claude(&mut self) {
        // On a replacement's drafts: a redraft, with what it's about quoted.
        if let Some(which) = self.replacement_draft(self.head.row) {
            let (s, e) = self.ordered();
            let info = self.rows[self.head.row].info;
            // (The draft's own lines: not what you asked of it, beside it.)
            let lines: Vec<String> = (s.row..=e.row)
                .filter(|&r| self.rows[r].info == info && self.rows[r].main.is_some() && self.rows[r].line.kind != Kind::File)
                .map(|r| format!("> {}", self.rows[r].line.text.trim_end()))
                .collect();
            let prs = self.replacement.as_ref().map(crate::replace::Record::ids).unwrap_or_default();
            self.select = false;
            // What you want of it first, as on a PR's comments.
            return self.pick_want(Action::Replace(prs, Some((which, format!("About the {which}:\n{}\n", lines.iter().map(|l| format!("{l}\n")).collect::<String>()), ""))), None);
        }
        if self.claude.path.is_none() {
            return self.msg = "no review file here (not in a git repo?)".into();
        }
        // On Claude's review summary: a reply to it, about what's selected there (quoted).
        let row = &self.rows[self.head.row];
        if row.info == Some(SUMMARY) && row.thread.is_none() {
            let prefill = match self.selected_quote() {
                Some(q) => format!("About this part of your review:\n{q}"),
                None => String::new(),
            };
            self.select = false;
            return self.action = Some(Action::AskConversation { url: ON_SUMMARY.into(), prefill, want: "" });
        }
        // A published replacement's closing note: yours to change as it goes out.
        if self.section_of(self.head.row) == Some(CLOSING_NOTE) {
            return self.msg = "S closes what it replaces with it, after your editor has it to change".into();
        }
        // On the PR's description: Claude proposes a new one (more changes, if it has).
        if self.section_of(self.head.row) == Some(DESCRIPTION) {
            return self.pick_claude(Ask::Describe);
        }
        // On Claude's closing comment: changes to it.
        if self.section_of(self.head.row) == Some(CLOSING_PR) {
            return self.pick_claude(Ask::Close);
        }
        // On a comment in the PR's conversation: a question about it, under it.
        if let Some((url, prefill)) = self.conversation_at_cursor() {
            return self.action = Some(Action::AskConversation { url, prefill, want: "" });
        }
        // On a thread of Claude's review (its comment, or your question and its answer): a reply
        // there, quoting what's selected of it.
        if let Some((t, k)) = self.claude_at_cursor() {
            let quote = self.selected_quote().map_or(String::new(), |q| format!("About this:\n{q}\n"));
            self.select = false;
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
        // Line numbers are per side; a range across removed and added lines has no one side.
        if anchor.start.is_some_and(|(_, left)| left != anchor.left) {
            return self.msg = "select lines on one side (removed, or added/unchanged) to leave Claude a comment".into();
        }
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
        // About a commit (on no lines): no suggestion to post; an answer, unless you said. A
        // draft's: nothing goes to GitHub from it either; the change, unless you said.
        let commit = matches!(&action, Action::AskConversation { url, .. } if commit_of(url).is_some());
        let draft = matches!(&action, Action::Replace(..));
        let wants: Vec<_> = WANTS.iter().filter(|w| !((commit || draft) && w.0 == "suggest")).collect();
        let last = t.and_then(|t| wants.iter().position(|w| w.0 == self.threads[t].want));
        let usual = match (commit, draft, self.theirs) {
            (true, ..) => "ask",
            (_, true, _) | (false, false, false) => "change",
            (false, false, true) => "suggest",
        };
        let sel = last.or_else(|| wants.iter().position(|w| w.0 == usual)).unwrap_or(0);
        let items = wants
            .iter()
            .map(|&&(key, _, _, label)| match (key, draft, self.theirs) {
                ("change", true, _) => Item::new("Change: Claude redrafts it (its drafts, and the code)", Target::Want(key)),
                ("change", false, true) => Item::new(format!("{label} (their PR: Space g pushes what you commit)"), Target::Want(key)),
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
            Some(Action::AskConversation { url, prefill, .. }) => Some(Action::AskConversation { url, prefill, want: w }),
            Some(Action::Replace(prs, Some((on, quote, _)))) => Some(Action::Replace(prs, Some((on, quote, w)))),
            other => other,
        };
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

    /// `a` on a GitHub review thread: a comment for Claude on its lines, asking about the comment
    /// under the cursor (quoted unless it's hidden on GitHub, with its link, else its id, so
    /// Claude reads it and the thread).
    pub(super) fn ask_about(&mut self, t: usize) {
        let th = &self.threads[t];
        let Some(line) = th.line else { return self.msg = "this thread is outdated: its lines aren't in the diff to ask about".into() };
        let Some(c) = self.comment_under_cursor(t) else { return self.msg = "move onto a comment to ask about it".into() };
        let anchor = Anchor { path: th.path.clone(), line, left: th.left, start: th.start_line.filter(|&s| s < line).map(|s| (s, th.left)) };
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
        self.action = Some(Action::AskClaude { anchor, code, about: Some((c.id, prefill)), want: "" });
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
        (self.select || self.anchor != self.head).then(|| super::quote(self.selection_text(false).trim_end())).filter(|q| !q.is_empty())
    }

    /// Which of a replacement's drafts row `r` is a line of: "description" or "closing note".
    pub(super) fn replacement_draft(&self, r: usize) -> Option<&'static str> {
        // (Published, it's a PR: its description is GitHub's, proposed anew as any PR's.)
        self.replacement.as_ref().filter(|_| self.unpublished())?;
        let row = &self.rows[r];
        let i = row.info.filter(|_| row.file == INFO)?;
        let heading = self.info.get(i.checked_sub(1)?).filter(|h| h.heading && self.info[i].md)?;
        [("description", "description"), ("closing note", "closing note")].into_iter().find(|(h, _)| heading.text.starts_with(h)).map(|x| x.1)
    }
}
