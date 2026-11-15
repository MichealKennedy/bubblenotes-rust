//! Dragging a window by a part of it, driven by the app instead of the
//! system's title-bar drag. That keeps clicks and double-clicks on the
//! dragged part (a system drag region swallows them), and reports every
//! step, so a panel beside the window can move along with it.

use std::time::Duration;

use nsh_core::desktop::WindowFrame;
use nsh_core::element::desktop::WindowRef;
use nsh_core::prelude::*;

use std::sync::atomic::{AtomicBool, Ordering};

use nsh_core::element::desktop::displays;

use crate::win;

/// How near a screen edge a dragged window snaps to it, in points.
const SNAP: f32 = 10.0;

/// Whether dragged windows snap to the screen's edges (the setting).
static SNAP_TO_EDGES: AtomicBool = AtomicBool::new(true);

pub fn set_snap_to_edges(on: bool) {
    SNAP_TO_EDGES.store(on, Ordering::Relaxed);
}

/// `at` (a window's top left corner, `size` big) pulled onto the edges of
/// the work area it is within `SNAP` of. Areas are (x, y, width, height).
fn snapped(at: (f32, f32), size: (f32, f32), areas: &[(f32, f32, f32, f32)]) -> (f32, f32) {
    let (mut x, mut y) = at;
    for &(left, top, width, height) in areas {
        let (right, bottom) = (left + width, top + height);
        // Only the screen the window is on pulls it.
        if at.0 + size.0 < left || at.0 > right || at.1 + size.1 < top || at.1 > bottom {
            continue;
        }
        if (at.0 - left).abs() <= SNAP {
            x = left;
        } else if (at.0 + size.0 - right).abs() <= SNAP {
            x = right - size.0;
        }
        if (at.1 - top).abs() <= SNAP {
            y = top;
        } else if (at.1 + size.1 - bottom).abs() <= SNAP {
            y = bottom - size.1;
        }
    }
    (x, y)
}

/// How far the pointer moves before a press becomes a drag, in pixels.
const THRESHOLD: f32 = 4.0;

/// Call when the primary button goes down on the part that drags `window`:
/// the window follows the pointer until the button is released.
///
/// `on_move` gets the window's new top left corner (in points) at every
/// step; `on_end` gets whether it moved at all (`false`: it was a click).
/// `owner` is the owner the drag's task runs under.
pub fn drag_window(owner: Owner, window: WindowRef, on_move: impl Fn(f32, f32) + 'static, on_end: impl Fn(bool) + 'static) {
    let Some(state) = untracked(|| window.state()) else { return on_end(false) };
    let Some(start) = win::cursor() else { return on_end(false) };
    let origin = (state.bounds.origin.x.0, state.bounds.origin.y.0);
    let scale = state.scale_factor.max(0.1);
    let size = (state.bounds.size.width.0, state.bounds.size.height.0);
    let areas: Vec<(f32, f32, f32, f32)> = if SNAP_TO_EDGES.load(Ordering::Relaxed) {
        untracked(displays)
            .iter()
            .map(|d| (d.visible_bounds.origin.x.0, d.visible_bounds.origin.y.0, d.visible_bounds.size.width.0, d.visible_bounds.size.height.0))
            .collect()
    } else {
        Vec::new()
    };
    owner.run(|| {
        let _ = spawn(async move {
            let mut moved = false;
            let mut last = origin;
            while win::primary_button_down() {
                timer(Duration::from_millis(8)).await;
                let Some(now) = win::cursor() else { break };
                let (dx, dy) = (now.0 - start.0, now.1 - start.1);
                if !moved && dx.abs() < THRESHOLD && dy.abs() < THRESHOLD {
                    continue;
                }
                moved = true;
                let to = snapped((origin.0 + dx / scale, origin.1 + dy / scale), size, &areas);
                if to != last {
                    last = to;
                    window.set_bounds(WindowFrame { x: Some(px(to.0)), y: Some(px(to.1)), width: None, height: None, animated: None });
                    on_move(to.0, to.1);
                }
            }
            on_end(moved);
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_near_an_edge_snaps_to_it() {
        let screen = [(0.0, 0.0, 1000.0, 800.0)];
        assert_eq!(snapped((6.0, 300.0), (200.0, 100.0), &screen), (0.0, 300.0));
        assert_eq!(snapped((795.0, 695.0), (200.0, 100.0), &screen), (800.0, 700.0));
        assert_eq!(snapped((400.0, 300.0), (200.0, 100.0), &screen), (400.0, 300.0));
        // Another screen far away doesn't pull it.
        assert_eq!(snapped((400.0, 300.0), (200.0, 100.0), &[(2000.0, 295.0, 1000.0, 800.0)]), (400.0, 300.0));
    }
}
