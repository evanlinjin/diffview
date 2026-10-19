//! Drawing dv list: its tabs, the cards, the status bar, and the keys and help popups.
use super::*;

impl List {
    /// Cards on screen (between the header and the status bar).
    pub(super) fn height(&self) -> usize {
        ratatui::crossterm::terminal::size().map_or(10, |(_, h)| h.saturating_sub(2) as usize / self.card_rows())
    }

    /// A card's rows: 3 in a wide window (two zones), else 4, and a blank one between cards; on
    /// the todo tab, one more, for what needs you.
    pub(super) fn card_rows(&self) -> usize {
        let lines = if self.wide { 3 } else { 4 };
        lines + usize::from(self.tab == Tab::Todo) + 1
    }

    pub(super) fn draw(&mut self, f: &mut Frame) {
        let area = f.area();
        self.wide = area.width as usize >= TWO_ZONES;
        let rows = self.card_rows();
        let h = area.height.saturating_sub(2) as usize / rows;
        let sel = self.sel.min(self.shown().len().saturating_sub(1));
        // Keep the selection on screen.
        (self.sel, self.top) = (sel, self.top.min(sel).max((sel + 1).saturating_sub(h)));
        let (ui, shown) = (&self.ui, self.shown());
        f.render_widget(Paragraph::new("").style(ui.base), area);

        let head = Rect { height: 1.min(area.height), ..area };
        // As the PR view's title bar: the darker of the status bar's and the headers' background.
        let hs = ui.base.patch(Style { bg: ui.bar, ..ui.header });
        let arrow = if self.rev { '↑' } else { '↓' };
        // Tabs: the current one bold, with counts.
        let open = self.prs.iter().filter(|p| p.open()).count();
        // As the PR view's tabs: the one shown lit, the others dim.
        let dim = hs.remove_modifier(ratatui::style::Modifier::BOLD).patch(Style { fg: ui.linenr.fg, ..Style::new() });
        let tab = |name: String, on: bool| Span::styled(format!(" {name} "), if on { ui.menu_selected } else { dim });
        // The top bar: the tabs, and where you are in the one shown. (What state the list is in
        // goes on the status bar's right; the keys, in the keys popup.)
        let names = [
            (Tab::Todo, format!("todo {}", self.todo_now())),
            (Tab::Open, format!("open {open}{}", if self.prs.len() >= github::LIST_MAX { "+" } else { "" })),
            (Tab::Worktrees, format!("worktrees {}", self.local.len())),
            (Tab::Replacements, format!("drafts {}", self.reps.len())),
        ];
        let mut spans = vec![Span::styled(" ", hs)];
        let mut hits = vec![];
        let mut x = 1u16;
        for (t, name) in names {
            let s = tab(name, self.tab == t);
            let w = s.width() as u16;
            hits.push((x..x + w, t));
            spans.extend([s, Span::styled(" ", hs)]);
            x += w + 1;
        }
        let tabs = Line::from(spans);
        f.render_widget(Paragraph::new(tabs).style(hs), head);
        let pos = if shown.is_empty() { "0 / 0 ".into() } else { format!("{} / {} ", sel + 1, shown.len()) };
        // Only where it goes (at the right end): over the whole line, its style would take the
        // lit tab's.
        let w = (pos.chars().count() as u16).min(head.width);
        f.render_widget(Paragraph::new(pos).style(hs), Rect { x: head.right() - w, width: w, ..head });

        // The base most PRs target goes without saying; others show as "base ← branch".
        let mut bases: HashMap<&str, usize> = HashMap::new();
        self.prs.iter().for_each(|p| *bases.entry(p.base.as_str()).or_default() += 1);
        let usual = bases.into_iter().max_by_key(|b| b.1).map(|b| b.0.to_owned()).unwrap_or_default();
        let cx = Cards {
            ui,
            num_w: shown.iter().map(|p| crate::replace::shown(p.number).chars().count()).max().unwrap_or(2),
            width: area.width as usize,
            checks_loading: self.checks.is_some(),
            usual: &usual,
        };
        let lines: Vec<Line> = shown
            .iter()
            .enumerate()
            .skip(self.top)
            .take(h)
            .flat_map(|(i, p)| {
                let item = (self.tab == Tab::Todo).then(|| self.todo_shown().into_iter().find(|(c, _)| std::ptr::eq(*c, *p)).map(|x| x.1)).flatten();
                let rep_card = self.tab == Tab::Replacements || item.is_some_and(|it| it.rep);
                let mark = match rep_card {
                    true => {
                        let replaces = self.rep_of.get(&p.number).map(|r| match r.replaces.is_empty() {
                            // A new PR: what you asked of it.
                            true => format!("asked: {}", r.task.lines().next().unwrap_or("")),
                            false => {
                                let each: Vec<String> = r.replaces.iter().map(|o| format!("#{} (@{})", o.number, o.author)).collect();
                                format!("replaces {}", each.join(", "))
                            }
                        });
                        Mark { marked: false, claude: self.draft_state(p.number, "claude"), new: false, replaces, awaiting: 0 }
                    }
                    false => {
                        // On the todo tab, what Claude could do there (`a a`), when it isn't at it.
                        let could = item.and_then(|it| it.claude.as_ref()).filter(|_| !self.batch.get(&p.number).is_some_and(batch::Status::running)).map(batch::Task::could);
                        let claude = match (self.claude_state(p.number), could) {
                            (Some((s, ok)), Some(c)) => Some((format!("{s} · {c}"), ok)),
                            (None, Some(c)) => Some((format!("claude {c}"), true)),
                            (s, None) => s,
                        };
                        let awaiting = self.extra.get(&p.number).map_or(0, |e| e.awaiting);
                        Mark { marked: self.marked.contains(&p.number), claude, new: self.new.contains(&p.number), replaces: None, awaiting }
                    }
                };
                let local = if rep_card { None } else { self.local.get(&p.number) };
                let mut lines = card(p, &cx, i == sel, local, mark);
                if let Some(it) = item {
                    lines.insert(1, todo_line(i + 1, it, &cx, i == sel)); // under the title
                }
                lines.push(Line::raw("")); // room between cards
                lines
            })
            .collect();
        let typed = self.query.trim().trim_start_matches('#').parse::<u64>().ok();
        let none = match (self.tab, self.query.is_empty(), typed) {
            (Tab::Worktrees, true, _) => " no PR worktrees here yet: `dv N -w` (or dv list -w, then Enter) makes one".into(),
            (Tab::Replacements, true, _) => " no replacements yet: A A on a PR (or marked ones) has Claude draft one".into(),
            (_, _, Some(n)) => format!(" #{n} isn't listed: Enter opens it anyway"),
            _ => String::new(),
        };
        if shown.is_empty() {
            f.render_widget(Paragraph::new(none).style(ui.base.patch(ui.stale)), Rect { y: area.y + 2, height: 1, ..area });
        }
        f.render_widget(Paragraph::new(lines), Rect { y: area.y + 1, height: (h * rows) as u16, ..area });

        let status = Rect { y: area.bottom().saturating_sub(1), height: 1.min(area.height), ..area };
        let bar = ui.base.patch(ui.status);
        let left = match (self.filtering, self.query.is_empty(), self.msg.is_empty()) {
            (true, _, _) => format!(" /{}▏  author: label: base: branch: is:draft review: ci:  -x negates", self.query),
            (_, _, false) => format!(" {}", self.msg),
            (_, false, _) => format!(" filter: {}  (Esc clears)", self.query),
            _ => String::new(), // the keys are in the bottom-right popup
        };
        // On the right, the list's state: how it's ordered (on the todo tab, Claude's ranking: how
        // old, what it cost, what's changed since), whether auto's on (it starts Claude runs, and
        // changes PRs on GitHub: in the warning color), and the flags PRs open with.
        let order = match (self.tab, &self.rank_rx, &self.ranked) {
            (Tab::Todo, Some(_), _) => "Claude is ranking…".to_owned(),
            (Tab::Todo, None, Some(r)) => format!("ranked {} ago, {}{}", ago_secs(crate::claude::now_secs() - r.at), todo::cost_text(r.cost), self.since_text()),
            (Tab::Todo, None, None) => "in the rules' order".to_owned(),
            _ => format!("by {} {arrow}", self.order.name()),
        };
        let state = Line::from(vec![
            Span::styled(format!("{order}  ·  "), bar),
            match self.auto {
                true => Span::styled("auto on", bar.patch(Style { fg: ui.warning.fg, ..Style::new() })),
                false => Span::styled("auto off", bar),
            },
            Span::styled(if self.flags.is_empty() { "  ".to_owned() } else { format!("  ·  opens with {}  ", self.flags) }, bar),
        ]);
        // The message first: the state where there's room beside it.
        let (lw, sw) = (Line::from(left.as_str()).width() as u16, state.width() as u16);
        let right = if lw + 2 + sw <= status.width { sw } else { 0 };
        if right > 0 {
            f.render_widget(Paragraph::new(state).alignment(Alignment::Right).style(bar), status);
        }
        f.render_widget(Paragraph::new(left).style(bar), Rect { width: status.width - right, ..status });
        if !self.help && !self.filtering {
            // The keys for this tab, bottom right, as dv's diff view shows them.
            let enter = match self.tab {
                Tab::Todo => "see to it (opens the PR or replacement)".to_owned(),
                Tab::Worktrees => "review, in its worktree".to_owned(),
                Tab::Replacements => "review it, in its worktree".to_owned(),
                Tab::Open if self.flags.is_empty() => "review".to_owned(),
                Tab::Open => format!("review (dv N {})", self.flags),
            };
            let tab = match self.tab {
                Tab::Todo => "open PRs",
                Tab::Open => "PRs with a worktree",
                Tab::Worktrees => "replacements",
                Tab::Replacements => "what needs you",
            };
            let remove = if self.tab == Tab::Replacements { "remove replacement" } else { "remove worktree" };
            let keys = [
                ("Enter", enter.as_str()),
                ("Tab", tab),
                ("/", "filter"),
                ("o O", "sort / reverse"),
                ("R", "refresh (from GitHub)"),
                ("r r", "Claude ranks what needs you (a run)"),
                ("s", "defer it (or bring it back)"),
                ("p", "post what it waits on (a comment)"),
                ("m", "mark"),
                ("a a", if self.tab == Tab::Todo { "Claude does its part (marked)" } else { "Claude reviews marked" }),
                ("A A", "Claude drafts a replacement"),
                ("M M", "merge (github-merge.py)"),
                ("W W", "approve its workflow runs"),
                ("U U", "unmark all"),
                ("N N", "Claude drafts a new PR (unpublished)"),
                ("C", "auto on / off (CI, reviews, ranking)"),
                ("b b", "ask for a rebase (a comment)"),
                ("D D", remove),
                ("?", "all keys"),
                ("q", "quit"),
            ];
            let reps = self.tab == Tab::Replacements;
            let lines: Vec<Line> = keys
                .iter()
                .filter(|(k, _)| match *k {
                    "D D" => !matches!(self.tab, Tab::Open | Tab::Todo),
                    "o O" => self.tab != Tab::Todo, // in the order of what needs you
                    "r r" | "s" | "p" => self.tab == Tab::Todo,
                    "m" | "a a" | "A A" | "M M" | "b b" | "W W" | "U U" => !reps,
                    _ => true,
                })
                .map(|(k, d)| Line::from(vec![Span::styled(format!("{k:<6}"), ui.key), Span::raw(d.to_string())]))
                .collect();
            let w = (lines.iter().map(Line::width).max().unwrap_or(10) as u16 + 4).min(area.width);
            let h = (lines.len() as u16 + 2).min(area.height.saturating_sub(1));
            let r = Rect { x: area.width - w, y: area.height.saturating_sub(1 + h), width: w, height: h };
            let inner = crate::view::popup_frame(ui, f, r, "keys", "");
            // (Down to its last row: it sits on the status bar.)
            f.render_widget(Paragraph::new(lines), Rect { height: inner.height + 1, ..inner });
        }
        if self.help {
            // Like dv's popups: no border, the popup background, a header row.
            let lines: Vec<&str> = HELP.lines().collect();
            let w = (lines.iter().map(|l| l.chars().count()).max().unwrap_or(0) as u16 + 4).min(area.width);
            let h = (lines.len() as u16 + 3).min(area.height);
            let r = Rect { x: (area.width - w) / 2, y: (area.height - h) / 2, width: w, height: h };
            let inner = crate::view::popup_frame(ui, f, r, "dv list keys", "any key closes");
            let body: Vec<Line> = lines
                .iter()
                .map(|l| {
                    let (k, d) = l.split_at(l.char_indices().nth(23).map_or(l.len(), |(i, _)| i));
                    Line::from(vec![Span::styled(k.to_owned(), ui.key), Span::raw(d.to_owned())])
                })
                .collect();
            f.render_widget(Paragraph::new(body), Rect { y: inner.y + 1, height: inner.height.saturating_sub(1), ..inner });
        }
        self.tab_hits = hits; // where the tabs are, for clicks
    }
}
