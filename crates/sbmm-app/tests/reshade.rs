//! ReShade presets: installing them, picking the one ReShade starts with,
//! and keeping a preset per profile.

use std::fs;
use std::path::PathBuf;

use sbmm_app::dto::DeploymentMode;
use sbmm_app::App;
use sbmm_core::reshade;

const WIN64: &str = "SB/Binaries/Win64";

const CINEMATIC: &str = "PreprocessorDefinitions=\r\nTechniques=Clarity@Clarity.fx,Bloom@MagicBloom.fx\r\nTechniqueSorting=Clarity@Clarity.fx,Bloom@MagicBloom.fx\r\n\r\n[Clarity.fx]\r\nClarityRadius=3\r\n";
const VIVID: &str = "Techniques=Vibrance@Vibrance.fx\r\n\r\n[Vibrance.fx]\r\nVibrance=0.3\r\n";
const RESHADE_INI: &str = "[GENERAL]\r\nEffectSearchPaths=.\\reshade-shaders\\Shaders\\**\r\nPresetPath=.\\ReShadePreset.ini\r\n\r\n[INPUT]\r\nKeyOverlay=36,0,0,0\r\n";

struct Fixture {
    _dir: tempfile::TempDir,
    app: App,
    game: PathBuf,
    source: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("game");
        let source = dir.path().join("source");
        fs::create_dir_all(game.join("SB/Content/Paks")).unwrap();
        fs::create_dir_all(game.join(WIN64)).unwrap();
        fs::write(game.join(WIN64).join("SB-Win64-Shipping.exe"), "").unwrap();
        let mut app = App::new(dir.path().join("data")).unwrap();
        app.set_game_root(&game).unwrap();
        Self {
            _dir: dir,
            app,
            game,
            source,
        }
    }

    /// ReShade as its installer leaves it: the DLL, its settings and one
    /// shader.
    fn install_reshade_by_hand(&self) {
        let win64 = self.game.join(WIN64);
        fs::write(win64.join("dxgi.dll"), reshade_dll()).unwrap();
        fs::write(win64.join("ReShade.ini"), RESHADE_INI).unwrap();
        fs::create_dir_all(win64.join("reshade-shaders/Shaders")).unwrap();
        fs::write(win64.join("reshade-shaders/Shaders/Clarity.fx"), "").unwrap();
    }

    fn install_folder(&mut self, name: &str, files: &[(&str, &[u8])]) -> i64 {
        let folder = self.source.join(name);
        for (rel, contents) in files {
            let path = folder.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }
        let staged = self.app.stage_folder(&folder).unwrap();
        self.app
            .confirm_install(&staged.staging_id, name, None)
            .unwrap()
    }

    fn enable(&mut self, id: i64) {
        self.app.set_enabled(&[id], true).unwrap();
        self.app.apply().unwrap();
    }

    fn preset_path(&self) -> Option<String> {
        let text = fs::read_to_string(self.game.join(WIN64).join("ReShade.ini")).unwrap();
        reshade::read_config(&text).preset_path
    }
}

/// Enough of a DLL for its version information to name ReShade.
fn reshade_dll() -> Vec<u8> {
    let mut bytes = b"MZ".to_vec();
    bytes.extend(
        "ProductName\0ReShade"
            .encode_utf16()
            .flat_map(u16::to_le_bytes),
    );
    bytes
}

#[test]
fn a_preset_archive_installs_into_its_own_folder_without_asking() {
    let mut fx = Fixture::new();
    let folder = fx.source.join("Cinematic");
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join("Cinematic.ini"), CINEMATIC).unwrap();
    fs::write(folder.join("screenshot.jpg"), "jpg").unwrap();

    let staged = fx.app.stage_folder(&folder).unwrap();
    assert!(!staged.needs_confirmation, "a preset is not a question");
    assert_eq!(
        staged.components[0].mod_type,
        sbmm_core::ModType::ReshadePreset
    );
    let id = fx
        .app
        .confirm_install(&staged.staging_id, "Cinematic", None)
        .unwrap();
    fx.enable(id);
    assert!(fx
        .game
        .join(WIN64)
        .join("reshade-presets/Cinematic.ini")
        .is_file());
}

#[test]
fn presets_are_listed_with_their_owner_and_missing_effects() {
    let mut fx = Fixture::new();
    fx.install_reshade_by_hand();
    let id = fx.install_folder("Cinematic", &[("Cinematic.ini", CINEMATIC.as_bytes())]);
    fx.enable(id);
    // One of the user's own, beside the executable.
    fs::write(fx.game.join(WIN64).join("Mine.ini"), VIVID).unwrap();
    // Not presets: settings of other tools.
    fs::write(fx.game.join(WIN64).join("dlssg.ini"), "[General]\nMode=1\n").unwrap();

    let view = fx.app.reshade_view().unwrap();
    let installed = view.installed.unwrap();
    assert_eq!(installed.dll, "dxgi.dll");
    assert_eq!(installed.provided_by, "Game folder");
    assert_eq!(view.effects, 1);
    assert_eq!(
        view.active.as_deref(),
        Some("SB/Binaries/Win64/ReShadePreset.ini")
    );
    assert!(!view.active_found, "ReShadePreset.ini was never made");

    let names: Vec<_> = view.presets.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Mine", "Cinematic"]);
    let cinematic = &view.presets[1];
    assert_eq!(cinematic.provided_by, "Cinematic");
    assert_eq!(cinematic.origin, "mod");
    assert_eq!(cinematic.folder, "reshade-presets");
    assert_eq!(cinematic.techniques, ["Clarity", "Bloom"]);
    assert_eq!(cinematic.missing_effects, ["MagicBloom.fx"]);
    let source = cinematic.source.as_ref().unwrap();
    assert_eq!(source.path, "Cinematic.ini");
    assert_eq!(view.presets[0].origin, "game");
    assert_eq!(view.presets[0].missing_effects, ["Vibrance.fx"]);
}

#[test]
fn picking_a_preset_changes_one_line_of_reshades_settings() {
    let mut fx = Fixture::new();
    fx.install_reshade_by_hand();
    let id = fx.install_folder("Cinematic", &[("Cinematic.ini", CINEMATIC.as_bytes())]);
    fx.enable(id);

    fx.app
        .set_reshade_preset("SB/Binaries/Win64/reshade-presets/Cinematic.ini")
        .unwrap();
    let text = fs::read_to_string(fx.game.join(WIN64).join("ReShade.ini")).unwrap();
    assert_eq!(
        text,
        RESHADE_INI.replace(r".\ReShadePreset.ini", r".\reshade-presets\Cinematic.ini")
    );
    let view = fx.app.reshade_view().unwrap();
    assert!(view.active_found);
    assert!(
        view.presets
            .iter()
            .find(|p| p.name == "Cinematic")
            .unwrap()
            .active
    );

    assert!(fx
        .app
        .set_reshade_preset("SB/Binaries/Win64/Nope.ini")
        .is_err());
}

#[test]
fn reshade_from_a_mod_has_its_choice_kept_in_the_mod() {
    let mut fx = Fixture::new();
    let reshade = fx.install_folder(
        "ReShade",
        &[
            ("dxgi.dll", &reshade_dll()),
            ("ReShade.ini", RESHADE_INI.as_bytes()),
            ("reshade-shaders/Shaders/Vibrance.fx", b""),
            ("Vivid.ini", VIVID.as_bytes()),
        ],
    );
    fx.enable(reshade);

    let view = fx.app.reshade_view().unwrap();
    assert_eq!(view.installed.unwrap().provided_by, "ReShade");
    assert_eq!(view.config_provided_by.as_deref(), Some("ReShade"));
    let vivid = view.presets.iter().find(|p| p.name == "Vivid").unwrap();
    assert!(vivid.missing_effects.is_empty());

    fx.app.set_reshade_preset(&vivid.path).unwrap();
    assert_eq!(fx.preset_path().as_deref(), Some(r".\Vivid.ini"));
    // Written in the mod, so taking the mod out and back keeps it.
    fx.app.set_enabled(&[reshade], false).unwrap();
    fx.app.apply().unwrap();
    fx.enable(reshade);
    assert_eq!(fx.preset_path().as_deref(), Some(r".\Vivid.ini"));
}

#[test]
fn a_profile_brings_its_own_preset() {
    let mut fx = Fixture::new();
    fx.install_reshade_by_hand();
    let id = fx.install_folder(
        "Looks",
        &[
            ("Cinematic.ini", CINEMATIC.as_bytes()),
            ("Vivid.ini", VIVID.as_bytes()),
        ],
    );
    fx.enable(id);
    let cinematic = "SB/Binaries/Win64/reshade-presets/Cinematic.ini";
    let vivid = "SB/Binaries/Win64/reshade-presets/Vivid.ini";

    let first = fx.app.profiles().unwrap()[0].id;
    fx.app
        .set_profile_reshade_preset(first, Some(cinematic))
        .unwrap();
    assert_eq!(
        fx.preset_path().as_deref(),
        Some(r".\reshade-presets\Cinematic.ini")
    );

    let second = fx.app.create_profile("Photo mode", true).unwrap();
    fx.app
        .set_profile_reshade_preset(second, Some(vivid))
        .unwrap();
    // Not the active profile: nothing changes yet.
    assert_eq!(
        fx.preset_path().as_deref(),
        Some(r".\reshade-presets\Cinematic.ini")
    );

    let report = fx.app.switch_profile(second).unwrap();
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert_eq!(
        fx.preset_path().as_deref(),
        Some(r".\reshade-presets\Vivid.ini")
    );
    assert_eq!(
        fx.app.reshade_view().unwrap().profile_preset.as_deref(),
        Some(vivid)
    );

    // ReShade's own switch in game is put right on the next launch.
    fx.app.set_reshade_preset(cinematic).unwrap();
    assert_eq!(fx.app.apply_profile_reshade_preset().unwrap(), None);
    assert_eq!(
        fx.preset_path().as_deref(),
        Some(r".\reshade-presets\Vivid.ini")
    );

    // A preset gone with its mod is said, not silently skipped.
    fx.app.set_enabled(&[id], false).unwrap();
    fx.app.apply().unwrap();
    let report = fx.app.switch_profile(first).unwrap();
    assert!(
        report.failed.is_empty(),
        "the first profile still has the mod"
    );
    let report = fx.app.switch_profile(second).unwrap();
    assert_eq!(report.failed.len(), 1);
    assert!(report.failed[0].reason.contains("not in the game folder"));
}

#[test]
fn own_presets_can_be_copied_renamed_and_deleted_and_mods_presets_only_copied() {
    let mut fx = Fixture::new();
    fx.install_reshade_by_hand();
    let id = fx.install_folder("Cinematic", &[("Cinematic.ini", CINEMATIC.as_bytes())]);
    fx.enable(id);
    let theirs = "SB/Binaries/Win64/reshade-presets/Cinematic.ini";

    assert!(fx.app.rename_reshade_preset(theirs, "Other").is_err());
    assert!(fx.app.delete_reshade_preset(theirs).is_err());

    let mine = fx
        .app
        .duplicate_reshade_preset(theirs, "My Cinematic")
        .unwrap();
    assert_eq!(mine, "SB/Binaries/Win64/reshade-presets/My Cinematic.ini");
    assert!(fx
        .app
        .duplicate_reshade_preset(theirs, "my cinematic")
        .is_err());
    assert!(fx.app.duplicate_reshade_preset(theirs, "a/b").is_err());

    let profile = fx.app.profiles().unwrap()[0].id;
    fx.app
        .set_profile_reshade_preset(profile, Some(&mine))
        .unwrap();
    let renamed = fx.app.rename_reshade_preset(&mine, "Night").unwrap();
    assert_eq!(renamed, "SB/Binaries/Win64/reshade-presets/Night.ini");
    // Still ReShade's choice, and the profile's.
    assert_eq!(
        fx.preset_path().as_deref(),
        Some(r".\reshade-presets\Night.ini")
    );
    assert_eq!(
        fx.app.reshade_view().unwrap().profile_preset.as_deref(),
        Some(renamed.as_str())
    );

    assert!(
        fx.app.delete_reshade_preset(&renamed).is_err(),
        "ReShade starts with it"
    );
    fx.app.set_reshade_preset(theirs).unwrap();
    fx.app.delete_reshade_preset(&renamed).unwrap();
    assert!(!fx.game.join(&renamed).exists());
    assert_eq!(fx.app.reshade_view().unwrap().profile_preset, None);
}

#[test]
fn a_preset_mod_without_reshade_says_so() {
    let mut fx = Fixture::new();
    let id = fx.install_folder("Cinematic", &[("Cinematic.ini", CINEMATIC.as_bytes())]);
    fx.enable(id);
    let missing = |fx: &Fixture| {
        fx.app.snapshot().unwrap().mods[0]
            .missing_requirements
            .clone()
    };
    assert_eq!(missing(&fx), ["ReShade"]);
    assert!(fx.app.reshade_view().unwrap().installed.is_none());
    assert!(fx
        .app
        .set_reshade_preset("SB/Binaries/Win64/reshade-presets/Cinematic.ini")
        .is_err());

    fx.install_reshade_by_hand();
    assert!(missing(&fx).is_empty());
}

#[test]
fn presets_work_the_same_under_the_virtual_file_system() {
    let mut fx = Fixture::new();
    fx.app.set_deployment_mode(DeploymentMode::Virtual).unwrap();
    fx.install_reshade_by_hand();
    let id = fx.install_folder("Cinematic", &[("Cinematic.ini", CINEMATIC.as_bytes())]);
    fx.enable(id);

    let view = fx.app.reshade_view().unwrap();
    let cinematic = view.presets.iter().find(|p| p.name == "Cinematic").unwrap();
    assert_eq!(cinematic.provided_by, "Cinematic");
    fx.app.set_reshade_preset(&cinematic.path).unwrap();
    assert_eq!(
        fx.preset_path().as_deref(),
        Some(r".\reshade-presets\Cinematic.ini")
    );
}
