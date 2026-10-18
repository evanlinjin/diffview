//! A PR's (or a replacement's) card in the list: two lines, and what they say about Claude.

use super::*;

/// A replacement's card: its drafted title, its branch, and its size against its base (from
/// git: nothing's on GitHub yet).
pub(super) fn rep_card(r: &crate::replace::Record, root: &std::path::Path) -> Listed {
    let branch = crate::replace::branch(r.id);
    let git = |args: &[&str]| crate::diff::git(&[&["-C", &root.to_string_lossy()], args].concat()).unwrap_or_default();
    // " 3 files changed, 10 insertions(+), 2 deletions(-)"
    let stat = git(&["diff", "--shortstat", &format!("origin/{}...{branch}", r.base)]);
    let count = |what: &str| -> u64 {
        stat.split(',').find(|p| p.contains(what)).and_then(|p| p.split_whitespace().next()?.parse().ok()).unwrap_or(0)
    };
    let title = match r.title.is_empty() {
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
            // Why, in short (the status bar said it in full): `a a` asks again.
            Some(batch::Status::Failed(e)) => Some((format!("claude failed: {}", cut(e.lines().next().unwrap_or(""), 60)), false)),
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

/// A PR as four lines, related things together: its number and title (with a draft / closed
/// badge, `[new]`, and its age at the right); who and how big (and labels); how it stands (CI,
/// review, Claude), in columns that line up from card to card; where its code is (branch, and
/// its worktree here). Each line is cut at the width rather than wrapped.
pub(super) fn card(p: &Listed, cx: &Cards, selected: bool, local: Option<&(PathBuf, Option<String>)>, mark: Mark) -> Vec<Line<'static>> {
    let (ui, num_w, width, usual) = (cx.ui, cx.num_w, cx.width, cx.usual);
    let row = card_row(ui, selected);
    // Text keeps its colors on the selected card too (only the background shows it).
    let tint = |st: Style| row.patch(Style { bg: None, ..st });
    let dim = tint(ui.stale);
    let sp = |t: String, st| Span::styled(t, st);
    // The selected card has a bar down its left edge.
    let bar = || sp(if selected { "▌".into() } else { " ".into() }, tint(ui.key));
    // Under the title: past the bar, the mark, the number and its gap.
    let indent = || [bar(), sp(" ".repeat(num_w + 3), row)];
    // A line of the card: its spans cut to the width, the highlight run to the edge.
    let line = |mut spans: Vec<Span<'static>>| {
        let mut used: usize = spans.iter().map(Span::width).sum();
        while used > width && spans.len() > 1 {
            used -= spans.pop().map_or(0, |s| s.width());
        }
        if used < width {
            spans.push(sp(" ".repeat(width - used), row));
        }
        Line::from(spans).style(row)
    };

    // Number and title; badges, [new] and the age at the right.
    let age = format!("{} ", gh::ago(&p.updated));
    let badge = match (p.state.as_str(), p.draft) {
        ("MERGED", _) => Some(("MERGED", "8957e5")),
        ("CLOSED", _) => Some(("CLOSED", "da3633")),
        (_, true) => Some(("DRAFT", "6e7681")),
        _ => None,
    };
    let new = if mark.new { "[new] " } else { "" };
    let room = width.saturating_sub(num_w + 4 + badge.map_or(0, |b| b.0.len() + 4) + new.len() + age.len()).max(10);
    let mut top = vec![
        bar(),
        sp(format!("{}{:>w$}  ", if mark.marked { "●" } else { " " }, format!("#{}", p.number), w = num_w), tint(ui.key)),
        sp(cut(&p.title, room), row.add_modifier(ratatui::style::Modifier::BOLD)),
    ];
    if let Some((text, color)) = badge {
        top.extend([sp("  ".into(), row), sp(format!(" {text} "), crate::view::chip(color))]);
    }
    let used: usize = top.iter().map(Span::width).sum();
    top.push(sp(" ".repeat(width.saturating_sub(used + new.len() + age.len())), row));
    top.extend([sp(new.into(), tint(ui.key).add_modifier(ratatui::style::Modifier::BOLD)), sp(age, dim)]);

    // Who, and how big; labels.
    let gap = || sp("   ".into(), row);
    let who = mark.replaces.clone().unwrap_or_else(|| format!("@{}", p.author));
    let mut people = indent().to_vec();
    people.extend([sp(who, dim), gap()]);
    people.extend([
        sp(format!("+{}", p.additions), tint(ui.plus)),
        sp(" ".into(), row),
        sp(format!("−{}", p.deletions), tint(ui.minus)),
        sp(format!(" in {} file{}", p.files, if p.files == 1 { "" } else { "s" }), dim),
    ]);
    for (k, (name, color)) in p.labels.iter().enumerate() {
        people.extend([sp(if k == 0 { "   " } else { " " }.into(), row), sp(format!(" {name} "), crate::view::chip(color))]);
    }

    // How it stands, in columns: CI, review, Claude.
    // A column takes its width only when it has something (else what's after it moves up).
    let col = |spans: Vec<Span<'static>>, w: usize| {
        let used: usize = spans.iter().map(Span::width).sum();
        let mut v = spans;
        if used > 0 {
            v.push(sp(" ".repeat(w.saturating_sub(used).max(3)), row));
        }
        v
    };
    let ci: Vec<Span<'static>> = match p.checks {
        Some((0, 0, 0)) => vec![],
        Some((pass, fail, pending)) => {
            let mut v = vec![sp("CI ".into(), dim)];
            let counts = [(pass, '✓', ui.plus), (fail, '✗', ui.error), (pending, '●', ui.warning)];
            for (k, &(n, m, st)) in counts.iter().filter(|c| c.0 > 0).enumerate() {
                v.extend([sp(if k == 0 { "" } else { " " }.into(), row), sp(format!("{m}{n}"), tint(st))]);
            }
            v
        }
        None if cx.checks_loading && mark.replaces.is_none() => vec![sp("CI …".into(), dim)],
        None => vec![],
    };
    let review = match p.decision.as_str() {
        "APPROVED" => vec![sp("review ".into(), dim), sp("✓ approved".into(), tint(ui.plus))],
        "CHANGES_REQUESTED" => vec![sp("review ".into(), dim), sp("✗ changes requested".into(), tint(ui.error))],
        "REVIEW_REQUIRED" => vec![sp("review needed".into(), dim)],
        _ => vec![],
    };
    let claude = mark.claude.map(|(text, ok)| sp(text, tint(if ok { ui.key } else { ui.error })));
    let mut status = indent().to_vec();
    status.extend(col(ci, 16));
    status.extend(col(review, 26));
    status.extend(claude);

    // Where its code is.
    let branch = match p.base == usual {
        true => cut(&p.branch, 48),
        false => format!("{} ← {}", p.base, cut(&p.branch, 40)),
    };
    let mut place = indent().to_vec();
    place.push(sp(branch, dim));
    if let Some((dir, branch)) = local {
        let name = dir.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
        let on = branch.as_deref().map_or("detached".to_owned(), |b| format!("on {}", cut(b, 24)));
        place.extend([gap(), sp(format!("⌂ {name} ({on})"), tint(ui.key))]);
    }
    vec![line(top), line(people), line(status), line(place)]
}

/// A todo card's reason line, under the title: its place in the order, and what needs you
/// (Claude's word, or the needs).
pub(super) fn todo_line(rank: usize, it: &crate::todo::Item, cx: &Cards, selected: bool) -> Line<'static> {
    let ui = cx.ui;
    let row = card_row(ui, selected);
    let st = row.patch(Style { bg: None, ..ui.key });
    let bar = Span::styled(if selected { "▌" } else { " " }, st);
    // Its place in the number's column, the arrow under the title.
    let lead = format!(" {:>w$}  → ", format!("{rank}."), w = cx.num_w);
    let says = cut(&it.says(), cx.width.saturating_sub(lead.chars().count() + 1));
    let pad = cx.width.saturating_sub(1 + lead.chars().count() + says.chars().count());
    Line::from(vec![bar, Span::styled(lead, st), Span::styled(says, st), Span::styled(" ".repeat(pad), row)]).style(row)
}

/// A card's background: the window's, or on the selected one a quiet tint (the theme's cursor
/// line, as in the diff view; else its menu selection's background), its text keeping its colors.
fn card_row(ui: &Ui, selected: bool) -> Style {
    match selected {
        true => ui.base.bg(ui.cursorline.or(ui.menu_selected.bg).unwrap_or(ratatui::style::Color::DarkGray)),
        false => ui.base,
    }
}

/// `s` cut to `n` chars, ending in `…` if it was longer.
pub(super) fn cut(s: &str, n: usize) -> String {
    match s.chars().count() > n {
        true => format!("{}…", s.chars().take(n.saturating_sub(1)).collect::<String>()),
        false => s.to_owned(),
    }
}
