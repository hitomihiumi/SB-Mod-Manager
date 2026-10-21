use std::fs;
use std::path::{Path, PathBuf};

use sbmm_core::plan::DeployPlan;

use crate::backend::{resolve_target, DeployBackend, DeployContext, DeployError, Drift, DriftKind};
use crate::manifest::{DeployedFile, Manifest};

/// Deploys by hard-linking staged files into the game folder.
///
/// Hard links cost no extra disk space and are indistinguishable from real
/// files to the game, which matters because Stellar Blade mounts paks through
/// ordinary file APIs. When the staging folder and the game live on different
/// volumes the link is not possible and we fall back to copying.
#[derive(Debug, Default, Clone, Copy)]
pub struct HardlinkBackend;

impl DeployBackend for HardlinkBackend {
    fn name(&self) -> &'static str {
        "hardlink"
    }

    fn deploy(&self, ctx: &DeployContext, plan: &DeployPlan) -> Result<Manifest, DeployError> {
        let mut manifest = Manifest::default();
        // Deploying half a mod is worse than deploying none of it, so any
        // failure unwinds the files this pass already wrote.
        match deploy_all(ctx, plan, &mut manifest) {
            Ok(()) => Ok(manifest),
            Err(err) => {
                if let Err(cleanup_err) = self.undeploy(ctx, &manifest) {
                    tracing::error!(
                        error = %cleanup_err,
                        "failed to roll back a partial deployment"
                    );
                }
                Err(err)
            }
        }
    }

    fn undeploy(&self, ctx: &DeployContext, manifest: &Manifest) -> Result<(), DeployError> {
        // Reverse order so restores land after the files that displaced them
        // have been cleared out.
        for file in manifest.files.iter().rev() {
            let target = resolve_target(&ctx.game_root, &file.target)?;
            match fs::remove_file(&target) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(DeployError::io(&target, e)),
            }

            if let Some(backup_rel) = &file.backup {
                let backup = resolve_target(&ctx.backup_root, backup_rel)?;
                if backup.exists() {
                    create_parents(&target)?;
                    move_file(&backup, &target)?;
                }
            }
        }

        prune_dirs(&ctx.game_root, &manifest.created_dirs);
        Ok(())
    }

    fn verify(&self, ctx: &DeployContext, manifest: &Manifest) -> Result<Vec<Drift>, DeployError> {
        let mut drifts = Vec::new();
        for file in &manifest.files {
            let target = resolve_target(&ctx.game_root, &file.target)?;
            match fs::metadata(&target) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => drifts.push(Drift {
                    target: file.target.clone(),
                    kind: DriftKind::Missing,
                }),
                Err(e) => return Err(DeployError::io(&target, e)),
                Ok(meta) if meta.len() != file.size => drifts.push(Drift {
                    target: file.target.clone(),
                    kind: DriftKind::Modified,
                }),
                Ok(_) => {}
            }
        }
        Ok(drifts)
    }
}

/// How [`repair`] put a drifted file right.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Repair {
    /// It was gone and is linked in again.
    Relinked,
    /// It was changed through the link, so the mod's own copy changed with
    /// it; only the record was out of date.
    Recorded,
    /// A settings file edited in place of the link: the edit is kept, by
    /// becoming the mod's copy.
    Adopted,
    /// Something else was put there — usually a game update restoring its
    /// own file. It is kept as the file to put back when the mod is
    /// disabled, and the mod's file goes back in.
    Displaced,
}

/// What [`repair`] did to one file.
#[derive(Debug, Clone)]
pub struct Repaired {
    /// The file's new record.
    pub file: DeployedFile,
    pub how: Repair,
    /// Folders made to put a removed file back, relative to the game root.
    pub created_dirs: Vec<PathBuf>,
}

/// Put one drifted file of a mod back the way the mod has it.
///
/// Nothing found in the game folder is thrown away: a file that is not the
/// mod's either joins the mod (settings edits) or becomes the backup that
/// disabling the mod restores, replacing an older backup — after a game
/// update the new game file is the one to restore, not the old.
pub fn repair(
    ctx: &DeployContext,
    plan: &DeployPlan,
    file: &DeployedFile,
) -> Result<Repaired, DeployError> {
    let planned = plan
        .files
        .iter()
        .find(|p| p.target == file.target)
        .ok_or_else(|| DeployError::MissingSource(file.target.clone()))?;
    let source = plan.staging_root.join(&planned.source);
    if !source.is_file() {
        return Err(DeployError::MissingSource(planned.source.clone()));
    }
    let target = resolve_target(&ctx.game_root, &file.target)?;

    let mut repaired = file.clone();
    let mut manifest = Manifest::default();
    let how = if fs::symlink_metadata(&target).is_err() {
        record_created_dirs(&ctx.game_root, &target, &mut manifest)?;
        link_or_copy(&source, &target)?;
        Repair::Relinked
    } else if same_contents(&source, &target)? {
        Repair::Recorded
    } else if is_settings_file(&file.target) {
        // The edit wins: it becomes the mod's file, then the link is restored.
        fs::copy(&target, &source).map_err(|e| DeployError::io(&source, e))?;
        fs::remove_file(&target).map_err(|e| DeployError::io(&target, e))?;
        link_or_copy(&source, &target)?;
        Repair::Adopted
    } else {
        let backup = resolve_target(&ctx.backup_root, &file.target)?;
        create_parents(&backup)?;
        match fs::remove_file(&backup) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(DeployError::io(&backup, e)),
        }
        move_file(&target, &backup)?;
        link_or_copy(&source, &target)?;
        repaired.backup = Some(file.target.clone());
        Repair::Displaced
    };
    repaired.size = fs::metadata(&source)
        .map_err(|e| DeployError::io(&source, e))?
        .len();
    Ok(Repaired {
        file: repaired,
        how,
        created_dirs: manifest.created_dirs,
    })
}

/// After the mod's own copy of a deployed file was edited, make the game's
/// copy match and return its new record. Through a hard link it already
/// does; a copied file (the library is on another drive) is replaced.
pub fn refresh(
    ctx: &DeployContext,
    plan: &DeployPlan,
    file: &DeployedFile,
) -> Result<DeployedFile, DeployError> {
    let planned = plan
        .files
        .iter()
        .find(|p| p.target == file.target)
        .ok_or_else(|| DeployError::MissingSource(file.target.clone()))?;
    let source = plan.staging_root.join(&planned.source);
    let target = resolve_target(&ctx.game_root, &file.target)?;
    if !target.exists() || !same_contents(&source, &target)? {
        match fs::remove_file(&target) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(DeployError::io(&target, e)),
        }
        create_parents(&target)?;
        link_or_copy(&source, &target)?;
    }
    let mut refreshed = file.clone();
    refreshed.size = fs::metadata(&source)
        .map_err(|e| DeployError::io(&source, e))?
        .len();
    Ok(refreshed)
}

/// Text settings a user may well have opened and saved in the game folder.
fn is_settings_file(path: &Path) -> bool {
    path.extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .is_some_and(|e| {
            matches!(
                e.as_str(),
                "ini" | "cfg" | "txt" | "json" | "toml" | "yaml" | "yml" | "xml"
            )
        })
}

fn same_contents(a: &Path, b: &Path) -> Result<bool, DeployError> {
    use std::io::Read;
    let open = |p: &Path| fs::File::open(p).map_err(|e| DeployError::io(p, e));
    let (mut fa, mut fb) = (open(a)?, open(b)?);
    let len = |f: &fs::File, p: &Path| {
        f.metadata()
            .map(|m| m.len())
            .map_err(|e| DeployError::io(p, e))
    };
    if len(&fa, a)? != len(&fb, b)? {
        return Ok(false);
    }
    let (mut ba, mut bb) = (vec![0u8; 1 << 16], vec![0u8; 1 << 16]);
    loop {
        let n = fa.read(&mut ba).map_err(|e| DeployError::io(a, e))?;
        if n == 0 {
            return Ok(true);
        }
        fb.read_exact(&mut bb[..n])
            .map_err(|e| DeployError::io(b, e))?;
        if ba[..n] != bb[..n] {
            return Ok(false);
        }
    }
}

fn deploy_all(
    ctx: &DeployContext,
    plan: &DeployPlan,
    manifest: &mut Manifest,
) -> Result<(), DeployError> {
    for file in &plan.files {
        let source = plan.staging_root.join(&file.source);
        let meta =
            fs::metadata(&source).map_err(|_| DeployError::MissingSource(file.source.clone()))?;
        if !meta.is_file() {
            continue;
        }

        let target = resolve_target(&ctx.game_root, &file.target)?;
        record_created_dirs(&ctx.game_root, &target, manifest)?;

        // Anything already sitting at the destination is preserved so the
        // original install can be restored exactly.
        let backup = if fs::symlink_metadata(&target).is_ok() {
            let backup_path = resolve_target(&ctx.backup_root, &file.target)?;
            create_parents(&backup_path)?;
            move_file(&target, &backup_path)?;
            Some(file.target.clone())
        } else {
            None
        };

        link_or_copy(&source, &target)?;

        manifest.files.push(DeployedFile {
            target: file.target.clone(),
            mod_id: plan.mod_id,
            backup,
            size: meta.len(),
        });
    }
    Ok(())
}

/// Create every missing directory above `target`, remembering which ones we
/// made so they can be pruned again on removal.
fn record_created_dirs(
    game_root: &Path,
    target: &Path,
    manifest: &mut Manifest,
) -> Result<(), DeployError> {
    let Some(parent) = target.parent() else {
        return Ok(());
    };

    let mut missing = Vec::new();
    let mut cursor = parent;
    while cursor.starts_with(game_root) && cursor != game_root {
        if cursor.exists() {
            break;
        }
        missing.push(cursor.to_path_buf());
        match cursor.parent() {
            Some(p) => cursor = p,
            None => break,
        }
    }

    // Shallowest first, so each mkdir has its parent in place.
    for dir in missing.iter().rev() {
        fs::create_dir(dir).or_else(|e| match e.kind() {
            std::io::ErrorKind::AlreadyExists => Ok(()),
            _ => Err(DeployError::io(dir, e)),
        })?;
        if let Ok(rel) = dir.strip_prefix(game_root) {
            let rel = rel.to_path_buf();
            if !manifest.created_dirs.contains(&rel) {
                manifest.created_dirs.push(rel);
            }
        }
    }
    Ok(())
}

/// Remove directories we created, deepest first, and only while empty.
///
/// A directory the user dropped their own files into simply stays.
pub fn prune_dirs(game_root: &Path, dirs: &[PathBuf]) {
    let mut sorted: Vec<&PathBuf> = dirs.iter().collect();
    sorted.sort_by_key(|d| std::cmp::Reverse(d.components().count()));
    for dir in sorted {
        let full = game_root.join(dir);
        let _ = fs::remove_dir(&full);
    }
}

fn create_parents(path: &Path) -> Result<(), DeployError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| DeployError::io(parent, e))?;
    }
    Ok(())
}

/// Rename where possible, copy across volumes.
fn move_file(from: &Path, to: &Path) -> Result<(), DeployError> {
    match fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(_) => {
            fs::copy(from, to).map_err(|e| DeployError::io(to, e))?;
            fs::remove_file(from).map_err(|e| DeployError::io(from, e))?;
            Ok(())
        }
    }
}

fn link_or_copy(source: &Path, target: &Path) -> Result<(), DeployError> {
    match fs::hard_link(source, target) {
        Ok(()) => Ok(()),
        // Cross-volume staging, or a filesystem without link support.
        Err(_) => {
            fs::copy(source, target).map_err(|e| DeployError::io(target, e))?;
            Ok(())
        }
    }
}
