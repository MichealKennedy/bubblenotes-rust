//! Where windows may go: snapping a dragged window to screen edges and to
//! other notes, and bringing back notes that are off every screen. Plain
//! arithmetic on rectangles, in points.

use crate::model::Rect;

/// How near an edge a dragged window snaps to it.
pub const SNAP: f32 = 10.0;
/// The gap left between two notes snapped side by side.
const GAP: f32 = 6.0;
/// How much of a note must be on a screen for it to count as reachable.
const REACHABLE: f32 = 48.0;

fn right(r: &Rect) -> f32 {
    r.x + r.w
}

fn bottom(r: &Rect) -> f32 {
    r.y + r.h
}

/// Whether two spans overlap or nearly do (so the things are side by side).
fn near(a0: f32, a1: f32, b0: f32, b1: f32) -> bool {
    a0 <= b1 + SNAP && b0 <= a1 + SNAP
}

/// `rect` (a window being dragged) moved onto what it is within `SNAP` of:
/// the edges of the screen it's on (`screens` are work areas), and the
/// edges of the other notes (`notes`), beside which it leaves a small gap.
/// Each direction snaps to the nearest candidate.
pub fn snapped(rect: Rect, screens: &[Rect], notes: &[Rect]) -> Rect {
    // (how far to move, in x), (in y)
    let mut dx: Option<f32> = None;
    let mut dy: Option<f32> = None;
    let offer = |slot: &mut Option<f32>, delta: f32| {
        if delta.abs() <= SNAP && slot.is_none_or(|best| delta.abs() < best.abs()) {
            *slot = Some(delta);
        }
    };
    for screen in screens {
        // Only the screen the window is on pulls it.
        if right(&rect) < screen.x || rect.x > right(screen) || bottom(&rect) < screen.y || rect.y > bottom(screen) {
            continue;
        }
        offer(&mut dx, screen.x - rect.x);
        offer(&mut dx, right(screen) - right(&rect));
        offer(&mut dy, screen.y - rect.y);
        offer(&mut dy, bottom(screen) - bottom(&rect));
    }
    for note in notes {
        if near(rect.y, bottom(&rect), note.y, bottom(note)) {
            // Beside it, or with the same left or right edge.
            offer(&mut dx, right(note) + GAP - rect.x);
            offer(&mut dx, note.x - GAP - right(&rect));
        }
        if near(rect.x, right(&rect), note.x, right(note)) {
            offer(&mut dy, bottom(note) + GAP - rect.y);
            offer(&mut dy, note.y - GAP - bottom(&rect));
            offer(&mut dx, note.x - rect.x);
            offer(&mut dx, right(note) - right(&rect));
        }
        if near(rect.y, bottom(&rect), note.y, bottom(note)) {
            offer(&mut dy, note.y - rect.y);
            offer(&mut dy, bottom(note) - bottom(&rect));
        }
    }
    Rect { x: rect.x + dx.unwrap_or(0.0), y: rect.y + dy.unwrap_or(0.0), ..rect }
}

/// `rect` moved onto a screen when too little of it is on any (a monitor
/// was unplugged, the resolution changed): fully inside the nearest work
/// area, as far as it fits. `None` when it's fine where it is.
pub fn rescued(rect: Rect, screens: &[Rect]) -> Option<Rect> {
    if screens.is_empty() {
        return None;
    }
    let visible = |screen: &Rect| {
        let w = right(&rect).min(right(screen)) - rect.x.max(screen.x);
        let h = bottom(&rect).min(bottom(screen)) - rect.y.max(screen.y);
        // Enough to see and grab it: a piece of its top (the title).
        w >= REACHABLE.min(rect.w) && h >= REACHABLE.min(rect.h) && rect.y >= screen.y - 1.0
    };
    if screens.iter().any(visible) {
        return None;
    }
    let distance = |screen: &Rect| {
        let cx = (rect.x + rect.w / 2.0).clamp(screen.x, right(screen)) - (rect.x + rect.w / 2.0);
        let cy = (rect.y + rect.h / 2.0).clamp(screen.y, bottom(screen)) - (rect.y + rect.h / 2.0);
        cx * cx + cy * cy
    };
    let screen = screens.iter().min_by(|a, b| distance(a).total_cmp(&distance(b)))?;
    let margin = 12.0;
    let x = rect.x.min(right(screen) - rect.w - margin).max(screen.x + margin);
    let y = rect.y.min(bottom(screen) - rect.h - margin).max(screen.y + margin);
    Some(Rect { x, y, ..rect })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Rect = Rect { x: 0.0, y: 0.0, w: 1000.0, h: 800.0 };

    fn at(x: f32, y: f32) -> Rect {
        Rect { x, y, w: 200.0, h: 100.0 }
    }

    #[test]
    fn a_window_near_a_screen_edge_snaps_to_it() {
        assert_eq!(snapped(at(6.0, 300.0), &[SCREEN], &[]), at(0.0, 300.0));
        assert_eq!(snapped(at(795.0, 695.0), &[SCREEN], &[]), at(800.0, 700.0));
        assert_eq!(snapped(at(400.0, 300.0), &[SCREEN], &[]), at(400.0, 300.0));
        // A screen it isn't on doesn't pull it.
        assert_eq!(snapped(at(400.0, 300.0), &[Rect { x: 2000.0, y: 295.0, ..SCREEN }], &[]), at(400.0, 300.0));
    }

    #[test]
    fn a_window_snaps_beside_another_note_and_lines_up_with_it() {
        let other = at(300.0, 300.0);
        // To its right, a little lower: sits beside it with a gap, tops level.
        assert_eq!(snapped(at(512.0, 304.0), &[SCREEN], &[other]), at(506.0, 300.0));
        // Under it, a little to the right: gap below, left edges level.
        assert_eq!(snapped(at(304.0, 410.0), &[SCREEN], &[other]), at(300.0, 406.0));
        // Far away: untouched.
        assert_eq!(snapped(at(600.0, 600.0), &[SCREEN], &[other]), at(600.0, 600.0));
    }

    #[test]
    fn a_note_off_every_screen_comes_back() {
        assert_eq!(rescued(at(100.0, 100.0), &[SCREEN]), None);
        // On a monitor that's gone, to the right.
        assert_eq!(rescued(at(2500.0, 300.0), &[SCREEN]), Some(at(788.0, 300.0)));
        // Its title above the top of the screen: can't be grabbed.
        assert_eq!(rescued(at(100.0, -60.0), &[SCREEN]), Some(at(100.0, 12.0)));
        // Barely a sliver showing at the left.
        assert_eq!(rescued(at(-190.0, 300.0), &[SCREEN]), Some(at(12.0, 300.0)));
        assert_eq!(rescued(at(5000.0, 5000.0), &[]), None);
    }
}
