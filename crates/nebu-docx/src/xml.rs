//! A lossless XML tree.
//!
//! Every node parsed from a part remembers the byte range it came from. When
//! the tree is written, a node that nobody changed is copied from those bytes,
//! so untouched content keeps its exact form: attribute order, quoting,
//! whitespace, namespace declarations and entity spelling. Only changed nodes
//! are serialised, and only as far down as the change reaches.
//!
//! Names are kept as written (`w:p`). [`Tree::ns_uri`] resolves a prefix
//! against the root's declarations so callers can check that `w` really is the
//! WordprocessingML namespace before they rely on it.

use std::fmt;
use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(u32);

impl NodeId {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Debug, Clone, Copy)]
struct Span {
    buf: u16,
    start: u32,
    end: u32,
}

impl Span {
    fn range(self) -> Range<usize> {
        self.start as usize..self.end as usize
    }
}

#[derive(Debug, Clone)]
pub enum Kind {
    /// The invisible root that holds the prolog and the document element.
    Document,
    Element(Element),
    Text(Option<String>),
    /// Comments, processing instructions, CDATA, declarations: copied as is.
    Raw,
}

#[derive(Debug, Clone)]
pub struct Element {
    pub name: Box<str>,
    /// `None` while the start tag is untouched and can be read from the source.
    attrs: Option<Vec<(Box<str>, String)>>,
    open: Option<Span>,
    close: Option<Span>,
}

#[derive(Debug, Clone)]
struct Node {
    kind: Kind,
    parent: Option<NodeId>,
    children: Vec<NodeId>,
    span: Option<Span>,
    dirty: bool,
}

#[derive(Debug)]
pub struct ParseError {
    pub offset: usize,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "XML error at byte {}: {}", self.offset, self.message)
    }
}

impl std::error::Error for ParseError {}

pub struct Tree {
    bufs: Vec<Vec<u8>>,
    nodes: Vec<Node>,
}

impl Tree {
    pub fn parse(src: Vec<u8>) -> Result<Tree, ParseError> {
        let mut t = Tree { bufs: vec![], nodes: vec![] };
        let root = t.push(Kind::Document, None, None);
        t.parse_into(src, root)?;
        t.nodes[root.index()].dirty = false;
        Ok(t)
    }

    pub fn root(&self) -> NodeId {
        NodeId(0)
    }

    /// The document element.
    pub fn document_element(&self) -> NodeId {
        self.children(self.root())
            .iter()
            .copied()
            .find(|&c| self.is_element(c))
            .expect("parsed XML has a document element")
    }

    fn push(&mut self, kind: Kind, parent: Option<NodeId>, span: Option<Span>) -> NodeId {
        let id = NodeId(self.nodes.len() as u32);
        self.nodes.push(Node { kind, parent, children: vec![], span, dirty: span.is_none() });
        if let Some(p) = parent {
            self.nodes[p.index()].children.push(id);
        }
        id
    }

    /// Parses `src` and appends its top-level nodes to `parent`. Returns them.
    fn parse_into(&mut self, src: Vec<u8>, parent: NodeId) -> Result<Vec<NodeId>, ParseError> {
        let buf = self.bufs.len() as u16;
        let first_new = self.nodes[parent.index()].children.len();
        let n = src.len();
        let mut stack: Vec<NodeId> = vec![parent];
        let mut i = 0usize;
        let err = |offset: usize, m: &str| ParseError { offset, message: m.to_string() };
        let sp = |s: usize, e: usize| Some(Span { buf, start: s as u32, end: e as u32 });
        while i < n {
            let top = *stack.last().unwrap();
            if src[i] != b'<' {
                let e = memchr(b'<', &src[i..]).map_or(n, |k| i + k);
                self.push(Kind::Text(None), Some(top), sp(i, e));
                i = e;
                continue;
            }
            let rest = &src[i..];
            if rest.starts_with(b"<!--") {
                let e = find(rest, b"-->").ok_or_else(|| err(i, "unclosed comment"))? + i + 3;
                self.push(Kind::Raw, Some(top), sp(i, e));
                i = e;
            } else if rest.starts_with(b"<![CDATA[") {
                let e = find(rest, b"]]>").ok_or_else(|| err(i, "unclosed CDATA"))? + i + 3;
                self.push(Kind::Raw, Some(top), sp(i, e));
                i = e;
            } else if rest.starts_with(b"<?") {
                let e = find(rest, b"?>").ok_or_else(|| err(i, "unclosed processing instruction"))? + i + 2;
                self.push(Kind::Raw, Some(top), sp(i, e));
                i = e;
            } else if rest.starts_with(b"<!") {
                let e = memchr(b'>', rest).ok_or_else(|| err(i, "unclosed declaration"))? + i + 1;
                self.push(Kind::Raw, Some(top), sp(i, e));
                i = e;
            } else if rest.starts_with(b"</") {
                let e = memchr(b'>', rest).ok_or_else(|| err(i, "unclosed end tag"))? + i + 1;
                let name = trim_ascii(&src[i + 2..e - 1]);
                if stack.len() < 2 {
                    return Err(err(i, "end tag without start tag"));
                }
                let open = stack.pop().unwrap();
                let node = &mut self.nodes[open.index()];
                let Kind::Element(el) = &mut node.kind else { unreachable!() };
                if el.name.as_bytes() != name {
                    return Err(err(i, &format!("</{}> closes <{}>", String::from_utf8_lossy(name), el.name)));
                }
                el.close = sp(i, e);
                let start = node.span.unwrap().start;
                node.span = Some(Span { buf, start, end: e as u32 });
                i = e;
            } else {
                let e = tag_end(rest).ok_or_else(|| err(i, "unclosed start tag"))? + i;
                let self_closing = src[e - 2] == b'/';
                let name_end = src[i + 1..e]
                    .iter()
                    .position(|&c| c.is_ascii_whitespace() || c == b'/' || c == b'>')
                    .map_or(e, |k| i + 1 + k);
                let name = std::str::from_utf8(&src[i + 1..name_end]).map_err(|_| err(i, "name is not UTF-8"))?;
                let el = Element { name: name.into(), attrs: None, open: sp(i, e), close: None };
                let id = self.push(Kind::Element(el), Some(top), sp(i, e));
                if !self_closing {
                    stack.push(id);
                }
                i = e;
            }
        }
        if stack.len() != 1 {
            return Err(err(n, "unclosed element"));
        }
        self.bufs.push(src);
        Ok(self.nodes[parent.index()].children[first_new..].to_vec())
    }

    /// Parses an XML fragment (one or more sibling nodes) without attaching it.
    /// The nodes keep their source bytes, so they are written verbatim.
    pub fn parse_fragment(&mut self, xml: &str) -> Result<Vec<NodeId>, ParseError> {
        let holder = self.push(Kind::Document, None, None);
        let ids = self.parse_into(xml.as_bytes().to_vec(), holder)?;
        for &c in &ids {
            self.nodes[c.index()].parent = None;
        }
        self.nodes[holder.index()].children.clear();
        Ok(ids)
    }

    // ---- reading ----------------------------------------------------------

    pub fn kind(&self, id: NodeId) -> &Kind {
        &self.nodes[id.index()].kind
    }

    pub fn is_element(&self, id: NodeId) -> bool {
        matches!(self.nodes[id.index()].kind, Kind::Element(_))
    }

    pub fn name(&self, id: NodeId) -> &str {
        match &self.nodes[id.index()].kind {
            Kind::Element(e) => &e.name,
            _ => "",
        }
    }

    pub fn is(&self, id: NodeId, name: &str) -> bool {
        self.name(id) == name
    }

    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.nodes[id.index()].parent
    }

    pub fn children(&self, id: NodeId) -> &[NodeId] {
        &self.nodes[id.index()].children
    }

    pub fn elements(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        self.children(id).iter().copied().filter(|&c| self.is_element(c))
    }

    pub fn child(&self, id: NodeId, name: &str) -> Option<NodeId> {
        self.children(id).iter().copied().find(|&c| self.is(c, name))
    }

    /// All descendants in document order, not including `id`.
    pub fn descendants(&self, id: NodeId) -> Descendants<'_> {
        Descendants { tree: self, stack: self.children(id).iter().rev().copied().collect() }
    }

    pub fn ancestors(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        std::iter::successors(self.parent(id), move |&p| self.parent(p))
    }

    fn src(&self, s: Span) -> &[u8] {
        &self.bufs[s.buf as usize][s.range()]
    }

    /// Verbatim source bytes of a node that has not changed.
    pub fn source(&self, id: NodeId) -> Option<&[u8]> {
        let n = &self.nodes[id.index()];
        if n.dirty { None } else { n.span.map(|s| self.src(s)) }
    }

    pub fn attrs(&self, id: NodeId) -> Vec<(String, String)> {
        match &self.nodes[id.index()].kind {
            Kind::Element(e) => match &e.attrs {
                Some(a) => a.iter().map(|(k, v)| (k.to_string(), v.clone())).collect(),
                None => parse_attrs(self.src(e.open.unwrap())),
            },
            _ => vec![],
        }
    }

    pub fn attr(&self, id: NodeId, name: &str) -> Option<String> {
        match &self.nodes[id.index()].kind {
            Kind::Element(e) => match &e.attrs {
                Some(a) => a.iter().find(|(k, _)| &**k == name).map(|(_, v)| v.clone()),
                None => find_attr(self.src(e.open.unwrap()), name),
            },
            _ => None,
        }
    }

    /// Decoded text of a text node.
    pub fn text(&self, id: NodeId) -> Option<String> {
        let n = &self.nodes[id.index()];
        match &n.kind {
            Kind::Text(Some(s)) => Some(s.clone()),
            Kind::Text(None) => Some(unescape(self.src(n.span.unwrap()))),
            _ => None,
        }
    }

    /// Concatenated text of all text nodes below `id`.
    pub fn inner_text(&self, id: NodeId) -> String {
        let mut s = String::new();
        for d in self.descendants(id) {
            if let Some(t) = self.text(d) {
                s.push_str(&t);
            }
        }
        s
    }

    /// Resolves a namespace prefix declared on the document element.
    pub fn ns_uri(&self, prefix: &str) -> Option<String> {
        let key = if prefix.is_empty() { "xmlns".to_string() } else { format!("xmlns:{prefix}") };
        self.attr(self.document_element(), &key)
    }

    // ---- editing ----------------------------------------------------------

    /// Marks a node and its ancestors as changed. An attached node that is
    /// already dirty has dirty ancestors, so the walk can stop there.
    fn touch(&mut self, id: NodeId) {
        let mut cur = Some(id);
        while let Some(c) = cur {
            if c != id && self.nodes[c.index()].dirty {
                break;
            }
            self.nodes[c.index()].dirty = true;
            cur = self.nodes[c.index()].parent;
        }
    }

    pub fn new_element(&mut self, name: &str, attrs: &[(&str, &str)]) -> NodeId {
        let el = Element {
            name: name.into(),
            attrs: Some(attrs.iter().map(|(k, v)| ((*k).into(), (*v).to_string())).collect()),
            open: None,
            close: None,
        };
        self.push(Kind::Element(el), None, None)
    }

    pub fn new_text(&mut self, text: &str) -> NodeId {
        self.push(Kind::Text(Some(text.to_string())), None, None)
    }

    pub fn set_attr(&mut self, id: NodeId, name: &str, value: &str) {
        let attrs = self.attrs(id);
        let Kind::Element(e) = &mut self.nodes[id.index()].kind else { return };
        let mut a: Vec<(Box<str>, String)> = attrs.into_iter().map(|(k, v)| (k.into(), v)).collect();
        match a.iter_mut().find(|(k, _)| &**k == name) {
            Some(slot) => slot.1 = value.to_string(),
            None => a.push((name.into(), value.to_string())),
        }
        e.attrs = Some(a);
        self.touch(id);
    }

    pub fn remove_attr(&mut self, id: NodeId, name: &str) {
        let attrs = self.attrs(id);
        if !attrs.iter().any(|(k, _)| k == name) {
            return;
        }
        let Kind::Element(e) = &mut self.nodes[id.index()].kind else { return };
        e.attrs = Some(attrs.into_iter().filter(|(k, _)| k != name).map(|(k, v)| (k.into(), v)).collect());
        self.touch(id);
    }

    /// Renames an element, for example `w:t` to `w:delText`.
    pub fn rename(&mut self, id: NodeId, name: &str) {
        let attrs = self.attrs(id);
        let Kind::Element(e) = &mut self.nodes[id.index()].kind else { return };
        e.name = name.into();
        e.attrs = Some(attrs.into_iter().map(|(k, v)| (k.into(), v)).collect());
        e.open = None;
        e.close = None;
        self.touch(id);
    }

    pub fn set_text(&mut self, id: NodeId, text: &str) {
        let kids = self.children(id).to_vec();
        if matches!(self.nodes[id.index()].kind, Kind::Text(_)) {
            self.nodes[id.index()].kind = Kind::Text(Some(text.to_string()));
            self.touch(id);
            return;
        }
        for k in kids {
            self.detach(k);
        }
        let t = self.new_text(text);
        self.append(id, t);
    }

    pub fn detach(&mut self, id: NodeId) {
        if let Some(p) = self.nodes[id.index()].parent.take() {
            self.nodes[p.index()].children.retain(|&c| c != id);
            self.touch(p);
        }
    }

    pub fn append(&mut self, parent: NodeId, child: NodeId) {
        self.detach(child);
        self.nodes[child.index()].parent = Some(parent);
        self.nodes[parent.index()].children.push(child);
        self.touch(parent);
    }

    pub fn insert_at(&mut self, parent: NodeId, index: usize, child: NodeId) {
        self.detach(child);
        self.nodes[child.index()].parent = Some(parent);
        self.nodes[parent.index()].children.insert(index, child);
        self.touch(parent);
    }

    pub fn index_in_parent(&self, id: NodeId) -> Option<usize> {
        let p = self.parent(id)?;
        self.children(p).iter().position(|&c| c == id)
    }

    pub fn insert_before(&mut self, anchor: NodeId, child: NodeId) {
        let p = self.parent(anchor).expect("anchor has a parent");
        self.detach(child);
        let i = self.index_in_parent(anchor).unwrap();
        self.insert_at(p, i, child);
    }

    pub fn insert_after(&mut self, anchor: NodeId, child: NodeId) {
        let p = self.parent(anchor).expect("anchor has a parent");
        self.detach(child);
        let i = self.index_in_parent(anchor).unwrap();
        self.insert_at(p, i + 1, child);
    }

    /// Puts `wrapper` where `id` was and moves `id` into it.
    pub fn wrap(&mut self, id: NodeId, wrapper: NodeId) {
        self.insert_before(id, wrapper);
        self.append(wrapper, id);
    }

    /// Deep copy. The copy keeps source spans, so it is still written verbatim.
    pub fn deep_clone(&mut self, id: NodeId) -> NodeId {
        let n = self.nodes[id.index()].clone();
        let kids = n.children.clone();
        let new = NodeId(self.nodes.len() as u32);
        self.nodes.push(Node { kind: n.kind, parent: None, children: vec![], span: n.span, dirty: n.dirty });
        for k in kids {
            let c = self.deep_clone(k);
            self.nodes[c.index()].parent = Some(new);
            self.nodes[new.index()].children.push(c);
        }
        new
    }

    // ---- writing ----------------------------------------------------------

    pub fn write(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.bufs.first().map_or(1024, |b| b.len() + 1024));
        for &c in self.children(self.root()) {
            self.write_node(c, &mut out);
        }
        out
    }

    pub fn write_node_to(&self, id: NodeId, out: &mut Vec<u8>) {
        self.write_node(id, out)
    }

    pub fn node_xml(&self, id: NodeId) -> String {
        let mut v = vec![];
        self.write_node(id, &mut v);
        String::from_utf8(v).unwrap_or_default()
    }

    /// True if nothing below the root changed since parsing.
    pub fn is_pristine(&self) -> bool {
        !self.nodes[0].dirty
    }

    /// Changed nodes, for diagnostics and tests.
    pub fn dirty_count(&self) -> usize {
        self.nodes.iter().filter(|n| n.dirty && n.parent.is_some()).count()
    }

    fn write_node(&self, id: NodeId, out: &mut Vec<u8>) {
        let n = &self.nodes[id.index()];
        if !n.dirty && !matches!(n.kind, Kind::Document) {
            if let Some(s) = n.span {
                out.extend_from_slice(self.src(s));
                return;
            }
        }
        match &n.kind {
            Kind::Document => {
                for &c in &n.children {
                    self.write_node(c, out);
                }
            }
            Kind::Raw => out.extend_from_slice(self.src(n.span.unwrap())),
            Kind::Text(Some(s)) => escape_text(s, out),
            Kind::Text(None) => out.extend_from_slice(self.src(n.span.unwrap())),
            Kind::Element(e) => {
                let empty = n.children.is_empty();
                match (&e.attrs, e.open) {
                    (None, Some(open)) => {
                        let tag = self.src(open);
                        let was_empty = tag.ends_with(b"/>");
                        if was_empty && !empty {
                            out.extend_from_slice(trim_end_ascii(&tag[..tag.len() - 2]));
                            out.push(b'>');
                        } else {
                            out.extend_from_slice(tag);
                            if was_empty {
                                return;
                            }
                        }
                    }
                    _ => {
                        out.push(b'<');
                        out.extend_from_slice(e.name.as_bytes());
                        for (k, v) in self.attrs(id) {
                            out.push(b' ');
                            out.extend_from_slice(k.as_bytes());
                            out.extend_from_slice(b"=\"");
                            escape_attr(&v, out);
                            out.push(b'"');
                        }
                        if empty {
                            out.extend_from_slice(b"/>");
                            return;
                        }
                        out.push(b'>');
                    }
                }
                for &c in &n.children {
                    self.write_node(c, out);
                }
                match e.close {
                    Some(c) if e.attrs.is_none() => out.extend_from_slice(self.src(c)),
                    _ => {
                        out.extend_from_slice(b"</");
                        out.extend_from_slice(e.name.as_bytes());
                        out.push(b'>');
                    }
                }
            }
        }
    }
}

pub struct Descendants<'a> {
    tree: &'a Tree,
    stack: Vec<NodeId>,
}

impl Iterator for Descendants<'_> {
    type Item = NodeId;
    fn next(&mut self) -> Option<NodeId> {
        let id = self.stack.pop()?;
        self.stack.extend(self.tree.children(id).iter().rev().copied());
        Some(id)
    }
}

// ---- byte helpers ---------------------------------------------------------

fn memchr(c: u8, s: &[u8]) -> Option<usize> {
    s.iter().position(|&b| b == c)
}

fn find(h: &[u8], n: &[u8]) -> Option<usize> {
    h.windows(n.len()).position(|w| w == n)
}

fn trim_ascii(s: &[u8]) -> &[u8] {
    s.trim_ascii()
}

fn trim_end_ascii(s: &[u8]) -> &[u8] {
    s.trim_ascii_end()
}

/// End (exclusive) of a start tag, skipping `>` inside quoted values.
fn tag_end(s: &[u8]) -> Option<usize> {
    let mut q = 0u8;
    for (i, &c) in s.iter().enumerate().skip(1) {
        if q != 0 {
            if c == q {
                q = 0;
            }
        } else if c == b'"' || c == b'\'' {
            q = c;
        } else if c == b'>' {
            return Some(i + 1);
        }
    }
    None
}

fn parse_attrs(tag: &[u8]) -> Vec<(String, String)> {
    let mut out = vec![];
    let body = tag.strip_prefix(b"<").unwrap_or(tag);
    let mut i = body.iter().position(|c| c.is_ascii_whitespace() || *c == b'/' || *c == b'>').unwrap_or(body.len());
    while i < body.len() {
        while i < body.len() && body[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= body.len() || body[i] == b'/' || body[i] == b'>' {
            break;
        }
        let ks = i;
        while i < body.len() && body[i] != b'=' && !body[i].is_ascii_whitespace() {
            i += 1;
        }
        let key = String::from_utf8_lossy(&body[ks..i]).into_owned();
        while i < body.len() && (body[i] == b'=' || body[i].is_ascii_whitespace()) {
            i += 1;
        }
        let q = body[i];
        i += 1;
        let vs = i;
        while i < body.len() && body[i] != q {
            i += 1;
        }
        out.push((key, unescape(&body[vs..i])));
        i += 1;
    }
    out
}

fn find_attr(tag: &[u8], name: &str) -> Option<String> {
    // Fast path: most lookups miss, and a byte search is cheaper than parsing.
    let nb = name.as_bytes();
    let mut from = 0;
    while let Some(k) = find(&tag[from..], nb) {
        let at = from + k;
        let before = tag[at - 1];
        let after = tag.get(at + nb.len()).copied();
        if before.is_ascii_whitespace() && matches!(after, Some(b'=') | Some(b' ')) {
            return parse_attrs(tag).into_iter().find(|(k, _)| k == name).map(|(_, v)| v);
        }
        from = at + nb.len();
    }
    None
}

pub fn unescape(s: &[u8]) -> String {
    if !s.contains(&b'&') {
        return String::from_utf8_lossy(s).into_owned();
    }
    let s = String::from_utf8_lossy(s);
    let mut out = String::with_capacity(s.len());
    let mut rest = &*s;
    while let Some(k) = rest.find('&') {
        out.push_str(&rest[..k]);
        rest = &rest[k..];
        let Some(e) = rest.find(';') else { break };
        let ent = &rest[1..e];
        let ch = match ent {
            "lt" => Some('<'),
            "gt" => Some('>'),
            "amp" => Some('&'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ if ent.starts_with("#x") => u32::from_str_radix(&ent[2..], 16).ok().and_then(char::from_u32),
            _ if ent.starts_with('#') => ent[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[e + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

pub fn escape_text(s: &str, out: &mut Vec<u8>) {
    for c in s.bytes() {
        match c {
            b'&' => out.extend_from_slice(b"&amp;"),
            b'<' => out.extend_from_slice(b"&lt;"),
            b'>' => out.extend_from_slice(b"&gt;"),
            _ => out.push(c),
        }
    }
}

pub fn escape_attr(s: &str, out: &mut Vec<u8>) {
    for c in s.bytes() {
        match c {
            b'&' => out.extend_from_slice(b"&amp;"),
            b'<' => out.extend_from_slice(b"&lt;"),
            b'"' => out.extend_from_slice(b"&quot;"),
            _ => out.push(c),
        }
    }
}

pub fn esc(s: &str) -> String {
    let mut v = vec![];
    escape_text(s, &mut v);
    String::from_utf8(v).unwrap()
}

pub fn esc_attr(s: &str) -> String {
    let mut v = vec![];
    escape_attr(s, &mut v);
    String::from_utf8(v).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n<w:document xmlns:w=\"urn:w\"><w:body><w:p w:rsidR='00A1'><w:r><w:t xml:space=\"preserve\">a &amp; b </w:t></w:r></w:p><w:p/><!-- c --></w:body></w:document>";

    #[test]
    fn untouched_tree_writes_its_source() {
        let t = Tree::parse(SRC.as_bytes().to_vec()).unwrap();
        assert_eq!(t.write(), SRC.as_bytes());
    }

    #[test]
    fn edits_stay_local() {
        let mut t = Tree::parse(SRC.as_bytes().to_vec()).unwrap();
        let doc = t.document_element();
        let body = t.child(doc, "w:body").unwrap();
        let p2 = t.elements(body).nth(1).unwrap();
        let r = t.new_element("w:r", &[]);
        t.append(p2, r);
        let out = String::from_utf8(t.write()).unwrap();
        assert!(out.contains("<w:p w:rsidR='00A1'><w:r><w:t xml:space=\"preserve\">a &amp; b </w:t></w:r></w:p><w:p><w:r/></w:p>"));
        assert!(out.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n"));
    }

    #[test]
    fn text_and_attrs_decode() {
        let t = Tree::parse(SRC.as_bytes().to_vec()).unwrap();
        let body = t.child(t.document_element(), "w:body").unwrap();
        let p = t.elements(body).next().unwrap();
        assert_eq!(t.attr(p, "w:rsidR").as_deref(), Some("00A1"));
        assert_eq!(t.inner_text(p), "a & b ");
        assert_eq!(t.ns_uri("w").as_deref(), Some("urn:w"));
    }
}
