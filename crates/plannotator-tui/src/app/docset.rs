//! The presented set, from the app's side: building one, switching which of its documents is
//! open, and keeping the counts the tab row shows.
//!
//! `crate::docs::DocSet` is the set itself - names, collisions, the tab row. This is the half
//! that needs an `App`: opening a document means replacing `open`, pointing the set at it,
//! re-deriving whether feedback is unsent and putting the viewport back at the top, and those
//! move together or the tab row describes a document that is not on screen.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use plannotator_tui_schema::Provenance;

use super::{App, Focus, Open, read_file};
use crate::delivery::Delivery;
use crate::docs::DocSet;
use crate::render::RenderSettings;
use crate::store::{Location, Store};
use crate::workspace_paths;

impl App {
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

    /// Open the document at `index` in the set, for a click on its tab.
    ///
    /// Clicking the tab already open is not a no-op by accident: it re-reads nothing, but it does
    /// return focus to the document, which is what a click on a tab means.
    pub(crate) fn show_document(&mut self, index: usize) -> Result<()> {
        let Some(path) = self.docs.as_ref().and_then(|set| set.docs().get(index).map(|d| d.path.clone()))
        else {
            return Ok(());
        };
        let total = self.docs.as_ref().map_or(0, DocSet::len);
        if matches!(&self.open.source.provenance, Provenance::File { path: open } if *open == path) {
            self.focus = Focus::Document;
            return Ok(());
        }
        self.open_doc(&path)?;
        let name =
            path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
        self.status = Some(format!("{name} ({}/{total})", index + 1));
        Ok(())
    }

    /// Swap the open document for `path`, keeping the set pointed at it.
    pub(super) fn open_doc(&mut self, path: &Path) -> Result<()> {
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
    pub(super) fn sync_doc_counts(&mut self) {
        let (data_dir, project) = (self.data_dir.clone(), self.project.clone());
        if let Some(set) = self.docs.as_mut() {
            set.set_counts(|path| Store::count_at(&Location::for_file(&data_dir, &project, path)));
        }
    }

    /// Whether `path` is the document currently open, for a walk over the set that must not
    /// re-read the file it already has in memory.
    pub(super) fn is_open(&self, path: &Path) -> bool {
        matches!(&self.open.source.provenance, Provenance::File { path: p } if p == path)
    }
}
