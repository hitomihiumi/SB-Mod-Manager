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
    #[error("no profile with id {0}")]
    UnknownProfile(i64),
    #[error("the active profile, or the only one left, cannot be deleted")]
    ProfileInUse,
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
    /// Set once the mod's containers have been read for their asset list.
    pub assets_indexed_at: Option<String>,
    /// The variant options chosen at install, as a JSON list of labels.
    pub variant_choice: Option<String>,
    /// The user's own labels.
    pub tags: Vec<String>,
}

/// An asset more than one mod provides.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContestedAsset {
    pub asset: String,
    /// False when this is a chunk id rather than a path, because the mod's
    /// container was built without a directory index.
    pub named: bool,
    /// Every mod providing it, in install order.
    pub mod_ids: Vec<i64>,
}

/// One entry of the download queue, as it survives a restart.
///
/// The short-lived `key`/`expires` credentials an `nxm://` link carries are
/// deliberately not here: they would be stale by the next start anyway, and a
/// credential does not belong in a file on disk. A restored entry asks for a
/// fresh link if it needs one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadRecord {
    pub id: i64,
    pub mod_id: i64,
    pub file_id: i64,
    pub name: String,
    pub file_name: String,
    pub version: Option<String>,
    pub state: String,
    pub bytes_done: i64,
    pub bytes_total: Option<i64>,
    pub error: Option<String>,
    pub collection: Option<String>,
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

/// A named set of enabled mods and a load order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileRecord {
    pub id: i64,
    pub name: String,
    pub active: bool,
    pub enabled_count: i64,
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

/// One mod's place in a restore point.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PointEntry {
    pub mod_id: i64,
    pub enabled: bool,
    pub priority: i64,
}

/// A profile's enabled set and load order at one moment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestorePointRecord {
    pub id: i64,
    pub profile_id: i64,
    /// UTC, ISO 8601.
    pub created_at: String,
    pub label: String,
    /// Taken by the manager rather than asked for.
    pub automatic: bool,
    /// The game build at the time, when it was known.
    pub game_build: Option<String>,
    pub entries: Vec<PointEntry>,
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

    pub fn delete_setting(&self, key: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM settings WHERE key = ?1", [key])?;
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

    pub fn list_profiles(&self) -> Result<Vec<ProfileRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT p.id, p.name, p.active,
                    (SELECT COUNT(*) FROM profile_mods pm
                      WHERE pm.profile_id = p.id AND pm.enabled = 1)
             FROM profiles p ORDER BY p.id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(ProfileRecord {
                id: r.get(0)?,
                name: r.get(1)?,
                active: r.get::<_, i64>(2)? != 0,
                enabled_count: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Add a profile, taking its load order from `copy_from` (or the active
    /// profile). With `copy_enabled` the enabled set comes along too, which
    /// is a duplicate; without it every mod starts disabled, which is a fresh
    /// profile that still knows the order the user arranged.
    pub fn create_profile(
        &mut self,
        name: &str,
        copy_from: Option<i64>,
        copy_enabled: bool,
    ) -> Result<i64> {
        let source = match copy_from {
            Some(id) => id,
            None => self.active_profile()?,
        };
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO profiles (name, active) VALUES (?1, 0)",
            [name.trim()],
        )?;
        let id = tx.last_insert_rowid();
        // Every mod gets a row, so the new profile has a full, gap-free order
        // even for mods the source profile never had a row for.
        tx.execute(
            "INSERT INTO profile_mods (profile_id, mod_id, enabled, priority)
             SELECT ?1, m.id,
                    CASE WHEN ?3 THEN COALESCE(pm.enabled, 0) ELSE 0 END,
                    COALESCE(pm.priority, 1000000 + m.id)
             FROM mods m
             LEFT JOIN profile_mods pm ON pm.mod_id = m.id AND pm.profile_id = ?2",
            params![id, source, copy_enabled],
        )?;
        tx.commit()?;
        Ok(id)
    }

    pub fn rename_profile(&self, id: i64, name: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE profiles SET name = ?2 WHERE id = ?1",
            params![id, name.trim()],
        )?;
        Ok(())
    }

    /// Delete a profile. The active profile and the last one left are
    /// refused: something has to describe what the game folder holds.
    pub fn delete_profile(&self, id: i64) -> Result<()> {
        let (active, count): (i64, i64) = self.conn.query_row(
            "SELECT (SELECT active FROM profiles WHERE id = ?1),
                    (SELECT COUNT(*) FROM profiles)",
            [id],
            |r| Ok((r.get::<_, Option<i64>>(0)?.unwrap_or(0), r.get(1)?)),
        )?;
        if active != 0 || count <= 1 {
            return Err(StoreError::ProfileInUse);
        }
        self.conn
            .execute("DELETE FROM profiles WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Make `id` the profile every per-mod query reads from.
    pub fn set_active_profile(&mut self, id: i64) -> Result<()> {
        let tx = self.conn.transaction()?;
        let exists: i64 =
            tx.query_row("SELECT COUNT(*) FROM profiles WHERE id = ?1", [id], |r| {
                r.get(0)
            })?;
        if exists == 0 {
            return Err(StoreError::UnknownProfile(id));
        }
        tx.execute("UPDATE profiles SET active = (id = ?1)", [id])?;
        tx.commit()?;
        Ok(())
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

        // New mods start disabled at the end of the load order — in every
        // profile, so installing never changes what any profile loads and a
        // mod never turns up at the top of another profile's order.
        self.conn.execute(
            "INSERT INTO profile_mods (profile_id, mod_id, enabled, priority)
             SELECT p.id, ?1, 0,
                    (SELECT COALESCE(MAX(priority), 0) + 10
                       FROM profile_mods WHERE profile_id = p.id)
             FROM profiles p",
            [id],
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
                    m.latest_version, m.update_checked_at, m.assets_indexed_at,
                    m.variant_choice, m.tags
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
                assets_indexed_at: row.get(17)?,
                variant_choice: row.get(18)?,
                tags: serde_json::from_str(&row.get::<_, String>(19)?).unwrap_or_default(),
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// The user's note on a mod; empty clears it.
    pub fn set_notes(&self, mod_id: i64, notes: Option<&str>) -> Result<()> {
        self.update_mod(mod_id, "notes", notes)
    }

    /// The mod's picture, relative to its staging folder.
    pub fn set_image(&self, mod_id: i64, image: Option<&str>) -> Result<()> {
        self.update_mod(mod_id, "image_path", image)
    }

    pub fn set_tags(&self, mod_id: i64, tags: &[String]) -> Result<()> {
        let json = serde_json::to_string(tags)?;
        self.update_mod(mod_id, "tags", Some(&json))
    }

    fn update_mod(&self, mod_id: i64, column: &str, value: Option<&str>) -> Result<()> {
        // `column` is one of the fixed names above, never user input.
        let changed = self.conn.execute(
            &format!("UPDATE mods SET {column} = ?2 WHERE id = ?1"),
            params![mod_id, value],
        )?;
        if changed == 0 {
            return Err(StoreError::UnknownMod(mod_id));
        }
        Ok(())
    }

    /// Remember which variants of its archive a mod was installed with.
    pub fn set_variant_choice(&self, mod_id: i64, choice: Option<&str>) -> Result<()> {
        self.conn.execute(
            "UPDATE mods SET variant_choice = ?2 WHERE id = ?1",
            params![mod_id, choice],
        )?;
        Ok(())
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

    // -- download queue ----------------------------------------------------

    /// The queue as it was when the manager last closed.
    pub fn list_downloads(&self) -> Result<Vec<DownloadRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, mod_id, file_id, name, file_name, version, state,
                    bytes_done, bytes_total, error, collection
               FROM downloads ORDER BY id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(DownloadRecord {
                id: row.get(0)?,
                mod_id: row.get(1)?,
                file_id: row.get(2)?,
                name: row.get(3)?,
                file_name: row.get(4)?,
                version: row.get(5)?,
                state: row.get(6)?,
                bytes_done: row.get(7)?,
                bytes_total: row.get(8)?,
                error: row.get(9)?,
                collection: row.get(10)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Write the whole queue back.
    ///
    /// Replacing it wholesale rather than tracking individual edits: the queue
    /// is small, lives in memory as the source of truth, and a rewrite cannot
    /// drift out of step with it the way incremental updates can.
    pub fn replace_downloads(&mut self, items: &[DownloadRecord]) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM downloads", [])?;
        for item in items {
            tx.execute(
                "INSERT INTO downloads
                    (id, mod_id, file_id, name, file_name, version, state,
                     bytes_done, bytes_total, error, collection)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    item.id,
                    item.mod_id,
                    item.file_id,
                    item.name,
                    item.file_name,
                    item.version,
                    item.state,
                    item.bytes_done,
                    item.bytes_total,
                    item.error,
                    item.collection,
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    // -- what each mod replaces --------------------------------------------

    /// Record the assets a mod provides, replacing whatever was there.
    ///
    /// `complete` says whether every container was read. Only a complete pass
    /// stamps the mod as indexed, so a mod whose pak could not be opened stays
    /// in [`Self::mods_without_assets`] and gets another go — and a caller can
    /// tell "replaces nothing" apart from "could not be read", which look
    /// identical in a conflict list and mean opposite things.
    pub fn replace_mod_assets(
        &mut self,
        mod_id: i64,
        assets: &[(String, bool)],
        complete: bool,
    ) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM mod_assets WHERE mod_id = ?1", [mod_id])?;
        {
            let mut insert = tx.prepare(
                "INSERT OR IGNORE INTO mod_assets (mod_id, asset, named) VALUES (?1, ?2, ?3)",
            )?;
            for (asset, named) in assets {
                insert.execute(params![mod_id, asset, *named as i64])?;
            }
        }
        tx.execute(
            "UPDATE mods
                SET assets_indexed_at = CASE WHEN ?2 THEN datetime('now') ELSE NULL END
              WHERE id = ?1",
            params![mod_id, complete],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Mods with no complete asset index, so they can be caught up.
    pub fn mods_without_assets(&self) -> Result<Vec<i64>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM mods WHERE assets_indexed_at IS NULL ORDER BY id")?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Every asset more than one mod provides, with the mods that provide it.
    ///
    /// Grouped in SQL rather than by comparing mods pairwise: with a hundred
    /// mods the pairwise version is ten thousand set intersections, while this
    /// is one index scan.
    pub fn contested_assets(&self) -> Result<Vec<ContestedAsset>> {
        let mut stmt = self.conn.prepare(
            "SELECT a.asset, a.named, a.mod_id
               FROM mod_assets a
               JOIN (SELECT asset FROM mod_assets GROUP BY asset HAVING COUNT(*) > 1) c
                 ON c.asset = a.asset
              ORDER BY a.asset, a.mod_id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)? != 0,
                row.get::<_, i64>(2)?,
            ))
        })?;

        let mut out: Vec<ContestedAsset> = Vec::new();
        for row in rows {
            let (asset, named, mod_id) = row?;
            match out.last_mut() {
                Some(last) if last.asset == asset => last.mod_ids.push(mod_id),
                _ => out.push(ContestedAsset {
                    asset,
                    named,
                    mod_ids: vec![mod_id],
                }),
            }
        }
        Ok(out)
    }

    /// Every asset one mod provides, as `(asset, named)`.
    pub fn mod_assets(&self, mod_id: i64) -> Result<Vec<(String, bool)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT asset, named FROM mod_assets WHERE mod_id = ?1 ORDER BY asset")?;
        let rows = stmt.query_map([mod_id], |r| Ok((r.get(0)?, r.get::<_, i64>(1)? != 0)))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Whether a timestamp this store wrote is more than `hours` old.
    pub fn older_than_hours(&self, at: &str, hours: i64) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT ?1 < strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?2)",
            params![at, format!("-{hours} hours")],
            |r| r.get::<_, bool>(0),
        )?)
    }

    /// How many assets one mod provides.
    pub fn asset_count(&self, mod_id: i64) -> Result<i64> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM mod_assets WHERE mod_id = ?1",
            [mod_id],
            |row| row.get(0),
        )?)
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

    /// Mods enabled in at least one profile.
    pub fn mods_enabled_anywhere(&self) -> Result<std::collections::BTreeSet<i64>> {
        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT mod_id FROM profile_mods WHERE enabled = 1")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
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

    // -- restore points ----------------------------------------------------

    pub fn add_restore_point(
        &self,
        profile_id: i64,
        label: &str,
        automatic: bool,
        game_build: Option<&str>,
        entries: &[PointEntry],
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO restore_points (profile_id, label, automatic, game_build, state)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                profile_id,
                label,
                automatic as i64,
                game_build,
                serde_json::to_string(entries)?
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// A profile's points, newest first.
    pub fn restore_points(&self, profile_id: i64) -> Result<Vec<RestorePointRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, profile_id, created_at, label, automatic, game_build, state
             FROM restore_points WHERE profile_id = ?1 ORDER BY id DESC",
        )?;
        let rows = stmt.query_map([profile_id], |r| {
            Ok((
                RestorePointRecord {
                    id: r.get(0)?,
                    profile_id: r.get(1)?,
                    created_at: r.get(2)?,
                    label: r.get(3)?,
                    automatic: r.get::<_, i64>(4)? != 0,
                    game_build: r.get(5)?,
                    entries: Vec::new(),
                },
                r.get::<_, String>(6)?,
            ))
        })?;
        let mut points = Vec::new();
        for row in rows {
            let (mut point, state) = row?;
            point.entries = serde_json::from_str(&state)?;
            points.push(point);
        }
        Ok(points)
    }

    pub fn delete_restore_point(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM restore_points WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Keep only the newest `keep` automatic points of a profile; the ones the
    /// user made stay until deleted.
    pub fn prune_automatic_points(&self, profile_id: i64, keep: usize) -> Result<()> {
        self.conn.execute(
            "DELETE FROM restore_points
             WHERE profile_id = ?1 AND automatic = 1 AND id NOT IN (
                 SELECT id FROM restore_points
                 WHERE profile_id = ?1 AND automatic = 1
                 ORDER BY id DESC LIMIT ?2
             )",
            params![profile_id, keep as i64],
        )?;
        Ok(())
    }

    // -- load order rules ---------------------------------------------------

    /// `before` loads before `after`. Adding a rule that exists is a no-op.
    pub fn add_order_rule(&self, before: i64, after: i64) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO order_rules (before_mod, after_mod) VALUES (?1, ?2)",
            params![before, after],
        )?;
        Ok(())
    }

    /// Every rule as `(id, before, after)`.
    pub fn order_rules(&self) -> Result<Vec<(i64, i64, i64)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, before_mod, after_mod FROM order_rules ORDER BY id")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn delete_order_rule(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM order_rules WHERE id = ?1", [id])?;
        Ok(())
    }

    // -- what Nexus says ------------------------------------------------------

    pub fn put_nexus_details(&self, nexus_mod_id: i64, details: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO nexus_details (nexus_mod_id, details) VALUES (?1, ?2)
             ON CONFLICT(nexus_mod_id) DO UPDATE
                SET details = excluded.details,
                    fetched_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')",
            params![nexus_mod_id, details],
        )?;
        Ok(())
    }

    /// Every cached answer as `nexus mod id → (details JSON, fetched at)`.
    pub fn nexus_details(&self) -> Result<std::collections::BTreeMap<i64, (String, String)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT nexus_mod_id, details, fetched_at FROM nexus_details")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
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
    fn order_rules_go_with_their_mods() {
        let store = Store::open_in_memory().unwrap();
        let a = store.insert_mod(&sample("Alpha")).unwrap();
        let b = store.insert_mod(&sample("Beta")).unwrap();
        store.add_order_rule(a, b).unwrap();
        store.add_order_rule(a, b).unwrap();
        assert_eq!(store.order_rules().unwrap().len(), 1);
        store.delete_mod(b).unwrap();
        assert!(store.order_rules().unwrap().is_empty());
    }

    #[test]
    fn nexus_answers_are_cached_per_mod() {
        let store = Store::open_in_memory().unwrap();
        store.put_nexus_details(5, "{\"a\":1}").unwrap();
        store.put_nexus_details(5, "{\"a\":2}").unwrap();
        let cached = store.nexus_details().unwrap();
        assert_eq!(cached[&5].0, "{\"a\":2}");
    }

    #[test]
    fn notes_tags_and_pictures_are_kept_on_the_mod() {
        let store = Store::open_in_memory().unwrap();
        let id = store.insert_mod(&sample("Alpha")).unwrap();
        assert!(store.list_mods().unwrap()[0].tags.is_empty());

        store
            .set_notes(id, Some("Needs the physics patch"))
            .unwrap();
        store.set_image(id, Some("images/cover.png")).unwrap();
        store
            .set_tags(id, &["outfit".to_string(), "nsfw".to_string()])
            .unwrap();
        let mod_ = &store.list_mods().unwrap()[0];
        assert_eq!(mod_.notes.as_deref(), Some("Needs the physics patch"));
        assert_eq!(mod_.image_path.as_deref(), Some("images/cover.png"));
        assert_eq!(mod_.tags, ["outfit", "nsfw"]);

        store.set_notes(id, None).unwrap();
        assert_eq!(store.list_mods().unwrap()[0].notes, None);
        assert!(store.set_notes(999, Some("x")).is_err());
    }

    #[test]
    fn restore_points_round_trip_and_only_automatic_ones_are_pruned() {
        let store = Store::open_in_memory().unwrap();
        let profile = store.active_profile().unwrap();
        let entries = vec![PointEntry {
            mod_id: 7,
            enabled: true,
            priority: 10,
        }];
        let mine = store
            .add_restore_point(
                profile,
                "Before the big update",
                false,
                Some("1870"),
                &entries,
            )
            .unwrap();
        for n in 0..5 {
            store
                .add_restore_point(profile, &format!("Applied {n}"), true, None, &entries)
                .unwrap();
        }
        store.prune_automatic_points(profile, 2).unwrap();

        let points = store.restore_points(profile).unwrap();
        let labels: Vec<_> = points.iter().map(|p| p.label.as_str()).collect();
        assert_eq!(labels, ["Applied 4", "Applied 3", "Before the big update"]);
        let kept = points.iter().find(|p| p.id == mine).unwrap();
        assert_eq!(kept.entries, entries);
        assert_eq!(kept.game_build.as_deref(), Some("1870"));
        assert!(kept.created_at.ends_with('Z'));

        store.delete_restore_point(mine).unwrap();
        assert_eq!(store.restore_points(profile).unwrap().len(), 2);
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
