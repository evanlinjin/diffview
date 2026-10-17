use super::*;

#[test]
fn filter_and_keys() {
    let pr = |number, title: &str, author: &str, updated: &str| Listed {
        number,
        title: title.into(),
        author: author.into(),
        base: "master".into(),
        branch: "b".into(),
        state: "OPEN".into(),
        updated: updated.into(),
        additions: number,
        files: 1,
        ..Listed::default()
    };
    let mut l = List::new("-wb".into());
    l.prs = vec![pr(1, "Fix CI", "ann", "2026-03"), pr(22, "Add docs", "bo", "2026-01"), pr(333, "fix fees", "cy", "2026-02")];
    (l.prs[1].draft, l.prs[1].decision, l.prs[1].labels) = (true, "APPROVED".into(), vec![("docs".into(), "0075ca".into())]);
    l.prs[2].checks = Some((3, 1, 0));
    let k = |l: &mut List, c| l.key(KeyEvent::new(c, KeyModifiers::NONE));
    let shown = |l: &List| l.shown().iter().map(|p| p.number).collect::<Vec<_>>();
    let filter = |l: &mut List, q: &str| (l.query = q.into(), shown(l)).1;
    assert_eq!(shown(&l), [1, 333, 22]); // most recently updated first
    assert_eq!(filter(&mut l, "fix"), [1, 333]); // smart case
    assert_eq!(filter(&mut l, "Fix"), [1]);
    assert_eq!(filter(&mut l, "is:draft"), [22]);
    assert_eq!(filter(&mut l, "-is:draft review:none"), [1, 333]);
    assert_eq!(filter(&mut l, "review:appr label:doc"), [22]);
    assert_eq!(filter(&mut l, "ci:fail"), [333]);
    assert_eq!(filter(&mut l, "ci:none -author:an"), [22]);
    assert_eq!(filter(&mut l, "docs"), [22]); // labels count as text too
    l.query.clear();
    // `/` types a filter; Enter keeps it, Esc (out of filter mode) clears it.
    k(&mut l, KeyCode::Char('/'));
    "fix".chars().for_each(|c| _ = k(&mut l, KeyCode::Char(c)));
    k(&mut l, KeyCode::Enter);
    assert_eq!(shown(&l), [1, 333]);
    k(&mut l, KeyCode::Char('j'));
    assert!(matches!(k(&mut l, KeyCode::Enter), Some(Do::Open(333))));
    k(&mut l, KeyCode::Esc); // clears the filter, doesn't quit
    assert_eq!(shown(&l), [1, 333, 22]);
    assert_eq!(filter(&mut l, "#4567"), [] as [u64; 0]); // not listed (older, or closed): Enter opens it anyway
    assert!(matches!(k(&mut l, KeyCode::Enter), Some(Do::Open(4567))));
    l.query.clear();
    // Ordering keeps the selected PR selected.
    k(&mut l, KeyCode::Char('j')); // on 333
    k(&mut l, KeyCode::Char('o')); // created: all equal, so stable
    k(&mut l, KeyCode::Char('o')); // number
    assert_eq!(shown(&l), [333, 22, 1]);
    k(&mut l, KeyCode::Char('O'));
    assert_eq!(shown(&l), [1, 22, 333]);
    assert_eq!(l.shown()[l.sel].number, 333);
    // `?` shows the help; the next key only closes it.
    k(&mut l, KeyCode::Char('?'));
    assert!(l.help);
    assert!(k(&mut l, KeyCode::Char('q')).is_none() && !l.help);
    // Tab: the PRs with a worktree, including one that's merged (not in the open tab).
    l.prs.push(Listed { number: 9, state: "MERGED".into(), ..pr(9, "old", "dee", "2025-01") });
    l.local.insert(9, ("/x/r-9".into(), None));
    l.local.insert(22, ("/x/r-22".into(), Some("feat".into())));
    assert_eq!(shown(&l), [1, 22, 333]); // open tab: not the merged one
    k(&mut l, KeyCode::Tab);
    assert_eq!(shown(&l), [9, 22]); // (still by number, ascending)
    k(&mut l, KeyCode::Char('j'));
    assert!(matches!(k(&mut l, KeyCode::Enter), Some(Do::Open(22))));
    // D asks first; D again removes it; anything else in between keeps it.
    assert!(k(&mut l, KeyCode::Char('D')).is_none() && l.msg.starts_with("D again removes /x/r-22"));
    k(&mut l, KeyCode::Char('o'));
    assert!(k(&mut l, KeyCode::Char('D')).is_none());
    assert!(matches!(k(&mut l, KeyCode::Char('D')), Some(Do::Remove(22))));
    // m marks (and moves on); a asks, a again has Claude review the marked ones.
    k(&mut l, KeyCode::BackTab); // open PRs: 1, 22, 333
    k(&mut l, KeyCode::Char('g'));
    k(&mut l, KeyCode::Char('g'));
    k(&mut l, KeyCode::Char('m'));
    k(&mut l, KeyCode::Char('j')); // past 22
    k(&mut l, KeyCode::Char('m'));
    assert_eq!(l.marked, HashSet::from([1, 333]));
    assert!(k(&mut l, KeyCode::Char('a')).is_none() && l.msg.starts_with("a again: Claude reviews #1 #333"));
    assert!(matches!(k(&mut l, KeyCode::Char('a')), Some(Do::Review(ref p)) if p == &[1, 333]));
    // A failed review can be asked for again.
    l.batch.insert(1, batch::Status::Failed("boom".into()));
    l.marked = HashSet::from([1]);
    assert!(k(&mut l, KeyCode::Char('a')).is_none() && l.msg.starts_with("a again: Claude reviews #1"), "{}", l.msg);
    assert!(matches!(k(&mut l, KeyCode::Char('a')), Some(Do::Review(ref p)) if p == &[1]));
    assert_eq!(l.claude_state(1), Some(("claude failed: boom".into(), false))); // why, on its card
    (l.marked, l.batch) = (HashSet::new(), HashMap::new());
    // While reviews run, cards say so, and q asks before quitting.
    l.batch.insert(333, batch::Status::Reviewing);
    l.reviewed.insert(1);
    assert_eq!(l.claude_state(333), Some(("claude: reviewing…".into(), true)));
    assert_eq!(l.claude_state(1), Some(("claude ✓".into(), true)));
    assert!(k(&mut l, KeyCode::Char('q')).is_none() && l.msg.starts_with("Claude is still reviewing"));
    assert!(matches!(k(&mut l, KeyCode::Char('q')), Some(Do::Quit)));
    // A A drafts one replacement for the marked PRs (or this one).
    l.marked = HashSet::from([22, 1]);
    assert!(k(&mut l, KeyCode::Char('A')).is_none() && l.msg.starts_with("A again: Claude suggests how to group #1 #22"));
    assert!(matches!(k(&mut l, KeyCode::Char('A')), Some(Do::Replace(ref p)) if p == &[1, 22]));
    // The replacements tab: its cards, and the PRs it replaces say so.
    let o = |number| crate::replace::Original { number, author: "bot".into(), ..Default::default() };
    let r = crate::replace::Record { id: 1, replaces: vec![o(1), o(22)], title: "Do it properly".into(), ..Default::default() };
    l.reps = vec![Listed { number: 1, title: r.title.clone(), ..Listed::default() }];
    l.rep_of.insert(1, r);
    assert_eq!(l.claude_state(22), Some(("replacement ✓".into(), true)));
    l.drafting.insert(1, batch::Status::Reviewing);
    assert_eq!(l.draft_state(1, "claude"), Some(("claude: drafting…".into(), true)));
    k(&mut l, KeyCode::Tab);
    k(&mut l, KeyCode::Tab);
    assert_eq!(l.tab, Tab::Replacements);
    assert_eq!(shown(&l), [1]);
    assert!(k(&mut l, KeyCode::Char('a')).is_none() && l.msg.starts_with("a replacement: Enter opens it"));
    assert!(k(&mut l, KeyCode::Char('D')).is_none());
    assert!(matches!(k(&mut l, KeyCode::Char('D')), Some(Do::RemoveReplacement(1))));
}

#[test]
fn selected_card_readable() {
    use ratatui::style::Color;
    let mut ui = crate::theme::Theme::load().ui();
    (ui.menu_selected, ui.stale) = (Style::new().fg(Color::Black).bg(Color::Cyan), Style::new().fg(Color::Gray));
    let cx = Cards { ui: &ui, num_w: 5, width: 120, checks_loading: false, usual: "main" };
    let p = Listed { number: 1, title: "t".into(), author: "amy".into(), labels: vec![("bug".into(), "d73a4a".into())], ..Listed::default() };
    let mark = || Mark { marked: false, claude: Some(("claude ✓".into(), true)), new: true, replaces: None };
    let [top, bottom] = card(&p, &cx, true, None, mark());
    for line in [&top, &bottom] {
        assert_eq!(line.width(), 120); // the highlight runs to the edge
        // The highlight's own text color throughout, but for the label chip.
        assert!(line.spans.iter().filter(|s| s.content != " bug ").all(|s| s.style.fg == Some(Color::Black)), "{line:?}");
    }
    let [_, bottom] = card(&p, &cx, false, None, mark());
    assert!(bottom.spans.iter().any(|s| s.style.fg == Some(Color::Gray))); // unselected: its colors
    // [new] at the right end of the top line, before the age.
    let [top, _] = card(&p, &cx, false, None, mark());
    assert_eq!(top.width(), 120);
    assert_eq!(top.spans[top.spans.len() - 2].content, "[new] ");
    let [top, _] = card(&p, &cx, false, None, Mark { new: false, ..mark() });
    assert!(!top.spans.iter().any(|s| s.content.contains("[new]")));
    // Claude's part at the right end, even when the rest has to be cut for it.
    assert_eq!(bottom.spans.last().unwrap().content, "claude ✓ ");
    let narrow = Cards { width: 30, ..cx };
    let p = Listed { branch: "a-very-long-branch-name-indeed".into(), ..p };
    let [_, bottom] = card(&p, &narrow, false, None, mark());
    assert_eq!(bottom.width(), 30);
    assert_eq!(bottom.spans.last().unwrap().content, "claude ✓ ");
    assert!(!bottom.spans.iter().rev().nth(2).is_some_and(|s| s.content == " · ")); // no separator left dangling
}

#[test]
fn marks_saved() {
    let dir = std::env::temp_dir().join(format!("dv-marks-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("sub/marked.json");
    assert!(read_marks(&p).is_empty()); // none yet
    write_marks(&p, &HashSet::from([22, 3])).unwrap();
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "[3,22]");
    assert_eq!(read_marks(&p), HashSet::from([3, 22]));
    // Another dv list marked 7 meanwhile; this one, which had 3 and 22, unmarks 3 and marks 9.
    write_marks(&p, &HashSet::from([3, 22, 7])).unwrap();
    let now = merge_marks(&p, &HashSet::from([3, 22]), &HashSet::from([22, 9])).unwrap();
    assert_eq!(now, HashSet::from([22, 7, 9]));
    _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn screens() {
    use crate::view::tests::{screen, snapshot};
    let pr = |number, title: &str, author: &str, updated: &str| Listed {
        number,
        title: title.into(),
        author: author.into(),
        base: "master".into(),
        branch: "b".into(),
        state: "OPEN".into(),
        updated: updated.into(), // not a time: shown as it is, so the snapshot doesn't age
        additions: number,
        files: 1,
        ..Listed::default()
    };
    let mut l = List::new("".into());
    l.prs = vec![pr(1, "Fix CI", "ann", "2026-03"), pr(22, "Add docs", "bo", "2026-01"), pr(333, "fix fees", "cy", "2026-02")];
    (l.prs[1].draft, l.prs[1].decision, l.prs[1].labels) = (true, "APPROVED".into(), vec![("docs".into(), "0075ca".into())]);
    l.prs[2].checks = Some((3, 1, 0));
    l.marked = HashSet::from([333]);
    l.new = HashSet::from([22]);
    l.reviewed = HashSet::from([1]);
    let k = |l: &mut List, c| l.key(KeyEvent::new(c, KeyModifiers::NONE));
    k(&mut l, KeyCode::Char('j'));
    snapshot("list", &screen(100, 12, |f| l.draw(f)));
    l.query = "fix".into();
    snapshot("list_filtered", &screen(100, 12, |f| l.draw(f)));
    l.query.clear();
    k(&mut l, KeyCode::Char('?'));
    snapshot("list_help", &screen(100, 60, |f| l.draw(f)));
    k(&mut l, KeyCode::Esc);
    k(&mut l, KeyCode::Tab);
    snapshot("list_worktrees", &screen(100, 8, |f| l.draw(f)));
    k(&mut l, KeyCode::Tab);
    snapshot("list_replacements", &screen(100, 8, |f| l.draw(f)));
}
