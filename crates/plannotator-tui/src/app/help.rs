//! The keymap: one table, read by the footer hint, the `?` overlay and nothing else.
//!
//! The footer hint used to be prose written by hand in `draw.rs`, which is how help goes stale:
//! it advertised `t hide` for a tree that had been deleted. Both surfaces now render from `KEYS`,
//! and a test walks `input.rs` to check that every key the handler matches is described here, so
//! adding a binding without documenting it fails the build rather than misleading a reader.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};

use super::{App, Focus};

/// Where a binding is live. Also how the overlay groups them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Scope {
    /// Live whatever has focus.
    Always,
    /// The document pane, with no selection waiting.
    Document,
    /// A finished selection is waiting for a verdict.
    Selection,
    /// The notes rail.
    Rail,
}

impl Scope {
    fn title(self) -> &'static str {
        match self {
            Self::Always => "anywhere",
            Self::Document => "document",
            Self::Selection => "selection",
            Self::Rail => "notes",
        }
    }
}

/// One binding: how it is shown, what it does, and the key literals it answers to.
#[derive(Debug)]
pub(super) struct Binding {
    /// As the reader sees it.
    pub(super) label: &'static str,
    /// What it does, for the overlay.
    pub(super) what: &'static str,
    pub(super) scope: Scope,
    /// Every key the input handler matches for this. `Char` keys are the character; named keys
    /// are the `KeyCode` variant. Nothing draws these; they exist so the drift test can compare
    /// the table against `input.rs`.
    #[allow(dead_code, reason = "read by the drift test, which is the point of recording them")]
    pub(super) codes: &'static [&'static str],
    /// Terse wording for the footer, or `None` to leave it to the overlay. Separate from `what`
    /// because the footer shares its line with the status, which must not be pushed off it.
    pub(super) hint: Option<&'static str>,
}

/// Every binding, in the order the overlay lists them.
pub(super) const KEYS: [Binding; 24] = [
    Binding { label: "?", what: "this list", scope: Scope::Always, codes: &["?"], hint: Some("keys") },
    Binding { label: "Tab", what: "next document", scope: Scope::Always, codes: &["Tab"], hint: None },
    Binding { label: "n", what: "notes", scope: Scope::Always, codes: &["n"], hint: None },
    Binding { label: "E", what: "send annotations", scope: Scope::Always, codes: &["E"], hint: Some("send") },
    Binding {
        label: "A",
        what: "send all, approve, close",
        scope: Scope::Always,
        codes: &["A"],
        hint: Some("close"),
    },
    Binding { label: "r", what: "reload from disk", scope: Scope::Always, codes: &["r"], hint: None },
    Binding { label: "p", what: "pick another reply", scope: Scope::Always, codes: &["p"], hint: None },
    Binding { label: "q", what: "quit", scope: Scope::Always, codes: &["q"], hint: Some("quit") },
    Binding {
        label: "j/k",
        what: "block by block",
        scope: Scope::Document,
        codes: &["j", "k", "Down", "Up"],
        hint: None,
    },
    Binding {
        label: "g/G",
        what: "first / last block",
        scope: Scope::Document,
        codes: &["g", "G", "Home", "End"],
        hint: None,
    },
    Binding {
        label: "h/l",
        what: "move the cursor",
        scope: Scope::Document,
        codes: &["h", "l", "Left", "Right"],
        hint: None,
    },
    Binding {
        label: "ctrl+d/u",
        what: "half a page",
        scope: Scope::Document,
        codes: &["d", "u", "PageDown", "PageUp"],
        hint: None,
    },
    Binding { label: "v", what: "select text", scope: Scope::Document, codes: &["v"], hint: Some("select") },
    Binding {
        label: "c",
        what: "comment on block",
        scope: Scope::Document,
        codes: &["c", "Enter"],
        hint: None,
    },
    Binding { label: "x", what: "clear block notes", scope: Scope::Document, codes: &["x"], hint: None },
    Binding { label: "drag", what: "select text", scope: Scope::Document, codes: &[], hint: None },
    Binding {
        label: "a",
        what: "looks good",
        scope: Scope::Selection,
        codes: &["a"],
        hint: Some("looks good"),
    },
    Binding { label: "c", what: "comment", scope: Scope::Selection, codes: &[], hint: Some("comment") },
    Binding { label: "d", what: "delete this", scope: Scope::Selection, codes: &[], hint: Some("delete") },
    Binding {
        label: "esc",
        what: "clear the selection",
        scope: Scope::Selection,
        codes: &["Esc"],
        hint: Some("clear"),
    },
    Binding {
        label: "j/k",
        what: "note by note",
        scope: Scope::Rail,
        codes: &["j", "k", "Down", "Up"],
        hint: Some(""),
    },
    Binding {
        label: "e",
        what: "edit the note",
        scope: Scope::Rail,
        codes: &["e", "Enter"],
        hint: Some("edit"),
    },
    Binding {
        label: "x",
        what: "remove the note",
        scope: Scope::Rail,
        codes: &["x", "Delete"],
        hint: Some("remove"),
    },
    Binding { label: "esc", what: "back to document", scope: Scope::Rail, codes: &["Esc"], hint: Some("") },
];

/// The footer's right-hand hint for what is focused now.
///
/// Built from the same table the overlay reads, so the two cannot disagree about what a key does.
/// The global keys ride along on the document line, where there is room for them; the selection
/// and rail lines are complete in themselves. `?` is offered everywhere, because the overlay is
/// how the rest is found.
pub(super) fn hint(focus: Focus, pending: bool) -> String {
    let scope = match (pending, focus) {
        (true, _) => Scope::Selection,
        (false, Focus::Rail) => Scope::Rail,
        (false, Focus::Document) => Scope::Document,
    };
    let show = |b: &Binding| {
        let hint = b.hint.unwrap_or_default();
        if hint.is_empty() { b.label.to_owned() } else { format!("{} {hint}", b.label) }
    };
    let mut shown: Vec<String> = KEYS
        .iter()
        .filter(|b| b.hint.is_some())
        .filter(|b| b.scope == scope || (b.scope == Scope::Always && scope == Scope::Document))
        .map(show)
        .collect();
    if scope != Scope::Document
        && let Some(overlay) = KEYS.iter().find(|b| b.label == "?")
    {
        shown.push(show(overlay));
    }
    format!("{} ", shown.join(" · "))
}

impl App {
    /// The `?` overlay: every binding, grouped by where it is live.
    ///
    /// Laid out in as many columns as the pane's height requires. A short pane would otherwise
    /// clip the last groups away with nothing on screen to say they existed, which is the same
    /// failure as the stale footer this table replaced.
    pub(super) fn draw_help(&self, frame: &mut Frame) {
        let theme = self.render.theme;
        let area = frame.area();
        let groups: Vec<Vec<Line<'static>>> = [Scope::Always, Scope::Document, Scope::Selection, Scope::Rail]
            .into_iter()
            .map(|scope| self.help_group(scope))
            .collect();

        // Two rows go to the border; a group is never split across columns.
        let room = usize::from(area.height.saturating_sub(2)).max(1);
        let columns = split_columns(&groups, room);
        // Only the columns that fit are drawn. A pane too small for the whole table says how
        // many bindings it is not showing, rather than dropping them where nobody can tell.
        let mut widths: Vec<usize> = Vec::new();
        let mut kept = 0usize;
        let mut spent = 2usize; // the border
        for column in &columns {
            let width = column.iter().map(Line::width).max().unwrap_or(10) + 2;
            if spent + width > usize::from(area.width) && kept > 0 {
                break;
            }
            spent += width;
            widths.push(width);
            kept += 1;
        }
        let hidden: usize = columns
            .iter()
            .skip(kept)
            .flatten()
            .filter(|line| line.width() > 0 && line.spans.len() > 1)
            .count();
        let columns: Vec<Vec<Line<'static>>> = columns.into_iter().take(kept.max(1)).collect();
        let height = columns.iter().map(Vec::len).max().unwrap_or(1) as u16 + 2;
        let width = (spent as u16).min(area.width);

        let rect = Rect {
            x: area.x + (area.width.saturating_sub(width)) / 2,
            y: area.y + (area.height.saturating_sub(height)) / 2,
            width,
            height: height.min(area.height),
        };
        frame.render_widget(Clear, rect);
        let title = if hidden > 0 {
            format!(" keys \u{b7} {hidden} more, widen the pane \u{b7} ? or esc closes ")
        } else {
            " keys \u{b7} ? or esc closes ".to_owned()
        };
        let boxed = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(theme.accent))
            .title(Span::styled(title, Style::new().fg(theme.muted)));
        let inner = boxed.inner(rect);
        frame.render_widget(boxed, rect);
        let mut x = inner.x;
        for (lines, column_width) in columns.into_iter().zip(widths) {
            let column_width = (column_width as u16).min(inner.right().saturating_sub(x));
            if column_width == 0 {
                break;
            }
            let rect = Rect { x, y: inner.y, width: column_width, height: inner.height };
            frame.render_widget(Paragraph::new(lines), rect);
            x += column_width;
        }
    }

    /// One scope's heading and its bindings, with a blank line after it.
    fn help_group(&self, scope: Scope) -> Vec<Line<'static>> {
        let theme = self.render.theme;
        let mut lines = vec![Line::from(Span::styled(
            scope.title().to_owned(),
            Style::new().fg(theme.accent).add_modifier(Modifier::BOLD),
        ))];
        lines.extend(KEYS.iter().filter(|b| b.scope == scope).map(|binding| {
            Line::from(vec![
                Span::styled(format!("  {:<10}", binding.label), Style::new().fg(theme.code)),
                Span::styled(binding.what.to_owned(), Style::new().fg(theme.text)),
            ])
        }));
        lines.push(Line::default());
        lines
    }
}

/// Pack groups into columns no taller than `room`, keeping each group whole.
fn split_columns(groups: &[Vec<Line<'static>>], room: usize) -> Vec<Vec<Line<'static>>> {
    let mut columns: Vec<Vec<Line<'static>>> = Vec::new();
    for group in groups {
        let fits = columns.last().is_some_and(|last: &Vec<Line<'static>>| last.len() + group.len() <= room);
        if fits {
            if let Some(last) = columns.last_mut() {
                last.extend(group.iter().cloned());
            }
        } else {
            columns.push(group.clone());
        }
    }
    if columns.is_empty() { vec![Vec::new()] } else { columns }
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    /// Keys the handler matches that are deliberately absent from the table, with the reason.
    const EXEMPT: [&str; 12] = [
        // The quit question owns the footer while it is up and names its own keys.
        "y",
        "Y",
        "N",
        // Vim motions inside a visual selection; the overlay says "select text" rather than
        // teaching vim, and `w`/`b`/`0`/`$` are only live mid-drag.
        "w",
        "b",
        "0",
        "$",
        // Aliases the table already covers under their primary key.
        "Backspace",
        "Char",
        "BackTab",
        "Tab",
        // ctrl+c, listed as `q`.
        "C",
    ];

    /// Every `KeyCode::Char('x')` and `KeyCode::Named` the input handler matches.
    fn keys_the_handler_matches() -> Vec<String> {
        let source = include_str!("input.rs");
        let mut found: Vec<String> = Vec::new();
        let mut rest = source;
        while let Some(at) = rest.find("KeyCode::") {
            let after = rest.get(at + "KeyCode::".len()..).unwrap_or("");
            if let Some(tail) = after.strip_prefix("Char('") {
                if let Some(ch) = tail.chars().next() {
                    found.push(ch.to_string());
                }
            } else {
                let name: String = after.chars().take_while(char::is_ascii_alphanumeric).collect();
                if !name.is_empty() {
                    found.push(name);
                }
            }
            rest = after;
        }
        found.sort_unstable();
        found.dedup();
        found
    }

    #[test]
    fn every_key_the_handler_answers_to_is_described_in_the_table() {
        let described: Vec<&str> = KEYS.iter().flat_map(|b| b.codes.iter().copied()).collect();
        let undocumented: Vec<String> = keys_the_handler_matches()
            .into_iter()
            .filter(|key| !described.contains(&key.as_str()) && !EXEMPT.contains(&key.as_str()))
            .collect();
        assert!(
            undocumented.is_empty(),
            "input.rs binds keys the help does not mention: {undocumented:?}. \
             Add them to KEYS, or to EXEMPT with the reason."
        );
    }

    #[test]
    fn the_footer_hint_changes_with_what_is_focused() {
        let document = hint(Focus::Document, false);
        assert!(document.contains("v select"), "{document}");
        assert!(document.contains("? keys"), "the overlay is always reachable: {document}");
        assert!(document.contains("A close"), "the global keys ride the document line: {document}");
        assert!(!document.contains("looks good"), "no verdict keys without a selection: {document}");

        let selection = hint(Focus::Document, true);
        assert!(selection.contains("a looks good"), "{selection}");
        assert!(selection.contains("? keys"), "{selection}");

        let rail = hint(Focus::Rail, false);
        assert!(rail.contains("e edit") && rail.contains("x remove"), "{rail}");
        assert!(!rail.contains("v select"), "document keys are not live in the rail: {rail}");
    }

    /// The footer shares its line with the status, which leads it. A hint that grows without
    /// bound pushes the status off the pane, which is what this width ceiling protects.
    #[test]
    fn no_footer_hint_crowds_out_the_status() {
        for (focus, pending) in [(Focus::Document, false), (Focus::Document, true), (Focus::Rail, false)] {
            let width = hint(focus, pending).chars().count();
            assert!(width <= 60, "{focus:?} pending={pending} hint is {width} columns wide");
        }
    }
}
