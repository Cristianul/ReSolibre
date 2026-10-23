//! Bounded full-analysis scheduling; stable queue IDs isolate library work from playback navigation.

use super::Snapshot;
use crate::{
    LocalPlayer,
    original_audio::SourceKey,
    track_analysis::{TrackInput, TrackJob},
};
use resolibre_core::queue::Queue;
use std::{collections::VecDeque, path::PathBuf, sync::Arc};

#[derive(Clone, Debug, Default)]
pub enum AnalysisStatus {
    #[default]
    Idle,
    Running {
        decoded_seconds: f64,
    },
    Complete(Arc<resolibre_core::analysis::TrackAnalysis>),
    Cancelled,
    Unavailable,
}

/// One explicitly inspected library occurrence, independent of the current playback generation.
#[derive(Clone, Debug)]
pub struct AnalysisInspection {
    pub id: u64,
    pub status: AnalysisStatus,
}

/// Restored full result or latest session attempt; BPM uncertainty is not a failed analysis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LibraryAnalysisStatus {
    Queued,
    Running,
    Complete,
    Failed,
}

#[derive(Clone, Copy)]
struct Request {
    id: u64,
    foreground: bool,
    force: bool,
}

struct Active {
    request: Request,
    generation: Option<u64>,
    cancelled: bool,
    job: TrackJob,
}

#[derive(Default)]
pub(super) struct AnalysisRuntime {
    persistent: Option<Arc<crate::analysis_cache::CacheConfig>>,
    active: Option<Active>,
    pending: VecDeque<Request>,
    completed: Option<u64>,
    queue_revision: Option<u64>,
    cache_status: Option<crate::analysis_cache::status::StatusJob>,
    cache_checked: std::collections::HashSet<u64>,
    requested: std::collections::HashSet<u64>,
}

impl AnalysisRuntime {
    /// Shares profile configuration without opening storage on the transport thread.
    pub(super) fn new(persistent: Option<Arc<crate::analysis_cache::CacheConfig>>) -> Self {
        Self {
            persistent,
            ..Self::default()
        }
    }

    /// Transfers one successful completion so automatic rhythm can discard older in-memory estimates.
    pub(super) fn take_completed(&mut self) -> Option<u64> {
        self.completed.take()
    }

    /// Library jobs yield between tracks to automatic current/next rhythm preparation.
    pub(super) fn wants_worker(&self) -> bool {
        self.active.is_some()
            || self
                .pending
                .front()
                .is_some_and(|request| request.foreground)
    }

    /// Releases the decoder for current-track rhythm and retains the exact request for later resumption.
    pub(super) fn yield_to_current(&mut self, snapshot: &mut Snapshot) {
        if let Some(active) = &mut self.active
            && !active.cancelled
        {
            active.job.cancel();
            active.cancelled = true;
            self.pending.push_front(active.request);
            mark(snapshot, active.request.id, LibraryAnalysisStatus::Queued);
        }
    }

    /// Includes cancelled work until its native decoder has actually exited.
    pub(super) fn has_worker(&self) -> bool {
        self.active.is_some()
    }

    /// Promotes a requested current-track plot ahead of other library work, including an in-flight background job.
    pub(super) fn prioritize_current(&mut self, queue: &Queue<PathBuf>, snapshot: &mut Snapshot) {
        let Some(current) = queue.current_id() else {
            return;
        };
        if !self.pending.iter().any(|request| request.id == current) {
            return;
        }
        if self
            .active
            .as_ref()
            .is_some_and(|active| active.request.id != current)
        {
            self.yield_to_current(snapshot);
        }
        if let Some(index) = self
            .pending
            .iter()
            .position(|request| request.id == current)
            && let Some(request) = self.pending.remove(index)
        {
            self.pending.push_front(request);
        }
    }

    /// Navigation releases the decoder for rhythm preparation, then resumes library work.
    pub(super) fn reset(&mut self, snapshot: &mut Snapshot) {
        let inspected = snapshot.inspection.as_ref().map(|view| view.id);
        if let Some(active) = &mut self.active
            && !active.cancelled
        {
            active.job.cancel();
            active.cancelled = true;
            if !active.request.foreground || inspected == Some(active.request.id) {
                self.pending.push_front(active.request);
                mark(snapshot, active.request.id, LibraryAnalysisStatus::Queued);
            } else {
                Arc::make_mut(&mut snapshot.library_analysis).remove(&active.request.id);
            }
        }
        self.pending.retain(|request| {
            if request.foreground && inspected != Some(request.id) {
                Arc::make_mut(&mut snapshot.library_analysis).remove(&request.id);
            }
            !request.foreground || inspected == Some(request.id)
        });
        snapshot.analysis = AnalysisStatus::Idle;
    }

    /// Stop and explicit batch cancellation remove pending work without waiting for the decoder.
    pub(super) fn cancel(&mut self, snapshot: &mut Snapshot) {
        if let Some(active) = &mut self.active {
            active.job.cancel();
            active.cancelled = true;
        }
        self.pending.clear();
        Arc::make_mut(&mut snapshot.library_analysis).retain(|_, status| {
            matches!(
                status,
                LibraryAnalysisStatus::Complete | LibraryAnalysisStatus::Failed
            )
        });
        if !matches!(snapshot.analysis, AnalysisStatus::Complete(_)) {
            snapshot.analysis = AnalysisStatus::Cancelled;
        }
        cancel_inspection(snapshot);
    }

    /// Closing the diagnostics view cancels only its explicit request, leaving a library batch running.
    pub(super) fn cancel_view(&mut self, snapshot: &mut Snapshot) {
        self.pending.retain(|request| {
            if request.foreground {
                Arc::make_mut(&mut snapshot.library_analysis).remove(&request.id);
            }
            !request.foreground
        });
        if let Some(active) = &mut self.active
            && active.request.foreground
        {
            active.job.cancel();
            active.cancelled = true;
            Arc::make_mut(&mut snapshot.library_analysis).remove(&active.request.id);
        }
        if !matches!(snapshot.analysis, AnalysisStatus::Complete(_)) {
            snapshot.analysis = AnalysisStatus::Cancelled;
        }
        cancel_inspection(snapshot);
    }

    /// Promotes a current-track request; explicit Analyze replaces even a completed stored report.
    pub(super) fn request(&mut self, queue: &Queue<PathBuf>, snapshot: &mut Snapshot, force: bool) {
        let Some(id) = queue.current_id() else {
            return;
        };
        if snapshot
            .inspection
            .as_ref()
            .is_some_and(|view| view.id != id)
        {
            self.cancel_view(snapshot);
        }
        snapshot.inspection = None;
        if !force && matches!(snapshot.analysis, AnalysisStatus::Complete(_)) {
            return;
        }
        self.enqueue(id, true, force, snapshot);
        snapshot.analysis = AnalysisStatus::Running {
            decoded_seconds: 0.0,
        };
    }

    /// Opens a concrete library result without selecting or playing it; explicit retries bypass the cache.
    pub(super) fn inspect(
        &mut self,
        queue: &Queue<PathBuf>,
        snapshot: &mut Snapshot,
        id: u64,
        force: bool,
    ) {
        if queue.index_of(id).is_none() {
            return;
        }
        if !force
            && snapshot.inspection.as_ref().is_some_and(|view| {
                view.id == id && matches!(view.status, AnalysisStatus::Complete(_))
            })
        {
            return;
        }
        let same_target = match &snapshot.inspection {
            Some(view) => view.id == id,
            None => queue.current_id() == Some(id),
        };
        if !same_target {
            self.cancel_view(snapshot);
        }
        let status = if !force
            && queue.current_id() == Some(id)
            && matches!(snapshot.analysis, AnalysisStatus::Complete(_))
        {
            snapshot.analysis.clone()
        } else {
            self.enqueue(id, true, force, snapshot);
            AnalysisStatus::Running {
                decoded_seconds: 0.0,
            }
        };
        snapshot.inspection = Some(AnalysisInspection { id, status });
    }

    /// Captures one existing occurrence, independent of the current playback selection.
    pub(super) fn request_track(
        &mut self,
        queue: &Queue<PathBuf>,
        snapshot: &mut Snapshot,
        id: u64,
    ) {
        if queue.index_of(id).is_some() {
            self.enqueue(id, false, true, snapshot);
        }
    }

    /// Captures the entire bounded library, including rows hidden by presentation search.
    pub(super) fn request_all(&mut self, queue: &Queue<PathBuf>, snapshot: &mut Snapshot) {
        for entry in queue.entries() {
            self.enqueue(entry.id, false, false, snapshot);
        }
    }

    /// Coalesces repeated clicks; stable IDs bound pending work to the library's maximum size.
    fn enqueue(&mut self, id: u64, foreground: bool, force: bool, snapshot: &mut Snapshot) {
        self.requested.insert(id);
        let mut force = force;
        if let Some(active) = &mut self.active
            && active.request.id == id
            && !active.cancelled
        {
            if !force || active.request.force {
                return;
            }
            // A forced retry must not accidentally reuse a cache lookup already in flight.
            active.job.cancel();
            active.cancelled = true;
        }
        if let Some(index) = self.pending.iter().position(|request| request.id == id) {
            if !foreground {
                return;
            }
            if let Some(previous) = self.pending.remove(index) {
                // Opening the same badge must not downgrade an explicit fresh retry to a cache read.
                force |= previous.force;
            }
        }
        let request = Request {
            id,
            foreground,
            force,
        };
        if foreground {
            self.pending.push_front(request);
        } else {
            self.pending.push_back(request);
        }
        mark(snapshot, id, LibraryAnalysisStatus::Queued);
    }

    /// Reaps one job and starts the next only after automatic rhythm has released its decoder.
    pub(super) fn tick(
        &mut self,
        player: Option<&mut LocalPlayer>,
        queue: &Queue<PathBuf>,
        snapshot: &mut Snapshot,
        available: bool,
    ) {
        self.reconcile_queue(queue, snapshot);
        self.restore_cached(queue, snapshot);
        let was_active = self.active.is_some();
        self.poll(snapshot);
        // Leave one scheduling turn between files for current/next rhythm and playback changes.
        if available && self.active.is_none() && !was_active {
            self.start_next(player, queue, snapshot);
        }
        snapshot.analysis_pending = self.pending.len()
            + usize::from(self.active.as_ref().is_some_and(|active| !active.cancelled));
    }

    /// Prunes removed occurrences only on queue changes, avoiding quadratic work on every playback tick.
    fn reconcile_queue(&mut self, queue: &Queue<PathBuf>, snapshot: &mut Snapshot) {
        if self.queue_revision == Some(snapshot.queue_revision) {
            return;
        }
        self.queue_revision = Some(snapshot.queue_revision);
        let ids: std::collections::BTreeSet<_> =
            queue.entries().iter().map(|entry| entry.id).collect();
        self.pending.retain(|request| ids.contains(&request.id));
        self.cache_checked.retain(|id| ids.contains(id));
        self.requested.retain(|id| ids.contains(id));
        if snapshot.library_analysis.keys().any(|id| !ids.contains(id)) {
            Arc::make_mut(&mut snapshot.library_analysis).retain(|id, _| ids.contains(id));
        }
        if let Some(view) = &mut snapshot.inspection
            && !ids.contains(&view.id)
        {
            view.status = AnalysisStatus::Unavailable;
        }
        if let Some(active) = &mut self.active
            && !ids.contains(&active.request.id)
        {
            active.job.cancel();
            active.cancelled = true;
        }
    }

    /// Restores cached badges independently of decoding; fresh attempts and removed IDs always win.
    fn restore_cached(&mut self, queue: &Queue<PathBuf>, snapshot: &mut Snapshot) {
        if let Some(job) = &mut self.cache_status {
            let restored = job.poll();
            if job.finished() {
                self.cache_status = None;
            }
            self.restore_marks(restored, queue, snapshot);
        }
        let Some(config) = &self.persistent else {
            return;
        };
        if self.cache_status.is_some() || self.cache_checked.len() == queue.entries().len() {
            return;
        }
        let entries = queue
            .entries()
            .iter()
            .filter(|entry| self.cache_checked.insert(entry.id))
            .map(|entry| (entry.id, entry.source.clone()))
            .collect();
        match crate::analysis_cache::status::StatusJob::start(entries, config.clone()) {
            Ok(job) => self.cache_status = Some(job),
            Err(()) => config.warn(),
        }
    }

    /// Merges observations without reviving removed rows or replacing newer attempts, including cancellations.
    fn restore_marks(&self, restored: Vec<u64>, queue: &Queue<PathBuf>, snapshot: &mut Snapshot) {
        for id in restored {
            if queue.index_of(id).is_some() && !self.requested.contains(&id) {
                Arc::make_mut(&mut snapshot.library_analysis)
                    .entry(id)
                    .or_insert(LibraryAnalysisStatus::Complete);
            }
        }
    }

    /// Publishes plots only for current playback and explicit inspection; other rows retain small statuses.
    fn poll(&mut self, snapshot: &mut Snapshot) {
        let Some(active) = &mut self.active else {
            return;
        };
        let current = active.generation == Some(snapshot.generation);
        let Some(result) = active.job.poll() else {
            if !active.cancelled {
                let status = AnalysisStatus::Running {
                    decoded_seconds: active.job.progress(),
                };
                if current {
                    snapshot.analysis = status.clone();
                }
                update_inspection(snapshot, active.request.id, status);
            }
            return;
        };
        let Some(active) = self.active.take() else {
            return;
        };
        if active.cancelled {
            return;
        }
        match result {
            Ok(report) => {
                self.completed = Some(active.request.id);
                mark(snapshot, active.request.id, LibraryAnalysisStatus::Complete);
                let status = AnalysisStatus::Complete(Arc::new(report));
                if current {
                    snapshot.analysis = status.clone();
                }
                update_inspection(snapshot, active.request.id, status);
            }
            Err(_) => {
                mark(snapshot, active.request.id, LibraryAnalysisStatus::Failed);
                if current {
                    snapshot.analysis = AnalysisStatus::Unavailable;
                }
                update_inspection(snapshot, active.request.id, AnalysisStatus::Unavailable);
                eprintln!("warn: track analysis unavailable");
            }
        }
    }

    /// Uses the retained selected stream when available; off-track files open inside the worker.
    fn start_next(
        &mut self,
        player: Option<&mut LocalPlayer>,
        queue: &Queue<PathBuf>,
        snapshot: &mut Snapshot,
    ) {
        let Some(request) = self.pending.front().copied() else {
            return;
        };
        self.pending.pop_front();
        let Some(index) = queue.index_of(request.id) else {
            return;
        };
        let selected = queue.current_id() == Some(request.id) && player.is_some();
        let generation = if queue.current_id() == Some(request.id) {
            Some(snapshot.generation)
        } else {
            None
        };
        let result = if selected {
            player
                .ok_or(())
                .and_then(|player| start_selected(player, self.persistent.clone(), request.force))
        } else {
            TrackJob::start_input(
                TrackInput::Path(queue.entries()[index].source.clone()),
                self.persistent.clone(),
                request.force,
            )
            .map_err(|_| ())
        };
        match result {
            Ok(job) => {
                mark(snapshot, request.id, LibraryAnalysisStatus::Running);
                self.active = Some(Active {
                    request,
                    generation,
                    cancelled: false,
                    job,
                });
            }
            Err(()) => {
                mark(snapshot, request.id, LibraryAnalysisStatus::Failed);
                if generation.is_some() {
                    snapshot.analysis = AnalysisStatus::Unavailable;
                }
                update_inspection(snapshot, request.id, AnalysisStatus::Unavailable);
                eprintln!("warn: track analysis unavailable");
            }
        }
    }
}

/// Only the requested occurrence may update the inspected plot; shared reports keep memory bounded.
fn update_inspection(snapshot: &mut Snapshot, id: u64, status: AnalysisStatus) {
    if let Some(view) = &mut snapshot.inspection
        && view.id == id
        && matches!(view.status, AnalysisStatus::Running { .. })
    {
        view.status = status;
    }
}

/// Cancels unfinished inspection while preserving a completed result for review.
fn cancel_inspection(snapshot: &mut Snapshot) {
    if let Some(view) = &mut snapshot.inspection
        && !matches!(view.status, AnalysisStatus::Complete(_))
    {
        view.status = AnalysisStatus::Cancelled;
    }
}

/// Mutates small row observations only on lifecycle changes, independently of queue ordering.
fn mark(snapshot: &mut Snapshot, id: u64, status: LibraryAnalysisStatus) {
    Arc::make_mut(&mut snapshot.library_analysis).insert(id, status);
}

/// Borrows the retained selected stream without reopening its source path.
fn start_selected(
    player: &mut LocalPlayer,
    persistent: Option<Arc<crate::analysis_cache::CacheConfig>>,
    force: bool,
) -> Result<TrackJob, ()> {
    let file = player.analysis_input().map_err(|_| ())?;
    let stream = player.audio_stream_index().map_err(|_| ())?;
    let key = SourceKey::read(&file, stream).map_err(|_| ())?;
    if force {
        TrackJob::start_input(TrackInput::Retained(file, key), persistent, true).map_err(|_| ())
    } else {
        TrackJob::start(file, key, persistent).map_err(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reopening a badge coalesces fresh retries, and only that inspection survives navigation.
    #[test]
    fn inspection_keeps_retry_intent_and_survives_navigation() {
        let mut queue = Queue::with_seed(42);
        queue.append(vec![
            PathBuf::from("first.wav"),
            PathBuf::from("second.wav"),
        ]);
        queue.select(0);
        let id = queue.entries()[1].id;
        let mut snapshot = Snapshot::default();
        let mut runtime = AnalysisRuntime::default();
        runtime.request_all(&queue, &mut snapshot);
        runtime.inspect(&queue, &mut snapshot, id, true);
        runtime.inspect(&queue, &mut snapshot, id, false);
        assert_eq!(runtime.pending.len(), 2);
        let pending = runtime.pending.front().unwrap();
        assert_eq!(pending.id, id);
        assert!(pending.foreground && pending.force);

        runtime.reset(&mut snapshot);
        assert_eq!(runtime.pending.len(), 2);
        assert!(matches!(
            snapshot.inspection.as_ref().unwrap().status,
            AnalysisStatus::Running { .. }
        ));

        runtime.request(&queue, &mut snapshot, false);
        assert!(snapshot.inspection.is_none());
        assert_eq!(runtime.pending.len(), 1);
        assert_eq!(
            runtime.pending.front().unwrap().id,
            queue.current_id().unwrap()
        );
    }

    /// Late cached observations cannot overwrite attempts or reintroduce removed/cancelled occurrences.
    #[test]
    fn cached_marks_respect_newer_attempts_and_removed_rows() {
        let mut queue = Queue::with_seed(42);
        queue.append(
            (0..4)
                .map(|index| PathBuf::from(format!("track-{index}.wav")))
                .collect(),
        );
        let ids: Vec<_> = queue.entries().iter().map(|entry| entry.id).collect();
        let mut snapshot = Snapshot::default();
        let mut runtime = AnalysisRuntime::default();
        runtime.request_track(&queue, &mut snapshot, ids[0]);
        runtime.restore_marks(vec![ids[0], ids[1], u64::MAX], &queue, &mut snapshot);
        assert_eq!(
            snapshot.library_analysis[&ids[0]],
            LibraryAnalysisStatus::Queued
        );
        assert_eq!(
            snapshot.library_analysis[&ids[1]],
            LibraryAnalysisStatus::Complete
        );
        assert!(!snapshot.library_analysis.contains_key(&u64::MAX));
        runtime.cancel(&mut snapshot);
        runtime.restore_marks(vec![ids[0]], &queue, &mut snapshot);
        assert!(!snapshot.library_analysis.contains_key(&ids[0]));
        mark(&mut snapshot, ids[2], LibraryAnalysisStatus::Failed);
        mark(&mut snapshot, ids[3], LibraryAnalysisStatus::Running);
        runtime.restore_marks(ids.clone(), &queue, &mut snapshot);
        assert_eq!(
            snapshot.library_analysis[&ids[2]],
            LibraryAnalysisStatus::Failed
        );
        assert_eq!(
            snapshot.library_analysis[&ids[3]],
            LibraryAnalysisStatus::Running
        );
    }

    /// Library scheduling is bounded by stable IDs, while view promotion/cancellation leaves unrelated work intact.
    #[test]
    fn batch_coalesces_and_yields_to_rhythm() {
        let mut queue = Queue::with_seed(42);
        assert!(
            queue.append(
                (0..resolibre_core::queue::MAX_QUEUE_ITEMS)
                    .map(|index| PathBuf::from(format!("track-{index}.wav")))
                    .collect()
            )
        );
        queue.select(0);
        let mut snapshot = Snapshot::default();
        let mut runtime = AnalysisRuntime::default();
        runtime.request_all(&queue, &mut snapshot);
        runtime.request_all(&queue, &mut snapshot);
        assert!(runtime.pending.iter().all(|request| !request.force));
        assert_eq!(
            runtime.pending.len(),
            resolibre_core::queue::MAX_QUEUE_ITEMS
        );
        assert!(
            !runtime.wants_worker(),
            "background requests must yield to automatic rhythm"
        );
        runtime.request(&queue, &mut snapshot, true);
        assert!(
            runtime.wants_worker(),
            "explicit current analysis can interrupt automatic rhythm"
        );
        assert_eq!(
            runtime.pending.len(),
            resolibre_core::queue::MAX_QUEUE_ITEMS
        );
        runtime.cancel_view(&mut snapshot);
        assert_eq!(
            runtime.pending.len(),
            resolibre_core::queue::MAX_QUEUE_ITEMS - 1
        );
        assert!(
            !snapshot
                .library_analysis
                .contains_key(&queue.current_id().unwrap())
        );
        assert!(!runtime.wants_worker());
        runtime.cancel(&mut snapshot);
        assert!(runtime.pending.is_empty());
        assert!(snapshot.library_analysis.is_empty());
        queue.select(10);
        runtime.request_all(&queue, &mut snapshot);
        runtime.prioritize_current(&queue, &mut snapshot);
        assert_eq!(
            runtime.pending.front().unwrap().id,
            queue.current_id().unwrap()
        );
        assert_eq!(
            runtime.pending.len(),
            resolibre_core::queue::MAX_QUEUE_ITEMS
        );
    }
}
