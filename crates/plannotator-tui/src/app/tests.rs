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
use unicode_width::UnicodeWidthStr;

use super::send::SendState;
use super::{App, Focus, GUTTER, Mode};
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

/// A delivery target that keeps what reached it, so a test can assert that nothing did.
///
/// `Discard` cannot tell "nothing was sent" from "a send was thrown away", which is the whole
/// question for a key that must deliver nothing.
#[derive(Debug, Default, Clone)]
struct Recording(std::sync::Arc<std::sync::Mutex<Vec<String>>>);

impl Delivery for Recording {
    fn describe(&self) -> String {
        "recording".to_owned()
    }

    fn deliver(&self, feedback: &str) -> Result<(), crate::delivery::DeliveryError> {
        if let Ok(mut sent) = self.0.lock() {
            sent.push(feedback.to_owned());
        }
        Ok(())
    }
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
    assert_eq!(app.focus, Focus::Document);
    // Nothing to focus while there are no annotations.
    app.handle_event(&key(KeyCode::Char('n'), KeyModifiers::NONE)).expect("n");
    assert_eq!(app.focus, Focus::Document, "an empty rail is not worth focusing");

    app.add_block_annotation(0, Kind::Comment, "note".to_owned()).expect("annotation");
    app.handle_event(&key(KeyCode::Char('n'), KeyModifiers::NONE)).expect("n");
    assert_eq!(app.focus, Focus::Rail);
    app.handle_event(&key(KeyCode::Char('n'), KeyModifiers::NONE)).expect("n");
    assert_eq!(app.focus, Focus::Document, "n comes back");
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

/// `q` drops the document from the presented set, which is what takes it out of `A`'s review.
///
/// Asserted on the review text rather than on the set's length, because dropping a tab from the
/// row while `A` still hands the document over would look like a working feature and tell the
/// agent the opposite of what the reviewer meant.
#[test]
fn closing_a_tab_drops_that_document_from_the_review_a_hands_over() {
    let root = folder("close-drops", 3);
    let mut app =
        App::open_folder(&root, 80, Box::new(Discard), RenderSettings::text_only()).expect("folder opens");
    app.data_dir = scratch_data_dir();
    app.open_doc(&root.join("f00.md")).expect("reopen under the scratch data dir");
    app.sync_doc_counts();

    app.handle_event(&key(KeyCode::Char('q'), KeyModifiers::NONE)).expect("q");

    assert_eq!(app.exit, super::Exit::Stay, "two documents are still worth reviewing");
    assert_eq!(open_path(&app), "f01.md", "the tab that took its place opened");
    let review = app.review_feedback().expect("the review A hands over");
    assert!(review.starts_with("# Review of 2 documents"), "{review}");
    assert!(review.contains("f01.md") && review.contains("f02.md"), "the rest is still reviewed: {review}");
    assert!(!review.contains("f00.md"), "the closed document is not part of the review: {review}");
    std::fs::remove_dir_all(&root).expect("cleanup");
}

/// Closing an annotated tab leaves real feedback unsent, so the status line says whose and how
/// much. Nothing is lost: annotations are written when they are made, and come back with the file.
#[test]
fn closing_an_annotated_tab_reports_the_annotations_it_left_unsent() {
    let (root, mut app) = set_app("close-annotated");
    app.add_block_annotation(0, Kind::Comment, "about one".to_owned()).expect("annotate one");

    app.handle_event(&key(KeyCode::Char('q'), KeyModifiers::NONE)).expect("q");

    let rows = draw_sized(&mut app, 160, 12);
    let footer = row(&rows, rows.len() - 1);
    assert!(footer.contains("closed one/doc.md"), "the closed file is named: {footer:?}");
    assert!(footer.contains("1 annotation(s) left unsent"), "the cost is stated: {footer:?}");
    // The store is the copy that survives: presenting the file again brings the note back.
    let closed = root.join("one/doc.md");
    let store = crate::store::Store::load(
        &crate::store::Location::for_file(&app.data_dir, &app.project, &closed),
        &crate::doc::Document::parse("# One\n\nalpha\n".to_owned()),
    )
    .expect("load the closed document's record");
    assert_eq!(store.len(), 1, "the annotation was dropped from disk, not just from the set");
}

/// The last tab has nothing left to review, so `q` there is `A`'s ending without the send.
#[test]
fn q_on_the_last_document_quits_and_closes_the_pane() {
    let (_root, mut set) = set_app("close-last");
    set.handle_event(&key(KeyCode::Char('q'), KeyModifiers::NONE)).expect("q");
    assert_eq!(set.exit, super::Exit::Stay, "one document is left");
    set.handle_event(&key(KeyCode::Char('q'), KeyModifiers::NONE)).expect("q again");
    assert_eq!(set.exit, super::Exit::QuitAndClosePane, "the set is empty: nothing is left to read");

    // A document presented on its own is the same situation with one fewer step: no other tab was
    // presented with it, so closing it leaves nothing either.
    let mut lone = app(Box::new(Discard));
    lone.handle_event(&key(KeyCode::Char('q'), KeyModifiers::NONE)).expect("q");
    assert_eq!(lone.exit, super::Exit::QuitAndClosePane);
}

/// ctrl+c is the escape hatch that leaves the pane behind, so Herdr finds it by label next time
/// and reuses it. It is the one exit that is not a verdict on the review.
#[test]
fn ctrl_c_leaves_the_reviewer_but_not_its_pane() {
    let (_root, mut app) = set_app("ctrl-c");
    app.handle_event(&key(KeyCode::Char('c'), KeyModifiers::CONTROL)).expect("ctrl+c");
    assert_eq!(app.exit, super::Exit::Quit, "the pane outlives the reviewer");
}

#[test]
fn a_closes_the_pane_only_when_the_review_actually_went_somewhere() {
    let mut app = app(Box::new(Discard));
    app.add_block_annotation(0, Kind::Comment, "ship it".to_owned()).expect("annotation");
    app.handle_event(&key(KeyCode::Char('A'), KeyModifiers::NONE)).expect("A");
    assert_eq!(app.send_state, SendState::Sent);
    assert_eq!(app.exit, super::Exit::QuitAndClosePane, "A dismisses the reviewer and its pane");
}

/// `Q` is the way out when the review is not wanted: it must reach the agent with nothing.
///
/// Asserted on the delivery seam and the archive rather than on `exit` alone, because a `Q` that
/// quits *and* hands the review over would pass any test that only watched the app leave.
#[test]
fn capital_q_quits_and_closes_the_pane_delivering_nothing() {
    let sent = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut app = app(Box::new(Recording(std::sync::Arc::clone(&sent))));
    app.add_block_annotation(0, Kind::Comment, "never mind".to_owned()).expect("annotation");

    app.handle_event(&key(KeyCode::Char('Q'), KeyModifiers::NONE)).expect("Q");

    assert_eq!(app.exit, super::Exit::QuitAndClosePane, "Q takes the pane with it, as A does");
    assert!(sent.lock().expect("the recorded sends").is_empty(), "Q delivered something");
    let index = app.data_dir.join("feedback").join(&app.project).join("index.jsonl");
    assert!(!index.exists(), "Q archived a submission");
    assert_eq!(app.send_count(), 1, "and the annotation is still pending, not cleared");
}

#[test]
fn a_refused_send_keeps_the_pane_open_so_its_status_can_be_read() {
    // A real agent target whose binary does not exist: delivery fails for a real reason.
    let mut app = app(agent());
    app.add_block_annotation(0, Kind::Comment, "ship it".to_owned()).expect("annotation");
    app.handle_event(&key(KeyCode::Char('A'), KeyModifiers::NONE)).expect("A");
    assert_eq!(app.exit, super::Exit::Stay, "nothing reached the agent, so both stay");
    let rows = draw(&mut app);
    let footer = row(&rows, rows.len() - 1);
    assert!(footer.contains("no agent to send to"), "the reason is on screen: {footer:?}");
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

#[test]
fn a_reports_a_clean_document_as_approved_without_inventing_an_annotation() {
    let (root, mut app) = set_app("clean-prose");
    // Only one of the two documents is annotated; the other is clean.
    app.add_block_annotation(0, Kind::Comment, "about one".to_owned()).expect("annotate one");

    let review = app.review_feedback().expect("review");
    assert!(review.starts_with("# Review of 2 documents"), "{review}");
    assert!(review.contains("## one/doc.md\n"), "the annotated document keeps its own heading: {review}");
    assert!(review.contains("about one"), "{review}");
    assert!(
        review.contains("## two/doc.md \u{2014} looks good, no changes requested"),
        "the clean document is approved in prose: {review}"
    );
    // Annotations under a document sit one level deeper than the document heading.
    assert!(review.contains("### Annotation 1 "), "{review}");

    app.handle_event(&key(KeyCode::Char('A'), KeyModifiers::NONE)).expect("A");
    assert_eq!(app.exit, super::Exit::QuitAndClosePane);
    // The approval was prose. Nothing may have been written against the clean document, or a
    // later --export would replay a note that was never made.
    let clean = root.join("two/doc.md");
    let store = crate::store::Store::load(
        &crate::store::Location::for_file(&app.data_dir, &app.project, &clean),
        &crate::doc::Document::parse("# Two\n\nbeta\n".to_owned()),
    )
    .expect("load the clean document's record");
    assert_eq!(store.len(), 0, "no annotation was invented for the approved document");
}

#[test]
fn a_hands_over_an_entirely_clean_set_where_e_has_nothing_to_send() {
    let (_root, mut app) = set_app("all-clean");
    assert_eq!(app.send_count(), 0, "nothing is annotated");

    // E has nothing to say, so it says nothing rather than sending "No annotations.".
    assert!(!app.send_feedback().expect("E"), "E does not deliver an empty review");
    assert_eq!(app.exit, super::Exit::Stay);

    // A still hands the set over: "I read all of it, it is fine" is the point of A.
    app.handle_event(&key(KeyCode::Char('A'), KeyModifiers::NONE)).expect("A");
    assert_eq!(app.exit, super::Exit::QuitAndClosePane, "A sends and closes even with nothing marked");
    assert_eq!(app.send_state, SendState::Sent);
}

#[test]
fn a_lone_clean_document_is_approved_by_name() {
    let app = app(Box::new(Discard));
    let review = app.review_feedback().expect("review");
    assert_eq!(review, "# Review of plan.md\n\nlooks good, no changes requested.\n");
}

#[test]
fn the_footer_names_the_document_by_path_not_just_its_file_name() {
    let (root, mut app) = set_app("footer-path");
    let rows = draw_sized(&mut app, 100, 12);
    let footer = row(&rows, rows.len() - 1);
    // The tab row already shows the bare name; the footer says which file on disk it is.
    assert!(footer.contains("one/doc.md"), "footer was {footer:?}");
    assert!(
        footer.contains(&root.display().to_string()) || footer.contains('\u{2026}'),
        "the path is shown in full or elided, never replaced by the name alone: {footer:?}"
    );
}

#[test]
fn a_document_with_no_path_falls_back_to_its_name() {
    // A reply or stdin has no file behind it, so there is nothing to spell out.
    let mut app = app(Box::new(Discard));
    let rows = draw(&mut app);
    let footer = row(&rows, rows.len() - 1);
    assert!(footer.contains("plan.md"), "footer was {footer:?}");
}

#[test]
fn the_keymap_overlay_opens_on_question_mark_and_closes_on_any_of_its_exits() {
    for closing in [KeyCode::Char('?'), KeyCode::Esc, KeyCode::Char('q')] {
        let mut app = app(Box::new(Discard));
        app.handle_event(&key(KeyCode::Char('?'), KeyModifiers::NONE)).expect("open");
        assert_eq!(app.mode, Mode::Help);
        let rows = draw(&mut app);
        let shown = rows.join("\n");
        assert!(shown.contains("send all, approve, close"), "A is described: {shown}");
        assert!(shown.contains("next document"), "Tab is described: {shown}");

        app.handle_event(&key(closing, KeyModifiers::NONE)).expect("close");
        assert_eq!(app.mode, Mode::Browse, "{closing:?} closes the list");
        // `q` closes the overlay rather than the app: quitting from a help screen would surprise.
        assert_eq!(app.exit, super::Exit::Stay, "{closing:?} left the app running");
    }
}

#[test]
fn a_pane_with_room_shows_every_group_and_a_cramped_one_says_what_it_hid() {
    let mut app = app(Box::new(Discard));
    app.handle_event(&key(KeyCode::Char('?'), KeyModifiers::NONE)).expect("open");

    // The reviewer's real pane is wide and tall: everything fits, so nothing is withheld.
    let roomy = draw_sized(&mut app, 120, 40).join("\n");
    for group in ["anywhere", "document", "selection", "notes"] {
        assert!(roomy.contains(group), "{group} is missing from a pane with room: {roomy}");
    }
    assert!(!roomy.contains("widen the pane"), "nothing was hidden, so nothing is claimed: {roomy}");

    // A pane too small for the table counts what it could not draw instead of dropping it silently.
    let cramped = draw_sized(&mut app, 80, 20).join("\n");
    assert!(cramped.contains("widen the pane"), "the shortfall is reported: {cramped}");
}

/// The status and the hint share the footer, and the hint must never cost the status columns.
///
/// Asserted through the rendered buffer at every width, because the old guard was a ceiling on the
/// hint string (`<= 60 columns`), which passed while the hint overwrote the status at 52, 64 and 80:
/// the layout gave the hint fixed columns and left the status whatever remained.
#[test]
fn the_footer_hint_never_takes_columns_from_the_status() {
    let mut app = app(Box::new(Discard));
    for width in 20..=200u16 {
        let rows = draw_sized(&mut app, width, 12);
        let footer = rows.last().expect("a footer row").trim_end().to_owned();
        let Some(at) = footer.find('?') else { continue };
        // Whenever the hint is on screen, the status before it is whole and separated by a gap.
        let before = footer.get(..at).expect("the status precedes the hint");
        assert!(before.ends_with(' '), "width {width}: the hint abuts the status: {footer:?}");
        assert!(
            before.contains("0 annotations"),
            "width {width}: the hint cost the status its counters: {footer:?}"
        );
        assert!(
            before.contains("block 1/"),
            "width {width}: the hint cut the status mid-counter: {footer:?}"
        );
    }
}

/// The status never claims more columns than the footer has.
///
/// Asserted by drawing every width and looking for the whole composed status on screen, not by
/// measuring any one field: the advisory the status appends below 80 columns made the status
/// itself longer than the pane, so the terminal cut it mid-word (`rail hidde` at 52 columns,
/// `widen to` at 64) while every field-level guard still held.
#[test]
fn the_footer_status_never_exceeds_the_room_it_has() {
    let (_root, mut app) = set_app("status-room");
    for width in 20..=200u16 {
        let rows = draw_sized(&mut app, width, 12);
        let footer = rows.last().expect("a footer row").clone();
        let status = app.footer_status(usize::from(width)).text;
        assert!(
            status.width() <= usize::from(width),
            "width {width}: the status claims {} columns: {status:?}",
            status.width()
        );
        assert!(footer.contains(&status), "width {width}: the status was cut: {footer:?} vs {status:?}");
        assert!(footer.contains("doc.md"), "width {width}: the document is not named: {footer:?}");
    }
}

/// A wide pane shows the whole hint; the shedding must not be one-way.
#[test]
fn even_a_wide_footer_advertises_only_the_overlay() {
    // Width is not a reason to restate the overlay. The keys live behind `?`, and the row is
    // worth more to the document's path than to a list nobody reads twice.
    let mut app = app(Box::new(Discard));
    let rows = draw_sized(&mut app, 200, 12);
    let footer = rows.last().expect("a footer row").clone();
    assert!(footer.contains("? keys"), "the overlay must stay discoverable: {footer:?}");
    for item in ["E send", "A close", "q close tab", "v select"] {
        assert!(!footer.contains(item), "{item} is behind ? now, not in the footer: {footer:?}");
    }
}

/// How a binding renders inside the overlay, so a test can look for it on screen.
fn binding_row(binding: &super::help::Binding) -> String {
    format!("{:<10}{}", binding.label, binding.what)
}

/// The shortfall the overlay's own title admits to, read back off the screen.
fn admitted_shortfall(screen: &str) -> usize {
    screen
        .split_once(" more,")
        .and_then(|(before, _)| before.rsplit(['\u{b7}', ' ']).find(|w| !w.is_empty())?.parse().ok())
        .unwrap_or(0)
}

/// Every binding is either on screen or counted in the title, at any pane size.
///
/// Bindings are lost two ways: columns past the pane's width are dropped, and rows past its height
/// are clipped by `Paragraph` silently. Only the first was counted, so a short pane quietly lost
/// bindings from the "anywhere" group while the overlay still implied it was complete.
#[test]
fn the_help_overlay_accounts_for_every_binding_it_cannot_show() {
    let mut app = app(Box::new(Discard));
    app.handle_event(&key(KeyCode::Char('?'), KeyModifiers::NONE)).expect("opens the overlay");
    for (width, height) in
        [(160u16, 40u16), (160, 16), (160, 12), (160, 10), (160, 8), (160, 6), (160, 5), (90, 40), (60, 40)]
    {
        let rows = draw_sized(&mut app, width, height);
        let screen = rows.join("\n");
        let shown = super::help::KEYS.iter().filter(|b| screen.contains(&binding_row(b))).count();
        let admitted = admitted_shortfall(&screen);
        assert_eq!(
            shown + admitted,
            super::help::KEYS.len(),
            "{width}x{height}: {shown} shown + {admitted} admitted != {} bindings\n{screen}",
            super::help::KEYS.len()
        );
    }
}

/// A pane too short to hold the table says so, rather than implying it is complete.
#[test]
fn a_short_pane_admits_the_overlay_is_cut() {
    let mut app = app(Box::new(Discard));
    app.handle_event(&key(KeyCode::Char('?'), KeyModifiers::NONE)).expect("opens the overlay");
    let short = draw_sized(&mut app, 160, 8).join("\n");
    assert!(admitted_shortfall(&short) > 0, "a short pane hides bindings silently:\n{short}");
    assert!(short.contains("lengthen the pane"), "the advice names the short dimension:\n{short}");

    let roomy = draw_sized(&mut app, 160, 40).join("\n");
    assert_eq!(admitted_shortfall(&roomy), 0, "nothing is hidden with room to spare:\n{roomy}");
}

/// A document with an annotation on it is never reported as approved.
///
/// `A` decides approval by set membership over `annotated_files`, which unions the on-disk records
/// with the set's in-memory counts. Any disagreement between those two lists about a path's form
/// makes an annotated document look clean, which is the worst thing this feature can get wrong: it
/// tells the agent its work was accepted while the objection sits unread on screen.
#[test]
fn a_document_with_an_annotation_is_never_reported_as_approved() {
    let (root, mut app) = set_app("never-approved");
    app.add_block_annotation(0, Kind::Comment, "this needs work".to_owned()).expect("annotation");

    let review = app.review_feedback().expect("composes the review");
    let set = app.docs.as_ref().expect("a set");
    let annotated =
        set.name_for(&root.join("one/doc.md")).expect("the annotated document is named").to_owned();
    let clean = set.name_for(&root.join("two/doc.md")).expect("the clean document is named").to_owned();

    for line in review.lines().filter(|l| l.starts_with("## ")) {
        if line.contains(&annotated) {
            assert!(!line.contains("looks good"), "the annotated document was approved: {line:?}\n{review}");
        }
    }
    assert!(review.contains("this needs work"), "the objection is in the review:\n{review}");
    assert!(
        review.lines().any(|l| l.starts_with("## ") && l.contains(&clean) && l.contains("looks good")),
        "the untouched document is still approved:\n{review}"
    );
}

/// One frame, as (symbol, foreground) for every row of screen column `x`.
fn column_of(app: &mut App, width: u16, height: u16, x: u16) -> Vec<(String, ratatui::style::Color)> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
    terminal.draw(|frame| app.draw(frame)).expect("draw");
    let buffer = terminal.backend().buffer();
    (0..buffer.area.height)
        .map(|y| {
            buffer.cell((x, y)).map_or_else(
                || (String::new(), ratatui::style::Color::Reset),
                |c| (c.symbol().to_owned(), c.fg),
            )
        })
        .collect()
}

/// One git command in `dir`, with an identity of its own so a developer's global config -
/// or the lack of one - cannot decide whether this test can commit.
fn git_in(dir: &std::path::Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=test", "-c", "user.email=test@example.com", "-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .expect("git runs");
    assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
}

/// A document opened from disk, with image art on so a block that becomes a picture is really
/// drawn as one. Mermaid stays off: it would need Node.
fn app_on(path: &std::path::Path, git: crate::config::GitConfig) -> App {
    let art = crate::config::ArtConfig {
        mermaid: crate::config::MermaidConfig { enabled: false, ..crate::config::MermaidConfig::default() },
        image: crate::config::ImageConfig::default(),
    };
    let render = RenderSettings { art, git, ..RenderSettings::text_only() };
    let mut app =
        App::open(super::read_file(path).expect("reads the document"), 80, Box::new(Discard), render)
            .expect("opens");
    app.data_dir = scratch_data_dir();
    app
}

/// A change inside a reflowed paragraph must still bar its row.
///
/// Asserted on the drawn buffer, not on `ChangeBar`, because the defect was neither in the parser
/// nor in the map: `draw.rs` asked about the row's FIRST mapped byte only, so a modified line was
/// invisible whenever an unchanged one happened to start the row. In prose that is most of a
/// paragraph. A test on the helper passes while the pane shows nothing.
#[test]
fn a_change_inside_a_wrapped_paragraph_still_bars_its_row() {
    let theme = crate::theme::Theme::default();
    let root = std::env::temp_dir().join(format!("plannotator-tui-reflow-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("mkdir");
    let doc = root.join("prose.md");
    // One paragraph of four short source lines: at 80 columns they reflow into a single row.
    std::fs::write(&doc, "# P\n\nalpha\nbeta\ngamma\ndelta\n").expect("write");
    git_in(&root, &["init", "-q"]);
    git_in(&root, &["add", "prose.md"]);
    git_in(&root, &["commit", "-q", "-m", "first"]);
    // Change only the THIRD line, so the row still starts on an unchanged one.
    std::fs::write(&doc, "# P\n\nalpha\nbeta\ngamma changed\ndelta\n").expect("edit");

    let mut app = app_on(&doc, crate::config::GitConfig::default());
    let signs = column_of(&mut app, 80, 12, 0);
    let rows = draw_sized(&mut app, 80, 12);
    let paragraph = rows
        .iter()
        .position(|row| row.contains("alpha") && row.contains("gamma changed"))
        .expect("the four lines reflowed into one row");

    let (symbol, colour) = signs.get(paragraph).expect("a sign cell for that row");
    assert_eq!(
        (symbol.as_str(), *colour),
        ("\u{2502}", theme.change_changed),
        "the row shows a changed line but was left unbarred"
    );
    std::fs::remove_dir_all(&root).expect("cleanup");
}

/// The change bar, end to end: real `git diff -U0 HEAD` output reaching the gutter's sign column.
///
/// The only test that builds a repository - every other change bar invariant is pure and lives in
/// `git.rs`. It is asserted on the drawn buffer rather than on the parsed hunks, because a bar
/// that is correct and drawn in the block marker's column is still a bug.
#[test]
fn the_change_bar_signs_the_gutters_first_column_from_what_git_reports() {
    let theme = crate::theme::Theme::default();
    let root = std::env::temp_dir().join(format!("plannotator-tui-repo-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("mkdir");
    // A space in the name, because the runner passes the path to git as one argument.
    let doc = root.join("a plan.md");
    let committed = "# Plan\n\nalpha\n\n![logo](logo.png)\n\nbeta\n";
    std::fs::write(&doc, committed).expect("write");
    let mut png = image::RgbaImage::new(4, 4);
    for pixel in png.pixels_mut() {
        *pixel = image::Rgba([10, 20, 30, 255]);
    }
    png.save(root.join("logo.png")).expect("writes png");
    git_in(&root, &["init", "-q"]);

    // A repository with no commit has no HEAD to measure against. That must degrade, never error:
    // nothing here is added, changed or deleted, because git was never able to say so.
    let fresh = column_of(&mut app_on(&doc, crate::config::GitConfig::default()), 80, 20, 0);
    for (row, (symbol, colour)) in fresh.iter().enumerate() {
        assert!(
            *colour != theme.change_added && *colour != theme.change_changed,
            "row {row}: an empty HEAD was read as a change: {symbol:?} in {colour:?}"
        );
    }

    git_in(&root, &["add", "a plan.md"]);
    git_in(&root, &["commit", "-q", "-m", "first"]);
    std::fs::write(&doc, "# Plan\n\nalpha changed\n\n![the logo](logo.png)\n\nbeta\n\ngamma\n")
        .expect("edit");

    let mut app = app_on(&doc, crate::config::GitConfig::default());
    let signs = column_of(&mut app, 80, 20, 0);
    let markers = column_of(&mut app, 80, 20, 1);
    // The body starts under the header and a blank row separates each block, so the screen rows
    // are: 1 `# Plan`, 3 `alpha changed`, 5-6 the image, 8 `beta`, 10 `gamma`.
    let sign_at = |y: usize| signs.get(y).map(|(symbol, colour)| (symbol.as_str(), *colour));
    assert_eq!(sign_at(3), Some(("│", theme.change_changed)), "the modified line");
    assert_eq!(sign_at(10), Some(("│", theme.change_added)), "the new line");
    assert_eq!(sign_at(1).map(|(symbol, _)| symbol), Some(" "), "an unchanged line gets no bar");
    assert_eq!(sign_at(8).map(|(symbol, _)| symbol), Some(" "), "an unchanged line gets no bar");
    // The image's rows came from no source byte at all, so they take their line from the block
    // they stand for - which is the line that changed.
    assert_eq!(sign_at(5), Some(("│", theme.change_changed)), "the first row of the picture");
    assert_eq!(sign_at(6), Some(("│", theme.change_changed)), "the last row of the picture");
    // The block marker moved to column 1, where it is closer to the text it marks.
    assert_eq!(markers.get(1).map(|(s, c)| (s.as_str(), *c)), Some(("▍", theme.accent)));
    assert_ne!(sign_at(1).map(|(symbol, _)| symbol), Some("▍"), "column 0 is the sign column now");

    // `r` measures again rather than keeping the bars it opened with: put the file back the way
    // HEAD has it and every bar goes.
    std::fs::write(&doc, committed).expect("revert");
    app.reload().expect("r reloads");
    let reloaded = column_of(&mut app, 80, 20, 0);
    assert!(
        reloaded.iter().all(|(symbol, _)| symbol == " "),
        "the document matches HEAD again, yet a bar survived the reload: {reloaded:?}"
    );

    // A file git has never been told about is untracked on every line, in its own colour, and is
    // never reported as added.
    let loose = root.join("untracked.md");
    std::fs::write(&loose, "# New\n\nfresh\n").expect("write");
    let untracked = column_of(&mut app_on(&loose, crate::config::GitConfig::default()), 80, 20, 0);
    for row in [1usize, 3] {
        assert_eq!(
            untracked.get(row).map(|(s, c)| (s.as_str(), *c)),
            Some(("│", theme.change_untracked)),
            "row {row} of an untracked file"
        );
    }

    // `[git] signs = false` reaches the gutter: the same file in the same repository, no bars.
    let mut off = app_on(&doc, crate::config::GitConfig { signs: false });
    let silent = column_of(&mut off, 80, 20, 0);
    assert!(
        silent.iter().all(|(symbol, _)| symbol == " "),
        "signs are off, yet the sign column was drawn: {silent:?}"
    );
    std::fs::remove_dir_all(&root).expect("cleanup");
}

#[test]
fn clicking_a_tab_opens_that_document() {
    let (_root, mut app) = set_app("click");
    let opened = |app: &App| match &app.open.source.provenance {
        Provenance::File { path } => path.clone(),
        _ => PathBuf::new(),
    };
    let first = opened(&app);

    // Draw so the tab spans are recorded, then click inside the SECOND tab's span. The span is
    // taken from geometry rather than guessed, because guessing a column would pass on an empty
    // tab row too.
    draw_sized(&mut app, 80, 12);
    let (span, index) = app.geometry.tabs.get(1).cloned().expect("a second tab was drawn");
    assert_eq!(index, 1, "spans are recorded in document order");
    app.handle_event(&click_at(span.start, 0)).expect("click the tab");

    assert_ne!(opened(&app), first, "the click opened the other document");
    assert_eq!(app.focus, Focus::Document, "and put focus back in the document");
}

#[test]
fn a_click_beside_the_tabs_opens_nothing() {
    let (_root, mut app) = set_app("click-miss");
    let opened = |app: &App| match &app.open.source.provenance {
        Provenance::File { path } => path.clone(),
        _ => PathBuf::new(),
    };
    let first = opened(&app);
    draw_sized(&mut app, 80, 12);
    let past = app.geometry.tabs.last().map_or(0, |(span, _)| span.end) + 2;
    app.handle_event(&click_at(past, 0)).expect("click past the last tab");
    assert_eq!(opened(&app), first, "empty space in the tab row is not a tab");
}
