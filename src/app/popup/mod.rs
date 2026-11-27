//! Small windows: prompts and menus. Anything that has to be seen over the
//! browser pane's WebView2 is its own top-level window owned by the window that
//! asked for it, and reports the answer through a channel.

pub mod contextmenu;
pub mod dialog;

use gpui::{App, AppContext as _, Context, Task, WindowOptions};

/// A popup window's content. A dialog and a menu differ only in what they draw
/// and when they leave, so this is the whole seam: the options, and the answer.
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
    /// Whether the owner's input was taken.
    modal: bool,
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
        // Every exit path ends here, so the owner's input is handed back once.
        crate::window::release_owner(self.owner, self.modal);
    }
}

/// Open `build`'s popup owned by `owner` and resolve with its answer. `None`
/// means it was dismissed without one.
///
/// Not from inside a draw: the new window paints its first frame before this
/// returns. Keep the returned task alive, or the window has nowhere to answer.
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
    // Right-clicking a second tab moves the menu rather than stacking another.
    // A prompt's owner is disabled, so there is never a second one.
    if !modal {
        close_for(owner_window, cx);
    }

    let (sender, receiver) = gpui::base::async_util::unbounded::<T>();
    // A dismissed popup never sends, so the channel closes and this is `None`.
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
        // After the window exists, because there is no handle to adopt before it.
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
            log::error!("could not open the popup window: {error}");
            Task::ready(None)
        }
    }
}

/// Close every popup `owner` still has open, so none floats once the owner
/// leaves the screen. An owned window does not follow its owner on its own.
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

    // Entries are never removed when a popup closes: `Drop` has no `App` to reach
    // the registry with. A handle to a closed window fails to update, so a stale
    // entry costs nothing until the owner's next hide clears the list.

    /// The popups `owner` still has open, forgetting them.
    pub(super) fn take(owner: WindowId, cx: &mut App) -> Option<Vec<AnyWindowHandle>> {
        map(cx).0.remove(&owner)
    }
}
