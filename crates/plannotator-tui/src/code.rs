//! Code blocks laid out here rather than by `tui-markdown`.
//!
//! A code block preserves its columns, so `tui-markdown`'s rendering could only be clipped: a
//! 123-character command showed 58 characters at 60 columns, the rest unreachable and, because
//! clipping drops the cell map too, unselectable. A truncated command that looks complete is
//! something a reader might copy and run, which is the reason this module exists.
//!
//! So a long line is **wrapped** instead, broken exactly at the column edge. No character is
//! inserted, removed or moved, so a copy of the wrapped rows is the original command; only the
//! break positions are ours. A [`CONTINUATION`] marker distinguishes "this line continues" from
//! "the next line starts", which matters when a block holds several commands.
//!
//! The fence is not shown (see `layout`'s `code_block_fence`), so the language moves to a label
//! above the block and a rule down its left marks how far the block extends.
//!
//! Like `table.rs`, this is a layout and not a second parser: the text and its source ranges come
//! from `pulldown-cmark`'s event stream, and every rendered character keeps the byte it came from
//! so a wrapped command stays selectable.

mod highlight;

use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Text};

use crate::code::highlight::Token;
use crate::doc::parse_options;
use crate::srcmap::LineOffsets;
use crate::theme::Theme;
use crate::wrap::{Painted, display_width, paint};

/// A character, the source byte it came from, and what it means.
type Char = (char, Option<usize>, Token);

/// Drawn down the left of every code row, so the block's extent is visible without a fence.
const RULE: char = '│';
/// Marks a row that continues the line above rather than starting a new one.
const CONTINUATION: char = '↳';
/// `│ ` before a code row.
const RULE_CHROME: usize = 2;
/// `│ ↳ ` before a continuation row.
const CONTINUATION_CHROME: usize = 4;

/// One code block, parsed once and laid out per width.
///
/// Held rather than re-parsed on resize because parsing and highlighting are width-independent;
/// only the wrapping is not.
#[derive(Debug)]
pub(crate) struct CodeBlock {
    /// The fence's info string. Shown as a label, since the fence itself is hidden.
    language: Option<String>,
    lines: Vec<Vec<Char>>,
}

/// Parse one code block, or `None` if `source` holds no code block.
pub(crate) fn parse(source: &str, base: usize) -> Option<CodeBlock> {
    let mut language = None;
    let mut inside = false;
    let mut chars: Vec<(char, Option<usize>)> = Vec::new();

    for (event, range) in Parser::new_ext(source, parse_options()).into_offset_iter() {
        match event {
            Event::Start(Tag::CodeBlock(kind)) => {
                inside = true;
                // An indented block has no info string, only a fenced one does.
                if let CodeBlockKind::Fenced(info) = kind {
                    let word = info.split_whitespace().next().unwrap_or_default();
                    if !word.is_empty() {
                        language = Some(word.to_owned());
                    }
                }
            }
            // Code text is verbatim, so its range matches byte for byte. Anything else would not,
            // and maps to nothing rather than to the wrong byte.
            Event::Text(text) if inside => {
                let exact = range.len() == text.len();
                chars
                    .extend(text.char_indices().map(|(i, ch)| (ch, exact.then_some(base + range.start + i))));
            }
            Event::End(TagEnd::CodeBlock) => break,
            _ => {}
        }
    }
    if !inside {
        return None;
    }
    let lines = split_lines(&chars);
    Some(CodeBlock { lines: highlighted(language.as_deref(), lines), language })
}

/// Attach a token to every character, from syntect where the language is known.
///
/// A language nobody names, or one syntect has no syntax for, leaves the block plain rather than
/// guessing: unhighlighted code still reads, mis-highlighted code misleads.
fn highlighted(language: Option<&str>, lines: Vec<Vec<(char, Option<usize>)>>) -> Vec<Vec<Char>> {
    let plain: Vec<String> = lines.iter().map(|line| line.iter().map(|&(ch, _)| ch).collect()).collect();
    let tokens = language.and_then(|language| highlight::classify(language, &plain));
    lines
        .into_iter()
        .enumerate()
        .map(|(index, line)| {
            let row = tokens.as_ref().and_then(|t| t.get(index));
            line.into_iter()
                .enumerate()
                .map(|(column, (ch, at))| {
                    (ch, at, row.and_then(|r| r.get(column)).copied().unwrap_or(Token::Plain))
                })
                .collect()
        })
        .collect()
}

/// Split verbatim code into lines, dropping the newlines themselves.
///
/// Code text ends with a newline, which would otherwise leave a blank row at the bottom of every
/// block.
fn split_lines(chars: &[(char, Option<usize>)]) -> Vec<Vec<(char, Option<usize>)>> {
    let mut lines: Vec<Vec<(char, Option<usize>)>> = Vec::new();
    let mut line: Vec<(char, Option<usize>)> = Vec::new();
    for &(ch, at) in chars {
        if ch == '\n' {
            lines.push(std::mem::take(&mut line));
        } else {
            line.push((ch, at));
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

impl CodeBlock {
    /// Lay the block out for `width` columns.
    pub(crate) fn to_text(&self, width: usize, theme: Theme) -> (Text<'static>, Vec<LineOffsets>) {
        let rule = Style::from(theme.code_block_border);
        let text = Style::from(theme.code_block);
        let label_style = Style::from(theme.code_block_border).add_modifier(Modifier::DIM);
        let mut lines: Vec<Line<'static>> = Vec::new();
        let mut offsets: Vec<LineOffsets> = Vec::new();

        if let Some(language) = &self.language {
            // The label is ours, not the author's text, so it maps to no source byte.
            let painted: Vec<Painted> = language.chars().map(|ch| (ch, None, label_style)).collect();
            push(&mut lines, &mut offsets, &painted);
        }

        let first = width.saturating_sub(RULE_CHROME).max(1);
        let rest = width.saturating_sub(CONTINUATION_CHROME).max(1);
        for line in &self.lines {
            let parts = wrap(line, first, rest);
            for (index, part) in parts.iter().enumerate() {
                let mut painted: Vec<Painted> = vec![(RULE, None, rule), (' ', None, rule)];
                if index > 0 {
                    painted.push((CONTINUATION, None, rule));
                    painted.push((' ', None, rule));
                }
                painted.extend(part.iter().map(|&(ch, at, token)| (ch, at, style(token, theme, text))));
                push(&mut lines, &mut offsets, &painted);
            }
            if parts.is_empty() {
                // A blank line in the source is still part of the block: keep the rule going.
                push(&mut lines, &mut offsets, &[(RULE, None, rule)]);
            }
        }
        (Text::from(lines), offsets)
    }
}

/// The palette entry for what a character means. `Plain` is the block's body text, so an
/// unhighlighted block looks exactly as it did before highlighting existed.
fn style(token: Token, theme: Theme, plain: Style) -> Style {
    let color = match token {
        Token::Plain => return plain,
        Token::Comment => theme.syntax_comment,
        Token::Keyword => theme.syntax_keyword,
        Token::Function => theme.syntax_function,
        Token::Variable => theme.syntax_variable,
        Token::Str => theme.syntax_string,
        Token::Number => theme.syntax_number,
        Token::Type => theme.syntax_type,
        Token::Operator => theme.syntax_operator,
        Token::Punctuation => theme.syntax_punctuation,
    };
    Style::from(color)
}

fn push(lines: &mut Vec<Line<'static>>, offsets: &mut Vec<LineOffsets>, painted: &[Painted]) {
    let (line, map) = paint(painted);
    lines.push(line);
    offsets.push(map);
}

/// Break a line at the column edge, `first` columns for its first row and `rest` after.
///
/// A hard break, not a word wrap: breaking at spaces would read as if a quoted string had ended,
/// and would make the rows something other than the characters that were there.
fn wrap(line: &[Char], first: usize, rest: usize) -> Vec<Vec<Char>> {
    let mut rows: Vec<Vec<Char>> = Vec::new();
    let mut row: Vec<Char> = Vec::new();
    let mut used = 0usize;
    for &(ch, at, token) in line {
        let budget = if rows.is_empty() { first } else { rest };
        let ch_width = display_width(ch);
        if used + ch_width > budget && !row.is_empty() {
            rows.push(std::mem::take(&mut row));
            used = 0;
        }
        used += ch_width;
        row.push((ch, at, token));
    }
    if !row.is_empty() {
        rows.push(row);
    }
    rows
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    const LONG: &str = "gh pr list --state open --limit 60 --json number,title,author,headRefName --jq '.[]'";
    const BASH: &str = "```bash\ngh pr list --limit 60\necho short\n```";

    fn shown(source: &str, width: usize) -> Vec<String> {
        let (text, _) = parse(source, 0).expect("a code block").to_text(width, Theme::default());
        text.lines.iter().map(ToString::to_string).collect()
    }

    /// Strip exactly the chrome this module adds: the rule, and the marker on a continued row.
    ///
    /// Exactly one space each, never `trim_start`, because a break can fall so that wrapped
    /// content legitimately begins with a space and trimming would hide a lost character.
    fn strip_chrome(row: &str, continued: bool) -> &str {
        let row = row.strip_prefix(RULE).unwrap_or(row);
        let row = row.strip_prefix(' ').unwrap_or(row);
        if !continued {
            return row;
        }
        let row = row.strip_prefix(CONTINUATION).unwrap_or(row);
        row.strip_prefix(' ').unwrap_or(row)
    }

    #[test]
    fn a_fence_renders_no_backticks_and_shows_its_language() {
        let rows = shown(BASH, 40);
        assert!(rows.iter().all(|r| !r.contains("```")), "no fence: {rows:?}");
        assert_eq!(rows.first().map(String::as_str), Some("bash"), "the language became a label");
        assert!(rows.iter().any(|r| r.contains("echo short")), "the code is there: {rows:?}");
    }

    #[test]
    fn an_indented_block_has_no_language_to_label() {
        let rows = shown("    indented code\n", 40);
        assert!(rows.iter().all(|r| r.contains('│')), "every row is ruled: {rows:?}");
        assert!(rows.iter().any(|r| r.contains("indented code")), "{rows:?}");
    }

    #[test]
    fn a_line_longer_than_the_pane_wraps_and_loses_nothing() {
        let source = format!("```bash\n{LONG}\n```");
        for width in [20, 40, 60, 200] {
            let rows = shown(&source, width);
            assert!(
                rows.iter().all(|r| r.chars().count() <= width),
                "nothing overflows at {width}: {rows:?}"
            );
            // Strip the rule and the continuation marker: what is left must be the command, whole
            // and in order, because a wrapped command is one a reader may copy.
            let rebuilt: String =
                rows.iter().skip(1).enumerate().map(|(index, row)| strip_chrome(row, index > 0)).collect();
            assert_eq!(rebuilt, LONG, "at width {width}");
        }
    }

    #[test]
    fn only_a_continued_row_carries_the_marker() {
        let source = format!("```bash\n{LONG}\n```");
        let rows = shown(&source, 40);
        let code: Vec<&String> = rows.iter().skip(1).collect();
        let [first, rest @ ..] = code.as_slice() else {
            unreachable!("a code block always renders at least one row")
        };
        assert!(!rest.is_empty(), "the line took more than one row: {rows:?}");
        assert!(!first.contains(CONTINUATION), "the first row starts the line: {first:?}");
        assert!(rest.iter().all(|r| r.contains(CONTINUATION)), "the rest continue it: {rest:?}");
    }

    #[test]
    fn a_wrapped_character_keeps_the_source_byte_it_came_from() {
        let source = format!("```bash\n{LONG}\n```");
        let base = 500;
        let (text, offsets) = parse(&source, base).expect("a code block").to_text(40, Theme::default());
        // One entry per rendered character, which is what `LineOffsets` means. Asserting the
        // column count here is what let a wide-character shift through; the invariant that a
        // column maps to the character drawn at it lives in `layout`'s tests.
        for (index, line) in text.lines.iter().enumerate() {
            let chars: usize = line.spans.iter().map(|s| s.content.chars().count()).sum();
            assert_eq!(offsets.get(index).map(Vec::len), Some(chars), "row {index}");
        }
        // A character on a continuation row still points at its own byte in the source.
        let last = offsets.last().expect("rows");
        let at = last.iter().flatten().next().copied().expect("a mapped character");
        let shown_char = source.get(at - base..).and_then(|s| s.chars().next()).expect("in source");
        let row = text.lines.last().expect("a row").to_string();
        let first_code_char = strip_chrome(&row, true);
        assert_eq!(
            first_code_char.chars().next(),
            Some(shown_char),
            "the offset points at the character on screen"
        );
    }

    #[test]
    fn a_blank_line_inside_a_block_keeps_the_rule() {
        let rows = shown("```\nfirst\n\nthird\n```", 40);
        assert_eq!(rows.len(), 3, "three code rows, the middle one blank: {rows:?}");
        assert!(rows.iter().all(|r| r.starts_with(RULE)), "the block reads as continuous: {rows:?}");
    }

    #[test]
    fn a_highlighted_block_paints_tokens_in_their_own_colors() {
        let theme = Theme::default();
        let (text, _) = parse("```bash\necho hi # note\n```", 0).expect("a code block").to_text(60, theme);
        let row = text.lines.get(1).expect("the code row");
        let colors: Vec<_> = row.spans.iter().filter_map(|s| s.style.fg).collect();
        assert!(colors.contains(&theme.syntax_comment), "the comment took its own color: {colors:?}");
        assert!(colors.contains(&theme.syntax_function), "the command took its own color: {colors:?}");
    }

    #[test]
    fn an_unknown_language_leaves_the_block_in_body_text() {
        let theme = Theme::default();
        let (text, _) = parse("```not-a-language\necho hi\n```", 0).expect("a code block").to_text(60, theme);
        let row = text.lines.get(1).expect("the code row");
        let colors: Vec<_> = row.spans.iter().filter_map(|s| s.style.fg).collect();
        assert!(
            colors.iter().all(|c| *c == theme.code_block || *c == theme.code_block_border),
            "nothing was guessed at: {colors:?}"
        );
    }

    #[test]
    fn prose_is_not_a_code_block() {
        assert!(parse("just a paragraph\n", 0).is_none());
    }
}
