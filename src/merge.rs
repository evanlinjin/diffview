//! Merging a PR with Bitcoin Core's `github-merge.py` (`Space g` in its review, `M M` in `dv
//! list`). The script is built into dv (`vendor/github-merge/`, pinned to an upstream commit)
//! and written to dv's data directory to run, so nothing is downloaded to merge; `git config
//! dv.mergescript <path>` runs your own copy instead. It's interactive (you look the merge over,
//! sign it, push it), so it gets a terminal: a zellij pane of its own, or this one.
use anyhow::{Context, Result};
use std::path::PathBuf;

const SCRIPT: &str = include_str!("../vendor/github-merge/github-merge.py");
/// Where it's from, and its SHA-256 there (a test checks it's still that).
pub const UPSTREAM: &str = "bitcoin-core/bitcoin-maintainer-tools@1519ef4883f89d7eb0c03134b2ae2878eff87988";
#[cfg(test)]
const SHA256: &str = "37e585e5c9bf354028e43b04b3833b3cea2010c15fdb01e48c4aa32bc86736fe";

/// The script to run: yours (`dv.mergescript`), else dv's copy, written out again whenever it
/// isn't exactly the one built in.
fn script() -> Result<PathBuf> {
    if let Ok(p) = crate::diff::git(&["config", "--get", "dv.mergescript"]) {
        let p = PathBuf::from(p.trim());
        anyhow::ensure!(p.is_file(), "dv.mergescript is {}, which isn't a file", p.display());
        return Ok(p);
    }
    let p = crate::claude::data_home()?.join(format!("github-merge-{}.py", &UPSTREAM[UPSTREAM.len() - 40..][..12]));
    if std::fs::read_to_string(&p).ok().as_deref() != Some(SCRIPT) {
        std::fs::create_dir_all(p.parent().unwrap_or(&p))?;
        std::fs::write(&p, SCRIPT)?;
    }
    Ok(p)
}

/// PR `n`'s worktree out of the way (the script checks out the base branch, which a worktree
/// would be in the way of), only if nothing in it would be lost: no local changes, no commits
/// that aren't on the PR.
fn clear_worktree(n: u64) -> Result<Option<String>> {
    // Its one checkout (a published replacement's worktree, its runs' too): nothing in it lost.
    // Removed only if it's its own (`<repo>-N`); a replacement's goes with the draft (`D D`).
    let (dir, own) = (crate::pr_checkout(n)?, crate::worktree_dir(n)?.1);
    if !dir.exists() {
        return Ok(None);
    }
    nothing_to_lose(&dir, n)?;
    match dir == own {
        true => crate::remove_worktree(n, false).map(Some),
        false => Ok(None),
    }
}

/// Worktree `dir` of PR `n` has nothing that isn't on GitHub: no local changes (untracked files
/// count), and no commits beyond the PR's head there.
pub(crate) fn nothing_to_lose(dir: &std::path::Path, n: u64) -> Result<()> {
    let d = dir.to_string_lossy().into_owned();
    anyhow::ensure!(crate::diff::git(&["-C", &d, "status", "--porcelain"])?.trim().is_empty(), "{d} has local changes: commit and push them (or drop them) first");
    // (Work only here, as updating the checkout has it: not the PR's own old head after a
    // force-push, nor its base's commits.)
    if let Some(why) = crate::local_work(n, dir) {
        anyhow::bail!("{d}: {why}");
    }
    Ok(())
}

/// The command that merges PR `n`: the script, in the main checkout, told for this run only
/// (git's `GIT_CONFIG_*`) what you haven't set: the repo (`githubmerge.repository`), and a token
/// for GitHub's API (`user.ghtoken`, from `gh auth token`: without one, GitHub allows 60 requests
/// an hour). The token is fetched by the command as it starts, not written into it (a zellij
/// pane shows its command).
fn command(n: u64) -> Result<(PathBuf, Vec<String>)> {
    let (main, _) = crate::worktree_dir(n)?;
    let unset = |k: &str| crate::diff::git(&["-C", &main.to_string_lossy(), "config", "--get", k]).is_err();
    let mut config: Vec<(&str, String)> = vec![];
    if unset("githubmerge.repository") {
        let repo = crate::gh::repo_name().context("which GitHub repo this is (for githubmerge.repository)")?;
        config.push(("githubmerge.repository", format!("'{}'", repo.replace('\'', ""))));
    }
    if unset("user.ghtoken")
        && let Some(gh) = on_path("gh")
    {
        config.push(("user.ghtoken", format!("\"$('{}' auth token 2>/dev/null)\"", gh.display())));
    }
    let set = config_prefix(&config);
    // dv's PATH: a zellij pane gets zellij's environment, not dv's (python3 may not be on it).
    let mut argv = vec!["sh".to_owned(), "-c".to_owned(), format!("{set}exec \"$@\""), "sh".to_owned()];
    argv.extend(["env".to_owned(), format!("PATH={}", std::env::var("PATH").unwrap_or_default())]);
    argv.extend(["python3".into(), script()?.to_string_lossy().into_owned(), n.to_string()]);
    Ok((main, argv))
}

/// The merge lock of the repo checked out at `main` (in its git dir): held by a script's run,
/// start to end (`run_locked`); a run that dies lets go.
fn lock(main: &std::path::Path) -> Result<std::fs::File> {
    let p = PathBuf::from(crate::common_dir(main)?).join("dv-merge.lock");
    Ok(std::fs::File::options().create(true).truncate(false).write(true).open(p)?)
}

/// `dv merge-run N` (a merge's pane, or a merge here): the script for PR `n`, in the main
/// checkout, holding the merge lock till it's done.
pub fn run_locked(n: u64) -> Result<std::process::ExitStatus> {
    // (The lock first: what the command looks up takes a moment.)
    let (main, _) = crate::worktree_dir(n)?;
    let held = lock(&main)?;
    held.try_lock().map_err(|_| anyhow::anyhow!("github-merge.py is merging in {} already: once it's done", main.display()))?;
    let (main, argv) = command(n)?;
    std::process::Command::new(&argv[0]).args(&argv[1..]).current_dir(&main).status().context("running python3")
}

/// Shell that gives a command git config `pairs` (key, shell word for its value), for its run.
fn config_prefix(pairs: &[(&str, String)]) -> String {
    let mut set = format!("export GIT_CONFIG_COUNT={}; ", pairs.len());
    for (i, (k, v)) in pairs.iter().enumerate() {
        set += &format!("export GIT_CONFIG_KEY_{i}={k} GIT_CONFIG_VALUE_{i}={v}; ");
    }
    set
}

/// Where program `name` is on dv's PATH.
fn on_path(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?).map(|d| d.join(name)).find(|p| p.is_file())
}

/// The branches github-merge.py makes for PR `n` (and leaves behind: base and head).
fn script_branches(n: u64) -> Vec<String> {
    ["base", "head", "merge", "local-merge"].iter().map(|b| format!("pull/{n}/{b}")).collect()
}

/// `argv` for a pane that closes when it's done: at once if it succeeded (deleting the script's
/// branches for PR `n`), else once you've read what it said (Enter; the branches stay, to look
/// into).
fn pane_argv(argv: &[String], n: u64) -> Vec<String> {
    // Done (either way), the focus back on dv's pane: zellij would pick one beside this.
    // (Not from tests: they'd move your focus, run from a zellij pane.)
    let back = std::env::var("ZELLIJ_PANE_ID").ok().filter(|_| !cfg!(test)).map_or(String::new(), |id| format!("zellij action focus-pane-id terminal_{id} 2>/dev/null; "));
    let wait = format!(
        r#""$@"; st=$?; if [ $st -eq 0 ]; then git branch -q -D {} 2>/dev/null; else printf '\ngithub-merge.py stopped (exit %s). Enter closes this pane.\n' "$st"; read _; fi; {back}exit $st"#,
        script_branches(n).join(" ")
    );
    ["sh".to_owned(), "-c".to_owned(), wait, "sh".to_owned()].into_iter().chain(argv.iter().cloned()).collect()
}

/// Why PR `n` can't be merged now, as far as what's here goes: Claude at work on it, from any
/// dv (a pane open on it too: its checkout's switched or removed under it), or in the main
/// checkout, or changes there (the script's checkouts switch it). Asked before a review ends
/// for it, as here.
pub fn waits(n: u64) -> Option<String> {
    // Its work under its own key, or its draft's (a pane opened before it was published keeps it).
    // (Its draft's worktree too: a pane there no marker says.)
    let draft = || {
        let r = crate::replace::all().into_iter().find(|r| r.published == Some(n))?;
        let dir = crate::worktree_dir_named(std::path::Path::new("."), &crate::replace::name(r.id)).ok();
        r.at_work().or_else(|| crate::claude::at_work_in(&dir?, None))
    };
    if let Some(w) = crate::claude::at_work(&crate::claude::pr_key(n), None, None).or_else(draft) {
        return Some(format!("Claude is {w} on #{n}: merge it once it's done (a pane: closed)"));
    }
    let main = match crate::worktree_dir(n) {
        Ok((main, _)) => main,
        Err(e) => return Some(format!("{e:#}")),
    };
    // Nor while the script's at a merge there already (from any dv: its sign-off, its push
    // prompt): a second would switch the checkout away, and drop its merge.
    if lock(&main).is_ok_and(|l| l.try_lock().is_err()) {
        return Some(format!("github-merge.py is merging in {}: once it's done", main.display()));
    }
    // Nor under Claude at work there, on whatever PR (its edits would land on the base branch).
    if let Some(w) = crate::claude::at_work_in(&main, None) {
        return Some(format!("Claude is {w} in {}, which the merge script's checkouts would switch under it: merge once it's done", main.display()));
    }
    match crate::diff::git(&["-C", &main.to_string_lossy(), "status", "--porcelain", "--untracked-files=no"]) {
        Ok(d) if d.trim().is_empty() => None,
        Ok(_) => Some(format!("{} has local changes, which the merge script's checkouts would trip over: commit or stash them first", main.display())),
        Err(e) => Some(format!("{e:#}")),
    }
}

/// Merge PR `n`: its worktree removed first (if it's safe to), then the script, in a zellij pane
/// beside this one when `pane` (it closes when the script succeeds; failing, it waits for Enter,
/// for you to read why), else in this terminal (dv waits for Enter after it). Returns what to
/// tell you.
pub fn merge(n: u64, pane: bool) -> Result<String> {
    if let Some(why) = waits(n) {
        anyhow::bail!(why);
    }
    // As GitHub has it now (the list may be older): merged or closed, there's nothing to merge,
    // and the script would only fail on the merge ref GitHub has dropped.
    if let Some(state) = crate::gh::pr_state(n)?.filter(|s| s != "OPEN") {
        anyhow::bail!("#{n} is {} already, on GitHub (R refreshes the list)", state.to_lowercase());
    }
    // GitHub would refuse the push: say so now, before any of it (the script fails at its end).
    // Can't tell (a GitHub hiccup)? The script goes ahead, as it would have.
    if let Ok(Some(why)) = crate::gh::unsigned_blocking(n) {
        anyhow::bail!("not merged: {why}");
    }
    let removed = clear_worktree(n)?;
    let (main, _) = crate::worktree_dir(n)?;
    let first = removed.map_or(String::new(), |r| format!("{r}; "));
    // dv's own run of it (`merge-run`), with dv's PATH (a zellij pane has zellij's).
    let path = format!("PATH={}", std::env::var("PATH").unwrap_or_default());
    let argv: Vec<String> = vec!["env".into(), path, crate::self_exe().to_string_lossy().into_owned(), "merge-run".into(), n.to_string()];
    if pane {
        let st = std::process::Command::new("zellij")
            .args(["run", "--close-on-exit", "--name", &format!("github-merge.py #{n}"), "--cwd"])
            .arg(&main)
            .arg("--")
            .args(pane_argv(&argv, n))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()?;
        anyhow::ensure!(st.success(), "zellij run failed ({st})");
        return Ok(format!("{first}merging #{n} with github-merge.py in its own pane"));
    }
    eprintln!("\n{first}github-merge.py {n}, in {}:\n", main.display());
    let st = run_locked(n)?;
    eprint!("\ngithub-merge.py is done ({st}). Enter goes back to dv… ");
    let mut line = String::new();
    _ = std::io::stdin().read_line(&mut line);
    Ok(match st.success() {
        true => {
            // Its branches for the PR, done with (they'd pile up: a pair a merge).
            let mut args = vec!["-C".to_owned(), main.to_string_lossy().into_owned(), "branch".into(), "-q".into(), "-D".into()];
            args.extend(script_branches(n));
            _ = crate::diff::git(&args.iter().map(String::as_str).collect::<Vec<_>>());
            format!("{first}github-merge.py finished merging #{n}")
        }
        false => format!("{first}github-merge.py stopped ({st}) on #{n}"),
    })
}

#[test]
fn built_in_script_is_upstreams() {
    // Its SHA-256, by whatever this system has for it.
    let p = std::env::temp_dir().join(format!("dv-merge-{}.py", std::process::id()));
    std::fs::write(&p, SCRIPT).unwrap();
    let sum = |cmd: &str, args: &[&str]| std::process::Command::new(cmd).args(args).arg(&p).output().ok().filter(|o| o.status.success());
    let out = sum("sha256sum", &[]).or_else(|| sum("shasum", &["-a", "256"])).expect("sha256sum or shasum");
    _ = std::fs::remove_file(&p);
    assert!(String::from_utf8_lossy(&out.stdout).starts_with(SHA256), "vendor/github-merge/github-merge.py isn't {UPSTREAM}'s");
}

#[test]
fn worktree_only_goes_when_nothing_would_be_lost() {
    let repo = crate::diff::scratch_repo("merge-wt");
    let root = repo.with_extension("d");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let (origin, wt) = (root.join("origin.git"), root.join("repo-7"));
    let git = |dir: &std::path::Path, args: &[&str]| crate::diff::git_in(dir, args).unwrap();
    git(&repo, &["checkout", "-q", "-b", "master"]);
    git(&repo, &["commit", "-q", "--allow-empty", "-m", "pr head"]);
    git(&root, &["init", "-q", "--bare", &origin.to_string_lossy()]);
    git(&repo, &["remote", "add", "origin", &origin.to_string_lossy()]);
    git(&repo, &["push", "-q", "origin", "HEAD:refs/pull/7/head", "HEAD:refs/heads/master"]);
    git(&repo, &["worktree", "add", "-q", "--detach", &wt.to_string_lossy()]);
    // At the PR's head, clean: fine.
    nothing_to_lose(&wt, 7).unwrap();
    // A change, even an untracked file: not.
    std::fs::write(wt.join("x"), "x").unwrap();
    assert!(format!("{:#}", nothing_to_lose(&wt, 7).unwrap_err()).contains("local changes"));
    // Committed, so not on the PR: not either.
    git(&wt, &["add", "x"]);
    git(&wt, &["commit", "-q", "-m", "mine"]);
    assert!(format!("{:#}", nothing_to_lose(&wt, 7).unwrap_err()).contains("1 commit here isn't on PR #7"));
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&repo);
}

#[test]
fn merge_pane_closes_on_success() {
    // The wrapped command's status comes through; a failure waits for Enter before closing.
    let run = |cmd: &[&str], input: &str| {
        let argv: Vec<String> = cmd.iter().map(|s| s.to_string()).collect();
        let a = pane_argv(&argv, 7);
        let mut c = std::process::Command::new(&a[0]);
        c.args(&a[1..]).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped());
        let mut child = c.spawn().unwrap();
        use std::io::Write;
        child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
        let out = child.wait_with_output().unwrap();
        (out.status.code(), String::from_utf8_lossy(&out.stdout).into_owned())
    };
    assert_eq!(run(&["true"], ""), (Some(0), String::new()));
    // Succeeding, it deletes the script's branches for the PR (in the checkout it runs in).
    let dir = crate::diff::scratch_repo("merge-branches");
    let git = |args: &[&str]| crate::diff::git_in(&dir, args).unwrap();
    git(&["checkout", "-q", "-b", "master"]);
    git(&["commit", "-q", "--allow-empty", "-m", "x"]);
    git(&["branch", "pull/7/base"]);
    git(&["branch", "pull/7/head"]);
    git(&["branch", "pull/8/head"]);
    let a = pane_argv(&["true".to_owned()], 7);
    assert!(std::process::Command::new(&a[0]).args(&a[1..]).current_dir(&dir).status().unwrap().success());
    assert_eq!(git(&["branch", "--list", "pull/*"]).trim(), "pull/8/head"); // another PR's stay
    _ = std::fs::remove_dir_all(&dir);
    let (code, said) = run(&["sh", "-c", "exit 3"], "\n");
    assert_eq!(code, Some(3));
    assert!(said.contains("stopped (exit 3). Enter closes this pane."), "{said}");
}

#[test]
fn merge_command_authenticates() {
    // The token's fetched by the command as it runs (not in it), into user.ghtoken for the
    // script; the script then sees both config values git gives it.
    let dir = std::env::temp_dir().join(format!("dv-merge-config-{}", std::process::id()));
    _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let fake = dir.join("gh");
    std::fs::write(&fake, "#!/bin/sh\necho tok123\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let set = config_prefix(&[("githubmerge.repository", "'o/r'".into()), ("user.ghtoken", format!("\"$('{}' auth token 2>/dev/null)\"", fake.display()))]);
    let out = std::process::Command::new("sh")
        .args(["-c", &format!("{set}exec \"$@\""), "sh", "sh", "-c", "git config --get user.ghtoken; git config --get githubmerge.repository"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout), "tok123\no/r\n");
    assert!(!set.contains("tok123")); // the command has the way to it, not the token
    _ = std::fs::remove_dir_all(&dir);
}
