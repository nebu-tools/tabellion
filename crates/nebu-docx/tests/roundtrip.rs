//! An unchanged save keeps every zip entry, and parsing a part and writing
//! it unchanged gives back its bytes.

mod support;

use nebu_docx::Document;
use nebu_docx::xml::Tree;
use support::{corpus, entries, raw_entries};

#[test]
fn unchanged_save_keeps_every_entry() {
    for (name, bytes) in corpus() {
        let doc = Document::from_bytes(bytes.clone()).unwrap_or_else(|e| panic!("{name}: {e}"));
        let out = doc.to_bytes().unwrap();
        assert_eq!(raw_entries(&out), raw_entries(&bytes), "{name}");
    }
}

#[test]
fn parsed_but_unchanged_parts_keep_their_entries() {
    for (name, bytes) in corpus() {
        let mut doc = Document::from_bytes(bytes.clone()).unwrap();
        let parts: Vec<String> = doc.parts().map(String::from).collect();
        for part in parts
            .iter()
            .filter(|p| p.ends_with(".xml") || p.ends_with(".rels"))
        {
            let tree = doc.xml(part).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(!tree.is_changed());
        }
        let out = doc.to_bytes().unwrap();
        assert_eq!(raw_entries(&out), raw_entries(&bytes), "{name}");
    }
}

#[test]
fn every_xml_part_writes_back_its_bytes() {
    for (name, bytes) in corpus() {
        for (part, data) in entries(&bytes) {
            if !(part.ends_with(".xml") || part.ends_with(".rels")) {
                continue;
            }
            let tree = Tree::parse(data.clone()).unwrap_or_else(|e| panic!("{name} {part}: {e}"));
            assert!(tree.to_bytes() == data, "{name} {part}");
        }
    }
}

#[test]
fn source_ranges_cover_the_node_bytes() {
    for (name, bytes) in corpus() {
        for (part, data) in entries(&bytes) {
            if !part.ends_with(".xml") {
                continue;
            }
            let tree = Tree::parse(data.clone()).unwrap();
            let root = tree.root_element();
            let r = tree.source_range(root).unwrap();
            let s = &data[r];
            match name {
                "utf16" if part.starts_with("docProps") => {
                    assert_eq!(s.len() % 2, 0);
                    assert!(
                        s.starts_with(&[b'<', 0]) || s.starts_with(&[0, b'<']),
                        "{part}"
                    );
                }
                _ => {
                    assert!(s.starts_with(b"<") && s.ends_with(b">"), "{name} {part}");
                }
            }
        }
    }
}

#[test]
fn saving_to_a_file_and_opening_it_again() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("out.docx");
    let doc = Document::from_bytes(support::sample()).unwrap();
    doc.save(&path).unwrap();
    let again = Document::open(&path).unwrap();
    assert_eq!(again.main_part(), "word/document.xml");
    assert_eq!(
        raw_entries(&std::fs::read(&path).unwrap()),
        raw_entries(&support::sample())
    );
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        1,
        "no temporary file is left"
    );
}

/// Writes the corpus, saved unchanged, and two edited documents to the
/// directory in `TABELLION_CORPUS_OUT`, for a check in Word with
/// `tools/word/check.sh`.
#[test]
#[ignore = "writes files for a Word check"]
fn write_corpus_for_word() {
    let Some(dir) = std::env::var_os("TABELLION_CORPUS_OUT") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    for (name, bytes) in corpus() {
        let doc = Document::from_bytes(bytes).unwrap();
        doc.save(dir.join(format!("{name}.docx"))).unwrap();
    }
    for name in ["prefixed", "default-namespace"] {
        let bytes = corpus().into_iter().find(|(n, _)| *n == name).unwrap().1;
        let mut doc = Document::from_bytes(bytes).unwrap();
        let t = doc.xml("word/document.xml").unwrap();
        let body = t
            .child(t.root_element(), Some(nebu_docx::ns::W), "body")
            .unwrap();
        let p = t
            .parse_fragment(
                r#"<w:p><w:pPr><w:pStyle w:val="BodyText"/></w:pPr><w:r><w:rPr><w:b/></w:rPr><w:t>Inserted by the engine.</w:t></w:r></w:p>"#,
                &[("w", nebu_docx::ns::W)],
            )
            .unwrap();
        t.insert(body, 1, p[0]).unwrap();
        doc.save(dir.join(format!("{name}-edited.docx"))).unwrap();
    }
}
