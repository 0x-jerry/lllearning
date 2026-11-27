//! The browser pane: the app's original webview, framed by a side pane.
//!
//! The navigation logic is unchanged from the single-view demo — address
//! resolution, back/forward/reload/stop, the event channel — because that logic
//! was correct and is unrelated to where the webview sits. What the pane adds is
//! its own toolbar, and the one hook the shell needs: the page is a native child
//! window, so the pane hides it whenever it stops being on show (risk R1b).

use std::rc::Rc;
use std::time::Duration;

use gpui::component::{
    ActiveTheme as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    input::{Input, InputEvent, InputState},
};
use gpui::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, Styled as _, Subscription, Task, Window,
    div, px,
};
use gpui_rsx::rsx;
use gpui_webview::WebView;

use crate::{
    documents::status_bar_helper::request_session_save,
    icons,
    panes::{PaneContent, PaneContext, PaneController, PaneProvider, PaneSlot, ViewState},
};

/// Shown until the user navigates somewhere, and when they clear the address bar.
const START_PAGE_HTML: &str = include_str!("../../statics/start.html");

/// The document URL the engines report for the start page.
const START_PAGE_URL: &str = "about:blank";

/// What the pane keeps between runs: the page it was on.
///
/// Its own type, so the session file's shape is spelled here rather than in the
/// shell, and a value this build cannot read leaves the start page in place
/// rather than failing the restore (risk R12).
#[derive(serde::Serialize, serde::Deserialize)]
struct BrowserViewState {
    #[serde(default)]
    url: Option<String>,
}

/// The page the session said this pane was on, if it said anything usable.
fn saved_url(state: &ViewState) -> Option<String> {
    let state: BrowserViewState = serde_json::from_value(state.clone()).ok()?;
    state.url
}

/// Prefix the injected script puts in front of the page URL it reports.
const URL_MESSAGE_PREFIX: &str = "url:";

/// Runs in every page before its own scripts, and reports what GPUI cannot see:
/// URL changes with no new document, and clicks (the page is a native child
/// window, risk R1).
const SETUP_JS: &str = include_str!("../../statics/setup.js");

/// Prefix the click reporter puts in front of its message.
const CLICK_MESSAGE_PREFIX: &str = "click:";

/// Whether `url` is the start page rather than a real page.
fn is_start_page(url: &str) -> bool {
    url.is_empty() || url == START_PAGE_URL
}

/// Something the webview reports about its current page.
enum WebViewEvent {
    /// A document started or finished loading at `url`.
    Load { url: String, loading: bool },
    /// The page changed its own URL without loading a new document.
    Url(String),
    /// Someone clicked in the page.
    Click,
}

/// What a page message reports, read by the prefix its reporter put on it.
fn reported_event(body: &str) -> Option<WebViewEvent> {
    if body.starts_with(CLICK_MESSAGE_PREFIX) {
        return Some(WebViewEvent::Click);
    }
    body.strip_prefix(URL_MESSAGE_PREFIX)
        .map(|url| WebViewEvent::Url(url.to_owned()))
}

/// The native page, and what the pane knows about starting it.
///
/// The build is a state of its own rather than an `Option` the pane reports as a
/// failure: WebView2 starts its browser process asynchronously, and until it is
/// done there is nothing native to show and nothing wrong.
enum Page {
    /// The build is in flight on this thread's own message loop.
    Starting,
    Ready(Entity<WebView>),
    /// WebView2 refused to start — another instance holds the profile lock
    /// (risk R2) — and the pane explains that instead of leaving an empty area.
    Failed,
}

/// What the build task hands back: the page as wry built it, or already wrapped
/// where the platform builds it from a window reference.
#[cfg(target_os = "windows")]
type BuiltPage = wry::WebView;
#[cfg(not(target_os = "windows"))]
type BuiltPage = Entity<WebView>;

/// Turn what the user typed into a URL to navigate to.
///
/// A bare host such as `example.com` or `localhost:3000` gets an `https://`
/// scheme, so it loads instead of failing as an unsupported scheme. Anything
/// else is passed through unchanged: a real scheme keeps working, and input
/// that is not a URL at all reaches the webview as typed, where it fails and
/// the error page takes over.
fn resolve_address(input: &str) -> String {
    let input = input.trim();
    let is_bare_host = !input.contains("://")
        && input
            .split(['/', '?', '#'])
            .next()
            .is_some_and(|authority| {
                let host = authority.split(':').next().unwrap_or_default();
                let mut labels = host.split('.');
                labels.next().is_some_and(|label| !label.is_empty())
                    && labels.all(|label| !label.is_empty())
            });

    if is_bare_host {
        format!("https://{input}")
    } else {
        input.to_owned()
    }
}

/// The browser's provider.
pub struct BrowserPane;

impl PaneProvider for BrowserPane {
    fn id(&self) -> &'static str {
        "browser"
    }

    fn title(&self) -> SharedString {
        SharedString::from("Browser")
    }

    fn icon(&self) -> lucide_icons::Icon {
        lucide_icons::Icon::Globe
    }

    fn slot(&self) -> PaneSlot {
        PaneSlot::Right
    }

    fn order(&self) -> i32 {
        0
    }

    fn build(&self, ctx: PaneContext, window: &mut Window, cx: &mut App) -> PaneContent {
        let view = cx.new(|cx| BrowserView::new(ctx.shell, window, cx));
        // The page is a native child window, so it does not leave the screen
        // when the element placing it does (risk R1b). The shell tells the pane
        // whenever it stops being the one on show, and the pane hides the page
        // itself.
        let for_activation = view.downgrade();
        let controller: Rc<dyn PaneController> = Rc::new(BrowserController { view: view.clone() });
        PaneContent {
            view: view.into(),
            controller,
            activation: Some(Rc::new(move |active, _window, cx| {
                let _ = for_activation.update(cx, |view, cx| view.set_webview_visible(active, cx));
            })),
        }
    }
}

/// The browser's state.
pub struct BrowserView {
    focus_handle: FocusHandle,
    /// The native page, if it has started.
    page: Page,
    /// Whether the pane is on screen. Remembered separately from the page
    /// because the page can arrive long after the transition that decided it
    /// (risk R1b).
    wanted_visible: bool,
    /// A navigation asked for before the page existed: an address typed in that
    /// window, a viewer that cannot play a file, or the page the session saved.
    pending_url: Option<String>,
    address: Entity<InputState>,
    loading: bool,
    address_edited: bool,
    /// Set while a failed navigation replaces the page.
    error: Option<String>,
    /// What the user asked for, shown alongside the error.
    error_address: String,
    /// Whether the page on screen is the start page, which has no URL to reload.
    start_page: bool,
    /// The shell, for the navigations other panes route through it.
    shell: gpui::WeakEntity<crate::app::shell::AppShell>,
    /// Kept so the build is not cancelled mid-flight, which would leak the
    /// container window wry creates before its first await.
    _build_task: Option<Task<()>>,
    _address_events: Subscription,
}

impl BrowserView {
    fn new(
        shell: gpui::WeakEntity<crate::app::shell::AppShell>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // Webview events arrive on the platform's own callback, so they are
        // funnelled through a channel into a foreground task that updates the view.
        let (event_tx, event_rx) = gpui::base::async_util::unbounded::<WebViewEvent>();

        let ipc_tx = event_tx.clone();
        let builder = wry::WebViewBuilder::new()
            .with_on_page_load_handler(move |event, url| {
                let loading = matches!(event, wry::PageLoadEvent::Started);
                let _ = event_tx.try_send(WebViewEvent::Load { url, loading });
            })
            .with_ipc_handler(move |request| {
                if let Some(event) = reported_event(request.body()) {
                    let _ = ipc_tx.try_send(event);
                }
            })
            .with_initialization_script(SETUP_JS)
            // Born hidden: only the element that places the page may show it,
            // and a build that fails or is cancelled must not paint an empty
            // native rectangle over the pane.
            .with_visible(false);
        #[cfg(any(debug_assertions, feature = "inspector"))]
        let builder = builder.with_devtools(true);

        #[cfg(target_os = "windows")]
        let parent = crate::window::ParentWindow::of(window);

        let build_task = cx.spawn_in(window, async move |this, cx| {
            // One executor turn first: a background timer resolves between frame
            // iterations, so the frame carrying the dock and this pane's
            // placeholder goes out before WebView2 starts (risk R19).
            cx.background_executor().timer(Duration::ZERO).await;
            crate::logging::mark("webview build started");

            // `WebView::build` waits for WebView2 inside a nested Win32 pump, so
            // every frame and task stalls for the length of that wait. wry's
            // async path awaits the same two steps instead, which leaves GPUI's
            // own loop draining messages (risk R21).
            #[cfg(target_os = "windows")]
            let started = match parent {
                Some(parent) => builder
                    .build_as_child_async(&parent)
                    .await
                    .map_err(|error| error.to_string()),
                None => Err("the window has no native handle".to_owned()),
            };
            #[cfg(not(target_os = "windows"))]
            let started = cx
                .update(|window, cx| {
                    WebView::build(builder, window, cx)
                        .map(|mut view| {
                            view.hide();
                            cx.new(|_| view)
                        })
                        .map_err(|error| error.to_string())
                })
                .unwrap_or_else(|_| Err("the window is gone".to_owned()));

            let _ = this.update_in(cx, |this, window, cx| this.page_ready(started, window, cx));
        });

        let address = cx.new(|cx| InputState::new(window, cx).placeholder("Enter an address"));

        let address_events = cx.subscribe_in(
            &address,
            window,
            |this: &mut Self, input, event: &InputEvent, _window, cx| match event {
                InputEvent::Change => this.address_edited = true,
                InputEvent::Blur => this.address_edited = false,
                InputEvent::PressEnter { .. } => {
                    this.address_edited = false;
                    let value = input.read(cx).value();
                    this.navigate(&value, cx);
                }
                _ => {}
            },
        );

        cx.spawn_in(window, async move |this, cx| {
            while let Ok(event) = event_rx.recv().await {
                let done = this.update_in(cx, |this, window, cx| {
                    this.webview_event(event, window, cx);
                });
                if done.is_err() {
                    break;
                }
            }
        })
        .detach();

        Self {
            focus_handle: cx.focus_handle(),
            page: Page::Starting,
            wanted_visible: false,
            pending_url: None,
            address,
            loading: false,
            address_edited: false,
            error: None,
            error_address: String::new(),
            start_page: true,
            shell,
            _build_task: Some(build_task),
            _address_events: address_events,
        }
    }

    /// The page, once it has started.
    fn webview(&self) -> Option<&Entity<WebView>> {
        match &self.page {
            Page::Ready(view) => Some(view),
            Page::Starting | Page::Failed => None,
        }
    }

    /// Adopt what the build task produced.
    fn page_ready(
        &mut self,
        built: Result<BuiltPage, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.page = match built {
            #[cfg(target_os = "windows")]
            Ok(raw) => {
                let mut view = WebView::new(raw, window, cx);
                view.hide();
                Page::Ready(cx.new(|_| view))
            }
            #[cfg(not(target_os = "windows"))]
            Ok(view) => Page::Ready(view),
            Err(message) => {
                log::error!("failed to create the webview: {message}");
                Page::Failed
            }
        };

        if matches!(self.page, Page::Failed) {
            self.set_loading(false, cx);
        } else if let Some(url) = self.pending_url.take() {
            self.navigate(&url, cx);
        } else {
            self.show_start_page(cx);
        }
        // The dock may have hidden or closed this pane while the page was
        // starting, and only this decides whether it goes on screen (risk R1b).
        self.set_webview_visible(self.wanted_visible, cx);
        crate::logging::mark("webview ready");
        cx.notify();
    }

    fn set_loading(&mut self, loading: bool, cx: &mut Context<Self>) {
        if self.loading != loading {
            self.loading = loading;
            cx.notify();
        }
    }

    /// Reflect what the webview reports in the toolbar.
    fn webview_event(&mut self, event: WebViewEvent, window: &mut Window, cx: &mut Context<Self>) {
        let url = match event {
            WebViewEvent::Click => {
                self.blur_address(window, cx);
                return;
            }
            WebViewEvent::Load { url, loading } => {
                if self.loading != loading {
                    self.loading = loading;
                    cx.notify();
                }
                self.start_page = is_start_page(&url);
                url
            }
            WebViewEvent::Url(url) => url,
        };
        let shown = if is_start_page(&url) {
            String::new()
        } else {
            url
        };
        // Only a change to the bar is worth a write, and a navigation changes no
        // document and no layout: without this request the address would only
        // be written by the next unrelated save, or by the quit hook, which a
        // killed process never reaches.
        if self.set_address(shown, window, cx) {
            request_session_save(&self.shell, window, cx);
        }
    }

    /// Take the focus off the address bar, so typing goes back where it was.
    fn blur_address(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.address.read(cx).focus_handle(cx).is_focused(window) {
            window.blur(cx);
        }
    }

    /// Show `url` in the address bar unless the user is editing it, and report
    /// whether the bar changed.
    fn set_address(&mut self, url: String, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let editing =
            self.address_edited && self.address.read(cx).focus_handle(cx).is_focused(window);
        if editing || self.address.read(cx).value().as_ref() == url {
            return false;
        }
        self.address
            .update(cx, |state, cx| state.set_value(url, window, cx));
        true
    }

    /// Navigate to what the user typed.
    fn navigate(&mut self, input: &str, cx: &mut Context<Self>) {
        if input.trim().is_empty() {
            // Nothing pending means the start page, which `page_ready` loads.
            self.pending_url = None;
            self.error = None;
            self.show_start_page(cx);
            return;
        }

        let url = resolve_address(input);
        let webview = self.webview().cloned();
        let Some(webview) = webview else {
            // Still starting: the page replays the request when it arrives, so a
            // navigation asked for in that window is not lost.
            self.pending_url = Some(url);
            return;
        };

        self.set_loading(true, cx);
        self.error = None;
        let started = webview
            .read(cx)
            .raw()
            .load_url(&url)
            .map_err(|error| error.to_string());
        match started {
            Ok(()) => {
                self.start_page = false;
            }
            Err(message) => {
                self.error = Some(message);
                self.error_address = input.trim().to_owned();
                self.set_loading(false, cx);
            }
        }
    }

    /// Replace the page with the built-in start page.
    fn show_start_page(&mut self, cx: &mut Context<Self>) {
        let Some(webview) = self.webview() else {
            return;
        };
        let loaded = webview
            .read(cx)
            .raw()
            .load_html(START_PAGE_HTML)
            .map_err(|error| error.to_string());
        self.start_page = loaded.is_ok();
        if let Err(message) = loaded {
            self.error = Some(message);
            self.error_address.clear();
        }
        self.set_loading(false, cx);
    }

    fn go_back(&mut self, cx: &mut Context<Self>) {
        let Some(webview) = self.webview() else {
            return;
        };
        let _ = webview.update(cx, |view, _| view.back());
        self.error = None;
        self.set_loading(true, cx);
    }

    fn go_forward(&mut self, cx: &mut Context<Self>) {
        let Some(webview) = self.webview() else {
            return;
        };
        let _ = webview.update(cx, |view, _| view.forward());
        self.error = None;
        self.set_loading(true, cx);
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let webview = self.webview().cloned();
        let Some(webview) = webview else {
            return;
        };
        self.error = None;
        // The start page is an in-memory document, so it is shown again rather
        // than reloaded through a URL it does not have.
        if self.start_page {
            self.set_loading(true, cx);
            self.show_start_page(cx);
            return;
        }
        let _ = webview.read(cx).raw().reload();
        self.set_loading(true, cx);
    }

    fn stop(&mut self, cx: &mut Context<Self>) {
        let Some(webview) = self.webview() else {
            return;
        };
        // wry has no stop API; `window.stop()` is supported by both engines.
        let _ = webview.read(cx).raw().evaluate_script("window.stop();");
        self.set_loading(false, cx);
    }

    /// Show or hide the native view so it agrees with what the pane draws.
    ///
    /// The webview is a child window of the app window, not something GPUI
    /// paints, so it does not leave the screen when the element placing it does.
    /// Collapsing the dock, switching tab or drawing an error over it all drop
    /// the element and leave the page painted at its last bounds (risk R1), so
    /// the pane hides it by hand and the next layout pass places it again.
    fn set_webview_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        // Remembered even with no page yet: WebView2 can take hundreds of
        // milliseconds to start, and the dock may be hidden or closed before it
        // does.
        self.wanted_visible = visible;
        let Some(webview) = self.webview() else {
            return;
        };
        webview.update(cx, |view, _| {
            if view.visible() == visible {
                return;
            }
            if visible {
                view.show();
            } else {
                view.hide();
            }
        });
    }
}

impl Focusable for BrowserView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for BrowserView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let border = cx.theme().border;
        let danger = cx.theme().danger;
        let muted = cx.theme().muted_foreground;
        let loading = self.loading;
        let error = self.error.clone();
        let error_address = self.error_address.clone();
        let webview = self.webview().cloned();
        let build_failed = matches!(self.page, Page::Failed);
        // The element below is the only thing that places the page, so this is
        // the only place allowed to show it: whatever this render decides to
        // draw, the native child window has to agree. Doing it here rather than
        // from every path that sets `error` leaves one place that can be wrong.
        self.set_webview_visible(error.is_none(), cx);

        rsx! {
            <div class="flex flex-col size-full">
                <div
                    class="flex flex-row items-center gap-1 px-2"
                    h={px(30.)}
                    border_b_1
                    border_color={border}
                >
                    <Button("browser-back")
                        ghost
                        xsmall
                        tooltip={"Back"}
                        onClick={cx.listener(|this, _, _, cx| this.go_back(cx))}
                    >
                        {icons::icon(lucide_icons::Icon::ArrowLeft, 14.)}
                    </Button>
                    <Button("browser-forward")
                        ghost
                        xsmall
                        tooltip={"Forward"}
                        onClick={cx.listener(|this, _, _, cx| this.go_forward(cx))}
                    >
                        {icons::icon(lucide_icons::Icon::ArrowRight, 14.)}
                    </Button>
                    {if loading {
                        rsx! {
                            <Button("browser-stop")
                                ghost
                                xsmall
                                tooltip={"Stop"}
                                onClick={cx.listener(|this, _, _, cx| this.stop(cx))}
                            >
                                {icons::icon(lucide_icons::Icon::X, 14.)}
                            </Button>
                        }
                    } else {
                        rsx! {
                            <Button("browser-reload")
                                ghost
                                xsmall
                                tooltip={"Reload"}
                                onClick={cx.listener(|this, _, _, cx| this.reload(cx))}
                            >
                                {icons::icon(lucide_icons::Icon::RotateCw, 14.)}
                            </Button>
                        }
                    }}
                    <div
                        class="flex-1 min-w-0"
                        on_mouse_down_out={cx.listener(|this, _, window, cx| {
                            this.blur_address(window, cx)
                        })}
                    >
                        {Input::new(&self.address).small()}
                    </div>
                </div>
                <div class="flex-1 min-h-0 overflow-hidden">
                    {if let Some(message) = error {
                        rsx! {
                            <div class="flex flex-col size-full items-center justify-center gap-2 p-4">
                                {icons::icon(lucide_icons::Icon::TriangleAlert, 24.)
                                    .text_color(danger)}
                                <div class="text-sm">{"This page could not be loaded"}</div>
                                <div class="text-xs" text_color={muted}>{error_address}</div>
                                <div class="text-xs" text_color={muted}>{message}</div>
                            </div>
                        }
                    } else if let Some(webview) = webview {
                        rsx! { <div class="size-full" pl={px(5.)}>{webview}</div> }
                    } else if build_failed {
                        rsx! {
                            <div class="flex flex-col size-full items-center justify-center gap-2 p-4">
                                {icons::icon(lucide_icons::Icon::TriangleAlert, 24.)
                                    .text_color(danger)}
                                <div class="text-sm">{"The web view could not start"}</div>
                                <div class="text-xs" text_color={muted}>
                                    {"Close the other window, then reopen the app."}
                                </div>
                            </div>
                        }
                    } else {
                        rsx! {
                            <div class="flex flex-col size-full items-center justify-center gap-2 p-4">
                                {icons::icon(lucide_icons::Icon::Globe, 24.).text_color(muted)}
                                <div class="text-sm">{"Starting the web view…"}</div>
                            </div>
                        }
                    }}
                </div>
            </div>
        }
    }
}

/// What the dock asks for the pane's state, and hands back on the build after a
/// restore.
struct BrowserController {
    view: Entity<BrowserView>,
}

impl PaneController for BrowserController {
    fn view_state(&self, cx: &App) -> Option<ViewState> {
        // The address bar *is* what the pane knows about the page: it is filled
        // from the engine's load and history events, and it is empty on the
        // start page or when the user clears it, which is what "nothing to
        // come back to" means. The loading state and the error on show are
        // consequences of a navigation, not something to replay.
        let url = self.view.read(cx).address.read(cx).value().to_string();
        if url.is_empty() {
            return None;
        }
        match serde_json::to_value(BrowserViewState { url: Some(url) }) {
            Ok(state) => Some(state),
            // A state that cannot be encoded is a wiring bug, not something the
            // user did: the pane opens on the start page next time.
            Err(error) => {
                log::warn!("could not encode the browser view state: {error}");
                None
            }
        }
    }

    fn restore_view_state(&self, state: &ViewState, _window: &mut Window, cx: &mut App) {
        let Some(url) = saved_url(state) else {
            return;
        };
        // The page starts asynchronously, so this is almost always a request
        // ahead of it; `navigate` queues it and `page_ready` replays it.
        self.view.update(cx, |view, cx| view.navigate(&url, cx));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_host_gets_https() {
        assert_eq!(resolve_address("example.com"), "https://example.com");
        assert_eq!(resolve_address("localhost:3000"), "https://localhost:3000");
        assert_eq!(
            resolve_address("example.com/a/b?q=1"),
            "https://example.com/a/b?q=1"
        );
    }

    #[test]
    fn an_explicit_scheme_is_left_alone() {
        // Anything with `://` is already a URL, so it is passed through.
        assert_eq!(resolve_address("https://a.b"), "https://a.b");
        assert_eq!(resolve_address("file:///tmp/a.html"), "file:///tmp/a.html");
        assert_eq!(
            resolve_address("http://localhost:8080/x"),
            "http://localhost:8080/x"
        );
    }

    #[test]
    fn a_scheme_without_slashes_is_treated_as_a_bare_host() {
        // Preserved from the original demo: only `://` marks a URL as one, so a
        // bare `scheme:rest` still gets the https prefix. The webview then
        // fails on it and the error page takes over, which is the documented
        // behaviour rather than a silent success.
        assert_eq!(resolve_address("about:blank"), "https://about:blank");
    }

    #[test]
    fn input_that_is_not_a_url_is_prefixed_like_a_host() {
        // Also preserved: a single word or a phrase looks like a bare host to
        // the heuristic, so it becomes an https URL and fails at the engine
        // with the error page. Only input with `://` bypasses the prefix.
        assert_eq!(resolve_address("hello world"), "https://hello world");
        assert_eq!(resolve_address(""), "");
    }

    #[test]
    fn a_page_message_is_read_by_its_prefix() {
        assert!(matches!(
            reported_event("click:"),
            Some(WebViewEvent::Click)
        ));
        assert!(matches!(
            reported_event("url:https://a.b"),
            Some(WebViewEvent::Url(url)) if url == "https://a.b"
        ));
        assert!(reported_event("hello").is_none());
    }

    #[test]
    fn surrounding_space_is_trimmed() {
        assert_eq!(resolve_address("  example.com  "), "https://example.com");
    }

    #[test]
    fn the_start_page_is_recognised() {
        assert!(is_start_page(""));
        assert!(is_start_page(START_PAGE_URL));
        assert!(!is_start_page("https://example.com"));
    }

    #[test]
    fn a_saved_page_is_read_back() {
        let state = serde_json::json!({ "url": "https://a.b/c" });
        assert_eq!(saved_url(&state).as_deref(), Some("https://a.b/c"));
    }

    #[test]
    fn a_saved_state_this_build_cannot_read_leaves_the_start_page() {
        assert_eq!(saved_url(&serde_json::json!({ "layout": "split" })), None);
        assert_eq!(saved_url(&serde_json::json!(42)), None);
    }
}
