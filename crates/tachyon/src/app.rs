use std::io::Write as _;
use std::path::PathBuf;
use std::time::Instant;

use futures::StreamExt as _;
use gpui::{
    App, Bounds, Context, Global, KeyBinding, PathPromptOptions, Pixels, Point, QuitMode,
    SharedString, Size, TitlebarOptions, WindowAppearance, WindowBounds, WindowHandle,
    WindowOptions, actions, point, prelude::*, px, size,
};
use tachyon_doc::Document;
use tachyon_editor::{Editor, Theme};
use tachyon_platform::{Listener, TrayEvent};

use crate::cli::{self, Cli};
use crate::startup::Startup;
use crate::whats_new;

actions!(tachyon, [Quit, NewWindow, Open, OpenSettings, WhatsNew]);

const APP_ID: &str = "tachyon";

/// The Windows AppUserModelID: what groups Tachyon's windows under one taskbar icon and what the
/// jump list (`tachyon_platform::update_jump_list`) attaches to. Set once, below, through GPUI's
/// own `App::set_app_identity` rather than calling `SetCurrentProcessExplicitAppUserModelID`
/// directly from `tachyon-platform`: GPUI already owns this call (`gpui_windows`'s
/// `WindowsPlatform::set_app_identity`), skips it automatically when the process has MSIX package
/// identity (which supplies its own AUMID), and there is no separate per-window AUMID anywhere in
/// GPUI's Windows backend to duplicate or fall out of sync with - one process-wide call is the
/// whole mechanism. `SetCurrentProcessExplicitAppUserModelID` itself is documented as a single
/// in-process assignment (no I/O); this is not independently benchmarked here (no Windows
/// machine in this environment), but `cargo xtask bench-startup` on Windows should show no change
/// since the call happens after `startup.mark("platform_ready")`, nowhere near the paint budget.
const APP_USER_MODEL_ID: &str = "DailenG.Tachyon";
const SAMPLE: &str = include_str!("sample.md");

enum Source {
    Sample,
    /// An unsaved document backed up by an earlier session (hot exit).
    Restored(tachyon_editor::Restored),
    /// An empty, untitled document (New Window).
    Blank,
    Clipboard,
    File(PathBuf),
    /// This version's embedded CHANGELOG notes (see `whats_new`), opened as an ordinary
    /// untitled document: on demand (the command palette's "What's new" row) or once, after an
    /// update (`check_whats_new`).
    WhatsNew {
        version: String,
    },
}

impl Source {
    fn from_cli(cli: Cli) -> Vec<Source> {
        // Absolute paths: the file is read later on another thread, and the
        // path is shown and saved to independently of the working directory.
        let mut sources: Vec<Source> = cli
            .files
            .into_iter()
            .map(|file| Source::File(std::path::absolute(&file).unwrap_or(file)))
            .collect();
        if cli.paste {
            sources.push(Source::Clipboard);
        }
        if sources.is_empty() {
            sources.push(Source::Sample);
        }
        sources
    }

    fn title(&self) -> SharedString {
        match self {
            Source::Sample => "Tachyon".into(),
            Source::Restored(restored) => match &restored.file {
                Some(path) => Source::File(path.clone()).title(),
                None => "Tachyon".into(),
            },
            Source::Blank => "Tachyon".into(),
            Source::Clipboard => "Clipboard - Tachyon".into(),
            Source::File(path) => {
                let name = path.file_name().unwrap_or(path.as_os_str()).to_string_lossy();
                format!("{name} - Tachyon").into()
            }
            Source::WhatsNew { version } => format!("What's new in Tachyon {version}").into(),
        }
    }
}

/// How the process ends. A resident primary outlives its windows so later
/// launches skip process and GPU start-up (docs/adr/0004).
struct Lifecycle {
    resident: bool,
    /// Quit was requested: the last window closing ends the process even
    /// when resident.
    quitting: bool,
    report_launches: bool,
    /// This process claimed the single-instance channel (`main::claim_instance`'s
    /// `Claim::Primary`), as opposed to a standalone (`-n`) process: only the primary compares
    /// and records the What's new version (`check_whats_new_after_first_window`).
    is_primary: bool,
}

impl Global for Lifecycle {}

pub fn run(cli: Cli, listener: Option<Listener>, mut startup: Startup) {
    startup.set_report(cli.startup_report);
    // Whether this process claimed the single-instance channel: only it should compare and
    // record the What's new version (`check_whats_new`), never a standalone (`-n`) or forwarded
    // launch. Resident only makes sense for the primary too: it needs the same channel.
    let is_primary = listener.is_some();
    let resident = cli.resident() && is_primary;
    let report_launches = cli.report_launches && resident;
    let system_appearance = tachyon_platform::query_system_appearance();
    // Hot exit: the primary backs up unsaved documents and restores them at the next start.
    // Settings are read on a thread while GPUI starts; the first window waits briefly for them.
    let settings_file = tachyon_platform::config_dir().map(|dir| dir.join("settings.toml"));
    let settings_read = settings_file.clone().map(|file| {
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let text = std::fs::read_to_string(file).unwrap_or_default();
            let _ = sender.send(tachyon_editor::Settings::parse(&text).0);
        });
        receiver
    });
    let primary_backups = listener
        .as_ref()
        .and_then(|_| tachyon_platform::state_dir())
        .map(|dir| tachyon_editor::Backups::new(dir.join("backups").join(crate::instance_id())));
    // Window lifetime is ours to manage (resident mode); GPUI would otherwise
    // quit when the last window closes on Linux and Windows.
    gpui_platform::application().with_quit_mode(QuitMode::Explicit).run(move |cx: &mut App| {
        startup.mark("platform_ready");
        cx.set_global(startup);
        cx.set_global(Lifecycle { resident, quitting: false, report_launches, is_primary });
        cx.set_global(tachyon_editor::AppInfo {
            version: env!("CARGO_PKG_VERSION").into(),
            resident,
        });

        // Before any window opens or a notification could be posted (`App::set_app_identity`'s
        // own requirement); negligible cost on Linux and macOS (a string clone into the
        // platform's own state, not a no-op, but not I/O either), and skipped by GPUI itself on
        // an MSIX install (package identity already supplies the AUMID there).
        cx.set_app_identity(APP_USER_MODEL_ID, "Tachyon");

        cx.bind_keys([
            KeyBinding::new("secondary-q", Quit, None),
            KeyBinding::new("secondary-n", NewWindow, None),
            KeyBinding::new("secondary-o", Open, None),
            KeyBinding::new("secondary-,", OpenSettings, None),
        ]);
        cx.on_action(|_: &OpenSettings, cx| {
            let Some(file) = cx.try_global::<tachyon_editor::SettingsFile>().map(|f| f.0.clone())
            else {
                return;
            };
            if !file.exists() {
                if let Some(dir) = file.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let _ = std::fs::write(&file, tachyon_editor::DEFAULT_SETTINGS);
            }
            open_paths(vec![file], cx);
        });
        cx.on_action(|_: &NewWindow, cx| {
            show_window(Source::Blank, cx);
        });
        cx.on_action(|_: &WhatsNew, cx| {
            if !whats_new::NOTES.is_empty() {
                open_window(Source::WhatsNew { version: whats_new::current_version() }, cx);
            }
        });
        cx.on_action(|_: &Open, cx| {
            let chosen = cx.prompt_for_paths(PathPromptOptions {
                files: true,
                directories: false,
                multiple: true,
                prompt: None,
            });
            cx.spawn(async move |cx| {
                if let Ok(Ok(Some(paths))) = chosen.await {
                    cx.update(|cx| open_paths(paths, cx));
                }
            })
            .detach();
        });
        cx.set_global(tachyon_editor::OpenPaths(std::rc::Rc::new(open_paths)));
        tachyon_editor::init(cx);
        if let Some(dir) = tachyon_platform::state_dir() {
            let recent = dir.join(format!("recent-{}.txt", crate::instance_id()));
            cx.set_global(tachyon_editor::RecentFiles::new(recent));
        }
        cx.set_global(tachyon_editor::RecentFilesOs {
            // A private instance (`TACHYON_INSTANCE_ID` set, e.g. by a benchmark or a test) has
            // its own recent-files list (see the `RecentFiles::new` call just above), but the
            // jump list is scoped to the whole app, not to one instance: writing it here would
            // overwrite the real jump list with this instance's own, unrelated recent files.
            update_jump_list: if crate::is_default_instance() {
                tachyon_platform::update_jump_list
            } else {
                |_| {}
            },
            note_recently_used: tachyon_platform::note_recently_used,
        });
        cx.set_global(tachyon_editor::HtmlClipboard(|window, html, text| {
            tachyon_platform::write_clipboard_html(window, html, text)
        }));
        if let Some(read) = tachyon_platform::clipboard_text_reader() {
            cx.set_global(tachyon_editor::ClipboardReader(read));
        }
        if !tachyon_platform::has_native_prompts() {
            cx.set_prompt_builder(tachyon_editor::keyboard_prompt);
        }
        // Quitting closes every window through the editor, so unsaved
        // changes are asked about; the last window closing quits the app.
        cx.on_action(|_: &Quit, cx| {
            cx.global_mut::<Lifecycle>().quitting = true;
            // With backups, windows close without asking: their unsaved text comes back next time.
            let hot_exit = cx.try_global::<tachyon_editor::Settings>().is_none_or(|s| s.hot_exit);
            if hot_exit && cx.has_global::<tachyon_editor::Backups>() {
                cx.set_global(tachyon_editor::HotExit);
            }
            // Deferred: the action is dispatched from inside a window update,
            // and updating that window again from here would fail.
            cx.defer(|cx| {
                let windows = cx.windows();
                if windows.is_empty() {
                    cx.quit();
                }
                for window in windows {
                    let _ = window.update(cx, |_, window, cx| {
                        window.dispatch_action(Box::new(tachyon_editor::CloseWindow), cx)
                    });
                }
            });
        });
        cx.on_window_closed(|cx, _| {
            let lifecycle = cx.global::<Lifecycle>();
            if cx.windows().is_empty() && (!lifecycle.resident || lifecycle.quitting) {
                cx.quit();
            }
        })
        .detach();

        // Where windows learn the system appearance late, wait briefly for our own query so the
        // first frame is not painted in the wrong theme. A portal that is still starting up is
        // not waited for.
        if let Some(query) = system_appearance {
            if let Ok(dark) = query.recv_timeout(APPEARANCE_WAIT) {
                cx.set_global(tachyon_editor::AppearanceHint { dark });
            }
            cx.global_mut::<Startup>().mark("appearance");
        }

        if let Some(listener) = listener {
            serve_forwarded_launches(listener, cx);
            if resident {
                show_tray(cx);
            }
            prepare_ready_window(cx);
            if report_launches {
                report_line("tachyon-ready");
            }
        }

        let settings = settings_read
            .and_then(|read| read.recv_timeout(SETTINGS_WAIT).ok())
            .unwrap_or_default();
        let primary_backups = primary_backups.filter(|_| settings.hot_exit);
        cx.set_global(settings);
        if let Some(file) = settings_file {
            cx.set_global(tachyon_editor::SettingsFile(file));
        }
        // Documents left unsaved by the last Quit come back (primary instance only, so two
        // processes never restore the same backups).
        let restored: Vec<Source> = match &primary_backups {
            Some(backups) => backups.restore().into_iter().map(Source::Restored).collect(),
            None => Vec::new(),
        };
        if let Some(backups) = primary_backups {
            cx.set_global(backups);
        }
        // A background start with nothing to open (login autostart) stays
        // windowless until the first launch arrives, which brings the restored documents along.
        let about = cli.about;
        let sources = if resident && cli.background && cli.opens_nothing() {
            cx.set_global(PendingRestore(restored));
            // No window will open to carry the theme to the tray's context menu (Windows) the
            // way `open_window` and `prepare_ready_window` do, so it is resolved here instead,
            // the one time this path is windowless.
            tachyon_platform::set_popup_menu_dark(resolved_dark(cx));
            Vec::new()
        } else if about {
            // `--about` shows the About window, not a document window; restored documents (hot
            // exit) still come back, the same as any other launch.
            restored
        } else if !restored.is_empty() && cli.opens_nothing() {
            restored
        } else {
            restored.into_iter().chain(Source::from_cli(cli)).collect()
        };
        let mut first = true;
        for source in sources {
            let Some(handle) = open_window(source, cx) else { continue };
            if std::mem::take(&mut first) {
                cx.global_mut::<Startup>().mark("window_open");
                // Set synchronously, before any forwarded launch reaching `show_window` could
                // possibly race it (both run on this same executor, but only one at a time): a
                // second `check_whats_new` for the same process would be harmless (idempotent
                // once the version file already matches), just redundant.
                if is_primary {
                    cx.set_global(WhatsNewChecked);
                }
                let _ = handle.update(cx, |_, window, _| {
                    window.on_next_frame(move |_, cx| {
                        if cx.global_mut::<Startup>().finish() {
                            cx.quit();
                        } else if is_primary {
                            check_whats_new(cx);
                        }
                    });
                });
            }
        }
        if about {
            tachyon_editor::open_about(cx);
        }
        cx.activate(true);
    });
}

const WINDOW_SIZE: Size<Pixels> = size(px(900.), px(1000.));

/// Room kept free for the frame and title bar around the window's content. The bounds GPUI takes
/// are the client area, and on Windows the frame is added outside them. A 1000 px window on a
/// 1080p screen at 125 % (816 px of work area) otherwise opened with its title bar above the
/// screen and its bottom edge under the taskbar. The Windows 11 frame is about 31 px.
const FRAME_ALLOWANCE: Pixels = px(48.);

/// One title-bar height: how far down and right each new window cascades from the last one.
const CASCADE_STEP: Pixels = px(32.);

/// Windows opened so far (real ones from `open_window`, and a hidden prepared-ahead one from
/// `prepare_ready_window`), oldest first, for `initial_bounds`'s cascade: the next window offsets
/// from the most recently opened one that is still open. Never pruned on close by itself; a
/// closed handle is dropped the next time `last_window_origin` walks past it looking for a
/// survivor, so closing windows costs nothing extra.
struct WindowCascade(Vec<WindowHandle<Editor>>);

impl Global for WindowCascade {}

/// Remembers `handle` as the most recently opened window, for the next one's cascade origin.
fn track_window(handle: WindowHandle<Editor>, cx: &mut App) {
    if cx.has_global::<WindowCascade>() {
        cx.global_mut::<WindowCascade>().0.push(handle);
    } else {
        cx.set_global(WindowCascade(vec![handle]));
    }
}

/// The screen position of the most recently opened window that is still open, or `None` if
/// there is not one (nothing has opened yet, or every window opened so far has since closed).
fn last_window_origin(cx: &mut App) -> Option<Point<Pixels>> {
    loop {
        let handle = *cx.try_global::<WindowCascade>()?.0.last()?;
        if let Ok(origin) = handle.update(cx, |_, window, _| window.bounds().origin) {
            return Some(origin);
        }
        cx.global_mut::<WindowCascade>().0.pop();
    }
}

/// Where a new window of `size` should go: `CASCADE_STEP` down and right from `last` (the most
/// recently opened window still open), or `base` - the centred position `initial_bounds` also
/// falls back to for the very first window - if `last` is `None` or the offset would put the
/// window outside `work_area` (it wraps back to `base` rather than walking further off-screen).
/// Pure and independent of any live display or window, so it is unit-tested directly
/// (`tests::cascade_origin_*`) without opening one.
fn cascade_origin(
    last: Option<Point<Pixels>>,
    base: Point<Pixels>,
    size: Size<Pixels>,
    work_area: Bounds<Pixels>,
) -> Point<Pixels> {
    let Some(last) = last else { return base };
    let next = point(last.x + CASCADE_STEP, last.y + CASCADE_STEP);
    let fits = next.x >= work_area.origin.x
        && next.y >= work_area.origin.y
        && next.x + size.width <= work_area.origin.x + work_area.size.width
        && next.y + size.height <= work_area.origin.y + work_area.size.height;
    if fits { next } else { base }
}

/// `WINDOW_SIZE`, shrunk to fit with its frame in the primary display's work area. The very
/// first window (nothing yet tracked in `WindowCascade`, so `last_window_origin` is a plain
/// global lookup - no cost added before its first frame) is centred there; every later one
/// cascades from the last still-open one (`cascade_origin`).
fn initial_bounds(cx: &mut App) -> Bounds<Pixels> {
    let Some(display) = cx.primary_display() else {
        return Bounds::centered(None, WINDOW_SIZE, cx);
    };
    let visible = display.visible_bounds();
    let fitted = size(
        WINDOW_SIZE.width.min(visible.size.width - FRAME_ALLOWANCE),
        WINDOW_SIZE.height.min(visible.size.height - FRAME_ALLOWANCE),
    );
    let centered = Bounds::centered_at(visible.center(), fitted);
    let last = last_window_origin(cx);
    Bounds { origin: cascade_origin(last, centered.origin, fitted, visible), size: fitted }
}

/// How long start-up waits for the settings file to be read (it is small and local).
const SETTINGS_WAIT: std::time::Duration = std::time::Duration::from_millis(15);

/// How long start-up waits for the system appearance (see `query_system_appearance`).
const APPEARANCE_WAIT: std::time::Duration = std::time::Duration::from_millis(15);

fn window_options(title: SharedString, show: bool, bounds: Bounds<Pixels>) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions { title: Some(title), ..Default::default() }),
        app_id: Some(APP_ID.to_owned()),
        show,
        focus: show,
        ..Default::default()
    }
}

/// The document `source` opens with; files are loaded in the background. A file's window shows
/// immediately with a placeholder (never a frozen or empty-looking window while a huge file
/// streams in off the UI thread; see `load_file`).
fn initial_document(source: &Source, cx: &App) -> Document {
    match source {
        Source::Sample => Document::new(SAMPLE),
        Source::Clipboard => Document::new(
            &cx.read_from_clipboard().and_then(|item| item.text()).unwrap_or_default(),
        ),
        Source::File(path) => {
            let name = path.file_name().unwrap_or(path.as_os_str()).to_string_lossy();
            Document::new(&format!("Loading {name}...\n"))
        }
        Source::Blank | Source::Restored(_) => Document::new(""),
        Source::WhatsNew { .. } => Document::new(whats_new::NOTES),
    }
}

fn open_window(source: Source, cx: &mut App) -> Option<WindowHandle<Editor>> {
    let title = source.title();
    let bounds = initial_bounds(cx);
    let options = window_options(title.clone(), true, bounds);
    let result = cx.open_window(options, move |window, cx| {
        tachyon_platform::set_window_icon(window);
        // Set before the window's first frame paints, so DWM never shows the OS dark-mode
        // setting's colour for an instant: Tachyon keeps the native title bar, and it should
        // follow the theme the editor is about to render with, not the system's. The tray's
        // context menu (Windows) follows the same value, so a new window's theme choice also
        // becomes the menu's next time it shows.
        let dark = Theme::for_window(window, cx).dark;
        tachyon_platform::set_title_bar_dark(window, dark);
        tachyon_platform::set_popup_menu_dark(dark);
        // `Editor::sync_title` otherwise overwrites this window's title every frame with the
        // usual file-name/scratch-buffer rule, which has no notion of "What's new".
        let fixed_title = matches!(source, Source::WhatsNew { .. }).then(|| title.to_string());
        cx.new(|cx| {
            let mut editor = Editor::with_document(initial_document(&source, cx), window, cx);
            if let Some(fixed_title) = fixed_title {
                editor.set_title_override(fixed_title);
            }
            fill(&mut editor, source, cx);
            editor
        })
    });
    match result {
        Ok(handle) => {
            track_window(handle, cx);
            Some(handle)
        }
        Err(e) => {
            eprintln!("tachyon: failed to open window: {e:#}");
            None
        }
    }
}

/// Reads and parses the file on the background executor so window creation
/// never waits on disk I/O or a large parse, then hands the document over.
/// What a new editor gets beyond its initial document: a file loads in the background, a backup is
/// adopted.
fn fill(editor: &mut Editor, source: Source, cx: &mut Context<Editor>) {
    match source {
        Source::File(path) => load_file(path, cx),
        Source::Restored(restored) => editor.adopt_backup(restored, cx),
        Source::Sample | Source::Blank | Source::Clipboard | Source::WhatsNew { .. } => {}
    }
}

/// Documents restored at a background start, opened with the first launch.
struct PendingRestore(Vec<Source>);

impl Global for PendingRestore {}

fn take_pending_restore(cx: &mut App) -> Vec<Source> {
    if cx.has_global::<PendingRestore>() {
        cx.remove_global::<PendingRestore>().0
    } else {
        Vec::new()
    }
}

/// Reads and builds the document (Markdown or plain text, chosen and size-checked by
/// `tachyon_editor::load_document`) on the background executor, so opening a huge file never
/// blocks window creation or a frame; the window already shows the "Loading ..." placeholder
/// above until this replaces it.
fn load_file(path: PathBuf, cx: &mut Context<Editor>) {
    cx.spawn(async move |editor, cx| {
        let read_path = path.clone();
        let outcome = cx
            .background_executor()
            .spawn(async move { tachyon_editor::load_document(&read_path) })
            .await;
        editor.update(cx, |editor, cx| match outcome {
            Ok(tachyon_editor::LoadOutcome::Loaded(loaded)) => {
                editor.set_loaded(*loaded, cx);
                editor.set_file(path, cx);
            }
            // Neither is associated with the file: saving must not overwrite it with the
            // message, and Save As still prompts as it would for a fresh scratch buffer.
            Ok(tachyon_editor::LoadOutcome::Refused(message)) => {
                editor.set_document(Document::new(&format!("{message}\n")), cx);
            }
            Err(e) => editor.set_document(
                Document::new(&format!("Could not read {}: {e}\n", path.display())),
                cx,
            ),
        })
    })
    .detach();
}

/// A resident instance's window created in advance and hidden, so a launch
/// only fills and shows it: creating a window costs 40-60 ms on Windows, and
/// the ready window also already has its final size, which saves resizing
/// its render targets while it is shown (docs/adr/0004).
struct ReadyWindow(Option<WindowHandle<Editor>>);

impl Global for ReadyWindow {}

/// Prepared after a launch's first frame, so the work never competes with it.
const READY_WINDOW_DELAY: std::time::Duration = std::time::Duration::from_millis(100);

fn ready_windows_enabled(cx: &App) -> bool {
    let lifecycle = cx.global::<Lifecycle>();
    lifecycle.resident && !lifecycle.quitting && tachyon_platform::keeps_hidden_windows_hidden()
}

fn has_ready_window(cx: &App) -> bool {
    cx.try_global::<ReadyWindow>().is_some_and(|ready| ready.0.is_some())
}

fn prepare_ready_window(cx: &mut App) {
    if !ready_windows_enabled(cx) || has_ready_window(cx) {
        return;
    }
    cx.spawn(async move |cx| {
        cx.background_executor().timer(READY_WINDOW_DELAY).await;
        cx.update(|cx| {
            if !ready_windows_enabled(cx) || has_ready_window(cx) {
                return;
            }
            let bounds = initial_bounds(cx);
            let options = window_options("Tachyon".into(), false, bounds);
            match cx.open_window(options, |window, cx| cx.new(|cx| Editor::new("", window, cx))) {
                Ok(handle) => {
                    // A hidden window only gets its final size when shown,
                    // and resizing the render targets then takes ≈ 20 ms.
                    // Without DWM's open animation the window appears as soon as it is
                    // shown, and its first frame is drawn sooner (docs/adr/0004). The size is
                    // the fitted one, so a small screen's work area still holds the window.
                    let _ = handle.update(cx, |_, window, cx| {
                        window.resize(bounds.size);
                        tachyon_platform::disable_window_transitions(window);
                        tachyon_platform::set_window_icon(window);
                        // Same reasoning as in `open_window`: set before this window is ever
                        // shown, so its title bar never flips visibly to match Tachyon's theme.
                        let dark = Theme::for_window(window, cx).dark;
                        tachyon_platform::set_title_bar_dark(window, dark);
                        tachyon_platform::set_popup_menu_dark(dark);
                    });
                    track_window(handle, cx);
                    cx.set_global(ReadyWindow(Some(handle)));
                }
                Err(e) => eprintln!("tachyon: could not prepare a window ({e:#})"),
            }
        });
    })
    .detach();
}

/// The ready window, filled with `source`; `source` back if there is none.
fn take_ready_window(source: Source, cx: &mut App) -> Result<WindowHandle<Editor>, Source> {
    if !ready_windows_enabled(cx) || !has_ready_window(cx) {
        return Err(source);
    }
    let Some(handle) = cx.global_mut::<ReadyWindow>().0.take() else { return Err(source) };
    // Closed behind our back (e.g. by the OS): open a window normally.
    if handle.update(cx, |_, _, _| ()).is_err() {
        return Err(source);
    }
    let doc = initial_document(&source, cx);
    let title = source.title();
    let _ = handle.update(cx, |editor, window, cx| {
        window.set_window_title(&title);
        editor.set_document(doc, cx);
        fill(editor, source, cx);
    });
    Ok(handle)
}

/// Opens `source` in the ready window if there is one, else in a new window, and brings it to the
/// front.
fn show_window(source: Source, cx: &mut App) -> Option<WindowHandle<Editor>> {
    let handle = match take_ready_window(source, cx) {
        Ok(handle) => Some(handle),
        Err(source) => open_window(source, cx),
    }?;
    // Covers the one path the initial-sources loop in `run` cannot: a `--background` primary
    // with nothing to open (login autostart) stays windowless until a forwarded launch (which
    // reaches this same function) finally opens one.
    check_whats_new_after_first_window(&handle, cx);
    let _ = handle.update(cx, |_, window, _| window.activate_window());
    let prepare = handle.update(cx, |_, window, _| {
        window.on_next_frame(|_, cx| prepare_ready_window(cx));
    });
    prepare.ok().map(|()| handle)
}

/// Opens each path in its own window (Open dialog, files dropped onto a window).
fn open_paths(paths: Vec<PathBuf>, cx: &mut App) {
    for path in paths.into_iter().filter(|path| !path.is_dir()) {
        show_window(Source::File(std::path::absolute(&path).unwrap_or(path)), cx);
    }
    cx.activate(true);
}

/// A line for `cargo xtask bench-startup`, flushed immediately.
fn report_line(line: &str) {
    let mut stdout = std::io::stdout().lock();
    let _ = writeln!(stdout, "{line}");
    let _ = stdout.flush();
}

/// Marker: `check_whats_new` has already been scheduled once for this process (see
/// `check_whats_new_after_first_window`, and the initial-sources loop in `run` which sets this
/// itself for the common case).
struct WhatsNewChecked;

impl Global for WhatsNewChecked {}

/// Schedules `check_whats_new` after `handle`'s next frame, but only the first time this is
/// called for the whole process (`WhatsNewChecked`) and only for the primary instance
/// (`Lifecycle::is_primary`). Exists for `show_window`: a `--background` primary with nothing to
/// open (login autostart) has no window for the initial-sources loop in `run` to hook, so its
/// first real window - opened later by a forwarded launch, always through `show_window` - has to
/// carry this instead.
fn check_whats_new_after_first_window(handle: &WindowHandle<Editor>, cx: &mut App) {
    if !cx.global::<Lifecycle>().is_primary || cx.has_global::<WhatsNewChecked>() {
        return;
    }
    cx.set_global(WhatsNewChecked);
    let _ = handle.update(cx, |_, window, _| {
        window.on_next_frame(|_, cx| check_whats_new(cx));
    });
}

/// After the first window's first frame, primary instances only (`check_whats_new`'s callers
/// gate on `is_primary`/`Lifecycle::is_primary`; a forwarded launch never reaches `run` at all,
/// and `--quit`/`--status` never reach `run` either): reads the version last seen against this
/// launch's (`whats_new::current_version`) off the UI thread, updates the record if it is stale,
/// and, if it changed, the record was written successfully, the `whats_new` setting allows it,
/// and there is anything embedded to show (`whats_new::should_open_window`), opens the notes as
/// a new window. A record that failed to write (an unwritable state directory) is logged once
/// and never opens a window itself - showing it anyway would reopen it on every single launch,
/// since there would be no way to remember it was already shown.
fn check_whats_new(cx: &mut App) {
    let Some(state_dir) = tachyon_platform::state_dir() else { return };
    let path = state_dir.join(format!("whats-new-{}.txt", crate::instance_id()));
    let current = whats_new::current_version();
    let enabled = cx.try_global::<tachyon_editor::Settings>().is_none_or(|s| s.whats_new);
    let has_notes = !whats_new::NOTES.is_empty();
    cx.spawn(async move |cx| {
        let record_version = current.clone();
        let log_path = path.clone();
        let (check, recorded) = cx
            .background_executor()
            .spawn(async move { whats_new::read_and_record(&path, &record_version) })
            .await;
        if !recorded {
            eprintln!(
                "tachyon: could not record the What's new version at {}",
                log_path.display()
            );
            return;
        }
        if whats_new::should_open_window(check, enabled, has_notes) {
            cx.update(|cx| {
                open_window(Source::WhatsNew { version: current }, cx);
            });
        }
    })
    .detach();
}

/// The resident instance's tray icon (Windows), removed when the app quits.
struct TrayIcon {
    _tray: tachyon_platform::Tray,
}

impl Global for TrayIcon {}

/// Whether Tachyon's resolved theme is dark, by the same rule `Theme::for_window` applies, but
/// usable with no window: a resident instance can stay windowless in the background
/// (`--background` with nothing to open) until the first launch arrives, and the tray's context
/// menu (Windows) can show before that. Prefers the `AppearanceHint` set before GPUI's first
/// window exists, else asks the platform directly (`App::window_appearance`, unlike
/// `Window::appearance`, needs no window), then applies the `theme` setting (`Settings::dark`).
fn resolved_dark(cx: &App) -> bool {
    let hint = cx.try_global::<tachyon_editor::AppearanceHint>().map(|hint| hint.dark);
    let system = hint.unwrap_or_else(|| {
        matches!(cx.window_appearance(), WindowAppearance::Dark | WindowAppearance::VibrantDark)
    });
    cx.try_global::<tachyon_editor::Settings>().map_or(system, |settings| settings.dark(system))
}

/// Shows the tray icon: clicking it opens a window, its menu opens a window or quits.
fn show_tray(cx: &mut App) {
    let (tx, mut rx) = futures::channel::mpsc::unbounded::<TrayEvent>();
    let Some(tray) = tachyon_platform::Tray::show("Tachyon", move |event| {
        let _ = tx.unbounded_send(event);
    }) else {
        return;
    };
    cx.set_global(TrayIcon { _tray: tray });
    // Without this the process would exit with the icon still shown until the pointer passes
    // over it.
    cx.on_app_quit(|cx| {
        if cx.has_global::<TrayIcon>() {
            cx.remove_global::<TrayIcon>();
        }
        async {}
    })
    .detach();
    cx.spawn(async move |cx| {
        while let Some(event) = rx.next().await {
            cx.update(|cx| match event {
                TrayEvent::Open => {
                    let restored = take_pending_restore(cx);
                    if restored.is_empty() {
                        show_window(Source::Blank, cx);
                    }
                    for source in restored {
                        show_window(source, cx);
                    }
                    cx.activate(true);
                }
                TrayEvent::About => {
                    tachyon_editor::open_about(cx);
                    cx.activate(true);
                }
                TrayEvent::Quit => cx.dispatch_action(&Quit),
            });
        }
    })
    .detach();
}

/// Moves launches forwarded by secondary processes from the listener thread
/// onto the main thread and opens them there.
fn serve_forwarded_launches(listener: Listener, cx: &mut App) {
    let (tx, mut rx) = futures::channel::mpsc::unbounded::<Vec<String>>();
    // Accepted once queued for the main thread; fails only while quitting.
    listener.spawn(move |args| tx.unbounded_send(args).is_ok());
    cx.spawn(async move |cx| {
        while let Some(args) = rx.next().await {
            let Ok(cli::Command::Run(cli)) = cli::parse(args) else { continue };
            let received = Instant::now();
            if cli.quit {
                cx.update(|cx| cx.dispatch_action(&Quit));
                continue;
            }
            if cli.about {
                cx.update(|cx| {
                    tachyon_editor::open_about(cx);
                    cx.activate(true);
                });
                continue;
            }
            cx.update(|cx| {
                let report = cx.global::<Lifecycle>().report_launches;
                let restored = take_pending_restore(cx);
                let sources = if !restored.is_empty() && cli.opens_nothing() {
                    restored
                } else {
                    restored.into_iter().chain(Source::from_cli(cli)).collect()
                };
                for source in sources {
                    if let Some(handle) = show_window(source, cx)
                        && report
                    {
                        let _ = handle.update(cx, |_, window, _| {
                            window.on_next_frame(move |window, _| {
                                let us = received.elapsed().as_micros();
                                report_line(&format!("tachyon-launch first_frame_us={us}"));
                                window.remove_window();
                            });
                        });
                    }
                }
                cx.activate(true);
            });
        }
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORK_AREA: Bounds<Pixels> = Bounds {
        origin: point(px(0.), px(0.)),
        size: Size { width: px(1920.), height: px(1080.) },
    };
    const WIN_SIZE: Size<Pixels> = size(px(900.), px(1000.));
    const BASE: Point<Pixels> = point(px(510.), px(40.));

    #[test]
    fn cascade_origin_centres_the_very_first_window() {
        assert_eq!(cascade_origin(None, BASE, WIN_SIZE, WORK_AREA), BASE);
    }

    #[test]
    fn cascade_origin_steps_down_and_right_from_the_last_window() {
        let last = point(px(100.), px(40.));
        let expected = point(last.x + CASCADE_STEP, last.y + CASCADE_STEP);
        assert_eq!(cascade_origin(Some(last), BASE, WIN_SIZE, WORK_AREA), expected);
    }

    #[test]
    fn cascade_origin_wraps_back_to_the_centred_position_past_the_work_area_edge() {
        // One step further would push the 900x1000 window past the 1920x1080 work area.
        let last = point(px(1000.), px(50.));
        assert_eq!(cascade_origin(Some(last), BASE, WIN_SIZE, WORK_AREA), BASE);

        let last = point(px(100.), px(50.));
        assert_eq!(cascade_origin(Some(last), BASE, WIN_SIZE, WORK_AREA), BASE);
    }

    #[test]
    fn cascade_origin_wraps_above_a_work_area_that_does_not_start_at_the_origin() {
        // A work area offset by a taskbar or a second monitor to its left/above.
        let work_area = Bounds {
            origin: point(px(1920.), px(40.)),
            size: Size { width: px(1920.), height: px(1040.) },
        };
        let last = point(px(1920.), px(40.));
        let expected = point(last.x + CASCADE_STEP, last.y + CASCADE_STEP);
        assert_eq!(cascade_origin(Some(last), BASE, WIN_SIZE, work_area), expected);

        // Cascading from well left of the work area's own top-left wraps, rather than landing
        // outside it.
        let last = point(px(1000.), px(40.));
        assert_eq!(cascade_origin(Some(last), BASE, WIN_SIZE, work_area), BASE);
    }
}
