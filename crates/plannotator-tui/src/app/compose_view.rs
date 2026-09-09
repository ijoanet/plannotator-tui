//! The compose overlay: the floating box a comment is typed into, for a new annotation or an
//! edit of an existing one.
//!
//! It draws over the document with `Clear` and takes no space in the layout, so `draw` never has
//! to reserve any for it. Where it lands comes from `draw`'s `float_origin` when a selection is
//! pending - the same anchor the toolbar uses - and from the rail bubble when an existing
//! annotation is being edited. The state it renders is `compose::Compose`.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};

use super::{App, Mode};

const COMPOSE_WIDTH: u16 = 48;

impl App {
    /// The compose box title; the Shift+Enter hint appears only when the terminal
    /// actually distinguishes it, so the hint is never a lie.
    pub(super) fn compose_title(&self, verb: &str) -> String {
        let newline = if self.shift_enter { "shift+enter new line" } else { "alt+enter new line" };
        format!(" {verb} \u{b7} enter saves \u{b7} {newline} \u{b7} esc cancels ")
    }

    /// The compose box: at the pending selection when there is one, else over the rail
    /// bubble being edited, else centered.
    pub(super) fn draw_compose(&self, frame: &mut Frame, title: &str) {
        let theme = self.render.theme;
        let wrap_width = usize::from(COMPOSE_WIDTH.saturating_sub(3));
        let (lines, cursor_row, cursor_col) = self.compose.wrapped(wrap_width);
        let content_rows = lines.len().clamp(1, 8);
        let height = content_rows as u16 + 2;
        let rect = self
            .float_origin(height, COMPOSE_WIDTH)
            .or_else(|| self.edit_origin(height, COMPOSE_WIDTH))
            .unwrap_or_else(|| {
                let area = frame.area();
                let width = COMPOSE_WIDTH.min(area.width);
                Rect { x: (area.width - width) / 2, y: area.height / 2, width, height }
            });
        frame.render_widget(Clear, rect);
        let boxed = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(theme.comment))
            .title(Span::styled(title.to_owned(), Style::new().dim()));
        let inner = boxed.inner(rect);
        frame.render_widget(boxed, rect);
        // Keep the cursor's row visible when the comment is taller than the box.
        let scroll = cursor_row.saturating_sub(content_rows - 1);
        for (i, line) in lines.iter().skip(scroll).take(content_rows).enumerate() {
            let row = Rect {
                x: inner.x + 1,
                y: inner.y + i as u16,
                width: inner.width.saturating_sub(1),
                height: 1,
            };
            frame.render_widget(Paragraph::new(Line::from(line.clone())), row);
        }
        let cursor_x = inner.x + 1 + cursor_col as u16;
        let cursor_y = inner.y + (cursor_row - scroll) as u16;
        frame.set_cursor_position((cursor_x.min(inner.right().saturating_sub(1)), cursor_y));
    }

    fn edit_origin(&self, height: u16, width: u16) -> Option<Rect> {
        let Mode::Edit(id) = &self.mode else { return None };
        let (rect, _) = self.geometry.bubbles.iter().find(|(_, bubble_id)| bubble_id == id)?;
        let area = self.geometry.doc.union(*rect);
        let x = rect.right().saturating_sub(width).max(area.x);
        Some(Rect { x, y: rect.y, width: width.min(area.width), height })
    }
}
