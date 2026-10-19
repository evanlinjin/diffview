use super::*;

#[test]
fn keys_and_yank() {
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    let press = |app: &mut App, s: &str| s.chars().for_each(|c| app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)));
    press(&mut app, "]g"); // first change: the deleted line
    assert_eq!(app.rows[app.head.row].line.kind, Kind::Del);
    press(&mut app, "jww");
    assert_eq!(app.selection_text(false), "new_call");
    press(&mut app, "x");
    assert_eq!(app.selection_text(false), "    new_call(x);\n");
    press(&mut app, "kx");
    assert_eq!(app.selection_text(true), "-    old();\n");
    let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
    press(&mut app, "/call");
    app.key(enter);
    assert_eq!((app.anchor, app.head), (Pos { row: 4, col: 8 }, Pos { row: 4, col: 11 }));
    press(&mut app, "N"); // n / N repeat the search, as in Helix (one match: stays)
    assert_eq!(app.head, Pos { row: 4, col: 11 });
    press(&mut app, "/CALL");
    app.key(enter);
    assert!(app.msg.starts_with("no match")); // smart case: uppercase query is case-sensitive
    assert_eq!(base64(b"hello"), "aGVsbG8=");
    let h = app.hover_lines("```rust\nfn x()\n```\n\ndocs");
    assert_eq!(h.len(), 3); // fences dropped
    assert_eq!(h[0].spans[1].style.fg, Some(Color::Red)); // `fn` highlighted as Rust (after the block's 1-col padding)
    let back = KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT);
    app.key(back);
    assert_eq!(app.rows.len(), 3); // only the three file headers
    app.key(back);
    assert!(app.rows.len() > 3);
    app.alphabet = vec!['a', 's'];
    app.height = 20;
    app.goto(4, 0);
    press(&mut app, "gw"); // nearest word after the cursor ("new_call") gets "aa"
    assert_eq!(app.labels[0].2, ['a', 'a']);
    press(&mut app, "aa");
    assert_eq!(app.selection_text(false), "new_call");
}

#[test]
fn suggestion_block() {
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    // rows: 0 file, 1 hunk, 2 ctx "fn main() {", 3 del, 4 add "    new_call(x);", 5 ctx "}"
    app.goto(4, 0);
    app.select = true;
    app.goto(5, 0);
    let (a, prefill) = app.suggestion().unwrap();
    assert_eq!((a.path.as_str(), a.line, a.left, a.start), ("src/a.rs", 3, false, Some((2, false))));
    assert_eq!(prefill, "\n```suggestion\n    new_call(x);\n}\n```\n");
    // Removed lines can't be part of a suggestion.
    app.goto(3, 0);
    assert!(app.suggestion().err().unwrap().to_string().contains("removed lines"));
    // Code containing ``` gets a longer fence.
    app.rows[4].line.text = "let s = \"```\";".into();
    app.select = false;
    app.goto(4, 0);
    assert!(app.suggestion().unwrap().1.contains("````suggestion\n"));
}

#[test]
fn viewed_files() {
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    app.pr = Some(gh::Pr { number: 1, node_id: "PR".into(), title: "t".into(), base: "main".into(), head_oid: "abc".into(), url: String::new(), ..Default::default() });
    let viewed = |p: &str, s: &str| HashMap::from([(p.to_owned(), s.to_owned())]);
    app.apply_viewed(viewed("src/a.rs", "VIEWED"));
    app.build();
    assert_eq!(app.header_tag(0).unwrap().0, "✓ Viewed ");
    assert!(app.header_tag(INFO).is_none()); // the PR's state, once it's known
    app.pr_state = Some(("DRAFT", "6e7681"));
    assert_eq!(app.header_tag(INFO).unwrap().0, " DRAFT ");
    let header = app.rows.iter().find(|r| r.line.kind == Kind::File && r.file == 0).unwrap();
    assert!(!header.line.text.contains("Viewed")); // drawn at the right edge, not part of the text
    assert!(app.collapsed.contains("src/a.rs")); // viewed → folded
    app.collapsed.clear(); // you unfold it…
    app.apply_viewed(viewed("src/a.rs", "VIEWED"));
    assert!(app.collapsed.is_empty()); // …and a refresh leaves it unfolded
    // Not with comments you haven't seen on it.
    let mut fresh = App::with("base".into(), diff::parse(diff::SAMPLE));
    fresh.seen.path = Some(std::env::temp_dir().join("dv-viewed-seen-unused"));
    let c = gh::Comment { id: 1, author: "amy".into(), body: "hm".into(), node_id: "n1".into(), ..Default::default() };
    fresh.threads = vec![gh::Thread { path: "src/a.rs".into(), line: Some(2), comments: vec![c], ..Default::default() }];
    fresh.apply_viewed(viewed("src/a.rs", "VIEWED"));
    assert!(fresh.collapsed.is_empty());
    // Folded already, and Claude comments there: it opens.
    fresh.threads.clear();
    fresh.collapsed.insert("src/a.rs".into());
    let dir = std::env::temp_dir().join(format!("dv-viewed-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("pr-1.json"), r#"{"comments": [{"path": "src/a.rs", "line": 2, "body": "hm"}]}"#).unwrap();
    fresh.claude.path = Some(dir.join("pr-1.json"));
    fresh.load_claude();
    _ = std::fs::remove_dir_all(&dir);
    assert!(fresh.collapsed.is_empty());
    app.apply_viewed(viewed("src/a.rs", "DISMISSED"));
    app.build();
    assert_eq!(app.header_tag(0).unwrap().0, "↻ changed since viewed ");
    let h = app.rows.iter().position(|r| r.line.kind == Kind::File && r.file == 0).unwrap();
    app.goto(h, 0);
    app.key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE));
    assert!(matches!(app.job, Some(Job::Viewed(ref p, true)) if p == "src/a.rs")); // dismissed → mark again
}

#[test]
fn help_search() {
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    let k = |app: &mut App, code| app.key(KeyEvent::new(code, KeyModifiers::NONE));
    let text = |app: &App| -> Vec<String> {
        app.popup.as_ref().unwrap().1.iter().map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect()).collect()
    };
    k(&mut app, KeyCode::Char('?'));
    let all = text(&app).len();
    k(&mut app, KeyCode::Char('/'));
    "bracket".chars().for_each(|c| k(&mut app, KeyCode::Char(c)));
    assert_eq!(text(&app), [" MOVE", "  mm                   matching bracket"]);
    k(&mut app, KeyCode::Enter); // keep the filter; j/k scroll again, popup stays
    k(&mut app, KeyCode::Char('j'));
    assert!(app.popup.is_some() && text(&app).len() == 2);
    k(&mut app, KeyCode::Char('/'));
    k(&mut app, KeyCode::Esc); // clear
    assert_eq!(text(&app).len(), all);
    k(&mut app, KeyCode::Char('/'));
    "zzzz".chars().for_each(|c| k(&mut app, KeyCode::Char(c)));
    assert!(text(&app)[0].contains("no keys match"));
    k(&mut app, KeyCode::Enter);
    k(&mut app, KeyCode::Char('x')); // any other key closes
    assert!(app.popup.is_none() && app.help.is_none());
}

#[test]
fn matching_bracket() {
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    // rows: 2 "fn main() {", 3 del "    old();", 4 add "    new_call(x);", 5 "}"
    let mm = |app: &mut App| {
        app.key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE));
        app.key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE));
    };
    app.goto(2, 10); // {
    mm(&mut app);
    assert_eq!(app.head, Pos { row: 5, col: 0 }); // } on the new side, skipping the removed line
    mm(&mut app);
    assert_eq!(app.head, Pos { row: 2, col: 10 });
    app.goto(4, 12); // ( in new_call(x)
    mm(&mut app);
    assert_eq!(app.head, Pos { row: 4, col: 14 });
    app.goto(2, 0);
    mm(&mut app);
    assert!(app.msg.contains("not on a bracket"));

    // The match is hidden between hunks: the hunk grows to show it.
    let body: String = (2..=19).map(|i| if i == 3 { "+    changed();\n".to_string() } else { format!("     line{i}();\n") }).collect();
    let text = format!("diff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -1,19 +1,20 @@\n fn f() {{\n{body} }}\n");
    let mut app = App::with("base".into(), diff::parse(&text));
    assert!(!app.rows.iter().any(|r| r.line.text == "}")); // hidden past the context
    let open = app.rows.iter().position(|r| r.line.text == "fn f() {").unwrap();
    app.goto(open, 7);
    mm(&mut app);
    assert_eq!(app.rows[app.head.row].line.text, "}");
    assert!(app.msg.contains("expanded"));
}

#[test]
fn split_view() {
    let sample = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,4 +1,3 @@\n ctx\n-old1\n-old2\n+new1\n end\n";
    let mut app = App::with("base".into(), diff::parse(sample));
    let ctrl = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL);
    app.key(ctrl('w'));
    app.key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE));
    assert!(app.split);
    // rows: 0 file, 1 hunk, 2 ctx, 3 old1, 4 old2, 5 new1, 6 end
    assert_eq!(app.drows, [
        DRow::Full(0), DRow::Full(1), DRow::Pair(Some(2), Some(2)),
        DRow::Pair(Some(3), Some(5)), DRow::Pair(Some(4), None), DRow::Pair(Some(6), Some(6)),
    ]);
    app.goto(2, 0); // ctx, cursor prefers the right (new) side
    app.vmove(1);
    assert_eq!(app.head.row, 5); // new1, beside old1
    app.key(ctrl('h'));
    assert_eq!(app.head.row, 3); // old1
    app.vmove(1);
    assert_eq!(app.head.row, 4); // stays on the left: old2
    app.key(ctrl('l'));
    assert_eq!(app.head.row, 4); // nothing on the right there; side switches for the next move
    app.vmove(1);
    assert_eq!(app.head.row, 6);
    // Ctrl-w h / Ctrl-w l do the same, as Helix moves between splits.
    app.goto(5, 0); // new1
    app.key(ctrl('w'));
    app.key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE));
    assert_eq!(app.head.row, 3); // old1, beside it
    app.key(ctrl('w'));
    app.key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE));
    assert_eq!(app.head.row, 5);
}

#[test]
fn comments_placed_and_anchored() {
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    let c = |id, body: &str| gh::Comment {
        id, node_id: format!("C{id}"), can_edit: id == 2, can_delete: id == 2, author: "amy".into(), body: body.into(), created: "2026-10-01".into(), pending: false, reactions: vec![], hidden: String::new(),
    };
    let t = |left, line, resolved, comments| Thread {
        id: format!("T{line:?}"),
        path: "src/a.rs".into(), left, line, start_line: None, original_line: Some(1), resolved, outdated: line.is_none(),
        hunk: vec![], comments, claude: None, severity: String::new(), about: None, conversation: String::new(),
        want: String::new(),
    };
    app.threads = vec![
        t(true, Some(2), false, vec![c(1, "why remove?"), c(2, "unused")]), // on the deleted line
        t(false, None, true, vec![c(3, "old note")]),                        // outdated + resolved
    ];
    app.build();
    let kinds: Vec<_> = app.rows.iter().take(11).map(|r| (r.line.kind, r.thread)).collect();
    assert_eq!(kinds, [
        (Kind::File, None),
        (Kind::Hunk, None),
        (Kind::Ctx, None),
        (Kind::Thread, Some(1)), // outdated (was L1) + resolved: folded, under line 1
        (Kind::Del, None),
        (Kind::Thread, Some(0)),
        (Kind::Comment, Some(0)),
        (Kind::Thread, Some(0)),
        (Kind::Comment, Some(0)),
        (Kind::Comment, Some(0)), // the box's bottom edge
        (Kind::Add, None),
    ]);
    assert!(app.rows[9].edge);
    let parts: Vec<_> = (3..10).map(|r| app.box_part(r)).collect();
    assert_eq!(parts, [None, None, Some(BoxPart::Top), Some(BoxPart::Body), Some(BoxPart::Divider), Some(BoxPart::Body), Some(BoxPart::Bottom)]);
    // Select from the deleted line down to the added line (across the thread): multi-line, mixed sides.
    app.goto(4, 0);
    app.select = true;
    app.goto(10, 0);
    let a = app.selection_anchor().unwrap();
    assert_eq!(app.file_ref().unwrap(), "src/a.rs:2"); // new-side lines only; the deleted one has none

    // An outdated thread is relocated by the commented line's text.
    let moved = Thread { hunk: vec!["+    new_call(x);".into()], ..t(false, None, false, vec![c(4, "nit")]) };
    let lines = &app.files[0].lines;
    assert_eq!(anchor_line(&moved, lines).map(|i| lines[i].kind), Some(Kind::Add));
    // A trivial line ("}") falls back to the original line number (3 → the closing brace's row).
    let brace = Thread { hunk: vec![" }".into()], original_line: Some(3), ..t(false, None, false, vec![c(5, "x")]) };
    assert_eq!(anchor_line(&brace, lines).map(|i| lines[i].new), Some(Some(3)));
    let far = Thread { original_line: Some(500), ..brace };
    assert_eq!(anchor_line(&far, lines), None); // nothing within 10 lines → under the file header

    // Pending replies are tagged; `c` on a thread queues a reply in the review, `C` posts it now.
    app.threads[0].comments[1].pending = true;
    app.build();
    assert!(app.rows.iter().any(|r| r.line.text.starts_with("[pending] amy")));
    app.pr = Some(gh::Pr { number: 1, node_id: "PR".into(), title: "t".into(), base: "main".into(), head_oid: "abc".into(), url: "https://github.com/o/r/pull/1".into(), ..Default::default() });
    let tr = app.rows.iter().position(|r| r.thread == Some(0)).unwrap();
    app.goto(tr, 0);
    let press = |app: &mut App, c| app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    press(&mut app, 'c'); // c comments on code; on a thread it points to r
    assert!(app.action.is_none() && app.msg.starts_with("r replies"));
    press(&mut app, 'r');
    assert!(matches!(app.action.take(), Some(Action::Reply { review: true, ref thread, .. }) if thread == "TSome(2)"));
    press(&mut app, 'C'); // and C replies right away
    assert!(matches!(app.action.take(), Some(Action::Reply { review: false, .. })));
    // On the second comment ("unused"), the reply quotes that comment, not the first.
    let second = app.rows.iter().position(|r| r.line.text == "unused").unwrap();
    app.goto(second, 0);
    press(&mut app, 'r');
    assert!(matches!(app.action.take(), Some(Action::Reply { ref quote, .. }) if quote == "> unused\n\n"));
    // Space e edits the comment under the cursor, if you're allowed to.
    press(&mut app, ' ');
    press(&mut app, 'e');
    assert!(matches!(app.action.take(), Some(Action::Edit { ref comment, ref body, .. }) if comment == "C2" && body == "unused"));
    let first = app.rows.iter().position(|r| r.line.text == "why remove?").unwrap();
    app.goto(first, 0);
    press(&mut app, ' ');
    press(&mut app, 'e');
    assert!(app.action.is_none() && app.msg.contains("can't edit"));
    // Space p lists pending comments and jumps to the chosen one.
    app.threads[0].comments[1].body = "> why remove?\n\nunused".into();
    app.build();
    press(&mut app, ' ');
    press(&mut app, 'p');
    let p = app.picker.take().unwrap();
    assert_eq!(p.items.len(), 1);
    assert_eq!((p.items[0].label.as_str(), p.items[0].detail.as_slice()), ("src/a.rs:2  reply", &["unused".to_owned()][..])); // quote skipped
    let Target::Comment(ti, ci) = p.items[0].target else { panic!() };
    // Ctrl-d there asks to delete it ("No" first), then deletes that comment.
    app.picker = Some(p);
    app.key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
    let confirm = app.picker.take().unwrap();
    assert_eq!((confirm.title, confirm.sel), (" delete comment? ", 0));
    assert!(matches!(confirm.items[1].target, Target::DeleteComment(ref id) if id == "C2"));
    app.goto_comment(ti, ci);
    assert!(app.rows[app.head.row].line.text.starts_with("[pending] amy"));
    app.threads[0].comments[1].body = "unused".into();
    app.build();
    app.goto(first, 0);
    // Space t lists every thread with its state.
    press(&mut app, ' ');
    press(&mut app, 't');
    let items = app.picker.take().unwrap().items;
    assert_eq!(items[0].label, "src/a.rs:2  1 reply · [pending] @amy");
    assert_eq!(items[0].detail, ["why remove?"]); // the comment's text on the card's next line
    assert!(items[1].label.contains("[resolved] [outdated] @amy") && items[1].detail == ["old note"]);
    // Space d asks first, defaulting to "No".
    press(&mut app, ' ');
    press(&mut app, 'd');
    assert!(app.msg.contains("can't delete"));
    let second = app.rows.iter().position(|r| r.line.text == "unused").unwrap();
    app.goto(second, 0);
    press(&mut app, ' ');
    press(&mut app, 'd');
    let p = app.picker.as_ref().unwrap();
    assert!(matches!(p.items[0].target, Target::Nothing) && matches!(p.items[1].target, Target::DeleteComment(ref id) if id == "C2"));
    app.picker = None;
    assert_eq!(clean_body("> unused\n\n<!-- dv: x -->\n", "<!-- dv:", "> unused\n\n"), ""); // untouched → cancel
    assert_eq!(clean_body("> unused\n\nagreed\n<!-- dv: x -->", "<!-- dv:", "> unused\n\n"), "> unused\n\nagreed");
    let tr = app.rows.iter().position(|r| r.thread == Some(0)).unwrap(); // rebuilt rows include the PR header now
    app.goto(tr, 0);
    press(&mut app, 'C');
    assert!(matches!(app.action.take(), Some(Action::Reply { review: false, .. })));
    press(&mut app, 'S');
    let labels: Vec<String> = app.picker.take().unwrap().items.into_iter().map(|i| i.label).collect();
    assert_eq!(labels[..4], ["Comment", "Approve", "Request changes", "Comment and close the PR"]); // no pending review → no discard option
    // Then what's still open: the thread here is unresolved (and the test's PR head isn't HEAD).
    assert_eq!(labels[4], "  ! 1 unresolved thread(s) on GitHub");
    assert!(labels[5].starts_with("  ! your checkout isn't the PR's head") && labels.len() == 6);
    app.review = Some("R".into());
    press(&mut app, 'S');
    assert_eq!(app.picker.as_ref().unwrap().items[4].label, "Discard pending review (1 comment(s))");
    app.picker = None;
    press(&mut app, 'C'); // GitHub refuses a single comment beside a pending review
    assert!(app.action.is_none() && app.msg.starts_with("you have a pending review (1)"));
    assert_eq!((a.line, a.left, a.start), (2, false, Some((2, true))));
    assert_eq!(wrap("aaa bbb ccc", 7), ["aaa bbb", "ccc"]);
}

#[test]
fn quote_into_conversation() {
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    app.pr = Some(gh::Pr { number: 1, node_id: "PR".into(), title: "t".into(), base: "main".into(), head_oid: "abc".into(), url: String::new(), ..Default::default() });
    let line = |heading, md, text: &str| InfoLine { heading, md, text: text.into(), ..Default::default() };
    app.info = vec![activity_heading(1), line(true, false, "@bo · 2d ago"), line(false, true, "Looks good\nbut fix CI")];
    app.build();
    let quoted = |app: &mut App, row: usize| {
        app.goto(row, 0);
        app.key(KeyEvent::new(KeyCode::Char('C'), KeyModifiers::NONE));
        match app.action.take() {
            Some(Action::Converse { quote, .. }) => quote,
            _ => panic!("no conversation comment started"),
        }
    };
    let body = app.rows.iter().position(|r| r.line.text == "but fix CI").unwrap();
    assert_eq!(quoted(&mut app, body), "> Looks good\n> but fix CI\n\n");
    let author = app.rows.iter().position(|r| r.line.text == "@bo · 2d ago").unwrap();
    assert_eq!(quoted(&mut app, author), "> Looks good\n> but fix CI\n\n"); // the author line quotes its comment
    let heading = app.rows.iter().position(|r| r.line.text == "activity (1)").unwrap();
    assert_eq!(quoted(&mut app, heading), ""); // other PR lines: an empty conversation comment
    // `c` quotes it at the end of your review summary (saved on GitHub), without checking git state.
    let c = |app: &mut App, row: usize| {
        app.goto(row, 0);
        app.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
        assert!(app.action.is_none());
        match app.job.take() {
            Some(Job::Post(Post::Summary(body))) => assert_eq!(body, app.summary_draft),
            _ => panic!("summary not saved"),
        }
    };
    app.summary_draft = "Mostly fine.\n".into();
    c(&mut app, body);
    c(&mut app, author);
    assert_eq!(app.summary_draft, "Mostly fine.\n\n> Looks good\n> but fix CI\n\n> Looks good\n> but fix CI");
    app.goto(heading, 0);
    app.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
    assert!(app.job.is_none() && app.msg.starts_with("c here: on a comment"), "{}", app.msg);
    // A rebuild (as after posting) keeps the cursor on the same PR row.
    app.goto(body, 0);
    let keep = app.cursor_key();
    app.rebuild(keep);
    assert_eq!(app.head.row, body);
    // An empty description shows GitHub's placeholder, which isn't quoted.
    app.info = vec![line(true, false, "description"), line(false, true, "")];
    app.build();
    let ph = app.rows.iter().position(|r| r.line.text == "(no description)").unwrap();
    assert_eq!(quoted(&mut app, ph), "");
    // Outside the PR section, `C` still posts a line comment.
    let code = app.rows.iter().position(|r| r.line.kind == Kind::Add).unwrap();
    app.goto(code, 0);
    app.key(KeyEvent::new(KeyCode::Char('C'), KeyModifiers::NONE));
    assert!(!matches!(app.action, Some(Action::Converse { .. })));
}

#[test]
fn claude_review() {
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    let p = std::env::temp_dir().join(format!("dv-claude-view-{}.json", std::process::id()));
    std::fs::write(&p, r#"{ "summary": "Looks fine.", "comments": [
        { "path": "src/a.rs", "line": 2, "body": "**nit:** name" },
        { "path": "src/a.rs", "line": 2, "side": "old", "body": "why remove?" } ] }"#).unwrap();
    app.claude.path = Some(p.clone());
    app.load_claude(); // not load_threads: that asks GitHub about PR 1 of this repo
    app.build();
    let texts: Vec<&str> = app.rows.iter().map(|r| r.line.text.as_str()).collect();
    assert_eq!(&texts[..3], ["Claude's review", "claude's review", "Looks fine."]); // summary up top, no PR needed
    assert_eq!((app.box_part(1), app.box_part(2), app.box_part(3)), (Some(BoxPart::Top), Some(BoxPart::Body), Some(BoxPart::Bottom)));
    let row = |app: &App, side_left: bool| {
        app.rows.iter().position(|r| r.line.kind == Kind::Thread && r.thread.is_some_and(|t| app.threads[t].left == side_left)).unwrap()
    };
    // Each sits under its line: the old-side one under the deleted line, the new-side one under the added line.
    assert_eq!(app.rows[row(&app, true) - 1].line.kind, Kind::Del);
    assert_eq!(app.rows[row(&app, false) - 1].line.kind, Kind::Add);
    let press = |app: &mut App, s: &str| s.chars().for_each(|c| app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)));
    // `r` replies to Claude (saved to the file, not GitHub).
    app.goto(row(&app, false), 0);
    press(&mut app, "r");
    // First what you want of it: on your PR, the change made, unless you said otherwise there.
    assert_eq!(app.picker.as_ref().map(|p| p.sel), Some(2));
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(matches!(app.action.take(), Some(Action::ClaudeReply(k, "change", _)) if k == "c1")); // ids given on load
    // Space r resolves it in the file: it folds, and the cursor stays on it.
    press(&mut app, " r");
    assert!(std::fs::read_to_string(&p).unwrap().contains("\"resolved\": true"));
    let t = app.rows[app.head.row].thread.unwrap();
    assert!(app.threads[t].resolved && !app.thread_open(t));
    // Space d dismisses one; Space e explains instead of editing.
    app.goto(row(&app, true), 0);
    press(&mut app, " e");
    assert!(app.action.is_none() && app.msg.contains("r replies to Claude"));
    press(&mut app, " d");
    assert_eq!(app.threads.iter().filter(|t| t.claude.is_some()).count(), 1);
    // Claude writing the file shows up without a refresh.
    std::fs::write(&p, r#"{ "comments": [] }"#).unwrap();
    app.claude.mtime = None;
    app.watch_claude();
    assert!(app.threads.is_empty() && app.msg.contains("0 comment(s)"));
    _ = std::fs::remove_file(&p);
}

#[test]
fn claude_outdated() {
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    let p = std::env::temp_dir().join(format!("dv-claude-outdated-{}.json", std::process::id()));
    // One whose code moved (recorded on line 1, now on 2) and one whose code is gone.
    std::fs::write(&p, r#"{ "comments": [
        { "path": "src/a.rs", "line": 1, "body": "moved", "code": ["    new_call(x);"] },
        { "path": "src/a.rs", "line": 2, "body": "gone", "code": ["    other();"] } ] }"#).unwrap();
    app.claude.path = Some(p.clone());
    app.load_claude(); // not load_threads: that asks GitHub about PR 1 of this repo
    app.build();
    _ = std::fs::remove_file(&p);
    let (moved, gone) = (&app.threads[0], &app.threads[1]);
    assert_eq!((moved.line, moved.outdated), (Some(2), false));
    assert!(gone.outdated && !app.thread_open(1)); // folded, like GitHub's outdated threads
    let head = app.rows.iter().position(|r| r.thread == Some(1)).unwrap();
    assert!(app.rows[head].line.text.starts_with("[outdated] ◆ claude"));
    // Under the line it moved to: the added line (new side 2).
    let m = app.rows.iter().position(|r| r.thread == Some(0)).unwrap();
    assert_eq!(app.rows[m - 1].line.new, Some(2));
    app.goto(head, 0);
    app.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
    assert!(app.action.is_none() && app.msg.contains("outdated"));
}

#[test]
fn claude_menu() {
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    let p = std::env::temp_dir().join(format!("dv-claude-menu-{}.json", std::process::id()));
    std::fs::write(&p, r#"{ "comments": [
        { "path": "src/a.rs", "line": 2, "body": "a", "replies": [ { "author": "user", "body": "why?" } ] },
        { "path": "src/a.rs", "line": 2, "body": "b", "resolved": true, "replies": [ { "author": "user", "body": "ok" } ] },
        { "path": "src/a.rs", "line": 2, "body": "**nit:** use `x` ([docs](https://x.y))", "replies": [ { "author": "claude", "body": "done" } ] } ] }"#).unwrap();
    app.claude.path = Some(p.clone());
    app.load_claude(); // not load_threads: that asks GitHub about PR 1 of this repo
    _ = std::fs::remove_file(&p);
    assert_eq!(app.waiting_replies(), 1); // resolved and answered ones don't count
    app.key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
    let p = app.picker.as_ref().unwrap();
    assert!(p.items[0].label.starts_with("Update Claude's review"));
    assert!(p.items[1].label.contains("(1 waiting)") && p.sel == 1); // answering preselected
    assert!(p.items[2].label.contains("make the changes (2 open comments")); // the resolved one is skipped
    // Picking it asks for your own instructions first (an editor), then runs Claude.
    app.picker = None;
    app.pick_claude(agent::Ask::Fix);
    assert!(matches!(app.action.take(), Some(Action::ClaudeWith(agent::Ask::Fix))));
    app.pick_claude(agent::Ask::Review); // reviewing asks too
    assert!(matches!(app.action.take(), Some(Action::ClaudeWith(agent::Ask::Review))));
    assert!(app.claude_prompt(agent::Ask::Fix, "keep the API").ends_with("The user adds:\nkeep the API"));
    assert!(!app.claude_prompt(agent::Ask::Fix, "  ").contains("The user adds"));
    app.picker = None;
    // Space A lists Claude's comments, the unanswered one tagged.
    app.key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Char('A'), KeyModifiers::NONE));
    let labels: Vec<&str> = app.picker.as_ref().unwrap().items.iter().map(|i| i.label.as_str()).collect();
    assert_eq!(labels.len(), 3);
    assert!(labels[0].contains("[waiting]") && !labels[1].contains("[waiting]") && !labels[2].contains("[waiting]"));
    assert_eq!(labels[0], "src/a.rs:2  1 reply · [waiting] @claude");
    let p = app.picker.as_ref().unwrap();
    assert_eq!(p.items[2].detail, ["nit: use x (docs)"]); // rendered: no ** or link URL
    // The filter matches the comment's text too.
    app.key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
    assert_eq!(app.matches(), [2]);
    app.picker = None;
    app.threads.clear();
    app.run_claude(agent::Ask::Answer, "");
    assert!(app.msg.contains("no replies waiting") && app.claude.job.is_none());
}

#[test]
fn posting_shows_progress() {
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    // Written text is queued as a job, so a "posting…" frame shows before the network call…
    let msg = app.post_later(Post::Converse("hi".into())).unwrap();
    assert_eq!(msg, "posting…");
    assert!(matches!(app.job, Some(Job::Post(Post::Converse(_)))));
    // …and a failure still reports (with no PR here, before anything is sent).
    let job = app.job.take().unwrap();
    app.run_job(job);
    assert!(app.msg.contains("no PR"));
}

#[test]
fn help_two_columns() {
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    let rows = |app: &mut App, w: u16| {
        app.width = w;
        app.key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE));
        let lines = app.popup.take().unwrap().1;
        app.help = None;
        lines
    };
    let one = rows(&mut app, 100);
    let two = rows(&mut app, 200);
    assert!(two.len() <= one.len() / 2 + 4, "{} vs {}", two.len(), one.len()); // about half as tall (sections stay whole)
    let text: String = two[0].spans.iter().map(|s| s.content.as_ref()).collect();
    assert!(text.starts_with(" MOVE") && text.trim_end().ends_with(text.split_whitespace().last().unwrap())); // a heading on each side
    // Both headings in the heading style, and nothing else on the line in it.
    let heading = app.ui.heading.add_modifier(ratatui::style::Modifier::BOLD);
    assert_eq!(two[0].style, Style::default());
    let styled: Vec<&str> = two[0].spans.iter().filter(|s| s.style == heading).map(|s| s.content.as_ref()).collect();
    assert_eq!(styled.len(), 2, "{:?}", two[0]);
    assert!(two.iter().all(|l| l.width() <= 197));
}

#[test]
fn copy_refs() {
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    let url = "https://github.com/o/r/pull/1";
    app.pr = Some(gh::Pr { number: 1, node_id: "PR".into(), title: "t".into(), base: "main".into(), head_oid: "abc".into(), url: url.into(), ..Default::default() });
    let c = |id| gh::Comment { id, node_id: String::new(), can_edit: false, can_delete: false, author: "amy".into(), body: "hi".into(), created: String::new(), pending: false, reactions: vec![], hidden: String::new() };
    app.threads = vec![gh::Thread {
        id: "T".into(), path: "src/a.rs".into(), left: false, line: Some(2), start_line: None, original_line: Some(2),
        resolved: false, outdated: false, hunk: vec![], comments: vec![c(11), c(12)], claude: None, severity: String::new(), about: None, conversation: String::new(),
        want: String::new(),
    }];
    let heading = |text: &str, url: &str| InfoLine { heading: true, md: false, text: text.into(), url: url.into(), ..Default::default() };
    let body = |text: &str| InfoLine { heading: false, md: true, text: text.into(), ..Default::default() };
    app.info = vec![activity_heading(1), heading("@bo · 1d ago", "https://github.com/o/r/pull/1#issuecomment-5"), body("Looks good")];
    app.build();
    let at = |app: &mut App, text: &str| {
        let r = app.rows.iter().position(|r| r.line.text == text).unwrap();
        app.goto(r, 0);
        app.cursor_ref().unwrap()
    };
    assert_eq!(at(&mut app, "Looks good"), "https://github.com/o/r/pull/1#issuecomment-5"); // conversation comment
    // On a link: Space c copies its URL (what Enter opens), and the hints say so.
    app.info[2].text = "See [the docs](https://example.com/docs).".into();
    app.build();
    let r = app.rows.iter().position(|r| r.links.iter().any(|l| l.2 == "https://example.com/docs")).unwrap();
    let col = app.rows[r].links.iter().find(|l| l.2 == "https://example.com/docs").unwrap().0;
    app.goto(r, col);
    assert_eq!(app.link_at_cursor(), Some("https://example.com/docs"));
    assert_eq!(app.context_keys().unwrap().0, "link");
    app.copy_ref();
    assert_eq!(app.msg, "copied https://example.com/docs");
    app.info[1].text = "Looks good".into();
    app.build();
    assert_eq!(at(&mut app, "PR #1 · t"), url); // the PR header
    let second = app.rows.iter().enumerate().filter(|(_, r)| r.thread == Some(0) && r.line.text == "hi").nth(1).unwrap().0;
    app.goto(second, 0);
    assert_eq!(app.cursor_ref().unwrap(), "https://github.com/o/r/pull/1#discussion_r12"); // the reply under the cursor
    // gf there opens it on GitHub (on its author line; in its text, if nothing else is under the cursor).
    let author = (0..=app.head.row).rev().find(|&r| app.rows[r].line.kind == Kind::Thread).unwrap();
    app.goto(author, 0);
    app.open_link();
    assert_eq!(app.msg, "opened https://github.com/o/r/pull/1#discussion_r12");
    assert_eq!(app.context_keys().unwrap().0, "review thread"); // the bottom-right key popup
    assert_eq!(app.context_keys().unwrap().1[0], ("r", "reply (review)"));
    app.threads[0].claude = Some("c1".into());
    app.threads[0].comments[0].author = crate::claude::AUTHOR.into();
    app.threads[0].start_line = Some(1);
    app.build();
    app.goto(app.rows.iter().position(|r| r.thread == Some(0)).unwrap(), 0);
    assert_eq!(app.cursor_ref().unwrap(), "src/a.rs:1-2"); // Claude's: where it is
    assert_eq!(app.context_keys().unwrap().0, "Claude's comment");
    let code = app.rows.iter().position(|r| r.line.kind == Kind::Add).unwrap();
    app.goto(code, 0);
    assert_eq!(app.cursor_ref().unwrap(), "src/a.rs:2");
    assert!(app.context_keys().is_none()); // plain code: no popup
}

#[test]
fn cycle_claude_comments() {
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    let p = std::env::temp_dir().join(format!("dv-claude-cycle-{}.json", std::process::id()));
    std::fs::write(&p, r#"{ "comments": [
        { "path": "src/a.rs", "line": 3, "body": "second" },
        { "path": "src/a.rs", "line": 1, "body": "first" } ] }"#).unwrap();
    app.claude.path = Some(p.clone());
    app.load_claude(); // not load_threads: that asks GitHub about PR 1 of this repo
    _ = std::fs::remove_file(&p);
    app.collapsed.insert("src/a.rs".into()); // folded: ]a opens it
    app.build();
    let press = |app: &mut App, s: &str| s.chars().for_each(|c| app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)));
    let on = |app: &App| app.rows[app.head.row].thread.map(|t| app.threads[t].comments[0].body.clone());
    press(&mut app, "]a");
    assert_eq!(on(&app).as_deref(), Some("first")); // by line, not file order
    press(&mut app, "]a");
    assert_eq!(on(&app).as_deref(), Some("second"));
    press(&mut app, "]a");
    assert_eq!(on(&app).as_deref(), Some("first")); // wraps around
    press(&mut app, "[a");
    assert_eq!(on(&app).as_deref(), Some("second")); // and back
    // From a code line, forward reaches the comment shown under it.
    let l3 = app.rows.iter().position(|r| r.line.new == Some(3) && r.thread.is_none()).unwrap();
    app.goto(l3, 0);
    press(&mut app, "]a");
    assert_eq!(on(&app).as_deref(), Some("second"));
}

#[test]
fn replies_while_claude_works_wait_for_it() {
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    // Claude busy with a run: a new reply waits for one more run instead of starting a second.
    let child = std::process::Command::new("sleep").arg("5").spawn().unwrap();
    let log = std::env::temp_dir().join("dv-test-none.log");
    app.claude.job = Some(agent::ClaudeJob { child, log, label: "answering", forked: false, started: std::time::Instant::now() });
    let msg = app.answer_now();
    assert!(app.claude.again && msg.contains("when it's done"));
    if let Some(mut job) = app.claude.job.take() {
        _ = job.child.kill();
    }
}

#[test]
fn claude_session_per_pr() {
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    let dir = std::env::temp_dir().join(format!("dv-session-{}", std::process::id()));
    let review = dir.join("pr-7.json");
    app.claude.path = Some(review.clone());
    // The first run starts the PR's session under a new id, saved next to the review file
    // for this checkout (Claude Code keeps sessions per directory)…
    app.set_session(Some("11111111-2222-4333-8444-555555555555".into()));
    let saved = std::fs::read_to_string(crate::claude::session_path(&review)).unwrap();
    assert!(saved.starts_with(&format!("{}\t11111111-2222-4333-8444-555555555555\t", app.root.display()))); // + its skill
    // Claude may touch dv's directory for the repo, where the notes (and reviews) are.
    let notes = crate::claude::notes_path().unwrap();
    assert_eq!(agent::claude_dirs(), [std::ffi::OsString::from("--add-dir"), notes.parent().unwrap().as_os_str().to_owned()]);
    // …and forgetting it (a failed run, or Space a → fresh session) removes it, so the next
    // run starts afresh.
    app.claude_menu();
    let items = app.picker.take().unwrap().items;
    assert!(matches!(items.last().unwrap().target, Target::ClaudeFresh));
    app.fresh_claude_session();
    assert!(app.claude.session.is_none() && app.msg.starts_with("the next Claude run starts a new session"));
    assert!(!crate::claude::session_path(&review).exists());
    _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn approve_says_ack() {
    let pr = gh::Pr { number: 1, node_id: "PR".into(), title: "t".into(), base: "main".into(), head_oid: "cb1cec09".into(), url: String::new(), ..Default::default() };
    assert_eq!(ack("APPROVE", Some(&pr), "cb1cec09"), "ACK cb1cec09");
    // You reviewed another commit: still the PR's head, with a hint line (dropped when posted).
    let diverged = ack("APPROVE", Some(&pr), "698eec92");
    assert!(diverged.starts_with("<!-- dv: your checkout is at 698eec92") && diverged.ends_with("\nACK cb1cec09"));
    assert_eq!(clean_body(&diverged, HINT, ""), "ACK cb1cec09");
    assert_eq!(ack("COMMENT", Some(&pr), "cb1cec09"), "");
    assert_eq!(ack("APPROVE", None, "cb1cec09"), "");
}

#[test]
fn ask_claude_on_lines() {
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    let dir = std::env::temp_dir().join(format!("dv-ask-{}", std::process::id()));
    app.claude.path = Some(dir.join("pr-1.json"));
    let add = app.rows.iter().position(|r| r.line.kind == Kind::Add).unwrap();
    app.goto(add, 0);
    app.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
    // What you want of it first (here a question), then the editor.
    let items: Vec<_> = app.picker.as_ref().unwrap().items.iter().map(|i| i.label.clone()).collect();
    assert!(items[0].starts_with("Ask:") && items[1].starts_with("Suggest:") && items[2].starts_with("Change:"));
    app.picker.as_mut().unwrap().sel = 0;
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let Some(Action::AskClaude { anchor, code, want: "ask", .. }) = app.action.take() else { panic!("no comment for Claude started") };
    assert_eq!((anchor.path.as_str(), anchor.line, anchor.left), ("src/a.rs", 2, false));
    assert_eq!(code, ["    new_call(x);"]);
    // Once written, it's waiting for Claude (the user had the last word).
    crate::claude::add_comment(app.claude.path.as_ref().unwrap(), &anchor, code, "why?", None, "ask").unwrap();
    app.load_claude();
    assert_eq!(app.waiting_replies(), 1);
    // It says what you want, and a question isn't a change for Claude to make.
    app.build();
    let heads: Vec<_> = app.rows.iter().filter(|r| r.thread.is_some()).map(|r| r.line.text.clone()).collect();
    assert!(heads.iter().any(|h| h.starts_with("[waiting] [ask] ")), "{heads:?}");
    assert_eq!(app.change_requests(), 0);
    // A reply there starts from what you last wanted; the reply's want is the thread's now.
    let k = app.threads[0].claude.clone().unwrap();
    app.goto(app.rows.iter().position(|r| r.thread == Some(0)).unwrap(), 0);
    app.key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE));
    assert_eq!(app.picker.as_ref().map(|p| p.sel), Some(0));
    crate::claude::reply(app.claude.path.as_ref().unwrap(), &k, "just change it", "change").unwrap();
    app.load_claude();
    _ = std::fs::remove_dir_all(&dir);
    assert_eq!((app.threads[0].want.as_str(), app.change_requests()), ("change", 1));
}

#[test]
fn cycle_pending_comments() {
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    let c = |id, pending| gh::Comment { id, node_id: String::new(), can_edit: true, can_delete: true, author: "me".into(), body: format!("c{id}"), created: String::new(), pending, reactions: vec![], hidden: String::new() };
    let t = |line, comments| gh::Thread {
        id: String::new(), path: "src/a.rs".into(), left: false, line: Some(line), start_line: None, original_line: Some(line),
        resolved: false, outdated: false, hunk: vec![], comments, claude: None, severity: String::new(), about: None, conversation: String::new(),
        want: String::new(),
    };
    // A pending reply on line 3's thread, a pending comment on line 1, a posted one between.
    app.threads = vec![t(3, vec![c(1, false), c(2, true)]), t(1, vec![c(3, true)]), t(2, vec![c(4, false)])];
    app.build();
    let press = |app: &mut App, s: &str| s.chars().for_each(|ch| app.key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE)));
    let on = |app: &App| app.rows[app.head.row + 1].line.text.clone(); // the body under the author line
    press(&mut app, "]p");
    assert_eq!(on(&app), "c3"); // line 1 first
    press(&mut app, "]p");
    assert_eq!(on(&app), "c2"); // the reply itself, not its thread's first comment
    press(&mut app, "]p");
    assert_eq!(on(&app), "c3"); // wraps
    press(&mut app, "[p");
    assert_eq!(on(&app), "c2");
}

#[test]
fn your_comments_for_claude_stay_private() {
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    let dir = std::env::temp_dir().join(format!("dv-private-{}", std::process::id()));
    let p = dir.join("pr-1.json");
    let a = gh::Anchor { path: "src/a.rs".into(), line: 2, left: false, start: None };
    crate::claude::add_comment(&p, &a, vec!["    new_call(x);".into()], "is this right?", None, "").unwrap();
    app.claude.path = Some(p);
    app.pr = Some(gh::Pr { number: 1, node_id: "PR".into(), title: "t".into(), base: "main".into(), head_oid: "abc".into(), url: String::new(), ..Default::default() });
    app.load_claude(); // not load_threads: that asks GitHub about PR 1 of this repo
    app.build();
    _ = std::fs::remove_dir_all(&dir);
    // Not Claude's: no copying it to GitHub, and it doesn't count as Claude's anywhere.
    app.goto(app.rows.iter().position(|r| r.thread == Some(0)).unwrap(), 0);
    assert_eq!(app.context_keys().unwrap().0, "your comment for Claude");
    app.key(KeyEvent::new(KeyCode::Char('C'), KeyModifiers::NONE));
    assert!(app.action.is_none() && app.msg.contains("private"));
    assert!(!agent::claude_wrote(&app.threads[0]));
    assert_eq!(app.change_requests(), 1); // but it's a request "make the changes" acts on
    // `a` across removed and added lines is refused: line numbers are per side.
    let del = app.rows.iter().position(|r| r.line.kind == Kind::Del).unwrap();
    let add = app.rows.iter().position(|r| r.line.kind == Kind::Add).unwrap();
    app.goto(del, 0);
    app.select = true;
    app.goto(add, 0);
    app.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
    assert!(app.action.is_none() && app.msg.contains("one side"));
}

#[test]
fn dismiss_outdated_claude_comments() {
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    let dir = std::env::temp_dir().join(format!("dv-outdated-{}", std::process::id()));
    let p = dir.join("pr-1.json");
    std::fs::create_dir_all(&dir).unwrap();
    // Two of Claude's whose code is gone, one still current, and one of yours that's outdated.
    std::fs::write(&p, r#"{ "comments": [
        { "id": "c1", "path": "src/a.rs", "line": 2, "body": "gone 1", "code": ["    nope();"] },
        { "id": "c2", "path": "src/a.rs", "line": 2, "body": "current", "code": ["    new_call(x);"] },
        { "id": "c3", "path": "src/a.rs", "line": 1, "body": "gone 2", "code": ["fn nope() {"] },
        { "id": "c4", "path": "src/a.rs", "line": 1, "author": "user", "body": "mine", "code": ["fn other() {"] } ] }"#).unwrap();
    app.claude.path = Some(p.clone());
    app.load_claude(); // not load_threads: that asks GitHub about PR 1 of this repo
    app.build();
    assert_eq!(app.outdated_claude_comments(), ["c1", "c3"]);
    app.dismiss_outdated(false);
    assert_eq!(app.picker.take().unwrap().sel, 0); // asks first, "No" selected
    app.dismiss_outdated(true);
    let left: Vec<_> = app.threads.iter().filter_map(|t| t.claude.clone()).collect();
    _ = std::fs::remove_dir_all(&dir);
    assert_eq!(left, ["c2", "c4"]); // the current one and yours stay
}

#[test]
fn comments_in_the_margin() {
    // Four lines of code, comments on lines 1 and 2: the second box is pushed below the first.
    let sample = "diff --git a/a.rs b/a.rs\nindex 1..2 100644\n--- a/a.rs\n+++ b/a.rs\n@@ -1,4 +1,4 @@\n one\n two\n three\n-four\n+four!\n";
    let mut app = App::with("base".into(), diff::parse(sample));
    let c = |id, body: &str| gh::Comment { id, node_id: String::new(), can_edit: false, can_delete: false, author: "amy".into(), body: body.into(), created: String::new(), pending: false, reactions: vec![], hidden: String::new() };
    let t = |line, comments| gh::Thread {
        id: String::new(), path: "a.rs".into(), left: false, line: Some(line), start_line: None, original_line: Some(line),
        resolved: false, outdated: false, hunk: vec![], comments, claude: None, severity: String::new(), about: None, conversation: String::new(),
        want: String::new(),
    };
    app.threads = vec![t(1, vec![c(1, "first")]), t(2, vec![c(2, "second")])];
    app.margin_active = true;
    app.build();
    // rows: 0 file, 1 hunk, 2 one, 3-5 thread 0 (top, body, bottom), 6 two, 7-9 thread 1, 10 three,
    // 11 -four, 12 +four!
    let code = |d: usize| match app.drows[d] { DRow::Margin(c, _, _) => c, _ => None };
    let note = |d: usize| match app.drows[d] { DRow::Margin(_, _, n) => n, _ => None };
    assert_eq!(app.drows[0], DRow::Full(0)); // the file header stays full width
    // The code runs on, uninterrupted: hunk, one, two, three, -four, +four!, then spacers.
    assert_eq!((1..7).map(code).collect::<Vec<_>>(), [Some(1), Some(2), Some(6), Some(10), Some(11), Some(12)]);
    // Thread 0 starts beside its line ("one"); thread 1 right after it, not beside "two".
    assert_eq!((2..5).map(note).collect::<Vec<_>>(), [Some(3), Some(4), Some(5)]);
    assert_eq!((5..7).map(note).collect::<Vec<_>>(), [Some(7), Some(8)]);
    // It needs more room than the code has: a spacer row (no code) after the last line.
    assert_eq!(app.drows.len(), 8);
    assert_eq!((code(7), note(7)), (None, Some(9)));
    // Moving: Ctrl-w l into the comment beside the line, Ctrl-w h back; j on code skips spacers.
    app.goto(2, 0); // "one"
    let ctrl_w = |app: &mut App, c| {
        app.key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
        app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    };
    ctrl_w(&mut app, 'l');
    assert_eq!(app.head.row, 3); // thread 0's top
    assert_eq!(app.lit_code(), Some((2, 2))); // and its line lit
    ctrl_w(&mut app, 'h');
    assert_eq!(app.head.row, 2);
    assert_eq!(app.lit_thread(), Some(0)); // its box lit from the code
    app.goto(12, 0); // the last line
    app.vmove(1); // only a spacer below: stays
    assert_eq!(app.head.row, 12);
    // A search that lands in a comment leaves the cursor in the margin: j stays there.
    let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
    app.goto(0, 0);
    "/first".chars().for_each(|ch| app.key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE)));
    app.key(enter);
    assert_eq!(app.head.row, 4); // thread 0's body
    app.key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
    assert_eq!(app.head.row, 5); // its bottom edge, not the code beside it
}

#[test]
fn pr_text_attached_to_the_description() {
    let draw = |app: &mut App| _ = screen(220, 40, |f| app.draw(f));
    let mut app = fixture();
    // A conversation of five, the second new to you.
    let heading = |text: &str, node: &str| InfoLine { heading: true, md: false, text: text.into(), url: format!("u-{node}"), node: node.into(), ..Default::default() };
    let body = |text: &str| InfoLine { heading: false, md: true, text: text.into(), ..Default::default() };
    app.info.push(activity_heading(5));
    for k in 1..=5 {
        app.info.push(heading(&format!("@cy · c{k}"), &format!("n{k}")));
        app.info.push(body(&format!("comment {k}")));
    }
    app.seen.before = ["n1", "n3", "n4", "n5"].map(String::from).into();
    app.seen.path = Some(std::env::temp_dir().join("dv-seen-unused.json")); // (somewhere to keep it)
    app.build();
    draw(&mut app);
    let at = |app: &App, t: &str| app.rows.iter().position(|r| r.line.text == t);
    // In the diff's column, in order: the facts, the description, then what's new (c2) and the
    // last three; c1 isn't, but a line says there's more.
    let (facts, desc, c2, c3, c5) = (at(&app, "status"), at(&app, "description"), at(&app, "comment 2"), at(&app, "comment 3"), at(&app, "comment 5"));
    assert!(desc < facts && facts < c2 && c2 < c3 && c3 < c5, "{facts:?} {desc:?} {c2:?} {c3:?} {c5:?}");
    assert!(at(&app, "comment 1").is_none());
    let more = app.rows.iter().position(|r| r.info == Some(MORE_CONVERSATION)).expect("the more line");
    assert!(app.rows[more].line.text.contains("1 earlier") && more < c2.unwrap()); // first: what's left out is older
    assert!(app.in_pane(c2.unwrap())); // in the pane, beside the description
    // The pane hidden: after the description, in the diff's column.
    app.key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
    app.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
    draw(&mut app);
    let c2 = at(&app, "comment 2").unwrap();
    assert!(!app.in_pane(c2) && matches!(app.drows[app.d_of[c2]], DRow::Full(_)));
    app.key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
    app.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
    draw(&mut app);
    assert_eq!(app.pane_tabs().len(), 3); // Comments, Claude, Pending
    // Enter on it: the whole conversation, there in the pane (the files as they were); the
    // line then says so, and Enter on it again: only the latest.
    app.goto(more, 0);
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(at(&app, "comment 1").is_some_and(|r| app.in_pane(r)) && app.rows.iter().any(|r| r.file < app.files.len()));
    let less = app.rows.iter().position(|r| r.info == Some(MORE_CONVERSATION)).expect("the line");
    assert!(app.rows[less].line.text.contains("only the latest"));
    app.goto(less, 0);
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(at(&app, "comment 1").is_none());
    // Your own word after c2 (as c3): what's before it you'd read, new or not here.
    app.me = "me".into();
    app.info[11].text = "@me · c4".into();
    app.seen.before.clear();
    app.build();
    assert!(at(&app, "comment 2").is_none() && at(&app, "comment 3").is_some());
}

#[test]
fn margin_tags_on_their_own_line() {
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    let c = gh::Comment { id: 1, node_id: String::new(), can_edit: false, can_delete: false, author: "amy".into(), body: "old".into(), created: String::new(), pending: false, reactions: vec![], hidden: String::new() };
    app.threads = vec![gh::Thread {
        id: String::new(), path: "src/a.rs".into(), left: false, line: Some(2), start_line: None, original_line: Some(2),
        resolved: true, outdated: false, hunk: vec![], comments: vec![c], claude: None, severity: String::new(), about: None, conversation: String::new(),
        want: String::new(),
    }];
    let rows = |app: &App| app.rows.iter().filter(|r| r.thread == Some(0)).map(|r| r.line.text.clone()).collect::<Vec<_>>();
    app.build();
    assert!(rows(&app)[0].starts_with("[resolved] amy")); // inline: one line
    app.margin_active = true;
    app.build();
    let r = rows(&app);
    assert!(r[0].starts_with("amy") && r[1] == "[resolved]", "{r:?}"); // margin: tags below, even folded
}

#[test]
fn what_claude_was_refused() {
    use agent::refusal;
    let v = |d: serde_json::Value| serde_json::json!({ "permission_denials": d });
    assert_eq!(refusal(&v(serde_json::json!([]))), None);
    let edit = serde_json::json!({ "tool_name": "Edit", "tool_input": { "file_path": "/r/notes.md" } });
    assert_eq!(refusal(&v(serde_json::json!([edit]))).unwrap(), "Edit /r/notes.md");
    // A long multi-line command: its first line, short; others counted.
    let bash = serde_json::json!({ "tool_name": "Bash", "tool_input": { "command": format!("cat <<EOF > x\n{}\nEOF", "y".repeat(200)) } });
    let grep = serde_json::json!({ "tool_name": "Grep", "tool_input": { "pattern": "fn main", "path": "/r/src" } });
    assert_eq!(refusal(&v(serde_json::json!([bash, grep]))).unwrap(), "Bash cat <<EOF > x (and 1 more)");
    assert_eq!(refusal(&v(serde_json::json!([grep]))).unwrap(), "Grep /r/src"); // its path, not a bare name
}

#[test]
fn mouse() {
    use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    app.build();
    app.height = 20;
    app.geom = mouse::Geom { relw: 0, gw: 10, ghw: 6, half: 40, ctw: 0, picker: None };
    let at = |kind, x: u16, y: u16| MouseEvent { kind, column: x, row: y, modifiers: KeyModifiers::NONE };
    // rows: 0 file header, 1 hunk, 2 `fn main() {`, 3 `-    old();`, 4 `+    new_call(x);`
    app.mouse(at(MouseEventKind::Down(MouseButton::Left), 10 + 4, 4)); // text column 4 of row 4
    assert_eq!(app.head, Pos { row: 4, col: 4 });
    // A drag selects from there.
    app.mouse(at(MouseEventKind::Drag(MouseButton::Left), 10 + 11, 4));
    assert_eq!(app.selection_text(false), "new_call");
    // A click on the file header's fold arrow folds the file.
    app.mouse(at(MouseEventKind::Down(MouseButton::Left), 1, 0));
    assert!(app.collapsed.contains("src/a.rs"));
    // The wheel scrolls a picker's selection; a click picks, again opens.
    app.key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE));
    app.mouse(at(MouseEventKind::ScrollDown, 30, 5));
    assert_eq!(app.picker.as_ref().unwrap().sel, 1);
    app.geom.picker = Some(mouse::PickerGeom { list: Rect { x: 10, y: 4, width: 60, height: 5 }, item_h: 1, off: 0 });
    app.mouse(at(MouseEventKind::Down(MouseButton::Left), 20, 6)); // the third file
    assert_eq!(app.picker.as_ref().unwrap().sel, 2);
    app.mouse(at(MouseEventKind::Down(MouseButton::Left), 20, 6)); // again: opens it
    assert!(app.picker.is_none() && app.rows[app.head.row].line.text.starts_with("img.png"));
}

#[test]
fn make_changes_without_comments() {
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    app.claude.path = Some(std::env::temp_dir().join(format!("dv-fix-{}", std::process::id())).join("pr-1.json"));
    // No open comments: still asks for instructions; with none written, nothing to do.
    app.pick_claude(agent::Ask::Fix);
    assert!(matches!(app.action.take(), Some(Action::ClaudeWith(agent::Ask::Fix))));
    assert!(!app.run_claude(agent::Ask::Fix, "  "));
    assert!(app.msg.starts_with("nothing to change") && app.claude.job.is_none());
    // While Claude is busy, it says so before the editor opens (so nothing written is lost).
    let child = std::process::Command::new("sleep").arg("5").spawn().unwrap();
    app.claude.job = Some(agent::ClaudeJob { child, log: std::env::temp_dir().join("dv-test-busy.log"), label: "reviewing", forked: false, started: std::time::Instant::now() });
    app.pick_claude(agent::Ask::Fix);
    assert!(app.action.is_none() && app.msg.contains("still working"));
    if let Some(mut job) = app.claude.job.take() {
        _ = job.child.kill();
    }
    // With instructions, those are the request.
    let prompt = app.claude_prompt(agent::Ask::Fix, "rename x to y");
    assert!(prompt.starts_with("Make the change the user asks for below") && prompt.ends_with("rename x to y"));
}

#[test]
fn whose_turn_and_severity() {
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    let p = std::env::temp_dir().join(format!("dv-claude-turn-{}.json", std::process::id()));
    std::fs::write(&p, r#"{ "comments": [
        { "path": "src/a.rs", "line": 2, "body": "a", "severity": "Bug", "replies": [ { "author": "user", "body": "why?" }, { "author": "claude", "body": "because" } ] },
        { "path": "src/a.rs", "line": 2, "body": "b", "replies": [ { "author": "user", "body": "why?" } ] },
        { "path": "src/a.rs", "line": 2, "body": "c" } ] }"#).unwrap();
    app.claude.path = Some(p.clone());
    app.load_claude();
    _ = std::fs::remove_file(&p);
    let turns: Vec<_> = app.threads.iter().map(agent::turn).collect();
    assert_eq!(turns, [Some("[answered] "), Some("[waiting] "), None]); // nobody's waiting on c
    app.build();
    assert!(app.rows.iter().any(|r| r.line.text.starts_with("[bug] [answered] ◆ claude")));
    app.key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Char('A'), KeyModifiers::NONE));
    assert_eq!(app.picker.as_ref().unwrap().items[0].label, "src/a.rs:2  2 replies · [bug] [answered] @claude");
    // Before submitting, what's still open in Claude's review shows.
    assert_eq!(app.submit_notes()[0], "3 open comment(s) in Claude's review (1 answered, your turn)");
    // On someone else's PR, Claude suggests the changes instead of making them.
    app.picker = None;
    app.theirs = true;
    app.claude_menu();
    assert_eq!(app.picker.take().unwrap().items[2].label, "Ask Claude to suggest the changes (3 open comments; their PR: no edits)");
    assert!(app.claude_prompt(agent::Ask::Fix, "").contains("don't edit the code"));
    // Replies: what you said you want, else a suggestion where one asks for a change.
    let p = app.claude_prompt(agent::Ask::Answer, "");
    assert!(p.contains("do exactly that") && p.contains("suggest it rather than edit the code"));
}

#[test]
fn your_reactions_lit() {
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    let c = gh::Comment { id: 1, author: "amy".into(), body: "hm".into(), node_id: "n1".into(), reactions: vec![("THUMBS_DOWN".into(), 2, true), ("EYES".into(), 1, false)], ..Default::default() };
    app.threads = vec![gh::Thread { path: "src/a.rs".into(), line: Some(2), comments: vec![c], ..Default::default() }];
    app.build();
    let row = app.rows.iter().find(|r| r.line.text.contains("👎 2")).expect("its reactions");
    let at = |e: char| row.line.text.chars().position(|c| c == e).unwrap();
    assert_eq!((row.syn.get(at('👎')).copied().flatten(), row.syn.get(at('👀')).copied().flatten()), (Some(app.ui.key), None));
}

#[test]
fn ask_claude_about_a_commit() {
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    let dir = std::env::temp_dir().join(format!("dv-ask-commit-{}", std::process::id()));
    app.claude.path = Some(dir.join("pr-1.json"));
    app.pr = Some(gh::Pr { number: 1, url: "https://github.com/o/r/pull/1".into(), ..Default::default() });
    app.commits = vec![files::Commit { sha: "abcdef1234".into(), short: "abcdef1".into(), subject: "add parse".into(), at: "1".into(), signed: true }];
    // `a` on it: what you want, then the editor, starting from which commit.
    app.ask_commit(0);
    // On no lines, no suggestion to post: ask (preselected) or change.
    let p = app.picker.as_mut().unwrap();
    assert_eq!((p.items.len(), p.sel), (2, 0));
    p.sel = 1;
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let Some(Action::AskConversation { url, prefill, want: "change" }) = app.action.take() else { panic!("no question about the commit") };
    assert_eq!(url, "https://github.com/o/r/pull/1/commits/abcdef1234");
    assert!(prefill.starts_with("About commit abcdef1 (add parse)"));
    // Saved, it's under the commit, waiting, a change.
    crate::claude::add_conversation(app.claude.path.as_ref().unwrap(), &url, "Split this.", "change").unwrap();
    app.load_claude();
    _ = std::fs::remove_dir_all(&dir);
    app.build();
    assert!(app.rows.iter().any(|r| r.line.text == "about commit abcdef1"));
    // Shown while that commit alone is too (it's on no lines).
    app.commit_shown = Some((0, 0));
    app.build();
    assert!(app.rows.iter().any(|r| r.line.text == "about commit abcdef1"));
    app.commit_shown = None;
    assert!(app.rows.iter().any(|r| r.line.text.starts_with("[waiting] [change] you")));
    assert_eq!(app.change_requests(), 1);
}

#[test]
fn commit_draft_from_claudes_changes() {
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    let p = std::env::temp_dir().join(format!("dv-claude-draft-{}.json", std::process::id()));
    std::fs::write(&p, r#"{ "comments": [
        { "path": "src/a.rs", "line": 2, "body": "a", "replies": [
            { "author": "user", "body": "rename it", "created": "2026-10-16T12:00:00Z" },
            { "author": "claude", "body": "Changed: renamed `x` to `count`.\nMore detail.", "created": "2026-10-16T12:01:00Z" } ] },
        { "path": "src/a.rs", "line": 2, "body": "b", "replies": [
            { "author": "claude", "body": "Changed: an older change", "created": "2026-10-16T11:00:00Z" } ] } ] }"#).unwrap();
    app.claude.path = Some(p.clone());
    app.load_claude();
    _ = std::fs::remove_file(&p);
    let since = gh::epoch_secs("2026-10-16T11:30:00Z").unwrap(); // the last commit: after the older change
    let d = git::draft(&app.threads, since, " M src/a.rs\n?? new.rs\n");
    assert_eq!(d, "Address review comments\n\n- renamed `x` to `count`.\n\n<!-- dv: will commit:  M src/a.rs -->\n<!-- dv: will commit: ?? new.rs -->\n");
    assert_eq!(clean_body(&d, HINT, ""), "Address review comments\n\n- renamed `x` to `count`."); // hints aren't committed
    assert!(git::draft(&app.threads, i64::MAX, "").trim().is_empty()); // nothing new from Claude: you write it
}

#[test]
fn messages_kept_in_full() {
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    app.msg = "posting…".into(); // on its way: not news yet
    app.log_msg();
    app.msg = "gh api: Validation Failed: user_id can only have one pending review per pull request".into();
    app.log_msg();
    app.log_msg(); // still showing: once
    app.msg.clear();
    app.key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE));
    assert_eq!(app.msgs.len(), 1);
    let (title, lines, _) = app.popup.take().unwrap();
    assert!(title.starts_with(" messages"));
    let text: Vec<String> = lines.iter().map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect::<String>()).collect();
    let text = text.join(" ").split_whitespace().collect::<Vec<_>>().join(" "); // across wrapped lines
    assert!(text.contains("one pending review per pull request"), "{text}");
}

#[test]
fn threads_on_files_gone_from_the_diff() {
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    let c = gh::Comment {
        id: 1, node_id: "C1".into(), can_edit: false, can_delete: false, author: "amy".into(), body: "still here?".into(),
        created: "2026-10-01".into(), pending: false, reactions: vec![], hidden: String::new(),
    };
    app.threads = vec![gh::Thread {
        id: "T1".into(), path: "src/reverted.rs".into(), left: false, line: Some(4), start_line: None, original_line: Some(4),
        resolved: false, outdated: false, hunk: vec![], comments: vec![c], claude: None, severity: String::new(), about: None, conversation: String::new(),
        want: String::new(),
    }];
    app.pr = Some(gh::Pr { number: 1, node_id: "PR".into(), title: "t".into(), base: "main".into(), head_oid: "abc".into(), url: String::new(), ..Default::default() });
    app.build();
    let texts: Vec<&str> = app.rows.iter().map(|r| r.line.text.as_str()).collect();
    let at = texts.iter().position(|t| *t == "comments on files no longer in the diff (1)").expect("listed");
    assert_eq!(texts[at + 1], "src/reverted.rs");
    assert!(texts[at + 2].contains("amy") && app.rows[at + 2].thread == Some(0));
    // ]c gets there, and C on it replies (it's a thread, though in the PR section).
    app.goto(0, 0);
    app.key(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
    assert_eq!(app.rows[app.head.row].thread, Some(0));
    app.key(KeyEvent::new(KeyCode::Char('C'), KeyModifiers::NONE));
    assert!(matches!(app.action.take(), Some(Action::Reply { review: false, .. })));
    // With the pane shown they're with the PR section, on its PR tab, every row placed once.
    app.margin_active = true;
    app.build();
    let gone_rows: Vec<usize> = (0..app.rows.len()).filter(|&r| app.rows[r].thread == Some(0)).collect();
    // In the pane, each once (as the PR's text is).
    assert!(gone_rows.iter().all(|&r| app.drows.iter().filter(|d| matches!(d, DRow::Margin(_, _, Some(x)) if *x == r)).count() == 1));
}

#[test]
fn untracked_files_stay_off_github() {
    let mut files = diff::parse(diff::SAMPLE);
    files[0].meta = "untracked".into();
    let mut app = App::with("base".into(), files);
    app.pr = Some(gh::Pr { number: 1, node_id: "PR".into(), title: "t".into(), base: "main".into(), head_oid: "abc".into(), url: String::new(), ..Default::default() });
    let a = Anchor { path: "src/a.rs".into(), line: 2, left: false, start: None };
    assert!(app.on_github(a).is_err_and(|e| format!("{e:#}").contains("untracked")));
    let line = app.rows.iter().position(|r| r.line.kind == Kind::Add).unwrap();
    app.goto(line, 0);
    app.toggle_viewed();
    assert!(app.msg.contains("untracked") && app.job.is_none());
}

#[test]
fn claude_runs_say_what_they_cost() {
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    let log = std::env::temp_dir().join(format!("dv-claude-cost-{}.log", std::process::id()));
    std::fs::write(&log, r#"{"is_error": false, "num_turns": 5, "total_cost_usd": 0.2231, "session_id": "s"}"#).unwrap();
    let mut child = std::process::Command::new("true").spawn().unwrap();
    _ = child.wait();
    app.claude.job = Some(agent::ClaudeJob { child, log: log.clone(), label: "answering", forked: true, started: std::time::Instant::now() });
    app.poll_claude();
    _ = std::fs::remove_file(&log);
    assert_eq!(app.msg, "Claude is done answering (5 turns, $0.22)");
    assert!((app.claude.spent - 0.2231).abs() < 1e-9);
}

#[test]
fn claude_pane_asking() {
    // Claude Code's prompts, as dump-screen shows them: dv mustn't type into these.
    assert!(agent::asking(" Do you want to make this edit to ensure.rs?\n ❯ 1. Yes\n   2. No"));
    assert!(agent::asking(" Is this a project you created or one you trust?\n ❯ No, exit\n   Yes, I trust this folder"));
    assert!(!agent::asking("● Done.\n───\n❯ \n───\n  ⏵⏵ auto mode on"));
    // At it: its spinner line (Claude Code's words vary), or the older "esc to interrupt".
    assert!(agent::working("● Reading files\n· Moseying… (59s · ↓ 1.3k tokens)\n───\n❯ \n"));
    assert!(agent::working("✻ Wibbling… (1m 9s · ↓ 3.4k tokens · thought for 2s)"));
    assert!(agent::working("✶ Thinking… (esc to interrupt)"));
    // Done, waiting at its prompt (what you typed there and didn't send isn't work).
    assert!(!agent::working("● Done… (see above)\n───\n❯ force push it\n───\n  ⏵⏵ auto mode on"));
}

#[test]
fn new_until_seen() {
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    // As in dv's directory: `reviews/` and `seen/` side by side.
    let dir = std::env::temp_dir().join(format!("dv-new-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("reviews")).unwrap();
    let p = dir.join("reviews/pr-1.json");
    std::fs::write(&p, r#"{ "comments": [
        { "path": "src/a.rs", "line": 2, "body": "a" },
        { "path": "src/a.rs", "line": 2, "body": "b", "replies": [ { "author": "user", "body": "why?" } ] },
        { "path": "src/a.rs", "line": 2, "author": "user", "body": "mine" } ] }"#).unwrap();
    app.claude.path = Some(p.clone());
    app.load_claude();
    app.load_seen();
    assert_eq!(app.seen.path.as_deref(), Some(dir.join("seen/pr-1.json").as_path()));
    let new = |app: &App| app.threads.iter().map(|t| app.is_new(t)).collect::<Vec<_>>();
    assert_eq!(new(&app), [true, true, false]); // your own comment isn't news to you
    app.build();
    assert!(app.rows.iter().any(|r| r.line.text.starts_with("[new] ◆ claude")));
    // Seen once the cursor leaves it, and remembered.
    app.next_new(true);
    app.watch_seen();
    assert_eq!(new(&app), [true, true, false]); // still tagged while you're on it
    let on = app.head.row;
    app.next_new(true);
    app.watch_seen();
    assert_eq!(new(&app), [false, true, false]);
    assert_eq!(app.rows.iter().filter(|r| r.line.text.starts_with("[new]")).count(), 1); // the tag's gone
    assert_eq!(app.rows[app.head.row].thread, Some(1)); // and the cursor still where it went
    assert!(app.head.row > on);
    app.seen.keys.clear();
    app.load_seen();
    assert_eq!(new(&app), [false, true, false]);
    app.threads_picker(THREADS_NEW);
    let items = &app.picker.as_ref().unwrap().items;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].label, "src/a.rs:2  1 reply · [new] [waiting] @claude");
    app.picker = None;
    // Or once the cursor has stayed on it a while.
    app.watch_seen();
    assert_eq!(new(&app), [false, true, false]);
    app.seen.since -= std::time::Duration::from_secs(5);
    app.watch_seen();
    assert_eq!(new(&app), [false, false, false]);
    assert_eq!(app.rows[app.head.row].thread, Some(1)); // the cursor stays put
    app.seen.keys.clear();
    app.load_seen();
    _ = std::fs::remove_dir_all(&dir);
    assert_eq!(new(&app), [false, false, false]);
}

#[test]
fn claude_view() {
    // A 30-line new file Claude commented on at line 20, and another file it didn't touch.
    let body: String = (1..=30).map(|n| format!("+line {n}\n")).collect();
    let text = format!(
        "diff --git a/big.rs b/big.rs\nnew file mode 100644\n--- /dev/null\n+++ b/big.rs\n@@ -0,0 +1,30 @@\n{body}{}",
        diff::SAMPLE
    );
    let mut app = App::with("base".into(), diff::parse(&text));
    app.pr = Some(gh::Pr { number: 1, node_id: "PR".into(), title: "t".into(), base: "main".into(), head_oid: "abc".into(), url: String::new(), ..Default::default() });
    let plain = |text: &str| InfoLine { heading: false, md: false, text: text.into(), ..Default::default() };
    let heading = |text: &str| InfoLine { heading: true, md: false, text: text.into(), ..Default::default() };
    let md = |text: &str| InfoLine { heading: false, md: true, text: text.into(), ..Default::default() };
    app.info = vec![plain("OPEN @bo · main ← x"), plain("review: waiting on @cy"), heading("description"), md("Long story")];
    app.claude.summary = "Looks fine.".into();
    let c = |author: &str| gh::Comment { id: 1, node_id: String::new(), can_edit: false, can_delete: false, author: author.into(), body: "hm".into(), created: String::new(), pending: false, reactions: vec![], hidden: String::new() };
    let t = |path: &str, line, claude: Option<&str>, author| gh::Thread {
        id: String::new(), path: path.into(), left: false, line: Some(line), start_line: None, original_line: Some(line),
        resolved: false, outdated: false, hunk: vec![], comments: vec![c(author)], claude: claude.map(Into::into), severity: String::new(), about: None, conversation: String::new(),
        want: String::new(),
    };
    app.threads = vec![t("big.rs", 20, Some("c1"), "claude"), t("big.rs", 5, None, "amy"), t("src/a.rs", 2, None, "amy")];
    app.build();
    let texts = |app: &App| app.rows.iter().map(|r| r.line.text.clone()).collect::<Vec<_>>();
    assert!(texts(&app).contains(&"Long story".to_owned()));
    app.toggle_view(View::Claude);
    let v = texts(&app);
    // The description, and Claude's summary attached to it; not the PR's facts (they're on the
    // title) nor its conversation.
    assert!(!v.contains(&"OPEN @bo · main ← x".to_owned()) && v.contains(&"Looks fine.".to_owned()), "{v:?}");
    assert!(v.iter().any(|t| t == "Long story") && !v.iter().any(|t| t.starts_with("review:")));
    // Only the file Claude commented on, and only lines 17-23 of it.
    let code: Vec<&str> = v.iter().filter(|t| t.starts_with("line ")).map(String::as_str).collect();
    assert_eq!(code, (17..=23).map(|n| format!("line {n}")).collect::<Vec<_>>());
    assert!(!v.iter().any(|t| t.contains("src/a.rs") || t.contains("new_call")));
    assert_eq!(app.rows.iter().filter_map(|r| r.thread).collect::<HashSet<_>>(), HashSet::from([0])); // only Claude's
    assert!(v.iter().any(|t| t.starts_with("@@ -0,0 +17,7 @@")));
    // + / - show more or less around the comment, as on a hunk.
    let code_rows = |app: &App| app.rows.iter().filter(|r| r.line.text.starts_with("line ")).count();
    app.head.row = app.rows.iter().position(|r| r.line.text == "line 20").unwrap();
    app.expand(2);
    assert_eq!(code_rows(&app), 11); // lines 15-25
    app.expand(-4);
    assert_eq!(code_rows(&app), 3); // 19-21
    app.expand(2);
    // Going to a thread that isn't Claude's leaves the Claude view.
    app.goto_comment(2, 0);
    assert!(app.view == View::All && app.rows[app.head.row].thread == Some(2));
    // The pending view: the same for your pending review (a comment queued on a.rs:2, its text).
    app.threads[2].comments[0].pending = true;
    app.summary_draft = "Nearly there.".into();
    app.toggle_view(View::Pending);
    let v = texts(&app);
    assert!(v.contains(&"your review (pending)".to_owned()) && v.contains(&"Nearly there.".to_owned()), "{v:?}");
    assert!(!v.contains(&"Looks fine.".to_owned()) && !v.iter().any(|t| t.contains("big.rs")));
    assert_eq!(app.rows.iter().filter_map(|r| r.thread).collect::<HashSet<_>>(), HashSet::from([2]));
    // Its text is boxed; `c` there points to `S`, `C` starts a conversation comment as elsewhere.
    app.head.row = app.rows.iter().position(|r| r.line.text == "Nearly there.").unwrap();
    assert!(app.box_part(app.head.row).is_some());
    app.comment_or_reply(true);
    assert!(app.msg.contains("S submits"), "{}", app.msg);
    app.toggle_view(View::Pending);
    assert_eq!(app.view, View::All);
}

#[test]
fn claude_review_is_not_pr_text() {
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    app.pr = Some(gh::Pr { number: 1, node_id: "PR".into(), title: "t".into(), base: "main".into(), head_oid: "abc".into(), url: String::new(), ..Default::default() });
    app.claude.summary = "Looks fine.".into();
    app.build();
    // Not on the PR tab: Claude's summary is the Claude tab's.
    assert!(!app.rows.iter().any(|r| r.line.text == "Looks fine."));
    app.toggle_view(View::Claude);
    let row = app.rows.iter().position(|r| r.line.text == "Looks fine.").unwrap();
    app.goto(row, 0);
    assert_eq!(app.context_keys().unwrap().0, "Claude's review");
    // C posts it as your own comment: not quoted, and posted even if left as it is.
    app.key(KeyEvent::new(KeyCode::Char('C'), KeyModifiers::NONE));
    assert!(matches!(app.action.take(), Some(Action::Converse { ref quote, as_is: true }) if quote == "Looks fine.\n"));
    // c puts it in your review summary, for S.
    app.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
    assert!(app.action.is_none());
    assert_eq!(app.summary_draft, "Looks fine.");
}

#[test]
fn replacement_in_the_pr_section() {
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    let o = |number, title: &str| crate::replace::Original { number, title: title.into(), author: "bot".into(), url: format!("https://github.com/o/r/pull/{number}") };
    app.show_replacement(crate::replace::Record {
        id: 12,
        replaces: vec![o(12, "fix"), o(15, "fix again")],
        title: "Fix it properly".into(),
        body: "Replaces #12 and #15.".into(),
        closing_note: "Thanks! Handled in {replacement}.".into(),
        ..Default::default()
    });
    app.build();
    let texts: Vec<&str> = app.rows.iter().map(|r| r.line.text.as_str()).collect();
    assert_eq!(texts[0], "Replacement of #12 · Fix it properly");
    assert!(texts.contains(&"replaces #12 fix (@bot), #15 fix again (@bot)"));
    assert!(texts.contains(&"Replaces #12 and #15.") && texts.contains(&"Thanks! Handled in {replacement}."));
    // Its description and closing note: sections before the files, in the diff (as a PR's
    // description is), not on the pane's PR tab; and in the contents.
    app.margin_active = true;
    app.build();
    let at = |app: &App, t: &str| app.rows.iter().position(|r| r.line.text == t).unwrap();
    let (desc, note) = (at(&app, "Replaces #12 and #15."), at(&app, "Thanks! Handled in {replacement}."));
    assert!(app.rows[desc].main == Some(DESCRIPTION) && app.rows[note].main == Some(CLOSING_NOTE));
    assert!(desc < note);
    let names: Vec<files::Spot> = app.file_entries().into_iter().map(|e| e.spot).take(2).collect();
    assert_eq!(names, [files::Spot::Section(DESCRIPTION), files::Spot::Section(CLOSING_NOTE)]);
    assert_eq!(app.header_tag(INFO).unwrap().0, " UNPUBLISHED ");
    // Unpublished: one tab, for Claude's comments and yours (the whole diff); no PR tab, what
    // it replaces is atop its description.
    let tabs = |app: &App| app.pane_tabs().into_iter().map(|t| t.1).collect::<Vec<_>>();
    assert_eq!(tabs(&app), [" Claude "]);
    app.cycle_bar(true);
    assert!(app.bar() == Bar::Claude && app.view == View::All);
    let replaces = app.rows.iter().position(|r| r.line.text.starts_with("replaces")).unwrap();
    assert!(app.rows[replaces].main == Some(DESCRIPTION) && replaces < desc);
    let row = &app.rows[replaces];
    assert_eq!(row.links.iter().map(|l| l.2.as_str()).collect::<Vec<_>>(), ["https://github.com/o/r/pull/12", "https://github.com/o/r/pull/15"]); // each original links to it
    // Space a offers to redraft it.
    app.claude.path = Some(std::env::temp_dir().join("dv-no-such-review.json"));
    app.claude_menu();
    assert!(app.picker.take().unwrap().items.iter().any(|i| matches!(i.target, Target::ClaudeReplace(ref p) if p == &[12, 15])));
    // a on a draft: a redraft, starting from a quote of what it's about.
    let note = app.rows.iter().position(|r| r.line.text == "Thanks! Handled in {replacement}.").unwrap();
    app.goto(note, 0);
    assert_eq!(app.context_keys().unwrap().0, "Claude's draft");
    app.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
    match app.action.take() {
        Some(Action::Replace(prs, Some(q))) => assert!(prs == [12, 15] && q == "About the closing note:\n> Thanks! Handled in {replacement}.\n\n", "{q:?}"),
        _ => panic!("a on the closing note asks for a redraft"),
    }
    let body = app.rows.iter().position(|r| r.line.text == "Replaces #12 and #15.").unwrap();
    app.goto(body, 0);
    app.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
    assert!(matches!(app.action.take(), Some(Action::Replace(_, Some(q))) if q.starts_with("About the description:")));
    // S: publish first; closing what it replaces comes after.
    let s = |app: &mut App| {
        app.review_menu();
        app.picker.take().unwrap().items.into_iter().map(|i| (i.label, i.target)).collect::<Vec<_>>()
    };
    // In a checkout with nothing uncommitted (else committing comes first).
    let clean = std::env::temp_dir().join(format!("dv-clean-{}", std::process::id()));
    std::fs::create_dir_all(&clean).unwrap();
    diff::git(&["-C", &clean.to_string_lossy(), "init", "-q"]).unwrap();
    app.root = clean.clone();
    let items = s(&mut app);
    // Only as a draft: ready for review once CI passes.
    assert!(items[0].0.starts_with("Publish as a draft PR (rebased on origin/") && matches!(items[0].1, Target::Publish(false)));
    assert!(items[1].0 == "Publish, then close #12, #15 with the closing note…" && matches!(items[1].1, Target::Publish(true)));
    assert!(!items.iter().any(|i| i.0.contains("ready for review")));
    std::fs::write(clean.join("x"), "").unwrap();
    let items = s(&mut app);
    assert!(items[0].0.starts_with("Commit the uncommitted changes first") && matches!(items[0].1, Target::GitCommit));
    assert_eq!(items.len(), 1); // nothing to publish until then
    _ = std::fs::remove_dir_all(&clean);
    let mut r = app.replacement.clone().unwrap();
    (r.published, r.closed) = (Some(20), vec![12]);
    app.show_replacement(r);
    app.build();
    let items = s(&mut app);
    assert_eq!(items[0].0, "Published as PR #20");
    assert!(items[1].0 == "Close #15 with the closing note (now)…" && matches!(items[1].1, Target::CloseOriginals));
    assert_eq!(app.header_tag(INFO).unwrap().0, " PUBLISHED ");
    assert_eq!(app.pane_tabs().len(), 3); // others comment now
    let replaces = app.rows.iter().position(|r| r.line.text.starts_with("replaces")).unwrap();
    assert!(app.rows[replaces].info == Some(FACTS)); // among its facts, as a PR's
    let row = app.rows.iter().find(|r| r.line.text == "published as PR #20").unwrap();
    assert_eq!(row.links[0].2, "https://github.com/o/r/pull/20"); // the new PR, linked
}

#[test]
fn range_comment_lights_its_lines() {
    // A comment on lines 2-4 (new side): its box lights them all, and each lights the box.
    let sample = "diff --git a/a.rs b/a.rs\nindex 1..2 100644\n--- a/a.rs\n+++ b/a.rs\n@@ -1,4 +1,4 @@\n one\n two\n three\n-four\n+four!\n";
    let mut app = App::with("base".into(), diff::parse(sample));
    let c = gh::Comment { id: 1, node_id: String::new(), can_edit: false, can_delete: false, author: "amy".into(), body: "these".into(), created: String::new(), pending: false, reactions: vec![], hidden: String::new() };
    app.threads = vec![gh::Thread {
        id: String::new(), path: "a.rs".into(), left: false, line: Some(4), start_line: Some(2), original_line: Some(4),
        resolved: false, outdated: false, hunk: vec![], comments: vec![c], claude: None, severity: String::new(), about: None, conversation: String::new(),
        want: String::new(),
    }];
    app.margin_active = true;
    app.build();
    let row = |app: &App, text: &str| app.rows.iter().position(|r| r.line.text == text && r.thread.is_none()).unwrap();
    let (one, two, three, del, four) = (row(&app, "one"), row(&app, "two"), row(&app, "three"), row(&app, "four"), row(&app, "four!"));
    let in_box = app.rows.iter().position(|r| r.thread == Some(0)).unwrap();
    app.goto(in_box, 0);
    assert!(app.margin_side());
    assert_eq!(app.lit_code(), Some((two, four))); // two, three, (-four), four!: from its first line on
    assert!(del > two && del < four);
    for r in [two, three, four] {
        app.goto(r, 0);
        assert_eq!(app.lit_thread(), Some(0), "row {r}");
    }
    app.goto(one, 0);
    assert_eq!(app.lit_thread(), None);
}

#[test]
fn gf_on_a_name() {
    // gf on a name a comment mentions: where it's defined, from a grep of the files.
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    let dir = std::env::temp_dir().join(format!("dv-defs-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    diff::git(&["-C", &dir.to_string_lossy(), "init", "-q"]).unwrap();
    std::fs::write(dir.join("t:é.rs"), "use x;\n\n/* é */ fn torn_prefix_file() {}\nfn other() { torn_prefix_file() }\n").unwrap(); // a path git would quote
    std::fs::write(dir.join("u.py"), "def two_defs(): pass\nclass two_defs: pass\n").unwrap();
    app.root = dir.clone();
    app.goto_definition("torn_prefix_file"); // one: straight there (in hx: it isn't in the diff)
    assert!(matches!(&app.action, Some(Action::Open(o)) if o.ends_with("/t:é.rs:3:12")), "{:?}", app.msg);
    app.goto_definition("two_defs"); // several: a picker
    assert_eq!(app.picker.take().unwrap().items.len(), 2);
    app.goto_definition("nowhere");
    assert_eq!(app.msg, "no definition of nowhere found");
    _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn gf_after_w() {
    // `w` leaves the cursor on the space after the word it selects: gf goes by the word.
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    let c = gh::Comment { id: 1, node_id: String::new(), can_edit: false, can_delete: false, author: "claude".into(), body: "Rename new_call there".into(), created: String::new(), pending: false, reactions: vec![], hidden: String::new() };
    app.threads = vec![gh::Thread {
        id: String::new(), path: "src/a.rs".into(), left: false, line: Some(2), start_line: None, original_line: Some(2),
        resolved: false, outdated: false, hunk: vec![], comments: vec![c], claude: Some("c1".into()), severity: String::new(), about: None, conversation: String::new(),
        want: String::new(),
    }];
    app.build();
    app.root = std::env::temp_dir().join(format!("dv-gfw-{}", std::process::id())); // nothing there
    app.head = Pos { row: app.rows.iter().position(|r| r.line.text.starts_with("Rename")).unwrap(), col: 0 };
    app.anchor = app.head;
    app.word('w');
    app.word('w'); // "new_call " selected, the cursor on its space
    app.open_link();
    assert_eq!(app.msg, "no definition of new_call found");
}

// Screen snapshots: what dv draws in its main states, as text (colors aside), checked against
// src/view/snapshots/*.txt. After a change that's meant to change the screen,
// `UPDATE_SNAPSHOTS=1 cargo test` rewrites them; review the diff.

/// The screen `app` draws at `w`×`h`, a line per row, trailing spaces trimmed.
pub(crate) fn screen(w: u16, h: u16, draw: impl FnMut(&mut Frame)) -> String {
    let mut draw = draw;
    let mut t = ratatui::Terminal::new(ratatui::backend::TestBackend::new(w, h)).unwrap();
    t.draw(&mut draw).unwrap(); // the first draw lays out for the size
    let buf = t.draw(&mut draw).unwrap().buffer.clone();
    (0..h).map(|y| (0..w).map(|x| buf[(x, y)].symbol()).collect::<String>().trim_end().to_owned() + "\n").collect()
}

pub(crate) fn snapshot(name: &str, got: &str) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("src/view/snapshots/{name}.txt"));
    if std::env::var_os("UPDATE_SNAPSHOTS").is_some() || !path.exists() {
        return std::fs::write(&path, got).unwrap();
    }
    let want = std::fs::read_to_string(&path).unwrap();
    assert!(want == got, "snapshot {name} changed:\n--- want\n{want}--- got\n{got}");
}

/// A PR with two changed files and every kind of thread: Claude's, someone's on GitHub (with your
/// reply), a pending one of yours, a resolved one; Claude's summary and your review's text.
fn fixture() -> App {
    let body: String = (1..=24).map(|n| if n == 12 { "-    let x = old(12);\n+    let x = new_call(12);\n".to_owned() } else { format!(" line {n}\n") }).collect();
    let text = format!("diff --git a/src/big.rs b/src/big.rs\nindex 1..2 100644\n--- a/src/big.rs\n+++ b/src/big.rs\n@@ -1,24 +1,24 @@ fn big\n{body}{}", diff::SAMPLE);
    let mut app = App::with("base".into(), diff::parse(&text));
    app.files_pref = Some(false); // the files' list has its own test; the rest as laid out without it
    app.pr = Some(gh::Pr { number: 7, node_id: "PR".into(), title: "Fix the thing".into(), base: "main".into(), head_oid: "abc".into(), url: String::new(), author: "bo".into(), head_branch: "fix".into(), head_owner: "amy".into(), cross_repo: true, ..Default::default() });
    let line = |heading, md, text: &str| InfoLine { heading, md, text: text.into(), ..Default::default() };
    app.info = vec![line(false, false, "OPEN @bo · main ← fix · +2 −2 in 3 files"), line(false, false, "review: waiting on @cy"), line(true, false, "description"), line(false, true, "Fixes **the thing**, see `new_call`.")];
    app.claude.summary = "Looks right; one nit.".into();
    app.summary_draft = "Nearly there.".into();
    // Each comment its own id, as on GitHub and in Claude's review (dv keys threads by them).
    let id = std::cell::Cell::new(0);
    let c = |author: &str, body: &str, pending| gh::Comment { id: { id.set(id.get() + 1); id.get() }, node_id: String::new(), can_edit: true, can_delete: true, author: author.into(), body: body.into(), created: String::new(), pending, reactions: vec![], hidden: String::new() };
    let t = |path: &str, line, comments, claude: Option<&str>, resolved| gh::Thread {
        id: String::new(), path: path.into(), left: false, line: Some(line), start_line: None, original_line: Some(line),
        resolved, outdated: false, hunk: vec![], comments, claude: claude.map(Into::into), severity: if claude.is_some() { "nit".into() } else { String::new() }, about: None, conversation: String::new(),
        want: String::new(),
    };
    app.threads = vec![
        t("src/big.rs", 12, vec![c("claude", "Name it `new_value`; see src/a.rs:2.", false)], Some("c1"), false),
        t("src/big.rs", 20, vec![c("amy", "Why line 20?", false), c("you", "Because.", false)], None, false),
        t("src/a.rs", 2, vec![c("you", "Pass `y` here.", true)], None, false),
        t("src/a.rs", 1, vec![c("amy", "Old news.", false)], None, true),
    ];
    app.build();
    app
}

#[test]
fn screens() {
    let shot = |name: &str, app: &mut App, w, h| snapshot(name, &screen(w, h, |f| app.draw(f)));
    let mut app = fixture();
    shot("diff", &mut app, 100, 60);
    let mut wide = fixture();
    shot("margin", &mut wide, 220, 45); // wide enough: shown
    wide.key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
    wide.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE));
    shot("pr_tab", &mut wide, 220, 30);
    let mut app = fixture();
    app.toggle_split();
    shot("split", &mut app, 120, 40); // too narrow: hidden, marked in the gutter
    shot("split_margin", &mut app, 240, 40);
    // Narrow, shown with Ctrl-w c: the margin takes half the window.
    let mut app = fixture();
    app.key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
    app.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
    shot("narrow_margin", &mut app, 100, 40);
    let mut app = fixture();
    app.toggle_view(View::Claude);
    shot("claude_view", &mut app, 100, 30);
    app.toggle_view(View::Pending);
    shot("pending_view", &mut app, 100, 30);
    let mut app = fixture();
    app.key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE));
    shot("help", &mut app, 120, 50);
    let mut app = fixture();
    app.key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
    shot("ctrl_w_popup", &mut app, 100, 30);
    let mut app = fixture();
    app.threads_picker(THREADS_ALL);
    shot("threads_picker", &mut app, 100, 30);
    let mut app = fixture();
    app.goto_comment(0, 0);
    shot("on_claude_comment", &mut app, 100, 30);
}

#[test]
fn key_hints_match_the_keys() {
    // The which-key popups list exactly the keys pending_key handles, and ? help has each.
    let src = include_str!("keys.rs");
    let body = &src[src.find("fn pending_key").unwrap()..];
    let name = |p: char| match p {
        ' ' => "Space ".to_owned(),
        CTRL_W => "Ctrl-w ".to_owned(),
        p => p.to_string(),
    };
    let mut handled = std::collections::BTreeSet::new();
    for p in [' ', 'g', 'z', ']', '[', 'm'].into_iter().map(|p| (p, format!("'{p}'"))).chain([(CTRL_W, "CTRL_W".to_owned())]) {
        for (i, _) in body.match_indices(&format!("({}, '", p.1)) {
            let c = body[i + p.1.len() + 4..].chars().next().unwrap();
            handled.insert(format!("{}{c}", name(p.0)));
        }
    }
    let mut hinted = std::collections::BTreeSet::new();
    for p in [' ', 'g', 'z', ']', '[', 'm', CTRL_W] {
        for (k, _) in App::which(p).unwrap().1 {
            hinted.insert(format!("{}{k}", name(p)));
        }
    }
    assert_eq!(handled, hinted);
    let missing: Vec<_> = hinted.iter().filter(|k| !HELP.contains(k.as_str())).collect();
    assert!(missing.is_empty(), "not in ? help: {missing:?}");
}

#[test]
fn github_changes_shown_when_idle() {
    let mut app = fixture();
    let (tx, rx) = std::sync::mpsc::channel();
    app.github = Some(rx);
    // What GitHub has now: amy's thread got a reply (Claude's thread isn't GitHub's).
    let gh_threads: Vec<Thread> = app.threads.iter().filter(|t| t.claude.is_none()).cloned().collect();
    let mut newer = gh_threads.clone();
    let mut reply = newer[0].comments[0].clone();
    (reply.id, reply.body) = (99, "Fair.".into());
    newer[0].comments.push(reply);
    let fetched = |threads: Vec<Thread>| -> github::Fetched {
        let r = gh::Review { pr_id: "PR".into(), mine: true, me: "you".into(), pending: None, pending_body: String::new(), threads, viewed: Default::default() };
        (Ok(r), Ok(Info { lines: app_info(), title: "Fix the thing".into(), head: "abc".into(), url: String::new(), state: ("OPEN", "238636"), unsigned: Some(vec![]), awaiting: 0, failing: 0, base: String::new() }))
    };
    let replies = |app: &App| app.threads.iter().flat_map(|t| &t.comments).filter(|c| c.body == "Fair.").count();
    // Mid-way through something (here a picker): it waits. (What's shown came from the cache.)
    (app.github_stale, app.msg) = (true, github::STALE.into());
    tx.send((0, fetched(newer.clone()))).unwrap();
    app.threads_picker(THREADS_ALL);
    assert!(app.picker.is_some());
    app.watch_github();
    assert_eq!(replies(&app), 0);
    app.picker = None;
    app.watch_github();
    assert_eq!(replies(&app), 1);
    assert!(app.threads.iter().any(|t| t.claude.is_some()), "Claude's thread stays");
    assert_eq!(app.msg, "1 new comment(s) on GitHub");
    assert!(!app.github_stale, "fresh now");
    // Fetched before a load of dv's own (after posting, say): that load is newer, so it's dropped.
    app.github_gen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    tx.send((0, fetched(gh_threads))).unwrap();
    app.watch_github();
    assert_eq!(replies(&app), 1);
}

fn app_info() -> Vec<InfoLine> {
    vec![InfoLine { heading: false, md: false, text: "OPEN @bo · main ← fix".into(), ..Default::default() }]
}

#[test]
fn github_cache_reads_back() {
    // What `fetch_github` keeps: GitHub's JSON as it came, parsed the same way when read back.
    let dir = std::env::temp_dir().join(format!("dv-cache-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("pr-1.json");
    let threads = json!({ "viewer": { "login": "you" }, "repository": { "pullRequest": { "id": "PR_1", "reviewThreads": { "nodes": [
        { "id": "T1", "path": "a.rs", "line": 2, "comments": { "nodes": [{ "databaseId": 5, "body": "hi", "author": { "login": "amy" } }] } }
    ] } } } });
    let info = json!({ "title": "Fix", "headRefOid": "abc", "url": "u", "state": "OPEN", "isDraft": true, "author": { "login": "bo" } });
    crate::claude::write(&p, &json!({ "threads": threads, "info": info })).unwrap();
    let (r, i) = github::cached_github(&p).unwrap();
    let (r, i) = (r.unwrap(), i.unwrap());
    assert_eq!((r.me.as_str(), r.threads.len(), r.threads[0].comments[0].body.as_str()), ("you", 1, "hi"));
    assert_eq!((i.title.as_str(), i.head.as_str(), i.state.0), ("Fix", "abc", "DRAFT"));
    assert!(github::cached_github(&dir.join("none.json")).is_none());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn long_code_cut_at_the_box_edge() {
    let mut app = fixture();
    (app.summary_draft, app.claude.summary) = (String::new(), String::new()); // no strip over it
    app.threads[1].comments[0].body = format!("```\n{}\n```", "x".repeat(300));
    app.build();
    let s = screen(220, 45, |f| app.draw(f));
    let line = s.lines().find(|l| l.contains("xxx")).unwrap();
    assert!(line.trim_end().ends_with("x›│"), "{line}");
    // The cursor to its end scrolls that line's block, not the code beside it or the screen.
    let r = app.rows.iter().position(|r| r.nowrap && r.line.text.contains("xxx")).unwrap();
    app.goto(r, 300);
    let t = screen(220, 45, |f| app.draw(f));
    let line = t.lines().find(|l| l.contains("xxx")).unwrap();
    assert!(line.contains("‹x") && !line.contains('›') && line.trim_end().ends_with("x│"), "{line}");
    let code = |s: &str| s.lines().take(20).map(|l| l.chars().take(100).collect::<String>()).collect::<Vec<_>>();
    assert_eq!((code(&s), app.left), (code(&t), 0));
}

#[test]
fn long_facts_cut_at_the_box_edge() {
    let mut app = fixture();
    app.info[0].text = format!("OPEN @bo · main ← {}", "b".repeat(200));
    app.build();
    let s = screen(220, 45, |f| app.draw(f));
    let line = s.lines().find(|l| l.contains("bbb")).unwrap();
    assert!(line.trim_end().ends_with("b›│"), "{line}"); // cut at the box's edge
}

#[test]
fn pane_kept_between_runs() {
    let ctrl_w = |app: &mut App, c: char| {
        app.key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
        app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    };
    let p = std::env::temp_dir().join(format!("dv-pane-{}.json", std::process::id()));
    let _ = std::fs::remove_file(&p);
    let mut app = fixture();
    app.pane_file = Some(p.clone());
    _ = screen(240, 40, |f| app.draw(f));
    ctrl_w(&mut app, 'c'); // hidden
    ctrl_w(&mut app, 'f'); // floating (which shows it, but only `c` is kept as shown/hidden)
    ctrl_w(&mut app, '.');
    let w = app.margin_w_pref;
    let mut next = fixture();
    next.pane_file = Some(p.clone());
    let v = next.pane_json();
    assert_eq!((v["shown"].as_bool(), v["float"].as_bool(), v["width"].as_u64().map(|w| w as usize)), (Some(false), Some(true), w));
    ctrl_w(&mut next, '=');
    assert!(next.pane_json()["width"].is_null());
    // Relative line numbers too: Ctrl-w n flips them, kept, and over Helix's setting next time.
    let rel = next.settings.relative;
    ctrl_w(&mut next, 'n');
    assert_eq!(next.pane_json()["relative"].as_bool(), Some(!rel));
    let mut again = fixture();
    (again.pane_file, again.settings.relative) = (Some(p.clone()), rel);
    again.apply_pane();
    assert_eq!(again.settings.relative, !rel);
    let _ = std::fs::remove_file(&p);
}

#[test]
fn comments_pane() {
    let key = |app: &mut App, c: char| app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    let ctrl = |app: &mut App, c: char| app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL));
    let draw = |app: &mut App, w: u16| _ = screen(w, 40, |f| app.draw(f));
    let on = |app: &App| app.rows[app.head.row].thread;
    // Narrow: hidden, so moving down the code never lands on a comment.
    let mut app = fixture();
    draw(&mut app, 100);
    assert!(!app.margin_active);
    let line12 = app.rows.iter().position(|r| r.line.kind == Kind::Add && r.line.new == Some(12)).unwrap();
    app.goto(line12 - 1, 0);
    key(&mut app, 'j');
    key(&mut app, 'j');
    assert_eq!((on(&app), app.rows[app.head.row].line.new), (None, Some(13)));
    // Going to a comment shows them; Ctrl-w c there hides them and leaves the cursor beside it.
    app.goto_comment(0, 0);
    draw(&mut app, 100);
    assert!(app.margin_active && on(&app) == Some(0));
    ctrl(&mut app, 'w');
    key(&mut app, 'c');
    draw(&mut app, 100);
    assert!(!app.margin_active && on(&app).is_none());
    assert_eq!(app.margin_pref, Some(false));
    // Wide split view: Ctrl-l goes old side → new side → the comment pane; Ctrl-h comes back.
    let mut app = fixture();
    app.toggle_split();
    draw(&mut app, 240);
    assert!(app.margin_active);
    let del12 = app.rows.iter().position(|r| r.line.kind == Kind::Del && r.line.old == Some(12)).unwrap();
    app.goto(del12, 0);
    ctrl(&mut app, 'l');
    assert_eq!(app.rows[app.head.row].line.kind, Kind::Add);
    ctrl(&mut app, 'l');
    assert!(on(&app).is_some() && app.margin_side());
    ctrl(&mut app, 'h');
    assert!(!app.margin_side() && on(&app).is_none());
}

#[test]
fn stop_a_stuck_claude() {
    let mut app = fixture();
    use std::os::unix::process::CommandExt;
    // As dv runs Claude: in a group of its own, here with a command of its own running.
    let mark = std::env::temp_dir().join(format!("dv-stuck-{}.pid", std::process::id()));
    let child = std::process::Command::new("sh")
        .args(["-c", &format!("sleep 30 & echo $! > {}; wait", mark.display())])
        .process_group(0)
        .spawn()
        .unwrap();
    let pid = child.id();
    while std::fs::read_to_string(&mark).map_or(true, |s| s.trim().is_empty()) {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let grandchild = std::fs::read_to_string(&mark).unwrap().trim().to_owned();
    app.claude.job = Some(agent::ClaudeJob { child, log: "/nonexistent/run.json".into(), label: "reviewing", forked: false, started: std::time::Instant::now() });
    // The menu offers to stop it, first; checking says it's still at it.
    app.claude.path = Some(std::env::temp_dir().join(format!("dv-stuck-{}.json", std::process::id())));
    app.claude_menu();
    let p = app.picker.take().unwrap();
    let items = p.items;
    assert!(matches!(items[0].target, Target::ClaudeStop) && items[0].label.starts_with("Stop Claude (reviewing for "));
    assert!(matches!(items[p.sel].target, Target::Claude(agent::Ask::Review)), "Enter doesn't stop it");
    assert!(items.iter().any(|i| matches!(i.target, Target::ClaudeCheck)));
    // Typed, a word picks what has it, not what has its letters in order ("Edit Claude's notes").
    app.claude_menu();
    app.picker.as_mut().unwrap().query = "check".into();
    let first = app.matches()[0];
    assert!(matches!(app.picker.take().unwrap().items[first].target, Target::ClaudeCheck));
    app.check_claude();
    assert!(app.claude.job.is_some() && app.msg.starts_with("Claude is still reviewing"), "{}", app.msg);
    app.stop_claude();
    assert!(app.claude.job.is_none() && app.msg.starts_with("stopped Claude (reviewing for "));
    app.check_claude();
    assert!(app.msg.starts_with("Claude hasn't written a review yet"), "{}", app.msg);
    let gone = |pid: &str| !std::path::Path::new(&format!("/proc/{pid}")).exists() || std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|s| s.contains(") Z"));
    assert!(gone(&pid.to_string()), "killed");
    let t = std::time::Instant::now();
    while !gone(&grandchild) && t.elapsed().as_secs() < 2 {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(gone(&grandchild), "what it was running, killed too");
    let _ = std::fs::remove_file(&mark);
}

#[test]
fn pinned_title_and_status_bar_frame_the_content() {
    let mut app = fixture();
    let s = screen(100, 12, |f| app.draw(f));
    let l: Vec<&str> = s.lines().collect();
    assert!(app.pin && l[0].contains("PR #7") && l[11].starts_with(" NOR"), "{s}");
    // The row above the status bar is content, not covered by it (the diff runs on past it).
    assert!(!l[10].trim().is_empty() && !l[10].starts_with(" NOR"), "{s}");
}

#[test]
fn click_the_pinned_title() {
    use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    let mut app = fixture();
    app.goto(20, 0);
    _ = screen(220, 30, |f| app.draw(f));
    assert!(app.pin);
    app.mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: 30, row: 0, modifiers: KeyModifiers::NONE });
    assert_eq!(app.head.row, 0);
    // Its gutter has no fold arrow, and a click there doesn't fold the PR's text away.
    app.mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: 1, row: 0, modifiers: KeyModifiers::NONE });
    assert!(!app.info_collapsed);
}

#[test]
fn comments_in_a_tiny_window() {
    // Too narrow for the pane beside anything: going to a comment floats it over the code.
    let mut app = fixture();
    app.goto_comment(0, 0);
    let s = screen(50, 40, |f| app.draw(f));
    assert!(app.margin_active && app.margin_float && s.contains("new_value"), "{s}");
}

#[test]
fn wheel_scrolls_what_is_under_it() {
    use ratatui::crossterm::event::{MouseEvent, MouseEventKind};
    let wheel = |app: &mut App, x: u16| app.mouse(MouseEvent { kind: MouseEventKind::ScrollDown, column: x, row: 6, modifiers: KeyModifiers::NONE });
    let draw = |app: &mut App| _ = screen(220, 14, |f| app.draw(f));
    // The cursor on a comment, the review's summaries over the pane: the wheel still scrolls.
    let mut app = fixture();
    draw(&mut app);
    app.goto_comment(0, 0);
    draw(&mut app);
    let pane = (app.geom.relw + app.geom.gw + app.geom.ctw + 10) as u16;
    let top = app.top;
    wheel(&mut app, pane);
    draw(&mut app);
    assert!(app.top > top, "the pane's wheel moved nothing ({top})");
    // The comments tab: the wheel over it scrolls (with the code) while there are comments
    // further down, and stops once the last one is on screen.
    let mut app = fixture();
    draw(&mut app);
    let pane = (app.geom.relw + app.geom.gw + app.geom.ctw + 10) as u16;
    for _ in 0..20 {
        wheel(&mut app, pane);
        draw(&mut app);
    }
    let top = app.top;
    assert!(!app.comments_beyond(true));
    wheel(&mut app, pane);
    draw(&mut app);
    assert_eq!(app.top, top, "the pane's wheel kept scrolling past its last comment");
    // The code's wheel goes on.
    wheel(&mut app, 20);
    draw(&mut app);
    assert!(app.top > top || app.top == app.drows.len() - 1);
}

#[test]
fn gg_in_the_pane() {
    let k = |app: &mut App, c: char| app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    let draw = |app: &mut App| _ = screen(220, 20, |f| app.draw(f));
    // In the comments: gg to the first, ge to the last (not the diff's top / end).
    let mut app = fixture();
    draw(&mut app);
    app.goto_comment(1, 0);
    draw(&mut app);
    let comments: Vec<usize> = (0..app.rows.len()).filter(|&r| app.in_margin(r)).collect();
    k(&mut app, 'g');
    k(&mut app, 'g');
    assert_eq!(app.head.row, comments[0]);
    k(&mut app, 'g');
    k(&mut app, 'e');
    assert_eq!(app.head.row, *comments.last().unwrap());
    // In the code: the diff's top, as before.
    app.key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::CONTROL));
    k(&mut app, 'g');
    k(&mut app, 'g');
    assert_eq!(app.head.row, 0);
}

#[test]
fn next_unresolved() {
    // The fixture's threads: big.rs 12 (open), big.rs 20 (open), a.rs 2 (open), a.rs 1 (resolved).
    let mut app = fixture();
    app.threads[1].resolved = true;
    let k = |app: &mut App, a: char, b: char| {
        app.key(KeyEvent::new(KeyCode::Char(a), KeyModifiers::NONE));
        app.key(KeyEvent::new(KeyCode::Char(b), KeyModifiers::NONE));
    };
    let on = |app: &App| app.rows[app.head.row].thread;
    k(&mut app, ']', 'u');
    assert_eq!(on(&app), Some(0));
    k(&mut app, ']', 'u'); // past the resolved one on big.rs 20
    assert_eq!(on(&app), Some(2));
    k(&mut app, ']', 'u'); // wraps around
    assert_eq!(on(&app), Some(0));
    k(&mut app, '[', 'u');
    assert_eq!(on(&app), Some(2));
}

#[test]
fn comment_marks_at_the_right_edge() {
    // Claude's comment on lines 10–12 (new side): ╎ on 10 and 11, ◆ on 12, at the code's edge.
    let mut app = fixture();
    app.threads[0].start_line = Some(10);
    app.build();
    let s = screen(220, 30, |f| app.draw(f));
    let edge = |n: u32| {
        let line = s.lines().find(|l| l.starts_with(&format!(" {n:>2}  {n:>2} ")) || l.starts_with(&format!("     {n:>2} "))).unwrap();
        line.chars().nth(app.geom.relw + app.geom.gw + app.geom.ctw - 1).unwrap()
    };
    assert_eq!((edge(9), edge(10), edge(11), edge(12)), (' ', '╎', '╎', '◆'), "{s}");
    // Nothing in the old gutter spot any more.
    assert!(!s.lines().any(|l| l.contains("12◆+")));
    // Split view: on the new side's edge, not the old side's.
    app.toggle_split();
    let s = screen(240, 30, |f| app.draw(f));
    let row = s.lines().find(|l| l.contains("new_call(12)")).unwrap();
    let half = app.geom.relw + app.geom.half;
    let (old, new): (String, String) = (row.chars().take(half).collect(), row.chars().skip(half).collect());
    assert!(!old.contains('◆') && new.contains('◆'), "{row}");
}

#[test]
fn between_the_code_and_the_pane() {
    let k = |app: &mut App, c: char| app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    let ctrl = |app: &mut App, c: char| app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL));
    let draw = |app: &mut App| _ = screen(220, 40, |f| app.draw(f));
    let mut app = fixture();
    draw(&mut app);
    // Claude's comment is about new line 12; its box may sit lower in the pane.
    let line12 = app.rows.iter().position(|r| r.line.kind == Kind::Add && r.line.new == Some(12)).unwrap();
    // Ctrl-l from that line: the comment about it; Ctrl-h from the comment: back to its line.
    app.goto(line12, 0);
    draw(&mut app);
    ctrl(&mut app, 'l');
    assert_eq!(app.rows[app.head.row].thread, Some(0));
    draw(&mut app);
    ctrl(&mut app, 'h');
    assert_eq!(app.head.row, line12);
    // w at the end of a code line carries on in the code, not into the comment after it in `rows`.
    let len = app.len(line12);
    app.goto(line12, len - 1);
    k(&mut app, 'w');
    assert!(app.rows[app.head.row].thread.is_none() && !app.in_pane(app.head.row), "{:?}", app.rows[app.head.row].line.text);
    // gr on a name in text, without rust-analyzer: said so (gd would find it by text, as gf).
    app.goto(app.rows.iter().position(|r| r.line.text.contains("new_value")).unwrap(), 0);
    let col = app.rows[app.head.row].line.text.find("new_value").unwrap();
    app.goto(app.head.row, col);
    k(&mut app, 'g');
    k(&mut app, 'r');
    assert!(app.msg.starts_with("rust-analyzer is not running"), "{}", app.msg);
}

#[test]
fn claude_and_github_review_comments() {
    let mut app = fixture();
    app.claude.path = Some(std::env::temp_dir().join(format!("dv-about-{}.json", std::process::id())));
    // a on a GitHub review thread (src/a.rs:2): a comment for Claude on its lines, asking about it.
    let t = 2;
    let first = app.rows.iter().position(|r| r.thread == Some(t)).unwrap();
    app.goto(first, 0);
    app.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
    let Some(Action::AskClaude { anchor, about: Some((id, quote)), .. }) = app.action.take() else { panic!("no question for Claude") };
    assert_eq!((anchor.path.as_str(), anchor.line, id), ("src/a.rs", 2, app.threads[t].comments[0].id));
    assert!(quote.starts_with("About @you's comment") && quote.contains("> Pass `y` here."), "{quote}");
    // Claude's comment answering amy's says so, and is marked as Claude's.
    let amy = app.threads[1].comments[0].id;
    app.threads[0].about = Some(amy);
    app.build();
    let head = app.rows.iter().find(|r| r.thread == Some(0)).unwrap();
    assert!(head.line.text.contains("◆ claude · re @amy"), "{}", head.line.text);
    // From the review file: its id, or its link.
    let v = serde_json::json!({ "head": "h", "comments": [
        { "path": "a.rs", "line": 1, "body": "x", "about": 123 },
        { "path": "a.rs", "line": 1, "body": "y", "about": "https://github.com/o/r/pull/1#discussion_r456" },
    ]});
    let p = std::env::temp_dir().join(format!("dv-about-parse-{}.json", std::process::id()));
    std::fs::write(&p, v.to_string()).unwrap();
    let r = crate::claude::load(&p).unwrap().unwrap();
    assert_eq!(r.threads.iter().map(|t| t.about).collect::<Vec<_>>(), [Some(123), Some(456)]);
    let _ = std::fs::remove_file(&p);
}

#[test]
fn comments_in_folded_files_and_empty_reviews() {
    let mut app = fixture();
    // src/a.rs (two open threads there) folded, as a viewed file is: its header counts them,
    // and ]c goes in (unfolding it).
    app.collapsed.insert("src/a.rs".into());
    app.build();
    let head = app.rows.iter().find(|r| r.line.kind == Kind::File && r.line.text.contains("src/a.rs")).unwrap();
    assert!(head.line.text.ends_with("· 1 comment") || head.line.text.contains("comments"), "{}", head.line.text);
    let a_rs = app.files.iter().position(|f| f.path == "src/a.rs").unwrap();
    app.goto(0, 0);
    for _ in 0..4 {
        app.key(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::NONE));
        app.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
        if app.rows[app.head.row].file == a_rs {
            break;
        }
    }
    assert!(app.rows[app.head.row].thread.is_some() && app.rows[app.head.row].file == a_rs && !app.collapsed.contains("src/a.rs"));
    // A review with no text (a verdict alone): its box closed, saying so.
    let line = |heading, md, text: &str| InfoLine { heading, md, text: text.into(), ..Default::default() };
    app.info = vec![activity_heading(1), line(true, false, "@bo · requested changes · 2m ago"), line(false, true, "")];
    app.build();
    let r = app.rows.iter().position(|r| r.line.text == "(no comment)").unwrap();
    assert_eq!(app.box_part(r + 1), Some(BoxPart::Bottom));
}

#[test]
fn files_list_and_pinned_file_header() {
    use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    let mut app = fixture();
    app.files_pref = Some(true);
    let s = screen(220, 20, |f| app.draw(f));
    let lines: Vec<&str> = s.lines().collect();
    // The list on the left: its heading, the description (a section before the files), each
    // folder once, the files under them, with their open comments (big.rs: two).
    assert!(app.files_w > 0 && lines[0].starts_with(" CONTENTS · 4 files"), "{s}");
    let col = |l: &str| l.chars().take(app.files_w - 1).collect::<String>();
    let list: Vec<String> = lines[1..7].iter().map(|l| col(l).trim_end().to_owned()).collect();
    assert_eq!(list[0].trim_start_matches('▌').trim(), "≡ description");
    assert_eq!(list[1].trim(), "src/");
    assert!(list[2].contains("big.rs") && list[2].ends_with("·2"), "{list:?}");
    assert!(list.iter().filter(|l| l.trim() == "src/").count() == 1);
    // A click on a.rs: there.
    let a = list.iter().position(|l| l.contains("a.rs")).unwrap() + 1;
    app.mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: 3, row: a as u16, modifiers: KeyModifiers::NONE });
    assert_eq!(app.files[app.rows[app.head.row].file].path, "src/a.rs");
    // Scrolled into big.rs, past its header: the header's pinned over the code's top line.
    let big = app.rows.iter().position(|r| r.line.kind == Kind::Add && r.line.new == Some(12)).unwrap();
    app.goto(big, 0);
    app.top = app.hd() - 3;
    let s = screen(220, 20, |f| app.draw(f));
    let top = s.lines().nth(1).unwrap(); // under the pinned PR title
    assert!(top.chars().skip(app.files_w).collect::<String>().contains("src/big.rs"), "{s}");
    // Popups (help, here) are over the whole window, the list too: as wide as without it.
    let help_w = |app: &mut App| {
        app.key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE));
        _ = screen(220, 20, |f| app.draw(f));
        let s = screen(220, 20, |f| app.draw(f));
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        let top = s.lines().find(|l| l.contains(" keys ")).expect(&s).to_owned();
        (top.find(" keys ").unwrap(), app.width)
    };
    let with = help_w(&mut app);
    app.files_pref = Some(false);
    let without = help_w(&mut app);
    app.files_pref = Some(true);
    _ = screen(220, 20, |f| app.draw(f));
    assert_eq!(with, without, "the help is narrower, or elsewhere, with the list shown");
    // Ctrl-h from the code into the list: j / k go file to file, Ctrl-l back; Ctrl-l again
    // into the comments, Ctrl-h back to the code.
    let ctrl = |app: &mut App, c: char| app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL));
    let file = |app: &App| app.files[app.rows[app.head.row].file].path.clone();
    ctrl(&mut app, 'h');
    assert!(app.files_focus);
    let Some(files::Spot::File(before)) = app.current_spot() else { panic!() };
    // j / k move the list's cursor; the diff stays where it is until Enter, which goes there.
    let row = app.head.row;
    app.key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
    assert_eq!((app.files_sel, app.head.row), (Some(files::Spot::File(before + 1)), row));
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.current_spot(), Some(files::Spot::File(before + 1)));
    assert!(app.rows[app.head.row].line.kind == Kind::File && !app.files_focus);
    ctrl(&mut app, 'h');
    app.key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE));
    assert_eq!(app.current_spot(), Some(files::Spot::File(before + 1))); // not yet
    ctrl(&mut app, 'l'); // back, without going
    assert!(!app.files_focus && app.current_spot() == Some(files::Spot::File(before + 1)));
    app.goto(big, 0);
    _ = screen(220, 20, |f| app.draw(f));
    ctrl(&mut app, 'l');
    assert!(app.margin_side(), "not in the comments");
    ctrl(&mut app, 'h');
    assert!(!app.margin_side() && !app.files_focus && file(&app) == "src/big.rs");
    ctrl(&mut app, 'h');
    assert!(app.files_focus);
    // Ctrl-w , / . / = resize the list while it has focus (the comments pane stays), Ctrl-w q
    // hides it and the code has focus again.
    let w = |app: &mut App, c: char| {
        ctrl(app, 'w');
        app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        _ = screen(220, 20, |f| app.draw(f));
    };
    _ = screen(220, 20, |f| app.draw(f));
    let fw = app.files_w;
    w(&mut app, '.');
    assert!(app.files_focus && app.files_w == fw + 4 && app.margin_w_pref.is_none(), "{fw} → {}", app.files_w);
    w(&mut app, '=');
    assert_eq!(app.files_w, fw);
    w(&mut app, 'q');
    assert!(app.files_w == 0 && !app.files_focus);
}

#[test]
fn commits_under_the_files() {
    let k = |app: &mut App, c: KeyCode| app.key(KeyEvent::new(c, KeyModifiers::NONE));
    let draw = |app: &mut App| screen(220, 24, |f| app.draw(f));
    let mut app = fixture();
    app.files_pref = Some(true);
    app.commits = (1..=3).map(|n| files::Commit { sha: format!("{n}{}", "0".repeat(39)), short: format!("c{n}"), at: String::new(), signed: true, subject: format!("commit {n}") }).collect();
    let s = draw(&mut app);
    // Under the files: the heading, "all commits" (shown), then each, oldest first.
    let list: Vec<String> = s.lines().map(|l| l.chars().take(app.files_w - 1).collect::<String>().trim_end().to_owned()).collect();
    let at = list.iter().position(|l| l.starts_with(" COMMITS 3")).expect(&s);
    assert_eq!(list[at + 1].trim_start_matches('▌').trim(), "all commits");
    assert!(list[at + 2].contains("c1 commit 1") && list[at + 4].contains("c3 commit 3"), "{list:?}");
    // An unsigned commit is marked.
    app.commits[1].signed = false;
    let s = draw(&mut app);
    assert!(s.contains("c2 ✗ commit 2"), "{s}");
    app.commits[1].signed = true;
    // From the code into the list, down past the last file into the commits; Enter on one shows
    // it alone: the diff from its parent (the previous commit) to it.
    app.key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::CONTROL));
    (0..6).for_each(|_| k(&mut app, KeyCode::Char('j')));
    assert!(app.in_commits);
    app.commit_cur = 2;
    k(&mut app, KeyCode::Enter);
    assert_eq!((app.commit_shown, app.base.as_str(), app.to.as_deref()), (Some((1, 1)), app.commits[0].sha.as_str(), Some(app.commits[1].sha.as_str())));
    // Before the PR's last commit: its comments (on the head's lines) are hidden, and can't be made.
    assert!(app.before_head() && !app.threads.iter().any(|t| app.in_view(t)));
    assert!(app.check_synced().unwrap_err().to_string().starts_with("comments go on the PR's head"));
    // v, then down: the range from the first to the last; comments again (it ends at the head).
    app.commit_cur = 1;
    k(&mut app, KeyCode::Char('v'));
    k(&mut app, KeyCode::Char('j'));
    k(&mut app, KeyCode::Char('j'));
    k(&mut app, KeyCode::Enter);
    assert_eq!((app.commit_shown, app.base.as_str(), app.to.as_deref()), (Some((0, 2)), "base", Some(app.commits[2].sha.as_str())));
    assert!(!app.before_head() && app.threads.iter().any(|t| app.in_view(t)));
    let s = draw(&mut app);
    assert!(s.contains(" COMMITS 1–3 of 3") && s.contains("commits c1..c3"), "{s}");
    // "all commits": the whole diff again.
    app.commit_cur = 0;
    k(&mut app, KeyCode::Enter);
    assert_eq!((app.commit_shown, app.base.as_str(), app.to.as_deref()), (None, "base", None));
    // k from "all commits" back up into the files.
    k(&mut app, KeyCode::Char('k'));
    assert!(!app.in_commits && app.files_focus);
    // Tab between the files and the commits; Ctrl-w j / k from anywhere.
    k(&mut app, KeyCode::Tab);
    assert!(app.in_commits && app.files_focus);
    k(&mut app, KeyCode::Tab);
    assert!(!app.in_commits);
    app.files_focus = false;
    app.key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
    k(&mut app, KeyCode::Char('j'));
    assert!(app.in_commits && app.files_focus);
    app.key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
    k(&mut app, KeyCode::Char('k'));
    assert!(!app.in_commits && app.files_focus);
    // Commits shown: each one's message is a section before the files, after the description;
    // they're in the list too; Tab on a header folds it.
    (app.commit_shown, app.commit_msgs) = (Some((0, 1)), vec!["commit 1".into(), "commit 2\n\nWhy it's done this way.".into()]);
    app.build();
    let desc = app.rows.iter().position(|r| r.main.is_some() && r.line.text == "description").expect("the description's");
    let c1 = app.rows.iter().position(|r| r.main.is_some() && r.line.text == "commit c1").expect("c1's section");
    let head = app.rows.iter().position(|r| r.main.is_some() && r.line.text == "commit c2").expect("c2's section");
    assert!(desc < c1 && c1 < head && app.rows[head + 1..].iter().take_while(|r| r.main.is_some()).any(|r| r.line.text.contains("Why it's done")));
    let spots: Vec<files::Spot> = app.file_entries().into_iter().map(|e| e.spot).take(3).collect();
    assert_eq!(spots, [files::Spot::Section(DESCRIPTION), files::Spot::Section(1), files::Spot::Section(2)]);
    app.goto(head, 0);
    app.toggle_collapse();
    assert!(!app.rows.iter().any(|r| r.line.text.contains("Why it's done")));
    // y in the commits: its hash, copied; the list's own keys are its hints while it has focus.
    (app.files_focus, app.in_commits, app.commit_cur) = (true, true, 2);
    k(&mut app, KeyCode::Char('y'));
    assert_eq!(app.msg, format!("copied {}", app.commits[1].sha));
    assert_eq!(app.list_keys().0, "commits");
    // Space c on a commit's message (its section in the diff): its hash too.
    app.goto(head, 0);
    assert_eq!(app.cursor_ref().as_deref(), Some(app.commits[1].sha.as_str()));
    // m in the commits: its whole message in a popup (git's, here: the fixture's aren't real).
    (app.files_focus, app.in_commits, app.commit_cur) = (true, true, 1);
    k(&mut app, KeyCode::Char('m'));
    assert!(app.popup.as_ref().is_some_and(|p| p.0.starts_with(" commit c1")));
}

#[test]
fn up_from_the_first_comment_stays_in_the_pane() {
    let mut app = fixture();
    _ = screen(220, 30, |f| app.draw(f));
    app.goto_comment(0, 0);
    _ = screen(220, 30, |f| app.draw(f));
    assert!(app.margin_side());
    let k = |app: &mut App| app.key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE));
    (0..10).for_each(|_| k(&mut app));
    assert!(app.margin_side(), "k left the pane, onto row {}", app.head.row);
    (0..200).for_each(|_| app.key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)));
    assert!(app.margin_side(), "j left the pane");
}

#[test]
fn ask_claude_about_a_conversation_comment() {
    let mut app = fixture();
    let url = "https://github.com/o/r/pull/7#issuecomment-5";
    let head = InfoLine { heading: true, md: false, text: "@cy · commented".into(), url: url.into(), ..Default::default() };
    app.info.push(activity_heading(1));
    app.info.push(head);
    app.info.push(InfoLine { heading: false, md: true, text: "Can we rename it?".into(), ..Default::default() });
    // A question about it in Claude's review file: under it, on the PR tab.
    let c = gh::Comment { id: 0, node_id: String::new(), can_edit: false, can_delete: false, author: "you".into(), body: "what do they mean?".into(), created: String::new(), pending: false, reactions: vec![], hidden: String::new() };
    app.threads.push(gh::Thread { claude: Some("c9".into()), comments: vec![c], conversation: url.into(), ..Default::default() });
    _ = screen(220, 30, |f| app.draw(f));
    let at = |text: &str| app.rows.iter().position(|r| r.line.text.contains(text)).unwrap();
    let (body, q) = (at("Can we rename it?"), at("what do they mean?"));
    assert!(q > body && app.rows[q].thread.is_some());
    assert!(!app.rows.iter().any(|r| r.line.text.starts_with("comments on files no longer")));
    // a on the comment: a question about it, quoting it, with its link.
    app.claude.path = Some(std::env::temp_dir().join("dv-unused-review.json"));
    app.goto(body, 0);
    app.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
    let Some(Action::AskConversation { url: u, prefill, .. }) = app.action.take() else { panic!("no question: {}", app.msg) };
    assert_eq!(u, url);
    assert!(prefill.starts_with("About @cy's comment (https://") && prefill.contains("> Can we rename it?"), "{prefill}");
    // In the Claude view (no conversation there): under a heading of their own, not lost.
    app.toggle_view(View::Claude);
    let q = app.rows.iter().position(|r| r.line.text.contains("what do they mean?")).expect("the question, in the Claude view");
    let heading = app.rows[..q].iter().rev().find(|r| r.line.kind == Kind::Thread && r.thread.is_none()).unwrap();
    assert_eq!(heading.line.text, "questions about the PR's conversation");
}

#[test]
fn claude_proposes_a_description() {
    let mut app = fixture();
    let path = std::env::temp_dir().join(format!("dv-desc-{}.json", std::process::id()));
    std::fs::write(&path, r#"{"description": "Fixes **the thing**, see `new_call`.\n\nAlso renames it."}"#).unwrap();
    app.claude.path = Some(path.clone());
    app.load_claude();
    app.build();
    // The description's section shows the proposal against the current text, line by line.
    let head = app.rows.iter().position(|r| r.main.is_some() && r.line.text.starts_with("description · Claude's proposal")).expect("its section");
    let added: Vec<&str> = app.rows[head..].iter().take_while(|r| r.main.is_some()).filter(|r| r.line.kind == Kind::Add).map(|r| r.line.text.as_str()).collect();
    assert_eq!(added, ["", "Also renames it."]);
    assert!(app.rows[head..].iter().take_while(|r| r.main.is_some()).any(|r| r.line.kind == Kind::Ctx && r.line.text.starts_with("Fixes **the thing**")));
    // Space a: approve (after a confirmation), more changes, discard; a on it asks for more.
    app.claude_menu();
    let items = app.picker.take().unwrap().items;
    assert!(matches!(items[0].target, Target::DescApprove(false)) && matches!(items[2].target, Target::DescDiscard));
    app.approve_description(false);
    assert!(app.picker.take().unwrap().items.iter().any(|i| matches!(i.target, Target::DescApprove(true))));
    app.goto(head + 1, 0);
    app.start_ask_claude();
    assert!(matches!(app.action.take(), Some(Action::ClaudeWith(agent::Ask::Describe))), "{}", app.msg);
    // Discarded: gone from the file, and the description is as it was.
    app.discard_description();
    assert!(!std::fs::read_to_string(&path).unwrap().contains("description"));
    assert!(app.rows.iter().any(|r| r.main.is_some() && r.line.text == "description"));
    _ = std::fs::remove_file(&path);
}

#[test]
fn claude_drafts_a_closing_comment() {
    let mut app = fixture();
    app.pr_state = Some(("OPEN", "238636"));
    let path = std::env::temp_dir().join(format!("dv-close-{}.json", std::process::id()));
    std::fs::write(&path, r#"{"closing": "Thanks! Superseded by #9."}"#).unwrap();
    app.claude.path = Some(path.clone());
    app.load_claude();
    app.build();
    // A section first, in the diff; Space a: close with it (your editor first), more, discard.
    let head = app.rows.iter().position(|r| r.main == Some(CLOSING_PR)).expect("its section");
    assert!(app.rows[head].line.text.starts_with("closing comment · Claude's draft") && app.rows[head + 1].line.text == "Thanks! Superseded by #9.");
    app.claude_menu();
    let items = app.picker.take().unwrap().items;
    assert!(matches!(items[0].target, Target::ClosePr) && matches!(items[1].target, Target::Claude(agent::Ask::Close)) && matches!(items[2].target, Target::CloseDiscard));
    app.goto(head + 1, 0);
    app.start_ask_claude();
    assert!(matches!(app.action.take(), Some(Action::ClaudeWith(agent::Ask::Close))), "{}", app.msg);
    // Discarded: gone; Space a offers to draft one.
    app.discard_closing();
    assert!(!std::fs::read_to_string(&path).unwrap().contains("closing") && !app.rows.iter().any(|r| r.main == Some(CLOSING_PR)));
    app.claude_menu();
    assert!(app.picker.take().unwrap().items.iter().any(|i| matches!(i.target, Target::Claude(agent::Ask::Close))));
    // Closed already: not offered; a draft left over can only be discarded.
    app.pr_state = Some(("CLOSED", "da3633"));
    app.claude_menu();
    assert!(!app.picker.take().unwrap().items.iter().any(|i| matches!(i.target, Target::Claude(agent::Ask::Close))));
    std::fs::write(&path, r#"{"closing": "Thanks!"}"#).unwrap();
    app.load_claude();
    app.claude_menu();
    let items = app.picker.take().unwrap().items;
    assert!(matches!(items[0].target, Target::CloseDiscard) && !items.iter().any(|i| matches!(i.target, Target::ClosePr)));
    _ = std::fs::remove_file(&path);
}

#[test]
fn claude_working_and_out_of_sync_on_the_title() {
    // What Claude's doing and how the checkout stands show on the pinned title, as badges, a
    // long title cut to make room for them.
    let mut app = fixture();
    app.sync = "↑2 unpushed · ✎ uncommitted".into();
    app.claude.pane_busy = true;
    let s = screen(160, 12, |f| app.draw(f));
    let title = s.lines().next().unwrap();
    assert!(title.contains("◐ claude working (its pane)") && title.contains("↑2 unpushed · ✎ uncommitted"), "{title}");
    let s = screen(90, 12, |f| app.draw(f));
    assert!(s.lines().next().unwrap().contains("↑2 unpushed"), "{s}");
    // Idle and in step: neither.
    (app.sync, app.claude.pane_busy) = (String::new(), false);
    let s = screen(160, 12, |f| app.draw(f));
    assert!(!s.lines().next().unwrap().contains('◐'));
}


#[test]
fn claude_tab_is_the_claude_view() {
    let mut app = fixture();
    app.claude.summary = "Looks right; one nit.\n\nSecond paragraph.\n\nThird, which a two-line strip would cut.".into();
    _ = screen(220, 30, |f| app.draw(f));
    // gn goes round: Comments, Claude, Pending, PR.
    assert_eq!(app.bar(), Bar::Comments);
    app.cycle_bar(true);
    assert_eq!((app.bar(), app.view), (Bar::Claude, View::Claude));
    let s = screen(220, 30, |f| app.draw(f));
    assert!(s.lines().next().unwrap().contains("Claude"));
    // Claude's summary in full, attached to the description (beside it, in the pane).
    let desc = app.rows.iter().position(|r| r.main == Some(DESCRIPTION) && r.line.kind == Kind::File).expect("the description");
    let head = app.rows.iter().position(|r| r.info == Some(SUMMARY) && r.line.text == "claude's review").expect("its box");
    let text: Vec<&str> = app.rows[head + 1..].iter().take_while(|r| r.info == Some(SUMMARY)).map(|r| r.line.text.as_str()).collect();
    assert!(desc < head && text.iter().any(|t| t.starts_with("Third")), "{text:?}");
    assert!(app.in_pane(head));
    app.cycle_bar(true);
    assert_eq!((app.bar(), app.view), (Bar::Pending, View::Pending)); // your pending review
    app.cycle_bar(true);
    assert_eq!((app.bar(), app.view), (Bar::Comments, View::All));
    // A click on a tab does the same.
    app.bar_to(Bar::Claude);
    assert_eq!(app.view, View::Claude);
    app.bar_to(Bar::Pending);
    assert_eq!(app.view, View::Pending);
    // Nothing pending: both sides say so (the fixture has a pending comment: gone here).
    app.threads.iter_mut().flat_map(|t| &mut t.comments).for_each(|c| c.pending = false);
    app.summary_draft.clear();
    app.build();
    let s = screen(220, 30, |f| app.draw(f));
    assert!(s.contains("Nothing here yet: your pending review is empty") && s.contains("No pending comments."), "{s}");
    app.bar_to(Bar::Comments);
    assert_eq!(app.view, View::All);
}

#[test]
fn space_g_marks_a_draft_ready() {
    let mut app = fixture();
    let ready = |app: &mut App| {
        app.git_menu();
        app.picker.take().unwrap().items.iter().any(|i| matches!(i.target, Target::GitReady))
    };
    app.pr_state = Some(("DRAFT", "6e7681"));
    assert!(ready(&mut app));
    app.pr_state = Some(("OPEN", "238636"));
    assert!(!ready(&mut app));
}

#[test]
fn reply_to_claudes_summary() {
    let mut app = fixture();
    app.claude.path = Some(std::env::temp_dir().join("dv-unused-review.json"));
    app.bar_to(Bar::Claude);
    _ = screen(220, 30, |f| app.draw(f));
    // r on Claude's summary (on the Claude tab): a reply to it, starting a thread about it.
    let row = app.rows.iter().position(|r| r.info == Some(SUMMARY) && r.line.text.contains("Looks right")).expect("the summary");
    app.goto(row, 0);
    app.start_reply(true);
    assert!(matches!(app.action.take(), Some(Action::AskConversation { url, .. }) if url == ON_SUMMARY), "{}", app.msg);
    // a on it, with part of it selected: a reply about that part, quoting it.
    app.goto(row, 0);
    app.anchor = Pos { row, col: 0 };
    app.head = Pos { row, col: "Looks right".len() - 1 };
    app.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
    let Some(Action::AskConversation { url, prefill, .. }) = app.action.take() else { panic!("no reply: {}", app.msg) };
    assert_eq!((url.as_str(), prefill.as_str()), (ON_SUMMARY, "About this part of your review:\n> Looks right\n\n"));
    // With that thread there: under the summary, and r on the summary goes on in it.
    let c = |who: &str, body: &str| gh::Comment { author: who.into(), body: body.into(), ..Default::default() };
    app.threads.push(gh::Thread { claude: Some("c7".into()), conversation: ON_SUMMARY.into(), comments: vec![c("you", "why approve?"), c("claude", "Because it's minimal.")], ..Default::default() });
    app.build();
    _ = screen(220, 30, |f| app.draw(f));
    let at = |app: &App, t: &str| app.rows.iter().position(|r| r.line.text.contains(t)).expect(t);
    // Under its summary, attached to the description; not elsewhere.
    let s = screen(220, 30, |f| app.draw(f));
    let q = at(&app, "why approve?");
    assert!(q > row && app.rows[q].thread.is_some());
    assert!(s.contains("Because it's minimal."), "{s}");
    assert!(!app.rows.iter().any(|r| r.line.text.starts_with("questions about") || r.line.text.starts_with("comments on files no longer")));
    let row = app.rows.iter().position(|r| r.info == Some(SUMMARY) && r.thread.is_none() && r.line.text.contains("Looks right")).unwrap();
    app.goto(row, 0);
    app.start_reply(true);
    assert!(matches!(app.action.take(), Some(Action::ClaudeReply(k, ..)) if k == "c7"));
    // `a` on what's selected of Claude's answer there: a reply, quoting it.
    let ans = at(&app, "Because it's minimal.");
    app.goto(ans, 0);
    app.key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
    assert!(matches!(app.action.take(), Some(Action::ClaudeReply(k, "", q)) if k == "c7" && q.contains("> Because it's minimal.")));
    // The whole diff (Comments): not there.
    app.bar_to(Bar::Comments);
    assert!(!app.rows.iter().any(|r| r.line.text.contains("why approve?")));
}

#[test]
fn key_hints_at_the_bottom_of_the_contents() {
    let mut app = fixture();
    app.files_pref = Some(true);
    app.commits = (1..=2).map(|n| files::Commit { sha: format!("{n}{}", "0".repeat(39)), short: format!("c{n}"), at: String::new(), signed: true, subject: format!("commit {n}") }).collect();
    let list = |app: &mut App| -> Vec<String> {
        let s = screen(220, 30, |f| app.draw(f));
        s.lines().map(|l| l.chars().take(app.files_w - 1).collect::<String>().trim_end().to_owned()).collect()
    };
    // After a prefix key: what can follow, at the list's bottom (above the status bar).
    app.key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
    let l = list(&mut app);
    let keys = l.iter().position(|r| r.starts_with(" KEYS: window")).expect("the hints' heading");
    assert!(l[keys + 1].trim_start().starts_with('v'), "{l:?}"); // Ctrl-w v: split view
    assert!(l[28].trim().len() > 1 && keys > 14, "pinned at the bottom: {l:?}");
    // The commits right after the files, not at the bottom.
    let files_end = l.iter().position(|r| r.contains("big.rs")).unwrap();
    let commits = l.iter().position(|r| r.starts_with(" COMMITS 2")).unwrap();
    assert!(commits <= files_end + 5 && commits < keys, "{l:?}");
    assert!(l[commits - 1].is_empty() && !l[commits - 2].is_empty(), "a blank line between the files and the commits: {l:?}");
    // Not a popup over the code as well.
    let s = screen(220, 30, |f| app.draw(f));
    assert_eq!(s.matches("window").count(), 1, "{s}");
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
}

#[test]
fn contents_and_commits_scroll_together() {
    // Many commits, a short window: the contents and the commits scroll as one list, following
    // the list's cursor; its heading stays at the top, and the key hints at the bottom.
    let mut app = fixture();
    app.files_pref = Some(true);
    app.commits = (1..=40).map(|n| files::Commit { sha: format!("{n:040}"), short: format!("c{n}"), at: String::new(), signed: true, subject: format!("commit {n}") }).collect();
    let list = |app: &mut App| -> Vec<String> {
        let s = screen(220, 20, |f| app.draw(f));
        s.lines().map(|l| l.chars().take(app.files_w - 1).collect::<String>().trim_end().to_owned()).collect()
    };
    _ = list(&mut app);
    app.focus_list(true);
    app.commit_cur = 31; // commit c31
    app.key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL)); // hints: what follows Ctrl-w
    let l = list(&mut app);
    assert!(l[0].starts_with(" CONTENTS"), "{l:?}");
    assert!(l.iter().any(|r| r.contains("c31 commit 31")), "the cursor's commit in view: {l:?}");
    let keys = l.iter().position(|r| r.starts_with(" KEYS: window")).expect("the hints, still there");
    assert!(!l[1..keys].iter().any(|r| r.contains("big.rs")), "scrolled past the files: {l:?}");
    // A click on a commit there: that one.
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    _ = list(&mut app);
    let y = app.list_hits.iter().position(|h| *h == files::Hit::Commit(31)).unwrap() + 1;
    app.click_file(y, false);
    assert_eq!(app.commit_shown, Some((30, 30)));
    // The wheel scrolls it back up to the files.
    app.focus_list(false);
    app.files_focus = false;
    app.scroll_files(-100);
    let l = list(&mut app);
    assert!(l[1..].iter().any(|r| r.contains("big.rs")), "{l:?}");
}

#[test]
fn claude_makes_changes_on_their_pr() {
    // Someone else's PR: Claude can suggest the changes, or make them here all the same (for you
    // to push to their branch, when they let maintainers).
    let mut app = fixture();
    app.claude.path = Some(std::env::temp_dir().join("dv-unused-review.json"));
    app.theirs = true;
    if let Some(p) = &mut app.pr {
        (p.cross_repo, p.maintainer_can_modify) = (true, true);
    }
    let menu = |app: &mut App| {
        app.claude_menu();
        app.picker.take().unwrap().items.into_iter().map(|i| (i.label, i.target)).collect::<Vec<_>>()
    };
    let items = menu(&mut app);
    assert!(items.iter().any(|(l, t)| l.starts_with("Ask Claude to suggest") && matches!(t, Target::Claude(agent::Ask::Fix))));
    let (label, _) = items.iter().find(|(_, t)| matches!(t, Target::Claude(agent::Ask::Edit))).expect("make them");
    assert!(label.contains("Space g pushes them to their branch"), "{label}");
    // The prompt makes them (no "don't edit the code").
    let p = app.claude_prompt(agent::Ask::Edit, "rename it");
    assert!(p.contains("make the change") && !p.contains("don't edit"), "{p}");
    // They don't allow maintainer edits: said so.
    if let Some(p) = &mut app.pr {
        p.maintainer_can_modify = false;
    }
    assert!(menu(&mut app).iter().any(|(l, _)| l.contains("they don't allow maintainer edits")));
    // Your own PR: only "make the changes", as before.
    app.theirs = false;
    assert!(!menu(&mut app).iter().any(|(_, t)| matches!(t, Target::Claude(agent::Ask::Edit))));
}

#[test]
fn diff_base_follows_a_rebase() {
    // The PR rebased onto its base branch, moved on: the diff starts at the new fork point, not
    // the old one (which would show the base's own commits as the PR's).
    let dir = diff::scratch_repo("rebase");
    let git = |args: &[&str]| diff::git_in(&dir, args).unwrap().trim().to_owned();
    let commit = |f: &str| {
        std::fs::write(dir.join(f), f).unwrap();
        git(&["add", f]);
        git(&["commit", "-qm", f]);
        git(&["rev-parse", "HEAD"])
    };
    let a = commit("a");
    git(&["checkout", "-qb", "pr"]);
    commit("p");
    git(&["checkout", "-q", "main"]);
    let b = commit("b");
    git(&["update-ref", "refs/remotes/origin/main", &b]);
    git(&["checkout", "-q", "pr"]);
    git(&["rebase", "-q", "main"]);
    let mut app = fixture();
    (app.root, app.base, app.full_base, app.follow_base) = (dir.clone(), a.clone(), a.clone(), Some("origin/main".into()));
    app.follow_rebase();
    assert_eq!((app.base.as_str(), app.full_base.as_str()), (b.as_str(), b.as_str()));
    // Not a base you named, nor backwards.
    (app.base, app.full_base) = (b.clone(), b.clone());
    app.follow_base = Some(a.clone());
    app.follow_rebase();
    assert_eq!(app.full_base, b);
    _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn claude_input_box_empty_or_not() {
    // What's between Claude Code's last two rules, past its prompt: empty, or words waiting.
    let rule = "─".repeat(40);
    let screen = |input: &str| format!("● Done.\n\n{rule}\n{input}\n{rule}\n  ~/repo │ main\n");
    assert!(agent::input_empty(&screen("❯\u{a0}")));
    assert!(!agent::input_empty(&screen("❯ force push it")));
    assert!(!agent::input_empty("no box here")); // not found: not taken for empty
    // Claude Code's suggestion, dimmed or grey, isn't typed; text in the plain color is.
    assert!(agent::input_empty(&screen("\x1b[38;2;153;153;153m❯\u{a0}\x1b[2mfix the tests\x1b[22m")));
    assert!(agent::input_empty(&screen("❯ \x1b[38;2;136;136;136mTry \"refactor\"\x1b[39m")));
    assert!(!agent::input_empty(&screen("\x1b[38;2;153;153;153m❯\x1b[39m \x1b[38;2;255;255;255mforce push\x1b[0m")));
}

#[test]
fn a_question_about_the_pr_before_any_review() {
    // Space a → a question about the PR: with no review yet, it still shows in the Claude tab,
    // under "questions for Claude".
    let mut app = fixture();
    let path = std::env::temp_dir().join(format!("dv-question-{}.json", std::process::id()));
    std::fs::write(&path, "{}").unwrap();
    app.claude.path = Some(path.clone());
    app.claude_menu();
    assert!(app.picker.take().unwrap().items.iter().any(|i| matches!(i.target, Target::ClaudeQuestion)));
    crate::claude::add_conversation(&path, ON_SUMMARY, "Why a new trait?", "").unwrap();
    app.load_claude();
    app.toggle_view(View::Claude);
    // Attached to the description (no summary yet: your questions), what's under it with it.
    let head = app.rows.iter().position(|r| r.line.text == "questions for Claude").expect("its heading");
    assert!(app.rows[head].main.is_none() && app.rows[head].info.is_none()); // no summary to quote or post from it
    assert!(app.rows[head..].iter().any(|r| r.line.text.contains("Why a new trait?")));
    _ = std::fs::remove_file(&path);
}

#[test]
fn hidden_on_github_folded() {
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    let c = |id, hidden: &str| gh::Comment {
        id, node_id: format!("C{id}"), can_edit: false, can_delete: false, author: "amy".into(), body: format!("secret{id}"), created: String::new(), pending: false, reactions: vec![], hidden: hidden.into(),
    };
    app.threads = vec![Thread {
        id: "T".into(), path: "src/a.rs".into(), left: true, line: Some(2), start_line: None, original_line: Some(2), resolved: false, outdated: false,
        hunk: vec![], comments: vec![c(1, "spam"), c(2, "off topic"), c(3, "")], claude: None, severity: String::new(), about: None, conversation: String::new(), want: String::new(),
    }];
    app.build();
    let text = |app: &App| app.rows.iter().map(|r| r.line.text.clone()).collect::<Vec<_>>().join("\n");
    // Just their headings say so.
    assert!(!text(&app).contains("secret1") && !text(&app).contains("secret2") && text(&app).contains("secret3"));
    assert!(text(&app).contains("amy · hidden on GitHub (off topic)") && !text(&app).contains("(off topic) ·  ·"));
    // Tab on one: shown; again, hidden.
    let r = app.rows.iter().position(|r| r.line.text.contains("hidden on GitHub (off topic)")).unwrap();
    app.goto(r, 0);
    // A reply to it doesn't quote it unseen.
    app.pr = Some(gh::Pr { number: 1, node_id: "PR".into(), url: "https://github.com/o/r/pull/1".into(), ..Default::default() });
    app.start_reply(true);
    assert!(matches!(app.action.take(), Some(Action::Reply { ref quote, .. }) if quote.is_empty()));
    // Nor previewed for deleting, nor edited.
    (app.threads[0].comments[1].can_edit, app.threads[0].comments[1].can_delete) = (true, true);
    app.confirm_delete(app.threads[0].comments[1].clone());
    assert!(!app.picker.take().unwrap().items[1].label.contains("secret"));
    app.start_edit();
    assert!(app.action.is_none() && app.msg.starts_with("it's hidden on GitHub"));
    app.pr = None;
    app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert!(text(&app).contains("secret2"));
    // Into its text, Tab hides it again: the cursor on its heading.
    app.goto(r + 1, 0);
    app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert!(!text(&app).contains("secret2") && app.head.row == r);
    // On the thread's first line, Tab goes round: shown, hidden again, the thread folded; and
    // unfolded, it starts over.
    let first = app.rows.iter().position(|r| r.thread == Some(0)).unwrap();
    let tab = |app: &mut App| app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    app.goto(first, 0);
    tab(&mut app);
    assert!(text(&app).contains("secret1"));
    tab(&mut app);
    assert!(!text(&app).contains("secret1") && app.thread_open(0));
    tab(&mut app);
    assert!(!app.thread_open(0));
    tab(&mut app);
    assert!(app.thread_open(0) && !text(&app).contains("secret1"));
    tab(&mut app);
    assert!(text(&app).contains("secret1"));
    // The tabs say how many threads each shows.
    assert_eq!(app.pane_tabs().iter().map(|t| t.1.as_str()).collect::<Vec<_>>(), [" Comments 1 ", " Claude ", " Pending "]);
    assert_eq!(gh::hidden(&serde_json::json!({"isMinimized": true, "minimizedReason": "OFF_TOPIC"})), "off topic");
    assert_eq!(gh::hidden(&serde_json::json!({"isMinimized": false})), "");
}

#[test]
fn hidden_conversation_comment_folded() {
    let mut app = fixture();
    let line = |heading, md, text: &str, node: &str, hidden: &str| InfoLine { heading, md, text: text.into(), hidden: hidden.into(), node: node.into(), ..Default::default() };
    app.info.extend([activity_heading(1), line(true, false, "@bo · 1d ago", "IC_1", "spam"), line(false, true, "buy now", "", "")]);
    app.build();
    let s = screen(220, 45, |f| app.draw(f));
    let line = s.lines().find(|l| l.contains("hidden on GitHub (spam)")).unwrap_or_else(|| panic!("{s}"));
    assert!(line.contains("▸ @bo · hidden on GitHub (spam) · 1d ago") && !s.contains("buy now"), "{line}");
    let r = app.rows.iter().position(|r| r.line.text.contains("hidden on GitHub (spam)")).unwrap();
    app.goto(r, 0);
    // Not quoted unseen: `C` opens a comment without it.
    let pr = app.pr.take();
    app.pr = Some(gh::Pr { number: 1, node_id: "PR".into(), url: "u".into(), ..Default::default() });
    app.start_conversation_comment();
    assert!(matches!(app.action.take(), Some(Action::Converse { ref quote, .. }) if !quote.contains("buy now") && quote.contains("not quoted")));
    app.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
    assert!(app.msg.starts_with("it's hidden on GitHub") && !app.summary_draft.contains("buy now"));
    app.pr = pr;
    app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert!(!app.rows.iter().any(|r| r.line.text.contains("hidden on GitHub")) && app.rows.iter().any(|r| r.line.text == "buy now"));
}

#[test]
fn related_code_tinted_apart_from_the_cursor_line() {
    let mut app = fixture();
    let rel = Color::Rgb(1, 2, 3);
    (app.ui.related, app.ui.add_bg_rel) = (Some(rel), Color::Rgb(4, 5, 6));
    let add = app.ui.add_bg;
    let out = app.related_tint(vec![Span::raw("a"), Span::styled("b", Style::new().bg(add)), Span::styled("c", Style::new().bg(Color::Rgb(9, 9, 9)))]);
    assert_eq!(out.iter().map(|s| s.style.bg).collect::<Vec<_>>(), [Some(rel), Some(Color::Rgb(4, 5, 6)), Some(Color::Rgb(9, 9, 9))]);
}

#[test]
fn quitting_while_writing_beside_says_so_first() {
    let mut app = fixture();
    let ago = |ms| std::time::Instant::now() - std::time::Duration::from_millis(ms);
    // (As the run loop does.)
    let quit = |app: &mut App, k: KeyEvent| {
        app.press(k);
        app.hold_quit();
    };
    let (q, ctrl_c) = (KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE), KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
    app.writing = Some((Action::Shell, term::test_writing()));
    quit(&mut app, q);
    assert!(!app.quit && app.msg.starts_with("you're writing in the editor's pane"));
    // A held key's repeat doesn't leave; nor asking long after.
    quit(&mut app, q);
    assert!(!app.quit);
    app.quit_armed = Some(ago(10_000));
    quit(&mut app, q);
    assert!(!app.quit);
    // Another key between: asked again.
    app.quit_armed = Some(ago(1000));
    quit(&mut app, KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
    quit(&mut app, q);
    assert!(!app.quit);
    // Merging is held the same way.
    (app.merge, app.quit) = (true, true);
    app.hold_quit();
    assert!(!app.quit && !app.merge);
    // Said a moment ago: leaves.
    app.quit_armed = Some(ago(1000));
    quit(&mut app, ctrl_c);
    assert!(app.quit);
}

#[test]
fn unsent_text_kept_but_no_empty_files() {
    let e = keep_on_error(Err(anyhow!("down")), "my words").unwrap_err().to_string();
    let path = e.split("text saved to ").nth(1).unwrap().trim_end_matches(')');
    assert_eq!(std::fs::read_to_string(path).unwrap(), "my words");
    std::fs::remove_file(path).unwrap();
    assert_eq!(keep_on_error(Err(anyhow!("down")), "").unwrap_err().to_string(), "down");
}

#[test]
fn retarget_waits_for_a_rebase() {
    // The PR's base moved on GitHub while a rebase is stopped: the diff follows only after.
    let dir = diff::scratch_repo("retarget-wait");
    let mut app = fixture();
    app.root = dir.clone();
    app.pr = Some(gh::Pr { number: 1, base: "main".into(), ..Default::default() });
    (app.follow_base, app.github_base, app.job) = (Some(diff::remote_branch("main")), Some("dev".into()), None);
    let merge = dir.join(".git/rebase-merge");
    std::fs::create_dir_all(&merge).unwrap();
    app.follow_github_base(true);
    assert!(app.job.is_none() && app.msg.contains("once the rebase is through"), "{}", app.msg);
    // Through: it's fetched and followed.
    std::fs::remove_dir_all(&merge).unwrap();
    app.follow_github_base(false);
    assert!(matches!(app.job, Some(Job::Retarget(ref b)) if b == "dev"));
    // Not following the PR's base (a `--base` given): not held for a rebase either.
    std::fs::create_dir_all(&merge).unwrap();
    (app.job, app.follow_base) = (None, Some("v1.0".into()));
    app.follow_github_base(false);
    assert!(matches!(app.job, Some(Job::Retarget(_))));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_stopped_rebase_holds_only_what_changes_code() {
    let dir = diff::scratch_repo("rebase-holds");
    let mut app = fixture();
    app.root = dir.clone();
    std::fs::create_dir_all(dir.join(".git/rebase-merge")).unwrap();
    assert!(app.claude_blocked(Some(agent::Ask::Fix)).is_some() && app.claude_blocked(None).is_some());
    assert!(app.claude_blocked(Some(agent::Ask::Describe)).is_none());
    // Answering: held only when a reply waiting wants a change.
    assert_eq!(app.edits(agent::Ask::Answer), app.threads.iter().any(|t| agent::turn(t) == Some("[waiting] ") && t.want == "change"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn code_blocks_shaded_to_the_box_edge() {
    let mut app = fixture();
    let shade = Color::Rgb(1, 2, 3);
    app.ui.md.code_bg = Some(shade);
    (app.summary_draft, app.claude.summary) = (String::new(), String::new());
    app.threads[1].comments[0].body = "see:\n```\nx = 1\n```".into();
    app.build();
    let mut t = ratatui::Terminal::new(ratatui::backend::TestBackend::new(220, 45)).unwrap();
    t.draw(|f| app.draw(f)).unwrap();
    let buf = t.draw(|f| app.draw(f)).unwrap().buffer.clone();
    let y = (0..45).find(|&y| (0..220).map(|x| buf[(x, y)].symbol()).collect::<String>().contains("x = 1")).unwrap();
    let row: Vec<_> = (0..220).map(|x| (buf[(x, y)].symbol().to_owned(), buf[(x, y)].bg)).collect();
    let edge = row.iter().rposition(|c| c.0 == "│").unwrap();
    // Shaded from the code on, up to the box's side.
    assert!(row[edge - 1].1 == shade && row[edge - 20].1 == shade, "{:?}", &row[edge - 3..=edge]);
    assert_ne!(row[edge].1, shade);
}

#[test]
fn cursor_on_a_comment_stays_as_lines_come_before_it() {
    let mut app = fixture();
    let line = |heading, md, text: &str, node: &str| InfoLine { heading, md, text: text.into(), node: node.into(), ..Default::default() };
    app.info.extend([activity_heading(2), line(true, false, "@a · 2d ago", "IC_1"), line(false, true, "first", ""), line(true, false, "@b · 1d ago", "IC_2"), line(false, true, "second", "")]);
    app.build();
    let r = app.rows.iter().position(|r| r.line.text == "second").unwrap();
    app.goto(r, 0);
    let keep = app.cursor_key();
    // A refresh: a line before the activity (a reaction, a check).
    app.info.insert(1, line(false, false, "reactions 👀 1", ""));
    app.rebuild(keep);
    assert_eq!(app.rows[app.head.row].line.text, "second");
}

/// The heading of the PR's activity, `n` items under it.
fn activity_heading(n: usize) -> InfoLine {
    InfoLine { heading: true, text: format!("activity ({n})"), key: "activity".into(), ..Default::default() }
}

#[test]
fn a_failed_refresh_keeps_the_comments() {
    let mut app = fixture();
    let before = app.threads.len();
    assert!(app.threads.iter().any(|t| t.claude.is_none()));
    app.apply_github((Err(anyhow!("rate limited")), Err(anyhow!("rate limited"))), true);
    assert_eq!(app.threads.len(), before);
}

#[test]
fn latest_activity_counts_comments_not_events() {
    let mut app = fixture();
    let said = |who: &str, node: &str| vec![InfoLine { heading: true, text: format!("@{who} · 1d ago"), node: node.into(), url: format!("u-{node}"), key: node.into(), ..Default::default() }, InfoLine { md: true, text: format!("{who} said"), key: format!("{node}/text"), ..Default::default() }];
    let event = |what: &str| InfoLine { heading: true, event: true, text: format!("● @x {what}"), key: what.into(), ..Default::default() };
    app.info.push(activity_heading(7));
    for (w, n) in [("a", "IC_1"), ("b", "IC_2"), ("c", "IC_3"), ("d", "IC_4")] {
        app.info.extend(said(w, n));
    }
    app.info.extend([event("labelled"), event("pushed"), event("renamed")]);
    app.build();
    let text: Vec<&str> = app.rows.iter().map(|r| r.line.text.as_str()).collect();
    // The last three comments, and the events after them; not the first comment.
    assert!(["b said", "c said", "d said", "● @x labelled", "● @x renamed"].iter().all(|t| text.contains(t)), "{text:?}");
    assert!(!text.contains(&"a said"));
}
