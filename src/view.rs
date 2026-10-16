//! Diff view state, Helix-style keys, and rendering.
use crate::diff::{self, FileDiff, Kind, Line as DLine};
use crate::gh::{self, Anchor, Pr, Thread};
use crate::lsp::{self, Loc, Lsp};
use anyhow::{bail, Context, Result};
use ratatui::{
    crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Clear, Paragraph, Wrap},
    DefaultTerminal, Frame,
};
use std::collections::{HashMap, HashSet};
use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

const DEFAULT_CONTEXT: u32 = 3;
const FULL: u32 = 1_000_000;
/// `Row::file` for the PR info section at the top.
const INFO: usize = usize::MAX - 1;

/// Position in the view: row index into `rows`, char index into the row's text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Pos {
    row: usize,
    col: usize,
}

struct Row {
    file: usize,
    line: DLine,
    /// Index into `threads` for comment rows.
    thread: Option<usize>,
}

type CursorKey = (Option<(String, Option<u32>, Option<u32>)>, Option<u64>);

/// Work that needs the terminal back (an editor), run between frames.
enum Action {
    Open(String),
    Comment(Anchor),
    Reply(u64),
}

enum Target {
    Row(usize),
    Loc(Loc),
}

struct Item {
    label: String,
    target: Target,
}

struct Picker {
    title: &'static str,
    items: Vec<Item>,
    query: String,
    sel: usize,
}

/// What an in-flight LSP request was for.
enum Req {
    Init,
    Goto,
    Hover,
}

pub struct App {
    base: String,
    files: Vec<FileDiff>,
    context: HashMap<String, u32>,
    collapsed: HashSet<String>,
    rows: Vec<Row>,
    anchor: Pos,
    head: Pos,
    want_col: usize,
    select: bool,
    count: usize,
    pending: Option<char>,
    prompt: Option<String>,
    picker: Option<Picker>,
    search: String,
    msg: String,
    top: usize,
    left: usize,
    height: usize,
    action: Option<Action>,
    quit: bool,
    pr: Option<Pr>,
    threads: Vec<Thread>,
    info: Vec<gh::InfoLine>,
    info_collapsed: bool,
    /// Width comment/info text is wrapped to; follows the terminal.
    wrap_w: usize,
    /// Root comment ids of threads folded/unfolded against the default (resolved = folded).
    toggled: HashSet<u64>,
    root: PathBuf,
    lsp: Option<Lsp>,
    lsp_ready: bool,
    inflight: HashMap<u64, Req>,
    progress: HashMap<String, (String, String)>,
    popup: Option<(String, u16)>,
    jumps: Vec<Pos>,
    alphabet: Vec<char>,
    /// `gw` jump labels: (word start, word end, label), plus the first label char typed.
    labels: Vec<(Pos, Pos, [char; 2])>,
    label_first: Option<char>,
}

impl App {
    pub fn new(base: String, pr: Option<Pr>) -> Result<Self> {
        let files = diff::diff(&base, DEFAULT_CONTEXT, &[])?;
        let mut app = Self::with(base, files);
        app.pr = pr;
        app.load_threads();
        app.build();
        match Lsp::start(&app.root) {
            Ok((l, id)) => {
                app.lsp = Some(l);
                app.inflight.insert(id, Req::Init);
            }
            Err(e) => app.msg = format!("{e:#}"),
        }
        Ok(app)
    }

    fn with(base: String, files: Vec<FileDiff>) -> Self {
        let mut app = App {
            base,
            files,
            context: HashMap::new(),
            collapsed: HashSet::new(),
            rows: vec![],
            anchor: Pos::default(),
            head: Pos::default(),
            want_col: 0,
            select: false,
            count: 0,
            pending: None,
            prompt: None,
            picker: None,
            search: String::new(),
            msg: String::new(),
            top: 0,
            left: 0,
            height: 1,
            action: None,
            quit: false,
            pr: None,
            threads: vec![],
            info: vec![],
            info_collapsed: false,
            wrap_w: 100,
            toggled: HashSet::new(),
            root: std::env::current_dir().unwrap_or_default(),
            lsp: None,
            lsp_ready: false,
            inflight: HashMap::new(),
            progress: HashMap::new(),
            popup: None,
            jumps: vec![],
            alphabet: jump_alphabet(),
            labels: vec![],
            label_first: None,
        };
        app.build();
        app
    }

    pub fn run(&mut self, term: &mut DefaultTerminal) -> Result<()> {
        while !self.quit {
            term.draw(|f| self.draw(f))?;
            // ponytail: 100ms poll so LSP replies show up without a keypress; a waker channel if it matters.
            if event::poll(Duration::from_millis(100))? {
                if let Event::Key(k) = event::read()? {
                    if k.kind == KeyEventKind::Press {
                        self.key(k);
                    }
                }
            }
            self.lsp_poll();
            if let Some(a) = self.action.take() {
                ratatui::restore();
                let res = self.perform(a);
                *term = ratatui::init();
                if let Err(e) = self.reload(true) {
                    self.msg = e.to_string();
                }
                match res {
                    Ok(m) => self.msg = m,
                    Err(e) => self.msg = format!("{e:#}"),
                }
            }
        }
        Ok(())
    }

    // ---- data ----

    fn perform(&mut self, a: Action) -> Result<String> {
        match a {
            Action::Open(target) => {
                std::process::Command::new("hx").arg(&target).status().context("running hx")?;
                Ok(String::new())
            }
            Action::Comment(anchor) => {
                let range = match anchor.start {
                    Some((s, _)) => format!("{s}-{}", anchor.line),
                    None => anchor.line.to_string(),
                };
                let Some(body) = compose(&format!("comment on {}:{range}", anchor.path))? else {
                    return Ok("comment cancelled".into());
                };
                let pr = self.pr.as_ref().context("no PR")?;
                let res = gh::comment(pr, &anchor, &body);
                _ = std::fs::remove_file(&body);
                res.map(|_| "comment posted".into())
            }
            Action::Reply(id) => {
                let Some(body) = compose("reply")? else { return Ok("reply cancelled".into()) };
                let pr = self.pr.as_ref().context("no PR")?;
                let res = gh::reply(pr, id, &body);
                _ = std::fs::remove_file(&body);
                res.map(|_| "reply posted".into())
            }
        }
    }

    fn load_threads(&mut self) {
        let Some(pr) = &self.pr else { return };
        match gh::threads(pr.number) {
            Ok(t) => self.threads = t,
            Err(e) => self.msg = format!("loading comments: {e:#}"),
        }
        match gh::info(pr.number) {
            Ok(i) => self.info = i,
            Err(e) => self.msg = format!("loading PR info: {e:#}"),
        }
    }

    fn reload(&mut self, threads: bool) -> Result<()> {
        let keep = self.cursor_key();
        if threads {
            self.load_threads();
        }
        let mut files = diff::diff(&self.base, DEFAULT_CONTEXT, &[])?;
        for f in &mut files {
            if let Some(&n) = self.context.get(&f.path) {
                if let Some(g) = diff::diff(&self.base, n, &[&f.old_path, &f.path])?.pop() {
                    *f = g;
                }
            }
        }
        self.files = files;
        self.rebuild(keep);
        Ok(())
    }

    /// What the cursor is on, independent of row indices: (file, old, new) and thread root id.
    fn cursor_key(&self) -> CursorKey {
        let r = &self.rows[self.head.row];
        let line = self.files.get(r.file).map(|f| (f.path.clone(), r.line.old, r.line.new));
        (line, r.thread.and_then(|t| self.root_id(t)))
    }

    /// Rebuild rows, keeping the cursor on the same code line or thread.
    fn rebuild(&mut self, (keep, keep_thread): CursorKey) {
        self.build();
        let row = keep_thread
            .and_then(|id| self.rows.iter().position(|r| r.thread.and_then(|t| self.root_id(t)) == Some(id)))
            .or_else(|| keep.map(|(p, old, new)| self.locate(&p, old, new)))
            .unwrap_or(0);
        self.select = false;
        self.goto(row, self.head.col);
    }

    fn build(&mut self) {
        self.jumps.clear();
        let mut rows = vec![];
        if let Some(pr) = &self.pr {
            let mk = |kind, text| Row { file: INFO, line: DLine { kind, old: None, new: None, text }, thread: None };
            rows.push(mk(Kind::File, format!("PR #{} · {}", pr.number, pr.title)));
            if !self.info_collapsed {
                for l in &self.info {
                    match l.heading {
                        true => rows.push(mk(Kind::Thread, l.text.clone())),
                        false => rows.extend(wrap(&l.text, self.wrap_w).into_iter().map(|t| mk(Kind::Comment, t))),
                    }
                }
            }
        }
        for (i, f) in self.files.iter().enumerate() {
            let text = if f.meta.is_empty() { f.path.clone() } else { format!("{} ({})", f.path, f.meta) };
            rows.push(Row { file: i, line: DLine { kind: Kind::File, old: None, new: None, text }, thread: None });
            if self.collapsed.contains(&f.path) {
                continue;
            }
            let mine: Vec<usize> = (0..self.threads.len()).filter(|&t| self.threads[t].path == f.path).collect();
            let at = |t: usize, l: &DLine| {
                let t = &self.threads[t];
                !t.outdated && t.line.is_some_and(|n| match t.left {
                    true => l.kind != Kind::Add && l.old == Some(n),
                    false => l.kind != Kind::Del && l.new == Some(n),
                })
            };
            // Outdated threads, or ones whose line isn't in this diff, go under the file header.
            for &t in &mine {
                if !f.lines.iter().any(|l| at(t, l)) {
                    rows.extend(self.thread_rows(t, i));
                }
            }
            for l in &f.lines {
                rows.push(Row { file: i, line: l.clone(), thread: None });
                for &t in mine.iter().filter(|&&t| at(t, l)) {
                    rows.extend(self.thread_rows(t, i));
                }
            }
        }
        if rows.is_empty() {
            let text = "(no changes)".into();
            rows.push(Row { file: usize::MAX, line: DLine { kind: Kind::Hunk, old: None, new: None, text }, thread: None });
        }
        self.rows = rows;
    }

    fn root_id(&self, t: usize) -> Option<u64> {
        Some(self.threads.get(t)?.comments.first()?.id)
    }

    fn thread_rows(&self, ti: usize, file: usize) -> Vec<Row> {
        let t = &self.threads[ti];
        let open = t.resolved == self.root_id(ti).is_some_and(|id| self.toggled.contains(&id));
        let range = match (t.start_line, t.line.or(t.original_line)) {
            (Some(s), Some(e)) if s != e => format!(" · L{s}-{e}"),
            (_, Some(e)) => format!(" · L{e}"),
            _ => String::new(),
        };
        let flags = [(t.resolved, " · resolved"), (t.outdated, " · outdated")]
            .iter()
            .filter_map(|&(on, s)| on.then_some(s))
            .collect::<String>();
        let mk = |kind, text| Row { file, line: DLine { kind, old: None, new: None, text }, thread: Some(ti) };
        let mut out = vec![];
        for (k, c) in t.comments.iter().enumerate() {
            if k == 0 {
                let mut head = format!("{} · {}{range}{flags}", c.author, c.created);
                if !open {
                    head += &format!(" · {} comment(s), Tab to expand", t.comments.len());
                    out.push(mk(Kind::Thread, head));
                    break;
                }
                out.push(mk(Kind::Thread, head));
            } else {
                out.push(mk(Kind::Thread, format!("{} · {}", c.author, c.created)));
            }
            out.extend(wrap(&c.body, self.wrap_w).into_iter().map(|l| mk(Kind::Comment, l)));
        }
        out
    }

    fn toggle_thread(&mut self) {
        let Some(ti) = self.rows[self.head.row].thread else { return };
        let Some(id) = self.root_id(ti) else { return };
        if !self.toggled.remove(&id) {
            self.toggled.insert(id);
        }
        self.build();
        self.select = false;
        let r = self.rows.iter().position(|r| r.thread == Some(ti)).unwrap_or(0);
        self.goto(r, 0);
    }

    /// GitHub anchor for the selected code lines (comment rows inside the selection are ignored).
    fn selection_anchor(&self) -> Result<Anchor> {
        let (s, e) = self.ordered();
        let code: Vec<usize> = (s.row..=e.row)
            .filter(|&r| matches!(self.rows[r].line.kind, Kind::Ctx | Kind::Add | Kind::Del))
            .collect();
        let (Some(&a), Some(&b)) = (code.first(), code.last()) else { bail!("select code lines to comment on") };
        if self.rows[a].file != self.rows[b].file {
            bail!("a comment can't span files");
        }
        let side = |r: usize| {
            let l = &self.rows[r].line;
            if l.kind == Kind::Del { (l.old, true) } else { (l.new, false) }
        };
        let ((Some(line), left), (Some(sl), sleft)) = (side(b), side(a)) else { bail!("no line number") };
        Ok(Anchor {
            path: self.files[self.rows[b].file].path.clone(),
            line,
            left,
            start: (a != b).then_some((sl, sleft)),
        })
    }

    fn start_comment(&mut self) {
        let res = (|| {
            let pr = self.pr.as_ref().context("no PR for this branch")?;
            // Lines are posted against the PR head; local edits would shift them.
            if diff::git(&["rev-parse", "HEAD"])?.trim() != pr.head_oid {
                bail!("HEAD isn't the PR head (pull, or `gh pr checkout {}`)", pr.number);
            }
            if !diff::git(&["status", "--porcelain", "--untracked-files=no"])?.trim().is_empty() {
                bail!("working tree has local changes; stash them to comment");
            }
            self.selection_anchor()
        })();
        match res {
            Ok(a) => self.action = Some(Action::Comment(a)),
            Err(e) => self.msg = format!("{e:#}"),
        }
    }

    fn start_reply(&mut self) {
        match self.rows[self.head.row].thread.and_then(|t| self.root_id(t)) {
            Some(id) if self.pr.is_some() => self.action = Some(Action::Reply(id)),
            _ => self.msg = "move onto a comment thread to reply".into(),
        }
    }

    /// Row in file `path` at (or just after) the given line, else the file header.
    fn locate(&self, path: &str, old: Option<u32>, new: Option<u32>) -> usize {
        let Some(fi) = self.files.iter().position(|f| f.path == path) else { return 0 };
        let in_file = |r: &&Row| r.file == fi;
        let hit = match (new, old) {
            (Some(n), _) => self.rows.iter().position(|r| in_file(&r) && r.line.new.is_some_and(|x| x >= n)),
            (None, Some(o)) => self.rows.iter().position(|r| in_file(&r) && r.line.old.is_some_and(|x| x >= o)),
            _ => None,
        };
        hit.or_else(|| self.rows.iter().position(|r| r.file == fi)).unwrap_or(0)
    }

    fn file_path(&self) -> Option<&str> {
        Some(&self.files.get(self.rows[self.head.row].file)?.path)
    }

    // ---- cursor ----

    fn len(&self, r: usize) -> usize {
        self.rows[r].line.text.chars().count()
    }

    fn chars(&self, r: usize) -> Vec<char> {
        self.rows[r].line.text.chars().collect()
    }

    /// Move the head; outside select mode the anchor follows.
    fn goto(&mut self, row: usize, col: usize) {
        let row = row.min(self.rows.len() - 1);
        self.head = Pos { row, col: col.min(self.len(row).saturating_sub(1)) };
        if !self.select {
            self.anchor = self.head;
        }
        self.want_col = self.head.col;
    }

    /// Set a selection (motions like `w` select); in select mode only the head moves.
    fn sel(&mut self, anchor: Pos, head: Pos) {
        self.head = head;
        if !self.select {
            self.anchor = anchor;
        }
        self.want_col = head.col;
    }

    fn vmove(&mut self, d: isize) {
        let last = self.rows.len() as isize - 1;
        let w = self.want_col;
        self.goto((self.head.row as isize + d).clamp(0, last) as usize, w);
        self.want_col = w;
    }

    fn ordered(&self) -> (Pos, Pos) {
        (self.anchor.min(self.head), self.anchor.max(self.head))
    }

    fn word(&mut self, m: char) {
        let big = m.is_ascii_uppercase();
        let class = |c: char| if c.is_whitespace() { 0 } else if big || c.is_alphanumeric() || c == '_' { 1 } else { 2 };
        let Pos { row, col: i } = self.head;
        let chars = self.chars(row);
        let n = chars.len();
        let cl = |j: usize| class(chars[j]);
        if m.eq_ignore_ascii_case(&'b') {
            if i == 0 || n == 0 {
                if row > 0 {
                    let p = Pos { row: row - 1, col: self.len(row - 1).saturating_sub(1) };
                    self.sel(p, p);
                }
                return;
            }
            let start = if cl(i) != cl(i - 1) { i - 1 } else { i };
            let mut j = start;
            while j > 0 && cl(j) == 0 {
                j -= 1;
            }
            let c = cl(j);
            while j > 0 && cl(j - 1) == c {
                j -= 1;
            }
            self.sel(Pos { row, col: start }, Pos { row, col: j });
            return;
        }
        if i + 1 >= n {
            if row + 1 < self.rows.len() {
                let p = Pos { row: row + 1, col: 0 };
                self.sel(p, p);
            }
            return;
        }
        let start = if cl(i) != cl(i + 1) { i + 1 } else { i };
        let mut j = start;
        if m.eq_ignore_ascii_case(&'w') {
            let c = cl(j);
            while j < n && cl(j) == c {
                j += 1;
            }
            while j < n && cl(j) == 0 {
                j += 1;
            }
            j -= 1;
        } else {
            while j + 1 < n && cl(j) == 0 {
                j += 1;
            }
            let c = cl(j);
            while j + 1 < n && cl(j + 1) == c {
                j += 1;
            }
        }
        self.sel(Pos { row, col: start }, Pos { row, col: j });
    }

    fn find_char(&mut self, p: char, ch: char) {
        let chars = self.chars(self.head.row);
        let (i, n) = (self.head.col, chars.len());
        let hit = match p {
            'f' => (i + 1..n).find(|&j| chars[j] == ch),
            't' => (i + 2..n).find(|&j| chars[j] == ch).map(|j| j - 1),
            'F' => (0..i).rev().find(|&j| chars[j] == ch),
            _ => (0..i.saturating_sub(1)).rev().find(|&j| chars[j] == ch).map(|j| j + 1),
        };
        if let Some(col) = hit {
            self.sel(self.head, Pos { row: self.head.row, col });
        }
    }

    fn line_select(&mut self) {
        let (s, e) = self.ordered();
        let end = self.len(e.row).saturating_sub(1);
        if s.col == 0 && e.col >= end && self.head == e && s != e {
            let r = (e.row + 1).min(self.rows.len() - 1);
            self.anchor = s;
            self.head = Pos { row: r, col: self.len(r).saturating_sub(1) };
        } else {
            self.anchor = Pos { row: s.row, col: 0 };
            self.head = Pos { row: e.row, col: end };
        }
        self.want_col = self.head.col;
    }

    fn next_kind(&mut self, fwd: bool, is_start: impl Fn(&[Row], usize) -> bool) {
        let rows = &self.rows;
        let hit = if fwd {
            (self.head.row + 1..rows.len()).find(|&r| is_start(rows, r))
        } else {
            (0..self.head.row).rev().find(|&r| is_start(rows, r))
        };
        if let Some(r) = hit {
            self.goto(r, 0);
        }
    }

    // ---- actions ----

    fn selection_text(&self, patch: bool) -> String {
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

    fn yank(&mut self, patch: bool) {
        let t = self.selection_text(patch);
        if t.is_empty() {
            self.msg = "nothing to copy".into();
            return;
        }
        let mut out = std::io::stdout();
        let _ = write!(out, "\x1b]52;c;{}\x07", base64(t.as_bytes()));
        let _ = out.flush();
        self.msg = format!("copied {} line(s)", t.lines().count().max(1));
    }

    fn find(&mut self, fwd: bool) {
        if self.search.is_empty() {
            return;
        }
        let smart = !self.search.chars().any(char::is_uppercase);
        let q = if smart { self.search.to_ascii_lowercase() } else { self.search.clone() };
        let qlen = q.chars().count();
        let n = self.rows.len();
        let (s, _) = self.ordered();
        for k in 0..=n {
            let r = if fwd { (self.head.row + k) % n } else { (self.head.row + n - k) % n };
            let raw = &self.rows[r].line.text;
            let t = if smart { raw.to_ascii_lowercase() } else { raw.clone() };
            let mut hits = t.match_indices(q.as_str()).map(|(b, _)| t[..b].chars().count());
            let hit = if fwd {
                hits.find(|&c| k > 0 || c > self.head.col)
            } else {
                hits.filter(|&c| k > 0 || c < s.col).last()
            };
            if let Some(c) = hit {
                self.anchor = Pos { row: r, col: c };
                self.head = Pos { row: r, col: c + qlen - 1 };
                self.want_col = self.head.col;
                return;
            }
        }
        self.msg = format!("no match: {}", self.search);
    }

    fn set_context(&mut self, f: impl Fn(u32) -> u32) {
        let Some(p) = self.file_path().map(str::to_owned) else { return };
        let cur = *self.context.get(&p).unwrap_or(&DEFAULT_CONTEXT);
        self.context.insert(p, f(cur));
        if let Err(e) = self.reload(false) {
            self.msg = e.to_string();
        }
    }

    fn toggle_collapse(&mut self) {
        let fi = self.rows[self.head.row].file;
        if fi == INFO {
            self.info_collapsed = !self.info_collapsed;
            self.build();
            self.select = false;
            return self.goto(0, 0);
        }
        let Some(p) = self.files.get(fi).map(|f| f.path.clone()) else { return };
        if !self.collapsed.remove(&p) {
            self.collapsed.insert(p);
        }
        self.build();
        self.select = false;
        let r = self.rows.iter().position(|r| r.file == fi).unwrap_or(0);
        self.goto(r, 0);
    }

    fn open_target(&self) -> Option<String> {
        let row = &self.rows[self.head.row];
        let f = self.files.get(row.file)?;
        // Deleted lines have no new-side position: use the next new-side line in the file.
        let line = self.rows[self.head.row..]
            .iter()
            .take_while(|r| r.file == row.file)
            .find_map(|r| r.line.new)
            .unwrap_or(1);
        let col = if row.line.new.is_some() { self.head.col + 1 } else { 1 };
        Some(format!("{}:{line}:{col}", f.path))
    }

    fn scroll(&mut self, d: isize) {
        let last = self.rows.len() as isize - 1;
        self.top = (self.top as isize + d).clamp(0, last) as usize;
        let so = 3.min(self.height / 2);
        let (lo, hi) = (self.top + so, (self.top + self.height).saturating_sub(so + 1));
        if self.head.row < lo || self.head.row > hi {
            let w = self.want_col;
            self.goto(self.head.row.clamp(lo, hi.max(lo)), w);
            self.want_col = w;
        }
    }

    /// Label every visible word (2+ chars), nearest to the cursor first, alternating forward/back.
    fn show_labels(&mut self) {
        let mut words = vec![];
        for row in self.top..(self.top + self.height).min(self.rows.len()) {
            if matches!(self.rows[row].line.kind, Kind::File | Kind::Hunk) {
                continue;
            }
            let chars = self.chars(row);
            let class = |c: char| if c.is_whitespace() { 0 } else if c.is_alphanumeric() || c == '_' { 1 } else { 2 };
            let mut i = self.left;
            while i < chars.len() {
                let (c, start) = (class(chars[i]), i);
                while i < chars.len() && class(chars[i]) == c {
                    i += 1;
                }
                if c != 0 && i - start >= 2 {
                    words.push((Pos { row, col: start }, Pos { row, col: i - 1 }));
                }
            }
        }
        let split = words.partition_point(|w| w.0 <= self.head);
        let (back, fwd) = words.split_at(split);
        let (mut fwd, mut back) = (fwd.iter(), back.iter().rev());
        let n = self.alphabet.len();
        let mut order = vec![];
        loop {
            let (f, b) = (fwd.next(), back.next());
            if f.is_none() && b.is_none() {
                break;
            }
            order.extend(f.into_iter().chain(b).copied());
        }
        self.labels = order
            .into_iter()
            .take(n * n)
            .enumerate()
            .map(|(i, (s, e))| (s, e, [self.alphabet[i / n], self.alphabet[i % n]]))
            .collect();
        self.label_first = None;
        if self.labels.is_empty() {
            self.msg = "no words to label".into();
        }
    }

    fn label_key(&mut self, k: KeyEvent) {
        let KeyCode::Char(c) = k.code else {
            self.labels.clear();
            return;
        };
        match self.label_first {
            None if self.labels.iter().any(|l| l.2[0] == c) => self.label_first = Some(c),
            Some(f) => {
                if let Some(&(s, e, _)) = self.labels.iter().find(|l| l.2 == [f, c]) {
                    self.sel(s, e);
                }
                self.labels.clear();
            }
            None => self.labels.clear(),
        }
    }

    /// Jump within the diff, remembering where we came from (Ctrl-o goes back).
    fn jump(&mut self, row: usize, col: usize) {
        self.jumps.push(self.head);
        self.select = false;
        self.goto(row, col);
    }

    // ---- rust-analyzer ----

    fn lsp_request(&mut self, method: &str, req: Req) {
        let row = &self.rows[self.head.row];
        let (Some(n), Some(f)) = (row.line.new, self.files.get(row.file)) else {
            self.msg = "not on a new-side line".into();
            return;
        };
        let mut params = json!({
            "textDocument": { "uri": lsp::uri(&self.root.join(&f.path)) },
            "position": { "line": n - 1, "character": lsp::utf16_col(&row.line.text, self.head.col) },
        });
        if method.ends_with("references") {
            params["context"] = json!({ "includeDeclaration": true });
        }
        match &mut self.lsp {
            None => self.msg = "rust-analyzer is not running".into(),
            Some(_) if !self.lsp_ready => self.msg = "rust-analyzer is starting…".into(),
            Some(l) => match l.request(method, params) {
                Ok(id) => _ = self.inflight.insert(id, req),
                Err(e) => self.msg = format!("rust-analyzer: {e}"),
            },
        }
    }

    fn lsp_poll(&mut self) {
        let msgs: Vec<Value> = self.lsp.as_ref().map(|l| l.rx.try_iter().collect()).unwrap_or_default();
        for m in msgs {
            self.lsp_msg(m);
        }
    }

    fn lsp_msg(&mut self, m: Value) {
        let Some(l) = &mut self.lsp else { return };
        match (m.get("id"), m["method"].as_str()) {
            (Some(id), Some(_)) => _ = l.reply(id.clone()),
            (None, Some("$/progress")) => {
                let (tok, v) = (m["params"]["token"].to_string(), &m["params"]["value"]);
                if v["kind"] == "end" {
                    self.progress.remove(&tok);
                } else {
                    let e = self.progress.entry(tok).or_default();
                    if let Some(t) = v["title"].as_str() {
                        e.0 = t.to_owned();
                    }
                    e.1 = v["message"].as_str().unwrap_or("").to_owned();
                }
            }
            (Some(id), None) => {
                let Some(req) = id.as_u64().and_then(|id| self.inflight.remove(&id)) else { return };
                if let Some(err) = m.get("error") {
                    self.msg = format!("rust-analyzer: {}", err["message"].as_str().unwrap_or("error"));
                    return;
                }
                let res = &m["result"];
                match req {
                    Req::Init => {
                        self.lsp_ready = true;
                        _ = l.notify("initialized", json!({}));
                    }
                    Req::Hover => match lsp::hover_text(res).trim().to_owned() {
                        t if t.is_empty() => self.msg = "no hover info".into(),
                        t => self.popup = Some((t, 0)),
                    },
                    Req::Goto => self.show_locations(lsp::locations(res)),
                }
            }
            _ => {}
        }
    }

    fn show_locations(&mut self, locs: Vec<Loc>) {
        match locs.as_slice() {
            [] => self.msg = "no results".into(),
            [l] => self.jump_loc(l),
            _ => {
                let mut cache = HashMap::new();
                let items = locs
                    .into_iter()
                    .map(|l| {
                        let mark = if self.loc_row(&l).is_some() { "●" } else { " " };
                        let shown = l.path.strip_prefix(&self.root).unwrap_or(&l.path).display().to_string();
                        let text = disk_line(&mut cache, &l.path, l.line);
                        let label = format!("{mark} {shown}:{}  {}", l.line + 1, text.trim());
                        Item { label, target: Target::Loc(l) }
                    })
                    .collect();
                self.picker = Some(Picker { title: " locations  ● = in diff ", items, query: String::new(), sel: 0 });
            }
        }
    }

    /// The diff row showing this location's line, if it is in the (expanded) diff.
    fn loc_row(&self, l: &Loc) -> Option<usize> {
        let rel = l.path.strip_prefix(&self.root).ok()?.to_str()?;
        let fi = self.files.iter().position(|f| f.path == rel)?;
        self.rows.iter().position(|r| r.file == fi && r.line.new == Some(l.line + 1))
    }

    fn jump_loc(&mut self, l: &Loc) {
        match self.loc_row(l) {
            Some(r) => {
                let col = lsp::char_col(&self.rows[r].line.text, l.character);
                self.jump(r, col);
            }
            None => {
                let col = lsp::char_col(&disk_line(&mut HashMap::new(), &l.path, l.line), l.character);
                self.action = Some(Action::Open(format!("{}:{}:{}", l.path.display(), l.line + 1, col + 1)));
            }
        }
    }

    // ---- keys ----

    fn key(&mut self, k: KeyEvent) {
        self.msg.clear();
        if let Some((_, scroll)) = &mut self.popup {
            match k.code {
                KeyCode::Char('j') | KeyCode::Down => *scroll += 1,
                KeyCode::Char('k') | KeyCode::Up => *scroll = scroll.saturating_sub(1),
                _ => self.popup = None,
            }
            return;
        }
        if !self.labels.is_empty() {
            return self.label_key(k);
        }
        if self.picker.is_some() {
            return self.picker_key(k);
        }
        if let Some(q) = &mut self.prompt {
            match k.code {
                KeyCode::Esc => self.prompt = None,
                KeyCode::Enter => {
                    self.search = self.prompt.take().unwrap_or_default();
                    self.find(true);
                }
                KeyCode::Backspace if q.is_empty() => self.prompt = None,
                KeyCode::Backspace => _ = q.pop(),
                KeyCode::Char(c) => q.push(c),
                _ => {}
            }
            return;
        }
        let ch = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);
        let c = match k.code {
            KeyCode::Char(c) => c,
            KeyCode::Esc => {
                (self.pending, self.count, self.select) = (None, 0, false);
                return;
            }
            KeyCode::Enter => {
                self.action = self.open_target().map(Action::Open);
                return;
            }
            KeyCode::Tab if self.rows[self.head.row].thread.is_some() => return self.toggle_thread(),
            KeyCode::Tab => return self.toggle_collapse(),
            KeyCode::Left => return self.key(ch('h')),
            KeyCode::Down => return self.key(ch('j')),
            KeyCode::Up => return self.key(ch('k')),
            KeyCode::Right => return self.key(ch('l')),
            _ => return,
        };
        if k.modifiers.contains(KeyModifiers::CONTROL) {
            let half = (self.height / 2).max(1) as isize;
            match c {
                'd' => {
                    self.top = (self.top + half as usize).min(self.rows.len() - 1);
                    self.vmove(half);
                }
                'u' => {
                    self.top = self.top.saturating_sub(half as usize);
                    self.vmove(-half);
                }
                'e' => self.scroll(5),
                'y' => self.scroll(-5),
                'o' => {
                    if let Some(p) = self.jumps.pop() {
                        self.select = false;
                        self.goto(p.row, p.col);
                    }
                }
                'c' => self.quit = true,
                _ => {}
            }
            return;
        }
        if k.modifiers.contains(KeyModifiers::ALT) {
            match c {
                'n' => self.find(true),
                'N' => self.find(false),
                _ => {}
            }
            return;
        }
        if let Some(p) = self.pending.take() {
            return self.pending_key(p, c);
        }
        if let Some(d) = c.to_digit(10).filter(|&d| d > 0 || self.count > 0) {
            self.count = self.count * 10 + d as usize;
            return;
        }
        let n = std::mem::take(&mut self.count).max(1);
        match c {
            'h' => self.goto(self.head.row, self.head.col.saturating_sub(n)),
            'l' => self.goto(self.head.row, self.head.col + n),
            'j' => self.vmove(n as isize),
            'k' => self.vmove(-(n as isize)),
            'w' | 'b' | 'e' | 'W' | 'B' | 'E' => (0..n).for_each(|_| self.word(c)),
            'x' => (0..n).for_each(|_| self.line_select()),
            'v' => self.select = !self.select,
            ';' => self.anchor = self.head,
            'y' => self.yank(false),
            'Y' => self.yank(true),
            '/' => self.prompt = Some(String::new()),
            'n' => (0..n).for_each(|_| self.next_kind(true, change_start)),
            'N' => (0..n).for_each(|_| self.next_kind(false, change_start)),
            '*' => {
                self.search = self.selection_text(false).trim_end_matches('\n').to_owned();
                self.msg = format!("search: {}", self.search);
            }
            '+' | '=' => self.set_context(|c| c.saturating_add(10)),
            '-' => self.set_context(|c| if c >= FULL { DEFAULT_CONTEXT } else { c.saturating_sub(10) }),
            'Z' => self.set_context(|c| if c >= FULL { DEFAULT_CONTEXT } else { FULL }),
            'c' => self.start_comment(),
            'r' => self.start_reply(),
            'q' => self.quit = true,
            'g' | ']' | '[' | ' ' | 'f' | 't' | 'F' | 'T' | 'z' => self.pending = Some(c),
            _ => {}
        }
    }

    fn pending_key(&mut self, p: char, c: char) {
        let file = |rows: &[Row], r: usize| rows[r].line.kind == Kind::File;
        let thread = |rows: &[Row], r: usize| rows[r].thread.is_some() && (r == 0 || rows[r - 1].thread != rows[r].thread);
        let so = 3.min(self.height / 2);
        match (p, c) {
            ('g', 'g') => self.goto(0, 0),
            ('g', 'e') => self.goto(self.rows.len() - 1, 0),
            ('g', 'h') => self.goto(self.head.row, 0),
            ('g', 'l') => self.goto(self.head.row, usize::MAX),
            ('g', 's') => {
                let col = self.chars(self.head.row).iter().position(|c| !c.is_whitespace()).unwrap_or(0);
                self.goto(self.head.row, col);
            }
            ('g', 'd') => self.lsp_request("textDocument/definition", Req::Goto),
            ('g', 'y') => self.lsp_request("textDocument/typeDefinition", Req::Goto),
            ('g', 'i') => self.lsp_request("textDocument/implementation", Req::Goto),
            ('g', 'r') => self.lsp_request("textDocument/references", Req::Goto),
            ('g', 'w') => self.show_labels(),
            (' ', 'k') => self.lsp_request("textDocument/hover", Req::Hover),
            (']', 'g') => self.next_kind(true, change_start),
            ('[', 'g') => self.next_kind(false, change_start),
            (']', 'c') => self.next_kind(true, thread),
            ('[', 'c') => self.next_kind(false, thread),
            ('z', 'z') => self.top = self.head.row.saturating_sub(self.height / 2),
            ('z', 't') => self.top = self.head.row.saturating_sub(so),
            ('z', 'b') => self.top = (self.head.row + so + 1).saturating_sub(self.height),
            (']', 'f') => self.next_kind(true, file),
            ('[', 'f') => self.next_kind(false, file),
            (' ', 'f') => {
                let items = self
                    .rows
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| r.line.kind == Kind::File)
                    .map(|(i, r)| Item { label: r.line.text.clone(), target: Target::Row(i) })
                    .collect();
                self.picker = Some(Picker { title: " files ", items, query: String::new(), sel: 0 });
            }
            (' ', 'y') => self.yank(false),
            ('f' | 't' | 'F' | 'T', c) => self.find_char(p, c),
            _ => {}
        }
    }

    /// Picker items whose label contains the query as a (case-insensitive) subsequence.
    fn matches(&self) -> Vec<usize> {
        let Some(p) = &self.picker else { return vec![] };
        let q = p.query.to_lowercase();
        (0..p.items.len())
            .filter(|&i| {
                let mut label = p.items[i].label.to_lowercase().chars().collect::<Vec<_>>().into_iter();
                q.chars().all(|c| label.any(|x| x == c))
            })
            .collect()
    }

    fn picker_key(&mut self, k: KeyEvent) {
        let m = self.matches();
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let Some(p) = &mut self.picker else { return };
        match k.code {
            KeyCode::Esc => self.picker = None,
            KeyCode::Enter => {
                let mut p = self.picker.take().unwrap();
                if let Some(&i) = m.get(p.sel) {
                    match p.items.swap_remove(i).target {
                        Target::Row(r) => self.jump(r, 0),
                        Target::Loc(l) => self.jump_loc(&l),
                    }
                }
            }
            KeyCode::Up => p.sel = p.sel.saturating_sub(1),
            KeyCode::Char('p') if ctrl => p.sel = p.sel.saturating_sub(1),
            KeyCode::Down | KeyCode::Tab => p.sel = (p.sel + 1).min(m.len().saturating_sub(1)),
            KeyCode::Char('n') if ctrl => p.sel = (p.sel + 1).min(m.len().saturating_sub(1)),
            KeyCode::Backspace => {
                p.query.pop();
                p.sel = 0;
            }
            KeyCode::Char(c) => {
                p.query.push(c);
                p.sel = 0;
            }
            _ => {}
        }
    }

    // ---- render ----

    fn draw(&mut self, f: &mut Frame) {
        let area = f.area();
        let h = area.height.saturating_sub(1) as usize;
        self.height = h.max(1);
        let so = 3.min(h / 2);
        if self.head.row < self.top + so {
            self.top = self.head.row.saturating_sub(so);
        }
        if self.head.row + so + 1 > self.top + h {
            self.top = self.head.row + so + 1 - h;
        }

        let maxno = self.rows.iter().filter_map(|r| r.line.old.max(r.line.new)).max().unwrap_or(0);
        let lw = maxno.to_string().len().max(3);
        let gw = lw * 2 + 4;
        let tw = (area.width as usize).saturating_sub(gw).max(1);
        let ww = tw.saturating_sub(1).clamp(40, 100);
        if ww != self.wrap_w {
            self.wrap_w = ww;
            let keep = self.cursor_key();
            self.rebuild(keep);
        }
        if self.head.col < self.left {
            self.left = self.head.col;
        }
        if self.head.col >= self.left + tw {
            self.left = self.head.col + 1 - tw;
        }

        let (s, e) = self.ordered();
        let overlay: HashMap<Pos, char> = self
            .labels
            .iter()
            .filter(|l| self.label_first.is_none_or(|f| l.2[0] == f))
            .flat_map(|&(p, _, [a, b])| match self.label_first {
                None => vec![(p, a), (Pos { col: p.col + 1, ..p }, b)],
                Some(_) => vec![(p, b)], // after the first key, show the remaining char at the word start
            })
            .collect();
        let lines: Vec<Line> = (self.top..(self.top + h).min(self.rows.len()))
            .map(|r| self.render_row(r, s, e, lw, gw, tw, &overlay))
            .collect();
        f.render_widget(Paragraph::new(lines), Rect { height: h as u16, ..area });

        let status = Rect { y: area.y + h as u16, height: 1, ..area };
        let left = match &self.prompt {
            Some(q) => format!("/{q}"),
            None => {
                let l = &self.rows[self.head.row].line;
                let pos = match (l.new, l.old) {
                    (Some(n), _) => format!("{n}:{}", self.head.col + 1),
                    (None, Some(o)) => format!("-{o}:{}", self.head.col + 1),
                    _ => String::new(),
                };
                let mode = if self.select { "SEL" } else { "NOR" };
                format!(" {mode}  {}  {pos}  {}", self.file_path().unwrap_or(""), self.msg)
            }
        };
        let pend = self.pending.map(String::from).unwrap_or_default();
        let count = if self.count > 0 { self.count.to_string() } else { String::new() };
        let ra = match self.progress.values().next() {
            Some((t, m)) => format!("ra: {t} {m}  "),
            None if self.lsp.is_some() && !self.lsp_ready => "ra: starting  ".into(),
            None => String::new(),
        };
        let pr = self.pr.as_ref().map(|p| format!("PR #{} · {} threads  ", p.number, self.threads.len())).unwrap_or_default();
        let right = format!("{ra}{pr}{count}{pend}  {}/{} ", self.head.row + 1, self.rows.len());
        let bar = Style::new().bg(Color::Rgb(0x2a, 0x2e, 0x38));
        f.render_widget(Paragraph::new(left).style(bar), status);
        f.render_widget(Paragraph::new(right).alignment(Alignment::Right).style(bar), status);

        if let Some(p) = &self.picker {
            let m = self.matches();
            let sel = p.sel.min(m.len().saturating_sub(1));
            let w = (area.width * 3 / 4).max(20).min(area.width);
            let ph = (m.len() as u16 + 3).min(area.height.saturating_sub(4)).max(3);
            let r = Rect { x: (area.width - w) / 2, y: 2.min(area.height), width: w, height: ph.min(area.height) };
            let vis = (ph as usize).saturating_sub(3);
            let off = sel.saturating_sub(vis.saturating_sub(1));
            let mut lines = vec![Line::from(format!("> {}", p.query))];
            lines.extend(m.iter().enumerate().skip(off).take(vis).map(|(i, &it)| {
                let st = if i == sel { Style::new().reversed() } else { Style::new() };
                Line::styled(p.items[it].label.clone(), st)
            }));
            f.render_widget(Clear, r);
            f.render_widget(Paragraph::new(lines).block(Block::bordered().title(p.title)), r);
        }

        if let Some((text, scroll)) = &self.popup {
            let w = area.width.saturating_sub(4).min(100);
            let h = (text.lines().count() as u16 + 2).min(area.height.saturating_sub(4)).max(3);
            let r = Rect { x: (area.width - w) / 2, y: 2.min(area.height), width: w, height: h.min(area.height) };
            f.render_widget(Clear, r);
            let p = Paragraph::new(text.as_str()).wrap(Wrap { trim: false }).scroll((*scroll, 0));
            f.render_widget(p.block(Block::bordered().title(" hover  j/k scroll ")), r);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_row(
        &self, r: usize, s: Pos, e: Pos, lw: usize, gw: usize, tw: usize, overlay: &HashMap<Pos, char>,
    ) -> Line<'static> {
        let row = &self.rows[r];
        let l = &row.line;
        let num = |n: Option<u32>| n.map(|n| n.to_string()).unwrap_or_default();
        let (gutter, style) = match l.kind {
            Kind::File => {
                let collapsed = match row.file {
                    INFO => self.info_collapsed,
                    f => self.files.get(f).is_some_and(|f| self.collapsed.contains(&f.path)),
                };
                (format!("{:<gw$}", if collapsed { "▸" } else { "▾" }), Style::new().bold().fg(Color::Yellow))
            }
            Kind::Hunk => (" ".repeat(gw), Style::new().fg(Color::Cyan)),
            Kind::Thread | Kind::Comment => {
                let resolved = row.thread.is_some_and(|t| self.threads[t].resolved);
                let check = l.text.chars().next().filter(|_| row.file == INFO);
                let st = match (l.kind, resolved, check) {
                    (_, _, Some('✗')) => Style::new().fg(Color::Red),
                    (_, _, Some('●')) => Style::new().fg(Color::Yellow),
                    (_, true, _) => Style::new().fg(Color::DarkGray),
                    (Kind::Thread, _, _) => Style::new().fg(Color::LightBlue).bold(),
                    _ => Style::new(),
                };
                (format!("{:>gw$}", "┃ "), st)
            }
            k => {
                let (m, st) = match k {
                    Kind::Add => ('+', Style::new().fg(Color::Green)),
                    Kind::Del => ('-', Style::new().fg(Color::Red)),
                    _ => (' ', Style::new()),
                };
                (format!("{:>lw$} {:>lw$} {m} ", num(l.old), num(l.new)), st)
            }
        };
        let mut spans = vec![Span::styled(gutter, Style::new().fg(Color::DarkGray))];
        let mut n = 0;
        for (i, c) in l.text.chars().enumerate().skip(self.left).take(tw) {
            let p = Pos { row: r, col: i };
            let mut st = style;
            if s <= p && p <= e {
                st = st.bg(Color::Rgb(0x3b, 0x42, 0x52));
            }
            if p == self.head {
                st = st.add_modifier(Modifier::REVERSED);
            }
            if let Some(&lc) = overlay.get(&p) {
                spans.push(Span::styled(lc.to_string(), Style::new().fg(Color::Magenta).bold()));
                n = i + 1;
                continue;
            }
            // ponytail: tab drawn as 4 cols but scrolled as 1 char; fine unless tab-heavy files.
            spans.push(Span::styled(if c == '\t' { "    ".into() } else { c.to_string() }, st));
            n = i + 1;
        }
        if self.head.row == r && self.head.col >= n.max(self.left) {
            spans.push(Span::styled(" ", Style::new().add_modifier(Modifier::REVERSED)));
        }
        Line::from(spans)
    }
}

/// Write a template, open $VISUAL/$EDITOR (default hx), and return the body file, or None if left empty.
fn compose(what: &str) -> Result<Option<String>> {
    const MARK: &str = "<!-- dv:";
    let path = std::env::temp_dir().join(format!("dv-comment-{}.md", std::process::id()));
    std::fs::write(&path, format!("\n{MARK} {what}. Save and quit to post; leave empty to cancel. -->\n"))?;
    let editor = std::env::var("VISUAL").or_else(|_| std::env::var("EDITOR")).unwrap_or_else(|_| "hx".into());
    let mut parts = editor.split_whitespace();
    let st = std::process::Command::new(parts.next().unwrap_or("hx")).args(parts).arg(&path).status()?;
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let body: Vec<&str> = text.lines().filter(|l| !l.starts_with(MARK)).collect();
    let body = body.join("\n").trim().to_owned();
    if !st.success() || body.is_empty() {
        _ = std::fs::remove_file(&path);
        return Ok(None);
    }
    std::fs::write(&path, &body)?;
    Ok(Some(path.to_string_lossy().into_owned()))
}

/// Word-wrap long lines at `w` chars; short lines (and their indentation) are kept as-is.
fn wrap(text: &str, w: usize) -> Vec<String> {
    let mut out = vec![];
    for line in text.lines() {
        if line.chars().count() <= w {
            out.push(line.to_owned());
            continue;
        }
        let mut cur = String::new();
        for word in line.split(' ') {
            if !cur.is_empty() && cur.chars().count() + 1 + word.chars().count() > w {
                out.push(std::mem::take(&mut cur));
            } else if !cur.is_empty() {
                cur.push(' ');
            }
            cur.push_str(word);
        }
        out.push(cur);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

/// `editor.jump-label-alphabet` from the Helix config, else Helix's default.
fn jump_alphabet() -> Vec<char> {
    let cfg = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|_| std::env::var("HOME").map(|h| Path::new(&h).join(".config")))
        .map(|d| std::fs::read_to_string(d.join("helix/config.toml")).unwrap_or_default())
        .unwrap_or_default();
    // ponytail: line scan instead of a TOML parser; fine for `key = "value"` on one line.
    cfg.lines()
        .filter_map(|l| l.trim().strip_prefix("jump-label-alphabet")?.trim().strip_prefix('=')?.trim().strip_prefix('"')?.strip_suffix('"').map(str::to_owned))
        .next()
        .filter(|a| !a.is_empty())
        .unwrap_or_else(|| "abcdefghijklmnopqrstuvwxyz".into())
        .chars()
        .collect()
}

/// Line `n` (0-based) of a file on disk, reading each file once per `cache`.
fn disk_line(cache: &mut HashMap<PathBuf, String>, p: &Path, n: u32) -> String {
    let text = cache.entry(p.to_owned()).or_insert_with(|| std::fs::read_to_string(p).unwrap_or_default());
    text.lines().nth(n as usize).unwrap_or("").to_owned()
}

/// First row of a run of added/removed lines.
fn change_start(rows: &[Row], r: usize) -> bool {
    let is = |r: usize| matches!(rows[r].line.kind, Kind::Add | Kind::Del);
    is(r) && (r == 0 || !is(r - 1))
}

fn base64(b: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut o = String::new();
    for c in b.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            o.push(if i <= c.len() { T[(n >> (18 - 6 * i)) as usize & 63] as char } else { '=' });
        }
    }
    o
}

#[test]
fn keys_and_yank() {
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    let press = |app: &mut App, s: &str| s.chars().for_each(|c| app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)));
    press(&mut app, "n"); // first change: the deleted line
    assert_eq!(app.rows[app.head.row].line.kind, Kind::Del);
    press(&mut app, "jww");
    assert_eq!(app.selection_text(false), "new_call");
    press(&mut app, "x");
    assert_eq!(app.selection_text(false), "    new_call(x);\n");
    press(&mut app, "kx");
    assert_eq!(app.selection_text(true), "-    old();\n");
    let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
    press(&mut app, "/call");
    app.key(enter);
    assert_eq!((app.anchor, app.head), (Pos { row: 4, col: 8 }, Pos { row: 4, col: 11 }));
    press(&mut app, "/CALL");
    app.key(enter);
    assert!(app.msg.starts_with("no match")); // smart case: uppercase query is case-sensitive
    assert_eq!(base64(b"hello"), "aGVsbG8=");
    app.alphabet = vec!['a', 's'];
    app.height = 20;
    app.goto(4, 0);
    press(&mut app, "gw"); // nearest word after the cursor ("new_call") gets "aa"
    assert_eq!(app.labels[0].2, ['a', 'a']);
    press(&mut app, "aa");
    assert_eq!(app.selection_text(false), "new_call");
}

#[test]
fn comments_placed_and_anchored() {
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    let c = |id, body: &str| gh::Comment { id, author: "amy".into(), body: body.into(), created: "2026-10-01".into() };
    let t = |left, line, resolved, comments| Thread {
        path: "src/a.rs".into(), left, line, start_line: None, original_line: Some(1), resolved, outdated: line.is_none(), comments,
    };
    app.threads = vec![
        t(true, Some(2), false, vec![c(1, "why remove?"), c(2, "unused")]), // on the deleted line
        t(false, None, true, vec![c(3, "old note")]),                        // outdated + resolved → folded
    ];
    app.build();
    let kinds: Vec<_> = app.rows.iter().take(10).map(|r| (r.line.kind, r.thread)).collect();
    assert_eq!(kinds, [
        (Kind::File, None),
        (Kind::Thread, Some(1)), // outdated, folded, under the header
        (Kind::Hunk, None),
        (Kind::Ctx, None),
        (Kind::Del, None),
        (Kind::Thread, Some(0)),
        (Kind::Comment, Some(0)),
        (Kind::Thread, Some(0)),
        (Kind::Comment, Some(0)),
        (Kind::Add, None),
    ]);
    // Select from the deleted line down to the added line (across the thread): multi-line, mixed sides.
    app.goto(4, 0);
    app.select = true;
    app.goto(9, 0);
    let a = app.selection_anchor().unwrap();
    assert_eq!((a.line, a.left, a.start), (2, false, Some((2, true))));
    assert_eq!(wrap("aaa bbb ccc", 7), ["aaa bbb", "ccc"]);
}
