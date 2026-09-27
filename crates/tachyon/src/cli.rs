use std::ffi::OsString;
use std::io;
use std::path::PathBuf;

pub const USAGE: &str = "\
Usage: tachyon [OPTIONS] [FILE]...

Opens each FILE in its own window. Without FILE or --paste, opens a scratch
window. Launches are forwarded to an already running instance.

The instance stays running after its last window closes (resident mode, the
default on Windows), so later launches open in about the time of one frame.
Ctrl+Q, or `tachyon --quit`, ends it.

Options:
  -p, --paste          Open the clipboard contents
  -n, --new-instance   Do not forward to a running instance
      --resident       Keep running after the last window closes
      --no-resident    Exit when the last window closes
      --background     Start without a window and stay running (for login
                       autostart); exits if an instance is already running
      --status         Report whether an instance is running and whether it
                       starts at login
      --quit           Ask the running instance to quit (it asks about unsaved
                       changes first)
      --autostart <on|off>
                       Start in the background at login, or stop doing so
      --startup-report Print startup timings after the first frame, then exit
                       (implies --new-instance; used by `cargo xtask bench-startup`)
  -h, --help           Print help
  -V, --version        Print version
";

pub enum Command {
    Run(Cli),
    Help,
    Version,
    Status,
    Autostart(bool),
}

#[derive(Debug, Default)]
pub struct Cli {
    pub files: Vec<PathBuf>,
    pub paste: bool,
    pub new_instance: bool,
    pub startup_report: bool,
    /// `--resident` / `--no-resident`; `None` means the platform default.
    pub resident: Option<bool>,
    /// Start without a window (login autostart).
    pub background: bool,
    /// Ask the running instance to quit.
    pub quit: bool,
    /// Primary prints one `tachyon-launch` line per forwarded launch once its
    /// window has drawn, then closes it (`cargo xtask bench-startup --warm`).
    pub report_launches: bool,
}

pub fn parse(
    args: impl IntoIterator<Item = impl Into<OsString>>,
) -> Result<Command, lexopt::Error> {
    use lexopt::prelude::*;

    let mut cli = Cli::default();
    let mut parser = lexopt::Parser::from_args(args);
    while let Some(arg) = parser.next()? {
        match arg {
            Short('p') | Long("paste") => cli.paste = true,
            Short('n') | Long("new-instance") => cli.new_instance = true,
            Long("startup-report") => {
                cli.startup_report = true;
                cli.new_instance = true;
            }
            Long("resident") => cli.resident = Some(true),
            Long("no-resident") => cli.resident = Some(false),
            Long("background") => cli.background = true,
            Long("quit") => cli.quit = true,
            Long("status") => return Ok(Command::Status),
            Long("autostart") => {
                return match parser.value()?.to_str() {
                    Some("on") => Ok(Command::Autostart(true)),
                    Some("off") => Ok(Command::Autostart(false)),
                    _ => Err(lexopt::Error::from("--autostart takes `on` or `off`")),
                };
            }
            Long("report-launches") => {
                cli.background = true;
                cli.report_launches = true;
            }
            Short('h') | Long("help") => return Ok(Command::Help),
            Short('V') | Long("version") => return Ok(Command::Version),
            Value(path) => cli.files.push(path.into()),
            _ => return Err(arg.unexpected()),
        }
    }
    Ok(Command::Run(cli))
}

impl Cli {
    /// Whether this process, if it becomes the primary instance, stays running
    /// after its last window closes.
    pub fn resident(&self) -> bool {
        self.background || self.resident.unwrap_or_else(tachyon_platform::resident_by_default)
    }

    /// Nothing to open: no files, no clipboard.
    pub fn opens_nothing(&self) -> bool {
        self.files.is_empty() && !self.paste
    }

    /// Arguments for the primary instance. Paths are made absolute because the
    /// primary has a different working directory.
    pub fn forward_args(&self) -> io::Result<Vec<String>> {
        let mut args = Vec::with_capacity(self.files.len() + 3);
        if self.quit {
            args.push("--quit".to_owned());
        }
        if self.paste {
            args.push("--paste".to_owned());
        }
        args.push("--".to_owned());
        for file in &self.files {
            let path = std::path::absolute(file)?.into_os_string();
            let path = path.into_string().map_err(|path| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("path is not valid Unicode: {}", path.to_string_lossy()),
                )
            })?;
            args.push(path);
        }
        Ok(args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(args: &[&str]) -> Cli {
        match parse(args.iter().copied()).expect("valid arguments") {
            Command::Run(cli) => cli,
            _ => panic!("not a run command"),
        }
    }

    #[test]
    fn residency_follows_flags_then_the_platform_default() {
        assert!(run(&["--resident"]).resident());
        assert!(!run(&["--no-resident"]).resident());
        assert!(run(&["--no-resident", "--background"]).resident(), "background implies it");
        assert_eq!(run(&[]).resident(), tachyon_platform::resident_by_default());
    }

    #[test]
    fn quit_is_forwarded_to_the_running_instance() {
        let args = run(&["--quit"]).forward_args().expect("no paths");
        assert_eq!(args, vec!["--quit", "--"]);
        match parse(args) {
            Ok(Command::Run(cli)) => assert!(cli.quit && cli.opens_nothing()),
            _ => panic!("forwarded arguments must parse"),
        }
    }

    #[test]
    fn autostart_takes_on_or_off() {
        assert!(matches!(parse(["--autostart", "on"]), Ok(Command::Autostart(true))));
        assert!(matches!(parse(["--autostart", "off"]), Ok(Command::Autostart(false))));
        assert!(parse(["--autostart", "yes"]).is_err());
        assert!(parse(["--autostart"]).is_err());
    }
}
