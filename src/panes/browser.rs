//! The browser pane: the app's original webview, framed by a side pane.
//!
//! The navigation logic is unchanged from the single-view demo — address
//! resolution, back/forward/reload/stop, the event channel — because that logic
//! was correct and is unrelated to where the webview sits. What the pane adds is
//! its own toolbar, and the one hook the shell needs: the page is a native child
//! window, so the pane hides it whenever it stops being on show (risk R1b).

use std::rc::Rc;

use gpui::component::{
    ActiveTheme as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    input::{Input, InputEvent, InputState},
};
use gpui::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable, IntoElement, ParentElement as _,
    Render, SharedString, Styled as _, Subscription, Window, div, px,
};
use gpui_rsx::rsx;
use gpui_webview::WebView;

use crate::{
    icons,
    panes::{PaneContent, PaneContext, PaneProvider, PaneSlot},
};

/// Shown until the user navigates somewhere, and when they clear the address bar.
const START_PAGE_HTML: &str = include_str!("../../statics/start.html");

/// The document URL the engines report for the start page.
const START_PAGE_URL: &str = "about:blank";

/// Prefix the injected script puts in front of the page URL it reports.
const URL_MESSAGE_PREFIX: &str = "url:";

/// Reports URL changes that load no new document, such as `#anchor` links and
/// `history.pushState`, which the page load handler never sees.
const URL_REPORTER_JS: &str = include_str!("../../statics/url-reporter.js");

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
}

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
        PaneContent {
            view: view.into(),
            activation: Some(Rc::new(move |active, _window, cx| {
                let _ = for_activation.update(cx, |view, cx| view.set_webview_visible(active, cx));
            })),
        }
    }
}

/// The browser's state.
pub struct BrowserView {
    focus_handle: FocusHandle,
    /// `None` when the webview could not be created, which the pane reports
    /// instead of leaving an empty area behind.
    webview: Option<Entity<WebView>>,
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

        let built = {
            let ipc_tx = event_tx.clone();
            let builder = wry::WebViewBuilder::new()
                .with_on_page_load_handler(move |event, url| {
                    let loading = matches!(event, wry::PageLoadEvent::Started);
                    let _ = event_tx.try_send(WebViewEvent::Load { url, loading });
                })
                .with_ipc_handler(move |request| {
                    if let Some(url) = request.body().strip_prefix(URL_MESSAGE_PREFIX) {
                        let _ = ipc_tx.try_send(WebViewEvent::Url(url.to_owned()));
                    }
                })
                .with_initialization_script(URL_REPORTER_JS);
            #[cfg(any(debug_assertions, feature = "inspector"))]
            let builder = builder.with_devtools(true);

            WebView::build(builder, window, cx)
        };

        // WebView2 refuses to start while another instance holds the profile lock,
        // and reports it as a plain error. Keep the app running with a message in
        // the pane rather than panicking inside the build closure.
        let webview = match built {
            Ok(mut view) => {
                let _ = view.raw().load_html(START_PAGE_HTML);
                // Starts hidden: a pane restored into a collapsed dock is never
                // told `set_active(false)`, so only a render that actually places
                // the page may show it.
                view.hide();
                Some(cx.new(|_| view))
            }
            Err(error) => {
                log::error!("failed to create the webview: {error}");
                None
            }
        };

        let address = cx.new(|cx| InputState::new(window, cx).placeholder("Enter an address"));

        let address_events = cx.subscribe_in(
            &address,
            window,
            |this: &mut Self, input, event: &InputEvent, window, cx| match event {
                InputEvent::Change => this.address_edited = true,
                InputEvent::Blur => this.address_edited = false,
                InputEvent::PressEnter { .. } => {
                    this.address_edited = false;
                    let value = input.read(cx).value();
                    this.navigate(&value, window, cx);
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
            webview,
            address,
            loading: true,
            address_edited: false,
            error: None,
            error_address: String::new(),
            start_page: true,
            shell,
            _address_events: address_events,
        }
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
        self.set_address(shown, window, cx);
    }

    /// Show `url` in the address bar unless the user is editing it.
    fn set_address(&mut self, url: String, window: &mut Window, cx: &mut Context<Self>) {
        let editing =
            self.address_edited && self.address.read(cx).focus_handle(cx).is_focused(window);
        if editing || self.address.read(cx).value().as_ref() == url {
            return;
        }
        self.address
            .update(cx, |state, cx| state.set_value(url, window, cx));
    }

    /// Navigate to what the user typed.
    fn navigate(&mut self, input: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.set_loading(true, cx);
        self.error = None;

        if input.trim().is_empty() {
            self.show_start_page(cx);
            return;
        }

        let url = resolve_address(input);
        let Some(webview) = &self.webview else {
            return;
        };
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
        let _ = window;
    }

    /// Replace the page with the built-in start page.
    fn show_start_page(&mut self, cx: &mut Context<Self>) {
        let Some(webview) = &self.webview else {
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
        let Some(webview) = &self.webview else {
            return;
        };
        let _ = webview.update(cx, |view, _| view.back());
        self.error = None;
        self.set_loading(true, cx);
    }

    fn go_forward(&mut self, cx: &mut Context<Self>) {
        let Some(webview) = &self.webview else {
            return;
        };
        let _ = webview.update(cx, |view, _| view.forward());
        self.error = None;
        self.set_loading(true, cx);
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let Some(webview) = &self.webview else {
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
        let Some(webview) = &self.webview else {
            return;
        };
        // wry has no stop API; `window.stop()` is supported by both engines.
        let _ = webview.read(cx).raw().evaluate_script("window.stop();");
        self.set_loading(false, cx);
    }

    /// Take a navigation another pane asked the shell for.
    fn take_shell_request(&self, cx: &mut Context<Self>) -> Option<String> {
        let shell = self.shell.upgrade()?;
        shell.update(cx, |shell, _| shell.take_browser_request())
    }

    /// Show or hide the native view so it agrees with what the pane draws.
    ///
    /// The webview is a child window of the app window, not something GPUI
    /// paints, so it does not leave the screen when the element placing it does.
    /// Collapsing the dock, switching tab or drawing an error over it all drop
    /// the element and leave the page painted at its last bounds (risk R1), so
    /// the pane hides it by hand and the next layout pass places it again.
    fn set_webview_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        let Some(webview) = &self.webview else {
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
        // A viewer that cannot play a file asks the shell to load it here; the
        // request is picked up on the next frame rather than pushed, so the
        // browser pane keeps owning its own navigation.
        if let Some(url) = self.take_shell_request(cx) {
            self.navigate(&url, _window, cx);
        }

        let border = cx.theme().border;
        let danger = cx.theme().danger;
        let muted = cx.theme().muted_foreground;
        let loading = self.loading;
        let error = self.error.clone();
        let error_address = self.error_address.clone();
        let webview = self.webview.clone();
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
                    <div class="flex-1 min-w-0">
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
                    } else {
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
                    }}
                </div>
            </div>
        }
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
    fn surrounding_space_is_trimmed() {
        assert_eq!(resolve_address("  example.com  "), "https://example.com");
    }

    #[test]
    fn the_start_page_is_recognised() {
        assert!(is_start_page(""));
        assert!(is_start_page(START_PAGE_URL));
        assert!(!is_start_page("https://example.com"));
    }
}
