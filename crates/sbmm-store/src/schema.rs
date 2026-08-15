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
];

pub fn migrate(conn: &Connection) -> Result<()> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;

    let version: i64 =
        conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;

    for (index, sql) in MIGRATIONS.iter().enumerate() {
        let target = index as i64 + 1;
        if version >= target {
            continue;
        }
        conn.execute_batch(&format!("BEGIN; {sql} PRAGMA user_version = {target}; COMMIT;"))?;
    }
    Ok(())
}
