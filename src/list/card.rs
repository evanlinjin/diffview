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

/// A PR as two lines: number, title, state / draft badge and age; then who, CI, review, size,
/// labels, branches and its worktree, most useful first so a narrow screen cuts the least useful.
pub(super) fn card(p: &Listed, cx: &Cards, selected: bool, local: Option<&(PathBuf, Option<String>)>, mark: Mark) -> [Line<'static>; 2] {
    let (ui, num_w, width, usual) = (cx.ui, cx.num_w, cx.width, cx.usual);
    let row = if selected { ui.base.patch(ui.menu_selected) } else { ui.base };
    // On the selected card the text takes the highlight's colors, to stay readable (labels
    // keep theirs).
    let tint = |st: Style| if selected { row } else { row.patch(st) };
    let dim = tint(ui.stale);
    let sp = |t: String, st| Span::styled(t, st);
    let age = format!("{} ", gh::ago(&p.updated));
    // Closed / merged (only in the worktrees tab), else draft, as GitHub colors them.
    let badge = match (p.state.as_str(), p.draft) {
        ("MERGED", _) => Some(("MERGED", "8957e5")),
        ("CLOSED", _) => Some(("CLOSED", "da3633")),
        (_, true) => Some(("DRAFT", "6e7681")),
        _ => None,
    };
    let new = if mark.new { "[new] " } else { "" };
    let room = width.saturating_sub(num_w + 3 + badge.map_or(0, |b| b.0.len() + 4) + new.len() + age.len()).max(10);
    let mut top = vec![
        sp(format!("{}{:>num_w$} ", if mark.marked { "●" } else { " " }, format!("#{}", p.number)), tint(ui.key)),
        sp(cut(&p.title, room), row),
    ];
    if let Some((text, color)) = badge {
        top.extend([sp("  ".into(), row), sp(format!(" {text} "), crate::view::chip(color))]);
    }
    // At the right end, before the age.
    let used: usize = top.iter().map(Span::width).sum();
    top.push(sp(" ".repeat(width.saturating_sub(used + new.len() + age.len())), row));
    top.extend([sp(new.into(), tint(ui.key).add_modifier(ratatui::style::Modifier::BOLD)), sp(age, dim)]);

    let sep = || sp(" · ".into(), dim);
    let who = mark.replaces.clone().unwrap_or_else(|| format!("@{}", p.author));
    let mut bottom = vec![sp(" ".repeat(num_w + 2), row), sp(who, dim)];
    match p.checks {
        Some((0, 0, 0)) => {}
        Some((pass, fail, pending)) => {
            let counts = [(pass, '✓', ui.plus), (fail, '✗', ui.error), (pending, '●', ui.warning)];
            for (k, &(n, mark, st)) in counts.iter().filter(|c| c.0 > 0).enumerate() {
                bottom.extend([sp(if k == 0 { " · " } else { " " }.into(), dim), sp(format!("{mark}{n}"), tint(st))]);
            }
        }
        None if cx.checks_loading && mark.replaces.is_none() => bottom.extend([sep(), sp("checks…".into(), dim)]),
        None => {}
    }
    match p.decision.as_str() {
        "APPROVED" => bottom.extend([sep(), sp("✓ approved".into(), tint(ui.plus))]),
        "CHANGES_REQUESTED" => bottom.extend([sep(), sp("✗ changes requested".into(), tint(ui.error))]),
        _ => {}
    }
    bottom.extend([
        sep(),
        sp(format!("+{}", p.additions), tint(ui.plus)),
        sp(" ".into(), row),
        sp(format!("−{}", p.deletions), tint(ui.minus)),
        sp(format!(" in {} file{}", p.files, if p.files == 1 { "" } else { "s" }), dim),
    ]);
    for (name, color) in &p.labels {
        bottom.extend([sp(" ".into(), row), sp(format!(" {name} "), crate::view::chip(color))]);
    }
    let branch = match p.base == usual {
        true => cut(&p.branch, 32),
        false => format!("{} ← {}", p.base, cut(&p.branch, 32)),
    };
    bottom.extend([sep(), sp(branch, dim)]);
    if let Some((dir, branch)) = local {
        let name = dir.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
        let on = branch.as_deref().map_or("detached".to_owned(), |b| format!("on {}", cut(b, 24)));
        bottom.extend([sep(), sp(format!("⌂ {name} ({on})"), tint(ui.key))]);
    }
    // Claude's part at the right end, where it stands out (under the age), the rest cut to
    // make room; the highlight runs to the edge.
    let claude = mark.claude.map(|(text, ok)| sp(format!("{text} "), tint(if ok { ui.key } else { ui.error })));
    let room = width.saturating_sub(claude.as_ref().map_or(0, |c| c.width() + 1));
    let mut used: usize = bottom.iter().map(Span::width).sum();
    while (used > room || bottom.last().is_some_and(|s| s.content == " · ")) && bottom.len() > 1 {
        used -= bottom.pop().map_or(0, |s| s.width());
    }
    bottom.push(sp(" ".repeat(width.saturating_sub(used + claude.as_ref().map_or(0, Span::width))), row));
    bottom.extend(claude);
    [Line::from(top).style(row), Line::from(bottom).style(row)]
}

/// A todo card's third line: its place, and what needs you (Claude's word, or the needs).
pub(super) fn todo_line(rank: usize, it: &crate::todo::Item, cx: &Cards, selected: bool) -> Line<'static> {
    let ui = cx.ui;
    let row = if selected { ui.base.patch(ui.menu_selected) } else { ui.base };
    let st = if selected { row } else { row.patch(ui.key) };
    let lead = format!("{:>w$}. ", rank, w = cx.num_w + 1);
    let says = cut(&it.says(), cx.width.saturating_sub(lead.chars().count() + 1));
    let pad = cx.width.saturating_sub(lead.chars().count() + says.chars().count());
    Line::from(vec![Span::styled(lead, st), Span::styled(says, st), Span::styled(" ".repeat(pad), row)]).style(row)
}

/// `s` cut to `n` chars, ending in `…` if it was longer.
pub(super) fn cut(s: &str, n: usize) -> String {
    match s.chars().count() > n {
        true => format!("{}…", s.chars().take(n.saturating_sub(1)).collect::<String>()),
        false => s.to_owned(),
    }
}
