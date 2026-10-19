//! Key handling: normal mode, pending (multi-key) commands, pickers, labels, and the help search.
use super::*;

pub(super) const HELP: &str = "
MOVE
  h j k l, arrows      move (counts: 5j)
  w b e  W B E         word motions (select)
  gh gl gs             line start / end / first non-blank
  f t F T <c>          find char
  gw                   jump labels
  mm                   matching bracket
  gg ge                top / end (in the comments pane: its first / last)
  Ctrl-d Ctrl-u        half page down / up
  Ctrl-e Ctrl-y        scroll 5 lines
  zz zt zb             cursor line to center / top / bottom
  Ctrl-o               jump back

DIFF
  ]g [g                next / prev hunk (change)
  ]f [f, Space f       next / prev file, file picker
  ]c [c                next / prev comment thread
  ]u [u                next / prev unresolved one (wraps around)
  ]a [a                next / prev Claude comment (wraps around)
  ]p [p                next / prev comment of your pending review (wraps around)
  /  n N  *            search, next / prev match, search selection

SELECT & COPY
  x                    select line (repeat to extend)
  v  ;  Esc            select mode / collapse / exit select mode
  y  (or Space y)      copy text
  Y                    copy as patch
  Space c              copy: a link's URL, file:line, or a comment's / the PR's link

VIEW
  + =  -               hunk context +10 / -10 lines
  Z                    whole file
  Tab                  fold file / PR info / thread
  gn  gp               comments pane: next / previous tab (comments, Claude's, the PR's)
  Shift-Tab            fold / unfold all files
  Ctrl-w v             toggle split (side-by-side) view
  Ctrl-w n             relative line numbers on / off (kept for next time)
  Ctrl-w e             the contents on the left (description, messages, files), on / off
                       and the PR's commits under it: Enter shows one, v … Enter a range,
                       m its whole message, a asks Claude about it (or for a change),
                       f folds what isn't committed into it;
                       Ctrl-w j / Ctrl-w k (or Tab there) go to the commits / the files
  Ctrl-w h  Ctrl-w l   pane left / right: files, old / new side, comments (or Ctrl-h Ctrl-l)
  Ctrl-w c             show / hide the comments pane (╎ ● mark their lines)
  Ctrl-w f             comments pane: a column beside the code / floating over it
  Ctrl-w ,  Ctrl-w .   the pane you're in narrower / wider (Ctrl-w = usual width)
  Ctrl-w q             hide the pane you're in (files' list or comments)
  Enter                open hx at cursor (on a link: open it)
  gf                   open the link under the cursor, or go to the file:line under it
                       (as comments cite code) or to where the function / type named
                       under it is defined (Ctrl-o comes back)

RUST-ANALYZER
  gd gr gy gi          definition / references / type / impl (on a name in a comment too)
  Space k              hover; on an issue/PR link: preview it

ON SELECTED LINES
  c                    comment, in your pending review (C: post it now)
  s                    suggest a change, in your pending review
  a                    a comment for Claude: an answer, a suggestion, or the change you want
                       (it acts right away; not on GitHub); on a
                       replacement's description or closing note: ask Claude to redraft it

ON A REVIEW THREAD
  r                    reply, in your pending review (C: post it now)
  Space r              resolve / unresolve
  Space =              react (👍 🎉 …), or take your reaction back (PR comments too)
  Space l              the PR's labels, assignees, reviewers: add or remove (request reviews)
  Space e  Space d     edit / delete your comment (delete asks first)

ON CLAUDE'S COMMENT
  r                    reply to Claude, saying what you want (it acts right away, in the
                       background)
  c                    copy it into your pending review (C: post it now)
  Space r  Space d     resolve (Claude skips it) / dismiss

ON PR TEXT
  c                    quote it in your review summary (S starts from it)
  C                    quote it in a new conversation comment

ON CLAUDE'S REVIEW (its summary)
  c                    add it to your review summary (S starts from it)
  C                    post it now, as your PR comment

REVIEW
  S                    submit (comment / approve / request changes), or discard
  Space g              git: show only what isn't committed (Claude's changes), commit, amend
                       it into the commits it belongs in, push, update the checkout, rebase
                       onto the base (Claude resolves conflicts)
  Space p  Space t     pick from your pending comments (Ctrl-d deletes) / all threads
  Space v              mark the file viewed (folds it, moves on)
  Space a              ask Claude Code to review, answer your replies, or make the changes
                       (or check on its run now, or stop one that's stuck)
  Space A              pick from Claude's comments ([waiting]: its turn, [answered]: yours)
  ]n [n  Space n       next / prev [new] thread (ones you haven't seen) / pick from them

GENERAL
  ?                    this help (/ to search it)
  R                    refresh (git diff + GitHub)
  Space m              messages so far, in full (the status bar cuts long ones)
  Space s              a shell in the checkout's directory (zellij: a floating pane)
  Space o              the PR's whole activity, or its latest (beside the description)
  mouse                wheel scrolls, click moves (or folds, on an arrow), drag selects
  q, Ctrl-c            quit
";

impl App {
    pub(super) fn label_key(&mut self, k: KeyEvent) {
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

    /// The help text, styled: section headings, keys (separators dimmed), descriptions wrapped
    /// with a hanging indent under the description column, `(notes)` dimmed.
    /// Rebuild the help overlay for its current search.
    pub(super) fn refresh_help(&mut self) {
        let Some((q, typing)) = self.help.clone() else { return };
        // Room for popup text: the window minus the popup's margins, padding and scrollbar. Help
        // takes two columns when they fit, else one of at most 97.
        let cw = (self.width.saturating_sub(4) as usize).saturating_sub(3);
        let title = match (typing, q.is_empty()) {
            (true, _) => format!(" keys /{q}▏ Enter keep · Esc clear "),
            (false, false) => format!(" keys /{q} · / edit · j/k scroll · other keys close "),
            (false, true) => " keys · / search · j/k scroll · other keys close ".into(),
        };
        self.popup = Some((title, self.help_lines(cw, &q), 0));
    }

    pub(super) fn key(&mut self, k: KeyEvent) {
        self.msg.clear();
        if let Some((q, typing)) = &mut self.help {
            let handled = match (*typing, k.code) {
                (true, KeyCode::Char(c)) => {
                    q.push(c);
                    true
                }
                (true, KeyCode::Backspace) => {
                    q.pop();
                    true
                }
                (true, KeyCode::Enter) => {
                    *typing = false;
                    true
                }
                (true, KeyCode::Esc) => {
                    q.clear();
                    *typing = false;
                    true
                }
                (true, _) => true,
                (false, KeyCode::Char('/')) => {
                    *typing = true;
                    true
                }
                _ => false, // j/k scroll, anything else closes: the popup keys below
            };
            if handled {
                return self.refresh_help();
            }
        }
        if let Some((_, _, scroll)) = &mut self.popup {
            match k.code {
                KeyCode::Char('j') | KeyCode::Down => *scroll += 1,
                KeyCode::Char('k') | KeyCode::Up => *scroll = scroll.saturating_sub(1),
                _ => (self.popup, self.help) = (None, None),
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
        if self.files_focus && self.files_w > 0 && self.pending.is_none() && self.files_key(k) {
            return;
        }
        let ch = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);
        let c = match k.code {
            KeyCode::Char(c) => c,
            KeyCode::Esc => {
                (self.pending, self.count, self.select) = (None, 0, false);
                return;
            }
            KeyCode::Enter if self.rows[self.head.row].info == Some(MORE_CONVERSATION) => return self.toggle_conversation(),
            KeyCode::Enter => {
                match self.link_at_cursor() {
                    Some(_) => self.open_link(),
                    None => self.action = self.open_target().map(Action::Open),
                }
                return;
            }
            // Many terminals send Ctrl-h as Backspace.
            KeyCode::Backspace => return self.side(false),
            // On a comment hidden on GitHub: it shown (or hidden again).
            KeyCode::Tab if self.toggle_hidden() => return,
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
            return; // no Alt bindings (n / N repeat the search, as in Helix)
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
            'n' => (0..n).for_each(|_| self.find(true)),
            'N' => (0..n).for_each(|_| self.find(false)),
            '*' => {
                self.search = self.selection_text(false).trim_end_matches('\n').to_owned();
                self.msg = format!("search: {}", self.search);
            }
            '+' | '=' => self.expand(10),
            '-' => self.expand(-10),
            'Z' => self.toggle_whole(),
            '?' => {
                self.help = Some((String::new(), false));
                self.refresh_help();
            }
            'c' => self.comment_or_reply(true),
            'a' => self.start_ask_claude(),
            's' => self.start_suggest(),
            'C' => self.comment_or_reply(false),
            'r' => self.start_reply(true),
            'S' => self.review_menu(),
            'R' => self.start(Job::Refresh),
            'q' => self.quit = true,
            'g' | ']' | '[' | ' ' | 'f' | 't' | 'F' | 'T' | 'z' | 'm' => self.pending = Some(c),
            _ => {}
        }
    }

    /// What can follow prefix key `p`, for the which-key popup (as Helix shows): (title, keys).
    /// Kept next to `pending_key`, which they mirror.
    pub(super) fn which(p: char) -> Option<(&'static str, &'static [(&'static str, &'static str)])> {
        Some(match p {
            ' ' => ("space", &[
                ("f", "file picker"),
                ("t", "review threads (Tab: unresolved, pending, …)"),
                ("p", "… your pending comments"),
                ("A", "… Claude's comments"),
                ("n", "… new comments (not seen yet)"),
                ("a", "ask Claude: review / answer / make changes"),
                ("k", "hover / preview link"),
                ("v", "mark file viewed"),
                ("g", "git: uncommitted only / commit / push / update"),
                ("m", "messages, in full"),
                ("s", "a shell in the checkout's directory"),
                ("o", "the PR's whole activity / its latest"),
                ("r", "resolve thread"),
                ("=", "react to the comment (👍 🎉 …)"),
                ("l", "the PR's labels, assignees, reviewers"),
                ("e", "edit your comment"),
                ("d", "delete / dismiss comment"),
                ("y", "copy"),
                ("c", "copy ref (file:line / comment link)"),
            ]),
            'g' => ("goto", &[
                ("g", "top"),
                ("e", "end"),
                ("h", "line start"),
                ("l", "line end"),
                ("s", "first non-blank"),
                ("w", "jump labels"),
                ("d", "definition"),
                ("r", "references"),
                ("y", "type definition"),
                ("i", "implementations"),
                ("f", "open link / file:line / definition"),
                ("n", "pane: next tab (comments / PR)"),
                ("p", "pane: previous tab"),
            ]),
            'z' => ("view", &[("z", "cursor line to center"), ("t", "… to top"), ("b", "… to bottom")]),
            ']' => ("next", &[("g", "change (hunk)"), ("f", "file"), ("c", "comment thread"), ("u", "unresolved comment"), ("a", "Claude's comment"), ("p", "pending comment"), ("n", "new comment")]),
            '[' => ("previous", &[("g", "change (hunk)"), ("f", "file"), ("c", "comment thread"), ("u", "unresolved comment"), ("a", "Claude's comment"), ("p", "pending comment"), ("n", "new comment")]),
            CTRL_W => ("window", &[
                ("v", "toggle split view"),
                ("n", "relative line numbers on / off"),
                ("e", "the files' list on / off"),
                ("c", "show / hide the comments pane"),
                ("f", "comments pane: a column / floating over the code"),
                (",", "narrower pane (the one you're in, else comments)"),
                (".", "wider pane"),
                ("=", "the pane at its usual width"),
                ("q", "hide the pane you're in (files / comments)"),
                ("j", "the commits (in the files' list)"),
                ("k", "the files (in the files' list)"),
                ("h", "the pane to the left"),
                ("l", "the pane to the right"),
            ]),
            'm' => ("match", &[("m", "matching bracket")]),
            _ => return None,
        })
    }

    pub(super) fn pending_key(&mut self, p: char, c: char) {
        let file = |rows: &[Row], r: usize| rows[r].line.kind == Kind::File;
        let so = 3.min(self.height / 2);
        match (p, c) {
            // In the pane, its own top / end (the first and last comment, or the PR's text).
            ('g', 'g') => self.goto(self.pane_rows().first().copied().unwrap_or(0), 0),
            ('g', 'e') => self.goto(self.pane_rows().last().copied().unwrap_or(self.rows.len() - 1), 0),
            ('g', 'h') => self.goto(self.head.row, 0),
            ('g', 'l') => self.goto(self.head.row, usize::MAX),
            ('m', 'm') => self.match_bracket(),
            ('g', 'f') => self.open_link(),
            ('g', 's') => {
                let col = self.chars(self.head.row).iter().position(|c| !c.is_whitespace()).unwrap_or(0);
                self.goto(self.head.row, col);
            }
            ('g', 'd') => self.lsp_request("textDocument/definition", Req::Goto),
            ('g', 'y') => self.lsp_request("textDocument/typeDefinition", Req::Goto),
            ('g', 'i') => self.lsp_request("textDocument/implementation", Req::Goto),
            ('g', 'r') => self.lsp_request("textDocument/references", Req::Goto),
            ('g', 'w') => self.show_labels(),
            (' ', 'k') => match self.link_at_cursor().filter(|u| gh::issue_ref(u).is_some()).map(str::to_owned) {
                Some(url) => {
                    self.start(Job::Preview(url));
                }
                None => self.lsp_request("textDocument/hover", Req::Hover),
            },
            (']', 'g') => self.next_kind(true, change_start),
            ('[', 'g') => self.next_kind(false, change_start),
            (']', 'c') => self.next_of('c', true),
            ('[', 'c') => self.next_of('c', false),
            (']', 'a') => self.next_of('a', true),
            ('[', 'a') => self.next_of('a', false),
            (']', 'n') => self.next_of('n', true),
            ('[', 'n') => self.next_of('n', false),
            (']', 'u') => self.next_of('u', true),
            ('[', 'u') => self.next_of('u', false),
            (']', 'p') => self.next_of('p', true),
            ('[', 'p') => self.next_of('p', false),
            ('z', 'z') => self.top = self.hd().saturating_sub(self.height / 2),
            ('z', 't') => self.top = self.hd().saturating_sub(so),
            ('z', 'b') => self.top = (self.hd() + so + 1).saturating_sub(self.height),
            (CTRL_W, 'v') => self.toggle_split(),
            (CTRL_W, 'c') => {
                let show = !self.margin_active;
                self.margin_pref = Some(show);
                self.save_pane("shown", show.into());
                // (PR text stays on screen, after the description.)
                if !show && self.margin_side() && !self.rows[self.head.row].attached {
                    // Off the comment, onto what it's beside (it'd show it again).
                    self.goto(self.line_beside(), 0);
                }
                self.msg = match show {
                    true => "comments shown (Ctrl-w c hides them)",
                    false => "comments hidden: their lines are marked at the code's right edge (Ctrl-w c shows them)",
                }
                .into();
            }
            ('g', 'n') | ('g', 'p') => self.cycle_bar(c == 'n'),
            (CTRL_W, 'e') => self.toggle_files(),
            (CTRL_W, 'n') => {
                let on = !self.settings.relative;
                self.settings.relative = on;
                self.save_pane("relative", on.into());
                self.msg = match on {
                    true => "relative line numbers (Ctrl-w n: off)",
                    false => "no relative line numbers (Ctrl-w n: on)",
                }
                .into();
            }
            (CTRL_W, 'f') => {
                let float = !self.margin_float;
                self.margin_float_pref = Some(float);
                self.save_pane("float", float.into());
                self.margin_pref = Some(true);
                self.msg = match float {
                    true => "comments pane floating over the code (Ctrl-w f: a column beside it)",
                    false => "comments pane as a column beside the code (Ctrl-w f: floating over it)",
                }
                .into();
            }
            (CTRL_W, ',') | (CTRL_W, '.') if self.files_focus => self.resize_files(if c == '.' { 4 } else { -4 }),
            (CTRL_W, '=') if self.files_focus => self.resize_files(0),
            (CTRL_W, 'q') if self.files_focus => self.toggle_files(),
            (CTRL_W, 'q') if self.margin_side() => self.pending_key(CTRL_W, 'c'),
            (CTRL_W, 'q') => self.msg = "the code stays; Ctrl-w q hides the files' list or comments pane you're in".into(),
            (CTRL_W, ',') | (CTRL_W, '.') => {
                let w = self.margin_w as isize + if c == '.' { MARGIN_STEP } else { -MARGIN_STEP };
                let w = w.max(MARGIN_NARROW as isize) as usize;
                self.margin_w_pref = Some(w);
                self.margin_pref = Some(true);
                self.save_pane("width", w.into());
            }
            (CTRL_W, '=') => {
                self.margin_w_pref = None;
                self.save_pane("width", Value::Null);
            }
            (CTRL_W, 'j') => self.focus_list(true),
            (CTRL_W, 'k') => self.focus_list(false),
            (CTRL_W, 'h') => self.side(false),
            (CTRL_W, 'l') => self.side(true),
            (']', 'f') => self.next_kind(true, file),
            ('[', 'f') => self.next_kind(false, file),
            (' ', 'f') => {
                let items = self
                    .rows
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| r.line.kind == Kind::File)
                    .map(|(i, r)| Item::new(r.line.text.clone(), Target::Row(i)))
                    .collect();
                self.picker = Some(Picker::new(" files ", items));
            }
            (' ', 'y') => self.yank(false),
            (' ', 'c') => self.copy_ref(),
            (' ', 'r') => self.toggle_resolved(),
            (' ', '=' | '+') => self.start_react(),
            (' ', 'l') => match self.pr {
                Some(_) => self.start(Job::Labels),
                None => self.msg = "no PR here".into(),
            },
            (' ', 'e') => self.start_edit(),
            (' ', 'd') => self.start_delete(),
            (' ', 't') => self.threads_picker(THREADS_ALL),
            (' ', 'p') => self.threads_picker(THREADS_PENDING),
            (' ', 'a') => self.claude_menu(),
            (' ', 'A') => self.threads_picker(THREADS_CLAUDE),
            (' ', 'n') => self.threads_picker(THREADS_NEW),
            (' ', 'v') => self.toggle_viewed(),
            (' ', 'g') => self.git_menu(),
            (' ', 'm') => self.messages(),
            (' ', 's') => self.action = Some(Action::Shell),
            (' ', 'o') => self.toggle_conversation(),
            ('f' | 't' | 'F' | 'T', c) => self.find_char(p, c),
            _ => {}
        }
    }

    /// Picker items whose label contains the query as a (case-insensitive) subsequence.
    pub(super) fn matches(&self) -> Vec<usize> {
        let Some(p) = &self.picker else { return vec![] };
        let q = p.query.to_lowercase();
        let text = |it: &Item| std::iter::once(&it.label).chain(&it.detail).map(|s| s.to_lowercase()).collect::<Vec<_>>().join(" ");
        let mut m: Vec<(bool, usize)> = (0..p.items.len())
            .filter_map(|i| {
                let text = text(&p.items[i]);
                let mut chars = text.chars();
                // What has the query as typed comes first ("check": "Check …", not "Edit Claude's notes").
                q.chars().all(|c| chars.any(|x| x == c)).then(|| (!text.contains(&q), i))
            })
            .collect();
        m.sort();
        m.into_iter().map(|(_, i)| i).collect()
    }

    pub(super) fn picker_key(&mut self, k: KeyEvent) {
        let m = self.matches();
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let Some(p) = &mut self.picker else { return };
        // The threads picker: Tab / Shift-Tab to the next / previous filter.
        if let Some(tab) = THREAD_TABS.iter().position(|&t| t == p.title)
            && matches!(k.code, KeyCode::Tab | KeyCode::BackTab)
        {
            let n = THREAD_TABS.len();
            return self.threads_picker(if k.code == KeyCode::Tab { (tab + 1) % n } else { (tab + n - 1) % n });
        }
        // In the pending comments picker, Ctrl-d deletes the selected one (after a confirmation).
        if ctrl && k.code == KeyCode::Char('d') && p.title == THREAD_TABS[THREADS_PENDING] {
            if let Some(Target::Comment(ti, ci)) = m.get(p.sel).map(|&i| &p.items[i].target)
                && let Some(c) = self.threads.get(*ti).and_then(|t| t.comments.get(*ci)).cloned()
            {
                self.confirm_delete(c);
            }
            return;
        }
        match k.code {
            KeyCode::Esc => (self.picker, self.wanting) = (None, None),
            KeyCode::Enter => {
                let mut p = self.picker.take().unwrap();
                if let Some(&i) = m.get(p.sel) {
                    match p.items.swap_remove(i).target {
                        Target::Do(a) => self.action = Some(a),
                        Target::Run(job) => self.start(job),
                        Target::Row(r) => self.jump(r, 0),
                        Target::Loc(l) => self.jump_loc(&l),
                        Target::Discard => {
                            let n = self.pending_count();
                            let items = vec![
                                Item::new("No, keep it", Target::Nothing),
                                Item::new(format!("Yes, discard {n} pending comment(s)"), self.review.clone().map_or(Target::Nothing, |r| Target::Run(Job::Discard(r)))),
                            ];
                            self.picker = Some(Picker::new(" discard pending review? ", items));
                        }
                        Target::Comment(ti, ci) => self.goto_comment(ti, ci),
                        Target::Claude(ask) => self.pick_claude(ask),
                        Target::Want(w) => self.want(w),
                        Target::ClaudeNotes => self.edit_claude_notes(),
                        Target::ClaudeQuestion => {
                            self.bar_to(Bar::Claude); // where it's answered
                            self.action = Some(Action::AskConversation { url: ON_SUMMARY.into(), prefill: String::new(), want: "" });
                        }
                        Target::ReplaceRemote(name, close) => match crate::replace::set_remote(&name) {
                            Ok(()) => self.action = Some(Action::Publish(close)),
                            Err(e) => self.msg = format!("{e:#}"),
                        },
                        Target::ClaudeFresh => self.fresh_claude_session(),
                        Target::ClaudePane => self.show_claude_pane(),
                        Target::ClaudeCheck => self.check_claude(),
                        Target::ClaudeStop => self.stop_claude(),
                        Target::AmendMenu => self.amend_menu(),
                        Target::RebaseClaude => {
                            if let Err(e) = self.rebase_claude() {
                                self.msg = format!("{e:#}");
                            } else {
                                self.msg = "Claude is resolving the rebase's conflicts".into();
                            }
                        }
                        Target::RebaseAbort => self.abort_rebase(),
                        Target::Amend => self.amend_now(),
                        Target::AmendAsk => self.amend_ask(false),
                        Target::AmendAll => self.amend_ask(true),
                        Target::GitView => self.toggle_uncommitted(),
                        Target::GitMerge => (self.merge, self.quit) = (true, true),
                        Target::GitTakeTheirs(sure) => self.take_theirs(sure),
                        Target::GitReady => self.mark_ready(),
                        Target::GitApproveRuns => self.approve_runs(),
                        Target::ClaudeDismissOutdated(sure) => self.dismiss_outdated(sure),
                        Target::DescApprove(sure) => self.approve_description(sure),
                        Target::DescDiscard => self.discard_description(),
                        Target::CloseDiscard => self.discard_closing(),
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
}
