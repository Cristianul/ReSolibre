//! Visual controls retain the same shared actions, capture rules and authoritative state.

use crate::PlayerWindow;
use crate::ui_tests::{click, pointer, send_key, slider_point};
use i_slint_backend_testing::ElementHandle;
use slint::ComponentHandle;
use std::cell::RefCell;
use std::rc::Rc;

/// The passive selector face preserves pointer opening, keyboard selection and disabled guards.
#[test]
fn styled_dropdown_preserves_toolkit_selection() {
    i_slint_backend_testing::init_no_event_loop();
    let window = PlayerWindow::new().unwrap();
    crate::equalizer::initialize(&window);
    window.set_eq_store_busy(false);
    window.show().unwrap();

    let selections = Rc::new(RefCell::new(Vec::new()));
    let observed = Rc::clone(&selections);
    window.on_eq_load(move |index| observed.borrow_mut().push(index));
    let selector = ElementHandle::find_by_accessible_label(&window, "Load EQ preset")
        .next()
        .unwrap();
    selector.mock_single_click(slint::platform::PointerEventButton::Left);
    assert_eq!(selector.accessible_expanded(), Some(true));
    send_key(&window, slint::platform::Key::DownArrow);
    send_key(&window, slint::platform::Key::Escape);
    assert_eq!(selector.accessible_expanded(), Some(false));
    assert_eq!(*selections.borrow(), vec![1]);
    assert_eq!(window.get_eq_preset_index(), 1);

    window.set_eq_store_busy(true);
    selector.mock_single_click(slint::platform::PointerEventButton::Left);
    send_key(&window, slint::platform::Key::DownArrow);
    assert_eq!(*selections.borrow(), vec![1]);
}

/// Two-axis rotary dragging is relative and bounded; release/cancel/disabled controls stop edits.
#[test]
fn rotary_volume_preserves_gain_on_press_and_clamps_drag() {
    i_slint_backend_testing::init_no_event_loop();
    let window = PlayerWindow::new().unwrap();
    window.set_volume(37.0);
    window.show().unwrap();
    let values = Rc::new(RefCell::new(Vec::new()));
    let observed = values.clone();
    window.on_change_volume(move |value| observed.borrow_mut().push(value));
    let start = slider_point(&window, "Application volume", 0.8);
    pointer(&window, start, "move");
    pointer(&window, start, "down");
    assert!(values.borrow().is_empty(), "press alone cannot change gain");
    for (delta_x, delta_y, expected) in [
        (35.0, 0.0, 62.0),
        (-35.0, 0.0, 12.0),
        (17.5, -17.5, 62.0),
        (35.0, 35.0, 37.0),
        (1000.0, 0.0, 100.0),
        (-1000.0, 0.0, 0.0),
    ] {
        pointer(
            &window,
            slint::LogicalPosition::new(start.x + delta_x, start.y + delta_y),
            "move",
        );
        assert_eq!(values.borrow().last(), Some(&expected));
    }
    pointer(
        &window,
        slint::LogicalPosition::new(start.x, start.y - 35.0),
        "move",
    );
    assert_eq!(values.borrow().last(), Some(&62.0));
    pointer(
        &window,
        slint::LogicalPosition::new(start.x, -500.0),
        "move",
    );
    assert_eq!(values.borrow().last(), Some(&100.0));
    pointer(
        &window,
        slint::LogicalPosition::new(start.x, 1000.0),
        "move",
    );
    assert_eq!(values.borrow().last(), Some(&0.0));
    window
        .window()
        .dispatch_event(slint::platform::WindowEvent::PointerExited);
    pointer(&window, start, "up");
    let count = values.borrow().len();
    pointer(
        &window,
        slint::LogicalPosition::new(start.x + 35.0, start.y),
        "move",
    );
    assert_eq!(values.borrow().len(), count);
    send_key(&window, slint::platform::Key::End);
    assert_eq!(values.borrow().last(), Some(&100.0));
    send_key(&window, slint::platform::Key::Home);
    assert_eq!(values.borrow().last(), Some(&0.0));
    assert_eq!(window.get_volume(), 37.0, "gain remains worker-confirmed");
    window.set_closing(true);
    let count = values.borrow().len();
    let dial = ElementHandle::find_by_accessible_label(&window, "Application volume")
        .next()
        .unwrap();
    dial.set_accessible_value("75");
    pointer(&window, start, "down");
    pointer(
        &window,
        slint::LogicalPosition::new(start.x + 35.0, 0.0),
        "move",
    );
    pointer(&window, start, "up");
    assert_eq!(values.borrow().len(), count);
}

/// Palette controls remain reachable at minimum width and never dispatch playback/EQ commands.
#[test]
fn palettes_preserve_playback_and_lower_panel_state() {
    i_slint_backend_testing::init_no_event_loop();
    let window = PlayerWindow::new().unwrap();
    crate::equalizer::initialize(&window);
    window.set_playing(true);
    window.set_position(103.0);
    window.set_volume(27.0);
    window.set_eq_enabled(true);
    window.set_eq_preamp(-1.0);
    window.on_change_volume(|_| panic!("palette must not change volume"));
    window.on_eq_gain(|_| panic!("palette must not change preamp"));
    window.on_eq_load(|_| panic!("palette must not load an EQ preset"));
    window.show().unwrap();
    window.window().set_size(slint::LogicalSize::new(
        760.0,
        window.invoke_window_height_for_eq(true),
    ));
    let themes = crate::themes::connect(&window, Default::default());
    window.invoke_open_themes();
    let chooser = themes.borrow().test_window();
    for (label, spectrum) in [("Spectrum palette", true), ("Green palette", false)] {
        let button = ElementHandle::find_by_accessible_label(&chooser, label)
            .next()
            .unwrap();
        assert!(button.absolute_position().x + button.size().width <= 480.0);
        button.mock_single_click(slint::platform::PointerEventButton::Left);
        assert_eq!(window.get_spectrum(), spectrum);
        assert!(window.get_eq_expanded());
        assert_eq!(window.get_volume(), 27.0);
        assert_eq!(window.get_position(), 103.0);
        assert_eq!(window.get_eq_preamp(), -1.0);
        assert!(window.get_playing());
        assert!(window.get_eq_enabled());
    }
}

/// Custom chrome uses the existing close-request path, respects cancellation and remains keyboard-operable.
#[test]
fn window_chrome_preserves_close_lifecycle_and_square_artwork() {
    i_slint_backend_testing::init_no_event_loop();
    let window = PlayerWindow::new().unwrap();
    window.show().unwrap();
    let requests = Rc::new(std::cell::Cell::new(0));
    let observed = requests.clone();
    window.window().on_close_requested(move || {
        observed.set(observed.get() + 1);
        slint::CloseRequestResponse::KeepWindowShown
    });
    for width in [760.0, 860.0, 1200.0] {
        window.window().set_size(slint::LogicalSize::new(
            width,
            window.invoke_window_height_for_eq(true),
        ));
        let artwork = ElementHandle::find_by_accessible_label(&window, "Collection illustration")
            .next()
            .unwrap();
        assert_eq!(artwork.size().width, artwork.size().height);
        for label in ["Minimize window", "Maximize window", "Close player"] {
            let button = ElementHandle::find_by_accessible_label(&window, label)
                .next()
                .unwrap();
            assert!(button.absolute_position().x >= 6.0);
            assert!(button.absolute_position().x + button.size().width <= width - 6.0);
        }
    }
    click(&window, "Maximize window");
    assert!(window.window().is_maximized());
    click(&window, "Restore window");
    assert!(!window.window().is_maximized());
    click(&window, "Minimize window");
    assert!(window.window().is_minimized());
    window.window().set_minimized(false);
    let close = ElementHandle::find_by_accessible_label(&window, "Close player")
        .next()
        .unwrap();
    close.mock_single_click(slint::platform::PointerEventButton::Left);
    assert_eq!(requests.get(), 1);
    assert!(
        window.window().is_visible(),
        "close must allow the existing shutdown to finish"
    );
    send_key(&window, slint::platform::Key::Return);
    assert_eq!(requests.get(), 2);
    window.set_closing(true);
    close.invoke_accessible_default_action();
    close.mock_single_click(slint::platform::PointerEventButton::Left);
    assert_eq!(
        requests.get(),
        2,
        "closing disables repeated custom requests"
    );
}

/// Short clicks use the rendered arc continuously; holds, drags, outside points and disabled input do not glide.
#[test]
fn rotary_click_maps_angles_and_rejects_other_gestures() {
    use std::time::Duration;
    i_slint_backend_testing::init_no_event_loop();
    let window = PlayerWindow::new().unwrap();
    window.set_volume(37.0);
    window.show().unwrap();
    let values = Rc::new(RefCell::new(Vec::new()));
    let observed = values.clone();
    window.on_glide_volume(move |value| observed.borrow_mut().push(value));
    let manual = Rc::new(RefCell::new(Vec::new()));
    let observed = manual.clone();
    window.on_change_volume(move |value| observed.borrow_mut().push(value));
    let dial = ElementHandle::find_by_accessible_label(&window, "Application volume")
        .next()
        .unwrap();
    let origin = dial.absolute_position();
    let center = slint::LogicalPosition::new(
        origin.x + dial.size().width / 2.0,
        origin.y + dial.size().height / 2.0,
    );
    // Include fractional angles: targets must not snap to quadrants or whole percentages.
    for percent in [0.0_f32, 12.345, 16.6667, 50.0, 66.6667, 83.3333, 100.0] {
        let angle = (-225.0 + percent * 2.7).to_radians();
        let point = slint::LogicalPosition::new(
            center.x + 30.0 * angle.cos(),
            center.y + 30.0 * angle.sin(),
        );
        i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
        pointer(&window, point, "move");
        pointer(&window, point, "down");
        assert!(manual.borrow().is_empty());
        i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(100));
        pointer(&window, point, "up");
        assert!((values.borrow().last().unwrap() - percent).abs() < 0.001);
    }
    let right = slint::LogicalPosition::new(center.x + 30.0, center.y);
    for duration in [1001, 2000] {
        i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
        let count = values.borrow().len();
        pointer(&window, right, "move");
        pointer(&window, right, "down");
        i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(duration));
        pointer(&window, right, "up");
        assert_eq!(values.borrow().len(), count);
        assert!(
            manual.borrow().is_empty(),
            "stationary hold must do nothing"
        );
    }
    for point in [
        center,
        slint::LogicalPosition::new(origin.x + 1.0, origin.y + 1.0),
    ] {
        let count = values.borrow().len();
        i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
        pointer(&window, point, "move");
        pointer(&window, point, "down");
        pointer(&window, point, "up");
        assert_eq!(values.borrow().len(), count);
    }
    i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
    let count = values.borrow().len();
    pointer(&window, right, "move");
    pointer(&window, right, "down");
    pointer(
        &window,
        slint::LogicalPosition::new(right.x - 14.0, right.y),
        "move",
    );
    pointer(&window, right, "move");
    pointer(&window, right, "up");
    assert_eq!(
        values.borrow().len(),
        count,
        "returning a drag to its origin is not a click"
    );
    assert!(!manual.borrow().is_empty());
    i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
    pointer(&window, right, "down");
    window
        .window()
        .dispatch_event(slint::platform::WindowEvent::PointerExited);
    pointer(&window, right, "up");
    assert_eq!(values.borrow().len(), count, "cancel cannot glide");
    i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
    pointer(&window, right, "move");
    pointer(&window, right, "down");
    window.set_closing(true);
    pointer(&window, right, "up");
    assert_eq!(values.borrow().len(), count);
}

/// Reset wins over the first click's glide; right-button presses never change rotary volume.
#[test]
fn rotary_double_click_resets_and_right_click_is_inert() {
    use slint::platform::{PointerEventButton, WindowEvent};
    use std::time::Duration;
    i_slint_backend_testing::init_no_event_loop();
    let window = PlayerWindow::new().unwrap();
    window.set_volume(37.0);
    window.show().unwrap();
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    window.on_glide_volume(move |value| observed.borrow_mut().push(("glide", value)));
    let observed = events.clone();
    window.on_change_volume(move |value| observed.borrow_mut().push(("manual", value)));
    let point = slider_point(&window, "Application volume", 0.8);
    pointer(&window, point, "move");
    for _ in 0..2 {
        pointer(&window, point, "down");
        i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(50));
        pointer(&window, point, "up");
    }
    assert_eq!(events.borrow().first().unwrap().0, "glide");
    assert_eq!(events.borrow().last(), Some(&("manual", 50.0)));
    let count = events.borrow().len();
    i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
    for _ in 0..2 {
        window.window().dispatch_event(WindowEvent::PointerPressed {
            position: point,
            button: PointerEventButton::Right,
        });
        i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(50));
        window
            .window()
            .dispatch_event(WindowEvent::PointerReleased {
                position: point,
                button: PointerEventButton::Right,
            });
    }
    assert_eq!(events.borrow().len(), count);
}
