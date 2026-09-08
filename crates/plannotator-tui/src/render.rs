//! What rendering a document needs from outside the layout.
//!
//! Its own module because both halves of rendering read it: the layout styles Markdown with
//! the theme, and `art` needs the same theme plus where relative image paths point. Neither
//! owns it, so neither imports the other to get it.

use std::path::{Path, PathBuf};

use crate::config::ArtConfig;
use crate::theme::Theme;

/// Rendering settings for the process, applied to every document opened.
#[derive(Debug, Clone)]
pub(crate) struct RenderSettings {
    pub(crate) art: ArtConfig,
    pub(crate) theme: Theme,
}

impl RenderSettings {
    /// Settings for a document read from `path`; relative paths follow the document.
    pub(crate) fn context(&self, path: Option<&Path>) -> RenderContext {
        RenderContext::for_document(path, self.art.clone(), self.theme)
    }

    /// Art disabled entirely, default palette: what a test wants unless it says otherwise.
    #[cfg(test)]
    pub(crate) fn text_only() -> Self {
        Self { art: ArtConfig::disabled(), theme: Theme::default() }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RenderContext {
    /// Directory that relative image paths resolve against.
    pub(crate) base_dir: PathBuf,
    pub(crate) art: ArtConfig,
    pub(crate) theme: Theme,
}

impl RenderContext {
    /// Context for a document read from `path`; relative paths follow the document.
    pub(crate) fn for_document(path: Option<&Path>, art: ArtConfig, theme: Theme) -> Self {
        let base_dir = path
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));
        Self { base_dir, art, theme }
    }

    /// Art disabled entirely: every block renders as text, in the default palette.
    #[cfg(test)]
    pub(crate) fn text_only() -> Self {
        Self { base_dir: PathBuf::from("."), art: ArtConfig::disabled(), theme: Theme::default() }
    }
}
