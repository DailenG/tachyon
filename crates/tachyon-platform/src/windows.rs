//! Windows: the primary is whoever creates the first instance of a named pipe
//! (`FILE_FLAG_FIRST_PIPE_INSTANCE` makes the election atomic). The pipe name
//! carries the session id and user name, so RDP sessions and fast user
//! switching each get their own primary.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::{Mutex, PoisonError, mpsc};
use std::time::{Duration, Instant};

use windows_sys::Wdk::System::SystemServices::RtlGetVersion;
use windows_sys::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_INSUFFICIENT_BUFFER, ERROR_PIPE_BUSY,
    ERROR_PIPE_CONNECTED, HWND, INVALID_HANDLE_VALUE, LocalFree,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX};
use windows_sys::Win32::Storage::Packaging::Appx::GetCurrentPackageFullName;
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT, WaitNamedPipeW,
};
use windows_sys::Win32::System::Recovery::{
    RegisterApplicationRestart, UnregisterApplicationRestart,
};
use windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows_sys::Win32::System::SystemInformation::OSVERSIONINFOW;
use windows_sys::Win32::UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow};

use crate::protocol;

mod hotkey;
mod jump_list;
mod tray;
pub use hotkey::{GlobalHotkey, register_global_hotkey};
pub use tray::{Tray, set_popup_menu_dark, set_window_icon, show_tray};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const REPLY_TIMEOUT: Duration = Duration::from_secs(2);
const PIPE_BUFFER: u32 = 64 * 1024;

/// Protected DACL granting access only to the pipe's owner (the user who
/// started the primary) and SYSTEM. The default DACL lets every local user
/// open the pipe for reading, which is enough to occupy the listener.
const PIPE_SDDL: &str = "D:P(A;;GA;;;OW)(A;;GA;;;SY)";

/// Security descriptor allocated by `ConvertStringSecurityDescriptorToSecurityDescriptorW`.
struct SecurityDescriptor(PSECURITY_DESCRIPTOR);

impl SecurityDescriptor {
    fn owner_only() -> io::Result<Self> {
        let sddl = wide(PIPE_SDDL);
        let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
        // SAFETY: `sddl` is NUL-terminated UTF-16 and `descriptor` a valid
        // out-pointer; the size out-parameter is optional.
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(descriptor))
    }
}

impl Drop for SecurityDescriptor {
    fn drop(&mut self) {
        // SAFETY: allocated with LocalAlloc by the conversion above and freed
        // exactly once here.
        unsafe { LocalFree(self.0) };
    }
}

pub enum Acquired {
    Primary(Listener),
    Secondary(Client),
}

pub struct Listener {
    name: Vec<u16>,
    pipe: OwnedHandle,
}

pub struct Client {
    name: String,
}

fn pipe_name(app_id: &str) -> String {
    let mut session = 0u32;
    // SAFETY: valid out-pointer; on failure `session` stays 0, which only
    // widens the instance scope to all sessions of this user.
    unsafe { ProcessIdToSessionId(std::process::id(), &mut session) };
    let user: String = std::env::var("USERNAME")
        .unwrap_or_default()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!(r"\\.\pipe\{app_id}-{session}-{user}")
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn create_instance(name: &[u16], first: bool) -> io::Result<OwnedHandle> {
    let open_mode = PIPE_ACCESS_DUPLEX | if first { FILE_FLAG_FIRST_PIPE_INSTANCE } else { 0 };
    let pipe_mode = PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS;
    let descriptor = SecurityDescriptor::owner_only()?;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: 0,
    };
    // SAFETY: `name` is NUL-terminated UTF-16 and `attributes` points to a
    // valid descriptor that outlives the call.
    let handle = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            open_mode,
            pipe_mode,
            PIPE_UNLIMITED_INSTANCES,
            PIPE_BUFFER,
            PIPE_BUFFER,
            0,
            &attributes,
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `handle` is a freshly created, owned, valid handle.
    Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
}

pub fn acquire(app_id: &str) -> io::Result<Acquired> {
    let name = pipe_name(app_id);
    let wide_name = wide(&name);
    match create_instance(&wide_name, true) {
        Ok(pipe) => Ok(Acquired::Primary(Listener { name: wide_name, pipe })),
        Err(e) if e.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32) => {
            Ok(Acquired::Secondary(Client { name }))
        }
        Err(e) => Err(e),
    }
}

/// Serves one client at a time. Pipe reads have no timeout here, so a process
/// of the same user that connects and never writes stalls forwarding; later
/// launches then time out waiting for their reply and start standalone
/// (ADR 0003). Other users cannot connect at all (see [`PIPE_SDDL`]).
pub fn serve(listener: Listener, mut on_message: impl FnMut(Vec<String>) -> bool) {
    let Listener { name, mut pipe } = listener;
    loop {
        // SAFETY: `pipe` is a valid synchronous pipe handle; no OVERLAPPED.
        let connected = unsafe { ConnectNamedPipe(pipe.as_raw_handle(), ptr::null_mut()) } != 0
            || io::Error::last_os_error().raw_os_error() == Some(ERROR_PIPE_CONNECTED as i32);
        // Keep a listening instance available before handling this client so
        // concurrent secondaries never observe a missing pipe.
        let Ok(next) = create_instance(&name, false) else { return };
        let mut current = File::from(std::mem::replace(&mut pipe, next));
        if connected
            && let Ok(Some(args)) = protocol::read_request(&current)
            && on_message(args)
        {
            let _ = current.write_all(protocol::ACK);
        }
    }
}

pub fn send(client: Client, request: &[u8]) -> io::Result<()> {
    // The primary is not the foreground process; without this grant Windows'
    // foreground lock prevents it from raising its window.
    // SAFETY: plain Win32 call without pointers.
    unsafe { AllowSetForegroundWindow(ASFW_ANY) };
    // Synchronous pipe reads cannot time out, so the exchange runs on a helper
    // thread. On timeout the thread is abandoned; the process goes on to start
    // standalone and the thread ends with it.
    let request = request.to_vec();
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new().name("instance-send".into()).spawn(move || {
        let _ = tx.send(exchange(&client.name, &request));
    })?;
    rx.recv_timeout(CONNECT_TIMEOUT + REPLY_TIMEOUT).unwrap_or_else(|_| {
        Err(io::Error::new(io::ErrorKind::TimedOut, "primary instance did not reply"))
    })
}

fn exchange(name: &str, request: &[u8]) -> io::Result<()> {
    let deadline = Instant::now() + CONNECT_TIMEOUT;
    let pipe = loop {
        match OpenOptions::new().read(true).write(true).open(name) {
            Ok(pipe) => break pipe,
            Err(e) if Instant::now() < deadline => match e.raw_os_error() {
                Some(code) if code == ERROR_PIPE_BUSY as i32 => {
                    let wide_name = wide(name);
                    // SAFETY: NUL-terminated UTF-16 name.
                    unsafe { WaitNamedPipeW(wide_name.as_ptr(), 100) };
                }
                Some(code) if code == ERROR_FILE_NOT_FOUND as i32 => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                _ => return Err(e),
            },
            Err(e) => return Err(e),
        }
    };
    protocol::send_request(&pipe, request)
}

pub fn disable_window_transitions(window: &impl raw_window_handle::HasWindowHandle) -> bool {
    use windows_sys::Win32::Graphics::Dwm::{
        DWMWA_TRANSITIONS_FORCEDISABLED, DwmSetWindowAttribute,
    };
    let Ok(handle) = window.window_handle() else { return false };
    let raw_window_handle::RawWindowHandle::Win32(win32) = handle.as_raw() else { return false };
    let disable: i32 = 1;
    // SAFETY: `win32.hwnd` is the live window handle GPUI just returned, and the attribute value
    // points at a 4-byte BOOL that outlives the call, as DWMWA_TRANSITIONS_FORCEDISABLED requires.
    let result = unsafe {
        DwmSetWindowAttribute(
            win32.hwnd.get() as _,
            DWMWA_TRANSITIONS_FORCEDISABLED as u32,
            (&raw const disable).cast(),
            size_of::<i32>() as u32,
        )
    };
    result == 0
}

/// Sets whether `window`'s native title bar (and its system menu, buttons and border) render
/// with light-on-dark colours (`DWMWA_USE_IMMERSIVE_DARK_MODE`). Tachyon keeps the OS title bar
/// rather than drawing its own, but DWM otherwise paints it for the *system's* dark-mode setting;
/// without this call a Tachyon window whose theme (Settings, not the OS) is light would show a
/// dark bar above a light canvas, or the reverse. Returns whether it was applied.
pub fn set_title_bar_dark(window: &impl raw_window_handle::HasWindowHandle, dark: bool) -> bool {
    use windows_sys::Win32::Graphics::Dwm::{DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute};
    let Ok(handle) = window.window_handle() else { return false };
    let raw_window_handle::RawWindowHandle::Win32(win32) = handle.as_raw() else { return false };
    let value: i32 = i32::from(dark);
    // SAFETY: `win32.hwnd` is the live window handle GPUI just returned, and the attribute value
    // points at a 4-byte BOOL that outlives the call, as DWMWA_USE_IMMERSIVE_DARK_MODE requires.
    let result = unsafe {
        DwmSetWindowAttribute(
            win32.hwnd.get() as _,
            DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
            (&raw const value).cast(),
            size_of::<i32>() as u32,
        )
    };
    result == 0
}

/// See [`crate::set_always_on_top`].
pub fn set_always_on_top(window: &impl raw_window_handle::HasWindowHandle, on: bool) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        HWND_NOTOPMOST, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowPos,
    };
    let Ok(handle) = window.window_handle() else { return false };
    let raw_window_handle::RawWindowHandle::Win32(win32) = handle.as_raw() else { return false };
    let insert_after = if on { HWND_TOPMOST } else { HWND_NOTOPMOST };
    let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE;
    // SAFETY: `win32.hwnd` is the live window handle GPUI just returned; `SWP_NOMOVE |
    // SWP_NOSIZE` makes the ignored position and size arguments harmless.
    let result = unsafe { SetWindowPos(win32.hwnd.get() as _, insert_after, 0, 0, 0, 0, flags) };
    result != 0
}

/// See [`crate::set_window_opacity`].
pub fn set_window_opacity(window: &impl raw_window_handle::HasWindowHandle, opacity: f32) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GWL_EXSTYLE, GetWindowLongPtrW, LWA_ALPHA, SetLayeredWindowAttributes, SetWindowLongPtrW,
        WS_EX_LAYERED,
    };
    let Ok(handle) = window.window_handle() else { return false };
    let raw_window_handle::RawWindowHandle::Win32(win32) = handle.as_raw() else { return false };
    let hwnd = win32.hwnd.get() as _;
    let alpha = crate::opacity_to_alpha(opacity);
    // SAFETY: `hwnd` is the live window handle GPUI just returned; reading its extended style has
    // no other effect.
    let style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) };
    let layered = WS_EX_LAYERED as isize;
    if alpha == 255 {
        if style & layered != 0 {
            // SAFETY: as above; only the layered bit is cleared, the rest of the style kept.
            unsafe { SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style & !layered) };
        }
        return true;
    }
    if style & layered == 0 {
        // SAFETY: as above; only the layered bit is added, the rest of the style kept.
        unsafe { SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style | layered) };
    }
    // SAFETY: `hwnd` is live and now layered, which `SetLayeredWindowAttributes` requires;
    // `LWA_ALPHA` uses only the alpha argument.
    let result = unsafe { SetLayeredWindowAttributes(hwnd, 0, alpha, LWA_ALPHA) };
    result != 0
}

/// See [`crate::register_restart`].
pub fn register_restart() {
    let command_line = wide(crate::RESTART_COMMAND_LINE);
    // SAFETY: `command_line` is a NUL-terminated wide string kept alive for the whole call;
    // `crate::RESTART_FLAGS` is a valid combination of the documented `dwFlags` bits. The
    // `HRESULT` result is ignored, matching how other optional Windows integrations here degrade
    // (the jump list, `set_title_bar_dark`): failing to register only means Windows will not
    // bring Tachyon back, never a crash or a startup failure.
    let _ = unsafe { RegisterApplicationRestart(command_line.as_ptr(), crate::RESTART_FLAGS) };
}

/// See [`crate::unregister_restart`].
pub fn unregister_restart() {
    // SAFETY: takes no arguments; safe to call even when nothing is currently registered.
    let _ = unsafe { UnregisterApplicationRestart() };
}

pub fn desktop_entry_installed() -> io::Result<bool> {
    Ok(false)
}

pub fn set_desktop_entry(_exe: &Path, _enabled: bool) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "desktop entries are a Linux and BSD feature; on Windows pin tachyon.exe to Start",
    ))
}

/// See [`crate::update_jump_list`]: rebuilds the taskbar/Start jump list off the UI thread,
/// coalesced (see `jump_list`'s module doc comment).
pub fn update_jump_list(recent: &[PathBuf]) {
    jump_list::update(recent);
}

/// See [`crate::note_recently_used`]. No Windows equivalent: the taskbar/Start jump list is
/// driven by [`update_jump_list`] from the whole recent list instead of one file at a time.
pub fn note_recently_used(_file: &Path) {}

pub fn clipboard_text_reader() -> Option<fn() -> Option<String>> {
    Some(read_clipboard_text)
}

/// `CF_UNICODETEXT`: Windows provides it for any text on the clipboard, converting from the ANSI
/// formats if needed.
const CF_UNICODETEXT: u32 = 13;

/// How long [`open_clipboard`] retries `OpenClipboard` while another task holds it.
const OPEN_CLIPBOARD_DEADLINE: Duration = Duration::from_millis(50);

/// Serializes every clipboard access this module makes (`read_clipboard_text`,
/// `write_clipboard_html`), so at most one of *this process's* threads is ever between
/// `OpenClipboard` and `CloseClipboard` at a time. `CloseClipboard` on one thread can invalidate
/// the `GlobalLock`ed memory a `GlobalLock` on another thread is still reading, which is what
/// crashed `tachyon.exe` (the Windows crash dumps that followed #47): `tachyon_editor`'s paste path already
/// avoids that by construction (`PasteText::Reading`'s claim lets only one thread read the
/// clipboard per paste), so this lock is defence in depth against any other caller added later.
/// It cannot serialize against GPUI's own clipboard calls (`cx.read_from_clipboard`,
/// `cx.write_to_clipboard`), which go through GPUI's platform layer, not this module.
static CLIPBOARD_LOCK: Mutex<()> = Mutex::new(());

/// Opens the clipboard for `owner` (`ptr::null_mut()` opens it for this task rather than a
/// window), retrying while another task holds it. The retries are bounded by an [`Instant`]
/// deadline, not by an attempt count: Windows only guarantees that `thread::sleep(1ms)` rounds up
/// to the scheduler's timer granularity, which is commonly around 15.6 ms without
/// `timeBeginPeriod`, so counting sleeps (as this used to) does not bound wall-clock time the way
/// its old doc comment claimed.
fn open_clipboard(owner: HWND) -> bool {
    use windows_sys::Win32::System::DataExchange::OpenClipboard;

    let deadline = Instant::now() + OPEN_CLIPBOARD_DEADLINE;
    loop {
        // SAFETY: `owner` is either null (opens for this task) or a live window handle.
        if unsafe { OpenClipboard(owner) != 0 } {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Reads the clipboard's text. The clipboard can be opened from any thread; while another task
/// holds it, this retries for up to [`OPEN_CLIPBOARD_DEADLINE`]. Held under [`CLIPBOARD_LOCK`]
/// for its whole `OpenClipboard`..`CloseClipboard` span: see that constant's docs for why.
fn read_clipboard_text() -> Option<String> {
    use windows_sys::Win32::System::DataExchange::{CloseClipboard, GetClipboardData};
    use windows_sys::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};

    struct Open;
    impl Drop for Open {
        fn drop(&mut self) {
            // SAFETY: only constructed after OpenClipboard succeeded on this thread.
            unsafe { CloseClipboard() };
        }
    }

    let _guard = CLIPBOARD_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    if !open_clipboard(ptr::null_mut()) {
        return None;
    }
    let _open = Open;
    // SAFETY: the clipboard is open; the handle stays owned by the clipboard.
    let handle = unsafe { GetClipboardData(CF_UNICODETEXT) };
    if handle.is_null() {
        return None;
    }
    // SAFETY: `handle` is a global memory object holding the text.
    let data = unsafe { GlobalLock(handle) } as *const u16;
    if data.is_null() {
        return None;
    }
    // SAFETY: as above.
    let units = unsafe { GlobalSize(handle) } / size_of::<u16>();
    if units == 0 {
        // SAFETY: balances the GlobalLock above; nothing to read.
        unsafe { GlobalUnlock(handle) };
        return None;
    }
    // SAFETY: `data` is locked and points at `units` UTF-16 code units until GlobalUnlock.
    let slice = unsafe { std::slice::from_raw_parts(data, units) };
    let len = slice.iter().position(|&unit| unit == 0).unwrap_or(units);
    let text = String::from_utf16_lossy(&slice[..len]);
    // SAFETY: balances the GlobalLock above; `slice` is not used after this.
    unsafe { GlobalUnlock(handle) };
    Some(text)
}

/// The `HTML Format` clipboard payload for `fragment`: a header of byte offsets (UTF-8), then the
/// fragment wrapped in a minimal document with the fragment markers Word and browsers look for.
fn cf_html(fragment: &str) -> String {
    const PREFIX: &str = "<html><body>\r\n<!--StartFragment-->";
    const SUFFIX: &str = "<!--EndFragment-->\r\n</body></html>";
    let header = |start_html: usize, end_html: usize, start: usize, end: usize| {
        format!(
            "Version:0.9\r\nStartHTML:{start_html:010}\r\nEndHTML:{end_html:010}\r\n\
             StartFragment:{start:010}\r\nEndFragment:{end:010}\r\n"
        )
    };
    let start_html = header(0, 0, 0, 0).len();
    let start = start_html + PREFIX.len();
    let end = start + fragment.len();
    let end_html = end + SUFFIX.len();
    format!("{}{PREFIX}{fragment}{SUFFIX}", header(start_html, end_html, start, end))
}

pub fn write_clipboard_html(
    window: &impl raw_window_handle::HasWindowHandle,
    html: &str,
    text: &str,
) -> bool {
    use windows_sys::Win32::Foundation::GlobalFree;
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, RegisterClipboardFormatW, SetClipboardData,
    };
    use windows_sys::Win32::System::Memory::{
        GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock,
    };

    /// Copies `bytes` into a movable global block and hands it to the clipboard.
    fn set(format: u32, bytes: &[u8]) -> bool {
        // SAFETY: allocates a block the clipboard takes ownership of on success.
        let block = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes.len()) };
        if block.is_null() {
            return false;
        }
        // SAFETY: `block` was just allocated with room for `bytes`.
        let data = unsafe { GlobalLock(block) }.cast::<u8>();
        if data.is_null() {
            // SAFETY: the block is ours until SetClipboardData succeeds.
            unsafe { GlobalFree(block) };
            return false;
        }
        // SAFETY: `data` points at `bytes.len()` writable bytes; the regions do not overlap.
        unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), data, bytes.len()) };
        // SAFETY: balances the GlobalLock above.
        unsafe { GlobalUnlock(block) };
        // SAFETY: the clipboard is open and emptied by this task; on success it owns `block`.
        if unsafe { SetClipboardData(format, block) }.is_null() {
            // SAFETY: not taken by the clipboard, so still ours.
            unsafe { GlobalFree(block) };
            return false;
        }
        true
    }

    let _guard = CLIPBOARD_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    let Ok(handle) = window.window_handle() else { return false };
    let raw_window_handle::RawWindowHandle::Win32(win32) = handle.as_raw() else { return false };
    // SAFETY: the string is NUL-terminated UTF-16.
    let format = unsafe { RegisterClipboardFormatW(wide("HTML Format").as_ptr()) };
    if format == 0 {
        return false;
    }
    // The window must own the clipboard: SetClipboardData fails after opening it with no owner.
    if !open_clipboard(win32.hwnd.get() as _) {
        return false;
    }
    let mut payload = cf_html(html).into_bytes();
    payload.push(0);
    let plain: Vec<u8> = text.encode_utf16().chain(Some(0)).flat_map(u16::to_le_bytes).collect();
    // SAFETY: the clipboard is open by this task.
    let written =
        unsafe { EmptyClipboard() } != 0 && set(format, &payload) && set(CF_UNICODETEXT, &plain);
    // SAFETY: balances the OpenClipboard above.
    unsafe { CloseClipboard() };
    written
}

pub fn query_system_appearance() -> Option<mpsc::Receiver<bool>> {
    None
}

pub fn attach_parent_console() {
    use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
    // SAFETY: no pointers; failing (no parent console, or one already attached) is harmless.
    unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
}

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
/// Task Manager's per-app startup switch for `Run` entries: a 12-byte REG_BINARY whose first byte
/// is even when enabled and odd when disabled; absent means enabled.
const APPROVED_KEY: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
const RUN_VALUE: &str = "Tachyon";

/// Reads a user registry value into `data` (`None` for a presence check). `Ok(None)` if absent.
fn reg_get(key: &str, flags: u32, data: Option<&mut [u8]>) -> io::Result<Option<usize>> {
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RegGetValueW};
    let (key, value) = (wide(key), wide(RUN_VALUE));
    let (ptr, mut len) = match data {
        Some(buffer) => (buffer.as_mut_ptr(), u32::try_from(buffer.len()).unwrap_or(u32::MAX)),
        None => (ptr::null_mut(), 0),
    };
    let len_ptr = if ptr.is_null() { ptr::null_mut() } else { &raw mut len };
    // SAFETY: `key` and `value` are NUL-terminated UTF-16 strings that outlive the call; `ptr`
    // is either null (presence check, with a null size pointer) or a buffer of `len` bytes.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value.as_ptr(),
            flags,
            ptr::null_mut(),
            ptr.cast(),
            len_ptr,
        )
    };
    match status {
        ERROR_SUCCESS => Ok(Some(len as usize)),
        ERROR_FILE_NOT_FOUND => Ok(None),
        code => Err(io::Error::from_raw_os_error(code as i32)),
    }
}

fn reg_delete(key: &str) -> io::Result<()> {
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RegDeleteKeyValueW};
    let (key, value) = (wide(key), wide(RUN_VALUE));
    // SAFETY: `key` and `value` are NUL-terminated UTF-16 strings that outlive the call.
    match unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr()) } {
        ERROR_SUCCESS | ERROR_FILE_NOT_FOUND => Ok(()),
        code => Err(io::Error::from_raw_os_error(code as i32)),
    }
}

/// On when the `Run` value exists and Task Manager has not disabled it.
pub fn autostart_enabled() -> io::Result<bool> {
    use windows_sys::Win32::System::Registry::{RRF_RT_REG_BINARY, RRF_RT_REG_SZ};
    if reg_get(RUN_KEY, RRF_RT_REG_SZ, None)?.is_none() {
        return Ok(false);
    }
    let mut approved = [0u8; 12];
    Ok(match reg_get(APPROVED_KEY, RRF_RT_REG_BINARY, Some(&mut approved))? {
        Some(len) if len > 0 => approved[0] % 2 == 0,
        _ => true,
    })
}

/// The App Execution Alias Tachyon's manifest registers (`packaging/msix/AppxManifest.xml`),
/// relative to `%LOCALAPPDATA%`: stable across MSIX updates, unlike
/// `GetCurrentPackageFullName`'s versioned `C:\Program Files\WindowsApps\...` install folder.
const PACKAGED_ALIAS: &str = r"Microsoft\WindowsApps\tachyon.exe";

/// The installed MSIX package's full 4-part version ("X.Y.Z.B"), for the About window's
/// Environment table: `GetCurrentPackageFullName`'s result has the shape
/// `Name_Version_Architecture_ResourceId_PublisherHash`, the same identity
/// `packaging/msix/AppxManifest.xml` and `cargo xtask msix` produce, so the version is its second
/// underscore-separated segment. `None` if this process is not packaged (`ERROR_INSUFFICIENT_BUFFER`
/// on the length query is packaged; `APPMODEL_ERROR_NO_PACKAGE` is not) or the name does not have
/// the expected shape.
pub fn packaged_version() -> Option<String> {
    let mut len: u32 = 0;
    // SAFETY: `&raw mut len` is a valid, live `u32` for the call to write its required buffer
    // length to; a null buffer pointer with that length asks only for the length.
    let status = unsafe { GetCurrentPackageFullName(&raw mut len, ptr::null_mut()) };
    if status != ERROR_INSUFFICIENT_BUFFER {
        return None;
    }
    let mut buffer = vec![0u16; len as usize];
    // SAFETY: `buffer` has exactly the `len` UTF-16 units (including the terminator) the call
    // just reported it needs, a valid destination for it to fill.
    let status = unsafe { GetCurrentPackageFullName(&raw mut len, buffer.as_mut_ptr()) };
    if status != 0 {
        return None;
    }
    version_from_full_name(String::from_utf16_lossy(&buffer).trim_end_matches('\0'))
}

/// The version segment of a package full name (see [`packaged_version`]), split out so the
/// parsing is unit-tested without a real package or a WinAPI call.
fn version_from_full_name(full_name: &str) -> Option<String> {
    full_name.split('_').nth(1).map(String::from)
}

/// Whether this process runs from an installed MSIX package. Called from [`set_autostart`]
/// (never on the startup path) and the About window (opened on demand, never at start-up).
fn running_packaged() -> bool {
    packaged_version().is_some()
}

/// The real Windows build number. `GetVersionExW` (and `GetVersion`) report an older, shimmed
/// version once an executable has no manifest asserting support for the running Windows release,
/// so this asks `RtlGetVersion` instead: ntdll.dll always exports it and never shims it.
pub(crate) fn windows_build_number() -> u32 {
    // SAFETY: every all-zero bit pattern is a valid OSVERSIONINFOW.
    let mut info: OSVERSIONINFOW = unsafe { std::mem::zeroed() };
    info.dwOSVersionInfoSize = size_of::<OSVERSIONINFOW>() as u32;
    // SAFETY: `info` is a plain struct `RtlGetVersion` fills in place; `dwOSVersionInfoSize` was
    // just set, as the call needs it to know which version of the struct was passed.
    unsafe { RtlGetVersion(&mut info) };
    info.dwBuildNumber
}

/// "Windows 11" or "Windows 10" plus the build number, for the About window's Environment table
/// ("Platform"). Windows 11 shares NT 10.0 with Windows 10 and is not otherwise distinguishable
/// through `RtlGetVersion`; splitting on build 22000 is the same threshold `winver` and Settings
/// use.
pub fn os_version() -> String {
    let build = windows_build_number();
    format!("{} (build {build})", os_name_for_build(build))
}

/// The name half of [`os_version`], split out so the build-number threshold is unit-tested
/// without a real build number.
fn os_name_for_build(build: u32) -> &'static str {
    if build >= 22000 { "Windows 11" } else { "Windows 10" }
}

/// The path the `Run` key should launch: `exe` unless this process is `packaged`, in which case
/// `exe` is the versioned MSIX install path that changes on every update, and
/// `local_app_data`'s [`PACKAGED_ALIAS`] (stable across updates) is used instead, when known.
fn autostart_target(exe: &Path, packaged: bool, local_app_data: Option<&Path>) -> PathBuf {
    match local_app_data {
        Some(dir) if packaged => dir.join(PACKAGED_ALIAS),
        _ => exe.to_path_buf(),
    }
}

/// Turning it on also clears a Task Manager "disabled" mark (absent means enabled); turning it
/// off removes both values. Points the `Run` key at `exe`, unless this is an MSIX install, in
/// which case it points at the App Execution Alias instead (see [`autostart_target`]): an MSIX
/// package's real path is versioned and would otherwise silently break autostart on the next
/// update.
pub fn set_autostart(exe: &Path, enabled: bool) -> io::Result<()> {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, REG_SZ, RegSetKeyValueW};
    reg_delete(APPROVED_KEY)?;
    if !enabled {
        return reg_delete(RUN_KEY);
    }
    let local_app_data = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    let target = autostart_target(exe, running_packaged(), local_app_data.as_deref());
    let (key, value) = (wide(RUN_KEY), wide(RUN_VALUE));
    let command = wide(&format!("\"{}\" --background", target.display()));
    let bytes = u32::try_from(command.len() * 2)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path too long"))?;
    // SAFETY: all strings are NUL-terminated UTF-16 that outlive the call, and `bytes` is the
    // size of `command` including its terminator, as REG_SZ requires.
    let status = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value.as_ptr(),
            REG_SZ,
            command.as_ptr().cast(),
            bytes,
        )
    };
    match status {
        ERROR_SUCCESS => Ok(()),
        code => Err(io::Error::from_raw_os_error(code as i32)),
    }
}

/// See [`crate::documents_dir`]: the shell's own resolution of the Documents known folder,
/// which follows a OneDrive redirect if the user has set one up.
pub fn documents_dir() -> Option<PathBuf> {
    use windows_sys::Win32::Foundation::S_OK;
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::UI::Shell::{
        FOLDERID_Documents, KF_FLAG_DEFAULT, SHGetKnownFolderPath,
    };

    let mut path: *mut u16 = ptr::null_mut();
    // SAFETY: `path` receives a `CoTaskMemFree`-owned wide string on `S_OK`, freed below;
    // `FOLDERID_Documents` and the NULL token outlive the call.
    let result = unsafe {
        SHGetKnownFolderPath(
            &FOLDERID_Documents,
            KF_FLAG_DEFAULT as u32,
            ptr::null_mut(),
            &mut path,
        )
    };
    if result != S_OK {
        return None;
    }
    let text = pwstr_to_string(path);
    // SAFETY: `path` was allocated by `SHGetKnownFolderPath` above, which documents
    // `CoTaskMemFree` as the way to release it; not read again after this.
    unsafe { CoTaskMemFree(path.cast()) };
    text.map(PathBuf::from)
}

/// Copies a NUL-terminated wide string into a Rust `String`: the caller only has a pointer, as
/// [`documents_dir`]'s `SHGetKnownFolderPath` call returns, not a length.
fn pwstr_to_string(raw: *const u16) -> Option<String> {
    if raw.is_null() {
        return None;
    }
    let mut len = 0usize;
    loop {
        // SAFETY: offsetting by `len` wide characters stays within the NUL-terminated string
        // `SHGetKnownFolderPath` returned; this reads nothing yet.
        let at = unsafe { raw.add(len) };
        // SAFETY: `at` is that same string's `len`-th wide character, guaranteed readable up to
        // and including its terminating NUL.
        if unsafe { at.read() } == 0 {
            break;
        }
        len += 1;
    }
    // SAFETY: the loop above confirmed `len` wide characters before the terminator, all part of
    // one allocation.
    let text = unsafe { std::slice::from_raw_parts(raw, len) };
    Some(String::from_utf16_lossy(text))
}

#[cfg(test)]
mod tests {
    use super::{
        PACKAGED_ALIAS, autostart_target, cf_html, os_name_for_build, version_from_full_name,
    };
    use std::path::Path;

    #[test]
    fn cf_html_offsets_point_at_the_document_and_the_fragment() {
        let payload = cf_html("<p>h\u{e9}llo</p>");
        let offset = |key: &str| -> usize {
            let at = payload.find(key).expect("header field") + key.len();
            payload[at..at + 10].parse().expect("ten digits")
        };
        assert_eq!(&payload[offset("StartFragment:")..offset("EndFragment:")], "<p>h\u{e9}llo</p>");
        assert!(payload[offset("StartHTML:")..].starts_with("<html>"));
        assert_eq!(offset("EndHTML:"), payload.len());
    }

    #[test]
    fn autostart_target_prefers_the_execution_alias_when_packaged() {
        let exe =
            Path::new(r"C:\Program Files\WindowsApps\Tachyon_0.1.0.0_x64__abc123\tachyon.exe");
        let local_app_data = Path::new(r"C:\Users\Dailen\AppData\Local");
        assert_eq!(
            autostart_target(exe, true, Some(local_app_data)),
            local_app_data.join(PACKAGED_ALIAS)
        );
    }

    #[test]
    fn autostart_target_keeps_the_given_exe_when_not_packaged() {
        let exe = Path::new(r"C:\Users\Dailen\Tachyon\tachyon.exe");
        let local_app_data = Path::new(r"C:\Users\Dailen\AppData\Local");
        assert_eq!(autostart_target(exe, false, Some(local_app_data)), exe);
    }

    #[test]
    fn autostart_target_falls_back_to_the_given_exe_without_local_app_data() {
        let exe =
            Path::new(r"C:\Program Files\WindowsApps\Tachyon_0.1.0.0_x64__abc123\tachyon.exe");
        assert_eq!(autostart_target(exe, true, None), exe);
    }

    #[test]
    fn version_from_full_name_is_the_second_underscore_segment() {
        assert_eq!(
            version_from_full_name("Tachyon_0.1.0.5_x64__abc123defg"),
            Some("0.1.0.5".to_owned())
        );
        assert_eq!(version_from_full_name("not-a-package-name"), None);
    }

    #[test]
    fn os_name_for_build_splits_windows_10_and_11_at_22000() {
        assert_eq!(os_name_for_build(19045), "Windows 10");
        assert_eq!(os_name_for_build(22000), "Windows 11");
        assert_eq!(os_name_for_build(26100), "Windows 11");
    }

    #[test]
    fn windows_build_number_is_plausible() {
        // The reference and CI machines are Windows 10 or 11; both report a build number well
        // past `SetPreferredAppMode`'s (18362, checked in `tray::tests`), and well short of a
        // value that would suggest the struct was filled in wrong.
        let build = super::windows_build_number();
        assert!((10000..100_000).contains(&build), "implausible build number {build}");
    }
}
