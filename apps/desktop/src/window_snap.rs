//! Window placement adapter: only explicitly registered ReSolibre surfaces can attract one another.

pub(crate) mod drag;
mod geometry;
mod movement;
#[cfg(unix)]
mod x11;
#[cfg(windows)]
#[path = "window_snap/windows.rs"]
mod x11;

use geometry::Rect;

type WindowIdentity = Box<dyn Fn() -> Option<x11::Identity>>;
type WindowPlacement = Box<dyn Fn(&SnapGroup, i32, i32)>;

struct Peer {
    identity: WindowIdentity,
    place: WindowPlacement,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Player,
    Auxiliary,
}
use slint::ComponentHandle;
use slint::winit_030::{WinitWindowAccessor, winit};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Default)]
pub struct SnapGroup {
    peers: Vec<Peer>,
    player: Option<usize>,
    movement: RefCell<movement::Movement>,
    positioner: RefCell<Option<Result<x11::Positioner, &'static str>>>,
    drag: RefCell<Option<drag::GroupDrag>>,
}

impl SnapGroup {
    /// Native Wayland/testing need no global placement; X11 waits for its first managed frame.
    #[cfg(test)]
    pub fn ready_to_dock(&self, main: &slint::Window) -> bool {
        if supported(main) != Some(true) {
            return true;
        }
        self.prepare();
        let positioner = self.positioner.borrow();
        let Some(Ok(adapter)) = positioner.as_ref() else {
            return true;
        };
        main.with_winit_window(|native| adapter.mapped(native))
            .unwrap_or(false)
    }
    /// Registers one weak component handle and a move listener; hidden windows never participate.
    pub fn watch<C: ComponentHandle + 'static>(
        group: &Rc<RefCell<Self>>,
        component: &C,
        role: Role,
        events: &crate::window_events::WindowEvents,
    ) {
        let index = group.borrow().peers.len();
        // The current app has two surfaces. Bound future registration mistakes explicitly.
        if index >= 16 {
            eprintln!("warn: window snapping surface limit reached");
            return;
        }
        let weak = component.as_weak();
        let placement = component.as_weak();
        let mut registry = group.borrow_mut();
        if role == Role::Player {
            registry.player = Some(index);
        }
        registry.movement.borrow_mut().register();
        registry.peers.push(Peer {
            identity: Box::new(move || {
                let component = weak.upgrade()?;
                if !component.window().is_visible()
                    || component.window().is_minimized()
                    || component.window().is_maximized()
                    || component.window().is_fullscreen()
                {
                    return None;
                }
                component
                    .window()
                    .with_winit_window(x11::identity)
                    .flatten()
            }),
            place: Box::new(move |group, x, y| {
                if let Some(component) = placement.upgrade() {
                    group.place(component.window(), x, y);
                }
            }),
        });
        drop(registry);
        let owner = Rc::downgrade(group);
        events.listen(move |_, event| {
            let Some(group) = owner.upgrade() else {
                return;
            };
            let group = group.borrow();
            group.handle_drag_interruption(index, event);
            match event {
                winit::event::WindowEvent::Moved(_) => group.moved(index),
                winit::event::WindowEvent::Resized(_) => {
                    group.end_drag();
                    group.moved(index);
                }
                _ => {}
            }
        });
    }

    /// A follower losing focus while the player is activated must not cancel its captured drag.
    fn handle_drag_interruption(&self, index: usize, event: &winit::event::WindowEvent) {
        let player_lost_focus = self.player == Some(index)
            && matches!(event, winit::event::WindowEvent::Focused(false));
        let peer_destroyed = matches!(event, winit::event::WindowEvent::Destroyed);
        let escape_pressed = matches!(event, winit::event::WindowEvent::KeyboardInput { event, .. }
            if event.logical_key == winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape));
        if player_lost_focus || peer_destroyed || escape_pressed {
            self.end_drag();
        }
    }

    /// Moves the player's attached group or snaps an independently moved auxiliary window.
    fn moved(&self, moving_index: usize) {
        if self.drag.borrow().is_some() {
            return;
        }
        let windows: Vec<_> = self.peers.iter().map(|peer| (peer.identity)()).collect();
        if windows[moving_index].is_none() {
            return;
        }
        self.prepare();
        let mut positioner = self.positioner.borrow_mut();
        let Some(Ok(adapter)) = positioner.as_ref() else {
            return;
        };
        let frames = match adapter.observe(&windows) {
            Ok(frames) => frames,
            Err(error) => {
                eprintln!("warn: {error}");
                *positioner = Some(Err(error));
                return;
            }
        };
        let requests = self
            .movement
            .borrow_mut()
            .update(moving_index, self.player, &frames);
        drop(positioner);
        for (index, target) in requests {
            (self.peers[index].place)(self, target.x, target.y);
        }
    }
    /// Captures the owned window's last geometry, including when minimized before hiding.
    pub fn position(&self, window: &slint::Window) -> Option<(i32, i32)> {
        if !supported(window).unwrap_or(false) {
            return None;
        }
        self.prepare();
        let positioner = self.positioner.borrow();
        let adapter = positioner.as_ref()?.as_ref().ok()?;
        let frame = window
            .with_winit_window(|native| adapter.bounds(native))
            .flatten()?;
        Some((frame.x, frame.y))
    }

    /// Restores owned geometry after remapping; the compositor may center a newly shown client.
    pub fn restore_position(&self, window: &slint::Window, target: (i32, i32)) -> bool {
        if self.position(window) == Some(target) {
            return true;
        }
        self.place(window, target.0, target.1);
        false
    }

    /// Sets creation coordinates and ICCCM placement hints before showing an owned hidden window.
    pub fn prepare_show(&self, window: &slint::Window, target: (i32, i32)) {
        window.set_position(slint::PhysicalPosition::new(target.0, target.1));
        self.prepare();
        let mut positioner = self.positioner.borrow_mut();
        let Some(Ok(adapter)) = positioner.as_ref() else {
            return;
        };
        if let Some(Err(error)) =
            window.with_winit_window(|native| adapter.prepare_show(native, target.0, target.1))
        {
            eprintln!("warn: {error}");
            *positioner = Some(Err(error));
        }
    }

    /// Centers the initial pair and sets both positions before either X11 client is mapped.
    pub fn prepare_startup_pair(&self, main: &slint::Window, library: &slint::Window) -> bool {
        if supported(main) != Some(true) {
            return true;
        }
        let monitor = main
            .with_winit_window(|native| native.current_monitor())
            .flatten();
        let Some(monitor) = monitor else {
            return false;
        };
        let origin = monitor.position();
        let area = monitor.size();
        // Hidden Slint surfaces may still report their pre-DPI size until their first draw.
        let Some(player) = main.with_winit_window(|native| native.inner_size()) else {
            return false;
        };
        let Some(queue) = library.with_winit_window(|native| native.inner_size()) else {
            return false;
        };
        let width = i64::from(player.width) + i64::from(queue.width);
        let height = i64::from(player.height.max(queue.height));
        let x = i64::from(origin.x) + (i64::from(area.width) - width).max(0) / 2;
        let y = i64::from(origin.y) + (i64::from(area.height) - height).max(0) / 2;
        let Ok(main_x) = i32::try_from(x) else {
            return false;
        };
        let Ok(main_y) = i32::try_from(y) else {
            return false;
        };
        let Ok(library_x) = i32::try_from(x + i64::from(player.width)) else {
            return false;
        };
        self.prepare_show(main, (main_x, main_y));
        self.prepare_show(library, (library_x, main_y));
        true
    }

    /// Initially docks this owned playlist to the right; later user moves are never overridden.
    pub fn dock_right(&self, main: &slint::Window, playlist: &slint::Window) -> bool {
        self.place_right(main, playlist, false)
    }

    /// Supplies the initial dock coordinates before the playlist's first mapped frame.
    pub fn prepare_dock(&self, main: &slint::Window, playlist: &slint::Window) -> bool {
        self.place_right(main, playlist, true)
    }

    /// Fits the pair onto the monitor, using premapping hints only for a hidden playlist.
    fn place_right(
        &self,
        main: &slint::Window,
        playlist: &slint::Window,
        before_map: bool,
    ) -> bool {
        if supported(main) == Some(false) || supported(playlist) == Some(false) {
            return true;
        }
        self.prepare();
        let positioner = self.positioner.borrow();
        let Some(Ok(adapter)) = positioner.as_ref() else {
            return false;
        };
        let Some(player) = bounds(adapter, main) else {
            return false;
        };
        let queue = bounds(adapter, playlist).or_else(|| {
            let size = playlist.size();
            (before_map && size.width > 0 && size.height > 0).then_some(Rect {
                x: 0,
                y: 0,
                width: size.width,
                height: size.height,
            })
        });
        let Some(queue) = queue else {
            return false;
        };
        let monitor = main
            .with_winit_window(|native| {
                let monitor = native.current_monitor()?;
                let position = monitor.position();
                let size = monitor.size();
                Some(Rect {
                    x: position.x,
                    y: position.y,
                    width: size.width,
                    height: size.height,
                })
            })
            .flatten();
        let Some(monitor) = monitor else {
            return false;
        };
        let Some((main_x, main_y, playlist_x)) = geometry::right_dock(player, queue, monitor)
        else {
            return false;
        };
        drop(positioner);
        if before_map {
            if player.x != main_x || player.y != main_y {
                self.place(main, main_x, main_y);
            }
            self.prepare_show(playlist, (playlist_x, main_y));
            return true;
        }
        if player.x == main_x && player.y == main_y && queue.x == playlist_x && queue.y == main_y {
            return true;
        }
        self.place(main, main_x, main_y);
        self.place(playlist, playlist_x, main_y);
        false
    }

    /// Lazily owns one placement connection; a display failure disables retries and logs once.
    fn prepare(&self) {
        let mut positioner = self.positioner.borrow_mut();
        if positioner.is_none() {
            *positioner = Some(x11::Positioner::connect());
            if let Some(Err(error)) = positioner.as_ref() {
                eprintln!("warn: {error}");
            }
        }
    }

    /// Sends a position request through the same display adapter used for geometry.
    fn place(&self, window: &slint::Window, x: i32, y: i32) {
        self.prepare();
        let mut positioner = self.positioner.borrow_mut();
        let Some(Ok(adapter)) = positioner.as_ref() else {
            return;
        };
        let result = window.with_winit_window(|native| adapter.place(native, x, y));
        if let Some(Err(error)) = result {
            eprintln!("warn: {error}");
            *positioner = Some(Err(error));
        }
    }
}

/// Gets the actual outer frame, including decorations; unsupported backends return no geometry.
fn bounds(adapter: &x11::Positioner, window: &slint::Window) -> Option<Rect> {
    window
        .with_winit_window(|native| {
            if native.is_maximized()
                || native.fullscreen().is_some()
                || native.is_minimized() == Some(true)
            {
                return None;
            }
            adapter.bounds(native)
        })
        .flatten()
}

/// Exposes capability honestly instead of claiming magnetic positioning on native Wayland.
pub fn supported(window: &slint::Window) -> Option<bool> {
    window.with_winit_window(|native| native.outer_position().is_ok())
}

/// Raises an already-detached surface without changing the selected track or main lower panel.
pub fn present(window: &slint::Window) {
    let _ = window.show();
    window.set_minimized(false);
    window.with_winit_window(|native| {
        native.focus_window();
    });
}

#[cfg(all(test, unix))]
mod native_tests;

/// Selects X11/XWayland for this invocation only, before creating any Slint window.
#[cfg(unix)]
pub fn select_x11() -> Result<(), &'static str> {
    use winit::platform::x11::EventLoopBuilderExtX11;
    let mut builder =
        winit::event_loop::EventLoop::<slint::winit_030::SlintEvent>::with_user_event();
    builder.with_x11();
    slint::BackendSelector::new().backend_name("winit".into())
        .with_winit_event_loop_builder(builder).select()
        .map_err(|_| "Cannot start X11/XWayland display; launch with --native-display to use the default display")
}

/// Windows always uses the native display backend.
#[cfg(windows)]
pub fn select_x11() -> Result<(), &'static str> {
    Err("X11 is unavailable on Windows; use --native-display")
}
