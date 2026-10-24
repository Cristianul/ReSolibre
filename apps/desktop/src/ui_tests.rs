//! UI interactions use the actual compiled controls, with native preview kept explicit.

use crate::*;
use i_slint_backend_testing::ElementHandle;
use slint::Model;
use slint::{ModelRc, VecModel};

/// Finds a real accessible control and activates its production callback.
pub(super) fn click(window: &impl ComponentHandle, label: &str) {
    let element = ElementHandle::find_by_accessible_label(window, label)
        .next()
        .expect("accessible control");
    assert_eq!(element.accessible_enabled(), Some(true));
    element.invoke_accessible_default_action();
}

/// Independent visibility preserves track/gain/time, with EQ expanded on startup.
#[test]
fn drawer_visibility_does_not_reset_player() {
    i_slint_backend_testing::init_no_event_loop();
    let window = PlayerWindow::new().unwrap();
    window.set_track_title("Long Unicode title — 音楽 <text>".into());
    window.set_volume(27.0);
    window.set_position(103.0);
    window.set_duration(140.0);
    window.set_can_play(true);
    window.set_playing(true);
    window.show().unwrap();
    let host = playlist_window::connect(&window, Default::default(), Default::default());
    host.borrow_mut().show(&window, &host).unwrap();
    assert!(window.get_eq_expanded());
    click(&window, "Library");
    assert!(!window.get_playlist_visible());
    assert!(window.get_eq_expanded());
    click(&window, "Equalizer");
    assert!(!window.get_eq_expanded());
    click(&window, "Equalizer");
    assert!(window.get_eq_expanded());
    assert_eq!(window.get_position(), 103.0);
    assert_eq!(window.get_volume(), 27.0);
    assert!(window.get_playing());
    assert!(window.get_track_title().contains("音楽"));
    host.borrow_mut().close();
}

/// Pointer/keyboard-accessible actions and sliders invoke typed application callbacks.
#[test]
fn transport_and_slider_accessibility_are_wired() {
    i_slint_backend_testing::init_no_event_loop();
    let window = PlayerWindow::new().unwrap();
    let calls = Rc::new(RefCell::new(0));
    let count = Rc::clone(&calls);
    window.on_play_pause(move || *count.borrow_mut() += 1);
    window.set_can_play(true);
    window.set_can_seek(true);
    window.set_duration(130.0);
    window.show().unwrap();
    click(&window, "Play");
    assert_eq!(*calls.borrow(), 1);
    let seek = ElementHandle::find_by_accessible_label(&window, "Playback position")
        .next()
        .unwrap();
    assert_eq!(seek.accessible_value_maximum(), Some(130.0));
    let observed = Rc::new(RefCell::new(0.0));
    let changed = Rc::clone(&observed);
    window.on_seek(move |value| *changed.borrow_mut() = value);
    seek.set_accessible_value("103");
    assert_eq!(*observed.borrow(), 103.0);
}

/// Renders both palettes, panel states and minimum-size edge cases on an isolated native display.
#[test]
#[ignore = "requires a desktop session and RESOUL_PREVIEW_DIR"]
fn native_preview() {
    crate::window_snap::select_x11().unwrap();
    let directory = std::path::PathBuf::from(
        std::env::var_os("RESOUL_PREVIEW_DIR").expect("preview directory"),
    );
    assert!(directory.is_dir());
    let window = PlayerWindow::new().unwrap();
    // Static layout captures settle immediately; dedicated drawer tests exercise motion.
    window.set_reduced_motion(true);
    crate::equalizer::initialize(&window);
    window.set_track_title("Night Drive.flac".into());
    window.set_state_label("Paused · Local audio".into());
    window.set_lossless(true);
    window.set_technical_info("FLAC · 44.1 kHz · 24-bit · Stereo · ≈ 120.0 BPM".into());
    window.set_live_available(true);
    window.set_live_levels(ModelRc::new(VecModel::from(
        (0..24)
            .map(|index| 0.25 + ((index as f32 * 0.45).sin() + 1.0) * 0.3)
            .collect::<Vec<_>>(),
    )));
    window.set_time_label("2:09".into());
    window.set_duration_label("5:22".into());
    window.set_position(129.0);
    window.set_duration(322.0);
    window.set_can_play(true);
    window.set_can_seek(true);
    window.set_can_next(true);
    window.set_current(0);
    window.set_tracks(ModelRc::new(VecModel::from(vec![
        TrackRow {
            duration: "--:--".into(),
            search_text: Default::default(),
            title: "Night Drive.flac".into(),
            number: "1".into(),
            id: "1".into(),
        },
        TrackRow {
            duration: "--:--".into(),
            search_text: Default::default(),
            title: "Static Bloom.mp3".into(),
            number: "2".into(),
            id: "2".into(),
        },
        TrackRow {
            duration: "--:--".into(),
            search_text: Default::default(),
            title: "After Hours.flac".into(),
            number: "3".into(),
            id: "3".into(),
        },
    ])));
    window.set_eq_store_busy(false);
    let cases = [
        ("green-compact", 0, false, 800.0),
        ("green-equalizer", 2, false, 800.0),
        ("spectrum-equalizer", 2, true, 800.0),
        ("minimum-equalizer", 2, true, 760.0),
        ("minimum-long-title", 0, true, 760.0),
        ("minimum-empty", 0, false, 760.0),
        ("minimum-error", 0, false, 760.0),
        ("preset-name", 2, false, 800.0),
    ];
    let timer = slint::Timer::default();
    let resize_timer = slint::Timer::default();
    let weak = window.as_weak();
    let mut step = 0;
    timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_millis(200),
        move || {
            let window = weak.upgrade().unwrap();
            let (name, panel, spectrum, width) = cases[step / 2];
            if step % 2 == 0 {
                window.set_eq_expanded(panel == 2);
                window.set_spectrum(spectrum);
                window.set_eq_enabled(true);
                crate::presentation::resize_for_equalizer(&window, &resize_timer, width);
                if name == "preset-name" {
                    window.set_eq_preset_name("Night headphones".into());
                    window.set_eq_preset_message("".into());
                    window.set_eq_dialog_open(true);
                }
                if name == "minimum-long-title" {
                    window.set_track_title("Long Unicode filename — 音楽 — ".repeat(8).into());
                }
                if name == "minimum-empty" {
                    window.set_live_levels(ModelRc::new(VecModel::from(vec![0.0; 24])));
                    window.set_live_available(false);
                    window.set_technical_info("".into());
                    window.set_tracks(ModelRc::default());
                    window.set_track_title("Your music. Ready when you are.".into());
                    window.set_state_label("Open a local audio file to begin".into());
                    window.set_can_play(false);
                    window.set_can_seek(false);
                    window.set_can_next(false);
                    window.set_position(0.0);
                    window.set_duration(1.0);
                    window.set_time_label("—:—".into());
                    window.set_duration_label("—:—".into());
                }
                if name == "minimum-error" {
                    window.set_error_message(
                        "Playback failed. Play to retry or choose another track.".into(),
                    );
                }
            } else {
                let pixels = window.window().take_snapshot().unwrap();
                assert_eq!(pixels.as_bytes()[3], 0, "window corner must be transparent");
                let scale = std::env::var("WINIT_X11_SCALE_FACTOR").unwrap_or("1".into());
                let factor: f32 = scale.parse().unwrap();
                assert_eq!(
                    pixels.width(),
                    (width * factor) as u32,
                    "{name}: native width"
                );
                assert_eq!(
                    pixels.height(),
                    (window.invoke_window_height_for_eq(panel == 2) * factor) as u32,
                    "{name}: native height"
                );
                image::save_buffer(
                    directory.join(format!("{name}-{scale}x.png")),
                    pixels.as_bytes(),
                    pixels.width(),
                    pixels.height(),
                    image::ColorType::Rgba8,
                )
                .unwrap();
            }
            step += 1;
            if step == cases.len() * 2 {
                slint::quit_event_loop().unwrap();
            }
        },
    );
    window.run().unwrap();
}

/// Generates owned silence for a real session-to-UI journey, keeping music out of tests.
pub(crate) fn write_ui_fixture(directory: &std::path::Path) -> std::path::PathBuf {
    let path = directory.join("Generated audio.wav");
    let size = 130_u32 * 8000 * 2;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + size).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt \x10\0\0\0\x01\0\x01\0");
    bytes.extend_from_slice(&8000_u32.to_le_bytes());
    bytes.extend_from_slice(&16000_u32.to_le_bytes());
    bytes.extend_from_slice(b"\x02\0\x10\0data");
    bytes.extend_from_slice(&size.to_le_bytes());
    bytes.resize(44 + size as usize, 0);
    std::fs::write(&path, bytes).unwrap();
    path
}

/// Runs production bindings against native audio while switching palettes/panels and using controls.
#[test]
#[ignore = "requires a real desktop session"]
fn native_session_journey() {
    crate::window_snap::select_x11().unwrap();
    use std::time::{Duration, Instant};
    let directory = std::env::temp_dir().join(format!("resolibre-ui-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let path = write_ui_fixture(&directory);
    let window = PlayerWindow::new().unwrap();
    let session = Session::start(AudioOutput::Silent).unwrap();
    let controller = Rc::new(RefCell::new(presentation::Controller::new(session)));
    presentation::connect(&window, &controller);
    controller
        .borrow_mut()
        .submit(&window, Command::Open(vec![path.clone(), path.clone()]));
    let host = playlist_window::connect(&window, Default::default(), Default::default());
    host.borrow_mut().show(&window, &host).unwrap();
    let themes = crate::themes::connect(&window, Default::default());
    window.invoke_open_themes();
    let chooser = themes.borrow().test_window();
    let timer = slint::Timer::default();
    let start = Instant::now();
    let mut step = 0;
    let mut cycles = 0;
    let weak = window.as_weak();
    let mut callback_times = Vec::new();
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(33), move || {
        assert!(start.elapsed() < Duration::from_secs(30), "native UI journey deadline");
        let window = weak.upgrade().unwrap();
        controller.borrow_mut().refresh(&window);
        host.borrow().refresh(&window);
        match step {
            0 if window.get_playing() && window.get_can_seek() => {
                click(&chooser, "Spectrum palette"); click(&chooser, "Green palette");
                assert!(window.get_playing());
                click(&window, "Pause"); step += 1;
            }
            1 if window.get_state_label().starts_with("Paused") => {
                let seek = ElementHandle::find_by_accessible_label(&window, "Playback position").next().unwrap();
                seek.set_accessible_value("103"); step += 1;
            }
            2 if (window.get_position() - 103.0).abs() < 0.5 => {
                for _ in 0..30 {
                    let submitted = Instant::now();
                    click(&chooser, "Spectrum palette"); click(&window, "Library"); click(&window, "Equalizer"); click(&chooser, "Green palette"); click(&window, "Equalizer");
                    callback_times.push(submitted.elapsed().as_secs_f64() * 1000.0);
                }
                assert!(window.get_eq_expanded());
                assert!(!window.get_spectrum());
                assert!((window.get_position() - 103.0).abs() < 0.5);
                assert!(!window.get_playing());
                click(&window, "Next track"); step += 1;
            }
            3 if window.get_current() == 1 && window.get_state_label().starts_with("Paused") => {
                click(&window, "Play"); step += 1;
            }
            4 if window.get_playing() => { click(&window, "Stop"); step += 1; }
            5 if window.get_state_label().starts_with("Stopped") => { click(&window, "Play"); step += 1; }
            6 if window.get_playing() => {
                cycles += 1;
                if cycles < 12 {
                    controller.borrow_mut().submit(&window, Command::Queue(resolibre_core::queue::QueueAction::Select(1)));
                    step = 0;
                } else {
                    callback_times.sort_by(f64::total_cmp);
                    let p95 = callback_times[(callback_times.len() * 95 / 100) - 1];
                    println!("native-ui cycles={cycles}, panel callback p95_ms={p95:.3}, max_ms={:.3}", callback_times.last().unwrap());
                    assert!(p95 <= 50.0, "UI acknowledgement gate");
                    host.borrow_mut().close();
                    themes.borrow_mut().close();
                    controller.borrow_mut().close(); step += 1;
                }
            }
            _ => {}
        }
    });
    window.run().unwrap();
    assert_eq!(std::fs::metadata(&path).unwrap().len(), 44 + 130 * 8000 * 2);
    std::fs::remove_dir_all(directory).unwrap();
}

/// Real pointer focus enables Space/arrow keys, and panel replacement restores toggle focus.
#[test]
fn keyboard_controls_and_scaled_layout_remain_usable() {
    i_slint_backend_testing::init_no_event_loop();
    let window = PlayerWindow::new().unwrap();
    window.set_can_play(true);
    window.set_track_title("Long Unicode filename — 音楽 — ".repeat(15).into());
    window.show().unwrap();
    window.window().set_size(slint::LogicalSize::new(
        760.0,
        window.invoke_window_height_for_eq(false),
    ));
    window
        .window()
        .dispatch_event(slint::platform::WindowEvent::ScaleFactorChanged { scale_factor: 2.0 });
    let calls = Rc::new(RefCell::new(0));
    let count = Rc::clone(&calls);
    window.on_play_pause(move || *count.borrow_mut() += 1);
    let play = ElementHandle::find_by_accessible_label(&window, "Play")
        .next()
        .unwrap();
    play.mock_single_click(slint::platform::PointerEventButton::Left);
    send_key(&window, slint::platform::Key::Space);
    assert_eq!(*calls.borrow(), 2);
    let volume = ElementHandle::find_by_accessible_label(&window, "Application volume")
        .next()
        .unwrap();
    let observed = Rc::new(RefCell::new(0.0));
    let changed = Rc::clone(&observed);
    window.on_change_volume(move |value| *changed.borrow_mut() = value);
    volume.mock_single_click(slint::platform::PointerEventButton::Left);
    send_key(&window, slint::platform::Key::RightArrow);
    assert_eq!(*observed.borrow(), 51.0);
    assert!(volume.absolute_position().x + volume.size().width <= 760.0);
    for label in ["Shuffle off", "Repeat off", "Mix off"] {
        let control = ElementHandle::find_by_accessible_label(&window, label)
            .next()
            .unwrap();
        assert!(control.absolute_position().x + control.size().width <= 760.0);
        assert!(
            control.absolute_position().y + control.size().height
                <= window.invoke_window_height_for_eq(false)
        );
    }
    click(&window, "Equalizer");
    send_key(&window, slint::platform::Key::Space);
    assert!(window.get_eq_expanded());
}

/// Dispatches a complete keyboard press/release through Slint's public window API.
pub(super) fn send_key(window: &PlayerWindow, key: slint::platform::Key) {
    let text: slint::SharedString = key.into();
    window
        .window()
        .dispatch_event(slint::platform::WindowEvent::KeyPressed { text: text.clone() });
    window
        .window()
        .dispatch_event(slint::platform::WindowEvent::KeyReleased { text });
}

/// Exercises production row bindings during paused native playback, including empty-queue recovery.
#[test]
#[ignore = "requires a real desktop session"]
fn native_queue_edit_journey() {
    use std::time::{Duration, Instant};
    let directory = std::env::temp_dir().join(format!("resolibre-queue-ui-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let path = write_ui_fixture(&directory);
    let original = std::fs::read(&path).unwrap();
    let window = PlayerWindow::new().unwrap();
    let session = Session::start(AudioOutput::Silent).unwrap();
    let controller = Rc::new(RefCell::new(presentation::Controller::new(session)));
    presentation::connect(&window, &controller);
    controller
        .borrow_mut()
        .submit(&window, Command::Open(vec![path.clone(); 2]));
    let host = playlist_window::connect(&window, Default::default(), Default::default());
    host.borrow_mut().show(&window, &host).unwrap();
    let playlist = host.borrow().test_window();
    let timer = slint::Timer::default();
    let start = Instant::now();
    let weak = window.as_weak();
    let next_path = path.clone();
    let mut step = 0;
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(33),
        move || {
            use slint::Model;
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "queue UI deadline"
            );
            let window = weak.upgrade().unwrap();
            controller.borrow_mut().refresh(&window);
            host.borrow().refresh(&window);
            match step {
                0 if window.get_playing() && window.get_can_seek() => {
                    click(&window, "Pause");
                    step += 1;
                }
                1 if window.get_state_label().starts_with("Paused") => {
                    window.invoke_seek(103.0);
                    step += 1;
                }
                2 if (window.get_position() - 103.0).abs() < 0.5 => {
                    let first = playlist.get_tracks().row_data(0).unwrap().id;
                    let second = playlist.get_tracks().row_data(1).unwrap().id;
                    playlist.invoke_reorder_track(first, second, true);
                    step += 1;
                }
                3 if window.get_current() == 1 => {
                    assert!(!window.get_playing());
                    assert!((window.get_position() - 103.0).abs() < 0.5);
                    playlist.invoke_remove_track(playlist.get_tracks().row_data(0).unwrap().id);
                    step += 1;
                }
                4 if window.get_tracks().row_count() == 1 => {
                    assert_eq!(window.get_current(), 0);
                    assert!(!window.get_playing());
                    assert!((window.get_position() - 103.0).abs() < 0.5);
                    playlist.invoke_remove_track(playlist.get_tracks().row_data(0).unwrap().id);
                    step += 1;
                }
                5 if window.get_tracks().row_count() == 0 => {
                    assert!(!window.get_can_play());
                    assert!(!window.get_can_seek());
                    assert_eq!(window.get_current(), -1);
                    assert_eq!(window.get_track_title(), "Your music. Ready when you are.");
                    controller
                        .borrow_mut()
                        .submit(&window, Command::Open(vec![next_path.clone()]));
                    step += 1;
                }
                6 if window.get_playing() => {
                    assert_eq!(window.get_tracks().row_count(), 1);
                    host.borrow_mut().close();
                    controller.borrow_mut().close();
                    step += 1;
                }
                _ => {}
            }
        },
    );
    window.run().unwrap();
    assert_eq!(std::fs::read(path).unwrap(), original);
    std::fs::remove_dir_all(directory).unwrap();
}

/// Mode buttons dispatch intents; visual state changes only when the session confirms it.
#[test]
fn mode_controls_use_confirmed_state_and_callbacks() {
    i_slint_backend_testing::init_no_event_loop();
    let window = PlayerWindow::new().unwrap();
    let shuffle = Rc::new(RefCell::new(0));
    let count = Rc::clone(&shuffle);
    window.on_toggle_shuffle(move || *count.borrow_mut() += 1);
    let repeat = Rc::new(RefCell::new(0));
    let count = Rc::clone(&repeat);
    window.on_cycle_repeat(move || *count.borrow_mut() += 1);
    window.show().unwrap();
    click(&window, "Shuffle off");
    assert!(!window.get_shuffle());
    window.set_shuffle(true);
    click(&window, "Shuffle on");
    assert_eq!(*shuffle.borrow(), 2);
    click(&window, "Repeat off");
    assert_eq!(window.get_repeat_mode(), RepeatState::Off);
    window.set_repeat_mode(RepeatState::All);
    click(&window, "Repeat all");
    window.set_repeat_mode(RepeatState::One);
    click(&window, "Repeat one");
    assert_eq!(*repeat.borrow(), 3);
    click(&window, "Library");
    click(&window, "Equalizer");
    assert!(window.get_shuffle());
    assert_eq!(window.get_repeat_mode(), RepeatState::One);
}

/// Checks the rendered Back LED inside its button, independent of other yellow indicators.
fn assert_back_light(window: &PlayerWindow, lit: bool) {
    let button = ElementHandle::find_by_accessible_label(window, "Previous track")
        .next()
        .unwrap();
    let position = button.absolute_position();
    let size = button.size();
    let scale = window.window().scale_factor();
    let pixels = window.window().take_snapshot().unwrap();
    let mut yellow_pixels = 0;
    for y in (position.y * scale) as u32..((position.y + size.height) * scale) as u32 {
        for x in (position.x * scale) as u32..((position.x + size.width) * scale) as u32 {
            let offset = ((y * pixels.width() + x) * 4) as usize;
            let rgb = &pixels.as_bytes()[offset..offset + 3];
            if rgb == [0xd9, 0xb9, 0x5a] {
                yellow_pixels += 1;
            }
        }
    }
    assert_eq!(yellow_pixels > 0, lit, "Back LED visibility");
}

/// Exercises real mode bindings, paused history and panel changes against native playback.
#[test]
#[ignore = "requires a real desktop session"]
fn native_modes_journey() {
    use std::time::{Duration, Instant};
    let directory = std::env::temp_dir().join(format!("resolibre-modes-ui-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let path = write_ui_fixture(&directory);
    let original = std::fs::read(&path).unwrap();
    let window = PlayerWindow::new().unwrap();
    let session = Session::start(AudioOutput::Silent).unwrap();
    let controller = Rc::new(RefCell::new(presentation::Controller::new(session)));
    presentation::connect(&window, &controller);
    controller
        .borrow_mut()
        .submit(&window, Command::Open(vec![path.clone(); 3]));
    let timer = slint::Timer::default();
    let start = Instant::now();
    let weak = window.as_weak();
    let mut step = 0;
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(33),
        move || {
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "mode UI deadline"
            );
            let window = weak.upgrade().unwrap();
            controller.borrow_mut().refresh(&window);
            match step {
                0 if window.get_playing() && window.get_can_seek() => {
                    click(&window, "Pause");
                    step += 1;
                }
                1 if window.get_state_label().starts_with("Paused") => {
                    window.invoke_seek(103.0);
                    step += 1;
                }
                2 if (window.get_position() - 103.0).abs() < 0.5 => {
                    click(&window, "Shuffle off");
                    step += 1;
                }
                3 if window.get_shuffle() => {
                    assert!(!window.get_playing());
                    assert!((window.get_position() - 103.0).abs() < 0.5);
                    click(&window, "Repeat off");
                    step += 1;
                }
                4 if window.get_repeat_mode() == RepeatState::All => {
                    click(&window, "Repeat all");
                    step += 1;
                }
                5 if window.get_repeat_mode() == RepeatState::One => {
                    assert!((window.get_position() - 103.0).abs() < 0.5);
                    assert!(!window.get_previous_navigates());
                    assert_back_light(&window, false);
                    click(&window, "Next track");
                    step += 1;
                }
                6 if window.get_current() != 0
                    && window.get_state_label().starts_with("Paused") =>
                {
                    assert!(window.get_previous_navigates());
                    assert_back_light(&window, true);
                    click(&window, "Previous track");
                    step += 1;
                }
                7 if window.get_current() == 0
                    && window.get_state_label().starts_with("Paused") =>
                {
                    click(&window, "Shuffle on");
                    step += 1;
                }
                8 if !window.get_shuffle() => {
                    click(&window, "Repeat one");
                    step += 1;
                }
                9 if window.get_repeat_mode() == RepeatState::Off => {
                    click(&window, "Library");
                    click(&window, "Equalizer");
                    click(&window, "Equalizer");
                    assert!(!window.get_shuffle());
                    assert!(!window.get_playing());
                    controller.borrow_mut().close();
                    step += 1;
                }
                _ => {}
            }
        },
    );
    window.run().unwrap();
    assert_eq!(std::fs::read(path).unwrap(), original);
    std::fs::remove_dir_all(directory).unwrap();
}

/// Vertical shared sliders expose real keyboard/range actions and remain inside the scaled EQ panel.
#[test]
fn equalizer_controls_keep_focus_and_confirmed_gains() {
    i_slint_backend_testing::init_no_event_loop();
    let window = PlayerWindow::new().unwrap();
    crate::equalizer::initialize(&window);
    window.set_eq_expanded(true);
    window.show().unwrap();
    window.window().set_size(slint::LogicalSize::new(
        760.0,
        window.invoke_window_height_for_eq(true),
    ));
    window
        .window()
        .dispatch_event(slint::platform::WindowEvent::ScaleFactorChanged { scale_factor: 2.0 });
    let observed = Rc::new(RefCell::new((0, 0.0)));
    let target = Rc::clone(&observed);
    window.on_eq_band(move |index, gain| *target.borrow_mut() = (index, gain));
    let band = ElementHandle::find_by_accessible_label(&window, "EQ 1k")
        .next()
        .unwrap();
    band.mock_single_click(slint::platform::PointerEventButton::Left);
    send_key(&window, slint::platform::Key::UpArrow);
    assert_eq!(*observed.borrow(), (5, 0.5));
    assert!(
        band.absolute_position().y + band.size().height <= window.invoke_window_height_for_eq(true)
    );
    let mut snapshot = resolibre_audio::session::Snapshot::default();
    snapshot
        .equalizer
        .apply(resolibre_core::equalizer::EqAction::Band { index: 5, db: 4.0 })
        .unwrap();
    crate::equalizer::show(&window, &snapshot);
    send_key(&window, slint::platform::Key::UpArrow);
    assert_eq!(*observed.borrow(), (5, 4.5));
    snapshot.eq_sample_rate = 8000;
    crate::equalizer::show(&window, &snapshot);
    assert!(!window.get_eq_bands().row_data(9).unwrap().available);
    let preamp = ElementHandle::find_by_accessible_label(&window, "EQ preamp")
        .next()
        .unwrap();
    assert!(preamp.absolute_position().x + preamp.size().width <= 760.0);
}

/// Runs EQ edits, explicit Save/Load and exclusive panels through the native session and private storage.
#[test]
#[ignore = "requires a real desktop session"]
fn native_equalizer_journey() {
    use std::time::{Duration, Instant};
    let directory = std::env::temp_dir().join(format!("resolibre-eq-ui-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let path = write_ui_fixture(&directory);
    let original = std::fs::read(&path).unwrap();
    let window = PlayerWindow::new().unwrap();
    let session = Session::start(AudioOutput::Silent).unwrap();
    let controller = Rc::new(RefCell::new(presentation::Controller::new(session)));
    controller
        .borrow_mut()
        .start_presets(directory.join("settings"));
    presentation::connect(&window, &controller);
    controller
        .borrow_mut()
        .submit(&window, Command::Open(vec![path.clone()]));
    let timer = slint::Timer::default();
    let start = Instant::now();
    let weak = window.as_weak();
    let mut step = 0;
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(50),
        move || {
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "EQ native journey deadline"
            );
            let window = weak.upgrade().unwrap();
            controller.borrow_mut().refresh(&window);
            match step {
                0 if window.get_playing() => {
                    click(&window, "Pause");
                    step += 1;
                }
                1 if !window.get_playing() => {
                    click(&window, "Enable EQ");
                    step += 1;
                }
                2 if window.get_eq_enabled() => {
                    ElementHandle::find_by_accessible_label(&window, "EQ 1k")
                        .next()
                        .unwrap()
                        .set_accessible_value("6");
                    ElementHandle::find_by_accessible_label(&window, "EQ preamp")
                        .next()
                        .unwrap()
                        .set_accessible_value("-1.5");
                    step += 1;
                }
                3 if window.get_eq_preamp() == -1.5 && !window.get_eq_store_busy() => {
                    click(&window, "Save EQ preset");
                    assert!(window.get_eq_dialog_open());
                    window.set_eq_preset_name("Night headphones".into());
                    click(&window, "Confirm preset save");
                    step += 1;
                }
                4 if !window.get_eq_store_busy() => {
                    // The saved curve comes first; Flat follows it in the selector.
                    window.invoke_eq_load(1);
                    step += 1;
                }
                5 if window.get_eq_preamp() == 0.0 => {
                    window.invoke_eq_load(0);
                    step += 1;
                }
                6 if window.get_eq_preamp() == -1.5 => {
                    assert_eq!(window.get_eq_bands().row_data(5).unwrap().gain, 6.0);
                    click(&window, "Equalizer");
                    click(&window, "Equalizer");
                    assert!(!window.get_playing());
                    click(&window, "Bypass EQ");
                    step += 1;
                }
                7 if !window.get_eq_enabled() => {
                    assert_eq!(window.get_eq_preamp(), -1.5);
                    controller.borrow_mut().close();
                    step += 1;
                }
                _ => {}
            }
        },
    );
    window.run().unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert!(
        directory
            .join("settings/eq-presets/preset-1.resolibre-eq")
            .is_file()
    );
    std::fs::remove_dir_all(directory).unwrap();
}

/// Locates a slider and returns a pointer position inside its usable track.
pub(super) fn slider_point(
    window: &PlayerWindow,
    label: &str,
    fraction: f32,
) -> slint::LogicalPosition {
    let element = ElementHandle::find_by_accessible_label(window, label)
        .next()
        .expect("slider exists");
    let origin = element.absolute_position();
    let size = element.size();
    slint::LogicalPosition::new(
        origin.x + size.width / 2.0,
        origin.y + 8.0 + fraction * (size.height - 16.0),
    )
}

/// Delivers native-style pointer input, including capture outside the starting control.
pub(super) fn pointer(window: &impl ComponentHandle, position: slint::LogicalPosition, kind: &str) {
    use slint::platform::{PointerEventButton, WindowEvent};
    let button = PointerEventButton::Left;
    let event = match kind {
        "down" => WindowEvent::PointerPressed { position, button },
        "up" => WindowEvent::PointerReleased { position, button },
        "move" => WindowEvent::PointerMoved { position },
        _ => panic!("unknown pointer event"),
    };
    window.window().dispatch_event(event);
}

/// Pointer endpoints and midpoint map to the core preamp range; accessibility cannot exceed it.
#[test]
fn preamp_uses_symmetric_core_limits() {
    use resolibre_core::equalizer::{PREAMP_MAX_DB, PREAMP_MIN_DB};
    i_slint_backend_testing::init_no_event_loop();
    let window = PlayerWindow::new().unwrap();
    crate::equalizer::initialize(&window);
    window.set_eq_expanded(true);
    window.show().unwrap();
    window
        .window()
        .set_size(slint::LogicalSize::new(800.0, 516.0));
    let observed = Rc::new(RefCell::new(99.0));
    let target = Rc::clone(&observed);
    window.on_eq_gain(move |gain| *target.borrow_mut() = gain);
    for (fraction, expected) in [(0.0, PREAMP_MAX_DB), (0.5, 0.0), (1.0, PREAMP_MIN_DB)] {
        i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_secs(1));
        let point = slider_point(&window, "EQ preamp", fraction);
        pointer(&window, point, "move");
        pointer(&window, point, "down");
        pointer(&window, point, "up");
        assert_eq!(*observed.borrow(), expected as f32);
    }
    let preamp = ElementHandle::find_by_accessible_label(&window, "EQ preamp")
        .next()
        .unwrap();
    for (input, expected) in [("-20", PREAMP_MIN_DB), ("20", PREAMP_MAX_DB)] {
        preamp.set_accessible_value(input);
        assert_eq!(*observed.borrow(), expected as f32);
    }
}

/// Double clicks use the toolkit recognizer and shared callbacks, including disabled guards.
#[test]
fn sliders_double_click_to_defaults() {
    use std::time::Duration;
    i_slint_backend_testing::init_no_event_loop();
    let window = PlayerWindow::new().unwrap();
    crate::equalizer::initialize(&window);
    window.set_eq_expanded(true);
    window.set_can_seek(true);
    window.set_duration(300.0);
    window.show().unwrap();
    window.window().set_size(slint::LogicalSize::new(
        860.0,
        window.invoke_window_height_for_eq(true),
    ));
    let calls = Rc::new(RefCell::new(Vec::new()));
    let target = Rc::clone(&calls);
    window.on_change_volume(move |gain| target.borrow_mut().push(("volume", gain)));
    let target = Rc::clone(&calls);
    window.on_seek(move |position| target.borrow_mut().push(("seek", position)));
    let target = Rc::clone(&calls);
    window.on_eq_gain(move |gain| target.borrow_mut().push(("preamp", gain)));
    let target = Rc::clone(&calls);
    window.on_eq_band(move |_, gain| target.borrow_mut().push(("band", gain)));
    let labels = [
        ("Application volume", "volume", 50.0),
        ("Playback position", "seek", 0.0),
        ("EQ preamp", "preamp", 0.0),
        ("EQ 31.25", "band", 0.0),
        ("EQ 1k", "band", 0.0),
        ("EQ 16k", "band", 0.0),
    ];
    for (label, action, expected) in labels {
        i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
        let point = slider_point(&window, label, 0.2);
        pointer(&window, point, "move");
        for _ in 0..2 {
            pointer(&window, point, "down");
            i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(50));
            pointer(&window, point, "up");
        }
        assert_eq!(calls.borrow().last(), Some(&(action, expected)), "{label}");
        // Release has ended capture: a subsequent hover must not overwrite the reset.
        let count = calls.borrow().len();
        pointer(&window, slider_point(&window, label, 0.8), "move");
        assert_eq!(calls.borrow().len(), count);
    }
    window.set_closing(true);
    calls.borrow_mut().clear();
    for label in [
        "Application volume",
        "Playback position",
        "EQ preamp",
        "EQ 1k",
    ] {
        let point = slider_point(&window, label, 0.2);
        for _ in 0..2 {
            pointer(&window, point, "down");
            pointer(&window, point, "up");
        }
    }
    assert!(calls.borrow().is_empty());
}

/// A captured drag edits only the hovered available band, across resizing and refreshed models.
#[test]
fn equalizer_drag_follows_pointer_and_ends_cleanly() {
    i_slint_backend_testing::init_no_event_loop();
    let window = PlayerWindow::new().unwrap();
    crate::equalizer::initialize(&window);
    window.set_eq_expanded(true);
    window.show().unwrap();
    let calls = Rc::new(RefCell::new(Vec::new()));
    let target = Rc::clone(&calls);
    window.on_eq_band(move |index, gain| target.borrow_mut().push((index, gain)));
    window.on_eq_gain(|_| panic!("band sweep must not edit preamp"));
    let labels = [
        "EQ 31.25", "EQ 62.5", "EQ 125", "EQ 250", "EQ 500", "EQ 1k", "EQ 2k", "EQ 4k", "EQ 8k",
        "EQ 16k",
    ];
    for width in [760.0, 1060.0] {
        window.window().set_size(slint::LogicalSize::new(
            width,
            window.invoke_window_height_for_eq(true),
        ));
        let start = slider_point(&window, labels[0], 0.25);
        pointer(&window, start, "move");
        pointer(&window, start, "down");
        for (index, label) in labels.iter().enumerate() {
            let point = slider_point(&window, label, 0.25);
            pointer(&window, point, "move");
            let (actual_index, gain) = *calls.borrow().last().unwrap();
            assert_eq!(actual_index, index as i32);
            assert!((gain - 6.0).abs() < 0.01);
        }
        // Moving back edits the new band, not the band that owns pointer capture.
        pointer(&window, slider_point(&window, labels[3], 0.75), "move");
        assert_eq!(*calls.borrow().last().unwrap(), (3, -6.0));
        let count = calls.borrow().len();
        let neighbor = slider_point(&window, labels[1], 0.25);
        let gap = slint::LogicalPosition::new((start.x + neighbor.x) / 2.0, start.y);
        pointer(&window, gap, "move");
        pointer(&window, slint::LogicalPosition::new(-10.0, start.y), "move");
        pointer(&window, slider_point(&window, "EQ preamp", 0.25), "move");
        pointer(&window, slint::LogicalPosition::new(start.x, 0.0), "move");
        assert_eq!(calls.borrow().len(), count);
        // Source-rate refresh must not destroy capture or activate an unavailable band.
        let snapshot = resolibre_audio::session::Snapshot {
            eq_sample_rate: 8000,
            ..Default::default()
        };
        crate::equalizer::show(&window, &snapshot);
        pointer(&window, slider_point(&window, labels[9], 0.25), "move");
        assert_eq!(calls.borrow().len(), count);
        let finish = slider_point(&window, labels[5], 0.75);
        pointer(&window, finish, "move");
        assert_eq!(*calls.borrow().last().unwrap(), (5, -6.0));
        pointer(&window, finish, "up");
        let count = calls.borrow().len();
        pointer(&window, start, "move");
        assert_eq!(calls.borrow().len(), count);
        // Disabling during capture must cancel without applying another edit.
        pointer(&window, start, "down");
        window.set_closing(true);
        let count = calls.borrow().len();
        pointer(&window, finish, "move");
        pointer(&window, finish, "up");
        assert_eq!(calls.borrow().len(), count);
        window.set_closing(false);
        crate::equalizer::initialize(&window);
        // Folding destroys the captured view; reopening must not inherit the drag.
        pointer(&window, start, "down");
        window.set_eq_expanded(false);
        pointer(&window, finish, "up");
        window.set_eq_expanded(true);
        let count = calls.borrow().len();
        pointer(&window, finish, "move");
        assert_eq!(calls.borrow().len(), count);
        // Only the left button starts adjustment.
        window
            .window()
            .dispatch_event(slint::platform::WindowEvent::PointerPressed {
                position: start,
                button: slint::platform::PointerEventButton::Right,
            });
        pointer(&window, finish, "move");
        window
            .window()
            .dispatch_event(slint::platform::WindowEvent::PointerReleased {
                position: finish,
                button: slint::platform::PointerEventButton::Right,
            });
        assert_eq!(calls.borrow().len(), count);
    }
}

/// Refreshes production bindings until a UI condition, rejecting failures immediately.
fn wait_for_ui(
    window: &PlayerWindow,
    controller: &Rc<RefCell<presentation::Controller>>,
    condition: impl Fn(&PlayerWindow) -> bool,
) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        controller.borrow_mut().refresh(window);
        assert!(
            !window.get_state_label().starts_with("Playback failed"),
            "{}",
            window.get_error_message()
        );
        if condition(window) {
            return;
        }
        assert!(std::time::Instant::now() < deadline, "UI playback deadline");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// Holds the real scrubber across EOF in a three-track session, then seeks the next track afresh.
#[test]
fn scrub_gesture_does_not_seek_the_next_track() {
    use std::time::Duration;
    i_slint_backend_testing::init_no_event_loop();
    let directory = std::env::temp_dir().join(format!("resolibre-scrub-ui-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let path = write_ui_fixture(&directory);
    let original = std::fs::read(&path).unwrap();
    let window = PlayerWindow::new().unwrap();
    let session = Session::start(AudioOutput::Silent).unwrap();
    let controller = Rc::new(RefCell::new(presentation::Controller::new(session)));
    presentation::connect(&window, &controller);
    window.show().unwrap();
    let paths = match (
        std::env::var_os("RESOUL_TEST_MP3"),
        std::env::var_os("RESOUL_TEST_FLAC"),
    ) {
        (Some(mp3), Some(flac)) => vec![mp3.clone().into(), flac.into(), mp3.into()],
        (None, None) => vec![path.clone(); 3],
        _ => panic!("supply both optional codec fixtures"),
    };
    controller
        .borrow_mut()
        .submit(&window, Command::Open(paths));
    wait_for_ui(&window, &controller, |w| {
        w.get_playing() && w.get_can_seek()
    });
    let slider = ElementHandle::find_by_accessible_label(&window, "Playback position")
        .next()
        .unwrap();
    let origin = slider.absolute_position();
    let size = slider.size();
    let end =
        slint::LogicalPosition::new(origin.x + size.width - 8.0, origin.y + size.height / 2.0);
    pointer(&window, end, "move");
    pointer(&window, end, "down");
    // Let native playback advance between UI frames while the pointer remains captured.
    std::thread::sleep(Duration::from_millis(150));
    wait_for_ui(&window, &controller, |w| {
        w.get_current() == 1 && w.get_playing() && w.get_can_seek()
    });
    for offset in 0..30 {
        let point = slint::LogicalPosition::new(end.x + 1.0 + (offset % 2) as f32, end.y);
        pointer(&window, point, "move");
        controller.borrow_mut().refresh(&window);
        assert_eq!(
            window.get_current(),
            1,
            "held scrub must not skip another track"
        );
        assert!(!window.get_state_label().starts_with("Playback failed"));
        std::thread::sleep(Duration::from_millis(5));
    }
    pointer(&window, end, "up");
    assert!(window.get_position() < 2.0);
    i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
    let target = window.get_duration() / 2.0;
    let origin = slider.absolute_position();
    let size = slider.size();
    let middle = slint::LogicalPosition::new(origin.x + size.width / 2.0, end.y);
    pointer(&window, middle, "move");
    pointer(&window, middle, "down");
    pointer(&window, middle, "up");
    wait_for_ui(&window, &controller, |w| {
        (w.get_position() - target).abs() < 1.0
    });
    controller.borrow_mut().close();
    let shutdown_deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !controller.borrow().snapshot().finished {
        controller.borrow_mut().refresh(&window);
        assert!(std::time::Instant::now() < shutdown_deadline);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(window.get_current(), 1);
    drop(controller);
    assert_eq!(std::fs::read(&path).unwrap(), original);
    std::fs::remove_dir_all(directory).unwrap();
}

/// Real pointer holds scan once; release, focus loss and disabling never navigate afterward.
#[test]
fn transport_pointer_hold_is_distinct_from_click() {
    use slint::platform::WindowEvent;
    use std::time::Duration;
    i_slint_backend_testing::init_no_event_loop();
    let window = PlayerWindow::new().unwrap();
    window.set_can_next(true);
    window.set_can_previous(true);
    window.set_can_seek(true);
    window.show().unwrap();
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    let events = Rc::new(RefCell::new(Vec::new()));
    let log = Rc::clone(&events);
    window.on_next(move || log.borrow_mut().push("next"));
    let log = Rc::clone(&events);
    window.on_previous(move || log.borrow_mut().push("previous"));
    let log = Rc::clone(&events);
    window.on_scan_started(move |forward| {
        log.borrow_mut()
            .push(if forward { "forward" } else { "backward" })
    });
    let log = Rc::clone(&events);
    window.on_scan_ended(move || log.borrow_mut().push("release"));
    for (label, click_event, hold_event) in [
        ("Next track", "next", "forward"),
        ("Previous track", "previous", "backward"),
    ] {
        let button = ElementHandle::find_by_accessible_label(&window, label)
            .next()
            .unwrap();
        let origin = button.absolute_position();
        let point = slint::LogicalPosition::new(
            origin.x + button.size().width / 2.0,
            origin.y + button.size().height / 2.0,
        );
        pointer(&window, point, "move");
        pointer(&window, point, "down");
        i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(100));
        pointer(&window, point, "up");
        assert_eq!(events.borrow().as_slice(), &[click_event]);
        events.borrow_mut().clear();
        pointer(&window, point, "down");
        i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(400));
        assert_eq!(events.borrow().as_slice(), &[hold_event]);
        i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
        assert_eq!(events.borrow().as_slice(), &[hold_event]);
        let outside = slint::LogicalPosition::new(-20.0, -20.0);
        pointer(&window, outside, "move");
        pointer(&window, outside, "up");
        assert_eq!(events.borrow().as_slice(), &[hold_event, "release"]);
        events.borrow_mut().clear();
        pointer(&window, point, "move");
        pointer(&window, point, "down");
        i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(400));
        window
            .window()
            .dispatch_event(WindowEvent::WindowActiveChanged(false));
        i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(1));
        assert_eq!(events.borrow().as_slice(), &[hold_event, "release"]);
        pointer(&window, point, "up");
        assert_eq!(events.borrow().len(), 2);
        window
            .window()
            .dispatch_event(WindowEvent::WindowActiveChanged(true));
        events.borrow_mut().clear();
    }
    window.set_can_next(false);
    window.set_can_previous(false);
    assert!(
        ElementHandle::find_by_accessible_label(&window, "Next track")
            .next()
            .unwrap()
            .accessible_enabled()
            .unwrap()
    );
    click(&window, "Next track");
    assert!(events.borrow().is_empty());
}

/// Production hold bindings capture the pressed track and release without reborrowing refresh.
#[test]
fn scan_gesture_uses_pressed_generation_and_cancels_on_disable() {
    use std::time::Duration;
    i_slint_backend_testing::init_no_event_loop();
    let directory = std::env::temp_dir().join(format!("resolibre-scan-ui-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let path = write_ui_fixture(&directory);
    let original = std::fs::read(&path).unwrap();
    let window = PlayerWindow::new().unwrap();
    let controller = Rc::new(RefCell::new(presentation::Controller::new(
        Session::start(AudioOutput::Silent).unwrap(),
    )));
    presentation::connect(&window, &controller);
    window.show().unwrap();
    controller
        .borrow_mut()
        .submit(&window, Command::Open(vec![path.clone(); 2]));
    wait_for_ui(&window, &controller, |w| {
        w.get_playing() && w.get_can_seek()
    });
    let button = ElementHandle::find_by_accessible_label(&window, "Next track")
        .next()
        .unwrap();
    let origin = button.absolute_position();
    let point = slint::LogicalPosition::new(
        origin.x + button.size().width / 2.0,
        origin.y + button.size().height / 2.0,
    );
    pointer(&window, point, "move");
    pointer(&window, point, "down");
    controller.borrow_mut().submit(&window, Command::Next);
    wait_for_ui(&window, &controller, |w| {
        w.get_current() == 1 && w.get_can_seek()
    });
    i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(400));
    std::thread::sleep(Duration::from_millis(50));
    controller.borrow_mut().refresh(&window);
    assert!(!window.get_state_label().starts_with("Scanning"));
    pointer(&window, point, "up");
    i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(1));
    pointer(&window, point, "down");
    i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(400));
    wait_for_ui(&window, &controller, |w| {
        w.get_state_label().starts_with("Scanning forward")
    });
    // Simulate a capability update while Controller is borrowed, as happens during refresh.
    {
        let _borrow = controller.borrow_mut();
        window.set_can_seek(false);
        i_slint_backend_testing::mock_elapsed_time(Duration::ZERO);
    }
    i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(1));
    wait_for_ui(&window, &controller, |w| {
        w.get_state_label().starts_with("Playing")
    });
    pointer(&window, point, "up");
    controller.borrow_mut().close();
    let shutdown_deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !controller.borrow().snapshot().finished {
        controller.borrow_mut().refresh(&window);
        assert!(std::time::Instant::now() < shutdown_deadline);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(window.get_current(), 1);
    drop(controller);
    window.hide().unwrap();
    drop(window);
    assert_eq!(std::fs::read(&path).unwrap(), original);
    std::fs::remove_dir_all(directory).unwrap();
}
