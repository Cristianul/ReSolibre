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
                present(main.window());
                group.place(main.window(), 300, 300);
                group.place(queue.window(), 300 + main.window().size().width as i32, 300);
            } else if step == 6 {
                assert!(group.begin_drag(main.window()));
                pointer = group.drag.borrow().as_ref().unwrap().pointer;
            } else if (7..=47).contains(&step) {
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
