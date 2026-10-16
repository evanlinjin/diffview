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
        match ss.find_syntax_by_extension(ext).or_else(|| ss.find_syntax_by_first_line(first)) {
            Some(syntax) => self.highlight(ss, syntax, text),
            None => vec![],
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
