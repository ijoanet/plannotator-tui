//! Per-block rendering and the row map.
//!
//! Each block is rendered in isolation through `tui-markdown` (cached, width-independent),
//! aligned back to its source bytes, and wrapped to the current column width. Every screen
//! cell therefore knows its block and, where it shows real text, its source byte.

use std::ops::Range;

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Text};
use tui_markdown::{Options, StyleSheet};

use crate::art::{Art, ArtSource};
use crate::doc::{BlockKind, Document};
use crate::render::RenderContext;
use crate::srcmap::{LineOffsets, align};
use crate::theme::Theme;
use crate::wrap::{Row, clip_line, wrap_line};

/// Rows of vertical space between blocks.
const BLOCK_GAP: usize = 1;

/// House style: no `#` markers, headings carry weight through bold/underline rather than
/// background color, so the palette stays available for selection and annotations.
#[derive(Debug, Clone)]
struct Styles(Theme);

impl StyleSheet for Styles {
    fn heading(&self, level: u8) -> Style {
        match level {
            1 => Style::new().fg(self.0.heading).add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            2 => Style::new().fg(self.0.heading).add_modifier(Modifier::BOLD),
            _ => Style::new().fg(self.0.heading_deep).add_modifier(Modifier::BOLD),
        }
    }
    fn heading_marker(&self, _level: u8) -> &'static str {
        ""
    }
    fn code(&self) -> Style {
        Style::new().fg(self.0.code)
    }
    fn link(&self) -> Style {
        Style::new().fg(self.0.link).add_modifier(Modifier::UNDERLINED)
    }
    fn blockquote(&self) -> Style {
        Style::new().fg(self.0.quote).add_modifier(Modifier::ITALIC)
    }
}

#[derive(Debug)]
pub(crate) struct RenderedBlock {
    /// Width-independent styled lines from the renderer (owned, cached).
    text: Text<'static>,
    /// Per line, per char: absolute source byte offset (cached with `text`).
    offsets: Vec<LineOffsets>,
    kind: BlockKind,
    /// The block's source bytes, so an art block can quote what its picture stands for.
    range: Range<usize>,
    /// Set when this block rendered as a picture instead of text.
    art: Option<ArtSource>,
    /// Widest rendered line, which is what decides whether the block fits.
    intrinsic_width: usize,
    /// A table's own Markdown and where it starts, to re-render it narrow at any width.
    table_source: Option<(String, usize)>,
    /// The narrow rendering for the current width, when `showing_source`.
    source_view: Option<(Text<'static>, Vec<LineOffsets>)>,
    /// Set during reflow when `source_view` is what the current width shows.
    showing_source: bool,
    /// Rows for the current width.
    pub(crate) rows: Vec<Row>,
    /// First screen row of this block in document coordinates.
    pub(crate) first_row: usize,
}

impl RenderedBlock {
    /// Art and code keep their columns; prose word-wraps. A table shown as its own Markdown
    /// wraps, because that is the point of falling back to it.
    fn preserves_columns(&self) -> bool {
        !self.showing_source && (self.art.is_some() || self.kind.preserves_columns())
    }

    /// The text and offsets the current width shows.
    fn shown(&self) -> (&Text<'static>, &Vec<LineOffsets>) {
        match (&self.source_view, self.showing_source) {
            (Some((text, offsets)), true) => (text, offsets),
            _ => (&self.text, &self.offsets),
        }
    }
}

fn widest(text: &Text<'_>) -> usize {
    text.lines.iter().map(Line::width).max().unwrap_or(0)
}

#[derive(Debug)]
pub(crate) struct DocLayout {
    pub(crate) width: usize,
    pub(crate) blocks: Vec<RenderedBlock>,
    pub(crate) total_rows: usize,
    /// Kept because a table too wide for the pane is re-rendered on resize, not at build.
    theme: Theme,
}

/// Offsets for art: one `None` per rendered character, since no character came from the
/// source. `wrap`'s cell mapping needs an entry per char, present or not.
fn art_offsets(text: &Text<'_>) -> Vec<LineOffsets> {
    text.lines
        .iter()
        .map(|line| vec![None; line.spans.iter().map(|s| s.content.chars().count()).sum()])
        .collect()
}

fn render_block(doc: &Document, index: usize, theme: Theme) -> (Text<'static>, Vec<LineOffsets>) {
    let source = doc.block_text(index);
    let text = own(tui_markdown::from_str_with_options(source, &Options::new(Styles(theme))));
    let plain: Vec<String> = text.lines.iter().map(ToString::to_string).collect();
    let base = doc.blocks.get(index).map_or(0, |b| b.range.start);
    let offsets = align(&plain, source, base);
    (text, offsets)
}

fn own(text: Text<'_>) -> Text<'static> {
    let lines = text
        .lines
        .into_iter()
        .map(|line| {
            let spans =
                line.spans.into_iter().map(|s| ratatui::text::Span::styled(s.content.into_owned(), s.style));
            Line::from(spans.collect::<Vec<_>>()).style(line.style)
        })
        .collect::<Vec<_>>();
    Text::from(lines).style(text.style)
}

impl DocLayout {
    /// Render every block once (the expensive part) and lay out for `width`.
    pub(crate) fn build(doc: &Document, width: usize, ctx: &RenderContext) -> Self {
        let mut art = crate::art::render_all(doc, width, ctx);
        let blocks = doc
            .blocks
            .iter()
            .enumerate()
            .map(|(i, block)| {
                // A block that renders as a picture skips the markdown renderer entirely.
                let (text, offsets, art) = if let Some(Art { text, source }) = art.remove(&i) {
                    let offsets = art_offsets(&text);
                    (text, offsets, Some(source))
                } else {
                    let (text, offsets) = render_block(doc, i, ctx.theme);
                    (text, offsets, None)
                };
                let intrinsic_width = widest(&text);
                // Only a table can outgrow its pane in a way that loses content; code blocks are
                // expected to be clipped and art has no smaller form.
                let table_source = (block.kind == BlockKind::Table)
                    .then(|| (doc.block_text(i).to_owned(), block.range.start));
                RenderedBlock {
                    text,
                    offsets,
                    kind: block.kind,
                    range: block.range.clone(),
                    art,
                    intrinsic_width,
                    table_source,
                    source_view: None,
                    showing_source: false,
                    rows: Vec::new(),
                    first_row: 0,
                }
            })
            .collect();
        let mut layout = Self { width: 0, blocks, total_rows: 0, theme: ctx.theme };
        layout.reflow(width);
        layout
    }

    /// Re-wrap for a new width without re-rendering markdown.
    ///
    /// An image is re-sampled from its cached thumbnail, because unlike text and diagram art
    /// its shape is chosen to fit the width.
    pub(crate) fn reflow(&mut self, width: usize) {
        let width = width.max(1);
        let resample = width != self.width;
        let theme = self.theme;
        self.width = width;
        let mut row = 0usize;
        for block in &mut self.blocks {
            block.first_row = row;
            if resample && let Some(ArtSource::Image(art)) = &block.art {
                block.text = art.to_text(width);
                block.offsets = art_offsets(&block.text);
            }
            // A table wider than the pane is re-rendered as records for this width. Cheap: only
            // tables that do not fit, only when the width changes.
            if block.intrinsic_width > width
                && let Some((source, base)) = &block.table_source
            {
                if resample || block.source_view.is_none() {
                    block.source_view = crate::table::render(source, *base, width, theme);
                }
            } else {
                block.source_view = None;
            }
            block.showing_source = block.source_view.is_some();
            let preserve = block.preserves_columns();
            let (text, offsets) = block.shown();
            let lines = text.lines.iter().zip(offsets);
            let rows: Vec<Row> = if preserve {
                lines.map(|(l, o)| clip_line(l, o, width)).collect()
            } else {
                lines.flat_map(|(l, o)| wrap_line(l, o, width)).collect()
            };
            block.rows = rows;
            row += block.rows.len() + BLOCK_GAP;
        }
        self.total_rows = row.saturating_sub(BLOCK_GAP);
    }

    /// Which block owns a document row (gap rows belong to nobody).
    pub(crate) fn block_at_row(&self, row: usize) -> Option<usize> {
        let idx = self.blocks.partition_point(|b| b.first_row <= row);
        let i = idx.checked_sub(1)?;
        let block = self.blocks.get(i)?;
        (row < block.first_row + block.rows.len()).then_some(i)
    }

    /// The row at document coordinate `row`, if it exists (None for gap rows).
    pub(crate) fn row(&self, row: usize) -> Option<&Row> {
        let block = self.blocks.get(self.block_at_row(row)?)?;
        block.rows.get(row - block.first_row)
    }

    /// First document row on which any cell falls inside `range`.
    pub(crate) fn first_row_in_range(&self, block: usize, range: &Range<usize>) -> Option<usize> {
        let b = self.blocks.get(block)?;
        b.rows
            .iter()
            .position(|r| r.cells.iter().any(|c| c.is_some_and(|o| range.contains(&o))))
            .map(|i| b.first_row + i)
    }

    /// The rendered text under a source range, in the web client's form: rendered
    /// characters whose source offset falls in `range`, blocks joined with no separator.
    ///
    /// The renderer maps a soft line break to nothing, so a break inside a wrapping block
    /// is recovered from the source: when consecutive rendered characters skip over
    /// whitespace in the source, one space is emitted (the DOM renders a soft break as a
    /// space). Code and tables keep their newlines.
    pub(crate) fn rendered_in_range(&self, source: &str, range: &Range<usize>) -> String {
        let mut out = String::new();
        for block in &self.blocks {
            // Art has no per-character source map — a diagram's box-drawing is not the
            // author's text — so it contributes the source it stands for. Without this,
            // commenting on a diagram would store an empty quote for the anchor to resolve.
            if block.art.is_some() {
                let start = block.range.start.max(range.start);
                let end = block.range.end.min(range.end);
                if start < end
                    && let Some(text) = source.get(start..end)
                {
                    out.push_str(text);
                }
                continue;
            }
            let break_char = if block.kind.preserves_columns() { '\n' } else { ' ' };
            let mut last_offset: Option<usize> = None;
            let (text, offsets) = block.shown();
            let chars = text.lines.iter().zip(offsets).flat_map(|(line, offsets)| {
                line.spans.iter().flat_map(|s| s.content.chars()).zip(offsets.iter())
            });
            for (ch, offset) in chars {
                let Some(offset) = offset.filter(|o| range.contains(o)) else { continue };
                if let Some(prev) = last_offset
                    && let Some(skipped) = source.get(prev..offset)
                    && skipped.chars().any(char::is_whitespace)
                    && !out.ends_with(char::is_whitespace)
                {
                    out.push(break_char);
                }
                out.push(ch);
                last_offset = Some(offset + ch.len_utf8());
            }
        }
        out
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::config::ArtConfig;

    #[test]
    fn rendered_text_strips_markup_and_joins_blocks_without_separator() {
        let doc = Document::parse("Ship the **login page**\nby Friday.\n\nNext para.\n".to_owned());
        let layout = DocLayout::build(&doc, 80, &RenderContext::text_only());
        let whole = 0..doc.source.len();
        assert_eq!(layout.rendered_in_range(&doc.source, &whole), "Ship the login page by Friday.Next para.");
        let bold = doc.source.find("**login").expect("present");
        let bold_range = bold..bold + "**login page**".len();
        assert_eq!(layout.rendered_in_range(&doc.source, &bold_range), "login page");
    }

    /// A scratch directory holding one opaque PNG of `size` × `size` pixels.
    fn image_dir(name: &str, size: u32) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("plannotator-tui-layout-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        let mut image = image::RgbaImage::new(size, size);
        for pixel in image.pixels_mut() {
            *pixel = image::Rgba([10, 20, 30, 255]);
        }
        image.save(dir.join("logo.png")).expect("writes png");
        dir
    }

    fn image_layout(dir: &Path, width: usize) -> (Document, DocLayout) {
        let doc = Document::parse("![logo](logo.png)\n".to_owned());
        let ctx =
            RenderContext { base_dir: dir.to_path_buf(), art: ArtConfig::default(), theme: Theme::default() };
        let layout = DocLayout::build(&doc, width, &ctx);
        (doc, layout)
    }

    #[test]
    fn an_image_paragraph_becomes_art_whose_cells_map_to_no_source_byte() {
        let dir = image_dir("art-cells", 40);
        let (_, layout) = image_layout(&dir, 80);
        let block = layout.blocks.first().expect("one block");
        assert!(block.art.is_some(), "the image paragraph rendered as art");
        assert!(
            block.rows.iter().all(|row| row.cells.iter().all(Option::is_none)),
            "art stands for the block; no cell is a source byte"
        );
        assert!(block.rows.iter().all(|row| row.line.to_string().chars().count() <= 80));
    }

    #[test]
    fn art_quotes_the_source_it_stands_for_rather_than_nothing() {
        let dir = image_dir("art-quote", 40);
        let (doc, layout) = image_layout(&dir, 80);
        // Commenting on the block must store a quote the anchor can resolve in the source.
        assert_eq!(layout.rendered_in_range(&doc.source, &(0..doc.source.len())), "![logo](logo.png)");
    }

    #[test]
    fn a_table_too_wide_for_the_pane_falls_back_to_its_own_markdown() {
        let source = "| Key | Meaning |\n|:---|:---|\n| a | a long cell that will not fit anywhere |\n";
        let doc = Document::parse(source.to_owned());
        let mut layout = DocLayout::build(&doc, 200, &RenderContext::text_only());

        let block = layout.blocks.first().expect("one block");
        assert!(!block.showing_source, "it fits at 200 columns");
        assert!(
            block.rows.iter().any(|r| r.line.to_string().contains('\u{250c}')),
            "and renders as a drawn table"
        );

        layout.reflow(30);
        let block = layout.blocks.first().expect("one block");
        assert!(block.showing_source, "it cannot fit at 30 columns");
        let shown: Vec<String> = block.rows.iter().map(|r| r.line.to_string()).collect();
        // What used to be clipped away is still on screen, and still maps to the source, so it
        // can be selected and quoted.
        assert!(shown.iter().any(|l| l.contains("anywhere")), "the tail survives: {shown:?}");
        assert!(block.rows.iter().any(|r| r.cells.iter().any(Option::is_some)), "and is selectable");
        assert!(shown.iter().all(|l| l.chars().count() <= 30), "and fits: {shown:?}");
    }

    #[test]
    fn a_narrower_width_re_samples_the_image_instead_of_clipping_it() {
        let dir = image_dir("art-reflow", 40);
        let (_, mut layout) = image_layout(&dir, 80);
        // 40 square pixels at two per row: 40 columns, 20 rows.
        assert_eq!(layout.blocks.first().map(|b| b.rows.len()), Some(20));
        layout.reflow(20);
        let block = layout.blocks.first().expect("one block");
        assert_eq!(block.rows.len(), 10, "half the columns is half the rows, aspect ratio kept");
        assert!(block.rows.iter().all(|row| row.line.to_string().chars().count() <= 20));
    }
}
