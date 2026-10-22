//! Amending what isn't committed into the PR's commits it belongs in (`Space g` → amend): git
//! blame places a change on lines a commit of the PR wrote; Claude is asked about the rest (new
//! lines, lines from two commits). Nothing changes until you say so in the picker.
use super::*;
use std::path::Path;

/// A change not committed yet (a hunk of `git diff HEAD`), and the PR's commit it goes in.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Hunk {
    pub(super) path: String,
    /// Its file's header in the diff (`diff --git` … `+++`), to apply it on its own.
    head: String,
    /// Its lines (`-` / `+`), after the `@@` line.
    body: String,
    /// What it changes in HEAD's version: (start, count; 0: it only adds, after `start`).
    old: (u32, u32),
    new: (u32, u32),
    /// The PR's commit it goes in (full sha), and who said so: "blame", or Claude's reason.
    pub(super) to: Option<String>,
    pub(super) why: String,
}

impl Hunk {
    /// `path:12-14` (new side), for the picker.
    pub(super) fn place(&self) -> String {
        match self.new.1 {
            0 | 1 => format!("{}:{}", self.path, self.new.0),
            n => format!("{}:{}-{}", self.path, self.new.0, self.new.0 + n - 1),
        }
    }

    /// The hunk as it applies once the ones in `done` (of this file, above it) are in HEAD,
    /// after `before` (this patch's, of this file, above it). `git apply --unidiff-zero` goes
    /// by both starts, so both are worked out: the new side's from the old's.
    fn patch(&self, done: &[&Hunk], before: &[&Hunk]) -> String {
        let grows = |hs: &[&Hunk]| -> i64 {
            hs.iter().filter(|d| d.head == self.head && d.old.0 + d.old.1 <= self.old.0).map(|d| d.new.1 as i64 - d.old.1 as i64).sum()
        };
        let old = (self.old.0 as i64 + grows(done)).max(0);
        // -U0: a pure addition's old start is the line before it; a pure removal's new start too.
        let new = old + grows(before) + (self.old.1 == 0) as i64 - (self.new.1 == 0) as i64;
        format!("@@ -{old},{} +{},{} @@\n{}", self.old.1, new.max(0), self.new.1, self.body)
    }

    /// The hunk as the diff has it, to match Claude's answers by.
    fn text(&self) -> String {
        self.patch(&[], &[])
    }
}

/// The changes, placed or not, and the PR's commits (sha, subject), oldest first.
type Plan = (Vec<Hunk>, Vec<(String, String)>);

use diff::git_in;

/// `git diff HEAD -U0`'s hunks (binary files have none: they stay).
fn parse(text: &str) -> Vec<Hunk> {
    let range = |s: &str| -> (u32, u32) {
        let mut p = s[1..].split(',');
        let a = p.next().and_then(|n| n.parse().ok()).unwrap_or(0);
        (a, p.next().map_or(1, |n| n.parse().unwrap_or(0)))
    };
    let (mut out, mut head, mut path) = (Vec::<Hunk>::new(), String::new(), String::new());
    let mut in_head = false;
    // Lines as they are, endings kept (a CRLF file's `\r` is part of the change).
    for full in text.split_inclusive('\n') {
        let l = full.trim_end_matches('\n').trim_end_matches('\r');
        if l.starts_with("diff --git ") {
            (head, in_head, path) = (full.to_owned(), true, String::new());
        } else if in_head && !l.starts_with("@@") {
            head += full;
            // (Still quoted: a name with a quote or a newline in it. Left unplaced.)
            if let Some(p) = l.strip_prefix("+++ b/").or_else(|| l.strip_prefix("--- a/")) {
                path = p.trim_end_matches('\t').to_owned(); // git ends a name with a space in a tab
            }
        } else if let Some(rest) = l.strip_prefix("@@ ") {
            in_head = false;
            let mut parts = rest.split(' ');
            let (old, new) = (range(parts.next().unwrap_or("-0")), range(parts.next().unwrap_or("+0")));
            out.push(Hunk { path: path.clone(), head: head.clone(), body: String::new(), old, new, to: None, why: String::new() });
        } else if let Some(h) = out.last_mut() {
            h.body += full;
            if !full.ends_with('\n') {
                h.body.push('\n');
            }
        }
    }
    out
}

/// What isn't committed in `dir`, each hunk placed in the PR's commit (of those after its
/// merge-base with `base`) that wrote all the lines it changes, if one did; and those commits
/// (sha, subject), oldest first.
pub(super) fn plan(dir: &Path, base: &str, blame: bool) -> Result<Plan> {
    let mb = git_in(dir, &["merge-base", base, "HEAD"])?.trim().to_owned();
    // The branch's own (as the commits list has them): not what a merge brought in.
    let commits: Vec<(String, String)> = git_in(dir, &["log", "--first-parent", "--no-merges", "--reverse", "--format=%H%x09%s", &format!("{mb}..HEAD")])?
        .lines()
        .filter_map(|l| l.split_once('\t').map(|(a, b)| (a.to_owned(), b.to_owned())))
        .collect();
    // As bytes: a change that isn't UTF-8 text can't go through dv's patches unharmed.
    let out = std::process::Command::new("git")
        .args(["-C", &dir.to_string_lossy(), "-c", "core.quotePath=false", "diff", "HEAD", "-U0", "--no-color", "--no-ext-diff", "--no-renames", "--src-prefix=a/", "--dst-prefix=b/"])
        .output()?;
    anyhow::ensure!(out.status.success(), "git diff: {}", String::from_utf8_lossy(&out.stderr).trim());
    let text = String::from_utf8(out.stdout).map_err(|_| anyhow::anyhow!("a change isn't UTF-8 text: amend by hand (git commit --fixup)"))?;
    let mut hunks = parse(&text);
    if !blame {
        return Ok((hunks, commits));
    }
    // One blame a file, for the lines all its hunks change: who wrote each line.
    let mut paths: Vec<String> = hunks.iter().filter(|h| h.old.1 > 0 && !h.path.is_empty()).map(|h| h.path.clone()).collect();
    paths.dedup();
    for path in paths {
        // Porcelain: each line's commit and its line number now, whatever its old name was.
        let mut args = vec!["blame".to_owned(), "--porcelain".into()];
        for h in hunks.iter().filter(|h| h.path == path && h.old.1 > 0) {
            args.extend(["-L".to_owned(), format!("{},+{}", h.old.0, h.old.1)]);
        }
        args.extend(["HEAD".into(), "--".into(), path.clone()]);
        let Ok(out) = git_in(dir, &args.iter().map(String::as_str).collect::<Vec<_>>()) else { continue };
        // "<sha> <old line> <line now> [<lines>]" heads each line's entry.
        let by: HashMap<u32, &str> = out
            .lines()
            .filter(|l| !l.starts_with('\t'))
            .filter_map(|l| {
                let mut w = l.split(' ');
                let sha = w.next().filter(|s| s.len() >= 40 && s.bytes().all(|b| b.is_ascii_hexdigit()))?;
                w.next()?;
                Some((w.next()?.parse().ok()?, sha))
            })
            .collect();
        for h in hunks.iter_mut().filter(|h| h.path == path && h.old.1 > 0) {
            let mut who: Vec<&str> = (h.old.0..h.old.0 + h.old.1).filter_map(|n| by.get(&n).copied()).collect();
            who.dedup();
            if let [one] = who[..]
                && commits.iter().any(|c| c.0 == one)
            {
                (h.to, h.why) = (Some(one.to_owned()), "blame".into());
            }
        }
    }
    Ok((hunks, commits))
}

/// Where amends record what they made of commits: the repo's common git dir (every
/// worktree's, as the review files are every checkout's).
fn rewritten_file(dir: &Path) -> Option<std::path::PathBuf> {
    git_in(dir, &["rev-parse", "--path-format=absolute", "--git-common-dir"]).ok().map(|d| std::path::PathBuf::from(d.trim()).join("dv-rewritten"))
}

/// What amends here made of commits: each one's SHA and what it became. (Whole lines of whole
/// SHAs only: one being appended meanwhile may be read half written.)
pub(super) fn rewritten_in(dir: &Path) -> Vec<(String, String)> {
    let text = rewritten_file(dir).and_then(|f| std::fs::read_to_string(f).ok()).unwrap_or_default();
    let sha = |s: &str| matches!(s.len(), 40 | 64) && s.bytes().all(|b| b.is_ascii_hexdigit());
    text.split_inclusive('\n')
        .filter_map(|l| l.strip_suffix('\n')?.split_once(' ').filter(|(o, n)| sha(o) && sha(n)))
        .map(|(o, n)| (o.to_owned(), n.to_owned()))
        .collect()
}

/// Record what an amend made of commits, after what those before did. Appended, a line each in
/// one write: amends in two dvs at once both kept.
fn record_rewritten(dir: &Path, made: &[(String, String)]) {
    use std::io::Write;
    let Some(f) = rewritten_file(dir) else { return };
    let lines: String = made.iter().map(|(o, n)| format!("\n{o} {n}\n")).collect();
    _ = std::fs::OpenOptions::new().create(true).append(true).open(f).and_then(|mut f| f.write_all(lines.as_bytes()));
}

/// Commit `sha`'s change, as `git patch-id` names it: the same for the same change, whatever
/// its message or parent.
fn patch_id(dir: &Path, sha: &str) -> Option<String> {
    use std::io::Write;
    let patch = git_in(dir, &["show", "--format=", sha]).ok()?;
    let mut child = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["patch-id", "--stable"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .ok()?;
    child.stdin.take()?.write_all(patch.as_bytes()).ok()?;
    let out = child.wait_with_output().ok()?;
    String::from_utf8_lossy(&out.stdout).split_whitespace().next().map(str::to_owned)
}

/// What an amend dropped, said: ", dropped a commit (its changes all undone): "first"".
fn said_dropped(dropped: &[String], subject: impl Fn(&str) -> String) -> String {
    match dropped {
        [] => String::new(),
        d => format!(
            ", dropped {} (its changes all undone): {}",
            match d.len() { 1 => "a commit".to_owned(), k => format!("{k} commits") },
            d.iter().map(|t| format!("\"{}\"", subject(t))).collect::<Vec<_>>().join(", "),
        ),
    }
}

/// An amend's outcome: how many commits took changes, those that didn't (sha, git's word why),
/// and those dropped (sha).
type Amended = (usize, Vec<(String, String)>, Vec<String>);

/// Amend `hunks` (those with a commit to go in) into their commits: for each commit, newest
/// first, a fixup commit and `rebase --autosquash` from the merge-base with `base` (newest first:
/// the older ones' shas stay as they were). One that conflicts (a later commit changed those
/// lines) is undone and its changes stay uncommitted; the rest go in. HEAD is kept on `backup`
/// first. `rewords`: (a commit, its new message) each, with its changes or none. How many commits
/// were amended, those that weren't (with git's word why), and those dropped (the changes undid
/// all they did: nothing left of them); none amended: an error, all as it was.
pub(super) fn amend(dir: &Path, base: &str, hunks: &[Hunk], rewords: &[(&str, &str)], backup: &str) -> Result<Amended> {
    let mb = git_in(dir, &["merge-base", base, "HEAD"])?.trim().to_owned();
    let order = git_in(dir, &["rev-list", "--first-parent", &format!("{mb}..HEAD")])?; // newest first
    let mut targets: Vec<&str> = vec![];
    for t in hunks.iter().filter_map(|h| h.to.as_deref()).chain(rewords.iter().map(|r| r.0)) {
        if !targets.contains(&t) {
            targets.push(t);
        }
    }
    targets.sort_by_key(|t| order.lines().position(|l| l == *t));
    anyhow::ensure!(!targets.is_empty(), "nothing placed to amend");
    // (Its own rebase it aborts if it fails: not one stopped part way, yours or Claude's.)
    anyhow::ensure!(!super::rebase::rebasing(dir), "a rebase is stopped part way here: finish or abort it first");
    anyhow::ensure!(git_in(dir, &["ls-files", "-u", "--", ":/"])?.trim().is_empty(), "unmerged files here: resolve them first");
    // New files only marked (`git add -N`): added for real, as `--autostash` can't stash them,
    // and `write-tree` would leave them out (untracked, back as it was).
    let marked = git_in(dir, &["diff", "--name-only", "--no-relative", "--no-renames", "--diff-filter=A", "-z"])?;
    let marked: Vec<&str> = marked.split('\0').filter(|f| !f.is_empty()).collect();
    if !marked.is_empty() {
        git_in(dir, &[&["--literal-pathspecs", "add", "--"], marked.as_slice()].concat())?;
    }
    // What you staged, to put back if nothing goes in (and a conflicted file: refused here).
    let staged = git_in(dir, &["write-tree"])?.trim().to_owned();
    let orig = git_in(dir, &["rev-parse", "HEAD"])?.trim().to_owned();
    git_in(dir, &["branch", "-f", backup, "HEAD"])?;
    let gitdir = std::path::PathBuf::from(git_in(dir, &["rev-parse", "--absolute-git-dir"])?.trim());
    let (patch, index) = (gitdir.join("dv-amend.patch"), gitdir.join("dv-amend.index"));
    // The fixup commits are made in an index of dv's own, so what you staged needn't be unstaged
    // first (what's left may come back staged or not; new files stay added).
    let own = |args: &[&str]| -> Result<()> {
        let out = std::process::Command::new("git").arg("-C").arg(dir).args(args).env("GIT_INDEX_FILE", &index).output()?;
        // ("nothing to commit" is on stdout, last of a status report.)
        let said = match out.stderr.is_empty() {
            true => String::from_utf8_lossy(&out.stdout).lines().rfind(|l| !l.trim().is_empty()).unwrap_or("").trim().to_owned(),
            false => String::from_utf8_lossy(&out.stderr).trim().to_owned(),
        };
        anyhow::ensure!(out.status.success(), "git {}: {said}", args.join(" "));
        Ok(())
    };
    let mut done: Vec<&Hunk> = vec![];
    let (mut skipped, mut dropped) = (vec![], vec![]);
    for &t in &targets {
        let was = git_in(dir, &["rev-parse", "HEAD"])?.trim().to_owned();
        let mine: Vec<&Hunk> = hunks.iter().filter(|h| h.to.as_deref() == Some(t)).collect();
        let one = (|| -> Result<bool> {
            let mut text = String::new();
            for (i, h) in mine.iter().enumerate() {
                if i == 0 || mine[i - 1].head != h.head {
                    text += &h.head;
                }
                text += &h.patch(&done, &mine[..i]);
            }
            own(&["read-tree", "HEAD"])?;
            if !text.is_empty() {
                std::fs::write(&patch, text)?;
                own(&["apply", "--cached", "--unidiff-zero", &patch.to_string_lossy()])?;
            }
            // By its sha, not its subject: two commits may share one ("wip"). `amend!`: its
            // message replaced too, by what follows the first line.
            match rewords.iter().find(|r| r.0 == t) {
                Some((_, m)) => own(&["commit", "-q", "--no-verify", "--allow-empty", "-m", &format!("amend! {t}\n\n{m}")])?,
                None => own(&["commit", "-q", "--no-verify", "-m", &format!("fixup! {t}")])?,
            }
            let fix = git_in(dir, &["rev-parse", "HEAD"])?.trim().to_owned();
            // Merges kept (not flattened), and the backup branch left where it is. (No editor:
            // see `diff::git`.)
            let Err(e) = git_in(dir, &["-c", "rebase.updateRefs=false", "rebase", "-q", "-i", "--autosquash", "--autostash", "--rebase-merges", &mb]) else {
                return Ok(false);
            };
            // Stopped on our fixup with the commit left as its parent was (its changes undo all
            // it did: git won't make it empty): the commit goes, and the rebase goes on.
            let at = |r: &str| git_in(dir, &["rev-parse", "-q", "--verify", r]).map(|s| s.trim().to_owned()).unwrap_or_default();
            let emptied = at("REBASE_HEAD") == fix && git_in(dir, &["write-tree"]).is_ok_and(|w| w.trim() == at("HEAD^^{tree}"));
            anyhow::ensure!(emptied, "{e:#}");
            git_in(dir, &["reset", "-q", "--soft", "HEAD^"])?;
            git_in(dir, &["rebase", "--continue"])?;
            Ok(true)
        })();
        match one {
            Ok(gone) => {
                done.extend(mine);
                if gone {
                    dropped.push(t.to_owned());
                }
            }
            Err(e) => {
                _ = git_in(dir, &["rebase", "--abort"]);
                // This one's changes uncommitted again (staged: a mixed reset would untrack new
                // files a later commit's fixup still needs).
                _ = git_in(dir, &["reset", "-q", "--soft", &was]);
                let msg = format!("{e:#}");
                let why = match msg.contains("could not apply") {
                    true => "a later commit changes those lines".to_owned(),
                    false => msg.rsplit("error: ").next().unwrap_or(&msg).lines().next().unwrap_or("").trim().to_owned(),
                };
                skipped.push((t.to_owned(), why));
            }
        }
    }
    _ = std::fs::remove_file(&patch);
    _ = std::fs::remove_file(&index);
    // What became of each commit, kept beside the repo: any dv (a view, `dv amend` from Claude)
    // follows one by it, not by guessing (reworded and changed, it has nothing left the same).
    // Those not dropped kept their order: the branch's commits now, each in its old one's place.
    let now = git_in(dir, &["rev-list", "--first-parent", &format!("{mb}..HEAD")]).unwrap_or_default();
    let kept: Vec<&str> = order.lines().filter(|o| !dropped.iter().any(|d| d == o)).collect();
    let mut made: Vec<(String, String)> = vec![];
    if kept.len() == now.lines().count() {
        made.extend(kept.into_iter().zip(now.lines()).filter(|(o, n)| o != n).map(|(o, n)| (o.to_owned(), n.to_owned())));
    }
    if !made.is_empty() {
        record_rewritten(dir, &made);
    }
    if skipped.len() == targets.len() {
        _ = git_in(dir, &["reset", "-q", &orig]);
        _ = git_in(dir, &["read-tree", &staged]); // staged as it was
        if !marked.is_empty() {
            // (Those only marked, marked again.)
            _ = git_in(dir, &[&["--literal-pathspecs", "reset", "-q", "--"], marked.as_slice()].concat());
            _ = git_in(dir, &[&["--literal-pathspecs", "add", "-N", "--"], marked.as_slice()].concat());
        }
        _ = git_in(dir, &["branch", "-D", backup]);
        let why = skipped.first().map(|s| s.1.as_str()).unwrap_or("");
        anyhow::bail!("nothing amended: it's all as it was ({why})");
    }
    Ok((targets.len() - skipped.len(), skipped, dropped))
}

/// Everything uncommitted (`paths`: in those files) folded into the branch's commit `sha` (full;
/// one of `base`'s branch's own), HEAD kept on `backup` first. With a `message`, that's its
/// message now, and only the `paths` given are folded (none: just the message). Whether the
/// commit was dropped instead (the changes undid all it did).
pub(super) fn fold(dir: &Path, base: &str, sha: &str, paths: &[String], message: Option<&str>, backup: &str) -> Result<bool> {
    let (mut hunks, commits) = plan(dir, base, false)?;
    anyhow::ensure!(commits.iter().any(|c| c.0 == sha), "that isn't one of the branch's own commits (a merge?): amend into another");
    // (A name git still quotes: no knowing its file. It stays.)
    hunks.retain(|h| !h.path.is_empty() && (paths.is_empty() && message.is_none() || paths.contains(&h.path)));
    anyhow::ensure!(!hunks.is_empty() || message.is_some(), "nothing uncommitted to amend");
    hunks.iter_mut().for_each(|h| h.to = Some(sha.to_owned()));
    // (One commit: in, dropped, or an error.)
    amend(dir, base, &hunks, &message.map(|m| (sha, m)).into_iter().collect::<Vec<_>>(), backup).map(|(_, _, d)| !d.is_empty())
}

/// `dv amend <commit> [file…] [-F <message file>]` (for Claude: dv's amend, not a rebase by
/// hand): what isn't committed (in those files) folded into that commit of the branch, and/or
/// its message replaced, HEAD kept on a backup branch first.
pub(crate) fn fold_cli(args: &[String], message: Option<&str>, base: &str) -> Result<String> {
    let (rev, paths) = args.split_first().context("dv amend <commit> [file…] [-F <message file>]")?;
    let dir = std::env::current_dir()?;
    let sha = git_in(&dir, &["rev-parse", "--verify", &format!("{rev}^{{commit}}")])?.trim().to_owned();
    let short = git_in(&dir, &["rev-parse", "--short", "HEAD"])?.trim().to_owned();
    let backup = format!("dv-backup/amend-{short}");
    let what = match fold(&dir, base, &sha, paths, message, &backup)? {
        true => format!("dropped {} (its changes all undone)", &sha[..7]),
        false => format!("amended into {}", &sha[..7]),
    };
    Ok(format!("{what} (HEAD was {short}: {backup})"))
}


/// Claude's answers for what blame couldn't place, from the request file at `p` (written at
/// HEAD `head`): each hunk's commit, where it gave one (of `commits`).
fn claude_answers(p: &Path, head: &str, hunks: &mut [Hunk], commits: &[(String, String)]) {
    let Some(v) = std::fs::read_to_string(p).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()) else { return };
    if v["head"] != head {
        return; // asked about another HEAD: its answers may not fit
    }
    for a in v["hunks"].as_array().into_iter().flatten() {
        let (Some(c), Some(path), Some(body)) = (a["commit"].as_str(), a["path"].as_str(), a["diff"].as_str()) else { continue };
        let Some(sha) = commits.iter().find(|k| !c.is_empty() && k.0.starts_with(c)).map(|k| k.0.clone()) else { continue };
        // (Asked about all of them, its answer over blame's.)
        if let Some(h) = hunks.iter_mut().find(|h| h.path == path && h.text() == body) {
            let why = a["why"].as_str().unwrap_or("").trim();
            (h.to, h.why) = (Some(sha), format!("Claude: {}", if why.is_empty() { "fits there" } else { why }));
        }
    }
}

/// The new messages Claude gave commits in `p` (their changes there make the old ones wrong),
/// answered at HEAD `head`: (full sha, message), of `commits` only.
fn claude_messages(p: &Path, head: &str, commits: &[(String, String)]) -> Vec<(String, String)> {
    let Some(v) = std::fs::read_to_string(p).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()) else { return vec![] };
    if v["head"] != head {
        return vec![];
    }
    v["commits"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| {
            let m = c["message"].as_str()?.trim();
            let (sha, old) = commits.iter().find(|k| c["sha"].as_str().is_some_and(|s| !s.is_empty() && k.0.starts_with(s)))?;
            // Not a subject a later autosquash would take for its own; not the same subject
            // with no body (nothing to change).
            let squashy = ["fixup!", "amend!", "squash!"].iter().any(|p| m.starts_with(p));
            let same = m.trim() == old.trim();
            (!m.is_empty() && !squashy && !same).then(|| (sha.clone(), m.to_owned()))
        })
        .fold(vec![], |mut all: Vec<(String, String)>, m| {
            if !all.iter().any(|a| a.0 == m.0) {
                all.push(m); // (one each: the first)
            }
            all
        })
}

impl App {
    /// Where Claude answers about the changes blame couldn't place: beside the review file.
    fn amend_file(&self) -> Option<std::path::PathBuf> {
        let p = self.claude.path.as_ref()?;
        Some(p.parent()?.parent()?.join("amend").join(p.file_name()?))
    }

    /// Claude's new messages for the commits that take changes (those that'd no longer fit).
    fn amend_messages(&self, hunks: &[Hunk], commits: &[(String, String)]) -> Vec<(String, String)> {
        let head = git_in(&self.root, &["rev-parse", "HEAD"]).map(|h| h.trim().to_owned()).unwrap_or_default();
        let all = self.amend_file().map(|p| claude_messages(&p, &head, commits)).unwrap_or_default();
        all.into_iter().filter(|m| hunks.iter().any(|h| h.to.as_deref() == Some(m.0.as_str()))).collect()
    }

    fn amend_plan(&self) -> Result<Plan> {
        let (mut hunks, commits) = plan(&self.root, &self.full_base, true)?;
        if let Some(p) = self.amend_file() {
            let head = git_in(&self.root, &["rev-parse", "HEAD"])?.trim().to_owned();
            claude_answers(&p, &head, &mut hunks, &commits);
        }
        Ok((hunks, commits))
    }

    /// Commits rewritten (amended, rebased: by dv or not): the questions about them follow
    /// them, to the commit now with the same subject and author date (both kept by a rebase or
    /// an amend; the old commit's still here to ask). Once the commits change.
    pub(super) fn rekey_commits(&mut self) {
        let shas: Vec<String> = self.commits.iter().map(|c| c.sha.clone()).collect();
        if shas == self.rekeyed {
            return;
        }
        self.rekeyed = shas;
        let Some(p) = self.claude.path.clone() else { return };
        let mut lost: Vec<(String, String)> = self
            .threads
            .iter()
            .filter_map(|t| Some((t.conversation.clone(), super::commit_of(&t.conversation)?.to_owned())))
            .filter(|(_, sha)| !self.commits.iter().any(|c| &c.sha == sha))
            .collect();
        lost.sort();
        lost.dedup();
        if lost.is_empty() {
            return;
        }
        let rewritten = std::cell::OnceCell::new();
        let moves: Vec<(String, String)> = lost
            .into_iter()
            .filter_map(|(url, sha)| Some((url.clone(), url.replace(&sha, &self.now_as(&sha, &rewritten)?))))
            .collect();
        if !moves.is_empty() && crate::claude::rename_conversations(&p, &moves).is_ok() {
            let keep = self.cursor_key();
            self.load_claude();
            self.rebuild(keep);
        }
    }

    /// The commit `sha` of the list is now: itself, still here; else one dv's amends linked it to
    /// (`rewritten`, read the first time it's needed): what it became, or what it was (reset back
    /// to the backup), any steps on; else (rebased, amended outside dv) what one of those is now,
    /// by `guess`. Dropped (its changes all undone): none.
    pub(super) fn now_as(&self, sha: &str, rewritten: &std::cell::OnceCell<Vec<(String, String)>>) -> Option<String> {
        let here = |s: &String| self.commits.iter().any(|c| &c.sha == s);
        let mut linked = vec![sha.to_owned()];
        if !here(&linked[0]) {
            let links = rewritten.get_or_init(|| rewritten_in(&self.root));
            let mut i = 0;
            while let Some(s) = linked.get(i).cloned() {
                for (o, n) in links {
                    let other = if *o == s { n } else if *n == s { o } else { continue };
                    if !linked.contains(other) {
                        linked.push(other.clone());
                    }
                }
                i += 1;
            }
        }
        linked.iter().find(|s| here(s)).cloned().or_else(|| linked.iter().find_map(|s| self.guess(s)))
    }

    /// The one commit of the list with `sha`'s subject and author date (both kept by a rebase or
    /// an amend), or, reworded, its date, author and change. (A dropped one's: none, its change
    /// nowhere.)
    fn guess(&self, sha: &str) -> Option<String> {
        let was = git_in(&self.root, &["log", "-1", "--format=%at%x09%ae%x09%s", sha]).ok()?;
        let mut f = was.trim_end_matches('\n').splitn(3, '\t');
        let (at, email, subject) = (f.next()?, f.next()?, f.next()?);
        let one = |v: Vec<&String>| (v.len() == 1).then(|| v[0].clone());
        let dated = || self.commits.iter().filter(|c| c.at == at);
        let by_subject = one(dated().filter(|c| c.subject.trim() == subject.trim()).map(|c| &c.sha).collect());
        // (Reworded: its change the same too, its patch's id; not another made the same second.)
        let email_of = |sha: &str| git_in(&self.root, &["log", "-1", "--format=%ae", sha]).is_ok_and(|e| e.trim() == email);
        by_subject.or_else(|| {
            let patch = patch_id(&self.root, sha)?;
            one(dated().filter(|c| email_of(&c.sha) && patch_id(&self.root, &c.sha).as_ref() == Some(&patch)).map(|c| &c.sha).collect())
        })
    }

    /// `Space g` → amend: where each change goes, and doing it (or asking Claude about the rest).
    pub(super) fn amend_menu(&mut self) {
        let (hunks, commits) = match self.amend_plan() {
            Ok(p) => p,
            Err(e) => return self.msg = format!("can't amend: {e:#}"),
        };
        if hunks.is_empty() {
            return self.msg = "nothing uncommitted to amend (new files stay untracked: commit them)".into();
        }
        if self.claude_busy() {
            return self.msg = App::busy_edit("amend");
        }
        let subject = |sha: &str| commits.iter().find(|c| c.0 == sha).map_or(String::new(), |c| format!("{} {}", &sha[..7.min(sha.len())], c.1));
        let placed = hunks.iter().filter(|h| h.to.is_some()).count();
        let into = hunks.iter().filter_map(|h| h.to.as_deref()).collect::<HashSet<_>>().len();
        let mut items = vec![];
        if placed > 0 {
            let rest = if placed < hunks.len() { format!("; {} stay uncommitted", hunks.len() - placed) } else { String::new() };
            items.push(Item::new(format!("Amend: {placed} change(s) into {into} commit(s) (a backup branch first{rest})"), Target::Amend));
        }
        let unplaced = hunks.len() - placed;
        if unplaced > 0 && !commits.is_empty() {
            items.push(Item::new(format!("Ask Claude where the other {unplaced} belong… (it answers here; nothing changes till you amend)"), Target::AmendAsk));
        }
        // What Claude would reword, said before it's done.
        for (sha, m) in self.amend_messages(&hunks, &commits) {
            items.push(Item::new(format!("✎ {}  → new message: {}", subject(&sha), m.lines().next().unwrap_or("")), Target::Note("Where each change goes")));
        }
        for h in &hunks {
            let label = match &h.to {
                Some(t) => format!("{}  → {}{}", h.place(), subject(t), if h.why == "blame" { String::new() } else { format!("  ({})", h.why) }),
                None if h.old.1 == 0 => format!("{}  ✗ new lines: stays uncommitted", h.place()),
                None => format!("{}  ✗ no one commit of the PR wrote these lines: stays uncommitted", h.place()),
            };
            items.push(Item::new(label, Target::Note("Where each change goes")));
        }
        self.picker = Some(Picker::new(" amend into the PR's commits ", items));
    }

    /// HEAD's short sha, and the branch HEAD is kept on before it's rewritten.
    pub(super) fn backup_name(&self) -> Result<(String, String)> {
        let short = git_in(&self.root, &["rev-parse", "--short", "HEAD"])?.trim().to_owned();
        let who = self.pr.as_ref().map_or("branch".to_owned(), |p| format!("pr-{}", p.number));
        Ok((short.clone(), format!("dv-backup/{who}-{short}")))
    }

    /// The amend picker's Enter: do it.
    pub(super) fn amend_now(&mut self) {
        self.msg = match self.try_amend() {
            Ok(m) => m,
            Err(e) => format!("{e:#}"),
        };
    }

    fn try_amend(&mut self) -> Result<String> {
        anyhow::ensure!(!self.claude_busy(), "{}", App::busy_edit("amend"));
        let (hunks, commits) = self.amend_plan()?;
        let (short, backup) = self.backup_name()?;
        // Claude's new messages, for the commits that take changes (what they say then fits).
        let messages = self.amend_messages(&hunks, &commits);
        let messages: Vec<(&str, &str)> = messages.iter().map(|m| (m.0.as_str(), m.1.as_str())).collect();
        let (n, skipped, dropped) = amend(&self.root, &self.full_base, &hunks, &messages, &backup)?;
        // (By subject: the sha's gone.)
        let gone = said_dropped(&dropped, |t| commits.iter().find(|c| c.0 == t).map_or("?".to_owned(), |c| c.1.clone()));
        // (Those that went in: not skipped, nor dropped.)
        let reworded = match messages.iter().filter(|m| !skipped.iter().any(|s| s.0 == m.0) && !dropped.iter().any(|d| d == m.0)).count() {
            0 => String::new(),
            k => format!(", {k} reworded by Claude"),
        };
        if let Some(p) = self.amend_file() {
            _ = std::fs::remove_file(p); // answered for a HEAD that's gone
        }
        let left = match skipped.as_slice() {
            [] => String::new(),
            // By its subject: its sha may be gone (an older one amended rewrote it).
            [(t, why), ..] => format!(
                "; {} left uncommitted (\"{}\": {why}): f on a later commit folds them there",
                match skipped.len() { 1 => "a commit's changes".to_owned(), k => format!("the changes for {k} commits") },
                commits.iter().find(|c| c.0 == *t).map_or("", |c| c.1.as_str()),
            ),
        };
        Ok(self.and_reload(format!("amended into {n} commit(s){reworded}{gone} (was {short}: {backup}){left}; {}", self.next_step(true))))
    }

    /// `f` on a commit in the list: everything uncommitted folded into that one (new lines too).
    pub(super) fn amend_into(&mut self, sha: &str) {
        let r = (|| -> Result<String> {
            anyhow::ensure!(!self.claude_busy(), "{}", App::busy_edit("amend"));
            let (short, backup) = self.backup_name()?;
            let what = match fold(&self.root, &self.full_base, sha, &[], None, &backup)? {
                true => format!("dropped {} (its changes all undone)", sha.chars().take(7).collect::<String>()),
                false => format!("amended into {}", sha.chars().take(7).collect::<String>()),
            };
            Ok(self.and_reload(format!("{what} (was {short}: {backup}); {}", self.next_step(true))))
        })();
        self.msg = r.unwrap_or_else(|e| format!("{e:#}"));
    }

    /// Ask Claude which of the PR's commits the changes blame couldn't place belong in (`all`:
    /// every change, blame's place a hint): it answers in a file beside the review; the amend
    /// picker shows its answers when it's done (`all`: dv amends them in then).
    pub(super) fn amend_ask(&mut self, all: bool) {
        let r = (|| -> Result<()> {
            let p = self.amend_file().context("no review file here")?;
            if let Some(why) = self.claude_blocked(None) {
                anyhow::bail!(why);
            }
            let (hunks, commits) = self.amend_plan()?;
            anyhow::ensure!(!hunks.is_empty(), "nothing uncommitted to amend (new files stay untracked: commit them)");
            let head = git_in(&self.root, &["rev-parse", "HEAD"])?.trim().to_owned();
            ask_request(&p, &head, &hunks, &commits, all)?;
            let prompt = ask_prompt(&p, &head, all);
            let forked = self.claude_pane().is_some();
            self.claude.job = Some(self.spawn_claude(&prompt, AMEND_LABEL, crate::claude::Work::Amend, forked, &[])?);
            self.amend_then = all;
            Ok(())
        })();
        self.msg = match r {
            Ok(()) if all => "Claude is placing every change in its commit (dv amends them in when it's done, a backup branch first)".into(),
            Ok(()) => "Claude is placing the rest of the changes (the amend picker opens with its answers)".into(),
            Err(e) => format!("{e:#}"),
        };
    }
}

/// Write the request Claude answers in `p`: the hunks blame couldn't place (`all`: every one,
/// with where blame would put it), at HEAD `head`.
fn ask_request(p: &Path, head: &str, hunks: &[Hunk], commits: &[(String, String)], all: bool) -> Result<()> {
    let ask: Vec<serde_json::Value> = hunks
        .iter()
        .filter(|h| all || h.to.is_none())
        .map(|h| serde_json::json!({ "path": h.path, "diff": h.text(), "blame": h.to.clone().unwrap_or_default(), "commit": "", "why": "" }))
        .collect();
    let list: Vec<serde_json::Value> = commits.iter().map(|c| serde_json::json!({ "sha": c.0, "subject": c.1 })).collect();
    std::fs::create_dir_all(p.parent().context("no dir")?)?;
    crate::claude::write(p, &serde_json::json!({ "head": head, "commits": list, "hunks": ask }))
}

/// Claude's prompt for answering the request in `p` (`all`: about every change).
fn ask_prompt(p: &Path, head: &str, all: bool) -> String {
    let which = match all {
        true => "Every change is asked about: \"blame\" says which commit last wrote the lines it changes, if one did: a \
                 hint, not the answer (a fix to one commit's line can belong with another commit's change)",
        false => "git blame placed some; these it couldn't (new lines, or lines from more than one commit)",
    };
    format!(
        "dv is about to amend the uncommitted changes in this checkout into the commits of the branch they belong \
         in. {which}: read {}. \
         For each entry in \"hunks\" (a change, as a diff of {}'s file), decide which of the commits in \"commits\" \
         it belongs in: the one whose change it completes or fixes (look at them: `git show <sha>`; `git diff` shows \
         all the uncommitted changes). Set its \"commit\" to that commit's full sha, and \"why\" to a few words why; \
         leave \"commit\" empty if none fits (it then stays uncommitted). A change that undoes what a commit \
         did (removes what it added) belongs in that commit; if it undoes all of it, dv drops the commit. Then, for each commit that gets changes: \
         if its message (`git log -1 --format=%B <sha>`) would no longer describe it with them in (they add \
         something it doesn't mention, or change what it says), set that entry's \"message\" in \"commits\" to the \
         whole new message (subject, a blank line, the body), in the repo's style (Conventional Commits, if it uses \
         them); otherwise leave it out. Edit only that file, with your Edit tool: don't change the code, and don't \
         commit.",
        p.display(),
        &head[..7.min(head.len())],
    )
}

/// The background run placing changes: when it's done, the amend picker opens again.
pub(super) const AMEND_LABEL: &str = "placing changes in commits";

#[test]
fn claude_rewords_commits_that_take_changes() {
    let p = std::env::temp_dir().join(format!("dv-amend-msg-{}.json", std::process::id()));
    std::fs::write(&p, r#"{"head": "h1", "commits": [{"sha": "aaaa1", "subject": "a", "message": "feat: a, and b\n\nBoth."},
        {"sha": "aaaa", "message": "again"}, {"sha": "bbbb2", "subject": "b", "message": "fixup! a"}, {"sha": "cccc", "message": "c"}]}"#).unwrap();
    // (One each; not one autosquash would take; not the same.)
    let commits = vec![("aaaa111".to_owned(), "a".to_owned()), ("bbbb222".to_owned(), "b".to_owned()), ("cccc333".to_owned(), "c".to_owned())];
    assert_eq!(claude_messages(&p, "h1", &commits), [("aaaa111".to_owned(), "feat: a, and b\n\nBoth.".to_owned())]);
    assert!(claude_messages(&p, "h2", &commits).is_empty()); // (asked at another HEAD)
    _ = std::fs::remove_file(p);
}

#[test]
fn amend_undoing_a_whole_commit() {
    let dir = crate::diff::scratch_repo("amend-undo");
    let git = |args: &[&str]| git_in(&dir, args).unwrap();
    // (Whatever the todo looks like: abbreviated, its lines without subjects.)
    git(&["config", "rebase.abbreviateCommands", "true"]);
    git(&["config", "rebase.instructionFormat", "%an"]);
    let write = |f: &str, s: &str| std::fs::write(dir.join(f), s).unwrap();
    write("a.rs", "base\n");
    write("b.rs", "b\n");
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "base"]);
    let base = git(&["rev-parse", "HEAD"]).trim().to_owned();
    write("a.rs", "base\nmine\n");
    git(&["commit", "-q", "-am", "first"]);
    write("b.rs", "b\nsecond\n");
    git(&["commit", "-q", "-am", "second"]);
    // What "first" did, undone: amended in, it's gone.
    write("a.rs", "base\n");
    let (hunks, _) = plan(&dir, &base, true).unwrap();
    let first = git(&["rev-parse", "HEAD~1"]).trim().to_owned();
    let was_second = git(&["rev-parse", "HEAD"]).trim().to_owned();
    let (n, skipped, dropped) = amend(&dir, &base, &hunks, &[], "dv-backup/undo").unwrap();
    assert_eq!((n, skipped.len(), dropped), (1, 0, vec![first.clone()]));
    assert_eq!(git(&["log", "--format=%s", &format!("{base}..HEAD")]).lines().collect::<Vec<_>>(), ["second"]);
    // Nothing of it left, here or uncommitted; the backup has it.
    assert_eq!((git(&["show", "HEAD:a.rs"]).as_str(), git(&["status", "--porcelain"]).as_str()), ("base\n", ""));
    assert_eq!(git(&["log", "-1", "--format=%s", "dv-backup/undo~1"]).trim(), "first");
    assert!(git_in(&dir, &["merge-base", "--is-ancestor", &base, "HEAD"]).is_ok()); // (the base as it was)
    // Recorded: "second" rewritten (followed); "first" gone, nothing.
    let second = git(&["rev-parse", "HEAD"]).trim().to_owned();
    assert_eq!(rewritten_in(&dir), [(was_second, second.clone())]);
    // `f` / `dv amend`: says it dropped it.
    write("b.rs", "b\n");
    assert!(fold(&dir, &base, &second, &[], None, "dv-backup/undo2").unwrap());
}

#[test]
fn amend_by_blame() {
    let dir = crate::diff::scratch_repo("amend");
    let git = |args: &[&str]| git_in(&dir, args).unwrap();
    let commit = |m: &str| git(&["commit", "-q", "-am", m]);
    let write = |f: &str, s: &str| std::fs::write(dir.join(f), s).unwrap();
    write("a.rs", "base\n");
    git(&["add", "."]);
    commit("base");
    let base = git(&["rev-parse", "HEAD"]).trim().to_owned();
    // Two commits of the PR, each writing some of a.rs.
    write("a.rs", "base\n// one\n// two\nkeep\n");
    commit("first");
    let first = git(&["rev-parse", "HEAD"]).trim().to_owned();
    write("a.rs", "base\n// one\n// two\nkeep\n// three\n// four\n");
    commit("second");
    let second = git(&["rev-parse", "HEAD"]).trim().to_owned();
    // Changes: two identical ones, one on each commit's lines, a line added to the first's
    // (shifting what follows), and new lines at the end.
    write("a.rs", "base\n/// one\n/// one, more\n// two\nadded\nkeep\n/// three\n// four\nnew\n");
    let (hunks, commits) = plan(&dir, &base, true).unwrap();
    assert_eq!(commits.iter().map(|c| c.1.as_str()).collect::<Vec<_>>(), ["first", "second"]);
    let to: Vec<_> = hunks.iter().map(|h| (h.place(), h.to.as_deref())).collect();
    assert_eq!(
        to,
        [
            ("a.rs:2-3".into(), Some(first.as_str())), // a line longer: what's after it moves
            ("a.rs:5".into(), None),                   // added: no commit wrote it
            ("a.rs:7".into(), Some(second.as_str())),
            ("a.rs:9".into(), None),
        ]
    );
    let (n, skipped, _) = amend(&dir, &base, &hunks, &[], "dv-backup/t").unwrap();
    assert_eq!((n, skipped.len()), (2, 0));
    // Each change in its commit; the history still two commits; the rest uncommitted.
    assert_eq!(git(&["log", "--format=%s", &format!("{base}..HEAD")]).lines().collect::<Vec<_>>(), ["second", "first"]);
    assert_eq!(git(&["show", "HEAD~1:a.rs"]), "base\n/// one\n/// one, more\n// two\nkeep\n");
    assert_eq!(git(&["show", "HEAD:a.rs"]), "base\n/// one\n/// one, more\n// two\nkeep\n/// three\n// four\n");
    assert_eq!(std::fs::read_to_string(dir.join("a.rs")).unwrap(), "base\n/// one\n/// one, more\n// two\nadded\nkeep\n/// three\n// four\nnew\n");
    assert_eq!(git(&["rev-parse", "dv-backup/t"]).trim(), second);
    // Claude's answer for one blame couldn't place: taken when it's for this HEAD.
    let (mut hunks, commits) = plan(&dir, &base, true).unwrap();
    let head = git(&["rev-parse", "HEAD"]).trim().to_owned();
    let first_now = commits[0].0.clone();
    let p = dir.join("answers.json");
    let answer = serde_json::json!({ "head": head, "hunks": [{ "path": "a.rs", "diff": hunks[0].text(), "commit": &first_now[..10], "why": "its comments" }] });
    std::fs::write(&p, answer.to_string()).unwrap();
    claude_answers(&p, &head, &mut hunks, &commits);
    assert_eq!((hunks[0].to.as_deref(), hunks[0].why.as_str()), (Some(first_now.as_str()), "Claude: its comments"));
    assert_eq!(hunks[1].to, None);
    // Asked about all of them: its answer over blame's (blame put hunks[0] elsewhere, say).
    hunks[0].to = Some(commits[1].0.clone());
    claude_answers(&p, &head, &mut hunks, &commits);
    assert_eq!(hunks[0].to.as_deref(), Some(first_now.as_str()));
    // A pure addition placed so (above another change, not placed) lands where it was.
    amend(&dir, &base, &hunks, &[], "dv-backup/t2").unwrap();
    assert_eq!(git(&["show", "HEAD~1:a.rs"]), "base\n/// one\n/// one, more\n// two\nadded\nkeep\n");
    assert_eq!(git(&["diff", "HEAD", "--stat"]).lines().count(), 2); // "new" still uncommitted
    // Staged changes, a staged new file too: amended all the same (dv's own index makes the fixups).
    git(&["add", "a.rs"]);
    std::fs::write(dir.join("n.rs"), "n\n").unwrap();
    git(&["add", "n.rs"]);
    let (mut hunks, commits) = plan(&dir, &base, true).unwrap();
    let last = commits.last().unwrap().0.clone();
    hunks.iter_mut().for_each(|h| h.to = Some(last.clone()));
    amend(&dir, &base, &hunks, &[], "dv-backup/t3").unwrap();
    assert_eq!(git(&["status", "--porcelain", "--untracked-files=no"]), "");
    // Into a commit a later one changes those lines of: that one's left uncommitted, the rest go in.
    let now = git(&["show", "HEAD:a.rs"]);
    write("a.rs", &now.replace("// two", "// TWO").replace("/// three", "/// THREE"));
    let (mut hunks, commits) = plan(&dir, &base, false).unwrap();
    let (first, second) = (commits[0].0.clone(), commits[1].0.clone());
    for h in hunks.iter_mut() {
        h.to = Some(if h.body.contains("TWO") { second.clone() } else { first.clone() }); // "/// three" is second's
    }
    let (n, skipped, _) = amend(&dir, &base, &hunks, &[], "dv-backup/t4").unwrap();
    assert_eq!((n, skipped.iter().map(|s| s.0.as_str()).collect::<Vec<_>>()), (1, vec![first.as_str()]));
    assert!(git(&["show", "HEAD:a.rs"]).contains("// TWO"));
    assert_eq!(git(&["diff", "HEAD"]).lines().filter(|l| l.starts_with('+') && !l.starts_with("+++")).collect::<Vec<_>>(), ["+/// THREE"]);
    // A message alone (`dv amend <sha> -F`): reworded, nothing else changed.
    let first = git(&["rev-list", "--reverse", &format!("{base}..HEAD")]).lines().next().unwrap().to_owned();
    amend(&dir, &base, &[], &[(&first, "first, reworded\n\nwith a body")], "dv-backup/t5").unwrap();
    assert_eq!(git(&["log", "--format=%s", &format!("{base}..HEAD")]).lines().collect::<Vec<_>>(), ["second", "first, reworded"]);
    assert_eq!(git(&["diff", "HEAD"]).lines().filter(|l| l.starts_with('+') && !l.starts_with("+++")).collect::<Vec<_>>(), ["+/// THREE"]);
    _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn amend_same_subjects_renames_crlf() {
    let dir = crate::diff::scratch_repo("amend2");
    let git = |args: &[&str]| git_in(&dir, args).unwrap();
    let write = |f: &str, s: &str| std::fs::write(dir.join(f), s).unwrap();
    git(&["config", "core.autocrlf", "false"]);
    write("x.rs", "a\r\nb\r\n");
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "base"]);
    let base = git(&["rev-parse", "HEAD"]).trim().to_owned();
    git(&["mv", "x.rs", "y.rs"]); // renamed: blame names files then
    git(&["commit", "-q", "-m", "wip"]);
    write("y.rs", "a\r\nb\r\nc\r\n");
    git(&["commit", "-q", "-am", "wip"]); // the same subject as the one before
    let second = git(&["rev-parse", "HEAD"]).trim().to_owned();
    write("y.rs", "a\r\nb\r\nC\r\n");
    let (hunks, _) = plan(&dir, &base, true).unwrap();
    assert_eq!(hunks.iter().map(|h| h.to.as_deref()).collect::<Vec<_>>(), [Some(second.as_str())]);
    amend(&dir, &base, &hunks, &[], "dv-backup/t").unwrap();
    // Into the second "wip", CRLF and all; nothing left over.
    assert_eq!(git(&["show", "HEAD:y.rs"]), "a\r\nb\r\nC\r\n");
    assert_eq!(git(&["show", "HEAD~1:y.rs"]), "a\r\nb\r\n");
    assert!(git(&["status", "--porcelain"]).trim().is_empty());
    _ = std::fs::remove_dir_all(&dir);
}

/// Live: Claude says which commit changes blame can't place belong in. Runs `claude`.
/// `cargo test live_claude_amend_ask -- --ignored --nocapture`
#[test]
#[ignore]
fn live_claude_amend_ask() {
    let dir = crate::diff::scratch_repo("live-amend");
    let git = |args: &[&str]| git_in(&dir, args).unwrap();
    let write = |f: &str, s: &str| std::fs::write(dir.join(f), s).unwrap();
    write("lib.rs", "pub fn one() -> u32 {\n    1\n}\n");
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "one"]);
    let base = git(&["rev-parse", "HEAD"]).trim().to_owned();
    write("lib.rs", "pub fn one() -> u32 {\n    1\n}\n\npub fn parse(s: &str) -> u32 {\n    s.parse().unwrap()\n}\n");
    git(&["commit", "-q", "-am", "add parse"]);
    let parse = git(&["rev-parse", "HEAD"]).trim().to_owned();
    write("README.md", "# lib\n");
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "readme"]);
    // New lines: a doc comment on parse (blame can't say whose; Claude should: "add parse"),
    // asked as for every change (Space g → amend it all).
    write("lib.rs", "pub fn one() -> u32 {\n    1\n}\n\n/// Parses `s`; panics if it isn't a number.\npub fn parse(s: &str) -> u32 {\n    s.parse().unwrap()\n}\n");
    let (hunks, commits) = plan(&dir, &base, true).unwrap();
    assert!(hunks.iter().all(|h| h.to.is_none()));
    let head = git(&["rev-parse", "HEAD"]).trim().to_owned();
    // Outside the checkout, as dv's own directory is (Claude gets to it as it does there).
    let data = std::env::temp_dir().join(format!("dv-live-amend-data-{}", std::process::id()));
    let p = data.join("amend/pr-1.json");
    ask_request(&p, &head, &hunks, &commits, true).unwrap();
    let out = crate::claude::claude_command(crate::claude::Work::Amend, &ask_prompt(&p, &head, true), &dir, &[], &[], Some(&data)).stdin(std::process::Stdio::null()).output().unwrap();
    let said: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
    println!("claude: {}\nrefused: {}", said["result"].as_str().unwrap_or(&String::from_utf8_lossy(&out.stderr)), said["permission_denials"]);
    let mut hunks = hunks;
    claude_answers(&p, &head, &mut hunks, &commits);
    println!("placed: {:?}", hunks.iter().map(|h| (h.place(), h.to.clone(), h.why.clone())).collect::<Vec<_>>());
    assert_eq!(hunks[0].to.as_deref(), Some(parse.as_str()));
    amend(&dir, &base, &hunks, &[], "dv-backup/live").unwrap();
    assert!(git(&["show", "HEAD~1:lib.rs"]).contains("/// Parses"));
    _ = std::fs::remove_dir_all(&dir);
    _ = std::fs::remove_dir_all(&data);
}
