//! Owns the built-in theme chooser; every caller uses one shared selection action.

use crate::{PlayerWindow, ThemesWindow};
use slint::ComponentHandle;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Default)]
pub struct ThemeChooser {
    window: Option<ThemesWindow>,
    windows: crate::window_group::SharedGroup,
}

impl ThemeChooser {
    /// Lazily creates one reusable chooser, with the player's palette as authoritative state.
    fn show(&mut self, main: &PlayerWindow) -> Result<(), &'static str> {
        if self.window.is_none() {
            let window = ThemesWindow::new().map_err(|_| "Cannot create Themes window")?;
            let weak = main.as_weak();
            window.on_choose_theme(move |spectrum| {
                if let Some(main) = weak.upgrade() {
                    main.invoke_select_theme(spectrum);
                }
            });
            let weak = main.as_weak();
            let chooser = window.as_weak();
            window.window().on_close_requested(move || {
                if let (Some(main), Some(chooser)) = (weak.upgrade(), chooser.upgrade()) {
                    if chooser.hide().is_err() {
                        report(&main, "Cannot hide Themes window");
                    } else {
                        main.invoke_focus_themes();
                    }
                }
                slint::CloseRequestResponse::KeepWindowShown
            });
            let events = crate::window_events::WindowEvents::new(window.window());
            crate::window_group::WindowGroup::watch(&self.windows, &window, &events);
            self.window = Some(window);
        }
        let window = self.window.as_ref().ok_or("Cannot create Themes window")?;
        window.set_spectrum(main.get_spectrum());
        window.show().map_err(|_| "Cannot show Themes window")?;
        crate::window_snap::present(window.window());
        main.set_theme_error("".into());
        Ok(())
    }

    /// Releases the optional chooser during main-window shutdown.
    pub fn close(&mut self) {
        if let Some(window) = self.window.take()
            && window.hide().is_err()
        {
            eprintln!("warn: cannot hide Themes window during shutdown");
        }
    }

    /// Returns the existing chooser for compiled/native UI interaction checks.
    #[cfg(test)]
    pub(crate) fn test_window(&self) -> ThemesWindow {
        self.window.as_ref().expect("Themes opened").clone_strong()
    }
}

/// Reports safe lifecycle failures without exposing native diagnostics or changing audio state.
fn report(main: &PlayerWindow, message: &'static str) {
    eprintln!("warn: {message}");
    main.set_theme_error(message.into());
}

/// Connects menu and selection intents once; weak references avoid retaining closed windows.
pub fn connect(
    main: &PlayerWindow,
    windows: crate::window_group::SharedGroup,
) -> Rc<RefCell<ThemeChooser>> {
    let owner = Rc::new(RefCell::new(ThemeChooser {
        windows,
        ..Default::default()
    }));
    let weak = main.as_weak();
    let host = Rc::downgrade(&owner);
    main.on_open_themes(move || {
        if let (Some(main), Some(host)) = (weak.upgrade(), host.upgrade())
            && !main.get_closing()
            && !main.get_settings_loading()
            && let Err(error) = host.borrow_mut().show(&main)
        {
            report(&main, error);
        }
    });
    let weak = main.as_weak();
    let host = Rc::downgrade(&owner);
    main.on_select_theme(move |spectrum| {
        let Some(main) = weak.upgrade() else {
            return;
        };
        if main.get_closing() || main.get_settings_loading() {
            return;
        }
        main.set_spectrum(spectrum);
        if let Some(host) = host.upgrade()
            && let Some(window) = host.borrow().window.as_ref()
        {
            window.set_spectrum(spectrum);
        }
    });
    owner
}

#[cfg(test)]
mod tests;
