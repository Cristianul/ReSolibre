//! One prepared libmpv stream, owned by the same transport worker as current playback.

use super::Worker;
use crate::original_audio::SourceKey;
use crate::session::{Command, MixStatus, edges::EdgeRuntime};
use crate::{LocalPlayer, PlaybackError};
use resolibre_core::playback_edges::{FadeMode, PlaybackRegion};
use resolibre_core::queue::Advance;
use resolibre_core::session::{SessionState, Volume};
use resolibre_core::transition::{TempoReturn, TransitionFrame, TransitionPlan};
use resolibre_core::{PlaybackEvent, SeekPosition};
use std::time::{Duration, Instant};

mod phase;
use phase::{PhaseAdjustment, PhaseAlignment};

#[derive(Default)]
pub(super) struct MixRuntime {
    incoming: Option<Incoming>,
    attempted: Option<(u64, u64)>,
    returning: Option<ReturningTempo>,
}

struct ReturningTempo {
    curve: TempoReturn,
    generation: u64,
    speed: f64,
}

impl MixRuntime {
    /// Exposes only confirmed bounds from the retained transition, never a UI estimate.
    pub(super) fn region(&self) -> Option<PlaybackRegion> {
        let incoming = self.incoming.as_ref()?;
        if !incoming.ready || incoming.cancelling.is_some() {
            return None;
        }
        let plan = incoming.plan?;
        Some(PlaybackRegion {
            start: plan.outgoing_start,
            end: plan.outgoing_end,
        })
    }

    /// Publishes only the ratio of a retained, ready tempo plan; cancellation clears it.
    pub(super) fn beat_ratio(&self) -> Option<resolibre_core::transition::BeatRatio> {
        let incoming = self.incoming.as_ref()?;
        if !incoming.ready || incoming.cancelling.is_some() {
            return None;
        }
        let plan = incoming.plan?;
        plan.outgoing_bpm.map(|_| plan.beat_ratio)
    }

    /// Keeps the transport's master-volume ramp from briefly bypassing an active overlap envelope.
    pub(super) fn controls_gain(&self) -> bool {
        self.incoming
            .as_ref()
            .is_some_and(|incoming| incoming.started)
    }
}

struct Incoming {
    player: LocalPlayer,
    edges: EdgeRuntime,
    generation: u64,
    id: u64,
    loaded: bool,
    seeked: bool,
    ready: bool,
    started: bool,
    phase: PhaseAlignment,
    phase_release: Option<f64>,
    opened: Instant,
    plan: Option<TransitionPlan>,
    outgoing_speed: f64,
    incoming_speed: f64,
    preroll: f64,
    last_frame: Option<TransitionFrame>,
    cancelling: Option<(Instant, TransitionFrame)>,
    source_keys: Option<(SourceKey, SourceKey)>,
}

impl Worker {
    /// Keeps explicit transport edits authoritative; pause addresses both audible streams.
    pub(super) fn prepare_mix_command(&mut self, command: &Command) -> Result<(), PlaybackError> {
        match command {
            Command::Stop => self.mixing = MixRuntime::default(),
            Command::SetMix(false) if !self.paused => {
                if let Some(incoming) = self.mixing.incoming.as_mut()
                    && let Some(frame) = incoming.last_frame
                {
                    incoming.cancelling = Some((Instant::now(), frame));
                } else {
                    self.cancel_mix()?;
                }
            }
            Command::Seek { generation, .. } => {
                // A muted next stream and its source-time plan are unaffected by a seek.
                // Only an audible overlap needs cancellation; obsolete gestures do nothing.
                if *generation == self.snapshot.generation
                    && self.loaded
                    && self.snapshot.seekable
                    && (self.mixing.controls_gain() || self.mixing.returning.is_some())
                {
                    self.cancel_mix()?;
                }
            }
            Command::TogglePause => {
                if let Some(incoming) = self.mixing.incoming.as_mut() {
                    incoming
                        .player
                        .set_paused(!self.paused || !incoming.started)?;
                }
            }
            Command::Volume(_)
            | Command::GlideVolume(_)
            | Command::Equalizer(_)
            | Command::Analyze { .. }
            | Command::ViewAnalysis { .. }
            | Command::InspectTrack { .. }
            | Command::CancelInspection { .. }
            | Command::AnalyzeTrack { .. }
            | Command::AnalyzeAll
            | Command::CancelAllAnalysis
            | Command::CancelAnalysis { .. }
            | Command::Add(_)
            | Command::SetMix(true) => {}
            _ => self.cancel_mix()?,
        }
        Ok(())
    }

    /// Cancels one prepared overlap and restores ordinary current-track processing.
    fn cancel_mix(&mut self) -> Result<(), PlaybackError> {
        let transitioning = self
            .mixing
            .incoming
            .take()
            .is_some_and(|incoming| incoming.started || incoming.outgoing_speed != 1.0);
        self.mixing.attempted = None;
        let returning = self.mixing.returning.take().is_some();
        let active = transitioning || returning;
        if active && let Some(player) = self.player.as_mut() {
            player.set_mix_speed(1.0)?;
            player.set_amplitude(self.amplitude)?;
            player.apply_edges(
                self.edges
                    .region
                    .map(|region| (region, self.snapshot.edges.fade)),
                self.snapshot.equalizer,
                false,
            )?;
        }
        Ok(())
    }

    /// Revalidates queue intent each tick and contains optional transition failures without stopping music.
    pub(super) fn tick_mix(&mut self) {
        if let Err(error) = self.advance_mix() {
            let stage = match self.mixing.incoming.as_ref() {
                Some(incoming) if incoming.started => "overlap",
                Some(incoming) if incoming.ready => "start",
                _ if self.mixing.returning.is_some() => "tempo return",
                _ => "prepare",
            };
            let attempted = self.mixing.attempted;
            if let Err(error) = self.cancel_mix() {
                self.fail(error.to_string());
            }
            self.mixing.attempted = attempted;
            self.snapshot.mix_status = MixStatus::Unavailable;
            eprintln!("warn: MIX unavailable at {stage}: {error}; retaining ordinary playback");
        }
    }

    /// Prepares only the core queue's next occurrence and only near the outgoing edge.
    fn advance_mix(&mut self) -> Result<(), PlaybackError> {
        if self
            .mixing
            .incoming
            .as_ref()
            .is_some_and(|incoming| incoming.cancelling.is_some())
        {
            return self.fade_back_to_current();
        }
        let usable = self.snapshot.mix_enabled
            && self.loaded
            && self.snapshot.scanning.is_none()
            && matches!(
                self.snapshot.state,
                SessionState::Playing | SessionState::Paused
            );
        if !usable {
            self.cancel_mix()?;
            self.snapshot.mix_status = if self.snapshot.mix_enabled {
                MixStatus::Preparing
            } else {
                MixStatus::Off
            };
            return Ok(());
        }
        if self.continue_tempo_return()? {
            return Ok(());
        }
        let mut lookahead = self.queue.clone();
        let Some(index) = lookahead.advance(Advance::Ended) else {
            self.cancel_mix()?;
            self.snapshot.mix_status = MixStatus::Unavailable;
            return Ok(());
        };
        let next = &lookahead.entries()[index];
        let target = (self.snapshot.generation, next.id);
        if self
            .mixing
            .incoming
            .as_ref()
            .is_some_and(|incoming| (incoming.generation, incoming.id) != target)
        {
            self.cancel_mix()?;
        }
        if self.mixing.incoming.is_none() {
            if self.mixing.attempted == Some(target) {
                return Ok(());
            }
            self.snapshot.mix_status = MixStatus::Preparing;
            let (Some(position), Some(duration)) = (self.snapshot.position, self.snapshot.duration)
            else {
                return Ok(());
            };
            let region = self.edges.region.unwrap_or(PlaybackRegion {
                start: 0.0,
                end: duration,
            });
            let Some(start) = TransitionPlan::preparation_start(region) else {
                return Ok(());
            };
            if position < start || self.paused {
                return Ok(());
            }
            self.mixing.attempted = Some(target);
            let silence = Volume::from_percent(0.0).ok_or(PlaybackError::InvalidEvent)?;
            let player = LocalPlayer::open_with_equalizer(
                &next.source,
                self.output,
                true,
                silence,
                self.snapshot.equalizer,
            )?;
            self.mixing.incoming = Some(Incoming {
                player,
                edges: EdgeRuntime::default(),
                generation: target.0,
                id: target.1,
                loaded: false,
                seeked: false,
                ready: false,
                started: false,
                phase: PhaseAlignment::default(),
                phase_release: None,
                opened: Instant::now(),
                plan: None,
                outgoing_speed: 1.0,
                incoming_speed: 1.0,
                preroll: 0.0,
                last_frame: None,
                cancelling: None,
                source_keys: None,
            });
        }
        self.prepare_incoming()?;
        self.render_mix()
    }

    /// Continues the retained source-clock curve on the promoted stream; pause and buffering freeze it.
    fn continue_tempo_return(&mut self) -> Result<bool, PlaybackError> {
        let Some(returning) = self.mixing.returning.as_mut() else {
            return Ok(false);
        };
        if returning.generation != self.snapshot.generation {
            self.mixing.returning = None;
            return Ok(false);
        }
        self.snapshot.mix_status = MixStatus::TempoMixing;
        if self.paused {
            return Ok(true);
        }
        let player = self.player.as_mut().ok_or(PlaybackError::InvalidEvent)?;
        if player.is_seeking()? {
            return Ok(true);
        }
        let position = match player.audio_position()? {
            Some(position) => Some(position),
            None => player.timing()?.position,
        };
        let Some(frame) = position.and_then(|position| returning.curve.frame(position)) else {
            return Ok(true);
        };
        if frame.complete || (frame.speed - returning.speed).abs() > 0.0001 {
            player.set_mix_speed(frame.speed)?;
            returning.speed = frame.speed;
        }
        if frame.complete {
            self.mixing.returning = None;
            return Ok(false);
        }
        Ok(true)
    }

    /// Loads, validates source identities and seeks the muted next stream before allowing any overlap.
    fn prepare_incoming(&mut self) -> Result<(), PlaybackError> {
        let incoming = self
            .mixing
            .incoming
            .as_mut()
            .ok_or(PlaybackError::InvalidEvent)?;
        match incoming.player.try_event()? {
            Some(PlaybackEvent::Loaded) => incoming.loaded = true,
            Some(PlaybackEvent::Ended) => return Err(PlaybackError::UnexpectedEnd),
            None => {}
        }
        if incoming.started {
            if incoming.player.is_seeking()? {
                return Ok(());
            }
            if incoming.player.equalizer_rate() == 0
                && !incoming.player.prepare_equalizer(self.snapshot.equalizer)?
            {
                return Ok(());
            }
            incoming
                .player
                .ramp_equalizer(self.snapshot.equalizer, 0.01)?;
            return Ok(());
        }
        if incoming.opened.elapsed() > Duration::from_secs(15) && !incoming.ready {
            return Err(PlaybackError::UnexpectedEnd);
        }
        if !incoming.loaded || !incoming.player.prepare_equalizer(self.snapshot.equalizer)? {
            return Ok(());
        }
        let mut prepared = self.snapshot.clone();
        prepared.edges.fade = FadeMode::Off;
        if !incoming
            .edges
            .prepare(&mut incoming.player, &mut prepared)?
        {
            return Ok(());
        }
        if incoming.player.equalizer_rate() == 0 || incoming.player.is_seeking()? {
            return Ok(());
        }
        if incoming.plan.is_none() {
            let current = self.player.as_mut().ok_or(PlaybackError::InvalidEvent)?;
            let current_id = self.queue.current_id().ok_or(PlaybackError::InvalidEvent)?;
            let current_report = self.rhythm.verified_report(current_id, current);
            let next_report = self
                .rhythm
                .verified_report(incoming.id, &mut incoming.player);
            if self.rhythm.busy() {
                return Ok(());
            }
            let outgoing = self.edges.region.or(self
                .snapshot
                .duration
                .map(|end| PlaybackRegion { start: 0.0, end }));
            let next_duration = incoming.player.timing()?.duration;
            let opening = incoming
                .edges
                .region
                .or(next_duration.map(|end| PlaybackRegion { start: 0.0, end }));
            if let (Some(outgoing), Some(opening)) = (outgoing, opening) {
                incoming.plan = TransitionPlan::prepare(
                    outgoing,
                    opening,
                    current_report.as_deref(),
                    next_report.as_deref(),
                );
                incoming.source_keys =
                    Some((source_key(current)?, source_key(&mut incoming.player)?));
            }
        }
        let Some(plan) = incoming.plan else {
            return Ok(());
        };
        if !incoming.seeked {
            let speed = plan.frame(plan.outgoing_start).incoming_speed;
            let source_start = incoming.edges.region.map_or(0.0, |region| region.start);
            if plan.outgoing_bpm.is_some() {
                let outgoing_speed = plan.frame(plan.outgoing_start).outgoing_speed;
                let lead_seconds = ((plan.incoming_start - source_start) / speed).clamp(0.0, 0.3);
                incoming.preroll = lead_seconds * outgoing_speed;
            }
            let start = plan
                .incoming_position(plan.outgoing_start - incoming.preroll)
                .unwrap_or(plan.incoming_start)
                .max(source_start);
            // Equal-BPM plans may still need phase feedback. Prime their stretcher while muted too.
            if plan.phase_matched {
                incoming.player.prepare_mix_tempo()?;
            }
            incoming.player.set_mix_speed(speed)?;
            incoming.incoming_speed = speed;
            incoming.player.seek_with_equalizer(
                SeekPosition::from_seconds(start).ok_or(PlaybackError::InvalidEvent)?,
                self.snapshot.equalizer,
            )?;
            incoming.seeked = true;
            return Ok(());
        }
        incoming.ready = true;
        self.snapshot.mix_status = if plan.outgoing_bpm.is_some() {
            MixStatus::Ready
        } else {
            MixStatus::ReadyCrossfade
        };
        Ok(())
    }

    /// Follows the outgoing audio clock, sharing gain and tempo intent across both native streams.
    fn render_mix(&mut self) -> Result<(), PlaybackError> {
        let incoming = self
            .mixing
            .incoming
            .as_mut()
            .ok_or(PlaybackError::InvalidEvent)?;
        if !incoming.ready || self.paused {
            return Ok(());
        }
        let Some(plan) = incoming.plan else {
            return Ok(());
        };
        let current = self.player.as_mut().ok_or(PlaybackError::InvalidEvent)?;
        // Native clocks can still describe the old position while a seek is in flight.
        if current.is_seeking()? {
            return Ok(());
        }
        let audio_position = current.audio_position()?;
        let position = match audio_position {
            Some(position) => Some(position),
            None => current.timing()?.position,
        };
        let Some(position) = position else {
            return Ok(());
        };
        let mut frame = plan.frame(position);
        if (incoming.outgoing_speed - frame.outgoing_speed).abs() > 0.0001 {
            current.set_mix_speed(frame.outgoing_speed)?;
            incoming.outgoing_speed = frame.outgoing_speed;
        }
        if incoming.outgoing_speed != 1.0 {
            incoming.last_frame = Some(frame);
        }
        // Some outputs omit audio-pts while time-pos still advances. Do not leave
        // READY stuck until EOF; render the overlap without claiming verified sync.
        if position < plan.outgoing_start - incoming.preroll {
            return Ok(());
        }
        if !incoming.started {
            let keys = (source_key(current)?, source_key(&mut incoming.player)?);
            if incoming.source_keys != Some(keys) {
                return Err(PlaybackError::InvalidEvent);
            }
            if position >= plan.outgoing_end - 0.5 {
                return Err(PlaybackError::UnexpectedEnd);
            }
            current.apply_edges(
                self.edges.region.map(|region| (region, FadeMode::Off)),
                self.snapshot.equalizer,
                false,
            )?;
            incoming.player.set_paused(false)?;
            incoming.started = true;
            let now = Instant::now();
            incoming.phase.start(now);
        }
        self.snapshot.mix_status = if plan.outgoing_bpm.is_some() {
            MixStatus::TempoMixing
        } else {
            MixStatus::Crossfading
        };
        let mut incoming_speed = frame.incoming_speed;
        let seeking = incoming.player.is_seeking()?;
        let next_position = if seeking {
            None
        } else {
            incoming.player.audio_position()?
        };
        if plan.phase_matched
            && audio_position.is_some()
            && let Some(wanted) = plan.incoming_position(position)
            && let Some(next_position) = next_position
        {
            let can_seek = incoming.phase_release.is_none();
            let adjustment = incoming.phase.observe(
                wanted,
                next_position,
                frame.incoming_speed,
                can_seek,
                Instant::now(),
            );
            match adjustment {
                Some(PhaseAdjustment::Seek(target)) => {
                    incoming.player.seek_with_equalizer(
                        SeekPosition::from_seconds(target).ok_or(PlaybackError::InvalidEvent)?,
                        self.snapshot.equalizer,
                    )?;
                }
                Some(PhaseAdjustment::Follow {
                    speed,
                    synchronized,
                }) => {
                    incoming_speed = speed;
                    if synchronized {
                        self.snapshot.mix_status = MixStatus::BeatMixing;
                        incoming.phase_release.get_or_insert(position);
                    }
                }
                None => {}
            }
        }
        if plan.phase_matched {
            if !seeking
                && (audio_position.is_none() || next_position.is_none())
                && incoming.phase.permits_unverified_fallback(Instant::now())
            {
                // Missing clocks retain the ordinary TEMPO fallback; measured misalignment never does.
                incoming.phase_release.get_or_insert(position);
            }
            frame = phase::gate_gains(plan, position, frame, incoming.phase_release)
                .ok_or(PlaybackError::UnexpectedEnd)?;
        }
        incoming.last_frame = Some(frame);
        if (incoming.incoming_speed - incoming_speed).abs() > 0.0001 {
            incoming.player.set_mix_speed(incoming_speed)?;
            incoming.incoming_speed = incoming_speed;
        }
        current.set_amplitude(self.amplitude * frame.outgoing_gain)?;
        incoming
            .player
            .set_amplitude(self.amplitude * frame.incoming_gain)?;
        if frame.complete {
            self.promote_mix()?;
        }
        Ok(())
    }

    /// Disabling MIX restores current speed and complementary gains over 150 ms, including before overlap.
    fn fade_back_to_current(&mut self) -> Result<(), PlaybackError> {
        let incoming = self
            .mixing
            .incoming
            .as_mut()
            .ok_or(PlaybackError::InvalidEvent)?;
        let (started, frame) = incoming.cancelling.ok_or(PlaybackError::InvalidEvent)?;
        let progress = (started.elapsed().as_secs_f64() / 0.15).min(1.0);
        let current = self.player.as_mut().ok_or(PlaybackError::InvalidEvent)?;
        current.set_amplitude(
            self.amplitude * (frame.outgoing_gain + frame.incoming_gain * progress),
        )?;
        current.set_mix_speed(frame.outgoing_speed + (1.0 - frame.outgoing_speed) * progress)?;
        incoming
            .player
            .set_amplitude(self.amplitude * frame.incoming_gain * (1.0 - progress))?;
        if progress >= 1.0 {
            self.cancel_mix()?;
            self.snapshot.mix_status = if self.snapshot.mix_enabled {
                MixStatus::Preparing
            } else {
                MixStatus::Off
            };
        }
        Ok(())
    }

    /// Advances the authoritative queue once, adopting the already-running next stream without replaying its intro.
    pub(super) fn promote_mix(&mut self) -> Result<bool, PlaybackError> {
        if !self.mixing.incoming.as_ref().is_some_and(|incoming| {
            incoming.started
                && (!incoming.plan.is_some_and(|plan| plan.phase_matched)
                    || incoming.phase_release.is_some())
        }) {
            return Ok(false);
        }
        let mut incoming = self
            .mixing
            .incoming
            .take()
            .ok_or(PlaybackError::InvalidEvent)?;
        let mut lookahead = self.queue.clone();
        let next = lookahead.advance(Advance::Ended);
        if next.is_none() || lookahead.current_id() != Some(incoming.id) {
            return Err(PlaybackError::InvalidEvent);
        }
        incoming.player.set_amplitude(self.amplitude)?;
        let timing = incoming.player.timing()?;
        self.snapshot.position = timing.position;
        self.snapshot.duration = incoming.edges.source_duration().or(timing.duration);
        self.snapshot.seekable = timing.seekable;
        self.snapshot.eq_sample_rate = incoming.player.equalizer_rate();
        self.queue = lookahead;
        self.analysis.reset(&mut self.snapshot);
        self.player = Some(incoming.player);
        self.edges = incoming.edges;
        self.loaded = true;
        self.snapshot.generation += 1;
        // Carry the original curve across ownership transfer; never reset speed at handover.
        self.mixing.returning = incoming
            .plan
            .and_then(TransitionPlan::tempo_return)
            .map(|curve| ReturningTempo {
                curve,
                generation: self.snapshot.generation,
                speed: incoming.incoming_speed,
            });
        self.snapshot.current = next;
        self.snapshot.rhythm = crate::session::RhythmStatus::Detecting;
        self.snapshot.next_rhythm = None;
        self.snapshot.mix_status = if self.mixing.returning.is_some() {
            MixStatus::TempoMixing
        } else {
            MixStatus::Preparing
        };
        self.snapshot.stream_info = None;
        self.snapshot.live_spectrum = None;
        self.metadata
            .select(self.queue.current_id(), &mut self.snapshot);
        self.metadata.reload(&mut self.snapshot);
        self.previous_restart_requested = None;
        self.info_refresh = None;
        self.info_warning_logged = false;
        self.spectrum_failed = false;
        self.mixing.attempted = None;
        Ok(true)
    }
}

/// Rechecks the retained file and selected stream before muted preparation becomes audible playback.
fn source_key(player: &mut LocalPlayer) -> Result<SourceKey, PlaybackError> {
    let file = player.analysis_input()?;
    let stream = player.audio_stream_index()?;
    SourceKey::read(&file, stream).map_err(|_| PlaybackError::InvalidEvent)
}
