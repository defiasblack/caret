use std::{
    io::{self, BufRead, BufReader, Read, Write},
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{mpsc::{self, Receiver}, Mutex},
    thread,
};

use serde_json::{json, Value};

pub struct LspClient {
    child: Child,
    input: Mutex<ChildStdin>,
    messages: Receiver<Value>,
    next_id: u64,
}

impl LspClient {
    pub fn start(command: &str, root: &Path) -> io::Result<Self> {
        let mut process = Command::new(command);
        process.current_dir(root);
        if command == "csharp-ls" {
            if let Some(solution) = find_solution(root) {
                process.arg("--solution").arg(solution);
            }
        }
        let mut child = process
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let input = child.stdin.take().expect("piped stdin");
        let output = child.stdout.take().expect("piped stdout");
        let (sender, messages) = mpsc::channel();
        thread::spawn(move || read_messages(output, sender));

        let mut client = Self {
            child,
            input: Mutex::new(input),
            messages,
            next_id: 1,
        };
        let root_uri = file_uri(root);
        client.request("initialize", json!({
            "processId": std::process::id(),
            "rootUri": root_uri,
            "workspaceFolders": [{ "uri": root_uri, "name": root.file_name().and_then(|name| name.to_str()).unwrap_or("workspace") }],
            "capabilities": {
                "workspace": { "configuration": true, "workspaceFolders": true },
                "window": { "workDoneProgress": true },
                "textDocument": {
                    "hover": { "contentFormat": ["plaintext", "markdown"] },
                    "publishDiagnostics": { "relatedInformation": true }
                }
            }
        }))?;
        Ok(client)
    }

    pub fn request(&mut self, method: &str, params: Value) -> io::Result<u64> {
        let id = self.next_id;
        self.next_id += 1;
        self.write(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))?;
        Ok(id)
    }

    pub fn notify(&self, method: &str, params: Value) -> io::Result<()> {
        self.write(json!({"jsonrpc": "2.0", "method": method, "params": params}))
    }

    pub fn respond(&self, id: &Value, result: Value) -> io::Result<()> {
        self.write(json!({"jsonrpc": "2.0", "id": id, "result": result}))
    }

    pub fn try_recv(&self) -> Option<Value> {
        self.messages.try_recv().ok()
    }

    fn write(&self, message: Value) -> io::Result<()> {
        let body = serde_json::to_vec(&message)
            .map_err(|error| io::Error::other(format!("LSP serialization failed: {error}")))?;
        let mut input = self.input.lock().expect("LSP input lock");
        write!(input, "Content-Length: {}\r\n\r\n", body.len())?;
        input.write_all(&body)?;
        input.flush()
    }
}

fn find_solution(root: &Path) -> Option<std::path::PathBuf> {
    let mut solutions = std::fs::read_dir(root)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("sln"))
        })
        .collect::<Vec<_>>();
    solutions.sort();
    solutions.into_iter().next()
}

impl Drop for LspClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

pub fn server_for_extension(path: &Path) -> Option<&'static str> {
    match path.extension().and_then(|extension| extension.to_str())?.to_ascii_lowercase().as_str() {
        "cs" => Some("csharp-ls"),
        "rs" => Some("rust-analyzer"),
        _ => None,
    }
}

pub fn file_uri(path: &Path) -> String {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    format!("file:///{}", path.to_string_lossy().replace('\\', "/").replace(' ', "%20"))
}

fn read_messages(output: impl io::Read, sender: mpsc::Sender<Value>) {
    let mut reader = BufReader::new(output);
    loop {
        let mut length = None;
        let mut line = String::new();
        loop {
            line.clear();
            if reader.read_line(&mut line).ok().filter(|count| *count > 0).is_none() {
                return;
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                break;
            }
            if let Some(value) = trimmed.strip_prefix("Content-Length:") {
                length = value.trim().parse::<usize>().ok();
            }
        }
        let Some(length) = length else { continue };
        let mut body = vec![0; length];
        if reader.read_exact(&mut body).is_err() {
            return;
        }
        if let Ok(message) = serde_json::from_slice(&body) {
            let _ = sender.send(message);
        }
    }
}
