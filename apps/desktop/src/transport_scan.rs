//! Maps shared hold gestures onto generation-scoped session actions.

use crate::{PlayerWindow, presentation::Controller, submit};
use resolibre_audio::session::Command;
use resolibre_core::session::ScanDirection;
use slint::ComponentHandle;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

/// Captures the track on pointer-down, so a delayed hold cannot scan a replacement track.
/// Cancellation is deferred: disabling a control can happen inside a borrowed refresh.
pub fn connect(window: &PlayerWindow, controller: &Rc<RefCell<Controller>>) {
    let generation = Rc::new(Cell::new(None));
    let captured = Rc::clone(&generation);
    let controls = Rc::clone(controller);
    window.on_scan_pressed(move || captured.set(Some(controls.borrow().generation)));

    let captured = Rc::clone(&generation);
    let controls = Rc::clone(controller);
    let weak = window.as_weak();
    window.on_scan_started(move |forward| {
        let (Some(window), Some(generation)) = (weak.upgrade(), captured.get()) else {
            return;
        };
        let direction = if forward {
            ScanDirection::Forward
        } else {
            ScanDirection::Backward
        };
        submit(
            &window,
            &controls,
            Command::Scan {
                generation,
                direction: Some(direction),
            },
        );
    });

    let weak = window.as_weak();
    let controls = Rc::clone(controller);
    window.on_scan_ended(move || {
        let Some(generation) = generation.take() else {
            return;
        };
        let weak = weak.clone();
        let controls = Rc::clone(&controls);
        slint::Timer::single_shot(Duration::ZERO, move || {
            if let Some(window) = weak.upgrade() {
                submit(
                    &window,
                    &controls,
                    Command::Scan {
                        generation,
                        direction: None,
                    },
                );
            }
        });
    });
}
