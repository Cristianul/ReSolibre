//! Bounded expansion and shared UI/session regression checks use only owned temporary fixtures.

use super::*;
use crate::{PlayerWindow, playlist_window, presentation, ui_tests};
use resolibre_audio::{
    AudioOutput,
    session::{Command, Session},
};
use slint::{ComponentHandle, Model};
use std::sync::atomic::AtomicU64;

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Fixture {
    /// Creates a unique directory without touching user music or profile data.
    fn new() -> Self {
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("resolibre-import-{}-{id}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for Fixture {
    /// Deletes only the test's generated directory after assertions.
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

/// Recursive expansion is ordered, case-insensitive, read-only and never follows symlink loops.
#[test]
fn folders_expand_in_order_and_skip_links_and_non_audio() {
    let fixture = Fixture::new();
    std::fs::create_dir(fixture.0.join("album")).unwrap();
    for name in ["b.FLAC", "a.mp3", "notes.txt", "album/c.ogg"] {
        std::fs::write(fixture.0.join(name), b"unchanged").unwrap();
    }
    std::os::unix::fs::symlink(&fixture.0, fixture.0.join("loop")).unwrap();
    let result = scan::expand(vec![fixture.0.clone()], &AtomicBool::new(false)).unwrap();
    assert_eq!(
        result.paths,
        vec![
            fixture.0.join("a.mp3"),
            fixture.0.join("album/c.ogg"),
            fixture.0.join("b.FLAC")
        ]
    );

    for path in result.paths {
        assert_eq!(std::fs::read(path).unwrap(), b"unchanged");
    }
}

/// Empty input is a silent no-op; missing, relative, cancelled and oversized batches fail.
#[test]
fn rejected_scans_never_produce_partial_replacement() {
    let fixture = Fixture::new();
    assert!(
        scan::expand(vec![fixture.0.clone()], &AtomicBool::new(false))
            .unwrap()
            .paths
            .is_empty()
    );
    assert!(scan::expand(vec![fixture.0.join("missing")], &AtomicBool::new(false)).is_err());
    assert!(scan::expand(vec!["relative.mp3".into()], &AtomicBool::new(false)).is_err());
    assert!(scan::expand(vec![fixture.0.clone()], &AtomicBool::new(true)).is_err());
    let path = fixture.0.join("a.mp3");
    std::fs::write(&path, b"test").unwrap();
    let mut roots = vec![path; MAX_QUEUE_ITEMS];
    roots.push(fixture.0.clone());
    assert!(scan::expand(roots, &AtomicBool::new(false)).is_err());
}

/// Overflow poisons the complete native batch; repeated drops cannot bypass the queue limit.
#[test]
fn native_batches_are_bounded_and_busy_input_is_rejected() {
    let mut inbox = Inbox::default();
    for _ in 0..=MAX_QUEUE_ITEMS {
        inbox.receive("/a.mp3".into(), Target::Player, (10.0, 20.0));
    }
    assert!(inbox.rejected);
    assert!(inbox.paths.is_empty());
    inbox.clear();
    inbox.busy = true;
    inbox.receive("/b.mp3".into(), Target::Playlist, (0.0, 0.0));
    assert!(inbox.paths.is_empty());
    assert!(inbox.error.is_some());
}

/// Refreshes the production presentation until the expected queue or menu state is observable.
fn pump(
    main: &PlayerWindow,
    controller: &Rc<RefCell<presentation::Controller>>,
    host: &Rc<RefCell<playlist_window::PlaylistHost>>,
    predicate: impl Fn() -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        controller.borrow_mut().refresh(main);
        host.borrow().refresh(main);
        if predicate() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "drop import did not settle: {}",
            main.get_error_message()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Both hosts share Add/Replace/cancel, empty queues bypass prompts, and duplicate choices are inert.
#[test]
fn drops_share_menu_and_preserve_playback_until_replace() {
    i_slint_backend_testing::init_no_event_loop();
    let fixture = Fixture::new();
    let path = ui_tests::write_ui_fixture(&fixture.0);
    let original = std::fs::read(&path).unwrap();
    let image = fixture.0.join("cover.jpeg");
    std::fs::write(&image, b"ignored image").unwrap();
    let main = PlayerWindow::new().unwrap();
    let controller = Rc::new(RefCell::new(presentation::Controller::new(
        Session::start(AudioOutput::Silent).unwrap(),
    )));
    presentation::connect(&main, &controller);
    let inbox = controller.borrow().import_inbox();
    let host = playlist_window::connect(&main, inbox.clone(), Default::default());
    main.show().unwrap();
    host.borrow_mut().show(&main, &host).unwrap();

    inbox
        .borrow_mut()
        .receive(image.clone(), Target::Player, (30.0, 40.0));
    pump(&main, &controller, &host, || !main.get_picking());
    assert_eq!(main.get_tracks().row_count(), 0);
    assert!(main.get_import_status().is_empty());
    assert!(main.get_error_message().is_empty());
    assert!(!main.get_drop_menu());

    inbox
        .borrow_mut()
        .receive(image.clone(), Target::Player, (30.0, 40.0));
    inbox
        .borrow_mut()
        .receive(path.clone(), Target::Player, (30.0, 40.0));
    pump(&main, &controller, &host, || {
        main.get_tracks().row_count() == 1
    });
    assert!(!main.get_drop_menu());
    assert!(!main.get_playing());
    assert!(main.get_import_status().is_empty());
    main.invoke_play_pause();
    pump(&main, &controller, &host, || main.get_playing());

    inbox
        .borrow_mut()
        .receive(path.clone(), Target::Playlist, (100.0, 80.0));
    pump(&main, &controller, &host, || main.get_drop_menu());
    let playlist = host.borrow().test_window();
    assert!(playlist.get_drop_menu());
    assert!(main.get_playing());
    ui_tests::click(&playlist, "Add dropped music");
    pump(&main, &controller, &host, || {
        main.get_tracks().row_count() == 2
    });
    assert!(main.get_playing());
    assert!(main.get_import_status().is_empty());
    inbox
        .borrow_mut()
        .receive(image.clone(), Target::Playlist, (30.0, 40.0));
    pump(&main, &controller, &host, || !main.get_picking());
    assert_eq!(main.get_tracks().row_count(), 2);
    assert!(main.get_playing());
    assert!(main.get_import_status().is_empty());
    assert!(main.get_error_message().is_empty());
    assert!(!main.get_drop_menu());

    inbox
        .borrow_mut()
        .receive(path.clone(), Target::Player, (60.0, 70.0));
    pump(&main, &controller, &host, || main.get_drop_menu());
    ui_tests::send_key(&main, slint::platform::Key::Escape);
    assert!(!main.get_drop_menu());
    assert_eq!(main.get_tracks().row_count(), 2);

    inbox
        .borrow_mut()
        .receive(path.clone(), Target::Playlist, (300.0, 480.0));
    pump(&main, &controller, &host, || main.get_drop_menu());
    // Outside clicks cancel without reaching the transport underneath the overlay.
    ui_tests::pointer(&playlist, slint::LogicalPosition::new(30.0, 180.0), "down");
    ui_tests::pointer(&playlist, slint::LogicalPosition::new(30.0, 180.0), "up");
    assert!(!main.get_drop_menu());
    inbox
        .borrow_mut()
        .receive(path.clone(), Target::Playlist, (300.0, 480.0));
    pump(&main, &controller, &host, || main.get_drop_menu());
    let old_id = main.get_tracks().row_data(0).unwrap().id;
    ui_tests::click(&playlist, "Replace library");
    main.invoke_drop_choice(1);
    pump(&main, &controller, &host, || {
        main.get_tracks().row_count() == 1 && !main.get_playing()
    });
    assert_ne!(main.get_tracks().row_data(0).unwrap().id, old_id);
    assert_eq!(main.get_current(), 0);
    assert_eq!(std::fs::read(&path).unwrap(), original);
    controller.borrow_mut().submit(
        &main,
        Command::Queue(resolibre_core::queue::QueueAction::Select(
            old_id.parse().unwrap(),
        )),
    );
    controller.borrow_mut().refresh(&main);
    assert!(!main.get_playing());
    inbox
        .borrow_mut()
        .receive(path.clone(), Target::Playlist, (100.0, 80.0));
    main.invoke_toggle_playlist();
    pump(&main, &controller, &host, || main.get_drop_menu());
    assert!(
        !main.get_drop_playlist(),
        "hidden target falls back to Player"
    );
    main.invoke_drop_choice(-1);
    inbox
        .borrow_mut()
        .receive(fixture.0.join("missing"), Target::Player, (0.0, 0.0));
    pump(&main, &controller, &host, || {
        main.get_error_message().contains("Cannot read")
    });
    assert_eq!(main.get_tracks().row_count(), 1);
    assert!(!main.get_drop_menu());
    host.borrow_mut().close();
}

/// Cancellation owns and reaps the worker instead of leaving a detached scanner behind.
#[test]
fn importer_close_cancels_and_reclaims_worker() {
    let fixture = Fixture::new();
    let mut importer = Importer::default();
    importer
        .inbox
        .borrow_mut()
        .receive(fixture.0.clone(), Target::Player, (0.0, 0.0));
    importer.inbox.borrow_mut().last_drop = Some(Instant::now() - Duration::from_secs(1));
    assert!(importer.poll().is_none());
    let deadline = Instant::now() + Duration::from_secs(2);
    while !importer.close() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(!importer.reserved());
}

/// Recursive queue/depth limits reject the complete result even after earlier playable files.
#[test]
fn recursive_limits_and_non_utf8_paths_are_handled() {
    use std::os::unix::ffi::OsStringExt;
    let fixture = Fixture::new();
    let encoded = std::ffi::OsString::from_vec(b"music-\xff.mp3".to_vec());
    let path = fixture.0.join(encoded);
    std::fs::write(&path, b"unchanged").unwrap();
    let result = scan::expand(vec![fixture.0.clone()], &AtomicBool::new(false)).unwrap();
    assert_eq!(result.paths, vec![path.clone()]);
    for index in 0..MAX_QUEUE_ITEMS {
        std::fs::write(fixture.0.join(format!("{index:04}.wav")), b"test").unwrap();
    }
    assert!(scan::expand(vec![fixture.0.clone()], &AtomicBool::new(false)).is_err());
    let nested = Fixture::new();
    let mut directory = nested.0.clone();
    for _ in 0..33 {
        directory.push("sub");
        std::fs::create_dir(&directory).unwrap();
    }
    assert!(scan::expand(vec![nested.0.clone()], &AtomicBool::new(false)).is_err());
    assert_eq!(std::fs::read(path).unwrap(), b"unchanged");
}

/// Audio/video candidates share case-insensitive filtering; images/manifests are silent no-ops.
#[test]
fn media_candidates_exclude_images_and_playlists() {
    let fixture = Fixture::new();
    for name in [
        "cover.JPEG",
        "cover.png",
        "notes.txt",
        "list.m3u",
        "list.m3u8",
    ] {
        std::fs::write(fixture.0.join(name), b"ignored").unwrap();
    }
    let ignored = scan::expand(vec![fixture.0.clone()], &AtomicBool::new(false)).unwrap();
    assert!(ignored.paths.is_empty());
    for name in [
        "audio.M4A",
        "audio.OPUS",
        "video.MP4",
        "video.MKV",
        "video.webm",
    ] {
        let path = fixture.0.join(name);
        std::fs::write(&path, b"candidate, not decoded during scan").unwrap();
        let selection = scan::expand(vec![path.clone()], &AtomicBool::new(false)).unwrap();
        assert_eq!(selection.paths, vec![path]);
    }
}
