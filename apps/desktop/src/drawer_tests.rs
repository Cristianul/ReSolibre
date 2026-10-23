//! The drawer owns visual progress only; toggling must preserve audio controls and native geometry.
use crate::{PlayerWindow, equalizer, presentation};
use i_slint_backend_testing::{ElementHandle, mock_elapsed_time};
use slint::ComponentHandle;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

/// Returns the labelled control's origin for ordering and stationary-player assertions.
fn control_position(window: &PlayerWindow, label: &str) -> slint::LogicalPosition {
    ElementHandle::find_by_accessible_label(window, label)
        .next()
        .unwrap()
        .absolute_position()
}

/// Closing/reversing uses continuous progress; disabled drawer gestures cannot change DSP.
#[test]
fn drawer_progress_reverses_and_respects_reduced_motion() {
    i_slint_backend_testing::init_no_event_loop();
    let window = PlayerWindow::new().unwrap();
    equalizer::initialize(&window);
    window.set_eq_enabled(true);
    window.set_position(103.0);
    window.set_eq_preamp(-1.0);
    window.on_eq_gain(|_| panic!("visibility must not change audio"));
    window.show().unwrap();
    assert_eq!(window.get_eq_reveal(), 1.0, "startup is already expanded");
    assert!(control_position(&window, "Equalizer").x < control_position(&window, "Library").x);
    let play = control_position(&window, "Play");
    window.invoke_toggle_equalizer();
    assert_eq!(window.get_eq_reveal(), 1.0);
    mock_elapsed_time(Duration::from_millis(120));
    let closing = window.get_eq_reveal();
    assert!(closing > 0.0 && closing < 1.0);
    assert_eq!(control_position(&window, "Play"), play);
    let preamp = ElementHandle::find_by_accessible_label(&window, "EQ preamp")
        .next()
        .unwrap();
    assert_eq!(preamp.accessible_enabled(), Some(false));
    window.invoke_toggle_equalizer();
    assert!(
        (window.get_eq_reveal() - closing).abs() < 0.001,
        "reversing must not jump"
    );
    mock_elapsed_time(Duration::from_millis(320));
    assert_eq!(window.get_eq_reveal(), 1.0);
    window.invoke_toggle_equalizer();
    let _ = window.get_eq_reveal();
    mock_elapsed_time(Duration::from_millis(320));
    assert_eq!(window.get_eq_reveal(), 0.0);
    window.set_reduced_motion(true);
    window.invoke_toggle_equalizer();
    assert_eq!(window.get_eq_reveal(), 1.0);
    window.invoke_toggle_equalizer();
    assert_eq!(window.get_eq_reveal(), 0.0);
    assert!(window.get_eq_enabled());
    assert_eq!(window.get_eq_preamp(), -1.0);
    assert_eq!(window.get_position(), 103.0);
}

/// Captures native animation frames at the actual scale, using only the harness-owned display.
fn capture(window: &PlayerWindow, phase: &str) {
    let pixels = window.window().take_snapshot().unwrap();
    let scale = window.window().scale_factor();
    if phase == "open" {
        let x = (20.0 * scale) as usize;
        let y = (264.0 * scale) as usize;
        let alpha = pixels.as_bytes()[(y * pixels.width() as usize + x) * 4 + 3];
        assert_eq!(
            alpha, 255,
            "drawer must meet the main case without an air gap"
        );
    }
    let directory = std::path::PathBuf::from(std::env::var_os("RESOUL_PREVIEW_DIR").unwrap());
    image::save_buffer(
        directory.join(format!("drawer-{phase}-{scale}x.png")),
        pixels.as_bytes(),
        pixels.width(),
        pixels.height(),
        image::ColorType::Rgba8,
    )
    .unwrap();
}

/// Native sizing keeps the closing slide visible, restores full size and cancels stale collapses.
#[test]
#[ignore = "requires isolated KWin/XWayland; run through check_window_snap.py"]
fn native_drawer_animation() {
    crate::window_snap::select_x11().unwrap();
    let window = PlayerWindow::new().unwrap();
    equalizer::initialize(&window);
    let session = resolibre_audio::session::Session::start(resolibre_audio::AudioOutput::Silent).unwrap();
    let controller = Rc::new(RefCell::new(presentation::Controller::new(session)));
    presentation::connect(&window, &controller);
    window.set_reduced_motion(false);
    window.set_eq_enabled(true);
    let weak = window.as_weak();
    let timer = slint::Timer::default();
    let mut step = 0;
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(150),
        move || {
            let window = weak.upgrade().unwrap();
            let height = window
                .window()
                .size()
                .to_logical(window.window().scale_factor())
                .height;
            match step {
                0 => capture(&window, "open"),
                1 => {
                    window.invoke_toggle_equalizer();
                }
                2 => {
                    assert!(window.get_eq_reveal() > 0.0 && window.get_eq_reveal() < 1.0);
                    assert_eq!(height, window.invoke_window_height_for_eq(true));
                    capture(&window, "closing");
                }
                4 => {
                    assert_eq!(height, window.invoke_window_height_for_eq(false));
                    assert_eq!(window.get_eq_reveal(), 0.0);
                    window.invoke_toggle_equalizer();
                }
                5 => {
                    assert!(window.get_eq_reveal() > 0.0 && window.get_eq_reveal() < 1.0);
                    capture(&window, "opening");
                }
                7 => {
                    assert_eq!(height, window.invoke_window_height_for_eq(true));
                    assert_eq!(window.get_eq_reveal(), 1.0);
                    window.invoke_toggle_equalizer();
                }
                8 => {
                    let progress = window.get_eq_reveal();
                    assert!(progress > 0.0 && progress < 1.0);
                    window.invoke_toggle_equalizer();
                    assert!((window.get_eq_reveal() - progress).abs() < 0.001);
                }
                11 => {
                    assert_eq!(height, window.invoke_window_height_for_eq(true));
                    assert_eq!(window.get_eq_reveal(), 1.0);
                    assert!(window.get_eq_enabled());
                    slint::quit_event_loop().unwrap();
                }
                _ => {}
            }
            step += 1;
        },
    );
    window.run().unwrap();
}
