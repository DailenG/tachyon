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

#[cfg(not(target_os = "macos"))]
mod recently_used;

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

/// No native title bar to recolor here: Linux and macOS windows do not have one that follows
/// the OS dark-mode setting independently of the application.
pub fn set_title_bar_dark(_window: &impl raw_window_handle::HasWindowHandle, _dark: bool) -> bool {
    false
}

/// No popup menu to recolor here yet: Tachyon has none on Linux or macOS (the tray icon, the
/// only one so far, is Windows-only).
pub fn set_popup_menu_dark(_dark: bool) {}

pub fn attach_parent_console() {}

pub fn write_clipboard_html(
    _window: &impl raw_window_handle::HasWindowHandle,
    _html: &str,
    _text: &str,
) -> bool {
    false
}

/// X11 and Wayland clipboards are served by GPUI's event loop.
pub fn clipboard_text_reader() -> Option<fn() -> Option<String>> {
    None
}

/// No tray icon here yet; uninhabited, so no value exists.
pub enum Tray {}

pub fn show_tray(
    _tooltip: &str,
    _on_event: impl Fn(crate::TrayEvent) + Send + 'static,
) -> Option<Tray> {
    None
}

pub fn set_window_icon(_window: &impl raw_window_handle::HasWindowHandle) {}

/// No OS restart-registration API on Linux or macOS: Windows only (see `crate::register_restart`).
pub fn register_restart() {}

/// See [`register_restart`]; `crate::unregister_restart`.
pub fn unregister_restart() {}

#[cfg(target_os = "macos")]
pub fn query_system_appearance() -> Option<std::sync::mpsc::Receiver<bool>> {
    None
}

#[cfg(not(target_os = "macos"))]
pub fn query_system_appearance() -> Option<std::sync::mpsc::Receiver<bool>> {
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("appearance".to_owned())
        .spawn(move || {
            if let Some(dark) = portal_prefers_dark() {
                let _ = sender.send(dark);
            }
        })
        .ok()?;
    Some(receiver)
}

/// `org.freedesktop.appearance color-scheme` from the desktop portal: 1 prefers dark, 2 light,
/// 0 no preference (treated as light, as GPUI does).
#[cfg(not(target_os = "macos"))]
fn portal_prefers_dark() -> Option<bool> {
    use zbus::zvariant::{OwnedValue, Value};
    let connection = zbus::blocking::Connection::session().ok()?;
    let reply = connection
        .call_method(
            Some("org.freedesktop.portal.Desktop"),
            "/org/freedesktop/portal/desktop",
            Some("org.freedesktop.portal.Settings"),
            "ReadOne",
            &("org.freedesktop.appearance", "color-scheme"),
        )
        .ok()?;
    let value: OwnedValue = reply.body().deserialize().ok()?;
    // Some portals wrap the value in a second variant.
    let scheme = match &*value {
        Value::Value(inner) => u32::try_from(&**inner).ok()?,
        other => u32::try_from(other).ok()?,
    };
    Some(scheme == 1)
}

#[cfg(target_os = "macos")]
pub fn autostart_enabled() -> io::Result<bool> {
    Ok(false)
}

#[cfg(target_os = "macos")]
pub fn set_autostart(_exe: &std::path::Path, _enabled: bool) -> io::Result<()> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "autostart is not supported on macOS yet"))
}

/// The XDG autostart spec: a user entry overrides system entries (`$XDG_CONFIG_DIRS`, default
/// `/etc/xdg`) with the same name, and `Hidden=true` disables it.
#[cfg(not(target_os = "macos"))]
pub fn autostart_enabled() -> io::Result<bool> {
    let user = autostart_entry()?;
    if user.is_file() {
        return Ok(!is_hidden(&fs::read_to_string(&user)?));
    }
    for entry in system_autostart_entries() {
        if let Ok(text) = fs::read_to_string(entry) {
            return Ok(!is_hidden(&text));
        }
    }
    Ok(false)
}

/// Turning it off removes the user entry, or replaces it with a `Hidden=true` override when a
/// system entry would otherwise still start Tachyon.
#[cfg(not(target_os = "macos"))]
pub fn set_autostart(exe: &std::path::Path, enabled: bool) -> io::Result<()> {
    let entry = autostart_entry()?;
    let system = system_autostart_entries().into_iter().any(|path| path.is_file());
    let contents = match (enabled, system) {
        (true, _) => autostart_desktop_file(exe),
        (false, true) => {
            "[Desktop Entry]\nType=Application\nName=Tachyon\nHidden=true\n".to_owned()
        }
        (false, false) => {
            return match fs::remove_file(&entry) {
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
                result => result,
            };
        }
    };
    if let Some(dir) = entry.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(&entry, contents)
}

#[cfg(not(target_os = "macos"))]
fn system_autostart_entries() -> Vec<PathBuf> {
    let dirs = std::env::var("XDG_CONFIG_DIRS").ok().filter(|dirs| !dirs.is_empty());
    dirs.as_deref()
        .unwrap_or("/etc/xdg")
        .split(':')
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join("autostart").join("tachyon.desktop"))
        .collect()
}

/// Whether a desktop entry has `Hidden=true` in its `[Desktop Entry]` group.
#[cfg(not(target_os = "macos"))]
fn is_hidden(entry: &str) -> bool {
    let mut in_group = false;
    for line in entry.lines().map(str::trim) {
        if line.starts_with('[') {
            in_group = line == "[Desktop Entry]";
        } else if in_group && let Some(value) = line.strip_prefix("Hidden=") {
            return value.trim() == "true";
        }
    }
    false
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

/// `exe` quoted as the Desktop Entry spec requires for `Exec` (backslash before `"`, `` ` ``, `$`
/// and `\`, `%` doubled), then escaped as a desktop-file string (backslashes doubled).
#[cfg(not(target_os = "macos"))]
fn desktop_exec(exe: &std::path::Path) -> String {
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
    quoted.replace('\\', "\\\\")
}

/// An XDG autostart entry running `exe --background`.
#[cfg(not(target_os = "macos"))]
fn autostart_desktop_file(exe: &std::path::Path) -> String {
    let exec = desktop_exec(exe);
    format!(
        "[Desktop Entry]\nType=Application\nName=Tachyon\nComment=Keep Tachyon ready in the background\nExec={exec} --background\nNoDisplay=true\nX-GNOME-Autostart-enabled=true\n"
    )
}

/// The application's desktop entry: it shows Tachyon in launchers and "Open with" menus for
/// Markdown and text files. `StartupWMClass` matches the windows' app id, so the launcher and the
/// windows share one icon.
#[cfg(not(target_os = "macos"))]
fn application_desktop_file(exe: &std::path::Path) -> String {
    let exec = desktop_exec(exe);
    format!(
        "[Desktop Entry]\nType=Application\nName=Tachyon\nGenericName=Markdown Editor\nComment=Read and edit Markdown\nExec={exec} %F\nIcon=tachyon\nTerminal=false\nCategories=Utility;TextEditor;\nMimeType=text/markdown;text/x-markdown;text/plain;\nKeywords=markdown;notes;editor;scratchpad;\nStartupWMClass=tachyon\n"
    )
}

/// `$XDG_DATA_HOME` (default `~/.local/share`).
#[cfg(not(target_os = "macos"))]
fn data_home() -> io::Result<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "neither XDG_DATA_HOME nor HOME is set")
        })
}

/// The desktop entry and its icon, per user.
#[cfg(not(target_os = "macos"))]
fn desktop_entry_paths() -> io::Result<(PathBuf, PathBuf)> {
    let data = data_home()?;
    Ok((
        data.join("applications").join("tachyon.desktop"),
        data.join("icons/hicolor/scalable/apps/tachyon.svg"),
    ))
}

#[cfg(not(target_os = "macos"))]
pub fn desktop_entry_installed() -> io::Result<bool> {
    Ok(desktop_entry_paths()?.0.is_file())
}

#[cfg(not(target_os = "macos"))]
pub fn set_desktop_entry(exe: &std::path::Path, enabled: bool) -> io::Result<()> {
    let (entry, icon) = desktop_entry_paths()?;
    if !enabled {
        for path in [entry, icon] {
            match fs::remove_file(&path) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
                _ => {}
            }
        }
        return Ok(());
    }
    for (path, contents) in [
        (&icon, include_str!("../assets/tachyon.svg").to_owned()),
        (&entry, application_desktop_file(exe)),
    ] {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        fs::write(path, contents)?;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn desktop_entry_installed() -> io::Result<bool> {
    Ok(false)
}

#[cfg(target_os = "macos")]
pub fn set_desktop_entry(_exe: &std::path::Path, _enabled: bool) -> io::Result<()> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "desktop entries are a Linux and BSD feature"))
}

/// `/etc/os-release`'s `PRETTY_NAME`, for the About window's Environment table ("Platform"):
/// present on every systemd-based distribution and read by every other tool that reports the
/// running distribution, so it needs no per-distribution special-casing. "Linux" if the file is
/// missing or has no such line (a non-systemd distribution, or a minimal container image).
#[cfg(not(target_os = "macos"))]
pub fn os_version() -> String {
    fs::read_to_string("/etc/os-release")
        .ok()
        .as_deref()
        .and_then(pretty_name)
        .unwrap_or_else(|| "Linux".to_owned())
}

/// The unquoted value of an `/etc/os-release` `PRETTY_NAME=` line, split out so the quoting rule
/// is unit-tested without a real `/etc/os-release`.
#[cfg(not(target_os = "macos"))]
fn pretty_name(os_release: &str) -> Option<String> {
    let value = os_release.lines().find_map(|line| line.strip_prefix("PRETTY_NAME="))?;
    Some(value.trim_matches('"').to_owned())
}

/// macOS has no `/etc/os-release`; nothing else here reads a system version file yet.
#[cfg(target_os = "macos")]
pub fn os_version() -> String {
    "macOS".to_owned()
}

/// MSIX packaging is Windows-only (see `packaging/msix/AppxManifest.xml`); Linux, BSD and macOS
/// builds are always the plain `.tar.gz` archive.
pub fn packaged_version() -> Option<String> {
    None
}

/// See [`crate::update_jump_list`]. No Linux, BSD or macOS equivalent yet: [`note_recently_used`]
/// is this platform's own recent-files surface instead.
pub fn update_jump_list(_recent: &[PathBuf]) {}

/// See [`crate::note_recently_used`]: upserts `file` into the freedesktop recently-used list.
#[cfg(not(target_os = "macos"))]
pub fn note_recently_used(file: &std::path::Path) {
    let _ = recently_used::add(file);
}

/// No freedesktop recently-used list, or equivalent, on macOS yet.
#[cfg(target_os = "macos")]
pub fn note_recently_used(_file: &std::path::Path) {}

/// No native "always on top" call here yet: see [`crate::set_always_on_top`].
pub fn set_always_on_top(_window: &impl raw_window_handle::HasWindowHandle, _on: bool) -> bool {
    false
}

/// See [`crate::resize_without_activating`]: not needed here (no hidden ready window).
pub fn resize_without_activating(
    _window: &impl raw_window_handle::HasWindowHandle,
    _width: i32,
    _height: i32,
) -> bool {
    false
}

/// See [`crate::set_window_opacity`]: nothing at the OS level here.
pub fn set_window_opacity(
    _window: &impl raw_window_handle::HasWindowHandle,
    _opacity: f32,
) -> bool {
    false
}

/// No global-hotkey API here yet: Linux compositors bind `tachyon --note` directly (the
/// README's Hyprland example), and macOS has no binding surface yet either. See
/// [`crate::register_global_hotkey`].
pub enum GlobalHotkey {}

pub fn register_global_hotkey(
    _hotkey: crate::Hotkey,
    _on_press: Box<dyn Fn() + Send + 'static>,
) -> Result<GlobalHotkey, crate::HotkeyError> {
    Err(crate::HotkeyError::Unsupported)
}

/// `$HOME`, as a path; `None` if unset or empty.
fn env_home() -> Option<PathBuf> {
    std::env::var_os("HOME").filter(|value| !value.is_empty()).map(PathBuf::from)
}

/// See [`crate::documents_dir`]: `$HOME/Documents`. macOS has no `user-dirs.dirs` equivalent.
#[cfg(target_os = "macos")]
pub fn documents_dir() -> Option<PathBuf> {
    env_home().map(|home| home.join("Documents"))
}

/// See [`crate::documents_dir`]: `user-dirs.dirs`' `XDG_DOCUMENTS_DIR`, or `$HOME/Documents` if
/// that file is missing, unreadable, or has no such line.
#[cfg(not(target_os = "macos"))]
pub fn documents_dir() -> Option<PathBuf> {
    let home = env_home()?;
    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));
    let contents = fs::read_to_string(config_home.join("user-dirs.dirs")).ok();
    let dir = contents.as_deref().and_then(|contents| parse_user_dirs_documents(contents, &home));
    Some(dir.unwrap_or_else(|| home.join("Documents")))
}

/// The unquoted, `$HOME`-expanded value of a `user-dirs.dirs` `XDG_DOCUMENTS_DIR="..."` line
/// (`xdg-user-dirs-update`'s own format: double-quoted, with a literal `$HOME` token as the only
/// substitution it ever writes), split out so the exact quoting is unit-tested without a real
/// config file. Comment lines and any other key are ignored; the first matching line wins, as
/// `xdg-user-dirs-update` never writes more than one.
#[cfg(not(target_os = "macos"))]
fn parse_user_dirs_documents(contents: &str, home: &std::path::Path) -> Option<PathBuf> {
    for line in contents.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("XDG_DOCUMENTS_DIR") else { continue };
        let Some(rest) = rest.trim_start().strip_prefix('=') else { continue };
        let Some(quoted) = rest.trim().strip_prefix('"').and_then(|s| s.strip_suffix('"')) else {
            continue;
        };
        return Some(match quoted.strip_prefix("$HOME") {
            Some(suffix) => {
                let suffix = suffix.strip_prefix('/').unwrap_or(suffix);
                if suffix.is_empty() { home.to_path_buf() } else { home.join(suffix) }
            }
            None => PathBuf::from(quoted),
        });
    }
    None
}

#[cfg(all(test, not(target_os = "macos")))]
mod autostart_tests {
    use super::*;

    #[test]
    fn the_application_entry_opens_files_with_the_quoted_executable() {
        let entry = application_desktop_file(std::path::Path::new("/opt/My Apps/tachyon"));
        assert!(entry.contains("\nExec=\"/opt/My Apps/tachyon\" %F\n"));
        assert!(entry.contains("\nIcon=tachyon\n"));
        assert!(entry.contains("\nStartupWMClass=tachyon\n"));
        assert!(entry.contains("MimeType=text/markdown;"));
    }

    #[test]
    fn hidden_is_read_from_the_desktop_entry_group_only() {
        assert!(is_hidden("[Desktop Entry]\nName=Tachyon\nHidden=true\n"));
        assert!(!is_hidden("[Desktop Entry]\nHidden=false\n"));
        assert!(!is_hidden(&autostart_desktop_file(std::path::Path::new("/t"))));
        assert!(!is_hidden("[Desktop Entry]\nName=T\n[Desktop Action x]\nHidden=true\n"));
    }

    #[test]
    fn desktop_entry_quotes_the_executable_path() {
        let plain = autostart_desktop_file(std::path::Path::new("/opt/tachyon/tachyon"));
        assert!(plain.contains("\nExec=\"/opt/tachyon/tachyon\" --background\n"), "{plain}");
        let odd = autostart_desktop_file(std::path::Path::new("/home/a b/100%/$x\"y"));
        assert!(odd.contains("Exec=\"/home/a b/100%%/\\\\$x\\\\\"y\" --background"), "{odd}");
    }

    #[test]
    fn pretty_name_is_read_and_unquoted_or_absent() {
        assert_eq!(
            pretty_name("NAME=Ubuntu\nPRETTY_NAME=\"Ubuntu 24.04.1 LTS\"\nID=ubuntu\n"),
            Some("Ubuntu 24.04.1 LTS".to_owned())
        );
        assert_eq!(pretty_name("NAME=Ubuntu\nID=ubuntu\n"), None);
    }
}

#[cfg(all(test, not(target_os = "macos")))]
mod documents_dir_tests {
    use super::*;

    #[test]
    fn xdg_documents_dir_expands_home_in_a_quoted_line() {
        let home = std::path::Path::new("/home/alice");
        let contents = "XDG_DESKTOP_DIR=\"$HOME/Desktop\"\nXDG_DOCUMENTS_DIR=\"$HOME/Documents\"\n";
        assert_eq!(parse_user_dirs_documents(contents, home), Some(home.join("Documents")));
    }

    #[test]
    fn xdg_documents_dir_ignores_a_commented_out_line() {
        let home = std::path::Path::new("/home/alice");
        let contents = "# XDG_DOCUMENTS_DIR=\"$HOME/Nope\"\n";
        assert_eq!(parse_user_dirs_documents(contents, home), None);
    }

    #[test]
    fn xdg_documents_dir_is_none_when_the_key_is_missing() {
        let home = std::path::Path::new("/home/alice");
        let contents = "XDG_DESKTOP_DIR=\"$HOME/Desktop\"\n";
        assert_eq!(parse_user_dirs_documents(contents, home), None);
    }

    #[test]
    fn xdg_documents_dir_accepts_an_absolute_path_without_home() {
        let home = std::path::Path::new("/home/alice");
        let contents = "XDG_DOCUMENTS_DIR=\"/mnt/docs\"\n";
        assert_eq!(parse_user_dirs_documents(contents, home), Some(PathBuf::from("/mnt/docs")));
    }
}
