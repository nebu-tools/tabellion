//! Malformed and hostile files give an error, never a panic, and stay within
//! the limits.

mod support;

use nebu_docx::xml::Tree;
use nebu_docx::{Document, Error, Limits};
use support::{entries, sample, sample_with, zip};

/// Opens a file and, if that works, reads every part and saves.
fn exercise(bytes: Vec<u8>, limits: Limits) -> Result<(), Error> {
    let mut doc = Document::from_bytes_with_limits(bytes, limits)?;
    let parts: Vec<String> = doc.parts().map(String::from).collect();
    for p in &parts {
        doc.read_part(p)?;
        if p.ends_with(".xml") || p.ends_with(".rels") {
            doc.xml(p)?;
        }
        doc.relationships(p)?;
    }
    doc.to_bytes()?;
    Ok(())
}

fn with_document(xml: &str) -> Vec<u8> {
    sample_with(&[("word/document.xml", Some(xml.as_bytes().to_vec()))])
}

#[test]
fn truncated_files_are_refused() {
    let s = sample();
    for n in (0..s.len()).step_by(37) {
        assert!(Document::from_bytes(s[..n].to_vec()).is_err(), "{n} bytes");
    }
}

#[test]
fn damaged_bytes_never_panic() {
    let s = sample();
    let mut seed = 1u64;
    for _ in 0..3000 {
        let mut b = s.clone();
        for _ in 0..1 + (seed % 4) {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let i = (seed >> 33) as usize % b.len();
            b[i] = (seed >> 13) as u8;
        }
        let _ = exercise(b, Limits::default());
    }
}

#[test]
fn a_zip_bomb_stops_at_the_part_limit() {
    let zeros = vec![0u8; 4 << 20];
    let bytes = sample_with(&[("word/media/bomb.bin", Some(zeros))]);
    assert!(bytes.len() < 100_000);
    let limits = Limits {
        max_part_size: 1 << 20,
        ..Limits::default()
    };
    let doc = Document::from_bytes_with_limits(bytes.clone(), limits).unwrap();
    let e = doc.read_part("word/media/bomb.bin").unwrap_err();
    assert!(
        matches!(
            e,
            Error::Limit {
                field: "max_part_size",
                ..
            }
        ),
        "{e}"
    );
    assert!(e.to_string().contains("raise Limits::max_part_size"), "{e}");
    // Saving copies the entry without decompressing it, with a change too.
    assert!(doc.to_bytes().unwrap() == bytes);
    let mut doc = doc;
    let t = doc.xml("word/document.xml").unwrap();
    let root = t.root_element();
    t.set_attribute(root, None, "x", "1").unwrap();
    support::zipraw::assert_records_kept(&bytes, &doc.to_bytes().unwrap(), &["word/document.xml"]);
}

#[test]
fn a_part_that_lies_about_its_size_stops_at_the_limit() {
    let mut bytes = sample_with(&[("word/media/big.bin", Some(vec![0u8; 3 << 20]))]);
    // Claim 16 bytes in the local and the central header.
    let size = (3u32 << 20).to_le_bytes();
    let mut i = 0;
    while let Some(k) = bytes[i..].windows(4).position(|w| w == size) {
        bytes[i + k..i + k + 4].copy_from_slice(&16u32.to_le_bytes());
        i += k + 4;
    }
    let limits = Limits {
        max_part_size: 1 << 20,
        ..Limits::default()
    };
    let doc = Document::from_bytes_with_limits(bytes, limits).unwrap();
    match doc.read_part("word/media/big.bin") {
        Ok(b) => assert!(b.len() <= 1 << 20),
        Err(e) => assert!(matches!(e, Error::Limit { .. } | Error::Zip(_)), "{e}"),
    }
}

#[test]
fn the_total_limit_counts_every_part_read() {
    let part = vec![b'a'; 600 << 10];
    let bytes = sample_with(&[
        ("word/media/a.bin", Some(part.clone())),
        ("word/media/b.bin", Some(part)),
    ]);
    let limits = Limits {
        max_total_size: 1 << 20,
        ..Limits::default()
    };
    let doc = Document::from_bytes_with_limits(bytes, limits).unwrap();
    doc.read_part("word/media/a.bin").unwrap();
    let e = doc.read_part("word/media/b.bin").unwrap_err();
    assert!(
        matches!(
            e,
            Error::Limit {
                field: "max_total_size",
                ..
            }
        ),
        "{e}"
    );
}

#[test]
fn deep_nesting_stops_at_the_depth_limit() {
    let n = 20_000;
    let xml = format!("<a>{}{}</a>", "<b>".repeat(n), "</b>".repeat(n));
    let e = Tree::parse(xml.clone()).unwrap_err();
    assert!(
        matches!(
            e,
            Error::Limit {
                field: "max_depth",
                ..
            }
        ),
        "{e}"
    );
    let doc_xml = format!(
        "<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body>{}{}</w:body></w:document>",
        "<w:sdt>".repeat(n),
        "</w:sdt>".repeat(n)
    );
    let mut doc = Document::from_bytes(with_document(&doc_xml)).unwrap();
    let e = doc.xml("word/document.xml").unwrap_err();
    assert!(
        matches!(
            e,
            Error::Limit {
                field: "max_depth",
                ..
            }
        ),
        "{e}"
    );
    // With the limit raised, deep trees are read, edited and written
    // without recursion.
    let limits = Limits {
        max_depth: n + 10,
        ..Limits::default()
    };
    let mut doc = Document::from_bytes_with_limits(with_document(&doc_xml), limits).unwrap();
    let t = doc.xml("word/document.xml").unwrap();
    let deepest = t.descendants(t.root_element()).unwrap().last().unwrap();
    let new = t.create_element(Some(nebu_docx::ns::W), "w:p").unwrap();
    t.append(deepest, new).unwrap();
    let copy = t.deep_clone(t.root_element()).unwrap();
    assert!(t.descendants(copy).unwrap().count() > n);
    let out = doc.to_bytes().unwrap();
    assert!(
        String::from_utf8(support::entry(&out, "word/document.xml"))
            .unwrap()
            .contains("<w:sdt><w:p/></w:sdt>")
    );
}

fn is_limit(r: Result<impl Sized, Error>, name: &str) -> bool {
    matches!(r, Err(Error::Limit { field, .. }) if field == name)
}

#[test]
fn the_node_limit_counts_every_kind_of_node() {
    // 10,000 comments, then the same for CDATA sections and processing
    // instructions, under a limit of 100.
    for node in ["<!--c-->", "<![CDATA[c]]>", "<?pi c?>", "<b/>", "t<b/>"] {
        let xml = format!("<a>{}</a>", node.repeat(10_000));
        let limits = Limits {
            max_xml_nodes: 100,
            ..Limits::default()
        };
        let mut doc =
            Document::from_bytes_with_limits(with_part("customXml/n.xml", &xml), limits).unwrap();
        assert!(
            is_limit(doc.xml("customXml/n.xml"), "max_xml_nodes"),
            "{node}"
        );
    }
}

/// The sample with one more part.
fn with_part(name: &str, data: &str) -> Vec<u8> {
    sample_with(&[(name, Some(data.as_bytes().to_vec()))])
}

/// Each limit lets through an input of exactly its value and refuses one
/// more.
#[test]
fn every_limit_holds_at_its_value() {
    let with = |f: &dyn Fn(&mut Limits)| {
        let mut l = Limits::default();
        f(&mut l);
        l
    };

    // Entries: the sample has 11.
    let n = entries(&sample()).len();
    assert!(Document::from_bytes_with_limits(sample(), with(&|l| l.max_entries = n)).is_ok());
    assert!(is_limit(
        Document::from_bytes_with_limits(sample(), with(&|l| l.max_entries = n - 1)),
        "max_entries"
    ));

    // The size of one part.
    let part = "x".repeat(100_000);
    let bytes = with_part("word/media/x.bin", &part);
    let open = |l: Limits| Document::from_bytes_with_limits(bytes.clone(), l).unwrap();
    let size = part.len() as u64;
    assert!(
        open(with(&|l| l.max_part_size = size))
            .read_part("word/media/x.bin")
            .is_ok()
    );
    assert!(is_limit(
        open(with(&|l| l.max_part_size = size - 1)).read_part("word/media/x.bin"),
        "max_part_size"
    ));

    // The size of all parts read: opening reads the content types and the
    // package relationships.
    let opened: u64 = entries(&bytes)
        .iter()
        .filter(|(n, _)| n == "[Content_Types].xml" || n == "_rels/.rels")
        .map(|(_, d)| d.len() as u64)
        .sum();
    let total = opened + size;
    assert!(
        open(with(&|l| l.max_total_size = total))
            .read_part("word/media/x.bin")
            .is_ok()
    );
    assert!(is_limit(
        open(with(&|l| l.max_total_size = total - 1)).read_part("word/media/x.bin"),
        "max_total_size"
    ));
    assert!(is_limit(
        Document::from_bytes_with_limits(bytes.clone(), with(&|l| l.max_total_size = opened - 1)),
        "max_total_size"
    ));

    // Nesting depth: 40 elements, one in the other.
    let deep = format!("{}{}", "<a>".repeat(40), "</a>".repeat(40));
    let bytes = with_part("customXml/deep.xml", &deep);
    let depth = |d: usize| {
        let mut doc =
            Document::from_bytes_with_limits(bytes.clone(), with(&|l| l.max_depth = d)).unwrap();
        doc.xml("customXml/deep.xml").map(|_| ())
    };
    assert!(depth(40).is_ok());
    assert!(is_limit(depth(39), "max_depth"));

    // Nodes: the declaration, the root, and 4 nodes of each kind 25 times.
    let xml = format!(
        "<?xml version=\"1.0\"?><a>{}</a>",
        "<!--c--><?p d?><![CDATA[x]]>t".repeat(25)
    );
    let nodes = 2 + 4 * 25;
    let bytes = with_part("customXml/nodes.xml", &xml);
    let count = |n: usize| {
        let mut doc =
            Document::from_bytes_with_limits(bytes.clone(), with(&|l| l.max_xml_nodes = n))
                .unwrap();
        doc.xml("customXml/nodes.xml").map(|_| ())
    };
    assert!(count(nodes).is_ok());
    assert!(is_limit(count(nodes - 1), "max_xml_nodes"));
}

#[test]
fn too_many_entries_stop_at_the_entry_limit() {
    let limits = Limits {
        max_entries: 5,
        ..Limits::default()
    };
    let e = Document::from_bytes_with_limits(sample(), limits).unwrap_err();
    assert!(
        matches!(
            e,
            Error::Limit {
                field: "max_entries",
                ..
            }
        ),
        "{e}"
    );
}

#[test]
fn malformed_xml_is_reported_with_its_place() {
    let bad = [
        ("<a>", "is not closed"),
        ("<a></b>", "does not close"),
        ("<a><b></a></b>", "does not close"),
        ("<a x='1' x='2'/>", "same attribute twice"),
        (
            "<a xmlns:p='u' xmlns:q='u' p:x='1' q:x='2'/>",
            "same attribute twice",
        ),
        ("<a>&nbsp;</a>", "unknown entity"),
        ("<a>&#0;</a>", "does not name a character"),
        ("<a>& b</a>", "must start a reference"),
        ("<a b=c/>", "must be in quotes"),
        ("<a b='<'/>", "not allowed in an attribute value"),
        ("<a>]]></a>", "']]>'"),
        ("<a/><b/>", "only one root element"),
        ("text<a/>", "outside the root element"),
        ("", "no root element"),
        ("<p:a/>", "prefix p is not declared"),
        ("<a p:b='1'/>", "prefix p is not declared"),
        ("<a xmlns:p=''/>", "cannot undeclare"),
        ("<a xmlns:xml='urn:x'/>", "belong together"),
        ("<a xmlns:xmlns='urn:x'/>", "reserved"),
        ("<a:b:c/>", "not a valid element name"),
        ("<a><!-- x -- y --></a>", "'--'"),
        ("<a><![CDATA[x</a>", "not closed"),
        (
            "<?xml version='1.0'?><?xml version='1.0'?><a/>",
            "only once",
        ),
        ("<a>\u{1}</a>", "U+0001"),
        ("<a b='1'c='2'/>", "separated by white space"),
        ("<a></a  b>", "expected '>'"),
        (
            "<!DOCTYPE a [<!ENTITY x 'y'>]><a>&x;</a>",
            "document type declarations",
        ),
        (
            "<?xml version='1.0'?><!DOCTYPE lolz [<!ENTITY lol 'lol'><!ENTITY lol2 '&lol;&lol;'>]><lolz>&lol2;</lolz>",
            "document type",
        ),
    ];
    for (xml, want) in bad {
        let e = Tree::parse(xml).unwrap_err();
        assert!(e.to_string().contains(want), "{xml:?}: {e}");
        let mut doc = Document::from_bytes(with_document(xml)).unwrap();
        let e = doc.xml("word/document.xml").unwrap_err();
        assert!(e.to_string().starts_with("word/document.xml"), "{e}");
    }
    let e = Tree::parse("<a>\n  <b>\n</a>").unwrap_err();
    assert_eq!(e.to_string(), "line 3, column 1: </a> does not close <b>");
}

#[test]
fn other_encodings_are_refused_clearly() {
    let e = Tree::parse("<?xml version='1.0' encoding='ISO-8859-1'?><a/>").unwrap_err();
    assert!(matches!(e, Error::Unsupported(_)), "{e}");
    let e = Tree::parse(b"<a>\xFF</a>".to_vec()).unwrap_err();
    assert!(e.to_string().contains("not valid UTF-8"), "{e}");
    let e = Tree::parse(b"\xFF\xFE<\x00a".to_vec()).unwrap_err();
    assert!(e.to_string().contains("odd number of bytes"), "{e}");
    assert!(matches!(
        Tree::parse("<?xml version='1.1'?><a/>"),
        Err(Error::Unsupported(_))
    ));
}

#[test]
fn bad_entry_names_are_refused() {
    for name in [
        "../evil.xml",
        "/abs.xml",
        "word\\x.xml",
        "word//x.xml",
        "word/./x.xml",
        "a/../../b.xml",
    ] {
        let mut files = entries(&sample());
        files.push((name.to_string(), b"<a/>".to_vec()));
        let refs: Vec<(&str, &[u8])> = files
            .iter()
            .map(|(n, d)| (n.as_str(), d.as_slice()))
            .collect();
        let e = Document::from_bytes(zip(&refs, false)).unwrap_err();
        assert!(matches!(e, Error::PartName { .. }), "{name}: {e}");
    }
}

#[test]
fn duplicate_entry_names_are_refused() {
    let e = Document::from_bytes(sample_with(&[(
        "WORD/document.xml",
        Some(b"<a/>".to_vec()),
    )]))
    .unwrap_err();
    assert!(matches!(e, Error::PartName { .. }), "{e}");
    // Two entries with exactly the same name: written under two names, then
    // renamed in place.
    let mut bytes = sample_with(&[("word/documenX.xml", Some(b"<a/>".to_vec()))]);
    let (from, to) = (b"word/documenX.xml", b"word/document.xml");
    let mut i = 0;
    while let Some(k) = bytes[i..].windows(from.len()).position(|w| w == from) {
        bytes[i + k..i + k + from.len()].copy_from_slice(to);
        i += k + from.len();
    }
    assert!(Document::from_bytes(bytes).is_err());
}

#[test]
fn files_that_are_not_word_documents_are_refused() {
    let cases: Vec<(Vec<u8>, &str)> = vec![
        (vec![], "not a readable"),
        (b"PK\x03\x04 not really".to_vec(), "not a readable"),
        (b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1".to_vec(), "password-protected"),
        (sample_with(&[("[Content_Types].xml", None)]), "no [Content_Types].xml"),
        (sample_with(&[("_rels/.rels", None)]), "no main document"),
        (sample_with(&[("word/document.xml", None)]), "no part word/document.xml"),
        (
            sample_with(&[(
                "[Content_Types].xml",
                Some(
                    String::from_utf8(support::entry(&sample(), "[Content_Types].xml"))
                        .unwrap()
                        .replace("wordprocessingml.document.main", "spreadsheetml.sheet.main")
                        .into_bytes(),
                ),
            )]),
            "content type",
        ),
        (
            sample_with(&[(
                "_rels/.rels",
                Some(
                    String::from_utf8(support::entry(&sample(), "_rels/.rels"))
                        .unwrap()
                        .replace(
                            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument",
                            "http://purl.oclc.org/ooxml/officeDocument/relationships/officeDocument",
                        )
                        .into_bytes(),
                ),
            )]),
            "Strict",
        ),
        (sample_with(&[("[Content_Types].xml", Some(b"<Types/>".to_vec()))]), "does not list content types"),
        (sample_with(&[("_rels/.rels", Some(b"<oops".to_vec()))]), "_rels/.rels"),
    ];
    for (bytes, want) in cases {
        let e = Document::from_bytes(bytes).unwrap_err();
        assert!(e.to_string().contains(want), "want {want:?}: {e}");
    }
}

#[test]
fn an_unreadable_file_names_its_path() {
    let e = Document::open("no/such/file.docx").unwrap_err();
    assert!(
        e.to_string().starts_with("cannot read no/such/file.docx"),
        "{e}"
    );
    let doc = Document::from_bytes(sample()).unwrap();
    let e = doc.save("no/such/dir/out.docx").unwrap_err();
    assert!(
        e.to_string()
            .starts_with("cannot write no/such/dir/out.docx"),
        "{e}"
    );
}

#[test]
fn character_references_take_any_number_of_leading_zeros() {
    let t = Tree::parse("<r>&#000000000000065;&#x0000000000000042;</r>").unwrap();
    assert_eq!(t.text_content(t.root_element()).unwrap(), "AB");
    for bad in [
        "&#0000000000000000;",
        "&#x00000110000;",
        "&#99999999999999999999;",
        "&#x0000FFFE;",
    ] {
        assert!(Tree::parse(format!("<r>{bad}</r>")).is_err(), "{bad}");
    }
    // A long run of digits costs time in proportion to its length.
    let long = format!("<r>&#{}65;</r>", "0".repeat(1 << 20));
    let t = Tree::parse(long).unwrap();
    assert_eq!(t.text_content(t.root_element()).unwrap(), "A");
}

/// An edit to try on a tree.
type Edit<'a> = &'a dyn Fn(&mut Tree) -> Result<(), Error>;

/// A tree of `src` with these limits.
fn limited(src: &str, nodes: usize, depth: usize) -> Tree {
    let limits = Limits {
        max_xml_nodes: nodes,
        max_depth: depth,
        ..Limits::default()
    };
    Tree::parse_with_limits(src, &limits).unwrap()
}

/// `edit` works on a tree of `src` whose `field` is `at`, and with one less
/// it gives the same error as an open over the limit and changes nothing.
fn holds_at(src: &str, field: &str, at: usize, edit: Edit) {
    let tree = |v: usize| match field {
        "max_xml_nodes" => limited(src, v, 256),
        _ => limited(src, 10_000, v),
    };
    let mut t = tree(at);
    edit(&mut t).unwrap_or_else(|e| panic!("{field} {at}: {e}"));
    let mut t = tree(at - 1);
    let before = t.to_bytes();
    let e = edit(&mut t).expect_err("over the limit");
    assert!(
        matches!(&e, Error::Limit { field: f, .. } if *f == field),
        "{field} {}: {e}",
        at - 1
    );
    let shown = e.to_string();
    assert!(shown.contains(&format!("raise Limits::{field}")), "{shown}");
    assert_eq!(t.to_bytes(), before, "a refused edit changed the tree");
}

/// Each edit that makes nodes stops at the node limit. The tree `<a><b/></a>`
/// has 2 nodes.
#[test]
fn every_edit_that_makes_nodes_holds_at_the_node_limit() {
    let src = "<a><b/></a>";
    let b = |t: &Tree| t.elements(t.root_element()).unwrap().next().unwrap();
    let edits: [(usize, Edit); 5] = [
        (1, &|t| t.create_element(None, "c").map(drop)),
        (1, &|t| t.create_text("c").map(drop)),
        (1, &|t| t.set_text(b(t), "c")),
        (3, &|t| t.parse_fragment("<x><y/>z</x>", &[]).map(drop)),
        (2, &|t| t.deep_clone(t.root_element()).map(drop)),
    ];
    for (k, (more, edit)) in edits.into_iter().enumerate() {
        holds_at(src, "max_xml_nodes", 2 + more, edit);
        // A refused edit uses none of the budget: what is left still fits.
        let mut t = limited(src, 2 + more - 1, 256);
        let _ = edit(&mut t);
        for _ in 0..more - 1 {
            t.create_element(None, "e")
                .unwrap_or_else(|e| panic!("edit {k}: {e}"));
        }
        assert!(is_limit(t.create_element(None, "e"), "max_xml_nodes"));
    }
}

/// Each edit that can deepen the tree stops at the depth limit, attached
/// or not. `<a><b><c/></b><d><e/></d></a>` is 3 deep, and each edit here
/// makes it 4 or 5 deep.
#[test]
fn every_edit_that_deepens_holds_at_the_depth_limit() {
    let src = "<a><b><c/></b><d><e/></d></a>";
    let get = |t: &Tree, name: &str| {
        t.descendants(t.document())
            .unwrap()
            .find(|&n| t.is(n, None, name).unwrap())
            .unwrap()
    };
    // A detached x holding y.
    let chain = |t: &mut Tree| -> Result<_, Error> {
        let (x, y) = (t.create_element(None, "x")?, t.create_element(None, "y")?);
        t.append(x, y)?;
        Ok((x, y))
    };
    let edits: [(usize, Edit); 9] = [
        (4, &|t| {
            let n = t.create_element(None, "n")?;
            t.append(get(t, "c"), n)
        }),
        (4, &|t| {
            let n = t.create_element(None, "n")?;
            t.insert(get(t, "c"), 0, n)
        }),
        // A detached subtree moved in.
        (5, &|t| {
            let (x, _) = chain(t)?;
            t.append(get(t, "c"), x)
        }),
        (4, &|t| {
            let (x, _) = chain(t)?;
            t.insert_before(get(t, "c"), x)
        }),
        (4, &|t| {
            let (x, _) = chain(t)?;
            t.insert_after(get(t, "c"), x)
        }),
        // An attached subtree moved deeper.
        (5, &|t| t.append(get(t, "c"), get(t, "d"))),
        // A detached chain made deeper while detached: x, y, z, z is 4
        // deep.
        (4, &|t| {
            let (_, y) = chain(t)?;
            let (z, z2) = (t.create_element(None, "z")?, t.create_element(None, "z")?);
            t.append(z, z2)?;
            t.append(y, z)
        }),
        // A detached subtree that takes in an attached one: x, y, d, e.
        (4, &|t| {
            let (_, y) = chain(t)?;
            t.append(y, get(t, "d"))
        }),
        // A fragment attached deep down.
        (5, &|t| {
            let f = t.parse_fragment("<x><y/></x>", &[])?;
            t.append(get(t, "c"), f[0])
        }),
    ];
    for (at, edit) in edits {
        holds_at(src, "max_depth", at, edit);
    }
    // A fragment deeper than the limit is refused as it is parsed.
    holds_at(src, "max_depth", 4, &|t| {
        t.parse_fragment("<x><y><z><w/></z></y></x>", &[]).map(drop)
    });
}

/// A document's trees keep the document's limits under edits.
#[test]
fn a_documents_trees_keep_its_limits_under_edits() {
    let part = "customXml/n.xml";
    // Opening reads the content types and the package relationships, which
    // are 2 deep and have fewer nodes than this part's 301.
    let bytes = with_part(part, &format!("<a>{}</a>", "<b/>".repeat(300)));
    let limits = Limits {
        max_xml_nodes: 302,
        max_depth: 2,
        ..Limits::default()
    };
    let mut doc = Document::from_bytes_with_limits(bytes, limits).unwrap();
    let t = doc.xml(part).unwrap();
    let b = t.elements(t.root_element()).unwrap().next().unwrap();
    let c = t.create_element(None, "c").unwrap();
    assert!(is_limit(t.append(b, c), "max_depth"));
    assert!(is_limit(t.create_element(None, "d"), "max_xml_nodes"));
    // A tree made without a document has the default limits.
    let mut t = Tree::parse("<a/>").unwrap();
    let mut n = t.root_element();
    for _ in 1..Limits::default().max_depth {
        let c = t.create_element(None, "c").unwrap();
        t.append(n, c).unwrap();
        n = c;
    }
    let c = t.create_element(None, "c").unwrap();
    assert!(is_limit(t.append(n, c), "max_depth"));
}
