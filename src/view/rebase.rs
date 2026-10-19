//! Rebasing onto the base branch (`Space g` → rebase): git does it; when it stops on
//! conflicts, Claude resolves them and carries it on. HEAD is kept on a backup branch first.
use super::*;
use std::path::Path;

/// The background run finishing a rebase: when it's done, dv says how it went.
pub(super) const REBASE_LABEL: &str = "resolving the rebase's conflicts";

/// What Claude may run on top of the usual, to carry a rebase on.
const REBASE_TOOLS: &[&str] = &["Bash(git status:*)", "Bash(git add:*)", "Bash(git rm:*)", "Bash(git rebase --continue:*)", "Bash(git rebase --skip:*)", "Bash(git rebase --abort:*)"];

fn git_in(dir: &Path, args: &[&str]) -> Result<String> {
    let d = dir.to_string_lossy();
    diff::git(&[&["-C", d.as_ref()], args].concat())
}

/// A rebase stopped part way in `dir` (on conflicts, say).
pub(super) fn rebasing(dir: &Path) -> bool {
    ["rebase-merge", "rebase-apply"].iter().any(|d| git_in(dir, &["rev-parse", "--git-path", d]).is_ok_and(|p| dir.join(p.trim()).exists() || Path::new(p.trim()).exists()))
}

/// Rebase HEAD in `dir` onto `onto`, HEAD kept on `backup` first: `Ok(true)` done, `Ok(false)`
/// stopped on conflicts (left so, for resolving), an error otherwise (then as it was).
pub(super) fn rebase(dir: &Path, onto: &str, backup: &str) -> Result<bool> {
    anyhow::ensure!(git_in(dir, &["status", "--porcelain", "--untracked-files=no"])?.trim().is_empty(), "there are uncommitted changes: commit (or amend) them first");
    git_in(dir, &["branch", "-f", backup, "HEAD"])?;
    // The backup branch stays where it is; no editor to wait on.
    match git_in(dir, &["-c", "rebase.updateRefs=false", "-c", "core.editor=true", "rebase", "-q", onto]) {
        Ok(_) => Ok(true),
        Err(_) if rebasing(dir) && !git_in(dir, &["diff", "--name-only", "--diff-filter=U"])?.trim().is_empty() => Ok(false),
        Err(e) => {
            _ = git_in(dir, &["rebase", "--abort"]);
            Err(e)
        }
    }
}

impl App {
    /// The branch the PR goes into, as fetched: `origin/<base>`.
    pub(super) fn rebase_onto(&self) -> Option<String> {
        self.pr.as_ref().map(|p| format!("origin/{}", p.base)).or_else(|| self.follow_base.clone())
    }

    /// `Space g` → rebase: fetch the base, then rebase onto it; on conflicts, Claude resolves
    /// them (in the background) and carries the rebase on.
    pub(super) fn start_rebase(&mut self) {
        let r = (|| -> Result<String> {
            anyhow::ensure!(self.claude.job.is_none(), "Claude is still at work here: rebase once it's done");
            let onto = self.rebase_onto().context("no base branch to rebase onto")?;
            let (remote, branch) = onto.split_once('/').context("no remote base branch")?;
            git_in(&self.root, &["fetch", "-q", remote, branch])?;
            let (short, backup) = self.backup_name()?;
            let done = rebase(&self.root, &onto, &backup)?;
            self.reload_tree();
            match done {
                true => Ok(format!("rebased onto {onto} (was {short}: {backup}); Space g force-pushes (with a lease)")),
                false => {
                    self.rebase_claude()?;
                    Ok(format!("conflicts rebasing onto {onto}: Claude is resolving them (was {short}: {backup})"))
                }
            }
        })();
        self.msg = r.unwrap_or_else(|e| format!("not rebased: {e:#}"));
    }

    /// Claude carries a stopped rebase on: resolves the conflicts, `--continue`s, to the end.
    pub(super) fn rebase_claude(&mut self) -> Result<()> {
        if let Some(why) = self.claude_blocked(false) {
            anyhow::bail!(why);
        }
        let onto = self.rebase_onto().unwrap_or_else(|| "its base".into());
        let prompt = format!(
            "This branch is being rebased onto {onto}, and the rebase stopped on conflicts. Resolve them: for each \
             conflicted file (`git status`), keep what both sides meant (this branch's change, made on top of what \
             {onto} has now), remove the conflict markers, and `git add` it; then `git rebase --continue`. Repeat \
             until the rebase is done. Then check it still builds and its tests pass. Don't push. If a conflict is \
             beyond you (you can't tell what's meant), run `git rebase --abort` and say why. First run `dv review-info`: \
             if the notes it lists exist, read them and follow them."
        );
        let forked = self.claude_pane().is_some();
        self.claude.job = Some(self.spawn_claude(&prompt, REBASE_LABEL, forked, REBASE_TOOLS)?);
        Ok(())
    }

    /// `Space g` → abort the rebase: back to before it.
    pub(super) fn abort_rebase(&mut self) {
        self.msg = match git_in(&self.root, &["rebase", "--abort"]) {
            Ok(_) => "rebase aborted: as it was before".into(),
            Err(e) => format!("{e:#}"),
        };
        self.reload_tree();
    }

    /// Claude's done with a rebase: how it stands.
    pub(super) fn rebase_ended(&mut self) {
        self.reload_tree();
        if rebasing(&self.root) {
            self.msg += " · the rebase is still stopped: Space g finishes it (Claude again) or aborts it";
        } else {
            self.msg += " · Space g force-pushes (with a lease)";
        }
    }
}

#[test]
fn rebase_clean_and_conflicting() {
    let dir = std::env::temp_dir().join(format!("dv-rebase-onto-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let git = |args: &[&str]| git_in(&dir, args).unwrap();
    let write = |f: &str, s: &str| std::fs::write(dir.join(f), s).unwrap();
    git(&["init", "-q", "-b", "main"]);
    for (k, v) in [("user.name", "t"), ("user.email", "t@t"), ("commit.gpgsign", "false"), ("core.hooksPath", "/dev/null")] {
        git(&["config", k, v]);
    }
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
    git(&["rebase", "--abort"]);
    assert!(!rebasing(&dir));
    // Uncommitted changes: refused.
    write("a", "5\n");
    assert!(rebase(&dir, "main", "dv-backup/t3").is_err());
    _ = std::fs::remove_dir_all(&dir);
}
