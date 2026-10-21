mod batch;
mod claude;
mod diff;
mod gh;
mod highlight;
mod list;
mod lsp;
mod md;
mod merge;
mod pane;
mod replace;
mod theme;
mod todo;
mod view;

use anyhow::Context;
use std::path::{Path, PathBuf};

const USAGE: &str = "usage: dv [base | PR | list] [-p N] [-f] [-w [-b]]
  dv               diff working tree vs merge-base of the PR's base (or origin/HEAD)
  dv main          ... vs merge-base of main
  dv 123           review PR 123 (gh pr checkout 123 first); same as -p 123 / --pr 123
  dv list          pick from the open PRs; Enter reviews one with the flags given here
                   (dv list -wb: each in a worktree on its branch), q goes back to the list
  dv issues        the open issues (dv list's, I I there): Claude triages them
  dv review-info   for Claude Code (or any tool) writing a review dv shows: the review
                   file's path, and the base and head of the diff dv shows (of PR N with -p N)
  dv review-check  check that file's comments against the diff (exit 1 on errors)
  dv review-comments  for Claude: the PR's review comments on GitHub, as JSON (ids, lines,
                   who, what), to answer them in its review (read only)
  dv rerun-failed  for Claude fixing CI: rerun the failed jobs on the PR's head (only those)
  dv cargo [+T] …  for Claude: cargo check/build/test/clippy/doc/fmt --check on any toolchain T
  dv scratch R     for Claude: a checkout of commit R of its own (detached) to try things in;
                   `dv scratch --rm <dir>` removes one (else they go after a day)
  dv repo-file O/R[@REF] P  for Claude: file P of public GitHub repo O/R (at REF), or a
                   directory's entries (`.`: the top), read only
  dv amend C [F…] [-F M]  fold what isn't committed (in files F) into the branch's commit C, and/or
                   give it the message in file M (a backup branch first)
  dv replace 123 [124 …]   Claude drafts a replacement for those PRs, unpublished, on
                   branch dv/replace-123 in a worktree <repo>-replace-123 (dv list's
                   replacements tab shows it)
  dv replace-info  for Claude: the replacement checked out here (its record, what it replaces)
  dv install-skill install the dv-review skill for Claude Code (~/.claude/skills/dv-review)
  dv version       print dv's version (also --version, -V)
  -f, --force      with a PR: reset the local PR branch to the PR head (drops its local commits)
  -w, --worktree   with a PR: check it out in a worktree next to the repo, <repo>-123
                   (created or updated, detached at the PR head), and review it there
  -b, --branch     with -w: check out the PR's branch there instead (tracking it, so you can
                   push to the PR); git won't if that branch is checked out elsewhere
  --rm             remove the PR's worktree (dv 123 --rm); -f if it has local changes.
                   A -b branch is kept, in case it has unpushed commits
  --no-mouse       leave the mouse to the terminal (dv: wheel scrolls, click moves, drag selects)
  short flags combine: dv 123 -wb   (worktree on the PR's branch)";

/// What dv is doing before its screen is up, on one line that each step overwrites (only on
/// a terminal, so piped output stays clean). `None` clears it.
pub(crate) fn progress(msg: Option<&str>) {
    use std::io::IsTerminal;
    if std::io::stderr().is_terminal() {
        match msg {
            Some(m) => eprint!("\r\x1b[2Kdv: {m}…"),
            None => eprint!("\r\x1b[2K"),
        }
    }
}

#[derive(Debug, Default, PartialEq)]
struct Opts {
    base: Option<String>,
    pr: Option<u64>,
    force: bool,
    worktree: bool,
    branch: bool,
    rm: bool,
    no_mouse: bool,
    list: bool,
    /// review-info / review-check / install-skill / replace / replace-info / version
    cmd: Option<&'static str>,
    /// More PR numbers after the first (`dv replace 12 15`).
    more: Vec<u64>,
    /// `dv amend`'s commit and files.
    rest: Vec<String>,
    /// `dv issues`: dv list, on the issues.
    issues: bool,
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
    // (After a command taking what follows as its own, `dv repo-file o/r -x.txt`: as given.)
    let mut theirs = false;
    for a in args {
        if theirs {
            expanded.push(a);
            continue;
        }
        theirs = matches!(a.as_str(), "amend" | "cargo" | "scratch" | "repo-file");
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
            "version" | "--version" | "-V" => o.cmd = Some("version"),
            "-p" | "--pr" => o.pr = Some(num(args.next())?),
            "-f" | "--force" => o.force = true,
            "-w" | "--worktree" => o.worktree = true,
            "-b" | "--branch" => o.branch = true,
            "--rm" | "--remove" => o.rm = true,
            "--no-mouse" => o.no_mouse = true,
            "list" => o.list = true,
            "issues" => (o.list, o.issues) = (true, true),
            "review-info" => o.cmd = Some("review-info"),
            "review-check" => o.cmd = Some("review-check"),
            "review-comments" => o.cmd = Some("review-comments"),
            "rerun-failed" => o.cmd = Some("rerun-failed"),
            "amend" => (o.cmd, o.rest) = (Some("amend"), args.by_ref().collect()),
            "cargo" => (o.cmd, o.rest) = (Some("cargo"), args.by_ref().collect()),
            "scratch" => (o.cmd, o.rest) = (Some("scratch"), args.by_ref().collect()),
            "repo-file" => (o.cmd, o.rest) = (Some("repo-file"), args.by_ref().collect()),
            "install-skill" => o.cmd = Some("install-skill"),
            "replace" => o.cmd = Some("replace"),
            "replace-info" => o.cmd = Some("replace-info"),
            // For `dv list`'s batch reviews: a process of its own, as it changes directory.
            "prepare-worktree" => o.cmd = Some("prepare-worktree"),
            _ if a.starts_with('-') => anyhow::bail!("unknown flag {a}\n{USAGE}"),
            _ if !a.is_empty() && a.bytes().all(|b| b.is_ascii_digit()) && o.pr.is_some() && o.cmd == Some("replace") => {
                o.more.push(num(Some(a))?)
            }
            _ if !a.is_empty() && a.bytes().all(|b| b.is_ascii_digit()) => o.pr = Some(num(Some(a))?),
            _ => o.base = Some(a),
        }
    }
    Ok(o)
}

fn main() -> anyhow::Result<()> {
    let res = run();
    progress(None); // an error goes on its own line
    res
}

fn run() -> anyhow::Result<()> {
    let mut o = parse_args(std::env::args().skip(1))?;
    view::MOUSE.store(!o.no_mouse, std::sync::atomic::Ordering::Relaxed);
    if o.help {
        println!("{USAGE}");
        return Ok(());
    }
    if o.cmd == Some("version") {
        println!("dv {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    // Where it's run (a crate of its own in a subdirectory, say), needing nothing of dv's.
    if o.cmd == Some("cargo") {
        std::process::exit(checked_cargo(&o.rest)?);
    }
    // Another repo's file, as it is (no newline added): needing no repo here, nor dv's skill.
    if o.cmd == Some("repo-file") {
        let [repo, path] = o.rest.as_slice() else {
            anyhow::bail!("dv repo-file <owner/repo[@ref]> <path> (a path with spaces: quote it; `.`: the top)")
        };
        let text = gh::repo_file(repo, path)?;
        // (Read in part, `| head`: done, not a panic.)
        if let Err(e) = std::io::Write::write_all(&mut std::io::stdout(), text.as_bytes())
            && e.kind() != std::io::ErrorKind::BrokenPipe
        {
            return Err(e.into());
        }
        return Ok(());
    }
    if (o.list || o.cmd.is_some()) && o.rm {
        anyhow::bail!("--rm goes with a PR number, e.g. `dv 123 --rm`\n{USAGE}");
    }
    if o.list && o.pr.is_some() {
        anyhow::bail!("dv list picks the PR itself\n{USAGE}");
    }
    if o.rm {
        let n = o.pr.context("--rm needs a PR, e.g. `dv 123 --rm`")?;
        return remove_worktree(n, o.force).map(|m| println!("{m}"));
    }
    // `dv amend`'s files as git names them, from the repo's root (where dv goes next): those
    // changed of the ones given (`./a`, `../b`, a dir, a glob).
    // Its `-F <file>`: the commit's new message (read here, from where it was given).
    let mut message = None;
    if let Some(i) = o.rest.iter().position(|a| a == "-F" || a == "--file").filter(|_| o.cmd == Some("amend")) {
        let f = o.rest.get(i + 1).context("-F needs a file with the commit's new message")?.clone();
        let m = std::fs::read_to_string(&f).with_context(|| format!("reading {f}"))?;
        anyhow::ensure!(!m.trim().is_empty(), "{f} is empty: no message to give the commit");
        message = Some(m);
        o.rest.drain(i..i + 2);
    }
    if o.cmd == Some("amend") && o.rest.len() > 1 {
        let mut args = vec!["diff", "HEAD", "--name-only", "--no-renames", "--no-relative", "-z", "--"];
        args.extend(o.rest[1..].iter().map(String::as_str));
        let names = diff::git(&args)?;
        let names: Vec<String> = names.split('\0').filter(|n| !n.is_empty()).map(str::to_owned).collect();
        anyhow::ensure!(!names.is_empty(), "nothing uncommitted in {}", o.rest[1..].join(" "));
        o.rest.truncate(1);
        o.rest.extend(names);
    }
    let root = PathBuf::from(diff::git(&["rev-parse", "--show-toplevel"])?.trim());
    std::env::set_current_dir(&root)?;
    // An installed skill follows this dv (its review files, its keys): an older one would
    // have Claude work to rules this dv doesn't follow.
    if o.cmd != Some("install-skill") && claude::update_skill() {
        progress(Some("updated Claude's dv-review skill to this dv's"));
    }
    match o.cmd {
        Some("install-skill") => return install_skill(),
        Some("prepare-worktree") => {
            let n = o.pr.context("prepare-worktree needs a PR number")?;
            let known = gh::pr(Some(n)).ok();
            let (dir, note) = enter_worktree(n, o.branch, false, known.as_ref())?; // -b: it's on the PR's branch; keep it so
            // (Its note, why it's not at the PR's head, if it isn't, on the next line.)
            println!("{}\n{}", dir.display(), note.unwrap_or_default());
            return Ok(());
        }
        Some("replace-info") => return replace::info(),
        Some("scratch") => return scratch(&o.rest).map(|m| println!("{m}")),
        Some("amend") => {
            // The branch's own commits are those over the base dv diffs against (`review-info`'s).
            let base = diff_base(&o, gh::pr(None).ok().as_ref())?;
            println!("{}", view::fold_cli(&o.rest, message.as_deref(), &base)?);
            return Ok(());
        }
        Some("replace") => {
            let prs: Vec<u64> = o.pr.into_iter().chain(o.more.iter().copied()).collect();
            anyhow::ensure!(!prs.is_empty(), "dv replace needs the PRs to replace, e.g. `dv replace 123`");
            eprintln!("Claude is drafting {} (dv list's drafts tab shows it)…", replace::label(prs[0]));
            let took = replace::draft(&prs, &root, "", || {})?;
            println!("drafted ({took}): `dv list`, drafts tab, or {}", worktree_dir_named(&root, &replace::name(prs[0]))?.display());
            return Ok(());
        }
        Some(cmd) => return review_file(cmd, &o),
        None => {}
    }
    // Which remote is GitHub's, asked once here if nothing says (several remotes).
    diff::ensure_remote()?;
    if o.branch && !o.worktree {
        anyhow::bail!("--branch goes with --worktree (plain --pr already checks out a branch)\n{USAGE}");
    }
    if o.list {
        return pick_and_review(&o, &root);
    }
    if o.worktree && o.pr.is_none() {
        anyhow::bail!("--worktree needs a PR, e.g. `dv 123 -w`\n{USAGE}");
    }
    let (wt, res) = review(&o, o.pr, None)?;
    if res.as_ref().is_ok_and(|m| *m) {
        std::env::set_current_dir(&root)?; // out of the worktree it may remove
        let n = o.pr.context("merging needs the PR's number")?;
        eprintln!("{}", merge::merge(n, false)?);
        return Ok(());
    }
    if let Some(dir) = wt {
        match diff::git(&["branch", "--show-current"]).map(|b| b.trim().to_owned()) {
            Ok(b) if !b.is_empty() => eprintln!("PR checked out in {} on branch {b}", dir.display()),
            _ => eprintln!("PR checked out in {}", dir.display()),
        }
    }
    res.map(drop)
}

/// `dv list`: review the picked PR with `o`'s flags, then back to the list, until `q` there.
fn pick_and_review(o: &Opts, root: &Path) -> anyhow::Result<()> {
    let mut list = list::List::new(o.flags());
    let mut issues = list::issues::Issues::new();
    let mut on_issues = o.issues;
    loop {
        let mut term = view::init_terminal();
        // The issues (`dv issues`, `I I`): until you quit there or go back to the PRs.
        if on_issues {
            let res = issues.run(&mut term);
            view::restore_terminal();
            match res? {
                // (Not with Claude still at work on PRs from the list: back there, saying so.)
                // (Its own work: `Issues` asks before it quits.)
                list::issues::IssuesDo::Quit if list.busy() => {
                    list.msg = "Claude is still at work on PRs here: q again (in dv list) quits".into();
                    on_issues = false;
                }
                list::issues::IssuesDo::Quit => return Ok(()),
                list::issues::IssuesDo::Prs => on_issues = false,
                // A PR from an issue (Enter on its link): reviewed, then back to the issues.
                list::issues::IssuesDo::Pr(n) => {
                    if let Some(m) = review_back(o, n, None, root)? {
                        issues.msg = m;
                    }
                }
            }
            continue;
        }
        let picked = list.run(&mut term);
        view::restore_terminal();
        let Some(pick) = picked? else { return Ok(()) };
        if pick.issues {
            on_issues = true;
            continue;
        }
        let n = pick.number;
        if pick.merge {
            list.msg = merge::merge(n, view::in_zellij()).unwrap_or_else(|e| format!("not merged: {e:#}"));
            std::env::set_current_dir(root)?;
            continue;
        }
        if pick.replacement {
            let res = review_replacement(n, root);
            progress(None);
            std::env::set_current_dir(root)?;
            if let Err(e) = res {
                list.msg = format!("{}: {e:#}", replace::label(n));
            }
            continue;
        }
        // A PR from the worktrees tab opens in its worktree, as it was made (on its branch or not).
        let wt;
        let opts = match pick.worktree {
            Some(branch) => {
                wt = Opts { worktree: true, branch, force: false, base: o.base.clone(), ..Opts::default() };
                &wt
            }
            None => o,
        };
        if let Some(m) = review_back(opts, n, pick.pr, root)? {
            list.msg = m;
        }
    }
}

/// Review PR `n` from a list (`known`: as the list has it), then back in `root`: what to say
/// there, if anything (it failed; `Space g` → merge, from the review: merged, or why not).
fn review_back(o: &Opts, n: u64, known: Option<gh::Pr>, root: &Path) -> anyhow::Result<Option<String>> {
    let res = review(o, Some(n), known).and_then(|(_, res)| res);
    progress(None);
    std::env::set_current_dir(root)?; // a worktree review leaves us in the worktree
    Ok(match res {
        Err(e) => Some(format!("PR #{n}: {e:#}")),
        Ok(true) => Some(merge::merge(n, view::in_zellij()).unwrap_or_else(|e| format!("not merged: {e:#}"))),
        Ok(false) => None,
    })
}

/// Review replacement `id` in dv, in its worktree: its diff against its base, and in the top
/// section, what it replaces and Claude's drafts.
fn review_replacement(id: u64, root: &Path) -> anyhow::Result<()> {
    let r = replace::load(id)?;
    let dir = worktree_dir_named(root, &replace::name(id))?;
    anyhow::ensure!(dir.exists(), "its worktree {} is gone (A A on the PR drafts it again)", dir.display());
    ensure_worktree_of(&dir, root)?;
    std::env::set_current_dir(&dir)?;
    let base = diff::merge_base(Some(&crate::diff::remote_branch(&r.base)))?;
    // Published: it's that PR (its comments, CI, review).
    let pr = r.published.and_then(|n| gh::pr(Some(n)).ok());
    let mut app = view::App::new(base, pr)?;
    // Its base followed as a PR's is: a rebase (a redraft's, publishing's) moves the diff's base.
    app.follow_base = Some(crate::diff::remote_branch(&r.base));
    progress(None);
    let mut term = view::init_terminal();
    let res = app.run(&mut term);
    view::restore_terminal();
    res
}

/// Check PR `n` out as `o` says and review it. The outer error is setup (checkout, diff); the
/// inner one is from the TUI, which the worktree note is printed after; its `true`: you asked
/// to merge the PR (`Space g`), which is for the caller to do, out of the review.
/// `known`: PR `pr_num` as GitHub has it, if that's just been looked up (`dv list`).
fn review(o: &Opts, pr_num: Option<u64>, known: Option<gh::Pr>) -> anyhow::Result<(Option<PathBuf>, anyhow::Result<bool>)> {
    // A PR with a worktree already (`<repo>-N`) opens there, as it was made: its branch can't
    // be checked out here too, and that's where you were reviewing it.
    let found = pr_num.filter(|_| !o.worktree).and_then(|n| pr_worktrees().into_iter().find(|w| w.0 == n));
    let adjusted;
    let o = match found {
        Some((_, dir, branch)) => {
            progress(Some(&format!("PR has a worktree at {}; opening it there", dir.display())));
            adjusted = Opts { worktree: true, branch: branch.is_some(), base: o.base.clone(), force: o.force, ..Opts::default() };
            &adjusted
        }
        None => o,
    };
    // The PR first, when it's given: its head says whether there's anything to check out.
    progress(Some("looking up the PR"));
    let known = known.filter(|p| Some(p.number) == pr_num).or_else(|| pr_num.and_then(|n| gh::pr(Some(n)).ok()));
    let (wt, note) = match (pr_num, o.worktree) {
        (Some(n), true) => {
            let (dir, note) = enter_worktree(n, o.branch, o.force, known.as_ref())?;
            (Some(dir), note)
        }
        (Some(_), false) if known.as_ref().is_some_and(|p| at_head(p, false)) => (None, None),
        (Some(n), false) => {
            // On its branch already, with work the PR doesn't have: kept, not updated over.
            let on_it = known.as_ref().is_some_and(|p| {
                let here = diff::git(&["branch", "--show-current"]).unwrap_or_default();
                let here = here.trim();
                here == p.head_branch || here == format!("{}-{}", p.head_owner, p.head_branch)
            });
            match on_it.then(|| local_work(n)).flatten() {
                Some(note) => (None, Some(note)),
                None => {
                    progress(Some(&format!("checking out PR #{n}")));
                    let note = checkout_or_keep(n, o.force, false)?;
                    record_checkout(n);
                    (None, note)
                }
            }
        }
        (None, _) => (None, None),
    };
    // Comments only load when the branch has a PR; plain local diffs still work without one.
    let pr = known.or_else(|| gh::pr(pr_num).ok());
    let base = diff_base(o, pr.as_ref())?;
    // The branch the base follows (not one you named): a rebase onto it moves the diff's base.
    let follow = pr.as_ref().filter(|_| o.base.is_none()).map(|p| crate::diff::remote_branch(&p.base));
    let app = view::App::new(base, pr);
    progress(None);
    let mut app = app?;
    app.follow_base = follow;
    if let Some(n) = note {
        app.notice(n);
    }
    let mut term = view::init_terminal();
    let res = app.run(&mut term);
    view::restore_terminal();
    Ok((wt, res.map(|_| app.merge)))
}

/// The commit the diff starts from: the merge-base with the given base, else the PR's base
/// (its commit if it's here, else its branch, fetched first), else origin/HEAD.
fn diff_base(o: &Opts, pr: Option<&gh::Pr>) -> anyhow::Result<String> {
    let mut base = o.base.clone();
    if let (Some(p), None) = (pr, &base) {
        let here = !p.base_oid.is_empty() && diff::git(&["cat-file", "-e", &format!("{}^{{commit}}", p.base_oid)]).is_ok();
        base = Some(match here {
            true => p.base_oid.clone(),
            false => {
                progress(Some(&format!("fetching {}/{}", crate::diff::remote(), p.base)));
                _ = diff::git(&["fetch", "-q", &crate::diff::remote(), &p.base]);
                crate::diff::remote_branch(&p.base)
            }
        });
    }
    diff::merge_base(base.as_deref())
}

/// `dv review-info` / `dv review-check`, for the current checkout (or PR `-p N`), no checkout.
fn review_file(cmd: &str, o: &Opts) -> anyhow::Result<()> {
    // (Claude may run this when fixing CI, not `gh run rerun`: that could rerun anything. On
    // the head as GitHub has it now.)
    if cmd == "rerun-failed" {
        let p = gh::pr(o.pr).ok().context("no PR here (dv rerun-failed -p N for PR N)")?;
        println!("{}", gh::rerun(&p.head_oid, false)?);
        return Ok(());
    }
    // The PR dv ran Claude on (`claude::PR_ENV`), while this checkout is still on it, or the one
    // named: as dv last fetched it and its threads, if lately (not GitHub again for each of a
    // review's many checks), else from GitHub.
    let theirs = |p: &gh::Pr| diff::git(&["symbolic-ref", "-q", "--short", "HEAD"]).map_or(true, |b| b.trim() == p.head_branch);
    let dv_pr = std::env::var(claude::PR_ENV).ok().and_then(|n| n.parse().ok());
    let cached = o.pr.or(dv_pr).and_then(view::recent_github).filter(|(p, _)| o.pr.is_some() || theirs(p));
    let (pr, threads) = match cached {
        Some((p, t)) => (Some(p), Some(t)),
        None => (gh::pr(o.pr).ok(), None),
    };
    if cmd == "review-comments" {
        return review_comments(pr.as_ref(), threads);
    }
    let path = claude::path(pr.as_ref().map(|p| p.number))?;
    let base = diff_base(o, pr.as_ref())?;
    if cmd == "review-info" {
        let head = diff::git(&["rev-parse", "HEAD"])?;
        println!("file: {}", path.display());
        println!("base: {base}    (`git diff {base}` is the diff dv shows: base → working tree)");
        println!("head: {}", head.trim());
        match &pr {
            Some(p) => println!("pr:   #{} {}", p.number, p.title),
            None => println!("pr:   none (a local review of this branch)"),
        }
        println!("exists: {}", path.exists());
        let notes = claude::notes_path()?;
        println!("notes: {}    (your notes on this repo, kept across reviews; exists: {})", notes.display(), notes.exists());
        if pr.is_some() {
            println!("review comments: `dv review-comments` lists the PR's, on GitHub (to answer them)");
        }
        return Ok(());
    }
    let files = diff::load(&base, None, 3)?;
    let r = claude::check(&path, &files)?;
    for (kind, msgs) in [("fixed", &r.notes), ("warning", &r.warnings), ("error", &r.errors)] {
        msgs.iter().for_each(|m| println!("{kind}: {m}"));
    }
    if !r.errors.is_empty() {
        anyhow::bail!("{} of {} comment(s) can't be placed", r.errors.len(), r.comments);
    }
    let notes = claude::notes_path().ok().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
    let over = claude::notes_over(&notes);
    if let Some(w) = &over {
        println!("warning: {w}");
    }
    println!("ok: {} comment(s), {} warning(s)", r.comments, r.warnings.len() + over.is_some() as usize);
    Ok(())
}

/// `dv review-comments`: the PR's review threads on GitHub as JSON, for Claude to read (and
/// answer, with `"about": <id>` on a comment of its own). Read only.
fn review_comments(pr: Option<&gh::Pr>, threads: Option<serde_json::Value>) -> anyhow::Result<()> {
    let p = pr.context("no PR here (dv review-comments -p N for PR N)")?;
    let review = gh::parse_threads(&threads.map_or_else(|| gh::threads_json(p.number), Ok)?);
    let threads: Vec<serde_json::Value> = review
        .threads
        .iter()
        .map(|t| {
            serde_json::json!({
                "path": t.path, "line": t.line, "start_line": t.start_line, "side": if t.left { "old" } else { "new" },
                "resolved": t.resolved, "outdated": t.outdated,
                "comments": t.comments.iter().map(|c| serde_json::json!({ "id": c.id, "author": c.author, "body": c.body })).collect::<Vec<_>>(),
            })
        })
        .collect();
    println!("{}", serde_json::to_string_pretty(&threads)?);
    Ok(())
}

/// Write the dv-review skill where Claude Code finds personal skills.
fn install_skill() -> anyhow::Result<()> {
    let p = claude::skill_path()?;
    std::fs::create_dir_all(p.parent().context("skill path")?)?;
    std::fs::write(&p, claude::skill_file())?;
    println!("installed {} (dv keeps it up to date from now on)", p.display());
    Ok(())
}

/// `gh pr checkout`, except when you're already on the PR's branch and it has commits that
/// aren't on the PR (unpushed work, or a local rebase): then it stays as it is, and the note
/// returned says so. Never resets your branch unless `force`.
pub(crate) fn checkout_or_keep(n: u64, force: bool, detach: bool) -> anyhow::Result<Option<String>> {
    let Err(e) = gh::checkout(n, force, detach) else { return Ok(None) };
    let msg = format!("{e:#}");
    let diverged = ["Not possible to fast-forward", "Diverging branches"].iter().any(|p| msg.contains(p));
    let here = diff::git(&["branch", "--show-current"]).unwrap_or_default().trim().to_owned();
    let (theirs, owner) = gh::pr(Some(n)).map(|p| (p.head_branch, p.head_owner)).unwrap_or_default();
    // gh names the local branch as the PR's, or `<owner>-<branch>` when that name is taken.
    let on_it = !theirs.is_empty() && (here == theirs || here == format!("{owner}-{theirs}"));
    match (diverged && !force && !detach, on_it) {
        (true, true) => Ok(Some(format!(
            "{here} has commits that aren't on PR #{n}: showing it as is (push to update the PR; -f resets to the PR)"
        ))),
        (true, false) => Err(e.context(format!(
            "your local branch for PR #{n} has diverged from the PR (local commits, or the PR was force-pushed); \
             check it out to review it as is, or `dv {n} -f` to reset it to the PR (dropping its local commits)"
        ))),
        _ => Err(e),
    }
}

/// Commits here that PR `n` doesn't have (yours, or Claude's: a commit, an amend, a rebase): a
/// note saying they're kept, else `None` (nothing here but what dv checked out, or older:
/// updating to the PR is safe). Updating never drops them, `-f` or not: dv records the commit
/// it last checked out (`checkouts/pr-N` in its directory), so a PR its author force-pushed
/// still updates, while work done here since stays until you push it or drop it yourself.
fn local_work(n: u64) -> Option<String> {
    let head = diff::git(&["rev-parse", "HEAD"]).ok()?.trim().to_owned();
    diff::git(&["fetch", "-q", &crate::diff::remote(), &format!("refs/pull/{n}/head")]).ok()?;
    let pr = diff::git(&["rev-parse", "FETCH_HEAD"]).ok()?.trim().to_owned();
    let behind = head == pr || diff::git(&["merge-base", "--is-ancestor", &head, &pr]).is_ok();
    if behind || last_checkout(n).as_deref() == Some(head.as_str()) {
        return None;
    }
    // Work done here: commits on neither the PR nor its base (a checkout left on the base's
    // commits has none: they're not yours to push to the PR).
    let base = gh::pr(Some(n)).map(|p| crate::diff::remote_branch(&p.base)).unwrap_or_default();
    let mut count = vec!["rev-list".to_owned(), "--count".to_owned(), format!("{pr}..HEAD")];
    if !base.is_empty() {
        count.extend(["--not".to_owned(), base]);
    }
    let ahead = diff::git(&count.iter().map(String::as_str).collect::<Vec<_>>()).unwrap_or_default();
    let ahead = ahead.trim();
    if ahead == "0" {
        return None;
    }
    let (s, are, them) = if ahead == "1" { ("", "isn't", "it") } else { ("s", "aren't", "them") };
    Some(format!(
        "{ahead} commit{s} here {are} on PR #{n} (yours or Claude's): kept, not updated to the PR. \
         Space g pushes {them}; to drop {them} instead, `git reset --hard {}`",
        &pr[..pr.len().min(12)]
    ))
}

fn checkout_record(n: u64) -> Option<PathBuf> {
    claude::dv_dir().ok().map(|d| d.join("checkouts").join(format!("pr-{n}")))
}

/// This dv's binary, to run another of: as it is now. Replaced while dv runs (an install),
/// Linux gives the old one's path with " (deleted)" on it, which isn't there to run: the new
/// one at its path then (else `dv` on PATH).
/// Whether process `pid` is running: there, and not one that's exited and waits for its parent
/// to collect it (a zombie, `kill -0` all the same: a Claude run of a view since closed, under
/// `dv list`).
pub(crate) fn alive(pid: u32) -> bool {
    // Linux: its state in /proc, read (no process started: callers ask often); else `ps`.
    if let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        return stat.rsplit_once(") ").is_some_and(|(_, rest)| !rest.starts_with('Z'));
    }
    if std::path::Path::new("/proc/self/stat").exists() {
        return false; // (Linux, and it isn't there)
    }
    let out = std::process::Command::new("ps").args(["-o", "stat=", "-p", &pid.to_string()]).stderr(std::process::Stdio::null()).output();
    out.is_ok_and(|o| o.status.success() && o.stdout.first().is_some_and(|&s| s != b'Z'))
}

#[test]
fn a_zombie_isnt_alive() {
    assert!(alive(std::process::id()));
    // Exited, not waited for yet: a zombie.
    let mut child = std::process::Command::new("true").spawn().unwrap();
    let pid = child.id();
    std::thread::sleep(std::time::Duration::from_millis(200));
    assert!(!alive(pid));
    _ = child.wait();
    assert!(!alive(pid));
}

pub(crate) fn self_exe() -> PathBuf {
    std::env::current_exe().map_or_else(|_| PathBuf::from("dv"), runnable)
}

/// `p` (this binary's path, as Linux gives it) as one there is to run.
fn runnable(p: PathBuf) -> PathBuf {
    if p.exists() {
        return p;
    }
    p.to_str().and_then(|s| s.strip_suffix(" (deleted)")).map(PathBuf::from).filter(|p| p.exists()).unwrap_or_else(|| PathBuf::from("dv"))
}

/// The commit dv last checked PR `n` out at, here or in its worktree.
pub(crate) fn last_checkout(n: u64) -> Option<String> {
    Some(std::fs::read_to_string(checkout_record(n)?).ok()?.trim().to_owned())
}

/// Note what PR `n` is checked out at now, for `local_work` next time.
fn record_checkout(n: u64) {
    let (Some(p), Ok(head)) = (checkout_record(n), diff::git(&["rev-parse", "HEAD"])) else { return };
    _ = std::fs::create_dir_all(p.parent().unwrap_or(&p));
    _ = std::fs::write(p, head.trim());
}

/// Already at PR `p`'s head as `gh pr checkout` would leave it (`detach`ed, or on its branch),
/// so there's nothing to fetch.
fn at_head(p: &gh::Pr, detach: bool) -> bool {
    let head = diff::git(&["rev-parse", "HEAD"]).unwrap_or_default();
    let here = diff::git(&["branch", "--show-current"]).unwrap_or_default();
    let here = here.trim();
    // gh names the local branch as the PR's, or `<owner>-<branch>` when that name is taken.
    let on_it = match detach {
        true => here.is_empty(),
        false => !here.is_empty() && (here == p.head_branch || here == format!("{}-{}", p.head_owner, p.head_branch)),
    };
    !p.head_oid.is_empty() && head.trim() == p.head_oid && on_it
}

/// Check PR `n` out in `<repo>-<n>` next to the repo (creating the worktree if needed) and make
/// it the current dir: detached at the PR head, or with `branch`, on the PR's branch (as
/// `gh pr checkout` sets it up, so pushing updates the PR). Refuses a dir that isn't a worktree
/// of this repo; git refuses to update one with local changes.
fn enter_worktree(n: u64, branch: bool, force: bool, known: Option<&gh::Pr>) -> anyhow::Result<(PathBuf, Option<String>)> {
    let (_, dir) = worktree_dir(n)?;
    // On its branch (-b), when that branch is checked out in another worktree already (a
    // replacement's own, say): there, as it is (git won't check one branch out twice).
    if let Some(p) = known.filter(|_| branch)
        && let Some((other, one_line)) = worktree_on_branch(p).filter(|d| d.0 != dir)
    {
        std::env::set_current_dir(&other)?;
        let apart = if one_line { "" } else { " (diverged from the PR's head: Space g says how)" };
        let note = format!("#{n}'s branch {} is checked out in {}: opened there{apart}", p.head_branch, other.display());
        return Ok((other, Some(note)));
    }
    let existed = dir.exists();
    if existed {
        ensure_ours(&dir)?;
    } else {
        // At the PR's head from the start: one left behind half set up (gh failed, dv was
        // killed) is then just behind the PR, never someone else's commits taken for work here.
        progress(Some(&format!("creating the worktree {}", dir.display())));
        // (Its own ref, not FETCH_HEAD: `dv list` prepares several worktrees at once.)
        let at = format!("refs/dv/pull/{n}");
        diff::git(&["fetch", "-q", &crate::diff::remote(), &format!("+refs/pull/{n}/head:{at}")])?;
        diff::git(&["worktree", "add", "--detach", &dir.to_string_lossy(), &at])?;
        _ = diff::git(&["update-ref", "-d", &at]); // (its HEAD keeps the commit)
    }
    std::env::set_current_dir(&dir)?;
    if !existed {
        record_checkout(n); // what dv put here: nothing here is work done since
    }
    if known.is_some_and(|p| at_head(p, !branch)) {
        record_checkout(n);
        return Ok((dir, None));
    }
    // Work done here since (yours, Claude's) stays: not updated to the PR over it.
    if existed && let Some(note) = local_work(n) {
        return Ok((dir, Some(note)));
    }
    progress(Some(&format!("checking out PR #{n} in {}", dir.display())));
    let note = checkout_or_keep(n, force, !branch).map_err(|e| {
        // git words this a few ways depending on the step that hits it.
        let msg = format!("{e:#}");
        let in_use = ["already checked out", "already used by worktree", "checked out at"].iter().any(|p| msg.contains(p));
        let hint = match in_use {
            true => "\nthat branch is checked out in another worktree: drop -b, or switch that checkout to another branch",
            false => "",
        };
        e.context(format!("checking out PR #{n} in {}{hint}", dir.display()))
    })?;
    record_checkout(n);
    Ok((dir, note))
}

/// The main checkout's root (found from inside a worktree too) and PR `n`'s worktree next to it.
pub(crate) fn worktree_dir(n: u64) -> anyhow::Result<(PathBuf, PathBuf)> {
    let main = main_checkout(Path::new("."))?;
    Ok((main, worktree_dir_named(Path::new("."), &n.to_string())?))
}

/// The main checkout of the repo `d` is in (from inside a worktree too).
fn main_checkout(d: &Path) -> anyhow::Result<PathBuf> {
    let common = PathBuf::from(common_dir(d)?);
    Ok(match common.file_name().is_some_and(|f| f == ".git") {
        true => common.parent().context("no repo dir")?.to_owned(),
        false => PathBuf::from(diff::git(&["-C", &d.to_string_lossy(), "rev-parse", "--show-toplevel"])?.trim()),
    })
}

/// A worktree's dir next to the main checkout of the repo `d` is in: `<repo>-<suffix>`.
pub(crate) fn worktree_dir_named(d: &Path, suffix: &str) -> anyhow::Result<PathBuf> {
    let main = main_checkout(d)?;
    let name = main.file_name().context("repo has no dir name")?.to_string_lossy().into_owned();
    Ok(main.parent().context("repo has no parent dir")?.join(format!("{name}-{suffix}")))
}

/// Only ever touch a dir that's a worktree of the repo `root` is in.
pub(crate) fn ensure_worktree_of(dir: &Path, root: &Path) -> anyhow::Result<()> {
    if common_dir(dir).ok() != Some(common_dir(root)?) {
        anyhow::bail!("{} exists and isn't a worktree of this repo", dir.display());
    }
    Ok(())
}

/// PR worktrees of this repo (`<repo>-<n>` next to it, as `-w` makes them): (PR, dir, branch,
/// or `None` when detached).
pub(crate) fn pr_worktrees() -> Vec<(u64, PathBuf, Option<String>)> {
    let Ok((main, _)) = worktree_dir(0) else { return vec![] };
    let (Some(parent), Some(name)) = (main.parent(), main.file_name().map(|n| n.to_string_lossy().into_owned())) else { return vec![] };
    let list = diff::git(&["worktree", "list", "--porcelain"]).unwrap_or_default();
    let mut out = vec![];
    // Blocks of "worktree <path>", "HEAD <sha>", then "branch refs/heads/<b>" or "detached".
    for block in list.split("\n\n") {
        let field = |k: &str| block.lines().find_map(|l| l.strip_prefix(k));
        let Some(path) = field("worktree ").map(PathBuf::from) else { continue };
        let n = path.file_name().and_then(|f| f.to_str()?.strip_prefix(&format!("{name}-"))?.parse().ok());
        if let (Some(n), true) = (n, path.parent() == Some(parent)) {
            out.push((n, path, field("branch refs/heads/").map(str::to_owned)));
        }
    }
    out
}

fn common_dir(d: &Path) -> anyhow::Result<String> {
    Ok(diff::git(&["-C", &d.to_string_lossy(), "rev-parse", "--path-format=absolute", "--git-common-dir"])?.trim().to_owned())
}

/// `dv scratch <rev>`: for Claude, a checkout of its own at `rev` (detached, in dv's directory,
/// where it may write), to find out rather than ask: whether a test fails without the fix,
/// whether the base builds. Ones over a day old go first; `dv scratch --rm <dir>` removes one
/// now. What to tell Claude.
fn scratch(rest: &[String]) -> anyhow::Result<String> {
    let root = claude::scratch_dir()?.canonicalize()?;
    // (git's record of it too: pruned only if git couldn't remove it, not the user's others.)
    let remove = |d: &std::path::Path| {
        let failed = diff::git(&["worktree", "remove", "--force", &d.to_string_lossy()]).is_err();
        _ = std::fs::remove_dir_all(d);
        if failed {
            _ = diff::git(&["worktree", "prune"]);
        }
    };
    if rest.first().map(String::as_str) == Some("--rm") {
        let given = rest.get(1).context("dv scratch --rm <dir>")?;
        let d = PathBuf::from(given).canonicalize().with_context(|| format!("no {given}"))?;
        anyhow::ensure!(is_scratch(&root, &d), "{} isn't one of dv's scratch checkouts", d.display());
        remove(&d);
        return Ok(format!("removed {}", d.display()));
    }
    let rev = rest.first().filter(|r| !r.starts_with('-')).context("dv scratch <rev>, e.g. `dv scratch HEAD` or `dv scratch origin/master`")?;
    let day = std::time::Duration::from_secs(24 * 60 * 60);
    for e in std::fs::read_dir(&root).into_iter().flatten().flatten() {
        if e.metadata().and_then(|m| m.modified()).is_ok_and(|t| t.elapsed().is_ok_and(|a| a > day)) {
            remove(&e.path());
        }
    }
    let sha = diff::git(&["rev-parse", "--short", "--verify", "--end-of-options", &format!("{rev}^{{commit}}")])?.trim().to_owned();
    let dir = root.join(format!("{sha}-{}", std::process::id()));
    let d = dir.display();
    diff::git(&["worktree", "add", "--detach", "--quiet", &dir.to_string_lossy(), &sha])?;
    Ok(format!(
        "{d}\n(a checkout of {rev} at {sha}, detached and yours: edit its files, then build and test it with \
         `cargo test --manifest-path {d}/Cargo.toml` (or any cargo command allowed, with --manifest-path; it \
         builds from nothing, and takes the toolchain and cargo config of where you are, not its own); removed \
         after a day, or now with `dv scratch --rm {d}`)"
    ))
}

/// Whether `d` is one of the scratch checkouts in `root` (itself: not a directory in one).
fn is_scratch(root: &std::path::Path, d: &std::path::Path) -> bool {
    d.parent() == Some(root)
}

/// `dv cargo [+toolchain] <check|build|test|clippy|doc|fmt --check> …`: for Claude, cargo on any
/// toolchain (as CI pins one), as a permission rule can't name every version; only what checks
/// the code, nothing that changes or publishes it. Its exit code.
fn checked_cargo(args: &[String]) -> anyhow::Result<i32> {
    let sub = args.iter().find(|a| !a.starts_with(['+', '-'])).map(String::as_str).unwrap_or(""); // (past `-q` and the like)
    let fmt_check = sub == "fmt" && args.iter().any(|a| a == "--check");
    anyhow::ensure!(
        ["check", "build", "test", "clippy", "doc"].contains(&sub) || fmt_check,
        "dv cargo runs check, build, test, clippy, doc or fmt --check (on any +toolchain), not `{sub}`"
    );
    Ok(std::process::Command::new("cargo").args(args).status()?.code().unwrap_or(1))
}

/// Only ever touch a dir that's a worktree of this repo.
/// The worktree of this repo that has PR `p`'s branch checked out, if one does and it's the
/// PR's (not just a branch of the same name, a fork's `master`, say): it pushes to the PR's
/// branch (diverged or not: a rebase there, an amend), or its history and the PR head are one
/// line (either contains the other). And whether it's one line with the PR's head.
fn worktree_on_branch(p: &gh::Pr) -> Option<(PathBuf, bool)> {
    let (branch, oid) = (p.head_branch.as_str(), p.head_oid.as_str());
    let list = diff::git(&["worktree", "list", "--porcelain"]).ok()?;
    let dir = checked_out_at(&list, branch)?;
    let d = dir.to_string_lossy().into_owned();
    let git = |args: &[&str]| diff::git(&[&["-C", d.as_str()], args].concat()).ok().map(|s| s.trim().to_owned());
    // Where Space g there pushes (as it decides it): a remote's URL, or the URL itself.
    let pushes_there = diff::push_target(&dir, branch).is_some_and(|(remote, to)| {
        let url = git(&["remote", "get-url", "--push", &remote]).unwrap_or(remote);
        let repo = url.trim_end_matches('/').trim_end_matches(".git").to_lowercase();
        let want = p.head_repo.to_lowercase();
        !want.is_empty() && to == branch && repo.strip_suffix(&want).is_some_and(|r| r.ends_with(['/', ':']))
    });
    let one_line = !oid.is_empty() && (git(&["merge-base", "--is-ancestor", oid, "HEAD"]).is_some() || git(&["merge-base", "--is-ancestor", "HEAD", oid]).is_some());
    (pushes_there || one_line).then_some((dir, one_line))
}

/// From `git worktree list --porcelain`: the worktree with `branch` checked out.
fn checked_out_at(list: &str, branch: &str) -> Option<PathBuf> {
    let want = format!("branch refs/heads/{branch}");
    list.split("\n\n").find(|w| w.lines().any(|l| l == want)).and_then(|w| w.lines().find_map(|l| l.strip_prefix("worktree "))).map(PathBuf::from)
}

fn ensure_ours(dir: &Path) -> anyhow::Result<()> {
    ensure_worktree_of(dir, Path::new("."))
}

/// `dv 123 --rm` (and `D D` in `dv list`): remove PR 123's worktree, saying what was done. git
/// refuses if it has local changes (`force` discards them). A branch checked out there is
/// kept, since it may have unpushed commits.
pub(crate) fn remove_worktree(n: u64, force: bool) -> anyhow::Result<String> {
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
    Ok(match Some(branch.trim()).filter(|b| !b.is_empty()) {
        Some(b) => format!("removed {}; kept branch {b} (`git branch -D {b}` deletes it, if nothing on it is unpushed)", dir.display()),
        None => format!("removed {}", dir.display()),
    })
}

#[test]
fn scratch_rm_only_a_checkout() {
    let root = std::path::Path::new("/d/scratch");
    assert!(is_scratch(root, &root.join("abc-1")));
    assert!(!is_scratch(root, root)); // all of them
    assert!(!is_scratch(root, &root.join("abc-1/src"))); // in one
    assert!(!is_scratch(root, std::path::Path::new("/tmp")));
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
    assert_eq!(p("review-info -p 7").unwrap(), Opts { pr: Some(7), cmd: Some("review-info"), ..Opts::default() });
    for v in ["version", "--version", "-V"] {
        assert_eq!(p(v).unwrap(), Opts { cmd: Some("version"), ..Opts::default() });
    }
    assert_eq!(p("review-check").unwrap().cmd, Some("review-check"));
    let a = p("amend abc1234 src/a.rs 12").unwrap();
    assert_eq!((a.cmd, a.rest, a.pr, a.base), (Some("amend"), vec!["abc1234".into(), "src/a.rs".into(), "12".into()], None, None));
    let r = p("replace 12 15").unwrap();
    assert_eq!((r.cmd, r.pr, r.more), (Some("replace"), Some(12), vec![15]));
}

#[test]
fn branch_checked_out_elsewhere() {
    let list = "worktree /r/bdk\nHEAD 1111\nbranch refs/heads/master\n\nworktree /r/bdk-2390\nHEAD 2222\ndetached\n\nworktree /r/bdk-replace-2345\nHEAD 8e81\nbranch refs/heads/dv/replace-2345\n";
    assert_eq!(checked_out_at(list, "dv/replace-2345"), Some(PathBuf::from("/r/bdk-replace-2345")));
    assert_eq!(checked_out_at(list, "dv/replace"), None); // the whole name, not a prefix
    assert_eq!(checked_out_at(list, "fix"), None);
}

#[test]
fn self_exe_after_an_install() {
    // As a running dv sees itself: here, the test binary, which is there.
    assert!(self_exe().exists());
    // Replaced by an install meanwhile: the path Linux gives isn't there; the new one at it is.
    let dir = std::env::temp_dir().join(format!("dv-self-exe-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let bin = dir.join("dv");
    std::fs::write(&bin, "").unwrap();
    assert_eq!(runnable(dir.join("dv (deleted)")), bin);
    assert_eq!(runnable(dir.join("gone (deleted)")), PathBuf::from("dv")); // nothing there: PATH's
    _ = std::fs::remove_dir_all(&dir);
}
