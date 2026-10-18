//! Selections as text: copying, patches, and file:line refs.
use super::*;

impl App {
    pub(super) fn selection_text(&self, patch: bool) -> String {
        let (s, e) = self.ordered();
        // Comment text is copied only when the selection starts in a comment.
        let in_comment = matches!(self.rows[s.row].line.kind, Kind::Thread | Kind::Comment);
        let mut out = vec![];
        for r in s.row..=e.row {
            let l = &self.rows[r].line;
            let marker = match l.kind {
                Kind::File => continue,
                Kind::Thread | Kind::Comment if patch || !in_comment => continue,
                Kind::Thread | Kind::Comment => ' ',
                Kind::Hunk if !patch => continue,
                Kind::Hunk => {
                    out.push(l.text.clone());
                    continue;
                }
                Kind::Add => '+',
                Kind::Del => '-',
                Kind::Ctx => ' ',
            };
            if patch {
                out.push(format!("{marker}{}", l.text));
                continue;
            }
            let chars = self.chars(r);
            let from = if r == s.row { s.col.min(chars.len()) } else { 0 };
            let to = if r == e.row { (e.col + 1).min(chars.len()) } else { chars.len() };
            out.push(chars[from..to.max(from)].iter().collect());
        }
        let mut t = out.join("\n");
        if !t.is_empty() && (patch || (s.col == 0 && e.col + 1 >= self.len(e.row))) {
            t.push('\n');
        }
        t
    }

    pub(super) fn yank(&mut self, patch: bool) {
        let t = self.selection_text(patch);
        if t.is_empty() {
            self.msg = "nothing to copy".into();
            return;
        }
        clipboard(&t);
        self.msg = format!("copied {} line(s)", t.lines().count().max(1));
    }

    /// `path:line` or `path:start-end` for the selection (new-side numbering), for pasting elsewhere.
    pub(super) fn file_ref(&self) -> Option<String> {
        let (s, e) = self.ordered();
        let fi = self.rows[self.head.row].file;
        let f = self.files.get(fi)?;
        let in_file = || self.rows[s.row..=e.row].iter().filter(|r| r.file == fi && r.thread.is_none());
        let span = |p: &str, a: u32, b: u32| if a == b { format!("{p}:{a}") } else { format!("{p}:{a}-{b}") };
        let news: Vec<u32> = in_file().filter_map(|r| r.line.new).collect();
        let olds: Vec<u32> = in_file().filter_map(|r| r.line.old).collect();
        Some(match (news.first(), news.last(), olds.first(), olds.last()) {
            (Some(&a), Some(&b), _, _) => span(&f.path, a, b),
            (_, _, Some(&a), Some(&b)) => {
                format!("{} (removed lines, at base {})", span(&f.old_path, a, b), &self.base[..self.base.len().min(10)])
            }
            _ => f.path.clone(),
        })
    }

    /// What to paste to Claude for the thing under the cursor: a GitHub comment's (or the PR's)
    /// link, a Claude comment's lines, else the selected lines' `path:line`.
    pub(super) fn cursor_ref(&self) -> Option<String> {
        let row = &self.rows[self.head.row];
        // A commit's message (its section): the commit's hash.
        if let Some(c) = row.main.and_then(|m| m.checked_sub(1)).filter(|_| row.main != Some(CLAUDE_REVIEW)).and_then(|i| self.commits.get(i)) {
            return Some(c.sha.clone());
        }
        if let Some(t) = row.thread {
            let th = &self.threads[t];
            if th.claude.is_some() {
                let line = th.line.or(th.original_line)?;
                return Some(match th.start_line {
                    Some(s) => format!("{}:{s}-{line}", th.path),
                    None => format!("{}:{line}", th.path),
                });
            }
            let pr = self.pr.as_ref()?;
            return Some(format!("{}#discussion_r{}", pr.url, self.comment_under_cursor(t)?.id));
        }
        if row.file == INFO {
            // A comment's rows carry its body's index; the link is on the heading just before.
            let url = row.info.and_then(|i| self.info.get(i.checked_sub(1)?)).map(|h| h.url.clone()).filter(|u| !u.is_empty());
            return url.or_else(|| self.pr.as_ref().map(|p| p.url.clone()).filter(|u| !u.is_empty()));
        }
        self.file_ref()
    }

    pub(super) fn copy_ref(&mut self) {
        // On a link: its URL (what Enter opens in the browser).
        if let Some(u) = self.link_at_cursor().map(str::to_owned) {
            clipboard(&u);
            return self.msg = format!("copied {u}");
        }
        match self.cursor_ref() {
            Some(r) => {
                clipboard(&r);
                self.msg = format!("copied {r}");
            }
            None => self.msg = "nothing to copy a reference to here".into(),
        }
    }
}
