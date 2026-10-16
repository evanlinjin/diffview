use super::*;

#[test]
fn keys_and_yank() {
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    let press = |app: &mut App, s: &str| s.chars().for_each(|c| app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)));
    press(&mut app, "n"); // first change: the deleted line
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
    app.pr = Some(gh::Pr { number: 1, node_id: "PR".into(), title: "t".into(), base: "main".into(), head_oid: "abc".into(), url: String::new() });
    let viewed = |p: &str, s: &str| HashMap::from([(p.to_owned(), s.to_owned())]);
    app.apply_viewed(viewed("src/a.rs", "VIEWED"));
    app.build();
    assert_eq!(app.viewed_tag(0).unwrap().0, "✓ Viewed ");
    let header = app.rows.iter().find(|r| r.line.kind == Kind::File && r.file == 0).unwrap();
    assert!(!header.line.text.contains("Viewed")); // drawn at the right edge, not part of the text
    assert!(app.collapsed.contains("src/a.rs")); // viewed → folded
    app.collapsed.clear(); // you unfold it…
    app.apply_viewed(viewed("src/a.rs", "VIEWED"));
    assert!(app.collapsed.is_empty()); // …and a refresh leaves it unfolded
    app.apply_viewed(viewed("src/a.rs", "DISMISSED"));
    app.build();
    assert_eq!(app.viewed_tag(0).unwrap().0, "↻ changed since viewed ");
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
}

#[test]
fn comments_placed_and_anchored() {
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    let c = |id, body: &str| gh::Comment {
        id, node_id: format!("C{id}"), can_edit: id == 2, can_delete: id == 2, author: "amy".into(), body: body.into(), created: "2026-10-01".into(), pending: false,
    };
    let t = |left, line, resolved, comments| Thread {
        id: format!("T{line:?}"),
        path: "src/a.rs".into(), left, line, start_line: None, original_line: Some(1), resolved, outdated: line.is_none(),
        hunk: vec![], comments, claude: None,
    };
    app.threads = vec![
        t(true, Some(2), false, vec![c(1, "why remove?"), c(2, "unused")]), // on the deleted line
        t(false, None, true, vec![c(3, "old note")]),                        // outdated + resolved
    ];
    app.build();
    let kinds: Vec<_> = app.rows.iter().take(10).map(|r| (r.line.kind, r.thread)).collect();
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
        (Kind::Add, None),
    ]);
    // Select from the deleted line down to the added line (across the thread): multi-line, mixed sides.
    app.goto(4, 0);
    app.select = true;
    app.goto(9, 0);
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
    app.pr = Some(gh::Pr { number: 1, node_id: "PR".into(), title: "t".into(), base: "main".into(), head_oid: "abc".into(), url: "https://github.com/o/r/pull/1".into() });
    let tr = app.rows.iter().position(|r| r.thread == Some(0)).unwrap();
    app.goto(tr, 0);
    let press = |app: &mut App, c| app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    press(&mut app, 'c');
    assert!(matches!(app.action.take(), Some(Action::Reply { review: true, ref thread, .. }) if thread == "TSome(2)"));
    // On the second comment ("unused"), the reply quotes that comment, not the first.
    let second = app.rows.iter().position(|r| r.line.text == "unused").unwrap();
    app.goto(second, 0);
    press(&mut app, 'c');
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
    assert_eq!(p.items[0].label, "src/a.rs:2  reply: unused"); // quote skipped
    let Target::Comment(ti, ci) = p.items[0].target else { panic!() };
    app.goto_comment(ti, ci);
    assert!(app.rows[app.head.row].line.text.starts_with("[pending] amy"));
    app.threads[0].comments[1].body = "unused".into();
    app.build();
    app.goto(first, 0);
    // Space t lists every thread with its state.
    press(&mut app, ' ');
    press(&mut app, 't');
    let labels: Vec<String> = app.picker.take().unwrap().items.into_iter().map(|i| i.label).collect();
    assert_eq!(labels[0], "src/a.rs:2  1 reply · [pending] @amy: why remove?");
    assert!(labels[1].contains("[resolved] [outdated] @amy: old note"));
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
    assert_eq!(app.picker.take().unwrap().items.len(), 3); // no pending review → no discard option
    app.review = Some("R".into());
    press(&mut app, 'S');
    assert_eq!(app.picker.as_ref().unwrap().items[3].label, "Discard pending review (1 comment(s))");
    assert_eq!((a.line, a.left, a.start), (2, false, Some((2, true))));
    assert_eq!(wrap("aaa bbb ccc", 7), ["aaa bbb", "ccc"]);
}

#[test]
fn quote_into_conversation() {
    let mut app = App::with("base".into(), diff::parse(diff::SAMPLE));
    app.pr = Some(gh::Pr { number: 1, node_id: "PR".into(), title: "t".into(), base: "main".into(), head_oid: "abc".into(), url: String::new() });
    let line = |heading, md, text: &str| gh::InfoLine { heading, md, text: text.into(), marks: vec![] };
    app.info = vec![line(true, false, "conversation (1)"), line(true, false, "@bo · 2d ago"), line(false, true, "Looks good\nbut fix CI")];
    app.build();
    let quoted = |app: &mut App, row: usize| {
        app.goto(row, 0);
        app.key(KeyEvent::new(KeyCode::Char('C'), KeyModifiers::NONE));
        match app.action.take() {
            Some(Action::Converse { quote }) => quote,
            _ => panic!("no conversation comment started"),
        }
    };
    let body = app.rows.iter().position(|r| r.line.text == "but fix CI").unwrap();
    assert_eq!(quoted(&mut app, body), "> Looks good\n> but fix CI\n\n");
    let author = app.rows.iter().position(|r| r.line.text == "@bo · 2d ago").unwrap();
    assert_eq!(quoted(&mut app, author), "> Looks good\n> but fix CI\n\n"); // the author line quotes its comment
    let heading = app.rows.iter().position(|r| r.line.text == "conversation (1)").unwrap();
    assert_eq!(quoted(&mut app, heading), ""); // other PR lines: an empty conversation comment
    // `c` has nothing to queue in the PR section, and says so without checking git state.
    app.goto(body, 0);
    app.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
    assert!(app.action.is_none());
    assert_eq!(app.msg, "select code lines to comment on");
    // A rebuild (as after posting) keeps the cursor on the same PR row.
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
    app.claude_path = Some(p.clone());
    app.load_threads();
    app.build();
    let texts: Vec<&str> = app.rows.iter().map(|r| r.line.text.as_str()).collect();
    assert_eq!(&texts[..3], ["Claude's review", "claude's review", "Looks fine."]); // summary up top, no PR needed
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
    assert!(matches!(app.action.take(), Some(Action::ClaudeReply(k)) if k == "c1")); // ids given on load
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
    app.claude_mtime = None;
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
    app.claude_path = Some(p.clone());
    app.load_threads();
    app.build();
    _ = std::fs::remove_file(&p);
    let (moved, gone) = (&app.threads[0], &app.threads[1]);
    assert_eq!((moved.line, moved.outdated), (Some(2), false));
    assert!(gone.outdated && !app.thread_open(1)); // folded, like GitHub's outdated threads
    let head = app.rows.iter().position(|r| r.thread == Some(1)).unwrap();
    assert!(app.rows[head].line.text.starts_with("[outdated] claude"));
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
    app.claude_path = Some(p.clone());
    app.load_threads();
    _ = std::fs::remove_file(&p);
    assert_eq!(app.waiting_replies(), 1); // resolved and answered ones don't count
    app.key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
    let p = app.picker.as_ref().unwrap();
    assert!(p.items[0].label.starts_with("Update Claude's review"));
    assert!(p.items[1].label.contains("(1 waiting)") && p.sel == 1); // answering preselected
    app.picker = None;
    // Space A lists Claude's comments, the unanswered one tagged.
    app.key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Char('A'), KeyModifiers::NONE));
    let labels: Vec<&str> = app.picker.as_ref().unwrap().items.iter().map(|i| i.label.as_str()).collect();
    assert_eq!(labels.len(), 3);
    assert!(labels[0].contains("[waiting]") && !labels[1].contains("[waiting]") && !labels[2].contains("[waiting]"));
    assert!(labels[0].starts_with("src/a.rs:2  1 reply · [waiting] @claude: a"), "{}", labels[0]);
    assert!(labels[2].ends_with("@claude: nit: use x (docs)"), "{}", labels[2]); // markdown stripped
    app.picker = None;
    app.threads.clear();
    app.run_claude(agent::Ask::Answer);
    assert!(app.msg.contains("no replies waiting") && app.claude_job.is_none());
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
