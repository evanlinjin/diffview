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

    /// The issue and Claude's triage as lines `width` wide: its title, labels and who; its
    /// text; each comment; then the triage.
    fn render(&mut self, ui: &Ui, width: usize) {
        let w = width.saturating_sub(4).clamp(20, 100);
        // (`#123` linked to the repo's, as the PR view's.)
        let repo = self.url.rsplit_once("/issues/").map(|(r, _)| r.to_owned());
        let md = |text: &str, out: &mut Vec<Line<'static>>| {
            for l in crate::md::render(text, w, &ui.md, &self.hl, repo.as_deref()) {
                let mut line = l.into_line();
                line.spans.insert(0, Span::raw("  "));
                out.push(line);
            }
        };
        let head = |text: String, out: &mut Vec<Line<'static>>| out.push(Line::from(Span::styled(text, ui.heading.add_modifier(ratatui::style::Modifier::BOLD))));
        let dim = ui.stale;
        // (Its number, title, who and labels: on its title bar.)
        let mut out = vec![Line::from(Span::styled(format!("  {}", self.url), dim)), Line::default()];
        // Claude's triage first, where it's seen (then the issue, however long its thread).
        match self.triage.is_null() {
            true => head("claude's triage: none yet (a a)".into(), &mut out),
            false => {
                let what = [self.t("kind"), self.t("priority")].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
                head(format!("claude's triage · {what}"), &mut out);
                md(&self.t("summary"), &mut out);
                let mut said = |k: &str, v: String| {
                    if !v.is_empty() {
                        out.push(Line::from(vec![Span::styled(format!("  {k}: "), ui.key), Span::raw(v)]));
                    }
                };
                said("next", self.t("next"));
                said("labels to add (L L)", self.new_labels().join(", "));
                said("title (T T)", self.new_title().unwrap_or_default());
                said("close (X X)", self.t("close"));
                if let Some(c) = self.closing().filter(|c| !c.is_empty()) {
                    out.push(Line::from(Span::styled("  closing comment:", ui.key)));
                    md(&c, &mut out);
                }
            }
        }
        out.push(Line::default());
        out.push(Line::from(Span::styled("─".repeat(w + 2), dim)));
        out.push(Line::default());
        md(if self.body.trim().is_empty() { "_(no description)_" } else { &self.body }, &mut out);
        for (who, when, body) in &self.comments {
            out.push(Line::default());
            head(format!("@{who} · {}", gh::ago(when)), &mut out);
            md(body, &mut out);
        }
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
}
