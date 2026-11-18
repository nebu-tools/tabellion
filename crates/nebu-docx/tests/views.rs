//! The views on documents written by hand: what they report, where they
//! refuse, and that hostile input costs neither a panic nor a deep stack.

mod support;

use nebu_docx::{
    BlockKind, Document, Error, Inline, Limits, Locator, Revisions, StoryKind, TextOptions,
};
use proptest::prelude::*;

const NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:x="urn:example:unknown""#;

/// The sample template with this body.
fn with_body(body: &str, limits: Limits) -> Document {
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><w:document {NS}><w:body>{body}</w:body></w:document>"#
    );
    let bytes = support::sample_with(&[("word/document.xml", Some(xml.into_bytes()))]);
    Document::from_bytes_with_limits(bytes, limits).unwrap()
}

fn doc(body: &str) -> Document {
    with_body(body, Limits::default())
}

fn p(text: &str) -> String {
    format!("<w:p><w:r><w:t xml:space=\"preserve\">{text}</w:t></w:r></w:p>")
}

/// Every view, the text with every option, and every locator kind.
fn all_views(d: &mut Document) -> Result<(), Error> {
    d.blocks()?;
    d.styles()?;
    d.tables()?;
    d.fields()?;
    d.revisions()?;
    d.comments()?;
    d.images()?;
    d.bookmarks()?;
    d.notes()?;
    d.sections()?;
    d.numbering()?;
    d.content_controls()?;
    for revisions in [Revisions::Accepted, Revisions::Rejected, Revisions::Current] {
        d.text(&TextOptions {
            revisions,
            headers_footers: true,
            notes: true,
            comments: true,
            text_boxes: true,
        })?;
    }
    for l in ["heading:x", "bookmark:x", "placeholder:x", "text:x", "#0"] {
        match d.locate(&l.parse()?) {
            Ok(_) | Err(Error::Invalid(_)) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Runs `f` on a thread with a small stack, so that recursion in proportion
/// to the input would overflow it.
fn small_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap()
}

fn deep() -> Limits {
    Limits {
        max_depth: 1_000_000,
        ..Limits::default()
    }
}

#[test]
fn deeply_nested_tables_cost_no_stack() {
    let n = 2000;
    let mut body = String::new();
    for _ in 0..n {
        body.push_str("<w:tbl><w:tr><w:tc>");
    }
    body.push_str(&p("core"));
    for _ in 0..n {
        body.push_str("</w:tc></w:tr></w:tbl><w:p/>");
    }
    let tables = small_stack(move || {
        let mut d = with_body(&body, deep());
        all_views(&mut d).unwrap();
        d.tables().unwrap()
    });
    assert_eq!(tables.len(), n);
    assert_eq!(tables[n - 1].cells[0].text, "core");
    assert_eq!(tables[n - 1].parent.unwrap().table, n - 2);
    assert_eq!(tables[0].cells[0].tables, vec![1]);
}

#[test]
fn deeply_nested_controls_changes_links_and_fields_cost_no_stack() {
    let n = 3000;
    let mut body = String::from("<w:p>");
    for _ in 0..n {
        body.push_str(r#"<w:sdt><w:sdtContent><w:ins w:author="Wren Ashby"><w:hyperlink w:anchor="a"><w:smartTag>"#);
    }
    for i in 0..n {
        body.push_str(&format!(
            r#"<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText> F{i} </w:instrText></w:r>"#
        ));
    }
    body.push_str("<w:r><w:t>deep</w:t></w:r>");
    for _ in 0..n {
        body.push_str(r#"<w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>x</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r>"#);
    }
    for _ in 0..n {
        body.push_str("</w:smartTag></w:hyperlink></w:ins></w:sdtContent></w:sdt>");
    }
    body.push_str("</w:p>");
    let (fields, controls, revisions) = small_stack(move || {
        let mut d = with_body(&body, deep());
        all_views(&mut d).unwrap();
        let main = |l: &nebu_docx::Location| l.story == StoryKind::Main;
        (
            d.fields()
                .unwrap()
                .into_iter()
                .filter(|f| main(&f.location))
                .collect::<Vec<_>>(),
            d.content_controls()
                .unwrap()
                .into_iter()
                .filter(|c| main(&c.location))
                .collect::<Vec<_>>(),
            d.revisions().unwrap(),
        )
    });
    assert_eq!(fields.len(), n);
    assert_eq!(controls.len(), n);
    assert_eq!(fields[n - 1].parent, Some(n - 2));
    // The innermost code ends where its own separator is.
    assert_eq!(fields[n - 1].code, format!("F{} deep", n - 1));
    assert!(fields[0].code.starts_with("F0 { F1 { F2"));
    assert!(!revisions.is_empty());
}

#[test]
fn ranges_open_at_once_stay_within_the_limit() {
    let mut body = String::new();
    for i in 0..3000 {
        body.push_str(&format!(r#"<w:bookmarkStart w:id="{i}" w:name="b{i}"/>"#));
    }
    for _ in 0..3000 {
        body.push_str(&p(&"a".repeat(80)));
    }
    let limits = Limits {
        max_total_size: 4 << 20,
        ..Limits::default()
    };
    let mut d = with_body(&body, limits);
    match d.bookmarks() {
        Err(Error::Limit { field, .. }) => assert_eq!(field, "max_total_size"),
        other => panic!("{other:?}"),
    }
    // The views are read together, so all of them give the error.
    assert!(matches!(d.blocks(), Err(Error::Limit { .. })));
    // Within the limit, the same document reads.
    let mut d = with_body(&body, Limits::default());
    assert_eq!(d.bookmarks().unwrap().len(), 3000);
    assert_eq!(d.blocks().unwrap().len(), 6000);
}

#[test]
fn a_table_wider_than_any_person_makes_is_refused() {
    let mut rows = String::new();
    for _ in 0..200 {
        rows.push_str(
            r#"<w:tr><w:tc><w:tcPr><w:gridSpan w:val="1000000"/></w:tcPr><w:p/></w:tc></w:tr>"#,
        );
    }
    let mut d = doc(&format!("<w:tbl>{rows}</w:tbl>"));
    assert!(matches!(d.tables(), Err(Error::Unsupported(_))));
    assert!(matches!(d.blocks(), Err(Error::Unsupported(_))));
}

#[test]
fn broken_field_structure_gives_views() {
    let body = [
        r#"<w:p><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>stray</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>"#,
        r#"<w:p><w:r><w:instrText>LOOSE</w:instrText></w:r><w:fldChar w:fldCharType="begin"/></w:p>"#,
        r#"<w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText>OPEN</w:instrText></w:r></w:p>"#,
        r#"<w:p><w:fldSimple w:instr="SIMPLE"><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText>INNER</w:instrText></w:r></w:fldSimple><w:r><w:t>after</w:t></w:r></w:p>"#,
    ]
    .concat();
    let mut d = doc(&body);
    all_views(&mut d).unwrap();
    let fields = d.fields().unwrap();
    let codes: Vec<&str> = fields
        .iter()
        .filter(|f| f.location.story == StoryKind::Main)
        .map(|f| f.code.as_str())
        .collect();
    // The field that never ends holds the rest of the story in its code.
    assert_eq!(codes.len(), 3);
    assert!(codes[0].starts_with("OPEN"));
    assert_eq!(codes[1..], ["SIMPLE", "INNER"]);
    let blocks = d.blocks().unwrap();
    assert_eq!(blocks[0].text, "stray");
    // The open field holds the rest of the story in its code.
    assert_eq!(blocks[3].text, "");
}

#[test]
fn unknown_content_is_reported_and_kept_in_place() {
    let body = [
        "<x:thing><w:p/></x:thing>",
        &p("before"),
        r#"<w:p><w:r><w:t>a</w:t></w:r><x:mark/><mc:AlternateContent><mc:Choice Requires="x"><w:r><w:t>no</w:t></w:r></mc:Choice></mc:AlternateContent><w:r><w:t>b</w:t></w:r></w:p>"#,
    ]
    .concat();
    let mut d = doc(&body);
    let blocks = d.blocks().unwrap();
    assert_eq!(blocks[0].kind, BlockKind::Other);
    assert_eq!(blocks[0].element.as_deref(), Some("x:thing"));
    let content = &blocks[2].content;
    let opaque: Vec<&str> = content
        .iter()
        .filter_map(|i| match i {
            Inline::Opaque { element } => Some(element.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(opaque, ["x:mark", "mc:AlternateContent"]);
    assert_eq!(blocks[2].text, "ab");
}

#[test]
fn alternate_content_is_read_once_from_the_branch_word_takes() {
    let body = r#"<w:p><mc:AlternateContent><mc:Choice Requires="w14"><w:r><w:t>new</w:t></w:r></mc:Choice><mc:Fallback><w:r><w:t>old</w:t></w:r></mc:Fallback></mc:AlternateContent><mc:AlternateContent><mc:Choice Requires="x"><w:r><w:t>unknown</w:t></w:r></mc:Choice><mc:Fallback><w:r><w:t> fallback</w:t></w:r></mc:Fallback></mc:AlternateContent></w:p>"#;
    let mut d = doc(body);
    assert_eq!(d.blocks().unwrap()[0].text, "new fallback");
}

#[test]
fn ids_are_stable_where_the_file_allows() {
    let body = [
        r#"<w:p w14:paraId="0A0B0C0D"><w:r><w:t>one</w:t></w:r></w:p>"#,
        r#"<w:p w14:paraId="11111111"><w:r><w:t>twin</w:t></w:r></w:p>"#,
        r#"<w:p w14:paraId="11111111"><w:r><w:t>twin</w:t></w:r></w:p>"#,
        r#"<w:tbl><w:tr><w:tc><w:p w14:paraId="2A2B2C2D"/></w:tc></w:tr></w:tbl>"#,
        &p("no id"),
    ]
    .concat();
    let mut d = doc(&body);
    let ids: Vec<String> = d.blocks().unwrap().into_iter().map(|b| b.id).collect();
    assert_eq!(ids, ["p:0A0B0C0D", "#1", "#2", "t:2A2B2C2D", "#4"]);
    assert_eq!(d.sha256().len(), 64);
}

#[test]
fn locators_name_one_block_or_say_why_not() {
    let body = [
        r#"<w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Results</w:t></w:r></w:p>"#,
        r#"<w:bookmarkStart w:id="1" w:name="here"/>"#,
        &p("{{results}}"),
        r#"<w:bookmarkEnd w:id="1"/>"#,
        &p("twice"),
        &p("twice"),
    ]
    .concat();
    let mut d = doc(&body);
    let at = |d: &mut Document, s: &str| d.locate(&s.parse().unwrap()).map(|b| b.index);
    assert_eq!(at(&mut d, "heading:Results").unwrap(), 0);
    assert_eq!(at(&mut d, "bookmark:here").unwrap(), 2);
    assert_eq!(at(&mut d, "placeholder:results").unwrap(), 2);
    assert_eq!(at(&mut d, "#4").unwrap(), 4);
    let many = at(&mut d, "text:twice").unwrap_err().to_string();
    assert!(
        many.starts_with("2 blocks match the text \"twice\""),
        "{many}"
    );
    let none = at(&mut d, "heading:Methods").unwrap_err().to_string();
    assert_eq!(none, "no block matches the heading \"Methods\"");
    assert!(matches!(
        "street:1".parse::<Locator>(),
        Err(Error::Invalid(_))
    ));
}

#[test]
fn styles_are_found_by_id_name_and_the_name_word_shows() {
    let mut d = doc(&p("x"));
    let styles = d.styles().unwrap();
    let h = styles.find("Heading1").unwrap();
    assert_eq!(styles.find("heading 1").unwrap().id, h.id);
    assert_eq!(styles.find("Heading 1").unwrap().id, h.id);
    assert_eq!(styles.find("Überschrift 1").unwrap().id, h.id);
    assert_eq!(styles.find("titre 1").unwrap().id, h.id);
    assert_eq!(h.outline_level, Some(1));
    assert!(styles.find("Überschrift 7").is_none());
}

#[test]
fn an_edit_makes_the_views_read_again() {
    let mut d = doc(&p("before"));
    assert_eq!(d.blocks().unwrap()[0].text, "before");
    let main = d.main_part().to_string();
    let t = d.xml(&main).unwrap();
    let body = t
        .child(t.root_element(), Some(nebu_docx::ns::W), "body")
        .unwrap()
        .unwrap();
    let new = t
        .parse_fragment(
            "<w:p><w:r><w:t>first</w:t></w:r></w:p>",
            &[("w", nebu_docx::ns::W)],
        )
        .unwrap();
    t.insert(body, 0, new[0]).unwrap();
    let texts: Vec<String> = d.blocks().unwrap().into_iter().map(|b| b.text).collect();
    assert_eq!(texts, ["first", "before"]);
    // A text change inside a block counts too.
    let t = d.xml(&main).unwrap();
    let text_node = t
        .descendants(t.root_element())
        .unwrap()
        .find(|&n| t.text(n).unwrap().as_deref() == Some("first"))
        .unwrap();
    t.set_text(text_node, "changed").unwrap();
    assert_eq!(d.blocks().unwrap()[0].text, "changed");
}

#[test]
fn deleted_paragraph_marks_join_paragraphs_in_the_text() {
    let body = [
        r#"<w:p><w:pPr><w:rPr><w:del w:id="1" w:author="Wren Ashby"/></w:rPr></w:pPr><w:r><w:t>one</w:t></w:r></w:p>"#,
        r#"<w:p><w:r><w:t> two</w:t></w:r><w:ins w:id="2" w:author="Pip Tanager"><w:r><w:t> three</w:t></w:r></w:ins></w:p>"#,
    ]
    .concat();
    let mut d = doc(&body);
    let text = |d: &mut Document, revisions| {
        d.text(&TextOptions {
            revisions,
            ..TextOptions::default()
        })
        .unwrap()
    };
    assert_eq!(text(&mut d, Revisions::Accepted), "one two three\n");
    assert_eq!(text(&mut d, Revisions::Rejected), "one\n two\n");
    assert_eq!(text(&mut d, Revisions::Current), "one\n two three\n");
}

/// Random bodies from the elements the walk knows, nested at random.
fn element() -> impl Strategy<Value = String> {
    let leaf = prop_oneof![
        Just("<w:r><w:t>a</w:t></w:r>".to_string()),
        Just(r#"<w:r><w:fldChar w:fldCharType="begin"/></w:r>"#.to_string()),
        Just(r#"<w:r><w:fldChar w:fldCharType="separate"/></w:r>"#.to_string()),
        Just(r#"<w:r><w:fldChar w:fldCharType="end"/></w:r>"#.to_string()),
        Just("<w:r><w:instrText>REF x</w:instrText></w:r>".to_string()),
        Just(r#"<w:bookmarkStart w:id="1" w:name="b"/>"#.to_string()),
        Just(r#"<w:bookmarkEnd w:id="1"/>"#.to_string()),
        Just(r#"<w:commentRangeStart w:id="0"/>"#.to_string()),
        Just(r#"<w:r><w:footnoteReference w:id="1"/></w:r>"#.to_string()),
        Just("<w:r><w:tab/><w:br/><w:sym w:char=\"F0FC\"/></w:r>".to_string()),
        Just(r#"<w:r><w:rPr><w:vanish/></w:rPr><w:t>h</w:t></w:r>"#.to_string()),
        Just("<w:sectPr/>".to_string()),
        Just("<x:unknown/>".to_string()),
        Just("<w:p/>".to_string()),
    ];
    leaf.prop_recursive(6, 64, 6, |inner| {
        let kids = prop::collection::vec(inner, 0..6).prop_map(|v| v.concat());
        (0..14usize, kids).prop_map(|(k, c)| match k {
            0 => format!("<w:p>{c}</w:p>"),
            1 => format!("<w:tbl><w:tr><w:tc>{c}</w:tc></w:tr></w:tbl>"),
            2 => format!(r#"<w:tbl><w:tr><w:tc><w:tcPr><w:vMerge/><w:gridSpan w:val="3"/></w:tcPr>{c}</w:tc></w:tr></w:tbl>"#),
            3 => format!("<w:tr><w:tc>{c}</w:tc></w:tr>"),
            4 => format!("<w:tc>{c}</w:tc>"),
            5 => format!("<w:sdt><w:sdtPr><w:text/></w:sdtPr><w:sdtContent>{c}</w:sdtContent></w:sdt>"),
            6 => format!(r#"<w:ins w:author="Wren Ashby">{c}</w:ins>"#),
            7 => format!(r#"<w:del w:author="Pip Tanager">{c}</w:del>"#),
            8 => format!(r#"<w:fldSimple w:instr="SEQ T">{c}</w:fldSimple>"#),
            9 => format!(r#"<w:hyperlink w:anchor="b">{c}</w:hyperlink>"#),
            10 => format!(r#"<mc:AlternateContent><mc:Choice Requires="w14">{c}</mc:Choice><mc:Fallback>{c}</mc:Fallback></mc:AlternateContent>"#),
            11 => format!("<w:r>{c}</w:r>"),
            12 => format!("<w:customXml>{c}</w:customXml>"),
            _ => format!("<w:smartTag>{c}</w:smartTag>"),
        })
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    #[test]
    fn any_body_gives_views_without_a_panic(parts in prop::collection::vec(element(), 0..8)) {
        let mut d = doc(&parts.concat());
        // An error is allowed; a panic is not.
        let _ = all_views(&mut d);
        // Blocks and text agree on what a reader sees.
        if let (Ok(blocks), Ok(text)) = (d.blocks(), d.text(&TextOptions { text_boxes: false, ..TextOptions::default() })) {
            // Blocks other than paragraphs and tables end no line, so the
            // comparison leaves white space out.
            let bare = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
            let joined: String = blocks.iter().map(|b| b.text.as_str()).collect();
            prop_assert_eq!(bare(&joined), bare(&text));
        }
    }
}
