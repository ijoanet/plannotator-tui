//! The header row: the Send button, right-aligned, and nothing else. The pane label
//! (Herdr's) names the app; the footer names the file. The button is clickable, so its rect
//! is recorded for hit-testing.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use unicode_width::UnicodeWidthStr;

use super::App;
use super::send::SendState;

impl App {
    pub(super) fn draw_header(&mut self, frame: &mut Frame, area: Rect) {
        let button = format!(" {} ", self.send_label());
        let width = button.width() as u16;
        self.geometry.send_button =
            (area.width >= width).then(|| Rect { x: area.right() - width, y: area.y, width, height: 1 });
        if let Some(rect) = self.geometry.send_button {
            let span = Span::styled(button, self.button_style());
            frame.buffer_mut().set_span(rect.x, rect.y, &span, rect.width);
        }
    }

    /// Teal when there is something to send, grey at zero, green once sent, yellow when the
    /// agent refused it.
    fn button_style(&self) -> Style {
        let theme = self.render.theme;
        match &self.send_state {
            SendState::Ready if self.send_count() == 0 => {
                Style::new().fg(theme.muted).bg(theme.toolbar_bg).add_modifier(Modifier::DIM)
            }
            SendState::Ready => {
                Style::new().fg(crate::theme::readable_on(theme.send_bg)).bg(theme.send_bg).bold()
            }
            SendState::Sent => {
                Style::new().fg(crate::theme::readable_on(theme.approve_bg)).bg(theme.approve_bg).bold()
            }
            SendState::Blocked(_) => {
                Style::new().fg(crate::theme::readable_on(theme.comment_bg)).bg(theme.comment_bg).bold()
            }
        }
    }
}
