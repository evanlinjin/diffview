//! dv list's keys and mouse: what each does, and the help it shows.
use super::*;

pub(super) const HELP: &str = "
MOVE
  j k, Up Down         move
  gg ge                first / last PR
  Ctrl-d Ctrl-u        half a page down / up
  l h, gn gp           next / previous tab (todo, open, close, stalled, worktrees, drafts, done)
  H                    stalled tab: ask the author whether they need help (your editor first;
                       stalled: a review or maintainer's comment unanswered dv.stalledDays, 14)
  Enter                review the PR (in its worktree if it has one; q comes back)
  ?                    this help
  q, Ctrl-c            quit

FIND
  /                    filter: all terms match (Enter keeps it, Esc clears it)
                       text: number, title, author, branch, labels
                       author: label: base: branch:  (part of that field)
                       is:draft|merged|closed  review:approved|changes|required|none
                       ci:pass|fail|pending|none  (-term excludes)
  Esc                  clear the filter (none: quit)
  o O                  next sort order (created, number, size, updated) / reverse
  R                    refresh from GitHub (no Claude)

TODO
  r r                  Claude ranks what needs you (one run; the bar says its cost)
  s                    defer it: 1 an hour, 4 four hours, d a day, w a week, u until…
                       (s on a deferred one brings it back)
  p                    post what it waits on (dv reads the comment back, for everyone)
  a a                  Claude does its part (answers, updates or writes its review)

CLAUDE
  m                    mark the PR (again: unmark; marks are kept)
  U U                  unmark all
  a a                  Claude reviews the marked PRs (or this one), two at a time
  A A                  Claude drafts a replacement (unpublished, on the drafts tab)
  N N                  Claude drafts a new PR from what you write (unpublished)
  C                    auto: Claude does its part as things come in (git config dv.auto)

PR
  b b                  ask the author to rebase (a comment; todo waits for their push)
  W W                  approve its workflow runs (GitHub holds a first-timer's)
  M M                  merge with github-merge.py (dv.mergescript: your own copy)
  I I                  the issues (dv issues opens there)
  D D                  remove a worktree or draft (their tabs; not with unpushed work)";

impl List {
    /// Quit, asked first while Claude's at work here (`busy`): it'd stop with dv.
    fn quit(&mut self, armed: Option<(char, u64)>) -> Option<Do> {
        let quits = !self.busy() || self.again(armed, 'q', (), "Claude is still at work in dv (reviewing, triaging, drafting, ranking; here or in the issues): q again quits (what's under way is lost; queued reviews don't start)");
        quits.then_some(Do::Quit)
    }

    pub(super) fn key(&mut self, k: KeyEvent) -> Option<Do> {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let n = self.shown().len();
        self.msg.clear();
        let armed = self.confirm.take();
        // (Ctrl-c is q, wherever: asked first while Claude's at work here.)
        if ctrl && k.code == KeyCode::Char('c') {
            return self.quit(armed);
        }
        // Ctrl keys: only Ctrl-d and Ctrl-u (half a page); no Ctrl+letter is the letter. (It
        // calls off what waits for a key, as any other.)
        if ctrl {
            self.todos.deferring = None;
            self.nav.g = false;
            if matches!(k.code, KeyCode::Char('d' | 'u')) {
                self.nav.key(k, n);
            }
            return None;
        }
        if std::mem::take(&mut self.nav.help) {
            return None; // any key closes the help
        }
        if self.nav.filter_key(k) {
            return None;
        }
        // Esc after a key waiting for its second (M, D, s, …; g's: `Nav`): called off, as Helix's,
        // not quit.
        if k.code == KeyCode::Esc && armed.is_some() {
            return None;
        }
        // After `s`: how long to defer it for, or until what (any other key: not).
        match self.todos.deferring_key(k.code) {
            Some(After::Deferred(said)) => {
                self.msg = said;
                self.save_defer();
                return None;
            }
            Some(After::Until(key)) => return Some(Do::DeferUntil(key)),
            Some(After::Not(why)) => return (self.msg = why, None).1,
            Some(After::Off) => return None,
            None => {}
        }
        // Moving about, the tabs (`gp` before `p` on a tab), the filter, the help.
        if self.nav.key(k, n) {
            return None;
        }
        // The done tab's PRs are to read: what acts on a PR (marks, Claude, merging, deferring)
        // isn't for them.
        if self.nav.tab == Tab::Done && matches!(k.code, KeyCode::Char('m' | 'a' | 'A' | 'M' | 'b' | 'W')) {
            self.msg = "merged and closed PRs are to read here: Enter reviews one".into();
            return None;
        }
        match k.code {
            // A filter that's just a number opens that PR, listed or not (older, or closed).
            KeyCode::Enter => {
                let typed = self.nav.query.trim().trim_start_matches('#').parse().ok();
                return self.shown().get(self.nav.sel).map(|p| p.number).or(typed).map(Do::Open);
            }
            // Claude at work here (`busy`): it'd stop with dv. q asks first; Esc only says so (a
            // second Esc calls things off, it doesn't quit).
            KeyCode::Esc if !self.busy() => return Some(Do::Quit),
            KeyCode::Esc => self.msg = "Claude is still at work in dv: q, then q again, quits".into(),
            KeyCode::Char('q') => return self.quit(armed),
            KeyCode::Char('R') => return Some(Do::Refresh),
            KeyCode::Char('D') if self.nav.tab == Tab::Replacements => {
                let id = self.shown().get(self.nav.sel).map(|p| p.number)?;
                if self.again(armed, 'D', id, format!("D again removes {} (its worktree and record; its branch stays); any other key keeps it", crate::replace::label(id))) {
                    return Some(Do::RemoveReplacement(id));
                }
            }
            KeyCode::Char('H') if self.nav.tab == Tab::Stalled => return Some(Do::Nudge(self.shown().get(self.nav.sel)?.number)),
            KeyCode::Char('D') if self.nav.tab == Tab::Worktrees => {
                let p = self.shown().get(self.nav.sel).map(|p| p.number)?;
                let (dir, _) = self.local.get(&p)?;
                if self.again(armed, 'D', p, format!("D again removes {} (its branch stays); any other key keeps it", dir.display())) {
                    return Some(Do::Remove(p));
                }
            }
            KeyCode::Char('s') if self.nav.tab == Tab::Todo => self.defer_key(),
            KeyCode::Char('p') if self.nav.tab == Tab::Todo => {
                let it = self.selected_item()?;
                let rep = it.is_rep();
                match it.deferred.and_then(|d| d.cond).filter(|_| !rep) {
                    Some(c) => return Some(Do::PostWait(it.number, format!("Waiting on: {}.", c.says()))),
                    None => self.msg = "p posts what an item is deferred until (s then u) on its PR: this one isn't".into(),
                }
            }
            KeyCode::Char('r') if self.nav.tab == Tab::Todo => {
                let offer = match self.todos.rank_offer("PR(s)") {
                    Ok(o) => o,
                    Err(why) => return (self.msg = why.into(), None).1,
                };
                if self.again(armed, 'r', (), offer) {
                    self.rank_now(true);
                    return None;
                }
            }
            KeyCode::Char('b') => {
                let p = self.selected_pr()?;
                if self.again(armed, 'b', p, format!("b again: ask #{p}'s author for a rebase (a comment; your editor opens first); any other key doesn't")) {
                    return Some(Do::AskRebase(p));
                }
            }
            KeyCode::Char('N') => {
                if self.again(armed, 'N', (), "N again: Claude drafts a new PR, unpublished (your editor opens first: what it should do; issues by #N); any other key doesn't") {
                    return Some(Do::NewPr);
                }
            }
            KeyCode::Char('I') => {
                if self.again(armed, 'I', (), "I again: the issues (I I there comes back)") {
                    return Some(Do::Issues);
                }
            }
            KeyCode::Char('W') => {
                let p = self.selected_pr()?;
                let Some(head) = self.extra.get(&p).filter(|e| e.awaiting > 0).map(|e| e.head_oid.clone()) else {
                    self.msg = format!("#{p} has no workflow runs waiting for approval");
                    return None;
                };
                if self.again(armed, 'W', p, format!("W again: approve the workflow runs waiting on #{p} (they run then); any other key doesn't")) {
                    self.msg = match crate::gh::approve_runs(&head) {
                        Ok(k) => {
                            if let Some(e) = self.extra.get_mut(&p) {
                                e.awaiting = 0;
                            }
                            self.find_todo();
                            format!("approved {k} workflow run{} on #{p}: they run now", if k == 1 { "" } else { "s" })
                        }
                        Err(e) => format!("not approved: {e:#}"),
                    };
                    return None;
                }
            }
            KeyCode::Char('M') => {
                let p = self.selected_pr()?;
                // (Claude on it, from here, queued too, or any dv: merged once it's done.)
                if let Some(w) = self.claude_doing(p) {
                    return (self.msg = format!("Claude is {w} on #{p}: merge it once it's done"), None).1;
                }
                let wt = if self.local.contains_key(&p) { " (its worktree goes first, if nothing in it is unpushed)" } else { "" };
                if self.again(armed, 'M', p, format!("M again: merge #{p} with github-merge.py{wt}; any other key doesn't")) {
                    return Some(Do::Merge(p));
                }
            }
            KeyCode::Char('m') => {
                // (Marked: unmarked, open or not. Else marked as a PR's acted on: open.)
                if !self.card_pr().is_some_and(|p| self.marked.remove(&p)) {
                    let p = self.selected_pr()?;
                    self.marked.insert(p);
                }
                self.nav.step(true, n); // on to the next, to mark a run
                self.msg = format!("{} marked: a a has Claude do its part on them", self.marked.len());
            }
            // Auto on / off: Claude runs, drafts marked ready, rankings, without you.
            KeyCode::Char('C') => {
                self.todos.toggle_auto();
                self.msg = match self.todos.auto {
                    true => "auto on: Claude does its part as things come in (fixes your CI, reviews, answers you, drafts the comment closing a PR it'd close, drafts a replacement of a low-effort PR worth keeping; Claude runs, at your cost), green drafts are marked ready, and it ranks the list again as it changes".into(),
                    false => "auto off: a a has Claude do its part, r r ranks, Space g in a PR marks it ready".into(),
                };
                self.find_todo();
            }
            // All the marks off (kept between runs, they pile up): asked first.
            KeyCode::Char('U') => {
                if self.marked.is_empty() {
                    return (self.msg = "nothing marked".into(), None).1;
                }
                if self.again(armed, 'U', (), format!("U again unmarks all {} marked PRs; any other key doesn't", self.marked.len())) {
                    self.msg = format!("unmarked {}", self.marked.len());
                    self.marked.clear();
                    return None;
                }
            }
            KeyCode::Char('a') => {
                let (mut prs, mut skipped) = match self.marked.is_empty() {
                    true => (self.selected_pr().into_iter().collect(), String::new()),
                    false => self.marked_open(),
                };
                // (Claude at it, from here or any dv: kept marked, said.)
                let at_it: Vec<String> = prs.iter().filter(|&&n| self.claude_doing(n).is_some()).map(|n| format!("#{n}")).collect();
                if !at_it.is_empty() {
                    skipped += &format!(" (Claude's at {} already)", at_it.join(", "));
                }
                prs.retain(|&n| self.claude_doing(n).is_none());
                prs.sort();
                if prs.is_empty() {
                    if !skipped.is_empty() {
                        self.msg = format!("nothing to do{skipped}");
                    }
                    return None;
                }
                let which = match self.nav.tab {
                    Tab::Todo => prs.iter().map(|&n| format!("{} #{n}", self.todo_task(n).doing())).collect::<Vec<_>>().join(", "),
                    _ => format!("reviews {}", prs.iter().map(|n| format!("#{n}")).collect::<Vec<_>>().join(" ")),
                };
                if self.again(armed, 'a', (&prs, &which), format!("a again: Claude {which} in the background, each in its worktree (<repo>-N); your editor opens first, for any instructions{skipped}")) {
                    return Some(Do::Review(prs));
                }
            }
            KeyCode::Char('A') => {
                if self.group_rx.is_some() {
                    return (self.msg = "Claude is still grouping the marked PRs: once it's done".into(), None).1;
                }
                let (mut prs, skipped): (Vec<u64>, String) = match self.marked.is_empty() {
                    true => match self.selected() {
                        Some((id, true)) => (vec![id], String::new()), // a draft: drafted again
                        _ => (self.selected_pr().into_iter().collect(), String::new()),
                    },
                    false => self.marked_open(),
                };
                prs.sort();
                if prs.is_empty() && !skipped.is_empty() {
                    return (self.msg = format!("nothing to draft{skipped}"), None).1;
                }
                // (What's busy or published is said per draft, once grouped: `draft_groups`,
                // which drafts what's asked as `replace::target` has it; said so here.)
                let drafts = crate::replace::target(&prs);
                let id = *drafts.first()?;
                let which = drafts.iter().map(|&n| crate::replace::shown(n)).collect::<Vec<_>>().join(" ");
                let again = if self.rep_of.contains_key(&id) { " (again: it has one, which Claude carries on from)" } else { "" };
                let ask = match crate::replace::one_draft(&prs) {
                    true => format!("A again: Claude drafts {}{again}, unpublished; your editor opens first, for instructions", crate::replace::label(id)),
                    false => format!("A again: Claude suggests how to group {which} into replacements (you decide, in your editor), then drafts them, unpublished"),
                };
                if self.again(armed, 'A', &prs, ask + &skipped) {
                    return Some(Do::Replace(prs));
                }
            }
            KeyCode::Char('o') if !self.nav.tab.own_order() => self.sort(self.order.next(), self.rev),
            KeyCode::Char('O') if !self.nav.tab.own_order() => self.sort(self.order, !self.rev),
            _ => {}
        }
        None
    }

    /// A click selects a card, a double-click opens it (`true`: as Enter); a click on a tab
    /// switches to it; the wheel moves the selection.
    pub(super) fn mouse(&mut self, m: event::MouseEvent) -> bool {
        use event::{MouseButton, MouseEventKind};
        // (A scroll or click calls off what waits for a key, as a key does: what was to be asked
        // again, and its ask; `s`'s how long; `g`'s second key.)
        if matches!(m.kind, MouseEventKind::ScrollDown | MouseEventKind::ScrollUp | MouseEventKind::Down(_)) {
            if self.confirm.take().is_some() || self.todos.deferring.take().is_some() {
                self.msg.clear();
            }
            self.nav.g = false;
        }
        let n = self.shown().len();
        match m.kind {
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => self.nav.step(m.kind == MouseEventKind::ScrollDown, n),
            MouseEventKind::Down(MouseButton::Left) if !self.nav.help => return self.nav.click(m.column, m.row, n, self.card_rows()),
            _ => {}
        }
        false
    }
}
