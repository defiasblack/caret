//! Ordered, coalesced persistence. Shutdown is a barrier followed by journal removal.
use crate::{recovery, session};
use std::sync::{Arc, Condvar, Mutex};

#[derive(Default)]
struct Pending {
    recovery: Option<Vec<recovery::Snapshot>>,
    session: Option<session::SessionState>,
}
#[derive(Default)]
struct State {
    pending: Pending,
    submitted: u64,
    completed: u64,
    stopped: bool,
    error: Option<String>,
}

pub struct PersistenceWorker {
    shared: Arc<(Mutex<State>, Condvar)>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl PersistenceWorker {
    pub fn new() -> Self {
        let enabled = !cfg!(test) || std::env::var_os("CARET_DATA_DIR").is_some();
        Self::spawn(move |job| {
            let mut errors = Vec::new();
            if let Some(entries) = job.recovery.filter(|_| enabled) {
                let result = if entries.is_empty() {
                    recovery::discard_current()
                } else {
                    recovery::save_snapshots(entries)
                };
                if let Err(error) = result {
                    errors.push(format!("Recovery checkpoint failed: {error}"));
                }
            }
            if let Some(session) = job.session.filter(|_| enabled) {
                if let Err(error) = session::save(&session) {
                    errors.push(format!("Session checkpoint failed: {error}"));
                }
            }
            if errors.is_empty() {
                Ok(())
            } else {
                Err(std::io::Error::other(errors.join("; ")))
            }
        })
    }

    fn spawn(write: impl Fn(Pending) -> std::io::Result<()> + Send + 'static) -> Self {
        let shared = Arc::new((Mutex::new(State::default()), Condvar::new()));
        let worker = Arc::clone(&shared);
        let thread = std::thread::Builder::new()
            .name("caret-persistence".to_string())
            .spawn(move || loop {
                let (job, sequence) = {
                    let (lock, wake) = &*worker;
                    let mut state = lock.lock().unwrap();
                    while state.completed == state.submitted && !state.stopped {
                        state = wake.wait(state).unwrap();
                    }
                    if state.stopped {
                        break;
                    }
                    (std::mem::take(&mut state.pending), state.submitted)
                };
                let result = write(job);
                let mut state = worker.0.lock().unwrap();
                if let Err(error) = result {
                    state.error = Some(error.to_string());
                }
                state.completed = sequence;
                worker.1.notify_all();
            })
            .expect("start persistence worker");
        Self {
            shared,
            thread: Some(thread),
        }
    }

    pub fn recovery(&self, entries: Vec<recovery::Snapshot>) {
        let mut state = self.shared.0.lock().unwrap();
        if state.stopped {
            return;
        }
        state.pending.recovery = Some(entries);
        state.submitted += 1;
        self.shared.1.notify_all();
    }
    pub fn session(&self, session: session::SessionState) {
        let mut state = self.shared.0.lock().unwrap();
        if state.stopped {
            return;
        }
        state.pending.session = Some(session);
        state.submitted += 1;
        self.shared.1.notify_all();
    }
    pub fn poll_error(&self) -> Option<String> {
        self.shared.0.lock().unwrap().error.take()
    }
    pub fn flush(&self) -> std::io::Result<()> {
        let mut state = self.shared.0.lock().unwrap();
        let sequence = state.submitted;
        while state.completed < sequence {
            state = self.shared.1.wait(state).unwrap();
        }
        state
            .error
            .take()
            .map_or(Ok(()), |error| Err(std::io::Error::other(error)))
    }
    pub fn finish(&mut self, session: Option<session::SessionState>) -> std::io::Result<()> {
        if self.thread.is_none() {
            return Ok(());
        }
        if let Some(session) = session {
            self.session(session);
        }
        self.recovery(Vec::new());
        let result = self.flush();
        self.stop();
        result
    }
    fn stop(&mut self) {
        self.shared.0.lock().unwrap().stopped = true;
        self.shared.1.notify_all();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
impl Drop for PersistenceWorker {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    fn snapshot(text: &str) -> Vec<recovery::Snapshot> {
        vec![recovery::Snapshot {
            path: None,
            text: ropey::Rope::from_str(text),
            cursor: Default::default(),
        }]
    }
    #[test]
    fn shutdown_discard_follows_running_write_and_coalesces_pending_recovery() {
        let (started, start) = mpsc::channel();
        let (release, gate) = mpsc::channel();
        let writes = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&writes);
        let mut worker = PersistenceWorker::spawn(move |job| {
            if let Some(entries) = job.recovery {
                let text = entries
                    .first()
                    .map(|entry| entry.text.to_string())
                    .unwrap_or_else(|| "discard".to_string());
                if text == "first" {
                    started.send(()).unwrap();
                    gate.recv().unwrap();
                }
                recorded.lock().unwrap().push(text);
            }
            Ok(())
        });
        worker.recovery(snapshot("first"));
        start
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        worker.recovery(snapshot("obsolete"));
        worker.recovery(snapshot("latest"));
        let state = Arc::clone(&worker.shared);
        let finishing = std::thread::spawn(move || {
            worker.finish(None).unwrap();
            worker.recovery(snapshot("must not recreate journal"));
        });
        {
            let mut shared = state.0.lock().unwrap();
            while shared
                .pending
                .recovery
                .as_ref()
                .is_none_or(|entries| !entries.is_empty())
            {
                let (next, timeout) = state
                    .1
                    .wait_timeout(shared, std::time::Duration::from_secs(10))
                    .unwrap();
                if timeout.timed_out() {
                    release.send(()).unwrap();
                    panic!("shutdown did not enqueue discard");
                }
                shared = next;
            }
        }
        release.send(()).unwrap();
        finishing.join().unwrap();
        assert_eq!(*writes.lock().unwrap(), ["first", "discard"]);
        assert!(state.0.lock().unwrap().stopped);
    }
    #[test]
    fn reports_worker_io_errors_and_can_flush_later_work() {
        let worker =
            PersistenceWorker::spawn(|_| Err(std::io::Error::other("simulated full disk")));
        worker.recovery(snapshot("work"));
        assert!(worker
            .flush()
            .unwrap_err()
            .to_string()
            .contains("full disk"));
        assert!(worker.poll_error().is_none());
    }
}
