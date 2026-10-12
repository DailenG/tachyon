// Cuts release sections out of `CHANGELOG.md`'s text. `build.rs` embeds the whole file
// (`whats_new::CHANGELOG`); what the What's new window shows is decided here, at the moment it
// opens, from the version the user updated from and the one now running.

/// A numeric `major.minor.patch`, from `1.4.2`, `v1.4.2` or a four-part MSIX package version such
/// as `1.4.2.0` (the fourth part separates nightlies of one release from each other and from the
/// stable build, not releases, so it is dropped). `None` for anything else.
pub(crate) fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let mut parts = text.trim().trim_start_matches('v').split('.');
    let mut next = || parts.next()?.parse::<u64>().ok();
    let version = (next()?, next()?, next()?);
    match parts.next() {
        None => Some(version),
        Some(build) if build.parse::<u64>().is_ok() && parts.next().is_none() => Some(version),
        Some(_) => None,
    }
}

/// Every `## [label]` section in order, as `(label, text)`: the text runs from its heading line
/// up to, but not including, the next line starting `## ` (any level-two heading, not only a
/// bracketed version - a following `## Migration notes` ends the section too), or the end of the
/// file. Level-two headings that are not `## [..]` end a section but are not returned.
fn sections(changelog: &str) -> Vec<(&str, &str)> {
    let mut found = Vec::new();
    let mut open: Option<(&str, usize)> = None;
    let mut offset = 0;
    for line in changelog.split_inclusive('\n') {
        if line.starts_with("## ") {
            if let Some((label, start)) = open.take() {
                found.push((label, &changelog[start..offset]));
            }
            if let Some(label) = line.strip_prefix("## [").and_then(|rest| rest.split_once(']')) {
                open = Some((label.0, offset));
            }
        }
        offset += line.len();
    }
    if let Some((label, start)) = open {
        found.push((label, &changelog[start..]));
    }
    found
}

/// The notes for `version`: its own section, or the `## [Unreleased]` one if it has none (a
/// development build between releases). `None` if neither is present.
pub(crate) fn extract_section<'a>(changelog: &'a str, version: &str) -> Option<&'a str> {
    let sections = sections(changelog);
    let wanted = parse_version(version);
    sections
        .iter()
        .find(|(label, _)| wanted.is_some() && parse_version(label) == wanted)
        .or_else(|| sections.iter().find(|(label, _)| *label == "Unreleased"))
        .map(|&(_, text)| text)
}

/// Every release newer than `from` up to and including `to`, newest first, with how many there
/// are. Used when someone updates across several releases (1.3.0 to 1.4.2 should show 1.4.0,
/// 1.4.1 and 1.4.2, not only the last). When there is no range to show - `from` is unknown or
/// unparseable (a fresh install, a corrupt record), equal to or newer than `to` (a nightly of the
/// same release, a rollback), or no release in the changelog falls inside it - this is just
/// [`extract_section`] for `to`, as before.
///
/// A development build, whose version has no section of its own yet (its changes are under
/// `## [Unreleased]`), gets that section first, then the releases it jumped over: leaving it out
/// would show the history but not the running build's own newest changes.
pub(crate) fn sections_since(changelog: &str, from: Option<&str>, to: &str) -> Vec<String> {
    let range = from.and_then(parse_version).zip(parse_version(to));
    if let Some((from, to_version)) = range.filter(|(from, to)| from < to) {
        let all = sections(changelog);
        let mut inside: Vec<String> = Vec::new();
        let has_own_section = all.iter().any(|(label, _)| parse_version(label) == Some(to_version));
        if !has_own_section
            && let Some((_, text)) = all.iter().find(|(label, _)| *label == "Unreleased")
        {
            inside.push(text.trim_end().to_owned());
        }
        inside.extend(
            all.iter()
                .filter(|(label, _)| {
                    parse_version(label).is_some_and(|v| v > from && v <= to_version)
                })
                .map(|(_, text)| text.trim_end().to_owned()),
        );
        // Only the Unreleased fallback and nothing between: the single-section answer below.
        if inside.len() > 1 || (inside.len() == 1 && has_own_section) {
            return inside;
        }
    }
    extract_section(changelog, to).map(|s| vec![s.trim_end().to_owned()]).unwrap_or_default()
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

    const MANY: &str = "\
# Changelog

## [Unreleased]

- pending

## [1.4.2] - 2026-10-12

- four two

## [1.4.1] - 2026-10-12

- four one

## [1.4.0] - 2026-10-12

- four zero

## [1.3.0] - 2026-10-04

- three zero

## [1.2.0] - 2026-09-30

- two zero
";

    #[test]
    fn versions_parse_with_or_without_the_msix_build_part() {
        assert_eq!(parse_version("1.4.2"), Some((1, 4, 2)));
        assert_eq!(parse_version("v1.4.2"), Some((1, 4, 2)));
        assert_eq!(parse_version(" 1.4.2\n"), Some((1, 4, 2)));
        assert_eq!(parse_version("1.4.2.0"), Some((1, 4, 2)), "a packaged install's version");
        assert_eq!(parse_version("1.4.2.37"), Some((1, 4, 2)), "a nightly's build part");
        for bad in ["", "1.4", "1.4.x", "Unreleased", "1.4.2.0.1", "1.4.2-rc1", "a.b.c"] {
            assert_eq!(parse_version(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn an_update_across_releases_shows_each_one_newest_first() {
        let got = sections_since(MANY, Some("1.3.0"), "1.4.2");
        let heads: Vec<_> = got.iter().map(|s| s.lines().next().unwrap().to_owned()).collect();
        assert_eq!(
            heads,
            ["## [1.4.2] - 2026-10-12", "## [1.4.1] - 2026-10-12", "## [1.4.0] - 2026-10-12"],
            "1.3.0 itself is not new"
        );
        assert!(got.iter().all(|s| !s.contains("pending")), "never Unreleased");
    }

    #[test]
    fn a_four_part_package_version_compares_by_its_release() {
        let got = sections_since(MANY, Some("1.3.0.0"), "1.4.2.0");
        assert_eq!(got.len(), 3);
        // The same release on a different build is not an update across releases.
        let same = sections_since(MANY, Some("1.4.2.7"), "1.4.2.0");
        assert_eq!(same.len(), 1);
        assert!(same[0].starts_with("## [1.4.2]"));
    }

    #[test]
    fn a_single_step_update_shows_one_release() {
        let got = sections_since(MANY, Some("1.4.1"), "1.4.2");
        assert_eq!(got.len(), 1);
        assert!(got[0].starts_with("## [1.4.2]"));
    }

    #[test]
    fn anything_without_a_usable_range_falls_back_to_the_current_release() {
        for from in [None, Some(""), Some("garbage"), Some("1.4.2"), Some("2.0.0"), Some("1.4.3")] {
            let got = sections_since(MANY, from, "1.4.2");
            assert_eq!(got.len(), 1, "{from:?}");
            assert!(got[0].starts_with("## [1.4.2]"), "{from:?}: {:?}", got[0]);
        }
        // A build ahead of the last release, updated from an older one, shows its own
        // Unreleased changes first, then every release in between.
        let ahead = sections_since(MANY, Some("1.3.0"), "9.9.9");
        assert_eq!(ahead.len(), 4, "Unreleased, then 1.4.2 to 1.4.0");
        assert!(ahead[0].starts_with("## [Unreleased]"), "{:?}", ahead[0]);
        assert!(ahead[1].starts_with("## [1.4.2]"));
        // With no `from`, a build with no section of its own shows Unreleased, as before.
        let dev = sections_since(MANY, None, "9.9.9");
        assert_eq!(dev.len(), 1);
        assert!(dev[0].starts_with("## [Unreleased]"));
        assert!(sections_since("# Changelog\n", Some("1.0.0"), "1.1.0").is_empty());
    }

    #[test]
    fn a_development_build_shows_unreleased_ahead_of_the_releases_it_jumped_over() {
        // The build reports 1.4.3, whose section is not cut yet; the record says 1.3.0.
        let got = sections_since(MANY, Some("1.3.0"), "1.4.3");
        let heads: Vec<_> = got.iter().map(|s| s.lines().next().unwrap().to_owned()).collect();
        assert_eq!(
            heads,
            [
                "## [Unreleased]",
                "## [1.4.2] - 2026-10-12",
                "## [1.4.1] - 2026-10-12",
                "## [1.4.0] - 2026-10-12"
            ]
        );
        // A release with its own section never also pulls in Unreleased.
        let released = sections_since(MANY, Some("1.3.0"), "1.4.2");
        assert!(released.iter().all(|s| !s.contains("pending")), "{released:?}");
        // No jump, no changelog Unreleased: unchanged single-section behaviour.
        let one = sections_since("## [1.4.2]\n\n- a\n", Some("1.4.1"), "1.4.2");
        assert_eq!(one.len(), 1);
    }

    #[test]
    fn a_range_with_no_release_inside_it_still_shows_the_current_one() {
        // 1.4.5 has no section and nothing between 1.4.2 and it: the Unreleased fallback.
        let got = sections_since(MANY, Some("1.4.2"), "1.4.5");
        assert_eq!(got.len(), 1);
        assert!(got[0].starts_with("## [Unreleased]"));
    }

    #[test]
    fn a_non_version_heading_ends_a_section_and_is_not_one() {
        let changelog = "## [1.1.0]\n\n- b\n\n## Migration notes\n\nprose\n\n## [1.0.0]\n\n- a\n";
        let got = sections_since(changelog, Some("0.9.0"), "1.1.0");
        assert_eq!(got.len(), 2);
        assert!(got.iter().all(|s| !s.contains("prose")), "{got:?}");
    }

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
