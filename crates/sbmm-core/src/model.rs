use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::paths;

/// The kinds of mod payload we know how to place.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ModType {
    /// UE4SS itself: the DLL proxy plus the `ue4ss` runtime directory.
    Ue4ssFramework,
    /// UE4SS Blueprint mod. Must land in `LogicMods`, never in `~mods`.
    LogicMod,
    /// UE4SS Lua mod — a folder containing `scripts/main.lua`.
    Ue4ssLua,
    /// UE4SS C++ mod — a folder containing `dlls/main.dll`.
    Ue4ssDll,
    /// Movie/cutscene replacement.
    Movie,
    /// ReShade or a similar DLL proxy dropped next to the executable.
    RootBinary,
    /// Plain pak mod (with its IoStore siblings, when present).
    GenericPak,
    /// An archive that already encodes exact game-relative paths under `SB/`
    /// and does not fit any narrower category — laid down as-is. This is how
    /// CNS-style packages ship.
    GameRootOverlay,
    /// Files whose type we could not infer; the user is asked to choose.
    Unknown,
}

impl ModType {
    /// Game-root-relative directory this type installs into.
    ///
    /// `Ue4ssLua` and `Ue4ssDll` return the shared parent; the per-mod
    /// subfolder is appended during planning because it depends on the mod name.
    pub fn target_subdir(self) -> &'static str {
        match self {
            ModType::Ue4ssFramework | ModType::RootBinary => paths::BINARIES_WIN64,
            ModType::LogicMod => paths::PAKS_LOGICMODS,
            ModType::Ue4ssLua | ModType::Ue4ssDll => paths::UE4SS_MODS,
            ModType::Movie => paths::CONTENT_MOVIES,
            ModType::GenericPak => paths::PAKS_MODS,
            ModType::GameRootOverlay => paths::GAME_SUBDIR,
            ModType::Unknown => "",
        }
    }

    /// Whether this type participates in the alphanumeric pak load order.
    ///
    /// Only `~mods` is ordered this way. Logic mods are resolved by UE4SS
    /// itself, so renaming them to force an order would be wrong.
    pub fn is_load_ordered(self) -> bool {
        matches!(self, ModType::GenericPak)
    }

    /// Whether installing this type requires registering the mod folder in
    /// UE4SS's `mods.txt`.
    pub fn needs_ue4ss_registration(self) -> bool {
        matches!(self, ModType::Ue4ssLua | ModType::Ue4ssDll)
    }

    /// Types whose destination directory is flat — subfolders in the archive
    /// carry no meaning and are collapsed away.
    pub fn is_flat_target(self) -> bool {
        matches!(
            self,
            ModType::GenericPak | ModType::LogicMod | ModType::Movie
        )
    }

    /// Stable identifier used in the UI and in the database.
    pub fn as_str(self) -> &'static str {
        match self {
            ModType::Ue4ssFramework => "ue4ssFramework",
            ModType::LogicMod => "logicMod",
            ModType::Ue4ssLua => "ue4ssLua",
            ModType::Ue4ssDll => "ue4ssDll",
            ModType::Movie => "movie",
            ModType::RootBinary => "rootBinary",
            ModType::GenericPak => "genericPak",
            ModType::GameRootOverlay => "gameRootOverlay",
            ModType::Unknown => "unknown",
        }
    }

    pub fn from_str_id(s: &str) -> Option<Self> {
        Some(match s {
            "ue4ssFramework" => ModType::Ue4ssFramework,
            "logicMod" => ModType::LogicMod,
            "ue4ssLua" => ModType::Ue4ssLua,
            "ue4ssDll" => ModType::Ue4ssDll,
            "movie" => ModType::Movie,
            "rootBinary" => ModType::RootBinary,
            "genericPak" => ModType::GenericPak,
            "gameRootOverlay" => ModType::GameRootOverlay,
            "unknown" => ModType::Unknown,
            _ => return None,
        })
    }

    /// Every type the user may pick from when correcting a detection.
    pub fn all_selectable() -> &'static [ModType] {
        &[
            ModType::GenericPak,
            ModType::LogicMod,
            ModType::Ue4ssLua,
            ModType::Ue4ssDll,
            ModType::Ue4ssFramework,
            ModType::RootBinary,
            ModType::Movie,
            ModType::GameRootOverlay,
        ]
    }
}

/// How sure the detector is about a classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Confidence {
    /// An unambiguous structural marker was found (`scripts/main.lua`, a
    /// `LogicMods` directory, an explicit game-relative tree...).
    High,
    /// Inferred from file extensions alone.
    Medium,
    /// Nothing conclusive; the UI must ask before installing.
    AskUser,
}

/// One IoStore pak set. The three files must keep a common base name, and the
/// `_P` suffix must stay at the very end of that base name or the game can
/// fail to mount the archive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PakSet {
    /// Base name with any `_P` suffix removed.
    pub base: String,
    pub pak: PathBuf,
    pub utoc: Option<PathBuf>,
    pub ucas: Option<PathBuf>,
    /// Whether the original file already carried the `_P` suffix.
    pub had_p_suffix: bool,
}

impl PakSet {
    /// Every source file belonging to this set.
    pub fn files(&self) -> Vec<&PathBuf> {
        let mut out = vec![&self.pak];
        out.extend(self.utoc.iter());
        out.extend(self.ucas.iter());
        out
    }

    /// A `.utoc` without its `.ucas` (or vice versa) will not mount.
    pub fn is_complete(&self) -> bool {
        self.utoc.is_some() == self.ucas.is_some()
    }
}

/// A single file's journey from the staging folder into the game.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentFile {
    /// Relative to the extraction/staging root.
    pub source: PathBuf,
    /// Relative to the game root.
    pub target: PathBuf,
}

/// A distinct payload found inside one archive.
///
/// A single download frequently mixes several of these — a logic mod bundled
/// with a cosmetic pak, say — so detection returns a list rather than a single
/// verdict, and each entry is routed independently.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedComponent {
    pub mod_type: ModType,
    pub confidence: Confidence,
    pub files: Vec<ComponentFile>,
    /// Destination root relative to the game root; informational, for the UI.
    pub target_subdir: PathBuf,
    /// For UE4SS mods, the folder name the mod must be installed under (and
    /// registered as in `mods.txt`).
    pub ue4ss_mod_name: Option<String>,
    /// Pak sets found in this component, keyed by source path.
    pub pak_sets: Vec<PakSet>,
    /// Real problems the user should know about: a missing `_P` suffix, a
    /// half-complete IoStore set, an undetectable payload.
    pub warnings: Vec<String>,
    /// Descriptive detail shown in the install dialog. Not a problem, so it
    /// must not raise a warning marker in the mod list.
    pub notes: Vec<String>,
}

impl DetectedComponent {
    pub fn source_paths(&self) -> Vec<&PathBuf> {
        self.files.iter().map(|f| &f.source).collect()
    }
}
