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

    pub(super) fn copy_ref(&mut self) {
        match self.file_ref() {
            Some(r) => {
                clipboard(&r);
                self.msg = format!("copied {r}");
            }
            None => self.msg = "not in a file".into(),
        }
    }
}
