//! The footer: the transient status, the document, its counters, and the key hint.
//!
//! Two halves share one row. The status leads, because it says what just happened; the hint
//! takes what the status leaves and sheds keys to fit. `RAIL_MIN_TOTAL_WIDTH` lives in `draw`,
//! which is what actually drops the rail.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Stylize;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use super::draw::RAIL_MIN_TOTAL_WIDTH;
use super::{App, Mode, help};

/// Why the rail is missing, shown below `RAIL_MIN_TOTAL_WIDTH` when there is room for it.
const RAIL_ADVICE: &str = "rail hidden: widen to \u{2265}80 cols";

/// The footer's left half, and what it leaves the key hint.
pub(super) struct Status {
    pub(super) text: String,
    /// Columns the hint may use. Zero once a counter had to be dropped: a line too narrow for
    /// what the footer always says has none to spare for keys.
    hint_room: usize,
}

impl App {
    pub(super) fn draw_footer(&self, frame: &mut Frame, area: Rect) {
        if self.mode == Mode::ConfirmQuit {
            // The question owns the footer: the browse help would name keys that are not
            // live while it is up.
            let question = format!(
                " send feedback to {} before quitting? y send · n quit · esc cancel",
                self.delivery.describe()
            );
            frame.render_widget(Paragraph::new(Line::from(Span::raw(question).bold())), area);
            return;
        }
        let status = self.footer_status(usize::from(area.width));
        let help = help::hint(self.focus, self.pending.is_some(), status.hint_room);
        let [left_area, right_area] =
            Layout::horizontal([Constraint::Min(0), Constraint::Length(help.width() as u16)]).areas(area);
        frame.render_widget(Paragraph::new(Line::from(Span::raw(status.text).dim())), left_area);
        frame.render_widget(Paragraph::new(Line::from(Span::raw(help).dim()).right_aligned()), right_area);
    }

    /// The status, sized to fit `width` rather than to be cut by it.
    ///
    /// Fields are added in the order they matter and the first that will not fit ends the line,
    /// the way the hint sheds keys from its end. Nothing may spend the columns the document's own
    /// name needs, because which document is on screen is the one thing the footer always says.
    pub(super) fn footer_status(&self, width: usize) -> Status {
        // The status leads: it is the transient half of the line, and the name and counters it
        // pushes right are on screen for the whole session anyway.
        let lead: Vec<String> = self.status.iter().cloned().collect();
        let mut spent: usize = lead.iter().map(|field| field.width() + 3).sum();
        let budget = width.saturating_sub(1); // the leading space
        let mut free = budget.saturating_sub(spent).saturating_sub(self.document_least());

        let orphans = self.open.store.orphans();
        let count = format!(
            "{} annotations{}",
            self.open.store.len(),
            if orphans > 0 { format!(" ({orphans} orphaned)") } else { String::new() }
        );
        let position = match &self.pending {
            Some(p) => {
                let chars = self.open.doc.source.get(p.range.clone()).map_or(0, |s| s.chars().count());
                format!("selected {chars} chars")
            }
            None => format!("block {}/{}", self.selected + 1, self.open.doc.blocks.len()),
        };
        let mut counters: Vec<String> = Vec::new();
        let mut all_counters = true;
        for field in [count, position] {
            let cost = field.width() + 3;
            if cost > free {
                all_counters = false;
                break;
            }
            free -= cost;
            spent += cost;
            counters.push(field);
        }
        // The `?` item keeps its columns however long the path is, so the overlay stays
        // discoverable - but only on a line wide enough for its counters, and only whole:
        // columns held back for a hint that then does not fit are columns the path lost.
        let reserved =
            if all_counters && help::reserved_hint_width() <= free { help::reserved_hint_width() } else { 0 };
        free -= reserved;
        // The advisory is the first field to go. It explains a missing rail, which is worth less
        // than the path and the counts it would otherwise push off the end of the line.
        if width < usize::from(RAIL_MIN_TOTAL_WIDTH) && RAIL_ADVICE.width() + 3 <= free {
            spent += RAIL_ADVICE.width() + 3;
            counters.push(RAIL_ADVICE.to_owned());
        }

        let document = self.document_field(budget.saturating_sub(spent).saturating_sub(reserved));
        let mut parts = lead;
        if !document.is_empty() {
            parts.push(document);
        }
        parts.extend(counters);
        let text = format!(" {}", parts.join(" · "));
        // One column of gap, so the hint can never sit flush against the status.
        let hint_room = if all_counters { width.saturating_sub(text.width()).saturating_sub(1) } else { 0 };
        Status { text, hint_room }
    }

    /// The document, home-relative and elided from the middle, within `room` columns. A reply or
    /// stdin has no file behind it, so there is nothing to spell out.
    fn document_field(&self, room: usize) -> String {
        match self.document_path() {
            // `display_path` reads a room of zero as "as long as it likes", which is the one
            // thing this line cannot afford.
            Some(_) if room == 0 => String::new(),
            Some(path) => {
                let home = std::env::home_dir();
                crate::docs::display_path(&path, home.as_deref(), room)
            }
            None => self.open.source.name.clone(),
        }
    }

    /// The fewest columns worth giving the document: below its file name it names nothing.
    ///
    /// A path pays two more for the `…/` marking the middle it dropped; without them the
    /// elision eats into the name itself.
    fn document_least(&self) -> usize {
        match self.document_path() {
            Some(path) => path.file_name().map_or(1, |name| name.to_string_lossy().width() + 2),
            None => self.open.source.name.width(),
        }
    }
}
