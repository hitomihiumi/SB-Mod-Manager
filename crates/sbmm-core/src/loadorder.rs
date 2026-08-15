//! Load order for `~mods`.
//!
//! Stellar Blade mounts loose paks in alphanumeric order, so priority is
//! expressed by rewriting the file name with a zero-padded numeric prefix. The
//! `_P` suffix has to stay at the end of the base name, and every file of an
//! IoStore set must keep the same base name, or the archive will not mount.

use std::path::{Path, PathBuf};

use crate::model::{ComponentFile, PakSet};

/// Width of the numeric prefix. Four digits keeps names sorting correctly well
/// past any realistic mod count.
const PREFIX_WIDTH: usize = 4;

/// Base name a pak set gets on disk at the given priority.
pub fn ordered_base(priority: i64, base: &str) -> String {
    let clamped = priority.clamp(0, 9999);
    format!("{:0width$}_{}_P", clamped, base, width = PREFIX_WIDTH)
}

/// Recover the author's original base name from a deployed file name.
///
/// Used by the UI so the list shows `MyCoolMod`, not `0100_MyCoolMod_P`.
pub fn strip_ordering(name: &str) -> &str {
    let without_ext = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
    let without_prefix = match without_ext.split_once('_') {
        Some((prefix, rest))
            if prefix.len() == PREFIX_WIDTH && prefix.chars().all(|c| c.is_ascii_digit()) =>
        {
            rest
        }
        _ => without_ext,
    };
    without_prefix
        .strip_suffix("_P")
        .or_else(|| without_prefix.strip_suffix("_p"))
        .unwrap_or(without_prefix)
}

/// Rewrite the destination names of every pak set so the game mounts them in
/// priority order. Files outside the given sets are left untouched.
pub fn apply_load_order(files: &mut [ComponentFile], pak_sets: &[PakSet], priority: i64) {
    for set in pak_sets {
        let new_base = ordered_base(priority, &set.base);
        for source in set.files() {
            let Some(ext) = extension_of(source) else {
                continue;
            };
            let new_name = format!("{new_base}.{ext}");
            for file in files.iter_mut() {
                if &file.source == source {
                    let dir = file
                        .target
                        .parent()
                        .map(Path::to_path_buf)
                        .unwrap_or_default();
                    file.target = dir.join(&new_name);
                }
            }
        }
    }
}

fn extension_of(path: &Path) -> Option<String> {
    path.extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
}

/// Convenience for building a deployed pak file name directly.
pub fn ordered_file_name(priority: i64, base: &str, ext: &str) -> PathBuf {
    PathBuf::from(format!("{}.{}", ordered_base(priority, base), ext))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ComponentFile;

    fn set() -> PakSet {
        PakSet {
            base: "CoolMod".into(),
            pak: PathBuf::from("CoolMod_P.pak"),
            utoc: Some(PathBuf::from("CoolMod_P.utoc")),
            ucas: Some(PathBuf::from("CoolMod_P.ucas")),
            had_p_suffix: true,
        }
    }

    #[test]
    fn renames_every_file_of_a_set_to_one_base_name() {
        let mut files: Vec<ComponentFile> = set()
            .files()
            .into_iter()
            .map(|p| ComponentFile {
                source: p.clone(),
                target: Path::new("SB/Content/Paks/~mods").join(p.file_name().unwrap()),
            })
            .collect();

        apply_load_order(&mut files, &[set()], 100);

        let names: Vec<String> = files
            .iter()
            .map(|f| f.target.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            vec![
                "0100_CoolMod_P.pak",
                "0100_CoolMod_P.utoc",
                "0100_CoolMod_P.ucas"
            ]
        );
    }

    #[test]
    fn adds_the_p_suffix_when_the_author_omitted_it() {
        assert_eq!(ordered_base(7, "Thing"), "0007_Thing_P");
    }

    #[test]
    fn round_trips_back_to_the_original_name() {
        assert_eq!(strip_ordering("0100_CoolMod_P.pak"), "CoolMod");
        assert_eq!(strip_ordering("CoolMod_P.pak"), "CoolMod");
        assert_eq!(strip_ordering("CoolMod.pak"), "CoolMod");
    }

    #[test]
    fn keeps_targets_in_their_directory() {
        let mut files = vec![ComponentFile {
            source: PathBuf::from("CoolMod_P.pak"),
            target: PathBuf::from("SB/Content/Paks/~mods/CoolMod_P.pak"),
        }];
        apply_load_order(&mut files, &[set()], 3);
        assert_eq!(
            files[0].target,
            PathBuf::from("SB/Content/Paks/~mods/0003_CoolMod_P.pak")
        );
    }
}
