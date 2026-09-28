//! The About window: a separate top-level window, created only on request and reused if already
//! open, never on the startup path. Shows the version, the owner's signature artwork
//! (`assets/brand/`, approved for 1.0), and environment facts useful for support requests.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{
    App, Bounds, ClipboardItem, Context, FocusHandle, Focusable, FontWeight, Global, Image,
    ImageFormat, IntoElement, KeyDownEvent, ObjectFit, Pixels, Render, SharedString, Size,
    StyledImage as _, TitlebarOptions, Window, WindowBounds, WindowHandle, WindowOptions, actions,
    div, img, prelude::*, px, size,
};

use crate::backup::Backups;
use crate::editor::CloseWindow;
use crate::settings::SettingsFile;
use crate::theme::Theme;

actions!(about, [About]);

/// The window's app id (X11/Wayland grouping): the same value `tachyon::app::APP_ID` uses. A
/// literal here rather than a shared constant, since a compositor only uses it to group windows
/// and `tachyon-editor` does not otherwise depend on the binary crate.
const APP_ID: &str = "tachyon";

const ABOUT_SIZE: Size<Pixels> = size(px(440.), px(720.));

/// The owner's signature, its original dark-navy ink, for the light theme.
const SIGNATURE_DARK: &[u8] = include_bytes!("../../../assets/brand/signature.png");
/// The same signature recoloured to the dark theme's `text.primary` (alpha untouched); see
/// `assets/brand/README.md` for how it was prepared.
const SIGNATURE_LIGHT: &[u8] = include_bytes!("../../../assets/brand/signature-light.png");

const SUPPORT_LABEL: &str = "github.com/DailenG/tachyon/issues";
const SUPPORT_URL: &str = "https://github.com/DailenG/tachyon/issues";
const WEBSITE_LABEL: &str = "daileng.github.io/tachyon";
const WEBSITE_URL: &str = "https://daileng.github.io/tachyon";

/// Facts about this process that only the binary which started it knows: its own crate version
/// (`tachyon`, not `tachyon-editor`'s) and whether it stays resident after its last window
/// closes. Set once at start-up so [`About`] - dispatched with no other context, from the tray,
/// the command palette, or `--about` - and the window it opens can read them without every
/// caller threading them through. Not set by tests that never run the application's start-up;
/// [`AboutView::new`] falls back to sensible defaults then.
pub struct AppInfo {
    pub version: SharedString,
    pub resident: bool,
}

impl Global for AppInfo {}

/// The one About window, if it is open, so a second request focuses it instead of opening
/// another.
#[derive(Default)]
struct AboutWindowHandle(Option<WindowHandle<AboutView>>);

impl Global for AboutWindowHandle {}

/// Opens the About window, or brings the existing one to the front if it is already open.
pub fn open_about(cx: &mut App) -> Option<WindowHandle<AboutView>> {
    if let Some(handle) = cx.try_global::<AboutWindowHandle>().and_then(|h| h.0)
        && handle.update(cx, |_, window, _| window.activate_window()).is_ok()
    {
        return Some(handle);
    }
    let handle = create_window(cx)?;
    cx.set_global(AboutWindowHandle(Some(handle)));
    Some(handle)
}

fn create_window(cx: &mut App) -> Option<WindowHandle<AboutView>> {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, ABOUT_SIZE, cx))),
        titlebar: Some(TitlebarOptions {
            title: Some("About Tachyon".into()),
            ..Default::default()
        }),
        app_id: Some(APP_ID.to_owned()),
        is_resizable: false,
        show: true,
        focus: true,
        ..Default::default()
    };
    let result = cx.open_window(options, |window, cx| {
        tachyon_platform::set_window_icon(window);
        let theme = Theme::for_window(window, cx);
        tachyon_platform::set_title_bar_dark(window, theme.dark);
        cx.new(|cx| AboutView::new(theme, window, cx))
    });
    match result {
        Ok(handle) => Some(handle),
        Err(e) => {
            eprintln!("tachyon: failed to open the About window: {e:#}");
            None
        }
    }
}

/// The About window's content.
pub struct AboutView {
    theme: Theme,
    focus: FocusHandle,
    signature: Arc<Image>,
    version: SharedString,
    packaged_version: Option<String>,
    resident: bool,
    platform: String,
    settings_path: Option<PathBuf>,
    backups_dir: Option<PathBuf>,
}

impl AboutView {
    fn new(theme: Theme, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let signature_bytes = if theme.dark { SIGNATURE_LIGHT } else { SIGNATURE_DARK };
        let (version, resident) = cx
            .try_global::<AppInfo>()
            .map(|info| (info.version.clone(), info.resident))
            .unwrap_or_else(|| (env!("CARGO_PKG_VERSION").into(), false));
        AboutView {
            theme,
            focus,
            signature: Arc::new(Image::from_bytes(ImageFormat::Png, signature_bytes.to_vec())),
            version,
            packaged_version: tachyon_platform::packaged_version(),
            resident,
            platform: tachyon_platform::os_version(),
            settings_path: cx.try_global::<SettingsFile>().map(|f| f.0.clone()),
            backups_dir: cx.try_global::<Backups>().map(|b| b.dir().to_path_buf()),
        }
    }

    fn install(&self) -> &'static str {
        if self.packaged_version.is_some() { "Installed package" } else { "Portable" }
    }

    fn mode(&self) -> &'static str {
        if self.resident { "Resident" } else { "Standalone" }
    }

    /// "Stable"/"Nightly" come from the packaged version's build number (the fourth part of
    /// `X.Y.Z.B`; see `docs/adr/0007-versions-and-release-channels.md`): `B = 0` is a stable
    /// release, anything else a nightly build. A portable build checks nothing, so it says so.
    fn updates(&self) -> &'static str {
        match &self.packaged_version {
            Some(version) => {
                let build: Option<u32> = version.rsplit('.').next().and_then(|b| b.parse().ok());
                if build == Some(0) { "Stable, automatic" } else { "Nightly, automatic" }
            }
            None => "Manual",
        }
    }

    fn path_text(path: Option<&PathBuf>) -> String {
        path.map_or_else(|| "not available".to_owned(), |p| p.display().to_string())
    }

    /// The Environment table as plain text, for the "Copy details" button and support requests.
    pub(crate) fn environment_text(&self) -> String {
        let mut text = format!("Tachyon {}\n", self.version);
        if let Some(package) = &self.packaged_version {
            text.push_str(&format!("Package {package}\n"));
        }
        text.push_str(&format!(
            "Platform: {}\nInstall: {}\nMode: {}\nUpdates: {}\nSettings: {}\nBackups: {}\n\
             Website: {WEBSITE_URL}\n",
            self.platform,
            self.install(),
            self.mode(),
            self.updates(),
            Self::path_text(self.settings_path.as_ref()),
            Self::path_text(self.backups_dir.as_ref()),
        ));
        text
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.key.as_str() == "escape" {
            window.remove_window();
            cx.stop_propagation();
        }
    }

    fn close_window(&mut self, _: &CloseWindow, window: &mut Window, _: &mut Context<Self>) {
        window.remove_window();
    }

    fn copy_details(&mut self, _: &gpui::ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(self.environment_text()));
    }
}

impl Focusable for AboutView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for AboutView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme.clone();

        fn row(theme: &Theme, label_text: &'static str, value: impl IntoElement) -> gpui::Div {
            let label =
                div().w(px(84.)).flex_shrink_0().text_color(theme.text.muted).child(label_text);
            div().w_full().flex().flex_row().gap_2().child(label).child(value)
        }
        let value = |theme: &Theme, text: String| div().text_color(theme.text.primary).child(text);
        let link = |theme: &Theme, id: &'static str, text: String| {
            div().id(id).cursor_pointer().text_color(theme.text.link).underline().child(text)
        };
        // A path is often wider than the window: shown with `~` for the home folder and cut in
        // the middle, so both the drive or home and the file name stay visible. Clicking still
        // reveals the real path, and Copy details has it in full.
        let path_cell = |theme: &Theme, id: &'static str, path: Option<PathBuf>| match path {
            Some(path) => link(theme, id, display_path(&path))
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis_middle()
                .on_click(cx.listener(move |_, _, _, cx| cx.reveal_path(&path)))
                .into_any_element(),
            None => value(theme, "not available".to_owned()).into_any_element(),
        };

        let package_line = self
            .packaged_version
            .clone()
            .map(|package| div().text_color(theme.text.muted).child(format!("Package {package}")));

        div()
            .size_full()
            .bg(theme.surface.canvas)
            .text_color(theme.text.primary)
            .font_family(theme.text_font.clone())
            .text_size(theme.text_size)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            .on_action(cx.listener(Self::close_window))
            .flex()
            .flex_col()
            .items_center()
            .p_6()
            .gap_2()
            .child(
                img(self.signature.clone())
                    .w(px(200.))
                    .h(px(100.))
                    .object_fit(ObjectFit::ScaleDown),
            )
            .child(
                div()
                    .font_weight(FontWeight::BOLD)
                    .text_size(theme.heading_sizes[0])
                    .child("Tachyon"),
            )
            .child(div().text_color(theme.text.muted).child(format!("Version {}", self.version)))
            .children(package_line)
            .child(div().text_color(theme.text.muted).child("Created by Dailen Gunter"))
            .child(
                link(&theme, "support-link", SUPPORT_LABEL.to_owned())
                    .on_click(cx.listener(|_, _, _, cx| cx.open_url(SUPPORT_URL))),
            )
            .child(div().w_full().h(px(1.)).my_2().bg(theme.border.subtle))
            .child(
                div()
                    .w_full()
                    .font_weight(FontWeight::BOLD)
                    .text_color(theme.text.muted)
                    .child("Environment"),
            )
            .child(
                div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(row(&theme, "Platform", value(&theme, self.platform.clone())))
                    .child(row(&theme, "Install", value(&theme, self.install().to_owned())))
                    .child(row(&theme, "Mode", value(&theme, self.mode().to_owned())))
                    .child(row(&theme, "Updates", value(&theme, self.updates().to_owned())))
                    .child(row(
                        &theme,
                        "Settings",
                        path_cell(&theme, "settings-path", self.settings_path.clone()),
                    ))
                    .child(row(
                        &theme,
                        "Backups",
                        path_cell(&theme, "backups-dir", self.backups_dir.clone()),
                    ))
                    .child(row(
                        &theme,
                        "Website",
                        link(&theme, "website-link", WEBSITE_LABEL.to_owned())
                            .on_click(cx.listener(|_, _, _, cx| cx.open_url(WEBSITE_URL))),
                    )),
            )
            .child(
                div()
                    .w_full()
                    .flex()
                    .flex_row()
                    .justify_end()
                    .gap_2()
                    .mt_2()
                    .child(
                        div()
                            .id("copy-details")
                            .px_3()
                            .py_1()
                            .rounded(theme.radius_small)
                            .border_1()
                            .cursor_pointer()
                            .border_color(theme.border.control)
                            .child("Copy details")
                            .on_click(cx.listener(Self::copy_details)),
                    )
                    .child(
                        div()
                            .id("close")
                            .px_3()
                            .py_1()
                            .rounded(theme.radius_small)
                            .border_1()
                            .cursor_pointer()
                            .border_color(theme.accent)
                            .bg(theme.accent)
                            .text_color(theme.text.on_accent)
                            .child("Close")
                            .on_click(cx.listener(|_, _, window, _| window.remove_window())),
                    ),
            )
    }
}

/// `path` with the home folder written as `~` (`HOME`, or `USERPROFILE` on Windows), for
/// display only.
fn display_path(path: &Path) -> String {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" });
    match home
        .map(PathBuf::from)
        .and_then(|home| path.strip_prefix(&home).ok().map(Path::to_path_buf))
    {
        Some(rest) => {
            let separator = std::path::MAIN_SEPARATOR;
            format!("~{separator}{}", rest.display())
        }
        None => path.display().to_string(),
    }
}
