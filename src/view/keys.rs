//! Key handling: normal mode, pending (multi-key) commands, pickers, labels, and the help search.
use super::*;

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
        let ch = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);
        let c = match k.code {
            KeyCode::Char(c) => c,
            KeyCode::Esc => {
                (self.pending, self.count, self.select) = (None, 0, false);
                return;
            }
            KeyCode::Enter => {
                match self.link_at_cursor() {
                    Some(_) => self.open_link(),
                    None => self.action = self.open_target().map(Action::Open),
                }
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
            'R' => {
                self.job = Some(Job::Refresh);
                self.msg = "refreshing…".into();
            }
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
                ("t", "review threads"),
                ("p", "your pending comments"),
                ("A", "Claude's comments"),
                ("n", "new comments (not seen yet)"),
                ("a", "ask Claude: review / answer / make changes"),
                ("k", "hover / preview link"),
                ("v", "mark file viewed"),
                ("u", "update checkout (git pull)"),
                ("g", "git: uncommitted only / commit / push"),
                ("m", "messages, in full"),
                ("r", "resolve thread"),
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
                ("f", "open link in browser"),
            ]),
            'z' => ("view", &[("z", "cursor line to center"), ("t", "… to top"), ("b", "… to bottom")]),
            ']' => ("next", &[("g", "change (hunk)"), ("f", "file"), ("c", "comment thread"), ("a", "Claude's comment"), ("p", "pending comment"), ("n", "new comment")]),
            '[' => ("previous", &[("g", "change (hunk)"), ("f", "file"), ("c", "comment thread"), ("a", "Claude's comment"), ("p", "pending comment"), ("n", "new comment")]),
            CTRL_W => ("window", &[
                ("v", "toggle split view"),
                ("c", "comments in the margin / inline"),
                ("a", "Claude view: just Claude's review"),
                ("h", "old side / back to the code"),
                ("l", "new side / into the comments"),
            ]),
            'm' => ("match", &[("m", "matching bracket")]),
            _ => return None,
        })
    }

    pub(super) fn pending_key(&mut self, p: char, c: char) {
        let file = |rows: &[Row], r: usize| rows[r].line.kind == Kind::File;
        let thread = |rows: &[Row], r: usize| rows[r].thread.is_some() && (r == 0 || rows[r - 1].thread != rows[r].thread);
        let so = 3.min(self.height / 2);
        match (p, c) {
            ('g', 'g') => self.goto(0, 0),
            ('g', 'e') => self.goto(self.rows.len() - 1, 0),
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
                    self.msg = format!("loading {url}…");
                    self.job = Some(Job::Preview(url));
                }
                None => self.lsp_request("textDocument/hover", Req::Hover),
            },
            (']', 'g') => self.next_kind(true, change_start),
            ('[', 'g') => self.next_kind(false, change_start),
            (']', 'c') => self.next_kind(true, thread),
            ('[', 'c') => self.next_kind(false, thread),
            (']', 'a') => self.next_claude(true),
            ('[', 'a') => self.next_claude(false),
            (']', 'n') => self.next_new(true),
            ('[', 'n') => self.next_new(false),
            (']', 'p') => self.next_pending(true),
            ('[', 'p') => self.next_pending(false),
            ('z', 'z') => self.top = self.hd().saturating_sub(self.height / 2),
            ('z', 't') => self.top = self.hd().saturating_sub(so),
            ('z', 'b') => self.top = (self.hd() + so + 1).saturating_sub(self.height),
            (CTRL_W, 'v') => self.toggle_split(),
            (CTRL_W, 'a') => self.toggle_claude_view(),
            (CTRL_W, 'c') => {
                self.margin_pref = !self.margin_pref;
                self.msg = match self.margin_pref {
                    true => "comments in the right margin (when the window is wide enough)",
                    false => "comments inline",
                }
                .into();
            }
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
                self.picker = Some(Picker { title: " files ", items, query: String::new(), sel: 0 });
            }
            (' ', 'y') => self.yank(false),
            (' ', 'c') => self.copy_ref(),
            (' ', 'r') => self.toggle_resolved(),
            (' ', 'e') => self.start_edit(),
            (' ', 'd') => self.start_delete(),
            (' ', 'p') => self.pending_picker(),
            (' ', 't') => self.threads_picker(),
            (' ', 'a') => self.claude_menu(),
            (' ', 'A') => self.claude_picker(),
            (' ', 'n') => self.new_picker(),
            (' ', 'v') => self.toggle_viewed(),
            (' ', 'g') => self.git_menu(),
            (' ', 'm') => self.messages(),
            (' ', 'u') => {
                self.msg = "updating the checkout…".into();
                self.job = Some(Job::Pull);
            }
            ('f' | 't' | 'F' | 'T', c) => self.find_char(p, c),
            _ => {}
        }
    }

    /// Picker items whose label contains the query as a (case-insensitive) subsequence.
    pub(super) fn matches(&self) -> Vec<usize> {
        let Some(p) = &self.picker else { return vec![] };
        let q = p.query.to_lowercase();
        (0..p.items.len())
            .filter(|&i| {
                let it = &p.items[i];
                let text = std::iter::once(&it.label).chain(&it.detail).map(|s| s.to_lowercase()).collect::<Vec<_>>().join(" ");
                let mut chars = text.chars();
                q.chars().all(|c| chars.any(|x| x == c))
            })
            .collect()
    }

    pub(super) fn picker_key(&mut self, k: KeyEvent) {
        let m = self.matches();
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let Some(p) = &mut self.picker else { return };
        // In the pending comments picker, Ctrl-d deletes the selected one (after a confirmation).
        if ctrl && k.code == KeyCode::Char('d') && p.title == " pending comments " {
            if let Some(Target::Comment(ti, ci)) = m.get(p.sel).map(|&i| &p.items[i].target)
                && let Some(c) = self.threads.get(*ti).and_then(|t| t.comments.get(*ci)).cloned()
            {
                self.confirm_delete(c);
            }
            return;
        }
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
                                Item::new("No, keep it", Target::Nothing),
                                Item::new(format!("Yes, discard {n} pending comment(s)"), Target::DiscardYes),
                            ];
                            self.picker = Some(Picker { title: " discard pending review? ", items, query: String::new(), sel: 0 });
                        }
                        Target::DiscardYes => {
                            if let Some(r) = self.review.clone() {
                                self.msg = "discarding…".into();
                                self.job = Some(Job::Discard(r));
                            }
                        }
                        Target::Comment(ti, ci) => self.goto_comment(ti, ci),
                        Target::DeleteComment(id) => {
                            self.msg = "deleting…".into();
                            self.job = Some(Job::DeleteComment(id));
                        }
                        Target::Claude(ask) => self.pick_claude(ask),
                        Target::ClaudeNotes => self.edit_claude_notes(),
                        Target::ClaudeReplace(prs) => self.action = Some(Action::Replace(prs)),
                        Target::Publish => self.action = Some(Action::Publish),
                        Target::CloseOriginals => self.action = Some(Action::CloseOriginals),
                        Target::ClaudeFresh => self.fresh_claude_session(),
                        Target::GitView => self.toggle_uncommitted(),
                        Target::GitCommit => self.action = Some(Action::Commit(self.commit_draft())),
                        Target::GitPush => (self.msg, self.job) = ("pushing…".into(), Some(Job::Push)),
                        Target::ClaudeDismissOutdated(sure) => self.dismiss_outdated(sure),
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
