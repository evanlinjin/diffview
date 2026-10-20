//! An issue in dv (Enter in `dv issues`): its text and conversation, Claude's triage under them,
//! and what to do about it: comment (`C`), apply Claude's labels (`L L`) or title (`T T`),
//! close it with Claude's comment (`X X`, after your editor), have Claude draft a PR fixing it
//! (`N N`), or triage it again (`a a`).
use super::*;
use serde_json::Value;

/// A line's links: their columns, and where they go.
type Links = Vec<(std::ops::Range<usize>, String)>;

/// `links`, `by` columns to the right.
fn shift(links: Links, by: usize) -> Links {
    links.into_iter().map(|(r, u)| (r.start + by..r.end + by, u)).collect()
}

/// Blank lines after each side's: the keys' box (`dv issues`'s, 7 keys and its frame) sits
/// over the bottom right, and the last lines scroll clear of it. The cursor stays above them.
const PAD: usize = 10;

/// A side of the view: its lines, where each's text is (the screen's columns: not margins,
/// bubbles' sides) and its links, and its columns on screen. Its lines are its rows: the cursor's
/// and a selection's rows count in the side they're on.
#[derive(Default)]
struct Side {
    lines: Vec<Line<'static>>,
    text: Vec<std::ops::Range<usize>>,
    links: Vec<Links>,
    x: usize,
    w: usize,
}

/// A thing in an issue's timeline: a comment (who, when, what), or something done (who, what,
/// when, a link: what mentioned it, say).
#[derive(Clone, Debug)]
pub enum Act {
    Comment(String, String, String),
    Event(String, String, String, Option<String>),
}

/// What of an issue's timeline dv shows, and each type's fields.
const ISSUE_TIMELINE: &str = "timelineItems(first: 100, after: $after, itemTypes: [ISSUE_COMMENT, LABELED_EVENT, UNLABELED_EVENT, \
ASSIGNED_EVENT, UNASSIGNED_EVENT, RENAMED_TITLE_EVENT, CLOSED_EVENT, REOPENED_EVENT, CROSS_REFERENCED_EVENT, \
MARKED_AS_DUPLICATE_EVENT, CONNECTED_EVENT]) { pageInfo { hasNextPage endCursor } nodes { __typename \
... on IssueComment { createdAt author { login } body } \
... on LabeledEvent { createdAt actor { login } label { name } } \
... on UnlabeledEvent { createdAt actor { login } label { name } } \
... on AssignedEvent { createdAt actor { login } assignee { ... on User { login } ... on Bot { login } ... on Mannequin { login } ... on Organization { login } } } \
... on UnassignedEvent { createdAt actor { login } assignee { ... on User { login } ... on Bot { login } ... on Mannequin { login } ... on Organization { login } } } \
... on RenamedTitleEvent { createdAt actor { login } previousTitle currentTitle } \
... on ClosedEvent { createdAt actor { login } stateReason closer { ... on PullRequest { number url } ... on Commit { abbreviatedOid url } } } \
... on ReopenedEvent { createdAt actor { login } } \
... on CrossReferencedEvent { createdAt actor { login } source { ... on Issue { number title url repository { nameWithOwner } } ... on PullRequest { number title url repository { nameWithOwner } } } } \
... on MarkedAsDuplicateEvent { createdAt actor { login } canonical { ... on Issue { number url repository { nameWithOwner } } ... on PullRequest { number url repository { nameWithOwner } } } } \
... on ConnectedEvent { createdAt actor { login } subject { ... on Issue { number title url repository { nameWithOwner } } ... on PullRequest { number title url repository { nameWithOwner } } } } } }";

/// `owner/name` of a GitHub URL (`https://github.com/o/r/issues/7`).
fn url_repo(url: &str) -> &str {
    let rest = url.strip_prefix("https://github.com/").unwrap_or("");
    rest.match_indices('/').nth(1).map_or(rest, |(i, _)| &rest[..i])
}

/// An issue's timeline (`ISSUE_TIMELINE`'s nodes), as dv shows it (`here`: its repo,
/// `owner/name`: another's things are named with theirs).
fn acts(nodes: &[Value], here: &str) -> Vec<Act> {
    let s = |v: &Value| v.as_str().unwrap_or("").to_owned();
    // (A deleted account's: GitHub's "ghost".)
    let login = |v: &Value| v["login"].as_str().unwrap_or("ghost").to_owned();
    nodes
        .iter()
        .filter_map(|it| {
            let (who, at) = (login(&it["actor"]), s(&it["createdAt"]));
            let ev = |what: String, url: Option<String>| Some(Act::Event(who.clone(), what, at.clone(), url));
            let to = |x: &Value| {
                let repo = x["repository"]["nameWithOwner"].as_str().filter(|r| !r.eq_ignore_ascii_case(here)).unwrap_or("");
                let n = x["number"].as_u64().map_or("a deleted or private one".to_owned(), |n| format!("{repo}#{n}"));
                (format!("{n} {}", s(&x["title"])).trim_end().to_owned(), Some(s(&x["url"])).filter(|u| !u.is_empty()))
            };
            match it["__typename"].as_str()? {
                "IssueComment" => Some(Act::Comment(login(&it["author"]), at, s(&it["body"]).replace("\r\n", "\n"))),
                "LabeledEvent" => ev(format!("added the label {}", s(&it["label"]["name"])), None),
                "UnlabeledEvent" => ev(format!("removed the label {}", s(&it["label"]["name"])), None),
                "AssignedEvent" => ev(format!("assigned @{}", login(&it["assignee"])), None),
                "UnassignedEvent" => ev(format!("unassigned @{}", login(&it["assignee"])), None),
                "RenamedTitleEvent" => ev(format!("changed the title: {} → {}", s(&it["previousTitle"]), s(&it["currentTitle"])), None),
                "ClosedEvent" => {
                    let c = &it["closer"];
                    let by = c["number"].as_u64().map(|n| format!(" with #{n}")).or(c["abbreviatedOid"].as_str().map(|o| format!(" with {o}"))).unwrap_or_default();
                    let why = s(&it["stateReason"]).to_lowercase().replace('_', " ");
                    ev(format!("closed it{by}{}", if why.is_empty() { String::new() } else { format!(" ({why})") }), Some(s(&c["url"])).filter(|u| !u.is_empty()))
                }
                "ReopenedEvent" => ev("reopened it".into(), None),
                "CrossReferencedEvent" => {
                    let (what, url) = to(&it["source"]);
                    ev(format!("mentioned this in {what}"), url)
                }
                "MarkedAsDuplicateEvent" => {
                    let (what, url) = to(&it["canonical"]);
                    ev(format!("marked this a duplicate of {what}"), url)
                }
                "ConnectedEvent" => {
                    let (what, url) = to(&it["subject"]);
                    ev(format!("linked {what}"), url)
                }
                _ => None,
            }
        })
        .collect()
}

/// What a key in the view came to, for the list to do: text to copy, a link to open.
pub enum Did {
    Nothing,
    Copy(String),
    Open(String),
    /// Not the view's key.
    Not,
}

/// An issue as dv shows it: as GitHub has it, and Claude's triage (all of it: the summary,
/// labels, title and closing comment too, not just what its card shows).
pub struct IssueView {
    pub number: u64,
    title: String,
    pub author: String,
    url: String,
    body: String,
    labels: Vec<(String, String)>,
    /// Its timeline, oldest first: comments, and what else happened.
    activity: Vec<Act>,
    triage: Value,
    /// How far each side's scrolled (one column: the first's), on its own.
    scroll: [usize; 2],
    /// The sides as last drawn (side by side: the description, the pane; else one, all of it),
    /// at what width, where the pane starts, and how many lines show.
    sides: Vec<Side>,
    width: usize,
    pane_x: Option<usize>,
    shown: usize,
    /// The cursor (line, column), as Helix's: `v` selects from where it was (`sel` then
    /// follows it), `x` the line.
    cur: (usize, usize),
    select: bool,
    /// The selection is `x`'s, whole lines: `x` again takes the next.
    lines_sel: bool,
    /// How many lines the description takes (one column: the pane's after them).
    desc_rows: usize,
    /// Code blocks and tables scrolled sideways (the mouse's), the description's and the
    /// pane's, and how far each could go (as last drawn).
    code_left: [usize; 2],
    code_over: [usize; 2],
    /// The drafts of Claude's that fix it (published or not), as `dv issues` found them.
    pub drafts: Vec<String>,
    /// Whether you may label it (triage access and up): Claude's labels are suggestions else.
    can_triage: bool,
    /// Text dragged over (line, column), from where the drag began to where it is.
    sel: Option<((usize, usize), (usize, usize))>,
    hl: crate::highlight::Highlighter,
}

impl IssueView {
    /// Issue `n` from GitHub, with its triage from `dir` (the reviews' directory).
    pub fn load(n: u64, dir: Option<&std::path::Path>, can_triage: bool) -> Result<IssueView> {
        let v: Value = gh::gh_json(&["issue", "view", &n.to_string(), "--json", "number,title,author,url,body,labels"])?;
        // Its timeline: comments, and what else happened (labels, who has it, what mentions it).
        // (All of it, a page at a time: ponytail, 10 pages, 1,000 at most.)
        let q = format!("query($owner: String!, $name: String!, $n: Int!, $after: String) {{ repository(owner: $owner, name: $name) {{ issue(number: $n) {{ {ISSUE_TIMELINE} }} }} }}");
        let (mut nodes, mut after) = (vec![], None::<String>);
        for _ in 0..10 {
            let mut args = vec!["api".to_owned(), "graphql".into(), "-F".into(), "owner={owner}".into(), "-F".into(), "name={repo}".into(), "-F".into(), format!("n={n}"), "-f".into(), format!("query={q}")];
            if let Some(a) = &after {
                args.extend(["-f".into(), format!("after={a}")]);
            }
            let t: Value = gh::gh_json(&args.iter().map(String::as_str).collect::<Vec<_>>())?;
            let page = &t["data"]["repository"]["issue"]["timelineItems"];
            nodes.extend(page["nodes"].as_array().cloned().unwrap_or_default());
            match page["pageInfo"]["endCursor"].as_str().filter(|_| page["pageInfo"]["hasNextPage"] == true) {
                Some(c) => after = Some(c.to_owned()),
                None => break,
            }
        }
        let s = |v: &Value| v.as_str().unwrap_or("").to_owned();
        let triage = dir
            .and_then(|d| std::fs::read_to_string(d.join(format!("issue-{n}.json"))).ok())
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or(Value::Null);
        Ok(IssueView {
            number: n,
            title: s(&v["title"]),
            author: s(&v["author"]["login"]),
            url: s(&v["url"]),
            body: s(&v["body"]).replace("\r\n", "\n"),
            labels: v["labels"].as_array().into_iter().flatten().map(|l| (s(&l["name"]), s(&l["color"]))).collect(),
            activity: acts(&nodes, url_repo(&s(&v["url"]))),
            triage,
            scroll: [0; 2],
            sides: vec![],
            width: 0,
            pane_x: None,
            shown: 0,
            cur: (0, 2),
            select: false,
            lines_sel: false,
            desc_rows: 0,
            code_left: [0; 2],
            code_over: [0; 2],
            can_triage,
            drafts: vec![],
            sel: None,
            hl: crate::highlight::Highlighter::new(&Theme::load()),
        })
    }

    /// Its title bar, as the PR view's: `Issue #N · title`, who opened it, its labels.
    pub fn title_bar(&self, f: &mut Frame, ui: &Ui, r: Rect) {
        let hs = ui.base.patch(Style { bg: ui.bar, ..ui.header });
        let dim = hs.remove_modifier(ratatui::style::Modifier::BOLD).patch(Style { fg: ui.linenr.fg, ..Style::new() });
        let mut spans = vec![Span::styled(format!(" Issue #{} · {}", self.number, self.title), hs.add_modifier(ratatui::style::Modifier::BOLD)), Span::styled(format!("  @{}", self.author), dim)];
        for (name, color) in &self.labels {
            spans.extend([Span::styled(" ", hs), Span::styled(format!(" {name} "), crate::view::chip(color))]);
        }
        f.render_widget(Paragraph::new(Line::from(spans)).style(hs), r);
    }

    /// Claude's triage's `k`, trimmed (empty: not said).
    fn t(&self, k: &str) -> String {
        self.triage[k].as_str().unwrap_or("").trim().to_owned()
    }

    /// The labels Claude's triage would add (those it doesn't have yet).
    pub fn new_labels(&self) -> Vec<String> {
        let has = |l: &str| self.labels.iter().any(|(n, _)| n.eq_ignore_ascii_case(l));
        self.triage["labels"].as_array().into_iter().flatten().filter_map(Value::as_str).filter(|l| !has(l)).map(str::to_owned).collect()
    }

    /// Claude's better title, if it has one (and it isn't the title already).
    pub fn new_title(&self) -> Option<String> {
        Some(self.t("title")).filter(|t| !t.is_empty() && *t != self.title)
    }

    /// Claude's closing comment, if it would close it.
    pub fn closing(&self) -> Option<String> {
        (!self.t("close").is_empty()).then(|| self.t("closing"))
    }

    /// What a PR fixing it is for Claude to do (`N N`): the issue, and what its triage found.
    pub fn fix_task(&self) -> String {
        let summary = self.t("summary");
        let mut task = format!("Fix #{}: {}", self.number, self.title);
        if !summary.is_empty() {
            task += &format!("\n\nFrom triaging it:\n{summary}");
        }
        task
    }

    /// The issue as the PR view shows a PR: its description on the left (as the PR's text), and
    /// beside it a pane (a third of the window, 60–80 columns) with Claude's triage in its box,
    /// then the conversation, a bubble a comment. Too narrow for two: one column, the pane's
    /// boxes after the description.
    fn render(&mut self, ui: &Ui, width: usize) {
        let pane_w = (width / 3).clamp(60, 80);
        let two = width >= pane_w + 60;
        let (left_w, right_w) = match two {
            true => (width - pane_w - 1, pane_w),
            false => (width, width),
        };
        // (`#123` linked to the repo's, as the PR view's.)
        let repo = self.repo().map(str::to_owned);
        // (A code block's line in its background to the width given, as hover popups have it.)
        let code_bg = ui.md.code_bg;
        // (Code and tables scrolled sideways, as far as the side's are; how far they'd go.)
        let over = std::cell::Cell::new([0; 2]);
        // (`seen`: the columns shown, the description's 2 more than it's wrapped at.)
        let md = |text: &str, w: usize, side: usize| -> Vec<(Line<'static>, Links)> {
            let seen = if side == 0 { w + 2 } else { w };
            let lines = crate::md::render(text, w, &ui.md, &self.hl, repo.as_deref());
            // Each block (code, a table: its lines in a run) by as much of the side's scroll as
            // it's wider than shown: one that fits stays put.
            let mut by = vec![0; lines.len()];
            let mut i = 0;
            while i < lines.len() {
                let n = lines[i..].iter().take_while(|l| l.nowrap).count().max(1);
                let wide = lines[i..i + n].iter().filter(|l| l.nowrap).map(|l| Span::raw(l.text.as_str()).width()).max().unwrap_or(0);
                let o = wide.saturating_sub(seen);
                let mut all = over.get();
                all[side] = all[side].max(o);
                over.set(all);
                by[i..i + n].fill(self.code_left[side].min(o));
                i += n;
            }
            lines
                .into_iter()
                .zip(by)
                .map(|(l, by)| {
                    let code = code_bg.filter(|&bg| l.nowrap && l.styles.last().copied().flatten().and_then(|s| s.bg) == Some(bg));
                    // (Its links' columns, as drawn: chars to columns, less what's scrolled.)
                    let col = |c: usize| Span::raw(l.text.chars().take(c).collect::<String>()).width().saturating_sub(by);
                    let links: Links = l.links.iter().map(|(s, e, u)| (col(*s)..col(*e).min(seen), u.clone())).filter(|(r, _)| !r.is_empty()).collect();
                    let wide = Span::raw(l.text.as_str()).width();
                    let nowrap = l.nowrap;
                    let mut line = l.into_line();
                    line.spans = crate::view::skip_cols(line.spans, by);
                    // Cut where it's shown, `‹` / `›` where there's more to scroll to.
                    if nowrap && wide > seen {
                        let more = Style::new().fg(ui.linenr.fg.unwrap_or(ratatui::style::Color::DarkGray));
                        if wide.saturating_sub(by) > seen {
                            line.spans = super::card::cut_spans(std::mem::take(&mut line.spans), seen.saturating_sub(1));
                            line.spans.push(Span::styled("›", more));
                        }
                        if by > 0 && !line.spans.is_empty() {
                            // (Over a wide character: a space after, the rest where it was.)
                            let first = line.spans.remove(0);
                            let pad = " ".repeat(first.width().saturating_sub(1));
                            line.spans.splice(0..0, [Span::styled("‹", first.style.patch(more)), Span::styled(pad, first.style)]);
                        }
                    }
                    if let Some(bg) = code {
                        line.spans.push(Span::styled(" ".repeat(w.saturating_sub(line.width())), Style::new().bg(bg)));
                    }
                    (line, links)
                })
                .collect()
        };
        let dim = ui.stale;
        let border = ui.linenr;
        // A bubble: `╭─ title ──╮`, its lines between `│ … │`, `╰──╯`, `w` wide (each with the
        // columns of its text: none on its frame).
        let bubble = |title: Vec<Span<'static>>, body: Vec<(Line<'static>, Links)>, w: usize| -> Vec<(Line<'static>, std::ops::Range<usize>, Links)> {
            let tw: usize = title.iter().map(Span::width).sum();
            let mut top = vec![Span::styled("╭─ ", border)];
            top.extend(title);
            top.push(Span::styled(format!(" {}╮", "─".repeat(w.saturating_sub(tw + 5))), border));
            let mut out = vec![(Line::from(top), 0..0, vec![])];
            for (l, links) in body {
                // (A code block or table line wider than the bubble: cut at its edge.)
                let mut v = vec![Span::styled("│ ", border)];
                v.extend(super::card::fit(l.spans, w.saturating_sub(4), Style::new()));
                v.push(Span::styled(" │", border));
                out.push((Line::from(v), 2..w.saturating_sub(2), shift(links, 2)));
            }
            out.push((Line::from(Span::styled(format!("╰{}╯", "─".repeat(w.saturating_sub(2))), border)), 0..0, vec![]));
            out
        };
        // The description, as the PR view's text.
        let url_w = Span::raw(self.url.as_str()).width();
        let mut left: Vec<(Line, Links)> = vec![
            (Line::from(Span::styled(format!("  {}", self.url), dim)), vec![(2..2 + url_w, self.url.clone())]),
            (Line::default(), vec![]),
            (Line::from(Span::styled("  description", ui.heading.add_modifier(ratatui::style::Modifier::BOLD))), vec![]),
        ];
        let body = if self.body.trim().is_empty() { "_(no description)_" } else { &self.body };
        left.extend(md(body, left_w.saturating_sub(4).max(20), 0).into_iter().map(|(mut l, links)| {
            l.spans.insert(0, Span::raw("  "));
            (l, shift(links, 2))
        }));
        // The pane: Claude's triage, then the conversation.
        let inner = right_w.saturating_sub(4).max(20);
        let claude = |text: String| vec![Span::styled("◆ ", ui.key), Span::styled(text, ui.thread)];
        let mut right = vec![];
        // Triaged (its kind or summary said: a closing comment alone isn't), or not yet; what's
        // said of closing it shows either way.
        let triaged = !self.t("kind").is_empty() || !self.t("summary").is_empty();
        {
            let title = ["claude's triage".to_owned(), self.t("kind"), self.t("priority")].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
            let mut b = match triaged {
                true => md(&self.t("summary"), inner, 1),
                false => vec![(Line::from(Span::styled("none yet: a a triages it", dim)), vec![])],
            };
            // (Wrapped as markdown: a long `next` stays in the bubble.)
            let said = |b: &mut Vec<(Line<'static>, Links)>, k: &str, v: String| {
                if !v.is_empty() {
                    if !b.is_empty() {
                        b.push((Line::default(), vec![]));
                    }
                    b.extend(md(&format!("**{k}:** {v}"), inner, 1));
                }
            };
            said(&mut b, "next", self.t("next"));
            right.extend(bubble(claude(title), b, right_w));
            // What it suggests doing, a box of its own, each with its keys.
            let mut acts = vec![];
            said(&mut acts, if self.can_triage { "add the labels (L L)" } else { "labels to suggest (labelling takes triage access)" }, self.new_labels().join(", "));
            said(&mut acts, "retitle it (T T)", self.new_title().unwrap_or_default());
            said(&mut acts, "close it (X X)", self.t("close"));
            if let Some(c) = self.closing().filter(|c| !c.is_empty()) {
                if !acts.is_empty() {
                    acts.push((Line::default(), vec![]));
                }
                acts.push((Line::from(Span::styled("closing comment:", ui.key)), vec![]));
                acts.extend(md(&c, inner, 1));
            }
            if !acts.is_empty() {
                right.push((Line::default(), 0..0, vec![]));
                right.extend(bubble(claude("claude suggests".into()), acts, right_w));
            }
        }
        // The drafts of Claude's that fix it, published or not.
        if !self.drafts.is_empty() {
            right.push((Line::default(), 0..0, vec![]));
            right.push((Line::from(Span::styled(format!("draft fixes ({})", self.drafts.len()), ui.heading)), 0..right_w, vec![]));
            for d in &self.drafts {
                right.extend(md(&format!("- {d}"), inner + 2, 1).into_iter().map(|(l, links)| (l, 0..right_w, links)));
            }
        }
        right.push((Line::default(), 0..0, vec![]));
        right.push((Line::from(Span::styled(format!("activity ({})", self.activity.len()), ui.heading)), 0..right_w, vec![]));
        for a in &self.activity {
            match a {
                Act::Comment(who, when, body) => {
                    let mut title = vec![Span::styled(format!("@{who}"), ui.thread)];
                    if !when.is_empty() {
                        title.push(Span::styled(format!(" · {}", gh::ago(when)), dim));
                    }
                    right.extend(bubble(title, md(body, inner, 1), right_w));
                }
                // What else happened: a dim line, as the PR view's (a link: Enter opens it).
                Act::Event(who, what, when, url) => {
                    let text = format!("● @{who} {what} · {}", gh::ago(when));
                    let w = Span::raw(text.as_str()).width();
                    let links = url.iter().map(|u| (0..w.min(right_w), u.clone())).collect();
                    right.push((Line::from(Span::styled(super::card::cut(&text, right_w), dim)), 0..w.min(right_w), links));
                }
            }
        }
        // Side by side, a dim line between; or one after the other.
        // (The description's text: after its margin.)
        let desc = 2..left_w;
        self.desc_rows = left.len();
        let sides = match two {
            true => vec![
                Side {
                    // (The description cut at its column: a wide code line doesn't spill over.)
                    text: vec![desc.clone(); left.len()],
                    links: left.iter().map(|(_, l)| l.iter().filter(|(r, _)| r.start < left_w).cloned().collect()).collect(),
                    lines: left.into_iter().map(|(l, _)| Line::from(super::card::fit(l.spans, left_w, Style::new()))).collect(),
                    x: 0,
                    w: left_w,
                },
                Side {
                    text: right.iter().map(|(_, at, _)| at.start + left_w + 1..at.end + left_w + 1).collect(),
                    links: right.iter().map(|(_, _, l)| shift(l.clone(), left_w + 1)).collect(),
                    lines: right.into_iter().map(|(l, ..)| l).collect(),
                    x: left_w + 1,
                    w: right_w,
                },
            ],
            false => {
                let mut one = Side { x: 0, w: width, ..Side::default() };
                for (l, at, links) in left.into_iter().map(|(l, links)| (l, desc.clone(), links)).chain([(Line::default(), 0..0, vec![])]).chain(right) {
                    one.lines.push(l);
                    one.text.push(at);
                    one.links.push(links);
                }
                vec![one]
            }
        };
        // (Room at the end of each side, under the keys' box: the last lines scroll clear of it.)
        let mut sides = sides;
        for side in &mut sides {
            for _ in 0..PAD {
                side.lines.push(Line::default());
                side.text.push(0..0);
                side.links.push(vec![]);
            }
        }
        (self.sides, self.width, self.pane_x, self.sel) = (sides, width, two.then_some(left_w + 1), None);
        (self.select, self.lines_sel) = (false, false); // (what it was over may have moved)
        self.code_over = over.get();
    }

    /// How it closes, as its triage says (`--reason` of `gh issue close`, and for a duplicate,
    /// of what); not said, from its words: "fixed …" completed, else not planned.
    pub fn close_reason(&self) -> (String, Option<u64>) {
        let dup = self.triage["duplicate_of"].as_u64();
        match self.t("reason").as_str() {
            r @ ("completed" | "not planned") => (r.to_owned(), None),
            "duplicate" if dup.is_some() => ("duplicate".into(), dup),
            _ if self.t("close").to_lowercase().starts_with("fixed") => ("completed".into(), None),
            _ => ("not planned".into(), None),
        }
    }

    /// Its repo's URL (`https://github.com/o/r`).
    fn repo(&self) -> Option<&str> {
        self.url.rsplit_once("/issues/").map(|(r, _)| r)
    }

    /// The PR or issue of its repo link `u` is to (`/pull/N…`: a PR, `Some(true)`; `/issues/N…`:
    /// either; not this issue, nor another repo's).
    pub fn number_in(&self, u: &str) -> Option<(u64, bool)> {
        let t = u.strip_prefix(self.repo()?)?;
        let (rest, pr) = match t.strip_prefix("/pull/") {
            Some(r) => (r, true),
            None => (t.strip_prefix("/issues/")?, false),
        };
        let n = rest.split(['#', '/', '?']).next()?.parse::<u64>().ok()?;
        (n != self.number).then_some((n, pr))
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn scroll_at(&self) -> [usize; 2] {
        self.scroll
    }

    pub fn set_scroll(&mut self, at: [usize; 2]) {
        self.scroll = at;
    }

    /// Side `i`'s lines (none drawn yet: none).
    fn lines(&self, i: usize) -> &[Line<'static>] {
        self.sides.get(i).map_or(&[], |s| &s.lines)
    }

    /// The side at column `x` scrolled by `by` lines (the wheel: the side it's over).
    pub fn scroll(&mut self, x: usize, by: isize) {
        let i = self.side_of(x);
        let max = self.lines(i).len().saturating_sub(self.shown);
        self.scroll[i] = (self.scroll[i] as isize + by).clamp(0, max as isize) as usize;
    }

    pub fn draw(&mut self, f: &mut Frame, ui: &Ui, area: Rect) {
        let h = area.height as usize;
        if self.width != area.width as usize || self.sides.is_empty() {
            // (The cursor's side, and where on it, as it was: the pane moves with the width.)
            let (side, old_x) = (self.side_of(self.cur.1), self.pane_x);
            self.render(ui, area.width as usize);
            // (Fewer lines now, a narrower window wider: not past the end; the cursor where it
            // can be, on its side if it's still there, else the first.)
            for i in 0..2 {
                self.scroll[i] = self.scroll[i].min(self.lines(i).len().saturating_sub(h));
            }
            let (side, col) = match (side, old_x, self.pane_x) {
                (1, Some(o), Some(p)) => (1, p + self.cur.1.saturating_sub(o)),
                (1, ..) => (0, 0),
                _ => (0, self.cur.1),
            };
            self.cur = self.clamp(self.cur.0, col, side);
        }
        self.shown = h;
        f.render_widget(Paragraph::new("").style(ui.base), area);
        for (i, side) in self.sides.iter().enumerate() {
            let body: Vec<Line> = side.lines.iter().skip(self.scroll[i]).take(h).cloned().collect();
            f.render_widget(Paragraph::new(body).style(ui.base), Rect { x: area.x + side.x as u16, width: side.w as u16, ..area });
        }
        // Side by side, a dim line between, the whole height.
        if let Some(p) = self.pane_x {
            let line: Vec<Line> = (0..h).map(|_| Line::from(Span::styled("│", ui.linenr))).collect();
            f.render_widget(Paragraph::new(line), Rect { x: area.x + p as u16 - 1, width: 1, ..area });
        }
        // What's selected (dragged over, `v`, `x`), reversed; the cursor.
        for y in 0..area.height {
            for x in 0..area.width {
                if self.selected(self.scroll[self.side_of(x as usize)] + y as usize, x as usize) {
                    f.buffer_mut()[(area.x + x, area.y + y)].modifier.insert(ratatui::style::Modifier::REVERSED);
                }
            }
        }
        let (row, col) = self.cur;
        let top = self.scroll[self.side_of(col)];
        if (top..top + h).contains(&row) && col < area.width as usize {
            let style = if self.select { ui.cursor_select } else { ui.cursor };
            f.buffer_mut()[(area.x + col as u16, area.y + (row - top) as u16)].set_style(style);
        }
    }

    /// The side column `x` is on: 0 the description's, 1 the pane's (side by side; one column:
    /// 0, all of it).
    fn side_of(&self, x: usize) -> usize {
        self.pane_x.is_some_and(|p| x >= p) as usize
    }

    /// Line `row`'s text columns, on side `i`.
    fn text_on(&self, i: usize, row: usize) -> std::ops::Range<usize> {
        self.sides.get(i).and_then(|s| s.text.get(row).cloned()).unwrap_or(0..0)
    }

    /// Line `row`'s text columns on the cursor's side.
    fn text(&self, row: usize) -> std::ops::Range<usize> {
        self.text_on(self.side_of(self.cur.1), row)
    }

    /// Line `row` of the cursor's side, column by column across the screen: what each shows (a
    /// wide character's second, and the other side's: empty).
    fn cells(&self, row: usize) -> Vec<String> {
        self.cells_on(self.side_of(self.cur.1), row)
    }

    /// Line `row` of side `i`, as `cells`.
    fn cells_on(&self, i: usize, row: usize) -> Vec<String> {
        let Some(side) = self.sides.get(i) else { return vec![] };
        let w = self.width as u16;
        let mut buf = ratatui::buffer::Buffer::empty(Rect::new(0, 0, w, 1));
        if let Some(l) = side.lines.get(row) {
            buf.set_line(side.x as u16, 0, l, side.w as u16);
        }
        let mut covered: usize = 0;
        (0..self.width)
            .map(|x| {
                let sym = buf[(x as u16, 0)].symbol().to_owned();
                if covered > 0 || !(side.x..side.x + side.w).contains(&x) {
                    covered = covered.saturating_sub(1);
                    return String::new();
                }
                covered = Span::raw(sym.as_str()).width().saturating_sub(1);
                sym
            })
            .collect()
    }

    /// The cursor to (`row`, `col`), kept in that line's text on its side (or the side's
    /// start, a line with none there), in view; `v` on: the selection follows it.
    fn go(&mut self, row: usize, col: usize) {
        self.cur = self.clamp(row, col, self.side_of(self.cur.1));
        self.lines_sel = false;
        match (self.select, &mut self.sel) {
            (true, Some((_, b))) => *b = self.cur,
            (true, None) => self.sel = Some((self.cur, self.cur)),
            (false, _) => self.sel = None,
        }
        // (In view on its side, three lines from its edges where there are.)
        let (row, i) = (self.cur.0, self.side_of(self.cur.1));
        let (h, n) = (self.shown.max(1), self.lines(i).len());
        let off = 3.min(h / 2);
        if row < self.scroll[i] + off {
            self.scroll[i] = row.saturating_sub(off);
        } else if row + off >= self.scroll[i] + h {
            self.scroll[i] = (row + off + 1).saturating_sub(h).min(n.saturating_sub(h));
        }
    }

    /// (`row`, `col`) as a cursor can be there: in that line's text on `side`, or that side's
    /// columns on a line with none there.
    fn clamp(&self, row: usize, col: usize, side: usize) -> (usize, usize) {
        let row = row.min(self.lines(side).len().saturating_sub(1 + PAD));
        let t = self.text_on(side, row);
        // (The side's own columns, text there or not: not past it, nor into the other.)
        let cols = match (side, self.pane_x) {
            (1, Some(p)) => p..self.width.max(p + 1),
            (_, Some(p)) => 0..p.saturating_sub(1).max(1),
            _ => 0..self.width.max(1),
        };
        let col = if t.is_empty() { col.clamp(cols.start, cols.end - 1) } else { col.clamp(t.start, t.end - 1) };
        (row, col)
    }

    /// The next (`fwd`) or previous word's start (`end`: its end) from the cursor, as Helix's
    /// `w`, `b`, `e`: on through the lines of the cursor's side.
    fn word(&self, fwd: bool, end: bool) -> (usize, usize) {
        let class = |s: &str| match s.chars().next() {
            None => 0,
            Some(c) if c.is_whitespace() => 0,
            Some(c) if c.is_alphanumeric() || c == '_' => 1,
            Some(_) => 2,
        };
        // Every column of the side's text, in reading order, with its class.
        let (row0, col0) = self.cur;
        let n = self.lines(self.side_of(col0)).len();
        let rows: Box<dyn Iterator<Item = usize>> = if fwd { Box::new(row0..n) } else { Box::new((0..=row0).rev()) };
        let mut seq = vec![];
        for row in rows.take(200) {
            let cells = self.cells(row);
            let t = self.text(row);
            let mut cols: Vec<(usize, usize)> = t.filter(|&x| !cells.get(x).is_some_and(String::is_empty)).map(|x| (row, x)).collect();
            if !fwd {
                cols.reverse();
            }
            let line: Vec<((usize, usize), u8)> = cols.into_iter().map(|p| (p, class(cells.get(p.1).map_or("", String::as_str)))).collect();
            seq.extend(line);
            seq.push(((row, usize::MAX), 0)); // (a line's end: a space)
        }
        let at = seq.iter().position(|(p, _)| *p == (row0, col0)).unwrap_or(0);
        let mut i = at + 1;
        let is_start = |i: usize| seq[i].1 != 0 && (i == 0 || seq[i - 1].1 != seq[i].1);
        let is_end = |i: usize| seq[i].1 != 0 && (i + 1 == seq.len() || seq[i + 1].1 != seq[i].1);
        while i < seq.len() {
            let hit = match (fwd, end) {
                (true, false) => is_start(i),
                (true, true) => is_end(i),
                // (Back: the start of a word is where the reversed run ends.)
                (false, _) => is_end(i),
            };
            if hit {
                return seq[i].0;
            }
            i += 1;
        }
        self.cur
    }

    /// A key in the view, Helix's: `h j k l` (and arrows), `w b e`, `gh gl`, `gg ge`,
    /// `Ctrl-d Ctrl-u`, `Tab` to the other side; `v` selects, `x` the line (again: the next
    /// too), `;` drops it, `y` copies it; Enter opens the link under the cursor.
    pub fn key(&mut self, pending: Option<char>, k: KeyCode, ctrl: bool) -> Did {
        let (row, col) = self.cur;
        let half = (self.shown / 2).max(1);
        match (pending, k) {
            (_, KeyCode::Char('d')) if ctrl => self.go(row + half, col),
            (_, KeyCode::Char('u')) if ctrl => self.go(row.saturating_sub(half), col),
            // Ctrl-h / Ctrl-l: to the description / the pane (as the PR view's sides).
            (_, KeyCode::Char(c @ ('h' | 'l'))) if ctrl => self.go_side(c == 'l'),
            _ if ctrl => return Did::Not,
            (Some('g'), KeyCode::Char('g')) => self.go(0, col),
            (Some('g'), KeyCode::Char('e')) => self.go(usize::MAX, col),
            (Some('g'), KeyCode::Char('h')) => self.go(row, 0),
            (Some('g'), KeyCode::Char('l')) => {
                // (Its last character, not the space after.)
                let cells = self.cells(row);
                let last = self.text(row).rev().find(|&x| cells.get(x).is_some_and(|c| !c.trim().is_empty()));
                self.go(row, last.unwrap_or(col));
            }
            (_, KeyCode::Char('j') | KeyCode::Down) => self.go(row + 1, col),
            (_, KeyCode::Char('k') | KeyCode::Up) => self.go(row.saturating_sub(1), col),
            (_, KeyCode::Char('h') | KeyCode::Left) => self.go(row, col.saturating_sub(1)),
            (_, KeyCode::Char('l') | KeyCode::Right) => self.go(row, col + 1),
            (_, KeyCode::Char(c @ ('w' | 'b' | 'e'))) => {
                let (r, x) = self.word(c != 'b', c == 'e');
                self.go(r, x);
            }
            (_, KeyCode::Tab) => self.go_side(self.side_of(col) == 0),
            // (Many terminals send Ctrl-h as Backspace, as the PR view has it.)
            (_, KeyCode::Backspace) => self.go_side(false),
            (_, KeyCode::Char('v')) => {
                (self.select, self.lines_sel) = (!self.select, false);
                self.sel = self.select.then_some((self.cur, self.cur));
            }
            (_, KeyCode::Char('x')) => {
                // The line's text; again (`x`'s lines selected): on to the next with text.
                let (anchor, mut to) = match (self.lines_sel, self.sel) {
                    (true, Some((a, _))) => (a, row + 1),
                    _ => ((row, self.text(row).start), row),
                };
                // (No line after the last: it stays, the selection whole.)
                let last = self.lines(self.side_of(col)).len().saturating_sub(1 + PAD);
                to = to.min(last);
                while to < last && self.text(to).is_empty() {
                    to += 1;
                }
                let end = self.text(to).end.saturating_sub(1);
                self.select = false;
                self.go(to, end);
                (self.sel, self.lines_sel) = (Some((anchor, self.cur)), true);
            }
            (_, KeyCode::Char(';')) => (self.select, self.sel, self.lines_sel) = (false, None, false),
            (_, KeyCode::Char('y')) => {
                let text = self.selection();
                (self.select, self.sel) = (false, None);
                return match text.trim().is_empty() {
                    true => Did::Nothing,
                    false => Did::Copy(text),
                };
            }
            (_, KeyCode::Enter) => {
                let links = self.sides.get(self.side_of(col)).and_then(|s| s.links.get(row));
                let link = links.and_then(|l| l.iter().find(|(r, _)| r.contains(&col)));
                return link.map_or(Did::Nothing, |(_, u)| Did::Open(u.clone()));
            }
            _ => return Did::Not,
        }
        Did::Nothing
    }

    /// The cursor to the pane (`pane`) or the description, on its line (side by side).
    fn go_side(&mut self, pane: bool) {
        let Some(p) = self.pane_x else { return };
        // (At the same height on screen: the sides' rows are their own.)
        let (from, to) = (self.side_of(self.cur.1), pane as usize);
        let row = self.scroll[to] + self.cur.0.saturating_sub(self.scroll[from]);
        // (A selection's rows are its side's: not carried over to the other.)
        (self.select, self.sel, self.lines_sel) = (false, None, false);
        self.cur.1 = if pane { p } else { 0 };
        self.go(row, self.cur.1);
    }

    /// The issue's title.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// What's selected (none: the cursor's line, on its side), as text.
    pub fn picked_text(&self) -> String {
        if self.sel.is_some_and(|(a, b)| a != b) {
            return self.selection();
        }
        let row = self.cur.0;
        let cells = self.cells(row);
        self.text(row).map(|x| cells.get(x).map_or("", String::as_str)).collect::<String>().trim_end().to_owned()
    }

    /// Whether there's a selection to drop (`Esc` drops it before leaving).
    pub fn selecting(&self) -> bool {
        self.select || self.sel.is_some()
    }

    pub fn drop_selection(&mut self) {
        (self.select, self.sel) = (false, None);
    }

    /// Whether (line, column) is dragged over: from where the drag began to where it is, as text
    /// runs (whole lines between), in the text of the side it began on.
    fn selected(&self, row: usize, x: usize) -> bool {
        let Some((from, to)) = self.sel else { return false };
        let (a, b) = if from <= to { (from, to) } else { (to, from) };
        self.text_on(self.side_of(from.1), row).contains(&x) && (row, x) >= a && (row, x) <= b
    }

    /// The mouse, at (column, row) of the issue's area (rows past it: its last): a press begins a
    /// drag, a drag moves its end, and the release gives the text dragged over, to copy (a click:
    /// nothing, and nothing kept).
    pub fn mouse(&mut self, kind: ratatui::crossterm::event::MouseEventKind, x: usize, y: usize) -> Option<String> {
        use ratatui::crossterm::event::{MouseButton, MouseEventKind};
        // A drag at the edge (or past it): its side scrolls on, a line each move.
        if let (MouseEventKind::Drag(_), Some(((_, ax), _))) = (kind, self.sel) {
            match y {
                0 => self.scroll(ax, -1),
                y if y >= self.shown => self.scroll(ax, 1),
                _ => {}
            }
        }
        // (A row on the side it's over; a drag's, on the side it began on.)
        let y = y.min(self.shown.saturating_sub(1));
        let side = self.sel.filter(|_| matches!(kind, MouseEventKind::Drag(_) | MouseEventKind::Up(_))).map_or(self.side_of(x), |((_, ax), _)| self.side_of(ax));
        let at = (self.scroll[side] + y, x);
        match kind {
            MouseEventKind::Down(MouseButton::Left) => {
                // The cursor there, kept on that side's columns (the view where it is: a drag
                // goes on from the row under the mouse); the drag from the very point.
                self.cur = self.clamp(at.0, at.1, self.side_of(at.1));
                (self.select, self.lines_sel, self.sel) = (false, false, Some((at, at)));
            }
            // Sideways: the code blocks and tables of the side it's over.
            MouseEventKind::ScrollLeft | MouseEventKind::ScrollRight => {
                let side = if self.pane_x.is_some() { self.side_of(x) } else { (at.0 >= self.desc_rows) as usize };
                let by = if kind == MouseEventKind::ScrollRight { 8 } else { -8 };
                self.code_left[side] = (self.code_left[side] as isize + by).clamp(0, self.code_over[side] as isize) as usize;
                self.redraw();
            }
            MouseEventKind::Drag(MouseButton::Left) | MouseEventKind::Up(MouseButton::Left) if self.sel.is_some() => {
                let ax = self.sel.map_or(at.1, |((_, x), _)| x);
                if let Some((_, b)) = &mut self.sel {
                    *b = at;
                }
                // (On the side the drag began on, as what it selects is.)
                self.cur = self.clamp(at.0, at.1, self.side_of(ax));
                if kind == MouseEventKind::Drag(MouseButton::Left) {
                    return None;
                }
                self.sel = self.sel.filter(|(a, b)| a != b);
                return self.sel.map(|_| self.selection());
            }
            _ => {}
        }
        None
    }

    /// The text dragged over, line by line.
    fn selection(&self) -> String {
        let Some(((r0, _), (r1, _))) = self.sel.map(|(a, b)| if a <= b { (a, b) } else { (b, a) }) else { return String::new() };
        let side = self.sel.map_or(0, |((_, ax), _)| self.side_of(ax));
        let out: Vec<String> = (r0..=r1.min(self.lines(side).len().saturating_sub(1)))
            .map(|row| {
                let text: String = self.cells_on(side, row).iter().enumerate().filter(|(x, _)| self.selected(row, *x)).map(|(_, c)| c.as_str()).collect();
                text.trim_end().to_owned()
            })
            .collect();
        out.join("\n").trim_matches('\n').to_owned()
    }

    /// Forget the lines drawn: the issue or its triage changed.
    pub fn redraw(&mut self) {
        self.sides.clear();
    }

    /// Its triage, read again (Claude's just written it).
    pub fn reload_triage(&mut self, dir: Option<&std::path::Path>) {
        if let Some(t) = dir.and_then(|d| std::fs::read_to_string(d.join(format!("issue-{}.json", self.number))).ok()).and_then(|t| serde_json::from_str(&t).ok()) {
            self.triage = t;
            self.redraw();
        }
    }
}

pub(super) const VIEW_HELP: &str = "
MOVE
  h j k l, arrows      move the cursor (Helix's); w b e by word; gh gl the line's start / end
  Ctrl-d Ctrl-u        half a page down / up
  gg ge                top / end
  Tab, Ctrl-h Ctrl-l   to the other side / the description / the pane
  v, x                 select from here; the line (again: the next too); ; drops it
  y                    copy what's selected
  Enter                open the link under the cursor (this repo's issue: here; its PR:
                       reviewed as dv list does, checked out, or in a worktree with -w)
  mouse                wheel scrolls the side it's over (Shift or sideways: the code
                       blocks); a drag copies
                       the text it's over (one side's)
  q, Esc               back to the issues
  ?                    this help

ON GITHUB
  C                    comment on it (your editor first)
  Space l              its labels and assignees (Enter toggles one; triage access)
  Space N              Claude drafts a new PR from what's selected (or the line): your editor
                       first (N N: a PR fixing the whole issue)
  L L                  add the labels Claude's triage suggests
  T T                  give it the title Claude's triage suggests
  X X                  close it, with Claude's closing comment (your editor first; none
                       yet: Claude drafts one, then X X closes with it)
  o                    open it in the browser

CLAUDE
  a a                  triage it (again)
  N N                  Claude drafts a PR fixing it (unpublished, on dv list's drafts tab)";

#[test]
fn an_issue_and_its_triage() {
    use crate::view::tests::{screen, snapshot};
    let triage = serde_json::json!({"kind": "bug", "priority": "high", "summary": "Still on master: `src/a.rs:12` unwraps.",
        "labels": ["bug", "chain"], "title": "LocalChain panics on reorg past genesis", "next": "review #9",
        "close": "fixed by #9", "closing": "Fixed by #9, thanks!"});
    let mut v = IssueView {
        number: 7, title: "panic".into(), author: "amy".into(), url: "https://github.com/o/r/issues/7".into(),
        body: "It **panics**.".into(), labels: vec![("bug".into(), "d73a4a".into())],
        activity: vec![Act::Comment("bo".into(), String::new(), "Same here.".into())], triage, scroll: [0; 2], sides: vec![], width: 0, pane_x: None, shown: 0, cur: (0, 2), select: false, lines_sel: false, desc_rows: 0, code_left: [0; 2], code_over: [0; 2], can_triage: true, drafts: vec!["new PR 3: Claude is drafting it".into()], sel: None,
        hl: crate::highlight::Highlighter::default(),
    };
    assert_eq!(v.new_labels(), ["chain"]); // bug it has
    assert_eq!(v.new_title().as_deref(), Some("LocalChain panics on reorg past genesis"));
    assert_eq!(v.closing().as_deref(), Some("Fixed by #9, thanks!"));
    assert_eq!(v.close_reason(), ("completed".into(), None)); // "fixed by …"
    v.triage["reason"] = "duplicate".into();
    v.triage["duplicate_of"] = 3.into();
    assert_eq!(v.close_reason(), ("duplicate".into(), Some(3)));
    assert!(v.fix_task().starts_with("Fix #7: panic\n\nFrom triaging it:\nStill on master"));
    let ui = Theme::load().ui();
    snapshot("issue_view", &screen(80, 22, |f| {
        let a = f.area();
        v.title_bar(f, &ui, Rect { height: 1, ..a });
        v.draw(f, &ui, Rect { y: 1, height: a.height - 1, ..a });
    }));
    // Wide: the description, and the pane beside it (triage, then the conversation in bubbles).
    v.redraw();
    snapshot("issue_view_wide", &screen(160, 22, |f| {
        let a = f.area();
        v.title_bar(f, &ui, Rect { height: 1, ..a });
        v.draw(f, &ui, Rect { y: 1, height: a.height - 1, ..a });
    }));
    // A drag in the pane copies the comment's text, not the description beside it nor the bubble.
    use ratatui::crossterm::event::{MouseButton, MouseEventKind};
    let row = v.lines(1).iter().position(|l| l.to_string().contains("Same here.")).unwrap();
    let p = v.pane_x.unwrap();
    v.shown = v.lines(0).len().max(v.lines(1).len()); // (all of it on screen: rows are lines)
    v.mouse(MouseEventKind::Down(MouseButton::Left), p, row);
    v.mouse(MouseEventKind::Drag(MouseButton::Left), 159, row);
    assert_eq!(v.mouse(MouseEventKind::Up(MouseButton::Left), 159, row).as_deref(), Some("Same here."));
    // From above the bubble to below it: its text, not its frame.
    v.mouse(MouseEventKind::Down(MouseButton::Left), p, row - 1);
    assert_eq!(v.mouse(MouseEventKind::Up(MouseButton::Left), 159, row + 1).as_deref(), Some("Same here."));
    // The description, from its margin: its text.
    let row = v.lines(0).iter().position(|l| l.to_string().contains("It panics.")).unwrap();
    v.mouse(MouseEventKind::Down(MouseButton::Left), 0, row);
    assert_eq!(v.mouse(MouseEventKind::Up(MouseButton::Left), p - 2, row).as_deref(), Some("It panics."));
    // The cursor (Helix's): Enter on the link under it opens it; `x` then `y` copies its line;
    // `w` to the next word; `Tab` to the pane.
    v.cur = (0, 2);
    assert!(matches!(v.key(None, KeyCode::Enter, false), Did::Open(u) if u == "https://github.com/o/r/issues/7"));
    let row = v.lines(0).iter().position(|l| l.to_string().contains("It panics.")).unwrap();
    v.go(row, 0);
    assert_eq!(v.cur, (row, 2)); // (its text, past the margin)
    v.key(None, KeyCode::Char('w'), false);
    assert_eq!(v.cur, (row, 5)); // "panics"
    v.key(None, KeyCode::Char('x'), false);
    assert!(matches!(v.key(None, KeyCode::Char('y'), false), Did::Copy(t) if t == "It panics."));
    // Space N's text: the selection, else the cursor's line.
    let row = v.lines(0).iter().position(|l| l.to_string().contains("It panics.")).unwrap();
    (v.cur, v.sel) = ((row, 2), None);
    assert_eq!(v.picked_text(), "It panics.");
    v.key(None, KeyCode::Tab, false);
    assert!(v.cur.1 >= v.pane_x.unwrap());
    v.key(None, KeyCode::Char('v'), false);
    v.key(None, KeyCode::Char('j'), false);
    v.key(None, KeyCode::Tab, false); // (a selection stays on its side: dropped)
    assert!(v.sel.is_none() && !v.select);
    v.key(None, KeyCode::Char('h'), true); // Ctrl-h: the description
    assert!(v.cur.1 < v.pane_x.unwrap());
    v.key(None, KeyCode::Char('l'), true); // Ctrl-l: the pane
    assert!(v.cur.1 >= v.pane_x.unwrap());
    // `x` again: on through lines with no text (a bubble's frame), not dropping what it had.
    let row = v.lines(1).iter().position(|l| l.to_string().contains("Same here.")).unwrap();
    v.go(row, 0);
    v.key(None, KeyCode::Char('x'), false);
    v.key(None, KeyCode::Char('x'), false);
    let (a, b) = v.sel.unwrap();
    assert!(a.0 == row && b.0 > row, "{a:?} {b:?}");
    // On a line with no text on its side, `l` stays on that side's columns.
    v.go(row + 1, 0); // (the bubble's frame)
    for _ in 0..300 {
        v.key(None, KeyCode::Char('l'), false);
    }
    assert!(v.cur.1 < v.width && v.side_of(v.cur.1) == 1);
    // Sideways over the description: its code, from its start.
    v.body = format!("```\n{}\n{}\n```\n\n| ab | cd |\n|---|---|\n| 1 | 2 |", "x".repeat(300), "y".repeat(110));
    v.redraw();
    let _ = screen(160, 22, |f| v.draw(f, &ui, f.area()));
    assert!(v.lines(0).iter().any(|l| l.to_string().contains("  xxx")));
    v.cur = v.clamp(4, 3, 0);
    let at = v.cur;
    assert!(at.0 > 0);
    v.mouse(MouseEventKind::ScrollRight, 2, 0);
    let _ = screen(160, 22, |f| v.draw(f, &ui, f.area()));
    assert_eq!((v.code_left, v.cur), ([8, 0], at)); // (the cursor where it was)
    // (Cut both ways now: `‹` and `›` say there's more.)
    let code = v.lines(0).iter().map(|l| l.to_string()).find(|l| l.contains("xxx")).unwrap();
    assert!(code.contains("  ‹x") && code.contains("x›"), "{code}");
    // (The table beside it fits: it stays put.)
    assert!(v.lines(0).iter().any(|l| l.to_string().contains("ab │ cd")));
    // (Far right: a shorter line of the block scrolled past its end, no overflow.)
    for _ in 0..30 {
        v.mouse(MouseEventKind::ScrollRight, 2, 0);
        let _ = screen(160, 22, |f| v.draw(f, &ui, f.area()));
    }
    v.code_left = [8, 0];
    v.mouse(MouseEventKind::ScrollLeft, 2, 0);
    v.mouse(MouseEventKind::ScrollLeft, 2, 0);
    assert_eq!(v.code_left, [0, 0]);
    let _ = screen(160, 22, |f| v.draw(f, &ui, f.area()));
    // `ge x x y` on the last line: still the whole line.
    v.go(usize::MAX, 0);
    v.key(None, KeyCode::Char('x'), false);
    v.key(None, KeyCode::Char('x'), false);
    let (a, b) = v.sel.unwrap();
    assert!(a.0 == b.0 && b.1 > a.1, "{a:?} {b:?}");
    // A click in the margin: the cursor on the text beside it, nothing copied, the view still.
    let _ = screen(160, 22, |f| v.draw(f, &ui, f.area()));
    (v.scroll, v.shown) = ([0; 2], 20);
    v.mouse(MouseEventKind::Down(MouseButton::Left), 0, 0); // (the issue's link line, "  https://…")
    assert_eq!((v.mouse(MouseEventKind::Up(MouseButton::Left), 0, 0), v.cur, v.scroll), (None, (0, 2), [0; 2]));
    v.mouse(MouseEventKind::Down(MouseButton::Left), 2, 19); // (near the bottom: no scroll)
    assert_eq!(v.scroll, [0; 2]);
    // A link: this repo's PR or issue (not this one, nor another repo's).
    assert_eq!(v.number_in("https://github.com/o/r/pull/12/files"), Some((12, true)));
    assert_eq!(v.number_in("https://github.com/o/r/issues/12#issuecomment-1"), Some((12, false)));
    assert_eq!(v.number_in("https://github.com/o/r/issues/7"), None);
    assert_eq!(v.number_in("https://github.com/o/r2/issues/3"), None);
    assert_eq!(v.number_in("https://github.com/o/r/issues/new"), None);
    // Each side scrolls on its own: the wheel over the pane moves the pane only.
    let p = v.pane_x.unwrap();
    v.shown = 3;
    v.scroll(p + 1, 2);
    assert_eq!(v.scroll, [0, 2]);
    v.scroll(1, 1);
    assert_eq!(v.scroll, [1, 2]);
    // A click: nothing, and nothing left selected.
    v.mouse(MouseEventKind::Down(MouseButton::Left), 2, 0);
    assert_eq!(v.mouse(MouseEventKind::Up(MouseButton::Left), 2, 0), None);
    assert!(v.sel.is_none());
    // A drag past the bottom scrolls its side on; at the top, back.
    v.mouse(MouseEventKind::Down(MouseButton::Left), 2, 1);
    v.mouse(MouseEventKind::Drag(MouseButton::Left), 2, 5);
    assert_eq!(v.scroll, [2, 2]);
    v.mouse(MouseEventKind::Drag(MouseButton::Left), 2, 0);
    assert_eq!(v.scroll, [1, 2]);
}

#[test]
fn an_issues_timeline() {
    let nodes = serde_json::json!([
        { "__typename": "IssueComment", "createdAt": "2026-01-01T00:00:00Z", "author": { "login": "amy" }, "body": "it panics" },
        { "__typename": "LabeledEvent", "createdAt": "2026-01-02T00:00:00Z", "actor": { "login": "bo" }, "label": { "name": "bug" } },
        { "__typename": "CrossReferencedEvent", "createdAt": "2026-01-03T00:00:00Z", "actor": { "login": "cy" },
          "source": { "number": 9, "title": "Fix the panic", "url": "https://github.com/o/r/pull/9" } },
        { "__typename": "ClosedEvent", "createdAt": "2026-01-04T00:00:00Z", "actor": { "login": "bo" }, "stateReason": "COMPLETED",
          "closer": { "number": 9, "url": "https://github.com/o/r/pull/9" } },
        { "__typename": "CrossReferencedEvent", "createdAt": "2026-01-05T00:00:00Z", "actor": null,
          "source": { "number": 4, "title": "Elsewhere", "url": "https://github.com/o/w/issues/4", "repository": { "nameWithOwner": "o/w" } } },
        { "__typename": "SubscribedEvent" },
    ]);
    let a = acts(nodes.as_array().unwrap(), "o/r");
    assert!(matches!(&a[0], Act::Comment(who, _, body) if who == "amy" && body == "it panics"));
    let said: Vec<String> = a[1..].iter().map(|e| match e { Act::Event(who, what, _, url) => format!("@{who} {what} {}", url.as_deref().unwrap_or("")), _ => String::new() }).collect();
    assert_eq!(said, ["@bo added the label bug ", "@cy mentioned this in #9 Fix the panic https://github.com/o/r/pull/9", "@bo closed it with #9 (completed) https://github.com/o/r/pull/9",
        "@ghost mentioned this in o/w#4 Elsewhere https://github.com/o/w/issues/4"]); // (another repo's, named so; a deleted account's)
    assert_eq!(url_repo("https://github.com/o/r/issues/7"), "o/r");
}
