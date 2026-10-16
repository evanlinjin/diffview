//! The diff view: state, loading and laying out rows. Keys, motions, rendering, GitHub review
//! actions and rust-analyzer live in the submodules.
use crate::diff::{self, FileDiff, Kind, Line as DLine};
use crate::gh::{self, Anchor, Pr, Thread};
use crate::highlight::{Colors, Highlighter};
use crate::theme::{Settings, Theme, Ui};
use crate::lsp::{self, Loc, Lsp};
use anyhow::{bail, Context, Result};
use ratatui::{
    crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    layout::{Alignment, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap},
    DefaultTerminal, Frame,
};
use std::collections::{HashMap, HashSet};
use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

mod keys;
mod motion;
mod ra;
mod render;
mod review;
mod select;
#[cfg(test)]
mod tests;

const HELP: &str = "
MOVE
  h j k l, arrows      move (counts: 5j)
  w b e  W B E         word motions (select)
  gh gl gs             line start / end / first non-blank
  f t F T <c>          find char
  gw                   jump labels
  mm                   matching bracket
  gg ge                top / end
  Ctrl-d Ctrl-u        half page down / up
  Ctrl-e Ctrl-y        scroll 5 lines
  zz zt zb             cursor line to center / top / bottom
  Ctrl-o               jump back

DIFF
  n N, ]g [g           next / prev hunk
  ]f [f, Space f       next / prev file, file picker
  ]c [c                next / prev comment thread
  /  Alt-n Alt-N  *    search, next / prev match, search selection

SELECT & COPY
  x                    select line (repeat to extend)
  v  ;  Esc            select mode / collapse / exit select mode
  y, Space y           copy text
  Y                    copy as patch
  Space c              copy file:line ref (e.g. src/a.rs:12-15)

VIEW
  + =  -               hunk context +10 / -10 lines
  Z                    whole file
  Tab                  fold file / PR info / thread
  Shift-Tab            fold / unfold all files
  Ctrl-w v             toggle split (side-by-side) view
  Ctrl-h Ctrl-l        split view: go to the old / new side
  Enter                open hx at cursor (on a link: open it)
  gf                   open the link under the cursor in the browser

RUST-ANALYZER
  gd gr gy gi          definition / references / type / impl
  Space k              hover

PR
  c                    comment on selection / reply on a thread (pending review)
  C                    same, but post now; on PR text: quote it in a new conversation comment
  s                    suggest a change to the selected lines (pending review)
  S                    submit review: comment / approve / request changes, or discard
  Space r              resolve / unresolve thread
  Space e              edit your comment under the cursor
  Space d              delete the comment under the cursor (asks first)
  Space p              pick from your pending comments
  Space t              pick from all review threads
  Space v              mark / unmark the file as viewed (folds it, moves on)

GENERAL
  ?                    this help (/ to search it)
  R                    refresh (git diff + GitHub)
  q, Ctrl-c            quit
";

const DEFAULT_CONTEXT: u32 = 3;
/// Pending-key marker for Ctrl-w (window commands).
const CTRL_W: char = '\u{17}';
/// Files bigger than this aren't highlighted.
const HL_MAX_BYTES: usize = 1 << 20;
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
    /// Per-char styles for markdown rows; code rows use `hl_cache` instead.
    syn: Vec<Option<Style>>,
    /// Links in rendered markdown: (start, end, url) char ranges.
    links: Vec<(usize, usize, String)>,
    /// For PR info rows: the index in `info` of the markdown body this row belongs to.
    info: Option<usize>,
}

impl Row {
    fn new(file: usize, line: DLine) -> Row {
        Row { file, line, thread: None, syn: vec![], links: vec![], info: None }
    }

    /// A row of plain text (headers, comments, PR info).
    fn text(file: usize, kind: Kind, text: String) -> Row {
        Row::new(file, DLine { kind, old: None, new: None, text })
    }
}

/// What the cursor is on, independent of row indices.
struct CursorKey {
    /// (file, old, new) for a diff row.
    line: Option<(String, Option<u32>, Option<u32>)>,
    /// Root comment id for a thread row.
    thread: Option<u64>,
    /// Row index for a PR info row (the section is at the top and only grows at its end).
    info: Option<usize>,
}

/// A screen row: full width, or a side-by-side pair (old line, new line) in split view.
#[derive(Clone, Copy, Debug, PartialEq)]
enum DRow {
    Full(usize),
    Pair(Option<usize>, Option<usize>),
}

/// Work that needs the terminal back (an editor), run between frames.
enum Action {
    Open(String),
    /// `review`: add to the pending review instead of posting immediately.
    /// `prefill`: starting text in the editor (a ```suggestion block for `s`).
    Comment { anchor: Anchor, review: bool, prefill: String },
    /// `quote`: the comment under the cursor as a `> ` quote, pre-filled in the editor.
    Reply { thread: String, comment: u64, review: bool, quote: String },
    /// A new PR conversation comment, starting with `quote` (may be empty).
    Converse { quote: String },
    /// Rewrite one of your comments (node id, current body).
    Edit { comment: String, body: String, author: String },
    /// Submit the pending review: COMMENT, APPROVE or REQUEST_CHANGES.
    Submit(&'static str),
}

/// Network work that doesn't need the terminal; run after a frame shows its "…" message.
enum Job {
    Refresh,
    Resolve(String, bool),
    /// Mark (true) or unmark a file as viewed.
    Viewed(String, bool),
    Discard(String),
    DeleteComment(String),
}

enum Target {
    Row(usize),
    Loc(Loc),
    Submit(&'static str),
    /// Asks for confirmation first; `DiscardYes` does it.
    Discard,
    DiscardYes,
    DeleteComment(String),
    /// A comment: (thread index, comment index within it).
    Comment(usize, usize),
    Nothing,
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
    /// Extra context lines per hunk: (path, hunk's first full index) → lines (can be negative).
    extra: HashMap<(String, usize), i64>,
    /// Files shown whole (`Z`).
    whole: HashSet<String>,
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
    width: u16,
    action: Option<Action>,
    /// Done on the next loop, so its "…" message is drawn first.
    job: Option<Job>,
    /// The viewer's pending review on the PR, if any.
    review: Option<String>,
    /// Your "Viewed" marks from GitHub: path → VIEWED / DISMISSED.
    viewed: HashMap<String, String>,
    /// Side-by-side view; `right` is which side the cursor prefers on lines shown on both.
    split: bool,
    right: bool,
    /// Screen rows, and the screen row of each row (`top` and scrolling count screen rows).
    drows: Vec<DRow>,
    d_of: Vec<usize>,
    /// Changed-word spans (char ranges) for paired removed/added rows.
    emph: HashMap<usize, Vec<(usize, usize)>>,
    /// Helix `cursorline` / relative `line-number`.
    settings: Settings,
    quit: bool,
    pr: Option<Pr>,
    threads: Vec<Thread>,
    info: Vec<gh::InfoLine>,
    info_collapsed: bool,
    hl: Highlighter,
    ui: Ui,
    /// Highlighted files by (path, new side?), filled lazily as rows are drawn.
    hl_cache: HashMap<(String, bool), Colors>,
    /// Highlighted markdown by wrapped text (filled while building rows, hence the RefCell).
    md_cache: std::cell::RefCell<HashMap<String, Vec<crate::md::MdLine>>>,
    /// Width comment/info text is wrapped to; follows the terminal.
    wrap_w: usize,
    /// Root comment ids of threads folded/unfolded against the default (resolved = folded).
    toggled: HashSet<u64>,
    root: PathBuf,
    lsp: Option<Lsp>,
    lsp_ready: bool,
    inflight: HashMap<u64, Req>,
    progress: HashMap<String, (String, String)>,
    /// Scrollable overlay: (title, text, scroll).
    popup: Option<(String, Vec<Line<'static>>, u16)>,
    /// Help overlay search: (query, typing it).
    help: Option<(String, bool)>,
    jumps: Vec<Pos>,
    alphabet: Vec<char>,
    /// `gw` jump labels: (word start, word end, label), plus the first label char typed.
    labels: Vec<(Pos, Pos, [char; 2])>,
    label_first: Option<char>,
}

impl App {
    pub fn new(base: String, pr: Option<Pr>) -> Result<Self> {
        let files = diff::load(&base, DEFAULT_CONTEXT)?;
        let mut app = Self::with(base, files);
        let theme = Theme::load();
        (app.hl, app.ui) = (Highlighter::new(&theme), theme.ui());
        app.settings = crate::theme::settings();
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
            extra: HashMap::new(),
            whole: HashSet::new(),
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
            width: 80,
            action: None,
            job: None,
            review: None,
            viewed: HashMap::new(),
            split: false,
            right: true,
            drows: vec![],
            d_of: vec![],
            emph: HashMap::new(),
            settings: Settings::default(),
            quit: false,
            pr: None,
            threads: vec![],
            info: vec![],
            info_collapsed: false,
            wrap_w: 100,
            hl: Highlighter::new(&Theme::ansi()),
            ui: Theme::ansi().ui(),
            hl_cache: HashMap::new(),
            md_cache: Default::default(),
            toggled: HashSet::new(),
            root: std::env::current_dir().unwrap_or_default(),
            lsp: None,
            lsp_ready: false,
            inflight: HashMap::new(),
            progress: HashMap::new(),
            popup: None,
            help: None,
            jumps: vec![],
            alphabet: jump_alphabet(),
            labels: vec![],
            label_first: None,
        };
        app.shape();
        app.build();
        app
    }

    pub fn run(&mut self, term: &mut DefaultTerminal) -> Result<()> {
        while !self.quit {
            term.draw(|f| self.draw(f))?;
            if let Some(job) = self.job.take() {
                self.run_job(job);
                continue;
            }
            // ponytail: 100ms poll so LSP replies show up without a keypress; a waker channel if it matters.
            if event::poll(Duration::from_millis(100))?
                && let Event::Key(k) = event::read()?
                    && k.kind == KeyEventKind::Press {
                        self.key(k);
                    }
            self.lsp_poll();
            if let Some(a) = self.action.take() {
                // In zellij the editor gets a floating pane and dv stays on screen underneath.
                let floating = in_zellij();
                if !floating {
                    ratatui::restore();
                }
                // Only posting changes GitHub state; skip the network round-trips after plain hx.
                let posted = !matches!(a, Action::Open(_));
                let res = self.perform(a);
                self.hl_cache.retain(|k, _| !k.1); // working-tree files may have changed; the base can't
                if !floating {
                    *term = ratatui::init();
                }
                if let Err(e) = self.reload(posted) {
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

    fn perform(&mut self, a: Action) -> Result<String> {
        match a {
            Action::Open(target) => {
                interactive("hx", std::slice::from_ref(&target), &format!("hx {target}")).context("running hx")?;
                Ok(String::new())
            }
            Action::Comment { anchor, review, prefill } => {
                let range = match anchor.start {
                    Some((s, _)) => format!("{s}-{}", anchor.line),
                    None => anchor.line.to_string(),
                };
                let kind = if prefill.is_empty() { "comment" } else { "suggestion" };
                let what = format!("{kind} on {}:{range}", anchor.path);
                let pr = self.pr.clone().context("no PR")?; // before the editor, so nothing written is lost
                let Some(body) = compose_with(&what, false, &prefill)? else { return Ok(format!("{kind} cancelled")) };
                let res = match review {
                    true => self.ensure_review().and_then(|r| gh::add_thread(&r, &anchor, &body)),
                    false => gh::comment(&pr, &anchor, &body),
                };
                keep_on_error(res, &body).map(|_| posted_msg(review))
            }
            Action::Reply { thread, comment, review, quote } => {
                let pr = self.pr.clone().context("no PR")?;
                let Some(body) = compose_with("reply", false, &quote)? else { return Ok("reply cancelled".into()) };
                let res = match review {
                    true => self.ensure_review().and_then(|r| gh::add_reply(&r, &thread, &body)),
                    false => gh::reply(&pr, comment, &body),
                };
                keep_on_error(res, &body).map(|_| posted_msg(review))
            }
            Action::Converse { quote } => {
                let pr = self.pr.clone().context("no PR")?;
                let Some(body) = compose_with("comment on the PR conversation", false, &quote)? else {
                    return Ok("comment cancelled".into());
                };
                keep_on_error(gh::conversation_comment(&pr, &body), &body)
                    .map(|_| "comment posted to the conversation".into())
            }
            Action::Edit { comment, body: old, author } => {
                // Unchanged or emptied counts as cancelled; deleting a comment is a separate thing.
                // Maintainers can edit others' comments too, so the hint names whose this is.
                let what = format!("editing @{author}'s comment");
                let Some(body) = compose_with(&what, false, &old)? else { return Ok("edit cancelled".into()) };
                keep_on_error(gh::edit_comment(&comment, &body), &body).map(|_| "comment updated".into())
            }
            Action::Submit(event) => {
                // An empty summary is fine; quitting the editor with an error (`:cq`) cancels.
                let Some(body) = compose(&format!("review summary ({})", event.to_lowercase().replace('_', " ")), true)?
                else {
                    return Ok("review not submitted".into());
                };
                let res = self.ensure_review().and_then(|r| gh::submit_review(&r, event, &body));
                keep_on_error(res, &body)?;
                self.review = None;
                Ok("review submitted".into())
            }
        }
    }

    fn reload(&mut self, threads: bool) -> Result<()> {
        let keep = self.cursor_key();
        if threads {
            self.load_threads();
        }
        self.files = diff::load(&self.base, DEFAULT_CONTEXT)?;
        self.shape();
        self.rebuild(keep);
        Ok(())
    }

    /// Recompute each file's shown lines from its full diff and the per-hunk context.
    fn shape(&mut self) {
        let (extra, whole) = (&self.extra, &self.whole);
        for f in &mut self.files {
            if f.full.is_empty() {
                f.prepare(); // parsed directly (tests)
            }
            let e = |a| *extra.get(&(f.path.clone(), a)).unwrap_or(&0);
            let w = diff::windows(&f.full, DEFAULT_CONTEXT as usize, e, whole.contains(&f.path));
            f.lines = diff::shown(&f.full, &w, &f.headers);
        }
    }

    fn cursor_key(&self) -> CursorKey {
        let r = &self.rows[self.head.row];
        CursorKey {
            line: self.files.get(r.file).map(|f| (f.path.clone(), r.line.old, r.line.new)),
            thread: r.thread.and_then(|t| self.root_id(t)),
            info: (r.file == INFO).then_some(self.head.row),
        }
    }

    /// Rebuild rows, keeping the cursor on the same code line, thread or PR info row.
    fn rebuild(&mut self, keep: CursorKey) {
        self.build();
        let row = keep
            .thread
            .and_then(|id| self.rows.iter().position(|r| r.thread.and_then(|t| self.root_id(t)) == Some(id)))
            .or_else(|| keep.line.map(|(p, old, new)| self.locate(&p, old, new)))
            .or(keep.info.filter(|&r| self.rows.get(r).is_some_and(|r| r.file == INFO)))
            .unwrap_or(0);
        self.select = false;
        self.goto(row, self.head.col);
    }

    fn build(&mut self) {
        self.jumps.clear();
        let mut rows = vec![];
        if let Some(pr) = &self.pr {
            let mk = |kind, text| Row::text(INFO, kind, text);
            rows.push(mk(Kind::File, format!("PR #{} · {}", pr.number, pr.title)));
            if !self.info_collapsed {
                for (ii, l) in self.info.iter().enumerate() {
                    match (l.heading, l.md) {
                        // An author line belongs to the body after it, so `C` there quotes it.
                        (true, _) => {
                            let body = self.info.get(ii + 1).filter(|n| n.md).map(|_| ii + 1);
                            rows.push(Row { info: body, ..mk(Kind::Thread, l.text.clone()) });
                        }
                        // GitHub shows a placeholder for an empty description; there's nothing to quote.
                        (false, true) if l.text.trim().is_empty() => rows.extend(self.md_rows("_(no description)_", mk)),
                        (false, true) => rows.extend(self.md_rows(&l.text, mk).into_iter().map(|r| Row { info: Some(ii), ..r })),
                        (false, false) => rows.push(self.info_row(l, mk(Kind::Comment, l.text.clone()))),
                    }
                }
            }
        }
        for (i, f) in self.files.iter().enumerate() {
            let count = |k| f.lines.iter().filter(|l| l.kind == k).count();
            // "dir/" dimmed, the file name in the header color, "(new)" etc. dimmed, +/- colored.
            let (dir, name) = f.path.rsplit_once('/').map_or(("", f.path.as_str()), |(d, n)| (d, n));
            let dir = if dir.is_empty() { String::new() } else { format!("{dir}/") };
            let meta = if f.meta.is_empty() { String::new() } else { format!(" ({})", f.meta) };
            let dim = Some(self.ui.stale);
            let parts = [
                (dir, dim),
                (name.to_owned(), None),
                (meta, dim),
                ("  ".into(), None),
                (format!("+{}", count(Kind::Add)), Some(self.ui.plus)),
                (" ".into(), None),
                (format!("−{}", count(Kind::Del)), Some(self.ui.minus)),
            ];
            let text: String = parts.iter().map(|p| p.0.as_str()).collect();
            let syn = parts.iter().flat_map(|(t, st)| t.chars().map(move |_| *st)).collect();
            rows.push(Row { syn, ..Row::text(i, Kind::File, text) });
            if self.collapsed.contains(&f.path) {
                continue;
            }
            // (thread, index into f.lines it sits under); unplaceable ones go under the file header.
            let placed: Vec<(usize, Option<usize>)> = (0..self.threads.len())
                .filter(|&t| self.threads[t].path == f.path)
                .map(|t| (t, anchor_line(&self.threads[t], &f.lines)))
                .collect();
            for &(t, _) in placed.iter().filter(|p| p.1.is_none()) {
                rows.extend(self.thread_rows(t, i));
            }
            for (li, l) in f.lines.iter().enumerate() {
                rows.push(Row::new(i, l.clone()));
                for &(t, _) in placed.iter().filter(|p| p.1 == Some(li)) {
                    rows.extend(self.thread_rows(t, i));
                }
            }
        }
        if rows.is_empty() {
            rows.push(Row::text(usize::MAX, Kind::Hunk, "(no changes)".into()));
        }
        self.rows = rows;
        self.emph = self.word_spans();
        self.layout();
    }

    /// Pair each run of removed rows with the added rows after it (as split view does) and
    /// find the changed words in each pair.
    fn word_spans(&self) -> HashMap<usize, Vec<(usize, usize)>> {
        let (n, kind) = (self.rows.len(), |i: usize| self.rows[i].line.kind);
        let mut out = HashMap::new();
        let mut i = 0;
        while i < n {
            if kind(i) != Kind::Del {
                i += 1;
                continue;
            }
            let dels = (i..n).take_while(|&j| kind(j) == Kind::Del).count();
            let adds = (i + dels..n).take_while(|&j| kind(j) == Kind::Add).count();
            for k in 0..dels.min(adds) {
                let (d, a) = (i + k, i + dels + k);
                if let Some((x, y)) = diff::word_diff(&self.rows[d].line.text, &self.rows[a].line.text) {
                    out.insert(d, x);
                    out.insert(a, y);
                }
            }
            i += dels + adds;
        }
        out
    }

    /// Map rows to screen rows. In split view, each run of removed lines pairs with the added
    /// lines that follow it; unchanged lines show on both sides; everything else is full width.
    fn layout(&mut self) {
        let n = self.rows.len();
        let kind = |i: usize| self.rows[i].line.kind;
        let mut d = vec![];
        let mut i = 0;
        while i < n {
            match kind(i) {
                _ if !self.split => d.push(DRow::Full(i)),
                Kind::Ctx => d.push(DRow::Pair(Some(i), Some(i))),
                Kind::Del | Kind::Add => {
                    let dels = (i..n).take_while(|&j| kind(j) == Kind::Del).count();
                    let adds = (i + dels..n).take_while(|&j| kind(j) == Kind::Add).count();
                    for k in 0..dels.max(adds) {
                        d.push(DRow::Pair((k < dels).then_some(i + k), (k < adds).then_some(i + dels + k)));
                    }
                    i += dels + adds;
                    continue;
                }
                _ => d.push(DRow::Full(i)),
            }
            i += 1;
        }
        let mut d_of = vec![0; n];
        for (di, dr) in d.iter().enumerate() {
            match *dr {
                DRow::Full(r) => d_of[r] = di,
                DRow::Pair(l, r) => l.into_iter().chain(r).for_each(|x| d_of[x] = di),
            }
        }
        (self.drows, self.d_of) = (d, d_of);
    }

    /// The row to put the cursor on for screen row `d`, preferring the current side.
    fn pick(&self, d: usize) -> usize {
        match self.drows[d] {
            DRow::Full(r) => r,
            DRow::Pair(l, r) => if self.right { r.or(l) } else { l.or(r) }.unwrap_or(0),
        }
    }

    fn toggle_split(&mut self) {
        self.split = !self.split;
        self.layout();
        self.msg = if self.split { "split view (Ctrl-h / Ctrl-l switch sides)" } else { "unified view" }.into();
    }

    /// Screen row of the cursor.
    fn hd(&self) -> usize {
        self.d_of[self.head.row]
    }

    /// Wrap markdown into comment rows, highlighted.
    fn md_rows(&self, text: &str, mk: impl Fn(Kind, String) -> Row) -> Vec<Row> {
        let key = format!("{}\0{text}", self.wrap_w);
        let lines = self
            .md_cache
            .borrow_mut()
            .entry(key)
            .or_insert_with(|| crate::md::render(text, self.wrap_w, &self.ui.md, &self.hl, self.repo_url()))
            .clone();
        lines.into_iter().map(|l| Row { syn: l.styles, links: l.links, ..mk(Kind::Comment, l.text) }).collect()
    }

    /// Color a PR info line by its marks: badges, pass/fail colors, links.
    fn info_row(&self, l: &gh::InfoLine, mut row: Row) -> Row {
        let ui = &self.ui;
        row.syn = vec![None; l.text.chars().count()];
        for (a, b, m) in &l.marks {
            let st = match m {
                gh::Mark::Chip(hex) => chip(hex),
                gh::Mark::Pass => ui.plus,
                gh::Mark::Fail => ui.error,
                gh::Mark::Pending => ui.warning,
                gh::Mark::Dim => ui.stale,
                gh::Mark::Link(url) => {
                    row.links.push((*a, *b, url.clone()));
                    ui.md.link
                }
            };
            for s in row.syn.iter_mut().take(*b).skip(*a) {
                *s = Some(st);
            }
        }
        row
    }

    /// The file's GitHub "Viewed" state, drawn at the right end of its header.
    fn viewed_tag(&self, file: usize) -> Option<(&'static str, Style)> {
        match self.viewed.get(&self.files.get(file)?.path)?.as_str() {
            "VIEWED" => Some(("✓ Viewed ", self.ui.plus)),
            _ => Some(("↻ changed since viewed ", self.ui.warning)),
        }
    }

    /// https://github.com/owner/repo, for `#123` links.
    fn repo_url(&self) -> Option<&str> {
        self.pr.as_ref().and_then(|p| p.url.rsplit_once("/pull/")).map(|(r, _)| r)
    }

    fn root_id(&self, t: usize) -> Option<u64> {
        Some(self.threads.get(t)?.comments.first()?.id)
    }

    fn thread_rows(&self, ti: usize, file: usize) -> Vec<Row> {
        let t = &self.threads[ti];
        let open = self.thread_open(ti);
        let range = match (t.start_line, t.line.or(t.original_line)) {
            (Some(s), Some(e)) if s != e => format!(" · L{s}-{e}"),
            (_, Some(e)) if t.outdated => format!(" · was L{e}"),
            (_, Some(e)) => format!(" · L{e}"),
            _ => String::new(),
        };
        // Tags lead the header so stale threads are easy to spot when scanning.
        let first_pending = t.comments.first().is_some_and(|c| c.pending);
        let tags: Vec<(&str, Style)> = [
            (first_pending, "[pending] ", self.ui.pending),
            (t.outdated, "[outdated] ", self.ui.warning),
            (t.resolved, "[resolved] ", self.ui.resolved),
        ]
            .into_iter()
            .filter_map(|(on, s, c)| on.then_some((s, c)))
            .collect();
        let tag_text: String = tags.iter().map(|t| t.0).collect();
        let tag_syn: Vec<Option<Style>> =
            tags.iter().flat_map(|&(s, st)| s.chars().map(move |_| Some(st))).collect();
        let mk = |kind, text| Row { thread: Some(ti), ..Row::text(file, kind, text) };
        let mut out = vec![];
        for (k, c) in t.comments.iter().enumerate() {
            if k == 0 {
                let mut head = format!("{tag_text}{} · {}{range}", c.author, gh::ago(&c.created));
                if !open {
                    head += &format!(" · {} comment(s), Tab to expand", t.comments.len());
                }
                out.push(Row { syn: tag_syn.clone(), ..mk(Kind::Thread, head) });
                if !open {
                    break;
                }
                // Outdated: the code may have moved or changed, so show what was commented on.
                if t.outdated {
                    let dim = Some(self.ui.stale);
                    out.extend(t.hunk.iter().map(|h| Row { syn: vec![dim; h.chars().count()], ..mk(Kind::Comment, h.clone()) }));
                }
            } else {
                let tag = if c.pending { "[pending] " } else { "" };
                let syn = vec![Some(self.ui.pending); tag.chars().count()];
                out.push(Row { syn, ..mk(Kind::Thread, format!("{tag}{} · {}", c.author, gh::ago(&c.created))) });
            }
            let rows = self.md_rows(&c.body, mk);
            match t.stale() {
                // Resolved/outdated: rendered the same, but dimmed throughout (links still open).
                true => out.extend(rows.into_iter().map(|mut r| {
                    r.syn = vec![Some(self.ui.stale); r.line.text.chars().count()];
                    r
                })),
                false => out.extend(rows),
            }
        }
        out
    }

    /// Stale threads start folded; `Tab` flips a thread from its default.
    fn thread_open(&self, ti: usize) -> bool {
        self.threads[ti].stale() == self.root_id(ti).is_some_and(|id| self.toggled.contains(&id))
    }

    fn toggle_thread(&mut self) {
        let Some(ti) = self.rows[self.head.row].thread else { return };
        let Some(id) = self.root_id(ti) else { return };
        flip(&mut self.toggled, id);
        self.rebuild_at(|r| r.thread == Some(ti));
    }

    /// Rebuild rows and put the cursor on the first row matching `at` (else the top).
    fn rebuild_at(&mut self, at: impl Fn(&Row) -> bool) {
        self.build();
        self.select = false;
        let r = self.rows.iter().position(at).unwrap_or(0);
        self.goto(r, 0);
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

    fn len(&self, r: usize) -> usize {
        self.rows[r].line.text.chars().count()
    }

    fn chars(&self, r: usize) -> Vec<char> {
        self.rows[r].line.text.chars().collect()
    }

    /// `+`/`-`: more or less context around the hunk under the cursor (on a header: the next one).
    fn expand(&mut self, d: i64) {
        let row = &self.rows[self.head.row];
        let Some(f) = self.files.get(row.file) else { return };
        let path = f.path.clone();
        if d < 0 && self.whole.remove(&path) {
            return self.reshape();
        }
        let code = self.rows[self.head.row..]
            .iter()
            .take_while(|r| r.file == row.file)
            .find(|r| matches!(r.line.kind, Kind::Ctx | Kind::Add | Kind::Del));
        let Some(idx) = code.and_then(|c| f.full.iter().position(|x| x.old == c.line.old && x.new == c.line.new)) else {
            return;
        };
        let e = |a| *self.extra.get(&(path.clone(), a)).unwrap_or(&0);
        let ws = diff::windows(&f.full, DEFAULT_CONTEXT as usize, e, self.whole.contains(&path));
        let Some((_, _, hunks)) = ws.into_iter().find(|w| w.0 <= idx && idx <= w.1) else { return };
        for a in hunks {
            let x = self.extra.entry((path.clone(), a)).or_insert(0);
            *x = (*x + d).max(-(DEFAULT_CONTEXT as i64));
        }
        self.reshape();
    }

    fn toggle_whole(&mut self) {
        let Some(p) = self.file_path().map(str::to_owned) else { return };
        flip(&mut self.whole, p);
        self.reshape();
    }

    fn reshape(&mut self) {
        let keep = self.cursor_key();
        self.shape();
        self.rebuild(keep);
    }

    fn toggle_collapse(&mut self) {
        let fi = self.rows[self.head.row].file;
        if fi == INFO {
            self.info_collapsed = !self.info_collapsed;
            return self.rebuild_at(|_| true);
        }
        let Some(p) = self.files.get(fi).map(|f| f.path.clone()) else { return };
        flip(&mut self.collapsed, p);
        self.rebuild_at(|r| r.file == fi);
    }

    /// Fold every file if any is unfolded, else unfold them all. The cursor goes to its file's header.
    fn toggle_collapse_all(&mut self) {
        let fi = self.rows[self.head.row].file;
        if self.files.iter().any(|f| !self.collapsed.contains(&f.path)) {
            self.collapsed = self.files.iter().map(|f| f.path.clone()).collect();
        } else {
            self.collapsed.clear();
        }
        self.rebuild_at(|r| r.file == fi);
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

    /// Rows on screen, each once (a context line shows on both sides but counts once).
    fn visible_rows(&self) -> Vec<usize> {
        let mut out = vec![];
        for d in &self.drows[self.top.min(self.drows.len())..(self.top + self.height).min(self.drows.len())] {
            match *d {
                DRow::Full(r) => out.push(r),
                DRow::Pair(l, r) => out.extend(l.into_iter().chain(r.filter(|&r| Some(r) != l))),
            }
        }
        out
    }

    /// Which highlighted file a code row's colors come from: the base for removed lines, else the working tree.
    fn hl_key(&self, r: usize) -> Option<(String, bool)> {
        let row = &self.rows[r];
        let f = self.files.get(row.file).filter(|f| f.meta != "binary")?;
        match row.line.kind {
            Kind::Del => Some((f.old_path.clone(), false)),
            Kind::Add | Kind::Ctx => Some((f.path.clone(), true)),
            _ => None,
        }
    }
}


/// Write a template, open $VISUAL/$EDITOR (default hx), and return the body file, or None if left empty.
/// `None` if cancelled: the editor failed (e.g. `:cq`), or the text is empty and `allow_empty` isn't set.
fn compose(what: &str, allow_empty: bool) -> Result<Option<String>> {
    compose_with(what, allow_empty, "")
}

fn in_zellij() -> bool {
    std::env::var_os("ZELLIJ").is_some()
}

/// Run an interactive program and wait for it: in a floating pane inside zellij (its exit
/// status comes through), otherwise in this terminal (the caller suspends the TUI).
fn interactive(prog: &str, args: &[String], name: &str) -> std::io::Result<std::process::ExitStatus> {
    if !in_zellij() {
        return std::process::Command::new(prog).args(args).status();
    }
    let cwd = std::env::current_dir()?;
    std::process::Command::new("zellij")
        .args(["run", "--floating", "--close-on-exit", "--block-until-exit", "--name", name])
        .args(["--width", "90%", "--height", "90%", "-x", "5%", "-y", "5%"])
        .arg("--cwd")
        .arg(cwd)
        .arg("--")
        .arg(prog)
        .args(args)
        .status()
}

/// Like `compose`, with `prefill` above the cursor (e.g. a quote). Leaving only the prefill counts as empty.
fn compose_with(what: &str, allow_empty: bool, prefill: &str) -> Result<Option<String>> {
    const MARK: &str = "<!-- dv:";
    let path = std::env::temp_dir().join(format!("dv-comment-{}.md", std::process::id()));
    let hint = if allow_empty { "leave empty for none; :cq to cancel" } else { "leave empty to cancel" };
    std::fs::write(&path, format!("{prefill}\n{MARK} {what}. Save and quit to post; {hint}. -->\n"))?;
    let editor = std::env::var("VISUAL").or_else(|_| std::env::var("EDITOR")).unwrap_or_else(|_| "hx".into());
    let mut parts = editor.split_whitespace();
    let prog = parts.next().unwrap_or("hx");
    // hx takes `file:line`: start on the prefill's first empty line (after a quote, or above a
    // suggestion block, where the explanation goes), else after it.
    let line = prefill.lines().position(str::is_empty).unwrap_or(prefill.lines().count()) + 1;
    let arg = match Path::new(prog).file_name().is_some_and(|n| n == "hx") {
        true => format!("{}:{line}", path.display()),
        false => path.display().to_string(),
    };
    let args: Vec<String> = parts.map(str::to_owned).chain([arg]).collect();
    let st = interactive(prog, &args, &format!("dv: {what}"))?;
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    _ = std::fs::remove_file(&path);
    let body = clean_body(&text, MARK, prefill);
    Ok((st.success() && (allow_empty || !body.is_empty())).then_some(body))
}

/// Drop dv's hint line and trim; untouched prefill alone counts as nothing written.
fn clean_body(text: &str, mark: &str, prefill: &str) -> String {
    let body: Vec<&str> = text.lines().filter(|l| !l.starts_with(mark)).collect();
    let body = body.join("\n").trim().to_owned();
    if !prefill.is_empty() && body == prefill.trim() { String::new() } else { body }
}

/// If posting failed, save what was written so it isn't lost, and say where.
fn keep_on_error(res: Result<()>, body: &str) -> Result<()> {
    res.map_err(|e| {
        let saved = std::env::temp_dir().join(format!("dv-unsent-{}.md", std::process::id()));
        match std::fs::write(&saved, body) {
            Ok(()) if !body.is_empty() => e.context(format!("text saved to {}", saved.display())),
            _ => e,
        }
    })
}

fn posted_msg(review: bool) -> String {
    if review { "added to pending review (S to submit)" } else { "posted" }.into()
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

/// `text` as a markdown quote (`> ` lines), followed by a blank line to write under.
/// Add `x` to `s`, or remove it if it's there.
fn flip<T: std::hash::Hash + Eq>(s: &mut HashSet<T>, x: T) {
    if !s.remove(&x) {
        s.insert(x);
    }
}

fn quote(text: &str) -> String {
    let q: Vec<String> = text.lines().map(|l| format!("> {l}").trim_end().to_owned()).collect();
    format!("{}\n\n", q.join("\n"))
}

/// The first line of a comment that isn't a quote or a fence: what was actually written.
fn gist(body: &str) -> &str {
    body.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with('>') && !l.starts_with("```")).unwrap_or("")
}

/// A GitHub-style badge: the given background (hex, no '#'), black or white text for contrast.
pub(crate) fn chip(hex: &str) -> Style {
    let n = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0x6e7681);
    let (r, g, b) = ((n >> 16) as u8, (n >> 8) as u8, n as u8);
    let light = 0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32 > 150.0;
    let fg = if light { Color::Black } else { Color::White };
    Style::new().bg(Color::Rgb(r, g, b)).fg(fg).add_modifier(ratatui::style::Modifier::BOLD)
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

/// How far (in lines) an outdated thread may land from its original line number.
const OUTDATED_SLACK: i64 = 10;

/// The diff line a thread belongs under. Current threads use their line. Outdated ones are found
/// by the commented line's text when it's distinctive, else by their original line number.
fn anchor_line(t: &Thread, lines: &[DLine]) -> Option<usize> {
    let side = |l: &DLine| match t.left {
        true => matches!(l.kind, Kind::Ctx | Kind::Del),
        false => matches!(l.kind, Kind::Ctx | Kind::Add),
    };
    let no = |l: &DLine| if t.left { l.old } else { l.new };
    if !t.outdated {
        let n = t.line?;
        return lines.iter().position(|l| side(l) && no(l) == Some(n));
    }
    let want = t.original_line? as i64;
    let dist = |i: usize| (no(&lines[i]).unwrap_or(0) as i64 - want).abs();
    let target = t.hunk.last().and_then(|l| l.get(1..)).unwrap_or("").trim();
    let distinctive = target.chars().filter(|c| c.is_alphanumeric()).count() >= 3;
    let by_text = (0..lines.len())
        .filter(|&i| distinctive && side(&lines[i]) && lines[i].text.trim() == target)
        .min_by_key(|&i| dist(i));
    by_text.or_else(|| (0..lines.len()).filter(|&i| side(&lines[i])).min_by_key(|&i| dist(i)).filter(|&i| dist(i) <= OUTDATED_SLACK))
}

/// First row of a run of added/removed lines.
fn change_start(rows: &[Row], r: usize) -> bool {
    let is = |r: usize| matches!(rows[r].line.kind, Kind::Add | Kind::Del);
    is(r) && (r == 0 || !is(r - 1))
}

/// Copy to the system clipboard via OSC 52 (handled by the terminal; works over SSH).
fn clipboard(t: &str) {
    let mut out = std::io::stdout();
    let _ = write!(out, "\x1b]52;c;{}\x07", base64(t.as_bytes()));
    let _ = out.flush();
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
