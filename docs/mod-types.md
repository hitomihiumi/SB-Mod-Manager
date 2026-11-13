# Where mods are installed

Stellar Blade mods do not all go in one place: paks go in `~mods`, Blueprint
mods in `LogicMods`, Lua mods in their own UE4SS folder plus a line in
`mods.txt`, ReShade next to the executable. Getting this wrong is the usual
reason a mod "doesn't work". Stelle works out the type of every downloaded
archive from its structure and puts each piece in the right place.

Paths are relative to the folder containing `SB/`.

| Type | Marker | Destination |
| --- | --- | --- |
| Pak | `.pak` (+ `.utoc`/`.ucas`) | `SB/Content/Paks/~mods/` |
| Logic mod | a `LogicMods` directory | `SB/Content/Paks/LogicMods/` |
| UE4SS Lua / C++ | `scripts/main.lua` / `dlls/main.dll` | `SB/Binaries/Win64/ue4ss/Mods/<Name>/` + `mods.txt` |
| UE4SS runtime | `UE4SS.dll` | `SB/Binaries/Win64/` |
| Root / ReShade | `ReShade.ini`, a proxy DLL | `SB/Binaries/Win64/` |
| ReShade preset | an INI with `Techniques=` or `[*.fx]` sections | `SB/Binaries/Win64/reshade-presets/` |
| CNS addon | `*.dekcns.json` with its paks, or a pak named `…_CNS_…` | `SB/Content/Paks/~mods/CustomNanosuitSystem/`, names and layout kept |
| Engine tweak | `Engine.ini`, `GameUserSettings.ini`… | merged into `%LOCALAPPDATA%\SB\Saved\Config\WindowsNoEditor` |
| Movie | `.mp4`/`.bk2`, a `Movies` directory | `SB/Content/Movies/` |
| Splash art | `Splash.bmp`, a `Splash` directory | `SB/Content/Splash/` |
| Game root overlay | an explicit `SB/` tree | laid onto the game folder |

One archive can contain several of these; each piece is routed on its own.
When nothing is conclusive, Stelle asks once and remembers the answer.

## Load order

Priority is written into pak file names (`0010_Outfit_P.pak`), because that is
the only ordering the game honours. "Load after" rules are kept as you
reorder, and `.pak`/`.utoc`/`.ucas` sets keep one base name and the `_P`
suffix.

## Where things live on disk

Mods stay unpacked in a library folder, which can live on any drive (only
`sbmm.db` stays in the app data folder). They are hard-linked into the game
when enabled, or shown to it by the virtual file system hook. App data is in
`%APPDATA%/dev.sbmm.manager` (`~/.local/share/dev.sbmm.manager` on Linux).
