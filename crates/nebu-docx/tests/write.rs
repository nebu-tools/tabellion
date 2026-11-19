//! The writer: content into documents, checked against the schemas, read
//! back through the views, and compared byte for byte outside the
//! insertion.

mod support;

use std::path::Path;

use nebu_docx::content::{Block, Cell, Image, Inline, Row, Table};
use nebu_docx::{
    BlockKind, Content, Document, Error, Locator, Place, Role, RoleSource, StoryKind, TableStyle,
    WriteOptions,
};
use serde_json::json;

fn corpus(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../corpus")
            .join(name),
    )
    .unwrap()
}

fn valid(bytes: &[u8]) {
    let e = support::xsd::validate_docx(bytes);
    assert!(e.is_empty(), "\n{}", e.join("\n"));
}

fn content(v: serde_json::Value) -> Content {
    Content::from_json(v.to_string().as_bytes()).unwrap()
}

fn png(w: u32, h: u32) -> Vec<u8> {
    support::png(w, h)
}

/// Inserts and returns the saved bytes.
fn insert(bytes: Vec<u8>, c: &Content, place: impl Fn(&mut Document) -> Place) -> Vec<u8> {
    let mut doc = Document::from_bytes(bytes).unwrap();
    let p = place(&mut doc);
    doc.insert(p, &c.blocks, &WriteOptions::for_content(c, None))
        .unwrap();
    doc.to_bytes().unwrap()
}

fn at(doc: &mut Document, locator: &str) -> nebu_docx::xml::NodeId {
    let l: Locator = locator.parse().unwrap();
    doc.views().unwrap().locate(&l).unwrap().node
}

fn survey() -> Content {
    let pic = support::base64(&png(120, 80));
    content(json!({
        "blocks": [
            {"type": "heading", "level": 2, "content": ["Counts"], "label": "sec-counts"},
            {"type": "paragraph", "content": [
                "The counts are in ", {"type": "reference", "label": "tab-sites"},
                " and the wren in ", {"type": "reference", "label": "fig-wren"}, ".",
                {"type": "footnote", "content": [{"type": "paragraph", "content": ["Counted from song at dawn."]}]}
            ]},
            {"type": "table", "header_rows": 1, "caption": ["Sites and pairs"], "label": "tab-sites",
             "style": "Field Green", "rows": [
                {"cells": [{"content": [{"type": "paragraph", "content": ["Site"]}]},
                           {"content": [{"type": "paragraph", "content": ["Pairs"]}]}]},
                {"cells": [{"content": [{"type": "paragraph", "content": ["North"]}], "row_span": 2},
                           {"content": [{"type": "paragraph", "content": ["12"]}], "align": "right"}]},
                {"cells": [{"content": [{"type": "paragraph", "content": ["9"]}], "align": "right", "shading": "E2EFDA"}]}
            ]},
            {"type": "list", "ordered": true, "items": [
                {"content": ["Walk the transect"], "list": {"ordered": false, "items": [
                    {"content": ["slowly"]}, {"content": ["quietly"]}]}},
                {"content": ["Count the pairs"]}
            ]},
            {"type": "figure", "image": {"data": pic, "alt": "A wren"}, "caption": ["A wren at the north site"], "label": "fig-wren"},
            {"type": "paragraph", "content": [
                {"type": "link", "target": {"url": "https://example.org/birds"}, "content": ["the survey site"]},
                " and ", {"type": "link", "target": {"label": "sec-counts"}, "content": ["the counts"]}
            ]}
        ],
        "table_styles": [{"name": "Field Green", "header": {"fill": "385723", "color": "FFFFFF", "bold": true},
            "band_fill": "E2EFDA", "borders": {"top": {"width": 1, "color": "385723"}, "bottom": {"width": 1, "color": "385723"}}}]
    }))
}

#[test]
fn a_survey_section_reads_back_through_the_views() {
    for source in [
        support::sample(),
        corpus("birds.docx"),
        corpus("cascade.docx"),
    ] {
        let out = insert(source, &survey(), |_| Place::End);
        valid(&out);
        let mut doc = Document::from_bytes(out).unwrap();
        let v = doc.views().unwrap();
        let text = v.text(&Default::default());
        assert!(text.contains("Counts\n"), "{text}");
        // The table caption is numbered after the tables before it.
        let tables_before = text.matches("Table ").count();
        assert!(tables_before >= 1);
        let caption = v
            .paragraphs()
            .iter()
            .find(|p| p.text.ends_with("Sites and pairs"))
            .unwrap();
        assert!(caption.text.starts_with("Table "), "{}", caption.text);
        // The reference shows the caption's label and number.
        let number = caption
            .text
            .trim_end_matches(": Sites and pairs")
            .to_string();
        assert!(text.contains(&format!(
            "The counts are in {number} and the wren in Figure"
        )));
        // The table: three rows, a cell over two rows, a style.
        let t = v.tables().last().unwrap();
        assert_eq!((t.row_count, t.column_count), (3, 2));
        assert!(t.cells.iter().any(|c| c.row_span == 2 && c.text == "North"));
        let style = v.styles().get(t.style.as_deref().unwrap()).unwrap();
        assert_eq!(style.name, "Field Green");
        // The footnote, the picture, the bookmarks of the labels.
        assert!(
            v.notes()
                .iter()
                .any(|n| n.text == "Counted from song at dawn.")
        );
        let img = v.images().last().unwrap();
        assert_eq!(img.alt.as_deref(), Some("A wren"));
        assert_eq!((img.width, img.height), (Some(90.0), Some(60.0)));
        for l in ["_Ref_tab-sites", "_Ref_fig-wren", "_Ref_sec-counts"] {
            assert!(v.bookmarks().iter().any(|b| b.name == l && b.hidden), "{l}");
        }
        // The list items take numbering.
        let items: Vec<_> = v
            .blocks()
            .iter()
            .filter(|b| b.numbering.is_some() && b.text.contains("transect") || b.text == "slowly")
            .collect();
        assert_eq!(items.len(), 2);
        assert_eq!(items[1].numbering.as_ref().unwrap().level, 1);
    }
}

#[test]
fn errors_name_the_json_path() {
    let bad = [
        (
            json!({"blocks": [{"type": "heading", "level": 12, "content": []}]}),
            "blocks[0].level",
        ),
        (
            json!({"blocks": [{"type": "paragraph", "content": [{"type": "texxt"}]}]}),
            "blocks[0].content[0].type",
        ),
        (
            json!({"blocks": [{"type": "paragraph", "colour": 1}]}),
            "unknown key \"colour\"",
        ),
        (
            json!({"blocks": [{"type": "table", "rows": [
            {"cells": [{"content": []}, {"content": []}]},
            {"cells": [{"content": []}]}]}]}),
            "blocks[0]: row 1, column 1",
        ),
        (
            json!({"blocks": [{"type": "table", "rows": [
            {"cells": [{"content": []}, {"content": [], "row_span": 2}]},
            {"cells": [{"content": [], "column_span": 2}]}]}]}),
            "row 1, column 1: a cell from row 1 overlaps a cell from row 0",
        ),
        (
            json!({"blocks": [{"type": "paragraph", "content": [{"type": "footnote", "content": [
            {"type": "paragraph", "content": [{"type": "footnote", "content": [{"type": "paragraph"}]}]}]}]}]}),
            "blocks[0].content[0].content[0].content[0]: a footnote cannot stand in a footnote",
        ),
        (
            json!({"blocks": [{"type": "equation", "math": {"omml": "<m:oMath><w:p/></m:oMath>"}}]}),
            "blocks[0].math.omml",
        ),
        (
            json!({"blocks": [{"type": "raw", "xml": "<w:r/>"}]}),
            "blocks[0].xml: w:r cannot stand where a block goes",
        ),
        (
            json!({"blocks": [{"type": "raw", "xml": "<w:p>"}]}),
            "not well-formed",
        ),
        (
            json!({"blocks": [{"type": "paragraph", "content": [{"type": "text", "text": "x", "marks": {"color": "green"}}]}]}),
            "blocks[0].content[0].marks.color",
        ),
        (
            json!({"blocks": [{"type": "heading", "level": 1, "label": "two words"}]}),
            "blocks[0].label",
        ),
        (
            json!({"blocks": [{"type": "image", "image": {"data": "***"}}]}),
            "blocks[0].image.data: not base64",
        ),
    ];
    for (j, want) in bad {
        let e = Content::from_json(j.to_string().as_bytes()).unwrap_err();
        assert!(matches!(e, Error::Invalid(_)), "{e}");
        assert!(e.to_string().contains(want), "{want:?} not in {e}");
    }
    // Limits.
    let deep = (0..40).fold(json!([{"type": "paragraph"}]), |inner, _| {
        json!([{"type": "list", "items": [{"content": [], "list": {"items": [{"content": []}]}}]}, {"type": "table", "rows": [{"cells": [{"content": inner}]}]}])
    });
    let e = Content::from_json(json!({"blocks": deep}).to_string().as_bytes()).unwrap_err();
    assert!(
        e.to_string().contains("max_depth") || e.to_string().contains("128"),
        "{e}"
    );
    let mut small = nebu_docx::ContentLimits {
        max_items: 3,
        ..Default::default()
    };
    let e = Content::from_json_with_limits(
        json!(
            ["a", "b", "c", "d"]
                .iter()
                .map(|t| json!({"type": "paragraph", "content": [t]}))
                .collect::<Vec<_>>()
        )
        .to_string()
        .as_bytes(),
        &small,
    )
    .unwrap_err();
    assert!(e.to_string().contains("max_items"), "{e}");
    small = nebu_docx::ContentLimits {
        max_bytes: 10,
        ..Default::default()
    };
    let e = Content::from_json_with_limits(b"[{\"type\": \"page_break\"}]", &small).unwrap_err();
    assert!(e.to_string().contains("max_bytes"), "{e}");
}

#[test]
fn the_json_form_round_trips() {
    let c = survey();
    let again = Content::from_json(c.to_json().as_bytes()).unwrap();
    assert_eq!(c, again);
}

/// The part's bytes are the old ones with one stretch inserted, and one
/// removed for a replacement.
fn spliced(before: &[u8], after: &[u8]) -> usize {
    let prefix = before.iter().zip(after).take_while(|(a, b)| a == b).count();
    let suffix = before[prefix..]
        .iter()
        .rev()
        .zip(after[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    before.len() - prefix - suffix
}

#[test]
fn everything_outside_the_insertion_keeps_its_bytes() {
    let c = content(json!([
        {"type": "paragraph", "content": ["A plain paragraph."]},
        {"type": "heading", "level": 1, "content": ["Plain heading"]}
    ]));
    for source in [
        support::sample(),
        corpus("birds.docx"),
        corpus("parts.docx"),
    ] {
        for which in ["end", "before", "after", "replace"] {
            let mut doc = Document::from_bytes(source.clone()).unwrap();
            let main = doc.main_part().to_string();
            let old = doc.read_part(&main).unwrap();
            let node = {
                let v = doc.views().unwrap();
                let b = v
                    .blocks()
                    .iter()
                    .filter(|b| b.kind == BlockKind::Paragraph)
                    .nth(2)
                    .unwrap();
                b.node
            };
            let place = match which {
                "end" => Place::End,
                "before" => Place::Before(node),
                "after" => Place::After(node),
                _ => Place::Replace(node),
            };
            doc.insert(place, &c.blocks, &WriteOptions::default())
                .unwrap();
            let out = doc.to_bytes().unwrap();
            valid(&out);
            let new = doc.read_part(&main).unwrap();
            let removed = spliced(&old, &new);
            match which {
                "replace" => assert!(removed > 0 && removed < 2000, "{which}: {removed}"),
                _ => {
                    if removed != 0 {
                        let q = old
                            .iter()
                            .rev()
                            .zip(new.iter().rev())
                            .take_while(|(a, b)| a == b)
                            .count();
                        let p = old.len() - q;
                        panic!(
                            "{which}: bytes of the old part changed at {p}:\nold {}\nnew {}",
                            String::from_utf8_lossy(
                                &old[p.saturating_sub(80)..(p + 300).min(old.len())]
                            ),
                            String::from_utf8_lossy(
                                &new[p.saturating_sub(80)..(p + 300).min(new.len())]
                            )
                        );
                    }
                }
            }
            // Every other part is byte-identical, the styles part too: both
            // roles have their style in these documents.
            let before = support::entries(&source);
            let after = support::entries(&out);
            for (name, bytes) in &before {
                if *name == main {
                    continue;
                }
                let now = after.iter().find(|(n, _)| n == name).map(|(_, b)| b);
                assert_eq!(now, Some(bytes), "{which}: {name} changed");
            }
            // The zip records of untouched parts are those of the source.
            support::assert_records_kept(&source, &out, &[main.as_str()]);
        }
    }
}

#[test]
fn the_same_insertion_gives_the_same_bytes_and_deleting_it_restores_the_part() {
    let c = content(json!([
        {"type": "paragraph", "content": ["One."]},
        {"type": "table", "rows": [{"cells": [{"content": [{"type": "paragraph", "content": ["a"]}]}]}]},
        {"type": "paragraph", "content": ["Two."]}
    ]));
    for source in [support::sample(), corpus("birds.docx")] {
        let a = insert(source.clone(), &c, |_| Place::End);
        let b = insert(source.clone(), &c, |_| Place::End);
        assert_eq!(a, b, "the same insertion gave other bytes");
        // Deleting what was inserted gives back the main part's bytes.
        let mut doc = Document::from_bytes(source.clone()).unwrap();
        let main = doc.main_part().to_string();
        let old = doc.read_part(&main).unwrap();
        let r = doc
            .insert(Place::End, &c.blocks, &WriteOptions::default())
            .unwrap();
        let t = doc.xml(&main).unwrap();
        for n in r.nodes.iter().flatten() {
            t.detach(*n).unwrap();
        }
        assert_eq!(doc.read_part(&main).unwrap(), old);
    }
}

#[test]
fn a_document_without_numbering_or_footnotes_gets_them() {
    let c = content(json!([
        {"type": "list", "ordered": true, "items": [{"content": ["one"]}, {"content": ["two"]}]},
        {"type": "list", "ordered": true, "items": [{"content": ["again"]}]},
        {"type": "paragraph", "content": ["A note.", {"type": "footnote", "content": [{"type": "paragraph", "content": ["Seen."]}]}]}
    ]));
    let mut doc = Document::from_bytes(corpus("cascade.docx")).unwrap();
    let r = doc
        .insert(Place::End, &c.blocks, &WriteOptions::default())
        .unwrap();
    assert!(
        r.roles
            .iter()
            .any(|(role, s)| *role == Role::ListNumber && matches!(s, RoleSource::Fallback { .. }))
    );
    let out = doc.to_bytes().unwrap();
    valid(&out);
    let mut doc = Document::from_bytes(out.clone()).unwrap();
    let v = doc.views().unwrap();
    // Two lists, two instances, each starting at 1.
    let nums: Vec<String> = v
        .blocks()
        .iter()
        .filter_map(|b| b.numbering.as_ref().map(|n| n.id.clone()))
        .collect();
    assert_eq!(nums.len(), 3);
    assert_eq!(nums[0], nums[1]);
    assert_ne!(nums[1], nums[2]);
    assert!(
        v.numbering()
            .instances
            .iter()
            .all(|i| i.definition.is_some())
    );
    assert_eq!(v.notes().len(), 1);
    assert_eq!(v.notes()[0].number, Some(1));
    let names: Vec<&str> = support::entries(&out).iter().map(|_| "").collect();
    let _ = names;
    let entries = support::entries(&out);
    assert!(entries.iter().any(|(n, _)| n == "word/numbering.xml"));
    let foot = entries
        .iter()
        .find(|(n, _)| n == "word/footnotes.xml")
        .unwrap();
    let foot = String::from_utf8_lossy(&foot.1);
    assert!(
        foot.contains("w:type=\"separator\"") && foot.contains("w:type=\"continuationSeparator\"")
    );
    let settings = entries
        .iter()
        .find(|(n, _)| n == "word/settings.xml")
        .unwrap();
    assert!(String::from_utf8_lossy(&settings.1).contains("<w:footnotePr>"));
}

#[test]
fn places_inside_cells_controls_headers_notes_and_text_boxes() {
    let para = content(json!([{"type": "paragraph", "content": ["Inserted here."]}]));
    let table = content(
        json!([{"type": "table", "rows": [{"cells": [{"content": [{"type": "paragraph", "content": ["In a note's table."]}]}]}]}]),
    );
    let note = content(
        json!([{"type": "paragraph", "content": ["x", {"type": "footnote", "content": [{"type": "paragraph", "content": ["y"]}]}]}]),
    );
    let page = content(json!([{"type": "page_break"}]));
    let caption = content(
        json!([{"type": "figure", "image": {"data": support::base64(&png(10, 10))}, "caption": ["c"]}]),
    );
    let mut doc = Document::from_bytes(corpus("birds.docx")).unwrap();
    let (cell, header, footnote, text_box, control) = {
        let v = doc.views().unwrap();
        let find = |f: &dyn Fn(&nebu_docx::Paragraph) -> bool| {
            v.paragraphs()
                .iter()
                .find(|p| f(p))
                .map(|p| p.node)
                .unwrap()
        };
        (
            find(&|p| p.cell.is_some() && p.text == "North"),
            find(&|p| p.location.story == StoryKind::Header),
            find(&|p| p.location.story == StoryKind::Footnote),
            find(&|p| p.location.story == StoryKind::TextBox),
            None::<nebu_docx::xml::NodeId>,
        )
    };
    let _ = control;
    let opts = WriteOptions::default();
    for n in [cell, header, footnote, text_box] {
        doc.insert(Place::After(n), &para.blocks, &opts).unwrap();
    }
    doc.insert(Place::After(footnote), &table.blocks, &opts)
        .unwrap();
    for (n, c, why) in [
        (footnote, &note, "a footnote cannot stand in a footnote"),
        (header, &note, "a footnote cannot stand in a header"),
        (text_box, &note, "a footnote cannot stand in a text box"),
        (cell, &page, "a page break cannot stand in a table cell"),
        (
            header,
            &caption,
            "a caption is numbered in the body; it cannot stand in a header",
        ),
    ] {
        let e = doc.insert(Place::After(n), &c.blocks, &opts).unwrap_err();
        assert!(e.to_string().contains(why), "{why:?} not in {e}");
    }
    let out = doc.to_bytes().unwrap();
    valid(&out);
    let mut doc = Document::from_bytes(out).unwrap();
    let v = doc.views().unwrap();
    let stories: Vec<(StoryKind, bool)> = v
        .paragraphs()
        .iter()
        .filter(|p| p.text == "Inserted here.")
        .map(|p| (p.location.story, p.cell.is_some()))
        .collect();
    for want in [
        (StoryKind::Main, true),
        (StoryKind::Header, false),
        (StoryKind::Footnote, false),
        (StoryKind::TextBox, false),
    ] {
        assert!(stories.contains(&want), "{want:?} not in {stories:?}");
    }
    assert!(
        v.tables()
            .iter()
            .any(|t| t.location.story == StoryKind::Footnote)
    );
}

#[test]
fn a_content_control_takes_blocks_inside() {
    let para = content(json!([{"type": "paragraph", "content": ["In the group."]}]));
    let mut doc = Document::from_bytes(corpus("defaults.docx")).unwrap();
    let inside = {
        let v = doc.views().unwrap();
        v.paragraphs()
            .iter()
            .find(|p| p.text.starts_with("Delivery to"))
            .unwrap()
            .node
    };
    doc.insert(Place::After(inside), &para.blocks, &WriteOptions::default())
        .unwrap();
    let out = doc.to_bytes().unwrap();
    valid(&out);
    let mut doc = Document::from_bytes(out).unwrap();
    let v = doc.views().unwrap();
    let group = v
        .content_controls()
        .iter()
        .find(|c| c.control.tag.as_deref() == Some("delivery:group"))
        .unwrap();
    assert!(group.text.contains("In the group."), "{}", group.text);
}

#[test]
fn captions_renumber_the_later_ones_and_their_references() {
    // birds.docx has Table 1 and Figure 1; a table before them makes the
    // old one Table 2, and the REF to it says so.
    let c = content(
        json!([{"type": "table", "caption": ["Weather"], "label": "tab-weather",
        "rows": [{"cells": [{"content": [{"type": "paragraph", "content": ["dry"]}]}]}]}]),
    );
    let mut doc = Document::from_bytes(corpus("birds.docx")).unwrap();
    let first = at(&mut doc, "heading:Survey sites");
    let r = doc
        .insert(Place::Before(first), &c.blocks, &WriteOptions::default())
        .unwrap();
    assert!(r.renumbered >= 2, "{r:?}");
    let out = doc.to_bytes().unwrap();
    valid(&out);
    let mut doc = Document::from_bytes(out).unwrap();
    let v = doc.views().unwrap();
    let seq: Vec<String> = v
        .fields()
        .iter()
        .filter(|f| f.kind == "SEQ" && f.code.contains("Table") && !f.code.contains("\\c"))
        .map(|f| f.result.clone())
        .collect();
    assert_eq!(&seq[..2], ["1", "2"]);
    let text = v.text(&Default::default());
    assert!(text.contains("Table 1: Weather"), "{text}");
    assert!(text.contains("Table 2: Sites, visits and counts"), "{text}");
    assert!(text.contains("As Table 2 shows"), "{text}");
}

#[test]
fn unknown_labels_and_styles_are_errors_that_list_what_exists() {
    let mut doc = Document::from_bytes(corpus("birds.docx")).unwrap();
    let c = content(
        json!([{"type": "paragraph", "content": [{"type": "reference", "label": "tab-nothing"}]}]),
    );
    let e = doc
        .insert(Place::End, &c.blocks, &WriteOptions::default())
        .unwrap_err();
    assert!(e.to_string().contains("no label \"tab-nothing\""), "{e}");
    let c = content(
        json!([{"type": "table", "style": "Nonesuch", "rows": [{"cells": [{"content": []}]}]}]),
    );
    let e = doc
        .insert(Place::End, &c.blocks, &WriteOptions::default())
        .unwrap_err();
    let m = e.to_string();
    assert!(
        m.contains("no table style \"Nonesuch\"") && m.contains("\"Table Normal\""),
        "{m}"
    );
    // Nothing was written by the failures.
    assert_eq!(doc.to_bytes().unwrap(), corpus("birds.docx"));
    let e = doc
        .ensure_table_style(&TableStyle::Named("Nonesuch".into()))
        .unwrap_err();
    assert!(e.to_string().contains("Table Normal"), "{e}");
}

#[test]
fn table_styles_resolve_in_order_and_are_never_overwritten() {
    let def = nebu_docx::content::TableStyleDef {
        name: "Report Table".into(),
        band_fill: Some(nebu_docx::content::Color::new("FF0000").unwrap()),
        ..Default::default()
    };
    // The sample template has a Report Table: the definition does not
    // replace it.
    let mut doc = Document::from_bytes(support::sample()).unwrap();
    let id = doc
        .ensure_table_style(&TableStyle::Defined(Box::new(def.clone())))
        .unwrap();
    assert_eq!(doc.to_bytes().unwrap(), support::sample());
    let v = doc.views().unwrap();
    assert_eq!(v.styles().get(&id).unwrap().name, "Report Table");
    // A new one is a table style Word lists, with its sections.
    let fresh = nebu_docx::content::TableStyleDef {
        name: "Field Green".into(),
        ..def
    };
    let id = doc
        .ensure_table_style(&TableStyle::Defined(Box::new(fresh)))
        .unwrap();
    let out = doc.to_bytes().unwrap();
    valid(&out);
    let styles = String::from_utf8(support::entry(&out, "word/styles.xml")).unwrap();
    assert!(styles.contains(&format!("w:styleId=\"{id}\"")));
    assert!(styles.contains("w:type=\"band1Horz\""));
    // A table without a style in a document without Table Grid gets it.
    let mut doc = Document::from_bytes(corpus("cascade.docx")).unwrap();
    let c = content(json!([{"type": "table", "rows": [{"cells": [{"content": []}]}]}]));
    let r = doc
        .insert(Place::End, &c.blocks, &WriteOptions::default())
        .unwrap();
    assert!(r.styles_added.iter().any(|s| s == "Table Grid"), "{r:?}");
    valid(&doc.to_bytes().unwrap());
}

#[test]
fn pictures_are_stored_once_and_sized_to_the_text_width() {
    let big = support::base64(&png(4000, 1000));
    let svg =
        support::base64(br#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100"/>"#);
    let fallback = support::base64(&png(200, 100));
    let c = content(json!([
        {"type": "image", "image": {"data": big}},
        {"type": "image", "image": {"data": big, "width": 72}},
        {"type": "image", "image": {"data": svg, "fallback": {"data": fallback}, "alt": "A drawn wren"}}
    ]));
    let out = insert(support::sample(), &c, |_| Place::End);
    valid(&out);
    let media: Vec<_> = support::entries(&out)
        .into_iter()
        .filter(|(n, _)| n.starts_with("word/media/"))
        .collect();
    assert_eq!(
        media.len(),
        3,
        "{:?}",
        media.iter().map(|m| &m.0).collect::<Vec<_>>()
    );
    let mut doc = Document::from_bytes(out).unwrap();
    let v = doc.views().unwrap();
    let width = v.sections()[0].text_width.unwrap();
    let imgs = v.images();
    assert_eq!(imgs.len(), 3);
    assert!((imgs[0].width.unwrap() - width).abs() < 0.1);
    assert_eq!(imgs[1].width, Some(72.0));
    assert_eq!(imgs[1].height, Some(18.0));
    // An SVG without a fallback is an error.
    let c = content(json!([{"type": "image", "image": {"data": svg}}]));
    let mut doc = Document::from_bytes(support::sample()).unwrap();
    let e = doc
        .insert(Place::End, &c.blocks, &WriteOptions::default())
        .unwrap_err();
    assert!(e.to_string().contains("needs a PNG fallback"), "{e}");
    // A file is read only from the directory the options name.
    let c = content(json!([{"type": "image", "image": {"file": "bird.png"}}]));
    let e = doc
        .insert(Place::End, &c.blocks, &WriteOptions::default())
        .unwrap_err();
    assert!(e.to_string().contains("reads no files"), "{e}");
}

#[test]
fn raw_ooxml_math_fields_and_breaks_are_written() {
    let c = Content {
        blocks: vec![
            Block::Raw {
                xml: "<w:p><w:r><w:t>Raw paragraph.</w:t></w:r></w:p>".into(),
            },
            Block::Equation {
                math: nebu_docx::content::Math::omml(
                    "<m:oMath><m:r><m:t>a=b</m:t></m:r></m:oMath>",
                ),
                label: Some("eq-pairs".into()),
            },
            Block::Paragraph {
                style: None,
                align: None,
                content: vec![
                    Inline::text("See "),
                    Inline::Reference {
                        label: "eq-pairs".into(),
                    },
                    Inline::LineBreak,
                    Inline::Field {
                        code: "PAGE".into(),
                        result: "1".into(),
                    },
                    Inline::Math {
                        math: nebu_docx::content::Math::omml(
                            "<m:oMath><m:r><m:t>x</m:t></m:r></m:oMath>",
                        ),
                    },
                    Inline::Raw {
                        xml: "<w:r><w:t>raw run</w:t></w:r>".into(),
                    },
                    Inline::Image {
                        image: Image::new(png(20, 20)),
                    },
                ],
            },
            Block::PageBreak,
        ],
        table_styles: vec![],
    };
    let out = insert(support::sample(), &c, |_| Place::End);
    valid(&out);
    let mut doc = Document::from_bytes(out).unwrap();
    let text = doc.views().unwrap().text(&Default::default());
    assert!(text.contains("Raw paragraph."), "{text}");
    assert!(text.contains("a=b\t(1)"), "{text}");
    assert!(text.contains("See Equation (1)\n1xraw run"), "{text}");
}

#[test]
fn a_named_paragraph_style_must_exist() {
    let mut doc = Document::from_bytes(support::sample()).unwrap();
    let c = content(json!([{"type": "paragraph", "style": "Body Text", "content": ["ok"]}]));
    doc.insert(Place::End, &c.blocks, &WriteOptions::default())
        .unwrap();
    let c = content(json!([{"type": "paragraph", "style": "Nonesuch", "content": ["no"]}]));
    let e = doc
        .insert(Place::End, &c.blocks, &WriteOptions::default())
        .unwrap_err();
    assert!(
        e.to_string()
            .contains("blocks[0].style: the document has no paragraph style \"Nonesuch\""),
        "{e}"
    );
}

#[test]
fn roles_can_be_chosen_and_say_where_they_came_from() {
    let mut doc = Document::from_bytes(support::sample()).unwrap();
    let mut opts = WriteOptions::default();
    opts.roles.set(Role::Body, "Normal");
    let c = content(
        json!([{"type": "paragraph", "content": ["plain"]}, {"type": "heading", "level": 4, "content": ["deep"]}]),
    );
    let r = doc.insert(Place::End, &c.blocks, &opts).unwrap();
    assert!(
        r.roles
            .contains(&(Role::Body, RoleSource::Chosen("Normal".into())))
    );
    assert!(
        r.roles
            .contains(&(Role::Heading(4), RoleSource::Added("heading 4".into())))
    );
    assert_eq!(r.styles_added, vec!["heading 4".to_string()]);
    valid(&doc.to_bytes().unwrap());
    opts.roles.set(Role::Caption, "Nonesuch");
    let c = content(
        json!([{"type": "figure", "image": {"data": support::base64(&png(5, 5))}, "caption": ["x"]}]),
    );
    let e = doc.insert(Place::End, &c.blocks, &opts).unwrap_err();
    assert!(e.to_string().contains("chosen for captions"), "{e}");
}

#[test]
fn overlapping_places_in_one_batch_are_an_error() {
    let mut doc = Document::from_bytes(corpus("birds.docx")).unwrap();
    let before = doc.to_bytes().unwrap();
    let (table, inner) = {
        let v = doc.views().unwrap();
        let t = v
            .blocks()
            .iter()
            .find(|b| b.kind == BlockKind::Table)
            .unwrap();
        let p = v
            .paragraphs()
            .iter()
            .find(|p| p.location.block == Some(t.id))
            .unwrap();
        (t.node, p.node)
    };
    let one = content(json!([{"type": "paragraph", "content": ["beside."]}]));
    let two = content(json!([{"type": "paragraph", "content": ["instead."]}]));
    let e = doc
        .insert_all(
            &[
                (Place::Replace(table), one.blocks.as_slice()),
                (Place::Before(inner), two.blocks.as_slice()),
            ],
            &WriteOptions::default(),
        )
        .unwrap_err();
    assert!(e.to_string().contains("overlap"), "{e}");
    // The failure left the document as it was.
    assert_eq!(doc.to_bytes().unwrap(), before);
}

#[test]
fn a_reference_to_a_stale_caption_matches_what_the_caption_says() {
    // A caption whose cached number is stale keeps its bytes; a reference
    // written after it shows the kept number, not a recalculated one.
    let c = content(
        json!([{"type": "table", "caption": ["Dry"], "label": "tab-dry",
        "rows": [{"cells": [{"content": [{"type": "paragraph", "content": ["x"]}]}]}]}]),
    );
    let mut doc = Document::from_bytes(support::sample()).unwrap();
    doc.insert(Place::End, &c.blocks, &WriteOptions::default())
        .unwrap();
    // Age the cached result: Table 1 becomes Table 7 in the bytes.
    let bytes = doc.to_bytes().unwrap();
    let mut files = support::entries(&bytes);
    let i = files
        .iter()
        .position(|(n, _)| n == "word/document.xml")
        .unwrap();
    let mut xml = String::from_utf8(files[i].1.clone()).unwrap();
    let mark = xml.find("_Ref_tab-dry").expect("the label bookmark");
    let at = mark + xml[mark..].find(">1</w:t>").expect("the cached number");
    assert!(xml[mark..at].contains("SEQ"), "no SEQ field before {at}");
    xml.replace_range(at + 1..at + 2, "7");
    files[i].1 = xml.into_bytes();
    let refs: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(n, d)| (n.as_str(), d.as_slice()))
        .collect();
    let mut doc = Document::from_bytes(support::zip(&refs, false)).unwrap();
    assert!(
        doc.views()
            .unwrap()
            .paragraphs()
            .iter()
            .any(|p| p.text == "Table 7: Dry"),
        "the caption is not stale"
    );
    let c = content(
        json!([{"type": "paragraph", "content": ["See ", {"type": "reference", "label": "tab-dry"}, "."]}]),
    );
    doc.insert(Place::End, &c.blocks, &WriteOptions::default())
        .unwrap();
    let out = doc.to_bytes().unwrap();
    valid(&out);
    let mut doc = Document::from_bytes(out).unwrap();
    let v = doc.views().unwrap();
    let reference = v
        .fields()
        .iter()
        .find(|f| f.kind == "REF" && f.code.contains("tab-dry"))
        .unwrap();
    assert_eq!(reference.result, "Table 7", "{reference:?}");
}

#[test]
fn after_a_section_break_the_next_section_sizes_what_is_written() {
    // The first paragraph ends a wide section; what is written after it
    // stands in the sample's own narrower one.
    let mut xml =
        String::from_utf8(support::entry(&support::sample(), "word/document.xml")).unwrap();
    let start = xml.rfind("<w:sectPr").expect("the final section");
    let end = xml[start..].find("</w:sectPr>").expect("its end") + "</w:sectPr>".len();
    let section = xml[start..start + end].to_string();
    let pg = section.find("<w:pgSz").expect("a page size");
    let wstart = section[pg..].find("w:w=\"").unwrap() + pg + 5;
    let wend = section[wstart..].find('"').unwrap() + wstart;
    let wide: i64 = section[wstart..wend].parse().unwrap();
    let mut wide_section = section.clone();
    wide_section.replace_range(wstart..wend, &(wide + 7200).to_string());
    let first_close = xml.find("</w:pPr>").expect("a paragraph style");
    xml.insert_str(first_close, &wide_section);
    let mut files = support::entries(&support::sample());
    let i = files
        .iter()
        .position(|(n, _)| n == "word/document.xml")
        .unwrap();
    files[i].1 = xml.into_bytes();
    let refs: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(n, d)| (n.as_str(), d.as_slice()))
        .collect();
    let mut doc = Document::from_bytes(support::zip(&refs, false)).unwrap();
    let (narrow, first) = {
        let v = doc.views().unwrap();
        assert_eq!(v.sections().len(), 2, "one break, two sections");
        let wide = v.sections()[0].text_width.unwrap();
        let narrow = v.sections()[1].text_width.unwrap();
        assert!(wide - narrow > 100.0, "{wide} against {narrow}");
        (narrow, v.blocks()[0].node)
    };
    let big =
        content(json!([{"type": "image", "image": {"data": support::base64(&png(4000, 1000))}}]));
    doc.insert(Place::After(first), &big.blocks, &WriteOptions::default())
        .unwrap();
    let out = doc.to_bytes().unwrap();
    valid(&out);
    let mut doc = Document::from_bytes(out).unwrap();
    let images = doc.views().unwrap().images();
    assert_eq!(images.len(), 1);
    assert!(
        (images[0].width.unwrap() - narrow).abs() < 0.1,
        "{} against {narrow}",
        images[0].width.unwrap()
    );
}

#[test]
fn replacing_a_caption_by_nothing_renumbers_the_later_ones() {
    // A table before the one birds.docx has makes it Table 2; deleting the
    // new caption by replacing it with nothing makes it Table 1 again.
    let c = content(
        json!([{"type": "table", "caption": ["Weather"], "label": "tab-weather",
        "rows": [{"cells": [{"content": [{"type": "paragraph", "content": ["dry"]}]}]}]}]),
    );
    let mut doc = Document::from_bytes(corpus("birds.docx")).unwrap();
    let first = at(&mut doc, "heading:Survey sites");
    doc.insert(Place::Before(first), &c.blocks, &WriteOptions::default())
        .unwrap();
    let caption = {
        let v = doc.views().unwrap();
        v.paragraphs()
            .iter()
            .find(|p| p.text.starts_with("Table 1:"))
            .unwrap()
            .node
    };
    let nothing: &[Block] = &[];
    let r = doc
        .insert(Place::Replace(caption), nothing, &WriteOptions::default())
        .unwrap();
    assert!(r.renumbered >= 1, "{r:?}");
    let out = doc.to_bytes().unwrap();
    valid(&out);
    let mut doc = Document::from_bytes(out).unwrap();
    let text = doc.views().unwrap().text(&Default::default());
    assert!(!text.contains("Weather"), "{text}");
    assert!(text.contains("Table 1: Sites, visits and counts"), "{text}");
    assert!(text.contains("As Table 1 shows"), "{text}");
}

#[test]
fn table_widths_must_be_finite_numbers_above_zero() {
    let table = |widths| Table {
        rows: vec![Row {
            cells: vec![Cell::new(vec![]), Cell::new(vec![])],
        }],
        header_rows: 0,
        first_column: false,
        style: None,
        widths,
        caption: None,
        label: None,
    };
    let limits = nebu_docx::ContentLimits::default();
    for widths in [
        Some(vec![-1.0, 1.0]),
        Some(vec![0.0, 1.0]),
        Some(vec![f64::NAN, 1.0]),
        Some(vec![f64::INFINITY, 1.0]),
    ] {
        let c = Content {
            blocks: vec![Block::Table(table(widths))],
            table_styles: vec![],
        };
        let e = c.check(&limits).unwrap_err();
        assert!(e.to_string().contains("widths[0]"), "{e}");
    }
    // Huge but finite weights pass the checker and fail at the writer,
    // where their sum is no finite total and every column but the last
    // would collapse.
    let c = Content {
        blocks: vec![Block::Table(table(Some(vec![1e308, 1e308])))],
        table_styles: vec![],
    };
    c.check(&limits).unwrap();
    let mut doc = Document::from_bytes(support::sample()).unwrap();
    let e = doc
        .insert(Place::End, &c.blocks, &WriteOptions::default())
        .unwrap_err();
    assert!(e.to_string().contains("no finite total"), "{e}");
}

#[test]
fn hostile_sequence_numbers_stay_arabic() {
    // A reset past every sane count and a counter at the top of its range
    // must not hang or fill memory on an alphabetic or Roman format.
    let c = content(json!([
        {"type": "table", "caption": ["Dry"], "label": "tab-dry",
         "rows": [{"cells": [{"content": [{"type": "paragraph", "content": ["x"]}]}]}]},
        {"type": "table", "caption": ["Wet"], "label": "tab-wet",
         "rows": [{"cells": [{"content": [{"type": "paragraph", "content": ["y"]}]}]}]},
    ]));
    let mut doc = Document::from_bytes(support::sample()).unwrap();
    doc.insert(Place::End, &c.blocks, &WriteOptions::default())
        .unwrap();
    let bytes = doc.to_bytes().unwrap();
    let mut files = support::entries(&bytes);
    let i = files
        .iter()
        .position(|(n, _)| n == "word/document.xml")
        .unwrap();
    let xml = String::from_utf8(files[i].1.clone()).unwrap();
    let at = xml
        .find("SEQ Table \\* ARABIC")
        .expect("the first caption number");
    let mut xml = xml.into_bytes();
    let arabic = b"\\* ARABIC";
    xml.splice(
        at + "SEQ Table ".len()..at + "SEQ Table ".len() + arabic.len(),
        b"\\* ALPHABETIC \\r 18446744073709551615".iter().copied(),
    );
    files[i].1 = xml;
    let refs: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(n, d)| (n.as_str(), d.as_slice()))
        .collect();
    let mut doc = Document::from_bytes(support::zip(&refs, false)).unwrap();
    let caption = {
        let v = doc.views().unwrap();
        v.paragraphs()
            .iter()
            .find(|p| p.text.starts_with("Table 1:"))
            .unwrap()
            .node
    };
    let c = content(json!([{"type": "paragraph", "content": ["Before."]}]));
    doc.insert(Place::Before(caption), &c.blocks, &WriteOptions::default())
        .unwrap();
    let out = doc.to_bytes().unwrap();
    valid(&out);
    let mut doc = Document::from_bytes(out).unwrap();
    let numbers: Vec<String> = doc
        .views()
        .unwrap()
        .fields()
        .iter()
        .filter(|f| f.kind == "SEQ" && f.code.contains("Table") && !f.code.contains("\\c"))
        .map(|f| f.result.clone())
        .collect();
    assert_eq!(
        numbers,
        vec!["18446744073709551615", "18446744073709551615"],
        "{numbers:?}"
    );
}

#[test]
fn a_batch_deletion_and_insertion_renumbers_between_them() {
    // Deleting the first caption and adding one at the end in one batch:
    // the caption between them takes the deleted number.
    let two = content(json!([
        {"type": "table", "caption": ["Dry"], "label": "tab-dry",
         "rows": [{"cells": [{"content": [{"type": "paragraph", "content": ["x"]}]}]}]},
        {"type": "table", "caption": ["Wet"], "label": "tab-wet",
         "rows": [{"cells": [{"content": [{"type": "paragraph", "content": ["y"]}]}]}]},
    ]));
    let mut doc = Document::from_bytes(support::sample()).unwrap();
    doc.insert(Place::End, &two.blocks, &WriteOptions::default())
        .unwrap();
    let caption = {
        let v = doc.views().unwrap();
        v.paragraphs()
            .iter()
            .find(|p| p.text.starts_with("Table 1:"))
            .unwrap()
            .node
    };
    let late = content(
        json!([{"type": "table", "caption": ["Late"], "label": "tab-late",
        "rows": [{"cells": [{"content": [{"type": "paragraph", "content": ["z"]}]}]}]}]),
    );
    let nothing: &[Block] = &[];
    let r = doc
        .insert_all(
            &[
                (Place::Replace(caption), nothing),
                (Place::End, late.blocks.as_slice()),
            ],
            &WriteOptions::default(),
        )
        .unwrap();
    assert_eq!(r.renumbered, 1, "{r:?}");
    let out = doc.to_bytes().unwrap();
    valid(&out);
    let mut doc = Document::from_bytes(out).unwrap();
    let text = doc.views().unwrap().text(&Default::default());
    assert!(!text.contains("Dry"), "{text}");
    assert!(text.contains("Table 1: Wet"), "{text}");
    assert!(text.contains("Table 2: Late"), "{text}");
}

#[test]
fn a_deletion_keeps_what_stands_before_it() {
    // A stale caption before a deleted one keeps its bytes, and deleting
    // in a footer renumbers nothing in the body.
    let two = content(json!([
        {"type": "table", "caption": ["Dry"], "label": "tab-dry",
         "rows": [{"cells": [{"content": [{"type": "paragraph", "content": ["x"]}]}]}]},
        {"type": "table", "caption": ["Wet"], "label": "tab-wet",
         "rows": [{"cells": [{"content": [{"type": "paragraph", "content": ["y"]}]}]}]},
    ]));
    let mut doc = Document::from_bytes(support::sample()).unwrap();
    doc.insert(Place::End, &two.blocks, &WriteOptions::default())
        .unwrap();
    let bytes = doc.to_bytes().unwrap();
    let mut files = support::entries(&bytes);
    let i = files
        .iter()
        .position(|(n, _)| n == "word/document.xml")
        .unwrap();
    let mut xml = String::from_utf8(files[i].1.clone()).unwrap();
    let mark = xml.find("_Ref_tab-dry").expect("the label bookmark");
    let at = mark + xml[mark..].find(">1</w:t>").expect("the cached number");
    xml.replace_range(at + 1..at + 2, "7");
    files[i].1 = xml.into_bytes();
    let refs: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(n, d)| (n.as_str(), d.as_slice()))
        .collect();
    let mut doc = Document::from_bytes(support::zip(&refs, false)).unwrap();
    let caption = {
        let v = doc.views().unwrap();
        v.paragraphs()
            .iter()
            .find(|p| p.text.starts_with("Table 2:"))
            .unwrap()
            .node
    };
    let nothing: &[Block] = &[];
    let r = doc
        .insert(Place::Replace(caption), nothing, &WriteOptions::default())
        .unwrap();
    assert_eq!(r.renumbered, 0, "{r:?}");
    let footer = {
        let v = doc.views().unwrap();
        v.paragraphs()
            .iter()
            .find(|p| p.location.story == StoryKind::Footer)
            .unwrap()
            .node
    };
    let r = doc
        .insert(Place::Replace(footer), nothing, &WriteOptions::default())
        .unwrap();
    assert_eq!(r.renumbered, 0, "{r:?}");
    let out = doc.to_bytes().unwrap();
    valid(&out);
    let mut doc = Document::from_bytes(out).unwrap();
    let text = doc.views().unwrap().text(&Default::default());
    assert!(!text.contains("Wet"), "{text}");
    assert!(!text.contains("Table 2"), "{text}");
    assert!(text.contains("Table 7: Dry"), "{text}");
}

#[test]
fn tracked_insertion_marks_new_blocks_as_revisions() {
    use nebu_docx::{RevisionTarget, Revisions, Track};
    let c = content(json!([
        {"type": "paragraph", "content": ["Tracked hello."]},
        {"type": "heading", "level": 2, "content": ["Tracked heading"]},
        {"type": "table", "header_rows": 1, "rows": [
            {"cells": [{"content": [{"type": "paragraph", "content": ["H"]}]}]},
            {"cells": [{"content": [{"type": "paragraph", "content": ["B"]}]}]},
        ]},
    ]));
    let mut doc = Document::from_bytes(support::sample()).unwrap();
    let opts = WriteOptions {
        track: Some(Track {
            author: "Analyst".into(),
            date: "2026-11-19T00:00:00Z".into(),
        }),
        ..Default::default()
    };
    doc.insert(Place::End, &c.blocks, &opts).unwrap();
    let out = doc.to_bytes().unwrap();
    valid(&out);
    let xml = String::from_utf8(support::entry(&out, "word/document.xml")).unwrap();
    assert!(xml.contains("<w:ins "), "{xml}");
    assert!(xml.contains("w:author=\"Analyst\""), "{xml}");
    assert!(xml.contains("<w:trPr>"), "{xml}");
    // Paragraph marks carry their own insertion record; the views merge a
    // paragraph's text and its mark into one revision when they share
    // author and date (collect.rs), so the XML is the place that shows both.
    assert!(xml.contains("<w:rPr><w:ins "), "{xml}");
    let mut doc = Document::from_bytes(out).unwrap();
    let v = doc.views().unwrap();
    let mine: Vec<_> = v
        .revisions()
        .iter()
        .filter(|r| r.author == "Analyst")
        .collect();
    assert!(!mine.is_empty(), "no tracked revisions by Analyst");
    // The views merge a paragraph's text and its mark into one revision
    // when author and date match, so Text covers both here; rows stay
    // separate.
    assert!(
        mine.iter().any(|r| r.target == RevisionTarget::Text),
        "no text insertion: {:?}",
        mine.iter().map(|r| r.target).collect::<Vec<_>>()
    );
    assert!(
        mine.iter().any(|r| r.target == RevisionTarget::Row),
        "no row insertion: {:?}",
        mine.iter().map(|r| r.target).collect::<Vec<_>>()
    );
    let accepted = v.text(&Default::default());
    assert!(accepted.contains("Tracked hello."), "{accepted}");
    let rejected = v.text(&nebu_docx::TextOptions {
        revisions: Revisions::Rejected,
        ..Default::default()
    });
    assert!(!rejected.contains("Tracked hello."), "{rejected}");
    assert!(!rejected.contains("Tracked heading"), "{rejected}");

    // New revision ids continue past every revision record the document
    // has, including formatting changes, and never reuse one.
    let mut files = support::entries(&support::sample());
    let existing: std::collections::HashSet<u64> = files
        .iter()
        .filter(|(n, _)| n == "word/document.xml" || n == "word/numbering.xml")
        .flat_map(|(_, d)| {
            let s = String::from_utf8_lossy(d);
            s.match_indices("w:id=\"")
                .filter_map(|(at, _)| s[at + 6..].split('"').next()?.parse::<u64>().ok())
                .collect::<Vec<_>>()
        })
        .collect();
    let i = files
        .iter()
        .position(|(n, _)| n == "word/document.xml")
        .unwrap();
    let mut doc_xml = String::from_utf8(files[i].1.clone()).unwrap();
    // Declare w14 as ignorable so the validator leaves the planted conflict
    // record out, as a reader that knows only the transitional schemas
    // would; the writer's id scan still reserves its w:id.
    doc_xml = doc_xml.replacen(
        "xmlns:wp=\"http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing\"",
        "xmlns:wp=\"http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing\" xmlns:w14=\"http://schemas.microsoft.com/office/word/2010/wordml\" xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\" mc:Ignorable=\"w14\"",
        1,
    );
    let anchor = doc_xml
        .rfind("<w:sectPr")
        .or_else(|| doc_xml.find("</w:body>"))
        .expect("a body");
    doc_xml.insert_str(
        anchor,
        concat!(
            "<w:p><w:pPr><w:pPrChange w:id=\"900\" w:author=\"Old\" w:date=\"2026-01-01T00:00:00Z\"><w:pPr/></w:pPrChange></w:pPr>",
            "<w:r><w:rPr><w:rPrChange w:id=\"901\" w:author=\"Old\" w:date=\"2026-01-01T00:00:00Z\"><w:rPr/></w:rPrChange></w:rPr><w:t>old</w:t></w:r></w:p>",
            "<w:p><w:pPr><w:numPr><w:ilvl w:val=\"0\"/><w:numId w:val=\"1\"/><w:numberingChange w:id=\"903\" w:author=\"Old\" w:date=\"2026-01-01T00:00:00Z\" w:original=\"1:0\"/></w:numPr></w:pPr><w:r><w:t>numbered old</w:t></w:r></w:p>",
            "<w:customXmlInsRangeStart w:id=\"904\" w:author=\"Old\" w:date=\"2026-01-01T00:00:00Z\"/><w:customXmlInsRangeEnd w:id=\"904\"/>",
            "<w14:conflictIns w:id=\"905\" w:author=\"Old\" w:date=\"2026-01-01T00:00:00Z\"/>",
        ),
    );
    files[i].1 = doc_xml.into_bytes();
    let refs: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(n, d)| (n.as_str(), d.as_slice()))
        .collect();
    let mut doc = Document::from_bytes(support::zip(&refs, false)).unwrap();
    let c = content(json!([{"type": "paragraph", "content": ["After formatting."]}]));
    doc.insert(Place::End, &c.blocks, &opts).unwrap();
    let out = doc.to_bytes().unwrap();
    valid(&out);
    let xml = String::from_utf8(support::entry(&out, "word/document.xml")).unwrap();
    let fresh: Vec<u64> = xml
        .match_indices("<w:ins w:id=\"")
        .filter_map(|(at, _)| {
            xml[at + "<w:ins w:id=\"".len()..]
                .split('"')
                .next()?
                .parse::<u64>()
                .ok()
        })
        .collect();
    // One paragraph writes two insertion records (paragraph mark and body);
    // both must be present and their ids must differ. Checking the raw
    // list, not a deduplicated one, so reuse cannot hide.
    assert!(
        fresh.len() >= 2,
        "expected paragraph-mark and body insertions in {xml}"
    );
    let distinct: std::collections::HashSet<u64> = fresh.iter().copied().collect();
    assert_eq!(
        distinct.len(),
        fresh.len(),
        "new insertion ids reuse each other: {fresh:?}"
    );
    for id in &distinct {
        assert!(
            !existing.contains(id),
            "new insertion id {id} reuses an existing id"
        );
        assert!(
            ![900, 901, 903, 904, 905].contains(id),
            "new insertion id {id} reuses a planted change id"
        );
    }
}

#[test]
fn picture_files_stay_inside_their_directory() {
    let dir = std::env::temp_dir().join(format!("tabellion-files-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("bird.png"), png(20, 20)).unwrap();
    let opts = WriteOptions {
        files: Some(dir.clone()),
        ..Default::default()
    };
    for file in ["../bird.png", "/bird.png", "..\\bird.png", "\\bird.png"] {
        let c = content(json!([{"type": "image", "image": {"file": file}}]));
        let mut doc = Document::from_bytes(support::sample()).unwrap();
        let e = doc.insert(Place::End, &c.blocks, &opts).unwrap_err();
        assert!(
            e.to_string().contains("not inside the files directory"),
            "{file}: {e}"
        );
    }
    // A plain name in the directory still reads.
    let c = content(json!([{"type": "image", "image": {"file": "bird.png"}}]));
    let mut doc = Document::from_bytes(support::sample()).unwrap();
    doc.insert(Place::End, &c.blocks, &opts).unwrap();
    valid(&doc.to_bytes().unwrap());
    #[cfg(unix)]
    {
        // A link inside the directory that points outside it does not.
        let outside =
            std::env::temp_dir().join(format!("tabellion-outside-{}.png", std::process::id()));
        std::fs::write(&outside, png(20, 20)).unwrap();
        std::os::unix::fs::symlink(&outside, dir.join("evil.png")).unwrap();
        let c = content(json!([{"type": "image", "image": {"file": "evil.png"}}]));
        let mut doc = Document::from_bytes(support::sample()).unwrap();
        let e = doc.insert(Place::End, &c.blocks, &opts).unwrap_err();
        assert!(
            e.to_string().contains("not inside the files directory"),
            "{e}"
        );
        // A link to a file inside still reads.
        std::os::unix::fs::symlink(dir.join("bird.png"), dir.join("kind.png")).unwrap();
        let c = content(json!([{"type": "image", "image": {"file": "kind.png"}}]));
        let mut doc = Document::from_bytes(support::sample()).unwrap();
        doc.insert(Place::End, &c.blocks, &opts).unwrap();
        valid(&doc.to_bytes().unwrap());
        std::fs::remove_file(&outside).ok();
    }
    std::fs::remove_dir_all(&dir).ok();
}
