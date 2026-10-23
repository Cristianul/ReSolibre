use super::*;
use slint::Model;

/// The tempo lane shares exact source bounds with the plot and exposes unknown gaps and accessible selection.
#[test]
fn tempo_lane_shows_measured_regions_and_unknown_gaps() {
    i_slint_backend_testing::init_no_event_loop();
    let window = AnalysisWindow::new().unwrap();
    let report = variable_report();
    let zones = timeline::zones(&report);
    assert_eq!(zones.row_count(), 3);
    let first = zones.row_data(0).unwrap();
    let gap = zones.row_data(1).unwrap();
    let last = zones.row_data(2).unwrap();
    assert_eq!((first.start, first.end), (0.0, 1.0 / 3.0));
    assert_eq!((gap.start, gap.end), (1.0 / 3.0, 2.0 / 3.0));
    assert_eq!(last.end, 1.0);
    assert!(first.details.contains("0:00.0–0:32.0"));
    assert!(first.details.contains("120.0 BPM"));
    assert!(last.details.contains("1:04.0–1:36.0"));
    assert!(last.details.contains("140.0 BPM"));
    assert!(!gap.measured);
    assert!(gap.details.contains("BPM unavailable"));
    window.set_tempo_zones(zones);
    window.show().unwrap();
    crate::ui_tests::click(&window, &last.details);
    assert_eq!(window.get_tempo_detail(), last.details);
    crate::ui_tests::click(&window, &gap.details);
    assert_eq!(window.get_tempo_detail(), gap.details);
    window.invoke_reset_tempo_selection();
    assert!(window.get_tempo_detail().contains("select an interval"));
}

/// Invalid geometry cannot allocate unbounded widgets, escape the source timeline or invent a neighbouring BPM.
#[test]
fn tempo_lane_bounds_invalid_measurements() {
    let mut report = (*report()).clone();
    let valid = report.tempo.segments[0].clone();
    for (start, end, bpm) in [
        (f64::NAN, 8.0, 120.0),
        (-1.0, 8.0, 120.0),
        (0.0, f64::INFINITY, 120.0),
        (8.0, 4.0, 120.0),
        (0.0, 17.0, 120.0),
        (16.005, 16.009, 120.0),
        (0.0, 8.0, f64::NAN),
    ] {
        let mut invalid = valid.clone();
        invalid.start_seconds = start;
        invalid.end_seconds = end;
        invalid.bpm = bpm;
        report.tempo.segments = vec![invalid];
        let zones = timeline::zones(&report);
        assert_eq!(zones.row_count(), 1);
        assert!(!zones.row_data(0).unwrap().measured);
    }
    report.tempo.segments = vec![valid; 10_000];
    let zones = timeline::zones(&report);
    assert_eq!(zones.row_count(), 1, "overlapping intervals are discarded");
    report.duration_seconds = 14_400.0;
    let template = report.tempo.segments[0].clone();
    report.tempo.segments = (0..600)
        .map(|index| {
            let mut segment = template.clone();
            segment.start_seconds = index as f64 * 20.0;
            segment.end_seconds = segment.start_seconds + 10.0;
            segment
        })
        .collect();
    assert_eq!(
        timeline::zones(&report).row_count(),
        1024,
        "region count stays bounded"
    );
    for duration in [f64::NAN, f64::INFINITY, -1.0, 0.0, 14_401.0] {
        report.duration_seconds = duration;
        assert_eq!(timeline::zones(&report).row_count(), 0);
    }
}

/// A complete variable report keeps its measured numbers visible instead of replacing them with a label.
#[test]
fn variable_tempo_displays_measured_range() {
    let mut tempo = report().tempo.clone();
    let mut faster = tempo.segments[0].clone();
    faster.bpm = 140.0;
    tempo.segments.push(faster);
    tempo.variable = true;
    tempo.bpm = None;
    let text = tempo_text(&tempo);
    assert!(text.contains("120.0–140.0 BPM"), "{text}");
    assert!(text.contains("local estimates"));
    tempo.segments.clear();
    assert!(tempo_text(&tempo).contains("inconclusive"));
}

/// Produces real shared-core measurements for view tests without a decoder or private file.
fn report() -> Arc<TrackAnalysis> {
    section_report(&[(Some(120.0), 16)])
}

/// Measures real local tempos separated by silence so UI tests exercise unknown regions too.
fn variable_report() -> Arc<TrackAnalysis> {
    section_report(&[(Some(120.0), 32), (None, 32), (Some(140.0), 32)])
}

/// Streams generated percussive sections through the production analyzer without retaining full PCM.
fn section_report(sections: &[(Option<f64>, usize)]) -> Arc<TrackAnalysis> {
    let mut analyzer = resolibre_core::analysis::TrackAnalyzer::new(8000, 1).unwrap();
    for &(bpm, seconds) in sections {
        for block in 0..seconds {
            let samples: Vec<f32> = (0..8000)
                .map(|index| {
                    let Some(bpm) = bpm else {
                        return 0.0;
                    };
                    let time = (block * 8000 + index) as f64 / 8000.0;
                    let pulse = (time - 0.25).rem_euclid(60.0 / bpm);
                    if time >= 0.25 && pulse < 0.03 {
                        ((std::f64::consts::TAU * 1000.0 * pulse).sin()
                            * (-pulse * 100.0).exp()
                            * 0.5) as f32
                    } else {
                        0.0
                    }
                })
                .collect();
            analyzer.push(&samples).unwrap();
        }
    }
    Arc::new(analyzer.finish().unwrap())
}

/// A completed inspection retains its report and geometry through navigation and unrelated analyses.
#[test]
fn result_view_retains_inspected_snapshot() {
    i_slint_backend_testing::init_no_event_loop();
    let window = AnalysisWindow::new().unwrap();
    window.show().unwrap();
    let result = report();
    let mut host = AnalysisHost {
        window: Some(window.clone_strong()),
        generation: Rc::new(Cell::new(Some(7))),
        ..Default::default()
    };
    let mut snapshot = Snapshot {
        generation: 7,
        analysis: AnalysisStatus::Complete(result),
        ..Default::default()
    };
    host.refresh(&snapshot);
    assert!(window.get_ready());
    assert!(window.get_tempo_text().contains("120.0 BPM"));
    assert!(window.get_interpretation().contains("Origin unknown"));
    assert!(window.get_frequency_note().contains("half the sample rate"));
    assert!(frequency_note(44100).contains("44.1 kHz"));
    assert!(frequency_note(44100).contains("22.05 kHz"));
    assert_eq!(window.get_plot().size().width, 512);
    let original = Arc::clone(host.rendered.as_ref().unwrap());
    let zones = window.get_tempo_zones();
    let status = window.get_status_text();
    for generation in [8, 7] {
        snapshot.generation = generation;
        for analysis in [
            AnalysisStatus::Cancelled,
            AnalysisStatus::Unavailable,
            AnalysisStatus::Running {
                decoded_seconds: 4.0,
            },
            AnalysisStatus::Complete(report()),
        ] {
            snapshot.analysis = analysis;
            host.refresh(&snapshot);
            assert!(window.get_ready());
            assert!(!window.get_running());
            assert_eq!(window.get_status_text(), status);
            assert_eq!(window.get_plot().size().width, 512);
            assert_eq!(window.get_tempo_zones(), zones);
            assert!(Arc::ptr_eq(host.rendered.as_ref().unwrap(), &original));
        }
    }
    assert_eq!(
        compression(AudioCodec::Flac),
        "Lossless codec; origin unknown"
    );
    assert_eq!(
        compression(AudioCodec::WavPack),
        "Compression mode unknown; origin unknown"
    );
}

/// A retry cannot freeze the previous completed result while its command is still queued.
#[test]
fn result_view_waits_for_requested_revision_and_rejects_other_tracks() {
    i_slint_backend_testing::init_no_event_loop();
    let window = AnalysisWindow::new().unwrap();
    window.show().unwrap();
    let mut host = AnalysisHost {
        window: Some(window.clone_strong()),
        generation: Rc::new(Cell::new(Some(7))),
        requested_revision: 9,
        ..Default::default()
    };
    let mut snapshot = Snapshot {
        generation: 7,
        revision: 8,
        analysis: AnalysisStatus::Complete(report()),
        ..Default::default()
    };
    host.refresh(&snapshot);
    assert!(!window.get_ready());
    assert!(host.rendered.is_none());
    snapshot.revision = 9;
    snapshot.analysis = AnalysisStatus::Running {
        decoded_seconds: 2.0,
    };
    host.refresh(&snapshot);
    assert!(window.get_running());
    snapshot.generation = 8;
    snapshot.analysis = AnalysisStatus::Complete(variable_report());
    host.refresh(&snapshot);
    assert!(!window.get_running());
    assert!(!window.get_ready());
    assert!(host.rendered.is_none());
    assert!(
        window
            .get_status_text()
            .contains("before analysis completed")
    );
}

/// The compiled artwork action remains keyboard accessible even when its hover treatment is hidden.
#[test]
fn artwork_action_and_close_restore_focus() {
    i_slint_backend_testing::init_no_event_loop();
    let main = PlayerWindow::new().unwrap();
    let session = resolibre_audio::session::Session::start(resolibre_audio::AudioOutput::Silent).unwrap();
    let controller = Rc::new(RefCell::new(Controller::new(session)));
    let host = connect(&main, &controller, Default::default());
    main.set_analysis_enabled(true);
    main.show().unwrap();
    main.invoke_focus_analysis();
    crate::ui_tests::send_key(&main, slint::platform::Key::Return);
    let window = host.borrow().window.as_ref().unwrap().clone_strong();
    assert!(window.window().is_visible());
    crate::ui_tests::click(&window, "Close analysis");
    assert!(!window.window().is_visible());
    crate::ui_tests::send_key(&main, slint::platform::Key::Return);
    assert!(window.window().is_visible());
    main.set_closing(true);
    host.borrow_mut().close();
    main.invoke_open_analysis();
    assert!(host.borrow().window.is_none());
    controller.borrow_mut().close();
}

/// Uses native rendering to inspect frequency/time/dB scales, long text and both DPI settings.
#[test]
#[ignore = "requires isolated native display and RESOUL_PREVIEW_DIR"]
fn native_analysis_preview() {
    crate::window_snap::select_x11().unwrap();
    let window = AnalysisWindow::new().unwrap();
    window.set_track_title("Generated percussion · original audio analysis".into());
    window.set_facts("PCM · 8 kHz · Mono · Lossless codec; origin unknown".into());
    window.show().unwrap();
    let mut host = AnalysisHost {
        window: Some(window.clone_strong()),
        generation: Rc::new(Cell::new(Some(1))),
        ..Default::default()
    };
    host.refresh(&Snapshot {
        generation: 1,
        analysis: AnalysisStatus::Complete(variable_report()),
        ..Default::default()
    });
    let weak = window.as_weak();
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::SingleShot,
        std::time::Duration::from_millis(350),
        move || {
            let window = weak.upgrade().unwrap();
            let last = window.get_tempo_zones().row_data(2).unwrap();
            crate::ui_tests::click(&window, &last.details);
            assert!(window.get_tempo_detail().contains("1:04.0–1:36.0"));
            let pixels = window.window().take_snapshot().unwrap();
            let directory =
                std::path::PathBuf::from(std::env::var_os("RESOUL_PREVIEW_DIR").unwrap());
            let scale = std::env::var("WINIT_X11_SCALE_FACTOR").unwrap_or_else(|_| "1".into());
            image::save_buffer(
                directory.join(format!("analysis-{scale}x.png")),
                pixels.as_bytes(),
                pixels.width(),
                pixels.height(),
                image::ColorType::Rgba8,
            )
            .unwrap();
            slint::quit_event_loop().unwrap();
        },
    );
    window.run().unwrap();
}

/// Exercises production decoder/session/window wiring with cancel, retry, BPM strip and keyboard focus at native DPI.
#[test]
#[ignore = "requires isolated native display and RESOUL_PREVIEW_DIR"]
fn native_analysis_journey() {
    use std::time::{Duration, Instant};
    crate::window_snap::select_x11().unwrap();
    let directory = std::env::temp_dir().join(format!("resolibre-analysis-ui-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let path = crate::ui_tests::write_ui_fixture(&directory);
    let mut bytes = std::fs::read(&path).unwrap();
    // Keep cancellation observably in flight even with optimized DSP and a software-rendered window opening.
    let size = 1800_u32 * 8000 * 2;
    bytes.resize(44 + size as usize, 0);
    bytes[4..8].copy_from_slice(&(36 + size).to_le_bytes());
    bytes[40..44].copy_from_slice(&size.to_le_bytes());
    for (frame, sample) in bytes[44..].as_chunks_mut::<2>().0.iter_mut().enumerate() {
        let time = frame as f64 / 8000.0;
        let phase = (time - 0.25).rem_euclid(0.5);
        let value = if time >= 0.25 && phase < 0.03 {
            ((std::f64::consts::TAU * 900.0 * phase).sin() * (-phase * 100.0).exp() * 12000.0)
                as i16
        } else {
            0
        };
        sample.copy_from_slice(&value.to_le_bytes());
    }
    std::fs::write(&path, &bytes).unwrap();
    let main = PlayerWindow::new().unwrap();
    let controller = Rc::new(RefCell::new(Controller::new(
        resolibre_audio::session::Session::start(resolibre_audio::AudioOutput::Silent).unwrap(),
    )));
    crate::presentation::connect(&main, &controller);
    let group = crate::window_group::SharedGroup::default();
    let events = crate::window_events::WindowEvents::new(main.window());
    crate::window_group::WindowGroup::watch(&group, &main, &events);
    let host = connect(&main, &controller, group);
    controller
        .borrow_mut()
        .submit(&main, Command::Open(vec![path.clone()]));
    let timer = slint::Timer::default();
    let weak = main.as_weak();
    let started = Instant::now();
    let mut stage = 0;
    let mut peak_rss_kib = 0_u64;
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(25),
        move || {
            assert!(
                started.elapsed() < Duration::from_secs(12),
                "analysis UI deadline stage {stage}"
            );
            let status = std::fs::read_to_string("/proc/self/status").unwrap();
            let rss = status
                .lines()
                .find_map(|line| line.strip_prefix("VmRSS:"))
                .unwrap()
                .split_whitespace()
                .next()
                .unwrap()
                .parse::<u64>()
                .unwrap();
            peak_rss_kib = peak_rss_kib.max(rss);
            let main = weak.upgrade().unwrap();
            controller.borrow_mut().refresh(&main);
            let snapshot = controller.borrow().snapshot();
            host.borrow_mut().refresh(&snapshot);
            match stage {
                0 if main.get_analysis_enabled() => {
                    eprintln!("analysis-ui-baseline-rss-kib: {rss}");
                    main.invoke_focus_analysis();
                    save_window(main.window(), "artwork-analysis");
                    crate::ui_tests::send_key(&main, slint::platform::Key::Return);
                    stage = 1;
                }
                1 if matches!(snapshot.analysis, AnalysisStatus::Running { .. }) => {
                    host.borrow()
                        .window
                        .as_ref()
                        .unwrap()
                        .invoke_cancel_analysis();
                    stage = 2;
                }
                2 if matches!(snapshot.analysis, AnalysisStatus::Cancelled) => {
                    assert!(snapshot.state == resolibre_core::session::SessionState::Playing);
                    main.invoke_open_analysis();
                    stage = 3;
                }
                3 if matches!(snapshot.analysis, AnalysisStatus::Complete(_)) => {
                    assert!(main.get_technical_info().contains("120.0 BPM"));
                    let window = host.borrow().window.as_ref().unwrap().clone_strong();
                    assert!(window.get_ready());
                    save_window(window.window(), "analysis-complete");
                    crate::ui_tests::click(&window, "Close analysis");
                    assert!(!window.window().is_visible());
                    stage = 4;
                }
                4 => {
                    assert!(main.get_technical_info().contains("120.0 BPM"));
                    crate::ui_tests::send_key(&main, slint::platform::Key::Return);
                    assert!(host.borrow().window.as_ref().unwrap().window().is_visible());
                    eprintln!("analysis-ui-peak-rss-kib: {peak_rss_kib}");
                    // This isolated harness uses software rendering; report separately from the hardware reference gate.
                    host.borrow_mut().close();
                    controller.borrow_mut().close();
                    stage = 5;
                }
                _ => {}
            }
        },
    );
    main.run().unwrap();
    timer.stop();
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    std::fs::remove_dir_all(directory).unwrap();
}

/// Saves only the explicitly-owned test surface in the harness output directory.
fn save_window(window: &slint::Window, name: &str) {
    let pixels = window.take_snapshot().unwrap();
    let directory = std::path::PathBuf::from(std::env::var_os("RESOUL_PREVIEW_DIR").unwrap());
    let scale = std::env::var("WINIT_X11_SCALE_FACTOR").unwrap();
    image::save_buffer(
        directory.join(format!("{name}-{scale}x.png")),
        pixels.as_bytes(),
        pixels.width(),
        pixels.height(),
        image::ColorType::Rgba8,
    )
    .unwrap();
}

/// The visible retry button replaces a completed report through production bindings without restarting audio.
#[test]
fn retry_button_replaces_completed_report() {
    i_slint_backend_testing::init_no_event_loop();
    let directory =
        std::env::temp_dir().join(format!("resolibre-analysis-retry-ui-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let path = crate::ui_tests::write_ui_fixture(&directory);
    let original = std::fs::read(&path).unwrap();
    let next_path = directory.join("next.wav");
    std::fs::write(&next_path, &original).unwrap();
    let main = PlayerWindow::new().unwrap();
    let session = resolibre_audio::session::Session::start_with_analysis_cache(
        resolibre_audio::AudioOutput::Silent,
        Some(directory.join("analysis")),
    )
    .unwrap();
    let controller = Rc::new(RefCell::new(Controller::new(session)));
    crate::presentation::connect(&main, &controller);
    let host = connect(&main, &controller, Default::default());
    controller
        .borrow_mut()
        .submit(&main, Command::Open(vec![path.clone(), next_path.clone()]));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
    let mut previous = None;
    let mut opened = false;
    loop {
        controller.borrow_mut().refresh(&main);
        let snapshot = controller.borrow().snapshot();
        host.borrow_mut().refresh(&snapshot);
        if !opened && main.get_analysis_enabled() {
            main.invoke_open_analysis();
            opened = true;
        }
        if let AnalysisStatus::Complete(report) = snapshot.analysis {
            if let Some((generation, previous)) = &previous {
                if !Arc::ptr_eq(previous, &report) {
                    assert_eq!(*generation, snapshot.generation);
                    assert_eq!(snapshot.state, resolibre_core::session::SessionState::Playing);
                    break;
                }
            } else {
                previous = Some((snapshot.generation, report));
                let window = host.borrow().window.as_ref().unwrap().clone_strong();
                window.invoke_retry_analysis();
            }
        }
        assert!(
            std::time::Instant::now() < deadline,
            "retry must publish a new report"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let inspected = Arc::clone(host.borrow().rendered.as_ref().unwrap());
    let view = host.borrow().window.as_ref().unwrap().clone_strong();
    let inspected_title = view.get_track_title();
    controller.borrow_mut().submit(&main, Command::Next);
    loop {
        controller.borrow_mut().refresh(&main);
        let snapshot = controller.borrow().snapshot();
        host.borrow_mut().refresh(&snapshot);
        assert!(Arc::ptr_eq(
            host.borrow().rendered.as_ref().unwrap(),
            &inspected
        ));
        assert_eq!(view.get_track_title(), inspected_title);
        if snapshot.current == Some(1) && main.get_analysis_enabled() {
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    view.invoke_retry_analysis();
    loop {
        controller.borrow_mut().refresh(&main);
        let snapshot = controller.borrow().snapshot();
        host.borrow_mut().refresh(&snapshot);
        if view.get_ready() {
            assert!(!Arc::ptr_eq(
                host.borrow().rendered.as_ref().unwrap(),
                &inspected
            ));
            assert_ne!(view.get_track_title(), inspected_title);
            assert_eq!(snapshot.current, Some(1));
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(std::fs::read(&next_path).unwrap(), original);
    host.borrow_mut().close();
    drop(host);
    drop(controller);
    assert_eq!(std::fs::read(&path).unwrap(), original);
    std::fs::remove_dir_all(directory).unwrap();
}
