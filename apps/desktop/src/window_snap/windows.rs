//! Win32 positioning of explicitly registered application windows, sharing the existing snap policy.

use super::geometry::Rect;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use slint::winit_030::winit;
use windows_sys::Win32::Foundation::{HWND, POINT, RECT};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetWindowRect, IsIconic, IsZoomed,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct Identity {
    handle: HWND,
    pub(super) decorated: bool,
}

pub(super) struct Positioner;

/// Borrows only a handle from the current live component; never discovers external windows.
pub(super) fn identity(native: &winit::window::Window) -> Option<Identity> {
    let handle = native.window_handle().ok()?;
    match handle.as_raw() {
        RawWindowHandle::Win32(handle) => Some(Identity {
            handle: handle.hwnd.get() as HWND,
            decorated: native.is_decorated(),
        }),
        _ => None,
    }
}

impl Positioner {
    /// Win32 positioning needs no external display connection.
    pub(super) fn connect() -> Result<Self, &'static str> {
        Ok(Self)
    }

    /// Reads cursor/button state only while handling an owned title-bar drag.
    pub(super) fn pointer(&self, native: &winit::window::Window) -> Option<((i32, i32), bool)> {
        identity(native)?;
        let mut point = POINT::default();
        // SAFETY: the output points to live storage; querying button state retains no references.
        unsafe {
            if GetCursorPos(&mut point) == 0 {
                return None;
            }
            Some((
                (point.x, point.y),
                GetAsyncKeyState(i32::from(VK_LBUTTON)) < 0,
            ))
        }
    }

    /// Positions the owned outer frame without changing focus or size.
    pub(super) fn place(
        &self,
        native: &winit::window::Window,
        x: i32,
        y: i32,
    ) -> Result<(), &'static str> {
        identity(native).ok_or("Window positioning unavailable")?;
        native.set_outer_position(winit::dpi::PhysicalPosition::new(x, y));
        Ok(())
    }

    /// Applies the same physical coordinates before showing a hidden auxiliary window.
    pub(super) fn prepare_show(
        &self,
        native: &winit::window::Window,
        x: i32,
        y: i32,
    ) -> Result<(), &'static str> {
        self.place(native, x, y)
    }

    /// Reports whether the toolkit has made this owned surface visible.
    #[cfg(test)]
    pub(super) fn mapped(&self, native: &winit::window::Window) -> bool {
        native.is_visible() == Some(true)
    }

    /// Captures geometry for at most the sixteen explicitly registered peers.
    pub(super) fn observe(
        &self,
        windows: &[Option<Identity>],
    ) -> Result<Vec<Option<Rect>>, &'static str> {
        Ok(windows
            .iter()
            .map(|window| window.and_then(frame))
            .collect())
    }

    /// Reads the physical outer rectangle, including decorations.
    pub(super) fn bounds(&self, native: &winit::window::Window) -> Option<Rect> {
        frame(identity(native)?)
    }
}

/// Excludes minimized/maximized handles before sharing observations with portable movement policy.
fn frame(window: Identity) -> Option<Rect> {
    let mut rectangle = RECT::default();
    // SAFETY: handles originate from live registered components and Win32 validates handles on each call.
    unsafe {
        if IsIconic(window.handle) != 0
            || IsZoomed(window.handle) != 0
            || GetWindowRect(window.handle, &mut rectangle) == 0
        {
            return None;
        }
    }
    let width = u32::try_from(rectangle.right.checked_sub(rectangle.left)?).ok()?;
    let height = u32::try_from(rectangle.bottom.checked_sub(rectangle.top)?).ok()?;
    if width == 0 || height == 0 {
        return None;
    }
    Some(Rect {
        x: rectangle.left,
        y: rectangle.top,
        width,
        height,
    })
}
