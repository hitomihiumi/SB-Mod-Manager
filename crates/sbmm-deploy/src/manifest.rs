use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// One file the manager put into the game folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployedFile {
    /// Path relative to the game root.
    pub target: PathBuf,
    /// Which mod owns this file.
    pub mod_id: i64,
    /// Where the pre-existing file was moved to, if this deployment displaced
    /// a real game file. Relative to the backup root.
    pub backup: Option<PathBuf>,
    /// Size at deploy time, used by `verify` to spot outside edits.
    pub size: u64,
}

/// The complete record of one deployment pass.
///
/// This is the only thing `undeploy` trusts. It is persisted so that a crash,
/// or the app being reinstalled, still leaves the game folder recoverable.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub files: Vec<DeployedFile>,
    /// Directories we created, relative to the game root. Recorded deepest
    /// last so removal can walk the list in reverse.
    pub created_dirs: Vec<PathBuf>,
}

impl Manifest {
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    pub fn merge(&mut self, other: Manifest) {
        self.files.extend(other.files);
        for dir in other.created_dirs {
            if !self.created_dirs.contains(&dir) {
                self.created_dirs.push(dir);
            }
        }
    }

    /// Split out the entries belonging to one mod, leaving the rest in place.
    pub fn take_mod(&mut self, mod_id: i64) -> Manifest {
        let (mine, theirs): (Vec<_>, Vec<_>) =
            self.files.drain(..).partition(|f| f.mod_id == mod_id);
        self.files = theirs;
        Manifest {
            files: mine,
            // Directory pruning is best-effort and only removes empty dirs, so
            // it is safe to offer the full list to each removal.
            created_dirs: self.created_dirs.clone(),
        }
    }
}
