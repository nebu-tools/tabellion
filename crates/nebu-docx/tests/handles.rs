//! A node handle knows its tree. A handle from another tree, a clone
//! included, is an error in every method, never a panic or another node.

mod support;

use nebu_docx::xml::{NodeId, Tree};
use nebu_docx::{Document, Error};

fn foreign(r: Result<impl Sized, Error>) -> bool {
    matches!(r, Err(Error::ForeignNode))
}

/// Calls every method of `t` that takes a handle, with `x` in each place a
/// handle goes, and checks that each one refuses it. `own` is a node of `t`.
fn refuses_everywhere(t: &mut Tree, x: NodeId, own: NodeId) {
    assert!(foreign(t.kind(x)));
    assert!(foreign(t.is_element(x)));
    assert!(foreign(t.name(x)));
    assert!(foreign(t.is(x, None, "a")));
    assert!(foreign(t.parent(x)));
    assert!(t.children(x).is_err());
    assert!(t.elements(x).is_err());
    assert!(foreign(t.child(x, None, "b")));
    assert!(t.descendants(x).is_err());
    assert!(t.ancestors(x).is_err());
    assert!(foreign(t.attribute(x, None, "k")));
    assert!(foreign(t.attributes(x)));
    assert!(foreign(t.text(x)));
    assert!(foreign(t.text_content(x)));
    assert!(foreign(t.source_range(x)));
    assert!(foreign(t.set_text(x, "new")));
    assert!(foreign(t.set_attribute(x, None, "k", "v")));
    assert!(foreign(t.remove_attribute(x, None, "k")));
    assert!(foreign(t.set_name(x, None, "z")));
    assert!(foreign(t.append(x, own)));
    assert!(foreign(t.append(own, x)));
    assert!(foreign(t.insert(x, 0, own)));
    assert!(foreign(t.insert(own, 0, x)));
    assert!(foreign(t.insert_before(x, own)));
    assert!(foreign(t.insert_before(own, x)));
    assert!(foreign(t.insert_after(x, own)));
    assert!(foreign(t.insert_after(own, x)));
    assert!(foreign(t.detach(x)));
    assert!(foreign(t.deep_clone(x)));
}

const SRC: &str = "<a k='1'><b>text</b><c/><!--n--><?p d?><![CDATA[x]]></a>";

#[test]
fn a_handle_from_another_tree_is_refused() {
    let other = Tree::parse(SRC).unwrap();
    let mut t = Tree::parse(SRC).unwrap();
    let own = t.elements(t.root_element()).unwrap().nth(1).unwrap();
    // Every node of the other tree, including ones whose index this tree
    // also has and ones it does not.
    let mut all: Vec<NodeId> = vec![other.document()];
    all.extend(other.descendants(other.document()).unwrap());
    let mut bigger = Tree::parse(format!("<a>{}</a>", "<b/>".repeat(100))).unwrap();
    let far = bigger.create_element(None, "far").unwrap();
    all.push(far);
    for x in all {
        refuses_everywhere(&mut t, x, own);
    }
    assert!(!t.is_changed());
    assert_eq!(t.to_bytes(), SRC.as_bytes());
}

#[test]
fn a_clone_is_a_tree_of_its_own() {
    let t = Tree::parse(SRC).unwrap();
    let mut c = t.clone();
    let own = c.root_element();
    let x = t.elements(t.root_element()).unwrap().next().unwrap();
    refuses_everywhere(&mut c, x, own);
    // Its own handles work, and the original's are untouched.
    let b = c.elements(own).unwrap().next().unwrap();
    c.set_text(b, "changed").unwrap();
    assert_eq!(t.text_content(x).unwrap(), "text");
}

#[test]
fn fragments_and_created_nodes_belong_to_their_tree() {
    let mut a = Tree::parse("<a/>").unwrap();
    let mut b = Tree::parse("<a/>").unwrap();
    let frag = a.parse_fragment("<x/>y", &[]).unwrap();
    let made = a.create_text("z").unwrap();
    let root = b.root_element();
    for x in frag.into_iter().chain([made]) {
        assert!(foreign(b.append(root, x)));
    }
    assert!(!b.is_changed());
}

#[test]
fn each_part_of_a_document_has_its_own_handles() {
    let mut doc = Document::from_bytes(support::sample()).unwrap();
    let styles_root = doc.xml("word/styles.xml").unwrap().root_element();
    let t = doc.xml("word/document.xml").unwrap();
    assert!(foreign(t.kind(styles_root)));
    let root = t.root_element();
    assert!(foreign(t.append(root, styles_root)));
}

#[test]
fn a_detached_node_stays_usable() {
    let mut t = Tree::parse("<a><b>x</b><c/></a>").unwrap();
    let root = t.root_element();
    let b = t.elements(root).unwrap().next().unwrap();
    t.detach(b).unwrap();
    assert_eq!(t.parent(b).unwrap(), None);
    assert_eq!(t.text_content(b).unwrap(), "x");
    t.append(root, b).unwrap();
    assert_eq!(t.to_bytes(), b"<a><c/><b>x</b></a>");
}

/// A fresh tree of `SRC` and all its nodes in document order.
fn fresh() -> (Tree, Vec<NodeId>) {
    let t = Tree::parse(format!("<?xml version='1.0'?>{SRC}")).unwrap();
    let doc = t.document();
    let all = std::iter::once(doc)
        .chain(t.descendants(doc).unwrap())
        .collect();
    (t, all)
}

/// Runs an edit on a fresh tree and checks that whatever it did, the tree
/// still writes XML that reads back as the same tree.
fn edit(f: impl FnOnce(&mut Tree, &[NodeId])) {
    let (mut t, all) = fresh();
    f(&mut t, &all);
    let out = t.to_bytes();
    let again = Tree::parse(out.clone())
        .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&out)));
    assert_eq!(again.to_bytes(), out);
}

/// Arguments at the edges of what each method takes give errors or plain
/// answers, never a panic, and never a tree that cannot be written.
#[test]
fn edge_arguments_do_not_panic() {
    let names = [
        "", "a", "1a", "a:b", "a:b:c", ":a", "xml:x", "xmlns", "xmlns:x", "\u{0}", "é",
    ];
    let uris = [
        None,
        Some(""),
        Some("urn:x"),
        Some(nebu_docx::xml::XML_NS),
        Some(nebu_docx::xml::XMLNS_NS),
        Some("\u{FFFE}"),
        Some("\"<&"),
    ];
    let texts = [
        "",
        "]]>",
        "]]",
        ">",
        "\u{0}",
        "\u{FFFF}",
        "\r\n",
        "\n",
        "a\u{D7FF}",
    ];
    let count = fresh().1.len();
    for n in 0..count {
        for ns in uris {
            for name in names {
                let (t, all) = fresh();
                let _ = t.is(all[n], ns, name);
                let _ = t.child(all[n], ns, name);
                let _ = t.attribute(all[n], ns, name);
                edit(|t, all| drop(t.set_name(all[n], ns, name)));
                edit(|t, all| drop(t.remove_attribute(all[n], ns, name)));
                for v in texts {
                    edit(|t, all| drop(t.set_attribute(all[n], ns, name, v)));
                }
                edit(|t, all| {
                    if let Ok(e) = t.create_element(ns, name) {
                        let _ = t.append(all[n], e);
                    }
                });
            }
        }
        for v in texts {
            edit(|t, all| drop(t.set_text(all[n], v)));
            edit(|t, all| {
                if let Ok(x) = t.create_text(v) {
                    let _ = t.insert(all[n], 0, x);
                }
            });
        }
        let (t, all) = fresh();
        let _ = t.kind(all[n]);
        let _ = t.text(all[n]);
        let _ = t.attributes(all[n]);
        let _ = t.source_range(all[n]);
        edit(|t, all| drop(t.detach(all[n])));
        edit(|t, all| {
            if let Ok(c) = t.deep_clone(all[n]) {
                let _ = t.append(t.root_element(), c);
            }
        });
        for m in 0..count {
            for i in [0, 1, 5, usize::MAX] {
                edit(|t, all| drop(t.insert(all[n], i, all[m])));
            }
            edit(|t, all| drop(t.append(all[n], all[m])));
            edit(|t, all| drop(t.insert_before(all[n], all[m])));
            edit(|t, all| drop(t.insert_after(all[n], all[m])));
        }
    }
    let bindings: [&[(&str, &str)]; 5] = [
        &[],
        &[("p", "urn:a"), ("p", "urn:b")],
        &[("", "")],
        &[("xml", "urn:x")],
        &[("p", "\u{0}")],
    ];
    for xml in [
        "",
        "<",
        "&",
        "]]>",
        "<a>",
        "</a>",
        "<?xml version='1.0'?><a/>",
        "<!DOCTYPE a><a/>",
        "<p:a/>",
    ] {
        for ns in bindings {
            edit(|t, _| {
                if let Ok(f) = t.parse_fragment(xml, ns) {
                    for x in f {
                        let _ = t.append(t.root_element(), x);
                    }
                }
            });
        }
    }
    // The declaration cannot be copied, and nothing goes before it.
    let (mut t, all) = fresh();
    let decl = all[1];
    assert!(t.deep_clone(decl).is_err());
    let b = t.elements(t.root_element()).unwrap().next().unwrap();
    assert!(t.insert_before(decl, b).is_err());
}
