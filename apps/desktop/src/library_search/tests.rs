use super::*;
use crate::ui_tests::{click, pointer};
use i_slint_backend_testing::ElementHandle;
use slint::ComponentHandle;
use std::cell::RefCell;
use std::rc::Rc;

/// Supplies duplicated titles, searchable hidden tags and occurrence IDs beyond floating-point precision.
fn tracks() -> ModelRc<TrackRow> {
    ModelRc::new(VecModel::from(vec![
        TrackRow {
            duration: "--:--".into(),
            id: "9007199254740993".into(),
            number: "1".into(),
            title: "Same".into(),
            search_text: "first.flac\nartist one\nnight album\n2024\n2/3\n音楽".into(),
        },
        TrackRow {
            duration: "--:--".into(),
            id: "9007199254740994".into(),
            number: "2".into(),
            title: "Same".into(),
            search_text: "second.mp3\nartist two\nday album\n1999\n1/2".into(),
        },
        TrackRow {
            duration: "--:--".into(),
            id: "9007199254740995".into(),
            number: "3".into(),
            title: "ÉTÉ [Live]".into(),
            search_text: "third.wav\nnight album\n2024".into(),
        },
    ]))
}

/// Search terms combine across fields, remain literal, preserve order and handle bounded Unicode safely.
#[test]
fn matching_is_literal_bounded_and_preserves_occurrences() {
    let rows = tracks();
    for query in ["ARTIST 2024", "night 2/3", "音楽", "first.flac"] {
        let result = filter_tracks(rows.clone(), query.into());
        assert_eq!(result.row_count(), 1);
        assert_eq!(result.row_data(0).unwrap().id, "9007199254740993");
    }
    assert_eq!(
        filter_tracks(rows.clone(), "Été [live]".into()).row_count(),
        1
    );
    assert_eq!(filter_tracks(rows.clone(), "  \t  ".into()).row_count(), 3);
    for query in [".*", "artist 1970", "http://secret", "absent"] {
        assert_eq!(filter_tracks(rows.clone(), query.into()).row_count(), 0);
    }
    assert_eq!(
        filter_tracks(rows.clone(), "night".into())
            .row_data(1)
            .unwrap()
            .number,
        "3"
    );
    assert_eq!(rows.row_count(), 3);
    assert_eq!(limit_query("音".repeat(10000).into()).chars().count(), 256);
}

/// Exercises the real editor, identity-based playback, late metadata, clear and current-track reveal.
#[test]
fn editor_filters_without_editing_playback_and_reveal_clears_it() {
    i_slint_backend_testing::init_no_event_loop();
    let window = crate::playlist_window::new_window().unwrap();
    window.set_tracks(tracks());
    window.set_current(1);
    window.set_playing(true);
    window.show().unwrap();
    let played = Rc::new(RefCell::new(Vec::new()));
    let observed = played.clone();
    window.on_select_track(move |id| observed.borrow_mut().push(id.to_string()));
    window.on_remove_track(|_| panic!("filtering must not delete"));
    window.on_reorder_track(|_, _, _| panic!("filtering must not reorder"));
    click(&window, "Toggle library search");
    assert!(window.get_search_open());
    for character in "night".chars() {
        window
            .window()
            .dispatch_event(slint::platform::WindowEvent::KeyPressed {
                text: character.to_string().into(),
            });
        window
            .window()
            .dispatch_event(slint::platform::WindowEvent::KeyReleased {
                text: character.to_string().into(),
            });
    }
    assert_eq!(window.get_search_query(), "night");
    assert_eq!(window.get_visible_tracks().row_count(), 2);
    assert_eq!(window.get_visible_current(), -1);
    assert_eq!(window.get_current(), 1);
    assert!(window.get_playing());
    assert!(played.borrow().is_empty());
    let row = ElementHandle::find_by_accessible_label(&window, "3 ÉTÉ [Live]")
        .next()
        .unwrap();
    let position = row.absolute_position();
    let point = slint::LogicalPosition::new(position.x + 60.0, position.y + 15.0);
    pointer(&window, point, "down");
    pointer(&window, point, "up");
    window
        .window()
        .dispatch_event(slint::platform::WindowEvent::KeyPressed {
            text: slint::platform::Key::Return.into(),
        });
    assert_eq!(*played.borrow(), ["9007199254740995"]);
    window.set_search_query("not loaded yet".into());
    assert_eq!(window.get_visible_tracks().row_count(), 0);
    let mut updated: Vec<_> = tracks().iter().collect();
    updated[1].search_text = "not loaded yet".into();
    window.set_tracks(ModelRc::new(VecModel::from(updated)));
    assert_eq!(window.get_visible_tracks().row_count(), 1);
    assert_eq!(window.get_visible_current(), 0);
    window.invoke_reveal_current();
    assert_eq!(window.get_search_query(), "");
    assert_eq!(window.get_selected_id(), "9007199254740994");
    assert_eq!(window.get_visible_current(), 1);
    window.set_search_query("none".into());
    click(&window, "Clear search");
    assert_eq!(window.get_visible_tracks().row_count(), 3);
    click(&window, "Toggle library search");
    assert!(!window.get_search_open());
    assert_eq!(window.get_tracks().row_count(), 3);
}

/// Filtered rows still delete/reorder the correct occurrences; a changed query cancels captured drags.
#[test]
fn filtered_row_gestures_use_full_queue_identities() {
    i_slint_backend_testing::init_no_event_loop();
    let window = crate::playlist_window::new_window().unwrap();
    window.set_tracks(tracks());
    window.set_search_query("night".into());
    window.show().unwrap();
    let actions = Rc::new(RefCell::new(Vec::new()));
    let observed = actions.clone();
    window.on_reorder_track(move |id, target, after| {
        observed.borrow_mut().push(format!("{id}:{target}:{after}"))
    });
    let observed = actions.clone();
    window.on_remove_track(move |id| observed.borrow_mut().push(format!("delete:{id}")));
    let first = ElementHandle::find_by_accessible_label(&window, "1 Same")
        .next()
        .unwrap()
        .absolute_position();
    let last = ElementHandle::find_by_accessible_label(&window, "3 ÉTÉ [Live]")
        .next()
        .unwrap()
        .absolute_position();
    let from = slint::LogicalPosition::new(first.x + 60.0, first.y + 15.0);
    let to = slint::LogicalPosition::new(last.x + 60.0, last.y + 30.0);
    pointer(&window, from, "down");
    pointer(&window, to, "move");
    pointer(&window, to, "up");
    assert_eq!(
        *actions.borrow(),
        ["9007199254740993:9007199254740995:true"]
    );
    actions.borrow_mut().clear();
    pointer(&window, from, "down");
    pointer(&window, to, "move");
    window.set_search_query("absent".into());
    pointer(&window, to, "up");
    assert!(actions.borrow().is_empty());
    window.set_search_query("night".into());
    window
        .window()
        .dispatch_event(slint::platform::WindowEvent::PointerPressed {
            position: to,
            button: slint::platform::PointerEventButton::Right,
        });
    window
        .window()
        .dispatch_event(slint::platform::WindowEvent::PointerReleased {
            position: to,
            button: slint::platform::PointerEventButton::Right,
        });
    click(&window, "Delete from library");
    assert_eq!(*actions.borrow(), ["delete:9007199254740995"]);
    click(&window, "Toggle library search");
    window
        .window()
        .dispatch_event(slint::platform::WindowEvent::KeyPressed {
            text: slint::platform::Key::Escape.into(),
        });
    assert!(!window.get_search_open());
    assert_eq!(window.get_search_query(), "");
}
