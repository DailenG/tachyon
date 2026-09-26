//! Realistic LLM output: every edit position and every streaming chunking
//! must converge to the from-scratch parse.

use std::path::PathBuf;

use tachyon_doc::{Block, Document};
use tachyon_md::{BlockKind, ParsedBlock};

fn corpus() -> Vec<(String, String)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/corpus");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .expect("corpus directory")
        .map(|entry| {
            let path = entry.expect("corpus entry").path();
            let name = path.file_name().expect("file name").to_string_lossy().into_owned();
            (name, std::fs::read_to_string(&path).expect("corpus file is UTF-8"))
        })
        .collect();
    files.sort();
    assert!(files.len() >= 5, "corpus files missing from {}", dir.display());
    files
}

fn parsed(doc: &Document) -> Vec<&ParsedBlock> {
    doc.blocks().iter().map(Block::parsed).collect()
}

fn boundaries(text: &str) -> impl Iterator<Item = usize> + '_ {
    (0..=text.len()).filter(|&i| text.is_char_boundary(i))
}

#[test]
fn streaming_in_any_chunk_size_converges() {
    for (name, text) in corpus() {
        let fresh = Document::new(&text);
        for chunk in [1, 3, 7, 64] {
            let mut doc = Document::new("");
            let mut at = 0;
            while at < text.len() {
                let mut end = (at + chunk).min(text.len());
                while !text.is_char_boundary(end) {
                    end += 1;
                }
                doc.edit(doc.len()..doc.len(), &text[at..end]).unwrap();
                doc.reparse_now();
                at = end;
            }
            assert_eq!(parsed(&doc), parsed(&fresh), "{name}, chunk {chunk}");
        }
    }
}

#[test]
fn a_keystroke_anywhere_converges() {
    for (name, text) in corpus() {
        let mut doc = Document::new(&text);
        let normalized = doc.buffer().text();
        for at in boundaries(&normalized) {
            for key in ["x", "\n", "`", "|", "*"] {
                doc.edit(at..at, key).unwrap();
                doc.reparse_now();
                let edited = doc.buffer().text();
                assert_eq!(
                    parsed(&doc),
                    parsed(&Document::new(&edited)),
                    "{name}: {key:?} at {at}"
                );
                doc.edit(at..at + key.len(), "").unwrap();
            }
        }
        doc.reparse_now();
        assert_eq!(parsed(&doc), parsed(&Document::new(&normalized)), "{name}: restored");
    }
}

#[test]
fn deleting_any_line_converges() {
    for (name, text) in corpus() {
        let base = Document::new(&text).buffer().text();
        let mut offset = 0;
        for line in base.split_inclusive('\n') {
            let mut doc = Document::new(&base);
            doc.edit(offset..offset + line.len(), "").unwrap();
            doc.reparse_now();
            let edited = doc.buffer().text();
            assert_eq!(parsed(&doc), parsed(&Document::new(&edited)), "{name}: line at {offset}");
            offset += line.len();
        }
    }
}

#[test]
fn corpus_renders_the_expected_structure() {
    let files = corpus();
    let get = |name: &str| Document::new(&files.iter().find(|(n, _)| n == name).unwrap().1);

    let truncated = get("truncated-in-fence.md");
    let last = truncated.blocks().last().unwrap().parsed();
    assert_eq!(last.kind, BlockKind::CodeBlock { fenced: true, lang: Some("python".into()) });
    assert!(last.ir.text.ends_with("    return"));

    let tables = get("tables-alerts-refs.md");
    let kinds: Vec<_> = tables.blocks().iter().map(|b| b.parsed().kind.clone()).collect();
    assert!(kinds.iter().any(|k| matches!(k, BlockKind::Table(a) if a.len() == 3)));
    let text: String = tables.blocks().iter().map(|b| b.parsed().ir.text.clone()).collect();
    assert!(text.contains("the methodology"), "reference link rendered: {text}");
    assert!(text.contains("docs[bench]"), "footnote reference rendered: {text}");

    let crlf_text = &files.iter().find(|(n, _)| n == "crlf.md").unwrap().1;
    assert!(crlf_text.contains("\r\n"), "fixture must keep CRLF (see .gitattributes)");
    let crlf = Document::new(crlf_text);
    assert!(!crlf.buffer().text().contains('\r'));
    assert_eq!(&crlf.buffer().to_saved_text(), crlf_text);
}
