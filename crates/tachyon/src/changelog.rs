// Cuts one version's section out of `CHANGELOG.md`'s text: its own `## [X.Y.Z]` heading line up
// to, but not including, the next line starting `## ` (any level-two heading, not only a
// bracketed version - a following `## Migration notes` or the like ends the section too), or the
// end of the file. `build.rs` embeds the
// running version's section into the binary (see `whats_new::NOTES`) by `include!`-ing this
// file's source rather than depending on this crate: a build script runs before the crate it
// belongs to exists as a built artifact, so it cannot simply `use` it. The function lives here,
// not inline in `build.rs`, so it has its own unit tests, run the ordinary way
// (`cargo test -p tachyon`). A plain comment, not `//!`: `include!`'s expansion point in
// `build.rs` is never the very start of that file, where an inner doc comment would have to be.

/// The section for `version` - the module comment above says exactly what that means - or the
/// `## [Unreleased]` section if `version` has no heading of its own (the common case for a
/// development build between releases). `None` if neither is present.
///
/// Unused outside tests in the linked `tachyon` binary itself: `build.rs`, the only production
/// caller, is a separate compilation (see the module comment), so the compiler cannot see that
/// use from here.
#[allow(dead_code, reason = "called by build.rs's separate compilation via include!, and tests")]
pub(crate) fn extract_section<'a>(changelog: &'a str, version: &str) -> Option<&'a str> {
    let heading = format!("## [{version}]");
    find_section(changelog, &heading).or_else(|| find_section(changelog, "## [Unreleased]"))
}

/// Finds `heading` as a line prefix (so `## [1.0.0] - 2026-09-28` matches `## [1.0.0]`), then
/// returns the text from that line up to the next line starting with `## ` (any level-two
/// heading), or the end of the text if there is none.
fn find_section<'a>(changelog: &'a str, heading: &str) -> Option<&'a str> {
    let mut offset = 0;
    let mut start = None;
    for line in changelog.split_inclusive('\n') {
        match start {
            None if line.starts_with(heading) => start = Some(offset),
            Some(section_start) if line.starts_with("## ") => {
                return Some(&changelog[section_start..offset]);
            }
            _ => {}
        }
        offset += line.len();
    }
    start.map(|section_start| &changelog[section_start..])
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHANGELOG: &str = "\
# Changelog

## [Unreleased]

### Added

- unreleased thing

## [1.1.0] - 2026-10-01

### Added

- new thing

## [1.0.0] - 2026-09-28

### Added

- old thing
";

    #[test]
    fn a_version_with_its_own_section() {
        let section = extract_section(CHANGELOG, "1.1.0").expect("found");
        assert!(section.starts_with("## [1.1.0] - 2026-10-01"), "{section:?}");
        assert!(section.contains("- new thing"));
        assert!(!section.contains("- old thing"), "stopped before the next heading: {section:?}");
        assert!(!section.contains("- unreleased thing"));
    }

    #[test]
    fn falls_back_to_unreleased_when_the_version_has_no_section() {
        let section = extract_section(CHANGELOG, "9.9.9").expect("found");
        assert!(section.starts_with("## [Unreleased]"), "{section:?}");
        assert!(section.contains("- unreleased thing"));
        assert!(!section.contains("- new thing"), "stopped before the next heading: {section:?}");
    }

    #[test]
    fn missing_entirely_with_no_unreleased_section_either() {
        let changelog = "# Changelog\n\n## [1.0.0] - 2026-09-28\n\n- old thing\n";
        assert_eq!(extract_section(changelog, "9.9.9"), None);
    }

    #[test]
    fn the_last_section_in_the_file_runs_to_the_end() {
        let section = extract_section(CHANGELOG, "1.0.0").expect("found");
        assert!(section.starts_with("## [1.0.0] - 2026-09-28"), "{section:?}");
        assert!(section.trim_end().ends_with("- old thing"), "{section:?}");
    }

    #[test]
    fn stops_at_any_level_two_heading_not_only_bracketed_ones() {
        let changelog = "\
# Changelog

## [1.1.0] - 2026-10-01

### Added

- new thing

## Migration notes

Some prose that is not a version section at all.

## [1.0.0] - 2026-09-28

### Added

- old thing
";
        let section = extract_section(changelog, "1.1.0").expect("found");
        assert!(section.starts_with("## [1.1.0] - 2026-10-01"), "{section:?}");
        assert!(section.contains("- new thing"));
        assert!(
            !section.contains("Migration notes"),
            "stopped before the next heading: {section:?}"
        );
    }
}
