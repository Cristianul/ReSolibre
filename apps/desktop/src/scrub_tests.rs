//! Native scrub checks observe both confirmed playback and actual renderer progress.

use crate::{PlayerWindow, playlist_window, presentation, ui_tests};
use i_slint_backend_testing::ElementHandle;
use resolibre_audio::AudioOutput;
use resolibre_audio::session::{Command, MixStatus, Session, Snapshot};
use resolibre_core::queue::QueueAction;
use slint::ComponentHandle;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

/// Rapid captured pointer seeks must leave the native clock and rendered frames advancing.
#[test]
#[ignore = "requires a real desktop session"]
fn native_repeated_scrub_keeps_clock_and_renderer_running() {
    let directory =
        std::env::temp_dir().join(format!("resolibre-scrub-render-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let first = write_rhythmic_fixture(&directory.join("first"), 120.0);
    let second = write_rhythmic_fixture(&directory.join("second"), 124.0);
    let originals = [first.clone(), second.clone()].map(|path| {
        let bytes = std::fs::read(&path).unwrap();
        (path, bytes)
    });
    let window = PlayerWindow::new().unwrap();
    let session = Session::start(AudioOutput::Silent).unwrap();
    let controller = Rc::new(RefCell::new(presentation::Controller::new(session)));
    presentation::connect(&window, &controller);
    controller
        .borrow_mut()
        .submit(&window, Command::SetMix(true));
    controller
        .borrow_mut()
        .submit(&window, Command::Queue(QueueAction::CycleRepeat));
    controller
        .borrow_mut()
        .submit(&window, Command::Open(vec![first, second]));
    let host = playlist_window::connect(&window, Default::default(), Default::default());
    host.borrow_mut().show(&window, &host).unwrap();
    let rendered = Rc::new(Cell::new(0_u64));
    let frame_count = rendered.clone();
    window
        .window()
        .set_rendering_notifier(move |state, _| {
            if matches!(state, slint::RenderingState::AfterRendering) {
                frame_count.set(frame_count.get() + 1);
            }
        })
        .unwrap();

    // A stalled toolkit timer cannot enforce its own deadline. The watchdog wakes the
    // event loop from another thread and does not touch widgets or native audio.
    let complete = Rc::new(Cell::new(false));
    let (cancel, deadline) = std::sync::mpsc::channel();
    let watchdog = std::thread::spawn(move || {
        if deadline.recv_timeout(Duration::from_secs(30)).is_err() {
            let _ = slint::quit_event_loop();
        }
    });

    let timer = slint::Timer::default();
    let weak = window.as_weak();
    let finished = complete.clone();
    let mut progress = ScrubProgress::new();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(33),
        move || {
            let window = weak.upgrade().unwrap();
            controller.borrow_mut().refresh(&window);
            host.borrow().refresh(&window);
            if finished.get() {
                return;
            }
            let snapshot = controller.borrow().snapshot();
            if !progress.tick(&window, &snapshot, rendered.get()) {
                return;
            }
            finished.set(true);
            host.borrow_mut().close();
            controller.borrow_mut().close();
        },
    );
    window.run().unwrap();
    timer.stop();
    let _ = cancel.send(());
    watchdog.join().unwrap();
    assert!(complete.get(), "UI refresh timer stopped");
    for (path, original) in originals {
        assert_eq!(std::fs::read(path).unwrap(), original);
    }
    std::fs::remove_dir_all(directory).unwrap();
}

struct ScrubProgress {
    started: Instant,
    primed: bool,
    gesture: u32,
    released: Option<(Instant, f32, slint::SharedString, u64)>,
    observed_overlap: bool,
    applied_seeks: u32,
    previous_position: f32,
    last_frame_check: (Instant, u64),
}

impl ScrubProgress {
    /// Starts bounded native input, observation and post-release verification state.
    fn new() -> Self {
        Self {
            started: Instant::now(),
            primed: false,
            gesture: 0,
            released: None,
            observed_overlap: false,
            applied_seeks: 0,
            previous_position: 0.0,
            last_frame_check: (Instant::now(), 0),
        }
    }

    /// Advances one real event-loop turn; completion requires confirmed playback and renderer motion.
    fn tick(&mut self, window: &PlayerWindow, snapshot: &Snapshot, rendered: u64) -> bool {
        assert!(!window.get_state_label().starts_with("Playback failed"));
        assert!(
            self.started.elapsed() < Duration::from_secs(28),
            "native scrub deadline"
        );
        if !window.get_can_seek() || !window.get_playing() {
            return false;
        }
        self.observed_overlap |= matches!(
            snapshot.mix_status,
            MixStatus::BeatMixing | MixStatus::TempoMixing | MixStatus::Crossfading
        );
        if (window.get_position() - self.previous_position).abs() > 1.0 {
            self.applied_seeks += 1;
        }
        self.previous_position = window.get_position();
        if !self.primed {
            if snapshot.mix_status == MixStatus::Ready {
                window.invoke_seek(10.0);
                self.primed = true;
            }
            return false;
        }
        if !self.observed_overlap {
            return false;
        }
        if self.last_frame_check.0.elapsed() >= Duration::from_secs(1) {
            assert!(
                rendered > self.last_frame_check.1,
                "native frames stopped during seek"
            );
            self.last_frame_check = (Instant::now(), rendered);
        }
        if self.gesture < 450 {
            self.move_pointer(window);
            return false;
        }
        if self.gesture == 450 {
            window.invoke_seek(2.0);
            self.gesture += 1;
            return false;
        }
        self.playback_resumed(window, rendered)
    }

    /// Performs fifty captures/releases while alternating across both sides of the MIX window.
    fn move_pointer(&mut self, window: &PlayerWindow) {
        let slider = ElementHandle::find_by_accessible_label(window, "Playback position")
            .next()
            .unwrap();
        let origin = slider.absolute_position();
        let size = slider.size();
        let fraction = 0.1 + (self.gesture * 37 % 80) as f32 / 100.0;
        let point = slint::LogicalPosition::new(
            origin.x + 8.0 + fraction * (size.width - 16.0),
            origin.y + size.height / 2.0,
        );
        send_pointer(window, point, "move");
        if self.gesture.is_multiple_of(9) {
            send_pointer(window, point, "down");
        }
        if self.gesture % 9 == 8 {
            send_pointer(window, point, "up");
        }
        self.gesture += 1;
    }

    /// Waits for the final seek to settle, then checks two seconds of independent playback motion.
    fn playback_resumed(&mut self, window: &PlayerWindow, rendered: u64) -> bool {
        let Some((release_time, initial_position, initial_label, initial_frames)) = &self.released
        else {
            if (1.9..2.5).contains(&window.get_position()) {
                self.released = Some((
                    Instant::now(),
                    window.get_position(),
                    window.get_time_label(),
                    rendered,
                ));
            }
            return false;
        };
        if release_time.elapsed() < Duration::from_secs(2) {
            return false;
        }
        assert!(
            window.get_position() > initial_position + 0.5,
            "confirmed playback clock stopped"
        );
        assert_ne!(
            window.get_time_label(),
            *initial_label,
            "visible clock label stopped"
        );
        assert!(
            rendered > *initial_frames + 5,
            "rendering stopped after scrub"
        );
        assert!(
            self.applied_seeks >= 10,
            "native pointer seeks were not applied"
        );
        assert!(self.observed_overlap, "MIX overlap was not exercised");
        true
    }
}

/// Builds owned sixteen-second impulse trains whose measured grids activate native MIX.
fn write_rhythmic_fixture(directory: &std::path::Path, bpm: f64) -> std::path::PathBuf {
    std::fs::create_dir(directory).unwrap();
    let path = ui_tests::write_ui_fixture(directory);
    let mut bytes = std::fs::read(&path).unwrap();
    let data_length = 16_u32 * 8000 * 2;
    bytes.truncate(44 + data_length as usize);
    bytes[4..8].copy_from_slice(&(36 + data_length).to_le_bytes());
    bytes[40..44].copy_from_slice(&data_length.to_le_bytes());
    for frame in 0..16 * 8000 {
        let time = frame as f64 / 8000.0;
        let phase = (time - 1.0).rem_euclid(60.0 / bpm);
        let sample = if (1.0..15.0).contains(&time) && phase < 0.035 {
            ((std::f64::consts::TAU * 900.0 * phase).sin() * (-phase * 110.0).exp() * 12000.0)
                as i16
        } else {
            0
        };
        bytes[44 + frame * 2..46 + frame * 2].copy_from_slice(&sample.to_le_bytes());
    }
    std::fs::write(&path, bytes).unwrap();
    path
}

/// Sends input only to the test's native HWND, exercising winit capture and event ordering.
#[cfg(windows)]
fn send_pointer(window: &PlayerWindow, point: slint::LogicalPosition, action: &str) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use slint::winit_030::WinitWindowAccessor;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        PostMessageW, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
    };

    let message = match action {
        "move" => WM_MOUSEMOVE,
        "down" => WM_LBUTTONDOWN,
        "up" => WM_LBUTTONUP,
        _ => panic!("unknown pointer action"),
    };
    let point = point.to_physical(window.window().scale_factor());
    let coordinates = ((point.x as u32 & 0xffff) | ((point.y as u32 & 0xffff) << 16)) as isize;
    window
        .window()
        .with_winit_window(|native| {
            let RawWindowHandle::Win32(handle) = native.window_handle().unwrap().as_raw() else {
                panic!("native Windows test requires an HWND");
            };
            // SAFETY: this live HWND belongs to the test, and the message contains values only.
            let posted = unsafe { PostMessageW(handle.hwnd.get() as _, message, 0, coordinates) };
            assert_ne!(posted, 0);
        })
        .expect("test window is mapped");
}

/// Other desktops retain the shared toolkit pointer path without global input injection.
#[cfg(not(windows))]
fn send_pointer(window: &PlayerWindow, point: slint::LogicalPosition, action: &str) {
    ui_tests::pointer(window, point, action);
}
