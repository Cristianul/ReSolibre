//! Low-priority cache-only badge recovery; at most one bounded identity read runs at a time.

use super::{LibraryAnalysisStatus, Snapshot};
use crate::analysis_cache::CacheConfig;
use resolibre_core::queue::Queue;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread::{self, JoinHandle};

struct Lookup {
    id: u64,
    cancel: Arc<AtomicBool>,
    thread: Option<JoinHandle<bool>>,
}

impl Drop for Lookup {
    /// Reaps the owned reader on shutdown; ordinary preemption only signals cancellation.
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[derive(Default)]
pub(super) struct Restoration {
    checked: BTreeSet<u64>,
    ids: BTreeSet<u64>,
    exhausted: bool,
    pending: Option<Lookup>,
    warned: bool,
}

impl Restoration {
    /// Removed occurrences cannot receive late marks; bookkeeping stays within the queue bound.
    pub(super) fn retain(&mut self, ids: &BTreeSet<u64>) {
        if &self.ids == ids {
            return;
        }
        self.ids = ids.clone();
        self.exhausted = false;
        self.checked.retain(|id| ids.contains(id));
        if let Some(pending) = &self.pending
            && !ids.contains(&pending.id)
        {
            pending.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// Reads only stored reports, yielding to rhythm/full analysis and preserving newer UI attempts.
    pub(super) fn tick(
        &mut self,
        queue: &Queue<PathBuf>,
        snapshot: &mut Snapshot,
        config: Option<&Arc<CacheConfig>>,
        available: bool,
    ) {
        let Some(config) = config else {
            return;
        };
        if let Some(pending) = &self.pending {
            if !available {
                pending.cancel.store(true, Ordering::Relaxed);
            }
            if !pending
                .thread
                .as_ref()
                .is_some_and(|thread| thread.is_finished())
            {
                return;
            }
            let Some(mut pending) = self.pending.take() else {
                return;
            };
            let complete = pending
                .thread
                .take()
                .and_then(|thread| thread.join().ok())
                .unwrap_or(false);
            if !pending.cancel.load(Ordering::Relaxed) && queue.index_of(pending.id).is_some() {
                self.checked.insert(pending.id);
                if complete {
                    Arc::make_mut(&mut snapshot.library_analysis)
                        .entry(pending.id)
                        .or_insert(LibraryAnalysisStatus::Complete);
                }
            }
        }
        if !available || self.exhausted {
            return;
        }
        let Some(entry) = queue.entries().iter().find(|entry| {
            !self.checked.contains(&entry.id) && !snapshot.library_analysis.contains_key(&entry.id)
        }) else {
            self.exhausted = true;
            return;
        };
        let id = entry.id;
        let path = entry.source.clone();
        let config = config.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        match thread::Builder::new()
            .name("resolibre-analysis-marks".into())
            .spawn(move || config.restore_mark(&path, &worker_cancel))
        {
            Ok(thread) => {
                self.pending = Some(Lookup {
                    id,
                    cancel,
                    thread: Some(thread),
                })
            }
            Err(_) => {
                self.checked.insert(id);
                if !self.warned {
                    self.warned = true;
                    eprintln!("warn: cannot restore analysis indicator");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Supplies a completed reader deterministically without touching user media or storage.
    fn completed(id: u64) -> Lookup {
        let thread = thread::spawn(|| true);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while !thread.is_finished() {
            assert!(std::time::Instant::now() < deadline);
            thread::sleep(std::time::Duration::from_millis(1));
        }
        Lookup {
            id,
            cancel: Arc::new(AtomicBool::new(false)),
            thread: Some(thread),
        }
    }

    /// Late cache hits cannot replace new attempts, restore deleted rows or survive preemption.
    #[test]
    fn late_results_respect_identity_attempts_and_cancellation() {
        let config = Arc::new(CacheConfig::new(PathBuf::new()));
        for status in [LibraryAnalysisStatus::Queued, LibraryAnalysisStatus::Failed] {
            let mut queue = Queue::with_seed(1);
            queue.append(vec![PathBuf::from("/unused.wav")]);
            let id = queue.entries()[0].id;
            let mut snapshot = Snapshot::default();
            Arc::make_mut(&mut snapshot.library_analysis).insert(id, status);
            let mut restore = Restoration {
                pending: Some(completed(id)),
                ..Default::default()
            };
            restore.tick(&queue, &mut snapshot, Some(&config), true);
            assert_eq!(snapshot.library_analysis.get(&id), Some(&status));
        }
        let queue = Queue::with_seed(1);
        let mut snapshot = Snapshot::default();
        let mut restore = Restoration {
            pending: Some(completed(7)),
            ..Default::default()
        };
        restore.retain(&BTreeSet::new());
        restore.tick(&queue, &mut snapshot, Some(&config), true);
        assert!(snapshot.library_analysis.is_empty());
        let mut queue = Queue::with_seed(1);
        queue.append(vec![PathBuf::from("/unused.wav")]);
        let id = queue.entries()[0].id;
        let mut restore = Restoration {
            pending: Some(completed(id)),
            ..Default::default()
        };
        restore.tick(&queue, &mut snapshot, Some(&config), false);
        assert!(snapshot.library_analysis.is_empty());
        assert!(
            !restore.checked.contains(&id),
            "preempted work remains retryable"
        );
    }
}
