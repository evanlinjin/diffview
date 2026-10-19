//! Syntax highlighting with syntect, mapped to named terminal colors so the terminal theme decides.
use crate::theme::Theme;
use ratatui::style::Style;
#[cfg(test)]
use ratatui::style::Color;
use syntect::parsing::{ParseState, Scope, ScopeStack, SyntaxDefinition, SyntaxSet};
use syntect::util::LinesWithEndings;

/// One style (or none) per char, per line.
pub type Colors = Vec<Vec<Option<Style>>>;

/// syntect's bundled grammars have no TOML; this covers Cargo.toml-style files.
const TOML: &str = r#"%YAML 1.2
---
name: TOML
file_extensions: [toml]
scope: source.toml
contexts:
  main:
    - match: '#.*$'
      scope: comment.line.number-sign.toml
    - match: '^\s*(\[\[?)([^\]#]*)(\]\]?)'
      captures:
        1: punctuation.definition.table.toml
        2: entity.name.section.toml
        3: punctuation.definition.table.toml
    - match: '"""'
      push:
        - meta_scope: string.quoted.triple.toml
        - match: '"""'
          pop: true
    - match: "'''"
      push:
        - meta_scope: string.quoted.triple.toml
        - match: "'''"
          pop: true
    - match: '"'
      push:
        - meta_scope: string.quoted.double.toml
        - match: '\\.'
          scope: constant.character.escape.toml
        - match: '"|$'
          pop: true
    - match: "'"
      push:
        - meta_scope: string.quoted.single.toml
        - match: "'|$"
          pop: true
    - match: '\b(true|false|inf|nan)\b'
      scope: constant.language.toml
    - match: '\b\d{4}-\d{2}-\d{2}([T ][\d:.]+)?(Z|[+-]\d{2}:\d{2})?'
      scope: constant.other.datetime.toml
    - match: '[+-]?\b(0x[0-9A-Fa-f_]+|0o[0-7_]+|0b[01_]+|\d[\d_]*(\.\d[\d_]*)?([eE][+-]?\d+)?)\b'
      scope: constant.numeric.toml
    - match: '([A-Za-z0-9_-]+)\s*(?=[.=])'
      captures:
        1: variable.other.key.toml
"#;

pub struct Highlighter {
    ss: SyntaxSet,
    /// Separate set: adding TOML to the bundled one re-links everything (~200ms at startup).
    toml: SyntaxSet,
    rules: Vec<(Scope, Style)>,
}

/// With the plain terminal colors (`Theme::ansi`), until dv has its theme.
impl Default for Highlighter {
    fn default() -> Self {
        Highlighter::new(&crate::theme::Theme::ansi())
    }
}

impl Highlighter {
    pub fn new(theme: &Theme) -> Self {
        // syntect scope prefix → Helix scope. Checked innermost syntect scope first; within a
        // scope, the first matching prefix wins. "" means "no style" (stops the search).
        let rules = [
            // Markdown. Fence lines are dimmed; fenced code is re-highlighted in its language.
            ("punctuation.definition.raw.code-fence", "comment"),
            ("markup.raw.code-fence", ""),
            ("markup.raw", "markup.raw"),
            ("markup.heading", "markup.heading"),
            ("entity.name.section.markdown", "markup.heading"),
            ("markup.bold", "markup.bold"),
            ("markup.italic", "markup.italic"),
            ("markup.underline.link", "markup.link.url"),
            ("meta.link.inline.description", "markup.link.text"),
            ("markup.quote", "markup.quote"),
            ("punctuation.definition.list_item", "markup.list"),
            // TOML.
            ("entity.name.section.toml", "type"),
            ("variable.other.key", "variable.other.member"),
            // Code.
            ("comment", "comment"),
            ("string", "string"),
            ("constant.character.escape", "constant.character.escape"),
            ("constant.numeric", "constant.numeric"),
            ("constant", "constant"),
            ("variable.language", "variable.builtin"),
            ("support.macro", "function.macro"),
            ("entity.name.macro", "function.macro"),
            ("entity.name.function", "function"),
            ("support.function", "function"),
            ("variable.function", "function"),
            ("entity.name", "type"),
            ("support.type", "type"),
            ("support.class", "type"),
            ("storage", "keyword"),
            ("keyword.operator", "operator"),
            ("keyword.control", "keyword.control"),
            ("keyword", "keyword"),
            // Quote and comment markers belong to their string/comment, as in Helix.
            ("punctuation.definition.string", "string"),
            ("punctuation.definition.comment", "comment"),
            ("punctuation", "punctuation"),
        ];
        let rules = rules
            .iter()
            .filter_map(|&(s, h)| Some((Scope::new(s).ok()?, if h.is_empty() { Style::new() } else { theme.get(h) })))
            .collect();
        let mut b = syntect::parsing::SyntaxSetBuilder::new();
        b.add(SyntaxDefinition::load_from_str(TOML, true, None).expect("built-in TOML grammar"));
        Highlighter { ss: SyntaxSet::load_defaults_newlines(), toml: b.build(), rules }
    }

    /// Highlight a whole file; empty if the language isn't known.
    pub fn file(&self, path: &str, text: &str) -> Colors {
        let ext = path.rsplit('.').next().unwrap_or("");
        let first = text.lines().next().unwrap_or("");
        let ss = if ext == "toml" { &self.toml } else { &self.ss };
        let mut colors = match ss.find_syntax_by_extension(ext).or_else(|| ss.find_syntax_by_first_line(first)) {
            Some(syntax) => self.highlight(ss, syntax, text),
            None => return vec![],
        };
        // Code blocks in it: in their own language (Markdown's, Rust's doc comments').
        match ext {
            "md" | "markdown" => self.fences(text, &mut colors, |l| Some(l), None),
            "rs" => self.fences(text, &mut colors, doc_line, Some("rust")),
            _ => {}
        }
        colors
    }

    /// Fenced code blocks (```` ```lang ````) in the lines `inner` finds text in (Markdown's:
    /// the line; a doc comment's: past its `///`), highlighted as their language (its first
    /// word: `rust,no_run` is Rust), over `colors`. `default`: for a fence without one, or with
    /// rustdoc's attributes only (`ignore`, `should_panic`, ...).
    fn fences(&self, text: &str, colors: &mut Colors, inner: fn(&str) -> Option<&str>, default: Option<&str>) {
        let lines: Vec<&str> = text.lines().collect();
        // (A backtick fence's info has no backtick: "```foo``` bar" is inline code, no fence.)
        let fence = |l| {
            inner(l).map(str::trim_start).filter(|t: &&str| t.starts_with("~~~") || t.starts_with("```") && !t.trim_start_matches('`').contains('`'))
        };
        let mut i = 0;
        while i < lines.len() {
            let Some(open) = fence(lines[i]) else {
                i += 1;
                continue;
            };
            // Its run of ` or ~: what closes it is a line of as many at least, and nothing else
            // (as CommonMark has it: a ```` ```` ```` block can hold a ```` ``` ```` one).
            let mark = open.as_bytes()[0] as char;
            let run = open.len() - open.trim_start_matches(mark).len();
            let token = fence_lang(&open[run..]);
            let closes = |l: &str| inner(l).map(str::trim).is_some_and(|t| t.len() >= run && t.chars().all(|c| c == mark));
            let body: Vec<usize> = (i + 1..lines.len()).take_while(|&k| inner(lines[k]).is_some() && !closes(lines[k])).collect();
            let code: String = body.iter().map(|&k| format!("{}\n", inner(lines[k]).unwrap_or(""))).collect();
            let mut c = self.lang(token, &code);
            // (rustdoc's attributes in place of a language: still Rust; `text` isn't.)
            let attr = token.is_empty() || ["ignore", "no_run", "should_panic", "compile_fail", "edition"].iter().any(|a| token.starts_with(a));
            if c.is_empty() && attr && let Some(d) = default {
                c = self.lang(d, &code);
            }
            for (&k, lc) in body.iter().zip(c) {
                let at = lines[k].chars().count() - inner(lines[k]).map_or(0, |t| t.chars().count());
                if let Some(row) = colors.get_mut(k) {
                    for (j, s) in lc.into_iter().enumerate() {
                        if let Some(slot) = row.get_mut(at + j) {
                            *slot = s;
                        }
                    }
                }
            }
            i += body.len() + 2; // past its closing line (or what ended it)
        }
    }

    /// Highlight by a fence's language token ("rust", "rs", "toml", "sh", ...).
    pub fn lang(&self, token: &str, text: &str) -> Colors {
        [&self.toml, &self.ss]
            .into_iter()
            .find_map(|ss| Some((ss, ss.find_syntax_by_token(token).filter(|_| !token.is_empty())?)))
            .map_or(vec![], |(ss, syntax)| self.highlight(ss, syntax, text))
    }

    fn highlight(&self, ss: &SyntaxSet, syntax: &syntect::parsing::SyntaxReference, text: &str) -> Colors {
        let (mut ps, mut stack) = (ParseState::new(syntax), ScopeStack::new());
        let mut out = vec![];
        for line in LinesWithEndings::from(text) {
            let Ok(ops) = ps.parse_line(line, ss) else { return vec![] };
            let mut colors = vec![];
            let mut prev = 0;
            for (i, op) in ops {
                let c = self.style(&stack);
                colors.extend(line[prev..i].chars().map(|_| c));
                prev = i;
                if stack.apply(&op).is_err() {
                    return vec![];
                }
            }
            let c = self.style(&stack);
            colors.extend(line[prev..].chars().map(|_| c));
            colors.truncate(line.trim_end_matches(['\n', '\r']).chars().count());
            out.push(colors);
        }
        out
    }

    fn style(&self, stack: &ScopeStack) -> Option<Style> {
        stack.as_slice().iter().rev().find_map(|s| self.rules.iter().find(|(r, _)| r.is_prefix_of(*s)).map(|r| r.1))
    }
}

/// A code fence's language: its info string's first word (`rust,no_run` → `rust`, `{.rust}` →
/// `.rust`'s `rust`). The one rule, for files' code blocks and comments' alike.
pub fn fence_lang(info: &str) -> &str {
    info.trim().trim_start_matches(['{', '.']).split([',', ' ', '{', '}']).next().unwrap_or("").trim()
}

/// A Rust doc comment's text: past `///` or `//!` (and the one space after).
fn doc_line(l: &str) -> Option<&str> {
    let t = l.trim_start();
    let rest = t.strip_prefix("///").or_else(|| t.strip_prefix("//!"))?;
    Some(rest.strip_prefix(' ').unwrap_or(rest))
}

#[test]
fn code_blocks_in_markdown_and_doc_comments() {
    let h = Highlighter::new(&Theme::ansi());
    let fg = |c: &Option<Style>| c.and_then(|s| s.fg);
    // Markdown: a fence with rustdoc's attributes is still Rust.
    let c = h.file("README.md", "# Usage\n\n```rust,no_run\nfn main() {}\n```\n");
    assert_eq!(fg(&c[3][0]), Some(Color::Red)); // fn
    // A doc comment: past its `/// `, a fence without a language is Rust.
    let c = h.file("a.rs", "/// ```\n/// let x = 1;\n/// ```\nfn f() {}\n");
    assert_eq!(fg(&c[1][4]), Some(Color::Red)); // let
    assert_eq!(fg(&c[3][0]), Some(Color::Red)); // fn, after it
    let c = h.file("a.rs", "/// ```text\n/// let x = 1;\n/// ```\n");
    assert_ne!(fg(&c[1][4]), Some(Color::Red)); // text: not Rust
    // An empty block doesn't put the next out of step; a ```` ```` ```` block holds a ``` one.
    let c = h.file("a.md", "```\n```\n\n````md\n```rust\n````\n\n```rust\nfn f() {}\n```\n");
    assert_eq!(fg(&c[8][0]), Some(Color::Red)); // fn, in the last block
    assert_eq!(fence_lang("{.rust}"), "rust");
    // Inline code at a line's start isn't a fence.
    let c = h.file("a.md", "```foo``` bar\n\n```rust\nfn f() {}\n```\n");
    assert_eq!(fg(&c[3][0]), Some(Color::Red));
}

#[test]
fn highlights() {
    let h = Highlighter::new(&Theme::ansi());
    let fg = |c: &Option<Style>| c.and_then(|s| s.fg);
    let c = h.file("a.rs", "// hi\nfn main() { let s = \"x\"; }\n");
    assert_eq!(fg(&c[0][0]), Some(Color::DarkGray));
    assert_eq!(fg(&c[1][0]), Some(Color::Red)); // fn
    assert_eq!(fg(&c[1][3]), Some(Color::Blue)); // main
    let q = "fn main() { let s = ".chars().count();
    assert_eq!(fg(&c[1][q]), Some(Color::Green)); // "x"
    assert_eq!(c[1].len(), "fn main() { let s = \"x\"; }".chars().count());
    assert!(h.file("a.unknownext", "x").is_empty());

    let t = h.file("Cargo.toml", "[dependencies]\nserde = { version = \"1\", default-features = false } # x\n");
    assert_eq!(fg(&t[0][1]), Some(Color::Yellow)); // dependencies
    assert_eq!(fg(&t[1][0]), Some(Color::Blue)); // serde
    assert_eq!(fg(&t[1][20]), Some(Color::Green)); // "1"
    assert_eq!(fg(&t[1][44]), Some(Color::Magenta)); // false
    assert_eq!(fg(t[1].last().unwrap()), Some(Color::DarkGray)); // comment

}
