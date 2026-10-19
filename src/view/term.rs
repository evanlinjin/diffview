//! The terminal, and the programs dv runs in it: your editor, hx, zellij panes, the clipboard.

use super::*;

/// dv's hx pane inside zellij.
pub(super) struct HxPane {
    /// e.g. `terminal_12`, as `zellij run` prints it.
    pub(super) pane: String,
}

/// Whether zellij pane `id` (e.g. `terminal_12`) is still there and running.
pub(super) fn pane_alive(id: &str) -> bool {
    let Some(n) = id.strip_prefix("terminal_").and_then(|n| n.parse::<u64>().ok()) else { return false };
    let Ok(out) = std::process::Command::new("zellij").args(["action", "list-panes", "-j", "-s"]).stdin(std::process::Stdio::null()).output() else { return false };
    let panes: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
    panes.as_array().is_some_and(|ps| ps.iter().any(|p| p["id"].as_u64() == Some(n) && p["is_plugin"] != true && p["exited"] != true))
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

/// `zellij run`, opening the pane to the right of dv's. Not when dv's own pane is floating:
/// zellij then quietly opens nothing (and doesn't block), so it goes where zellij puts it.
pub(super) fn zellij_run() -> Vec<&'static str> {
    let floating = || {
        let me = std::env::var("ZELLIJ_PANE_ID").ok()?.parse::<u64>().ok()?;
        let out = std::process::Command::new("zellij").args(["action", "list-panes", "-j"]).stdin(std::process::Stdio::null()).output().ok()?;
        let panes: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
        panes.as_array()?.iter().find(|p| p["id"].as_u64() == Some(me) && p["is_plugin"] != true)?["is_floating"].as_bool()
    };
    match floating() {
        Some(true) => vec!["run"],
        _ => vec!["run", "--direction", "right"],
    }
}

/// Run an interactive program and wait for it: in a pane to the right inside zellij (its exit
/// status comes through), otherwise in this terminal (the caller suspends the TUI).
pub(super) fn interactive(prog: &str, args: &[String], name: &str) -> std::io::Result<std::process::ExitStatus> {
    if !in_zellij() {
        return std::process::Command::new(prog).args(args).status();
    }
    let cwd = std::env::current_dir()?;
    let st = std::process::Command::new("zellij")
        .args(zellij_run())
        .args(["--close-on-exit", "--block-until-exit", "--name", name])
        // `zellij run` doesn't need dv's terminal; sharing it, it would take dv's keystrokes
        // while it waits (and freeze dv when hx runs beside it).
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .arg("--cwd")
        .arg(cwd)
        .arg("--")
        .arg(prog)
        .args(args)
        .status();
    // Its pane gone, zellij focuses one beside it, not always dv's: your keys would go there.
    focus_dv();
    st
}

/// zellij's focus back on dv's pane (`$ZELLIJ_PANE_ID`), when it may have gone to another.
pub(crate) fn focus_dv() {
    if let Some(id) = std::env::var("ZELLIJ_PANE_ID").ok().filter(|_| in_zellij()) {
        _ = std::process::Command::new("zellij")
            .args(["action", "focus-pane-id", &format!("terminal_{id}")])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

/// In zellij, dv goes on while you write: the editor in a pane of its own, not waited for.
/// `compose_with` then fails with this, and what asked is done again once it's written, the
/// text then at hand (`written`).
#[derive(Debug)]
pub(crate) struct Writing;

impl std::fmt::Display for Writing {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("writing in the editor's pane")
    }
}

impl std::error::Error for Writing {}

thread_local! {
    /// Whether the editor may run beside dv, not waited for (`writing_beside`).
    static BESIDE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The file being written in the editor's pane, and where its exit status lands once it quits.
static PANE: std::sync::Mutex<Option<(PathBuf, PathBuf)>> = std::sync::Mutex::new(None);
/// What was written there (and whether the editor quit cleanly), for the one doing it again.
static WRITTEN: std::sync::Mutex<Option<(String, bool)>> = std::sync::Mutex::new(None);

/// `f`, with the editor (in zellij) beside dv rather than waited for.
pub(super) fn writing_beside<T>(f: impl FnOnce() -> T) -> T {
    BESIDE.set(in_zellij());
    let out = f();
    BESIDE.set(false);
    out
}

/// Whether what was being written in the editor's pane is done (it quit): its text is then
/// at hand for `compose_with`.
pub(super) fn written() -> bool {
    let mut pane = PANE.lock().unwrap();
    let Some((path, done)) = pane.as_ref() else { return false };
    let Ok(status) = std::fs::read_to_string(done) else { return false };
    let text = std::fs::read_to_string(path).unwrap_or_default();
    _ = (std::fs::remove_file(path), std::fs::remove_file(done));
    *WRITTEN.lock().unwrap() = Some((text, status.trim() == "0"));
    *pane = None;
    true
}

/// Like `compose`, with `prefill` above the cursor (e.g. a quote). Leaving only the prefill
/// counts as empty, unless `untouched_ok` (the prefill is a draft to post as is).
pub(crate) fn compose_with(what: &str, allow_empty: bool, prefill: &str, untouched_ok: bool) -> Result<Option<String>> {
    const MARK: &str = HINT;
    let finish = |text: &str, ok: bool| {
        let body = clean_body(text, MARK, if untouched_ok { "" } else { prefill });
        Ok((ok && (allow_empty || !body.is_empty())).then_some(body))
    };
    // Written beside dv: the text, as the editor left it.
    if let Some((text, ok)) = WRITTEN.lock().unwrap().take() {
        return finish(&text, ok);
    }
    let beside = BESIDE.get();
    if beside && PANE.lock().unwrap().is_some() {
        anyhow::bail!("you're writing in the editor's pane already: save and quit there first (:cq cancels)");
    }
    let path = std::env::temp_dir().join(format!("dv-comment-{}.md", std::process::id()));
    let hint = if allow_empty { "leave empty for none; :cq to cancel" } else { "leave empty to cancel" };
    std::fs::write(&path, format!("{prefill}\n{MARK} {what}. Save and quit when done; {hint}. -->\n"))?;
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
    if beside {
        // Not waited for: its exit status written to `done` as it quits (`written` looks).
        let done = path.with_extension("done");
        _ = std::fs::remove_file(&done);
        let q = |s: &str| format!("'{}'", s.replace('\'', "'\\''"));
        let script = format!("\"$0\" \"$@\"; echo $? > {}", q(&done.to_string_lossy()));
        let st = std::process::Command::new("zellij")
            .args(zellij_run())
            .args(["--close-on-exit", "--name", &format!("dv: {what}"), "--cwd"])
            .arg(std::env::current_dir()?)
            .args(["--", "sh", "-c", &script, prog])
            .args(&args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()?;
        anyhow::ensure!(st.success(), "zellij run failed ({st})");
        *PANE.lock().unwrap() = Some((path, done));
        return Err(Writing.into());
    }
    let st = interactive(prog, &args, &format!("dv: {what}"))?;
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    _ = std::fs::remove_file(&path);
    finish(&text, st.success())
}

/// Copy to the system clipboard via OSC 52 (handled by the terminal; works over SSH).
pub(super) fn clipboard(t: &str) {
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
    // The editor's pane quit (status 0), having written this: what asked takes it, once.
    let dir = std::env::temp_dir().join(format!("dv-beside-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (path, done) = (dir.join("c.md"), dir.join("c.done"));
    std::fs::write(&path, format!("> quote\n\nmy reply\n{HINT} reply. Save and quit when done -->\n")).unwrap();
    *PANE.lock().unwrap() = Some((path.clone(), done.clone()));
    assert!(!written()); // still writing
    std::fs::write(&done, "0\n").unwrap();
    assert!(written() && !path.exists() && !written());
    assert_eq!(compose_with("reply", false, "> quote", false).unwrap().as_deref(), Some("> quote\n\nmy reply"));
    // `:cq`: cancelled.
    *WRITTEN.lock().unwrap() = Some(("x\n".into(), false));
    assert_eq!(compose_with("reply", false, "", false).unwrap(), None);
    std::fs::remove_dir_all(dir).unwrap();
}
