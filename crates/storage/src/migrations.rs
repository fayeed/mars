use std::fmt::Write;

use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};

use crate::{Result, StorageError, now_utc};

struct Migration {
    version: u32,
    name: &'static str,
    sql: &'static str,
}

const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "core",
        sql: include_str!("../migrations/0001_core.sql"),
    },
    Migration {
        version: 2,
        name: "projects",
        sql: include_str!("../migrations/0002_projects.sql"),
    },
    Migration {
        version: 3,
        name: "code_graph",
        sql: include_str!("../migrations/0003_code_graph.sql"),
    },
    Migration {
        version: 4,
        name: "tasks_worktrees",
        sql: include_str!("../migrations/0004_tasks_worktrees.sql"),
    },
    Migration {
        version: 5,
        name: "agent_sessions",
        sql: include_str!("../migrations/0005_agent_sessions.sql"),
    },
    Migration {
        version: 6,
        name: "context",
        sql: include_str!("../migrations/0006_context.sql"),
    },
    Migration {
        version: 7,
        name: "skills",
        sql: include_str!("../migrations/0007_skills.sql"),
    },
    Migration {
        version: 8,
        name: "knowledge_fts",
        sql: include_str!("../migrations/0008_knowledge_fts.sql"),
    },
    Migration {
        version: 9,
        name: "knowledge",
        sql: include_str!("../migrations/0009_knowledge.sql"),
    },
    Migration {
        version: 10,
        name: "reviews",
        sql: include_str!("../migrations/0010_reviews.sql"),
    },
];

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
