//! Syntax highlighting with syntect, mapped to named terminal colors so the terminal theme decides.
use ratatui::style::Color;
use syntect::parsing::{ParseState, Scope, ScopeStack, SyntaxSet};
use syntect::util::LinesWithEndings;

/// One color (or none) per char, per line.
pub type Colors = Vec<Vec<Option<Color>>>;

pub struct Highlighter {
    ss: SyntaxSet,
    rules: Vec<(Scope, Color)>,
}

impl Highlighter {
    pub fn new() -> Self {
        // Checked innermost scope first; within a scope, first matching prefix wins.
        let rules = [
            ("comment", Color::DarkGray),
            ("string", Color::Green),
            ("constant.character.escape", Color::Cyan),
            ("constant", Color::Magenta),
            ("variable.language", Color::Magenta),
            ("support.macro", Color::Cyan),
            ("entity.name.macro", Color::Cyan),
            ("entity.name.function", Color::Blue),
            ("support.function", Color::Blue),
            ("variable.function", Color::Blue),
            ("entity.name", Color::Yellow),
            ("support.type", Color::Yellow),
            ("support.class", Color::Yellow),
            ("storage.type", Color::Red),
            ("storage", Color::Red),
            ("keyword", Color::Red),
        ];
        let rules = rules.iter().filter_map(|&(s, c)| Some((Scope::new(s).ok()?, c))).collect();
        Highlighter { ss: SyntaxSet::load_defaults_newlines(), rules }
    }

    /// Highlight a whole file; empty if the language isn't known.
    pub fn file(&self, path: &str, text: &str) -> Colors {
        let ext = path.rsplit('.').next().unwrap_or("");
        let first = text.lines().next().unwrap_or("");
        let Some(syntax) = self.ss.find_syntax_by_extension(ext).or_else(|| self.ss.find_syntax_by_first_line(first))
        else {
            return vec![];
        };
        let (mut ps, mut stack) = (ParseState::new(syntax), ScopeStack::new());
        let mut out = vec![];
        for line in LinesWithEndings::from(text) {
            let Ok(ops) = ps.parse_line(line, &self.ss) else { return vec![] };
            let mut colors = vec![];
            let mut prev = 0;
            for (i, op) in ops {
                let c = self.color(&stack);
                colors.extend(line[prev..i].chars().map(|_| c));
                prev = i;
                if stack.apply(&op).is_err() {
                    return vec![];
                }
            }
            let c = self.color(&stack);
            colors.extend(line[prev..].trim_end_matches(['\n', '\r']).chars().map(|_| c));
            colors.truncate(line.trim_end_matches(['\n', '\r']).chars().count());
            out.push(colors);
        }
        out
    }

    fn color(&self, stack: &ScopeStack) -> Option<Color> {
        stack.as_slice().iter().rev().find_map(|s| self.rules.iter().find(|(r, _)| r.is_prefix_of(*s)).map(|r| r.1))
    }
}

#[test]
fn highlights_rust() {
    let h = Highlighter::new();
    let c = h.file("a.rs", "// hi\nfn main() { let s = \"x\"; }\n");
    assert_eq!(c[0][0], Some(Color::DarkGray));
    assert_eq!(c[1][0], Some(Color::Red)); // fn
    assert_eq!(c[1][3], Some(Color::Blue)); // main
    let q = "fn main() { let s = ".chars().count();
    assert_eq!(c[1][q], Some(Color::Green)); // "x"
    assert_eq!(c[1].len(), "fn main() { let s = \"x\"; }".chars().count());
    assert!(h.file("a.unknownext", "x").is_empty());
}
