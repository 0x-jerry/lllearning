//! The log sink, and the startup marks the app writes to it.
//!
//! There is exactly one thing the app needs a logger for: knowing how long the
//! window stays black at startup. It is shown before its first frame is
//! presented, and the platform's window class paints it black until one is
//! (risk R19), so the width of that gap is a number only a real run can give.
//! [`mark`] writes one record per milestone — `shell built`, `first draw`,
//! `panes built`, `first frame presented` — which is also how a regression in
//! the deferral shows up: a `panes built` before `first draw` means the expensive
//! work is back in front of the frame that was supposed to go out first.
//!
//! Reading the records in order needs one caveat: the present mark is written at
//! the start of the *next* frame-loop iteration, so the real present is a hair
//! earlier and `panes built` legitimately lands between the two.
//!
//! `log` is a facade, so something must own the sink: `simple_logger` does, at a
//! fixed `info` level. There are four records and no consumer to filter for, so
//! a knob nobody turns is not worth parsing.

use std::sync::{
    OnceLock,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

/// When [`init`] ran, which is the first thing `main` does. The loader mapping
/// the executable is not counted: it is constant and small, and a `static`
/// would still only be read after `main` got control.
static START: OnceLock<Instant> = OnceLock::new();

/// Install the sink and start the clock.
pub fn init() {
    START.get_or_init(Instant::now);
    simple_logger::SimpleLogger::new()
        .with_level(log::LevelFilter::Info)
        .init()
        .expect("`logging::init` must run before anything else installs a logger");
}

/// How long the app has been running, for a caller's own record.
pub fn elapsed() -> Duration {
    START.get().map_or(Duration::ZERO, Instant::elapsed)
}

/// Write one startup mark: what happened, and how long after [`init`].
pub fn mark(what: &str) {
    log::info!("{what} at {:.1} ms", elapsed().as_secs_f64() * 1000.);
}

/// Time the first presented frame.
///
/// Called from the shell's `render`, which runs inside the first draw, so it
/// also marks when drawing began — the line to compare `panes built` against,
/// because the deferred restore must not be ahead of it (risk R19).
///
/// The callback is queued on `on_next_frame`, and those run at the start of the
/// *next* frame-loop iteration — after the present of the frame being drawn
/// now, which is the earliest point the app can observe its own pixels. A
/// callback queued before the first draw, or `defer_in`, would run before it
/// instead. Only the first call arms; later frames return here immediately.
pub fn arm_first_paint(window: &gpui::Window) {
    static ARMED: AtomicBool = AtomicBool::new(false);
    if ARMED.swap(true, Ordering::Relaxed) {
        return;
    }
    mark("first draw");
    window.on_next_frame(|_, _| mark("first frame presented"));
}
