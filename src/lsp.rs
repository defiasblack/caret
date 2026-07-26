use std::{
    env,
    io::{self, BufRead, BufReader, Read, Write},
    path::Path,
    process::{Child, ChildStdin, Command, ExitStatus, Stdio},
    sync::{
        mpsc::{self, Receiver},
        Mutex,
    },
    thread,
};

use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LanguageServerSpec {
    pub id: &'static str,
    pub command: &'static str,
    pub arguments: &'static [&'static str],
    pub language_id: &'static str,
    pub install_guidance: &'static str,
}

const SERVERS: [(&[&str], LanguageServerSpec); 5] = [
    (
        &["rs"],
        LanguageServerSpec {
            id: "rust-analyzer",
            command: "rust-analyzer",
            arguments: &[],
            language_id: "rust",
            install_guidance: "Install with `rustup component add rust-analyzer`.",
        },
    ),
    (
        &["cs"],
        LanguageServerSpec {
            id: "csharp-ls",
            command: "csharp-ls",
            arguments: &[],
            language_id: "csharp",
            install_guidance: "Install with `dotnet tool install --global csharp-ls`.",
        },
    ),
    (
        &["py", "pyw"],
        LanguageServerSpec {
            id: "pyright",
            command: "pyright-langserver",
            arguments: &["--stdio"],
            language_id: "python",
            install_guidance: "Install with `npm install --global pyright`.",
        },
    ),
    (
        &["go"],
        LanguageServerSpec {
            id: "gopls",
            command: "gopls",
            arguments: &[],
            language_id: "go",
            install_guidance: "Install with `go install golang.org/x/tools/gopls@latest`.",
        },
    ),
    (
        &["js", "jsx", "mjs", "cjs", "ts", "tsx", "mts", "cts"],
        LanguageServerSpec {
            id: "typescript-language-server",
            command: "typescript-language-server",
            arguments: &["--stdio"],
            language_id: "typescript",
            install_guidance:
                "Install with `npm install --global typescript typescript-language-server`.",
        },
    ),
];

pub struct LspClient {
    child: Child,
    input: Mutex<ChildStdin>,
    messages: Receiver<Value>,
    next_id: u64,
}

impl LspClient {
    #[cfg(test)]
    pub fn start(command: &str, root: &Path) -> io::Result<Self> {
        Self::start_with(command, &[], root)
    }

    pub fn start_server(server: &LanguageServerSpec, root: &Path) -> io::Result<Self> {
        Self::start_with(server.command, server.arguments, root)
    }

    fn start_with(command: &str, arguments: &[&str], root: &Path) -> io::Result<Self> {
        let mut process = Command::new(command);
        process.current_dir(root);
        process.args(arguments);
        if command == "csharp-ls" {
            if let Some(solution) = find_solution(root) {
                process.arg("--solution").arg(solution);
            }
        }
        let mut child = process
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let input = child.stdin.take().expect("piped stdin");
        let output = child.stdout.take().expect("piped stdout");
        let errors = child.stderr.take().expect("piped stderr");
        let (sender, messages) = mpsc::channel();
        thread::spawn(move || read_messages(output, sender));
        thread::spawn(move || read_errors(errors));

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
                "general": { "positionEncodings": ["utf-16"] },
                "textDocument": {
                    "definition": { "linkSupport": true },
                    "hover": { "contentFormat": ["plaintext", "markdown"] },
                    "completion": { "completionItem": { "snippetSupport": true, "documentationFormat": ["plaintext", "markdown"] } },
                    "references": {},
                    "rename": { "prepareSupport": false },
                    "codeAction": { "codeActionLiteralSupport": { "codeActionKind": { "valueSet": ["", "quickfix", "refactor", "source"] } } },
                    "formatting": {},
                    "rangeFormatting": {},
                    "signatureHelp": { "signatureInformation": { "documentationFormat": ["plaintext", "markdown"], "parameterInformation": { "labelOffsetSupport": true } } },
                    "documentSymbol": { "hierarchicalDocumentSymbolSupport": true },
                    "publishDiagnostics": { "relatedInformation": true }
                }
            }
        }))?;
        client.next_id = 1_000;
        Ok(client)
    }

    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.child.try_wait()
    }

    pub fn stop_gracefully(&mut self) {
        let shutdown = self.request("shutdown", json!(null)).ok();
        for _ in 0..10 {
            if self
                .try_recv()
                .as_ref()
                .and_then(|message| message.get("id"))
                .and_then(Value::as_u64)
                == shutdown
            {
                break;
            }
            if self.child.try_wait().ok().flatten().is_some() {
                return;
            }
            thread::sleep(std::time::Duration::from_millis(20));
        }
        let _ = self.notify("exit", json!(null));
        for _ in 0..10 {
            if self.child.try_wait().ok().flatten().is_some() {
                return;
            }
            thread::sleep(std::time::Duration::from_millis(20));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
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
                .is_some_and(|extension| {
                    extension.eq_ignore_ascii_case("sln") || extension.eq_ignore_ascii_case("slnx")
                })
        })
        .collect::<Vec<_>>();
    solutions.sort();
    solutions
        .into_iter()
        .next()
        .map(|path| without_windows_verbatim_prefix(&path))
}

impl Drop for LspClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

pub fn server_for_extension(path: &Path) -> Option<&'static LanguageServerSpec> {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())?
        .to_ascii_lowercase();
    SERVERS
        .iter()
        .find(|(extensions, _)| extensions.contains(&extension.as_str()))
        .map(|(_, server)| server)
}

pub fn executable_available(command: &str) -> bool {
    let command = Path::new(command);
    if command.components().count() > 1 {
        return executable_candidate(command);
    }
    let Some(path) = env::var_os("PATH") else {
        return false;
    };
    env::split_paths(&path).any(|directory| {
        let candidate = directory.join(command);
        executable_candidate(&candidate)
            || executable_extensions()
                .iter()
                .any(|extension| executable_candidate(&candidate.with_extension(extension)))
    })
}

fn executable_candidate(path: &Path) -> bool {
    path.metadata().is_ok_and(|metadata| metadata.is_file())
}

fn executable_extensions() -> Vec<String> {
    #[cfg(windows)]
    {
        env::var_os("PATHEXT")
            .map(|value| {
                value
                    .to_string_lossy()
                    .split(';')
                    .filter_map(|extension| {
                        let extension = extension.trim().trim_start_matches('.');
                        (!extension.is_empty()).then(|| extension.to_string())
                    })
                    .collect()
            })
            .unwrap_or_else(|| vec!["EXE".to_string(), "CMD".to_string(), "BAT".to_string()])
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

pub fn file_uri(path: &Path) -> String {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let path = without_windows_verbatim_prefix(&path)
        .to_string_lossy()
        .into_owned();
    let path = percent_encode_path(&path.replace('\\', "/"));
    if path.starts_with("//") {
        format!("file:{path}")
    } else if path.starts_with('/') {
        format!("file://{path}")
    } else {
        format!("file:///{path}")
    }
}

fn without_windows_verbatim_prefix(path: &Path) -> std::path::PathBuf {
    #[cfg(windows)]
    {
        let path = path.to_string_lossy();
        if let Some(path) = path.strip_prefix(r"\\?\UNC\") {
            return std::path::PathBuf::from(format!(r"\\{path}"));
        }
        if let Some(path) = path.strip_prefix(r"\\?\") {
            return std::path::PathBuf::from(path);
        }
    }
    path.to_path_buf()
}

pub fn path_from_uri(uri: &str) -> Option<std::path::PathBuf> {
    let encoded = uri.strip_prefix("file://")?;
    let decoded = percent_decode(encoded)?;
    #[cfg(windows)]
    let decoded = if let Some(path) = decoded.strip_prefix('/') {
        path.replace('/', "\\")
    } else {
        format!(r"\\{}", decoded.replace('/', "\\"))
    };
    Some(std::path::PathBuf::from(decoded))
}

fn percent_encode_path(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b':' | b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            use std::fmt::Write;
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
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

fn read_errors(output: impl io::Read) {
    read_errors_with(output, |line| {
        let _ = crate::diagnostics::append("lsp", line);
    });
}

fn read_errors_with<F>(output: impl io::Read, mut record: F)
where
    F: FnMut(&str),
{
    for line in BufReader::new(output).lines() {
        match line {
            Ok(line) if !line.trim().is_empty() => {
                record(&line);
            }
            Ok(_) => {}
            Err(error) => {
                record(&format!("stderr read failed: {error}"));
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::time::{Duration, Instant};

    #[test]
    fn file_uris_round_trip_spaces_and_unicode() {
        let uri = "file:///C:/code/hello%20world/%E2%9C%93.cs";
        let path = path_from_uri(uri).expect("decode URI");
        assert!(path.to_string_lossy().contains("hello world"));
        assert!(path.to_string_lossy().contains('✓'));
    }

    #[test]
    fn stderr_lines_are_written_to_the_structured_log() {
        let mut lines = Vec::new();
        read_errors_with(Cursor::new(b"server failed\n\nsecond line\n"), |line| {
            lines.push(line.to_string())
        });
        assert_eq!(lines, ["server failed", "second line"]);
    }

    #[test]
    fn official_language_servers_are_data_driven_and_actionable() {
        for (name, id, command_fragment) in [
            ("main.rs", "rust-analyzer", "rustup"),
            ("Program.cs", "csharp-ls", "dotnet"),
            ("main.py", "pyright", "npm"),
            ("main.go", "gopls", "go install"),
            ("main.tsx", "typescript-language-server", "npm"),
        ] {
            let server = server_for_extension(Path::new(name)).unwrap();
            assert_eq!(server.id, id);
            assert!(server.install_guidance.contains(command_fragment));
            assert!(!server.language_id.is_empty());
        }
    }

    #[test]
    fn missing_executables_are_detected_without_spawning_them() {
        assert!(!executable_available(
            "caret-language-server-that-does-not-exist"
        ));
    }

    #[cfg(windows)]
    #[test]
    fn windows_drive_paths_round_trip_through_file_uris() {
        let root = std::env::temp_dir().join(format!("caret-lsp-uri-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("hello #%.cs");
        std::fs::write(&path, "").unwrap();
        let uri = file_uri(&path);
        assert!(uri.starts_with("file:///"), "{uri}");
        assert!(!uri.contains("/?/"), "{uri}");
        assert!(uri.ends_with("hello%20%23%25.cs"), "{uri}");
        let decoded = path_from_uri(&uri).unwrap();
        assert_eq!(
            decoded.canonicalize().unwrap(),
            path.canonicalize().unwrap()
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(windows)]
    #[test]
    fn windows_solution_arguments_never_use_verbatim_paths() {
        let path = std::path::PathBuf::from(r"\\?\C:\code\Demo.slnx");
        assert_eq!(
            without_windows_verbatim_prefix(&path),
            std::path::PathBuf::from(r"C:\code\Demo.slnx")
        );
        let unc = std::path::PathBuf::from(r"\\?\UNC\server\share\Demo.sln");
        assert_eq!(
            without_windows_verbatim_prefix(&unc),
            std::path::PathBuf::from(r"\\server\share\Demo.sln")
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_unc_paths_round_trip_through_file_uris() {
        let path = std::path::PathBuf::from(r"\\server\share\hello world.cs");
        let uri = file_uri(&path);
        assert_eq!(uri, "file://server/share/hello%20world.cs");
        let decoded = path_from_uri(&file_uri(&path)).unwrap();
        assert_eq!(decoded, path);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_paths_round_trip_through_file_uris() {
        let path = std::path::PathBuf::from("/Users/test/project/main.rs");
        let uri = file_uri(&path);
        assert_eq!(uri, "file:///Users/test/project/main.rs");
        let decoded = path_from_uri(&uri).unwrap();
        assert_eq!(decoded, path);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_paths_round_trip_through_file_uris() {
        let path = std::path::PathBuf::from("/home/test/project/main.rs");
        let uri = file_uri(&path);
        assert_eq!(uri, "file:///home/test/project/main.rs");
        let decoded = path_from_uri(&uri).unwrap();
        assert_eq!(decoded, path);
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

    #[test]
    #[ignore = "requires csharp-ls and a .NET SDK on PATH"]
    fn csharp_ls_definition_and_formatting_round_trip_for_a_loaded_solution() {
        if !Command::new("csharp-ls")
            .arg("--version")
            .status()
            .is_ok_and(|status| status.success())
        {
            return;
        }

        let root = std::env::temp_dir().join(format!("caret-csharp-ls-{}", std::process::id()));
        let project = root.join("src").join("Demo");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&project).expect("create C# fixture");
        std::fs::write(
            root.join("Demo.sln"),
            r#"
Microsoft Visual Studio Solution File, Format Version 12.00
# Visual Studio Version 17
Project("{FAE04EC0-301F-11D3-BF4B-00C04F79EFBC}") = "Demo", "src\Demo\Demo.csproj", "{11111111-1111-1111-1111-111111111111}"
EndProject
Global
EndGlobal
"#
            .trim_start(),
        )
        .expect("write solution");
        std::fs::write(
            project.join("Demo.csproj"),
            r#"<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <OutputType>Exe</OutputType>
    <TargetFramework>net8.0</TargetFramework>
  </PropertyGroup>
</Project>
"#,
        )
        .expect("write project");
        std::fs::write(
            project.join("Greeter.cs"),
            "namespace Demo;\npublic class Greeter { public string Message() => \"hi\"; }\n",
        )
        .expect("write definition source");
        let program = project.join("Program.cs");
        let program_text =
            "using Demo;\nvar greeter = new Greeter();\nConsole.WriteLine(greeter.Message());\n";
        std::fs::write(&program, program_text).expect("write use source");

        let canonical_root = root.canonicalize().expect("canonical fixture root");
        let mut client = LspClient::start("csharp-ls", &canonical_root).expect("start csharp-ls");
        let load_deadline = Instant::now() + Duration::from_secs(30);
        let mut initialized = false;
        while Instant::now() < load_deadline && !initialized {
            if let Some(message) = client.try_recv() {
                if message["id"].as_u64() == Some(1) {
                    assert!(message.get("error").is_none(), "{message}");
                    client
                        .notify("initialized", json!({}))
                        .expect("initialized notification");
                    client
                        .notify(
                            "textDocument/didOpen",
                            json!({
                                "textDocument": {
                                    "uri": file_uri(&program),
                                    "languageId": "csharp",
                                    "version": 1,
                                    "text": program_text
                                }
                            }),
                        )
                        .expect("didOpen");
                    initialized = true;
                } else if message.get("method").is_some() && message.get("id").is_some() {
                    let count = message["params"]["items"].as_array().map_or(0, Vec::len);
                    let result = if message["method"] == "workspace/configuration" {
                        json!(vec![json!({}); count])
                    } else if message["method"] == "workspace/workspaceFolders" {
                        json!([{
                            "uri": file_uri(&root),
                            "name": "Demo"
                        }])
                    } else {
                        Value::Null
                    };
                    client
                        .respond(&message["id"], result)
                        .expect("respond to server");
                }
            } else {
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        assert!(initialized, "csharp-ls did not initialize in time");

        let request = client
            .request(
                "textDocument/definition",
                json!({
                    "textDocument": { "uri": file_uri(&program) },
                    "position": { "line": 1, "character": 22 }
                }),
            )
            .expect("definition request");
        let request_deadline = Instant::now() + Duration::from_secs(60);
        while Instant::now() < request_deadline {
            if let Some(message) = client.try_recv() {
                if message["id"].as_u64() == Some(request) {
                    assert!(message.get("error").is_none(), "{message}");
                    let location = message["result"]
                        .as_array()
                        .and_then(|locations| locations.first())
                        .unwrap_or(&message["result"]);
                    let uri = location["uri"]
                        .as_str()
                        .or_else(|| location["targetUri"].as_str())
                        .expect("definition URI");
                    assert!(
                        uri.ends_with("/Greeter.cs"),
                        "unexpected definition response: {message}"
                    );
                    let formatting = client
                        .request(
                            "textDocument/formatting",
                            json!({
                                "textDocument": { "uri": file_uri(&program) },
                                "options": { "tabSize": 4, "insertSpaces": true }
                            }),
                        )
                        .expect("formatting request");
                    let formatting_deadline = Instant::now() + Duration::from_secs(30);
                    while Instant::now() < formatting_deadline {
                        if let Some(format_message) = client.try_recv() {
                            if format_message["id"].as_u64() == Some(formatting) {
                                assert!(format_message.get("error").is_none(), "{format_message}");
                                assert!(
                                    format_message["result"].is_array()
                                        || format_message["result"].is_null(),
                                    "unexpected formatting response: {format_message}"
                                );
                                let _ = std::fs::remove_dir_all(root);
                                return;
                            }
                            if format_message.get("method").is_some()
                                && format_message.get("id").is_some()
                            {
                                client
                                    .respond(&format_message["id"], Value::Null)
                                    .expect("respond to server");
                            }
                        } else {
                            std::thread::sleep(Duration::from_millis(20));
                        }
                    }
                    panic!("csharp-ls formatting timed out");
                }
                if message.get("method").is_some() && message.get("id").is_some() {
                    client
                        .respond(&message["id"], Value::Null)
                        .expect("respond to server");
                }
            } else {
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        panic!("csharp-ls definition timed out");
    }
}
