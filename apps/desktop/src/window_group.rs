//! Shared activation for explicitly registered application windows; never changes keyboard focus.

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use slint::ComponentHandle;
use slint::winit_030::{WinitWindowAccessor, winit};
use std::cell::RefCell;
use std::rc::Rc;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ClientMessageEvent, ConnectionExt, EventMask};
use x11rb::rust_connection::RustConnection;

type Snapshot = Box<dyn Fn() -> Option<Surface>>;
pub type SharedGroup = Rc<RefCell<WindowGroup>>;

mod membership;

struct Member {
    snapshot: Snapshot,
    configured: Option<(u32, u32)>,
}

struct Surface {
    id: u32,
    focused: bool,
    eligible: bool,
}

#[derive(Default)]
pub struct WindowGroup {
    surfaces: Vec<Member>,
    timer: slint::Timer,
    membership_timer: slint::Timer,
    adapter: Option<Result<Restacker, ()>>,
}

impl WindowGroup {
    /// Registers a weak host; register the main player first as the group's switcher entry.
    pub fn watch<T: ComponentHandle + 'static>(
        owner: &SharedGroup,
        component: &T,
        events: &crate::window_events::WindowEvents,
    ) {
        let mut group = owner.borrow_mut();
        if group.surfaces.len() >= 16 {
            eprintln!("warn: application window group is full");
            return;
        }
        let index = group.surfaces.len();
        let weak = component.as_weak();
        group.surfaces.push(Member {
            configured: None,
            snapshot: Box::new(move || {
                let component = weak.upgrade()?;
                let window = component.window();
                window
                    .with_winit_window(|native| {
                        let handle = native.window_handle().ok()?;
                        let id = match handle.as_raw() {
                            RawWindowHandle::Xlib(handle) => u32::try_from(handle.window).ok()?,
                            RawWindowHandle::Xcb(handle) => handle.window.get(),
                            _ => return None,
                        };
                        Some(Surface {
                            id,
                            focused: native.has_focus(),
                            eligible: window.is_visible() && native.is_minimized() != Some(true),
                        })
                    })
                    .flatten()
            }),
        });
        drop(group);
        let weak_owner = Rc::downgrade(owner);
        events.listen(move |_, event| {
            if let Some(owner) = weak_owner.upgrade() {
                Self::handle_event(&owner, index, event);
            }
        });
        Self::schedule_membership(owner, false);
    }

    /// Schedules bounded restacking after focus or clicks, including an already-focused window.
    fn handle_event(owner: &SharedGroup, index: usize, event: &winit::event::WindowEvent) {
        // Native handles arrive after registration. Reapply state hints on focus/remapping;
        // ordinary redraws do no work after all current handles have been configured.
        match event {
            winit::event::WindowEvent::Focused(_) => Self::schedule_membership(owner, true),
            winit::event::WindowEvent::Resized(_) | winit::event::WindowEvent::RedrawRequested => {
                Self::schedule_membership(owner, false)
            }
            winit::event::WindowEvent::Destroyed => {
                owner.borrow_mut().surfaces[index].configured = None
            }
            _ => {}
        }
        if !matches!(
            event,
            winit::event::WindowEvent::Focused(true)
                | winit::event::WindowEvent::MouseInput {
                    state: winit::event::ElementState::Pressed,
                    ..
                }
        ) {
            return;
        }
        let pending = Rc::downgrade(owner);
        // The WM may finish click-to-raise after focus delivery. Each pass checks active ownership.
        let mut remaining = 8;
        owner.borrow().timer.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_millis(16),
            move || {
                if let Some(owner) = pending.upgrade() {
                    let mut group = owner.borrow_mut();
                    remaining -= 1;
                    let focused = group.raise_visible(index);
                    if !focused || remaining == 0 {
                        group.timer.stop();
                    }
                }
            },
        );
    }

    /// Defers native-handle access until Slint has finished delivering its current window event.
    fn schedule_membership(owner: &SharedGroup, refresh: bool) {
        let group = owner.borrow();
        if !refresh
            && (group.membership_timer.running()
                || group
                    .surfaces
                    .iter()
                    .all(|member| member.configured.is_some()))
        {
            return;
        }
        let pending = Rc::downgrade(owner);
        let mut remaining = 8;
        group.membership_timer.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_millis(16),
            move || {
                if let Some(owner) = pending.upgrade() {
                    let mut group = owner.borrow_mut();
                    group.configure_members();
                    remaining -= 1;
                    if remaining == 0 {
                        group.membership_timer.stop();
                    }
                }
            },
        );
    }

    /// Places visible peers below the still-focused surface; hidden/minimized/dead peers stay put.
    fn raise_visible(&mut self, selected: usize) -> bool {
        let Some(active) = (self.surfaces[selected].snapshot)() else {
            return false;
        };
        if !active.focused || !active.eligible {
            return false;
        }
        let adapter = self.adapter.get_or_insert_with(Restacker::connect);
        let Ok(adapter) = adapter else {
            return false;
        };
        let result = adapter.raise_visible(&self.surfaces, active.id);
        if result.is_err() {
            eprintln!("warn: application window grouping unavailable");
            self.adapter = Some(Err(()));
            return false;
        }
        true
    }

    /// Declares only registered X11 hosts as one application, preserving other WM state.
    fn configure_members(&mut self) {
        let Some(leader) = self.surfaces.first().and_then(|member| (member.snapshot)()) else {
            return;
        };
        let Ok(adapter) = self.adapter.get_or_insert_with(Restacker::connect) else {
            return;
        };
        for member in &mut self.surfaces {
            let Some(surface) = (member.snapshot)() else {
                continue;
            };
            let identity = (surface.id, leader.id);
            let declare = member.configured != Some(identity);
            match adapter.configure_member(surface.id, leader.id, declare) {
                Ok(true) => member.configured = Some(identity),
                Ok(false) => member.configured = None,
                Err(()) => {
                    eprintln!("warn: application window grouping unavailable");
                    self.adapter = Some(Err(()));
                    return;
                }
            }
        }
    }
}

struct Restacker {
    connection: RustConnection,
    root: u32,
    restack: u32,
    active: u32,
    membership: membership::Atoms,
}

impl Restacker {
    /// Opens one connection lazily on X11 only; failures disable this optional behavior once.
    fn connect() -> Result<Self, ()> {
        let result = Self::open();
        if result.is_err() {
            eprintln!("warn: application window grouping unavailable");
        }
        result
    }

    /// Resolves standard WM properties without enumerating or modifying foreign windows.
    fn open() -> Result<Self, ()> {
        let (connection, screen) = x11rb::connect(None).map_err(|_| ())?;
        let root = connection.setup().roots.get(screen).ok_or(())?.root;
        let restack = connection
            .intern_atom(false, b"_NET_RESTACK_WINDOW")
            .map_err(|_| ())?
            .reply()
            .map_err(|_| ())?
            .atom;
        let active = connection
            .intern_atom(false, b"_NET_ACTIVE_WINDOW")
            .map_err(|_| ())?
            .reply()
            .map_err(|_| ())?
            .atom;
        let membership = membership::Atoms::open(&connection)?;
        Ok(Self {
            connection,
            root,
            restack,
            active,
            membership,
        })
    }

    /// Uses application-source restacking below the chosen window, never an activation/topmost request.
    fn raise_visible(&self, surfaces: &[Member], selected: u32) -> Result<(), ()> {
        let reply = self
            .connection
            .get_property(
                false,
                self.root,
                self.active,
                x11rb::protocol::xproto::AtomEnum::WINDOW,
                0,
                1,
            )
            .map_err(|_| ())?
            .reply()
            .map_err(|_| ())?;
        // Reject stale queued focus events if the WM has already activated another application.
        if reply.bytes_after != 0
            || reply.value32().and_then(|mut ids| ids.next()) != Some(selected)
        {
            return Ok(());
        }
        for member in surfaces {
            let Some(surface) = (member.snapshot)() else {
                continue;
            };
            if surface.id == selected || !surface.eligible {
                continue;
            }
            let event = ClientMessageEvent::new(
                32,
                surface.id,
                self.restack,
                [
                    1,
                    selected,
                    u32::from(x11rb::protocol::xproto::StackMode::BELOW),
                    0,
                    0,
                ],
            );
            self.connection
                .send_event(
                    false,
                    self.root,
                    EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
                    event,
                )
                .map_err(|_| ())?;
        }
        self.connection.flush().map_err(|_| ())
    }
}

#[cfg(test)]
mod native_tests;
