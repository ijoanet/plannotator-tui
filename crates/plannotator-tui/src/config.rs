//! User configuration: one small TOML file, strict about what it accepts.
//!
//! `$PLANNOTATOR_TUI_CONFIG` → `$XDG_CONFIG_HOME/plannotator-tui/config.toml` →
//! `~/.config/plannotator-tui/config.toml`. A missing file means defaults. An unknown key is an
//! error that names the key, so a typo never silently falls back to a default.

use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(crate) struct Config {
    pub(crate) herdr: HerdrConfig,
    pub(crate) mermaid: MermaidConfig,
    pub(crate) image: ImageConfig,
    pub(crate) review: ReviewConfig,
    pub(crate) code: CodeConfig,
    pub(crate) git: GitConfig,
    pub(crate) theme: crate::theme::ThemeConfig,
}

/// What git contributes to the display.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(crate) struct GitConfig {
    /// Bar the gutter with what changed since `HEAD`. Off costs one git call less per document
    /// and leaves the sign column empty.
    pub(crate) signs: bool,
}

impl Default for GitConfig {
    fn default() -> Self {
        Self { signs: true }
    }
}

/// How code blocks render.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(crate) struct CodeConfig {
    /// Colour code by language. Off leaves every block in body text, which is how it looked
    /// before highlighting existed, and costs nothing.
    pub(crate) highlight: bool,
}

impl Default for CodeConfig {
    fn default() -> Self {
        Self { highlight: true }
    }
}

/// How a review behaves once it has been handed over.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(crate) struct ReviewConfig {
    /// Clear a file's annotations once they have been sent *and* archived, so the next send
    /// carries only what is new. The archive keeps what was sent.
    pub(crate) clear_on_send: bool,
}

impl Default for ReviewConfig {
    fn default() -> Self {
        Self { clear_on_send: true }
    }
}

impl Config {
    /// The art renderers' settings, as the layout wants them.
    pub(crate) fn art(&self) -> ArtConfig {
        ArtConfig { mermaid: self.mermaid.clone(), image: self.image.clone() }
    }

    /// The resolved palette, or an error naming the token that is not a color.
    pub(crate) fn theme(&self) -> Result<crate::theme::Theme> {
        crate::theme::Theme::resolve(&self.theme)
    }
}

/// Rendering of blocks that become pictures instead of text.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct ArtConfig {
    pub(crate) mermaid: MermaidConfig,
    pub(crate) image: ImageConfig,
}

impl ArtConfig {
    /// Both renderers off, for callers that must render text only.
    #[cfg(test)]
    pub(crate) fn disabled() -> Self {
        Self {
            mermaid: MermaidConfig { enabled: false, ..MermaidConfig::default() },
            image: ImageConfig { enabled: false, ..ImageConfig::default() },
        }
    }
}

/// Mermaid fences rendered as Unicode art by `grok-mermaid` under Node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(crate) struct MermaidConfig {
    pub(crate) enabled: bool,
    /// Directory holding `node_modules/grok-mermaid`; empty means the working directory.
    pub(crate) base_dir: String,
    /// Node binary; empty means `node` on `PATH`.
    pub(crate) node: String,
    /// Give up on a diagram after this long.
    pub(crate) timeout_ms: u64,
}

impl Default for MermaidConfig {
    fn default() -> Self {
        Self { enabled: true, base_dir: String::new(), node: String::new(), timeout_ms: 5_000 }
    }
}

impl MermaidConfig {
    /// `PLANNOTATOR_MERMAID_NODE`, else the configured binary, else `node` on `PATH`.
    pub(crate) fn resolved_node(&self) -> String {
        env_override("PLANNOTATOR_MERMAID_NODE")
            .or_else(|| Some(self.node.clone()).filter(|s| !s.is_empty()))
            .unwrap_or_else(|| "node".to_owned())
    }

    /// `PLANNOTATOR_MERMAID_BASE`, else the configured directory, else the working directory.
    pub(crate) fn resolved_base_dir(&self) -> Result<PathBuf> {
        let home = || std::env::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        if let Some(dir) = env_override("PLANNOTATOR_MERMAID_BASE") {
            return Ok(expand_home(&dir, &home()));
        }
        if !self.base_dir.is_empty() {
            return Ok(expand_home(&self.base_dir, &home()));
        }
        std::env::current_dir().context("locating the working directory for the mermaid renderer")
    }
}

/// Expand a leading `~`, the way `PLANNOTATOR_DATA_DIR` is expanded: a config file is written
/// by hand, so a home-relative path has to work there.
fn expand_home(dir: &str, home: &Path) -> PathBuf {
    let dir = dir.trim();
    match dir.strip_prefix("~/").or_else(|| dir.strip_prefix("~\\")) {
        Some(rest) => home.join(rest),
        None if dir == "~" => home.to_path_buf(),
        None => PathBuf::from(dir),
    }
}

/// Images rendered as Unicode half-blocks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(crate) struct ImageConfig {
    pub(crate) enabled: bool,
    /// Tallest an image may render, in terminal rows.
    pub(crate) max_rows: usize,
    /// Also render Obsidian's `![[image.png]]`, which is not Markdown. Off by default: a
    /// `CommonMark` document that happens to contain `![[x]]` means nothing by it.
    pub(crate) obsidian_embeds: bool,
}

impl Default for ImageConfig {
    fn default() -> Self {
        Self { enabled: true, max_rows: 20, obsidian_embeds: false }
    }
}

fn env_override(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|value| !value.is_empty())
}

/// How plannotator-tui opens inside Herdr.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(crate) struct HerdrConfig {
    pub(crate) placement: Placement,
    /// Split only.
    pub(crate) split_direction: SplitDirection,
    /// Popup only; cells or a percentage like `90%`.
    pub(crate) popup_width: String,
    pub(crate) popup_height: String,
}

impl Default for HerdrConfig {
    fn default() -> Self {
        Self {
            placement: Placement::Overlay,
            split_direction: SplitDirection::Right,
            popup_width: "90%".to_owned(),
            popup_height: "85%".to_owned(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Placement {
    /// A real pane zoomed over the whole tab; Herdr restores focus and zoom on exit.
    #[default]
    Overlay,
    /// Beside the target pane.
    Split,
    /// A modal floating box.
    Popup,
}

impl Placement {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Overlay => "overlay",
            Self::Split => "split",
            Self::Popup => "popup",
        }
    }
}

impl fmt::Display for Placement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Placement {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "overlay" => Ok(Self::Overlay),
            "split" => Ok(Self::Split),
            "popup" => Ok(Self::Popup),
            other => anyhow::bail!("unknown placement {other:?}; expected overlay, split or popup"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum SplitDirection {
    #[default]
    Right,
    Down,
}

impl SplitDirection {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Right => "right",
            Self::Down => "down",
        }
    }
}

impl fmt::Display for SplitDirection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Where the config file lives, given an environment lookup and the home directory.
pub(crate) fn config_path(env: impl Fn(&str) -> Option<String>, home: &Path) -> PathBuf {
    if let Some(explicit) = env("PLANNOTATOR_TUI_CONFIG").filter(|s| !s.is_empty()) {
        return PathBuf::from(explicit);
    }
    #[cfg(windows)]
    if let Some(appdata) = env("APPDATA").map(PathBuf::from).filter(|p| p.is_absolute()) {
        return appdata.join("plannotator-tui").join("config.toml");
    }
    let xdg = env("XDG_CONFIG_HOME").map(PathBuf::from).filter(|p| p.is_absolute());
    xdg.unwrap_or_else(|| home.join(".config")).join("plannotator-tui").join("config.toml")
}

impl Config {
    pub(crate) fn parse(text: &str) -> Result<Self> {
        toml::from_str(text).map_err(|e| anyhow::anyhow!("{}", e.message().trim()))
    }

    /// Read the config file for this process; a missing file is the default config.
    pub(crate) fn load() -> Result<Self> {
        let home = std::env::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let path = config_path(|k| std::env::var(k).ok(), &home);
        Self::load_from(&path)
    }

    pub(crate) fn load_from(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text).with_context(|| format!("in {}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    /// The effective config as TOML, for `plannotator-tui config`.
    pub(crate) fn to_toml(&self) -> Result<String> {
        Ok(toml::to_string(self)?)
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    #[test]
    fn empty_file_is_the_default_config() {
        assert_eq!(Config::parse("").expect("parses"), Config::default());
        assert_eq!(Config::default().herdr.placement, Placement::Overlay);
    }

    #[test]
    fn unknown_key_error_names_the_key() {
        let err = Config::parse("[herdr]\nplacment = \"popup\"\n").expect_err("rejected");
        assert!(err.to_string().contains("placment"), "{err}");
    }

    #[test]
    fn invalid_placement_error_names_the_value() {
        let err = Config::parse("[herdr]\nplacement = \"floating\"\n").expect_err("rejected");
        assert!(err.to_string().contains("floating"), "{err}");
    }

    #[test]
    fn partial_file_keeps_other_defaults() {
        let config = Config::parse("[herdr]\nplacement = \"split\"\n").expect("parses");
        assert_eq!(config.herdr.placement, Placement::Split);
        assert_eq!(config.herdr.split_direction, SplitDirection::Right);
        assert_eq!(config.herdr.popup_width, "90%");
    }

    #[cfg(windows)]
    #[test]
    fn config_lives_under_appdata_on_windows() {
        let lookup = |k: &str| (k == "APPDATA").then(|| r"C:\Users\u\AppData\Roaming".to_owned());
        assert_eq!(
            config_path(lookup, Path::new(r"C:\Users\u")),
            PathBuf::from(r"C:\Users\u\AppData\Roaming\plannotator-tui\config.toml")
        );
    }

    #[cfg(unix)]
    #[test]
    fn config_path_precedence() {
        let home = Path::new("/home/u");
        let lookup = |vars: &'static [(&'static str, &'static str)]| {
            move |k: &str| vars.iter().find(|(name, _)| *name == k).map(|(_, v)| (*v).to_owned())
        };
        assert_eq!(
            config_path(
                lookup(&[("PLANNOTATOR_TUI_CONFIG", "/etc/p.toml"), ("XDG_CONFIG_HOME", "/x")]),
                home
            ),
            PathBuf::from("/etc/p.toml")
        );
        assert_eq!(
            config_path(lookup(&[("XDG_CONFIG_HOME", "/x")]), home),
            PathBuf::from("/x/plannotator-tui/config.toml")
        );
        // A relative XDG_CONFIG_HOME is ignored, as the spec requires.
        assert_eq!(
            config_path(lookup(&[("XDG_CONFIG_HOME", "rel")]), home),
            PathBuf::from("/home/u/.config/plannotator-tui/config.toml")
        );
        assert_eq!(
            config_path(lookup(&[]), home),
            PathBuf::from("/home/u/.config/plannotator-tui/config.toml")
        );
    }

    #[test]
    fn roundtrips_through_toml() {
        let text = Config::default().to_toml().expect("serializes");
        assert_eq!(Config::parse(&text).expect("parses"), Config::default());
    }

    #[test]
    fn a_home_relative_renderer_directory_is_expanded() {
        let home = Path::new("/home/u");
        assert_eq!(expand_home("~/.local/share/gm", home), PathBuf::from("/home/u/.local/share/gm"));
        assert_eq!(expand_home("~", home), PathBuf::from("/home/u"));
        assert_eq!(expand_home("/abs/gm", home), PathBuf::from("/abs/gm"));
        // Not a home reference: a directory that merely begins with a tilde.
        assert_eq!(expand_home("~weird/gm", home), PathBuf::from("~weird/gm"));
    }

    #[test]
    fn art_sections_default_on_and_accept_partial_overrides() {
        let config = Config::parse("[image]\nmax_rows = 4\n").expect("parses");
        assert!(config.image.enabled, "unset keys keep their default");
        assert_eq!(config.image.max_rows, 4);
        assert!(config.mermaid.enabled);
        assert_eq!(config.art().image.max_rows, 4);

        let off = Config::parse("[mermaid]\nenabled = false\n").expect("parses");
        assert!(!off.mermaid.enabled);
        assert!(off.image.enabled);
    }

    #[test]
    fn an_unknown_art_key_names_the_key() {
        let err = Config::parse("[image]\nmax_row = 4\n").expect_err("rejected");
        assert!(err.to_string().contains("max_row"), "{err}");
    }
}
