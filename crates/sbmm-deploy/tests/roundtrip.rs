//! The contract that matters most: after disabling everything, the game folder
//! is byte-for-byte what it was before the manager touched it.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use sbmm_core::plan::{DeployPlan, PlannedFile};
use sbmm_deploy::backend::{DeployContext, DriftKind};
use sbmm_deploy::{DeployBackend, HardlinkBackend};

struct Fixture {
    _dir: tempfile::TempDir,
    game: PathBuf,
    staging: PathBuf,
    backup: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("game");
        let staging = dir.path().join("staging");
        let backup = dir.path().join("backup");
        for p in [&game, &staging, &backup] {
            fs::create_dir_all(p).unwrap();
        }
        Self {
            _dir: dir,
            game,
            staging,
            backup,
        }
    }

    fn ctx(&self) -> DeployContext {
        DeployContext {
            game_root: self.game.clone(),
            backup_root: self.backup.clone(),
        }
    }

    fn write_game(&self, rel: &str, contents: &str) {
        write_file(&self.game.join(rel), contents);
    }

    fn write_staged(&self, rel: &str, contents: &str) {
        write_file(&self.staging.join(rel), contents);
    }

    fn plan(&self, mod_id: i64, files: &[(&str, &str)]) -> DeployPlan {
        DeployPlan {
            mod_id,
            staging_root: self.staging.clone(),
            files: files
                .iter()
                .map(|(source, target)| PlannedFile {
                    source: PathBuf::from(source),
                    target: PathBuf::from(target),
                })
                .collect(),
            ue4ss_registrations: Vec::new(),
        }
    }
}

fn write_file(path: &Path, contents: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

/// Every file under `root`, keyed by relative path, with contents.
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
        if path.is_dir() {
            // Record directories too, so a stray empty folder is caught.
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            out.insert(format!("{rel}/"), Vec::new());
            walk(root, &path, out);
        } else {
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            out.insert(rel, fs::read(&path).unwrap());
        }
    }
}

#[test]
fn deploy_then_undeploy_restores_the_game_folder_exactly() {
    let fx = Fixture::new();
    fx.write_game("SB/Content/Paks/pakchunk0-WindowsNoEditor.pak", "vanilla");
    fx.write_staged("Outfit_P.pak", "mod payload");

    let before = snapshot(&fx.game);

    let backend = HardlinkBackend;
    let plan = fx.plan(
        1,
        &[("Outfit_P.pak", "SB/Content/Paks/~mods/0100_Outfit_P.pak")],
    );
    let manifest = backend.deploy(&fx.ctx(), &plan).unwrap();

    assert!(fx
        .game
        .join("SB/Content/Paks/~mods/0100_Outfit_P.pak")
        .exists());
    assert_ne!(snapshot(&fx.game), before);

    backend.undeploy(&fx.ctx(), &manifest).unwrap();
    assert_eq!(
        snapshot(&fx.game),
        before,
        "the game folder must be identical after undeploy, including created directories"
    );
}

#[test]
fn a_displaced_game_file_is_backed_up_and_restored() {
    let fx = Fixture::new();
    let target = "SB/Content/Movies/Intro.mp4";
    fx.write_game(target, "original movie");
    fx.write_staged("Intro.mp4", "modded movie");

    let before = snapshot(&fx.game);
    let backend = HardlinkBackend;
    let plan = fx.plan(1, &[("Intro.mp4", target)]);

    let manifest = backend.deploy(&fx.ctx(), &plan).unwrap();
    assert_eq!(
        fs::read_to_string(fx.game.join(target)).unwrap(),
        "modded movie"
    );
    assert!(
        manifest.files[0].backup.is_some(),
        "the original must be preserved"
    );

    backend.undeploy(&fx.ctx(), &manifest).unwrap();
    assert_eq!(
        fs::read_to_string(fx.game.join(target)).unwrap(),
        "original movie"
    );
    assert_eq!(snapshot(&fx.game), before);
}

#[test]
fn undeploy_never_touches_files_it_did_not_create() {
    let fx = Fixture::new();
    fx.write_staged("Mine_P.pak", "mine");

    let backend = HardlinkBackend;
    let plan = fx.plan(1, &[("Mine_P.pak", "SB/Content/Paks/~mods/Mine_P.pak")]);
    let manifest = backend.deploy(&fx.ctx(), &plan).unwrap();

    // The user drops their own pak into the same folder by hand.
    fx.write_game("SB/Content/Paks/~mods/Manual_P.pak", "hand placed");

    backend.undeploy(&fx.ctx(), &manifest).unwrap();

    assert!(!fx.game.join("SB/Content/Paks/~mods/Mine_P.pak").exists());
    assert_eq!(
        fs::read_to_string(fx.game.join("SB/Content/Paks/~mods/Manual_P.pak")).unwrap(),
        "hand placed",
        "a non-empty mod folder must survive, along with the user's own file"
    );
}

#[test]
fn several_mods_can_be_removed_independently() {
    let fx = Fixture::new();
    fx.write_staged("a.pak", "a");
    fx.write_staged("b.pak", "b");

    let before = snapshot(&fx.game);
    let backend = HardlinkBackend;

    // The app keeps one manifest for the whole deployed set, because directory
    // ownership is shared: the first mod creates `~mods`, the last one out
    // should be able to take it away again.
    let mut global = sbmm_deploy::Manifest::default();
    global.merge(
        backend
            .deploy(
                &fx.ctx(),
                &fx.plan(1, &[("a.pak", "SB/Content/Paks/~mods/a.pak")]),
            )
            .unwrap(),
    );
    global.merge(
        backend
            .deploy(
                &fx.ctx(),
                &fx.plan(2, &[("b.pak", "SB/Content/Paks/~mods/b.pak")]),
            )
            .unwrap(),
    );

    let first = global.take_mod(1);
    backend.undeploy(&fx.ctx(), &first).unwrap();

    assert!(!fx.game.join("SB/Content/Paks/~mods/a.pak").exists());
    assert!(
        fx.game.join("SB/Content/Paks/~mods/b.pak").exists(),
        "removing one mod must not disturb another"
    );

    let second = global.take_mod(2);
    backend.undeploy(&fx.ctx(), &second).unwrap();

    assert_eq!(
        snapshot(&fx.game),
        before,
        "with the last mod gone the folders it needed go too"
    );
}

#[test]
fn verify_reports_outside_changes() {
    let fx = Fixture::new();
    fx.write_staged("a.pak", "a");
    let backend = HardlinkBackend;
    let manifest = backend
        .deploy(
            &fx.ctx(),
            &fx.plan(1, &[("a.pak", "SB/Content/Paks/~mods/a.pak")]),
        )
        .unwrap();

    assert!(backend.verify(&fx.ctx(), &manifest).unwrap().is_empty());

    fs::remove_file(fx.game.join("SB/Content/Paks/~mods/a.pak")).unwrap();
    let drifts = backend.verify(&fx.ctx(), &manifest).unwrap();
    assert_eq!(drifts.len(), 1);
    assert_eq!(drifts[0].kind, DriftKind::Missing);
}

#[test]
fn a_traversing_target_is_refused() {
    let fx = Fixture::new();
    fx.write_staged("evil.dll", "payload");

    let backend = HardlinkBackend;
    let plan = fx.plan(1, &[("evil.dll", "../../windows/system32/evil.dll")]);

    let err = backend.deploy(&fx.ctx(), &plan).unwrap_err();
    assert!(
        matches!(err, sbmm_deploy::DeployError::EscapesGameRoot(_)),
        "got {err:?}"
    );
    assert!(snapshot(&fx.game).is_empty(), "nothing may be written");
}

#[test]
fn a_failed_deployment_leaves_nothing_behind() {
    let fx = Fixture::new();
    fx.write_staged("good.pak", "good");

    let before = snapshot(&fx.game);
    let backend = HardlinkBackend;
    // The second file does not exist in staging, so the pass must unwind.
    let plan = fx.plan(
        1,
        &[
            ("good.pak", "SB/Content/Paks/~mods/good.pak"),
            ("missing.pak", "SB/Content/Paks/~mods/missing.pak"),
        ],
    );

    assert!(backend.deploy(&fx.ctx(), &plan).is_err());
    assert_eq!(
        snapshot(&fx.game),
        before,
        "a partial deployment must roll itself back"
    );
}

/// Repair puts the mod back without losing anything found in its place.
#[test]
fn repair_restores_the_mod_and_keeps_what_replaced_it() {
    let fx = Fixture::new();
    fx.write_game("SB/Content/Paks/pakchunk0.pak", "game v1");
    fx.write_staged("chunk.pak", "modded");
    fx.write_staged("missing.pak", "mine");
    fx.write_staged("dlssg_sm86.ini", "fg=0\n");
    let backend = HardlinkBackend;
    let plan = fx.plan(
        1,
        &[
            ("chunk.pak", "SB/Content/Paks/pakchunk0.pak"),
            ("missing.pak", "SB/Content/Paks/~mods/missing.pak"),
            ("dlssg_sm86.ini", "SB/Binaries/Win64/dlssg_sm86.ini"),
        ],
    );
    let manifest = backend.deploy(&fx.ctx(), &plan).unwrap();

    // A game update puts its own (new) file back, something deletes a pak
    // and its folder, and an editor saves the settings file as a new file.
    let chunk = fx.game.join("SB/Content/Paks/pakchunk0.pak");
    fs::remove_file(&chunk).unwrap();
    fs::write(&chunk, "game v2, longer").unwrap();
    fs::remove_dir_all(fx.game.join("SB/Content/Paks/~mods")).unwrap();
    let ini = fx.game.join("SB/Binaries/Win64/dlssg_sm86.ini");
    fs::remove_file(&ini).unwrap();
    fs::write(&ini, "fg=1 # mine\n").unwrap();
    assert_eq!(backend.verify(&fx.ctx(), &manifest).unwrap().len(), 3);

    let mut repaired = Vec::new();
    let mut hows = Vec::new();
    for file in &manifest.files {
        let done = sbmm_deploy::repair(&fx.ctx(), &plan, file).unwrap();
        hows.push(done.how);
        repaired.push(done.file);
    }
    use sbmm_deploy::Repair;
    assert_eq!(hows, [Repair::Displaced, Repair::Relinked, Repair::Adopted]);
    let repaired = sbmm_deploy::Manifest {
        files: repaired,
        created_dirs: manifest.created_dirs.clone(),
    };
    assert!(backend.verify(&fx.ctx(), &repaired).unwrap().is_empty());
    assert_eq!(fs::read_to_string(&chunk).unwrap(), "modded");
    assert_eq!(
        fs::read_to_string(fx.staging.join("dlssg_sm86.ini")).unwrap(),
        "fg=1 # mine\n",
        "the edit is now the mod's own"
    );

    // Disabling restores the updated game file, not the stale one.
    backend.undeploy(&fx.ctx(), &repaired).unwrap();
    assert_eq!(fs::read_to_string(&chunk).unwrap(), "game v2, longer");
}

/// A file changed through its link changed the mod too; only the record moves.
#[test]
fn an_edit_through_the_link_is_only_recorded() {
    let fx = Fixture::new();
    fx.write_staged("a.ini", "x=1\n");
    let backend = HardlinkBackend;
    let plan = fx.plan(1, &[("a.ini", "SB/Binaries/Win64/a.ini")]);
    let manifest = backend.deploy(&fx.ctx(), &plan).unwrap();
    // Write the staged copy: through a hard link that is the game's file
    // too, and on a copy-only filesystem the game file is rewritten to match.
    fs::write(fx.staging.join("a.ini"), "x=22\n").unwrap();
    fs::write(fx.game.join("SB/Binaries/Win64/a.ini"), "x=22\n").unwrap();

    let done = sbmm_deploy::repair(&fx.ctx(), &plan, &manifest.files[0]).unwrap();
    assert_eq!(done.how, sbmm_deploy::Repair::Recorded);
    assert_eq!(done.file.size, 5);
}

/// A copied (not linked) file is brought up to date after the mod's copy is
/// edited; a linked one already is.
#[test]
fn refresh_replaces_a_stale_copy() {
    let fx = Fixture::new();
    fx.write_staged("a.ini", "x=1\n");
    let backend = HardlinkBackend;
    let plan = fx.plan(1, &[("a.ini", "SB/Binaries/Win64/a.ini")]);
    let manifest = backend.deploy(&fx.ctx(), &plan).unwrap();
    // As if the library were on another drive: the game has a copy.
    let target = fx.game.join("SB/Binaries/Win64/a.ini");
    fs::remove_file(&target).unwrap();
    fs::write(&target, "x=1\n").unwrap();

    fs::write(fx.staging.join("a.ini"), "x=100\n").unwrap();
    let file = sbmm_deploy::refresh(&fx.ctx(), &plan, &manifest.files[0]).unwrap();
    assert_eq!(fs::read_to_string(&target).unwrap(), "x=100\n");
    assert_eq!(file.size, 6);
}
