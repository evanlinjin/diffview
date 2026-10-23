//! Marked PRs (`m`), kept per repo across runs (and other dv lists of it).
use super::*;

pub(super) fn marks_path() -> Option<PathBuf> {
    crate::claude::dv_dir().ok().map(|d| d.join("marked.json"))
}

pub(super) fn load_marks() -> HashSet<u64> {
    marks_path().map(|p| read_marks(&p)).unwrap_or_default()
}

/// Save the change from `was` to `marked` onto the marks saved now, and return those.
pub(super) fn save_marks(was: &HashSet<u64>, marked: &HashSet<u64>) -> Result<HashSet<u64>> {
    merge_marks(&marks_path().context("no dv directory for this repo")?, was, marked)
}

pub(super) fn merge_marks(p: &std::path::Path, was: &HashSet<u64>, marked: &HashSet<u64>) -> Result<HashSet<u64>> {
    let mut now = read_marks(p);
    now.retain(|n| marked.contains(n) || !was.contains(n));
    now.extend(marked.difference(was));
    write_marks(p, &now)?;
    Ok(now)
}

pub(super) fn read_marks(p: &std::path::Path) -> HashSet<u64> {
    let text = std::fs::read_to_string(p).unwrap_or_default();
    serde_json::from_str::<Vec<u64>>(&text).unwrap_or_default().into_iter().collect()
}

pub(super) fn write_marks(p: &std::path::Path, marked: &HashSet<u64>) -> Result<()> {
    let mut v: Vec<u64> = marked.iter().copied().collect();
    v.sort();
    crate::todo::write_json(p, &serde_json::json!(v))
}

