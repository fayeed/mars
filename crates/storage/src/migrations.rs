use std::fmt::Write;

use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};

use crate::{Result, StorageError, now_utc};

struct Migration {
    version: u32,
    name: &'static str,
    sql: &'static str,
}

const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    name: "core",
    sql: include_str!("../migrations/0001_core.sql"),
}];

pub(crate) fn apply_all(connection: &mut Connection) -> Result<()> {
    for migration in MIGRATIONS {
        apply(connection, migration)?;
    }
    Ok(())
}

fn apply(connection: &mut Connection, migration: &Migration) -> Result<()> {
    let transaction = connection.transaction()?;
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS _mars_migrations (
            version INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            checksum TEXT NOT NULL,
            applied_at TEXT NOT NULL
        ) STRICT;",
    )?;

    let checksum = checksum(migration.sql);
    let existing: Option<String> = transaction
        .query_row(
            "SELECT checksum FROM _mars_migrations WHERE version = ?1",
            [migration.version],
            |row| row.get(0),
        )
        .optional()?;

    if let Some(existing) = existing {
        if existing != checksum {
            return Err(StorageError::MigrationChecksum {
                version: migration.version,
            });
        }
        transaction.commit()?;
        return Ok(());
    }

    transaction.execute_batch(migration.sql)?;
    transaction.execute(
        "INSERT INTO _mars_migrations (version, name, checksum, applied_at)
         VALUES (?1, ?2, ?3, ?4)",
        params![migration.version, migration.name, checksum, now_utc()?],
    )?;
    transaction.pragma_update(None, "user_version", migration.version)?;
    transaction.commit()?;
    Ok(())
}

fn checksum(contents: &str) -> String {
    let digest = Sha256::digest(contents.as_bytes());
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    encoded
}
