//! Sending feedback to the delivery target and the state the Send button shows.

use std::path::PathBuf;

use anyhow::Result;

use super::{App, Exit, Mode, Open, read_file};
use crate::delivery::{Clipboard, Delivery as _, DeliveryError};
use crate::store::Store;
use plannotator_tui_schema::{Kind, Provenance};

/// What the Send button says. Re-derived from the store on load and file switch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SendState {
    /// Something to send (or nothing yet, in which case the button is dimmed).
    Ready,
    /// Everything on record has been sent; nothing changed since.
    Sent,
    /// The last send was refused because the agent is at a dialog.
    Blocked(String),
}

impl App {
    /// `E`: send the annotations. Nothing to say means nothing is sent, which is what separates
    /// it from `A`: an agent should not be handed "No annotations." as though it were a review.
    ///
    /// Returns whether it actually reached the target. `Blocked` and `Unavailable` fall back to
    /// the clipboard, so the agent received nothing; callers that act on a send must not read
    /// `send_state` instead, because it may still say `Sent` from an earlier one.
    pub(super) fn send_feedback(&mut self) -> Result<bool> {
        if self.send_count() == 0 {
            self.status = Some("nothing to send yet \u{b7} A hands over the whole review".into());
            return Ok(false);
        }
        let text = if self.docs.is_some() { self.set_feedback()? } else { self.feedback() };
        self.deliver_review(&text)
    }

    /// Hand `text` to the delivery target, then record, archive and clear what it covered.
    fn deliver_review(&mut self, text: &str) -> Result<bool> {
        let count = self.send_count();
        let target = self.delivery.describe();
        match self.delivery.deliver(text) {
            Ok(()) => {
                self.record_delivery(&target)?;
                // The store is the recovery copy when the archive cannot write, so a file is only
                // cleared once its feedback is durable somewhere else.
                let archived = self.archive_submission(text);
                let cleared = if archived { self.clear_sent()? } else { 0 };
                self.send_state = SendState::Sent;
                self.status = Some(match (cleared, archived) {
                    (0, false) => format!("sent {count} annotation(s) → {target} · kept, not archived"),
                    (0, true) => format!("sent {count} annotation(s) → {target}"),
                    (n, _) => format!("sent {count} annotation(s) → {target} · cleared {n}"),
                });
                Ok(true)
            }
            Err(DeliveryError::Blocked(msg)) => {
                self.copy_fallback(text);
                self.status =
                    Some(format!("{target} is at a dialog — copied to clipboard instead; E retries"));
                self.send_state = SendState::Blocked(msg);
                Ok(false)
            }
            Err(DeliveryError::Unavailable(msg)) => {
                self.copy_fallback(text);
                self.status = Some(format!("no agent to send to ({msg}) — copied to clipboard"));
                Ok(false)
            }
            Err(DeliveryError::Failed(err)) => {
                self.status = Some(format!("send failed: {err:#}"));
                Ok(false)
            }
        }
    }

    /// `A`: hand the whole review over and dismiss the reviewer, pane and all.
    ///
    /// Every open document is covered, not only the annotated ones: a clean document is reported
    /// as approved, so the agent learns it was read and found fine. That is why this sends even
    /// when the entire set is clean, where `E` would have nothing to say.
    ///
    /// Only a delivery that reached the target closes anything. A refused or failed send leaves
    /// the pane open, because the status line is then the only thing that says what happened, and
    /// closing the pane would take it with them.
    pub(super) fn send_and_close(&mut self) -> Result<()> {
        let text = self.review_feedback()?;
        if self.deliver_review(&text)? {
            self.exit = Exit::QuitAndClosePane;
        }
        Ok(())
    }

    fn copy_fallback(&self, text: &str) {
        if self.clipboard {
            let _ = Clipboard.deliver(text);
        }
    }

    /// Record the submission in the shared feedback archive (contract: Plannotator's
    /// `feedback-archive.ts` v1). Never fails the send; the annotation store is the
    /// recovery copy when archiving cannot write.
    /// Returns whether the submission was written, which is what makes clearing safe.
    fn archive_submission(&self, feedback: &str) -> bool {
        use crate::archive::{self, Submission, Target};
        if !archive::enabled(|key| std::env::var(key).ok(), &self.data_dir) {
            return false;
        }
        let (surface, target, annotations) = if let Some(set) = &self.docs {
            // A set submits one body of feedback for the whole session; the per-document records
            // are not part of it (contract semantics).
            ("annotate-folder", Target::file(set.root()), Vec::new())
        } else {
            let annotations = Self::annotation_records(&self.open.store);
            match &self.open.source.provenance {
                Provenance::File { path } => ("annotate", Target::file(path), annotations),
                Provenance::AgentMessage { host, session, .. } => (
                    "annotate-last",
                    Target::agent(
                        archive::origin_label(host),
                        session.clone(),
                        (!self.message_transcript.is_empty()).then(|| self.message_transcript.clone()),
                    ),
                    annotations,
                ),
                _ => ("annotate", Target::default(), annotations),
            }
        };
        let origin = self.delivery.agent_host().map(|host| archive::origin_label(host).to_owned());
        archive::append(&Submission {
            data_dir: &self.data_dir,
            project: &self.project,
            surface,
            origin,
            target,
            feedback,
            annotations,
            count: self.send_count(),
            now_ms: None,
        })
        .is_some()
    }

    /// Clear what the send just covered: the open document's annotations, and in a set every
    /// other annotated document's too, mirroring `record_delivery`.
    ///
    /// A review that has been handed over and archived has done its job; leaving it behind made
    /// every later send repeat it, so an agent received items it had already acted on.
    fn clear_sent(&mut self) -> Result<usize> {
        if !self.render.review.clear_on_send {
            return Ok(0);
        }
        let mut cleared = 0usize;
        if self.docs.is_none() {
            cleared += self.open.store.remove_placed()?;
        } else {
            let width = self.open.layout.width;
            for path in self.annotated_files() {
                if self.is_open(&path) {
                    cleared += self.open.store.remove_placed()?;
                } else {
                    let mut open =
                        Open::new(read_file(&path)?, width, &self.data_dir, &self.project, &self.render)?;
                    cleared += open.store.remove_placed()?;
                }
            }
        }
        self.rail_cursor = 0;
        self.clear_selection();
        self.sync_doc_counts();
        Ok(cleared)
    }

    fn annotation_records(store: &Store) -> Vec<crate::archive::AnnotationRecord> {
        store
            .placed()
            .iter()
            .map(|placed| {
                let a = placed.annotation;
                crate::archive::AnnotationRecord {
                    id: Some(a.id.clone()),
                    kind: Some(
                        match a.anchor.kind() {
                            Kind::Comment => "comment",
                            Kind::LooksGood => "looks-good",
                            Kind::Delete => "delete",
                        }
                        .to_owned(),
                    ),
                    text: (!a.body.is_empty()).then(|| a.body.clone()),
                    original_text: (!a.anchor.original_text.is_empty())
                        .then(|| a.anchor.original_text.clone()),
                }
            })
            .collect()
    }

    /// Annotations the next send covers: the whole set's, or the open document's.
    pub(super) fn send_count(&self) -> usize {
        match &self.docs {
            Some(set) => set.total_annotations(),
            None => self.open.store.placed().len(),
        }
    }

    /// Text for the Send button.
    pub(super) fn send_label(&self) -> String {
        let target = self.delivery.describe();
        let count = self.send_count();
        if self.delivery.is_agent() {
            match &self.send_state {
                SendState::Ready => format!("Send {count} to {target} ▸"),
                SendState::Sent => format!("Sent ▸ {target}"),
                SendState::Blocked(_) => format!("{target} at a dialog · copied · click to retry"),
            }
        } else {
            match &self.send_state {
                SendState::Sent => "Copied".to_owned(),
                SendState::Ready | SendState::Blocked(_) => format!("Copy {count} as feedback"),
            }
        }
    }

    /// True when an agent is waiting on feedback that has not been sent since it changed.
    pub(super) fn has_unsent(&self) -> bool {
        self.delivery.is_agent() && self.send_count() > 0 && self.send_state != SendState::Sent
    }

    /// Quit, unless an agent is still waiting on feedback: then ask in the footer first.
    pub(super) fn request_quit(&mut self) {
        if self.has_unsent() {
            self.mode = Mode::ConfirmQuit;
        } else {
            self.exit = Exit::Quit;
        }
    }

    /// Recompute the send state from the record (on load and document switch).
    pub(super) fn derive_send_state(&mut self) {
        let delivered = match &self.docs {
            Some(_) => self.set_all_delivered().unwrap_or(false),
            None => self.open.store.all_delivered(),
        };
        self.send_state = if delivered { SendState::Sent } else { SendState::Ready };
    }

    /// Any annotation change makes the record unsent again.
    pub(super) fn mark_unsent(&mut self) {
        self.send_state = SendState::Ready;
    }
    /// Paths of every annotated file a send covers: the project's records (which carry their
    /// document path since 0.5.0) plus any document in the set with a count, so a record written
    /// by an older build is still found.
    ///
    /// This is what `E`, `record_delivery` and `clear_sent` all walk. It used to come from the
    /// tree's rows; it now comes from the set's counts, which `sync_doc_counts` keeps current.
    pub(super) fn annotated_files(&self) -> Vec<PathBuf> {
        let Some(set) = &self.docs else { return Vec::new() };
        let mut found = Store::annotated_documents(&self.data_dir, &self.project);
        for doc in set.docs().iter().filter(|d| d.annotations > 0) {
            found.push(doc.path.clone());
        }
        found.sort();
        found.dedup();
        found.retain(|p| p.is_file());
        found
    }
    /// Remember the send on every file it covered: the open one in memory, the rest on disk.
    fn record_delivery(&mut self, target: &str) -> Result<()> {
        if self.docs.is_none() {
            return self.open.store.record_delivery(target);
        }
        let width = self.open.layout.width;
        for path in self.annotated_files() {
            if self.is_open(&path) {
                self.open.store.record_delivery(target)?;
            } else {
                let mut open =
                    Open::new(read_file(&path)?, width, &self.data_dir, &self.project, &self.render)?;
                open.store.record_delivery(target)?;
            }
        }
        Ok(())
    }

    /// True when every annotated document in the set has been sent since it last changed.
    fn set_all_delivered(&self) -> Result<bool> {
        let files = self.annotated_files();
        if files.is_empty() {
            return Ok(false);
        }
        let width = self.open.layout.width;
        for path in files {
            let delivered = if self.is_open(&path) {
                self.open.store.all_delivered()
            } else {
                Open::new(read_file(&path)?, width, &self.data_dir, &self.project, &self.render)?
                    .store
                    .all_delivered()
            };
            if !delivered {
                return Ok(false);
            }
        }
        Ok(true)
    }
}
