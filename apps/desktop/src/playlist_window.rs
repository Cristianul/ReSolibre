//! Owns one separate playlist window in this process, sharing the main session and actions.

use crate::{PlayerWindow, PlaylistWindow};
use slint::ComponentHandle;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[derive(Default)]
pub struct PlaylistHost {
    window: Option<PlaylistWindow>,
    visible: bool,
    ever_shown: bool,
    startup_positioned: bool,
    startup_sized: bool,
    reduce_motion: bool,
    placement_attempts: Cell<u8>,
    saved_position: Option<(i32, i32)>,
    snap_available: Cell<Option<bool>>,
    snapping: Rc<RefCell<crate::window_snap::SnapGroup>>,
    imports: Rc<RefCell<crate::file_import::Inbox>>,
    windows: crate::window_group::SharedGroup,
}

impl PlaylistHost {
    /// Defers startup placement until the main window has a compositor-assigned position.
    #[cfg(test)]
    pub fn ready_to_show(&self, main: &PlayerWindow) -> bool {
        self.snapping.borrow().ready_to_dock(main.window())
    }
    /// Creates the separate surface once, forwarding every queue action to the existing bindings.
    fn create(
        &mut self,
        main: &PlayerWindow,
        owner: &Rc<RefCell<Self>>,
    ) -> Result<(), &'static str> {
        if self.window.is_some() {
            return Ok(());
        }
        let window = new_window().map_err(|_| "Cannot create library window")?;
        let weak = main.as_weak();
        window.on_open_files(move || {
            if let Some(main) = weak.upgrade() {
                main.invoke_open_files();
            }
        });
        let weak = main.as_weak();
        window.on_analyze_track(move |id| {
            if let Some(main) = weak.upgrade() {
                main.invoke_analyze_track(id);
            }
        });
        let weak = main.as_weak();
        window.on_analyze_all(move || {
            if let Some(main) = weak.upgrade() {
                main.invoke_analyze_all();
            }
        });
        let weak = main.as_weak();
        window.on_cancel_analysis(move || {
            if let Some(main) = weak.upgrade() {
                main.invoke_cancel_library_analysis();
            }
        });
        let weak = main.as_weak();
        window.on_select_track(move |id| {
            if let Some(main) = weak.upgrade() {
                main.invoke_select_track(id);
            }
        });
        let weak = main.as_weak();
        window.on_remove_track(move |id| {
            if let Some(main) = weak.upgrade() {
                main.invoke_remove_track(id);
            }
        });
        let weak = main.as_weak();
        window.on_move_track(move |id, forward| {
            if let Some(main) = weak.upgrade() {
                main.invoke_move_track(id, forward);
            }
        });
        let weak = main.as_weak();
        window.on_reorder_track(move |id, target, after| {
            if let Some(main) = weak.upgrade() {
                main.invoke_reorder_track(id, target, after);
            }
        });
        let weak = main.as_weak();
        let host = Rc::downgrade(owner);
        window.window().on_close_requested(move || {
            hide(&weak, &host);
            slint::CloseRequestResponse::KeepWindowShown
        });
        let weak = main.as_weak();
        window.on_drop_choice(move |choice| {
            if let Some(main) = weak.upgrade() {
                main.invoke_drop_choice(choice);
            }
        });
        let events = crate::window_events::WindowEvents::new(window.window());
        let weak = window.as_weak();
        events.listen(move |_, event| {
            if matches!(
                event,
                slint::winit_030::winit::event::WindowEvent::Focused(false)
            ) && let Some(window) = weak.upgrade()
            {
                window.invoke_cancel_interaction();
            }
        });
        crate::window_group::WindowGroup::watch(&self.windows, &window, &events);
        crate::window_snap::SnapGroup::watch(
            &self.snapping,
            &window,
            crate::window_snap::Role::Auxiliary,
            &events,
        );
        crate::file_import::watch(&events, &self.imports, crate::file_import::Target::Playlist);
        self.window = Some(window);
        Ok(())
    }

    /// Builds and populates the initial Library while both native surfaces are still hidden.
    pub fn prepare_startup(
        &mut self,
        main: &PlayerWindow,
        owner: &Rc<RefCell<Self>>,
    ) -> Result<bool, &'static str> {
        use slint::winit_030::WinitWindowAccessor;
        self.create(main, owner)?;
        let window = self.window.as_ref().ok_or("Cannot create library window")?;
        self.refresh_content(main, window);
        if !main.window().has_winit_window() || !window.window().has_winit_window() {
            return Ok(false);
        }
        if !self.startup_sized {
            let height = main.invoke_window_height_for_eq(main.get_eq_expanded());
            main.window()
                .set_size(slint::LogicalSize::new(800.0, height));
            window.window().set_size(slint::LogicalSize::new(
                window.get_initial_width(),
                window.get_initial_height(),
            ));
            self.startup_sized = true;
            // Native DPI/resize events must settle before physical dock coordinates are derived.
            return Ok(false);
        }
        // Warm the second frame before mapping main, whose first draw is prepared by Slint.
        // Drop the bounded image immediately; no startup frame cache is retained.
        window
            .window()
            .take_snapshot()
            .map_err(|_| "Cannot prepare library frame")?;
        self.startup_positioned = self
            .snapping
            .borrow()
            .prepare_startup_pair(main.window(), window.window());
        Ok(true)
    }

    /// Shows the shared window, placing it to the right only on its first appearance.
    pub fn show(
        &mut self,
        main: &PlayerWindow,
        owner: &Rc<RefCell<Self>>,
    ) -> Result<(), &'static str> {
        if self.visible {
            return Ok(());
        }
        let first_show = !self.ever_shown;
        self.create(main, owner)?;
        self.visible = true;
        self.refresh(main);
        let window = self.window.as_ref().ok_or("Cannot create library window")?;
        if let Some(position) = self.saved_position {
            self.snapping
                .borrow()
                .prepare_show(window.window(), position);
        } else if first_show && !self.startup_positioned {
            let size =
                slint::LogicalSize::new(window.get_initial_width(), window.get_initial_height());
            // A hidden new host has not learned its native scale yet. Use the parent's monitor
            // scale when fitting the pair, then let normal native DPI events handle later moves.
            window
                .window()
                .set_size(size.to_physical(main.window().scale_factor()));
            self.snapping
                .borrow()
                .prepare_dock(main.window(), window.window());
        }
        if window.show().is_err() {
            self.visible = false;
            return Err("Cannot show library window");
        }
        self.ever_shown = true;
        crate::window_snap::present(window.window());
        if first_show || self.saved_position.is_some() {
            // Mapping and the compositor's initial placement are asynchronous.
            self.placement_attempts.set(30);
        }
        main.set_playlist_visible(true);
        main.set_playlist_requested(true);
        main.set_playlist_error("".into());
        Ok(())
    }

    /// Hides without destroying the window, preserving scroll, geometry and the shared queue.
    fn hide(&mut self, main: &PlayerWindow) -> Result<(), &'static str> {
        if !self.visible {
            return Ok(());
        }
        if let Some(window) = self.window.as_ref() {
            let position = self.snapping.borrow().position(window.window());
            window.invoke_cancel_interaction();
            window.set_motion_enabled(false);
            window.hide().map_err(|_| "Cannot hide library window")?;
            if self.placement_attempts.get() == 0 || self.saved_position.is_none() {
                self.saved_position = position.or(self.saved_position);
            }
        }
        if main.get_drop_playlist() {
            main.invoke_drop_choice(-1);
        }
        self.visible = false;
        self.placement_attempts.set(0);
        main.set_playlist_visible(false);
        main.set_playlist_requested(false);
        main.invoke_focus_playlist();
        main.set_playlist_error("".into());
        Ok(())
    }

    /// Mirrors already-confirmed presentation values; models are shared rather than copied.
    pub fn refresh(&self, main: &PlayerWindow) {
        if !self.visible {
            return;
        }
        let Some(window) = self.window.as_ref() else {
            return;
        };
        let attempts = self.placement_attempts.get();
        if attempts > 0 {
            let snapping = self.snapping.borrow();
            let placed = if let Some(position) = self.saved_position {
                snapping.restore_position(window.window(), position)
            } else {
                snapping.dock_right(main.window(), window.window())
            };
            self.placement_attempts
                .set(if placed { 0 } else { attempts - 1 });
            if !placed && attempts == 1 && self.snap_available.get() == Some(true) {
                eprintln!("warn: library placement unavailable");
                main.set_playlist_error("Move the library next to the player to snap it".into());
            }
        }
        self.refresh_content(main, window);
    }

    /// Shares the existing models with a hidden or visible host without copying the queue.
    fn refresh_content(&self, main: &PlayerWindow, window: &PlaylistWindow) {
        window.set_drop_menu(main.get_drop_menu() && main.get_drop_playlist());
        window.set_drop_x(main.get_drop_x());
        window.set_drop_y(main.get_drop_y());
        window.set_import_status(main.get_import_status());
        window.set_tracks(main.get_tracks());
        window.set_analysis_states(main.get_analysis_states());
        window.set_analysis_busy(main.get_analysis_busy());
        window.set_analysis_summary(main.get_analysis_summary());
        window.set_total_duration(main.get_library_duration());
        window.set_current(main.get_current());
        window.set_playing(main.get_playing());
        window.set_spectrum(main.get_spectrum());
        window.set_motion_enabled(!self.reduce_motion);
        window.set_picking(main.get_picking());
        window.set_closing(main.get_closing());
        window.set_error_message(main.get_error_message());
        if self.snap_available.get().is_none()
            && let Some(available) = crate::window_snap::supported(window.window())
        {
            self.snap_available.set(Some(available));
        }
    }

    /// Exposes the existing surface to compiled/native UI checks without constructing another host.
    #[cfg(test)]
    pub(crate) fn test_window(&self) -> PlaylistWindow {
        self.window
            .as_ref()
            .expect("playlist created")
            .clone_strong()
    }

    /// Closes this owned auxiliary surface when the main application starts shutting down.
    pub fn close(&mut self) {
        self.visible = false;
        if let Some(window) = self.window.take() {
            let _ = window.hide();
        }
    }
}

/// Creates a queue view with one stable-identity lookup used by all presentation gestures.
/// Keeping this beside host creation also gives component tests the production bindings.
pub(crate) fn new_window() -> Result<PlaylistWindow, slint::PlatformError> {
    let window = PlaylistWindow::new()?;
    crate::library_search::connect(&window);
    Ok(window)
}

/// Handles custom and OS close through the same hide action.
fn hide(main: &slint::Weak<PlayerWindow>, host: &std::rc::Weak<RefCell<PlaylistHost>>) {
    if let (Some(main), Some(host)) = (main.upgrade(), host.upgrade())
        && let Err(error) = host.borrow_mut().hide(&main)
    {
        report(&main, error);
    }
}

/// Reports a safe window failure once while retaining a usable main player.
fn report(main: &PlayerWindow, error: &'static str) {
    eprintln!("warn: {error}");
    main.set_playlist_error(error.into());
}

/// Connects host actions once; weak references avoid retaining closed controllers/windows.
pub fn connect(
    main: &PlayerWindow,
    imports: Rc<RefCell<crate::file_import::Inbox>>,
    windows: crate::window_group::SharedGroup,
) -> Rc<RefCell<PlaylistHost>> {
    let host = Rc::new(RefCell::new(PlaylistHost {
        reduce_motion: std::env::var_os("RESOUL_REDUCED_MOTION").is_some(),
        imports: imports.clone(),
        windows: windows.clone(),
        ..Default::default()
    }));
    let events = crate::window_events::WindowEvents::new(main.window());
    crate::window_group::WindowGroup::watch(&windows, main, &events);
    crate::window_snap::SnapGroup::watch(
        &host.borrow().snapping,
        main,
        crate::window_snap::Role::Player,
        &events,
    );
    crate::window_snap::drag::connect(main, &host.borrow().snapping);
    crate::file_import::watch(&events, &imports, crate::file_import::Target::Player);
    let weak = main.as_weak();
    let owner = Rc::downgrade(&host);
    main.on_toggle_playlist(move || {
        if let (Some(main), Some(host)) = (weak.upgrade(), owner.upgrade())
            && !main.get_closing()
            && !main.get_settings_loading()
        {
            let mut state = host.borrow_mut();
            let result = if state.visible {
                state.hide(&main)
            } else {
                state.show(&main, &host)
            };
            if let Err(error) = result {
                report(&main, error);
            }
        }
    });
    let weak = main.as_weak();
    let owner = Rc::downgrade(&host);
    main.on_reveal_current(move || {
        if let (Some(main), Some(host)) = (weak.upgrade(), owner.upgrade())
            && !main.get_closing()
            && !main.get_settings_loading()
        {
            let mut state = host.borrow_mut();
            let already_visible = state.visible;
            if let Err(error) = state.show(&main, &host) {
                report(&main, error);
                return;
            }
            // A newly mapped window must wait for the regular refresh before checking placement.
            // An immediate check can see its old position just before the WM recenters it.
            if already_visible {
                state.refresh(&main);
            }
            if let Some(window) = state.window.as_ref() {
                crate::window_snap::present(window.window());
                window.invoke_reveal_current();
            }
        }
    });
    host
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod interaction_tests;

#[cfg(test)]
mod placement_tests;
