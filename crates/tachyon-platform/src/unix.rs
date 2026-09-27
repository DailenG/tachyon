//! Linux/macOS: an advisory lock file elects the primary, which serves a Unix
//! domain socket next to it. The lock (not the socket) is the source of truth,
//! so a stale socket left by a crashed primary never blocks a new one.

use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{self, Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::protocol;

/// How long a secondary waits for a primary that holds the lock but has not
/// bound its socket yet.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);

/// Bounds each read on either side, so a stalled peer cannot wedge the
/// listener thread or leave a secondary waiting forever for its reply.
const IO_TIMEOUT: Duration = Duration::from_secs(2);

pub enum Acquired {
    Primary(Listener),
    Secondary(Client),
}

pub struct Listener {
    socket: UnixListener,
    socket_path: PathBuf,
    _lock: File,
}

impl Drop for Listener {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.socket_path);
    }
}

pub struct Client {
    socket_path: PathBuf,
}

/// `$XDG_RUNTIME_DIR` is per-user and mode 0700. The fallback (macOS
/// `$TMPDIR` is per-user; `/tmp` is not) gets the user name in the file name.
fn paths(app_id: &str) -> (PathBuf, PathBuf) {
    let (dir, stem) = match std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from) {
        Some(dir) if dir.is_dir() => (dir, app_id.to_owned()),
        _ => {
            let user = std::env::var("USER").unwrap_or_default();
            (std::env::temp_dir(), format!("{app_id}-{user}"))
        }
    };
    (dir.join(format!("{stem}.lock")), dir.join(format!("{stem}.sock")))
}

pub fn acquire(app_id: &str) -> io::Result<Acquired> {
    let (lock_path, socket_path) = paths(app_id);
    let lock =
        OpenOptions::new().create(true).truncate(false).write(true).mode(0o600).open(&lock_path)?;
    match lock.try_lock() {
        Ok(()) => {
            match fs::remove_file(&socket_path) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
                _ => {}
            }
            let socket = UnixListener::bind(&socket_path)?;
            // Connecting needs write permission on the socket file; do not
            // rely on the umask when the directory is shared (/tmp fallback).
            fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600))?;
            Ok(Acquired::Primary(Listener { socket, socket_path, _lock: lock }))
        }
        Err(TryLockError::WouldBlock) => Ok(Acquired::Secondary(Client { socket_path })),
        Err(TryLockError::Error(e)) => Err(e),
    }
}

pub fn serve(listener: Listener, mut on_message: impl FnMut(Vec<String>) -> bool) {
    for stream in listener.socket.incoming() {
        let Ok(mut stream) = stream else { continue };
        let request = DeadlineReader { stream: &stream, deadline: Instant::now() + IO_TIMEOUT };
        if let Ok(Some(args)) = protocol::read_request(request)
            && on_message(args)
        {
            let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
            let _ = stream.write_all(protocol::ACK);
        }
    }
}

/// Bounds a whole request by one deadline, so a peer that trickles bytes
/// cannot hold the sequential listener longer than [`IO_TIMEOUT`].
struct DeadlineReader<'a> {
    stream: &'a UnixStream,
    deadline: Instant,
}

impl Read for DeadlineReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let remaining = self
            .deadline
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
            .ok_or_else(|| io::Error::from(io::ErrorKind::TimedOut))?;
        self.stream.set_read_timeout(Some(remaining))?;
        self.stream.read(buf)
    }
}

pub fn send(client: Client, request: &[u8]) -> io::Result<()> {
    let deadline = Instant::now() + CONNECT_TIMEOUT;
    let stream = loop {
        match UnixStream::connect(&client.socket_path) {
            Ok(stream) => break stream,
            Err(e)
                if Instant::now() < deadline
                    && matches!(
                        e.kind(),
                        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                    ) =>
            {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => return Err(e),
        }
    };
    // A primary that accepts but never reads must not block a large write.
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    protocol::send_request(&stream, request)
}

pub fn disable_window_transitions(_window: &impl raw_window_handle::HasWindowHandle) -> bool {
    false
}

pub fn attach_parent_console() {}

#[cfg(target_os = "macos")]
pub fn autostart_enabled() -> io::Result<bool> {
    Ok(false)
}

#[cfg(target_os = "macos")]
pub fn set_autostart(_exe: &std::path::Path, _enabled: bool) -> io::Result<()> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "autostart is not supported on macOS yet"))
}

#[cfg(not(target_os = "macos"))]
pub fn autostart_enabled() -> io::Result<bool> {
    Ok(autostart_entry()?.is_file())
}

#[cfg(not(target_os = "macos"))]
pub fn set_autostart(exe: &std::path::Path, enabled: bool) -> io::Result<()> {
    let entry = autostart_entry()?;
    if enabled {
        if let Some(dir) = entry.parent() {
            fs::create_dir_all(dir)?;
        }
        fs::write(&entry, autostart_desktop_file(exe))
    } else {
        match fs::remove_file(&entry) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            result => result,
        }
    }
}

/// `$XDG_CONFIG_HOME/autostart/tachyon.desktop` (default `~/.config`).
#[cfg(not(target_os = "macos"))]
fn autostart_entry() -> io::Result<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "neither XDG_CONFIG_HOME nor HOME is set")
        })?;
    Ok(config.join("autostart").join("tachyon.desktop"))
}

/// An XDG autostart entry running `exe --background`. The path is quoted as the Desktop Entry
/// spec requires for `Exec` (backslash before `"`, `` ` ``, `$` and `\`, `%` doubled), then
/// escaped as a desktop-file string (backslashes doubled).
#[cfg(not(target_os = "macos"))]
fn autostart_desktop_file(exe: &std::path::Path) -> String {
    let mut quoted = String::from("\"");
    for c in exe.to_string_lossy().chars() {
        match c {
            '"' | '`' | '$' | '\\' => {
                quoted.push('\\');
                quoted.push(c);
            }
            '%' => quoted.push_str("%%"),
            _ => quoted.push(c),
        }
    }
    quoted.push('"');
    let exec = quoted.replace('\\', "\\\\");
    format!(
        "[Desktop Entry]\nType=Application\nName=Tachyon\nComment=Keep Tachyon ready in the background\nExec={exec} --background\nNoDisplay=true\nX-GNOME-Autostart-enabled=true\n"
    )
}

#[cfg(all(test, not(target_os = "macos")))]
mod autostart_tests {
    use super::*;

    #[test]
    fn desktop_entry_quotes_the_executable_path() {
        let plain = autostart_desktop_file(std::path::Path::new("/opt/tachyon/tachyon"));
        assert!(plain.contains("\nExec=\"/opt/tachyon/tachyon\" --background\n"), "{plain}");
        let odd = autostart_desktop_file(std::path::Path::new("/home/a b/100%/$x\"y"));
        assert!(odd.contains("Exec=\"/home/a b/100%%/\\\\$x\\\\\"y\" --background"), "{odd}");
    }
}
