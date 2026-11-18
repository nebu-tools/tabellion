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
    // Saving copies the entry without decompressing it.
    assert_eq!(
        support::raw_entries(&doc.to_bytes().unwrap()),
        support::raw_entries(&bytes)
    );
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
    let deepest = t.descendants(t.root_element()).last().unwrap();
    let new = t.create_element(Some(nebu_docx::ns::W), "w:p").unwrap();
    t.append(deepest, new).unwrap();
    let copy = t.deep_clone(t.root_element()).unwrap();
    assert!(t.descendants(copy).count() > n);
    let out = doc.to_bytes().unwrap();
    assert!(
        String::from_utf8(support::entry(&out, "word/document.xml"))
            .unwrap()
            .contains("<w:sdt><w:p/></w:sdt>")
    );
}

#[test]
fn too_many_nodes_stop_at_the_node_limit() {
    let xml = format!("<a>{}</a>", "<b/>".repeat(5000));
    let limits = Limits {
        max_xml_nodes: 1000,
        ..Limits::default()
    };
    let mut doc = Document::from_bytes_with_limits(with_document(&xml), limits).unwrap();
    assert!(matches!(
        doc.xml("word/document.xml"),
        Err(Error::Limit {
            field: "max_xml_nodes",
            ..
        })
    ));
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
