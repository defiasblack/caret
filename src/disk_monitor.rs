//! One bounded worker for disk observations; the UI only handles completed work.
use crate::{
    document::{self, DiskState},
    editor::Editor,
};
use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver, SyncSender},
    thread,
    time::{Duration, Instant},
};

const CHECK_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq, Eq)]
struct Request {
    path: PathBuf,
    epoch: u64,
    expected: DiskState,
}

pub struct DiskMonitor {
    requests: SyncSender<Request>,
    results: Receiver<(Request, DiskState)>,
    pending: bool,
    last: Option<(Request, Instant)>,
}

impl DiskMonitor {
    pub fn new() -> Self {
        let (requests, receiver) = mpsc::sync_channel::<Request>(1);
        let (sender, results) = mpsc::sync_channel(1);
        thread::spawn(move || {
            while let Ok(request) = receiver.recv() {
                let state = document::disk_state(&request.path);
                if sender.send((request, state)).is_err() {
                    break;
                }
            }
        });
        Self {
            requests,
            results,
            pending: false,
            last: None,
        }
    }

    pub fn poll(&mut self, editor: &Editor) -> Option<DiskState> {
        self.poll_at(editor, Instant::now())
    }

    fn poll_at(&mut self, editor: &Editor, now: Instant) -> Option<DiskState> {
        let current = editor.path.as_ref().map(|path| Request {
            path: path.clone(),
            epoch: editor.disk_epoch(),
            expected: editor.expected_disk_state(),
        });
        let mut changed = None;
        while let Ok((request, observed)) = self.results.try_recv() {
            self.pending = false;
            if current.as_ref() == Some(&request) && observed != request.expected {
                changed = Some(observed);
            }
        }
        if let Some(request) = current {
            let due = self.last.as_ref().is_none_or(|(last, at)| {
                *last != request || now.saturating_duration_since(*at) >= CHECK_INTERVAL
            });
            if !self.pending
                && due
                && changed.is_none()
                && self.requests.try_send(request.clone()).is_ok()
            {
                self.pending = true;
                self.last = Some((request, now));
            }
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn coalesces_requests_and_rejects_obsolete_paths_and_save_generations() {
        let path = std::env::temp_dir().join(format!("caret-monitor-{}", std::process::id()));
        std::fs::write(&path, "original").unwrap();
        let mut editor = Editor::from_file(&path).unwrap();
        let (requests, receiver) = mpsc::sync_channel(1);
        let (sender, results) = mpsc::sync_channel(1);
        let mut monitor = DiskMonitor {
            requests,
            results,
            pending: false,
            last: None,
        };
        let at = Instant::now();
        assert!(monitor.poll_at(&editor, at).is_none());
        let first = receiver.try_recv().unwrap();
        for _ in 0..100 {
            monitor.poll_at(&editor, at + Duration::from_secs(30));
        }
        assert!(
            receiver.try_recv().is_err(),
            "only one hash can be in flight"
        );
        sender.send((first.clone(), first.expected)).unwrap();
        monitor.poll_at(&editor, at + Duration::from_secs(1));
        assert!(receiver.try_recv().is_err(), "two-second minimum interval");
        monitor.poll_at(&editor, at + Duration::from_secs(2));
        let stale = receiver.try_recv().unwrap();
        editor.insert_char('!');
        editor.save().unwrap();
        sender
            .send((
                stale.clone(),
                DiskState {
                    fingerprint: None,
                    ..stale.expected
                },
            ))
            .unwrap();
        assert!(monitor
            .poll_at(&editor, at + Duration::from_secs(3))
            .is_none());
        let saved = receiver.try_recv().unwrap();
        editor.path = Some(path.with_extension("other"));
        sender
            .send((
                saved.clone(),
                DiskState {
                    fingerprint: None,
                    ..saved.expected
                },
            ))
            .unwrap();
        assert!(monitor
            .poll_at(&editor, at + Duration::from_secs(4))
            .is_none());
        let active = receiver.try_recv().unwrap();
        let changed = DiskState {
            fingerprint: None,
            ..active.expected
        };
        sender.send((active, changed)).unwrap();
        assert_eq!(
            monitor.poll_at(&editor, at + Duration::from_secs(5)),
            Some(changed)
        );
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    #[ignore = "manual release performance benchmark"]
    fn benchmark_background_disk_checks() {
        let path = std::env::temp_dir().join(format!("caret-disk-bench-{}", std::process::id()));
        std::fs::write(&path, vec![b'x'; 16 * 1024 * 1024]).unwrap();
        let editor = Editor::from_file(&path).unwrap();
        let mut monitor = DiskMonitor::new();
        let at = Instant::now();
        for _ in 0..10_000 {
            std::hint::black_box(monitor.poll_at(&editor, at));
        }
        eprintln!("10,000 coalesced foreground polls: {:?}", at.elapsed());
        let at = Instant::now();
        for _ in 0..20 {
            std::hint::black_box(document::disk_state(&path));
        }
        eprintln!(
            "20 streaming worker fingerprints of 16 MiB: {:?}",
            at.elapsed()
        );
        std::fs::remove_file(path).unwrap();
    }
}
