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
        created: updated.into(),
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
    assert_eq!(shown(&l), [1, 333, 22]); // newest first
    assert!(matches!(l.order, Order::Created)); // by default
    l.order = Order::Updated; // what the rest goes on from (`o` steps from it)
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
    assert!(k(&mut l, KeyCode::Char('a')).is_none() && l.msg.starts_with("a draft: Enter opens it"));
    assert!(k(&mut l, KeyCode::Char('D')).is_none());
    assert!(matches!(k(&mut l, KeyCode::Char('D')), Some(Do::RemoveReplacement(1))));
}

#[test]
fn selected_card_readable() {
    use ratatui::style::Color;
    let mut ui = crate::theme::Theme::load().ui();
    (ui.menu_selected, ui.stale, ui.cursorline) = (Style::new().fg(Color::Black).bg(Color::Cyan), Style::new().fg(Color::Gray), Some(Color::Blue));
    let cx = Cards { ui: &ui, num_w: 5, width: 120, checks_loading: false, usual: "main" };
    let p = Listed { number: 1, title: "t".into(), author: "amy".into(), checks: Some((3, 1, 0)), labels: vec![("bug".into(), "d73a4a".into())], ..Listed::default() };
    let mark = || Mark { marked: false, claude: Some(("claude ✓".into(), true)), new: true, replaces: None, awaiting: 0 };
    let card_of = |selected, cx: &Cards| card(&p, cx, selected, None, mark());
    // Wide: three lines in two zones, what it is on the left, how it stands on the right.
    let lines = card_of(true, &cx);
    assert_eq!(lines.len(), 3);
    for line in &lines {
        assert_eq!(line.width(), 120); // the highlight runs to the edge
        // The selected card: a quiet background (the cursor line's), a bar down its left edge…
        assert_eq!(line.spans[0].content, "▌");
        assert!(line.spans.iter().filter(|s| s.content != " bug ").all(|s| s.style.bg == Some(Color::Blue)), "{line:?}");
    }
    // …and its text keeps its colors (not the menu highlight's).
    assert!(lines[1].spans.iter().any(|s| s.style.fg == Some(Color::Gray)) && !lines.iter().flat_map(|l| &l.spans).any(|s| s.style.fg == Some(Color::Black)));
    let text = |l: &Line| l.spans.iter().map(|s| s.content.as_ref()).collect::<String>();
    let lines = card_of(false, &cx);
    assert!(lines.iter().all(|l| l.spans[0].content == " ")); // no bar
    // The right zone starts at the same column on every line (CI and age; review; Claude), and
    // [new] follows the title.
    let at = |l: &Line, what: &str| text(l).find(what).map(|b| text(l)[..b].chars().count());
    let right = at(&lines[0], "CI ✓3 ✗1").unwrap();
    assert_eq!(at(&lines[2], "claude ✓"), Some(right));
    assert!(text(&lines[0]).contains("t  [new]"));
    assert!(!text(&card(&p, &cx, false, None, Mark { new: false, ..mark() })[0]).contains("[new]"));
    // Narrow: four lines, how it stands on its own (the third), each cut at the width.
    let narrow = Cards { width: 80, ..cx };
    let lines = card(&p, &narrow, false, None, mark());
    assert_eq!(lines.len(), 4);
    assert!(text(&lines[2]).contains("CI ✓3 ✗1") && text(&lines[2]).contains("claude ✓"));
    let p = Listed { branch: "a-very-long-branch-name-indeed-and-then-some-more-and-more-and-more".into(), ..p };
    assert!(card(&p, &Cards { width: 30, ..cx }, false, None, mark()).iter().all(|l| l.width() == 30));
    assert!(card(&p, &cx, false, None, mark()).iter().all(|l| l.width() == 120));
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

#[test]
fn reviews_from_a_pr_view_show_up() {
    let dir = std::env::temp_dir().join(format!("dv-list-reviews-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut l = List::new("".into());
    l.reviews_dir = Some(dir.clone());
    l.watch_reviews();
    assert_eq!(l.claude_state(7), None);
    // Claude, in a PR's view (or its pane), writes the review while the list is up.
    std::thread::sleep(std::time::Duration::from_millis(20)); // a newer mtime
    std::fs::write(dir.join("pr-7.session"), "").unwrap();
    std::fs::write(dir.join("pr-7.json"), "{}").unwrap();
    l.watch_reviews();
    assert_eq!(l.claude_state(7), Some(("claude ✓".into(), true)));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn todo_tab() {
    use crate::view::tests::{screen, snapshot};
    let pr = |number, title: &str, author: &str| Listed {
        number,
        title: title.into(),
        author: author.into(),
        base: "master".into(),
        branch: "b".into(),
        state: "OPEN".into(),
        updated: format!("2026-0{}", number % 9),
        files: 1,
        ..Listed::default()
    };
    let dir = std::env::temp_dir().join(format!("dv-todo-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("reviews")).unwrap();
    let mut l = List::new("".into());
    (l.reviews_dir, l.ranked, l.deferred, l.deferred_file) = (Some(dir.join("reviews")), None, Default::default(), None);
    l.prs = vec![pr(1, "Fix fees", "amy"), pr(2, "Docs", "bo"), pr(3, "Mine", "me")];
    l.me = "me".into();
    let extra = |requested| super::Extra { requested, mergeable: "MERGEABLE".into(), head_oid: "h".into(), ..Default::default() };
    l.extra = HashMap::from([(1, extra(false)), (2, extra(true)), (3, extra(false))]);
    // #3: Claude answered you there.
    let review = r#"{"head": "h", "summary": "ok", "comments": [{"path": "a.rs", "line": 1, "body": "x",
        "replies": [{"author": "user", "body": "why?"}, {"author": "claude", "body": "because"}]}]}"#;
    std::fs::write(dir.join("reviews/pr-3.json"), review).unwrap();
    l.find_todo();
    l.tab = Tab::Todo;
    let shown = |l: &List| l.shown().iter().map(|p| p.number).collect::<Vec<_>>();
    // Answered (75) before requested (70) before no review (30); yours with nothing else isn't on it.
    assert_eq!(shown(&l), [3, 2, 1]);
    // CI still running on one: after the rest, back once it's done.
    l.prs[1].checks = Some((3, 0, 2));
    l.find_todo();
    assert_eq!(shown(&l), [3, 1, 2]);
    // Not with a check failing already: that's to fix now.
    l.prs[1].checks = Some((2, 1, 2));
    l.find_todo();
    assert_eq!(shown(&l), [3, 2, 1]);
    l.prs[1].checks = Some((5, 0, 0));
    l.find_todo();
    assert_eq!(shown(&l), [3, 2, 1]);
    assert_eq!(l.todo[0].says(), "Claude answered you on 1 thread · Claude's review is new to you");
    // Claude's part: a review where there's none (#1, #2), nothing on #3 (up to date, nothing waiting).
    assert!(l.todo.iter().all(|it| (it.number == 3) == it.claude.is_none()));
    // Claude's ranking: its order and words, kept while nothing's changed.
    let fp = crate::todo::fingerprint(&l.todo);
    // Nothing ranks by itself: not opening the tab, not R (only r r, twice).
    assert!(l.rank_rx.is_none());
    let k = |l: &mut List, c| l.key(KeyEvent::new(c, KeyModifiers::NONE));
    k(&mut l, KeyCode::Char('r'));
    assert!(l.rank_rx.is_none() && l.msg.starts_with("r again: Claude ranks the 3 PR(s) that need you, one run (never ranked before)"), "{}", l.msg);
    k(&mut l, KeyCode::Char('j')); // anything else calls it off
    assert!(l.confirm.is_none());
    l.ranked = Some(crate::todo::Ranked { at: crate::claude::now_secs(), cost: 0.38, fingerprint: fp, waits: Default::default(), items: vec![("pr-1".into(), "a fee bug — review it".into()), ("pr-3".into(), "y".into()), ("pr-2".into(), "z".into())] });
    l.find_todo();
    assert_eq!((shown(&l), l.since), (vec![1, 3, 2], crate::todo::Since::default()));
    assert!(l.since_text().contains("nothing's changed since"));
    // A new one since: counted, and said on its card.
    l.prs.push(pr(4, "New", "cy"));
    l.extra.insert(4, extra(true));
    l.find_todo();
    assert_eq!(l.since.new, 1);
    assert!(l.rank_offer().contains("cost $0.38, since: 1 new"), "{}", l.rank_offer());
    l.prs.pop();
    l.find_todo();
    snapshot("list_todo", &screen(100, 12, |f| l.draw(f)));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn defer_from_the_todo_tab() {
    let pr = |number, author: &str| Listed { number, title: "t".into(), author: author.into(), state: "OPEN".into(), updated: "2026-01".into(), ..Listed::default() };
    let dir = std::env::temp_dir().join(format!("dv-defer-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("reviews")).unwrap();
    let mut l = List::new("".into());
    (l.reviews_dir, l.ranked, l.deferred_file, l.deferred) = (Some(dir.join("reviews")), None, Some(dir.join("deferred.json")), Default::default());
    l.prs = vec![pr(1, "amy"), pr(2, "bo")];
    l.me = "me".into();
    let extra = |requested| super::Extra { requested, mergeable: "MERGEABLE".into(), head_oid: "h".into(), ..Default::default() };
    l.extra = HashMap::from([(1, extra(true)), (2, extra(false))]);
    l.find_todo();
    l.tab = Tab::Todo;
    let shown = |l: &List| l.shown().iter().map(|p| p.number).collect::<Vec<_>>();
    assert_eq!((shown(&l), l.todo_now()), (vec![1, 2], 2));
    let k = |l: &mut List, c| l.key(KeyEvent::new(c, KeyModifiers::NONE));
    // s, then anything but a time: not deferred.
    k(&mut l, KeyCode::Char('s'));
    k(&mut l, KeyCode::Char('x'));
    assert_eq!((shown(&l), l.msg.as_str()), (vec![1, 2], "not deferred"));
    // s 4: four hours, to the bottom, out of the count; kept.
    k(&mut l, KeyCode::Char('s'));
    k(&mut l, KeyCode::Char('4'));
    assert_eq!((shown(&l), l.todo_now()), (vec![2, 1], 1));
    assert!(l.todo[1].deferred.as_ref().is_some_and(|d| d.until - crate::claude::now_secs() > 3 * 3600));
    assert!(crate::todo::load_deferred(Some(&dir.join("deferred.json"))).contains_key("pr-1"));
    // Claude at work on #2 (here or in any dv): to the bottom, out of the count, said so.
    l.running.insert(2, "making the changes".into());
    l.find_todo();
    assert_eq!((shown(&l), l.todo_now()), (vec![1, 2], 0)); // #1 deferred, #2 busy: nothing for you now
    assert_eq!(l.todo[1].busy.as_deref(), Some("making the changes"));
    assert_eq!(l.claude_state(2), Some(("claude: making the changes…".into(), true)));
    // Done: back.
    l.running.clear();
    l.find_todo();
    assert_eq!(shown(&l), [2, 1]); // (#1 is still deferred)
    // A trusted comment on #2 says it's blocked on #9: at the bottom, out of the count, for
    // everyone; back once #9's merged.
    l.extra.get_mut(&2).unwrap().said = vec![("bo".into(), "Blocked on #9.".into())];
    l.find_todo();
    assert_eq!(l.todo.last().map(|it| (it.number, it.waits.clone())), Some((2, Some((crate::todo::Until::Merged(9), "bo".into())))));
    l.others.insert(9, "MERGED".into());
    l.find_todo();
    assert!(l.todo.iter().all(|it| it.waits.is_none()));
    l.extra.get_mut(&2).unwrap().said.clear();
    // b b: ask for a rebase (the first b says what it'll do; anything else calls it off).
    k(&mut l, KeyCode::Char('b'));
    assert!(l.msg.starts_with("b again: ask #2's author for a rebase"), "{}", l.msg);
    assert!(k(&mut l, KeyCode::Char('x')).is_none());
    k(&mut l, KeyCode::Char('b'));
    assert!(matches!(k(&mut l, KeyCode::Char('b')), Some(Do::AskRebase(2))));
    // s on it again: back.
    k(&mut l, KeyCode::Char('j'));
    k(&mut l, KeyCode::Char('s'));
    assert_eq!((shown(&l), l.todo_now()), (vec![1, 2], 2));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn claude_failure_readable_on_a_wide_card() {
    // Wide window: why Claude failed runs on past the right zone, to the window's edge, all of
    // its lines; the first line's age still ends the zone.
    let ui = crate::theme::Theme::load().ui();
    let cx = Cards { ui: &ui, num_w: 5, width: 260, checks_loading: false, usual: "main" };
    let p = Listed { number: 1, title: "t".into(), author: "amy".into(), updated: "2026-01-01T00:00:00Z".into(), ..Listed::default() };
    let why = "claude failed: failed to run the worktree's hook · fatal: 'bdk-2335' is already checked out at '/home/x/bdk-2335'";
    let mark = Mark { marked: false, claude: Some((why.into(), false)), new: false, replaces: None, awaiting: 0 };
    let lines = card(&p, &cx, false, None, mark);
    let text = |l: &Line| l.spans.iter().map(|s| s.content.as_ref()).collect::<String>();
    assert!(text(&lines[2]).contains(why), "{}", text(&lines[2]));
    assert!(text(&lines[0]).trim_end().len() < 140, "the age stays at the zone's end: {}", text(&lines[0]));
    assert!(lines.iter().all(|l| l.width() == 260));
}

#[test]
fn runs_to_approve_on_the_card() {
    let ui = crate::theme::Theme::load().ui();
    let cx = Cards { ui: &ui, num_w: 5, width: 120, checks_loading: false, usual: "main" };
    let p = Listed { number: 1, title: "t".into(), author: "amy".into(), ..Listed::default() };
    let text = |m: Mark| card(&p, &cx, false, None, m).iter().map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect::<String>()).collect::<Vec<_>>().join("\n");
    let mark = |awaiting| Mark { marked: false, claude: None, new: false, replaces: None, awaiting };
    assert!(text(mark(2)).contains("⏸ 2 to approve"));
    assert!(!text(mark(0)).contains("to approve"));
}

#[test]
fn n_n_drafts_a_new_pr() {
    let mut l = List::new("-wb".into());
    let k = |l: &mut List, c| l.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    assert!(k(&mut l, 'N').is_none() && l.msg.starts_with("N again: Claude drafts a new PR"));
    assert!(matches!(k(&mut l, 'N'), Some(Do::NewPr)));
    // Another key between: not.
    k(&mut l, 'N');
    k(&mut l, 'j');
    assert!(k(&mut l, 'N').is_none());
}

#[test]
fn unmark_all() {
    let mut l = List::new("-wb".into());
    let k = |l: &mut List, c| l.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    assert!(k(&mut l, 'U').is_none() && l.msg == "nothing marked");
    l.marked.extend([1, 22, 333]);
    k(&mut l, 'U');
    assert!(l.msg.starts_with("U again unmarks all 3") && l.marked.len() == 3);
    k(&mut l, 'j'); // another key between: not
    k(&mut l, 'U');
    k(&mut l, 'U');
    assert!(l.marked.is_empty() && l.msg == "unmarked 3");
}

#[test]
fn mouse_in_the_list() {
    use crate::view::tests::screen;
    use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    let pr = |number, title: &str| Listed { number, title: title.into(), author: "a".into(), base: "master".into(), state: "OPEN".into(), files: 1, ..Listed::default() };
    let mut l = List::new("".into());
    l.prs = vec![pr(1, "one"), pr(2, "two"), pr(3, "three")];
    l.tab = Tab::Open;
    _ = screen(120, 20, |f| l.draw(f));
    let click = |l: &mut List, column: u16, row: u16| l.mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column, row, modifiers: KeyModifiers::NONE });
    // A click selects a card (the second: under the header, a card's rows down); it doesn't open
    // it, even when it's the one selected; a second click soon after does.
    let second = 1 + l.card_rows() as u16;
    assert!(!click(&mut l, 10, second) && l.sel == 1);
    l.last_click = None; // (as if a while ago)
    assert!(!click(&mut l, 10, second), "a single click on the selected card opened it");
    assert!(click(&mut l, 10, second), "a double-click didn't open it");
    // A click on a tab: that tab.
    let (x, _) = l.tab_hits.iter().find(|(_, t)| *t == Tab::Worktrees).cloned().unwrap();
    click(&mut l, x.start + 1, 0);
    assert_eq!(l.tab, Tab::Worktrees);
}

#[test]
fn drafting_a_replacement_on_the_todo_tab() {
    let pr = |number, author: &str| Listed { number, title: "t".into(), author: author.into(), state: "OPEN".into(), updated: format!("2026-0{number}"), files: 1, ..Listed::default() };
    let mut l = List::new("".into());
    (l.ranked, l.deferred, l.deferred_file) = (None, Default::default(), None);
    l.prs = vec![pr(1, "amy"), pr(2, "bo"), pr(3, "cy")];
    l.me = "me".into();
    let extra = || super::Extra { requested: true, mergeable: "MERGEABLE".into(), head_oid: "h".into(), ..Default::default() };
    l.extra = HashMap::from([(1, extra()), (2, extra()), (3, extra())]);
    let order = |l: &mut List| {
        l.find_todo();
        l.todo.iter().map(|it| (it.rep, it.number)).collect::<Vec<_>>()
    };
    let first = order(&mut l)[0].1;
    // Claude drafting #first's replacement: #first goes to the bottom, saying so.
    let original = crate::replace::Original { number: first, ..Default::default() };
    l.rep_of.insert(first, crate::replace::Record { id: first, replaces: vec![original], ..Default::default() });
    l.drafting.insert(first, crate::batch::Status::Reviewing);
    let o = order(&mut l);
    assert_eq!(o.last(), Some(&(false, first)), "{o:?}");
    assert_eq!(l.todo.last().unwrap().busy.as_deref(), Some("drafting a replacement"));
    // Drafted: the replacement, ready for you to review, on top.
    l.rep_of.get_mut(&first).unwrap().title = "Do it properly".into();
    l.drafting.insert(first, crate::batch::Status::Done("3 turns".into()));
    let o = order(&mut l);
    assert_eq!(o[0], (true, first), "{o:?}");
    // Claude at it again in the replacement's worktree (making changes there): to the bottom too.
    l.running_reps.insert(first, "working (in its pane)".into());
    let o = order(&mut l);
    assert_eq!(o.last(), Some(&(true, first)), "{o:?}");
    assert_eq!(l.todo.last().unwrap().busy.as_deref(), Some("working (in its pane)"));
    l.running_reps.clear();
    assert_eq!(order(&mut l)[0], (true, first));
    // On it, what's done to a PR on GitHub isn't done to the one it replaces: said so.
    (l.reps, l.tab, l.sel) = (vec![pr(first, "me")], Tab::Todo, 0);
    for k in ['M', 'b', 'W', 'm'] {
        l.msg.clear();
        assert!(l.key(KeyEvent::new(KeyCode::Char(k), KeyModifiers::NONE)).is_none() && l.confirm.is_none());
        assert!(l.msg.contains("draft replacement"), "{k}: {}", l.msg);
    }
}

#[test]
fn active_tab_lit() {
    // The tab shown is drawn lit (the menu's selection colors), the others not.
    let mut l = List::new("".into());
    l.prs = vec![Listed { number: 1, title: "t".into(), state: "OPEN".into(), ..Listed::default() }];
    l.tab = Tab::Open;
    let backend = ratatui::backend::TestBackend::new(100, 10);
    let mut term = ratatui::Terminal::new(backend).unwrap();
    term.draw(|f| l.draw(f)).unwrap();
    let buf = term.backend().buffer().clone();
    let (open, _) = l.tab_hits.iter().find(|(_, t)| *t == Tab::Open).cloned().unwrap();
    let (todo, _) = l.tab_hits.iter().find(|(_, t)| *t == Tab::Todo).cloned().unwrap();
    // Lit: the selection's background, or (a theme without one, as with none at all) reversed.
    let lit = |x: u16| match l.ui.menu_selected.bg {
        Some(bg) => buf[(x, 0)].bg == bg,
        None => buf[(x, 0)].modifier.contains(ratatui::style::Modifier::REVERSED),
    };
    assert!(lit(open.start + 1), "the tab shown isn't lit");
    assert!(!lit(todo.start + 1));
}

#[test]
fn a_a_retries_a_ci_fix() {
    // Claude found CI failing isn't the PR's doing: a a has it try again, unless it has you to
    // answer, or changes of its wait to be pushed.
    use crate::todo::{Item, Need};
    let mut l = List::new("-wb".into());
    let item = |needs: Vec<Need>, claude| Item { rep: false, number: 7, needs, claude, updated: String::new(), reason: None, changed: false, deferred: None, busy: None, waits: None };
    let ci = Need::CiNotThisPr("flaky".into(), true);
    l.todo = vec![item(vec![ci.clone()], Some(batch::Task::Review))];
    assert_eq!(l.todo_task(7), batch::Task::FixChecks);
    l.todo = vec![item(vec![ci.clone()], Some(batch::Task::Answer { theirs: false }))];
    assert_eq!(l.todo_task(7), batch::Task::Answer { theirs: false });
    l.todo = vec![item(vec![ci, Need::ClaudeChanged(1, 0)], None)];
    assert_eq!(l.todo_task(7), batch::Task::Review);
}

#[test]
fn closed_worktree_prs_looked_up_in_the_background() {
    // A worktree's PR not in the list shows as being looked up (not open: not on the open tab
    // nor the todo), then as GitHub has it; a closed one's mark goes, a failed lookup says so.
    let mut l = List::new("-wb".into());
    let looking = |n| Listed { number: n, title: "(looking it up…)".into(), state: super::LOOKING.into(), ..Listed::default() };
    l.prs = vec![looking(3), looking(9)];
    l.marked = HashSet::from([3, 9]);
    l.gone_pending = HashSet::from([3, 9]);
    assert!(l.shown().is_empty()); // the open tab
    let (tx, rx) = mpsc::channel();
    tx.send(vec![(3, Some(Listed { number: 3, title: "old fix".into(), state: "MERGED".into(), ..Listed::default() })), (9, None)]).unwrap();
    l.gone_rx.push(rx);
    l.gone_news();
    assert_eq!((l.prs[0].title.as_str(), l.prs[1].title.as_str()), ("old fix", "(couldn't load this PR)"));
    assert!(l.gone.contains_key(&3) && !l.gone.contains_key(&9)); // 9 is asked again next time
    assert_eq!(l.marked, HashSet::from([9])); // merged: its mark goes; unknown: kept
    assert!(l.gone_rx.is_empty() && l.gone_pending.is_empty());
}
