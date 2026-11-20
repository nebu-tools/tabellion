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

use super::Track;
use super::fields::copy_element;
use super::ids::Ids;
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

/// The author a revision element belongs to: `w:author` of an
/// insertion, a move or a property change. Missing or empty means
/// hand-made markup, which counts as someone else's; the reporting
/// layer names it, never the identity check.
fn revision_author(tree: &Tree, node: NodeId) -> Result<Option<String>, Error> {
    Ok(tree
        .attribute(node, Some(W), "author")?
        .map(|v| v.into_owned())
        .filter(|a| !a.is_empty()))
}

/// Adds an author once.
fn push_author(authors: &mut Vec<String>, author: String) {
    if !authors.contains(&author) {
        authors.push(author);
    }
}

/// The authors of pending changes a match would disturb: insertions
/// and moves holding covered text that belong to someone else — moves
/// always do — and property changes on covered runs that belong to
/// someone else. Sorted and deduplicated for a stable message.
fn foreign_authors(
    tree: &Tree,
    scope: NodeId,
    pieces: &[Piece],
    first: usize,
    last: usize,
    rev: &Track,
) -> Result<Vec<String>, Error> {
    let mut authors: Vec<String> = Vec::new();
    // Covered runs, once each.
    let mut runs: Vec<NodeId> = Vec::new();
    for piece in &pieces[first..=last] {
        let node = piece.node.expect("a boundary text node");
        let mut run = tree.parent(node)?;
        while let Some(r) = run {
            if tree.is(r, Some(W), "r")? {
                break;
            }
            run = tree.parent(r)?;
        }
        let Some(run) = run else {
            return Err(Error::Invalid("a run without a paragraph".to_string()));
        };
        if !runs.contains(&run) {
            runs.push(run);
        }
    }
    for run in runs {
        // A property change on the run itself. A missing author counts
        // as someone else's; only an explicit match is its own.
        if let Some(rpr) = tree.child(run, Some(W), "rPr")?
            && let Some(change) = tree.child(rpr, Some(W), "rPrChange")?
        {
            let author = revision_author(tree, change)?;
            if author.as_deref() != Some(rev.author.as_str()) {
                push_author(
                    &mut authors,
                    author.unwrap_or_else(|| "unknown".to_string()),
                );
            }
        }
        // Insertions and moves holding the run, up to the story: the
        // paragraph itself may sit in a tracked insertion. Moves always
        // belong to someone else: the engine never rewrites moved text.
        let mut ancestor = tree.parent(run)?;
        while let Some(a) = ancestor {
            if a == scope {
                break;
            }
            let name = tree.name(a)?;
            let tracked = name
                .as_ref()
                .is_some_and(|n| n.namespace == Some(W) && matches!(n.local, "ins" | "moveTo"));
            if tracked {
                let author = revision_author(tree, a)?;
                let foreign = name.as_ref().is_some_and(|n| n.local == "moveTo")
                    || author.as_deref() != Some(rev.author.as_str());
                if foreign {
                    push_author(
                        &mut authors,
                        author.unwrap_or_else(|| "unknown".to_string()),
                    );
                }
            }
            ancestor = tree.parent(a)?;
        }
    }
    authors.sort();
    Ok(authors)
}

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
/// character XML cannot write. With a tracking author, the author and
/// date must be writable too, so a bad stamp fails before anything
/// changes instead of halfway through a match. The messages name
/// `--find` or `--with`, so a caller sweeping many paragraphs reports
/// one diagnostic for an argument no paragraph can take.
pub fn validate(find: &str, with: &str, rev: Option<&Track>) -> Result<(), Error> {
    use crate::xml::chars::check_chars;
    if find.is_empty() {
        return Err(Error::Invalid("cannot replace an empty string".to_string()));
    }
    for (flag, s) in [("--find", find), ("--with", with)] {
        if let Some(c) = s.chars().find(|c| *c == '\t' || *c == '\n' || *c == '\r') {
            return Err(Error::Invalid(format!(
                "cannot replace across {c:?} in {flag}: tabs and line ends are not text a run holds"
            )));
        }
        if let Err(e) = check_chars(s) {
            return Err(Error::Invalid(format!("cannot use {flag} text: {e}")));
        }
    }
    if let Some(rev) = rev {
        for (flag, s) in [("author", &rev.author), ("date", &rev.date)] {
            if let Err(e) = check_chars(s) {
                return Err(Error::Invalid(format!("cannot use tracking {flag}: {e}")));
            }
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
///
/// With a tracking author, the covered runs are wrapped in a deletion
/// by that author instead of removed, and the replacement goes in an
/// insertion after it; a match that would disturb another author's
/// pending change is a [`Error::ForeignRevision`].
pub fn replace_in_paragraph(
    doc: &mut Document,
    para: NodeId,
    find: &str,
    with: &str,
    rev: Option<&Track>,
) -> Result<usize, Error> {
    let mut report = replace_all(doc, &[para], find, with, rev)?;
    if let Some((_, error)) = report.refused.pop() {
        return Err(error);
    }
    Ok(report.replaced.pop().map(|(_, n)| n).unwrap_or(0))
}

/// What replacing many paragraphs did: the count per paragraph that
/// changed, and the error per paragraph that refused.
#[derive(Debug, Default)]
pub struct BatchReport {
    /// The paragraphs replaced with their counts, in sweep order.
    pub replaced: Vec<(NodeId, usize)>,
    /// The paragraphs refused with why, in sweep order.
    pub refused: Vec<(NodeId, Error)>,
}

/// Replaces every non-overlapping occurrence of `find` with `with` in
/// each paragraph and reports what each did.
///
/// Paragraphs come from the document's views, in any stories; each is
/// replaced as [`replace_in_paragraph`] would replace it, with the same
/// refusals. One walk per story seeds every paragraph's field depth,
/// and one snapshot per part rolls a paragraph back if it fails, so a
/// sweep costs a story walk and a part copy rather than both per
/// paragraph. Paragraphs are independent subtrees: a refusal leaves the
/// others as they were.
pub fn replace_all(
    doc: &mut Document,
    paras: &[NodeId],
    find: &str,
    with: &str,
    rev: Option<&Track>,
) -> Result<BatchReport, Error> {
    use std::collections::HashMap;
    validate(find, with, rev)?;
    let stories = place::stories(doc)?;
    // Group the paragraphs by part, keeping sweep order.
    let mut by_part: Vec<(String, Vec<NodeId>)> = Vec::new();
    let mut report = BatchReport::default();
    for &para in paras {
        match stories
            .iter()
            .find(|(p, _)| doc.parsed(p).is_some_and(|t| t.kind(para).is_ok()))
        {
            Some((part, _)) => match by_part.iter_mut().find(|(p, _)| p == part) {
                Some((_, paras)) => paras.push(para),
                None => by_part.push((part.clone(), vec![para])),
            },
            None => report.refused.push((
                para,
                Error::Invalid(
                    "the paragraph is a node of no story of this document; take it from this document's views"
                        .into(),
                ),
            )),
        }
    }
    for (part, paras) in by_part {
        // Revision ids are collected for the part on the first tracked
        // match that applies, not up front: most paragraphs match
        // nothing, and the scan costs a tree walk of its own.
        let mut ids: Option<Ids> = None;
        // Seed every paragraph in one walk per scope.
        let mut scoped: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
        {
            let tree = doc.xml(&part)?;
            for &para in &paras {
                scoped
                    .entry(scope_root(tree, para)?)
                    .or_default()
                    .push(para);
            }
        }
        let mut seeds: HashMap<NodeId, (NodeId, i32)> = HashMap::new();
        for (scope, paras) in &scoped {
            let tree = doc.xml(&part)?;
            // A scope detached from the part — a text box pulled out of
            // the tree, for example — holds no replaceable paragraphs:
            // the seed walk below would find them anyway, and the edit
            // would never serialize. They report as detached below.
            if !reachable(tree, *scope)? {
                continue;
            }
            let mut depth = 0i32;
            let mut pending: std::collections::HashSet<NodeId> = paras.iter().copied().collect();
            for node in tree.descendants(*scope)? {
                if pending.remove(&node) {
                    seeds.insert(node, (*scope, depth));
                    if pending.is_empty() {
                        break;
                    }
                    continue;
                }
                if !tree.is(node, Some(W), "fldChar")? {
                    continue;
                }
                if excluded(tree, *scope, node, false)? {
                    continue;
                }
                match tree.attribute(node, Some(W), "fldCharType")?.as_deref() {
                    Some("begin") => depth += 1,
                    Some("end") => depth = (depth - 1).max(0),
                    _ => {}
                }
            }
        }
        for &para in &paras {
            let Some(&(scope, seed)) = seeds.get(&para) else {
                // Detached or otherwise unreachable: reported, not skipped.
                report.refused.push((
                    para,
                    Error::Invalid("the paragraph is not attached to its story".to_string()),
                ));
                continue;
            };
            match replace_one(doc, &part, scope, seed, para, find, with, rev, &mut ids) {
                Ok(n) => report.replaced.push((para, n)),
                Err(e) => report.refused.push((para, e)),
            }
        }
    }
    Ok(report)
}

/// Replaces every match in one paragraph whose seeds are known: checks
/// the paragraph, validates every match, then applies them right to
/// left under one part snapshot.
#[allow(clippy::too_many_arguments)]
fn replace_one(
    doc: &mut Document,
    part: &str,
    scope: NodeId,
    seed: i32,
    para: NodeId,
    find: &str,
    with: &str,
    rev: Option<&Track>,
    ids: &mut Option<Ids>,
) -> Result<usize, Error> {
    {
        let tree = doc.xml(part)?;
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
    let (pieces, offsets) = {
        let tree = doc.xml(part)?;
        pieces_of(tree, scope, seed, para, rev)?
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
        let tree = doc.xml(part)?;
        for start in &matches {
            locate(
                tree,
                scope,
                para,
                &pieces,
                &offsets,
                *start,
                find.len(),
                rev,
            )?;
        }
    }
    // One paragraph log rolls the paragraph back if a match fails:
    // every mutation is recorded, the arena is truncated to the
    // checkpoint, the recorded flags keep the serializer from rewriting
    // what the edit never touched, and no node budget leaks into the
    // next paragraph.
    let mut undo: Vec<Undo> = Vec::new();
    let (checkpoint, clean) = {
        let tree = doc.xml(part)?;
        (tree.checkpoint(), tree.flags_snapshot())
    };
    // Revision ids are collected on the first paragraph that actually
    // applies a tracked match: most paragraphs match nothing, and the
    // scan costs a tree walk of its own.
    if rev.is_some() && ids.is_none() {
        let owned = part.to_string();
        *ids = Some(Ids::collect(
            doc,
            std::slice::from_ref(&owned),
            super::ids::seed(doc),
        )?);
    }
    let mut count = 0;
    // Right to left, so an edit never moves an earlier match. Each match
    // walks the paragraph afresh, so every node and offset is current;
    // the undo log puts the document back if one fails.
    for start in matches.into_iter().rev() {
        match apply_match(
            doc,
            part,
            scope,
            seed,
            para,
            start,
            find.len(),
            with,
            rev,
            ids,
            &mut undo,
        ) {
            Ok(()) => count += 1,
            Err(e) => {
                let tree = doc.xml(part)?;
                rollback(tree, undo, checkpoint, &clean)?;
                return Err(e);
            }
        }
    }
    Ok(count)
}

/// Whether a scope container is attached to its part: the story
/// containers the walk starts from, and anything they hold, serialize.
/// A detached container — or a paragraph in one — reports as detached
/// instead of seeding matches that would never be written.
fn reachable(tree: &Tree, scope: NodeId) -> Result<bool, Error> {
    let root = tree.root_element();
    if scope == root {
        return Ok(true);
    }
    for a in tree.ancestors(scope)? {
        if a == root {
            return Ok(true);
        }
    }
    Ok(false)
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

/// Collects the text pieces of a paragraph: every `w:t` and `w:tab` in
/// document order that the views show, except inside deleted and
/// moved-away revisions and nested block structures. Non-editable text
/// reports what holds it; complex-field ownership starts at `seed`, the
/// depth from the story. With a tracking author, text inside that
/// author's insertions stays editable: replacing it wraps it like any
/// other text, and only another author's pending change refuses the
/// match.
fn pieces_of(
    tree: &Tree,
    scope: NodeId,
    seed: i32,
    para: NodeId,
    rev: Option<&Track>,
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
        // Insertions, moves and content controls hold the paragraph from
        // above it too: a tracked-inserted paragraph is wrapped in `w:ins`
        // around the paragraph itself. Walk to the story container, not
        // just the paragraph. With a tracking author, that author's own
        // insertions stay editable and never block; anything else a match
        // touches is refused, by name without tracking and as a foreign
        // revision with it.
        let mut ancestor = tree.parent(node)?;
        while let Some(a) = ancestor {
            if a == scope {
                break;
            }
            let name = tree.name(a)?;
            let local = name.as_ref().map(|n| (n.namespace, n.local));
            match local {
                Some((Some(W), "hyperlink")) => blocked = blocked.or(Some("a hyperlink")),
                Some((Some(W), "fldSimple")) => blocked = blocked.or(Some("a field")),
                Some((Some(W), "ins")) => {
                    // Without tracking every insertion refuses; with it,
                    // authorship is decided per match, so only another
                    // author's pending change refuses.
                    if rev.is_none() {
                        blocked = blocked.or(Some("an inserted revision"));
                    }
                }
                Some((Some(W), "moveTo")) => {
                    if rev.is_none() {
                        blocked = blocked.or(Some("a moved revision"));
                    }
                }
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
/// markers around the match. With a tracking author, a match that
/// would disturb another author's pending change is a foreign
/// revision instead of an invalid edit.
#[allow(clippy::too_many_arguments)]
fn locate(
    tree: &Tree,
    scope: NodeId,
    para: NodeId,
    pieces: &[Piece],
    offsets: &[usize],
    start: usize,
    len: usize,
    rev: Option<&Track>,
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
    // A match that would disturb another author's pending change is a
    // foreign revision, not an invalid edit.
    if let Some(rev) = rev {
        let authors = foreign_authors(tree, scope, pieces, first, last, rev)?;
        if !authors.is_empty() {
            return Err(Error::ForeignRevision { authors });
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
            // A tracked match may cross the author's own insertions:
            // the covered runs are wrapped where they stand.
            || (rev.is_some() && tree.is(node, Some(W), "ins")?)
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

/// What one apply step changed, enough to put it back. The log is
/// paragraph-local: every mutated node belongs to the paragraph but
/// created ones, which outlive the log only if the apply succeeds.
/// Replay allocates nothing, so rollback itself cannot fail on limits.
enum Undo {
    /// An element whose children were replaced, with the originals in
    /// order. Restoring detaches whatever is there and appends these.
    Children { node: NodeId, old: Vec<NodeId> },
    /// `xml:space` set on an existing node, with what it had.
    Space { node: NodeId, old: Option<String> },
    /// A node moved or detached, with where it was.
    Moved {
        node: NodeId,
        parent: NodeId,
        index: usize,
    },
    /// Text renamed to deleted text, with the qualified name and rename
    /// state it had, so rollback restores the original spelling.
    Renamed {
        node: NodeId,
        namespace: Option<String>,
        prefix: String,
        local: String,
        renamed: bool,
    },
    /// A node created during the apply, to detach on rollback.
    Created { node: NodeId },
}

/// Records where a node sits before the apply moves or detaches it.
fn record_move(tree: &Tree, undo: &mut Vec<Undo>, node: NodeId) -> Result<(), Error> {
    let parent = tree
        .parent(node)?
        .ok_or_else(|| Error::Invalid("a run without a paragraph".to_string()))?;
    let kids: Vec<NodeId> = tree.children(parent)?.collect();
    let index = kids
        .iter()
        .position(|&k| k == node)
        .ok_or_else(|| Error::Invalid("a run without a paragraph".to_string()))?;
    undo.push(Undo::Moved {
        node,
        parent,
        index,
    });
    Ok(())
}

/// Puts back every step in reverse: original children, names and
/// properties return, moved nodes go home, and created subtrees are
/// detached. Truncating the arena then drops them for good and restores
/// the node budget; restoring the recorded flags keeps the serializer
/// from rewriting what the edit never touched — including earlier
/// paragraphs' successful edits, whose dirt stays dirty. The edits
/// counter stays bumped so cached views recompute rather than trust
/// handles from the rolled-back attempt.
fn rollback(
    tree: &mut Tree,
    undo: Vec<Undo>,
    checkpoint: usize,
    flags: &[u8],
) -> Result<(), Error> {
    for op in undo.into_iter().rev() {
        match op {
            Undo::Children { node, old } => {
                let current: Vec<NodeId> = tree.children(node)?.collect();
                for kid in current {
                    tree.detach(kid)?;
                }
                for kid in old {
                    tree.append(node, kid)?;
                }
            }
            Undo::Space { node, old } => match old {
                Some(value) => tree.set_attribute(node, Some(XML_NS), "xml:space", &value)?,
                None => {
                    let _ = tree.remove_attribute(node, Some(XML_NS), "space")?;
                }
            },
            Undo::Moved {
                node,
                parent,
                index,
            } => {
                tree.detach(node)?;
                tree.insert(parent, index, node)?;
            }
            Undo::Renamed {
                node,
                namespace,
                prefix,
                local,
                renamed,
            } => tree.restore_name(node, namespace.as_deref(), &prefix, &local, renamed)?,
            Undo::Created { node } => {
                if tree.parent(node)?.is_some() {
                    tree.detach(node)?;
                }
            }
        }
    }
    tree.truncate(checkpoint);
    tree.restore_flags(flags);
    Ok(())
}

/// Applies one match: splits the boundary text nodes, clears what the
/// match covers, and writes the replacement into its first run. With a
/// tracking author, the covered runs are wrapped in a deletion instead
/// of detached, and the replacement goes in an insertion after it.
///
/// Walks the paragraph afresh so every node and offset is current. The
/// field seed cannot move: an edit never touches a marker outside the
/// paragraph.
///
/// Every mutation is recorded, so a failure puts the paragraph back:
/// original nodes return, created ones are detached, and the arena is
/// truncated to the checkpoint, which restores the node budget too.
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
    rev: Option<&Track>,
    ids: &mut Option<Ids>,
    undo: &mut Vec<Undo>,
) -> Result<(), Error> {
    let tree = doc.xml(part)?;
    let (pieces, offsets) = pieces_of(tree, scope, seed, para, rev)?;
    let located = locate(tree, scope, para, &pieces, &offsets, start, len, rev)?;
    let end = start + len;
    // Split the boundary text nodes so the covered ones hold exactly
    // the match. The boundaries are always text: a match holds no tabs.
    let mut covered: Vec<NodeId> = Vec::new();
    if located.first == located.last {
        let mut node = text_node(&pieces[located.first])?;
        let text = pieces[located.first].text.clone();
        let head = start - offsets[located.first];
        if head > 0 {
            node = split_text(tree, node, &text, head, undo)?;
        }
        // The node starts exactly at the match; split off what follows it.
        let rest = text[head.min(text.len())..].to_string();
        if len < rest.len() {
            split_text(tree, node, &rest, len, undo)?;
        }
        covered.push(node);
    } else {
        let mut first_node = text_node(&pieces[located.first])?;
        let head = start - offsets[located.first];
        if head > 0 {
            let text = pieces[located.first].text.clone();
            first_node = split_text(tree, first_node, &text, head, undo)?;
        }
        covered.push(first_node);
        for piece in &pieces[located.first + 1..located.last] {
            covered.push(text_node(piece)?);
        }
        let last_node = text_node(&pieces[located.last])?;
        let tail = end - offsets[located.last];
        if tail < pieces[located.last].text.len() {
            let text = pieces[located.last].text.clone();
            split_text(tree, last_node, &text, tail, undo)?;
        }
        covered.push(last_node);
    }
    match rev {
        None => {
            // The replacement goes where the match began; the rest goes away.
            if !with.is_empty() {
                let new: Vec<NodeId> = tree
                    .parse_fragment(&run_text(with), &[("w", W)])?
                    .into_iter()
                    .collect();
                for &node in &new {
                    undo.push(Undo::Created { node });
                }
                for node in new {
                    tree.insert_before(covered[0], node)?;
                }
            }
            for node in covered {
                record_move(tree, undo, node)?;
                tree.detach(node)?;
            }
        }
        Some(rev) => {
            let ids = ids.as_mut().expect("revision ids with a tracking author");
            apply_tracked(tree, &covered, with, rev, ids, undo)?
        }
    }
    Ok(())
}

/// The runs holding covered text nodes, in document order.
fn covered_runs(tree: &Tree, covered: &[NodeId]) -> Result<Vec<NodeId>, Error> {
    let mut runs: Vec<NodeId> = Vec::new();
    for &node in covered {
        let mut run = tree.parent(node)?;
        while let Some(r) = run {
            if tree.is(r, Some(W), "r")? {
                break;
            }
            run = tree.parent(r)?;
        }
        let Some(run) = run else {
            return Err(Error::Invalid("a run without a paragraph".to_string()));
        };
        if runs.last() != Some(&run) {
            runs.push(run);
        }
    }
    Ok(runs)
}

/// Gives every revision mark in a copied properties element a fresh
/// id: the mark records the same change on a new run, so it needs its
/// own identity rather than sharing the original's.
fn fresh_revision_ids(tree: &mut Tree, rpr: NodeId, ids: &mut Ids) -> Result<(), Error> {
    let mut marks: Vec<NodeId> = Vec::new();
    for node in tree.descendants(rpr)? {
        if tree.name(node)?.is_some_and(|n| {
            n.namespace == Some(W)
                && matches!(n.local, "ins" | "del" | "moveFrom" | "moveTo" | "rPrChange")
        }) && tree.attribute(node, Some(W), "id")?.is_some()
        {
            marks.push(node);
        }
    }
    for mark in marks {
        tree.set_attribute(mark, Some(W), "w:id", &ids.revision().to_string())?;
    }
    Ok(())
}

/// Splits a run before a child node: the run keeps the children ahead
/// of it, and a twin with copied properties takes the node and what
/// follows it. Markers keep their side of the split.
fn split_run_before(
    tree: &mut Tree,
    run: NodeId,
    node: NodeId,
    ids: &mut Ids,
    undo: &mut Vec<Undo>,
) -> Result<NodeId, Error> {
    let twin = tree.create_element(Some(W), "w:r")?;
    undo.push(Undo::Created { node: twin });
    if let Some(rpr) = tree.child(run, Some(W), "rPr")? {
        let twin_props = copy_element(tree, rpr)?;
        undo.push(Undo::Created { node: twin_props });
        tree.append(twin, twin_props)?;
        fresh_revision_ids(tree, twin_props, ids)?;
    }
    tree.insert_after(run, twin)?;
    // Every moved child sits at the split point when it moves: earlier
    // moves shift the rest down into the same slot.
    let at = tree
        .children(run)?
        .position(|k| k == node)
        .ok_or_else(|| Error::Invalid("a run without a paragraph".to_string()))?;
    let kids: Vec<NodeId> = tree.children(run)?.collect();
    let mut started = false;
    for child in kids {
        if child == node {
            started = true;
        }
        if started {
            undo.push(Undo::Moved {
                node: child,
                parent: run,
                index: at,
            });
            tree.append(twin, child)?;
        }
    }
    Ok(twin)
}

/// Applies one tracked match: isolates the covered runs, wraps them in
/// a deletion, and writes the replacement into an insertion after it.
/// Runs keep their properties; a run inside the author's own insertion
/// stays where it is, so the deletion nests like Word nests it.
fn apply_tracked(
    tree: &mut Tree,
    covered: &[NodeId],
    with: &str,
    rev: &Track,
    ids: &mut Ids,
    undo: &mut Vec<Undo>,
) -> Result<(), Error> {
    // Isolate the covered runs: split the boundary runs at the edges of
    // the match so every covered run holds exactly matched text.
    let mut runs = covered_runs(tree, covered)?;
    let first_run = runs[0];
    let kids: Vec<NodeId> = tree.elements(first_run)?.collect();
    let first_at = kids
        .iter()
        .position(|&k| k == covered[0])
        .expect("covered text in its run");
    let content_at = kids
        .iter()
        .position(|&k| !tree.is(k, Some(W), "rPr").unwrap_or(false))
        .unwrap_or(kids.len());
    if first_at > content_at {
        split_run_before(tree, first_run, covered[0], ids, undo)?;
        runs = covered_runs(tree, covered)?;
    }
    let last_run = *runs.last().expect("a covered run");
    let kids: Vec<NodeId> = tree.elements(last_run)?.collect();
    let last_at = kids
        .iter()
        .position(|&k| k == covered[covered.len() - 1])
        .expect("covered text in its run");
    if last_at + 1 < kids.len() {
        split_run_before(tree, last_run, kids[last_at + 1], ids, undo)?;
        runs = covered_runs(tree, covered)?;
    }
    // Wrap each run group in a deletion where it stands. Runs that
    // share a parent with nothing between them go in one deletion, so
    // rejecting one puts every run back exactly where it stood. Range
    // markers travel with their runs, and paragraph-level markers stay
    // put.
    let mut first_del = None;
    let mut group: Vec<NodeId> = Vec::new();
    let mut kids: Vec<NodeId> = Vec::new();
    for &run in &runs {
        let parent = tree
            .parent(run)?
            .ok_or_else(|| Error::Invalid("a run without a paragraph".to_string()))?;
        let continues = match group.last() {
            None => true,
            Some(_) => {
                // Same parent, and nothing between the runs: the group's
                // children list is walked once per parent.
                let same = tree.parent(group[0])? == Some(parent);
                same && {
                    if kids.is_empty() {
                        kids = tree.children(parent)?.collect();
                    }
                    adjacent_positions(&kids, group[group.len() - 1], run)
                }
            }
        };
        if !continues {
            let del = wrap_deletion(tree, &group, rev, ids, undo)?;
            first_del = first_del.or(Some(del));
            group.clear();
            kids.clear();
        }
        group.push(run);
    }
    if !group.is_empty() {
        let del = wrap_deletion(tree, &group, rev, ids, undo)?;
        first_del = first_del.or(Some(del));
    }
    // The replacement goes in an insertion after the first deletion.
    if !with.is_empty() {
        let Some(del) = first_del else {
            return Err(Error::Invalid("a match without a paragraph".to_string()));
        };
        let ins = tree.create_element(Some(W), "w:ins")?;
        undo.push(Undo::Created { node: ins });
        set_revision(tree, ins, rev, ids)?;
        let run = tree.create_element(Some(W), "w:r")?;
        undo.push(Undo::Created { node: run });
        if let Some(rpr) = tree.child(runs[0], Some(W), "rPr")? {
            let run_props = copy_element(tree, rpr)?;
            undo.push(Undo::Created { node: run_props });
            tree.append(run, run_props)?;
            fresh_revision_ids(tree, run_props, ids)?;
        }
        for node in tree.parse_fragment(&run_text(with), &[("w", W)])? {
            undo.push(Undo::Created { node });
            tree.append(run, node)?;
        }
        tree.append(ins, run)?;
        tree.insert_after(del, ins)?;
    }
    Ok(())
}

/// Whether one child comes right after another with nothing between
/// them: runs, text, comments and processing instructions all hold a
/// deletion group apart.
fn adjacent_positions(kids: &[NodeId], first: NodeId, second: NodeId) -> bool {
    match (
        kids.iter().position(|&k| k == first),
        kids.iter().position(|&k| k == second),
    ) {
        (Some(a), Some(b)) => b == a + 1,
        _ => false,
    }
}

/// Wraps runs in a deletion where they stand and renames their text to
/// deleted text, returning the deletion. The runs share one parent
/// with nothing between them.
fn wrap_deletion(
    tree: &mut Tree,
    runs: &[NodeId],
    rev: &Track,
    ids: &mut Ids,
    undo: &mut Vec<Undo>,
) -> Result<NodeId, Error> {
    let del = tree.create_element(Some(W), "w:del")?;
    undo.push(Undo::Created { node: del });
    set_revision(tree, del, rev, ids)?;
    tree.insert_before(runs[0], del)?;
    for &run in runs {
        // Deleted text reads back as deleted text.
        let mut texts: Vec<NodeId> = Vec::new();
        for node in tree.descendants(run)? {
            if tree.is(node, Some(W), "t")? {
                texts.push(node);
            }
        }
        for text in texts {
            let (namespace, prefix, local, renamed) = tree.saved_name(text)?;
            undo.push(Undo::Renamed {
                node: text,
                namespace,
                prefix,
                local,
                renamed,
            });
            tree.set_name(text, Some(W), "w:delText")?;
        }
        record_move(tree, undo, run)?;
        tree.append(del, run)?;
    }
    Ok(del)
}

/// Stamps a revision element with a fresh id, the author and the date.
fn set_revision(tree: &mut Tree, node: NodeId, rev: &Track, ids: &mut Ids) -> Result<(), Error> {
    tree.set_attribute(node, Some(W), "w:id", &ids.revision().to_string())?;
    tree.set_attribute(node, Some(W), "w:author", &rev.author)?;
    tree.set_attribute(node, Some(W), "w:date", &rev.date)?;
    Ok(())
}

/// Accepts or rejects tracked revisions and reports how many records
/// settled.
///
/// Every story is walked: insertions, deletions and moves of text that
/// belong to `author` — or to anyone, when no author is given — are
/// accepted or rejected. Accepting unwraps an insertion and removes a
/// deletion; rejecting removes an insertion and unwraps a deletion,
/// turning deleted text back into text. Accepting a move keeps the
/// destination text and removes the source; rejecting keeps the source
/// and removes the destination, with the range markers going either
/// way. A move whose mate is missing stays, as do property marks and
/// records in properties, such as paragraph-mark revisions. A record
/// inside another settles with the outer one first: dropping the outer
/// record drops what it holds.
pub fn settle(doc: &mut Document, accept: bool, author: Option<&str>) -> Result<usize, Error> {
    use std::collections::HashSet;
    let stories = place::stories(doc)?;
    let mut settled = 0usize;
    for (part, _) in &stories {
        let tree = doc.xml(part)?;
        let root = tree.root_element();
        // Move ids already settled in this part, so a mate reached
        // through a stale child list is not settled twice.
        let mut moves: HashSet<String> = HashSet::new();
        let kids: Vec<NodeId> = tree.children(root)?.collect();
        for kid in kids {
            settle_node(tree, root, kid, accept, author, &mut settled, &mut moves)?;
        }
    }
    Ok(settled)
}

/// Settles one node and, when it survives unwrapped, what it holds.
fn settle_node(
    tree: &mut Tree,
    root: NodeId,
    node: NodeId,
    accept: bool,
    author: Option<&str>,
    settled: &mut usize,
    moves: &mut std::collections::HashSet<String>,
) -> Result<(), Error> {
    let Some(name) = tree.name(node)? else {
        return Ok(());
    };
    if name.namespace != Some(W) {
        return settle_children(tree, root, node, accept, author, settled, moves);
    }
    // Property records stay: only text revisions settle, and paragraph
    // marks with them are left for a later slice.
    if name.local.ends_with("Pr") {
        return Ok(());
    }
    if name.local == "moveFrom" {
        return settle_move(tree, root, node, accept, author, settled, moves);
    }
    if name.local == "moveTo" {
        // The source side settles the pair; a destination reached on its
        // own only settles what it holds. One whose pair already settled
        // is gone from the tree: leave it alone.
        if let Some(id) = tree.attribute(node, Some(W), "id")?
            && moves.contains(id.as_ref())
        {
            return Ok(());
        }
        return settle_children(tree, root, node, accept, author, settled, moves);
    }
    if matches!(name.local, "ins" | "del") {
        // An empty or missing author never matches a named filter, as in
        // foreign-revision detection; without a filter everything settles.
        let mine = match &author {
            None => true,
            Some(want) => revision_author(tree, node)?.as_deref() == Some(*want),
        };
        if !mine {
            return settle_children(tree, root, node, accept, author, settled, moves);
        }
        if name.local == "ins" && accept || name.local == "del" && !accept {
            // Unwrap: the content stays, the record goes.
            if name.local == "del" {
                restore_deleted(tree, node)?;
            }
            let moved: Vec<NodeId> = tree.children(node)?.collect();
            for child in &moved {
                tree.insert_before(node, *child)?;
            }
            tree.detach(node)?;
            for child in moved {
                settle_node(tree, root, child, accept, author, settled, moves)?;
            }
            *settled += 1;
            return Ok(());
        }
        // Drop: the content goes with the record.
        tree.detach(node)?;
        *settled += 1;
        return Ok(());
    }
    settle_children(tree, root, node, accept, author, settled, moves)
}

/// Settles the children of a node that itself stands.
fn settle_children(
    tree: &mut Tree,
    root: NodeId,
    node: NodeId,
    accept: bool,
    author: Option<&str>,
    settled: &mut usize,
    moves: &mut std::collections::HashSet<String>,
) -> Result<(), Error> {
    let kids: Vec<NodeId> = tree.children(node)?.collect();
    for kid in kids {
        settle_node(tree, root, kid, accept, author, settled, moves)?;
    }
    Ok(())
}

/// Settles a move from its source record: the pair keeps one side and
/// drops the other, and the range markers go either way. Accepting
/// keeps the destination text; rejecting restores the source text. A
/// move without a mate in this part stays, as does one belonging to
/// another author — though what they hold still settles.
fn settle_move(
    tree: &mut Tree,
    root: NodeId,
    node: NodeId,
    accept: bool,
    author: Option<&str>,
    settled: &mut usize,
    moves: &mut std::collections::HashSet<String>,
) -> Result<(), Error> {
    let id = tree
        .attribute(node, Some(W), "id")?
        .map(|id| id.into_owned())
        .filter(|id| !id.is_empty());
    let Some(id) = id else {
        return settle_children(tree, root, node, accept, author, settled, moves);
    };
    if moves.contains(&id) {
        return Ok(());
    }
    let mate = find_mate(tree, root, node, &id)?;
    let Some(mate) = mate else {
        // Half a move cannot settle: leave the record, settle inside.
        return settle_children(tree, root, node, accept, author, settled, moves);
    };
    let mine = match &author {
        None => true,
        Some(want) => {
            revision_author(tree, node)?.as_deref() == Some(*want)
                || revision_author(tree, mate)?.as_deref() == Some(*want)
        }
    };
    if !mine {
        return settle_children(tree, root, node, accept, author, settled, moves);
    }
    // The kept side unwraps where it stands and its children settle in
    // turn; the dropped side goes with its record.
    let (keep, drop) = if accept { (mate, node) } else { (node, mate) };
    let keep_side = if accept { "moveTo" } else { "moveFrom" };
    remove_markers(tree, root, keep_side, &id)?;
    let kept: Vec<NodeId> = tree.children(keep)?.collect();
    for child in &kept {
        tree.insert_before(keep, *child)?;
    }
    tree.detach(keep)?;
    for child in kept {
        settle_node(tree, root, child, accept, author, settled, moves)?;
    }
    let drop_side = if accept { "moveFrom" } else { "moveTo" };
    remove_markers(tree, root, drop_side, &id)?;
    tree.detach(drop)?;
    moves.insert(id);
    *settled += 2;
    Ok(())
}

/// The mate of a move record: the attached record on the other side
/// with the same id. Detached records never match, so a half-settled
/// or half-dropped pair leaves its survivor alone.
fn find_mate(tree: &Tree, root: NodeId, node: NodeId, id: &str) -> Result<Option<NodeId>, Error> {
    let want = tree
        .name(node)?
        .map(|n| n.local == "moveFrom")
        .unwrap_or(false);
    let want = if want { "moveTo" } else { "moveFrom" };
    for candidate in tree.descendants(root)? {
        if candidate == node {
            continue;
        }
        let name = tree.name(candidate)?;
        if !name.is_some_and(|n| n.namespace == Some(W) && n.local == want) {
            continue;
        }
        if tree
            .attribute(candidate, Some(W), "id")?
            .is_some_and(|other| other.as_ref() == id)
        {
            return Ok(Some(candidate));
        }
    }
    Ok(None)
}

/// Detaches the range markers of one side of a move: the start and end
/// elements carrying the move id. The records themselves are unwrapped
/// or dropped by the caller.
fn remove_markers(tree: &mut Tree, root: NodeId, side: &str, id: &str) -> Result<(), Error> {
    let (start, end) = match side {
        "moveFrom" => ("moveFromRangeStart", "moveFromRangeEnd"),
        _ => ("moveToRangeStart", "moveToRangeEnd"),
    };
    let mut markers = Vec::new();
    for candidate in tree.descendants(root)? {
        let name = tree.name(candidate)?;
        if !name.is_some_and(|n| n.namespace == Some(W) && (n.local == start || n.local == end)) {
            continue;
        }
        if tree
            .attribute(candidate, Some(W), "id")?
            .is_some_and(|other| other.as_ref() == id)
        {
            markers.push(candidate);
        }
    }
    for marker in markers {
        tree.detach(marker)?;
    }
    Ok(())
}

/// Turns deleted text back into text for a rejected deletion. Nested
/// deletions and moves keep their own records: settlement handles each
/// surviving revision separately.
fn restore_deleted(tree: &mut Tree, del: NodeId) -> Result<(), Error> {
    let kids: Vec<NodeId> = tree.children(del)?.collect();
    for kid in kids {
        restore_in(tree, kid)?;
    }
    Ok(())
}

/// Restores deleted text below one node, stopping at nested deletions
/// and moves.
fn restore_in(tree: &mut Tree, node: NodeId) -> Result<(), Error> {
    let name = tree.name(node)?;
    if name
        .is_some_and(|n| n.namespace == Some(W) && matches!(n.local, "del" | "moveFrom" | "moveTo"))
    {
        return Ok(());
    }
    if tree.is(node, Some(W), "delText")? {
        tree.set_name(node, Some(W), "w:t")?;
    } else if tree.is(node, Some(W), "delInstrText")? {
        tree.set_name(node, Some(W), "w:instrText")?;
    }
    let kids: Vec<NodeId> = tree.children(node)?.collect();
    for kid in kids {
        restore_in(tree, kid)?;
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
fn split_text(
    tree: &mut Tree,
    node: NodeId,
    text: &str,
    at: usize,
    undo: &mut Vec<Undo>,
) -> Result<NodeId, Error> {
    let (head, tail) = text.split_at(at);
    undo.push(Undo::Children {
        node,
        old: tree.children(node)?.collect(),
    });
    tree.set_text(node, head)?;
    if (head.starts_with(' ') || head.ends_with(' '))
        && tree.attribute(node, Some(XML_NS), "space")?.is_none()
    {
        undo.push(Undo::Space { node, old: None });
        tree.set_attribute(node, Some(XML_NS), "xml:space", "preserve")?;
    }
    let mut tail_node = None;
    for n in tree.parse_fragment("<w:t/>", &[("w", W)])? {
        tail_node = Some(n);
    }
    let tail_node =
        tail_node.ok_or_else(|| Error::Invalid("cannot split a text node".to_string()))?;
    undo.push(Undo::Created { node: tail_node });
    tree.set_text(tail_node, tail)?;
    if tail.starts_with(' ') || tail.ends_with(' ') {
        tree.set_attribute(tail_node, Some(XML_NS), "xml:space", "preserve")?;
    }
    tree.insert_after(node, tail_node)?;
    Ok(tail_node)
}
