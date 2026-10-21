//! A diagnostics bundle the user can attach to a bug report.
//!
//! When something goes wrong the evidence is spread over the library, the
//! game folder and the database, and asking someone to find three log files
//! by path is how bug reports arrive without any of them. One zip, written
//! where the user chooses, holds all of it.

use std::fmt::Write as _;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::dto::DeploymentMode;
use crate::{App, AppError, Result};

/// Logs larger than this are cut to their end, which is where a failure is.
const MAX_LOG_BYTES: u64 = 2 * 1024 * 1024;

/// The frame generation package keeps a log per run; the newest few are
/// enough to see the current problem.
const FRAMEGEN_LOGS: usize = 5;

impl App {
    /// Write a zip of everything needed to diagnose a problem to `dest`.
    ///
    /// It holds a plain-text summary of the setup, the virtual file system's
    /// launch log, hook log and mapping, and the frame generation package's
    /// recent logs. `context` carries what only the shell knows — the
    /// manager's version, the operating system, the graphics card — as
    /// label/value pairs.
    ///
    /// Nothing secret goes in: the Nexus key lives in the OS credential store
    /// and is never read here. Paths are included as they are, so they can
    /// contain the Windows user name; the screen offering this says so.
    pub fn write_diagnostics(&self, dest: &Path, context: &[(String, String)]) -> Result<PathBuf> {
        let file = std::fs::File::create(dest).map_err(|e| AppError::io(dest, e))?;
        let mut zip = zip::ZipWriter::new(file);

        add(
            &mut zip,
            dest,
            "summary.txt",
            self.diagnostics_summary(context)?.as_bytes(),
        )?;

        for name in ["launch.log", "hook.log", "mapping.json"] {
            if let Some(bytes) = read_tail(&self.vfs_dir().join(name)) {
                add(&mut zip, dest, &format!("vfs/{name}"), &bytes)?;
            }
        }

        if let Some(game) = self.game()? {
            let logs = game.binaries_dir().join("dlssg_sm86").join("logs");
            for path in newest_files(&logs, FRAMEGEN_LOGS) {
                let Some(bytes) = read_tail(&path) else {
                    continue;
                };
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                add(&mut zip, dest, &format!("framegen/{name}"), &bytes)?;
            }
        }

        zip.finish().map_err(|e| zip_error(dest, e))?;
        Ok(dest.to_path_buf())
    }

    /// The setup in words, for the top of the bundle.
    fn diagnostics_summary(&self, context: &[(String, String)]) -> Result<String> {
        let snapshot = self.snapshot()?;
        let folders = self.folders()?;
        let mut out = String::from("SB Mod Manager diagnostics\n\n");
        for (label, value) in context {
            let _ = writeln!(out, "{label}: {value}");
        }

        out.push_str("\nGame\n");
        match &snapshot.game {
            Some(game) => {
                let _ = writeln!(out, "  folder: {}", game.root.display());
                let _ = writeln!(out, "  found via: {:?}", game.source);
                let exe = game.executable_path();
                let version = exe
                    .as_deref()
                    .and_then(sbmm_upscaler::pe::file_version_opt)
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "unknown".into());
                let _ = writeln!(
                    out,
                    "  executable: {} (version {version})",
                    game.executable.as_deref().unwrap_or("not found")
                );
            }
            None => out.push_str("  not chosen\n"),
        }

        let mode = match snapshot.deployment_mode {
            DeploymentMode::Hardlink => "hard links",
            DeploymentMode::Virtual => "virtual file system",
        };
        let _ = writeln!(out, "\nDeployment: {mode}");
        let _ = writeln!(out, "Apply automatically: {}", snapshot.auto_apply);
        let _ = writeln!(out, "Library: {}", folders.library);
        let _ = writeln!(
            out,
            "Library on the game's drive: {}",
            folders.same_volume_as_game
        );

        if let Some(framegen) = self.framegen_installed()? {
            let _ = writeln!(
                out,
                "Frame generation package: {} as {}, {}",
                framegen.version,
                framegen.proxy.file_name(),
                if framegen.enabled {
                    "enabled"
                } else {
                    "disabled"
                }
            );
        }

        let enabled = snapshot.mods.iter().filter(|m| m.enabled).count();
        let _ = writeln!(
            out,
            "\nMods ({} installed, {enabled} enabled), in load order",
            snapshot.mods.len()
        );
        let mut mods: Vec<_> = snapshot.mods.iter().collect();
        mods.sort_by_key(|m| (m.priority, m.id));
        for m in mods {
            let _ = writeln!(
                out,
                "  [{}] {:>4}  {}  {}  ({}, from {}){}",
                if m.enabled { "x" } else { " " },
                m.priority,
                m.name,
                m.version.as_deref().unwrap_or("-"),
                m.component_types.join("+"),
                m.source,
                if m.warnings.is_empty() {
                    String::new()
                } else {
                    format!("  warnings: {}", m.warnings.join("; "))
                }
            );
        }

        if !snapshot.pending.is_empty() {
            out.push_str("\nNot yet applied\n");
            for change in &snapshot.pending {
                let _ = writeln!(out, "  {:?}: {}", change.kind, change.name);
            }
        }
        if !snapshot.drift.is_empty() {
            out.push_str("\nChanged outside the manager\n");
            for path in &snapshot.drift {
                let _ = writeln!(out, "  {path}");
            }
        }
        Ok(out)
    }
}

fn add(
    zip: &mut zip::ZipWriter<std::fs::File>,
    dest: &Path,
    name: &str,
    bytes: &[u8],
) -> Result<()> {
    zip.start_file(name, zip::write::SimpleFileOptions::default())
        .map_err(|e| zip_error(dest, e))?;
    zip.write_all(bytes).map_err(|e| AppError::io(dest, e))
}

fn zip_error(dest: &Path, error: zip::result::ZipError) -> AppError {
    AppError::io(dest, std::io::Error::other(error))
}

/// A file's contents, or its last [`MAX_LOG_BYTES`] with a note saying so.
/// `None` when it does not exist or cannot be read: a missing log is part of
/// the picture, not a reason to fail the bundle.
fn read_tail(path: &Path) -> Option<Vec<u8>> {
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let mut out = Vec::new();
    if len > MAX_LOG_BYTES {
        out.extend(format!("[first {} bytes left out]\n", len - MAX_LOG_BYTES).bytes());
        file.seek(SeekFrom::Start(len - MAX_LOG_BYTES)).ok()?;
    }
    file.read_to_end(&mut out).ok()?;
    Some(out)
}

/// The `count` most recently written files directly in `dir`.
fn newest_files(dir: &Path, count: usize) -> Vec<PathBuf> {
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let meta = entry.metadata().ok()?;
            meta.is_file().then(|| {
                (
                    meta.modified().unwrap_or(std::time::UNIX_EPOCH),
                    entry.path(),
                )
            })
        })
        .collect();
    files.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    files.into_iter().take(count).map(|(_, p)| p).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_log_keeps_its_end() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.log");
        let mut text = "x".repeat(MAX_LOG_BYTES as usize);
        text.push_str("THE FAILURE");
        std::fs::write(&path, &text).unwrap();

        let tail = String::from_utf8(read_tail(&path).unwrap()).unwrap();
        assert!(tail.starts_with("[first 11 bytes left out]"));
        assert!(tail.ends_with("THE FAILURE"));
    }

    #[test]
    fn a_missing_log_is_simply_absent() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_tail(&dir.path().join("nope.log")).is_none());
        assert!(newest_files(&dir.path().join("nope"), 5).is_empty());
    }
}
