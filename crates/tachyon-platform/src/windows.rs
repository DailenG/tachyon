//! Windows: the primary is whoever creates the first instance of a named pipe
//! (`FILE_FLAG_FIRST_PIPE_INSTANCE` makes the election atomic). The pipe name
//! carries the session id and user name, so RDP sessions and fast user
//! switching each get their own primary.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::ptr;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED,
    INVALID_HANDLE_VALUE, LocalFree,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT, WaitNamedPipeW,
};
use windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows_sys::Win32::UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow};

use crate::protocol;

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
