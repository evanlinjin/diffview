//! Following what's under the cursor: a link (in the browser, or a GitHub link in dv), a
//! `file:line` reference, or a name to its definition (rust-analyzer).
use super::*;

/// The `path:line` reference in `text` around char `col`, if there is one: `src/a.rs:12`,
/// `a.rs:12-15`, `a.rs:12:5`, `a.rs#L12` (a trailing `.`/`,`/`:` is punctuation).
pub(super) fn file_ref(text: &str, col: usize) -> Option<(String, u32)> {
    let token = token_at(text, col, |c| c.is_alphanumeric() || "_./-:#".contains(c))?;
    let token = token.trim_end_matches(['.', ',', ':', '-']);
    let (path, rest) = token.split_once("#L").or_else(|| token.split_once(':'))?;
    let line: u32 = rest.split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()?;
    // A file: it has an extension or a directory (not "note:3" or a time "12:30").
    let file = path.rsplit('/').next().is_some_and(|f| f.contains('.') && !f.starts_with('.') || path.contains('/'));
    (file && line > 0 && !path.chars().all(|c| c.is_ascii_digit())).then(|| (path.to_owned(), line))
}

/// The run of `ok` chars in `text` around char `col` (on one).
fn token_at(text: &str, col: usize, ok: impl Fn(char) -> bool) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    if !chars.get(col).is_some_and(|&c| ok(c)) {
        return None;
    }
    let a = (0..col).rev().take_while(|&i| ok(chars[i])).last().unwrap_or(col);
    let b = (col..chars.len()).take_while(|&i| ok(chars[i])).last()? + 1;
    Some(chars[a..b].iter().collect())
}

/// The words a definition starts with, across the usual languages.
pub(super) const DEF_WORDS: &str = "fn|struct|enum|trait|type|union|mod|macro_rules!|const|static|def|class|function|func|interface";

/// The name under char `col` of `text`, if it's code: `in_code` (a `code` span), or a word
/// with `_`, a capital after its start, or a digit (`next_returns_none`, `parseArgs`), not a
/// plain English word.
pub(super) fn ident_at(text: &str, col: usize, in_code: bool) -> Option<String> {
    let w = token_at(text, col, |c| c.is_alphanumeric() || c == '_')?;
    let code = in_code || w.contains('_') || w.chars().skip(1).any(char::is_uppercase) || w.chars().any(|c| c.is_ascii_digit());
    (code && !w.starts_with(|c: char| c.is_ascii_digit())).then_some(w)
}

impl App {
    /// The link under the cursor, if any.
    pub(super) fn link_at_cursor(&self) -> Option<&str> {
        let col = self.head.col;
        self.rows[self.head.row].links.iter().find(|(a, b, _)| *a <= col && col < *b).map(|l| l.2.as_str())
    }

    /// Enter off code: the link under the cursor in the browser, or the `file:line` under it (as comments
    /// cite code: `src/a.rs:12`, `a.rs:12-15`, `a.rs#L12`) in dv, or in hx if it isn't shown;
    /// or, on a name (`next_returns_none`), where it's defined.
    pub(super) fn open_link(&mut self) {
        // On a GitHub comment's author line (a review thread's, or the conversation's): that
        // comment on GitHub. In its text: what's under the cursor, else the comment too.
        let row = &self.rows[self.head.row];
        // A line standing for a comment (Claude's activity): to that comment.
        if let Some((t, c)) = row.goes {
            return self.goto_comment(t, c);
        }
        // (Its review, no comment of it left: its summary.)
        if row.info == Some(CLAUDE_ACTIVITY) && row.line.kind == Kind::Comment && !row.line.text.is_empty()
            && let Some(r) = self.rows.iter().position(|r| r.info == Some(SUMMARY))
        {
            return self.goto(r, 0);
        }
        // (Not other headings there: Claude's review, your pending one: nothing on GitHub.)
        let has_link = row.thread.is_some_and(|t| self.threads[t].claude.is_none())
            || row.file == INFO && row.info.and_then(|i| self.info.get(i.checked_sub(1)?)).is_some_and(|h| h.heading && !h.url.is_empty());
        let comment = self.cursor_ref().filter(|u| has_link && u.starts_with("https://"));
        let on_author = row.line.kind == Kind::Thread;
        let url = self.link_at_cursor().map(str::to_owned).or_else(|| comment.clone().filter(|_| on_author));
        let Some(url) = url else {
            if let Some((path, line)) = file_ref(&self.rows[self.head.row].line.text, self.word_col()) {
                return self.goto_file_line(&path, line);
            }
            return match (self.name_at_cursor(), comment) {
                (Some(name), _) => self.goto_definition(&name),
                (None, Some(u)) => self.open_url(&u),
                // (A comment on code, nothing under the cursor: that code, in your editor.)
                (None, None) if self.rows[self.head.row].file < self.files.len() => self.action = self.open_target().map(Action::Open),
                (None, None) => self.msg = "no link, file:line or name under the cursor".into(),
            };
        };
        // This repo's issue: in dv's issue view (and back here after).
        if let Some(n) = self.issue_here(&url) {
            return self.start(Job::OpenIssue(n, url));
        }
        self.open_url(&url);
    }

    /// The number in `url` when it's an issue or a PR (/issues/ or /pull/: which is asked, as
    /// GitHub links PRs to /issues/ too) of this repo, not this PR.
    pub(super) fn issue_here(&self, url: &str) -> Option<u64> {
        let tail = url.strip_prefix(self.repo_url()?)?;
        let rest = tail.strip_prefix("/issues/").or_else(|| tail.strip_prefix("/pull/"))?;
        let n = rest.split(['#', '/', '?']).next()?.parse::<u64>().ok()?;
        (self.subject().map(|s| s.n()) != Some(n)).then_some(n)
    }

    pub(super) fn open_url(&mut self, url: &str) {
        self.msg = open_url(url);
    }

    /// Go to `line` of `path` (as a comment wrote it: from the repo root, or just its end, like
    /// `view.rs`): in the diff when it's shown there, else in hx.
    pub(super) fn goto_file_line(&mut self, path: &str, line: u32) {
        let path = path.trim_start_matches("./");
        let ends = |p: &str| p == path || p.ends_with(&format!("/{path}"));
        let found = match self.root.join(path).is_file() {
            true => Some(path.to_owned()),
            false => self.files.iter().map(|f| f.path.clone()).find(|p| ends(p)).or_else(|| {
                let all = diff::git(&["ls-files"]).unwrap_or_default();
                let mut hits = all.lines().filter(|p| ends(p));
                // Only if it's unambiguous.
                match (hits.next(), hits.next()) {
                    (Some(p), None) => Some(p.to_owned()),
                    _ => None,
                }
            }),
        };
        let Some(rel) = found else { return self.msg = format!("no file {path} here") };
        self.jump_loc(&crate::lsp::Loc { path: self.root.join(&rel), line: line.saturating_sub(1), character: 0 });
    }

    /// The column of the word the cursor's on: after `w` the cursor is on the space after the
    /// word, and the selection's start is on it.
    fn word_col(&self) -> usize {
        let on_space = self.rows[self.head.row].line.text.chars().nth(self.head.col).is_none_or(char::is_whitespace);
        if on_space && self.anchor.row == self.head.row { self.anchor.col } else { self.head.col }
    }

    /// The name under the cursor in text (a comment, the PR's): in `code`, or one that looks
    /// like code (`snake_case`, `CamelCase`, `a::b`).
    pub(super) fn name_at_cursor(&self) -> Option<String> {
        let (text, col) = (&self.rows[self.head.row].line.text, self.word_col());
        // Code's color (it may be bold too, in bold text or a heading).
        let raw = self.ui.md.raw.fg;
        let in_code = raw.is_some() && self.rows[self.head.row].syn.get(col).is_some_and(|s| s.and_then(|s| s.fg) == raw);
        ident_at(text, col, in_code)
    }

    /// Where `name` is defined (by a grep for `fn name`, `class name` and the like, in the files
    /// on disk), the diff's files first: there, or a picker of them.
    // ponytail: a grep, not the language server; good enough for names in comments.
    pub(super) fn goto_definition(&mut self, name: &str) {
        // POSIX classes, not \b / \s: those are GNU-only.
        let pat = format!("(^|[^[:alnum:]_])({DEF_WORDS})[[:space:]]+{name}([^[:alnum:]_]|$)");
        let root = self.root.display().to_string();
        // -z: path NUL line NUL text, the path unquoted (it may hold `:` or non-ASCII).
        let out = diff::git(&["-C", &root, "grep", "-n", "-z", "-I", "--untracked", "-E", &pat]).unwrap_or_default();
        let mut locs: Vec<crate::lsp::Loc> = out
            .lines()
            .filter_map(|l| {
                let mut parts = l.splitn(3, '\0');
                let (path, line, text) = (parts.next()?, parts.next()?.parse::<u32>().ok()?, parts.next()?);
                // A UTF-16 column, as LSP's.
                let character = text.find(name).map_or(0, |i| text[..i].encode_utf16().count()) as u32;
                Some(crate::lsp::Loc { path: self.root.join(path), line: line - 1, character })
            })
            .collect();
        let in_diff: HashSet<PathBuf> = self.files.iter().map(|f| self.root.join(&f.path)).collect();
        locs.sort_by_key(|l| !in_diff.contains(&l.path));
        locs.truncate(50);
        if locs.is_empty() {
            return self.msg = format!("no definition of {name} found");
        }
        self.show_locations(locs);
    }
}

#[test]
fn idents() {
    let at = |text: &str, needle: &str| ident_at(text, text.find(needle).unwrap(), false);
    let t = "half of next_returns_none_after_error, so inline parseArgs or Foo::bar.";
    assert_eq!(at(t, "none").as_deref(), Some("next_returns_none_after_error")); // anywhere on it
    assert_eq!(at(t, "parse").as_deref(), Some("parseArgs"));
    assert_eq!(at(t, "half"), None); // a word
    assert_eq!(at(t, "Foo"), None); // could be anything: ask by its method
    assert_eq!(at(t, " so"), None);
    assert_eq!(ident_at(t, t.find("bar").unwrap(), true).as_deref(), Some("bar")); // in `code`: any name
}

#[test]
fn file_refs() {
    let at = |text: &str, needle: &str| file_ref(text, text.find(needle).unwrap());
    let t = "see src/view/mod.rs:1213 and a.rs:12-15, also b.rs#L7.";
    assert_eq!(at(t, "mod.rs"), Some(("src/view/mod.rs".into(), 1213)));
    assert_eq!(at(t, "1213"), Some(("src/view/mod.rs".into(), 1213))); // anywhere on it
    assert_eq!(at(t, "a.rs"), Some(("a.rs".into(), 12)));
    assert_eq!(at(t, "b.rs"), Some(("b.rs".into(), 7)));
    assert_eq!(at(t, "see"), None);
    assert_eq!(at("at 12:30 today", "12"), None); // a time
    assert_eq!(at("note: this", "note"), None);
    assert_eq!(at("x.rs:12:5,", "x.rs"), Some(("x.rs".into(), 12)));
}

/// `url` in the browser (`xdg-open` / `open`): what happened, for the status bar.
pub(crate) fn open_url(url: &str) -> String {
    if cfg!(test) {
        return format!("opened {url}"); // (no browser from tests)
    }
    // (Only the web's: a link's text, from anyone's markdown, may hide another scheme.)
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return format!("not opened: only http(s) links are ({url})");
    }
    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    let spawned = std::process::Command::new(opener)
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    match spawned {
        Ok(_) => format!("opened {url}"),
        Err(e) => format!("{opener}: {e}"),
    }
}
