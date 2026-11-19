//! Every view of every corpus document has the JSON shape that
//! `schema/views.json` gives, and the schema names every field the views
//! write.
//!
//! The validator (`support/jsonschema.rs`) knows the part of JSON Schema
//! the schemas use.

mod support;

use std::path::Path;

use nebu_docx::Document;
use serde_json::{Map, Value, json};
use support::jsonschema::validate;

fn schema() -> Value {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("schema/views.json");
    serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap()
}

/// Every view of a document in the shape the command line prints.
fn views(doc: &mut Document) -> Vec<(&'static str, Value)> {
    let sha = doc.sha256();
    let mut out = vec![];
    let mut add = |key: &'static str, v: Value| {
        let mut m = Map::new();
        m.insert("sha256".into(), json!(sha));
        m.insert(key.into(), v);
        out.push((key, Value::Object(m)));
    };
    let v = doc.views().unwrap();
    let j = |x: serde_json::Result<Value>| x.unwrap();
    add("blocks", j(serde_json::to_value(v.blocks())));
    add("paragraphs", j(serde_json::to_value(v.paragraphs())));
    let styles: Vec<_> = v.styles().iter().collect();
    add("styles", j(serde_json::to_value(styles)));
    add("tables", j(serde_json::to_value(v.tables())));
    add("fields", j(serde_json::to_value(v.fields())));
    add("comments", j(serde_json::to_value(v.comments())));
    add("revisions", j(serde_json::to_value(v.revisions())));
    add("images", j(serde_json::to_value(v.images())));
    add("bookmarks", j(serde_json::to_value(v.bookmarks())));
    add("notes", j(serde_json::to_value(v.notes())));
    add("sections", j(serde_json::to_value(v.sections())));
    add("numbering", j(serde_json::to_value(v.numbering())));
    add(
        "content_controls",
        j(serde_json::to_value(v.content_controls())),
    );
    out
}

#[test]
fn every_view_has_the_documented_shape() {
    let root = schema();
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files: Vec<_> = std::fs::read_dir(dir.join("corpus"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "docx"))
        .collect();
    files.push(dir.join("samples/report-template.docx"));
    let mut errors = vec![];
    let mut seen = std::collections::HashSet::new();
    for f in files {
        let mut doc = Document::open(&f).unwrap();
        for (key, v) in views(&mut doc) {
            validate(&root, &root, &v, key, &mut errors);
            // Which kinds of content occur, so that the schema's variants
            // are exercised.
            if key == "blocks" || key == "paragraphs" {
                for b in v[key].as_array().unwrap() {
                    for i in b["content"].as_array().unwrap() {
                        seen.insert(i["type"].as_str().unwrap().to_string());
                    }
                }
            }
        }
        if errors.len() > 20 {
            break;
        }
    }
    assert!(errors.is_empty(), "\n{}", errors.join("\n"));
    for kind in [
        "text",
        "field_start",
        "field_end",
        "hyperlink_start",
        "hyperlink_end",
        "content_control_start",
        "content_control_end",
        "bookmark_start",
        "bookmark_end",
        "comment_start",
        "comment_end",
        "comment_reference",
        "note_reference",
        "drawing",
        "math",
        "break",
        "opaque",
    ] {
        assert!(
            seen.contains(kind),
            "no corpus document has inline content {kind}"
        );
    }
}

#[test]
fn the_validator_finds_errors() {
    let root = schema();
    let mut errors = vec![];
    let bad = json!({"sha256": "x", "bookmarks": [{"name": "a", "id": "1", "hidden": "no", "text": "", "location": {"story": "main", "part": "p", "block": null}, "extra": 1}]});
    validate(&root, &root, &bad, "bookmarks", &mut errors);
    assert_eq!(errors.len(), 2, "{errors:?}");
}
