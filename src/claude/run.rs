//! How dv runs Claude Code in the background: the prompts, what it may do, the command.
use std::path::Path;

/// What `claude -p` may do in the background: edit files (the review, and code when asked),
/// and run only the commands the skill needs.
pub(crate) const ALLOWED: &[&str] = &[
    "Bash(dv review-info:*)",
    "Bash(dv review-check:*)",
    "Bash(dv review-comments:*)",
    "Bash(git diff:*)",
    "Bash(git show:*)",
    "Bash(git log:*)",
    "Bash(git rev-parse:*)",
    // The PR's description and conversation, to propose a new description from.
    "Bash(gh pr view:*)",
    // Why CI fails, to fix it.
    "Bash(gh pr checks:*)",
    "Bash(gh run view:*)",
    // To check a change it made still builds and passes.
    "Bash(cargo check:*)",
    "Bash(cargo build:*)",
    "Bash(cargo test:*)",
    "Bash(cargo clippy:*)",
    // As CI runs them (bdk's: nightly rustfmt; clippy, on the default toolchain here).
    // (Checking only: a `cargo fmt` would rewrap files the PR never touched.)
    "Bash(cargo fmt --check:*)",
    "Bash(cargo fmt -- --check:*)",
    "Bash(cargo fmt --all --check:*)",
    "Bash(cargo fmt --all -- --check:*)",
    "Bash(cargo +nightly fmt --check:*)",
    "Bash(cargo +nightly fmt -- --check:*)",
    "Bash(cargo +nightly fmt --all --check:*)",
    "Bash(cargo +nightly fmt --all -- --check:*)",
    "Bash(cargo +nightly clippy:*)",
    "Bash(cargo +stable clippy:*)",
    // Any other toolchain (a pinned one, as CI's MSRV job): through dv, which runs only these.
    "Bash(dv cargo:*)",
    // A checkout of its own to try things in (`dv scratch`), rather than ask.
    "Bash(dv scratch:*)",
];

/// What a background run may do beyond `ALLOWED`: when fixing PR `n`'s CI, rerun its failed
/// jobs (that PR's: no other; once a head, see `fix_checks_prompt`); when answering you or
/// making changes, commit them (`COMMIT_TOOLS`).
pub(crate) fn tools_for(fixing_ci: Option<u64>, commits: bool) -> Vec<String> {
    let rerun = fixing_ci.map(|n| format!("Bash(dv rerun-failed -p {n})"));
    let commit = COMMIT_TOOLS.iter().filter(|_| commits).map(|t| t.to_string());
    rerun.into_iter().chain(commit).collect()
}

/// Committing a change you asked for, or amending it into the commit it belongs in (dv's own
/// amend: a backup branch, merges kept); local, pushed only by you.
pub(crate) const COMMIT_TOOLS: &[&str] = &["Bash(git status:*)", "Bash(git add:*)", "Bash(git commit:*)", "Bash(dv amend:*)"];

/// What drafting a replacement needs on top: reading the PRs it replaces, and committing (to
/// its own local branch; dv never pushes it without you).
pub(crate) const REPLACE_TOOLS: &[&str] = &[
    "Bash(dv replace-info:*)",
    "Bash(gh pr view:*)",
    "Bash(gh pr diff:*)",
    "Bash(gh issue view:*)",
    "Bash(git status:*)",
    "Bash(git add:*)",
    "Bash(git commit:*)",
];

/// Asking Claude to review `what` ("PR #12").
pub(crate) fn review_prompt(what: &str) -> String {
    format!("Use the dv-review skill to review {what} for dv. If there's already a review, update it.")
}

/// Claude's prompt for updating its review of `what` after new commits on top of `old`.
pub(crate) fn update_prompt(what: &str, old: &str) -> String {
    format!(
        "Use the dv-review skill to update your dv review of {what}: review only what changed since you reviewed {old} \
         (`git diff {old}`), and keep the comments that still apply."
    )
}

/// Claude's prompt for answering your replies in its review of `what` (`theirs`: someone
/// else's PR, so suggestions where you didn't say).
pub(crate) fn answer_prompt(what: &str, theirs: bool) -> String {
    let base = format!(
        "Use the dv-review skill to answer my replies (and the comments I left for you) in the dv review of {what}. \
         Where I said what I want (\"want\"), do exactly that (see \"What the user wants\")."
    );
    match theirs {
        true => format!(
            "{base} It's someone else's PR: where I didn't say, and ask for a change, suggest it rather than edit the code \
             (see \"Suggest the changes\")."
        ),
        false => base,
    }
}

/// Claude's prompt for fixing PR `n`'s failing CI.
pub(crate) fn fix_checks_prompt(n: u64) -> String {
    let note = crate::claude::ci_note_path(n).map(|p| p.display().to_string()).unwrap_or_else(|_| "ci/pr-N.json in dv's directory".into());
    format!(
        "CI fails on PR #{n}. See which checks fail (`gh pr checks`) and why (`gh run view <run id> --log-failed`), \
         then fix the cause in the code here, and check the fix (build, tests). Don't commit or push: the user \
         reviews and pushes it. If it isn't this PR's fault (a flaky test, CI itself, something already broken on \
         its base branch), change nothing here; instead write {note} as JSON: \
         {{\"head\": \"<git rev-parse HEAD>\", \"why\": \"<one line: what fails and why it isn't this PR>\", \
         \"task\": \"<what a separate PR fixing it should do, a few lines; empty if nothing in the repo would fix \
         it>\", \"rerun\": false}}: the user can have a PR drafted from it. If the failure looks transient (a \
         network error, a timeout, a runner problem, a test known to be flaky), also rerun the failed jobs once \
         (`dv rerun-failed -p {n}`: the failed jobs only, of every failed run on its head) and set \"rerun\": true; but not if {note} \
         already has \"rerun\": true for this head (CI was rerun once already: it isn't transient), and then \
         keep \"rerun\": true in the note you write. First run \
         `dv review-info`: if the notes it lists exist, read them and follow them."
    )
}

/// `prompt`, with the user's own instructions (if any) after it.
pub(crate) fn with_instructions(prompt: &str, extra: &str) -> String {
    match extra.trim() {
        "" => prompt.to_owned(),
        e => format!("{prompt}\n\nThe user adds:\n{e}"),
    }
}

/// Where `dv scratch` puts Claude's checkouts (made, so Claude Code can be given it).
pub(crate) fn scratch_dir() -> anyhow::Result<std::path::PathBuf> {
    let d = crate::claude::dv_dir()?.join("scratch");
    std::fs::create_dir_all(&d)?;
    Ok(d)
}

/// `--add-dir` for dv's directory (the review files and notes): it's outside the checkout
/// Claude runs in, which is all Claude Code lets it touch otherwise.
pub(crate) fn claude_dirs() -> Vec<std::ffi::OsString> {
    match crate::claude::dv_dir() {
        Ok(dv) => vec!["--add-dir".into(), dv.into_os_string()],
        Err(_) => vec![],
    }
}

/// What a reading-only run may use beyond its own reading tools (Read, Grep, Glob): looking,
/// never changing.
const READ_ONLY: &[&str] = &["Bash(git diff:*)", "Bash(git show:*)", "Bash(git log:*)", "Bash(git rev-parse:*)", "Bash(gh pr view:*)"];

/// What Claude is run for: each can have its own model (`model_args`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Work {
    Review,
    Answer,
    Change,
    Describe,
    FixCi,
    /// A comment closing a PR or an issue.
    Close,
    Commit,
    Amend,
    Rebase,
    Replace,
    Triage,
    Rank,
    Interpret,
    Group,
}

impl Work {
    /// Its name in `git config dv.model.<name>` (the README lists them).
    pub(crate) fn key(self) -> &'static str {
        match self {
            Work::Review => "review",
            Work::Answer => "answer",
            Work::Change => "change",
            Work::Describe => "describe",
            Work::FixCi => "fix-ci",
            Work::Close => "close",
            Work::Commit => "commit",
            Work::Amend => "amend",
            Work::Rebase => "rebase",
            Work::Replace => "replace",
            Work::Triage => "triage",
            Work::Rank => "rank",
            Work::Interpret => "interpret",
            Work::Group => "group",
        }
    }
}

/// `--model` for `work`: `git config dv.model.<work>`, else `dv.model` (no work: Claude's pane,
/// for anything), else none: Claude Code's own default.
pub(crate) fn model_args(work: Option<Work>, dir: &Path) -> Vec<String> {
    let get = |k: &str| crate::diff::git_in(dir, &["config", "--get", k]).ok().map(|v| v.trim().to_owned()).filter(|v| !v.is_empty());
    work.and_then(|w| get(&format!("dv.model.{}", w.key()))).or_else(|| get("dv.model")).map_or(vec![], |m| vec!["--model".into(), m])
}

#[test]
fn a_model_per_task() {
    let dir = crate::diff::scratch_repo("model-args");
    let set = |k: &str, v: &str| crate::diff::git_in(&dir, &["config", k, v]).unwrap();
    set("dv.model", "opus");
    set("dv.model.rank", "haiku");
    assert_eq!((model_args(Some(Work::Rank), &dir), model_args(Some(Work::Review), &dir)), (vec!["--model".to_owned(), "haiku".into()], vec!["--model".to_owned(), "opus".into()]));
    assert_eq!(model_args(None, &dir), ["--model", "opus"]);
    // Each named in the README, as it says.
    let readme = include_str!("../../README.md");
    use Work::*;
    for w in [Review, Answer, Change, Describe, FixCi, Close, Commit, Amend, Rebase, Replace, Triage, Rank, Interpret, Group] {
        assert!(readme.contains(&format!("`{}`", w.key())), "{w:?}");
    }
    std::fs::remove_dir_all(dir).unwrap();
}

/// `claude -p prompt` in `dir` that only reads: what it says is its answer. `dontAsk`: whatever
/// isn't allowed is refused (left unsaid, the user's own default mode would rule, and `auto`
/// lets it edit). `more`: commands on top of `READ_ONLY`. No session. `work`: its model's.
pub(crate) fn claude_reader(work: Work, prompt: &str, dir: &Path, more: &[&str]) -> std::process::Command {
    let mut cmd = std::process::Command::new("claude");
    cmd.env("PATH", path_for_claude()).args(["-p", prompt, "--output-format", "json", "--permission-mode", "dontAsk"]).args(model_args(Some(work), dir));
    cmd.arg("--allowedTools").args(READ_ONLY).args(more).current_dir(dir);
    cmd
}

/// The repo's notes for Claude (`notes_path`), as a reading-only run's prompt starts with them
/// (it can't reach dv's directory to read them itself); empty if there are none.
pub(crate) fn notes_preamble() -> String {
    let notes = crate::claude::notes_path().ok().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
    match notes.trim() {
        "" => String::new(),
        n => format!("Your notes on this repo, from reviewing it (follow them):\n{n}\n\n"),
    }
}

/// The first JSON value of kind `object`/`array` in Claude's `answer` (words around it, a
/// fence, braces in its strings: what parses whole from a `{`/`[` on is it).
pub(crate) fn json_in(answer: &str, open: char) -> Option<serde_json::Value> {
    answer.match_indices(open).find_map(|(i, _)| serde_json::Deserializer::from_str(&answer[i..]).into_iter::<serde_json::Value>().next()?.ok())
}

/// `claude -p prompt` in `dir` as dv runs it in the background: its result as JSON on stdout,
/// edits allowed, in the session `session` names (`--resume <id>`, `--session-id <id>`, …),
/// with dv's directory reachable, and only the commands the skill needs.
/// `more`: tools allowed on top of the usual ones; `access`: the one directory outside `dir`
/// Claude may write to (`None`: all of dv's), besides its scratch checkouts (`dv scratch`).
/// `work`: its model's (`model_args`).
pub(crate) fn claude_command(work: Work, prompt: &str, dir: &Path, session: &[String], more: &[&str], access: Option<&Path>) -> std::process::Command {
    let mut cmd = std::process::Command::new("claude");
    // No editor to wait on (`git rebase --continue`, `git commit` without -m).
    cmd.env("PATH", path_for_claude()).env("GIT_EDITOR", "true")
        .args(["-p", prompt, "--output-format", "json", "--permission-mode", "acceptEdits"])
        .args(model_args(Some(work), dir))
        .args(session);
    match access {
        Some(d) => _ = cmd.arg("--add-dir").arg(d).args(scratch_dir().ok().map(|s| [std::ffi::OsString::from("--add-dir"), s.into_os_string()]).into_iter().flatten()),
        None => _ = cmd.args(claude_dirs()),
    }
    // Last: it takes every argument after it.
    cmd.arg("--allowedTools").args(ALLOWED).args(more).current_dir(dir);
    cmd
}

/// PATH for Claude, with this dv's directory first: the skill runs `dv review-*`, which must
/// be found (dv may have been started by its full path, or from a shell whose PATH a zellij
/// pane doesn't get) and be this version.
pub(crate) fn path_for_claude() -> std::ffi::OsString {
    let own = crate::self_exe().parent().map(Path::to_path_buf).filter(|d| !d.as_os_str().is_empty());
    let rest = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect::<Vec<_>>()).unwrap_or_default();
    std::env::join_paths(own.into_iter().chain(rest)).unwrap_or_default()
}

/// `XDG_DATA_HOME=…` as dv has it (set, or its default): where its review files are, whatever
/// the pane's environment says.
pub(crate) fn data_env() -> Option<std::ffi::OsString> {
    let mut v = std::ffi::OsString::from("XDG_DATA_HOME=");
    v.push(crate::claude::data_home().ok()?.parent()?);
    Some(v)
}

/// Live: on someone else's PR, a comment saying `"want": "change"` gets the code changed (not
/// a suggestion). Runs `claude` with the installed skill and `dv`.
/// `cargo test live_claude_change_on_their_pr -- --ignored --nocapture`
#[test]
#[ignore]
fn live_claude_change_on_their_pr() {
    let dir = crate::diff::scratch_repo("live-change");
    let data = std::env::temp_dir().join(format!("dv-live-change-data-{}", std::process::id()));
    let git = |args: &[&str]| crate::diff::git_in(&dir, args).unwrap();
    std::fs::write(dir.join("count.py"), "def total(xs):\n    return 0\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "base"]);
    git(&["checkout", "-q", "-b", "feat"]);
    std::fs::write(dir.join("count.py"), "def total(xs):\n    x = 0\n    for v in xs:\n        x += v\n    return x\n").unwrap();
    git(&["commit", "-q", "-am", "total: sum them"]);
    // dv's directory for this run: on each command (not the test process's, shared).
    let info = std::process::Command::new("dv").arg("review-info").current_dir(&dir).env("XDG_DATA_HOME", &data).output().unwrap();
    let info = String::from_utf8_lossy(&info.stdout);
    let file = info.lines().find_map(|l| l.strip_prefix("file: ")).expect("review-info's file").trim().to_owned();
    std::fs::create_dir_all(Path::new(&file).parent().unwrap()).unwrap();
    let review = serde_json::json!({ "comments": [{
        "path": "count.py", "line": 2, "side": "new", "author": "user", "code": ["    x = 0"],
        "body": "Rename `x` to `acc`, here and below.", "created": crate::claude::now(), "want": "change",
    }]});
    std::fs::write(&file, review.to_string()).unwrap();
    // (As dv grants the repo it runs in: here the test is elsewhere, so this run's directory.)
    let out = claude_command(Work::Answer, &answer_prompt("the current branch", true), &dir, &[], &[], Some(&data.join("dv"))).env("XDG_DATA_HOME", &data).stdin(std::process::Stdio::null()).output().unwrap();
    let said: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
    println!("claude: {}\nrefused: {}", said["result"].as_str().unwrap_or(&String::from_utf8_lossy(&out.stderr)), said["permission_denials"]);
    let code = std::fs::read_to_string(dir.join("count.py")).unwrap();
    println!("count.py:\n{code}");
    let after: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    println!("review: {after:#}");
    assert!(code.contains("acc = 0") && !code.contains(" x "), "the code wasn't changed");
    assert!(!after.to_string().contains("```suggestion"), "suggested instead");
    _ = std::fs::remove_dir_all(&dir);
    _ = std::fs::remove_dir_all(&data);
}

#[test]
fn json_in_an_answer() {
    let v = json_in("Here {it is}: ```json\n{\"a\": \"x } y\", \"b\": [1]}\n``` (a note})", '{').unwrap();
    assert_eq!((v["a"].as_str(), v["b"][0].as_u64()), (Some("x } y"), Some(1)));
    assert!(json_in("no object", '{').is_none());
}
