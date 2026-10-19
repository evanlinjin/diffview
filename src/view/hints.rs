//! What the keys do, where you are: the help popup, the context hints on the status bar, hover text.
use super::*;

impl App {
    /// The help text, styled: section headings, keys (separators dimmed), descriptions wrapped
    /// with a hanging indent under the description column, `(notes)` dimmed. With a `query`
    /// (smart case), only matching entries and their headings are kept, matches highlighted.
    pub(super) fn help_lines(&self, width: usize, query: &str) -> Vec<Line<'static>> {
        const KEY_COL: usize = 23; // entries are "  <key, padded to col 23><description>"
        let ui = &self.ui;
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
        for l in HELP.trim_matches('\n').lines() {
            match l {
                "" => {}
                h if !h.starts_with(' ') => sections.push((h, vec![])),
                e if q.is_empty() || fold(e).contains(&q) => {
                    if let Some(s) = sections.last_mut() {
                        s.1.push(e);
                    }
                }
                _ => {}
            }
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

    /// Hover markdown, highlighted (code fences in their language), with the fence lines dropped.
    pub(super) fn hover_lines(&self, md: &str) -> Vec<Line<'static>> {
        let cw = (self.width.saturating_sub(4).min(100) as usize).saturating_sub(3);
        let code_bg = self.ui.md.code_bg;
        crate::md::render(md, cw, &self.ui.md, &self.hl, None)
            .into_iter()
            .map(|l| {
                // A code block's line: its background to the popup's edge, as in a comment's box.
                let code = l.nowrap && code_bg.is_some() && l.styles.first().copied().flatten().and_then(|s| s.bg) == code_bg;
                let pad = cw.saturating_sub(Line::from(l.text.as_str()).width());
                let mut spans: Vec<Span> = l
                    .text
                    .chars()
                    .zip(l.styles.into_iter().chain(std::iter::repeat(None)))
                    .map(|(ch, st)| Span::styled(ch.to_string(), st.unwrap_or_default()))
                    .collect();
                if let Some(bg) = code_bg.filter(|_| code) {
                    spans.push(Span::styled(" ".repeat(pad), Style::new().bg(bg)));
                }
                Line::from(spans)
            })
            .collect()
    }

    /// What the overloaded keys do where the cursor is, for the bottom-right key popup: on a
    /// selection, a review thread, Claude's comment, the PR header or PR text. Nothing elsewhere,
    /// to keep it quiet.
    pub(super) fn context_keys(&self) -> Option<(&'static str, &'static [(&'static str, &'static str)])> {
        let row = &self.rows[self.head.row];
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
            return Some(("link", &[("Enter", "open it in the browser"), ("Space c", "copy it"), ("Space k", "preview it (an issue or PR)")]));
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
                    ("gf", "go to a file:line or name in it"),
                    ("Space r", "resolve"),
                    ("Space d", "dismiss"),
                ]),
                (true, true, false) => ("Claude's comment", &[
                    ("r", "reply to Claude"),
                    ("gf", "go to a file:line or name in it"),
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
        if row.file == INFO && row.line.kind == Kind::File && !row.links.is_empty() {
            return Some(("PR", &[("gf", "open on GitHub"), ("Tab", "fold")]));
        }
        if row.info == Some(MORE_CONVERSATION) {
            return Some(("activity", &[("Enter", "all of it, or only the latest"), ("Space o", "the same")]));
        }
        if row.info == Some(SUMMARY) && self.pr.is_none() {
            return Some(("Claude's review", &[("r", "reply (Claude answers)")]));
        }
        if row.info == Some(SUMMARY) {
            return Some(("Claude's review", &[("r", "reply (Claude answers)"), ("c", "add to your review summary"), ("C", "post it as your PR comment")]));
        }
        if self.replacement_draft(self.head.row).is_some() {
            return Some(("Claude's draft", &[("a", "ask Claude about it, or for a change (quoted)")]));
        }
        if row.info == Some(FACTS) {
            return Some(("PR", &[("Enter", "open the link under the cursor (a check: its run)"), ("gf", "the same")]));
        }
        if row.info == Some(DRAFT) {
            return Some(("your review", &[("S", "edit it and submit"), ("C", "new conversation comment"), ("Space c", "copy link")]));
        }
        (row.file == INFO && row.info.is_some() && self.pr.is_some())
            .then_some(("PR text", &[("c", "quote in your review summary"), ("C", "quote in a new conversation comment"), ("Space =", "react"), ("Space c", "copy link")]))
    }

    /// The Claude or pending view with nothing in it: what the code side and the pane say.
    pub(super) fn empty_note(&self) -> Option<(&'static str, &'static str)> {
        let any = self.any_in_view();
        match self.view {
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
