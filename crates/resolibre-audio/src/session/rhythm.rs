//! One automatic current/next rhythm worker and a two-recording, identity-checked memory cache.

use super::Snapshot;
use crate::{LocalPlayer, original_audio::SourceKey, rhythm_analysis::RhythmJob};
use resolibre_core::{
    analysis::tempo::TempoReport,
    queue::{Advance, Queue, RepeatMode},
};
use std::{collections::VecDeque, fs::File, path::PathBuf, sync::Arc};

#[derive(Clone, Debug, Default)]
pub enum RhythmStatus {
    #[default]
    Detecting,
    Ready(Arc<TempoReport>),
    Unavailable,
}

#[derive(Clone, Debug)]
pub struct PreparedRhythm {
    pub queue_id: u64,
    pub report: Arc<TempoReport>,
}

struct Cached {
    id: u64,
    key: SourceKey,
    report: Arc<TempoReport>,
    whole: bool,
}

#[derive(Default)]
pub(super) struct RhythmRuntime {
    persistent: Option<Arc<crate::analysis_cache::CacheConfig>>,
    job: Option<RhythmJob>,
    target: u64,
    target_whole: bool,
    whole_attempted: Option<u64>,
    cancelled: bool,
    signature: Option<(u64, u64, bool, RepeatMode, bool)>,
    current: Option<u64>,
    current_key: Option<SourceKey>,
    next: Option<(u64, PathBuf)>,
    attempted: Vec<u64>,
    cache: VecDeque<Cached>,
}

impl RhythmRuntime {
    /// Shares explicit profile configuration without opening storage on the transport thread.
    pub(super) fn new(persistent: Option<Arc<crate::analysis_cache::CacheConfig>>) -> Self {
        Self {
            persistent,
            ..Self::default()
        }
    }

    /// Returns cached rhythm only for this retained file and the actually selected stream.
    pub(super) fn verified_report(
        &self,
        id: u64,
        player: &mut LocalPlayer,
    ) -> Option<Arc<TempoReport>> {
        let (_, key) = current_input(player).ok()?;
        self.cache
            .iter()
            .find(|entry| entry.id == id && entry.key == key)
            .map(|entry| Arc::clone(&entry.report))
    }

    /// A refreshed full analysis supersedes older in-memory BPM and beat observations for this occurrence.
    pub(super) fn invalidate(&mut self, id: u64, snapshot: &mut Snapshot) {
        self.cache.retain(|entry| entry.id != id);
        self.attempted.retain(|attempted| *attempted != id);
        if self.whole_attempted == Some(id) {
            self.whole_attempted = None;
        }
        if self.current == Some(id) {
            snapshot.rhythm = RhythmStatus::Detecting;
        }
        if snapshot
            .next_rhythm
            .as_ref()
            .is_some_and(|next| next.queue_id == id)
        {
            snapshot.next_rhythm = None;
        }
    }

    /// Selects the current recording before optional work; both its quick estimate and full timeline have priority.
    pub(super) fn current_priority(
        &mut self,
        queue: &Queue<PathBuf>,
        snapshot: &mut Snapshot,
    ) -> bool {
        self.select_targets(queue, snapshot);
        let Some(current) = self.current else {
            return false;
        };
        if self.job.is_some() && self.target == current && !self.cancelled {
            return true;
        }
        let cached = self
            .cache
            .iter()
            .find(|entry| entry.id == current && Some(entry.key) == self.current_key);
        if cached.is_none() && !self.attempted.contains(&current) {
            return true;
        }
        cached.is_some_and(|entry| !entry.whole) && self.whole_attempted != Some(current)
    }

    /// Reports native work still being reaped; full analysis waits rather than starting a second decoder.
    pub(super) fn busy(&self) -> bool {
        self.job.is_some()
    }

    /// Cancels on Stop/shutdown without joining on the transport thread.
    pub(super) fn cancel(&mut self) {
        if let Some(job) = &self.job {
            job.cancel();
        }
        self.cancelled = true;
    }

    /// Maintains actual queue order, rejects replaced results and prepares only current/next excerpts.
    pub(super) fn tick(
        &mut self,
        mut player: Option<&mut LocalPlayer>,
        queue: &Queue<PathBuf>,
        snapshot: &mut Snapshot,
        full_analysis_waiting: bool,
    ) {
        self.select_targets(queue, snapshot);
        if full_analysis_waiting {
            self.cancel();
        }
        if let Some(job) = self.job.as_mut()
            && let Some(result) = job.poll()
        {
            self.job = None;
            if self.cancelled {
                self.attempted.retain(|id| *id != self.target);
                if self.target_whole {
                    self.whole_attempted = None;
                }
            } else {
                match result {
                    Ok((key, report)) => {
                        self.cache.retain(|entry| entry.id != self.target);
                        self.cache.push_back(Cached {
                            id: self.target,
                            whole: self.target_whole,
                            key,
                            report: Arc::new(report),
                        });
                        while self.cache.len() > 2 {
                            self.cache.pop_front();
                        }
                    }
                    Err(()) => {
                        if self.current == Some(self.target) && !self.target_whole {
                            snapshot.rhythm = RhythmStatus::Unavailable;
                        }
                        eprintln!("warn: rhythm excerpt unavailable");
                    }
                }
            }
        }
        let Some(player) = player.as_mut() else {
            return;
        };
        if self.current_key.is_none() {
            self.current_key = current_input(player).ok().map(|(_, key)| key);
        }
        self.publish(snapshot);
        if self.busy() || full_analysis_waiting {
            return;
        }
        let Some(current) = self.current else {
            return;
        };
        let current_cached = self
            .cache
            .iter()
            .any(|entry| entry.id == current && Some(entry.key) == self.current_key);
        if !self.attempted.contains(&current) && !current_cached {
            let input = current_input(player);
            self.start(current, input, false);
            if self.job.is_none() {
                snapshot.rhythm = RhythmStatus::Unavailable;
            }
            return;
        }
        let whole_cached = self
            .cache
            .iter()
            .any(|entry| entry.id == current && Some(entry.key) == self.current_key && entry.whole);
        if current_cached && !whole_cached && self.whole_attempted != Some(current) {
            self.whole_attempted = Some(current);
            self.start(current, current_input(player), true);
            return;
        }
        if let Some((id, path)) = &self.next
            && !self.attempted.contains(id)
            && !self.cache.iter().any(|entry| entry.id == *id)
        {
            let file = crate::platform::open_media(path, false).map_err(|_| ());
            let input = file.and_then(|file| {
                let key = SourceKey::read(&file, u32::MAX).map_err(|_| ())?;
                Ok((file, key))
            });
            self.start(*id, input, false);
        }
    }

    /// Recomputes lookahead only when queue intent changes, using the core's real shuffle/repeat rules.
    fn select_targets(&mut self, queue: &Queue<PathBuf>, snapshot: &mut Snapshot) {
        let signature = (
            snapshot.generation,
            snapshot.queue_revision,
            queue.shuffle(),
            queue.repeat(),
            snapshot.mix_enabled,
        );
        if self.signature == Some(signature) {
            return;
        }
        let generation_changed = self
            .signature
            .is_none_or(|previous| previous.0 != snapshot.generation);
        if generation_changed {
            // An in-flight job belongs to its original generation. Even the same
            // queue occurrence may now refer to externally replaced source bytes.
            self.cancel();
            self.current_key = None;
            self.whole_attempted = None;
            snapshot.rhythm = RhythmStatus::Detecting;
        }
        if self
            .signature
            .is_some_and(|previous| previous.4 != snapshot.mix_enabled)
        {
            self.attempted.clear();
            self.whole_attempted = None;
            self.cancel();
        }
        self.signature = Some(signature);
        self.current = queue.current_id();
        let mut lookahead = queue.clone();
        self.next = lookahead
            .advance(Advance::Ended)
            .map(|index| {
                let entry = &lookahead.entries()[index];
                (entry.id, entry.source.clone())
            })
            .filter(|(id, _)| snapshot.mix_enabled && Some(*id) != self.current);
        if generation_changed {
            self.attempted.clear();
        }
        self.attempted.retain(|id| {
            Some(*id) == self.current || self.next.as_ref().is_some_and(|next| next.0 == *id)
        });
        if self.job.is_some() {
            if !self.attempted.contains(&self.target) {
                self.attempted.push(self.target);
            }
            if Some(self.target) != self.current
                && self.next.as_ref().map(|next| next.0) != Some(self.target)
            {
                self.cancel();
            }
        }
        snapshot.next_rhythm = None;
    }

    /// Reuses a current report only after matching its retained file and selected audio stream.
    fn publish(&self, snapshot: &mut Snapshot) {
        if let Some(entry) = self
            .cache
            .iter()
            .find(|entry| Some(entry.id) == self.current && Some(entry.key) == self.current_key)
        {
            snapshot.rhythm = RhythmStatus::Ready(Arc::clone(&entry.report));
        }
        snapshot.next_rhythm = self.next.as_ref().and_then(|next| {
            self.cache
                .iter()
                .find(|entry| entry.id == next.0)
                .map(|entry| PreparedRhythm {
                    queue_id: entry.id,
                    report: Arc::clone(&entry.report),
                })
        });
    }

    /// Records one attempt even on failure, preventing a bad source from becoming an unbounded retry loop.
    fn start(&mut self, id: u64, input: Result<(File, SourceKey), ()>, whole: bool) {
        if !self.attempted.contains(&id) {
            self.attempted.push(id);
        }
        self.target = id;
        self.target_whole = whole;
        self.cancelled = false;
        self.job = input
            .and_then(|(file, key)| RhythmJob::start(file, key, whole, self.persistent.clone()))
            .ok();
        if self.job.is_none() {
            eprintln!("warn: rhythm excerpt unavailable");
        }
    }
}

/// Gets source identity from the active retained file, never from a filename extension.
fn current_input(player: &mut LocalPlayer) -> Result<(File, SourceKey), ()> {
    let file = player.analysis_input().map_err(|_| ())?;
    let stream = player.audio_stream_index().map_err(|_| ())?;
    let key = SourceKey::read(&file, stream).map_err(|_| ())?;
    Ok((file, key))
}
