//! Filesystem watching. `notify` emits several events per logical change (a
//! rename pair for one save, temp-file writes); this turns that stream into one
//! deduplicated, debounced batch.

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    time::Duration,
};

use gpui::base::async_util::{Receiver, unbounded};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher as _};

/// How long to wait for the stream to go quiet before reporting a batch.
/// Long enough to swallow a rename pair, short enough to feel immediate.
const DEBOUNCE: Duration = Duration::from_millis(250);

/// A running recursive watcher. Dropping this stops watching, which closes the
/// receiver the drain task waits on.
pub struct FileWatcher {
    _inner: RecommendedWatcher,
}

impl FileWatcher {
    /// Watch `root` recursively. Every batch is deduplicated and sorted.
    pub fn start(root: PathBuf) -> notify::Result<(Self, Receiver<Vec<PathBuf>>)> {
        let (raw_tx, raw_rx) = std::sync::mpsc::channel::<notify::Result<Event>>();

        let mut inner = notify::recommended_watcher(move |event: notify::Result<Event>| {
            // A send failure means the app is shutting down.
            let _ = raw_tx.send(event);
        })?;
        inner.watch(&root, RecursiveMode::Recursive)?;

        let (batch_tx, batch_rx) = unbounded::<Vec<PathBuf>>();

        // On its own thread rather than a GPUI task: a blocking loop over a std
        // channel must keep draining while the UI thread renders.
        std::thread::Builder::new()
            .name("lllearning-watch".to_owned())
            .spawn(move || {
                let mut pending = BTreeSet::new();
                while let Ok(event) = raw_rx.recv() {
                    collect(event, &mut pending);
                    loop {
                        match raw_rx.recv_timeout(DEBOUNCE) {
                            Ok(event) => collect(event, &mut pending),
                            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => break,
                            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                                let _ = batch_tx.try_send(pending.into_iter().collect());
                                return;
                            }
                        }
                    }
                    if pending.is_empty() {
                        continue;
                    }
                    let batch: Vec<PathBuf> = std::mem::take(&mut pending).into_iter().collect();
                    if batch_tx.try_send(batch).is_err() {
                        break;
                    }
                }
            })
            .expect("spawn the watcher thread");

        Ok((Self { _inner: inner }, batch_rx))
    }
}

/// Add an event's paths to the pending set, ignoring events with none.
fn collect(event: notify::Result<Event>, pending: &mut BTreeSet<PathBuf>) {
    let Ok(event) = event else { return };
    // Access events are reads, which a save does not need to report.
    if matches!(event.kind, EventKind::Access(_)) {
        return;
    }
    pending.extend(event.paths);
}

/// Whether `path` is under `root`.
#[allow(dead_code)]
pub fn is_within(root: &Path, path: &Path) -> bool {
    path.starts_with(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_below_the_root_is_within_it() {
        assert!(is_within(Path::new("/w"), Path::new("/w/a.txt")));
        assert!(!is_within(Path::new("/w"), Path::new("/elsewhere/a.txt")));
    }

    #[test]
    fn an_access_event_is_not_a_change() {
        let mut pending = BTreeSet::new();
        let event = Event {
            kind: EventKind::Access(notify::event::AccessKind::Read),
            paths: vec![PathBuf::from("/w/a.txt")],
            attrs: Default::default(),
        };
        collect(Ok(event), &mut pending);
        assert!(
            pending.is_empty(),
            "reading a file is not a change to report"
        );
    }

    #[test]
    fn a_modify_event_contributes_its_paths() {
        let mut pending = BTreeSet::new();
        let event = Event {
            kind: EventKind::Modify(notify::event::ModifyKind::Any),
            paths: vec![PathBuf::from("/w/a.txt"), PathBuf::from("/w/b.txt")],
            attrs: Default::default(),
        };
        collect(Ok(event), &mut pending);
        assert_eq!(pending.len(), 2);
    }

    #[test]
    fn duplicate_paths_collapse() {
        let mut pending = BTreeSet::new();
        for _ in 0..3 {
            let event = Event {
                kind: EventKind::Modify(notify::event::ModifyKind::Any),
                paths: vec![PathBuf::from("/w/a.txt")],
                attrs: Default::default(),
            };
            collect(Ok(event), &mut pending);
        }
        assert_eq!(pending.len(), 1, "one file changed, not three");
    }

    #[test]
    fn watching_a_real_directory_reports_a_write() {
        let dir = std::env::temp_dir().join(format!(
            "lllearning-watch-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let (watcher, receiver) = FileWatcher::start(dir.clone()).expect("watch the directory");
        std::fs::write(dir.join("new.txt"), b"x").unwrap();

        let batch = receiver
            .recv_blocking()
            .expect("the write is reported within the debounce window");
        assert!(
            batch.iter().any(|path| path.ends_with("new.txt")),
            "the created file is in the batch: {batch:?}"
        );

        drop(watcher);
        std::fs::remove_dir_all(&dir).ok();
    }
}
