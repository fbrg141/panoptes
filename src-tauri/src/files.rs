//! Files the composer offers: the workspace listing behind @-mentions, and
//! files dropped onto the composer. Neither depends on a harness.

use std::path::Path;

use base64::Engine;
use serde::Serialize;

/// The most files a mention picker lists; a home folder opened as a
/// workspace must not walk the whole disk.
pub const MAX_FILES: usize = 20_000;

/// Images larger than this are sent as a path, not inlined.
const MAX_IMAGE_BYTES: u64 = 20 * 1024 * 1024;

/// Files under `root`, relative to it with `/` separators, sorted. Honours
/// `.gitignore` and skips hidden files, as the agent's own tools would.
pub fn list(root: &Path, limit: usize) -> Vec<String> {
    let mut files: Vec<String> = ignore::WalkBuilder::new(root)
        .build()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_some_and(|kind| kind.is_file()))
        .filter_map(|entry| {
            let relative = entry.path().strip_prefix(root).ok()?;
            Some(relative.to_string_lossy().replace('\\', "/"))
        })
        .take(limit)
        .collect();
    files.sort();
    files
}

/// A file dropped onto the composer.
#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    pub name: String,
    pub path: String,
    /// Set for images the model can be sent inline.
    pub mime_type: Option<String>,
    /// The image bytes, base64-encoded, when `mime_type` is set.
    pub data: Option<String>,
}

fn image_type(path: &Path) -> Option<&'static str> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match extension.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => return None,
    })
}

/// Reads a dropped file: images inline, anything else by path.
pub fn attachment(path: &Path) -> anyhow::Result<Attachment> {
    let metadata = std::fs::metadata(path)?;
    anyhow::ensure!(metadata.is_file(), "{} is not a file", path.display());
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let (mime_type, data) = match image_type(path) {
        Some(mime) if metadata.len() <= MAX_IMAGE_BYTES => {
            let bytes = std::fs::read(path)?;
            let data = base64::engine::general_purpose::STANDARD.encode(bytes);
            (Some(mime.to_string()), Some(data))
        }
        _ => (None, None),
    };
    Ok(Attachment {
        name,
        path: path.to_string_lossy().into_owned(),
        mime_type,
        data,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listing_honours_gitignore_and_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // `ignore` reads .gitignore only inside a repository.
        std::fs::create_dir(root.join(".git")).unwrap();
        std::fs::write(root.join(".gitignore"), "target/\n").unwrap();
        std::fs::create_dir_all(root.join("src/deep")).unwrap();
        std::fs::create_dir(root.join("target")).unwrap();
        std::fs::write(root.join("src/deep/b.rs"), "").unwrap();
        std::fs::write(root.join("src/a.rs"), "").unwrap();
        std::fs::write(root.join("target/out.o"), "").unwrap();
        std::fs::write(root.join(".env"), "").unwrap();

        assert_eq!(list(root, 100), ["src/a.rs", "src/deep/b.rs"]);
        assert_eq!(list(root, 1).len(), 1);
    }

    #[test]
    fn images_are_inlined_and_other_files_go_by_path() {
        let dir = tempfile::tempdir().unwrap();
        let image = dir.path().join("Shot.PNG");
        std::fs::write(&image, [1, 2, 3]).unwrap();
        let notes = dir.path().join("notes.txt");
        std::fs::write(&notes, "hi").unwrap();

        let read = attachment(&image).unwrap();
        assert_eq!(read.name, "Shot.PNG");
        assert_eq!(read.mime_type.as_deref(), Some("image/png"));
        assert_eq!(read.data.as_deref(), Some("AQID"));

        let read = attachment(&notes).unwrap();
        assert_eq!((read.mime_type, read.data), (None, None));
        assert!(attachment(dir.path()).is_err());
    }
}
