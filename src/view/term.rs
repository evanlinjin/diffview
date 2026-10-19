//! The terminal, and the programs dv runs in it: your editor, hx, zellij panes, the clipboard.

use super::*;

/// dv's hx pane inside zellij.
pub(super) struct HxPane {
    /// e.g. `terminal_12`, as `zellij run` prints it.
    pub(super) pane: String,
}

/// Mouse support (wheel, click, drag); `--no-mouse` turns it off, leaving the terminal's own
/// selection as it is.
pub static MOUSE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Take over the terminal (ratatui), with mouse capture unless `--no-mouse`.
pub fn init_terminal() -> DefaultTerminal {
    let term = ratatui::init();
    if MOUSE.load(std::sync::atomic::Ordering::Relaxed) {
        _ = ratatui::crossterm::execute!(std::io::stdout(), ratatui::crossterm::event::EnableMouseCapture);
    }
    term
}

/// Give the terminal back (mouse capture off first, so an editor or the shell gets it clean).
pub fn restore_terminal() {
    if MOUSE.load(std::sync::atomic::Ordering::Relaxed) {
        _ = ratatui::crossterm::execute!(std::io::stdout(), ratatui::crossterm::event::DisableMouseCapture);
    }
    ratatui::restore();
}

pub(crate) fn in_zellij() -> bool {
    std::env::var_os("ZELLIJ").is_some()
}

/// `zellij run`, opening the pane below dv's. Not when dv's own pane is floating:
/// zellij then quietly opens nothing (and doesn't block), so it goes where zellij puts it.
pub(super) fn zellij_run() -> Vec<&'static str> {
    let floating = || {
        let me = std::env::var("ZELLIJ_PANE_ID").ok()?.parse::<u64>().ok()?;
        crate::pane::find(&crate::pane::list(&[])?, me)?["is_floating"].as_bool()
    };
    match floating() {
        Some(true) => vec!["run"],
        // Below dv: it keeps its width (its comments' pane on its right), the editor under it.
        _ => vec!["run", "--direction", "down"],
    }
}

/// Run an interactive program and wait for it: in a pane below dv inside zellij (its exit
/// status comes through), otherwise in this terminal (the caller suspends the TUI).
pub(super) fn interactive(prog: &str, args: &[String], name: &str) -> std::io::Result<std::process::ExitStatus> {
    if !in_zellij() {
        return std::process::Command::new(prog).args(args).status();
    }
    let st = zellij_pane(prog, args, name, &std::env::current_dir()?, true).map(|(st, _)| st);
    // Its pane gone, zellij focuses one beside it, not always dv's: your keys would go there.
    focus_dv();
    st
}

/// `prog args` in a pane of its own (`zellij run`, named `name`, in `cwd`): waited for (its
/// exit status), or not; and its pane id (`terminal_N`): as zellij says it, else the newest
/// pane of that name, so a pane closed from zellij is noticed either way.
pub(super) fn zellij_pane(prog: &str, args: &[String], name: &str, cwd: &Path, wait: bool) -> std::io::Result<(std::process::ExitStatus, Option<String>)> {
    let out = std::process::Command::new("zellij")
        .args(zellij_run())
        .args(["--close-on-exit", "--name", name])
        .args(wait.then_some("--block-until-exit"))
        .arg("--cwd")
        .arg(cwd)
        .arg("--")
        .arg(prog)
        .args(args)
        // `zellij run` doesn't need dv's terminal; sharing it, it would take dv's keystrokes
        // while it waits (and freeze dv when hx runs beside it).
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()?;
    // (Waited for, it's gone: no id to find.)
    if wait {
        return Ok((out.status, None));
    }
    let said = String::from_utf8_lossy(&out.stdout);
    let id = said.lines().map(str::trim).find(|l| l.starts_with("terminal_")).map(str::to_owned).or_else(|| {
        let id = crate::pane::list(&[])?.iter().filter(|p| p["title"] == name && p["is_plugin"] != true).filter_map(|p| p["id"].as_u64()).max()?;
        Some(format!("terminal_{id}"))
    });
    Ok((out.status, id))
}

/// zellij's focus back on dv's pane (`$ZELLIJ_PANE_ID`), when it may have gone to another.
pub(crate) fn focus_dv() {
    if let Some(id) = std::env::var("ZELLIJ_PANE_ID").ok().filter(|_| in_zellij()) {
        _ = crate::pane::action(&["focus-pane-id", &format!("terminal_{id}")]);
    }
}

/// In zellij, dv goes on while you write: the editor in a pane of its own, not waited for.
/// `compose_with` fails with this; dv keeps it with what asked (`App::writing`), and once it's
/// `done`, does that again, given what it'd have returned (`writing_beside`).
#[derive(Debug)]
pub(crate) struct Writing {
    path: PathBuf,
    /// Where the editor's exit status lands as it quits.
    done: PathBuf,
    /// Where the shell running the editor in its pane notes its pid: that gone (the pane closed
    /// from zellij) without the editor quitting, it's cancelled. (Not asked of zellij, which
    /// can't always list its panes.)
    pid: PathBuf,
    looked: std::time::Instant,
    /// As `compose_with` was asked: the text's made of what's written as it would have been.
    allow_empty: bool,
    prefill: String,
    untouched_ok: bool,
    /// Its pane closed without the editor quitting: where what it had saved is kept.
    pub(super) kept: Option<PathBuf>,
}

impl std::fmt::Display for Writing {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("writing in the editor's pane")
    }
}

impl std::error::Error for Writing {}

impl Writing {
    /// Once it's done: what `compose_with` would have returned (the text, or `None`:
    /// cancelled). Its pane gone without a word (closed from zellij): cancelled.
    pub(super) fn done(&mut self) -> Option<Option<String>> {
        let status = match std::fs::read_to_string(&self.done) {
            Ok(s) => s,
            // (Its shell looked for every few seconds; gone only if it's gone, and the editor
            // didn't quit just then. Not started yet, no pid: still writing.)
            Err(_) if self.looked.elapsed() > std::time::Duration::from_secs(3) => {
                self.looked = std::time::Instant::now();
                let pid = std::fs::read_to_string(&self.pid).unwrap_or_default();
                let alive = |p: &str| std::process::Command::new("kill").args(["-0", p.trim()]).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success());
                if pid.trim().is_empty() || alive(&pid) {
                    return None;
                }
                std::fs::read_to_string(&self.done).unwrap_or_default()
            }
            Err(_) => return None,
        };
        let text = std::fs::read_to_string(&self.path).unwrap_or_default();
        _ = std::fs::remove_file(&self.done);
        _ = std::fs::remove_file(&self.pid);
        let closed = status.is_empty();
        // Closed from zellij with something saved: kept, not lost.
        // (Something of yours: not just the prefill it started with.)
        if closed && finish(&text, true, false, &self.prefill, false).is_some() {
            self.kept = Some(unsent_path());
            if self.kept.as_ref().is_some_and(|k| std::fs::rename(&self.path, k).is_ok()) {
                return Some(None);
            }
            self.kept = None;
        }
        _ = std::fs::remove_file(&self.path);
        Some(finish(&text, status.trim() == "0", self.allow_empty, &self.prefill, self.untouched_ok))
    }
}

thread_local! {
    /// While dv does something (`writing_beside`): whether the editor may run beside it (and
    /// whether one already does), and what was written for it, given in place of the editor.
    static BESIDE: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
    static GIVEN: std::cell::RefCell<Option<Option<String>>> = const { std::cell::RefCell::new(None) };
}

/// `f`, with the editor (in zellij) beside dv rather than waited for (`busy`: one is already),
/// and `given` (what was written for it beside dv) for its `compose_with`. Also what of that
/// it didn't take (it gave up before its editor).
pub(super) fn writing_beside<T>(busy: bool, given: Option<Option<String>>, f: impl FnOnce() -> T) -> (T, Option<Option<String>>) {
    BESIDE.set(in_zellij().then_some(busy));
    GIVEN.set(given);
    let out = f();
    BESIDE.set(None);
    (out, GIVEN.take())
}

/// Where text that couldn't be sent is kept: a file of its own each time.
pub(super) fn unsent_path() -> PathBuf {
    temp_file("dv-unsent")
}

/// A new temp file's path, `<prefix>-<pid>-<n>.md`: one of its own each time.
fn temp_file(prefix: &str) -> PathBuf {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    std::env::temp_dir().join(format!("{prefix}-{}-{n}.md", std::process::id()))
}

/// What's written in the editor, as `compose_with` returns it: empty (or just the prefill,
/// unless `untouched_ok`) is nothing, unless `allow_empty`; `ok`: the editor quit cleanly.
fn finish(text: &str, ok: bool, allow_empty: bool, prefill: &str, untouched_ok: bool) -> Option<String> {
    let body = clean_body(text, HINT, if untouched_ok { "" } else { prefill });
    (ok && (allow_empty || !body.is_empty())).then_some(body)
}

/// Like `compose`, with `prefill` above the cursor (e.g. a quote). Leaving only the prefill
/// counts as empty, unless `untouched_ok` (the prefill is a draft to post as is).
pub(crate) fn compose_with(what: &str, allow_empty: bool, prefill: &str, untouched_ok: bool) -> Result<Option<String>> {
    // Written beside dv already: that.
    if let Some(given) = GIVEN.take() {
        return Ok(given);
    }
    let beside = BESIDE.get();
    if beside == Some(true) {
        anyhow::bail!("you're writing in the editor's pane already: save and quit there first (:cq cancels)");
    }
    // (One file each: one being written beside dv may outlive the view that asked.)
    let path = temp_file("dv-comment");
    let hint = if allow_empty { "leave empty for none; :cq to cancel" } else { "leave empty to cancel" };
    std::fs::write(&path, format!("{prefill}\n{HINT} {what}. Save and quit when done; {hint}. -->\n"))?;
    let editor = std::env::var("VISUAL").or_else(|_| std::env::var("EDITOR")).unwrap_or_else(|_| "hx".into());
    let mut parts = editor.split_whitespace();
    let prog = parts.next().unwrap_or("hx");
    // hx takes `file:line`: start on the prefill's first empty line (after a quote, or above a
    // suggestion block, where the explanation goes), else after it.
    let line = prefill.lines().position(str::is_empty).unwrap_or(prefill.lines().count()) + 1;
    let arg = match Path::new(prog).file_name().is_some_and(|n| n == "hx") {
        true => format!("{}:{line}", path.display()),
        false => path.display().to_string(),
    };
    let args: Vec<String> = parts.map(str::to_owned).chain([arg]).collect();
    if beside.is_some() {
        // Not waited for: its exit status lands in `done` as it quits (whole: moved there).
        let (done, pid) = (path.with_extension("done"), path.with_extension("pid"));
        _ = std::fs::remove_file(&done);
        _ = std::fs::remove_file(&pid);
        let q = |p: &Path| format!("'{}'", p.to_string_lossy().replace('\'', "'\\''"));
        let script = format!("echo $$ > {pid}; \"$0\" \"$@\"; echo $? > {tmp} && mv {tmp} {done}", pid = q(&pid), tmp = q(&path.with_extension("done.tmp")), done = q(&done));
        let sh: Vec<String> = ["-c".to_owned(), script, prog.to_owned()].into_iter().chain(args).collect();
        let (st, _) = zellij_pane("sh", &sh, &format!("dv: {what}"), &std::env::current_dir()?, false)?;
        anyhow::ensure!(st.success(), "zellij run failed ({st})");
        let looked = std::time::Instant::now();
        let prefill = prefill.to_owned();
        return Err(Writing { path, done, pid, looked, allow_empty, prefill, untouched_ok, kept: None }.into());
    }
    let st = interactive(prog, &args, &format!("dv: {what}"))?;
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    _ = std::fs::remove_file(&path);
    Ok(finish(&text, st.success(), allow_empty, prefill, untouched_ok))
}

/// Copy to the system clipboard via OSC 52 (handled by the terminal; works over SSH).
pub fn clipboard(t: &str) {
    let mut out = std::io::stdout();
    let _ = write!(out, "\x1b]52;c;{}\x07", base64(t.as_bytes()));
    let _ = out.flush();
}

pub(super) fn base64(b: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut o = String::new();
    for c in b.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            o.push(if i <= c.len() { T[(n >> (18 - 6 * i)) as usize & 63] as char } else { '=' });
        }
    }
    o
}

#[test]
fn written_beside_then_done_again() {
    // The editor's pane quit (status 0), having written this: what asked is given the text.
    let dir = std::env::temp_dir().join(format!("dv-beside-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (path, done) = (dir.join("c.md"), dir.join("c.done"));
    std::fs::write(&path, format!("> quote\n\nmy reply\n{HINT} reply. Save and quit when done -->\n")).unwrap();
    let looked = std::time::Instant::now();
    let mut w = Writing { path: path.clone(), done: done.clone(), pid: path.with_extension("pid"), looked, allow_empty: false, prefill: "> quote".into(), untouched_ok: false, kept: None };
    assert_eq!(w.done(), None); // still writing
    std::fs::write(&done, "0\n").unwrap();
    let text = w.done().unwrap();
    assert!(!path.exists() && text.as_deref() == Some("> quote\n\nmy reply"));
    // Given to what asks again; what it doesn't take comes back.
    let (got, left) = writing_beside(false, Some(text.clone()), || compose_with("reply", false, "", false).unwrap());
    assert_eq!((got, left), (text.clone(), None));
    let (_, left) = writing_beside(false, Some(text.clone()), || ());
    assert_eq!(left, Some(text));
    // Its pane closed from zellij: its shell's gone, the editor never quit. Kept, not lost.
    std::fs::write(&path, "> quote\n\nhalf a reply\n").unwrap();
    let mut gone = std::process::Command::new("true").spawn().unwrap();
    gone.wait().unwrap();
    std::fs::write(path.with_extension("pid"), gone.id().to_string()).unwrap();
    w.looked = std::time::Instant::now() - std::time::Duration::from_secs(4);
    assert_eq!(w.done(), Some(None));
    let kept = w.kept.take().unwrap();
    assert_eq!(std::fs::read_to_string(&kept).unwrap(), "> quote\n\nhalf a reply\n");
    _ = std::fs::remove_file(kept);
    // `:cq`: cancelled; just the prefill: nothing.
    std::fs::write(&path, "x\n").unwrap();
    std::fs::write(&done, "1\n").unwrap();
    assert_eq!(w.done(), Some(None));
    std::fs::write(&path, "> quote\n").unwrap();
    std::fs::write(&done, "0\n").unwrap();
    assert_eq!(w.done(), Some(None));
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(test)]
/// Something being written beside dv, for tests (in files that never appear).
pub(super) fn test_writing() -> Writing {
    let path = temp_file("dv-test-writing");
    let (done, looked) = (path.with_extension("done"), std::time::Instant::now());
    let pid = path.with_extension("pid");
    Writing { path, done, pid, looked, allow_empty: false, prefill: String::new(), untouched_ok: false, kept: None }
}
