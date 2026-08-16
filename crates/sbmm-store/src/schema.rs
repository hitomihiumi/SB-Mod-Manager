//! Database schema and forward migrations.
//!
//! Versioning uses SQLite's `user_version` pragma. Each migration is applied
//! in order inside a transaction, so an interrupted upgrade leaves the file at
//! its previous version rather than half-migrated.

use rusqlite::{Connection, Result};

/// Every migration, in order. Append only — never edit a shipped entry.
const MIGRATIONS: &[&str] = &[
    // v1 — initial schema.
    r#"
    CREATE TABLE settings (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );

    CREATE TABLE groups (
        id         INTEGER PRIMARY KEY AUTOINCREMENT,
        name       TEXT NOT NULL UNIQUE,
        color      TEXT,
        sort_index INTEGER NOT NULL DEFAULT 0,
        collapsed  INTEGER NOT NULL DEFAULT 0
    );

    CREATE TABLE mods (
        id             INTEGER PRIMARY KEY AUTOINCREMENT,
        name           TEXT NOT NULL,
        staging_folder TEXT NOT NULL UNIQUE,
        version        TEXT,
        source         TEXT NOT NULL DEFAULT 'manual',
        nexus_mod_id   INTEGER,
        nexus_file_id  INTEGER,
        group_id       INTEGER REFERENCES groups(id) ON DELETE SET NULL,
        primary_type   TEXT NOT NULL DEFAULT 'unknown',
        components     TEXT NOT NULL DEFAULT '[]',
        size_bytes     INTEGER NOT NULL DEFAULT 0,
        archive_hash   TEXT,
        image_path     TEXT,
        notes          TEXT,
        installed_at   TEXT NOT NULL DEFAULT (datetime('now'))
    );

    CREATE TABLE profiles (
        id     INTEGER PRIMARY KEY AUTOINCREMENT,
        name   TEXT NOT NULL UNIQUE,
        active INTEGER NOT NULL DEFAULT 0
    );

    -- Enabled state and load order live per profile from the start, so
    -- adding profile switching later needs no data migration.
    CREATE TABLE profile_mods (
        profile_id INTEGER NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
        mod_id     INTEGER NOT NULL REFERENCES mods(id) ON DELETE CASCADE,
        enabled    INTEGER NOT NULL DEFAULT 0,
        priority   INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY (profile_id, mod_id)
    );

    -- The record of what is physically in the game folder right now.
    CREATE TABLE deployed_files (
        id      INTEGER PRIMARY KEY AUTOINCREMENT,
        mod_id  INTEGER NOT NULL,
        target  TEXT NOT NULL,
        backup  TEXT,
        size    INTEGER NOT NULL DEFAULT 0
    );
    CREATE INDEX idx_deployed_files_mod ON deployed_files(mod_id);

    CREATE TABLE created_dirs (
        path TEXT PRIMARY KEY
    );

    -- User corrections to detection, so the same mistake is not repeated.
    CREATE TABLE detection_rules (
        pattern  TEXT PRIMARY KEY,
        mod_type TEXT NOT NULL
    );

    INSERT INTO profiles (name, active) VALUES ('Default', 1);
    "#,
    // v2 — what a Nexus update check leaves behind.
    //
    // The version a mod is *at* is already in `mods.version`; these record
    // what Nexus last said was available, so the badge survives a restart
    // without re-spending the API allowance.
    r#"
    ALTER TABLE mods ADD COLUMN latest_version TEXT;
    ALTER TABLE mods ADD COLUMN update_checked_at TEXT;
    "#,
    // v3 — the download queue, so closing the manager mid-collection does not
    // lose forty entries and orphan whatever was half-fetched.
    //
    // The `key`/`expires` pair from an nxm:// link is deliberately absent:
    // it is a short-lived credential that would be stale by the next start,
    // and it has no business sitting in a file on disk.
    r#"
    CREATE TABLE downloads (
        id          INTEGER PRIMARY KEY,
        mod_id      INTEGER NOT NULL,
        file_id     INTEGER NOT NULL,
        name        TEXT NOT NULL,
        file_name   TEXT NOT NULL,
        version     TEXT,
        state       TEXT NOT NULL,
        bytes_done  INTEGER NOT NULL DEFAULT 0,
        bytes_total INTEGER,
        error       TEXT,
        collection  TEXT
    );
    "#,
];

pub fn migrate(conn: &Connection) -> Result<()> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;

    let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;

    for (index, sql) in MIGRATIONS.iter().enumerate() {
        let target = index as i64 + 1;
        if version >= target {
            continue;
        }
        conn.execute_batch(&format!(
            "BEGIN; {sql} PRAGMA user_version = {target}; COMMIT;"
        ))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Existing installs are at v1, so the upgrade has to land on a database
    /// that already holds mods — and leave them alone.
    #[test]
    fn a_v1_database_upgrades_without_losing_anything() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(&format!(
            "BEGIN; {} PRAGMA user_version = 1; COMMIT;",
            MIGRATIONS[0]
        ))
        .unwrap();
        conn.execute(
            "INSERT INTO mods (name, staging_folder, version) VALUES ('Old Mod', 'old-mod', '1.0')",
            [],
        )
        .unwrap();

        migrate(&conn).unwrap();

        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, MIGRATIONS.len() as i64);

        let (name, latest): (String, Option<String>) = conn
            .query_row(
                "SELECT name, latest_version FROM mods WHERE staging_folder = 'old-mod'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(name, "Old Mod");
        assert_eq!(latest, None, "nothing has been checked yet");
    }

    /// Running it twice must be a no-op, since it runs on every start.
    #[test]
    fn migrating_an_up_to_date_database_changes_nothing() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        migrate(&conn).unwrap();

        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, MIGRATIONS.len() as i64);
    }
}
