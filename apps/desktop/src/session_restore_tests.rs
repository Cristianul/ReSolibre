//! Restart journeys through production bindings, the native worker and private storage.

use super::*;
use resolibre_audio::AudioOutput;
use resolibre_core::equalizer::{EqAction, Equalizer};
use slint::Model;
use std::path::PathBuf;

/// Polls the production close/startup path with a deadline instead of assuming I/O completion.
fn wait(
    window: &PlayerWindow,
    controller: &Rc<RefCell<Controller>>,
    ready: impl Fn(&Controller) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        controller.borrow_mut().refresh(window);
        if ready(&controller.borrow()) {
            return;
        }
        assert!(Instant::now() < deadline, "session persistence deadline");
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Constructs a fresh controller and real callbacks without touching the user's profile.
fn open(directory: PathBuf) -> (PlayerWindow, Rc<RefCell<Controller>>) {
    let window = PlayerWindow::new().unwrap();
    let session = Session::start(AudioOutput::Silent).unwrap();
    let controller = Rc::new(RefCell::new(Controller::new(session)));
    controller.borrow_mut().start_session_storage(directory);
    connect(&window, &controller);
    (window, controller)
}

/// Closes immediately after a control; accepted intent must settle and persist before exit.
fn close(window: &PlayerWindow, controller: &Rc<RefCell<Controller>>) {
    controller.borrow_mut().close();
    wait(window, controller, |c| {
        c.session.snapshot().finished && !c.storage.as_ref().unwrap().busy()
    });
}

/// Queue edits, duplicates, last-second settings and an empty queue survive real controller restarts.
#[test]
fn controls_and_close_restore_the_latest_session() {
    i_slint_backend_testing::init_no_event_loop();
    let directory = std::env::temp_dir().join(format!("resolibre-restore-ui-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let (window, controller) = open(directory.clone());
    // Startup cannot allow a later disk result to replace a newly accepted action.
    window.invoke_change_volume(13.0);
    assert_eq!(controller.borrow().expected_revision, 0);
    wait(&window, &controller, |c| {
        !c.restoring() && c.session.snapshot().revision >= c.expected_revision
    });
    let paths: Vec<PathBuf> = [
        "/unavailable/one.mp3",
        "/unavailable/two.flac",
        "/unavailable/one.mp3",
    ]
    .into_iter()
    .map(PathBuf::from)
    .collect();
    controller
        .borrow_mut()
        .submit(&window, Command::Add(paths.clone()));
    wait(&window, &controller, |c| {
        c.session.snapshot().tracks.len() == 3
    });
    let id = window.get_tracks().row_data(2).unwrap().id;
    window.invoke_move_track(id, false);
    window.invoke_toggle_shuffle();
    window.invoke_cycle_repeat();
    let eq = Equalizer::new(true, [4.0; 10], -1.0).unwrap();
    controller
        .borrow_mut()
        .submit(&window, Command::Equalizer(EqAction::Load(eq)));
    window.invoke_toggle_trim();
    window.invoke_cycle_fade();
    window.invoke_cycle_fade();
    window.invoke_cycle_fade();
    window.invoke_glide_volume(37.0);
    let themes = crate::themes::connect(&window, Default::default());
    window.invoke_select_theme(true);
    window.invoke_toggle_equalizer();
    let library = crate::playlist_window::connect(&window, Default::default(), Default::default());
    window.invoke_toggle_playlist();
    window.invoke_toggle_playlist();
    assert!(!window.get_playlist_requested());
    library.borrow_mut().close();
    themes.borrow_mut().close();
    close(&window, &controller);
    drop(library);
    drop(themes);
    drop(controller);
    drop(window);
    let (window, controller) = open(directory.clone());
    window.set_initializing(true);
    wait(&window, &controller, |c| {
        !c.restoring() && c.session.snapshot().revision >= c.expected_revision
    });
    let snapshot = controller.borrow().session.snapshot();
    assert_eq!(
        snapshot.saved.paths.as_ref(),
        &[paths[0].clone(), paths[2].clone(), paths[1].clone()]
    );
    assert_eq!(window.get_volume(), 37.0);
    assert!(window.get_spectrum());
    assert!(!window.get_eq_expanded());
    assert_eq!(window.get_eq_reveal(), 0.0);
    assert!(!controller.borrow().startup_pending());
    assert!(!window.get_playlist_requested());
    assert!(window.get_trim_enabled());
    assert_eq!(window.get_fade_mode(), 3);
    assert!(window.get_shuffle());
    assert_eq!(window.get_repeat_mode(), crate::RepeatState::All);
    assert_eq!(snapshot.equalizer, eq);
    assert_eq!(snapshot.generation, 0);
    assert!(!window.get_playing());
    assert!(snapshot.error.is_none());
    for track in snapshot.tracks.iter() {
        window.invoke_remove_track(track.id.to_string().into());
    }
    window.invoke_change_volume(0.0);
    close(&window, &controller);
    drop(controller);
    drop(window);
    let (window, controller) = open(directory.clone());
    wait(&window, &controller, |c| {
        !c.restoring() && c.session.snapshot().revision >= c.expected_revision
    });
    assert_eq!(window.get_tracks().row_count(), 0);
    assert_eq!(window.get_volume(), 0.0);
    assert!(window.get_spectrum());
    assert!(!window.get_eq_expanded());
    assert!(!window.get_playlist_requested());
    assert_eq!(controller.borrow().session.snapshot().equalizer, eq);
    close(&window, &controller);
    drop(controller);
    drop(window);
    // Closing before the first load completes must preserve the existing settings.
    let original = std::fs::read(directory.join("session-v1.bin")).unwrap();
    let (window, controller) = open(directory.clone());
    close(&window, &controller);
    drop(controller);
    drop(window);
    assert_eq!(
        std::fs::read(directory.join("session-v1.bin")).unwrap(),
        original
    );
    std::fs::remove_dir_all(directory).unwrap();
}

/// Cancel is harmless; confirmed reset drains pending writes and a new session starts with defaults.
#[test]
fn confirmed_reset_clears_profile_after_owned_workers_stop() {
    i_slint_backend_testing::init_no_event_loop();
    let directory = std::env::temp_dir().join(format!("resolibre-reset-ui-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let (window, controller) = open(directory.clone());
    window.show().unwrap();
    wait(&window, &controller, |c| !c.restoring());
    controller.borrow_mut().submit(
        &window,
        Command::Add(vec![PathBuf::from("/unavailable/music.flac")]),
    );
    wait(&window, &controller, |c| {
        c.session.snapshot().tracks.len() == 1
    });
    crate::ui_tests::click(&window, "Application menu");
    crate::ui_tests::click(&window, "Reset application data");
    assert!(window.get_reset_confirm());
    crate::ui_tests::click(&window, "Cancel reset");
    assert!(!window.get_reset_confirm());
    assert!(controller.borrow().close_started.is_none());
    assert_eq!(controller.borrow().session.snapshot().tracks.len(), 1);
    std::fs::create_dir(directory.join("analysis")).unwrap();
    std::fs::write(directory.join("analysis/analysis.sqlite3"), b"cache").unwrap();
    crate::ui_tests::click(&window, "Application menu");
    crate::ui_tests::click(&window, "Reset application data");
    crate::ui_tests::click(&window, "Reset and close ReSolibre");
    wait(&window, &controller, |c| {
        matches!(c.reset, reset::ResetState::Done)
    });
    assert!(controller.borrow().session.snapshot().finished);
    assert!(!directory.join("session-v1.bin").exists());
    assert!(!directory.join("analysis/analysis.sqlite3").exists());
    // Late UI refresh must never resurrect pre-reset autosaved state.
    controller.borrow_mut().refresh(&window);
    assert!(!directory.join("session-v1.bin").exists());
    drop(controller);
    window.hide().unwrap();
    drop(window);
    let (window, controller) = open(directory.clone());
    wait(&window, &controller, |c| !c.restoring());
    assert!(controller.borrow().session.snapshot().tracks.is_empty());
    assert!(!window.get_spectrum());
    close(&window, &controller);
    drop(controller);
    drop(window);
    std::fs::remove_dir_all(directory).unwrap();
}

/// A reset lease denial must reveal its error while keeping destructive/user controls blocked.
#[test]
fn startup_reports_profile_denial_instead_of_waiting_forever() {
    i_slint_backend_testing::init_no_event_loop();
    let directory =
        std::env::temp_dir().join(format!("resolibre-startup-denied-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let lease = crate::profile_storage::share_profile(&directory).unwrap();
    lease.make_exclusive().unwrap();
    let (window, controller) = open(directory.clone());
    wait(&window, &controller, |c| !c.startup_pending());
    assert!(controller.borrow().restoring());
    assert!(window.get_settings_loading());
    assert!(
        window
            .get_error_message()
            .contains("Profile reset in progress")
    );
    close(&window, &controller);
    drop(controller);
    drop(lease);
    std::fs::remove_dir_all(directory).unwrap();
}
