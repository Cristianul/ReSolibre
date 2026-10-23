//! Pointer journeys through the compiled virtualized view, with production identity lookup.
use super::*;
use crate::TrackRow;
use crate::ui_tests::{click, pointer};
use i_slint_backend_testing::{ElementHandle, mock_elapsed_time};
use slint::{Model, ModelRc, VecModel};
use std::time::Duration;

/// Builds duplicate-title rows with identities larger than an exact floating-point integer.
fn window() -> PlaylistWindow {
    let window = new_window().unwrap();
    window.set_tracks(ModelRc::new(VecModel::from(
        (0..3)
            .map(|index| TrackRow {
                duration: "--:--".into(),
                search_text: Default::default(),
                id: (9007199254740993_u64 + index).to_string().into(),
                title: "Same.mp3".into(),
                number: (index + 1).to_string().into(),
            })
            .collect::<Vec<_>>(),
    )));
    window.show().unwrap();
    window
}

/// Locates a visible row through its accessible label, independent of window chrome dimensions.
fn row_point(window: &PlaylistWindow, number: usize) -> slint::LogicalPosition {
    let row = ElementHandle::find_by_accessible_label(window, &format!("{number} Same.mp3"))
        .next()
        .unwrap();
    let position = row.absolute_position();
    slint::LogicalPosition::new(position.x + 60.0, position.y + row.size().height / 2.0)
}

/// Sends an ordinary click through the toolkit's pointer recognizer.
fn pointer_click(window: &PlaylistWindow, point: slint::LogicalPosition) {
    pointer(window, point, "move");
    pointer(window, point, "down");
    pointer(window, point, "up");
}

/// Badge pointer/accessibility actions retain stable IDs without changing selection, playback or drag state.
#[test]
fn library_badge_opens_inspection_without_play_selection_or_drag() {
    i_slint_backend_testing::init_no_event_loop();
    let window = window();
    window.set_selected_id("9007199254740993".into());
    window.set_analysis_states(ModelRc::new(VecModel::from(
        (0..3)
            .map(|index| crate::AnalysisRow {
                id: (9007199254740993_u64 + index).to_string().into(),
                status: 3,
            })
            .collect::<Vec<_>>(),
    )));
    window.on_select_track(|_| panic!("badge must not start playback"));
    window.on_reorder_track(|_, _, _| panic!("badge must not reorder"));
    window.on_analyze_track(|_| panic!("badge opens a result, not forced batch analysis"));
    let inspected = Rc::new(RefCell::new(Vec::new()));
    let observed = inspected.clone();
    window.on_view_track_analysis(move |id| observed.borrow_mut().push(id.to_string()));
    let badge = ElementHandle::find_by_accessible_label(&window, "View analysis for Same.mp3")
        .nth(1)
        .unwrap();
    let position = badge.absolute_position();
    let point = slint::LogicalPosition::new(
        position.x + badge.size().width / 2.0,
        position.y + badge.size().height / 2.0,
    );
    pointer_click(&window, point);
    assert_eq!(*inspected.borrow(), ["9007199254740994"]);
    assert_eq!(window.get_selected_id(), "9007199254740993");
    mock_elapsed_time(Duration::from_millis(80));
    pointer_click(&window, point);
    assert_eq!(
        inspected.borrow().len(),
        2,
        "double clicking a badge never plays"
    );
    badge.invoke_accessible_default_action();
    assert_eq!(inspected.borrow().len(), 3);
    pointer(&window, point, "down");
    pointer(
        &window,
        slint::LogicalPosition::new(point.x, point.y + 40.0),
        "move",
    );
    pointer(&window, point, "up");
    assert_eq!(inspected.borrow().len(), 3, "drag away cancels inspection");
    window.set_closing(true);
    pointer_click(&window, point);
    badge.invoke_accessible_default_action();
    assert_eq!(inspected.borrow().len(), 3);
    window.set_closing(false);
    let mut target = window.get_tracks().row_data(1).unwrap();
    target.title = "Target.mp3".into();
    window.set_tracks(ModelRc::new(VecModel::from(vec![target])));
    window.set_search_query("Target".into());
    click(&window, "View analysis for Target.mp3");
    assert_eq!(inspected.borrow().last().unwrap(), "9007199254740994");
}

/// Single/double clicks, cancellation, stable-ID drop and contextual deletion share one view.
#[test]
fn selection_activation_drag_and_context_delete() {
    i_slint_backend_testing::init_no_event_loop();
    let window = window();
    let actions = Rc::new(RefCell::new(Vec::new()));
    let observed = actions.clone();
    window.on_select_track(move |id| observed.borrow_mut().push(format!("play:{id}")));
    let observed = actions.clone();
    window.on_reorder_track(move |id, target, after| {
        observed
            .borrow_mut()
            .push(format!("move:{id}:{target}:{after}"))
    });
    let observed = actions.clone();
    window.on_remove_track(move |id| observed.borrow_mut().push(format!("delete:{id}")));
    let first = row_point(&window, 1);
    let second = row_point(&window, 2);
    let third = row_point(&window, 3);
    pointer_click(&window, second);
    assert_eq!(window.get_selected_id(), "9007199254740994");
    assert!(actions.borrow().is_empty());
    window
        .window()
        .dispatch_event(slint::platform::WindowEvent::KeyPressed {
            text: slint::platform::Key::Return.into(),
        });
    assert_eq!(*actions.borrow(), ["play:9007199254740994"]);
    actions.borrow_mut().clear();
    mock_elapsed_time(Duration::from_millis(600));
    pointer_click(&window, second);
    mock_elapsed_time(Duration::from_millis(80));
    pointer_click(&window, second);
    assert_eq!(*actions.borrow(), ["play:9007199254740994"]);
    actions.borrow_mut().clear();
    mock_elapsed_time(Duration::from_millis(600));
    pointer(&window, first, "down");
    let below = slint::LogicalPosition::new(third.x, third.y + 12.0);
    pointer(&window, below, "move");
    window
        .window()
        .dispatch_event(slint::platform::WindowEvent::KeyPressed {
            text: slint::platform::Key::Escape.into(),
        });
    pointer(&window, below, "up");
    assert!(
        actions.borrow().is_empty(),
        "Escape must not commit a drag or play"
    );
    pointer(&window, first, "down");
    pointer(&window, below, "move");
    pointer(&window, below, "up");
    assert_eq!(
        *actions.borrow(),
        ["move:9007199254740993:9007199254740995:true"]
    );
    actions.borrow_mut().clear();
    window
        .window()
        .dispatch_event(slint::platform::WindowEvent::PointerPressed {
            position: second,
            button: slint::platform::PointerEventButton::Right,
        });
    window
        .window()
        .dispatch_event(slint::platform::WindowEvent::PointerReleased {
            position: second,
            button: slint::platform::PointerEventButton::Right,
        });
    assert!(actions.borrow().is_empty(), "right click never plays");
    click(&window, "Delete from library");
    assert_eq!(*actions.borrow(), ["delete:9007199254740994"]);
    for prefix in ["Move up", "Move down", "Remove from queue"] {
        assert!(
            ElementHandle::find_by_accessible_label(&window, &format!("{prefix} 1 Same.mp3"))
                .next()
                .is_none()
        );
    }
}

/// Closing, model replacement and capture cancellation cannot commit a stale gesture.
#[test]
fn stale_and_disabled_gestures_do_not_edit() {
    i_slint_backend_testing::init_no_event_loop();
    let window = window();
    window.on_reorder_track(|_, _, _| panic!("cancelled drag"));
    window.on_select_track(|_| panic!("cancelled click"));
    let first = row_point(&window, 1);
    let third = row_point(&window, 3);
    pointer(&window, first, "down");
    pointer(&window, third, "move");
    window.invoke_cancel_interaction();
    pointer(&window, third, "up");
    pointer(&window, first, "down");
    let outside = slint::LogicalPosition::new(-20.0, third.y);
    pointer(&window, outside, "move");
    pointer(&window, outside, "up");
    pointer(&window, first, "down");
    pointer(&window, third, "move");
    window.set_tracks(ModelRc::new(VecModel::<TrackRow>::default()));
    mock_elapsed_time(Duration::from_millis(20));
    pointer(&window, third, "up");
    assert_eq!(window.get_selected_id(), "");
    window.set_closing(true);
    pointer_click(&window, first);
}

/// Edge dragging scrolls virtualized rows; revealing current selects it without playback.
#[test]
fn long_queue_autoscroll_and_reveal() {
    i_slint_backend_testing::init_no_event_loop();
    let window = new_window().unwrap();
    window.set_tracks(ModelRc::new(VecModel::from(
        (1..=100)
            .map(|number| TrackRow {
                duration: "--:--".into(),
                search_text: Default::default(),
                id: number.to_string().into(),
                title: "Same.mp3".into(),
                number: number.to_string().into(),
            })
            .collect::<Vec<_>>(),
    )));
    window.on_select_track(|_| panic!("selection must not play"));
    window.on_reorder_track(|_, _, _| panic!("cancel must not reorder"));
    window.show().unwrap();
    let first = row_point(&window, 1);
    let edge = slint::LogicalPosition::new(first.x, window.window().size().height as f32 - 65.0);
    pointer(&window, first, "down");
    pointer(&window, edge, "move");
    for _ in 0..10 {
        mock_elapsed_time(Duration::from_millis(40));
    }
    assert!(window.get_playlist_scroll() < 0.0);
    window.invoke_cancel_interaction();
    pointer(&window, edge, "up");
    window.set_current(99);
    window.invoke_reveal_current();
    assert_eq!(window.get_selected_id(), "100");
    let last = row_point(&window, 100);
    assert!(last.y > first.y && last.y < edge.y + 24.0);
    let scroll = window.get_playlist_scroll();
    window.set_current(0);
    assert_eq!(
        window.get_selected_id(),
        "100",
        "automatic track change preserves browsing selection"
    );
    assert_eq!(window.get_playlist_scroll(), scroll);
    window.invoke_reveal_current();
    assert_eq!(window.get_selected_id(), "1");
    assert_eq!(window.get_playlist_scroll(), 0.0);
}

/// Real FemtoVG snapshots verify rotation (Slint's software renderer does not implement rotate).
#[test]
#[ignore = "requires isolated KWin/XWayland; run through check_window_snap.py"]
fn native_note_animation() {
    crate::window_snap::select_x11().unwrap();
    let window = window();
    window.set_current(0);
    window.set_playing(true);
    let weak = window.as_weak();
    let timer = slint::Timer::default();
    let mut step = 0;
    let mut previous = Vec::new();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(200),
        move || {
            let window = weak.upgrade().unwrap();
            let pixels = window.window().take_snapshot().unwrap();
            match step {
                0 => previous = pixels.as_bytes().to_vec(),
                2 => {
                    assert!(previous != pixels.as_bytes(), "playing marker must animate");
                    window.set_playing(false);
                }
                5 => previous = pixels.as_bytes().to_vec(),
                6 => {
                    assert!(previous == pixels.as_bytes(), "paused marker must rest");
                    window.set_motion_enabled(false);
                    window.set_playing(true);
                }
                7 => previous = pixels.as_bytes().to_vec(),
                8 => {
                    assert!(previous == pixels.as_bytes(), "reduced motion must rest");
                    window.set_spectrum(true);
                }
                9 => {
                    assert!(previous != pixels.as_bytes(), "palette must recolor marker");
                    slint::quit_event_loop().unwrap();
                }
                _ => {}
            }
            step += 1;
        },
    );
    window.run().unwrap();
}

/// Off-track context actions and the whole-library button survive independent badge updates and search.
#[test]
fn context_analysis_badges_and_batch_controls_share_stable_ids() {
    i_slint_backend_testing::init_no_event_loop();
    let window = window();
    window.set_current(0);
    window.on_select_track(|_| panic!("analysis must not start playback"));
    let actions = Rc::new(RefCell::new(Vec::new()));
    let observed = actions.clone();
    window.on_analyze_track(move |id| observed.borrow_mut().push(id.to_string()));
    let observed = actions.clone();
    window.on_analyze_all(move || observed.borrow_mut().push("all".into()));
    let observed = actions.clone();
    window.on_cancel_analysis(move || observed.borrow_mut().push("cancel".into()));
    let point = row_point(&window, 2);
    window
        .window()
        .dispatch_event(slint::platform::WindowEvent::PointerPressed {
            position: point,
            button: slint::platform::PointerEventButton::Right,
        });
    window.set_analysis_states(ModelRc::new(VecModel::from(vec![
        crate::AnalysisRow {
            id: "9007199254740993".into(),
            status: 3,
        },
        crate::AnalysisRow {
            id: "9007199254740994".into(),
            status: 4,
        },
    ])));
    click(&window, "Analyze track");
    assert_eq!(*actions.borrow(), ["9007199254740994"]);
    assert!(
        ElementHandle::find_by_accessible_label(&window, "Analysis complete")
            .next()
            .is_some()
    );
    assert!(
        ElementHandle::find_by_accessible_label(&window, "Analysis failed")
            .next()
            .is_some()
    );
    window.set_search_query("does not match".into());
    assert_eq!(window.get_visible_tracks().row_count(), 0);
    click(&window, "Analyze all library tracks");
    window.set_analysis_busy(true);
    click(&window, "Cancel library analysis");
    assert_eq!(*actions.borrow(), ["9007199254740994", "all", "cancel"]);
}
