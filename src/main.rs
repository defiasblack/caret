mod app;
mod clipboard;
mod config;
mod diagnostics;
mod document;
mod editor;
mod lsp;
mod plugin;
mod project;
mod project_search;
mod recovery;
mod search;
mod session;
mod syntax;
mod tabs;
mod terminal;
mod theme;
mod ui;

use std::{
    env,
    io::{self, stdout, Write},
    path::PathBuf,
    time::Duration,
};

use app::App;
use crossterm::{
    cursor::{Hide, Show},
    event::{self, DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{
        disable_raw_mode, enable_raw_mode, Clear, ClearType, EnterAlternateScreen,
        LeaveAlternateScreen,
    },
};

struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> io::Result<Self> {
        enable_raw_mode()?;
        execute!(
            stdout(),
            EnterAlternateScreen,
            EnableMouseCapture,
            Hide,
            Clear(ClearType::All)
        )?;
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = execute!(stdout(), Show, DisableMouseCapture, LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}

fn print_help() {
    println!(
        "Caret {version}\n\
         \n\
         A fast terminal text editor written in Rust.\n\
         \n\
         USAGE:\n\
         \x20 caret [FILE_OR_DIRECTORY]\n\
         \x20 caret doctor\n\
         \n\
         OPTIONS:\n\
         \x20 -h, --help       Show this help\n\
         \x20 -V, --version    Show version\n\
         \n\
         Inside the editor, press F1 or ? for key bindings.",
        version = env!("CARGO_PKG_VERSION")
    );
}

fn print_doctor() {
    println!("{}", diagnostics::report(env!("CARGO_PKG_VERSION")));
}

fn parse_args() -> Option<PathBuf> {
    let mut file = None;

    for arg in env::args().skip(1) {
        match arg.as_str() {
            "doctor" => {
                print_doctor();
                std::process::exit(0);
            }
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            "-V" | "--version" => {
                println!("caret {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            value if value.starts_with('-') => {
                eprintln!("Unknown option: {value}");
                eprintln!("Run `caret --help` for usage.");
                std::process::exit(2);
            }
            value => {
                if file.is_some() {
                    eprintln!("Caret accepts one file or directory at a time.");
                    std::process::exit(2);
                }
                file = Some(PathBuf::from(value));
            }
        }
    }

    file
}

fn run<W: Write>(out: &mut W, app: &mut App) -> io::Result<()> {
    ui::draw(out, app)?;

    while !app.should_quit {
        let changed = if event::poll(Duration::from_millis(50))? {
            app.handle_event(event::read()?)
        } else {
            app.poll_background()
        };
        if changed && !app.should_quit {
            ui::draw(out, app)?;
        }
    }

    Ok(())
}

fn main() {
    std::panic::set_hook(Box::new(|info| {
        let location = info
            .location()
            .map(|location| format!("{}:{}", location.file(), location.line()))
            .unwrap_or_else(|| "unknown location".to_string());
        let _ = diagnostics::append("panic", &format!("Caret panic at {location}: {info}"));
        // The release build aborts on panic, so Drop will not run. Restore the
        // user's terminal from the hook before that abort can strand raw mode.
        let _ = execute!(stdout(), Show, DisableMouseCapture, LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }));
    let path = parse_args();

    let mut app = match App::new(path.as_deref()) {
        Ok(app) => app,
        Err(error) => {
            eprintln!("caret: {error}");
            std::process::exit(1);
        }
    };

    let guard = match TerminalGuard::enter() {
        Ok(guard) => guard,
        Err(error) => {
            eprintln!("caret: could not initialize terminal: {error}");
            std::process::exit(1);
        }
    };

    let mut out = stdout();
    let result = run(&mut out, &mut app);

    drop(guard);

    if let Err(error) = result {
        eprintln!("caret: {error}");
        std::process::exit(1);
    }
}
