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

/// Latest full-analysis attempt in this library session; BPM uncertainty is not a failed analysis.
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
        if let Some(active) = &mut self.active
            && !active.cancelled
        {
            active.job.cancel();
            active.cancelled = true;
            if !active.request.foreground {
                self.pending.push_front(active.request);
                mark(snapshot, active.request.id, LibraryAnalysisStatus::Queued);
            } else {
                Arc::make_mut(&mut snapshot.library_analysis).remove(&active.request.id);
            }
        }
        self.pending.retain(|request| {
            if request.foreground {
                Arc::make_mut(&mut snapshot.library_analysis).remove(&request.id);
            }
            !request.foreground
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
    }

    /// Promotes a current-track request; explicit Analyze replaces even a completed stored report.
    pub(super) fn request(&mut self, queue: &Queue<PathBuf>, snapshot: &mut Snapshot, force: bool) {
        let Some(id) = queue.current_id() else {
            return;
        };
        if !force && matches!(snapshot.analysis, AnalysisStatus::Complete(_)) {
            return;
        }
        self.enqueue(id, true, force, snapshot);
        snapshot.analysis = AnalysisStatus::Running {
            decoded_seconds: 0.0,
        };
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
            self.pending.remove(index);
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
        if snapshot.library_analysis.keys().any(|id| !ids.contains(id)) {
            Arc::make_mut(&mut snapshot.library_analysis).retain(|id, _| ids.contains(id));
        }
        if let Some(active) = &mut self.active
            && !ids.contains(&active.request.id)
        {
            active.job.cancel();
            active.cancelled = true;
        }
    }

    /// Publishes only the selected generation's plot; other jobs retain just a small row status in memory.
    fn poll(&mut self, snapshot: &mut Snapshot) {
        let Some(active) = &mut self.active else {
            return;
        };
        let current = active.generation == Some(snapshot.generation);
        let Some(result) = active.job.poll() else {
            if current && !active.cancelled {
                snapshot.analysis = AnalysisStatus::Running {
                    decoded_seconds: active.job.progress(),
                };
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
                if current {
                    snapshot.analysis = AnalysisStatus::Complete(Arc::new(report));
                }
            }
            Err(_) => {
                mark(snapshot, active.request.id, LibraryAnalysisStatus::Failed);
                if current {
                    snapshot.analysis = AnalysisStatus::Unavailable;
                }
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
        if request.foreground && player.is_none() {
            return;
        }
        self.pending.pop_front();
        let Some(index) = queue.index_of(request.id) else {
            return;
        };
        let selected = queue.current_id() == Some(request.id) && player.is_some();
        let generation = if selected {
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
                if selected {
                    snapshot.analysis = AnalysisStatus::Unavailable;
                }
                eprintln!("warn: track analysis unavailable");
            }
        }
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
        assert_eq!(runtime.pending.len(), resolibre_core::queue::MAX_QUEUE_ITEMS);
        assert!(
            !runtime.wants_worker(),
            "background requests must yield to automatic rhythm"
        );
        runtime.request(&queue, &mut snapshot, true);
        assert!(
            runtime.wants_worker(),
            "explicit current analysis can interrupt automatic rhythm"
        );
        assert_eq!(runtime.pending.len(), resolibre_core::queue::MAX_QUEUE_ITEMS);
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
        assert_eq!(runtime.pending.len(), resolibre_core::queue::MAX_QUEUE_ITEMS);
    }
}
