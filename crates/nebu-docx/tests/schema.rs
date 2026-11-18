//! Every view of every corpus document has the JSON shape that
//! `schema/views.json` gives, and the schema names every field the views
//! write.
//!
//! The validator below knows the part of JSON Schema the file uses: types,
//! `enum`, `const`, `properties`, `required`, `additionalProperties: false`,
//! `items`, `$ref`, `oneOf`, `minimum`, `minProperties` and
//! `maxProperties`.

use std::path::Path;

use nebu_docx::Document;
use serde_json::{Map, Value, json};

fn schema() -> Value {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("schema/views.json");
    serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap()
}

fn type_ok(v: &Value, t: &str) -> bool {
    match t {
        "object" => v.is_object(),
        "array" => v.is_array(),
        "string" => v.is_string(),
        "integer" => v.is_i64() || v.is_u64(),
        "number" => v.is_number(),
        "boolean" => v.is_boolean(),
        "null" => v.is_null(),
        _ => panic!("the schema uses the type {t}"),
    }
}

fn validate(root: &Value, s: &Value, v: &Value, path: &str, errors: &mut Vec<String>) {
    let Some(s) = s.as_object() else { return };
    for k in s.keys() {
        let known = [
            "$schema",
            "$id",
            "$defs",
            "title",
            "description",
            "type",
            "enum",
            "const",
            "properties",
            "required",
            "additionalProperties",
            "items",
            "$ref",
            "oneOf",
            "minimum",
            "minProperties",
            "maxProperties",
        ];
        assert!(
            known.contains(&k.as_str()),
            "the schema uses {k}, which this test does not check"
        );
    }
    if let Some(r) = s.get("$ref").and_then(Value::as_str) {
        let name = r.strip_prefix("#/$defs/").expect("a local reference");
        validate(root, &root["$defs"][name], v, path, errors);
    }
    if let Some(t) = s.get("type") {
        let ok = match t {
            Value::String(t) => type_ok(v, t),
            Value::Array(ts) => ts.iter().any(|t| type_ok(v, t.as_str().unwrap())),
            _ => false,
        };
        if !ok {
            errors.push(format!("{path}: {v} is not of type {t}"));
            return;
        }
    }
    if let Some(e) = s.get("enum").and_then(Value::as_array)
        && !e.contains(v)
    {
        errors.push(format!("{path}: {v} is not one of {e:?}"));
    }
    if let Some(c) = s.get("const")
        && c != v
    {
        errors.push(format!("{path}: {v} is not {c}"));
    }
    if let (Some(m), Some(n)) = (s.get("minimum").and_then(Value::as_f64), v.as_f64())
        && n < m
    {
        errors.push(format!("{path}: {n} is less than {m}"));
    }
    if let Some(o) = v.as_object() {
        if let Some(n) = s.get("minProperties").and_then(Value::as_u64)
            && (o.len() as u64) < n
        {
            errors.push(format!("{path}: fewer than {n} keys"));
        }
        if let Some(n) = s.get("maxProperties").and_then(Value::as_u64)
            && (o.len() as u64) > n
        {
            errors.push(format!("{path}: more than {n} keys"));
        }
        let props = s.get("properties").and_then(Value::as_object);
        for r in s
            .get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let r = r.as_str().unwrap();
            if !o.contains_key(r) {
                errors.push(format!("{path}: no {r}"));
            }
        }
        for (k, x) in o {
            match props.and_then(|p| p.get(k)) {
                Some(ps) => validate(root, ps, x, &format!("{path}.{k}"), errors),
                None if s.get("additionalProperties") == Some(&Value::Bool(false)) => {
                    errors.push(format!("{path}: {k} is not in the schema"));
                }
                None => {}
            }
        }
    }
    if let (Some(items), Some(a)) = (s.get("items"), v.as_array()) {
        for (i, x) in a.iter().enumerate() {
            validate(root, items, x, &format!("{path}[{i}]"), errors);
        }
    }
    if let Some(alts) = s.get("oneOf").and_then(Value::as_array) {
        let matching: Vec<Vec<String>> = alts
            .iter()
            .map(|a| {
                let mut e = vec![];
                validate(root, a, v, path, &mut e);
                e
            })
            .collect();
        let ok = matching.iter().filter(|e| e.is_empty()).count();
        if ok != 1 {
            let best = matching
                .iter()
                .min_by_key(|e| e.len())
                .cloned()
                .unwrap_or_default();
            errors.push(format!(
                "{path}: matches {ok} of oneOf, not 1; nearest: {best:?}"
            ));
        }
    }
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
    add(
        "blocks",
        serde_json::to_value(doc.blocks().unwrap()).unwrap(),
    );
    let styles: Vec<_> = doc.styles().unwrap().iter().cloned().collect();
    add("styles", serde_json::to_value(styles).unwrap());
    add(
        "tables",
        serde_json::to_value(doc.tables().unwrap()).unwrap(),
    );
    add(
        "fields",
        serde_json::to_value(doc.fields().unwrap()).unwrap(),
    );
    add(
        "comments",
        serde_json::to_value(doc.comments().unwrap()).unwrap(),
    );
    add(
        "revisions",
        serde_json::to_value(doc.revisions().unwrap()).unwrap(),
    );
    add(
        "images",
        serde_json::to_value(doc.images().unwrap()).unwrap(),
    );
    add(
        "bookmarks",
        serde_json::to_value(doc.bookmarks().unwrap()).unwrap(),
    );
    add("notes", serde_json::to_value(doc.notes().unwrap()).unwrap());
    add(
        "sections",
        serde_json::to_value(doc.sections().unwrap()).unwrap(),
    );
    add(
        "numbering",
        serde_json::to_value(doc.numbering().unwrap()).unwrap(),
    );
    add(
        "content_controls",
        serde_json::to_value(doc.content_controls().unwrap()).unwrap(),
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
            if key == "blocks" {
                for b in v["blocks"].as_array().unwrap() {
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
