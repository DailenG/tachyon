use std::collections::{HashMap, HashSet};

use pulldown_cmark::{Options, Parser};
use unicase::UniCase;

/// Destination and title of a link reference definition (`[label]: dest "title"`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkTarget {
    pub dest: String,
    pub title: String,
}

/// Document-wide definitions a block's rendering can depend on: link
/// reference definitions and footnote labels. Blocks are rendered against
/// this table rather than against their parse window, so a block renders the
/// same whether it is parsed alone or as part of the document.
///
/// Labels compare like CommonMark labels (Unicode case folding; whitespace is
/// already collapsed by the parser). The first definition of a label wins.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DefTable {
    links: HashMap<UniCase<String>, LinkTarget>,
    footnotes: HashSet<UniCase<String>>,
    /// Footnote labels in first-definition order (for the parse prefix).
    footnote_order: Vec<String>,
    /// Build-time only: the label inserted last (always `None` afterward).
    last_link: Option<String>,
}

impl DefTable {
    /// Builds the table from definitions in document order.
    pub fn new<'a>(
        links: impl IntoIterator<Item = &'a (String, LinkTarget)>,
        footnotes: impl IntoIterator<Item = &'a String>,
    ) -> Self {
        let mut table = DefTable::default();
        for (label, target) in links {
            // Documents often repeat a label; skip those before allocating.
            if table.last_link.as_deref() == Some(label.as_str()) {
                continue;
            }
            table.links.entry(UniCase::new(label.clone())).or_insert_with(|| target.clone());
            table.last_link = Some(label.clone());
        }
        table.last_link = None;
        for label in footnotes {
            if table.footnotes.insert(UniCase::new(label.clone())) {
                table.footnote_order.push(label.clone());
            }
        }
        table
    }

    /// Collects the link reference definitions of a complete document
    /// (footnotes need no table when the whole document is parsed at once).
    pub fn from_source(src: &str, options: Options) -> Self {
        let parser = Parser::new_ext(src, options);
        let mut links: Vec<_> = parser
            .reference_definitions()
            .iter()
            .map(|(label, def)| {
                (
                    def.span.start,
                    label.to_owned(),
                    LinkTarget {
                        dest: def.dest.to_string(),
                        title: def.title.as_deref().unwrap_or_default().to_owned(),
                    },
                )
            })
            .collect();
        links.sort_by_key(|(start, ..)| *start);
        let links: Vec<_> = links.into_iter().map(|(_, label, target)| (label, target)).collect();
        Self::new(&links, &[])
    }

    /// The definitions made by `blocks`, in order.
    pub fn from_blocks<'a>(
        blocks: impl IntoIterator<Item = &'a crate::ParsedBlock> + Clone,
    ) -> Self {
        Self::new(
            blocks.clone().into_iter().flat_map(|b| &b.defs),
            blocks.into_iter().flat_map(|b| &b.footnotes),
        )
    }

    /// Same link definitions (footnotes ignored).
    pub fn links_equal(&self, other: &DefTable) -> bool {
        self.links == other.links
    }

    pub fn get(&self, label: &str) -> Option<&LinkTarget> {
        self.links.get(&UniCase::new(label.to_owned()))
    }

    pub fn has_footnote(&self, label: &str) -> bool {
        self.footnotes.contains(&UniCase::new(label.to_owned()))
    }

    /// Source prepended to a parse window so footnote references to
    /// definitions outside the window resolve: one definition per known
    /// footnote, then a blank line and a thematic break that closes them off.
    /// Empty when there are none.
    pub(crate) fn footnote_prefix(&self) -> String {
        if self.footnote_order.is_empty() {
            return String::new();
        }
        let mut prefix = String::new();
        for label in &self.footnote_order {
            prefix.push_str("[^");
            prefix.push_str(label);
            prefix.push_str("]: .\n");
        }
        prefix.push_str("\n***\n");
        prefix
    }

    /// Identifies the footnote definitions a window is parsed against (the
    /// [`DefTable::footnote_prefix`] is a function of them): a block that
    /// mentions footnotes renders stale when the key changes.
    pub fn footnote_key(&self) -> u64 {
        let mut hasher = std::hash::DefaultHasher::new();
        std::hash::Hash::hash(&self.footnote_order, &mut hasher);
        std::hash::Hasher::finish(&hasher)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(dest: &str) -> LinkTarget {
        LinkTarget { dest: dest.into(), title: String::new() }
    }

    #[test]
    fn first_definition_wins_and_labels_fold_case() {
        let src = "[Foo]: /first\n\n[foo]: /second\n\n[ẞ]: /sharp-s\n";
        let table = DefTable::from_source(src, crate::options());
        assert_eq!(table.get("FOO").map(|t| t.dest.as_str()), Some("/first"));
        assert_eq!(table.get("ß").map(|t| t.dest.as_str()), Some("/sharp-s"));
    }

    #[test]
    fn footnote_key_follows_footnote_definitions_only() {
        let notes = |labels: &[&str]| labels.iter().map(|l| (*l).to_owned()).collect::<Vec<_>>();
        let table = DefTable::new(&[("a".into(), target("/a"))], &notes(&["n", "m"]));
        let retargeted = DefTable::new(&[("a".into(), target("/b"))], &notes(&["n", "m", "N"]));
        assert_eq!(
            table.footnote_key(),
            retargeted.footnote_key(),
            "links and repeats do not count"
        );
        let added = DefTable::new(&[], &notes(&["n", "m", "o"]));
        assert_ne!(table.footnote_key(), added.footnote_key());
        let reordered = DefTable::new(&[], &notes(&["m", "n"]));
        assert_ne!(
            table.footnote_key(),
            reordered.footnote_key(),
            "the prefix lists them in order"
        );
    }
}
