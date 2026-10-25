//! Grouped diagnostics retain one immutable inspected result independently of playback navigation.

mod provenance;
mod timeline;

use crate::{AnalysisWindow, PlayerWindow, presentation::Controller};
use resolibre_audio::session::{AnalysisStatus, Command, Snapshot};
use resolibre_core::{
    analysis::{ROWS, TrackAnalysis},
    track_info::AudioCodec,
};
use slint::{ComponentHandle, Image, Rgb8Pixel, SharedPixelBuffer};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
};

#[derive(Default)]
pub(crate) struct AnalysisHost {
    window: Option<AnalysisWindow>,
    windows: crate::window_group::SharedGroup,
    generation: Rc<Cell<Option<u64>>>,
    track_id: Rc<Cell<Option<u64>>>,
    source_duration: Option<f64>,
    rendered: Option<Arc<TrackAnalysis>>,
    completed: bool,
    requested_revision: u64,
    source_bits: Option<u8>,
}

impl AnalysisHost {
    /// Opens the reusable grouped surface for a concrete track; UI text never acts as an input path.
    fn show(
        &mut self,
        main: &PlayerWindow,
        controller: &Rc<RefCell<Controller>>,
        track_id: Option<u64>,
        force: bool,
    ) -> Result<(), &'static str> {
        let snapshot = controller.borrow().snapshot();
        let index = match track_id {
            Some(id) => Some(
                snapshot
                    .tracks
                    .iter()
                    .position(|track| track.id == id)
                    .ok_or("This track is no longer in the library")?,
            ),
            None => snapshot.current,
        };
        if self.window.is_none() {
            let window = AnalysisWindow::new().map_err(|_| "Cannot create analysis window")?;
            let weak = main.as_weak();
            let view = window.as_weak();
            let owner = Rc::downgrade(controller);
            let target = Rc::clone(&self.generation);
            let inspected = Rc::clone(&self.track_id);
            window.window().on_close_requested(move || {
                if let (Some(main), Some(view), Some(controller)) =
                    (weak.upgrade(), view.upgrade(), owner.upgrade())
                {
                    let command = cancel_command(target.get(), inspected.get());
                    controller.borrow_mut().submit(&main, command);
                    if view.hide().is_err() {
                        eprintln!("warn: cannot hide analysis window");
                    }
                    main.invoke_focus_analysis();
                }
                slint::CloseRequestResponse::KeepWindowShown
            });
            let weak = main.as_weak();
            let owner = Rc::downgrade(controller);
            let target = Rc::clone(&self.generation);
            let inspected = Rc::clone(&self.track_id);
            window.on_cancel_analysis(move || {
                if let (Some(main), Some(controller)) = (weak.upgrade(), owner.upgrade()) {
                    let command = cancel_command(target.get(), inspected.get());
                    controller.borrow_mut().submit(&main, command);
                }
            });
            let weak = main.as_weak();
            let inspected = Rc::clone(&self.track_id);
            window.on_retry_analysis(move || {
                if let Some(main) = weak.upgrade() {
                    if let Some(id) = inspected.get() {
                        main.invoke_reanalyze_track(id.to_string().into());
                    } else {
                        main.invoke_reanalyze_current();
                    }
                }
            });
            let events = crate::window_events::WindowEvents::new(window.window());
            crate::window_group::WindowGroup::watch(&self.windows, &window, &events);
            self.window = Some(window);
        }
        self.generation.set(Some(snapshot.generation));
        self.track_id.set(track_id);
        let track = index.and_then(|index| snapshot.tracks.get(index));
        let stream_info = if index == snapshot.current {
            snapshot.stream_info
        } else {
            snapshot
                .audible_next
                .filter(|next| Some(next.index) == index)
                .and_then(|next| next.stream_info)
        };
        self.source_duration = if index == snapshot.current {
            snapshot.duration
        } else {
            snapshot
                .audible_next
                .filter(|next| Some(next.index) == index)
                .map(|next| next.duration)
        }
        .or_else(|| track.and_then(|track| track.duration_seconds.map(f64::from)));
        self.rendered = None;
        self.completed = false;
        let window = self
            .window
            .as_ref()
            .ok_or("Cannot create analysis window")?;
        window.set_library_inspection(track_id.is_some());
        window.set_track_title(
            track
                .map(|track| track.title.as_str())
                .unwrap_or("No track")
                .into(),
        );
        self.source_bits = stream_info.and_then(|info| info.bits_per_sample);
        window.set_frequency_note(
            stream_info
                .and_then(|info| info.sample_rate_hz)
                .map(frequency_note)
                .unwrap_or_default()
                .into(),
        );
        window.set_plot(Image::default());
        window.set_tempo_zones(Default::default());
        window.invoke_reset_tempo_selection();
        window.set_ready(false);
        window.set_provenance_text("Lossy-source detection: waiting for audio analysis…".into());
        window.set_tempo_text("BPM: analyzing rhythmic changes…".into());
        window.set_interpretation("Original decoded audio, before EQ and volume. Origin unknown; a lossless codec does not prove an originally lossless recording.".into());
        window.set_facts(
            stream_info
                .map(|info| {
                    format!(
                        "{} · {}",
                        crate::track_info::describe(info),
                        compression(info.codec)
                    )
                })
                .unwrap_or_else(|| "Stream information unavailable".into())
                .into(),
        );
        window.set_status_text(
            if force {
                "Starting analysis…"
            } else {
                "Opening analysis…"
            }
            .into(),
        );
        window.set_running(true);
        let generation = snapshot.generation;
        let command = if let Some(id) = track_id {
            Command::InspectTrack { id, force }
        } else if force {
            Command::Analyze { generation }
        } else {
            Command::ViewAnalysis { generation }
        };
        {
            let mut controller = controller.borrow_mut();
            controller.submit(main, command);
            self.requested_revision = controller.expected_revision();
        }
        window.show().map_err(|_| "Cannot show analysis window")?;
        crate::window_snap::present(window.window());
        Ok(())
    }

    /// Freezes a completed inspection until an explicit Analyze action selects a new result.
    pub(crate) fn refresh(&mut self, snapshot: &Snapshot) {
        let Some(window) = self
            .window
            .as_ref()
            .filter(|window| window.window().is_visible())
        else {
            return;
        };
        if snapshot.revision < self.requested_revision || self.completed {
            return;
        }
        if self.track_id.get().is_none() && self.generation.get() != Some(snapshot.generation) {
            window.set_running(false);
            window.set_status_text(
                "Playback changed before analysis completed. Analyze the current track to retry."
                    .into(),
            );
            return;
        }
        let status = if let Some(id) = self.track_id.get() {
            let Some(view) = snapshot.inspection.as_ref().filter(|view| view.id == id) else {
                window.set_running(false);
                window.set_status_text(
                    "This track's analysis is no longer available. Retry to analyze it.".into(),
                );
                return;
            };
            &view.status
        } else {
            &snapshot.analysis
        };
        window.set_running(matches!(status, AnalysisStatus::Running { .. }));
        match status {
            AnalysisStatus::Running {
                decoded_seconds,
                phase,
                preview,
            } => {
                if let Some(report) = preview {
                    if self
                        .rendered
                        .as_ref()
                        .is_none_or(|previous| !Arc::ptr_eq(previous, report))
                    {
                        show_measurements(window, report, self.source_bits);
                        self.rendered = Some(Arc::clone(report));
                    }
                    window
                        .set_status_text("Spectrogram and BPM ready · source check running".into());
                    let progress = match phase {
                        resolibre_audio::session::AnalysisPhase::DetectingSource { .. }
                        | resolibre_audio::session::AnalysisPhase::Saving => {
                            analysis_progress(*phase, *decoded_seconds, self.source_duration)
                        }
                        _ => "Preparing lossy-source check…".into(),
                    };
                    window.set_provenance_text(progress.into());
                    return;
                }
                if self.rendered.take().is_some() {
                    window.set_ready(false);
                    window.set_plot(Image::default());
                    window.set_tempo_zones(Default::default());
                    window.set_tempo_text("BPM: analyzing rhythmic changes…".into());
                    window.set_provenance_text(
                        "Lossy-source detection: waiting for audio analysis…".into(),
                    );
                }
                let progress = analysis_progress(*phase, *decoded_seconds, self.source_duration);
                window.set_status_text(progress.into());
                if self.track_id.get().is_none()
                    && let resolibre_audio::session::RhythmStatus::Ready(report) = &snapshot.rhythm
                {
                    window.set_tempo_text(tempo_text(report).into());
                }
            }
            AnalysisStatus::Complete(report) => {
                if self
                    .rendered
                    .as_ref()
                    .is_none_or(|previous| !same_measurements(previous, report))
                {
                    show_measurements(window, report, self.source_bits);
                }
                window.set_status_text(
                    format!("Complete · {:.1} s analyzed", report.duration_seconds).into(),
                );
                window.set_provenance_text(provenance::describe(report.provenance).into());
                self.completed = true;
                self.rendered = Some(Arc::clone(report));
            }
            AnalysisStatus::Cancelled | AnalysisStatus::Unavailable => {
                if matches!(status, AnalysisStatus::Cancelled) && self.rendered.is_some() {
                    window.set_status_text(
                        "Source check cancelled · spectrogram and BPM retained for this view"
                            .into(),
                    );
                    window.set_provenance_text("Lossy-source detection: cancelled".into());
                    self.completed = true;
                    return;
                }
                window.set_ready(false);
                window.set_plot(Image::default());
                window.set_tempo_text("BPM: not available".into());
                window.set_provenance_text("Lossy-source detection: not available".into());
                self.rendered = None;
                let message = if matches!(status, AnalysisStatus::Cancelled) {
                    "Analysis cancelled. No partial result is used."
                } else {
                    "Analysis unavailable: decoder, changed source or resource limit. Playback continues; retry is available."
                };
                window.set_status_text(message.into());
            }
            AnalysisStatus::Idle => {}
        }
    }

    /// Releases the window and image during main-window shutdown; the session cancels/reaps its decoder.
    pub(crate) fn close(&mut self) {
        self.rendered = None;
        self.completed = false;
        if let Some(window) = self.window.take()
            && window.hide().is_err()
        {
            eprintln!("warn: cannot hide analysis window during shutdown");
        }
    }
}

/// Renders validated spectral/tempo measurements without presenting a pending source check as a verdict.
fn show_measurements(window: &AnalysisWindow, report: &TrackAnalysis, source_bits: Option<u8>) {
    window.set_plot(render(report));
    window.set_tempo_zones(timeline::zones(report));
    window.set_ready(true);
    window.set_nyquist(format!("{:.2} kHz", f64::from(report.sample_rate_hz) / 2000.0).into());
    window.set_half_nyquist(format!("{:.1} kHz", f64::from(report.sample_rate_hz) / 4000.0).into());
    window.set_duration(
        format!(
            "{}:{:02}",
            report.duration_seconds as u64 / 60,
            report.duration_seconds as u64 % 60
        )
        .into(),
    );
    window.set_frequency_note(frequency_note(report.sample_rate_hz).into());
    window.set_tempo_text(tempo_text(&report.tempo).into());
    let precision = source_bits
        .map(|bits| format!("Source bit depth: {bits}-bit."))
        .unwrap_or_else(|| "Source bit depth: unknown for this format.".into());
    let bandwidth = report
        .sustained_bandwidth_hz
        .map(|hz| {
            format!(
                "Sustained spectral extent: approximately {:.1} kHz.",
                hz / 1000.0
            )
        })
        .unwrap_or_else(|| {
            "Sustained spectral extent: inconclusive (silence, very short or weak audio).".into()
        });
    window.set_interpretation(format!("{bandwidth} Filtering, instruments and recording can also limit bandwidth. {precision} Lossless describes the codec; analysis cannot certify the original recording. Tempo and downbeat interpretation require review before automatic mixing.").into());
}

/// Final source detection must not rebuild an unchanged plot or reset the selected tempo interval.
fn same_measurements(previous: &TrackAnalysis, report: &TrackAnalysis) -> bool {
    previous.sample_rate_hz == report.sample_rate_hz
        && previous.channels == report.channels
        && previous.duration_seconds == report.duration_seconds
        && previous.columns == report.columns
        && previous.seconds_per_column == report.seconds_per_column
        && previous.sustained_bandwidth_hz == report.sustained_bandwidth_hz
        && previous.tempo == report.tempo
        && previous.power_db == report.power_db
}

/// Reports actual stages; the decode percentage never masquerades as overall completion.
fn analysis_progress(
    phase: resolibre_audio::session::AnalysisPhase,
    decoded_seconds: f64,
    duration: Option<f64>,
) -> String {
    use resolibre_audio::session::AnalysisPhase;
    match phase {
        AnalysisPhase::Opening => "Opening analysis…".into(),
        AnalysisPhase::LoadingSpectrogram => "Loading saved spectrogram…".into(),
        AnalysisPhase::Decoding => {
            if let Some(duration) = duration.filter(|value| *value > 0.0) {
                format!(
                    "Reading audio… {:.0}% · BPM and source checks follow",
                    (decoded_seconds / duration * 100.0).clamp(0.0, 100.0)
                )
            } else {
                format!("Reading audio… {decoded_seconds:.1} s decoded")
            }
        }
        AnalysisPhase::ResolvingTempo => "Resolving BPM and beat positions…".into(),
        AnalysisPhase::DetectingSource { completed, total } => {
            format!("Checking lossy-source evidence… {completed}/{total} excerpts complete")
        }
        AnalysisPhase::Saving => "Saving analysis…".into(),
    }
}

/// Distinguishes encoding from recording provenance; mixed-mode codecs stay unknown without stream evidence.
fn compression(codec: AudioCodec) -> &'static str {
    use resolibre_core::track_info::CompressionKind;
    match codec.compression() {
        CompressionKind::Lossless => "Lossless codec; origin unknown",
        CompressionKind::Lossy => "Lossy codec; origin unknown",
        CompressionKind::Unknown => "Compression mode unknown; origin unknown",
    }
}

/// Labels algorithm output as an estimate and preserves unknown/variable states instead of averaging them away.
pub(crate) fn tempo_text(report: &resolibre_core::analysis::tempo::TempoReport) -> String {
    if report.variable {
        if let Some((minimum, maximum)) = report.bpm_range() {
            return format!(
                "≈ {minimum:.1}–{maximum:.1} BPM · local estimates vary across the track"
            );
        }
        return "BPM: inconclusive — no usable local tempo estimates".into();
    }
    let Some(bpm) = report.bpm else {
        return "BPM: inconclusive — no sufficiently regular pulse measured".into();
    };
    let confidence = report
        .segments
        .iter()
        .map(|segment| segment.confidence)
        .fold(1.0_f64, f64::min);
    let beats: usize = report
        .segments
        .iter()
        .map(|segment| segment.beats.len())
        .sum();
    let alignment = if beats == 0 {
        "beat alignment uncertain".to_owned()
    } else {
        format!("{beats} observed beats")
    };
    format!("≈ {bpm:.1} BPM · estimated · periodicity {confidence:.2}/1 · {alignment}")
}

/// Produces a fixed scientific heatmap from measured power, resampling time to the exact decoded duration.
fn render(report: &TrackAnalysis) -> Image {
    let mut pixels = SharedPixelBuffer::<Rgb8Pixel>::new(512, ROWS as u32);
    let stops = [
        [8.0, 10.0, 16.0],
        [41.0, 32.0, 112.0],
        [168.0, 50.0, 112.0],
        [249.0, 137.0, 53.0],
        [255.0, 241.0, 160.0],
    ];
    for (index, pixel) in pixels.make_mut_slice().iter_mut().enumerate() {
        let time = (index % 512) as f64 / 512.0 * report.duration_seconds;
        let column = ((time / report.seconds_per_column) as usize).min(report.columns - 1);
        let row = ROWS - 1 - index / 512;
        let level = ((report.power_db[column * ROWS + row] + 120.0) / 120.0).clamp(0.0, 1.0) * 4.0;
        let stop = (level as usize).min(3);
        let mix = level - stop as f32;
        let mut color = [0; 3];
        for channel in 0..3 {
            color[channel] =
                (stops[stop][channel] * (1.0 - mix) + stops[stop + 1][channel] * mix) as u8;
        }
        *pixel = Rgb8Pixel::new(color[0], color[1], color[2]);
    }
    Image::from_rgb8(pixels)
}

/// Connects the artwork action once and retains only weak controller/window references in callbacks.
pub(crate) fn connect(
    main: &PlayerWindow,
    controller: &Rc<RefCell<Controller>>,
    windows: crate::window_group::SharedGroup,
) -> Rc<RefCell<AnalysisHost>> {
    let host = Rc::new(RefCell::new(AnalysisHost {
        windows,
        ..Default::default()
    }));
    main.on_open_analysis(open_action(main, controller, &host, false));
    main.on_reanalyze_current(open_action(main, controller, &host, true));
    main.on_open_track_analysis(track_action(main, controller, &host, false));
    main.on_reanalyze_track(track_action(main, controller, &host, true));
    host
}

/// Shares view eligibility and setup while dispatching exactly one cached-view or forced-analysis action.
fn open_action(
    main: &PlayerWindow,
    controller: &Rc<RefCell<Controller>>,
    host: &Rc<RefCell<AnalysisHost>>,
    force: bool,
) -> impl Fn() + 'static {
    let weak = main.as_weak();
    let owner = Rc::downgrade(controller);
    let view = Rc::downgrade(host);
    move || {
        if let (Some(main), Some(controller), Some(host)) =
            (weak.upgrade(), owner.upgrade(), view.upgrade())
            && main.get_analysis_enabled()
            && !main.get_closing()
            && !main.get_settings_loading()
            && let Err(error) = {
                let snapshot = controller.borrow().snapshot();
                let track_id = snapshot
                    .audible_next
                    .filter(|_| snapshot.display_next)
                    .and_then(|next| snapshot.tracks.get(next.index))
                    .map(|track| track.id);
                host.borrow_mut().show(&main, &controller, track_id, force)
            }
        {
            main.set_error_message(error.into());
            eprintln!("warn: {error}");
        }
    }
}

/// Routes a library badge or retry to its stable occurrence without requiring active playback.
fn track_action(
    main: &PlayerWindow,
    controller: &Rc<RefCell<Controller>>,
    host: &Rc<RefCell<AnalysisHost>>,
    force: bool,
) -> impl Fn(slint::SharedString) + 'static {
    let weak = main.as_weak();
    let owner = Rc::downgrade(controller);
    let view = Rc::downgrade(host);
    move |id| {
        if let (Some(main), Some(controller), Some(host), Ok(id)) = (
            weak.upgrade(),
            owner.upgrade(),
            view.upgrade(),
            id.parse::<u64>(),
        ) && !main.get_closing()
            && !main.get_settings_loading()
            && let Err(error) = host.borrow_mut().show(&main, &controller, Some(id), force)
        {
            main.set_error_message(error.into());
            eprintln!("warn: {error}");
        }
    }
}

/// Closing/cancelling targets the displayed request rather than whichever track is now playing.
fn cancel_command(generation: Option<u64>, id: Option<u64>) -> Command {
    match id {
        Some(id) => Command::CancelInspection { id },
        None => Command::CancelAnalysis {
            generation: generation.unwrap_or(0),
        },
    }
}

#[cfg(test)]
mod tests;

/// Explains why audio sample rate is twice the maximum frequency represented on the plot.
fn frequency_note(rate: u32) -> String {
    format!(
        "Sample rate: {:.1} kHz · Frequency range: 0–{:.2} kHz (half the sample rate)",
        f64::from(rate) / 1000.0,
        f64::from(rate) / 2000.0
    )
}
