//! Bounded move bookkeeping shared by every registered window; no native handles or audio state.

use super::geometry::{self, Rect};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

#[derive(Default)]
struct TrackedWindow {
    frame: Option<Rect>,
    last_observed: Option<Rect>,
    pending: VecDeque<(i32, i32)>,
    requested_at: Option<Instant>,
}

impl TrackedWindow {
    /// Consumes recent programmatic positions without treating them as independent user moves.
    /// History expires so a denied WM request cannot mask a later move to that coordinate.
    fn acknowledge(&mut self, actual: Rect) -> bool {
        let recent = self
            .requested_at
            .is_some_and(|time| time.elapsed() < Duration::from_millis(500));
        // Several native notifications can read the same server position. A duplicate must
        // not replace the newer destination already requested while the follower catches up.
        if recent && self.last_observed == Some(actual) {
            return true;
        }
        self.last_observed = Some(actual);
        if recent
            && let Some(index) = self
                .pending
                .iter()
                .position(|point| *point == (actual.x, actual.y))
        {
            self.pending.drain(..=index);
            if let Some(frame) = self.frame.as_mut() {
                frame.width = actual.width;
                frame.height = actual.height;
            }
            return true;
        }
        self.pending.clear();
        false
    }
}

#[derive(Default)]
pub(super) struct Movement {
    windows: Vec<TrackedWindow>,
}

impl Movement {
    /// Records every direct group destination before dispatch so native acknowledgements cannot snap it back.
    pub(super) fn request_group(&mut self, targets: &[(usize, Rect)]) {
        for (index, frame) in targets {
            self.record_target(*index, *frame);
        }
    }

    /// Adds bookkeeping for an explicitly registered peer; registration owns the 16-window bound.
    pub(super) fn register(&mut self) {
        self.windows.push(TrackedWindow::default());
    }

    /// Applies one observed move or resize and returns owned-window placement requests.
    /// The player carries its connected peers; independently moved auxiliaries only snap themselves.
    pub(super) fn update(
        &mut self,
        moving: usize,
        player: Option<usize>,
        observed: &[Option<Rect>],
    ) -> Vec<(usize, Rect)> {
        for (window, actual) in self.windows.iter_mut().zip(observed) {
            if actual.is_none() {
                window.frame = None;
                window.last_observed = None;
                window.pending.clear();
            } else if window.frame.is_none() {
                window.frame = *actual;
            }
        }
        let Some(actual) = observed[moving] else {
            return Vec::new();
        };
        let window = &mut self.windows[moving];
        if window.acknowledge(actual) {
            return Vec::new();
        }
        let previous = window.frame.unwrap_or(actual);
        let frames: Vec<_> = self.windows.iter().map(|window| window.frame).collect();
        let attached = if player == Some(moving) {
            geometry::connected(moving, &frames)
        } else {
            vec![false; frames.len()]
        };
        let anchors: Vec<_> = observed
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != moving && !attached[*index])
            .filter_map(|(_, frame)| *frame)
            .collect();
        let mut destination = actual;
        if let Some((x, y)) = geometry::snap(actual, &anchors) {
            destination.x = x;
            destination.y = y;
        }
        self.windows[moving].frame = Some(destination);
        let mut requests = Vec::new();
        if destination != actual {
            self.request(moving, destination, &mut requests);
        }
        let dx = i64::from(destination.x) - i64::from(previous.x);
        let dy = i64::from(destination.y) - i64::from(previous.y);
        if dx == 0 && dy == 0 {
            return requests;
        }
        for (index, frame) in frames.into_iter().enumerate() {
            if index == moving || !attached[index] {
                continue;
            }
            if let Some(target) = frame.and_then(|frame| geometry::translate(frame, dx, dy)) {
                self.request(index, target, &mut requests);
            }
        }
        requests
    }

    /// Records the intended geometry before dispatch, with a bounded history for coalesced events.
    fn request(&mut self, index: usize, frame: Rect, requests: &mut Vec<(usize, Rect)>) {
        self.record_target(index, frame);
        requests.push((index, frame));
    }

    /// Shares bounded acknowledgement tracking between native snapping and direct pointer gestures.
    fn record_target(&mut self, index: usize, frame: Rect) {
        let window = &mut self.windows[index];
        window.frame = Some(frame);
        if window.pending.len() == 32 {
            window.pending.pop_front();
        }
        window.pending.push_back((frame.x, frame.y));
        window.requested_at = Some(Instant::now());
    }
}

#[cfg(test)]
mod tests;
