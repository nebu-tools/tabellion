//! Engine properties the rebuild must keep: byte identity, local edits,
//! refusal to touch other authors' revisions, Word-like field results.

use nebu_docx::content::{Block, Figure, FigureKind, Inline};
use nebu_docx::document::Locator;
use nebu_docx::{Cell, Document, Place, Revision, Roles, Table, WriteOptions};

const SAMPLE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../samples/report-template.docx");

fn entries(bytes: &[u8]) -> Vec<(String, u32, Vec<u8>)> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    (0..z.len())
        .map(|i| {
            let mut f = z.by_index(i).unwrap();
            let mut v = vec![];
            std::io::Read::read_to_end(&mut f, &mut v).unwrap();
            (f.name().unwrap().to_string(), f.crc32(), v)
        })
        .collect()
}

#[test]
fn unchanged_document_keeps_every_entry() {
    let src = std::fs::read(SAMPLE).unwrap();
    let mut doc = Document::from_bytes(src.clone()).unwrap();
    doc.blocks().unwrap();
    doc.styles().unwrap();
    let out = doc.to_bytes().unwrap();
    assert_eq!(entries(&src), entries(&out));
}

#[test]
fn tracked_replacement_changes_only_its_paragraph() {
    let src = std::fs::read(SAMPLE).unwrap();
    let mut doc = Document::from_bytes(src.clone()).unwrap();
    let b = doc.locate(&Locator::Bookmark("summary".into())).unwrap();
    let rev = Revision { author: "Tester".into(), date: "2026-01-01T00:00:00Z".into() };
    assert!(nebu_docx::revise::replace_in_paragraph(&mut doc, b.node, "goes here", "is ready", Some(&rev)).unwrap());
    let out = doc.to_bytes().unwrap();
    let (a, b) = (entries(&src), entries(&out));
    for (x, y) in a.iter().zip(&b) {
        if x.0 != "word/document.xml" {
            assert_eq!(x, y, "{} changed", x.0);
        }
    }
    let old = String::from_utf8(a.iter().find(|e| e.0 == "word/document.xml").unwrap().2.clone()).unwrap();
    let new = String::from_utf8(b.iter().find(|e| e.0 == "word/document.xml").unwrap().2.clone()).unwrap();
    // Everything up to the run that is split stays as it was.
    let pre = old.find("w:name=\"summary\"/>").unwrap() + "w:name=\"summary\"/>".len();
    assert_eq!(old[..pre], new[..pre]);
    assert!(new.contains("<w:delText xml:space=\"preserve\">goes here</w:delText>"));
    assert!(new.contains("w:author=\"Tester\""));
    // The second edit by another author must refuse the paragraph.
    let mut doc = Document::from_bytes(out).unwrap();
    let b = doc.locate(&Locator::Bookmark("summary".into())).unwrap();
    let other = Revision { author: "Someone else".into(), date: "2026-01-02T00:00:00Z".into() };
    let e = nebu_docx::revise::replace_in_paragraph(&mut doc, b.node, "summary", "abstract", Some(&other)).unwrap_err();
    assert!(e.to_string().contains("Tester"), "{e}");
}

#[test]
fn captions_and_references_get_word_results() {
    let mut doc = Document::open(SAMPLE).unwrap();
    let roles = Roles::detect(doc.styles().unwrap());
    let table = |label: &str| {
        Block::Figure(Figure {
            kind: FigureKind::Table,
            body: vec![Block::Table(Table { style: None, widths: vec![], header_rows: 1, rows: vec![vec![Cell::text("a")], vec![Cell::text("1")]], align: vec![] })],
            caption: Some(vec![Inline::text("Caption")]),
            label: Some(label.into()),
        })
    };
    let blocks = vec![
        table("one"),
        table("two"),
        Block::Paragraph { inlines: vec![Inline::Ref { target: "two".into() }], style: None, align: None },
    ];
    doc.insert(Place::End, &blocks, &WriteOptions { roles, ..Default::default() }).unwrap();
    let fields = nebu_docx::fields::list(&mut doc);
    let results: Vec<&str> = fields.iter().map(|(_, r)| r.as_str()).collect();
    assert_eq!(results, ["1", "2", "Table 2"]);
}
