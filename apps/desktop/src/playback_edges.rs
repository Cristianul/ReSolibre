//! Panel bindings for the shared TRIM/FADE policy; no detection or envelope math lives in the UI.
use crate::{PlayerWindow, presentation::Controller, submit};
use resolibre_audio::session::{Command, MixStatus, Snapshot};
use resolibre_core::playback_edges::{EdgeAction, FadeMode};
use slint::ComponentHandle;
use std::{cell::RefCell, rc::Rc};

/// Sends button actions to the same session entry point used by future shortcuts.
pub fn connect(window: &PlayerWindow, controller: &Rc<RefCell<Controller>>) {
    let weak = window.as_weak();
    let owner = Rc::clone(controller);
    window.on_toggle_mix(move || {
        if let Some(window) = weak.upgrade() {
            submit(&window, &owner, Command::SetMix(!window.get_mix_enabled()));
        }
    });
    let weak = window.as_weak();
    let owner = Rc::clone(controller);
    window.on_toggle_trim(move || {
        if let Some(window) = weak.upgrade() {
            submit(&window, &owner, Command::Edges(EdgeAction::ToggleTrim));
        }
    });
    let weak = window.as_weak();
    let owner = Rc::clone(controller);
    window.on_cycle_fade(move || {
        if let Some(window) = weak.upgrade() {
            submit(&window, &owner, Command::Edges(EdgeAction::CycleFade));
        }
    });
}

/// Displays confirmed intent and original-timeline skipped ranges; pending analysis invents no cuts.
pub fn show(window: &PlayerWindow, snapshot: &Snapshot) {
    window.set_mix_enabled(snapshot.mix_enabled);
    window.set_mix_caption(
        match snapshot.mix_status {
            MixStatus::Off => "",
            MixStatus::Preparing => "PREP",
            MixStatus::Ready => "READY",
            MixStatus::ReadyCrossfade => "FADE…",
            MixStatus::Crossfading => "FADE",
            MixStatus::TempoMixing => "TEMPO",
            MixStatus::BeatMixing => "SYNC",
            MixStatus::Unavailable => "—",
        }
        .into(),
    );
    if snapshot.mix_status == MixStatus::BeatMixing
        && let Some(ratio) = snapshot.mix_beat_ratio
        && ratio != resolibre_core::transition::BeatRatio::OneToOne
    {
        let (outgoing, incoming) = ratio.counts();
        window.set_mix_caption(format!("{outgoing}:{incoming}").into());
    }
    let mix_region = snapshot.mix_region.filter(|region| {
        snapshot.mix_enabled
            && region.start.is_finite()
            && region.end.is_finite()
            && region.start >= 0.0
            && region.end > region.start
            && snapshot
                .duration
                .is_some_and(|duration| duration.is_finite() && duration >= region.end)
    });
    let duration = snapshot.duration.unwrap_or(1.0);
    window.set_mix_start(mix_region.map_or(0.0, |region| (region.start / duration) as f32));
    window.set_mix_end(mix_region.map_or(0.0, |region| (region.end / duration) as f32));
    window.set_trim_enabled(snapshot.edges.trim);
    window.set_trim_regions(snapshot.trim_end.is_some() && snapshot.edges.trim);
    window.set_fade_mode(match snapshot.edges.fade {
        FadeMode::Off => 0,
        FadeMode::In => 1,
        FadeMode::Out => 2,
        FadeMode::Both => 3,
    });
    window.set_trim_start(snapshot.trim_start as f32);
    window.set_trim_end(
        snapshot
            .trim_end
            .unwrap_or(snapshot.duration.unwrap_or(0.0)) as f32,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use i_slint_backend_testing::ElementHandle;
    use resolibre_audio::{
        AudioOutput,
        session::{EdgeStatus, Session},
    };
    use resolibre_core::playback_edges::PlaybackEdges;
    use std::time::{Duration, Instant};

    /// Button cycling reaches authoritative state while yellow ranges preserve source-time seeking.
    #[test]
    fn buttons_share_session_policy_and_seek_keeps_original_time() {
        i_slint_backend_testing::init_no_event_loop();
        let window = PlayerWindow::new().unwrap();
        let session = Session::start(AudioOutput::Silent).unwrap();
        let controller = Rc::new(RefCell::new(Controller::new(session)));
        crate::presentation::connect(&window, &controller);
        window.show().unwrap();
        crate::ui_tests::click(&window, "Trim off");
        let deadline = Instant::now() + Duration::from_secs(3);
        while !window.get_trim_enabled() {
            controller.borrow_mut().refresh(&window);
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        for (label, expected) in [
            ("Fade off", 1),
            ("Fade in", 2),
            ("Fade out", 3),
            ("Fade in and out", 0),
        ] {
            crate::ui_tests::click(&window, label);
            while window.get_fade_mode() != expected {
                controller.borrow_mut().refresh(&window);
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        crate::ui_tests::click(&window, "Mix off");
        while !window.get_mix_enabled() {
            controller.borrow_mut().refresh(&window);
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        let mut snapshot = Snapshot {
            edges: PlaybackEdges {
                trim: true,
                fade: FadeMode::Both,
            },
            edge_status: EdgeStatus::Ready,
            duration: Some(300.0),
            trim_start: 10.0,
            trim_end: Some(290.0),
            ..Snapshot::default()
        };
        snapshot.mix_status = MixStatus::ReadyCrossfade;
        show(&window, &snapshot);
        assert_eq!(window.get_mix_caption(), "FADE…");
        snapshot.mix_status = MixStatus::Ready;
        show(&window, &snapshot);
        assert_eq!(window.get_mix_caption(), "READY");
        snapshot.mix_status = MixStatus::TempoMixing;
        show(&window, &snapshot);
        assert_eq!(window.get_mix_caption(), "TEMPO");
        snapshot.mix_status = MixStatus::BeatMixing;
        show(&window, &snapshot);
        assert_eq!(window.get_mix_caption(), "SYNC");
        snapshot.mix_beat_ratio = Some(resolibre_core::transition::BeatRatio::OneToThree);
        show(&window, &snapshot);
        assert_eq!(window.get_mix_caption(), "1:3");
        snapshot.mix_status = MixStatus::TempoMixing;
        show(&window, &snapshot);
        assert_eq!(
            window.get_mix_caption(),
            "TEMPO",
            "unverified phase cannot claim a synchronized ratio"
        );
        window.set_duration(300.0);
        window.set_can_seek(true);
        let observed = Rc::new(RefCell::new(0.0));
        let target = Rc::clone(&observed);
        window.on_seek(move |seconds| *target.borrow_mut() = seconds);
        let seek = ElementHandle::find_by_accessible_label(&window, "Playback position")
            .next()
            .unwrap();
        seek.set_accessible_value("295");
        assert_eq!(
            *observed.borrow(),
            295.0,
            "native policy, not the view, clamps skipped areas"
        );
        assert_eq!(window.get_duration(), 300.0);
        assert_eq!(window.get_trim_start(), 10.0);
        window.set_closing(true);
        ElementHandle::find_by_accessible_label(&window, "Fade in and out")
            .next()
            .unwrap()
            .invoke_accessible_default_action();
        assert_eq!(window.get_fade_mode(), 3);
        controller.borrow_mut().close();
    }

    /// Only finite, enabled, source-bounded transition regions reach the seek face.
    #[test]
    fn mix_region_rejects_invalid_bounds_and_clears_when_disabled() {
        use resolibre_core::playback_edges::PlaybackRegion;
        i_slint_backend_testing::init_no_event_loop();
        let window = PlayerWindow::new().unwrap();
        let mut snapshot = Snapshot {
            mix_enabled: true,
            duration: Some(100.0),
            mix_region: Some(PlaybackRegion {
                start: 82.0,
                end: 90.0,
            }),
            ..Snapshot::default()
        };
        show(&window, &snapshot);
        assert_eq!(window.get_mix_start(), 0.82);
        assert_eq!(window.get_mix_end(), 0.9);
        for (start, end) in [
            (f64::NAN, 90.0),
            (80.0, f64::INFINITY),
            (-1.0, 90.0),
            (90.0, 80.0),
            (90.0, 90.0),
            (80.0, 101.0),
        ] {
            snapshot.mix_region = Some(PlaybackRegion { start, end });
            show(&window, &snapshot);
            assert_eq!(window.get_mix_start(), 0.0);
            assert_eq!(window.get_mix_end(), 0.0);
        }
        snapshot.mix_region = Some(PlaybackRegion {
            start: 82.0,
            end: 90.0,
        });
        for duration in [None, Some(f64::NAN), Some(f64::INFINITY), Some(0.0)] {
            snapshot.duration = duration;
            show(&window, &snapshot);
            assert_eq!(window.get_mix_end(), 0.0);
        }
        snapshot.duration = Some(100.0);
        snapshot.mix_enabled = false;
        show(&window, &snapshot);
        assert_eq!(window.get_mix_end(), 0.0);
        snapshot.mix_enabled = true;
        snapshot.mix_region = None;
        show(&window, &snapshot);
        assert_eq!(window.get_mix_end(), 0.0);
    }

    /// Renders both palettes and asserts real yellow skipped segments at native scale.
    #[test]
    #[ignore = "run inside tools/check_window_snap.py"]
    fn native_processing_preview() {
        crate::window_snap::select_x11().unwrap();
        let window = PlayerWindow::new().unwrap();
        crate::equalizer::initialize(&window);
        window.set_trim_enabled(true);
        window.set_mix_enabled(true);
        window.set_mix_caption("FADE…".into());
        window.set_trim_regions(true);
        window.set_fade_mode(3);
        window.set_can_seek(true);
        window.set_duration(100.0);
        window.set_position(35.0);
        window.set_trim_start(10.0);
        window.set_trim_end(90.0);
        window.set_mix_start(0.82);
        window.set_mix_end(0.90);
        window.set_track_title("TRIM + MIX · original timeline".into());
        window.set_time_label("0:35".into());
        window.set_duration_label("1:40".into());
        let mut rgba = vec![0_u8; 512 * 512 * 4];
        for pixel in rgba.as_chunks_mut::<4>().0 {
            pixel.copy_from_slice(&[170, 30, 70, 255]);
        }
        crate::artwork::show(
            &window,
            Some(&resolibre_audio::metadata::Cover { rgba: rgba.into() }),
        );
        let weak = window.as_weak();
        let timer = slint::Timer::default();
        let mut phase = 0;
        timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(250),
            move || {
                let window = weak.upgrade().unwrap();
                if phase == 1 || phase == 3 {
                    let pixels = window.window().take_snapshot().unwrap();
                    let yellow = pixels
                        .as_bytes()
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .filter(|p| p[0] == 217 && p[1] == 185 && p[2] == 90)
                        .count();
                    assert!(yellow > 100, "both skipped regions must be visible");
                    let orange = pixels
                        .as_bytes()
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .filter(|pixel| pixel[..3] == [237, 120, 79])
                        .count();
                    let hatch = pixels
                        .as_bytes()
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .filter(|pixel| pixel[..3] == [136, 63, 43])
                        .count();
                    assert!(orange > 30, "MIX interval and start must be visible");
                    assert!(hatch > 5, "MIX texture must differ from solid TRIM");
                    let scale = window.window().scale_factor();
                    let requested: f32 = std::env::var("WINIT_X11_SCALE_FACTOR")
                        .unwrap()
                        .parse()
                        .unwrap();
                    assert_eq!(
                        scale, requested,
                        "capture must use the requested native scale"
                    );
                    let width = pixels.width() as usize;
                    let lower_keys = pixels
                        .as_bytes()
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .enumerate()
                        .filter(|(index, pixel)| {
                            *index / width > (220.0 * scale) as usize
                                && pixel[0] == 217
                                && pixel[1] == 185
                                && pixel[2] == 90
                        })
                        .count();
                    assert!(
                        lower_keys >= 12,
                        "TRIM LED must match the yellow seek regions"
                    );
                    let center =
                        (((100.0 * scale) as usize * width) + (100.0 * scale) as usize) * 4;
                    assert_eq!(
                        &pixels.as_bytes()[center..center + 4],
                        &[170, 30, 70, 255],
                        "current cover replaces the illustration"
                    );
                    let directory =
                        std::path::PathBuf::from(std::env::var_os("RESOUL_PREVIEW_DIR").unwrap());
                    image::save_buffer(
                        directory.join(format!(
                            "processing-{}-{scale}x.png",
                            if phase == 1 { "green" } else { "spectrum" }
                        )),
                        pixels.as_bytes(),
                        pixels.width(),
                        pixels.height(),
                        image::ColorType::Rgba8,
                    )
                    .unwrap();
                }
                if phase == 2 {
                    window.set_spectrum(true);
                    window.set_mix_enabled(true);
                    window.set_mix_caption("SYNC".into());
                }
                if phase == 3 {
                    slint::quit_event_loop().unwrap();
                }
                phase += 1;
            },
        );
        window.run().unwrap();
    }
}
