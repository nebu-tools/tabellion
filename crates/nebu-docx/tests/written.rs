//! Documents the writer makes, and what Word makes of them.
//!
//! Each fixture under `corpus/written/` is written by [`fixtures`] from a
//! document of the corpus or the sample template and content of the bird
//! survey. Next to it, `NAME.word.json` is what Word reported when it
//! opened the file (`tools/word/written.sh`), and `NAME.saved.docx` is the
//! file Word saved. The tests check that:
//!
//! - the writer still makes the committed fixtures, byte for byte, so the
//!   readings are of the code as it is (`TABELLION_WRITE_FIXTURES=1 cargo
//!   test --test written` writes them anew; then run Word again);
//! - Word opened each without repair and without conversion;
//! - what Word reports agrees with the views and with the content: the
//!   paragraphs and tables, the styles of the paragraphs, the numbers of
//!   list items as Word shows them, the captions Word lists for
//!   cross-references, every field's result before and after Word updates
//!   the fields, the footnotes, the pictures and their sizes, the table
//!   styles with their header row and banding, and the text each
//!   cross-reference points to;
//! - the file Word saved still has every label, and the cross-references
//!   still name them.

mod support;

use std::path::{Path, PathBuf};

use nebu_docx::content::{Block, Content, Inline};
use nebu_docx::{Document, Locator, Place, StoryKind, WriteOptions};
use serde_json::{Value, json};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn corpus(name: &str) -> Vec<u8> {
    std::fs::read(root().join("corpus").join(name)).unwrap()
}

fn data(name: &str) -> String {
    support::base64(&std::fs::read(root().join("crates/nebu-docx/tests/data").join(name)).unwrap())
}

fn survey() -> Content {
    let json = std::fs::read(root().join("samples/survey.json")).unwrap();
    let mut c = Content::from_json(&json).unwrap();
    // The JSON names its picture as a file; the fixtures carry the bytes.
    for b in &mut c.blocks {
        if let Block::Figure { image, .. } = b {
            image.source = nebu_docx::content::Source::Data(
                std::fs::read(root().join("samples/wren.png")).unwrap(),
            );
        }
    }
    c
}

fn content(v: Value) -> Content {
    Content::from_json(v.to_string().as_bytes()).unwrap()
}

fn locate(doc: &mut Document, l: &str) -> nebu_docx::xml::NodeId {
    let l: Locator = l.parse().unwrap();
    doc.views().unwrap().locate(&l).unwrap().node
}

/// The fixtures: name and bytes.
fn fixtures() -> Vec<(&'static str, Vec<u8>)> {
    let mut out = vec![];
    // The README's example: the survey in place of {{results}}.
    let mut doc = Document::from_bytes(support::sample()).unwrap();
    let c = survey();
    let at = locate(&mut doc, "placeholder:results");
    doc.insert(
        Place::Replace(at),
        &c.blocks,
        &WriteOptions::for_content(&c, None),
    )
    .unwrap();
    out.push(("survey-sample", doc.to_bytes().unwrap()));

    // A document Word made, with captions and references of its own: a
    // table and a figure before them renumber them, and content goes into
    // a table cell, a header, a footnote and a text box.
    let mut doc = Document::from_bytes(corpus("birds.docx")).unwrap();
    let c = content(json!({"blocks": [
        {"type": "table", "caption": ["Weather on the survey days"], "label": "tab-weather", "header_rows": 1,
         "rows": [
            {"cells": [{"content": [{"type": "paragraph", "content": ["Day"]}]}, {"content": [{"type": "paragraph", "content": ["Sky"]}]}]},
            {"cells": [{"content": [{"type": "paragraph", "content": ["3 May"]}]}, {"content": [{"type": "paragraph", "content": ["clear"]}]}]}
         ]},
        {"type": "figure", "image": {"data": data("heron.jpg"), "alt": "A heron"}, "caption": ["A heron on the east pool"], "label": "fig-heron"},
        {"type": "paragraph", "content": [
            "The weather is in ", {"type": "reference", "label": "tab-weather"},
            ", the heron in ", {"type": "reference", "label": "fig-heron"}, "."
        ]}
    ]}));
    let first = locate(&mut doc, "heading:Survey sites");
    let (cell, header, note, text_box) = {
        let v = doc.views().unwrap();
        let find = |f: &dyn Fn(&nebu_docx::Paragraph) -> bool| {
            v.paragraphs().iter().find(|p| f(p)).unwrap().node
        };
        (
            find(&|p| p.cell.is_some() && p.text == "Reed beds"),
            find(&|p| p.location.story == StoryKind::Header),
            find(&|p| p.location.story == StoryKind::Footnote),
            find(&|p| p.location.story == StoryKind::TextBox),
        )
    };
    let small = |t: &str| {
        vec![Block::Paragraph {
            style: None,
            align: None,
            content: vec![Inline::text(t)],
        }]
    };
    let in_cell = small("Wrens nest here.");
    let in_header = small("Draft for the survey group");
    let in_note = content(json!([{"type": "table", "rows": [{"cells": [
        {"content": [{"type": "paragraph", "content": ["dawn"]}]}, {"content": [{"type": "paragraph", "content": ["05:00"]}]}]}]}]));
    let in_box = small("Seen from the hide.");
    doc.insert_all(
        &[
            (Place::Before(first), &c.blocks),
            (Place::After(cell), &in_cell),
            (Place::After(header), &in_header),
            (Place::After(note), &in_note.blocks),
            (Place::After(text_box), &in_box),
        ],
        &WriteOptions::default(),
    )
    .unwrap();
    out.push(("survey-birds", doc.to_bytes().unwrap()));

    // A document without numbering, footnotes or Table Grid: the writer
    // adds them.
    let mut doc = Document::from_bytes(corpus("cascade.docx")).unwrap();
    let c = survey();
    doc.insert(Place::End, &c.blocks, &WriteOptions::for_content(&c, None))
        .unwrap();
    let plain = content(json!([
        {"type": "heading", "level": 1, "content": ["Plain tables"]},
        {"type": "table", "rows": [{"cells": [{"content": [{"type": "paragraph", "content": ["Wren"]}]}, {"content": [{"type": "paragraph", "content": ["4"]}]}]}]},
        {"type": "list", "ordered": false, "items": [{"content": ["reed"]}, {"content": ["sedge"], "list": {"ordered": true, "items": [{"content": ["first"]}, {"content": ["second"]}]}}]},
        {"type": "list", "ordered": true, "items": [{"content": ["again from one"]}, {"content": ["and two"]}]}
    ]));
    doc.insert(Place::End, &plain.blocks, &WriteOptions::default())
        .unwrap();
    out.push(("parts-added", doc.to_bytes().unwrap()));

    // Pictures of every format, and the same picture twice.
    let svg = support::base64(br##"<svg xmlns="http://www.w3.org/2000/svg" width="160" height="80" viewBox="0 0 160 80"><rect width="160" height="80" fill="#BED2EB"/><ellipse cx="70" cy="50" rx="40" ry="14" fill="#555"/><circle cx="112" cy="36" r="10" fill="#555"/></svg>"##);
    let mut doc = Document::from_bytes(support::sample()).unwrap();
    let c = content(json!([
        {"type": "figure", "image": {"data": data("wren.png"), "alt": "A wren"}, "caption": ["PNG"], "label": "fig-png"},
        {"type": "figure", "image": {"data": data("heron.jpg"), "alt": "A heron"}, "caption": ["JPEG at 150 pixels per inch"], "label": "fig-jpeg"},
        {"type": "figure", "image": {"data": data("teal.gif"), "alt": "A teal"}, "caption": ["GIF"], "label": "fig-gif"},
        {"type": "figure", "image": {"data": svg, "fallback": {"data": data("wren.png")}, "alt": "A drawn wren"}, "caption": ["SVG with a PNG fallback"], "label": "fig-svg"},
        {"type": "paragraph", "content": ["The same wren again, smaller: ", {"type": "image", "image": {"data": data("wren.png"), "width": 36}}]},
        {"type": "paragraph", "content": [{"type": "reference", "label": "fig-png"}, " to ", {"type": "reference", "label": "fig-svg"}, " show four formats."]}
    ]));
    let at = locate(&mut doc, "heading:Methods");
    doc.insert(Place::Before(at), &c.blocks, &WriteOptions::default())
        .unwrap();
    out.push(("pictures", doc.to_bytes().unwrap()));

    // Tables: spans both ways, a nested table, borders and shading over the
    // style, a first column, widths, and a style the template has.
    let mut doc = Document::from_bytes(support::sample()).unwrap();
    let cell = |t: &str| json!({"content": [{"type": "paragraph", "content": [t]}]});
    let c = content(json!({"blocks": [
        {"type": "table", "header_rows": 2, "first_column": true, "style": "Marsh Grid", "caption": ["Counts on two visits"], "label": "tab-visits",
         "widths": [3, 1, 1, 1, 1],
         "rows": [
            {"cells": [{"content": [{"type": "paragraph", "content": ["Site"]}], "row_span": 2, "vertical_align": "center"},
                       {"content": [{"type": "paragraph", "content": ["First visit"]}], "column_span": 2, "align": "center"},
                       {"content": [{"type": "paragraph", "content": ["Second visit"]}], "column_span": 2, "align": "center"}]},
            {"cells": [cell("Visits"), cell("Pairs"), cell("Visits"), cell("Pairs")]},
            {"cells": [cell("North"), cell("6"), cell("21"), cell("6"), cell("20.5")]},
            {"cells": [cell("East"), {"content": [{"type": "paragraph", "content": ["5"]}], "shading": "FFF2CC",
                        "borders": {"bottom": {"width": 1.5, "color": "7F6000"}, "top": {"width": 0.5, "color": "7F6000", "style": "dashed"}}},
                       cell("17"), cell("4"), cell("16")]},
            {"cells": [{"content": [
                {"type": "paragraph", "content": ["Species at the south site"]},
                {"type": "table", "rows": [{"cells": [cell("Wren"), cell("4")]}, {"cells": [cell("Reed warbler"), cell("7")]}]}
              ], "column_span": 5}]}
         ]},
        {"type": "table", "style": "Report Table", "header_rows": 1, "caption": ["The template's own table style"], "label": "tab-template",
         "rows": [{"cells": [cell("Site"), cell("Pairs")]}, {"cells": [cell("West"), cell("28")]}]},
        {"type": "paragraph", "content": ["See ", {"type": "reference", "label": "tab-visits"}, " and ", {"type": "reference", "label": "tab-template"}, "."]}
     ],
     "table_styles": [{"name": "Marsh Grid", "font": "Arial", "size": 9,
        "borders": {"top": {"width": 1.5, "color": "7F6000"}, "bottom": {"width": 1.5, "color": "7F6000"}, "inside_h": {"width": 0.5, "color": "BF9000"}},
        "padding": {"top": 1, "right": 5.4, "bottom": 1, "left": 5.4},
        "header": {"fill": "7F6000", "color": "FFFFFF", "bold": true, "border": {"width": 1, "color": "FFFFFF"}},
        "band_fill": "FFF2CC",
        "first_column": {"bold": true}}]}));
    let at = locate(&mut doc, "heading:Methods");
    doc.insert(
        Place::Before(at),
        &c.blocks,
        &WriteOptions::for_content(&c, None),
    )
    .unwrap();
    out.push(("tables", doc.to_bytes().unwrap()));

    // Equations, numbered and not, in the line and on their own, and the
    // references to them.
    let mut doc = Document::from_bytes(corpus("birds.docx")).unwrap();
    let frac = "<m:oMath><m:r><m:t>d</m:t></m:r><m:r><m:t>=</m:t></m:r><m:f><m:num><m:r><m:t>n</m:t></m:r></m:num><m:den><m:r><m:t>A</m:t></m:r></m:den></m:f></m:oMath>";
    let sum = "<m:oMath><m:r><m:t>N</m:t></m:r><m:r><m:t>=</m:t></m:r><m:nary><m:naryPr><m:chr m:val=\"∑\"/></m:naryPr><m:sub><m:r><m:t>i</m:t></m:r></m:sub><m:sup><m:r><m:t>k</m:t></m:r></m:sup><m:e><m:sSub><m:e><m:r><m:t>n</m:t></m:r></m:e><m:sub><m:r><m:t>i</m:t></m:r></m:sub></m:sSub></m:e></m:nary></m:oMath>";
    let c = content(json!([
        {"type": "heading", "level": 2, "content": ["Density"], "label": "sec-density"},
        {"type": "equation", "math": {"omml": frac}, "label": "eq-density"},
        {"type": "equation", "math": {"omml": sum}, "label": "eq-total"},
        {"type": "equation", "math": {"omml": "<m:oMath><m:r><m:t>A</m:t></m:r><m:r><m:t>=</m:t></m:r><m:r><m:t>4</m:t></m:r></m:oMath>"}},
        {"type": "paragraph", "content": [
            "With ", {"type": "math", "math": {"omml": "<m:oMath><m:r><m:t>n</m:t></m:r></m:oMath>"}}, " pairs, ",
            {"type": "reference", "label": "eq-density"}, " gives the density and ",
            {"type": "reference", "label": "eq-total"}, " the total; see ",
            {"type": "reference", "label": "sec-density"}, "."
        ]}
    ]));
    let at = locate(&mut doc, "heading:Methods");
    doc.insert(Place::Before(at), &c.blocks, &WriteOptions::default())
        .unwrap();
    out.push(("equations", doc.to_bytes().unwrap()));
    out
}

fn written() -> PathBuf {
    root().join("corpus/written")
}

#[test]
fn the_writer_makes_the_committed_fixtures() {
    let write = std::env::var_os("TABELLION_WRITE_FIXTURES").is_some();
    let mut stale = vec![];
    for (name, bytes) in fixtures() {
        let errors = support::xsd::validate_docx(&bytes);
        assert!(errors.is_empty(), "{name}:\n{}", errors.join("\n"));
        let path = written().join(format!("{name}.docx"));
        if write {
            std::fs::create_dir_all(written()).unwrap();
            std::fs::write(&path, &bytes).unwrap();
        } else if std::fs::read(&path).ok().as_deref() != Some(bytes.as_slice()) {
            stale.push(name);
        }
    }
    assert!(
        stale.is_empty(),
        "the writer no longer makes {stale:?}; run TABELLION_WRITE_FIXTURES=1 cargo test --test written, then tools/word/written.sh"
    );
}

fn reading(name: &str) -> Value {
    let p = written().join(format!("{name}.word.json"));
    serde_json::from_slice(
        &std::fs::read(&p)
            .unwrap_or_else(|_| panic!("{name} has no Word reading; run tools/word/written.sh")),
    )
    .unwrap()
}

/// Word's text in the views' terms: a cell end is a tab, a paragraph end
/// or a line break a line end, no character for a note mark or a picture,
/// and no line end or tab at the end.
fn word_text(s: &str) -> String {
    s.replace("\r\x07", "\t")
        .replace(['\r', '\x0B'], "\n")
        .replace(['\x01', '\x02', '/'], "")
        .trim_end_matches(['\n', '\t'])
        .to_string()
}

#[test]
fn word_opens_every_fixture_and_agrees_with_the_views() {
    let mut failures = vec![];
    for (name, _) in fixtures() {
        let r = reading(name);
        let mut fail = |what: String| failures.push(format!("{name}: {what}"));
        if r["opened"] != json!(true) || !r["error"].is_null() {
            fail(format!("Word did not open it: {}", r["error"]));
            continue;
        }
        if r["converted"] == json!(true) {
            fail("Word converted it".into());
        }
        let mut doc = Document::open(written().join(format!("{name}.docx"))).unwrap();
        let v = doc.views().unwrap();
        // Tables of the body, without the nested ones, which Word counts
        // with the table that holds them.
        let ours = v
            .tables()
            .iter()
            .filter(|t| t.location.story == StoryKind::Main && t.parent.is_none())
            .count();
        if r["table_count"].as_u64() != Some(ours as u64) {
            fail(format!(
                "{} tables in Word, {ours} in the views",
                r["table_count"]
            ));
        }
        // The paragraphs of the body with their styles: those outside
        // tables one by one, those in tables that have text in order. Word
        // lists the end of each row as a paragraph of its own.
        let word: Vec<(bool, String, String)> = r["paragraphs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| {
                (
                    p["in_table"].as_bool().unwrap_or(false),
                    p["style"].as_str().unwrap_or("").to_string(),
                    word_text(p["text"].as_str().unwrap_or("")),
                )
            })
            .filter(|(t, _, text)| !t || !text.is_empty())
            .collect();
        let mine: Vec<(bool, String, String)> = v
            .paragraphs()
            .iter()
            .filter(|p| p.location.story == StoryKind::Main)
            .map(|p| {
                (
                    p.cell.is_some(),
                    p.style_name.clone().unwrap_or_default(),
                    p.text.clone(),
                )
            })
            .filter(|(t, _, text)| !t || !text.is_empty())
            .collect();
        // Word joins a paragraph whose mark is hidden to the next one, as
        // cascade.docx has them at its top; there the lists are compared
        // from the first inserted paragraph on.
        let (word, mine) = if word.len() != mine.len() && name == "parts-added" {
            let at = |l: &[(bool, String, String)]| {
                l.iter().position(|p| p.2 == "Counts by site").unwrap_or(0)
            };
            (word[at(&word)..].to_vec(), mine[at(&mine)..].to_vec())
        } else {
            (word, mine)
        };
        if word.len() != mine.len() {
            fail(format!(
                "{} paragraphs in Word, {} in the views",
                word.len(),
                mine.len()
            ));
        } else {
            for (i, (w, m)) in word.iter().zip(&mine).enumerate() {
                if w.0 != m.0 || w.1 != m.1 {
                    fail(format!(
                        "paragraph {i} {:?}: style {:?} in Word, {:?} in the views",
                        m.2, w.1, m.1
                    ));
                }
            }
        }
        // Fields: Word's update leaves every result as written.
        let before = r["fields_before"].as_array().unwrap();
        let after = r["fields_after"].as_array().unwrap();
        for (b, a) in before.iter().zip(after) {
            let code = b["code"].as_str().unwrap_or("");
            if code.starts_with("PAGE")
                || code.starts_with("NUMPAGES")
                || code.starts_with("TOC")
                || code.starts_with("PAGEREF")
                || code.starts_with("DATE")
            {
                continue;
            }
            if b["result"] != a["result"] {
                fail(format!(
                    "{code}: {} as written, {} after Word's update",
                    b["result"], a["result"]
                ));
            }
        }
        // Each cross-reference shows the text of the bookmark it names.
        for x in r["references"].as_array().unwrap() {
            if word_text(x["result"].as_str().unwrap_or(""))
                != word_text(x["target"].as_str().unwrap_or(""))
            {
                fail(format!(
                    "{}: shows {} but its bookmark holds {}",
                    x["code"], x["result"], x["target"]
                ));
            }
        }
        // Captions Word lists for cross-references, against the views. Word
        // lists a numbered equation by its paragraph, the number at the end.
        let equations: Vec<String> = r["captions"]["Equation"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| {
                i.as_str()
                    .unwrap()
                    .rsplit('\t')
                    .next()
                    .unwrap_or("")
                    .to_string()
            })
            .collect();
        let numbered: Vec<String> = (1..=v
            .fields()
            .iter()
            .filter(|f| f.code.starts_with("SEQ Equation") && f.location.story == StoryKind::Main)
            .count())
            .map(|n| format!("({n})"))
            .collect();
        if equations != numbered {
            fail(format!("equations: Word {equations:?}, views {numbered:?}"));
        }
        for (label, items) in r["captions"].as_object().unwrap() {
            if label == "Equation" {
                continue;
            }
            let ours: Vec<String> = v
                .paragraphs()
                .iter()
                .filter(|p| {
                    p.location.story == StoryKind::Main && p.text.starts_with(&format!("{label} "))
                })
                .filter(|p| p.style_name.as_deref() == Some("Caption"))
                .map(|p| p.text.clone())
                .collect();
            let word: Vec<String> = items
                .as_array()
                .unwrap()
                .iter()
                .map(|i| i.as_str().unwrap().to_string())
                .collect();
            if word != ours {
                fail(format!("captions {label}: Word {word:?}, views {ours:?}"));
            }
        }
        // Footnotes in order.
        let notes: Vec<String> = v
            .notes()
            .iter()
            .filter(|n| n.kind == nebu_docx::NoteKind::Footnote && n.number.is_some())
            .map(|n| n.text.clone())
            .collect();
        let word: Vec<String> = r["footnotes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| word_text(n["text"].as_str().unwrap()).trim().to_string())
            .collect();
        if word != notes {
            fail(format!("footnotes: Word {word:?}, views {notes:?}"));
        }
        // Pictures and their sizes.
        let ours: Vec<(f64, f64)> = v
            .images()
            .iter()
            .filter(|i| {
                i.location.story == StoryKind::Main && i.placement == nebu_docx::Placement::Inline
            })
            .map(|i| (i.width.unwrap_or(0.0), i.height.unwrap_or(0.0)))
            .collect();
        let word: Vec<(f64, f64)> = r["inline_shapes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| (s["width"].as_f64().unwrap(), s["height"].as_f64().unwrap()))
            .collect();
        if word.len() != ours.len()
            || word
                .iter()
                .zip(&ours)
                .any(|(a, b)| (a.0 - b.0).abs() > 0.1 || (a.1 - b.1).abs() > 0.1)
        {
            fail(format!("pictures: Word {word:?}, views {ours:?}"));
        }
        // Table styles: in the document, applied, with header row and
        // banding as the table asks.
        for t in r["tables"].as_array().unwrap() {
            let style = t["style"].as_str().unwrap_or("");
            if !r["table_styles"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s["name"] == style)
            {
                fail(format!(
                    "the table style {style} is not among the document's"
                ));
            }
            if t["heading_rows"] != json!(t["header_rows"].as_u64().unwrap_or(0) > 0) {
                fail(format!(
                    "table in {style}: heading rows {} with {} header rows",
                    t["heading_rows"], t["header_rows"]
                ));
            }
            if t["row_bands"] != json!(true) {
                fail(format!("table in {style}: no banded rows"));
            }
        }
        // The header rows and first columns the content asked for.
        let want: &[(&str, u64, bool)] = match name {
            "tables" => &[("Marsh Grid", 2, true), ("Report Table", 1, false)],
            "survey-sample" => &[("Field Green", 1, false)],
            _ => &[],
        };
        for (i, (style, rows, first)) in want.iter().enumerate() {
            let t = &r["tables"][i];
            if t["style"] != json!(style)
                || t["header_rows"] != json!(rows)
                || t["first_column"] != json!(first)
            {
                fail(format!(
                    "table {i}: Word reads {t}, the content asks for {style}, {rows} header rows, first column {first}"
                ));
            }
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn list_numbers_are_what_word_shows() {
    // As Word shows them: each list starts at 1, nested items take their
    // own level, a bulleted list has no number.
    let r = reading("parts-added");
    let shown: Vec<(String, String)> = r["paragraphs"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| !p["list"].as_str().unwrap_or("").is_empty())
        .map(|p| {
            (
                p["list"].as_str().unwrap().to_string(),
                word_text(p["text"].as_str().unwrap()),
            )
        })
        .collect();
    let texts: Vec<(&str, &str)> = shown
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    for want in [
        ("1.", "Walk the transect at dawn"),
        ("2.", "Count singing males as pairs"),
        // The second level of the writer's own numbered list.
        ("a.", "first"),
        ("b.", "second"),
        ("1.", "again from one"),
        ("2.", "and two"),
    ] {
        assert!(texts.contains(&want), "{want:?} not in {texts:?}");
    }
    let bullets: Vec<&&str> = texts
        .iter()
        .filter(|(_, t)| ["slowly", "stopping every 100 m", "reed", "sedge"].contains(t))
        .map(|(n, _)| n)
        .collect();
    assert_eq!(bullets.len(), 4, "{texts:?}");
    assert!(
        bullets
            .iter()
            .all(|n| !n.chars().any(|c| c.is_ascii_digit())),
        "{bullets:?}"
    );
}

#[test]
fn labels_survive_a_save_by_word() {
    for (name, _) in fixtures() {
        let mut ours = Document::open(written().join(format!("{name}.docx"))).unwrap();
        let labels: Vec<String> = ours
            .views()
            .unwrap()
            .bookmarks()
            .iter()
            .filter(|b| b.name.starts_with("_Ref_"))
            .map(|b| b.name.clone())
            .collect();
        let saved = written().join(format!("{name}.saved.docx"));
        let mut doc = Document::open(&saved).unwrap_or_else(|e| panic!("{name}: {e}"));
        let v = doc.views().unwrap();
        for l in &labels {
            let b = v.bookmarks().iter().find(|b| &b.name == l);
            assert!(b.is_some(), "{name}: Word's save lost {l}");
            let refs = v
                .fields()
                .iter()
                .filter(|f| f.code.contains(l.as_str()))
                .count();
            let had = ours_refs(&written().join(format!("{name}.docx")), l);
            assert_eq!(
                refs, had,
                "{name}: {l}: {had} references before Word's save, {refs} after"
            );
        }
    }
}

fn ours_refs(path: &Path, label: &str) -> usize {
    let mut d = Document::open(path).unwrap();
    d.views()
        .unwrap()
        .fields()
        .iter()
        .filter(|f| f.code.contains(label))
        .count()
}
