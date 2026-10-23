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
    // A code block's background goes to the popup's edge as it's drawn (in a list too), not
    // the text after it; the popup stays as wide as its text.
    app.ui.md.code_bg = Some(Color::Indexed(236));
    let h = app.hover_lines("- step:\n  ```sh\n  cargo x\n  ```\n\ndocs");
    let w = h.iter().map(Line::width).max().unwrap();
    let h = super::render::code_to_edge(h, 40, app.ui.md.code_bg);
    let code = h.iter().find(|l| l.to_string().contains("cargo x")).unwrap();
    assert_eq!((code.width(), code.spans.last().unwrap().style.bg), (40, Some(Color::Indexed(236))));
    assert!(h.iter().any(|l| l.to_string().contains("docs") && l.width() < 40) && w < 40);
    // Inline code at a line's end, in a theme giving it the same background: not a block.
    app.ui.md.raw = app.ui.md.raw.bg(Color::Indexed(236));
    let h = super::render::code_to_edge(app.hover_lines("run `cargo test`"), 40, app.ui.md.code_bg);
    assert!(h[0].width() < 40);
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
    // Dismissed → marked again: at once, folded, and said "saving" till GitHub has it.
    assert!(app.job.is_none() && app.viewed["src/a.rs"] == "VIEWED" && app.collapsed.contains("src/a.rs"));
    assert_eq!(app.header_tag(0).unwrap().0, "✓ Viewed · saving… ");
    app.apply_viewed(viewed("src/a.rs", "DISMISSED")); // a refresh before it's landed: it stands
    assert_eq!(app.viewed["src/a.rs"], "VIEWED");
    let t = std::time::Instant::now();
    while !app.marking.is_empty() && t.elapsed().as_secs() < 5 {
        std::thread::sleep(std::time::Duration::from_millis(10));
        app.watch_marks();
    }
    assert_eq!(app.header_tag(0).unwrap().0, "✓ Viewed ");
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
    // A match on an entry's second line keeps the entry, whole.
    k(&mut app, KeyCode::Char('/'));
    k(&mut app, KeyCode::Esc);
    k(&mut app, KeyCode::Char('/'));
    "comes back".chars().for_each(|c| k(&mut app, KeyCode::Char(c)));
    let t = text(&app);
    assert!(t.iter().any(|l| l.trim_start().starts_with("Enter")) && t.iter().any(|l| l.contains("comes back")), "{t:?}");
    k(&mut app, KeyCode::Char('/'));
    k(&mut app, KeyCode::Esc);
    k(&mut app, KeyCode::Char('/'));
    "bracket".chars().for_each(|c| k(&mut app, KeyCode::Char(c)));
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
        path: "src/a.rs".into(), left, line, start_line: None, start_left: None, original_line: Some(1), resolved, talk_resolved: false, outdated: line.is_none(),
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
    app.select = false; // (as an action taken ends it)

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
    assert!(matches!(confirm.items[1].target, Target::Run(Job::DeleteComment(ref id)) if id == "C2"));
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
    assert!(matches!(p.items[0].target, Target::Nothing) && matches!(p.items[1].target, Target::Run(Job::DeleteComment(ref id)) if id == "C2"));
    app.picker = None;
    assert_eq!(clean_body("> unused\n\n<!-- dv: x -->\n", "<!-- dv:", "> unused\n\n"), ""); // untouched → cancel
    assert_eq!(clean_body("> unused\n\nagreed\n<!-- dv: x -->", "<!-- dv:", "> unused\n\n"), "> unused\n\nagreed");
    let tr = app.rows.iter().position(|r| r.thread == Some(0)).unwrap(); // rebuilt rows include the PR header now
    app.goto(tr, 0);
    press(&mut app, 'C');
    assert!(matches!(app.action.take(), Some(Action::Reply { review: false, .. })));
    press(&mut app, 'S');
    let labels: Vec<String> = app.picker.take().unwrap().items.into_iter().map(|i| i.label).collect();
    assert_eq!(labels[..4], ["Comment", "Approve", "Request changes", "Close the PR (with a comment, optional)"]); // no pending review → no discard option
    // Then what's still open: the thread here is unresolved (and the test's PR head isn't HEAD).
    assert_eq!(labels[4], "1 unresolved thread(s) on GitHub");
    assert!(labels[5].starts_with("your checkout isn't the PR's head") && labels.len() == 6);
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
    app.info = vec![InfoLine { key: "description".into(), ..line(true, false, "description") }, line(false, true, "")];
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
    // When it was written, on its heading; and of what, once the PR has moved on.
    app.claude.created = "2020-01-01T00:00:00Z".into();
    app.build();
    assert!(app.rows.iter().any(|r| r.line.text.starts_with("claude's review (") && r.line.text.ends_with("y ago)")));
    app.claude.head = "42995cfc0e".into();
    app.pr = Some(gh::Pr { head_oid: "5e1c0189b1".into(), ..Default::default() });
    app.view = View::Claude; // (with a PR, the summary's there)
    app.build();
    let s = screen(220, 40, |f| app.draw(f));
    assert!(s.contains("y ago · of 42995cf, before the PR's latest)"), "{s}");
    (app.pr, app.claude.created, app.claude.head, app.view) = (None, String::new(), String::new(), View::All);
    app.build();
    // Its file unreadable a moment (Claude mid-edit): what was read stays, and dv says why.
    let good = std::fs::read_to_string(&p).unwrap();
    std::fs::write(&p, &good[..good.len() / 2]).unwrap();
    app.load_claude();
    assert!(app.claude.broken.is_some() && app.msg.contains("can't be read"));
    assert_eq!((app.claude.summary.as_str(), app.threads.len()), ("Looks fine.", 2));
    std::fs::write(&p, &good).unwrap();
    app.load_claude();
    assert!(app.claude.broken.is_none());
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
    // Claude's comment is what's kept: Space r on it alone resolves nothing (Space d deletes it).
    press(&mut app, " r");
    assert!(app.msg.contains("no conversation to resolve") && !std::fs::read_to_string(&p).unwrap().contains("\"resolved\": true"));
    // With talk under it, Space r resolves that: the replies fold to a line, the comment stays.
    crate::claude::reply(&p, "c1", "Why not a guard here?", "ask").unwrap();
    app.load_claude();
    app.build();
    app.goto(row(&app, false), 0);
    let t = app.rows[app.head.row].thread.unwrap();
    assert!(app.rows.iter().any(|r| r.thread == Some(t) && r.guide && r.line.text.starts_with("Why not a guard"))); // (indented)
    press(&mut app, " r");
    assert!(std::fs::read_to_string(&p).unwrap().contains("\"talk_resolved\": true"));
    let t = app.rows[app.head.row].thread.unwrap();
    assert!(app.threads[t].talk_resolved && !app.threads[t].resolved && app.thread_open(t));
    assert!(app.rows.iter().any(|r| r.thread == Some(t) && r.line.text.contains("1 reply under it, resolved")));
    // Tab on that line shows the talk to read, without reopening it; Tab again folds it.
    let fold = app.rows.iter().position(|r| r.talk).unwrap();
    app.goto(fold, 0);
    app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert!(app.rows.iter().any(|r| r.guide && r.line.text.starts_with("Why not a guard")));
    assert!(std::fs::read_to_string(&p).unwrap().contains("\"talk_resolved\": true"));
    let reply = app.rows.iter().position(|r| r.guide).unwrap();
    app.goto(reply, 0); // (on a reply: on the comment, Tab folds the thread)
    app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert!(app.rows.iter().any(|r| r.talk));
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
    assert!(p.items.iter().any(|i| i.label.contains("Make the changes… (2 open comments"))); // the resolved one is skipped
    // Picking it asks how they land, then for your own instructions (an editor), then runs Claude.
    app.picker = None;
    app.pick_claude(agent::Ask::Fix);
    let p = app.picker.take().unwrap();
    assert!(matches!(p.items[1].target, Target::Land(agent::Ask::Fix, "amend")) && p.sel == 0);
    app.land(agent::Ask::Fix, "amend");
    assert!(matches!(app.action.take(), Some(Action::ClaudeWith(agent::Ask::Fix))));
    assert!(app.claude_prompt(agent::Ask::Fix, "").contains("`dv amend <sha> <files>`"));
    app.pick_claude(agent::Ask::Fix); // the last answer first, next time
    assert_eq!(app.picker.take().map(|p| p.sel), Some(1));
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
    let two = rows(&mut app, 210);
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
        id: "T".into(), path: "src/a.rs".into(), left: false, line: Some(2), start_line: None, start_left: None, original_line: Some(2),
        resolved: false, outdated: false, hunk: vec![], comments: vec![c(11), c(12)], claude: None, severity: String::new(), about: None, conversation: String::new(),
        talk_resolved: false,
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
    // Plain code: Enter opens it in your editor, there (said, with the rest).
    assert!(app.context_keys().is_some_and(|(what, keys)| what == "code" && keys[0].0 == "Enter"));
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
    app.claude.job = Some(agent::ClaudeJob::new(child, log, "answering", false));
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
    assert_eq!(crate::claude::claude_dirs(), [std::ffi::OsString::from("--add-dir"), notes.parent().unwrap().as_os_str().to_owned()]);
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
        id: String::new(), path: "src/a.rs".into(), left: false, line: Some(line), start_line: None, start_left: None, original_line: Some(line),
        resolved: false, outdated: false, hunk: vec![], comments, claude: None, severity: String::new(), about: None, conversation: String::new(),
        talk_resolved: false,
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
    let a = gh::Anchor { path: "src/a.rs".into(), line: 2, left: false, start: None, ..Default::default() };
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
    assert!(!app.threads[0].by_claude());
    assert_eq!(app.change_requests(), 1); // but it's a request "make the changes" acts on
    // `a` across removed and added lines: on the added side's lines (line numbers are per
    // side), from the first there.
    let del = app.rows.iter().position(|r| r.line.kind == Kind::Del).unwrap();
    let add = app.rows.iter().position(|r| r.line.kind == Kind::Add).unwrap();
    app.goto(del, 0);
    app.select = true;
    app.goto(add, 0);
    app.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
    let Some(Action::AskClaude { anchor, .. }) = &app.wanting else { panic!("{}", app.msg) };
    let first = app.rows[add].line.new.unwrap();
    assert!(!anchor.left);
    assert!(anchor.start.is_none() || anchor.start == Some((first, false)), "{:?} (first added: {first})", anchor.start);
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
        id: String::new(), path: "a.rs".into(), left: false, line: Some(line), start_line: None, start_left: None, original_line: Some(line),
        resolved: false, outdated: false, hunk: vec![], comments, claude: None, severity: String::new(), about: None, conversation: String::new(),
        talk_resolved: false,
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
        id: String::new(), path: "src/a.rs".into(), left: false, line: Some(2), start_line: None, start_left: None, original_line: Some(2),
        resolved: true, outdated: false, hunk: vec![], comments: vec![c], claude: None, severity: String::new(), about: None, conversation: String::new(),
        talk_resolved: false,
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
    use crate::claude::refusal;
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
    app.geom.picker = Some(mouse::PickerGeom { list: Rect { x: 10, y: 4, width: 60, height: 5 }, off: 0 });
    app.picker.as_mut().unwrap().lines = (0..5).map(Some).collect();
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
    app.land(agent::Ask::Fix, "");
    assert!(matches!(app.action.take(), Some(Action::ClaudeWith(agent::Ask::Fix))));
    assert!(!app.run_claude(agent::Ask::Fix, "  "));
    assert!(app.msg.starts_with("nothing to change") && app.claude.job.is_none());
    // While Claude is busy, it says so before the editor opens (so nothing written is lost).
    let child = std::process::Command::new("sleep").arg("5").spawn().unwrap();
    app.claude.job = Some(agent::ClaudeJob::new(child, std::env::temp_dir().join("dv-test-busy.log"), "reviewing", false));
    app.pick_claude(agent::Ask::Fix);
    assert!(app.action.is_none() && app.msg.contains("once Claude's done"));
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
    // On someone else's PR, Claude suggests the changes, unless you say to make them here.
    app.picker = None;
    app.theirs = true;
    app.claude_menu();
    assert!(app.picker.take().unwrap().items.iter().any(|i| i.label == "Make the changes… (3 open comments)"));
    app.pick_claude(agent::Ask::Fix);
    let p = app.picker.take().unwrap();
    assert!(matches!(p.items[p.sel].target, Target::Land(agent::Ask::Fix, agent::SUGGEST)));
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
    app.commits = vec![files::Commit { sha: "abcdef1234".into(), short: "abcdef1".into(), subject: "add parse".into(), at: "1".into() }];
    // `a` on it: what you want, then the editor, starting from which commit.
    app.ask_commit(0);
    // On no lines, no suggestion to post: ask (preselected), change, amend or commit.
    let p = app.picker.as_mut().unwrap();
    assert_eq!((p.items.len(), p.sel), (4, 0));
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
        id: "T1".into(), path: "src/reverted.rs".into(), left: false, line: Some(4), start_line: None, start_left: None, original_line: Some(4),
        resolved: false, outdated: false, hunk: vec![], comments: vec![c], claude: None, severity: String::new(), about: None, conversation: String::new(),
        talk_resolved: false,
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
    let a = Anchor { path: "src/a.rs".into(), line: 2, left: false, start: None, ..Default::default() };
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
    app.claude.job = Some(agent::ClaudeJob::new(child, log.clone(), "answering", true));
    app.poll_claude();
    _ = std::fs::remove_file(&log);
    assert_eq!(app.msg, "Claude is done answering (5 turns, $0.22)");
    assert!((app.claude.spent - 0.2231).abs() < 1e-9);
}

#[test]
fn claude_pane_asking() {
    // Claude Code's prompts, as dump-screen shows them: dv mustn't type into these.
    assert!(crate::pane::asking(" Do you want to make this edit to ensure.rs?\n ❯ 1. Yes\n   2. No"));
    assert!(crate::pane::asking(" Is this a project you created or one you trust?\n ❯ No, exit\n   Yes, I trust this folder"));
    assert!(!crate::pane::asking("● Done.\n───\n❯ \n───\n  ⏵⏵ auto mode on"));
    // At it: its spinner line (Claude Code's words vary), or the older "esc to interrupt".
    assert!(crate::pane::working("● Reading files\n· Moseying… (59s · ↓ 1.3k tokens)\n───\n❯ \n"));
    assert!(crate::pane::working("✻ Wibbling… (1m 9s · ↓ 3.4k tokens · thought for 2s)"));
    assert!(crate::pane::working("✶ Thinking… (esc to interrupt)"));
    // Done, waiting at its prompt (what you typed there and didn't send isn't work).
    assert!(!crate::pane::working("● Done… (see above)\n───\n❯ force push it\n───\n  ⏵⏵ auto mode on"));
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
    app.next_of('n', true);
    app.watch_seen();
    assert_eq!(new(&app), [true, true, false]); // still tagged while you're on it
    let on = app.head.row;
    app.next_of('n', true);
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
    app.seen.since = app.seen.since.map(|t| t - std::time::Duration::from_secs(5));
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
    app.info = vec![plain("OPEN @bo · main ← x"), plain("review: waiting on @cy"), InfoLine { key: "description".into(), ..heading("description") }, md("Long story")];
    app.claude.summary = "Looks fine.".into();
    let c = |author: &str| gh::Comment { id: 1, node_id: String::new(), can_edit: false, can_delete: false, author: author.into(), body: "hm".into(), created: String::new(), pending: false, reactions: vec![], hidden: String::new() };
    let t = |path: &str, line, claude: Option<&str>, author| gh::Thread {
        id: String::new(), path: path.into(), left: false, line: Some(line), start_line: None, start_left: None, original_line: Some(line),
        resolved: false, outdated: false, hunk: vec![], comments: vec![c(author)], claude: claude.map(Into::into), severity: String::new(), about: None, conversation: String::new(),
        talk_resolved: false,
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
    // Claude's activity: what it did, newest first: its review once (how many comments, its
    // summary's start), not each comment of it.
    let at = v.iter().position(|t| t.starts_with("◆ ") && t.ends_with("reviewed · 1 comment")).expect("its review");
    assert!(v.iter().any(|t| t == "claude's activity") && v[at + 1] == "  Looks fine.", "{v:?}");
    assert!(!v.iter().any(|t| t.starts_with("◆ ") && t.ends_with("big.rs:20")), "{v:?}");
    // Its keys there: going to it, not the PR text's (nothing of it to quote or react to).
    app.goto(app.rows.iter().position(|r| r.line.text.starts_with("◆ ")).unwrap(), 0);
    assert_eq!(app.context_keys().map(|k| k.0), Some("Claude's activity"));
    // A newer one landing above: the cursor stays on its comment (not its place in the box).
    let item = app.rows.iter().position(|r| r.line.text.starts_with("◆ ")).unwrap();
    app.goto(item + 1, 0);
    let was = app.rows[app.head.row].goes;
    let newer = gh::Comment { id: 2, node_id: String::new(), can_edit: false, can_delete: false, author: "claude".into(), body: "Later.".into(), created: "2099-01-01T00:00:00Z".into(), pending: false, reactions: vec![], hidden: String::new() };
    app.threads[0].comments.push(newer);
    let keep = app.cursor_key();
    app.rebuild(keep);
    // (A reply: its own entry, where it is.)
    assert!(app.rows.iter().any(|r| r.line.text.starts_with("◆ ") && r.line.text.ends_with("big.rs:20")));
    assert!(app.rows[app.head.row].goes == was && app.rows.iter().position(|r| r.line.text == "  Later.").unwrap() < app.head.row);
    app.threads[0].comments.pop();
    let keep = app.cursor_key();
    app.rebuild(keep);
    // Its answer to another's comment: its own entry; the review, only its summary then.
    app.threads[0].about = Some(5);
    let keep = app.cursor_key();
    app.rebuild(keep);
    let v = texts(&app);
    assert!(v.iter().any(|t| t.starts_with("◆ ") && t.ends_with("big.rs:20")) && v.iter().any(|t| t == "◆ reviewed"), "{v:?}");
    // Enter on it: its summary.
    app.goto(app.rows.iter().position(|r| r.line.text == "◆ reviewed").unwrap(), 0);
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.rows[app.head.row].info, Some(SUMMARY));
    app.threads[0].about = None;
    app.build();
    // Enter on the review: to its comment (wherever the cursor was on the line).
    app.goto(app.rows.iter().position(|r| r.line.text.starts_with("◆ ") && r.line.text.ends_with("reviewed · 1 comment")).unwrap(), 0);
    app.head.col = 3;
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.rows[app.head.row].thread, Some(0));
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
    // (No description here to sit beside: in the pane all the same.)
    assert!(app.rows[row].attached);
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
    assert!(app.picker.take().unwrap().items.iter().any(|i| matches!(i.target, Target::Do(Action::Replace(ref p, None)) if p == &[12, 15])));
    // a on a draft: a redraft, starting from a quote of what it's about.
    let note = app.rows.iter().position(|r| r.line.text == "Thanks! Handled in {replacement}.").unwrap();
    app.goto(note, 0);
    assert_eq!(app.context_keys().unwrap().0, "Claude's draft");
    app.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
    // What you want of it first: ask or change (nothing goes to GitHub: no suggestion), the change preselected.
    let picker = app.picker.take().unwrap();
    let wants: Vec<_> = picker.items.iter().map(|i| &i.target).collect();
    assert!(matches!(wants[..], [Target::Want("ask"), Target::Want("change")]) && picker.sel == 1);
    app.want("ask");
    match app.action.take() {
        Some(Action::Replace(prs, Some(("closing note", q, "ask")))) => assert!(prs == [12, 15] && q == "About the closing note:\n> Thanks! Handled in {replacement}.\n\n", "{q:?}"),
        _ => panic!("a on the closing note asks about it"),
    }
    let body = app.rows.iter().position(|r| r.line.text == "Replaces #12 and #15.").unwrap();
    app.goto(body, 0);
    app.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
    app.picker = None;
    app.want("change");
    assert!(matches!(app.action.take(), Some(Action::Replace(_, Some(("description", q, "change")))) if q.starts_with("About the description:")));
    // What you asked of the drafts shows with them, until a redraft takes it in and after.
    let mut r = app.replacement.clone().unwrap();
    r.asks = vec![crate::replace::Request { text: "About the description:\n> Replaces #12 and #15.\n\nSay why.".into(), on: "description".into(), ..Default::default() }];
    app.show_replacement(r);
    let keep = app.cursor_key();
    app.rebuild(keep);
    let text: Vec<&str> = app.rows.iter().map(|r| r.line.text.as_str()).collect();
    let said = app.rows.iter().position(|r| r.line.text == "Say why.").expect("what you asked");
    assert!(app.rows[said].attached && text.iter().any(|t| t.ends_with("not redrafted with it yet")), "{text:#?}");
    // Under way: only what the draft was given is being redrafted with; what came after waits.
    let mut r = app.replacement.clone().unwrap();
    r.asks.push(crate::replace::Request { text: "Shorter.".into(), want: "change".into(), ..Default::default() });
    r.asks.push(crate::replace::Request { text: "Why two?".into(), want: "ask".into(), answer: "One per original.".into(), ..Default::default() });
    r.taking = 1;
    let claim = crate::claude::claim(&crate::claude::rep_key(r.id), "drafting", None).unwrap().unwrap();
    app.show_replacement(r);
    let keep = app.cursor_key();
    app.rebuild(keep);
    let text: Vec<String> = app.rows.iter().map(|r| r.line.text.clone()).collect();
    let at = |s: &str| text.iter().position(|t| t.contains(s)).unwrap();
    // (Answered: done, Claude's answer under what you asked, in its box.)
    assert!(at("Claude is redrafting with it") < at("Say why.") && at("Say why.") < at("[change] · not redrafted with it yet") && at("[change] · not redrafted with it yet") < at("Shorter."), "{text:#?}");
    assert!(at("Shorter.") < at("[ask] · answered") && at("[ask] · answered") < at("Why two?") && at("Why two?") < at("◆ claude") && at("◆ claude") < at("One per original."), "{text:#?}");
    // a on the line saying what it replaces: its draft's, the description's (not quoted).
    let replaces = app.rows.iter().position(|r| r.line.text.starts_with("replaces #12")).unwrap();
    assert_eq!(app.replacement_draft(replaces), Some("description"));
    app.goto(replaces, 0);
    app.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
    app.picker = None;
    app.want("ask");
    assert!(matches!(app.action.take(), Some(Action::Replace(_, Some(("description", q, "ask")))) if q == "About the description:\n\n"), "only the draft's text is quoted");
    // Each ask its own box (one after another on the same draft).
    let tops = (0..app.rows.len()).filter(|&r| app.rows[r].line.text.starts_with("you · asked of the drafts") && app.box_part(r) == Some(BoxPart::Top)).count();
    assert_eq!(tops, 3);
    // a on a box: about the draft it's beside, as on the draft, not quoting the box as its text.
    app.goto(at("Why two?"), 0);
    assert_eq!(app.replacement_draft(app.head.row), Some("description"));
    app.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
    app.picker = None;
    app.want("ask");
    assert!(matches!(app.action.take(), Some(Action::Replace(_, Some(("description", q, "ask")))) if q == "About the description:\n\n"), "the box quoted");
    drop(claim);
    // A change taken in without Claude saying so: dv's note, not as Claude's answer.
    let mut r = app.replacement.clone().unwrap();
    r.asks = vec![crate::replace::Request { text: "Tidy it.".into(), want: "change".into(), answer: crate::replace::NO_ANSWER.into(), ..Default::default() }];
    app.show_replacement(r);
    let keep = app.cursor_key();
    app.rebuild(keep);
    let text: Vec<String> = app.rows.iter().map(|r| r.line.text.clone()).collect();
    assert!(text.iter().any(|t| t.contains("(redrafted with it)")) && !text.iter().any(|t| t == "◆ claude"), "{text:#?}");
    // Claude's review is beside the description, in the pane, as on a PR's Claude view.
    app.claude.summary = "**What it does.** Builds for the target.".into();
    let keep = app.cursor_key();
    app.rebuild(keep);
    let review = app.rows.iter().position(|r| r.line.text == "claude's review").expect("claude's review");
    assert!(app.rows[review].attached && app.rows[review].main.is_none());
    // Published, on its PR: its closing note is still there, while what it replaces is open.
    let mut r = app.replacement.clone().unwrap();
    r.published = Some(20);
    let draft_pr = app.pr.replace(gh::Pr { number: 20, ..Default::default() });
    app.show_replacement(r);
    let keep = app.cursor_key();
    app.rebuild(keep);
    assert!(app.rows.iter().any(|r| r.main == Some(CLOSING_NOTE) && r.line.text.starts_with("closing note") && r.line.text.contains("S closes")));
    app.pr = draft_pr;
    let mut r = app.replacement.clone().unwrap();
    r.published = None;
    app.show_replacement(r);
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
    assert!(items[0].0.starts_with("Publish as a draft PR (rebased on origin/") && matches!(items[0].1, Target::Do(Action::Publish(false))));
    assert!(items[1].0 == "Publish, then close #12, #15 with the closing note…" && matches!(items[1].1, Target::Do(Action::Publish(true))));
    assert!(!items.iter().any(|i| i.0.contains("ready for review")));
    std::fs::write(clean.join("x"), "").unwrap();
    let items = s(&mut app);
    assert!(items[0].0.starts_with("Commit the uncommitted changes first") && matches!(items[0].1, Target::Do(Action::Commit)));
    // (A new file only: nothing to amend into a commit.)
    assert_eq!(items.len(), 1); // nothing to publish until then
    std::fs::write(clean.join("tracked"), "").unwrap();
    crate::diff::git_in(&clean, &["add", "tracked"]).unwrap();
    crate::diff::git_in(&clean, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "t"]).unwrap();
    std::fs::write(clean.join("tracked"), "changed").unwrap();
    let items = s(&mut app);
    assert!(matches!(items[1].1, Target::AmendMenu) && matches!(items[2].1, Target::AmendAll), "changes to tracked files: amend them in, too");
    // Space g on a draft: no pushing or pulling (it's published with S), and messages say S.
    app.git_menu();
    let items = app.picker.take().unwrap().items;
    assert!(!items.iter().any(|i| matches!(i.target, Target::Run(Job::Push(_) | Job::Pull))), "{:?}", items.iter().map(|i| &i.label).collect::<Vec<_>>());
    assert!(items.iter().any(|i| i.label == "Can't push: it's a draft: S publishes it"));
    assert_eq!(app.next_step(true), "S publishes it");
    // Nor does Claude's review there offer what goes to a PR (there's none).
    let review = app.rows.iter().position(|r| r.info == Some(SUMMARY)).unwrap_or(0);
    if app.rows[review].info == Some(SUMMARY) {
        app.goto(review, 0);
        assert!(!app.context_keys().unwrap().1.iter().any(|k| k.0 == "C"));
    }
    _ = std::fs::remove_dir_all(&clean);
    let mut r = app.replacement.clone().unwrap();
    (r.published, r.closed) = (Some(20), vec![12]);
    app.show_replacement(r);
    app.build();
    let items = s(&mut app);
    assert_eq!(items[0].0, "Published as PR #20");
    assert!(items[1].0 == "Close #15 with the closing note (now)…" && matches!(items[1].1, Target::Do(Action::CloseOriginals)));
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
        id: String::new(), path: "a.rs".into(), left: false, line: Some(4), start_line: Some(2), start_left: None, original_line: Some(4),
        resolved: false, outdated: false, hunk: vec![], comments: vec![c], claude: None, severity: String::new(), about: None, conversation: String::new(),
        talk_resolved: false,
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
        id: String::new(), path: "src/a.rs".into(), left: false, line: Some(2), start_line: None, start_left: None, original_line: Some(2),
        resolved: false, outdated: false, hunk: vec![], comments: vec![c], claude: Some("c1".into()), severity: String::new(), about: None, conversation: String::new(),
        talk_resolved: false,
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
    app.info = vec![line(false, false, "OPEN @bo · main ← fix · +2 −2 in 3 files"), line(false, false, "review: waiting on @cy"), InfoLine { key: "description".into(), ..line(true, false, "description") }, line(false, true, "Fixes **the thing**, see `new_call`.")];
    app.claude.summary = "Looks right; one nit.".into();
    app.summary_draft = "Nearly there.".into();
    // Each comment its own id, as on GitHub and in Claude's review (dv keys threads by them).
    let id = std::cell::Cell::new(0);
    let c = |author: &str, body: &str, pending| gh::Comment { id: { id.set(id.get() + 1); id.get() }, node_id: String::new(), can_edit: true, can_delete: true, author: author.into(), body: body.into(), created: String::new(), pending, reactions: vec![], hidden: String::new() };
    let t = |path: &str, line, comments, claude: Option<&str>, resolved| gh::Thread {
        id: String::new(), path: path.into(), left: false, line: Some(line), start_line: None, start_left: None, original_line: Some(line),
        resolved, talk_resolved: false, outdated: false, hunk: vec![], comments, claude: claude.map(Into::into), severity: if claude.is_some() { "nit".into() } else { String::new() }, about: None, conversation: String::new(),
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
        let r = gh::Review { pr_id: "PR".into(), mine: true, me: "you".into(), pending: None, pending_body: String::new(), pending_on: String::new(), threads, viewed: Default::default() };
        (Ok(r), Ok(Info { lines: app_info(), title: "Fix the thing".into(), head: "abc".into(), base_oid: String::new(), url: String::new(), state: ("OPEN", "238636"), unsigned: Some(vec![]), awaiting: 0, failing: 0, base: String::new(), conflicts: false, updated: String::new(), fixes: vec![] }))
    };
    let replies = |app: &App| app.threads.iter().flat_map(|t| &t.comments).filter(|c| c.body == "Fair.").count();
    // Mid-way through something (here a picker): it waits. (What's shown came from the cache.)
    (app.github_stale, app.msg) = (true, github::STALE.into());
    tx.send((0, fetched(newer.clone()))).unwrap();
    app.threads_picker(THREADS_ALL);
    assert!(app.picker.is_some());
    app.watch();
    assert_eq!(replies(&app), 0);
    app.picker = None;
    app.watch();
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
    // The mouse sideways over it (Shift and the wheel): that block scrolls, from its start.
    use ratatui::crossterm::event::{MouseEvent, MouseEventKind};
    app.goto(0, 0);
    let s = screen(220, 45, |f| app.draw(f));
    let (y, line) = s.lines().enumerate().find(|(_, l)| l.contains("xxx")).unwrap();
    let x = line.chars().position(|c| c == 'x').unwrap() as u16;
    app.mouse(MouseEvent { kind: MouseEventKind::ScrollDown, column: x, row: y as u16, modifiers: KeyModifiers::SHIFT });
    let t = screen(220, 45, |f| app.draw(f));
    // (The cursor, scrolled off, at its first column shown: there, not `‹`.)
    assert_eq!((app.code_left, app.head.col), (8, 8), "{}", t.lines().nth(y).unwrap());
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
    app.claude.job = Some(agent::ClaudeJob::new(child, "/nonexistent/run.json".into(), "reviewing", false));
    // The menu offers to stop it, first; checking says it's still at it.
    app.claude.path = Some(std::env::temp_dir().join(format!("dv-stuck-{}.json", std::process::id())));
    app.claude_menu();
    let p = app.picker.take().unwrap();
    let items = p.items;
    assert!(items.iter().any(|i| matches!(i.target, Target::ClaudeStop) && i.label.starts_with("Stop Claude (reviewing for ")));
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
    app.commits = (1..=3).map(|n| files::Commit { sha: format!("{n}{}", "0".repeat(39)), short: format!("c{n}"), at: String::new(), subject: format!("commit {n}") }).collect();
    let s = draw(&mut app);
    // Under the files: the heading, "all commits" (shown), then each, oldest first.
    let list: Vec<String> = s.lines().map(|l| l.chars().take(app.files_w - 1).collect::<String>().trim_end().to_owned()).collect();
    let at = list.iter().position(|l| l.starts_with(" COMMITS 3")).expect(&s);
    assert_eq!(list[at + 1].trim_start_matches('▌').trim(), "all commits");
    assert!(list[at + 2].contains("c1 commit 1") && list[at + 4].contains("c3 commit 3"), "{list:?}");
    // An unsigned commit is marked.
    app.unsigned = vec![app.commits[1].sha.clone()];
    let s = draw(&mut app);
    assert!(s.contains("c2 ✗ commit 2"), "{s}");
    app.unsigned.clear();
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
    assert!(app.on_head().err().is_some_and(|e| e.to_string().starts_with("comments go on the PR's head")));
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
    // Commits shown: each one's message is a section before the files, and the PR's text (the
    // description, what's beside it) isn't there; they're in the list too; Tab on a header folds it.
    (app.commit_shown, app.commit_msgs) = (Some((0, 1)), vec!["commit 1".into(), "commit 2\n\nWhy it's done this way.".into()]);
    app.build();
    assert!(!app.rows.iter().any(|r| r.main == Some(DESCRIPTION) || r.attached), "the PR's text, with some commits");
    let c1 = app.rows.iter().position(|r| r.main.is_some() && r.line.text == "commit c1").expect("c1's section");
    let head = app.rows.iter().position(|r| r.main.is_some() && r.line.text == "commit c2").expect("c2's section");
    assert!(c1 < head && app.rows[head + 1..].iter().take_while(|r| r.main.is_some()).any(|r| r.line.text.contains("Why it's done")));
    let spots: Vec<files::Spot> = app.file_entries().into_iter().map(|e| e.spot).take(2).collect();
    assert_eq!(spots, [files::Spot::Section(1), files::Spot::Section(2)]);
    // Only what isn't committed: no PR text either.
    let (base, shown) = (std::mem::replace(&mut app.base, "HEAD".into()), app.commit_shown.take());
    app.build();
    assert!(!app.rows.iter().any(|r| r.main == Some(DESCRIPTION) || r.attached), "the PR's text, uncommitted");
    (app.base, app.commit_shown) = (base, shown);
    app.build();
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
    assert_eq!(heading.line.text, "questions about PR #7's conversation");
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
    let items: Vec<Item> = app.picker.take().unwrap().items.into_iter().filter(|i| i.section == "Description").collect();
    assert!(matches!(items[0].target, Target::DescApprove(false)) && matches!(items[2].target, Target::DescDiscard));
    app.approve_description(false);
    assert!(app.picker.take().unwrap().items.iter().any(|i| matches!(i.target, Target::DescApprove(true))));
    app.goto(head + 1, 0);
    app.start_ask_claude();
    assert!(matches!(app.action.take(), Some(Action::ClaudeWith(agent::Ask::Describe))), "{}", app.msg);
    // Discarded: gone from the file, and the description is as it was.
    app.discard(&["description"], "Claude's proposed description");
    assert!(!std::fs::read_to_string(&path).unwrap().contains("description"));
    assert!(app.rows.iter().any(|r| r.main.is_some() && r.line.text == "description"));
    _ = std::fs::remove_file(&path);
}

#[test]
fn claude_tab_says_the_rankings_next_step() {
    let mut app = fixture();
    app.ranked_next = Some(("Review #2258; this closes when it merges".into(), crate::claude::now_secs() - 7200));
    app.view = View::Claude;
    app.build();
    let s = screen(160, 40, |f| app.draw(f));
    assert!(s.contains("next, as Claude ranked it (2h ago)") && s.contains("Review #2258; this closes when it merges"), "{s}");
}

#[test]
fn commit_picker_lists_the_commits() {
    let mut app = fixture();
    app.commits.clear();
    app.commit_picker();
    assert_eq!(app.msg, "no commits here to pick from");
    let c = |sha: &str, subject: &str| super::files::Commit { sha: sha.into(), short: sha.into(), subject: subject.into(), at: String::new() };
    app.commits = vec![c("aaa", "first"), c("bbb", "second")];
    app.commit_picker();
    let p = app.picker.as_ref().unwrap();
    assert!(p.items[0].label.contains("all commits") && p.items[0].label.starts_with('●'));
    assert_eq!(p.items.len(), app.commits.len() + 1 + usize::from(app.dirty.has()));
    assert!(p.items[1].label.contains("bbb second")); // newest first
    assert!(matches!(&p.items[1].target, Target::CommitRow(2)));
}

#[test]
fn conflicts_on_the_state_chip() {
    let mut app = fixture();
    app.pr_state = Some(("OPEN", "238636"));
    assert_eq!(app.header_tag(INFO).map(|t| t.0), Some(" OPEN ".into()));
    app.conflicts = true;
    assert_eq!(app.header_tag(INFO).map(|t| t.0), Some(" OPEN · conflicts ".into()));
    app.pr_state = Some(("MERGED", "8957e5")); // (not after it's merged)
    assert_eq!(app.header_tag(INFO).map(|t| t.0), Some(" MERGED ".into()));
    // GitHub's word, from the PR's info.
    assert!(parse_info(&serde_json::json!({ "mergeable": "CONFLICTING" })).conflicts);
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
    let items: Vec<Item> = app.picker.take().unwrap().items.into_iter().filter(|i| i.section == "Closing").collect();
    assert!(matches!(items[0].target, Target::Do(Action::ClosePr)) && matches!(items[1].target, Target::Claude(agent::Ask::Close)) && matches!(items[2].target, Target::CloseDiscard));
    app.goto(head + 1, 0);
    app.start_ask_claude();
    assert!(matches!(app.action.take(), Some(Action::ClaudeWith(agent::Ask::Close))), "{}", app.msg);
    // Discarded: gone; Space a offers to draft one.
    app.discard(&["closing"], "Claude's closing comment");
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
    assert!(items.iter().any(|i| matches!(i.target, Target::CloseDiscard)) && !items.iter().any(|i| matches!(i.target, Target::Do(Action::ClosePr))));
    _ = std::fs::remove_file(&path);
}

#[test]
fn claudes_badge_gives_way_to_what_s_said() {
    // A long run's badge on a narrow bar: what's said keeps its room, the badge is cut.
    let mut app = fixture();
    let child = std::process::Command::new("sleep").arg("5").spawn().unwrap();
    app.claude.job = Some(agent::ClaudeJob::new(child, std::env::temp_dir().join("dv-test-badge.log"), "resolving the rebase's conflicts", false));
    app.msg = App::busy_says("amend (its changes would be half made)");
    let s = screen(80, 12, |f| app.draw(f));
    let bar = s.lines().last().unwrap();
    assert!(bar.contains("amend") && bar.contains("◐ claude") && bar.contains("1/"), "{bar}");
    if let Some(mut job) = app.claude.job.take() {
        _ = job.child.kill();
        _ = job.child.wait();
    }
}

#[test]
fn claude_working_on_the_status_bar_out_of_sync_on_the_title() {
    // How the checkout stands shows on the pinned title, a badge, a long title cut to make room;
    // what Claude's doing on the status bar (always, a badge), not the title.
    let mut app = fixture();
    app.sync = "↑2 unpushed · ✎ uncommitted".into();
    app.claude.pane_busy = true;
    let s = screen(160, 12, |f| app.draw(f));
    let title = s.lines().next().unwrap();
    assert!(!title.contains("◐ claude") && title.contains("↑2 unpushed · ✎ uncommitted"), "{title}");
    assert!(s.lines().last().unwrap().contains("◐ claude working (its pane)"), "{s}");
    let s = screen(60, 12, |f| app.draw(f));
    assert!(s.lines().last().unwrap().contains("◐ claude working (its pane)"), "{s}");
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
    // r on it, likewise: the reply's draft starts with what's selected, quoted.
    app.goto(ans, 0);
    app.key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE));
    assert!(matches!(app.action.take(), Some(Action::ClaudeReply(k, "", q)) if k == "c7" && q.starts_with("> Because it's minimal.")));
    assert!(!app.select);
    // The whole diff (Comments): not there.
    app.bar_to(Bar::Comments);
    assert!(!app.rows.iter().any(|r| r.line.text.contains("why approve?")));
}

#[test]
fn key_hints_at_the_bottom_of_the_contents() {
    let mut app = fixture();
    app.files_pref = Some(true);
    app.commits = (1..=2).map(|n| files::Commit { sha: format!("{n}{}", "0".repeat(39)), short: format!("c{n}"), at: String::new(), subject: format!("commit {n}") }).collect();
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
    let files_end = l.iter().position(|r| r.contains("img.png")).unwrap();
    let commits = l.iter().position(|r| r.starts_with(" COMMITS 2")).unwrap();
    assert!(commits == files_end + 2 && commits < keys, "{l:?}");
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
    app.commits = (1..=40).map(|n| files::Commit { sha: format!("{n:040}"), short: format!("c{n}"), at: String::new(), subject: format!("commit {n}") }).collect();
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
    // Someone else's PR: making the changes asks how, suggested (the first) or made here all
    // the same (for you to push to their branch, when they let maintainers).
    let mut app = fixture();
    app.claude.path = Some(std::env::temp_dir().join("dv-unused-review.json"));
    app.theirs = true;
    if let Some(p) = &mut app.pr {
        (p.cross_repo, p.maintainer_can_modify) = (true, true);
    }
    let lands = |app: &mut App| {
        app.pick_claude(agent::Ask::Fix);
        app.picker.take().unwrap().items.into_iter().map(|i| (i.label, i.target)).collect::<Vec<_>>()
    };
    let items = lands(&mut app);
    assert!(items[0].0.starts_with("As suggestions") && matches!(items[0].1, Target::Land(agent::Ask::Fix, agent::SUGGEST)));
    let (label, _) = items.iter().find(|(_, t)| matches!(t, Target::Land(agent::Ask::Edit, "commit"))).expect("make them");
    assert!(label.contains("Space g pushes them to their branch"), "{label}");
    // The prompt makes them (no "don't edit the code").
    let p = app.claude_prompt(agent::Ask::Edit, "rename it");
    assert!(p.contains("make the change") && !p.contains("don't edit"), "{p}");
    // They don't allow maintainer edits: said so.
    if let Some(p) = &mut app.pr {
        p.maintainer_can_modify = false;
    }
    assert!(lands(&mut app).iter().any(|(l, _)| l.contains("they don't allow maintainer edits")));
    // Your own PR: made here, no suggesting.
    app.theirs = false;
    assert!(lands(&mut app).iter().all(|(_, t)| matches!(t, Target::Land(agent::Ask::Fix, k) if *k != agent::SUGGEST)));
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
    assert!(crate::pane::input_empty(&screen("❯\u{a0}")));
    assert!(!crate::pane::input_empty(&screen("❯ force push it")));
    assert!(!crate::pane::input_empty("no box here")); // not found: not taken for empty
    // Claude Code's suggestion, dimmed or grey, isn't typed; text in the plain color is.
    assert!(crate::pane::input_empty(&screen("\x1b[38;2;153;153;153m❯\u{a0}\x1b[2mfix the tests\x1b[22m")));
    assert!(crate::pane::input_empty(&screen("❯ \x1b[38;2;136;136;136mTry \"refactor\"\x1b[39m")));
    assert!(!crate::pane::input_empty(&screen("\x1b[38;2;153;153;153m❯\x1b[39m \x1b[38;2;255;255;255mforce push\x1b[0m")));
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
        id: "T".into(), path: "src/a.rs".into(), left: true, line: Some(2), start_line: None, start_left: None, original_line: Some(2), resolved: false, talk_resolved: false, outdated: false,
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
    assert_eq!(app.pane_tabs().iter().map(|t| t.1.as_str()).collect::<Vec<_>>(), [" Comments 0/1 ", " Claude ", " Pending "]);
    // How many are resolved of them.
    app.threads[0].resolved = true;
    assert_eq!(app.pane_tabs()[0].1, " Comments 1/1 ");
    // Outdated (its code changed since): done too, folded as a resolved one is.
    (app.threads[0].resolved, app.threads[0].outdated) = (false, true);
    assert_eq!(app.pane_tabs()[0].1, " Comments 1/1 ");
    app.threads[0].outdated = false;
    app.threads[0].resolved = false;
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
    app.quit_armed = Some((ago(10_000), None));
    quit(&mut app, q);
    assert!(!app.quit);
    // Another key between: asked again.
    app.quit_armed = Some((ago(1000), None));
    quit(&mut app, KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
    quit(&mut app, q);
    assert!(!app.quit);
    // Merging is held the same way, with the hold, kept while you ask again (`Space g` is a
    // sequence)…
    (app.merge, app.quit) = (Some(1), true);
    app.hold_quit();
    assert!(!app.quit && app.merge.is_none() && app.quit_armed.is_some_and(|(_, m)| m == Some(1)));
    app.quit_armed = app.quit_armed.map(|(_, m)| (ago(1000), m));
    app.press(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
    assert!(app.quit_armed.is_some());
    app.pending = None;
    // Said a moment ago: leaves (merging).
    quit(&mut app, ctrl_c);
    assert!(app.quit && app.merge == Some(1));
    // …another key between: neither.
    (app.quit, app.merge) = (true, Some(1));
    app.hold_quit();
    app.quit_armed = app.quit_armed.map(|(_, m)| (ago(1000), m));
    quit(&mut app, KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
    assert!(app.quit_armed.is_none() && app.merge.is_none());
    // A held key's repeat keeps the merge held; another PR's merge asked is asked anew.
    (app.merge, app.quit) = (Some(1), true);
    app.hold_quit();
    quit(&mut app, q); // (at once: a repeat)
    assert!(!app.quit && app.quit_armed.is_some_and(|(_, m)| m == Some(1)) && app.msg.ends_with("merging #1)"));
    app.quit_armed = app.quit_armed.map(|(_, m)| (ago(1000), m));
    (app.merge, app.quit) = (Some(2), true);
    app.hold_quit();
    assert!(!app.quit && app.quit_armed.is_some_and(|(_, m)| m == Some(2)));
    app.quit_armed = app.quit_armed.map(|(_, m)| (ago(1000), m));
    quit(&mut app, q);
    assert!(app.quit && app.merge == Some(2));
    app.quit = false;
    // The writing over: a quit later is only that.
    (app.merge, app.quit) = (Some(1), true);
    app.hold_quit();
    app.writing = None;
    quit(&mut app, q);
    assert!(app.quit && app.merge.is_none());
}

#[test]
fn unsent_text_kept_but_no_empty_files() {
    let e = keep_on_error::<()>(Err(anyhow!("down")), "my words").unwrap_err().to_string();
    let path = e.split("text saved to ").nth(1).unwrap().trim_end_matches(')');
    assert_eq!(std::fs::read_to_string(path).unwrap(), "my words");
    std::fs::remove_file(path).unwrap();
    assert_eq!(keep_on_error::<()>(Err(anyhow!("down")), "").unwrap_err().to_string(), "down");
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
    assert_eq!(app.edits(agent::Ask::Answer), app.threads.iter().any(|t| t.waiting() && t.want == "change"));
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
    // The cursor on it: the cursor line's tint, all along it.
    let cur = Color::Rgb(7, 7, 7);
    (app.ui.cursorline, app.settings.cursorline) = (Some(cur), true);
    let r = app.rows.iter().position(|r| r.line.text.contains("x = 1")).unwrap();
    app.goto(r, 0);
    let buf = t.draw(|f| app.draw(f)).unwrap().buffer.clone();
    let y = (0..45).find(|&y| (0..220).map(|x| buf[(x, y)].symbol()).collect::<String>().contains("x = 1")).unwrap();
    assert!(buf[(edge as u16 - 1, y)].bg == cur && buf[(edge as u16 - 20, y)].bg == cur);
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
    // One comment and many events after it: the latest nine, not all.
    app.info.truncate(app.info.iter().position(|l| l.key == "activity").unwrap() + 1);
    app.info.extend(said("a", "IC_1"));
    app.info.extend((0..20).map(|i| event(&format!("pushed {i}"))));
    app.build();
    let shown = app.rows.iter().filter(|r| r.line.text.starts_with("● @x pushed")).count();
    assert_eq!(shown, 9);
}

#[test]
fn labels_picker_comes_back_as_left() {
    let mut app = fixture();
    let on = |xs: &[(&str, bool)]| xs.iter().map(|(x, o)| (x.to_string(), *o)).collect::<Vec<_>>();
    app.fields = Some((on(&[("bug", true), ("docs", false)]), on(&[("amy", false)]), on(&[("bo", false)])));
    app.fields_menu("o".into(), Some((gh::Field::Label, "docs".into())));
    let p = app.picker.as_ref().unwrap();
    let on = |app: &App| app.picker.as_ref().map(|p| p.items[app.matches()[p.sel]].label.clone());
    assert_eq!((p.query.as_str(), on(&app).as_deref()), ("o", Some("  docs")));
    // Enter on one: its job, and which it was kept for the picker to come back on.
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(matches!(app.job, Some(Job::SetField(..))) && app.fields_at == Some(("o".into(), gh::Field::Label, "docs".into())));
    // Back, docs now on (sorted first): the cursor on it still, not on what's in its old place.
    app.fields.as_mut().unwrap().0[1].1 = true;
    app.fields_menu("o".into(), Some((gh::Field::Label, "docs".into())));
    assert_eq!(on(&app).as_deref(), Some("✓ docs"));
}

#[test]
fn picker_matches_words_not_scattered_letters() {
    let mut app = fixture();
    let items = ["Draft a comment closing the PR… (you edit it, then close)", "Update Claude's review", "Check Claude's review now"];
    app.picker = Some(Picker::new(" claude ", items.iter().map(|l| Item::new(*l, Target::Nothing)).collect()));
    let found = |app: &mut App, q: &str| {
        app.picker.as_mut().unwrap().query = q.into();
        app.matches().iter().map(|&i| app.picker.as_ref().unwrap().items[i].label.clone()).collect::<Vec<_>>()
    };
    // "close": only what says it ("Update Claude's review" has its letters, scattered).
    assert_eq!(found(&mut app, "close"), [items[0]]);
    // Words in any order; what has the query whole first.
    assert_eq!(found(&mut app, "review check"), [items[2]]);
    assert_eq!(found(&mut app, "claude's review"), [items[1], items[2]]);
}

#[test]
fn a_failed_reload_is_said_after_what_was_done() {
    use super::git::with_reload;
    assert_eq!(with_reload("amended (was abc: dv-backup/x)".into(), Ok(())), "amended (was abc: dv-backup/x)");
    let said = with_reload("amended (was abc: dv-backup/x)".into(), Err(anyhow!("git diff failed")));
    assert!(said.starts_with("amended (was abc: dv-backup/x)") && said.contains("git diff failed"), "{said}");
    assert_eq!(with_reload(String::new(), Err(anyhow!("git diff failed"))), "git diff failed");
}

#[test]
fn one_reply_to_several_claude_comments() {
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    let p = std::env::temp_dir().join(format!("dv-claude-many-{}.json", std::process::id()));
    std::fs::write(&p, r#"{ "summary": "s", "comments": [
        { "path": "src/a.rs", "line": 2, "body": "rename it" },
        { "path": "src/a.rs", "line": 2, "side": "old", "body": "keep this" } ] }"#).unwrap();
    app.claude.path = Some(p.clone());
    app.load_claude();
    app.build();
    // A selection over both: one reply to them all, after what you want (amend, here).
    let rows: Vec<usize> = app.rows.iter().enumerate().filter(|(_, r)| r.thread.is_some()).map(|(i, _)| i).collect();
    app.goto(rows[0], 0);
    app.select = true;
    app.goto(rows[rows.len() - 1], 0);
    assert_eq!(app.selected_claude_threads().map(|k| k.len()), Some(2));
    assert_eq!(app.context_keys().map(|c| c.0), Some("Claude's comments, selected"));
    app.start_reply(false);
    let picker = app.picker.take().unwrap();
    assert!(picker.items.iter().any(|i| matches!(i.target, Target::Want("amend"))) && picker.items.iter().any(|i| matches!(i.target, Target::Want("commit"))));
    app.want("amend");
    assert!(matches!(app.action.take(), Some(Action::ClaudeReplyAll(ref ks, "amend")) if ks.len() == 2));
    _ = std::fs::remove_file(&p);
}

#[test]
fn enter_opens_code_or_what_is_under_the_cursor() {
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    let enter = |app: &mut App| app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    // On code, and on its file's header: the file in your editor.
    for kind in [Kind::Add, Kind::File] {
        let r = app.rows.iter().position(|r| r.line.kind == kind && r.file != INFO).unwrap();
        app.goto(r, 0);
        enter(&mut app);
        assert!(matches!(app.action.take(), Some(Action::Open(p)) if p.starts_with("src/a.rs:")), "{kind:?}");
    }
}

#[test]
fn mouse_sideways_keeps_the_cursor_on_screen() {
    use ratatui::crossterm::event::{MouseEvent, MouseEventKind};
    let body = format!("+{}\n", "y".repeat(400));
    let text = format!("diff --git a/w.rs b/w.rs\nnew file mode 100644\n--- /dev/null\n+++ b/w.rs\n@@ -0,0 +1,1 @@\n{body}");
    let mut app = App::with("base".into(), diff::parse(&text));
    let r = app.rows.iter().position(|r| r.line.text.contains("yyy")).unwrap();
    app.goto(r, 0);
    let s = screen(120, 20, |f| app.draw(f));
    let (y, line) = s.lines().enumerate().find(|(_, l)| l.contains("yyy")).unwrap();
    let x = line.chars().position(|c| c == 'y').unwrap() as u16;
    for _ in 0..3 {
        app.mouse(MouseEvent { kind: MouseEventKind::ScrollRight, column: x + 2, row: y as u16, modifiers: KeyModifiers::NONE });
        _ = screen(120, 20, |f| app.draw(f));
    }
    // Scrolled, the cursor (off the left) pinned to the first column shown.
    assert_eq!((app.left, app.head.col), (24, 24));
    // (The wheel with Alt: kitty keeps Shift and the mouse.)
    app.mouse(MouseEvent { kind: MouseEventKind::ScrollUp, column: x, row: y as u16, modifiers: KeyModifiers::ALT });
    _ = screen(120, 20, |f| app.draw(f));
    assert_eq!((app.left, app.head.col), (16, 24));
}

#[test]
fn tabs_and_wide_chars_scroll_by_their_columns() {
    // A tab is 4 columns, a wide character 2: the cursor at the end of a line of them is shown,
    // and a click lands on the character under it.
    for lead in ["\t".repeat(60), "中".repeat(100)] {
        let text = format!("diff --git a/w.rs b/w.rs\nnew file mode 100644\n--- /dev/null\n+++ b/w.rs\n@@ -0,0 +1,1 @@\n+{lead}xEND\n");
        let mut app = App::with("base".into(), diff::parse(&text));
        let r = app.rows.iter().position(|r| r.line.text.ends_with("END")).unwrap();
        let n = lead.chars().count();
        app.goto(r, n + 1);
        let s = screen(120, 20, |f| app.draw(f));
        let (y, line) = s.lines().enumerate().find(|(_, l)| l.contains("xE")).expect(&s);
        let x = line.find("xE").map(|b| line[..b].chars().count()).unwrap();
        assert_eq!(app.at_screen(x, y).map(|(r2, c, ..)| (r2, c)), Some((r, n)));
    }
}

#[test]
fn scrolled_sideways_tab_and_space_lines_stay_aligned() {
    // Scrolled by columns, every row alike: a tab-indented line and a space-indented one,
    // their text under each other.
    let text = "diff --git a/w.rs b/w.rs\nnew file mode 100644\n--- /dev/null\n+++ b/w.rs\n@@ -0,0 +1,2 @@\n+\t\tabcdefghij\n+        abcdefghij\n";
    let mut app = App::with("base".into(), diff::parse(text));
    let r = app.rows.iter().position(|r| r.line.text.starts_with(' ')).unwrap();
    (app.left, _) = (5, app.goto(r, 12));
    let s = screen(120, 30, |f| app.draw(f));
    let at: Vec<usize> = s.lines().filter_map(|l| l.find("ghij").map(|b| l[..b].chars().count())).collect();
    assert!(at.len() == 2 && at[0] == at[1] && app.left == 5, "{s}");
}

#[test]
fn a_thread_gone_leaves_the_cursor_on_its_line() {
    // Claude's comment posted (its copy dismissed): the cursor on the line it was on, not the
    // file's top.
    let mut app = fixture();
    let r = app.rows.iter().position(|r| r.thread == Some(0)).unwrap();
    app.goto(r, 0);
    let keep = app.cursor_key();
    app.threads.remove(0);
    app.rebuild(keep);
    assert_eq!(app.rows[app.head.row].line.new, Some(12), "{:?}", app.rows[app.head.row].line.text);
    // Outdated (no line of its own now, drawn where it was): the same.
    let mut app = fixture();
    (app.threads[0].outdated, app.threads[0].line) = (true, None);
    app.build();
    let r = app.rows.iter().position(|r| r.thread == Some(0)).unwrap();
    let above = app.rows[..r].iter().rev().find(|x| x.line.kind.is_code()).map(|x| x.line.new);
    app.goto(r, 0);
    let keep = app.cursor_key();
    app.threads.remove(0);
    app.rebuild(keep);
    assert_eq!(Some(app.rows[app.head.row].line.new), above);
}

#[test]
fn the_cursor_not_under_a_floating_comment() {
    let text = format!("diff --git a/w.rs b/w.rs\nnew file mode 100644\n--- /dev/null\n+++ b/w.rs\n@@ -0,0 +1,1 @@\n+{}\n", "z".repeat(300));
    let mut app = App::with("base".into(), diff::parse(&text));
    app.files_pref = Some(false);
    let mut th = fixture().threads.remove(0);
    (th.path, th.line, th.original_line) = ("w.rs".into(), Some(1), Some(1));
    app.threads = vec![th];
    (app.margin_pref, app.margin_float_pref) = (Some(true), Some(true));
    app.build();
    let r = app.rows.iter().position(|r| r.line.new == Some(1)).unwrap();
    app.goto(r, 0);
    _ = screen(110, 20, |f| app.draw(f));
    app.goto(r, app.left + 90);
    _ = screen(110, 20, |f| app.draw(f));
    assert!(app.margin_float && app.head.col - app.left < app.geom.ctw, "{} {} {}", app.left, app.head.col, app.geom.ctw);
}

#[test]
fn the_cursor_not_under_a_floating_comment_in_split_view() {
    let z = "z".repeat(300);
    let text = format!("diff --git a/w.rs b/w.rs\n--- a/w.rs\n+++ b/w.rs\n@@ -1,1 +1,1 @@\n-{z}\n+{z}y\n");
    let mut app = App::with("base".into(), diff::parse(&text));
    app.files_pref = Some(false);
    let mut th = fixture().threads.remove(0);
    (th.path, th.line, th.original_line) = ("w.rs".into(), Some(1), Some(1));
    app.threads = vec![th];
    (app.margin_pref, app.margin_float_pref, app.split) = (Some(true), Some(true), true);
    app.build();
    let r = app.rows.iter().position(|r| r.line.new == Some(1)).unwrap();
    app.goto(r, 0);
    _ = screen(160, 20, |f| app.draw(f));
    app.goto(r, app.left + 60);
    _ = screen(160, 20, |f| app.draw(f));
    let g = app.geom;
    assert!(app.margin_float && app.right);
    assert!(app.right_x() + app.head.col - app.left < g.relw + g.gw + g.ctw, "{} {} {:?}", app.left, app.head.col, (app.right_x(), g.ctw));
}

#[test]
fn the_pinned_title_doesnt_scroll_the_diff() {
    let mut app = fixture();
    app.left = 4;
    _ = screen(40, 20, |f| app.draw(f));
    app.goto(0, 30); // along the title, past the window
    _ = screen(40, 20, |f| app.draw(f));
    assert!(app.pin);
    assert_eq!(app.left, 4);
}

#[test]
fn a_comment_atop_the_second_file_keyed_by_its_own() {
    // An outdated comment of a.rs (no line to sit under: under its header), keyed by a.rs's
    // header, not the line of src/big.rs before it.
    let mut app = fixture();
    (app.threads[2].outdated, app.threads[2].line, app.threads[2].original_line, app.threads[2].hunk) = (true, None, None, vec![]);
    app.build();
    let r = app.rows.iter().position(|r| r.thread == Some(2)).unwrap();
    app.goto(r, 0);
    let keep = app.cursor_key();
    app.threads.remove(2);
    app.rebuild(keep);
    let row = &app.rows[app.head.row];
    assert_eq!((app.files[row.file].path.as_str(), row.line.kind), ("src/a.rs", Kind::File));
}

#[test]
fn a_pane_maximized_till_you_leave_it() {
    let ctrl_w = |app: &mut App, c: char| {
        app.key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
        app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        _ = screen(220, 40, |f| app.draw(f));
    };
    // The code: no files' list, no comments pane.
    let mut app = fixture();
    app.files_pref = Some(true);
    _ = screen(220, 40, |f| app.draw(f));
    assert!(app.files_w > 0 && app.margin_active);
    let r = app.rows.iter().position(|r| r.line.new == Some(12)).unwrap();
    app.goto(r, 0);
    ctrl_w(&mut app, 'o');
    assert!(app.files_w == 0 && !app.margin_active && app.zoom == Some(false));
    // Into the pane: as it was, and there.
    ctrl_w(&mut app, 'l');
    assert!(app.zoom.is_none() && app.margin_active && app.files_w > 0 && app.margin_side());
    // The pane: over all of the code; back to the code, as it was.
    ctrl_w(&mut app, 'o');
    assert!(app.zoom == Some(true) && app.margin_float && app.geom.ctw == 0);
    ctrl_w(&mut app, 'h');
    assert!(app.zoom.is_none() && !app.margin_side() && !app.margin_float);
    // Again: as it was.
    ctrl_w(&mut app, 'o');
    ctrl_w(&mut app, 'o');
    assert!(app.zoom.is_none() && app.files_w > 0);
    // The window's other keys act on it as set, not as maximized: Ctrl-w c hides the pane.
    ctrl_w(&mut app, 'o');
    ctrl_w(&mut app, 'c');
    assert!(app.zoom.is_none() && app.margin_pref == Some(false) && app.files_w > 0);
    ctrl_w(&mut app, 'c');
    // Onto a comment (]c): as it was, the comment shown.
    ctrl_w(&mut app, 'o');
    app.key(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
    _ = screen(220, 40, |f| app.draw(f));
    assert!(app.zoom.is_none() && app.margin_side(), "{}", app.msg);
    // The pane maximized, Ctrl-l (as far right as it goes): still maximized.
    ctrl_w(&mut app, 'o');
    ctrl_w(&mut app, 'l');
    assert_eq!(app.zoom, Some(true));
}

#[test]
fn claudes_comment_posted_in_its_view_leaves_the_cursor_near() {
    // The Claude view, its last comment in a file posted (`c`): its copy gone with that file's
    // rows; the cursor where it was, the next file's (not the top).
    let mut app = fixture();
    let t = app.threads.iter().position(|t| t.claude.is_some()).unwrap();
    let mut later = app.threads[t].clone();
    (later.path, later.line, later.original_line, later.claude) = ("src/a.rs".into(), Some(2), Some(2), Some("c2".into()));
    later.comments.iter_mut().for_each(|c| c.id += 1000); // (a comment of its own)
    app.threads.push(later);
    app.view = View::Claude;
    app.build();
    let r = app.rows.iter().position(|r| r.thread == Some(t)).unwrap();
    app.goto(r, 0);
    let keep = app.cursor_key();
    app.threads.remove(t);
    app.rebuild(keep);
    let row = &app.rows[app.head.row];
    assert!(app.head.row > 0 && app.files[row.file].path == "src/a.rs", "{:?}", row.line.text);
    // Only so for a comment gone: from a line of a file not in the view (into Claude's view, say),
    // its top (its summary).
    let mut app = fixture();
    let r = app.rows.iter().position(|r| app.files.get(r.file).is_some_and(|f| f.path == "src/a.rs") && r.line.new == Some(2)).unwrap();
    app.goto(r, 0);
    let keep = app.cursor_key();
    app.view = View::Claude;
    app.rebuild(keep);
    assert_eq!(app.head.row, 0);
}

#[test]
fn no_cursor_while_unfocused() {
    let mut app = fixture();
    assert_eq!(app.cursor_style(), app.ui.cursor);
    app.unfocused = true;
    assert_eq!(app.cursor_style(), Style::new());
}

#[test]
fn the_contents_with_no_files_yet() {
    // A draft not drafted (no files): the contents still shows, saying so.
    // No files: the list still shows, its heading saying so (none to come here: not a draft);
    // nothing to go to, it doesn't take the cursor.
    let mut app = App::with("base".into(), diff::parse(""));
    let s = screen(200, 20, |f| app.draw(f));
    assert!(s.contains("CONTENTS · no changes"), "{s}");
    app.focus_list(false);
    assert!(!app.files_focus && app.msg.contains("nothing to go to"));
    // Files: counted, not that.
    let mut app = fixture();
    app.files_pref = Some(true);
    let s = screen(200, 20, |f| app.draw(f));
    assert!(s.contains("CONTENTS · 4 files") && !s.contains("no changes"), "{s}");
}

#[test]
fn the_claude_view_without_comments_says_so() {
    // Its one comment went to GitHub (dismissed here): its summary, and why there's no code.
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    app.claude.summary = "Looks fine.".into();
    app.view = View::Claude;
    app.build();
    assert!(app.rows.iter().any(|r| r.line.text == "(no comments from Claude on the code: gn / gp, the whole diff)"));
    app.view = View::All;
    app.build();
    assert!(!app.rows.iter().any(|r| r.line.text.starts_with("(no comments from Claude")));
}

#[test]
fn a_folded_files_comments_stay_in_the_pane() {
    let mut app = fixture();
    _ = screen(220, 40, |f| app.draw(f)); // the pane shown
    app.collapsed.insert("src/big.rs".into());
    app.build();
    let s = screen(220, 40, |f| app.draw(f));
    // Folded: its comments by their headings, beside its header; not a count on the header.
    assert!(s.contains("─ amy · L20 · 2 comments ─") && s.contains("─ ◆ claude · L12 · 1 comment ─"), "{s}");
    assert!(!s.contains("src/big.rs  +1 −1  · "), "{s}");
    // Tab on one unfolds the file, there.
    let h = app.rows.iter().position(|r| r.line.text.contains("claude · L12")).unwrap();
    app.goto(h, 0);
    app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert!(!app.collapsed.contains("src/big.rs"));
    let t = app.rows[app.head.row].thread.unwrap();
    assert!(app.threads[t].claude.is_some(), "on Claude's comment");
}

#[test]
fn this_repos_issue_links_open_in_dv() {
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    app.pr = Some(gh::Pr { number: 87, url: "https://github.com/o/r/pull/87".into(), ..Default::default() });
    assert_eq!(app.issue_here("https://github.com/o/r/issues/82"), Some(82));
    assert_eq!(app.issue_here("https://github.com/o/r/issues/82#issuecomment-1"), Some(82));
    assert_eq!(app.issue_here("https://github.com/o/r/issues/87"), None); // this PR
    assert_eq!(app.issue_here("https://github.com/o/other/issues/82"), None); // another repo's: the browser
    assert_eq!(app.issue_here("https://github.com/o/r/pull/82"), Some(82)); // a PR: as #82 is
    assert_eq!(app.issue_here("https://github.com/o/r/pull/87/files"), None); // this PR
    // No PR (a replacement not published): the repo by its remote.
    (app.pr, app.github_url) = (None, Some("https://github.com/o/r".into()));
    assert_eq!(app.issue_here("https://github.com/o/r/issues/82"), Some(82));
}

#[test]
fn a_prs_replacement_being_drafted_shows_and_opens() {
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    app.pr = Some(gh::Pr { number: 87, ..Default::default() });
    // Claude at it (this process stands in for its run): on the status bar.
    app.replaced_by = Some(crate::replace::Record { id: 87, ..Default::default() });
    let claim = crate::claude::claim(&crate::claude::rep_key(87), "drafting", None).unwrap().unwrap();
    app.watch_elsewhere();
    assert_eq!(app.claude_doing().as_deref(), Some("◐ its replacement: claude drafting (Space a: open it)"));
    drop(claim);
    // Space a: open it, which hands the view over (and comes back after).
    app.claude.path = Some(std::env::temp_dir().join("dv-replaced-by-unused.json"));
    app.claude_menu();
    let p = app.picker.as_ref().unwrap();
    let i = p.items.iter().position(|it| matches!(it.target, Target::OpenReplacement(87))).expect("an item opening it");
    app.picker.as_mut().unwrap().sel = i;
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.hand_off, Some(HandOff::Replacement(87)));
}

#[test]
fn long_lines_wrap() {
    let long = "x".repeat(150);
    // (The code's, not the keys' hints over it: up to the line after it.)
    let xs = |s: &str| s.lines().take_while(|l| !l.contains("end")).map(|l| l.matches('x').count()).sum::<usize>();
    let text = format!("diff --git a/a.rs b/a.rs\nindex 1..2 100644\n--- a/a.rs\n+++ b/a.rs\n@@ -1,2 +1,2 @@\n-old\n+{long}\n end\n");
    let mut app = App::with("base".into(), diff::parse(&text));
    app.files_pref = Some(false);
    app.margin_pref = Some(false);
    _ = screen(80, 20, |f| app.draw(f));
    let row = app.rows.iter().position(|r| r.line.text == long).unwrap();
    let rows_before = app.drows.len();
    app.key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
    app.key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE));
    assert!(app.soft_wrap, "{}", app.msg);
    let s = screen(80, 20, |f| app.draw(f));
    // The long line takes more screen rows, its parts marked, its numbers on the first only.
    let w = app.code_w();
    assert_eq!(app.drows.len(), rows_before + long.len().div_ceil(w) - 1, "{s}");
    assert!(s.contains('↪'), "{s}");
    assert_eq!(xs(&s), long.len(), "{s}"); // all of it, nothing cut
    // The cursor at its end: on its last part.
    app.goto(row, long.len() - 1);
    assert_eq!(app.hd(), app.d_of[row] + long.len().div_ceil(w) - 1);
    // Split view, and beside the pane: all of it there too, each side wrapped to its half.
    app.toggle_split();
    let s = screen(80, 20, |f| app.draw(f));
    assert_eq!(xs(&s), long.len(), "{s}");
    app.margin_pref = Some(true);
    let s = screen(200, 20, |f| app.draw(f));
    assert_eq!(xs(&s), long.len(), "{s}");
    app.toggle_split();
    let s = screen(200, 20, |f| app.draw(f));
    assert_eq!(xs(&s), long.len(), "{s}");
    (app.margin_pref, app.split) = (Some(false), false);
    // Off again: one row, scrolled.
    app.toggle_wrap();
    _ = screen(80, 20, |f| app.draw(f));
    assert_eq!(app.drows.len(), rows_before);
}

#[test]
fn wrapped_lines_move_and_draw_whole() {
    // 40 long lines: j goes through each's parts, the wheel scrolls past them.
    let body: String = (0..40).map(|i| format!("+{}{}\n", i % 10, "y".repeat(299))).collect();
    let text = format!("diff --git a/a.rs b/a.rs\nindex 1..2 100644\n--- a/a.rs\n+++ b/a.rs\n@@ -0,0 +1,40 @@\n{body}");
    let mut app = App::with("base".into(), diff::parse(&text));
    (app.files_pref, app.margin_pref, app.soft_wrap) = (Some(false), Some(false), true);
    _ = screen(80, 20, |f| app.draw(f));
    let first = app.rows.iter().position(|r| r.line.kind == Kind::Add).unwrap();
    app.goto(first, 0);
    let parts = app.segs(Some(first), None);
    assert!(parts > 1);
    for k in 1..parts {
        app.key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
        assert_eq!((app.head.row, app.hd()), (first, app.d_of[first] + k), "j to part {k}");
    }
    app.key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
    assert_eq!(app.head.row, first + 1, "then the next line");
    let top = app.top;
    for _ in 0..6 {
        app.scroll_code(3);
        _ = screen(80, 20, |f| app.draw(f));
    }
    assert!(app.top >= top + 18, "the wheel scrolls: {} from {top}", app.top);
    // A wide character where a part would end: whole, on the next part.
    let w = app.code_w();
    let line = format!("{}界yz", "a".repeat(w - 1));
    let text = format!("diff --git a/b.rs b/b.rs\nindex 1..2 100644\n--- a/b.rs\n+++ b/b.rs\n@@ -1 +1 @@\n-old\n+{line}\n");
    let mut app = App::with("base".into(), diff::parse(&text));
    (app.files_pref, app.margin_pref, app.soft_wrap) = (Some(false), Some(false), true);
    _ = screen(80, 20, |f| app.draw(f));
    let s = screen(80, 20, |f| app.draw(f));
    assert!(s.contains("↪ 界"), "{s}"); // (its second column: a space, in this screen's text)
    // Split view: the short side's later rows have no ↪ of their own.
    app.toggle_split();
    let s = screen(80, 20, |f| app.draw(f));
    assert_eq!(s.matches('↪').count(), app.segs(None, app.rows.iter().position(|r| r.line.text == line)) - 1, "{s}");
}

#[test]
fn wrapped_moves_keep_their_column_and_get_past_pairs() {
    // Split: a short removed line beside a long added one. From the short side, j goes on past
    // the pair (its empty rows aren't stops); k comes back onto it.
    let long = "y".repeat(200);
    let text = format!("diff --git a/a.rs b/a.rs\nindex 1..2 100644\n--- a/a.rs\n+++ b/a.rs\n@@ -1,2 +1,2 @@\n-old\n+{long}\n end\n");
    let mut app = App::with("base".into(), diff::parse(&text));
    (app.files_pref, app.margin_pref, app.soft_wrap, app.split) = (Some(false), Some(false), true, true);
    _ = screen(80, 30, |f| app.draw(f));
    let del = app.rows.iter().position(|r| r.line.text == "old").unwrap();
    let end = app.rows.iter().position(|r| r.line.text == "end").unwrap();
    app.goto(del, 0);
    app.right = false;
    app.key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
    assert_eq!(app.head.row, end, "j past the pair");
    // Unified: from column 70 of a long line's second part, through a short line, onto a
    // long line's first part: still 70 into it.
    let body = format!("+{long}\n+short\n+{long}\n");
    let text = format!("diff --git a/b.rs b/b.rs\nindex 1..2 100644\n--- a/b.rs\n+++ b/b.rs\n@@ -0,0 +1,3 @@\n{body}");
    let mut app = App::with("base".into(), diff::parse(&text));
    (app.files_pref, app.margin_pref, app.soft_wrap) = (Some(false), Some(false), true);
    _ = screen(80, 30, |f| app.draw(f));
    let first = app.rows.iter().position(|r| r.line.text == long).unwrap();
    let w = app.code_w();
    app.goto(first, w + 70.min(w - 1));
    let parts = app.segs(Some(first), None);
    for _ in 0..parts - 1 {
        app.key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)); // to the short line
    }
    app.key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
    assert_eq!(app.rows[app.head.row].line.text, long, "onto the next long line");
    assert_eq!(app.head.col, 70.min(w - 1), "as far into its part");
}

#[test]
fn a_short_sides_empty_rows_stay_in_their_half() {
    // Split, wrapped: a 50-char removed line beside a 110-char added one. The removed side's
    // rows past its end are blank, its half's width: the divider stays put.
    let (a, b) = ("x".repeat(50), "y".repeat(110));
    let text = format!("diff --git a/a.rs b/a.rs\nindex 1..2 100644\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-{a}\n+{b}\n");
    let mut app = App::with("base".into(), diff::parse(&text));
    (app.files_pref, app.margin_pref, app.soft_wrap, app.split) = (Some(false), Some(false), true, true);
    _ = screen(80, 30, |f| app.draw(f));
    let s = screen(80, 30, |f| app.draw(f));
    let at: Vec<usize> = s.lines().filter(|l| l.contains('y')).map(|l| l.chars().position(|c| c == '│').unwrap()).collect();
    assert!(at.len() > 2 && at.iter().all(|&x| x == at[0]), "{s}");
    // gw: each word once, though its line takes several rows.
    app.alphabet = vec!['a', 'b', 'c'];
    app.show_labels();
    let starts: HashSet<_> = app.labels.iter().map(|l| l.0).collect();
    assert_eq!(starts.len(), app.labels.len());
}

#[test]
fn moves_keep_the_column_as_seen_scrolled_split_and_tall() {
    // Unwrapped, scrolled sideways: j keeps the column it's on, not that plus the scroll.
    let long = "z".repeat(200);
    let text = format!("diff --git a/a.rs b/a.rs\nindex 1..2 100644\n--- a/a.rs\n+++ b/a.rs\n@@ -0,0 +1,2 @@\n+{long}\n+{long}\n");
    let mut app = App::with("base".into(), diff::parse(&text));
    (app.files_pref, app.margin_pref) = (Some(false), Some(false));
    let first = app.rows.iter().position(|r| r.line.text == long).unwrap();
    _ = screen(80, 20, |f| app.draw(f));
    app.goto(first, 150);
    _ = screen(80, 20, |f| app.draw(f)); // (scrolled to it)
    app.goto(first, 120);
    _ = screen(80, 20, |f| app.draw(f));
    app.key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
    assert_eq!((app.head.row, app.head.col), (first + 1, 120));
    // Split, wrapped: Ctrl-h from a long line's later part goes to the short line beside it.
    let text = format!("diff --git a/b.rs b/b.rs\nindex 1..2 100644\n--- a/b.rs\n+++ b/b.rs\n@@ -1 +1 @@\n-old\n+{}\n end\n", "y".repeat(3000));
    let mut app = App::with("base".into(), diff::parse(&text));
    (app.files_pref, app.margin_pref, app.soft_wrap, app.split) = (Some(false), Some(false), true, true);
    _ = screen(80, 20, |f| app.draw(f));
    let (del, add) = (app.rows.iter().position(|r| r.line.text == "old").unwrap(), app.rows.iter().position(|r| r.line.kind == Kind::Add).unwrap());
    app.goto(add, 100);
    app.key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::CONTROL));
    assert_eq!((app.head.row, app.right), (del, false), "onto the short line");
    // The wheel over the tall pair, from its short side: it scrolls (not held back).
    app.goto(del, 0);
    for _ in 0..10 {
        app.scroll_code(3);
        _ = screen(80, 20, |f| app.draw(f));
    }
    assert!(app.top >= 20, "scrolled: top {}", app.top);
}

#[test]
fn the_kept_column_survives_scrolling_clicks_and_the_pane() {
    // Unwrapped: col 150 of a long line, j through a short one, j onto another long one: 150.
    let long = "z".repeat(200);
    let text = format!("diff --git a/a.rs b/a.rs\nindex 1..2 100644\n--- a/a.rs\n+++ b/a.rs\n@@ -0,0 +1,3 @@\n+{long}\n+short\n+{long}\n");
    let mut app = App::with("base".into(), diff::parse(&text));
    (app.files_pref, app.margin_pref) = (Some(false), Some(false));
    let first = app.rows.iter().position(|r| r.line.text == long).unwrap();
    _ = screen(80, 20, |f| app.draw(f));
    app.goto(first, 150);
    for _ in 0..2 {
        _ = screen(80, 20, |f| app.draw(f));
        app.key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
    }
    assert_eq!((app.head.row, app.head.col), (first + 2, 150));
    // Wrapped: a click in the gap at a part's end (a wide char didn't fit) stays on that part.
    let w = { app.soft_wrap = true; _ = screen(80, 20, |f| app.draw(f)); app.code_w() };
    let line = format!("{}界yz", "a".repeat(w - 1));
    let text = format!("diff --git a/b.rs b/b.rs\nindex 1..2 100644\n--- a/b.rs\n+++ b/b.rs\n@@ -1 +1 @@\n-old\n+{line}\n");
    let mut app = App::with("base".into(), diff::parse(&text));
    (app.files_pref, app.margin_pref, app.soft_wrap) = (Some(false), Some(false), true);
    _ = screen(80, 20, |f| app.draw(f));
    let r = app.rows.iter().position(|x| x.line.text == line).unwrap();
    let d = app.d_of[r];
    let g = app.geom;
    let y = d - app.top + app.pin as usize;
    let (row, col, ..) = app.at_screen(g.relw + g.gw + w, y).unwrap();
    assert_eq!((row, app.part_at(row, col)), (r, 0), "on the part clicked");
}

#[test]
fn the_wheel_from_the_pane_and_labels_under_the_pinned_header() {
    // A comment near the top, the cursor on it in the pane, then the wheel: it scrolls on.
    let body: String = (0..200).map(|i| format!("+word{i} other{i}\n")).collect();
    let text = format!("diff --git a/a.rs b/a.rs\nindex 1..2 100644\n--- a/a.rs\n+++ b/a.rs\n@@ -0,0 +1,200 @@\n{body}");
    let mut app = App::with("base".into(), diff::parse(&text));
    app.files_pref = Some(false);
    app.threads = vec![gh::Thread { path: "a.rs".into(), line: Some(3), comments: vec![gh::Comment { author: "amy".into(), body: "hm".into(), ..Default::default() }], ..Default::default() }];
    app.build();
    _ = screen(220, 20, |f| app.draw(f));
    let c = app.rows.iter().position(|r| r.thread.is_some()).unwrap();
    app.goto(c, 0);
    for _ in 0..6 {
        app.scroll_code(3);
        _ = screen(220, 20, |f| app.draw(f));
    }
    assert!(app.top >= 12, "scrolled: top {}", app.top);
    // The file's header pinned over the top: no labels on what it covers.
    assert!(app.sticky().is_some());
    app.show_labels();
    assert!(app.labels.iter().all(|l| app.d_at(l.0) > app.top), "none on the row under the header");
}

#[test]
fn the_cursor_lands_where_it_can_be_seen() {
    // The pinned title, cut, scrolls to the cursor along it (its chips kept).
    let mut app = fixture();
    app.pr_state = Some(("OPEN", "238636"));
    app.pr.as_mut().unwrap().title = "x".repeat(200) + "END";
    app.build();
    _ = screen(100, 12, |f| app.draw(f));
    app.goto(0, 0);
    for _ in 0..150 {
        app.key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE));
    }
    let s = screen(100, 12, |f| app.draw(f));
    let shown = app.span_of(0, 0).unwrap().chars;
    assert!(app.head.row == 0 && shown.contains(&app.head.col), "{:?} within {shown:?}", app.head);
    assert!(s.lines().next().unwrap().contains("OPEN"), "{s}");
    // A click on the file header pinned over the top goes to the header.
    let body: String = (0..60).map(|i| format!("+word{i} other{i}\n")).collect();
    let text = format!("diff --git a/a.rs b/a.rs\nindex 1..2 100644\n--- a/a.rs\n+++ b/a.rs\n@@ -0,0 +1,60 @@\n{body}");
    let mut app = App::with("base".into(), diff::parse(&text));
    (app.files_pref, app.margin_pref) = (Some(false), Some(false));
    _ = screen(80, 20, |f| app.draw(f));
    app.goto(40, 0);
    _ = screen(80, 20, |f| app.draw(f));
    let h = app.sticky().expect("pinned");
    let g = app.geom;
    assert_eq!(app.clicked_at(g.relw + g.gw + 2, 0).map(|a| a.0), Some(h));
    assert_ne!(app.at_screen(g.relw + g.gw + 2, 0).map(|a| a.0), Some(h)); // (a drag: the code under it)
}

#[test]
fn every_cursor_move_takes_its_side() {
    // Split: the half the cursor's drawn on is its line's (a removed line's left, an added
    // line's right), whatever moved it; the side you chose stays yours.
    let text = "diff --git a/a.rs b/a.rs\nindex 1..2 100644\n--- a/a.rs\n+++ b/a.rs\n@@ -1,5 +1,4 @@\n a\n-b1\n-b2\n+c1\n d\n e\n";
    let mut app = App::with("base".into(), diff::parse(text));
    (app.files_pref, app.margin_pref, app.split) = (Some(false), Some(false), true);
    _ = screen(80, 20, |f| app.draw(f));
    let at = |app: &App, t: &str| app.rows.iter().position(|r| r.line.text == t).unwrap();
    app.goto(at(&app, "b1"), 1);
    assert!(!app.head_right());
    app.key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE)); // on to b2, then c1 …
    app.goto(at(&app, "c1"), 0);
    assert!(app.head_right(), "an added line: the right");
    // Chosen right at `a`: j through the pair (b2 beside nothing: its hatch) and back: right.
    app.goto(at(&app, "a"), 0);
    app.right = true;
    for _ in 0..3 {
        app.key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
    }
    for _ in 0..2 {
        app.key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE));
    }
    assert_eq!(app.rows[app.head.row].line.text, "c1", "back on the right");
}

#[test]
fn a_hatch_click_and_the_way_out_of_the_pane() {
    // Split: a click on the hatch beside b2 lands on b2; your side stays the left.
    let text = "diff --git a/a.rs b/a.rs\nindex 1..2 100644\n--- a/a.rs\n+++ b/a.rs\n@@ -1,5 +1,4 @@\n a\n-b1\n-b2\n+c1\n d\n e\n";
    let mut app = App::with("base".into(), diff::parse(text));
    (app.files_pref, app.margin_pref, app.split) = (Some(false), Some(false), true);
    _ = screen(80, 20, |f| app.draw(f));
    let b2 = app.rows.iter().position(|r| r.line.text == "b2").unwrap();
    app.right = false;
    let y = app.d_of[b2] - app.top + app.pin as usize;
    let (row, _, _, side) = app.clicked_at(app.right_x() + 2, y).unwrap();
    assert_eq!((row, side), (b2, Some(false)), "the line it lands on, its side");
    // With the pane: from b2 (nothing to its right), Ctrl-l goes on into the pane.
    let mut app = App::with("base".into(), diff::parse(text));
    (app.files_pref, app.margin_pref, app.split) = (Some(false), Some(true), true);
    app.threads = vec![gh::Thread { path: "a.rs".into(), line: Some(4), comments: vec![gh::Comment { author: "amy".into(), body: "hm".into(), ..Default::default() }], ..Default::default() }];
    app.build();
    _ = screen(220, 20, |f| app.draw(f));
    app.goto(b2, 0);
    app.key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL));
    assert!(app.margin_side(), "into the pane: {}", app.msg);
}

#[test]
fn the_codes_edges_are_one_rule() {
    // Split: an added line with nothing to its left is at the left edge (Ctrl-h: the files'
    // list); a header across the width is at both (Ctrl-l: straight into the pane).
    let text = "diff --git a/a.rs b/a.rs\nindex 1..2 100644\n--- a/a.rs\n+++ b/a.rs\n@@ -1,3 +1,4 @@\n a\n-b1\n+c1\n+c2\n d\n";
    let mut app = App::with("base".into(), diff::parse(text));
    (app.files_pref, app.margin_pref, app.split) = (Some(true), Some(true), true);
    app.threads = vec![gh::Thread { path: "a.rs".into(), line: Some(4), comments: vec![gh::Comment { author: "amy".into(), body: "hm".into(), ..Default::default() }], ..Default::default() }];
    app.build();
    _ = screen(220, 20, |f| app.draw(f));
    let c2 = app.rows.iter().position(|r| r.line.text == "c2").unwrap();
    app.goto(c2, 0);
    app.key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::CONTROL));
    assert!(app.files_focus, "into the files' list: {}", app.msg);
    app.files_focus = false;
    let hunk = app.rows.iter().position(|r| r.line.kind == Kind::Hunk).unwrap();
    app.goto(hunk, 0);
    app.right = false;
    app.key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL));
    assert!(app.margin_side(), "into the pane at once: {}", app.msg);
}

#[test]
fn a_comment_on_no_line_is_about_no_other_files_line() {
    let text = "diff --git a/a.rs b/a.rs\nindex 1..2 100644\n--- a/a.rs\n+++ b/a.rs\n@@ -1,2 +1,2 @@\n a\n-b\n+c\ndiff --git a/z.rs b/z.rs\nindex 1..2 100644\n--- a/z.rs\n+++ b/z.rs\n@@ -1 +1 @@\n-y\n+z\n";
    let mut app = App::with("base".into(), diff::parse(text));
    (app.files_pref, app.margin_pref) = (Some(false), Some(true));
    app.threads = vec![gh::Thread { path: "z.rs".into(), line: Some(99), comments: vec![gh::Comment { author: "amy".into(), body: "hm".into(), ..Default::default() }], ..Default::default() }];
    app.build();
    _ = screen(220, 20, |f| app.draw(f));
    assert!(!app.margin_anchor.contains_key(&0), "about no line (not a.rs's last)");
}

#[test]
fn split_anchors_and_a_folded_files_comment() {
    // Split: a comment on a file gone from the diff is about no line (not the description
    // proposal's, which reads as a diff too).
    let mut app = fixture();
    app.claude.description = "Fixes the thing better.".into();
    app.split = true;
    app.threads.push(gh::Thread { path: "gone.rs".into(), line: Some(1), comments: vec![gh::Comment { author: "amy".into(), body: "hm".into(), ..Default::default() }], ..Default::default() });
    app.build();
    _ = screen(220, 40, |f| app.draw(f));
    let gone = app.threads.len() - 1;
    assert!(app.margin_anchor.get(&gone).is_none_or(|&r| app.rows[r].file != INFO), "{:?}", app.margin_anchor.get(&gone));
    // A folded file's comment: Ctrl-h goes to its own header, not the last file's code.
    let text = "diff --git a/a.rs b/a.rs\nindex 1..2 100644\n--- a/a.rs\n+++ b/a.rs\n@@ -1,2 +1,2 @@\n a\n-b\n+c\ndiff --git a/z.rs b/z.rs\nindex 1..2 100644\n--- a/z.rs\n+++ b/z.rs\n@@ -1 +1 @@\n-y\n+z\n";
    let mut app = App::with("base".into(), diff::parse(text));
    (app.files_pref, app.margin_pref) = (Some(false), Some(true));
    app.threads = vec![gh::Thread { path: "z.rs".into(), line: Some(1), comments: vec![gh::Comment { author: "amy".into(), body: "hm".into(), ..Default::default() }], ..Default::default() }];
    app.collapsed.insert("z.rs".into());
    app.build();
    _ = screen(220, 20, |f| app.draw(f));
    let c = app.rows.iter().position(|r| r.thread.is_some()).unwrap();
    app.goto(c, 0);
    app.key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::CONTROL));
    assert_eq!(app.rows[app.head.row].file, 1, "its own file's header: {:?}", app.rows[app.head.row].line.text);
}

#[test]
fn a_ranges_bar_passes_a_dot_on_a_wrapped_line() {
    // A range comment on lines 1–3, another on line 2 alone, line 2 wrapped: the range's bar
    // runs on beside line 2's later parts, past the other's dot on its first.
    let long = "q".repeat(200);
    let text = format!("diff --git a/a.rs b/a.rs\nindex 1..2 100644\n--- a/a.rs\n+++ b/a.rs\n@@ -0,0 +1,3 @@\n+one\n+{long}\n+three\n");
    let mut app = App::with("base".into(), diff::parse(&text));
    (app.files_pref, app.margin_pref, app.soft_wrap) = (Some(false), Some(false), true);
    let c = |b: &str| gh::Comment { author: "amy".into(), body: b.into(), ..Default::default() };
    app.threads = vec![
        gh::Thread { path: "a.rs".into(), start_line: Some(1), line: Some(3), comments: vec![c("range")], ..Default::default() },
        gh::Thread { path: "a.rs".into(), line: Some(2), comments: vec![c("dot")], ..Default::default() },
    ];
    app.build();
    _ = screen(80, 20, |f| app.draw(f));
    let r = app.rows.iter().position(|x| x.line.text == long).unwrap();
    assert_eq!(app.comment_mark(r, None).map(|m| m.0), Some('●'));
    assert_eq!(app.comment_bar(r, None), Some(('╎', app.ui.thread)));
    // Resolved, its bar is as stale there as on the line's first part.
    app.threads[0].resolved = true;
    app.build();
    assert_eq!(app.comment_bar(r, None), Some(('╎', app.ui.stale)));
}

#[test]
fn words_pair_past_comments_and_a_range_window_starts_at_its_first_line() {
    // A comment between two removed lines: the word emphasis pairs them with the added lines
    // as split view does (1st with 1st, 2nd with 2nd).
    let text = "diff --git a/a.rs b/a.rs\nindex 1..2 100644\n--- a/a.rs\n+++ b/a.rs\n@@ -1,2 +1,2 @@\n-let a = 1;\n-let b = 2;\n+let a = 10;\n+let b = 20;\n";
    let mut app = App::with("base".into(), diff::parse(text));
    app.threads = vec![gh::Thread { path: "a.rs".into(), line: Some(1), left: true, comments: vec![gh::Comment { author: "amy".into(), body: "hm".into(), ..Default::default() }], ..Default::default() }];
    app.build();
    let at = |t: &str| app.rows.iter().position(|r| r.line.text == t).unwrap();
    let emph = |r: usize| app.emph.get(&r).cloned().unwrap_or_default();
    let changed = |r: usize| emph(r).iter().map(|&(a, b)| app.rows[r].line.text.chars().skip(a).take(b - a).collect::<String>()).collect::<Vec<_>>();
    assert_eq!(changed(at("let b = 20;")), ["20"]);
    assert_eq!(changed(at("let b = 2;")), ["2"]);
    // A range comment (new lines 1–4) with removed lines inside it: its view's window starts at
    // its first line.
    let dels: String = (0..6).map(|i| format!("-gone{i}\n")).collect();
    let text = format!("diff --git a/b.rs b/b.rs\nindex 1..2 100644\n--- a/b.rs\n+++ b/b.rs\n@@ -1,9 +1,4 @@\n one\n{dels} two\n three\n four\n");
    let f = &diff::parse(&text)[0];
    let t = gh::Thread { path: "b.rs".into(), start_line: Some(1), line: Some(4), ..Default::default() };
    let w = view_windows(&[&t], f, |_| -(DEFAULT_CONTEXT as i64), false);
    assert_eq!(w[0].0, 0, "from `one`: {w:?}");
}

#[test]
fn windows_and_widening_go_by_one_rule() {
    // Two comments on one line: one window, its line once (one `+`, widened once).
    let w = diff::merge_windows(vec![(3, 9, 6), (3, 9, 6)], 20, false);
    assert_eq!(w, [(3, 9, vec![6])]);
    // An outdated comment's old range start doesn't stretch its window.
    let body: String = (1..=60).map(|i| format!(" l{i}\n")).collect();
    let text = format!("diff --git a/a.rs b/a.rs\nindex 1..2 100644\n--- a/a.rs\n+++ b/a.rs\n@@ -1,60 +1,60 @@\n{body}");
    let f = &diff::parse(&text)[0];
    let t = gh::Thread { path: "a.rs".into(), start_line: Some(2), line: None, outdated: true, original_line: Some(50), ..Default::default() };
    let w = view_windows(&[&t], f, |_| 0, false);
    assert!(w[0].0 > 40, "{w:?}");
    // `+` on a comment under the header (its line in hidden context): the hunk nearest grows to it.
    let body: String = (1..=40).map(|i| if i == 30 { "-old\n+new\n".to_owned() } else { format!(" l{i}\n") }).collect();
    let text = format!("diff --git a/b.rs b/b.rs\nindex 1..2 100644\n--- a/b.rs\n+++ b/b.rs\n@@ -1,40 +1,40 @@\n{body}");
    let mut app = App::with("base".into(), diff::parse(&text));
    app.threads = vec![gh::Thread { path: "b.rs".into(), line: Some(5), comments: vec![gh::Comment { author: "amy".into(), body: "hm".into(), ..Default::default() }], ..Default::default() }];
    app.build();
    let c = app.rows.iter().position(|r| r.thread.is_some()).unwrap();
    app.goto(c, 0);
    app.expand(10);
    assert!(app.rows.iter().any(|r| r.line.text == "l5"), "line 5 shown");
}

#[test]
fn a_views_windows_widen_per_comment() {
    // A new file, Claude's comments on lines 10, 12 and 50, the Claude view: `+` on the first
    // widens its window by one each side, and not the comment's at 50.
    let body: String = (1..=60).map(|i| format!("+n{i}\n")).collect();
    let text = format!("diff --git a/new.rs b/new.rs\nnew file mode 100644\n--- /dev/null\n+++ b/new.rs\n@@ -0,0 +1,60 @@\n{body}");
    let mut app = App::with("base".into(), diff::parse(&text));
    app.files_pref = Some(false);
    let t = |line, key: &str| gh::Thread { path: "new.rs".into(), line: Some(line), claude: Some(key.into()), comments: vec![gh::Comment { author: crate::claude::AUTHOR.into(), body: "x".into(), ..Default::default() }], ..Default::default() };
    app.threads = vec![t(10, "c1"), t(12, "c2"), t(50, "c3")];
    app.view = View::Claude;
    app.build();
    let shown = |app: &App| app.rows.iter().filter(|r| r.line.kind == Kind::Add).count();
    let before = shown(&app);
    let c = app.rows.iter().position(|r| r.thread == Some(0)).unwrap();
    app.goto(c, 0);
    app.expand(1);
    assert_eq!(shown(&app), before + 2, "one more line each side of the 10–12 window, only");
}

#[test]
fn a_range_from_the_old_side_reads_as_the_new_sides() {
    // GitHub's range from old line 20 to new line 6: on the new side, it starts at the first new
    // line from there (5), as Claude reads it.
    let text = "diff --git a/a.rs b/a.rs\nindex 1..2 100644\n--- a/a.rs\n+++ b/a.rs\n@@ -20,2 +5,3 @@\n-gone\n ctx\n+one\n+two\n";
    let f = &diff::parse(text)[0];
    assert_eq!(super::build::start_on_side(f, (20, true), false), Some(5));
    assert_eq!(super::build::start_on_side(f, (6, false), false), Some(6));
    let t = gh::Thread { path: "a.rs".into(), start_line: Some(20), start_left: Some(true), line: Some(6), ..Default::default() };
    assert_eq!(t.range_start(), Some((20, true)));
}

#[test]
fn commits_followed_through_amends_either_way() {
    let dir = diff::scratch_repo("follow-amends");
    let git = |args: &[&str]| diff::git_in(&dir, args).unwrap().trim().to_owned();
    let write = |f: &str, s: &str| std::fs::write(dir.join(f), s).unwrap();
    write("a.rs", "a\n");
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "base"]);
    let base = git(&["rev-parse", "HEAD"]);
    write("a.rs", "a\nfirst\n");
    git(&["commit", "-q", "-am", "first"]);
    write("b.rs", "b\n");
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "second"]);
    let (first, second) = (git(&["rev-parse", "HEAD~1"]), git(&["rev-parse", "HEAD"]));
    let mut app = fixture();
    app.root = dir.clone();
    let list = |app: &mut App| {
        let out = git(&["log", "--format=%H%x09%at%x09%s", &format!("{base}..HEAD")]);
        app.commits = out
            .lines()
            .map(|l| {
                let p: Vec<&str> = l.splitn(3, '\t').collect();
                files::Commit { sha: p[0].into(), short: p[0][..7].into(), at: p[1].into(), subject: p[2].into() }
            })
            .collect();
    };
    // "second" changed and reworded by an amend: nothing the same but its date, yet followed.
    write("b.rs", "b\nmore\n");
    let (hunks, _) = amend::plan(&dir, &base, true).unwrap();
    amend::amend(&dir, &base, &hunks, &[(second.as_str(), "second, reworded")], "dv-backup/f").unwrap();
    let now = git(&["rev-parse", "HEAD"]);
    list(&mut app);
    let cell = || std::cell::OnceCell::new();
    assert_eq!(app.now_as(&second, &cell()).as_deref(), Some(now.as_str()));
    // Reset back to the backup: the new one followed back to what it was.
    git(&["reset", "-q", "--hard", "dv-backup/f"]);
    list(&mut app);
    assert_eq!(app.now_as(&now, &cell()).as_deref(), Some(second.as_str()));
    // "first" dropped (its change undone): followed nowhere.
    write("a.rs", "a\n");
    let (hunks, _) = amend::plan(&dir, &base, true).unwrap();
    amend::amend(&dir, &base, &hunks, &[], "dv-backup/g").unwrap();
    list(&mut app);
    assert_eq!(app.now_as(&first, &cell()), None);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_draft_has_no_github_comments() {
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    app.show_replacement(crate::replace::Record { id: 3, title: "New".into(), ..Default::default() });
    app.build();
    let add = app.rows.iter().position(|r| r.line.kind == Kind::Add).unwrap();
    app.goto(add, 0);
    // The keys shown are those that do something: `a`, not `c` or `s`…
    let (_, keys) = app.context_keys().unwrap();
    assert!(keys.iter().any(|k| k.0 == "a") && !keys.iter().any(|k| matches!(k.0, "c" | "s")));
    // …and they say why.
    for c in ['c', 's'] {
        app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        assert!(app.action.is_none() && app.msg.starts_with("it's a draft"), "{c}: {}", app.msg);
    }
}

#[test]
fn popups_fit_a_tiny_terminal() {
    // (A box floored taller than the screen: cut to it, its scrollbar too, not drawn past it.)
    for (w, h) in [(10, 3), (7, 5), (1, 1)] {
        let mut app = fixture();
        app.key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE));
        screen(w, h, |f| app.draw(f));
    }
}

#[test]
fn a_wide_title_is_measured_in_columns() {
    let mut app = fixture();
    app.pr.as_mut().unwrap().title = "修复".repeat(10);
    app.build();
    screen(160, 20, |f| app.draw(f));
    // A click into the pinned title lands on the char drawn there (each 2 columns wide).
    let start = app.geom.relw + app.geom.gw;
    let text = app.rows[0].line.text.clone();
    let x = text.chars().take_while(|&c| c != '修').map(super::row::char_w).sum::<usize>() + 20;
    let (r, ch, ..) = app.at_screen(start + x, 0).unwrap();
    assert_eq!((r, text.chars().nth(ch)), (0, Some('修')), "{ch}");
}

#[test]
fn a_long_title_keeps_its_chips() {
    let mut app = fixture();
    app.pr_state = Some(("OPEN", "238636"));
    app.sync = "↑2 unpushed".into();
    app.pr.as_mut().unwrap().title = "x".repeat(200);
    app.build();
    let s = screen(100, 12, |f| app.draw(f));
    let title = s.lines().next().unwrap();
    assert!(title.contains("OPEN") && title.contains("unpushed"), "{title}");
}

#[test]
fn chips_too_wide_leave_the_title() {
    let mut app = fixture();
    app.pr_state = Some(("OPEN", "238636"));
    app.conflicts = true;
    app.sync = "⇅ diverged: force-push · ✎ uncommitted".into();
    app.build();
    let s = screen(60, 12, |f| app.draw(f));
    assert!(s.lines().next().unwrap().contains("Fix the thing"), "{s}");
}

#[test]
fn a_long_path_keeps_viewed() {
    let p = format!("src/{}.rs", "deep/".repeat(20));
    let text = format!("diff --git a/{p} b/{p}\nindex 1..2 100644\n--- a/{p}\n+++ b/{p}\n@@ -1 +1 @@\n-a\n+b\n");
    let mut app = App::with("base".into(), diff::parse(&text));
    (app.files_pref, app.margin_pref) = (Some(false), Some(false));
    app.apply_viewed(HashMap::from([(p.clone(), "VIEWED".to_owned())]));
    let s = screen(80, 10, |f| app.draw(f));
    assert!(s.lines().any(|l| l.contains("deep/") && l.contains("Viewed")), "{s}");
}

#[test]
fn the_wheel_scrolls_the_title_only_where_its_cut() {
    use ratatui::crossterm::event::{MouseEvent, MouseEventKind};
    let mut app = fixture();
    _ = screen(220, 20, |f| app.draw(f));
    app.goto(3, 0);
    let wheel = |app: &mut App, x: usize| {
        app.mouse(MouseEvent { kind: MouseEventKind::ScrollRight, column: x as u16, row: 0, modifiers: KeyModifiers::NONE });
        screen(220, 20, |f| app.draw(f))
    };
    // Over the pane's tabs: not the title.
    let g = app.geom;
    wheel(&mut app, g.relw + g.gw + g.ctw + 5);
    assert_eq!(app.head.row, 3);
    // On a title that fits: nothing to scroll to, the cursor left where it is.
    let s = wheel(&mut app, 20);
    assert!(s.lines().next().unwrap().contains("PR #7 · Fix the thing") && app.hscroll(0) == 0, "{s}");
    assert_eq!(app.head.row, 3);
}

#[test]
fn the_wheel_over_a_short_line_leaves_the_scroll() {
    use ratatui::crossterm::event::{MouseEvent, MouseEventKind};
    let long = "x".repeat(300);
    let text = format!("diff --git a/a.rs b/a.rs\nindex 1..2 100644\n--- a/a.rs\n+++ b/a.rs\n@@ -1,3 +1,3 @@\n short\n-{long}\n+{long}y\n short2\n");
    let mut app = App::with("base".into(), diff::parse(&text));
    (app.files_pref, app.margin_pref) = (Some(false), Some(false));
    _ = screen(80, 12, |f| app.draw(f));
    let long_r = (0..app.rows.len()).find(|&r| app.len(r) == 300).unwrap();
    let y = |app: &App, r: usize| (app.d_of[r] - app.top + app.pin as usize) as u16;
    let wheel = |app: &mut App, kind, r: usize| {
        let row = y(app, r);
        app.mouse(MouseEvent { kind, column: 40, row, modifiers: KeyModifiers::NONE });
        _ = screen(80, 12, |f| app.draw(f));
    };
    for _ in 0..15 {
        wheel(&mut app, MouseEventKind::ScrollRight, long_r);
    }
    let left = app.left;
    // Over the short line above, scrolled past its end: nothing either way (not back to it).
    wheel(&mut app, MouseEventKind::ScrollLeft, long_r - 1);
    assert_eq!(app.left, left);
    // Scrolled to the long line's end (its last char showing): back left over it.
    for _ in 0..60 {
        wheel(&mut app, MouseEventKind::ScrollRight, long_r);
    }
    let left = app.left;
    wheel(&mut app, MouseEventKind::ScrollLeft, long_r);
    assert!(app.left < left, "{} → {}", left, app.left);
}

#[test]
fn the_wheel_at_a_blocks_end_leaves_the_cursor() {
    use ratatui::crossterm::event::{MouseEvent, MouseEventKind};
    let mut app = fixture();
    (app.summary_draft, app.claude.summary) = (String::new(), String::new());
    app.threads[1].comments[0].body = format!("```\n{}\n```", "x".repeat(300));
    app.build();
    let s = screen(220, 45, |f| app.draw(f));
    let (y, line) = s.lines().enumerate().find(|(_, l)| l.contains("xxx")).unwrap();
    let x = line.chars().position(|c| c == 'x').unwrap() as u16;
    let wheel = |app: &mut App, x: u16| {
        app.mouse(MouseEvent { kind: MouseEventKind::ScrollDown, column: x, row: y as u16, modifiers: KeyModifiers::SHIFT });
        _ = screen(220, 45, |f| app.draw(f));
    };
    for _ in 0..80 {
        wheel(&mut app, x + 5);
    }
    // At its end: another notch elsewhere on it scrolls nothing, and moves nothing.
    let (left, col) = (app.code_left, app.head.col);
    wheel(&mut app, x + 20);
    assert_eq!((app.code_left, app.head.col), (left, col));
}

#[test]
fn as_far_as_a_line_scrolls_is_its_last_char() {
    // (A wide char or a tab last: its start, where the cursor on it pulls the scroll back to.)
    for (end, at) in [("界", 300), ("\t", 300), ("y", 300)] {
        let text = format!("diff --git a/a.rs b/a.rs\nindex 1..2 100644\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-a\n+{}{end}\n", "x".repeat(300));
        let mut app = App::with("base".into(), diff::parse(&text));
        _ = screen(80, 12, |f| app.draw(f));
        let r = (0..app.rows.len()).find(|&r| app.len(r) == 301).unwrap();
        assert_eq!(app.hmost(r, app.row_w(r)), at, "{end:?}");
    }
}

/// An issue as `App::open_issue` shows it, from GitHub's answer `v` (as `gh::issue_json`'s).
fn issue_app(v: serde_json::Value) -> App {
    let mut app = App::with(String::new(), vec![]);
    app.issue = Some(IssueRec { number: 7, ..IssueRec::default() });
    let me = gh::parse_threads(&serde_json::json!({ "viewer": { "login": "me" }, "repository": { "pullRequest": { "viewerDidAuthor": false } } }));
    app.apply_github((Ok(me), Ok(info::parse_info(&v))), true);
    app.build();
    app
}

fn issue_fixture() -> serde_json::Value {
    // (Just now: what's on screen says "just now", whenever the test runs.)
    let now = crate::claude::now();
    serde_json::json!({
        "__typename": "Issue", "number": 7, "title": "LocalChain panics", "url": "https://github.com/o/r/issues/7", "author": { "login": "amy" },
        "createdAt": now, "state": "OPEN", "body": "It **panics** on reorg.", "labels": [{ "name": "bug", "color": "d73a4a" }], "assignees": [],
        "dvActivity": { "items": [
            { "__typename": "IssueComment", "id": "c1", "url": "https://github.com/o/r/issues/7#issuecomment-1", "createdAt": now, "author": { "login": "bo" }, "body": "Same here." },
        ] },
    })
}

#[test]
fn an_issue_in_the_pr_view() {
    let mut app = issue_app(issue_fixture());
    let dir = std::env::temp_dir().join(format!("dv-issue-view-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("issue-7.json");
    std::fs::write(&p, r#"{ "issue": 7, "kind": "bug", "priority": "high", "summary": "Still on master: `src/a.rs:12` unwraps.",
        "next": "review #9", "title": "LocalChain panics on reorg past genesis", "labels": ["bug", "chain"] }"#).unwrap();
    app.claude.path = Some(p);
    app.load_claude();
    app.build();
    // Its title, Claude's triage first (with its clearer title as a proposal to approve), its
    // facts and conversation; no files, no checkout's state, one tab.
    snapshot("issue", &screen(120, 30, |f| app.draw(f)));
    assert!(app.has_proposal() && app.proposed_title().as_deref() == Some("LocalChain panics on reorg past genesis"));
    assert_eq!(app.pane_tabs().len(), 1);
    // Space a: its triage, questions, a PR fixing it; nothing that changes code.
    app.key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
    let items: Vec<String> = app.picker.take().unwrap().items.iter().map(|i| i.label.clone()).collect();
    let has = |t: &str| items.iter().any(|i| i.starts_with(t));
    assert!(has("Triage issue #7 again") && has("Ask a question about issue #7") && has("Draft a PR fixing #7") && has("Approve Claude's description"), "{items:#?}");
    assert!(!has("Make a") && !has("Review this") && !has("Draft a replacement"), "{items:#?}");
    // Space g, S: no code.
    app.key(KeyEvent::new(KeyCode::Char('S'), KeyModifiers::NONE));
    assert_eq!(app.msg, git::NO_CODE);
    app.key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
    assert_eq!(app.msg, git::NO_CODE);
    // What's GitHub's works on an issue as on a PR: Space l (its labels), Space o (all of its
    // conversation); `c` has no review to queue into.
    app.key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE));
    assert!(matches!(app.job, Some(Job::Labels)), "{}", app.msg);
    app.job = None;
    let all = app.conversation_all;
    app.toggle_conversation();
    assert_ne!(app.conversation_all, all, "{}", app.msg);
    app.quote_into_summary();
    assert!(app.msg.starts_with("an issue has no review"), "{}", app.msg);
    _ = std::fs::remove_dir_all(dir);
}

/// Live: an issue opened in the PR view, as `App::open_issue` loads it.
/// `DV_LIVE_REPO=<a checkout> DV_LIVE_ISSUE=<n> cargo test live_open_issue -- --ignored --nocapture`
#[test]
#[ignore]
fn live_open_issue() {
    std::env::set_current_dir(std::env::var("DV_LIVE_REPO").expect("DV_LIVE_REPO")).unwrap();
    let n: u64 = std::env::var("DV_LIVE_ISSUE").expect("DV_LIVE_ISSUE").parse().unwrap();
    let mut app = App::open_issue(n).unwrap();
    app.load_threads(); // (afresh, not what was cached)
    app.build();
    println!("{}", screen(160, 40, |f| app.draw(f)));
}

#[test]
fn a_runs_claim_goes_with_it() {
    // Its claim on the PR, held while it runs (by its own process), let go as it ends: before
    // anything after it (answering what came in meanwhile) claims anew.
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    let n = 990_000_000 + u64::from(std::process::id() % 1000);
    app.pr = Some(gh::Pr { number: n, ..Default::default() });
    let mut child = std::process::Command::new("sleep").arg("0.2").spawn().unwrap();
    let key = crate::claude::pr_key(n);
    let mut claim = crate::claude::claim(&key, "answering", None).unwrap().unwrap();
    claim.set_pid(child.id());
    assert_eq!(crate::claude::at_work(&key, None, None).as_deref(), Some("answering"));
    assert!(app.claude_blocked(Some(agent::Ask::Answer)).is_some_and(|w| w.contains("already")));
    _ = child.wait();
    let child = std::process::Command::new("true").spawn().unwrap();
    app.claude.job = Some(agent::ClaudeJob { _claim: Some(claim), ..agent::ClaudeJob::new(child, std::env::temp_dir().join("dv-test-claim.log"), "answering", false) });
    std::thread::sleep(std::time::Duration::from_millis(100));
    app.poll_claude();
    assert!(app.claude.job.is_none() && crate::claude::at_work(&key, None, None).is_none());
}

#[test]
fn the_wheel_over_code_beside_a_floating_pane_scrolls_it() {
    // The pane floating covers the code only where a comment's box is: elsewhere the wheel
    // there is over code, and scrolls it (no comments beyond, even).
    use ratatui::crossterm::event::{MouseEvent, MouseEventKind};
    let mut app = fixture();
    app.margin_pref = Some(true);
    let draw = |app: &mut App| screen(100, 14, |f| app.draw(f));
    draw(&mut app);
    assert!(app.margin_float);
    while app.comments_beyond(true) {
        app.scroll_code(1);
        draw(&mut app);
    }
    let y = (1..12).find(|&y| app.note_at(app.top + y - 1).is_none()).unwrap();
    let x = (app.geom.relw + app.geom.gw + app.geom.ctw + 5) as u16;
    let top = app.top;
    app.mouse(MouseEvent { kind: MouseEventKind::ScrollDown, column: x, row: y as u16, modifiers: KeyModifiers::NONE });
    assert!(app.top > top, "{top} -> {}", app.top);
}

#[test]
fn an_issues_key_hints_are_an_issues() {
    // Its text and comments: C quotes them (no review for c); its facts box: the issue's.
    let mut app = issue_app(issue_fixture());
    _ = screen(120, 30, |f| app.draw(f));
    let at = |app: &mut App, t: &str| {
        let r = app.rows.iter().position(|r| r.line.text.contains(t)).unwrap();
        app.goto(r, 0);
        app.context_keys().map(|k| (k.0, k.1.iter().map(|e| e.0).collect::<Vec<_>>()))
    };
    assert_eq!(at(&mut app, "Same here."), Some(("issue text", vec!["C", "Space =", "Space c"])));
    let facts = at(&mut app, "labels");
    assert!(facts.as_ref().is_some_and(|k| k.0.starts_with("issue")), "{facts:?}");
    // A selection there: what the keys do there (a selection changes them on code only).
    at(&mut app, "Same here.");
    app.key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE));
    let keys: Vec<&str> = app.context_keys().unwrap().1.iter().map(|e| e.0).collect();
    assert_eq!(keys, ["C", "Space =", "Space c"]);
}

#[test]
fn a_selection_onto_a_hunks_rule_hints_the_selection() {
    // `v` then `k` onto the hunk's rule: the keys still act on the lines selected.
    let mut app = fixture();
    let first = (1..app.rows.len()).find(|&r| app.rows[r].line.kind.is_code() && app.rows[r - 1].line.kind == Kind::Hunk).unwrap();
    app.goto(first, 0);
    app.key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE));
    assert_eq!(app.rows[app.head.row].line.kind, Kind::Hunk);
    assert_eq!(app.context_keys().map(|k| k.0), Some("selection"));
}

#[test]
fn a_selections_keys_are_those_that_act() {
    let keys = |app: &App| app.context_keys().map(|(t, k)| (t, k.iter().map(|e| e.0).collect::<Vec<_>>()));
    // `v` on a file's header: no code selected, so only copying (the comment keys would refuse).
    let mut app = fixture();
    let header = app.rows.iter().position(|r| r.file != INFO && r.line.kind == Kind::File).unwrap();
    app.goto(header, 0);
    app.key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE));
    assert_eq!(keys(&app), Some(("selection", vec!["y", "Space c"])));
    // Across two files: the same.
    let mut app = fixture();
    let first = app.rows.iter().position(|r| r.file != INFO && r.line.kind.is_code()).unwrap();
    let other = app.rows.iter().position(|r| r.file != INFO && r.file != app.rows[first].file && r.line.kind.is_code()).unwrap();
    app.goto(first, 0);
    app.key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE));
    app.goto(other, 0);
    assert_eq!(keys(&app), Some(("selection", vec!["y", "Space c"])));
    // On a removed line: no suggestion (it replaces new lines only).
    let mut app = fixture();
    let del = app.rows.iter().position(|r| r.line.kind == Kind::Del).unwrap();
    app.goto(del, 0);
    assert!(keys(&app).is_some_and(|(t, k)| t == "code" && !k.contains(&"s") && k.contains(&"c")));
    // A review pending: `C` posts nothing now, on a thread as on code (c adds to the review).
    let mut app = fixture();
    app.review = Some("R".into());
    let thread = app.rows.iter().position(|r| r.thread.is_some_and(|t| app.threads[t].claude.is_none())).unwrap();
    app.goto(thread, 0);
    assert!(keys(&app).is_some_and(|(_, k)| !k.contains(&"C")), "{:?}", keys(&app));
    // Claude's comment on a file GitHub hasn't got (untracked): nothing to copy or post.
    let mut app = fixture();
    let t = app.threads.iter().position(|t| t.by_claude() && !t.outdated && t.conversation.is_empty()).unwrap();
    let path = app.threads[t].path.clone();
    app.files.iter_mut().filter(|f| f.path == path).for_each(|f| f.meta = "untracked".into());
    app.build();
    let r = app.rows.iter().position(|r| r.thread == Some(t)).unwrap();
    app.goto(r, 0);
    assert!(keys(&app).is_some_and(|(_, k)| !k.contains(&"c") && !k.contains(&"C") && k.contains(&"r")), "{:?}", keys(&app));
}

fn press(app: &mut App, s: &str) { s.chars().for_each(|c| app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))); }
fn ctrl(app: &mut App, c: char) { app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)); }

/// a.rs with comments under lines 1 and 3, shown in the margin pane.
fn margin_app() -> App {
    let sample = "diff --git a/a.rs b/a.rs\nindex 1..2 100644\n--- a/a.rs\n+++ b/a.rs\n@@ -1,4 +1,4 @@\n one\n two\n three\n-four\n+four!\n";
    let mut app = App::with("base".into(), diff::parse(sample));
    let c = |id, body: &str| gh::Comment { id, node_id: String::new(), can_edit: false, can_delete: false, author: "amy".into(), body: body.into(), created: String::new(), pending: false, reactions: vec![], hidden: String::new() };
    let t = |line, comments| gh::Thread {
        id: String::new(), path: "a.rs".into(), left: false, line: Some(line), start_line: None, start_left: None, original_line: Some(line),
        resolved: false, outdated: false, hunk: vec![], comments, claude: None, severity: String::new(), about: None, conversation: String::new(),
        talk_resolved: false, want: String::new(),
    };
    app.threads = vec![t(1, vec![c(1, "first")]), t(3, vec![c(2, "second")])];
    app.margin_active = true;
    app.build();
    app
}

#[test]
fn x_skips_pane_comments() {
    let mut app = margin_app();
    app.goto(2, 0);
    press(&mut app, "xx");
    assert!(app.rows[app.head.row].thread.is_none(), "x x landed on a comment row {}", app.head.row);
}

#[test]
fn copy_across_pane_comments_keeps_to_the_pane() {
    let mut app = margin_app();
    let at = |app: &App, s| app.rows.iter().position(|r| r.line.text.contains(s)).unwrap();
    app.goto(at(&app, "first"), 0);
    app.select = true;
    app.goto(at(&app, "second"), usize::MAX);
    let t = app.selection_text(false);
    assert!(!t.contains("two"), "code between the comments copied: {t:?}");
}

#[test]
fn ctrl_keys_end_prefix_and_count() {
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    app.goto(4, 0);
    ctrl(&mut app, 'w');
    ctrl(&mut app, 'l');
    assert_eq!(app.pending, None, "Ctrl-w still pending");
    press(&mut app, "3");
    ctrl(&mut app, 'e');
    assert_eq!(app.count, 0, "count kept after Ctrl-e");
}

#[test]
fn search_back_from_selection_start() {
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    app.goto(3, 0); // "    old();" through "    new_call(x);"
    app.select = true;
    app.goto(4, 15);
    app.search = "(".into();
    press(&mut app, "N"); // back from the selection's start: main's, not new_call's
    assert_eq!(app.head, Pos { row: 2, col: 7 });
    assert_eq!(app.anchor, Pos { row: 3, col: 0 }, "select mode: the selection extends");
}

#[test]
fn click_in_code_takes_focus_from_list() {
    use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    app.build();
    app.height = 20;
    app.files_w = 20;
    app.files_focus = true;
    app.geom = mouse::Geom { relw: 0, gw: 10, ghw: 6, half: 40, ctw: 60, picker: None };
    app.mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: 34, row: 2, modifiers: KeyModifiers::NONE });
    assert!(!app.files_focus, "list still focused after a click in the code");
}

#[test]
fn x_grows_from_one_char_and_empty_lines() {
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    let r = app.rows.iter().position(|r| r.line.text == "}").unwrap();
    app.goto(r - 1, 0);
    press(&mut app, "xx"); // a long line, then "}"
    assert_eq!((app.anchor, app.head.row), (Pos { row: r - 1, col: 0 }, r));
    app.goto(r, 0);
    press(&mut app, "x");
    assert_eq!(app.head.row, r, "one x: its own line");
    press(&mut app, "x");
    assert!(app.head.row > r, "x x stuck on a one-char line");
}

#[test]
fn search_and_ge_keep_to_rows_drawn_on_your_side() {
    let mut app = margin_app();
    app.threads[1].line = Some(4);
    app.threads[1].original_line = Some(4);
    app.build();
    app.goto(2, 0);
    press(&mut app, "ge");
    assert!(!app.margin_side(), "ge from the code went into the pane");
    app.margin_active = false;
    app.build();
    app.goto(2, 0);
    press(&mut app, "ge");
    assert!(app.side_of(app.head.row).is_some(), "ge onto a hidden comment");
    app.goto(0, 0);
    app.search = "second".into();
    press(&mut app, "n");
    assert!(app.side_of(app.head.row).is_some(), "search landed on a hidden comment");
}

#[test]
fn arrow_is_a_move_not_a_prefix_char() {
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    let r = app.rows.iter().position(|r| r.line.text == "    new_call(x);").unwrap();
    app.goto(r, 0);
    press(&mut app, "f");
    app.key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
    assert_eq!((app.pending, app.head.col), (None, 1), "f then Right ran fl");
    press(&mut app, "3");
    app.key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT));
    assert_eq!(app.count, 0, "count kept after Alt-x");
}

#[test]
fn star_on_a_hunk_header_searches_its_text() {
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    let r = app.rows.iter().position(|r| r.line.kind == Kind::Hunk).unwrap();
    app.goto(r, 0);
    press(&mut app, "w*");
    assert!(!app.search.is_empty());
}

#[test]
fn a_rebuild_keeps_the_selection() {
    // A resize (re-wrapped rows) keeps x's lines, in the pane as in the code.
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    let _ = screen(120, 30, |f| app.draw(f));
    app.goto(4, 0);
    press(&mut app, "xx");
    let before = (app.anchor, app.head);
    let _ = screen(90, 30, |f| app.draw(f));
    assert_eq!((app.anchor, app.head), before, "resize lost the selection");
    // The comment seen (its [new] gone): rebuilt under a selection, it stays.
    let mut app = margin_app();
    for t in &mut app.threads {
        for c in &mut t.comments {
            c.node_id = format!("n{}", c.id);
        }
    }
    app.build();
    app.seen.path = Some(std::env::temp_dir().join(format!("dv-seen-{}.json", std::process::id())));
    let r = app.rows.iter().position(|r| r.line.text.contains("first")).unwrap();
    app.goto(r, 0);
    app.watch_seen();
    app.seen.since = Some(std::time::Instant::now() - std::time::Duration::from_secs(10));
    press(&mut app, "x");
    let before = (app.anchor, app.head);
    app.watch_seen();
    assert_eq!((app.anchor, app.head), before, "seen rebuild dropped the selection");
    let _ = std::fs::remove_file(app.seen.path.as_ref().unwrap());
}

#[test]
fn x_over_two_claude_threads_replies_to_both() {
    let mut app = margin_app();
    app.threads[0].claude = Some("k1".into());
    app.threads[1].claude = Some("k2".into());
    app.build();
    let a = app.rows.iter().position(|r| r.line.text.contains("first")).unwrap();
    let b = app.rows.iter().position(|r| r.line.text.contains("second")).unwrap();
    app.goto(a, 0);
    while app.head.row < b {
        let h = app.head;
        press(&mut app, "x");
        if app.head == h {
            break;
        }
    }
    assert!(app.selected_claude_threads().is_some(), "x over two of Claude's threads: not both");
}

#[test]
fn a_long_count_is_capped_and_a_click_ends_labels() {
    use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    press(&mut app, "99999999999999999999w");
    let _ = screen(120, 30, |f| app.draw(f));
    press(&mut app, "gw");
    assert!(!app.labels.is_empty());
    ctrl(&mut app, 'd');
    assert!(app.labels.is_empty(), "Ctrl-d taken as a label's letter");
    press(&mut app, "gw");
    app.mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: 30, row: 6, modifiers: KeyModifiers::NONE });
    assert!(app.labels.is_empty(), "labels still up after a click");
}

#[test]
fn a_rebuild_finds_the_anchor_as_it_was_before_threads_change() {
    // A new thread first (GitHub's poll, Claude's review): the anchor stays on its comment.
    let mut app = margin_app();
    let at = |app: &App, s| app.rows.iter().position(|r| r.line.text.contains(s)).unwrap();
    app.goto(at(&app, "first"), 0);
    app.select = true;
    app.goto(at(&app, "second"), 0);
    let keep = app.cursor_key();
    let mut new = app.threads[1].clone();
    new.line = Some(4);
    new.comments[0].id = 9;
    app.threads.insert(0, new);
    app.rebuild(keep);
    assert_eq!(app.rows[app.anchor.row].thread.and_then(|t| app.root_id(t)), Some(1), "anchor moved to another thread");
    // A thread gone, the file folded (no code above its comments): no panic.
    let mut app = margin_app();
    app.collapsed.insert("a.rs".into());
    app.build();
    app.goto(2, 0);
    app.select = true;
    app.goto(1, 0);
    let keep = app.cursor_key();
    app.threads.truncate(1);
    app.rebuild(keep);
}

#[test]
fn a_rebuild_ends_jump_labels() {
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    let _ = screen(120, 30, |f| app.draw(f));
    press(&mut app, "gw");
    assert!(!app.labels.is_empty());
    let keep = app.cursor_key();
    app.rebuild(keep);
    assert!(app.labels.is_empty(), "labels kept over rows that moved");
}

#[test]
fn nothing_changes_the_rows_under_half_typed_jump_labels() {
    let mut app = margin_app();
    for t in &mut app.threads {
        for c in &mut t.comments {
            c.node_id = format!("n{}", c.id);
        }
    }
    app.build();
    app.seen.path = Some(std::env::temp_dir().join(format!("dv-seen-labels-{}.json", std::process::id())));
    let r = app.rows.iter().position(|r| r.line.text.contains("first")).unwrap();
    app.goto(r, 0);
    let _ = screen(120, 30, |f| app.draw(f));
    app.watch();
    press(&mut app, "gw");
    app.seen.since = app.seen.since.map(|t| t - std::time::Duration::from_secs(5));
    app.watch(); // the comment seen by now: not till the labels are done
    assert!(!app.labels.is_empty(), "labels ended by a background rebuild");
    let _ = std::fs::remove_file(app.seen.path.as_ref().unwrap());
}

#[test]
fn a_file_from_the_list_is_a_jump_back_from() {
    let s = "diff --git a/a.rs b/a.rs\nindex 1..2 100644\n--- a/a.rs\n+++ b/a.rs\n@@ -1,4 +1,4 @@\n one\n two\n three\n-four\n+four!\ndiff --git a/b.rs b/b.rs\nindex 1..2 100644\n--- a/b.rs\n+++ b/b.rs\n@@ -1,2 +1,2 @@\n alpha\n-beta\n+gamma\n";
    let mut app = App::with("base".into(), diff::parse(s));
    app.collapsed.insert("a.rs".into());
    app.build();
    let at = |app: &App, t: &str| app.rows.iter().position(|r| r.line.text == t).unwrap();
    app.goto(at(&app, "alpha"), 0);
    app.select = true;
    app.goto(at(&app, "gamma"), 0);
    app.goto_spot(super::files::Spot::File(0)); // a click on a.rs in the files' list: a jump
    assert_eq!(app.rows[app.head.row].line.text, "a.rs  +1 −1");
    ctrl(&mut app, 'o');
    assert_eq!(app.rows[app.head.row].line.text, "gamma", "Ctrl-o not back where you were");
}

#[test]
fn the_files_picker_goes_to_its_file_after_a_resize() {
    let mut app = fixture();
    _ = screen(200, 40, |f| app.draw(f));
    press(&mut app, " fa.rs");
    _ = screen(60, 40, |f| app.draw(f)); // the PR's text re-wrapped: the rows moved
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.rows[app.head.row].line.kind, Kind::File);
    assert!(app.rows[app.head.row].line.text.contains("a.rs"));
}

#[test]
fn ctrl_o_survives_rebuilds() {
    // The comment seen meanwhile (a rebuild without its [new]): Ctrl-o still goes back.
    let mut app = margin_app();
    for t in &mut app.threads {
        for c in &mut t.comments {
            c.node_id = format!("n{}", c.id);
        }
    }
    app.build();
    app.seen.path = Some(std::env::temp_dir().join(format!("dv-seen-jumps-{}.json", std::process::id())));
    let four = |app: &App| app.rows.iter().position(|r| r.line.text == "four!").unwrap();
    app.goto(four(&app), 0);
    let _ = screen(120, 30, |f| app.draw(f));
    press(&mut app, "[c");
    app.watch();
    app.seen.since = app.seen.since.map(|t| t - std::time::Duration::from_secs(5));
    app.watch();
    ctrl(&mut app, 'o');
    let _ = std::fs::remove_file(app.seen.path.as_ref().unwrap());
    assert_eq!(app.rows[app.head.row].line.text, "four!");
    // A folded thread unfolded to get there: Ctrl-o back to the line, not its old row.
    let mut app = margin_app();
    let id = app.root_id(1).unwrap();
    app.toggled.insert(id, false);
    app.build();
    app.goto(four(&app), 0);
    press(&mut app, "[c");
    ctrl(&mut app, 'o');
    assert_eq!(app.rows[app.head.row].line.text, "four!");
}

#[test]
fn a_picker_open_holds_back_what_changed() {
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    let p = std::env::temp_dir().join(format!("dv-claude-picker-{}.json", std::process::id()));
    std::fs::write(&p, r#"{ "summary": "", "comments": [] }"#).unwrap();
    app.claude.path = Some(p.clone());
    app.load_claude();
    app.build();
    press(&mut app, " f");
    let want = app.picker.as_ref().unwrap().items.last().map(|i| i.label.clone()).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(&p, r#"{ "summary": "Looks fine.\n\nmore\n\nmore", "comments": [] }"#).unwrap();
    app.watch();
    for _ in 0..10 {
        app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    }
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let _ = std::fs::remove_file(&p);
    assert!(app.rows[app.head.row].line.text.starts_with(&want), "{want}");
}

#[test]
fn a_hunk_header_is_kept_across_rebuilds_and_ctrl_o() {
    let sample = "diff --git a/a.rs b/a.rs\nindex 1..2 100644\n--- a/a.rs\n+++ b/a.rs\n@@ -1,2 +1,2 @@ fn a\n one\n-two\n+two!\n@@ -20,2 +20,2 @@ fn b\n x\n-y\n+y!\n";
    let mut app = App::with("base".into(), diff::parse(sample));
    let h = app.rows.iter().rposition(|r| r.line.kind == Kind::Hunk).unwrap();
    app.goto(h, 0);
    let keep = app.cursor_key();
    app.rebuild(keep);
    assert_eq!(app.head.row, h, "a rebuild moved off the hunk's header");
    press(&mut app, " f");
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    ctrl(&mut app, 'o');
    assert_eq!(app.head.row, h, "Ctrl-o not back on the hunk's header");
}

#[test]
fn space_f_lists_the_files_list_and_goes_to_a_section_as_a_jump() {
    let mut app = fixture();
    _ = screen(200, 40, |f| app.draw(f));
    press(&mut app, " f");
    let labels: Vec<String> = app.picker.as_ref().unwrap().items.iter().map(|i| i.label.clone()).collect();
    assert!(!labels.iter().any(|l| l.contains("Fix the thing")), "the PR's title isn't a file: {labels:?}");
    app.picker = None;
    // A section, from the list, with a selection up: a jump (it ends, Ctrl-o comes back).
    let code = app.rows.iter().position(|r| r.line.kind.is_code()).unwrap();
    app.goto(code, 0);
    press(&mut app, "v");
    let id = app.rows.iter().find_map(|r| r.main).unwrap();
    app.goto_spot(super::files::Spot::Section(id));
    assert!(!app.select && app.anchor == app.head && app.rows[app.head.row].main == Some(id));
    ctrl(&mut app, 'o');
    assert_eq!(app.head.row, code);
    // Every item goes somewhere (none's the title, which would panic).
    press(&mut app, " f");
    let n = app.picker.as_ref().unwrap().items.len();
    for k in 0..n {
        press(&mut app, " f");
        (0..k).for_each(|_| app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)));
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    }
}

#[test]
fn a_listed_file_outside_the_view_is_shown_and_a_click_leaves_the_list() {
    let mut app = fixture();
    app.toggle_view(View::Pending);
    let big = app.files.iter().position(|f| f.path == "src/big.rs").unwrap();
    app.goto_spot(super::files::Spot::File(big));
    assert_eq!(app.rows[app.head.row].file, big, "a listed file outside the view went nowhere");
    // A click in the list, it focused: back to the code (Enter isn't the list's any more).
    let mut app = fixture();
    app.files_pref = Some(true);
    _ = screen(220, 30, |f| app.draw(f));
    while !app.files_focus {
        ctrl(&mut app, 'h');
    }
    let sel0 = app.files_sel;
    let other = app.list_hits.iter().position(|h| matches!(h, files::Hit::Spot(s) if Some(*s) != sel0 && matches!(s, files::Spot::File(_)))).unwrap();
    let files::Hit::Spot(clicked) = app.list_hits[other] else { panic!() };
    app.mouse(ratatui::crossterm::event::MouseEvent { kind: ratatui::crossterm::event::MouseEventKind::Down(ratatui::crossterm::event::MouseButton::Left), column: 3, row: (other + 1) as u16, modifiers: KeyModifiers::NONE });
    assert!(!app.files_focus);
    _ = screen(220, 30, |f| app.draw(f));
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.current_spot(), Some(clicked), "Enter after a click went back to the list's old cursor");
}

#[test]
fn a_count_goes_through_a_prefix() {
    let mut app = fixture();
    _ = screen(200, 60, |f| app.draw(f));
    app.goto(0, 0);
    press(&mut app, "]f]f");
    let two = app.head.row;
    app.goto(0, 0);
    press(&mut app, "2]f");
    assert_eq!((app.head.row, app.count), (two, 0));
    press(&mut app, "3gh"); // spent by a prefix's key that takes none
    assert_eq!(app.count, 0);
    // A counted jump is one jump: Ctrl-o back to where it began.
    app.goto(0, 0);
    press(&mut app, "3]c");
    assert_ne!(app.head.row, 0);
    ctrl(&mut app, 'o');
    assert_eq!(app.head.row, 0);
}

#[test]
fn jumps_are_moves_and_gg_is_one() {
    // ]c onto where you are: no jump; one Ctrl-o back to where it began.
    let mut app = margin_app();
    app.threads.truncate(1);
    app.build();
    app.goto(0, 0);
    press(&mut app, "]c]c]c");
    ctrl(&mut app, 'o');
    assert_eq!(app.head.row, 0);
    // gg / ge: jumps, a selection going on.
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    app.goto(4, 0);
    press(&mut app, "gg");
    ctrl(&mut app, 'o');
    assert_eq!(app.head.row, 4);
    press(&mut app, "vgg");
    assert_eq!((app.anchor.row, app.head.row), (4, 0));
}

#[test]
fn text_inputs_take_no_ctrl_chars_and_ctrl_c_closes_them() {
    let mut app = fixture();
    press(&mut app, " f");
    ctrl(&mut app, 'u');
    assert_eq!(app.picker.as_ref().map(|p| p.query.as_str()), Some(""));
    ctrl(&mut app, 'c');
    assert!(app.picker.is_none(), "Ctrl-c didn't close the picker");
    assert!(!app.quit);
    press(&mut app, "?/");
    ctrl(&mut app, 'x');
    assert_eq!(app.help.as_ref().map(|h| h.0.as_str()), Some(""));
    // An empty search: the last one again.
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    app.search = "call".into();
    press(&mut app, "/");
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.search, "call");
}

#[test]
fn focus_leaves_a_list_hidden_by_a_resize() {
    let mut app = fixture();
    app.files_pref = Some(true);
    _ = screen(220, 20, |f| app.draw(f));
    ctrl(&mut app, 'w');
    press(&mut app, "k");
    assert!(app.files_focus);
    _ = screen(40, 20, |f| app.draw(f)); // no room for it
    assert!(!app.files_focus, "focus in a list not shown");
}

#[test]
fn the_lists_cursor_stays_on_its_file_as_files_come() {
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    (app.files_w, app.height) = (30, 20);
    app.focus_list(false);
    press(&mut app, "j");
    let Some(files::Spot::File(i)) = app.files_sel else { panic!("{:?}", app.files_sel) };
    let was = app.files[i].path.clone();
    let extra = format!("diff --git a/0.rs b/0.rs\nnew file mode 100644\n--- /dev/null\n+++ b/0.rs\n@@ -0,0 +1 @@\n+x\n{}", diff::SAMPLE);
    app.set_files(diff::parse(&extra));
    let Some(files::Spot::File(i)) = app.files_sel else { panic!("{:?}", app.files_sel) };
    assert_eq!(app.files[i].path, was);
}

#[test]
fn the_list_is_entered_fresh_every_way() {
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    app.commits = (1..=3).map(|n| files::Commit { sha: format!("{n}{}", "0".repeat(39)), short: format!("c{n}"), at: String::new(), subject: format!("commit {n}") }).collect();
    app.files_pref = Some(true);
    _ = screen(220, 30, |f| app.draw(f));
    let esc = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
    // Into the commits and out; Ctrl-h at the code's left edge: the contents, not the commits.
    ctrl(&mut app, 'w');
    press(&mut app, "j");
    assert!(app.files_focus && app.in_commits);
    app.key(esc);
    app.goto(app.head.row, 0);
    ctrl(&mut app, 'h');
    assert!(app.files_focus && !app.in_commits, "Ctrl-h into the commits");
    // A range begun (v), the list left by another key: back in, none begun.
    app.key(esc);
    ctrl(&mut app, 'w');
    press(&mut app, "jjv");
    assert!(app.commit_anchor.is_some());
    press(&mut app, "?");
    app.key(esc);
    ctrl(&mut app, 'w');
    press(&mut app, "j");
    assert_eq!(app.commit_anchor, None, "a range kept from before");
}

#[test]
fn a_change_with_a_comment_in_it_is_one_stop_for_g() {
    let sample = "diff --git a/a.rs b/a.rs\nindex 1..2 100644\n--- a/a.rs\n+++ b/a.rs\n@@ -1,1 +1,4 @@\n one\n+two\n+three\n+four\n";
    let mut app = App::with("base".into(), diff::parse(sample));
    let c = gh::Comment { id: 1, node_id: String::new(), can_edit: false, can_delete: false, author: "amy".into(), body: "hm".into(), created: String::new(), pending: false, reactions: vec![], hidden: String::new() };
    app.threads = vec![gh::Thread {
        id: String::new(), path: "a.rs".into(), left: false, line: Some(2), start_line: None, start_left: None, original_line: Some(2),
        resolved: false, outdated: false, hunk: vec![], comments: vec![c], claude: None, severity: String::new(), about: None, conversation: String::new(),
        talk_resolved: false, want: String::new(),
    }];
    app.margin_active = true;
    app.build();
    app.goto(0, 0);
    press(&mut app, "]g");
    let two = app.head.row;
    assert_eq!(app.rows[two].line.text, "two");
    press(&mut app, "]g");
    assert_eq!(app.head.row, two, "]g stopped inside the change, past its comment");
}

#[test]
fn comment_motions_go_by_where_threads_are_drawn() {
    let sample = "diff --git a/a.rs b/a.rs\nindex 1..2 100644\n--- a/a.rs\n+++ b/a.rs\n@@ -1,13 +1,4 @@\n a\n-x1\n-x2\n-x3\n-x4\n-x5\n-x6\n-x7\n-x8\n b\n c\n-d\n e\n";
    let c = |id| gh::Comment { id, node_id: String::new(), can_edit: false, can_delete: false, author: "amy".into(), body: "hm".into(), created: String::new(), pending: false, reactions: vec![], hidden: String::new() };
    let t = |left, line, id| gh::Thread {
        id: format!("t{id}"), path: "a.rs".into(), left, line: Some(line), start_line: None, start_left: None, original_line: Some(line),
        resolved: false, outdated: false, hunk: vec![], comments: vec![c(id)], claude: None, severity: String::new(), about: None, conversation: String::new(),
        talk_resolved: false, want: String::new(),
    };
    // On a removed line (old 12) after deletions: ]c goes down to e's (new 4), not back to b's.
    let mut app = App::with("base".into(), diff::parse(sample));
    app.threads = vec![t(false, 2, 1), t(false, 4, 2)];
    app.margin_active = true;
    app.build();
    let d = app.rows.iter().position(|r| r.line.text == "d").unwrap();
    app.goto(d, 0);
    press(&mut app, "]c");
    assert_eq!(app.rows[app.head.row].thread, Some(1));
    // A left thread on d and a right one on e: d's first, as drawn.
    let mut app = App::with("base".into(), diff::parse(sample));
    app.threads = vec![t(true, 12, 1), t(false, 4, 2)];
    app.margin_active = true;
    app.build();
    app.goto(0, 0);
    press(&mut app, "]c");
    assert_eq!(app.rows[app.head.row].thread, Some(0));
}

#[test]
fn split_view_motions_and_copies_keep_to_your_half() {
    let sample = "diff --git a/a.rs b/a.rs\nindex 1..2 100644\n--- a/a.rs\n+++ b/a.rs\n@@ -1,4 +1,4 @@\n one\n two\n-three\n+three!\n four\n";
    let mut app = App::with("base".into(), diff::parse(sample));
    app.split = true;
    app.build();
    let two = app.rows.iter().position(|r| r.line.text == "two").unwrap();
    app.right = true;
    app.goto(two, 0);
    press(&mut app, "xx");
    assert_eq!(app.rows[app.head.row].line.text, "three!");
    assert_eq!(app.selection_text(false), "two\nthree!\n");
    // ]g still stops once a change (its removed and added lines one).
    app.goto(0, 0);
    press(&mut app, "]g]g");
    assert_eq!(app.rows[app.head.row].line.text, "three");
}

#[test]
fn c_takes_a_comment_under_the_header_first() {
    // A thread on hidden context (new line 8) is drawn under the header: ]c from the header
    // goes there first, then on down, never back up.
    let mut body = String::from("diff --git a/a.rs b/a.rs\nindex 1..2 100644\n--- a/a.rs\n+++ b/a.rs\n@@ -1,13 +1,15 @@\n+top\n");
    for i in 1..=13 {
        body.push_str(&format!(" l{i}\n"));
    }
    body.push_str("+bottom\n");
    let c = |id| gh::Comment { id, node_id: String::new(), can_edit: false, can_delete: false, author: "amy".into(), body: "hm".into(), created: String::new(), pending: false, reactions: vec![], hidden: String::new() };
    let t = |line, id| gh::Thread {
        id: format!("t{id}"), path: "a.rs".into(), left: false, line: Some(line), start_line: None, start_left: None, original_line: Some(line),
        resolved: false, outdated: false, hunk: vec![], comments: vec![c(id)], claude: None, severity: String::new(), about: None, conversation: String::new(),
        talk_resolved: false, want: String::new(),
    };
    let mut app = App::with("base".into(), diff::parse(&body));
    app.threads = vec![t(8, 1), t(1, 2)];
    app.build();
    let hdr = app.rows.iter().position(|r| r.line.kind == Kind::File).unwrap();
    app.goto(hdr, 0);
    press(&mut app, "]c");
    let first = app.head.row;
    assert_eq!(app.rows[first].thread, Some(0));
    press(&mut app, "]c");
    assert!(app.head.row > first && app.rows[app.head.row].thread == Some(1), "]c went back up");
}

#[test]
fn a_split_selection_is_drawn_copied_and_suggested_alike() {
    let sample = "diff --git a/a.rs b/a.rs\nindex 1..2 100644\n--- a/a.rs\n+++ b/a.rs\n@@ -1,4 +1,4 @@\n one\n two\n-three\n+three!\n four\n";
    let mut app = App::with("base".into(), diff::parse(sample));
    app.split = true;
    app.build();
    let two = app.rows.iter().position(|r| r.line.text == "two").unwrap();
    app.right = true;
    app.goto(two, 0);
    press(&mut app, "xx");
    let three = app.rows.iter().position(|r| r.line.text == "three").unwrap();
    assert!(!app.in_selection(three), "the left half's removed line drawn selected");
    assert_eq!(app.selection_text(false), "two\nthree!\n");
    assert!(app.suggestion().is_ok(), "{:?}", app.suggestion().err());
}

#[test]
fn a_left_half_selection_comments_and_refers_to_the_old_side() {
    let sample = "diff --git a/a.rs b/a.rs\nindex 1..2 100644\n--- a/a.rs\n+++ b/a.rs\n@@ -1,4 +1,4 @@\n one\n two\n-three\n+three!\n four\n";
    let mut app = App::with("base".into(), diff::parse(sample));
    app.split = true;
    app.build();
    let two = app.rows.iter().position(|r| r.line.text == "two").unwrap();
    app.right = false;
    app.goto(two, 0);
    press(&mut app, "xxx");
    assert_eq!(app.selection_text(false), "two\nthree\nfour\n");
    let a = app.selection_anchor().unwrap();
    assert_eq!((a.line, a.left, a.start), (4, true, Some((2, true))));
    assert_eq!(app.file_ref().unwrap(), "a.rs:2-4 (the old side, at base base)");
    // From the removed line: the old side's numbers.
    let three = app.rows.iter().position(|r| r.line.text == "three").unwrap();
    app.goto(three, 0);
    press(&mut app, "xx");
    assert!(app.file_ref().unwrap().starts_with("a.rs:3-4"), "{:?}", app.file_ref());
}

#[test]
fn a_split_selection_stays_on_the_half_it_began() {
    let sample = "diff --git a/a.rs b/a.rs\nindex 1..2 100644\n--- a/a.rs\n+++ b/a.rs\n@@ -1,4 +1,4 @@\n one\n two\n-three\n+three!\n four\n";
    let mut app = App::with("base".into(), diff::parse(sample));
    app.split = true;
    app.right = true;
    app.build();
    app.goto(0, 0);
    press(&mut app, "]gxx"); // onto the removed line, the side you chose the right
    assert_eq!(app.selection_text(false), "three\nfour\n");
    let a = app.selection_anchor().unwrap();
    assert_eq!((a.line, a.left, a.start), (4, true, Some((3, true))));
    assert!(app.file_ref().unwrap().starts_with("a.rs:3-4"));
}

#[test]
fn a_split_select_mode_selection_stays_on_its_half() {
    let sample = "diff --git a/a.rs b/a.rs\nindex 1..2 100644\n--- a/a.rs\n+++ b/a.rs\n@@ -1,4 +1,4 @@\n one\n two\n-three\n+three!\n four\n";
    let mut app = App::with("base".into(), diff::parse(sample));
    app.split = true;
    app.right = true;
    app.build();
    app.goto(0, 0);
    press(&mut app, "]gvj");
    assert_eq!(app.selection_text(false), "three\nf", "onto the right half");
}

#[test]
fn select_mode_j_passes_a_removed_line_across_a_hatch() {
    let sample = "diff --git a/a.rs b/a.rs\nindex 1..2 100644\n--- a/a.rs\n+++ b/a.rs\n@@ -1,5 +1,4 @@\n one\n-gone\n two\n-three\n+three!\n four\n";
    let mut app = App::with("base".into(), diff::parse(sample));
    app.split = true;
    app.right = true;
    app.build();
    let one = app.rows.iter().position(|r| r.line.text == "one").unwrap();
    app.goto(one, 0);
    press(&mut app, "vjjj");
    assert_eq!(app.selection_text(false), "one\ntwo\nthree!\nf");
}

#[test]
fn the_wheel_in_select_mode_keeps_a_split_selection_on_its_half() {
    let body: String = (0..60).map(|i| format!(" ctx{i}\n")).collect();
    let sample = format!("diff --git a/a.rs b/a.rs\nindex 1..2 100644\n--- a/a.rs\n+++ b/a.rs\n@@ -1,63 +1,63 @@\n one\n-three\n+three!\n two\n{body}");
    let mut app = App::with("base".into(), diff::parse(&sample));
    (app.files_pref, app.margin_pref) = (Some(false), Some(false));
    (app.split, app.right) = (true, true);
    app.build();
    _ = screen(120, 14, |f| app.draw(f));
    let one = app.rows.iter().position(|r| r.line.text == "one").unwrap();
    app.goto(one, 0);
    press(&mut app, "v/thre");
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)); // onto the removed line, the left
    _ = screen(120, 14, |f| app.draw(f));
    for _ in 0..3 {
        app.mouse(ratatui::crossterm::event::MouseEvent { kind: ratatui::crossterm::event::MouseEventKind::ScrollDown, column: 60, row: 5, modifiers: KeyModifiers::NONE });
        _ = screen(120, 14, |f| app.draw(f));
    }
    assert!(app.selection_text(false).starts_with("one\nthree\n"), "{:?}", app.selection_text(false));
}

#[test]
fn the_wheel_in_select_mode_moves_on_over_a_tall_removed_block() {
    let gone: String = (0..40).map(|i| format!("-gone{i}\n")).collect();
    let sample = format!("diff --git a/a.rs b/a.rs\nindex 1..2 100644\n--- a/a.rs\n+++ b/a.rs\n@@ -1,42 +1,2 @@\n one\n{gone} two\n");
    let mut app = App::with("base".into(), diff::parse(&sample));
    (app.files_pref, app.margin_pref) = (Some(false), Some(false));
    (app.split, app.right) = (true, true);
    app.build();
    _ = screen(120, 14, |f| app.draw(f));
    let one = app.rows.iter().position(|r| r.line.text == "one").unwrap();
    app.goto(one, 0);
    press(&mut app, "v");
    for _ in 0..3 {
        app.mouse(ratatui::crossterm::event::MouseEvent { kind: ratatui::crossterm::event::MouseEventKind::ScrollDown, column: 60, row: 5, modifiers: KeyModifiers::NONE });
        _ = screen(120, 14, |f| app.draw(f));
    }
    assert!(app.top > 0 && app.head.row != one, "the view froze: top {} head {:?}", app.top, app.head);
}

#[test]
fn the_commits_list_knows_uncommitted_is_shown_and_k_goes_up_one() {
    let mut app = fixture();
    app.files_pref = Some(true);
    _ = screen(220, 30, |f| app.draw(f));
    let c = |sha: &str| files::Commit { sha: sha.into(), short: sha.into(), subject: String::new(), at: String::new() };
    app.commits = vec![c("aaa"), c("bbb")];
    app.base = "HEAD".into(); // showing what isn't committed
    app.dirty = files::Dirty { changed: 1, ..Default::default() };
    let n = app.commits.len();
    assert_eq!(app.shown_rows(), (n + 1, n + 1));
    app.focus_list(true);
    assert_eq!(app.commit_cur, n + 1, "the cursor not on uncommitted, the one shown");
    app.commit_picker();
    let p = app.picker.take().unwrap();
    assert!(p.items[1].label.starts_with('●') && p.items[1].label.contains("uncommitted"), "uncommitted: newest, after all commits");
    // k from "all commits": the list's last entry, not where the diff is.
    let mut app = fixture();
    app.files_pref = Some(true);
    app.commits = vec![c("aaa")];
    _ = screen(220, 30, |f| app.draw(f));
    app.focus_list(true);
    app.commit_cur = 0;
    press(&mut app, "k");
    let last = app.file_entries().into_iter().map(|e| e.spot).rfind(|&s| s != files::Spot::Folder);
    assert_eq!((app.in_commits, app.files_sel), (false, last));
}

#[test]
fn ctrl_w_e_says_when_theres_no_room() {
    let mut app = fixture();
    _ = screen(40, 20, |f| app.draw(f));
    let pref = app.files_pref;
    ctrl(&mut app, 'w');
    press(&mut app, "e");
    assert_eq!(app.msg, "too narrow for the files' list");
    assert_eq!(app.files_pref, pref, "a setting saved that can't show");
    // 90 wide: room (a third, past its least), so it shows.
    _ = screen(90, 20, |f| app.draw(f));
    ctrl(&mut app, 'w');
    press(&mut app, "e");
    _ = screen(90, 20, |f| app.draw(f));
    assert!(app.files_w > 0);
}

#[test]
fn ctrl_w_e_while_maximized_shows_the_list() {
    let mut app = fixture();
    app.files_pref = Some(true);
    _ = screen(220, 30, |f| app.draw(f));
    ctrl(&mut app, 'w');
    press(&mut app, "o");
    _ = screen(220, 30, |f| app.draw(f));
    assert_eq!(app.files_w, 0);
    ctrl(&mut app, 'w');
    press(&mut app, "e");
    _ = screen(220, 30, |f| app.draw(f));
    assert!(app.files_w > 0 && app.files_pref == Some(true) && app.zoom.is_none(), "{}", app.msg);
}

#[test]
fn a_on_uncommitted_says_what_a_does() {
    let mut app = fixture();
    app.files_pref = Some(true);
    let c = |sha: &str| files::Commit { sha: sha.into(), short: sha.into(), subject: String::new(), at: String::new() };
    app.commits = vec![c("aaa")];
    app.dirty = files::Dirty { changed: 1, ..Default::default() };
    _ = screen(220, 30, |f| app.draw(f));
    app.focus_list(true);
    app.commit_cur = app.last_commit_row();
    press(&mut app, "a");
    assert!(app.msg.starts_with("a on a commit"), "{:?}", app.msg);
}

#[test]
fn a_viewed_files_row_is_as_wide_as_the_others() {
    let mut app = fixture();
    app.files_pref = Some(true);
    app.viewed.insert("src/a.rs".into(), "VIEWED".into());
    let s = screen(220, 30, |f| app.draw(f));
    // Each file's row: its comment count ends at the list's edge, viewed (✓) or not.
    let ends: Vec<usize> = s.lines().map(|l| l.split('│').next().unwrap()).filter(|l| l.contains('·') && (l.contains("a.rs") || l.contains("big.rs"))).map(|l| l.trim_end().chars().count()).collect();
    assert!(ends.len() >= 2 && ends.windows(2).all(|w| w[0] == w[1]), "{ends:?}\n{s}");
}

#[test]
fn the_list_measures_wide_chars_in_columns() {
    assert_eq!(files::cut("说明书很长", 6), "说明…");
    assert_eq!(files::cut("ab", 2), "ab");
    let sample = "diff --git a/文档/说明书很长的名字测试.md b/文档/说明书很长的名字测试.md\nindex 1..2 100644\n--- a/文档/说明书很长的名字测试.md\n+++ b/文档/说明书很长的名字测试.md\n@@ -1 +1 @@\n-a\n+b\n";
    let mut app = App::with("base".into(), diff::parse(sample));
    app.files_pref = Some(true);
    app.files_w_pref = Some(20);
    let s = screen(120, 20, |f| app.draw(f));
    let row = s.lines().map(|l| l.split('│').next().unwrap()).find(|l| l.contains('说')).expect(&s);
    assert!(row.contains('…'), "a wide name not cut to the list: {row}");
}

#[test]
fn an_emoji_with_its_selector_is_two_columns() {
    let s = "♻\u{fe0f} refactor ♻\u{fe0f} things";
    let c = files::cut(s, 12);
    assert!(Line::from(c.as_str()).width() <= 12, "{c:?} {}", Line::from(c.as_str()).width());
    assert!(c.ends_with('…'));
}

#[test]
fn a_tab_is_four_columns() {
    assert_eq!(row::cols("\t"), 4);
    assert_eq!(row::cols("a\tb"), 6);
}

#[test]
fn a_folder_is_listed_once() {
    let d = |p: &str| format!("diff --git a/{p} b/{p}\nindex 1..2 100644\n--- a/{p}\n+++ b/{p}\n@@ -1 +1 @@\n-a\n+b\n");
    let app = App::with("base".into(), diff::parse(&[d("src/foo.rs"), d("src/foo/bar.rs"), d("src/main.rs")].concat()));
    let paths: Vec<&str> = app.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, ["src/foo.rs", "src/main.rs", "src/foo/bar.rs"]);
    let folders = app.file_entries().iter().filter(|e| e.spot == files::Spot::Folder).count();
    assert_eq!(folders, 2);
}

#[test]
fn root_files_have_a_heading_among_folders() {
    let d = |p: &str| format!("diff --git a/{p} b/{p}\nindex 1..2 100644\n--- a/{p}\n+++ b/{p}\n@@ -1 +1 @@\n-a\n+b\n");
    let app = App::with("base".into(), diff::parse(&[d("benches/x.rs"), d("build.rs")].concat()));
    let list: Vec<String> = app.file_entries().into_iter().map(|e| e.text).collect();
    assert_eq!(list, ["benches/", "x.rs", "./", "build.rs"]);
    // Only root files: no heading.
    let app = App::with("base".into(), diff::parse(&d("build.rs")));
    assert_eq!(app.file_entries().into_iter().map(|e| e.text).collect::<Vec<_>>(), ["build.rs"]);
}

#[test]
fn a_new_comment_unfolds_its_viewed_file_however_it_arrives() {
    let review = |n: usize| {
        let cs: Vec<gh::Comment> = (0..n).map(|i| gh::Comment { id: 10 + i as u64, node_id: format!("N{i}"), author: "amy".into(), body: "hm".into(), ..Default::default() }).collect();
        gh::Review {
            pr_id: "PR".into(), mine: false, me: "me".into(), pending: None, pending_body: String::new(), pending_on: String::new(),
            threads: vec![gh::Thread { id: "T".into(), path: "src/a.rs".into(), line: Some(2), comments: cs, ..Default::default() }],
            viewed: HashMap::from([("src/a.rs".to_owned(), "VIEWED".to_owned())]),
        }
    };
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    app.seen.path = Some(std::env::temp_dir().join(format!("dv-seen-unfold-{}", std::process::id())));
    app.apply_github((Ok(review(0)), Err(anyhow!("x"))), true);
    assert!(app.collapsed.contains("src/a.rs"));
    // As a reload has it (R, after a post): GitHub's answer, then Claude's file.
    app.apply_github((Ok(review(1)), Err(anyhow!("x"))), true);
    app.load_claude();
    assert!(!app.collapsed.contains("src/a.rs"), "a viewed file with a new comment stayed folded");
}

#[test]
fn c_goes_through_gone_files_threads_as_drawn() {
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    let c = |id: u64, body: &str| gh::Comment { id, node_id: format!("C{id}"), author: "amy".into(), body: body.into(), ..Default::default() };
    let t = |id: &str, line: u32, cm| gh::Thread { id: id.into(), path: "src/reverted.rs".into(), line: Some(line), original_line: Some(line), comments: vec![cm], ..Default::default() };
    app.threads = vec![t("T1", 30, c(1, "thirty")), t("T2", 10, c(2, "ten")), gh::Thread { path: "src/a.rs".into(), line: Some(2), comments: vec![c(3, "here")], ..Default::default() }];
    app.pr = Some(gh::Pr { number: 1, node_id: "PR".into(), title: "t".into(), base: "main".into(), head_oid: "abc".into(), url: String::new(), ..Default::default() });
    app.build();
    let r0 = app.rows.iter().position(|r| r.thread == Some(0)).unwrap();
    let r1 = app.rows.iter().position(|r| r.thread == Some(1)).unwrap();
    assert!(r0 < r1, "drawn in thread order");
    app.goto(r0, 0);
    app.key(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
    assert_eq!(app.rows[app.head.row].thread, Some(1), "]c from the first gone thread drawn goes to the one under it");
}

#[test]
fn a_thread_opened_with_tab_stays_open_unresolved() {
    let mut app = fixture();
    // amy's resolved thread on src/a.rs:1 (index 3): folded; Tab opens it to read.
    let head = app.rows.iter().position(|r| r.thread == Some(3)).unwrap();
    app.goto(head, 0);
    assert!(!app.thread_open(3));
    app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert!(app.thread_open(3), "Tab opens it");
    // Space r unresolves it; GitHub's answer (the post-job reload) says so.
    app.threads[3].resolved = false;
    let keep = app.cursor_key();
    app.rebuild(keep);
    assert!(app.thread_open(3), "unresolved, the thread you opened stays open");
}

#[test]
fn a_pending_review_on_another_head_refuses_lines_and_an_approval_read_on_this() {
    let mut app = fixture();
    (app.review, app.review_on) = (Some("R".into()), "aaa".into());
    let e = app.ensure_review("", Some("bbb")).unwrap_err().to_string();
    assert!(e.contains("your pending review is on aaa"), "{e}");
    assert_eq!(app.ensure_review("", Some("aaa")).unwrap(), "R");
    assert_eq!(app.ensure_review("", None).unwrap(), "R", "a reply has no lines");
}

#[test]
fn a_jump_to_a_thread_doesnt_pin_it_open() {
    let mut app = fixture();
    let t = (0..app.threads.len()).find(|&t| app.thread_open(t) && app.threads[t].claude.is_none()).unwrap();
    app.goto_comment(t, 0);
    app.threads[t].resolved = true;
    let keep = app.cursor_key();
    app.rebuild(keep);
    assert!(!app.thread_open(t), "resolved after a jump onto it, it stays open");
}

#[test]
fn c_on_lines_is_refused_up_front_with_a_review_on_another_head() {
    let mut app = fixture();
    app.pr = Some(gh::Pr { number: 1, node_id: "PR".into(), head_oid: "new".into(), ..Default::default() });
    (app.review, app.review_on) = (Some("R".into()), "old".into());
    let code = app.rows.iter().position(|r| r.line.kind == Kind::Add).unwrap();
    app.goto(code, 0);
    let keys = app.context_keys().map(|(_, k)| k.iter().map(|e| e.0).collect::<Vec<_>>()).unwrap_or_default();
    assert!(!keys.contains(&"c") && !keys.contains(&"s"), "{keys:?}");
    assert!(app.on_head().err().is_some_and(|e| e.to_string().contains("pending review is on old")));
    assert!(app.posts_now().err().is_some_and(|e| e.to_string().contains("pending review is on old")), "C sends you to a c that's refused");
    // Submitted (or discarded): its old commit no longer counts.
    app.review = None;
    assert!(app.on_head().is_ok());
}

#[test]
fn a_folded_threads_reply_isnt_seen_by_passing() {
    let mut app = fixture();
    app.seen.path = Some(std::env::temp_dir().join(format!("dv-probe-seen-{}", std::process::id())));
    app.threads[3].comments.push(gh::Comment { id: 99, node_id: "N99".into(), author: "bo".into(), body: "actually not fixed".into(), ..Default::default() });
    for (i, t) in app.threads.iter_mut().enumerate() { for (j, c) in t.comments.iter_mut().enumerate() { if c.node_id.is_empty() { c.node_id = format!("n{i}{j}"); } } }
    app.build();
    assert!(app.is_new(&app.threads[3]) && !app.thread_open(3));
    let r = app.rows.iter().position(|r| r.thread == Some(3)).unwrap();
    app.goto(r - 1, 0);
    app.watch_seen();
    app.goto(r, 0); // j onto its folded heading
    app.watch_seen();
    app.goto(r + 1, 0); // j past it
    app.watch_seen();
    _ = std::fs::remove_file(app.seen.path.as_ref().unwrap());
    assert!(!app.thread_open(3));
    assert!(app.is_new(&app.threads[3]), "the folded reply, never shown, counted as seen");
}

#[test]
fn a_github_thread_follows_its_line_through_local_edits() {
    // PR head: line 2 is `    new_call(x);`; the working tree adds two lines above it.
    let text = "diff --git a/src/a.rs b/src/a.rs\nindex 1..2 100644\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,3 +1,5 @@\n fn main() {\n+    // local 1\n+    // local 2\n-    old();\n+    new_call(x);\n }\n";
    let mut app = App::with("base".into(), diff::parse(text));
    let c = gh::Comment { id: 1, node_id: "n".into(), author: "amy".into(), body: "hm".into(), ..Default::default() };
    app.threads = vec![gh::Thread { id: "T".into(), path: "src/a.rs".into(), line: Some(2), original_line: Some(2), hunk: vec![" fn main() {".into(), "-    old();".into(), "+    new_call(x);".into()], comments: vec![c], ..Default::default() }];
    app.build();
    let r = app.rows.iter().position(|r| r.thread == Some(0)).unwrap();
    let above = app.rows[..r].iter().rev().find(|r| r.line.kind.is_code()).unwrap();
    eprintln!("thread under: {:?}", above.line.text);
    assert_eq!(above.line.text, "    new_call(x);", "the thread sits under the line it's about");
}

#[test]
fn c_on_claudes_comment_or_a_thread_names_what_queues_there() {
    let mut app = fixture();
    app.pr = Some(gh::Pr { number: 1, node_id: "PR".into(), head_oid: "new".into(), ..Default::default() });
    (app.review, app.review_on) = (Some("R".into()), "old".into());
    let claude = (0..app.threads.len()).find(|&t| app.threads[t].claude.is_some()).unwrap();
    app.goto(app.rows.iter().position(|r| r.thread == Some(claude)).unwrap(), 0);
    assert!(app.posts_now().err().is_some_and(|e| e.to_string().contains("pending review is on old")));
    let theirs = (0..app.threads.len()).find(|&t| app.threads[t].claude.is_none()).unwrap();
    app.goto(app.rows.iter().position(|r| r.thread == Some(theirs)).unwrap(), 0);
    assert!(app.posts_now().err().is_some_and(|e| e.to_string().contains("r adds a reply")));
}

#[test]
fn a_thread_on_a_line_edited_here_stays_on_it() {
    // Its line edited (as answering the comment does); the old text as context far below.
    let mut text = String::from("diff --git a/src/a.rs b/src/a.rs\nindex 1..2 100644\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,41 +1,41 @@\n fn main() {\n-    foo_bar(x);\n+    foo_bar(y);\n");
    for i in 3..40 {
        text.push_str(&format!(" l{i}\n"));
    }
    text.push_str("     foo_bar(x);\n }\n");
    let mut app = App::with("base".into(), diff::parse(&text));
    let c = gh::Comment { id: 1, node_id: "n".into(), author: "amy".into(), body: "y?".into(), ..Default::default() };
    app.threads = vec![gh::Thread { id: "T".into(), path: "src/a.rs".into(), line: Some(2), original_line: Some(2), hunk: vec![" fn main() {".into(), "+    foo_bar(x);".into()], comments: vec![c], ..Default::default() }];
    app.build();
    let r = app.rows.iter().position(|r| r.thread == Some(0)).unwrap();
    let above = app.rows[..r].iter().rev().find(|r| r.line.kind.is_code()).unwrap();
    assert_eq!(above.line.new, Some(2), "moved to a copy of its old text: {:?}", above.line.text);
}

#[test]
fn a_head_line_maps_through_the_moves_here() {
    use crate::diff::{hunks_by_file, moved};
    // 3 lines added after line 1; line 5 edited; 2 removed at 8-9.
    // (A file deleted here after it: its hunks its own.)
    let m = hunks_by_file(&diff::parse("diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,0 +2,3 @@\n+a\n+b\n+c\n@@ -5 +8 @@\n-x\n+y\n@@ -8,2 +10,0 @@\n-p\n-q\ndiff --git a/b.rs b/b.rs\ndeleted file mode 100644\n--- a/b.rs\n+++ /dev/null\n@@ -1,20 +0,0 @@\n-z\n"));
    assert_eq!(m["a.rs"].len(), 3);
    let a = &m["a.rs"];
    assert_eq!([1, 2, 4, 5, 7, 10].map(|n| moved(a, n)), [1, 5, 7, 8, 10, 11]);
}

#[test]
fn a_github_thread_follows_its_lines_however_far_they_moved_here() {
    // 15 lines added above its line here (the head's line 2): it's on line 17, and a range's
    // start moves with its end.
    let mut text = String::from("diff --git a/a.rs b/a.rs\nindex 1..2 100644\n--- a/a.rs\n+++ b/a.rs\n@@ -1,4 +1,19 @@\n fn main() {\n");
    for i in 0..15 {
        text.push_str(&format!("+    added_{i}();\n"));
    }
    text.push_str("     foo_bar(x);\n     baz(y);\n }\n");
    let mut app = App::with("base".into(), diff::parse(&text));
    app.head_moves = HashMap::from([("a.rs".to_owned(), vec![(1, 0, 2, 15)])]);
    let c = gh::Comment { id: 1, node_id: "n".into(), author: "amy".into(), body: "these".into(), ..Default::default() };
    app.threads = vec![gh::Thread { id: "T".into(), path: "a.rs".into(), line: Some(3), start_line: Some(2), original_line: Some(3), hunk: vec![" fn main() {".into(), " foo_bar(x);".into(), " baz(y);".into()], comments: vec![c], ..Default::default() }];
    app.margin_active = true;
    app.build();
    let r = app.rows.iter().position(|r| r.thread == Some(0)).unwrap();
    let above = app.rows[..r].iter().rev().find(|r| r.line.kind.is_code()).unwrap();
    assert_eq!(above.line.text.trim(), "baz(y);");
    let (a, b) = app.code_span(0).unwrap();
    let lit: Vec<_> = (a..=b).map(|r| app.rows[r].line.text.trim().to_owned()).collect();
    assert_eq!(lit, ["foo_bar(x);", "baz(y);"]);
}

#[test]
fn asking_claude_about_a_moved_comment_quotes_its_code_here() {
    let mut text = String::from("diff --git a/a.rs b/a.rs\nindex 1..2 100644\n--- a/a.rs\n+++ b/a.rs\n@@ -1,4 +1,19 @@\n fn main() {\n");
    for i in 0..15 {
        text.push_str(&format!("+    added_{i}();\n"));
    }
    text.push_str("     foo_bar(x);\n     baz(y);\n }\n");
    let mut app = App::with("base".into(), diff::parse(&text));
    app.head_moves = HashMap::from([("a.rs".to_owned(), vec![(1, 0, 2, 15)])]);
    let c = gh::Comment { id: 1, node_id: "n".into(), author: "amy".into(), body: "these".into(), ..Default::default() };
    app.threads = vec![gh::Thread { id: "T".into(), path: "a.rs".into(), line: Some(3), original_line: Some(3), hunk: vec![" fn main() {".into(), " foo_bar(x);".into(), " baz(y);".into()], comments: vec![c], ..Default::default() }];
    app.build();
    let r = app.rows.iter().position(|r| r.thread == Some(0)).unwrap();
    app.goto(r + 1, 0);
    app.ask_about(0);
    let Some(Action::AskClaude { code, .. }) = app.action.take() else { panic!("{}", app.msg) };
    assert_eq!(code, ["    baz(y);"]);
}

#[test]
fn a_moved_threads_labels_name_its_line_here() {
    let mut text = String::from("diff --git a/a.rs b/a.rs\nindex 1..2 100644\n--- a/a.rs\n+++ b/a.rs\n@@ -1,4 +1,19 @@\n fn main() {\n");
    for i in 0..15 {
        text.push_str(&format!("+    added_{i}();\n"));
    }
    text.push_str("     foo_bar(x);\n     baz(y);\n }\n");
    let mut app = App::with("base".into(), diff::parse(&text));
    app.head_moves = HashMap::from([("a.rs".to_owned(), vec![(1, 0, 2, 15)])]);
    let c = gh::Comment { id: 1, node_id: "n".into(), author: "amy".into(), body: "these".into(), ..Default::default() };
    app.threads = vec![gh::Thread { id: "T".into(), path: "a.rs".into(), line: Some(3), original_line: Some(3), comments: vec![c], ..Default::default() }];
    app.build();
    assert!(app.rows.iter().any(|r| r.thread == Some(0) && r.line.text.contains("L18")), "its header names the head's L3");
    app.threads_picker(THREADS_ALL);
    assert!(app.picker.as_ref().unwrap().items.iter().any(|i| i.label.contains("a.rs:18")));
}

#[test]
fn an_events_line_isnt_quoted_nor_hinted_as_text() {
    let mut app = fixture();
    let now = crate::claude::now();
    let v = serde_json::json!({
        "id": "PR_1", "number": 7, "title": "Fix", "url": "https://github.com/o/r/pull/7", "headRefOid": "abc", "author": { "login": "bo" },
        "state": "OPEN", "isDraft": false, "baseRefName": "main", "headRefName": "fix", "body": "Fixes it.",
        "dvActivity": { "reactions": [], "items": [
            { "__typename": "IssueComment", "id": "c1", "url": "https://github.com/o/r/pull/7#issuecomment-1", "createdAt": now, "author": { "login": "amy" }, "body": "One." },
            { "__typename": "LabeledEvent", "id": "L1", "createdAt": now, "actor": { "login": "amy" }, "label": { "name": "bug", "color": "d73a4a" } },
        ] },
    });
    let me = gh::parse_threads(&serde_json::json!({ "viewer": { "login": "me" }, "repository": { "pullRequest": { "viewerDidAuthor": false } } }));
    app.apply_github((Ok(me), Ok(info::parse_info(&v))), true);
    app.build();
    let r = app.rows.iter().position(|r| r.line.text.starts_with("● @amy added")).unwrap();
    app.goto(r, 3);
    assert!(app.quotable().is_none());
    assert!(app.context_keys().is_none_or(|(t, _)| t != "PR text"), "{:?}", app.context_keys());
    app.start_conversation_comment();
    assert!(matches!(&app.action, Some(Action::Converse { quote, .. }) if quote.is_empty()));
    // The rerun of failed checks: offered, whether or not a required one failed.
    app.failing = 0;
    app.git_menu();
    assert!(app.picker.as_ref().unwrap().items.iter().any(|i| i.label.starts_with("Rerun the failed checks")));
}

#[test]
fn a_bots_login_and_page() {
    assert_eq!(gh::login(&serde_json::json!({ "login": "app/dependabot", "is_bot": true })), "dependabot");
    let v = serde_json::json!({ "number": 1, "title": "Bump", "url": "u", "headRefOid": "a", "state": "OPEN", "isDraft": false, "author": { "login": "app/dependabot", "is_bot": true } });
    let lines = info::parse_info(&v).lines;
    let author = lines.iter().find(|l| l.text.starts_with("@dependabot")).expect("its author line");
    assert!(author.marks.iter().any(|m| matches!(&m.2, info::Mark::Link(u) if u == "https://github.com/apps/dependabot")));
    // An issue's (GraphQL's shape): a Bot.
    assert_eq!(gh::page(&serde_json::json!({ "__typename": "Bot", "login": "github-actions" })), "https://github.com/apps/github-actions");
    assert_eq!(gh::page(&serde_json::json!({ "__typename": "User", "login": "amy" })), "https://github.com/amy");
}

#[test]
fn a_closed_issues_chip_is_coloured_by_why() {
    let st = |v: serde_json::Value| info::state(&v);
    assert_eq!(st(serde_json::json!({ "__typename": "Issue", "state": "CLOSED", "stateReason": "COMPLETED" })), ("CLOSED", "8957e5"));
    assert_eq!(st(serde_json::json!({ "__typename": "Issue", "state": "CLOSED", "stateReason": "NOT_PLANNED" })), ("CLOSED", "6e7681"));
    assert_eq!(st(serde_json::json!({ "state": "CLOSED" })), ("CLOSED", "da3633"), "a PR's");
}

#[test]
fn space_c_on_an_issues_text_copies_its_link() {
    let mut app = issue_app(issue_fixture());
    let r = app.rows.iter().position(|r| r.line.text.starts_with("labels")).expect("a facts line");
    app.goto(r, 0);
    assert!(app.cursor_ref().is_some_and(|u| u.contains("/issues/")), "{:?}", app.cursor_ref());
}

#[test]
fn a_reply_on_a_resolved_claude_comment_waits_for_an_answer() {
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    let dir = std::env::temp_dir().join(format!("dv-reply-resolved-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("pr-1.json");
    std::fs::write(&p, r#"{ "comments": [ { "id": "c1", "path": "src/a.rs", "line": 2, "body": "rename x", "code": ["    new_call(x);"], "resolved": true,
        "replies": [ { "author": "user", "body": "do it", "want": "change" }, { "author": "claude", "body": "Changed: renamed." } ] } ] }"#).unwrap();
    app.claude.path = Some(p.clone());
    crate::claude::reply(&p, "c1", "not right: it's still x in b.rs", "change").unwrap();
    app.load_claude();
    _ = std::fs::remove_dir_all(&dir);
    assert_eq!((app.threads[0].resolved, app.waiting_replies()), (false, 1));
}

#[test]
fn a_drafts_view_claims_its_runs_too() {
    // A replacement's or new PR's draft (no PR yet): its runs and panes go by its own key, so a
    // redraft (dv list's) sees one at work, as a PR's does.
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    let id = 980_000_000 + u64::from(std::process::id() % 1000);
    app.replacement = Some(crate::replace::Record { id, ..Default::default() });
    assert_eq!(app.run_key(), Some(crate::claude::rep_key(id)));
    let claim = crate::claude::claim(&app.run_key().unwrap(), "answering", None).unwrap().unwrap();
    assert!(crate::replace::Record { id, ..Default::default() }.at_work().is_some());
    drop(claim);
    assert!(crate::replace::Record { id, ..Default::default() }.at_work().is_none());
}

#[test]
fn the_diff_follows_a_rebase_github_doesnt_know_of_yet() {
    // Space g → rebase here: onto origin/main (b), fetched. GitHub's baseRefOid is still what the
    // PR was last pushed on (a): it can't know of a rebase not pushed yet.
    let dir = diff::scratch_repo("probe-rebase");
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
    app.pr.as_mut().unwrap().base_oid = a.clone();
    (app.root, app.base, app.full_base, app.follow_base) = (dir.clone(), a.clone(), a.clone(), Some("origin/main".into()));
    app.follow_rebase();
    assert_eq!(app.full_base, b, "the diff didn't follow the rebase");
    _ = std::fs::remove_dir_all(&dir);
}
