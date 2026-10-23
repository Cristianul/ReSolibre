//! Small reusable tag/duration records; file facts are not audio-content hashes.

use crate::{AnalysisCache, CacheError};
use rusqlite::{OptionalExtension, params};

/// Sanitized presentation data only; artwork and full analysis stay out of this cache.
#[derive(Debug, PartialEq)]
pub struct Metadata {
    pub title: Option<String>,
    pub search_text: String,
    pub duration_seconds: Option<u32>,
}

impl Metadata {
    /// Rejects oversized or unsafe stored text before presentation, including tampered rows.
    fn valid(&self) -> bool {
        let title = self.title.as_deref().unwrap_or("");
        title.len() <= 1300
            && self.search_text.len() <= 8192
            && title
                .chars()
                .chain(self.search_text.chars())
                .all(|character| {
                    (!character.is_control() || character == '\n')
                        && !matches!(character, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
                })
    }
}

impl AnalysisCache {
    /// Initializes an additive, rebuildable metadata table without invalidating audio analyses.
    pub fn prepare_metadata(&self) -> Result<(), CacheError> {
        self.connection
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS metadata_v1 (
            identity BLOB PRIMARY KEY CHECK(length(identity)=56),
            title TEXT, search TEXT NOT NULL, duration INTEGER,
            accessed INTEGER NOT NULL);",
            )
            .map_err(|_| CacheError)
    }

    /// Loads one bounded row; changed file facts miss without reading compressed audio.
    pub fn metadata(&self, identity: &[u8]) -> Result<Option<Metadata>, CacheError> {
        if identity.len() != 56 {
            return Err(CacheError);
        }
        let record = self
            .connection
            .query_row(
                "SELECT title,search,duration FROM metadata_v1 WHERE identity=?1
             AND length(CAST(coalesce(title,'') AS BLOB))<=1300
             AND length(CAST(search AS BLOB))<=8192",
                [identity],
                |row| {
                    Ok(Metadata {
                        title: row.get(0)?,
                        search_text: row.get(1)?,
                        duration_seconds: row.get(2)?,
                    })
                },
            )
            .optional()
            .map_err(|_| CacheError)?;
        Ok(record.filter(Metadata::valid))
    }

    /// Saves only validated display facts; bounds records to 4,096 without loading the table.
    pub fn save_metadata(&mut self, identity: &[u8], record: &Metadata) -> Result<(), CacheError> {
        if identity.len() != 56 || !record.valid() {
            return Err(CacheError);
        }
        let transaction = self.connection.transaction().map_err(|_| CacheError)?;
        transaction
            .execute(
                "INSERT OR REPLACE INTO metadata_v1 VALUES (?1,?2,?3,?4,unixepoch('subsec')*1000)",
                params![
                    identity,
                    record.title,
                    record.search_text,
                    record.duration_seconds
                ],
            )
            .map_err(|_| CacheError)?;
        transaction
            .execute(
                "DELETE FROM metadata_v1 WHERE rowid IN
            (SELECT rowid FROM metadata_v1 ORDER BY accessed DESC,rowid DESC LIMIT -1 OFFSET 4096)",
                [],
            )
            .map_err(|_| CacheError)?;
        transaction.commit().map_err(|_| CacheError)
    }
}
