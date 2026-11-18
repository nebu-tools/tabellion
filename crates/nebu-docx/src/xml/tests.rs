//! Tests of what the tree keeps to itself: where each node came from and
//! whether anything changed.

use super::{NodeKind, Tree};

/// Each node's source range covers the bytes it was parsed from.
fn ranges_match(src: &[u8], decode: impl Fn(&[u8]) -> String) {
    let t = Tree::parse(src.to_vec()).unwrap();
    let doc = t.document();
    assert_eq!(t.source_range(doc).unwrap(), None);
    for n in t.descendants(doc).unwrap() {
        let r = t.source_range(n).unwrap().expect("a parsed node");
        let s = decode(&src[r]);
        match t.kind(n).unwrap() {
            NodeKind::Element => assert!(s.starts_with('<') && s.ends_with('>'), "{s:?}"),
            NodeKind::Text => assert!(!s.contains('<'), "{s:?}"),
            NodeKind::Comment => assert!(s.starts_with("<!--") && s.ends_with("-->")),
            NodeKind::CData => assert!(s.starts_with("<![CDATA[") && s.ends_with("]]>")),
            NodeKind::ProcessingInstruction | NodeKind::Declaration => {
                assert!(s.starts_with("<?") && s.ends_with("?>"), "{s:?}")
            }
            NodeKind::Document => unreachable!(),
        }
    }
    let root = t.root_element();
    let r = t.source_range(root).unwrap().unwrap();
    assert_eq!(
        decode(&src[r]),
        "<a k='>'>é<b/><!--c--><?p d?><![CDATA[x]]></a>"
    );
}

const DOC: &str = "<?xml version='1.0'?>\n<a k='>'>é<b/><!--c--><?p d?><![CDATA[x]]></a>\n";

#[test]
fn source_ranges_cover_the_node_bytes() {
    let utf8 = |b: &[u8]| String::from_utf8(b.to_vec()).unwrap();
    ranges_match(DOC.as_bytes(), utf8);
    let mut bom = b"\xEF\xBB\xBF".to_vec();
    bom.extend_from_slice(DOC.as_bytes());
    ranges_match(&bom, utf8);
}

#[test]
fn source_ranges_count_utf16_bytes() {
    let doc = DOC.replace(
        "<?xml version='1.0'?>",
        "<?xml version='1.0' encoding='UTF-16'?>",
    );
    let utf16 = |b: &[u8]| {
        assert_eq!(b.len() % 2, 0);
        let units: Vec<u16> = b
            .chunks(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16(&units).unwrap()
    };
    let mut src = vec![0xFF, 0xFE];
    for u in doc.encode_utf16() {
        src.extend_from_slice(&u.to_le_bytes());
    }
    ranges_match(&src, utf16);
}

#[test]
fn created_and_fragment_nodes_have_no_source_range() {
    let mut t = Tree::parse("<a/>").unwrap();
    let e = t.create_element(None, "b").unwrap();
    let f = t.parse_fragment("<c/>", &[]).unwrap();
    assert_eq!(t.source_range(e).unwrap(), None);
    assert_eq!(t.source_range(f[0]).unwrap(), None);
}

#[test]
fn only_an_edit_marks_the_tree_changed() {
    let mut t = Tree::parse("<a><b>x</b></a>").unwrap();
    assert!(!t.is_changed());
    let root = t.root_element();
    // Reading, making detached nodes and refused edits change nothing.
    let _ = t.text_content(root).unwrap();
    let made = t.create_element(None, "c").unwrap();
    let _ = t.deep_clone(root).unwrap();
    assert!(t.append(made, root).is_err());
    assert!(t.detach(t.document()).is_err());
    assert!(!t.is_changed());
    t.append(root, made).unwrap();
    assert!(t.is_changed());

    let mut t = Tree::parse("<a>x</a>").unwrap();
    let x = t.children(t.root_element()).unwrap().next().unwrap();
    t.set_text(x, "y").unwrap();
    assert!(t.is_changed());
}
