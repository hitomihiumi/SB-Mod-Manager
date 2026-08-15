//! Populate a data directory with a fake game and a few mods.
//!
//! Used to exercise the UI without a real Stellar Blade install:
//! `cargo run -p sbmm-app --example seed -- <data-dir> <game-root>`

use std::fs;
use std::path::Path;

use sbmm_app::App;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let data_dir = args.next().expect("usage: seed <data-dir> <game-root>");
    let game_root = args.next().expect("usage: seed <data-dir> <game-root>");
    let game = Path::new(&game_root);

    // A minimal but valid-looking install.
    fs::create_dir_all(game.join("SB/Content/Paks"))?;
    fs::create_dir_all(game.join("SB/Binaries/Win64"))?;
    fs::write(game.join("SB/Binaries/Win64/SB-Win64-Shipping.exe"), "")?;
    fs::write(
        game.join("SB/Content/Paks/pakchunk0-WindowsNoEditor.pak"),
        "vanilla",
    )?;

    let mut app = App::new(&data_dir)?;
    app.set_game_root(game)?;

    // Optional third argument relocates the library first, so the UI can be
    // exercised with mods stored away from the app data directory.
    if let Some(library) = args.next() {
        app.set_library_root(&library)?;
        println!("library root set to {library}");
    }

    let source = Path::new(&data_dir).join("_seed_sources");
    let mods: &[(&str, &[(&str, usize)])] = &[
        (
            "Aurora Suit Recolour",
            &[
                ("Aurora_P.pak", 4_200_000),
                ("Aurora_P.utoc", 12_000),
                ("Aurora_P.ucas", 3_100_000),
            ],
        ),
        (
            "Holiday Outfit Pack",
            &[
                ("Holiday_P.pak", 18_400_000),
                ("Holiday_P.utoc", 26_000),
                ("Holiday_P.ucas", 9_800_000),
            ],
        ),
        ("Neon Blade VFX", &[("NeonBlade_P.pak", 2_100_000)]),
        ("Free Camera", &[("LogicMods/FreeCamera_P.pak", 640_000)]),
        ("Cutscene Skip", &[("CutsceneSkip/scripts/main.lua", 8_400)]),
        (
            "Intro Movie Replacer",
            &[("Movies/Startup.mp4", 31_000_000)],
        ),
        (
            "ReShade Cinematic",
            &[
                ("dxgi.dll", 2_400_000),
                ("ReShade.ini", 3_100),
                ("reshade-shaders/Shaders/Clarity.fx", 14_000),
            ],
        ),
        ("Combat Tweaks", &[("CombatTweaks_P.pak", 890_000)]),
    ];

    let mut ids = Vec::new();
    for (name, files) in mods {
        let folder = source.join(name);
        for (rel, size) in *files {
            let path = folder.join(rel);
            fs::create_dir_all(path.parent().unwrap())?;
            fs::write(path, vec![b'x'; *size])?;
        }
        let staged = app.stage_folder(&folder)?;
        ids.push(app.confirm_install(&staged.staging_id, name, None)?);
    }

    // One mod pretends to have come from Nexus and to be a version behind, so
    // the update badge can be seen without an API key.
    {
        let folder = source.join("Nano Suit");
        fs::create_dir_all(&folder)?;
        fs::write(folder.join("NanoSuit_P.pak"), vec![b'x'; 5_600_000])?;
        let staged = app.stage_folder(&folder)?;
        let id = app.commit_install(
            &staged.staging_id,
            "Nano Suit",
            None,
            sbmm_app::Origin::nexus(9001, 40100).with_version(Some("1.2".into())),
        )?;
        app.record_update_check(id, Some("1.4"))?;
        ids.push(id);
    }

    let outfits = app.create_group("Outfits", Some("#d946a6"))?;
    let gameplay = app.create_group("Gameplay", Some("#7c7ff5"))?;
    app.assign_group(&ids[0..2], Some(outfits))?;
    app.assign_group(&ids[3..5], Some(gameplay))?;

    // A mix of applied and pending states, so the UI shows both.
    app.set_enabled(&[ids[0], ids[3], ids[4]], true)?;
    app.apply()?;
    app.set_enabled(&[ids[1], ids[7]], true)?;

    println!("seeded {} mods into {data_dir}", ids.len());
    Ok(())
}
