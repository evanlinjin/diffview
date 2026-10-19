//! A PR's (or a replacement's) card in the list: two lines, and what they say about Claude.

use super::*;

/// A replacement's card: its drafted title, its branch, and its size against its base (from
/// git: nothing's on GitHub yet).
pub(super) fn rep_card(r: &crate::replace::Record, root: &std::path::Path) -> Listed {
    let branch = crate::replace::branch(r.id);
    let git = |args: &[&str]| crate::diff::git(&[&["-C", &root.to_string_lossy()], args].concat()).unwrap_or_default();
    // " 3 files changed, 10 insertions(+), 2 deletions(-)"
    let stat = git(&["diff", "--shortstat", &format!("{}...{branch}", crate::diff::remote_branch(&r.base))]);
    let count = |what: &str| -> u64 {
        stat.split(',').find(|p| p.contains(what)).and_then(|p| p.split_whitespace().next()?.parse().ok()).unwrap_or(0)
    };
    let title = match r.title.is_empty() {
        true if crate::replace::is_new(r.id) => format!("(not drafted yet: {})", r.task.lines().next().unwrap_or("")),
        true => format!("(replacement of #{}, not drafted yet)", r.id),
        false => r.title.clone(),
    };
    Listed {
        number: r.id,
        title,
        base: r.base.clone(),
        branch,
        state: "OPEN".into(),
        updated: git(&["log", "-1", "--format=%cI", &crate::replace::branch(r.id)]).trim().to_owned(),
        files: count("file"),
        additions: count("insertion"),
        deletions: count("deletion"),
        ..Listed::default()
    }
}

/// A card's own marks: picked for review (`m`), and how Claude's review of it stands.
pub(super) struct Mark {
    /// Something on it you haven't seen.
    pub(super) new: bool,
    /// A replacement's card: what it replaces (said instead of an author).
    pub(super) replaces: Option<String>,
    pub(super) marked: bool,
    pub(super) claude: Option<(String, bool)>,
    /// Workflow runs on it waiting for a maintainer's approval.
    pub(super) awaiting: usize,
    /// On the todo tab, with Claude's word in a column of its own: its place in the order
    /// (under its number), and what holds it up, in short (on the title line, after `[new]`).
    pub(super) rank: Option<usize>,
    pub(super) tags: Vec<String>,
}

impl List {
    /// What a card says about Claude's review: ("claude ✓", good) or ("claude: reviewing…", …).
    pub(super) fn claude_state(&self, n: u64) -> Option<(String, bool)> {
        // A PR being replaced says so too.
        let id = self.rep_of.values().find(|r| r.replaces.iter().any(|o| o.number == n)).map(|r| r.id);
        let replaced = id.or(self.drafting.contains_key(&n).then_some(n)).and_then(|id| self.draft_state(id, "replacement"));
        match (self.review_state(n), replaced) {
            (Some((a, x)), Some((b, y))) => Some((format!("{a} · {b}"), x && y)),
            (a, b) => a.or(b),
        }
    }

    pub(super) fn review_state(&self, n: u64) -> Option<(String, bool)> {
        match self.batch.get(&n) {
            Some(batch::Status::Queued) => Some(("claude: queued".into(), true)),
            Some(batch::Status::Preparing) => Some(("claude: checking out…".into(), true)),
            Some(batch::Status::Reviewing) => Some((format!("claude: {}…", self.batch_task.get(&n).map_or("reviewing", batch::Task::doing)), true)),
            // A run from another dv (a PR's view, another dv list).
            _ if self.running.contains_key(&n) => Some((format!("claude: {}…", self.running[&n]), true)),
            // Why, in short (the status bar said it in full): `a a` asks again.
            // All of it (the card cuts it where the window ends).
            Some(batch::Status::Failed(e)) => Some((format!("claude failed: {}", e.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" · ")), false)),
            _ if self.reviewed.contains(&n) => Some(("claude ✓".into(), true)),
            _ => None,
        }
    }
}

/// What every card on screen shares.
pub(super) struct Cards<'a> {
    pub(super) ui: &'a Ui,
    pub(super) num_w: usize,
    pub(super) width: usize,
    pub(super) checks_loading: bool,
    /// The base most PRs target, left unsaid.
    pub(super) usual: &'a str,
}

/// Wide enough for a card's two zones: what it is on the left, how it stands on the right.
pub(super) const TWO_ZONES: usize = 110;
/// The right zone's width, and the most a card's content spans (wider, the right zone would
/// drift away from what it's about).
pub(super) const RIGHT: usize = 30;
pub(super) const CARD_MAX: usize = 140;

/// A PR's card, related things together. Wide (`TWO_ZONES`), three lines in two zones: on the
/// left what it is (number and title, with a draft / closed badge and `[new]`; who and how big,
/// labels; branch and its worktree here), on the right how it stands (CI and its age; review;
/// Claude), a column that lines up from card to card. Narrower, four lines: those, with how it
/// stands on a line of its own. Each line is cut at its width rather than wrapped.
/// The mark's column on a card's first line (` ● ` when marked).
pub(super) const MARK_W: usize = 3;

pub(super) fn card(p: &Listed, cx: &Cards, selected: bool, local: Option<&(PathBuf, Option<String>)>, mark: Mark) -> Vec<Line<'static>> {
    let (ui, num_w, width, usual) = (cx.ui, cx.num_w, cx.width, cx.usual);
    let row = card_row(ui, selected);
    // Text keeps its colors on the selected card too (only the background shows it).
    let tint = |st: Style| row.patch(Style { bg: None, ..st });
    let dim = tint(ui.stale);
    let sp = |t: String, st| Span::styled(t, st);
    let gap = || sp("   ".into(), row);
    // The selected card has a bar down its left edge.
    let bar = || sp(if selected { "▌".into() } else { " ".into() }, tint(ui.key));
    // Under the title: past the bar, the mark's column, the number and its gap.
    let indent = || sp(" ".repeat(MARK_W + num_w + 2), row);

    // What it is: number and title, badges.
    let badge = match (p.state.as_str(), p.draft) {
        ("MERGED", _) => Some(("MERGED", "8957e5")),
        ("CLOSED", _) => Some(("CLOSED", "da3633")),
        (_, true) => Some(("DRAFT", "6e7681")),
        _ => None,
    };
    let lead = vec![
        // A column of its own for the mark (`m`): marked cards line up down the list.
        sp(if mark.marked { " ● ".into() } else { "   ".into() }, tint(ui.key).add_modifier(ratatui::style::Modifier::BOLD)),
        sp(format!("{:>w$}  ", crate::replace::shown(p.number), w = num_w - 1), tint(ui.key)),
    ];
    let lead_w: usize = lead.iter().map(Span::width).sum();
    // The title is what's cut for its badges and tags, never the mark or number (the tags cut,
    // when they're wider still, where the line ends).
    let title_in = |room: usize, tags: Vec<Span<'static>>| {
        let mut v = lead.clone();
        v.extend(cut_spans(vec![sp(p.title.clone(), row.add_modifier(ratatui::style::Modifier::BOLD))], room.saturating_sub(lead_w).max(8)));
        v.extend(tags);
        v
    };
    // Badges stay in view: the title is what's cut for them.
    let mut tags = vec![];
    if let Some((text, color)) = badge {
        tags.extend([sp("  ".into(), row), sp(format!(" {text} "), crate::view::chip(color))]);
    }
    if mark.new {
        tags.extend([sp("  ".into(), row), sp("[new]".into(), tint(ui.key).add_modifier(ratatui::style::Modifier::BOLD))]);
    }
    if !mark.tags.is_empty() {
        tags.extend([sp("  ".into(), row), sp(format!("[{}]", mark.tags.join(" · ")), tint(ui.warning))]);
    }
    let age = sp(gh::ago(&p.updated), dim);
    // Who, and how big; labels.
    let who = mark.replaces.clone().unwrap_or_else(|| format!("@{}", p.author));
    // (Its place in the order, when it's said here: under the number.)
    let first = match mark.rank {
        Some(r) => sp(format!("{}{:>w$}  ", " ".repeat(MARK_W), format!("{r}."), w = num_w - 1), tint(ui.key)),
        None => indent(),
    };
    let mut people = vec![first, sp(who, dim), gap()];
    people.extend([
        sp(format!("+{}", p.additions), tint(ui.plus)),
        sp(" ".into(), row),
        sp(format!("−{}", p.deletions), tint(ui.minus)),
        sp(format!(" in {} file{}", p.files, if p.files == 1 { "" } else { "s" }), dim),
    ]);
    for (k, (name, color)) in p.labels.iter().enumerate() {
        people.extend([sp(if k == 0 { "   " } else { " " }.into(), row), sp(format!(" {name} "), crate::view::chip(color))]);
    }
    // Where its code is.
    let branch = match p.base == usual {
        true => p.branch.clone(),
        false => format!("{} ← {}", p.base, p.branch),
    };
    let mut place = vec![indent(), sp(branch, dim)];
    if let Some((dir, branch)) = local {
        let name = dir.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
        let on = branch.as_deref().map_or("detached".to_owned(), |b| format!("on {}", cut(b, 24)));
        place.extend([gap(), sp(format!("⌂ {name} ({on})"), tint(ui.key))]);
    }

    // How it stands: CI, review, Claude.
    let ci: Vec<Span<'static>> = match p.checks {
        Some((0, 0, 0)) => vec![],
        Some((pass, fail, pending)) => {
            let mut v = vec![sp("CI ".into(), dim)];
            let counts = [(pass, '✓', ui.plus), (fail, '✗', ui.error), (pending, '●', ui.warning)];
            for (k, &(n, m, st)) in counts.iter().filter(|c| c.0 > 0).enumerate() {
                v.extend([sp(if k == 0 { "" } else { " " }.into(), row), sp(format!("{m}{n}"), tint(st))]);
            }
            // Optional ones failing (not required by its base): said, apart, dim.
            if p.optional_failing > 0 {
                v.push(sp(format!(" · ✗{} optional", p.optional_failing), dim));
            }
            v
        }
        None if cx.checks_loading && mark.replaces.is_none() => vec![sp("CI …".into(), dim)],
        None => vec![],
    };
    // Runs that won't start until approved: said first (`W W` approves them).
    let ci = match mark.awaiting {
        0 => ci,
        n => [vec![sp(format!("⏸ {n} to approve"), tint(ui.warning))], if ci.is_empty() { vec![] } else { vec![sp(" ".into(), row)] }, ci].concat(),
    };
    let review = match p.decision.as_str() {
        "APPROVED" => vec![sp("✓ approved".into(), tint(ui.plus))],
        "CHANGES_REQUESTED" => vec![sp("✗ changes requested".into(), tint(ui.error))],
        "REVIEW_REQUIRED" => vec![sp("review needed".into(), dim)],
        _ => vec![],
    };
    let claude: Vec<Span<'static>> = mark.claude.map(|(text, ok)| sp(text, tint(if ok { ui.key } else { ui.error }))).into_iter().collect();
    // The line out to the window's edge (the highlight with it).
    let full = |mut spans: Vec<Span<'static>>| {
        let used: usize = spans.iter().map(Span::width).sum();
        spans.push(sp(" ".repeat(width.saturating_sub(used)), row));
        Line::from(spans).style(row)
    };

    if width >= TWO_ZONES {
        let content = width.min(CARD_MAX);
        let left_w = content - 1 - RIGHT; // past the bar
        // The left zone (cut to fit, the title making room for its badges), then the right one.
        // The right zone lines up from card to card; what's in it may run on to the window's edge
        // (a review's who and whom, why Claude failed), but not on the first line, whose age
        // ends the zone.
        let zones = |left: Vec<Span<'static>>, right: Vec<Span<'static>>, run_on: bool| {
            let mut v = vec![bar()];
            v.extend(fit(left, left_w, row));
            v.extend(fit(right, if run_on { width.saturating_sub(1 + left_w).max(RIGHT) } else { RIGHT }, row));
            full(v)
        };
        let tags_w: usize = tags.iter().map(Span::width).sum();
        let top = title_in(left_w.saturating_sub(tags_w), tags);
        // CI on the left of the zone, the age at its right end.
        let age_w = age.width();
        let mut stand = fit(ci, RIGHT.saturating_sub(age_w + 1), row);
        stand.push(age);
        return vec![zones(top, stand, false), zones(people, review, true), zones(place, claude, true)];
    }

    // Narrow: four lines.
    let line = |mut spans: Vec<Span<'static>>| {
        spans.insert(0, bar());
        full(fit(spans, width, row))
    };
    let age = sp(format!("  {} ", age.content), age.style);
    let tags_w: usize = tags.iter().map(Span::width).sum();
    let mut top = title_in(width.saturating_sub(1 + tags_w + age.width()), tags);
    let used: usize = top.iter().map(Span::width).sum::<usize>() + 1;
    top.push(sp(" ".repeat(width.saturating_sub(used + age.width())), row));
    top.push(age);
    let mut status = vec![indent()];
    for (part, w) in [(ci, 16usize), (review, 26)] {
        let used: usize = part.iter().map(Span::width).sum();
        if used > 0 {
            status.extend(part);
            status.push(sp(" ".repeat(w.saturating_sub(used).max(3)), row));
        }
    }
    status.extend(claude);
    vec![line(top), line(people), line(status), line(place)]
}

/// `spans` in exactly `w` columns: cut to it, or padded.
pub(super) fn fit(spans: Vec<Span<'static>>, w: usize, pad: Style) -> Vec<Span<'static>> {
    let mut out = cut_spans(spans, w);
    let used: usize = out.iter().map(Span::width).sum();
    if used < w {
        out.push(Span::styled(" ".repeat(w - used), pad));
    }
    out
}

/// `spans` in at most `w` columns, the last one that doesn't fit ending in `…`.
pub(super) fn cut_spans(spans: Vec<Span<'static>>, w: usize) -> Vec<Span<'static>> {
    let mut out = vec![];
    let mut used = 0;
    for sp in spans {
        let n = sp.width();
        if used + n <= w {
            used += n;
            out.push(sp);
            continue;
        }
        let room = w - used;
        if room > 0 {
            out.push(Span::styled(cut(&sp.content, room), sp.style));
        }
        break;
    }
    out
}

/// Under a card's title on the "can close" tab: why Claude's review would close the PR.
pub(super) fn close_line(why: &str, cx: &Cards, selected: bool) -> Line<'static> {
    let row = card_row(cx.ui, selected);
    let bar = Span::styled(if selected { "▌" } else { " " }, row.patch(Style { bg: None, ..cx.ui.key }));
    let lead = format!("{}{:w$}  ✕ ", " ".repeat(MARK_W), "", w = cx.num_w - 1);
    let text: String = why.lines().next().unwrap_or("").chars().take(cx.width.saturating_sub(lead.chars().count() + 2)).collect();
    Line::from(vec![bar, Span::styled(lead, row), Span::styled(text, row.patch(Style { bg: None, ..cx.ui.warning }))])
}

/// What a todo card says: what holds it up, in short (with Claude's word: its needs' tags), and
/// what it says (Claude's word, "why\nwhat to do", or the needs), with when it's back
/// (deferred), what it waits on, or Claude at it.
pub(super) fn todo_says(it: &crate::todo::Item) -> (Vec<String>, String) {
    let (tags, core) = match &it.reason {
        Some(r) => (it.tags(), r.clone()),
        None => (vec![], it.says()),
    };
    // Placed by the rules since Claude ranked (new, or its needs changed): said so. Deferred:
    // when it's back.
    let says = match &it.deferred {
        _ if it.busy.is_some() => format!("Claude is {}: nothing for you till it's done", it.busy.as_deref().unwrap_or("at it")),
        _ if it.waits.is_some() => {
            let (c, who) = it.waits.as_ref().map(|(c, w)| (c.says(), w.clone())).unwrap_or_default();
            let from = if who.is_empty() { "as Claude read the PR's conversation".to_owned() } else { format!("@{who} said, on the PR") };
            format!("waiting until {c} ({from}) · {core}")
        }
        Some(d) => {
            let when = d.cond.as_ref().map_or_else(|| format!("in {}", crate::todo::in_secs(d.until - crate::claude::now_secs())), |c| format!("until {}", c.says()));
            format!("deferred, back {when} (or sooner if it changes; s brings it back now) · {core}")
        }
        None if it.changed => format!("(placed since Claude ranked) {core}"),
        None => core,
    };
    (tags, says)
}

/// The todo tab's message column, beside the cards: at least this wide, else the message goes
/// under the title.
const MSG_MIN: usize = 40;
/// Its rows: the card's three, and one more.
pub(super) const MSG_ROWS: usize = 4;

/// In a window `width` wide, the cards' width when the todo tab's messages go in a column of
/// their own beside them (a two-zone card and the column's least both fit).
pub(super) fn msg_column(width: usize) -> Option<usize> {
    let content = width.checked_sub(3 + MSG_MIN)?.min(CARD_MAX);
    (content >= TWO_ZONES).then_some(content)
}

/// A todo card's reason line under its title (the window too narrow for the message column):
/// its place in the order, and what needs you, cut where the window ends.
pub(super) fn todo_line(rank: usize, it: &crate::todo::Item, cx: &Cards, selected: bool) -> Line<'static> {
    let ui = cx.ui;
    let row = card_row(ui, selected);
    let st = row.patch(Style { bg: None, ..ui.key });
    let bar = Span::styled(if selected { "▌" } else { " " }, st);
    // Its place in the number's column, the arrow under the title.
    let lead = format!("{}{:>w$}  → ", " ".repeat(MARK_W), format!("{rank}."), w = cx.num_w - 1);
    let (tags, says) = todo_says(it);
    let says = says.replace('\n', " — ");
    let says = match tags.is_empty() {
        true => says,
        false => format!("[{}] {says}", tags.join(" · ")),
    };
    let says = cut(&says, cx.width.saturating_sub(lead.chars().count() + 1));
    let pad = cx.width.saturating_sub(1 + lead.chars().count() + says.chars().count());
    Line::from(vec![bar, Span::styled(lead, st), Span::styled(says, st), Span::styled(" ".repeat(pad), row)]).style(row)
}

/// `lines` (a card, as wide as the cards are) with the todo tab's message column beside them,
/// out to the window's `width`: why it's where it is, wrapped, then `→ what to do` in the
/// accent colour, in `MSG_ROWS` rows (what doesn't fit is cut, the why first).
pub(super) fn with_message(mut lines: Vec<Line<'static>>, says: &str, cx: &Cards, card_w: usize, width: usize, selected: bool) -> Vec<Line<'static>> {
    let ui = cx.ui;
    let row = card_row(ui, selected);
    let st = |s: Style| row.patch(Style { bg: None, ..s });
    let w = width.saturating_sub(card_w + 3);
    // Claude's why and what next, a line each (one ranked before 0.36.1: all why, till ranked again).
    let (why, todo) = match says.split_once('\n') {
        Some((a, b)) => (a, Some(b)),
        None => (says, None),
    };
    let mut todo = todo.map(|t| wrap(&format!("→ {t}"), w)).unwrap_or_default();
    todo.truncate(2);
    let mut why = wrap(why, w);
    let room = MSG_ROWS - todo.len();
    if why.len() > room {
        let rest = why[room.saturating_sub(1)..].join(" ");
        why.truncate(room.saturating_sub(1));
        why.push(cut(&rest, w));
    }
    let text: Vec<(String, Style)> = why.into_iter().map(|l| (l, st(ui.base))).chain(todo.into_iter().map(|l| (l, st(ui.key)))).collect();
    while lines.len() < MSG_ROWS {
        let bar = Span::styled(if selected { "▌" } else { " " }, st(ui.key));
        lines.push(Line::from(vec![bar, Span::styled(" ".repeat(card_w - 1), row)]).style(row));
    }
    for (k, line) in lines.iter_mut().enumerate() {
        let (t, s) = text.get(k).cloned().unwrap_or_default();
        let pad = w.saturating_sub(t.chars().count());
        line.spans.extend([Span::styled(" │ ", st(ui.stale)), Span::styled(t, s), Span::styled(" ".repeat(pad), row)]);
    }
    lines
}

/// `s` in lines at most `w` wide, broken between words (a word longer than that: cut).
fn wrap(s: &str, w: usize) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    let mut cur = String::new();
    for word in s.split_whitespace() {
        let word = cut(word, w.max(1));
        if !cur.is_empty() && cur.chars().count() + 1 + word.chars().count() > w {
            out.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(&word);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// A card's background: the window's, or on the selected one a quiet tint (the theme's cursor
/// line, as in the diff view; else its menu selection's background), its text keeping its colors.
pub(super) fn card_row(ui: &Ui, selected: bool) -> Style {
    match selected {
        true => ui.base.bg(ui.cursorline.or(ui.menu_selected.bg).unwrap_or(ratatui::style::Color::DarkGray)),
        false => ui.base,
    }
}

pub(super) use crate::view::cut;
