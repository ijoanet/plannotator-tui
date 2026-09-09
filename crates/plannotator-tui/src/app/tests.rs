//! Behaviour of the header's Send button and the quit confirmation, drawn into a
//! `TestBackend` the way the `--snapshot` CLI does.

#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use plannotator_tui_schema::{DocumentSource, Kind, Provenance};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};

use super::send::SendState;
use super::{App, GUTTER, Mode};
use crate::delivery::{Delivery, Discard, HerdrAgent};
use crate::render::RenderSettings;

/// A fresh, empty data directory for one test. `App::open` resolves the real one, and a
/// successful send archives into it, so every app under test is pointed here instead:
/// nothing a test does may reach the developer's own Plannotator data.
fn scratch_data_dir() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("plannotator-tui-app-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch data dir");
    dir
}

/// A transient source: the app runs exactly as it does on a file, but nothing is written
/// to the Plannotator data directory.
fn app(delivery: Box<dyn Delivery>) -> App {
    let source =
        DocumentSource::new("# Plan\n\nfirst thing\n".to_owned(), "plan.md", true, Provenance::Stdin);
    let mut app = App::open(source, 60, delivery, RenderSettings::text_only()).expect("app opens");
    app.data_dir = scratch_data_dir();
    app
}

/// `App::open_message` on `candidates()`, isolated like `app`.
fn message_app(session_id: Option<&str>, delivery: Box<dyn Delivery>) -> App {
    let mut app = App::open_message(
        "claude",
        "/tmp/transcript.jsonl",
        session_id,
        candidates(),
        60,
        delivery,
        RenderSettings::text_only(),
    )
    .expect("opens");
    app.data_dir = scratch_data_dir();
    app
}

/// Send the open message review through `Discard` and return the archive's one record.
fn archived_message_review(app: &mut App) -> serde_json::Value {
    app.handle_event(&Event::Key(KeyEvent::from(KeyCode::Esc))).expect("esc");
    app.add_block_annotation(0, Kind::Comment, "x".to_owned()).expect("annotation");
    app.send_feedback().expect("send");
    assert_eq!(app.send_state, SendState::Sent);
    let index = app.data_dir.join("feedback").join(&app.project).join("index.jsonl");
    let text = std::fs::read_to_string(&index).expect("index written under the test's data dir");
    serde_json::from_str(text.trim()).expect("one json record")
}

#[test]
fn a_message_review_archives_the_session_id_and_the_transcript_path_separately() {
    let id = "01a04583-a848-7b21-a890-f3ed0c9fef05";
    let mut app = message_app(Some(id), Box::new(Discard));
    let record = archived_message_review(&mut app);
    assert_eq!(record["surface"], "annotate-last");
    assert_eq!(record["target"]["agent"]["host"], "claude-code");
    assert_eq!(record["target"]["agent"]["session"], id);
    assert_eq!(record["target"]["agent"]["transcript"], "/tmp/transcript.jsonl");
}

#[test]
fn a_message_review_without_a_session_id_archives_only_the_transcript_path() {
    let mut app = message_app(None, Box::new(Discard));
    let record = archived_message_review(&mut app);
    assert!(record["target"]["agent"].get("session").is_none(), "no id means no session, never the path");
    assert_eq!(record["target"]["agent"]["transcript"], "/tmp/transcript.jsonl");
}

fn agent() -> Box<dyn Delivery> {
    Box::new(HerdrAgent::new(
        PathBuf::from("/nonexistent/herdr"),
        "w1:p1".into(),
        Some("claude".into()),
        None,
    ))
}

/// One frame, as one string per screen row.
fn draw(app: &mut App) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(80, 20)).expect("terminal");
    terminal.draw(|frame| app.draw(frame)).expect("draw");
    let buffer = terminal.backend().buffer();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .filter_map(|x| buffer.cell((x, y)))
                .map(|c| c.symbol().to_owned())
                .collect()
        })
        .collect()
}

fn row(rows: &[String], index: usize) -> &str {
    rows.get(index).map_or("", String::as_str)
}

#[test]
fn the_header_draws_the_send_button_and_records_where_it_is() {
    let mut app = app(agent());
    app.add_block_annotation(0, Kind::Comment, "x".to_owned()).expect("annotation");
    let rows = draw(&mut app);
    let header = row(&rows, 0);
    assert!(header.contains("Send 1 to claude in w1:p1 ▸"), "header was {header:?}");
    let rect = app.geometry.send_button.expect("button rect recorded");
    assert_eq!(rect.y, 0);
    assert_eq!(rect.right(), 80, "the button sits on the right edge");
}

#[test]
fn clicking_the_send_button_sends() {
    let mut app = app(Box::new(Discard));
    app.add_block_annotation(0, Kind::Comment, "x".to_owned()).expect("annotation");
    draw(&mut app);
    let rect = app.geometry.send_button.expect("button rect recorded");
    let click = Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: rect.x + rect.width / 2,
        row: rect.y,
        modifiers: KeyModifiers::NONE,
    });
    app.handle_event(&click).expect("click");
    assert_eq!(app.send_state, SendState::Sent);
    let index = app.data_dir.join("feedback").join(&app.project).join("index.jsonl");
    assert!(index.is_file(), "the send was archived under the test's own data dir");
}

#[test]
fn quitting_with_unsent_feedback_asks_before_it_quits() {
    let mut app = app(agent());
    app.add_block_annotation(0, Kind::Comment, "x".to_owned()).expect("annotation");
    app.handle_event(&Event::Key(KeyEvent::from(KeyCode::Char('q')))).expect("q");
    assert_eq!(app.mode, Mode::ConfirmQuit);
    assert!(!app.quit, "the question is asked instead of quitting");
    let rows = draw(&mut app);
    let footer = row(&rows, 19);
    assert!(footer.contains("before quitting? y send · n quit · esc cancel"), "footer was {footer:?}");
    app.handle_event(&Event::Key(KeyEvent::from(KeyCode::Char('n')))).expect("n");
    assert!(app.quit, "n quits without sending");
    assert_eq!(app.send_state, SendState::Ready, "nothing was sent");
}

fn candidates() -> Vec<plannotator_tui_hosts::Message> {
    use plannotator_tui_hosts::{Message, Role};
    let message = |id: &str, text: &str, at: &str| Message {
        id: id.to_owned(),
        role: Role::Assistant,
        text: text.to_owned(),
        at: Some(at.to_owned()),
    };
    vec![
        message("m3", "# Third\n\nnewest message\n", "2026-08-28T12:41:00.000Z"),
        message("m2", "# Second\n\nmiddle message\n", "2026-08-28T12:38:00.000Z"),
        message("m1", "# First\n\noldest message\n", "2026-08-28T12:30:00.000Z"),
    ]
}

#[test]
fn the_picker_lists_newest_first_and_opens_the_chosen_message() {
    let mut app = message_app(None, Box::new(Discard));
    app.clock_offset = 0;
    assert_eq!(app.mode, Mode::Pick, "more than one candidate asks which");
    let rows = draw(&mut app);
    let listed: Vec<&str> = rows.iter().map(String::as_str).filter(|r| r.contains("12:")).collect();
    assert_eq!(listed.len(), 3, "{rows:?}");
    assert!(listed[0].contains("12:41  # Third"), "{:?}", listed[0]);
    assert!(listed[2].contains("12:30  # First"), "{:?}", listed[2]);

    app.handle_event(&Event::Key(KeyEvent::from(KeyCode::Char('j')))).expect("j");
    app.handle_event(&Event::Key(KeyEvent::from(KeyCode::Enter))).expect("enter");
    assert_eq!(app.mode, Mode::Browse);
    assert_eq!(app.open.doc.source, "# Second\n\nmiddle message\n");
    assert!(app.open.store.is_transient(), "a message is never written to disk");
    app.add_block_annotation(0, Kind::Comment, "x".to_owned()).expect("annotate");
    assert!(app.open.store.is_transient());
}

#[test]
fn a_status_leads_the_footer_so_a_narrow_pane_cannot_truncate_it_away() {
    let mut app = message_app(None, Box::new(Discard));
    app.handle_event(&Event::Key(KeyEvent::from(KeyCode::Esc))).expect("esc");
    app.set_status("no session id from Herdr, showing the newest transcript for this folder".to_owned());
    let rows = draw(&mut app);
    let footer = row(&rows, rows.len() - 1);
    assert!(footer.trim_start().starts_with("no session id from Herdr"), "footer was {footer:?}");
}

#[test]
fn escaping_the_picker_keeps_the_newest_message() {
    let mut app = message_app(None, Box::new(Discard));
    app.handle_event(&Event::Key(KeyEvent::from(KeyCode::Esc))).expect("esc");
    assert_eq!(app.mode, Mode::Browse);
    assert_eq!(app.open.doc.source, "# Third\n\nnewest message\n");
    assert_eq!(app.open.source.name, "claude · last message");
    app.handle_event(&Event::Key(KeyEvent::from(KeyCode::Char('p')))).expect("p");
    assert_eq!(app.mode, Mode::Pick, "p reopens the picker");
}

#[test]
fn moving_the_picker_cursor_previews_that_message() {
    let mut app = message_app(None, Box::new(Discard));
    assert_eq!(app.open.doc.source, "# Third\n\nnewest message\n", "the newest opens behind the picker");

    app.handle_event(&Event::Key(KeyEvent::from(KeyCode::Char('j')))).expect("j");

    assert_eq!(app.mode, Mode::Pick, "previewing does not leave the picker");
    assert_eq!(app.open.doc.source, "# Second\n\nmiddle message\n", "the document follows the cursor");
}

#[test]
fn previewing_away_and_back_keeps_annotations() {
    let mut app = message_app(None, Box::new(Discard));
    app.handle_event(&Event::Key(KeyEvent::from(KeyCode::Esc))).expect("esc");
    app.add_block_annotation(0, Kind::Comment, "keep me".to_owned()).expect("annotate");
    assert_eq!(app.open.store.placed().len(), 1);

    app.handle_event(&Event::Key(KeyEvent::from(KeyCode::Char('p')))).expect("p");
    app.handle_event(&Event::Key(KeyEvent::from(KeyCode::Char('j')))).expect("j");
    app.handle_event(&Event::Key(KeyEvent::from(KeyCode::Char('k')))).expect("k");

    assert_eq!(app.open.doc.source, "# Third\n\nnewest message\n", "back where we started");
    assert_eq!(app.open.store.placed().len(), 1, "a reply review only holds annotations in memory");
}

/// A folder of `count` Markdown files named `f00.md`, `f01.md`, … in a fresh temp dir.
///
/// `name` keeps callers apart: tests run in parallel, and a shared directory has them deleting
/// each other's files.
fn folder(name: &str, count: usize) -> PathBuf {
    let root = std::env::temp_dir().join(format!("plannotator-tui-folder-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("mkdir");
    for i in 0..count {
        std::fs::write(root.join(format!("f{i:02}.md")), format!("# File {i}\n")).expect("write");
    }
    root
}

/// One frame at `width` × `height`, as one string per screen row.
fn draw_sized(app: &mut App, width: u16, height: u16) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
    terminal.draw(|frame| app.draw(frame)).expect("draw");
    let buffer = terminal.backend().buffer();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .filter_map(|x| buffer.cell((x, y)))
                .map(|c| c.symbol().to_owned())
                .collect()
        })
        .collect()
}

fn open_path(app: &App) -> String {
    match &app.open.source.provenance {
        Provenance::File { path } => path.file_name().expect("name").to_string_lossy().into_owned(),
        _ => String::new(),
    }
}

#[test]
fn a_folder_argument_becomes_a_set_of_its_markdown_files() {
    let root = folder("as-set", 3);
    let mut app =
        App::open_folder(&root, 100, Box::new(Discard), RenderSettings::text_only()).expect("folder opens");
    app.data_dir = scratch_data_dir();
    let set = app.docs.as_ref().expect("a set");
    let names: Vec<&str> = set.docs().iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, ["f00.md", "f01.md", "f02.md"]);
    assert_eq!(open_path(&app), "f00.md", "the first document opens");
    std::fs::remove_dir_all(&root).expect("cleanup");
}

#[test]
fn a_folder_with_no_markdown_says_so_instead_of_opening_empty() {
    let root = std::env::temp_dir().join(format!("plannotator-tui-nomd-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("mkdir");
    std::fs::write(root.join("notes.txt"), "not markdown").expect("write");
    // With no tree to browse, a placeholder document would tell you to use a pane that is gone.
    let err = App::open_folder(&root, 100, Box::new(Discard), RenderSettings::text_only())
        .expect_err("nothing to review");
    assert!(err.to_string().contains(&root.display().to_string()), "{err}");
    std::fs::remove_dir_all(&root).expect("cleanup");
}

fn key(code: KeyCode, modifiers: KeyModifiers) -> Event {
    Event::Key(KeyEvent::new(code, modifiers))
}

fn click_at(column: u16, row: u16) -> Event {
    Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::NONE,
    })
}

#[test]
fn double_clicking_a_block_offers_the_toolbar_for_the_whole_block() {
    let mut app = app(Box::new(Discard));
    draw(&mut app);
    // "first thing" is the second block; single click starts a selection, not a toolbar.
    let (col, row) = (5, 3);
    app.handle_event(&click_at(col, row)).expect("first click");
    let rows = draw(&mut app);
    assert!(!rows.iter().any(|r| r.contains("looks good")), "no toolbar after one click");
    app.handle_event(&click_at(col, row)).expect("second click");
    let rows = draw(&mut app);
    assert!(rows.iter().any(|r| r.contains("looks good")), "toolbar after a double-click: {rows:?}");
    // The toolbar acts on the whole block: the 'a' key approves it.
    let placed_before = app.open.store.placed().len();
    app.handle_event(&key(KeyCode::Char('a'), KeyModifiers::NONE)).expect("approve");
    assert_eq!(app.open.store.placed().len(), placed_before + 1);
    let rows = draw(&mut app);
    assert!(rows.iter().any(|r| r.contains("👍")), "rail shows the approval: {rows:?}");
}

#[test]
fn a_comment_can_span_lines_and_enter_saves_it() {
    let mut app = app(Box::new(Discard));
    draw(&mut app);
    let (col, row) = (5, 3);
    app.handle_event(&click_at(col, row)).expect("click");
    app.handle_event(&click_at(col, row)).expect("double click");
    app.handle_event(&key(KeyCode::Char('c'), KeyModifiers::NONE)).expect("open compose");
    for c in "first line".chars() {
        app.handle_event(&key(KeyCode::Char(c), KeyModifiers::NONE)).expect("type");
    }
    // Shift+Enter and Alt+Enter both insert a newline; Ctrl+J too.
    app.handle_event(&key(KeyCode::Enter, KeyModifiers::SHIFT)).expect("shift+enter");
    for c in "second".chars() {
        app.handle_event(&key(KeyCode::Char(c), KeyModifiers::NONE)).expect("type");
    }
    app.handle_event(&key(KeyCode::Enter, KeyModifiers::ALT)).expect("alt+enter");
    for c in "third".chars() {
        app.handle_event(&key(KeyCode::Char(c), KeyModifiers::NONE)).expect("type");
    }
    let rows = draw(&mut app);
    assert!(rows.iter().any(|r| r.contains("first line")), "compose shows line one: {rows:?}");
    assert!(rows.iter().any(|r| r.contains("second")), "compose shows line two");
    assert!(rows.iter().any(|r| r.contains("alt+enter new line")), "hint shows the fallback key");
    app.handle_event(&key(KeyCode::Enter, KeyModifiers::NONE)).expect("save");
    let placed = app.open.store.placed();
    assert_eq!(placed.last().expect("annotation").annotation.body, "first line\nsecond\nthird");
}

#[test]
fn pasting_into_the_comment_box_keeps_newlines() {
    let mut app = app(Box::new(Discard));
    draw(&mut app);
    app.handle_event(&click_at(5, 3)).expect("click");
    app.handle_event(&click_at(5, 3)).expect("double click");
    app.handle_event(&key(KeyCode::Char('c'), KeyModifiers::NONE)).expect("compose");
    app.handle_event(&Event::Paste("pasted one\r\npasted two".to_owned())).expect("paste");
    app.handle_event(&key(KeyCode::Enter, KeyModifiers::NONE)).expect("save");
    let placed = app.open.store.placed();
    assert_eq!(placed.last().expect("annotation").annotation.body, "pasted one\npasted two");
}

#[test]
fn the_rail_costs_no_width_until_an_annotation_exists() {
    let mut app = app(Box::new(Discard));
    // 110 columns: wide enough for the rail to be eligible, too narrow for the tree.
    draw_sized(&mut app, 110, 20);
    let unmarked = app.open.layout.width;
    assert_eq!(unmarked, 110 - usize::from(GUTTER), "an unmarked document uses the whole pane");

    app.add_block_annotation(0, Kind::Comment, "note".to_owned()).expect("annotates");
    draw_sized(&mut app, 110, 20);
    let marked = app.open.layout.width;
    assert!(marked < unmarked, "the rail appears with the first annotation: {marked} < {unmarked}");
    assert!(marked >= 20, "the document keeps its minimum width");
}

/// Two documents in different directories, presented together. Each caller gets its own root:
/// tests run in parallel, so a shared one has them deleting each other's fixtures.
///
/// The document is reopened after the scratch data directory is set, because `App::open_files`
/// builds the first store against the real one; without that, this file's annotations would land
/// in the developer's own Plannotator data and the set's counts would disagree with the records.
fn set_app(name: &str) -> (PathBuf, App) {
    let root = std::env::temp_dir().join(format!("plannotator-tui-set-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("one")).expect("dir");
    std::fs::create_dir_all(root.join("two")).expect("dir");
    std::fs::write(root.join("one/doc.md"), "# One\n\nalpha\n").expect("write");
    std::fs::write(root.join("two/doc.md"), "# Two\n\nbeta\n").expect("write");
    let files = vec![root.join("one/doc.md"), root.join("two/doc.md")];
    let mut app = App::open_files(&files, &root, 80, Box::new(Discard), RenderSettings::text_only())
        .expect("opens the set");
    app.data_dir = scratch_data_dir();
    app.open_doc(&root.join("one/doc.md")).expect("reopen under the scratch data dir");
    app.sync_doc_counts();
    (root, app)
}

/// The open document's path, which distinguishes two files that share a basename.
fn opened(app: &App) -> PathBuf {
    match &app.open.source.provenance {
        Provenance::File { path } => path.clone(),
        _ => PathBuf::new(),
    }
}

#[test]
fn a_presented_set_lists_exactly_its_documents_by_relative_name() {
    let (_root, app) = set_app("lists");
    let set = app.docs.as_ref().expect("a set");
    let names: Vec<&str> = set.docs().iter().map(|d| d.name.as_str()).collect();
    // Same basename in both directories: the relative path keeps them apart.
    assert_eq!(names, ["one/doc.md", "two/doc.md"]);
}

#[test]
fn tab_walks_the_documents_and_wraps() {
    let (_root, mut app) = set_app("walks");
    let first = opened(&app);
    app.handle_event(&key(KeyCode::Tab, KeyModifiers::NONE)).expect("cycles");
    let second = opened(&app);
    assert_ne!(first, second, "tab opened the other document");
    assert_eq!(app.docs.as_ref().map(crate::docs::DocSet::current), Some(1), "the tab row followed");
    app.handle_event(&key(KeyCode::Tab, KeyModifiers::NONE)).expect("cycles");
    assert_eq!(opened(&app), first, "cycling wraps back to the first");
}

#[test]
fn one_document_shows_no_tab_row_and_a_set_shows_one() {
    let mut single = app(Box::new(Discard));
    let rows = draw_sized(&mut single, 80, 12);
    assert!(!row(&rows, 0).contains("plan.md"), "a lone document spends no row on tabs: {:?}", row(&rows, 0));

    let (_root, mut set) = set_app("tabrow");
    let rows = draw_sized(&mut set, 80, 12);
    let tabs = row(&rows, 0);
    assert!(tabs.contains("one/doc.md"), "tab row was {tabs:?}");
    assert!(tabs.contains("two/doc.md"), "tab row was {tabs:?}");
}

#[test]
fn n_moves_focus_between_the_document_and_its_notes() {
    let mut app = app(Box::new(Discard));
    assert_eq!(app.focus, super::Focus::Document);
    // Nothing to focus while there are no annotations.
    app.handle_event(&key(KeyCode::Char('n'), KeyModifiers::NONE)).expect("n");
    assert_eq!(app.focus, super::Focus::Document, "an empty rail is not worth focusing");

    app.add_block_annotation(0, Kind::Comment, "note".to_owned()).expect("annotation");
    app.handle_event(&key(KeyCode::Char('n'), KeyModifiers::NONE)).expect("n");
    assert_eq!(app.focus, super::Focus::Rail);
    app.handle_event(&key(KeyCode::Char('n'), KeyModifiers::NONE)).expect("n");
    assert_eq!(app.focus, super::Focus::Document, "n comes back");
}

#[test]
fn a_send_covers_every_document_in_the_set_not_just_the_open_one() {
    let (_root, mut app) = set_app("coverage");
    app.add_block_annotation(0, Kind::Comment, "about one".to_owned()).expect("annotate one");
    app.handle_event(&key(KeyCode::Tab, KeyModifiers::NONE)).expect("tab");
    app.add_block_annotation(0, Kind::Comment, "about two".to_owned()).expect("annotate two");

    // The count is the set's, which is what the Send button promises and what a send must deliver.
    assert_eq!(app.send_count(), 2, "both documents count towards the send");
    let feedback = app.set_feedback().expect("feedback for the set");
    assert!(feedback.contains("about one"), "the document not on screen is still sent: {feedback}");
    assert!(feedback.contains("about two"), "{feedback}");
    assert!(feedback.contains("one/doc.md") && feedback.contains("two/doc.md"), "{feedback}");
}

#[test]
fn cycling_one_document_is_a_no_op() {
    let mut app = app(Box::new(Discard));
    let before = app.open.doc.source.clone();
    app.cycle_document().expect("no-op");
    assert_eq!(app.open.doc.source, before);
}

#[test]
fn a_sent_and_archived_review_is_cleared_so_the_next_send_repeats_nothing() {
    let mut app = app(Box::new(Discard));
    app.add_block_annotation(0, Kind::Comment, "fix this".to_owned()).expect("annotation");
    assert_eq!(app.send_count(), 1);

    app.send_feedback().expect("sends");
    assert_eq!(app.send_state, SendState::Sent);
    assert!(app.data_dir.join("feedback").join(&app.project).join("index.jsonl").is_file());
    assert_eq!(app.send_count(), 0, "the review was handed over, so it is no longer pending");
    assert!(app.feedback().is_empty() || !app.feedback().contains("fix this"), "and not re-sent");
}

#[test]
fn a_review_that_could_not_be_archived_is_kept() {
    let mut app = app(Box::new(Discard));
    // The archive is the durable copy; with it off, the store is the only record and must stay.
    std::fs::write(app.data_dir.join("config.json"), r#"{"feedbackHistory": false}"#).expect("archive off");
    app.add_block_annotation(0, Kind::Comment, "keep me".to_owned()).expect("annotation");
    app.send_feedback().expect("sends");
    assert_eq!(app.send_count(), 1, "nothing archived it, so nothing may clear it");
    assert!(app.feedback().contains("keep me"));
}

#[test]
fn clearing_on_send_can_be_turned_off() {
    let source =
        DocumentSource::new("# Plan\n\nfirst thing\n".to_owned(), "plan.md", true, Provenance::Stdin);
    let render = RenderSettings {
        review: crate::config::ReviewConfig { clear_on_send: false },
        ..RenderSettings::text_only()
    };
    let mut app = App::open(source, 60, Box::new(Discard), render).expect("app opens");
    app.data_dir = scratch_data_dir();
    app.add_block_annotation(0, Kind::Comment, "stay".to_owned()).expect("annotation");
    app.send_feedback().expect("sends");
    assert_eq!(app.send_count(), 1, "the annotation stays when the flag is off");
}

#[test]
fn a_narrow_tab_row_keeps_the_open_document_and_counts_the_rest() {
    let root = folder("narrow-tabs", 30);
    let mut app =
        App::open_folder(&root, 100, Box::new(Discard), RenderSettings::text_only()).expect("folder opens");
    app.data_dir = scratch_data_dir();
    let rows = draw_sized(&mut app, 80, 12);
    let tabs = row(&rows, 0);
    assert!(tabs.contains("f00.md"), "the open document is on the row: {tabs:?}");
    // Thirty tabs cannot fit in eighty columns, so the remainder is a count at the edge.
    assert!(tabs.contains('\u{203a}'), "hidden tabs are counted: {tabs:?}");
    assert_eq!(tabs.chars().count(), 80, "the row is exactly the pane's width");
    std::fs::remove_dir_all(&root).expect("cleanup");
}
