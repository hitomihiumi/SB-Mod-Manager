//! Game-relative locations that Stellar Blade mods get installed into.
//!
//! Every path here is relative to the *game root* — the directory that
//! contains `SB/`. Stellar Blade is UE 4.26 with IoStore enabled, so pak mods
//! arrive as `.pak` / `.utoc` / `.ucas` triplets sharing one base name.

/// The engine content directory prefix inside the install.
pub const GAME_SUBDIR: &str = "SB";

/// Loose pak mods. Load order inside this folder is alphanumeric.
pub const PAKS_MODS: &str = "SB/Content/Paks/~mods";

/// UE4SS Blueprint ("logic") mods. These deliberately do *not* live in `~mods`.
pub const PAKS_LOGICMODS: &str = "SB/Content/Paks/LogicMods";

/// Parent directory for UE4SS Lua and C++ mods; each mod gets its own subfolder.
pub const UE4SS_MODS: &str = "SB/Binaries/Win64/ue4ss/Mods";

/// The UE4SS registration file. Each line is `<ModFolderName> : <0|1>`.
pub const UE4SS_MODS_TXT: &str = "SB/Binaries/Win64/ue4ss/Mods/mods.txt";

/// Where the game executable and any injected DLL proxies live.
pub const BINARIES_WIN64: &str = "SB/Binaries/Win64";

/// Bink/mp4 movie replacements.
pub const CONTENT_MOVIES: &str = "SB/Content/Movies";

/// The startup splash image. Unreal looks for `Splash.bmp` here before the
/// engine is up, which is why splash art is loose files rather than a pak.
pub const CONTENT_SPLASH: &str = "SB/Content/Splash";

/// Directories that must never be treated as a redundant archive wrapper,
/// because their name is itself the signal that identifies the mod type.
pub const MEANINGFUL_DIRS: &[&str] = &[
    "sb",
    "logicmods",
    "~mods",
    "mods",
    "ue4ss",
    "scripts",
    "dlls",
    "movies",
    "paks",
    "content",
    "binaries",
    "win64",
    "splash",
];

/// DLL names commonly used as injector proxies by ReShade and similar tools.
pub const PROXY_DLLS: &[&str] = &[
    "dxgi.dll",
    "d3d11.dll",
    "d3d12.dll",
    "d3d9.dll",
    "dinput8.dll",
    "winmm.dll",
    "version.dll",
    "dwmapi.dll",
    "opengl32.dll",
];

/// File extensions that indicate a movie replacement mod.
pub const MOVIE_EXTS: &[&str] = &["mp4", "bk2", "bik", "usm"];

/// The image files Unreal loads as the startup splash. `EdSplash` is the
/// editor's and is harmless to ship, so a mod carrying it is still splash art.
pub const SPLASH_FILES: &[&str] = &["splash.bmp", "edsplash.bmp"];

/// Extensions a splash image can plausibly have. Unreal wants a `.bmp`, but
/// mods are packaged by hand and a stray `.png` beside one is common.
pub const SPLASH_EXTS: &[&str] = &["bmp", "png"];
