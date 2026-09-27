//! The notification-area (tray) icon of a resident instance, and the icon of Tachyon's windows.
//!
//! The icon ships inside the binary as an `.ico` with 32-bit DIB images (no PNG, so creating an
//! icon never loads an image codec). Regenerate it from `assets/tachyon.svg` with:
//!
//! ```sh
//! for s in 16 20 24 32 40 48 64; do rsvg-convert -w $s -h $s tachyon.svg -o $s.png; done
//! magick 16.png 20.png 24.png 32.png 40.png 48.png 64.png tachyon.ico
//! ```

use std::ptr;
use std::sync::{Mutex, mpsc};
use std::thread::JoinHandle;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{GetDpiForSystem, GetDpiForWindow, GetSystemMetricsForDpi};
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_SETVERSION, NIN_SELECT,
    NINF_KEY, NOTIFYICON_VERSION_4, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconFromResourceEx, CreatePopupMenu, CreateWindowExW, DefWindowProcW,
    DestroyMenu, DestroyWindow, DispatchMessageW, GWLP_USERDATA, GetMessageW, GetWindowLongPtrW,
    HICON, ICON_BIG, ICON_SMALL, LR_DEFAULTCOLOR, MF_SEPARATOR, MF_STRING, MSG, PostMessageW,
    PostQuitMessage, RegisterClassExW, RegisterWindowMessageW, SM_CXICON, SM_CXSMICON,
    SendMessageW, SetForegroundWindow, SetWindowLongPtrW, TPM_NONOTIFY, TPM_RETURNCMD,
    TPM_RIGHTBUTTON, TrackPopupMenuEx, TranslateMessage, WM_APP, WM_CLOSE, WM_CONTEXTMENU,
    WM_DESTROY, WM_NULL, WM_SETICON, WNDCLASSEXW, WS_OVERLAPPED,
};

use super::wide;
use crate::TrayEvent;

const ICO: &[u8] = include_bytes!("../../assets/tachyon.ico");

/// Message the shell sends for clicks on the icon.
const WM_TRAY: u32 = WM_APP + 1;
/// Enter or Space on the focused icon (`NIN_KEYSELECT`, which windows-sys does not define).
const NIN_KEYSELECT: u32 = NIN_SELECT | NINF_KEY;
/// Context menu commands.
const CMD_NEW_WINDOW: usize = 1;
const CMD_QUIT: usize = 2;

/// The image in [`ICO`] best suited to `size` pixels: the smallest at least that large, else the
/// largest. Returns its bytes (a DIB, as `CreateIconFromResourceEx` takes it).
fn ico_image(size: i32) -> Option<&'static [u8]> {
    let count = usize::from(u16::from_le_bytes([*ICO.get(4)?, *ICO.get(5)?]));
    let entries = (0..count).filter_map(|i| {
        let entry = ICO.get(6 + 16 * i..22 + 16 * i)?;
        let width = if entry[0] == 0 { 256 } else { i32::from(entry[0]) };
        let len = u32::from_le_bytes(entry[8..12].try_into().ok()?) as usize;
        let offset = u32::from_le_bytes(entry[12..16].try_into().ok()?) as usize;
        Some((width, ICO.get(offset..offset + len)?))
    });
    type Image = Option<(i32, &'static [u8])>;
    let (mut best, mut largest): (Image, Image) = (None, None);
    for (width, bytes) in entries {
        if width >= size && best.is_none_or(|(w, _)| width < w) {
            best = Some((width, bytes));
        }
        if largest.is_none_or(|(w, _)| width > w) {
            largest = Some((width, bytes));
        }
    }
    best.or(largest).map(|(_, bytes)| bytes)
}

/// Tachyon's icon at `size` pixels, created once per size and kept for the process's lifetime.
fn icon(size: i32) -> HICON {
    // HICON is a pointer, so it is kept as an integer to share the cache between threads.
    static ICONS: Mutex<Vec<(i32, usize)>> = Mutex::new(Vec::new());
    let mut icons = ICONS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(&(_, icon)) = icons.iter().find(|(s, _)| *s == size) {
        return icon as HICON;
    }
    let Some(image) = ico_image(size) else { return ptr::null_mut() };
    // SAFETY: `image` is a complete icon image (BITMAPINFOHEADER, XOR and AND bitmaps) from the
    // embedded file, valid for the call; 0x00030000 is the format version the function requires.
    let icon = unsafe {
        CreateIconFromResourceEx(
            image.as_ptr(),
            image.len() as u32,
            1,
            0x0003_0000,
            size,
            size,
            LR_DEFAULTCOLOR,
        )
    };
    if !icon.is_null() {
        icons.push((size, icon as usize));
    }
    icon
}

pub fn set_window_icon(window: &impl raw_window_handle::HasWindowHandle) {
    let Ok(handle) = window.window_handle() else { return };
    let raw_window_handle::RawWindowHandle::Win32(win32) = handle.as_raw() else { return };
    let hwnd = win32.hwnd.get() as HWND;
    // SAFETY: `hwnd` is the live window GPUI just returned.
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    for (kind, metric) in [(ICON_SMALL, SM_CXSMICON), (ICON_BIG, SM_CXICON)] {
        // SAFETY: plain query.
        let icon = icon(unsafe { GetSystemMetricsForDpi(metric, dpi) });
        if !icon.is_null() {
            // SAFETY: `hwnd` is live; WM_SETICON takes the icon by value (the window does not own
            // it, and the cache keeps it alive).
            unsafe { SendMessageW(hwnd, WM_SETICON, kind as WPARAM, icon as LPARAM) };
        }
    }
}

/// The tray icon's hidden window, running its message loop on its own thread.
pub struct Tray {
    hwnd: usize,
    thread: Option<JoinHandle<()>>,
}

impl Drop for Tray {
    fn drop(&mut self) {
        // SAFETY: posting to a window handle is safe even if the window is already gone.
        unsafe { PostMessageW(self.hwnd as HWND, WM_CLOSE, 0, 0) };
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub fn show_tray(tooltip: &str, on_event: impl Fn(TrayEvent) + Send + 'static) -> Option<Tray> {
    let tooltip = tooltip.to_owned();
    let (ready, window) = mpsc::sync_channel(1);
    let thread = std::thread::Builder::new()
        .name("tray".to_owned())
        .spawn(move || run(&tooltip, Box::new(on_event), &ready))
        .ok()?;
    match window.recv() {
        Ok(Some(hwnd)) => Some(Tray { hwnd, thread: Some(thread) }),
        _ => {
            let _ = thread.join();
            None
        }
    }
}

/// State of the tray window, owned by its thread and reachable from the window procedure.
struct State {
    tooltip: Vec<u16>,
    on_event: Box<dyn Fn(TrayEvent) + Send>,
    /// Broadcast when Explorer (re)starts; the icon has to be added again.
    taskbar_created: u32,
}

fn run(
    tooltip: &str,
    on_event: Box<dyn Fn(TrayEvent) + Send>,
    ready: &mpsc::SyncSender<Option<usize>>,
) {
    let class = wide("TachyonTray");
    // SAFETY: a NULL module name returns this executable's handle.
    let instance = unsafe { GetModuleHandleW(ptr::null()) };
    let wc = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(window_proc),
        hInstance: instance,
        lpszClassName: class.as_ptr(),
        // SAFETY: WNDCLASSEXW is plain data; the remaining fields are optional and zero.
        ..unsafe { std::mem::zeroed() }
    };
    // SAFETY: `wc` and the class name it points to outlive the call. Registering twice (a second
    // tray in one process) fails harmlessly and the existing class is used.
    unsafe { RegisterClassExW(&wc) };
    // A hidden top-level window rather than a message-only one: only top-level windows receive
    // the TaskbarCreated broadcast.
    // SAFETY: the class is registered and the strings are NUL-terminated UTF-16.
    let hwnd = unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            class.as_ptr(),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            ptr::null_mut(),
            ptr::null_mut(),
            instance,
            ptr::null(),
        )
    };
    if hwnd.is_null() {
        let _ = ready.send(None);
        return;
    }
    let state = Box::new(State {
        tooltip: wide(tooltip),
        on_event,
        // SAFETY: the string is NUL-terminated UTF-16.
        taskbar_created: unsafe { RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()) },
    });
    let state = Box::into_raw(state);
    // SAFETY: `hwnd` belongs to this thread; the pointer stays valid until WM_DESTROY frees it.
    unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, state as isize) };
    // SAFETY: `state` was just created and is only used on this thread.
    if !add_icon(hwnd, unsafe { &*state }) {
        // SAFETY: `hwnd` is ours; WM_DESTROY frees the state.
        unsafe { DestroyWindow(hwnd) };
        let _ = ready.send(None);
        return;
    }
    let _ = ready.send(Some(hwnd as usize));
    // SAFETY: MSG is plain data filled by GetMessageW.
    let mut msg: MSG = unsafe { std::mem::zeroed() };
    // SAFETY: standard message loop for this thread's windows; ends with WM_QUIT (0) or an
    // error (-1).
    while unsafe { GetMessageW(&mut msg, ptr::null_mut(), 0, 0) } > 0 {
        // SAFETY: `msg` was filled by GetMessageW.
        unsafe { TranslateMessage(&msg) };
        // SAFETY: as above.
        unsafe { DispatchMessageW(&msg) };
    }
}

fn notify_data(hwnd: HWND, state: &State) -> NOTIFYICONDATAW {
    // SAFETY: NOTIFYICONDATAW is plain data; zero is valid for every field.
    let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = hwnd;
    data.uID = 1;
    let tip = &state.tooltip[..state.tooltip.len().min(data.szTip.len())];
    data.szTip[..tip.len()].copy_from_slice(tip);
    data
}

fn add_icon(hwnd: HWND, state: &State) -> bool {
    let mut data = notify_data(hwnd, state);
    data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
    data.uCallbackMessage = WM_TRAY;
    // SAFETY: plain query.
    let dpi = unsafe { GetDpiForSystem() };
    // SAFETY: plain query.
    data.hIcon = icon(unsafe { GetSystemMetricsForDpi(SM_CXSMICON, dpi) });
    data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
    // SAFETY: `data` is initialized and outlives the call.
    let added = unsafe { Shell_NotifyIconW(NIM_ADD, &data) } != 0;
    // SAFETY: as above.
    added && unsafe { Shell_NotifyIconW(NIM_SETVERSION, &data) } != 0
}

/// Shows the context menu at screen position (`x`, `y`) and returns the chosen command, or 0.
fn context_menu(hwnd: HWND, x: i32, y: i32) -> usize {
    // SAFETY: creates an empty menu, destroyed below.
    let menu = unsafe { CreatePopupMenu() };
    if menu.is_null() {
        return 0;
    }
    let (new_window, quit) = (wide("New window"), wide("Quit Tachyon"));
    // SAFETY: `menu` is ours; the item strings outlive the calls (the menu copies them).
    unsafe { AppendMenuW(menu, MF_STRING, CMD_NEW_WINDOW, new_window.as_ptr()) };
    // SAFETY: as above; a separator has no string.
    unsafe { AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null()) };
    // SAFETY: as above.
    unsafe { AppendMenuW(menu, MF_STRING, CMD_QUIT, quit.as_ptr()) };
    // Making the window foreground first lets the menu close when the user clicks elsewhere.
    // SAFETY: `hwnd` is ours.
    unsafe { SetForegroundWindow(hwnd) };
    let flags = TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON;
    // SAFETY: `menu` and `hwnd` are ours; with TPM_RETURNCMD the chosen command is returned.
    let command = unsafe { TrackPopupMenuEx(menu, flags, x, y, hwnd, ptr::null()) };
    // The documented workaround for the menu failing to open a second time.
    // SAFETY: `hwnd` is ours.
    unsafe { PostMessageW(hwnd, WM_NULL, 0, 0) };
    // SAFETY: `menu` is ours and no longer shown.
    unsafe { DestroyMenu(menu) };
    command as usize
}

/// Signed low and high words of `value`, as the shell packs coordinates.
fn words(value: usize) -> (i32, i32) {
    (i32::from(value as u16 as i16), i32::from((value >> 16) as u16 as i16))
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: plain query of this window's user data.
    let state = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const State;
    // SAFETY: the user data is the `State` set right after creation (or 0 before that), freed
    // only in WM_DESTROY and only touched on this window's thread.
    let state = unsafe { state.as_ref() };
    match (msg, state) {
        (WM_TRAY, Some(state)) => {
            // With NOTIFYICON_VERSION_4 the event is in the low word of `lparam` and the
            // anchor point in `wparam`.
            match lparam as u32 & 0xffff {
                NIN_SELECT | NIN_KEYSELECT => (state.on_event)(TrayEvent::Open),
                WM_CONTEXTMENU => {
                    let (x, y) = words(wparam);
                    match context_menu(hwnd, x, y) {
                        CMD_NEW_WINDOW => (state.on_event)(TrayEvent::Open),
                        CMD_QUIT => (state.on_event)(TrayEvent::Quit),
                        _ => {}
                    }
                }
                _ => {}
            }
            0
        }
        (WM_CLOSE, Some(state)) => {
            let data = notify_data(hwnd, state);
            // SAFETY: `data` identifies the icon added in `add_icon`; `hwnd` is ours.
            unsafe { Shell_NotifyIconW(NIM_DELETE, &data) };
            // SAFETY: `hwnd` is ours.
            unsafe { DestroyWindow(hwnd) };
            0
        }
        (WM_DESTROY, _) => {
            // SAFETY: the pointer came from `Box::into_raw` in `run` and is cleared here, so it
            // is freed exactly once.
            let state = unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) } as *mut State;
            if !state.is_null() {
                // SAFETY: as above.
                drop(unsafe { Box::from_raw(state) });
            }
            // SAFETY: ends this thread's message loop.
            unsafe { PostQuitMessage(0) };
            0
        }
        (msg, Some(state)) if msg == state.taskbar_created => {
            add_icon(hwnd, state);
            0
        }
        // SAFETY: default handling for everything else.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_the_smallest_image_at_least_as_large_as_asked() {
        let width = |image: &[u8]| i32::from_le_bytes(image[4..8].try_into().expect("header"));
        assert_eq!(ico_image(16).map(width), Some(16));
        assert_eq!(ico_image(18).map(width), Some(20), "the next size up, scaled down");
        assert_eq!(ico_image(30).map(width), Some(32));
        assert_eq!(ico_image(128).map(width), Some(64), "the largest when none is large enough");
    }
}
