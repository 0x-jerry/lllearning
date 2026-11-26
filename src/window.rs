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

/// The window's native handle.
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

/// The owner handle for a prompt window, in the platform's own representation.
pub(crate) fn owner_of(window: &Window) -> Option<isize> {
    raw_hwnd(window).map(|hwnd| hwnd as isize)
}

/// Make a prompt window owned by `owner`, and optionally take the owner's input.
///
/// GPUI's `WindowKind::Dialog` cannot do this: it adopts whatever window is
/// active at creation time, and it also sets `WS_EX_APPWINDOW`, so the prompt
/// gets a taskbar button. Adopting after the fact names the right owner and
/// trades the taskbar entry for a tool window, which is what a prompt should be.
#[cfg(target_os = "windows")]
pub(crate) fn adopt_owner(window: &mut Window, owner: Option<isize>, modal: bool) {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GWL_EXSTYLE, GWLP_HWNDPARENT, GetWindowLongPtrW, SW_HIDE, SW_SHOW, SWP_FRAMECHANGED,
        SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SetWindowLongPtrW, SetWindowPos, ShowWindow,
        WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
    };

    let (Some(hwnd), Some(owner)) = (raw_hwnd(window), owner) else {
        return;
    };
    // SAFETY: both handles were obtained from live windows and are used only to
    // set styles and enable state, not to dereference.
    unsafe {
        // The platform has already shown the window by now, and it was created
        // with `WS_EX_APPWINDOW` and no owner, so it has a taskbar button. Taking
        // that style away while the window is visible leaves the button behind:
        // Windows only re-reads a window's taskbar eligibility when it is hidden
        // and shown again. Nothing has painted yet, so this is not a flicker.
        ShowWindow(hwnd, SW_HIDE);
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        let style = (style & !WS_EX_APPWINDOW) | WS_EX_TOOLWINDOW;
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style as isize);
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED,
        );
        // The owner is set through the parent slot, which for a window without
        // `WS_CHILD` means "owned by" rather than "contained in".
        SetWindowLongPtrW(hwnd, GWLP_HWNDPARENT, owner);
        ShowWindow(hwnd, SW_SHOW);
        if modal {
            EnableWindow(owner as _, 0);
        }
    }
}

/// Undo [`adopt_owner`]. Safe to call for a window that never adopted an owner,
/// and for an owner that has already gone.
#[cfg(target_os = "windows")]
pub(crate) fn release_owner(owner: Option<isize>, modal: bool) {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
    use windows_sys::Win32::UI::WindowsAndMessaging::{IsWindow, SetForegroundWindow};

    let Some(owner) = owner else {
        return;
    };
    // SAFETY: `IsWindow` validates the handle before it is used.
    unsafe {
        if IsWindow(owner as _) == 0 {
            return;
        }
        // Only a modal prompt disabled it, and re-enabling a window that was
        // never disabled would override a disable the owner set itself.
        if modal {
            EnableWindow(owner as _, 1);
        }
        SetForegroundWindow(owner as _);
    }
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn adopt_owner(_window: &mut Window, _owner: Option<isize>, _modal: bool) {}

#[cfg(not(target_os = "windows"))]
pub(crate) fn release_owner(_owner: Option<isize>, _modal: bool) {}

/// Hide the app's window, leaving the app running in the tray.
///
/// The prompt windows it owns go with it: they are owned on the platform but
/// nothing takes them off the screen when the owner is hidden.
pub(crate) fn hide_first(cx: &mut AsyncApp) {
    let Some(handle) = cx.update(|cx| cx.windows().into_iter().next()) else {
        return;
    };
    let _ = handle.update(cx, |_, window, cx| {
        crate::app::popup::close_for(window.window_handle().window_id(), cx);
        hide(window);
    });
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
