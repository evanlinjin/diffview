//! Diff view state, Helix-style keys, and rendering.
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

const HELP: &str = "
MOVE
  h j k l, arrows      move (counts: 5j)
  w b e  W B E         word motions (select)
  gh gl gs             line start / end / first non-blank
  f t F T <c>          find char
  gw                   jump labels
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
  Enter                open hx at cursor

RUST-ANALYZER
  gd gr gy gi          definition / references / type / impl
  Space k              hover

PR
  c                    comment on selection / reply on a thread (pending review)
  C                    same, but post immediately
  s                    suggest a change to the selected lines (pending review)
  S                    submit review: comment / approve / request changes, or discard
  Space r              resolve / unresolve thread
  Space e              edit your comment under the cursor

GENERAL
  ?                    this help
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
}

type CursorKey = (Option<(String, Option<u32>, Option<u32>)>, Option<u64>);

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
    /// Rewrite one of your comments (node id, current body).
    Edit { comment: String, body: String, author: String },
    /// Submit the pending review: COMMENT, APPROVE or REQUEST_CHANGES.
    Submit(&'static str),
}

/// Network work that doesn't need the terminal; run after a frame shows its "…" message.
enum Job {
    Refresh,
    Resolve(String, bool),
    Discard(String),
}

enum Target {
    Row(usize),
    Loc(Loc),
    Submit(&'static str),
    /// Asks for confirmation first; `DiscardYes` does it.
    Discard,
    DiscardYes,
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
    md_cache: std::cell::RefCell<HashMap<String, Colors>>,
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
    popup: Option<(&'static str, Vec<Line<'static>>, u16)>,
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
            if event::poll(Duration::from_millis(100))? {
                if let Event::Key(k) = event::read()? {
                    if k.kind == KeyEventKind::Press {
                        self.key(k);
                    }
                }
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

    // ---- data ----

    fn perform(&mut self, a: Action) -> Result<String> {
        match a {
            Action::Open(target) => {
                interactive("hx", &[target.clone()], &format!("hx {target}")).context("running hx")?;
                Ok(String::new())
            }
            Action::Comment { anchor, review, prefill } => {
                let range = match anchor.start {
                    Some((s, _)) => format!("{s}-{}", anchor.line),
                    None => anchor.line.to_string(),
                };
                let kind = if prefill.is_empty() { "comment" } else { "suggestion" };
                let what = format!("{kind} on {}:{range}", anchor.path);
                let Some(body) = compose_with(&what, false, &prefill)? else { return Ok(format!("{kind} cancelled")) };
                let res = match review {
                    true => self.ensure_review().and_then(|r| gh::add_thread(&r, &anchor, &body)),
                    false => gh::comment(self.pr.as_ref().context("no PR")?, &anchor, &body),
                };
                keep_on_error(res, &body).map(|_| posted_msg(review))
            }
            Action::Reply { thread, comment, review, quote } => {
                let Some(body) = compose_with("reply", false, &quote)? else { return Ok("reply cancelled".into()) };
                let res = match review {
                    true => self.ensure_review().and_then(|r| gh::add_reply(&r, &thread, &body)),
                    false => gh::reply(self.pr.as_ref().context("no PR")?, comment, &body),
                };
                keep_on_error(res, &body).map(|_| posted_msg(review))
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

    /// The pending review to add to, starting one on the PR head if there isn't one yet.
    fn ensure_review(&mut self) -> Result<String> {
        if let Some(r) = &self.review {
            return Ok(r.clone());
        }
        let pr = self.pr.as_ref().context("no PR")?;
        let id = gh::start_review(&pr.node_id, &pr.head_oid)?;
        self.review = Some(id.clone());
        Ok(id)
    }

    fn run_job(&mut self, job: Job) {
        let res = match job {
            Job::Refresh => Ok("refreshed"),
            Job::Resolve(t, on) => gh::set_resolved(&t, on).map(|_| if on { "thread resolved" } else { "thread unresolved" }),
            Job::Discard(r) => gh::delete_review(&r).map(|_| {
                self.review = None;
                "pending review discarded"
            }),
        };
        self.hl_cache.retain(|k, _| !k.1);
        let reload = self.reload(true);
        self.msg = match (res, reload) {
            (Err(e), _) | (_, Err(e)) => format!("{e:#}"),
            (Ok(m), Ok(())) => m.into(),
        };
    }

    fn pending_count(&self) -> usize {
        self.threads.iter().flat_map(|t| &t.comments).filter(|c| c.pending).count()
    }

    fn review_menu(&mut self) {
        if self.pr.is_none() {
            self.msg = "no PR for this branch".into();
            return;
        }
        let item = |label: &str, target| Item { label: label.into(), target };
        let mut items = vec![
            item("Comment", Target::Submit("COMMENT")),
            item("Approve", Target::Submit("APPROVE")),
            item("Request changes", Target::Submit("REQUEST_CHANGES")),
        ];
        if self.review.is_some() {
            let n = self.pending_count();
            items.push(item(&format!("Discard pending review ({n} comment(s))"), Target::Discard));
        }
        self.picker = Some(Picker { title: " submit review ", items, query: String::new(), sel: 0 });
    }

    fn start_edit(&mut self) {
        let Some(c) = self.rows[self.head.row].thread.and_then(|t| self.comment_under_cursor(t)) else {
            self.msg = "move onto a comment to edit it".into();
            return;
        };
        match c.can_edit {
            true => {
                let (comment, body, author) = (c.node_id.clone(), c.body.clone(), c.author.clone());
                self.action = Some(Action::Edit { comment, body, author });
            }
            false => self.msg = format!("you can't edit @{}'s comment", c.author),
        }
    }

    fn toggle_resolved(&mut self) {
        match self.rows[self.head.row].thread.map(|t| &self.threads[t]) {
            Some(t) => {
                self.msg = if t.resolved { "unresolving…" } else { "resolving…" }.into();
                self.job = Some(Job::Resolve(t.id.clone(), !t.resolved));
            }
            None => self.msg = "move onto a comment thread".into(),
        }
    }

    /// Fetch review threads and PR info (in parallel: each is a ~1s GitHub round-trip).
    fn load_threads(&mut self) {
        let Some(n) = self.pr.as_ref().map(|p| p.number) else { return };
        let (threads, info) = std::thread::scope(|s| {
            let info = s.spawn(|| gh::info(n));
            (gh::threads(n), info.join())
        });
        match threads {
            Ok(r) => {
                (self.threads, self.review) = (r.threads, r.pending);
                if let Some(pr) = &mut self.pr {
                    pr.node_id = r.pr_id;
                }
            }
            Err(e) => self.msg = format!("loading comments: {e:#}"),
        }
        match info {
            Ok(Ok((lines, title, head))) => {
                self.info = lines;
                if let Some(pr) = &mut self.pr {
                    (pr.title, pr.head_oid) = (title, head);
                }
            }
            Ok(Err(e)) => self.msg = format!("loading PR info: {e:#}"),
            Err(_) => self.msg = "loading PR info panicked".into(),
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
            let mk = |kind, text| Row { file: INFO, line: DLine { kind, old: None, new: None, text }, thread: None, syn: vec![] };
            rows.push(mk(Kind::File, format!("PR #{} · {}", pr.number, pr.title)));
            if !self.info_collapsed {
                for l in &self.info {
                    match l.heading {
                        true => rows.push(mk(Kind::Thread, l.text.clone())),
                        false => rows.extend(self.md_rows(&l.text, mk)),
                    }
                }
            }
        }
        for (i, f) in self.files.iter().enumerate() {
            let count = |k| f.lines.iter().filter(|l| l.kind == k).count();
            let meta = if f.meta.is_empty() { String::new() } else { format!(" ({})", f.meta) };
            let text = format!("{}{meta}  +{} −{}", f.path, count(Kind::Add), count(Kind::Del));
            rows.push(Row { file: i, line: DLine { kind: Kind::File, old: None, new: None, text }, thread: None, syn: vec![] });
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
                rows.push(Row { file: i, line: l.clone(), thread: None, syn: vec![] });
                for &(t, _) in placed.iter().filter(|p| p.1 == Some(li)) {
                    rows.extend(self.thread_rows(t, i));
                }
            }
        }
        if rows.is_empty() {
            let text = "(no changes)".into();
            rows.push(Row { file: usize::MAX, line: DLine { kind: Kind::Hunk, old: None, new: None, text }, thread: None, syn: vec![] });
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

    /// Ctrl-h / Ctrl-l: move to the old / new side of the current split row.
    fn side(&mut self, right: bool) {
        self.right = right;
        if let DRow::Pair(l, r) = self.drows[self.d_of[self.head.row]] {
            if let Some(row) = if right { r } else { l } {
                let col = self.head.col;
                self.goto(row, col);
            }
        }
    }

    /// Screen row of the cursor.
    fn hd(&self) -> usize {
        self.d_of[self.head.row]
    }

    /// Wrap markdown into comment rows, highlighted.
    fn md_rows(&self, text: &str, mk: impl Fn(Kind, String) -> Row) -> Vec<Row> {
        let lines = wrap(text, self.wrap_w);
        let joined = lines.join("\n");
        let colors = self.md_cache.borrow_mut().entry(joined).or_insert_with_key(|t| self.hl.markdown(t)).clone();
        let mut rows: Vec<Row> = lines.into_iter().map(|l| mk(Kind::Comment, l)).collect();
        for (row, c) in rows.iter_mut().zip(colors) {
            row.syn = c;
        }
        rows
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
        let mk = |kind, text| Row { file, line: DLine { kind, old: None, new: None, text }, thread: Some(ti), syn: vec![] };
        let mut out = vec![];
        for (k, c) in t.comments.iter().enumerate() {
            if k == 0 {
                let mut head = format!("{tag_text}{} · {}{range}", c.author, c.created);
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
                out.push(Row { syn, ..mk(Kind::Thread, format!("{tag}{} · {}", c.author, c.created)) });
            }
            match t.stale() {
                true => out.extend(wrap(&c.body, self.wrap_w).into_iter().map(|l| mk(Kind::Comment, l))),
                false => out.extend(self.md_rows(&c.body, mk)),
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

    /// `c`/`C`: reply when on a thread, else comment on the selected lines.
    fn comment_or_reply(&mut self, review: bool) {
        match self.rows[self.head.row].thread {
            Some(_) => self.start_reply(review),
            None => self.start_comment(review),
        }
    }

    /// Line comments are posted against the PR head; local edits would shift the lines.
    fn check_synced(&self) -> Result<()> {
        let pr = self.pr.as_ref().context("no PR for this branch")?;
        if diff::git(&["rev-parse", "HEAD"])?.trim() != pr.head_oid {
            bail!("HEAD isn't the PR head (pull, or `gh pr checkout {}`)", pr.number);
        }
        if !diff::git(&["status", "--porcelain", "--untracked-files=no"])?.trim().is_empty() {
            bail!("working tree has local changes; stash them to comment");
        }
        Ok(())
    }

    fn start_comment(&mut self, review: bool) {
        match self.check_synced().and_then(|_| self.selection_anchor()) {
            Ok(anchor) => self.action = Some(Action::Comment { anchor, review, prefill: String::new() }),
            Err(e) => self.msg = format!("{e:#}"),
        }
    }

    /// `s`: a GitHub suggestion for the selected new-side lines, pre-filled with their code.
    fn start_suggest(&mut self) {
        match self.check_synced().and_then(|_| self.suggestion()) {
            Ok((anchor, prefill)) => self.action = Some(Action::Comment { anchor, review: true, prefill }),
            Err(e) => self.msg = format!("{e:#}"),
        }
    }

    fn suggestion(&self) -> Result<(Anchor, String)> {
        let (s, e) = self.ordered();
        let rows: Vec<&Row> =
            self.rows[s.row..=e.row].iter().filter(|r| matches!(r.line.kind, Kind::Ctx | Kind::Add | Kind::Del)).collect();
        let (Some(first), Some(last)) = (rows.first(), rows.last()) else { bail!("select the lines to suggest a change for") };
        if rows.iter().any(|r| r.line.kind == Kind::Del) {
            bail!("suggestions replace lines in the new version; leave removed lines out of the selection");
        }
        if first.file != last.file {
            bail!("a suggestion can't span files");
        }
        let (Some(a), Some(b)) = (first.line.new, last.line.new) else { bail!("no line number") };
        let code: Vec<&str> = rows.iter().map(|r| r.line.text.as_str()).collect();
        let code = code.join("\n");
        // A fence longer than any backtick run in the code, so the block can't end early.
        let mut fence = String::from("```");
        while code.contains(&fence) {
            fence.push('`');
        }
        let anchor =
            Anchor { path: self.files[first.file].path.clone(), line: b, left: false, start: (a != b).then_some((a, false)) };
        Ok((anchor, format!("\n{fence}suggestion\n{code}\n{fence}\n")))
    }

    fn start_reply(&mut self, review: bool) {
        let t = self.rows[self.head.row].thread;
        match (t, t.and_then(|t| self.root_id(t))) {
            (Some(t), Some(comment)) if self.pr.is_some() => {
                let thread = self.threads[t].id.clone();
                let quote = self.comment_under_cursor(t).map_or(String::new(), |c| {
                    let q: Vec<String> = c.body.lines().map(|l| format!("> {l}").trim_end().to_owned()).collect();
                    format!("{}\n\n", q.join("\n"))
                });
                self.action = Some(Action::Reply { thread, comment, review, quote });
            }
            _ => self.msg = "move onto a comment thread to reply".into(),
        }
    }

    /// Which comment of thread `t` the cursor is in: each comment starts with an author row.
    fn comment_under_cursor(&self, t: usize) -> Option<&gh::Comment> {
        let start = self.rows[..=self.head.row].iter().rposition(|r| r.thread != Some(t)).map_or(0, |i| i + 1);
        let heads = self.rows[start..=self.head.row].iter().filter(|r| r.line.kind == Kind::Thread).count();
        self.threads[t].comments.get(heads.saturating_sub(1))
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
        match self.rows[row].line.kind {
            Kind::Del => self.right = false,
            Kind::Add => self.right = true,
            _ => {}
        }
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
        let last = self.drows.len() as isize - 1;
        let w = self.want_col;
        let target = (self.hd() as isize + d).clamp(0, last) as usize;
        self.goto(self.pick(target), w);
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
        clipboard(&t);
        self.msg = format!("copied {} line(s)", t.lines().count().max(1));
    }

    /// `path:line` or `path:start-end` for the selection (new-side numbering), for pasting elsewhere.
    fn file_ref(&self) -> Option<String> {
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

    fn copy_ref(&mut self) {
        match self.file_ref() {
            Some(r) => {
                clipboard(&r);
                self.msg = format!("copied {r}");
            }
            None => self.msg = "not in a file".into(),
        }
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
        if !self.whole.remove(&p) {
            self.whole.insert(p);
        }
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

    /// Fold every file if any is unfolded, else unfold them all. The cursor goes to its file's header.
    fn toggle_collapse_all(&mut self) {
        let fi = self.rows[self.head.row].file;
        if self.files.iter().any(|f| !self.collapsed.contains(&f.path)) {
            self.collapsed = self.files.iter().map(|f| f.path.clone()).collect();
        } else {
            self.collapsed.clear();
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
        let last = self.drows.len() as isize - 1;
        self.top = (self.top as isize + d).clamp(0, last) as usize;
        let so = 3.min(self.height / 2);
        let (lo, hi) = (self.top + so, (self.top + self.height).saturating_sub(so + 1));
        let hd = self.hd();
        if hd < lo || hd > hi {
            let w = self.want_col;
            self.goto(self.pick(hd.clamp(lo, hi.max(lo)).min(last as usize)), w);
            self.want_col = w;
        }
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

    /// Label every visible word (2+ chars), nearest to the cursor first, alternating forward/back.
    fn show_labels(&mut self) {
        let mut words = vec![];
        for row in self.visible_rows() {
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
                        t => self.popup = Some((" hover  j/k scroll ", self.hover_lines(&t), 0)),
                    },
                    Req::Goto => self.show_locations(lsp::locations(res)),
                }
            }
            _ => {}
        }
    }

    /// The help text, styled: section headings, keys (separators dimmed), descriptions wrapped
    /// with a hanging indent under the description column, `(notes)` dimmed.
    fn help_lines(&self, width: usize) -> Vec<Line<'static>> {
        const KEY_COL: usize = 23; // entries are "  <key, padded to col 23><description>"
        let ui = &self.ui;
        let dim = Style { fg: ui.stale.fg, ..Style::new() };
        let mut out = vec![];
        for l in HELP.trim_matches('\n').lines() {
            if l.is_empty() {
                out.push(Line::default());
                continue;
            }
            if !l.starts_with(' ') {
                out.push(Line::styled(format!(" {l}"), ui.heading.add_modifier(ratatui::style::Modifier::BOLD)));
                continue;
            }
            let (key, desc) = l.split_at(KEY_COL.min(l.len()));
            let mut spans: Vec<Span> = vec![Span::raw("  ")];
            for c in key[2..].chars() {
                let st = if c == ',' { dim } else { ui.key }; // `/` is a key itself (search)
                spans.push(Span::styled(c.to_string(), st));
            }
            let mut depth = 0; // inside "(...)", which may wrap
            for (i, line) in wrap(desc, width.saturating_sub(KEY_COL).max(10)).into_iter().enumerate() {
                if i > 0 {
                    out.push(Line::from(std::mem::take(&mut spans)));
                    spans.push(Span::raw(" ".repeat(KEY_COL)));
                }
                // Dim parenthesised notes, e.g. "(pending review)".
                for c in line.chars() {
                    depth += (c == '(') as i32;
                    spans.push(Span::styled(c.to_string(), if depth > 0 { dim } else { Style::new() }));
                    depth -= (c == ')') as i32;
                }
            }
            out.push(Line::from(spans));
        }
        out
    }

    /// Hover markdown, highlighted (code fences in their language), with the fence lines dropped.
    fn hover_lines(&self, md: &str) -> Vec<Line<'static>> {
        let colors = self.hl.markdown(md);
        md.lines()
            .zip(colors.into_iter().chain(std::iter::repeat(vec![])))
            .filter(|(l, _)| !l.trim_start().starts_with("```"))
            .map(|(l, c)| {
                let spans: Vec<Span> =
                    l.chars().zip(c.into_iter().chain(std::iter::repeat(None))).map(|(ch, st)| Span::styled(ch.to_string(), st.unwrap_or_default())).collect();
                Line::from(spans)
            })
            .collect()
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
        if let Some((_, _, scroll)) = &mut self.popup {
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
            // Many terminals send Ctrl-h as Backspace.
            KeyCode::Backspace => return self.side(false),
            KeyCode::Tab if self.rows[self.head.row].thread.is_some() => return self.toggle_thread(),
            KeyCode::Tab => return self.toggle_collapse(),
            KeyCode::BackTab => return self.toggle_collapse_all(),
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
                    self.top = (self.top + half as usize).min(self.drows.len() - 1);
                    self.vmove(half);
                }
                'u' => {
                    self.top = self.top.saturating_sub(half as usize);
                    self.vmove(-half);
                }
                'e' => self.scroll(5),
                'w' => self.pending = Some(CTRL_W),
                'h' => self.side(false),
                'l' => self.side(true),
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
            '+' | '=' => self.expand(10),
            '-' => self.expand(-10),
            'Z' => self.toggle_whole(),
            '?' => {
                // Popup text width: popup is min(width - 4, 100) wide, minus padding and scrollbar.
                let cw = (self.width.saturating_sub(4).min(100) as usize).saturating_sub(3);
                self.popup = Some((" keys · j/k scroll · any other key closes ", self.help_lines(cw), 0));
            }
            'c' => self.comment_or_reply(true),
            's' => self.start_suggest(),
            'C' => self.comment_or_reply(false),
            'r' => self.start_reply(true),
            'S' => self.review_menu(),
            'R' => {
                self.job = Some(Job::Refresh);
                self.msg = "refreshing…".into();
            }
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
            ('z', 'z') => self.top = self.hd().saturating_sub(self.height / 2),
            ('z', 't') => self.top = self.hd().saturating_sub(so),
            ('z', 'b') => self.top = (self.hd() + so + 1).saturating_sub(self.height),
            (CTRL_W, 'v') => self.toggle_split(),
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
            (' ', 'c') => self.copy_ref(),
            (' ', 'r') => self.toggle_resolved(),
            (' ', 'e') => self.start_edit(),
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
                        Target::Submit(ev) => self.action = Some(Action::Submit(ev)),
                        Target::Discard => {
                            let n = self.pending_count();
                            let items = vec![
                                Item { label: "No, keep it".into(), target: Target::Nothing },
                                Item { label: format!("Yes, discard {n} pending comment(s)"), target: Target::DiscardYes },
                            ];
                            self.picker = Some(Picker { title: " discard pending review? ", items, query: String::new(), sel: 0 });
                        }
                        Target::DiscardYes => {
                            if let Some(r) = self.review.clone() {
                                self.msg = "discarding…".into();
                                self.job = Some(Job::Discard(r));
                            }
                        }
                        Target::Nothing => {}
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
        self.width = area.width;
        let so = 3.min(h / 2);
        let hd = self.hd();
        if hd < self.top + so {
            self.top = hd.saturating_sub(so);
        }
        if hd + so + 1 > self.top + h {
            self.top = hd + so + 1 - h;
        }

        let maxno = self.rows.iter().filter_map(|r| r.line.old.max(r.line.new)).max().unwrap_or(0);
        let lw = maxno.to_string().len().max(3);
        let gw = lw * 2 + 4;
        // Relative line numbers (Helix `line-number = "relative"`) get their own column on the left.
        let relw = if self.settings.relative { 4 } else { 0 };
        let width = (area.width as usize).saturating_sub(relw);
        let tw = width.saturating_sub(gw).max(1);
        // Split halves: one line-number column each, a 1-col divider between.
        let half = width.saturating_sub(1) / 2;
        let ghw = lw + 3;
        let (htw_l, htw_r) = (half.saturating_sub(ghw).max(1), width.saturating_sub(1 + half).saturating_sub(ghw).max(1));
        let ww = tw.saturating_sub(1).clamp(40, 100);
        if ww != self.wrap_w {
            self.wrap_w = ww;
            let keep = self.cursor_key();
            self.rebuild(keep);
        }
        if self.head.col < self.left {
            self.left = self.head.col;
        }
        let cur_w = match self.drows[self.hd()] {
            DRow::Pair(..) => htw_l.min(htw_r),
            DRow::Full(_) => tw,
        };
        if self.head.col >= self.left + cur_w {
            self.left = self.head.col + 1 - cur_w;
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
        for r in self.visible_rows() {
            if let Some(key) = self.hl_key(r).filter(|k| !self.hl_cache.contains_key(k)) {
                let text = match key.1 {
                    true => std::fs::read_to_string(&key.0).unwrap_or_default(),
                    false => diff::git(&["show", &format!("{}:{}", self.base, key.0)]).unwrap_or_default(),
                };
                // ponytail: whole file highlighted synchronously on first view; a thread if big PRs stutter.
                let colors = if text.len() > HL_MAX_BYTES { vec![] } else { self.hl.file(&key.0, &text) };
                self.hl_cache.insert(key, colors);
            }
        }
        let divider = Span::styled("│", self.ui.linenr);
        let hd = self.hd();
        let lines: Vec<Line> = (self.top..(self.top + h).min(self.drows.len()))
            .map(|d| {
                let cur = d == hd;
                let rel = match (relw, cur) {
                    (0, _) => vec![],
                    (_, true) => vec![Span::styled(format!("{:>w$} ", "", w = relw - 1), self.ui.linenr_selected)],
                    _ => vec![Span::styled(format!("{:>w$} ", d.abs_diff(hd), w = relw - 1), self.ui.linenr)],
                };
                let mut spans = rel;
                match self.drows[d] {
                    DRow::Full(r) => spans.extend(self.render_row(r, s, e, (lw, gw, tw), None, cur, &overlay)),
                    DRow::Pair(l, rr) => {
                        let half = |row: Option<usize>, right: bool, w: usize| match row {
                            Some(row) => self.render_row(row, s, e, (lw, ghw, w), Some(right), cur, &overlay),
                            None => vec![Span::raw(" ".repeat(ghw + w))],
                        };
                        spans.extend(half(l, false, htw_l));
                        spans.push(divider.clone());
                        spans.extend(half(rr, true, htw_r));
                    }
                }
                Line::from(spans)
            })
            .collect();
        f.render_widget(Block::new().style(self.ui.base), area);
        f.render_widget(Paragraph::new(lines), Rect { height: h as u16, ..area });

        let status = Rect { y: area.y + h as u16, height: 1, ..area };
        let left = match &self.prompt {
            Some(q) => Line::from(format!("/{q}")),
            None => {
                let l = &self.rows[self.head.row].line;
                let pos = match (l.new, l.old) {
                    (Some(n), _) => format!("{n}:{}", self.head.col + 1),
                    (None, Some(o)) => format!("-{o}:{}", self.head.col + 1),
                    _ => String::new(),
                };
                let (mode, mst) = if self.select { (" SEL ", self.ui.mode_select) } else { (" NOR ", self.ui.mode_normal) };
                Line::from(vec![
                    Span::styled(mode, mst),
                    Span::raw(format!("  {}  {pos}  {}", self.file_path().unwrap_or(""), self.msg)),
                ])
            }
        };
        let pend = match self.pending {
            Some(CTRL_W) => "C-w".into(),
            p => p.map(String::from).unwrap_or_default(),
        };
        let count = if self.count > 0 { self.count.to_string() } else { String::new() };
        let ra = match self.progress.values().next() {
            Some((t, m)) => format!("ra: {t} {m}  "),
            None if self.lsp.is_some() && !self.lsp_ready => "ra: starting  ".into(),
            None => String::new(),
        };
        let pending = match self.pending_count() {
            0 if self.review.is_none() => String::new(),
            n => format!(" · {n} pending, S to submit"),
        };
        let pr = self.pr.as_ref().map(|p| format!("PR #{} · {} threads{pending}  ", p.number, self.threads.len())).unwrap_or_default();
        let view = if self.split { "split  " } else { "" };
        let right = format!("{ra}{pr}{view}{count}{pend}  {}/{} ", self.head.row + 1, self.rows.len());
        let bar = self.ui.base.patch(self.ui.status);
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
                let st = if i == sel { self.ui.menu_selected } else { Style::new() };
                Line::styled(p.items[it].label.clone(), st)
            }));
            let pos = if m.is_empty() { "0 / 0".into() } else { format!("{} / {}", sel + 1, m.len()) };
            let inner = self.popup_frame(f, r, p.title, &pos);
            f.render_widget(Paragraph::new(lines), inner);
            // Items start below the query line.
            let list = Rect { y: inner.y + 1, height: inner.height.saturating_sub(1), ..inner };
            self.scrollbar(f, list, m.len(), vis, off);
        }

        if let Some((title, text, scroll)) = self.popup.clone() {
            let w = area.width.saturating_sub(4).min(100);
            // ponytail: wrapped height estimated from line widths (char wrap, not word wrap).
            let cw = w.saturating_sub(3).max(1) as usize;
            let total: usize = text.iter().map(|l| l.width().div_ceil(cw).max(1)).sum();
            let h = (total as u16 + 2).min(area.height.saturating_sub(4)).max(3);
            let r = Rect { x: (area.width - w) / 2, y: 2.min(area.height), width: w, height: h.min(area.height) };
            let vis = h.saturating_sub(2) as usize;
            // Don't scroll past the end.
            let scroll = scroll.min(total.saturating_sub(vis) as u16);
            if let Some(p) = &mut self.popup {
                p.2 = scroll;
            }
            let first = scroll as usize + 1;
            let pos = if total > vis { format!("{first}–{} / {total}", (first + vis - 1).min(total)) } else { String::new() };
            let inner = self.popup_frame(f, r, title, &pos);
            let text_area = Rect { width: inner.width.saturating_sub(1), ..inner }; // room for the scrollbar
            f.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }).scroll((scroll, 0)), text_area);
            self.scrollbar(f, inner, total, vis, scroll as usize);
        }
    }

    /// A thin scrollbar on the right edge of `area`, only when `total` items don't fit in `vis`.
    fn scrollbar(&self, f: &mut Frame, area: Rect, total: usize, vis: usize, pos: usize) {
        if total <= vis {
            return;
        }
        let mut state = ScrollbarState::new(total.saturating_sub(vis)).viewport_content_length(vis).position(pos);
        let bar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .track_symbol(Some("│"))
            .track_style(self.ui.linenr)
            .thumb_symbol("┃")
            .thumb_style(self.ui.linenr_selected);
        f.render_stateful_widget(bar, area, &mut state);
    }

    /// Helix-style popup: no border, the theme's popup background, the title as a header row
    /// in the header color. Returns the padded content area.
    fn popup_frame(&self, f: &mut Frame, r: Rect, title: &str, pos: &str) -> Rect {
        f.render_widget(Clear, r);
        f.render_widget(Block::new().style(self.ui.base.patch(self.ui.popup)), r);
        let head = Rect { height: 1.min(r.height), ..r };
        let head_style = self.ui.base.patch(self.ui.header);
        f.render_widget(Paragraph::new(format!(" {}", title.trim())).style(head_style), head);
        // Where you are in the content, on the right of the header.
        f.render_widget(Paragraph::new(format!("{pos} ")).alignment(Alignment::Right).style(head_style), head);
        Rect { x: r.x + 1, y: r.y + 1, width: r.width.saturating_sub(2), height: r.height.saturating_sub(2) }
    }

    /// A hunk header drawn as a quiet rule: `── fn main ─────── L12–18`. The raw `@@` text stays
    /// the row's text (search, `Y`); only the drawing changes.
    fn hunk_divider(&self, r: usize, gw: usize, tw: usize) -> Option<Vec<Span<'static>>> {
        let (start, count, ctx) = diff::hunk_label(&self.rows[r].line.text)?;
        let range = match count {
            0 | 1 => format!(" L{start} "),
            n => format!(" L{start}–{} ", start + n - 1),
        };
        let mut left = String::from("──");
        if !ctx.is_empty() {
            left += &format!(" {ctx} ");
        }
        let fill = tw.saturating_sub(left.chars().count() + range.chars().count() + 1);
        let text = format!("{left}{}{range}─", "─".repeat(fill));
        let mut spans = vec![Span::styled(" ".repeat(gw), self.ui.linenr)];
        let mut chars = text.chars();
        if self.head.row == r {
            let c = chars.next().unwrap_or(' ');
            spans.push(Span::styled(c.to_string(), if self.select { self.ui.cursor_select } else { self.ui.cursor }));
        }
        spans.push(Span::styled(chars.collect::<String>(), self.ui.hunk));
        Some(spans)
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

    /// One row's spans, padded to the gutter + `tw` width. `side`: None = full width, else
    /// Some(false/true) = the old/new half of a split row (one line number, not two).
    #[allow(clippy::too_many_arguments)]
    fn render_row(
        &self, r: usize, s: Pos, e: Pos, (lw, gw, tw): (usize, usize, usize), side: Option<bool>, cursor_row: bool,
        overlay: &HashMap<Pos, char>,
    ) -> Vec<Span<'static>> {
        let row = &self.rows[r];
        let l = &row.line;
        let num = |n: Option<u32>| n.map(|n| n.to_string()).unwrap_or_default();
        let mut line_bg = None;
        let ui = &self.ui;
        let mut gstyle = ui.linenr;
        let (gutter, style) = match l.kind {
            Kind::File => {
                let collapsed = match row.file {
                    INFO => self.info_collapsed,
                    f => self.files.get(f).is_some_and(|f| self.collapsed.contains(&f.path)),
                };
                line_bg = ui.header.bg;
                gstyle = ui.header;
                (format!(" {:<w$}", if collapsed { "▸" } else { "▾" }, w = gw - 1), ui.header)
            }
            Kind::Hunk => {
                if let Some(spans) = self.hunk_divider(r, gw, tw) {
                    return spans;
                }
                (" ".repeat(gw), ui.hunk)
            }
            Kind::Thread | Kind::Comment => {
                let resolved = row.thread.is_some_and(|t| self.threads[t].stale());
                let check = l.text.chars().next().filter(|_| row.file == INFO);
                let st = match (l.kind, resolved, check) {
                    (_, _, Some('✗')) => ui.error,
                    (_, _, Some('●')) => ui.warning,
                    (_, true, _) => ui.stale,
                    (Kind::Thread, _, _) if row.file == INFO => ui.heading,
                    (Kind::Thread, _, _) => ui.thread,
                    _ => Style::new(),
                };
                let first = row.thread.is_some() && (r == 0 || self.rows[r - 1].thread != row.thread);
                let mark = match row.thread {
                    Some(t) if first => {
                        let folded = !self.thread_open(t);
                        if folded { "▸ " } else { "▾ " }
                    }
                    _ => "┃ ",
                };
                // A filled box from the text column on; the gutter keeps the page background.
                line_bg = Some(ui.comment_bg);
                gstyle = gstyle.bg(ui.base.bg.unwrap_or(Color::Reset));
                (format!("{:>gw$}", mark), st)
            }
            k => {
                let m = match k {
                    Kind::Add => {
                        (line_bg, gstyle) = (Some(ui.add_bg), ui.plus);
                        '+'
                    }
                    Kind::Del => {
                        (line_bg, gstyle) = (Some(ui.del_bg), ui.minus);
                        '-'
                    }
                    _ => ' ',
                };
                let gutter = match side {
                    None => format!("{:>lw$} {:>lw$} {m} ", num(l.old), num(l.new)),
                    Some(right) => format!("{:>lw$} {m} ", num(if right { l.new } else { l.old })),
                };
                (gutter, Style::new())
            }
        };
        if cursor_row && self.settings.cursorline {
            if matches!(l.kind, Kind::Ctx | Kind::Add | Kind::Del) {
                gstyle = gstyle.patch(ui.linenr_selected);
            }
            // Like Helix: the cursor's line is tinted. Added/removed lines keep their color, lifted.
            line_bg = match l.kind {
                Kind::Add => Some(ui.add_bg_cur),
                Kind::Del => Some(ui.del_bg_cur),
                Kind::Ctx | Kind::Thread | Kind::Comment => ui.cursorline.or(line_bg),
                _ => line_bg,
            };
        }
        let emph_bg = if l.kind == Kind::Add { ui.emph_add } else { ui.emph_del };
        let emph = self.emph.get(&r);
        let syn = match row.syn.is_empty() {
            false => Some(&row.syn),
            true => self.hl_key(r).and_then(|k| {
                let n = if k.1 { l.new } else { l.old }?;
                self.hl_cache.get(&k)?.get(n as usize - 1)
            }),
        };
        let mut spans = vec![Span::styled(gutter, gstyle)];
        let mut n = 0;
        // A context line shows on both sides of a split row; the cursor goes on the current side.
        let this_side = side.is_none() || l.kind != Kind::Ctx || side == Some(self.right);
        let cursor_here = self.head.row == r && this_side;
        for (i, c) in l.text.chars().enumerate().skip(self.left).take(tw) {
            let p = Pos { row: r, col: i };
            let mut st = style;
            if let Some(c) = syn.and_then(|syn| syn.get(i).copied().flatten()) {
                st = st.patch(c);
            }
            if emph.is_some_and(|e| e.iter().any(|&(a, b)| a <= i && i < b)) {
                st = st.bg(emph_bg);
            }
            if s <= p && p <= e && this_side {
                st = st.patch(ui.selection);
            }
            if p == self.head && cursor_here {
                st = st.patch(if self.select { ui.cursor_select } else { ui.cursor });
            }
            if let Some(&lc) = overlay.get(&p) {
                spans.push(Span::styled(lc.to_string(), ui.jump_label));
                n = i + 1;
                continue;
            }
            // ponytail: tab drawn as 4 cols but scrolled as 1 char; fine unless tab-heavy files.
            spans.push(Span::styled(if c == '\t' { "    ".into() } else { c.to_string() }, st));
            n = i + 1;
        }
        if cursor_here && self.head.col >= n.max(self.left) {
            spans.push(Span::styled(" ", if self.select { ui.cursor_select } else { ui.cursor }));
        }
        // Pad to the full width (so split halves line up), and tint the whole row, so
        // added/removed lines read as bands. Spans with their own bg (the cursor) keep it.
        let used: usize = spans[1..].iter().map(|s| s.content.chars().count()).sum();
        spans.push(Span::raw(" ".repeat(tw.saturating_sub(used))));
        if let Some(bg) = line_bg {
            for sp in &mut spans {
                sp.style = Style::new().bg(bg).patch(sp.style);
            }
        }
        spans
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
    let h = app.hover_lines("```rust\nfn x()\n```\n\ndocs");
    assert_eq!(h.len(), 3); // fences dropped
    assert_eq!(h[0].spans[0].style.fg, Some(Color::Red)); // `fn` highlighted as Rust
    let back = KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT);
    app.key(back);
    assert_eq!(app.rows.len(), 3); // only the three file headers
    app.key(back);
    assert!(app.rows.len() > 3);
    app.alphabet = vec!['a', 's'];
    app.height = 20;
    app.goto(4, 0);
    press(&mut app, "gw"); // nearest word after the cursor ("new_call") gets "aa"
    assert_eq!(app.labels[0].2, ['a', 'a']);
    press(&mut app, "aa");
    assert_eq!(app.selection_text(false), "new_call");
}

#[test]
fn suggestion_block() {
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    // rows: 0 file, 1 hunk, 2 ctx "fn main() {", 3 del, 4 add "    new_call(x);", 5 ctx "}"
    app.goto(4, 0);
    app.select = true;
    app.goto(5, 0);
    let (a, prefill) = app.suggestion().unwrap();
    assert_eq!((a.path.as_str(), a.line, a.left, a.start), ("src/a.rs", 3, false, Some((2, false))));
    assert_eq!(prefill, "\n```suggestion\n    new_call(x);\n}\n```\n");
    // Removed lines can't be part of a suggestion.
    app.goto(3, 0);
    assert!(app.suggestion().err().unwrap().to_string().contains("removed lines"));
    // Code containing ``` gets a longer fence.
    app.rows[4].line.text = "let s = \"```\";".into();
    app.select = false;
    app.goto(4, 0);
    assert!(app.suggestion().unwrap().1.contains("````suggestion\n"));
}

#[test]
fn split_view() {
    let sample = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,4 +1,3 @@\n ctx\n-old1\n-old2\n+new1\n end\n";
    let mut app = App::with("base".into(), diff::parse(sample));
    let ctrl = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL);
    app.key(ctrl('w'));
    app.key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE));
    assert!(app.split);
    // rows: 0 file, 1 hunk, 2 ctx, 3 old1, 4 old2, 5 new1, 6 end
    assert_eq!(app.drows, [
        DRow::Full(0), DRow::Full(1), DRow::Pair(Some(2), Some(2)),
        DRow::Pair(Some(3), Some(5)), DRow::Pair(Some(4), None), DRow::Pair(Some(6), Some(6)),
    ]);
    app.goto(2, 0); // ctx, cursor prefers the right (new) side
    app.vmove(1);
    assert_eq!(app.head.row, 5); // new1, beside old1
    app.key(ctrl('h'));
    assert_eq!(app.head.row, 3); // old1
    app.vmove(1);
    assert_eq!(app.head.row, 4); // stays on the left: old2
    app.key(ctrl('l'));
    assert_eq!(app.head.row, 4); // nothing on the right there; side switches for the next move
    app.vmove(1);
    assert_eq!(app.head.row, 6);
}

#[test]
fn comments_placed_and_anchored() {
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    let c = |id, body: &str| gh::Comment {
        id, node_id: format!("C{id}"), can_edit: id == 2, author: "amy".into(), body: body.into(), created: "2026-10-01".into(), pending: false,
    };
    let t = |left, line, resolved, comments| Thread {
        id: format!("T{line:?}"),
        path: "src/a.rs".into(), left, line, start_line: None, original_line: Some(1), resolved, outdated: line.is_none(),
        hunk: vec![], comments,
    };
    app.threads = vec![
        t(true, Some(2), false, vec![c(1, "why remove?"), c(2, "unused")]), // on the deleted line
        t(false, None, true, vec![c(3, "old note")]),                        // outdated + resolved
    ];
    app.build();
    let kinds: Vec<_> = app.rows.iter().take(10).map(|r| (r.line.kind, r.thread)).collect();
    assert_eq!(kinds, [
        (Kind::File, None),
        (Kind::Hunk, None),
        (Kind::Ctx, None),
        (Kind::Thread, Some(1)), // outdated (was L1) + resolved: folded, under line 1
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
    assert_eq!(app.file_ref().unwrap(), "src/a.rs:2"); // new-side lines only; the deleted one has none

    // An outdated thread is relocated by the commented line's text.
    let moved = Thread { hunk: vec!["+    new_call(x);".into()], ..t(false, None, false, vec![c(4, "nit")]) };
    let lines = &app.files[0].lines;
    assert_eq!(anchor_line(&moved, lines).map(|i| lines[i].kind), Some(Kind::Add));
    // A trivial line ("}") falls back to the original line number (3 → the closing brace's row).
    let brace = Thread { hunk: vec![" }".into()], original_line: Some(3), ..t(false, None, false, vec![c(5, "x")]) };
    assert_eq!(anchor_line(&brace, lines).map(|i| lines[i].new), Some(Some(3)));
    let far = Thread { original_line: Some(500), ..brace };
    assert_eq!(anchor_line(&far, lines), None); // nothing within 10 lines → under the file header

    // Pending replies are tagged; `c` on a thread queues a reply in the review, `C` posts it now.
    app.threads[0].comments[1].pending = true;
    app.build();
    assert!(app.rows.iter().any(|r| r.line.text.starts_with("[pending] amy")));
    app.pr = Some(gh::Pr { number: 1, node_id: "PR".into(), title: "t".into(), base: "main".into(), head_oid: "abc".into() });
    let tr = app.rows.iter().position(|r| r.thread == Some(0)).unwrap();
    app.goto(tr, 0);
    let press = |app: &mut App, c| app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    press(&mut app, 'c');
    assert!(matches!(app.action.take(), Some(Action::Reply { review: true, ref thread, .. }) if thread == "TSome(2)"));
    // On the second comment ("unused"), the reply quotes that comment, not the first.
    let second = app.rows.iter().position(|r| r.line.text == "unused").unwrap();
    app.goto(second, 0);
    press(&mut app, 'c');
    assert!(matches!(app.action.take(), Some(Action::Reply { ref quote, .. }) if quote == "> unused\n\n"));
    // Space e edits the comment under the cursor, if you're allowed to.
    press(&mut app, ' ');
    press(&mut app, 'e');
    assert!(matches!(app.action.take(), Some(Action::Edit { ref comment, ref body, .. }) if comment == "C2" && body == "unused"));
    let first = app.rows.iter().position(|r| r.line.text == "why remove?").unwrap();
    app.goto(first, 0);
    press(&mut app, ' ');
    press(&mut app, 'e');
    assert!(app.action.is_none() && app.msg.contains("can't edit"));
    assert_eq!(clean_body("> unused\n\n<!-- dv: x -->\n", "<!-- dv:", "> unused\n\n"), ""); // untouched → cancel
    assert_eq!(clean_body("> unused\n\nagreed\n<!-- dv: x -->", "<!-- dv:", "> unused\n\n"), "> unused\n\nagreed");
    app.goto(tr, 0);
    press(&mut app, 'C');
    assert!(matches!(app.action.take(), Some(Action::Reply { review: false, .. })));
    press(&mut app, 'S');
    assert_eq!(app.picker.take().unwrap().items.len(), 3); // no pending review → no discard option
    app.review = Some("R".into());
    press(&mut app, 'S');
    assert_eq!(app.picker.as_ref().unwrap().items[3].label, "Discard pending review (1 comment(s))");
    assert_eq!((a.line, a.left, a.start), (2, false, Some((2, true))));
    assert_eq!(wrap("aaa bbb ccc", 7), ["aaa bbb", "ccc"]);
}
