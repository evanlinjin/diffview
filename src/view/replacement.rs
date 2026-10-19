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
            _ if r.drafting_now() => vec![item("Claude is drafting it: wait for it to finish".into(), Target::Nothing)],
            None if r.title.is_empty() => vec![item("Claude hasn't drafted it yet (Space a → Redraft)".into(), Target::Nothing)],
            // Only commits are published: what isn't committed would be left out.
            None if dirty => vec![
                item("Commit the uncommitted changes first (they'd be left out)…".into(), Target::Do(Action::Commit)),
                item("Amend them into its commits they belong in… (you see where each change goes first)".into(), Target::AmendMenu),
                item("Amend them all into the commits Claude finds they belong in (a backup branch first)".into(), Target::AmendAll),
            ],
            None => {
                let mut v = vec![item(format!("Publish as a draft PR (rebased on {}, pushed {to})…", diff::remote_branch(&r.base)), Target::Do(Action::Publish(false)))];
                if !open.is_empty() {
                    v.push(item(format!("Publish, then close {} with the closing note…", open.join(", ")), Target::Do(Action::Publish(true))));
                }
                v
            }
            Some(n) => vec![item(format!("Published as PR #{n}"), Target::Nothing)],
        };
        match (r.published, open.is_empty()) {
            (_, true) => items.push(item("  the PRs it replaces are closed".into(), Target::Nothing)),
            (Some(_), false) => items.push(item(format!("Close {} with the closing note (now)…", open.join(", ")), Target::Do(Action::CloseOriginals))),
            (None, false) => {}
        }
        self.picker = Some(Picker::new(" replacement ", items));
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

    /// The replacement checked out here (no PR), as PR info: what it replaces, and Claude's
    /// drafts of its description and closing note.
    pub(super) fn load_replacement(&mut self) {
        if let Some(r) = crate::replace::current().and_then(|id| crate::replace::load(id).ok()) {
            self.show_replacement(r);
        }
    }

    pub(super) fn show_replacement(&mut self, r: crate::replace::Record) {
        let line = |heading, md, text: &str| InfoLine { heading, md, text: text.into(), ..Default::default() };
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
            let url = r.replaces.first().and_then(|o| Some(format!("{}/{n}", o.url.rsplit_once('/')?.0))).unwrap_or_default();
            let mut l = line(false, false, &format!("published as PR #{n}"));
            l.marks.push((l.text.chars().count() - format!("#{n}").len(), l.text.chars().count(), Mark::Link(url)));
            status.push(l);
        }
        if !r.closed.is_empty() {
            let closed: Vec<String> = r.closed.iter().map(|n| format!("#{n}")).collect();
            status.push(line(false, false, &format!("closed {}, with the note", closed.join(", "))));
        }
        if r.drafting_now() {
            status.push(line(false, false, "Claude is redrafting it…"));
        }
        // A new PR: what you asked for, in place of what it replaces (and no closing note).
        let new = r.replaces.is_empty();
        if new {
            replaces = line(false, false, &format!("asked: {}", r.task.lines().next().unwrap_or("")));
        }
        self.info = vec![replaces];
        self.info.extend(status);
        self.info.extend([line(true, false, "description (Claude's draft)"), line(false, true, &r.body)]);
        if !new {
            self.info.extend([line(true, false, "closing note, for the PRs it replaces (Claude's draft)"), line(false, true, &r.closing_note)]);
        }
        self.pr_state = Some(if r.published.is_some() { ("PUBLISHED", "238636") } else { ("UNPUBLISHED", "6e7681") });
        self.replacement = Some(r);
    }
}
