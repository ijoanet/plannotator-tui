//! The colors the app draws with.
//!
//! One semantic token per thing that has a color, so a document, a diagram and the chrome stay
//! coherent when any of them is retuned. Tokens are named for their meaning (`accent`,
//! `muted`) rather than a color, which is what lets a config file match an outside palette —
//! a terminal theme, or the agent's own — without this module knowing about it.
//!
//! Diagram parts follow the same tokens on purpose: `grok-mermaid` labels each span
//! `border`/`text`/`edge`/`edgeLabel`/`title`, and those map onto `border`, `text`, `accent`,
//! `muted` and `accent`. It is the mapping pi uses for the same renderer, so a diagram looks
//! the same in the reviewer as it did in the agent that wrote it.
//!
//! A value is anything `ratatui` parses: `#41464e`, `cyan`, `light-yellow`, `238`, `reset`.
//! An empty value keeps the built-in default, so a config file only names what it changes.

use std::str::FromStr as _;

use anyhow::Result;
use ratatui::style::Color;
use serde::{Deserialize, Serialize};

/// Colors as written in the config file; empty means "keep the default".
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(crate) struct ThemeConfig {
    pub(crate) text: String,
    pub(crate) heading: String,
    /// Headings below level two, so a deep outline can stay distinguishable.
    pub(crate) heading_deep: String,
    pub(crate) code: String,
    pub(crate) link: String,
    pub(crate) quote: String,
    /// Focused borders, the selected-block marker, diagram edges and diagram titles.
    pub(crate) accent: String,
    /// Unfocused borders and diagram edge labels.
    pub(crate) muted: String,
    /// Diagram box-drawing.
    pub(crate) border: String,
    pub(crate) comment: String,
    pub(crate) approve: String,
    pub(crate) delete: String,
    pub(crate) comment_bg: String,
    pub(crate) approve_bg: String,
    /// Background of the selected block.
    pub(crate) block_bg: String,
    /// Background of the cursor row in a list.
    pub(crate) cursor_bg: String,
    /// Background of the selection toolbar and of an idle Send button.
    pub(crate) toolbar_bg: String,
    /// Background of a Send button with something to send.
    pub(crate) send_bg: String,
}

/// Resolved colors, ready to draw with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Theme {
    pub(crate) text: Color,
    pub(crate) heading: Color,
    pub(crate) heading_deep: Color,
    pub(crate) code: Color,
    pub(crate) link: Color,
    pub(crate) quote: Color,
    pub(crate) accent: Color,
    pub(crate) muted: Color,
    pub(crate) border: Color,
    pub(crate) comment: Color,
    pub(crate) approve: Color,
    pub(crate) delete: Color,
    pub(crate) comment_bg: Color,
    pub(crate) approve_bg: Color,
    pub(crate) block_bg: Color,
    pub(crate) cursor_bg: Color,
    pub(crate) toolbar_bg: Color,
    pub(crate) send_bg: Color,
}

impl Default for Theme {
    /// The house style: headings carry weight through bold and underline rather than a
    /// background, so the palette stays available for selection and annotations.
    fn default() -> Self {
        Self {
            text: Color::Reset,
            heading: Color::Cyan,
            heading_deep: Color::LightCyan,
            code: Color::LightYellow,
            link: Color::Blue,
            quote: Color::Green,
            accent: Color::Cyan,
            muted: Color::DarkGray,
            border: Color::DarkGray,
            comment: Color::Yellow,
            approve: Color::Green,
            delete: Color::Red,
            comment_bg: Color::Indexed(58),
            approve_bg: Color::Indexed(22),
            block_bg: Color::Indexed(236),
            cursor_bg: Color::Indexed(240),
            toolbar_bg: Color::Indexed(238),
            send_bg: Color::Indexed(30),
        }
    }
}

impl Theme {
    /// Apply a config over the defaults, naming the key and the value on a bad color.
    pub(crate) fn resolve(config: &ThemeConfig) -> Result<Self> {
        let d = Self::default();
        Ok(Self {
            text: color(&config.text, "text", d.text)?,
            heading: color(&config.heading, "heading", d.heading)?,
            heading_deep: color(&config.heading_deep, "heading_deep", d.heading_deep)?,
            code: color(&config.code, "code", d.code)?,
            link: color(&config.link, "link", d.link)?,
            quote: color(&config.quote, "quote", d.quote)?,
            accent: color(&config.accent, "accent", d.accent)?,
            muted: color(&config.muted, "muted", d.muted)?,
            border: color(&config.border, "border", d.border)?,
            comment: color(&config.comment, "comment", d.comment)?,
            approve: color(&config.approve, "approve", d.approve)?,
            delete: color(&config.delete, "delete", d.delete)?,
            comment_bg: color(&config.comment_bg, "comment_bg", d.comment_bg)?,
            approve_bg: color(&config.approve_bg, "approve_bg", d.approve_bg)?,
            block_bg: color(&config.block_bg, "block_bg", d.block_bg)?,
            cursor_bg: color(&config.cursor_bg, "cursor_bg", d.cursor_bg)?,
            toolbar_bg: color(&config.toolbar_bg, "toolbar_bg", d.toolbar_bg)?,
            send_bg: color(&config.send_bg, "send_bg", d.send_bg)?,
        })
    }

    /// The accent for an annotation kind.
    pub(crate) fn kind(self, kind: plannotator_tui_schema::Kind) -> Color {
        match kind {
            plannotator_tui_schema::Kind::Comment => self.comment,
            plannotator_tui_schema::Kind::LooksGood => self.approve,
            plannotator_tui_schema::Kind::Delete => self.delete,
        }
    }
}

/// A foreground that stays legible on `background`.
///
/// The badges paint text on a themed fill, so a fixed foreground only works for the palette it
/// was chosen against: black on the default green was already poor, and on a darker themed green
/// it fell to 1.6:1. Relative luminance per WCAG, with the same 0.179 threshold browsers use,
/// picks the better of black and white for any fill.
pub(crate) fn readable_on(background: Color) -> Color {
    let Color::Rgb(r, g, b) = background else {
        // A named or indexed color has no channels to read; assume a dark terminal, where a
        // light foreground is the safer default.
        return Color::White;
    };
    let channel = |c: u8| {
        let c = f64::from(c) / 255.0;
        if c <= 0.039_28 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    let luminance = 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b);
    if luminance > 0.179 { Color::Black } else { Color::White }
}

fn color(value: &str, key: &str, fallback: Color) -> Result<Color> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(fallback);
    }
    Color::from_str(value).map_err(|_| {
        anyhow::anyhow!("theme.{key}: {value:?} is not a color (try \"#41464e\", \"cyan\" or \"238\")")
    })
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    #[test]
    fn an_empty_config_is_the_house_style() {
        assert_eq!(Theme::resolve(&ThemeConfig::default()).expect("resolves"), Theme::default());
    }

    #[test]
    fn hex_names_and_indexed_values_all_parse() {
        let config = ThemeConfig {
            border: "#41464e".to_owned(),
            accent: "cyan".to_owned(),
            code: "light-yellow".to_owned(),
            block_bg: "238".to_owned(),
            text: "reset".to_owned(),
            ..ThemeConfig::default()
        };
        let theme = Theme::resolve(&config).expect("resolves");
        assert_eq!(theme.border, Color::Rgb(65, 70, 78));
        assert_eq!(theme.accent, Color::Cyan);
        assert_eq!(theme.code, Color::LightYellow);
        assert_eq!(theme.block_bg, Color::Indexed(238));
        assert_eq!(theme.text, Color::Reset);
        // Untouched tokens keep their default.
        assert_eq!(theme.heading, Theme::default().heading);
    }

    #[test]
    fn a_bad_color_names_the_key_and_the_value() {
        let config = ThemeConfig { accent: "burnt siena".to_owned(), ..ThemeConfig::default() };
        let err = Theme::resolve(&config).expect_err("rejected");
        assert!(err.to_string().contains("theme.accent"), "{err}");
        assert!(err.to_string().contains("burnt siena"), "{err}");
    }

    #[test]
    fn a_badge_foreground_is_chosen_for_contrast_against_its_fill() {
        // The regression this exists for: black on a dark themed green was 1.6:1.
        assert_eq!(readable_on(Color::Rgb(0x0b, 0x3a, 0x20)), Color::White);
        assert_eq!(readable_on(Color::Rgb(0xff, 0xff, 0xff)), Color::Black);
        assert_eq!(readable_on(Color::Rgb(0xc9, 0xd3, 0x64)), Color::Black, "a bright yellow fill");
        // Channels unknown: assume a dark terminal.
        assert_eq!(readable_on(Color::Indexed(22)), Color::White);
    }

    #[test]
    fn each_annotation_kind_has_its_own_accent() {
        let theme = Theme::default();
        assert_eq!(theme.kind(plannotator_tui_schema::Kind::Comment), Color::Yellow);
        assert_eq!(theme.kind(plannotator_tui_schema::Kind::LooksGood), Color::Green);
        assert_eq!(theme.kind(plannotator_tui_schema::Kind::Delete), Color::Red);
    }
}
