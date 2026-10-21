//! Telling one build of the game from another.
//!
//! Mods are made against a particular build, and an update can both break
//! them and put the original files back over the ones the manager deployed.
//! Steam records the build it installed in the app manifest next to the game,
//! which is exact; any other install falls back to the shipping executable's
//! size and modification time, which an update changes too.

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::{vdf, GameInstall, STEAM_APP_ID};

/// What identifies the installed build.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameBuild {
    /// Steam's build id, when Steam installed the game.
    pub build_id: Option<String>,
    pub exe_size: Option<u64>,
    /// Seconds since the Unix epoch.
    pub exe_modified: Option<u64>,
}

impl GameBuild {
    pub fn read(game: &GameInstall) -> Self {
        let exe = game
            .executable_path()
            .and_then(|exe| std::fs::metadata(exe).ok());
        GameBuild {
            build_id: steam_build_id(&game.root),
            exe_size: exe.as_ref().map(|meta| meta.len()),
            exe_modified: exe
                .and_then(|meta| meta.modified().ok())
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|age| age.as_secs()),
        }
    }

    /// Whether `other` is the same build. Steam's build id decides when both
    /// have one; otherwise the executable does.
    pub fn same_as(&self, other: &GameBuild) -> bool {
        match (&self.build_id, &other.build_id) {
            (Some(a), Some(b)) => a == b,
            _ => self.exe_size == other.exe_size && self.exe_modified == other.exe_modified,
        }
    }

    /// Nothing could be read, so nothing can be compared.
    pub fn is_unknown(&self) -> bool {
        self.build_id.is_none() && self.exe_size.is_none()
    }
}

/// The manifest Steam keeps for a game it installed: two folders up from the
/// install, beside `common/`.
fn app_manifest(root: &Path) -> Option<PathBuf> {
    let steamapps = root.parent()?.parent()?;
    Some(steamapps.join(format!("appmanifest_{STEAM_APP_ID}.acf")))
}

fn steam_build_id(root: &Path) -> Option<String> {
    let text = std::fs::read_to_string(app_manifest(root)?).ok()?;
    vdf::parse(&text)
        .get("AppState")?
        .text_at("buildid")
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::InstallSource;
    use std::fs;

    fn install(root: &Path) -> GameInstall {
        fs::create_dir_all(root.join("SB/Binaries/Win64")).unwrap();
        fs::write(root.join("SB/Binaries/Win64/SB-Win64-Shipping.exe"), b"exe").unwrap();
        GameInstall {
            root: root.to_path_buf(),
            source: InstallSource::Steam,
            executable: Some("SB-Win64-Shipping.exe".into()),
        }
    }

    #[test]
    fn steam_says_which_build_is_installed() {
        let dir = tempfile::tempdir().unwrap();
        let steamapps = dir.path().join("steamapps");
        let game = install(&steamapps.join("common/StellarBlade"));
        fs::write(
            steamapps.join(format!("appmanifest_{STEAM_APP_ID}.acf")),
            "\"AppState\"\n{\n \"appid\" \"3489700\"\n \"buildid\" \"18734512\"\n}\n",
        )
        .unwrap();

        let before = GameBuild::read(&game);
        assert_eq!(before.build_id.as_deref(), Some("18734512"));

        fs::write(
            steamapps.join(format!("appmanifest_{STEAM_APP_ID}.acf")),
            "\"AppState\"\n{\n \"buildid\" \"19002211\"\n}\n",
        )
        .unwrap();
        assert!(!before.same_as(&GameBuild::read(&game)));
    }

    #[test]
    fn without_steam_the_executable_tells_builds_apart() {
        let dir = tempfile::tempdir().unwrap();
        let game = install(dir.path());
        let before = GameBuild::read(&game);
        assert!(before.build_id.is_none());
        assert!(!before.is_unknown());
        assert!(before.same_as(&GameBuild::read(&game)));

        fs::write(game.executable_path().unwrap(), b"a newer executable").unwrap();
        assert!(!before.same_as(&GameBuild::read(&game)));
    }
}
