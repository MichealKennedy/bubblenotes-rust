//! Dragging a window by a part of it, driven by the app instead of the
//! system's title-bar drag. That keeps clicks and double-clicks on the
//! dragged part (a system drag region swallows them), and reports every
//! step, so a panel beside the window can move along with it. The dragged
//! window snaps to screen edges and to the other notes (`layout`).

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use nsh_core::desktop::WindowFrame;
use nsh_core::element::desktop::{WindowRef, displays};
use nsh_core::prelude::*;

use crate::layout;
use crate::model::Rect;
use crate::win;

/// Whether dragged windows snap (the setting).
static SNAP: AtomicBool = AtomicBool::new(true);

thread_local! {
    /// The windows the next drag snaps to, besides the screens' edges.
    static NEIGHBOURS: RefCell<Vec<Rect>> = const { RefCell::new(Vec::new()) };
}

pub fn set_snap(on: bool) {
    SNAP.store(on, Ordering::Relaxed);
}

/// The other notes' windows, for the drag about to start to snap to.
pub fn set_neighbours(rects: Vec<Rect>) {
    NEIGHBOURS.with(|n| *n.borrow_mut() = rects);
}

/// The work areas of the displays.
pub fn work_areas() -> Vec<Rect> {
    untracked(displays)
        .iter()
        .map(|d| Rect { x: d.visible_bounds.origin.x.0, y: d.visible_bounds.origin.y.0, w: d.visible_bounds.size.width.0, h: d.visible_bounds.size.height.0 })
        .collect()
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
    let neighbours = NEIGHBOURS.with(|n| std::mem::take(&mut *n.borrow_mut()));
    let Some(state) = untracked(|| window.state()) else { return on_end(false) };
    let Some(start) = win::cursor() else { return on_end(false) };
    let origin = Rect { x: state.bounds.origin.x.0, y: state.bounds.origin.y.0, w: state.bounds.size.width.0, h: state.bounds.size.height.0 };
    let scale = state.scale_factor.max(0.1);
    let snap = SNAP.load(Ordering::Relaxed);
    let screens = if snap { work_areas() } else { Vec::new() };
    let neighbours = if snap { neighbours } else { Vec::new() };
    owner.run(|| {
        let _ = spawn(async move {
            let mut moved = false;
            let mut last = origin;
            // One step to where the pointer is now.
            let mut step = |moved: &mut bool| {
                let Some(now) = win::cursor() else { return false };
                let (dx, dy) = (now.0 - start.0, now.1 - start.1);
                if !*moved && dx.abs() < THRESHOLD && dy.abs() < THRESHOLD {
                    return true;
                }
                *moved = true;
                let to = layout::snapped(Rect { x: origin.x + dx / scale, y: origin.y + dy / scale, ..origin }, &screens, &neighbours);
                if to != last {
                    last = to;
                    window.set_bounds(WindowFrame { x: Some(px(to.x)), y: Some(px(to.y)), width: None, height: None, animated: None });
                    on_move(to.x, to.y);
                }
                true
            };
            while win::primary_button_down() {
                timer(Duration::from_millis(8)).await;
                if !step(&mut moved) {
                    break;
                }
            }
            // Where the pointer ended up, also when the drag was over before
            // the first look at it.
            step(&mut moved);
            on_end(moved);
        });
    });
}
