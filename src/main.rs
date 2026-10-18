mod batch;
mod claude;
mod diff;
mod gh;
mod highlight;
mod list;
mod lsp;
mod md;
mod merge;
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
  dv review-info   for Claude Code (or any tool) writing a review dv shows: the review
                   file's path, and the base and head of the diff dv shows (of PR N with -p N)
  dv review-check  check that file's comments against the diff (exit 1 on errors)
  dv review-comments  for Claude: the PR's review comments on GitHub, as JSON (ids, lines,
                   who, what), to answer them in its review (read only)
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
            "version" | "--version" | "-V" => o.cmd = Some("version"),
            "-p" | "--pr" => o.pr = Some(num(args.next())?),
            "-f" | "--force" => o.force = true,
            "-w" | "--worktree" => o.worktree = true,
            "-b" | "--branch" => o.branch = true,
            "--rm" | "--remove" => o.rm = true,
            "--no-mouse" => o.no_mouse = true,
            "list" => o.list = true,
            "review-info" => o.cmd = Some("review-info"),
            "review-check" => o.cmd = Some("review-check"),
            "review-comments" => o.cmd = Some("review-comments"),
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
    let o = parse_args(std::env::args().skip(1))?;
    view::MOUSE.store(!o.no_mouse, std::sync::atomic::Ordering::Relaxed);
    if o.help {
        println!("{USAGE}");
        return Ok(());
    }
    if o.cmd == Some("version") {
        println!("dv {}", env!("CARGO_PKG_VERSION"));
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
            let (dir, _) = enter_worktree(n, o.branch, false, known.as_ref())?; // -b: it's on the PR's branch; keep it so
            println!("{}", dir.display());
            return Ok(());
        }
        Some("replace-info") => return replace::info(),
        Some("replace") => {
            let prs: Vec<u64> = o.pr.into_iter().chain(o.more.iter().copied()).collect();
            anyhow::ensure!(!prs.is_empty(), "dv replace needs the PRs to replace, e.g. `dv replace 123`");
            // Your last instructions for replacements (dv saves them as you write them); a new PR
            // has its task in its record.
            let extra = if replace::is_new(prs[0]) { String::new() } else { claude::instructions("replace") };
            eprintln!("Claude is drafting {} (dv list's drafts tab shows it)…", replace::label(prs[0]));
            let took = replace::draft(&prs, &root, &extra, || {})?;
            println!("drafted ({took}): `dv list`, drafts tab, or {}", worktree_dir_named(&root, &replace::name(prs[0]))?.display());
            return Ok(());
        }
        Some(cmd) => return review_file(cmd, &o),
        None => {}
    }
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
    loop {
        let mut term = view::init_terminal();
        let picked = list.run(&mut term);
        view::restore_terminal();
        let Some(pick) = picked? else { return Ok(()) };
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
        let res = review(opts, Some(n), pick.pr).and_then(|(_, res)| res);
        progress(None);
        std::env::set_current_dir(root)?; // a worktree review leaves us in the worktree
        match res {
            Err(e) => list.msg = format!("PR #{n}: {e:#}"),
            // `Space g` → merge, from the review.
            Ok(true) => list.msg = merge::merge(n, view::in_zellij()).unwrap_or_else(|e| format!("not merged: {e:#}")),
            Ok(false) => {}
        }
    }
}

/// Review replacement `id` in dv, in its worktree: its diff against its base, and in the top
/// section, what it replaces and Claude's drafts.
fn review_replacement(id: u64, root: &Path) -> anyhow::Result<()> {
    let r = replace::load(id)?;
    let dir = worktree_dir_named(root, &replace::name(id))?;
    anyhow::ensure!(dir.exists(), "its worktree {} is gone (A A on the PR drafts it again)", dir.display());
    ensure_worktree_of(&dir, root)?;
    std::env::set_current_dir(&dir)?;
    let base = diff::merge_base(Some(&format!("origin/{}", r.base)))?;
    let mut app = view::App::new(base, None)?;
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
    let follow = pr.as_ref().filter(|_| o.base.is_none()).map(|p| format!("origin/{}", p.base));
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
                progress(Some(&format!("fetching origin/{}", p.base)));
                _ = diff::git(&["fetch", "-q", "origin", &p.base]);
                format!("origin/{}", p.base)
            }
        });
    }
    diff::merge_base(base.as_deref())
}

/// `dv review-info` / `dv review-check`, for the current checkout (or PR `-p N`), no checkout.
fn review_file(cmd: &str, o: &Opts) -> anyhow::Result<()> {
    let pr = gh::pr(o.pr).ok();
    if cmd == "review-comments" {
        return review_comments(pr.as_ref());
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
    println!("ok: {} comment(s), {} warning(s)", r.comments, r.warnings.len());
    Ok(())
}

/// `dv review-comments`: the PR's review threads on GitHub as JSON, for Claude to read (and
/// answer, with `"about": <id>` on a comment of its own). Read only.
fn review_comments(pr: Option<&gh::Pr>) -> anyhow::Result<()> {
    let p = pr.context("no PR here (dv review-comments -p N for PR N)")?;
    let review = gh::parse_threads(&gh::threads_json(p.number)?);
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
    let (theirs, owner) = gh::head_branch(n).unwrap_or_default();
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
    diff::git(&["fetch", "-q", "origin", &format!("refs/pull/{n}/head")]).ok()?;
    let pr = diff::git(&["rev-parse", "FETCH_HEAD"]).ok()?.trim().to_owned();
    let behind = head == pr || diff::git(&["merge-base", "--is-ancestor", &head, &pr]).is_ok();
    if behind || last_checkout(n).as_deref() == Some(head.as_str()) {
        return None;
    }
    let ahead = diff::git(&["rev-list", "--count", &format!("{pr}..HEAD")]).unwrap_or_default();
    let ahead = ahead.trim();
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
        && let Some(other) = worktree_on_branch(&p.head_branch, &p.head_oid).filter(|d| *d != dir)
    {
        std::env::set_current_dir(&other)?;
        let note = format!("#{n}'s branch {} is checked out in {}: opened there", p.head_branch, other.display());
        return Ok((other, Some(note)));
    }
    let existed = dir.exists();
    if existed {
        ensure_ours(&dir)?;
    } else {
        progress(Some(&format!("creating the worktree {}", dir.display())));
        diff::git(&["worktree", "add", "--detach", &dir.to_string_lossy()])?;
    }
    std::env::set_current_dir(&dir)?;
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

/// Only ever touch a dir that's a worktree of this repo.
/// The worktree of this repo that has `branch` checked out, if one does and it's the PR's
/// (its history and the PR head's `oid` one line: either contains the other).
fn worktree_on_branch(branch: &str, oid: &str) -> Option<PathBuf> {
    let list = diff::git(&["worktree", "list", "--porcelain"]).ok()?;
    let dir = checked_out_at(&list, branch)?;
    let d = dir.to_string_lossy().into_owned();
    let git = |args: &[&str]| diff::git(&[&["-C", d.as_str()], args].concat()).is_ok();
    let ours = !oid.is_empty() && (git(&["merge-base", "--is-ancestor", oid, "HEAD"]) || git(&["merge-base", "--is-ancestor", "HEAD", oid]));
    ours.then_some(dir)
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
