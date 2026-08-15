//! Reading mod archives.
//!
//! Everything is extracted into the mod's staging folder before it is
//! inspected: the detector works on a real directory tree, which keeps it
//! independent of the container format and means a mod that was shipped as a
//! plain folder is handled by exactly the same code path.

use std::fs::File;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum ArchiveError {
    #[error("i/o error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("unsupported archive format: {0}")]
    Unsupported(PathBuf),
    #[error("archive entry escapes the destination folder: {0}")]
    UnsafeEntry(String),
    #[error("could not read zip archive: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("could not read 7z archive: {0}")]
    SevenZ(String),
    #[error("could not read rar archive: {0}")]
    Rar(String),
}

impl ArchiveError {
    fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        ArchiveError::Io {
            path: path.into(),
            source,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveKind {
    Zip,
    SevenZ,
    Rar,
}

/// Identify an archive by its magic bytes, falling back to the extension.
///
/// Magic bytes come first because mods are routinely uploaded with the wrong
/// extension — a `.zip` that is really a RAR is common enough to matter.
pub fn detect_kind(path: &Path) -> Option<ArchiveKind> {
    if let Ok(kind) = kind_from_magic(path) {
        if kind.is_some() {
            return kind;
        }
    }
    match path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .as_deref()
    {
        Some("zip") => Some(ArchiveKind::Zip),
        Some("7z") => Some(ArchiveKind::SevenZ),
        Some("rar") => Some(ArchiveKind::Rar),
        _ => None,
    }
}

fn kind_from_magic(path: &Path) -> std::io::Result<Option<ArchiveKind>> {
    use std::io::Read;
    let mut file = File::open(path)?;
    let mut magic = [0u8; 8];
    let read = file.read(&mut magic)?;
    let magic = &magic[..read];

    if magic.starts_with(b"PK\x03\x04") || magic.starts_with(b"PK\x05\x06") {
        return Ok(Some(ArchiveKind::Zip));
    }
    if magic.starts_with(&[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C]) {
        return Ok(Some(ArchiveKind::SevenZ));
    }
    // "Rar!\x1a\x07\x00" (v4) and "Rar!\x1a\x07\x01\x00" (v5).
    if magic.starts_with(b"Rar!\x1a\x07") {
        return Ok(Some(ArchiveKind::Rar));
    }
    Ok(None)
}

/// Extract an archive into `dest`, which is created if needed.
pub fn extract(archive: &Path, dest: &Path) -> Result<(), ArchiveError> {
    std::fs::create_dir_all(dest).map_err(|e| ArchiveError::io(dest, e))?;
    match detect_kind(archive) {
        Some(ArchiveKind::Zip) => extract_zip(archive, dest),
        Some(ArchiveKind::SevenZ) => extract_7z(archive, dest),
        Some(ArchiveKind::Rar) => extract_rar(archive, dest),
        None => Err(ArchiveError::Unsupported(archive.to_path_buf())),
    }
}

fn extract_zip(archive: &Path, dest: &Path) -> Result<(), ArchiveError> {
    let file = File::open(archive).map_err(|e| ArchiveError::io(archive, e))?;
    let mut zip = zip::ZipArchive::new(file)?;

    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        // `enclosed_name` refuses absolute paths and `..` traversal.
        let Some(relative) = entry.enclosed_name() else {
            return Err(ArchiveError::UnsafeEntry(entry.name().to_string()));
        };
        let out = dest.join(&relative);

        if entry.is_dir() {
            std::fs::create_dir_all(&out).map_err(|e| ArchiveError::io(&out, e))?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| ArchiveError::io(parent, e))?;
        }
        let mut writer = File::create(&out).map_err(|e| ArchiveError::io(&out, e))?;
        std::io::copy(&mut entry, &mut writer).map_err(|e| ArchiveError::io(&out, e))?;
    }
    Ok(())
}

fn extract_7z(archive: &Path, dest: &Path) -> Result<(), ArchiveError> {
    sevenz_rust2::decompress_file(archive, dest)
        .map_err(|e| ArchiveError::SevenZ(e.to_string()))?;
    ensure_contained(dest)?;
    Ok(())
}

fn extract_rar(archive: &Path, dest: &Path) -> Result<(), ArchiveError> {
    let mut open = unrar::Archive::new(archive)
        .open_for_processing()
        .map_err(|e| ArchiveError::Rar(e.to_string()))?;

    while let Some(header) = open
        .read_header()
        .map_err(|e| ArchiveError::Rar(e.to_string()))?
    {
        let entry = header.entry();
        if entry.is_file() {
            let name = entry.filename.to_string_lossy().into_owned();
            if !is_safe_relative(Path::new(&name)) {
                return Err(ArchiveError::UnsafeEntry(name));
            }
            open = header
                .extract_with_base(dest)
                .map_err(|e| ArchiveError::Rar(e.to_string()))?;
        } else {
            open = header.skip().map_err(|e| ArchiveError::Rar(e.to_string()))?;
        }
    }
    ensure_contained(dest)?;
    Ok(())
}

/// Reject absolute paths and any `..` component.
fn is_safe_relative(path: &Path) -> bool {
    if path.is_absolute() {
        return false;
    }
    path.components()
        .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

/// Belt-and-braces check that nothing landed outside `dest`, for the formats
/// where extraction is delegated to a third-party crate.
fn ensure_contained(dest: &Path) -> Result<(), ArchiveError> {
    let root = dest
        .canonicalize()
        .map_err(|e| ArchiveError::io(dest, e))?;
    for entry in walkdir::WalkDir::new(dest).into_iter().flatten() {
        if let Ok(resolved) = entry.path().canonicalize() {
            if !resolved.starts_with(&root) {
                return Err(ArchiveError::UnsafeEntry(
                    entry.path().to_string_lossy().into_owned(),
                ));
            }
        }
    }
    Ok(())
}

/// Every file under `dir`, as paths relative to `dir`.
///
/// This is what feeds the detector, so directories are omitted — the tree
/// infers them from the files.
pub fn walk_relative(dir: &Path) -> Vec<PathBuf> {
    walkdir::WalkDir::new(dir)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| e.path().strip_prefix(dir).ok().map(Path::to_path_buf))
        .collect()
}

/// Total size on disk of an extracted mod, for display.
pub fn directory_size(dir: &Path) -> u64 {
    walkdir::WalkDir::new(dir)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| e.metadata().ok())
        .map(|m| m.len())
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn make_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let file = File::create(path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let options: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
        for (name, data) in entries {
            writer.start_file(*name, options).unwrap();
            writer.write_all(data).unwrap();
        }
        writer.finish().unwrap();
    }

    #[test]
    fn extracts_a_zip_and_lists_its_files() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("mod.zip");
        make_zip(
            &archive,
            &[
                ("Outfit_P.pak", b"pak"),
                ("nested/readme.txt", b"hello"),
            ],
        );

        let dest = dir.path().join("out");
        extract(&archive, &dest).unwrap();

        let mut files: Vec<String> = walk_relative(&dest)
            .iter()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .collect();
        files.sort();
        assert_eq!(files, vec!["Outfit_P.pak", "nested/readme.txt"]);
    }

    #[test]
    fn identifies_a_zip_by_its_magic_bytes_despite_a_wrong_extension() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("actually-a-zip.rar");
        make_zip(&archive, &[("a.pak", b"x")]);
        assert_eq!(detect_kind(&archive), Some(ArchiveKind::Zip));
    }

    #[test]
    fn rejects_a_zip_entry_that_escapes_the_destination() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("evil.zip");
        make_zip(&archive, &[("../escaped.txt", b"nope")]);

        let dest = dir.path().join("out");
        let err = extract(&archive, &dest).unwrap_err();
        assert!(matches!(err, ArchiveError::UnsafeEntry(_)), "got {err:?}");
    }

    #[test]
    fn reports_an_unknown_format() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.md");
        std::fs::write(&path, "not an archive").unwrap();
        assert!(matches!(
            extract(&path, &dir.path().join("out")).unwrap_err(),
            ArchiveError::Unsupported(_)
        ));
    }

    #[test]
    fn measures_extracted_size() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a"), b"12345").unwrap();
        assert_eq!(directory_size(dir.path()), 5);
    }
}
