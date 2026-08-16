//! Reading the table of contents of an IoStore container.
//!
//! A `.utoc` is a fixed 0x90-byte header followed by several arrays whose
//! lengths the header gives, and then the directory index — the part that
//! holds the actual file names. Everything before the directory index is
//! skipped by arithmetic rather than parsed, because none of it says anything
//! about which assets are present: a container with fifty thousand chunks is
//! read in two seeks.
//!
//! Layout confirmed against Epic's `FIoStoreTocHeader` as implemented by
//! `trumank/retoc`, which is the reference for the on-disk shape.

use std::collections::BTreeSet;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::{Asset, AssetError, Result};

const MAGIC: &[u8; 16] = b"-==--==--==--==-";
/// Epic asserts this exact size; a different one means a format we do not know.
const HEADER_SIZE: u32 = 0x90;

/// Sizes of the arrays sitting between the header and the directory index.
const CHUNK_ID_SIZE: u64 = 12;
const OFFSET_AND_LENGTH_SIZE: u64 = 10;
const PERFECT_HASH_SEED_SIZE: u64 = 4;
const SIGNATURE_HASH_SIZE: u64 = 20;

/// Versions, in the order Epic added them. Only the ordering matters here.
mod version {
    pub const PERFECT_HASH: u8 = 4;
    pub const PERFECT_HASH_WITH_OVERFLOW: u8 = 5;
}

mod flags {
    pub const ENCRYPTED: u8 = 0b0010;
    pub const SIGNED: u8 = 0b0100;
}

/// Everything the container's directory index names.
pub fn read(path: &Path) -> Result<BTreeSet<Asset>> {
    let mut file = std::fs::File::open(path).map_err(|e| AssetError::io(path, e))?;

    let mut header = [0u8; HEADER_SIZE as usize];
    file.read_exact(&mut header)
        .map_err(|_| AssetError::NotAToc(path.to_path_buf()))?;
    let header = Header::parse(path, &header)?;

    if header.flags & flags::ENCRYPTED != 0 {
        return Err(AssetError::Encrypted(path.to_path_buf()));
    }
    if header.directory_index_size == 0 {
        // Built without a directory index. The chunk ids are still identities
        // two mods can clash on, so they are read instead of giving up.
        return chunk_ids(&mut file, &header, path);
    }

    let offset = directory_index_offset(&mut file, &header, path)?;
    file.seek(SeekFrom::Start(offset))
        .map_err(|e| AssetError::io(path, e))?;
    let mut blob = vec![0u8; header.directory_index_size as usize];
    file.read_exact(&mut blob).map_err(|_| {
        AssetError::malformed(path, "the directory index runs past the end of file")
    })?;

    parse_directory_index(path, &blob)
}

struct Header {
    version: u8,
    entry_count: u32,
    compressed_block_count: u32,
    compressed_block_size: u32,
    compression_name_count: u32,
    compression_name_length: u32,
    directory_index_size: u32,
    flags: u8,
    perfect_hash_seed_count: u32,
    chunks_without_perfect_hash: u32,
}

impl Header {
    fn parse(path: &Path, bytes: &[u8; HEADER_SIZE as usize]) -> Result<Header> {
        if &bytes[..16] != MAGIC {
            return Err(AssetError::NotAToc(path.to_path_buf()));
        }
        let u32_at = |offset: usize| -> u32 {
            u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("fixed slice"))
        };

        if u32_at(20) != HEADER_SIZE {
            return Err(AssetError::UnsupportedVersion(path.to_path_buf()));
        }

        Ok(Header {
            version: bytes[16],
            entry_count: u32_at(24),
            compressed_block_count: u32_at(28),
            compressed_block_size: u32_at(32),
            compression_name_count: u32_at(36),
            compression_name_length: u32_at(40),
            directory_index_size: u32_at(48),
            flags: bytes[80],
            perfect_hash_seed_count: u32_at(84),
            chunks_without_perfect_hash: u32_at(96),
        })
    }

    /// Where the signature block would start, which is everything before it
    /// added up. All of these lengths are in the header.
    fn offset_before_signatures(&self) -> u64 {
        HEADER_SIZE as u64
            + self.chunk_ids_size()
            + self.entry_count as u64 * OFFSET_AND_LENGTH_SIZE
            + self.hash_map_size()
            + self.compressed_block_count as u64 * self.compressed_block_size as u64
            + self.compression_name_count as u64 * self.compression_name_length as u64
    }

    fn chunk_ids_size(&self) -> u64 {
        self.entry_count as u64 * CHUNK_ID_SIZE
    }

    /// The perfect-hash tables, which only exist from version 4 onwards and
    /// gained a second array in version 5.
    fn hash_map_size(&self) -> u64 {
        if self.version < version::PERFECT_HASH {
            return 0;
        }
        let overflow = if self.version >= version::PERFECT_HASH_WITH_OVERFLOW {
            self.chunks_without_perfect_hash as u64
        } else {
            0
        };
        (self.perfect_hash_seed_count as u64 + overflow) * PERFECT_HASH_SEED_SIZE
    }

    fn is_signed(&self) -> bool {
        self.flags & flags::SIGNED != 0
    }
}

/// Where the directory index starts.
///
/// Everything before it is skipped by arithmetic except the signature block,
/// whose length is written into the stream rather than the header — so a
/// signed container costs one extra four-byte read and nothing else.
fn directory_index_offset(file: &mut std::fs::File, header: &Header, path: &Path) -> Result<u64> {
    let before_signatures = header.offset_before_signatures();
    if !header.is_signed() {
        return Ok(before_signatures);
    }

    file.seek(SeekFrom::Start(before_signatures))
        .map_err(|e| AssetError::io(path, e))?;
    let mut size = [0u8; 4];
    file.read_exact(&mut size).map_err(|_| {
        AssetError::malformed(path, "the signature block runs past the end of file")
    })?;
    let size = u32::from_le_bytes(size) as u64;

    // A signature over the toc, one over the block table, then a hash per
    // compression block.
    Ok(before_signatures
        + 4
        + size * 2
        + header.compressed_block_count as u64 * SIGNATURE_HASH_SIZE)
}

/// A container with no directory index still has chunk ids, and two mods
/// shipping the same chunk still collide.
fn chunk_ids(file: &mut std::fs::File, header: &Header, path: &Path) -> Result<BTreeSet<Asset>> {
    file.seek(SeekFrom::Start(HEADER_SIZE as u64))
        .map_err(|e| AssetError::io(path, e))?;

    let mut ids = BTreeSet::new();
    let mut raw = vec![0u8; header.chunk_ids_size() as usize];
    file.read_exact(&mut raw)
        .map_err(|_| AssetError::malformed(path, "the chunk table runs past the end of file"))?;

    for chunk in raw.chunks_exact(CHUNK_ID_SIZE as usize) {
        ids.insert(Asset::Chunk(hex(chunk)));
    }
    Ok(ids)
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// The directory index: a mount point, then three flat arrays that form a tree
/// by index — directories, files, and the strings both refer to.
pub fn parse_directory_index(path: &Path, blob: &[u8]) -> Result<BTreeSet<Asset>> {
    let mut reader = Cursor::new(path, blob);

    let mount_point = reader.string()?;
    let directories: Vec<DirectoryEntry> = reader.array(DirectoryEntry::parse)?;
    let files: Vec<FileEntry> = reader.array(FileEntry::parse)?;
    let strings: Vec<String> = reader.array(Cursor::string)?;

    let mut assets = BTreeSet::new();
    if directories.is_empty() {
        return Ok(assets);
    }

    // The mount point is where the container attaches in the game's tree, and
    // the names below it are relative to it.
    let prefix = crate::normalise(mount_point.trim_start_matches("../"));
    let mut stack = Vec::new();
    walk(
        0,
        &directories,
        &files,
        &strings,
        &prefix,
        &mut stack,
        &mut assets,
        0,
    );
    Ok(assets)
}

/// Deep enough for any real asset tree, and a bound so a corrupt index that
/// points a directory at itself cannot spin forever.
const MAX_DEPTH: usize = 64;

#[allow(clippy::too_many_arguments)]
fn walk(
    dir: usize,
    directories: &[DirectoryEntry],
    files: &[FileEntry],
    strings: &[String],
    prefix: &str,
    stack: &mut Vec<String>,
    out: &mut BTreeSet<Asset>,
    depth: usize,
) {
    if depth > MAX_DEPTH {
        return;
    }
    let Some(entry) = directories.get(dir) else {
        return;
    };

    // The root has no name of its own; every other level contributes one.
    let named = match entry.name.and_then(|i| strings.get(i as usize)) {
        Some(name) => {
            stack.push(name.clone());
            true
        }
        None => false,
    };

    let mut file = entry.first_file;
    while let Some(index) = file {
        let Some(entry) = files.get(index as usize) else {
            break;
        };
        if let Some(name) = strings.get(entry.name as usize) {
            let mut full = String::new();
            if !prefix.is_empty() {
                full.push_str(prefix);
                if !full.ends_with('/') {
                    full.push('/');
                }
            }
            for part in stack.iter() {
                full.push_str(part);
                full.push('/');
            }
            full.push_str(name);
            out.insert(Asset::path(full));
        }
        file = entry.next_file;
    }

    let mut child = entry.first_child;
    while let Some(index) = child {
        walk(
            index as usize,
            directories,
            files,
            strings,
            prefix,
            stack,
            out,
            depth + 1,
        );
        child = directories.get(index as usize).and_then(|d| d.next_sibling);
    }

    if named {
        stack.pop();
    }
}

struct DirectoryEntry {
    name: Option<u32>,
    first_child: Option<u32>,
    next_sibling: Option<u32>,
    first_file: Option<u32>,
}

impl DirectoryEntry {
    fn parse(reader: &mut Cursor<'_>) -> Result<DirectoryEntry> {
        Ok(DirectoryEntry {
            name: reader.index()?,
            first_child: reader.index()?,
            next_sibling: reader.index()?,
            first_file: reader.index()?,
        })
    }
}

struct FileEntry {
    name: u32,
    next_file: Option<u32>,
}

impl FileEntry {
    fn parse(reader: &mut Cursor<'_>) -> Result<FileEntry> {
        let name = reader.u32()?;
        let next_file = reader.index()?;
        // user_data: the chunk this file lives in. Not needed to name it.
        let _ = reader.u32()?;
        Ok(FileEntry { name, next_file })
    }
}

/// A bounds-checked reader over the index blob.
struct Cursor<'a> {
    path: std::path::PathBuf,
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn new(path: &Path, bytes: &'a [u8]) -> Cursor<'a> {
        Cursor {
            path: path.to_path_buf(),
            bytes,
            at: 0,
        }
    }

    fn short(&self) -> AssetError {
        AssetError::malformed(&self.path, "the directory index ends mid-record")
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

    /// An index into one of the arrays. `u32::MAX` is Epic's "none".
    fn index(&mut self) -> Result<Option<u32>> {
        let value = self.u32()?;
        Ok((value != u32::MAX).then_some(value))
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

    /// A count followed by that many records.
    fn array<T>(&mut self, mut parse: impl FnMut(&mut Cursor<'a>) -> Result<T>) -> Result<Vec<T>> {
        let count = self.u32()? as usize;
        // A corrupt count could ask for gigabytes; the blob itself is the
        // ceiling, since no record is smaller than four bytes.
        if count > self.bytes.len() {
            return Err(self.short());
        }
        let mut out = Vec::with_capacity(count.min(1024));
        for _ in 0..count {
            out.push(parse(self)?);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a directory index blob the way Unreal writes one.
    struct IndexBuilder {
        strings: Vec<String>,
        directories: Vec<[u32; 4]>,
        files: Vec<[u32; 3]>,
        mount: String,
    }

    const NONE: u32 = u32::MAX;

    impl IndexBuilder {
        fn new(mount: &str) -> Self {
            Self {
                strings: Vec::new(),
                // The root directory, unnamed.
                directories: vec![[NONE, NONE, NONE, NONE]],
                files: Vec::new(),
                mount: mount.to_string(),
            }
        }

        fn intern(&mut self, name: &str) -> u32 {
            if let Some(i) = self.strings.iter().position(|s| s == name) {
                return i as u32;
            }
            self.strings.push(name.to_string());
            self.strings.len() as u32 - 1
        }

        /// Add a file at a slash-separated path, creating directories.
        fn add(&mut self, path: &str) {
            let mut parts: Vec<&str> = path.split('/').collect();
            let file_name = parts.pop().expect("a path has a last component");

            let mut dir = 0usize;
            for part in parts {
                let name = self.intern(part);
                let mut child = self.directories[dir][1];
                let mut found = None;
                let mut last = None;
                while child != NONE {
                    if self.directories[child as usize][0] == name {
                        found = Some(child as usize);
                        break;
                    }
                    last = Some(child as usize);
                    child = self.directories[child as usize][2];
                }
                dir = match found {
                    Some(existing) => existing,
                    None => {
                        self.directories.push([name, NONE, NONE, NONE]);
                        let new = self.directories.len() - 1;
                        match last {
                            Some(sibling) => self.directories[sibling][2] = new as u32,
                            None => self.directories[dir][1] = new as u32,
                        }
                        new
                    }
                };
            }

            let name = self.intern(file_name);
            self.files.push([name, NONE, self.files.len() as u32]);
            let new = self.files.len() as u32 - 1;

            let mut file = self.directories[dir][3];
            if file == NONE {
                self.directories[dir][3] = new;
            } else {
                while self.files[file as usize][1] != NONE {
                    file = self.files[file as usize][1];
                }
                self.files[file as usize][1] = new;
            }
        }

        fn build(&self) -> Vec<u8> {
            let mut out = Vec::new();
            let string = |out: &mut Vec<u8>, value: &str| {
                out.extend_from_slice(&(value.len() as u32 + 1).to_le_bytes());
                out.extend_from_slice(value.as_bytes());
                out.push(0);
            };

            string(&mut out, &self.mount);
            out.extend_from_slice(&(self.directories.len() as u32).to_le_bytes());
            for dir in &self.directories {
                for field in dir {
                    out.extend_from_slice(&field.to_le_bytes());
                }
            }
            out.extend_from_slice(&(self.files.len() as u32).to_le_bytes());
            for file in &self.files {
                for field in file {
                    out.extend_from_slice(&field.to_le_bytes());
                }
            }
            out.extend_from_slice(&(self.strings.len() as u32).to_le_bytes());
            let strings = self.strings.clone();
            for value in &strings {
                string(&mut out, value);
            }
            out
        }
    }

    fn paths(assets: &BTreeSet<Asset>) -> Vec<String> {
        assets.iter().map(|a| a.label().to_string()).collect()
    }

    #[test]
    fn the_directory_tree_is_flattened_into_full_paths() {
        let mut index = IndexBuilder::new("../../../");
        index.add("SB/Content/Characters/Eve/Body.uasset");
        index.add("SB/Content/Characters/Eve/Body.ubulk");
        index.add("SB/Content/Weapons/Blade.uasset");

        let assets = parse_directory_index(Path::new("test.utoc"), &index.build()).unwrap();
        assert_eq!(
            paths(&assets),
            vec![
                "sb/content/characters/eve/body.uasset",
                "sb/content/characters/eve/body.ubulk",
                "sb/content/weapons/blade.uasset",
            ]
        );
    }

    #[test]
    fn a_mount_point_is_prepended_to_every_path() {
        let mut index = IndexBuilder::new("../../../SB/Content/");
        index.add("Paks/Thing.uasset");

        let assets = parse_directory_index(Path::new("test.utoc"), &index.build()).unwrap();
        assert_eq!(paths(&assets), vec!["sb/content/paks/thing.uasset"]);
    }

    #[test]
    fn several_files_in_one_directory_are_all_followed() {
        let mut index = IndexBuilder::new("");
        for name in ["a.uasset", "b.uasset", "c.uasset", "d.uasset"] {
            index.add(&format!("Content/{name}"));
        }
        let assets = parse_directory_index(Path::new("test.utoc"), &index.build()).unwrap();
        assert_eq!(assets.len(), 4, "the file list is a chain, not an array");
    }

    #[test]
    fn an_empty_index_is_empty_rather_than_an_error() {
        let index = IndexBuilder::new("../../../");
        let assets = parse_directory_index(Path::new("test.utoc"), &index.build()).unwrap();
        assert!(assets.is_empty());
    }

    #[test]
    fn a_truncated_index_is_refused_rather_than_read_past_the_end() {
        let mut index = IndexBuilder::new("../../../");
        index.add("SB/Content/Thing.uasset");
        let blob = index.build();

        for cut in [1, blob.len() / 3, blob.len() / 2, blob.len() - 1] {
            let result = parse_directory_index(Path::new("test.utoc"), &blob[..cut]);
            assert!(result.is_err(), "a blob cut at {cut} should not parse");
        }
    }

    #[test]
    fn a_count_larger_than_the_blob_is_refused() {
        // A directory count claiming four billion entries must not allocate.
        let mut blob = vec![0u8; 4]; // empty mount point string
        blob.extend_from_slice(&u32::MAX.to_le_bytes());
        assert!(parse_directory_index(Path::new("test.utoc"), &blob).is_err());
    }

    #[test]
    fn a_file_that_is_not_a_toc_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fake.utoc");
        std::fs::write(&path, vec![b'x'; 512]).unwrap();
        assert!(matches!(read(&path), Err(AssetError::NotAToc(_))));
    }

    /// Build a whole `.utoc`, so the header arithmetic that finds the
    /// directory index is exercised rather than bypassed.
    fn utoc_file(index: &[u8], entry_count: u32, signed: bool) -> Vec<u8> {
        let mut header = Vec::new();
        header.extend_from_slice(MAGIC);
        header.push(5); // version: PerfectHashWithOverflow
        header.push(0); // reserved0
        header.extend_from_slice(&0u16.to_le_bytes()); // reserved1
        header.extend_from_slice(&HEADER_SIZE.to_le_bytes());
        header.extend_from_slice(&entry_count.to_le_bytes());
        header.extend_from_slice(&2u32.to_le_bytes()); // compressed block count
        header.extend_from_slice(&12u32.to_le_bytes()); // compressed block size
        header.extend_from_slice(&1u32.to_le_bytes()); // compression name count
        header.extend_from_slice(&32u32.to_le_bytes()); // compression name length
        header.extend_from_slice(&0x10000u32.to_le_bytes()); // compression block size
        header.extend_from_slice(&(index.len() as u32).to_le_bytes());
        header.extend_from_slice(&1u32.to_le_bytes()); // partition count
        header.extend_from_slice(&0u64.to_le_bytes()); // container id
        header.extend_from_slice(&[0u8; 16]); // encryption key guid
        header.push(if signed { flags::SIGNED } else { 0 });
        header.push(0); // reserved3
        header.extend_from_slice(&0u16.to_le_bytes()); // reserved4
        header.extend_from_slice(&3u32.to_le_bytes()); // perfect hash seed count
        header.extend_from_slice(&0u64.to_le_bytes()); // partition size
        header.extend_from_slice(&1u32.to_le_bytes()); // chunks without perfect hash
        header.extend_from_slice(&0u32.to_le_bytes()); // reserved7
        header.extend_from_slice(&[0u8; 40]); // reserved8
        assert_eq!(header.len(), HEADER_SIZE as usize);

        let mut file = header;
        file.extend_from_slice(&vec![0xAB; entry_count as usize * 12]); // chunk ids
        file.extend_from_slice(&vec![0u8; entry_count as usize * 10]); // offsets
        file.extend_from_slice(&[0u8; (3 + 1) * 4]); // hash map
        file.extend_from_slice(&[0u8; 2 * 12]); // compression blocks
        file.extend_from_slice(&[0u8; 32]); // one compression name

        if signed {
            let signature = 8usize;
            file.extend_from_slice(&(signature as u32).to_le_bytes());
            file.extend_from_slice(&vec![0u8; signature * 2]);
            file.extend_from_slice(&[0u8; 2 * 20]); // a hash per block
        }

        file.extend_from_slice(index);
        file
    }

    #[test]
    fn a_whole_container_is_read_from_its_header_through_to_its_names() {
        let mut index = IndexBuilder::new("../../../");
        index.add("SB/Content/Characters/Eve/Body.uasset");

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Outfit_P.utoc");
        std::fs::write(&path, utoc_file(&index.build(), 3, false)).unwrap();

        let assets = read(&path).unwrap();
        assert_eq!(
            paths(&assets),
            vec!["sb/content/characters/eve/body.uasset"]
        );
    }

    /// A signed container puts a block of unpredictable length in front of the
    /// directory index, and its length is in the stream rather than the header.
    #[test]
    fn a_signed_container_is_read_past_its_signatures() {
        let mut index = IndexBuilder::new("../../../");
        index.add("SB/Content/Weapons/Blade.uasset");

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Signed_P.utoc");
        std::fs::write(&path, utoc_file(&index.build(), 2, true)).unwrap();

        let assets = read(&path).unwrap();
        assert_eq!(paths(&assets), vec!["sb/content/weapons/blade.uasset"]);
    }

    /// Without a directory index the names are gone, but the chunk ids still
    /// identify what is in there — reporting nothing would hide a real clash.
    #[test]
    fn a_container_without_a_directory_index_falls_back_to_chunk_ids() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Nameless_P.utoc");
        std::fs::write(&path, utoc_file(&[], 3, false)).unwrap();

        let assets = read(&path).unwrap();
        assert_eq!(assets.len(), 1, "three identical chunk ids are one asset");
        assert!(!assets.iter().next().unwrap().is_named());
    }

    #[test]
    fn an_encrypted_container_says_so_rather_than_returning_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Locked_P.utoc");
        let mut file = utoc_file(&[], 1, false);
        file[80] = flags::ENCRYPTED;
        std::fs::write(&path, file).unwrap();

        assert!(matches!(read(&path), Err(AssetError::Encrypted(_))));
    }
}
