//! Small windows: prompts and menus.
//!
//! `window.open_dialog` and an in-window popover both draw into the main
//! window's element tree, where the browser pane's WebView2 child window paints
//! over them (risk R1, risk R17). Anything that has to be seen over the page is
//! therefore its own small top-level window, owned by the window that asked for
//! it: the platform keeps it above the owner, and it reports what the user chose
//! back through a channel.
//!
//! [`dialog`] is a modal prompt with footer buttons; [`contextmenu`] is a
//! positioned menu that closes itself when it loses focus.

pub mod contextmenu;
pub mod dialog;

use gpui::{App, AppContext as _, Context, Task, WindowOptions};

/// A popup window's content.
///
/// A dialog and a context menu differ only in what they draw and in how they
/// decide to leave, so this is the whole seam between them: the window's
/// options, and the answer it reports.
pub(crate) trait Popup: gpui::Render + Sized + 'static {
    /// What the popup reports when it is answered.
    type Answer;

    /// The window's placement, chrome and size.
    fn options(&self) -> WindowOptions;

    /// Called once the window exists, with the view already built.
    fn opened(&mut self, _window: &mut gpui::Window, _cx: &mut Context<Self>) {}
}

/// The window and the answer channel every popup shares.
pub(crate) struct Host<T> {
    sender: Option<gpui::base::async_util::Sender<T>>,
    /// The owner's native handle, so `Drop` can hand its input back.
    owner: Option<isize>,
    /// Whether the owner's input was taken, so it is only handed back if it was.
    modal: bool,
    /// The owner, for the registry the hide paths walk.
    owner_window: gpui::WindowId,
}

impl<T> Host<T> {
    /// The owner this popup is filed under.
    #[allow(dead_code)]
    pub(crate) fn owner_window(&self) -> gpui::WindowId {
        self.owner_window
    }

    /// Whether the owner's input was taken, so it is only handed back if it was.
    pub(crate) fn modal(&self) -> bool {
        self.modal
    }

    /// Hand the answer over, if there is one, and close the window.
    pub(crate) fn answer(&mut self, value: Option<T>, window: &mut gpui::Window) {
        if let (Some(sender), Some(value)) = (self.sender.take(), value) {
            let _ = sender.try_send(value);
        }
        window.remove_window();
    }
}

impl<T> Drop for Host<T> {
    fn drop(&mut self) {
        // Every exit path — a button, Escape, a click outside, Alt+F4, the owner
        // going away — ends here, so the owner's input is handed back once.
        crate::window::release_owner(self.owner, self.modal);
    }
}

/// Open `build`'s popup owned by `owner` and resolve with its answer.
///
/// `None` means the window was dismissed without one, so a caller that needs an
/// answer must treat it as one.
///
/// Must not be called while a window is being drawn: the new window paints its
/// first frame before this returns. Defer it, as `AppShell` does.
///
/// Keep the returned task alive. Dropping it drops the receiving end, and the
/// window then has nowhere to report its answer.
pub(crate) fn open<V, T>(
    owner: &gpui::Window,
    cx: &mut App,
    owner_native: Option<isize>,
    modal: bool,
    build: impl FnOnce(Host<T>) -> V,
) -> Task<Option<T>>
where
    V: Popup<Answer = T> + 'static,
    T: 'static,
{
    let owner_window = owner.window_handle().window_id();
    // A menu is a place, not a pile: right-clicking a second tab must move the
    // menu rather than leave the first one behind. A prompt is the opposite —
    // its owner is disabled, so there is never a second one to collide with.
    if !modal {
        close_for(owner_window, cx);
    }

    let (sender, receiver) = gpui::base::async_util::unbounded::<T>();
    // A dismissed popup never sends, so the channel closes instead and this
    // resolves to `None` with no extra plumbing.
    let answered = cx.spawn(async move |_| receiver.recv().await.ok());

    let host = Host {
        sender: Some(sender),
        owner: owner_native,
        modal,
        owner_window,
    };
    let mut built = Some(build(host));
    let options = built
        .as_ref()
        .map(Popup::options)
        .expect("a popup is built once");

    let opened = gpui::open_window(options, cx, move |window, cx| {
        // After the window exists: there is no handle to adopt before it. It is
        // already shown by now, which `adopt_owner` has to undo and redo to shed
        // the taskbar button it was created with.
        crate::window::adopt_owner(window, owner_native, modal);
        let popup = built.take().expect("the popup is built once");
        let view = cx.new(|_| popup);
        view.update(cx, |popup, cx| popup.opened(window, cx));
        registry::add(window.window_handle(), owner_window, cx);
        view
    });

    match opened {
        Ok(_) => answered,
        Err(error) => {
            eprintln!("could not open the popup window: {error}");
            Task::ready(None)
        }
    }
}

/// Close every popup `owner` still has open, so none is left floating once the
/// owner leaves the screen.
///
/// The tray's Hide and the close-to-tray guard both take the owner away without
/// it being closed, which no owned window follows on its own.
pub fn close_for(owner: gpui::WindowId, cx: &mut App) {
    let Some(popups) = registry::take(owner, cx) else {
        return;
    };
    for popup in popups {
        let _ = popup.update(cx, |_, window, _| window.remove_window());
    }
}

/// Which popups are open, by the window that owns them.
mod registry {
    use std::collections::HashMap;

    use gpui::{AnyWindowHandle, App, Global, WindowId};

    #[derive(Default)]
    struct OpenPopups(HashMap<WindowId, Vec<AnyWindowHandle>>);

    impl Global for OpenPopups {}

    fn map(cx: &mut App) -> &mut OpenPopups {
        if cx.try_global::<OpenPopups>().is_none() {
            cx.set_global(OpenPopups::default());
        }
        cx.global_mut::<OpenPopups>()
    }

    pub(super) fn add(popup: AnyWindowHandle, owner: WindowId, cx: &mut App) {
        map(cx).0.entry(owner).or_default().push(popup);
    }

    // Entries are never removed when a popup closes: `Drop` has no `App` to
    // reach the registry with. They stay until the owner's next hide clears the
    // list, and a handle to a closed window simply fails to update, so the stale
    // ones cost nothing but their place in the list.

    /// The popups `owner` still has open, forgetting them.
    pub(super) fn take(owner: WindowId, cx: &mut App) -> Option<Vec<AnyWindowHandle>> {
        map(cx).0.remove(&owner)
    }
}
