//! Persistence for installed mods, groups, load order and the deployment
//! record.
//!
//! Enabled state and priority are stored per profile rather than on the mod
//! itself, so profile switching can be added without touching existing data.

pub mod schema;

use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};
use sbmm_core::model::DetectedComponent;
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("could not encode mod components: {0}")]
    Encode(#[from] serde_json::Error),
    #[error("no mod with id {0}")]
    UnknownMod(i64),
}

type Result<T> = std::result::Result<T, StoreError>;

/// One recorded deployed file: `(mod_id, target, backup, size)`.
pub type DeployedRow = (i64, String, Option<String>, i64);

/// A deployed file without its owning mod: `(target, backup, size)`.
pub type DeployedFileRow = (String, Option<String>, i64);

/// An installed mod as the UI sees it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModRecord {
    pub id: i64,
    pub name: String,
    pub staging_folder: String,
    pub version: Option<String>,
    pub source: String,
    pub nexus_mod_id: Option<i64>,
    pub nexus_file_id: Option<i64>,
    pub group_id: Option<i64>,
    pub primary_type: String,
    pub size_bytes: i64,
    pub image_path: Option<String>,
    pub notes: Option<String>,
    pub installed_at: String,
    pub enabled: bool,
    pub priority: i64,
    /// The newest version Nexus reported, from the last update check.
    pub latest_version: Option<String>,
    pub update_checked_at: Option<String>,
}

/// A new mod being registered after a successful install.
#[derive(Debug, Clone)]
pub struct NewMod {
    pub name: String,
    pub staging_folder: String,
    pub version: Option<String>,
    pub source: String,
    pub nexus_mod_id: Option<i64>,
    pub nexus_file_id: Option<i64>,
    pub primary_type: String,
    pub components: Vec<DetectedComponent>,
    pub size_bytes: i64,
    pub image_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupRecord {
    pub id: i64,
    pub name: String,
    pub color: Option<String>,
    pub sort_index: i64,
    pub collapsed: bool,
}

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let conn = Connection::open(path)?;
        schema::migrate(&conn)?;
        Ok(Self { conn })
    }

    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        schema::migrate(&conn)?;
        Ok(Self { conn })
    }

    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    // -- settings ----------------------------------------------------------

    pub fn get_setting(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
                r.get(0)
            })
            .optional()?)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    // -- profiles ----------------------------------------------------------

    pub fn active_profile(&self) -> Result<i64> {
        Ok(self.conn.query_row(
            "SELECT id FROM profiles WHERE active = 1 ORDER BY id LIMIT 1",
            [],
            |r| r.get(0),
        )?)
    }

    // -- mods --------------------------------------------------------------

    pub fn insert_mod(&self, new: &NewMod) -> Result<i64> {
        let components = serde_json::to_string(&new.components)?;
        self.conn.execute(
            "INSERT INTO mods
                (name, staging_folder, version, source, nexus_mod_id, nexus_file_id,
                 primary_type, components, size_bytes, image_path)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                new.name,
                new.staging_folder,
                new.version,
                new.source,
                new.nexus_mod_id,
                new.nexus_file_id,
                new.primary_type,
                components,
                new.size_bytes,
                new.image_path,
            ],
        )?;
        let id = self.conn.last_insert_rowid();

        // New mods start disabled at the end of the load order, so installing
        // never changes what the game currently loads.
        let profile = self.active_profile()?;
        let next_priority: i64 = self.conn.query_row(
            "SELECT COALESCE(MAX(priority), 0) + 10 FROM profile_mods WHERE profile_id = ?1",
            [profile],
            |r| r.get(0),
        )?;
        self.conn.execute(
            "INSERT INTO profile_mods (profile_id, mod_id, enabled, priority)
             VALUES (?1, ?2, 0, ?3)",
            params![profile, id, next_priority],
        )?;
        Ok(id)
    }

    pub fn list_mods(&self) -> Result<Vec<ModRecord>> {
        let profile = self.active_profile()?;
        let mut stmt = self.conn.prepare(
            "SELECT m.id, m.name, m.staging_folder, m.version, m.source,
                    m.nexus_mod_id, m.nexus_file_id, m.group_id, m.primary_type,
                    m.size_bytes, m.image_path, m.notes, m.installed_at,
                    COALESCE(pm.enabled, 0), COALESCE(pm.priority, 0),
                    m.latest_version, m.update_checked_at
             FROM mods m
             LEFT JOIN profile_mods pm
                    ON pm.mod_id = m.id AND pm.profile_id = ?1
             ORDER BY pm.priority, m.name COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([profile], |row| {
            Ok(ModRecord {
                id: row.get(0)?,
                name: row.get(1)?,
                staging_folder: row.get(2)?,
                version: row.get(3)?,
                source: row.get(4)?,
                nexus_mod_id: row.get(5)?,
                nexus_file_id: row.get(6)?,
                group_id: row.get(7)?,
                primary_type: row.get(8)?,
                size_bytes: row.get(9)?,
                image_path: row.get(10)?,
                notes: row.get(11)?,
                installed_at: row.get(12)?,
                enabled: row.get::<_, i64>(13)? != 0,
                priority: row.get(14)?,
                latest_version: row.get(15)?,
                update_checked_at: row.get(16)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Record what a Nexus update check found for one mod.
    ///
    /// The timestamp is written even when the version is unchanged, so a mod
    /// that has simply not been looked at yet is distinguishable from one that
    /// is known to be current.
    pub fn record_update_check(&self, mod_id: i64, latest_version: Option<&str>) -> Result<()> {
        self.conn.execute(
            "UPDATE mods
                SET latest_version = ?2, update_checked_at = datetime('now')
              WHERE id = ?1",
            params![mod_id, latest_version],
        )?;
        Ok(())
    }

    pub fn components_for(&self, mod_id: i64) -> Result<Vec<DetectedComponent>> {
        let json: String = self
            .conn
            .query_row("SELECT components FROM mods WHERE id = ?1", [mod_id], |r| {
                r.get(0)
            })
            .optional()?
            .ok_or(StoreError::UnknownMod(mod_id))?;
        Ok(serde_json::from_str(&json)?)
    }

    pub fn set_components(&self, mod_id: i64, components: &[DetectedComponent]) -> Result<()> {
        let json = serde_json::to_string(components)?;
        let primary = components
            .first()
            .map(|c| c.mod_type.as_str())
            .unwrap_or("unknown");
        self.conn.execute(
            "UPDATE mods SET components = ?2, primary_type = ?3 WHERE id = ?1",
            params![mod_id, json, primary],
        )?;
        Ok(())
    }

    pub fn delete_mod(&self, mod_id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM mods WHERE id = ?1", [mod_id])?;
        self.conn
            .execute("DELETE FROM deployed_files WHERE mod_id = ?1", [mod_id])?;
        Ok(())
    }

    pub fn rename_mod(&self, mod_id: i64, name: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE mods SET name = ?2 WHERE id = ?1",
            params![mod_id, name],
        )?;
        Ok(())
    }

    // -- enabled state and load order --------------------------------------

    /// Flip a batch of mods in one transaction.
    ///
    /// Bulk toggles — a whole group, or everything — go through here so the
    /// UI never leaves the database half-updated.
    pub fn set_enabled(&mut self, mod_ids: &[i64], enabled: bool) -> Result<()> {
        let profile = self.active_profile()?;
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO profile_mods (profile_id, mod_id, enabled, priority)
                 VALUES (?1, ?2, ?3, 0)
                 ON CONFLICT(profile_id, mod_id) DO UPDATE SET enabled = excluded.enabled",
            )?;
            for id in mod_ids {
                stmt.execute(params![profile, id, enabled as i64])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn enabled_mod_ids(&self) -> Result<Vec<i64>> {
        let profile = self.active_profile()?;
        let mut stmt = self.conn.prepare(
            "SELECT mod_id FROM profile_mods
             WHERE profile_id = ?1 AND enabled = 1
             ORDER BY priority",
        )?;
        let rows = stmt.query_map([profile], |r| r.get(0))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Rewrite the load order from an explicit list of mod ids.
    pub fn set_order(&mut self, ordered_ids: &[i64]) -> Result<()> {
        let profile = self.active_profile()?;
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO profile_mods (profile_id, mod_id, enabled, priority)
                 VALUES (?1, ?2, 0, ?3)
                 ON CONFLICT(profile_id, mod_id) DO UPDATE SET priority = excluded.priority",
            )?;
            // Leave gaps so a single insert does not require renumbering.
            for (index, id) in ordered_ids.iter().enumerate() {
                stmt.execute(params![profile, id, (index as i64 + 1) * 10])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    // -- groups ------------------------------------------------------------

    pub fn list_groups(&self) -> Result<Vec<GroupRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, color, sort_index, collapsed
             FROM groups ORDER BY sort_index, name COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(GroupRecord {
                id: row.get(0)?,
                name: row.get(1)?,
                color: row.get(2)?,
                sort_index: row.get(3)?,
                collapsed: row.get::<_, i64>(4)? != 0,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn create_group(&self, name: &str, color: Option<&str>) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO groups (name, color, sort_index)
             VALUES (?1, ?2, (SELECT COALESCE(MAX(sort_index), 0) + 1 FROM groups))",
            params![name, color],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn delete_group(&self, group_id: i64) -> Result<()> {
        // Mods fall back to ungrouped rather than being deleted with the group.
        self.conn
            .execute("DELETE FROM groups WHERE id = ?1", [group_id])?;
        Ok(())
    }

    pub fn set_group_collapsed(&self, group_id: i64, collapsed: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE groups SET collapsed = ?2 WHERE id = ?1",
            params![group_id, collapsed as i64],
        )?;
        Ok(())
    }

    pub fn assign_group(&mut self, mod_ids: &[i64], group_id: Option<i64>) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare("UPDATE mods SET group_id = ?2 WHERE id = ?1")?;
            for id in mod_ids {
                stmt.execute(params![id, group_id])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    // -- deployment record -------------------------------------------------

    /// Replace the recorded deployment for one mod.
    pub fn replace_deployment(&mut self, mod_id: i64, files: &[DeployedFileRow]) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM deployed_files WHERE mod_id = ?1", [mod_id])?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO deployed_files (mod_id, target, backup, size)
                 VALUES (?1, ?2, ?3, ?4)",
            )?;
            for (target, backup, size) in files {
                stmt.execute(params![mod_id, target, backup, size])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn clear_deployment(&self, mod_id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM deployed_files WHERE mod_id = ?1", [mod_id])?;
        Ok(())
    }

    /// Every recorded file, as `(mod_id, target, backup, size)`.
    pub fn deployed_files(&self) -> Result<Vec<DeployedRow>> {
        let mut stmt = self
            .conn
            .prepare("SELECT mod_id, target, backup, size FROM deployed_files ORDER BY id")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn deployed_files_for(&self, mod_id: i64) -> Result<Vec<DeployedFileRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT target, backup, size FROM deployed_files WHERE mod_id = ?1 ORDER BY id",
        )?;
        let rows = stmt.query_map([mod_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn record_created_dirs(&mut self, dirs: &[String]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare("INSERT OR IGNORE INTO created_dirs (path) VALUES (?1)")?;
            for dir in dirs {
                stmt.execute([dir])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn created_dirs(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT path FROM created_dirs")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn forget_created_dirs(&self, dirs: &[String]) -> Result<()> {
        for dir in dirs {
            self.conn
                .execute("DELETE FROM created_dirs WHERE path = ?1", [dir])?;
        }
        Ok(())
    }

    // -- learned detection overrides ---------------------------------------

    pub fn remember_detection(&self, pattern: &str, mod_type: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO detection_rules (pattern, mod_type) VALUES (?1, ?2)
             ON CONFLICT(pattern) DO UPDATE SET mod_type = excluded.mod_type",
            params![pattern, mod_type],
        )?;
        Ok(())
    }

    pub fn recall_detection(&self, pattern: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT mod_type FROM detection_rules WHERE pattern = ?1",
                [pattern],
                |r| r.get(0),
            )
            .optional()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(name: &str) -> NewMod {
        NewMod {
            name: name.to_string(),
            staging_folder: name.to_lowercase(),
            version: Some("1.0".into()),
            source: "manual".into(),
            nexus_mod_id: None,
            nexus_file_id: None,
            primary_type: "genericPak".into(),
            components: Vec::new(),
            size_bytes: 42,
            image_path: None,
        }
    }

    #[test]
    fn a_new_mod_starts_disabled_at_the_end_of_the_order() {
        let store = Store::open_in_memory().unwrap();
        let first = store.insert_mod(&sample("Alpha")).unwrap();
        let second = store.insert_mod(&sample("Beta")).unwrap();

        let mods = store.list_mods().unwrap();
        assert_eq!(mods.len(), 2);
        assert!(
            mods.iter().all(|m| !m.enabled),
            "installing must not change what loads"
        );

        let alpha = mods.iter().find(|m| m.id == first).unwrap();
        let beta = mods.iter().find(|m| m.id == second).unwrap();
        assert!(beta.priority > alpha.priority);
    }

    #[test]
    fn bulk_toggles_apply_to_every_selected_mod() {
        let mut store = Store::open_in_memory().unwrap();
        let ids: Vec<i64> = ["A", "B", "C"]
            .iter()
            .map(|n| store.insert_mod(&sample(n)).unwrap())
            .collect();

        store.set_enabled(&ids, true).unwrap();
        assert_eq!(store.enabled_mod_ids().unwrap().len(), 3);

        store.set_enabled(&ids[..2], false).unwrap();
        assert_eq!(store.enabled_mod_ids().unwrap(), vec![ids[2]]);
    }

    #[test]
    fn reordering_rewrites_priorities_with_gaps() {
        let mut store = Store::open_in_memory().unwrap();
        let a = store.insert_mod(&sample("A")).unwrap();
        let b = store.insert_mod(&sample("B")).unwrap();

        store.set_order(&[b, a]).unwrap();
        let mods = store.list_mods().unwrap();
        assert_eq!(mods[0].id, b, "list_mods orders by priority");
        assert_eq!(mods[0].priority, 10);
        assert_eq!(mods[1].priority, 20);
    }

    #[test]
    fn deleting_a_group_leaves_its_mods_alone() {
        let mut store = Store::open_in_memory().unwrap();
        let id = store.insert_mod(&sample("A")).unwrap();
        let group = store.create_group("Outfits", Some("#c33")).unwrap();
        store.assign_group(&[id], Some(group)).unwrap();

        store.delete_group(group).unwrap();

        let mods = store.list_mods().unwrap();
        assert_eq!(mods.len(), 1);
        assert_eq!(mods[0].group_id, None);
    }

    #[test]
    fn the_deployment_record_round_trips() {
        let mut store = Store::open_in_memory().unwrap();
        let id = store.insert_mod(&sample("A")).unwrap();

        store
            .replace_deployment(id, &[("SB/Content/Paks/~mods/a.pak".into(), None, 12)])
            .unwrap();
        assert_eq!(store.deployed_files_for(id).unwrap().len(), 1);

        store.clear_deployment(id).unwrap();
        assert!(store.deployed_files_for(id).unwrap().is_empty());
    }

    #[test]
    fn deleting_a_mod_clears_its_deployment_record() {
        let mut store = Store::open_in_memory().unwrap();
        let id = store.insert_mod(&sample("A")).unwrap();
        store
            .replace_deployment(id, &[("x".into(), None, 1)])
            .unwrap();

        store.delete_mod(id).unwrap();
        assert!(store.deployed_files().unwrap().is_empty());
    }

    #[test]
    fn settings_and_learned_rules_persist() {
        let store = Store::open_in_memory().unwrap();
        store
            .set_setting("gameRoot", "C:/Games/StellarBlade")
            .unwrap();
        assert_eq!(
            store.get_setting("gameRoot").unwrap().as_deref(),
            Some("C:/Games/StellarBlade")
        );

        store.remember_detection("weird-mod", "genericPak").unwrap();
        assert_eq!(
            store.recall_detection("weird-mod").unwrap().as_deref(),
            Some("genericPak")
        );
    }

    #[test]
    fn migrating_an_existing_file_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sbmm.db");
        {
            let store = Store::open(&path).unwrap();
            store.insert_mod(&sample("A")).unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(store.list_mods().unwrap().len(), 1);
    }
}
