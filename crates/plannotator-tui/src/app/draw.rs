//! Drawing: the tab row, header, gutter + document, annotation rail, footer and the floating
//! toolbar. Pure over `App` except for recording geometry for hit-testing. The compose box is
//! `compose_view`: it floats over the document and needs nothing from this layout but an anchor.

use std::ops::Range;

use plannotator_tui_schema::Kind;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

use super::{App, Focus, GUTTER, Geometry, Mode, TOOLBAR, glyph, label};
use crate::docs::{DocSet, marker_width};
use crate::wrap::wrap_line;

const RAIL_WIDTH: u16 = 36;
const RAIL_MIN_WIDTH: u16 = 28;
/// Below this the rail is dropped and annotations are only marked in the gutter.
pub(super) const RAIL_MIN_TOTAL_WIDTH: u16 = 80;

/// Painting precedence when annotations overlap a cell.
fn priority(kind: Kind) -> u8 {
    match kind {
        Kind::Comment => 0,
        Kind::LooksGood => 1,
        Kind::Delete => 2,
    }
}

impl App {
    pub(crate) fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        // The tab row sits above the header, where nvim puts its tabline. One document needs no
        // tabs, so a single presented file costs no chrome.
        let tabs_height = u16::from(self.docs.as_ref().is_some_and(|set| set.len() > 1));
        let [tabs, header, body, footer] = Layout::vertical([
            Constraint::Length(tabs_height),
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .areas(area);

        // The rail only holds bubbles for annotations that exist. Reserving it while there are
        // none costs about a third of the width and shows nothing - which is most of the time a
        // document is being read rather than marked up. The toolbar and the compose box float
        // over the document, so they do not need this space either. Adding the first annotation
        // reflows the document once, which is also how the rail announces itself.
        let rail_wanted = self.open.store.has_placed() && area.width >= RAIL_MIN_TOTAL_WIDTH;
        let rail_width =
            if rail_wanted { (area.width * 3 / 10).clamp(RAIL_MIN_WIDTH, RAIL_WIDTH) } else { 0 };
        let [gutter, doc, _gap, rail] = Layout::horizontal([
            Constraint::Length(GUTTER),
            Constraint::Min(20),
            Constraint::Length(u16::from(rail_width > 0)),
            Constraint::Length(rail_width),
        ])
        .areas(body);
        self.geometry = Geometry {
            doc,
            toolbar: None,
            bubbles: Vec::new(),
            send_button: None,
            pick_rows: Vec::new(),
            tabs: Vec::new(),
        };

        if self.open.layout.width != usize::from(doc.width) {
            self.open.layout.reflow(usize::from(doc.width));
            self.clear_selection();
            self.scroll_by(0);
        }

        if tabs_height > 0 {
            self.draw_tabs(frame, tabs);
        }
        self.draw_header(frame, header);
        self.draw_document(frame, gutter, doc);
        if rail_width > 0 {
            self.draw_rail(frame, rail);
        }
        self.draw_footer(frame, footer);
        match &self.mode {
            Mode::Compose => self.draw_compose(frame, &self.compose_title("comment")),
            Mode::Edit(_) => self.draw_compose(frame, &self.compose_title("edit")),
            Mode::Browse if self.pending.is_some() => self.draw_toolbar(frame),
            Mode::Pick => self.draw_pick(frame),
            Mode::Help => self.draw_help(frame),
            Mode::Browse | Mode::ConfirmQuit => {}
        }
    }

    /// The tab row: one tab per presented document, the open one highlighted, each with its
    /// annotation count. Tabs that do not fit become a count at the edge they went past.
    fn draw_tabs(&mut self, frame: &mut Frame, area: Rect) {
        let theme = self.render.theme;
        let Some(set) = &self.docs else { return };
        let width = usize::from(area.width);
        let row = set.tab_row(width);
        let mut spans: Vec<Span<'static>> = Vec::new();
        if row.hidden_before > 0 {
            spans.push(Span::styled(format!("\u{2039}{} ", row.hidden_before), Style::new().fg(theme.muted)));
        }
        // What the markers leave for labels; the open tab is truncated rather than dropped.
        let budget = width
            .saturating_sub(marker_width(row.hidden_before))
            .saturating_sub(marker_width(row.hidden_after));
        let mut used = 0usize;
        let mut spans_at = usize::from(area.x) + marker_width(row.hidden_before);
        let mut clickable: Vec<(Range<u16>, usize)> = Vec::new();
        for index in row.visible.clone() {
            let Some(doc) = set.docs().get(index) else { continue };
            if index != row.visible.start {
                spans.push(Span::styled("\u{2502}", Style::new().fg(theme.border)));
                used += 1;
                spans_at += 1;
            }
            let label = DocSet::label(doc);
            let room = budget.saturating_sub(used);
            let label = if label.width() > room { truncate(&label, room) } else { label };
            // The span a click lands in, recorded where it is laid out: nothing else knows it.
            let start = u16::try_from(spans_at).unwrap_or(u16::MAX);
            let end = u16::try_from(spans_at + label.width()).unwrap_or(u16::MAX);
            clickable.push((start..end, index));
            spans_at += label.width();
            used += label.width();
            let style = if index == set.current() {
                Style::new().fg(theme.accent).add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(theme.muted)
            };
            spans.push(Span::styled(label, style));
        }
        if row.hidden_after > 0 {
            let pad = width.saturating_sub(used).saturating_sub(marker_width(row.hidden_before));
            let pad = pad.saturating_sub(marker_width(row.hidden_after));
            spans.push(Span::raw(" ".repeat(pad)));
            spans.push(Span::styled(format!(" {}\u{203a}", row.hidden_after), Style::new().fg(theme.muted)));
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
        self.geometry.tabs = clickable;
    }

    fn draw_document(&self, frame: &mut Frame, gutter: Rect, doc: Rect) {
        let theme = self.render.theme;
        let placed = self.open.store.placed();
        let text_selection_active = self.selection.is_some();
        let doc_focused = self.focus == Focus::Document;
        let buf = frame.buffer_mut();

        for y in 0..doc.height {
            let row_index = self.scroll + usize::from(y);
            let Some(block) = self.open.layout.block_at_row(row_index) else { continue };
            let Some(row) = self.open.layout.row(row_index) else { continue };
            let screen_y = doc.y + y;
            buf.set_line(doc.x, screen_y, &row.line, doc.width);

            if block == self.selected && !text_selection_active && self.pending.is_none() && doc_focused {
                buf.set_style(
                    Rect { x: doc.x, y: screen_y, width: doc.width, height: 1 },
                    Style::new().bg(theme.block_bg),
                );
            }

            let mut row_has_annotation = false;
            for (col, cell) in row.cells.iter().enumerate().take(usize::from(doc.width)) {
                let Some(offset) = cell else { continue };
                let kind = placed
                    .iter()
                    .filter(|p| p.range.contains(offset))
                    .map(crate::store::Placed::kind)
                    .max_by_key(|&k| priority(k));
                let Some(kind) = kind else { continue };
                row_has_annotation = true;
                let style = match kind {
                    Kind::Comment => Style::new().bg(theme.comment_bg),
                    Kind::LooksGood => Style::new().bg(theme.approve_bg),
                    Kind::Delete => {
                        Style::new().fg(theme.delete).add_modifier(Modifier::CROSSED_OUT | Modifier::DIM)
                    }
                };
                buf.set_style(Rect { x: doc.x + col as u16, y: screen_y, width: 1, height: 1 }, style);
            }

            if let Some(cols) = self.selection.and_then(|s| s.columns_on(row_index, row.cells.len().max(1))) {
                let start = cols.start.min(usize::from(doc.width)) as u16;
                let end = cols.end.min(usize::from(doc.width)) as u16;
                if end > start {
                    let rect = Rect { x: doc.x + start, y: screen_y, width: end - start, height: 1 };
                    buf.set_style(rect, Style::new().add_modifier(Modifier::REVERSED));
                }
            }

            // Keyboard cursor, visible while selecting with the keyboard.
            if doc_focused && self.selection.is_some_and(|s| s.dragging) && row_index == self.cursor.0 {
                let x = doc.x + (self.cursor.1.min(usize::from(doc.width).saturating_sub(1))) as u16;
                buf.set_style(Rect { x, y: screen_y, width: 1, height: 1 }, Style::new().bg(theme.cursor_bg));
            }

            // Column 0 is the sign column, gitsigns-style; the block marker moves to column 1,
            // closer to the text it marks. A row's first mapped source byte gives its line, and
            // art rows carry no offsets at all, so those fall back to where their block starts.
            // Every byte the row shows, not just its first: a reflowed paragraph carries several
            // source lines, and only one of them needs to have changed.
            let mut bytes = row.cells.iter().flatten().copied().peekable();
            let kind = if bytes.peek().is_some() {
                self.open.changes.kind_over(bytes)
            } else {
                // Art has no source bytes of its own, so it answers for its whole block.
                self.open.doc.blocks.get(block).and_then(|b| self.open.changes.kind_over(b.range.clone()))
            };
            if let Some(kind) = kind {
                let bar = Span::styled("│", Style::new().fg(theme.change(kind)));
                buf.set_span(gutter.x, screen_y, &bar, 1);
            }

            let marker = match (block == self.selected, row_has_annotation) {
                (true, _) => Span::styled("▍", Style::new().fg(theme.accent)),
                (false, true) => Span::styled("▍", Style::new().fg(theme.comment)),
                (false, false) => Span::raw(" "),
            };
            buf.set_span(gutter.x + 1, screen_y, &marker, 1);
        }
    }

    /// Screen position for a floating widget anchored at the pending selection: one row
    /// above its first row when there is room, else just below its last row. The toolbar and
    /// the compose box share it, so both appear where the selection is.
    pub(super) fn float_origin(&self, height: u16, width: u16) -> Option<Rect> {
        let pending = self.pending.as_ref()?;
        let doc = self.geometry.doc;
        let (row, col) = pending.at;
        if row < self.scroll || row >= self.scroll + usize::from(doc.height) {
            return None;
        }
        let screen_row = doc.y + (row - self.scroll) as u16;
        let width = width.min(doc.width);
        let x = (doc.x + col as u16).min(doc.right().saturating_sub(width));
        let y = if screen_row >= doc.y + height {
            screen_row - height
        } else {
            let last_row = self.selection.map_or(row, |s| s.ordered().1.0);
            let below = doc.y + last_row.saturating_sub(self.scroll) as u16 + 1;
            below.min(doc.bottom().saturating_sub(height))
        };
        Some(Rect { x, y, width, height })
    }

    fn draw_toolbar(&mut self, frame: &mut Frame) {
        let theme = self.render.theme;
        let labels: Vec<String> = TOOLBAR.iter().map(|(g, l, k, _)| format!(" {g} {l} ({k}) ")).collect();
        let width: u16 = labels.iter().map(|l| l.width() as u16).sum::<u16>() + 1;
        let Some(rect) = self.float_origin(1, width) else { return };
        frame.render_widget(Clear, rect);
        let buf = frame.buffer_mut();
        buf.set_style(rect, Style::new().bg(theme.toolbar_bg));
        let mut x = rect.x + 1;
        let mut spans = [0..0, 0..0, 0..0];
        for ((label, item), span) in labels.iter().zip(TOOLBAR.iter()).zip(spans.iter_mut()) {
            let w = label.width() as u16;
            let style = Style::new().fg(theme.kind(item.3)).bg(theme.toolbar_bg).bold();
            buf.set_span(x, rect.y, &Span::styled(label.as_str(), style), w);
            *span = x..x + w;
            x += w;
        }
        self.geometry.toolbar = Some((rect, spans));
    }

    fn draw_rail(&mut self, frame: &mut Frame, rail: Rect) {
        let theme = self.render.theme;
        let view_end = self.scroll + usize::from(rail.height);
        let rail_focused = self.focus == Focus::Rail;
        let mut next_y = rail.y;
        let placed = self.open.store.placed();
        let mut bubbles = Vec::new();
        for (index, placed) in placed.iter().enumerate() {
            let Some(block) = self.open.doc.block_containing(placed.range.start) else { continue };
            let Some(rendered) = self.open.layout.blocks.get(block) else { continue };
            let anchor_row =
                self.open.layout.first_row_in_range(block, placed.range).unwrap_or(rendered.first_row);
            if anchor_row + 1 < self.scroll.saturating_sub(2)
                || anchor_row >= view_end
                || next_y >= rail.bottom()
            {
                continue;
            }
            let anchored_y = rail.y + anchor_row.saturating_sub(self.scroll) as u16;
            let y = anchored_y.max(next_y);
            let kind = placed.kind();
            let body = if placed.annotation.body.is_empty() {
                label(kind).to_owned()
            } else {
                placed.annotation.body.clone()
            };
            let inner_width = usize::from(rail.width.saturating_sub(4));
            let lines: Vec<Line<'static>> =
                wrap_line(&Line::from(body.as_str()), &[], inner_width).into_iter().map(|r| r.line).collect();
            let height = (lines.len() as u16 + 2).min(rail.bottom().saturating_sub(y));
            if height < 3 {
                break;
            }
            let highlighted = if rail_focused { index == self.rail_cursor } else { block == self.selected };
            let border =
                if highlighted { Style::new().fg(theme.kind(kind)) } else { Style::new().fg(theme.muted) };
            let border = if rail_focused && index == self.rail_cursor { border.bold() } else { border };
            let title = Span::styled(
                format!(" {} {} ", glyph(kind), short_id(&placed.annotation.id)),
                Style::new().fg(theme.kind(kind)),
            );
            let bubble = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(border)
                .title(title);
            let rect = Rect { x: rail.x, y, width: rail.width, height };
            let inner = bubble.inner(rect);
            frame.render_widget(bubble, rect);
            let body_style =
                if placed.annotation.body.is_empty() { Style::new().dim().italic() } else { Style::new() };
            let text_area = Rect { x: inner.x + 1, width: inner.width.saturating_sub(1), ..inner };
            frame.render_widget(Paragraph::new(lines).style(body_style), text_area);
            bubbles.push((rect, placed.annotation.id.clone()));
            next_y = y + height;
        }
        self.geometry.bubbles = bubbles;
    }
}

/// The tail of an id, enough to tell bubbles apart: `anno_…F0123` → `F0123`.
fn short_id(id: &str) -> String {
    let tail: Vec<char> = id.chars().rev().take(5).collect();
    tail.into_iter().rev().collect()
}

/// Cut a tab label to `room` display columns, marking that it was cut.
fn truncate(label: &str, room: usize) -> String {
    if room == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0usize;
    for ch in label.chars() {
        let next = used + ch.to_string().width();
        if next > room.saturating_sub(1) {
            break;
        }
        out.push(ch);
        used = next;
    }
    out.push('\u{2026}');
    out
}
