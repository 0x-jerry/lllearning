//! Showing and hiding the app window.
//!
//! GPUI exposes minimizing but not hiding, so closing to the tray would leave a
//! taskbar entry behind. The platform calls below hide the window outright, and
//! the tray's Show item brings it back.

use gpui::{App, Window};

#[cfg(target_os = "windows")]
pub(crate) fn hide(window: &mut Window) {
    use raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
    use windows_sys::Win32::UI::WindowsAndMessaging::{SW_HIDE, ShowWindow};

    let Ok(handle) = window.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return;
    };
    let hwnd = win32.hwnd.get() as *mut core::ffi::c_void;
    // SAFETY: the handle belongs to this window and stays alive while it is used.
    unsafe { ShowWindow(hwnd, SW_HIDE) };
}

#[cfg(target_os = "windows")]
pub(crate) fn show(window: &mut Window) {
    use raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
    use windows_sys::Win32::UI::WindowsAndMessaging::{SW_SHOW, ShowWindow};

    let Ok(handle) = window.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return;
    };
    let hwnd = win32.hwnd.get() as *mut core::ffi::c_void;
    // SAFETY: the handle belongs to this window and stays alive while it is used.
    unsafe { ShowWindow(hwnd, SW_SHOW) };
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn hide(window: &mut Window) {
    window.minimize_window();
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn show(window: &mut Window) {
    window.activate_window();
}

/// Bring the app's window back, or do nothing when there is none yet.
pub(crate) fn show_active(cx: &mut App) {
    let Some(handle) = cx
        .active_window()
        .or_else(|| cx.window_stack().and_then(|stack| stack.first().copied()))
    else {
        return;
    };
    let _ = handle.update(cx, |_, window, _| show(window));
}
