//! Obsidian wiki-embeds: `![[image.png]]`.
//!
//! This is the one place the crate parses markup itself, and it is deliberate: `![[…]]` is not
//! Markdown. `pulldown-cmark` is right to hand it back as plain text, so no amount of walking
//! its event stream will find an embed. A vault written in Obsidian uses this form for every
//! attachment, which would otherwise be the one syntax the image renderer cannot see.
//!
//! It stays behind `[image] obsidian_embeds`, off by default: a `CommonMark` document that
//! happens to contain `![[x]]` means nothing by it, and should not suddenly grow a picture.
//!
//! Resolution follows Obsidian rather than the filesystem: a target may be a path relative to
//! the note, a path from the vault root, or **just a filename**, which Obsidian resolves
//! against the whole vault. The vault is the nearest ancestor holding `.obsidian`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Extensions worth decoding; `![[note.md]]` is a transclusion, not a picture, and is left
/// as text rather than rendered as something it is not.
const IMAGE_EXTENSIONS: [&str; 5] = ["png", "jpg", "jpeg", "webp", "gif"];

/// Directories never worth walking for an attachment.
const SKIP_DIRS: [&str; 3] = ["node_modules", "target", ".git"];

/// Upper bound on entries visited while indexing a vault, so a huge tree cannot stall a render.
const MAX_ENTRIES: usize = 20_000;

/// The image target of a paragraph that is exactly one embed, else `None`.
///
/// `![[a.png|300]]` and `![[a.png|alt text]]` carry a size or alias after a pipe; the target is
/// what precedes it. Anything beside the embed means the paragraph has content of its own and
/// keeps its normal rendering, matching the rule for `![alt](url)`.
pub(super) fn embed_target(source: &str) -> Option<&str> {
    let trimmed = source.trim();
    let inner = trimmed.strip_prefix("![[")?.strip_suffix("]]")?;
    // A second embed, a stray bracket or a line break means this is not one picture.
    if inner.contains("[[") || inner.contains("]]") || inner.contains('\n') {
        return None;
    }
    let target = inner.split('|').next().unwrap_or_default().trim();
    (!target.is_empty()).then_some(target)
}

/// Whether `path` names a file this crate can decode.
fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| IMAGE_EXTENSIONS.iter().any(|known| e.eq_ignore_ascii_case(known)))
}

/// The nearest ancestor of `from` that holds `.obsidian`, if any.
fn vault_root(from: &Path) -> Option<PathBuf> {
    from.ancestors().find(|dir| dir.join(".obsidian").is_dir()).map(Path::to_path_buf)
}

/// A vault's attachments, indexed by file name the way Obsidian resolves them.
#[derive(Debug)]
pub(super) struct Vault {
    root: Option<PathBuf>,
    /// Lowercased file name to path. First one wins, so a walk order change cannot flip which
    /// of two same-named files is chosen mid-session.
    by_name: HashMap<String, PathBuf>,
}

impl Vault {
    /// Find the vault around `base_dir` and index its images. Built once per document, and only
    /// when a document actually contains an embed.
    pub(super) fn around(base_dir: &Path) -> Self {
        let root = vault_root(base_dir);
        let by_name = root.as_deref().map(index_images).unwrap_or_default();
        Self { root, by_name }
    }

    /// Resolve an embed target: relative to the note, then from the vault root, then by name.
    pub(super) fn resolve(&self, target: &str, base_dir: &Path) -> Option<PathBuf> {
        let target = Path::new(target);
        if !is_image(target) {
            return None;
        }
        let relative = base_dir.join(target);
        if relative.is_file() {
            return Some(relative);
        }
        if let Some(root) = &self.root {
            let from_root = root.join(target);
            if from_root.is_file() {
                return Some(from_root);
            }
        }
        // Obsidian's shortest-path form: the file name alone, found anywhere in the vault.
        let name = target.file_name()?.to_str()?.to_lowercase();
        self.by_name.get(&name).cloned()
    }
}

/// Index every image under `root`, breadth-first, bounded.
fn index_images(root: &Path) -> HashMap<String, PathBuf> {
    let mut found: HashMap<String, PathBuf> = HashMap::new();
    let mut queue = vec![root.to_path_buf()];
    let mut visited = 0usize;

    while let Some(dir) = queue.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            visited += 1;
            if visited > MAX_ENTRIES {
                return found;
            }
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                // Hidden directories hold Obsidian's own state, not the user's attachments.
                if !name.starts_with('.') && !SKIP_DIRS.contains(&name) {
                    queue.push(path);
                }
            } else if is_image(&path) {
                found.entry(name.to_lowercase()).or_insert(path);
            }
        }
    }
    found
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    #[test]
    fn a_paragraph_that_is_exactly_one_embed_yields_its_target() {
        assert_eq!(embed_target("![[shot1.png]]"), Some("shot1.png"));
        assert_eq!(embed_target("  ![[dir/shot1.png]]\n"), Some("dir/shot1.png"));
        // A pipe carries Obsidian's size or alias; the target precedes it.
        assert_eq!(embed_target("![[shot1.png|300]]"), Some("shot1.png"));
        assert_eq!(embed_target("![[shot1.png|300x200]]"), Some("shot1.png"));
        assert_eq!(embed_target("![[shot1.png|a caption]]"), Some("shot1.png"));
    }

    #[test]
    fn anything_beside_the_embed_keeps_the_paragraph_as_text() {
        assert_eq!(embed_target("see ![[a.png]]"), None);
        assert_eq!(embed_target("![[a.png]] and ![[b.png]]"), None);
        assert_eq!(embed_target("![[a.png]]\n![[b.png]]"), None);
        // A link, not an embed: no leading bang.
        assert_eq!(embed_target("[[a.png]]"), None);
        assert_eq!(embed_target("![[]]"), None);
        assert_eq!(embed_target("plain text"), None);
    }

    #[test]
    fn only_decodable_extensions_resolve() {
        let vault = Vault { root: None, by_name: HashMap::new() };
        let base = Path::new("/vault/notes");
        // A note transclusion is not a picture.
        assert_eq!(vault.resolve("other-note.md", base), None);
        assert_eq!(vault.resolve("archive.zip", base), None);
    }

    /// A vault with `.obsidian` at the root, a note in a subfolder, and one attachment.
    fn scratch_vault(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("plannotator-tui-vault-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".obsidian")).expect("vault marker");
        std::fs::create_dir_all(root.join("notes")).expect("notes dir");
        std::fs::create_dir_all(root.join("attachments")).expect("attachments dir");
        let png = image::RgbaImage::new(2, 2);
        png.save(root.join("attachments/shot.png")).expect("writes png");
        root
    }

    #[test]
    fn a_bare_filename_resolves_anywhere_in_the_vault() {
        let root = scratch_vault("by-name");
        let notes = root.join("notes");
        let vault = Vault::around(&notes);
        assert_eq!(vault.resolve("shot.png", &notes), Some(root.join("attachments/shot.png")));
        // Case-insensitively, as on the platforms this runs on.
        assert_eq!(vault.resolve("SHOT.PNG", &notes), Some(root.join("attachments/shot.png")));
        assert_eq!(vault.resolve("missing.png", &notes), None);
    }

    #[test]
    fn a_vault_relative_path_resolves_from_the_root() {
        let root = scratch_vault("from-root");
        let notes = root.join("notes");
        let vault = Vault::around(&notes);
        assert_eq!(vault.resolve("attachments/shot.png", &notes), Some(root.join("attachments/shot.png")));
    }

    #[test]
    fn without_a_vault_only_note_relative_paths_resolve() {
        let dir = std::env::temp_dir().join(format!("plannotator-tui-novault-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        image::RgbaImage::new(2, 2).save(dir.join("beside.png")).expect("writes png");
        let vault = Vault::around(&dir);
        assert!(vault.root.is_none(), "no .obsidian anywhere above a temp dir");
        assert_eq!(vault.resolve("beside.png", &dir), Some(dir.join("beside.png")));
        assert_eq!(vault.resolve("elsewhere.png", &dir), None);
    }
}
