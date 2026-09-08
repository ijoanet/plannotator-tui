//! Images as Unicode half-blocks: two vertical pixels per cell, colored with truecolor.
//!
//! Half-blocks rather than a terminal graphics protocol, because the whole app is a grid of
//! styled cells: art made of cells scrolls, clips and selects like text, renders the same on
//! every terminal that does truecolor, and can be asserted in a `TestBackend` snapshot. A
//! kitty/sixel image would need out-of-band escapes that the buffer diff knows nothing about.
//!
//! With half-blocks each sub-cell pixel is square (a cell is about twice as tall as it is
//! wide), so fitting the picture to a pixel grid of `cols × 2·rows` preserves aspect ratio
//! without a correction factor.
//!
//! Decoding happens once into a bounded thumbnail; a resize re-samples that thumbnail
//! instead of touching the file again.

use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use image::imageops::FilterType;
use image::{ImageReader, Limits, RgbaImage};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span, Text};

use crate::config::ImageConfig;

/// Widest thumbnail kept in memory; wider terminals clip rather than re-decode.
const MAX_COLS: u32 = 240;
/// Refuse to decode a file larger than this; a document should not stall on one image.
const MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;
/// Guards against a small file that decodes into an enormous allocation.
const MAX_DECODE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_DIMENSION: u32 = 20_000;

/// Alpha at or above this counts as opaque; below it the terminal background shows through.
const OPAQUE: u8 = 128;

const UPPER: char = '▀';
const LOWER: char = '▄';

/// A decoded, bounded thumbnail that can be re-sampled to any column width.
pub(crate) struct ImageArt {
    thumbnail: RgbaImage,
    max_rows: usize,
}

impl fmt::Debug for ImageArt {
    /// Dimensions only: the pixel buffer is noise in a debug dump.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImageArt")
            .field("thumbnail", &(self.thumbnail.width(), self.thumbnail.height()))
            .field("max_rows", &self.max_rows)
            .finish()
    }
}

/// Load the image at `path`, or `None` if it cannot be decoded.
///
/// A missing or undecodable image is not an error: the block keeps its markdown rendering.
pub(super) fn load(path: &Path, config: &ImageConfig) -> Option<ImageArt> {
    decode(path, config.max_rows).ok()
}

/// The file a markdown destination points at, or `None` for anything not on this disk.
///
/// Remote images are skipped rather than fetched: the app has no business making network
/// requests while rendering a document.
pub(super) fn local_path(url: &str, base_dir: &Path) -> Option<PathBuf> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }
    if let Some(rest) = url.strip_prefix("file://") {
        // Only a local-host file URL; "file://host/path" is somewhere else.
        let rest = rest.strip_prefix("localhost").unwrap_or(rest);
        return rest.starts_with('/').then(|| PathBuf::from(rest));
    }
    if has_scheme(url) {
        return None;
    }
    let path = Path::new(url);
    Some(if path.is_absolute() { path.to_path_buf() } else { base_dir.join(path) })
}

/// Whether `url` starts with a URL scheme (`https:`, `data:`), as opposed to a path.
///
/// A Windows drive letter (`C:\x`) is a path, not a scheme, so a scheme needs two or more
/// leading characters.
fn has_scheme(url: &str) -> bool {
    match url.split_once(':') {
        Some((scheme, _)) => {
            scheme.len() > 1
                && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
                && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        }
        None => false,
    }
}

fn decode(path: &Path, max_rows: usize) -> Result<ImageArt> {
    let size = std::fs::metadata(path).with_context(|| format!("reading {}", path.display()))?.len();
    anyhow::ensure!(size <= MAX_FILE_BYTES, "{} is too large to render ({size} bytes)", path.display());

    let mut limits = Limits::no_limits();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits.max_alloc = Some(MAX_DECODE_BYTES);

    let mut reader = ImageReader::open(path).with_context(|| format!("opening {}", path.display()))?;
    reader.limits(limits);
    let reader = reader.with_guessed_format().with_context(|| format!("reading {}", path.display()))?;
    let image = reader.decode().with_context(|| format!("decoding {}", path.display()))?;

    let (width, height) = fit(image.width(), image.height(), MAX_COLS, pixel_rows(max_rows));
    Ok(ImageArt { thumbnail: image.resize_exact(width, height, FilterType::Triangle).to_rgba8(), max_rows })
}

/// Pixel rows available for `rows` terminal rows: two per row.
fn pixel_rows(rows: usize) -> u32 {
    u32::try_from(rows.max(1)).unwrap_or(u32::MAX).saturating_mul(2)
}

/// Largest `w × h` that fits in `max_w × max_h` with the aspect ratio kept, never upscaling.
fn fit(w: u32, h: u32, max_w: u32, max_h: u32) -> (u32, u32) {
    if w == 0 || h == 0 {
        return (1, 1);
    }
    let scale = f64::from(max_w) / f64::from(w);
    let scale = scale.min(f64::from(max_h) / f64::from(h)).min(1.0);
    let scaled = |v: u32| ((f64::from(v) * scale).round() as u32).max(1);
    (scaled(w), scaled(h))
}

impl ImageArt {
    /// Styled rows for a `width`-column budget.
    pub(crate) fn to_text(&self, width: usize) -> Text<'static> {
        let cols = u32::try_from(width.max(1)).unwrap_or(MAX_COLS);
        let (w, h) = fit(self.thumbnail.width(), self.thumbnail.height(), cols, pixel_rows(self.max_rows));
        if w == self.thumbnail.width() && h == self.thumbnail.height() {
            return half_blocks(&self.thumbnail);
        }
        half_blocks(&image::imageops::resize(&self.thumbnail, w, h, FilterType::Triangle))
    }
}

/// One cell: the character to draw and the colors it needs.
fn cell(top: Option<&image::Rgba<u8>>, bottom: Option<&image::Rgba<u8>>) -> (char, Style) {
    let color = |px: Option<&image::Rgba<u8>>| {
        px.filter(|p| p.0.get(3).is_some_and(|&a| a >= OPAQUE)).map(|p| {
            let [r, g, b, _] = p.0;
            Color::Rgb(r, g, b)
        })
    };
    match (color(top), color(bottom)) {
        (Some(top), Some(bottom)) => (UPPER, Style::new().fg(top).bg(bottom)),
        (Some(top), None) => (UPPER, Style::new().fg(top)),
        (None, Some(bottom)) => (LOWER, Style::new().fg(bottom)),
        (None, None) => (' ', Style::new()),
    }
}

/// Convert a pixel grid to one row per two pixel rows, merging equal-styled runs.
fn half_blocks(image: &RgbaImage) -> Text<'static> {
    let rows = (image.height() as usize).div_ceil(2);
    let lines = (0..rows)
        .map(|row| {
            let y = (row as u32).saturating_mul(2);
            let mut spans: Vec<Span<'static>> = Vec::new();
            for x in 0..image.width() {
                let (ch, style) = cell(image.get_pixel_checked(x, y), image.get_pixel_checked(x, y + 1));
                match spans.last_mut() {
                    Some(last) if last.style == style => last.content.to_mut().push(ch),
                    _ => spans.push(Span::styled(ch.to_string(), style)),
                }
            }
            Line::from(spans)
        })
        .collect::<Vec<_>>();
    Text::from(lines)
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    #[test]
    fn remote_and_data_urls_are_never_read_from_disk() {
        let base = Path::new("/docs");
        assert_eq!(local_path("https://x/y.png", base), None);
        assert_eq!(local_path("http://x/y.png", base), None);
        assert_eq!(local_path("data:image/png;base64,AAAA", base), None);
        assert_eq!(local_path("", base), None);
    }

    #[test]
    fn relative_paths_resolve_against_the_document_directory() {
        let base = Path::new("/docs");
        assert_eq!(local_path("a.png", base), Some(PathBuf::from("/docs/a.png")));
        assert_eq!(local_path("./img/a.png", base), Some(PathBuf::from("/docs/./img/a.png")));
        assert_eq!(local_path("/abs/a.png", base), Some(PathBuf::from("/abs/a.png")));
        assert_eq!(local_path("file:///abs/a.png", base), Some(PathBuf::from("/abs/a.png")));
        // A drive letter is a path, not a URL scheme.
        assert!(local_path(r"C:\img\a.png", base).is_some());
    }

    #[test]
    fn fit_keeps_aspect_ratio_and_never_upscales() {
        assert_eq!(fit(100, 50, 50, 100), (50, 25));
        assert_eq!(fit(100, 50, 500, 100), (100, 50));
        // Height is the binding constraint here.
        assert_eq!(fit(100, 100, 80, 40), (40, 40));
        assert_eq!(fit(0, 10, 80, 40), (1, 1));
    }

    #[test]
    fn two_pixel_rows_make_one_cell_with_top_as_foreground() {
        let mut image = RgbaImage::new(1, 2);
        image.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
        image.put_pixel(0, 1, image::Rgba([0, 0, 255, 255]));
        let text = half_blocks(&image);
        assert_eq!(text.lines.len(), 1);
        let span = text.lines.first().and_then(|l| l.spans.first()).expect("one span");
        assert_eq!(span.content.as_ref(), "▀");
        assert_eq!(span.style.fg, Some(Color::Rgb(255, 0, 0)));
        assert_eq!(span.style.bg, Some(Color::Rgb(0, 0, 255)));
    }

    #[test]
    fn an_odd_pixel_height_leaves_the_last_cell_half_drawn() {
        let mut image = RgbaImage::new(1, 1);
        image.put_pixel(0, 0, image::Rgba([1, 2, 3, 255]));
        let text = half_blocks(&image);
        let span = text.lines.first().and_then(|l| l.spans.first()).expect("one span");
        assert_eq!(span.content.as_ref(), "▀");
        assert_eq!(span.style.bg, None, "no second pixel: terminal background shows through");
    }

    #[test]
    fn transparent_pixels_show_the_terminal_background() {
        let mut image = RgbaImage::new(2, 2);
        image.put_pixel(0, 0, image::Rgba([0, 0, 0, 0]));
        image.put_pixel(0, 1, image::Rgba([9, 9, 9, 255]));
        image.put_pixel(1, 0, image::Rgba([0, 0, 0, 0]));
        image.put_pixel(1, 1, image::Rgba([0, 0, 0, 0]));
        let line = half_blocks(&image).lines.into_iter().next().expect("one row");
        assert_eq!(line.to_string(), "▄ ");
        let lower = line.spans.first().expect("lower half");
        assert_eq!(lower.style.fg, Some(Color::Rgb(9, 9, 9)));
        assert_eq!(lower.style.bg, None);
    }

    #[test]
    fn equal_styled_cells_merge_into_one_span() {
        let mut image = RgbaImage::new(4, 2);
        for (x, y) in [(0, 0), (1, 0), (2, 0), (3, 0), (0, 1), (1, 1), (2, 1), (3, 1)] {
            image.put_pixel(x, y, image::Rgba([7, 7, 7, 255]));
        }
        let line = half_blocks(&image).lines.into_iter().next().expect("one row");
        assert_eq!(line.spans.len(), 1);
        assert_eq!(line.to_string(), "▀▀▀▀");
    }
}
