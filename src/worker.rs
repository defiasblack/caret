//! One running job and one replaceable pending job; obsolete results are dropped.
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Condvar, Mutex,
};

struct State<Q> {
    pending: Option<(u64, Q, Arc<AtomicBool>)>,
    active: Option<Arc<AtomicBool>>,
    stopped: bool,
}

pub struct LatestWorker<Q, R> {
    state: Arc<(Mutex<State<Q>>, Condvar)>,
    result: Arc<Mutex<Option<(u64, R)>>>,
    generation: u64,
}

impl<Q: Send + 'static, R: Send + 'static> LatestWorker<Q, R> {
    pub fn new(name: &str, work: impl Fn(Q, &AtomicBool) -> R + Send + 'static) -> Self {
        let state = Arc::new((
            Mutex::new(State {
                pending: None::<(u64, Q, Arc<AtomicBool>)>,
                active: None,
                stopped: false,
            }),
            Condvar::new(),
        ));
        let result = Arc::new(Mutex::new(None));
        let shared = Arc::clone(&state);
        let output = Arc::clone(&result);
        std::thread::Builder::new()
            .name(name.to_owned())
            .spawn(move || loop {
                let (generation, request, cancelled) = {
                    let (lock, wake) = &*shared;
                    let mut state = lock.lock().unwrap();
                    while state.pending.is_none() && !state.stopped {
                        state = wake.wait(state).unwrap();
                    }
                    if state.stopped {
                        break;
                    }
                    let job = state.pending.take().unwrap();
                    state.active = Some(Arc::clone(&job.2));
                    job
                };
                let value = work(request, &cancelled);
                if !cancelled.load(Ordering::Acquire) {
                    *output.lock().unwrap() = Some((generation, value));
                }
                shared.0.lock().unwrap().active = None;
                shared.1.notify_all();
            })
            .expect("start background worker");
        Self {
            state,
            result,
            generation: 0,
        }
    }

    #[cfg(test)]
    pub fn wait_idle(&self) {
        let mut state = self.state.0.lock().unwrap();
        while state.pending.is_some() || state.active.is_some() {
            let (next, timeout) = self
                .state
                .1
                .wait_timeout(state, std::time::Duration::from_secs(15))
                .unwrap();
            assert!(!timeout.timed_out(), "background job did not complete");
            state = next;
        }
    }

    pub fn cancel(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        let mut state = self.state.0.lock().unwrap();
        if let Some(cancelled) = &state.active {
            cancelled.store(true, Ordering::Release);
        }
        state.pending = None;
        *self.result.lock().unwrap() = None;
    }

    pub fn cancel_and_wait(&mut self) {
        self.cancel();
        let mut state = self.state.0.lock().unwrap();
        while state.active.is_some() {
            state = self.state.1.wait(state).unwrap();
        }
    }

    pub fn submit(&mut self, request: Q) {
        self.cancel();
        self.state.0.lock().unwrap().pending =
            Some((self.generation, request, Arc::new(AtomicBool::new(false))));
        self.state.1.notify_one();
    }

    pub fn poll(&mut self) -> Option<R> {
        self.result
            .lock()
            .unwrap()
            .take()
            .and_then(|(generation, result)| (generation == self.generation).then_some(result))
    }
}

impl<Q, R> Drop for LatestWorker<Q, R> {
    fn drop(&mut self) {
        let mut state = self.state.0.lock().unwrap();
        state.stopped = true;
        state.pending = None;
        if let Some(cancelled) = &state.active {
            cancelled.store(true, Ordering::Release);
        }
        self.state.1.notify_one();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    #[test]
    fn replaces_pending_jobs_and_discards_cancelled_results() {
        let (started, start) = mpsc::channel();
        let (release, gate) = mpsc::channel();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&calls);
        let mut worker = LatestWorker::new("test-coalescing", move |value, cancelled| {
            recorded.lock().unwrap().push(value);
            if value == 1 {
                started.send(()).unwrap();
                gate.recv().unwrap();
                assert!(cancelled.load(Ordering::Acquire));
            }
            value
        });
        worker.submit(1);
        start
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        worker.submit(2);
        worker.submit(3);
        release.send(()).unwrap();
        worker.wait_idle();
        assert_eq!(worker.poll(), Some(3));
        assert_eq!(*calls.lock().unwrap(), [1, 3]);
    }
    #[test]
    fn cancellation_waits_for_running_job_cleanup() {
        let (started, start) = mpsc::channel();
        let cleaned = Arc::new(AtomicBool::new(false));
        let done = Arc::clone(&cleaned);
        let mut worker = LatestWorker::new("test-cancellation", move |(), cancelled| {
            started.send(()).unwrap();
            while !cancelled.load(Ordering::Acquire) {
                std::thread::yield_now();
            }
            done.store(true, Ordering::Release);
        });
        worker.submit(());
        start
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        worker.cancel_and_wait();
        assert!(cleaned.load(Ordering::Acquire));
        assert!(worker.poll().is_none());
    }
}
