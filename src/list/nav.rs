//! Where you are in a list of cards (dv list's PRs, the issues): the card selected and the first
//! shown, the filter, the help, the tab; and the keys and clicks that move about it, the same in
//! both.
use super::*;

/// A list's tabs, in order (round from the last to the first).
pub(super) trait Tabs: Copy + Default + PartialEq {
    fn next(self) -> Self;
    fn prev(self) -> Self;
}

#[derive(Default)]
pub(super) struct Nav<T> {
    /// The card selected, and the first shown.
    pub(super) sel: usize,
    pub(super) top: usize,
    /// The `/` filter, and whether it's being typed (Enter keeps it, Esc clears it).
    pub(super) query: String,
    pub(super) filtering: bool,
    /// The help shown (any key closes it).
    pub(super) help: bool,
    pub(super) tab: T,
    /// The tabs' columns on the top bar (for clicks), and the last click on a card (when, which:
    /// a second soon after on the same one opens it).
    pub(super) tab_hits: Vec<(std::ops::Range<u16>, T)>,
    pub(super) last_click: Option<(std::time::Instant, usize)>,
    /// `g` pressed, for the key after (`gg`, `ge`, `gn`, `gp`).
    pub(super) g: bool,
    /// How many cards the last draw had room for (`keep`): a page, for the keys and clicks.
    pub(super) page: usize,
}

/// How many cards of `rows` lines fit a window `height` high (under the top bar, over the
/// status bar).
pub(super) fn cards_fit(rows: usize, height: u16) -> usize {
    height.saturating_sub(2) as usize / rows.max(1)
}

impl<T: Tabs> Nav<T> {
    /// Where the selected card is drawn in `area` (`rows` lines each, under the top bar): what the
    /// key hints keep off.
    pub(super) fn card(&self, rows: usize, area: Rect) -> Rect {
        Rect { y: area.y + 1 + (self.sel.saturating_sub(self.top) * rows) as u16, height: rows as u16, ..area }
    }

    /// The status bar's left, as the filter has it: typing it (`fields`: what it takes), what's
    /// said (`msg`), the filter kept; else `idle`.
    pub(super) fn status(&self, msg: &str, fields: &str, idle: &str) -> String {
        match (self.filtering, self.query.is_empty(), msg.is_empty()) {
            (true, ..) => format!(" /{}▏  {fields}  -x negates", self.query),
            (_, _, false) => format!(" {msg}"),
            (_, false, _) => format!(" filter: {}  (Esc clears)", self.query),
            _ => idle.into(),
        }
    }

    /// To `tab`, at its first card.
    pub(super) fn go(&mut self, tab: T) {
        (self.tab, self.sel, self.top) = (tab, 0, 0);
    }

    /// The selection on one of `n` cards.
    pub(super) fn clamp(&mut self, n: usize) {
        self.sel = self.sel.min(n.saturating_sub(1));
    }

    /// The next card of `n` (`down`), else the one before (`j` / `k`, the wheel).
    pub(super) fn step(&mut self, down: bool, n: usize) {
        self.sel = if down { self.sel + 1 } else { self.sel.saturating_sub(1) };
        self.clamp(n);
    }

    /// The selection on one of `n` cards, `h` of them shown: on screen (its first shown, as near
    /// as it was).
    pub(super) fn keep(&mut self, n: usize, h: usize) {
        self.clamp(n);
        self.page = h;
        self.top = self.top.min(self.sel).max((self.sel + 1).saturating_sub(h));
    }

    /// `k` while the filter's being typed: what it does to it (`true`: it was).
    pub(super) fn filter_key(&mut self, k: KeyEvent) -> bool {
        if !self.filtering {
            return false;
        }
        match k.code {
            KeyCode::Esc => (self.query.clear(), self.filtering = false).1,
            KeyCode::Enter => self.filtering = false,
            KeyCode::Backspace => _ = self.query.pop(),
            KeyCode::Char(c) => self.query.push(c),
            _ => {}
        }
        self.sel = 0;
        true
    }

    /// `k` on `n` cards: moving about them (a page: as many as were shown) and the tabs, the
    /// filter (Esc clears it), the help (`true`: it was one of those). `g` waits for the key
    /// after it; Esc calls it off.
    pub(super) fn key(&mut self, k: KeyEvent, n: usize) -> bool {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let (last, half) = (n.saturating_sub(1), (self.page / 2).max(1));
        let g = std::mem::take(&mut self.g);
        match (g, k.code) {
            (_, KeyCode::Char('d')) if ctrl => self.sel = (self.sel + half).min(last),
            (_, KeyCode::Char('u')) if ctrl => self.sel = self.sel.saturating_sub(half),
            (_, KeyCode::Char('j') | KeyCode::Down) => self.step(true, n),
            (_, KeyCode::Char('k') | KeyCode::Up) => self.step(false, n),
            (false, KeyCode::Char('g')) => self.g = true,
            (true, KeyCode::Esc) => {}
            (false, KeyCode::Esc) if !self.query.is_empty() => (self.query.clear(), self.sel = 0).1,
            (true, KeyCode::Char('g')) => self.sel = 0,
            (true, KeyCode::Char('e')) => self.sel = last,
            // The next / previous tab: `l` / `h`, or as in dv, `gn` / `gp`.
            (true, KeyCode::Char('n')) | (false, KeyCode::Char('l')) => self.go(self.tab.next()),
            (true, KeyCode::Char('p')) | (false, KeyCode::Char('h')) => self.go(self.tab.prev()),
            (_, KeyCode::Char('/')) => self.filtering = true, // (edits the filter there is)
            (_, KeyCode::Char('?')) => self.help = true,
            _ => return false,
        }
        true
    }

    /// A left click at (`x`, `y`) on the top bar's tabs or on a card (`n`, `rows` lines each,
    /// as shown under the bar): that tab, or that card selected (`true`: a second click soon on
    /// the same card, to open it).
    pub(super) fn click(&mut self, x: u16, y: u16, n: usize, rows: usize) -> bool {
        if y == 0 {
            if let Some(&(_, tab)) = self.tab_hits.iter().find(|(r, _)| r.contains(&x)) {
                self.go(tab);
            }
            return false;
        }
        let k = (y as usize - 1) / rows.max(1);
        let i = self.top + k;
        if k >= self.page || i >= n {
            return false;
        }
        let double = self.last_click.is_some_and(|(at, j)| j == i && at.elapsed() < Duration::from_millis(400));
        self.last_click = if double { None } else { Some((std::time::Instant::now(), i)) };
        self.sel = i;
        double
    }
}

/// (Three tabs, for tests.)
#[cfg(test)]
impl Tabs for u8 {
    fn next(self) -> u8 {
        (self + 1) % 3
    }
    fn prev(self) -> u8 {
        (self + 2) % 3
    }
}

#[test]
fn moving_about_a_list() {
    let key = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);
    let mut nav = Nav::<u8> { page: 2, ..Default::default() };
    // j past the end stays on the last; gg, ge; l / h round the tabs, at their first card.
    (0..5).for_each(|_| _ = nav.key(key('j'), 3));
    assert_eq!(nav.sel, 2);
    assert!(nav.key(key('g'), 3) && nav.key(key('g'), 3) && nav.sel == 0);
    assert!(nav.key(key('g'), 3) && nav.key(key('e'), 3) && nav.sel == 2);
    assert!(nav.key(key('h'), 3) && (nav.tab, nav.sel) == (2, 0));
    // g, then Esc: called off (the next h is a tab again, not after a g).
    let esc = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
    assert!(nav.key(key('g'), 3) && nav.key(esc, 3) && nav.key(key('l'), 3) && nav.tab == 0);
    assert!(!nav.key(key('x'), 3) && !nav.key(esc, 3));
    // The filter takes what's typed till Enter.
    assert!(nav.key(key('/'), 3) && nav.filter_key(key('a')) && nav.query == "a");
    assert!(nav.filter_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)) && !nav.filter_key(key('b')));
    // Esc, the filter kept: cleared.
    assert!(nav.key(esc, 3) && nav.query.is_empty());
    // A click selects a card (3 rows each, under the bar); a second on it soon opens it.
    nav.page = 4;
    assert!(!nav.click(0, 4, 5, 3) && nav.sel == 1);
    assert!(nav.click(0, 5, 5, 3));
    // Kept on screen: 10 cards, 4 shown, the last selected.
    nav.sel = 9;
    nav.keep(10, 4);
    assert_eq!(nav.top, 6);
}
