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

/// A shortcut names its profile, so the name is found however it is cased.
#[test]
fn a_profile_is_found_by_the_name_a_shortcut_carries() {
    let mut fx = Fixture::new();
    let id = fx.app.create_profile("Фото режим", false).unwrap();
    assert_eq!(fx.app.profile_named("  фото РЕЖИМ ").unwrap().id, id);
    let missing = fx.app.profile_named("Vanilla").unwrap_err().to_string();
    assert!(missing.contains("“Vanilla”"), "{missing}");
    assert!(
        missing.contains("“Фото режим”"),
        "names the ones there are: {missing}"
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

const FOMOD_CONFIG: &str = r#"<?xml version="1.0" encoding="UTF-16"?>
<config>
  <moduleName>Planet Diving Suit</moduleName>
  <moduleImage path="fomod\images\header.jpg"/>
  <requiredInstallFiles><file source="Core\Shared_P.pak" destination=""/></requiredInstallFiles>
  <installSteps order="Explicit">
    <installStep name="Colour"><optionalFileGroups order="Explicit">
      <group name="Colour" type="SelectExactlyOne"><plugins order="Explicit">
        <plugin name="Black">
          <description>The original.</description>
          <image path="fomod\images\black.jpg"/>
          <files><folder source="Black" destination=""/></files>
          <conditionFlags><flag name="colour">black</flag></conditionFlags>
          <typeDescriptor><type name="Optional"/></typeDescriptor>
        </plugin>
        <plugin name="White">
          <description>Snow white.</description>
          <files><folder source="White" destination=""/></files>
          <conditionFlags><flag name="colour">white</flag></conditionFlags>
          <typeDescriptor><type name="Recommended"/></typeDescriptor>
        </plugin>
      </plugins></group>
    </optionalFileGroups></installStep>
    <installStep name="Extras">
      <visible><flagDependency flag="colour" value="white"/></visible>
      <optionalFileGroups order="Explicit"><group name="Extras" type="SelectAny"><plugins order="Explicit">
        <plugin name="Gloves"><description>Gloves.</description>
          <files><file source="Extras\Gloves_P.pak" destination=""/></files>
          <typeDescriptor><type name="Optional"/></typeDescriptor></plugin>
      </plugins></group></optionalFileGroups>
    </installStep>
  </installSteps>
</config>"#;

/// A FOMOD archive, its script in UTF-16 as the old tools wrote it.
fn fomod_archive(path: &Path) {
    use std::io::Write;
    let mut config = vec![0xFF, 0xFE];
    for unit in FOMOD_CONFIG.encode_utf16() {
        config.extend(unit.to_le_bytes());
    }
    let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
    let options: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
    zip.start_file("Suit/fomod/ModuleConfig.xml", options)
        .unwrap();
    zip.write_all(&config).unwrap();
    for (name, contents) in [
        ("Suit/fomod/images/header.jpg", "header picture"),
        ("Suit/fomod/images/black.jpg", "black picture"),
        ("Suit/Core/Shared_P.pak", "shared"),
        ("Suit/Black/Outfit_P.pak", "black"),
        ("Suit/White/Outfit_P.pak", "white"),
        ("Suit/Extras/Gloves_P.pak", "gloves"),
    ] {
        zip.start_file(name, options).unwrap();
        zip.write_all(contents.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
}

fn deployed_paks(game: &Path) -> Vec<String> {
    let mut found: Vec<String> = fs::read_dir(game.join("SB/Content/Paks/~mods"))
        .map(|dir| {
            dir.flatten()
                .map(|e| fs::read_to_string(e.path()).unwrap())
                .collect()
        })
        .unwrap_or_default();
    found.sort();
    found
}

/// A FOMOD runs as a wizard: the answers decide what is installed, a step
/// shows only when an earlier answer calls for it, and the chosen option's
/// picture becomes the mod's.
#[test]
fn a_fomod_installs_what_its_wizard_was_answered() {
    let mut fx = Fixture::new();
    let archive = fx.source.join("Planet Diving Suit.zip");
    fomod_archive(&archive);

    let staged = fx.app.stage_archive(&archive).unwrap();
    assert!(staged.needs_confirmation);
    assert!(
        staged.variants.is_none(),
        "the FOMOD asks; nothing else does"
    );
    let view = staged.fomod.clone().expect("the installer is found");
    assert_eq!(view.root, "Suit");
    assert_eq!(view.installer.name, "Planet Diving Suit");
    assert_eq!(view.selection, vec![[0, 0, 1]], "White is recommended");
    assert!(!view.remembered);
    assert!(staged
        .previews
        .contains(&"Suit/fomod/images/black.jpg".to_string()));

    let black = fx
        .app
        .fomod_state(&staged.staging_id, &[[0, 0, 0]])
        .unwrap();
    assert_eq!(black.visible, vec![true, false]);
    assert!(black.problems.is_empty());
    let both = fx
        .app
        .fomod_state(&staged.staging_id, &[[0, 0, 0], [0, 0, 1]])
        .unwrap();
    assert_eq!(both.problems.len(), 1);
    assert!(fx
        .app
        .confirm_install_fomod(&staged.staging_id, "Suit", &[[0, 0, 0], [0, 0, 1]])
        .is_err());

    // Black, with a stale tick on the hidden gloves step.
    let id = fx
        .app
        .confirm_install_fomod(&staged.staging_id, "Suit", &[[0, 0, 0], [1, 0, 0]])
        .unwrap();
    fx.app.set_enabled(&[id], true).unwrap();
    fx.app.apply().unwrap();
    assert_eq!(deployed_paks(&fx.game), vec!["black", "shared"]);

    let preview = fx
        .app
        .preview_data_url(&staged.staging_id, "_preview.jpg")
        .unwrap();
    assert!(preview.starts_with("data:image/jpeg;base64,"));

    // The same archive again opens on last time's answers.
    let again = fx.app.stage_archive(&archive).unwrap();
    let view = again.fomod.unwrap();
    assert!(view.remembered);
    assert_eq!(view.selection, vec![[0, 0, 0], [1, 0, 0]]);
    fx.app.cancel_install(&again.staging_id).unwrap();
}

/// From Nexus, a FOMOD waits for the wizard the first time, and installs the
/// next version with the same answers without asking.
#[test]
fn a_downloaded_fomod_asks_once_and_then_remembers() {
    let mut fx = Fixture::new();
    let first = fx.source.join("Suit-202-1-0.zip");
    fomod_archive(&first);
    let staged = match fx.app.install_download(
        &first,
        "Planet Diving Suit",
        sbmm_app::Origin::nexus(202, 1).with_version(Some("1.0".into())),
    ) {
        Err(sbmm_app::AppError::NeedsChoice(staged)) => staged,
        other => panic!("expected the wizard, got {other:?}"),
    };
    let id = fx
        .app
        .confirm_install_fomod(
            &staged.staging_id,
            &staged.suggested_name,
            &[[0, 0, 1], [1, 0, 0]],
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
    assert_eq!(installed.nexus_mod_id, Some(202));

    let update = fx.source.join("Suit-202-1-1.zip");
    fomod_archive(&update);
    let new_id = fx
        .app
        .install_download(
            &update,
            "Planet Diving Suit",
            sbmm_app::Origin::nexus(202, 2).with_version(Some("1.1".into())),
        )
        .expect("remembered answers install it");
    fx.app.set_enabled(&[id], false).unwrap();
    fx.app.set_enabled(&[new_id], true).unwrap();
    fx.app.apply().unwrap();
    assert_eq!(deployed_paks(&fx.game), vec!["gloves", "shared", "white"]);
}

#[test]
fn previews_are_only_pictures_from_inside_the_library() {
    let mut fx = Fixture::new();
    let id = fx.install_folder(
        "Pictured",
        &[
            ("Outfit_P.pak", "pak"),
            ("preview.png", "png"),
            ("notes.txt", "hi"),
        ],
    );
    let folder = fx
        .store()
        .list_mods()
        .unwrap()
        .into_iter()
        .find(|m| m.id == id)
        .unwrap();
    assert_eq!(folder.image_path.as_deref(), Some("preview.png"));
    let staging = folder.staging_folder;

    assert!(fx
        .app
        .preview_data_url(&staging, "preview.png")
        .unwrap()
        .starts_with("data:image/png;base64,"));
    assert!(
        fx.app.preview_data_url(&staging, "notes.txt").is_err(),
        "not a picture"
    );
    assert!(fx.app.preview_data_url(&staging, "../../sbmm.db").is_err());
    assert!(fx.app.preview_data_url("..", "x.png").is_err());
    assert!(fx
        .app
        .preview_data_url(&staging, "C:/Windows/x.png")
        .is_err());
}

// -- stage 7: getting back from things going wrong -------------------------

/// The first build seen is remembered quietly; a later one is reported until
/// the user acknowledges it.
#[test]
fn a_game_update_is_reported_until_acknowledged() {
    let mut fx = Fixture::new();
    assert!(fx.app.snapshot().unwrap().game_update.is_none());

    fs::write(
        fx.game.join("SB/Binaries/Win64/SB-Win64-Shipping.exe"),
        "a patched executable",
    )
    .unwrap();
    let update = fx.app.snapshot().unwrap().game_update.expect("reported");
    assert_ne!(update.previous, update.current);

    fx.app.acknowledge_game_update().unwrap();
    assert!(fx.app.snapshot().unwrap().game_update.is_none());
}

/// Files an update put back, and files deleted, are repaired; the game's new
/// file is what disabling the mod restores.
#[test]
fn drifted_files_are_repaired_without_losing_the_games_new_file() {
    let mut fx = Fixture::new();
    let chunk = fx.game.join("SB/Content/Movies/Intro.mp4");
    fs::create_dir_all(chunk.parent().unwrap()).unwrap();
    fs::write(&chunk, "vanilla").unwrap();
    let id = fx.install_folder(
        "Intro Replacer",
        &[
            ("Movies/Intro.mp4", "modded"),
            ("Movies/Extra.mp4", "extra"),
        ],
    );
    fx.app.set_enabled(&[id], true).unwrap();
    fx.app.apply().unwrap();
    assert_eq!(fs::read_to_string(&chunk).unwrap(), "modded");

    fs::remove_file(&chunk).unwrap();
    fs::write(&chunk, "vanilla, patched").unwrap();
    let extra: Vec<PathBuf> = fx
        .store()
        .deployed_files_for(id)
        .unwrap()
        .into_iter()
        .map(|(target, _, _)| fx.game.join(target))
        .filter(|p| p.to_string_lossy().contains("Extra"))
        .collect();
    fs::remove_file(&extra[0]).unwrap();
    assert_eq!(fx.app.snapshot().unwrap().drift.len(), 2);

    let report = fx.app.repair_drift().unwrap();
    assert_eq!((report.displaced, report.relinked), (1, 1), "{report:?}");
    assert!(report.failed.is_empty());
    assert!(fx.app.snapshot().unwrap().drift.is_empty());
    assert_eq!(fs::read_to_string(&chunk).unwrap(), "modded");

    fx.app.set_enabled(&[id], false).unwrap();
    fx.app.apply().unwrap();
    assert_eq!(fs::read_to_string(&chunk).unwrap(), "vanilla, patched");
}

/// Every apply that changes something leaves a point; going back to one
/// re-applies it and can itself be undone.
#[test]
fn a_restore_point_puts_the_mod_list_back() {
    let mut fx = Fixture::new();
    let a = fx.install_folder("Outfit", &[("Outfit_P.pak", "a")]);
    let b = fx.install_folder("Hair", &[("Hair_P.pak", "b")]);

    fx.app.set_enabled(&[a], true).unwrap();
    fx.app.apply().unwrap();
    // Applying with nothing changed adds no point.
    fx.app.apply().unwrap();
    fx.app.set_enabled(&[b], true).unwrap();
    fx.app.set_order(&[b, a]).unwrap();
    fx.app.apply().unwrap();

    let points = fx.app.restore_points().unwrap();
    assert_eq!(points.len(), 2, "{points:?}");
    assert!(points.iter().all(|p| p.automatic));
    let only_outfit = points[1].clone();
    assert_eq!(only_outfit.enabled, 1);
    assert!(
        only_outfit.label.contains("Outfit"),
        "{}",
        only_outfit.label
    );

    let report = fx.app.restore_to_point(only_outfit.id).unwrap();
    assert_eq!(report.uninstalled, 0);
    assert_eq!(report.applied.removed, ["Hair"]);
    let mods = fx.app.snapshot().unwrap().mods;
    let enabled: Vec<&str> = mods
        .iter()
        .filter(|m| m.enabled)
        .map(|m| m.name.as_str())
        .collect();
    assert_eq!(enabled, ["Outfit"]);
    assert!(!fx.game_snapshot().keys().any(|k| k.contains("Hair")));

    // The setup left behind was already the newest point, so going forward
    // again is one more restore away and no duplicate was taken.
    let points = fx.app.restore_points().unwrap();
    assert_eq!(points.len(), 2, "{points:?}");
    assert_eq!(points[0].enabled, 2);

    // A setup changed but never applied is kept before going back.
    fx.app.set_enabled(&[b], true).unwrap();
    fx.app.restore_to_point(only_outfit.id).unwrap();
    let points = fx.app.restore_points().unwrap();
    assert!(
        points[0].label.starts_with("Before going back"),
        "{points:?}"
    );

    let manual = fx.app.create_restore_point("Known good").unwrap();
    assert!(fx.app.create_restore_point("  ").is_err());
    fx.app.delete_restore_point(manual).unwrap();
}

/// Saves are zipped on request and before launches when they changed, and
/// a restore backs up what it replaces.
#[test]
fn saves_are_backed_up_and_restored() {
    use sbmm_app::saves::BackupKind;
    let mut fx = Fixture::new();
    let saves = fx.source.join("SaveGames");
    fs::create_dir_all(saves.join("76561198000000000")).unwrap();
    fs::write(
        saves.join("76561198000000000/StellarBladeSave00.sav"),
        "chapter 1",
    )
    .unwrap();
    fx.app.set_saves_dir(Some(&saves)).unwrap();

    let first = fx.app.back_up_saves(BackupKind::Manual).unwrap().unwrap();
    assert_eq!(first.kind, "manual");
    assert!(
        fx.app
            .back_up_saves(BackupKind::Automatic)
            .unwrap()
            .is_none(),
        "nothing changed since the last backup"
    );

    fs::write(
        saves.join("76561198000000000/StellarBladeSave00.sav"),
        "chapter 2, now corrupted",
    )
    .unwrap();
    fx.app.back_up_saves_before_launch();
    let view = fx.app.saves().unwrap();
    assert_eq!(view.files, 1);
    assert_eq!(view.backups.len(), 2);
    assert_eq!(view.backups[0].kind, "auto");

    fx.app.restore_saves(&first.file).unwrap();
    assert_eq!(
        fs::read_to_string(saves.join("76561198000000000/StellarBladeSave00.sav")).unwrap(),
        "chapter 1"
    );
    let kinds = |fx: &Fixture| -> Vec<String> {
        fx.app
            .saves()
            .unwrap()
            .backups
            .into_iter()
            .map(|b| b.kind)
            .collect()
    };
    // What was replaced was already in the newest backup, so no copy of it.
    assert_eq!(kinds(&fx), ["auto", "manual"]);

    // Saves written since the last backup are kept before a restore.
    fs::write(
        saves.join("76561198000000000/StellarBladeSave01.sav"),
        "new slot",
    )
    .unwrap();
    fx.app.restore_saves(&first.file).unwrap();
    assert_eq!(kinds(&fx), ["before-restore", "auto", "manual"]);
    assert!(!saves
        .join("76561198000000000/StellarBladeSave01.sav")
        .exists());

    assert!(fx.app.restore_saves("../sbmm.db").is_err());
    fx.app.delete_save_backup(&first.file).unwrap();
}

// -- stage 8: settings files, notes, tags and pictures ---------------------

/// Editing a mod's INI edits what the game reads, keeps the deployment
/// record honest, and the installed copy is there to reset to.
#[test]
fn a_mods_settings_file_is_edited_where_the_game_reads_it() {
    use sbmm_app::config::ConfigScope;
    let mut fx = Fixture::new();
    let id = fx.install_folder(
        "Photo Mode",
        &[
            ("PhotoMode/Scripts/main.lua", "print('hi')"),
            ("PhotoMode/config.ini", "[Camera]\nFov=90\n"),
            ("PhotoMode/enabled.txt", ""),
        ],
    );
    fx.app.set_enabled(&[id], true).unwrap();
    fx.app.apply().unwrap();

    let scope = ConfigScope::Mod { mod_id: id };
    let files: Vec<String> = fx
        .app
        .config_files(scope)
        .unwrap()
        .into_iter()
        .map(|f| f.path)
        .collect();
    assert!(
        files.contains(&"PhotoMode/config.ini".to_string()),
        "{files:?}"
    );

    fx.app
        .write_config(scope, "PhotoMode/config.ini", "[Camera]\nFov=110\n")
        .unwrap();
    let deployed = fx
        .game
        .join("SB/Binaries/Win64/ue4ss/Mods/PhotoMode/config.ini");
    assert_eq!(
        fs::read_to_string(&deployed).unwrap(),
        "[Camera]\nFov=110\n"
    );
    assert!(
        fx.app.snapshot().unwrap().drift.is_empty(),
        "the record follows the edit"
    );

    let text = fx.app.read_config(scope, "PhotoMode/config.ini").unwrap();
    assert_eq!(text.original.as_deref(), Some("[Camera]\nFov=90\n"));
    let edited = fx.app.config_files(scope).unwrap();
    assert!(edited
        .iter()
        .any(|f| f.path == "PhotoMode/config.ini" && f.edited));

    // Only settings files inside the mod can be reached.
    assert!(fx.app.read_config(scope, "../../sbmm.db").is_err());
    assert!(fx
        .app
        .write_config(scope, "PhotoMode/Scripts/../../x.pak", "")
        .is_err());
}

/// Reinstalling or updating frame generation keeps what the user changed in
/// dlssg_sm86.ini, and takes the new version's defaults for the rest.
#[test]
fn frame_generation_keeps_the_users_ini_across_a_reinstall() {
    use sbmm_upscaler::framegen::Proxy;
    let mut fx = Fixture::new();
    let ini = fx.game.join("SB/Binaries/Win64/dlssg_sm86.ini");

    let first = fake_framegen_release(&fx.source.join("first"));
    fx.app
        .install_framegen(&first, Proxy::Version, "0.4.0")
        .unwrap();
    // The user switches it off, in the game folder like the README says.
    fs::write(&ini, "[General]\nEnabled=0\n").unwrap();

    let second = fake_framegen_release(&fx.source.join("second"));
    fs::write(
        second.root.join("dlssg_sm86.ini"),
        "[General]\nEnabled=1\nLogLevel=2\n",
    )
    .unwrap();
    fx.app
        .install_framegen(&second, Proxy::Version, "0.5.0")
        .unwrap();

    assert_eq!(
        fs::read_to_string(&ini).unwrap(),
        "[General]\nEnabled=0\nLogLevel=2\n"
    );
    // And the new version's own file is what a reset goes back to.
    let id = fx.app.framegen_installed().unwrap().unwrap().mod_id;
    let text = fx
        .app
        .read_config(
            sbmm_app::config::ConfigScope::Mod { mod_id: id },
            "SB/Binaries/Win64/dlssg_sm86.ini",
        )
        .unwrap();
    assert_eq!(
        text.original.as_deref(),
        Some("[General]\nEnabled=1\nLogLevel=2\n")
    );
}

/// The game's own INI files, with the first version found kept to reset to.
#[test]
fn the_games_settings_files_can_be_edited_and_reset() {
    use sbmm_app::config::ConfigScope;
    let mut fx = Fixture::new();
    let config = fx.source.join("Config");
    fs::create_dir_all(config.join("WindowsNoEditor")).unwrap();
    // Unreal writes these as UTF-16 often enough.
    let utf16: Vec<u8> = [0xFF, 0xFE]
        .into_iter()
        .chain(
            "[/Script/Engine.RendererSettings]\r\nr.Tonemapper.Sharpen=0\r\n"
                .encode_utf16()
                .flat_map(u16::to_le_bytes),
        )
        .collect();
    fs::write(config.join("WindowsNoEditor/Engine.ini"), &utf16).unwrap();
    fs::write(config.join("WindowsNoEditor/notes.txt"), "not a setting").unwrap();
    fx.app.set_game_config_dir(Some(&config)).unwrap();

    let files = fx.app.config_files(ConfigScope::Game).unwrap();
    assert_eq!(files.len(), 1, "only INI files: {files:?}");
    let path = &files[0].path;
    assert_eq!(path, "WindowsNoEditor/Engine.ini");

    let text = fx.app.read_config(ConfigScope::Game, path).unwrap();
    assert!(text.text.contains("Sharpen=0"));
    assert_eq!(text.original, None);
    let edited = text.text.replace("Sharpen=0", "Sharpen=1");
    fx.app
        .write_config(ConfigScope::Game, path, &edited)
        .unwrap();

    let bytes = fs::read(config.join("WindowsNoEditor/Engine.ini")).unwrap();
    assert_eq!(&bytes[..2], &[0xFF, 0xFE], "still UTF-16");
    let again = fx.app.read_config(ConfigScope::Game, path).unwrap();
    assert!(again.text.contains("Sharpen=1"));
    assert!(again.original.unwrap().contains("Sharpen=0"));
}

#[test]
fn a_mod_carries_notes_tags_and_a_chosen_picture() {
    let mut fx = Fixture::new();
    let id = fx.install_folder(
        "Outfit",
        &[
            ("Outfit_P.pak", "pak"),
            ("images/cover.png", "png"),
            ("images/back.jpg", "jpg"),
        ],
    );
    let images = fx.app.mod_images(id).unwrap();
    assert_eq!(images.len(), 2);
    let installed = fx.app.snapshot().unwrap().mods[0].image.clone();
    assert_eq!(installed.as_deref(), Some("images/cover.png"));

    fx.app.set_mod_image(id, Some("images/back.jpg")).unwrap();
    assert!(fx.app.set_mod_image(id, Some("Outfit_P.pak")).is_err());
    fx.app
        .set_mod_notes(id, "  Goes with the white hair  ")
        .unwrap();
    fx.app
        .set_mod_tags(
            id,
            &[
                "outfit".into(),
                " Outfit ".into(),
                "".into(),
                "white".into(),
            ],
        )
        .unwrap();

    let view = fx.app.snapshot().unwrap().mods.remove(0);
    assert_eq!(view.image.as_deref(), Some("images/back.jpg"));
    assert_eq!(view.notes.as_deref(), Some("Goes with the white hair"));
    assert_eq!(view.tags, ["outfit", "white"]);

    fx.app.set_mod_notes(id, " ").unwrap();
    fx.app.set_mod_image(id, None).unwrap();
    let view = fx.app.snapshot().unwrap().mods.remove(0);
    assert_eq!((view.notes, view.image), (None, None));
}

// -- stage 9: what a mod changes, load order rules, Nexus pages ------------

#[test]
fn load_order_rules_are_reported_when_broken_and_sorted_by() {
    let mut fx = Fixture::new();
    let a = fx.install_folder("Base Outfit", &[("A_P.pak", "a")]);
    let b = fx.install_folder("Hair", &[("B_P.pak", "b")]);
    let c = fx.install_folder("Outfit Recolour", &[("C_P.pak", "c")]);
    fx.app.set_order(&[c, a, b]).unwrap();

    // The recolour has to win over the base outfit.
    fx.app.add_order_rule(a, c).unwrap();
    let mods = fx.app.snapshot().unwrap().mods;
    let recolour = mods.iter().find(|m| m.id == c).unwrap();
    assert_eq!(recolour.misordered, ["should load after Base Outfit"]);
    assert!(fx.app.order_rules().unwrap()[0].broken);

    assert!(
        fx.app.add_order_rule(c, a).is_err(),
        "contradicts the first"
    );
    assert!(fx.app.add_order_rule(a, a).is_err());

    assert_eq!(fx.app.sort_by_rules().unwrap(), 2);
    let order: Vec<i64> = fx
        .app
        .snapshot()
        .unwrap()
        .mods
        .iter()
        .map(|m| m.id)
        .collect();
    assert_eq!(order, [a, c, b]);
    assert!(!fx.app.order_rules().unwrap()[0].broken);
    assert_eq!(fx.app.sort_by_rules().unwrap(), 0);

    fx.app.uninstall(c).unwrap();
    assert!(fx.app.order_rules().unwrap().is_empty());
}

#[test]
fn a_mods_contents_say_what_it_changes_and_who_else_does() {
    let mut fx = Fixture::new();
    let a = fx.install_folder(
        "Intro A",
        &[("Movies/Intro.mp4", "a"), ("Movies/Credits.mp4", "c")],
    );
    let b = fx.install_folder("Intro B", &[("Movies/Intro.mp4", "b")]);
    fx.app.set_enabled(&[a, b], true).unwrap();

    let contents = fx.app.mod_contents(a).unwrap();
    assert_eq!(contents.file_count, 2);
    assert!(contents
        .files
        .iter()
        .any(|f| f.ends_with("Movies/Intro.mp4")));
    assert_eq!(contents.clashes.len(), 1);
    let clash = &contents.clashes[0];
    assert_eq!((clash.mod_id, clash.shared, clash.wins), (b, 1, false));
    assert!(
        fx.app.mod_contents(b).unwrap().clashes[0].wins,
        "later wins"
    );
}

#[test]
fn nexus_requirements_are_checked_against_what_is_installed() {
    use sbmm_nexus::{ModDetails, Requirement};
    let mut fx = Fixture::new();
    let install = |fx: &mut Fixture, name: &str, nexus: i64| {
        let folder = fx.source.join(name);
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join(format!("{name}_P.pak")), name).unwrap();
        let staged = fx.app.stage_folder(&folder).unwrap();
        fx.app
            .commit_install(
                &staged.staging_id,
                name,
                None,
                sbmm_app::Origin {
                    source: "nexus".into(),
                    nexus_mod_id: Some(nexus),
                    nexus_file_id: None,
                    version: None,
                },
            )
            .unwrap()
    };
    let suit = install(&mut fx, "Suit", 1496);
    fx.app.set_enabled(&[suit], true).unwrap();
    assert!(fx.app.nexus_details_due(suit).unwrap());

    let details = ModDetails {
        mod_id: 1496,
        name: "CNS".into(),
        author: Some("Someone".into()),
        version: Some("2.1".into()),
        summary: None,
        endorsements: 1,
        downloads: 2,
        picture_url: Some(
            "https://staticdelivery.nexusmods.com/mods/7804/images/1496/a.jpg".into(),
        ),
        updated_at: None,
        adult: false,
        category: None,
        tags: vec![],
        requirements: vec![
            Requirement {
                mod_id: Some(5),
                name: "Body Framework".into(),
                url: String::new(),
                notes: None,
            },
            Requirement {
                mod_id: None,
                name: "Some Tool".into(),
                url: String::new(),
                notes: None,
            },
        ],
    };
    fx.app.record_nexus_details(&details).unwrap();
    assert!(!fx.app.nexus_details_due(suit).unwrap());

    let view = fx.app.nexus_view(suit).unwrap().unwrap();
    let states: Vec<&str> = view.requirements.iter().map(|r| r.state.as_str()).collect();
    assert_eq!(states, ["missing", "external"]);
    let suit_view = |fx: &Fixture| {
        fx.app
            .snapshot()
            .unwrap()
            .mods
            .into_iter()
            .find(|m| m.id == suit)
            .unwrap()
    };
    assert_eq!(suit_view(&fx).missing_requirements, ["Body Framework"]);

    let body = install(&mut fx, "Body", 5);
    assert_eq!(
        fx.app.nexus_view(suit).unwrap().unwrap().requirements[0].state,
        "disabled"
    );
    fx.app.set_enabled(&[body], true).unwrap();
    assert!(suit_view(&fx).missing_requirements.is_empty());

    // The page picture goes beside the mod's own and becomes its picture.
    let picture = fx
        .app
        .nexus_picture_path(suit, details.picture_url.as_deref().unwrap())
        .unwrap()
        .unwrap();
    fs::write(&picture, "jpg").unwrap();
    fx.app.adopt_nexus_picture(suit, &picture).unwrap();
    assert_eq!(suit_view(&fx).image.as_deref(), Some("_nexus.jpg"));
    assert!(fx
        .app
        .mod_images(suit)
        .unwrap()
        .contains(&"_nexus.jpg".to_string()));
}

// -- stage 10: disk space and the watched downloads folder -----------------

/// Back-date a file or folder past the "may still be in use" window.
fn age(path: &Path) {
    let an_hour_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    fs::File::open(path)
        .unwrap()
        .set_modified(an_hour_ago)
        .unwrap();
}

#[test]
fn cleaning_frees_only_what_nothing_needs() {
    use sbmm_app::storage::Cleanup;
    let mut fx = Fixture::new();
    let kept = fx.install_folder("Outfit", &[("Outfit_P.pak", "outfit")]);
    let used = fx.install_folder("Hair", &[("Hair_P.pak", "hair")]);
    fx.app.set_enabled(&[used], true).unwrap();

    let mods = fx.app.folders().unwrap().mods;
    let leftover = Path::new(&mods).join("Cancelled Install");
    fs::create_dir_all(&leftover).unwrap();
    fs::write(leftover.join("big.pak"), vec![0u8; 4096]).unwrap();
    age(&leftover);
    // An install still waiting for an answer is too new to touch.
    let waiting = Path::new(&mods).join("Waiting");
    fs::create_dir_all(&waiting).unwrap();
    fs::write(waiting.join("x.pak"), "x").unwrap();

    let downloads = fx.app.downloads_dir();
    fs::create_dir_all(&downloads).unwrap();
    fs::write(downloads.join("old.zip"), vec![0u8; 1000]).unwrap();
    age(&downloads.join("old.zip"));
    fs::write(downloads.join("just-now.zip"), "new").unwrap();

    let report = fx.app.storage_report().unwrap();
    let part = |key: &str| report.parts.iter().find(|p| p.key == key).unwrap().clone();
    assert_eq!(part("mods").cleanable_bytes, 4096);
    assert_eq!(part("downloads").cleanable_bytes, 1000);
    let unused: Vec<i64> = report.unused_mods.iter().map(|m| m.id).collect();
    assert_eq!(unused, [kept], "never enabled anywhere");

    assert_eq!(fx.app.clean(Cleanup::Orphans).unwrap(), 4096);
    assert_eq!(fx.app.clean(Cleanup::Downloads).unwrap(), 1000);
    assert!(!leftover.exists());
    assert!(waiting.exists());
    assert!(downloads.join("just-now.zip").exists());
    // The installed mods are untouched.
    fx.app.apply().unwrap();
    assert!(fx.game_snapshot().keys().any(|k| k.contains("Hair_P")));
}

#[test]
fn a_watched_folder_offers_each_new_archive_once() {
    let mut fx = Fixture::new();
    let browser = fx.source.join("Downloads");
    fs::create_dir_all(&browser).unwrap();
    fs::write(browser.join("from last year.zip"), "old").unwrap();
    fs::write(browser.join("notes.txt"), "not a mod").unwrap();

    fx.app.set_watch_dir(Some(&browser)).unwrap();
    assert!(
        fx.app.new_watched_archives().unwrap().is_empty(),
        "what was there is seen"
    );

    fs::write(browser.join("Planet Diving Suit-101-1-3.7z"), "new").unwrap();
    let found = fx.app.new_watched_archives().unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name, "Planet Diving Suit-101-1-3.7z");

    fx.app.mark_seen(&[found[0].key.clone()]).unwrap();
    assert!(fx.app.new_watched_archives().unwrap().is_empty());
    // The same name with other contents is a new download.
    fs::write(
        browser.join("Planet Diving Suit-101-1-3.7z"),
        "newer, larger",
    )
    .unwrap();
    assert_eq!(fx.app.new_watched_archives().unwrap().len(), 1);

    fx.app.set_watch_dir(None).unwrap();
    assert!(fx.app.new_watched_archives().unwrap().is_empty());
    assert!(fx.app.set_watch_dir(Some(&browser.join("nope"))).is_err());
}

// -- stage 11: mod lists and updates ----------------------------------------

/// Install a loose folder as if it came from Nexus.
fn install_from_nexus(
    fx: &mut Fixture,
    name: &str,
    files: &[(&str, &str)],
    nexus: (i64, i64),
) -> i64 {
    let folder = fx.source.join(format!("{name}-{}", nexus.1));
    for (rel, contents) in files {
        let path = folder.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }
    let staged = fx.app.stage_folder(&folder).unwrap();
    fx.app
        .commit_install(
            &staged.staging_id,
            name,
            None,
            sbmm_app::Origin::nexus(nexus.0, nexus.1).with_version(Some(format!("1.{}", nexus.1))),
        )
        .unwrap()
}

#[test]
fn a_mod_list_moves_a_setup_to_another_install() {
    let mut home = Fixture::new();
    let outfit = install_from_nexus(&mut home, "Outfit", &[("Outfit_P.pak", "o")], (101, 1));
    let hair = install_from_nexus(&mut home, "Hair", &[("Hair_P.pak", "h")], (202, 1));
    let local = home.install_folder("My Recolour", &[("Recolour_P.pak", "r")]);
    home.app.set_order(&[local, hair, outfit]).unwrap();
    home.app.set_enabled(&[outfit, local], true).unwrap();
    home.app.add_order_rule(local, outfit).unwrap();
    home.app.set_mod_tags(outfit, &["suit".into()]).unwrap();

    let file = home.source.join("setup.sbmm.json");
    assert_eq!(home.app.export_mod_list(&file).unwrap(), 3);

    // Elsewhere, only the hair mod is installed, under another name.
    let mut away = Fixture::new();
    let theirs = install_from_nexus(
        &mut away,
        "Hair (renamed)",
        &[("Hair_P.pak", "h")],
        (202, 1),
    );
    let plan = away.app.plan_import(&file).unwrap();
    let statuses: Vec<(&str, &str)> = plan
        .entries
        .iter()
        .map(|e| (e.name.as_str(), e.status.as_str()))
        .collect();
    assert_eq!(
        statuses,
        [
            ("My Recolour", "manual"),
            ("Hair", "installed"),
            ("Outfit", "download")
        ]
    );
    assert_eq!(plan.rules, 1);

    let outcome = away.app.import_mod_list(&file, "From home").unwrap();
    assert_eq!(outcome.matched, 1);
    assert_eq!(outcome.manual, ["My Recolour"]);
    assert_eq!(outcome.downloads.len(), 1);
    let snapshot = away.app.snapshot().unwrap();
    assert!(snapshot
        .profiles
        .iter()
        .any(|p| p.active && p.name == "From home"));
    assert!(
        !snapshot
            .mods
            .iter()
            .find(|m| m.id == theirs)
            .unwrap()
            .enabled
    );

    // The download arrives and takes its place, enabled, after the hair mod.
    let arrived = install_from_nexus(&mut away, "Outfit", &[("Outfit_P.pak", "o")], (101, 1));
    let mods = away.app.snapshot().unwrap().mods;
    let order: Vec<i64> = mods.iter().map(|m| m.id).collect();
    assert_eq!(order, [theirs, arrived]);
    assert!(mods.iter().find(|m| m.id == arrived).unwrap().enabled);

    assert!(
        away.app.import_mod_list(&file, "From home").is_err(),
        "name taken"
    );
    let junk = away.source.join("junk.json");
    fs::write(&junk, "{\"hello\": 1}").unwrap();
    assert!(away.app.plan_import(&junk).is_err());
}

/// An update replaces the version it updates, keeping everything the user
/// set on it.
#[test]
fn an_update_takes_the_old_versions_place() {
    let mut fx = Fixture::new();
    let other = fx.install_folder("Base", &[("Base_P.pak", "b")]);
    let old = install_from_nexus(
        &mut fx,
        "Outfit",
        &[("Outfit_P.pak", "v1"), ("Outfit.ini", "[Look]\nShine=1\n")],
        (101, 1),
    );
    fx.app.set_order(&[old, other]).unwrap();
    fx.app.set_enabled(&[old, other], true).unwrap();
    fx.app.add_order_rule(other, old).unwrap();
    fx.app.sort_by_rules().unwrap();
    fx.app.set_mod_tags(old, &["suit".into()]).unwrap();
    fx.app.set_mod_notes(old, "the white one").unwrap();
    fx.app.apply().unwrap();
    fx.app
        .write_config(
            sbmm_app::config::ConfigScope::Mod { mod_id: old },
            "Outfit.ini",
            "[Look]\nShine=5\n",
        )
        .unwrap();
    fx.app.record_update_check(old, Some("1.2")).unwrap();
    assert_eq!(fx.app.updatable_mods().unwrap().len(), 1);

    fx.app.expect_update(old).unwrap();
    let new = install_from_nexus(
        &mut fx,
        "Outfit",
        &[
            ("Outfit_P.pak", "v2"),
            ("Outfit.ini", "[Look]\nShine=1\nGloss=0\n"),
        ],
        (101, 2),
    );

    let mods = fx.app.snapshot().unwrap().mods;
    assert!(!mods.iter().any(|m| m.id == old), "the old version is gone");
    let view = mods.iter().find(|m| m.id == new).unwrap();
    assert!(view.enabled);
    assert_eq!(view.tags, ["suit"]);
    assert_eq!(view.notes.as_deref(), Some("the white one"));
    assert!(view.misordered.is_empty());
    let rules = fx.app.order_rules().unwrap();
    assert_eq!((rules[0].before_id, rules[0].after_id), (other, new));

    let ini = fx
        .app
        .read_config(
            sbmm_app::config::ConfigScope::Mod { mod_id: new },
            "Outfit.ini",
        )
        .unwrap();
    assert_eq!(ini.text, "[Look]\nShine=5\nGloss=0\n");
    let game = fx.game_snapshot();
    let pak = game
        .iter()
        .find(|(k, _)| k.contains("Outfit_P.pak"))
        .unwrap();
    assert_eq!(pak.1, b"v2");
    assert!(fx.app.updatable_mods().unwrap().is_empty());
}

/// A mod's Engine.ini is merged into the game's own, never copied over it,
/// and disabling it puts back exactly what was there.
#[test]
fn an_engine_ini_tweak_is_merged_in_and_taken_back_out() {
    let mut fx = Fixture::new();
    let config = fx.source.join("Config");
    let engine = config.join("WindowsNoEditor/Engine.ini");
    fs::create_dir_all(engine.parent().unwrap()).unwrap();
    fs::write(&engine, "[Core.System]\r\nPaths=../../Engine\r\n\r\n[SystemSettings]\r\nr.Shadow.MaxResolution=1024\r\n").unwrap();
    let before = fs::read_to_string(&engine).unwrap();
    fx.app.set_game_config_dir(Some(&config)).unwrap();

    let sharp = fx.install_folder(
        "Sharper Shadows",
        &[
            (
                "Engine.ini",
                "[SystemSettings]\nr.Shadow.MaxResolution=4096\nr.Shadow.RadiusThreshold=0.01\n",
            ),
            ("readme.txt", "x"),
        ],
    );
    let fog = fx.install_folder(
        "No Fog",
        &[(
            "Config/Engine.ini",
            "[SystemSettings]\nr.Fog=0\nr.Shadow.MaxResolution=2048\n",
        )],
    );
    let view = |fx: &Fixture, id: i64| {
        fx.app
            .snapshot()
            .unwrap()
            .mods
            .into_iter()
            .find(|m| m.id == id)
            .unwrap()
    };
    assert_eq!(view(&fx, sharp).component_types, ["configTweak"]);

    fx.app.set_order(&[sharp, fog]).unwrap();
    fx.app.set_enabled(&[sharp, fog], true).unwrap();
    assert!(
        fx.app.snapshot().unwrap().pending.len() == 2,
        "waiting for apply"
    );
    let report = fx.app.apply().unwrap();
    assert!(report.failed.is_empty(), "{report:?}");
    assert!(view(&fx, sharp).deployed);

    let merged = fs::read_to_string(&engine).unwrap();
    // The later mod wins the shared key; the rest of the file is untouched.
    assert!(merged.contains("r.Shadow.MaxResolution=2048"), "{merged}");
    assert!(merged.contains("r.Shadow.RadiusThreshold=0.01"));
    assert!(merged.contains("r.Fog=0"));
    assert!(merged.starts_with("[Core.System]\r\nPaths=../../Engine\r\n"));
    assert!(
        !fx.game_snapshot().keys().any(|k| k.ends_with("Engine.ini")),
        "nothing in the game folder"
    );

    // The user changes one of the keys by hand; that survives disabling.
    let edited = merged.replace("r.Fog=0", "r.Fog=1");
    fs::write(&engine, &edited).unwrap();
    fx.app.set_enabled(&[sharp, fog], false).unwrap();
    fx.app.apply().unwrap();
    let after = fs::read_to_string(&engine).unwrap();
    assert!(after.contains("r.Shadow.MaxResolution=1024"), "{after}");
    assert!(!after.contains("RadiusThreshold"));
    assert!(after.contains("r.Fog=1"), "the user's own value stays");
    assert!(!view(&fx, sharp).deployed);

    // Uninstalling an enabled tweak takes it out too.
    fs::write(&engine, &before).unwrap();
    fx.app.set_enabled(&[sharp], true).unwrap();
    fx.app.apply().unwrap();
    fx.app.uninstall(sharp).unwrap();
    assert_eq!(fs::read_to_string(&engine).unwrap(), before);
}

// -- installs repaired from a real library ----------------------------------

/// A CNS outfit installed while the dialog forced "pak" on the whole
/// archive had its definition among the paks; choosing a type now only
/// places the unrecognised files, and detecting again repairs old installs.
#[test]
fn a_forced_type_only_places_what_was_not_recognised() {
    let mut fx = Fixture::new();
    let folder = fx.source.join("Herald");
    for (rel, body) in [
        ("~mods/CustomNanosuitSystem/Herald_P.pak", "p"),
        ("~mods/CustomNanosuitSystem/Herald_P.utoc", "u"),
        ("~mods/CustomNanosuitSystem/Herald_P.ucas", "c"),
        ("~mods/CustomNanosuitSystem/Herald.dekcns.json", "{}"),
        ("tool.dll", "MZ"),
    ] {
        let path = folder.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }
    let staged = fx.app.stage_folder(&folder).unwrap();
    assert!(staged.needs_confirmation, "the dll is asked about");
    let id = fx
        .app
        .confirm_install(&staged.staging_id, "Herald", Some(ModType::RootBinary))
        .unwrap();
    fx.app.set_enabled(&[id], true).unwrap();
    fx.app.apply().unwrap();
    let game = fx.game_snapshot();
    assert!(
        game.contains_key("SB/Content/Paks/~mods/CustomNanosuitSystem/Herald.dekcns.json"),
        "{:?}",
        game.keys()
    );
    assert!(game.contains_key("SB/Binaries/Win64/tool.dll"));
}

#[test]
fn detecting_again_repairs_a_mod_installed_the_old_way() {
    let mut fx = Fixture::new();
    let id = fx.install_folder(
        "Better Jiggle",
        &[
            ("ue4ss/Mods/JiggleUpdate/Scripts/main.lua", "print(1)"),
            ("ue4ss/UE4SS-settings.ini", "[General]\n"),
        ],
    );
    // As the old installer left it: the whole archive forced to "Lua".
    let staging = fx.app.folders().unwrap().mods;
    let folder = fx.app.snapshot().unwrap().mods[0].folder.clone();
    let broken = vec![sbmm_core::model::DetectedComponent {
        mod_type: ModType::Ue4ssLua,
        confidence: sbmm_core::model::Confidence::High,
        files: vec![sbmm_core::model::ComponentFile {
            source: "ue4ss/Mods/JiggleUpdate/Scripts/main.lua".into(),
            target:
                "SB/Binaries/Win64/ue4ss/Mods/JiggleUpdate/ue4ss/Mods/JiggleUpdate/Scripts/main.lua"
                    .into(),
        }],
        target_subdir: "SB/Binaries/Win64/ue4ss/Mods/JiggleUpdate".into(),
        ue4ss_mod_name: Some("JiggleUpdate".into()),
        pak_sets: vec![],
        warnings: vec![],
        notes: vec![],
    }];
    fx.store().set_components(id, &broken).unwrap();
    assert!(Path::new(&staging).join(&folder).is_dir());
    fx.app.set_enabled(&[id], true).unwrap();
    fx.app.apply().unwrap();
    assert!(fx
        .game_snapshot()
        .keys()
        .any(|k| k.contains("JiggleUpdate/ue4ss/Mods")));

    let report = fx.app.redetect_mods(None).unwrap();
    assert_eq!(report.changed, ["Better Jiggle"]);
    assert!(report.applied.is_some());
    let game = fx.game_snapshot();
    assert!(
        game.contains_key("SB/Binaries/Win64/ue4ss/Mods/JiggleUpdate/Scripts/main.lua"),
        "{:?}",
        game.keys()
    );
    assert!(game.contains_key("SB/Binaries/Win64/ue4ss/UE4SS-settings.ini"));
    assert!(!game.keys().any(|k| k.contains("JiggleUpdate/ue4ss/Mods")));

    let again = fx.app.redetect_mods(None).unwrap();
    assert!(again.changed.is_empty());
    assert_eq!(again.unchanged, 1);
}
