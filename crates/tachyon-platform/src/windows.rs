//! Windows: the primary is whoever creates the first instance of a named pipe
//! (`FILE_FLAG_FIRST_PIPE_INSTANCE` makes the election atomic). The pipe name
//! carries the session id and user name, so RDP sessions and fast user
//! switching each get their own primary.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::ptr;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED,
    INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_INBOUND};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT, WaitNamedPipeW,
};
use windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows_sys::Win32::UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow};

use crate::protocol;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const PIPE_BUFFER: u32 = 64 * 1024;

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
    let open_mode = PIPE_ACCESS_INBOUND | if first { FILE_FLAG_FIRST_PIPE_INSTANCE } else { 0 };
    let pipe_mode = PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS;
    // SAFETY: `name` is NUL-terminated UTF-16; null security attributes select
    // the default DACL (owner, SYSTEM and administrators get write access).
    let handle = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            open_mode,
            pipe_mode,
            PIPE_UNLIMITED_INSTANCES,
            0,
            PIPE_BUFFER,
            0,
            ptr::null(),
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

pub fn serve(listener: Listener, mut on_message: impl FnMut(Vec<String>)) {
    let Listener { name, mut pipe } = listener;
    loop {
        // SAFETY: `pipe` is a valid synchronous pipe handle; no OVERLAPPED.
        let connected = unsafe { ConnectNamedPipe(pipe.as_raw_handle(), ptr::null_mut()) } != 0
            || io::Error::last_os_error().raw_os_error() == Some(ERROR_PIPE_CONNECTED as i32);
        // Keep a listening instance available before handling this client so
        // concurrent secondaries never observe a missing pipe.
        let Ok(next) = create_instance(&name, false) else { return };
        let current = File::from(std::mem::replace(&mut pipe, next));
        if connected && let Ok(Some(args)) = protocol::read_message(&current) {
            on_message(args);
        }
    }
}

pub fn send(client: Client, message: &[u8]) -> io::Result<()> {
    // The primary is not the foreground process; without this grant Windows'
    // foreground lock prevents it from raising its window.
    // SAFETY: plain Win32 call without pointers.
    unsafe { AllowSetForegroundWindow(ASFW_ANY) };
    let deadline = Instant::now() + CONNECT_TIMEOUT;
    let mut pipe = loop {
        match OpenOptions::new().write(true).open(&client.name) {
            Ok(pipe) => break pipe,
            Err(e) if Instant::now() < deadline => match e.raw_os_error() {
                Some(code) if code == ERROR_PIPE_BUSY as i32 => {
                    let wide_name = wide(&client.name);
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
    pipe.write_all(message)
}
