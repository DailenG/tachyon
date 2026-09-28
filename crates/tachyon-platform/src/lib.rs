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

/// Sets whether `window`'s native title bar renders in dark or light colours, to match Tachyon's
/// resolved theme (`Theme::for_window` in `tachyon-editor`) instead of the OS dark-mode setting
/// DWM would otherwise follow: Tachyon keeps the native title bar rather than drawing its own, so
/// without this a window whose theme differs from the system's would show a mismatched bar
/// (Windows: `DWMWA_USE_IMMERSIVE_DARK_MODE`). Returns whether it was applied; a no-op elsewhere.
pub fn set_title_bar_dark(window: &impl raw_window_handle::HasWindowHandle, dark: bool) -> bool {
    imp::set_title_bar_dark(window, dark)
}

/// Sets whether popup menus shown after this call follow Tachyon's resolved theme (the same
/// `dark` given to [`set_title_bar_dark`]) instead of the OS dark-mode setting. Windows: the tray
/// icon's context menu, the only popup menu Tachyon has so far, applied lazily - the next time a
/// menu is actually shown, not by this call itself - through an undocumented `uxtheme.dll`
/// mode-switch (`SetPreferredAppMode` / `FlushMenuThemes`, ordinals 135 and 136), the same one
/// Windows Terminal and Notepad++ use, since `TrackPopupMenuEx` has no documented way to ask for
/// a dark menu. A no-op before Windows 10 1903 (build 18362) and everywhere else.
pub fn set_popup_menu_dark(dark: bool) {
    imp::set_popup_menu_dark(dark);
}

/// Whether the primary instance stays running after its last window closes unless told otherwise
/// (docs/adr/0004). On by default where resident launches have been measured and a terminal is
/// not tied to the process: Windows release builds are GUI-subsystem executables. On Linux and
/// macOS a resident process started from a shell would keep that shell busy; there it is opt-in
/// (`--resident`, or autostart with `--background`).
pub fn resident_by_default() -> bool {
    cfg!(target_os = "windows")
}

/// A function that reads the clipboard's text and may run on any thread (Windows), so a paste does
/// not read it on the UI thread: GPUI's read there takes ≈ 12 ms for 5 MB. `None` where the
/// clipboard has to be read through GPUI.
pub fn clipboard_text_reader() -> Option<fn() -> Option<String>> {
    imp::clipboard_text_reader()
}

/// Puts `html` on the clipboard as rich text, with `text` as its plain-text form, for `window`'s
/// process (Windows: the `HTML Format` and `CF_UNICODETEXT` formats). Returns whether it did;
/// `false` where rich text is not supported yet (Linux and macOS: GPUI's clipboard is text only).
pub fn write_clipboard_html(
    window: &impl raw_window_handle::HasWindowHandle,
    html: &str,
    text: &str,
) -> bool {
    imp::write_clipboard_html(window, html, text)
}

/// Gives `window` Tachyon's icon (title bar, taskbar, Alt+Tab) on Windows at the sizes its
/// monitor's DPI asks for; GPUI only loads the executable's icon resource at the default size.
/// A no-op elsewhere.
pub fn set_window_icon(window: &impl raw_window_handle::HasWindowHandle) {
    imp::set_window_icon(window);
}

/// The command line an OS-triggered restart registers with the system ([`register_restart`]): no
/// leading executable path. `RegisterApplicationRestart`'s own documentation says never to
/// include it (the OS prepends it), so the relaunch looks exactly like a login autostart launch:
/// a windowless resident primary that reopens hot exit's backups once a window is asked for
/// (`crate::instance_id`'s process, `crates/tachyon/src/main.rs`'s `--background` handling).
/// A plain `pub const`, not only inside the Windows-only implementation, so its shape is covered
/// by an ordinary unit test on every platform Tachyon builds for, not only when cross-compiling.
pub const RESTART_COMMAND_LINE: &str = "--background";

/// The flags [`register_restart`] passes to `RegisterApplicationRestart`: `RESTART_NO_CRASH`
/// (bit `0b0001`) and `RESTART_NO_HANG` (bit `0b0010`) opt out of the two restart reasons Windows
/// Error Reporting already shows its own "this program has stopped working" dialog for, which
/// itself offers to restart the app - stacking a silent, automatic restart on top of that dialog
/// would be confusing. Those two reasons are also the ones gated by the documented "system will
/// only restart the application if it has been running for a minimum of 60 seconds" rule (to
/// prevent a crash loop); a reboot or an application-level patch install (`RESTART_NO_PATCH`
/// `0b0100` and `RESTART_NO_REBOOT` `0b1000`, both left clear) are a different code path with no
/// such minimum, no confirmation dialog, and no 60 s wait - exactly what a Windows Update restart
/// or the "restart my apps when I sign back in" setting needs, so a silent restart there is
/// exactly what `RegisterApplicationRestart` documents itself for:
/// <https://learn.microsoft.com/windows/win32/api/winbase/nf-winbase-registerapplicationrestart>,
/// <https://learn.microsoft.com/windows/win32/recovery/registering-for-application-restart>. The
/// registration itself is a plain Kernel32 export with no packaging requirement, so an MSIX build
/// needs nothing extra in its manifest for this call; ADR 0007's `packaged_version` distinction is
/// irrelevant here.
/// A plain `u32` (the same underlying type as the Windows-only `REGISTER_APPLICATION_RESTART_FLAGS`),
/// not only inside the Windows-only implementation, so the bits are covered by an ordinary unit
/// test on every platform.
pub const RESTART_FLAGS: u32 = 0b0011;

/// Registers this process to be relaunched by Windows after a reboot or an application-level
/// patch install, with [`RESTART_COMMAND_LINE`] and [`RESTART_FLAGS`], when the user has enabled
/// "Automatically save my restartable apps and restart them when I sign back in" (Settings >
/// Accounts > Sign-in options): the relaunch comes back as a windowless resident instance and hot
/// exit reopens whatever was unsaved. Tachyon calls this only for a resident primary instance,
/// tied to the `hot_exit` setting (restarting without it would bring the instance back with
/// nothing to reopen); see `tachyon_editor::RestartRegistration` and
/// `crates/tachyon/src/app.rs`'s call sites. A no-op everywhere but Windows, and best-effort
/// there: a failure just means Windows will not bring Tachyon back, never a crash or a startup
/// failure.
pub fn register_restart() {
    imp::register_restart();
}

/// Reverses [`register_restart`]: called when hot exit is turned off (settings save or the
/// command palette's toggle), so Windows stops trying to bring Tachyon back with nothing to
/// restore. A no-op everywhere but Windows.
pub fn unregister_restart() {
    imp::unregister_restart();
}

/// What the tray icon asks the application for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayEvent {
    /// Clicking the icon, or its "New window" menu item.
    Open,
    /// The "About Tachyon" menu item.
    About,
    /// The "Quit Tachyon" menu item.
    Quit,
    /// Windows is ending the session (logoff, sign-out, shutdown or restart:
    /// `WM_QUERYENDSESSION`/`WM_ENDSESSION`): write every open window's unsaved-document backup
    /// now, without closing anything, so hot exit is current even if the session ends before the
    /// next typing-pause backup would have written it. Harmless if the session end this followed
    /// is later cancelled by another application: Tachyon just keeps running with every window
    /// untouched.
    EndSession,
}

/// A resident instance's notification-area icon (Windows): clicking it opens a window, its menu
/// opens a window, shows the About window, or quits. Removed when dropped.
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

/// Per-user directory for state Tachyon keeps between runs (backups of unsaved documents):
/// `%LOCALAPPDATA%\Tachyon` on Windows, `~/Library/Application Support/Tachyon` on macOS,
/// `$XDG_STATE_HOME/tachyon` (default `~/.local/state/tachyon`) elsewhere. `None` if the
/// environment names no home.
pub fn state_dir() -> Option<std::path::PathBuf> {
    use std::path::PathBuf;
    let env = |name| std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from);
    if cfg!(target_os = "windows") {
        env("LOCALAPPDATA").map(|dir| dir.join("Tachyon"))
    } else if cfg!(target_os = "macos") {
        env("HOME").map(|home| home.join("Library/Application Support/Tachyon"))
    } else {
        env("XDG_STATE_HOME")
            .or_else(|| env("HOME").map(|home| home.join(".local/state")))
            .map(|dir| dir.join("tachyon"))
    }
}

/// Per-user directory for Tachyon's settings: `%APPDATA%\Tachyon` on Windows,
/// `~/Library/Application Support/Tachyon` on macOS, `$XDG_CONFIG_HOME/tachyon` (default
/// `~/.config/tachyon`) elsewhere. `None` if the environment names no home.
pub fn config_dir() -> Option<std::path::PathBuf> {
    use std::path::PathBuf;
    let env = |name| std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from);
    if cfg!(target_os = "windows") {
        env("APPDATA").map(|dir| dir.join("Tachyon"))
    } else if cfg!(target_os = "macos") {
        env("HOME").map(|home| home.join("Library/Application Support/Tachyon"))
    } else {
        env("XDG_CONFIG_HOME")
            .or_else(|| env("HOME").map(|home| home.join(".config")))
            .map(|dir| dir.join("tachyon"))
    }
}

/// Whether the per-user desktop entry (launcher and "Open with" item) is installed.
pub fn desktop_entry_installed() -> io::Result<bool> {
    imp::desktop_entry_installed()
}

/// Installs or removes the per-user desktop entry for `exe` with Tachyon's icon (Linux and BSD:
/// `$XDG_DATA_HOME/applications/tachyon.desktop` and the hicolor icon theme). Unsupported
/// elsewhere.
pub fn set_desktop_entry(exe: &std::path::Path, enabled: bool) -> io::Result<()> {
    imp::set_desktop_entry(exe, enabled)
}

/// Rebuilds the taskbar/Start jump list's "Recent" category (and its "New window" task) from
/// `recent` (`tachyon_editor::RecentFiles`'s own list, newest first): call this whenever that
/// list changes (a file is opened, saved, or a backup restored). Returns immediately; the rebuild
/// happens off the caller's thread, coalesced against any rebuild already queued (Windows: see
/// `windows::jump_list`'s module doc comment). No-op on Linux, BSD and macOS, which have no
/// equivalent surface yet.
pub fn update_jump_list(recent: &[std::path::PathBuf]) {
    imp::update_jump_list(recent);
}

/// Adds `file` to the freedesktop recently-used list (`~/.local/share/recently-used.xbel`),
/// which GTK and Qt file choosers read as "Recent" (Linux and BSD only). Call this alongside
/// [`update_jump_list`], whenever a file is opened or saved. No-op on Windows (its own recent
/// surface is [`update_jump_list`]) and macOS (no equivalent yet).
pub fn note_recently_used(file: &std::path::Path) {
    imp::note_recently_used(file);
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

/// A human-readable OS name and version for the About window's Environment table ("Platform").
/// Windows: the name plus build number (`RtlGetVersion`, unaffected by the executable's
/// manifest). Linux and BSD: `/etc/os-release`'s `PRETTY_NAME`, or "Linux" if it cannot be read.
/// A fixed string on macOS, where nothing else here reads a system version file yet.
pub fn os_version() -> String {
    imp::os_version()
}

/// The installed MSIX package's full version ("X.Y.Z.B"), for the About window's Environment
/// table, if this process runs packaged (Windows only; see `packaging/msix/AppxManifest.xml`
/// and ADR 0007). `None` elsewhere, or when unpackaged (the `.zip`/`.tar.gz` builds).
pub fn packaged_version() -> Option<String> {
    imp::packaged_version()
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression for the exact bit pattern, not merely `RESTART_FLAGS`'s own defining
    /// expression: a typo (`&` for `|`, or the wrong constant) would silently change which
    /// restart reasons Tachyon opts out of, and only a literal expected value catches that.
    #[test]
    fn restart_flags_allow_reboot_and_patch_but_not_crash_or_hang() {
        const RESTART_NO_CRASH: u32 = 0b0001;
        const RESTART_NO_HANG: u32 = 0b0010;
        const RESTART_NO_PATCH: u32 = 0b0100;
        const RESTART_NO_REBOOT: u32 = 0b1000;
        assert_eq!(RESTART_FLAGS & RESTART_NO_CRASH, RESTART_NO_CRASH);
        assert_eq!(RESTART_FLAGS & RESTART_NO_HANG, RESTART_NO_HANG);
        assert_eq!(RESTART_FLAGS & RESTART_NO_PATCH, 0, "a patch install must still restart it");
        assert_eq!(RESTART_FLAGS & RESTART_NO_REBOOT, 0, "a reboot must still restart it");
    }
}
