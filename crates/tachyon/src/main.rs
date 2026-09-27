// Release builds on Windows are GUI-subsystem executables (no console window).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod cli;
mod startup;

use std::process::ExitCode;

use tachyon_platform::{Instance, Listener};

use crate::cli::{Cli, Command};

/// Name of the single-instance channel. `TACHYON_INSTANCE_ID` overrides it
/// so benchmarks and tests do not talk to the user's running instance.
pub(crate) fn instance_id() -> String {
    std::env::var("TACHYON_INSTANCE_ID").unwrap_or_else(|_| "tachyon".to_owned())
}

fn main() -> ExitCode {
    let startup = startup::Startup::begin();
    let command = cli::parse(std::env::args_os().skip(1));
    if !matches!(command, Ok(Command::Run(ref cli)) if !cli.quit) {
        // Only command-line output follows; make it visible when started from a console.
        tachyon_platform::attach_parent_console();
    }
    let cli = match command {
        Ok(Command::Run(cli)) if cli.quit => return quit_running_instance(&cli),
        Ok(Command::Run(cli)) => cli,
        Ok(Command::Status) => return status(),
        Ok(Command::Autostart(enabled)) => return autostart(enabled),
        Ok(Command::DesktopEntry(enabled)) => return desktop_entry(enabled),
        Ok(Command::Help) => {
            print!("{}", cli::USAGE);
            return ExitCode::SUCCESS;
        }
        Ok(Command::Version) => {
            println!("tachyon {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("tachyon: {e}\n\n{}", cli::USAGE);
            return ExitCode::from(2);
        }
    };

    let listener = if cli.new_instance {
        None
    } else {
        match claim_instance(&cli) {
            Claim::Forwarded => return ExitCode::SUCCESS,
            Claim::Primary(listener) => Some(listener),
            Claim::Standalone => None,
        }
    };

    // Without a display GPUI falls back to a headless platform whose windows
    // never render, so the process would hang. Forwarding above still works.
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    if gpui::guess_compositor() == "Headless" {
        eprintln!("tachyon: no display server (neither WAYLAND_DISPLAY nor DISPLAY is set)");
        return ExitCode::FAILURE;
    }

    app::run(cli, listener, startup);
    ExitCode::SUCCESS
}

/// `--status`: exit code 0 if an instance is running, 1 if not.
fn status() -> ExitCode {
    let running = match Instance::acquire(&instance_id()) {
        Ok(Instance::Secondary(_)) => true,
        // Claimed only to check; dropping the listener releases it at once.
        Ok(Instance::Primary(_)) => false,
        Err(e) => {
            eprintln!("tachyon: instance check failed: {e}");
            return ExitCode::FAILURE;
        }
    };
    println!("instance: {}", if running { "running" } else { "not running" });
    match tachyon_platform::autostart_enabled() {
        Ok(on) => println!("autostart: {}", if on { "on" } else { "off" }),
        Err(e) => println!("autostart: unknown ({e})"),
    }
    if running { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}

/// `--quit`: forwards the request; nothing running is not an error.
fn quit_running_instance(cli: &Cli) -> ExitCode {
    let sent = match Instance::acquire(&instance_id()) {
        Ok(Instance::Secondary(client)) => cli.forward_args().and_then(|args| client.send(&args)),
        Ok(Instance::Primary(_)) => {
            println!("tachyon: no running instance");
            return ExitCode::SUCCESS;
        }
        Err(e) => Err(e),
    };
    match sent {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("tachyon: could not reach the running instance: {e}");
            ExitCode::FAILURE
        }
    }
}

/// `--desktop-entry on|off`.
fn desktop_entry(enabled: bool) -> ExitCode {
    let result =
        std::env::current_exe().and_then(|exe| tachyon_platform::set_desktop_entry(&exe, enabled));
    match result {
        Ok(()) => {
            println!("tachyon: desktop entry {}", if enabled { "installed" } else { "removed" });
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("tachyon: could not change the desktop entry: {e}");
            ExitCode::FAILURE
        }
    }
}

/// `--autostart on|off`.
fn autostart(enabled: bool) -> ExitCode {
    let result =
        std::env::current_exe().and_then(|exe| tachyon_platform::set_autostart(&exe, enabled));
    match result {
        Ok(()) => {
            println!("tachyon: autostart {}", if enabled { "on" } else { "off" });
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("tachyon: could not change autostart: {e}");
            ExitCode::FAILURE
        }
    }
}

enum Claim {
    Forwarded,
    Primary(Listener),
    Standalone,
}

/// Forwards this launch to a running instance if there is one. Any IPC
/// failure degrades to a standalone process instead of losing the launch.
fn claim_instance(cli: &Cli) -> Claim {
    match Instance::acquire(&instance_id()) {
        Ok(Instance::Primary(listener)) => Claim::Primary(listener),
        // A background start with nothing to open (login autostart) has
        // nothing to forward when an instance already runs.
        Ok(Instance::Secondary(_)) if cli.background && cli.opens_nothing() => Claim::Forwarded,
        Ok(Instance::Secondary(client)) => match cli.forward_args().and_then(|a| client.send(&a)) {
            Ok(()) => Claim::Forwarded,
            Err(e) => {
                eprintln!("tachyon: could not reach running instance ({e}); starting standalone");
                Claim::Standalone
            }
        },
        Err(e) => {
            eprintln!("tachyon: instance check failed ({e}); starting standalone");
            Claim::Standalone
        }
    }
}
