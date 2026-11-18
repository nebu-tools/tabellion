//! Tracked changes: replacing text inside a paragraph as a deletion plus an
//! insertion, and accepting or rejecting revisions.
//!
//! Every edit first checks the target for revisions by other authors and
//! refuses to touch it if it finds one. Their pending work is theirs to accept
//! or reject.

use crate::document::{Document, Revision};
use crate::xml::{NodeId, Tree};
use crate::Error;

pub const REVISION_ELEMENTS: &[&str] = &[
    "w:ins", "w:del", "w:moveFrom", "w:moveTo", "w:rPrChange", "w:pPrChange", "w:tcPrChange", "w:trPrChange",
    "w:tblPrChange", "w:sectPrChange", "w:cellIns", "w:cellDel", "w:cellMerge", "w:numberingChange",
];

/// Authors of pending revisions below `node`.
pub fn revision_authors(t: &Tree, node: NodeId) -> Vec<String> {
    let mut v: Vec<String> = t
        .descendants(node)
        .filter(|&d| REVISION_ELEMENTS.contains(&t.name(d)))
        .filter_map(|d| t.attr(d, "w:author"))
        .collect();
    v.sort();
    v.dedup();
    v
}

pub fn check_foreign(t: &Tree, node: NodeId, author: &str) -> Result<(), Error> {
    let others: Vec<String> = revision_authors(t, node).into_iter().filter(|a| a != author).collect();
    if others.is_empty() {
        Ok(())
    } else {
        Err(Error::ForeignRevision { authors: others })
    }
}

/// One character position of a paragraph's visible text.
#[derive(Clone, Copy)]
struct Slot {
    run: NodeId,
    child: NodeId,
    /// Character offset inside a `w:t`, 0 for single-character elements.
    offset: usize,
}

fn run_slots(t: &Tree, p: NodeId) -> (String, Vec<Slot>, Vec<NodeId>) {
    let mut text = String::new();
    let mut slots = vec![];
    let mut atoms = vec![];
    let mut runs = vec![];
    collect_runs(t, p, &mut runs);
    for r in runs {
        for &c in t.children(r) {
            match t.name(c) {
                "w:rPr" | "" => {}
                "w:t" => {
                    for (i, ch) in t.inner_text(c).chars().enumerate() {
                        text.push(ch);
                        slots.push(Slot { run: r, child: c, offset: i });
                    }
                }
                "w:tab" => {
                    text.push('\t');
                    slots.push(Slot { run: r, child: c, offset: 0 });
                }
                "w:noBreakHyphen" => {
                    text.push('\u{2011}');
                    slots.push(Slot { run: r, child: c, offset: 0 });
                }
                "w:softHyphen" => {
                    text.push('\u{00AD}');
                    slots.push(Slot { run: r, child: c, offset: 0 });
                }
                "w:lastRenderedPageBreak" => {}
                _ => {
                    // Fields, drawings, footnote references: never split, never deleted here.
                    text.push('\u{FFFC}');
                    slots.push(Slot { run: r, child: c, offset: 0 });
                    atoms.push(c);
                }
            }
        }
    }
    (text, slots, atoms)
}

/// Live runs in reading order: direct runs and runs in hyperlinks, smart tags
/// and own insertions. Deleted runs are not text.
fn collect_runs(t: &Tree, n: NodeId, out: &mut Vec<NodeId>) {
    for &c in t.children(n) {
        match t.name(c) {
            "w:r" => out.push(c),
            "w:hyperlink" | "w:smartTag" | "w:ins" | "w:customXml" | "w:fldSimple" => collect_runs(t, c, out),
            _ => {}
        }
    }
}

/// Splits `run` so that slot `at` starts a new run. Returns the new run.
fn split_run(t: &mut Tree, at: Slot) -> NodeId {
    let run = at.run;
    let copy = t.deep_clone(run);
    let orig: Vec<NodeId> = t.children(run).to_vec();
    let cloned: Vec<NodeId> = t.children(copy).to_vec();
    let pos = orig.iter().position(|&c| c == at.child).unwrap();
    // Original keeps everything before the split, the copy everything after.
    for (i, (&o, &c)) in orig.iter().zip(&cloned).enumerate() {
        if t.is(o, "w:rPr") {
            continue;
        }
        if i < pos {
            t.detach(c);
        } else if i > pos {
            t.detach(o);
        } else if at.offset == 0 {
            t.detach(o);
        } else {
            let s: Vec<char> = t.inner_text(o).chars().collect();
            let head: String = s[..at.offset].iter().collect();
            let tail: String = s[at.offset..].iter().collect();
            t.set_text(o, &head);
            t.set_attr(o, "xml:space", "preserve");
            t.set_text(c, &tail);
            t.set_attr(c, "xml:space", "preserve");
        }
    }
    t.insert_after(run, copy);
    copy
}

/// Replaces the first occurrence of `find` in paragraph `p` with `replace`,
/// tracked when `rev` is given. Returns false if `find` does not occur.
pub fn replace_in_paragraph(
    doc: &mut Document,
    p: NodeId,
    find: &str,
    replace: &str,
    rev: Option<&Revision>,
) -> Result<bool, Error> {
    if let Some(r) = rev {
        check_foreign(doc.tree(), p, &r.author)?;
    }
    let (text, slots, atoms) = run_slots(doc.tree(), p);
    let Some(byte) = text.find(find) else { return Ok(false) };
    let start = text[..byte].chars().count();
    let len = find.chars().count();
    if len == 0 {
        return Err(Error::Invalid("the text to replace is empty".into()));
    }
    let end = start + len;
    if slots[start..end].iter().any(|s| atoms.contains(&s.child)) {
        return Err(Error::Invalid(
            "the text to replace contains a field, drawing or note reference; target plain text only".into(),
        ));
    }
    let (del_id, ins_id) = {
        let ids = doc.ids()?;
        (ids.next(), ids.next())
    };
    let t = doc.tree();
    // Split at the end first so that the start slot stays valid.
    if end < slots.len() && (slots[end].run == slots[end - 1].run) {
        split_run(t, slots[end]);
    }
    let first_run = if slots[start].offset > 0 || (start > 0 && slots[start - 1].run == slots[start].run) {
        split_run(t, slots[start])
    } else {
        slots[start].run
    };
    // The covered runs are now whole runs from first_run up to the run of the
    // last covered character (which may have been split off as well).
    let mut covered = vec![];
    let mut cur = Some(first_run);
    let last_text: String = find.chars().collect();
    let mut seen = String::new();
    while let Some(r) = cur {
        covered.push(r);
        seen.push_str(&run_text(t, r));
        if seen.chars().count() >= last_text.chars().count() {
            break;
        }
        cur = next_run(t, r);
    }
    let rpr = t.child(first_run, "w:rPr").map(|n| t.node_xml(n)).unwrap_or_default();
    let new_run = format!(
        "<w:r>{rpr}<w:t xml:space=\"preserve\">{}</w:t></w:r>",
        crate::xml::esc(replace)
    );
    match rev {
        None => {
            let ins = t.parse_fragment(&new_run).map_err(|e| Error::Xml("fragment".into(), e))?;
            t.insert_before(covered[0], ins[0]);
            for r in covered {
                t.detach(r);
            }
        }
        Some(r) => {
            let del = t.new_element(
                "w:del",
                &[("w:id", &del_id.to_string()), ("w:author", &r.author), ("w:date", &r.date)],
            );
            t.insert_before(covered[0], del);
            for &c in &covered {
                t.append(del, c);
                let kids: Vec<NodeId> = t.descendants(c).collect();
                for k in kids {
                    match t.name(k) {
                        "w:t" => t.rename(k, "w:delText"),
                        "w:instrText" => t.rename(k, "w:delInstrText"),
                        _ => {}
                    }
                }
            }
            if !replace.is_empty() {
                let frag = format!(
                    "<w:ins w:id=\"{ins_id}\" w:author=\"{}\" w:date=\"{}\">{new_run}</w:ins>",
                    crate::xml::esc_attr(&r.author),
                    r.date
                );
                let ins = t.parse_fragment(&frag).map_err(|e| Error::Xml("fragment".into(), e))?;
                t.insert_after(del, ins[0]);
            }
        }
    }
    Ok(true)
}

fn run_text(t: &Tree, r: NodeId) -> String {
    let mut s = String::new();
    for &c in t.children(r) {
        match t.name(c) {
            "w:t" => s.push_str(&t.inner_text(c)),
            "w:rPr" | "w:lastRenderedPageBreak" | "" => {}
            _ => s.push('\u{FFFC}'),
        }
    }
    s
}

fn next_run(t: &Tree, r: NodeId) -> Option<NodeId> {
    let p = t.parent(r)?;
    let kids = t.children(p);
    let i = kids.iter().position(|&c| c == r)?;
    kids[i + 1..].iter().copied().find(|&c| t.is(c, "w:r"))
}

/// Accepts (`accept = true`) or rejects every revision below `node`, or only
/// those of `author`.
pub fn settle(t: &mut Tree, node: NodeId, accept: bool, author: Option<&str>) -> usize {
    let targets: Vec<NodeId> = t
        .descendants(node)
        .filter(|&d| matches!(t.name(d), "w:ins" | "w:del"))
        .filter(|&d| author.is_none_or(|a| t.attr(d, "w:author").as_deref() == Some(a)))
        .collect();
    let mut n = 0;
    for d in targets {
        let in_props = t.parent(d).is_some_and(|p| matches!(t.name(p), "w:rPr" | "w:trPr"));
        let keep_content = (t.is(d, "w:ins") && accept) || (t.is(d, "w:del") && !accept);
        if in_props {
            // A paragraph mark or row marked as inserted or deleted. Removing the
            // whole paragraph or row on a matching decision is left to Word in
            // this version; the mark itself goes.
            t.detach(d);
        } else if keep_content {
            let kids: Vec<NodeId> = t.children(d).to_vec();
            for k in kids {
                for x in t.descendants(k).collect::<Vec<_>>() {
                    match t.name(x) {
                        "w:delText" => t.rename(x, "w:t"),
                        "w:delInstrText" => t.rename(x, "w:instrText"),
                        _ => {}
                    }
                }
                t.insert_before(d, k);
            }
            t.detach(d);
        } else {
            t.detach(d);
        }
        n += 1;
    }
    n
}

/// Marks a whole block (paragraph or table) as deleted by `rev`.
pub fn mark_deleted(doc: &mut Document, node: NodeId, rev: &Revision) -> Result<(), Error> {
    check_foreign(doc.tree(), node, &rev.author)?;
    let t = doc.tree();
    let mut runs = vec![];
    let mut paras = vec![];
    let mut rows = vec![];
    let all: Vec<NodeId> = std::iter::once(node).chain(t.descendants(node)).collect();
    for d in all {
        match t.name(d) {
            "w:r" if !t.ancestors(d).any(|a| t.is(a, "w:del")) => runs.push(d),
            "w:p" => paras.push(d),
            "w:tr" => rows.push(d),
            _ => {}
        }
    }
    let n = runs.len() + paras.len() + rows.len();
    let first = doc.ids()?.next();
    for _ in 1..n {
        doc.ids()?.next();
    }
    let t = doc.tree();
    let mut id = first;
    let mut attrs = || {
        let a = vec![("w:id".to_string(), id.to_string()), ("w:author".into(), rev.author.clone()), ("w:date".into(), rev.date.clone())];
        id += 1;
        a
    };
    for r in runs {
        for k in t.descendants(r).collect::<Vec<_>>() {
            match t.name(k) {
                "w:t" => t.rename(k, "w:delText"),
                "w:instrText" => t.rename(k, "w:delInstrText"),
                _ => {}
            }
        }
        let a = attrs();
        let a: Vec<(&str, &str)> = a.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let del = t.new_element("w:del", &a);
        t.wrap(r, del);
    }
    for p in paras {
        let ppr = match t.child(p, "w:pPr") {
            Some(x) => x,
            None => {
                let x = t.new_element("w:pPr", &[]);
                t.insert_at(p, 0, x);
                x
            }
        };
        let rpr = match t.child(ppr, "w:rPr") {
            Some(x) => x,
            None => {
                let x = t.new_element("w:rPr", &[]);
                // rPr comes before sectPr and pPrChange.
                let at = t.children(ppr).iter().position(|&c| matches!(t.name(c), "w:sectPr" | "w:pPrChange"));
                match at {
                    Some(i) => t.insert_at(ppr, i, x),
                    None => t.append(ppr, x),
                }
                x
            }
        };
        let a = attrs();
        let a: Vec<(&str, &str)> = a.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let del = t.new_element("w:del", &a);
        // After a w:ins mark, otherwise first.
        let at = t.children(rpr).iter().position(|&c| t.is(c, "w:ins")).map_or(0, |i| i + 1);
        t.insert_at(rpr, at, del);
    }
    for r in rows {
        let trpr = match t.child(r, "w:trPr") {
            Some(x) => x,
            None => {
                let x = t.new_element("w:trPr", &[]);
                let at = t.children(r).iter().position(|&c| t.is(c, "w:tblPrEx")).map_or(0, |i| i + 1);
                t.insert_at(r, at, x);
                x
            }
        };
        let a = attrs();
        let a: Vec<(&str, &str)> = a.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let del = t.new_element("w:del", &a);
        let at = t.children(trpr).iter().position(|&c| t.is(c, "w:trPrChange"));
        match at {
            Some(i) => t.insert_at(trpr, i, del),
            None => t.append(trpr, del),
        }
    }
    Ok(())
}
