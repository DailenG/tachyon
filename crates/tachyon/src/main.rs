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
fn instance_id() -> String {
    std::env::var("TACHYON_INSTANCE_ID").unwrap_or_else(|_| "tachyon".to_owned())
}

fn main() -> ExitCode {
    let startup = startup::Startup::begin();
    let cli = match cli::parse(std::env::args_os().skip(1)) {
        Ok(Command::Run(cli)) => cli,
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
        // A resident launch with nothing to open (login autostart) has
        // nothing to forward when an instance already runs.
        Ok(Instance::Secondary(_)) if cli.resident && cli.files.is_empty() && !cli.paste => {
            Claim::Forwarded
        }
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
