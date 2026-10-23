//! Pure geometry for magnetic placement; all coordinates and the threshold are physical pixels.

pub const DISTANCE: i64 = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Finds the connected component using exact edge contact, including corner docking.
/// Missing/ineligible windows break attachment chains.
pub(super) fn connected(root: usize, frames: &[Option<Rect>]) -> Vec<bool> {
    let mut attached = vec![false; frames.len()];
    attached[root] = true;
    for _ in 0..frames.len() {
        let mut changed = false;
        for index in 0..frames.len() {
            if attached[index] {
                continue;
            }
            let Some(frame) = frames[index] else { continue };
            for peer in 0..frames.len() {
                if attached[peer] && frames[peer].is_some_and(|other| touching(frame, other)) {
                    attached[index] = true;
                    changed = true;
                    break;
                }
            }
        }
        if !changed {
            break;
        }
    }
    attached
}

/// Tests actual edge contact; proximity alone does not create a persistent attachment.
fn touching(a: Rect, b: Rect) -> bool {
    if a.width == 0 || a.height == 0 || b.width == 0 || b.height == 0 {
        return false;
    }
    let (ax, ay) = (i64::from(a.x), i64::from(a.y));
    let (bx, by) = (i64::from(b.x), i64::from(b.y));
    let (ar, ab) = (ax + i64::from(a.width), ay + i64::from(a.height));
    let (br, bb) = (bx + i64::from(b.width), by + i64::from(b.height));
    ((ar == bx || br == ax) && ay <= bb && by <= ab)
        || ((ab == by || bb == ay) && ax <= br && bx <= ar)
}

/// Preserves the relative offset without wrapping native signed coordinates.
pub(super) fn translate(frame: Rect, dx: i64, dy: i64) -> Option<Rect> {
    Some(Rect {
        x: i32::try_from(i64::from(frame.x).checked_add(dx)?).ok()?,
        y: i32::try_from(i64::from(frame.y).checked_add(dy)?).ok()?,
        ..frame
    })
}

/// Keeps the initial pair on the current monitor when it fits; negative monitor origins are valid.
pub fn right_dock(main: Rect, playlist: Rect, monitor: Rect) -> Option<(i32, i32, i32)> {
    if main.width == 0 || playlist.width == 0 || monitor.width == 0 || monitor.height == 0 {
        return None;
    }
    let left = i64::from(monitor.x);
    let top = i64::from(monitor.y);
    let right = left + i64::from(monitor.width);
    let bottom = top + i64::from(monitor.height);
    let total_width = i64::from(main.width) + i64::from(playlist.width);
    let height = i64::from(main.height.max(playlist.height));
    let x = i64::from(main.x).clamp(left, (right - total_width).max(left));
    let y = i64::from(main.y).clamp(top, (bottom - height).max(top));
    Some((
        i32::try_from(x).ok()?,
        i32::try_from(y).ok()?,
        i32::try_from(x + i64::from(main.width)).ok()?,
    ))
}

/// Finds the nearest adjacent edge, optionally aligning the ends of that same edge.
/// Only caller-supplied peers participate; no desktop/global window discovery occurs.
pub fn snap(moving: Rect, peers: &[Rect]) -> Option<(i32, i32)> {
    if moving.width == 0 || moving.height == 0 {
        return None;
    }
    let mut best: Option<(i64, i64, i64)> = None;
    let x = i64::from(moving.x);
    let y = i64::from(moving.y);
    let width = i64::from(moving.width);
    let height = i64::from(moving.height);
    for peer in peers {
        if peer.width == 0 || peer.height == 0 {
            continue;
        }
        let left = i64::from(peer.x);
        let top = i64::from(peer.y);
        let right = left + i64::from(peer.width);
        let bottom = top + i64::from(peer.height);
        for target_x in [left - width, right] {
            if (target_x - x).abs() > DISTANCE {
                continue;
            }
            let target_y = align(y, &[top, bottom - height, top - height, bottom]);
            if target_y > bottom || target_y + height < top {
                continue;
            }
            consider(&mut best, x, y, target_x, target_y);
        }
        for target_y in [top - height, bottom] {
            if (target_y - y).abs() > DISTANCE {
                continue;
            }
            let target_x = align(x, &[left, right - width, left - width, right]);
            if target_x > right || target_x + width < left {
                continue;
            }
            consider(&mut best, x, y, target_x, target_y);
        }
    }
    let (_, target_x, target_y) = best?;
    if target_x == x && target_y == y {
        return None;
    }
    Some((i32::try_from(target_x).ok()?, i32::try_from(target_y).ok()?))
}

/// Chooses the nearest perpendicular alignment without reaching beyond the snap threshold.
fn align(position: i64, edges: &[i64]) -> i64 {
    let mut target = position;
    let mut distance = DISTANCE + 1;
    for edge in edges {
        let candidate = (*edge - position).abs();
        if candidate < distance {
            distance = candidate;
            target = *edge;
        }
    }
    target
}

/// Resolves competing edges deterministically and rejects positions outside native coordinate bounds.
fn consider(best: &mut Option<(i64, i64, i64)>, x: i64, y: i64, target_x: i64, target_y: i64) {
    if i32::try_from(target_x).is_err() || i32::try_from(target_y).is_err() {
        return;
    }
    let distance = (target_x - x).pow(2) + (target_y - y).pow(2);
    if best.is_none_or(|(previous, _, _)| distance < previous) {
        *best = Some((distance, target_x, target_y));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const PEER: Rect = Rect {
        x: 100,
        y: 100,
        width: 400,
        height: 300,
    };

    /// Initial placement fits a pair, handles small/negative monitors and rejects invalid sizes.
    #[test]
    fn initial_right_dock_respects_monitor_bounds() {
        let main = Rect {
            x: 900,
            y: 600,
            width: 860,
            height: 608,
        };
        let playlist = Rect { width: 420, ..main };
        let monitor = Rect {
            x: 0,
            y: 0,
            width: 1280,
            height: 720,
        };
        assert_eq!(right_dock(main, playlist, monitor), Some((0, 112, 860)));
        let negative = Rect {
            x: -1920,
            y: -1080,
            width: 1920,
            height: 1080,
        };
        assert_eq!(
            right_dock(main, playlist, negative),
            Some((-1280, -608, -420))
        );
        assert_eq!(
            right_dock(
                main,
                playlist,
                Rect {
                    width: 800,
                    ..monitor
                }
            ),
            Some((0, 112, 860))
        );
        assert_eq!(
            right_dock(
                main,
                playlist,
                Rect {
                    width: 0,
                    ..monitor
                }
            ),
            None
        );
        assert_eq!(
            right_dock(
                main,
                playlist,
                Rect {
                    x: i32::MAX,
                    ..monitor
                }
            ),
            None
        );
    }

    /// Five pixels is inclusive on every side; six pixels and distant projections do not snap.
    #[test]
    fn all_edges_threshold_and_alignment() {
        for (x, y, expected) in [
            (-105, 180, Some((-100, 180))),
            (505, 180, Some((500, 180))),
            (200, -55, Some((200, -50))),
            (200, 405, Some((200, 400))),
            (-106, 180, None),
            (506, 180, None),
            (200, -56, None),
            (200, 406, None),
            (505, 104, Some((500, 100))),
            (505, 246, Some((500, 250))),
            (104, 405, Some((100, 400))),
            (296, 405, Some((300, 400))),
            (505, 405, Some((500, 400))),
            (505, 800, None),
            (100, 900, None),
            (500, 180, None),
            (250, 200, None),
        ] {
            let moving = Rect {
                x,
                y,
                width: 200,
                height: 150,
            };
            assert_eq!(snap(moving, &[PEER]), expected, "origin {x}, {y}");
        }
    }

    /// Peers are explicit, negative desktop coordinates work and arithmetic cannot overflow.
    #[test]
    fn boundaries_and_owned_peers_only() {
        let moving = Rect {
            x: -705,
            y: -300,
            width: 200,
            height: 150,
        };
        let peer = Rect {
            x: -500,
            y: -300,
            ..PEER
        };
        assert_eq!(snap(moving, &[]), None);
        assert_eq!(snap(moving, &[peer]), Some((-700, -300)));
        assert_eq!(snap(Rect { width: 0, ..moving }, &[peer]), None);
        let extreme = Rect {
            x: i32::MIN,
            y: i32::MAX,
            width: u32::MAX,
            height: u32::MAX,
        };
        assert_eq!(snap(extreme, &[PEER]), None);
        assert_eq!(snap(PEER, &[extreme]), None);
    }
}
