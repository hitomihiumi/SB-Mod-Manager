//! End-to-end flow: install a mod, enable it, apply, and confirm the game
//! folder ends up exactly as it started once everything is switched off again.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use sbmm_app::App;
use sbmm_core::model::ModType;

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
        let data = dir.path().join("data");
        let source = dir.path().join("source");

        fs::create_dir_all(game.join("SB/Content/Paks")).unwrap();
        fs::create_dir_all(game.join("SB/Binaries/Win64")).unwrap();
        fs::write(game.join("SB/Binaries/Win64/SB-Win64-Shipping.exe"), "").unwrap();
        fs::write(
            game.join("SB/Content/Paks/pakchunk0-WindowsNoEditor.pak"),
            "vanilla",
        )
        .unwrap();
        fs::create_dir_all(&source).unwrap();

        let mut app = App::new(&data).unwrap();
        app.set_game_root(&game).unwrap();

        Self {
            _dir: dir,
            app,
            game,
            source,
        }
    }

    /// Build a loose mod folder and register it.
    fn install_folder(&mut self, name: &str, files: &[(&str, &str)]) -> i64 {
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

    fn game_snapshot(&self) -> BTreeMap<String, Vec<u8>> {
        snapshot(&self.game)
    }
}

fn snapshot(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let rel = path
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if path.is_dir() {
            out.insert(format!("{rel}/"), Vec::new());
            walk(root, &path, out);
        } else {
            out.insert(rel, fs::read(&path).unwrap());
        }
    }
}

#[test]
fn a_mod_is_installed_disabled_and_changes_nothing_until_applied() {
    let mut fx = Fixture::new();
    let before = fx.game_snapshot();

    fx.install_folder("Cool Outfit", &[("Outfit_P.pak", "payload")]);

    let snap = fx.app.snapshot().unwrap();
    assert_eq!(snap.mods.len(), 1);
    assert!(!snap.mods[0].enabled, "installing must not enable");
    assert!(
        snap.pending.is_empty(),
        "a disabled new mod is not a pending change"
    );
    assert_eq!(fx.game_snapshot(), before);
}

#[test]
fn enabling_and_applying_puts_files_in_the_game_and_disabling_takes_them_back_out() {
    let mut fx = Fixture::new();
    let before = fx.game_snapshot();
    let id = fx.install_folder("Cool Outfit", &[("Outfit_P.pak", "payload")]);

    fx.app.set_enabled(&[id], true).unwrap();
    let snap = fx.app.snapshot().unwrap();
    assert_eq!(snap.pending.len(), 1);

    let report = fx.app.apply().unwrap();
    assert_eq!(report.deployed, vec!["Cool Outfit"]);
    assert!(report.failed.is_empty());

    let deployed = fx.game.join("SB/Content/Paks/~mods/0010_Outfit_P.pak");
    assert!(deployed.exists(), "expected {deployed:?}");
    assert_eq!(fs::read_to_string(&deployed).unwrap(), "payload");
    assert!(fx.app.snapshot().unwrap().pending.is_empty());

    fx.app.set_enabled(&[id], false).unwrap();
    let report = fx.app.apply().unwrap();
    assert_eq!(report.removed, vec!["Cool Outfit"]);

    assert_eq!(
        fx.game_snapshot(),
        before,
        "disabling everything must restore the original game folder"
    );
}

#[test]
fn all_mods_can_be_toggled_in_one_go() {
    let mut fx = Fixture::new();
    let before = fx.game_snapshot();
    for name in ["Alpha", "Beta", "Gamma"] {
        fx.install_folder(name, &[("Thing_P.pak", name)]);
    }

    let all = fx.app.all_mod_ids().unwrap();
    fx.app.set_enabled(&all, true).unwrap();
    let report = fx.app.apply().unwrap();
    assert_eq!(report.deployed.len(), 3);

    let mods_dir = fx.game.join("SB/Content/Paks/~mods");
    assert_eq!(fs::read_dir(&mods_dir).unwrap().count(), 3);

    fx.app.set_enabled(&all, false).unwrap();
    fx.app.apply().unwrap();
    assert_eq!(fx.game_snapshot(), before);
}

#[test]
fn a_group_can_be_toggled_as_a_unit() {
    let mut fx = Fixture::new();
    let a = fx.install_folder("Alpha", &[("A_P.pak", "a")]);
    let b = fx.install_folder("Beta", &[("B_P.pak", "b")]);
    let loose = fx.install_folder("Loose", &[("C_P.pak", "c")]);

    let group = fx.app.create_group("Outfits", Some("#c33")).unwrap();
    fx.app.assign_group(&[a, b], Some(group)).unwrap();

    let in_group = fx.app.mods_in_group(Some(group)).unwrap();
    assert_eq!(in_group.len(), 2);

    fx.app.set_enabled(&in_group, true).unwrap();
    fx.app.apply().unwrap();

    let mods_dir = fx.game.join("SB/Content/Paks/~mods");
    assert_eq!(fs::read_dir(&mods_dir).unwrap().count(), 2);

    let snap = fx.app.snapshot().unwrap();
    let loose_view = snap.mods.iter().find(|m| m.id == loose).unwrap();
    assert!(!loose_view.deployed, "a mod outside the group is untouched");
}

#[test]
fn changing_the_load_order_renames_the_deployed_pak() {
    let mut fx = Fixture::new();
    let a = fx.install_folder("Alpha", &[("A_P.pak", "a")]);
    let b = fx.install_folder("Beta", &[("B_P.pak", "b")]);

    fx.app.set_enabled(&[a, b], true).unwrap();
    fx.app.apply().unwrap();
    assert!(fx.game.join("SB/Content/Paks/~mods/0010_A_P.pak").exists());
    assert!(fx.game.join("SB/Content/Paks/~mods/0020_B_P.pak").exists());

    // Put Beta first.
    fx.app.set_order(&[b, a]).unwrap();
    let snap = fx.app.snapshot().unwrap();
    assert_eq!(snap.pending.len(), 2, "both mods need renaming");

    fx.app.apply().unwrap();
    assert!(fx.game.join("SB/Content/Paks/~mods/0010_B_P.pak").exists());
    assert!(fx.game.join("SB/Content/Paks/~mods/0020_A_P.pak").exists());
    assert!(!fx.game.join("SB/Content/Paks/~mods/0010_A_P.pak").exists());
}

#[test]
fn a_ue4ss_lua_mod_is_registered_and_deregistered_in_mods_txt() {
    let mut fx = Fixture::new();
    let id = fx.install_folder(
        "Cutscene Skip",
        &[("CutsceneSkip/scripts/main.lua", "-- lua")],
    );

    fx.app.set_enabled(&[id], true).unwrap();
    fx.app.apply().unwrap();

    let mods_txt = fx.game.join("SB/Binaries/Win64/ue4ss/Mods/mods.txt");
    assert_eq!(
        fs::read_to_string(&mods_txt).unwrap().trim(),
        "CutsceneSkip : 1"
    );
    assert!(fx
        .game
        .join("SB/Binaries/Win64/ue4ss/Mods/CutsceneSkip/scripts/main.lua")
        .exists());

    fx.app.set_enabled(&[id], false).unwrap();
    fx.app.apply().unwrap();
    assert!(
        !mods_txt.exists(),
        "with no UE4SS mods left, the mods.txt we created must go too"
    );
}

#[test]
fn a_ue4ss_mod_is_switched_off_while_another_stays_on() {
    let mut fx = Fixture::new();
    let a = fx.install_folder("Skip", &[("Skip/scripts/main.lua", "-- a")]);
    let b = fx.install_folder("Zoom", &[("Zoom/scripts/main.lua", "-- b")]);

    fx.app.set_enabled(&[a, b], true).unwrap();
    fx.app.apply().unwrap();

    let mods_txt = fx.game.join("SB/Binaries/Win64/ue4ss/Mods/mods.txt");
    let text = fs::read_to_string(&mods_txt).unwrap();
    assert!(text.contains("Skip : 1") && text.contains("Zoom : 1"));

    fx.app.set_enabled(&[a], false).unwrap();
    fx.app.apply().unwrap();

    let text = fs::read_to_string(&mods_txt).unwrap();
    assert!(text.contains("Skip : 0"), "got {text:?}");
    assert!(text.contains("Zoom : 1"), "the other mod must stay loaded");
}

#[test]
fn a_users_own_mods_txt_survives_and_is_restored() {
    let mut fx = Fixture::new();
    let mods_txt = fx.game.join("SB/Binaries/Win64/ue4ss/Mods/mods.txt");
    fs::create_dir_all(mods_txt.parent().unwrap()).unwrap();
    fs::write(&mods_txt, "HandInstalled : 1\n").unwrap();

    let id = fx.install_folder("Skip", &[("Skip/scripts/main.lua", "-- lua")]);
    fx.app.set_enabled(&[id], true).unwrap();
    fx.app.apply().unwrap();

    let text = fs::read_to_string(&mods_txt).unwrap();
    assert!(
        text.contains("HandInstalled : 1"),
        "their entry must survive"
    );
    assert!(text.contains("Skip : 1"));

    fx.app.set_enabled(&[id], false).unwrap();
    fx.app.apply().unwrap();
    assert_eq!(
        fs::read_to_string(&mods_txt).unwrap(),
        "HandInstalled : 1\n",
        "their original file must come back untouched"
    );
}

#[test]
fn disabling_a_ue4ss_mod_leaves_no_trace_in_the_game_folder() {
    let mut fx = Fixture::new();
    let before = fx.game_snapshot();
    let id = fx.install_folder("Skip", &[("Skip/scripts/main.lua", "-- lua")]);

    fx.app.set_enabled(&[id], true).unwrap();
    fx.app.apply().unwrap();
    fx.app.set_enabled(&[id], false).unwrap();
    fx.app.apply().unwrap();

    assert_eq!(
        fx.game_snapshot(),
        before,
        "mods.txt and the ue4ss folders it lived in must all be gone"
    );
}

#[test]
fn our_own_mods_txt_is_not_mistaken_for_the_users_on_a_later_run() {
    // Regression: ownership used to be inferred from whether a backup existed,
    // so a second session would back up the file we wrote ourselves and then
    // restore it forever instead of removing it.
    let dir = tempfile::tempdir().unwrap();
    let game = dir.path().join("game");
    let data = dir.path().join("data");
    let source = dir.path().join("source");
    fs::create_dir_all(game.join("SB/Content/Paks")).unwrap();
    fs::create_dir_all(game.join("SB/Binaries/Win64")).unwrap();
    fs::write(game.join("SB/Binaries/Win64/SB-Win64-Shipping.exe"), "").unwrap();
    let folder = source.join("Skip/Skip/scripts");
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join("main.lua"), "-- lua").unwrap();

    let before = snapshot(&game);
    let mods_txt = game.join("SB/Binaries/Win64/ue4ss/Mods/mods.txt");

    // First session: install and enable.
    let id = {
        let mut app = App::new(&data).unwrap();
        app.set_game_root(&game).unwrap();
        let staged = app.stage_folder(source.join("Skip")).unwrap();
        let id = app
            .confirm_install(&staged.staging_id, "Skip", None)
            .unwrap();
        app.set_enabled(&[id], true).unwrap();
        app.apply().unwrap();
        assert!(mods_txt.exists());
        id
    };

    // Second session against the same data directory.
    let mut app = App::new(&data).unwrap();
    app.set_enabled(&[id], false).unwrap();
    app.apply().unwrap();

    assert_eq!(
        snapshot(&game),
        before,
        "the file we created in an earlier session must still be removable"
    );
}

#[test]
fn uninstalling_removes_the_files_and_the_staging_copy() {
    let mut fx = Fixture::new();
    let before = fx.game_snapshot();
    let id = fx.install_folder("Cool Outfit", &[("Outfit_P.pak", "payload")]);

    fx.app.set_enabled(&[id], true).unwrap();
    fx.app.apply().unwrap();

    let staging = fx.app.mods_dir().join("Cool Outfit");
    assert!(staging.is_dir());

    fx.app.uninstall(id).unwrap();

    assert!(!staging.exists(), "the staged copy must go too");
    assert!(fx.app.snapshot().unwrap().mods.is_empty());
    assert_eq!(fx.game_snapshot(), before);
}

#[test]
fn a_mod_that_overwrites_a_game_file_restores_it_on_disable() {
    let mut fx = Fixture::new();
    let movie = fx.game.join("SB/Content/Movies/Intro.mp4");
    fs::create_dir_all(movie.parent().unwrap()).unwrap();
    fs::write(&movie, "original").unwrap();

    let before = fx.game_snapshot();
    let id = fx.install_folder("Custom Intro", &[("Movies/Intro.mp4", "modded")]);

    fx.app.set_enabled(&[id], true).unwrap();
    fx.app.apply().unwrap();
    assert_eq!(fs::read_to_string(&movie).unwrap(), "modded");

    fx.app.set_enabled(&[id], false).unwrap();
    fx.app.apply().unwrap();
    assert_eq!(fs::read_to_string(&movie).unwrap(), "original");
    assert_eq!(fx.game_snapshot(), before);
}

#[test]
fn an_unrecognised_archive_asks_and_honours_the_users_choice() {
    let mut fx = Fixture::new();
    let folder = fx.source.join("Mystery");
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join("data.bin"), "???").unwrap();

    let staged = fx.app.stage_folder(&folder).unwrap();
    assert!(staged.needs_confirmation);
    assert_eq!(staged.components[0].mod_type, ModType::Unknown);

    let id = fx
        .app
        .confirm_install(&staged.staging_id, "Mystery", Some(ModType::RootBinary))
        .unwrap();

    fx.app.set_enabled(&[id], true).unwrap();
    fx.app.apply().unwrap();
    assert!(fx.game.join("SB/Binaries/Win64/data.bin").exists());
}

#[test]
fn a_zip_archive_installs_end_to_end() {
    let mut fx = Fixture::new();
    let archive = fx.source.join("Cool Outfit-1234-1-0-1699999999.zip");
    {
        let file = fs::File::create(&archive).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
        zip.start_file("Cool Outfit/Outfit_P.pak", options).unwrap();
        zip.write_all(b"payload").unwrap();
        zip.finish().unwrap();
    }

    let staged = fx.app.stage_archive(&archive).unwrap();
    assert_eq!(
        staged.suggested_name, "Cool Outfit",
        "the Nexus id/version tail should be trimmed from the name"
    );
    assert!(!staged.needs_confirmation);

    let id = fx
        .app
        .confirm_install(&staged.staging_id, &staged.suggested_name, None)
        .unwrap();
    fx.app.set_enabled(&[id], true).unwrap();
    fx.app.apply().unwrap();

    assert!(fx
        .game
        .join("SB/Content/Paks/~mods/0010_Outfit_P.pak")
        .exists());
}

#[test]
fn drift_is_reported_when_a_deployed_file_disappears() {
    let mut fx = Fixture::new();
    let id = fx.install_folder("Cool Outfit", &[("Outfit_P.pak", "payload")]);
    fx.app.set_enabled(&[id], true).unwrap();
    fx.app.apply().unwrap();

    assert!(fx.app.snapshot().unwrap().drift.is_empty());

    fs::remove_file(fx.game.join("SB/Content/Paks/~mods/0010_Outfit_P.pak")).unwrap();
    assert_eq!(fx.app.snapshot().unwrap().drift.len(), 1);
}
