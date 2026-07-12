mod app;
mod config;
mod editor;
mod lsp;
mod project;
mod syntax;
mod tabs;
mod theme;
mod ui;

use std::{
    env,
    io::{self, stdout, Write},
    path::PathBuf,
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
        let _ = execute!(
            stdout(),
            Show,
            DisableMouseCapture,
            LeaveAlternateScreen
        );
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
         \n\
         OPTIONS:\n\
         \x20 -h, --help       Show this help\n\
         \x20 -V, --version    Show version\n\
         \n\
         Inside the editor, press F1 or ? for key bindings.",
        version = env!("CARGO_PKG_VERSION")
    );
}

fn parse_args() -> Option<PathBuf> {
    let mut file = None;

    for arg in env::args().skip(1) {
        match arg.as_str() {
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
    // Draw once, then block until an input or resize event occurs.
    // This prevents the terminal from being cleared and repainted on a timer.
    ui::draw(out, app)?;

    while !app.should_quit {
        let event = event::read()?;
        if app.handle_event(event) && !app.should_quit {
            ui::draw(out, app)?;
        }
    }

    Ok(())
}

fn main() {
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
