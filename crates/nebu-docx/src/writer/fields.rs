//! The results of `SEQ` and `REF` fields after an insertion, and for a
//! whole-document update.
//!
//! One pass over the body in document order collects every field with the
//! text nodes of its result, and the text of every bookmark the pass
//! follows. Then:
//!
//! - each `SEQ` field gets its number in document order (`\r` resets,
//!   `\c` repeats, `\* ARABIC`, `ALPHABETIC`, `alphabetic`, `ROMAN` and
//!   `roman` format). Only fields from the first inserted node on are
//!   written; earlier ones keep their bytes. A sequence with a `\s` field
//!   (numbered again after each heading of a level) is left alone, which
//!   the whole-document pass numbers with heading restarts;
//! - each `REF` to a followed bookmark inside the insertion, and each `REF`
//!   outside it whose bookmark's text changed with the numbers, gets that
//!   text as its result. The bookmarks followed are the labels and Word's
//!   own hidden cross-reference bookmarks (`_Ref` and digits), whose text
//!   is read with the new numbers of the `SEQ` fields inside them. The
//!   whole-document pass follows every bookmark and rewrites every stale
//!   bare `REF`, repeating until forward references resolve.
//!
//! A field nested in another field's code (`{ IF { SEQ Table \c } > 0 …}`)
//! counts and gets its number like any other, written into the code as
//! Word writes it there; the field around it keeps its result, which only
//! the fields update computes.
//!
//! Text boxes, the `mc:Fallback` branch of alternate content and deleted
//! content are not walked: Word numbers a text box as a story of its own,
//! reads the branch it understands, and leaves deleted fields out.

use std::collections::{HashMap, HashSet};

use crate::view::BlockKind;
use crate::xml::{NodeId, Tree};
use crate::{Document, Error, ns};

use super::session::Session;

const W: &str = ns::W;
const MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";
const M: &str = "http://schemas.openxmlformats.org/officeDocument/2006/math";

struct Field {
    code: String,
    /// The `w:t` of the result, without those of nested fields.
    texts: Vec<NodeId>,
    /// Separator nodes of the result (`w:tab`, line breaks): detached
    /// with the old text on a rewrite, so a second update cannot double
    /// them.
    seps: Vec<NodeId>,
    /// Where a run goes if the result has no text: before the end run of
    /// a complex field, into a simple field.
    end: Option<NodeId>,
    /// The end marker itself. Splits look it up where it is now: an
    /// earlier split in the same run may already have moved it.
    end_mark: Option<NodeId>,
    simple: Option<NodeId>,
    /// A complex field whose code and result are divided by a separator
    /// run. Without one the result has no readable text.
    separated: bool,
    /// The bookmarks enclosing the first cached `w:t`: where a rewrite
    /// of the result lands, so a cycle check can see what it touches.
    anchor: HashSet<String>,
    /// At or after the first inserted node.
    after: bool,
    /// Inside inserted content.
    inside: bool,
    /// Nested in another field's code, where its result is part of that
    /// code: never written.
    in_code: bool,
    /// The cached result text, separators included.
    old: String,
}

enum Event {
    /// Text and which field's result holds it, if any. The flag marks
    /// the event that starts at the field's first retained `w:t`:
    /// separators and math text share the shape but never claim the
    /// in-place number projection.
    Text(String, Option<usize>, bool),
    FieldEnd(usize),
    Open(String, String),
    Close(String),
    /// A body paragraph with this outline level (1 to 9) was passed.
    Heading(u8),
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
    // Past a short result the count is absurd, so the number stays
    // Arabic rather than looping towards it: a hostile field code must
    // neither hang nor fill memory.
    if n > 100_000 {
        return n.to_string();
    }
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
    // Word repeats the letter: a … z, aa … zz. Past a short result the
    // count is absurd, so the number stays Arabic rather than filling
    // memory: a hostile field code must not exhaust it.
    if n == 0 {
        return String::new();
    }
    if (n - 1) / 26 + 1 > 64 {
        return n.to_string();
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

/// The number a `SEQ` field shows, counting through `counters`, which are
/// keyed per sequence and restart level so mixed `\s` levels under one
/// name count independently. `emitted` remembers the last displayed
/// number per sequence, which is what `\c` repeats. `None` for anything
/// else, for hidden results, and for a restart outside levels 1 to 9
/// (or missing), which are never written.
fn seq_number(
    f: &Field,
    counters: &mut HashMap<(String, Option<u64>), u64>,
    emitted: &mut HashMap<String, u64>,
) -> Option<String> {
    let w = words(&f.code);
    if !w.first().is_some_and(|k| k.eq_ignore_ascii_case("SEQ")) {
        return None;
    }
    let id = w.get(1).map(|s| s.to_lowercase())?;
    let restart = w
        .iter()
        .position(|x| x.eq_ignore_ascii_case("\\s"))
        .and_then(|k| w.get(k + 1))
        .and_then(|v| v.parse::<u64>().ok());
    if w.iter().any(|x| x.eq_ignore_ascii_case("\\s"))
        && !restart.is_some_and(|n| (1..=9).contains(&n))
    {
        return None;
    }
    // `\c` repeats the number last shown, not the counter next in line.
    // A hidden repeat stays hidden and advances neither counter.
    if w.iter().any(|x| x.eq_ignore_ascii_case("\\c")) {
        if w.iter().any(|x| x.eq_ignore_ascii_case("\\h")) {
            return None;
        }
        let n = emitted.get(&id).copied().unwrap_or(0);
        return Some(format(n, &w));
    }
    let c = counters.entry((id.clone(), restart)).or_insert(0);
    let reset = w
        .iter()
        .position(|x| x.eq_ignore_ascii_case("\\r"))
        .and_then(|k| w.get(k + 1))
        .and_then(|v| v.parse::<u64>().ok());
    if let Some(r) = reset {
        *c = r;
    } else {
        *c = c.saturating_add(1);
    }
    if w.iter().any(|x| x.eq_ignore_ascii_case("\\h")) {
        return None;
    }
    let out = format(*c, &w);
    emitted.insert(id, *c);
    Some(out)
}

/// Collects fields and label texts in one pass. `markers` holds the nodes
/// past deletions in the body stand after: an empty replacement deletes,
/// and what follows it renumbers. What stands before keeps its bytes.
/// True if the node itself sits in a subtree the main walk skips:
/// deleted or moved-away runs, text boxes, or fallback content.
fn under_excluded(t: &Tree, mut n: NodeId, stop: NodeId) -> Result<bool, Error> {
    loop {
        if n == stop {
            return Ok(false);
        }
        if let Some(name) = t.name(n)? {
            if name.namespace == Some(W)
                && matches!(
                    name.local,
                    "txbxContent" | "del" | "moveFrom" | "delInstrText"
                )
            {
                return Ok(true);
            }
            if name.namespace == Some(MC) && name.local == "Fallback" {
                return Ok(true);
            }
        }
        let Some(p) = t.parent(n)? else {
            return Ok(false);
        };
        n = p;
    }
}

fn collect(
    t: &Tree,
    body: NodeId,
    inserted: &HashSet<NodeId>,
    labels: &HashSet<String>,
    markers: &HashSet<NodeId>,
    all: bool,
    headings: &HashMap<NodeId, u8>,
) -> Result<(Vec<Field>, Vec<Event>), Error> {
    let mut fields: Vec<Field> = vec![];
    let mut events = vec![];
    let mut open: Vec<Open> = vec![];
    // Bookmarks enclosing the current node, for the write anchor below.
    // Only followed bookmarks can resolve a REF, so the rest stay out,
    // like in the events.
    let mut bstack: Vec<(String, String)> = vec![];
    // The whole-document pass rewrites every stale result, follows every
    // bookmark, and honors heading restarts; the insertion pass only
    // touches fields at or after the edit.
    let mut after = all;
    // Explicit stack: (node, inside inserted content). Children are pushed
    // in reverse so they pop in order.
    let mut stack: Vec<(NodeId, bool)> = vec![];
    let kids: Vec<NodeId> = t.elements(body)?.collect();
    for k in kids.into_iter().rev() {
        stack.push((k, false));
    }
    while let Some((n, inside0)) = stack.pop() {
        // Past a deletion the numbers change: what follows renumbers,
        // while what stands before keeps its bytes.
        if markers.contains(&n) {
            after = true;
        }
        // A body heading restarts the `SEQ` sequences that name its level.
        if let Some(lvl) = headings.get(&n) {
            events.push(Event::Heading(*lvl));
        }
        let inside = inside0 || inserted.contains(&n);
        if inside {
            after = true;
        }
        let Some(name) = t.name(n)? else { continue };
        let local = name.local;
        if name.namespace == Some(MC) && local == "Fallback" {
            continue;
        }
        if name.namespace == Some(M) && local == "t" {
            if !open.last().is_some_and(|o| !o.in_result) {
                let field = open.last().map(|o| o.index);
                events.push(Event::Text(t.text_content(n)?, field, false));
            }
            continue;
        }
        if name.namespace == Some(W) {
            match local {
                "txbxContent" | "del" | "moveFrom" | "delInstrText" => continue,
                "instrText" => {
                    if let Some(o) = open.last() {
                        let text = t.text_content(n)?;
                        if !o.in_result {
                            fields[o.index].code.push_str(&text);
                        } else if fields[o.index].in_code {
                            // The result of a field nested in a code is code
                            // text of the field around it.
                            fields[o.index].texts.push(n);
                            fields[o.index].old.push_str(&text);
                        }
                    }
                    continue;
                }
                "fldSimple" => {
                    let code = t
                        .attribute(n, Some(W), "instr")?
                        .map(|v| v.into_owned())
                        .unwrap_or_default();
                    let index = fields.len();
                    let in_code = open.last().is_some_and(|o| !o.in_result);
                    let mut texts = vec![];
                    let mut seps = vec![];
                    let mut old = String::new();
                    // In a code, the result of a field is code text.
                    // Otherwise the collapsed text is split around
                    // bookmarks nested in the cached result, so a REF
                    // elsewhere can resolve them. The anchor records what
                    // encloses the first cached run, mirroring the main
                    // walk below.
                    let mut buf = String::new();
                    let mut anchor = HashSet::new();
                    let mut sub: Vec<(String, String)> = bstack.clone();
                    let mut anchored = false;
                    // True once the first retained `w:t` is scanned: the
                    // next flush starts there and claims the in-place
                    // number projection.
                    let mut first_pending = false;
                    for d in t.descendants(n)? {
                        if under_excluded(t, d, n)? {
                            continue;
                        }
                        if in_code {
                            if t.is(d, Some(W), "instrText")? {
                                old.push_str(&t.text_content(d)?);
                                texts.push(d);
                            }
                            continue;
                        }
                        let Some(dname) = t.name(d)? else {
                            continue;
                        };
                        if dname.namespace != Some(W) {
                            continue;
                        }
                        if dname.local == "bookmarkStart" {
                            let nm = t
                                .attribute(d, Some(W), "name")?
                                .map(|v| v.into_owned())
                                .unwrap_or_default();
                            if nm.to_lowercase().starts_with("_ref")
                                || labels.contains(&nm.to_lowercase())
                                || all
                            {
                                // A pending first `w:t` claims its event
                                // even when it holds no text: flushing an
                                // empty buffer puts the flagged event at
                                // its actual position, before this mark.
                                if !buf.is_empty() || first_pending {
                                    events.push(Event::Text(
                                        std::mem::take(&mut buf),
                                        Some(index),
                                        std::mem::take(&mut first_pending),
                                    ));
                                }
                                let id = t
                                    .attribute(d, Some(W), "id")?
                                    .map(|v| v.into_owned())
                                    .unwrap_or_default();
                                events.push(Event::Open(id.clone(), nm.clone()));
                                sub.push((id, nm));
                            }
                            continue;
                        }
                        if dname.local == "bookmarkEnd" {
                            // See above: an empty first `w:t` still
                            // claims its event at its position.
                            if !buf.is_empty() || first_pending {
                                events.push(Event::Text(
                                    std::mem::take(&mut buf),
                                    Some(index),
                                    std::mem::take(&mut first_pending),
                                ));
                            }
                            let id = t
                                .attribute(d, Some(W), "id")?
                                .map(|v| v.into_owned())
                                .unwrap_or_default();
                            events.push(Event::Close(id.clone()));
                            if let Some(k) = sub.iter().position(|(i, _)| i == &id) {
                                sub.remove(k);
                            }
                            continue;
                        }
                        match dname.local {
                            "t" => {
                                if texts.is_empty() && !anchored {
                                    anchored = true;
                                    anchor =
                                        sub.iter().map(|(_, name)| name.to_lowercase()).collect();
                                }
                                if texts.is_empty() {
                                    // The first retained `w:t` starts
                                    // here: flush any separators before
                                    // it so they cannot claim the
                                    // in-place number projection.
                                    if !buf.is_empty() {
                                        events.push(Event::Text(
                                            std::mem::take(&mut buf),
                                            Some(index),
                                            false,
                                        ));
                                    }
                                    first_pending = true;
                                }
                                old.push_str(&t.text_content(d)?);
                                texts.push(d);
                                buf.push_str(&t.text_content(d)?);
                            }
                            "tab" => {
                                old.push('\t');
                                seps.push(d);
                                buf.push('\t');
                            }
                            "cr" => {
                                old.push('\n');
                                seps.push(d);
                                buf.push('\n');
                            }
                            "noBreakHyphen" => {
                                old.push('\u{2011}');
                                seps.push(d);
                                buf.push('\u{2011}');
                            }
                            "br" => {
                                // A page or column break is no text of a label.
                                if t.attribute(d, Some(W), "type")?
                                    .is_some_and(|v| v != "textWrapping")
                                {
                                    continue;
                                }
                                old.push('\n');
                                seps.push(d);
                                buf.push('\n');
                            }
                            _ => {}
                        }
                    }
                    fields.push(Field {
                        code,
                        texts,
                        seps,
                        end: None,
                        end_mark: None,
                        simple: Some(n),
                        separated: true,
                        anchor,
                        after,
                        inside,
                        in_code,
                        old: old.clone(),
                    });
                    if in_code {
                        events.push(Event::Text(old, Some(index), false));
                    } else if !buf.is_empty() || first_pending {
                        // An empty trailing cache still emits its
                        // flagged event rather than falling back to
                        // the end marker.
                        events.push(Event::Text(
                            buf,
                            Some(index),
                            std::mem::take(&mut first_pending),
                        ));
                    }
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
                                seps: vec![],
                                end: None,
                                end_mark: None,
                                simple: None,
                                separated: false,
                                anchor: HashSet::new(),
                                after,
                                inside,
                                in_code,
                                old: String::new(),
                            });
                        }
                        Some("separate") => {
                            if let Some(o) = open.last_mut() {
                                o.in_result = true;
                                fields[o.index].separated = true;
                            }
                        }
                        Some("end") => {
                            if let Some(o) = open.pop() {
                                fields[o.index].end = t.parent(n)?;
                                fields[o.index].end_mark = Some(n);
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
                    if lower.starts_with("_ref") || labels.contains(&lower) || all {
                        let id = t
                            .attribute(n, Some(W), "id")?
                            .map(|v| v.into_owned())
                            .unwrap_or_default();
                        events.push(Event::Open(id.clone(), nm.clone()));
                        bstack.push((id, nm));
                    }
                    continue;
                }
                "bookmarkEnd" => {
                    let id = t
                        .attribute(n, Some(W), "id")?
                        .map(|v| v.into_owned())
                        .unwrap_or_default();
                    events.push(Event::Close(id.clone()));
                    if let Some(k) = bstack.iter().position(|(i, _)| i == &id) {
                        bstack.remove(k);
                    }
                    continue;
                }
                "t" | "tab" | "br" | "cr" | "noBreakHyphen" => {
                    let s = match local {
                        "t" => t.text_content(n)?,
                        "tab" => "\t".to_string(),
                        "noBreakHyphen" => "\u{2011}".to_string(),
                        // A page or column break is no text of a label.
                        "br" if t
                            .attribute(n, Some(W), "type")?
                            .is_some_and(|v| v != "textWrapping") =>
                        {
                            continue;
                        }
                        _ => "\n".to_string(),
                    };
                    let in_code = open.last().is_some_and(|o| !o.in_result);
                    if in_code {
                        continue;
                    }
                    let field = open.last().map(|o| o.index);
                    // Only the first retained `w:t` claims the in-place
                    // number projection: separators share the event
                    // shape, but the writer detaches them.
                    let first_wt =
                        local == "t" && field.is_some_and(|f| fields[f].texts.is_empty());
                    if let Some(f) = field {
                        if local == "t" {
                            // The first cached text is where a rewrite
                            // lands: record what encloses it.
                            if fields[f].texts.is_empty() {
                                fields[f].anchor =
                                    bstack.iter().map(|(_, name)| name.to_lowercase()).collect();
                            }
                            fields[f].texts.push(n);
                        } else {
                            // A tab or line break of the result: detached
                            // with the old text on a rewrite.
                            fields[f].seps.push(n);
                        }
                        fields[f].old.push_str(&s);
                    }
                    events.push(Event::Text(s, field, first_wt));
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
/// outside it changed. `deleted` holds the nodes past empty replacements
/// in the body, so that a replacement by nothing renumbers what stands
/// after it: an insertion of nothing otherwise changes nothing.
pub(crate) fn update(
    doc: &mut Document,
    s: &Session<'_>,
    nodes: &[Vec<NodeId>],
    deleted: &[NodeId],
) -> Result<usize, Error> {
    let labels: HashSet<String> = s
        .labels
        .values()
        .map(|l| l.bookmark.to_lowercase())
        .collect();
    let main = s.main.clone();
    let empty = HashMap::new();
    update_inner(doc, &main, &labels, nodes, deleted, false, &empty)
}

/// Recomputes every `SEQ` and bare `REF` result in the body and returns
/// how many distinct fields changed. Fields Word alone can compute
/// (page numbers, dates, external includes) and `REF`s with switches
/// keep their results. `SEQ` sequences that restart at a heading level
/// start again after each heading of that level.
pub(crate) fn update_all(doc: &mut Document) -> Result<usize, Error> {
    let main = doc.main_part().to_string();
    // Heading levels for `SEQ \s` restarts: the body's own paragraphs.
    let levels: Vec<(NodeId, u8)> = {
        let v = doc.views()?;
        v.blocks()
            .iter()
            .filter_map(|b| match (b.kind, b.level) {
                (BlockKind::Paragraph, Some(l)) => Some((b.node, l)),
                _ => None,
            })
            .collect()
    };
    let mut headings = HashMap::new();
    {
        let t = doc.xml(&main)?;
        let Some(body) = t.child(t.root_element(), Some(W), "body")? else {
            return Ok(0);
        };
        for (node, l) in levels {
            if t.parent(node)? == Some(body) {
                headings.insert(node, l);
            }
        }
    }
    let empty = HashSet::new();
    update_inner(doc, &main, &empty, &[], &[], true, &headings)
}

fn update_inner(
    doc: &mut Document,
    main: &str,
    labels: &HashSet<String>,
    nodes: &[Vec<NodeId>],
    deleted: &[NodeId],
    all: bool,
    headings: &HashMap<NodeId, u8>,
) -> Result<usize, Error> {
    let inserted: HashSet<NodeId> = nodes.iter().flatten().copied().collect();
    // Label bookmarks always start with `_ref` (matched by prefix in
    // `collect`); the insertion pass additionally names its content labels,
    // the whole-document pass needs nothing more.
    if !all {
        let t = doc.xml(main)?;
        let any = nodes.iter().flatten().any(|n| t.kind(*n).is_ok());
        if !any && deleted.is_empty() {
            return Ok(0);
        }
        let markers: HashSet<NodeId> = deleted.iter().copied().collect();
        let ((changed, _), _) = update_once(doc, main, labels, &inserted, &markers, all, headings)?;
        return Ok(changed);
    }
    // Whole-document fixpoint: repeat until a pass writes nothing, so
    // forward references resolve through any length of chain. Distinct
    // fields are counted, not rewrites. A dependency cycle stops at the
    // cap with the remaining caches left as they are.
    let markers: HashSet<NodeId> = deleted.iter().copied().collect();
    let mut total: HashSet<usize> = HashSet::new();
    let mut pass = 0u64;
    loop {
        let ((_, written), nfields) =
            update_once(doc, main, labels, &inserted, &markers, all, headings)?;
        if written.is_empty() {
            break;
        }
        total.extend(written);
        pass += 1;
        if pass > nfields as u64 {
            break;
        }
    }
    Ok(total.len())
}

/// Copies an element and everything it holds into a detached twin, so
/// run properties travel with a run split in two. Comments and
/// processing instructions do not travel; runs do not hold any.
fn copy_element(t: &mut Tree, src: NodeId) -> Result<NodeId, Error> {
    fn qualified(namespace: Option<&str>, prefix: &str, local: &str) -> String {
        let _ = namespace;
        if prefix.is_empty() {
            local.to_string()
        } else {
            format!("{prefix}:{local}")
        }
    }
    let (ns, qn): (Option<String>, String) = match t.name(src)? {
        Some(name) => (
            name.namespace.map(str::to_owned),
            qualified(name.namespace, name.prefix, name.local),
        ),
        None => return Err(Error::Edit("can only copy an element".into())),
    };
    let out = t.create_element(ns.as_deref(), &qn)?;
    let attrs: Vec<(Option<String>, String, String)> = t
        .attributes(src)?
        .into_iter()
        .map(|a| {
            (
                a.name.namespace.map(str::to_owned),
                qualified(a.name.namespace, a.name.prefix, a.name.local),
                a.value.into_owned(),
            )
        })
        .collect();
    for (ns, qn, v) in &attrs {
        t.set_attribute(out, ns.as_deref(), qn, v)?;
    }
    let kids: Vec<NodeId> = t.elements(src)?.collect();
    for k in kids {
        let kind = t.kind(k)?;
        if kind == crate::xml::NodeKind::Element {
            let n = copy_element(t, k)?;
            t.append(out, n)?;
            continue;
        }
        if matches!(
            kind,
            crate::xml::NodeKind::Text | crate::xml::NodeKind::CData
        ) && let Some(text) = t.text(k)?.as_deref().map(str::to_owned)
        {
            let n = t.create_text(&text)?;
            t.append(out, n)?;
        }
    }
    Ok(out)
}

/// Splits the run holding a complex field's end marker so the marker
/// starts its own run, and returns that run. The marker is looked up
/// where it is now: an earlier split in the same run may already have
/// moved it, and a stored run would point at the wrong field. A begin,
/// instruction and end sharing one run would otherwise receive the new
/// result outside the field, where the next pass cannot read it back.
/// Run properties travel with the split, so formatted text after the
/// marker keeps its look. Returns `None` when the marker left the tree,
/// in which case the caller leaves a missing result unwritten rather
/// than guessing a run.
fn split_at_end(t: &mut Tree, mark: NodeId) -> Result<Option<NodeId>, Error> {
    let Some(run) = t.parent(mark)? else {
        return Ok(None);
    };
    let kids: Vec<NodeId> = t.elements(run)?.collect();
    let Some(at) = kids.iter().position(|&k| k == mark) else {
        return Ok(Some(run));
    };
    if at == 0 {
        return Ok(Some(run));
    }
    let tail = t.parse_fragment("<w:r/>", &[("w", W)])?[0];
    if let Some(rpr) = t.child(run, Some(W), "rPr")? {
        let copy = copy_element(t, rpr)?;
        t.append(tail, copy)?;
    }
    for k in &kids[at..] {
        t.detach(*k)?;
    }
    for k in &kids[at..] {
        t.append(tail, *k)?;
    }
    t.insert_after(run, tail)?;
    Ok(Some(tail))
}

/// One pass over the body: number `SEQ` fields, resolve label texts, and
/// rewrite stale results. Returns the fields written outside the
/// insertion, if any, every field written, and fields seen.
#[allow(clippy::too_many_arguments)]
fn update_once(
    doc: &mut Document,
    main: &str,
    labels: &HashSet<String>,
    inserted: &HashSet<NodeId>,
    markers: &HashSet<NodeId>,
    all: bool,
    headings: &HashMap<NodeId, u8>,
) -> Result<((usize, Vec<usize>), usize), Error> {
    let budget = doc.limits().max_total_size;
    let t = doc.xml(main)?;
    let Some(body) = t.child(t.root_element(), Some(W), "body")? else {
        return Ok(((0, vec![]), 0));
    };
    let (fields, events) = collect(t, body, inserted, labels, markers, all, headings)?;
    // Numbers of SEQ fields, tracked per sequence and restart level: each
    // field's own `\s` level governs it, so mixed levels under one name
    // count independently.
    let mut counters: HashMap<(String, Option<u64>), u64> = HashMap::new();
    let mut emitted: HashMap<String, u64> = HashMap::new();
    let mut new: Vec<Option<String>> = vec![None; fields.len()];
    if all {
        // Whole-document numbering honors `\s` restarts: after each
        // heading of a named level the sequences naming it start again.
        // Only levels 1 to 9 restart, as heading levels do.
        let mut restarts: HashMap<String, HashSet<u64>> = HashMap::new();
        for f in &fields {
            let w = words(&f.code);
            if w.first().is_some_and(|k| k.eq_ignore_ascii_case("SEQ"))
                && let Some(id) = w.get(1).map(|s| s.to_lowercase())
                && let Some(n) = w
                    .iter()
                    .position(|x| x.eq_ignore_ascii_case("\\s"))
                    .and_then(|k| w.get(k + 1))
                    .and_then(|v| v.parse::<u64>().ok())
                && (1..=9).contains(&n)
            {
                restarts.entry(id).or_default().insert(n);
            }
        }
        for e in &events {
            crate::cost::tick(1);
            match e {
                Event::Heading(lvl) => {
                    let lvl = u64::from(*lvl);
                    for (id, lvls) in &restarts {
                        if lvls.contains(&lvl) {
                            counters.insert((id.clone(), Some(lvl)), 0);
                        }
                    }
                }
                Event::FieldEnd(i) => {
                    new[*i] = seq_number(&fields[*i], &mut counters, &mut emitted);
                }
                _ => {}
            }
        }
    } else {
        let mut restart: HashSet<String> = HashSet::new();
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
            let id = w
                .first()
                .is_some_and(|k| k.eq_ignore_ascii_case("SEQ"))
                .then(|| w.get(1).map(|s| s.to_lowercase()))
                .flatten();
            if id.is_some_and(|id| restart.contains(&id)) {
                continue;
            }
            new[i] = seq_number(f, &mut counters, &mut emitted);
        }
    }
    // Label texts, before and after.
    let mut texts: HashMap<String, (String, String)> = HashMap::new();
    let mut open_marks: Vec<(String, String)> = vec![];
    let mut used = 0u64;
    // SEQ fields whose new number is already projected in place at
    // their first retained text (see below).
    let mut seq_emitted: HashSet<usize> = HashSet::new();
    let seq_value = |i: usize| new[i].clone();
    for e in &events {
        crate::cost::tick(1);
        match e {
            Event::Open(id, name) => open_marks.push((id.clone(), name.clone())),
            Event::Close(id) => {
                if let Some(k) = open_marks.iter().position(|(i, _)| i == id) {
                    let (_, name) = open_marks.remove(k);
                    if all {
                        // Even an empty bookmark resolves, so a REF to
                        // it can have a stale result cleared. The entry
                        // is made at the close so a forward reference
                        // does not copy an empty cache first.
                        texts.entry(name.to_lowercase()).or_default();
                    }
                }
            }
            Event::Text(s, field, first) => {
                // A rewritten SEQ lends its new number in place of its
                // first retained `w:t`, so a bookmark reads the number
                // where the cache shows it even beside nested fields:
                // appending at the field's end would put it after them.
                // Separators never claim the emission (the writer
                // detaches them), nor does code text (it falls back to
                // the end marker below). Later texts of the same result
                // stay suppressed.
                let mut emit: Option<String> = None;
                if *first
                    && let Some(f) = field
                    && !fields[*f].in_code
                    && fields[*f].after
                    && new[*f].is_some()
                    && seq_emitted.insert(*f)
                {
                    emit = new[*f].clone();
                }
                for (_, name) in &open_marks {
                    let e = texts.entry(name.to_lowercase()).or_default();
                    e.0.push_str(s);
                    let mut added = s.len();
                    if let Some(v) = &emit {
                        e.1.push_str(v);
                        added = v.len();
                    } else if !field.is_some_and(|f| fields[f].after && new[f].is_some()) {
                        // The text after the insertion keeps the kept bytes of
                        // the fields before it: only a field whose result is
                        // rewritten lends its new number.
                        e.1.push_str(s);
                    }
                    used += (s.len() + added) as u64;
                }
            }
            Event::FieldEnd(f) => {
                if fields[*f].after
                    && let Some(v) = seq_value(*f)
                    && !seq_emitted.contains(f)
                {
                    if fields[*f].in_code || fields[*f].texts.is_empty() {
                        // No retained result text: contribute the number
                        // where the end marker is.
                        for (_, name) in &open_marks {
                            texts.entry(name.to_lowercase()).or_default().1.push_str(&v);
                            used += v.len() as u64;
                        }
                    } else {
                        // No Text event carried the result (e.g. an empty
                        // cache): project the number where its cached
                        // text lives, as before.
                        for name in fields[*f].anchor.clone() {
                            texts.entry(name.to_lowercase()).or_default().1.push_str(&v);
                            used += v.len() as u64;
                        }
                    }
                }
            }
            // Heading marks only steer numbering, above.
            Event::Heading(_) => {}
        }
        if used > budget {
            return Err(Error::Limit {
                what: format!("label bookmarks that hold more than {budget} bytes of text"),
                field: "max_total_size",
                limit: budget,
            });
        }
    }
    // Bare, resolvable cross-references and their targets. In the
    // whole-document pass only a bare `REF <bookmark>` is rewritten.
    let mut refs: Vec<(usize, String)> = vec![];
    for (i, f) in fields.iter().enumerate() {
        crate::cost::tick(1);
        let w = words(&f.code);
        if f.in_code || !w.first().is_some_and(|k| k.eq_ignore_ascii_case("REF")) {
            continue;
        }
        if all {
            if w.len() != 2 {
                continue;
            }
        } else if w.iter().skip(2).any(|x| {
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
        if !texts.contains_key(&target) {
            continue;
        }
        refs.push((i, target));
    }
    // Dependency cycles (a REF whose target holds another REF back to
    // it, directly or through others, including itself) never settle:
    // each rewrite feeds the next. Their caches stay as they are. Only
    // the whole-document pass looks: the insertion pass runs once.
    // Edges use the first retained text node where a field has one:
    // the actual rewrite anchor. Without one, the write location
    // stands in, so a cached result wholly outside its target does not
    // report a self-cycle it cannot feed.
    let frozen: HashSet<usize> = if all && !refs.is_empty() {
        // Which bookmarks enclose each field's end: where an empty result
        // would be written.
        let mut enclosing: Vec<HashSet<String>> = vec![HashSet::new(); fields.len()];
        {
            let mut open: Vec<(String, String)> = vec![];
            for e in &events {
                match e {
                    Event::Open(id, name) => open.push((id.clone(), name.clone())),
                    Event::Close(id) => {
                        if let Some(k) = open.iter().position(|(i, _)| i == id) {
                            open.remove(k);
                        }
                    }
                    Event::FieldEnd(i) => {
                        for (_, name) in &open {
                            enclosing[*i].insert(name.to_lowercase());
                        }
                    }
                    _ => {}
                }
            }
        }
        // Bookmarks are graph vertices of their own: edges run from each
        // REF to its target and from each bookmark to the REFs it
        // encloses, so one bookmark holding N REFs costs N edges, not
        // N squared.
        #[derive(Clone, Copy, PartialEq, Eq, Hash)]
        enum GNode {
            Ref(usize),
            Mark(usize),
        }
        let in_refs: HashSet<usize> = refs.iter().map(|(i, _)| *i).collect();
        let mut enclosed: Vec<(usize, &HashSet<String>)> = vec![];
        for (i, _) in &refs {
            let f = &fields[*i];
            if f.texts.is_empty() {
                // No retained text: the write goes to the end marker.
                enclosed.push((*i, &enclosing[*i]));
            } else if !f.anchor.is_empty() {
                enclosed.push((*i, &f.anchor));
            }
            // Otherwise the cached text lies wholly outside any bookmark:
            // no edge, as the rewrite touches nothing a REF could name.
        }
        let mut mark_names: Vec<String> = vec![];
        let mut mark_idx: HashMap<String, usize> = HashMap::new();
        let mut adj: HashMap<GNode, Vec<GNode>> = HashMap::new();
        let mut radj: HashMap<GNode, Vec<GNode>> = HashMap::new();
        let intern =
            |name: &str, mark_names: &mut Vec<String>, mark_idx: &mut HashMap<String, usize>| {
                if let Some(&m) = mark_idx.get(name) {
                    return m;
                }
                let m = mark_names.len();
                mark_names.push(name.to_string());
                mark_idx.insert(name.to_string(), m);
                m
            };
        let link = |a: GNode,
                    b: GNode,
                    adj: &mut HashMap<GNode, Vec<GNode>>,
                    radj: &mut HashMap<GNode, Vec<GNode>>| {
            adj.entry(a).or_default().push(b);
            radj.entry(b).or_default().push(a);
        };
        for (j, marks) in enclosed {
            if !in_refs.contains(&j) {
                continue;
            }
            for m in marks {
                let midx = intern(m, &mut mark_names, &mut mark_idx);
                link(GNode::Mark(midx), GNode::Ref(j), &mut adj, &mut radj);
            }
        }
        for (i, t) in &refs {
            let midx = intern(t, &mut mark_names, &mut mark_idx);
            link(GNode::Ref(*i), GNode::Mark(midx), &mut adj, &mut radj);
        }
        // Finish order on the graph.
        let mut visited: HashSet<GNode> = HashSet::new();
        let mut order: Vec<GNode> = vec![];
        let mut all_nodes: Vec<GNode> = refs.iter().map(|(i, _)| GNode::Ref(*i)).collect();
        for m in 0..mark_names.len() {
            all_nodes.push(GNode::Mark(m));
        }
        for &v in &all_nodes {
            if visited.contains(&v) {
                continue;
            }
            let mut stack = vec![(v, false)];
            while let Some((n, done)) = stack.pop() {
                if done {
                    order.push(n);
                    continue;
                }
                if !visited.insert(n) {
                    continue;
                }
                stack.push((n, true));
                if let Some(ns) = adj.get(&n) {
                    for w in ns {
                        if !visited.contains(w) {
                            stack.push((*w, false));
                        }
                    }
                }
            }
        }
        // Its reverse, in reverse finish order: each tree is a strongly
        // connected component. In this bipartite graph a cycle always
        // has more than one member, so every multi-node component is
        // a dependency cycle.
        let mut comp: HashMap<GNode, usize> = HashMap::new();
        let mut frozen = HashSet::new();
        let mut cid = 0usize;
        for &v in order.iter().rev() {
            if comp.contains_key(&v) {
                continue;
            }
            let mut members = vec![];
            let mut stack = vec![v];
            comp.insert(v, cid);
            while let Some(n) = stack.pop() {
                members.push(n);
                if let Some(ns) = radj.get(&n) {
                    for &w in ns {
                        if let std::collections::hash_map::Entry::Vacant(e) = comp.entry(w) {
                            e.insert(cid);
                            stack.push(w);
                        }
                    }
                }
            }
            if members.len() > 1 {
                for m in members {
                    if let GNode::Ref(i) = m {
                        frozen.insert(i);
                    }
                }
            }
            cid += 1;
        }
        frozen
    } else {
        HashSet::new()
    };
    let mut changed = 0;
    let mut written: Vec<usize> = vec![];
    let mut writes: Vec<(usize, String)> = vec![];
    // SEQ fields number above; their writes come first, as before.
    for (i, f) in fields.iter().enumerate() {
        if let Some(v) = &new[i]
            && f.after
            && *v != f.old
        {
            writes.push((i, v.clone()));
        }
    }
    for (i, target) in &refs {
        if frozen.contains(i) {
            continue;
        }
        let f = &fields[*i];
        let Some((old, now)) = texts.get(target) else {
            continue;
        };
        if (f.inside || old != now || all) && *now != f.old {
            writes.push((*i, now.clone()));
        }
    }
    for (i, v) in writes {
        let f = &fields[i];
        if f.in_code && f.texts.is_empty() && f.seps.is_empty() {
            continue;
        }
        if !f.inside {
            changed += 1;
        }
        written.push(i);
        match f.texts.split_first() {
            Some((first, rest)) => {
                for r in rest {
                    t.detach(*r)?;
                }
                for s in &f.seps {
                    t.detach(*s)?;
                }
                // Text up to the first tab or line break goes into the
                // first w:t; the rest follows it in the same run, with tabs
                // and breaks as Word writes them.
                let cut = v.find(['\t', '\n']).unwrap_or(v.len());
                let (head, tail) = v.split_at(cut);
                t.set_text(*first, head)?;
                let mut at = *first;
                for n in t.parse_fragment(&super::xml::run_text(tail), &[("w", W)])? {
                    t.insert_after(at, n)?;
                    at = n;
                }
                let v = head;
                if (v.starts_with(' ') || v.ends_with(' '))
                    && t.attribute(*first, Some(crate::xml::XML_NS), "space")?
                        .is_none()
                {
                    t.set_attribute(*first, Some(crate::xml::XML_NS), "xml:space", "preserve")?;
                }
            }
            None => {
                for s in &f.seps {
                    t.detach(*s)?;
                }
                // A complex field without a readable result gets one where
                // it belongs: split a shared run at the end marker first
                // so the result lands inside the field, then materialize
                // a missing separator as Word does on update. The marker
                // is looked up where it is now, not where collection saw
                // it: an earlier split of a shared run may have moved it.
                let mut anchor = f.end;
                if f.simple.is_none()
                    && let Some(mark) = f.end_mark
                    && let Some(split) = split_at_end(t, mark)?
                {
                    anchor = Some(split);
                    if !f.separated {
                        let sep = t.parse_fragment(
                            "<w:r><w:fldChar w:fldCharType=\"separate\"/></w:r>",
                            &[("w", W)],
                        )?[0];
                        t.insert_before(split, sep)?;
                    }
                }
                let run = t.parse_fragment(
                    &format!("<w:r>{}</w:r>", super::xml::run_text(&v)),
                    &[("w", W)],
                )?[0];
                match (f.simple, anchor) {
                    (Some(sf), _) => t.append(sf, run)?,
                    (None, Some(end)) => t.insert_before(end, run)?,
                    _ => {}
                }
            }
        }
    }
    Ok(((changed, written), fields.len()))
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
        // Past every sane count the number stays Arabic rather than
        // filling memory or hanging on a long alphabetic or Roman form.
        assert_eq!(
            format(u64::MAX, &w("SEQ Table \\* ALPHABETIC")),
            u64::MAX.to_string()
        );
        assert_eq!(
            format(u64::MAX, &w("SEQ Table \\* roman")),
            u64::MAX.to_string()
        );
        assert_eq!(
            format(u64::MAX, &w("SEQ Table \\* ROMAN")),
            u64::MAX.to_string()
        );
        assert_eq!(
            format(65 * 26, &w("SEQ Table \\* alphabetic")),
            (65 * 26).to_string()
        );
        assert_eq!(format(100_001, &w("SEQ Table \\* ROMAN")), "100001");
    }
}
