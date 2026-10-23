//! Reusable temporary-queue policy; consumers dispatch actions by occurrence identity.

mod order;
use order::ShuffleOrder;
pub use order::{Advance, RepeatMode};

pub const MAX_QUEUE_ITEMS: usize = 1_024;

/// One action vocabulary for UI, tests and future keyboard/remote consumers.
pub enum QueueAction {
    Select(u64),
    Remove(u64),
    Move { id: u64, forward: bool },
    MoveTo { id: u64, target: u64, after: bool },
    SetShuffle(bool),
    SetRepeat(RepeatMode),
    ToggleShuffle,
    CycleRepeat,
}

/// An occurrence retains identity even when the same source appears twice.
#[derive(Clone, Debug)]
pub struct QueueEntry<T> {
    pub id: u64,
    pub source: T,
}

/// Owns order and selection; the source adapter supplies its own payload type.
#[derive(Clone, Debug)]
pub struct Queue<T> {
    entries: Vec<QueueEntry<T>>,
    current: Option<usize>,
    next_id: u64,
    shuffle: bool,
    repeat: RepeatMode,
    order: ShuffleOrder,
}

impl<T> Default for Queue<T> {
    /// Creates an empty session whose occurrence IDs are never path-derived.
    fn default() -> Self {
        Self::with_seed(1)
    }
}

impl<T> Queue<T> {
    /// Creates a portable queue with reproducible shuffle; the runtime supplies a fresh seed.
    pub fn with_seed(seed: u64) -> Self {
        Self {
            entries: Vec::new(),
            current: None,
            next_id: 1,
            shuffle: false,
            repeat: RepeatMode::Off,
            order: ShuffleOrder::new(seed),
        }
    }

    /// Adds a whole bounded batch or rejects it without changing the queue.
    pub fn append(&mut self, sources: Vec<T>) -> bool {
        if sources.len() > MAX_QUEUE_ITEMS.saturating_sub(self.entries.len()) {
            return false;
        }
        let Some(next_id) = self.next_id.checked_add(sources.len() as u64) else {
            return false;
        };
        for source in sources {
            self.entries.push(QueueEntry {
                id: self.next_id,
                source,
            });
            if self.shuffle {
                self.order.append(self.next_id);
            }
            self.next_id += 1;
        }
        self.next_id = next_id;
        true
    }

    /// Replaces a bounded batch atomically, preserving modes and never reusing occurrence IDs.
    pub fn replace(&mut self, sources: Vec<T>) -> bool {
        if sources.is_empty()
            || sources.len() > MAX_QUEUE_ITEMS
            || self.next_id.checked_add(sources.len() as u64).is_none()
        {
            return false;
        }

        self.entries.clear();
        self.current = None;
        self.order.clear();
        self.append(sources);
        self.select(0);
        true
    }

    /// Reads ordered occurrences without allowing an independent external queue.
    pub fn entries(&self) -> &[QueueEntry<T>] {
        &self.entries
    }

    /// Returns the selected index, even while stopped, ended or failed.
    pub fn current(&self) -> Option<usize> {
        self.current
    }

    /// Selects only an existing occurrence; invalid requests preserve selection.
    pub fn select(&mut self, index: usize) -> bool {
        if index >= self.entries.len() {
            return false;
        }
        if self.shuffle && self.current != Some(index) {
            self.order.select(self.entries[index].id);
        }
        self.current = Some(index);
        true
    }

    /// Resolves a stable occurrence, rejecting stale IDs rather than selecting another row.
    pub fn index_of(&self, id: u64) -> Option<usize> {
        self.entries.iter().position(|entry| entry.id == id)
    }

    /// Returns the selected occurrence identity independently of its current position.
    pub fn current_id(&self) -> Option<u64> {
        self.current.map(|index| self.entries[index].id)
    }

    /// Removes only a queue occurrence, never its source. Selects the following item
    /// (or the last remaining item) when removing the current one; does not start audio.
    pub fn remove(&mut self, id: u64) -> bool {
        let Some(index) = self.index_of(id) else {
            return false;
        };
        let selected = self.current_id();
        self.entries.remove(index);
        self.order.remove(id);
        if selected == Some(id) {
            self.current = if self.entries.is_empty() {
                None
            } else {
                Some(index.min(self.entries.len() - 1))
            };
        } else {
            self.current = selected.and_then(|id| self.index_of(id));
        }
        if self.shuffle && selected == Some(id) {
            self.reset_order();
        }
        true
    }

    /// Moves an occurrence one position, preserving the selected identity and bounds.
    pub fn move_adjacent(&mut self, id: u64, forward: bool) -> bool {
        let Some(index) = self.index_of(id) else {
            return false;
        };
        let neighbour = if forward {
            index.checked_add(1)
        } else {
            index.checked_sub(1)
        };
        let Some(neighbour) = neighbour.filter(|next| *next < self.entries.len()) else {
            return false;
        };
        let selected = self.current_id();
        self.entries.swap(index, neighbour);
        self.current = selected.and_then(|id| self.index_of(id));
        true
    }

    /// Relocates one occurrence relative to another, preserving playback and shuffle history.
    /// Stale identities and self-drops are rejected without mutation.
    pub fn move_to(&mut self, id: u64, target: u64, after: bool) -> bool {
        let (Some(source), Some(destination)) = (self.index_of(id), self.index_of(target)) else {
            return false;
        };
        if source == destination {
            return false;
        }
        let insertion = destination + usize::from(after);
        let insertion = if source < insertion {
            insertion - 1
        } else {
            insertion
        };
        if insertion == source {
            return false;
        }
        let current = self.current_id();
        let entry = self.entries.remove(source);
        self.entries.insert(insertion, entry);
        self.current = current.and_then(|id| self.index_of(id));
        true
    }

    /// Returns confirmed Shuffle mode; the visible queue order remains unchanged.
    pub fn shuffle(&self) -> bool {
        self.shuffle
    }

    /// Returns confirmed repeat policy independently of playback state.
    pub fn repeat(&self) -> RepeatMode {
        self.repeat
    }

    /// Changes traversal mode without changing current selection; repeated sets are idempotent.
    pub fn set_shuffle(&mut self, enabled: bool) {
        if self.shuffle == enabled {
            return;
        }
        self.shuffle = enabled;
        if enabled {
            self.reset_order();
        } else {
            self.order.clear();
        }
    }

    /// Changes repeat behavior without resetting pending shuffle work or history.
    pub fn set_repeat(&mut self, mode: RepeatMode) {
        self.repeat = mode;
    }

    /// Reports navigation from the same policy used by native execution, without mutation.
    pub fn can_advance(&self, reason: Advance) -> bool {
        if reason == Advance::Ended && self.repeat == RepeatMode::One {
            return self.current.is_some();
        }
        if self.shuffle {
            if reason == Advance::Previous {
                return self.order.can_previous();
            }
            return self
                .order
                .can_next(self.repeat == RepeatMode::All, !self.entries.is_empty());
        }
        self.ordered_target(reason).is_some()
    }

    /// Selects the next policy target; natural EOF alone honors Repeat One.
    pub fn advance(&mut self, reason: Advance) -> Option<usize> {
        if reason == Advance::Ended && self.repeat == RepeatMode::One {
            return self.current;
        }
        let target = if self.shuffle {
            let id = if reason == Advance::Previous {
                self.order.previous()?
            } else {
                self.order.next(
                    &self.ids(),
                    self.current_id(),
                    self.repeat == RepeatMode::All,
                )?
            };
            self.index_of(id)?
        } else {
            self.ordered_target(reason)?
        };
        self.current = Some(target);
        Some(target)
    }

    /// Resolves ordered navigation and Repeat All wrapping, including a one-item queue.
    fn ordered_target(&self, reason: Advance) -> Option<usize> {
        if self.entries.is_empty() {
            return None;
        }
        let Some(index) = self.current else {
            return if reason == Advance::Previous {
                None
            } else {
                Some(0)
            };
        };
        if reason == Advance::Previous {
            if index > 0 {
                return Some(index - 1);
            }
            if self.repeat == RepeatMode::All {
                return Some(self.entries.len() - 1);
            }
        } else {
            if index + 1 < self.entries.len() {
                return Some(index + 1);
            }
            if self.repeat == RepeatMode::All {
                return Some(0);
            }
        }
        None
    }

    /// Collects only bounded occurrence IDs; source data stays out of traversal state.
    fn ids(&self) -> Vec<u64> {
        self.entries.iter().map(|entry| entry.id).collect()
    }

    /// Anchors a fresh shuffle pass at the current occurrence after mode/current removal changes.
    fn reset_order(&mut self) {
        self.order.reset(&self.ids(), self.current_id());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Repeated sources retain distinct identities and navigation never wraps.
    #[test]
    fn repeated_entries_and_boundaries() {
        let mut queue = Queue::default();
        assert!(!queue.can_advance(Advance::Next));
        assert!(queue.append(vec!["same", "same"]));
        assert_ne!(queue.entries()[0].id, queue.entries()[1].id);
        assert!(queue.select(0));
        assert!(!queue.can_advance(Advance::Previous));
        assert!(queue.can_advance(Advance::Next));
        assert!(!queue.select(3));
        assert_eq!(queue.current(), Some(0));
        assert!(queue.select(1));
        assert!(!queue.can_advance(Advance::Next));
    }

    /// Oversized batches are atomic, preserving selection and existing IDs.
    #[test]
    fn refuses_queue_overflow() {
        let mut queue = Queue::default();
        assert!(queue.append(vec![0; MAX_QUEUE_ITEMS]));
        assert!(queue.select(7));
        assert!(!queue.append(vec![1]));
        assert_eq!(queue.entries().len(), MAX_QUEUE_ITEMS);
        assert_eq!(queue.current(), Some(7));
    }
    /// Editing duplicate sources preserves selected identity; stale IDs never hit neighbours.
    #[test]
    fn edits_preserve_identity_and_reject_stale_occurrences() {
        let mut queue = Queue::default();
        assert!(queue.append(vec!["same", "same", "third"]));
        let first = queue.entries()[0].id;
        let second = queue.entries()[1].id;
        let third = queue.entries()[2].id;
        assert!(queue.select(1));
        assert!(queue.move_adjacent(second, false));
        assert_eq!(queue.current_id(), Some(second));
        assert_eq!(queue.current(), Some(0));
        assert!(!queue.move_adjacent(second, false));
        assert!(queue.remove(first));
        assert_eq!(queue.current_id(), Some(second));
        assert!(!queue.remove(first));
        assert!(!queue.move_adjacent(first, true));
        assert_eq!(queue.index_of(first), None);
        assert!(queue.remove(second));
        assert_eq!(queue.current_id(), Some(third));
        assert!(queue.remove(third));
        assert_eq!(queue.current(), None);
        assert!(queue.append(vec!["same"]));
        assert!(queue.entries()[0].id > third);
        assert_eq!(queue.current(), None);
    }

    /// Relative moves preserve playback identity/modes and reject stale/self/no-op targets.
    #[test]
    fn relative_moves_are_atomic_and_identity_based() {
        let mut queue = Queue::default();
        queue.append(vec!["same", "same", "last", "fourth"]);
        let ids: Vec<_> = queue.entries().iter().map(|entry| entry.id).collect();
        queue.select(0);
        queue.set_shuffle(true);
        queue.set_repeat(RepeatMode::All);
        assert!(queue.move_to(ids[0], ids[3], true));
        assert_eq!(queue.current(), Some(3));
        assert_eq!(queue.current_id(), Some(ids[0]));
        assert!(queue.move_to(ids[0], ids[1], false));
        assert_eq!(queue.current(), Some(0));
        assert!(!queue.move_to(ids[0], ids[1], false));
        assert!(!queue.move_to(ids[0], ids[0], true));
        assert!(!queue.move_to(u64::MAX, ids[0], true));
        assert!(!queue.move_to(ids[0], u64::MAX, true));
        assert_eq!(
            queue
                .entries()
                .iter()
                .map(|entry| entry.id)
                .collect::<Vec<_>>(),
            ids
        );
        assert!(queue.shuffle());
        assert_eq!(queue.repeat(), RepeatMode::All);
    }

    /// Replacement preserves modes, resets traversal and rejects invalid batches without mutation.
    #[test]
    fn replacement_is_atomic_and_never_reuses_ids() {
        let mut queue = Queue::default();
        queue.append(vec![1, 2]);
        queue.select(1);
        queue.set_shuffle(true);
        queue.set_repeat(RepeatMode::All);
        let old_id = queue.current_id().unwrap();
        assert!(!queue.replace(Vec::new()));
        assert!(!queue.replace(vec![3; MAX_QUEUE_ITEMS + 1]));
        assert_eq!(queue.current_id(), Some(old_id));
        assert!(queue.replace(vec![4, 5]));
        assert!(queue.shuffle());
        assert_eq!(queue.repeat(), RepeatMode::All);
        assert_eq!(queue.current(), Some(0));
        assert!(queue.current_id().unwrap() > old_id);
        assert_eq!(queue.index_of(old_id), None);
        assert!(queue.advance(Advance::Next).is_some());
    }

    /// Removing the last selected row falls back to its predecessor; earlier edits keep identity.
    #[test]
    fn removing_rows_keeps_selection_valid() {
        let mut queue = Queue::default();
        assert!(queue.append(vec![1, 2, 3]));
        let ids: Vec<_> = queue.entries().iter().map(|entry| entry.id).collect();
        assert!(queue.select(2));
        assert!(queue.move_adjacent(ids[0], true));
        assert_eq!(queue.current_id(), Some(ids[2]));
        assert!(queue.remove(ids[2]));
        assert_eq!(queue.current_id(), Some(ids[0]));
        assert!(queue.remove(ids[1]));
        assert_eq!(queue.current(), Some(0));
    }
}

#[cfg(test)]
mod order_tests;
