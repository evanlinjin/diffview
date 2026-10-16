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
    pub lines: Vec<Line>,
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
            files.push(FileDiff { path: p.clone(), old_path: p, meta: String::new(), lines: vec![] });
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
