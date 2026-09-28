//! "What's new": after an update, the CHANGELOG section for the version that just started opens
//! as an ordinary untitled document, once, the next time Tachyon starts
//! (`app::check_whats_new`). Whether this counts as "an update" is decided by comparing the
//! version last seen - one line in a file in the state directory, next to hot-exit backups and
//! recent files (`tachyon_platform::state_dir`) - against this launch's version. The notes
//! themselves are embedded at build time (`build.rs`, `changelog.rs`): no network code, and
//! nothing to fail if the machine is offline.

/// This launch's CHANGELOG section, embedded by `build.rs`: the section heading through the next
/// line starting `## ` (any level-two heading), with a friendly `# What's new in Tachyon X.Y.Z`
/// heading prepended. Empty for a `crates.io`-style build with no `CHANGELOG.md` to read
/// (`build.rs` never fails because of it); `should_open_window` never opens a window when this
/// is empty.
pub(crate) const NOTES: &str = include_str!(concat!(env!("OUT_DIR"), "/whats-new.md"));

/// The version compared against the one last seen, and shown in the window's title: the
/// installed MSIX package's version when this runs packaged, so an update through that channel
/// (the only auto-update Tachyon has) is what triggers it; otherwise this crate's own version.
pub(crate) fn current_version() -> String {
    tachyon_platform::packaged_version().unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_owned())
}

/// What comparing the version last recorded against [`current_version`] means for the version
/// file and the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VersionCheck {
    /// No file yet: a fresh install. Nothing to show, but the version is now on record.
    Fresh,
    /// The version already on record; nothing changed.
    Unchanged,
    /// A different version than the one on record: just updated (or rolled back).
    Changed,
}

/// Compares the version last recorded (`None` if the file did not exist or could not be read)
/// against `current`.
pub(crate) fn check_version(stored: Option<&str>, current: &str) -> VersionCheck {
    match stored.map(str::trim) {
        None => VersionCheck::Fresh,
        Some(stored) if stored == current => VersionCheck::Unchanged,
        Some(_) => VersionCheck::Changed,
    }
}

/// Whether the version file needs (re)writing: anything but the version already on record.
pub(crate) fn should_write(check: VersionCheck) -> bool {
    !matches!(check, VersionCheck::Unchanged)
}

/// Whether the What's new window should open: an update happened, the setting allows it, and
/// there is something embedded to show for it.
pub(crate) fn should_open_window(check: VersionCheck, enabled: bool, has_notes: bool) -> bool {
    check == VersionCheck::Changed && enabled && has_notes
}

/// Reads the version file at `path`, decides what that means against `current`
/// (`check_version`), and writes `current` back when the record needs updating
/// (`should_write`) - the only I/O in the whole check, meant to run on the background executor,
/// off the UI thread. The second value is whether the record is now accurate: always true when
/// no write was needed, otherwise whether the write actually succeeded - an unwritable state
/// directory must not be reported as "recorded" (which would suppress the window on every launch
/// as if it had already been shown) nor as "changed but never written" (which would show it
/// again on every launch instead).
pub(crate) fn read_and_record(path: &std::path::Path, current: &str) -> (VersionCheck, bool) {
    let stored = std::fs::read_to_string(path).ok();
    let check = check_version(stored.as_deref(), current);
    if !should_write(check) {
        return (check, true);
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let recorded = std::fs::write(path, current).is_ok();
    (check, recorded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_install_shows_nothing_but_the_version_is_recorded() {
        assert_eq!(check_version(None, "1.1.0"), VersionCheck::Fresh);
        assert!(should_write(VersionCheck::Fresh));
        assert!(!should_open_window(VersionCheck::Fresh, true, true));
    }

    #[test]
    fn the_same_version_shows_nothing_and_needs_no_write() {
        assert_eq!(check_version(Some("1.1.0"), "1.1.0"), VersionCheck::Unchanged);
        assert!(!should_write(VersionCheck::Unchanged));
        assert!(!should_open_window(VersionCheck::Unchanged, true, true));
    }

    #[test]
    fn a_changed_version_shows_the_window_and_is_recorded() {
        assert_eq!(check_version(Some("1.0.0"), "1.1.0"), VersionCheck::Changed);
        assert!(should_write(VersionCheck::Changed));
        assert!(should_open_window(VersionCheck::Changed, true, true));
    }

    #[test]
    fn whats_new_false_still_records_the_version_but_shows_nothing() {
        assert!(!should_open_window(VersionCheck::Changed, false, true));
    }

    #[test]
    fn no_embedded_notes_shows_nothing_even_on_a_changed_version() {
        assert!(!should_open_window(VersionCheck::Changed, true, false));
    }

    #[test]
    fn read_and_record_round_trips_through_a_real_file() {
        let dir =
            std::env::temp_dir().join(format!("tachyon-whats-new-test-{}", std::process::id()));
        // Isolation: a leftover directory from an earlier, differently-shaped run (or, in theory,
        // a reused pid) must not be mistaken for this run's own fresh state.
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("version.txt");

        assert_eq!(read_and_record(&path, "1.0.0"), (VersionCheck::Fresh, true));
        assert_eq!(std::fs::read_to_string(&path).expect("written"), "1.0.0");

        assert_eq!(read_and_record(&path, "1.0.0"), (VersionCheck::Unchanged, true));

        assert_eq!(read_and_record(&path, "1.1.0"), (VersionCheck::Changed, true));
        assert_eq!(std::fs::read_to_string(&path).expect("written"), "1.1.0");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_write_that_cannot_happen_is_reported_as_not_recorded() {
        let dir = std::env::temp_dir()
            .join(format!("tachyon-whats-new-unwritable-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        // A plain file where the version file's parent directory would need to be: portable
        // (no permission bits to fiddle with), `create_dir_all` for anything under it must fail
        // (ENOTDIR/its Windows equivalent), so the write below never happens.
        let blocker = dir.join("blocker");
        std::fs::write(&blocker, "not a directory").expect("write blocker");
        let path = blocker.join("subdir").join("version.txt");

        let (check, recorded) = read_and_record(&path, "1.0.0");
        assert_eq!(check, VersionCheck::Fresh);
        assert!(!recorded, "a write that could not happen must not be reported as recorded");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
