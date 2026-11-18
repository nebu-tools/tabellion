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
    let v = d.views()?;
    for revisions in [Revisions::Accepted, Revisions::Rejected, Revisions::Current] {
        v.text(&TextOptions {
            revisions,
            headers_footers: true,
            notes: true,
            comments: true,
            text_boxes: true,
        });
    }
    for l in ["heading:x", "bookmark:x", "placeholder:x", "text:x", "#0"] {
        match v.locate(&l.parse()?) {
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
        d.views().unwrap().tables().to_vec()
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
            d.views()
                .unwrap()
                .fields()
                .iter()
                .filter(|f| main(&f.location))
                .cloned()
                .collect::<Vec<_>>(),
            d.views()
                .unwrap()
                .content_controls()
                .iter()
                .filter(|c| main(&c.location))
                .cloned()
                .collect::<Vec<_>>(),
            d.views().unwrap().revisions().to_vec(),
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
    match d.views() {
        Err(Error::Limit { field, .. }) => assert_eq!(field, "max_total_size"),
        other => panic!("{other:?}"),
    }
    // Within the limit, the same document reads.
    let mut d = with_body(&body, Limits::default());
    let v = d.views().unwrap();
    assert_eq!(v.bookmarks().len(), 3000);
    assert_eq!(v.blocks().len(), 6000);
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
    assert!(matches!(d.views(), Err(Error::Unsupported(_))));
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
    let fields = d.views().unwrap().fields().to_vec();
    let codes: Vec<&str> = fields
        .iter()
        .filter(|f| f.location.story == StoryKind::Main)
        .map(|f| f.code.as_str())
        .collect();
    // The field that never ends holds the rest of the story in its code.
    assert_eq!(codes.len(), 3);
    assert!(codes[0].starts_with("OPEN"));
    assert_eq!(codes[1..], ["SIMPLE", "INNER"]);
    let blocks = d.views().unwrap().blocks().to_vec();
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
    let blocks = d.views().unwrap().blocks().to_vec();
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
    assert_eq!(d.views().unwrap().blocks()[0].text, "new fallback");
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
    let ids: Vec<String> = d
        .views()
        .unwrap()
        .blocks()
        .iter()
        .map(|b| b.id.to_string())
        .collect();
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
    let at = |d: &mut Document, s: &str| {
        d.views()
            .unwrap()
            .locate(&s.parse().unwrap())
            .map(|b| b.index)
    };
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
    let styles = d.views().unwrap().styles().clone();
    let h = styles.find("Heading1").unwrap();
    assert_eq!(styles.find("heading 1").unwrap().id, h.id);
    assert_eq!(styles.find("Heading 1").unwrap().id, h.id);
    assert_eq!(h.outline_level, Some(1));
    assert!(styles.find("Heading 7").is_none());
}

#[test]
fn an_edit_makes_the_views_read_again() {
    let mut d = doc(&p("before"));
    assert_eq!(d.views().unwrap().blocks()[0].text, "before");
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
    let texts: Vec<String> = d
        .views()
        .unwrap()
        .blocks()
        .iter()
        .map(|b| b.text.clone())
        .collect();
    assert_eq!(texts, ["first", "before"]);
    // A text change inside a block counts too.
    let t = d.xml(&main).unwrap();
    let text_node = t
        .descendants(t.root_element())
        .unwrap()
        .find(|&n| t.text(n).unwrap().as_deref() == Some("first"))
        .unwrap();
    t.set_text(text_node, "changed").unwrap();
    assert_eq!(d.views().unwrap().blocks()[0].text, "changed");
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
        d.views().unwrap().text(&TextOptions {
            revisions,
            ..TextOptions::default()
        })
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
        if let Ok(v) = d.views() {
            let blocks = v.blocks();
            let text = v.text(&TextOptions { text_boxes: false, ..TextOptions::default() });
            // Blocks other than paragraphs and tables end no line, so the
            // comparison leaves white space out.
            let bare = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
            let joined: String = blocks.iter().map(|b| b.text.as_str()).collect();
            prop_assert_eq!(bare(&joined), bare(&text));
        }
    }
}

// ---- One definition of visible text ------------------------------------------

const W_NS: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// The sample template with this body, these styles added to its own,
/// these footnotes and these comments.
fn package(body: &str, styles: &str, footnotes: &str, comments: &str) -> Document {
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><w:document {NS}><w:body>{body}</w:body></w:document>"#
    );
    let sample = support::sample();
    let part = |name: &str| String::from_utf8(support::entry(&sample, name)).unwrap();
    let style_xml = part("word/styles.xml").replace("</w:styles>", &format!("{styles}</w:styles>"));
    let notes = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><w:footnotes xmlns:w="{W_NS}"><w:footnote w:type="separator" w:id="-1"><w:p><w:r><w:separator/></w:r></w:p></w:footnote>{footnotes}</w:footnotes>"#
    );
    let comments_xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><w:comments xmlns:w="{W_NS}">{comments}</w:comments>"#
    );
    let rels = part("word/_rels/document.xml.rels").replace(
        "</Relationships>",
        &format!(r#"<Relationship Id="rId90" Type="{REL}/comments" Target="comments.xml"/></Relationships>"#),
    );
    let types = part("[Content_Types].xml").replace(
        "</Types>",
        r#"<Override PartName="/word/comments.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml"/></Types>"#,
    );
    let bytes = support::sample_with(&[
        ("word/document.xml", Some(xml.into_bytes())),
        ("word/styles.xml", Some(style_xml.into_bytes())),
        ("word/footnotes.xml", Some(notes.into_bytes())),
        ("word/comments.xml", Some(comments_xml.into_bytes())),
        ("word/_rels/document.xml.rels", Some(rels.into_bytes())),
        ("[Content_Types].xml", Some(types.into_bytes())),
    ]);
    Document::from_bytes(bytes).unwrap()
}

/// `OLD` replaced by `NEW` as a tracked change.
const REPLACED: &str = r#"<w:r><w:t xml:space="preserve">Count </w:t></w:r><w:del w:id="1" w:author="Wren Ashby"><w:r><w:delText>OLD</w:delText></w:r></w:del><w:ins w:id="2" w:author="Wren Ashby"><w:r><w:t>NEW</w:t></w:r></w:ins>"#;

fn text_of(d: &mut Document, revisions: Revisions) -> String {
    d.views().unwrap().text(&TextOptions {
        revisions,
        notes: true,
        comments: true,
        ..TextOptions::default()
    })
}

#[test]
fn notes_and_comments_follow_the_view_of_revisions() {
    let body = format!(
        r#"<w:p><w:commentRangeStart w:id="5"/>{REPLACED}<w:commentRangeEnd w:id="5"/><w:r><w:commentReference w:id="5"/></w:r><w:r><w:footnoteReference w:id="1"/></w:r></w:p>"#
    );
    let note = format!(r#"<w:footnote w:id="1"><w:p>{REPLACED}</w:p></w:footnote>"#);
    let comment = format!(
        r#"<w:comment w:id="5" w:author="Pip Tanager" w:initials="PT"><w:p>{REPLACED}</w:p></w:comment>"#
    );
    let mut d = package(&body, "", &note, &comment);
    let accepted = text_of(&mut d, Revisions::Accepted);
    assert_eq!(
        accepted,
        "Count NEW[1]\n\n[1] Count NEW\n\n[Comment by Pip Tanager on \u{201C}Count NEW\u{201D}] Count NEW\n"
    );
    let rejected = text_of(&mut d, Revisions::Rejected);
    assert_eq!(
        rejected,
        "Count OLD[1]\n\n[1] Count OLD\n\n[Comment by Pip Tanager on \u{201C}Count OLD\u{201D}] Count OLD\n"
    );
    let current = text_of(&mut d, Revisions::Current);
    assert!(
        current.contains("[1] Count OLDNEW\n") && current.contains("] Count OLDNEW\n"),
        "{current}"
    );
    // The views show revisions accepted.
    let v = d.views().unwrap();
    assert_eq!(v.notes()[0].text, "Count NEW");
    assert_eq!(v.comments()[0].text, "Count NEW");
}

#[test]
fn a_bookmark_reads_as_the_text_reads() {
    let body = [
        r#"<w:p><w:pPr><w:rPr><w:del w:id="1" w:author="Wren Ashby"/></w:rPr></w:pPr><w:bookmarkStart w:id="1" w:name="joined"/><w:r><w:t>Alpha</w:t></w:r></w:p>"#,
        r#"<w:p><w:r><w:t>Beta</w:t></w:r><w:bookmarkEnd w:id="1"/></w:p>"#,
        r#"<w:tbl><w:tr><w:tc><w:p><w:bookmarkStart w:id="2" w:name="cells"/><w:r><w:t>wren</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>robin</w:t></w:r></w:p><w:p><w:r><w:t>dunnock</w:t></w:r><w:bookmarkEnd w:id="2"/></w:p></w:tc></w:tr></w:tbl>"#,
        r#"<w:bookmarkStart w:id="3" w:name="table"/><w:tbl><w:tr><w:tc><w:p><w:r><w:t>a</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>b</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:bookmarkEnd w:id="3"/>"#,
    ]
    .concat();
    let mut d = doc(&body);
    let v = d.views().unwrap();
    let text = v.text(&TextOptions::default());
    assert_eq!(text, "AlphaBeta\nwren\trobin dunnock\na\tb\n");
    let marks: Vec<(&str, &str)> = v
        .bookmarks()
        .iter()
        .map(|b| (b.name.as_str(), b.text.as_str()))
        .collect();
    assert_eq!(
        marks,
        [
            ("joined", "AlphaBeta"),
            // Inside one table, cells are separated by tabs and the
            // paragraphs of a cell by line ends.
            ("cells", "wren\trobin\ndunnock"),
            ("table", "a\tb"),
        ]
    );
    // Each block's text is the text of its part of the body.
    let blocks: Vec<&str> = v.blocks().iter().map(|b| b.text.as_str()).collect();
    assert_eq!(blocks[..2], ["Alpha", "Beta"]);
    assert_eq!(blocks[2], "wren\trobin dunnock");
}

#[test]
fn cells_fields_and_revisions_read_from_the_same_pieces() {
    let body = [
        // A field whose result spans two paragraphs, one of them hidden in
        // part, and a field nested in the code of another.
        r#"<w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText> TOC </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>one</w:t></w:r><w:r><w:rPr><w:vanish/></w:rPr><w:t>secret</w:t></w:r></w:p>"#,
        r#"<w:p><w:r><w:t>two</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>"#,
        r#"<w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText> IF </w:instrText></w:r><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText> = 2 </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>2</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r><w:r><w:instrText> = 2 "yes" </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>yes</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>"#,
        // A cell of two paragraphs, a deleted row and a cell that spans two
        // grid columns.
        r#"<w:tbl><w:tblGrid><w:gridCol/><w:gridCol/><w:gridCol/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:t>wren</w:t></w:r></w:p><w:p><w:r><w:t>robin</w:t></w:r></w:p></w:tc><w:tc><w:tcPr><w:gridSpan w:val="2"/></w:tcPr><w:p><w:r><w:t>2</w:t></w:r></w:p></w:tc></w:tr>"#,
        r#"<w:tr><w:trPr><w:del w:id="9" w:author="Pip Tanager"/></w:trPr><w:tc><w:p><w:del w:id="10" w:author="Pip Tanager"><w:r><w:delText>gone</w:delText></w:r></w:del></w:p></w:tc><w:tc><w:p/></w:tc><w:tc><w:p/></w:tc></w:tr></w:tbl>"#,
    ]
    .concat();
    let mut d = doc(&body);
    let v = d.views().unwrap();
    let fields: Vec<(&str, &str)> = v
        .fields()
        .iter()
        .filter(|f| f.location.story == StoryKind::Main)
        .map(|f| (f.code.as_str(), f.result.as_str()))
        .collect();
    assert_eq!(
        fields,
        [
            ("TOC", "one\ntwo"),
            ("IF { = 2 } = 2 \"yes\"", "yes"),
            ("= 2", "2")
        ]
    );
    let t = &v.tables()[0];
    assert_eq!(t.cells[0].text, "wren\nrobin");
    assert_eq!(t.row_count, 1);
    let text = v.text(&TextOptions::default());
    assert_eq!(text, "one\ntwo\nyes\nwren robin\t2\t\n");
    let current = v.text(&TextOptions {
        revisions: Revisions::Current,
        ..TextOptions::default()
    });
    assert_eq!(current, "one\ntwo\nyes\nwren robin\t2\t\ngone\t\t\n");
    let row = v
        .revisions()
        .iter()
        .find(|r| r.target == nebu_docx::RevisionTarget::Row)
        .unwrap();
    assert_eq!(row.text, "gone\t\t\n");
}

// ---- Hidden text and the style cascade ----------------------------------------

const CASCADE_STYLES: &str = r#"<w:style w:type="paragraph" w:styleId="HiddenNote"><w:name w:val="Hidden Note"/><w:basedOn w:val="Normal"/><w:rPr><w:vanish/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="BoldPara"><w:name w:val="Bold Para"/><w:basedOn w:val="Normal"/><w:rPr><w:b/></w:rPr></w:style><w:style w:type="character" w:styleId="Bird"><w:name w:val="Bird"/><w:rPr><w:color w:val="00FF00"/></w:rPr></w:style><w:style w:type="character" w:styleId="Secret"><w:name w:val="Secret"/><w:rPr><w:vanish/></w:rPr></w:style><w:style w:type="character" w:styleId="Strong2"><w:name w:val="Strong Two"/><w:rPr><w:b/></w:rPr></w:style>"#;

fn para(style: &str, rpr: &str, text: &str) -> String {
    format!(
        r#"<w:p><w:pPr><w:pStyle w:val="{style}"/></w:pPr><w:r><w:rPr>{rpr}</w:rPr><w:t>{text}</w:t></w:r></w:p>"#
    )
}

fn run_toggles(d: &mut Document) -> Vec<(String, nebu_docx::Toggles)> {
    d.views()
        .unwrap()
        .blocks()
        .iter()
        .flat_map(|b| b.content.iter())
        .filter_map(|i| match i {
            Inline::Text { text, toggles, .. } => Some((text.clone(), *toggles)),
            _ => None,
        })
        .collect()
}

#[test]
fn a_character_style_that_says_nothing_of_hidden_text_leaves_it_hidden() {
    let body = [
        para("HiddenNote", r#"<w:rStyle w:val="Bird"/>"#, "styled"),
        para("HiddenNote", "", "plain"),
        para("HiddenNote", r#"<w:vanish w:val="0"/>"#, "shown"),
        para("Normal", r#"<w:rStyle w:val="Secret"/>"#, "secret"),
        para("Normal", "", "seen"),
    ]
    .concat();
    let mut d = package(&body, CASCADE_STYLES, "", "");
    let text = d.views().unwrap().text(&TextOptions::default());
    // The paragraph style hides the marks of its paragraphs too, so the
    // shown text runs on into the paragraph after it, as in Word.
    assert_eq!(text, "shown\nseen\n");
    let hidden: Vec<(String, bool)> = run_toggles(&mut d)
        .into_iter()
        .map(|(t, g)| (t, g.has(nebu_docx::Toggle::Hidden)))
        .collect();
    assert_eq!(
        hidden,
        [
            ("styled".to_string(), true),
            ("plain".into(), true),
            ("shown".into(), false),
            ("secret".into(), true),
            ("seen".into(), false),
        ]
    );
}

#[test]
fn toggle_properties_turn_over_between_styles_and_direct_formatting_sets_them() {
    let body = [
        para("BoldPara", "", "para"),
        para("BoldPara", r#"<w:rStyle w:val="Strong2"/>"#, "both"),
        para("BoldPara", r#"<w:rStyle w:val="Strong2"/><w:b/>"#, "direct"),
        para(
            "Normal",
            r#"<w:rStyle w:val="Strong2"/><w:b w:val="0"/>"#,
            "off",
        ),
        para("HiddenNote", r#"<w:rStyle w:val="Secret"/>"#, "twice"),
    ]
    .concat();
    let mut d = package(&body, CASCADE_STYLES, "", "");
    let on: Vec<(String, Vec<nebu_docx::Toggle>)> = run_toggles(&mut d)
        .into_iter()
        .map(|(t, g)| (t, g.iter().collect()))
        .collect();
    use nebu_docx::Toggle::{Bold, Hidden};
    assert_eq!(
        on,
        [
            ("para".to_string(), vec![Bold]),
            ("both".into(), vec![]),
            ("direct".into(), vec![Bold]),
            ("off".into(), vec![]),
            ("twice".into(), vec![]),
        ]
    );
    assert!(!on.iter().any(|(_, t)| t.contains(&Hidden)));
}

// ---- Locators -----------------------------------------------------------------

#[test]
fn a_locator_that_matches_several_blocks_names_them() {
    let body = [
        r#"<w:p w14:paraId="0000ABCD"><w:bookmarkStart w:id="1" w:name="site"/><w:r><w:t>one</w:t></w:r><w:bookmarkEnd w:id="1"/></w:p>"#,
        r#"<w:p w14:paraId="0000ABCE"><w:bookmarkStart w:id="2" w:name="Site"/><w:r><w:t>two</w:t></w:r><w:bookmarkEnd w:id="2"/></w:p>"#,
        r#"<w:p w14:paraId="1C0F4262"><w:r><w:t>three</w:t></w:r></w:p>"#,
    ]
    .concat();
    let mut d = doc(&body);
    let v = d.views().unwrap();
    let err = v
        .locate(&"bookmark:site".parse().unwrap())
        .unwrap_err()
        .to_string();
    assert_eq!(
        err,
        "2 blocks match the bookmark \"site\": p:0000ABCD, p:0000ABCE"
    );
    // Paragraph ids compare as Word compares them.
    let b = v.locate(&"p:1c0f4262".parse().unwrap()).unwrap();
    assert_eq!(b.text, "three");
    assert_eq!(b.id, nebu_docx::BlockId::Paragraph(0x1C0F4262));
}

#[test]
fn shared_and_invalid_paragraph_ids_give_numbered_ids() {
    let body = [
        r#"<w:p w14:paraId="0000abcd"><w:r><w:t>one</w:t></w:r></w:p>"#,
        r#"<w:p w14:paraId="ABCD"><w:r><w:t>two</w:t></w:r></w:p>"#,
        r#"<w:p w14:paraId="80000000"><w:r><w:t>three</w:t></w:r></w:p>"#,
        r#"<w:p w14:paraId="00000001"><w:r><w:t>four</w:t></w:r></w:p>"#,
    ]
    .concat();
    let mut d = doc(&body);
    let v = d.views().unwrap();
    let ids: Vec<(String, bool)> = v
        .blocks()
        .iter()
        .map(|b| (b.id.to_string(), b.shared_para_id))
        .collect();
    assert_eq!(
        ids,
        [
            ("#0".to_string(), true),
            ("#1".into(), true),
            ("#2".into(), false),
            ("p:00000001".into(), false),
        ]
    );
}

#[test]
fn the_views_are_borrowed_from_one_reading() {
    let mut d = doc(&p("x"));
    let first = d.views().unwrap().blocks().as_ptr();
    let again = d.views().unwrap().blocks().as_ptr();
    assert!(std::ptr::eq(first, again));
    let styles = d.views().unwrap().styles();
    let h = styles.find("Heading 1").unwrap();
    assert_eq!(
        styles
            .find_of(nebu_docx::StyleKind::Paragraph, "heading 1")
            .unwrap()
            .id,
        h.id
    );
    assert!(
        styles
            .find_of(nebu_docx::StyleKind::Table, "heading 1")
            .is_none()
    );
}
