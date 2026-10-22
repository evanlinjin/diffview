//! The PR's text as rows: its description and sections, the facts box, its activity, Claude's
//! review and activity, your pending review's text, a draft's requests; what of it is hidden.
use super::*;

/// A thing Claude did, in its activity: when, what (and where), what it said, and the comment
/// Enter goes to.
type Did<'a> = (&'a str, String, &'a str, Option<(usize, usize)>);

impl App {
    /// A section of PR text in the diff's column: a header like a file's (`name` folds it),
    /// then `text` rendered to the code's width (`info`: the `info` line it is, for `C`).
    pub(super) fn section(&self, id: usize, title: String, text: &str, info: Option<usize>) -> Vec<Row> {
        self.section_with(id, title, info, |mk| self.md_rows_at(text, self.wrap_w, mk))
    }

    /// A section (`section`) whose body `body` makes, with its rows' maker.
    pub(super) fn section_with(&self, id: usize, title: String, info: Option<usize>, body: impl FnOnce(&dyn Fn(Kind, String) -> Row) -> Vec<Row>) -> Vec<Row> {
        let mk = |kind, text| Row { main: Some(id), info, ..Row::text(INFO, kind, text) };
        let mut rows = vec![mk(Kind::File, title)];
        if !self.sections_folded.contains(&id) {
            rows.extend(body(&mk));
        }
        rows
    }

    /// The PR's facts (author, branches, size, reviews, checks), in a box whose title opens
    /// the PR on GitHub.
    pub(super) fn pr_facts(&self) -> Vec<Row> {
        let mk = |kind, text| Row::text(INFO, kind, text);
        let facts = self.info.iter().take_while(|l| !l.heading && !l.md).count();
        if facts == 0 || self.unpublished() {
            return vec![];
        }
        let url = self.pr.as_ref().map(|p| p.url.clone()).filter(|u| !u.is_empty());
        let fact = |r: Row| Row { info: Some(FACTS), ..r };
        let title = if url.is_some() { "status · on GitHub ↗" } else { "status" };
        let links = url.iter().map(|u| (9, title.chars().count(), u.clone())).collect();
        let mut rows = vec![fact(Row { links, ..mk(Kind::Thread, title.into()) })];
        // Not wrapped: cut at the box's edge and scrolled sideways, as code is.
        rows.extend(self.info[..facts].iter().map(|l| fact(Row { nowrap: true, ..self.info_row(l, mk(Kind::Comment, l.text.clone())) })));
        rows.push(Row { edge: true, ..fact(mk(Kind::Comment, String::new())) });
        rows
    }

    /// The PR's conversation (its comments and reviews, and the questions to Claude under
    /// them): all of it (`None`), or (`Some(n)`) those new since you looked (after your own last
    /// comment or review, not seen in dv), the last `n` at least, and any with a question under
    /// it; a line first saying how many older ones there are.
    pub(super) fn conversation(&self, latest: Option<usize>) -> Vec<Row> {
        let mk = |kind, text| Row::text(INFO, kind, text);
        let width = self.info_w();
        // Its items, after its own heading: a comment's or a review's heading then its text, or
        // what else happened (`event`) and the lines under it.
        let Some(start) = self.info.iter().position(|l| l.key == "activity") else { return vec![] };
        let items: Vec<usize> = (start + 1..self.info.len()).filter(|&i| self.info[i].heading).collect();
        // What the latest shows (`LATEST`, or `n`), all of it or not.
        let latest_shown: Vec<usize> = {
            let n = latest.unwrap_or(LATEST);
                let asked = |h: usize| !self.info[h].url.is_empty() && self.threads.iter().any(|t| t.conversation == self.info[h].url && self.in_view(t));
                // New: after your own last word there (you'd read up to it, as GitHub has it),
                // and not seen in dv since.
                let me = format!("@{} ·", self.me);
                let yours = |h: usize| !self.me.is_empty() && self.info[h].text.starts_with(&me);
                let after = items.iter().rposition(|&h| yours(h)).map_or(0, |k| k + 1);
                // (Nowhere to keep what you've seen: nothing's new, as for threads; nor your own.)
                let new = |k: usize, h: usize| {
                    self.seen.path.is_some() && k >= after && !yours(h) && !self.info[h].node.is_empty() && !self.seen.before.contains(&self.info[h].node)
                };
                // From the `n`th last comment or review (events don't count: three of those would
                // leave out the comment before them), the first if there are fewer; of the events
                // since, the last `3n` (many after a comment: the latest of them).
                let said: Vec<usize> = (0..items.len()).filter(|&k| !self.info[items[k]].event).collect();
                let from = said.len().checked_sub(n).map_or(said.first().copied().unwrap_or(0), |k| said[k]);
                let recent = |k: usize| k >= from && (!self.info[items[k]].event || k >= items.len().saturating_sub(3 * n));
                items.iter().enumerate().filter(|&(k, &h)| recent(k) || new(k, h) || asked(h)).map(|(_, &h)| h).collect()
        };
        let shown = if latest.is_some() { &latest_shown } else { &items };
        let mut rows = vec![];
        // (Not GitHub's answer: what went wrong, in its place.)
        if !items.is_empty() || !self.info[start].heading {
            rows.push(Row { info: None, ..mk(Kind::Thread, self.info[start].text.clone()) });
        }
        // What's left out is older (newest last): said first, under its heading.
        let more = items.len() - shown.len();
        if more > 0 {
            let what = if more == 1 { "1 earlier".to_owned() } else { format!("{more} earlier") };
            rows.push(Row { info: Some(MORE_CONVERSATION), ..mk(Kind::Comment, format!("… {what}: Space o shows them")) });
        } else if latest.is_none() && latest_shown.len() < items.len() {
            rows.push(Row { info: Some(MORE_CONVERSATION), ..mk(Kind::Comment, "… all of it: Space o shows only the latest".into()) });
        }
        for h in shown.iter().copied() {
            let body = h + 1;
            // What else happened: its line, and those under it (a push's commits), no box.
            if self.info[h].event {
                let under = self.info[body..].iter().take_while(|l| l.event && !l.heading).count();
                rows.extend((h..body + under).map(|i| Row { info: Some(i), ..self.info_row(&self.info[i], mk(Kind::Comment, self.info[i].text.clone())) }));
                continue;
            }
            // Hidden on GitHub: a line, as a folded thread is (Tab shows it), and what's under it.
            if let Some(note) = self.hidden_note(&self.info[h].hidden, &self.info[h].node) {
                // (The note right after who: a long line cut at the box's edge still says so.)
                let l = &self.info[h];
                let at = l.text.find(" · ").map_or(l.text.chars().count(), |i| l.text[..i].chars().count());
                let n = note.chars().count();
                let rest = l.text.chars().skip(at).collect::<String>();
                let rest = if rest.trim() == "·" { String::new() } else { rest }; // (no time to say)
                let text: String = l.text.chars().take(at).chain(note.chars()).chain(rest.chars()).collect();
                let mut row = self.info_row(l, mk(Kind::Thread, text.clone()));
                for link in &mut row.links {
                    if link.0 >= at {
                        (link.0, link.1) = (link.0 + n, link.1 + n);
                    }
                }
                rows.push(Row { info: self.info.get(body).filter(|n| n.md).map(|_| body), syn: vec![Some(self.ui.stale); text.chars().count()], ..row });
                rows.extend(self.conversation_rows(body));
                continue;
            }
            rows.push(Row { info: self.info.get(body).filter(|n| n.md).map(|_| body), ..self.info_row(&self.info[h], mk(Kind::Thread, self.info[h].text.clone())) });
            let text = self.info.get(body).map_or("", |l| l.text.as_str());
            // A review with no text, just its verdict: GitHub's placeholder, in its box.
            let text = if text.trim().is_empty() { "_(no comment)_" } else { text };
            rows.extend(self.md_rows_at(text, width, mk).into_iter().map(|r| Row { info: Some(body), ..r }));
            rows.push(Row { info: Some(body), edge: true, ..mk(Kind::Comment, String::new()) });
            rows.extend(self.conversation_rows(body));
        }
        rows
    }

    /// The questions for Claude about conversation comment `info[body]` (and its answers),
    /// under it.
    pub(super) fn conversation_rows(&self, body: usize) -> Vec<Row> {
        let Some(url) = body.checked_sub(1).and_then(|h| self.info.get(h)).filter(|h| h.heading && !h.url.is_empty()).map(|h| &h.url) else { return vec![] };
        (0..self.threads.len())
            .filter(|&t| self.threads[t].conversation == *url && self.in_view(&self.threads[t]))
            .flat_map(|t| self.thread_rows(t, INFO))
            .collect()
    }

    /// The conversation comment with this URL: its body's index in `info`.
    pub(super) fn conversation_item(&self, url: &str) -> Option<usize> {
        (0..self.info.len()).find(|&i| self.info[i].heading && self.info[i].url == url).map(|h| h + 1).filter(|&b| self.info.get(b).is_some_and(|l| l.md))
    }

    /// The Claude view's, attached to the description: Claude's summary (or, with no review
    /// yet, your questions about the PR), and what's said under it.
    pub(super) fn claude_attached(&self) -> Vec<Row> {
        let questions = self.threads.iter().any(|t| t.conversation == ON_SUMMARY && self.in_view(t));
        let summary = !self.claude.summary.trim().is_empty();
        let mk = |kind, text| Row::text(INFO, kind, text);
        // What `dv list`'s ranking said to do next, first.
        let mut next = vec![];
        if let Some((says, at)) = &self.ranked_next {
            next.push(mk(Kind::Thread, format!("next, as Claude ranked it ({} ago)", crate::gh::ago_secs(crate::claude::now_secs() - at))));
            next.extend(self.md_rows_at(says, self.info_w(), mk));
            next.push(mk(Kind::Comment, String::new()));
        }
        if !summary && !questions {
            next.extend(self.claude_activity()); // (what it did all the same)
            return next;
        }
        let mut rows = match summary {
            // (No summary: nothing of it to quote or post, so not the summary's rows.)
            false => vec![mk(Kind::Thread, "questions for Claude".into())],
            true => {
                let summary = |r: Row| Row { info: Some(SUMMARY), ..r };
                // (When it was written, and of what if the PR has moved on since: how current it is.)
                let old = self.claude.head.as_str();
                let moved = self.pr.as_ref().is_some_and(|p| !old.is_empty() && !p.head_oid.is_empty() && !p.head_oid.starts_with(old));
                let mut when: Vec<String> = vec![];
                if !self.claude.created.is_empty() {
                    when.push(crate::gh::ago(&self.claude.created));
                }
                if moved {
                    when.push(format!("of {}, before the PR's latest", old.get(..7).unwrap_or(old)));
                }
                let when = if when.is_empty() { String::new() } else { format!(" ({})", when.join(" · ")) };
                // An issue's: its triage, what it is and how urgent; what next, and why close it.
                let t = &self.claude.triage;
                let (head, text) = match self.has_code() {
                    true => ("claude's review".to_owned(), self.claude.summary.clone()),
                    false => {
                        let head = ["claude's triage", &t.kind, &t.priority].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
                        let labels = match t.labels.is_empty() {
                            true => String::new(),
                            false => format!("{} (Space l: marked ◆)", t.labels.join(", ")),
                        };
                        let said = [("labels", &labels), ("next", &t.next), ("close it", &self.claude.close)].into_iter().filter(|s| !s.1.trim().is_empty());
                        (head, said.fold(self.claude.summary.clone(), |text, (k, v)| format!("{text}\n\n**{k}:** {v}")))
                    }
                };
                let mut rows = vec![summary(mk(Kind::Thread, format!("{head}{when}")))];
                rows.extend(self.md_rows_at(&text, self.info_w(), mk).into_iter().map(summary));
                rows.push(Row { edge: true, ..summary(mk(Kind::Comment, String::new())) });
                rows
            }
        };
        rows.extend(self.summary_threads());
        rows.extend(self.claude_activity());
        next.extend(rows);
        next
    }

    /// The Claude tab's activity, as the Comments tab has the PR's: what Claude did, newest
    /// first, a line each (when, what) and its gist: its review (once: "reviewed", how many
    /// comments, the start of its summary), then each reply (an answer, a change it made, a
    /// suggestion), where.
    pub(super) fn claude_activity(&self) -> Vec<Row> {
        let claude = |c: &gh::Comment| c.author == crate::claude::AUTHOR;
        // (Outdated: where it was.)
        let at = |t: &Thread| match (t.path.as_str(), t.line.or(t.original_line), t.conversation.as_str()) {
            ("", _, ON_SUMMARY) => "on its review".to_owned(),
            ("", _, c) if commit_of(c).is_some() => format!("on commit {}", self.commit_short(commit_of(c).unwrap_or(""))),
            ("", ..) => "on the conversation".to_owned(),
            // (The file's name: its path is in the comment it goes to.)
            (p, Some(l), _) => format!("{}:{l}", p.rsplit('/').next().unwrap_or(p)),
            (p, None, _) => p.rsplit('/').next().unwrap_or(p).to_owned(),
        };
        // Its review: the comments it opened with (not its answers to others' comments, nor on
        // no lines), one entry, at the latest's time; its summary's start says what it found.
        // Enter: the first of them in the diff.
        let review = |t: &Thread| t.by_claude() && t.about.is_none() && t.conversation.is_empty();
        let file_of = |t: &Thread| self.files.iter().position(|f| f.path == t.path);
        let mut roots: Vec<(usize, &Thread)> = self.threads.iter().enumerate().filter(|(_, t)| review(t)).collect();
        // (A file not in the diff: last. ponytail: an old-side line is compared as a new one.)
        roots.sort_by_cached_key(|(_, t)| (file_of(t).unwrap_or(usize::MAX), t.line.or(t.original_line)));
        let mut said: Vec<Did> = vec![];
        if !roots.is_empty() || !self.claude.summary.trim().is_empty() {
            let created = roots.iter().filter_map(|r| r.1.comments.first()).map(|c| c.created.as_str()).max().unwrap_or(&self.claude.created);
            let n = match roots.len() {
                0 => String::new(),
                1 => " · 1 comment".into(),
                n => format!(" · {n} comments"),
            };
            let did = if self.has_code() { "reviewed" } else { "triaged" };
            said.push((created, format!("{did}{n}"), self.claude.summary.as_str(), roots.first().map(|r| (r.0, 0))));
        }
        // Then all else it wrote: its replies, its answers to your questions and to others'
        // comments (what it answered, changed, suggested), where.
        for (ti, t) in self.threads.iter().enumerate().filter(|(_, t)| t.claude.is_some()) {
            let from = if review(t) { 1 } else { 0 };
            for (ci, c) in t.comments.iter().enumerate().skip(from).filter(|(_, c)| claude(c)) {
                said.push((c.created.as_str(), at(t), c.body.as_str(), Some((ti, ci))));
            }
        }
        if said.is_empty() {
            return vec![];
        }
        said.sort_by(|a, b| b.0.cmp(a.0)); // ISO times sort as text
        let mk = |kind, text| Row { info: Some(CLAUDE_ACTIVITY), ..Row::text(INFO, kind, text) };
        let mut rows = vec![mk(Kind::Thread, "claude's activity".into())];
        let w = self.info_w();
        // Each: when and where, dim; then what it said (its first paragraph), wrapped, two lines
        // at most (Enter: all of it).
        let dim = Some(self.ui.linenr);
        for (created, at, body, goes) in said.into_iter().take(CLAUDE_LATEST) {
            let when = if created.is_empty() { String::new() } else { format!("{} · ", crate::gh::ago(created)) };
            let head = super::cut(&format!("◆ {when}{at}"), w.saturating_sub(1));
            rows.push(Row { goes, syn: vec![dim; head.chars().count()], ..mk(Kind::Comment, head) });
            let lines: Vec<&str> = body.lines().map(str::trim).skip_while(|l| l.is_empty()).collect();
            let para = lines.iter().take_while(|l| !l.is_empty()).copied().collect::<Vec<_>>().join(" ");
            let tw = w.saturating_sub(3).max(1);
            let wrapped = super::wrap(&para, tw);
            // (More than shows: past two lines, or past its first paragraph.)
            let more = wrapped.len() > 2 || lines.iter().skip_while(|l| !l.is_empty()).any(|l| !l.is_empty());
            let shown = wrapped.len().min(2);
            for (k, l) in wrapped.iter().take(2).enumerate() {
                let l = if more && k + 1 == shown { format!("{l} …") } else { l.clone() };
                // (Cut too: a word longer than a line isn't broken by `wrap`.)
                rows.push(Row { goes, ..mk(Kind::Comment, format!("  {}", super::cut(&l, tw))) });
            }
        }
        rows.push(Row { edge: true, ..mk(Kind::Comment, String::new()) });
        rows
    }

    /// Your replies to Claude's review summary (and its answers), under it.
    pub(super) fn summary_threads(&self) -> Vec<Row> {
        (0..self.threads.len())
            .filter(|&t| self.threads[t].conversation == ON_SUMMARY && self.in_view(&self.threads[t]))
            .flat_map(|t| self.thread_rows(t, INFO))
            .collect()
    }

    /// With no PR (a branch Claude reviewed), Claude's review is all the PR section has.
    pub(super) fn no_pr_summary(&self) -> Vec<Row> {
        match self.pr.is_none() && self.replacement.is_none() {
            true => self.claude_attached(),
            false => vec![],
        }
    }

    /// What you asked of an unpublished draft's drafts `on` (`replace::Request::on`), a box
    /// each, oldest first, saying whether a redraft has taken it in: beside the draft it's about.
    /// (Each row is its draft's, `info`: `a` there asks about that draft too.)
    pub(super) fn request_rows(&self, on: &[&str], draft: usize) -> Vec<Row> {
        let Some(r) = self.replacement.as_ref().filter(|_| self.unpublished()) else { return vec![] };
        let mk = |kind, text| Row { info: Some(draft), ..Row::text(INFO, kind, text) };
        let drafting = r.drafting_now();
        let mut rows = vec![];
        for (i, a) in r.asks.iter().enumerate().filter(|(_, a)| on.contains(&a.on.as_str())) {
            // (A question is answered; anything else, redrafted with.)
            let [done, doing, todo] = match a.want.as_str() {
                "ask" => ["answered", "Claude is answering it", "not answered yet"],
                _ => ["redrafted with it", "Claude is redrafting with it", "not redrafted with it yet"],
            };
            let state = match (a.done(), drafting && i < r.taking) {
                (true, _) => done,
                (false, true) => doing,
                (false, false) => todo,
            };
            let want = match a.want.as_str() {
                "ask" => " [ask]",
                "change" => " [change]",
                _ => "",
            };
            rows.push(mk(Kind::Thread, format!("you · asked of the drafts{want} · {state}")));
            rows.extend(self.md_rows_at(&a.text, self.info_w(), mk));
            match a.answer.trim() {
                "" => {}
                // dv's note (Claude said nothing): not as Claude's answer.
                crate::replace::NO_ANSWER => rows.extend(self.md_rows_at(crate::replace::NO_ANSWER, self.info_w(), mk)),
                ans => {
                    rows.push(mk(Kind::Thread, "◆ claude".into()));
                    rows.extend(self.md_rows_at(ans, self.info_w(), mk));
                }
            }
            rows.push(Row { edge: true, ..mk(Kind::Comment, String::new()) });
        }
        rows
    }

    /// Your pending review's text, in full, as it goes to GitHub with the review (`S`).
    pub(super) fn draft_rows(&self) -> Vec<Row> {
        if self.summary_draft.trim().is_empty() {
            return vec![];
        }
        let mk = |kind, text| Row::text(INFO, kind, text);
        let draft = |r: Row| Row { info: Some(DRAFT), ..r };
        let mut rows = vec![draft(mk(Kind::Thread, "your review (pending)".into()))];
        rows.extend(self.md_rows_at(&self.summary_draft, self.info_w(), mk).into_iter().map(draft));
        rows.push(Row { edge: true, ..draft(mk(Kind::Comment, String::new())) });
        rows
    }

    /// A section with what's `attached` to it: beside it from its header on (the pane), or
    /// after it.
    pub(super) fn attach(&self, mut s: Vec<Row>, attached: &mut Vec<Row>) -> Vec<Row> {
        let at = if self.margin_active { 1 } else { s.len() }.min(s.len());
        s.splice(at..at, std::mem::take(attached).into_iter().map(|r| Row { attached: true, ..r }));
        s
    }

    /// Wrap markdown into comment rows, highlighted.
    pub(super) fn md_rows_at(&self, text: &str, width: usize, mk: impl Fn(Kind, String) -> Row) -> Vec<Row> {
        let key = format!("{width}\0{text}");
        let lines = self
            .md_cache
            .borrow_mut()
            .entry(key)
            .or_insert_with(|| crate::md::render(text, width, &self.ui.md, &self.hl, self.repo_url()))
            .clone();
        lines.into_iter().map(|l| Row { syn: l.styles, links: l.links, nowrap: l.nowrap, ..mk(Kind::Comment, l.text) }).collect()
    }

    /// Color a PR info line by its marks: badges, pass/fail colors, links.
    pub(super) fn info_row(&self, l: &InfoLine, mut row: Row) -> Row {
        let ui = &self.ui;
        row.syn = vec![None; l.text.chars().count()];
        for (a, b, m) in &l.marks {
            let st = match m {
                Mark::Chip(hex) => chip(hex),
                Mark::Pass => ui.plus,
                Mark::Fail => ui.error,
                Mark::Pending => ui.warning,
                Mark::Dim => ui.stale,
                Mark::Mine => ui.key,
                Mark::Link(url) => {
                    row.links.push((*a, *b, url.clone()));
                    ui.md.link
                }
            };
            for s in row.syn.iter_mut().take(*b).skip(*a) {
                *s = Some(st);
            }
        }
        row
    }

    /// A comment hidden on GitHub (`why`), not shown here (Tab): what its heading says after
    /// who and when, in place of its text.
    pub(super) fn hidden_note(&self, why: &str, node: &str) -> Option<String> {
        self.unseen(why, node).then(|| format!(" · {}", hidden_words(why)))
    }

    /// Whether a comment (`hidden` on GitHub, why; its node id) is hidden here still: not to be
    /// quoted (`c`, `C`, `r`, `a`) before you've seen it.
    pub(super) fn unseen(&self, hidden: &str, node: &str) -> bool {
        !hidden.is_empty() && !self.revealed.contains(node)
    }

    /// A review comment's text, as far as you've seen it: none while it's hidden on GitHub
    /// (and not shown here). What quotes, previews or edits it reads it through this.
    pub(super) fn seen_body<'a>(&self, c: &'a gh::Comment) -> Option<&'a str> {
        (!self.unseen(&c.hidden, &c.node_id)).then_some(c.body.as_str())
    }

    /// The same for the PR's text: `info` line `i`, unless it's a conversation comment's,
    /// hidden on GitHub and not shown here.
    pub(super) fn seen_info(&self, i: usize) -> Option<&InfoLine> {
        let head = i.checked_sub(1).and_then(|h| self.info.get(h)).filter(|h| h.heading);
        self.info.get(i).filter(|_| !head.is_some_and(|h| self.unseen(&h.hidden, &h.node)))
    }

    /// A review comment's text for a list, or (hidden on GitHub, unseen) that it's hidden.
    pub(super) fn gist<'a>(&self, c: &'a gh::Comment) -> std::borrow::Cow<'a, str> {
        self.seen_body(c).map_or_else(|| format!("_({})_", hidden_words(&c.hidden)).into(), Into::into)
    }

    /// Whether row `r` is the folded line of a conversation comment hidden on GitHub.
    pub(super) fn hidden_line(&self, r: usize) -> bool {
        self.hidden_row(&self.rows[r])
    }

    pub(super) fn hidden_row(&self, row: &Row) -> bool {
        row.file == INFO && row.thread.is_none() && row.line.kind == Kind::Thread && row.info.is_some_and(|i| i < self.info.len() && self.seen_info(i).is_none())
    }

    /// The comment hidden on GitHub under the cursor (its node id), for Tab to show or hide:
    /// not on an open thread's first line once it's shown (Tab there folds the thread).
    pub(super) fn hidden_here(&self) -> Option<String> {
        let r = self.head.row;
        match self.rows[r].thread {
            Some(t) if !self.thread_open(t) => None,
            Some(t) => {
                let c = self.comment_under_cursor(t).filter(|c| !c.hidden.is_empty())?;
                let first = r == 0 || self.rows[r - 1].thread != Some(t);
                // On the thread's first line, Tab goes round: shown, hidden again, the thread folded.
                (!(first && self.hid_again.contains(&c.node_id))).then(|| c.node_id.clone())
            }
            None => self.conversation_node().filter(|n| self.info.iter().any(|l| l.node == *n && !l.hidden.is_empty())),
        }
    }

    /// Tab on a comment hidden on GitHub: its text shown, or hidden again. Whether it was one.
    pub(super) fn toggle_hidden(&mut self) -> bool {
        let r = self.head.row;
        let first = self.rows[r].thread.filter(|&t| r == 0 || self.rows[r - 1].thread != Some(t)).and_then(|t| self.comment_under_cursor(t)).map(|c| c.node_id.clone());
        let Some(node) = self.hidden_here() else {
            // Hidden again there: this Tab folds the thread, and unfolded, it starts over.
            if let Some(n) = first {
                self.hid_again.remove(&n);
            }
            return false;
        };
        if first.as_ref() == Some(&node) && self.revealed.contains(&node) {
            self.hid_again.insert(node.clone());
        }
        flip(&mut self.revealed, node.clone());
        // The cursor on its heading, whether its text went or came (else where it was).
        let at = self.rows[self.head.row].thread.map(|t| (self.root_id(t), self.comment_index_under_cursor(t)));
        let (keep, col) = (self.cursor_key(), self.head.col);
        self.rebuild(keep);
        let row = match at {
            Some((id, k)) => {
                let t = (0..self.threads.len()).find(|&t| self.root_id(t) == id);
                self.rows.iter().enumerate().filter(|(_, r)| r.thread.is_some() && r.thread == t && r.line.kind == Kind::Thread).nth(k).map(|(i, _)| i)
            }
            None => self.rows.iter().position(|r| {
                let head = r.info.and_then(|b| self.info.get(b.checked_sub(1)?));
                r.file == INFO && r.line.kind == Kind::Thread && r.thread.is_none() && head.is_some_and(|l| l.heading && l.node == node)
            }),
        };
        if let Some(row) = row {
            self.goto(row, col);
        }
        true
    }

    /// `info[i]` is the description's heading.
    pub(super) fn is_description(&self, i: usize) -> bool {
        // A replacement's: "description (Claude's draft)".
        self.info.get(i).is_some_and(|l| l.heading && (l.text == "description" || self.replacement.is_some() && l.text.starts_with("description")))
    }

    /// `info[i]` is a replacement's closing note's heading.
    pub(super) fn is_closing_note(&self, i: usize) -> bool {
        self.replacement.is_some() && self.info.get(i).is_some_and(|l| l.heading && l.text.starts_with("closing note"))
    }

    /// `Space o` (or Enter on the line saying there's more): the PR's whole conversation beside
    /// the description, or (again) only its latest. In the whole diff (where it is), the cursor
    /// on the conversation's heading.
    pub(super) fn toggle_conversation(&mut self) {
        if self.subject().is_none() {
            return self.msg = "no PR here".into();
        }
        let start = self.info.iter().position(|l| l.key == "activity");
        // (Not GitHub's answer, what went wrong: its line says.)
        if let Some(s) = start.filter(|&s| !self.info[s].heading) {
            return self.msg = self.info[s].text.clone();
        }
        if !start.is_some_and(|s| self.info[s + 1..].iter().any(|l| l.heading)) {
            return self.msg = "no activity on this PR yet".into();
        }
        // From another tab: to the whole diff, with all of it.
        self.conversation_all = self.view != View::All || !self.conversation_all;
        if self.view != View::All {
            (self.view, self.msg) = (View::All, "the whole diff, with all of the PR's activity".into());
        }
        self.sections_folded.remove(&DESCRIPTION);
        self.info_collapsed = false;
        self.rebuild_at(|r| r.attached && r.line.text.starts_with("activity ("));
    }
}
