//! Direct title-bar movement for attached peers avoids the compositor snapping main to its follower.

use super::*;

pub(super) struct GroupDrag {
    pointer: (i32, i32),
    last_pointer: (i32, i32),
    members: Vec<(usize, Rect)>,
}

impl GroupDrag {
    /// Translates the whole original group by the pointer's total displacement, without rounding drift.
    fn targets(&self, pointer: (i32, i32)) -> Option<Vec<(usize, Rect)>> {
        let dx = i64::from(pointer.0) - i64::from(self.pointer.0);
        let dy = i64::from(pointer.1) - i64::from(self.pointer.1);
        let mut targets = Vec::with_capacity(self.members.len());
        for (index, frame) in &self.members {
            targets.push((*index, geometry::translate(*frame, dx, dy)?));
        }
        Some(targets)
    }
}

impl SnapGroup {
    /// Takes over only an attached, frameless X11 player; other gestures retain toolkit native drag.
    pub fn begin_drag(&self, main: &slint::Window) -> bool {
        self.end_drag();
        let Some(player) = self.player else {
            return false;
        };
        let identities: Vec<_> = self.peers.iter().map(|peer| (peer.identity)()).collect();
        if identities[player].is_none_or(|identity| identity.decorated) {
            return false;
        }
        self.prepare();
        let positioner = self.positioner.borrow();
        let Some(Ok(adapter)) = positioner.as_ref() else {
            return false;
        };
        let Ok(frames) = adapter.observe(&identities) else {
            return false;
        };
        let attached = geometry::connected(player, &frames);
        let mut members = Vec::new();
        for (index, frame) in frames.iter().enumerate() {
            if attached[index]
                && let Some(frame) = frame
            {
                members.push((index, *frame));
            }
        }
        if members.len() < 2 {
            return false;
        }
        let Some((pointer, _)) = main
            .with_winit_window(|native| adapter.pointer(native))
            .flatten()
        else {
            return false;
        };
        *self.drag.borrow_mut() = Some(GroupDrag {
            pointer,
            last_pointer: pointer,
            members,
        });
        true
    }

    /// Follows a captured title-bar gesture; release outside the surface and query failure end it safely.
    pub fn update_drag(&self, main: &slint::Window) {
        if self.drag.borrow().is_none() {
            return;
        }
        let pointer = {
            let positioner = self.positioner.borrow();
            let Some(Ok(adapter)) = positioner.as_ref() else {
                return;
            };
            main.with_winit_window(|native| adapter.pointer(native))
                .flatten()
        };
        match pointer {
            Some((pointer, true)) => self.drag_to(pointer),
            _ => self.end_drag(),
        }
    }

    /// Applies absolute pointer displacement to every still-eligible member in the captured group.
    fn drag_to(&self, pointer: (i32, i32)) {
        let targets = {
            let mut gesture = self.drag.borrow_mut();
            let Some(drag) = gesture.as_mut() else {
                return;
            };
            if drag.last_pointer == pointer {
                return;
            }
            drag.last_pointer = pointer;
            drag.targets(pointer)
        };
        let Some(targets) = targets else {
            self.end_drag();
            return;
        };
        if targets
            .iter()
            .any(|(index, _)| (self.peers[*index].identity)().is_none())
        {
            self.end_drag();
            return;
        }
        self.movement.borrow_mut().request_group(&targets);
        for (index, target) in targets {
            (self.peers[index].place)(self, target.x, target.y);
        }
    }

    /// Releases only local gesture state; the native pointer grab is owned by the toolkit's press.
    pub fn end_drag(&self) {
        self.drag.borrow_mut().take();
    }
}

/// Binds the shared header intents once, keeping the UI independent of native positioning details.
pub(crate) fn connect(main: &crate::PlayerWindow, group: &Rc<RefCell<SnapGroup>>) {
    let owner = Rc::downgrade(group);
    let weak = main.as_weak();
    main.on_begin_group_move(move || {
        let (Some(group), Some(main)) = (owner.upgrade(), weak.upgrade()) else {
            return false;
        };
        group.borrow().begin_drag(main.window())
    });
    let owner = Rc::downgrade(group);
    let weak = main.as_weak();
    main.on_update_group_move(move || {
        if let (Some(group), Some(main)) = (owner.upgrade(), weak.upgrade()) {
            group.borrow().update_drag(main.window());
        }
    });
    let owner = Rc::downgrade(group);
    main.on_end_group_move(move || {
        if let Some(group) = owner.upgrade() {
            group.borrow().end_drag();
        }
    });
}

#[cfg(test)]
mod tests;
