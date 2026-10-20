//! Paths inside the virtual game folder.
//!
//! The hook sees whatever the game passes to the file APIs: absolute paths,
//! `\\?\` long-path prefixes, forward and back slashes, `..` segments, and any
//! mixture of case. Windows treats all of those as the same file, so the
//! overlay has to as well — otherwise `SB\Content\Paks\~mods\X.pak` and
//! `sb/content/paks/~MODS/x.pak` would be two different files and a mod would
//! silently fail to load depending on how the engine spelled the path.

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::path::Path;

/// A path relative to the game root.
///
/// Compared, hashed and ordered by its lookup spelling — every component
/// lower-cased and joined with `/` — so two spellings of one file are one
/// key. The spelling it was made from is kept as well, because a file the
/// game creates has to appear under the name the game gave it. The empty
/// path is the root.
#[derive(Clone, Default)]
pub struct VPath {
    key: String,
    display: String,
}

impl PartialEq for VPath {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

impl Eq for VPath {}

impl Hash for VPath {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.key.hash(state);
    }
}

impl PartialOrd for VPath {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for VPath {
    fn cmp(&self, other: &Self) -> Ordering {
        self.key.cmp(&other.key)
    }
}

impl VPath {
    pub fn root() -> VPath {
        VPath::default()
    }

    pub fn is_root(&self) -> bool {
        self.key.is_empty()
    }

    /// Normalise a game-relative path.
    ///
    /// Absolute paths and `..` that climbs above the root are refused: the
    /// overlay only ever describes the game folder, and a mod path that leaves
    /// it is the same untrusted input the deploy backend refuses.
    pub fn from_relative(path: impl AsRef<Path>) -> Option<VPath> {
        let text = path.as_ref().to_string_lossy();
        if text.starts_with('/') || text.starts_with('\\') || has_drive(&text) {
            return None;
        }
        normalise(text.split(['/', '\\']))
    }

    /// The last component, as it was spelled.
    pub fn file_name(&self) -> &str {
        self.display.rsplit('/').next().unwrap_or("")
    }

    pub fn parent(&self) -> Option<VPath> {
        if self.is_root() {
            return None;
        }
        // Lower-casing never changes how many components there are, so the
        // two spellings split at the same place.
        Some(match (self.key.rfind('/'), self.display.rfind('/')) {
            (Some(k), Some(d)) => VPath {
                key: self.key[..k].to_string(),
                display: self.display[..d].to_string(),
            },
            _ => VPath::root(),
        })
    }

    /// Every directory above this path, the root first.
    pub fn ancestors(&self) -> Vec<VPath> {
        let mut out = Vec::new();
        let mut cursor = self.parent();
        while let Some(dir) = cursor {
            cursor = dir.parent();
            out.push(dir);
        }
        out.reverse();
        out
    }

    pub fn join(&self, name: &str) -> VPath {
        if self.is_root() {
            VPath {
                key: key(name),
                display: name.to_string(),
            }
        } else {
            VPath {
                key: format!("{}/{}", self.key, key(name)),
                display: format!("{}/{name}", self.display),
            }
        }
    }

    /// The lookup spelling.
    pub fn as_str(&self) -> &str {
        &self.key
    }

    /// The spelling the path was made from, `/`-separated.
    pub fn display(&self) -> &str {
        &self.display
    }

    /// Each component as it was spelled.
    pub fn components(&self) -> impl Iterator<Item = &str> {
        self.display.split('/').filter(|c| !c.is_empty())
    }

    /// Whether `self` is `other` or lies underneath it.
    pub fn starts_with(&self, other: &VPath) -> bool {
        other.is_root()
            || self.key == other.key
            || (self.key.starts_with(&other.key)
                && self.key.as_bytes().get(other.key.len()) == Some(&b'/'))
    }
}

impl fmt::Debug for VPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "VPath({:?})", self.display)
    }
}

impl fmt::Display for VPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.display)
    }
}

/// The lookup spelling of one name. Windows compares names case-insensitively
/// with its own upper-case table; Unicode lower-casing agrees with it for every
/// name a game ships.
pub fn key(name: &str) -> String {
    name.to_lowercase()
}

/// Where a path the game asked for lands inside the game folder, if it does.
///
/// `root` and `path` are both absolute Win32 paths. The `\\?\` and `\\.\`
/// prefixes are dropped, separators and case are ignored, and `.`/`..` are
/// resolved lexically, as `GetFullPathNameW` would. A path that only shares a
/// prefix (`C:\Games\SB2` against `C:\Games\SB`) is not inside.
pub fn relative_to_root(root: &str, path: &str) -> Option<VPath> {
    let root = absolute_components(root)?;
    let path = absolute_components(path)?;
    if path.len() < root.len() {
        return None;
    }
    if path.iter().zip(&root).any(|(a, b)| key(a) != key(b)) {
        return None;
    }
    normalise(path[root.len()..].iter().map(String::as_str))
}

/// Split an absolute path into its components, the drive or share first, with
/// `.` and `..` already applied.
fn absolute_components(path: &str) -> Option<Vec<String>> {
    let path = path
        .strip_prefix(r"\\?\UNC\")
        .map(|rest| format!(r"\\{rest}"))
        .unwrap_or_else(|| path.to_string());
    let path = path
        .strip_prefix(r"\\?\")
        .or_else(|| path.strip_prefix(r"\\.\"))
        .unwrap_or(&path);

    let mut parts = Vec::new();
    for part in path.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => {
                // The drive itself cannot be climbed out of.
                if parts.len() > 1 {
                    parts.pop();
                }
            }
            other => parts.push(other.to_string()),
        }
    }
    if parts.is_empty() {
        return None;
    }
    Some(parts)
}

fn normalise<'a>(parts: impl Iterator<Item = &'a str>) -> Option<VPath> {
    let mut out: Vec<&str> = Vec::new();
    for part in parts {
        match part {
            "" | "." => {}
            ".." => {
                out.pop()?;
            }
            // Windows ignores trailing dots and spaces on a name, which is how
            // `Mods.` and `Mods` end up being the same folder.
            other => {
                let trimmed = other.trim_end_matches(['.', ' ']);
                if trimmed.is_empty() {
                    return None;
                }
                out.push(trimmed);
            }
        }
    }
    let display = out.join("/");
    Some(VPath {
        key: key(&display),
        display,
    })
}

fn has_drive(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spelling_and_separators_do_not_matter() {
        assert_eq!(
            VPath::from_relative(r"SB\Content\Paks\~mods\Outfit_P.pak"),
            VPath::from_relative("sb/content/paks/~MODS/outfit_p.PAK"),
        );
        assert_eq!(
            VPath::from_relative("SB/./Binaries//Win64/")
                .unwrap()
                .as_str(),
            "sb/binaries/win64"
        );
    }

    #[test]
    fn paths_that_leave_the_game_folder_are_refused() {
        assert_eq!(VPath::from_relative("../outside.dll"), None);
        assert_eq!(VPath::from_relative("SB/../../outside.dll"), None);
        assert_eq!(VPath::from_relative(r"C:\Windows\System32\x.dll"), None);
        assert_eq!(VPath::from_relative("/etc/passwd"), None);
        assert_eq!(
            VPath::from_relative("SB/Binaries/../Content")
                .unwrap()
                .as_str(),
            "sb/content",
            "climbing that stays inside is fine"
        );
    }

    #[test]
    fn trailing_dots_and_spaces_are_dropped_as_windows_does() {
        assert_eq!(
            VPath::from_relative("SB/Mods. /x.txt").unwrap().as_str(),
            "sb/mods/x.txt"
        );
    }

    #[test]
    fn absolute_paths_are_placed_relative_to_the_game() {
        let root = r"C:\Games\Stellar Blade";
        assert_eq!(
            relative_to_root(
                root,
                r"c:\games\STELLAR BLADE\SB\Content\Paks\pakchunk0.pak"
            )
            .unwrap()
            .as_str(),
            "sb/content/paks/pakchunk0.pak"
        );
        assert_eq!(
            relative_to_root(
                root,
                r"\\?\C:\Games\Stellar Blade\SB\Binaries\Win64\..\..\Content"
            )
            .unwrap()
            .as_str(),
            "sb/content"
        );
        assert_eq!(
            relative_to_root(root, r"C:/Games/Stellar Blade").unwrap(),
            VPath::root()
        );
    }

    #[test]
    fn a_folder_that_only_shares_a_prefix_is_not_inside() {
        let root = r"C:\Games\SB";
        assert_eq!(relative_to_root(root, r"C:\Games\SB2\x.pak"), None);
        assert_eq!(relative_to_root(root, r"D:\Games\SB\x.pak"), None);
        assert_eq!(relative_to_root(root, r"C:\Games"), None);
    }

    #[test]
    fn the_original_spelling_is_kept_for_display() {
        let path = VPath::from_relative(r"SB\Saved\Config\Engine.ini").unwrap();
        assert_eq!(path.display(), "SB/Saved/Config/Engine.ini");
        assert_eq!(path.file_name(), "Engine.ini");
        assert_eq!(path.parent().unwrap().display(), "SB/Saved/Config");
        assert_eq!(
            path,
            VPath::from_relative("sb/saved/config/engine.ini").unwrap()
        );
    }

    #[test]
    fn ancestors_run_from_the_root_down() {
        let path = VPath::from_relative("SB/Content/Paks/x.pak").unwrap();
        let ancestors = path.ancestors();
        let names: Vec<&str> = ancestors.iter().map(VPath::as_str).collect();
        assert_eq!(names, vec!["", "sb", "sb/content", "sb/content/paks"]);
        assert!(path.starts_with(&VPath::from_relative("SB/Content").unwrap()));
        assert!(!path.starts_with(&VPath::from_relative("SB/Cont").unwrap()));
    }
}
