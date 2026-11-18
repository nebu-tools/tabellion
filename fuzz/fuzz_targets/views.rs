//! The views on any input: nothing may panic.
//!
//! The first byte chooses what the rest is. Even: the bytes of a file, opened
//! as a document. Odd: XML put into the body of the sample template, with the
//! prefixes of WordprocessingML, markup compatibility and math bound, which
//! reaches the views far more often than random archives do.
#![no_main]

use libfuzzer_sys::fuzz_target;
use nebu_docx::{Document, Limits, Revisions, TextOptions};

const SAMPLE: &[u8] = include_bytes!("../../samples/report-template.docx");

const PREFIXES: &[(&str, &str)] = &[
    ("w", "http://schemas.openxmlformats.org/wordprocessingml/2006/main"),
    ("w14", "http://schemas.microsoft.com/office/word/2010/wordml"),
    ("mc", "http://schemas.openxmlformats.org/markup-compatibility/2006"),
    ("m", "http://schemas.openxmlformats.org/officeDocument/2006/math"),
    ("r", "http://schemas.openxmlformats.org/officeDocument/2006/relationships"),
    ("wp", "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"),
    ("a", "http://schemas.openxmlformats.org/drawingml/2006/main"),
    ("v", "urn:schemas-microsoft-com:vml"),
    ("o", "urn:schemas-microsoft-com:office:office"),
    ("w10", "urn:schemas-microsoft-com:office:word"),
    ("w15", "http://schemas.microsoft.com/office/word/2012/wordml"),
    ("wp14", "http://schemas.microsoft.com/office/word/2010/wordprocessingDrawing"),
    ("wps", "http://schemas.microsoft.com/office/word/2010/wordprocessingShape"),
    ("wpg", "http://schemas.microsoft.com/office/word/2010/wordprocessingGroup"),
    ("pic", "http://schemas.openxmlformats.org/drawingml/2006/picture"),
    ("a14", "http://schemas.microsoft.com/office/drawing/2010/main"),
    ("w16du", "http://schemas.microsoft.com/office/word/2023/wordml/word16du"),
    ("x", "urn:example:field-notes"),
    ("future", "urn:example:future-word"),
];

fn limits() -> Limits {
    Limits {
        max_part_size: 1 << 20,
        max_total_size: 8 << 20,
        max_depth: 4096,
        ..Limits::default()
    }
}

fn document(data: &[u8]) -> Option<Document> {
    let (&mode, rest) = data.split_first()?;
    if mode % 2 == 0 {
        return Document::from_bytes_with_limits(rest, limits()).ok();
    }
    let xml = std::str::from_utf8(rest).ok()?;
    let mut doc = Document::from_bytes_with_limits(SAMPLE, limits()).ok()?;
    let main = doc.main_part().to_string();
    let t = doc.xml(&main).ok()?;
    let body = t.child(t.root_element(), Some(PREFIXES[0].1), "body").ok()??;
    let nodes = t.parse_fragment(xml, PREFIXES).ok()?;
    for (i, n) in nodes.into_iter().enumerate() {
        t.insert(body, i, n).ok()?;
    }
    Some(doc)
}

fuzz_target!(|data: &[u8]| {
    let Some(mut doc) = document(data) else { return };
    let _ = doc.blocks();
    let _ = doc.styles();
    let _ = doc.tables();
    let _ = doc.fields();
    let _ = doc.revisions();
    let _ = doc.comments();
    let _ = doc.images();
    let _ = doc.bookmarks();
    let _ = doc.notes();
    let _ = doc.sections();
    let _ = doc.numbering();
    let _ = doc.content_controls();
    for revisions in [Revisions::Accepted, Revisions::Rejected, Revisions::Current] {
        let _ = doc.text(&TextOptions {
            revisions,
            headers_footers: true,
            notes: true,
            comments: true,
            text_boxes: true,
        });
    }
    for l in ["heading:Results", "bookmark:summary", "placeholder:results", "text:the"] {
        if let Ok(l) = l.parse() {
            let _ = doc.locate(&l);
        }
    }
});
