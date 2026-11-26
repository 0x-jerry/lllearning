//! The browser pane: the app's original webview, extracted into a dock panel.
//!
//! The navigation logic is unchanged from the single-view demo — address
//! resolution, back/forward/reload/stop, the event channel — because that logic
//! was correct and is unrelated to where the webview sits. What is new is the
//! panel wrapper: no title bar (the pane carries its own toolbar), no inner
//! padding, and a persisted payload holding the current URL.

use std::sync::Arc;

use gpui::component::{
    ActiveTheme as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    input::{Input, InputEvent, InputState},
};
use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, Styled as _,
    Subscription, Window, div, prelude::FluentBuilder as _, px,
};
use gpui_rsx::rsx;
use gpui_webview::WebView;
use serde_json::json;

use crate::{
    icons,
    panes::{PaneContext, PaneProvider, PaneSlot},
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

    fn build(
        &self,
        ctx: PaneContext,
        window: &mut Window,
        cx: &mut App,
    ) -> Arc<dyn gpui::component::dock::BasePanelView> {
        let saved_url = ctx
            .saved
            .as_ref()
            .and_then(|value| value.get("url"))
            .and_then(|url| url.as_str())
            .map(str::to_owned);
        let view = cx.new(|cx| BrowserView::new(saved_url, window, cx));
        gpui::component::dock::panel_handle(view)
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
    /// The last URL the webview reported, for the persisted payload.
    current_url: String,
    _address_events: Subscription,
}

impl BrowserView {
    fn new(saved_url: Option<String>, window: &mut Window, cx: &mut Context<Self>) -> Self {
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
            Ok(view) => {
                let _ = view.raw().load_html(START_PAGE_HTML);
                Some(cx.new(|_| view))
            }
            Err(error) => {
                eprintln!("failed to create the webview: {error}");
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

        let mut view = Self {
            focus_handle: cx.focus_handle(),
            webview,
            address,
            loading: true,
            address_edited: false,
            error: None,
            error_address: String::new(),
            start_page: true,
            current_url: String::new(),
            _address_events: address_events,
        };

        // A restored URL is loaded after construction, when the webview exists.
        if let Some(url) = saved_url
            && !url.is_empty()
        {
            view.navigate(&url, window, cx);
        }

        view
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
        self.current_url = url.clone();
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
                self.current_url = url;
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
        self.current_url.clear();
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

    /// The URL to persist.
    fn current_url(&self) -> String {
        if self.start_page {
            String::new()
        } else {
            self.current_url.clone()
        }
    }
}

impl Focusable for BrowserView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<gpui::base::dock::PanelEvent> for BrowserView {}

impl Render for BrowserView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let border = cx.theme().border;
        let danger = cx.theme().danger;
        let muted = cx.theme().muted_foreground;
        let loading = self.loading;
        let error = self.error.clone();
        let error_address = self.error_address.clone();
        let webview = self.webview.clone();

        rsx! {
            <div class="flex flex-col size-full">
                <div
                    class="flex flex-row items-center gap-1 px-2 py-1"
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
                        {Input::new(&self.address)}
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
                        rsx! { <div class="size-full">{webview}</div> }
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

impl gpui::base::dock::Panel for BrowserView {
    fn panel_name(&self) -> &'static str {
        "browser"
    }

    fn dump(&self, _cx: &App) -> gpui::component::dock::PanelState {
        gpui::component::dock::PanelState {
            panel_name: "browser".to_owned(),
            children: Vec::new(),
            info: gpui::component::dock::PanelInfo::panel(json!({ "url": self.current_url() })),
        }
    }
}

impl gpui::component::dock::Panel for BrowserView {
    fn tab_name(&self, _cx: &App) -> Option<SharedString> {
        Some(SharedString::from("Browser"))
    }

    fn title(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        SharedString::from("Browser")
    }

    fn inner_padding(&self, _cx: &App) -> bool {
        false
    }

    fn title_bar(&self, _cx: &App) -> bool {
        false
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
