//! Opt-in real X11/XWayland event-loop verification, with no global input injection.

use super::*;
use std::time::{Duration, Instant};

slint::slint! {
    export component SnapTestWindow inherits Window {
        title: "ReSolibre snap verification";
        in property <bool> frameless: true;
        no-frame: root.frameless;
        resize-border-width: 6px;
        min-width: 220px;
        min-height: 160px;
        preferred-width: 220px;
        preferred-height: 160px;
        Text { text: "ReSolibre · window snap test"; }
    }
}

/// Checks native edge snapping, grouped main moves, follower detachment and reattachment on all sides.
#[test]
#[ignore = "requires an X11/XWayland desktop; run individually"]
fn native_snap_journey() {
    snap_journey(true);
}

/// Exercises native snapping with the production frameless host configuration.
#[test]
#[ignore = "requires isolated KWin/XWayland; run through check_window_snap.py"]
fn native_frameless_snap_journey() {
    snap_journey(false);
}

/// Shares the edge, detachment and burst-movement journey across both native frame styles.
fn snap_journey(decorated: bool) {
    select_x11().unwrap();
    let first = SnapTestWindow::new().unwrap();
    first.set_frameless(!decorated);
    let second = SnapTestWindow::new().unwrap();
    let foreign = SnapTestWindow::new().unwrap();
    let group = Rc::new(RefCell::new(SnapGroup::default()));
    SnapGroup::watch(
        &group,
        &first,
        Role::Player,
        &crate::window_events::WindowEvents::new(first.window()),
    );
    SnapGroup::watch(
        &group,
        &second,
        Role::Auxiliary,
        &crate::window_events::WindowEvents::new(second.window()),
    );
    first.show().unwrap();
    let windows = [first.as_weak(), second.as_weak()];
    let placement = Rc::clone(&group);
    let measurements = x11::Positioner::connect().unwrap();
    let outsider = foreign.as_weak();
    let timer = slint::Timer::default();
    let failure = Rc::new(RefCell::new(None));
    let result = failure.clone();
    let mut started = Instant::now();
    let mut stage = 0;
    let mut expected = None;
    let mut case = 0;
    let mut group_case = 0;
    let mut expected_group = None;
    let mut measured = false;
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(30),
        move || {
            let first = windows[0].upgrade().unwrap();
            let second = windows[1].upgrade().unwrap();
            let foreign = outsider.upgrade().unwrap();
            if stage == 0 {
                present(first.window());
                present(second.window());
                present(foreign.window());
                first
                    .window()
                    .set_position(slint::PhysicalPosition::new(600, 400));
                second
                    .window()
                    .set_position(slint::PhysicalPosition::new(100, 100));
                foreign
                    .window()
                    .set_position(slint::PhysicalPosition::new(200, 650));
                stage = 1;
                started = Instant::now();
                return;
            }
            if stage == 1 {
                present(first.window());
                present(second.window());
                present(foreign.window());
                stage = 2;
                started = Instant::now();
                return;
            }
            if started.elapsed() < Duration::from_millis(250) {
                return;
            }
            if !measured {
                compare_geometry_reads(&measurements, first.window(), second.window());
                measured = true;
            }
            if let Some((wanted_x, wanted_y)) = expected.take() {
                let moving = if case < 8 { &second } else { &first };
                let actual = bounds(&measurements, moving.window()).unwrap();
                if (actual.x, actual.y) != (wanted_x, wanted_y) {
                    *result.borrow_mut() = Some(format!(
                        "case {case}: expected ({wanted_x}, {wanted_y}), observed ({}, {})",
                        actual.x, actual.y
                    ));
                    let _ = slint::quit_event_loop();
                    return;
                }
                case += 1;
            }
            if case == 10 {
                let player = bounds(&measurements, first.window()).unwrap();
                let playlist = bounds(&measurements, second.window()).unwrap();
                if let Some((wanted_player, wanted_playlist)) = expected_group.take() {
                    if (player, playlist) != (wanted_player, wanted_playlist) {
                        *result.borrow_mut() = Some(format!(
                            "group case {group_case}: wanted {wanted_player:?}, {wanted_playlist:?}; observed {player:?}, {playlist:?}"
                        ));
                        let _ = slint::quit_event_loop();
                        return;
                    }
                    group_case += 1;
                }
                if group_case == 16 {
                    let _ = slint::quit_event_loop();
                    return;
                }
                let phase = group_case % 4;
                let mut wanted_player = player;
                let mut wanted_playlist = playlist;
                match phase {
                    0 => {
                        // Reattach on each side, using the same inclusive five-pixel magnet.
                        let (x, y, dx, dy) = match group_case / 4 {
                            0 => (player.x + player.width as i32, player.y, 5, 0),
                            1 => (player.x - playlist.width as i32, player.y, -5, 0),
                            2 => (player.x, player.y + player.height as i32, 0, 5),
                            _ => (player.x, player.y - playlist.height as i32, 0, -5),
                        };
                        wanted_playlist = Rect { x, y, ..playlist };
                        placement.borrow().place(second.window(), x + dx, y + dy);
                    }
                    1 => {
                        wanted_player = geometry::translate(player, 40, -30).unwrap();
                        wanted_playlist = geometry::translate(playlist, 40, -30).unwrap();
                        // Two requests before the follower can acknowledge the first one.
                        placement.borrow().place(first.window(), player.x + 20, player.y - 15);
                        placement.borrow().place(first.window(), wanted_player.x, wanted_player.y);
                    }
                    2 => {
                        wanted_playlist = geometry::translate(playlist, 90, 70).unwrap();
                        placement.borrow().place(second.window(), wanted_playlist.x, wanted_playlist.y);
                    }
                    _ => {
                        wanted_player = geometry::translate(player, -40, 30).unwrap();
                        placement.borrow().place(first.window(), wanted_player.x, wanted_player.y);
                    }
                }
                expected_group = Some((wanted_player, wanted_playlist));
                started = Instant::now();
                return;
            }
            let moving = if case < 8 { &second } else { &first };
            let peer = if case < 8 {
                &first
            } else if case == 8 {
                &second
            } else {
                &foreign
            };
            let moving_rect = bounds(&measurements, moving.window())
                .expect("moving window must be mapped and unminimized");
            let peer_rect = bounds(&measurements, peer.window())
                .expect("peer window must be mapped and unminimized");
            let gap = if case % 2 == 0 || case >= 8 { 5 } else { 6 };
            let side = if case >= 8 { 0 } else { case / 2 };
            // Keep perpendicular edges apart: mixed frame heights can put a 30 px offset
            // within the separate alignment threshold, which this adjacency check excludes.
            let (x, y, snap_x, snap_y) = match side {
                0 => (
                    peer_rect.x + peer_rect.width as i32 + gap,
                    peer_rect.y + 80,
                    peer_rect.x + peer_rect.width as i32,
                    peer_rect.y + 80,
                ),
                1 => (
                    peer_rect.x - moving_rect.width as i32 - gap,
                    peer_rect.y + 80,
                    peer_rect.x - moving_rect.width as i32,
                    peer_rect.y + 80,
                ),
                2 => (
                    peer_rect.x + 80,
                    peer_rect.y + peer_rect.height as i32 + gap,
                    peer_rect.x + 80,
                    peer_rect.y + peer_rect.height as i32,
                ),
                _ => (
                    peer_rect.x + 80,
                    peer_rect.y - moving_rect.height as i32 - gap,
                    peer_rect.x + 80,
                    peer_rect.y - moving_rect.height as i32,
                ),
            };
            expected = Some(if gap == 5 && case != 9 {
                (snap_x, snap_y)
            } else {
                (x, y)
            });
            placement.borrow().place(moving.window(), x, y);
            started = Instant::now();
        },
    );
    first.run().unwrap();
    assert!(failure.borrow().is_none(), "{:?}", failure.borrow());
    first.hide().unwrap();
    second.hide().unwrap();
    foreign.hide().unwrap();
}

/// Compares complete live geometry/eligibility and measures the old serial path against batching.
fn compare_geometry_reads(
    adapter: &x11::Positioner,
    first: &slint::Window,
    second: &slint::Window,
) {
    let identities =
        [first, second].map(|window| window.with_winit_window(x11::identity).flatten());
    let mut serial = Vec::new();
    let mut batched = Vec::new();
    for _ in 0..40 {
        let started = Instant::now();
        let expected = vec![bounds(adapter, first), bounds(adapter, second)];
        serial.push(started.elapsed().as_micros());
        let started = Instant::now();
        let observed = adapter.observe(&identities).unwrap();
        batched.push(started.elapsed().as_micros());
        assert_eq!(observed, expected);
        assert!(observed.iter().all(Option::is_some));
    }
    serial.sort_unstable();
    batched.sort_unstable();
    println!(
        "geometry queries (us): serial p50={} p95={}; batched p50={} p95={}",
        serial[20], serial[38], batched[20], batched[38]
    );
}
