use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OpenFlags};

const GROUP_CONTAINER: &str = "9K33E3U3T4.net.shinyfrog.bear";
const DB_RELATIVE: &str = "Application Data/database.sqlite";

/// Absolute path to Bear's SQLite database.
pub fn db_path() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").context("$HOME is not set")?;
    let path = PathBuf::from(home)
        .join("Library")
        .join("Group Containers")
        .join(GROUP_CONTAINER)
        .join(DB_RELATIVE);
    if !path.exists() {
        bail!("cannot open Bear database at {}", path.display());
    }
    Ok(path)
}

/// Absolute path to the Bear group container root.
pub fn group_container_path() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").context("$HOME is not set")?;
    Ok(PathBuf::from(home)
        .join("Library")
        .join("Group Containers")
        .join(GROUP_CONTAINER))
}

/// Open the Bear database read-only.
pub fn open_ro() -> Result<Connection> {
    let path = db_path()?;
    let conn = Connection::open_with_flags(
        &path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .with_context(|| format!("cannot open Bear database at {}", path.display()))?;
    conn.execute_batch("PRAGMA busy_timeout=3000; PRAGMA query_only=1;")?;
    Ok(conn)
}

/// Open the Bear database read-write (WAL mode, busy timeout).
pub fn open_rw() -> Result<Connection> {
    let path = db_path()?;
    let conn = Connection::open_with_flags(
        &path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .with_context(|| format!("cannot open Bear database at {}", path.display()))?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")?;
    Ok(conn)
}

// ── CoreData timestamp conversion ─────────────────────────────────────────────
//
// CoreData stores NSDate as seconds since 2001-01-01 00:00:00 UTC (f64).
// Unix epoch offset from that date: 978 307 200 seconds.

pub const COREDATA_EPOCH_OFFSET: i64 = 978_307_200;

/// CoreData f64 timestamp → Unix i64 timestamp (seconds).
#[inline]
pub fn coredata_to_unix(ts: f64) -> i64 {
    ts as i64 + COREDATA_EPOCH_OFFSET
}

/// Unix i64 timestamp → CoreData f64 timestamp.
#[inline]
pub fn unix_to_coredata(ts: i64) -> f64 {
    (ts - COREDATA_EPOCH_OFFSET) as f64
}

/// Current time as a CoreData timestamp.
pub fn now_coredata() -> f64 {
    unix_to_coredata(chrono::Utc::now().timestamp())
}

/// Entity number for SFNote in the current CoreData model (used in join tables).
/// Verified from live schema: Z_5TAGS.Z_5NOTES / Z_5PINNEDINTAGS.Z_5PINNEDNOTES.
pub const SFNOTE_ENT: i64 = 5;

/// Entity number for SFNoteTag in the current CoreData model.
/// Verified from live schema: Z_5TAGS.Z_13TAGS / Z_5PINNEDINTAGS.Z_13PINNEDINTAGS.
pub const SFNOTETAG_ENT: i64 = 13;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_timestamp() {
        let unix = 1_700_000_000i64;
        let cd = unix_to_coredata(unix);
        assert_eq!(coredata_to_unix(cd), unix);
    }

    #[test]
    fn known_coredata_value() {
        // 0.0 in CoreData = 2001-01-01 00:00:00 UTC = Unix 978307200
        assert_eq!(coredata_to_unix(0.0), COREDATA_EPOCH_OFFSET);
    }
}
