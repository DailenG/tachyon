use std::path::PathBuf;

use futures::StreamExt as _;
use gpui::{
    App, Bounds, Context, KeyBinding, SharedString, TitlebarOptions, WindowBounds, WindowHandle,
    WindowOptions, actions, prelude::*, px, size,
};
use tachyon_doc::Document;
use tachyon_editor::Editor;
use tachyon_platform::Listener;

use crate::cli::{self, Cli};
use crate::startup::Startup;

actions!(tachyon, [Quit, CloseWindow]);

const APP_ID: &str = "tachyon";
const SAMPLE: &str = include_str!("sample.md");

enum Source {
    Sample,
    Clipboard,
    File(PathBuf),
}

impl Source {
    fn from_cli(cli: Cli) -> Vec<Source> {
        let mut sources: Vec<Source> = cli.files.into_iter().map(Source::File).collect();
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

pub fn run(cli: Cli, listener: Option<Listener>, mut startup: Startup) {
    startup.set_report(cli.startup_report);
    gpui_platform::application().run(move |cx: &mut App| {
        startup.mark("platform_ready");
        cx.set_global(startup);

        cx.bind_keys([
            KeyBinding::new("secondary-q", Quit, None),
            KeyBinding::new("secondary-w", CloseWindow, None),
        ]);
        tachyon_editor::init(cx);
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.on_action(|_: &CloseWindow, cx| {
            if let Some(window) = cx.active_window() {
                let _ = window.update(cx, |_, window, _| window.remove_window());
            }
        });
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        if let Some(listener) = listener {
            serve_forwarded_launches(listener, cx);
        }

        let mut first = true;
        for source in Source::from_cli(cli) {
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
        let doc = cx
            .background_executor()
            .spawn(async move {
                let text = std::fs::read_to_string(&read_path)
                    .unwrap_or_else(|e| format!("Could not read {}: {e}\n", read_path.display()));
                Document::new(&text)
            })
            .await;
        editor.update(cx, |editor, cx| editor.set_document(doc, cx))
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
            cx.update(|cx| {
                for source in Source::from_cli(cli) {
                    if let Some(handle) = open_window(source, cx) {
                        let _ = handle.update(cx, |_, window, _| window.activate_window());
                    }
                }
                cx.activate(true);
            });
        }
    })
    .detach();
}
