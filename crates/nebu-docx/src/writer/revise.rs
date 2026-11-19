//! Text replacement inside one paragraph.
//!
//! [`replace_in_paragraph`] swaps every non-overlapping occurrence of one
//! literal string for another in the text of a single paragraph. Runs
//! keep their formatting: the covered text nodes are replaced in place,
//! split at the edges of a match. Anything else a match would have to
//! cut through refuses the replacement with an error that names it, and
//! the document is unchanged.

use crate::view::walk;
use crate::xml::{NodeId, Tree, XML_NS};
use crate::{Document, Error, ns};

use super::place;
use super::xml::run_text;

const W: &str = ns::W;
const MC: &str = walk::MC;

/// Empty elements a match may span: bookmarks, comment anchors and
/// proofing marks. Anything else between the runs of a match refuses
/// it.
const PASS_THROUGH: &[&str] = &[
    "bookmarkStart",
    "bookmarkEnd",
    "commentRangeStart",
    "commentRangeEnd",
    "commentReference",
    "proofErr",
    "permStart",
    "permEnd",
];

/// Inline wrappers whose text reads as the paragraph's but whose
/// boundaries a match may not cross: the replacement lands where the
/// match began, so crossing one would move text across the structure.
/// Alternate-content branches count too: a match that starts in the
/// shown branch and ends outside it would land inside the branch.
const WRAPPERS: &[&str] = &["customXml", "smartTag", "dir", "bdo", "ruby", "rubyBase"];

/// Alternate-content containers, always part of the wrapper chain.
const BRANCHES: &[&str] = &["AlternateContent", "Choice", "Fallback"];

/// What a match refuses to cut through, in plain words.
fn obstruction_name(local: &str) -> &'static str {
    match local {
        "hyperlink" => "a hyperlink",
        "fldSimple" | "fldChar" | "instrText" | "delInstrText" => "a field",
        "drawing" | "pict" | "object" => "a drawing",
        "footnoteReference" => "a footnote",
        "endnoteReference" => "an endnote",
        "sym" => "a symbol",
        "br" | "cr" => "a line break",
        "noBreakHyphen" => "a hyphen",
        "tab" => "a tab stop",
        "sdt" | "sdtContent" => "a content control",
        "ins" => "an inserted revision",
        "del" => "a deleted revision",
        "moveFrom" | "moveFromRangeStart" | "moveFromRangeEnd" => "a moved revision",
        "moveTo" | "moveToRangeStart" | "moveToRangeEnd" => "a moved revision",
        "customXml" => "a custom XML element",
        "smartTag" => "a smart tag",
        "dir" | "bdo" => "a directional wrapper",
        "ruby" => "a ruby annotation",
        "rubyBase" => "a ruby base",
        "AlternateContent" | "Choice" | "Fallback" => "an alternate content block",
        _ => "an inline object",
    }
}

/// One text piece of the paragraph, in document order.
struct Piece {
    /// The `w:t` node, or `None` for a tab.
    node: Option<NodeId>,
    /// Its text; a tab counts as one character.
    text: String,
    /// Why it cannot be replaced, if it cannot.
    blocked: Option<&'static str>,
}

/// Checks the replacement strings before a sweep: the find text must
/// not be empty, and neither string may hold a tab, a line ending or a
/// character XML cannot write. The messages name `--find` or `--with`,
/// so a caller sweeping many paragraphs reports one diagnostic for an
/// argument no paragraph can take.
pub fn validate(find: &str, with: &str) -> Result<(), Error> {
    if find.is_empty() {
        return Err(Error::Invalid("cannot replace an empty string".to_string()));
    }
    for (flag, s) in [("--find", find), ("--with", with)] {
        if let Some(c) = s.chars().find(|c| *c == '\t' || *c == '\n' || *c == '\r') {
            return Err(Error::Invalid(format!(
                "cannot replace across {c:?} in {flag}: tabs and line ends are not text a run holds"
            )));
        }
        if let Err(e) = crate::xml::chars::check_chars(s) {
            return Err(Error::Invalid(format!("cannot use {flag} text: {e}")));
        }
    }
    Ok(())
}

/// Replaces every non-overlapping occurrence of `find` with `with` in
/// the text of one paragraph and reports how many it replaced.
///
/// Both strings are literal text without tabs or line ends, and `find`
/// must not be empty. The paragraph is a paragraph of the views, in any
/// story; runs keep their formatting, split at the edges of a match.
/// Text the reader sees but that is not plain paragraph text — field
/// results, hyperlinks, inserted revisions — is not replaced: a match
/// that touches it is an error that names it. Anything else a match
/// would have to cut through is an error too, and the document is
/// unchanged.
pub fn replace_in_paragraph(
    doc: &mut Document,
    para: NodeId,
    find: &str,
    with: &str,
) -> Result<usize, Error> {
    validate(find, with)?;
    let stories = place::stories(doc)?;
    let Some((part, _)) = stories
        .iter()
        .find(|(p, _)| doc.parsed(p).is_some_and(|t| t.kind(para).is_ok()))
        .cloned()
    else {
        return Err(Error::Invalid(
            "the paragraph is a node of no story of this document; take it from this document's views".into(),
        ));
    };
    {
        let tree = doc.xml(&part)?;
        let Some(name) = tree.name(para)? else {
            return Err(Error::Invalid("the paragraph is not an element".into()));
        };
        if name.namespace != Some(W) || name.local != "p" {
            return Err(Error::Invalid(format!(
                "the paragraph is a {} element, not a paragraph",
                name.local
            )));
        }
    }
    // The text pieces of the paragraph, in document order. Tabs count as
    // the characters the reader sees, so no match ever spans one. Visible
    // hyperlink, field-result and inserted-revision text is included but
    // marked, so a match that touches it refuses; deleted and moved-away
    // text, and text outside the chosen alternate-content branch, is left
    // out entirely. Field ownership starts from the story, so a field
    // that began in an earlier paragraph still holds its result here.
    let (scope, seed, pieces, offsets) = {
        let tree = doc.xml(&part)?;
        let scope = scope_root(tree, para)?;
        let seed = field_seed(tree, scope, para)?;
        let (pieces, offsets) = pieces_of(tree, scope, seed, para)?;
        (scope, seed, pieces, offsets)
    };
    let haystack: String = pieces.iter().map(|p| p.text.as_str()).collect();
    let mut matches = Vec::new();
    let mut from = 0;
    while let Some(at) = haystack[from..].find(find) {
        matches.push(from + at);
        from += at + find.len();
    }
    if matches.is_empty() {
        return Ok(0);
    }
    // Validate every match before changing anything. The seed cannot
    // move: an edit never touches a field marker outside the paragraph.
    {
        let tree = doc.xml(&part)?;
        for start in &matches {
            locate(tree, para, &pieces, &offsets, *start, find.len())?;
        }
    }
    let backup = doc.snapshot();
    let mut count = 0;
    // Right to left, so an edit never moves an earlier match. Each match
    // walks the paragraph afresh, so every node and offset is current;
    // the backup puts the document back if one fails.
    for start in matches.into_iter().rev() {
        match apply_match(doc, &part, scope, seed, para, start, find.len(), with) {
            Ok(()) => count += 1,
            Err(e) => {
                doc.restore(backup);
                return Err(e);
            }
        }
    }
    Ok(count)
}

/// The story container that holds a paragraph: the body, a header or
/// footer, a note, a comment, or the text box around it. Field depth is
/// counted from here, so text boxes stay stories of their own.
fn scope_root(tree: &Tree, para: NodeId) -> Result<NodeId, Error> {
    let mut cur = tree.parent(para)?;
    while let Some(c) = cur {
        if tree.is(c, Some(W), "body")?
            || tree.is(c, Some(W), "hdr")?
            || tree.is(c, Some(W), "ftr")?
            || tree.is(c, Some(W), "footnote")?
            || tree.is(c, Some(W), "endnote")?
            || tree.is(c, Some(W), "comment")?
            || tree.is(c, Some(W), "txbxContent")?
        {
            return Ok(c);
        }
        cur = tree.parent(c)?;
    }
    Ok(tree.root_element())
}

/// Whether `node` sits under the branch of an alternate-content element
/// that the views show: the chosen branch, if any.
fn under_chosen(tree: &Tree, ac: NodeId, node: NodeId) -> Result<bool, Error> {
    let Some(branch) = walk::choose(tree, ac)? else {
        return Ok(false);
    };
    if branch == node {
        return Ok(true);
    }
    let mut cur = tree.parent(node)?;
    while let Some(c) = cur {
        if c == ac {
            return Ok(false);
        }
        if c == branch {
            return Ok(true);
        }
        cur = tree.parent(c)?;
    }
    Ok(false)
}

/// Whether the content of `node` is left out between `stop` (exclusive)
/// and the node: deleted and moved-away revisions, foreign text boxes,
/// and alternate-content branches the views do not show. Paragraph text
/// additionally leaves out nested block structures, which have
/// paragraphs of their own; field depth counts across paragraphs, so it
/// does not.
fn excluded(tree: &Tree, stop: NodeId, node: NodeId, for_text: bool) -> Result<bool, Error> {
    let mut cur = tree.parent(node)?;
    while let Some(a) = cur {
        if a == stop {
            break;
        }
        let name = tree.name(a)?;
        let w_local = name
            .as_ref()
            .filter(|n| n.namespace == Some(W))
            .map(|n| n.local);
        match w_local {
            Some("del" | "moveFrom") => return Ok(true),
            // Ruby annotations are not the paragraph's text: the views
            // skip `w:rt`, and markers there change no field.
            Some("rt") => return Ok(true),
            Some("txbxContent") => {
                if a != stop {
                    return Ok(true);
                }
            }
            Some("p" | "tbl" | "tr" | "tc") if for_text => return Ok(true),
            _ => {}
        }
        if name.is_some_and(|n| n.namespace == Some(MC) && n.local == "AlternateContent")
            && !under_chosen(tree, a, node)?
        {
            return Ok(true);
        }
        cur = tree.parent(a)?;
    }
    Ok(false)
}

/// The field depth at the start of a paragraph: `fldChar` begin and end
/// markers in document order from the story container, nested fields
/// nesting the count, floored at zero for malformed files. Markers in
/// left-out content change nothing.
fn field_seed(tree: &Tree, scope: NodeId, para: NodeId) -> Result<i32, Error> {
    let mut depth = 0i32;
    for node in tree.descendants(scope)? {
        if node == para {
            break;
        }
        if !tree.is(node, Some(W), "fldChar")? {
            continue;
        }
        if excluded(tree, scope, node, false)? {
            continue;
        }
        match tree.attribute(node, Some(W), "fldCharType")?.as_deref() {
            Some("begin") => depth += 1,
            Some("end") => depth = (depth - 1).max(0),
            _ => {}
        }
    }
    Ok(depth)
}

/// Collects the text pieces of a paragraph: every `w:t` and `w:tab` in
/// document order that the views show, except inside deleted and
/// moved-away revisions and nested block structures. Non-editable text
/// reports what holds it; complex-field ownership starts at `seed`, the
/// depth from the story.
fn pieces_of(
    tree: &Tree,
    scope: NodeId,
    seed: i32,
    para: NodeId,
) -> Result<(Vec<Piece>, Vec<usize>), Error> {
    // A paragraph the views do not show has no replaceable text. The
    // nested-block rule does not apply here: cells hold paragraphs.
    if excluded(tree, scope, para, false)? {
        return Ok((Vec::new(), vec![0]));
    }
    let mut depth = seed;
    // The text nodes in order with the field depth at each: replay the
    // markers as they come.
    let mut texts: Vec<(NodeId, bool)> = Vec::new();
    for node in tree.descendants(para)? {
        if tree.is(node, Some(W), "fldChar")? {
            if excluded(tree, para, node, false)? {
                continue;
            }
            match tree.attribute(node, Some(W), "fldCharType")?.as_deref() {
                Some("begin") => depth += 1,
                Some("end") => depth = (depth - 1).max(0),
                _ => {}
            }
            continue;
        }
        if tree.is(node, Some(W), "t")? || tree.is(node, Some(W), "tab")? {
            if excluded(tree, para, node, true)? {
                continue;
            }
            texts.push((node, depth > 0));
        }
    }
    let mut pieces: Vec<Piece> = Vec::new();
    for (node, owned) in texts {
        let mut blocked: Option<&'static str> = if owned { Some("a field") } else { None };
        let mut ancestor = tree.parent(node)?;
        while let Some(a) = ancestor {
            if a == para {
                break;
            }
            let name = tree.name(a)?;
            let local = name.as_ref().map(|n| (n.namespace, n.local));
            match local {
                Some((Some(W), "hyperlink")) => blocked = blocked.or(Some("a hyperlink")),
                Some((Some(W), "fldSimple")) => blocked = blocked.or(Some("a field")),
                Some((Some(W), "ins")) => blocked = blocked.or(Some("an inserted revision")),
                Some((Some(W), "moveTo")) => blocked = blocked.or(Some("a moved revision")),
                Some((Some(W), "sdt" | "sdtContent")) => {
                    blocked = blocked.or(Some("a content control"))
                }
                _ => {}
            }
            ancestor = tree.parent(a)?;
        }
        let is_tab = tree.is(node, Some(W), "tab")?;
        let text = if is_tab {
            "\t".to_string()
        } else {
            tree.text_content(node)?
        };
        if text.is_empty() {
            continue;
        }
        pieces.push(Piece {
            node: if is_tab { None } else { Some(node) },
            text,
            blocked,
        });
    }
    let mut offsets = vec![0usize];
    for piece in &pieces {
        offsets.push(offsets.last().expect("an offset") + piece.text.len());
    }
    Ok((pieces, offsets))
}

/// A match located in a fresh walk: the indices of its first and last
/// pieces.
struct Located {
    first: usize,
    last: usize,
}

/// The inline wrappers around a text node, outermost first: the
/// structures a match may not cross.
fn wrapper_chain(tree: &Tree, para: NodeId, node: NodeId) -> Result<Vec<NodeId>, Error> {
    let mut chain = Vec::new();
    let mut cur = tree.parent(node)?;
    while let Some(a) = cur {
        if a == para {
            break;
        }
        if tree.name(a)?.is_some_and(|n| {
            (n.namespace == Some(W) && WRAPPERS.contains(&n.local))
                || (n.namespace == Some(MC) && BRANCHES.contains(&n.local))
        }) {
            chain.push(a);
        }
        cur = tree.parent(a)?;
    }
    chain.reverse();
    Ok(chain)
}

/// Validates one match and locates the nodes it covers.
///
/// Refuses anything but plain editable text runs with only empty
/// markers around the match.
fn locate(
    tree: &Tree,
    para: NodeId,
    pieces: &[Piece],
    offsets: &[usize],
    start: usize,
    len: usize,
) -> Result<Located, Error> {
    let end = start + len;
    let first = offsets
        .iter()
        .rposition(|o| *o <= start)
        .ok_or_else(|| Error::Invalid("no text matches".to_string()))?;
    let last = offsets
        .iter()
        .rposition(|o| *o < end)
        .ok_or_else(|| Error::Invalid("no text matches".to_string()))?;
    // A match never starts or ends on a tab: it holds no tabs.
    for index in [first, last] {
        if pieces[index].node.is_none() {
            return Err(Error::Invalid(
                "cannot replace text that contains a tab stop".to_string(),
            ));
        }
    }
    // Every covered piece must be editable.
    for piece in &pieces[first..=last] {
        if let Some(why) = piece.blocked {
            return Err(Error::Invalid(format!(
                "cannot replace text that contains {why}"
            )));
        }
    }
    // Every covered piece must sit in the same wrappers: a match that
    // starts inside one and ends outside would move text across it.
    let first_chain = wrapper_chain(
        tree,
        para,
        pieces[first].node.expect("a boundary text node"),
    )?;
    for piece in &pieces[first + 1..=last] {
        let chain = wrapper_chain(tree, para, piece.node.expect("a boundary text node"))?;
        if chain != first_chain {
            // Name the wrapper the match would cross: the first one the
            // two pieces do not share.
            let mut culprit = None;
            for (i, node) in first_chain.iter().enumerate() {
                if chain.get(i) != Some(node) {
                    culprit = Some(*node);
                    break;
                }
            }
            let culprit = culprit
                .or_else(|| chain.get(first_chain.len()).copied())
                .or(first_chain.first().copied())
                .or(chain.first().copied());
            let local = match culprit {
                Some(n) => tree
                    .name(n)?
                    .map(|n| n.local.to_string())
                    .unwrap_or_default(),
                None => String::new(),
            };
            return Err(Error::Invalid(format!(
                "cannot replace text that contains {}",
                obstruction_name(&local)
            )));
        }
    }
    // Every element between the first and the last covered text node
    // must be a run, its properties or text, the paragraph properties,
    // or an empty marker.
    let mut flat: Vec<NodeId> = Vec::new();
    for node in tree.descendants(para)? {
        if tree.name(node)?.is_some() {
            flat.push(node);
        }
    }
    let from = flat
        .iter()
        .position(|&n| Some(n) == pieces[first].node)
        .ok_or_else(|| Error::Invalid("a run without a paragraph".to_string()))?;
    let to = flat
        .iter()
        .position(|&n| n == pieces[last].node.expect("a boundary text node"))
        .ok_or_else(|| Error::Invalid("a run without a paragraph".to_string()))?;
    for &node in &flat[from..=to] {
        if tree.is(node, Some(W), "r")?
            || tree.is(node, Some(W), "t")?
            || tree.is(node, Some(W), "tab")?
            || tree.is(node, Some(W), "rPr")?
            || tree.is(node, Some(W), "pPr")?
        {
            continue;
        }
        // Formatting internals never obstruct: only the run's text is
        // replaced, and the properties stay.
        let mut under_rpr = false;
        let mut ancestor = tree.parent(node)?;
        while let Some(a) = ancestor {
            if a == para {
                break;
            }
            if tree
                .name(a)?
                .is_some_and(|n| n.namespace == Some(W) && n.local == "rPr")
            {
                under_rpr = true;
                break;
            }
            ancestor = tree.parent(a)?;
        }
        if under_rpr {
            continue;
        }
        let name = tree.name(node)?;
        let is_marker = name
            .as_ref()
            .is_some_and(|n| n.namespace == Some(W) && PASS_THROUGH.contains(&n.local));
        if !is_marker {
            let local = name.map(|n| n.local.to_string()).unwrap_or_default();
            return Err(Error::Invalid(format!(
                "cannot replace text that contains {}",
                obstruction_name(&local)
            )));
        }
    }
    Ok(Located { first, last })
}

/// Applies one match: splits the boundary text nodes, clears what the
/// match covers, and writes the replacement into its first run.
///
/// Walks the paragraph afresh so every node and offset is current. The
/// field seed cannot move: an edit never touches a marker outside the
/// paragraph.
#[allow(clippy::too_many_arguments)]
fn apply_match(
    doc: &mut Document,
    part: &str,
    scope: NodeId,
    seed: i32,
    para: NodeId,
    start: usize,
    len: usize,
    with: &str,
) -> Result<(), Error> {
    let tree = doc.xml(part)?;
    let (pieces, offsets) = pieces_of(tree, scope, seed, para)?;
    let located = locate(tree, para, &pieces, &offsets, start, len)?;
    let end = start + len;
    // Split the boundary text nodes so the covered ones hold exactly
    // the match. The boundaries are always text: a match holds no tabs.
    let mut covered: Vec<NodeId> = Vec::new();
    if located.first == located.last {
        let mut node = text_node(&pieces[located.first])?;
        let text = pieces[located.first].text.clone();
        let head = start - offsets[located.first];
        if head > 0 {
            node = split_text(tree, node, &text, head)?;
        }
        // The node starts exactly at the match; split off what follows it.
        let rest = text[head.min(text.len())..].to_string();
        if len < rest.len() {
            split_text(tree, node, &rest, len)?;
        }
        covered.push(node);
    } else {
        let mut first_node = text_node(&pieces[located.first])?;
        let head = start - offsets[located.first];
        if head > 0 {
            let text = pieces[located.first].text.clone();
            first_node = split_text(tree, first_node, &text, head)?;
        }
        covered.push(first_node);
        for piece in &pieces[located.first + 1..located.last] {
            covered.push(text_node(piece)?);
        }
        let last_node = text_node(&pieces[located.last])?;
        let tail = end - offsets[located.last];
        if tail < pieces[located.last].text.len() {
            let text = pieces[located.last].text.clone();
            split_text(tree, last_node, &text, tail)?;
        }
        covered.push(last_node);
    }
    // The replacement goes where the match began; the rest goes away.
    if !with.is_empty() {
        for node in tree.parse_fragment(&run_text(with), &[("w", W)])? {
            tree.insert_before(covered[0], node)?;
        }
    }
    for node in covered {
        tree.detach(node)?;
    }
    Ok(())
}

/// The text node of a piece, which is always text: a match holds no
/// tabs, so a tab boundary would mean the match itself is broken.
fn text_node(piece: &Piece) -> Result<NodeId, Error> {
    piece
        .node
        .ok_or_else(|| Error::Invalid("cannot replace text that contains a tab stop".to_string()))
}

/// Splits a text node after a byte offset, returning the node with the
/// tail. The offset falls strictly inside the text.
fn split_text(tree: &mut Tree, node: NodeId, text: &str, at: usize) -> Result<NodeId, Error> {
    let (head, tail) = text.split_at(at);
    tree.set_text(node, head)?;
    if (head.starts_with(' ') || head.ends_with(' '))
        && tree.attribute(node, Some(XML_NS), "space")?.is_none()
    {
        tree.set_attribute(node, Some(XML_NS), "xml:space", "preserve")?;
    }
    let mut tail_node = None;
    for n in tree.parse_fragment("<w:t/>", &[("w", W)])? {
        tail_node = Some(n);
    }
    let tail_node =
        tail_node.ok_or_else(|| Error::Invalid("cannot split a text node".to_string()))?;
    tree.set_text(tail_node, tail)?;
    if tail.starts_with(' ') || tail.ends_with(' ') {
        tree.set_attribute(tail_node, Some(XML_NS), "xml:space", "preserve")?;
    }
    tree.insert_after(node, tail_node)?;
    Ok(tail_node)
}
