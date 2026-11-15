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

/// Whether Ctrl and Alt are both held down.
#[cfg(target_os = "windows")]
pub fn ctrl_alt_down() -> bool {
    use nsh_platform_windows::windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_CONTROL, VK_MENU};
    // SAFETY: plain queries of the keyboard state.
    unsafe { GetAsyncKeyState(VK_CONTROL.0 as i32) < 0 && GetAsyncKeyState(VK_MENU.0 as i32) < 0 }
}

/// Makes this app's window titled `title` let clicks through to what's
/// under it, or take them again. NSH refuses click-through windows to
/// apps like this one, so the window's style is set directly. NSH sets a
/// window's styles again when its attributes change: call this again
/// after that. Whether the window was found.
#[cfg(target_os = "windows")]
pub fn set_click_through(title: &str, on: bool) -> bool {
    use nsh_platform_windows::windows::Win32::Foundation::{COLORREF, HWND, LPARAM};
    use nsh_platform_windows::windows::Win32::System::Threading::GetCurrentProcessId;
    use nsh_platform_windows::windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GWL_EXSTYLE, GetWindowLongPtrW, GetWindowTextW, GetWindowThreadProcessId, LWA_ALPHA, SetLayeredWindowAttributes,
        SetWindowLongPtrW, WS_EX_LAYERED, WS_EX_TRANSPARENT,
    };
    use nsh_platform_windows::windows::core::BOOL;

    struct Search {
        title: Vec<u16>,
        found: Option<HWND>,
    }
    unsafe extern "system" fn visit(hwnd: HWND, param: LPARAM) -> BOOL {
        // SAFETY: `param` is the `Search` the caller below passes, alive for
        // the whole enumeration; the calls only read the window.
        unsafe {
            let search = &mut *(param.0 as *mut Search);
            let mut pid = 0;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if pid == GetCurrentProcessId() {
                let mut text = [0u16; 512];
                let len = GetWindowTextW(hwnd, &mut text).max(0) as usize;
                if text[..len] == search.title[..] {
                    search.found = Some(hwnd);
                    return BOOL(0);
                }
            }
            BOOL(1)
        }
    }
    let mut search = Search { title: title.encode_utf16().collect(), found: None };
    // SAFETY: `visit` gets a pointer to `search`, which outlives the call.
    // (Stopping the enumeration early reports an error: ignored.)
    let _ = unsafe { EnumWindows(Some(visit), LPARAM(&mut search as *mut Search as isize)) };
    let Some(hwnd) = search.found else { return false };
    let bits = (WS_EX_LAYERED.0 | WS_EX_TRANSPARENT.0) as isize;
    // SAFETY: style changes on a window of this process.
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let wanted = if on { style | bits } else { style & !bits };
        if wanted != style {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, wanted);
            if on {
                // A layered window shows nothing until it's given its alpha.
                let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA);
            }
        }
    }
    true
}

#[cfg(not(target_os = "windows"))]
pub fn ctrl_alt_down() -> bool {
    false
}

#[cfg(not(target_os = "windows"))]
pub fn set_click_through(_title: &str, _on: bool) -> bool {
    false
}

#[cfg(not(target_os = "windows"))]
pub fn cursor() -> Option<(f32, f32)> {
    None
}

#[cfg(not(target_os = "windows"))]
pub fn primary_button_down() -> bool {
    false
}
