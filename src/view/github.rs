//! The PR from GitHub: fetched (in the background, kept to show at once next time), watched
//! for changes, applied to the view, and its base followed when it changes there.
use super::*;

/// Shown while what's on screen is from the cache.
pub(super) const STALE: &str = "showing the PR as dv last saw it; refreshing from GitHub…";

/// How often dv looks at whether the PR changed on GitHub (a 304 costs no rate limit).
pub(super) const GITHUB_POLL: std::time::Duration = std::time::Duration::from_secs(5);

/// The PR's review threads and its info, as `fetch_github` got them.
pub(super) type Fetched = (Result<gh::Review>, Result<Info>);

/// Both at once (each is a ~1s GitHub round-trip), kept in `cache` when both came.
pub(super) fn fetch_github(n: u64, cache: Option<&Path>) -> Fetched {
    let (threads, mut info) = std::thread::scope(|s| {
        let info = s.spawn(|| gh::info_json(n));
        (gh::threads_json(n), info.join().unwrap_or_else(|_| Err(anyhow!("panicked"))))
    });
    // What GitHub couldn't give this time: the last it gave, kept.
    if let (Some(p), Ok(i)) = (cache, &mut info)
        && let Some(old) = std::fs::read_to_string(p).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok())
    {
        gh::keep_known(i, &old["info"]);
    }
    if let (Some(p), Ok(t), Ok(i)) = (cache, &threads, &info) {
        // Not keeping it only costs the next start its head start.
        _ = p.parent().map(std::fs::create_dir_all);
        _ = crate::claude::write(p, &json!({ "threads": t, "info": i }));
    }
    (threads.map(|t| gh::parse_threads(&t)), info.map(|i| parse_info(&i)))
}

/// Where the PR's last answers from GitHub are kept, to show at once next time.
pub(super) fn cache_path(n: u64) -> Option<PathBuf> {
    Some(crate::claude::dv_dir().ok()?.join("cache").join(format!("pr-{n}.json")))
}

/// For `dv list`: PR `n`'s comments and info from GitHub, kept for its first opening if
/// nothing is (else they show at once already, fetched afresh as they do).
pub(crate) fn prefetch_github(n: u64) {
    if let Some(p) = cache_path(n).filter(|p| !p.exists()) {
        _ = fetch_github(n, Some(&p));
    }
}

/// What `fetch_github` kept, if anything.
pub(super) fn cached_github(p: &Path) -> Option<Fetched> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()?;
    Some((Ok(gh::parse_threads(&v["threads"])), Ok(parse_info(&v["info"]))))
}

/// `msg` with `note` said after it, in place of `last` (the note before) and of the "as dv
/// last saw it" notice.
pub(super) fn base_note_in(msg: &str, last: &str, note: &str) -> String {
    let keep = msg.split(" · ").filter(|p| !p.is_empty() && *p != last && *p != STALE);
    keep.chain([note]).collect::<Vec<_>>().join(" · ")
}

impl App {
    /// Fetch review threads and PR info (in parallel: each is a ~1s GitHub round-trip).
    pub(super) fn load_threads(&mut self) {
        self.load_github();
        self.load_claude();
    }

    pub(super) fn load_github(&mut self) {
        let Some(n) = self.pr.as_ref().map(|p| p.number) else { return };
        self.github_gen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.github_stale = false;
        self.apply_github(fetch_github(n, cache_path(n).as_deref()), true);
    }

    /// At start: the PR as dv last saw it, if it kept that, while `start_github_watch` fetches
    /// it afresh. Until that's in, `ensure_fresh` holds back anything that changes GitHub.
    pub(super) fn show_cached(&mut self, n: u64) -> bool {
        let Some(f) = cache_path(n).as_deref().and_then(cached_github) else { return false };
        let head = self.pr.as_ref().map(|p| p.head_oid.clone()); // just looked up; the cache's may be old
        self.github_gen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.apply_github(f, false);
        if let (Some(pr), Some(h)) = (&mut self.pr, head) {
            pr.head_oid = h;
        }
        self.github_stale = true;
        self.msg = STALE.into();
        true
    }

    /// Before changing anything on GitHub: the PR as it is now, not as cached.
    pub(super) fn ensure_fresh(&mut self) {
        if self.github_stale {
            let keep = self.cursor_key();
            self.load_threads(); // the background fetch, when it comes, is older: dropped
            self.rebuild(keep);
        }
    }

    /// Every few seconds, a look at the PR's ETag (free while nothing has changed); when it changes,
    /// its threads and info are fetched in the background, for `watch_github` to show.
    /// `fetch_now`: once at the start too (what's shown came from the cache).
    pub(super) fn start_github_watch(&mut self, n: u64, fetch_now: bool) {
        let (tx, rx) = std::sync::mpsc::channel();
        let loads = self.github_gen.clone();
        let (poke, poked) = std::sync::mpsc::channel();
        self.github_poke = Some(poke);
        let cache = cache_path(n);
        std::thread::spawn(move || {
            let mut etag = gh::pr_etag(n, "").ok().flatten().unwrap_or_default();
            let at = loads.load(std::sync::atomic::Ordering::SeqCst);
            if fetch_now && tx.send((at, fetch_github(n, cache.as_deref()))).is_err() {
                return;
            }
            loop {
                // Every so often, or at once when poked (CI rerun: its checks change, not the
                // PR's ETag: fetched whatever it says; the ETag, conditionally asked first, so a
                // change since doesn't fetch again).
                match poked.recv_timeout(GITHUB_POLL) {
                    Ok(()) => {
                        if let Ok(Some(e)) = gh::pr_etag(n, &etag) {
                            etag = e;
                        }
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        let Ok(Some(e)) = gh::pr_etag(n, &etag) else { continue };
                        etag = e;
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return, // dv is done
                }
                let at = loads.load(std::sync::atomic::Ordering::SeqCst);
                if tx.send((at, fetch_github(n, cache.as_deref()))).is_err() {
                    return; // dv is done
                }
            }
        });
        self.github = Some(rx);
    }

    /// dv has just changed the PR on GitHub (pushed, posted its description, …): what a fetch
    /// under way brings back is from before, and would undo what dv shows now. The next one,
    /// after it, is current.
    pub(super) fn github_changed(&mut self) {
        self.github_gen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// Shows what the background look at GitHub fetched, once nothing is in the middle of
    /// something (a picker's or selection's rows would move under it).
    pub(super) fn watch_github(&mut self) {
        let busy = self.action.is_some() || self.job.is_some() || self.picker.is_some() || self.prompt.is_some();
        if busy || self.pending.is_some() || self.select || !self.labels.is_empty() {
            return;
        }
        let Some((at, f)) = self.github.as_ref().and_then(|rx| rx.try_iter().last()) else { return };
        if at != self.github_gen.load(std::sync::atomic::Ordering::SeqCst) {
            return; // loaded since: that's newer
        }
        let count = |a: &App| a.threads.iter().filter(|t| t.claude.is_none()).map(|t| t.comments.len()).sum::<usize>();
        let before = count(self);
        let had_new = self.new_paths();
        let keep = self.cursor_key();
        self.apply_github(f, true);
        self.shape();
        let new = count(self).saturating_sub(before);
        self.unfold_new(&had_new);
        self.rebuild(keep);
        if std::mem::take(&mut self.github_stale) && self.msg == STALE {
            self.msg.clear();
        }
        if new > 0 && self.msg.is_empty() {
            self.msg = format!("{new} new comment(s) on GitHub");
        }
    }

    /// (`fresh`: just fetched, not the cache's: only then does a retarget count.)
    pub(super) fn apply_github(&mut self, (threads, info): Fetched, fresh: bool) {
        match threads {
            Ok(r) => {
                // A draft GitHub couldn't take (see `Post::Summary`) stays until it can.
                if r.pending.is_none() || !r.pending_body.is_empty() {
                    self.summary_draft = r.pending_body;
                }
                // GitHub's threads, new; Claude's (from its review file) as they were, after them.
                let claude: Vec<Thread> = std::mem::take(&mut self.threads).into_iter().filter(|t| t.claude.is_some()).collect();
                (self.threads, self.review) = (r.threads.into_iter().chain(claude).collect(), r.pending);
                (self.theirs, self.me) = (!r.mine, r.me); // (before: your own comments aren't new)
                self.apply_viewed(r.viewed); // after the threads: not folded with new ones
                if let Some(pr) = &mut self.pr {
                    pr.node_id = r.pr_id;
                }
            }
            Err(e) => self.msg = format!("loading comments: {e:#}"),
        }
        match info {
            Ok(Info { lines, title, head, url, state, unsigned, awaiting, failing, base, conflicts }) => {
                (self.awaiting, self.failing, self.conflicts) = (awaiting, failing, conflicts);
                (self.info, self.pr_state) = (lines, Some(state));
                // GitHub's word on its commits' signatures, for the contents' commits (none this
                // time: what was known stands).
                if let Some(unsigned) = unsigned {
                    for c in &mut self.commits {
                        c.signed = !unsigned.contains(&c.sha);
                    }
                    self.unsigned = unsigned;
                }
                if let Some(pr) = &mut self.pr {
                    (pr.title, pr.head_oid, pr.url) = (title, head, url);
                    if fresh && !base.is_empty() {
                        self.github_base = Some(base);
                    }
                }
                self.refresh_sync(); // the PR's head may have moved
            }
            Err(e) => self.msg = format!("loading PR info: {e:#}"),
        }
        self.follow_github_base(true);
    }

    /// The PR's base on GitHub isn't the one here (retargeted): the diff starts from the new
    /// one, once fetched (a job: a frame says so first). Mid-rebase, a diff that follows the base
    /// waits till it's through (`say`: say so, as a fetch shows it); looked at again on each
    /// reload (the rebase's end is one). Cheap while the bases agree.
    pub(super) fn follow_github_base(&mut self, say: bool) {
        let Some(base) = self.github_base.clone().filter(|b| self.pr.as_ref().is_some_and(|p| p.base != *b)) else { return };
        if self.job.is_some() {
            return;
        }
        let follows = self.pr.as_ref().is_some_and(|p| self.follow_base.as_deref() == Some(crate::diff::remote_branch(&p.base).as_str()));
        if follows && super::rebase::rebasing(&self.root) {
            if say {
                self.base_note(&format!("the PR's base is {base} now: the diff follows once the rebase is through"));
            }
            return;
        }
        self.base_note(&format!("the PR's base is {base} now: fetching it…"));
        self.job = Some(Job::Retarget(base));
    }

    /// Say `note` about the PR's base after what's said, in place of an earlier one about it
    /// (and of the "as dv last saw it" notice: this is fresh): not piled up, not over a
    /// rebase's ending, say.
    pub(super) fn base_note(&mut self, note: &str) {
        let last = std::mem::replace(&mut self.base_said, note.to_owned());
        self.msg = base_note_in(&self.msg, &last, note);
    }

    /// The PR's base changed on GitHub to `base`: a rebase goes onto it, and the diff starts
    /// from it (fetched), when dv chose the diff's base (no `--base` given).
    pub(super) fn retarget(&mut self, base: String) {
        let Some(old) = self.pr.as_ref().map(|p| p.base.clone()) else { return };
        let follows = self.follow_base.as_deref() == Some(crate::diff::remote_branch(&old).as_str());
        let onto = crate::diff::remote_branch(&base);
        if follows {
            let fetched = diff::git_in(&self.root, &["fetch", "-q", &crate::diff::remote(), &format!("+refs/heads/{base}:refs/remotes/{onto}")]);
            let Ok(mb) = fetched.and_then(|_| diff::git_in(&self.root, &["merge-base", &onto, "HEAD"])) else {
                self.github_base = None; // (asked again when GitHub next says)
                let said = format!("the PR's base is {base} now, but {onto} couldn't be fetched: the diff still starts from {old} (tried again on the next refresh)");
                return self.base_note(&said);
            };
            let mb = mb.trim().to_owned();
            if !self.uncommitted() {
                self.base = mb.clone();
            }
            (self.full_base, self.follow_base) = (mb, Some(onto));
            (self.to, self.commit_shown, self.commit_anchor) = (None, None, None);
        }
        if let Some(pr) = &mut self.pr {
            pr.base = base.clone();
        }
        self.reload_tree();
        let said = match follows {
            true => format!("the PR's base is {base} now: the diff starts from there"),
            false => format!("the PR's base is {base} now (the diff keeps the base you gave)"),
        };
        // After what was said before (a rebase's ending, say), not over it.
        self.base_note(&said);
    }
}

#[test]
pub(super) fn notes_on_the_base() {
    let a = base_note_in(&format!("rebased · {STALE}"), "", "the base is x now: fetching it…");
    assert_eq!(a, "rebased · the base is x now: fetching it…");
    assert_eq!(base_note_in(&a, "the base is x now: fetching it…", "the base is x now"), "rebased · the base is x now");
}
