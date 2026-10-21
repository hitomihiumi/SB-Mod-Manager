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
    data: PathBuf,
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
            data,
        }
    }

    /// A second connection to the same database, for asserting on rows the
    /// service has no reason to expose.
    fn store(&self) -> sbmm_store::Store {
        sbmm_store::Store::open(self.data.join("sbmm.db")).unwrap()
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

// -- library relocation -----------------------------------------------------

#[test]
fn the_library_can_be_moved_while_mods_are_enabled() {
    let mut fx = Fixture::new();
    let pristine = fx.game_snapshot();
    let id = fx.install_folder("Cool Outfit", &[("Outfit_P.pak", "payload")]);

    fx.app.set_enabled(&[id], true).unwrap();
    fx.app.apply().unwrap();
    let deployed = fx.game.join("SB/Content/Paks/~mods/0010_Outfit_P.pak");
    assert!(deployed.exists());

    let new_root = fx._dir.path().join("elsewhere");
    let report = fx.app.set_library_root(&new_root).unwrap();

    assert!(
        report.redeployed.failed.is_empty(),
        "{:?}",
        report.redeployed
    );
    assert_eq!(report.folders.library, new_root.to_string_lossy());

    // The staged copy followed the library, and the mod is still deployed.
    assert!(new_root.join("mods/Cool Outfit/Outfit_P.pak").exists());
    assert_eq!(fs::read_to_string(&deployed).unwrap(), "payload");

    // And the guarantee still holds from the new location.
    fx.app.set_enabled(&[id], false).unwrap();
    fx.app.apply().unwrap();
    assert_eq!(fx.game_snapshot(), pristine);
}

#[test]
fn moving_the_library_leaves_nothing_at_the_old_location() {
    let mut fx = Fixture::new();
    let old_root = fx.app.library_root();
    let id = fx.install_folder("Cool Outfit", &[("Outfit_P.pak", "payload")]);
    fx.app.set_enabled(&[id], true).unwrap();
    fx.app.apply().unwrap();

    fx.app
        .set_library_root(fx._dir.path().join("elsewhere"))
        .unwrap();

    assert!(
        !old_root.join("mods").exists(),
        "the old mods folder must not be left behind"
    );
}

#[test]
fn the_library_cannot_be_put_inside_the_game_folder() {
    let mut fx = Fixture::new();
    let err = fx
        .app
        .set_library_root(fx.game.join("SB/library"))
        .unwrap_err();
    assert!(
        matches!(err, sbmm_app::AppError::BadLibraryRoot(_)),
        "got {err:?}"
    );
}

#[test]
fn a_folder_that_already_holds_a_library_is_refused() {
    let mut fx = Fixture::new();
    let occupied = fx._dir.path().join("occupied");
    fs::create_dir_all(occupied.join("mods/Something")).unwrap();
    fs::write(occupied.join("mods/Something/a.pak"), b"x").unwrap();

    assert!(fx.app.set_library_root(&occupied).is_err());
}

#[test]
fn a_new_library_location_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let game = dir.path().join("game");
    let data = dir.path().join("data");
    let library = dir.path().join("library");
    fs::create_dir_all(game.join("SB/Content/Paks")).unwrap();
    fs::create_dir_all(game.join("SB/Binaries/Win64")).unwrap();
    fs::write(game.join("SB/Binaries/Win64/SB-Win64-Shipping.exe"), "").unwrap();

    {
        let mut app = App::new(&data).unwrap();
        app.set_game_root(&game).unwrap();
        app.set_library_root(&library).unwrap();
    }

    let app = App::new(&data).unwrap();
    assert_eq!(app.library_root(), library);
    assert_eq!(app.mods_dir(), library.join("mods"));
}

// -- Nexus credentials ------------------------------------------------------

#[test]
fn the_api_key_never_reaches_the_database() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");

    let mut app = App::new(&data)
        .unwrap()
        .with_key_store(Box::new(sbmm_app::credentials::MemoryKeyStore::default()));

    let account = sbmm_app::dto::NexusAccount {
        name: "tester".into(),
        is_premium: true,
        user_id: 7,
    };
    app.set_nexus_account("super-secret-key", Some(&account))
        .unwrap();

    // The account summary is fine to persist; the key is not.
    assert_eq!(app.nexus_account().unwrap().as_ref(), Some(&account));
    assert_eq!(
        app.nexus_api_key().unwrap().as_deref(),
        Some("super-secret-key")
    );

    drop(app);
    let raw = fs::read(data.join("sbmm.db")).unwrap();
    assert!(
        !contains(&raw, b"super-secret-key"),
        "the API key must not be written to the database file"
    );
    assert!(
        contains(&raw, b"tester"),
        "sanity check: the account name is stored, so the scan works"
    );
}

#[test]
fn clearing_the_key_forgets_the_account_too() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = App::new(dir.path().join("data"))
        .unwrap()
        .with_key_store(Box::new(sbmm_app::credentials::MemoryKeyStore::default()));

    let account = sbmm_app::dto::NexusAccount {
        name: "tester".into(),
        is_premium: false,
        user_id: 1,
    };
    app.set_nexus_account("key", Some(&account)).unwrap();
    app.set_nexus_account("", None).unwrap();

    assert_eq!(app.nexus_api_key().unwrap(), None);
    assert_eq!(app.nexus_account().unwrap(), None);
}

#[test]
fn downloads_live_in_the_library_and_move_with_it() {
    let mut fx = Fixture::new();
    assert_eq!(
        fx.app.downloads_dir(),
        fx.app.library_root().join("downloads")
    );

    // Something part-downloaded should survive a relocation.
    fs::create_dir_all(fx.app.downloads_dir()).unwrap();
    fs::write(fx.app.downloads_dir().join("half.zip"), b"partial").unwrap();

    let new_root = fx._dir.path().join("elsewhere");
    fx.app.set_library_root(&new_root).unwrap();

    assert_eq!(
        fs::read_to_string(new_root.join("downloads/half.zip")).unwrap(),
        "partial"
    );
}

/// A download goes through the same pipeline as a dropped archive, but the mod
/// has to remember it came from Nexus — that is what a later update check has
/// to work from.
#[test]
fn a_nexus_download_is_installed_and_keeps_its_origin() {
    let mut fx = Fixture::new();
    let archive = fx.source.join("440-1899.zip");
    {
        let file = fs::File::create(&archive).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
        zip.start_file("Outfit_P.pak", options).unwrap();
        zip.write_all(b"payload").unwrap();
        zip.finish().unwrap();
    }

    let id = fx
        .app
        .install_download(
            &archive,
            "Nice Outfit",
            sbmm_app::Origin::nexus(440, 1899).with_version(Some("1.2".into())),
        )
        .unwrap();

    let snapshot = fx.app.snapshot().unwrap();
    let installed = snapshot.mods.iter().find(|m| m.id == id).unwrap();
    assert_eq!(installed.name, "Nice Outfit");
    assert_eq!(installed.source, "nexus");
    assert_eq!(installed.version.as_deref(), Some("1.2"));

    // And it still deploys like any other pak.
    fx.app.set_enabled(&[id], true).unwrap();
    fx.app.apply().unwrap();
    assert!(fx
        .game
        .join("SB/Content/Paks/~mods/0010_Outfit_P.pak")
        .exists());
}

/// An empty name falls back to the archive's, so a download that arrived
/// before the API could be asked still gets a sensible label.
#[test]
fn a_download_without_a_name_uses_the_archive_name() {
    let mut fx = Fixture::new();
    let archive = fx.source.join("Cutscene Skip-77-1-0-1699999999.zip");
    {
        let file = fs::File::create(&archive).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
        zip.start_file("Skip_P.pak", options).unwrap();
        zip.write_all(b"payload").unwrap();
        zip.finish().unwrap();
    }

    let id = fx
        .app
        .install_download(&archive, "", sbmm_app::Origin::nexus(77, 1))
        .unwrap();

    let snapshot = fx.app.snapshot().unwrap();
    let installed = snapshot.mods.iter().find(|m| m.id == id).unwrap();
    assert_eq!(installed.name, "Cutscene Skip");
}

/// Swapping an upscaler DLL has to be as reversible as any other change: the
/// game's own file is displaced, not destroyed.
#[test]
fn a_newer_upscaler_replaces_the_games_dll_and_gives_it_back_afterwards() {
    let mut fx = Fixture::new();
    let dll = fx.game.join("SB/Binaries/Win64/nvngx_dlss.dll");
    fs::write(&dll, b"the version that shipped with the game").unwrap();
    let before = fx.game_snapshot();

    let downloaded = fx.source.join("nvngx_dlss.dll");
    fs::write(&downloaded, b"a much newer dlss").unwrap();

    let id = fx
        .app
        .install_upscaler(
            sbmm_upscaler::Component::DlssSuperResolution,
            "310.7.0",
            &[(
                downloaded.clone(),
                PathBuf::from("SB/Binaries/Win64/nvngx_dlss.dll"),
            )],
        )
        .unwrap();

    assert_eq!(
        fs::read(&dll).unwrap(),
        b"a much newer dlss",
        "a swap has to take effect immediately; staging it would be a lie"
    );

    fx.app.uninstall(id).unwrap();
    assert_eq!(
        fx.game_snapshot(),
        before,
        "removing the swap has to restore the game's own DLL exactly"
    );
}

/// Updating twice must not leave our own previous DLL behind as the "original".
#[test]
fn updating_an_upscaler_again_still_restores_the_games_file() {
    let mut fx = Fixture::new();
    let dll = fx.game.join("SB/Binaries/Win64/nvngx_dlss.dll");
    fs::write(&dll, b"the version that shipped with the game").unwrap();
    let before = fx.game_snapshot();

    let target = PathBuf::from("SB/Binaries/Win64/nvngx_dlss.dll");
    for (version, contents) in [
        ("310.5.0", &b"dlss 310.5"[..]),
        ("310.7.0", &b"dlss 310.7"[..]),
    ] {
        let downloaded = fx.source.join(format!("nvngx_dlss-{version}.dll"));
        fs::write(&downloaded, contents).unwrap();
        fx.app
            .install_upscaler(
                sbmm_upscaler::Component::DlssSuperResolution,
                version,
                &[(downloaded, target.clone())],
            )
            .unwrap();
        assert_eq!(fs::read(&dll).unwrap(), contents);
    }

    let installed: Vec<_> = fx
        .app
        .snapshot()
        .unwrap()
        .mods
        .into_iter()
        .filter(|m| m.source == "upscaler")
        .collect();
    assert_eq!(
        installed.len(),
        1,
        "the older swap should have been replaced"
    );

    fx.app.uninstall(installed[0].id).unwrap();
    assert_eq!(
        fx.game_snapshot(),
        before,
        "the file restored has to be the game's, not our first replacement"
    );
}

/// A DLL the game does not have cannot be updated, and saying so beats
/// installing one into a folder the loader never looks at.
#[test]
fn an_upscaler_the_game_does_not_ship_is_refused() {
    let mut fx = Fixture::new();
    let result = fx.app.install_upscaler(
        sbmm_upscaler::Component::DlssFrameGeneration,
        "310.7.0",
        &[],
    );
    assert!(result.is_err());
}

/// An unpacked dlssg_for_sm86 release with dummy binaries.
fn fake_framegen_release(root: &Path) -> sbmm_upscaler::framegen::Package {
    for (rel, contents) in [
        ("version.dll", "MZ version proxy"),
        ("alternatives/winmm.dll", "MZ winmm proxy"),
        ("alternatives/dbghelp.dll", "MZ dbghelp proxy"),
        ("alternatives/dinput8.dll", "MZ dinput8 proxy"),
        ("alternatives/dxgi.dll", "MZ dxgi proxy"),
        ("alternatives/d3d12.dll", "MZ d3d12 proxy"),
        ("dlssg_sm86.ini", "[General]\nEnabled=1\n"),
        ("dlssg_sm86/backend/sm86_backend.dll", "MZ backend"),
        ("dlssg_sm86/kernels/fg_kernels.dll", "MZ kernels"),
        ("dlssg_sm86/runtime/nvngx_dlssg.dll", "MZ runtime"),
        ("README.en.md", "readme"),
    ] {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }
    sbmm_upscaler::framegen::inspect(root).unwrap()
}

/// The frame generation package goes beside the executable at once, takes a
/// hand-placed DLL of the same name aside, and gives it back on removal.
#[test]
fn frame_generation_for_older_cards_installs_and_comes_back_out_exactly() {
    use sbmm_upscaler::framegen::{Owner, Proxy};

    let mut fx = Fixture::new();
    let win64 = fx.game.join("SB/Binaries/Win64");
    fs::write(win64.join("version.dll"), "someone else's proxy").unwrap();
    fs::write(win64.join("sl.dlss_g.dll"), "MZ streamline").unwrap();
    let before = fx.game_snapshot();

    let facts = fx.app.framegen_facts().unwrap().unwrap();
    assert_eq!(facts.streamline.len(), 1);
    assert_eq!(facts.proxies[0].owner, Owner::Unmanaged);

    let package = fake_framegen_release(&fx.source.join("release"));
    fx.app
        .install_framegen(&package, Proxy::Version, "0.4.0")
        .unwrap();

    assert_eq!(
        fs::read(win64.join("version.dll")).unwrap(),
        b"MZ version proxy"
    );
    assert!(win64.join("dlssg_sm86.ini").is_file());
    assert!(win64.join("dlssg_sm86/runtime/nvngx_dlssg.dll").is_file());
    assert!(
        !win64.join("winmm.dll").exists(),
        "only one proxy may ever be in place"
    );
    assert!(!win64.join("README.en.md").exists());

    let installed = fx.app.framegen_installed().unwrap().unwrap();
    assert_eq!(installed.proxy, Proxy::Version);
    assert_eq!(installed.version, "0.4.0");
    let facts = fx.app.framegen_facts().unwrap().unwrap();
    assert_eq!(facts.proxies[0].owner, Owner::Ours);
    assert!(!facts.hand_installed, "our own INI is not a manual install");

    assert!(fx.app.remove_framegen().unwrap());
    assert_eq!(fx.game_snapshot(), before);
    assert!(!fx.app.remove_framegen().unwrap());
}

/// Switching the proxy name replaces the install rather than adding a second
/// loader beside the first.
#[test]
fn reinstalling_frame_generation_under_another_proxy_replaces_the_first() {
    use sbmm_upscaler::framegen::Proxy;

    let mut fx = Fixture::new();
    let win64 = fx.game.join("SB/Binaries/Win64");
    let before = fx.game_snapshot();

    let first = fake_framegen_release(&fx.source.join("first"));
    fx.app
        .install_framegen(&first, Proxy::Version, "0.4.0")
        .unwrap();
    let second = fake_framegen_release(&fx.source.join("second"));
    fx.app
        .install_framegen(&second, Proxy::Winmm, "0.4.0")
        .unwrap();

    assert!(!win64.join("version.dll").exists());
    assert_eq!(
        fs::read(win64.join("winmm.dll")).unwrap(),
        b"MZ winmm proxy"
    );
    assert_eq!(
        fx.app.framegen_installed().unwrap().unwrap().proxy,
        Proxy::Winmm
    );

    fx.app.remove_framegen().unwrap();
    assert_eq!(fx.game_snapshot(), before);
}

/// A proxy name another mod already deploys is reported as that mod's.
#[test]
fn a_proxy_name_owned_by_another_mod_is_attributed_to_it() {
    use sbmm_upscaler::framegen::{assess, Owner, Proxy, Verdict};

    let mut fx = Fixture::new();
    let id = fx.install_folder("ReShade", &[("dxgi.dll", "MZ reshade")]);
    fx.app.set_enabled(&[id], true).unwrap();
    fx.app.apply().unwrap();
    fs::write(fx.game.join("SB/Binaries/Win64/sl.dlss_g.dll"), "MZ").unwrap();

    let facts = fx.app.framegen_facts().unwrap().unwrap();
    let dxgi = facts
        .proxies
        .iter()
        .find(|p| p.proxy == Proxy::Dxgi)
        .unwrap();
    assert_eq!(dxgi.owner, Owner::Mod("ReShade".into()));

    let card = sbmm_upscaler::Adapter::new(
        "NVIDIA GeForce RTX 3070",
        sbmm_upscaler::Vendor::Nvidia,
        8 << 30,
    );
    assert_eq!(
        assess(Some(&card), &facts, Proxy::Dxgi).verdict,
        Verdict::Incompatible
    );
    assert_eq!(
        assess(Some(&card), &facts, Proxy::Version).verdict,
        Verdict::Compatible
    );
}

/// What the game would see through the virtual file system: the real folder
/// with every overlay file laid over it, in the same shape as
/// [`Fixture::game_snapshot`].
fn virtual_view(game: &Path, mapping: sbmm_vfs::Mapping) -> BTreeMap<String, Vec<u8>> {
    let vfs = mapping.into_vfs();
    let mut view = snapshot(game);
    for (path, file) in vfs.overlay().files() {
        // The real entry may be spelled differently; the overlay replaces it.
        view.retain(|existing, _| existing.to_lowercase() != path.as_str());
        for dir in path.ancestors().iter().filter(|d| !d.is_root()) {
            let spelled = format!("{}/", dir.display());
            if !view
                .keys()
                .any(|k| k.to_lowercase() == spelled.to_lowercase())
            {
                view.insert(spelled, Vec::new());
            }
        }
        view.insert(path.display().to_string(), fs::read(&file.source).unwrap());
    }
    view
}

/// The point of the virtual file system is that the game cannot tell it from
/// a real deployment. So the view it builds has to be exactly the folder the
/// hard-link backend produces — paks renamed for load order, UE4SS folders
/// and `mods.txt`, a replaced game file, and the winner of a clash — while
/// the real folder is left untouched.
#[test]
fn the_virtual_view_matches_a_real_deployment_exactly() {
    let mut fx = Fixture::new();
    let movie = fx.game.join("SB/Content/Movies/Intro.mp4");
    fs::create_dir_all(movie.parent().unwrap()).unwrap();
    fs::write(&movie, "original").unwrap();
    let mods_txt = fx.game.join("SB/Binaries/Win64/ue4ss/Mods/mods.txt");
    fs::create_dir_all(mods_txt.parent().unwrap()).unwrap();
    fs::write(&mods_txt, "; mine\nKeepMe : 1\n").unwrap();

    let outfit = fx.install_folder("Outfit", &[("Outfit_P.pak", "outfit")]);
    let hair = fx.install_folder("Hair", &[("Hair_P.pak", "hair")]);
    let skip = fx.install_folder(
        "Cutscene Skip",
        &[("CutsceneSkip/scripts/main.lua", "-- lua")],
    );
    let intro_a = fx.install_folder("Intro A", &[("Movies/Intro.mp4", "intro a")]);
    let intro_b = fx.install_folder("Intro B", &[("Movies/Intro.mp4", "intro b")]);
    let _off = fx.install_folder("Switched Off", &[("Off_P.pak", "off")]);
    fx.app
        .set_enabled(&[outfit, hair, skip, intro_a, intro_b], true)
        .unwrap();
    fx.app
        .set_order(&[hair, outfit, intro_a, skip, intro_b])
        .unwrap();

    let untouched = fx.game_snapshot();
    let view = virtual_view(&fx.game, fx.app.vfs_mapping().unwrap());
    assert_eq!(
        fx.game_snapshot(),
        untouched,
        "building the view must not write to the game folder"
    );

    fx.app.apply().unwrap();
    let deployed = fx.game_snapshot();
    assert_eq!(view, deployed);
    assert_eq!(
        fs::read_to_string(&movie).unwrap(),
        "intro b",
        "the clash goes to the mod further down the load order"
    );
}

/// Switching to the virtual file system takes the mods out of the game folder —
/// all but the files the Windows loader needs before the hook exists — and the
/// game still sees exactly what it saw before. Switching back restores the
/// folder as hard links made it.
#[test]
fn switching_to_the_virtual_file_system_and_back_changes_nothing_the_game_sees() {
    use sbmm_app::dto::DeploymentMode;

    let mut fx = Fixture::new();
    let outfit = fx.install_folder("Outfit", &[("Outfit_P.pak", "outfit")]);
    let skip = fx.install_folder(
        "Cutscene Skip",
        &[("CutsceneSkip/scripts/main.lua", "-- lua")],
    );
    let reshade = fx.install_folder("ReShade", &[("dxgi.dll", "MZ reshade")]);
    fx.app.set_enabled(&[outfit, skip, reshade], true).unwrap();
    fx.app.apply().unwrap();
    let linked = fx.game_snapshot();

    fx.app.set_deployment_mode(DeploymentMode::Virtual).unwrap();
    let win64 = fx.game.join("SB/Binaries/Win64");
    assert!(
        win64.join("dxgi.dll").is_file(),
        "a proxy DLL is loaded before any hook exists, so it stays real"
    );
    assert!(!fx.game.join("SB/Content/Paks/~mods").exists());
    assert!(
        !win64.join("ue4ss").exists(),
        "no mods.txt, no Lua mod folder"
    );

    let snapshot = fx.app.snapshot().unwrap();
    assert_eq!(snapshot.deployment_mode, DeploymentMode::Virtual);
    assert!(snapshot.pending.is_empty(), "{:?}", snapshot.pending);
    assert!(snapshot.mods.iter().all(|m| m.deployed));

    let view = virtual_view(&fx.game, fx.app.vfs_mapping().unwrap());
    assert_eq!(view, linked);

    // Changes keep working while virtual: the disabled mod drops out of the
    // view without anything being written to the game.
    fx.app.set_enabled(&[outfit], false).unwrap();
    assert!(fx.app.snapshot().unwrap().pending.is_empty());
    let view = virtual_view(&fx.game, fx.app.vfs_mapping().unwrap());
    assert!(!view.keys().any(|k| k.contains("Outfit")));
    fx.app.set_enabled(&[outfit], true).unwrap();

    fx.app
        .set_deployment_mode(DeploymentMode::Hardlink)
        .unwrap();
    assert_eq!(fx.game_snapshot(), linked);
}

#[test]
fn launching_under_the_virtual_file_system_writes_a_current_mapping() {
    use sbmm_app::dto::DeploymentMode;

    let mut fx = Fixture::new();
    let outfit = fx.install_folder("Outfit", &[("Outfit_P.pak", "outfit")]);
    fx.app.set_deployment_mode(DeploymentMode::Virtual).unwrap();
    fx.app.set_enabled(&[outfit], true).unwrap();

    let options = fx.app.launch_options(PathBuf::from("hook.dll")).unwrap();
    assert_eq!(
        options.executable,
        fx.game.join("SB/Binaries/Win64/SB-Win64-Shipping.exe")
    );
    assert_eq!(options.working_dir, fx.game.join("SB/Binaries/Win64"));
    assert!(
        options.environment.is_empty(),
        "a hand-picked folder is not known to be a Steam install"
    );

    let vfs = sbmm_vfs::Mapping::load(&options.mapping)
        .unwrap()
        .into_vfs();
    let pak = sbmm_vfs::VPath::from_relative("SB/Content/Paks/~mods/0010_Outfit_P.pak").unwrap();
    assert!(vfs.overlay().file(&pak).is_some());
}

/// The Files view shows the merged tree: a mod's file where a mod wins, the
/// game's own where nothing touches it, and the losing mod named as hidden.
#[test]
fn the_virtual_file_tree_names_the_winner_and_what_it_hides() {
    let mut fx = Fixture::new();
    let movie = fx.game.join("SB/Content/Movies/Intro.mp4");
    fs::create_dir_all(movie.parent().unwrap()).unwrap();
    fs::write(&movie, "original").unwrap();

    let a = fx.install_folder("Intro A", &[("Movies/Intro.mp4", "a")]);
    let b = fx.install_folder("Intro B", &[("Movies/Intro.mp4", "b")]);
    fx.app.set_enabled(&[a, b], true).unwrap();
    fx.app.set_order(&[a, b]).unwrap();

    // Navigate down to the Movies folder.
    let root = fx.app.vfs_tree("").unwrap();
    assert!(root.entries.iter().any(|e| e.name == "SB" && e.is_dir));

    let movies = fx.app.vfs_tree("SB/Content/Movies").unwrap();
    let intro = movies
        .entries
        .iter()
        .find(|e| e.name == "Intro.mp4")
        .unwrap();
    assert_eq!(intro.origin, "mod");
    assert_eq!(
        intro.provided_by, "Intro B",
        "the mod lower in the order wins"
    );
    assert_eq!(intro.shadowed_by, vec!["Intro A".to_string()]);
    assert!(intro.source.is_some());

    // A game file nothing replaces is attributed to the game.
    fs::write(fx.game.join("SB/Content/Movies/Logo.mp4"), "logo").unwrap();
    let movies = fx.app.vfs_tree("SB/Content/Movies").unwrap();
    let logo = movies
        .entries
        .iter()
        .find(|e| e.name == "Logo.mp4")
        .unwrap();
    assert_eq!(logo.origin, "game");
    assert!(logo.shadowed_by.is_empty());
}

/// Files the game writes under the virtual file system go to overwrite and are
/// there again next time, without ever touching the game folder.
#[test]
fn files_the_game_creates_go_to_overwrite() {
    use sbmm_vfs::{Access, Redirect, VPath};

    let mut fx = Fixture::new();
    let id = fx.install_folder(
        "Cutscene Skip",
        &[("CutsceneSkip/scripts/main.lua", "-- lua")],
    );
    fx.app.set_enabled(&[id], true).unwrap();

    let path = fx.app.write_vfs_mapping().unwrap();
    let vfs = sbmm_vfs::Mapping::load(&path).unwrap().into_vfs();
    let log = VPath::from_relative("SB/Binaries/Win64/ue4ss/UE4SS.log").unwrap();
    let Redirect::To(real) = vfs.redirect(&log, Access::Create, false) else {
        panic!("a new file has to be redirected");
    };
    assert!(real.starts_with(fx.app.overwrite_dir()));
    fs::create_dir_all(real.parent().unwrap()).unwrap();
    fs::write(&real, "log").unwrap();

    let again = sbmm_vfs::Mapping::load(&fx.app.write_vfs_mapping().unwrap())
        .unwrap()
        .into_vfs();
    assert_eq!(
        again.redirect(&log, Access::Read, false),
        Redirect::To(real)
    );
    assert!(!fx.game.join("SB/Binaries/Win64/ue4ss/UE4SS.log").exists());
}

/// Closing the manager mid-collection must not lose the queue — that is forty
/// entries and a part-downloaded file the user would have to redo by hand.
#[test]
fn the_download_queue_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");

    {
        let mut app = App::new(&data).unwrap();
        let queued = sbmm_nexus::QueueItem::new(440, 1899, "Nice Outfit", "outfit.zip")
            .with_version(Some("1.2".into()))
            .in_collection("essential-sb");
        let done = sbmm_nexus::QueueItem::new(12, 34, "Already In", "in.zip");

        let mut items = vec![queued, done];
        items[0].id = 1;
        items[1].id = 2;
        items[1].state = sbmm_nexus::DownloadState::Done;
        app.save_download_queue(&items).unwrap();
    }

    let app = App::new(&data).unwrap();
    let restored = app.restore_download_queue().unwrap();

    assert_eq!(
        restored.len(),
        1,
        "a finished download is installed already and should not come back"
    );
    assert_eq!(restored[0].id, 1);
    assert_eq!(restored[0].mod_id, 440);
    assert_eq!(restored[0].file_id, 1899);
    assert_eq!(restored[0].name, "Nice Outfit");
    assert_eq!(restored[0].file_name, "outfit.zip");
    assert_eq!(restored[0].version.as_deref(), Some("1.2"));
    assert_eq!(restored[0].collection.as_deref(), Some("essential-sb"));
}

/// The credentials an nxm:// link carries are short-lived, and a credential
/// has no business sitting in a file on disk.
#[test]
fn a_saved_queue_never_holds_the_download_credentials() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let secret = "a-very-secret-download-key";

    let mut app = App::new(&data).unwrap();
    let mut item = sbmm_nexus::QueueItem::new(440, 1899, "Nice Outfit", "outfit.zip")
        .with_credentials(secret, 1_800_000_000);
    item.id = 1;
    app.save_download_queue(&[item]).unwrap();
    drop(app);

    let raw = fs::read(data.join("sbmm.db")).unwrap();
    assert!(
        !contains(&raw, secret.as_bytes()),
        "the download key must not reach the database"
    );

    let restored = App::new(&data).unwrap().restore_download_queue().unwrap();
    assert!(!restored[0].has_credentials());
}

/// Build a v11 pak whose index names `assets`, so a mod can be given real
/// contents to be read rather than a stub.
fn pak_with(assets: &[&str]) -> Vec<u8> {
    fn string(out: &mut Vec<u8>, value: &str) {
        out.extend_from_slice(&(value.len() as u32 + 1).to_le_bytes());
        out.extend_from_slice(value.as_bytes());
        out.push(0);
    }

    // Each asset becomes its own directory entry, which keeps the builder
    // trivial and is a shape UnrealPak itself produces.
    let mut directory = Vec::new();
    directory.extend_from_slice(&(assets.len() as u32).to_le_bytes());
    for (index, asset) in assets.iter().enumerate() {
        let (dir, file) = asset.rsplit_once('/').unwrap_or(("", asset));
        string(&mut directory, &format!("{dir}/"));
        directory.extend_from_slice(&1u32.to_le_bytes());
        string(&mut directory, file);
        directory.extend_from_slice(&(index as u32).to_le_bytes());
    }

    let mut file = vec![0u8; 32];
    let directory_offset = file.len() as u64;
    file.extend_from_slice(&directory);

    let index_offset = file.len() as u64;
    let mut index = Vec::new();
    string(&mut index, "../../../");
    index.extend_from_slice(&(assets.len() as u32).to_le_bytes());
    index.extend_from_slice(&0u64.to_le_bytes());
    index.extend_from_slice(&0u32.to_le_bytes()); // no path hash index
    index.extend_from_slice(&1u32.to_le_bytes()); // a directory index follows
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

impl Fixture {
    /// Install a pak mod whose index names `assets`.
    fn install_pak(&mut self, name: &str, assets: &[&str]) -> i64 {
        let folder = self.source.join(name);
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join(format!("{name}_P.pak")), pak_with(assets)).unwrap();

        let staged = self.app.stage_folder(&folder).unwrap();
        self.app
            .confirm_install(&staged.staging_id, name, None)
            .unwrap()
    }
}

/// Two mods replacing the same asset is the case the whole feature exists for:
/// both look installed, both are enabled, and only one of them is doing
/// anything.
#[test]
fn two_mods_replacing_one_asset_are_reported_with_the_winner_named() {
    let mut fx = Fixture::new();
    let shared = "SB/Content/Characters/Eve/Body.uasset";

    let first = fx.install_pak("Aurora", &[shared, "SB/Content/Only/Aurora.uasset"]);
    let second = fx.install_pak("Nova", &[shared]);
    fx.app.set_enabled(&[first, second], true).unwrap();

    let report = fx.app.conflicts().unwrap();
    assert_eq!(
        report.conflicts.len(),
        1,
        "only the shared asset is contested"
    );

    let conflict = &report.conflicts[0];
    assert_eq!(conflict.asset, "sb/content/characters/eve/body.uasset");
    assert!(conflict.named);
    assert_eq!(conflict.claimants.len(), 2);

    // Nova installed later, so it sits further down the load order and mounts
    // last, which is what the game actually loads.
    let winner = conflict.claimants.iter().find(|c| c.wins).unwrap();
    assert_eq!(winner.mod_id, second);
    assert_eq!(
        conflict.claimants.iter().filter(|c| c.wins).count(),
        1,
        "exactly one mod can win an asset"
    );
}

/// Moving a mod up the load order has to change who wins, or the conflict
/// screen is decoration rather than a tool.
#[test]
fn reordering_hands_the_asset_to_the_other_mod() {
    let mut fx = Fixture::new();
    let shared = "SB/Content/Characters/Eve/Body.uasset";

    let first = fx.install_pak("Aurora", &[shared]);
    let second = fx.install_pak("Nova", &[shared]);
    fx.app.set_enabled(&[first, second], true).unwrap();

    let before = fx.app.conflicts().unwrap();
    assert!(before.conflicts[0]
        .claimants
        .iter()
        .any(|c| c.mod_id == second && c.wins));

    fx.app.set_order(&[second, first]).unwrap();

    let after = fx.app.conflicts().unwrap();
    assert!(
        after.conflicts[0]
            .claimants
            .iter()
            .any(|c| c.mod_id == first && c.wins),
        "the mod moved to the end of the order should now win"
    );
}

/// A disabled mod is not in the game folder, so it cannot be overriding
/// anything and must not be reported as doing so.
#[test]
fn a_disabled_mod_is_not_part_of_a_conflict() {
    let mut fx = Fixture::new();
    let shared = "SB/Content/Characters/Eve/Body.uasset";

    let first = fx.install_pak("Aurora", &[shared]);
    let second = fx.install_pak("Nova", &[shared]);

    fx.app.set_enabled(&[first], true).unwrap();
    assert!(
        fx.app.conflicts().unwrap().conflicts.is_empty(),
        "one enabled mod cannot conflict with a disabled one"
    );

    fx.app.set_enabled(&[second], true).unwrap();
    assert_eq!(fx.app.conflicts().unwrap().conflicts.len(), 1);
}

/// The badge the mod list shows: how much of a mod is actually reaching the
/// game, and how much of it another mod has taken over.
#[test]
fn each_mod_is_counted_for_what_it_wins_and_loses() {
    let mut fx = Fixture::new();
    let a = "SB/Content/A.uasset";
    let b = "SB/Content/B.uasset";

    let first = fx.install_pak("Aurora", &[a, b]);
    let second = fx.install_pak("Nova", &[a, b]);
    fx.app.set_enabled(&[first, second], true).unwrap();

    let report = fx.app.conflicts().unwrap();
    let loser = report
        .overridden
        .iter()
        .find(|c| c.mod_id == first)
        .unwrap();
    let winner = report
        .overridden
        .iter()
        .find(|c| c.mod_id == second)
        .unwrap();

    assert_eq!(loser.losing, 2, "both of Aurora's assets are overridden");
    assert_eq!(loser.winning, 0);
    assert_eq!(winner.winning, 2);
    assert_eq!(winner.losing, 0);
}

/// Mods that touch different things must not be dragged into the list.
#[test]
fn mods_that_replace_different_assets_do_not_conflict() {
    let mut fx = Fixture::new();
    let first = fx.install_pak("Aurora", &["SB/Content/A.uasset"]);
    let second = fx.install_pak("Nova", &["SB/Content/B.uasset"]);
    fx.app.set_enabled(&[first, second], true).unwrap();

    assert!(fx.app.conflicts().unwrap().conflicts.is_empty());
}

/// A mod whose containers could not be read has an unknown asset list, and
/// saying nothing about it would read as "this mod conflicts with nothing".
#[test]
fn a_mod_whose_containers_could_not_be_read_is_named_as_unknown() {
    let mut fx = Fixture::new();
    let folder = fx.source.join("Mystery");
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join("Mystery_P.pak"), b"this is not a pak at all").unwrap();

    let staged = fx.app.stage_folder(&folder).unwrap();
    let id = fx
        .app
        .confirm_install(&staged.staging_id, "Mystery", None)
        .unwrap();
    fx.app.set_enabled(&[id], true).unwrap();

    let report = fx.app.conflicts().unwrap();
    assert_eq!(
        report.unreadable,
        vec!["Mystery"],
        "an unreadable mod is listed rather than treated as empty"
    );
}

/// Mods installed before the index existed have to be caught up, or the
/// feature only works for things installed after the update.
#[test]
fn mods_installed_before_the_index_existed_are_caught_up() {
    let mut fx = Fixture::new();
    let id = fx.install_pak("Aurora", &["SB/Content/A.uasset"]);

    // Undo the indexing the install did, which is the state a database
    // upgraded from before the feature is in.
    {
        let mut store = fx.store();
        store.replace_mod_assets(id, &[], false).unwrap();
    }

    assert_eq!(fx.app.index_missing_assets().unwrap(), 1);
    assert_eq!(fx.store().asset_count(id).unwrap(), 1);
}

/// Naive substring search over the raw database bytes.
fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// The diagnostics bundle holds a summary of the setup and the logs that
/// exist, so a bug report needs one attachment rather than a hunt by path.
#[test]
fn a_diagnostics_bundle_carries_the_setup_and_the_logs() {
    use std::io::Read;

    let mut fx = Fixture::new();
    let outfit = fx.install_folder("Outfit", &[("Outfit_P.pak", "outfit")]);
    fx.app.set_enabled(&[outfit], true).unwrap();
    fx.app.apply().unwrap();

    let vfs = fx.app.vfs_dir();
    fs::create_dir_all(&vfs).unwrap();
    fs::write(vfs.join("launch.log"), "FAILED: hook not loaded\n").unwrap();
    let framegen_logs = fx.game.join("SB/Binaries/Win64/dlssg_sm86/logs");
    fs::create_dir_all(&framegen_logs).unwrap();
    fs::write(framegen_logs.join("loader_1.jsonl"), "{}\n").unwrap();

    let dest = fx.source.join("diagnostics.zip");
    fx.app
        .write_diagnostics(&dest, &[("Manager".into(), "0.2.0".into())])
        .unwrap();

    let mut zip = zip::ZipArchive::new(fs::File::open(&dest).unwrap()).unwrap();
    let mut read = |name: &str| {
        let mut text = String::new();
        zip.by_name(name)
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        text
    };
    let summary = read("summary.txt");
    assert!(summary.contains("Manager: 0.2.0"));
    assert!(summary.contains("Deployment: hard links"));
    assert!(summary.contains("[x]") && summary.contains("Outfit"));
    assert_eq!(read("vfs/launch.log"), "FAILED: hook not loaded\n");
    assert_eq!(read("framegen/loader_1.jsonl"), "{}\n");
    assert!(
        zip.by_name("vfs/hook.log").is_err(),
        "a log that was never written is left out, not faked"
    );
}

/// Switching profile reconciles the game folder with the other profile's
/// enabled set and load order, and switching back restores the first exactly.
#[test]
fn switching_profiles_swaps_what_is_in_the_game_folder() {
    let mut fx = Fixture::new();
    let outfit = fx.install_folder("Outfit", &[("Outfit_P.pak", "outfit")]);
    let hair = fx.install_folder("Hair", &[("Hair_P.pak", "hair")]);
    fx.app.set_enabled(&[outfit, hair], true).unwrap();
    fx.app.apply().unwrap();
    let everything = fx.game_snapshot();
    let first = fx
        .app
        .profiles()
        .unwrap()
        .into_iter()
        .find(|p| p.active)
        .unwrap()
        .id;

    // A fresh profile keeps the order but enables nothing.
    let clean = fx.app.create_profile("Clean", false).unwrap();
    fx.app.switch_profile(clean).unwrap();
    assert!(!fx.game.join("SB/Content/Paks/~mods").exists());
    assert!(fx.app.snapshot().unwrap().pending.is_empty());

    // A change here does not leak into the other profile.
    fx.app.set_enabled(&[hair], true).unwrap();
    fx.app.apply().unwrap();
    assert!(fx
        .game
        .join("SB/Content/Paks/~mods/0020_Hair_P.pak")
        .exists());
    assert!(!fx
        .game
        .join("SB/Content/Paks/~mods/0010_Outfit_P.pak")
        .exists());

    fx.app.switch_profile(first).unwrap();
    assert_eq!(fx.game_snapshot(), everything);

    let profiles = fx.app.profiles().unwrap();
    assert_eq!(profiles.len(), 2);
    assert_eq!(
        profiles
            .iter()
            .find(|p| p.id == clean)
            .unwrap()
            .enabled_count,
        1
    );
}

#[test]
fn a_duplicate_carries_the_enabled_set_and_new_mods_join_every_profile_disabled() {
    let mut fx = Fixture::new();
    let outfit = fx.install_folder("Outfit", &[("Outfit_P.pak", "outfit")]);
    fx.app.set_enabled(&[outfit], true).unwrap();

    let copy = fx.app.create_profile("Copy", true).unwrap();
    // Installed while the first profile is active, it still lands at the end
    // of the copy's order, disabled.
    let hair = fx.install_folder("Hair", &[("Hair_P.pak", "hair")]);
    fx.app.switch_profile(copy).unwrap();

    let mods = fx.app.snapshot().unwrap().mods;
    let outfit_view = mods.iter().find(|m| m.id == outfit).unwrap();
    let hair_view = mods.iter().find(|m| m.id == hair).unwrap();
    assert!(outfit_view.enabled, "a duplicate keeps the enabled set");
    assert!(!hair_view.enabled);
    assert!(hair_view.priority > outfit_view.priority);
}

#[test]
fn profiles_are_named_sensibly_and_the_active_one_cannot_be_deleted() {
    let mut fx = Fixture::new();
    assert!(fx.app.create_profile("  ", false).is_err());
    let id = fx.app.create_profile("Cosmetic", false).unwrap();
    assert!(fx.app.create_profile("cosmetic", false).is_err());
    fx.app.rename_profile(id, "Cosmetics").unwrap();
    fx.app.rename_profile(id, "Cosmetics").unwrap();

    let active = fx
        .app
        .profiles()
        .unwrap()
        .into_iter()
        .find(|p| p.active)
        .unwrap()
        .id;
    assert!(fx.app.delete_profile(active).is_err());
    fx.app.delete_profile(id).unwrap();
    assert!(
        fx.app.delete_profile(active).is_err(),
        "the last profile left stays"
    );
}

/// The mod list flags loose-file clashes the way Mod Organizer 2 does: the
/// mod further down wins, the one above loses, and a mod whose every file is
/// taken is called out as doing nothing.
#[test]
fn file_conflicts_are_flagged_per_mod_and_listed_for_the_files_view() {
    let mut fx = Fixture::new();
    let a = fx.install_folder("Intro A", &[("Movies/Intro.mp4", "a")]);
    let b = fx.install_folder(
        "Intro B",
        &[("Movies/Intro.mp4", "b"), ("Movies/Credits.mp4", "credits")],
    );
    let pak = fx.install_folder("Outfit", &[("Outfit_P.pak", "outfit")]);
    fx.app.set_enabled(&[a, b, pak], true).unwrap();
    fx.app.set_order(&[a, b, pak]).unwrap();

    let mods = fx.app.snapshot().unwrap().mods;
    let flags = |id: i64| {
        mods.iter()
            .find(|m| m.id == id)
            .unwrap()
            .file_conflicts
            .clone()
    };
    let a_flags = flags(a).unwrap();
    assert_eq!((a_flags.wins, a_flags.loses), (0, 1));
    assert!(a_flags.redundant, "Intro A's only file is taken by Intro B");
    let b_flags = flags(b).unwrap();
    assert_eq!((b_flags.wins, b_flags.loses), (1, 0));
    assert!(!b_flags.redundant);
    assert_eq!(
        flags(pak),
        None,
        "a pak's name carries its order, so it clashes with nothing"
    );

    // Disabling the winner leaves nothing to clash with.
    fx.app.set_enabled(&[b], false).unwrap();
    let mods = fx.app.snapshot().unwrap().mods;
    assert!(mods.iter().all(|m| m.file_conflicts.is_none()));
    fx.app.set_enabled(&[b], true).unwrap();

    let for_a = fx.app.vfs_conflicts(Some(a)).unwrap();
    assert_eq!(for_a.len(), 1);
    assert_eq!(for_a[0].path, "SB/Content/Movies/Intro.mp4");
    assert_eq!(for_a[0].provided_by, "Intro B");
    assert_eq!(for_a[0].shadowed_by, vec!["Intro A".to_string()]);
    assert!(fx.app.vfs_conflicts(Some(pak)).unwrap().is_empty());
    assert_eq!(fx.app.vfs_conflicts(None).unwrap().len(), 1);
}

/// What the game created under the virtual file system becomes a mod of its
/// own, enabled where overwrite sat, so the game sees exactly the same files.
#[test]
fn overwrite_becomes_a_mod_without_changing_what_the_game_sees() {
    use sbmm_app::dto::DeploymentMode;

    let mut fx = Fixture::new();
    fx.app.set_deployment_mode(DeploymentMode::Virtual).unwrap();
    assert!(fx.app.create_mod_from_overwrite("Nothing").is_err());

    let overwrite = fx.app.overwrite_dir();
    let created = overwrite.join("SB/Binaries/Win64/ue4ss/Mods/Skip/config.ini");
    fs::create_dir_all(created.parent().unwrap()).unwrap();
    fs::write(&created, "skip=1").unwrap();
    assert_eq!(fx.app.snapshot().unwrap().overwrite.files, 1);

    let before = virtual_view(&fx.game, fx.app.vfs_mapping().unwrap());
    let untouched = fx.game_snapshot();
    assert!(fx.app.create_mod_from_overwrite("  ").is_err());
    let id = fx.app.create_mod_from_overwrite("Skip settings").unwrap();

    assert_eq!(fx.app.snapshot().unwrap().overwrite.files, 0);
    assert!(fs::read_dir(&overwrite).unwrap().next().is_none());
    let new_mod = fx
        .app
        .snapshot()
        .unwrap()
        .mods
        .into_iter()
        .find(|m| m.id == id)
        .unwrap();
    assert!(new_mod.enabled);
    assert_eq!(
        virtual_view(&fx.game, fx.app.vfs_mapping().unwrap()),
        before,
        "the game sees the same file, now from the mod"
    );
    fx.app.apply().unwrap();
    assert_eq!(fx.game_snapshot(), untouched, "nothing physical was needed");
}

#[test]
fn clearing_overwrite_empties_it_and_keeps_the_folder() {
    let mut fx = Fixture::new();
    let overwrite = fx.app.overwrite_dir();
    fs::create_dir_all(overwrite.join("SB/Saved")).unwrap();
    fs::write(overwrite.join("SB/Saved/a.log"), "a").unwrap();
    fs::write(overwrite.join("b.log"), "b").unwrap();

    assert_eq!(fx.app.clear_overwrite().unwrap(), 2);
    assert!(overwrite.is_dir());
    assert!(fs::read_dir(&overwrite).unwrap().next().is_none());
}

/// A zip shaped like the Stellar Blade UE4SS release: a version-named wrapper
/// around the proxy and the runtime, with UE4SS's own `mods.txt`.
fn ue4ss_release(path: &Path, settings: &str) {
    use std::io::Write;
    let file = fs::File::create(path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let options: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
    for (name, contents) in [
        ("UE4SS_v3.1.0-6/dwmapi.dll", "MZ proxy"),
        ("UE4SS_v3.1.0-6/ue4ss/UE4SS.dll", "MZ runtime"),
        ("UE4SS_v3.1.0-6/ue4ss/UE4SS-settings.ini", settings),
        (
            "UE4SS_v3.1.0-6/ue4ss/Mods/mods.txt",
            "ActorDumperMod : 0\nBPModLoaderMod : 1\n",
        ),
        (
            "UE4SS_v3.1.0-6/ue4ss/Mods/BPModLoaderMod/Scripts/main.lua",
            "-- bpml",
        ),
        (
            "UE4SS_v3.1.0-6/ue4ss/Mods/BPModLoaderMod/load_order.txt",
            "",
        ),
        (
            "UE4SS_v3.1.0-6/ue4ss/Mods/ActorDumperMod/Scripts/main.lua",
            "-- dumper",
        ),
    ] {
        zip.start_file(name, options).unwrap();
        zip.write_all(contents.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
}

/// A mod that needs UE4SS says so until UE4SS is there, and installing it
/// keeps UE4SS's own bundled mods switched on beside the user's.
#[test]
fn a_mod_needing_ue4ss_is_flagged_until_ue4ss_is_installed() {
    let mut fx = Fixture::new();
    let skip = fx.install_folder(
        "Cutscene Skip",
        &[("CutsceneSkip/scripts/main.lua", "-- lua")],
    );
    let outfit = fx.install_folder("Outfit", &[("Outfit_P.pak", "outfit")]);
    fx.app.set_enabled(&[skip, outfit], true).unwrap();

    let snap = fx.app.snapshot().unwrap();
    assert_eq!(snap.ue4ss.state, "missing");
    assert_eq!(snap.ue4ss.needed_by, vec!["Cutscene Skip".to_string()]);
    let missing = |snap: &sbmm_app::dto::AppSnapshot, id: i64| {
        snap.mods
            .iter()
            .find(|m| m.id == id)
            .unwrap()
            .missing_requirements
            .clone()
    };
    assert_eq!(missing(&snap, skip), vec!["UE4SS".to_string()]);
    assert!(missing(&snap, outfit).is_empty(), "a pak needs nothing");

    let archive = fx.source.join("UE4SS_v3.1.0-6.zip");
    ue4ss_release(&archive, "[General]\nConsole=0\n");
    fx.app.install_ue4ss(&archive, "3.1.0-6").unwrap();

    let snap = fx.app.snapshot().unwrap();
    assert_eq!(snap.ue4ss.state, "managed");
    assert_eq!(snap.ue4ss.version.as_deref(), Some("3.1.0-6"));
    assert!(missing(&snap, skip).is_empty());

    let win64 = fx.game.join("SB/Binaries/Win64");
    assert!(win64.join("dwmapi.dll").is_file());
    assert!(win64.join("ue4ss/UE4SS.dll").is_file());
    let mods_txt = fs::read_to_string(win64.join("ue4ss/Mods/mods.txt")).unwrap();
    assert!(mods_txt.contains("BPModLoaderMod : 1"), "{mods_txt}");
    assert!(mods_txt.contains("ActorDumperMod : 0"), "{mods_txt}");
    assert!(mods_txt.contains("CutsceneSkip : 1"), "{mods_txt}");

    // Switching the UE4SS mod off brings the warning back.
    fx.app
        .set_enabled(&[snap.ue4ss.mod_id.unwrap()], false)
        .unwrap();
    let snap = fx.app.snapshot().unwrap();
    assert_eq!(snap.ue4ss.state, "disabled");
    assert_eq!(missing(&snap, skip), vec!["UE4SS".to_string()]);
}

/// Updating replaces the previous install rather than stacking a second
/// one, and keeps the user's UE4SS settings.
#[test]
fn updating_ue4ss_keeps_the_users_settings() {
    let mut fx = Fixture::new();
    let first = fx.source.join("first.zip");
    ue4ss_release(&first, "[General]\nConsole=0\n");
    let id = fx.app.install_ue4ss(&first, "3.1.0-5").unwrap();

    // The user turns the console on, in the deployed file.
    let settings = fx.game.join("SB/Binaries/Win64/ue4ss/UE4SS-settings.ini");
    fs::write(&settings, "[General]\nConsole=1\n").unwrap();

    let second = fx.source.join("second.zip");
    ue4ss_release(&second, "[General]\nConsole=0\n");
    let new_id = fx.app.install_ue4ss(&second, "3.1.0-6").unwrap();
    assert_ne!(id, new_id);

    assert_eq!(
        fs::read_to_string(&settings).unwrap(),
        "[General]\nConsole=1\n"
    );
    let ue4ss_mods: Vec<_> = fx
        .app
        .snapshot()
        .unwrap()
        .mods
        .into_iter()
        .filter(|m| m.source == "ue4ss")
        .collect();
    assert_eq!(ue4ss_mods.len(), 1);
    assert_eq!(ue4ss_mods[0].version.as_deref(), Some("3.1.0-6"));
}

#[test]
fn a_hand_placed_ue4ss_counts_and_a_wrong_archive_is_refused() {
    let mut fx = Fixture::new();
    let skip = fx.install_folder("Skip", &[("Skip/scripts/main.lua", "-- lua")]);
    fx.app.set_enabled(&[skip], true).unwrap();

    let dll = fx.game.join("SB/Binaries/Win64/ue4ss/UE4SS.dll");
    fs::create_dir_all(dll.parent().unwrap()).unwrap();
    fs::write(&dll, "MZ").unwrap();
    let snap = fx.app.snapshot().unwrap();
    assert_eq!(snap.ue4ss.state, "manual");
    assert!(snap.mods.iter().all(|m| m.missing_requirements.is_empty()));

    let not_ue4ss = fx.source.join("outfit.zip");
    {
        use std::io::Write;
        let mut zip = zip::ZipWriter::new(fs::File::create(&not_ue4ss).unwrap());
        let options: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
        zip.start_file("Outfit_P.pak", options).unwrap();
        zip.write_all(b"pak").unwrap();
        zip.finish().unwrap();
    }
    let before = fx.app.snapshot().unwrap().mods.len();
    assert!(fx.app.install_ue4ss(&not_ue4ss, "1.0").is_err());
    assert_eq!(fx.app.snapshot().unwrap().mods.len(), before);
}

/// Under the virtual file system `mods.txt` is generated, and it has to start
/// from UE4SS's own, or BPModLoaderMod — and with it every logic mod — would
/// silently stop loading.
#[test]
fn the_generated_mods_txt_keeps_ue4ss_bundled_mods_switched_on() {
    use sbmm_app::dto::DeploymentMode;

    let mut fx = Fixture::new();
    fx.app.set_deployment_mode(DeploymentMode::Virtual).unwrap();
    let archive = fx.source.join("ue4ss.zip");
    ue4ss_release(&archive, "");
    fx.app.install_ue4ss(&archive, "3.1.0-6").unwrap();
    let skip = fx.install_folder("Skip", &[("Skip/scripts/main.lua", "-- lua")]);
    fx.app.set_enabled(&[skip], true).unwrap();

    let vfs = sbmm_vfs::Mapping::load(&fx.app.write_vfs_mapping().unwrap())
        .unwrap()
        .into_vfs();
    let path = sbmm_vfs::VPath::from_relative("SB/Binaries/Win64/ue4ss/Mods/mods.txt").unwrap();
    let file = vfs.overlay().file(&path).unwrap();
    assert_eq!(file.origin, sbmm_vfs::Origin::Generated);
    let text = fs::read_to_string(&file.source).unwrap();
    assert!(text.contains("BPModLoaderMod : 1"), "{text}");
    assert!(text.contains("ActorDumperMod : 0"), "{text}");
    assert!(text.contains("Skip : 1"), "{text}");
    assert!(
        !fx.game
            .join("SB/Binaries/Win64/ue4ss/Mods/mods.txt")
            .exists(),
        "the real one is never written under the virtual file system"
    );
}

/// With automatic apply off, installing UE4SS puts UE4SS in and nothing else
/// the user has staged.
#[test]
fn installing_ue4ss_leaves_staged_changes_staged() {
    let mut fx = Fixture::new();
    fx.app.set_auto_apply(false).unwrap();
    let outfit = fx.install_folder("Outfit", &[("Outfit_P.pak", "outfit")]);
    fx.app.set_enabled(&[outfit], true).unwrap();

    let archive = fx.source.join("ue4ss.zip");
    ue4ss_release(&archive, "");
    fx.app.install_ue4ss(&archive, "3.1.0-6").unwrap();

    assert!(fx.game.join("SB/Binaries/Win64/ue4ss/UE4SS.dll").is_file());
    assert!(!fx.game.join("SB/Content/Paks/~mods").exists());
    let pending = fx.app.snapshot().unwrap().pending;
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].name, "Outfit");
}

/// The real Stellar Blade UE4SS release, installed end to end. The archive is
/// not part of the suite: download `UE4SS_v3.1.0-6.zip` and point
/// `SBMM_UE4SS_ARCHIVE` at it to run this.
#[test]
fn the_real_ue4ss_release_installs_with_its_bundled_mods_switched_on() {
    let Some(archive) = std::env::var_os("SBMM_UE4SS_ARCHIVE").map(PathBuf::from) else {
        eprintln!("SBMM_UE4SS_ARCHIVE is not set; skipping");
        return;
    };
    let mut fx = Fixture::new();
    let skip = fx.install_folder("Skip", &[("Skip/scripts/main.lua", "-- lua")]);
    fx.app.set_enabled(&[skip], true).unwrap();
    fx.app.install_ue4ss(&archive, "3.1.0-6").unwrap();

    let win64 = fx.game.join("SB/Binaries/Win64");
    assert!(win64.join("dwmapi.dll").is_file());
    assert!(win64.join("ue4ss/UE4SS.dll").is_file());
    assert!(win64
        .join("ue4ss/Mods/BPModLoaderMod/Scripts/main.lua")
        .is_file());
    let mods_txt = fs::read_to_string(win64.join("ue4ss/Mods/mods.txt")).unwrap();
    assert!(mods_txt.contains("BPModLoaderMod : 1"), "{mods_txt}");
    assert!(mods_txt.contains("Keybinds : 1"), "{mods_txt}");
    assert!(mods_txt.contains("Skip : 1"), "{mods_txt}");
    let snap = fx.app.snapshot().unwrap();
    assert_eq!(snap.ue4ss.state, "managed");
    assert!(snap.mods.iter().all(|m| m.missing_requirements.is_empty()));
    assert_eq!(
        snap.mods.iter().filter(|m| m.source == "ue4ss").count(),
        1,
        "one UE4SS mod, not one per bundled mod"
    );
}

fn write_zip(path: &Path, files: &[(&str, &str)]) {
    use std::io::Write;
    let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
    let options: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
    for (name, contents) in files {
        zip.start_file(*name, options).unwrap();
        zip.write_all(contents.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
}

const TWO_COLOURS: &[(&str, &str)] = &[
    ("Diving Suit/Black/Outfit_P.pak", "black"),
    ("Diving Suit/Black/Outfit_P.utoc", "black utoc"),
    ("Diving Suit/White/Outfit_P.pak", "white"),
    ("Diving Suit/White/Outfit_P.utoc", "white utoc"),
    ("Diving Suit/readme.txt", "pick one"),
];

/// An archive holding several colours asks which, installs only that one,
/// and remembers the answer for the next time the same mod comes in.
#[test]
fn a_multi_variant_archive_installs_only_the_chosen_version() {
    let mut fx = Fixture::new();
    let archive = fx.source.join("Diving Suit.zip");
    write_zip(&archive, TWO_COLOURS);

    let staged = fx.app.stage_archive(&archive).unwrap();
    assert!(staged.needs_confirmation);
    let choice = staged.variants.clone().unwrap();
    assert!(choice.exclusive);
    assert_eq!(staged.remembered_variants, Vec::<String>::new());

    let id = fx
        .app
        .confirm_install_with(
            &staged.staging_id,
            "Diving Suit",
            None,
            Some(&["White".to_string()]),
        )
        .unwrap();
    fx.app.set_enabled(&[id], true).unwrap();
    fx.app.apply().unwrap();
    let paks = fx.game.join("SB/Content/Paks/~mods");
    assert_eq!(
        fs::read_to_string(paks.join("0010_Outfit_P.pak")).unwrap(),
        "white"
    );
    assert_eq!(
        fs::read_dir(&paks).unwrap().count(),
        2,
        "one colour's pak and utoc, not both colours"
    );

    // The same archive again preselects what was chosen.
    let again = fx.app.stage_archive(&archive).unwrap();
    assert_eq!(again.remembered_variants, vec!["White".to_string()]);
    fx.app.cancel_install(&again.staging_id).unwrap();

    assert!(fx
        .app
        .confirm_install_with(&again.staging_id, "x", None, Some(&["Red".to_string()]))
        .is_err());
}

/// A Nexus download of such an archive waits for the user without losing
/// that it came from Nexus; its next version installs the same way unasked.
#[test]
fn a_downloaded_multi_variant_mod_asks_once_and_then_remembers() {
    let mut fx = Fixture::new();
    let first = fx.source.join("Diving Suit-101-1-0.zip");
    write_zip(&first, TWO_COLOURS);

    let origin = sbmm_app::Origin::nexus(101, 1).with_version(Some("1.0".into()));
    let staged = match fx.app.install_download(&first, "Diving Suit", origin) {
        Err(sbmm_app::AppError::NeedsChoice(staged)) => staged,
        other => panic!("expected to be asked, got {other:?}"),
    };
    let id = fx
        .app
        .confirm_install_with(
            &staged.staging_id,
            &staged.suggested_name,
            None,
            Some(&["Black".to_string()]),
        )
        .unwrap();
    let installed = fx
        .app
        .snapshot()
        .unwrap()
        .mods
        .into_iter()
        .find(|m| m.id == id)
        .unwrap();
    assert_eq!(installed.nexus_mod_id, Some(101), "it is still a Nexus mod");
    assert_eq!(installed.version.as_deref(), Some("1.0"));

    let update = fx.source.join("Diving Suit-101-1-1.zip");
    write_zip(&update, TWO_COLOURS);
    let origin = sbmm_app::Origin::nexus(101, 2).with_version(Some("1.1".into()));
    let new_id = fx
        .app
        .install_download(&update, "Diving Suit", origin)
        .expect("the remembered choice applies without asking");
    fx.app.set_enabled(&[new_id], true).unwrap();
    fx.app.apply().unwrap();
    let pak = fx.game.join("SB/Content/Paks/~mods");
    let contents: Vec<String> = fs::read_dir(&pak)
        .unwrap()
        .flatten()
        .map(|e| fs::read_to_string(e.path()).unwrap())
        .collect();
    assert!(contents.contains(&"black".to_string()));
    assert!(!contents.contains(&"white".to_string()));
}

#[test]
fn a_bundle_of_separate_mods_still_installs_whole_without_asking() {
    let mut fx = Fixture::new();
    let archive = fx.source.join("Lua pack.zip");
    write_zip(
        &archive,
        &[
            ("Photo Mode/scripts/main.lua", "-- photo"),
            ("Quick Sprint/scripts/main.lua", "-- sprint"),
        ],
    );
    let staged = fx.app.stage_archive(&archive).unwrap();
    assert!(staged.variants.is_none());
    assert!(!staged.needs_confirmation);
}
