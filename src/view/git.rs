//! Git from dv (`Space g`): see only what isn't committed yet (to check what Claude just
//! changed), commit it, and push it to the PR, so the review loop doesn't leave dv.
use super::*;

impl App {
    /// Diffing against HEAD: only what isn't committed yet.
    pub(super) fn uncommitted(&self) -> bool {
        self.base == "HEAD"
    }

    /// `Space g`: what there is to do with the working tree and the branch.
    pub(super) fn git_menu(&mut self) {
        let changed = diff::git(&["status", "--porcelain"]).unwrap_or_default().lines().count();
        let mut items = vec![match self.uncommitted() {
            true => Item::new("Show the whole diff again", Target::GitView),
            false => Item::new(format!("Show only what isn't committed ({changed} file(s))"), Target::GitView),
        }];
        if changed > 0 {
            items.push(Item::new(format!("Commit everything that isn't ({changed} file(s))…"), Target::GitCommit));
        }
        match push_target() {
            Ok((remote, branch)) => match self.unpushed(&remote, &branch) {
                Some(0) => {}
                Some(n) => items.push(Item::new(format!("Push {n} commit(s) to {branch} on {remote}"), Target::GitPush)),
                None => items.push(Item::new(format!("Push to {branch} on {remote}"), Target::GitPush)),
            },
            Err(e) => items.push(Item::new(format!("Can't push: {e:#}"), Target::Nothing)),
        }
        if let Some(p) = &self.pr {
            let label = format!("Merge #{} with github-merge.py (ends this review; its worktree goes first, if nothing in it is unpushed)", p.number);
            items.push(Item::new(label, Target::GitMerge));
        }
        self.picker = Some(Picker { title: " git ", items, query: String::new(), sel: 0 });
    }

    /// Between the whole diff and only what isn't committed.
    pub(super) fn toggle_uncommitted(&mut self) {
        let to = if self.uncommitted() { self.full_base.clone() } else { "HEAD".into() };
        self.base = to;
        self.extra.clear(); // per-hunk context: the hunks are different ones now
        self.hl_cache.retain(|k, _| !k.1);
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

    /// Commit everything that isn't (new files too: dv shows them), with a message written in
    /// the editor; leaving it empty cancels.
    pub(super) fn commit(&mut self, draft: &str) -> Result<String> {
        let Some(msg) = compose_with("commit message (everything uncommitted, new files too)", false, draft, true)? else {
            return Ok("commit cancelled".into());
        };
        let file = std::env::temp_dir().join(format!("dv-commit-{}.txt", std::process::id()));
        std::fs::write(&file, &msg)?;
        let res = diff::git(&["add", "-A"]).and_then(|_| diff::git(&["commit", "-q", "-F", &file.to_string_lossy()]));
        _ = std::fs::remove_file(&file);
        keep_on_error(res.map(drop), &msg)?;
        let head = diff::git(&["log", "-1", "--format=%h %s"]).unwrap_or_default();
        Ok(format!("committed {} (Space g pushes it)", head.trim()))
    }

    /// Commits on HEAD that the PR (or the upstream branch) doesn't have yet, if that's known.
    fn unpushed(&self, remote: &str, branch: &str) -> Option<usize> {
        let theirs = match &self.pr {
            Some(p) if !p.head_oid.is_empty() => p.head_oid.clone(),
            _ => format!("refs/remotes/{remote}/{branch}"),
        };
        diff::git(&["rev-list", "--count", &format!("{theirs}..HEAD")]).ok()?.trim().parse().ok()
    }

    /// Push HEAD to the branch it came from (a PR's, as `gh pr checkout` set it up).
    pub(super) fn push(&mut self) -> Result<String> {
        let (remote, branch) = push_target()?;
        diff::git(&["push", "-q", &remote, &format!("HEAD:refs/heads/{branch}")])?;
        let next = match self.threads.iter().any(agent::claude_wrote) {
            true => " · Space a → Update Claude's review looks at just what's new",
            false => "",
        };
        Ok(format!("pushed to {branch} on {remote}{next}"))
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

/// Where HEAD pushes to: the current branch's push remote (or its remote), and the branch
/// it tracks there (a PR's own name, even when the local one is `<owner>-<branch>`).
fn push_target() -> Result<(String, String)> {
    let here = diff::git(&["branch", "--show-current"])?.trim().to_owned();
    if here.is_empty() {
        bail!("not on a branch (a `dv N -w` worktree is detached; `dv N -wb` checks out the PR's branch)");
    }
    let config = |k: &str| diff::git(&["config", "--get", k]).ok().map(|v| v.trim().to_owned()).filter(|v| !v.is_empty());
    let remote = config(&format!("branch.{here}.pushRemote"))
        .or_else(|| config("remote.pushDefault"))
        .or_else(|| config(&format!("branch.{here}.remote")))
        .with_context(|| format!("{here} has no remote to push to"))?;
    let branch = config(&format!("branch.{here}.merge")).map_or(here, |m| m.trim_start_matches("refs/heads/").to_owned());
    Ok((remote, branch))
}
