use std::{
    collections::HashMap,
    fs, io,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use crossterm::style::Color;
use serde::{Deserialize, Serialize};

use crate::theme::{Theme, ThemeKind};

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct PluginManifest {
    pub name: String,
    pub version: String,
    pub commands: Vec<PluginCommand>,
    pub languages: Vec<PluginLanguage>,
    pub themes: Vec<PluginTheme>,
    pub hooks: PluginHooks,
}

impl Default for PluginManifest {
    fn default() -> Self {
        Self {
            name: String::new(),
            version: "0.1.0".to_string(),
            commands: Vec::new(),
            languages: Vec::new(),
            themes: Vec::new(),
            hooks: PluginHooks::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct PluginCommand {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PluginLanguage {
    pub name: String,
    #[serde(default)]
    pub extensions: Vec<String>,
    pub line_comment: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PluginTheme {
    pub name: String,
    #[serde(default)]
    pub base: ThemeKind,
    #[serde(default)]
    pub colors: HashMap<String, String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PluginHooks {
    #[serde(default)]
    pub on_save: Vec<String>,
}

#[derive(Debug, Clone)]
struct LoadedPlugin {
    directory: PathBuf,
    manifest: PluginManifest,
}

#[derive(Debug, Default)]
pub struct PluginRegistry {
    plugins: Vec<LoadedPlugin>,
    errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PluginContext {
    pub project: String,
    pub file: Option<String>,
    pub language: String,
    pub text: String,
    pub selection: Option<String>,
    pub cursor_line: usize,
    pub cursor_column: usize,
    pub arguments: Vec<String>,
    pub event: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct PluginResponse {
    pub message: Option<String>,
    pub replace_document: Option<String>,
    pub replace_selection: Option<String>,
    pub insert_text: Option<String>,
    pub open: Option<String>,
}

impl PluginRegistry {
    pub fn load(directory: &Path) -> Self {
        let mut registry = Self::default();
        let Ok(entries) = fs::read_dir(directory) else {
            return registry;
        };
        let mut manifests = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("toml"))
            })
            .collect::<Vec<_>>();
        manifests.sort();
        for path in manifests {
            match fs::read_to_string(&path).and_then(|contents| {
                toml::from_str::<PluginManifest>(&contents)
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
            }) {
                Ok(manifest) if !manifest.name.trim().is_empty() => {
                    registry.plugins.push(LoadedPlugin {
                        directory: path.parent().unwrap_or(directory).to_path_buf(),
                        manifest,
                    })
                }
                Ok(_) => registry
                    .errors
                    .push(format!("{}: plugin name is required", path.display())),
                Err(error) => registry.errors.push(format!("{}: {error}", path.display())),
            }
        }
        registry
    }

    pub fn count(&self) -> usize {
        self.plugins.len()
    }
    pub fn errors(&self) -> &[String] {
        &self.errors
    }

    pub fn summary(&self) -> String {
        if self.plugins.is_empty() {
            return "No plugins loaded. Add TOML manifests to the plugins directory.".to_string();
        }
        self.plugins
            .iter()
            .map(|plugin| {
                let commands = plugin
                    .manifest
                    .commands
                    .iter()
                    .map(|command| {
                        if command.description.is_empty() {
                            command.name.clone()
                        } else {
                            format!("{} — {}", command.name, command.description)
                        }
                    })
                    .collect::<Vec<_>>();
                if commands.is_empty() {
                    format!("{} {}", plugin.manifest.name, plugin.manifest.version)
                } else {
                    format!(
                        "{} {} [{}]",
                        plugin.manifest.name,
                        plugin.manifest.version,
                        commands.join(", ")
                    )
                }
            })
            .collect::<Vec<_>>()
            .join(" · ")
    }

    pub fn command_names(&self) -> Vec<&str> {
        self.plugins
            .iter()
            .flat_map(|plugin| {
                plugin
                    .manifest
                    .commands
                    .iter()
                    .map(|command| command.name.as_str())
            })
            .collect()
    }

    pub fn language_for_path(&self, path: Option<&Path>) -> Option<&PluginLanguage> {
        let extension = path?.extension()?.to_str()?.trim_start_matches('.');
        self.plugins
            .iter()
            .flat_map(|plugin| &plugin.manifest.languages)
            .find(|language| {
                language.extensions.iter().any(|candidate| {
                    candidate
                        .trim_start_matches('.')
                        .eq_ignore_ascii_case(extension)
                })
            })
    }

    pub fn theme(&self, name: &str) -> Option<Theme> {
        let definition = self
            .plugins
            .iter()
            .flat_map(|plugin| &plugin.manifest.themes)
            .find(|theme| theme.name.eq_ignore_ascii_case(name))?;
        let mut theme = Theme::for_kind(definition.base);
        apply_colors(&mut theme, &definition.colors);
        Some(theme)
    }

    pub fn theme_names(&self) -> Vec<&str> {
        self.plugins
            .iter()
            .flat_map(|plugin| {
                plugin
                    .manifest
                    .themes
                    .iter()
                    .map(|theme| theme.name.as_str())
            })
            .collect()
    }

    pub fn on_save_commands(&self) -> Vec<String> {
        self.plugins
            .iter()
            .flat_map(|plugin| plugin.manifest.hooks.on_save.iter().cloned())
            .collect()
    }

    pub fn run(&self, name: &str, context: &PluginContext) -> Result<PluginResponse, String> {
        let (plugin, command) = self
            .plugins
            .iter()
            .find_map(|plugin| {
                plugin
                    .manifest
                    .commands
                    .iter()
                    .find(|command| command.name.eq_ignore_ascii_case(name))
                    .map(|command| (plugin, command))
            })
            .ok_or_else(|| format!("Unknown plugin command: {name}"))?;
        let program = resolve_program(&plugin.directory, &command.program);
        let payload =
            serde_json::to_vec(context).map_err(|error| format!("Plugin input failed: {error}"))?;
        let deadline = Instant::now() + Duration::from_millis(command.timeout_ms.max(100));
        let child = Command::new(&program)
            .args(&command.args)
            .current_dir(&plugin.directory)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("Could not start plugin command {name}: {error}"))?;
        let mut guard = PluginChild(child);
        let mut input = guard.0.stdin.take().expect("piped stdin");
        let output = guard.0.stdout.take().expect("piped stdout");
        let errors = guard.0.stderr.take().expect("piped stderr");
        let (sender, receiver) = std::sync::mpsc::channel();
        let writer_sender = sender.clone();
        thread::spawn(move || {
            let result = input
                .write_all(&payload)
                .and_then(|()| input.flush())
                .map(|()| Vec::new());
            drop(input);
            let _ = writer_sender.send((0, result));
        });
        drain_plugin_pipe(output, sender.clone(), 1);
        drain_plugin_pipe(errors, sender, 2);
        let mut streams: [Option<Vec<u8>>; 3] = [None, None, None];
        let status = loop {
            if Instant::now() >= deadline {
                return Err(format!(
                    "Plugin {name} timed out after {} ms",
                    command.timeout_ms.max(100)
                ));
            }
            while let Ok((index, result)) = receiver.try_recv() {
                streams[index] =
                    Some(result.map_err(|error| format!("Plugin I/O failed: {error}"))?);
            }
            let exited = guard
                .0
                .try_wait()
                .map_err(|error| format!("Plugin command failed: {error}"))?;
            if let Some(status) = exited {
                if streams.iter().all(Option::is_some) {
                    break status;
                }
            }
            thread::sleep(Duration::from_millis(5));
        };
        let output = std::process::Output {
            status,
            stdout: streams[1].take().unwrap_or_default(),
            stderr: streams[2].take().unwrap_or_default(),
        };
        if !output.status.success() {
            return Err(format!(
                "Plugin {name} failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        if output.stdout.is_empty() {
            return Ok(PluginResponse::default());
        }
        serde_json::from_slice(&output.stdout)
            .map_err(|error| format!("Plugin {name} returned invalid JSON: {error}"))
    }
}

struct PluginChild(std::process::Child);
impl Drop for PluginChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn drain_plugin_pipe(
    mut pipe: impl Read + Send + 'static,
    sender: std::sync::mpsc::Sender<(usize, io::Result<Vec<u8>>)>,
    index: usize,
) {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = pipe.read_to_end(&mut bytes).map(|_| bytes);
        let _ = sender.send((index, result));
    });
}

fn default_timeout_ms() -> u64 {
    15_000
}

fn resolve_program(directory: &Path, program: &str) -> PathBuf {
    let path = PathBuf::from(program);
    if path.is_absolute() || (!program.contains('/') && !program.contains('\\')) {
        path
    } else {
        directory.join(path)
    }
}

fn apply_colors(theme: &mut Theme, colors: &HashMap<String, String>) {
    for (name, value) in colors {
        let Some(color) = parse_color(value) else {
            continue;
        };
        match name.as_str() {
            "background" => theme.background = color,
            "foreground" => theme.foreground = color,
            "muted" => theme.muted = color,
            "current_line" => theme.current_line = color,
            "top_bar" => theme.top_bar = color,
            "top_bar_text" => theme.top_bar_text = color,
            "status_bar" => theme.status_bar = color,
            "status_text" => theme.status_text = color,
            "prompt_bar" => theme.prompt_bar = color,
            "prompt_text" => theme.prompt_text = color,
            "keyword" => theme.keyword = color,
            "string" => theme.string = color,
            "number" => theme.number = color,
            "comment" => theme.comment = color,
            "type_name" => theme.type_name = color,
            "heading" => theme.heading = color,
            "border" => theme.border = color,
            "overlay" => theme.overlay = color,
            "overlay_text" => theme.overlay_text = color,
            "error" => theme.error = color,
            "success" => theme.success = color,
            _ => {}
        }
    }
}

fn parse_color(value: &str) -> Option<Color> {
    let hex = value.trim().strip_prefix('#').unwrap_or(value.trim());
    if hex.len() != 6 {
        return None;
    }
    Some(Color::Rgb {
        r: u8::from_str_radix(&hex[0..2], 16).ok()?,
        g: u8::from_str_radix(&hex[2..4], 16).ok()?,
        b: u8::from_str_radix(&hex[4..6], 16).ok()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plugin_theme_colors() {
        assert_eq!(
            parse_color("#12abEF"),
            Some(Color::Rgb {
                r: 0x12,
                g: 0xab,
                b: 0xef
            })
        );
        assert_eq!(parse_color("bad"), None);
    }

    #[test]
    fn loads_sample_capabilities() {
        let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/plugins");
        let registry = PluginRegistry::load(&directory);
        assert_eq!(registry.count(), 1, "{:?}", registry.errors());
        assert!(registry.theme("sample-midnight").is_some());
        assert_eq!(
            registry
                .language_for_path(Some(Path::new("todo.note")))
                .map(|language| language.name.as_str()),
            Some("Caret Notes")
        );
        assert_eq!(registry.command_names(), vec!["uppercase"]);
    }

    #[cfg(windows)]
    #[test]
    fn sample_command_round_trips_json() {
        let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/plugins");
        let registry = PluginRegistry::load(&directory);
        let context = PluginContext {
            project: directory.display().to_string(),
            file: None,
            language: "Plain Text".to_string(),
            text: "hello".to_string(),
            selection: Some("hello".to_string()),
            cursor_line: 0,
            cursor_column: 5,
            arguments: Vec::new(),
            event: "command".to_string(),
        };
        let response = registry
            .run("uppercase", &context)
            .expect("run sample plugin");
        assert_eq!(response.replace_selection.as_deref(), Some("HELLO"));
    }

    fn shell_registry(script: &str, timeout_ms: u64) -> (PluginRegistry, PluginContext) {
        #[cfg(windows)]
        let (program, args) = (
            "powershell.exe",
            vec!["-NoProfile", "-NonInteractive", "-Command", script],
        );
        #[cfg(not(windows))]
        let (program, args) = ("/bin/sh", vec!["-c", script]);
        let directory = std::env::temp_dir();
        let registry = PluginRegistry {
            plugins: vec![LoadedPlugin {
                directory: directory.clone(),
                manifest: PluginManifest {
                    name: "test".into(),
                    commands: vec![PluginCommand {
                        name: "test".into(),
                        description: String::new(),
                        program: program.into(),
                        args: args.into_iter().map(str::to_string).collect(),
                        timeout_ms,
                    }],
                    ..Default::default()
                },
            }],
            errors: vec![],
        };
        let context = PluginContext {
            project: directory.display().to_string(),
            file: None,
            language: "Text".into(),
            text: "x".repeat(2 * 1024 * 1024),
            selection: None,
            cursor_line: 0,
            cursor_column: 0,
            arguments: vec![],
            event: "command".into(),
        };
        (registry, context)
    }
    #[test]
    fn drains_both_large_pipes_while_sending_large_input() {
        #[cfg(windows)]
        let script = r#"[Console]::Out.Write('{"message":"'); [Console]::Out.Write(('x' * 1048576)); [Console]::Error.Write(('e' * 1048576)); $inputText = [Console]::In.ReadToEnd(); if ($inputText.Length -lt 2097152) { exit 2 }; [Console]::Out.Write('"}')"#;
        #[cfg(not(windows))]
        let script = r#"printf '{"message":"'; head -c 1048576 /dev/zero | tr '\000' x; head -c 1048576 /dev/zero | tr '\000' e >&2; cat >/dev/null; printf '"}'"#;
        let (registry, context) = shell_registry(script, 15000);
        let response = registry.run("test", &context).unwrap();
        assert_eq!(response.message.unwrap().len(), 1048576);
    }
    #[test]
    fn malformed_plugin_responses_are_reported() {
        #[cfg(windows)]
        let script = "[Console]::In.ReadToEnd() | Out-Null; [Console]::Out.Write('invalid')";
        #[cfg(not(windows))]
        let script = "cat >/dev/null; printf invalid";
        let (registry, context) = shell_registry(script, 10000);
        assert!(registry
            .run("test", &context)
            .unwrap_err()
            .contains("invalid JSON"));
    }
    #[test]
    fn timeout_covers_blocked_stdin_and_reaps_the_child() {
        #[cfg(windows)]
        let script = "Start-Sleep -Seconds 30";
        #[cfg(not(windows))]
        let script = "exec sleep 30";
        let (registry, context) = shell_registry(script, 1000);
        let started = Instant::now();
        assert!(registry
            .run("test", &context)
            .unwrap_err()
            .contains("timed out"));
        assert!(started.elapsed() < Duration::from_secs(5));
        let command = &registry.plugins[0].manifest.commands[0];
        let child = Command::new(&command.program)
            .args(&command.args)
            .spawn()
            .unwrap();
        let id = child.id();
        drop(PluginChild(child));
        #[cfg(unix)]
        unsafe {
            assert_eq!(
                libc::waitpid(id as i32, std::ptr::null_mut(), libc::WNOHANG),
                -1
            );
        }
        #[cfg(windows)]
        {
            let status = Command::new("powershell.exe")
                .args([
                    "-NoProfile",
                    "-Command",
                    &format!(
                        "if (Get-Process -Id {id} -ErrorAction SilentlyContinue) {{ exit 1 }}"
                    ),
                ])
                .status()
                .unwrap();
            assert!(status.success(), "timed-out child still running");
        }
    }
}
