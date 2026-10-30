//! ReShade's settings file and its presets, read and changed as text.
//!
//! ReShade keeps its own settings in `ReShade.ini` beside the DLL that loads
//! it. Two of them matter here: `PresetPath`, the preset it starts with, and
//! `EffectSearchPaths`, where it looks for the `.fx` files presets use. A
//! preset is another INI file: a top-level `Techniques=` line naming the
//! effects switched on (`Name@File.fx`, or just `Name` in presets written by
//! ReShade 4 and older), and a `[File.fx]` section of values per effect.
//!
//! Paths in `ReShade.ini` are relative to its own folder unless absolute, and
//! written with backslashes and a leading `.\`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::ini;
use crate::model::{ComponentFile, Confidence, DetectedComponent, ModType};
use crate::paths;

/// ReShade's settings file, beside the executable.
pub const CONFIG_FILE: &str = "ReShade.ini";

/// The section of `ReShade.ini` the paths are in.
const GENERAL: &str = "GENERAL";

/// Where ReShade looks for effects when `ReShade.ini` does not say: the
/// folder its installer creates.
const DEFAULT_EFFECT_PATH: &str = r".\reshade-shaders\Shaders\**";

/// A folder ReShade searches, from `EffectSearchPaths`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchPath {
    /// As written, without the trailing `\**`.
    pub path: String,
    /// `\**` at the end: subfolders are searched too.
    pub recursive: bool,
}

/// What `ReShade.ini` says about presets and effects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// As written, e.g. `.\ReShadePreset.ini`.
    pub preset_path: Option<String>,
    pub effect_paths: Vec<SearchPath>,
}

/// Read `ReShade.ini`.
pub fn read_config(text: &str) -> Config {
    let preset_path = ini::get(text, GENERAL, "PresetPath").filter(|p| !p.trim().is_empty());
    let effects = ini::get(text, GENERAL, "EffectSearchPaths")
        .filter(|p| !p.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_EFFECT_PATH.to_string());
    Config {
        preset_path: preset_path.map(|p| unquote(&p).to_string()),
        effect_paths: search_paths(&effects),
    }
}

/// `ReShade.ini` with ReShade starting on the preset at `value`.
pub fn with_preset_path(text: &str, value: &str) -> String {
    ini::set(text, GENERAL, "PresetPath", value)
}

fn unquote(text: &str) -> &str {
    let text = text.trim();
    text.strip_prefix('"')
        .and_then(|t| t.strip_suffix('"'))
        .unwrap_or(text)
}

fn search_paths(value: &str) -> Vec<SearchPath> {
    value
        .split(',')
        .map(unquote)
        .filter(|p| !p.is_empty())
        .map(|p| {
            let trimmed = p.trim_end_matches(['\\', '/']);
            match trimmed.strip_suffix("**") {
                Some(rest) => SearchPath {
                    path: rest.trim_end_matches(['\\', '/']).to_string(),
                    recursive: true,
                },
                None => SearchPath {
                    path: trimmed.to_string(),
                    recursive: false,
                },
            }
        })
        .collect()
}

/// Whether a path from `ReShade.ini` is absolute rather than relative to it.
pub fn is_absolute(value: &str) -> bool {
    let value = value.trim();
    let bytes = value.as_bytes();
    value.starts_with(['\\', '/']) || (bytes.len() >= 2 && bytes[1] == b':')
}

/// A relative path from `ReShade.ini` as a game-relative one, given the
/// game-relative folder `ReShade.ini` is in. `None` for an absolute path or
/// one that climbs out of the game folder.
pub fn game_relative(base: &str, value: &str) -> Option<String> {
    if is_absolute(value) {
        return None;
    }
    let mut parts: Vec<&str> = base.split(['/', '\\']).filter(|s| !s.is_empty()).collect();
    for part in value.trim().split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// A game-relative path written the way ReShade writes paths, relative to
/// `base`: `.\reshade-presets\Bright.ini`. `None` when `path` is not inside
/// `base`.
pub fn relative_value(base: &str, path: &str) -> Option<String> {
    let base = base.replace('\\', "/").to_ascii_lowercase();
    let base = base.trim_end_matches('/');
    let normal = path.replace('\\', "/");
    let rest = normal
        .to_ascii_lowercase()
        .strip_prefix(&format!("{base}/"))
        .map(|_| &normal[base.len() + 1..])?;
    Some(format!(r".\{}", rest.replace('/', "\\")))
}

// -- presets -------------------------------------------------------------------

/// An effect switched on in a preset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Technique {
    pub name: String,
    /// The `.fx` file it is in, when the preset says (ReShade 5 and later).
    pub effect: Option<String>,
}

/// A preset, as far as the manager cares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preset {
    pub techniques: Vec<Technique>,
}

impl Preset {
    /// The effect files the switched-on techniques are in, each once.
    pub fn effects(&self) -> Vec<String> {
        let mut seen = BTreeSet::new();
        self.techniques
            .iter()
            .filter_map(|t| t.effect.clone())
            .filter(|e| seen.insert(e.to_lowercase()))
            .collect()
    }

    /// Techniques that do not name their file, so cannot be checked.
    pub fn unchecked(&self) -> usize {
        self.techniques
            .iter()
            .filter(|t| t.effect.is_none())
            .count()
    }

    /// The effects this preset uses that are not among `available`, which
    /// holds lower-cased file names.
    pub fn missing(&self, available: &BTreeSet<String>) -> Vec<String> {
        self.effects()
            .into_iter()
            .filter(|e| !available.contains(&e.to_lowercase()))
            .collect()
    }
}

/// Whether an INI file is a ReShade preset: it has a top-level `Techniques`
/// line, or a section named after an effect file.
pub fn is_preset(text: &str) -> bool {
    ini::get(text, "", "Techniques").is_some()
        || text.lines().any(|line| {
            let line = line.trim();
            line.starts_with('[') && line.to_ascii_lowercase().ends_with(".fx]")
        })
}

/// Read a preset, or `None` when the text is not one.
pub fn read_preset(text: &str) -> Option<Preset> {
    if !is_preset(text) {
        return None;
    }
    let techniques = ini::get(text, "", "Techniques")
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(|t| match t.split_once('@') {
            Some((name, effect)) => Technique {
                name: name.trim().to_string(),
                effect: Some(effect.trim().to_string()).filter(|e| !e.is_empty()),
            },
            None => Technique {
                name: t.to_string(),
                effect: None,
            },
        })
        .collect();
    Some(Preset { techniques })
}

// -- installing presets ----------------------------------------------------------

/// Extensions that are clearly not part of a preset archive's payload, so
/// can be left out beside presets without asking.
const BESIDE_PRESETS: &[&str] = &[
    "txt", "md", "rtf", "pdf", "jpg", "jpeg", "png", "webp", "gif", "bmp", "url", "html", "htm",
    "nfo", "log",
];

/// Turn the INI files of unrecognised components that are ReShade presets
/// into a preset component, read through `read` (relative to the staging
/// root, `None` when unreadable). A component left with nothing that could
/// be a payload is dropped, with a note of what was left out.
///
/// Presets install into `reshade-presets` beside the executable, flat when
/// their names allow it; two presets of one name in different folders keep
/// those folders, so neither is lost.
pub fn claim_presets(
    components: Vec<DetectedComponent>,
    read: impl Fn(&Path) -> Option<String>,
) -> Vec<DetectedComponent> {
    let mut presets: Vec<ComponentFile> = Vec::new();
    let mut out = Vec::new();
    let mut left_out = Vec::new();

    for component in components {
        if component.mod_type != ModType::Unknown {
            out.push(component);
            continue;
        }
        let (found, rest): (Vec<ComponentFile>, Vec<ComponentFile>) =
            component.files.iter().cloned().partition(|f| {
                extension(&f.source).as_deref() == Some("ini")
                    && read(&f.source).is_some_and(|text| is_preset(&text))
            });
        if found.is_empty() {
            out.push(component);
            continue;
        }
        presets.extend(found);
        let only_extras = rest.iter().all(|f| {
            extension(&f.source).is_some_and(|ext| BESIDE_PRESETS.contains(&ext.as_str()))
        });
        if only_extras {
            left_out.extend(rest.iter().map(|f| display_name(&f.source)));
        } else {
            out.push(DetectedComponent {
                files: rest,
                ..component
            });
        }
    }
    if presets.is_empty() {
        return out;
    }

    let files = preset_targets(presets);
    let mut notes = vec![
        "ReShade presets. They install into reshade-presets beside the game executable; \
         pick the one ReShade starts with on the ReShade screen."
            .to_string(),
    ];
    if !left_out.is_empty() {
        notes.push(format!("Not installed: {}.", left_out.join(", ")));
    }
    out.push(DetectedComponent {
        mod_type: ModType::ReshadePreset,
        confidence: Confidence::High,
        files,
        target_subdir: PathBuf::from(paths::RESHADE_PRESETS),
        ue4ss_mod_name: None,
        pak_sets: Vec::new(),
        warnings: Vec::new(),
        notes,
    });
    out
}

fn extension(path: &Path) -> Option<String> {
    path.extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

/// Flat when every name is distinct, otherwise relative to the folder all
/// the presets share.
fn preset_targets(presets: Vec<ComponentFile>) -> Vec<ComponentFile> {
    let names: BTreeSet<String> = presets
        .iter()
        .map(|f| display_name(&f.source).to_lowercase())
        .collect();
    let root = Path::new(paths::RESHADE_PRESETS);
    if names.len() == presets.len() {
        return presets
            .into_iter()
            .map(|f| ComponentFile {
                target: root.join(display_name(&f.source)),
                source: f.source,
            })
            .collect();
    }

    let parents: Vec<Vec<String>> = presets
        .iter()
        .map(|f| {
            f.target
                .parent()
                .map(|p| {
                    p.components()
                        .map(|c| c.as_os_str().to_string_lossy().into_owned())
                        .collect()
                })
                .unwrap_or_default()
        })
        .collect();
    let mut shared = parents.first().cloned().unwrap_or_default();
    for parent in &parents[1..] {
        let same = shared
            .iter()
            .zip(parent)
            .take_while(|(a, b)| a.eq_ignore_ascii_case(b))
            .count();
        shared.truncate(same);
    }
    presets
        .into_iter()
        .map(|f| {
            let rest: PathBuf = f.target.components().skip(shared.len()).collect();
            ComponentFile {
                target: root.join(rest),
                source: f.source,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const RESHADE_INI: &str = "[GENERAL]\r\nEffectSearchPaths=.\\reshade-shaders\\Shaders\\**,D:\\More Shaders\r\nPresetPath=.\\ReShadePreset.ini\r\n\r\n[INPUT]\r\nKeyOverlay=36,0,0,0\r\n";

    const PRESET: &str = "PreprocessorDefinitions=\r\nTechniques=Clarity@Clarity.fx,LumaSharpen@LumaSharpen.fx,Vibrance@Vibrance.fx,Tonemap\r\nTechniqueSorting=Clarity@Clarity.fx,LumaSharpen@LumaSharpen.fx\r\n\r\n[Clarity.fx]\r\nClarityRadius=3\r\n";

    #[test]
    fn reads_reshades_own_settings() {
        let config = read_config(RESHADE_INI);
        assert_eq!(config.preset_path.as_deref(), Some(r".\ReShadePreset.ini"));
        assert_eq!(
            config.effect_paths,
            [
                SearchPath {
                    path: r".\reshade-shaders\Shaders".into(),
                    recursive: true
                },
                SearchPath {
                    path: r"D:\More Shaders".into(),
                    recursive: false
                },
            ]
        );
        // The installer's folder when nothing is said.
        assert_eq!(
            read_config("[GENERAL]\n").effect_paths[0].path,
            r".\reshade-shaders\Shaders"
        );
    }

    #[test]
    fn switching_preset_changes_only_that_line() {
        let text = with_preset_path(RESHADE_INI, r".\reshade-presets\Bright.ini");
        assert_eq!(
            text,
            RESHADE_INI.replace(r".\ReShadePreset.ini", r".\reshade-presets\Bright.ini")
        );
        let fresh = with_preset_path("", r".\A.ini");
        assert_eq!(read_config(&fresh).preset_path.as_deref(), Some(r".\A.ini"));
    }

    #[test]
    fn paths_resolve_against_the_settings_folder() {
        let base = "SB/Binaries/Win64";
        assert_eq!(
            game_relative(base, r".\reshade-presets\A.ini").as_deref(),
            Some("SB/Binaries/Win64/reshade-presets/A.ini")
        );
        assert_eq!(
            game_relative(base, "A.ini").as_deref(),
            Some("SB/Binaries/Win64/A.ini")
        );
        assert_eq!(
            game_relative(base, r"..\..\Presets\A.ini").as_deref(),
            Some("SB/Presets/A.ini")
        );
        assert_eq!(game_relative(base, r"C:\Presets\A.ini"), None);
        assert_eq!(game_relative(base, r"..\..\..\..\A.ini"), None);
        assert_eq!(
            relative_value(base, "SB/Binaries/Win64/reshade-presets/A.ini").as_deref(),
            Some(r".\reshade-presets\A.ini")
        );
        assert_eq!(relative_value(base, "SB/Content/A.ini"), None);
    }

    #[test]
    fn presets_name_their_effects() {
        assert!(is_preset(PRESET));
        assert!(is_preset("[SMAA.fx]\nEdgeDetectionType=1\n"));
        assert!(!is_preset(RESHADE_INI));
        assert!(!is_preset(
            "[/Script/Engine.RendererSettings]\nr.Tonemapper.Sharpen=1\n"
        ));

        let preset = read_preset(PRESET).unwrap();
        assert_eq!(preset.techniques.len(), 4);
        assert_eq!(
            preset.effects(),
            ["Clarity.fx", "LumaSharpen.fx", "Vibrance.fx"]
        );
        assert_eq!(preset.unchecked(), 1);
        let available: BTreeSet<String> = ["clarity.fx".to_string(), "vibrance.fx".to_string()]
            .into_iter()
            .collect();
        assert_eq!(preset.missing(&available), ["LumaSharpen.fx"]);
    }

    fn unknown(files: &[&str]) -> DetectedComponent {
        DetectedComponent {
            mod_type: ModType::Unknown,
            confidence: Confidence::AskUser,
            files: files
                .iter()
                .map(|f| ComponentFile {
                    source: PathBuf::from(f),
                    target: PathBuf::from(f),
                })
                .collect(),
            target_subdir: PathBuf::new(),
            ue4ss_mod_name: None,
            pak_sets: Vec::new(),
            warnings: Vec::new(),
            notes: Vec::new(),
        }
    }

    fn read(path: &Path) -> Option<String> {
        let name = path.to_string_lossy().to_lowercase();
        if name.ends_with("settings.ini") {
            Some("[General]\nVolume=3\n".into())
        } else if name.ends_with(".ini") {
            Some(PRESET.into())
        } else {
            None
        }
    }

    #[test]
    fn preset_archives_install_without_asking() {
        let out = claim_presets(
            vec![unknown(&[
                "Cinematic.ini",
                "Vivid.ini",
                "screenshot.jpg",
                "readme.txt",
            ])],
            read,
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].mod_type, ModType::ReshadePreset);
        assert_eq!(out[0].confidence, Confidence::High);
        let targets: Vec<_> = out[0].files.iter().map(|f| f.target.clone()).collect();
        assert_eq!(
            targets,
            [
                Path::new(paths::RESHADE_PRESETS).join("Cinematic.ini"),
                Path::new(paths::RESHADE_PRESETS).join("Vivid.ini"),
            ]
        );
        assert!(out[0].notes[1].contains("screenshot.jpg"));
    }

    #[test]
    fn presets_of_one_name_keep_their_folders() {
        let out = claim_presets(
            vec![unknown(&[
                "Presets/Day/Preset.ini",
                "Presets/Night/Preset.ini",
            ])],
            read,
        );
        let targets: Vec<_> = out[0].files.iter().map(|f| f.target.clone()).collect();
        let root = Path::new(paths::RESHADE_PRESETS);
        assert_eq!(
            targets,
            [root.join("Day/Preset.ini"), root.join("Night/Preset.ini")]
        );
    }

    #[test]
    fn other_unknown_files_are_still_asked_about() {
        let out = claim_presets(
            vec![unknown(&["Preset.ini", "Settings.ini", "hook.dll"])],
            read,
        );
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].mod_type, ModType::Unknown);
        assert_eq!(out[0].files.len(), 2);
        assert_eq!(out[1].mod_type, ModType::ReshadePreset);

        // Nothing that is a preset: untouched.
        let out = claim_presets(vec![unknown(&["Settings.ini"])], read);
        assert_eq!(out, vec![unknown(&["Settings.ini"])]);
    }
}
