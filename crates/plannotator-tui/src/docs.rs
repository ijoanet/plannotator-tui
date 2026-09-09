//! The documents presented together, and the tab row that shows them.
//!
//! An agent presents a set of files; this is that set, in tab order, with each one's annotation
//! count. There is no folder tree, because the tool exists for an agent to show changes rather
//! than for a human to browse a repository: the only navigation is `TAB` through the set.
//!
//! A folder argument still works. It expands to the Markdown files beneath it, breadth-first and
//! bounded, so a huge repository cannot hold a blank pane while it is walked.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use unicode_width::UnicodeWidthStr;

/// Directories that hold dependencies or build output, never docs worth listing.
const SKIPPED_DIRS: [&str; 8] =
    ["node_modules", "target", "vendor", "dist", "build", "out", "__pycache__", "venv"];

/// Most documents a folder may expand to. A set is navigated one `TAB` at a time, so a
/// thousand tabs would be unreachable in practice as well as slow to gather.
pub(crate) const FOLDER_LIMIT: usize = 200;

/// Entries a folder walk may visit before giving up on finding more Markdown.
const WALK_BUDGET: usize = 2_000;

/// One presented document.
#[derive(Debug, Clone)]
pub(crate) struct Doc {
    pub(crate) path: PathBuf,
    /// What the tab shows: the file name, lengthened only far enough to tell it from another
    /// document that would read the same.
    pub(crate) name: String,
    pub(crate) annotations: usize,
}

/// The presented set: every document, which one is open, and the directory they share.
#[derive(Debug)]
pub(crate) struct DocSet {
    root: PathBuf,
    docs: Vec<Doc>,
    current: usize,
}

impl DocSet {
    /// Exactly these files, in name order.
    pub(crate) fn of_files(root: &Path, files: &[PathBuf]) -> Self {
        let mut paths: Vec<PathBuf> = files.to_vec();
        paths.sort();
        paths.dedup();
        let names = disambiguate(&paths);
        let mut docs: Vec<Doc> =
            paths.into_iter().zip(names).map(|(path, name)| Doc { path, name, annotations: 0 }).collect();
        docs.sort_by(|a, b| a.name.cmp(&b.name));
        Self { root: root.to_path_buf(), docs, current: 0 }
    }

    /// Every Markdown file beneath `root`, shallowest first.
    pub(crate) fn of_folder(root: &Path) -> Result<Self> {
        let files = markdown_under(root, FOLDER_LIMIT, WALK_BUDGET)?;
        anyhow::ensure!(!files.is_empty(), "no Markdown file in {}", root.display());
        Ok(Self::of_files(root, &files))
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn docs(&self) -> &[Doc] {
        &self.docs
    }

    pub(crate) fn len(&self) -> usize {
        self.docs.len()
    }

    pub(crate) fn current(&self) -> usize {
        self.current
    }

    pub(crate) fn current_path(&self) -> Option<&Path> {
        self.docs.get(self.current).map(|d| d.path.as_path())
    }

    /// The tab name for `path`, when it is in the set.
    pub(crate) fn name_for(&self, path: &Path) -> Option<&str> {
        self.docs.iter().find(|d| d.path == path).map(|d| d.name.as_str())
    }

    /// Point at `path` when it is in the set, so the tab row follows what is on screen.
    pub(crate) fn focus(&mut self, path: &Path) {
        if let Some(index) = self.docs.iter().position(|d| d.path == path) {
            self.current = index;
        }
    }

    /// The document after the open one, wrapping. `None` when there is nothing to move to.
    pub(crate) fn next(&self) -> Option<(usize, PathBuf)> {
        if self.docs.len() < 2 {
            return None;
        }
        let index = (self.current + 1) % self.docs.len();
        self.docs.get(index).map(|d| (index, d.path.clone()))
    }

    /// Annotations across the whole set, which is what a send covers.
    pub(crate) fn total_annotations(&self) -> usize {
        self.docs.iter().map(|d| d.annotations).sum()
    }

    /// Recount every document from `count`, which reads the records on disk.
    pub(crate) fn set_counts(&mut self, count: impl Fn(&Path) -> usize) {
        for doc in &mut self.docs {
            doc.annotations = count(&doc.path);
        }
    }

    /// What a tab shows: its name, and its annotation count when it has any.
    pub(crate) fn label(doc: &Doc) -> String {
        if doc.annotations > 0 {
            format!(" {} {} ", doc.name, doc.annotations)
        } else {
            format!(" {} ", doc.name)
        }
    }

    /// Which tabs fit in `width`, and how many are hidden either side.
    pub(crate) fn tab_row(&self, width: usize) -> TabRow {
        let widths: Vec<usize> = self.docs.iter().map(|d| Self::label(d).width()).collect();
        tab_row(&widths, self.current, width)
    }
}

/// The tab row for one width: the tabs to draw, and the counts hidden past each edge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TabRow {
    pub(crate) hidden_before: usize,
    /// Indices into the set, in display order. Always contains the open document.
    pub(crate) visible: std::ops::Range<usize>,
    pub(crate) hidden_after: usize,
}

/// Columns an edge marker needs: `‹12 ` and ` 12›` are both the digits plus two.
pub(crate) fn marker_width(hidden: usize) -> usize {
    if hidden == 0 { 0 } else { 2 + hidden.to_string().len() }
}

/// The last `depth` components of a path, joined for display.
fn tail(parts: &[String], depth: usize) -> String {
    let start = parts.len().saturating_sub(depth.max(1));
    parts.get(start..).map_or_else(String::new, |rest| rest.join("/"))
}

fn components(path: &Path) -> Vec<String> {
    path.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect()
}

/// Tab names: the file name, gaining one parent at a time only where two documents would
/// otherwise read the same.
///
/// This is the rule editors use. Naming every tab by its path relative to a common root instead
/// lengthens documents that do not collide, because some unrelated pair did: `plan.md` should stay
/// `plan.md` when `one/doc.md` and `two/doc.md` are also open.
fn disambiguate(paths: &[PathBuf]) -> Vec<String> {
    let parts: Vec<Vec<String>> = paths.iter().map(|p| components(p)).collect();
    let mut depth: Vec<usize> = vec![1; parts.len()];
    loop {
        let names: Vec<String> = parts.iter().zip(&depth).map(|(p, d)| tail(p, *d)).collect();
        let mut next = depth.clone();
        let mut grew = false;
        for (index, name) in names.iter().enumerate() {
            let collides = names.iter().enumerate().any(|(other, seen)| other != index && seen == name);
            let room = parts.get(index).is_some_and(|p| depth.get(index).is_some_and(|d| *d < p.len()));
            if collides
                && room
                && let Some(d) = next.get_mut(index)
            {
                *d += 1;
                grew = true;
            }
        }
        // Depth only ever grows and is bounded by the path's own length, so this settles.
        if !grew {
            return names;
        }
        depth = next;
    }
}

/// Grow a window outwards from the open tab while it still fits.
///
/// The open tab is always in the window, even when it alone overflows: the caller truncates it
/// rather than showing a row that does not say where you are. Nothing becomes unreachable by
/// being hidden, because `TAB` wraps through the whole set.
fn tab_row(widths: &[usize], current: usize, width: usize) -> TabRow {
    let total = widths.len();
    if total == 0 {
        return TabRow { hidden_before: 0, visible: 0..0, hidden_after: 0 };
    }
    let current = current.min(total - 1);
    let (mut start, mut end) = (current, current + 1);
    // One column per separator between neighbouring tabs, plus whatever the markers need.
    let cost = |start: usize, end: usize| -> usize {
        let labels: usize = widths.get(start..end).map_or(0, |w| w.iter().sum());
        let separators = end.saturating_sub(start).saturating_sub(1);
        labels + separators + marker_width(start) + marker_width(total - end)
    };
    loop {
        let grew_right = end < total && cost(start, end + 1) <= width;
        if grew_right {
            end += 1;
        }
        let grew_left = start > 0 && cost(start - 1, end) <= width;
        if grew_left {
            start -= 1;
        }
        if !grew_right && !grew_left {
            break;
        }
    }
    TabRow { hidden_before: start, visible: start..end, hidden_after: total - end }
}

fn is_markdown(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "md" | "markdown" | "mdx"))
}

fn is_hidden(path: &Path) -> bool {
    path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with('.'))
}

/// A real (non-symlinked) directory that is not a dependency or build tree.
fn is_walkable_dir(path: &Path) -> bool {
    let by_name = path.file_name().and_then(|n| n.to_str()).is_none_or(|n| !SKIPPED_DIRS.contains(&n));
    by_name && std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_dir())
}

/// One directory's Markdown files and walkable subdirectories, both sorted.
fn list(dir: &Path) -> Result<(Vec<PathBuf>, Vec<PathBuf>)> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| !is_hidden(p))
        .collect();
    entries.sort();
    let files = entries.iter().filter(|p| p.is_file() && is_markdown(p)).cloned().collect();
    let dirs = entries.iter().filter(|p| is_walkable_dir(p)).cloned().collect();
    Ok((files, dirs))
}

/// Markdown beneath `root`, breadth-first so the shallowest files come first. Stops at `limit`
/// files or `budget` visited entries, whichever comes first, so a huge tree stays fast.
///
/// An unreadable subdirectory is skipped rather than failing the walk; only an unreadable `root`
/// is an error worth reporting, because then there is nothing to show at all.
fn markdown_under(root: &Path, limit: usize, budget: usize) -> Result<Vec<PathBuf>> {
    let (mut found, dirs) = list(root)?;
    let mut queue = VecDeque::from(dirs);
    let mut seen = found.len();
    while let Some(dir) = queue.pop_front() {
        if found.len() >= limit || seen >= budget {
            break;
        }
        let Ok((files, dirs)) = list(&dir) else { continue };
        seen += files.len() + dirs.len();
        found.extend(files);
        queue.extend(dirs);
    }
    found.truncate(limit);
    Ok(found)
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    fn fixture(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("plannotator-tui-docs-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("docs/deep")).expect("mkdir");
        std::fs::create_dir_all(root.join(".hidden")).expect("mkdir");
        std::fs::create_dir_all(root.join("node_modules/pkg")).expect("mkdir");
        std::fs::write(root.join("b.md"), "").expect("write");
        std::fs::write(root.join("a.MD"), "").expect("write");
        std::fs::write(root.join("notes.txt"), "").expect("write");
        std::fs::write(root.join("docs/deep/plan.md"), "").expect("write");
        std::fs::write(root.join(".hidden/x.md"), "").expect("write");
        std::fs::write(root.join("node_modules/pkg/readme.md"), "").expect("write");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&root, root.join("loop")).expect("symlink");
        root
    }

    #[test]
    fn a_folder_becomes_the_markdown_beneath_it_shallowest_first() {
        let root = fixture("folder");
        let set = DocSet::of_folder(&root).expect("expands");
        let names: Vec<&str> = set.docs().iter().map(|d| d.name.as_str()).collect();
        // Hidden entries, node_modules, the symlink loop and non-Markdown are all skipped. None of
        // these three collide, so each keeps its bare file name.
        assert_eq!(names, ["a.MD", "b.md", "plan.md"]);
        std::fs::remove_dir_all(&root).expect("cleanup");
    }

    #[test]
    fn a_folder_with_no_markdown_is_an_error_that_names_it() {
        let root = std::env::temp_dir().join(format!("plannotator-tui-docs-empty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("mkdir");
        let err = DocSet::of_folder(&root).expect_err("nothing to show");
        assert!(err.to_string().contains(&root.display().to_string()), "{err}");
        std::fs::remove_dir_all(&root).expect("cleanup");
    }

    #[test]
    fn a_folder_walk_stops_at_its_limit() {
        let root = std::env::temp_dir().join(format!("plannotator-tui-docs-many-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("mkdir");
        for i in 0..10 {
            std::fs::write(root.join(format!("f{i}.md")), "").expect("write");
        }
        assert_eq!(markdown_under(&root, 4, WALK_BUDGET).expect("walks").len(), 4);
        std::fs::remove_dir_all(&root).expect("cleanup");
    }

    #[test]
    fn a_tab_is_named_by_its_file_and_grows_only_to_break_a_collision() {
        let root = PathBuf::from("/set");
        let set = DocSet::of_files(
            &root,
            &[root.join("two/doc.md"), root.join("one/doc.md"), root.join("plan.md")],
        );
        let names: Vec<&str> = set.docs().iter().map(|d| d.name.as_str()).collect();
        // The two `doc.md`s gain a parent each; `plan.md` collides with nothing and stays short.
        assert_eq!(names, ["one/doc.md", "plan.md", "two/doc.md"]);
    }

    #[test]
    fn a_collision_grows_only_as_far_as_it_must() {
        let root = PathBuf::from("/set");
        // Same file name three deep: one parent separates two of them, two are needed for the third.
        let set = DocSet::of_files(
            &root,
            &[root.join("a/api/spec.md"), root.join("b/api/spec.md"), root.join("c/spec.md")],
        );
        let mut names: Vec<&str> = set.docs().iter().map(|d| d.name.as_str()).collect();
        names.sort_unstable();
        assert_eq!(names, ["a/api/spec.md", "b/api/spec.md", "c/spec.md"]);
    }

    #[test]
    fn next_wraps_forward_and_is_nothing_to_do_for_one_document() {
        let root = PathBuf::from("/set");
        let mut set = DocSet::of_files(&root, &[root.join("a.md"), root.join("b.md")]);
        assert_eq!(set.next().map(|(i, _)| i), Some(1));
        set.focus(&root.join("b.md"));
        assert_eq!(set.next().map(|(i, _)| i), Some(0), "the last tab wraps to the first");
        let single = DocSet::of_files(&root, &[root.join("a.md")]);
        assert_eq!(single.next(), None);
    }

    #[test]
    fn the_open_tab_always_renders_and_the_rest_become_edge_counts() {
        // Six tabs of ten columns each, in a row that fits three.
        let widths = vec![10; 6];
        let row = tab_row(&widths, 0, 34);
        assert!(row.visible.contains(&0), "the open tab is in the window");
        assert_eq!(row.hidden_before, 0);
        assert_eq!(row.hidden_after, 6 - row.visible.end);
        assert!(row.hidden_after > 0, "a row this narrow cannot show six");

        // Open in the middle: the window grows both ways and both markers appear.
        let row = tab_row(&widths, 3, 40);
        assert!(row.visible.contains(&3));
        assert!(row.hidden_before > 0 && row.hidden_after > 0, "{row:?}");

        // Everything fits: no markers at all.
        let row = tab_row(&widths, 0, 200);
        assert_eq!(row, TabRow { hidden_before: 0, visible: 0..6, hidden_after: 0 });
    }

    #[test]
    fn a_tab_wider_than_the_row_is_still_the_one_shown() {
        let widths = vec![50, 10];
        let row = tab_row(&widths, 0, 12);
        assert_eq!(row.visible, 0..1, "the open tab stays, for the caller to truncate");
        assert_eq!(row.hidden_after, 1);
    }

    #[test]
    fn a_row_never_costs_more_than_its_width() {
        // Uneven labels, every open tab, every width: the drawn row must never overflow.
        let widths = vec![4, 12, 7, 30, 5, 9];
        for width in 1..60 {
            for current in 0..widths.len() {
                let row = tab_row(&widths, current, width);
                let labels: usize = widths.get(row.visible.clone()).map_or(0, |w| w.iter().sum::<usize>());
                let separators = row.visible.len().saturating_sub(1);
                let cost =
                    labels + separators + marker_width(row.hidden_before) + marker_width(row.hidden_after);
                // One tab alone may overflow; two or more never may.
                assert!(
                    cost <= width || row.visible.len() == 1,
                    "width {width}, current {current}: cost {cost} for {row:?}"
                );
            }
        }
    }
}
