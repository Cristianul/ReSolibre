//! Compiled shared-panel checks use an isolated testing backend and existing queue callbacks.

use super::*;
use crate::{TrackRow, presentation};
use resolibre_audio::{
    AudioOutput,
    session::{Command, Session},
};
use slint::{Model, ModelRc, VecModel};

/// Default surfaces and independent toggles preserve queue identity, scroll and EQ processing.
#[test]
fn playlist_visibility_reuses_window_and_actions() {
    i_slint_backend_testing::init_no_event_loop();
    let main = PlayerWindow::new().unwrap();
    main.show().unwrap();
    assert!(main.get_eq_expanded());
    let rows = Rc::new(VecModel::from(
        (1..100)
            .map(|number| TrackRow {
                duration: "--:--".into(),
                search_text: Default::default(),
                id: number.to_string().into(),
                title: "Track".into(),
                number: number.to_string().into(),
            })
            .collect::<Vec<_>>(),
    ));
    main.set_tracks(ModelRc::from(rows.clone()));
    main.set_current(0);
    main.set_eq_enabled(true);
    let host = connect(&main, Default::default(), Default::default());
    let selected = Rc::new(RefCell::new(String::new()));
    let observed = selected.clone();
    main.on_select_track(move |id| *observed.borrow_mut() = id.to_string());
    let calls = Rc::new(RefCell::new(Vec::new()));
    let observed = calls.clone();
    main.on_move_track(move |id, forward| observed.borrow_mut().push((id.to_string(), forward)));
    host.borrow_mut().show(&main, &host).unwrap();
    assert!(main.get_playlist_visible());
    let playlist = host.borrow().window.as_ref().unwrap().clone_strong();
    assert!(playlist.window().is_visible());
    for label in [
        "Minimize window",
        "Maximize window",
        "Restore window",
        "Close library",
    ] {
        assert!(
            i_slint_backend_testing::ElementHandle::find_by_accessible_label(&playlist, label)
                .next()
                .is_none()
        );
    }
    assert!(
        i_slint_backend_testing::ElementHandle::find_by_accessible_label(&playlist, "Move library")
            .next()
            .is_some()
    );
    let opened = Rc::new(Cell::new(0));
    let observed = opened.clone();
    main.on_open_files(move || observed.set(observed.get() + 1));
    main.on_add_files(|| panic!("Open must use the existing Open action"));
    crate::ui_tests::click(&playlist, "Open files");
    assert_eq!(opened.get(), 1);
    assert!(
        i_slint_backend_testing::ElementHandle::find_by_accessible_label(&main, "Open files")
            .next()
            .is_none()
    );
    main.set_picking(true);
    host.borrow().refresh(&main);
    i_slint_backend_testing::ElementHandle::find_by_accessible_label(&playlist, "Open files")
        .next()
        .unwrap()
        .invoke_accessible_default_action();
    assert_eq!(opened.get(), 1, "pending picker blocks duplicate requests");
    main.set_picking(false);
    host.borrow().refresh(&main);
    playlist.invoke_select_track("41".into());
    playlist.invoke_move_track("41".into(), true);
    assert_eq!(&*selected.borrow(), "41");
    assert_eq!(&*calls.borrow(), &[("41".to_string(), true)]);
    main.set_current(80);
    main.invoke_reveal_current();
    assert_eq!(playlist.get_selected_id(), "81");
    assert!(playlist.get_playlist_scroll() < -2000.0);
    main.invoke_toggle_playlist();
    assert!(!playlist.window().is_visible());
    crate::ui_tests::click(&main, "Show current track in library");
    assert!(playlist.window().is_visible());
    assert_eq!(playlist.get_selected_id(), "81");
    main.set_current(0);
    playlist.set_playlist_scroll(-216.0);
    main.invoke_toggle_playlist();
    assert!(!main.get_playlist_visible());
    assert!(!playlist.window().is_visible());
    assert!(main.get_eq_expanded());
    main.invoke_toggle_equalizer();
    assert!(!main.get_eq_expanded());
    assert!(
        main.get_eq_enabled(),
        "hiding the drawer must not bypass DSP"
    );
    rows.push(TrackRow {
        duration: "--:--".into(),
        search_text: Default::default(),
        id: "100".into(),
        title: "Added while hidden".into(),
        number: "100".into(),
    });
    main.invoke_toggle_playlist();
    assert!(playlist.window().is_visible());
    assert_eq!(playlist.get_tracks().row_count(), 100);
    assert_eq!(playlist.get_playlist_scroll(), -216.0);
    assert!(!main.get_eq_expanded());
    playlist
        .window()
        .dispatch_event(slint::platform::WindowEvent::CloseRequested);
    assert!(!main.get_playlist_visible());
    main.invoke_toggle_playlist();
    crate::ui_tests::click(&main, "Library");
    assert!(!main.get_playlist_visible());
    main.invoke_toggle_playlist();
    main.set_closing(true);
    main.invoke_toggle_playlist();
    assert!(
        main.get_playlist_visible(),
        "shutdown ignores new visibility intents"
    );
    host.borrow_mut().close();
    assert!(!playlist.window().is_visible());
}

/// Queue edits from the separate surface reach the production worker, without a second session.
#[test]
fn detached_queue_uses_production_bindings() {
    i_slint_backend_testing::init_no_event_loop();
    let directory =
        std::env::temp_dir().join(format!("resolibre-detach-audio-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let path = crate::ui_tests::write_ui_fixture(&directory);
    let original = std::fs::read(&path).unwrap();
    let main = PlayerWindow::new().unwrap();
    let session = Session::start(AudioOutput::Silent).unwrap();
    let controller = Rc::new(RefCell::new(presentation::Controller::new(session)));
    presentation::connect(&main, &controller);
    let host = connect(&main, Default::default(), Default::default());
    controller.borrow_mut().submit(
        &main,
        Command::Open(vec![path.clone(), "/missing/two.flac".into()]),
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while main.get_tracks().row_count() != 2
        || !main.get_playing()
        || main.get_library_duration() != "02:10+"
    {
        controller.borrow_mut().refresh(&main);
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    host.borrow_mut().show(&main, &host).unwrap();
    let detached = host.borrow().window.as_ref().unwrap().clone_strong();
    assert_eq!(detached.get_total_duration(), "02:10+");
    assert_eq!(detached.get_tracks().row_data(0).unwrap().duration, "02:10");
    let first = detached.get_tracks().row_data(1).unwrap().id;
    let generation = controller.borrow().snapshot().generation;
    detached.invoke_analyze_track(first.clone());
    while !detached
        .get_analysis_states()
        .iter()
        .any(|row| row.id == first && row.status == 4)
    {
        controller.borrow_mut().refresh(&main);
        host.borrow().refresh(&main);
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert_eq!(controller.borrow().snapshot().generation, generation);
    assert!(main.get_playing());
    detached.invoke_analyze_all();
    while !detached.get_analysis_busy() {
        controller.borrow_mut().refresh(&main);
        host.borrow().refresh(&main);
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    detached.invoke_cancel_analysis();
    while detached.get_analysis_busy() {
        controller.borrow_mut().refresh(&main);
        host.borrow().refresh(&main);
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    detached.invoke_remove_track(first);
    while main.get_tracks().row_count() != 1 {
        controller.borrow_mut().refresh(&main);
        host.borrow().refresh(&main);
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert_eq!(detached.get_tracks().row_count(), 1);
    assert_eq!(detached.get_total_duration(), "02:10");
    assert_eq!(
        detached.get_tracks().row_data(0).unwrap().title,
        main.get_track_title()
    );
    main.invoke_toggle_playlist();
    controller.borrow_mut().refresh(&main);
    assert!(main.get_playing());
    host.borrow_mut().show(&main, &host).unwrap();
    main.set_closing(true);
    host.borrow().refresh(&main);
    assert!(detached.get_closing());
    host.borrow_mut().close();
    drop(controller);
    drop(main);
    assert_eq!(std::fs::read(&path).unwrap(), original);
    std::fs::remove_dir_all(directory).unwrap();
}

/// Captures the actual two production surfaces, then verifies native placement and hide lifecycle.
#[test]
#[ignore = "requires X11/XWayland and RESOUL_PREVIEW_DIR; run individually"]
fn native_playlist_preview() {
    let wayland = std::env::var_os("RESOUL_PREVIEW_WAYLAND").is_some();
    if !wayland {
        crate::window_snap::select_x11().unwrap();
    }
    let directory = std::path::PathBuf::from(
        std::env::var_os("RESOUL_PREVIEW_DIR").expect("preview directory"),
    );
    let main = PlayerWindow::new().unwrap();
    crate::equalizer::initialize(&main);
    let rows = (1..=20)
        .map(|number| TrackRow {
            duration: crate::library_duration::track(Some(180 + number)).into(),
            search_text: Default::default(),
            id: number.to_string().into(),
            number: number.to_string().into(),
            title: format!("{number:02} · Night Drive — 音楽.flac").into(),
        })
        .collect::<Vec<_>>();
    main.set_tracks(ModelRc::new(VecModel::from(rows)));
    main.set_analysis_states(ModelRc::new(VecModel::from(vec![
        crate::AnalysisRow {
            id: "1".into(),
            status: 3,
        },
        crate::AnalysisRow {
            id: "2".into(),
            status: 4,
        },
        crate::AnalysisRow {
            id: "3".into(),
            status: 2,
        },
        crate::AnalysisRow {
            id: "4".into(),
            status: 1,
        },
    ])));
    main.set_analysis_summary("1 analyzed · 1 failed · 2 pending".into());
    main.set_analysis_busy(true);
    main.set_library_duration("01:03:30".into());
    main.set_current(2);
    main.set_track_title("Night Drive — 音楽".into());
    main.set_playing(true);
    main.set_state_label("Playing · Local audio".into());
    main.set_can_play(true);
    let host = connect(&main, Default::default(), Default::default());
    let weak = main.as_weak();
    let owner = host.clone();
    let timer = slint::Timer::default();
    let mut step = 0;
    let mut moved_position = None;
    timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_millis(250),
        move || {
            let main = weak.upgrade().unwrap();
            if step == 0 {
                owner.borrow_mut().show(&main, &owner).unwrap();
            } else if step <= 2 {
                owner.borrow().refresh(&main);
            } else if step == 3 {
                owner.borrow().refresh(&main);
                assert!(main.get_eq_expanded());
                if !wayland {
                    use slint::winit_030::WinitWindowAccessor;
                    let player_position = main
                        .window()
                        .with_winit_window(|native| native.outer_position().unwrap())
                        .unwrap();
                    let queue_position = owner
                        .borrow()
                        .window
                        .as_ref()
                        .unwrap()
                        .window()
                        .with_winit_window(|native| native.outer_position().unwrap())
                        .unwrap();
                    assert_eq!(
                        queue_position.x,
                        player_position.x + main.window().size().width as i32
                    );
                    assert_eq!(queue_position.y, player_position.y);
                }
                for (window, name) in [
                    (main.window(), "player.png"),
                    (
                        owner.borrow().window.as_ref().unwrap().window(),
                        "playlist.png",
                    ),
                ] {
                    use slint::winit_030::WinitWindowAccessor;
                    assert_eq!(
                        window.with_winit_window(|native| native.is_decorated()),
                        Some(false)
                    );
                    let pixels = window.take_snapshot().unwrap();
                    assert_eq!(pixels.as_bytes()[3], 0, "window corner must be transparent");
                    image::save_buffer(
                        directory.join(if wayland {
                            format!("wayland-{name}")
                        } else {
                            name.to_owned()
                        }),
                        pixels.as_bytes(),
                        pixels.width(),
                        pixels.height(),
                        image::ColorType::Rgba8,
                    )
                    .unwrap();
                }
            } else if step == 4 {
                let playlist = owner.borrow().test_window();
                crate::ui_tests::click(&playlist, "Toggle library search");
                playlist.set_search_query("03".into());
                assert_eq!(playlist.get_visible_tracks().row_count(), 1);
                assert_eq!(playlist.get_visible_current(), 0);
                assert_eq!(playlist.get_total_duration(), "01:03:30");
                assert_eq!(
                    playlist.get_visible_tracks().row_data(0).unwrap().duration,
                    "03:03"
                );
                let pixels = playlist.window().take_snapshot().unwrap();
                image::save_buffer(
                    directory.join(if wayland {
                        "wayland-library-search.png"
                    } else {
                        "library-search.png"
                    }),
                    pixels.as_bytes(),
                    pixels.width(),
                    pixels.height(),
                    image::ColorType::Rgba8,
                )
                .unwrap();
                if !wayland {
                    // Move well outside the magnetic threshold before hide/show.
                    playlist
                        .window()
                        .set_position(slint::PhysicalPosition::new(80, 140));
                }
                playlist
                    .window()
                    .set_size(slint::LogicalSize::new(320.0, 516.0));
            } else if step == 5 {
                let playlist = owner.borrow().test_window();
                let pixels = playlist.window().take_snapshot().unwrap();
                image::save_buffer(
                    directory.join(if wayland {
                        "wayland-library-minimum.png"
                    } else {
                        "library-minimum.png"
                    }),
                    pixels.as_bytes(),
                    pixels.width(),
                    pixels.height(),
                    image::ColorType::Rgba8,
                )
                .unwrap();
                if !wayland {
                    use slint::winit_030::WinitWindowAccessor;
                    moved_position = playlist
                        .window()
                        .with_winit_window(|native| native.outer_position().unwrap());
                }
                crate::ui_tests::click(&main, "Library");
                assert!(!main.get_playlist_visible());
                assert!(!playlist.window().is_visible());
                assert!(main.get_eq_expanded());
            } else if step == 6 {
                main.set_current(19);
                crate::ui_tests::click(&main, "Show current track in library");
            } else if step <= 8 {
                owner.borrow().refresh(&main);
            } else {
                let playlist = owner.borrow().test_window();
                assert!(playlist.window().is_visible());
                assert!(main.get_playlist_visible());
                assert_eq!(playlist.get_selected_id(), "20");
                assert!(playlist.get_playlist_scroll() < -200.0);
                if !wayland {
                    use slint::winit_030::WinitWindowAccessor;
                    assert_eq!(
                        playlist
                            .window()
                            .with_winit_window(|native| native.has_focus()),
                        Some(true)
                    );
                    let restored = playlist
                        .window()
                        .with_winit_window(|native| native.outer_position().unwrap());
                    assert_eq!(
                        restored, moved_position,
                        "PL must preserve the user's placement"
                    );
                }
                slint::quit_event_loop().unwrap();
            }
            step += 1;
        },
    );
    main.run().unwrap();
    host.borrow_mut().close();
}
