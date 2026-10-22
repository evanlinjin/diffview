//! zellij panes: listing them, and the Claude Code panes dv runs (what their screens say, offering
//! them a prompt, closing them when Claude is done).
use anyhow::Result;

/// `zellij action args…`: what it printed, or why it failed.
pub(crate) fn action(args: &[&str]) -> Result<String> {
    let out = std::process::Command::new("zellij").arg("action").args(args).stdin(std::process::Stdio::null()).output()?;
    anyhow::ensure!(out.status.success(), "zellij {} failed ({})", args[0], out.status);
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Pane `pane` (a terminal: not a plugin) among `panes`.
pub(crate) fn find(panes: &[serde_json::Value], pane: u64) -> Option<&serde_json::Value> {
    panes.iter().find(|p| p["id"].as_u64() == Some(pane) && p["is_plugin"] != true)
}

/// Whether zellij pane `id` (e.g. `terminal_12`) is still there and running.
pub(crate) fn alive(id: &str) -> bool {
    state(id) == Some(true)
}

/// Whether pane `id` is open (`Some(false)`: zellij says it isn't), or `None`: can't tell.
pub(crate) fn state(id: &str) -> Option<bool> {
    let n = id.strip_prefix("terminal_").and_then(|n| n.parse::<u64>().ok())?;
    Some(find(&list(&[])?, n).is_some_and(|p| p["exited"] != true))
}

/// zellij's panes (`list-panes -j -s`, and `more`: `-c` their commands, `-t` their tabs), or
/// `None`: it couldn't say.
pub(crate) fn list(more: &[&str]) -> Option<Vec<serde_json::Value>> {
    let out = action(&[&["list-panes", "-j", "-s"], more].concat()).ok()?;
    serde_json::from_str::<serde_json::Value>(&out).ok()?.as_array().cloned()
}

/// Put `prompt` in a Claude Code pane's input, as a paste (so a multi-line prompt arrives
/// whole), and focus the pane; you press Enter there. Not if the pane is asking you something
/// (a permission prompt): `Ok(false)`, and nothing is typed.
pub(crate) fn offer(pane: u64, prompt: &str) -> Result<bool> {
    let id = format!("terminal_{pane}");
    if asking(&action(&["dump-screen", "-p", &id])?) {
        return Ok(false);
    }
    // In another tab (an older dv's Claude tab): not pasted into, as it couldn't be focused
    // there (zellij can crash on it), and Enter would go to whichever pane is.
    anyhow::ensure!(other_tab(pane).is_none(), "it's in another zellij tab (an older dv's): close it there, and Space a starts one here");
    action(&["paste", "-p", &id, prompt])?;
    show(pane)?;
    Ok(true)
}

/// Close Claude's pane `pane` (session `session`, run since `since`) if Claude's done there:
/// its transcript says it ended its turn (`turn_ended`: not a guess from the screen), not on a
/// question for you, nor asking you something (a permission), with nothing in its input box (a
/// request waiting for Enter, words of yours), and you aren't looking at it. Whether it closed.
pub(crate) fn close_if_done(pane: u64, session: &str, since: i64) -> bool {
    let Some(last) = crate::claude::turn_ended(session, since) else { return false };
    if last.trim_end().ends_with('?') {
        return false;
    }
    let id = format!("terminal_{pane}");
    // With its styling: what's typed, not Claude Code's dimmed suggestions.
    let Ok(screen) = action(&["dump-screen", "-a", "-p", &id]) else { return false };
    if asking(&strip_ansi(&screen)) || !input_empty(&screen) {
        return false;
    }
    // Focused in its tab, and up there (its tab's floating panes shown, or tiled): you may be
    // reading it.
    let panes = list(&["-t"]).unwrap_or_default();
    let Some(me) = find(&panes, pane) else { return false };
    let tab = me["tab_id"].as_u64().map(|t| t.to_string()).unwrap_or_default();
    let shown = || action(&["are-floating-panes-visible", "--tab-id", &tab]).is_ok_and(|o| o.trim() == "true");
    if me["is_focused"] == true && (me["is_floating"] != true || shown()) {
        return false;
    }
    action(&["close-pane", "-p", &id]).is_ok()
}

/// Whether Claude Code's input box on `screen` (with its styling, `dump-screen -a`) is empty:
/// what's between its last two rules (`────`), past the `❯` prompt, has nothing typed (in the
/// text's own color: its suggestions, dimmed or grey, aren't). Not found: not empty (nothing's
/// closed on a guess).
pub(crate) fn input_empty(screen: &str) -> bool {
    let lines: Vec<&str> = screen.lines().collect();
    let rule = |l: &str| {
        let t = strip_ansi(l);
        t.trim().chars().count() >= 10 && t.trim().chars().all(|c| c == '─')
    };
    let rules: Vec<usize> = (0..lines.len()).filter(|&i| rule(lines[i])).collect();
    let [.., a, b] = rules[..] else { return false };
    !lines[a + 1..b].iter().any(|l| typed(l))
}

/// Whether `line` (with its styling) has text in a plain color: not dimmed, not grey.
fn typed(line: &str) -> bool {
    let (mut dim, mut grey) = (false, false);
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                let mut params = String::new();
                for d in chars.by_ref() {
                    if d.is_ascii_alphabetic() {
                        if d == 'm' {
                            sgr(&params, &mut dim, &mut grey);
                        }
                        break;
                    }
                    params.push(d);
                }
            }
            continue;
        }
        if !dim && !grey && !c.is_whitespace() && !matches!(c, '❯' | '>' | '\u{a0}' | '│') {
            return true;
        }
    }
    false
}

/// An SGR's effect on dim and grey (a grey: equal r, g, b below 200; 90; 8 or 232–250).
fn sgr(params: &str, dim: &mut bool, grey: &mut bool) {
    let n: Vec<u32> = params.split(';').map(|p| p.parse().unwrap_or(0)).collect();
    let mut i = 0;
    while i < n.len() {
        match n[i] {
            0 => (*dim, *grey) = (false, false),
            2 => *dim = true,
            22 => *dim = false,
            30..=37 | 39 | 91..=97 => *grey = false,
            90 => *grey = true,
            38 if n.get(i + 1) == Some(&2) && i + 4 < n.len() => {
                let (r, g, b) = (n[i + 2], n[i + 3], n[i + 4]);
                *grey = r == g && g == b && r < 200;
                i += 4;
            }
            38 if n.get(i + 1) == Some(&5) => {
                *grey = n.get(i + 2).is_some_and(|&c| c == 8 || (232..=250).contains(&c));
                i += 2;
            }
            _ => {}
        }
        i += 1;
    }
}

/// `s` without its ANSI escape sequences.
pub(crate) fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            chars.next();
            for d in chars.by_ref() {
                if d.is_ascii_alphabetic() {
                    break;
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// What zellij pane `pane` shows (its screen, plain).
pub(crate) fn screen(pane: u64) -> String {
    action(&["dump-screen", "-p", &format!("terminal_{pane}")]).unwrap_or_default()
}

/// Bring Claude's pane `pane` up: in another tab (an older dv's Claude tab), that tab; in dv's
/// own, the pane focused. Not focused in another tab: zellij
/// (0.45) can panic doing that, taking the session down ("Cannot focus pane … as it is not
/// selectable").
pub(crate) fn show(pane: u64) -> Result<()> {
    match other_tab(pane) {
        Some(t) => action(&["go-to-tab-by-id", &t.to_string()]),
        None => action(&["focus-pane-id", &format!("terminal_{pane}")]),
    }
    .map(drop)
}

/// Whether a Claude Code pane's screen shows it working: its spinner line, a word and how long
/// it's been at it ("· Moseying… (59s · ↓ 1.3k tokens)"), or, as it used to say, how to stop it.
pub(crate) fn working(screen: &str) -> bool {
    let screen = live(screen);
    screen.contains("esc to interrupt") || screen.lines().any(|l| l.split_once("… (").is_some_and(|(_, t)| t.starts_with(|c: char| c.is_ascii_digit())))
}

/// Whether a Claude Code pane's screen shows it asking you something: a permission prompt (a
/// numbered menu, its cursor on an option), or (in a folder it hasn't seen) whether you trust
/// it. Typing in then would answer it. (Not its words asking: a reply ending "Would you like
/// me to…?" is done, waiting as any.)
pub(crate) fn asking(screen: &str) -> bool {
    // (Its cursor indented, in its menu: a message of yours, `❯ 1. yes`, is at the line's start.)
    let option = |l: &str| {
        let o = l.strip_prefix([' ', '│']).map(|l| l.trim_start_matches([' ', '│']));
        o.and_then(|o| o.strip_prefix("❯ ")).is_some_and(|o| o.split_once('.').is_some_and(|(n, _)| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())))
    };
    let screen = live(screen);
    screen.contains("trust this folder") || screen.lines().any(option)
}

/// What's live on a Claude Code screen: past its transcript's last entry (`●` at a line's
/// start) and that entry's own text (blank or indented lines after it): its spinner, tasks, a
/// menu, the input box. Not what it said or showed (a reply reading "… (3 passed)", a file's
/// text).
fn live(screen: &str) -> &str {
    // (No entry on screen: its start scrolled off, its tail at the top, skipped as one's.)
    let mut rest = match screen.rfind("\n●").map(|i| i + 1).or_else(|| screen.starts_with('●').then_some(0)) {
        Some(i) => screen[i..].split_once('\n').map_or("", |r| r.1),
        None => screen,
    };
    while let Some((l, more)) = rest.split_once('\n') {
        if !(l.trim().is_empty() || l.starts_with("  ")) {
            break;
        }
        rest = more;
    }
    rest
}

/// What Claude Code in pane `p` is doing, if it's Claude Code there (its command: zellij's is
/// the pane's foreground process, not an editor's or a shell's, whatever their text) and at
/// it: working, or asking you something (by what's live on its screen).
pub(crate) fn claude_doing(p: &Pane) -> Option<&'static str> {
    p.1.contains("claude").then(|| doing(&screen(p.0)))?
}

fn doing(screen: &str) -> Option<&'static str> {
    match (asking(screen), working(screen)) {
        (true, _) => Some("asking you (in its pane)"),
        (_, true) => Some("working (in its pane)"),
        _ => None,
    }
}

/// The tab zellij pane `pane` is in, if it's another than dv's.
fn other_tab(pane: u64) -> Option<u64> {
    let panes = list(&["-t"])?;
    let tab = |n: u64| find(&panes, n).and_then(|p| p["tab_id"].as_u64());
    let own = std::env::var("ZELLIJ_PANE_ID").ok().and_then(|id| id.parse().ok()).and_then(tab)?;
    tab(pane).filter(|&t| t != own)
}

/// A zellij pane: its id, its command line, its working directory.
pub(crate) type Pane = (u64, String, std::path::PathBuf);

/// zellij's open panes (none outside zellij).
pub(crate) fn open() -> Vec<Pane> {
    if !crate::view::in_zellij() {
        return vec![];
    }
    list(&["-c"])
        .unwrap_or_default()
        .iter()
        .filter(|p| p["exited"] != true && p["is_plugin"] != true)
        .filter_map(|p| Some((p["id"].as_u64()?, p["pane_command"].as_str()?.to_owned(), std::path::PathBuf::from(p["pane_cwd"].as_str().unwrap_or("")))))
        .collect()
}

/// The pane of Claude Code's session `id` among `panes`: its own, not a fork of it
/// (`--fork-session`: a session of its own, "a copy" to talk in).
pub(crate) fn of_session<'a>(panes: &'a [Pane], id: &str) -> Option<&'a Pane> {
    panes.iter().find(|p| p.1.contains(id) && !p.1.contains("--fork-session"))
}

/// Whether a Claude run is at it: its process (`pid`) still there, or Claude Code open in its
/// pane (by its `session`, among `panes`, looked up once): open, it's at the session, whether
/// working now or waiting on you (a pane done with its request closes, `close_if_done`).
pub(crate) fn run_live(pid: Option<u32>, session: Option<&str>, panes: &std::cell::OnceCell<Vec<Pane>>) -> bool {
    match (pid, session) {
        (Some(pid), _) => crate::alive(pid),
        (None, Some(id)) => of_session(panes.get_or_init(open), id).is_some(),
        _ => false,
    }
}

#[test]
fn what_claude_is_doing_now() {
    let foot = "───\n❯ \n───\n  ⏵⏵ auto mode on";
    // Done, its reply asking you something in words: waiting, as any.
    assert_eq!(doing(&format!("● Fixed it.\n  Would you like me to also update the docs?\n✻ Worked for 49s\n{foot}")), None);
    // Working, its tasks under the spinner however many.
    let tasks: String = (1..9).map(|i| format!("  ☐ task {i}\n")).collect();
    assert_eq!(doing(&format!("● Reading\n· Moseying… (59s · ↓ 1.3k tokens)\n{tasks}{foot}")), Some("working (in its pane)"));
    // Asking: its menu.
    assert_eq!(doing("● Bash(rm x)\n Do you want to proceed?\n ❯ 1. Yes\n   2. No"), Some("asking you (in its pane)"));
    // Not: a long reply whose start scrolled off.
    assert_eq!(doing(&format!("  and more of it\n  Ran the suite again… (342 passed, 0 failed).\n\n✻ Worked for 9s\n{foot}")), None);
    // Not: a numbered message of yours, its reply's own text, what a tool showed.
    assert_eq!(doing(&format!("❯ 1. yes\n● Ran the suite again… (342 passed, 0 failed).\n  ⎿ \"trust this folder\"\n✻ Worked for 9s\n{foot}")), None);
}
