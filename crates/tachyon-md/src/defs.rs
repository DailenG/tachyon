use std::collections::HashMap;

use pulldown_cmark::{Options, Parser};
use unicase::UniCase;

/// Destination and title of a link reference definition (`[label]: dest "title"`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkTarget {
    pub dest: String,
    pub title: String,
}

/// Document-wide link reference definitions. Reference links are resolved
/// against this table rather than against the parse window, so a block
/// renders the same whether it is parsed alone or as part of the document.
///
/// Labels compare like CommonMark labels (Unicode case folding; whitespace is
/// already collapsed by the parser). The first definition of a label wins.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DefTable {
    map: HashMap<UniCase<String>, LinkTarget>,
}

impl DefTable {
    /// Builds the table from definitions in document order.
    pub fn from_defs<'a>(defs: impl IntoIterator<Item = &'a (String, LinkTarget)>) -> Self {
        let mut map = HashMap::new();
        for (label, target) in defs {
            map.entry(UniCase::new(label.clone())).or_insert_with(|| target.clone());
        }
        Self { map }
    }

    /// Collects the definitions of a complete document.
    pub fn from_source(src: &str, options: Options) -> Self {
        let parser = Parser::new_ext(src, options);
        let mut defs: Vec<_> = parser
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
        defs.sort_by_key(|(start, ..)| *start);
        let defs: Vec<_> = defs.into_iter().map(|(_, label, target)| (label, target)).collect();
        Self::from_defs(&defs)
    }

    pub fn get(&self, label: &str) -> Option<&LinkTarget> {
        self.map.get(&UniCase::new(label.to_owned()))
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Labels whose target differs between `self` and `other` (added, removed
    /// or changed), in unspecified order.
    pub fn changed_labels(&self, other: &DefTable) -> Vec<String> {
        let mut changed: Vec<String> = self
            .map
            .iter()
            .filter(|(label, target)| other.map.get(*label) != Some(target))
            .map(|(label, _)| label.to_string())
            .collect();
        changed.extend(
            other
                .map
                .keys()
                .filter(|label| !self.map.contains_key(*label))
                .map(|label| label.to_string()),
        );
        changed
    }
}

/// Whether two labels match under CommonMark label comparison.
pub fn labels_match(a: &str, b: &str) -> bool {
    UniCase::new(a) == UniCase::new(b)
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
        assert_eq!(table.len(), 2);
    }

    #[test]
    fn changed_labels_reports_additions_removals_and_changes() {
        let old = DefTable::from_defs(&[("a".into(), target("/a")), ("b".into(), target("/b"))]);
        let new = DefTable::from_defs(&[("A".into(), target("/a")), ("c".into(), target("/c"))]);
        let mut changed = old.changed_labels(&new);
        changed.sort();
        assert_eq!(changed, vec!["b".to_owned(), "c".to_owned()]);
        assert!(old.changed_labels(&old.clone()).is_empty());
    }
}
