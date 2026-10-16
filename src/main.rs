mod diff;
mod gh;
mod highlight;
mod list;
mod lsp;
mod md;
mod theme;
mod view;

use anyhow::Context;
use std::path::{Path, PathBuf};

const USAGE: &str = "usage: dv [base | PR | list] [-p N] [-f] [-w [-b]]
  dv               diff working tree vs merge-base of the PR's base (or origin/HEAD)
  dv main          ... vs merge-base of main
  dv 123           review PR 123 (gh pr checkout 123 first); same as -p 123 / --pr 123
  dv list          pick from the open PRs; Enter reviews one with the flags given here
                   (dv list -wb: each in a worktree on its branch), q goes back to the list
  -f, --force      with a PR: reset the local PR branch to the PR head (drops its local commits)
  -w, --worktree   with a PR: check it out in a worktree next to the repo, <repo>-123
                   (created or updated, detached at the PR head), and review it there
  -b, --branch     with -w: check out the PR's branch there instead (tracking it, so you can
                   push to the PR); git won't if that branch is checked out elsewhere
  --rm             remove the PR's worktree (dv 123 --rm); -f if it has local changes.
                   A -b branch is kept, in case it has unpushed commits
  short flags combine: dv 123 -wb   (worktree on the PR's branch)";

#[derive(Debug, Default, PartialEq)]
struct Opts {
    base: Option<String>,
    pr: Option<u64>,
    force: bool,
    worktree: bool,
    branch: bool,
    rm: bool,
    list: bool,
    help: bool,
}

impl Opts {
    /// The flags a PR from `dv list` opens with, as typed: "-wbf".
    fn flags(&self) -> String {
        let on: String = [(self.worktree, 'w'), (self.branch, 'b'), (self.force, 'f')].iter().filter(|f| f.0).map(|f| f.1).collect();
        if on.is_empty() { on } else { format!("-{on}") }
    }
}

/// Parse args: `-wbf` bundles short flags, `--pr=N` works like `--pr N`, and a bare number is a
/// PR (base branches are never all digits).
fn parse_args(args: impl IntoIterator<Item = String>) -> anyhow::Result<Opts> {
    let mut expanded = vec![];
    for a in args {
        match a.split_once('=') {
            Some((flag, v)) if a.starts_with("--") => expanded.extend([flag.to_owned(), v.to_owned()]),
            _ if a.starts_with('-') && !a.starts_with("--") && a.len() > 2 => {
                expanded.extend(a[1..].chars().map(|c| format!("-{c}")));
            }
            _ => expanded.push(a),
        }
    }
    let mut o = Opts::default();
    let mut args = expanded.into_iter();
    let num = |v: Option<String>| -> anyhow::Result<u64> { v.context("--pr needs a number")?.parse().context("--pr needs a number") };
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => o.help = true,
            "-p" | "--pr" => o.pr = Some(num(args.next())?),
            "-f" | "--force" => o.force = true,
            "-w" | "--worktree" => o.worktree = true,
            "-b" | "--branch" => o.branch = true,
            "--rm" | "--remove" => o.rm = true,
            "list" => o.list = true,
            _ if a.starts_with('-') => anyhow::bail!("unknown flag {a}\n{USAGE}"),
            _ if !a.is_empty() && a.bytes().all(|b| b.is_ascii_digit()) => o.pr = Some(num(Some(a))?),
            _ => o.base = Some(a),
        }
    }
    Ok(o)
}

fn main() -> anyhow::Result<()> {
    let o = parse_args(std::env::args().skip(1))?;
    if o.help {
        println!("{USAGE}");
        return Ok(());
    }
    if o.list && (o.pr.is_some() || o.rm) {
        anyhow::bail!("dv list picks the PR itself\n{USAGE}");
    }
    if o.rm {
        let n = o.pr.context("--rm needs a PR, e.g. `dv 123 --rm`")?;
        return remove_worktree(n, o.force);
    }
    let root = PathBuf::from(diff::git(&["rev-parse", "--show-toplevel"])?.trim());
    std::env::set_current_dir(&root)?;
    if o.branch && !o.worktree {
        anyhow::bail!("--branch goes with --worktree (plain --pr already checks out a branch)\n{USAGE}");
    }
    if o.list {
        return pick_and_review(&o, &root);
    }
    if o.worktree && o.pr.is_none() {
        anyhow::bail!("--worktree needs a PR, e.g. `dv 123 -w`\n{USAGE}");
    }
    let (wt, res) = review(&o, o.pr)?;
    if let Some(dir) = wt {
        match diff::git(&["branch", "--show-current"]).map(|b| b.trim().to_owned()) {
            Ok(b) if !b.is_empty() => eprintln!("PR checked out in {} on branch {b}", dir.display()),
            _ => eprintln!("PR checked out in {}", dir.display()),
        }
    }
    res
}

/// `dv list`: review the picked PR with `o`'s flags, then back to the list, until `q` there.
fn pick_and_review(o: &Opts, root: &Path) -> anyhow::Result<()> {
    let mut list = list::List::new(o.flags());
    loop {
        let mut term = ratatui::init();
        let picked = list.run(&mut term);
        ratatui::restore();
        let Some(n) = picked? else { return Ok(()) };
        let res = review(o, Some(n)).and_then(|(_, res)| res);
        std::env::set_current_dir(root)?; // a worktree review leaves us in the worktree
        if let Err(e) = res {
            list.msg = format!("PR #{n}: {e:#}");
        }
    }
}

/// Check PR `n` out as `o` says and review it. The outer error is setup (checkout, diff); the
/// inner one is from the TUI, which the worktree note is printed after.
fn review(o: &Opts, pr_num: Option<u64>) -> anyhow::Result<(Option<PathBuf>, anyhow::Result<()>)> {
    let wt = match (pr_num, o.worktree) {
        (Some(n), true) => Some(enter_worktree(n, o.branch, o.force)?),
        (Some(n), false) => {
            gh::checkout(n, o.force, false)?;
            None
        }
        (None, _) => None,
    };
    let mut base = o.base.clone();
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
    Ok((wt, res))
}

/// Check PR `n` out in `<repo>-<n>` next to the repo (creating the worktree if needed) and make
/// it the current dir: detached at the PR head, or with `branch`, on the PR's branch (as
/// `gh pr checkout` sets it up, so pushing updates the PR). Refuses a dir that isn't a worktree
/// of this repo; git refuses to update one with local changes.
fn enter_worktree(n: u64, branch: bool, force: bool) -> anyhow::Result<std::path::PathBuf> {
    let (_, dir) = worktree_dir(n)?;
    if dir.exists() {
        ensure_ours(&dir)?;
    } else {
        diff::git(&["worktree", "add", "--detach", &dir.to_string_lossy()])?;
    }
    std::env::set_current_dir(&dir)?;
    gh::checkout(n, force, !branch).map_err(|e| {
        // git words this a few ways depending on the step that hits it.
        let msg = format!("{e:#}");
        let in_use = ["already checked out", "already used by worktree", "checked out at"].iter().any(|p| msg.contains(p));
        let hint = match in_use {
            true => "\nthat branch is checked out in another worktree: drop -b, or switch that checkout to another branch",
            false => "",
        };
        e.context(format!("checking out PR #{n} in {}{hint}", dir.display()))
    })?;
    Ok(dir)
}

/// The main checkout's root (found from inside a worktree too) and PR `n`'s worktree next to it.
fn worktree_dir(n: u64) -> anyhow::Result<(PathBuf, PathBuf)> {
    let common = PathBuf::from(common_dir(Path::new("."))?);
    let main = match common.file_name().is_some_and(|f| f == ".git") {
        true => common.parent().context("no repo dir")?.to_owned(),
        false => PathBuf::from(diff::git(&["rev-parse", "--show-toplevel"])?.trim()),
    };
    let name = main.file_name().context("repo has no dir name")?.to_string_lossy().into_owned();
    let dir = main.parent().context("repo has no parent dir")?.join(format!("{name}-{n}"));
    Ok((main, dir))
}

fn common_dir(d: &Path) -> anyhow::Result<String> {
    Ok(diff::git(&["-C", &d.to_string_lossy(), "rev-parse", "--path-format=absolute", "--git-common-dir"])?.trim().to_owned())
}

/// Only ever touch a dir that's a worktree of this repo.
fn ensure_ours(dir: &Path) -> anyhow::Result<()> {
    if common_dir(dir).ok() != Some(common_dir(Path::new("."))?) {
        anyhow::bail!("{} exists and isn't a worktree of this repo", dir.display());
    }
    Ok(())
}

/// `dv 123 --rm`: remove PR 123's worktree. git refuses if it has local changes (`-f` discards
/// them). A branch checked out there is kept, since it may have unpushed commits.
fn remove_worktree(n: u64, force: bool) -> anyhow::Result<()> {
    let (main, dir) = worktree_dir(n)?;
    if !dir.exists() {
        anyhow::bail!("no worktree at {}", dir.display());
    }
    ensure_ours(&dir)?;
    let branch = diff::git(&["-C", &dir.to_string_lossy(), "branch", "--show-current"]).unwrap_or_default();
    std::env::set_current_dir(&main)?; // not from inside the dir being removed
    let d = dir.to_string_lossy();
    let mut args = vec!["worktree", "remove"];
    if force {
        args.push("--force");
    }
    args.push(&d);
    diff::git(&args).map_err(|e| match format!("{e:#}").contains("modified or untracked") {
        true => e.context(format!("{} has local changes; `dv {n} --rm -f` discards them", dir.display())),
        false => e,
    })?;
    println!("removed {}", dir.display());
    if let Some(b) = Some(branch.trim()).filter(|b| !b.is_empty()) {
        println!("kept branch {b} (delete it with `git branch -D {b}` if it has nothing unpushed)");
    }
    Ok(())
}

#[test]
fn args() {
    let p = |s: &str| parse_args(s.split_whitespace().map(str::to_owned));
    let pr = |n, w, b, f| Opts { pr: Some(n), worktree: w, branch: b, force: f, ..Opts::default() };
    assert_eq!(p("2258 -wb").unwrap(), pr(2258, true, true, false));
    assert_eq!(p("-wbf 2258").unwrap(), pr(2258, true, true, true));
    assert_eq!(p("--pr 2258 -w -b").unwrap(), pr(2258, true, true, false));
    assert_eq!(p("--pr=2258 --worktree").unwrap(), pr(2258, true, false, false));
    assert_eq!(p("-p 7").unwrap(), pr(7, false, false, false));
    assert_eq!(p("main").unwrap(), Opts { base: Some("main".into()), ..Opts::default() });
    assert!(p("-wx").is_err());
    assert_eq!(p("2258 --rm -f").unwrap(), Opts { pr: Some(2258), rm: true, force: true, ..Opts::default() });
    let l = p("list -wbf").unwrap();
    assert!(l.list && l.pr.is_none());
    assert_eq!(l.flags(), "-wbf");
    assert_eq!(p("list").unwrap().flags(), "");
}
