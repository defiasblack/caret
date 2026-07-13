use std::{
    io::{self, BufRead, BufReader, Read, Write},
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        mpsc::{self, Receiver},
        Mutex,
    },
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
                "workspace": { "configuration": true, "workspaceFolders": true, "workspaceEdit": { "documentChanges": true } },
                "window": { "workDoneProgress": true },
                "textDocument": {
                    "definition": { "linkSupport": true },
                    "hover": { "contentFormat": ["plaintext", "markdown"] },
                    "completion": { "completionItem": { "snippetSupport": true, "documentationFormat": ["plaintext", "markdown"] } },
                    "references": {},
                    "rename": { "prepareSupport": false },
                    "codeAction": { "codeActionLiteralSupport": { "codeActionKind": { "valueSet": ["", "quickfix", "refactor", "source"] } } },
                    "formatting": {},
                    "publishDiagnostics": { "relatedInformation": true }
                }
            }
        }))?;
        client.next_id = 1_000;
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
    match path
        .extension()
        .and_then(|extension| extension.to_str())?
        .to_ascii_lowercase()
        .as_str()
    {
        "cs" => Some("csharp-ls"),
        "rs" => Some("rust-analyzer"),
        _ => None,
    }
}

pub fn file_uri(path: &Path) -> String {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    format!(
        "file:///{}",
        path.to_string_lossy()
            .replace('\\', "/")
            .replace(' ', "%20")
    )
}

pub fn path_from_uri(uri: &str) -> Option<std::path::PathBuf> {
    let encoded = uri.strip_prefix("file://")?;
    let decoded = percent_decode(encoded)?;
    #[cfg(windows)]
    let decoded = decoded
        .strip_prefix('/')
        .unwrap_or(&decoded)
        .replace('/', "\\");
    Some(std::path::PathBuf::from(decoded))
}

fn percent_decode(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return None;
            }
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).ok()?;
            decoded.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

fn read_messages(output: impl io::Read, sender: mpsc::Sender<Value>) {
    let mut reader = BufReader::new(output);
    loop {
        let mut length = None;
        let mut line = String::new();
        loop {
            line.clear();
            if reader
                .read_line(&mut line)
                .ok()
                .filter(|count| *count > 0)
                .is_none()
            {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn file_uris_round_trip_spaces_and_unicode() {
        let uri = "file:///C:/code/hello%20world/%E2%9C%93.cs";
        let path = path_from_uri(uri).expect("decode URI");
        assert!(path.to_string_lossy().contains("hello world"));
        assert!(path.to_string_lossy().contains('✓'));
    }

    #[test]
    #[ignore = "requires rust-analyzer on PATH"]
    fn rust_analyzer_round_trip() {
        if !Command::new("rust-analyzer")
            .arg("--version")
            .status()
            .is_ok_and(|status| status.success())
        {
            return;
        }
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let file = root.join("src/main.rs");
        let mut client = LspClient::start("rust-analyzer", root).expect("start rust-analyzer");
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut initialized = false;
        while Instant::now() < deadline && !initialized {
            if let Some(message) = client.try_recv() {
                if message["id"].as_u64() == Some(1) {
                    assert!(message.get("error").is_none(), "{message}");
                    client
                        .notify("initialized", json!({}))
                        .expect("initialized notification");
                    client.notify("textDocument/didOpen", json!({ "textDocument": { "uri": file_uri(&file), "languageId": "rust", "version": 1, "text": std::fs::read_to_string(&file).expect("read source") } })).expect("didOpen");
                    initialized = true;
                } else if message.get("method").is_some() && message.get("id").is_some() {
                    let id = message["id"].clone();
                    let count = message["params"]["items"].as_array().map_or(0, Vec::len);
                    let result = if message["method"] == "workspace/configuration" {
                        json!(vec![json!({}); count])
                    } else {
                        Value::Null
                    };
                    client.respond(&id, result).expect("respond to server");
                }
            } else {
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        assert!(initialized, "rust-analyzer did not initialize in time");

        let request = client.request("textDocument/hover", json!({ "textDocument": { "uri": file_uri(&file) }, "position": { "line": 0, "character": 4 } })).expect("hover request");
        while Instant::now() < deadline + Duration::from_secs(10) {
            if let Some(message) = client.try_recv() {
                if message["id"].as_u64() == Some(request) {
                    assert!(message.get("error").is_none(), "{message}");
                    assert!(!message["result"].is_null(), "hover returned null");
                    return;
                }
                if message.get("method").is_some() && message.get("id").is_some() {
                    let _ = client.respond(&message["id"], Value::Null);
                }
            } else {
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        panic!("rust-analyzer hover timed out");
    }
}
