//! Gesture math, shared chrome routing and native group movement regressions.

use super::*;

/// Slow one-pixel movement and reversals preserve every member's exact offset; overflow moves nobody.
#[test]
fn one_pixel_moves_and_overflow() {
    let frame = Rect {
        x: -500,
        y: 100,
        width: 300,
        height: 200,
    };
    let mut drag = GroupDrag {
        pointer: (40, 50),
        last_pointer: (40, 50),
        members: vec![(0, frame), (1, Rect { x: -200, ..frame })],
    };
    for delta in (-100..100).chain((-100..100).rev()) {
        let targets = drag.targets((40 + delta, 50 - delta)).unwrap();
        assert_eq!(targets[0].1.x, -500 + delta);
        assert_eq!(targets[0].1.y, 100 - delta);
        assert_eq!(targets[1].1.x - targets[0].1.x, 300);
    }
    drag.members[1].1.x = i32::MAX;
    assert!(drag.targets((41, 50)).is_none());
}

/// Activating the player may blur Library after press; that blur must preserve the captured group.
#[test]
fn follower_focus_loss_does_not_cancel_player_drag() {
    use slint::winit_030::winit::event::WindowEvent;

    let group = SnapGroup {
        player: Some(0),
        ..Default::default()
    };
    let frame = Rect {
        x: 100,
        y: 100,
        width: 400,
        height: 300,
    };
    *group.drag.borrow_mut() = Some(GroupDrag {
        pointer: (150, 150),
        last_pointer: (150, 150),
        members: vec![(0, frame), (1, Rect { x: 500, ..frame })],
    });

    group.handle_drag_interruption(1, &WindowEvent::Focused(false));
    assert!(
        group.drag.borrow().is_some(),
        "Library blur follows the same press"
    );
    group.handle_drag_interruption(0, &WindowEvent::Focused(true));
    assert!(group.drag.borrow().is_some());

    group.handle_drag_interruption(0, &WindowEvent::Focused(false));
    assert!(
        group.drag.borrow().is_none(),
        "player blur still cancels drag"
    );

    *group.drag.borrow_mut() = Some(GroupDrag {
        pointer: (150, 150),
        last_pointer: (150, 150),
        members: vec![(0, frame), (1, Rect { x: 500, ..frame })],
    });
    group.handle_drag_interruption(1, &WindowEvent::Destroyed);
    assert!(
        group.drag.borrow().is_none(),
        "a removed follower cancels drag"
    );
}

/// The real title region delegates only accepted group gestures and releases capture cleanly.
#[test]
fn header_routes_group_gestures() {
    use slint::platform::{PointerEventButton, WindowEvent};
    i_slint_backend_testing::init_no_event_loop();
    let main = crate::PlayerWindow::new().unwrap();
    main.show().unwrap();
    main.window().set_size(slint::LogicalSize::new(800., 516.));
    let counts = Rc::new(RefCell::new([0; 3]));
    let seen = counts.clone();
    main.on_begin_group_move(move || {
        seen.borrow_mut()[0] += 1;
        true
    });
    let seen = counts.clone();
    main.on_update_group_move(move || {
        seen.borrow_mut()[1] += 1;
    });
    let seen = counts.clone();
    main.on_end_group_move(move || {
        seen.borrow_mut()[2] += 1;
    });
    let start = slint::LogicalPosition::new(100., 25.);
    let end = slint::LogicalPosition::new(120., 25.);
    main.window().dispatch_event(WindowEvent::PointerPressed {
        position: start,
        button: PointerEventButton::Left,
    });
    main.window()
        .dispatch_event(WindowEvent::PointerMoved { position: end });
    main.window().dispatch_event(WindowEvent::PointerReleased {
        position: end,
        button: PointerEventButton::Left,
    });
    assert_eq!(counts.borrow()[0], 1);
    assert!(counts.borrow()[1] > 0);
    assert_eq!(counts.borrow()[2], 1);
    let after_release = *counts.borrow();
    main.window()
        .dispatch_event(WindowEvent::PointerMoved { position: start });
    assert_eq!(*counts.borrow(), after_release);
    main.on_begin_group_move(|| false);
    main.window().dispatch_event(WindowEvent::PointerPressed {
        position: start,
        button: PointerEventButton::Left,
    });
    main.window().dispatch_event(WindowEvent::PointerReleased {
        position: start,
        button: PointerEventButton::Left,
    });
    assert_eq!(*counts.borrow(), after_release);
}

/// Passive cover, readout and chassis drags share group movement while active buttons keep clicks.
#[test]
fn passive_player_surfaces_drag_without_stealing_controls() {
    use i_slint_backend_testing::ElementHandle;
    use slint::platform::{PointerEventButton, WindowEvent};

    i_slint_backend_testing::init_no_event_loop();
    let main = crate::PlayerWindow::new().unwrap();
    main.set_can_play(true);
    main.set_analysis_enabled(true);
    main.show().unwrap();
    main.window()
        .set_size(slint::LogicalSize::new(800.0, 516.0));

    let counts = Rc::new(RefCell::new([0; 3]));
    let seen = Rc::clone(&counts);
    main.on_begin_group_move(move || {
        seen.borrow_mut()[0] += 1;
        true
    });
    let seen = Rc::clone(&counts);
    main.on_update_group_move(move || seen.borrow_mut()[1] += 1);
    let seen = Rc::clone(&counts);
    main.on_end_group_move(move || seen.borrow_mut()[2] += 1);

    for (x, y) in [(100.0, 120.0), (300.0, 82.0), (18.0, 300.0)] {
        let start = slint::LogicalPosition::new(x, y);
        let end = slint::LogicalPosition::new(x + 15.0, y + 10.0);
        let before = *counts.borrow();
        main.window().dispatch_event(WindowEvent::PointerPressed {
            position: start,
            button: PointerEventButton::Left,
        });
        main.window()
            .dispatch_event(WindowEvent::PointerMoved { position: end });
        main.window().dispatch_event(WindowEvent::PointerReleased {
            position: end,
            button: PointerEventButton::Left,
        });
        let after = *counts.borrow();
        assert_eq!(after[0], before[0] + 1, "passive point ({x}, {y})");
        assert!(after[1] > before[1]);
        assert_eq!(after[2], before[2] + 1);
    }

    let clicks = Rc::new(RefCell::new(0));
    let seen = Rc::clone(&clicks);
    main.on_play_pause(move || *seen.borrow_mut() += 1);
    let play = ElementHandle::find_by_accessible_label(&main, "Play")
        .next()
        .unwrap();
    let origin = play.absolute_position();
    let center = slint::LogicalPosition::new(
        origin.x + play.size().width / 2.0,
        origin.y + play.size().height / 2.0,
    );
    let before = *counts.borrow();
    main.window().dispatch_event(WindowEvent::PointerPressed {
        position: center,
        button: PointerEventButton::Left,
    });
    main.window().dispatch_event(WindowEvent::PointerReleased {
        position: center,
        button: PointerEventButton::Left,
    });
    assert_eq!(*clicks.borrow(), 1);
    assert_eq!(*counts.borrow(), before);

    main.set_can_seek(true);
    main.set_duration(120.0);
    let seek = ElementHandle::find_by_accessible_label(&main, "Playback position")
        .next()
        .unwrap();
    let origin = seek.absolute_position();
    let start = slint::LogicalPosition::new(
        origin.x + seek.size().width / 2.0,
        origin.y + seek.size().height / 2.0,
    );
    let end = slint::LogicalPosition::new(start.x + 20.0, start.y);
    main.window().dispatch_event(WindowEvent::PointerPressed {
        position: start,
        button: PointerEventButton::Left,
    });
    main.window()
        .dispatch_event(WindowEvent::PointerMoved { position: end });
    main.window().dispatch_event(WindowEvent::PointerReleased {
        position: end,
        button: PointerEventButton::Left,
    });
    assert_eq!(*counts.borrow(), before);

    let analyses = Rc::new(RefCell::new(0));
    let seen = Rc::clone(&analyses);
    main.on_open_analysis(move || *seen.borrow_mut() += 1);
    let button =
        ElementHandle::find_by_accessible_label(&main, "Analyze track: spectrogram and BPM")
            .next()
            .unwrap();
    let origin = button.absolute_position();
    let center = slint::LogicalPosition::new(
        origin.x + button.size().width / 2.0,
        origin.y + button.size().height / 2.0,
    );
    main.window()
        .dispatch_event(WindowEvent::PointerMoved { position: center });
    main.window().dispatch_event(WindowEvent::PointerPressed {
        position: center,
        button: PointerEventButton::Left,
    });
    main.window().dispatch_event(WindowEvent::PointerReleased {
        position: center,
        button: PointerEventButton::Left,
    });
    assert_eq!(*analyses.borrow(), 1);
    assert_eq!(*counts.borrow(), before);

    main.set_closing(true);
    let point = slint::LogicalPosition::new(100.0, 120.0);
    main.window().dispatch_event(WindowEvent::PointerPressed {
        position: point,
        button: PointerEventButton::Left,
    });
    main.window().dispatch_event(WindowEvent::PointerReleased {
        position: point,
        button: PointerEventButton::Left,
    });
    assert_eq!(*counts.borrow(), before);
}

/// Title buttons reveal on click and move the attached group on drag, including incoming MIX focus.
#[test]
fn track_title_click_and_drag_remain_distinct() {
    use i_slint_backend_testing::ElementHandle;
    use slint::platform::{PointerEventButton, WindowEvent};

    i_slint_backend_testing::init_no_event_loop();
    let main = crate::PlayerWindow::new().unwrap();
    main.set_track_title("Outgoing".into());
    main.set_incoming_title("Incoming".into());
    main.show().unwrap();
    main.window()
        .set_size(slint::LogicalSize::new(800.0, 516.0));

    let moves = Rc::new(RefCell::new([0; 3]));
    let seen = Rc::clone(&moves);
    main.on_begin_group_move(move || {
        seen.borrow_mut()[0] += 1;
        true
    });
    let seen = Rc::clone(&moves);
    main.on_update_group_move(move || seen.borrow_mut()[1] += 1);
    let seen = Rc::clone(&moves);
    main.on_end_group_move(move || seen.borrow_mut()[2] += 1);
    let reveals = Rc::new(RefCell::new(0));
    let seen = Rc::clone(&reveals);
    main.on_reveal_current(move || *seen.borrow_mut() += 1);

    for (progress, label, expected_reveals) in [
        (0.0, "Show current track in library", 1),
        (1.0, "Show incoming track in library", 2),
    ] {
        main.set_title_mix_progress(progress);
        let button = ElementHandle::find_by_accessible_label(&main, label)
            .next()
            .unwrap();
        let origin = button.absolute_position();
        let start = slint::LogicalPosition::new(
            origin.x + button.size().width / 2.0,
            origin.y + button.size().height / 2.0,
        );
        let end = slint::LogicalPosition::new(start.x + 25.0, start.y);

        main.window().dispatch_event(WindowEvent::PointerPressed {
            position: start,
            button: PointerEventButton::Left,
        });
        main.window().dispatch_event(WindowEvent::PointerReleased {
            position: start,
            button: PointerEventButton::Left,
        });
        assert_eq!(*reveals.borrow(), expected_reveals);

        let before = *moves.borrow();
        main.window().dispatch_event(WindowEvent::PointerPressed {
            position: start,
            button: PointerEventButton::Left,
        });
        main.window()
            .dispatch_event(WindowEvent::PointerMoved { position: end });
        main.window().dispatch_event(WindowEvent::PointerReleased {
            position: end,
            button: PointerEventButton::Left,
        });
        let after = *moves.borrow();
        assert_eq!(*reveals.borrow(), expected_reveals);
        assert_eq!(after[0], before[0] + 1);
        assert!(after[1] > before[1]);
        assert_eq!(after[2], before[2] + 1);
    }

    main.on_begin_group_move(|| false);
    main.set_title_mix_progress(0.0);
    let button = ElementHandle::find_by_accessible_label(&main, "Show current track in library")
        .next()
        .unwrap();
    let origin = button.absolute_position();
    let start = slint::LogicalPosition::new(
        origin.x + button.size().width / 2.0,
        origin.y + button.size().height / 2.0,
    );
    let end = slint::LogicalPosition::new(start.x + 25.0, start.y);
    main.window().dispatch_event(WindowEvent::PointerPressed {
        position: start,
        button: PointerEventButton::Left,
    });
    main.window().dispatch_event(WindowEvent::PointerReleased {
        position: start,
        button: PointerEventButton::Left,
    });
    assert_eq!(*reveals.borrow(), 3);
    let nudge = slint::LogicalPosition::new(start.x + 3.0, start.y);
    main.window().dispatch_event(WindowEvent::PointerPressed {
        position: start,
        button: PointerEventButton::Left,
    });
    main.window()
        .dispatch_event(WindowEvent::PointerMoved { position: nudge });
    main.window().dispatch_event(WindowEvent::PointerReleased {
        position: nudge,
        button: PointerEventButton::Left,
    });
    assert_eq!(*reveals.borrow(), 4, "small pointer motion remains a click");
    main.window().dispatch_event(WindowEvent::PointerPressed {
        position: start,
        button: PointerEventButton::Left,
    });
    main.window()
        .dispatch_event(WindowEvent::PointerMoved { position: end });
    main.window().dispatch_event(WindowEvent::PointerReleased {
        position: end,
        button: PointerEventButton::Left,
    });
    assert_eq!(
        *reveals.borrow(),
        4,
        "native window drag must not reveal Library"
    );
}

/// Actual native frames follow one-pixel requests in both directions; hidden/detached peers cannot drag.
#[cfg(unix)]
#[test]
#[ignore = "requires isolated KWin/XWayland; run through check_window_snap.py"]
fn native_direct_group_drag() {
    use crate::window_snap::native_tests::SnapTestWindow;
    use std::time::Duration;
    select_x11().unwrap();
    let main = SnapTestWindow::new().unwrap();
    let queue = SnapTestWindow::new().unwrap();
    let group = Rc::new(RefCell::new(SnapGroup::default()));
    SnapGroup::watch(
        &group,
        &main,
        Role::Player,
        &crate::window_events::WindowEvents::new(main.window()),
    );
    SnapGroup::watch(
        &group,
        &queue,
        Role::Auxiliary,
        &crate::window_events::WindowEvents::new(queue.window()),
    );
    main.show().unwrap();
    queue.show().unwrap();
    let timer = slint::Timer::default();
    let weak_main = main.as_weak();
    let weak_queue = queue.as_weak();
    let mut step = 0;
    let mut pointer = (0, 0);
    let mut expected: Option<Vec<(usize, Rect)>> = None;
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(100),
        move || {
            let main = weak_main.upgrade().unwrap();
            let queue = weak_queue.upgrade().unwrap();
            let group = group.borrow();
            if step == 3 {
                present(queue.window());
                group.place(main.window(), 300, 300);
                group.place(queue.window(), 300 + main.window().size().width as i32, 300);
            } else if step == 6 {
                assert!(group.begin_drag(main.window()));
                pointer = group.drag.borrow().as_ref().unwrap().pointer;
                present(main.window());
            } else if (7..=47).contains(&step) {
                if step == 7 {
                    assert!(
                        group.drag.borrow().is_some(),
                        "activating the player must preserve the first drag"
                    );
                }
                if let Some(expected) = expected.take() {
                    for (index, target) in expected {
                        let window = if index == 0 {
                            main.window()
                        } else {
                            queue.window()
                        };
                        assert_eq!(group.position(window), Some((target.x, target.y)));
                    }
                }
                if step < 47 {
                    let offset = if step < 27 { step - 6 } else { 46 - step };
                    let next = (pointer.0 + offset, pointer.1 - offset);
                    expected = group.drag.borrow().as_ref().unwrap().targets(next);
                    group.drag_to(next);
                } else {
                    group.end_drag();
                    let before = group.position(main.window());
                    group.drag_to((pointer.0 + 100, pointer.1));
                    assert_eq!(group.position(main.window()), before);
                    assert!(group.begin_drag(main.window()));
                    present(queue.window());
                }
            } else if step == 48 {
                assert!(
                    group.drag.borrow().is_none(),
                    "native focus loss must cancel the gesture"
                );
                queue.hide().unwrap();
                assert!(!group.begin_drag(main.window()));
                slint::quit_event_loop().unwrap();
            }
            step += 1;
        },
    );
    main.run().unwrap();
}
