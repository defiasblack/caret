use portable_pty::{native_pty_system, Child, CommandBuilder, ExitStatus, MasterPty, PtySize};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

struct PtyProcess {
    _master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    input: Box<dyn Write + Send>,
    output: Receiver<Vec<u8>>,
    captured: Vec<u8>,
    parser: vt100::Parser,
}

impl PtyProcess {
    fn start(file: &Path, data_dir: &Path, config_dir: &Path) -> Self {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 30,
                cols: 100,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("open editor PTY");
        let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_caret"));
        command.arg(file);
        command.cwd(file.parent().expect("smoke file parent"));
        command.env("CARET_DATA_DIR", data_dir);
        command.env("CARET_CONFIG_DIR", config_dir);
        command.env("TERM", "xterm-256color");

        let child = pair
            .slave
            .spawn_command(command)
            .expect("start Caret in PTY");
        let mut reader = pair.master.try_clone_reader().expect("clone PTY reader");
        let input = pair.master.take_writer().expect("take PTY writer");
        drop(pair.slave);

        let (sender, output) = mpsc::channel();
        thread::spawn(move || {
            let mut buffer = [0; 8192];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(count) if sender.send(buffer[..count].to_vec()).is_err() => break,
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
        });

        Self {
            _master: pair.master,
            child,
            input,
            output,
            captured: Vec::new(),
            parser: vt100::Parser::new(30, 100, 0),
        }
    }

    fn send(&mut self, bytes: &[u8]) {
        self.input.write_all(bytes).expect("write editor input");
        self.input.flush().expect("flush editor input");
    }

    fn wait_for_output(&mut self, needle: &[u8], timeout: Duration) {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            while let Ok(bytes) = self.output.try_recv() {
                if bytes.windows(4).any(|window| window == b"\x1b[6n") {
                    // ConPTY asks the terminal for its cursor position before
                    // delivering application input.
                    self.send(b"\x1b[1;1R");
                }
                self.parser.process(&bytes);
                self.captured.extend_from_slice(&bytes);
                let matched_output = self
                    .captured
                    .windows(needle.len())
                    .any(|window| window == needle);
                let matched_screen = self
                    .parser
                    .screen()
                    .contents()
                    .as_bytes()
                    .windows(needle.len())
                    .any(|window| window == needle);
                if matched_output || matched_screen {
                    return;
                }
            }
            if self.child.try_wait().expect("poll Caret").is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        panic!(
            "Caret did not render expected text {:?}; output was {:?}",
            String::from_utf8_lossy(needle),
            String::from_utf8_lossy(&self.captured)
        );
    }

    fn wait_for_exit(&mut self, timeout: Duration) -> ExitStatus {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if let Some(status) = self.child.try_wait().expect("poll Caret") {
                return status;
            }
            thread::sleep(Duration::from_millis(20));
        }
        panic!("Caret did not exit after a clean save and Ctrl-Q");
    }

    fn terminate(&mut self) -> ExitStatus {
        self.child.kill().expect("terminate Caret");
        self.wait_for_exit(Duration::from_secs(10))
    }
}

impl Drop for PtyProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn temp_root() -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "caret-pty-editor-smoke-{}-{unique}",
        std::process::id()
    ))
}

#[test]
fn edits_saves_and_exits_cleanly_in_a_real_pty() {
    let root = temp_root();
    let file = root.join("important.txt");
    let data_dir = root.join("data");
    let config_dir = root.join("config");
    fs::create_dir_all(&root).expect("create smoke directory");
    fs::write(&file, "original").expect("create smoke file");

    let mut process = PtyProcess::start(&file, &data_dir, &config_dir);
    process.wait_for_output(b"-- INSERT --", Duration::from_secs(10));
    process.send(b"safe ");
    process.send(&[0x13]); // Ctrl-S

    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline && fs::read_to_string(&file).unwrap() != "safe original" {
        thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(fs::read_to_string(&file).unwrap(), "safe original");

    process.send(&[0x11]); // Ctrl-Q
    let status = process.wait_for_exit(Duration::from_secs(10));
    assert!(
        status.success(),
        "Caret exited unsuccessfully: code={} signal={:?}",
        status.exit_code(),
        status.signal()
    );

    let recovery_dir = data_dir.join("recovery");
    let recovery_journals = fs::read_dir(&recovery_dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|entry| entry.file_name().to_string_lossy().starts_with("journal-"))
                .count()
        })
        .unwrap_or(0);
    assert_eq!(
        recovery_journals, 0,
        "clean exit left a crash-recovery journal"
    );
    let session: serde_json::Value = serde_json::from_slice(
        &fs::read(data_dir.join("session.json")).expect("clean exit did not save session state"),
    )
    .expect("saved session is invalid");
    assert_eq!(session["tabs"][0]["path"], file.to_string_lossy().as_ref());
    assert_eq!(session["tabs"][0]["cursor"]["column"], 5);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn forced_termination_is_reported_by_the_next_real_pty_session() {
    let root = temp_root();
    let file = root.join("recovery.txt");
    let unrelated = root.join("unrelated.txt");
    let data_dir = root.join("data");
    let config_dir = root.join("config");
    fs::create_dir_all(&root).expect("create smoke directory");
    fs::write(&file, "original").expect("create smoke file");
    fs::write(&unrelated, "must remain unchanged").expect("create unrelated file");

    let mut first = PtyProcess::start(&file, &data_dir, &config_dir);
    first.wait_for_output(b"-- INSERT --", Duration::from_secs(10));
    first.send(b"unsaved ");

    let recovery_dir = data_dir.join("recovery");
    let deadline = Instant::now() + Duration::from_secs(10);
    let journal = loop {
        let found = fs::read_dir(&recovery_dir).ok().and_then(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .find(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.starts_with("journal-") && name.ends_with(".json"))
                })
        });
        if found.is_some() || Instant::now() >= deadline {
            break found;
        }
        thread::sleep(Duration::from_millis(50));
    }
    .expect("Caret did not checkpoint unsaved PTY work");
    let journal_text = fs::read_to_string(&journal).expect("read recovery journal");
    assert!(
        journal_text.contains("unsaved original"),
        "journal did not contain the unsaved buffer"
    );
    let _ = first.terminate();
    drop(first);

    let config_path = config_dir.join("config.toml");
    let config = fs::read_to_string(&config_path).expect("read generated config");
    let vim_config = config.replace("keymap = \"caret\"", "keymap = \"vim\"");
    assert_ne!(
        vim_config, config,
        "generated config did not name the keymap"
    );
    fs::write(&config_path, vim_config).expect("select Vim profile for command input");

    // Start on a different file: recovery must honor the path recorded in the
    // snapshot instead of replacing whichever tab happens to be active.
    let mut second = PtyProcess::start(&unrelated, &data_dir, &config_dir);
    second.wait_for_output(b"Recovery:", Duration::from_secs(10));
    second.send(b":recover 1\r");
    second.wait_for_output(b"Recovered", Duration::from_secs(10));
    second.send(&[0x13]); // Ctrl-S
    let save_deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < save_deadline && fs::read_to_string(&file).unwrap() != "unsaved original"
    {
        thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(fs::read_to_string(&file).unwrap(), "unsaved original");
    assert_eq!(
        fs::read_to_string(&unrelated).unwrap(),
        "must remain unchanged"
    );
    second.send(&[0x11]); // Ctrl-Q
    assert!(second.wait_for_exit(Duration::from_secs(10)).success());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn external_changes_require_confirmation_in_a_real_pty() {
    let root = temp_root();
    let file = root.join("conflict.txt");
    let data_dir = root.join("data");
    let config_dir = root.join("config");
    fs::create_dir_all(&root).expect("create smoke directory");
    fs::write(&file, "original").expect("create smoke file");

    let mut process = PtyProcess::start(&file, &data_dir, &config_dir);
    process.wait_for_output(b"-- INSERT --", Duration::from_secs(10));
    process.send(b"buffer ");
    process.wait_for_output(b"buffer original", Duration::from_secs(10));

    fs::write(&file, "external").expect("replace file outside Caret");
    process.wait_for_output(b"File changed on disk", Duration::from_secs(10));
    process.send(&[0x13]); // Ctrl-S is not confirmation.
    thread::sleep(Duration::from_millis(200));
    assert_eq!(fs::read_to_string(&file).unwrap(), "external");

    process.send(b"k");
    process.wait_for_output(b"Kept current buffer", Duration::from_secs(10));
    process.captured.clear();
    process.send(&[0x13]); // Ask to save again.
    process.wait_for_output(b"Disk changes were kept", Duration::from_secs(10));
    process.send(b"k"); // Explicitly confirm the pending save.

    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline && fs::read_to_string(&file).unwrap() != "buffer original" {
        thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(fs::read_to_string(&file).unwrap(), "buffer original");

    process.send(&[0x11]); // Ctrl-Q
    assert!(process.wait_for_exit(Duration::from_secs(10)).success());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn repeated_edits_and_atomic_saves_remain_stable_in_a_real_pty() {
    let root = temp_root();
    let file = root.join("soak.txt");
    let data_dir = root.join("data");
    let config_dir = root.join("config");
    fs::create_dir_all(&root).expect("create smoke directory");
    fs::write(&file, "base").expect("create smoke file");

    let mut process = PtyProcess::start(&file, &data_dir, &config_dir);
    process.wait_for_output(b"-- INSERT --", Duration::from_secs(10));

    for count in 1..=25 {
        process.send(b"x");
        process.send(&[0x13]); // Ctrl-S
        let expected = format!("{}base", "x".repeat(count));
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline && fs::read_to_string(&file).unwrap() != expected {
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            fs::read_to_string(&file).unwrap(),
            expected,
            "save cycle {count} did not produce the complete document"
        );
    }

    process.send(&[0x11]); // Ctrl-Q
    assert!(process.wait_for_exit(Duration::from_secs(10)).success());
    let _ = fs::remove_dir_all(root);
}
