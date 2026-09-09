//! Tables narrow enough to read: columns shrunk to the pane and cell text wrapped inside them.
//!
//! `tui-markdown` sizes columns from content alone (`max(cell.width())`) with no notion of the
//! space available, and offers no width option, so a table wider than the pane cannot be asked to
//! fit. It used to be clipped - the tail of every row and the right border silently gone, and
//! clipped cells could not even be selected.
//!
//! So the table is laid out here instead: shrink the widest columns until the whole thing fits,
//! then wrap each cell's text inside its column. The table stays a table.
//!
//! Cells come from `pulldown-cmark`'s event stream with source ranges, not from string matching,
//! and every rendered character keeps the byte it came from, so a selection inside a wide table
//! works.
//!
//! When a pane is too narrow even for `MIN_COLUMN` per column, one record per row is the only
//! readable shape left, so `records` takes over.

use pulldown_cmark::{Event, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Text};

use crate::doc::parse_options;
use crate::srcmap::LineOffsets;
use crate::theme::Theme;
use crate::wrap::Painted;

/// A cell's characters, each with the source byte it came from.
type Cell = Vec<(char, Option<usize>)>;

/// Narrowest a column may be squeezed to before a table stops being worth drawing.
const MIN_COLUMN: usize = 6;
/// Per column: a border, a leading space and a trailing space. Plus one closing border.
const COLUMN_CHROME: usize = 3;
/// Rows of vertical space between records, in the fallback shape.
const RECORD_GAP: usize = 1;
/// A label needs this much room left over before it shares a line with its value.
const LABEL_HEADROOM: usize = 12;

/// Render one table block to fit `width`, as a table if it can be, else as records.
pub(crate) fn render(
    source: &str,
    base: usize,
    width: usize,
    theme: Theme,
) -> Option<(Text<'static>, Vec<LineOffsets>)> {
    let (headers, rows) = collect(source, base);
    if rows.is_empty() {
        return None;
    }
    let columns = headers.len().max(rows.iter().map(Vec::len).max().unwrap_or(0));
    if columns == 0 {
        return None;
    }
    let chrome = COLUMN_CHROME * columns + 1;
    let available = width.saturating_sub(chrome);
    if available < MIN_COLUMN * columns {
        // Too many columns for this pane: a drawn table would be unreadable at any split.
        return records(&headers, &rows, width, theme);
    }
    let widths = shrink(&natural_widths(&headers, &rows, columns), available);
    Some(draw(&headers, &rows, &widths, theme))
}

/// Widest cell in each column, header included.
fn natural_widths(headers: &[Cell], rows: &[Vec<Cell>], columns: usize) -> Vec<usize> {
    let mut widths = vec![1usize; columns];
    for row in std::iter::once(headers).chain(rows.iter().map(Vec::as_slice)) {
        for (column, cell) in row.iter().enumerate() {
            if let Some(width) = widths.get_mut(column) {
                *width = (*width).max(cell_width(cell));
            }
        }
    }
    widths
}

/// Take width off the widest column, one at a time, until the row fits.
///
/// One column at a time rather than a proportional formula: a table is a handful of columns, and
/// "always shrink the widest" is a rule a reader can check against the result.
fn shrink(natural: &[usize], available: usize) -> Vec<usize> {
    let mut widths = natural.to_vec();
    while widths.iter().sum::<usize>() > available {
        let Some(widest) = widths.iter().copied().enumerate().max_by_key(|&(i, w)| (w, usize::MAX - i))
        else {
            break;
        };
        if widest.1 <= MIN_COLUMN {
            break;
        }
        if let Some(width) = widths.get_mut(widest.0) {
            *width -= 1;
        }
    }
    widths
}

fn draw(
    headers: &[Cell],
    rows: &[Vec<Cell>],
    widths: &[usize],
    theme: Theme,
) -> (Text<'static>, Vec<LineOffsets>) {
    let border = Style::from(theme.border);
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut offsets: Vec<LineOffsets> = Vec::new();

    push(&mut lines, &mut offsets, &rule(widths, ['┌', '┬', '┐'], border));
    let header_style = Style::from(theme.text).add_modifier(Modifier::BOLD);
    for line in body(headers, widths, header_style, border) {
        push(&mut lines, &mut offsets, &line);
    }
    push(&mut lines, &mut offsets, &rule(widths, ['├', '┼', '┤'], border));
    for row in rows {
        for line in body(row, widths, Style::from(theme.text), border) {
            push(&mut lines, &mut offsets, &line);
        }
    }
    push(&mut lines, &mut offsets, &rule(widths, ['└', '┴', '┘'], border));
    (Text::from(lines), offsets)
}

/// A horizontal rule: `[left, junction, right]` around each column's dashes.
fn rule(widths: &[usize], corners: [char; 3], border: Style) -> Vec<Painted> {
    let [left, mid, right] = corners;
    let mut painted: Vec<Painted> = vec![(left, None, border)];
    for (index, width) in widths.iter().enumerate() {
        if index > 0 {
            painted.push((mid, None, border));
        }
        painted.extend(std::iter::repeat_n(('─', None, border), width + 2));
    }
    painted.push((right, None, border));
    painted
}

/// One table row as however many screen lines its tallest wrapped cell needs.
fn body(row: &[Cell], widths: &[usize], text: Style, border: Style) -> Vec<Vec<Painted>> {
    let columns: Vec<Vec<Cell>> = widths
        .iter()
        .enumerate()
        .map(|(index, width)| wrap(row.get(index).map_or(&[][..], Vec::as_slice), *width))
        .collect();
    let height = columns.iter().map(Vec::len).max().unwrap_or(1).max(1);

    (0..height)
        .map(|line| {
            let mut painted: Vec<Painted> = Vec::new();
            for (column, width) in widths.iter().enumerate() {
                painted.push(('│', None, border));
                painted.push((' ', None, text));
                let part = columns.get(column).and_then(|c| c.get(line));
                let mut used = 0usize;
                if let Some(part) = part {
                    for &(ch, at) in part {
                        painted.push((ch, at, text));
                        used += display_width(ch);
                    }
                }
                painted.extend(std::iter::repeat_n((' ', None, text), (width + 1).saturating_sub(used)));
            }
            painted.push(('│', None, border));
            painted
        })
        .collect()
}

/// Add one line, merging equal-styled runs into spans.
fn push(lines: &mut Vec<Line<'static>>, offsets: &mut Vec<LineOffsets>, painted: &[Painted]) {
    let (line, map) = crate::wrap::paint(painted);
    lines.push(line);
    offsets.push(map);
}

fn display_width(ch: char) -> usize {
    crate::wrap::display_width(ch)
}

fn cell_width(cell: &Cell) -> usize {
    cell.iter().map(|&(ch, _)| display_width(ch)).sum()
}

/// Greedy word wrap over characters that carry their source offset.
fn wrap(cell: &[(char, Option<usize>)], width: usize) -> Vec<Cell> {
    let width = width.max(1);
    let mut out: Vec<Cell> = Vec::new();
    let mut line: Cell = Vec::new();
    let mut used = 0usize;
    for word in split_words(cell) {
        let word_width: usize = word.iter().map(|&(ch, _)| display_width(ch)).sum();
        if !line.is_empty() && used + 1 + word_width > width {
            out.push(std::mem::take(&mut line));
            used = 0;
        }
        if !line.is_empty() {
            line.push((' ', None));
            used += 1;
        }
        if word_width > width {
            // A word wider than the column: break it rather than overflow.
            for ch in word {
                if used + display_width(ch.0) > width && !line.is_empty() {
                    out.push(std::mem::take(&mut line));
                    used = 0;
                }
                used += display_width(ch.0);
                line.push(ch);
            }
        } else {
            line.extend(word);
            used += word_width;
        }
    }
    if !line.is_empty() {
        out.push(line);
    }
    out
}

fn split_words(cell: &[(char, Option<usize>)]) -> Vec<Cell> {
    let mut words: Vec<Cell> = Vec::new();
    let mut word: Cell = Vec::new();
    for &(ch, at) in cell {
        if ch.is_whitespace() {
            if !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
        } else {
            word.push((ch, at));
        }
    }
    if !word.is_empty() {
        words.push(word);
    }
    words
}

/// One record per row, for a pane too narrow to draw any table in.
fn records(
    headers: &[Cell],
    rows: &[Vec<Cell>],
    width: usize,
    theme: Theme,
) -> Option<(Text<'static>, Vec<LineOffsets>)> {
    let width = width.max(8);
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut offsets: Vec<LineOffsets> = Vec::new();
    let label_style = Style::from(theme.muted);
    let text_style = Style::from(theme.text);

    for (index, row) in rows.iter().enumerate() {
        if index > 0 {
            for _ in 0..RECORD_GAP {
                lines.push(Line::default());
                offsets.push(Vec::new());
            }
        }
        for (column, cell) in row.iter().enumerate() {
            let label: String = headers
                .get(column)
                .map_or_else(|| (column + 1).to_string(), |c| c.iter().map(|(ch, _)| *ch).collect());
            let prefix = format!("{label}: ");
            let inline = width > prefix.chars().count() + LABEL_HEADROOM;
            if !inline {
                let label: Vec<Painted> = prefix.trim_end().chars().map(|c| (c, None, label_style)).collect();
                push(&mut lines, &mut offsets, &label);
            }
            let body_width = if inline { width - prefix.chars().count() } else { width - 2 };
            let wrapped = wrap(cell, body_width.max(1));
            for (line, part) in wrapped.iter().enumerate() {
                let mut painted: Vec<Painted> = if inline && line == 0 {
                    prefix.chars().map(|c| (c, None, label_style)).collect()
                } else {
                    vec![(' ', None, text_style), (' ', None, text_style)]
                };
                painted.extend(part.iter().map(|&(ch, at)| (ch, at, text_style)));
                push(&mut lines, &mut offsets, &painted);
            }
        }
    }
    (!lines.is_empty()).then(|| (Text::from(lines), offsets))
}

/// Header cells and body rows, each character paired with its source byte.
fn collect(source: &str, base: usize) -> (Vec<Cell>, Vec<Vec<Cell>>) {
    let mut headers: Vec<Cell> = Vec::new();
    let mut rows: Vec<Vec<Cell>> = Vec::new();
    let mut row: Vec<Cell> = Vec::new();
    let mut cell: Cell = Vec::new();
    let mut in_head = false;

    for (event, range) in Parser::new_ext(source, parse_options()).into_offset_iter() {
        match event {
            Event::Start(Tag::TableHead) => in_head = true,
            Event::End(TagEnd::TableHead) => {
                in_head = false;
                headers = std::mem::take(&mut row);
            }
            Event::Start(Tag::TableRow) => row.clear(),
            Event::End(TagEnd::TableRow) => {
                if !in_head {
                    rows.push(std::mem::take(&mut row));
                }
            }
            Event::Start(Tag::TableCell) => cell.clear(),
            Event::End(TagEnd::TableCell) => row.push(std::mem::take(&mut cell)),
            // Text maps to the source byte for byte; anything else (an escape, an entity) would
            // not, so those characters map to nothing rather than to the wrong byte.
            Event::Text(text) => {
                let exact = range.len() == text.len();
                cell.extend(text.char_indices().map(|(i, ch)| (ch, exact.then_some(base + range.start + i))));
            }
            // Inline code's range includes its backticks; the text starts one byte in.
            Event::Code(code) => {
                let exact = range.len() == code.len() + 2;
                cell.extend(
                    code.char_indices().map(|(i, ch)| (ch, exact.then_some(base + range.start + 1 + i))),
                );
            }
            Event::SoftBreak | Event::HardBreak => cell.push((' ', None)),
            _ => {}
        }
    }
    (headers, rows)
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    const TABLE: &str =
        "| Env | Detail |\n|---|---|\n| Dev | a fairly long description here |\n| QA | short |\n";

    fn plain(text: &Text<'_>) -> Vec<String> {
        text.lines.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn a_narrow_table_is_still_a_table_with_its_cells_wrapped() {
        let (text, _) = render(TABLE, 0, 34, Theme::default()).expect("renders");
        let shown = plain(&text);
        assert!(shown.first().is_some_and(|l| l.starts_with('┌')), "still drawn: {shown:?}");
        assert!(shown.last().is_some_and(|l| l.starts_with('└')), "and closed: {shown:?}");
        // The long cell wrapped inside its column instead of being cut off.
        let joined = shown.join(" ");
        for word in ["fairly", "long", "description", "here"] {
            assert!(joined.contains(word), "{word} survived: {shown:?}");
        }
    }

    #[test]
    fn every_line_fits_the_width_exactly() {
        for width in [24, 30, 40, 60, 100] {
            let (text, offsets) = render(TABLE, 0, width, Theme::default()).expect("renders");
            for (index, line) in text.lines.iter().enumerate() {
                assert!(line.width() <= width, "line {index} at width {width}: {line:?}");
                // One offset entry per display column, which the selection map relies on.
                assert_eq!(offsets.get(index).map(Vec::len), Some(line.width()));
            }
        }
    }

    #[test]
    fn text_keeps_the_source_byte_it_came_from() {
        let (text, offsets) = render(TABLE, 100, 40, Theme::default()).expect("renders");
        let row = plain(&text).iter().position(|l| l.contains("Dev")).expect("the Dev row");
        let map = offsets.get(row).expect("offsets for it");
        let at = map.iter().flatten().next().copied().expect("a mapped character");
        assert_eq!(TABLE.get(at - 100..at - 100 + 3), Some("Dev"));
    }

    #[test]
    fn too_many_columns_for_the_pane_become_one_record_per_row() {
        let wide = "| a | b | c | d | e | f |\n|---|---|---|---|---|---|\n| 1 | 2 | 3 | 4 | 5 | 6 |\n";
        let (text, _) = render(wide, 0, 30, Theme::default()).expect("renders");
        let shown = plain(&text);
        assert!(shown.iter().all(|l| !l.starts_with('┌')), "no table drawn: {shown:?}");
        assert!(shown.iter().any(|l| l.starts_with("a: 1")), "labelled records: {shown:?}");
    }

    #[test]
    fn a_table_with_no_body_rows_has_nothing_to_lay_out() {
        assert!(render("| a | b |\n|---|---|\n", 0, 40, Theme::default()).is_none());
    }
}
