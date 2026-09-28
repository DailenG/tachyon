//! The taskbar/Start jump list (right-click the pinned or running icon): a "Recent" category
//! mirroring `Open recent` (newest first, up to [`RECENT_LIMIT`]) and a "New window" task that
//! relaunches the executable with no file - exactly what double-clicking it does - which the
//! single-instance protocol then forwards to the running instance as a fresh window
//! (`Source::from_cli`'s own choice for an empty argument list, `crates/tachyon/src/app.rs`; this
//! task does not pick the document, only performs the same bare launch a click always does).
//!
//! Built with `ICustomDestinationList` the same way Zed's own jump list does
//! (`gpui_windows::destination_list`, which this follows closely): `BeginList`, `AppendCategory` /
//! `AddUserTasks` with `IShellLink` items collected through `IObjectCollection`, `CommitList`.
//! Explorer requires every `IShellLink` item added this way to carry an explicit `PKEY_Title`
//! property (see `ICustomDestinationList::AppendCategory`'s remarks): without one, Explorer
//! renders a blank line instead of the item.
//!
//! The AppUserModelID this list attaches to is set once, early in start-up, through GPUI's own
//! `App::set_app_identity` (called from `crates/tachyon/src/app.rs`) rather than duplicated here;
//! see that call site's comment for why.
//!
//! Rebuilding a jump list is a handful of COM round trips (tens of microseconds each, but COM
//! objects here are apartment-threaded, so they need a dedicated, COM-initialized thread rather
//! than running on whichever background-executor thread happens to call [`update`]). That thread
//! is spawned once, the first time anything touches [`MAILBOX`], and lives for the process's
//! lifetime, like the tray's own message-loop thread. A [`Mailbox`] hands it work: a call to
//! [`update`] while a rebuild is already in flight replaces the pending list rather than queuing
//! another one, so a burst of opens (restoring several backups at once) costs one rebuild, not one
//! per file.

use std::path::{Path, PathBuf};
use std::sync::{Condvar, LazyLock, Mutex};

use windows::Win32::Storage::EnhancedStorage::PKEY_Title;
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
};
use windows::Win32::UI::Shell::Common::{IObjectArray, IObjectCollection};
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows::Win32::UI::Shell::{
    DestinationList, EnumerableObjectCollection, ICustomDestinationList, IShellLinkW, ShellLink,
};
use windows::core::{HSTRING, Interface, Result};

/// How many recent files the "Recent" category shows: fewer than `Open recent`'s own history
/// (`tachyon_editor::picker::RECENT_LIMIT`, 30), matching the assignment's "up to 10" and the
/// taskbar's limited real estate.
const RECENT_LIMIT: usize = 10;

/// A single-slot mailbox: [`send`](Mailbox::send) overwrites whatever is waiting, so a reader
/// blocked in [`recv`](Mailbox::recv) always wakes to the *latest* value, never a queue of stale
/// ones. This is the coalescing: several rapid [`update`] calls collapse into one rebuild using
/// whichever list was current when the worker thread got back around to checking.
struct Mailbox<T> {
    slot: Mutex<Option<T>>,
    wake: Condvar,
}

impl<T> Mailbox<T> {
    const fn new() -> Self {
        Self { slot: Mutex::new(None), wake: Condvar::new() }
    }

    fn send(&self, value: T) {
        let mut slot = self.slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        *slot = Some(value);
        drop(slot);
        self.wake.notify_one();
    }

    fn recv(&self) -> T {
        let mut slot = self.slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        loop {
            if let Some(value) = slot.take() {
                return value;
            }
            slot = self.wake.wait(slot).unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }
}

/// The worker thread's mailbox. Spawning the thread is this static's side effect, the first time
/// anything touches it (`update`, below): if the thread cannot be spawned (an exceedingly
/// unlikely resource exhaustion), `send` still works, just with nothing ever reading it - the
/// jump list silently never updates rather than the process failing to start.
static MAILBOX: LazyLock<Mailbox<Vec<PathBuf>>> = LazyLock::new(|| {
    let _ = std::thread::Builder::new().name("tachyon-jump-list".to_owned()).spawn(run);
    Mailbox::new()
});

/// Queues a jump-list rebuild from `recent` (`Open recent`'s own list, newest first; only the
/// first [`RECENT_LIMIT`] are shown). Returns immediately; the rebuild happens on the worker
/// thread (see the module doc comment), never the caller's.
pub fn update(recent: &[PathBuf]) {
    MAILBOX.send(recent.to_vec());
}

/// The worker thread's body: initializes COM once for this thread (`ICustomDestinationList` and
/// `IShellLinkW` are apartment-threaded shell components), then rebuilds the jump list for every
/// value the mailbox delivers, forever - there is no shutdown, since this runs for the process's
/// lifetime.
fn run() {
    // SAFETY: single COM call, on a thread that makes no other apartment-affecting call before
    // it; the `HRESULT` is intentionally ignored (`S_OK` and `S_FALSE`, already-initialized, are
    // both fine, and any other failure just makes every `CoCreateInstance` below fail instead of
    // panicking).
    unsafe { _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    loop {
        let recent = MAILBOX.recv();
        _ = apply(&recent);
    }
}

fn apply(recent: &[PathBuf]) -> Result<()> {
    // SAFETY: `DestinationList`'s in-proc server implements `ICustomDestinationList`.
    let list: ICustomDestinationList =
        unsafe { CoCreateInstance(&DestinationList, None, CLSCTX_INPROC_SERVER) }?;
    let mut slots = 0u32;
    // SAFETY: `list` is a freshly created, live `ICustomDestinationList`. The removed-destinations
    // array this returns is intentionally left unused: Tachyon always rebuilds both the category
    // and the task from the current recent list, so a user-removed item just stays absent until
    // it is opened again rather than being tracked and excluded from future rebuilds.
    let _removed: IObjectArray = unsafe { list.BeginList(&mut slots) }?;

    if !recent.is_empty() {
        append_recent_category(&list, recent)?;
    }
    append_new_window_task(&list)?;

    // SAFETY: single COM call on the live `list`.
    unsafe { list.CommitList() }
}

/// The files `append_recent_category` turns into shell-link items, in order: `recent` truncated
/// to [`RECENT_LIMIT`]. Pulled out as pure logic so a rename or an off-by-one is a unit test,
/// not a manual jump-list inspection.
fn recent_category_files(recent: &[PathBuf]) -> &[PathBuf] {
    &recent[..recent.len().min(RECENT_LIMIT)]
}

fn append_recent_category(list: &ICustomDestinationList, recent: &[PathBuf]) -> Result<()> {
    let items = new_object_collection()?;
    for file in recent_category_files(recent) {
        let link = shell_link_for_file(file)?;
        // SAFETY: `link` is a fully initialized `IShellLinkW` with `PKEY_Title` set
        // (`shell_link_for_file`); `items` takes its own reference.
        unsafe { items.AddObject(&link) }?;
    }
    // SAFETY: `list` and `items` are both live; `IObjectCollection` derives from `IObjectArray`,
    // which `AppendCategory` needs.
    unsafe { list.AppendCategory(&HSTRING::from("Recent"), &items) }
}

fn append_new_window_task(list: &ICustomDestinationList) -> Result<()> {
    let tasks = new_object_collection()?;
    let task = shell_link_for_new_window()?;
    // SAFETY: see `append_recent_category`.
    unsafe { tasks.AddObject(&task) }?;
    // SAFETY: see `append_recent_category`.
    unsafe { list.AddUserTasks(&tasks) }
}

fn new_object_collection() -> Result<IObjectCollection> {
    // SAFETY: `EnumerableObjectCollection`'s in-proc server implements `IObjectCollection`.
    unsafe { CoCreateInstance(&EnumerableObjectCollection, None, CLSCTX_INPROC_SERVER) }
}

fn new_shell_link() -> Result<IShellLinkW> {
    // SAFETY: `ShellLink`'s in-proc server implements `IShellLinkW`.
    unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }
}

/// A recent-file jump-list item's title: its own file name, falling back to the full path for the
/// rare file with none (matches `Open recent`'s own picker labels,
/// `tachyon_editor::picker::Editor::open_recent`).
fn recent_item_title(file: &Path) -> std::borrow::Cow<'_, str> {
    file.file_name().map_or_else(|| file.to_string_lossy(), |name| name.to_string_lossy())
}

/// A recent-file jump-list item: its target path and [`recent_item_title`]. No icon location:
/// Explorer resolves a document shell link's icon from its target's file association on its own.
fn shell_link_for_file(file: &Path) -> Result<IShellLinkW> {
    let link = new_shell_link()?;
    let path = HSTRING::from(file.as_os_str());
    // SAFETY: `path` outlives this call.
    unsafe { link.SetPath(&path) }?;
    set_title(&link, &recent_item_title(file))?;
    Ok(link)
}

/// The "New window" task: the executable with no arguments, which opens the same way
/// double-clicking it does - forwarded to the running instance as a fresh window
/// (`crates/tachyon/src/main.rs`'s single-instance claim) rather than starting a second process.
fn shell_link_for_new_window() -> Result<IShellLinkW> {
    let exe = std::env::current_exe()
        .map_err(|_| windows::core::Error::from(windows::Win32::Foundation::E_FAIL))?;
    let link = new_shell_link()?;
    let path = HSTRING::from(exe.as_os_str());
    // SAFETY: `path` outlives this call.
    unsafe { link.SetPath(&path) }?;
    // SAFETY: `path` outlives this call; the task's own icon comes from the executable itself,
    // unlike a recent-file item's (see `shell_link_for_file`), since a plain "new window" task has
    // no document to derive one from.
    unsafe { link.SetIconLocation(&path, 0) }?;
    set_title(&link, "New window")?;
    let description = HSTRING::from("Opens a new window");
    // SAFETY: `description` outlives this call.
    unsafe { link.SetDescription(&description) }?;
    Ok(link)
}

/// Sets a shell link's required `PKEY_Title` and commits it: Explorer will not display a
/// jump-list item added as an `IShellLink` without one (see the module doc comment).
fn set_title(link: &IShellLinkW, title: &str) -> Result<()> {
    let store: IPropertyStore = link.cast()?;
    let value = PROPVARIANT::from(title);
    // SAFETY: `value` is a valid `PROPVARIANT` for the duration of this call; `SetValue` copies
    // it rather than taking ownership, so `value`'s own `Drop` (which frees its `BSTR`) still runs
    // normally afterwards.
    unsafe { store.SetValue(&PKEY_Title, &value) }?;
    // SAFETY: single COM call on the live `store`.
    unsafe { store.Commit() }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{Mailbox, RECENT_LIMIT, recent_category_files, recent_item_title};

    #[test]
    fn mailbox_send_overwrites_a_value_no_reader_has_taken_yet() {
        let mailbox = Mailbox::new();
        mailbox.send(vec![1, 2, 3]);
        mailbox.send(vec![4]);
        assert_eq!(mailbox.recv(), vec![4], "the stale first value should be replaced, not queued");
    }

    #[test]
    fn mailbox_recv_blocks_until_a_value_arrives() {
        use std::sync::Arc;
        use std::time::Duration;

        let mailbox = Arc::new(Mailbox::<u32>::new());
        let reader = std::thread::spawn({
            let mailbox = Arc::clone(&mailbox);
            move || mailbox.recv()
        });
        std::thread::sleep(Duration::from_millis(20));
        assert!(!reader.is_finished(), "recv should still be waiting with nothing sent");
        mailbox.send(7);
        assert_eq!(reader.join().expect("reader thread"), 7);
    }

    #[test]
    fn recent_category_files_keeps_at_most_the_limit_newest_first() {
        let recent: Vec<PathBuf> =
            (0..RECENT_LIMIT + 5).map(|i| PathBuf::from(format!("{i}.md"))).collect();
        let kept = recent_category_files(&recent);
        assert_eq!(kept.len(), RECENT_LIMIT);
        assert_eq!(kept, &recent[..RECENT_LIMIT], "the newest entries (the front of the list) win");
    }

    #[test]
    fn recent_category_files_keeps_everything_under_the_limit() {
        let recent = vec![PathBuf::from("a.md"), PathBuf::from("b.md")];
        assert_eq!(recent_category_files(&recent), recent.as_slice());
    }

    #[test]
    fn recent_item_title_is_the_file_name() {
        assert_eq!(recent_item_title(Path::new("/home/user/notes.md")), "notes.md");
    }

    #[test]
    fn recent_item_title_falls_back_to_the_whole_path_with_no_file_name() {
        assert_eq!(recent_item_title(Path::new("/")), "/");
    }
}
