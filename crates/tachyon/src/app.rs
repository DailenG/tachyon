use std::io::Write as _;
use std::path::PathBuf;
use std::time::Instant;

use futures::StreamExt as _;
use gpui::{
    App, Bounds, Context, Global, KeyBinding, QuitMode, SharedString, TitlebarOptions,
    WindowBounds, WindowHandle, WindowOptions, actions, prelude::*, px, size,
};
use tachyon_doc::Document;
use tachyon_editor::Editor;
use tachyon_platform::Listener;

use crate::cli::{self, Cli};
use crate::startup::Startup;

actions!(tachyon, [Quit]);

const APP_ID: &str = "tachyon";
const SAMPLE: &str = include_str!("sample.md");

enum Source {
    Sample,
    Clipboard,
    File(PathBuf),
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
            Source::Clipboard => "Clipboard - Tachyon".into(),
            Source::File(path) => {
                let name = path.file_name().unwrap_or(path.as_os_str()).to_string_lossy();
                format!("{name} - Tachyon").into()
            }
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
}

impl Global for Lifecycle {}

pub fn run(cli: Cli, listener: Option<Listener>, mut startup: Startup) {
    startup.set_report(cli.startup_report);
    // Resident only makes sense for the primary: it needs the channel that
    // later launches arrive on.
    let resident = cli.resident && listener.is_some();
    let report_launches = cli.report_launches && resident;
    // Window lifetime is ours to manage (resident mode); GPUI would otherwise
    // quit when the last window closes on Linux and Windows.
    gpui_platform::application().with_quit_mode(QuitMode::Explicit).run(move |cx: &mut App| {
        startup.mark("platform_ready");
        cx.set_global(startup);
        cx.set_global(Lifecycle { resident, quitting: false, report_launches });

        cx.bind_keys([KeyBinding::new("secondary-q", Quit, None)]);
        tachyon_editor::init(cx);
        if !tachyon_platform::has_native_prompts() {
            cx.set_prompt_builder(tachyon_editor::keyboard_prompt);
        }
        // Quitting closes every window through the editor, so unsaved
        // changes are asked about; the last window closing quits the app.
        cx.on_action(|_: &Quit, cx| {
            cx.global_mut::<Lifecycle>().quitting = true;
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

        if let Some(listener) = listener {
            serve_forwarded_launches(listener, cx);
            if report_launches {
                report_line("tachyon-ready");
            }
        }

        // A resident start with nothing to open (login autostart) stays
        // windowless until the first launch arrives.
        let sources = if resident && cli.files.is_empty() && !cli.paste {
            Vec::new()
        } else {
            Source::from_cli(cli)
        };
        let mut first = true;
        for source in sources {
            let Some(handle) = open_window(source, cx) else { continue };
            if std::mem::take(&mut first) {
                cx.global_mut::<Startup>().mark("window_open");
                let _ = handle.update(cx, |_, window, _| {
                    window.on_next_frame(|_, cx| {
                        if cx.global_mut::<Startup>().finish() {
                            cx.quit();
                        }
                    });
                });
            }
        }
        cx.activate(true);
    });
}

fn open_window(source: Source, cx: &mut App) -> Option<WindowHandle<Editor>> {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(900.), px(1000.)),
            cx,
        ))),
        titlebar: Some(TitlebarOptions { title: Some(source.title()), ..Default::default() }),
        app_id: Some(APP_ID.to_owned()),
        ..Default::default()
    };
    let result = cx.open_window(options, move |window, cx| {
        cx.new(|cx| match source {
            Source::Sample => Editor::new(SAMPLE, window, cx),
            Source::Clipboard => {
                let text =
                    cx.read_from_clipboard().and_then(|item| item.text()).unwrap_or_default();
                Editor::new(&text, window, cx)
            }
            Source::File(path) => {
                let editor = Editor::new("", window, cx);
                load_file(path, cx);
                editor
            }
        })
    });
    match result {
        Ok(handle) => Some(handle),
        Err(e) => {
            eprintln!("tachyon: failed to open window: {e:#}");
            None
        }
    }
}

/// Reads and parses the file on the background executor so window creation
/// never waits on disk I/O or a large parse, then hands the document over.
fn load_file(path: PathBuf, cx: &mut Context<Editor>) {
    cx.spawn(async move |editor, cx| {
        let read_path = path.clone();
        let loaded = cx
            .background_executor()
            .spawn(
                async move { std::fs::read_to_string(&read_path).map(|text| Document::new(&text)) },
            )
            .await;
        editor.update(cx, |editor, cx| match loaded {
            Ok(doc) => {
                editor.set_document(doc, cx);
                editor.set_file(path, cx);
            }
            // Not associated with the file: saving must not overwrite it
            // with the error message.
            Err(e) => editor.set_document(
                Document::new(&format!("Could not read {}: {e}\n", path.display())),
                cx,
            ),
        })
    })
    .detach();
}

/// A line for `cargo xtask bench-startup`, flushed immediately.
fn report_line(line: &str) {
    let mut stdout = std::io::stdout().lock();
    let _ = writeln!(stdout, "{line}");
    let _ = stdout.flush();
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
            cx.update(|cx| {
                let report = cx.global::<Lifecycle>().report_launches;
                for source in Source::from_cli(cli) {
                    if let Some(handle) = open_window(source, cx) {
                        let _ = handle.update(cx, |_, window, _| {
                            window.activate_window();
                            if report {
                                window.on_next_frame(move |window, _| {
                                    let us = received.elapsed().as_micros();
                                    report_line(&format!("tachyon-launch first_frame_us={us}"));
                                    window.remove_window();
                                });
                            }
                        });
                    }
                }
                cx.activate(true);
            });
        }
    })
    .detach();
}
