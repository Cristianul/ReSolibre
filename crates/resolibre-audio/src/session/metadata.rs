//! Queue-owned tag labels and one current cover; late results cannot relabel another occurrence.

use super::Snapshot;
use crate::metadata::Job;
use resolibre_core::queue::Queue;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

struct Pending {
    id: u64,
    cover: bool,
    job: Job,
}

#[derive(Default)]
struct CachedMetadata {
    title: Option<String>,
    search_text: String,
    duration_seconds: Option<u32>,
}

#[derive(Default)]
pub(super) struct MetadataRuntime {
    persistent: Option<Arc<crate::analysis_cache::CacheConfig>>,
    labels: HashMap<u64, CachedMetadata>,
    pending: Option<Pending>,
    warmup: Option<Job<crate::metadata::WarmMetadata>>,
    warmed: bool,
    current: Option<u64>,
    cover_attempted: bool,
    preview: Option<u64>,
    preview_attempted: bool,
    warned: bool,
}

impl MetadataRuntime {
    /// Shares storage configuration without opening files on the session thread.
    pub fn new(persistent: Option<Arc<crate::analysis_cache::CacheConfig>>) -> Self {
        Self {
            persistent,
            ..Self::default()
        }
    }

    /// Releases removed occurrences and their work; cache size never exceeds the bounded queue.
    pub fn retain(&mut self, queue: &Queue<PathBuf>, snapshot: &mut Snapshot) {
        let ids: HashSet<_> = queue.entries().iter().map(|entry| entry.id).collect();
        self.labels.retain(|id, _| ids.contains(id));
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| !ids.contains(&pending.id))
        {
            self.pending = None;
        }
        self.select(queue.current_id(), snapshot);
    }

    /// Returns cached, sanitized tags or the shared filename fallback without probing synchronously.
    pub fn title(&self, id: u64, path: &Path) -> String {
        self.labels
            .get(&id)
            .and_then(|details| details.title.clone())
            .unwrap_or_else(|| super::display_title(path))
    }

    /// Includes the filename even when tags supply a different display title; no private directory is exposed.
    pub fn search_text(&self, id: u64, path: &Path) -> String {
        let filename = super::display_title(path).to_lowercase();
        match self.labels.get(&id) {
            Some(details) if !details.search_text.is_empty() => {
                format!("{filename}\n{}", details.search_text)
            }
            _ => filename,
        }
    }

    /// Returns source duration from the shared probe, independent of playback processing.
    pub fn duration_seconds(&self, id: u64) -> Option<u32> {
        self.labels
            .get(&id)
            .and_then(|details| details.duration_seconds)
    }

    /// Clears old artwork immediately, prioritizing the newly selected track over background labels.
    pub fn select(&mut self, current: Option<u64>, snapshot: &mut Snapshot) {
        if self.current == current {
            return;
        }
        self.pending = None;
        self.current = current;
        self.cover_attempted = current.is_some()
            && current == self.preview
            && self.preview_attempted
            && !snapshot.next_cover_pending;
        snapshot.cover = if current == self.preview {
            snapshot.next_cover.take()
        } else {
            None
        };
        snapshot.cover_pending = current.is_some() && !self.cover_attempted;
        snapshot.cover_revision += 1;
    }

    /// Preloads the retained next occurrence's cover with the same single bounded metadata job.
    pub fn preview(&mut self, next: Option<u64>, snapshot: &mut Snapshot) {
        if self.preview == next {
            return;
        }
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| !pending.cover || Some(pending.id) == self.preview)
        {
            self.pending = None;
        }
        self.preview = next;
        self.preview_attempted = false;
        snapshot.next_cover = None;
        snapshot.next_cover_pending = next.is_some();
        snapshot.next_cover_revision += 1;
    }

    /// Reopening an occurrence revalidates its source; repeated playback cannot reuse a replaced cover.
    pub fn reload(&mut self, snapshot: &mut Snapshot) {
        self.pending = None;
        self.cover_attempted = false;
        snapshot.cover = None;
        snapshot.cover_pending = self.current.is_some();
        snapshot.cover_revision += 1;
    }

    /// Hydrates small cached labels in one pass; stale removed occurrences cannot be restored.
    fn restore_cached(&mut self, queue: &Queue<PathBuf>, snapshot: &mut Snapshot) -> bool {
        if !self.warmed && !queue.entries().is_empty() {
            self.warmed = true;
            if let Some(config) = &self.persistent {
                let entries = queue
                    .entries()
                    .iter()
                    .map(|entry| (entry.id, entry.source.clone()))
                    .collect();
                self.warmup = Job::warm(entries, config.clone()).ok();
            }
        }
        if let Some(warmup) = &mut self.warmup {
            let Some(result) = warmup.poll() else {
                return false;
            };
            self.warmup = None;
            if let Ok(records) = result {
                let ids: HashSet<_> = queue.entries().iter().map(|entry| entry.id).collect();
                for (id, details) in records {
                    if ids.contains(&id) {
                        self.labels.insert(
                            id,
                            CachedMetadata {
                                title: details.title,
                                search_text: details.search_text,
                                duration_seconds: details.duration_seconds,
                            },
                        );
                    }
                }
                for (track, entry) in Arc::make_mut(&mut snapshot.tracks)
                    .iter_mut()
                    .zip(queue.entries())
                {
                    track.title = self.title(entry.id, &entry.source);
                    track.search_text = self.search_text(entry.id, &entry.source);
                    track.duration_seconds = self.duration_seconds(entry.id);
                }
                snapshot.queue_revision += 1;
            }
        }
        true
    }

    /// Polls one job and schedules current artwork before unprobed playlist rows; playback never waits.
    pub fn tick(&mut self, queue: &Queue<PathBuf>, snapshot: &mut Snapshot) {
        self.select(queue.current_id(), snapshot);
        if !self.restore_cached(queue, snapshot) {
            return;
        }
        if let Some(pending) = self.pending.as_mut() {
            let Some(result) = pending.job.poll() else {
                return;
            };
            let id = pending.id;
            let cover = pending.cover;
            self.pending = None;
            if let Some(index) = queue.index_of(id) {
                if cover && self.current == Some(id) {
                    snapshot.cover_pending = false;
                    snapshot.cover_revision += 1;
                }
                if cover && self.preview == Some(id) {
                    snapshot.next_cover_pending = false;
                    snapshot.next_cover_revision += 1;
                }
                match result {
                    Ok(details) => {
                        self.labels.insert(
                            id,
                            CachedMetadata {
                                title: details.title,
                                search_text: details.search_text,
                                duration_seconds: details.duration_seconds,
                            },
                        );
                        if cover && self.current == Some(id) {
                            snapshot.cover = details.cover.clone();
                        }
                        if cover && self.preview == Some(id) {
                            snapshot.next_cover = details.cover;
                        }
                    }
                    Err(()) => {
                        self.labels.insert(id, CachedMetadata::default());
                        if !self.warned {
                            self.warned = true;
                            eprintln!(
                                "warn: some track metadata unavailable; using display fallbacks"
                            );
                        }
                    }
                }
                let title = self.title(id, &queue.entries()[index].source);
                let search_text = self.search_text(id, &queue.entries()[index].source);
                let duration_seconds = self.duration_seconds(id);
                if let Some(track) = Arc::make_mut(&mut snapshot.tracks).get_mut(index)
                    && (track.title != title
                        || track.search_text != search_text
                        || track.duration_seconds != duration_seconds)
                {
                    track.title = title;
                    track.search_text = search_text;
                    track.duration_seconds = duration_seconds;
                    snapshot.queue_revision += 1;
                }
            }
        }
        let current = queue.current().filter(|_| !self.cover_attempted);
        let preview = self
            .preview
            .filter(|_| !self.preview_attempted)
            .and_then(|id| queue.index_of(id));
        let index = current.or(preview).or_else(|| {
            queue
                .entries()
                .iter()
                .position(|entry| !self.labels.contains_key(&entry.id))
        });
        let Some(index) = index else {
            return;
        };
        let entry = &queue.entries()[index];
        let cover = current.is_some() || preview == Some(index);
        if current.is_some() {
            self.cover_attempted = true;
        } else if preview == Some(index) {
            self.preview_attempted = true;
        }
        match Job::start_cached(entry.source.clone(), cover, self.persistent.clone()) {
            Ok(job) => {
                self.pending = Some(Pending {
                    id: entry.id,
                    cover,
                    job,
                })
            }
            Err(()) => {
                self.labels.entry(entry.id).or_default();
                if cover {
                    if current.is_some() {
                        snapshot.cover_pending = false;
                        snapshot.cover_revision += 1;
                    } else {
                        snapshot.next_cover_pending = false;
                        snapshot.next_cover_revision += 1;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A completed preview becomes the selected cover, while unfinished work remains pending.
    #[test]
    fn promotion_uses_only_completed_matching_preview() {
        let mut runtime = MetadataRuntime::default();
        let mut snapshot = Snapshot::default();
        runtime.select(Some(1), &mut snapshot);
        runtime.preview(Some(2), &mut snapshot);
        runtime.preview_attempted = true;
        let cover = Arc::new(crate::metadata::Cover {
            rgba: vec![0; 4].into(),
        });
        snapshot.next_cover = Some(Arc::clone(&cover));
        snapshot.next_cover_pending = false;

        runtime.select(Some(2), &mut snapshot);
        assert!(Arc::ptr_eq(snapshot.cover.as_ref().unwrap(), &cover));
        assert!(!snapshot.cover_pending);

        runtime.preview(Some(3), &mut snapshot);
        runtime.preview_attempted = true;
        runtime.select(Some(3), &mut snapshot);
        assert!(snapshot.cover.is_none());
        assert!(snapshot.cover_pending);

        runtime.preview(None, &mut snapshot);
        assert!(snapshot.next_cover.is_none());
        assert!(!snapshot.next_cover_pending);
    }
}
