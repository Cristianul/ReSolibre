//! One native event hook per window, shared by activation, snapping and local file drops.

use slint::winit_030::{EventResult, WinitWindowAccessor, winit};
use std::cell::RefCell;
use std::rc::Rc;

type Listener = Box<dyn FnMut(&slint::Window, &winit::event::WindowEvent)>;

pub struct WindowEvents(Rc<RefCell<Vec<Listener>>>);

impl WindowEvents {
    /// Installs the toolkit's single hook; listeners use weak owners and never consume toolkit input.
    pub fn new(window: &slint::Window) -> Self {
        let listeners = Rc::new(RefCell::new(Vec::<Listener>::new()));
        let observed = listeners.clone();
        window.on_winit_window_event(move |window, event| {
            for listener in observed.borrow_mut().iter_mut() {
                listener(window, event);
            }
            EventResult::Propagate
        });
        Self(listeners)
    }

    /// Registers a feature listener during window composition, before event delivery begins.
    pub fn listen(
        &self,
        listener: impl FnMut(&slint::Window, &winit::event::WindowEvent) + 'static,
    ) {
        self.0.borrow_mut().push(Box::new(listener));
    }
}
