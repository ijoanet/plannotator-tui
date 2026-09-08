//! Blocks that render as art instead of text: Mermaid diagrams and images.
//!
//! A normal block is rendered by `tui-markdown` and every rendered character maps back to a
//! source byte (see `srcmap`). Art has no such mapping: the picture stands for the whole
//! block. So an art block keeps its source range but no per-cell offsets, never word-wraps,
//! and quotes its source text when something selects across it.
//!
//! Detection uses `pulldown-cmark`, never string matching on markup: the fence's info string
//! and the image destination come from the event stream, as everywhere else in this crate.

mod image;
mod mermaid;
mod obsidian;

use std::collections::HashMap;

use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag, TagEnd};
use ratatui::text::Text;

use crate::doc::{BlockKind, Document, parse_options};
use crate::render::RenderContext;

pub(crate) use self::image::ImageArt;

/// What an art block was made from, and what a resize needs in order to redo it.
#[derive(Debug)]
pub(crate) enum ArtSource {
    /// Mermaid art has an intrinsic width; a narrower terminal clips it.
    Mermaid,
    /// An image is re-sampled to the current width from a bounded thumbnail.
    Image(ImageArt),
}

/// One rendered picture: the styled rows and what could redo them at another width.
#[derive(Debug)]
pub(crate) struct Art {
    pub(crate) text: Text<'static>,
    pub(crate) source: ArtSource,
}

/// Render every block of `doc` that becomes a picture, keyed by block index.
///
/// One pass over the whole document rather than a call per block, because all of its Mermaid
/// diagrams are rendered by a single Node process. `width` is the column budget; only images
/// use it.
pub(crate) fn render_all(doc: &Document, width: usize, ctx: &RenderContext) -> HashMap<usize, Art> {
    let mut art = HashMap::new();

    if ctx.art.image.enabled {
        // The vault index is built at most once per document, and only if an embed needs it.
        let mut vault: Option<obsidian::Vault> = None;
        for (index, source) in blocks_of_kind(doc, BlockKind::Paragraph) {
            let Some(path) = image_path(source, ctx, &mut vault) else { continue };
            let Some(image) = image::load(&path, &ctx.art.image) else { continue };
            let text = image.to_text(width);
            art.insert(index, Art { text, source: ArtSource::Image(image) });
        }
    }

    if ctx.art.mermaid.enabled {
        let (indices, sources): (Vec<usize>, Vec<String>) = blocks_of_kind(doc, BlockKind::CodeBlock)
            .filter_map(|(index, source)| mermaid_code(source).map(|code| (index, code)))
            .unzip();
        for (index, text) in
            indices.into_iter().zip(mermaid::render_all(&sources, &ctx.art.mermaid, ctx.theme))
        {
            if let Some(text) = text {
                art.insert(index, Art { text, source: ArtSource::Mermaid });
            }
        }
    }

    art
}

/// The image file a paragraph shows, whether written as Markdown or as an Obsidian embed.
fn image_path(
    source: &str,
    ctx: &RenderContext,
    vault: &mut Option<obsidian::Vault>,
) -> Option<std::path::PathBuf> {
    if let Some(url) = single_image_url(source) {
        return image::local_path(&url, &ctx.base_dir);
    }
    if !ctx.art.image.obsidian_embeds {
        return None;
    }
    let target = obsidian::embed_target(source)?;
    vault.get_or_insert_with(|| obsidian::Vault::around(&ctx.base_dir)).resolve(target, &ctx.base_dir)
}

/// Index and source text of every block of `kind`.
fn blocks_of_kind(doc: &Document, kind: BlockKind) -> impl Iterator<Item = (usize, &str)> {
    doc.blocks
        .iter()
        .enumerate()
        .filter(move |(_, block)| block.kind == kind)
        .map(|(index, _)| (index, doc.block_text(index)))
}

/// The code inside a fenced block whose info string names Mermaid, else `None`.
///
/// The info string may carry more words (`mermaid title="x"`); the language is the first.
fn mermaid_code(source: &str) -> Option<String> {
    let mut inside = false;
    let mut code = String::new();
    for event in Parser::new_ext(source, parse_options()) {
        match event {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info))) => {
                let language = info.split_whitespace().next().unwrap_or_default();
                if !language.eq_ignore_ascii_case("mermaid") {
                    return None;
                }
                inside = true;
            }
            Event::Text(text) if inside => code.push_str(&text),
            Event::End(TagEnd::CodeBlock) => break,
            _ => {}
        }
    }
    inside.then_some(code)
}

/// The destination of the only image in a paragraph that holds nothing else.
///
/// A paragraph mixing an image with real text keeps its normal rendering: the text is
/// content someone may want to quote.
fn single_image_url(source: &str) -> Option<String> {
    let mut url: Option<String> = None;
    let mut depth = 0usize;
    for event in Parser::new_ext(source, parse_options()) {
        match event {
            Event::Start(Tag::Image { dest_url, .. }) => {
                if url.is_some() {
                    return None; // a second image: not a single picture
                }
                url = Some(dest_url.into_string());
                depth += 1;
            }
            Event::End(TagEnd::Image) => depth = depth.saturating_sub(1),
            // Alt text lives inside the image and does not count as content.
            Event::Text(text) if depth == 0 && !text.trim().is_empty() => return None,
            // Anything else at paragraph level is content someone may want to quote.
            Event::Code(_)
            | Event::InlineMath(_)
            | Event::DisplayMath(_)
            | Event::Start(Tag::Link { .. })
                if depth == 0 =>
            {
                return None;
            }
            _ => {}
        }
    }
    url
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    #[test]
    fn mermaid_fence_yields_its_code_and_other_languages_do_not() {
        assert_eq!(mermaid_code("```mermaid\ngraph TD\n A-->B\n```").as_deref(), Some("graph TD\n A-->B\n"));
        assert_eq!(mermaid_code("```rust\nfn x() {}\n```"), None);
        assert_eq!(mermaid_code("```\nplain\n```"), None);
    }

    #[test]
    fn mermaid_info_string_is_case_insensitive_and_may_carry_more_words() {
        assert!(mermaid_code("```Mermaid\ngraph TD\n```").is_some());
        assert!(mermaid_code("```mermaid title=\"flow\"\ngraph TD\n```").is_some());
        // A language that merely starts with "mermaid" is a different language.
        assert_eq!(mermaid_code("```mermaidish\ngraph TD\n```"), None);
    }

    #[test]
    fn only_a_paragraph_that_is_just_an_image_becomes_art() {
        assert_eq!(single_image_url("![alt](a.png)").as_deref(), Some("a.png"));
        assert_eq!(single_image_url("![](a.png)").as_deref(), Some("a.png"));
        // Text alongside the image must stay quotable.
        assert_eq!(single_image_url("see ![alt](a.png)"), None);
        assert_eq!(single_image_url("![a](a.png) ![b](b.png)"), None);
        // A linked image is a link first; leave it to the markdown renderer.
        assert_eq!(single_image_url("[![a](a.png)](https://x)"), None);
        assert_eq!(single_image_url("just text"), None);
    }
}
