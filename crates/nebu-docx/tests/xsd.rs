//! The schema validator against documents Word wrote: what Word saves is
//! valid, and the deliberate errors of a broken part are found.

mod support;

use std::path::Path;

#[test]
fn documents_word_wrote_are_valid() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus");
    let mut failures = vec![];
    for f in [
        "birds",
        "parts",
        "changes",
        "cascade",
        "recipes-de",
        "recettes-fr",
    ] {
        let bytes = std::fs::read(dir.join(format!("{f}.docx"))).unwrap();
        for e in support::xsd::validate_docx(&bytes) {
            failures.push(format!("{f}.docx: {e}"));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn errors_are_found() {
    let w = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
    let bad = format!(
        r#"<w:document xmlns:w="{w}"><w:body><w:p><w:r><w:t>a</w:t></w:r><w:pPr/></w:p><w:tbl/><w:p><w:pPr><w:jc w:val="middle"/></w:pPr></w:p><w:frog/></w:body></w:document>"#
    );
    let e = support::xsd::validate(bad.as_bytes());
    assert_eq!(e.len(), 4, "{e:#?}");
    let good = format!(
        r#"<w:document xmlns:w="{w}"><w:body><w:p><w:pPr><w:jc w:val="center"/></w:pPr><w:r><w:t xml:space="preserve"> a</w:t></w:r></w:p></w:body></w:document>"#
    );
    assert_eq!(
        support::xsd::validate(good.as_bytes()),
        Vec::<String>::new()
    );
}
