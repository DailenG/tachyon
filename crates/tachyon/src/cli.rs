use std::ffi::OsString;
use std::io;
use std::path::PathBuf;

pub const USAGE: &str = "\
Usage: tachyon [OPTIONS] [FILE]...

Opens each FILE in its own window. Without FILE or --paste, opens a scratch
window. Launches are forwarded to an already running instance.

Options:
  -p, --paste          Open the clipboard contents
  -n, --new-instance   Do not forward to a running instance
      --resident       Keep running after the last window closes, so later
                       launches open in about the time of one frame. Without
                       FILE or --paste, starts with no window (for login
                       autostart); exits if an instance is already running.
                       Ctrl+Q quits for real.
      --startup-report Print startup timings after the first frame, then exit
                       (implies --new-instance; used by `cargo xtask bench-startup`)
  -h, --help           Print help
  -V, --version        Print version
";

pub enum Command {
    Run(Cli),
    Help,
    Version,
}

#[derive(Debug, Default)]
pub struct Cli {
    pub files: Vec<PathBuf>,
    pub paste: bool,
    pub new_instance: bool,
    pub startup_report: bool,
    pub resident: bool,
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
            Long("resident") => cli.resident = true,
            Long("report-launches") => {
                cli.resident = true;
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
    /// Arguments for the primary instance. Paths are made absolute because the
    /// primary has a different working directory.
    pub fn forward_args(&self) -> io::Result<Vec<String>> {
        let mut args = Vec::with_capacity(self.files.len() + 2);
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
