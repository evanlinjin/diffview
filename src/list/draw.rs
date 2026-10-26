//! Drawing dv list: its tabs, the cards, the status bar, and the keys and help popups.
use super::*;

impl List {
    /// Whether PR `n` conflicts with its base.
    fn conflicts(&self, n: u64) -> bool {
        self.extra.get(&n).is_some_and(|e| e.mergeable == "CONFLICTING")
    }

    /// A card's rows: 3 in a wide window (two zones), else 4, and a blank one between cards; on
    /// the todo and can close tabs, one more under the title (what needs you, why to close it).
    pub(super) fn card_rows(&self) -> usize {
        let lines = if self.wide { 3 } else { 4 };
        let more = match self.nav.tab {
            // A line under its title, or (as many rows) beside the card (`msg_column`).
            Tab::Todo | Tab::CanClose | Tab::Stalled => 1,
            _ => 0,
        };
        lines + more + 1
    }

    pub(super) fn draw(&mut self, f: &mut Frame) {
        let area = f.area();
        self.wide = area.width as usize >= TWO_ZONES;
        let msg_col = msg_column(area.width as usize);
        let rows = self.card_rows();
        let h = cards_fit(rows, area.height);
        // Keep the selection on screen.
        self.nav.keep(self.shown().len(), h);
        let sel = self.nav.sel;
        let (ui, cards) = (&self.ui, self.shown_cards());
        let shown: Vec<&Listed> = cards.iter().map(|c| c.0).collect();
        f.render_widget(Paragraph::new("").style(ui.base), area);

        let arrow = if self.rev { '↑' } else { '↓' };
        let open = self.prs.iter().filter(|p| p.open()).count();
        let names = [
            (Tab::Todo, format!("todo {}", self.todo_now())),
            (Tab::Open, format!("open {open}{}", if self.prs.len() >= github::LIST_MAX { "+" } else { "" })),
            (Tab::CanClose, format!("close {}", self.prs.iter().filter(|p| p.open() && self.closable.contains_key(&p.number)).count())),
            (Tab::Stalled, format!("stalled {}", self.prs.iter().filter(|p| p.open() && self.stalled(p).is_some()).count())),
            (Tab::Worktrees, format!("worktrees {}", self.local.len())),
            (Tab::Replacements, format!("drafts {}", self.reps.len())),
            (Tab::Done, self.done.as_ref().map_or("done".into(), |d| format!("done {}{}", d.len(), if d.len() >= self.done_limit { "+" } else { "" }))),
        ];
        let tabs: Vec<(String, bool)> = names.iter().map(|(t, n)| (n.clone(), self.nav.tab == *t)).collect();
        let hits = top_bar(f, ui, "PRs", &tabs, "I I the issues", sel, shown.len());
        let hits: Vec<_> = hits.into_iter().zip(names.iter().map(|n| n.0)).collect();

        // The base most PRs target goes without saying; others show as "base ← branch".
        let mut bases: HashMap<&str, usize> = HashMap::new();
        self.prs.iter().for_each(|p| *bases.entry(p.base.as_str()).or_default() += 1);
        let usual = bases.into_iter().max_by_key(|b| b.1).map(|b| b.0.to_owned()).unwrap_or_default();
        let cx = Cards {
            ui,
            num_w: shown.iter().map(|p| crate::replace::shown(p.number).chars().count()).max().unwrap_or(2),
            // (The todo tab's messages in a column of their own: the cards stop short of it.)
            width: match msg_col.filter(|_| self.nav.tab == Tab::Todo) {
                Some(w) => w,
                None => area.width as usize,
            },
            checks_loading: self.checks.is_some(),
            usual: &usual,
        };
        let lines: Vec<Line> = shown
            .iter()
            .enumerate()
            .skip(self.nav.top)
            .take(h)
            .flat_map(|(i, p)| {
                let item = cards[i].1;
                let rep_card = self.nav.tab == Tab::Replacements || item.is_some_and(|it| it.is_rep());
                let says = item.map(todo_says);
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
                        // (Published: its PR's conflicts, as any PR's.)
                        let published = self.rep_of.get(&p.number).and_then(|r| r.published);
                        let tags = published.filter(|n| self.conflicts(*n)).map(|_| vec!["conflicts".to_owned()]).unwrap_or_default();
                        Mark { marked: published.is_some_and(|n| self.marked.contains(&n)), claude: self.draft_state(p.number, "claude"), new: false, replaces, awaiting: 0, rank: None, tags }
                    }
                    false => {
                        // On the todo tab, what Claude could do there (`a a`), when it isn't at it.
                        let could = item.and_then(|it| it.claude.as_ref()).filter(|_| self.claude_doing(p.number).is_none()).map(batch::Task::could);
                        let claude = match (self.claude_state(p.number), could) {
                            (Some((s, ok)), Some(c)) => Some((format!("{s} · {c}"), ok)),
                            (None, Some(c)) => Some((format!("claude {c}"), true)),
                            (s, None) => s,
                        };
                        let awaiting = self.extra.get(&p.number).map_or(0, |e| e.awaiting);
                        // Beside the card: its place under its number, its tags on its title line.
                        let (rank, mut tags) = match says.as_ref().filter(|_| msg_col.is_some()) {
                            Some((tags, _)) => (Some(i + 1), tags.clone()),
                            None => (None, vec![]),
                        };
                        // Conflicts with its base: on every tab (drafts too), whatever's ranked.
                        if self.conflicts(p.number) && !tags.iter().any(|t| t == "conflicts") {
                            tags.push("conflicts".into());
                        }
                        Mark { marked: self.marked.contains(&p.number), claude, new: self.new.contains(&p.number), replaces: None, awaiting, rank, tags }
                    }
                };
                let local = if rep_card { None } else { self.local.get(&p.number) };
                let mut lines = card(p, &cx, i == sel, local, mark);
                match (item, msg_col) {
                    (Some(_), Some(card_w)) => lines = with_message(lines, says.as_ref().map_or("", |s| s.1.as_str()), &cx, card_w, area.width as usize, i == sel),
                    (Some(it), None) => {
                        lines.insert(1, todo_line(i + 1, it, &cx, i == sel)); // under the title
                    }
                    (None, _) => {}
                }
                // What Claude would close it for, under the title.
                if let Some(why) = self.closable.get(&p.number).filter(|_| self.nav.tab == Tab::CanClose) {
                    lines.insert(1, close_line(why, &cx, i == sel));
                }
                // How long its author hasn't answered, and whom.
                if let Some((days, waiting)) = self.stalled(p).filter(|_| self.nav.tab == Tab::Stalled) {
                    let why = match waiting {
                        Some((who, _)) => format!("@{} hasn't answered @{who} for {days} days (H asks whether they need help)", p.author),
                        None => format!("@{}'s draft, nothing on it for {days} days (H asks whether they need help)", p.author),
                    };
                    lines.insert(1, note_line("⏸", &why, &cx, i == sel));
                }
                lines.push(Line::raw("")); // room between cards
                lines
            })
            .collect();
        let typed = self.nav.query.trim().trim_start_matches('#').parse::<u64>().ok();
        let none = match (self.nav.tab, self.nav.query.is_empty(), typed) {
            (Tab::CanClose, true, _) => " nothing Claude would close: its reviews say when a PR's premise is wrong, or a competing PR does it better".into(),
            (Tab::Stalled, true, _) => format!(" nothing stalled: no author has left a review or a maintainer's comment unanswered, or a draft, for {} days or more (git config dv.stalledDays)", self.stalled_days),
            (Tab::Worktrees, true, _) => " no PR worktrees here yet: `dv N -w` (or dv list -w, then Enter) makes one".into(),
            (Tab::Replacements, true, _) => " no replacements yet: A A on a PR (or marked ones) has Claude draft one".into(),
            (Tab::Done, true, _) if self.done.is_none() => " fetching the merged and closed PRs…".into(),
            (Tab::Done, true, _) => " no merged or closed PRs".into(),
            (_, _, Some(n)) => format!(" #{n} isn't listed: Enter opens it anyway"),
            _ => String::new(),
        };
        if shown.is_empty() {
            f.render_widget(Paragraph::new(none).style(ui.base.patch(ui.stale)), Rect { y: area.y + 2, height: 1, ..area });
        }
        f.render_widget(Paragraph::new(lines), Rect { y: area.y + 1, height: (h * rows) as u16, ..area });

        // (The keys: in their popup, but beside the todo tab's word on each.)
        let idle = if self.nav.tab == Tab::Todo && msg_col.is_some() { " ? keys" } else { "" };
        // (`is:merged`, `is:closed` on the done tab alone: open PRs are neither; `ci:` not there:
        // its PRs' checks aren't fetched, any value said so.)
        let keys: &[(&str, &[&str])] = match self.nav.tab {
            Tab::Done => &[("is", IS), ("review", REVIEW), ("ci", &[])],
            _ => &[("is", &["draft"]), ("review", REVIEW), ("ci", CI)],
        };
        // (What it takes, from the same table: the done tab's own.)
        // (A key's values where they're few; else the key alone: `unknown` lists them when one's
        // wrong. Short enough for the bar.)
        let vals: Vec<String> = keys.iter().filter(|(_, v)| !v.is_empty()).map(|(k, v)| if v.len() <= 3 { format!("{k}:{}", v.join("|")) } else { format!("{k}:") }).collect();
        let fields = format!("author: label: base: branch: {}", vals.join(" "));
        let left = self.nav.status(&self.msg, &fields, idle, keys);
        // On the right, how it's ordered (on the todo tab, Claude's ranking: how old, what it
        // cost, what's changed since), and the flags PRs open with.
        let order = match self.nav.tab {
            Tab::Todo => self.todos.order_text(),
            Tab::Done => "most recently updated first".into(),
            _ => format!("by {} {arrow}", self.order.name()),
        };
        let flags = if self.flags.is_empty() { String::new() } else { format!("opens with {}", self.flags) };
        status_bar(f, ui, &left, &order, self.todos.auto, &flags);
        // (Not over the todo tab's message column: `?` shows them, the status bar says.)
        let column = self.nav.tab == Tab::Todo && msg_col.is_some();
        if !self.nav.help && !self.nav.filtering && !column {
            // The keys for this tab, bottom right, as dv's diff view shows them.
            let enter = match self.nav.tab {
                Tab::Todo => "see to it (opens the PR or replacement)".to_owned(),
                Tab::Worktrees => "review, in its worktree".to_owned(),
                Tab::CanClose => "review (Space a there drafts a closing comment)".to_owned(),
                Tab::Stalled => "review (H asks its author whether they need help)".to_owned(),
                Tab::Replacements => "review it, in its worktree".to_owned(),
                Tab::Done => "review it, as it was merged or closed".to_owned(),
                Tab::Open if self.flags.is_empty() => "review".to_owned(),
                Tab::Open => format!("review (dv N {})", self.flags),
            };
            let tab = match self.nav.tab {
                Tab::Todo => "open PRs",
                Tab::Open => "what Claude would close",
                Tab::CanClose => "stalled PRs",
                Tab::Stalled => "PRs with a worktree",
                Tab::Worktrees => "replacements",
                Tab::Replacements => "merged and closed PRs",
                Tab::Done => "what needs you",
            };
            let remove = if self.nav.tab == Tab::Replacements { "remove replacement" } else { "remove worktree" };
            let keys = [
                ("Enter", enter.as_str()),
                ("h l", tab),
                ("/", "filter"),
                ("o O", "sort / reverse"),
                ("R", "refresh (from GitHub)"),
                ("r r", "Claude ranks what needs you (a run)"),
                ("s", "defer it (or bring it back)"),
                ("p", "post what it waits on (a comment)"),
                ("m", "mark"),
                ("a a", if self.nav.tab == Tab::Todo { "Claude does its part (marked)" } else { "Claude reviews marked" }),
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
            let lines: Vec<(&str, &str)> = keys
                .iter()
                .filter(|(k, _)| match *k {
                    "D D" => matches!(self.nav.tab, Tab::Worktrees | Tab::Replacements), // (as the key does)
                    "o O" => !self.nav.tab.own_order(),
                    // (Done PRs are to read.)
                    "m" | "a a" | "A A" | "M M" | "W W" | "b b" => self.nav.tab != Tab::Done,
                    "r r" | "s" | "p" => self.nav.tab == Tab::Todo,
                    _ => true,
                })
                .map(|(k, d)| (*k, *d))
                .collect();
            // (Off the selected card: the list's cursor.)
            draw_keys(f, ui, &lines, Some(self.nav.card(rows, area)));
        }
        if self.nav.help {
            draw_help(f, ui, HELP, "dv list keys");
        }
        self.nav.tab_hits = hits; // where the tabs are, for clicks
    }
}

/// A list's help (`?`), centered: as dv's (sections, keys in their color, notes dim; two
/// columns when they fit).
pub(super) fn draw_help(f: &mut Frame, ui: &Ui, help: &str, title: &str) {
    let area = f.area();
    let body = crate::view::help_lines(help, ui, area.width.saturating_sub(4) as usize, "");
    let w = (body.iter().map(Line::width).max().unwrap_or(0) as u16 + 4).min(area.width);
    let h = (body.len() as u16 + 2).min(area.height);
    let r = Rect { x: (area.width - w) / 2, y: (area.height - h) / 2, width: w, height: h };
    let inner = crate::view::popup_frame(ui, f, r, title, "any key closes");
    f.render_widget(Paragraph::new(body), inner);
}

/// A list's top bar: which list it is (`title`), its tabs (the one shown lit), how to get to the
/// other list, and where the cursor is (`sel` of `n`) at the right end. Each tab's columns, for
/// clicks.
pub(super) fn top_bar(f: &mut Frame, ui: &Ui, title: &str, tabs: &[(String, bool)], other: &str, sel: usize, n: usize) -> Vec<std::ops::Range<u16>> {
    let area = f.area();
    let head = Rect { height: 1.min(area.height), ..area };
    // As the PR view's title bar: the darker of the status bar's and the headers' background;
    // its tabs, the one shown lit, the others dim.
    let hs = ui.base.patch(Style { bg: ui.bar, ..ui.header });
    let dim = hs.remove_modifier(ratatui::style::Modifier::BOLD).patch(Style { fg: ui.linenr.fg, ..Style::new() });
    let mut spans = vec![Span::styled(format!(" {title} "), hs.add_modifier(ratatui::style::Modifier::BOLD)), Span::styled(" ", hs)];
    let (mut hits, mut x) = (vec![], title.chars().count() as u16 + 3);
    for (name, on) in tabs {
        let s = Span::styled(format!(" {name} "), if *on { ui.menu_selected } else { dim });
        let w = s.width() as u16;
        hits.push(x..x + w);
        spans.extend([s, Span::styled(" ", hs)]);
        x += w + 1;
    }
    let pos = if n == 0 { "0 / 0 ".into() } else { format!("{} / {n} ", sel + 1) };
    // (The other list's key where there's room for it before the count, not cut under it.)
    let other = format!("   {other}");
    if x as usize + other.chars().count() + pos.chars().count() < head.width as usize {
        spans.push(Span::styled(other, dim));
    }
    f.render_widget(Paragraph::new(Line::from(spans)).style(hs), head);
    // Only where it goes (at the right end): over the whole line, its style would take the lit tab's.
    let w = (pos.chars().count() as u16).min(head.width);
    f.render_widget(Paragraph::new(pos).style(hs), Rect { x: head.right() - w, width: w, ..head });
    hits
}

/// A list's status bar: what's said (`left`: a message, the filter), and the list's state on
/// the right where there's room: how it's ordered, whether auto's on (it starts Claude runs:
/// in the warning color), and anything more (`more`).
pub(super) fn status_bar(f: &mut Frame, ui: &Ui, left: &str, order: &str, auto: bool, more: &str) {
    let area = f.area();
    let status = Rect { y: area.bottom().saturating_sub(1), height: 1.min(area.height), ..area };
    let bar = ui.base.patch(ui.status);
    let mut state = vec![
        Span::styled(format!("{order}  ·  "), bar),
        match auto {
            true => Span::styled("auto on", bar.patch(Style { fg: ui.warning.fg, ..Style::new() })),
            false => Span::styled("auto off", bar),
        },
    ];
    state.push(Span::styled(if more.is_empty() { "  ".to_owned() } else { format!("  ·  {more}  ") }, bar));
    let state = Line::from(state);
    // The message first: the state where there's room beside it.
    let (lw, sw) = (Line::from(left).width() as u16, state.width() as u16);
    let right = if lw + 2 + sw <= status.width { sw } else { 0 };
    if right > 0 {
        f.render_widget(Paragraph::new(state).alignment(Alignment::Right).style(bar), status);
    }
    f.render_widget(Paragraph::new(left.to_owned()).style(bar), Rect { width: status.width - right, ..status });
}

/// A list's keys for what's shown, between its top bar and status bar, as dv's view shows them
/// (`view::draw_hints`): off the cursor (`avoid`), where they cover the least.
pub(super) fn draw_keys(f: &mut Frame, ui: &Ui, keys: &[(&str, &str)], avoid: Option<Rect>) {
    let area = f.area();
    crate::view::draw_hints(f, ui, Rect { y: 1.min(area.height), height: area.height.saturating_sub(2), ..area }, "keys", keys, avoid);
}
