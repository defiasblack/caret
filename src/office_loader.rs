//! Supervise one disposable parser process. Neither slow I/O, allocation
//! failures, nor parser panics are allowed to block or terminate the editor.
use crate::office_viewer::OfficeViewer;
use std::{
    io::{self, Read, Write},
    path::Path,
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};

pub const WORKER_ARGUMENT: &str = "--office-worker";
pub const MEMORY_BYTES: usize = 512 * 1024 * 1024;
pub const RESPONSE_BYTES: usize = 64 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(5);

pub fn worker(path: &Path) -> i32 {
    crate::ALLOCATOR.set_limit(MEMORY_BYTES);
    let response: Result<OfficeViewer, String> =
        OfficeViewer::open(path).map_err(|error| error.to_string());
    let mut output = LimitedWriter {
        inner: io::stdout().lock(),
        remaining: RESPONSE_BYTES,
    };
    match serde_json::to_writer(&mut output, &response)
        .and_then(|()| output.flush().map_err(serde_json::Error::io))
    {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

struct LimitedWriter<W> {
    inner: W,
    remaining: usize,
}
impl<W: Write> Write for LimitedWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.remaining {
            return Err(io::Error::other(
                "Office response exceeds the 64 MiB budget",
            ));
        }
        let count = self.inner.write(bytes)?;
        self.remaining -= count;
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

struct Job {
    child: Child,
    result: Receiver<io::Result<OfficeViewer>>,
    started: Instant,
    generation: u64,
}
impl Drop for Job {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[derive(Default)]
pub struct OfficeLoader {
    generation: u64,
    job: Option<Job>,
}
impl OfficeLoader {
    pub fn start(&mut self, path: &Path) -> io::Result<()> {
        self.start_with(&std::env::current_exe()?, path)
    }

    fn start_with(&mut self, executable: &Path, path: &Path) -> io::Result<()> {
        self.cancel();
        // Input validation belongs to the helper too: metadata on a slow
        // filesystem must be covered by the deadline without blocking the UI.
        let started = Instant::now();
        let mut command = Command::new(executable);
        command.arg(WORKER_ARGUMENT).arg(path);
        self.launch(&mut command, started)
    }

    fn launch(&mut self, command: &mut Command, started: Instant) -> io::Result<()> {
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let pipe = child.stdout.take().expect("piped stdout");
        let (sender, result) = mpsc::sync_channel(1);
        thread::spawn(move || {
            let result = read_response(pipe);
            let _ = sender.send(result);
        });
        self.job = Some(Job {
            child,
            result,
            started,
            generation: self.generation,
        });
        Ok(())
    }

    pub fn cancel(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.job = None;
    }

    pub fn poll(&mut self) -> Option<io::Result<OfficeViewer>> {
        let job = self.job.as_mut()?;
        if job.generation != self.generation {
            self.job = None;
            return None;
        }
        if job.started.elapsed() >= TIMEOUT {
            self.cancel();
            return Some(Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Office loading exceeded the five-second budget",
            )));
        }
        if let Ok(result) = job.result.try_recv() {
            self.job = None;
            return Some(result);
        }
        if let Err(error) = job.child.try_wait() {
            self.cancel();
            return Some(Err(error));
        }
        None
    }
}

fn read_response(pipe: impl Read) -> io::Result<OfficeViewer> {
    let mut bytes = Vec::new();
    pipe.take(RESPONSE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > RESPONSE_BYTES {
        return Err(io::Error::other(
            "Office response exceeds the 64 MiB budget",
        ));
    }
    let response: Result<OfficeViewer, String> = serde_json::from_slice(&bytes).map_err(|_| {
        io::Error::other("Office parser failed; memory or response budget may have been exceeded")
    })?;
    response.map_err(io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_malformed_and_oversized_responses() {
        assert!(read_response(&b"not json"[..]).is_err());
        assert!(read_response(io::repeat(b'x').take(RESPONSE_BYTES as u64 + 1)).is_err());
        let error = serde_json::to_vec(&Result::<OfficeViewer, _>::Err("bad workbook")).unwrap();
        assert_eq!(
            read_response(&error[..]).unwrap_err().to_string(),
            "bad workbook"
        );
    }
    #[test]
    fn response_writer_enforces_budget_before_writing() {
        let mut output = LimitedWriter {
            inner: Vec::new(),
            remaining: 3,
        };
        assert!(output.write_all(b"four").is_err());
        assert!(output.inner.is_empty());
    }
    fn sleeping_command() -> Command {
        #[cfg(windows)]
        {
            let mut command = Command::new("powershell.exe");
            command.args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Start-Sleep -Seconds 30",
            ]);
            command
        }
        #[cfg(not(windows))]
        {
            let mut command = Command::new("/bin/sh");
            command.args(["-c", "exec sleep 30"]);
            command
        }
    }
    #[test]
    fn cancellation_supersession_and_deadline_reap_jobs() {
        let mut loader = OfficeLoader::default();
        loader
            .launch(&mut sleeping_command(), Instant::now())
            .unwrap();
        let first_generation = loader.generation;
        loader.cancel();
        assert!(loader.job.is_none());
        assert!(loader.generation != first_generation);
        loader
            .launch(&mut sleeping_command(), Instant::now())
            .unwrap();
        assert!(loader
            .start_with(
                Path::new("missing-caret-test-worker"),
                Path::new("superseded.xlsx")
            )
            .is_err());
        assert!(
            loader.job.is_none(),
            "failed supersession must still cancel the old helper"
        );
        loader
            .launch(&mut sleeping_command(), Instant::now())
            .unwrap();
        loader.generation += 1;
        assert!(loader.poll().is_none());
        assert!(loader.job.is_none());
        loader
            .launch(&mut sleeping_command(), Instant::now() - TIMEOUT)
            .unwrap();
        assert_eq!(
            loader.poll().unwrap().unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
        assert!(loader.job.is_none());
    }
    #[test]
    fn oversized_input_is_rejected_before_parsing() {
        let path =
            std::env::temp_dir().join(format!("caret-office-size-{}.xlsx", std::process::id()));
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(crate::office_viewer::MAX_INPUT_BYTES + 1)
            .unwrap();
        drop(file);
        assert!(OfficeViewer::open(&path)
            .unwrap_err()
            .to_string()
            .contains("32 MiB"));
        std::fs::remove_file(path).unwrap();
    }
}
