//! Native Windows ownership keeps auxiliary surfaces with the registered player.

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use slint::ComponentHandle;
use slint::winit_030::{WinitWindowAccessor, winit};
use std::cell::RefCell;
use std::rc::Rc;
use windows_sys::Win32::Foundation::{GetLastError, SetLastError};
use windows_sys::Win32::UI::WindowsAndMessaging::{GWLP_HWNDPARENT, SetWindowLongPtrW};

pub type SharedGroup = Rc<RefCell<WindowGroup>>;
type Configure = Box<dyn Fn(isize) -> Option<isize>>;

#[derive(Default)]
pub struct WindowGroup {
    main: Option<Box<dyn Fn() -> Option<isize>>>,
    members: Vec<Configure>,
    configured: Vec<Option<(isize, isize)>>,
    timer: slint::Timer,
}

impl WindowGroup {
    /// Registers weak application handles; no discovery or modification of external windows.
    pub fn watch<T: ComponentHandle + 'static>(
        owner: &SharedGroup,
        component: &T,
        events: &crate::window_events::WindowEvents,
    ) {
        let mut group = owner.borrow_mut();
        if group.members.len() >= 15 {
            eprintln!("warn: application window group is full");
            return;
        }
        let weak = component.as_weak();
        if group.main.is_none() {
            group.main = Some(Box::new(move || {
                weak.upgrade()?.window().with_winit_window(handle).flatten()
            }));
        } else {
            group.members.push(Box::new(move |leader| {
                let component = weak.upgrade()?;
                component
                    .window()
                    .with_winit_window(|native| {
                        let child = handle(native)?;
                        if leader != 0 {
                            use winit::platform::windows::WindowExtWindows;
                            // SAFETY: these two handles belong to live registered top-level components on this UI thread.
                            let failed = unsafe {
                                SetLastError(0);
                                SetWindowLongPtrW(child as _, GWLP_HWNDPARENT, leader) == 0
                                    && GetLastError() != 0
                            };
                            if failed {
                                eprintln!("warn: application window grouping unavailable");
                            }
                            native.set_skip_taskbar(true);
                        }
                        Some(child)
                    })
                    .flatten()
            }));
            group.configured.push(None);
        }
        drop(group);
        let weak = Rc::downgrade(owner);
        events.listen(move |_, event| {
            if !matches!(
                event,
                winit::event::WindowEvent::RedrawRequested | winit::event::WindowEvent::Focused(_)
            ) {
                return;
            }
            let Some(owner) = weak.upgrade() else {
                return;
            };
            let weak = Rc::downgrade(&owner);
            owner.borrow().timer.start(
                slint::TimerMode::SingleShot,
                std::time::Duration::ZERO,
                move || {
                    if let Some(owner) = weak.upgrade() {
                        owner.borrow_mut().configure();
                    }
                },
            );
        });
    }

    /// Installs native ownership once for each live child/leader pair after toolkit event dispatch.
    fn configure(&mut self) {
        let Some(leader) = self.main.as_ref().and_then(|main| main()) else {
            return;
        };
        for (member, configured) in self.members.iter().zip(&mut self.configured) {
            let Some(child) = member(0) else {
                continue;
            };
            if *configured != Some((child, leader)) {
                member(leader);
                *configured = Some((child, leader));
            }
        }
    }
}

/// Borrows the HWND from the current component without extending its native lifetime.
fn handle(window: &winit::window::Window) -> Option<isize> {
    match window.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
        _ => None,
    }
}
