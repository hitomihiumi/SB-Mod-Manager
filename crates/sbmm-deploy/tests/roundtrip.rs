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
            let rel = path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
            out.insert(format!("{rel}/"), Vec::new());
            walk(root, &path, out);
        } else {
            let rel = path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
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
    let plan = fx.plan(1, &[("Outfit_P.pak", "SB/Content/Paks/~mods/0100_Outfit_P.pak")]);
    let manifest = backend.deploy(&fx.ctx(), &plan).unwrap();

    assert!(fx.game.join("SB/Content/Paks/~mods/0100_Outfit_P.pak").exists());
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
    assert_eq!(fs::read_to_string(fx.game.join(target)).unwrap(), "modded movie");
    assert!(manifest.files[0].backup.is_some(), "the original must be preserved");

    backend.undeploy(&fx.ctx(), &manifest).unwrap();
    assert_eq!(fs::read_to_string(fx.game.join(target)).unwrap(), "original movie");
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
            .deploy(&fx.ctx(), &fx.plan(1, &[("a.pak", "SB/Content/Paks/~mods/a.pak")]))
            .unwrap(),
    );
    global.merge(
        backend
            .deploy(&fx.ctx(), &fx.plan(2, &[("b.pak", "SB/Content/Paks/~mods/b.pak")]))
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
        .deploy(&fx.ctx(), &fx.plan(1, &[("a.pak", "SB/Content/Paks/~mods/a.pak")]))
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
