//! Application state. Input handling lives in `input`, drawing in `draw`; this module owns
//! the data they share and the operations that change it.

mod compose;
mod draw;
mod header;
mod help;
mod input;

mod pick;
mod review;
mod selection;
mod send;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use plannotator_tui_schema::{DocumentSource, Kind, Provenance};
use ratatui::layout::Rect;

use crate::delivery::Delivery;
use crate::doc::Document;
use crate::docs::DocSet;
use crate::layout::DocLayout;
use crate::render::RenderSettings;
use crate::store::{Location, Store};
use crate::workspace_paths;
use selection::Selection;
use send::SendState;

/// Width of the marker column left of the document.
pub(super) const GUTTER: u16 = 2;

/// Toolbar items in display order: (glyph, label, key, kind).
const TOOLBAR: [(&str, &str, char, Kind); 3] = [
    ("👍", "looks good", 'a', Kind::LooksGood),
    ("💬", "comment", 'c', Kind::Comment),
    ("✗", "delete", 'd', Kind::Delete),
];

#[derive(Debug, PartialEq, Eq)]
enum Mode {
    Browse,
    /// Typing a comment for the pending selection.
    Compose,
    /// Editing the body of an existing annotation (by id).
    Edit(String),
    /// Quit was asked for while feedback is unsent; the footer asks first.
    ConfirmQuit,
    /// Choosing which of the agent's recent messages to review.
    Pick,
    /// The `?` keymap overlay.
    Help,
}

/// Which pane keyboard input goes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    Document,
    Rail,
}

/// Screen geometry captured during the last draw, for hit-testing input.
#[derive(Debug, Default, Clone)]
struct Geometry {
    doc: Rect,
    /// Toolbar rect and the column span of each item, in screen coordinates.
    toolbar: Option<(Rect, [Range<u16>; 3])>,
    /// Screen rects of the rail bubbles drawn last frame, with their annotation ids.
    bubbles: Vec<(Rect, String)>,
    /// The header's Send button; `None` when the header was too narrow for it.
    send_button: Option<Rect>,
    /// Picker rows drawn last frame, with their candidate index.
    pick_rows: Vec<(Rect, usize)>,
}

/// A finished selection waiting for an action.
#[derive(Debug, Clone)]
struct Pending {
    range: Range<usize>,
    /// Document (row, col) where the selection starts; anchors the toolbar and compose box.
    at: (usize, usize),
}

/// Everything about the open document; swapped wholesale when the tab row switches files.
#[derive(Debug)]
struct Open {
    source: DocumentSource,
    doc: Document,
    layout: DocLayout,
    store: Store,
}

impl Open {
    fn new(
        source: DocumentSource,
        width: usize,
        data_dir: &Path,
        project: &str,
        render: &RenderSettings,
    ) -> Result<Self> {
        let doc = Document::parse(source.content.clone());
        let file = match &source.provenance {
            Provenance::File { path } => Some(path.as_path()),
            _ => None,
        };
        let layout = DocLayout::build(&doc, width, &render.context(file));
        let store = match (&source.provenance, source.transient) {
            (Provenance::File { path }, false) => {
                Store::load(&Location::for_file(data_dir, project, path), &doc)?
            }
            _ => Store::transient(),
        };
        Ok(Self { source, doc, layout, store })
    }
}

use self::compose::Compose;

pub(crate) struct App {
    open: Open,
    /// How documents are rendered; fixed for the process, applied to every file opened.
    render: RenderSettings,
    /// Where annotations are stored and how this folder is named there.
    data_dir: PathBuf,
    project: String,
    /// The documents presented together, when more than one file is in play. `None` for a
    /// document opened on its own, or one that never came from a file (stdin, a reply).
    docs: Option<DocSet>,
    delivery: Box<dyn Delivery>,
    send_state: SendState,
    focus: Focus,
    scroll: usize,
    selected: usize,
    selection: Option<Selection>,
    pending: Option<Pending>,
    /// Keyboard cursor for visual selection, in document (row, col).
    cursor: (usize, usize),
    /// Index into the rail's placed annotations.
    rail_cursor: usize,
    mode: Mode,
    /// `last`: the agent's recent messages, newest first, and the picker's cursor.
    candidates: Vec<plannotator_tui_hosts::Message>,
    pick_cursor: usize,
    /// Minutes east of UTC used to draw message times. Pinned in tests so the picker
    /// renders the same on any machine.
    clock_offset: i32,
    /// The candidate currently on screen, and the one Esc goes back to.
    pick_open: usize,
    pick_return: usize,
    /// Documents already built for candidates. Previewing swaps `open`, and a reply
    /// review's annotations live only in memory, so the one being left is kept here
    /// rather than dropped.
    pick_cache: HashMap<usize, Open>,
    message_host: String,
    /// The transcript path, for the archive's `transcript`; never the session id.
    message_transcript: String,
    /// The host-assigned session id, for the archive's `session`; never a path.
    message_session: Option<String>,
    compose: Compose,
    /// Whether the terminal reports Shift+Enter distinctly (kitty keyboard protocol).
    pub(super) shift_enter: bool,
    /// The last primary-button press, for double-click detection.
    last_click: Option<(std::time::Instant, u16, u16)>,
    geometry: Geometry,
    status: Option<String>,
    frame_ms: f64,
    frame_max_ms: f64,
    /// Copy selections to the terminal clipboard (off for headless runs).
    pub(crate) clipboard: bool,
    pub(crate) exit: Exit,
}

/// Whether the reviewer is still running, and what becomes of its pane when it is not.
///
/// One value rather than a `quit` flag beside a `close_pane` flag, because closing the pane
/// without leaving is not a state that means anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Exit {
    #[default]
    Stay,
    /// Leave the reviewer; the pane it runs in is someone else's.
    Quit,
    /// `A`: leave, and take the Herdr pane with it.
    QuitAndClosePane,
}

impl std::fmt::Debug for App {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("App")
            .field("source", &self.open.source.name)
            .field("project", &self.project)
            .finish_non_exhaustive()
    }
}

impl App {
    pub(crate) fn open(
        source: DocumentSource,
        width: usize,
        delivery: Box<dyn Delivery>,
        render: RenderSettings,
    ) -> Result<Self> {
        let data_dir = workspace_paths::data_dir();
        let folder = match &source.provenance {
            Provenance::File { path } => path.parent().map_or_else(|| PathBuf::from("."), Path::to_path_buf),
            _ => std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        };
        let project = workspace_paths::project_name(&folder);
        let open = Open::new(source, width, &data_dir, &project, &render)?;
        let send_state = if open.store.all_delivered() { SendState::Sent } else { SendState::Ready };
        Ok(Self {
            open,
            render,
            data_dir,
            project,
            docs: None,
            delivery,
            send_state,
            focus: Focus::Document,
            scroll: 0,
            selected: 0,
            selection: None,
            pending: None,
            cursor: (0, 0),
            rail_cursor: 0,
            mode: Mode::Browse,
            candidates: Vec::new(),
            pick_cursor: 0,
            clock_offset: pick::local_offset_minutes(),
            pick_open: 0,
            pick_return: 0,
            pick_cache: HashMap::new(),
            message_host: String::new(),
            message_transcript: String::new(),
            message_session: None,
            compose: Compose::default(),
            shift_enter: false,
            last_click: None,
            geometry: Geometry::default(),
            status: None,
            frame_ms: 0.0,
            frame_max_ms: 0.0,
            clipboard: false,
            exit: Exit::Stay,
        })
    }

    /// Folder mode: every Markdown file beneath `root`, presented as one set.
    ///
    /// There is no tree to browse, so a folder with no Markdown is an error naming the folder
    /// rather than a placeholder document telling you to pick from a pane that no longer exists.
    pub(crate) fn open_folder(
        root: &Path,
        width: usize,
        delivery: Box<dyn Delivery>,
        render: RenderSettings,
    ) -> Result<Self> {
        let set = DocSet::of_folder(root)?;
        let files: Vec<PathBuf> = set.docs().iter().map(|d| d.path.clone()).collect();
        Self::open_files(&files, root, width, delivery, render)
    }

    /// Documents presented together, the first one open and the rest a `TAB` away.
    ///
    /// `root` is their common directory and names the project, so annotations are keyed the same
    /// way whether a file is reached from a set or opened on its own.
    pub(crate) fn open_files(
        files: &[PathBuf],
        root: &Path,
        width: usize,
        delivery: Box<dyn Delivery>,
        render: RenderSettings,
    ) -> Result<Self> {
        let mut set = DocSet::of_files(root, files);
        let first = set.current_path().context("no document to open")?.to_path_buf();
        let mut app = Self::open(read_file(&first)?, width, delivery, render)?;
        app.project = workspace_paths::project_name(root);
        app.open = Open::new(read_file(&first)?, width, &app.data_dir, &app.project, &app.render)?;
        set.focus(&first);
        app.docs = Some(set);
        app.sync_doc_counts();
        app.derive_send_state();
        Ok(app)
    }

    /// Open the next document in the set, wrapping. `TAB` walks documents; there is nothing else
    /// to walk now that the tree is gone.
    pub(crate) fn cycle_document(&mut self) -> Result<()> {
        let Some((index, path)) = self.docs.as_ref().and_then(DocSet::next) else { return Ok(()) };
        let total = self.docs.as_ref().map_or(0, DocSet::len);
        self.open_doc(&path)?;
        let name =
            path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
        self.status = Some(format!("{name} ({}/{total})", index + 1));
        Ok(())
    }

    /// Swap the open document for `path`, keeping the set pointed at it.
    fn open_doc(&mut self, path: &Path) -> Result<()> {
        let width = self.open.layout.width;
        self.open = Open::new(read_file(path)?, width, &self.data_dir, &self.project, &self.render)?;
        if let Some(set) = self.docs.as_mut() {
            set.focus(path);
        }
        self.derive_send_state();
        self.focus = Focus::Document;
        self.scroll = 0;
        self.selected = 0;
        self.cursor = (0, 0);
        self.rail_cursor = 0;
        self.clear_selection();
        Ok(())
    }

    /// Recount every document in the set from the records on disk.
    ///
    /// The counts are what the tab row shows and what `annotated_files` walks, so a send stops
    /// covering a file the moment they go stale.
    fn sync_doc_counts(&mut self) {
        let (data_dir, project) = (self.data_dir.clone(), self.project.clone());
        if let Some(set) = self.docs.as_mut() {
            set.set_counts(|path| Store::count_at(&Location::for_file(&data_dir, &project, path)));
        }
    }

    /// The open document's path, when it came from a file. The footer shows it in full;
    /// stdin and a reply have no path to show.
    fn document_path(&self) -> Option<PathBuf> {
        match &self.open.source.provenance {
            Provenance::File { path } => Some(path.clone()),
            _ => None,
        }
    }

    pub(crate) fn set_status(&mut self, status: String) {
        self.status = Some(status);
    }

    pub(crate) fn record_frame(&mut self, ms: f64) {
        self.frame_ms = if self.frame_ms == 0.0 { ms } else { self.frame_ms * 0.9 + ms * 0.1 };
        self.frame_max_ms = self.frame_max_ms.max(ms);
    }

    /// Annotate a source range: the rendered text is derived from the layout so the
    /// Workspaces web client can find it. Saved immediately.
    fn annotate(&mut self, range: Range<usize>, kind: Kind, body: String) -> Result<()> {
        let rendered = self.open.layout.rendered_in_range(&self.open.doc.source, &range);
        self.open.store.add(&self.open.doc, range, rendered, kind, body)?;
        self.mark_unsent();
        self.sync_doc_counts();
        Ok(())
    }

    /// Apply a toolbar action to the pending selection.
    fn act(&mut self, kind: Kind) -> Result<()> {
        let Some(pending) = self.pending.clone() else { return Ok(()) };
        match kind {
            Kind::Comment => {
                self.mode = Mode::Compose;
                self.compose = Compose::default();
            }
            Kind::LooksGood | Kind::Delete => {
                self.annotate(pending.range, kind, String::new())?;
                self.clear_selection();
                self.status = Some(format!("{} saved", label(kind)));
            }
        }
        Ok(())
    }

    /// Begin editing the body of the annotation under the rail cursor.
    fn edit_selected_annotation(&mut self) {
        let placed = self.open.store.placed();
        let Some(target) = placed.get(self.rail_cursor) else { return };
        self.compose = Compose::with_text(&target.annotation.body);
        self.mode = Mode::Edit(target.annotation.id.clone());
    }

    fn remove_selected_annotation(&mut self) -> Result<()> {
        let id = self.open.store.placed().get(self.rail_cursor).map(|p| p.annotation.id.clone());
        let Some(id) = id else { return Ok(()) };
        if self.open.store.remove(&id)? {
            self.mark_unsent();
            self.status = Some("annotation removed".into());
            self.rail_cursor = self.rail_cursor.min(self.open.store.placed().len().saturating_sub(1));
            self.sync_doc_counts();
        }
        Ok(())
    }

    fn is_open(&self, path: &Path) -> bool {
        matches!(&self.open.source.provenance, Provenance::File { path: p } if p == path)
    }

    fn clear_selection(&mut self) {
        self.selection = None;
        self.pending = None;
    }

    fn select_block(&mut self, block: usize) {
        if self.open.doc.blocks.is_empty() {
            return;
        }
        self.clear_selection();
        self.selected = block.min(self.open.doc.blocks.len() - 1);
        if let Some(rendered) = self.open.layout.blocks.get(self.selected) {
            self.cursor = (rendered.first_row, 0);
        }
        self.ensure_selected_visible();
    }

    fn ensure_selected_visible(&mut self) {
        let height = usize::from(self.geometry.doc.height.max(1));
        let Some(block) = self.open.layout.blocks.get(self.selected) else { return };
        let first = block.first_row;
        let last = first + block.rows.len().saturating_sub(1);
        if first < self.scroll {
            self.scroll = first.saturating_sub(1);
        } else if last >= self.scroll + height {
            self.scroll = (last + 2).saturating_sub(height).min(first);
        }
    }

    fn ensure_cursor_visible(&mut self) {
        let height = usize::from(self.geometry.doc.height.max(1));
        if self.cursor.0 < self.scroll {
            self.scroll = self.cursor.0;
        } else if self.cursor.0 >= self.scroll + height {
            self.scroll = self.cursor.0 + 1 - height;
        }
    }

    fn scroll_by(&mut self, delta: i64) {
        let height = usize::from(self.geometry.doc.height.max(1));
        let max = self.open.layout.total_rows.saturating_sub(height);
        self.scroll = (self.scroll as i64 + delta).clamp(0, max as i64) as usize;
    }

    /// Re-read the document from its provenance and re-resolve every annotation.
    fn reload(&mut self) -> Result<()> {
        let Provenance::File { path } = &self.open.source.provenance else {
            self.status = Some("not a file; nothing to reload".into());
            return Ok(());
        };
        let path = path.clone();
        self.open.source = read_file(&path)?;
        self.open.doc = Document::parse(self.open.source.content.clone());
        self.open.layout =
            DocLayout::build(&self.open.doc, self.open.layout.width, &self.render.context(Some(&path)));
        self.open.store.resolve_all(&self.open.doc);
        self.clear_selection();
        self.selected = self.selected.min(self.open.doc.blocks.len().saturating_sub(1));
        self.status = Some(format!("reloaded · {} orphaned", self.open.store.orphans()));
        Ok(())
    }

    // ----- headless helpers (bench, snapshot, scripting) -----------------------------

    /// Annotate a whole block by index.
    pub(crate) fn add_block_annotation(&mut self, block: usize, kind: Kind, body: String) -> Result<()> {
        let range = self.open.doc.blocks.get(block).map(|b| b.range.clone());
        let range = range.ok_or_else(|| {
            anyhow::anyhow!("block {block} out of range ({} blocks)", self.open.doc.blocks.len())
        })?;
        self.annotate(range, kind, body)
    }

    /// Annotate the first occurrence of `quote` in the source.
    pub(crate) fn add_quote_annotation(&mut self, quote: &str, kind: Kind, body: String) -> Result<()> {
        let start =
            self.open.doc.source.find(quote).ok_or_else(|| anyhow::anyhow!("quote not found: {quote:?}"))?;
        self.annotate(start..start + quote.len(), kind, body)
    }

    pub(crate) fn describe_blocks(&self) -> Vec<String> {
        self.open
            .doc
            .blocks
            .iter()
            .zip(&self.open.layout.blocks)
            .enumerate()
            .map(|(i, (block, rendered))| {
                let first = self.open.doc.block_text(i).lines().next().unwrap_or("");
                let head: String = first.chars().take(60).collect();
                format!("{i:4} {:<10} row {:>5}  {head}", format!("{:?}", block.kind), rendered.first_row)
            })
            .collect()
    }

    /// Scroll and select the first visible block.
    pub(crate) fn scroll_for_snapshot(&mut self, delta: i64) {
        self.scroll_by(delta);
        if let Some(block) = self.open.layout.block_at_row(self.scroll) {
            self.selected = block;
        }
    }

    /// Simulate a finished drag over the first occurrence of `quote`.
    pub(crate) fn select_quote_for_snapshot(&mut self, quote: &str) -> Result<()> {
        let start =
            self.open.doc.source.find(quote).ok_or_else(|| anyhow::anyhow!("quote not found: {quote:?}"))?;
        let range = start..start + quote.len();
        let cells = self.open.layout.blocks.iter().flat_map(|b| {
            b.rows.iter().enumerate().flat_map(move |(ri, row)| {
                row.cells.iter().enumerate().map(move |(col, cell)| ((b.first_row + ri, col), *cell))
            })
        });
        let hits: Vec<_> =
            cells.filter(|(_, cell)| cell.is_some_and(|o| range.contains(&o))).map(|(pos, _)| pos).collect();
        let first = *hits.first().ok_or_else(|| anyhow::anyhow!("quote is not rendered"))?;
        let last = hits.last().copied().unwrap_or(first);
        self.selection = Some(Selection::finished(first, last));
        self.finish_selection();
        Ok(())
    }
}

fn read_file(path: &Path) -> Result<DocumentSource> {
    let content = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(DocumentSource::file(PathBuf::from(path), content))
}

fn label(kind: Kind) -> &'static str {
    match kind {
        Kind::Comment => "comment",
        Kind::LooksGood => "looks good",
        Kind::Delete => "delete this",
    }
}

fn glyph(kind: Kind) -> &'static str {
    match kind {
        Kind::Comment => "💬",
        Kind::LooksGood => "👍",
        Kind::Delete => "✗",
    }
}
