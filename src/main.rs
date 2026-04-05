//! Entry point: parses CLI args, starts the sampler, and launches either
//! the interactive TUI or the streaming JSON output mode.

use anyhow::Result;
use clap::Parser;
use crossterm::event::{self, Event, KeyEventKind};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use linpow::app::App;
use linpow::sampler::Sampler;
use linpow::types::{CliArgs, Metrics};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io::stdout;
use std::sync::mpsc;
use std::time::Duration;

fn main() -> Result<()> {
    let args = CliArgs::parse();
    let interval = args.interval;
    let json_mode = args.json;

    // Bounded channel: sampler produces Metrics, main thread consumes them.
    // A capacity of 2 gives one slot of slack to absorb brief stalls.
    let (tx, rx) = mpsc::sync_channel::<Metrics>(2);

    // Sampler runs in a background thread; it spawns its own sub-threads internally.
    std::thread::spawn(move || {
        let sampler = Sampler::new(interval);
        loop {
            std::thread::sleep(Duration::from_millis(interval));
            let m = sampler.snapshot();
            if tx.send(m).is_err() {
                break; // receiver dropped — exit
            }
        }
    });

    if json_mode {
        run_json(rx)
    } else {
        run_tui(rx)
    }
}

/// Streaming JSON mode: prints one pretty-printed Metrics JSON per sample.
/// Installs a SIGINT handler so Ctrl+C exits cleanly (no broken-pipe mess).
fn run_json(rx: mpsc::Receiver<Metrics>) -> Result<()> {
    unsafe {
        libc::signal(
            libc::SIGINT,
            sigint_handler as *const () as libc::sighandler_t,
        );
    }
    loop {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(m) => println!("{}", serde_json::to_string_pretty(&m)?),
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    Ok(())
}

/// Restore the terminal to a usable state after TUI mode (or a panic).
fn restore_terminal() {
    let _ = stdout().execute(event::DisableMouseCapture);
    let _ = disable_raw_mode();
    let _ = stdout().execute(LeaveAlternateScreen);
}

/// Interactive TUI mode.
///
/// Wraps the main loop in `catch_unwind` so that panics don't leave the
/// terminal in raw mode — the user sees a helpful error instead of a
/// broken shell.
fn run_tui(rx: mpsc::Receiver<Metrics>) -> Result<()> {
    if unsafe { libc::isatty(libc::STDOUT_FILENO) } == 0 {
        anyhow::bail!("TUI requires a real terminal. Use --json for piped output.");
    }
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    stdout().execute(event::EnableMouseCapture)?;

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<()> {
        let backend = CrosstermBackend::new(stdout());
        let mut terminal = Terminal::new(backend)?;
        let mut app = App::new();

        loop {
            // Drain all pending metrics so we always render the latest state.
            while let Ok(m) = rx.try_recv() {
                app.update(m);
            }
            terminal.draw(|f| app.draw(f))?;
            if event::poll(Duration::from_millis(app.poll_interval_ms()))? {
                match event::read()? {
                    Event::Key(key) if key.kind == KeyEventKind::Press => {
                        if app.handle_key(key) {
                            break; // user requested quit
                        }
                    }
                    Event::Mouse(mouse) => {
                        app.handle_mouse(mouse);
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }));

    restore_terminal();

    match result {
        Ok(inner) => inner,
        Err(_) => anyhow::bail!("TUI panicked unexpectedly. Terminal has been restored."),
    }
}

extern "C" fn sigint_handler(_: libc::c_int) {
    std::process::exit(0);
}
