//! Persistent interface preferences; independent of portable playback state.

use crate::PlayerWindow;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InterfaceState {
    pub spectrum: bool,
    pub equalizer_visible: bool,
    pub library_visible: bool,
}

impl Default for InterfaceState {
    /// Older profiles retain the original Green theme and both initially visible panels.
    fn default() -> Self {
        Self {
            spectrum: false,
            equalizer_visible: true,
            library_visible: true,
        }
    }
}

impl InterfaceState {
    /// Captures visibility intent rather than temporary native hiding during startup/shutdown.
    pub fn capture(window: &PlayerWindow) -> Self {
        Self {
            spectrum: window.get_spectrum(),
            equalizer_visible: window.get_eq_expanded(),
            library_visible: window.get_playlist_requested(),
        }
    }

    /// Restores preferences before startup placement; uses the existing EQ resize owner.
    pub fn apply(self, window: &PlayerWindow) {
        window.set_spectrum(self.spectrum);
        window.set_playlist_requested(self.library_visible);
        window.set_eq_expanded(self.equalizer_visible);
        window.invoke_eq_visibility_changed(self.equalizer_visible);
    }
}
