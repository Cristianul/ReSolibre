use super::*;

const PLAYER: Rect = Rect {
    x: 100,
    y: 100,
    width: 400,
    height: 300,
};

/// Initializes the same tracked state as the first native mapping events.
fn mapped(frames: &[Option<Rect>]) -> Movement {
    let mut state = Movement::default();
    for _ in frames {
        state.register();
    }
    state.update(0, Some(0), frames);
    state
}

/// Every attached side follows both axes; follower acknowledgements never move the player.
#[test]
fn follows_all_sides_without_feedback() {
    for (x, y) in [(500, 100), (-100, 100), (100, -50), (100, 400)] {
        let follower = Rect {
            x,
            y,
            width: 200,
            height: 150,
        };
        let mut frames = vec![Some(PLAYER), Some(follower)];
        let mut state = mapped(&frames);
        frames[0] = geometry::translate(PLAYER, 80, -40);
        let target = geometry::translate(follower, 80, -40).unwrap();
        assert_eq!(state.update(0, Some(0), &frames), vec![(1, target)]);
        frames[1] = Some(target);
        assert!(state.update(1, Some(0), &frames).is_empty());
        assert!(state.update(0, Some(0), &frames).is_empty());
    }
}

/// Dragging an auxiliary away breaks the link; snapping back restores it.
#[test]
fn detach_and_reattach() {
    let follower = Rect {
        x: 500,
        y: 100,
        width: 200,
        height: 150,
    };
    let mut frames = vec![Some(PLAYER), Some(follower)];
    let mut state = mapped(&frames);
    frames[1] = Some(Rect {
        x: 650,
        y: 180,
        ..follower
    });
    assert!(state.update(1, Some(0), &frames).is_empty());
    frames[0] = geometry::translate(PLAYER, 20, 20);
    assert!(state.update(0, Some(0), &frames).is_empty());
    frames[1] = Some(Rect {
        x: 525,
        y: 120,
        ..follower
    });
    let snapped = Rect {
        x: 520,
        y: 120,
        ..follower
    };
    assert_eq!(state.update(1, Some(0), &frames), vec![(1, snapped)]);
    frames[1] = Some(snapped);
    state.update(1, Some(0), &frames);
    frames[0] = geometry::translate(frames[0].unwrap(), 30, 40);
    assert_eq!(
        state.update(0, Some(0), &frames),
        vec![(1, geometry::translate(snapped, 30, 40).unwrap())]
    );
}

/// Fast main moves retain offsets before follower acknowledgements arrive, even when coalesced.
#[test]
fn rapid_moves_and_interrupted_follower() {
    let follower = Rect { x: 500, ..PLAYER };
    let mut frames = vec![Some(PLAYER), Some(follower)];
    let mut state = mapped(&frames);
    for step in 1..=100 {
        frames[0] = geometry::translate(PLAYER, step * 10, step * 2);
        let expected = geometry::translate(follower, step * 10, step * 2).unwrap();
        assert_eq!(state.update(0, Some(0), &frames), vec![(1, expected)]);
    }
    assert_eq!(state.windows[1].pending.len(), 32);
    frames[1] = geometry::translate(follower, 990, 198);
    assert!(state.update(1, Some(0), &frames).is_empty());
    frames[1] = geometry::translate(follower, 1000, 200);
    assert!(state.update(1, Some(0), &frames).is_empty());
    frames[1] = geometry::translate(follower, 1200, 250);
    assert!(state.update(1, Some(0), &frames).is_empty());
    frames[0] = geometry::translate(PLAYER, 1010, 202);
    assert!(state.update(0, Some(0), &frames).is_empty());
}

/// Only eligible edge-connected windows move; hidden chains, gaps and overlapping interiors do not.
#[test]
fn eligibility_chains_and_coordinate_bounds() {
    let follower = Rect { x: 500, ..PLAYER };
    let chained = Rect { x: 900, ..PLAYER };
    let frames = vec![
        Some(PLAYER),
        Some(follower),
        Some(chained),
        Some(Rect { x: 1306, ..PLAYER }),
        Some(Rect {
            x: 120,
            y: 120,
            width: 20,
            height: 20,
        }),
    ];
    assert_eq!(
        geometry::connected(0, &frames),
        vec![true, true, true, false, false]
    );
    let mut state = mapped(&frames);
    let mut hidden = frames.clone();
    hidden[1] = None;
    hidden[0] = geometry::translate(PLAYER, -70, -60);
    assert!(state.update(0, Some(0), &hidden).is_empty());
    assert_eq!(geometry::translate(PLAYER, i64::MAX, 0), None);
    assert_eq!(geometry::translate(PLAYER, 0, i64::MIN), None);
    assert_eq!(
        geometry::translate(
            Rect {
                x: i32::MAX,
                ..PLAYER
            },
            1,
            0
        ),
        None
    );
}

/// Resizing a stationary main updates its edges without displacing an auxiliary.
#[test]
fn resizing_does_not_drag_followers() {
    let follower = Rect { x: 500, ..PLAYER };
    let mut frames = vec![Some(PLAYER), Some(follower)];
    let mut state = mapped(&frames);
    frames[0] = Some(Rect {
        height: 150,
        ..PLAYER
    });
    assert!(state.update(0, Some(0), &frames).is_empty());
    frames[0] = geometry::translate(frames[0].unwrap(), 10, 20);
    assert_eq!(
        state.update(0, Some(0), &frames),
        vec![(1, geometry::translate(follower, 10, 20).unwrap())]
    );
}

/// Interrupted and expired placement requests cannot keep a manually moved follower attached.
#[test]
fn interrupted_and_expired_requests_detach() {
    for expire in [false, true] {
        let follower = Rect { x: 500, ..PLAYER };
        let mut frames = vec![Some(PLAYER), Some(follower)];
        let mut state = mapped(&frames);
        frames[0] = geometry::translate(PLAYER, 100, 50);
        state.update(0, Some(0), &frames);
        if expire {
            state.windows[1].requested_at = Some(Instant::now() - Duration::from_secs(1));
        }
        frames[1] = geometry::translate(follower, 250, 90);
        assert!(state.update(1, Some(0), &frames).is_empty());
        assert!(state.windows[1].pending.is_empty());
        frames[0] = geometry::translate(PLAYER, 110, 60);
        assert!(state.update(0, Some(0), &frames).is_empty());
    }
}

/// Duplicate geometry notifications must not overwrite a newer follower destination.
#[test]
fn duplicate_follower_observation_keeps_latest_destination() {
    let follower = Rect { x: 500, ..PLAYER };
    let mut frames = vec![Some(PLAYER), Some(follower)];
    let mut state = mapped(&frames);
    frames[0] = geometry::translate(PLAYER, 20, 0);
    state.update(0, Some(0), &frames);
    frames[1] = geometry::translate(follower, 20, 0);
    assert!(state.update(1, Some(0), &frames).is_empty());

    // The main advances while another notification still reads the acknowledged position.
    frames[0] = geometry::translate(PLAYER, 40, 0);
    state.update(0, Some(0), &frames);
    assert!(state.update(1, Some(0), &frames).is_empty());
    assert_eq!(state.windows[1].frame, geometry::translate(follower, 40, 0));
    frames[0] = geometry::translate(PLAYER, 60, 0);
    assert_eq!(
        state.update(0, Some(0), &frames),
        vec![(1, geometry::translate(follower, 60, 0).unwrap())]
    );
}

/// Duplicate suppression expires when the WM never applies a newer request.
#[test]
fn duplicate_observation_after_denied_request_expires() {
    let follower = Rect { x: 500, ..PLAYER };
    let mut frames = vec![Some(PLAYER), Some(follower)];
    let mut state = mapped(&frames);
    frames[0] = geometry::translate(PLAYER, 20, 0);
    state.update(0, Some(0), &frames);
    frames[1] = geometry::translate(follower, 20, 0);
    state.update(1, Some(0), &frames);
    frames[0] = geometry::translate(PLAYER, 40, 0);
    state.update(0, Some(0), &frames);
    state.windows[1].requested_at = Some(Instant::now() - Duration::from_secs(1));
    assert!(state.update(1, Some(0), &frames).is_empty());
    assert_eq!(state.windows[1].frame, frames[1]);
    assert!(state.windows[1].pending.is_empty());
    frames[0] = geometry::translate(PLAYER, 60, 0);
    assert!(state.update(0, Some(0), &frames).is_empty());
}
