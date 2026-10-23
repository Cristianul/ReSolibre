//! Bounded shuffle traversal. IDs refer to occurrences; source data never enters this module.

use super::MAX_QUEUE_ITEMS;
use std::collections::VecDeque;

/// Repeat policy shared by all controls; One applies only to natural EOF.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RepeatMode {
    #[default]
    Off,
    All,
    One,
}

impl RepeatMode {
    /// Cycles the UI action without defining another copy of repeat policy in presentation.
    pub fn cycle(self) -> Self {
        match self {
            Self::Off => Self::All,
            Self::All => Self::One,
            Self::One => Self::Off,
        }
    }
}

/// Distinguishes explicit navigation from natural completion and repeat-one behavior.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Advance {
    Next,
    Previous,
    Ended,
}

#[derive(Clone, Debug)]
pub(super) struct ShuffleOrder {
    remaining: VecDeque<u64>,
    history: VecDeque<u64>,
    cursor: usize,
    random_state: u64,
}

impl ShuffleOrder {
    /// Accepts a reproducible non-security seed; zero is mapped away from the fixed point.
    pub(super) fn new(seed: u64) -> Self {
        Self {
            remaining: VecDeque::new(),
            history: VecDeque::new(),
            cursor: 0,
            random_state: if seed == 0 { 1 } else { seed },
        }
    }

    /// Drops traversal memory without reseeding; disabling Shuffle preserves queue/audio state.
    pub(super) fn clear(&mut self) {
        self.remaining.clear();
        self.history.clear();
        self.cursor = 0;
    }

    /// Starts a pass with the current occurrence already visited, without changing playback.
    pub(super) fn reset(&mut self, ids: &[u64], current: Option<u64>) {
        self.clear();
        let mut pending: Vec<_> = ids
            .iter()
            .copied()
            .filter(|id| Some(*id) != current)
            .collect();
        self.shuffle(&mut pending);
        self.remaining = pending.into();
        if let Some(id) = current {
            self.history.push_back(id);
        }
    }

    /// Adds new work at a pseudo-random position, preserving the relative existing traversal.
    pub(super) fn append(&mut self, id: u64) {
        let position = self.random_index(self.remaining.len() + 1);
        self.remaining.insert(position, id);
    }

    /// Prunes a removed occurrence from pending work and all retained history visits.
    pub(super) fn remove(&mut self, id: u64) {
        let retained_prefix = self
            .history
            .iter()
            .take(self.cursor + 1)
            .filter(|entry| **entry != id)
            .count();
        self.remaining.retain(|entry| *entry != id);
        self.history.retain(|entry| *entry != id);
        self.cursor = retained_prefix.saturating_sub(1);
    }

    /// Records an explicit/new selection, discarding forward history and bounding retained visits.
    pub(super) fn select(&mut self, id: u64) {
        self.remaining.retain(|entry| *entry != id);
        self.history.truncate(self.cursor + 1);
        if self.history.back() == Some(&id) {
            return;
        }
        if self.history.len() == MAX_QUEUE_ITEMS {
            self.history.pop_front();
        }
        self.history.push_back(id);
        self.cursor = self.history.len() - 1;
    }

    /// Reports availability without consuming randomness or advancing history.
    pub(super) fn can_next(&self, repeat_all: bool, has_entries: bool) -> bool {
        self.cursor + 1 < self.history.len()
            || !self.remaining.is_empty()
            || (repeat_all && has_entries)
    }

    /// Previous traverses actual retained history; it never invents a wrapped shuffle predecessor.
    pub(super) fn can_previous(&self) -> bool {
        self.cursor > 0
    }

    /// Moves back one recorded visit; Next can then replay the forward branch.
    pub(super) fn previous(&mut self) -> Option<u64> {
        if !self.can_previous() {
            return None;
        }
        self.cursor -= 1;
        self.history.get(self.cursor).copied()
    }

    /// Replays forward history before drawing an unvisited occurrence or starting Repeat All.
    pub(super) fn next(
        &mut self,
        ids: &[u64],
        current: Option<u64>,
        repeat_all: bool,
    ) -> Option<u64> {
        if self.cursor + 1 < self.history.len() {
            self.cursor += 1;
            return self.history.get(self.cursor).copied();
        }
        if self.remaining.is_empty() && repeat_all {
            let mut pending = ids.to_vec();
            self.shuffle(&mut pending);
            if pending.len() > 1 && pending.first().copied() == current {
                // All occurrences still appear once in the new pass, but not the last one first.
                let replacement = 1 + self.random_index(pending.len() - 1);
                pending.swap(0, replacement);
            }
            self.remaining = pending.into();
        }
        let id = self.remaining.pop_front()?;
        self.select(id);
        Some(id)
    }

    /// Permutes a bounded pass in place using a deterministic Fisher-Yates traversal.
    fn shuffle(&mut self, ids: &mut [u64]) {
        for index in (1..ids.len()).rev() {
            let selected = self.random_index(index + 1);
            ids.swap(index, selected);
        }
    }

    /// Generates a bounded index for music order only, never keys, tokens or other security uses.
    fn random_index(&mut self, upper: usize) -> usize {
        let mut value = self.random_state;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        self.random_state = value;
        (value % upper as u64) as usize
    }
}
