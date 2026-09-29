//! `RegisterHotKey` associates a hotkey with the registering thread's message queue rather than a
//! window, so this spawns a dedicated thread with its own `GetMessageW` loop; `WM_HOTKEY` there
//! calls `on_press`. Dropping [`GlobalHotkey`] posts `WM_QUIT` to that thread and joins it, so
//! `UnregisterHotKey` always runs on the same thread that registered it, as the API requires -
//! including when the app is quitting, since `Drop` still runs on an ordinary scope exit either
//! way.

use std::ptr;
use std::sync::mpsc;
use std::thread::JoinHandle;

use windows_sys::Win32::Foundation::{ERROR_HOTKEY_ALREADY_REGISTERED, GetLastError};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    MOD_NOREPEAT, RegisterHotKey, UnregisterHotKey,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetMessageW, MSG, PostThreadMessageW, WM_HOTKEY, WM_QUIT,
};

use crate::{Hotkey, HotkeyError};

/// `RegisterHotKey`'s `id`: only one hotkey is ever registered per thread here, so any constant
/// value works.
const HOTKEY_ID: i32 = 1;

/// A live registration; dropping it unregisters (see the module doc comment).
pub struct GlobalHotkey {
    thread_id: u32,
    thread: Option<JoinHandle<()>>,
}

impl Drop for GlobalHotkey {
    fn drop(&mut self) {
        // SAFETY: `thread_id` names a thread that, by construction, already has a message queue
        // (created by its own `RegisterHotKey` call before it ever sent this id back);
        // `PostThreadMessageW` is otherwise safe to call even if that thread has since exited.
        unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0) };
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// See [`crate::register_global_hotkey`].
pub fn register_global_hotkey(
    hotkey: Hotkey,
    on_press: Box<dyn Fn() + Send + 'static>,
) -> Result<GlobalHotkey, HotkeyError> {
    let mods = crate::windows_hotkey_modifiers(hotkey) | MOD_NOREPEAT;
    let vk = crate::windows_virtual_key(hotkey.key);
    let (ready, registered) = mpsc::sync_channel::<Result<u32, HotkeyError>>(1);
    let thread = std::thread::Builder::new()
        .name("global-hotkey".to_owned())
        .spawn(move || run(mods, vk, on_press, &ready))
        .map_err(|err| HotkeyError::Failed(err.to_string()))?;
    match registered.recv() {
        Ok(Ok(thread_id)) => Ok(GlobalHotkey { thread_id, thread: Some(thread) }),
        Ok(Err(err)) => {
            let _ = thread.join();
            Err(err)
        }
        Err(_) => {
            let _ = thread.join();
            Err(HotkeyError::Failed("hotkey thread exited before registering".to_owned()))
        }
    }
}

/// Runs entirely on its own thread: registers the hotkey, reports the outcome through `ready`,
/// then serves `WM_HOTKEY` until `WM_QUIT` (posted by [`GlobalHotkey::drop`]). Never panics: a
/// panicking `on_press` is caught, so it cannot unwind this loop and leave the hotkey registered
/// with no thread left to unregister it.
fn run(
    mods: u32,
    vk: u32,
    on_press: Box<dyn Fn() + Send>,
    ready: &mpsc::SyncSender<Result<u32, HotkeyError>>,
) {
    // SAFETY: reads this thread's own id; no preconditions.
    let thread_id = unsafe { GetCurrentThreadId() };
    // SAFETY: a NULL `hwnd` registers the hotkey against the calling thread's message queue
    // (creating one, if this thread has none yet) instead of a window; `mods` and `vk` are plain
    // values, and `HOTKEY_ID` is only ever registered once per thread.
    let registered = unsafe { RegisterHotKey(ptr::null_mut(), HOTKEY_ID, mods, vk) };
    if registered == 0 {
        // SAFETY: reads this thread's last error, set by the failed call above.
        let error = unsafe { GetLastError() };
        let result = if error == ERROR_HOTKEY_ALREADY_REGISTERED {
            Err(HotkeyError::InUse)
        } else {
            Err(HotkeyError::Failed(format!("RegisterHotKey failed (error {error})")))
        };
        let _ = ready.send(result);
        return;
    }
    if ready.send(Ok(thread_id)).is_err() {
        // The caller gave up before this thread could report success; nothing left to serve.
        // SAFETY: `hwnd` NULL and `id` match the registration above, on the same thread.
        unsafe { UnregisterHotKey(ptr::null_mut(), HOTKEY_ID) };
        return;
    }
    let mut msg = MSG::default();
    loop {
        // SAFETY: `msg` is filled in place; the loop ends on `WM_QUIT` (0) or an error (-1).
        let got = unsafe { GetMessageW(&mut msg, ptr::null_mut(), 0, 0) };
        if got <= 0 {
            break;
        }
        if msg.message == WM_HOTKEY && msg.wParam == HOTKEY_ID as usize {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(&on_press));
        }
    }
    // SAFETY: `hwnd` NULL and `id` match the registration above, on the same thread.
    unsafe { UnregisterHotKey(ptr::null_mut(), HOTKEY_ID) };
}
