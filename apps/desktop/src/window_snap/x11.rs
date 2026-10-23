//! EWMH placement avoids ConfigureRequest gravity differences between window managers.

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use slint::winit_030::winit;
use x11rb::connection::Connection;
use x11rb::cookie::Cookie;
use x11rb::protocol::xproto::{AtomEnum, ClientMessageEvent, ConnectionExt, EventMask};
use x11rb::rust_connection::RustConnection;

pub(super) struct Positioner {
    connection: RustConnection,
    root: u32,
    move_atom: u32,
    frame_atom: u32,
    state_atom: u32,
    #[cfg(test)]
    managed_atom: u32,
    excluded_states: [u32; 4],
}

impl Positioner {
    /// Reads the pointer only through an owned window during its explicit title-bar gesture.
    pub(super) fn pointer(&self, native: &winit::window::Window) -> Option<((i32, i32), bool)> {
        let id = window_id(native).ok()?;
        let reply = self.connection.query_pointer(id).ok()?.reply().ok()?;
        if !reply.same_screen {
            return None;
        }
        Some((
            (i32::from(reply.root_x), i32::from(reply.root_y)),
            reply
                .mask
                .contains(x11rb::protocol::xproto::KeyButMask::BUTTON1),
        ))
    }
    /// Preserves size constraints and supplies an explicit position before a withdrawn client maps.
    pub(super) fn prepare_show(
        &self,
        native: &winit::window::Window,
        x: i32,
        y: i32,
    ) -> Result<(), &'static str> {
        use x11rb::properties::{WmSizeHints, WmSizeHintsSpecification};
        let id = window_id(native)?;
        let mut hints = WmSizeHints::get_normal_hints(&self.connection, id)
            .map_err(|_| "Cannot read window placement hints")?
            .reply()
            .map_err(|_| "Cannot read window placement hints")?
            .unwrap_or_default();
        hints.position = Some((WmSizeHintsSpecification::UserSpecified, x, y));
        hints
            .set_normal_hints(&self.connection, id)
            .map_err(|_| "Cannot prepare window placement")?
            .check()
            .map_err(|_| "Cannot prepare window placement")?;
        Ok(())
    }

    /// Opens one local display connection and resolves the standard move/frame atoms.
    pub(super) fn connect() -> Result<Self, &'static str> {
        let (connection, screen) =
            x11rb::connect(None).map_err(|_| "Window positioning unavailable")?;
        let root = connection
            .setup()
            .roots
            .get(screen)
            .ok_or("Window display unavailable")?
            .root;
        let move_atom = connection
            .intern_atom(false, b"_NET_MOVERESIZE_WINDOW")
            .map_err(|_| "Window positioning unavailable")?
            .reply()
            .map_err(|_| "Window positioning unavailable")?
            .atom;
        let frame_atom = connection
            .intern_atom(false, b"_NET_FRAME_EXTENTS")
            .map_err(|_| "Window geometry unavailable")?
            .reply()
            .map_err(|_| "Window geometry unavailable")?
            .atom;
        let mut states = Vec::new();
        for name in [
            b"_NET_WM_STATE".as_slice(),
            b"_NET_WM_STATE_HIDDEN",
            b"_NET_WM_STATE_MAXIMIZED_HORZ",
            b"_NET_WM_STATE_MAXIMIZED_VERT",
            b"_NET_WM_STATE_FULLSCREEN",
        ] {
            states.push(
                connection
                    .intern_atom(false, name)
                    .map_err(|_| "Window state unavailable")?
                    .reply()
                    .map_err(|_| "Window state unavailable")?
                    .atom,
            );
        }
        #[cfg(test)]
        let managed_atom = connection
            .intern_atom(false, b"WM_STATE")
            .map_err(|_| "Window state unavailable")?
            .reply()
            .map_err(|_| "Window state unavailable")?
            .atom;
        Ok(Self {
            #[cfg(test)]
            managed_atom,
            connection,
            root,
            move_atom,
            frame_atom,
            state_atom: states[0],
            excluded_states: [states[1], states[2], states[3], states[4]],
        })
    }

    /// Waits for the WM to manage the main client before deriving the initial dock from its frame.
    #[cfg(test)]
    pub(super) fn mapped(&self, native: &winit::window::Window) -> bool {
        let Ok(id) = window_id(native) else {
            return false;
        };
        let Ok(cookie) =
            self.connection
                .get_property(false, id, self.managed_atom, self.managed_atom, 0, 2)
        else {
            return false;
        };
        let Ok(reply) = cookie.reply() else {
            return false;
        };
        reply.bytes_after == 0 && reply.value32().and_then(|mut values| values.next()) == Some(1)
    }

    /// Pipelines all owned geometry/state requests before reading replies: one server round trip
    /// for the group instead of repeated synchronous winit calls on every move event.
    pub(super) fn observe(
        &self,
        windows: &[Option<Identity>],
    ) -> Result<Vec<Option<super::geometry::Rect>>, &'static str> {
        let mut pending = Vec::with_capacity(windows.len());
        for window in windows {
            let request = match window {
                Some(window) => Some(self.request_frame(*window)?),
                None => None,
            };
            pending.push(request);
        }
        self.connection
            .flush()
            .map_err(|_| "Window geometry unavailable")?;
        let mut frames = Vec::with_capacity(pending.len());
        for request in pending {
            frames.push(request.and_then(|request| request.read(&self.excluded_states)));
        }
        Ok(frames)
    }

    /// Queues bounded properties and geometry without waiting for any individual response.
    fn request_frame(&self, window: Identity) -> Result<PendingFrame<'_>, &'static str> {
        let connection = &self.connection;
        let state = connection
            .get_property(false, window.id, self.state_atom, AtomEnum::ATOM, 0, 32)
            .map_err(|_| "Window state unavailable")?;
        let position = connection
            .translate_coordinates(window.id, self.root, 0, 0)
            .map_err(|_| "Window geometry unavailable")?;
        let size = connection
            .get_geometry(window.id)
            .map_err(|_| "Window geometry unavailable")?;
        let extents = if window.decorated {
            Some(
                connection
                    .get_property(false, window.id, self.frame_atom, AtomEnum::CARDINAL, 0, 4)
                    .map_err(|_| "Window geometry unavailable")?,
            )
        } else {
            None
        };
        Ok(PendingFrame {
            state,
            position,
            size,
            extents,
        })
    }

    /// Reads live WM extents instead of cached pre-decoration geometry.
    pub(super) fn bounds(&self, native: &winit::window::Window) -> Option<super::geometry::Rect> {
        // Frameless clients have no external border. KWin can omit the frame property entirely.
        let values = if native.is_decorated() {
            self.frame_extents(native)?
        } else {
            [0; 4]
        };
        let origin = native.inner_position().ok()?;
        let size = native.inner_size();
        Some(super::geometry::Rect {
            x: origin.x.checked_sub(values[0] as i32)?,
            y: origin.y.checked_sub(values[2] as i32)?,
            width: size.width.checked_add(values[0])?.checked_add(values[1])?,
            height: size.height.checked_add(values[2])?.checked_add(values[3])?,
        })
    }

    /// Requires bounded live extents for decorated windows, rejecting absent or malformed replies.
    fn frame_extents(&self, native: &winit::window::Window) -> Option<[u32; 4]> {
        let id = window_id(native).ok()?;
        let reply = self
            .connection
            .get_property(false, id, self.frame_atom, AtomEnum::CARDINAL, 0, 4)
            .ok()?
            .reply()
            .ok()?;
        let values: Vec<_> = reply.value32()?.collect();
        if reply.bytes_after != 0 || values.len() != 4 || values.iter().any(|value| *value > 4096) {
            return None;
        }
        values.try_into().ok()
    }

    /// Sends a position-only application request for a handle borrowed from our own live window.
    pub(super) fn place(
        &self,
        native: &winit::window::Window,
        x: i32,
        y: i32,
    ) -> Result<(), &'static str> {
        let id = window_id(native)?;
        // NorthWest gravity targets the outer top-left; x/y only, application source.
        let flags = 1 | (1 << 8) | (1 << 9) | (1 << 12);
        let event =
            ClientMessageEvent::new(32, id, self.move_atom, [flags, x as u32, y as u32, 0, 0]);
        self.connection
            .send_event(
                false,
                self.root,
                EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
                event,
            )
            .map_err(|_| "Cannot position window")?;
        self.connection
            .flush()
            .map_err(|_| "Cannot position window")
    }
}

/// Accepts only a handle borrowed from our live component, never an external numeric ID.
fn window_id(native: &winit::window::Window) -> Result<u32, &'static str> {
    let handle = native
        .window_handle()
        .map_err(|_| "Window handle unavailable")?;
    match handle.as_raw() {
        RawWindowHandle::Xlib(handle) => {
            u32::try_from(handle.window).map_err(|_| "Invalid window handle")
        }
        RawWindowHandle::Xcb(handle) => Ok(handle.window.get()),
        _ => Err("This display does not support window positioning"),
    }
}

/// Borrowed identity from an explicitly registered live component; no discovery by title/PID.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct Identity {
    id: u32,
    pub(super) decorated: bool,
}

/// Reads cached decoration state and handle type without a synchronous X11 query.
pub(super) fn identity(native: &winit::window::Window) -> Option<Identity> {
    Some(Identity {
        id: window_id(native).ok()?,
        decorated: native.is_decorated(),
    })
}

struct PendingFrame<'a> {
    state: Cookie<'a, RustConnection, x11rb::protocol::xproto::GetPropertyReply>,
    position: Cookie<'a, RustConnection, x11rb::protocol::xproto::TranslateCoordinatesReply>,
    size: Cookie<'a, RustConnection, x11rb::protocol::xproto::GetGeometryReply>,
    extents: Option<Cookie<'a, RustConnection, x11rb::protocol::xproto::GetPropertyReply>>,
}

impl PendingFrame<'_> {
    /// Validates bounded native replies; unavailable, hidden and maximized peers never attract moves.
    fn read(self, excluded: &[u32; 4]) -> Option<super::geometry::Rect> {
        let state = self.state.reply().ok()?;
        if state.bytes_after != 0 {
            return None;
        }
        if state.type_ != u32::from(AtomEnum::NONE) {
            if state.type_ != u32::from(AtomEnum::ATOM)
                || state.value32()?.any(|atom| excluded.contains(&atom))
            {
                return None;
            }
        } else if !state.value.is_empty() {
            return None;
        }
        let extents = match self.extents {
            None => [0; 4],
            Some(cookie) => {
                let reply = cookie.reply().ok()?;
                let values: Vec<_> = reply.value32()?.collect();
                if reply.bytes_after != 0
                    || values.len() != 4
                    || values.iter().any(|value| *value > 4096)
                {
                    return None;
                }
                values.try_into().ok()?
            }
        };
        let position = self.position.reply().ok()?;
        let size = self.size.reply().ok()?;
        if !position.same_screen || size.width == 0 || size.height == 0 {
            return None;
        }
        Some(super::geometry::Rect {
            x: i32::from(position.dst_x).checked_sub(extents[0] as i32)?,
            y: i32::from(position.dst_y).checked_sub(extents[2] as i32)?,
            width: u32::from(size.width)
                .checked_add(extents[0])?
                .checked_add(extents[1])?,
            height: u32::from(size.height)
                .checked_add(extents[2])?
                .checked_add(extents[3])?,
        })
    }
}
