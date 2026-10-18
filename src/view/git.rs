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
        match self.push_target() {
            Ok((remote, branch, name)) => match self.unpushed(&remote, &branch) {
                Push::Nothing => {}
                Push::Ahead(n) => items.push(Item::new(format!("Push {n} commit(s) to {name}"), Target::GitPush)),
                Push::Unknown => items.push(Item::new(format!("Push to {name}"), Target::GitPush)),
                Push::Diverged => items.push(Item::new(
                    format!("Force-push to {name} (with a lease: it has commits this doesn't, as after a rebase or amend here)"),
                    Target::GitForcePush,
                )),
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
            let (remote, branch) = branch_target(&here)?;
            let name = format!("{branch} on {remote}");
            return Ok((remote, branch, name));
        }
        let p = self.pr.as_ref().filter(|p| !p.head_repo.is_empty() && !p.head_branch.is_empty()).context("not on a branch, and no PR to push to")?;
        let origin = diff::git(&["remote", "get-url", "origin"]).unwrap_or_default();
        let ssh = origin.starts_with("git@") || origin.starts_with("ssh://");
        let url = if ssh { format!("git@github.com:{}.git", p.head_repo) } else { format!("https://github.com/{}.git", p.head_repo) };
        let name = format!("the PR's branch, {}:{}", p.head_repo.split('/').next().unwrap_or(""), p.head_branch);
        Ok((url, p.head_branch.clone(), name))
    }

    /// Push HEAD to where `push_target` says; `force`: with a lease, so only over the PR's head as
    /// dv knows it (it looks every few seconds): a push there it hasn't seen yet stops it.
    pub(super) fn push(&mut self, force: bool) -> Result<String> {
        let (remote, branch, name) = self.push_target()?;
        let lease = format!("--force-with-lease=refs/heads/{branch}:{}", self.their_head(&remote, &branch));
        let mut args = vec!["push", "-q"];
        if force {
            args.push(&lease);
        }
        let to = format!("HEAD:refs/heads/{branch}");
        args.extend([remote.as_str(), to.as_str()]);
        diff::git(&args)?;
        let next = match self.threads.iter().any(agent::claude_wrote) {
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

/// Where HEAD pushes to: the current branch's push remote (or its remote), and the branch
/// it tracks there (a PR's own name, even when the local one is `<owner>-<branch>`).
fn branch_target(here: &str) -> Result<(String, String)> {
    let config = |k: &str| diff::git(&["config", "--get", k]).ok().map(|v| v.trim().to_owned()).filter(|v| !v.is_empty());
    let remote = config(&format!("branch.{here}.pushRemote"))
        .or_else(|| config("remote.pushDefault"))
        .or_else(|| config(&format!("branch.{here}.remote")))
        .with_context(|| format!("{here} has no remote to push to"))?;
    let branch = config(&format!("branch.{here}.merge")).map_or(here.to_owned(), |m| m.trim_start_matches("refs/heads/").to_owned());
    Ok((remote, branch))
}

/// How HEAD in the checkout at `dir` stands against `theirs` (the head where it's pushed).
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
    let dir = std::env::temp_dir().join(format!("dv-push-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let git = |args: &[&str]| diff::git(&[&["-C", &dir.to_string_lossy()], args].concat()).unwrap();
    let commit = |m: &str| git(&["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", m]);
    git(&["init", "-q"]);
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
    let _ = std::fs::remove_dir_all(&dir);
}
