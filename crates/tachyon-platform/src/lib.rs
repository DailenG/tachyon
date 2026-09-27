//! OS integration that GPUI does not provide. This crate and the binary's
//! `main.rs` are the only places allowed to contain `#[cfg(target_os)]` code.
//!
//! Each backend module exposes the same free functions and types; the active
//! one is selected at compile time, so there is no dynamic dispatch.

mod protocol;

#[cfg(unix)]
mod unix;
#[cfg(unix)]
use unix as imp;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as imp;

use std::io;
use std::thread::JoinHandle;

/// Turns off the compositor's open and close animations for `window` (DWM on Windows; a no-op
/// elsewhere). Returns whether it was applied.
pub fn disable_window_transitions(window: &impl raw_window_handle::HasWindowHandle) -> bool {
    imp::disable_window_transitions(window)
}

/// Whether the primary instance stays running after its last window closes unless told otherwise
/// (docs/adr/0004). On by default where resident launches have been measured and a terminal is
/// not tied to the process: Windows release builds are GUI-subsystem executables. On Linux and
/// macOS a resident process started from a shell would keep that shell busy; there it is opt-in
/// (`--resident`, or autostart with `--background`).
pub fn resident_by_default() -> bool {
    cfg!(target_os = "windows")
}

/// Gives `window` Tachyon's icon (title bar, taskbar, Alt+Tab) on Windows, where GPUI looks for
/// an icon resource Tachyon's executable does not have. A no-op elsewhere.
pub fn set_window_icon(window: &impl raw_window_handle::HasWindowHandle) {
    imp::set_window_icon(window);
}

/// What the tray icon asks the application for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayEvent {
    /// Clicking the icon, or its "New window" menu item.
    Open,
    /// The "Quit Tachyon" menu item.
    Quit,
}

/// A resident instance's notification-area icon (Windows): clicking it opens a window, its menu
/// opens a window or quits. Removed when dropped.
pub struct Tray {
    _inner: imp::Tray,
}

impl Tray {
    /// Shows the icon with `tooltip`. `on_event` runs on the tray's own thread. `None` where
    /// there is no tray support (Linux and macOS for now) or the shell refused the icon.
    pub fn show(tooltip: &str, on_event: impl Fn(TrayEvent) + Send + 'static) -> Option<Self> {
        imp::show_tray(tooltip, on_event).map(|inner| Tray { _inner: inner })
    }
}

/// Lets command-line output (help, status) reach the console the program was started from.
/// Windows GUI-subsystem executables get no console of their own; elsewhere a no-op.
pub fn attach_parent_console() {
    imp::attach_parent_console();
}

/// Starts asking whether the system prefers a dark appearance, where GPUI only learns it after its
/// first windows exist: on Linux and BSD it asks the desktop portal asynchronously, so its first
/// windows report light. There this spawns a thread for the D-Bus round trip (about 1 ms warm; a
/// portal that has to be started takes far longer) and the receiver gets the answer, if any.
/// `None` elsewhere: a window's appearance is right from the start.
pub fn query_system_appearance() -> Option<std::sync::mpsc::Receiver<bool>> {
    imp::query_system_appearance()
}

/// Whether Tachyon starts in the background at login.
pub fn autostart_enabled() -> io::Result<bool> {
    imp::autostart_enabled()
}

/// Makes `exe --background` start at login (per user), or stops it. Windows: the `Run` key of the
/// current user; Linux and BSD: an XDG autostart entry. Not supported on macOS yet.
pub fn set_autostart(exe: &std::path::Path, enabled: bool) -> io::Result<()> {
    imp::set_autostart(exe, enabled)
}

/// Monospace font families to try for code, most preferred first. The first
/// entry ships with the OS on Windows and macOS; Linux has no universal one,
/// so callers should pick the first installed family.
pub fn monospace_font_candidates() -> &'static [&'static str] {
    if cfg!(target_os = "windows") {
        &["Cascadia Mono", "Consolas", "Courier New"]
    } else if cfg!(target_os = "macos") {
        &["SF Mono", "Menlo", "Monaco"]
    } else {
        &[
            "JetBrains Mono",
            "DejaVu Sans Mono",
            "Noto Sans Mono",
            "Liberation Mono",
            "Ubuntu Mono",
            "Adwaita Mono",
            "Cascadia Mono",
        ]
    }
}

/// Font families to try for body text, most preferred first; empty where GPUI's system font has
/// every weight and style. On Linux GPUI asks for IBM Plex Sans, which it does not bundle, and its
/// fallback fonts come in the regular face only, so without an installed family from this list
/// bold and italic text would render regular.
pub fn text_font_candidates() -> &'static [&'static str] {
    if cfg!(any(target_os = "windows", target_os = "macos")) {
        &[]
    } else {
        &[
            "IBM Plex Sans",
            "Noto Sans",
            "Ubuntu",
            "Cantarell",
            "DejaVu Sans",
            "Liberation Sans",
            "Adwaita Sans",
        ]
    }
}

/// Whether GPUI shows native dialogs for prompts on this OS. Where it does
/// not (Linux, BSD), GPUI's in-window fallback is mouse-only.
pub fn has_native_prompts() -> bool {
    cfg!(any(target_os = "windows", target_os = "macos"))
}

/// Whether a window opened with `show: false` stays hidden until activated.
/// Wayland compositors map it anyway, so a resident instance keeps no ready
/// window there. Only enabled where it has been measured (docs/adr/0004).
pub fn keeps_hidden_windows_hidden() -> bool {
    cfg!(target_os = "windows")
}

/// Outcome of claiming the per-user, per-session application instance.
pub enum Instance {
    /// This process owns the instance and receives forwarded launches.
    Primary(Listener),
    /// Another process owns the instance; forward the launch to it and exit.
    Secondary(Client),
}

impl Instance {
    /// Claims the instance named `app_id`. Exactly one concurrent caller per
    /// user session observes [`Instance::Primary`].
    pub fn acquire(app_id: &str) -> io::Result<Self> {
        Ok(match imp::acquire(app_id)? {
            imp::Acquired::Primary(inner) => Instance::Primary(Listener { inner }),
            imp::Acquired::Secondary(inner) => Instance::Secondary(Client { inner }),
        })
    }
}

/// Server side of the instance channel. Must stay alive for the lifetime of
/// the primary process; dropping it releases the instance.
pub struct Listener {
    inner: imp::Listener,
}

impl Listener {
    /// Serves forwarded launches on a dedicated thread. `on_message` receives
    /// each launch's argument list in order and returns whether it accepted
    /// (e.g. queued) the launch; only accepted launches are acknowledged, so
    /// the sender starts standalone otherwise. Malformed messages are dropped.
    pub fn spawn(
        self,
        on_message: impl FnMut(Vec<String>) -> bool + Send + 'static,
    ) -> JoinHandle<()> {
        let inner = self.inner;
        std::thread::Builder::new()
            .name("instance-listener".into())
            .spawn(move || imp::serve(inner, on_message))
            .expect("failed to spawn instance listener thread")
    }
}

/// Client side of the instance channel.
pub struct Client {
    inner: imp::Client,
}

impl Client {
    /// Forwards `args` to the primary instance and allows it to take the
    /// foreground.
    pub fn send(self, args: &[String]) -> io::Result<()> {
        imp::send(self.inner, &protocol::encode(args))
    }
}
