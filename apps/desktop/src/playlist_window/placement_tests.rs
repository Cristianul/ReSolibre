//! Native remapping regression checks use only the harness-owned display and profile.

use super::*;
use slint::winit_030::WinitWindowAccessor;
use std::time::Duration;

/// A reused playlist must already have its saved coordinates when shown, even before refresh.
#[test]
#[ignore = "requires isolated KWin/XWayland; run through check_window_snap.py"]
fn native_playlist_remap_keeps_position() {
    crate::window_snap::select_x11().unwrap();
    let main = PlayerWindow::new().unwrap();
    let host = connect(&main, Default::default(), Default::default());
    main.show().unwrap();
    let timer = slint::Timer::default();
    let weak = main.as_weak();
    let mut step = 0;
    let mut waiting = 0;
    let mut first_id = None;
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(100),
        move || {
            use raw_window_handle::HasWindowHandle;
            let main = weak.upgrade().unwrap();
            if step == 0 {
                waiting += 1;
                assert!(
                    waiting < 30,
                    "main must become managed within the bounded startup wait"
                );
                if !host.borrow().ready_to_show(&main) {
                    return;
                }
                host.borrow_mut().show(&main, &host).unwrap();
            } else if step <= 5 {
                let playlist = host.borrow().test_window();
                let origin = host
                    .borrow()
                    .snapping
                    .borrow()
                    .position(main.window())
                    .unwrap();
                assert_eq!(
                    host.borrow().snapping.borrow().position(playlist.window()),
                    Some((origin.0 + main.window().size().width as i32, origin.1)),
                    "first map must dock before the refresh correction"
                );
                host.borrow().refresh(&main);
            } else if step == 6 {
                host.borrow()
                    .test_window()
                    .window()
                    .set_position(slint::PhysicalPosition::new(80, 140));
            } else {
                let playlist = host.borrow().test_window();
                let native_id = playlist
                    .window()
                    .with_winit_window(|native| native.window_handle().unwrap().as_raw())
                    .unwrap();
                assert_eq!(
                    *first_id.get_or_insert(native_id),
                    native_id,
                    "toggle must reuse one native window"
                );
                assert_eq!(
                    host.borrow().snapping.borrow().position(playlist.window()),
                    Some((80, 140)),
                    "position must survive every mapped frame without a refresh correction"
                );
                if step == 27 {
                    host.borrow_mut().close();
                    slint::quit_event_loop().unwrap();
                    return;
                }
                main.invoke_toggle_playlist();
                main.invoke_toggle_playlist();
                // Deliberately omit refresh: restoration belongs before map, not to a later UI tick.
            }
            step += 1;
        },
    );
    main.run().unwrap();
}

/// Restored theme/geometry and the Library model must be ready before either surface maps.
#[test]
#[ignore = "requires isolated KWin/XWayland; run through check_window_snap.py"]
fn native_prepared_startup() {
    use slint::{Model, ModelRc, VecModel};
    crate::window_snap::select_x11().unwrap();
    let main = PlayerWindow::new().unwrap();
    main.set_initializing(true);
    let session = resolibre_audio::session::Session::start(resolibre_audio::AudioOutput::Silent).unwrap();
    let controller = Rc::new(RefCell::new(crate::presentation::Controller::new(session)));
    crate::presentation::connect(&main, &controller);
    crate::interface_state::InterfaceState {
        spectrum: true,
        equalizer_visible: false,
        library_visible: true,
    }
    .apply(&main);
    main.set_tracks(ModelRc::new(VecModel::from(vec![crate::TrackRow {
        id: "1".into(),
        number: "1".into(),
        title: "Restored music".into(),
        duration: "03:25".into(),
        ..Default::default()
    }])));
    let host = connect(&main, Default::default(), Default::default());
    let timer = slint::Timer::default();
    let weak = main.as_weak();
    let mut shown = false;
    let mut checks = 0;
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(33),
        move || {
            let main = weak.upgrade().unwrap();
            if !shown {
                assert!(!main.window().is_visible());
                if !host.borrow_mut().prepare_startup(&main, &host).unwrap() {
                    return;
                }
                let library = host.borrow().test_window();
                assert!(!library.window().is_visible());
                assert!(library.get_spectrum());
                assert_eq!(library.get_tracks().row_data(0).unwrap().duration, "03:25");
                assert_eq!(main.get_eq_reveal(), 0.0);
                main.show().unwrap();
                host.borrow_mut().show(&main, &host).unwrap();
                main.set_initializing(false);
                shown = true;
            }
            let size = main
                .window()
                .size()
                .to_logical(main.window().scale_factor());
            assert_eq!(size.height, 264.0, "no startup drawer resize or animation");
            let library = host.borrow().test_window();
            let origin = host
                .borrow()
                .snapping
                .borrow()
                .position(main.window())
                .unwrap();
            assert_eq!(
                host.borrow().snapping.borrow().position(library.window()),
                Some((origin.0 + main.window().size().width as i32, origin.1)),
                "both positions must be correct without refresh corrections"
            );
            checks += 1;
            if checks == 8 {
                host.borrow_mut().close();
                slint::quit_event_loop().unwrap();
            }
        },
    );
    slint::run_event_loop_until_quit().unwrap();
}
