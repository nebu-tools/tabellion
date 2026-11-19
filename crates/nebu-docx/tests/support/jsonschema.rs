//! A validator for the part of JSON Schema that `schema/views.json` and
//! `schema/content.json` use: types, `enum`, `const`, `properties`,
//! `required`, `additionalProperties: false`, `items`, `$ref`, `oneOf`,
//! `minimum`, `maximum`, `exclusiveMinimum`, `minProperties` and
//! `maxProperties`. A keyword it does not know fails the test.

#![allow(dead_code)]

use serde_json::Value;

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

pub fn validate(root: &Value, s: &Value, v: &Value, path: &str, errors: &mut Vec<String>) {
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
            "maximum",
            "exclusiveMinimum",
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
    if let (Some(m), Some(n)) = (s.get("maximum").and_then(Value::as_f64), v.as_f64())
        && n > m
    {
        errors.push(format!("{path}: {n} is more than {m}"));
    }
    if let (Some(m), Some(n)) = (
        s.get("exclusiveMinimum").and_then(Value::as_f64),
        v.as_f64(),
    ) && n <= m
    {
        errors.push(format!("{path}: {n} is not more than {m}"));
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

/// Every property name the schema defines, with the definition it is in.
pub fn properties(root: &Value) -> Vec<(String, String)> {
    let mut out = vec![];
    let mut walk = vec![("top".to_string(), root)];
    while let Some((def, s)) = walk.pop() {
        if let Some(p) = s.get("properties").and_then(Value::as_object) {
            for (k, v) in p {
                out.push((def.clone(), k.clone()));
                walk.push((def.clone(), v));
            }
        }
        for k in ["oneOf"] {
            if let Some(a) = s.get(k).and_then(Value::as_array) {
                for x in a {
                    walk.push((def.clone(), x));
                }
            }
        }
        if let Some(i) = s.get("items") {
            walk.push((def.clone(), i));
        }
        if let Some(d) = s.get("$defs").and_then(Value::as_object) {
            for (k, v) in d {
                walk.push((k.clone(), v));
            }
        }
    }
    out
}
