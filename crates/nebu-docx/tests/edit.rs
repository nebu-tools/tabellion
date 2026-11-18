//! Edits change their target and nothing else.

mod support;

use std::ops::Range;

use nebu_docx::xml::{NodeKind, Tree};
use nebu_docx::{Document, Error, Target, ns};
use support::zipraw::assert_records_kept;
use support::{corpus, entry, large, sample};

const MAIN: &str = "word/document.xml";

/// `after` is `before` with `range` replaced by `with`.
fn assert_replaced(before: &[u8], after: &[u8], range: Range<usize>, with: &str) {
    let mut want = before[..range.start].to_vec();
    want.extend_from_slice(with.as_bytes());
    want.extend_from_slice(&before[range.end..]);
    if after != want {
        let p = after.iter().zip(&want).take_while(|(a, b)| a == b).count();
        panic!(
            "output differs at byte {p}: got {:?}, want {:?}",
            String::from_utf8_lossy(&after[p..(p + 120).min(after.len())]),
            String::from_utf8_lossy(&want[p..(p + 120).min(want.len())])
        );
    }
}

/// Every entry but `changed` keeps its complete records.
fn assert_others_kept(before: &[u8], after: &[u8], changed: &[&str]) {
    assert_records_kept(before, after, changed);
}

fn body(t: &Tree) -> nebu_docx::xml::NodeId {
    t.child(t.root_element(), Some(ns::W), "body")
        .unwrap()
        .unwrap()
}

fn well_formed(xml: &[u8]) -> String {
    let s = String::from_utf8(xml.to_vec()).unwrap();
    roxmltree::Document::parse(&s).unwrap_or_else(|e| panic!("{e}"));
    s
}

#[test]
fn an_insertion_adds_its_bytes_and_nothing_else() {
    let bytes = large().to_vec();
    let before = entry(&bytes, MAIN);
    let mut doc = Document::from_bytes(bytes.clone()).unwrap();
    let t = doc.xml(MAIN).unwrap();
    let b = body(t);
    let anchor = t.elements(b).unwrap().nth(2000).unwrap();
    let at = t.source_range(anchor).unwrap().unwrap().start;
    let xml = r#"<w:p><w:r><w:t>New &amp; inserted</w:t></w:r></w:p>"#;
    let p = t.parse_fragment(xml, &[("w", ns::W)]).unwrap();
    t.insert_before(anchor, p[0]).unwrap();
    let out = doc.to_bytes().unwrap();
    assert_replaced(&before, &entry(&out, MAIN), at..at, xml);
    assert_others_kept(&bytes, &out, &[MAIN]);
}

#[test]
fn an_attribute_change_rewrites_that_attribute_only() {
    let bytes = large().to_vec();
    let before = entry(&bytes, MAIN);
    let mut doc = Document::from_bytes(bytes.clone()).unwrap();
    let t = doc.xml(MAIN).unwrap();
    let style = t
        .descendants(body(t))
        .unwrap()
        .filter(|&n| t.is(n, Some(ns::W), "pStyle").unwrap())
        .nth(500)
        .unwrap();
    let r = t.source_range(style).unwrap().unwrap();
    let old = String::from_utf8(before[r.clone()].to_vec()).unwrap();
    t.set_attribute(style, Some(ns::W), "w:val", "Heading2")
        .unwrap();
    let out = doc.to_bytes().unwrap();
    let val = old.split('"').nth(1).unwrap();
    assert_replaced(
        &before,
        &entry(&out, MAIN),
        r,
        &old.replace(val, "Heading2"),
    );
}

#[test]
fn a_new_value_keeps_the_quote_style_and_the_other_attributes() {
    let src = "<a xmlns:w='urn:w'><b  w:x = 'one'\n w:y=\"two\"  /></a>";
    let mut t = Tree::parse(src).unwrap();
    let b = t.elements(t.root_element()).unwrap().next().unwrap();
    t.set_attribute(b, Some("urn:w"), "w:y", "2 & 3").unwrap();
    t.set_attribute(b, None, "z", "new").unwrap();
    assert_eq!(
        String::from_utf8(t.to_bytes()).unwrap(),
        "<a xmlns:w='urn:w'><b  w:x = 'one' w:y=\"2 &amp; 3\" z=\"new\"  /></a>"
    );
    assert!(t.remove_attribute(b, Some("urn:w"), "x").unwrap());
    assert!(!t.remove_attribute(b, Some("urn:w"), "x").unwrap());
    assert_eq!(
        String::from_utf8(t.to_bytes()).unwrap(),
        "<a xmlns:w='urn:w'><b w:y=\"2 &amp; 3\" z=\"new\"  /></a>"
    );
}

#[test]
fn text_changes_stay_inside_the_text_node() {
    let bytes = sample();
    let before = entry(&bytes, MAIN);
    let mut doc = Document::from_bytes(bytes.clone()).unwrap();
    let t = doc.xml(MAIN).unwrap();
    let text = t
        .descendants(body(t))
        .unwrap()
        .find(|&n| {
            t.kind(n).unwrap() == NodeKind::Text
                && t.text(n).unwrap().as_deref() == Some("{{author}}")
        })
        .unwrap();
    let r = t.source_range(text).unwrap().unwrap();
    t.set_text(text, "Ada <Lovelace> & co").unwrap();
    let out = doc.to_bytes().unwrap();
    assert_replaced(
        &before,
        &entry(&out, MAIN),
        r,
        "Ada &lt;Lovelace&gt; &amp; co",
    );
    assert_others_kept(&bytes, &out, &[MAIN]);
}

#[test]
fn a_removal_takes_out_exactly_the_node() {
    let bytes = large().to_vec();
    let before = entry(&bytes, MAIN);
    let mut doc = Document::from_bytes(bytes).unwrap();
    let t = doc.xml(MAIN).unwrap();
    let table = t
        .elements(body(t))
        .unwrap()
        .filter(|&n| t.is(n, Some(ns::W), "tbl").unwrap())
        .nth(150)
        .unwrap();
    let r = t.source_range(table).unwrap().unwrap();
    t.detach(table).unwrap();
    assert_replaced(&before, &entry(&doc.to_bytes().unwrap(), MAIN), r, "");
}

#[test]
fn a_move_writes_the_node_as_it_was() {
    let bytes = large().to_vec();
    let before = entry(&bytes, MAIN);
    let mut doc = Document::from_bytes(bytes).unwrap();
    let t = doc.xml(MAIN).unwrap();
    let b = body(t);
    let first = t.elements(b).unwrap().next().unwrap();
    let last_para = t
        .elements(b)
        .unwrap()
        .filter(|&n| t.is(n, Some(ns::W), "p").unwrap())
        .last()
        .unwrap();
    let r = t.source_range(first).unwrap().unwrap();
    let at = t.source_range(last_para).unwrap().unwrap().end;
    t.insert_after(last_para, first).unwrap();
    let out = entry(&doc.to_bytes().unwrap(), MAIN);
    let moved = &before[r.clone()];
    let mut want = before[..r.start].to_vec();
    want.extend_from_slice(&before[r.end..at]);
    want.extend_from_slice(moved);
    want.extend_from_slice(&before[at..]);
    assert!(out == want);
}

#[test]
fn a_rename_keeps_the_attributes_as_written() {
    let mut t =
        Tree::parse("<w:r xmlns:w='urn:w'><w:t xml:space='preserve'> a </w:t></w:r>").unwrap();
    let text = t.elements(t.root_element()).unwrap().next().unwrap();
    t.set_name(text, Some("urn:w"), "w:delText").unwrap();
    assert_eq!(
        String::from_utf8(t.to_bytes()).unwrap(),
        "<w:r xmlns:w='urn:w'><w:delText xml:space='preserve'> a </w:delText></w:r>"
    );
}

#[test]
fn new_content_takes_the_documents_prefixes() {
    // The second document binds WordprocessingML both as the default and to
    // w; the fragment keeps w, which is bound there.
    for (name, prefix) in [("prefixed", "x:"), ("default-namespace", "w:")] {
        let bytes = corpus().into_iter().find(|(n, _)| *n == name).unwrap().1;
        let before = entry(&bytes, MAIN);
        let mut doc = Document::from_bytes(bytes).unwrap();
        let t = doc.xml(MAIN).unwrap();
        let b = body(t);
        let first = t.elements(b).unwrap().next().unwrap();
        let at = t.source_range(first).unwrap().unwrap().start;
        let p = t
            .parse_fragment(r#"<w:p><w:pPr><w:pStyle w:val="BodyText"/></w:pPr><w:r><w:t>New</w:t></w:r></w:p>"#, &[("w", ns::W)])
            .unwrap();
        t.insert(b, 0, p[0]).unwrap();
        let out = entry(&doc.to_bytes().unwrap(), MAIN);
        let want = format!(
            "<{prefix}p><{prefix}pPr><{prefix}pStyle {prefix}val=\"BodyText\"/></{prefix}pPr><{prefix}r><{prefix}t>New</{prefix}t></{prefix}r></{prefix}p>"
        );
        assert_replaced(&before, &out, at..at, &want);
        let s = well_formed(&out);
        let x = roxmltree::Document::parse(&s).unwrap();
        let p = x
            .descendants()
            .find(|n| n.tag_name().name() == "p")
            .unwrap();
        assert_eq!(p.tag_name().namespace(), Some(ns::W), "{name}");
    }
}

#[test]
fn an_unbound_namespace_is_declared_where_it_is_used() {
    let mut t = Tree::parse(r#"<a xmlns="urn:a" xmlns:n="urn:taken"><b/></a>"#).unwrap();
    let root = t.root_element();
    let e = t.create_element(Some("urn:new"), "n:item").unwrap();
    t.set_attribute(e, Some("urn:other"), "o:flag", "1")
        .unwrap();
    let plain = t.create_element(None, "plain").unwrap();
    t.append(e, plain).unwrap();
    t.append(root, e).unwrap();
    let s = String::from_utf8(t.to_bytes()).unwrap();
    assert_eq!(
        s,
        r#"<a xmlns="urn:a" xmlns:n="urn:taken"><b/><ns0:item xmlns:ns0="urn:new" xmlns:o="urn:other" o:flag="1"><plain xmlns=""/></ns0:item></a>"#
    );
    let x = roxmltree::Document::parse(&s).unwrap();
    let item = x.root_element().last_element_child().unwrap();
    assert_eq!(item.tag_name().namespace(), Some("urn:new"));
    assert_eq!(
        item.first_element_child()
            .unwrap()
            .tag_name()
            .namespace()
            .unwrap_or(""),
        ""
    );
}

#[test]
fn a_node_moved_under_other_bindings_is_written_with_them() {
    let src = r#"<r xmlns:p="urn:one"><s xmlns:p="urn:two"><p:x p:a="1"/></s><t/></r>"#;
    let mut t = Tree::parse(src).unwrap();
    let root = t.root_element();
    let s = t.elements(root).unwrap().next().unwrap();
    let target = t.elements(root).unwrap().nth(1).unwrap();
    let x = t.elements(s).unwrap().next().unwrap();
    t.append(target, x).unwrap();
    let out = String::from_utf8(t.to_bytes()).unwrap();
    assert_eq!(
        out,
        r#"<r xmlns:p="urn:one"><s xmlns:p="urn:two"></s><t><ns0:x xmlns:ns0="urn:two" ns0:a="1"/></t></r>"#
    );
    let d = roxmltree::Document::parse(&out).unwrap();
    let x = d
        .descendants()
        .find(|n| n.tag_name().name() == "x")
        .unwrap();
    assert_eq!(x.tag_name().namespace(), Some("urn:two"));
    assert_eq!(x.attribute(("urn:two", "a")), Some("1"));
}

#[test]
fn names_resolve_through_any_prefix() {
    let t = Tree::parse(
        r#"<x:a xmlns:x="urn:w" xmlns="urn:d"><b x:v='1' v="2"/><y:c xmlns:y="urn:w"/></x:a>"#,
    )
    .unwrap();
    let root = t.root_element();
    assert!(t.is(root, Some("urn:w"), "a").unwrap());
    let b = t.elements(root).unwrap().next().unwrap();
    assert!(t.is(b, Some("urn:d"), "b").unwrap());
    assert_eq!(
        t.attribute(b, Some("urn:w"), "v").unwrap().as_deref(),
        Some("1")
    );
    assert_eq!(t.attribute(b, None, "v").unwrap().as_deref(), Some("2"));
    assert_eq!(t.attribute(b, Some("urn:d"), "v").unwrap(), None);
    assert!(t.child(root, Some("urn:w"), "c").unwrap().is_some());
    let attrs = t.attributes(b).unwrap();
    assert_eq!(attrs.len(), 2);
    assert_eq!(
        (attrs[0].name.prefix, attrs[0].name.namespace),
        ("x", Some("urn:w"))
    );
}

#[test]
fn edits_that_would_break_the_tree_are_refused() {
    let mut t = Tree::parse("<?xml version=\"1.0\"?><!--c--><a><b>text</b></a>").unwrap();
    let root = t.root_element();
    let b = t.elements(root).unwrap().next().unwrap();
    let text = t.children(b).unwrap().next().unwrap();
    let comment = t.children(t.document()).unwrap().nth(1).unwrap();
    let edit = |r: Result<(), Error>| matches!(r, Err(Error::Edit(_)));
    assert!(edit(t.append(text, b)));
    assert!(edit(t.append(b, root)));
    assert!(edit(t.append(b, b)));
    assert!(edit(t.detach(root)));
    assert!(edit(t.detach(comment)));
    assert!(edit(t.insert(b, 5, text)));
    assert!(edit(t.set_text(comment, "x")));
    assert!(t.create_element(None, "w:p").is_err());
    assert!(t.create_element(Some("urn:x"), "1p").is_err());
    assert!(t.create_element(Some("urn:x"), "xmlns:p").is_err());
    assert!(t.create_text("bell \u{7}").is_err());
    assert!(t.set_attribute(b, None, "xmlns", "urn:x").is_err());
    assert!(t.parse_fragment("<p:x/>", &[]).is_err());
    assert!(t.parse_fragment("<a>", &[]).is_err());
    // Nothing was changed by the refused edits.
    assert!(!t.is_changed());
}

#[test]
fn copies_are_independent() {
    let mut t = Tree::parse("<a><b k='v'>x</b></a>").unwrap();
    let root = t.root_element();
    let b = t.elements(root).unwrap().next().unwrap();
    let c = t.deep_clone(b).unwrap();
    t.set_attribute(c, None, "k", "w").unwrap();
    t.append(root, c).unwrap();
    assert_eq!(
        String::from_utf8(t.to_bytes()).unwrap(),
        "<a><b k='v'>x</b><b k='w'>x</b></a>"
    );
}

#[test]
fn parts_and_relationships_can_be_added() {
    let bytes = sample();
    let mut doc = Document::from_bytes(bytes.clone()).unwrap();
    let png = b"\x89PNG\r\n\x1a\n".to_vec();
    doc.add_part("word/media/image1.png", "image/png", png.clone())
        .unwrap();
    let id = doc
        .add_relationship(
            MAIN,
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image",
            Target::Part("word/media/image1.png"),
        )
        .unwrap();
    assert_eq!(id, "rId6");
    let link = doc
        .add_relationship(
            MAIN,
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink",
            Target::External("https://example.org/a?b&c"),
        )
        .unwrap();
    assert_eq!(link, "rId7");
    assert_eq!(
        doc.content_type("word/media/image1.png").as_deref(),
        Some("image/png")
    );
    assert!(matches!(
        doc.add_part("WORD/media/image1.png", "image/png", vec![]),
        Err(Error::PartExists(_))
    ));
    assert!(matches!(
        doc.add_part("word/../x.xml", "application/xml", vec![]),
        Err(Error::PartName { .. })
    ));
    assert!(matches!(
        doc.add_relationship(MAIN, "t", Target::Part("word/missing.png")),
        Err(Error::MissingPart(_))
    ));

    let out = doc.to_bytes().unwrap();
    assert_others_kept(
        &bytes,
        &out,
        &["[Content_Types].xml", "word/_rels/document.xml.rels"],
    );
    let again = Document::from_bytes(out.clone()).unwrap();
    assert_eq!(again.read_part("word/media/image1.png").unwrap(), png);
    let mut again = again;
    let rels = again.relationships(MAIN).unwrap();
    let image = rels.iter().find(|r| r.id == "rId6").unwrap();
    assert_eq!(image.target, "media/image1.png");
    assert_eq!(image.part.as_deref(), Some("word/media/image1.png"));
    let link = rels.iter().find(|r| r.id == "rId7").unwrap();
    assert!(link.external && link.part.is_none());
    assert_eq!(link.target, "https://example.org/a?b&c");
    let rels_xml = String::from_utf8(entry(&out, "word/_rels/document.xml.rels")).unwrap();
    assert!(rels_xml.trim_end().ends_with(
        r#"<Relationship Id="rId6" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/image1.png"/><Relationship Id="rId7" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://example.org/a?b&amp;c" TargetMode="External"/></Relationships>"#
    ));
    well_formed(&entry(&out, "[Content_Types].xml"));
}

#[test]
fn a_part_without_relationships_gets_a_relationships_part() {
    let mut doc = Document::from_bytes(sample()).unwrap();
    assert!(doc.relationships("word/footer1.xml").unwrap().is_empty());
    doc.add_part("word/media/logo.png", "image/png", vec![1, 2, 3])
        .unwrap();
    let id = doc
        .add_relationship(
            "word/footer1.xml",
            "urn:image",
            Target::Part("word/media/logo.png"),
        )
        .unwrap();
    assert_eq!(id, "rId1");
    let out = doc.to_bytes().unwrap();
    let rels = String::from_utf8(entry(&out, "word/_rels/footer1.xml.rels")).unwrap();
    assert!(
        rels.contains(r#"<Relationship Id="rId1" Type="urn:image" Target="media/logo.png"/>"#),
        "{rels}"
    );
    let mut again = Document::from_bytes(out).unwrap();
    assert_eq!(
        again.relationships("word/footer1.xml").unwrap()[0]
            .part
            .as_deref(),
        Some("word/media/logo.png")
    );
}

#[test]
fn the_package_is_read_as_the_format_describes() {
    let mut doc = Document::from_bytes(sample()).unwrap();
    assert_eq!(doc.main_part(), MAIN);
    assert_eq!(doc.part_name("WORD/Document.XML"), Some(MAIN));
    assert_eq!(
        doc.content_type("word/styles.xml").as_deref(),
        Some("application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml")
    );
    assert_eq!(
        doc.content_type("word/_rels/document.xml.rels").as_deref(),
        Some("application/vnd.openxmlformats-package.relationships+xml")
    );
    assert_eq!(doc.content_type("word/unknown.bin"), None);
    let rels = doc.relationships("").unwrap();
    assert_eq!(rels[0].part.as_deref(), Some(MAIN));
    let parts: Vec<&str> = doc.parts().collect();
    assert_eq!(parts.len(), 11);
    assert!(matches!(
        doc.xml("word/missing.xml"),
        Err(Error::MissingPart(_))
    ));
    let stored = corpus()
        .into_iter()
        .find(|(n, _)| *n == "stored")
        .unwrap()
        .1;
    let doc = Document::from_bytes(stored).unwrap();
    assert!(
        !doc.parts().any(|p| p.ends_with('/')),
        "directory entries are not parts"
    );
}

#[test]
fn a_changed_utf16_part_stays_utf16() {
    let bytes = corpus().into_iter().find(|(n, _)| *n == "utf16").unwrap().1;
    let before = entry(&bytes, "docProps/app.xml");
    let mut doc = Document::from_bytes(bytes).unwrap();
    let t = doc.xml("docProps/app.xml").unwrap();
    let app = t.elements(t.root_element()).unwrap().next().unwrap();
    let text = t.children(app).unwrap().next().unwrap();
    let r = t.source_range(text).unwrap().unwrap();
    t.set_text(text, "Survey tool").unwrap();
    let out = entry(&doc.to_bytes().unwrap(), "docProps/app.xml");
    let mut want = before[..r.start].to_vec();
    for u in "Survey tool".encode_utf16() {
        want.extend_from_slice(&u.to_le_bytes());
    }
    want.extend_from_slice(&before[r.end..]);
    assert_eq!(out, want);
}

/// Writes the tree, checks that the output parses back to the same text,
/// and returns it.
fn written(t: &Tree) -> String {
    let out = String::from_utf8(t.to_bytes()).unwrap();
    let again = Tree::parse(out.clone()).unwrap_or_else(|e| panic!("{e}: {out:?}"));
    assert_eq!(
        again.text_content(again.root_element()).unwrap(),
        t.text_content(t.root_element()).unwrap(),
        "{out:?}"
    );
    out
}

#[test]
fn text_next_to_text_never_forms_markup() {
    // New text before source text: the new `]` is escaped.
    let mut t = Tree::parse("<r>></r>").unwrap();
    let gt = t.children(t.root_element()).unwrap().next().unwrap();
    let x = t.create_text("]]").unwrap();
    t.insert_before(gt, x).unwrap();
    assert_eq!(written(&t), "<r>]&#93;></r>");

    // Two source texts that meet when what stood between them goes: the
    // `>` is escaped.
    for src in ["<r>]]<x/>></r>", "<r>]<x/>]></r>"] {
        let mut t = Tree::parse(src).unwrap();
        let x = t.elements(t.root_element()).unwrap().next().unwrap();
        t.detach(x).unwrap();
        assert_eq!(written(&t), "<r>]]&gt;</r>", "{src}");
    }

    // A carriage return and a line feed from different nodes stay two line
    // ends.
    let mut t = Tree::parse("<r>a\r<x/>\nb</r>").unwrap();
    let x = t.elements(t.root_element()).unwrap().next().unwrap();
    t.detach(x).unwrap();
    assert_eq!(written(&t), "<r>a\r&#10;b</r>");
    let mut t = Tree::parse("<r>a\r</r>").unwrap();
    let root = t.root_element();
    let lf = t.create_text("\nb").unwrap();
    t.append(root, lf).unwrap();
    assert_eq!(written(&t), "<r>a\r&#10;b</r>");
    assert_eq!(t.text_content(root).unwrap(), "a\n\nb");
}

#[test]
fn a_renamed_element_never_declares_a_prefix_twice() {
    // The element undeclares the default namespace itself, and is renamed
    // into a namespace that is the default where it is.
    let mut t = Tree::parse("<r xmlns='urn:a'><s xmlns='' k='1'/></r>").unwrap();
    let s = t.elements(t.root_element()).unwrap().next().unwrap();
    t.set_name(s, Some("urn:a"), "s").unwrap();
    let out = written(&t);
    assert_eq!(
        out,
        "<r xmlns='urn:a'><ns0:s xmlns:ns0=\"urn:a\" xmlns='' k='1'/></r>"
    );
}
