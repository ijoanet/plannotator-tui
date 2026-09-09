//! What git says changed in a document since `HEAD`, for the gutter's sign column.
//!
//! Split in two so the parsing needs no repository: `parse_hunks` is pure over the `@@` headers
//! of `git diff -U0`, and `change_bar` is the thin runner that asks git and maps the answer onto
//! the document's lines. Measured against `HEAD`, which is what "changed since the last commit"
//! means and covers staged and unstaged work together.
//!
//! A change bar is decoration. Like the art renderers it must never be able to fail a document,
//! so every way git can disappoint - no repository, no `HEAD` yet, no git at all, a binary file -
//! is simply no bars.

use std::ops::Range;
use std::path::Path;
use std::process::Command;

/// How a line differs from `HEAD`.
///
/// Untracked is its own kind rather than a synonym for added, the way `gitsigns.nvim`
/// distinguishes them: a file git has never seen is a different fact about a review than a line
/// that is new since the last commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChangeKind {
    Added,
    Changed,
    Deleted,
    Untracked,
}

/// A run of lines that differs from `HEAD`, in 1-based lines of the file on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Change {
    kind: ChangeKind,
    lines: Range<usize>,
}

/// The change bar for one document: one sign per source line, ready to index by row.
///
/// Built once when a document opens and once per `r`, never per frame: it costs a git call and a
/// scan of the source, while the gutter is drawn for every visible row of every frame.
#[derive(Debug, Clone, Default)]
pub(crate) struct ChangeBar {
    /// First byte of each line, for the byte-to-line lookup.
    line_starts: Vec<usize>,
    /// Per line, in the same order: the sign to draw, if any.
    signs: Vec<Option<ChangeKind>>,
}

impl ChangeBar {
    /// The sign for the line containing `byte`, if that line differs from `HEAD`.
    pub(crate) fn kind_at(&self, byte: usize) -> Option<ChangeKind> {
        let line = self.line_starts.partition_point(|start| *start <= byte).checked_sub(1)?;
        self.signs.get(line).copied().flatten()
    }

    /// `changes` mapped onto `source`'s lines.
    ///
    /// Added and changed runs are laid down first; a deleted bar then fills only a line with
    /// nothing else to say, because the row it lands on is not itself the line that went and
    /// must not claim to be unchanged elsewhere.
    fn build(source: &str, changes: &[Change]) -> Self {
        let line_starts = line_starts(source);
        let mut signs: Vec<Option<ChangeKind>> = vec![None; line_starts.len()];
        for change in changes.iter().filter(|c| c.kind != ChangeKind::Deleted) {
            for line in change.lines.clone() {
                if let Some(slot) = line.checked_sub(1).and_then(|i| signs.get_mut(i)) {
                    *slot = Some(change.kind);
                }
            }
        }
        for change in changes.iter().filter(|c| c.kind == ChangeKind::Deleted) {
            // A deletion past the last line has no row to follow it either, so the last row
            // carries it: otherwise removing the end of a document would show nothing at all.
            let line = change.lines.start.clamp(1, signs.len().max(1));
            if let Some(slot) = line.checked_sub(1).and_then(|i| signs.get_mut(i))
                && slot.is_none()
            {
                *slot = Some(ChangeKind::Deleted);
            }
        }
        Self { line_starts, signs }
    }

    /// Every line the same kind: a file git has never heard of, or one with no `HEAD` to
    /// compare against, where the whole file is the change.
    fn every_line(source: &str, kind: ChangeKind) -> Self {
        let line_starts = line_starts(source);
        let signs = vec![Some(kind); line_starts.len()];
        Self { line_starts, signs }
    }
}

/// First byte of every line the document actually has. A trailing newline ends the last line,
/// it does not start another, and `\r` belongs to the line it terminates rather than the next.
fn line_starts(source: &str) -> Vec<usize> {
    let mut starts = vec![0];
    starts.extend(source.match_indices('\n').map(|(at, _)| at + 1));
    if source.ends_with('\n') {
        starts.pop();
    }
    starts
}

/// The changed lines in `git diff -U0 --no-color HEAD -- <file>` output.
///
/// Only `@@` headers are read, so everything else git prints - `Binary files … differ`, rename
/// headers, `\ No newline at end of file` - contributes nothing instead of being miscounted.
fn parse_hunks(diff: &str) -> Vec<Change> {
    diff.lines().filter_map(hunk).collect()
}

/// One `@@ -a,b +c,d @@` header, where either count may be omitted and then means one line.
fn hunk(line: &str) -> Option<Change> {
    let (old, rest) = line.strip_prefix("@@ -")?.split_once(" +")?;
    let (new, _) = rest.split_once(" @@")?;
    let (_, removed) = span(old)?;
    let (start, added) = span(new)?;
    Some(match (removed, added) {
        // A removed line has no row of its own, so its bar goes on the row that now follows the
        // gap; git numbers a pure deletion by the line before it, and 0 when the file lost its
        // first lines.
        (_, 0) => Change { kind: ChangeKind::Deleted, lines: start + 1..start + 2 },
        (0, _) => Change { kind: ChangeKind::Added, lines: start..start + added },
        // Lines both went and arrived: the arrivals are what has a row, so they are the change.
        _ => Change { kind: ChangeKind::Changed, lines: start..start + added },
    })
}

/// `a,b`, or a bare `a` where the omitted count means one line.
fn span(text: &str) -> Option<(usize, usize)> {
    match text.split_once(',') {
        Some((start, count)) => Some((start.parse().ok()?, count.parse().ok()?)),
        None => Some((text.parse().ok()?, 1)),
    }
}

/// The change bar for `path`, measured against `HEAD`.
///
/// No bars at all when `signs` is off, when there is no file behind the document (stdin, a
/// reply), when the file is outside any repository, when the repository has no `HEAD` yet, or
/// when git will not run.
pub(crate) fn change_bar(path: Option<&Path>, source: &str, signs: bool) -> ChangeBar {
    if !signs {
        return ChangeBar::default();
    }
    let Some((path, dir)) = path.and_then(|p| Some((p, p.parent()?))) else {
        return ChangeBar::default();
    };
    if git(dir, &["rev-parse", "--show-toplevel"], None).is_none_or(|top| top.trim().is_empty()) {
        return ChangeBar::default();
    }
    // Untracked means git has never been told about the file, which is not the same as being
    // absent from `HEAD`: a newly `git add`ed file is tracked, and gitsigns calls its lines added.
    // Asking `ls-tree HEAD` instead reported every staged new file as untracked.
    if git(dir, &["ls-files", "--error-unmatch"], Some(path)).is_none() {
        return ChangeBar::every_line(source, ChangeKind::Untracked);
    }
    // Tracked, but there is no `HEAD` to diff against yet (a repository before its first commit),
    // so every line is new rather than unknown.
    if git(dir, &["rev-parse", "--verify", "HEAD"], None).is_none() {
        return ChangeBar::every_line(source, ChangeKind::Added);
    }
    let diff = git(dir, &["diff", "-U0", "--no-color", "HEAD"], Some(path)).unwrap_or_default();
    ChangeBar::build(source, &parse_hunks(&diff))
}

/// One git command in `dir`, with `file` as its pathspec, or `None` if it could not run or did
/// not succeed. The path goes in as one argument, so spaces in it need no quoting.
fn git(dir: &Path, args: &[&str], file: Option<&Path>) -> Option<String> {
    let mut command = Command::new("git");
    command.arg("-C").arg(dir).args(args);
    if let Some(file) = file {
        command.arg("--").arg(file);
    }
    let output = command.output().ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    /// The sign on the line containing the `n`th line's first byte (1-based).
    fn sign_on_line(bar: &ChangeBar, line: usize) -> Option<ChangeKind> {
        bar.kind_at(*bar.line_starts.get(line - 1).expect("the line exists"))
    }

    #[test]
    fn an_insertion_marks_only_its_new_lines_as_added() {
        let changes = parse_hunks("@@ -2,0 +3,2 @@\n+alpha\n+beta\n");
        let bar = ChangeBar::build("one\ntwo\nthree\nfour\nfive\n", &changes);
        assert_eq!(sign_on_line(&bar, 2), None, "the line before the insertion is untouched");
        assert_eq!(sign_on_line(&bar, 3), Some(ChangeKind::Added));
        assert_eq!(sign_on_line(&bar, 4), Some(ChangeKind::Added));
        assert_eq!(sign_on_line(&bar, 5), None, "the line after the insertion is untouched");
    }

    #[test]
    fn a_modification_marks_its_new_lines_as_changed() {
        let changes = parse_hunks("@@ -2,2 +2,2 @@\n-old one\n-old two\n+new one\n+new two\n");
        let bar = ChangeBar::build("one\ntwo\nthree\nfour\n", &changes);
        assert_eq!(sign_on_line(&bar, 1), None);
        assert_eq!(sign_on_line(&bar, 2), Some(ChangeKind::Changed));
        assert_eq!(sign_on_line(&bar, 3), Some(ChangeKind::Changed));
        assert_eq!(sign_on_line(&bar, 4), None);
    }

    #[test]
    fn a_deletion_marks_the_row_that_now_follows_the_gap() {
        // Old lines 3-5 are gone; the file's line 3 is what now follows them.
        let bar = ChangeBar::build("one\ntwo\nthree\nfour\n", &parse_hunks("@@ -3,3 +2,0 @@\n-gone\n"));
        assert_eq!(sign_on_line(&bar, 2), None, "the surviving line before the gap is unchanged");
        assert_eq!(sign_on_line(&bar, 3), Some(ChangeKind::Deleted));

        // A file that lost its very first lines is numbered from zero, and row 1 carries it.
        let top = ChangeBar::build("one\ntwo\n", &parse_hunks("@@ -1,2 +0,0 @@\n-gone\n"));
        assert_eq!(sign_on_line(&top, 1), Some(ChangeKind::Deleted));
    }

    #[test]
    fn a_hunk_header_without_counts_means_one_line() {
        let bar = ChangeBar::build("one\ntwo\nthree\n", &parse_hunks("@@ -2 +2 @@\n-old\n+new\n"));
        assert_eq!(sign_on_line(&bar, 1), None);
        assert_eq!(sign_on_line(&bar, 2), Some(ChangeKind::Changed), "one line, not none and not all");
        assert_eq!(sign_on_line(&bar, 3), None);
    }

    #[test]
    fn output_with_no_hunk_headers_leaves_the_document_unmarked() {
        // What a binary file, a rename and a missing final newline look like.
        let diff = "diff --git a/pic.png b/pic.png\nBinary files a/pic.png and b/pic.png differ\n\
                    similarity index 92%\n\\ No newline at end of file\n";
        assert_eq!(parse_hunks(diff), Vec::new());
        let bar = ChangeBar::build("one\ntwo\n", &parse_hunks(diff));
        assert_eq!(sign_on_line(&bar, 1), None);
        assert_eq!(sign_on_line(&bar, 2), None);
    }

    /// Asserted on byte offsets taken from the document itself, never on where the bar believes
    /// a line starts: a `\r` counted as its own byte on every line drifts the mapping one line at
    /// a time, and asking the bar for both halves of the question would hide that.
    #[test]
    fn crlf_lines_map_to_the_same_signs_as_lf() {
        let changes = parse_hunks("@@ -6 +6 @@\n-old\n+new\n");
        let unix = "a1\na2\na3\na4\na5\na6\na7\na8\n";
        let dos = "a1\r\na2\r\na3\r\na4\r\na5\r\na6\r\na7\r\na8\r\n";
        for source in [unix, dos] {
            let bar = ChangeBar::build(source, &changes);
            let at = |word: &str| source.find(word).expect("the word is in the document");
            assert_eq!(bar.kind_at(at("a6")), Some(ChangeKind::Changed), "line six of {source:?}");
            assert_eq!(bar.kind_at(at("a5")), None, "line five of {source:?}");
            assert_eq!(bar.kind_at(at("a7")), None, "line seven of {source:?}");
        }
    }

    #[test]
    fn a_document_without_a_trailing_newline_still_signs_its_last_line() {
        let bar = ChangeBar::build("one\ntwo", &parse_hunks("@@ -2 +2 @@\n-old\n+new\n"));
        assert_eq!(sign_on_line(&bar, 2), Some(ChangeKind::Changed));
        // The last byte of the file is on the last line, newline or no newline.
        assert_eq!(bar.kind_at("one\ntwo".len() - 1), Some(ChangeKind::Changed));
    }

    #[test]
    fn an_untracked_file_is_signed_on_every_line_and_never_as_added() {
        let bar = ChangeBar::every_line("one\ntwo\nthree\n", ChangeKind::Untracked);
        for line in 1..=3 {
            assert_eq!(sign_on_line(&bar, line), Some(ChangeKind::Untracked), "line {line}");
        }
        assert_eq!(bar.signs.len(), 3, "a trailing newline does not add a fourth line");
    }

    /// A scratch repository, and the path to a file inside it.
    fn scratch_repo(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("plannotator-tui-repo-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        for args in
            [vec!["init", "--quiet"], vec!["config", "user.email", "t@t"], vec!["config", "user.name", "t"]]
        {
            Command::new("git").arg("-C").arg(&dir).args(args).output().expect("git runs in tests");
        }
        dir
    }

    fn git_in(dir: &Path, args: &[&str]) {
        Command::new("git").arg("-C").arg(dir).args(args).output().expect("git");
    }

    #[test]
    fn a_staged_new_file_is_added_not_untracked() {
        // Untracked means git has never heard of the file. A `git add`ed file is tracked, and its
        // lines are added - which is what gitsigns shows. Keying on `HEAD` membership instead
        // reported every staged new file as untracked.
        let dir = scratch_repo("staged");
        std::fs::write(dir.join("seed.md"), "seed\n").expect("write");
        git_in(&dir, &["add", "seed.md"]);
        git_in(&dir, &["commit", "--quiet", "-m", "seed"]);

        let file = dir.join("new notes.md");
        std::fs::write(&file, "one\ntwo\n").expect("write");
        let before = change_bar(Some(&file), "one\ntwo\n", true);
        assert_eq!(before.kind_at(0), Some(ChangeKind::Untracked), "not yet added: untracked");

        git_in(&dir, &["add", "new notes.md"]);
        let after = change_bar(Some(&file), "one\ntwo\n", true);
        assert_eq!(after.kind_at(0), Some(ChangeKind::Added), "staged: added, not untracked");
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn a_tracked_file_in_a_repository_with_no_commit_yet_is_all_added() {
        // Nothing to diff against, so the whole file is new rather than unknown.
        let dir = scratch_repo("nohead");
        let file = dir.join("first.md");
        std::fs::write(&file, "one\ntwo\n").expect("write");
        git_in(&dir, &["add", "first.md"]);
        let bar = change_bar(Some(&file), "one\ntwo\n", true);
        assert_eq!(sign_on_line(&bar, 1), Some(ChangeKind::Added));
        assert_eq!(sign_on_line(&bar, 2), Some(ChangeKind::Added));
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn a_file_outside_a_repository_is_never_reported_as_changed() {
        let dir = std::env::temp_dir().join(format!("plannotator-tui-norepo-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let file = dir.join("loose notes.md");
        std::fs::write(&file, "one\ntwo\n").expect("write");
        let bar = change_bar(Some(&file), "one\ntwo\n", true);
        // No repository means no bars, not untracked ones: outside git there is nothing to be
        // untracked by. (The assumption is that a temp directory is not itself a repository. If
        // yours is, this test is what will tell you.)
        assert!(bar.signs.is_empty(), "a file outside a repository was given bars anyway");
        assert_eq!(bar.kind_at(0), None);
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }
}
