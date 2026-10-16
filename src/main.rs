mod diff;
mod gh;
mod highlight;
mod lsp;
mod md;
mod theme;
mod view;

use anyhow::Context;

const USAGE: &str = "usage: dv [base] [--pr N [-f] [-w]]
  dv               diff working tree vs merge-base of the PR's base (or origin/HEAD)
  dv main          ... vs merge-base of main
  dv --pr 123      gh pr checkout 123 first
  -f, --force      with --pr: reset the local PR branch to the PR head (drops its local commits)
  -w, --worktree   with --pr: check the PR out in a worktree next to the repo, <repo>-123
                   (created or updated, detached at the PR head), and review it there";

fn main() -> anyhow::Result<()> {
    let (mut base, mut pr_num, mut force, mut worktree) = (None, None, false, false);
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            "--pr" => pr_num = Some(args.next().context("--pr needs a number")?.parse().context("--pr needs a number")?),
            "-f" | "--force" => force = true,
            "-w" | "--worktree" => worktree = true,
            _ if a.starts_with('-') => anyhow::bail!("unknown flag {a}\n{USAGE}"),
            _ => base = Some(a),
        }
    }
    let root = diff::git(&["rev-parse", "--show-toplevel"])?;
    std::env::set_current_dir(root.trim())?;
    let wt = match (pr_num, worktree) {
        (None, true) => anyhow::bail!("--worktree needs --pr N\n{USAGE}"),
        (Some(n), true) => Some(enter_worktree(n)?),
        (Some(n), false) => {
            gh::checkout(n, force, false)?;
            None
        }
        (None, false) => None,
    };
    // Comments only load when the branch has a PR; plain local diffs still work without one.
    let pr = gh::pr(pr_num).ok();
    if let (Some(p), None) = (&pr, &base) {
        _ = diff::git(&["fetch", "-q", "origin", &p.base]);
        base = Some(format!("origin/{}", p.base));
    }
    let base = diff::merge_base(base.as_deref())?;
    let mut app = view::App::new(base, pr)?;
    let mut term = ratatui::init();
    let res = app.run(&mut term);
    ratatui::restore();
    if let Some(dir) = wt {
        eprintln!("PR checked out in {}", dir.display());
    }
    res
}

/// Check PR `n` out in `<repo>-<n>` next to the repo (creating the worktree if needed), detached
/// at the PR head, and make it the current dir. Refuses a dir that isn't a worktree of this repo;
/// git refuses to update one with local changes.
fn enter_worktree(n: u64) -> anyhow::Result<std::path::PathBuf> {
    let root = std::env::current_dir()?;
    let name = root.file_name().context("repo has no dir name")?.to_string_lossy();
    let dir = root.parent().context("repo has no parent dir")?.join(format!("{name}-{n}"));
    let common = |d: &std::path::Path| {
        diff::git(&["-C", &d.to_string_lossy(), "rev-parse", "--path-format=absolute", "--git-common-dir"])
            .map(|s| s.trim().to_owned())
    };
    if dir.exists() {
        if common(&dir).ok() != Some(common(&root)?) {
            anyhow::bail!("{} exists and isn't a worktree of this repo", dir.display());
        }
    } else {
        diff::git(&["worktree", "add", "--detach", &dir.to_string_lossy()])?;
    }
    std::env::set_current_dir(&dir)?;
    gh::checkout(n, false, true).with_context(|| format!("checking out PR #{n} in {}", dir.display()))?;
    Ok(dir)
}
