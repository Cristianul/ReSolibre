//! Converts the bounded thumbnail for the track currently shown by the transport.

use crate::PlayerWindow;
use resolibre_audio::metadata::{COVER_SIDE, Cover};
use slint::{Image, Rgba8Pixel, SharedPixelBuffer};

/// Keeps the last displayed image during a new probe, replacing it only with a definitive result.
pub(crate) fn update(window: &PlayerWindow, snapshot: &resolibre_audio::session::Snapshot) {
    if snapshot.display_next && snapshot.audible_next.is_some() {
        window.set_cover_loading(snapshot.next_cover_pending);
        show(window, snapshot.next_cover.as_deref());
    } else {
        window.set_cover_loading(snapshot.cover_pending);
        if !snapshot.cover_pending {
            show(window, snapshot.cover.as_deref());
        }
    }
}

/// Replaces or clears the cover in one update; defensive validation prevents malformed public snapshots.
pub(crate) fn show(window: &PlayerWindow, cover: Option<&Cover>) {
    let Some(cover) =
        cover.filter(|cover| cover.rgba.len() == COVER_SIDE as usize * COVER_SIDE as usize * 4)
    else {
        window.set_has_cover(false);
        window.set_cover_image(Image::default());
        return;
    };
    let pixels =
        SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&cover.rgba, COVER_SIDE, COVER_SIDE);
    window.set_cover_image(Image::from_rgba8(pixels));
    window.set_has_cover(true);
}

#[cfg(test)]
mod tests {
    use super::*;
    use i_slint_backend_testing::ElementHandle;
    use slint::ComponentHandle;

    /// Thumbnail changes preserve square geometry; absent and malformed buffers release the old image.
    #[test]
    fn artwork_updates_and_falls_back_safely() {
        i_slint_backend_testing::init_no_event_loop();
        let window = PlayerWindow::new().unwrap();
        window.show().unwrap();
        let cover = Cover {
            rgba: vec![255; COVER_SIDE as usize * COVER_SIDE as usize * 4].into(),
        };
        show(&window, Some(&cover));
        assert!(window.get_has_cover());
        assert_eq!(window.get_cover_image().size().width, COVER_SIDE);
        let view = ElementHandle::find_by_accessible_label(&window, "Album cover")
            .next()
            .unwrap();
        assert_eq!(view.size().width, view.size().height);
        show(&window, None);
        assert!(!window.get_has_cover());
        assert_eq!(window.get_cover_image().size().width, 0);
        show(
            &window,
            Some(&Cover {
                rgba: vec![0; 3].into(),
            }),
        );
        assert!(!window.get_has_cover());
    }

    /// Rapid track changes keep the displayed image while pending, then commit cover/missing-art atomically.
    #[test]
    fn pending_cover_never_flashes_the_fallback() {
        i_slint_backend_testing::init_no_event_loop();
        let window = PlayerWindow::new().unwrap();
        let cover = Cover {
            rgba: vec![255; COVER_SIDE as usize * COVER_SIDE as usize * 4].into(),
        };
        show(&window, Some(&cover));
        let mut snapshot = resolibre_audio::session::Snapshot {
            cover_pending: true,
            ..Default::default()
        };
        update(&window, &snapshot);
        assert!(window.get_has_cover() && window.get_cover_loading());
        assert_eq!(window.get_cover_image().size().width, COVER_SIDE);
        snapshot.generation += 1;
        update(&window, &snapshot);
        assert!(window.get_has_cover());
        snapshot.cover_pending = false;
        snapshot.cover = Some(std::sync::Arc::new(cover));
        update(&window, &snapshot);
        assert!(window.get_has_cover() && !window.get_cover_loading());
        snapshot.cover_pending = true;
        snapshot.cover = None;
        update(&window, &snapshot);
        assert!(window.get_has_cover());
        snapshot.cover_pending = false;
        update(&window, &snapshot);
        assert!(!window.get_has_cover() && !window.get_cover_loading());
    }

    /// Next focus displays the incoming artwork and never mislabels the outgoing cover.
    #[test]
    fn incoming_focus_uses_its_own_cover() {
        i_slint_backend_testing::init_no_event_loop();
        let window = PlayerWindow::new().unwrap();
        let outgoing = Cover {
            rgba: vec![255; COVER_SIDE as usize * COVER_SIDE as usize * 4].into(),
        };
        let incoming = Cover {
            rgba: vec![40; COVER_SIDE as usize * COVER_SIDE as usize * 4].into(),
        };
        let mut snapshot = resolibre_audio::session::Snapshot {
            cover: Some(std::sync::Arc::new(outgoing)),
            next_cover_pending: true,
            audible_next: Some(resolibre_audio::session::AudibleNext {
                index: 1,
                position: 2.0,
                duration: 10.0,
                gain: 0.3,
                mix_region: None,
                seekable: true,
                stream_info: None,
            }),
            display_next: true,
            ..Default::default()
        };
        update(&window, &snapshot);
        assert!(!window.get_has_cover());
        assert!(window.get_cover_loading());
        snapshot.next_cover = Some(std::sync::Arc::new(incoming));
        snapshot.next_cover_pending = false;
        update(&window, &snapshot);
        assert!(window.get_has_cover());
        assert!(!window.get_cover_loading());
    }
}
