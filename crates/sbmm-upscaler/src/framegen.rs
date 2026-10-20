//! DLSS Frame Generation on RTX 20 and 30 cards — experimental.
//!
//! NVIDIA only runs DLSS Frame Generation on RTX 40 and later. The community
//! project [`dlssg_for_sm86`](https://github.com/sdli1995/dlssg_for_sm86)
//! gets it running on Ampere (SM86) and Turing (SM75): a proxy DLL beside the
//! rendering executable intercepts the game's request for `nvngx_dlssg.dll`
//! and answers it with a bundled runtime driven by kernels rewritten for the
//! older cards. Nothing in the game is patched, so like every other swap the
//! manager makes it is a set of files that can be laid down and taken back.
//!
//! This module knows what the release looks like, how to tell that a
//! download arrived intact, and whether this game on this card can use it.
//! The download and the deployment are done by the layers above, the same way
//! as for the DLSS and FSR updates.
//!
//! The package is third-party code that hooks the graphics path, which is why
//! the UI labels it experimental and the checks here err on the side of
//! saying no: a frame generator that cannot load is invisible, and one loaded
//! twice through two proxies is worse than none.

use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::catalog::Release;
use crate::gpu::Adapter;
use crate::{pe, scan, Component, InstalledFile, Version};

/// `owner/repo` on GitHub.
pub const REPO: &str = "sdli1995/dlssg_for_sm86";

/// The project page, for the "what is this" link.
pub const PROJECT_URL: &str = "https://github.com/sdli1995/dlssg_for_sm86";

/// 0.4.0 moved the runtime and kernels out of the proxy into `dlssg_sm86\`
/// and renamed the release asset. Earlier releases have a different layout
/// and are not offered.
const FIRST_SUPPORTED: Version = Version(0, 4, 0, 0);

/// The configuration file the proxy reads, beside itself.
pub const CONFIG_FILE: &str = "dlssg_sm86.ini";

/// The runtime, backend and kernel packs. The proxy finds them relative to its
/// own folder, so the whole directory has to travel with it.
pub const DATA_DIR: &str = "dlssg_sm86";

/// The per-file size and SHA-256 list shipped inside the release archive.
const MANIFEST: &str = "manifest.json";

/// Where the release keeps the proxies other than `version.dll`.
const ALTERNATIVES: &str = "alternatives";

/// Streamline's DLSS-G plugin. A game that has it asks for `nvngx_dlssg.dll`
/// through Streamline, which is the request the proxy answers.
const STREAMLINE_FRAME_GENERATION: &str = "sl.dlss_g.dll";

/// Files that must be in the package for the proxy to do anything at all.
/// Copying only the DLL is the most common broken install upstream reports.
const REQUIRED: &[&str] = &[
    CONFIG_FILE,
    "dlssg_sm86/backend/sm86_backend.dll",
    "dlssg_sm86/kernels/fg_kernels.dll",
    "dlssg_sm86/runtime/nvngx_dlssg.dll",
];

/// Where to ask GitHub for the tag list.
pub fn tags_url() -> String {
    format!("https://api.github.com/repos/{REPO}/tags?per_page=100")
}

/// The name of the complete release archive for one tag.
pub fn asset_name(tag: &str) -> String {
    format!("dlssg-release-{tag}-x64.zip")
}

/// Where the complete release archive for one tag is published.
///
/// The complete archive rather than the repository tree, because the
/// super-resolution kernels are over GitHub's per-file limit and only ship
/// as a release asset.
pub fn download_url(tag: &str) -> String {
    format!(
        "https://github.com/{REPO}/releases/download/{tag}/{}",
        asset_name(tag)
    )
}

/// The newest release this module knows how to install.
pub fn newest<'a>(tags: impl IntoIterator<Item = &'a str>) -> Option<Release> {
    tags.into_iter()
        .filter_map(|tag| {
            let version = Version::parse(tag)?;
            (version >= FIRST_SUPPORTED).then(|| Release {
                tag: tag.to_string(),
                version,
            })
        })
        .max_by_key(|release| release.version)
}

/// The name the proxy DLL is installed under.
///
/// All six are the same loader; they differ only in which system DLL they
/// pretend to be, and so in whether the game loads them. Exactly one may be
/// in place at a time.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Proxy {
    /// The first choice upstream, loaded by almost every game.
    #[default]
    Version,
    Winmm,
    Dbghelp,
    Dinput8,
    /// Render-path proxies: last resort, and never both.
    Dxgi,
    D3d12,
}

impl Proxy {
    /// Upstream's order of preference, which is also the order to try them in.
    pub fn all() -> &'static [Proxy] {
        &[
            Proxy::Version,
            Proxy::Winmm,
            Proxy::Dbghelp,
            Proxy::Dinput8,
            Proxy::Dxgi,
            Proxy::D3d12,
        ]
    }

    pub fn file_name(self) -> &'static str {
        match self {
            Proxy::Version => "version.dll",
            Proxy::Winmm => "winmm.dll",
            Proxy::Dbghelp => "dbghelp.dll",
            Proxy::Dinput8 => "dinput8.dll",
            Proxy::Dxgi => "dxgi.dll",
            Proxy::D3d12 => "d3d12.dll",
        }
    }

    pub fn from_file_name(name: &str) -> Option<Proxy> {
        Proxy::all()
            .iter()
            .copied()
            .find(|p| p.file_name().eq_ignore_ascii_case(name))
    }

    /// Where this proxy sits inside the release archive.
    pub fn package_path(self) -> PathBuf {
        match self {
            Proxy::Version => PathBuf::from(self.file_name()),
            _ => Path::new(ALTERNATIVES).join(self.file_name()),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PackageError {
    #[error("i/o error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("the download is missing {0}; it is not a complete dlssg_for_sm86 release")]
    Missing(PathBuf),
    #[error("{0} is not a Windows DLL")]
    NotADll(PathBuf),
    #[error("{path} does not match the release manifest (expected {expected}, got {actual})")]
    Corrupt {
        path: PathBuf,
        expected: String,
        actual: String,
    },
}

/// An unpacked release, checked and ready to lay down.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    pub root: PathBuf,
    /// The release version the archive says it is, when it says.
    pub version: Option<String>,
    /// The DLSS-G runtime it bundles, which is what the game ends up running.
    pub runtime_version: Option<String>,
    /// Whether every file that will be deployed matched the archive's own
    /// hash list. False only when the archive carried no list at all.
    pub verified: bool,
}

/// Check an unpacked release before any of it goes near the game.
///
/// The archive carries its own size and SHA-256 list. Every file that is
/// going to be deployed is checked against it, so a truncated download or a
/// file swapped after the fact is refused rather than installed into the
/// graphics path.
pub fn inspect(root: &Path) -> Result<Package, PackageError> {
    for required in REQUIRED
        .iter()
        .map(PathBuf::from)
        .chain(Proxy::all().iter().map(|p| p.package_path()))
    {
        if !root.join(&required).is_file() {
            return Err(PackageError::Missing(required));
        }
    }
    for proxy in Proxy::all() {
        let path = root.join(proxy.package_path());
        if pe::file_version_opt(&path).is_none() && !is_pe(&path) {
            return Err(PackageError::NotADll(proxy.package_path()));
        }
    }

    let manifest_path = root.join(MANIFEST);
    let Ok(text) = std::fs::read_to_string(&manifest_path) else {
        return Ok(Package {
            root: root.to_path_buf(),
            version: None,
            runtime_version: None,
            verified: false,
        });
    };
    let manifest: serde_json::Value =
        serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);

    let deployed: Vec<PathBuf> = Proxy::all()
        .iter()
        .map(|p| p.package_path())
        .chain(payload(root))
        .collect();

    let listed = manifest
        .get("files")
        .and_then(|f| f.as_array())
        .cloned()
        .unwrap_or_default();
    let mut verified = !listed.is_empty();

    for entry in &listed {
        let Some(path) = entry.get("path").and_then(|p| p.as_str()) else {
            continue;
        };
        let relative = PathBuf::from(path);
        if !deployed.contains(&relative) {
            // Documentation and the like never reach the game, so they are
            // not worth refusing an install over.
            continue;
        }
        let Some(expected) = entry.get("sha256").and_then(|h| h.as_str()) else {
            verified = false;
            continue;
        };
        let full = root.join(&relative);
        if !full.is_file() {
            return Err(PackageError::Missing(relative));
        }
        let actual = sha256_file(&full)?;
        if !actual.eq_ignore_ascii_case(expected) {
            return Err(PackageError::Corrupt {
                path: relative,
                expected: expected.to_ascii_lowercase(),
                actual,
            });
        }
    }

    let bundle = manifest.get("bundle");
    let text_of = |key: &str| {
        bundle
            .and_then(|b| b.get(key))
            .and_then(|v| v.as_str())
            .map(str::to_string)
    };
    Ok(Package {
        root: root.to_path_buf(),
        version: text_of("version"),
        runtime_version: text_of("runtime_version"),
        verified,
    })
}

/// What to lay down for `proxy`: pairs of a path inside the package and the
/// path it goes to, relative to the folder holding the game executable.
///
/// The proxy, its INI and the whole `dlssg_sm86\` directory — never the
/// other five proxies, because two loaded at once is the one thing upstream
/// says must not happen.
pub fn files_to_deploy(package: &Package, proxy: Proxy) -> Vec<(PathBuf, PathBuf)> {
    let mut files = vec![(proxy.package_path(), PathBuf::from(proxy.file_name()))];
    files.extend(payload(&package.root).into_iter().map(|p| (p.clone(), p)));
    files
}

/// Everything under `dlssg_sm86\`, plus the INI, relative to the package.
fn payload(root: &Path) -> Vec<PathBuf> {
    let mut out = vec![PathBuf::from(CONFIG_FILE)];
    collect(root, &root.join(DATA_DIR), &mut out);
    out.sort();
    out
}

fn collect(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => collect(root, &path, out),
            Ok(kind) if kind.is_file() => {
                if let Ok(relative) = path.strip_prefix(root) {
                    out.push(relative.to_path_buf());
                }
            }
            _ => {}
        }
    }
}

fn sha256_file(path: &Path) -> Result<String, PackageError> {
    let io = |e| PackageError::Io {
        path: path.to_path_buf(),
        source: e,
    };
    let mut file = std::fs::File::open(path).map_err(io)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 16];
    loop {
        let read = file.read(&mut buffer).map_err(io)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// A DLL without a version resource is still a DLL; only the `MZ` header is
/// checked, which is enough to reject an HTML error page saved as `.dll`.
fn is_pe(path: &Path) -> bool {
    let mut magic = [0u8; 2];
    std::fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut magic))
        .is_ok()
        && magic == *b"MZ"
}

// -- compatibility ----------------------------------------------------------

/// Who put a file beside the game executable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "name")]
pub enum Owner {
    /// This feature, from an earlier install.
    Ours,
    /// Another mod the manager installed.
    Mod(String),
    /// Nothing the manager knows about: the game itself, or a hand install.
    Unmanaged,
}

/// A proxy-named DLL already beside the executable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyHolder {
    pub proxy: Proxy,
    pub owner: Owner,
}

/// What the game folder looks like, as far as this feature cares.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GameFacts {
    /// The shipping executable, when one was found.
    pub executable: Option<String>,
    /// Every `nvngx_dlssg.dll` the game ships.
    pub frame_generation: Vec<InstalledFile>,
    /// Every Streamline DLSS-G plugin, with its version when readable.
    pub streamline: Vec<(PathBuf, Option<Version>)>,
    /// Proxy-named DLLs already beside the executable.
    pub proxies: Vec<ProxyHolder>,
    /// A `dlssg_sm86.ini` or `dlssg_sm86\` the manager did not put there,
    /// which is what a hand install leaves behind.
    pub hand_installed: bool,
}

/// Read the facts off disk.
///
/// `owner_of` is asked about each file beside the executable, with a path
/// relative to the game root; the deployment record lives above this crate.
pub fn gather(
    game_root: &Path,
    binaries: &Path,
    executable: Option<String>,
    owner_of: impl Fn(&Path) -> Owner,
) -> GameFacts {
    let frame_generation = scan::scan(game_root)
        .into_iter()
        .filter(|f| f.component == Component::DlssFrameGeneration)
        .collect();
    let streamline = scan::find_named(game_root, &[STREAMLINE_FRAME_GENERATION])
        .into_iter()
        .map(|relative| {
            let version = pe::file_version_opt(&game_root.join(&relative));
            (relative, version)
        })
        .collect();

    let mut proxies = Vec::new();
    let mut hand_installed = false;
    if let Ok(entries) = std::fs::read_dir(game_root.join(binaries)) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let relative = binaries.join(&name);
            if let Some(proxy) = Proxy::from_file_name(&name) {
                proxies.push(ProxyHolder {
                    proxy,
                    owner: owner_of(&relative),
                });
            } else if name.eq_ignore_ascii_case(CONFIG_FILE) {
                hand_installed |= owner_of(&relative) == Owner::Unmanaged;
            }
        }
    }
    proxies.sort_by_key(|p| p.proxy as u8);

    GameFacts {
        executable,
        frame_generation,
        streamline,
        proxies,
        hand_installed,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CheckState {
    Pass,
    Warn,
    Fail,
    Info,
}

/// One line of the compatibility report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    /// Stable identifier, for tests and the UI's icons.
    pub id: String,
    pub state: CheckState,
    pub title: String,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Verdict {
    Compatible,
    /// Will install, but something is worth reading first.
    CompatibleWithWarnings,
    /// The card already runs DLSS Frame Generation itself.
    NotNeeded,
    Incompatible,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Assessment {
    pub verdict: Verdict,
    pub checks: Vec<Check>,
}

impl Assessment {
    pub fn installable(&self) -> bool {
        matches!(
            self.verdict,
            Verdict::Compatible | Verdict::CompatibleWithWarnings
        )
    }

    /// The first thing standing in the way, in words, for an error message.
    pub fn blocker(&self) -> Option<String> {
        if self.verdict == Verdict::NotNeeded {
            return self
                .checks
                .iter()
                .find(|c| c.id == "gpu")
                .map(|c| c.detail.clone());
        }
        self.checks
            .iter()
            .find(|c| c.state == CheckState::Fail)
            .map(|c| c.detail.clone())
    }
}

fn check(
    id: &str,
    state: CheckState,
    title: impl Into<String>,
    detail: impl Into<String>,
) -> Check {
    Check {
        id: id.to_string(),
        state,
        title: title.into(),
        detail: detail.into(),
    }
}

/// Whether this card and this game can use the package with `proxy`.
pub fn assess(adapter: Option<&Adapter>, facts: &GameFacts, proxy: Proxy) -> Assessment {
    let mut checks = Vec::new();
    let mut not_needed = false;

    // The card. Turing and Ampere are what the kernels are written for; Ada
    // and later already run the real thing, and anything else has no tensor
    // cores to run either.
    checks.push(match adapter {
        None => check(
            "gpu",
            CheckState::Warn,
            "Graphics card not identified",
            "The card could not be read, so whether it is an RTX 20 or 30 is unknown. \
             The package only does anything on those two series.",
        ),
        Some(card) => match card.rtx_generation() {
            Some(20) | Some(30) => check(
                "gpu",
                CheckState::Pass,
                "Graphics card",
                format!(
                    "{} is an RTX {} series card, which is what this package is built for.",
                    card.name,
                    card.rtx_generation().unwrap_or_default()
                ),
            ),
            Some(series) => {
                not_needed = true;
                check(
                    "gpu",
                    CheckState::Info,
                    "Graphics card",
                    format!(
                        "{} is an RTX {series} series card and runs DLSS Frame Generation \
                         natively; this package would only replace NVIDIA's own path.",
                        card.name
                    ),
                )
            }
            None => check(
                "gpu",
                CheckState::Fail,
                "Graphics card",
                format!(
                    "{} is not an NVIDIA RTX 20 or 30 series card; the package cannot run on it.",
                    card.name
                ),
            ),
        },
    });

    // The game. Stellar Blade renders with Direct3D 12 only, which is the one
    // API the package supports; what has to be found is where its executable
    // is, because the proxy has to sit beside it.
    checks.push(match &facts.executable {
        Some(exe) => check(
            "game",
            CheckState::Pass,
            "Direct3D 12 executable",
            format!("The proxy goes beside {exe} in SB/Binaries/Win64."),
        ),
        None => check(
            "game",
            CheckState::Fail,
            "Direct3D 12 executable",
            "No *-Win64-Shipping.exe was found in SB/Binaries/Win64, so there is nowhere \
             the proxy would be loaded from.",
        ),
    });

    // The game has to ask for DLSS-G in the first place: the proxy answers
    // that request, it does not add one.
    let dlssg_versions = versions(facts.frame_generation.iter().map(|f| f.version));
    let streamline_versions = versions(facts.streamline.iter().map(|(_, v)| *v));
    checks.push(
        match (
            facts.frame_generation.is_empty(),
            facts.streamline.is_empty(),
        ) {
            (true, true) => check(
                "dlssg",
                CheckState::Fail,
                "DLSS Frame Generation in the game",
                "Neither nvngx_dlssg.dll nor Streamline's sl.dlss_g.dll is in the game folder, \
                 so the game never asks for frame generation and there is nothing to take over. \
                 Launch the game once, or check that the install is complete.",
            ),
            (_, false) => check(
                "dlssg",
                CheckState::Pass,
                "DLSS Frame Generation in the game",
                format!(
                    "The game requests frame generation through Streamline (sl.dlss_g.dll {}){}. \
                     Its own runtime is set aside while the proxy is loaded, so its version \
                     does not matter.",
                    streamline_versions,
                    if facts.frame_generation.is_empty() {
                        String::new()
                    } else {
                        format!(", shipping nvngx_dlssg.dll {dlssg_versions}")
                    }
                ),
            ),
            (false, true) => check(
                "dlssg",
                CheckState::Warn,
                "DLSS Frame Generation in the game",
                format!(
                    "The game ships nvngx_dlssg.dll {dlssg_versions} but no Streamline plugin \
                     was found. The package is tested on Streamline games; it may still work."
                ),
            ),
        },
    );

    // The proxy name. Two mods owning one file name cannot both be deployed,
    // and a stranger's file there stops loading while ours is in place.
    let holder = facts.proxies.iter().find(|h| h.proxy == proxy);
    checks.push(match holder.map(|h| &h.owner) {
        None | Some(Owner::Ours) => check(
            "proxy",
            CheckState::Pass,
            "Proxy DLL name",
            format!(
                "Nothing else is using {} beside the executable.",
                proxy.file_name()
            ),
        ),
        Some(Owner::Mod(name)) => check(
            "proxy",
            CheckState::Fail,
            "Proxy DLL name",
            format!(
                "{} is already deployed by the mod “{name}”. Disable that mod, or choose \
                 another proxy name.",
                proxy.file_name()
            ),
        ),
        Some(Owner::Unmanaged) => check(
            "proxy",
            CheckState::Warn,
            "Proxy DLL name",
            format!(
                "A {} the manager did not install is beside the executable. It will be set \
                 aside while this is installed and put back on removal, but it will not load \
                 in the meantime.",
                proxy.file_name()
            ),
        ),
    });

    // Anything else proxy-named that the manager does not own may be an
    // earlier hand install, and two loaders at once is upstream's one hard
    // rule. A managed mod (ReShade as dxgi.dll, say) is a known quantity.
    let strangers: Vec<&str> = facts
        .proxies
        .iter()
        .filter(|h| h.proxy != proxy && h.owner == Owner::Unmanaged)
        .map(|h| h.proxy.file_name())
        .collect();
    if facts.hand_installed {
        checks.push(check(
            "handInstall",
            CheckState::Warn,
            "Earlier manual install",
            format!(
                "A {CONFIG_FILE} the manager did not put there is beside the executable, which \
                 is what a manual install of dlssg_for_sm86 leaves. Delete its proxy DLL, \
                 {CONFIG_FILE} and the {DATA_DIR} folder first, or the two copies will fight."
            ),
        ));
    } else if !strangers.is_empty() {
        checks.push(check(
            "otherProxies",
            CheckState::Info,
            "Other proxy DLLs",
            format!(
                "{} also sit beside the executable. That is normal for tools like ReShade; if \
                 one is left over from an older dlssg_for_sm86, remove it — only one may load.",
                strangers.join(", ")
            ),
        ));
    }

    let verdict = if checks.iter().any(|c| c.state == CheckState::Fail) {
        Verdict::Incompatible
    } else if not_needed {
        Verdict::NotNeeded
    } else if checks.iter().any(|c| c.state == CheckState::Warn) {
        Verdict::CompatibleWithWarnings
    } else {
        Verdict::Compatible
    };
    Assessment { verdict, checks }
}

fn versions(found: impl Iterator<Item = Option<Version>>) -> String {
    let mut list: Vec<String> = found
        .map(|v| v.map(|v| v.to_string()).unwrap_or_else(|| "unknown".into()))
        .collect();
    list.dedup();
    if list.is_empty() {
        "unknown".into()
    } else {
        list.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu::Vendor;
    use std::fs;

    fn card(name: &str) -> Adapter {
        Adapter::new(name, Vendor::Nvidia, 8 << 30)
    }

    fn streamline_game() -> GameFacts {
        GameFacts {
            executable: Some("SB-Win64-Shipping.exe".into()),
            frame_generation: vec![InstalledFile {
                component: Component::DlssFrameGeneration,
                path: "SB/Binaries/Win64/nvngx_dlssg.dll".into(),
                version: Some(Version(310, 2, 1, 0)),
            }],
            streamline: vec![(
                "SB/Binaries/Win64/sl.dlss_g.dll".into(),
                Some(Version(2, 7, 30, 0)),
            )],
            proxies: Vec::new(),
            hand_installed: false,
        }
    }

    fn state(assessment: &Assessment, id: &str) -> Option<CheckState> {
        assessment
            .checks
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.state)
    }

    #[test]
    fn an_ampere_card_on_a_streamline_game_is_compatible() {
        let report = assess(
            Some(&card("NVIDIA GeForce RTX 3070")),
            &streamline_game(),
            Proxy::Version,
        );
        assert_eq!(report.verdict, Verdict::Compatible);
        assert!(report.installable());
        assert_eq!(state(&report, "dlssg"), Some(CheckState::Pass));
    }

    #[test]
    fn turing_is_supported_too() {
        let report = assess(
            Some(&card("NVIDIA GeForce RTX 2080 Ti")),
            &streamline_game(),
            Proxy::Version,
        );
        assert!(report.installable());
    }

    #[test]
    fn a_card_that_runs_frame_generation_itself_is_told_it_does_not_need_this() {
        let report = assess(
            Some(&card("NVIDIA GeForce RTX 4070")),
            &streamline_game(),
            Proxy::Version,
        );
        assert_eq!(report.verdict, Verdict::NotNeeded);
        assert!(!report.installable());
        assert!(report.blocker().unwrap().contains("natively"));
    }

    #[test]
    fn cards_without_tensor_cores_are_refused() {
        for name in ["NVIDIA GeForce GTX 1080 Ti", "AMD Radeon RX 7800 XT"] {
            let adapter = if name.starts_with("AMD") {
                Adapter::new(name, Vendor::Amd, 16 << 30)
            } else {
                card(name)
            };
            let report = assess(Some(&adapter), &streamline_game(), Proxy::Version);
            assert_eq!(report.verdict, Verdict::Incompatible, "{name}");
        }
    }

    #[test]
    fn an_unknown_card_is_a_warning_not_a_refusal() {
        let report = assess(None, &streamline_game(), Proxy::Version);
        assert_eq!(report.verdict, Verdict::CompatibleWithWarnings);
    }

    #[test]
    fn a_game_with_no_frame_generation_has_nothing_to_take_over() {
        let mut facts = streamline_game();
        facts.frame_generation.clear();
        facts.streamline.clear();
        let report = assess(
            Some(&card("NVIDIA GeForce RTX 3060")),
            &facts,
            Proxy::Version,
        );
        assert_eq!(report.verdict, Verdict::Incompatible);
        assert!(report.blocker().unwrap().contains("sl.dlss_g.dll"));
    }

    #[test]
    fn a_missing_executable_is_a_refusal() {
        let mut facts = streamline_game();
        facts.executable = None;
        let report = assess(
            Some(&card("NVIDIA GeForce RTX 3060")),
            &facts,
            Proxy::Version,
        );
        assert_eq!(state(&report, "game"), Some(CheckState::Fail));
    }

    #[test]
    fn a_proxy_name_another_mod_owns_is_refused_but_a_different_name_is_fine() {
        let mut facts = streamline_game();
        facts.proxies.push(ProxyHolder {
            proxy: Proxy::Dxgi,
            owner: Owner::Mod("ReShade".into()),
        });
        let rtx = card("NVIDIA GeForce RTX 3080");

        let clash = assess(Some(&rtx), &facts, Proxy::Dxgi);
        assert_eq!(clash.verdict, Verdict::Incompatible);
        assert!(clash.blocker().unwrap().contains("ReShade"));

        let fine = assess(Some(&rtx), &facts, Proxy::Version);
        assert_eq!(
            fine.verdict,
            Verdict::Compatible,
            "a managed ReShade on another name is no concern"
        );
    }

    #[test]
    fn a_leftover_manual_install_is_flagged() {
        let mut facts = streamline_game();
        facts.hand_installed = true;
        let report = assess(
            Some(&card("NVIDIA GeForce RTX 3080")),
            &facts,
            Proxy::Version,
        );
        assert_eq!(report.verdict, Verdict::CompatibleWithWarnings);
        assert_eq!(state(&report, "handInstall"), Some(CheckState::Warn));
    }

    #[test]
    fn only_releases_with_the_current_layout_are_offered() {
        let newest = newest(["0.2.4", "0.3.5", "0.4.0", "0.3.10", "nightly"]).unwrap();
        assert_eq!(newest.tag, "0.4.0");
        assert_eq!(super::newest(["0.3.5", "0.1.0"]), None);
    }

    #[test]
    fn the_download_is_the_complete_release_asset() {
        assert_eq!(
            download_url("0.4.0"),
            "https://github.com/sdli1995/dlssg_for_sm86/releases/download/0.4.0/dlssg-release-0.4.0-x64.zip"
        );
    }

    #[test]
    fn proxies_are_named_as_windows_would_match_them() {
        assert_eq!(Proxy::from_file_name("VERSION.DLL"), Some(Proxy::Version));
        assert_eq!(Proxy::from_file_name("dwmapi.dll"), None);
        assert_eq!(Proxy::Version.package_path(), PathBuf::from("version.dll"));
        assert_eq!(
            Proxy::Winmm.package_path(),
            PathBuf::from("alternatives/winmm.dll")
        );
    }

    // -- package ------------------------------------------------------------

    fn write(root: &Path, relative: &str, bytes: &[u8]) {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    fn hex(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    /// A release shaped like the real 0.4.0 archive, with its manifest.
    fn fake_release(root: &Path) {
        let files: Vec<(&str, &[u8])> = vec![
            ("version.dll", b"MZ proxy"),
            ("alternatives/winmm.dll", b"MZ winmm"),
            ("alternatives/dbghelp.dll", b"MZ dbghelp"),
            ("alternatives/dinput8.dll", b"MZ dinput8"),
            ("alternatives/dxgi.dll", b"MZ dxgi"),
            ("alternatives/d3d12.dll", b"MZ d3d12"),
            ("dlssg_sm86.ini", b"[General]\nEnabled=1\n"),
            ("dlssg_sm86/backend/sm86_backend.dll", b"MZ backend"),
            ("dlssg_sm86/kernels/fg_kernels.dll", b"MZ kernels"),
            ("dlssg_sm86/runtime/nvngx_dlssg.dll", b"MZ runtime"),
            ("README.en.md", b"readme"),
        ];
        let mut listed = Vec::new();
        for (path, bytes) in &files {
            write(root, path, bytes);
            listed.push(serde_json::json!({
                "path": path, "size": bytes.len(), "sha256": hex(bytes)
            }));
        }
        let manifest = serde_json::json!({
            "bundle": {"version": "0.4.0", "runtime_version": "310.9.16.0"},
            "files": listed,
        });
        write(root, MANIFEST, manifest.to_string().as_bytes());
    }

    #[test]
    fn a_complete_release_is_verified_against_its_own_manifest() {
        let dir = tempfile::tempdir().unwrap();
        fake_release(dir.path());

        let package = inspect(dir.path()).unwrap();
        assert!(package.verified);
        assert_eq!(package.version.as_deref(), Some("0.4.0"));
        assert_eq!(package.runtime_version.as_deref(), Some("310.9.16.0"));
    }

    #[test]
    fn a_tampered_file_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        fake_release(dir.path());
        write(
            dir.path(),
            "dlssg_sm86/runtime/nvngx_dlssg.dll",
            b"MZ something else",
        );

        match inspect(dir.path()) {
            Err(PackageError::Corrupt { path, .. }) => {
                assert_eq!(path, PathBuf::from("dlssg_sm86/runtime/nvngx_dlssg.dll"))
            }
            other => panic!("expected a corrupt file, got {other:?}"),
        }
    }

    #[test]
    fn a_proxy_without_its_runtime_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        fake_release(dir.path());
        fs::remove_dir_all(dir.path().join("dlssg_sm86/runtime")).unwrap();

        assert!(matches!(inspect(dir.path()), Err(PackageError::Missing(_))));
    }

    #[test]
    fn an_error_page_saved_as_a_dll_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        fake_release(dir.path());
        fs::remove_file(dir.path().join(MANIFEST)).unwrap();
        write(dir.path(), "version.dll", b"<html>Not Found</html>");

        assert!(matches!(inspect(dir.path()), Err(PackageError::NotADll(_))));
    }

    #[test]
    fn exactly_one_proxy_is_deployed_with_the_ini_and_the_data_folder() {
        let dir = tempfile::tempdir().unwrap();
        fake_release(dir.path());
        let package = inspect(dir.path()).unwrap();

        let files = files_to_deploy(&package, Proxy::Winmm);
        let targets: Vec<String> = files
            .iter()
            .map(|(_, t)| t.to_string_lossy().replace('\\', "/"))
            .collect();
        assert_eq!(files[0].0, PathBuf::from("alternatives/winmm.dll"));
        assert!(targets.contains(&"winmm.dll".to_string()));
        assert!(targets.contains(&"dlssg_sm86.ini".to_string()));
        assert!(targets.contains(&"dlssg_sm86/runtime/nvngx_dlssg.dll".to_string()));
        assert!(!targets.contains(&"version.dll".to_string()));
        assert!(!targets.iter().any(|t| t.starts_with("alternatives")));
        assert!(!targets.iter().any(|t| t.ends_with(".md")));
        // The INI appears once even though it is both "payload" and named.
        assert_eq!(targets.iter().filter(|t| *t == "dlssg_sm86.ini").count(), 1);
    }

    #[test]
    fn facts_are_read_off_a_game_folder() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "SB/Binaries/Win64/SB-Win64-Shipping.exe", b"MZ");
        write(root, "SB/Binaries/Win64/sl.dlss_g.dll", b"MZ");
        write(root, "SB/Binaries/Win64/nvngx_dlssg.dll", b"MZ");
        write(root, "SB/Binaries/Win64/dxgi.dll", b"MZ");
        write(root, "SB/Binaries/Win64/dlssg_sm86.ini", b"");

        let facts = gather(
            root,
            Path::new("SB/Binaries/Win64"),
            Some("SB-Win64-Shipping.exe".into()),
            |_| Owner::Unmanaged,
        );
        assert_eq!(facts.streamline.len(), 1);
        assert_eq!(facts.frame_generation.len(), 1);
        assert_eq!(
            facts.proxies,
            vec![ProxyHolder {
                proxy: Proxy::Dxgi,
                owner: Owner::Unmanaged
            }]
        );
        assert!(facts.hand_installed);
    }
}
