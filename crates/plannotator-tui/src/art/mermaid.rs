//! Mermaid diagrams as Unicode art, rendered by `grok-mermaid` in one short-lived Node process.
//!
//! This is the crate's one runtime dependency on something outside the binary, so it is
//! failure-shaped throughout: a missing `node`, a missing `grok-mermaid`, a diagram the
//! renderer rejects and a renderer that hangs all end the same way — no art, and the block
//! shows the Mermaid source as a normal code block.
//!
//! Every diagram in a document is rendered by a single process, because Node's startup costs
//! far more than laying out a diagram. The first renderer-level failure disables the feature
//! for the rest of the process, so a second document does not pay for a spawn that cannot work.

use std::io::{Read as _, Write as _};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use serde::{Deserialize, Serialize};

use crate::config::MermaidConfig;
use crate::theme::Theme;

/// The bridge script, embedded so the binary stays self-contained.
const SCRIPT: &str = include_str!("render_mermaid.mjs");

/// Set once the renderer proves unusable (no Node, no `grok-mermaid`); stops further spawns.
static RENDERER_UNUSABLE: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Serialize)]
struct Request<'a> {
    sources: &'a [String],
}

#[derive(Debug, Deserialize)]
struct Response {
    ok: bool,
    #[serde(default)]
    diagrams: Vec<Diagram>,
}

#[derive(Debug, Deserialize)]
struct Diagram {
    ok: bool,
    #[serde(default)]
    rows: Vec<Vec<ArtSpan>>,
}

#[derive(Debug, Deserialize)]
struct ArtSpan {
    #[serde(default)]
    text: String,
    #[serde(default)]
    cls: String,
}

/// Render each source to styled rows. The result has one entry per input, `None` where that
/// diagram did not render; an unusable renderer yields all `None`.
pub(super) fn render_all(
    sources: &[String],
    config: &MermaidConfig,
    theme: Theme,
) -> Vec<Option<Text<'static>>> {
    let none = || sources.iter().map(|_| None).collect();
    if sources.is_empty() || RENDERER_UNUSABLE.load(Ordering::Relaxed) {
        return none();
    }
    let Ok(output) = run(sources, config) else {
        // A process that would not start, or would not stop, is a renderer failure.
        RENDERER_UNUSABLE.store(true, Ordering::Relaxed);
        return none();
    };
    let Ok(response) = serde_json::from_slice::<Response>(&output) else { return none() };
    if !response.ok {
        RENDERER_UNUSABLE.store(true, Ordering::Relaxed);
        return none();
    }
    // Trust the inputs' count, not the reply's: a short or long reply must not shift diagrams
    // onto the wrong blocks.
    sources
        .iter()
        .enumerate()
        .map(|(i, _)| {
            let diagram = response.diagrams.get(i)?;
            // An `ok` diagram with no rows would render as a blank gap; keep the source.
            (diagram.ok && !diagram.rows.is_empty()).then(|| to_text(&diagram.rows, theme))
        })
        .collect()
}

/// Run the bridge script over `sources` and return its stdout, bounded by `timeout_ms`.
fn run(sources: &[String], config: &MermaidConfig) -> anyhow::Result<Vec<u8>> {
    let base = config.resolved_base_dir()?;
    let request = serde_json::to_vec(&Request { sources })?;
    let mut child = Command::new(config.resolved_node())
        .arg("--input-type=module")
        .arg("-e")
        .arg(SCRIPT)
        .env("PLANNOTATOR_MERMAID_BASE", &base)
        // stderr is never read, so it must not be a pipe that could fill and block the child.
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;

    // Writer and reader both run off-thread so neither can block on the other's pipe.
    if let Some(mut stdin) = child.stdin.take() {
        std::thread::spawn(move || stdin.write_all(&request));
    }
    let (tx, rx) = mpsc::channel();
    if let Some(mut stdout) = child.stdout.take() {
        std::thread::spawn(move || {
            let mut buffer = Vec::new();
            let read = stdout.read_to_end(&mut buffer);
            let _ = tx.send(read.map(|_| buffer));
        });
    }

    let Ok(result) = rx.recv_timeout(Duration::from_millis(config.timeout_ms)) else {
        let _ = child.kill();
        let _ = child.wait();
        anyhow::bail!("mermaid renderer timed out after {}ms", config.timeout_ms)
    };
    let _ = child.wait();
    Ok(result?)
}

/// The theme token for each class `grok-mermaid` emits. This is the mapping pi uses for the
/// same renderer, so a diagram looks the same here as in the agent that produced it. An
/// unknown class renders as plain text, so a newer `grok-mermaid` degrades rather than
/// losing the span.
fn style_for(class: &str, theme: Theme) -> Style {
    match class {
        "border" => Style::new().fg(theme.border),
        "text" => Style::new().fg(theme.text),
        "edge" => Style::new().fg(theme.accent),
        "edgeLabel" => Style::new().fg(theme.muted),
        "title" => Style::new().fg(theme.accent).add_modifier(Modifier::BOLD),
        _ => Style::new(),
    }
}

fn to_text(rows: &[Vec<ArtSpan>], theme: Theme) -> Text<'static> {
    let lines = rows
        .iter()
        .map(|row| {
            let spans: Vec<Span<'static>> =
                row.iter().map(|s| Span::styled(s.text.clone(), style_for(&s.cls, theme))).collect();
            Line::from(spans)
        })
        .collect::<Vec<_>>();
    Text::from(lines)
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use ratatui::style::Color;

    use super::*;

    fn parse(json: &str) -> Response {
        serde_json::from_str(json).expect("parses")
    }

    #[test]
    fn styled_rows_become_lines_with_class_styles() {
        let response = parse(
            r#"{"ok":true,"diagrams":[{"ok":true,"rows":[
                [{"text":"  ","cls":"none"},{"text":"┌─┐","cls":"border"}],
                [{"text":"yes","cls":"edgeLabel"}]
            ]}]}"#,
        );
        let rows = &response.diagrams.first().expect("one diagram").rows;
        let text = to_text(rows, Theme::default());
        assert_eq!(text.lines.len(), 2);
        assert_eq!(text.lines.first().map(ToString::to_string).as_deref(), Some("  ┌─┐"));
        let border = text.lines.first().and_then(|l| l.spans.get(1)).expect("border span");
        assert_eq!(border.style.fg, Some(Theme::default().border));
        let label = text.lines.get(1).and_then(|l| l.spans.first()).expect("label span");
        assert_eq!(label.style.fg, Some(Theme::default().muted));
    }

    #[test]
    fn every_class_maps_to_the_token_pi_uses_for_it() {
        // The mapping is the contract with pi: the same renderer must colour the same way in
        // both, so a diagram is not re-read differently in review than it was when written.
        let theme = Theme { accent: Color::Blue, muted: Color::Gray, ..Theme::default() };
        assert_eq!(style_for("border", theme).fg, Some(theme.border), "border -> border");
        assert_eq!(style_for("text", theme).fg, Some(theme.text), "text -> text");
        assert_eq!(style_for("edge", theme).fg, Some(theme.accent), "edge -> accent");
        assert_eq!(style_for("edgeLabel", theme).fg, Some(theme.muted), "edgeLabel -> muted");
        let title = style_for("title", theme);
        assert_eq!(title.fg, Some(theme.accent), "title -> accent");
        assert!(title.add_modifier.contains(Modifier::BOLD), "title is bold");
    }

    #[test]
    fn unknown_class_keeps_its_text_in_the_default_style() {
        assert_eq!(style_for("somethingNew", Theme::default()), Style::new());
        assert_eq!(style_for("none", Theme::default()), Style::new());
    }

    #[test]
    fn no_diagrams_never_spawns_a_renderer() {
        assert!(render_all(&[], &MermaidConfig::default(), Theme::default()).is_empty());
    }
}
