//! Composing the review an agent receives: one document's feedback, a set's, and the whole
//! hand-over `A` sends.
//!
//! Split from `app/mod.rs`, which had grown past 300 lines while a module was being deleted from
//! it. Nothing here mutates the app: every function reads the open document, the set and the
//! store, and returns text. That is the seam - the state changes live with sending.

use std::fmt::Write as _;
use std::path::Path;

use anyhow::Result;

use super::{App, Open, read_file};
use crate::docs::DocSet;
use crate::export;

impl App {
    /// The feedback document for every placed annotation of the open file.
    pub(crate) fn feedback(&self) -> String {
        Self::feedback_for(&self.open, &self.open.source.name)
    }

    fn feedback_for(open: &Open, name: &str) -> String {
        export::feedback(&open.doc.source, name, &Self::entries_for(open))
    }

    /// Just the annotation blocks, at `level` `#`s, for nesting under a document heading.
    fn annotations_for(open: &Open, level: usize) -> String {
        export::annotations(&open.doc.source, &Self::entries_for(open), level)
    }

    fn entries_for(open: &Open) -> Vec<export::Entry<'_>> {
        let source = &open.doc.source;
        open.store
            .placed()
            .into_iter()
            .map(|p| export::Entry {
                annotation: p.annotation,
                lines: export::line_span(source, p.range),
                range: p.range.clone(),
            })
            .collect()
    }

    /// Feedback for every annotated document in the set, one `# Annotations on <path>` block each.
    pub(crate) fn set_feedback(&self) -> Result<String> {
        let Some(set) = &self.docs else { return Ok(self.feedback()) };
        let width = self.open.layout.width;
        let mut out = String::new();
        for path in self.annotated_files() {
            let open = Open::new(read_file(&path)?, width, &self.data_dir, &self.project, &self.render)?;
            let _ = writeln!(out, "{}", Self::feedback_for(&open, &Self::document_label(set, &path)));
        }
        Ok(if out.is_empty() { "No annotations.".to_owned() } else { out })
    }

    /// What to call `path` in a message: the tab's own name when it is in the set, so the agent
    /// reads the same label that is on screen. Otherwise its path relative to the set's root.
    fn document_label(set: &DocSet, path: &Path) -> String {
        set.name_for(path).map_or_else(
            || path.strip_prefix(set.root()).unwrap_or(path).display().to_string(),
            ToOwned::to_owned,
        )
    }

    /// The whole review `A` hands over: every open document, annotated or not.
    ///
    /// A document with nothing on it is reported as approved **in prose**. Writing a `LooksGood`
    /// annotation instead would enter a note in the store and the feedback archive as though it
    /// had been made by hand, and a later `--export` would replay it; this says the same thing to
    /// the agent and leaves no record behind.
    pub(crate) fn review_feedback(&self) -> Result<String> {
        const APPROVED: &str = "looks good, no changes requested";
        let Some(set) = &self.docs else {
            return Ok(if self.open.store.has_placed() {
                self.feedback()
            } else {
                format!("# Review of {}\n\n{APPROVED}.\n", self.open.source.name)
            });
        };
        let width = self.open.layout.width;
        let annotated = self.annotated_files();
        let mut out = format!("# Review of {} documents\n\n", set.len());
        for doc in set.docs() {
            let label = Self::document_label(set, &doc.path);
            if !annotated.contains(&doc.path) {
                let _ = writeln!(out, "## {label} \u{2014} {APPROVED}\n");
                continue;
            }
            let open = Open::new(read_file(&doc.path)?, width, &self.data_dir, &self.project, &self.render)?;
            let _ = writeln!(out, "## {label}\n");
            out.push_str(&Self::annotations_for(&open, 3));
        }
        Ok(out)
    }
}
