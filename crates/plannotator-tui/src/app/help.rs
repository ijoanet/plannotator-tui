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
use unicode_width::UnicodeWidthStr;

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
pub(super) const KEYS: [Binding; 25] = [
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
    Binding {
        label: "q",
        what: "close this tab, dropping it from the review",
        scope: Scope::Always,
        codes: &["q"],
        hint: Some("close tab"),
    },
    Binding { label: "Q", what: "quit now, send nothing", scope: Scope::Always, codes: &["Q"], hint: None },
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

/// The footer's hint items for what is focused now, most worth keeping first.
///
/// Built from the same table the overlay reads, so the two cannot disagree about what a key does.
/// The global keys ride along on the document line, where there is room for them; the selection
/// and rail lines are complete in themselves. `?` leads everywhere, because the overlay is how the
/// rest is found, and leading means it is the last item dropped when the line is tight.
fn hint_items(focus: Focus, pending: bool) -> Vec<String> {
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
        shown.insert(0, show(overlay));
    }
    shown
}

/// Display columns `items` occupy once joined, trailing gap included.
fn hint_width(items: &[String]) -> usize {
    if items.is_empty() {
        return 0;
    }
    items.join(" · ").width() + 1
}

/// The footer's right-hand hint, within `room` columns.
///
/// Items shed from the end until it fits, and the whole hint goes if even one will not: the status
/// shares this line and leads it, so a hint that cannot fit must cost keys rather than cost the
/// message saying what just happened. Half a key name would be worse than one fewer key.
/// The footer carries **only** `?`.
///
/// It used to list every hinted key, which spent a third of a narrow row restating what the
/// overlay says in full. One item is enough: `?` is how the rest is found, and the columns are
/// worth more to the document's path. The shedding logic stays because the item still has to fit.
pub(super) fn hint(focus: Focus, pending: bool, room: usize) -> String {
    let mut shown = hint_items(focus, pending);
    shown.truncate(1);
    while !shown.is_empty() && hint_width(&shown) > room {
        shown.pop();
    }
    if shown.is_empty() { String::new() } else { format!("{} ", shown.join(" · ")) }
}

/// Columns to keep for the hint when sizing the status, enough for the `?` item alone.
///
/// The overlay is how every other key is found, so the line keeps room for it even when the
/// document's path would otherwise fill the pane.
pub(super) fn reserved_hint_width() -> usize {
    hint_items(Focus::Document, false).first().map_or(0, |first| first.width() + 1)
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
        // Bindings are lost two ways and both must be owned up to. Columns past the pane's width
        // are dropped here; rows past its height are clipped by `Paragraph` with nothing on screen
        // to say so, which is the failure this module exists to prevent.
        let dropped: usize = columns.iter().skip(kept).flatten().filter(|l| is_binding(l)).count();
        let columns: Vec<Vec<Line<'static>>> = columns.into_iter().take(kept.max(1)).collect();
        let tallest = columns.iter().map(Vec::len).max().unwrap_or(1);
        let height = (tallest as u16 + 2).min(area.height);
        let visible_rows = usize::from(height.saturating_sub(2));
        let clipped: usize =
            columns.iter().flat_map(|c| c.iter().skip(visible_rows)).filter(|l| is_binding(l)).count();
        let hidden = dropped + clipped;
        let width = (spent as u16).min(area.width);

        let rect = Rect {
            x: area.x + (area.width.saturating_sub(width)) / 2,
            y: area.y + (area.height.saturating_sub(height)) / 2,
            width,
            height,
        };
        frame.render_widget(Clear, rect);
        // Name the dimension that is short, so the advice is actionable rather than a guess.
        let advice = match (dropped > 0, clipped > 0) {
            (true, true) => "resize the pane",
            (true, false) => "widen the pane",
            _ => "lengthen the pane",
        };
        let title = if hidden > 0 {
            format!(" keys \u{b7} {hidden} more, {advice} \u{b7} ? or esc closes ")
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

/// Whether a line is a binding rather than a heading or a spacer.
fn is_binding(line: &Line<'static>) -> bool {
    line.width() > 0 && line.spans.len() > 1
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
    const EXEMPT: [&str; 8] = [
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
        // ctrl+c, whose character is already listed as the comment key.
        "C",
    ];

    /// Every module that answers a key. `input.rs` is the main handler; the compose box and the
    /// reply picker read keys while they are up, and this module is scanned so its own prose cannot
    /// drift either.
    const HANDLERS: [(&str, &str); 4] = [
        ("input.rs", include_str!("input.rs")),
        ("compose.rs", include_str!("compose.rs")),
        ("pick.rs", include_str!("pick.rs")),
        ("help.rs", include_str!("help.rs")),
    ];

    /// Every `KeyCode` variant the handlers match, with the module that matched it.
    ///
    /// Comments are stripped first: this file names `KeyCode` variants in prose, and prose is not
    /// a binding. String literals are harmless, because the scanner only keeps a name when one
    /// follows the marker immediately.
    fn keys_the_handlers_match() -> Vec<(String, &'static str)> {
        let mut found: Vec<(String, &'static str)> = Vec::new();
        for (module, source) in HANDLERS {
            let code = source
                .lines()
                .map(|line| line.split("//").next().unwrap_or(""))
                .collect::<Vec<_>>()
                .join("\n");
            let mut rest = code.as_str();
            while let Some(at) = rest.find("KeyCode::") {
                let after = rest.get(at + "KeyCode::".len()..).unwrap_or("");
                if let Some(tail) = after.strip_prefix("Char('") {
                    if let Some(ch) = tail.chars().next() {
                        found.push((ch.to_string(), module));
                    }
                } else {
                    let name: String = after.chars().take_while(char::is_ascii_alphanumeric).collect();
                    if !name.is_empty() {
                        found.push((name, module));
                    }
                }
                rest = after;
            }
        }
        found.sort_unstable();
        found.dedup();
        found
    }

    #[test]
    fn every_key_the_handler_answers_to_is_described_in_the_table() {
        let described: Vec<&str> = KEYS.iter().flat_map(|b| b.codes.iter().copied()).collect();
        let undocumented: Vec<(String, &str)> = keys_the_handlers_match()
            .into_iter()
            .filter(|(key, _)| !described.contains(&key.as_str()) && !EXEMPT.contains(&key.as_str()))
            .collect();
        assert!(
            undocumented.is_empty(),
            "these bind keys the help does not mention: {undocumented:?}. \
             Add them to KEYS, or to EXEMPT with the reason."
        );
    }

    #[test]
    fn the_footer_carries_only_the_overlay_key_whatever_is_focused() {
        // It used to list every hinted key for the scope, which spent a third of a narrow row
        // restating the overlay. One item is the whole hint now, and it does not vary.
        for (focus, pending) in [(Focus::Document, false), (Focus::Document, true), (Focus::Rail, false)] {
            let shown = hint(focus, pending, usize::MAX);
            assert_eq!(shown.trim_end(), "? keys", "{focus:?}/{pending} showed {shown:?}");
        }
    }

    #[test]
    fn the_hint_disappears_rather_than_being_cut_in_half() {
        assert_eq!(hint(Focus::Document, false, usize::MAX).trim_end(), "? keys");
        // Exactly enough, and one column short of enough.
        let exact = hint(Focus::Document, false, "? keys ".width());
        assert_eq!(exact.trim_end(), "? keys");
        assert_eq!(hint(Focus::Document, false, 3), "", "no room is not room for half a key name");
        assert_eq!(hint(Focus::Document, false, 0), "");
    }
}
