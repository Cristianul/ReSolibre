//! Local, profile-scoped analysis storage. Call only from cancellable background jobs.

pub mod audio_identity;
mod files;
pub mod metadata;
#[cfg(test)]
mod tests;
mod track_sources;
mod validation;

use audio_identity::AudioIdentity;
use resolibre_core::analysis::{TrackAnalysis, tempo::TempoReport};
use rusqlite::{Connection, OpenFlags, OptionalExtension, config::DbConfig, params};
use std::path::Path;
use std::time::Duration;

const MAX_RESULT_BYTES: usize = 4 * 1024 * 1024;
const MAX_ROWS: usize = 1000;
const MAX_PAYLOAD_BYTES: usize = 192 * 1024 * 1024;
const SCHEMA: i64 = 6;

/// One connection owned by an analysis worker; neither UI nor playback callbacks use SQLite.
pub struct AnalysisCache {
    connection: Connection,
}

/// Storage errors deliberately exclude paths, private metadata and native diagnostics.
#[derive(Debug)]
pub struct CacheError;

impl AnalysisCache {
    /// Opens a private profile database with explicit schema compatibility and bounded native resources.
    pub fn open(directory: &Path) -> Result<Self, CacheError> {
        let path = files::database_path(directory).map_err(|_| CacheError)?;
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX
                | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .map_err(|_| CacheError)?;
        connection
            .busy_timeout(Duration::from_millis(50))
            .map_err(|_| CacheError)?;
        connection
            .set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true)
            .map_err(|_| CacheError)?;
        connection
            .set_db_config(DbConfig::SQLITE_DBCONFIG_TRUSTED_SCHEMA, false)
            .map_err(|_| CacheError)?;
        connection
            .set_limit(
                rusqlite::limits::Limit::SQLITE_LIMIT_LENGTH,
                (MAX_RESULT_BYTES + 16384) as i32,
            )
            .map_err(|_| CacheError)?;
        connection
            .execute_batch(
                "PRAGMA foreign_keys=ON; PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL;
            PRAGMA cache_size=-2048; PRAGMA temp_store=MEMORY; PRAGMA mmap_size=0;",
            )
            .map_err(|_| CacheError)?;
        let page_size: i64 = connection
            .pragma_query_value(None, "page_size", |row| row.get(0))
            .map_err(|_| CacheError)?;
        if !(512..=65536).contains(&page_size) {
            return Err(CacheError);
        }
        connection
            .pragma_update(
                None,
                "max_page_count",
                files::MAX_DATABASE_BYTES as i64 / page_size,
            )
            .map_err(|_| CacheError)?;
        let version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(|_| CacheError)?;
        if version == 0 {
            connection
                .execute_batch(
                    "BEGIN IMMEDIATE;
                CREATE TABLE IF NOT EXISTS analyses (
                    hash BLOB NOT NULL CHECK(length(hash)=32),
                    kind INTEGER NOT NULL CHECK(kind IN (0,1,2)),
                    artist TEXT NOT NULL, title TEXT NOT NULL,
                    payload BLOB NOT NULL, accessed INTEGER NOT NULL,
                    PRIMARY KEY(hash,kind));
                CREATE INDEX IF NOT EXISTS analysis_access ON analyses(accessed);
                PRAGMA user_version=6; COMMIT;",
                )
                .map_err(|_| CacheError)?;
        } else if matches!(version, 1..=5) {
            migrate_analysis_cache(&connection)?;
        } else if version != SCHEMA {
            return Err(CacheError);
        }
        let database = Self { connection };
        database.prepare_track_sources()?;
        Ok(database)
    }

    /// Returns a validated prefix or complete timeline; complete coverage can satisfy a prefix request.
    pub fn rhythm(
        &self,
        identity: &AudioIdentity,
        whole: bool,
    ) -> Result<Option<TempoReport>, CacheError> {
        let kinds: &[i64] = if whole { &[1, 2] } else { &[1, 2, 0] };
        for kind in kinds {
            if let Some(bytes) = self.read(identity, *kind)? {
                let report = if *kind == 2 {
                    serde_json::from_slice::<TrackAnalysis>(&bytes)
                        .ok()
                        .filter(validation::track)
                        .map(|report| report.tempo)
                } else {
                    serde_json::from_slice::<TempoReport>(&bytes)
                        .ok()
                        .filter(validation::tempo)
                };
                if let Some(report) = report {
                    return Ok(Some(report));
                }
                self.remove(identity, *kind)?;
            }
        }
        Ok(None)
    }

    /// Loads a bounded full spectrogram; damaged measurements become a retryable cache miss.
    pub fn track(&self, identity: &AudioIdentity) -> Result<Option<TrackAnalysis>, CacheError> {
        let Some(bytes) = self.read(identity, 2)? else {
            return Ok(None);
        };
        let report = serde_json::from_slice::<TrackAnalysis>(&bytes)
            .ok()
            .filter(validation::track);
        if report.is_none() {
            self.remove(identity, 2)?;
        }
        Ok(report)
    }

    /// Stores inconclusive results too, while preserving prefix/whole coverage as distinct facts.
    pub fn save_rhythm(
        &mut self,
        identity: &AudioIdentity,
        whole: bool,
        report: &TempoReport,
    ) -> Result<(), CacheError> {
        if !validation::tempo(report) {
            return Err(CacheError);
        }
        let bytes = serde_json::to_vec(report).map_err(|_| CacheError)?;
        self.write(identity, i64::from(whole), &bytes)
    }

    /// Persists a complete original-file analysis only after the adapter has revalidated its input.
    pub fn save_track(
        &mut self,
        identity: &AudioIdentity,
        report: &TrackAnalysis,
    ) -> Result<(), CacheError> {
        if !validation::track(report) {
            return Err(CacheError);
        }
        let bytes = serde_json::to_vec(report).map_err(|_| CacheError)?;
        self.write(identity, 2, &bytes)
    }

    /// Clears application measurements without touching media or saved playback preferences.
    pub fn clear(&mut self) -> Result<(), CacheError> {
        self.connection
            .execute_batch("DELETE FROM analyses; VACUUM;")
            .map_err(|_| CacheError)
    }

    /// Bounds allocation in SQL before loading a payload and refreshes descriptive labels on reuse.
    fn read(&self, identity: &AudioIdentity, kind: i64) -> Result<Option<Vec<u8>>, CacheError> {
        let bytes = self
            .connection
            .query_row(
                "SELECT payload FROM analyses WHERE hash=?1 AND kind=?2 AND length(payload)<=?3",
                params![identity.digest.as_slice(), kind, MAX_RESULT_BYTES as i64],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| CacheError)?;
        if bytes.is_some() {
            self.connection.execute(
                "UPDATE analyses SET artist=?3,title=?4,accessed=unixepoch('subsec')*1000 WHERE hash=?1 AND kind=?2",
                params![identity.digest.as_slice(), kind, identity.artist, identity.title]
            ).map_err(|_| CacheError)?;
        }
        Ok(bytes)
    }

    /// Removes only a damaged cache result, leaving other coverage and all original music intact.
    fn remove(&self, identity: &AudioIdentity, kind: i64) -> Result<(), CacheError> {
        self.connection
            .execute(
                "DELETE FROM analyses WHERE hash=?1 AND kind=?2",
                params![identity.digest.as_slice(), kind],
            )
            .map_err(|_| CacheError)?;
        Ok(())
    }

    /// Atomically replaces one result and evicts least-recently-used rows within count/byte budgets.
    fn write(
        &mut self,
        identity: &AudioIdentity,
        kind: i64,
        bytes: &[u8],
    ) -> Result<(), CacheError> {
        if bytes.len() > MAX_RESULT_BYTES
            || identity.artist.len() > 2048
            || identity.title.len() > 2048
        {
            return Err(CacheError);
        }
        let transaction = self.connection.transaction().map_err(|_| CacheError)?;
        // A new full report also replaces older rhythm coverage for this recording.
        if kind == 2 {
            transaction
                .execute(
                    "DELETE FROM analyses WHERE hash=?1 AND kind IN (0,1)",
                    params![identity.digest.as_slice()],
                )
                .map_err(|_| CacheError)?;
        }
        transaction
            .execute(
                "INSERT INTO analyses(hash,kind,artist,title,payload,accessed)
            VALUES(?1,?2,?3,?4,?5,unixepoch('subsec')*1000)
            ON CONFLICT(hash,kind) DO UPDATE SET artist=excluded.artist,title=excluded.title,
            payload=excluded.payload,accessed=excluded.accessed",
                params![
                    identity.digest.as_slice(),
                    kind,
                    identity.artist,
                    identity.title,
                    bytes
                ],
            )
            .map_err(|_| CacheError)?;
        loop {
            let (count, size): (i64, i64) = transaction
                .query_row(
                    "SELECT count(*),coalesce(sum(length(payload)),0) FROM analyses",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map_err(|_| CacheError)?;
            if count <= MAX_ROWS as i64 && size <= MAX_PAYLOAD_BYTES as i64 {
                break;
            }
            transaction.execute("DELETE FROM analyses WHERE rowid=(SELECT rowid FROM analyses ORDER BY accessed,rowid LIMIT 1)", []).map_err(|_| CacheError)?;
        }
        transaction.commit().map_err(|_| CacheError)
    }
}

/// Invalidates incompatible reports atomically; schema 5 retains rhythm and refreshes provenance/full results.
fn migrate_analysis_cache(connection: &Connection) -> Result<(), CacheError> {
    connection
        .execute_batch("BEGIN IMMEDIATE;")
        .map_err(|_| CacheError)?;
    // Recheck after acquiring the write lock: another process may have migrated already.
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|_| CacheError)?;
    if matches!(version, 1..=4) {
        connection
            .execute_batch("DELETE FROM analyses; PRAGMA user_version=6;")
            .map_err(|_| CacheError)?;
    } else if version == 5 {
        connection
            .execute_batch("DELETE FROM analyses WHERE kind=2; PRAGMA user_version=6;")
            .map_err(|_| CacheError)?;
    } else if version != SCHEMA {
        return Err(CacheError);
    }
    connection.execute_batch("COMMIT;").map_err(|_| CacheError)
}
