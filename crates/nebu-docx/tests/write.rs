//! The writer: content into documents, checked against the schemas, read
//! back through the views, and compared byte for byte outside the
//! insertion.

mod support;

use std::path::Path;

use nebu_docx::content::{Block, Image, Inline};
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
    let mut small = nebu_docx::ContentLimits::default();
    small.max_items = 3;
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
        .ensure_table_style(&TableStyle::Defined(def.clone()))
        .unwrap();
    assert_eq!(doc.to_bytes().unwrap(), support::sample());
    let v = doc.views().unwrap();
    assert_eq!(v.styles().get(&id).unwrap().name, "Report Table");
    // A new one is a table style Word lists, with its sections.
    let fresh = nebu_docx::content::TableStyleDef {
        name: "Field Green".into(),
        ..def
    };
    let id = doc.ensure_table_style(&TableStyle::Defined(fresh)).unwrap();
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
