//! A lossless XML tree.
//!
//! [`Tree::parse`] records, for every node, the bytes it came from.
//! [`Tree::to_bytes`] copies those bytes for every node that did not change,
//! so an untouched node keeps its exact form: attribute order, quote style,
//! namespace declarations, references and white space. Only what an edit
//! touched is written anew, and only as far as the edit reaches. Changing
//! one attribute rewrites that attribute; inserting a child rewrites nothing
//! of its parent but the place where the child goes.
//!
//! Names are resolved against the namespace declarations in scope, so a
//! caller asks for `(ns::W, "p")` and finds the paragraph whatever prefix the
//! file binds to WordprocessingML. New elements and attributes are written
//! with a prefix that is bound to their namespace where they end up; if none
//! is, the writer declares one on the element.
//!
//! The parser accepts well-formed XML 1.0 with namespaces in UTF-8 or UTF-16,
//! which is what Office Open XML allows. It refuses document type
//! declarations, which Office Open XML forbids, and with them every entity
//! but the five predefined ones.
//!
//! ```
//! use nebu_docx::xml::Tree;
//! use nebu_docx::ns;
//!
//! let src = r#"<x:document xmlns:x="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><x:body/></x:document>"#;
//! let mut tree = Tree::parse(src)?;
//! let body = tree.child(tree.root_element(), Some(ns::W), "body").unwrap();
//! let p = tree.create_element(Some(ns::W), "w:p")?;
//! tree.append(body, p)?;
//! // The new paragraph takes the prefix the file uses.
//! assert!(String::from_utf8(tree.to_bytes()).unwrap().contains("<x:body><x:p/></x:body>"));
//! # Ok::<(), nebu_docx::Error>(())
//! ```

mod chars;
mod parse;
mod write;

use std::borrow::Cow;
use std::collections::HashMap;
use std::ops::Range;

use crate::{Error, Limits};

/// The namespace that the `xml` prefix is always bound to.
pub const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";
/// The namespace of namespace declarations (`xmlns`, `xmlns:p`).
pub const XMLNS_NS: &str = "http://www.w3.org/2000/xmlns/";

/// A node of a [`Tree`]. Ids are only meaningful for the tree that made them;
/// passing an id to another tree's methods may panic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(u32);

/// What a node is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    /// The invisible node that holds the prolog, the root element and what
    /// follows it. [`Tree::document`] returns it.
    Document,
    /// An element.
    Element,
    /// Character data.
    Text,
    /// A CDATA section.
    CData,
    /// A comment.
    Comment,
    /// A processing instruction.
    ProcessingInstruction,
    /// The XML declaration (`<?xml version="1.0" …?>`).
    Declaration,
}

/// The resolved name of an element or attribute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Name<'a> {
    /// The namespace, or `None` for a name in no namespace.
    pub namespace: Option<&'a str>,
    /// The prefix as written, empty for none.
    pub prefix: &'a str,
    /// The local part.
    pub local: &'a str,
}

/// An attribute of an element, namespace declarations excluded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attribute<'a> {
    /// The attribute's name.
    pub name: Name<'a>,
    /// The value with references decoded and white space normalised.
    pub value: Cow<'a, str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Span {
    buf: u32,
    start: u32,
    end: u32,
}

impl Span {
    fn range(self) -> Range<usize> {
        self.start as usize..self.end as usize
    }
}

type ScopeId = u32;
type UriId = u32;
type NameId = u32;

/// The namespace bindings an element declares, and the scope around it.
#[derive(Debug, Clone)]
struct Scope {
    parent: Option<ScopeId>,
    /// Prefix (empty for the default namespace) and namespace (`None` for
    /// `xmlns=""`).
    bindings: Vec<(Box<str>, Option<UriId>)>,
    /// For a scope with many bindings, so that a lookup stays cheap.
    index: Option<HashMap<Box<str>, Option<UriId>>>,
}

impl Scope {
    fn new(parent: Option<ScopeId>, bindings: Vec<(Box<str>, Option<UriId>)>) -> Scope {
        let index = (bindings.len() > 8).then(|| bindings.iter().cloned().collect());
        Scope {
            parent,
            bindings,
            index,
        }
    }

    fn get(&self, prefix: &str) -> Option<Option<UriId>> {
        match &self.index {
            Some(map) => map.get(prefix).copied(),
            None => self
                .bindings
                .iter()
                .find(|(p, _)| **p == *prefix)
                .map(|(_, u)| *u),
        }
    }
}

#[derive(Debug, Clone)]
struct NameEntry {
    prefix: Box<str>,
    local: Box<str>,
    ns: Option<UriId>,
}

#[derive(Debug, Clone)]
enum AttrValue {
    /// The value between the quotes in the source, still escaped.
    Source(Span),
    Owned(Box<str>),
}

#[derive(Debug, Clone)]
struct Attr {
    name: NameId,
    value: AttrValue,
    /// The attribute as written, leading white space included. Copied while
    /// the value is unchanged.
    raw: Option<Span>,
    quote: u8,
}

#[derive(Debug, Clone)]
struct Element {
    name: NameId,
    /// Set by [`Tree::set_name`]: the source name no longer applies.
    renamed: bool,
    /// The scope inside the element (its own declarations included). `None`
    /// for an element that was created, not parsed.
    scope: Option<ScopeId>,
    /// The scope the element was parsed in. A parsed element is copied as
    /// written only where the bindings it relies on still hold.
    ctx: Option<ScopeId>,
    open: Option<Span>,
    close: Option<Span>,
    /// `None` while the attributes can be read from the source start tag.
    attrs: Option<Vec<Attr>>,
}

#[derive(Debug, Clone)]
enum Data {
    Document,
    Element(Box<Element>),
    /// `None`: the text is the source span, still escaped.
    Text(Option<Box<str>>),
    Cdata,
    Comment,
    Pi,
    Decl,
}

/// The node's own markup must be written anew.
const CHANGED: u8 = 1;
/// Something inside the node changed; its span cannot be copied whole.
const DESCEND: u8 = 2;

#[derive(Debug, Clone)]
struct Node {
    parent: Option<NodeId>,
    children: Vec<NodeId>,
    data: Data,
    span: Option<Span>,
    flags: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Encoding {
    Utf8 { bom: bool },
    Utf16 { big_endian: bool, bom: bool },
}

/// A parsed XML part. See the [module documentation](self).
#[derive(Debug, Clone)]
pub struct Tree {
    bufs: Vec<Box<str>>,
    nodes: Vec<Node>,
    scopes: Vec<Scope>,
    names: Vec<NameEntry>,
    name_index: HashMap<Box<str>, HashMap<Option<UriId>, NameId>>,
    uris: Vec<Box<str>>,
    uri_index: HashMap<Box<str>, UriId>,
    encoding: Encoding,
}

const DOCUMENT: NodeId = NodeId(0);
const XML_URI: UriId = 0;
const XMLNS_URI: UriId = 1;

impl Tree {
    /// Parses a complete XML document with the default [`Limits`].
    ///
    /// The bytes may be UTF-8, with or without a byte order mark, or UTF-16
    /// with a byte order mark; [`to_bytes`](Self::to_bytes) writes the same
    /// encoding back.
    pub fn parse(bytes: impl Into<Vec<u8>>) -> Result<Tree, Error> {
        Tree::parse_limited(bytes.into(), &Limits::default())
    }

    pub(crate) fn parse_limited(bytes: Vec<u8>, limits: &Limits) -> Result<Tree, Error> {
        let (text, encoding) = parse::decode_bytes(bytes)?;
        let mut t = Tree {
            bufs: vec![],
            nodes: vec![],
            scopes: vec![Scope::new(None, vec![])],
            names: vec![],
            name_index: HashMap::new(),
            uris: vec![],
            uri_index: HashMap::new(),
            encoding,
        };
        t.intern_uri(XML_NS);
        t.intern_uri(XMLNS_NS);
        t.nodes.push(Node {
            parent: None,
            children: vec![],
            data: Data::Document,
            span: None,
            flags: 0,
        });
        parse::parse(&mut t, text, Some(DOCUMENT), 0, false, limits)?;
        Ok(t)
    }

    /// Parses an XML fragment, one or more sibling nodes, and returns them
    /// unattached, ready for [`append`](Self::append) or
    /// [`insert`](Self::insert). `namespaces` binds the prefixes the
    /// fragment uses, as `(prefix, namespace)` pairs, with an empty prefix
    /// for the default namespace.
    ///
    /// The fragment's nodes are written as parsed wherever the tree binds
    /// the same prefixes to the same namespaces, and with the tree's own
    /// prefixes where it does not.
    pub fn parse_fragment(
        &mut self,
        xml: &str,
        namespaces: &[(&str, &str)],
    ) -> Result<Vec<NodeId>, Error> {
        let mut bindings: Vec<(Box<str>, Option<UriId>)> = vec![];
        for &(p, uri) in namespaces {
            if !(p.is_empty() || chars::is_ncname(p))
                || p == "xmlns"
                || (p == "xml") != (uri == XML_NS)
            {
                return Err(Error::Edit(format!("cannot bind prefix {p:?} to {uri:?}")));
            }
            if uri.is_empty() || uri == XMLNS_NS {
                return Err(Error::Edit(format!("cannot bind prefix {p:?} to {uri:?}")));
            }
            let u = self.intern_uri(uri);
            bindings.push((p.into(), Some(u)));
        }
        let scope = self.push_scope(Scope::new(None, bindings));
        let (nodes, scopes, bufs) = (self.nodes.len(), self.scopes.len(), self.bufs.len());
        match parse::parse(self, xml.to_owned(), None, scope, true, &Limits::default()) {
            Ok(ids) => Ok(ids),
            Err(e) => {
                self.nodes.truncate(nodes);
                self.scopes.truncate(scopes);
                self.bufs.truncate(bufs);
                Err(e)
            }
        }
    }

    /// Writes the tree. A tree that nobody changed gives back exactly the
    /// bytes it was parsed from.
    pub fn to_bytes(&self) -> Vec<u8> {
        let text = write::write(self);
        match self.encoding {
            Encoding::Utf8 { bom } => {
                let mut out = Vec::with_capacity(text.len() + 3);
                if bom {
                    out.extend_from_slice(b"\xEF\xBB\xBF");
                }
                out.extend_from_slice(text.as_bytes());
                out
            }
            Encoding::Utf16 { big_endian, bom } => {
                let mut out = Vec::with_capacity(text.len() * 2 + 2);
                let units = bom.then_some(0xFEFF).into_iter().chain(text.encode_utf16());
                for u in units {
                    out.extend_from_slice(&if big_endian {
                        u.to_be_bytes()
                    } else {
                        u.to_le_bytes()
                    });
                }
                out
            }
        }
    }

    /// True if an edit reached the document since it was parsed.
    pub fn is_changed(&self) -> bool {
        self.nodes[DOCUMENT.index()].flags != 0
    }

    // ---- reading ----------------------------------------------------------

    /// The document node: the parent of the prolog and the root element.
    pub fn document(&self) -> NodeId {
        DOCUMENT
    }

    /// The root element.
    pub fn root_element(&self) -> NodeId {
        // A parsed document always has one; the parser refuses others.
        self.children(DOCUMENT)
            .iter()
            .copied()
            .find(|&c| self.is_element(c))
            .unwrap_or(DOCUMENT)
    }

    /// What the node is.
    pub fn kind(&self, id: NodeId) -> NodeKind {
        match self.node(id).data {
            Data::Document => NodeKind::Document,
            Data::Element(_) => NodeKind::Element,
            Data::Text(_) => NodeKind::Text,
            Data::Cdata => NodeKind::CData,
            Data::Comment => NodeKind::Comment,
            Data::Pi => NodeKind::ProcessingInstruction,
            Data::Decl => NodeKind::Declaration,
        }
    }

    /// True for an element.
    pub fn is_element(&self, id: NodeId) -> bool {
        matches!(self.node(id).data, Data::Element(_))
    }

    /// The name of an element, `None` for other nodes.
    pub fn name(&self, id: NodeId) -> Option<Name<'_>> {
        match &self.node(id).data {
            Data::Element(e) => Some(self.name_of(e.name)),
            _ => None,
        }
    }

    /// True if the node is an element with this namespace and local name.
    pub fn is(&self, id: NodeId, namespace: Option<&str>, local: &str) -> bool {
        match &self.node(id).data {
            Data::Element(e) => self.name_matches(e.name, namespace, local),
            _ => false,
        }
    }

    /// The parent, `None` for the document node and for unattached nodes.
    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.node(id).parent
    }

    /// The children in document order.
    pub fn children(&self, id: NodeId) -> &[NodeId] {
        &self.node(id).children
    }

    /// The child elements in document order.
    pub fn elements(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        self.children(id)
            .iter()
            .copied()
            .filter(|&c| self.is_element(c))
    }

    /// The first child element with this name.
    pub fn child(&self, id: NodeId, namespace: Option<&str>, local: &str) -> Option<NodeId> {
        self.children(id)
            .iter()
            .copied()
            .find(|&c| self.is(c, namespace, local))
    }

    /// All nodes below `id` in document order, `id` itself excluded.
    pub fn descendants(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        let mut stack: Vec<NodeId> = self.children(id).iter().rev().copied().collect();
        std::iter::from_fn(move || {
            let next = stack.pop()?;
            stack.extend(self.children(next).iter().rev().copied());
            Some(next)
        })
    }

    /// The parent, its parent and so on up to the document node.
    pub fn ancestors(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        std::iter::successors(self.parent(id), move |&p| self.parent(p))
    }

    /// The value of an attribute, found by namespace and local name.
    pub fn attribute(
        &self,
        id: NodeId,
        namespace: Option<&str>,
        local: &str,
    ) -> Option<Cow<'_, str>> {
        let Data::Element(e) = &self.node(id).data else {
            return None;
        };
        match &e.attrs {
            Some(attrs) => attrs
                .iter()
                .find(|a| self.name_matches(a.name, namespace, local))
                .map(|a| self.attr_value(a)),
            None => {
                let open = e.open?;
                let tag = self.src(open);
                // Most lookups miss; a substring search is cheaper than a scan.
                if !tag.contains(local) {
                    return None;
                }
                parse::source_attrs(tag)
                    .find(|a| {
                        let (prefix, l) =
                            chars::split_qname(&tag[a.name.clone()]).unwrap_or(("", ""));
                        l == local && self.attr_ns(e, prefix) == namespace
                    })
                    .map(|a| chars::decode(&tag[a.value], true))
            }
        }
    }

    /// The attributes of an element in source order, without namespace
    /// declarations. Empty for other nodes.
    pub fn attributes(&self, id: NodeId) -> Vec<Attribute<'_>> {
        let Data::Element(e) = &self.node(id).data else {
            return vec![];
        };
        match &e.attrs {
            Some(attrs) => attrs
                .iter()
                .filter(|a| !self.is_decl(a.name))
                .map(|a| Attribute {
                    name: self.name_of(a.name),
                    value: self.attr_value(a),
                })
                .collect(),
            None => {
                let Some(open) = e.open else { return vec![] };
                let tag = self.src(open);
                parse::source_attrs(tag)
                    .filter_map(|a| {
                        let (prefix, local) = chars::split_qname(&tag[a.name.clone()])?;
                        if (prefix.is_empty() && local == "xmlns") || prefix == "xmlns" {
                            return None;
                        }
                        let namespace = self.attr_ns(e, prefix);
                        Some(Attribute {
                            name: Name {
                                namespace,
                                prefix,
                                local,
                            },
                            value: chars::decode(&tag[a.value], true),
                        })
                    })
                    .collect()
            }
        }
    }

    /// The text of a text or CDATA node, references decoded and line ends
    /// normalised. `None` for other nodes.
    pub fn text(&self, id: NodeId) -> Option<Cow<'_, str>> {
        let n = self.node(id);
        match (&n.data, n.span) {
            (Data::Text(Some(s)), _) => Some(Cow::Borrowed(s)),
            (Data::Text(None), Some(s)) => Some(chars::decode(self.src(s), false)),
            (Data::Cdata, Some(s)) => {
                let raw = self.src(s);
                let inner = &raw[9..raw.len() - 3];
                Some(match inner.contains('\r') {
                    true => Cow::Owned(inner.replace("\r\n", "\n").replace('\r', "\n")),
                    false => Cow::Borrowed(inner),
                })
            }
            _ => None,
        }
    }

    /// The text of all text and CDATA nodes at and below `id`, in order.
    pub fn text_content(&self, id: NodeId) -> String {
        let mut s = String::new();
        for d in std::iter::once(id).chain(self.descendants(id)) {
            if let Some(t) = self.text(d) {
                s.push_str(&t);
            }
        }
        s
    }

    /// The byte range a node was parsed from, in the bytes given to
    /// [`parse`](Self::parse). `None` for nodes that were created or came
    /// from a fragment. It stays the same after edits.
    pub fn source_range(&self, id: NodeId) -> Option<Range<usize>> {
        let s = self.node(id).span?;
        if s.buf != 0 {
            return None;
        }
        let text = &self.bufs[0];
        let r = s.range();
        Some(match self.encoding {
            Encoding::Utf8 { bom } => {
                let o = if bom { 3 } else { 0 };
                r.start + o..r.end + o
            }
            Encoding::Utf16 { bom, .. } => {
                let o = if bom { 2 } else { 0 };
                let start = text[..r.start].encode_utf16().count() * 2 + o;
                start..start + text[r].encode_utf16().count() * 2
            }
        })
    }

    // ---- editing ----------------------------------------------------------

    /// Creates an unattached element. `name` is a qualified name such as
    /// `w:p`; its prefix is a preference, used when it is bound to
    /// `namespace` where the element ends up or when it is free there.
    pub fn create_element(&mut self, namespace: Option<&str>, name: &str) -> Result<NodeId, Error> {
        let name = self.checked_name(namespace, name)?;
        let el = Element {
            name,
            renamed: false,
            scope: None,
            ctx: None,
            open: None,
            close: None,
            attrs: Some(vec![]),
        };
        Ok(self.push(Data::Element(Box::new(el)), None))
    }

    /// Creates an unattached text node.
    pub fn create_text(&mut self, text: &str) -> Result<NodeId, Error> {
        chars::check_chars(text).map_err(Error::Edit)?;
        Ok(self.push(Data::Text(Some(text.into())), None))
    }

    /// Replaces the text of a text or CDATA node, or replaces all children
    /// of an element by one text node.
    pub fn set_text(&mut self, id: NodeId, text: &str) -> Result<(), Error> {
        chars::check_chars(text).map_err(Error::Edit)?;
        match self.node(id).data {
            Data::Text(_) | Data::Cdata => {
                if self.parent(id) == Some(DOCUMENT) {
                    return Err(Error::Edit(
                        "text outside the root element cannot be changed".into(),
                    ));
                }
                self.node_mut(id).data = Data::Text(Some(text.into()));
                self.node_mut(id).span = None;
                self.touch(id, CHANGED);
                Ok(())
            }
            Data::Element(_) => {
                for c in self.children(id).to_vec() {
                    self.unlink(c);
                }
                let t = self.push(Data::Text(Some(text.into())), None);
                self.link(id, None, t);
                Ok(())
            }
            _ => Err(Error::Edit(
                "only text, CDATA and elements have text to set".into(),
            )),
        }
    }

    /// Sets an attribute, adding it if the element has none of that name.
    /// `name` is a qualified name such as `w:val`; an attribute in a
    /// namespace needs a prefix, which is chosen as for
    /// [`create_element`](Self::create_element).
    pub fn set_attribute(
        &mut self,
        id: NodeId,
        namespace: Option<&str>,
        name: &str,
        value: &str,
    ) -> Result<(), Error> {
        chars::check_chars(value).map_err(Error::Edit)?;
        let name = self.checked_name(namespace, name)?;
        self.attrs_mut(id)?;
        let Tree { nodes, names, .. } = self;
        let Data::Element(e) = &mut nodes[id.index()].data else {
            unreachable!("checked by attrs_mut")
        };
        let attrs = e.attrs.get_or_insert_with(Vec::new);
        let new = &names[name as usize];
        let value = AttrValue::Owned(value.into());
        match attrs.iter_mut().find(|a| {
            let old = &names[a.name as usize];
            old.ns == new.ns && old.local == new.local
        }) {
            Some(a) => a.value = value,
            None => attrs.push(Attr {
                name,
                value,
                raw: None,
                quote: b'"',
            }),
        }
        self.touch(id, CHANGED);
        Ok(())
    }

    /// Removes an attribute. Returns whether the element had it.
    pub fn remove_attribute(&mut self, id: NodeId, namespace: Option<&str>, local: &str) -> bool {
        if self.attribute(id, namespace, local).is_none() || self.attrs_mut(id).is_err() {
            return false;
        }
        let Tree {
            nodes, names, uris, ..
        } = self;
        let Data::Element(e) = &mut nodes[id.index()].data else {
            return false;
        };
        let attrs = e.attrs.get_or_insert_with(Vec::new);
        attrs.retain(|a| {
            let n = &names[a.name as usize];
            !(n.ns != Some(XMLNS_URI)
                && *n.local == *local
                && n.ns.map(|u| &*uris[u as usize]) == namespace)
        });
        self.touch(id, CHANGED);
        true
    }

    /// Renames an element, for example `w:t` to `w:delText`. Attributes and
    /// children stay.
    pub fn set_name(
        &mut self,
        id: NodeId,
        namespace: Option<&str>,
        name: &str,
    ) -> Result<(), Error> {
        let name = self.checked_name(namespace, name)?;
        self.attrs_mut(id)?;
        if namespace.is_none()
            && let Data::Element(e) = &self.node(id).data
            && let Some(scope) = e.scope.filter(|&s| Some(s) != e.ctx)
            && self.scopes[scope as usize].get("").flatten().is_some()
        {
            return Err(Error::Edit(
                "the element declares a default namespace, so its name cannot be in no namespace"
                    .into(),
            ));
        }
        if let Data::Element(e) = &mut self.node_mut(id).data {
            e.name = name;
            e.renamed = true;
        }
        self.touch(id, CHANGED);
        Ok(())
    }

    /// Appends `child` as the last child of `parent`, moving it if it is
    /// attached elsewhere.
    pub fn append(&mut self, parent: NodeId, child: NodeId) -> Result<(), Error> {
        self.check_insert(parent, child)?;
        self.unlink(child);
        self.link(parent, None, child);
        Ok(())
    }

    /// Inserts `child` as the child at `index` of `parent`, moving it if it
    /// is attached elsewhere. The index counts the children after `child`
    /// has left its old place.
    pub fn insert(&mut self, parent: NodeId, index: usize, child: NodeId) -> Result<(), Error> {
        self.check_insert(parent, child)?;
        let len = self.children(parent).len() - usize::from(self.parent(child) == Some(parent));
        if index > len {
            return Err(Error::Edit(format!(
                "index {index} is past the end of the element's {len} children"
            )));
        }
        self.unlink(child);
        self.link(parent, Some(index), child);
        Ok(())
    }

    /// Inserts `child` just before `anchor`.
    pub fn insert_before(&mut self, anchor: NodeId, child: NodeId) -> Result<(), Error> {
        self.insert_next_to(anchor, child, 0)
    }

    /// Inserts `child` just after `anchor`.
    pub fn insert_after(&mut self, anchor: NodeId, child: NodeId) -> Result<(), Error> {
        self.insert_next_to(anchor, child, 1)
    }

    /// Takes a node out of the tree. It can be attached again. The root
    /// element and the nodes around it cannot be detached.
    pub fn detach(&mut self, id: NodeId) -> Result<(), Error> {
        self.check_editable(id)?;
        self.unlink(id);
        Ok(())
    }

    /// Copies a node and everything below it. The copy is unattached and is
    /// written as the original was wherever it is attached in the same
    /// namespace scope.
    pub fn deep_clone(&mut self, id: NodeId) -> Result<NodeId, Error> {
        if matches!(self.node(id).data, Data::Document) {
            return Err(Error::Edit("the document node cannot be copied".into()));
        }
        let copy = |t: &mut Tree, src: NodeId, parent: Option<NodeId>| {
            let n = t.node(src).clone();
            let id = t.push_raw(Node {
                parent,
                children: vec![],
                data: n.data,
                span: n.span,
                flags: n.flags,
            });
            if let Some(p) = parent {
                t.node_mut(p).children.push(id);
            }
            id
        };
        let top = copy(self, id, None);
        let mut stack = vec![(id, top)];
        while let Some((src, dst)) = stack.pop() {
            for c in self.children(src).to_vec() {
                let n = copy(self, c, Some(dst));
                stack.push((c, n));
            }
        }
        Ok(top)
    }

    // ---- internals --------------------------------------------------------

    fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id.index()]
    }

    fn node_mut(&mut self, id: NodeId) -> &mut Node {
        &mut self.nodes[id.index()]
    }

    fn src(&self, s: Span) -> &str {
        &self.bufs[s.buf as usize][s.range()]
    }

    fn push_raw(&mut self, node: Node) -> NodeId {
        // Spans address at most 4 GiB, and a node takes at least one byte or
        // one call, so the count cannot overflow before memory runs out.
        let id = NodeId(u32::try_from(self.nodes.len()).expect("fewer than 2^32 nodes"));
        self.nodes.push(node);
        id
    }

    fn push(&mut self, data: Data, span: Option<Span>) -> NodeId {
        let flags = if span.is_some() { 0 } else { CHANGED };
        self.push_raw(Node {
            parent: None,
            children: vec![],
            data,
            span,
            flags,
        })
    }

    fn push_scope(&mut self, scope: Scope) -> ScopeId {
        let id = self.scopes.len() as ScopeId;
        self.scopes.push(scope);
        id
    }

    fn intern_uri(&mut self, uri: &str) -> UriId {
        if let Some(&u) = self.uri_index.get(uri) {
            return u;
        }
        let u = self.uris.len() as UriId;
        self.uris.push(uri.into());
        self.uri_index.insert(uri.into(), u);
        u
    }

    fn intern_name(&mut self, prefix: &str, local: &str, ns: Option<UriId>) -> NameId {
        let mut q = String::with_capacity(prefix.len() + local.len() + 1);
        if !prefix.is_empty() {
            q.push_str(prefix);
            q.push(':');
        }
        q.push_str(local);
        self.intern_qname(&q, prefix.len(), ns)
    }

    /// Interns a qualified name whose prefix is the first `prefix_len` bytes.
    fn intern_qname(&mut self, qname: &str, prefix_len: usize, ns: Option<UriId>) -> NameId {
        if let Some(&id) = self.name_index.get(qname).and_then(|m| m.get(&ns)) {
            return id;
        }
        let id = self.names.len() as NameId;
        let local = if prefix_len == 0 {
            qname
        } else {
            &qname[prefix_len + 1..]
        };
        self.names.push(NameEntry {
            prefix: qname[..prefix_len].into(),
            local: local.into(),
            ns,
        });
        self.name_index
            .entry(qname.into())
            .or_default()
            .insert(ns, id);
        id
    }

    fn name_of(&self, id: NameId) -> Name<'_> {
        let e = &self.names[id as usize];
        Name {
            namespace: e.ns.map(|u| &*self.uris[u as usize]),
            prefix: &e.prefix,
            local: &e.local,
        }
    }

    fn name_matches(&self, id: NameId, namespace: Option<&str>, local: &str) -> bool {
        let e = &self.names[id as usize];
        *e.local == *local && e.ns.map(|u| &*self.uris[u as usize]) == namespace
    }

    fn is_decl(&self, id: NameId) -> bool {
        self.names[id as usize].ns == Some(XMLNS_URI)
    }

    /// Resolves a prefix in a scope: `Some(None)` for no namespace, `None`
    /// for an unbound prefix.
    fn lookup(&self, mut scope: ScopeId, prefix: &str) -> Option<Option<UriId>> {
        loop {
            let s = &self.scopes[scope as usize];
            if let Some(u) = s.get(prefix) {
                return Some(u);
            }
            match s.parent {
                Some(p) => scope = p,
                None => break,
            }
        }
        match prefix {
            "" => Some(None),
            "xml" => Some(Some(XML_URI)),
            _ => None,
        }
    }

    fn attr_ns(&self, e: &Element, prefix: &str) -> Option<&str> {
        if prefix.is_empty() {
            return None;
        }
        let u = e.scope.and_then(|s| self.lookup(s, prefix)).flatten()?;
        Some(&self.uris[u as usize])
    }

    fn attr_value<'a>(&'a self, a: &'a Attr) -> Cow<'a, str> {
        match &a.value {
            AttrValue::Source(s) => chars::decode(self.src(*s), true),
            AttrValue::Owned(v) => Cow::Borrowed(v),
        }
    }

    /// The element's attributes as a list that edits can change, read from
    /// the source start tag the first time.
    fn attrs_mut(&mut self, id: NodeId) -> Result<&mut Vec<Attr>, Error> {
        let Data::Element(e) = &self.node(id).data else {
            return Err(Error::Edit("only elements have attributes".into()));
        };
        if e.attrs.is_none() {
            let (open, scope) = (e.open, e.scope);
            let list = self.source_attr_list(open, scope);
            if let Data::Element(e) = &mut self.node_mut(id).data {
                e.attrs = Some(list);
            }
        }
        match &mut self.node_mut(id).data {
            Data::Element(e) => Ok(e.attrs.get_or_insert_with(Vec::new)),
            _ => Err(Error::Edit("only elements have attributes".into())),
        }
    }

    /// The attributes of a parsed start tag, declarations included.
    fn source_attr_list(&mut self, open: Option<Span>, scope: Option<ScopeId>) -> Vec<Attr> {
        let (Some(open), Some(scope)) = (open, scope) else {
            return vec![];
        };
        let tag = self.src(open).to_owned();
        let mut list = vec![];
        for a in parse::source_attrs(&tag) {
            let Some((prefix, local)) = chars::split_qname(&tag[a.name.clone()]) else {
                continue;
            };
            let ns = if (prefix.is_empty() && local == "xmlns") || prefix == "xmlns" {
                Some(XMLNS_URI)
            } else if prefix.is_empty() {
                None
            } else {
                self.lookup(scope, prefix).flatten()
            };
            let name = self.intern_name(prefix, local, ns);
            let at = |r: Range<usize>| Span {
                buf: open.buf,
                start: open.start + r.start as u32,
                end: open.start + r.end as u32,
            };
            list.push(Attr {
                name,
                value: AttrValue::Source(at(a.value)),
                raw: Some(at(a.raw)),
                quote: a.quote,
            });
        }
        list
    }

    /// Validates a qualified name for an element or attribute and interns it.
    fn checked_name(&mut self, namespace: Option<&str>, name: &str) -> Result<NameId, Error> {
        let Some((prefix, local)) = chars::split_qname(name) else {
            return Err(Error::Edit(format!("{name:?} is not a valid XML name")));
        };
        let bad = |why: &str| Err(Error::Edit(format!("{name:?}: {why}")));
        if prefix == "xmlns"
            || (prefix.is_empty() && local == "xmlns")
            || namespace == Some(XMLNS_NS)
        {
            return bad("namespace declarations are managed by the tree");
        }
        if (prefix == "xml") != (namespace == Some(XML_NS)) {
            return bad("the prefix xml and its namespace belong together");
        }
        if namespace.is_none() && !prefix.is_empty() {
            return bad("a prefixed name needs a namespace");
        }
        if namespace == Some("") {
            return bad("the empty string is not a namespace; pass None");
        }
        let ns = namespace.map(|u| self.intern_uri(u));
        Ok(self.intern_name(prefix, local, ns))
    }

    /// Nodes of the prolog and epilogue, the root element and the document
    /// node stay where they are.
    fn check_editable(&self, id: NodeId) -> Result<(), Error> {
        if id == DOCUMENT || self.parent(id) == Some(DOCUMENT) {
            return Err(Error::Edit(
                "the root element and the nodes outside it cannot be moved or removed".into(),
            ));
        }
        Ok(())
    }

    fn check_insert(&self, parent: NodeId, child: NodeId) -> Result<(), Error> {
        if !self.is_element(parent) {
            return Err(Error::Edit("only an element can take children".into()));
        }
        self.check_editable(child)?;
        if child == parent || self.ancestors(parent).any(|a| a == child) {
            return Err(Error::Edit("a node cannot be inserted into itself".into()));
        }
        Ok(())
    }

    fn insert_next_to(&mut self, anchor: NodeId, child: NodeId, after: usize) -> Result<(), Error> {
        let Some(parent) = self.parent(anchor) else {
            return Err(Error::Edit("the anchor is not attached".into()));
        };
        if anchor == child {
            return Err(Error::Edit(
                "a node cannot be inserted next to itself".into(),
            ));
        }
        self.check_insert(parent, child)?;
        self.unlink(child);
        let i = self
            .children(parent)
            .iter()
            .position(|&c| c == anchor)
            .unwrap_or(0);
        self.link(parent, Some(i + after), child);
        Ok(())
    }

    fn unlink(&mut self, id: NodeId) {
        if let Some(p) = self.node_mut(id).parent.take() {
            self.node_mut(p).children.retain(|&c| c != id);
            self.touch(p, DESCEND);
        }
    }

    fn link(&mut self, parent: NodeId, index: Option<usize>, child: NodeId) {
        self.node_mut(child).parent = Some(parent);
        let kids = &mut self.node_mut(parent).children;
        match index {
            Some(i) => kids.insert(i.min(kids.len()), child),
            None => kids.push(child),
        }
        self.touch(parent, DESCEND);
    }

    /// Flags a node and marks its ancestors as changed below. An attached
    /// ancestor that is already marked has marked ancestors, so the walk
    /// stops there.
    fn touch(&mut self, id: NodeId, flag: u8) {
        self.node_mut(id).flags |= flag;
        let mut cur = self.parent(id);
        while let Some(c) = cur {
            if self.node(c).flags & DESCEND != 0 {
                break;
            }
            self.node_mut(c).flags |= DESCEND;
            cur = self.parent(c);
        }
    }
}

impl NodeId {
    fn index(self) -> usize {
        self.0 as usize
    }
}
