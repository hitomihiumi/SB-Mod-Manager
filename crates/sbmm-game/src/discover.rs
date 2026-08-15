//! Locating the install without asking the user, when we can.

use std::path::{Path, PathBuf};

use crate::{
    find_shipping_exe, looks_like_game_root, vdf, GameInstall, InstallSource, STEAM_APP_ID,
};

#[derive(Debug, thiserror::Error)]
pub enum DiscoveryError {
    #[error("{0} does not look like a Stellar Blade install (no SB/Content/Paks folder)")]
    NotAGameFolder(PathBuf),
}

/// Every install we can find, best candidates first.
pub fn discover() -> Vec<GameInstall> {
    let mut found: Vec<GameInstall> = Vec::new();

    for root in steam_candidates() {
        push_unique(&mut found, root, InstallSource::Steam);
    }
    for root in epic_candidates() {
        push_unique(&mut found, root, InstallSource::Epic);
    }

    found
}

/// Accept a folder the user picked by hand.
///
/// Tolerates being handed the `SB` subfolder instead of the real root, which
/// is an easy mistake to make.
pub fn validate(path: impl AsRef<Path>) -> Result<GameInstall, DiscoveryError> {
    let path = path.as_ref();

    let root = if looks_like_game_root(path) {
        path.to_path_buf()
    } else if path
        .file_name()
        .is_some_and(|n| n.eq_ignore_ascii_case("SB"))
        && path.parent().is_some_and(looks_like_game_root)
    {
        path.parent().unwrap().to_path_buf()
    } else {
        return Err(DiscoveryError::NotAGameFolder(path.to_path_buf()));
    };

    Ok(GameInstall {
        executable: find_shipping_exe(&root),
        root,
        source: InstallSource::Manual,
    })
}

fn push_unique(found: &mut Vec<GameInstall>, root: PathBuf, source: InstallSource) {
    if !looks_like_game_root(&root) {
        return;
    }
    if found.iter().any(|i| i.root == root) {
        return;
    }
    found.push(GameInstall {
        executable: find_shipping_exe(&root),
        root,
        source,
    });
}

// ---------------------------------------------------------------------------
// Steam
// ---------------------------------------------------------------------------

fn steam_candidates() -> Vec<PathBuf> {
    let Some(steam_root) = steam_root() else {
        return Vec::new();
    };

    let mut roots = Vec::new();
    for library in steam_libraries(&steam_root) {
        // The app manifest is authoritative about the folder name, which does
        // not always match the store title.
        let manifest = library
            .join("steamapps")
            .join(format!("appmanifest_{STEAM_APP_ID}.acf"));
        let Ok(text) = std::fs::read_to_string(&manifest) else {
            continue;
        };
        let parsed = vdf::parse(&text);
        let Some(install_dir) = parsed
            .get("AppState")
            .and_then(|state| state.text_at("installdir"))
        else {
            continue;
        };
        roots.push(library.join("steamapps").join("common").join(install_dir));
    }
    roots
}

/// Library roots, including the Steam install itself.
fn steam_libraries(steam_root: &Path) -> Vec<PathBuf> {
    let mut libraries = vec![steam_root.to_path_buf()];

    let vdf_path = steam_root.join("steamapps").join("libraryfolders.vdf");
    if let Ok(text) = std::fs::read_to_string(&vdf_path) {
        let parsed = vdf::parse(&text);
        if let Some(folders) = parsed.get("libraryfolders") {
            for (_, entry) in folders.entries() {
                if let Some(path) = entry.text_at("path") {
                    let path = PathBuf::from(path);
                    if !libraries.contains(&path) {
                        libraries.push(path);
                    }
                }
            }
        }
    }
    libraries
}

#[cfg(windows)]
fn steam_root() -> Option<PathBuf> {
    if let Some(over) = std::env::var_os("SBMM_STEAM_ROOT") {
        return Some(PathBuf::from(over));
    }
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let key = hkcu.open_subkey(r"Software\Valve\Steam").ok()?;
    let path: String = key.get_value("SteamPath").ok()?;
    Some(PathBuf::from(path))
}

#[cfg(not(windows))]
fn steam_root() -> Option<PathBuf> {
    // Only used by tests and by developers running the app on Linux.
    std::env::var_os("SBMM_STEAM_ROOT").map(PathBuf::from)
}

// ---------------------------------------------------------------------------
// Epic Games Launcher
// ---------------------------------------------------------------------------

fn epic_candidates() -> Vec<PathBuf> {
    let manifests_dir = epic_manifests_dir();
    let Ok(entries) = std::fs::read_dir(&manifests_dir) else {
        return Vec::new();
    };

    let mut roots = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "item") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };

        let display = json
            .get("DisplayName")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let app = json
            .get("AppName")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !display.contains("stellar blade") && !app.contains("stellarblade") {
            continue;
        }

        if let Some(location) = json.get("InstallLocation").and_then(|v| v.as_str()) {
            roots.push(PathBuf::from(location));
        }
    }
    roots
}

fn epic_manifests_dir() -> PathBuf {
    if let Some(over) = std::env::var_os("SBMM_EPIC_MANIFESTS") {
        return PathBuf::from(over);
    }
    PathBuf::from(r"C:\ProgramData\Epic\EpicGamesLauncher\Data\Manifests")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn make_game(root: &Path) {
        fs::create_dir_all(root.join("SB/Content/Paks")).unwrap();
        fs::create_dir_all(root.join("SB/Binaries/Win64")).unwrap();
        fs::write(root.join("SB/Binaries/Win64/SB-Win64-Shipping.exe"), "").unwrap();
    }

    #[test]
    fn validates_a_real_game_folder() {
        let dir = tempfile::tempdir().unwrap();
        make_game(dir.path());

        let install = validate(dir.path()).unwrap();
        assert_eq!(install.root, dir.path());
        assert_eq!(install.executable.as_deref(), Some("SB-Win64-Shipping.exe"));
    }

    #[test]
    fn accepts_the_sb_subfolder_and_walks_up() {
        let dir = tempfile::tempdir().unwrap();
        make_game(dir.path());

        let install = validate(dir.path().join("SB")).unwrap();
        assert_eq!(install.root, dir.path());
    }

    #[test]
    fn rejects_an_unrelated_folder() {
        let dir = tempfile::tempdir().unwrap();
        assert!(validate(dir.path()).is_err());
    }

    #[test]
    fn finds_a_steam_install_through_the_library_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let steam = dir.path().join("Steam");
        let library = dir.path().join("SteamLibrary");
        fs::create_dir_all(steam.join("steamapps")).unwrap();
        fs::create_dir_all(library.join("steamapps")).unwrap();

        fs::write(
            steam.join("steamapps/libraryfolders.vdf"),
            format!(
                "\"libraryfolders\"\n{{\n \"0\" {{ \"path\" \"{}\" }}\n}}\n",
                library.display()
            ),
        )
        .unwrap();
        fs::write(
            library.join(format!("steamapps/appmanifest_{STEAM_APP_ID}.acf")),
            "\"AppState\"\n{\n \"installdir\" \"StellarBlade\"\n}\n",
        )
        .unwrap();

        let game_root = library.join("steamapps/common/StellarBlade");
        make_game(&game_root);

        // SAFETY: single-threaded test setting a process-wide override.
        unsafe { std::env::set_var("SBMM_STEAM_ROOT", &steam) };
        let found = steam_candidates();
        unsafe { std::env::remove_var("SBMM_STEAM_ROOT") };

        assert!(
            found.contains(&game_root),
            "expected {game_root:?} in {found:?}"
        );
    }
}
