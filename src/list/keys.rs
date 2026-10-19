//! dv list's keys and mouse: what each does, and the help it shows.
use super::*;

pub(super) const HELP: &str = "\
  j k, Up Down         move
  gg ge                first / last PR
  Ctrl-d Ctrl-u        half a page down / up
  gn gp                todo (what needs you, ranked) / open PRs / can close (what Claude's
                       reviews would close, and why: its premise is wrong, or a competing PR
                       does it better) / PRs with a local worktree (<repo>-N, as dv N -w
                       makes) / replacements (Claude's drafts, unpublished)
                       On todo: a PR or replacement waiting on you (Claude's review or answers,
                       your review requested or none yet, pushed since you reviewed, ready to
                       merge, a replacement to publish or close for), in the order Claude puts
                       them (once you have it rank them: r r), with why and what next; a a
                       there has Claude do its part on one (answer your replies, update its
                       review, or review it)
  Enter                review the PR (with dv list's flags; from the worktrees tab, in its
                       worktree as it was made); q there comes back here
  /                    filter (Enter keeps it, Esc clears it); terms must all match:
                         text            number, title, author, branch, labels
                         author: label: base: branch:   part of that field
                         is:draft  review:approved|changes|required|none
                         ci:pass|fail|pending|none     -term excludes
  Esc                  clear the filter
  o O                  next sort order (created, the default; number, size, updated) / reverse
  m                    mark the PR (again: unmark), for Claude to review; marks are kept, for
                       going through them a PR at a time (quitting dv list too)
  U U                  unmark all
  C                    auto on / off (top bar; git config dv.auto): Claude does its part as
                       things come in, CI-green drafts go ready, replacements close what they
                       replace, the todo list is ranked again
  a a                  Claude reviews the marked PRs (or this one) in the background, two at a
                       time, each in its worktree (<repo>-N, made if need be); cards show how
                       it's going, and claude ✓ once a PR has a review
  A A                  Claude drafts a replacement for this PR, on a branch of its own in
                       <repo>-replace-N, unpublished, for you to review first (replacements
                       tab); for marked PRs, it first suggests which go together (one
                       replacement each group), which you edit; then your instructions
  b b                  ask the PR's author for a rebase: a comment saying it needs one (in your
                       editor first); on the todo tab it's then deferred until it changes
                       (their push brings it back)
  M M                  merge the PR with github-merge.py (Bitcoin Core's script, built into dv;
                       git config dv.mergescript <path> runs your own copy), in the main
                       checkout: its worktree goes first, unless something in it is unpushed;
                       in a zellij pane of its own, or in this terminal
  N N                  Claude drafts a new PR: your editor opens for what it should do (issues
                       by #N, which Claude reads); on a branch of its own (dv/new-K, in
                       <repo>-new-K), unpublished, on the drafts tab: Enter reviews it, S publishes
  W W                  approve the workflow runs waiting on the PR (GitHub holds a first-time
                       contributor's until a maintainer does): its card says '⏸ N to approve'
  D D                  worktrees tab: remove the PR's worktree (not with local changes in it;
                       its branch stays); replacements tab: remove the replacement (the same)
  R                    refresh (from GitHub; no Claude)
  s                    todo: defer the item (then 1 an hour, 4 four hours, d a day, w a week,
                       or u until something happens: #N merged, CI green, an approval, ...):
                       to the bottom till then, or till what it needs changes; s on a
                       deferred one brings it back. p posts what it waits on as a comment
                       (Waiting on: #N is merged.), which dv reads back for everyone: a
                       comment like that (by you or anyone with write access) defers the PR
                       till then, until a later Unblocked
  r r                  todo: Claude ranks what needs you (one Claude run: the header says what
                       the last cost and what's changed since, to judge if it's worth it)
  ?                    this help
  q, Ctrl-c            quit";

impl List {
    pub(super) fn key(&mut self, k: KeyEvent) -> Option<Do> {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && k.code == KeyCode::Char('c') {
            return Some(Do::Quit);
        }
        let n = self.shown().len();
        self.msg.clear();
        let armed = self.confirm.take();
        if std::mem::take(&mut self.help) {
            return None; // any key closes the help
        }
        if self.filtering {
            match k.code {
                KeyCode::Esc => (self.query.clear(), self.filtering = false).1,
                KeyCode::Enter => self.filtering = false,
                KeyCode::Backspace => _ = self.query.pop(),
                KeyCode::Char(c) => self.query.push(c),
                _ => {}
            }
            self.sel = 0;
            return None;
        }
        let half = (self.height() / 2).max(1);
        let g = std::mem::take(&mut self.pending_g);
        // After `s`: how long to defer it for, or until what (any other key: not).
        if let Some(key) = self.deferring.take() {
            match k.code {
                KeyCode::Char(c @ ('1' | '4' | 'd' | 'w')) => self.defer_for(&key, c),
                KeyCode::Char('u') => return Some(Do::DeferUntil(key)),
                _ => self.msg = "not deferred".into(),
            }
            return None;
        }
        // `gn` / `gp`: the next / previous tab, as in dv (before `p` and such on a tab).
        if let (true, KeyCode::Char(c @ ('n' | 'p'))) = (g, k.code) {
            let tab = if c == 'n' { self.tab.next() } else { self.tab.prev() };
            (self.tab, self.sel, self.top) = (tab, 0, 0);
            return None;
        }
        match k.code {
            // A filter that's just a number opens that PR, listed or not (older, or closed).
            KeyCode::Enter => {
                let typed = self.query.trim().trim_start_matches('#').parse().ok();
                return self.shown().get(self.sel).map(|p| p.number).or(typed).map(Do::Open);
            }
            KeyCode::Esc if !self.query.is_empty() => (self.query.clear(), self.sel = 0).1,
            // Reviews running: they'd stop with dv (the ones started finish; queued ones don't start).
            KeyCode::Char('q') | KeyCode::Esc => {
                let running = self.batch.values().any(batch::Status::running);
                if !running || self.again(armed, ('q', 0), "Claude is still reviewing: q again quits (reviews under way finish; queued ones don't start)") {
                    return Some(Do::Quit);
                }
            }
            KeyCode::Char('R') => return Some(Do::Refresh),
            KeyCode::Char('D') if self.tab == Tab::Replacements => {
                let id = self.shown().get(self.sel).map(|p| p.number)?;
                if self.again(armed, ('D', id), format!("D again removes {} (its worktree and record; its branch stays); any other key keeps it", crate::replace::label(id))) {
                    return Some(Do::RemoveReplacement(id));
                }
            }
            KeyCode::Char('m' | 'a' | 'A') if self.tab == Tab::Replacements => {
                self.msg = "a draft: Enter opens it in dv, D D removes it (N N drafts a new PR)".into();
            }
            KeyCode::Char('D') if self.tab == Tab::Worktrees => {
                let p = self.shown().get(self.sel).map(|p| p.number)?;
                let (dir, _) = self.local.get(&p)?;
                if self.again(armed, ('D', p), format!("D again removes {} (its branch stays); any other key keeps it", dir.display())) {
                    return Some(Do::Remove(p));
                }
            }
            KeyCode::Char('s') if self.tab == Tab::Todo => self.defer_key(),
            KeyCode::Char('p') if self.tab == Tab::Todo => {
                let it = self.selected_item()?;
                match it.deferred.and_then(|d| d.cond).filter(|_| !it.rep) {
                    Some(c) => return Some(Do::PostWait(it.number, format!("Waiting on: {}.", c.says()))),
                    None => self.msg = "p posts what an item is deferred until (s then u) on its PR: this one isn't".into(),
                }
            }
            KeyCode::Char('r') if self.tab == Tab::Todo => {
                if self.rank_rx.is_some() {
                    return (self.msg = "Claude is ranking already".into(), None).1;
                }
                if self.again(armed, ('r', 0), self.rank_offer()) {
                    self.rank_now();
                    return None;
                }
            }
            KeyCode::Char('b') => {
                let p = self.selected_pr()?;
                if self.again(armed, ('b', p), format!("b again: ask #{p}'s author for a rebase (a comment; your editor opens first); any other key doesn't")) {
                    return Some(Do::AskRebase(p));
                }
            }
            KeyCode::Char('N') => {
                if self.again(armed, ('N', 0), "N again: Claude drafts a new PR, unpublished (your editor opens first: what it should do; issues by #N); any other key doesn't") {
                    return Some(Do::NewPr);
                }
            }
            KeyCode::Char('W') => {
                let p = self.selected_pr()?;
                let Some(head) = self.extra.get(&p).filter(|e| e.awaiting > 0).map(|e| e.head_oid.clone()) else {
                    self.msg = format!("#{p} has no workflow runs waiting for approval");
                    return None;
                };
                if self.again(armed, ('W', p), format!("W again: approve the workflow runs waiting on #{p} (they run then); any other key doesn't")) {
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
                let wt = if self.local.contains_key(&p) { " (its worktree goes first, if nothing in it is unpushed)" } else { "" };
                if self.again(armed, ('M', p), format!("M again: merge #{p} with github-merge.py{wt}; any other key doesn't")) {
                    return Some(Do::Merge(p));
                }
            }
            KeyCode::Char('m') => {
                let state = self.shown().get(self.sel).map(|p| p.state.clone())?;
                let p = self.selected_pr()?;
                if !matches!(state.as_str(), "OPEN" | "") && !self.marked.contains(&p) {
                    return (self.msg = format!("#{p} isn't open: nothing to mark it for"), None).1;
                }
                if !self.marked.remove(&p) {
                    self.marked.insert(p);
                }
                self.sel = (self.sel + 1).min(n.saturating_sub(1)); // on to the next, to mark a run
                self.msg = format!("{} marked: a asks Claude to review them", self.marked.len());
            }
            // Auto on / off: Claude runs, drafts marked ready, rankings, without you.
            KeyCode::Char('C') => {
                self.auto = !self.auto;
                _ = crate::diff::git(&["config", "dv.auto", if self.auto { "true" } else { "false" }]);
                self.msg = match self.auto {
                    true => "auto on: Claude does its part as things come in (fixes your CI, reviews, answers you; Claude runs, at your cost), green drafts are marked ready, and it ranks the list again as it changes".into(),
                    false => "auto off: a a has Claude do its part, r r ranks, Space g in a PR marks it ready".into(),
                };
                self.find_todo();
            }
            // All the marks off (kept between runs, they pile up): asked first.
            KeyCode::Char('U') => {
                if self.marked.is_empty() {
                    return (self.msg = "nothing marked".into(), None).1;
                }
                if self.again(armed, ('U', 0), format!("U again unmarks all {} marked PRs; any other key doesn't", self.marked.len())) {
                    self.msg = format!("unmarked {}", self.marked.len());
                    self.marked.clear();
                    return None;
                }
            }
            KeyCode::Char('a') => {
                let mut prs: Vec<u64> = match self.marked.is_empty() {
                    true => self.selected_pr().into_iter().collect(),
                    false => self.marked.iter().copied().collect(),
                };
                prs.retain(|n| !self.batch.get(n).is_some_and(batch::Status::running));
                prs.sort();
                if prs.is_empty() {
                    return None;
                }
                let which = match self.tab {
                    Tab::Todo => prs.iter().map(|&n| format!("{} #{n}", self.todo_task(n).doing())).collect::<Vec<_>>().join(", "),
                    _ => format!("reviews {}", prs.iter().map(|n| format!("#{n}")).collect::<Vec<_>>().join(" ")),
                };
                if self.again(armed, ('a', 0), format!("a again: Claude {which} in the background, each in its worktree (<repo>-N); your editor opens first, for any instructions")) {
                    return Some(Do::Review(prs));
                }
            }
            KeyCode::Char('A') => {
                let mut prs: Vec<u64> = match self.marked.is_empty() {
                    true => self.selected().map(|s| vec![s.0]).unwrap_or_default(), // (on a draft: its PR's, drafted again)
                    false => self.marked.iter().copied().collect(),
                };
                prs.sort();
                let id = *prs.first()?;
                if self.drafting.get(&id).is_some_and(batch::Status::running) || self.drafting_elsewhere.contains(&id) {
                    return (self.msg = format!("Claude is already drafting {}", crate::replace::label(id)), None).1;
                }
                let which = prs.iter().map(|n| format!("#{n}")).collect::<Vec<_>>().join(" ");
                let again = if self.rep_of.contains_key(&id) { " (again: it has one, which Claude carries on from)" } else { "" };
                let ask = match prs.len() {
                    1 => format!("A again: Claude drafts a replacement for {which}{again}, unpublished; your editor opens first, for instructions"),
                    _ => format!("A again: Claude suggests how to group {which} into replacements (you decide, in your editor), then drafts them, unpublished"),
                };
                if self.again(armed, ('A', 0), ask) {
                    return Some(Do::Replace(prs));
                }
            }
            KeyCode::Char('?') => self.help = true,
            KeyCode::Char('/') => self.filtering = true, // edits the current filter
            KeyCode::Char('o') => self.sort(self.order.next(), self.rev),
            KeyCode::Char('O') => self.sort(self.order, !self.rev),
            KeyCode::Char('d') if ctrl => self.sel = (self.sel + half).min(n.saturating_sub(1)),
            KeyCode::Char('u') if ctrl => self.sel = self.sel.saturating_sub(half),
            KeyCode::Char('j') | KeyCode::Down => self.sel = (self.sel + 1).min(n.saturating_sub(1)),
            KeyCode::Char('k') | KeyCode::Up => self.sel = self.sel.saturating_sub(1),
            KeyCode::Char('g') if g => self.sel = 0,
            KeyCode::Char('e') if g => self.sel = n.saturating_sub(1),
            KeyCode::Char('g') => self.pending_g = true,
            _ => {}
        }
        None
    }

    /// A click selects a card, a double-click opens it (`true`: as Enter); a click on a tab
    /// switches to it; the wheel moves the selection.
    pub(super) fn mouse(&mut self, m: event::MouseEvent) -> bool {
        use event::{MouseButton, MouseEventKind};
        let n = self.shown().len();
        match m.kind {
            MouseEventKind::ScrollDown => self.sel = (self.sel + 1).min(n.saturating_sub(1)),
            MouseEventKind::ScrollUp => self.sel = self.sel.saturating_sub(1),
            MouseEventKind::Down(MouseButton::Left) if !self.help && m.row == 0 => {
                if let Some(&(_, tab)) = self.tab_hits.iter().find(|(x, _)| x.contains(&m.column)) {
                    (self.tab, self.sel, self.top) = (tab, 0, 0);
                }
            }
            MouseEventKind::Down(MouseButton::Left) if !self.help => {
                let i = self.top + (m.row as usize - 1) / self.card_rows(); // cards start under the header
                if i >= n {
                    return false;
                }
                let double = self.last_click.is_some_and(|(at, j)| j == i && at.elapsed() < Duration::from_millis(400));
                self.last_click = Some((std::time::Instant::now(), i));
                self.sel = i;
                if double {
                    self.last_click = None;
                    return true;
                }
            }
            _ => {}
        }
        false
    }
}
