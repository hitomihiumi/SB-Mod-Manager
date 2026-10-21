//! FOMOD installers and the pictures archives carry.
//!
//! The rules of a FOMOD script live in `sbmm_core::fomod`; this is the part
//! that touches the staged files: finding the script, laying out what the
//! user's answers install, remembering those answers, and handing preview
//! images to the window — only from inside the library, only images, and only
//! up to a size a dialog should ever need.

use std::path::{Path, PathBuf};

use base64::Engine;
use sbmm_core::fomod::{self, Fomod, Selection, WizardState};
use serde::{Deserialize, Serialize};

use crate::dto::FomodView;
use crate::{App, AppError, Result};

/// Image types a preview can be.
const IMAGE_EXTS: &[&str] = &["jpg", "jpeg", "png", "webp", "gif", "bmp"];

/// Larger than any preview a mod author ships, small enough that reading one
/// into the window is never a problem.
const MAX_PREVIEW_BYTES: u64 = 16 * 1024 * 1024;

/// How many of an archive's pictures the install dialog offers.
const MAX_PREVIEWS: usize = 12;

/// The name a FOMOD's chosen picture is kept under in the installed mod.
const KEPT_PREVIEW: &str = "_preview";

/// What is stored on a mod installed through a FOMOD, so its next version
/// can be installed with the same answers.
#[derive(Debug, Serialize, Deserialize)]
struct RememberedFomod {
    fomod: Vec<[String; 3]>,
    /// The installer's own name, which stays put across versions however
    /// the archive and the mod are named.
    #[serde(default)]
    module: Option<String>,
}

/// `fomod/ModuleConfig.xml` in a staged folder: the installer's root (the
/// folder holding `fomod/`, relative and `/`-separated) and the script. The
/// shallowest wins, so an installer bundled inside an option folder of
/// another does not take over.
pub(crate) fn find_fomod(dir: &Path) -> Option<(String, PathBuf)> {
    sbmm_archive::walk_relative(dir)
        .into_iter()
        .filter(|rel| {
            let text = rel.to_string_lossy().replace('\\', "/").to_lowercase();
            text == "fomod/moduleconfig.xml" || text.ends_with("/fomod/moduleconfig.xml")
        })
        .min_by_key(|rel| rel.components().count())
        .map(|rel| {
            let root = rel
                .parent()
                .and_then(Path::parent)
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();
            (root, dir.join(rel))
        })
}

/// Pictures in a staged folder, `/`-separated and relative to it. Ones whose
/// names say they are previews come first, then shallower ones.
pub(crate) fn previews(dir: &Path) -> Vec<String> {
    let mut found: Vec<String> = sbmm_archive::walk_relative(dir)
        .into_iter()
        .filter(|rel| is_image(rel))
        .map(|rel| rel.to_string_lossy().replace('\\', "/"))
        .collect();
    let rank = |path: &String| {
        let lower = path.to_lowercase();
        let named = ["preview", "cover", "thumb", "screenshot", "banner"]
            .iter()
            .any(|word| lower.contains(word));
        (!named, path.matches('/').count(), lower)
    };
    found.sort_by_key(rank);
    found.truncate(MAX_PREVIEWS);
    found
}

fn is_image(path: &Path) -> bool {
    path.extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .is_some_and(|e| IMAGE_EXTS.contains(&e.as_str()))
}

fn to_selection(picked: &[[usize; 3]]) -> Selection {
    picked.iter().map(|[s, g, p]| (*s, *g, *p)).collect()
}

impl App {
    /// Whether a file named by a `fileDependency` is in the game folder.
    fn game_file_check(&self) -> Result<impl Fn(&str) -> bool> {
        let root = self.game()?.map(|g| g.root);
        Ok(move |path: &str| {
            root.as_ref()
                .is_some_and(|root| root.join(path.trim_start_matches('/')).exists())
        })
    }

    /// The FOMOD in a staged folder, and its root on disk.
    fn load_fomod(&self, staging_id: &str) -> Result<(Fomod, PathBuf)> {
        let dir = self.staging_path(staging_id);
        let (root, config) = find_fomod(&dir)
            .ok_or_else(|| AppError::Invalid("this archive has no FOMOD installer".into()))?;
        let bytes = std::fs::read(&config).map_err(|e| AppError::io(&config, e))?;
        let parsed = fomod::parse(&bytes).map_err(|e| AppError::Invalid(e.to_string()))?;
        Ok((parsed, dir.join(root)))
    }

    /// What the install dialog needs to run a staged FOMOD, or `None` when
    /// the archive has none — or has one that cannot be read, which then
    /// installs like any other archive rather than not at all.
    pub(crate) fn fomod_view(
        &self,
        staging_id: &str,
        name: &str,
        nexus_mod_id: Option<i64>,
    ) -> Result<Option<FomodView>> {
        let dir = self.staging_path(staging_id);
        let Some((root, _)) = find_fomod(&dir) else {
            return Ok(None);
        };
        let Ok((parsed, _)) = self.load_fomod(staging_id) else {
            return Ok(None);
        };
        let check = self.game_file_check()?;
        let remembered = self
            .remembered_fomod(name, nexus_mod_id, &parsed.name)?
            .map(|names| fomod::selection_from_names(&parsed, &names))
            .filter(|s| !s.is_empty());
        let selection = remembered
            .clone()
            .unwrap_or_else(|| fomod::defaults(&parsed, &check));
        Ok(Some(FomodView {
            root,
            selection: selection.iter().map(|(s, g, p)| [*s, *g, *p]).collect(),
            remembered: remembered.is_some(),
            installer: parsed,
        }))
    }

    /// The wizard's state for a selection: which steps show, what each
    /// option is, and what stops the install.
    pub fn fomod_state(&self, staging_id: &str, picked: &[[usize; 3]]) -> Result<WizardState> {
        let (parsed, _) = self.load_fomod(staging_id)?;
        let check = self.game_file_check()?;
        Ok(fomod::evaluate(&parsed, &to_selection(picked), &check))
    }

    /// Install a staged FOMOD the way `picked` answers it.
    ///
    /// What the answers install is laid out on its own first, then replaces
    /// the extraction, so detection sees exactly what the user chose and the
    /// library keeps nothing else. The picture of the first chosen option
    /// that has one — or the installer's header — is kept as the mod's.
    pub fn confirm_install_fomod(
        &mut self,
        staging_id: &str,
        name: &str,
        picked: &[[usize; 3]],
    ) -> Result<i64> {
        let (parsed, root) = self.load_fomod(staging_id)?;
        let check = self.game_file_check()?;
        let selection = to_selection(picked);
        let state = fomod::evaluate(&parsed, &selection, &check);
        if let Some(problem) = state.problems.first() {
            return Err(AppError::Invalid(problem.clone()));
        }

        let staging = self.staging_path(staging_id);
        let out = self.staging_path(&format!("{staging_id}.fomod"));
        if out.exists() {
            std::fs::remove_dir_all(&out).map_err(|e| AppError::io(&out, e))?;
        }
        let laid = lay_out(&root, &out, &fomod::resolve(&parsed, &selection, &check));
        if let Err(error) = laid {
            let _ = std::fs::remove_dir_all(&out);
            return Err(error);
        }
        if sbmm_archive::walk_relative(&out).is_empty() {
            let _ = std::fs::remove_dir_all(&out);
            return Err(AppError::Invalid(
                "the chosen options install no files".into(),
            ));
        }

        let picture = selection
            .iter()
            .filter(|(s, _, _)| state.visible.get(*s).copied().unwrap_or(false))
            .find_map(|(s, g, p)| parsed.steps[*s].groups[*g].plugins[*p].image.clone())
            .or_else(|| parsed.image.clone());
        if let Some(picture) = picture {
            let from = root.join(&picture);
            let ext = Path::new(&picture)
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_else(|| "png".into());
            // A missing picture is the author's slip, not a reason to fail.
            let _ = std::fs::copy(&from, out.join(format!("{KEPT_PREVIEW}.{ext}")));
        }

        std::fs::remove_dir_all(&staging).map_err(|e| AppError::io(&staging, e))?;
        std::fs::rename(&out, &staging).map_err(|e| AppError::io(&staging, e))?;

        let origin = self
            .pending_origins
            .remove(staging_id)
            .unwrap_or_else(crate::Origin::manual);
        let id = self.commit_install(staging_id, name, None, origin)?;
        let remembered = RememberedFomod {
            fomod: fomod::selection_names(&parsed, &selection),
            module: Some(parsed.name.clone()).filter(|n| !n.is_empty()),
        };
        self.store
            .set_variant_choice(id, Some(&serde_json::to_string(&remembered)?))?;
        Ok(id)
    }

    /// The FOMOD answers this mod was installed with last time.
    pub(crate) fn remembered_fomod(
        &self,
        name: &str,
        nexus_mod_id: Option<i64>,
        module: &str,
    ) -> Result<Option<Vec<[String; 3]>>> {
        let records = self.store.list_mods()?;
        let parse = |m: &sbmm_store::ModRecord| {
            m.variant_choice
                .as_deref()
                .and_then(|json| serde_json::from_str::<RememberedFomod>(json).ok())
        };
        let found = nexus_mod_id
            .and_then(|id| {
                records
                    .iter()
                    .find(|m| m.nexus_mod_id == Some(id) && parse(m).is_some())
            })
            .or_else(|| {
                // The installer's own name first: it is the same in every
                // version, where the mod's name is whatever the user typed.
                records.iter().find(|m| {
                    !module.is_empty()
                        && parse(m)
                            .and_then(|r| r.module)
                            .is_some_and(|n| n.eq_ignore_ascii_case(module))
                })
            })
            .or_else(|| {
                records
                    .iter()
                    .find(|m| m.name.eq_ignore_ascii_case(name.trim()) && parse(m).is_some())
            });
        Ok(found.and_then(parse).map(|r| r.fomod))
    }

    /// Install a staged FOMOD with remembered answers, if they still make a
    /// valid install of this version of it.
    pub(crate) fn fomod_with_remembered(
        &mut self,
        staging_id: &str,
        name: &str,
        nexus_mod_id: Option<i64>,
    ) -> Result<Option<i64>> {
        let (parsed, _) = self.load_fomod(staging_id)?;
        let Some(names) = self.remembered_fomod(name, nexus_mod_id, &parsed.name)? else {
            return Ok(None);
        };
        let selection = fomod::selection_from_names(&parsed, &names);
        let check = self.game_file_check()?;
        if selection.is_empty()
            || !fomod::evaluate(&parsed, &selection, &check)
                .problems
                .is_empty()
        {
            return Ok(None);
        }
        let picked: Vec<[usize; 3]> = selection.iter().map(|(s, g, p)| [*s, *g, *p]).collect();
        self.confirm_install_fomod(staging_id, name, &picked)
            .map(Some)
    }

    /// A picture from a staged install or an installed mod, as a `data:` URL
    /// the window can show without being given the file system.
    ///
    /// `folder` is a staging id or a mod's staging folder and `path` is
    /// relative to it; anything that leaves the library, is not an image, or
    /// is larger than a preview has any business being is refused.
    pub fn preview_data_url(&self, folder: &str, path: &str) -> Result<String> {
        let refuse = |why: &str| AppError::Invalid(format!("cannot show {path}: {why}"));
        if folder.contains(['/', '\\']) || folder.starts_with('.') {
            return Err(refuse("not a library folder"));
        }
        let base = self.staging_path(folder);
        let full = sbmm_deploy::backend::resolve_target(&base, Path::new(path))
            .map_err(|_| refuse("outside the library"))?;
        if !is_image(&full) {
            return Err(refuse("not an image"));
        }
        let size = std::fs::metadata(&full)
            .map_err(|e| AppError::io(&full, e))?
            .len();
        if size > MAX_PREVIEW_BYTES {
            return Err(refuse("too large to preview"));
        }
        let bytes = std::fs::read(&full).map_err(|e| AppError::io(&full, e))?;
        let ext = full
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let mime = match ext.as_str() {
            "jpg" | "jpeg" => "image/jpeg",
            "png" => "image/png",
            "webp" => "image/webp",
            "gif" => "image/gif",
            _ => "image/bmp",
        };
        Ok(format!(
            "data:{mime};base64,{}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        ))
    }
}

/// Copy what a FOMOD installs from its root into `out`, in priority order so
/// later items overwrite earlier ones as the script intends. Every path is
/// checked to stay inside its folder: the script is as untrusted as the
/// archive it came in.
fn lay_out(root: &Path, out: &Path, items: &[fomod::FileItem]) -> Result<()> {
    let inside = |base: &Path, rel: &str| {
        sbmm_deploy::backend::resolve_target(base, Path::new(rel)).map_err(|_| {
            AppError::Invalid(format!(
                "the installer names a path outside its folder: {rel}"
            ))
        })
    };
    let copy = |from: &Path, to: &Path| -> Result<()> {
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(|e| AppError::io(parent, e))?;
        }
        std::fs::copy(from, to).map_err(|e| AppError::io(to, e))?;
        Ok(())
    };

    for item in items {
        let source = inside(root, &item.source)?;
        let destination = fomod::destination_of(item);
        if item.is_folder {
            if !source.is_dir() {
                return Err(AppError::Invalid(format!(
                    "the installer lists the folder {}, which is not in the archive",
                    item.source
                )));
            }
            for rel in sbmm_archive::walk_relative(&source) {
                let rel_text = rel.to_string_lossy().replace('\\', "/");
                let target = if destination.is_empty() {
                    rel_text
                } else {
                    format!("{destination}/{rel_text}")
                };
                copy(&source.join(&rel), &inside(out, &target)?)?;
            }
        } else {
            if !source.is_file() {
                return Err(AppError::Invalid(format!(
                    "the installer lists {}, which is not in the archive",
                    item.source
                )));
            }
            copy(&source, &inside(out, &destination)?)?;
        }
    }
    Ok(())
}
