//! Typst math -> Office Math.

use typst::foundations::Repr as _;
use nebu_docx::content::Math;
use typst::foundations::{Content, StyledElem, Value};

fn field(c: &Content, name: &str) -> Option<Value> {
    c.get_by_name(name).ok()
}

fn content(c: &Content, name: &str) -> Option<Content> {
    match field(c, name)? {
        Value::Content(x) => Some(x),
        Value::None => None,
        Value::Str(s) => Some(typst::text::TextElem::packed(s.as_str())),
        other => Some(typst::text::TextElem::packed(other.repr())),
    }
}

const NARY: &[&str] = &["∑", "∏", "∐", "∫", "∬", "∭", "∮", "⋃", "⋂", "⋁", "⋀"];

pub fn convert(c: &Content, warn: &mut Vec<String>) -> Math {
    let name = c.elem().name();
    let rec = |x: &Content, w: &mut Vec<String>| Box::new(convert(x, w));
    match name {
        "sequence" => {
            let kids: Vec<Content> = match field(c, "children") {
                Some(Value::Array(a)) => a.into_iter().filter_map(|v| v.cast::<Content>().ok()).collect(),
                _ => vec![],
            };
            let mut items = vec![];
            let mut i = 0;
            while i < kids.len() {
                let m = convert(&kids[i], warn);
                i += 1;
                // A big operator with limits takes the next item as its body.
                let nary = match &m {
                    Math::Scripts { base, sub, sup } => match base.as_ref() {
                        Math::Run { text, .. } if NARY.contains(&text.as_str()) => Some((text.clone(), sub.clone(), sup.clone())),
                        _ => None,
                    },
                    Math::Run { text, .. } if NARY.contains(&text.as_str()) => Some((text.clone(), None, None)),
                    _ => None,
                };
                if let Some((op, sub, sup)) = nary {
                    while i < kids.len() && kids[i].elem().name() == "space" {
                        i += 1;
                    }
                    let body = if i < kids.len() {
                        i += 1;
                        convert(&kids[i - 1], warn)
                    } else {
                        Math::Row { items: vec![] }
                    };
                    items.push(Math::Nary { op, sub, sup, body: Box::new(body) });
                    continue;
                }
                if !matches!(&m, Math::Row { items } if items.is_empty()) {
                    items.push(m);
                }
            }
            if items.len() == 1 { items.pop().unwrap() } else { Math::Row { items } }
        }
        "text" | "symbol" => match field(c, "text") {
            Some(Value::Str(s)) => Math::Run { text: s.to_string(), normal: false },
            Some(v) => Math::Run { text: v.repr().to_string(), normal: false },
            None => Math::Row { items: vec![] },
        },
        "space" | "h" | "align-point" | "alignpoint" => Math::Row { items: vec![] },
        "styled" => match c.to_packed::<StyledElem>() {
            Some(s) => convert(&s.child, warn),
            None => Math::Row { items: vec![] },
        },
        "attach" => {
            let base = content(c, "base").unwrap_or_default();
            let sub = content(c, "b").or_else(|| content(c, "br")).map(|x| rec(&x, warn));
            let sup = content(c, "t").or_else(|| content(c, "tr")).map(|x| rec(&x, warn));
            if content(c, "tl").is_some() || content(c, "bl").is_some() {
                warn.push("left scripts are placed on the right in Word".into());
            }
            Math::Scripts { base: rec(&base, warn), sub, sup }
        }
        "limits" | "scripts" | "class" | "math.class" | "lr" | "mid" | "op" | "primes" | "accent" | "frac" | "root" | "binom" | "mat" | "vec" | "cases" => {
            match name {
                "frac" => {
                    let num = content(c, "num").unwrap_or_default();
                    let den = content(c, "denom").unwrap_or_default();
                    Math::Frac { num: rec(&num, warn), den: rec(&den, warn) }
                }
                "binom" => {
                    let up = content(c, "upper").unwrap_or_default();
                    warn.push("binomial written as a bracketed fraction".into());
                    Math::Delim {
                        open: "(".into(),
                        close: ")".into(),
                        body: Box::new(Math::Frac { num: rec(&up, warn), den: Box::new(Math::Run { text: "…".into(), normal: false }) }),
                    }
                }
                "root" => {
                    let index = content(c, "index").map(|x| rec(&x, warn));
                    let radicand = content(c, "radicand").unwrap_or_default();
                    Math::Root { index, radicand: rec(&radicand, warn) }
                }
                "lr" => {
                    let body = content(c, "body").unwrap_or_default();
                    let inner = convert(&body, warn);
                    // lr(...) keeps its delimiters as the first and last items.
                    if let Math::Row { mut items } = inner {
                        let is_delim = |m: &Math| matches!(m, Math::Run { text, .. } if text.chars().count() == 1 && "([{|⟨⌈⌊‖)]}⟩⌉⌋".contains(text.as_str()));
                        if items.len() >= 2 && is_delim(&items[0]) && is_delim(items.last().unwrap()) {
                            let close = match items.pop() { Some(Math::Run { text, .. }) => text, _ => String::new() };
                            let open = match items.remove(0) { Math::Run { text, .. } => text, _ => String::new() };
                            return Math::Delim { open, close, body: Box::new(Math::Row { items }) };
                        }
                        return Math::Row { items };
                    }
                    inner
                }
                "op" => {
                    let t = content(c, "text").unwrap_or_default();
                    Math::Run { text: t.plain_text().to_string(), normal: true }
                }
                "primes" => {
                    let n = match field(c, "count") { Some(Value::Int(n)) => n.max(1) as usize, _ => 1 };
                    Math::Run { text: "′".repeat(n), normal: false }
                }
                "accent" => {
                    let base = content(c, "base").unwrap_or_default();
                    let acc = c
                        .to_packed::<typst::math::AccentElem>()
                        .map(|a| a.accent.0.to_string())
                        .unwrap_or_else(|| "\u{302}".into());
                    Math::Accent { accent: acc, base: rec(&base, warn) }
                }
                "mat" | "vec" | "cases" => {
                    let rows: Vec<Vec<Math>> = match name {
                        "mat" => match field(c, "rows") {
                            Some(Value::Array(rs)) => rs
                                .into_iter()
                                .map(|r| match r {
                                    Value::Array(cells) => cells.into_iter().filter_map(|v| v.cast::<Content>().ok()).map(|x| convert(&x, warn)).collect(),
                                    _ => vec![],
                                })
                                .collect(),
                            _ => vec![],
                        },
                        _ => match field(c, "children") {
                            Some(Value::Array(a)) => a.into_iter().filter_map(|v| v.cast::<Content>().ok()).map(|x| vec![convert(&x, warn)]).collect(),
                            _ => vec![],
                        },
                    };
                    let m = Math::Matrix { rows };
                    let (o, cl) = if name == "cases" { ("{", "") } else { ("(", ")") };
                    Math::Delim { open: o.into(), close: cl.into(), body: Box::new(m) }
                }
                _ => {
                    let body = content(c, "body").unwrap_or_default();
                    convert(&body, warn)
                }
            }
        }
        "linebreak" => {
            warn.push("line break inside an equation dropped; Word breaks equations itself".into());
            Math::Row { items: vec![] }
        }
        other => {
            warn.push(format!("math element {other} written as its text"));
            Math::Run { text: c.plain_text().to_string(), normal: false }
        }
    }
}
