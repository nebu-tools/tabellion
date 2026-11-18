//! The parser against the W3C XML Conformance Test Suite.
//!
//! `tools/xmlconf.sh` downloads the suite (release 20130923) and runs this
//! test with `TABELLION_XMLCONF` set to its `xmlconf` directory. The test
//! takes every XML 1.0 and Namespaces 1.0 case that a parser without
//! document type declarations can decide: cases without a DOCTYPE, for the
//! fifth edition, that are namespace-well-formed or not well-formed. Each
//! must be accepted (valid, invalid) or refused (not well-formed). An
//! accepted case must also write back its bytes.
//!
//! The cases where the parser disagrees on purpose are listed in
//! `tests/xmlconf-expected.txt` with the reason. The test fails on any other
//! disagreement, and on a listed one that no longer disagrees.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use nebu_docx::xml::Tree;

const MANIFESTS: [&str; 16] = [
    "xmltest/xmltest.xml",
    "japanese/japanese.xml",
    "sun/sun-valid.xml",
    "sun/sun-invalid.xml",
    "sun/sun-not-wf.xml",
    "sun/sun-error.xml",
    "oasis/oasis.xml",
    "ibm/ibm_oasis_invalid.xml",
    "ibm/ibm_oasis_not-wf.xml",
    "ibm/ibm_oasis_valid.xml",
    "eduni/errata-2e/errata2e.xml",
    "eduni/namespaces/1.0/rmt-ns10.xml",
    "eduni/errata-3e/errata3e.xml",
    "eduni/errata-4e/errata4e.xml",
    "eduni/namespaces/errata-1e/errata1e.xml",
    "eduni/misc/ht-bh.xml",
];

struct Case {
    id: String,
    accept: bool,
    file: PathBuf,
}

/// True if the bytes contain a document type declaration, in UTF-8 or
/// UTF-16.
fn has_doctype(b: &[u8]) -> bool {
    let utf16 = |be: bool| -> Vec<u8> {
        "<!DOCTYPE"
            .encode_utf16()
            .flat_map(|u| if be { u.to_be_bytes() } else { u.to_le_bytes() })
            .collect()
    };
    [b"<!DOCTYPE".to_vec(), utf16(true), utf16(false)]
        .iter()
        .any(|p| b.windows(p.len()).any(|w| w == p.as_slice()))
}

fn cases(root: &Path) -> Vec<Case> {
    let mut out = vec![];
    for m in MANIFESTS {
        let path = root.join(m);
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{m}: {e}"));
        let opts = || roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        };
        // Some manifests are lists of TEST elements without a root.
        let body = match text.find("?>") {
            Some(k) if text.starts_with("<?xml") => &text[k + 2..],
            _ => &text[..],
        };
        let wrapped = format!("<ALL>{body}</ALL>");
        let doc = roxmltree::Document::parse_with_options(&text, opts())
            .or_else(|_| roxmltree::Document::parse_with_options(&wrapped, opts()))
            .unwrap_or_else(|e| panic!("{m}: {e}"));
        let dir = path.parent().unwrap();
        for t in doc.descendants().filter(|n| n.has_tag_name("TEST")) {
            let a = |k| t.attribute(k).unwrap_or("");
            if matches!(a("RECOMMENDATION"), "XML1.1" | "NS1.1")
                || a("VERSION") == "1.1"
                || (!a("EDITION").is_empty() && !a("EDITION").split(' ').any(|e| e == "5"))
                || a("NAMESPACE") == "no"
            {
                continue;
            }
            let accept = match a("TYPE") {
                "valid" | "invalid" => true,
                "not-wf" => false,
                _ => continue,
            };
            // A TESTCASES element may set its own base.
            let base = t
                .ancestors()
                .filter_map(|n| n.attribute(("http://www.w3.org/XML/1998/namespace", "base")))
                .collect::<Vec<_>>();
            let mut file = dir.to_path_buf();
            for b in base.iter().rev() {
                file.push(b);
            }
            file.push(a("URI"));
            if !file.exists() {
                file = dir.join(a("URI"));
            }
            let bytes = std::fs::read(&file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
            if has_doctype(&bytes) || a("ENTITIES") != "none" && !a("ENTITIES").is_empty() {
                continue;
            }
            out.push(Case {
                id: a("ID").to_string(),
                accept,
                file,
            });
        }
    }
    out
}

#[test]
#[ignore = "needs the W3C XML test suite; run tools/xmlconf.sh"]
fn the_w3c_conformance_suite() {
    let Some(root) = std::env::var_os("TABELLION_XMLCONF") else {
        eprintln!("TABELLION_XMLCONF is not set; tools/xmlconf.sh sets it");
        return;
    };
    let expected_text = include_str!("xmlconf-expected.txt");
    let expected: BTreeMap<&str, &str> = expected_text
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .map(|l| l.split_once(' ').unwrap_or((l, "")))
        .collect();
    let cases = cases(Path::new(&root));
    assert!(cases.len() > 300, "only {} cases found", cases.len());
    let mut disagree = BTreeMap::new();
    let (mut accepted, mut refused) = (0, 0);
    for c in &cases {
        let bytes = std::fs::read(&c.file).unwrap();
        let got = std::panic::catch_unwind(|| Tree::parse(bytes.clone()));
        let Ok(got) = got else {
            disagree.insert(c.id.clone(), "PANIC".to_string());
            continue;
        };
        match (&got, c.accept) {
            (Ok(t), true) => {
                accepted += 1;
                assert!(t.to_bytes() == bytes, "{} does not write back", c.id);
            }
            (Err(e), false) => {
                refused += 1;
                if std::env::var_os("TABELLION_XMLCONF_VERBOSE").is_some() {
                    eprintln!("refused {}: {e}", c.id);
                }
            }
            (Ok(_), false) => {
                disagree.insert(c.id.clone(), "accepted, but not well-formed".to_string());
            }
            (Err(e), true) => {
                disagree.insert(c.id.clone(), format!("refused: {e}"));
            }
        }
    }
    eprintln!(
        "{} cases: {accepted} accepted and {refused} refused as the suite says, {} disagreements",
        cases.len(),
        disagree.len()
    );
    let mut report = String::new();
    for (id, why) in &disagree {
        if !expected.contains_key(id.as_str()) {
            report.push_str(&format!("new disagreement: {id} {why}\n"));
        }
    }
    for id in expected.keys() {
        if !disagree.contains_key(*id) {
            report.push_str(&format!("listed but agrees now: {id}\n"));
        }
    }
    assert!(report.is_empty(), "{report}");
}
