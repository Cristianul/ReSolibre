//! Compiled title motion checks exercise shared rendering and real Library/player consumers.

use crate::ui_tests::pointer;
use crate::{PlayerWindow, PlaylistWindow, TrackRow};
use i_slint_backend_testing::{ElementHandle, mock_elapsed_time};
use slint::{ComponentHandle, ModelRc, VecModel};
use std::time::Duration;

const LONG_TITLE: &str = "An artist with a long name — A very long title with Unicode 音楽 and a live recording at midnight";

/// Finds the full accessible row and its moving text, independent of clipping or window chrome.
fn row_title(window: &PlaylistWindow) -> (ElementHandle, ElementHandle) {
    let row = ElementHandle::find_by_accessible_label(window, &format!("1 {LONG_TITLE}"))
        .next()
        .unwrap();
    let text = row
        .query_descendants()
        .match_id("ScrollingText::label")
        .find_first()
        .unwrap();
    (row, text)
}

/// Advances toolkit time after bindings have evaluated, allowing start-pause and travel to run.
fn advance(milliseconds: u64) {
    mock_elapsed_time(Duration::from_millis(milliseconds));
}

/// Only interested overflowing rows move; hover/selection/playback remain independent of queue edits.
#[test]
fn library_titles_scroll_on_hover_selection_or_play() {
    i_slint_backend_testing::init_no_event_loop();
    let window = crate::playlist_window::new_window().unwrap();
    window.set_tracks(ModelRc::new(VecModel::from(vec![TrackRow {
        id: "1".into(),
        number: "1".into(),
        title: LONG_TITLE.into(),
        search_text: Default::default(),
        duration: "03:45".into(),
    }])));
    window.on_select_track(|_| panic!("hover/selection must not play"));
    window.show().unwrap();
    window
        .window()
        .set_size(slint::LogicalSize::new(420.0, 516.0));
    let (row, text) = row_title(&window);
    let start = text.absolute_position().x;
    advance(1500);
    assert_eq!(
        text.absolute_position().x,
        start,
        "unselected idle rows rest"
    );
    let point = row.absolute_position();
    pointer(
        &window,
        slint::LogicalPosition::new(point.x + 80.0, point.y + 18.0),
        "move",
    );
    let _ = text.absolute_position();
    advance(500);
    assert_eq!(text.absolute_position().x, start, "pause before moving");
    advance(1000);
    assert!(
        text.absolute_position().x < start,
        "hover starts slow motion"
    );
    let number = row
        .query_descendants()
        .match_id("QueueRow::number-label")
        .find_first()
        .unwrap();
    let number_x = number.absolute_position().x;
    advance(1000);
    assert_eq!(
        number.absolute_position().x,
        number_x,
        "row number stays fixed"
    );
    pointer(&window, slint::LogicalPosition::new(5.0, 5.0), "move");
    assert_eq!(text.absolute_position().x, start);
    window.set_selected_id("1".into());
    let _ = text.absolute_position();
    advance(1);
    advance(1500);
    assert!(
        text.absolute_position().x < start,
        "selection starts motion"
    );
    window.set_selected_id("".into());
    assert_eq!(text.absolute_position().x, start);
    window.set_current(0);
    window.set_playing(true);
    let _ = text.absolute_position();
    advance(1);
    let playing_start = text.absolute_position().x;
    assert_eq!(
        playing_start,
        start + 24.0,
        "only playing titles make room for the note"
    );
    advance(1);
    advance(1500);
    assert!(
        text.absolute_position().x < playing_start,
        "playing title scrolls"
    );
    window.set_playing(false);
    assert_eq!(text.absolute_position().x, start);
    window.set_selected_id("1".into());
    window.set_motion_enabled(false);
    let _ = text.absolute_position();
    advance(2500);
    assert_eq!(
        text.absolute_position().x,
        start,
        "hidden/reduced-motion host stops motion"
    );
    window.set_motion_enabled(true);
    window
        .window()
        .set_size(slint::LogicalSize::new(2200.0, 516.0));
    let start = text.absolute_position().x;
    advance(2500);
    assert_eq!(
        text.absolute_position().x,
        start,
        "a title that fits needs no motion"
    );
    window
        .window()
        .set_size(slint::LogicalSize::new(420.0, 516.0));
    window.set_tracks(ModelRc::new(VecModel::from(
        (1..=80)
            .map(|index| TrackRow {
                id: index.to_string().into(),
                number: index.to_string().into(),
                title: LONG_TITLE.into(),
                search_text: Default::default(),
                duration: "03:45".into(),
            })
            .collect::<Vec<_>>(),
    )));
    window.set_current(50);
    window.invoke_reveal_current();
    let revealed = ElementHandle::find_by_accessible_label(&window, &format!("51 {LONG_TITLE}"))
        .next()
        .unwrap();
    let revealed_text = revealed
        .query_descendants()
        .match_id("ScrollingText::label")
        .find_first()
        .unwrap();
    let revealed_start = revealed_text.absolute_position().x;
    advance(1);
    advance(1500);
    assert!(
        revealed_text.absolute_position().x < revealed_start,
        "revealed virtualized rows scroll at their viewport position"
    );
    window.hide().unwrap();
}

/// The player uses the same scrolling caption without losing its title-reveal click action.
#[test]
fn player_title_scrolls_resets_and_keeps_reveal_action() {
    i_slint_backend_testing::init_no_event_loop();
    let window = PlayerWindow::new().unwrap();
    window.set_track_title(LONG_TITLE.into());
    window.show().unwrap();
    window
        .window()
        .set_size(slint::LogicalSize::new(800.0, 516.0));
    let button = ElementHandle::find_by_accessible_label(&window, "Show current track in library")
        .next()
        .unwrap();
    let text = button
        .query_descendants()
        .match_type_name("ScrollingText")
        .find_first()
        .unwrap();
    let viewport = text.clone();
    let text = text
        .query_descendants()
        .match_id("ScrollingText::label")
        .find_first()
        .unwrap();
    let start = text.absolute_position().x;
    advance(500);
    assert_eq!(text.absolute_position().x, start);
    advance(1000);
    assert!(text.absolute_position().x < start);
    let overflow = text.size().width - viewport.size().width;
    let travel_ms = (overflow / 24.0 * 1000.0).max(1000.0) as u64;
    advance(travel_ms);
    assert!(
        (text.absolute_position().x - (start - overflow)).abs() < 0.5,
        "the full title reaches its last glyph and pauses"
    );
    advance(travel_ms + 1000);
    assert!(
        (text.absolute_position().x - start).abs() < 0.5,
        "the title returns to its beginning"
    );
    let called = std::rc::Rc::new(std::cell::Cell::new(false));
    let seen = called.clone();
    window.on_reveal_current(move || seen.set(true));
    crate::ui_tests::click(&window, "Show current track in library");
    assert!(called.get());
    window.set_reduced_motion(true);
    assert_eq!(text.absolute_position().x, start);
    advance(2000);
    assert_eq!(text.absolute_position().x, start);
    window.set_reduced_motion(false);
    window.set_track_title("Short title".into());
    assert_eq!(text.absolute_position().x, start);
    advance(2000);
    assert_eq!(
        text.absolute_position().x,
        start,
        "new short content remains static"
    );
    window.hide().unwrap();
}
