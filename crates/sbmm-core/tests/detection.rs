//! Fixture-driven tests for the detection engine.
//!
//! Each fixture is the file listing of a real-world archive shape.

use sbmm_core::model::{Confidence, ModType};
use sbmm_core::plan::build_plan;
use sbmm_core::{detect_components, DetectedComponent, FileTree};

fn detect(files: &[&str]) -> Vec<DetectedComponent> {
    detect_components(&FileTree::new(files))
}

fn only(files: &[&str]) -> DetectedComponent {
    let mut c = detect(files);
    assert_eq!(c.len(), 1, "expected exactly one component, got {c:?}");
    c.remove(0)
}

fn targets(component: &DetectedComponent) -> Vec<String> {
    let mut t: Vec<String> = component
        .files
        .iter()
        .map(|f| f.target.to_string_lossy().replace('\\', "/"))
        .collect();
    t.sort();
    t
}

#[test]
fn loose_pak_triplet_goes_to_the_mods_folder() {
    let c = only(&["Outfit_P.pak", "Outfit_P.utoc", "Outfit_P.ucas"]);
    assert_eq!(c.mod_type, ModType::GenericPak);
    assert_eq!(c.confidence, Confidence::High);
    assert_eq!(
        targets(&c),
        vec![
            "SB/Content/Paks/~mods/Outfit_P.pak",
            "SB/Content/Paks/~mods/Outfit_P.ucas",
            "SB/Content/Paks/~mods/Outfit_P.utoc",
        ]
    );
    assert_eq!(c.pak_sets.len(), 1);
    assert!(c.pak_sets[0].is_complete());
}

#[test]
fn a_redundant_wrapper_folder_is_ignored() {
    let c = only(&[
        "Awesome Outfit v2.1/Outfit_P.pak",
        "Awesome Outfit v2.1/Outfit_P.utoc",
        "Awesome Outfit v2.1/Outfit_P.ucas",
        "Awesome Outfit v2.1/readme.txt",
    ]);
    assert_eq!(c.mod_type, ModType::GenericPak);
    assert_eq!(c.files.len(), 3, "the readme must not be installed");
}

#[test]
fn logic_mods_never_land_in_the_mods_folder() {
    let c = only(&["LogicMods/BetterCamera_P.pak"]);
    assert_eq!(c.mod_type, ModType::LogicMod);
    assert_eq!(
        targets(&c),
        vec!["SB/Content/Paks/LogicMods/BetterCamera_P.pak"]
    );
}

#[test]
fn a_lua_mod_is_found_by_its_marker_and_keeps_its_folder_name() {
    let c = only(&[
        "CutsceneSkip/scripts/main.lua",
        "CutsceneSkip/scripts/helpers.lua",
    ]);
    assert_eq!(c.mod_type, ModType::Ue4ssLua);
    assert_eq!(c.ue4ss_mod_name.as_deref(), Some("CutsceneSkip"));
    assert_eq!(
        targets(&c),
        vec![
            "SB/Binaries/Win64/ue4ss/Mods/CutsceneSkip/scripts/helpers.lua",
            "SB/Binaries/Win64/ue4ss/Mods/CutsceneSkip/scripts/main.lua",
        ]
    );
}

#[test]
fn a_cpp_mod_is_found_by_its_dll_marker() {
    let c = only(&["TrainerMod/dlls/main.dll"]);
    assert_eq!(c.mod_type, ModType::Ue4ssDll);
    assert_eq!(c.ue4ss_mod_name.as_deref(), Some("TrainerMod"));
}

#[test]
fn the_ue4ss_distribution_is_not_mistaken_for_a_lua_mod() {
    let c = only(&[
        "dwmapi.dll",
        "ue4ss/UE4SS.dll",
        "ue4ss/UE4SS-settings.ini",
        "ue4ss/Mods/mods.txt",
        "ue4ss/Mods/Keybinds/scripts/main.lua",
    ]);
    assert_eq!(c.mod_type, ModType::Ue4ssFramework);
    assert!(targets(&c).contains(&"SB/Binaries/Win64/dwmapi.dll".to_string()));
    assert!(targets(&c).contains(&"SB/Binaries/Win64/ue4ss/UE4SS.dll".to_string()));
}

#[test]
fn reshade_installs_next_to_the_executable_with_its_shaders() {
    let c = only(&[
        "dxgi.dll",
        "ReShade.ini",
        "reshade-shaders/Shaders/Clarity.fx",
    ]);
    assert_eq!(c.mod_type, ModType::RootBinary);
    assert!(
        targets(&c).contains(&"SB/Binaries/Win64/reshade-shaders/Shaders/Clarity.fx".to_string())
    );
}

#[test]
fn movie_replacements_go_to_the_movies_folder() {
    let c = only(&["Movies/SplashScreen.mp4"]);
    assert_eq!(c.mod_type, ModType::Movie);
    assert_eq!(targets(&c), vec!["SB/Content/Movies/SplashScreen.mp4"]);
}

#[test]
fn a_cns_style_archive_is_routed_by_its_explicit_paths() {
    let components = detect(&[
        "SB/Content/Paks/~mods/Nanosuit_P.pak",
        "SB/Content/Paks/LogicMods/NanosuitLogic_P.pak",
        "SB/Binaries/Win64/ue4ss/Mods/CNS/scripts/main.lua",
    ]);
    assert_eq!(components.len(), 3);

    let by_type = |t: ModType| components.iter().find(|c| c.mod_type == t).unwrap();

    assert_eq!(
        targets(by_type(ModType::GenericPak)),
        vec!["SB/Content/Paks/~mods/Nanosuit_P.pak"]
    );
    assert_eq!(
        targets(by_type(ModType::LogicMod)),
        vec!["SB/Content/Paks/LogicMods/NanosuitLogic_P.pak"]
    );
    let lua = by_type(ModType::Ue4ssLua);
    assert_eq!(lua.ue4ss_mod_name.as_deref(), Some("CNS"));
    assert!(components.iter().all(|c| c.confidence == Confidence::High));
}

#[test]
fn a_mixed_archive_yields_one_component_per_payload() {
    let components = detect(&[
        "LogicMods/Toggle_P.pak",
        "Cosmetic_P.pak",
        "Cosmetic_P.utoc",
        "Cosmetic_P.ucas",
    ]);
    assert_eq!(components.len(), 2);

    let logic = components
        .iter()
        .find(|c| c.mod_type == ModType::LogicMod)
        .expect("logic mod component");
    let pak = components
        .iter()
        .find(|c| c.mod_type == ModType::GenericPak)
        .expect("generic pak component");

    assert_eq!(logic.files.len(), 1);
    assert_eq!(pak.files.len(), 3);
    assert_eq!(
        targets(pak),
        vec![
            "SB/Content/Paks/~mods/Cosmetic_P.pak",
            "SB/Content/Paks/~mods/Cosmetic_P.ucas",
            "SB/Content/Paks/~mods/Cosmetic_P.utoc",
        ]
    );
}

#[test]
fn a_pak_without_the_p_suffix_is_flagged_and_renamed_on_deploy() {
    let c = only(&["Outfit.pak"]);
    assert_eq!(c.mod_type, ModType::GenericPak);
    assert!(!c.pak_sets[0].had_p_suffix);
    assert!(
        c.warnings.iter().any(|w| w.contains("_P")),
        "expected a warning about the missing suffix, got {:?}",
        c.warnings
    );

    let plan = build_plan(1, "/staging", &[c], 12, "Outfit");
    assert_eq!(
        plan.files[0].target.to_string_lossy().replace('\\', "/"),
        "SB/Content/Paks/~mods/0012_Outfit_P.pak"
    );
}

#[test]
fn an_incomplete_iostore_pair_is_reported() {
    let c = only(&["Outfit_P.pak", "Outfit_P.utoc"]);
    assert!(!c.pak_sets[0].is_complete());
    assert!(c.warnings.iter().any(|w| w.contains("utoc")));
}

#[test]
fn an_archive_of_only_documentation_asks_the_user() {
    let c = only(&["readme.txt", "preview.png"]);
    assert_eq!(c.mod_type, ModType::Unknown);
    assert_eq!(c.confidence, Confidence::AskUser);
}

#[test]
fn ue4ss_mods_are_registered_in_mods_txt() {
    let components = detect(&["CutsceneSkip/scripts/main.lua"]);
    let plan = build_plan(1, "/staging", &components, 0, "Cutscene Skip");
    assert_eq!(plan.ue4ss_registrations, vec!["CutsceneSkip"]);
}

#[test]
fn a_lua_mod_without_its_own_folder_falls_back_to_the_mod_name() {
    let components = detect(&["scripts/main.lua"]);
    assert_eq!(components[0].mod_type, ModType::Ue4ssLua);
    assert!(components[0].ue4ss_mod_name.is_none());

    let plan = build_plan(1, "/staging", &components, 0, "Cutscene: Skip");
    assert_eq!(plan.ue4ss_registrations, vec!["Cutscene_ Skip"]);
    assert_eq!(
        plan.files[0].target.to_string_lossy().replace('\\', "/"),
        "SB/Binaries/Win64/ue4ss/Mods/Cutscene_ Skip/scripts/main.lua"
    );
}

#[test]
fn load_order_prefixes_apply_only_to_the_mods_folder() {
    let logic = detect(&["LogicMods/Toggle_P.pak"]);
    let plan = build_plan(1, "/staging", &logic, 42, "Toggle");
    assert_eq!(
        plan.files[0].target.to_string_lossy().replace('\\', "/"),
        "SB/Content/Paks/LogicMods/Toggle_P.pak",
        "logic mods are resolved by UE4SS and must not be renamed"
    );
}

#[test]
fn priority_changes_the_deployed_pak_name_consistently() {
    let components = detect(&["Outfit_P.pak", "Outfit_P.utoc", "Outfit_P.ucas"]);
    let plan = build_plan(1, "/staging", &components, 250, "Outfit");

    let mut names: Vec<String> = plan
        .files
        .iter()
        .map(|f| f.target.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            "0250_Outfit_P.pak",
            "0250_Outfit_P.ucas",
            "0250_Outfit_P.utoc"
        ]
    );
}

/// Splash art is loose images, not a pak: Unreal reads the splash off disk
/// before any pak is mounted, so a splash mod packaged as a pak would never be
/// seen. Real collections carry these as their own `stellarblade-splash` type.
#[test]
fn splash_art_is_routed_to_the_splash_folder() {
    let components = detect(&["Splash/Splash.bmp"]);
    assert_eq!(components.len(), 1);
    assert_eq!(components[0].mod_type, ModType::Splash);

    let plan = build_plan(1, "/staging", &components, 10, "Splash Art");
    assert_eq!(
        plan.files[0].target.to_string_lossy().replace('\\', "/"),
        "SB/Content/Splash/Splash.bmp"
    );
}

/// The file Unreal actually loads is enough on its own, wrapper folder or not.
#[test]
fn a_bare_splash_file_is_recognised_without_its_folder() {
    for layout in [
        &["Splash.bmp"][..],
        &["EVE Raven Tachy Splash Art/Splash.bmp"][..],
        &["Splash/EdSplash.bmp"][..],
    ] {
        let components = detect(layout);
        assert_eq!(
            components.first().map(|c| c.mod_type),
            Some(ModType::Splash),
            "failed for {layout:?}"
        );
    }
}

/// The reason splash is claimed before movies: both are loose media, and a
/// splash image swept into the movies folder is a file the engine never reads.
#[test]
fn a_splash_image_beside_a_movie_is_not_swept_into_the_movies_folder() {
    let components = detect(&["Splash/Splash.bmp", "Movies/Startup.mp4"]);

    let splash = components
        .iter()
        .find(|c| c.mod_type == ModType::Splash)
        .expect("the splash image is its own component");
    let movie = components
        .iter()
        .find(|c| c.mod_type == ModType::Movie)
        .expect("the movie is still a movie");

    assert_eq!(splash.files.len(), 1);
    assert_eq!(movie.files.len(), 1);
    assert!(movie.files[0]
        .target
        .to_string_lossy()
        .replace('\\', "/")
        .starts_with("SB/Content/Movies/"));
}

/// A screenshot shipped alongside a mod is not splash art, and claiming it
/// would put a preview image into the game folder.
#[test]
fn a_stray_image_is_not_mistaken_for_splash_art() {
    let components = detect(&["Outfit_P.pak", "preview.png", "screenshot.bmp"]);
    assert!(
        !components.iter().any(|c| c.mod_type == ModType::Splash),
        "only the name Unreal loads, or a Splash folder, means splash art"
    );
}

/// The Stellar Blade build of UE4SS (Chrisr0's fork, 3.1.0-6), exactly as
/// its release archive lays it out: a version-named wrapper folder around
/// the `dwmapi.dll` proxy and the `ue4ss` runtime with its bundled mods.
#[test]
fn the_stellar_blade_ue4ss_release_is_one_framework_install_beside_the_executable() {
    let files = [
        "UE4SS_v3.1.0-6/dwmapi.dll",
        "UE4SS_v3.1.0-6/ue4ss/Default_UVTD_Configs/Config/case_preserving_variants.json",
        "UE4SS_v3.1.0-6/ue4ss/Default_UVTD_Configs/Config/member_rename_map.json",
        "UE4SS_v3.1.0-6/ue4ss/Default_UVTD_Configs/Config/object_items.json",
        "UE4SS_v3.1.0-6/ue4ss/Default_UVTD_Configs/Config/pdbs_to_dump.json",
        "UE4SS_v3.1.0-6/ue4ss/Default_UVTD_Configs/Config/private_variables.json",
        "UE4SS_v3.1.0-6/ue4ss/Default_UVTD_Configs/Config/types_not_to_dump.json",
        "UE4SS_v3.1.0-6/ue4ss/Default_UVTD_Configs/Config/uprefix_to_fprefix.json",
        "UE4SS_v3.1.0-6/ue4ss/Default_UVTD_Configs/Config/valid_udt_names.json",
        "UE4SS_v3.1.0-6/ue4ss/Default_UVTD_Configs/Config/virtual_generator_includes.json",
        "UE4SS_v3.1.0-6/ue4ss/LICENSE",
        "UE4SS_v3.1.0-6/ue4ss/Mods/ActorDumperMod/Scripts/main.lua",
        "UE4SS_v3.1.0-6/ue4ss/Mods/BPML_GenericFunctions/Scripts/main.lua",
        "UE4SS_v3.1.0-6/ue4ss/Mods/BPModLoaderMod/load_order.txt",
        "UE4SS_v3.1.0-6/ue4ss/Mods/BPModLoaderMod/Scripts/main.lua",
        "UE4SS_v3.1.0-6/ue4ss/Mods/CheatManagerEnablerMod/Scripts/main.lua",
        "UE4SS_v3.1.0-6/ue4ss/Mods/ConsoleCommandsMod/Scripts/dump_object.lua",
        "UE4SS_v3.1.0-6/ue4ss/Mods/ConsoleCommandsMod/Scripts/main.lua",
        "UE4SS_v3.1.0-6/ue4ss/Mods/ConsoleCommandsMod/Scripts/set.lua",
        "UE4SS_v3.1.0-6/ue4ss/Mods/ConsoleCommandsMod/Scripts/summon_unloaded_assets.lua",
        "UE4SS_v3.1.0-6/ue4ss/Mods/ConsoleEnablerMod/Scripts/main.lua",
        "UE4SS_v3.1.0-6/ue4ss/Mods/jsbLuaProfilerMod/Scripts/main.lua",
        "UE4SS_v3.1.0-6/ue4ss/Mods/Keybinds/Scripts/main.lua",
        "UE4SS_v3.1.0-6/ue4ss/Mods/LineTraceMod/Scripts/main.lua",
        "UE4SS_v3.1.0-6/ue4ss/Mods/mods.json",
        "UE4SS_v3.1.0-6/ue4ss/Mods/mods.txt",
        "UE4SS_v3.1.0-6/ue4ss/Mods/shared/jsbProfiler/jsbProfi.lua",
        "UE4SS_v3.1.0-6/ue4ss/Mods/shared/Types.lua",
        "UE4SS_v3.1.0-6/ue4ss/Mods/shared/UEHelpers/UEHelpers.lua",
        "UE4SS_v3.1.0-6/ue4ss/Mods/SplitScreenMod/Scripts/main.lua",
        "UE4SS_v3.1.0-6/ue4ss/UE4SS-settings.ini",
        "UE4SS_v3.1.0-6/ue4ss/UE4SS.dll",
        "UE4SS_v3.1.0-6/ue4ss/UE4SS_Signatures/FName_ToString.lua.example",
        "UE4SS_v3.1.0-6/ue4ss/UE4SS_Signatures/FText_Constructor.lua",
        "UE4SS_v3.1.0-6/ue4ss/UE4SS_Signatures/GNatives.lua",
        "UE4SS_v3.1.0-6/ue4ss/UE4SS_Signatures/GUObjectArray.lua",
        "UE4SS_v3.1.0-6/ue4ss/UE4SS_Signatures/GUObjectArray.lua.example",
        "UE4SS_v3.1.0-6/ue4ss/VTableLayout.ini",
    ];
    let c = only(&files);
    assert_eq!(c.mod_type, ModType::Ue4ssFramework);
    let t = targets(&c);
    assert!(t.contains(&"SB/Binaries/Win64/dwmapi.dll".to_string()));
    assert!(t.contains(&"SB/Binaries/Win64/ue4ss/UE4SS.dll".to_string()));
    assert!(t.contains(&"SB/Binaries/Win64/ue4ss/Mods/mods.txt".to_string()));
    assert!(t.contains(&"SB/Binaries/Win64/ue4ss/Mods/BPModLoaderMod/Scripts/main.lua".to_string()));
    assert!(
        t.iter().all(|p| !p.contains("UE4SS_v3.1.0-6")),
        "the version-named wrapper folder is not part of the install"
    );
}

/// The same runtime shipped as an `SB/` tree, the way Nexus uploads of it
/// are packaged: its bundled mods stay part of it rather than becoming mods
/// of their own, which would switch on the ones its `mods.txt` leaves off.
#[test]
fn ue4ss_shipped_as_an_sb_tree_keeps_its_bundled_mods() {
    let c = only(&[
        "SB/Binaries/Win64/dwmapi.dll",
        "SB/Binaries/Win64/ue4ss/UE4SS.dll",
        "SB/Binaries/Win64/ue4ss/UE4SS-settings.ini",
        "SB/Binaries/Win64/ue4ss/Mods/mods.txt",
        "SB/Binaries/Win64/ue4ss/Mods/BPModLoaderMod/Scripts/main.lua",
        "SB/Binaries/Win64/ue4ss/Mods/BPModLoaderMod/load_order.txt",
        "SB/Binaries/Win64/ue4ss/Mods/ActorDumperMod/Scripts/main.lua",
    ]);
    assert_eq!(c.mod_type, ModType::Ue4ssFramework);
    assert_eq!(c.ue4ss_mod_name, None);
    assert_eq!(
        c.files.len(),
        7,
        "the dwmapi proxy goes with the runtime it loads"
    );
}

/// Real pak mods ship all sorts beside the pak; none of it should make the
/// whole archive unknown.
#[test]
fn extras_beside_a_pak_are_set_aside_not_asked_about() {
    let components = detect(&[
        "Suit/Suit_P.pak",
        "Suit/Suit_P.utoc",
        "Suit/Suit_P.ucas",
        "Suit/README",
        "Suit/LICENSE",
        "Suit/source/Suit.psd",
        "Suit/source/Suit.blend",
        "Suit/modinfo.json",
        "Suit/Optional colours.7z",
    ]);
    assert_eq!(components.len(), 1, "{components:?}");
    let pak = &components[0];
    assert_eq!(pak.mod_type, ModType::GenericPak);
    assert_eq!(pak.confidence, Confidence::High);
    let note = pak.notes.join(" ");
    assert!(note.contains("Suit.psd"), "{note}");
    assert!(
        note.contains("Optional colours.7z is an archive of its own"),
        "{note}"
    );
    assert!(
        !note.contains("README"),
        "readmes are junk, not worth naming"
    );
}

/// Something that might be a payload is still asked about.
#[test]
fn a_dll_beside_a_pak_is_still_asked_about() {
    let components = detect(&["Mod_P.pak", "Mod_P.utoc", "Mod_P.ucas", "helper.dll"]);
    assert_eq!(components.len(), 2);
    assert_eq!(components[1].mod_type, ModType::Unknown);
    assert_eq!(components[1].confidence, Confidence::AskUser);
}

/// Unreal settings files are a tweak of their own, never deployed as files.
#[test]
fn engine_ini_is_a_settings_tweak() {
    let c = only(&["Better Shadows/Engine.ini", "Better Shadows/readme.txt"]);
    assert_eq!(c.mod_type, ModType::ConfigTweak);
    assert_eq!(c.confidence, Confidence::High);
    assert_eq!(targets(&c), ["Engine.ini"]);
    let plan = build_plan(1, "/staging", &[c], 10, "Better Shadows");
    assert!(plan.files.is_empty(), "nothing goes into the game folder");

    let mixed = detect(&[
        "Mod_P.pak",
        "Mod_P.utoc",
        "Mod_P.ucas",
        "Config/Scalability.ini",
    ]);
    let types: Vec<ModType> = mixed.iter().map(|c| c.mod_type).collect();
    assert_eq!(types, [ModType::ConfigTweak, ModType::GenericPak]);
}
