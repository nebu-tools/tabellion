//! The results of `SEQ` and `REF` fields after an insertion.
//!
//! One pass over the body in document order collects every field with the
//! text nodes of its result, and the text of every label bookmark. Then:
//!
//! - each `SEQ` field gets its number in document order (`\r` resets,
//!   `\c` repeats, `\* ARABIC`, `ALPHABETIC`, `alphabetic`, `ROMAN` and
//!   `roman` format). Only fields from the first inserted node on are
//!   written; earlier ones keep their bytes. A sequence with a `\s` field
//!   (numbered again after each heading of a level) is left alone, which
//!   the fields update will do right;
//! - each `REF` to a label's bookmark inside the insertion, and each `REF`
//!   outside it whose bookmark's text changed with the numbers, gets that
//!   text as its result. The bookmarks followed are the labels and Word's
//!   own hidden cross-reference bookmarks (`_Ref` and digits), whose text
//!   is read with the new numbers of the `SEQ` fields inside them.
//!
//! A field nested in another field's code is part of that code: it counts,
//! and its result is never written.
//!
//! Text boxes, the `mc:Fallback` branch of alternate content and deleted
//! content are not walked: Word numbers a text box as a story of its own,
//! reads the branch it understands, and leaves deleted fields out.

use std::collections::{HashMap, HashSet};

use crate::xml::{NodeId, Tree};
use crate::{Document, Error, ns};

use super::session::Session;

const W: &str = ns::W;
const MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";

struct Field {
    code: String,
    /// The `w:t` of the result, without those of nested fields.
    texts: Vec<NodeId>,
    /// Where a run goes if the result has no text: before the end run of
    /// a complex field, into a simple field.
    end: Option<NodeId>,
    simple: Option<NodeId>,
    /// At or after the first inserted node.
    after: bool,
    /// Inside inserted content.
    inside: bool,
    /// Nested in another field's code, where its result is part of that
    /// code: never written.
    in_code: bool,
    old: String,
}

enum Event {
    Text(String, Option<usize>),
    FieldEnd(usize),
    Open(String, String),
    Close(String),
}

struct Open {
    index: usize,
    in_result: bool,
}

/// Splits a field code into words; quoted words keep their spaces.
fn words(code: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut quoted = false;
    for c in code.chars() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn roman(mut n: u64, upper: bool) -> String {
    let table = [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ];
    let mut s = String::new();
    for (v, r) in table {
        while n >= v {
            s.push_str(r);
            n -= v;
        }
    }
    if upper { s.to_uppercase() } else { s }
}

fn alpha(n: u64, upper: bool) -> String {
    // Word repeats the letter: a … z, aa … zz.
    if n == 0 {
        return String::new();
    }
    let letter = (b'a' + ((n - 1) % 26) as u8) as char;
    let s: String = std::iter::repeat_n(letter, ((n - 1) / 26 + 1) as usize).collect();
    if upper { s.to_uppercase() } else { s }
}

fn format(n: u64, w: &[String]) -> String {
    let fmt = w
        .iter()
        .position(|x| x == "\\*")
        .and_then(|i| w.get(i + 1))
        .map(String::as_str);
    match fmt {
        Some("ALPHABETIC") => alpha(n, true),
        Some("alphabetic") => alpha(n, false),
        Some("ROMAN") => roman(n, true),
        Some("roman") => roman(n, false),
        _ => n.to_string(),
    }
}

/// Collects fields and label texts in one pass.
fn collect(
    t: &Tree,
    body: NodeId,
    inserted: &HashSet<NodeId>,
    labels: &HashSet<String>,
) -> Result<(Vec<Field>, Vec<Event>), Error> {
    let mut fields: Vec<Field> = vec![];
    let mut events = vec![];
    let mut open: Vec<Open> = vec![];
    let mut after = false;
    // Explicit stack: (node, inside inserted content). Children are pushed
    // in reverse so they pop in order.
    let mut stack: Vec<(NodeId, bool)> = vec![];
    let kids: Vec<NodeId> = t.elements(body)?.collect();
    for k in kids.into_iter().rev() {
        stack.push((k, false));
    }
    while let Some((n, inside0)) = stack.pop() {
        let inside = inside0 || inserted.contains(&n);
        if inside {
            after = true;
        }
        let Some(name) = t.name(n)? else { continue };
        let local = name.local;
        if name.namespace == Some(MC) && local == "Fallback" {
            continue;
        }
        if name.namespace == Some(W) {
            match local {
                "txbxContent" | "del" | "moveFrom" | "delInstrText" => continue,
                "instrText" => {
                    if let Some(o) = open.last()
                        && !o.in_result
                    {
                        let code = t.text_content(n)?;
                        fields[o.index].code.push_str(&code);
                    }
                    continue;
                }
                "fldSimple" => {
                    let code = t
                        .attribute(n, Some(W), "instr")?
                        .map(|v| v.into_owned())
                        .unwrap_or_default();
                    let index = fields.len();
                    let mut texts = vec![];
                    let mut old = String::new();
                    for d in t.descendants(n)? {
                        if t.is(d, Some(W), "t")? {
                            old.push_str(&t.text_content(d)?);
                            texts.push(d);
                        }
                    }
                    let in_code = open.last().is_some_and(|o| !o.in_result);
                    fields.push(Field {
                        code,
                        texts,
                        end: None,
                        simple: Some(n),
                        after,
                        inside,
                        in_code,
                        old: old.clone(),
                    });
                    events.push(Event::Text(old, Some(index)));
                    events.push(Event::FieldEnd(index));
                    continue;
                }
                "fldChar" => {
                    match t.attribute(n, Some(W), "fldCharType")?.as_deref() {
                        Some("begin") => {
                            let in_code = open.last().is_some_and(|o| !o.in_result);
                            open.push(Open {
                                index: fields.len(),
                                in_result: false,
                            });
                            fields.push(Field {
                                code: String::new(),
                                texts: vec![],
                                end: None,
                                simple: None,
                                after,
                                inside,
                                in_code,
                                old: String::new(),
                            });
                        }
                        Some("separate") => {
                            if let Some(o) = open.last_mut() {
                                o.in_result = true;
                            }
                        }
                        Some("end") => {
                            if let Some(o) = open.pop() {
                                fields[o.index].end = t.parent(n)?;
                                events.push(Event::FieldEnd(o.index));
                            }
                        }
                        _ => {}
                    }
                    continue;
                }
                "bookmarkStart" => {
                    let nm = t
                        .attribute(n, Some(W), "name")?
                        .map(|v| v.into_owned())
                        .unwrap_or_default();
                    let lower = nm.to_lowercase();
                    if lower.starts_with("_ref") || labels.contains(&lower) {
                        let id = t
                            .attribute(n, Some(W), "id")?
                            .map(|v| v.into_owned())
                            .unwrap_or_default();
                        events.push(Event::Open(id, nm));
                    }
                    continue;
                }
                "bookmarkEnd" => {
                    let id = t
                        .attribute(n, Some(W), "id")?
                        .map(|v| v.into_owned())
                        .unwrap_or_default();
                    events.push(Event::Close(id));
                    continue;
                }
                "t" | "tab" => {
                    let s = if local == "t" {
                        t.text_content(n)?
                    } else {
                        "\t".to_string()
                    };
                    let in_code = open.last().is_some_and(|o| !o.in_result);
                    if in_code {
                        continue;
                    }
                    let field = open.last().map(|o| o.index);
                    if let Some(f) = field
                        && local == "t"
                    {
                        fields[f].texts.push(n);
                        fields[f].old.push_str(&s);
                    }
                    events.push(Event::Text(s, field));
                    continue;
                }
                _ => {}
            }
        }
        let kids: Vec<NodeId> = t.elements(n)?.collect();
        for k in kids.into_iter().rev() {
            stack.push((k, inside));
        }
    }
    Ok((fields, events))
}

/// Updates the results after an insertion and returns how many fields
/// outside it changed.
pub(crate) fn update(
    doc: &mut Document,
    s: &Session<'_>,
    nodes: &[Vec<NodeId>],
) -> Result<usize, Error> {
    let main = s.main.clone();
    let inserted: HashSet<NodeId> = nodes.iter().flatten().copied().collect();
    let labels: HashSet<String> = s
        .labels
        .values()
        .map(|l| l.bookmark.to_lowercase())
        .collect();
    let budget = doc.limits().max_total_size;
    let t = doc.xml(&main)?;
    let Some(body) = t.child(t.root_element(), Some(W), "body")? else {
        return Ok(0);
    };
    if !nodes.iter().flatten().any(|n| t.kind(*n).is_ok()) {
        return Ok(0);
    }
    let (fields, events) = collect(t, body, &inserted, &labels)?;
    // Numbers of SEQ fields.
    let mut counters: HashMap<String, u64> = HashMap::new();
    let mut restart: HashSet<String> = HashSet::new();
    let mut new: Vec<Option<String>> = vec![None; fields.len()];
    for f in &fields {
        let w = words(&f.code);
        if w.first().is_some_and(|k| k.eq_ignore_ascii_case("SEQ"))
            && w.iter().any(|x| x.eq_ignore_ascii_case("\\s"))
            && let Some(id) = w.get(1)
        {
            restart.insert(id.to_lowercase());
        }
    }
    for (i, f) in fields.iter().enumerate() {
        crate::cost::tick(1);
        let w = words(&f.code);
        if !w.first().is_some_and(|k| k.eq_ignore_ascii_case("SEQ")) {
            continue;
        }
        let Some(id) = w.get(1).map(|s| s.to_lowercase()) else {
            continue;
        };
        let c = counters.entry(id.clone()).or_insert(0);
        let reset = w
            .iter()
            .position(|x| x.eq_ignore_ascii_case("\\r"))
            .and_then(|k| w.get(k + 1))
            .and_then(|v| v.parse::<u64>().ok());
        if let Some(r) = reset {
            *c = r;
        } else if !w.iter().any(|x| x.eq_ignore_ascii_case("\\c")) {
            *c += 1;
        }
        let hidden = w.iter().any(|x| x.eq_ignore_ascii_case("\\h"));
        if !restart.contains(&id) && !hidden {
            new[i] = Some(format(*c, &w));
        }
    }
    // Label texts, before and after.
    let mut texts: HashMap<String, (String, String)> = HashMap::new();
    let mut open_marks: Vec<(String, String)> = vec![];
    let mut used = 0u64;
    let seq_value = |i: usize| new[i].clone();
    for e in &events {
        crate::cost::tick(1);
        match e {
            Event::Open(id, name) => open_marks.push((id.clone(), name.clone())),
            Event::Close(id) => {
                if let Some(k) = open_marks.iter().position(|(i, _)| i == id) {
                    open_marks.remove(k);
                }
            }
            Event::Text(s, field) => {
                for (_, name) in &open_marks {
                    let e = texts.entry(name.to_lowercase()).or_default();
                    e.0.push_str(s);
                    if !field.is_some_and(|f| new[f].is_some()) {
                        e.1.push_str(s);
                    }
                    used += 2 * s.len() as u64;
                }
            }
            Event::FieldEnd(f) => {
                if let Some(v) = seq_value(*f) {
                    for (_, name) in &open_marks {
                        texts.entry(name.to_lowercase()).or_default().1.push_str(&v);
                        used += v.len() as u64;
                    }
                }
            }
        }
        if used > budget {
            return Err(Error::Limit {
                what: format!("label bookmarks that hold more than {budget} bytes of text"),
                field: "max_total_size",
                limit: budget,
            });
        }
    }
    let mut changed = 0;
    let mut writes: Vec<(usize, String)> = vec![];
    for (i, f) in fields.iter().enumerate() {
        crate::cost::tick(1);
        if f.in_code {
            continue;
        }
        if let Some(v) = &new[i] {
            if f.after && *v != f.old {
                writes.push((i, v.clone()));
            }
            continue;
        }
        let w = words(&f.code);
        if !w.first().is_some_and(|k| k.eq_ignore_ascii_case("REF")) {
            continue;
        }
        if w.iter().skip(2).any(|x| {
            matches!(
                x.to_ascii_lowercase().as_str(),
                "\\r" | "\\n" | "\\w" | "\\p" | "\\d" | "\\t"
            )
        }) {
            continue;
        }
        let Some(target) = w.get(1).map(|s| s.to_lowercase()) else {
            continue;
        };
        let Some((old, now)) = texts.get(&target) else {
            continue;
        };
        if (f.inside || old != now) && *now != f.old {
            writes.push((i, now.clone()));
        }
    }
    for (i, v) in writes {
        let f = &fields[i];
        if !f.inside {
            changed += 1;
        }
        match f.texts.split_first() {
            Some((first, rest)) => {
                t.set_text(*first, &v)?;
                for r in rest {
                    t.detach(*r)?;
                }
                if (v.starts_with(' ') || v.ends_with(' '))
                    && t.attribute(*first, Some(crate::xml::XML_NS), "space")?
                        .is_none()
                {
                    t.set_attribute(*first, Some(crate::xml::XML_NS), "xml:space", "preserve")?;
                }
            }
            None => {
                let run =
                    t.parse_fragment(&format!("<w:r>{}</w:r>", super::xml::t(&v)), &[("w", W)])?[0];
                match (f.simple, f.end) {
                    (Some(sf), _) => t.append(sf, run)?,
                    (None, Some(end)) => t.insert_before(end, run)?,
                    _ => {}
                }
            }
        }
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_take_the_format_the_code_asks_for() {
        let w = |s: &str| words(s);
        assert_eq!(format(4, &w("SEQ Table \\* ARABIC")), "4");
        assert_eq!(format(4, &w("SEQ Table \\* roman")), "iv");
        assert_eq!(format(28, &w("SEQ Table \\* ALPHABETIC")), "BB");
        assert_eq!(words("REF \"a b\" \\h"), vec!["REF", "a b", "\\h"]);
    }
}
