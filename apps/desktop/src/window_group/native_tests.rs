//! Real WM stacking checks on the harness-owned desktop, using production hosts.

use super::*;
use crate::{PlayerWindow, ThemesWindow};
use x11rb::protocol::xproto::AtomEnum;

/// Reads one owned surface ID; never discovers targets by title or process enumeration.
fn id(window: &slint::Window) -> u32 {
    window
        .with_winit_window(|native| {
            let handle = native.window_handle().unwrap();
            match handle.as_raw() {
                RawWindowHandle::Xlib(handle) => handle.window as u32,
                RawWindowHandle::Xcb(handle) => handle.window.get(),
                _ => panic!("X11 test only"),
            }
        })
        .unwrap()
}

/// Reads bounded WM state only inside the isolated compositor, for stacking/focus assertions.
fn property(adapter: &Restacker, name: &[u8]) -> Vec<u32> {
    let atom = adapter
        .connection
        .intern_atom(false, name)
        .unwrap()
        .reply()
        .unwrap()
        .atom;
    let reply = adapter
        .connection
        .get_property(false, adapter.root, atom, AtomEnum::WINDOW, 0, 128)
        .unwrap()
        .reply()
        .unwrap();
    assert_eq!(reply.bytes_after, 0);
    reply.value32().unwrap().collect()
}

/// Emulates a taskbar activation on the private WM, avoiding application focus-stealing policy.
fn activate(adapter: &Restacker, target: u32) {
    let event = ClientMessageEvent::new(32, target, adapter.active, [2, 0, 0, 0, 0]);
    adapter
        .connection
        .send_event(
            false,
            adapter.root,
            EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
            event,
        )
        .unwrap();
    adapter.connection.flush().unwrap();
}

/// Every visible app host rises above an outsider; selected focus, hidden/minimized state survive.
#[test]
#[ignore = "requires isolated KWin/XWayland; run through check_window_snap.py"]
fn native_activation_journey() {
    crate::window_snap::select_x11().unwrap();
    let main = PlayerWindow::new().unwrap();
    let group = SharedGroup::default();
    let playlist = crate::playlist_window::connect(&main, Default::default(), group.clone());
    let themes = crate::themes::connect(&main, group);
    let outsider = ThemesWindow::new().unwrap();
    main.show().unwrap();
    playlist.borrow_mut().show(&main, &playlist).unwrap();
    main.invoke_open_themes();
    let queue = playlist.borrow().test_window();
    let chooser = themes.borrow().test_window();
    outsider.show().unwrap();
    let adapter = Restacker::connect().unwrap();
    let mut handles = None;
    let weak = main.as_weak();
    let timer = slint::Timer::default();
    let mut step = 0;
    timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_millis(250),
        move || {
            let main = weak.upgrade().unwrap();
            let (owned, foreign) = *handles.get_or_insert_with(|| {
                (
                    [id(main.window()), id(queue.window()), id(chooser.window())],
                    id(outsider.window()),
                )
            });
            match step {
                0 | 3 | 6 | 9 | 12 => activate(&adapter, foreign),
                1 | 4 | 7 => {
                    assert_eq!(property(&adapter, b"_NET_ACTIVE_WINDOW"), [foreign]);
                    let stack = property(&adapter, b"_NET_CLIENT_LIST_STACKING");
                    let foreign_index = stack.iter().position(|id| *id == foreign).unwrap();
                    assert!(
                        owned
                            .iter()
                            .all(|id| stack.iter().position(|entry| entry == id).unwrap()
                                < foreign_index)
                    );
                    activate(&adapter, owned[((step - 1) / 3) as usize]);
                }
                2 | 5 | 8 => {
                    let selected = owned[((step - 2) / 3) as usize];
                    assert_eq!(property(&adapter, b"_NET_ACTIVE_WINDOW"), [selected]);
                    let stack = property(&adapter, b"_NET_CLIENT_LIST_STACKING");
                    let foreign_index = stack.iter().position(|id| *id == foreign).unwrap();
                    assert!(
                        owned
                            .iter()
                            .all(|id| stack.iter().position(|entry| entry == id).unwrap()
                                > foreign_index),
                        "group did not rise: {stack:?}"
                    );
                    assert_eq!(stack.last(), Some(&selected));
                    if step == 8 {
                        main.invoke_toggle_playlist();
                        chooser.hide().unwrap();
                    }
                }
                10 => activate(&adapter, owned[0]),
                11 => {
                    assert!(!queue.window().is_visible());
                    assert!(!chooser.window().is_visible());
                    assert_eq!(property(&adapter, b"_NET_ACTIVE_WINDOW"), [owned[0]]);
                    main.invoke_toggle_playlist();
                    queue.window().set_minimized(true);
                }
                13 => activate(&adapter, owned[0]),
                _ => {
                    assert!(
                        queue
                            .window()
                            .with_winit_window(|native| native.is_minimized() == Some(true))
                            .unwrap()
                    );
                    assert!(!chooser.window().is_visible());
                    assert_eq!(property(&adapter, b"_NET_ACTIVE_WINDOW"), [owned[0]]);
                    playlist.borrow_mut().close();
                    themes.borrow_mut().close();
                    slint::quit_event_loop().unwrap();
                }
            }
            step += 1;
        },
    );
    main.run().unwrap();
}

/// Owns an unrelated process only on the harness display and reaps it even after an assertion.
struct ForeignWindow(std::process::Child);

impl ForeignWindow {
    /// Starts an independent X11 application; the private name scopes test-only discovery.
    fn start() -> Self {
        Self(
            std::process::Command::new("xmessage")
                .args([
                    "-name",
                    "resolibre-foreign-activation",
                    "-geometry",
                    "500x300+700+150",
                    "Isolated foreign window",
                ])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap(),
        )
    }

    /// Finds the one child on the owned desktop, reading bounded class properties only in tests.
    fn id(&self, adapter: &Restacker) -> Option<u32> {
        for id in property(adapter, b"_NET_CLIENT_LIST") {
            let reply = adapter
                .connection
                .get_property(false, id, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 64)
                .ok()?
                .reply()
                .ok()?;
            if reply.value.starts_with(b"resolibre-foreign-activation\0") {
                return Some(id);
            }
        }
        None
    }
}

impl Drop for ForeignWindow {
    /// Stops only the child created by this test, never a user's application.
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Delivers a typed click at the production event boundary; the headless WM has no physical seat.
fn click_client(group: &SharedGroup) {
    WindowGroup::handle_event(
        group,
        0,
        &winit::event::WindowEvent::MouseInput {
            device_id: winit::event::DeviceId::dummy(),
            state: winit::event::ElementState::Pressed,
            button: winit::event::MouseButton::Left,
        },
    );
}

/// Clicking an already-focused player must raise peers left behind a different process.
#[test]
#[ignore = "requires isolated KWin/XWayland; run through check_window_snap.py"]
fn native_click_raise_journey() {
    crate::window_snap::select_x11().unwrap();
    let main = PlayerWindow::new().unwrap();
    let group = SharedGroup::default();
    let host = crate::playlist_window::connect(&main, Default::default(), group.clone());
    main.show().unwrap();
    host.borrow_mut().show(&main, &host).unwrap();
    let queue = host.borrow().test_window();
    let outsider = ForeignWindow::start();
    let adapter = Restacker::connect().unwrap();
    let timer = slint::Timer::default();
    let weak = main.as_weak();
    let mut step = 0;
    let mut attempts = 0;
    timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_millis(200),
        move || {
            attempts += 1;
            assert!(attempts < 30, "foreign window mapping deadline");
            let Some(foreign) = outsider.id(&adapter) else {
                return;
            };
            let main = weak.upgrade().unwrap();
            let player = id(main.window());
            let playlist = id(queue.window());
            match step {
                0 => activate(&adapter, foreign),
                1 => activate(&adapter, player),
                2 => {
                    // Model a click-to-raise state: player focused/top, foreign between it and Playlist.
                    let event = ClientMessageEvent::new(
                        32,
                        foreign,
                        adapter.restack,
                        [
                            2,
                            player,
                            u32::from(x11rb::protocol::xproto::StackMode::BELOW),
                            0,
                            0,
                        ],
                    );
                    adapter
                        .connection
                        .send_event(
                            false,
                            adapter.root,
                            EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
                            event,
                        )
                        .unwrap();
                    adapter.connection.flush().unwrap();
                }
                3 => {
                    assert_eq!(property(&adapter, b"_NET_ACTIVE_WINDOW"), [player]);
                    let stack = property(&adapter, b"_NET_CLIENT_LIST_STACKING");
                    let foreign_index = stack.iter().position(|id| *id == foreign).unwrap();
                    assert!(stack.iter().position(|id| *id == playlist).unwrap() < foreign_index);
                    click_client(&group);
                }
                4 => {
                    let stack = property(&adapter, b"_NET_CLIENT_LIST_STACKING");
                    assert!(
                        stack.iter().position(|id| *id == playlist).unwrap()
                            > stack.iter().position(|id| *id == foreign).unwrap(),
                        "playlist remained behind foreign window"
                    );
                    assert_eq!(property(&adapter, b"_NET_ACTIVE_WINDOW"), [player]);
                    activate(&adapter, foreign);
                }
                5 => {
                    assert_eq!(property(&adapter, b"_NET_ACTIVE_WINDOW"), [foreign]);
                    click_client(&group);
                }
                _ => {
                    assert_eq!(property(&adapter, b"_NET_ACTIVE_WINDOW"), [foreign]);
                    let stack = property(&adapter, b"_NET_CLIENT_LIST_STACKING");
                    let foreign_index = stack.iter().position(|id| *id == foreign).unwrap();
                    for owned in [player, playlist] {
                        assert!(stack.iter().position(|id| *id == owned).unwrap() < foreign_index);
                    }
                    host.borrow_mut().close();
                    slint::quit_event_loop().unwrap();
                }
            }
            step += 1;
        },
    );
    main.run().unwrap();
}

/// Reads one bounded native hint from an explicitly owned test window.
fn window_property(adapter: &Restacker, window: u32, name: &[u8]) -> Vec<u32> {
    let atom = adapter
        .connection
        .intern_atom(false, name)
        .unwrap()
        .reply()
        .unwrap()
        .atom;
    let reply = adapter
        .connection
        .get_property(false, window, atom, AtomEnum::ANY, 0, 64)
        .unwrap()
        .reply()
        .unwrap();
    assert_eq!(reply.bytes_after, 0);
    reply.value32().map_or_else(Vec::new, Iterator::collect)
}

/// Checks WM-acknowledged switcher flags, shared identity and absence of always-on-top state.
fn assert_membership(adapter: &Restacker, owned: &[u32; 3]) {
    let mut skipped = Vec::new();
    for name in [
        b"_NET_WM_STATE_SKIP_TASKBAR".as_slice(),
        b"_NET_WM_STATE_SKIP_PAGER",
        b"_KDE_NET_WM_STATE_SKIP_SWITCHER",
    ] {
        skipped.push(
            adapter
                .connection
                .intern_atom(false, name)
                .unwrap()
                .reply()
                .unwrap()
                .atom,
        );
    }
    let above = adapter
        .connection
        .intern_atom(false, b"_NET_WM_STATE_ABOVE")
        .unwrap()
        .reply()
        .unwrap()
        .atom;
    for (index, window) in owned.iter().enumerate() {
        assert_eq!(
            window_property(adapter, *window, b"WM_CLIENT_LEADER"),
            [owned[0]]
        );
        let hints = x11rb::properties::WmHints::get(&adapter.connection, *window)
            .unwrap()
            .reply()
            .unwrap()
            .unwrap();
        assert_eq!(hints.window_group, Some(owned[0]));
        let states = window_property(adapter, *window, b"_NET_WM_STATE");
        for skip in &skipped {
            assert_eq!(
                states.contains(skip),
                index != 0,
                "switcher/taskbar member={index} expected={skipped:?} states={states:?}"
            );
        }
        assert!(!states.contains(&above));
    }
}

/// One switcher entry restores the group above a foreign application, including after hide/remap/minimize.
#[test]
#[ignore = "requires isolated KWin/XWayland; run through check_window_snap.py"]
fn native_task_switcher_journey() {
    crate::window_snap::select_x11().unwrap();
    let main = PlayerWindow::new().unwrap();
    let group = SharedGroup::default();
    let playlist = crate::playlist_window::connect(&main, Default::default(), group.clone());
    let themes = crate::themes::connect(&main, group);
    main.show().unwrap();
    playlist.borrow_mut().show(&main, &playlist).unwrap();
    main.invoke_open_themes();
    let queue = playlist.borrow().test_window();
    let chooser = themes.borrow().test_window();
    let outsider = ForeignWindow::start();
    let adapter = Restacker::connect().unwrap();
    let timer = slint::Timer::default();
    let weak = main.as_weak();
    let mut step = 0;
    let mut attempts = 0;
    timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_millis(250),
        move || {
            attempts += 1;
            assert!(attempts < 40, "switcher journey deadline");
            let Some(foreign) = outsider.id(&adapter) else {
                return;
            };
            let main = weak.upgrade().unwrap();
            let owned = [id(main.window()), id(queue.window()), id(chooser.window())];
            // WM state changes are asynchronous; initial native mapping/rendering may consume
            // the first timer frame before KWin has processed the application's messages.
            if step == 0
                && owned[1..]
                    .iter()
                    .any(|window| window_property(&adapter, *window, b"_NET_WM_STATE").len() < 3)
            {
                return;
            }
            match step {
                0 | 6 => {
                    assert_membership(&adapter, &owned);
                    assert!(
                        window_property(&adapter, foreign, b"WM_CLIENT_LEADER") != [owned[0]],
                        "foreign application must stay independent"
                    );
                    activate(&adapter, foreign);
                }
                1 | 7 => {
                    assert_eq!(property(&adapter, b"_NET_ACTIVE_WINDOW"), [foreign]);
                    activate(&adapter, owned[0]);
                }
                2 | 8 | 11 => {
                    assert_eq!(property(&adapter, b"_NET_ACTIVE_WINDOW"), [owned[0]]);
                    let stack = property(&adapter, b"_NET_CLIENT_LIST_STACKING");
                    let foreign_index = stack.iter().position(|id| *id == foreign).unwrap();
                    for window in owned {
                        assert!(stack.iter().position(|id| *id == window).unwrap() > foreign_index);
                    }
                    if step == 2 {
                        main.invoke_toggle_playlist();
                        chooser.hide().unwrap();
                    } else if step == 8 {
                        main.window().set_minimized(true);
                    } else {
                        assert_membership(&adapter, &owned);
                        playlist.borrow_mut().close();
                        themes.borrow_mut().close();
                        slint::quit_event_loop().unwrap();
                    }
                }
                3 | 9 => activate(&adapter, foreign),
                4 | 10 => activate(&adapter, owned[0]),
                5 => {
                    assert!(!queue.window().is_visible());
                    assert!(!chooser.window().is_visible());
                    main.invoke_toggle_playlist();
                    main.invoke_open_themes();
                }
                _ => unreachable!(),
            }
            step += 1;
        },
    );
    main.run().unwrap();
}
