//! Real XDND delivery to owned windows on an isolated X11 display, without global input injection.

use super::*;
use crate::{PlayerWindow, playlist_window, presentation, ui_tests};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use resolibre_audio::{AudioOutput, session::Session};
use slint::winit_030::WinitWindowAccessor;
use slint::{ComponentHandle, Model};
use x11rb::connection::Connection;
use x11rb::protocol::{Event, xproto::*};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

struct DragSource {
    connection: RustConnection,
    window: u32,
    enter: u32,
    position: u32,
    drop_atom: u32,
    selection: u32,
    uri: u32,
    copy: u32,
    destination: u32,
    payload: Vec<u8>,
}

impl DragSource {
    /// Owns a private XDND source and selection; it never reads another application's clipboard.
    fn new() -> Self {
        let (connection, screen) = x11rb::connect(None).unwrap();
        let window = connection.generate_id().unwrap();
        connection
            .create_window(
                0,
                window,
                connection.setup().roots[screen].root,
                0,
                0,
                1,
                1,
                0,
                WindowClass::INPUT_OUTPUT,
                0,
                &CreateWindowAux::new(),
            )
            .unwrap();
        let names = [
            "XdndEnter",
            "XdndPosition",
            "XdndDrop",
            "XdndSelection",
            "text/uri-list",
            "XdndActionCopy",
        ];
        let atoms: Vec<_> = names
            .iter()
            .map(|name| {
                connection
                    .intern_atom(false, name.as_bytes())
                    .unwrap()
                    .reply()
                    .unwrap()
                    .atom
            })
            .collect();
        connection
            .set_selection_owner(window, atoms[3], x11rb::CURRENT_TIME)
            .unwrap();
        Self {
            connection,
            window,
            enter: atoms[0],
            position: atoms[1],
            drop_atom: atoms[2],
            selection: atoms[3],
            uri: atoms[4],
            copy: atoms[5],
            destination: 0,
            payload: Vec::new(),
        }
    }

    /// Sends the documented X11 protocol only to the supplied test-owned native window.
    fn start(&mut self, window: &slint::Window, paths: &[PathBuf]) {
        self.destination = window
            .with_winit_window(|native| match native.window_handle().unwrap().as_raw() {
                RawWindowHandle::Xlib(handle) => handle.window as u32,
                RawWindowHandle::Xcb(handle) => handle.window.get(),
                _ => panic!("X11 test window"),
            })
            .unwrap();
        self.payload.clear();
        for path in paths {
            self.payload.extend_from_slice(b"file://");
            for byte in path.as_os_str().as_encoded_bytes() {
                if byte.is_ascii_alphanumeric() || b"/-._~".contains(byte) {
                    self.payload.push(*byte);
                } else {
                    self.payload
                        .extend_from_slice(format!("%{byte:02X}").as_bytes());
                }
            }
            self.payload.extend_from_slice(b"\r\n");
        }
        self.send(self.enter, [self.window, 5 << 24, self.uri, 0, 0]);
        self.send(
            self.position,
            [self.window, 0, 0, x11rb::CURRENT_TIME, self.copy],
        );
        self.connection.flush().unwrap();
    }

    /// Replies to the owned URI selection and completes the drop in server event order.
    fn progress(&mut self) {
        while let Some(event) = self.connection.poll_for_event().unwrap() {
            if let Event::SelectionRequest(request) = event {
                assert_eq!(request.selection, self.selection);
                assert_eq!(request.requestor, self.destination);
                self.connection
                    .change_property8(
                        PropMode::REPLACE,
                        request.requestor,
                        request.property,
                        self.uri,
                        &self.payload,
                    )
                    .unwrap();
                let response = SelectionNotifyEvent {
                    response_type: SELECTION_NOTIFY_EVENT,
                    sequence: 0,
                    time: request.time,
                    requestor: request.requestor,
                    selection: request.selection,
                    target: request.target,
                    property: request.property,
                };
                self.connection
                    .send_event(false, request.requestor, EventMask::NO_EVENT, response)
                    .unwrap();
                self.send(self.drop_atom, [self.window, 0, x11rb::CURRENT_TIME, 0, 0]);
                self.connection.flush().unwrap();
            }
        }
    }

    /// Encodes one XDND client message without pointer warping or global event injection.
    fn send(&self, kind: u32, data: [u32; 5]) {
        self.connection
            .send_event(
                false,
                self.destination,
                EventMask::NO_EVENT,
                ClientMessageEvent::new(32, self.destination, kind, data),
            )
            .unwrap();
    }
}

/// Native main/playlist drops preserve the batch and share the production scanner and decision menu.
#[test]
#[ignore = "requires isolated X11/XWayland; run through tools/check_window_snap.py"]
fn native_drop_journey() {
    crate::window_snap::select_x11().unwrap();
    let directory = std::env::temp_dir().join(format!("resolibre-native-drop-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let album = directory.join("Album 音楽");
    std::fs::create_dir(&album).unwrap();
    let first = ui_tests::write_ui_fixture(&album);
    let second = album.join("Second.FLAC");
    std::fs::copy(&first, &second).unwrap();
    std::fs::write(album.join("notes.txt"), b"not audio").unwrap();
    let original = std::fs::read(&first).unwrap();
    let main = PlayerWindow::new().unwrap();
    let controller = Rc::new(RefCell::new(presentation::Controller::new(
        Session::start(AudioOutput::Silent).unwrap(),
    )));
    presentation::connect(&main, &controller);
    let host = playlist_window::connect(
        &main,
        controller.borrow().import_inbox(),
        Default::default(),
    );
    main.show().unwrap();
    host.borrow_mut().show(&main, &host).unwrap();
    let playlist = host.borrow().test_window();
    let mut source = DragSource::new();
    let mut stage = 0;
    let started = Instant::now();
    let weak = main.as_weak();
    let timer = slint::Timer::default();
    let observed = controller.clone();
    let queue = host.clone();
    let test_first = first.clone();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(30),
        move || {
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "native drop stage {stage}"
            );
            let main = weak.upgrade().unwrap();
            source.progress();
            observed.borrow_mut().refresh(&main);
            queue.borrow().refresh(&main);
            match stage {
                0 => {
                    source.start(main.window(), std::slice::from_ref(&album));
                    stage = 1;
                }
                1 if main.get_tracks().row_count() == 2 => {
                    assert!(!main.get_drop_menu());
                    assert!(!main.get_playing());
                    source.start(playlist.window(), &[test_first.clone(), second.clone()]);
                    stage = 2;
                }
                2 if playlist.get_drop_menu() => {
                    let output = PathBuf::from(std::env::var_os("RESOUL_PREVIEW_DIR").unwrap());
                    let pixels = playlist.window().take_snapshot().unwrap();
                    image::save_buffer(
                        output.join("playlist-drop-menu.png"),
                        pixels.as_bytes(),
                        pixels.width(),
                        pixels.height(),
                        image::ColorType::Rgba8,
                    )
                    .unwrap();
                    playlist.invoke_drop_choice(0);
                    stage = 3;
                }
                3 if main.get_tracks().row_count() == 4 => {
                    source.start(main.window(), std::slice::from_ref(&test_first));
                    stage = 4;
                }
                4 if main.get_drop_menu() => {
                    main.invoke_drop_choice(1);
                    stage = 5;
                }
                5 if main.get_tracks().row_count() == 1 => {
                    assert_eq!(main.get_current(), 0);
                    assert!(!main.get_playing());
                    queue.borrow_mut().close();
                    slint::quit_event_loop().unwrap();
                    stage = 6;
                }
                _ => {}
            }
        },
    );
    main.run().unwrap();
    assert_eq!(std::fs::read(first).unwrap(), original);
    drop(controller);
    std::fs::remove_dir_all(directory).unwrap();
}
