//! The views do work in proportion to the document.
//!
//! These tests count work with [`crate::cost`]: every access to a node of a
//! tree and every step of the loops of the views over lists in memory. They
//! read a document of n items and one of 4n, and the second may cost no
//! more than five times the first. A scan of a list inside a loop over the
//! input makes the ratio grow with n and fails them, however fast the
//! machine.

use crate::cost;
use crate::{Document, TextOptions};

const SAMPLE: &[u8] = include_bytes!("../../../../samples/report-template.docx");
const W: &str = crate::ns::W;
const W14: &str = "http://schemas.microsoft.com/office/word/2010/wordml";
const COMMENTS: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments";

/// A document whose body, styles and comments get these children.
fn document(body: &str, styles: &str, comments: &str) -> Document {
    let mut doc = Document::from_bytes(SAMPLE.to_vec()).unwrap();
    let prefixes = [("w", W), ("w14", W14)];
    let main = doc.main_part().to_string();
    let t = doc.xml(&main).unwrap();
    let body_el = t.child(t.root_element(), Some(W), "body").unwrap().unwrap();
    for (i, n) in t
        .parse_fragment(body, &prefixes)
        .unwrap()
        .into_iter()
        .enumerate()
    {
        t.insert(body_el, i, n).unwrap();
    }
    let t = doc.xml("word/styles.xml").unwrap();
    let root = t.root_element();
    for n in t.parse_fragment(styles, &prefixes).unwrap() {
        t.append(root, n).unwrap();
    }
    if !comments.is_empty() {
        let xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><w:comments xmlns:w="{W}" xmlns:w14="{W14}">{comments}</w:comments>"#
        );
        doc.add_part(
            "word/comments.xml",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml",
            xml.into_bytes(),
        )
        .unwrap();
        doc.add_relationship(&main, COMMENTS, crate::Target::Part("word/comments.xml"))
            .unwrap();
    }
    doc
}

/// The work of reading every view and the text with every option.
fn work(make: &dyn Fn(usize) -> Document, n: usize) -> u64 {
    let mut doc = make(n);
    // Parsing is linear by its own tests; this counts the views.
    let main = doc.main_part().to_string();
    doc.xml(&main).unwrap();
    let start = cost::work();
    let v = doc.views().unwrap();
    v.text(&TextOptions {
        headers_footers: true,
        notes: true,
        comments: true,
        ..TextOptions::default()
    });
    for b in v.blocks().iter().take(50) {
        let _ = v.styles().find(b.style_name.as_deref().unwrap_or("x"));
    }
    cost::work() - start
}

fn linear(what: &str, make: impl Fn(usize) -> Document, n: usize) {
    let small = work(&make, n);
    let large = work(&make, 4 * n);
    assert!(
        large <= small * 5,
        "{what}: {small} steps for {n}, {large} for {}",
        4 * n
    );
}

fn repeat(n: usize, f: impl Fn(usize) -> String) -> String {
    (0..n).map(f).collect()
}

#[test]
fn styles_and_latent_styles() {
    linear(
        "styles",
        |n| {
            let latent = format!(
                "<w:latentStyles>{}</w:latentStyles>",
                repeat(n, |i| format!(r#"<w:lsdException w:name="latent {i}"/>"#))
            );
            let styles = repeat(n, |i| {
                format!(
                    r#"<w:style w:type="paragraph" w:styleId="S{i}"><w:name w:val="style {i}"/><w:basedOn w:val="Normal"/></w:style>"#
                )
            });
            // Paragraphs without a style take the default one.
            let body = repeat(n, |_| "<w:p><w:r><w:t>x</w:t></w:r></w:p>".to_string());
            document(&body, &format!("{latent}{styles}"), "")
        },
        500,
    );
}

#[test]
fn comments() {
    linear(
        "comments",
        |n| {
            let body = repeat(n, |i| {
                format!(
                    r#"<w:p><w:commentRangeStart w:id="{i}"/><w:r><w:t>x</w:t></w:r><w:commentRangeEnd w:id="{i}"/><w:r><w:commentReference w:id="{i}"/></w:r></w:p>"#
                )
            });
            let comments = repeat(n, |i| {
                format!(
                    r#"<w:comment w:id="{i}" w:author="Wren Ashby"><w:p w14:paraId="{:08X}"><w:r><w:t>c</w:t></w:r></w:p></w:comment>"#,
                    i + 1
                )
            });
            document(&body, "", &comments)
        },
        500,
    );
}

#[test]
fn bookmarks() {
    linear(
        "bookmarks",
        |n| {
            let body = repeat(n, |i| {
                format!(
                    r#"<w:p><w:bookmarkStart w:id="{i}" w:name="b{i}"/><w:r><w:t>x</w:t></w:r><w:bookmarkEnd w:id="{i}"/></w:p>"#
                )
            });
            document(&body, "", "")
        },
        500,
    );
}

#[test]
fn empty_paragraphs_and_tables() {
    linear(
        "paragraphs",
        |n| {
            let body = repeat(n, |i| {
                if i % 10 == 0 {
                    r#"<w:tbl><w:tr><w:tc><w:p/></w:tc><w:tc><w:p/></w:tc></w:tr></w:tbl>"#
                        .to_string()
                } else {
                    "<w:p/>".to_string()
                }
            });
            document(&body, "", "")
        },
        2000,
    );
}

#[test]
fn simple_fields_inside_open_complex_fields() {
    linear(
        "fields",
        |n| {
            let open = repeat(n, |_| {
                r#"<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText>X</w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r>"#.to_string()
            });
            let simple = repeat(n, |_| {
                r#"<w:fldSimple w:instr="PAGE"><w:r><w:t>1</w:t></w:r></w:fldSimple>"#.to_string()
            });
            document(&format!("<w:p>{open}{simple}</w:p>"), "", "")
        },
        300,
    );
}

#[test]
fn text_boxes_in_drawings() {
    linear(
        "text boxes",
        |n| {
            let boxes = repeat(n, |_| {
                r#"<w:r><w:pict><w:txbxContent><w:p><w:r><w:t>a</w:t></w:r></w:p><w:p><w:r><w:pict><w:txbxContent><w:p><w:r><w:t>b</w:t></w:r></w:p></w:txbxContent></w:pict></w:r></w:p></w:txbxContent></w:pict></w:r>"#.to_string()
            });
            document(&format!("<w:p>{boxes}</w:p>"), "", "")
        },
        200,
    );
}
