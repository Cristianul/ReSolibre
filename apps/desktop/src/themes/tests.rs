//! Chooser lifecycle and real native rendering use the same production bindings.

use super::*;

/// Opening is lazy/reusable; close preserves selection and shutdown rejects further theme intents.
#[test]
fn chooser_lifecycle_and_shutdown_guards() {
    i_slint_backend_testing::init_no_event_loop();
    let main = PlayerWindow::new().unwrap();
    main.show().unwrap();
    let host = connect(&main, Default::default());
    assert!(host.borrow().window.is_none());
    crate::ui_tests::click(&main, "Application menu");
    crate::ui_tests::click(&main, "Theme submenu");
    crate::ui_tests::click(&main, "Spectrum palette");
    assert!(main.get_spectrum());
    assert!(!main.get_app_menu_open());
    main.invoke_open_themes();
    let chooser = host.borrow().test_window();
    assert!(chooser.window().is_visible());
    crate::ui_tests::click(&chooser, "Spectrum palette");
    assert!(main.get_spectrum());
    assert!(chooser.get_spectrum());
    crate::ui_tests::click(&chooser, "Close themes");
    assert!(!chooser.window().is_visible());
    assert!(main.window().is_visible());
    crate::ui_tests::send_key(&main, slint::platform::Key::Return);
    assert!(
        main.get_app_menu_open(),
        "close returns focus to application menu"
    );
    main.set_app_menu_open(false);
    main.invoke_open_themes();
    assert!(chooser.get_spectrum());
    main.invoke_select_theme(false);
    assert!(!chooser.get_spectrum(), "all callers share selection state");
    main.set_closing(true);
    chooser.invoke_choose_theme(true);
    assert!(!main.get_spectrum());
    host.borrow_mut().close();
    assert!(!chooser.window().is_visible());
    main.invoke_open_themes();
    assert!(
        host.borrow().window.is_none(),
        "shutdown cannot reopen the chooser"
    );
}

/// Native chooser rendering, immediate choice, close and reopen work on both display backends.
#[test]
#[ignore = "requires isolated native display and RESOUL_PREVIEW_DIR"]
fn native_theme_preview() {
    let wayland = std::env::var_os("RESOUL_PREVIEW_WAYLAND").is_some();
    if !wayland {
        crate::window_snap::select_x11().unwrap();
    }
    let directory = std::path::PathBuf::from(std::env::var_os("RESOUL_PREVIEW_DIR").unwrap());
    let main = PlayerWindow::new().unwrap();
    let host = connect(&main, Default::default());
    let weak = main.as_weak();
    let owner = host.clone();
    let timer = slint::Timer::default();
    let mut step = 0;
    timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_millis(200),
        move || {
            let main = weak.upgrade().unwrap();
            match step {
                0 => main.invoke_open_themes(),
                1 => crate::ui_tests::click(&owner.borrow().test_window(), "Spectrum palette"),
                2 => {
                    assert!(main.get_spectrum());
                    let chooser = owner.borrow().test_window();
                    let pixels = chooser.window().take_snapshot().unwrap();
                    assert_eq!(pixels.as_bytes()[3], 0);
                    image::save_buffer(
                        directory.join(if wayland {
                            "themes-wayland.png"
                        } else {
                            "themes.png"
                        }),
                        pixels.as_bytes(),
                        pixels.width(),
                        pixels.height(),
                        image::ColorType::Rgba8,
                    )
                    .unwrap();
                    crate::ui_tests::click(&chooser, "Close themes");
                    assert!(!chooser.window().is_visible());
                    assert!(main.window().is_visible());
                }
                3 => main.invoke_open_themes(),
                _ => {
                    assert!(owner.borrow().test_window().get_spectrum());
                    owner.borrow_mut().close();
                    slint::quit_event_loop().unwrap();
                }
            }
            step += 1;
        },
    );
    main.run().unwrap();
}

/// Captures both submenu and destructive confirmation through real menu clicks without deleting data.
#[test]
#[ignore = "requires isolated native display and RESOUL_PREVIEW_DIR"]
fn native_app_menu_preview() {
    crate::window_snap::select_x11().unwrap();
    let directory = std::path::PathBuf::from(std::env::var_os("RESOUL_PREVIEW_DIR").unwrap());
    let main = PlayerWindow::new().unwrap();
    let _host = connect(&main, Default::default());
    let weak = main.as_weak();
    let timer = slint::Timer::default();
    let mut step = 0;
    timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_millis(200),
        move || {
            let main = weak.upgrade().unwrap();
            match step {
                0 => crate::ui_tests::click(&main, "Application menu"),
                1 => crate::ui_tests::click(&main, "Theme submenu"),
                2 | 6 => {
                    let pixels = main.window().take_snapshot().unwrap();
                    image::save_buffer(
                        directory.join(if step == 2 {
                            "app-menu.png"
                        } else {
                            "reset-dialog.png"
                        }),
                        pixels.as_bytes(),
                        pixels.width(),
                        pixels.height(),
                        image::ColorType::Rgba8,
                    )
                    .unwrap();
                }
                3 => crate::ui_tests::click(&main, "Spectrum palette"),
                4 => crate::ui_tests::click(&main, "Application menu"),
                5 => crate::ui_tests::click(&main, "Reset application data"),
                _ => {
                    crate::ui_tests::click(&main, "Cancel reset");
                    assert!(!main.get_reset_confirm());
                    assert!(main.get_spectrum());
                    slint::quit_event_loop().unwrap();
                }
            }
            step += 1;
        },
    );
    main.run().unwrap();
}
