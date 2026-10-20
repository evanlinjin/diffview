//! Git from dv (`Space g`): see only what isn't committed yet (to check what Claude just
//! changed), commit it, and push it to the PR, so the review loop doesn't leave dv.
use super::*;

/// What was done (`said`), and what went wrong reloading after it (`res`), if anything.
pub(super) fn with_reload(said: String, res: Result<()>) -> String {
    match (said.is_empty(), res) {
        (_, Ok(())) => said,
        (true, Err(e)) => format!("{e:#}"),
        (false, Err(e)) => format!("{said} (but the diff didn't reload: {e:#})"),
    }
}

impl App {
    /// `Space g` → update the checkout: on a PR, up to its head as `gh pr checkout` does (fast-forward only: a branch
    /// with commits that aren't on the PR is left as it is, and said so); else `git pull
    /// --ff-only`. Local changes in the way are git's to refuse, so nothing is overwritten.
    pub(super) fn pull(&self) -> Result<String> {
        match &self.pr {
            Some(p) => {
                let detached = diff::git(&["branch", "--show-current"])?.trim().is_empty();
                let note = crate::checkout_or_keep(p.number, false, detached)?;
                Ok(note.unwrap_or_else(|| format!("up to date with PR #{}", p.number)))
            }
            None => {
                diff::git(&["pull", "--ff-only"])?;
                Ok("pulled".into())
            }
        }
    }

    /// Diffing against HEAD: only what isn't committed yet.
    pub(super) fn uncommitted(&self) -> bool {
        self.base == "HEAD"
    }

    /// What takes the commits made here to GitHub, for a message after making them: `S` for a
    /// draft (publishing it), else `Space g` (`force`: a force-push, after a rewrite).
    pub(super) fn next_step(&self, force: bool) -> &'static str {
        match (self.unpublished(), force) {
            (true, _) => "S publishes it",
            (false, true) => "Space g force-pushes (with a lease)",
            (false, false) => "Space g pushes it",
        }
    }

    /// Amending what isn't committed into the branch's commits (`Space g`, a replacement's `S`):
    /// only changes to tracked files can be (new files aren't in `git diff HEAD`).
    pub(super) fn amend_items(&self) -> Vec<Item> {
        if !diff::git(&["-C", &self.root.to_string_lossy(), "diff", "--name-only", "HEAD"]).is_ok_and(|o| !o.trim().is_empty()) {
            return vec![];
        }
        vec![
            Item::new("Amend it into the commits it belongs in… (you see where each change goes first)", Target::AmendMenu),
            Item::new("Amend it all into the commits Claude finds it belongs in (a backup branch first)", Target::AmendAll),
        ]
    }

    /// `Space g`: what there is to do with the working tree and the branch.
    pub(super) fn git_menu(&mut self) {
        // dv's rebase that ended some other way (in a terminal, say): its note goes.
        if !super::rebase::rebasing(&self.root) && self.claude.job.is_none() {
            super::rebase::forget_rebase(&self.root);
        }
        // Stopped part way through a rebase: finish it or abort it, nothing else (HEAD is
        // detached, conflicts may be in the tree).
        if super::rebase::rebasing(&self.root) {
            // One you started yourself is yours to finish (in a terminal); dv's, Claude's.
            let items = match super::rebase::dv_rebase(&self.root) {
                Some(_) => vec![
                    Item::new("Ask Claude to finish the rebase (it resolves the conflicts)", Target::RebaseClaude),
                    Item::new("Abort the rebase (back to before it)", Target::RebaseAbort),
                ],
                None => vec![
                    Item::new("A rebase you started is stopped: finish it with git (git rebase --continue)", Target::Nothing),
                    Item::new("Abort it (back to before it)", Target::RebaseAbort),
                ],
            };
            return self.picker = Some(Picker::new(" git: a rebase is stopped ", items));
        }
        let changed = diff::git(&["status", "--porcelain"]).unwrap_or_default().lines().count();
        let mut items = vec![match self.uncommitted() {
            true => Item::new("Show the whole diff again", Target::GitView),
            false => Item::new(format!("Show only what isn't committed ({changed} file(s))"), Target::GitView),
        }];
        if changed > 0 {
            items.push(Item::new(format!("Commit everything that isn't ({changed} file(s))…"), Target::Do(Action::Commit)));
            items.extend(self.amend_items());
            // (This file's, when it has some: `git status` names it from the repo's root.)
            let path = self.file_path().map(str::to_owned).filter(|p| diff::git(&["status", "--porcelain", "--", p]).is_ok_and(|s| !s.trim().is_empty()));
            if let Some(p) = path.filter(|_| changed > 1) {
                items.push(Item::new(format!("Drop what isn't committed in {p}… (stashed)"), Target::GitDrop(Some(p), false)));
            }
            items.push(Item::new(format!("Drop everything that isn't committed ({changed} file(s))… (stashed)"), Target::GitDrop(None, false)));
        }
        // A draft no one has seen: published (S), not pushed.
        let push = if self.unpublished() { Err(anyhow!("it's a draft: S publishes it")) } else { self.push_target() };
        match push {
            Err(_) if self.unpublished() => items.push(Item::new("Publish it as a draft PR: S (nothing to push to before)", Target::Nothing)),
            Ok((remote, branch, name)) => match self.unpushed(&remote, &branch) {
                Push::Nothing => {}
                Push::Ahead(n) => items.push(Item::new(format!("Push {n} commit(s) to {name}"), Target::Run(Job::Push(false)))),
                Push::Unknown => items.push(Item::new(format!("Push to {name}"), Target::Run(Job::Push(false)))),
                Push::Diverged => {
                    // Behind only (nothing here it doesn't have): no force-push, just its version.
                    let theirs = self.their_head(&remote, &branch);
                    let ours = diff::git(&["rev-list", "--count", &format!("{theirs}..HEAD")]).ok().and_then(|n| n.trim().parse::<usize>().ok()).unwrap_or(0);
                    if ours > 0 {
                        items.push(Item::new(
                            format!("Force-push to {name} (with a lease: it has commits this doesn't, as after a rebase or amend here)"),
                            Target::Run(Job::Push(true)),
                        ));
                    }
                    let label = match ours {
                        0 => format!("Update to {name} as it is on GitHub (it has new commits)"),
                        n => format!("Take {name} as it is on GitHub instead ({n} commit(s) only here are kept on a backup branch)…"),
                    };
                    items.push(Item::new(label, Target::GitTakeTheirs(ours == 0)));
                }
            },
            Err(e) => items.push(Item::new(format!("Can't push: {e:#}"), Target::Nothing)),
        }
        // Onto the base branch: Claude resolves conflicts.
        if let Some(onto) = self.rebase_onto() {
            let behind = diff::git(&["rev-list", "--count", &format!("HEAD..{onto}")]).ok().and_then(|n| n.trim().parse::<usize>().ok()).unwrap_or(0);
            let behind = if behind > 0 { format!("{behind} new commit(s) there as last fetched; ") } else { String::new() };
            items.push(Item::new(format!("Rebase onto {onto} ({behind}fetched first; Claude resolves any conflicts)"), Target::Run(Job::Rebase)));
        }
        // Diverged, that's "Update to …" above (fast-forward can't).
        if !self.unpublished() && !items.iter().any(|i| matches!(i.target, Target::GitTakeTheirs(_))) {
            let label = match &self.pr {
                Some(p) => format!("Update the checkout to #{}'s head (fast-forward only; your commits stay)", p.number),
                None => "Update the checkout (git pull --ff-only)".into(),
            };
            items.push(Item::new(label, Target::Run(Job::Pull)));
        }
        // Failing for no fault of the PR's (a flaky test, a network blip): once more.
        if self.failing > 0 && self.pr.is_some() {
            let s = if self.failing == 1 { "" } else { "s" };
            items.push(Item::new(format!("Rerun the failed check{s} ({}: their failed jobs, on GitHub)", self.failing), Target::Run(Job::Rerun)));
        }
        if self.awaiting > 0 {
            let s = if self.awaiting == 1 { "" } else { "s" };
            items.insert(0, Item::new(format!("Approve its {} workflow run{s} (waiting for a maintainer: a first-time contributor's, say)", self.awaiting), Target::GitApproveRuns));
        }
        if let Some(p) = self.pr.as_ref().filter(|_| self.pr_state.is_some_and(|s| s.0 == "DRAFT")) {
            items.push(Item::new(format!("Mark #{} ready for review (it's a draft)", p.number), Target::GitReady));
        }
        if let Some(p) = &self.pr {
            let label = format!("Merge #{} with github-merge.py (ends this review; its worktree goes first, if nothing in it is unpushed)", p.number);
            items.push(Item::new(label, Target::GitMerge));
        }
        self.picker = Some(Picker::new(" git ", items));
    }

    /// `Space g` → approve the workflow runs waiting on the PR's head, so they run.
    pub(super) fn approve_runs(&mut self) {
        let Some(head) = self.pr.as_ref().map(|p| p.head_oid.clone()) else { return };
        self.msg = match crate::gh::approve_runs(&head) {
            Ok(0) => "no workflow runs waiting for approval (any more)".into(),
            Ok(k) => {
                self.awaiting = 0;
                self.github_changed();
                format!("approved {k} workflow run{}: they run now (CI shows them as they do)", if k == 1 { "" } else { "s" })
            }
            Err(e) => format!("not approved: {e:#}"),
        };
    }

    /// `Space g` → mark the PR ready for review: on GitHub, and its state here.
    pub(super) fn mark_ready(&mut self) {
        let Some(n) = self.pr.as_ref().map(|p| p.number) else { return };
        self.msg = match crate::gh::mark_ready(n) {
            Ok(()) => {
                self.pr_state = Some(("OPEN", "238636"));
                self.github_changed();
                format!("#{n} is ready for review")
            }
            Err(e) => format!("#{n} still a draft: {e:#}"),
        };
    }

    /// `Space g` → the branch as it is on GitHub, after a confirmation (`sure`) when there are
    /// commits only here: they're kept on a branch first (`dv-backup/…`), then HEAD moves to
    /// GitHub's (`reset --hard`). Not with uncommitted changes: they'd be lost.
    pub(super) fn take_theirs(&mut self, sure: bool) {
        self.msg = match self.try_take_theirs(sure) {
            Ok(m) => m,
            Err(e) => format!("not taken: {e:#}"),
        };
    }

    fn try_take_theirs(&mut self, sure: bool) -> Result<String> {
        let (remote, branch, name) = self.push_target()?;
        let theirs = self.their_head(&remote, &branch);
        anyhow::ensure!(diff::git(&["status", "--porcelain"])?.trim().is_empty(), "there are uncommitted changes (commit them first, Space g, or they'd be lost)");
        // Not here yet: fetched (a PR's head is on its base repo too).
        if diff::git(&["cat-file", "-e", &format!("{theirs}^{{commit}}")]).is_err() {
            match &self.pr {
                Some(p) => _ = diff::git(&["fetch", "-q", &crate::diff::remote(), &format!("refs/pull/{}/head", p.number)])?,
                None => _ = diff::git(&["fetch", "-q", &remote, &branch])?,
            }
        }
        let ours: usize = diff::git(&["rev-list", "--count", &format!("{theirs}..HEAD")])?.trim().parse().unwrap_or(0);
        if ours > 0 && !sure {
            let items = vec![
                Item::new("No, keep what's here", Target::Nothing),
                Item::new(format!("Yes, take {name} from GitHub ({ours} commit(s) only here kept on a backup branch)"), Target::GitTakeTheirs(true)),
            ];
            self.picker = Some(Picker::new(" take GitHub's version? ", items));
            return Ok(String::new());
        }
        let who = self.pr.as_ref().map_or(branch.replace('/', "-"), |p| format!("pr-{}", p.number));
        let kept = take_branch(std::path::Path::new("."), &theirs, &who)?.map_or(String::new(), |b| format!("; what was only here is on {b}"));
        let said = self.and_reload(format!("now at {name} as it is on GitHub{kept}"));
        self.refresh_sync();
        Ok(said)
    }

    /// Drop what isn't committed (in `path`; else all of it, new files too), after a yes
    /// (`sure`): stashed, so `git stash pop` brings it back.
    pub(super) fn drop_uncommitted(&mut self, path: Option<String>, sure: bool) -> Result<String> {
        anyhow::ensure!(!self.claude_busy(), "Claude is at work here: drop it once it's done");
        let what = path.as_deref().map_or("everything that isn't committed".to_owned(), |p| format!("what isn't committed in {p}"));
        if !sure {
            let items = vec![
                Item::new("No, keep it", Target::Nothing),
                Item::new(format!("Yes, drop {what} (stashed: git stash pop brings it back)"), Target::GitDrop(path, true)),
            ];
            self.picker = Some(Picker::new(" drop uncommitted changes? ", items));
            return Ok(String::new());
        }
        let mut args = vec!["stash", "push", "--include-untracked", "-q", "-m", "dv: dropped"];
        if let Some(p) = &path {
            args.extend(["--", p.as_str()]);
        }
        diff::git(&args)?;
        Ok(self.and_reload(format!("dropped {what} (git stash pop brings it back)")))
    }

    /// Between the whole diff and only what isn't committed.
    pub(super) fn toggle_uncommitted(&mut self) {
        let to = if self.uncommitted() { self.full_base.clone() } else { "HEAD".into() };
        self.base = to;
        (self.to, self.commit_shown, self.commit_anchor) = (None, None, None);
        self.extra.clear(); // per-hunk context: the hunks are different ones now
        self.msg = match (self.reload(false), self.uncommitted()) {
            (Err(e), _) => format!("{e:#}"),
            (Ok(()), true) if self.files.is_empty() => "nothing that isn't committed (Space g shows the whole diff)".into(),
            (Ok(()), true) => "only what isn't committed (Space g shows the whole diff)".into(),
            (Ok(()), false) => "the whole diff".into(),
        };
    }

    /// The commit message to start from: what Claude said it changed since the last commit
    /// (its "Changed: …" replies), and, as hints that aren't committed, what will be.
    pub(super) fn commit_draft(&self) -> String {
        let since = diff::git(&["log", "-1", "--format=%ct"]).ok().and_then(|t| t.trim().parse().ok()).unwrap_or(0);
        draft(&self.threads, since, &diff::git(&["status", "--porcelain"]).unwrap_or_default())
    }

    /// Where Claude writes the commit message it's asked for: beside the review file.
    pub(super) fn commit_msg_path(&self) -> Option<std::path::PathBuf> {
        self.claude.path.as_ref().map(|p| p.with_extension("commit.txt"))
    }

    /// "Commit everything": Claude writes the message, in the background (from the changes,
    /// in the repo's style); once it's done your editor opens with it (`COMMIT_LABEL`).
    pub(super) fn commit_ask(&mut self) -> Result<String> {
        let p = self.commit_msg_path().context("no review file")?;
        // One it wrote that your editor couldn't open with then: that one.
        if let Some(msg) = std::fs::read_to_string(&p).ok().filter(|m| !m.trim().is_empty()) {
            self.action = Some(Action::CommitWith(msg));
            return Ok(String::new());
        }
        let status = diff::git(&["status", "--porcelain"]).unwrap_or_default();
        let prompt = format!(
            "Write the commit message for committing everything not committed in this checkout (as `git add -A` would \
             take it): these files (`git status --porcelain`):\n{status}\nRead the changes (`git diff HEAD`; a new \
             file's content with your Read tool) and how this repo writes commit messages (`git log -15 --format=%B`), \
             and write a message in that style: a subject line saying what it does, and a body saying why when it isn't \
             obvious (the review file's \"Changed: …\" replies, in {}, say why for what you changed). Write only the \
             message, to {} (your Write tool). Don't commit, and change nothing else.",
            self.claude.path.as_ref().map_or(String::new(), |r| r.display().to_string()),
            p.display()
        );
        // On a fork of the PR's session when it has one: what it knows, without this side task
        // in the review's conversation (nor its failure costing it).
        let forked = self.claude.path.as_ref().and_then(|r| crate::claude::load_session(r, &self.root)).is_some_and(|(id, ok)| ok && crate::claude::session_exists(&id));
        self.claude.job = Some(self.spawn_claude(&prompt, COMMIT_LABEL, forked, &[])?);
        Ok("Claude is writing the commit message (your editor opens with it)…".into())
    }

    /// Commit everything that isn't (new files too: dv shows them), with a message written in
    /// the editor; leaving it empty cancels.
    pub(super) fn commit(&mut self, draft: &str) -> Result<String> {
        // (However it got here: an editor pane closing while Claude's at work again, say.)
        anyhow::ensure!(!self.claude_busy(), "Claude is still at work here: commit once it's done (its changes would be half made)");
        if let Some(p) = self.commit_msg_path() {
            _ = std::fs::remove_file(p); // (Claude's message: given to the editor now)
        }
        let Some(msg) = compose_with("commit message (everything uncommitted, new files too)", false, draft, true)? else {
            return Ok("commit cancelled".into());
        };
        let file = std::env::temp_dir().join(format!("dv-commit-{}.txt", std::process::id()));
        std::fs::write(&file, &msg)?;
        let res = diff::git(&["add", "-A"]).and_then(|_| diff::git(&["commit", "-q", "-F", &file.to_string_lossy()]));
        _ = std::fs::remove_file(&file);
        keep_on_error(res.map(drop), &msg)?;
        let head = diff::git(&["log", "-1", "--format=%h %s"]).unwrap_or_default();
        Ok(format!("committed {} ({})", head.trim(), self.next_step(false)))
    }

    /// How the checkout stands against the PR (or where it's pushed), for the pinned title:
    /// commits not pushed, a history that's diverged (an amend or rebase here), commits the PR
    /// has that this doesn't, changes not committed. Empty when it's all in step.
    pub(super) fn refresh_sync(&mut self) {
        let mut parts = vec![];
        // A draft not published yet: its commits are all unpushed, as they should be.
        if self.unpublished() {
            return self.sync.clear();
        }
        if let Ok((remote, branch, _)) = self.push_target() {
            let theirs = self.their_head(&remote, &branch);
            match self.unpushed(&remote, &branch) {
                Push::Ahead(n) => parts.push(format!("↑{n} unpushed")),
                // HEAD in the PR's history: it's ahead of this, not diverged from it.
                Push::Diverged if diff::git(&["merge-base", "--is-ancestor", "HEAD", &theirs]).is_ok() => parts.push("↓ behind the PR".into()),
                Push::Diverged => parts.push("⇅ diverged: force-push".into()),
                Push::Nothing | Push::Unknown => {}
            }
        }
        if !diff::git(&["status", "--porcelain", "--untracked-files=no"]).unwrap_or_default().trim().is_empty() {
            parts.push("✎ uncommitted".into());
        }
        self.sync = parts.join(" · ");
    }

    /// Commits on HEAD that the PR (or the upstream branch) doesn't have yet, if that's known.
    fn unpushed(&self, remote: &str, branch: &str) -> Push {
        push_state(std::path::Path::new("."), &self.their_head(remote, branch))
    }

    /// The branch's head where it's pushed to: the PR's, else the remote branch's.
    fn their_head(&self, remote: &str, branch: &str) -> String {
        match &self.pr {
            Some(p) if !p.head_oid.is_empty() => p.head_oid.clone(),
            _ => format!("refs/remotes/{remote}/{branch}"),
        }
    }

    /// Where `Space g` pushes: (remote or URL, branch, how to say it). The branch's own (its
    /// push remote, as `gh pr checkout` set it up); in a detached `dv N -w` worktree, the PR's
    /// branch in its repo (a fork's too: maintainers can push there when the author allows it),
    /// over SSH or HTTPS as `origin` is.
    fn push_target(&self) -> Result<(String, String, String)> {
        let here = diff::git(&["branch", "--show-current"])?.trim().to_owned();
        if !here.is_empty() {
            let (remote, branch) = diff::push_target(Path::new("."), &here).with_context(|| format!("{here} has no remote to push to"))?;
            let name = format!("{branch} on {remote}");
            return Ok((remote, branch, name));
        }
        let p = self.pr.as_ref().filter(|p| !p.head_repo.is_empty() && !p.head_branch.is_empty()).context("not on a branch, and no PR to push to")?;
        let origin = diff::git(&["remote", "get-url", &crate::diff::remote()]).unwrap_or_default();
        let ssh = origin.starts_with("git@") || origin.starts_with("ssh://");
        let url = if ssh { format!("git@github.com:{}.git", p.head_repo) } else { format!("https://github.com/{}.git", p.head_repo) };
        let name = format!("the PR's branch, {}:{}", p.head_repo.split('/').next().unwrap_or(""), p.head_branch);
        Ok((url, p.head_branch.clone(), name))
    }

    /// Push HEAD to where `push_target` says; `force`: with a lease, so only over the PR's head as
    /// dv knows it (it looks every few seconds): a push there it hasn't seen yet stops it.
    pub(super) fn push(&mut self, force: bool) -> Result<String> {
        let (remote, branch, name) = self.push_target()?;
        // Nothing of its own over its base: pushed, GitHub would close the PR (no commits).
        if let Some(p) = &self.pr {
            let base = diff::remote_branch(&p.base);
            let own = diff::git(&["rev-list", "--count", &format!("{base}..HEAD")]).unwrap_or_default();
            anyhow::ensure!(own.trim() != "0", "not pushed: HEAD has no commits over {base}, and GitHub would close #{} for it (is this checkout on the PR?)", p.number);
        }
        let lease = format!("--force-with-lease=refs/heads/{branch}:{}", self.their_head(&remote, &branch));
        let mut args = vec!["push", "-q"];
        if force {
            args.push(&lease);
        }
        let to = format!("HEAD:refs/heads/{branch}");
        args.extend([remote.as_str(), to.as_str()]);
        diff::git(&args)?;
        // The PR's head is what was just pushed (its next fetch says so too).
        if let (Some(p), Ok(h)) = (&mut self.pr, diff::git(&["rev-parse", "HEAD"])) {
            p.head_oid = h.trim().to_owned();
        }
        let next = match self.threads.iter().any(Thread::by_claude) {
            true => " · Space a → Update Claude's review looks at just what's new",
            false => "",
        };
        Ok(format!("{} to {name}{next}", if force { "force-pushed" } else { "pushed" }))
    }
}

/// A commit message from Claude's "Changed: …" replies after `since` (the last commit's time),
/// with `status` (`git status --porcelain`) as hint lines.
pub(super) fn draft(threads: &[Thread], since: i64, status: &str) -> String {
    let changes: Vec<String> = threads
        .iter()
        .filter(|t| t.claude.is_some())
        .flat_map(|t| &t.comments)
        .filter(|c| c.author == crate::claude::AUTHOR && gh::epoch_secs(&c.created).is_some_and(|t| t > since))
        .filter_map(|c| c.body.strip_prefix("Changed:"))
        .map(|b| format!("- {}", b.lines().next().unwrap_or("").trim()))
        .collect();
    let files: String = status.lines().map(|l| format!("{HINT} will commit: {l} -->\n")).collect();
    match changes.is_empty() {
        true => format!("\n\n{files}"),
        false => format!("Address review comments\n\n{}\n\n{files}", changes.join("\n")),
    }
}

/// How HEAD in the checkout at `dir` stands against `theirs` (the head where it's pushed).
/// HEAD in `dir` to `theirs`, its commits that `theirs` hasn't kept on a branch first,
/// `dv-backup/<who>-<its short sha>`: that branch's name, if there was anything to keep.
fn take_branch(dir: &std::path::Path, theirs: &str, who: &str) -> Result<Option<String>> {
    let d = dir.to_string_lossy().into_owned();
    let git = |args: &[&str]| diff::git(&[&["-C", d.as_str()], args].concat());
    let ours: usize = git(&["rev-list", "--count", &format!("{theirs}..HEAD")])?.trim().parse().unwrap_or(0);
    let backup = match ours {
        0 => None,
        _ => {
            let b = format!("dv-backup/{who}-{}", git(&["rev-parse", "--short", "HEAD"])?.trim());
            git(&["branch", "-f", &b, "HEAD"])?;
            Some(b)
        }
    };
    git(&["reset", "-q", "--hard", theirs])?;
    Ok(backup)
}

fn push_state(dir: &std::path::Path, theirs: &str) -> Push {
    let d = dir.to_string_lossy().into_owned();
    let git = |args: &[&str]| diff::git(&[&["-C", d.as_str()], args].concat());
    if git(&["cat-file", "-e", &format!("{theirs}^{{commit}}")]).is_err() {
        return Push::Unknown; // not here: can't tell
    }
    if git(&["merge-base", "--is-ancestor", theirs, "HEAD"]).is_err() {
        return Push::Diverged;
    }
    match git(&["rev-list", "--count", &format!("{theirs}..HEAD")]).ok().and_then(|n| n.trim().parse().ok()) {
        Some(0) => Push::Nothing,
        Some(n) => Push::Ahead(n),
        None => Push::Unknown,
    }
}

/// How HEAD stands against where it's pushed: nothing to push, commits ahead, or diverged (a
/// rebase or amend here: only a force-push does it), or unknown (their head isn't here).
#[derive(Debug, PartialEq)]
enum Push {
    Nothing,
    Ahead(usize),
    Diverged,
    Unknown,
}

#[test]
fn push_or_force() {
    let dir = diff::scratch_repo("push");
    let git = |args: &[&str]| diff::git_in(&dir, args).unwrap();
    let commit = |m: &str| git(&["commit", "-q", "--allow-empty", "-m", m]);
    commit("pr head");
    let pr_head = git(&["rev-parse", "HEAD"]).trim().to_owned();
    assert_eq!(push_state(&dir, &pr_head), Push::Nothing);
    commit("mine");
    assert_eq!(push_state(&dir, &pr_head), Push::Ahead(1));
    // Amended (or rebased): the PR's head isn't in it any more; only a force-push does it.
    git(&["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--amend", "--allow-empty", "-m", "mine, again"]);
    git(&["reset", "-q", "--soft", "HEAD~1"]);
    git(&["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--amend", "--allow-empty", "-m", "pr head, reworded"]);
    assert_eq!(push_state(&dir, &pr_head), Push::Diverged);
    assert_eq!(push_state(&dir, "0000000000000000000000000000000000000000"), Push::Unknown);
    // Taking GitHub's instead: at its head, what was only here kept on a backup branch.
    let mine = git(&["rev-parse", "--short", "HEAD"]).trim().to_owned();
    let backup = take_branch(&dir, &pr_head, "pr-7").unwrap();
    assert_eq!(backup.as_deref(), Some(format!("dv-backup/pr-7-{mine}").as_str()));
    assert_eq!(git(&["rev-parse", "HEAD"]).trim(), pr_head);
    assert_eq!(git(&["rev-parse", "--short", &backup.unwrap()]).trim(), mine);
    // Behind only: nothing to keep.
    assert_eq!(take_branch(&dir, &pr_head, "pr-7").unwrap(), None);
    let _ = std::fs::remove_dir_all(&dir);
}

impl App {
    /// What the working tree looks like, cheaply: the latest change time of the diff's files,
    /// and `git status` of tracked files (so a file Claude starts changing counts too).
    pub(super) fn tree_stamp(&self) -> (Option<std::time::SystemTime>, u64) {
        use std::hash::{Hash, Hasher};
        let newest = self.files.iter().filter_map(|f| std::fs::metadata(self.root.join(&f.path)).and_then(|m| m.modified()).ok()).max();
        let mut h = std::collections::hash_map::DefaultHasher::new();
        diff::git(&["status", "--porcelain"]).unwrap_or_default().hash(&mut h);
        (newest, h.finish())
    }

    /// About once a second: reload the diff when the working tree changed (Claude's edits, a
    /// save in hx, anything else), keeping the cursor where it is; and notice dv's hx pane
    /// closing.
    // ponytail: polls git status each second; a file watcher if that's ever slow on a big repo.
    pub(super) fn watch_tree(&mut self) {
        if self.tree_checked.is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(1)) {
            return;
        }
        self.tree_checked = Some(std::time::Instant::now());
        if self.hx.as_ref().is_some_and(|h| !crate::pane::alive(&h.pane)) {
            self.hx = None;
        }
        let now = self.tree_stamp();
        if self.tree_seen.replace(now).is_none_or(|before| before == now) {
            return;
        }
        self.reload_tree();
    }

    /// The diff reloaded after the working tree changed under dv (and what follows from it);
    /// what went wrong, said after what's said (`and_reload`).
    pub(super) fn reload_tree(&mut self) {
        let said = std::mem::take(&mut self.msg);
        self.msg = self.and_reload(said);
    }

    /// `said` (what was done), the working tree reloaded after it: with what went wrong
    /// reloading, if anything, not in its place.
    pub(super) fn and_reload(&mut self, said: String) -> String {
        let res = self.reload_all(false);
        with_reload(said, res)
    }

    /// The diff reloaded (`threads`: GitHub's too), and what follows from it: commits rewritten
    /// followed, a retarget that waited on a rebase, the tree as seen now.
    pub(super) fn reload_all(&mut self, threads: bool) -> Result<()> {
        self.forget_tree_colors();
        let res = self.reload(threads);
        self.rekey_commits(); // (commits rewritten here or elsewhere)
        self.follow_github_base(false); // (a retarget that waited on a rebase now through)
        self.tree_seen = Some(self.tree_stamp());
        res
    }

    pub(super) fn reload(&mut self, threads: bool) -> Result<()> {
        let keep = self.cursor_key();
        self.follow_rebase();
        self.load_commits();
        self.files = diff::load(&self.base, self.to.as_deref(), DEFAULT_CONTEXT)?;
        // After the diff: Claude's comments follow their code, which may have just been edited.
        match threads {
            true => self.load_threads(),
            false => self.load_claude(),
        }
        self.shape();
        self.rebuild(keep);
        self.refresh_sync();
        Ok(())
    }

    /// HEAD rebased onto a newer base (its branch has moved on, and the PR with it): the diff
    /// starts there, else it'd show the base's own commits between as the PR's. Only forward:
    /// a base ahead of the old one, on HEAD's history.
    pub(super) fn follow_rebase(&mut self) {
        let Some(branch) = &self.follow_base else { return };
        // Not mid-rebase: it may yet be aborted, and the base only ever moves forward.
        if rebase::rebasing(&self.root) {
            return;
        }
        let git = |args: &[&str]| diff::git_in(&self.root, args);
        let Ok(now) = git(&["merge-base", branch, "HEAD"]).map(|s| s.trim().to_owned()) else { return };
        if now == self.full_base || git(&["merge-base", "--is-ancestor", &self.full_base, &now]).is_err() {
            return;
        }
        if !self.uncommitted() {
            self.base = now.clone(); // the whole diff (a commit shown, or a range, goes)
        }
        self.full_base = now;
        (self.to, self.commit_shown, self.commit_anchor) = (None, None, None);
    }
}

/// The background run writing a commit message: when it's done, the commit goes on.
pub(super) const COMMIT_LABEL: &str = "writing the commit message";
