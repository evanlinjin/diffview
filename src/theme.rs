//! Helix themes: `theme` from the Helix config, resolved through `inherits` and `[palette]`,
//! searched in the user's themes dir then Helix's runtime dirs. Falls back to ANSI colors.
use ratatui::style::{Color, Modifier, Style};
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use toml::{Table, Value};

pub struct Theme {
    styles: HashMap<String, Style>,
}

/// Resolved UI styles, looked up once.
pub struct Ui {
    pub base: Style,
    pub linenr: Style,
    pub plus: Style,
    pub minus: Style,
    pub add_bg: Color,
    pub del_bg: Color,
    /// The same tints on the cursor's line: blended into the cursorline color instead.
    pub add_bg_cur: Color,
    pub del_bg_cur: Color,
    pub hunk: Style,
    pub header: Style,
    pub selection: Style,
    pub cursor: Style,
    pub cursor_select: Style,
    pub status: Style,
    pub mode_normal: Style,
    pub mode_select: Style,
    pub popup: Style,
    pub menu_selected: Style,
    pub jump_label: Style,
    pub heading: Style,
    pub thread: Style,
    pub stale: Style,
    pub error: Style,
    pub warning: Style,
    pub resolved: Style,
    pub pending: Style,
    /// Changed words within a paired removed/added line: a stronger tint than the line's.
    pub emph_add: Color,
    pub emph_del: Color,
    pub cursorline: Option<Color>,
    pub linenr_selected: Style,
    /// Keys in the help overlay.
    pub key: Style,
    pub md: crate::md::Styles,
}

/// Helix `[editor]` settings dv mirrors.
#[derive(Default)]
pub struct Settings {
    pub cursorline: bool,
    pub relative: bool,
}

pub fn settings() -> Settings {
    let cfg = helix_config_dir()
        .and_then(|d| std::fs::read_to_string(d.join("config.toml")).ok())
        .and_then(|s| toml::from_str::<Table>(&s).ok())
        .unwrap_or_default();
    let editor = cfg.get("editor").and_then(Value::as_table);
    let get = |k: &str| editor.and_then(|e| e.get(k));
    Settings {
        cursorline: get("cursorline").and_then(Value::as_bool).unwrap_or(false),
        relative: get("line-number").and_then(Value::as_str) == Some("relative"),
    }
}

impl Theme {
    pub fn load() -> Theme {
        Self::from_helix().unwrap_or_else(Self::ansi)
    }

    /// Style for a Helix scope, falling back to parents (`keyword.control` → `keyword`).
    pub fn find(&self, scope: &str) -> Option<Style> {
        let mut s = scope;
        loop {
            if let Some(st) = self.styles.get(s) {
                return Some(*st);
            }
            s = s.rsplit_once('.')?.0;
        }
    }

    pub fn get(&self, scope: &str) -> Style {
        self.find(scope).unwrap_or_default()
    }

    /// Terminal palette colors, for when no Helix theme can be loaded.
    pub fn ansi() -> Theme {
        let fg = |c| Style::new().fg(c);
        let bg = |c| Style::new().bg(c);
        let m = |m| Style::new().add_modifier(m);
        let styles = [
            ("comment", fg(Color::DarkGray)),
            ("string", fg(Color::Green)),
            ("constant", fg(Color::Magenta)),
            ("constant.character.escape", fg(Color::Cyan)),
            ("variable.builtin", fg(Color::Magenta)),
            ("variable.other.member", fg(Color::Blue)),
            ("function", fg(Color::Blue)),
            ("function.macro", fg(Color::Cyan)),
            ("type", fg(Color::Yellow)),
            ("keyword", fg(Color::Red)),
            ("operator", fg(Color::Red)),
            ("markup.heading", fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            ("markup.raw", fg(Color::Cyan)),
            ("markup.bold", m(Modifier::BOLD)),
            ("markup.italic", m(Modifier::ITALIC)),
            ("markup.link.url", fg(Color::Blue).add_modifier(Modifier::UNDERLINED)),
            ("markup.link.text", fg(Color::Blue)),
            ("markup.quote", fg(Color::DarkGray)),
            ("markup.list", fg(Color::Red)),
            ("diff.plus", fg(Color::Green)),
            ("diff.minus", fg(Color::Red)),
            ("diff.delta", fg(Color::Yellow)),
            ("error", fg(Color::Red)),
            ("warning", fg(Color::Yellow)),
            ("info", fg(Color::LightBlue)),
            ("hint", fg(Color::LightCyan)),
            ("ui.linenr", fg(Color::DarkGray)),
            ("ui.selection", bg(Color::Rgb(0x3b, 0x42, 0x52))),
            ("ui.statusline", bg(Color::Rgb(0x2a, 0x2e, 0x38))),
            ("ui.menu", bg(Color::Rgb(0x45, 0x40, 0x3d))),
            ("ui.menu.selected", m(Modifier::REVERSED)),
            ("ui.virtual.jump-label", fg(Color::Magenta).add_modifier(Modifier::BOLD)),
        ];
        Theme { styles: styles.into_iter().map(|(k, v)| (k.to_owned(), v)).collect() }
    }

    fn from_helix() -> Option<Theme> {
        let dir = helix_config_dir()?;
        let cfg: Table = toml::from_str(&std::fs::read_to_string(dir.join("config.toml")).ok()?).ok()?;
        let name = match cfg.get("theme")? {
            Value::String(s) => s.clone(),
            Value::Table(t) => t.get("dark").or(t.get("fallback"))?.as_str()?.to_owned(),
            _ => return None,
        };
        let mut dirs = vec![dir.join("themes")];
        dirs.extend(runtime_dirs().into_iter().map(|d| d.join("themes")));
        let t = load_table(&name, &dirs, 0)?;
        let palette = t.get("palette").and_then(Value::as_table).cloned().unwrap_or_default();
        let styles = t
            .iter()
            .filter(|(k, _)| *k != "palette" && *k != "inherits")
            .map(|(k, v)| (k.clone(), style(v, &palette)))
            .collect();
        Some(Theme { styles })
    }

    pub fn ui(&self) -> Ui {
        let bg = self.find("ui.background").and_then(|s| s.bg);
        let base = Style { bg, ..self.get("ui.text") };
        let plus = self.get("diff.plus");
        let minus = self.get("diff.minus");
        // Line tints: the diff color faded into the background, like most diff viewers.
        let tint_on = |s: Style, under: Option<Color>, fallback| match (s.fg, under) {
            (Some(c), Some(b)) => blend(c, b, 0.18).unwrap_or(fallback),
            _ => fallback,
        };
        let tint = |s: Style, fallback| tint_on(s, bg, fallback);
        let cursorline = self.find("ui.cursorline.primary").or(self.find("ui.cursorline")).and_then(|s| s.bg);
        let (add_bg, del_bg) = (tint(plus, Color::Rgb(0x1f, 0x33, 0x22)), tint(minus, Color::Rgb(0x3a, 0x20, 0x22)));
        let or_reversed = |s: Option<Style>| match s {
            Some(s) if s.fg.is_some() || s.bg.is_some() => s,
            _ => Style::new().add_modifier(Modifier::REVERSED),
        };
        let cursor = self.find("ui.cursor.primary.normal").or(self.find("ui.cursor.primary")).or(self.find("ui.cursor"));
        let comment = self.get("comment");
        Ui {
            base,
            linenr: self.get("ui.linenr"),
            plus,
            minus,
            add_bg,
            del_bg,
            add_bg_cur: tint_on(plus, cursorline, add_bg),
            del_bg_cur: tint_on(minus, cursorline, del_bg),
            hunk: self.find("ui.virtual.inlay-hint").unwrap_or(comment),
            header: Style { fg: self.get("diff.delta").fg, ..self.get("ui.menu") }.add_modifier(Modifier::BOLD),
            selection: self.get("ui.selection"),
            cursor: or_reversed(cursor),
            cursor_select: or_reversed(self.find("ui.cursor.primary.select").or(cursor)),
            status: self.get("ui.statusline"),
            mode_normal: self.find("ui.statusline.normal").unwrap_or(self.get("ui.statusline")),
            mode_select: self.find("ui.statusline.select").unwrap_or(self.get("ui.statusline")),
            popup: self.find("ui.popup").or(self.find("ui.menu")).unwrap_or_default(),
            menu_selected: or_reversed(self.find("ui.menu.selected")),
            jump_label: self.get("ui.virtual.jump-label"),
            heading: self.get("markup.heading"),
            thread: self.get("info").add_modifier(Modifier::BOLD),
            stale: Style { fg: comment.fg, ..Style::new() },
            error: self.get("error"),
            warning: self.get("warning"),
            resolved: self.get("diff.plus"),
            pending: self.find("hint").unwrap_or(self.get("info")),
            emph_add: plus.fg.zip(bg).and_then(|(c, b)| blend(c, b, 0.42)).unwrap_or(Color::Rgb(0x2f, 0x5a, 0x33)),
            emph_del: minus.fg.zip(bg).and_then(|(c, b)| blend(c, b, 0.42)).unwrap_or(Color::Rgb(0x6b, 0x2c, 0x2c)),
            cursorline,
            key: self.find("ui.text.focus").unwrap_or(self.get("function")).add_modifier(Modifier::BOLD),
            md: crate::md::Styles {
                heading: self.get("markup.heading"),
                link: self.get("markup.link.text").add_modifier(Modifier::UNDERLINED),
                raw: self.get("markup.raw"),
                quote: self.get("markup.quote"),
                list: self.get("markup.list"),
                dim: Style { fg: comment.fg, ..Style::new() },
                code_bg: self.find("ui.popup").and_then(|s| s.bg),
            },
            linenr_selected: self.find("ui.linenr.selected").unwrap_or(self.get("ui.linenr").add_modifier(Modifier::BOLD)),
        }
    }
}

fn helix_config_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".config")))?;
    Some(base.join("helix"))
}

/// `$HELIX_RUNTIME`, then the dirs `hx --health` reports (covers packaged installs, e.g. Nix).
fn runtime_dirs() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::env::var_os("HELIX_RUNTIME").map(PathBuf::from).into_iter().collect();
    let Ok(mut child) = Command::new("hx").arg("--health").stdout(Stdio::piped()).stderr(Stdio::null()).spawn()
    else {
        return out;
    };
    if let Some(stdout) = child.stdout.take() {
        // The header comes first; stop there instead of waiting for the per-language checks.
        for line in BufReader::new(stdout).lines().map_while(Result::ok).take(10) {
            if let Some(d) = line.strip_prefix("Runtime directories: ") {
                out.extend(d.split(';').map(PathBuf::from));
                break;
            }
        }
    }
    _ = child.kill();
    _ = child.wait();
    out
}

/// A theme table with its `inherits` chain merged in (child keys win; palettes merge).
fn load_table(name: &str, dirs: &[PathBuf], depth: u8) -> Option<Table> {
    let path = dirs.iter().map(|d| d.join(format!("{name}.toml"))).find(|p| p.exists())?;
    let mut t: Table = toml::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    let parent = t.get("inherits").and_then(Value::as_str).map(str::to_owned);
    // Helix's built-in "default" theme isn't a file; inheriting it just means no base here.
    if let Some(mut base) = parent.filter(|_| depth < 8).and_then(|p| load_table(&p, dirs, depth + 1)) {
        let mut palette = base.get("palette").and_then(Value::as_table).cloned().unwrap_or_default();
        palette.extend(t.get("palette").and_then(Value::as_table).cloned().unwrap_or_default());
        base.extend(t);
        base.insert("palette".into(), Value::Table(palette));
        t = base;
    }
    Some(t)
}

fn style(v: &Value, palette: &Table) -> Style {
    let color = |v: Option<&Value>| v.and_then(Value::as_str).and_then(|s| color(s, palette));
    let mut st = Style::new();
    match v {
        Value::String(_) => st.fg = color(Some(v)),
        Value::Table(t) => {
            st.fg = color(t.get("fg"));
            st.bg = color(t.get("bg"));
            for m in t.get("modifiers").and_then(Value::as_array).into_iter().flatten() {
                st = st.add_modifier(modifier(m.as_str().unwrap_or("")));
            }
        }
        _ => {}
    }
    st
}

fn color(s: &str, palette: &Table) -> Option<Color> {
    let s = palette.get(s).and_then(Value::as_str).unwrap_or(s);
    if let Some(h) = s.strip_prefix('#') {
        let n = u32::from_str_radix(h, 16).ok().filter(|_| h.len() == 6)?;
        return Some(Color::Rgb((n >> 16) as u8, (n >> 8) as u8, n as u8));
    }
    Some(match s {
        "black" => Color::Black,
        "red" => Color::Red,
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "blue" => Color::Blue,
        "magenta" => Color::Magenta,
        "cyan" => Color::Cyan,
        "gray" => Color::DarkGray,
        "light-red" => Color::LightRed,
        "light-green" => Color::LightGreen,
        "light-yellow" => Color::LightYellow,
        "light-blue" => Color::LightBlue,
        "light-magenta" => Color::LightMagenta,
        "light-cyan" => Color::LightCyan,
        "light-gray" => Color::Gray,
        "white" => Color::White,
        "reset" | "default" => Color::Reset,
        _ => return None,
    })
}

fn modifier(m: &str) -> Modifier {
    match m {
        "bold" => Modifier::BOLD,
        "dim" => Modifier::DIM,
        "italic" => Modifier::ITALIC,
        "underlined" => Modifier::UNDERLINED,
        "slow_blink" => Modifier::SLOW_BLINK,
        "rapid_blink" => Modifier::RAPID_BLINK,
        "reversed" => Modifier::REVERSED,
        "hidden" => Modifier::HIDDEN,
        "crossed_out" => Modifier::CROSSED_OUT,
        _ => Modifier::empty(),
    }
}

/// `a` mixed into `b` by `t` (0 = b, 1 = a); RGB only.
fn blend(a: Color, b: Color, t: f32) -> Option<Color> {
    let (Color::Rgb(ar, ag, ab), Color::Rgb(br, bg, bb)) = (a, b) else { return None };
    let mix = |x: u8, y: u8| (y as f32 + (x as f32 - y as f32) * t).round() as u8;
    Some(Color::Rgb(mix(ar, br), mix(ag, bg), mix(ab, bb)))
}

#[test]
fn resolves_inherits_and_palette() {
    let dir = std::env::temp_dir().join(format!("dv-theme-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("base.toml"), "keyword = \"red1\"\n\"ui.text\" = { fg = \"fg\", modifiers = [\"bold\"] }\n[palette]\nred1 = \"#fb4934\"\nfg = \"#ebdbb2\"\n").unwrap();
    std::fs::write(dir.join("child.toml"), "inherits = \"base\"\n\"keyword.control\" = { fg = \"light-blue\" }\n[palette]\nfg = \"#000000\"\n").unwrap();
    let t = load_table("child", std::slice::from_ref(&dir), 0).unwrap();
    let pal = t["palette"].as_table().unwrap().clone();
    let th = Theme { styles: t.iter().filter(|(k, _)| *k != "palette").map(|(k, v)| (k.clone(), style(v, &pal))).collect() };
    assert_eq!(th.get("keyword.function").fg, Some(Color::Rgb(0xfb, 0x49, 0x34))); // parent scope, parent palette
    assert_eq!(th.get("keyword.control.import").fg, Some(Color::LightBlue));
    let text = th.get("ui.text");
    assert_eq!((text.fg, text.add_modifier), (Some(Color::Rgb(0, 0, 0)), Modifier::BOLD)); // child palette wins
    assert_eq!(blend(Color::Rgb(200, 0, 0), Color::Rgb(0, 0, 0), 0.5), Some(Color::Rgb(100, 0, 0)));
    std::fs::remove_dir_all(dir).unwrap();
}
