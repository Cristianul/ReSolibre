//! Small file-fact hints for Library badges; playback still verifies the complete audio hash.

use crate::{AnalysisCache, CacheError, audio_identity::AudioIdentity};
use rusqlite::{OptionalExtension, params};

impl AnalysisCache {
    /// Adds a bounded, rebuildable index without invalidating existing full analyses.
    pub(crate) fn prepare_track_sources(&self) -> Result<(), CacheError> {
        self.connection
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS track_sources_v1 (
                    identity BLOB PRIMARY KEY CHECK(length(identity)=56),
                    hash BLOB NOT NULL CHECK(length(hash)=32),
                    accessed INTEGER NOT NULL);",
            )
            .map_err(|_| CacheError)
    }

    /// Avoids reading audio during startup when this profile has no full reports to restore.
    pub fn has_tracks(&self) -> Result<bool, CacheError> {
        self.connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM analyses WHERE kind=2)",
                [],
                |row| row.get(0),
            )
            .map_err(|_| CacheError)
    }

    /// Resolves unchanged file facts for presentation only; this does not authorize cached audio use.
    pub fn track_source(&self, identity: &[u8]) -> Result<Option<AudioIdentity>, CacheError> {
        if identity.len() != 56 {
            return Err(CacheError);
        }
        let digest: Option<Vec<u8>> = self
            .connection
            .query_row(
                "SELECT hash FROM track_sources_v1 WHERE identity=?1 AND length(hash)=32",
                [identity],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| CacheError)?;
        let Some(digest) = digest else {
            return Ok(None);
        };
        Ok(Some(AudioIdentity {
            digest: digest.try_into().map_err(|_| CacheError)?,
            artist: String::new(),
            title: String::new(),
        }))
    }

    /// Remembers verified content identity, including misses, so unchanged files need no startup rehash.
    pub fn save_track_source(
        &mut self,
        identity: &[u8],
        audio: &AudioIdentity,
    ) -> Result<(), CacheError> {
        if identity.len() != 56 {
            return Err(CacheError);
        }
        let transaction = self.connection.transaction().map_err(|_| CacheError)?;
        transaction
            .execute(
                "INSERT OR REPLACE INTO track_sources_v1 VALUES (?1,?2,unixepoch('subsec')*1000)",
                params![identity, audio.digest.as_slice()],
            )
            .map_err(|_| CacheError)?;
        transaction.execute(
            "DELETE FROM track_sources_v1 WHERE rowid IN
             (SELECT rowid FROM track_sources_v1 ORDER BY accessed DESC,rowid DESC LIMIT -1 OFFSET 4096)",
            [],
        ).map_err(|_| CacheError)?;
        transaction.commit().map_err(|_| CacheError)
    }

    /// Validates one bounded full report without changing descriptive labels or retaining the plot.
    pub fn has_track(&self, identity: &AudioIdentity) -> Result<bool, CacheError> {
        let bytes: Option<Vec<u8>> = self
            .connection
            .query_row(
                "SELECT payload FROM analyses WHERE hash=?1 AND kind=2 AND length(payload)<=?2",
                params![identity.digest.as_slice(), crate::MAX_RESULT_BYTES as i64],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| CacheError)?;
        let Some(bytes) = bytes else {
            return Ok(false);
        };
        let valid = serde_json::from_slice(&bytes)
            .ok()
            .is_some_and(|report| crate::validation::track(&report));
        if !valid {
            self.remove(identity, 2)?;
        }
        Ok(valid)
    }
}
