//! Reading the index of an Unreal `.pak`.
//!
//! A pak is written back to front: the footer is the last bytes of the file
//! and points at the index. The footer's own length depends on the version,
//! and the version is inside the footer — so the only way in is to try each
//! known length and see which one puts the magic where it belongs. That is
//! what UnrealPak itself does.
//!
//! From version 10 the index holds a *full directory index*: directory names,
//! then file names within each, which is all this needs. Older paks store the
//! names interleaved with version-dependent entry records, and rather than
//! half-guess those, they are refused with a clear reason. Stellar Blade is
//! UE 4.26 and writes version 11, so this only affects paks from other games.
//!
//! Layout confirmed against `trumank/repak`.

use std::collections::BTreeSet;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::{Asset, AssetError, Result};

const MAGIC: u32 = 0x5A6F_12E1;

/// The version that introduced the path hash and full directory indexes.
const VERSION_PATH_HASH_INDEX: u32 = 10;

/// Everything the pak's index names.
pub fn read(path: &Path) -> Result<BTreeSet<Asset>> {
    let mut file = std::fs::File::open(path).map_err(|e| AssetError::io(path, e))?;
    let length = file.metadata().map_err(|e| AssetError::io(path, e))?.len();

    let footer = find_footer(&mut file, length, path)?;
    if footer.encrypted {
        return Err(AssetError::Encrypted(path.to_path_buf()));
    }
    if footer.version < VERSION_PATH_HASH_INDEX {
        return Err(AssetError::UnsupportedVersion(path.to_path_buf()));
    }

    file.seek(SeekFrom::Start(footer.index_offset))
        .map_err(|e| AssetError::io(path, e))?;
    let mut index = vec![0u8; footer.index_size as usize];
    file.read_exact(&mut index)
        .map_err(|_| AssetError::malformed(path, "the index runs past the end of file"))?;

    let mut reader = Reader::new(path, &index);
    let mount_point = reader.string()?;
    let _entry_count = reader.u32()?;
    let _path_hash_seed = reader.u64()?;

    // Both sub-indexes are optional and live elsewhere in the file. Only the
    // second one carries names.
    reader.skip_optional_block()?;
    let Some((offset, size)) = reader.optional_block()? else {
        // No directory index means the pak lists nothing by name. That is a
        // real pak, just an opaque one, so it is empty rather than an error.
        return Ok(BTreeSet::new());
    };

    file.seek(SeekFrom::Start(offset))
        .map_err(|e| AssetError::io(path, e))?;
    let mut directory = vec![0u8; size as usize];
    file.read_exact(&mut directory).map_err(|_| {
        AssetError::malformed(path, "the directory index runs past the end of file")
    })?;

    parse_directory_index(path, &directory, &mount_point)
}

/// Directory names, each followed by the files inside it.
pub fn parse_directory_index(
    path: &Path,
    blob: &[u8],
    mount_point: &str,
) -> Result<BTreeSet<Asset>> {
    let mut reader = Reader::new(path, blob);
    let prefix = crate::normalise(mount_point.trim_start_matches("../"));

    let mut assets = BTreeSet::new();
    let directory_count = reader.counted()?;
    for _ in 0..directory_count {
        let directory = reader.string()?;
        let file_count = reader.counted()?;
        for _ in 0..file_count {
            let name = reader.string()?;
            // The entry offset, which says where the file is, not what it is.
            let _ = reader.u32()?;

            let mut full = String::new();
            if !prefix.is_empty() {
                full.push_str(&prefix);
                if !full.ends_with('/') {
                    full.push('/');
                }
            }
            full.push_str(directory.trim_start_matches('/'));
            if !full.is_empty() && !full.ends_with('/') {
                full.push('/');
            }
            full.push_str(&name);
            assets.insert(Asset::path(full));
        }
    }
    Ok(assets)
}

struct Footer {
    version: u32,
    index_offset: u64,
    index_size: u64,
    encrypted: bool,
}

/// Every footer length Unreal has written, newest first.
///
/// The version lives inside the footer, so the length cannot be known before
/// reading it. Trying the longest first matters: a short footer read at a long
/// offset can coincidentally land the magic somewhere plausible, whereas the
/// real one is checked against its own declared version.
const FOOTER_SIZES: &[u64] = &[
    // v11: uuid + encrypted + magic + version + offset + size + hash + 5 names
    16 + 1 + 4 + 4 + 8 + 8 + 20 + 32 * 5,
    // v8b: four compression names
    16 + 1 + 4 + 4 + 8 + 8 + 20 + 32 * 4,
    // v7: no compression names
    16 + 1 + 4 + 4 + 8 + 8 + 20,
    // v4: no encryption key guid
    1 + 4 + 4 + 8 + 8 + 20,
    // v3 and older: no encryption at all
    4 + 4 + 8 + 8 + 20,
];

fn find_footer(file: &mut std::fs::File, length: u64, path: &Path) -> Result<Footer> {
    for &size in FOOTER_SIZES {
        if length < size {
            continue;
        }
        file.seek(SeekFrom::End(-(size as i64)))
            .map_err(|e| AssetError::io(path, e))?;
        let mut raw = vec![0u8; size as usize];
        if file.read_exact(&mut raw).is_err() {
            continue;
        }

        // The magic sits after the optional uuid and encrypted flag, and where
        // those start is exactly what the footer length encodes.
        let head = size - (4 + 4 + 8 + 8 + 20) - compression_names(size);
        let Some(footer) = parse_footer(&raw, head as usize) else {
            continue;
        };
        return Ok(footer);
    }
    Err(AssetError::NotAPak(path.to_path_buf()))
}

/// How many bytes of the footer are compression method names.
fn compression_names(size: u64) -> u64 {
    match size {
        s if s >= 16 + 1 + 44 + 32 * 5 => 32 * 5,
        s if s >= 16 + 1 + 44 + 32 * 4 => 32 * 4,
        _ => 0,
    }
}

fn parse_footer(raw: &[u8], head: usize) -> Option<Footer> {
    let magic = u32::from_le_bytes(raw.get(head..head + 4)?.try_into().ok()?);
    if magic != MAGIC {
        return None;
    }
    let version = u32::from_le_bytes(raw.get(head + 4..head + 8)?.try_into().ok()?);
    // A version that does not fit the footer length means the magic landed
    // there by coincidence.
    if version == 0 || version > 20 {
        return None;
    }
    Some(Footer {
        version,
        index_offset: u64::from_le_bytes(raw.get(head + 8..head + 16)?.try_into().ok()?),
        index_size: u64::from_le_bytes(raw.get(head + 16..head + 24)?.try_into().ok()?),
        // The encrypted flag sits immediately before the magic when present.
        encrypted: head > 0 && raw[head - 1] == 1,
    })
}

/// A bounds-checked reader over an index blob.
struct Reader<'a> {
    path: std::path::PathBuf,
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn new(path: &Path, bytes: &'a [u8]) -> Reader<'a> {
        Reader {
            path: path.to_path_buf(),
            bytes,
            at: 0,
        }
    }

    fn short(&self) -> AssetError {
        AssetError::malformed(&self.path, "the index ends mid-record")
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        let end = self.at.checked_add(count).ok_or_else(|| self.short())?;
        let slice = self.bytes.get(self.at..end).ok_or_else(|| self.short())?;
        self.at = end;
        Ok(slice)
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(
            self.take(4)?.try_into().expect("fixed slice"),
        ))
    }

    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(
            self.take(8)?.try_into().expect("fixed slice"),
        ))
    }

    /// A count that cannot be larger than the blob it indexes into.
    fn counted(&mut self) -> Result<usize> {
        let count = self.u32()? as usize;
        if count > self.bytes.len() {
            return Err(self.short());
        }
        Ok(count)
    }

    /// An `FString`: a length, then bytes. A negative length means UTF-16.
    fn string(&mut self) -> Result<String> {
        let len = self.u32()? as i32;
        if len == 0 {
            return Ok(String::new());
        }
        if len < 0 {
            let count = len.unsigned_abs() as usize;
            let raw = self.take(count * 2)?;
            let wide: Vec<u16> = raw
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .take_while(|&c| c != 0)
                .collect();
            return Ok(String::from_utf16_lossy(&wide));
        }
        let raw = self.take(len as usize)?;
        let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
        Ok(String::from_utf8_lossy(&raw[..end]).into_owned())
    }

    /// A sub-index reference: a present flag, then offset, size and hash.
    fn optional_block(&mut self) -> Result<Option<(u64, u64)>> {
        if self.u32()? == 0 {
            return Ok(None);
        }
        let offset = self.u64()?;
        let size = self.u64()?;
        let _hash = self.take(20)?;
        Ok(Some((offset, size)))
    }

    fn skip_optional_block(&mut self) -> Result<()> {
        self.optional_block().map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn string(out: &mut Vec<u8>, value: &str) {
        out.extend_from_slice(&(value.len() as u32 + 1).to_le_bytes());
        out.extend_from_slice(value.as_bytes());
        out.push(0);
    }

    /// Build the blob a v11 pak's full directory index holds.
    fn directory_index(entries: &[(&str, &[&str])]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
        for (directory, files) in entries {
            string(&mut out, directory);
            out.extend_from_slice(&(files.len() as u32).to_le_bytes());
            for (index, file) in files.iter().enumerate() {
                string(&mut out, file);
                out.extend_from_slice(&(index as u32).to_le_bytes());
            }
        }
        out
    }

    fn paths(assets: &BTreeSet<Asset>) -> Vec<String> {
        assets.iter().map(|a| a.label().to_string()).collect()
    }

    #[test]
    fn directory_and_file_names_are_joined_under_the_mount_point() {
        let blob = directory_index(&[
            ("/Characters/Eve/", &["Body.uasset", "Body.uexp"][..]),
            ("/Weapons/", &["Blade.uasset"][..]),
        ]);
        let assets =
            parse_directory_index(Path::new("t.pak"), &blob, "../../../SB/Content/").unwrap();
        assert_eq!(
            paths(&assets),
            vec![
                "sb/content/characters/eve/body.uasset",
                "sb/content/characters/eve/body.uexp",
                "sb/content/weapons/blade.uasset",
            ]
        );
    }

    #[test]
    fn a_mod_rooted_at_the_game_needs_no_mount_prefix() {
        let blob = directory_index(&[("SB/Content/", &["Thing.uasset"][..])]);
        let assets = parse_directory_index(Path::new("t.pak"), &blob, "../../../").unwrap();
        assert_eq!(paths(&assets), vec!["sb/content/thing.uasset"]);
    }

    #[test]
    fn a_truncated_index_is_refused() {
        let blob = directory_index(&[("/A/", &["b.uasset"][..])]);
        for cut in [1, blob.len() / 2, blob.len() - 1] {
            assert!(
                parse_directory_index(Path::new("t.pak"), &blob[..cut], "").is_err(),
                "a blob cut at {cut} should not parse"
            );
        }
    }

    #[test]
    fn a_count_larger_than_the_blob_is_refused() {
        let mut blob = Vec::new();
        blob.extend_from_slice(&u32::MAX.to_le_bytes());
        assert!(parse_directory_index(Path::new("t.pak"), &blob, "").is_err());
    }

    /// The footer is found by trying each known length, so a file that is not
    /// a pak at all has to fall through all of them rather than match one.
    #[test]
    fn a_file_that_is_not_a_pak_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fake.pak");
        std::fs::write(&path, vec![0u8; 4096]).unwrap();
        assert!(matches!(read(&path), Err(AssetError::NotAPak(_))));
    }

    #[test]
    fn a_file_too_short_to_hold_a_footer_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tiny.pak");
        std::fs::write(&path, b"nope").unwrap();
        assert!(matches!(read(&path), Err(AssetError::NotAPak(_))));
    }

    /// Assembled end to end, because the footer offsets are the part most
    /// easily got wrong and the unit tests above bypass them.
    #[test]
    fn a_v11_pak_is_read_from_its_footer_through_to_its_names() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Outfit_P.pak");

        let directory = directory_index(&[("/Characters/", &["Eve.uasset"][..])]);

        // Payload, then the directory index, then the index, then the footer.
        let mut file = vec![0u8; 64];
        let directory_offset = file.len() as u64;
        file.extend_from_slice(&directory);

        let index_offset = file.len() as u64;
        let mut index = Vec::new();
        string(&mut index, "../../../SB/Content/");
        index.extend_from_slice(&1u32.to_le_bytes()); // entry count
        index.extend_from_slice(&0u64.to_le_bytes()); // path hash seed
        index.extend_from_slice(&0u32.to_le_bytes()); // no path hash index
        index.extend_from_slice(&1u32.to_le_bytes()); // a directory index follows
        index.extend_from_slice(&directory_offset.to_le_bytes());
        index.extend_from_slice(&(directory.len() as u64).to_le_bytes());
        index.extend_from_slice(&[0u8; 20]);
        let index_size = index.len() as u64;
        file.extend_from_slice(&index);

        file.extend_from_slice(&[0u8; 16]); // encryption key guid
        file.push(0); // not encrypted
        file.extend_from_slice(&MAGIC.to_le_bytes());
        file.extend_from_slice(&11u32.to_le_bytes());
        file.extend_from_slice(&index_offset.to_le_bytes());
        file.extend_from_slice(&index_size.to_le_bytes());
        file.extend_from_slice(&[0u8; 20]); // hash
        file.extend_from_slice(&[0u8; 32 * 5]); // compression names

        std::fs::write(&path, &file).unwrap();
        let assets = read(&path).unwrap();
        assert_eq!(paths(&assets), vec!["sb/content/characters/eve.uasset"]);
    }

    #[test]
    fn an_encrypted_index_says_so_rather_than_returning_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Locked_P.pak");

        let mut file = vec![0u8; 64];
        file.extend_from_slice(&[0u8; 16]);
        file.push(1); // encrypted
        file.extend_from_slice(&MAGIC.to_le_bytes());
        file.extend_from_slice(&11u32.to_le_bytes());
        file.extend_from_slice(&0u64.to_le_bytes());
        file.extend_from_slice(&0u64.to_le_bytes());
        file.extend_from_slice(&[0u8; 20]);
        file.extend_from_slice(&[0u8; 32 * 5]);

        std::fs::write(&path, &file).unwrap();
        assert!(matches!(read(&path), Err(AssetError::Encrypted(_))));
    }
}
