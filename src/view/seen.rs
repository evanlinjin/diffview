//! What you've seen of the PR's comments, kept across runs: what's new since, unfolded.
use super::*;

/// How long the cursor stays on a thread before its comments count as seen.
pub(super) const SEEN_AFTER: std::time::Duration = std::time::Duration::from_secs(3);

/// A comment's key in what you've seen: its GitHub id, or its place in Claude's review file.
pub(super) fn comment_key(t: &Thread, ci: usize) -> String {
    match &t.claude {
        Some(id) => format!("claude:{id}:{ci}"),
        None => t.comments[ci].node_id.clone(),
    }
}

impl App {
    /// Whether file `path` has comments you haven't seen.
    pub(super) fn new_on(&self, path: &str) -> bool {
        self.threads.iter().any(|t| t.path == path && self.is_new(t))
    }

    /// The files with comments you haven't seen.
    pub(super) fn new_paths(&self) -> HashSet<String> {
        self.threads.iter().filter(|t| self.is_new(t)).map(|t| t.path.clone()).collect()
    }

    /// New comments came (GitHub's or Claude's): the viewed files they're on unfold, so
    /// they're seen; not those that had some already (`before`: folded since, by you).
    pub(super) fn unfold_new(&mut self, before: &HashSet<String>) {
        let open: Vec<String> = self.collapsed.iter().filter(|p| self.viewed.get(*p).is_some_and(|s| s == "VIEWED") && self.new_on(p) && !before.contains(*p)).cloned().collect();
        for p in open {
            self.collapsed.remove(&p);
        }
    }

    /// What you've seen of this PR's (or branch's) threads, in dv's directory: `seen/` beside
    /// `reviews/`, by the same name.
    pub(super) fn load_seen(&mut self) {
        self.seen.path = self.claude.path.as_ref().and_then(|p| Some(p.parent()?.parent()?.join("seen").join(p.file_name()?)));
        let text = self.seen.path.as_ref().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
        self.seen.keys = serde_json::from_str(&text).unwrap_or_default();
        self.seen.before = self.seen.keys.clone();
    }

    /// Thread `t` has comments you haven't seen, not counting your own. (Never, with nowhere
    /// to keep what you've seen.)
    pub(super) fn is_new(&self, t: &Thread) -> bool {
        self.seen.path.is_some()
            && t.comments.iter().enumerate().any(|(ci, c)| !self.wrote(c) && !self.seen.keys.contains(&comment_key(t, ci)))
    }

    pub(super) fn wrote(&self, c: &gh::Comment) -> bool {
        c.pending || c.author == "you" || (!self.me.is_empty() && c.author == self.me)
    }

    /// A thread's comments are seen once the cursor leaves it (as they were when it got
    /// there), or has been on it for `SEEN_AFTER`.
    pub(super) fn watch_seen(&mut self) {
        if self.select {
            return; // the rebuild would drop the selection: later
        }
        let here = self.rows.get(self.head.row).and_then(|r| r.thread).map(|ti| {
            let t = &self.threads[ti];
            (0..t.comments.len()).map(|ci| comment_key(t, ci)).collect::<Vec<_>>()
        });
        // A conversation comment (its heading or text): its node id.
        let on_thread = here.is_some();
        let here = here.or_else(|| self.conversation_node().map(|n| vec![n])).unwrap_or_default();
        let (seen, conversation) = match here == self.seen.current {
            true if self.seen.since.elapsed() >= SEEN_AFTER => (self.seen.current.clone(), self.seen.conversation),
            true => return,
            false => {
                self.seen.since = std::time::Instant::now();
                let was = std::mem::replace(&mut self.seen.conversation, !on_thread);
                (std::mem::replace(&mut self.seen.current, here), was)
            }
        };
        let before = self.seen.keys.len();
        self.seen.keys.extend(seen.into_iter().filter(|k| !k.is_empty()));
        if self.seen.keys.len() != before
            && let Some(p) = &self.seen.path
        {
            let mut keys: Vec<&String> = self.seen.keys.iter().collect();
            keys.sort();
            // ponytail: rewrites the whole set; fine for a PR's worth of comments.
            _ = std::fs::create_dir_all(p.parent().unwrap_or(p)).and_then(|_| std::fs::write(p, serde_json::to_string(&keys).unwrap_or_default()));
            if !conversation {
                let keep = self.cursor_key();
                self.rebuild(keep); // without its [new] tag
            }
        }
    }
}
