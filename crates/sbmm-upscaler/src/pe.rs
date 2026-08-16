//! Reading a DLL's version without loading it.
//!
//! The version has to come from the file itself: the game folder may hold a
//! DLL somebody swapped in by hand, and its name says nothing about what it
//! is. Windows exposes `GetFileVersionInfo` for this, but the manager needs to
//! read these files on any machine — including the test suite — so the
//! `VS_FIXEDFILEINFO` structure is read out of the PE resource directory
//! directly.
//!
//! Only the fixed part is read. The string table underneath it is localised
//! and optional, whereas the fixed part is a plain struct behind a signature
//! that has not changed since Win32 was new.

use std::path::Path;

use crate::{Result, UpscalerError, Version};

/// `VS_FIXEDFILEINFO.dwSignature`, little-endian on disk.
const VS_FFI_SIGNATURE: [u8; 4] = [0xBD, 0x04, 0xEF, 0xFE];
/// Resource type 16 — `RT_VERSION`.
const RT_VERSION: u32 = 16;

/// The file version of a Windows DLL.
pub fn file_version(path: &Path) -> Result<Version> {
    let bytes = std::fs::read(path).map_err(|e| UpscalerError::io(path, e))?;
    parse(&bytes).ok_or_else(|| UpscalerError::NoVersion(path.to_path_buf()))
}

/// Same, but a file that cannot be read or parsed is simply unknown.
///
/// Used while scanning: one odd file in the game folder should not abort the
/// whole scan.
pub fn file_version_opt(path: &Path) -> Option<Version> {
    parse(&std::fs::read(path).ok()?)
}

/// Pull the version out of an in-memory PE image.
pub fn parse(bytes: &[u8]) -> Option<Version> {
    let rsrc = resource_section(bytes)?;
    let version_data = find_version_resource(bytes, &rsrc)?;

    // The fixed struct sits just past the `VS_VERSION_INFO` key and padding.
    // Searching for the signature skips having to replicate the alignment
    // rules, which are the only fiddly part of the layout.
    let start = find(version_data, &VS_FFI_SIGNATURE)?;
    let fixed = version_data.get(start..start + 24)?;

    // dwFileVersionMS at +8, dwFileVersionLS at +12.
    let ms = u32::from_le_bytes(fixed[8..12].try_into().ok()?);
    let ls = u32::from_le_bytes(fixed[12..16].try_into().ok()?);
    Some(Version(
        (ms >> 16) as u16,
        (ms & 0xffff) as u16,
        (ls >> 16) as u16,
        (ls & 0xffff) as u16,
    ))
}

/// Where `.rsrc` lives, in both address spaces.
struct Section {
    virtual_address: u32,
    raw_offset: u32,
    raw_size: u32,
}

impl Section {
    /// Resource entries address each other by RVA; the file is read by offset.
    fn offset_of(&self, rva: u32) -> Option<usize> {
        let delta = rva.checked_sub(self.virtual_address)?;
        if delta >= self.raw_size {
            return None;
        }
        Some((self.raw_offset + delta) as usize)
    }
}

fn resource_section(bytes: &[u8]) -> Option<Section> {
    if bytes.get(..2)? != b"MZ" {
        return None;
    }
    let pe_offset = u32::from_le_bytes(bytes.get(0x3c..0x40)?.try_into().ok()?) as usize;
    if bytes.get(pe_offset..pe_offset + 4)? != b"PE\0\0" {
        return None;
    }

    let section_count =
        u16::from_le_bytes(bytes.get(pe_offset + 6..pe_offset + 8)?.try_into().ok()?);
    let optional_size =
        u16::from_le_bytes(bytes.get(pe_offset + 20..pe_offset + 22)?.try_into().ok()?) as usize;
    let table = pe_offset + 24 + optional_size;

    for index in 0..section_count as usize {
        let entry = table + index * 40;
        let name = bytes.get(entry..entry + 8)?;
        if name != b".rsrc\0\0\0" {
            continue;
        }
        return Some(Section {
            virtual_address: u32::from_le_bytes(
                bytes.get(entry + 12..entry + 16)?.try_into().ok()?,
            ),
            raw_size: u32::from_le_bytes(bytes.get(entry + 16..entry + 20)?.try_into().ok()?),
            raw_offset: u32::from_le_bytes(bytes.get(entry + 20..entry + 24)?.try_into().ok()?),
        });
    }
    None
}

/// Walk type → name → language and return the bytes of the first RT_VERSION.
fn find_version_resource<'a>(bytes: &'a [u8], rsrc: &Section) -> Option<&'a [u8]> {
    let root = rsrc.offset_of(rsrc.virtual_address)?;

    for (id, child) in directory_entries(bytes, root)? {
        if id != RT_VERSION {
            continue;
        }
        // The two levels below the type are keyed by resource name and then by
        // language. A DLL carries one version resource, so the first branch of
        // each is the one wanted; picking a language would mean nothing here,
        // since the fixed part is the same in all of them.
        let names = rsrc.raw_offset as usize + (child & 0x7fff_ffff) as usize;
        let (_, language_dir) = first_entry(bytes, names)?;
        let languages = rsrc.raw_offset as usize + (language_dir & 0x7fff_ffff) as usize;
        let (_, leaf) = first_entry(bytes, languages)?;

        let entry = rsrc.raw_offset as usize + (leaf & 0x7fff_ffff) as usize;
        let data_rva = u32::from_le_bytes(bytes.get(entry..entry + 4)?.try_into().ok()?);
        let size = u32::from_le_bytes(bytes.get(entry + 4..entry + 8)?.try_into().ok()?) as usize;
        let start = rsrc.offset_of(data_rva)?;
        return bytes.get(start..start + size);
    }
    None
}

fn first_entry(bytes: &[u8], at: usize) -> Option<(u32, u32)> {
    directory_entries(bytes, at)?.into_iter().next()
}

/// The `(id, offset)` pairs of one resource directory.
///
/// The high bit of the offset marks a subdirectory; callers mask it off, since
/// the shape of the version resource tree is known in advance.
fn directory_entries(bytes: &[u8], at: usize) -> Option<Vec<(u32, u32)>> {
    let named = u16::from_le_bytes(bytes.get(at + 12..at + 14)?.try_into().ok()?) as usize;
    let by_id = u16::from_le_bytes(bytes.get(at + 14..at + 16)?.try_into().ok()?) as usize;

    let mut out = Vec::with_capacity(named + by_id);
    for index in 0..named + by_id {
        let entry = at + 16 + index * 8;
        let id = u32::from_le_bytes(bytes.get(entry..entry + 4)?.try_into().ok()?);
        let offset = u32::from_le_bytes(bytes.get(entry + 4..entry + 8)?.try_into().ok()?);
        out.push((id, offset));
    }
    Some(out)
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build the smallest PE that carries a version resource.
    ///
    /// Hand-built rather than checked in as a fixture: the real DLLs are tens
    /// of megabytes, and a synthetic one makes the layout being parsed
    /// explicit.
    fn pe_with_version(version: Version) -> Vec<u8> {
        const PE_OFFSET: usize = 0x80;
        const SECTION_RVA: u32 = 0x1000;
        const SECTION_RAW: u32 = 0x200;

        let mut image = vec![0u8; SECTION_RAW as usize];
        image[..2].copy_from_slice(b"MZ");
        image[0x3c..0x40].copy_from_slice(&(PE_OFFSET as u32).to_le_bytes());
        image[PE_OFFSET..PE_OFFSET + 4].copy_from_slice(b"PE\0\0");
        image[PE_OFFSET + 6..PE_OFFSET + 8].copy_from_slice(&1u16.to_le_bytes());
        let optional_size = 0xF0u16;
        image[PE_OFFSET + 20..PE_OFFSET + 22].copy_from_slice(&optional_size.to_le_bytes());

        // The resource section: three nested one-entry directories, then the
        // data entry, then VS_FIXEDFILEINFO.
        let mut rsrc = Vec::new();
        let dir = |entry_id: u32, child: u32| {
            let mut out = vec![0u8; 16];
            out[14..16].copy_from_slice(&1u16.to_le_bytes()); // one entry, by id
            out.extend_from_slice(&entry_id.to_le_bytes());
            out.extend_from_slice(&child.to_le_bytes());
            out
        };

        let type_dir = dir(RT_VERSION, 0x80000000 | 24);
        let name_dir = dir(1, 0x80000000 | 48);
        let lang_dir = dir(1033, 72);
        rsrc.extend_from_slice(&type_dir);
        rsrc.extend_from_slice(&name_dir);
        rsrc.extend_from_slice(&lang_dir);
        assert_eq!(rsrc.len(), 72);

        // Data entry: rva, size, codepage, reserved.
        let payload_offset = 88u32;
        let mut payload = vec![0u8; 40];
        payload[0..4].copy_from_slice(&VS_FFI_SIGNATURE);
        payload[8..12]
            .copy_from_slice(&(((version.0 as u32) << 16) | version.1 as u32).to_le_bytes());
        payload[12..16]
            .copy_from_slice(&(((version.2 as u32) << 16) | version.3 as u32).to_le_bytes());

        rsrc.extend_from_slice(&(SECTION_RVA + payload_offset).to_le_bytes());
        rsrc.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        rsrc.extend_from_slice(&0u32.to_le_bytes());
        rsrc.extend_from_slice(&0u32.to_le_bytes());
        assert_eq!(rsrc.len(), payload_offset as usize);
        rsrc.extend_from_slice(&payload);

        let table = PE_OFFSET + 24 + optional_size as usize;
        let mut header = vec![0u8; 40];
        header[..8].copy_from_slice(b".rsrc\0\0\0");
        header[12..16].copy_from_slice(&SECTION_RVA.to_le_bytes());
        header[16..20].copy_from_slice(&(rsrc.len() as u32).to_le_bytes());
        header[20..24].copy_from_slice(&SECTION_RAW.to_le_bytes());
        image[table..table + 40].copy_from_slice(&header);

        image.extend_from_slice(&rsrc);
        image
    }

    #[test]
    fn the_version_comes_out_of_the_resource_directory() {
        let image = pe_with_version(Version(310, 7, 0, 0));
        assert_eq!(parse(&image), Some(Version(310, 7, 0, 0)));
    }

    #[test]
    fn a_four_part_version_survives_intact() {
        let image = pe_with_version(Version(1, 0, 1, 41314));
        assert_eq!(parse(&image), Some(Version(1, 0, 1, 41314)));
    }

    #[test]
    fn anything_that_is_not_a_pe_is_refused_rather_than_misread() {
        assert_eq!(parse(b""), None);
        assert_eq!(parse(b"this is a text file"), None);
        // A truncated image must not panic on a slice out of range.
        let image = pe_with_version(Version(1, 2, 3, 4));
        assert_eq!(parse(&image[..image.len() / 2]), None);
    }

    #[test]
    fn a_pe_without_a_resource_section_has_no_version() {
        let mut image = pe_with_version(Version(1, 2, 3, 4));
        let table = 0x80 + 24 + 0xF0;
        image[table..table + 8].copy_from_slice(b".text\0\0\0");
        assert_eq!(parse(&image), None);
    }

    #[test]
    fn reading_a_missing_file_is_an_error_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nvngx_dlss.dll");
        assert!(file_version(&missing).is_err());
        assert_eq!(file_version_opt(&missing), None);
    }
}
