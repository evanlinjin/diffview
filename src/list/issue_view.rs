//! An issue in dv (Enter in `dv issues`): its text and conversation, Claude's triage under them,
//! and what to do about it: comment (`C`), apply Claude's labels (`L L`) or title (`T T`),
//! close it with Claude's comment (`X X`, after your editor), have Claude draft a PR fixing it
//! (`N N`), or triage it again (`a a`).
use super::*;
use serde_json::Value;

/// An issue as dv shows it: as GitHub has it, and Claude's triage (all of it: the summary,
/// labels, title and closing comment too, not just what its card shows).
pub struct IssueView {
    pub number: u64,
    title: String,
    author: String,
    url: String,
    body: String,
    labels: Vec<(String, String)>,
    /// (who, when, what), oldest first.
    comments: Vec<(String, String, String)>,
    triage: Value,
    scroll: usize,
    /// The lines as last drawn, and at what width.
    lines: Vec<Line<'static>>,
    width: usize,
    hl: crate::highlight::Highlighter,
}

impl IssueView {
    /// Issue `n` from GitHub, with its triage from `dir` (the reviews' directory).
    pub fn load(n: u64, dir: Option<&std::path::Path>) -> Result<IssueView> {
        let v: Value = gh::gh_json(&["issue", "view", &n.to_string(), "--json", "number,title,author,url,body,labels,comments"])?;
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
            comments: v["comments"].as_array().into_iter().flatten().map(|c| (s(&c["author"]["login"]), s(&c["createdAt"]), s(&c["body"]).replace("\r\n", "\n"))).collect(),
            triage,
            scroll: 0,
            lines: vec![],
            width: 0,
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
        let repo = self.url.rsplit_once("/issues/").map(|(r, _)| r.to_owned());
        // (A code block's line in its background to the width given, as hover popups have it.)
        let code_bg = ui.md.code_bg;
        let md = |text: &str, w: usize| -> Vec<Line<'static>> {
            crate::md::render(text, w, &ui.md, &self.hl, repo.as_deref())
                .into_iter()
                .map(|l| {
                    let code = code_bg.filter(|&bg| l.nowrap && l.styles.last().copied().flatten().and_then(|s| s.bg) == Some(bg));
                    let mut line = l.into_line();
                    if let Some(bg) = code {
                        line.spans.push(Span::styled(" ".repeat(w.saturating_sub(line.width())), Style::new().bg(bg)));
                    }
                    line
                })
                .collect()
        };
        let dim = ui.stale;
        let border = ui.linenr;
        // A bubble: `╭─ title ──╮`, its lines between `│ … │`, `╰──╯`, `w` wide.
        let bubble = |title: Vec<Span<'static>>, body: Vec<Line<'static>>, w: usize| -> Vec<Line<'static>> {
            let tw: usize = title.iter().map(Span::width).sum();
            let mut top = vec![Span::styled("╭─ ", border)];
            top.extend(title);
            top.push(Span::styled(format!(" {}╮", "─".repeat(w.saturating_sub(tw + 5))), border));
            let mut out = vec![Line::from(top)];
            for l in body {
                // (A code block or table line wider than the bubble: cut at its edge.)
                let mut v = vec![Span::styled("│ ", border)];
                v.extend(super::card::fit(l.spans, w.saturating_sub(4), Style::new()));
                v.push(Span::styled(" │", border));
                out.push(Line::from(v));
            }
            out.push(Line::from(Span::styled(format!("╰{}╯", "─".repeat(w.saturating_sub(2))), border)));
            out
        };
        // The description, as the PR view's text.
        let mut left = vec![Line::from(Span::styled(format!("  {}", self.url), dim)), Line::default(), Line::from(Span::styled("  description", ui.heading.add_modifier(ratatui::style::Modifier::BOLD)))];
        let body = if self.body.trim().is_empty() { "_(no description)_" } else { &self.body };
        left.extend(md(body, left_w.saturating_sub(4).max(20)).into_iter().map(|mut l| {
            l.spans.insert(0, Span::raw("  "));
            l
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
                true => md(&self.t("summary"), inner),
                false => vec![Line::from(Span::styled("none yet: a a triages it", dim))],
            };
            {
                // (Wrapped as markdown: a long `next` stays in the bubble.)
                let mut said = |k: &str, v: String| {
                    if !v.is_empty() {
                        b.push(Line::default());
                        b.extend(md(&format!("**{k}:** {v}"), inner));
                    }
                };
                said("next", self.t("next"));
                said("labels to add (L L)", self.new_labels().join(", "));
                said("title (T T)", self.new_title().unwrap_or_default());
                said("close (X X)", self.t("close"));
                if let Some(c) = self.closing().filter(|c| !c.is_empty()) {
                    b.extend([Line::default(), Line::from(Span::styled("closing comment:", ui.key))]);
                    b.extend(md(&c, inner));
                }
            }
            right.extend(bubble(claude(title), b, right_w));
        }
        right.push(Line::default());
        right.push(Line::from(Span::styled(format!("activity ({})", self.comments.len()), ui.heading)));
        for (who, when, body) in &self.comments {
            let mut title = vec![Span::styled(format!("@{who}"), ui.thread)];
            if !when.is_empty() {
                title.push(Span::styled(format!(" · {}", gh::ago(when)), dim));
            }
            right.extend(bubble(title, md(body, inner), right_w));
        }
        // Side by side, a dim line between; or one after the other.
        let out = match two {
            true => (0..left.len().max(right.len()))
                .map(|k| {
                    // (The description cut at its column: a wide code line doesn't push the pane.)
                    let mut l = Line::from(super::card::fit(left.get(k).cloned().unwrap_or_default().spans, left_w, Style::new()));
                    l.spans.push(Span::styled("│", border));
                    l.spans.extend(right.get(k).cloned().unwrap_or_default().spans);
                    l
                })
                .collect(),
            false => {
                left.push(Line::default());
                left.extend(right);
                left
            }
        };
        (self.lines, self.width) = (out, width);
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

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn scroll_at(&self) -> usize {
        self.scroll
    }

    pub fn set_scroll(&mut self, at: usize) {
        self.scroll = at;
    }

    pub fn scroll(&mut self, by: isize, height: usize) {
        let max = self.lines.len().saturating_sub(height);
        self.scroll = (self.scroll as isize + by).clamp(0, max as isize) as usize;
    }

    pub fn draw(&mut self, f: &mut Frame, ui: &Ui, area: Rect) {
        if self.width != area.width as usize || self.lines.is_empty() {
            self.render(ui, area.width as usize);
            // (Fewer lines now, a narrower window wider: not past the end.)
            self.scroll = self.scroll.min(self.lines.len().saturating_sub(area.height as usize));
        }
        let body: Vec<Line> = self.lines.iter().skip(self.scroll).take(area.height as usize).cloned().collect();
        f.render_widget(Paragraph::new(body).style(ui.base), area);
    }

    /// Forget the lines drawn: the issue or its triage changed.
    pub fn redraw(&mut self) {
        self.lines.clear();
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
  j k, Up Down         scroll
  Ctrl-d Ctrl-u        half a page down / up
  gg ge                top / end
  q, Esc               back to the issues
  ?                    this help

ON GITHUB
  C                    comment on it (your editor first)
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
        comments: vec![("bo".into(), String::new(), "Same here.".into())], triage, scroll: 0, lines: vec![], width: 0,
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
}
