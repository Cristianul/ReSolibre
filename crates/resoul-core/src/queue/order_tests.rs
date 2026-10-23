//! Seeded behavioral tests exercise public policy rather than a particular random permutation.

use super::*;
use std::collections::HashSet;

/// Builds a selected queue with reproducible traversal and duplicate source values.
fn shuffled(count: usize, seed: u64) -> Queue<&'static str> {
    let mut queue = Queue::with_seed(seed);
    assert!(queue.append(vec!["same source"; count]));
    if count > 0 {
        assert!(queue.select(0));
    }
    queue.set_shuffle(true);
    queue
}

/// Automatic Shuffle visits every occurrence once without changing display order.
#[test]
fn shuffled_passes_preserve_order_and_occurrence_uniqueness() {
    for seed in 0..32 {
        let mut queue = shuffled(12, seed);
        let visible = queue.ids();
        let mut seen = HashSet::from([queue.current_id().unwrap()]);
        while queue.advance(Advance::Next).is_some() {
            assert!(
                seen.insert(queue.current_id().unwrap()),
                "repeated occurrence in pass"
            );
        }
        assert_eq!(seen.len(), 12);
        assert_eq!(queue.ids(), visible);
        assert!(!queue.can_advance(Advance::Next));
    }
}

/// Repeat All retains every item in each new shuffle pass and avoids an immediate boundary repeat.
#[test]
fn repeat_all_starts_complete_new_passes() {
    let mut queue = shuffled(9, 45);
    while queue.advance(Advance::Ended).is_some() {}
    queue.set_repeat(RepeatMode::All);
    for _ in 0..10 {
        let last = queue.current_id().unwrap();
        let mut seen = HashSet::new();
        for index in 0..9 {
            assert!(queue.advance(Advance::Ended).is_some());
            let id = queue.current_id().unwrap();
            if index == 0 {
                assert_ne!(id, last);
            }
            assert!(seen.insert(id));
        }
        assert_eq!(seen.len(), 9);
    }
}

/// Previous/Next replay actual visits, while direct selection branches forward history.
#[test]
fn history_replays_then_branches_on_explicit_selection() {
    let mut queue = shuffled(8, 17);
    let first = queue.current_id().unwrap();
    queue.advance(Advance::Next).unwrap();
    let second = queue.current_id().unwrap();
    queue.advance(Advance::Next).unwrap();
    let third = queue.current_id().unwrap();
    queue.advance(Advance::Next).unwrap();
    queue.advance(Advance::Previous).unwrap();
    assert_eq!(queue.current_id(), Some(third));
    queue.advance(Advance::Previous).unwrap();
    assert_eq!(queue.current_id(), Some(second));
    queue.advance(Advance::Next).unwrap();
    assert_eq!(queue.current_id(), Some(third));
    queue.select(queue.index_of(first).unwrap());
    queue.advance(Advance::Previous).unwrap();
    assert_eq!(queue.current_id(), Some(third));
    queue.advance(Advance::Next).unwrap();
    assert_eq!(queue.current_id(), Some(first));
    let before = queue.current_id();
    queue.set_shuffle(true);
    assert_eq!(queue.current_id(), before);
    assert!(queue.can_advance(Advance::Previous));
}

/// Editing pending/history IDs preserves existing visits; current removal starts a new traversal.
#[test]
fn edits_prune_history_and_include_new_occurrences() {
    let mut queue = shuffled(5, 78);
    let first = queue.current_id().unwrap();
    queue.advance(Advance::Next).unwrap();
    let second = queue.current_id().unwrap();
    queue.advance(Advance::Next).unwrap();
    let third = queue.current_id().unwrap();
    assert!(queue.remove(second));
    queue.advance(Advance::Previous).unwrap();
    assert_eq!(queue.current_id(), Some(first));
    queue.advance(Advance::Next).unwrap();
    assert_eq!(queue.current_id(), Some(third));
    assert!(queue.append(vec!["new source"]));
    let added = queue.entries().last().unwrap().id;
    assert!(queue.move_adjacent(added, false));
    let mut remaining = HashSet::new();
    while queue.advance(Advance::Next).is_some() {
        assert!(remaining.insert(queue.current_id().unwrap()));
    }
    assert!(remaining.contains(&added));
    assert!(!remaining.contains(&second));
    let current = queue.current_id().unwrap();
    assert!(queue.remove(current));
    assert!(!queue.can_advance(Advance::Previous));
    assert!(queue.can_advance(Advance::Next));
    assert!(!queue.remove(current));
}

/// Repeat One affects EOF only; ordered All wraps both directions and defaults remain bounded.
#[test]
fn repeat_and_empty_single_queue_policies() {
    let mut queue = Queue::with_seed(9);
    for mode in [RepeatMode::Off, RepeatMode::All, RepeatMode::One] {
        queue.set_repeat(mode);
        for reason in [Advance::Next, Advance::Previous, Advance::Ended] {
            assert_eq!(queue.advance(reason), None);
            assert!(!queue.can_advance(reason));
        }
    }
    queue.append(vec![1, 2]);
    queue.select(0);
    queue.set_repeat(RepeatMode::One);
    assert_eq!(queue.advance(Advance::Ended), Some(0));
    assert_eq!(queue.advance(Advance::Next), Some(1));
    assert_eq!(queue.advance(Advance::Next), None);
    queue.set_repeat(RepeatMode::All);
    assert_eq!(queue.advance(Advance::Next), Some(0));
    assert_eq!(queue.advance(Advance::Previous), Some(1));
    let removed = queue.entries()[0].id;
    queue.remove(removed);
    for shuffle in [false, true] {
        queue.set_shuffle(shuffle);
        for mode in [RepeatMode::Off, RepeatMode::One, RepeatMode::All] {
            queue.set_repeat(mode);
            let expected = if mode == RepeatMode::Off {
                None
            } else {
                Some(0)
            };
            assert_eq!(queue.advance(Advance::Ended), expected);
        }
    }
    assert_eq!(RepeatMode::Off.cycle().cycle().cycle(), RepeatMode::Off);
}

/// Long repeated sessions retain bounded Previous history and never lose valid selection.
#[test]
fn repeat_history_is_bounded_and_modes_preserve_selection() {
    let mut queue = shuffled(7, 99);
    queue.set_repeat(RepeatMode::All);
    for _ in 0..5000 {
        queue.advance(Advance::Next).unwrap();
    }
    let mut visits = 0;
    while queue.advance(Advance::Previous).is_some() {
        visits += 1;
    }
    assert_eq!(visits, MAX_QUEUE_ITEMS - 1);
    let selected = queue.current_id();
    queue.set_shuffle(false);
    assert_eq!(queue.current_id(), selected);
    queue.set_shuffle(true);
    assert_eq!(queue.current_id(), selected);
    assert!(!queue.can_advance(Advance::Previous));
}

/// Repeated mixed edits/modes keep pending IDs valid, bounded and free of stale selections.
#[test]
fn mixed_edits_and_mode_changes_preserve_invariants() {
    let mut queue = shuffled(16, 36);
    for step in 0..3000 {
        match step % 9 {
            0 => {
                queue.advance(Advance::Next);
            }
            1 => {
                queue.advance(Advance::Previous);
            }
            2 => {
                if let Some(id) = queue.entries().first().map(|e| e.id) {
                    queue.remove(id);
                }
            }
            3 => {
                if queue.entries().len() < 32 {
                    queue.append(vec!["same source"]);
                }
            }
            4 => queue.set_repeat(queue.repeat().cycle()),
            5 => queue.set_shuffle(!queue.shuffle()),
            6 => {
                if let Some(id) = queue.entries().last().map(|e| e.id) {
                    queue.move_adjacent(id, false);
                }
            }
            7 => {
                queue.select(queue.entries().len() / 2);
            }
            _ => {
                queue.advance(Advance::Ended);
            }
        }
        if let Some(index) = queue.current() {
            assert!(index < queue.entries().len());
        }
        let ids: HashSet<_> = queue.ids().into_iter().collect();
        assert_eq!(ids.len(), queue.entries().len());
    }
}
