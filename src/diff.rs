//! Run `git diff` and parse its unified output.
use anyhow::{bail, Context, Result};
use std::process::Command;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    File,
    Hunk,
    Ctx,
    Add,
    Del,
    /// View-only rows (never parsed from git): a review comment's author line and body line.
    Thread,
    Comment,
}

#[derive(Clone, Debug)]
pub struct Line {
    pub kind: Kind,
    pub old: Option<u32>,
    pub new: Option<u32>,
    pub text: String,
}

#[derive(Debug)]
pub struct FileDiff {
    pub path: String,
    pub old_path: String,
    pub meta: String,
    /// What's shown: hunk headers and lines (`shown` over `full`).
    pub lines: Vec<Line>,
    /// Every line of the file's diff (whole-file context), without hunk headers.
    pub full: Vec<Line>,
    /// Git's hunk headers at default context: (new start, function-context suffix).
    pub headers: Vec<(u32, String)>,
}

impl FileDiff {
    /// Split parsed output into `full` lines and `headers`.
    pub fn prepare(&mut self) {
        self.headers = self.lines.iter().filter(|l| l.kind == Kind::Hunk).filter_map(|l| header(&l.text)).collect();
        self.full = self.lines.iter().filter(|l| l.kind != Kind::Hunk).cloned().collect();
    }
}

/// A shown hunk header: (new start, new count, function context).
pub fn hunk_label(h: &str) -> Option<(u32, u32, String)> {
    let new = h.split_whitespace().nth(2)?.get(1..)?;
    let (start, count) = new.split_once(',').unwrap_or((new, "1"));
    Some((start.parse().ok()?, count.parse().ok()?, h.splitn(3, "@@").nth(2).unwrap_or("").trim().to_owned()))
}

/// Char ranges within a line.
pub type Spans = Vec<(usize, usize)>;

/// Changed spans between a removed line and the added line it pairs with, as char ranges in
/// each. `None` when they're too different for word-level highlighting to help.
pub fn word_diff(a: &str, b: &str) -> Option<(Spans, Spans)> {
    let (ac, bc): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let (at, bt) = (tokens(&ac), tokens(&bc));
    if at.len() * bt.len() > 250_000 {
        return None; // ponytail: quadratic LCS; skip very long lines.
    }
    // LCS over tokens.
    let (n, m) = (at.len(), bt.len());
    let mut dp = vec![vec![0u16; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if tok(&ac, at[i]) == tok(&bc, bt[j]) { dp[i + 1][j + 1] + 1 } else { dp[i + 1][j].max(dp[i][j + 1]) };
        }
    }
    let (mut ka, mut kb) = (vec![false; n], vec![false; m]);
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if tok(&ac, at[i]) == tok(&bc, bt[j]) {
            (ka[i], kb[j]) = (true, true);
            (i, j) = (i + 1, j + 1);
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    // Too little in common: the whole line changed, so word emphasis would just be noise.
    let solid = |c: &[char], t: &[(usize, usize)], keep: &[bool], kept: bool| -> usize {
        t.iter().zip(keep).filter(|(_, k)| **k == kept).map(|(t, _)| c[t.0..t.1].iter().filter(|c| !c.is_whitespace()).count()).sum()
    };
    let same = solid(&ac, &at, &ka, true) + solid(&bc, &bt, &kb, true);
    let diff = solid(&ac, &at, &ka, false) + solid(&bc, &bt, &kb, false);
    if same == 0 || diff == 0 || same < diff {
        return None;
    }
    Some((spans(&ac, &at, &ka), spans(&bc, &bt, &kb)))
}

fn tok(c: &[char], t: (usize, usize)) -> &[char] {
    &c[t.0..t.1]
}

/// Word runs, whitespace runs, and single punctuation chars, as char ranges.
fn tokens(c: &[char]) -> Vec<(usize, usize)> {
    let class = |c: char| if c.is_alphanumeric() || c == '_' { 0 } else if c.is_whitespace() { 1 } else { 2 };
    let mut out = vec![];
    let mut i = 0;
    while i < c.len() {
        let k = class(c[i]);
        let mut j = i + 1;
        while k != 2 && j < c.len() && class(c[j]) == k {
            j += 1;
        }
        out.push((i, j));
        i = j;
    }
    out
}

/// Unmatched tokens as merged char ranges (whitespace between two changes joins them).
fn spans(c: &[char], t: &[(usize, usize)], keep: &[bool]) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = vec![];
    for (k, &(s, e)) in t.iter().enumerate() {
        let ws = c[s..e].iter().all(|c| c.is_whitespace());
        let changed = !keep[k] || (ws && k > 0 && !keep[k - 1] && t.get(k + 1).is_some() && !keep[k + 1]);
        if !changed {
            continue;
        }
        match out.last_mut() {
            Some(last) if last.1 == s => last.1 = e,
            _ => out.push((s, e)),
        }
    }
    out
}

fn header(h: &str) -> Option<(u32, String)> {
    let new = h.split_whitespace().nth(2)?.get(1..)?.split(',').next()?.parse().ok()?;
    Some((new, h.splitn(3, "@@").nth(2).unwrap_or("").to_owned()))
}

/// Load the diff with whole-file context (so any hunk can be expanded without asking git
/// again), plus git's function-context hunk headers from a normal `ctx`-line diff.
pub fn load(base: &str, ctx: u32) -> Result<Vec<FileDiff>> {
    let mut files = diff(base, 1_000_000, &[])?;
    let mut normal = diff(base, ctx, &[])?;
    for f in &mut files {
        f.prepare();
        if let Some(g) = normal.iter_mut().find(|g| g.path == f.path) {
            g.prepare();
            f.headers = std::mem::take(&mut g.headers);
        }
    }
    Ok(files)
}

/// Hunks over `full`: (first, last) changed-line index, grouped like git (changes at most
/// 2·ctx lines apart share a hunk). A hunk is identified by its first index.
pub fn hunks(full: &[Line], ctx: usize) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = vec![];
    for i in (0..full.len()).filter(|&i| matches!(full[i].kind, Kind::Add | Kind::Del)) {
        match out.last_mut() {
            Some((_, last)) if i - *last - 1 <= 2 * ctx => *last = i,
            _ => out.push((i, i)),
        }
    }
    out
}

/// Shown ranges of `full`: each hunk ± (ctx + its extra), merged where they meet, each with
/// the hunks it covers. `whole` shows the entire file.
pub fn windows(full: &[Line], ctx: usize, extra: impl Fn(usize) -> i64, whole: bool) -> Vec<(usize, usize, Vec<usize>)> {
    let hs = hunks(full, ctx);
    if full.is_empty() {
        return vec![];
    }
    if whole {
        return vec![(0, full.len() - 1, hs.iter().map(|h| h.0).collect())];
    }
    let mut out: Vec<(usize, usize, Vec<usize>)> = vec![];
    for (a, b) in hs {
        let x = (ctx as i64 + extra(a)).max(0) as usize;
        let (lo, hi) = (a.saturating_sub(x), (b + x).min(full.len() - 1));
        match out.last_mut() {
            Some(w) if lo <= w.1 + 1 => {
                w.1 = w.1.max(hi);
                w.2.push(a);
            }
            _ => out.push((lo, hi, vec![a])),
        }
    }
    out
}

/// The lines to show for `windows`, each range under a `@@` header (with git's function context).
pub fn shown(full: &[Line], windows: &[(usize, usize, Vec<usize>)], headers: &[(u32, String)]) -> Vec<Line> {
    let mut out = vec![];
    for (lo, hi, _) in windows {
        let part = &full[*lo..=*hi];
        let start = |f: fn(&Line) -> Option<u32>| part.iter().find_map(f).unwrap_or(0);
        let count = |f: fn(&Line) -> Option<u32>| part.iter().filter(|l| f(l).is_some()).count() as u32;
        let (os, oc, ns, nc) = (start(|l| l.old), count(|l| l.old), start(|l| l.new), count(|l| l.new));
        let suffix = headers.iter().find(|(n, _)| (ns..=ns + nc).contains(n)).map_or("", |h| h.1.as_str());
        let text = format!("@@ -{os},{oc} +{ns},{nc} @@{suffix}");
        out.push(Line { kind: Kind::Hunk, old: None, new: None, text });
        out.extend(part.iter().cloned());
    }
    out
}

pub fn git(args: &[&str]) -> Result<String> {
    let out = Command::new("git").args(args).output().context("running git")?;
    if !out.status.success() {
        bail!("git {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn merge_base(base: Option<&str>) -> Result<String> {
    let base = match base {
        Some(b) => b.to_owned(),
        None => git(&["rev-parse", "--abbrev-ref", "origin/HEAD"])
            .map(|s| s.trim().to_owned())
            .unwrap_or_else(|_| "main".to_owned()),
    };
    Ok(git(&["merge-base", &base, "HEAD"])
        .with_context(|| format!("no merge-base with {base}"))?
        .trim()
        .to_owned())
}

/// Diff `base` against the working tree, optionally limited to `paths`.
pub fn diff(base: &str, context: u32, paths: &[&str]) -> Result<Vec<FileDiff>> {
    let u = format!("-U{context}");
    let specs: Vec<String> = paths.iter().map(|p| format!(":(literal){p}")).collect();
    let mut args = vec![
        "-c", "core.quotepath=off", "diff", "--no-color", "--no-ext-diff", "-M",
        "--src-prefix=a/", "--dst-prefix=b/", &u, base, "--",
    ];
    args.extend(specs.iter().map(String::as_str));
    Ok(parse(&git(&args)?))
}

pub fn parse(s: &str) -> Vec<FileDiff> {
    let mut files: Vec<FileDiff> = vec![];
    let (mut old, mut new, mut in_hunk) = (0u32, 0u32, false);
    for l in s.lines() {
        if let Some(rest) = l.strip_prefix("diff --git ") {
            let p = rest.rsplit_once(" b/").map_or(rest, |(_, b)| b).to_owned();
            files.push(FileDiff { path: p.clone(), old_path: p, meta: String::new(), lines: vec![], full: vec![], headers: vec![] });
            in_hunk = false;
            continue;
        }
        let Some(f) = files.last_mut() else { continue };
        if l.starts_with("@@ ") {
            let mut it = l.split_whitespace().skip(1);
            let mut num = || -> u32 {
                it.next().and_then(|s| s.get(1..)?.split(',').next()?.parse().ok()).unwrap_or(0)
            };
            (old, new, in_hunk) = (num(), num(), true);
            f.lines.push(Line { kind: Kind::Hunk, old: None, new: None, text: l.to_owned() });
            continue;
        }
        if !in_hunk {
            if let Some(p) = l.strip_prefix("--- a/") {
                f.old_path = p.to_owned();
            } else if let Some(p) = l.strip_prefix("+++ b/") {
                f.path = p.to_owned();
            } else if let Some(p) = l.strip_prefix("rename from ") {
                f.old_path = p.to_owned();
                f.meta = format!("renamed from {p}");
            } else if let Some(p) = l.strip_prefix("rename to ") {
                f.path = p.to_owned();
            } else if l.starts_with("new file") {
                f.meta = "new".into();
            } else if l.starts_with("deleted file") {
                f.meta = "deleted".into();
            } else if l.starts_with("Binary files") {
                f.meta = "binary".into();
            }
            continue;
        }
        let kind = match l.as_bytes().first() {
            Some(b'+') => Kind::Add,
            Some(b'-') => Kind::Del,
            Some(b'\\') => continue, // "\ No newline at end of file"
            _ => Kind::Ctx,
        };
        let (o, n) = match kind {
            Kind::Add => (None, Some(new)),
            Kind::Del => (Some(old), None),
            _ => (Some(old), Some(new)),
        };
        old += o.is_some() as u32;
        new += n.is_some() as u32;
        f.lines.push(Line { kind, old: o, new: n, text: l.get(1..).unwrap_or("").to_owned() });
    }
    files
}

#[cfg(test)]
pub const SAMPLE: &str = "\
diff --git a/src/a.rs b/src/a.rs
index 1..2 100644
--- a/src/a.rs
+++ b/src/a.rs
@@ -1,3 +1,3 @@ fn main
 fn main() {
-    old();
+    new_call(x);
 }
diff --git a/old.rs b/new.rs
similarity index 100%
rename from old.rs
rename to new.rs
diff --git a/img.png b/img.png
new file mode 100644
Binary files /dev/null and b/img.png differ
";

#[test]
fn parses() {
    let f = parse(SAMPLE);
    assert_eq!(f.len(), 3);
    let k: Vec<_> = f[0].lines.iter().map(|l| (l.kind, l.old, l.new)).collect();
    assert_eq!(k, [
        (Kind::Hunk, None, None),
        (Kind::Ctx, Some(1), Some(1)),
        (Kind::Del, Some(2), None),
        (Kind::Add, None, Some(2)),
        (Kind::Ctx, Some(3), Some(3)),
    ]);
    assert_eq!(f[0].lines[3].text, "    new_call(x);");
    assert_eq!((f[1].old_path.as_str(), f[1].path.as_str()), ("old.rs", "new.rs"));
    assert_eq!(f[2].meta, "binary");
}

#[test]
fn words() {
    let (a, b) = word_diff("    let a = 1;", "    let ab = 3;").unwrap();
    assert_eq!((a, b), (vec![(8, 9), (12, 13)], vec![(8, 10), (13, 14)]));
    assert!(word_diff("fn alpha() {}", "struct Beta;").is_none()); // nothing in common
    assert_eq!(hunk_label("@@ -1,4 +12,7 @@ fn main"), Some((12, 7, "fn main".into())));
}

#[test]
fn hunk_windows() {
    // 30 lines, changes at 10 and 25: two hunks at 3 lines of context.
    let full: Vec<Line> = (0..30u32)
        .map(|i| {
            let kind = if i == 10 || i == 25 { Kind::Add } else { Kind::Ctx };
            Line { kind, old: (kind == Kind::Ctx).then_some(i + 1), new: Some(i + 1), text: String::new() }
        })
        .collect();
    assert_eq!(hunks(&full, 3), [(10, 10), (25, 25)]);
    assert_eq!(windows(&full, 3, |_| 0, false), [(7, 13, vec![10]), (22, 28, vec![25])]);
    // Expanding the first hunk by 10 reaches the second: they merge.
    assert_eq!(windows(&full, 3, |a| if a == 10 { 10 } else { 0 }, false), [(0, 28, vec![10, 25])]);
    let lines = shown(&full, &windows(&full, 3, |_| 0, false), &[(8, " fn x".into())]);
    assert_eq!(lines[0].text, "@@ -8,6 +8,7 @@ fn x");
    assert_eq!(lines.len(), 2 + 7 + 7);
}
