//! Linux/macOS: an advisory lock file elects the primary, which serves a Unix
//! domain socket next to it. The lock (not the socket) is the source of truth,
//! so a stale socket left by a crashed primary never blocks a new one.

use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{self, Write};
use std::net::Shutdown;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::protocol;

/// How long a secondary waits for a primary that holds the lock but has not
/// bound its socket yet.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);

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
            Ok(Acquired::Primary(Listener { socket, socket_path, _lock: lock }))
        }
        Err(TryLockError::WouldBlock) => Ok(Acquired::Secondary(Client { socket_path })),
        Err(TryLockError::Error(e)) => Err(e),
    }
}

pub fn serve(listener: Listener, mut on_message: impl FnMut(Vec<String>)) {
    for stream in listener.socket.incoming() {
        let Ok(stream) = stream else { continue };
        if let Ok(Some(args)) = protocol::read_message(stream) {
            on_message(args);
        }
    }
}

pub fn send(client: Client, message: &[u8]) -> io::Result<()> {
    let deadline = Instant::now() + CONNECT_TIMEOUT;
    let mut stream = loop {
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
    stream.write_all(message)?;
    stream.shutdown(Shutdown::Write)
}
