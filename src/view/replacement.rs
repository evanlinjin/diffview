//! A replacement (or a new PR) Claude drafts: its menu, its record watched, shown as the PR.
use super::*;

impl App {
    /// `S` on a replacement: publish it as a draft PR (ready for review once CI passes: `Space g`
    /// checks), then close what it replaces.
    pub(super) fn replacement_menu(&mut self) {
        let Some(r) = &self.replacement else { return };
        let item = |label: String, target| Item::new(label, target);
        let open: Vec<String> = r.replaces.iter().filter(|o| !r.closed.contains(&o.number)).map(|o| format!("#{}", o.number)).collect();
        let dirty = !diff::git(&["-C", &self.root.to_string_lossy(), "status", "--porcelain"]).unwrap_or_default().trim().is_empty();
        let to = crate::replace::remote().map_or("your remote: dv asks which".to_owned(), |r| format!("to {r}"));
        let mut items = match r.published {
            _ if self.at_it() => vec![item("Claude is at work on it: wait for it to finish".into(), Target::Note(""))],
            None if !r.drafted() => vec![item(format!("Claude: {} (Space a → Redraft)", r.undrafted()), Target::Note(""))],
            // Only commits are published: what isn't committed would be left out.
            None if dirty => {
                let mut v = vec![item("Commit the uncommitted changes first (they'd be left out)…".into(), Target::Do(Action::Commit))];
                v.extend(self.amend_items());
                v
            }
            None => {
                let mut v = vec![item(format!("Publish as a draft PR (rebased on {}, pushed {to})…", diff::remote_branch(&r.base)), Target::Do(Action::Publish(false)))];
                if !open.is_empty() {
                    v.push(item(format!("Publish, then close {} with the closing note…", open.join(", ")), Target::Do(Action::Publish(true))));
                }
                v
            }
            Some(n) => vec![item(format!("Published as PR #{n}"), Target::Note(""))],
        };
        match (r.published, open.is_empty()) {
            (_, true) if !r.replaces.is_empty() => items.push(item("The PRs it replaces are closed".into(), Target::Note(""))),
            (Some(_), false) => items.push(item(format!("Close {} with the closing note (now)…", open.join(", ")), Target::Do(Action::CloseOriginals))),
            (None, false) | (_, true) => {} // (a new PR replaces nothing)
        }
        self.picker = Some(Picker::new(" replacement ", items));
    }

    /// Published as PR `n`: dv is on that PR now, as if opened on it (its comments, CI, review),
    /// Claude's review of the draft going with it.
    pub(super) fn become_pr(&mut self, n: u64) {
        // (GitHub not answering: still the draft here; opened again, it's the PR.)
        let Ok(pr) = crate::gh::pr(Some(n)) else { return };
        crate::replace::adopt_review(n);
        self.claude.path = crate::claude::path(Some(n)).ok();
        self.pr = Some(pr);
        self.load_seen();
        self.load_github();
        self.start_github_watch(false);
    }

    /// Which remote keeps replacement branches: asked once, then kept in `dv.replaceRemote`.
    pub(super) fn ask_replace_remote(&mut self, close: bool) -> String {
        let choices = crate::replace::remote_choices();
        if choices.is_empty() {
            return "replacement branches go to a remote of your own, not origin: add one (git remote add <name> <your fork's URL>), then S again".into();
        }
        let items = choices.into_iter().map(|(n, url)| Item::new(format!("{n}  {url}"), Target::ReplaceRemote(n, close))).collect();
        self.picker = Some(Picker::new(" which remote keeps replacement branches? ", items));
        "dv remembers it (git config dv.replaceRemote)".into()
    }

    /// A PR's replacement, looked for again when the records change (every few seconds at
    /// most): Claude drafting it shows on the status bar, and Space a goes to it. Its own (of
    /// this PR first) over one of several it's in.
    pub(super) fn watch_replaced_by(&mut self) {
        let Some(n) = self.pr.as_ref().map(|p| p.number) else { return };
        if self.replaced_at.is_some_and(|(t, _)| t.elapsed() < std::time::Duration::from_secs(3)) {
            return;
        }
        let changed = crate::replace::changed();
        let seen = self.replaced_at.is_some_and(|(_, c)| c == changed);
        self.replaced_at = Some((std::time::Instant::now(), changed));
        if !seen {
            self.replaced_by = crate::replace::of(n);
        }
    }

    /// The replacement here, shown again when its record changes (Claude redrafting it).
    pub(super) fn watch_replacement(&mut self) {
        let Some(id) = self.replacement.as_ref().map(|r| r.id) else { return };
        let mtime = crate::replace::path(id).ok().and_then(|p| std::fs::metadata(p).ok()?.modified().ok());
        if mtime == self.replacement_mtime {
            return;
        }
        let first = self.replacement_mtime.is_none();
        self.replacement_mtime = mtime;
        if let (false, Ok(r)) = (first, crate::replace::load(id)) {
            let keep = self.cursor_key();
            self.show_replacement(r);
            self.rebuild(keep);
        }
    }

    /// The replacement checked out here (`replace::here`): unpublished, as PR info (what it
    /// replaces, Claude's drafts of its description and closing note); published, its record.
    pub(super) fn load_replacement(&mut self) {
        if let Some(r) = crate::replace::here(self.pr.as_ref().map(|p| p.number)) {
            self.show_replacement(r);
        }
    }

    /// (Published and on its PR, the PR's info is GitHub's: only the record is kept.)
    pub(super) fn show_replacement(&mut self, r: crate::replace::Record) {
        if self.pr.is_some() {
            return self.replacement = Some(r);
        }
        let line = |heading, md, text: &str| InfoLine { heading, md, text: text.into(), ..Default::default() };
        // (Keyed as GitHub's lines are, so the description's found the same way.)
        let keyed = |k: &str, l: InfoLine| InfoLine { key: k.into(), ..l };
        let mut replaces = line(false, false, "replaces ");
        for (i, o) in r.replaces.iter().enumerate() {
            let sep = if i == 0 { "" } else { ", " };
            replaces.text.push_str(sep);
            let at = replaces.text.chars().count();
            let num = format!("#{}", o.number);
            replaces.text.push_str(&format!("{num} {} (@{})", o.title, o.author));
            replaces.marks.push((at, at + num.chars().count(), Mark::Link(o.url.clone())));
        }
        let mut status = vec![];
        if let Some(n) = r.published {
            // (In the repo of what it replaces; a new PR's: this one's.)
            let repo = r.replaces.first().and_then(|o| Some(o.url.rsplit_once("/pull/")?.0.to_owned())).or_else(crate::diff::github_url);
            let url = repo.map(|u| format!("{u}/pull/{n}")).unwrap_or_default();
            let mut l = line(false, false, &format!("published as PR #{n}"));
            l.marks.push((l.text.chars().count() - format!("#{n}").len(), l.text.chars().count(), Mark::Link(url)));
            status.push(l);
        }
        if !r.closed.is_empty() {
            let closed: Vec<String> = r.closed.iter().map(|n| format!("#{n}")).collect();
            status.push(line(false, false, &format!("closed {}, with the note", closed.join(", "))));
        }
        // A new PR: what you asked for, in place of what it replaces (and no closing note).
        let new = r.replaces.is_empty();
        if new {
            replaces = line(false, false, &format!("asked: {}", r.task.lines().next().unwrap_or("")));
        }
        self.info = vec![replaces];
        self.info.extend(status);
        self.info.extend([keyed("description", line(true, false, "description (Claude's draft)")), keyed("description/text", line(false, true, &r.body))]);
        if !new {
            self.info.extend([keyed("closing note", line(true, false, "closing note, for the PRs it replaces (Claude's draft)")), keyed("closing note/text", line(false, true, &r.closing_note))]);
        }
        self.pr_state = Some(if r.published.is_some() { ("PUBLISHED", "238636") } else { ("UNPUBLISHED", "6e7681") });
        self.replacement = Some(r);
    }
}
