//! Driving the app with no terminal attached: `--bench`, the snapshot harness and the
//! scripting subcommands in `cli`.
//!
//! No keypress reaches anything here. These put the app in a state a test or a benchmark wants
//! to measure, by the same paths input takes - `annotate`, `scroll_by`, `finish_selection` -
//! so what they set up is what a reviewer would have.

use anyhow::Result;
use plannotator_tui_schema::Kind;

use super::App;
use super::selection::Selection;

impl App {
    /// Annotate a whole block by index.
    pub(crate) fn add_block_annotation(&mut self, block: usize, kind: Kind, body: String) -> Result<()> {
        let range = self.open.doc.blocks.get(block).map(|b| b.range.clone());
        let range = range.ok_or_else(|| {
            anyhow::anyhow!("block {block} out of range ({} blocks)", self.open.doc.blocks.len())
        })?;
        self.annotate(range, kind, body)
    }

    /// Annotate the first occurrence of `quote` in the source.
    pub(crate) fn add_quote_annotation(&mut self, quote: &str, kind: Kind, body: String) -> Result<()> {
        let start =
            self.open.doc.source.find(quote).ok_or_else(|| anyhow::anyhow!("quote not found: {quote:?}"))?;
        self.annotate(start..start + quote.len(), kind, body)
    }

    pub(crate) fn describe_blocks(&self) -> Vec<String> {
        self.open
            .doc
            .blocks
            .iter()
            .zip(&self.open.layout.blocks)
            .enumerate()
            .map(|(i, (block, rendered))| {
                let first = self.open.doc.block_text(i).lines().next().unwrap_or("");
                let head: String = first.chars().take(60).collect();
                format!("{i:4} {:<10} row {:>5}  {head}", format!("{:?}", block.kind), rendered.first_row)
            })
            .collect()
    }

    /// Scroll and select the first visible block.
    pub(crate) fn scroll_for_snapshot(&mut self, delta: i64) {
        self.scroll_by(delta);
        if let Some(block) = self.open.layout.block_at_row(self.scroll) {
            self.selected = block;
        }
    }

    /// Simulate a finished drag over the first occurrence of `quote`.
    pub(crate) fn select_quote_for_snapshot(&mut self, quote: &str) -> Result<()> {
        let start =
            self.open.doc.source.find(quote).ok_or_else(|| anyhow::anyhow!("quote not found: {quote:?}"))?;
        let range = start..start + quote.len();
        let cells = self.open.layout.blocks.iter().flat_map(|b| {
            b.rows.iter().enumerate().flat_map(move |(ri, row)| {
                row.cells.iter().enumerate().map(move |(col, cell)| ((b.first_row + ri, col), *cell))
            })
        });
        let hits: Vec<_> =
            cells.filter(|(_, cell)| cell.is_some_and(|o| range.contains(&o))).map(|(pos, _)| pos).collect();
        let first = *hits.first().ok_or_else(|| anyhow::anyhow!("quote is not rendered"))?;
        let last = hits.last().copied().unwrap_or(first);
        self.selection = Some(Selection::finished(first, last));
        self.finish_selection();
        Ok(())
    }
}
