//! Rebasing onto the base branch (`Space g` → rebase): git does it; when it stops on
//! conflicts, Claude resolves them and carries it on. HEAD is kept on a backup branch first.
use super::*;
use std::path::Path;

/// The background run finishing a rebase: when it's done, dv says how it went.
pub(super) const REBASE_LABEL: &str = "resolving the rebase's conflicts";

/// What Claude may run on top of the usual, to carry a rebase on.
pub(super) const REBASE_TOOLS: &[&str] = &["Bash(git status:*)", "Bash(git add:*)", "Bash(git rm:*)", "Bash(git rebase --continue:*)", "Bash(git rebase --skip:*)", "Bash(git rebase --abort:*)"];

use diff::git_in;

/// A rebase stopped part way in `dir` (on conflicts, say).
/// (Not `git am`'s: it keeps its state in `rebase-apply` too, with an `applying` in it.)
pub(super) fn rebasing(dir: &Path) -> bool {
    let Ok(out) = git_in(dir, &["rev-parse", "--git-path", "rebase-merge", "--git-path", "rebase-apply"]) else { return false };
    let mut p = out.lines().map(|l| dir.join(l.trim())); // (relative to `dir`, or absolute)
    let (merge, apply) = (p.next(), p.next());
    merge.is_some_and(|m| m.exists()) || apply.is_some_and(|a| a.exists() && !a.join("applying").exists())
}

/// Where dv notes a rebase it started (the backup branch's name), in the checkout's git dir:
/// so a rebase you started yourself isn't taken for one.
fn marker(dir: &Path) -> Option<std::path::PathBuf> {
    git_in(dir, &["rev-parse", "--git-path", "dv-rebase"]).ok().map(|p| dir.join(p.trim()))
}

/// The rebase dv last started in `dir`, as noted: its backup branch, and what it's onto.
fn noted(dir: &Path) -> Option<(String, String)> {
    let text = std::fs::read_to_string(marker(dir)?).ok()?;
    let mut l = text.lines().map(str::trim);
    Some((l.next().filter(|b| !b.is_empty())?.to_owned(), l.next().unwrap_or("").to_owned()))
}

/// The rebase stopped in `dir`, if it's the one dv started (its backup branch): the note is
/// checked against the rebase's own `orig-head`, so a stale one (dv's rebase ended some other
/// way) doesn't claim a rebase you started.
pub(super) fn dv_rebase(dir: &Path) -> Option<String> {
    let (backup, _) = noted(dir)?;
    let at = git_in(dir, &["rev-parse", "--verify", "-q", &backup]).ok()?;
    let out = git_in(dir, &["rev-parse", "--git-path", "rebase-merge/orig-head", "--git-path", "rebase-apply/orig-head"]).ok()?;
    let orig = out.lines().find_map(|p| std::fs::read_to_string(dir.join(p.trim())).ok())?;
    (orig.trim() == at.trim()).then_some(backup)
}

/// What a stopped rebase asks of you before anything else: dv's, Space g; yours, git.
pub(super) fn stopped_says(dir: &Path) -> &'static str {
    match dv_rebase(dir) {
        Some(_) => "a rebase is stopped here: Space g finishes it (Claude) or aborts it first",
        None => "a rebase you started is stopped here: finish it with git (or Space g aborts it) first",
    }
}

pub(super) fn forget_rebase(dir: &Path) {
    if let Some(m) = marker(dir) {
        _ = std::fs::remove_file(m);
    }
}

/// Rebase HEAD in `dir` onto `onto`, HEAD kept on `backup` first: `Ok(true)` done, `Ok(false)`
/// stopped on conflicts (left so, for resolving; noted as dv's), an error otherwise (then as
/// it was).
pub(super) fn rebase(dir: &Path, onto: &str, backup: &str) -> Result<bool> {
    anyhow::ensure!(git_in(dir, &["status", "--porcelain", "--untracked-files=no"])?.trim().is_empty(), "there are uncommitted changes: commit (or amend) them first");
    git_in(dir, &["branch", "-f", backup, "HEAD"])?;
    // The backup branch stays where it is (no editor: see `diff::git`).
    match git_in(dir, &["-c", "rebase.updateRefs=false", "rebase", "-q", onto]) {
        Ok(_) => Ok(true),
        Err(e) if rebasing(dir) && git_in(dir, &["diff", "--name-only", "--diff-filter=U"]).is_ok_and(|f| !f.trim().is_empty()) => {
            match marker(dir).map(|m| std::fs::write(m, format!("{backup}\n{onto}\n"))) {
                Some(Ok(())) => Ok(false),
                _ => {
                    _ = git_in(dir, &["rebase", "--abort"]);
                    Err(e.context("couldn't note the rebase as dv's: aborted"))
                }
            }
        }
        Err(e) => {
            _ = git_in(dir, &["rebase", "--abort"]);
            Err(e)
        }
    }
}

/// Claude's prompt for carrying on a rebase onto `onto` that stopped on conflicts.
fn rebase_prompt(onto: &str) -> String {
    format!(
        "This branch is being rebased onto {onto}, and the rebase stopped on conflicts. Resolve them: for each \
         conflicted file (`git status`), keep what both sides meant (this branch's change, made on top of what \
         {onto} has now), remove the conflict markers, and `git add` it; then `git rebase --continue`. Repeat \
         until the rebase is done. Run each git command on its own and as it is (`git add <file>`, then \
         `git rebase --continue`): not chained with `&&`, not prefixed, no `-c` (no editor opens: GIT_EDITOR is \
         set already); only those are allowed. Check it still builds and its tests pass before you `--continue` \
         past the last commit, and fix what the rebase broke in that commit (you can't commit afterwards). \
         Don't push. If a conflict is beyond you (you can't tell what's meant), run `git rebase --abort` and say \
         why. First run `dv review-info`: if the notes it lists exist, read them and follow them."
    )
}

impl App {
    /// The branch the PR goes into (as it is now: its base may change), as fetched:
    /// `<remote>/<base>` (`diff::remote`); without a PR, the one the diff's base follows.
    pub(super) fn rebase_onto(&self) -> Option<String> {
        self.pr.as_ref().map(|p| crate::diff::remote_branch(&p.base)).or_else(|| self.follow_base.clone())
    }

    /// `Space g` → rebase: fetch the base, then rebase onto it; on conflicts, Claude resolves
    /// them (in the background) and carries the rebase on.
    pub(super) fn start_rebase(&mut self) {
        let r = (|| -> Result<String> {
            anyhow::ensure!(self.claude.job.is_none(), "Claude is still at work here: rebase once it's done");
            let onto = self.rebase_onto().context("no base branch to rebase onto")?;
            let (remote, branch) = onto.split_once('/').context("no remote base branch")?;
            // Into that ref whatever the remote's refspec covers (a single-branch clone, say).
            git_in(&self.root, &["fetch", "-q", remote, &format!("+refs/heads/{branch}:refs/remotes/{onto}")])?;
            if git_in(&self.root, &["merge-base", "--is-ancestor", &onto, "HEAD"]).is_ok() {
                return Ok(format!("already on {onto}: nothing to rebase"));
            }
            let (short, backup) = self.backup_name()?;
            let done = rebase(&self.root, &onto, &backup)?;
            if done {
                self.follow(&onto);
                return Ok(self.and_reload(format!("rebased onto {onto} (was {short}: {backup}); Space g force-pushes (with a lease)")));
            }
            let reload = self.reload_all(false); // (the diff's base stays till it's done: see follow_rebase)
            match self.rebase_claude() {
                Ok(()) => Ok(with_reload(format!("conflicts rebasing onto {onto}: Claude is resolving them (was {short}: {backup})"), reload)),
                Err(e) => Ok(with_reload(format!("the rebase stopped on conflicts, and Claude couldn't take it on ({e:#}): Space g aborts it"), reload)),
            }
        })();
        self.msg = r.unwrap_or_else(|e| format!("not rebased: {e:#}"));
    }

    /// Claude carries a stopped rebase on: resolves the conflicts, `--continue`s, to the end.
    pub(super) fn rebase_claude(&mut self) -> Result<()> {
        anyhow::ensure!(self.claude.job.is_none(), "Claude is still working on the last request");
        // Onto what it started onto (the PR's base may have changed meanwhile).
        let onto = noted(&self.root).map(|n| n.1).filter(|o| !o.is_empty()).or_else(|| self.rebase_onto()).unwrap_or_else(|| "its base".into());
        let forked = self.claude_pane().is_some();
        self.claude.job = Some(self.spawn_claude(&rebase_prompt(&onto), REBASE_LABEL, forked, REBASE_TOOLS)?);
        Ok(())
    }

    /// Rebased onto `onto`: the diff starts from there (with `--base` given too), from the
    /// next reload on.
    /// Whether it didn't before (`--base` given): then it takes a reload more.
    fn follow(&mut self, onto: &str) -> bool {
        let new = self.follow_base.is_none();
        self.follow_base.get_or_insert_with(|| onto.to_owned());
        new
    }

    /// `Space g` → abort the rebase: back to before it.
    pub(super) fn abort_rebase(&mut self) {
        if self.claude.job.is_some() {
            return self.msg = "Claude is still at the rebase: stop it first (Space a), then abort".into();
        }
        self.msg = match git_in(&self.root, &["rebase", "--abort"]) {
            Ok(_) => {
                forget_rebase(&self.root);
                "rebase aborted: as it was before".into()
            }
            Err(e) => format!("{e:#}"),
        };
        self.reload_tree();
    }

    /// Claude's done with a rebase: how it stands (still stopped, done, or aborted: then HEAD
    /// isn't on the base).
    pub(super) fn rebase_ended(&mut self) {
        // Onto what it started onto (the PR's base may have changed meanwhile).
        let onto = noted(&self.root).map(|n| n.1).filter(|o| !o.is_empty()).or_else(|| self.rebase_onto()).unwrap_or_default();
        if rebasing(&self.root) {
            return self.msg += " · the rebase is still stopped: Space g finishes it (Claude again) or aborts it";
        }
        // Done: HEAD moved on from where it was, onto the base (on a branch or detached; an
        // abort puts it back where it was).
        let backup = noted(&self.root).map(|n| n.0);
        forget_rebase(&self.root);
        let head = git_in(&self.root, &["rev-parse", "HEAD"]).unwrap_or_default();
        let was = backup.and_then(|b| git_in(&self.root, &["rev-parse", &b]).ok()).unwrap_or_default();
        let moved = head.trim() != was.trim() && git_in(&self.root, &["merge-base", "--is-ancestor", &onto, "HEAD"]).is_ok();
        if moved {
            // (poll_claude has reloaded: the diff followed, unless it didn't follow till now.)
            if self.follow(&onto) {
                self.reload_tree();
            }
            let dirty = !git_in(&self.root, &["status", "--porcelain", "--untracked-files=no"]).unwrap_or_default().trim().is_empty();
            self.msg += match dirty {
                true => " · rebased, but Claude left changes uncommitted: check them, commit or amend (Space g), then force-push",
                false => " · rebased: Space g force-pushes (with a lease)",
            };
        } else {
            self.msg += " · Claude aborted the rebase: nothing changed (its answer says why: Space a → its pane)";
        }
    }
}

#[test]
fn rebase_clean_and_conflicting() {
    let dir = crate::diff::scratch_repo("rebase-onto");
    let git = |args: &[&str]| git_in(&dir, args).unwrap();
    let write = |f: &str, s: &str| std::fs::write(dir.join(f), s).unwrap();
    write("a", "1\n");
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "base"]);
    git(&["checkout", "-q", "-b", "pr"]);
    write("b", "pr\n");
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "pr"]);
    git(&["checkout", "-q", "main"]);
    write("a", "2\n");
    git(&["commit", "-q", "-am", "main moves"]);
    git(&["checkout", "-q", "pr"]);
    let before = git(&["rev-parse", "HEAD"]).trim().to_owned();
    // Clean: on top of main, the old HEAD on the backup branch.
    assert!(rebase(&dir, "main", "dv-backup/t").unwrap());
    assert_eq!(git(&["rev-parse", "HEAD~1"]).trim(), git(&["rev-parse", "main"]).trim());
    assert_eq!(git(&["rev-parse", "dv-backup/t"]).trim(), before);
    // Conflicting: stopped, for resolving.
    write("a", "3\n");
    git(&["commit", "-q", "-am", "pr too"]);
    git(&["checkout", "-q", "main"]);
    write("a", "4\n");
    git(&["commit", "-q", "-am", "main again"]);
    git(&["checkout", "-q", "pr"]);
    assert!(!rebase(&dir, "main", "dv-backup/t2").unwrap());
    assert!(rebasing(&dir));
    assert_eq!(dv_rebase(&dir).as_deref(), Some("dv-backup/t2")); // dv's: not one you started
    forget_rebase(&dir);
    git(&["rebase", "--abort"]);
    assert!(!rebasing(&dir));
    // Uncommitted changes: refused.
    write("a", "5\n");
    assert!(rebase(&dir, "main", "dv-backup/t3").is_err());
    _ = std::fs::remove_dir_all(&dir);
}

/// Live: Claude carries a conflicted rebase on to the end. Runs `claude`; costs a run.
/// `cargo test live_claude_rebase -- --ignored --nocapture`
#[test]
#[ignore]
fn live_claude_rebase() {
    let dir = crate::diff::scratch_repo("live-rebase");
    let git = |args: &[&str]| git_in(&dir, args).unwrap();
    let write = |f: &str, s: &str| std::fs::write(dir.join(f), s).unwrap();
    write("greet.py", "def greet(name):\n    return 'Hello, ' + name\n");
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "greet"]);
    git(&["checkout", "-q", "-b", "pr"]);
    // The branch: an exclamation mark. Main meanwhile: a name check. Both on the same line.
    write("greet.py", "def greet(name):\n    return 'Hello, ' + name + '!'\n");
    git(&["commit", "-q", "-am", "greet: excitedly"]);
    git(&["checkout", "-q", "main"]);
    write("greet.py", "def greet(name):\n    return 'Hello, ' + (name or 'stranger')\n");
    git(&["commit", "-q", "-am", "greet: a stranger when no name"]);
    git(&["checkout", "-q", "pr"]);
    assert!(!rebase(&dir, "main", "dv-backup/live").unwrap(), "should stop on the conflict");
    let out = crate::view::claude_command(&rebase_prompt("main"), &dir, &[], REBASE_TOOLS, None).stdin(std::process::Stdio::null()).output().unwrap();
    let said: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
    println!("claude: {}", said["result"].as_str().unwrap_or(&String::from_utf8_lossy(&out.stderr)));
    println!("refused: {}", said["permission_denials"]);
    assert!(!rebasing(&dir), "still stopped");
    assert!(git(&["reflog", "-1", "--format=%gs"]).starts_with("rebase (finish)"), "not finished: {}", git(&["reflog", "-3"]));
    let now = git(&["show", "HEAD:greet.py"]);
    println!("greet.py:\n{now}");
    assert!(!now.contains("<<<<<<<") && now.contains("stranger") && now.contains('!'), "both sides kept");
    assert_eq!(git(&["rev-parse", "HEAD~1"]).trim(), git(&["rev-parse", "main"]).trim());
    _ = std::fs::remove_dir_all(&dir);
}
