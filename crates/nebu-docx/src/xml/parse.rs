//! The parser: bytes to nodes with source spans.
//!
//! It checks well-formedness as XML 1.0 with namespaces defines it, so that
//! every span it records can later be read again without errors. The work is
//! one pass over the text with no recursion; nesting depth and node count
//! are limited by [`Limits`].

use std::ops::Range;

use super::chars::{self, is_space};
use super::{
    Data, Element, Encoding, NodeId, Scope, ScopeId, Span, Tree, UriId, XML_NS, XMLNS_NS, XMLNS_URI,
};
use crate::{Error, Limits};

/// Turns the bytes of a part into text, by its byte order mark or, without
/// one, by how its first characters are encoded.
pub(super) fn decode_bytes(bytes: Vec<u8>) -> Result<(String, Encoding), Error> {
    let utf16 = |big_endian: bool, bom: bool, b: &[u8]| -> Result<(String, Encoding), Error> {
        if !b.len().is_multiple_of(2) {
            return Err(syntax_at_byte(
                b.len(),
                "the part is UTF-16 but has an odd number of bytes",
            ));
        }
        let units = b.as_chunks::<2>().0.iter().map(|c| {
            if big_endian {
                u16::from_be_bytes([c[0], c[1]])
            } else {
                u16::from_le_bytes([c[0], c[1]])
            }
        });
        let mut s = String::with_capacity(b.len() / 2);
        for (i, c) in char::decode_utf16(units).enumerate() {
            match c {
                Ok(c) => s.push(c),
                Err(_) => {
                    return Err(syntax_at_byte(
                        i * 2,
                        "the UTF-16 text has an unpaired surrogate",
                    ));
                }
            }
        }
        Ok((s, Encoding::Utf16 { big_endian, bom }))
    };
    match bytes.as_slice() {
        [0xFF, 0xFE, rest @ ..] => utf16(false, true, rest),
        [0xFE, 0xFF, rest @ ..] => utf16(true, true, rest),
        [0x3C, 0x00, 0x3F, 0x00, ..] => utf16(false, false, &bytes),
        [0x00, 0x3C, 0x00, 0x3F, ..] => utf16(true, false, &bytes),
        _ => {
            let (bom, mut bytes) = match bytes.strip_prefix(b"\xEF\xBB\xBF") {
                Some(_) => (true, bytes),
                None => (false, bytes),
            };
            if bom {
                bytes.drain(..3);
            }
            match String::from_utf8(bytes) {
                Ok(s) => Ok((s, Encoding::Utf8 { bom })),
                Err(e) => {
                    let at = e.utf8_error().valid_up_to();
                    Err(syntax_at_byte(at, "the part is not valid UTF-8"))
                }
            }
        }
    }
}

fn syntax_at_byte(offset: usize, message: &str) -> Error {
    Error::Xml {
        part: None,
        line: 0,
        column: 0,
        message: format!("{message} (at byte {offset})"),
    }
}

/// A syntax error at a byte offset of the decoded text.
fn syntax(text: &str, offset: usize, message: impl Into<String>) -> Error {
    let offset = offset.min(text.len());
    let mut at = offset;
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    let before = &text[..at];
    let line = before.bytes().filter(|&b| b == b'\n').count() + 1;
    let column = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
    Error::Xml {
        part: None,
        line,
        column,
        message: message.into(),
    }
}

/// One attribute of a start tag, as ranges into the tag's text.
pub(super) struct RawAttr {
    /// From the white space before the name to the closing quote.
    pub raw: Range<usize>,
    pub name: Range<usize>,
    /// Between the quotes.
    pub value: Range<usize>,
    pub quote: u8,
}

/// The end (exclusive) of a name that starts at `at`, or `at` if there is
/// none. Colons are accepted here and checked by the caller.
pub(super) fn name_end(s: &str, at: usize) -> usize {
    let b = s.as_bytes();
    let mut i = at;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii() {
            let ok = c.is_ascii_alphabetic()
                || c == b'_'
                || c == b':'
                || (i > at && (c.is_ascii_digit() || c == b'-' || c == b'.'));
            if !ok {
                break;
            }
            i += 1;
        } else {
            let Some(ch) = s[i..].chars().next() else {
                break;
            };
            let ok = if i == at {
                chars::is_name_start(ch)
            } else {
                chars::is_name_char(ch)
            };
            if !ok {
                break;
            }
            i += ch.len_utf8();
        }
    }
    i
}

/// Reads the next attribute after `*pos` in a start tag. Returns `None`,
/// with `*pos` unchanged, when what follows the white space is not a name:
/// the end of the tag, or an error the caller reports.
fn next_attr(s: &str, pos: &mut usize) -> Result<Option<RawAttr>, (usize, String)> {
    let b = s.as_bytes();
    let start = *pos;
    let mut i = start;
    while i < b.len() && is_space(b[i]) {
        i += 1;
    }
    let ne = name_end(s, i);
    if ne == i {
        return Ok(None);
    }
    if i == start {
        return Err((i, "attributes must be separated by white space".into()));
    }
    let name = i..ne;
    i = ne;
    while i < b.len() && is_space(b[i]) {
        i += 1;
    }
    if b.get(i) != Some(&b'=') {
        return Err((
            i,
            format!("expected '=' after the attribute name {}", &s[name]),
        ));
    }
    i += 1;
    while i < b.len() && is_space(b[i]) {
        i += 1;
    }
    let quote = match b.get(i) {
        Some(&q @ (b'"' | b'\'')) => q,
        _ => return Err((i, format!("the value of {} must be in quotes", &s[name]))),
    };
    let vs = i + 1;
    let Some(len) = b[vs..].iter().position(|&c| c == quote) else {
        return Err((vs, "the attribute value is not closed".into()));
    };
    let value = vs..vs + len;
    if let Some(k) = s[value.clone()].find('<') {
        return Err((
            vs + k,
            "'<' is not allowed in an attribute value (write &lt;)".into(),
        ));
    }
    *pos = vs + len + 1;
    Ok(Some(RawAttr {
        raw: start..*pos,
        name,
        value,
        quote,
    }))
}

/// The attributes of a start tag that the parser has already checked.
pub(super) fn source_attrs(tag: &str) -> impl Iterator<Item = RawAttr> + '_ {
    let mut pos = name_end(tag, 1);
    std::iter::from_fn(move || next_attr(tag, &mut pos).ok().flatten())
}

struct Open {
    id: NodeId,
    name: Range<usize>,
    scope: ScopeId,
}

/// Parses `text` and appends the nodes to `root` (a document), or returns
/// them unattached (a fragment). `base` is the scope the top-level nodes are
/// parsed in.
pub(super) fn parse(
    tree: &mut Tree,
    text: String,
    root: Option<NodeId>,
    base: ScopeId,
    fragment: bool,
    limits: &Limits,
) -> Result<Vec<NodeId>, Error> {
    let buf = tree.bufs.len() as u32;
    if text.len() >= u32::MAX as usize {
        return Err(Error::Limit {
            what: "an XML part of 4 GiB or more".into(),
            field: "max_part_size",
            limit: u32::MAX as u64,
        });
    }
    let s = text.as_str();
    let b = s.as_bytes();
    let n = b.len();
    let err = |at: usize, m: String| syntax(s, at, m);
    if let Some(at) = chars::first_invalid_char(s) {
        let c = s[at..].chars().next().unwrap_or('\0');
        return Err(err(
            at,
            format!("the character U+{:04X} is not allowed in XML", c as u32),
        ));
    }
    let sp = |start: usize, end: usize| {
        Some(Span {
            buf,
            start: start as u32,
            end: end as u32,
        })
    };
    let mut stack: Vec<Open> = vec![];
    let mut top: Vec<NodeId> = vec![];
    let mut root_seen = false;
    let mut attrs: Vec<RawAttr> = vec![];
    let mut keys: Vec<(Option<UriId>, &str, usize)> = vec![];
    let mut i = 0;

    let attach = |tree: &mut Tree, stack: &[Open], top: &mut Vec<NodeId>, id: NodeId| {
        let parent = stack.last().map(|o| o.id).or(root);
        match parent {
            Some(p) => {
                tree.nodes[id.index()].parent = Some(p);
                tree.nodes[p.index()].children.push(id);
            }
            None => top.push(id),
        }
    };
    let check_count = |tree: &Tree| -> Result<(), Error> {
        if tree.nodes.len() > limits.max_xml_nodes {
            return Err(Error::Limit {
                what: format!("an XML part with more than {} nodes", limits.max_xml_nodes),
                field: "max_xml_nodes",
                limit: limits.max_xml_nodes as u64,
            });
        }
        Ok(())
    };

    if !fragment && s.starts_with("<?xml") && b.get(5).is_some_and(|&c| is_space(c) || c == b'?') {
        let end = s
            .find("?>")
            .ok_or_else(|| err(0, "the XML declaration is not closed".into()))?
            + 2;
        check_declaration(s, end, tree.encoding)?;
        let id = tree.push(Data::Decl, sp(0, end));
        attach(tree, &stack, &mut top, id);
        i = end;
    }

    while i < n {
        let scope = stack.last().map_or(base, |o| o.scope);
        let outside = stack.is_empty() && !fragment;
        if b[i] != b'<' {
            let e = b[i..].iter().position(|&c| c == b'<').map_or(n, |k| i + k);
            let t = &s[i..e];
            if outside && !t.bytes().all(is_space) {
                let k = t.bytes().position(|c| !is_space(c)).unwrap_or(0);
                return Err(err(
                    i + k,
                    "text is not allowed outside the root element".into(),
                ));
            }
            chars::check_references(t).map_err(|(k, m)| err(i + k, m))?;
            if let Some(k) = t.find("]]>") {
                return Err(err(
                    i + k,
                    "']]>' is not allowed in text (write ]]&gt;)".into(),
                ));
            }
            let id = tree.push(Data::Text(None), sp(i, e));
            attach(tree, &stack, &mut top, id);
            check_count(tree)?;
            i = e;
            continue;
        }
        let rest = &s[i..];
        if rest.starts_with("<!--") {
            let body = i + 4;
            let k = s[body..]
                .find("--")
                .ok_or_else(|| err(i, "the comment is not closed".into()))?;
            if b.get(body + k + 2) != Some(&b'>') {
                return Err(err(body + k, "'--' is not allowed inside a comment".into()));
            }
            let e = body + k + 3;
            let id = tree.push(Data::Comment, sp(i, e));
            attach(tree, &stack, &mut top, id);
            i = e;
        } else if rest.starts_with("<![CDATA[") {
            if outside {
                return Err(err(
                    i,
                    "a CDATA section is not allowed outside the root element".into(),
                ));
            }
            let k = s[i + 9..]
                .find("]]>")
                .ok_or_else(|| err(i, "the CDATA section is not closed".into()))?;
            let e = i + 9 + k + 3;
            let id = tree.push(Data::Cdata, sp(i, e));
            attach(tree, &stack, &mut top, id);
            i = e;
        } else if rest.starts_with("<!DOCTYPE") {
            return Err(err(
                i,
                "document type declarations are not allowed in Office Open XML".into(),
            ));
        } else if rest.starts_with("<!") {
            return Err(err(i, "unexpected '<!'".into()));
        } else if rest.starts_with("<?") {
            let te = name_end(s, i + 2);
            let target = &s[i + 2..te];
            if target.is_empty() || target.contains(':') {
                return Err(err(
                    i + 2,
                    "a processing instruction needs a target name without ':'".into(),
                ));
            }
            if target.eq_ignore_ascii_case("xml") {
                return Err(err(
                    i,
                    "the XML declaration must come first, and only once".into(),
                ));
            }
            let e = s[te..]
                .find("?>")
                .ok_or_else(|| err(i, "the processing instruction is not closed".into()))?
                + te;
            if e > te && !is_space(b[te]) {
                return Err(err(
                    te,
                    "expected white space after the processing instruction's target".into(),
                ));
            }
            let id = tree.push(Data::Pi, sp(i, e + 2));
            attach(tree, &stack, &mut top, id);
            i = e + 2;
        } else if rest.starts_with("</") {
            let ne = name_end(s, i + 2);
            let mut e = ne;
            while e < n && is_space(b[e]) {
                e += 1;
            }
            if b.get(e) != Some(&b'>') {
                return Err(err(e, "expected '>' to end the end tag".into()));
            }
            let Some(open) = stack.pop() else {
                return Err(err(
                    i,
                    format!("the end tag </{}> has no start tag", &s[i + 2..ne]),
                ));
            };
            if s[open.name.clone()] != s[i + 2..ne] {
                return Err(err(
                    i,
                    format!("</{}> does not close <{}>", &s[i + 2..ne], &s[open.name]),
                ));
            }
            let node = &mut tree.nodes[open.id.index()];
            if let Data::Element(el) = &mut node.data {
                el.close = sp(i, e + 1);
            }
            if let Some(span) = &mut node.span {
                span.end = (e + 1) as u32;
            }
            i = e + 1;
        } else {
            // A start tag.
            let ne = name_end(s, i + 1);
            if ne == i + 1 {
                return Err(err(i + 1, "expected an element name after '<'".into()));
            }
            let qname = &s[i + 1..ne];
            let Some((prefix, _)) = chars::split_qname(qname) else {
                return Err(err(i + 1, format!("{qname} is not a valid element name")));
            };
            attrs.clear();
            let mut pos = ne;
            loop {
                match next_attr(s, &mut pos) {
                    Ok(Some(a)) => attrs.push(a),
                    Ok(None) => break,
                    Err((at, m)) => return Err(err(at, m)),
                }
            }
            let mut e = pos;
            while e < n && is_space(b[e]) {
                e += 1;
            }
            let self_closing = match (b.get(e), b.get(e + 1)) {
                (Some(b'>'), _) => false,
                (Some(b'/'), Some(b'>')) => true,
                _ => {
                    return Err(err(
                        e,
                        format!("expected '>' or '/>' to end the start tag <{qname}>"),
                    ));
                }
            };
            let end = if self_closing { e + 2 } else { e + 1 };

            // Namespace declarations first: they apply to the whole tag.
            let mut bindings: Vec<(Box<str>, Option<UriId>)> = vec![];
            keys.clear();
            for a in &attrs {
                let an = &s[a.name.clone()];
                let declared = if an == "xmlns" {
                    ""
                } else if let Some(p) = an.strip_prefix("xmlns:") {
                    if !chars::is_ncname(p) {
                        return Err(err(
                            a.name.start,
                            format!("{an} declares an invalid prefix"),
                        ));
                    }
                    p
                } else {
                    continue;
                };
                chars::check_references(&s[a.value.clone()])
                    .map_err(|(k, m)| err(a.value.start + k, m))?;
                let uri = chars::decode(&s[a.value.clone()], true);
                let at = a.name.start;
                if declared == "xmlns" || uri == XMLNS_NS {
                    return Err(err(
                        at,
                        "the xmlns prefix and namespace are reserved".to_string(),
                    ));
                }
                if (declared == "xml") != (uri == XML_NS) {
                    return Err(err(
                        at,
                        "the xml prefix and namespace belong together".to_string(),
                    ));
                }
                if uri.is_empty() && !declared.is_empty() {
                    return Err(err(
                        at,
                        format!("XML 1.0 cannot undeclare the prefix {declared}"),
                    ));
                }
                let u = (!uri.is_empty()).then(|| tree.intern_uri(&uri));
                keys.push((Some(XMLNS_URI), declared, at));
                bindings.push((declared.into(), u));
            }
            let inner = if bindings.is_empty() {
                scope
            } else {
                tree.push_scope(Scope::new(Some(scope), bindings))
            };
            let Some(ns) = tree.lookup(inner, prefix) else {
                return Err(err(i + 1, format!("the prefix {prefix} is not declared")));
            };

            // Attributes: prefixes bound, references valid, no duplicates.
            for a in &attrs {
                let an = &s[a.name.clone()];
                if an == "xmlns" || an.starts_with("xmlns:") {
                    continue;
                }
                let Some((ap, al)) = chars::split_qname(an) else {
                    return Err(err(
                        a.name.start,
                        format!("{an} is not a valid attribute name"),
                    ));
                };
                let ans = if ap.is_empty() {
                    None
                } else {
                    match tree.lookup(inner, ap) {
                        Some(u) => u,
                        None => {
                            return Err(err(
                                a.name.start,
                                format!("the prefix {ap} is not declared"),
                            ));
                        }
                    }
                };
                chars::check_references(&s[a.value.clone()])
                    .map_err(|(k, m)| err(a.value.start + k, m))?;
                keys.push((ans, al, a.name.start));
            }
            if keys.len() > 1 {
                keys.sort_unstable();
                if let Some(w) = keys
                    .windows(2)
                    .find(|w| w[0].0 == w[1].0 && w[0].1 == w[1].1)
                {
                    return Err(err(
                        w[1].2.max(w[0].2),
                        format!("<{qname}> has the same attribute twice"),
                    ));
                }
            }

            if outside {
                if root_seen {
                    return Err(err(i, "a document has only one root element".into()));
                }
                root_seen = true;
            }
            if stack.len() >= limits.max_depth {
                return Err(Error::Limit {
                    what: format!("XML nested more than {} levels deep", limits.max_depth),
                    field: "max_depth",
                    limit: limits.max_depth as u64,
                });
            }
            let name = tree.intern_qname(qname, prefix.len(), ns);
            let el = Element {
                name,
                renamed: false,
                scope: Some(inner),
                ctx: Some(scope),
                open: sp(i, end),
                close: None,
                attrs: None,
            };
            let id = tree.push(Data::Element(Box::new(el)), sp(i, end));
            attach(tree, &stack, &mut top, id);
            check_count(tree)?;
            if !self_closing {
                stack.push(Open {
                    id,
                    name: i + 1..ne,
                    scope: inner,
                });
            }
            i = end;
        }
    }
    if let Some(open) = stack.last() {
        return Err(err(n, format!("<{}> is not closed", &s[open.name.clone()])));
    }
    if !fragment && !root_seen {
        return Err(err(n, "the document has no root element".into()));
    }
    tree.bufs.push(text.into_boxed_str());
    Ok(top)
}

/// Checks the XML declaration `s[..end]`: version 1.0, an encoding that
/// matches the bytes, and a valid standalone flag, in that order.
fn check_declaration(s: &str, end: usize, encoding: Encoding) -> Result<(), Error> {
    let decl = &s[..end - 2];
    let mut pos = 5;
    let mut fields = vec![];
    loop {
        match next_attr(decl, &mut pos) {
            Ok(Some(a)) => fields.push(a),
            Ok(None) => break,
            Err((at, m)) => return Err(syntax(s, at, m)),
        }
    }
    if !decl[pos..].bytes().all(is_space) {
        return Err(syntax(s, pos, "unexpected text in the XML declaration"));
    }
    let mut want = ["version", "encoding", "standalone"].iter();
    for a in &fields {
        let name = &decl[a.name.clone()];
        if !want.any(|w| *w == name) {
            return Err(syntax(
                s,
                a.name.start,
                format!("unexpected {name} in the XML declaration"),
            ));
        }
        let value = &decl[a.value.clone()];
        match name {
            "version" if value == "1.0" => {}
            "version" if value.starts_with("1.") => {
                return Err(Error::Unsupported(format!(
                    "XML version {value}; Office Open XML uses version 1.0"
                )));
            }
            "version" => {
                return Err(syntax(
                    s,
                    a.value.start,
                    format!("{value} is not an XML version"),
                ));
            }
            "encoding" => {
                let v = value.to_ascii_lowercase();
                let ok = match encoding {
                    Encoding::Utf8 { .. } => v == "utf-8" || v == "utf8",
                    Encoding::Utf16 { .. } => v.starts_with("utf-16"),
                };
                if !ok {
                    return Err(Error::Unsupported(format!(
                        "the part declares the encoding {value} but is {}; Office Open XML allows UTF-8 and UTF-16",
                        match encoding {
                            Encoding::Utf8 { .. } => "UTF-8",
                            Encoding::Utf16 { .. } => "UTF-16",
                        }
                    )));
                }
            }
            "standalone" if value == "yes" || value == "no" => {}
            _ => {
                return Err(syntax(
                    s,
                    a.value.start,
                    format!("standalone must be yes or no, not {value}"),
                ));
            }
        }
    }
    if fields
        .first()
        .is_none_or(|a| &decl[a.name.clone()] != "version")
    {
        return Err(syntax(
            s,
            0,
            "the XML declaration must start with the version",
        ));
    }
    Ok(())
}
