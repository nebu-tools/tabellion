//! Typst values as JSON, for definitions written in Typst (table styles,
//! data passed back to R). Lengths become points, colours hex strings,
//! strokes `{width, color}`, and keys lose their dashes.

use typst::foundations::Repr as _;
use serde_json::{json, Map, Value as J};
use typst::foundations::Value;
use typst::visualize::Paint;

pub fn color_hex(c: &typst::visualize::Color) -> String {
    c.to_hex().trim_start_matches('#').chars().take(6).collect::<String>().to_uppercase()
}

pub fn paint_hex(p: &Paint) -> Option<String> {
    match p {
        Paint::Solid(c) => Some(color_hex(c)),
        _ => None,
    }
}

pub fn to_json(v: &Value) -> J {
    match v {
        Value::None | Value::Auto => J::Null,
        Value::Bool(b) => J::Bool(*b),
        Value::Int(i) => json!(i),
        Value::Float(f) => json!(f),
        Value::Length(l) => json!(l.abs.to_pt()),
        Value::Ratio(r) => json!(r.get()),
        Value::Fraction(f) => json!(f.get()),
        Value::Color(c) => J::String(color_hex(c)),
        Value::Str(s) => J::String(s.to_string()),
        Value::Array(a) => J::Array(a.iter().map(to_json).collect()),
        Value::Dict(d) => {
            let mut m = Map::new();
            for (k, v) in d.iter() {
                m.insert(k.as_str().replace('-', "_"), to_json(v));
            }
            J::Object(m)
        }
        Value::Dyn(d) => {
            if let Some(s) = d.downcast::<typst::visualize::Stroke>() {
                let width = match s.thickness {
                    typst::foundations::Smart::Custom(t) => t.abs.to_pt(),
                    _ => 1.0,
                };
                let color = match &s.paint {
                    typst::foundations::Smart::Custom(p) => paint_hex(p),
                    _ => None,
                };
                return json!({ "width": width, "color": color.unwrap_or_else(|| "000000".into()) });
            }
            J::String(v.repr().to_string())
        }
        other => J::String(other.repr().to_string()),
    }
}
