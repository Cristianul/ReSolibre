//! Display-only motion; track identity and measured targets come from the session snapshot.

use crate::PlayerWindow;
use resolibre_audio::session::Snapshot;
use resolibre_core::session::SessionState;
use resolibre_core::spectrum::live::{BAND_COUNT, smooth};
use slint::{ComponentHandle, Model, VecModel};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

struct Motion {
    current: [f32; BAND_COUNT],
    target: [f32; BAND_COUNT],
    last_tick: Instant,
    model: Rc<VecModel<f32>>,
}

pub(crate) struct View {
    motion: Rc<RefCell<Motion>>,
    identity: Option<(u64, u64)>,
    timer: slint::Timer,
}

impl Default for View {
    /// Owns one fixed-size model and a stopped timer; idle players perform no animation work.
    fn default() -> Self {
        Self {
            motion: Rc::new(RefCell::new(Motion {
                current: [0.0; BAND_COUNT],
                target: [0.0; BAND_COUNT],
                last_tick: Instant::now(),
                model: Rc::new(VecModel::from(vec![0.0; BAND_COUNT])),
            })),
            identity: None,
            timer: slint::Timer::default(),
        }
    }
}

impl View {
    /// Releases animation scheduling immediately when the controller begins closing.
    pub(crate) fn stop(&self) {
        self.timer.stop();
    }

    /// Starts motion only for visible playback, freezing pause and clearing stale track/seek generations.
    pub(crate) fn observe(&mut self, window: &PlayerWindow, snapshot: &Snapshot) {
        let source = snapshot.live_spectrum;
        let identity = source.map(|levels| (snapshot.generation, levels.epoch));
        let mut motion = self.motion.borrow_mut();
        window.set_live_levels(motion.model.clone().into());
        window.set_live_available(source.is_some());
        if self.identity != identity {
            self.identity = identity;
            motion.current.fill(0.0);
            motion.target.fill(0.0);
            motion.publish();
        }
        if let Some(source) = source {
            motion.target = source.levels;
        }
        let active = source.is_some()
            && snapshot.state == SessionState::Playing
            && !window.window().is_minimized()
            && window.window().is_visible()
            && !window.get_closing();
        if !active || window.get_reduced_motion() {
            self.timer.stop();
            if active {
                motion.current = motion.target;
                motion.publish();
            }
            return;
        }
        if self.timer.running() {
            return;
        }
        motion.last_tick = Instant::now();
        let weak = window.as_weak();
        let shared = self.motion.clone();
        self.timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(16),
            move || {
                let Some(window) = weak.upgrade() else {
                    return;
                };
                if !window.get_playing()
                    || window.window().is_minimized()
                    || !window.window().is_visible()
                {
                    return;
                }
                let mut motion = shared.borrow_mut();
                let elapsed = motion.last_tick.elapsed().as_secs_f32();
                motion.last_tick = Instant::now();
                let target = motion.target;
                smooth(&mut motion.current, &target, elapsed);
                motion.publish();
            },
        );
    }
}

impl Motion {
    /// Updates only the existing 24 display rows; no history or PCM enters the UI.
    fn publish(&self) {
        for (row, level) in self.current.iter().enumerate() {
            self.model.set_row_data(row, *level);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use resolibre_core::spectrum::live::LiveLevels;

    /// Actual view state freezes on pause, drops stale generations, and stops timers for hidden/reduced motion.
    #[test]
    fn lifecycle_discards_stale_bars_and_suspends_motion() {
        i_slint_backend_testing::init_no_event_loop();
        let window = PlayerWindow::new().unwrap();
        window.show().unwrap();
        window.set_reduced_motion(true);
        let mut view = View::default();
        let mut snapshot = Snapshot {
            state: SessionState::Playing,
            generation: 1,
            live_spectrum: Some(LiveLevels {
                epoch: 1,
                levels: [0.7; BAND_COUNT],
            }),
            ..Snapshot::default()
        };
        view.observe(&window, &snapshot);
        assert_eq!(window.get_live_levels().row_data(0), Some(0.7));
        assert!(!view.timer.running());
        snapshot.state = SessionState::Paused;
        snapshot.live_spectrum.as_mut().unwrap().levels.fill(0.2);
        view.observe(&window, &snapshot);
        assert_eq!(window.get_live_levels().row_data(0), Some(0.7));
        snapshot.live_spectrum.as_mut().unwrap().epoch += 1;
        view.observe(&window, &snapshot);
        assert_eq!(window.get_live_levels().row_data(0), Some(0.0));
        snapshot.state = SessionState::Playing;
        window.set_reduced_motion(false);
        view.observe(&window, &snapshot);
        assert!(view.timer.running());
        window.hide().unwrap();
        view.observe(&window, &snapshot);
        assert!(!view.timer.running());
        snapshot.live_spectrum = None;
        view.observe(&window, &snapshot);
        assert!(!window.get_live_available());
        assert!(window.get_live_levels().iter().all(|level| level == 0.0));
    }

    /// Exercises real native PCM and display timers, capturing motion before and after a paused seek.
    #[test]
    #[ignore = "requires isolated KWin/XWayland; run through check_window_snap.py"]
    fn native_motion() {
        use resolibre_audio::{
            AudioOutput,
            session::{Command, Session},
        };
        crate::window_snap::select_x11().unwrap();
        let directory =
            std::env::temp_dir().join(format!("resolibre-live-ui-{}", std::process::id()));
        std::fs::create_dir(&directory).unwrap();
        let path = crate::ui_tests::write_ui_fixture(&directory);
        let mut bytes = std::fs::read(&path).unwrap();
        for (frame, sample) in bytes[44..].as_chunks_mut::<2>().0.iter_mut().enumerate() {
            let frequency = if frame < 16000 { 250.0 } else { 1500.0 };
            let phase = std::f64::consts::TAU * frequency * frame as f64 / 8000.0;
            let envelope = 0.12 + 0.10 * (frame as f64 / 1200.0).sin();
            sample.copy_from_slice(&((phase.sin() * envelope * 32767.0) as i16).to_le_bytes());
        }
        std::fs::write(&path, &bytes).unwrap();
        let window = PlayerWindow::new().unwrap();
        let controller = Rc::new(RefCell::new(crate::presentation::Controller::new(
            Session::start(AudioOutput::Silent).unwrap(),
        )));
        crate::presentation::connect(&window, &controller);
        controller
            .borrow_mut()
            .submit(&window, Command::Open(vec![path.clone()]));
        let timer = slint::Timer::default();
        let weak = window.as_weak();
        let started = Instant::now();
        let mut stage = 0;
        let mut changed = 0;
        let mut previous = vec![0.0; BAND_COUNT];
        let mut frozen = Vec::new();
        let mut checkpoint = Instant::now();
        timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(16),
            move || {
                assert!(
                    started.elapsed() < Duration::from_secs(12),
                    "live UI deadline, stage {stage}"
                );
                let window = weak.upgrade().unwrap();
                controller.borrow_mut().refresh(&window);
                let levels: Vec<_> = window.get_live_levels().iter().collect();
                if levels != previous {
                    changed += 1;
                    previous = levels.clone();
                }
                match stage {
                    0 if changed >= 20 => {
                        save_frame(&window, "live-playing");
                        window.invoke_play_pause();
                        stage = 1;
                    }
                    1 if !window.get_playing() => {
                        frozen = levels;
                        checkpoint = Instant::now();
                        stage = 2;
                    }
                    2 if checkpoint.elapsed() > Duration::from_millis(150) => {
                        assert_eq!(levels, frozen, "paused animation moved");
                        let generation = controller.borrow().generation;
                        controller.borrow_mut().submit(
                            &window,
                            Command::Seek {
                                generation,
                                seconds: 2.5,
                            },
                        );
                        stage = 3;
                    }
                    3 if levels.iter().all(|level| *level == 0.0) => {
                        window.set_spectrum(true);
                        window.invoke_play_pause();
                        checkpoint = Instant::now();
                        stage = 4;
                    }
                    4 if window.get_playing()
                        && checkpoint.elapsed() > Duration::from_millis(350) =>
                    {
                        assert!(
                            levels.iter().any(|level| *level > 0.3),
                            "resume lacks real audio"
                        );
                        save_frame(&window, "live-after-seek");
                        window.invoke_stop();
                        stage = 5;
                    }
                    5 if !window.get_live_available()
                        && levels.iter().all(|level| *level == 0.0) =>
                    {
                        controller.borrow_mut().close();
                        stage = 6;
                    }
                    _ => {}
                }
            },
        );
        window.run().unwrap();
        timer.stop();
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        std::fs::remove_dir_all(directory).unwrap();
    }

    /// Saves a native rendered frame in the harness-owned output directory.
    fn save_frame(window: &PlayerWindow, name: &str) {
        let output = std::path::PathBuf::from(std::env::var_os("RESOUL_PREVIEW_DIR").unwrap());
        let scale = std::env::var("WINIT_X11_SCALE_FACTOR").unwrap();
        let pixels = window.window().take_snapshot().unwrap();
        image::save_buffer(
            output.join(format!("{name}-{scale}x.png")),
            pixels.as_bytes(),
            pixels.width(),
            pixels.height(),
            image::ColorType::Rgba8,
        )
        .unwrap();
    }
}
