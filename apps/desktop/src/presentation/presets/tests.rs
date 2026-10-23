//! Production bindings exercise save/replace/cancel and imported drafts against isolated storage.

use super::*;
use slint::Model;

/// Polls actual session and storage owners until a bounded test condition is satisfied.
fn settle(controller: &Rc<RefCell<Controller>>, window: &PlayerWindow, done: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        controller.borrow_mut().refresh(window);
        if done() {
            break;
        }
        assert!(Instant::now() < deadline, "preset journey deadline");
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Named presets lead the list; replacement needs confirmation and cancelled imports never apply EQ.
#[test]
fn names_confirmation_import_and_restore_use_one_catalog() {
    i_slint_backend_testing::init_no_event_loop();
    let root = std::env::temp_dir().join(format!("resolibre-named-eq-ui-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let window = PlayerWindow::new().unwrap();
    window.show().unwrap();
    let session = Session::start(resolibre_audio::AudioOutput::Silent).unwrap();
    let controller = Rc::new(RefCell::new(Controller::new(session)));
    controller.borrow_mut().start_presets(root.join("settings"));
    crate::presentation::connect(&window, &controller);
    settle(&controller, &window, || !window.get_eq_store_busy());
    window.invoke_eq_load(1); // Built-in Bass before there are any saved entries.
    settle(&controller, &window, || {
        window.get_eq_bands().row_data(0).unwrap().gain == 5.0
    });
    crate::ui_tests::click(&window, "Save EQ preset");
    assert!(window.get_eq_dialog_open());
    window.set_eq_preset_name("Travel".into());
    crate::ui_tests::click(&window, "Confirm preset save");
    settle(&controller, &window, || !window.get_eq_store_busy());
    assert!(!window.get_eq_dialog_open());
    assert_eq!(window.get_eq_presets().row_data(0).unwrap(), "Travel");
    assert_eq!(window.get_eq_presets().row_data(1).unwrap(), "Flat");
    assert_eq!(window.get_eq_preset_index(), 0);
    let file = root.join("settings/eq-presets/preset-1.resolibre-eq");
    let previous = std::fs::read(&file).unwrap();
    window.invoke_eq_gain(-1.5);
    settle(&controller, &window, || {
        window.get_eq_preamp() == -1.5 && !window.get_eq_store_busy()
    });
    assert_eq!(
        window
            .get_eq_presets()
            .row_data(window.get_eq_preset_index() as usize)
            .unwrap(),
        "Custom"
    );
    crate::ui_tests::click(&window, "Save EQ preset");
    window.set_eq_preset_name("travel".into());
    crate::ui_tests::click(&window, "Confirm preset save");
    assert!(window.get_eq_replace_confirm());
    assert_eq!(std::fs::read(&file).unwrap(), previous);
    crate::ui_tests::click(&window, "Cancel preset save");
    assert!(!window.get_eq_dialog_open());
    crate::ui_tests::click(&window, "Save EQ preset");
    window.set_eq_preset_name("Travel".into());
    crate::ui_tests::click(&window, "Confirm preset save");
    crate::ui_tests::click(&window, "Replace named preset");
    settle(&controller, &window, || !window.get_eq_store_busy());
    assert_ne!(std::fs::read(&file).unwrap(), previous);
    assert_eq!(window.get_eq_presets().row_count(), 6); // Saved + four built-ins + Custom.
    window.invoke_eq_load(1); // Flat follows the saved preset.
    settle(&controller, &window, || window.get_eq_preamp() == 0.0);
    window.invoke_eq_load(0);
    settle(&controller, &window, || window.get_eq_preamp() == -1.5);
    let imported = root.join("friend.resolibre-eq");
    let friend = Preset::new("Friend's desk", Equalizer::preset(2).unwrap()).unwrap();
    std::fs::write(&imported, friend.encode()).unwrap();
    controller
        .borrow_mut()
        .presets
        .as_mut()
        .unwrap()
        .import(imported)
        .unwrap();
    settle(&controller, &window, || {
        window.get_eq_dialog_open() && !window.get_eq_store_busy()
    });
    assert_eq!(window.get_eq_preset_name(), "Friend's desk");
    assert_eq!(
        window.get_eq_preamp(),
        -1.5,
        "import does not apply settings"
    );
    crate::ui_tests::click(&window, "Confirm preset save");
    settle(&controller, &window, || !window.get_eq_dialog_open());
    assert_eq!(
        window.get_eq_presets().row_data(0).unwrap(),
        "Friend's desk"
    );
    window.invoke_eq_load(0);
    settle(&controller, &window, || window.get_eq_preamp() == 0.0);
    assert_eq!(window.get_eq_bands().row_data(0).unwrap().gain, -2.0);
    drop(controller);
    window.hide().unwrap();
    drop(window);
    let mut restored = crate::preset_store::PresetStore::start(root.join("settings")).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while restored.busy() {
        restored.poll().unwrap();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(restored.entries.len(), 2);
    assert_eq!(restored.entries[0].preset, friend);
    drop(restored);
    std::fs::remove_dir_all(root).unwrap();
}
