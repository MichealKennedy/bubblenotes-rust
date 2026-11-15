//! The few things the app asks Windows for directly, because NSH has no
//! API for them: where the pointer is on the screen and whether its button
//! is held. Elsewhere these report nothing, and what uses them does less.

/// The pointer's place on the screen, in physical pixels.
#[cfg(target_os = "windows")]
pub fn cursor() -> Option<(f32, f32)> {
    use nsh_platform_windows::windows::Win32::{Foundation::POINT, UI::WindowsAndMessaging::GetCursorPos};
    let mut point = POINT::default();
    // SAFETY: `point` is a valid POINT for the call to fill.
    unsafe { GetCursorPos(&mut point) }.ok()?;
    Some((point.x as f32, point.y as f32))
}

/// Whether the primary mouse button is held down.
#[cfg(target_os = "windows")]
pub fn primary_button_down() -> bool {
    use nsh_platform_windows::windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
    // SAFETY: a plain query of the keyboard state.
    unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) < 0 }
}

#[cfg(not(target_os = "windows"))]
pub fn cursor() -> Option<(f32, f32)> {
    None
}

#[cfg(not(target_os = "windows"))]
pub fn primary_button_down() -> bool {
    false
}
