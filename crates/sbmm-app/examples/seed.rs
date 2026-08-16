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
            // Paks get a real index naming plausible assets, so the conflict
            // screen has something to read. Everything else is just bulk —
            // which means the two mods with a `.utoc` beside their pak show up
            // as partly unreadable, and that is on purpose: it is what the
            // conflict screen looks like when a container cannot be parsed.
            if rel.ends_with(".pak") {
                fs::write(path, pak_naming(&assets_for(name), *size))?;
            } else {
                fs::write(path, vec![b'x'; *size])?;
            }
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

    // Two mods that genuinely replace the same assets, so the conflict screen
    // has something real to show rather than a mock.
    {
        let shared = [
            "SB/Content/Characters/Eve/Body.uasset",
            "SB/Content/Characters/Eve/Body.uexp",
        ];
        for name in ["Eve Retexture", "Eve Remesh"] {
            let assets = &shared[..];
            let folder = source.join(name);
            fs::create_dir_all(&folder)?;
            fs::write(
                folder.join(format!("{name}_P.pak")),
                pak_naming(assets, 435),
            )?;
            let staged = app.stage_folder(&folder)?;
            ids.push(app.confirm_install(&staged.staging_id, name, None)?);
        }
    }

    let outfits = app.create_group("Outfits", Some("#d946a6"))?;
    let gameplay = app.create_group("Gameplay", Some("#7c7ff5"))?;
    app.assign_group(&ids[0..2], Some(outfits))?;
    app.assign_group(&ids[3..5], Some(gameplay))?;

    // A mix of applied and pending states, so the UI shows both. The last two
    // are the pair that fight over the same assets.
    let contested = [ids[ids.len() - 2], ids[ids.len() - 1]];
    app.set_enabled(&contested, true)?;
    app.set_enabled(&[ids[0], ids[3], ids[4]], true)?;
    app.apply()?;
    app.set_enabled(&[ids[1], ids[7]], true)?;

    println!("seeded {} mods into {data_dir}", ids.len());
    Ok(())
}

/// Assets a seeded mod pretends to replace, chosen so most mods touch
/// different things and only the pair below is meant to clash.
fn assets_for(name: &str) -> Vec<String> {
    let slug: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase();
    vec![
        format!("SB/Content/{slug}/Mesh.uasset"),
        format!("SB/Content/{slug}/Mesh.uexp"),
        // One shared material, so a couple of unrelated mods overlap the way
        // real ones do.
        "SB/Content/Shared/Materials/Skin.uasset".to_string(),
    ]
}

/// A v11 pak whose index names `assets` and nothing else.
///
/// Real enough for the asset reader, which only ever looks at the index — the
/// alternative is a mod with no readable contents, which would make the
/// conflict screen permanently empty.
fn pak_naming(assets: &[impl AsRef<str>], pad_to: usize) -> Vec<u8> {
    fn string(out: &mut Vec<u8>, value: &str) {
        out.extend_from_slice(&(value.len() as u32 + 1).to_le_bytes());
        out.extend_from_slice(value.as_bytes());
        out.push(0);
    }

    let mut directory = Vec::new();
    directory.extend_from_slice(&(assets.len() as u32).to_le_bytes());
    for (index, asset) in assets.iter().enumerate() {
        let asset = asset.as_ref();
        let (dir, file) = asset.rsplit_once('/').unwrap_or(("", asset));
        string(&mut directory, &format!("{dir}/"));
        directory.extend_from_slice(&1u32.to_le_bytes());
        string(&mut directory, file);
        directory.extend_from_slice(&(index as u32).to_le_bytes());
    }

    // Padding stands in for the payload a real pak would carry, so the sizes
    // the mod list shows stay plausible.
    let mut file = vec![0u8; pad_to.min(64 << 20)];
    let directory_offset = file.len() as u64;
    file.extend_from_slice(&directory);

    let index_offset = file.len() as u64;
    let mut index = Vec::new();
    string(&mut index, "../../../");
    index.extend_from_slice(&(assets.len() as u32).to_le_bytes());
    index.extend_from_slice(&0u64.to_le_bytes());
    index.extend_from_slice(&0u32.to_le_bytes());
    index.extend_from_slice(&1u32.to_le_bytes());
    index.extend_from_slice(&directory_offset.to_le_bytes());
    index.extend_from_slice(&(directory.len() as u64).to_le_bytes());
    index.extend_from_slice(&[0u8; 20]);
    let index_size = index.len() as u64;
    file.extend_from_slice(&index);

    file.extend_from_slice(&[0u8; 16]);
    file.push(0);
    file.extend_from_slice(&0x5A6F_12E1u32.to_le_bytes());
    file.extend_from_slice(&11u32.to_le_bytes());
    file.extend_from_slice(&index_offset.to_le_bytes());
    file.extend_from_slice(&index_size.to_le_bytes());
    file.extend_from_slice(&[0u8; 20]);
    file.extend_from_slice(&[0u8; 32 * 5]);
    file
}
