//! Presentation bindings, file chooser lifetime and snapshot-to-view conversion.

use crate::{PlayerWindow, TrackRow, picker::Picker, submit};
use resolibre_audio::session::{Command, Session, Snapshot};
use resolibre_core::queue::{QueueAction, RepeatMode};
use resolibre_core::session::{SessionState, Volume};
use slint::{ComponentHandle, ModelRc, VecModel};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

mod presets;
mod reset;

pub struct Controller {
    session: Session,
    storage: Option<crate::session_store::SessionStore>,
    pending_presets: Option<std::path::PathBuf>,
    reset: reset::ResetState,
    shutdown_requested: bool,
    restore_revision: Option<u64>,
    presets: Option<crate::preset_store::PresetStore>,
    preset_picker: Option<Picker>,
    preset_export: Option<resolibre_core::equalizer::preset::Preset>,
    preset_draft: Option<resolibre_core::equalizer::Equalizer>,
    picker: Option<Picker>,
    imports: crate::file_import::Importer,
    dropped: Option<crate::file_import::Ready>,
    expected_revision: u64,
    queue_revision: u64,
    analysis_pending: usize,
    analysis_marks: std::sync::Arc<
        std::collections::BTreeMap<u64, resolibre_audio::session::LibraryAnalysisStatus>,
    >,
    cover_revision: u64,
    pub(crate) generation: u64,
    close_started: Option<Instant>,
    local_error: Option<String>,
    spectrum: crate::live_spectrum::View,
}

impl Controller {
    /// Starts with empty presentation state; the worker supplies authoritative data.
    pub fn new(session: Session) -> Self {
        Self {
            session,
            storage: None,
            pending_presets: None,
            reset: Default::default(),
            shutdown_requested: false,
            restore_revision: None,
            presets: None,
            preset_picker: None,
            preset_export: None,
            preset_draft: None,
            picker: None,
            imports: Default::default(),
            dropped: None,
            expected_revision: 0,
            queue_revision: 0,
            analysis_marks: Default::default(),
            analysis_pending: 0,
            cover_revision: 0,
            generation: 0,
            close_started: None,
            local_error: None,
            spectrum: Default::default(),
        }
    }

    /// Updates badge observations independently of queue models, preserving active row gestures.
    fn refresh_analysis_marks(&mut self, window: &PlayerWindow, snapshot: &Snapshot) {
        use resolibre_audio::session::LibraryAnalysisStatus as Status;
        if self.analysis_pending == snapshot.analysis_pending
            && std::sync::Arc::ptr_eq(&self.analysis_marks, &snapshot.library_analysis)
        {
            return;
        }
        self.analysis_pending = snapshot.analysis_pending;
        if !std::sync::Arc::ptr_eq(&self.analysis_marks, &snapshot.library_analysis) {
            let rows: Vec<_> = snapshot
                .library_analysis
                .iter()
                .map(|(id, status)| crate::AnalysisRow {
                    id: id.to_string().into(),
                    status: match status {
                        Status::Queued => 1,
                        Status::Running => 2,
                        Status::Complete => 3,
                        Status::Failed => 4,
                    },
                })
                .collect();
            window.set_analysis_states(ModelRc::new(VecModel::from(rows)));
            self.analysis_marks = snapshot.library_analysis.clone();
        }
        let complete = self
            .analysis_marks
            .values()
            .filter(|status| **status == Status::Complete)
            .count();
        let failed = self
            .analysis_marks
            .values()
            .filter(|status| **status == Status::Failed)
            .count();
        window.set_analysis_busy(snapshot.analysis_pending > 0);
        window.set_analysis_summary(
            format!(
                "{complete} analyzed · {failed} failed · {} pending",
                snapshot.analysis_pending
            )
            .into(),
        );
    }

    /// Returns the last accepted command revision so auxiliary views can reject pre-request snapshots.
    pub(crate) fn expected_revision(&self) -> u64 {
        self.expected_revision
    }

    /// Copies the typed session observation for auxiliary views without exposing native handles.
    pub(crate) fn snapshot(&self) -> Snapshot {
        self.session.snapshot()
    }

    /// Starts per-user adapters; storage failures remain visible without disabling playback.
    pub fn start_default_storage(&mut self) {
        match crate::profile_storage::default_directory() {
            Ok(directory) => {
                self.pending_presets = Some(directory.clone());
                self.start_session_storage(directory);
            }
            Err(error) => {
                eprintln!("warn: {error}");
                self.local_error = Some(error.into());
            }
        }
    }

    /// Attaches only the current profile's preset adapter; tests supply an isolated directory.
    pub fn start_presets(&mut self, directory: std::path::PathBuf) {
        match crate::preset_store::PresetStore::start(directory) {
            Ok(store) => self.presets = Some(store),
            Err(error) => {
                eprintln!("warn: {error}");
                self.local_error = Some(error.into());
            }
        }
    }

    /// Attaches session persistence independently of named EQ presets; tests use private profiles.
    pub fn start_session_storage(&mut self, directory: std::path::PathBuf) {
        match crate::session_store::SessionStore::start(directory) {
            Ok(store) => self.storage = Some(store),
            Err(error) => {
                eprintln!("warn: {error}");
                self.local_error = Some(error.into());
            }
        }
    }

    /// Prevents startup restore from racing new controls; no user command is silently discarded.
    pub fn restoring(&self) -> bool {
        self.storage.as_ref().is_some_and(|store| {
            store.loading() || (!store.profile_available() && self.close_started.is_none())
        }) || self
            .restore_revision
            .is_some_and(|revision| self.session.snapshot().revision < revision)
    }

    /// Waits for initial state, but lets profile errors appear instead of hiding the app forever.
    pub fn startup_pending(&self) -> bool {
        self.storage.as_ref().is_some_and(|store| store.loading())
            || self
                .restore_revision
                .is_some_and(|revision| self.session.snapshot().revision < revision)
            || self.presets.as_ref().is_some_and(|store| store.busy())
    }

    /// Polls storage and restores through the worker before accepting fresh user intent.
    fn poll_storage(&mut self, window: &PlayerWindow) {
        let Some(store) = self.storage.as_mut() else {
            return;
        };
        let result = store.poll(self.close_started.is_some());
        let profile_available = store.profile_available();
        match result {
            Ok(Some(saved)) => {
                saved.interface.apply(window);
                match self.session.submit(Command::Restore(saved.audio)) {
                    Ok(revision) => {
                        self.expected_revision = revision;
                        self.restore_revision = Some(revision);
                    }
                    Err(error) => {
                        eprintln!("warn: {error}");
                        self.local_error = Some(error.into());
                        self.storage = None;
                    }
                }
            }
            Ok(None) => {}
            Err(error) => {
                eprintln!("warn: {error}");
                self.local_error = Some(error.into());
            }
        }
        if profile_available && let Some(directory) = self.pending_presets.take() {
            self.start_presets(directory);
        }
    }

    /// Rejects stale observations until the latest accepted command is processed.
    pub fn submit(&mut self, window: &PlayerWindow, command: Command) {
        if self.close_started.is_some() || self.restoring() {
            window.set_error_message("Session is starting or closing; please wait".into());
            return;
        }
        match self.session.submit(command) {
            Ok(revision) => {
                self.expected_revision = revision;
                self.local_error = None;
            }
            Err(message) => {
                self.local_error = Some(message.into());
                window.set_error_message(message.into());
            }
        }
    }

    /// Opens at most one owned helper process; cancellation preserves playback.
    fn pick(&mut self, window: &PlayerWindow, append: bool) {
        if self.preset_picker.is_some()
            || self.picker.is_some()
            || self.imports.reserved()
            || self.close_started.is_some()
            || self.restoring()
        {
            return;
        }
        match Picker::start(append) {
            Ok(picker) => {
                self.picker = Some(picker);
                window.set_picking(true);
            }
            Err(message) => {
                self.local_error = Some(message.into());
                window.set_error_message(message.into());
            }
        }
    }

    /// Cancels the chooser; refresh drains accepted commands before final save and cleanup.
    pub fn close(&mut self) {
        if matches!(
            self.reset,
            reset::ResetState::Failed | reset::ResetState::Done
        ) {
            let _ = slint::quit_event_loop();
            return;
        }
        if self.close_started.is_none() {
            self.close_started = Some(Instant::now());
            self.spectrum.stop();
            self.picker = None;
            self.preset_picker = None;
            self.preset_export = None;
            self.preset_draft = None;
            self.dropped = None;
            self.imports.close();
        }
    }

    /// Polls bounded picker/session state; a slow close remains visible, never detached.
    pub fn refresh(&mut self, window: &PlayerWindow) {
        if self.poll_reset(window) {
            return;
        }
        self.poll_storage(window);
        window.set_settings_loading(self.restoring());
        let snapshot = self.session.snapshot();
        if !self.restoring()
            && snapshot.revision >= self.expected_revision
            && let Some(store) = self.storage.as_mut()
        {
            store.observe(crate::session_store::StoredSession {
                audio: snapshot.saved.clone(),
                interface: crate::interface_state::InterfaceState::capture(window),
            });
        }
        self.poll_presets(window);

        if let Some(started) = self.close_started {
            window.set_drop_menu(false);
            let imports_finished = self.imports.close();
            if self.restoring() || snapshot.revision < self.expected_revision {
                window.set_error_message("Finishing session changes…".into());
                return;
            }
            if !self.shutdown_requested {
                self.session.request_shutdown();
                self.shutdown_requested = true;
            }
            if !imports_finished {
                window.set_error_message("Cancelling folder import…".into());
                return;
            }
            if self.storage.as_ref().is_some_and(|store| store.busy()) {
                window.set_error_message("Saving session…".into());
                return;
            }
            if self.presets.as_ref().is_some_and(|store| store.busy()) {
                window.set_error_message("Finishing preset storage…".into());
                return;
            }

            match self.session.finish() {
                Ok(true) => {
                    if self.reset.requested() {
                        self.start_reset(window);
                    } else {
                        let _ = slint::quit_event_loop();
                    }
                }
                Ok(false) => {
                    if started.elapsed() > Duration::from_secs(5) {
                        window.set_error_message("Still waiting for audio cleanup…".into());
                    }
                }
                Err(message) => {
                    eprintln!("error: {message}");
                    let _ = slint::quit_event_loop();
                }
            }
            return;
        }
        self.poll_picker(window);
        if !self.restoring() && self.picker.is_none() && snapshot.revision >= self.expected_revision
        {
            self.poll_import(window);
        }
        let snapshot = self.session.snapshot();
        if snapshot.revision < self.expected_revision {
            return;
        }
        self.generation = snapshot.generation;
        if snapshot.queue_revision != self.queue_revision {
            let rows: Vec<_> = snapshot
                .tracks
                .iter()
                .enumerate()
                .map(|(index, track)| TrackRow {
                    id: track.id.to_string().into(),
                    title: track.title.as_str().into(),
                    search_text: track.search_text.as_str().into(),
                    duration: crate::library_duration::track(track.duration_seconds).into(),
                    number: (index + 1).to_string().into(),
                })
                .collect();
            window.set_tracks(ModelRc::new(VecModel::from(rows)));
            window.set_library_duration(crate::library_duration::total(&snapshot.tracks).into());
            self.queue_revision = snapshot.queue_revision;
        }
        self.refresh_analysis_marks(window, &snapshot);
        if snapshot.cover_revision != self.cover_revision {
            crate::artwork::update(window, &snapshot);
            self.cover_revision = snapshot.cover_revision;
        }
        window.set_analysis_enabled(matches!(
            snapshot.state,
            SessionState::Playing | SessionState::Paused
        ));
        show_snapshot(window, &snapshot);
        self.spectrum.observe(window, &snapshot);
        let error = self.local_error.as_ref().or(snapshot.error.as_ref());
        window.set_error_message(error.map(String::as_str).unwrap_or("").into());
        if self.session.is_finished() && !snapshot.finished {
            window.set_error_message("Playback worker stopped unexpectedly; reopen ReSolibre".into());
        }
    }

    /// Exposes the single bounded inbox to both native window hosts.
    pub fn import_inbox(&self) -> Rc<RefCell<crate::file_import::Inbox>> {
        self.imports.inbox.clone()
    }

    /// Expands dropped folders without blocking playback, then asks only for a nonempty queue.
    fn poll_import(&mut self, window: &PlayerWindow) {
        if let Some(notice) = self.imports.take_notice() {
            self.local_error = Some(notice.into());
        }
        if let Some(result) = self.imports.poll() {
            match result {
                Ok(ready) if ready.selection.paths.is_empty() => {
                    window.set_import_status("".into());
                    self.imports.finish();
                }
                Ok(ready) => {
                    self.local_error = None;
                    window.set_drop_playlist(
                        ready.target == crate::file_import::Target::Playlist
                            && window.get_playlist_visible(),
                    );
                    window.set_drop_x(ready.position.0);
                    window.set_drop_y(ready.position.1);
                    self.dropped = Some(ready);
                    if self.session.snapshot().tracks.is_empty() {
                        self.choose_drop(window, 0);
                    } else {
                        window.set_drop_menu(true);
                        window
                            .set_import_status("Add dropped music or replace the library?".into());
                    }
                }
                Err(error) => {
                    eprintln!("warn: {error}");
                    self.local_error = Some(error.into());
                    window.set_import_status("".into());
                    self.imports.finish();
                }
            }
        }
        if self.imports.scanning() {
            window.set_import_status("Reading dropped files and folders…".into());
        }
        window.set_picking(self.imports.reserved());
    }

    /// Consumes one menu decision through shared session commands; dismissal changes nothing.
    pub fn choose_drop(&mut self, window: &PlayerWindow, choice: i32) {
        if choice != -1 && choice != 0 && choice != 1 {
            return;
        }
        if choice >= 0 && (self.close_started.is_some() || self.restoring()) {
            return;
        }
        let Some(ready) = self.dropped.take() else {
            return;
        };
        window.set_drop_menu(false);
        window.set_import_status("".into());
        window.set_picking(false);
        self.imports.finish();
        if choice == -1 {
            return;
        }
        let command = if choice == 1 {
            Command::Replace(ready.selection.paths)
        } else {
            Command::Add(ready.selection.paths)
        };
        self.submit(window, command);
    }

    /// Completes one picker result; no cancellation or error replaces a playing track.
    fn poll_picker(&mut self, window: &PlayerWindow) {
        let Some(picker) = self.picker.as_mut() else {
            return;
        };
        let append = picker.append;
        match picker.poll() {
            Ok(None) => return,
            Ok(Some(mut paths)) => {
                paths.retain(|path| resolibre_audio::media_formats::is_candidate(path));
                if !paths.is_empty() {
                    let command = if append {
                        Command::Add(paths)
                    } else {
                        Command::Open(paths)
                    };
                    self.submit(window, command);
                }
            }
            Err(message) => {
                self.local_error = Some(message.into());
            }
        }
        self.picker = None;
        window.set_picking(false);
    }
}

/// Converts finite engine seconds into a display without claiming physical audio position.
fn clock_label(seconds: Option<f64>) -> String {
    match seconds {
        Some(value) if value.is_finite() && value >= 0.0 => {
            let total = value.min(u32::MAX as f64) as u64;
            format!("{}:{:02}", total / 60, total % 60)
        }
        _ => "—:—".into(),
    }
}

/// Updates display properties; queue models are replaced only on queue revisions.
fn show_snapshot(window: &PlayerWindow, snapshot: &Snapshot) {
    crate::equalizer::show(window, snapshot);
    crate::playback_edges::show(window, snapshot);
    if let Some(track) = snapshot
        .current
        .and_then(|index| snapshot.tracks.get(index))
    {
        window.set_track_title(track.title.as_str().into());
    } else {
        window.set_track_title("Your music. Ready when you are.".into());
    }
    let label = match snapshot.state {
        SessionState::Idle => "Open a local audio file to begin",
        SessionState::Loading => "Loading local audio…",
        SessionState::Playing => "Playing · Local audio",
        SessionState::Paused => "Paused · Local audio",
        SessionState::Stopped => "Stopped · Play to restart",
        SessionState::Ended => "Playback finished · Play to restart",
        SessionState::Failed => "Playback failed · Play to retry, or choose another track",
    };
    let label = match snapshot.scanning {
        Some(resolibre_core::session::ScanDirection::Forward) => "Scanning forward · 3×",
        Some(resolibre_core::session::ScanDirection::Backward) => "Scanning backward · 3×",
        None => label,
    };
    let label = match snapshot.edge_status {
        resolibre_audio::session::EdgeStatus::Pending => "Detecting track edges…",
        resolibre_audio::session::EdgeStatus::Unavailable => "TRIM/FADE unavailable · Original audio",
        _ => label,
    };
    window.set_state_label(label.into());
    crate::track_info::show(window, snapshot);
    window.set_playing(snapshot.state == SessionState::Playing);
    window.set_can_play(!snapshot.tracks.is_empty());
    window.set_can_seek(snapshot.seekable && snapshot.duration.is_some());
    window.set_can_previous(snapshot.can_previous);
    window.set_previous_navigates(snapshot.previous_navigates);
    window.set_can_next(snapshot.can_next);
    window.set_shuffle(snapshot.shuffle);
    window.set_repeat_mode(match snapshot.repeat {
        RepeatMode::Off => crate::RepeatState::Off,
        RepeatMode::All => crate::RepeatState::All,
        RepeatMode::One => crate::RepeatState::One,
    });
    window.set_current(snapshot.current.map(|index| index as i32).unwrap_or(-1));
    window.set_position(snapshot.position.unwrap_or(0.0) as f32);
    window.set_duration(snapshot.duration.unwrap_or(1.0) as f32);
    window.set_time_label(clock_label(snapshot.position).into());
    window.set_duration_label(clock_label(snapshot.duration).into());
    window.set_volume(snapshot.volume.percent() as f32);
}

/// Binds a transport action without retaining a strong UI reference in its callbacks.
fn transport(
    window: &PlayerWindow,
    controller: &Rc<RefCell<Controller>>,
    action: fn() -> Command,
) -> impl Fn() + 'static {
    let weak = window.as_weak();
    let controller = Rc::clone(controller);
    move || {
        if let Some(window) = weak.upgrade() {
            submit(&window, &controller, action());
        }
    }
}

/// Wires controls once; changing lower panels never reconstructs the session.
pub fn connect(window: &PlayerWindow, controller: &Rc<RefCell<Controller>>) {
    let reset_window = window.as_weak();
    let reset_controller = controller.clone();
    window.on_reset_application(move || {
        if let Some(window) = reset_window.upgrade() {
            reset_controller.borrow_mut().request_reset(&window);
        }
    });
    window.set_settings_loading(controller.borrow().restoring());
    window.set_reduced_motion(std::env::var_os("RESOUL_REDUCED_MOTION").is_some());
    let weak = window.as_weak();
    let owner = Rc::downgrade(controller);
    window.on_drop_choice(move |choice| {
        if let (Some(window), Some(controller)) = (weak.upgrade(), owner.upgrade()) {
            controller.borrow_mut().choose_drop(&window, choice);
        }
    });
    crate::equalizer::connect(window, controller);
    crate::transport_scan::connect(window, controller);
    crate::playback_edges::connect(window, controller);
    window.on_play_pause(transport(window, controller, || Command::TogglePause));
    window.on_stop(transport(window, controller, || Command::Stop));
    window.on_analyze_all(transport(window, controller, || Command::AnalyzeAll));
    window.on_cancel_library_analysis(transport(window, controller, || Command::CancelAllAnalysis));
    let weak = window.as_weak();
    let controls = Rc::clone(controller);
    window.on_analyze_track(move |id| {
        if let (Some(window), Ok(id)) = (weak.upgrade(), id.parse::<u64>()) {
            submit(&window, &controls, Command::AnalyzeTrack { id });
        }
    });
    window.on_toggle_shuffle(transport(window, controller, || {
        Command::Queue(QueueAction::ToggleShuffle)
    }));
    window.on_cycle_repeat(transport(window, controller, || {
        Command::Queue(QueueAction::CycleRepeat)
    }));
    window.on_next(transport(window, controller, || Command::Next));
    window.on_previous(transport(window, controller, || Command::Previous));
    // Gesture state is independent: a snapshot can disable/cancel the slider while
    // the controller is already borrowed for refresh. Cancellation must not reborrow it.
    let gesture_generation = Rc::new(Cell::new(None));
    let gesture = Rc::clone(&gesture_generation);
    let controls = Rc::clone(controller);
    window.on_seek_started(move || {
        gesture.set(Some(controls.borrow().generation));
    });
    let gesture = Rc::clone(&gesture_generation);
    window.on_seek_ended(move || gesture.set(None));
    let weak = window.as_weak();
    let controls = Rc::clone(controller);
    window.on_seek(move |seconds| {
        if let Some(window) = weak.upgrade() {
            let generation = gesture_generation
                .get()
                .unwrap_or(controls.borrow().generation);
            submit(
                &window,
                &controls,
                Command::Seek {
                    generation,
                    seconds: seconds as f64,
                },
            );
        }
    });
    let weak = window.as_weak();
    let controls = Rc::clone(controller);
    window.on_change_volume(move |percent| {
        if let (Some(window), Some(volume)) = (weak.upgrade(), Volume::from_percent(percent as f64))
        {
            submit(&window, &controls, Command::Volume(volume));
        }
    });
    let weak = window.as_weak();
    let controls = Rc::clone(controller);
    window.on_glide_volume(move |percent| {
        if let (Some(window), Some(volume)) = (weak.upgrade(), Volume::from_percent(percent as f64))
        {
            submit(&window, &controls, Command::GlideVolume(volume));
        }
    });
    let weak = window.as_weak();
    let controls = Rc::clone(controller);
    window.on_select_track(move |id| queue_action(&weak, &controls, id, QueueAction::Select));
    let weak = window.as_weak();
    let controls = Rc::clone(controller);
    window.on_remove_track(move |id| queue_action(&weak, &controls, id, QueueAction::Remove));
    let weak = window.as_weak();
    let controls = Rc::clone(controller);
    window.on_move_track(move |id, forward| {
        queue_action(&weak, &controls, id, |id| QueueAction::Move { id, forward });
    });
    let weak = window.as_weak();
    let controls = Rc::clone(controller);
    window.on_reorder_track(move |id, target, after| {
        if let Ok(target) = target.parse::<u64>() {
            queue_action(&weak, &controls, id, |id| QueueAction::MoveTo {
                id,
                target,
                after,
            });
        }
    });
    let weak = window.as_weak();
    let controls = Rc::clone(controller);
    window.on_open_files(move || {
        if let Some(window) = weak.upgrade() {
            controls.borrow_mut().pick(&window, false);
        }
    });
    let weak = window.as_weak();
    let controls = Rc::clone(controller);
    window.on_add_files(move || {
        if let Some(window) = weak.upgrade() {
            controls.borrow_mut().pick(&window, true);
        }
    });
    let weak = window.as_weak();
    let resize_timer = slint::Timer::default();
    window.on_eq_visibility_changed(move |_| {
        if let Some(window) = weak.upgrade() {
            let size = window
                .window()
                .size()
                .to_logical(window.window().scale_factor());
            resize_for_equalizer(&window, &resize_timer, size.width);
        }
    });
}

/// Expands before the drawer slides out, but shrinks only after its closing animation.
/// Restarting this one-shot timer cancels obsolete sizes during rapid reversals.
pub(crate) fn resize_for_equalizer(window: &PlayerWindow, timer: &slint::Timer, width: f32) {
    if window.get_initializing() {
        timer.stop();
        let height = window.invoke_window_height_for_eq(window.get_eq_expanded());
        window
            .window()
            .set_size(slint::LogicalSize::new(width.max(800.0), height));
        return;
    }
    let weak = window.as_weak();
    let closing_delay = if window.get_eq_expanded() {
        0
    } else {
        window.get_drawer_duration().max(0) as u64
    };
    timer.start(
        slint::TimerMode::SingleShot,
        Duration::from_millis(closing_delay + 16),
        move || {
            if let Some(window) = weak.upgrade()
                && !window.get_closing()
            {
                let height = window.invoke_window_height_for_eq(window.get_eq_expanded());
                window
                    .window()
                    .set_size(slint::LogicalSize::new(width, height));
            }
        },
    );
}

/// Converts an opaque display ID at one boundary and dispatches the shared queue action.
fn queue_action(
    window: &slint::Weak<PlayerWindow>,
    controller: &Rc<RefCell<Controller>>,
    id: slint::SharedString,
    action: impl FnOnce(u64) -> QueueAction,
) {
    if let Some(window) = window.upgrade()
        && let Ok(id) = id.parse::<u64>()
    {
        submit(&window, controller, Command::Queue(action(id)));
    }
}

#[cfg(test)]
#[path = "session_restore_tests.rs"]
mod session_restore_tests;
