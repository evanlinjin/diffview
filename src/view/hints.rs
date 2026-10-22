//! What the keys do, where you are: the help popup, the context hints on the status bar, hover text.
use super::*;

impl App {
    /// The help text, styled: section headings, keys (separators dimmed), descriptions wrapped
    /// with a hanging indent under the description column, `(notes)` dimmed. With a `query`
    /// (smart case), only matching entries and their headings are kept, matches highlighted.
    pub(super) fn help_lines(&self, width: usize, query: &str) -> Vec<Line<'static>> {
        help_lines(HELP, &self.ui, width, query)
    }
}

/// `help` (dv's, or dv list's: sections, a heading each, then "  <key, to column 23><what it
/// does>"), styled: headings, keys (separators dimmed), descriptions wrapped with a hanging
/// indent, `(notes)` dimmed; two columns when the window fits them unwrapped. With a `query`
/// (smart case), only matching entries and their headings are kept, matches highlighted.
pub(crate) fn help_lines(help: &str, ui: &Ui, width: usize, query: &str) -> Vec<Line<'static>> {
    {
        const KEY_COL: usize = 23; // entries are "  <key, padded to col 23><description>"
        let dim = Style { fg: ui.stale.fg, ..Style::new() };
        let smart = !query.chars().any(char::is_uppercase);
        let fold = |s: &str| if smart { s.to_lowercase() } else { s.to_owned() };
        let q = fold(query);
        // Char positions in `s` covered by a match of the query.
        let hits = |s: &str| -> Vec<bool> {
            let (chars, lower): (Vec<char>, Vec<char>) = (s.chars().collect(), fold(s).chars().collect());
            let qc: Vec<char> = q.chars().collect();
            let mut on = vec![false; chars.len()];
            if !qc.is_empty() && lower.len() == chars.len() {
                for i in 0..=lower.len().saturating_sub(qc.len()) {
                    if lower[i..].starts_with(&qc) {
                        on[i..i + qc.len()].iter_mut().for_each(|x| *x = true);
                    }
                }
            }
            on
        };
        // Sections: heading + entries, keeping only matching entries.
        let mut sections: Vec<(&str, Vec<&str>)> = vec![];
        // An entry: its line, and those going on under its description (blank where the key
        // is), kept or left out together.
        let mut entries: Vec<(&str, Vec<Vec<&str>>)> = vec![];
        for l in help.trim_matches('\n').lines() {
            match l {
                "" => {}
                h if !h.starts_with(' ') => entries.push((h, vec![])),
                e => match entries.last_mut() {
                    Some((_, es)) if e.starts_with(&" ".repeat(KEY_COL)) && !es.is_empty() => {
                        if let Some(x) = es.last_mut() {
                            x.push(e);
                        }
                    }
                    Some((_, es)) => es.push(vec![e]),
                    None => {}
                },
            }
        }
        for (h, es) in entries {
            let kept = es.into_iter().filter(|e| q.is_empty() || e.iter().any(|l| fold(l).contains(&q)));
            sections.push((h, kept.flatten().collect()));
        }
        sections.retain(|s| !s.1.is_empty());
        if sections.is_empty() {
            return vec![Line::styled(format!(" no keys match \"{query}\""), dim)];
        }
        // Two columns when the window fits two unwrapped ones (so there's no scrolling).
        const GAP: usize = 4;
        let natural = sections.iter().flat_map(|s| &s.1).map(|l| l.chars().count()).max().unwrap_or(0) + 1;
        let two = sections.len() > 1 && width >= 2 * natural + GAP;
        let width = if two { natural } else { width.min(97) };
        let mut blocks: Vec<Vec<Line<'static>>> = vec![];
        for (heading, entries) in sections {
            // Styled as a span, not a line: in two columns lines are joined span by span.
            let mut out = vec![Line::from(Span::styled(format!(" {heading}"), ui.heading.add_modifier(ratatui::style::Modifier::BOLD)))];
            for l in entries {
                let (key, desc) = l.split_at(KEY_COL.min(l.len()));
                let mut spans: Vec<Span> = vec![Span::raw("  ")];
                for (c, hit) in key[2..].chars().zip(hits(&key[2..])) {
                    let st = if c == ',' { dim } else { ui.key }; // `/` is a key itself (search)
                    spans.push(Span::styled(c.to_string(), if hit { st.patch(ui.selection) } else { st }));
                }
                let mut depth = 0; // inside "(...)", which may wrap
                for (i, line) in wrap(desc, width.saturating_sub(KEY_COL).max(10)).into_iter().enumerate() {
                    if i > 0 {
                        out.push(Line::from(std::mem::take(&mut spans)));
                        spans.push(Span::raw(" ".repeat(KEY_COL)));
                    }
                    // Dim parenthesised notes, e.g. "(pending review)".
                    for (c, hit) in line.chars().zip(hits(&line)) {
                        depth += (c == '(') as i32;
                        let st = if depth > 0 { dim } else { Style::new() };
                        spans.push(Span::styled(c.to_string(), if hit { st.patch(ui.selection) } else { st }));
                        depth -= (c == ')') as i32;
                    }
                }
                out.push(Line::from(spans));
            }
            blocks.push(out);
        }
        let join = |bs: &[Vec<Line<'static>>]| -> Vec<Line<'static>> {
            bs.iter().enumerate().flat_map(|(n, b)| (n > 0).then(Line::default).into_iter().chain(b.iter().cloned())).collect()
        };
        if !two {
            return join(&blocks);
        }
        // Split between sections where the columns come out most even.
        let len = |bs: &[Vec<Line>]| bs.iter().map(|b| b.len() + 1).sum::<usize>();
        let k = (1..blocks.len()).min_by_key(|&k| len(&blocks[..k]).abs_diff(len(&blocks[k..]))).unwrap_or(1);
        let (left, right) = (join(&blocks[..k]), join(&blocks[k..]));
        (0..left.len().max(right.len()))
            .map(|i| {
                let mut l = left.get(i).cloned().unwrap_or_default();
                let pad = (natural + GAP).saturating_sub(l.width());
                l.spans.push(Span::raw(" ".repeat(pad)));
                l.spans.extend(right.get(i).cloned().unwrap_or_default().spans);
                l
            })
            .collect()
    }
}

impl App {
    /// Hover markdown, highlighted (code fences in their language), with the fence lines dropped.
    pub(super) fn hover_lines(&self, md: &str) -> Vec<Line<'static>> {
        let cw = super::render::popup_text_w(self.width).min(97);
        let code_bg = self.ui.md.code_bg;
        crate::md::render(md, cw, &self.ui.md, &self.hl, None)
            .into_iter()
            .map(|l| {
                // A code block's line (not one ending in inline code): its background its line's,
                // for `code_to_edge` to take to the popup's edge.
                let code = code_bg.filter(|&bg| l.nowrap && l.styles.last().copied().flatten().and_then(|s| s.bg) == Some(bg));
                match code {
                    Some(bg) => l.into_line().style(Style::new().bg(bg)),
                    None => l.into_line(),
                }
            })
            .collect()
    }

    /// What the overloaded keys do where the cursor is, for the bottom-right key popup: on a
    /// selection, a review thread, Claude's comment, the PR header or PR text. Nothing elsewhere,
    /// to keep it quiet.
    pub(super) fn context_keys(&self) -> Option<(&'static str, &'static [(&'static str, &'static str)])> {
        let row = &self.rows[self.head.row];
        // Over several of Claude's threads: one reply to them all.
        if self.selected_claude_threads().is_some() {
            return Some(("Claude's comments, selected", &[("r", "one reply to them all (Claude answers, or makes the changes)"), ("a", "the same"), ("y", "copy")]));
        }
        // (A draft: nothing on GitHub to comment on yet.)
        if (self.select || self.anchor.row != self.head.row) && self.unpublished() {
            return Some(("selection", &[("a", "a comment for Claude"), ("y", "copy"), ("Space c", "copy file:line ref")]));
        }
        if self.select || self.anchor.row != self.head.row {
            return Some(("selection", &[
                ("c", "comment (review)"),
                ("C", "comment now"),
                ("s", "suggest a change"),
                ("a", "a comment for Claude"),
                ("y", "copy"),
                ("Space c", "copy file:line ref"),
            ]));
        }
        // (Shown, its text has the usual hints.)
        if self.hidden_here().is_some_and(|n| !self.revealed.contains(&n)) {
            return Some(match row.thread {
                Some(_) => ("hidden on GitHub", &[("Tab", "show it"), ("r", "reply")]),
                None => ("hidden on GitHub", &[("Tab", "show it"), ("C", "comment on the PR")]),
            });
        }
        // On a link: what can be done with it (before what the row it's in offers).
        if self.link_at_cursor().is_some() {
            return Some(("link", &[("Enter", "open it (this repo's issue: in dv)"), ("Space c", "copy it"), ("Space k", "preview it (an issue or PR)")]));
        }
        if let Some(t) = row.thread {
            let t = &self.threads[t];
            return Some(match (t.claude.is_some(), t.by_claude(), t.outdated) {
                // Yours for Claude: private, so nothing here goes to GitHub.
                (true, false, _) => ("your comment for Claude", &[("r", "reply"), ("Space r", "resolve"), ("Space d", "remove"), ("Tab", "fold")]),
                (true, true, true) => ("Claude's comment (outdated)", &[("r", "reply to Claude"), ("Space d", "dismiss"), ("Tab", "fold")]),
                // No PR (a draft, a branch): nothing to copy to a review or post.
                (true, true, false) if self.pr.is_none() => ("Claude's comment", &[
                    ("r", "reply to Claude"),
                    ("Enter", "go to the file:line, name or link under the cursor"),
                    ("Space r", "resolve"),
                    ("Space d", "dismiss"),
                ]),
                (true, true, false) => ("Claude's comment", &[
                    ("r", "reply to Claude"),
                    ("Enter", "go to the file:line, name or link under the cursor"),
                    ("c", "copy to your review"),
                    ("C", "post it now"),
                    ("Space r", "resolve"),
                    ("Space d", "dismiss"),
                ]),
                (false, ..) => ("review thread", &[
                    ("r", "reply (review)"),
                    ("C", "reply now"),
                    ("Space r", "resolve"),
                    ("Space e", "edit your comment"),
                    ("Space d", "delete your comment"),
                    ("Space =", "react"),
                    ("Space c", "copy link"),
                ]),
            });
        }
        // (What it's of: an issue has no review, nor code; `c` queues into a review.)
        let what = if self.has_code() { "PR" } else { "issue" };
        if row.file == INFO && row.line.kind == Kind::File && !row.links.is_empty() {
            return Some((what, &[("Enter", "open on GitHub"), ("Tab", "fold")]));
        }
        if row.info == Some(MORE_CONVERSATION) {
            return Some(("activity", &[("Enter", "all of it, or only the latest"), ("Space o", "the same")]));
        }
        if row.info == Some(SUMMARY) {
            return Some(match (self.subject().is_some(), self.has_code()) {
                (false, _) => ("Claude's review", &[("r", "reply (Claude answers)")]),
                (true, false) => ("Claude's triage", &[("r", "reply (Claude answers)"), ("C", "post it as your comment")]),
                (true, true) => ("Claude's review", &[("r", "reply (Claude answers)"), ("c", "add to your review summary"), ("C", "post it as your PR comment")]),
            });
        }
        if self.replacement_draft(self.head.row).is_some() {
            return Some(("Claude's draft", &[("a", "ask Claude about it, or for a change (quoted)")]));
        }
        if row.info == Some(FACTS) {
            return Some((what, &[("Enter", "open the link under the cursor (a check: its run)")]));
        }
        if row.info == Some(DRAFT) {
            return Some(("your review", &[("S", "edit it and submit"), ("C", "new conversation comment"), ("Space c", "copy link")]));
        }
        if row.info == Some(CLAUDE_ACTIVITY) {
            return Some(("Claude's activity", &[("Enter", "go to it")]));
        }
        // A line of code: Enter opens it in your editor, there.
        let code = row.file != INFO && matches!(row.line.kind, Kind::Add | Kind::Del | Kind::Ctx);
        if code && self.unpublished() {
            return Some(("code", &[
                ("Enter", "open the file in your editor, at this line"),
                ("a", "a comment for Claude"),
                ("v x", "select (to comment on lines)"),
                ("Space c", "copy file:line ref"),
            ]));
        }
        if code {
            return Some(("code", &[
                ("Enter", "open the file in your editor, at this line"),
                ("c", "comment (review)"),
                ("s", "suggest a change"),
                ("a", "a comment for Claude"),
                ("v x", "select (to comment on lines)"),
                ("Space c", "copy file:line ref"),
            ]));
        }
        // (A line of the PR's (or issue's) own text: not one of dv's boxes.)
        let text = row.file == INFO && row.info.is_some_and(|i| i < self.info.len()) && self.subject().is_some();
        text.then(|| match self.has_code() {
            true => ("PR text", &[("c", "quote in your review summary"), ("C", "quote in a new conversation comment"), ("Space =", "react"), ("Space c", "copy link")][..]),
            false => ("issue text", &[("C", "quote in a new comment"), ("Space =", "react"), ("Space c", "copy link")][..]),
        })
    }

    /// The Claude or pending view with nothing in it: what the code side and the pane say.
    pub(super) fn empty_note(&self) -> Option<(&'static str, &'static str)> {
        let any = self.any_in_view();
        match self.view {
            View::Claude if !any && self.claude.summary.is_empty() && self.claude.broken.is_some() => {
                Some(("Claude's review file can't be read right now (Claude mid-edit, or a slip in it): the status line says why; it shows again once it's fixed.", "Claude's review can't be read."))
            }
            View::Claude if !any && self.claude.summary.is_empty() && !self.unpublished() => {
                Some(("Nothing here yet: Claude hasn't reviewed this (Space a asks it).", "No comments from Claude."))
            }
            View::Pending if !any && self.summary_draft.is_empty() => {
                Some(("Nothing here yet: your pending review is empty (c on a line queues a comment).", "No pending comments."))
            }
            _ => None,
        }
    }
}
