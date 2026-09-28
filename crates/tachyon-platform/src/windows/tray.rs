//! The notification-area (tray) icon of a resident instance, and the icon of Tachyon's windows.
//!
//! The icon ships inside the binary as an `.ico` with 32-bit DIB images (no PNG, so creating an
//! icon never loads an image codec). `cargo xtask icons` regenerates it from the brand art in
//! `assets/brand/` and checks that every image is a DIB.
//!
//! The icon's context menu follows Tachyon's resolved theme (`set_popup_menu_dark`,
//! `apply_dark_menu_theme`): there is no documented way to ask `TrackPopupMenuEx` for a dark
//! menu, so this reaches for the same undocumented `uxtheme.dll` mode switch Windows Terminal and
//! Notepad++ use.

use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex, mpsc};
use std::thread::JoinHandle;

use windows_sys::Wdk::System::SystemServices::RtlGetVersion;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};
use windows_sys::Win32::System::SystemInformation::OSVERSIONINFOW;
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

/// `uxtheme.dll`'s ordinal 135 (undocumented: no header declares it, and it has no exported name,
/// only this ordinal): `int SetPreferredAppMode(int mode)`. Setting it makes windows and common
/// controls created afterwards *on the calling thread* - including a popup menu about to be shown
/// with `TrackPopupMenuEx` - paint dark or light instead of following the system's setting. This
/// is the same undocumented call Windows Terminal and Notepad++ make for their own dark menus;
/// there is no supported, documented alternative.
const ORD_SET_PREFERRED_APP_MODE: usize = 135;
/// `uxtheme.dll`'s ordinal 136: `void FlushMenuThemes()`. Repaints menu theme data the system may
/// already have cached from before `SetPreferredAppMode` changed; every public description of this
/// pair calls it immediately afterwards, so this does the same.
const ORD_FLUSH_MENU_THEMES: usize = 136;

/// `SetPreferredAppMode`'s `PreferredAppMode` enum: 2 forces dark, 3 forces light. 1 ("allow
/// dark", i.e. follow the system) is never used here: Tachyon's theme is an explicit choice
/// (`Theme::for_window` - settings, or the system only when `theme = "system"`), not "whatever the
/// system just changed to".
const FORCE_DARK: i32 = 2;
const FORCE_LIGHT: i32 = 3;

type SetPreferredAppMode = unsafe extern "system" fn(i32) -> i32;
type FlushMenuThemes = unsafe extern "system" fn();

/// The real Windows build number. `GetVersionExW` (and `GetVersion`) report an older, shimmed
/// version once an executable has no manifest asserting support for the running Windows release,
/// so `SetPreferredAppMode`'s availability (Windows 10 1903, build 18362) is checked with
/// `RtlGetVersion` instead: ntdll.dll always exports it and never shims it.
fn windows_build_number() -> u32 {
    // SAFETY: every all-zero bit pattern is a valid OSVERSIONINFOW.
    let mut info: OSVERSIONINFOW = unsafe { std::mem::zeroed() };
    info.dwOSVersionInfoSize = size_of::<OSVERSIONINFOW>() as u32;
    // SAFETY: `info` is a plain struct `RtlGetVersion` fills in place; `dwOSVersionInfoSize` was
    // just set, as the call needs it to know which version of the struct was passed.
    unsafe { RtlGetVersion(&mut info) };
    info.dwBuildNumber
}

/// Resolves [`ORD_SET_PREFERRED_APP_MODE`] and [`ORD_FLUSH_MENU_THEMES`], the first time a menu
/// needs to be dark or light rather than at process start: only [`apply_dark_menu_theme`] calls
/// this, and only [`context_menu`] calls that, right before a menu is ever shown. `None` before
/// Windows 10 1903 (earlier builds either lack the ordinal or export a different, incompatible
/// function there - `AllowDarkModeForApp`, which takes a `bool` - under the same number) or if
/// `uxtheme.dll` ever stops exporting them. The two addresses are kept as `usize` (the same trick
/// [`icon`]'s cache uses for `HICON`) so the `LazyLock` is `Send` and `Sync` without an `unsafe
/// impl`; `module` is never freed, so they stay valid for the rest of the process.
fn dark_menu_api() -> Option<(SetPreferredAppMode, FlushMenuThemes)> {
    static API: LazyLock<Option<(usize, usize)>> = LazyLock::new(|| {
        if windows_build_number() < 18362 {
            return None;
        }
        let name = wide("uxtheme.dll");
        // SAFETY: a well-known system DLL name, NUL-terminated UTF-16; if it is somehow missing,
        // the call just returns null.
        let module = unsafe { LoadLibraryW(name.as_ptr()) };
        if module.is_null() {
            return None;
        }
        // SAFETY: an ordinal under 0x10000 passed where `GetProcAddress` expects a name means
        // "look this export up by ordinal instead" (the documented `MAKEINTRESOURCEA`
        // convention) - the only way to reach an export that has no name in the table.
        let set_mode = unsafe { GetProcAddress(module, ORD_SET_PREFERRED_APP_MODE as *const u8) };
        // SAFETY: as above.
        let flush = unsafe { GetProcAddress(module, ORD_FLUSH_MENU_THEMES as *const u8) };
        match (set_mode, flush) {
            (Some(set_mode), Some(flush)) => Some((set_mode as usize, flush as usize)),
            _ => None,
        }
    });
    API.map(|(set_mode, flush)| {
        // SAFETY: `set_mode` came from `GetProcAddress` just above, resolved once and never
        // invalidated; called with the signature uxtheme.dll's own (never public) declaration
        // gives this ordinal, which every description of this undocumented API agrees on.
        let set_preferred_app_mode =
            unsafe { std::mem::transmute::<usize, SetPreferredAppMode>(set_mode) };
        // SAFETY: as above, for `flush` and `FlushMenuThemes`'s ordinal.
        let flush_menu_themes = unsafe { std::mem::transmute::<usize, FlushMenuThemes>(flush) };
        (set_preferred_app_mode, flush_menu_themes)
    })
}

/// The dark/light choice popup menus shown after this apply. Set by `set_popup_menu_dark`
/// (re-exported from `crate::windows`), called wherever a window or the settings resolve
/// Tachyon's theme (`Theme::for_window`, `apply_to_windows`, `follow_appearance`); read by
/// [`apply_dark_menu_theme`] right before [`context_menu`] shows one. A plain atomic store -
/// resolving and calling into uxtheme.dll happens only at the read site, not here, so a theme
/// change costs nothing until a menu is actually about to show.
static POPUP_MENU_DARK: AtomicBool = AtomicBool::new(false);

/// Records the dark/light choice the tray's context menu should show next. Cheap (an atomic
/// store): see [`POPUP_MENU_DARK`] for why the uxtheme calls themselves wait until the menu shows.
pub fn set_popup_menu_dark(dark: bool) {
    POPUP_MENU_DARK.store(dark, Ordering::Relaxed);
}

/// Applies [`POPUP_MENU_DARK`] to this thread's next popup menu: `SetPreferredAppMode`, then
/// `FlushMenuThemes` so a menu theme already cached from before is dropped. The undocumented
/// dark-mode preference is per-thread, which is why this runs on the tray window's own thread,
/// right before `TrackPopupMenuEx` in [`context_menu`], rather than wherever the theme was last
/// decided (typically GPUI's main thread). A no-op before Windows 10 1903 or if `uxtheme.dll`
/// does not export the ordinals ([`dark_menu_api`]).
fn apply_dark_menu_theme() {
    let Some((set_preferred_app_mode, flush_menu_themes)) = dark_menu_api() else { return };
    let mode = if POPUP_MENU_DARK.load(Ordering::Relaxed) { FORCE_DARK } else { FORCE_LIGHT };
    // SAFETY: `set_preferred_app_mode` takes and returns a `PreferredAppMode` `i32`; called on
    // this thread, right before showing a popup menu, as the undocumented API requires.
    unsafe { set_preferred_app_mode(mode) };
    // SAFETY: `flush_menu_themes` takes no arguments; called right after `SetPreferredAppMode`, as
    // every description of the pair agrees it must be, so no already-cached menu theme survives.
    unsafe { flush_menu_themes() };
}

/// Shows the context menu at screen position (`x`, `y`) and returns the chosen command, or 0.
fn context_menu(hwnd: HWND, x: i32, y: i32) -> usize {
    // Before the menu exists at all, so it is created with the right theme's system brushes
    // (there is no way to retheme an existing menu, only to influence the next one created).
    apply_dark_menu_theme();
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

    #[test]
    fn windows_build_number_is_plausible() {
        // The reference and CI machines are Windows 10 or 11; both report a build number well
        // past `SetPreferredAppMode`'s (18362), and well short of a value that would suggest the
        // struct was filled in wrong.
        let build = windows_build_number();
        assert!((10000..100_000).contains(&build), "implausible build number {build}");
    }
}
