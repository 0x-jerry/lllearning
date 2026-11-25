//! Showing and hiding the app window.
//!
//! GPUI exposes minimizing but not hiding, so closing to the tray would leave a
//! taskbar entry behind. The platform calls below hide the window outright, and
//! bring it back to the front when the tray asks for it.

use gpui::{AsyncApp, Window};

/// Show the window and raise it above the other windows on screen.
#[cfg(target_os = "windows")]
pub(crate) fn show(window: &mut Window) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        HWND_NOTOPMOST, HWND_TOPMOST, SW_SHOW, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW,
        SetForegroundWindow, SetWindowPos, ShowWindow,
    };

    let Some(hwnd) = raw_hwnd(window) else {
        return;
    };
    // SAFETY: the handle belongs to this window and stays alive while it is used.
    unsafe {
        ShowWindow(hwnd, SW_SHOW);
        // Showing a hidden window restores the z-order it had when it was hidden,
        // which is usually behind whatever the user moved on to. Passing through
        // the topmost band raises it above the other windows, and unlike
        // `SetForegroundWindow` alone it does not need foreground rights.
        SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
        SetWindowPos(
            hwnd,
            HWND_NOTOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
        );
        // Also give it the keyboard, when Windows lets this process take the
        // foreground. Choosing a tray menu item normally does.
        SetForegroundWindow(hwnd);
    }
}

#[cfg(target_os = "windows")]
pub(crate) fn hide(window: &mut Window) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{SW_HIDE, ShowWindow};

    let Some(hwnd) = raw_hwnd(window) else {
        return;
    };
    // SAFETY: the handle belongs to this window and stays alive while it is used.
    unsafe { ShowWindow(hwnd, SW_HIDE) };
}

/// The window's native handle, for the Win32 calls above.
#[cfg(target_os = "windows")]
fn raw_hwnd(window: &Window) -> Option<*mut core::ffi::c_void> {
    use raw_window_handle::RawWindowHandle;

    // Named through the trait: the inherent `Window::window_handle` returns an
    // `AnyWindowHandle` instead of the raw one.
    let handle = raw_window_handle::HasWindowHandle::window_handle(window).ok()?;
    match handle.as_raw() {
        RawWindowHandle::Win32(win32) => Some(win32.hwnd.get() as *mut core::ffi::c_void),
        _ => None,
    }
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn hide(window: &mut Window) {
    window.minimize_window();
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn show(window: &mut Window) {
    window.activate_window();
}

/// Bring the app's window back and raise it to the front.
pub(crate) fn show_first(cx: &mut AsyncApp) {
    with_window(cx, show);
}

/// Hide the app's window, leaving the app running in the tray.
pub(crate) fn hide_first(cx: &mut AsyncApp) {
    with_window(cx, hide);
}

/// Run `f` on the app's window, or do nothing when there is none.
///
/// Not `active_window()`: a hidden or unfocused window is not the platform's
/// active one, and the tray acts on exactly those. The app's own window list does
/// not depend on focus or visibility.
fn with_window(cx: &mut AsyncApp, f: impl FnOnce(&mut Window)) {
    let Some(handle) = cx.update(|cx| cx.windows().into_iter().next()) else {
        return;
    };
    let _ = handle.update(cx, |_, window, _| f(window));
}
