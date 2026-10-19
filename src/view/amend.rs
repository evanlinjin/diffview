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

    /// The hunk as it applies once the ones in `done` (of this file, above it) are in HEAD.
    fn patch(&self, done: &[&Hunk]) -> String {
        let shift: i64 = done.iter().filter(|d| d.path == self.path && d.old.0 + d.old.1 <= self.old.0).map(|d| d.new.1 as i64 - d.old.1 as i64).sum();
        let start = (self.old.0 as i64 + shift).max(0);
        format!("@@ -{start},{} +{},{} @@\n{}", self.old.1, self.new.0, self.new.1, self.body)
    }
}

/// The changes, placed or not, and the PR's commits (sha, subject), oldest first.
type Plan = (Vec<Hunk>, Vec<(String, String)>);

fn git_in(dir: &Path, args: &[&str]) -> Result<String> {
    let d = dir.to_string_lossy();
    diff::git(&[&["-C", d.as_ref()], args].concat())
}

/// `git diff HEAD -U0`'s hunks (binary files have none: they stay).
fn parse(text: &str) -> Vec<Hunk> {
    let range = |s: &str| -> (u32, u32) {
        let mut p = s[1..].split(',');
        let a = p.next().and_then(|n| n.parse().ok()).unwrap_or(0);
        (a, p.next().map_or(1, |n| n.parse().unwrap_or(0)))
    };
    let (mut out, mut head, mut path) = (Vec::<Hunk>::new(), String::new(), String::new());
    let mut in_head = false;
    for l in text.lines() {
        if l.starts_with("diff --git ") {
            (head, in_head) = (format!("{l}\n"), true);
        } else if in_head && !l.starts_with("@@") {
            head += &format!("{l}\n");
            if let Some(p) = l.strip_prefix("+++ b/").or_else(|| l.strip_prefix("--- a/")) {
                path = p.to_owned();
            }
        } else if let Some(rest) = l.strip_prefix("@@ ") {
            in_head = false;
            let mut parts = rest.split(' ');
            let (old, new) = (range(parts.next().unwrap_or("-0")), range(parts.next().unwrap_or("+0")));
            out.push(Hunk { path: path.clone(), head: head.clone(), body: String::new(), old, new, to: None, why: String::new() });
        } else if let Some(h) = out.last_mut() {
            h.body += &format!("{l}\n");
        }
    }
    out
}

/// What isn't committed in `dir`, each hunk placed in the PR's commit (of those after its
/// merge-base with `base`) that wrote all the lines it changes, if one did; and those commits
/// (sha, subject), oldest first.
pub(super) fn plan(dir: &Path, base: &str) -> Result<Plan> {
    let mb = git_in(dir, &["merge-base", base, "HEAD"])?.trim().to_owned();
    let commits: Vec<(String, String)> = git_in(dir, &["log", "--reverse", "--format=%H%x09%s", &format!("{mb}..HEAD")])?
        .lines()
        .filter_map(|l| l.split_once('\t').map(|(a, b)| (a.to_owned(), b.to_owned())))
        .collect();
    let text = git_in(dir, &["diff", "HEAD", "-U0", "--no-color", "--no-ext-diff", "--no-renames", "--src-prefix=a/", "--dst-prefix=b/"])?;
    let mut hunks = parse(&text);
    for h in hunks.iter_mut().filter(|h| h.old.1 > 0) {
        let range = format!("{},+{}", h.old.0, h.old.1);
        let Ok(blame) = git_in(dir, &["blame", "-l", "-s", "-L", &range, "HEAD", "--", &h.path]) else { continue };
        let mut by: Vec<&str> = blame.lines().filter_map(|l| l.split_whitespace().next()).map(|s| s.trim_start_matches('^')).collect();
        by.dedup();
        if let [one] = by[..]
            && commits.iter().any(|c| c.0 == one)
        {
            (h.to, h.why) = (Some(one.to_owned()), "blame".into());
        }
    }
    Ok((hunks, commits))
}

/// Amend `hunks` (those with a commit to go in) into their commits: a fixup commit each, then
/// `rebase --autosquash` from the merge-base with `base`. HEAD is kept on `backup` first; what
/// isn't placed stays uncommitted. A conflict leaves everything as it was. How many commits
/// were amended.
pub(super) fn amend(dir: &Path, base: &str, hunks: &[Hunk], backup: &str) -> Result<usize> {
    let mb = git_in(dir, &["merge-base", base, "HEAD"])?.trim().to_owned();
    let orig = git_in(dir, &["rev-parse", "HEAD"])?.trim().to_owned();
    let mut targets: Vec<&str> = vec![];
    for t in hunks.iter().filter_map(|h| h.to.as_deref()) {
        if !targets.contains(&t) {
            targets.push(t);
        }
    }
    anyhow::ensure!(!targets.is_empty(), "nothing placed to amend");
    git_in(dir, &["branch", "-f", backup, "HEAD"])?;
    git_in(dir, &["reset", "-q"])?; // the index as HEAD: only what's applied goes in
    let patch = std::env::temp_dir().join(format!("dv-amend-{}.patch", std::process::id()));
    let mut done: Vec<&Hunk> = vec![];
    let fixups = (|| -> Result<()> {
        for &t in &targets {
            let mine: Vec<&Hunk> = hunks.iter().filter(|h| h.to.as_deref() == Some(t)).collect();
            let mut text = String::new();
            for (i, h) in mine.iter().enumerate() {
                if i == 0 || mine[i - 1].path != h.path {
                    text += &h.head;
                }
                text += &h.patch(&done);
            }
            std::fs::write(&patch, text)?;
            git_in(dir, &["apply", "--cached", "--unidiff-zero", &patch.to_string_lossy()])?;
            git_in(dir, &["commit", "-q", "--no-verify", &format!("--fixup={t}")])?;
            done.extend(mine);
        }
        git_in(dir, &["-c", "sequence.editor=true", "-c", "core.editor=true", "rebase", "-q", "-i", "--autosquash", "--autostash", &mb])?;
        Ok(())
    })();
    _ = std::fs::remove_file(&patch);
    if let Err(e) = fixups {
        _ = git_in(dir, &["rebase", "--abort"]);
        _ = git_in(dir, &["reset", "-q", &orig]); // back as it was: all of it uncommitted again
        return Err(e.context("nothing amended: it's all as it was"));
    }
    Ok(targets.len())
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
        if let Some(h) = hunks.iter_mut().find(|h| h.to.is_none() && h.path == path && h.patch(&[]) == body) {
            let why = a["why"].as_str().unwrap_or("").trim();
            (h.to, h.why) = (Some(sha), format!("Claude: {}", if why.is_empty() { "fits there" } else { why }));
        }
    }
}

impl App {
    /// Where Claude answers about the changes blame couldn't place: beside the review file.
    fn amend_file(&self) -> Option<std::path::PathBuf> {
        let p = self.claude.path.as_ref()?;
        Some(p.parent()?.parent()?.join("amend").join(p.file_name()?))
    }

    fn amend_plan(&self) -> Result<Plan> {
        let (mut hunks, commits) = plan(&self.root, &self.full_base)?;
        if let Some(p) = self.amend_file() {
            let head = git_in(&self.root, &["rev-parse", "HEAD"])?.trim().to_owned();
            claude_answers(&p, &head, &mut hunks, &commits);
        }
        Ok((hunks, commits))
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
        for h in &hunks {
            let label = match &h.to {
                Some(t) => format!("  {}  → {}{}", h.place(), subject(t), if h.why == "blame" { String::new() } else { format!("  ({})", h.why) }),
                None if h.old.1 == 0 => format!("  {}  ✗ new lines: stays uncommitted", h.place()),
                None => format!("  {}  ✗ no one commit of the PR wrote these lines: stays uncommitted", h.place()),
            };
            items.push(Item::new(label, Target::Nothing));
        }
        self.picker = Some(Picker { title: " amend into the PR's commits ", items, query: String::new(), sel: 0 });
    }

    /// The amend picker's Enter: do it.
    pub(super) fn amend_now(&mut self) {
        self.msg = match self.try_amend() {
            Ok(m) => m,
            Err(e) => format!("{e:#}"),
        };
    }

    fn try_amend(&mut self) -> Result<String> {
        let (hunks, _) = self.amend_plan()?;
        let short = git_in(&self.root, &["rev-parse", "--short", "HEAD"])?.trim().to_owned();
        let who = self.pr.as_ref().map_or("branch".to_owned(), |p| format!("pr-{}", p.number));
        let backup = format!("dv-backup/{who}-{short}");
        let n = amend(&self.root, &self.full_base, &hunks, &backup)?;
        if let Some(p) = self.amend_file() {
            _ = std::fs::remove_file(p); // answered for a HEAD that's gone
        }
        self.reload_tree();
        Ok(format!("amended into {n} commit(s) (was {short}: {backup}); Space g pushes (with a lease)"))
    }

    /// `a` on a commit in the list: everything uncommitted into that one (new lines too).
    pub(super) fn amend_into(&mut self, sha: &str) {
        let r = (|| -> Result<String> {
            let (mut hunks, _) = plan(&self.root, &self.full_base)?;
            anyhow::ensure!(!hunks.is_empty(), "nothing uncommitted to amend");
            hunks.iter_mut().for_each(|h| h.to = Some(sha.to_owned()));
            let short = git_in(&self.root, &["rev-parse", "--short", "HEAD"])?.trim().to_owned();
            let who = self.pr.as_ref().map_or("branch".to_owned(), |p| format!("pr-{}", p.number));
            let backup = format!("dv-backup/{who}-{short}");
            amend(&self.root, &self.full_base, &hunks, &backup)?;
            self.reload_tree();
            Ok(format!("amended into {} (was {short}: {backup}); Space g pushes (with a lease)", &sha[..7.min(sha.len())]))
        })();
        self.msg = r.unwrap_or_else(|e| format!("{e:#}"));
    }

    /// Ask Claude which of the PR's commits the changes blame couldn't place belong in: it
    /// answers in a file beside the review; the amend picker shows its answers when it's done.
    pub(super) fn amend_ask(&mut self) {
        let r = (|| -> Result<()> {
            let p = self.amend_file().context("no review file here")?;
            if let Some(why) = self.claude_blocked(false) {
                anyhow::bail!(why);
            }
            let (hunks, commits) = self.amend_plan()?;
            let head = git_in(&self.root, &["rev-parse", "HEAD"])?.trim().to_owned();
            let ask: Vec<serde_json::Value> = hunks
                .iter()
                .filter(|h| h.to.is_none())
                .map(|h| serde_json::json!({ "path": h.path, "diff": h.patch(&[]), "commit": "", "why": "" }))
                .collect();
            let list: Vec<serde_json::Value> = commits.iter().map(|c| serde_json::json!({ "sha": c.0, "subject": c.1 })).collect();
            std::fs::create_dir_all(p.parent().context("no dir")?)?;
            crate::claude::write(&p, &serde_json::json!({ "head": head, "commits": list, "hunks": ask }))?;
            let prompt = format!(
                "dv is about to amend the uncommitted changes in this checkout into the commits of the branch they belong \
                 in. git blame placed some; these it couldn't (new lines, or lines from more than one commit): read {}. \
                 For each entry in \"hunks\" (a change, as a diff of {}'s file), decide which of the commits in \"commits\" \
                 it belongs in: the one whose change it completes or fixes (look at them: `git show <sha>`; `git diff` shows \
                 all the uncommitted changes). Set its \"commit\" to that commit's full sha, and \"why\" to a few words why; \
                 leave \"commit\" empty if none fits (it then stays uncommitted). Edit only that file: don't change the \
                 code, and don't commit.",
                p.display(),
                &head[..7.min(head.len())],
            );
            let forked = self.claude_pane().is_some();
            self.claude.job = Some(self.spawn_claude(&prompt, AMEND_LABEL, forked)?);
            Ok(())
        })();
        self.msg = match r {
            Ok(()) => "Claude is placing the rest of the changes (the amend picker opens with its answers)".into(),
            Err(e) => format!("{e:#}"),
        };
    }
}

/// The background run placing changes: when it's done, the amend picker opens again.
pub(super) const AMEND_LABEL: &str = "placing changes in commits";

#[test]
fn amend_by_blame() {
    let dir = std::env::temp_dir().join(format!("dv-amend-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let git = |args: &[&str]| git_in(&dir, args).unwrap();
    let id = ["-c", "user.name=t", "-c", "user.email=t@t"];
    let commit = |m: &str| git(&[&id[..], &["commit", "-q", "-am", m]].concat());
    let write = |f: &str, s: &str| std::fs::write(dir.join(f), s).unwrap();
    git(&["init", "-q"]);
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
    let (hunks, commits) = plan(&dir, &base).unwrap();
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
    let n = amend(&dir, &base, &hunks, "dv-backup/t").unwrap();
    assert_eq!(n, 2);
    // Each change in its commit; the history still two commits; the rest uncommitted.
    assert_eq!(git(&["log", "--format=%s", &format!("{base}..HEAD")]).lines().collect::<Vec<_>>(), ["second", "first"]);
    assert_eq!(git(&["show", "HEAD~1:a.rs"]), "base\n/// one\n/// one, more\n// two\nkeep\n");
    assert_eq!(git(&["show", "HEAD:a.rs"]), "base\n/// one\n/// one, more\n// two\nkeep\n/// three\n// four\n");
    assert_eq!(std::fs::read_to_string(dir.join("a.rs")).unwrap(), "base\n/// one\n/// one, more\n// two\nadded\nkeep\n/// three\n// four\nnew\n");
    assert_eq!(git(&["rev-parse", "dv-backup/t"]).trim(), second);
    // Claude's answer for one blame couldn't place: taken when it's for this HEAD.
    let (mut hunks, commits) = plan(&dir, &base).unwrap();
    let head = git(&["rev-parse", "HEAD"]).trim().to_owned();
    let first_now = commits[0].0.clone();
    let p = dir.join("answers.json");
    let answer = serde_json::json!({ "head": head, "hunks": [{ "path": "a.rs", "diff": hunks[0].patch(&[]), "commit": &first_now[..10], "why": "its comments" }] });
    std::fs::write(&p, answer.to_string()).unwrap();
    claude_answers(&p, &head, &mut hunks, &commits);
    assert_eq!((hunks[0].to.as_deref(), hunks[0].why.as_str()), (Some(first_now.as_str()), "Claude: its comments"));
    assert_eq!(hunks[1].to, None);
    _ = std::fs::remove_dir_all(&dir);
}
