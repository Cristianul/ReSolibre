//! Single-owner playback loop; the portable core decides queue navigation.

use super::{Command, MAX_SELECTION_BYTES, Request, Shared, Snapshot, TrackLabel};
use crate::{AudioOutput, LocalPlayer, PlaybackError};
use resolibre_core::queue::{Advance, Queue, QueueAction};
use resolibre_core::session::SessionState;
use resolibre_core::volume_glide::VolumeGlide;
use resolibre_core::{PlaybackEvent, SeekPosition};
use std::collections::hash_map::RandomState;
use std::hash::BuildHasher;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

mod mixing;

struct Worker {
    shared: Arc<Shared>,
    output: AudioOutput,
    queue: Queue<PathBuf>,
    player: Option<LocalPlayer>,
    edges: super::edges::EdgeRuntime,
    analysis: super::analysis::AnalysisRuntime,
    rhythm: super::rhythm::RhythmRuntime,
    mixing: mixing::MixRuntime,
    metadata: super::metadata::MetadataRuntime,
    snapshot: Snapshot,
    paused: bool,
    loaded: bool,
    previous_restart_requested: Option<Instant>,
    amplitude: f64,
    volume_glide: Option<VolumeGlide>,
    volume_tick: Instant,
    audio_wait_started: Option<Instant>,
    last_tick: Instant,
    info_refresh: Option<Instant>,
    info_warning_logged: bool,
    spectrum_failed: bool,
}

/// Runs until cancellation, then drops native state before publishing completion.
pub(super) fn run(
    shared: Arc<Shared>,
    output: AudioOutput,
    cache: Option<Arc<crate::analysis_cache::CacheConfig>>,
) {
    let mut worker = Worker {
        shared,
        output,
        queue: Queue::with_seed(RandomState::new().hash_one("ReSolibre playback order")),
        player: None,
        edges: super::edges::EdgeRuntime::default(),
        analysis: super::analysis::AnalysisRuntime::new(cache.clone()),
        rhythm: super::rhythm::RhythmRuntime::new(cache.clone()),
        mixing: mixing::MixRuntime::default(),
        metadata: super::metadata::MetadataRuntime::new(cache),
        snapshot: Snapshot::default(),
        paused: false,
        loaded: false,
        previous_restart_requested: None,
        amplitude: 0.25,
        volume_glide: None,
        volume_tick: Instant::now(),
        audio_wait_started: Some(Instant::now()),
        last_tick: Instant::now(),
        info_refresh: None,
        info_warning_logged: false,
        spectrum_failed: false,
    };
    loop {
        let request = {
            let mut mailbox = worker
                .shared
                .mailbox
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if mailbox.shutdown {
                break;
            }
            mailbox.requests.pop_front()
        };
        if let Some(request) = request {
            worker.handle(request);
        }
        // A stream of pointer commands must not starve native EOF/error events.
        if let Err(error) = worker.tick() {
            worker.fail(error.to_string());
        }
        worker.tick_analysis();
        worker.tick_mix();
        worker.publish();
        let mailbox = worker
            .shared
            .mailbox
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if mailbox.shutdown {
            break;
        }
        if mailbox.requests.is_empty() {
            // The condvar wakes immediately for controls; no 100 ms native event wait.
            let _wait = worker
                .shared
                .wake
                .wait_timeout(mailbox, Duration::from_millis(10));
        }
    }
    worker.analysis.reset(&mut worker.snapshot);
    worker.analysis = super::analysis::AnalysisRuntime::default();
    worker.rhythm = super::rhythm::RhythmRuntime::default();
    worker.edges.reset();
    worker.metadata = super::metadata::MetadataRuntime::default();
    worker.mixing = mixing::MixRuntime::default();
    worker.player = None;
    worker.snapshot.scanning = None;
    worker.snapshot.finished = true;
    worker.publish();
}

impl Worker {
    /// Makes one observation available without accumulating a stream of stale updates.
    fn publish(&mut self) {
        if self.player.is_none() {
            self.snapshot.stream_info = None;
            self.snapshot.live_spectrum = None;
        }
        self.snapshot.mix_region = if self.snapshot.mix_enabled {
            self.mixing.region()
        } else {
            None
        };
        self.snapshot.mix_beat_ratio = if self.snapshot.mix_enabled {
            self.mixing.beat_ratio()
        } else {
            None
        };
        self.snapshot.trim_start = self.edges.region.map_or(0.0, |region| region.start);
        self.snapshot.trim_end = self.edges.region.map(|region| region.end);
        self.snapshot.saved.current = self.queue.current();
        self.snapshot.saved.volume = self
            .volume_glide
            .as_ref()
            .map_or(self.snapshot.volume, VolumeGlide::target);
        self.snapshot.saved.equalizer = self.snapshot.equalizer;
        self.snapshot.saved.edges = self.snapshot.edges;
        self.snapshot.saved.mix_enabled = self.snapshot.mix_enabled;
        self.snapshot.saved.shuffle = self.queue.shuffle();
        self.snapshot.saved.repeat = self.queue.repeat();
        self.snapshot.shuffle = self.queue.shuffle();
        self.snapshot.repeat = self.queue.repeat();
        self.snapshot.can_next = self.queue.can_advance(Advance::Next);
        let restarts = self.can_restart_previous();
        let has_previous = self.queue.can_advance(Advance::Previous);
        self.snapshot.can_previous = restarts || has_previous;
        self.snapshot.previous_navigates = !restarts && has_previous;
        *self
            .shared
            .snapshot
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = self.snapshot.clone();
    }

    /// Detects urgent stop/shutdown that arrived while a native operation was running.
    fn cancelled(&self) -> bool {
        let mailbox = self
            .shared
            .mailbox
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        mailbox.shutdown || mailbox.stop_barrier > self.snapshot.revision
    }

    /// Applies one ordered command, discarding controls superseded by Stop.
    fn handle(&mut self, request: Request) {
        self.snapshot.revision = request.revision;
        if self.cancelled() {
            return;
        }
        self.snapshot.error = None;
        let result = self.apply(request.command);
        if let Err(error) = result {
            self.fail(error);
        }
    }

    /// Routes typed operations; rejected queue additions preserve current playback.
    fn apply(&mut self, command: Command) -> Result<(), String> {
        self.prepare_mix_command(&command)
            .map_err(|error| error.to_string())?;
        match command {
            Command::SetMix(enabled) => {
                self.snapshot.mix_enabled = enabled;
            }
            Command::Analyze { generation } => {
                if generation == self.snapshot.generation {
                    self.analysis.request(&self.queue, &mut self.snapshot, true);
                }
            }
            Command::ViewAnalysis { generation } => {
                if generation == self.snapshot.generation {
                    self.analysis
                        .request(&self.queue, &mut self.snapshot, false);
                }
            }
            Command::AnalyzeTrack { id } => {
                self.analysis
                    .request_track(&self.queue, &mut self.snapshot, id)
            }
            Command::InspectTrack { id, force } => {
                self.analysis
                    .inspect(&self.queue, &mut self.snapshot, id, force);
            }
            Command::CancelInspection { id } => {
                if self
                    .snapshot
                    .inspection
                    .as_ref()
                    .is_some_and(|view| view.id == id)
                {
                    self.analysis.cancel_view(&mut self.snapshot);
                }
            }
            Command::AnalyzeAll => self.analysis.request_all(&self.queue, &mut self.snapshot),
            Command::CancelAllAnalysis => self.analysis.cancel(&mut self.snapshot),
            Command::CancelAnalysis { generation } => {
                if generation == self.snapshot.generation {
                    self.analysis.cancel_view(&mut self.snapshot);
                }
            }
            Command::Restore(saved) => {
                // Validated before submission; restoration never probes or opens media.
                self.queue.append(saved.paths.to_vec());
                if let Some(index) = saved.current {
                    self.queue.select(index);
                }
                self.queue.set_shuffle(saved.shuffle);
                self.queue.set_repeat(saved.repeat);
                self.snapshot.volume = saved.volume;
                self.snapshot.equalizer = saved.equalizer;
                self.snapshot.edges = saved.edges;
                self.snapshot.mix_enabled = saved.mix_enabled;
                self.snapshot.state = if saved.paths.is_empty() {
                    SessionState::Idle
                } else {
                    SessionState::Stopped
                };
                self.snapshot.position = Some(0.0);
                self.refresh_queue();
            }
            Command::Open(paths) => self.add_paths(paths, true)?,
            Command::Add(paths) => self.add_paths(paths, false)?,
            Command::Replace(paths) => self.replace_paths(paths),
            Command::Queue(action) => self.edit_queue(action)?,
            Command::TogglePause => {
                self.end_scan().map_err(|error| error.to_string())?;
                if self.player.is_some() {
                    self.paused = !self.paused;
                    if self.loaded
                        && !self.edges.waiting_for_start()
                        && let Some(player) = self.player.as_mut()
                    {
                        player.set_paused(self.paused).map_err(|e| e.to_string())?;
                    }
                } else if self.queue.current().is_some() {
                    self.open_current(false)?;
                } else {
                    self.navigate(Advance::Next)?;
                }
            }
            Command::Edges(action) => {
                self.previous_restart_requested = None;
                self.end_scan().map_err(|error| error.to_string())?;
                self.snapshot.edges.apply(action);
            }
            Command::Equalizer(action) => {
                self.snapshot
                    .equalizer
                    .apply(action)
                    .map_err(str::to_owned)?;
            }
            Command::Stop => {
                self.analysis.cancel(&mut self.snapshot);
                self.rhythm.cancel();
                self.edges.reset();
                self.snapshot.edge_status = super::EdgeStatus::Off;
                self.previous_restart_requested = None;
                self.player = None;
                self.snapshot.scanning = None;
                self.loaded = false;
                self.snapshot.state = SessionState::Stopped;
                self.snapshot.position = Some(0.0);
                self.snapshot.seekable = false;
            }
            Command::Next => self.navigate(Advance::Next)?,
            Command::Previous => self.previous()?,
            Command::Seek {
                generation,
                seconds,
            } => {
                if generation == self.snapshot.generation && self.loaded && self.snapshot.seekable {
                    self.previous_restart_requested = None;
                    self.end_scan().map_err(|error| error.to_string())?;
                    let Some(player) = self.player.as_mut() else {
                        return Ok(());
                    };
                    // duration may be a growing demuxer estimate (stream-encoded FLAC).
                    // Clamping here would turn a 103 s seek into a seek near the start.
                    // The UI bounds its slider; the native engine resolves absolute EOF.
                    let target =
                        SeekPosition::from_seconds(seconds).ok_or("Invalid seek position")?;
                    player
                        .seek_with_equalizer(target, self.snapshot.equalizer)
                        .map_err(|e| e.to_string())?;
                }
            }
            Command::Scan {
                generation,
                direction,
            } => {
                if generation == self.snapshot.generation {
                    self.previous_restart_requested = None;
                    // A release remains valid when format reconfiguration hides seek capability.
                    if direction.is_none() {
                        self.end_scan().map_err(|error| error.to_string())?;
                    } else if self.loaded
                        && self.snapshot.seekable
                        && let Some(player) = self.player.as_mut()
                    {
                        player
                            .set_scan(direction, self.snapshot.equalizer, self.paused)
                            .map_err(|error| error.to_string())?;
                        self.snapshot.scanning = direction;
                    }
                }
            }
            Command::Volume(volume) => {
                self.volume_glide = None;
                self.snapshot.volume = volume;
            }
            Command::GlideVolume(volume) => {
                self.volume_glide = Some(VolumeGlide::new(self.snapshot.volume, volume));
                self.volume_tick = Instant::now();
            }
        }
        Ok(())
    }

    /// Restores ordinary forward playback without changing the user's pause intent.
    fn end_scan(&mut self) -> Result<(), PlaybackError> {
        if self.snapshot.scanning.take().is_some()
            && let Some(player) = self.player.as_mut()
        {
            player.set_scan(None, self.snapshot.equalizer, self.paused)?;
        }
        Ok(())
    }

    /// Executes the core traversal policy, preserving pause for explicit navigation.
    fn navigate(&mut self, reason: Advance) -> Result<(), String> {
        if self.queue.advance(reason).is_some() {
            let paused = self.player.is_some() && self.paused;
            self.open_current(paused)?;
        }
        Ok(())
    }

    /// Exposes the same Back policy to commands and all UI consumers, including a lone track.
    fn can_restart_previous(&self) -> bool {
        let start = self.edges.region.map_or(0.0, |region| region.start);
        self.loaded
            && self.snapshot.seekable
            && self.previous_restart_requested.is_none()
            && resolibre_core::session::previous_restarts(self.snapshot.position, start)
    }

    /// Restarts the current occurrence without changing history, EQ or pause; early Back navigates.
    fn previous(&mut self) -> Result<(), String> {
        if !self.can_restart_previous() {
            return self.navigate(Advance::Previous);
        }
        self.end_scan().map_err(|error| error.to_string())?;
        let start = self.edges.region.map_or(0.0, |region| region.start);
        let target = SeekPosition::from_seconds(start).ok_or("Invalid playback start")?;
        if let Some(player) = self.player.as_mut() {
            player
                .seek_with_equalizer(target, self.snapshot.equalizer)
                .map_err(|error| error.to_string())?;
            // Native seek completion is asynchronous. A second Back must use the
            // accepted restart intent until timing catches up, for at most one second.
            self.previous_restart_requested = Some(Instant::now());
        }
        Ok(())
    }

    /// Atomically extends the bounded queue; failed additions do not stop music.
    fn add_paths(&mut self, paths: Vec<PathBuf>, start: bool) -> Result<(), String> {
        let existing: usize = self
            .queue
            .entries()
            .iter()
            .map(|entry| entry.source.as_os_str().len())
            .sum();
        let incoming: usize = paths.iter().map(|path| path.as_os_str().len()).sum();
        let first = self.queue.entries().len();
        if existing.saturating_add(incoming) > MAX_SELECTION_BYTES || !self.queue.append(paths) {
            self.snapshot.error = Some("Track limit reached (1024 files / 1 MiB paths)".into());
            return Ok(());
        }
        self.refresh_queue();
        if start {
            self.select(first, false)?;
        }
        Ok(())
    }

    /// Commits validated replacement before releasing audio; rejected input preserves playback.
    fn replace_paths(&mut self, paths: Vec<PathBuf>) {
        if !self.queue.replace(paths) {
            self.snapshot.error = Some("Cannot replace the track list with this selection".into());
            return;
        }

        self.edges.reset();
        self.snapshot.edge_status = super::EdgeStatus::Off;
        self.previous_restart_requested = None;
        self.player = None;
        self.snapshot.scanning = None;
        self.loaded = false;
        self.paused = false;
        self.snapshot.generation += 1;
        self.snapshot.eq_sample_rate = 0;
        self.snapshot.position = Some(0.0);
        self.snapshot.duration = None;
        self.snapshot.seekable = false;
        self.snapshot.state = SessionState::Stopped;
        self.refresh_queue();
    }

    /// Rebuilds labels only after edits; all consumers observe the same core-owned order.
    fn refresh_queue(&mut self) {
        self.metadata.retain(&self.queue, &mut self.snapshot);
        self.snapshot.saved.paths = self
            .queue
            .entries()
            .iter()
            .map(|entry| entry.source.clone())
            .collect::<Vec<_>>()
            .into();
        self.snapshot.queue_revision += 1;
        self.snapshot.tracks = self
            .queue
            .entries()
            .iter()
            .map(|entry| TrackLabel {
                id: entry.id,
                title: self.metadata.title(entry.id, &entry.source),
                search_text: self.metadata.search_text(entry.id, &entry.source),
                duration_seconds: self.metadata.duration_seconds(entry.id),
            })
            .collect::<Vec<_>>()
            .into();
        self.snapshot.current = self.queue.current();
    }

    /// Executes shared queue actions; moving/removing another row never reloads audio.
    fn edit_queue(&mut self, action: QueueAction) -> Result<(), String> {
        match action {
            QueueAction::SetShuffle(enabled) => self.queue.set_shuffle(enabled),
            QueueAction::SetRepeat(mode) => self.queue.set_repeat(mode),
            QueueAction::ToggleShuffle => self.queue.set_shuffle(!self.queue.shuffle()),
            QueueAction::CycleRepeat => self.queue.set_repeat(self.queue.repeat().cycle()),
            QueueAction::Select(id) => {
                if let Some(index) = self.queue.index_of(id) {
                    self.select(index, false)?;
                }
            }
            QueueAction::Move { id, forward } => {
                if self.queue.move_adjacent(id, forward) {
                    self.refresh_queue();
                }
            }
            QueueAction::MoveTo { id, target, after } => {
                if self.queue.move_to(id, target, after) {
                    self.refresh_queue();
                }
            }
            QueueAction::Remove(id) => {
                let removed_current = self.queue.current_id() == Some(id);
                if !self.queue.remove(id) {
                    return Ok(());
                }
                if removed_current {
                    self.edges.reset();
                    self.snapshot.edge_status = super::EdgeStatus::Off;
                    self.previous_restart_requested = None;
                    self.player = None;
                    self.snapshot.scanning = None;
                    self.loaded = false;
                    self.paused = false;
                    self.snapshot.eq_sample_rate = 0;
                    self.snapshot.generation += 1;
                    self.snapshot.position = Some(0.0);
                    self.snapshot.duration = None;
                    self.snapshot.seekable = false;
                    self.snapshot.state = if self.queue.current().is_some() {
                        SessionState::Stopped
                    } else {
                        SessionState::Idle
                    };
                }
                self.refresh_queue();
            }
        }
        Ok(())
    }

    /// Replaces the engine only after cleanup; new audio starts paused to avoid races.
    fn select(&mut self, index: usize, paused: bool) -> Result<(), String> {
        if !self.queue.select(index) {
            return Ok(());
        }
        self.open_current(paused)
    }

    /// Loads the core's already-selected target without recording navigation twice.
    fn open_current(&mut self, paused: bool) -> Result<(), String> {
        self.analysis.reset(&mut self.snapshot);
        self.snapshot.rhythm = super::RhythmStatus::Detecting;
        self.snapshot.next_rhythm = None;
        let Some(index) = self.queue.current() else {
            return Ok(());
        };
        self.metadata
            .select(self.queue.current_id(), &mut self.snapshot);
        self.metadata.reload(&mut self.snapshot);
        self.edges.reset();
        self.snapshot.edge_status = super::EdgeStatus::Off;
        self.previous_restart_requested = None;
        self.player = None;
        self.snapshot.scanning = None;
        self.loaded = false;
        self.paused = paused;
        self.snapshot.generation += 1;
        self.snapshot.current = Some(index);
        self.snapshot.state = SessionState::Loading;
        self.info_refresh = None;
        self.info_warning_logged = false;
        self.spectrum_failed = false;
        self.snapshot.live_spectrum = None;
        self.snapshot.position = None;
        self.snapshot.duration = None;
        self.snapshot.seekable = false;
        self.publish();
        if self.cancelled() {
            return Ok(());
        }
        let path = &self.queue.entries()[index].source;
        let player = LocalPlayer::open_with_equalizer(
            path,
            self.output,
            true,
            self.snapshot.volume,
            self.snapshot.equalizer,
        )
        .map_err(|e| e.to_string())?;
        if self.cancelled() {
            drop(player);
            return Ok(());
        }
        self.amplitude = self.snapshot.volume.amplitude();
        self.audio_wait_started = Some(Instant::now());
        self.player = Some(player);
        Ok(())
    }

    /// Starts optional decoding only after this tick has confirmed native playback or pause.
    fn tick_analysis(&mut self) {
        let ready = self.loaded
            && matches!(
                self.snapshot.state,
                SessionState::Playing | SessionState::Paused
            );
        if ready {
            self.analysis
                .prioritize_current(&self.queue, &mut self.snapshot);
        }
        let current_priority = ready
            && self
                .rhythm
                .current_priority(&self.queue, &mut self.snapshot);
        if current_priority {
            self.analysis.yield_to_current(&mut self.snapshot);
        }
        let full_analysis_waiting = if current_priority {
            self.analysis.has_worker()
        } else {
            self.analysis.wants_worker()
        };
        let rhythm_player = if ready { self.player.as_mut() } else { None };
        self.rhythm.tick(
            rhythm_player,
            &self.queue,
            &mut self.snapshot,
            full_analysis_waiting,
        );
        let available = !self.rhythm.busy() && !current_priority;
        let analysis_player = if ready && available {
            self.player.as_mut()
        } else {
            None
        };
        self.analysis
            .tick(analysis_player, &self.queue, &mut self.snapshot, available);
        if let Some(id) = self.analysis.take_completed() {
            self.rhythm.invalidate(id, &mut self.snapshot);
        }
    }

    /// Drains native lifecycle events, ramps gain and observes confirmed timing.
    fn tick(&mut self) -> Result<(), PlaybackError> {
        let elapsed = self.last_tick.elapsed().as_secs_f64().min(0.05);
        self.last_tick = Instant::now();
        if self.cancelled() {
            return Ok(());
        }
        self.metadata.tick(&self.queue, &mut self.snapshot);
        // Continue gain intent while idle/paused/loading; no UI timer or command flood.
        let volume_elapsed = self.volume_tick.elapsed();
        self.volume_tick = Instant::now();
        if let Some(glide) = self.volume_glide.as_mut() {
            let (volume, complete) = glide.advance(volume_elapsed);
            self.snapshot.volume = volume;
            if complete {
                self.volume_glide = None;
            }
        }
        let Some(player) = self.player.as_mut() else {
            return Ok(());
        };
        match player.try_event()? {
            Some(PlaybackEvent::Loaded) => {
                self.loaded = true;
            }
            Some(PlaybackEvent::Ended) => {
                if self.promote_mix()? {
                    return Ok(());
                }
                self.edges.reset();
                self.snapshot.edge_status = super::EdgeStatus::Off;
                self.previous_restart_requested = None;
                self.player = None;
                self.snapshot.scanning = None;
                self.loaded = false;
                self.snapshot.state = SessionState::Ended;
                self.snapshot.position = self.snapshot.duration;
                self.snapshot.seekable = false;
                if self.queue.advance(Advance::Ended).is_some()
                    && let Err(error) = self.open_current(false)
                {
                    self.fail(error);
                }
                return Ok(());
            }
            None => {}
        }
        if !self.loaded {
            self.check_audio_wait();
            return Ok(());
        }
        if !self.spectrum_failed {
            match player.live_spectrum() {
                Ok(levels) => self.snapshot.live_spectrum = levels,
                Err(message) => {
                    eprintln!("warn: {message}");
                    self.snapshot.live_spectrum = None;
                    self.spectrum_failed = true;
                }
            }
        }
        // Optional presentation reads are bounded in cadence and never fail playback.
        if self
            .info_refresh
            .is_none_or(|last| last.elapsed() >= Duration::from_millis(500))
        {
            self.info_refresh = Some(Instant::now());
            match player.stream_info() {
                Ok(info) => self.snapshot.stream_info = info,
                Err(_) => {
                    self.snapshot.stream_info = None;
                    if !self.info_warning_logged {
                        eprintln!("warn: current audio information unavailable");
                        self.info_warning_logged = true;
                    }
                }
            }
        }
        let negotiated = player.equalizer_rate() == 0;
        if negotiated && !player.prepare_equalizer(self.snapshot.equalizer)? {
            // Seek/EOF can temporarily remove formats before the end event arrives.
            // Bound this pending interval, not time elapsed since the track was opened.
            self.check_audio_wait();
            return Ok(());
        }
        self.audio_wait_started = None;
        if !self.edges.prepare(player, &mut self.snapshot)? {
            self.snapshot.state = SessionState::Loading;
            self.snapshot.seekable = false;
            return Ok(());
        }
        if player.equalizer_rate() == 0 {
            // Initial TRIM or an excluded live edge sought; wait for its replacement graph.
            self.check_audio_wait();
            return Ok(());
        }
        let resume_needed = self.edges.take_resume_needed();
        if negotiated || resume_needed {
            player.set_paused(self.snapshot.scanning.is_none() && self.paused)?;
        }
        player.ramp_equalizer(self.snapshot.equalizer, elapsed)?;
        self.snapshot.eq_sample_rate = player.equalizer_rate();
        let target = self.snapshot.volume.amplitude();
        let difference = target - self.amplitude;
        let step = elapsed / 0.12;
        if difference.abs() > f64::EPSILON {
            self.amplitude += difference.clamp(-step, step);
            if !self.mixing.controls_gain() {
                player.set_amplitude(self.amplitude.clamp(0.0, 1.0))?;
            }
        }
        let timing = player.timing()?;
        self.snapshot.position = timing.position.map(|position| {
            self.edges
                .region
                .map_or(position, |region| position.clamp(region.start, region.end))
        });
        let start = self.edges.region.map_or(0.0, |region| region.start);
        let restart_observed = self
            .snapshot
            .position
            .is_some_and(|position| position >= start && position - start < 1.0);
        if self.previous_restart_requested.is_some_and(|requested| {
            restart_observed || requested.elapsed() >= Duration::from_secs(1)
        }) {
            self.previous_restart_requested = None;
        }
        self.snapshot.duration = self.edges.source_duration().or(timing.duration);
        self.snapshot.seekable = timing.seekable;
        self.snapshot.state = if timing.paused {
            SessionState::Paused
        } else {
            SessionState::Playing
        };
        Ok(())
    }

    /// Bounds each continuous initialization/reconfiguration wait while events keep draining.
    fn check_audio_wait(&mut self) {
        let started = self.audio_wait_started.get_or_insert_with(Instant::now);
        if started.elapsed() > Duration::from_secs(10) {
            self.fail("Audio initialization timed out; retry or open another file".into());
        }
    }

    /// Logs one safe failure where handled and retains the selected queue occurrence.
    fn fail(&mut self, message: String) {
        eprintln!("error: {message}");
        self.rhythm.cancel();
        self.analysis.cancel(&mut self.snapshot);
        self.edges.reset();
        self.snapshot.edge_status = super::EdgeStatus::Off;
        self.previous_restart_requested = None;
        self.player = None;
        self.snapshot.scanning = None;
        self.loaded = false;
        self.snapshot.state = SessionState::Failed;
        self.snapshot.seekable = false;
        self.snapshot.error = Some(message);
    }
}
