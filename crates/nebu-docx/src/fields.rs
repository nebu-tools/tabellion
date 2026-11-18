//! Field results without Word: what Word would store after "update fields"
//! for SEQ, REF and NOTEREF. Page numbers (PAGE, PAGEREF, NUMPAGES) and
//! the page column of a table of contents need layout and stay as they are
//! until Word updates them.

use std::collections::HashMap;

use crate::document::Document;
use crate::xml::{NodeId, Tree};
use crate::Error;

/// One complex field at paragraph level: its code and the `w:t` nodes of its
/// result.
struct Field {
    code: String,
    result: Vec<NodeId>,
    /// The run that holds `separate`, so a result can be added if missing.
    separate_run: Option<NodeId>,
}

fn fields_in(t: &Tree, p: NodeId) -> Vec<Field> {
    let mut out = vec![];
    // Stack of open fields: (code, in_result, result nodes, separate run).
    let mut stack: Vec<(String, bool, Vec<NodeId>, Option<NodeId>)> = vec![];
    for d in t.descendants(p) {
        match t.name(d) {
            "w:fldSimple" if stack.is_empty() => {
                let result: Vec<NodeId> = t.descendants(d).filter(|&x| t.is(x, "w:t")).collect();
                out.push(Field { code: t.attr(d, "w:instr").unwrap_or_default(), result, separate_run: None });
            }
            "w:fldChar" => match t.attr(d, "w:fldCharType").as_deref() {
                Some("begin") => stack.push((String::new(), false, vec![], None)),
                Some("separate") => {
                    if let Some(top) = stack.last_mut() {
                        top.1 = true;
                        top.3 = t.parent(d);
                    }
                }
                Some("end") => {
                    if let Some((code, _, result, sep)) = stack.pop() {
                        out.push(Field { code, result, separate_run: sep });
                    }
                }
                _ => {}
            },
            "w:instrText" => {
                if let Some(top) = stack.last_mut() {
                    if !top.1 {
                        top.0.push_str(&t.inner_text(d));
                    }
                }
            }
            "w:t" | "w:noBreakHyphen" if stack.len() == 1 && stack[0].1 => {
                if !t.ancestors(d).any(|a| t.is(a, "w:del")) {
                    stack[0].2.push(d);
                }
            }
            _ => {}
        }
    }
    out
}

fn result_text(t: &Tree, nodes: &[NodeId]) -> String {
    nodes.iter().map(|&n| if t.is(n, "w:noBreakHyphen") { "\u{2011}".to_string() } else { t.inner_text(n) }).collect()
}

fn set_result(t: &mut Tree, f: &Field, value: &str) -> bool {
    if result_text(t, &f.result) == value {
        return false;
    }
    let texts: Vec<NodeId> = f.result.iter().copied().filter(|&n| t.is(n, "w:t")).collect();
    let hyphens: Vec<NodeId> = f.result.iter().copied().filter(|&n| t.is(n, "w:noBreakHyphen")).collect();
    for n in hyphens {
        t.detach(n);
    }
    match texts.first() {
        Some(&first) => {
            t.set_text(first, value);
            for &n in &texts[1..] {
                t.set_text(n, "");
            }
        }
        None => {
            let Some(sep) = f.separate_run else { return false };
            let r = t.new_element("w:r", &[]);
            let w = t.new_element("w:t", &[("xml:space", "preserve")]);
            let tx = t.new_text(value);
            t.append(w, tx);
            t.append(r, w);
            t.insert_after(sep, r);
        }
    }
    true
}

/// Recomputes SEQ, REF and NOTEREF results in the body. Returns how many
/// results changed.
pub fn update(doc: &mut Document) -> Result<usize, Error> {
    let body = doc.body();
    let styles = doc.styles()?.clone();
    let t = doc.tree();
    let paras: Vec<NodeId> = t.descendants(body).filter(|&d| t.is(d, "w:p")).collect();
    let mut changed = 0;
    let mut seq: HashMap<String, u32> = HashMap::new();
    let mut headings = [0u64; 10];
    for &p in &paras {
        // Word counts only its built-in heading styles here, not every style
        // with an outline level.
        let builtin = crate::document::paragraph_style(t, p)
            .and_then(|s| styles.get(&s).map(|x| x.name.to_lowercase()))
            .and_then(|n| n.strip_prefix("heading ").and_then(|l| l.parse::<u8>().ok()));
        if let Some(l) = builtin {
            for h in headings.iter_mut().skip(l as usize) {
                *h += 1;
            }
        }
        for f in fields_in(t, p) {
            let mut words = f.code.split_whitespace();
            if words.next() != Some("SEQ") {
                continue;
            }
            let Some(id) = words.next() else { continue };
            let rest: Vec<&str> = words.collect();
            // `\s N` (restart after headings) is not applied: Word's own
            // results on the templates tried show no restart where the
            // heading is not numbered, and the rule is not pinned down yet.
            let n = seq.entry(id.to_string()).or_insert(0);
            if let Some(i) = rest.iter().position(|w| *w == "\\r") {
                if let Some(v) = rest.get(i + 1).and_then(|v| v.parse().ok()) {
                    *n = v;
                }
            } else if !rest.contains(&"\\c") {
                *n += 1;
            }
            if !rest.contains(&"\\h") && set_result(t, &f, &n.to_string()) {
                changed += 1;
            }
        }
    }
    // Bookmark text after the SEQ update; bookmarks within one paragraph.
    let mut marks: HashMap<String, (String, Option<String>)> = HashMap::new();
    let mut footnote_order: Vec<String> = vec![];
    for &p in &paras {
        let mut open: Vec<(String, String, String, Option<String>)> = vec![];
        let mut in_code = 0usize;
        let mut in_result = false;
        for d in t.descendants(p) {
            match t.name(d) {
                "w:bookmarkStart" => {
                    if let (Some(id), Some(name)) = (t.attr(d, "w:id"), t.attr(d, "w:name")) {
                        open.push((id, name, String::new(), None));
                    }
                }
                "w:bookmarkEnd" => {
                    if let Some(id) = t.attr(d, "w:id") {
                        if let Some(i) = open.iter().position(|o| o.0 == id) {
                            let (_, name, text, fn_id) = open.remove(i);
                            marks.insert(name, (text, fn_id));
                        }
                    }
                }
                "w:fldChar" => match t.attr(d, "w:fldCharType").as_deref() {
                    Some("begin") => {
                        in_code += 1;
                        in_result = false;
                    }
                    Some("separate") => in_result = true,
                    Some("end") => {
                        in_code = in_code.saturating_sub(1);
                        in_result = in_code > 0;
                    }
                    _ => {}
                },
                "w:t" | "w:noBreakHyphen" | "w:tab" if (in_code == 0 || in_result) && !t.ancestors(d).any(|a| t.is(a, "w:del")) => {
                    let s = match t.name(d) {
                        "w:noBreakHyphen" => "\u{2011}".to_string(),
                        "w:tab" if t.parent(d).is_some_and(|p| t.is(p, "w:r")) => "\t".to_string(),
                        "w:tab" => continue,
                        _ => t.inner_text(d),
                    };
                    for o in open.iter_mut() {
                        o.2.push_str(&s);
                    }
                }
                "w:footnoteReference" => {
                    if let Some(id) = t.attr(d, "w:id") {
                        footnote_order.push(id.clone());
                        for o in open.iter_mut() {
                            o.3.get_or_insert(id.clone());
                        }
                    }
                }
                _ => {}
            }
        }
        for (_, name, text, fn_id) in open {
            marks.insert(name, (text, fn_id));
        }
    }
    for &p in &paras {
        for f in fields_in(t, p) {
            let mut words = f.code.split_whitespace();
            let kind = words.next().unwrap_or("");
            let name = words.next().unwrap_or("");
            // Paragraph-number forms need the numbering engine; left for Word
            // in this version rather than written wrong.
            let switches: Vec<&str> = words.collect();
            if kind == "REF" && switches.iter().any(|w| matches!(*w, "\\r" | "\\n" | "\\w")) {
                continue;
            }
            let value = match (kind, marks.get(name)) {
                ("REF", Some((text, _))) => text.trim().to_string(),
                ("NOTEREF", Some((_, Some(fid)))) => {
                    match footnote_order.iter().position(|x| x == fid) {
                        Some(i) => (i + 1).to_string(),
                        None => continue,
                    }
                }
                _ => continue,
            };
            if set_result(t, &f, &value) {
                changed += 1;
            }
        }
    }
    Ok(changed)
}

/// Code and result of every complex field in the body, in document order.
pub fn list(doc: &mut Document) -> Vec<(String, String)> {
    let body = doc.body();
    let t = doc.tree();
    let paras: Vec<NodeId> = t.descendants(body).filter(|&d| t.is(d, "w:p")).collect();
    let mut v = vec![];
    for p in paras {
        for f in fields_in(t, p) {
            v.push((f.code.trim().to_string(), result_text(t, &f.result)));
        }
    }
    v
}
