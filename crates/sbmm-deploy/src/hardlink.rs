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
