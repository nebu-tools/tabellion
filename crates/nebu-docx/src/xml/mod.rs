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
//! Whatever the edits, the written bytes are well-formed and parse back to
//! the same tree. Edits that cannot be written so are refused with an
//! error when they are made. Where an edit puts character data next to
//! other character data, the writer escapes the one character that would
//! otherwise join them into something else: a `]` or `>` that would form
//! `]]>`, a line feed after a carriage return.
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
//! but the five predefined ones, so no input can make it expand entities.
//!
//! A [`NodeId`] is a handle that knows its tree. A handle from another tree
//! gives [`Error::ForeignNode`]; no method panics on any argument.
//!
//! ```
//! use nebu_docx::xml::Tree;
//! use nebu_docx::ns;
//!
//! let src = r#"<x:document xmlns:x="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><x:body/></x:document>"#;
//! let mut tree = Tree::parse(src)?;
//! let body = tree.child(tree.root_element(), Some(ns::W), "body")?.unwrap();
//! let p = tree.create_element(Some(ns::W), "w:p")?;
//! tree.append(body, p)?;
//! // The new paragraph takes the prefix the file uses.
//! assert!(String::from_utf8(tree.to_bytes()).unwrap().contains("<x:body><x:p/></x:body>"));
//! # Ok::<(), nebu_docx::Error>(())
//! ```

mod chars;
mod parse;
mod write;

#[cfg(test)]
mod tests;

use std::borrow::Cow;
use std::collections::HashMap;
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::{Error, Limits};

/// The namespace that the `xml` prefix is always bound to.
pub const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";
/// The namespace of namespace declarations (`xmlns`, `xmlns:p`).
pub const XMLNS_NS: &str = "http://www.w3.org/2000/xmlns/";

/// A handle to a node of a [`Tree`].
///
/// A handle carries the identity of the tree that made it. Every method of
/// [`Tree`] checks it and answers a handle from another tree, a clone of
/// the tree included, with [`Error::ForeignNode`]. Nodes are never freed
/// while their tree lives, so a handle cannot go stale: a detached node can
/// be attached again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId {
    tree: u64,
    index: u32,
}

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

/// The index of a node in its tree.
type Ix = u32;
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
    parent: Option<Ix>,
    children: Vec<Ix>,
    data: Data,
    span: Option<Span>,
    flags: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Encoding {
    Utf8 { bom: bool },
    Utf16 { big_endian: bool, bom: bool },
}

/// Gives every tree its own identity, so that handles know their tree.
static NEXT_TREE: AtomicU64 = AtomicU64::new(1);

fn new_tree_id() -> u64 {
    NEXT_TREE.fetch_add(1, Ordering::Relaxed)
}

/// A parsed XML part. The [`xml`](self) page explains how it keeps bytes.
///
/// A clone is a tree of its own: handles of the original do not address its
/// nodes.
#[derive(Debug)]
pub struct Tree {
    id: u64,
    bufs: Vec<Box<str>>,
    nodes: Vec<Node>,
    scopes: Vec<Scope>,
    names: Vec<NameEntry>,
    name_index: HashMap<Box<str>, HashMap<Option<UriId>, NameId>>,
    uris: Vec<Box<str>>,
    uri_index: HashMap<Box<str>, UriId>,
    encoding: Encoding,
    /// The limits the tree was made with. They bound it for its whole life:
    /// every node it allocates and every edit that deepens it.
    limits: Limits,
}

impl Clone for Tree {
    fn clone(&self) -> Tree {
        Tree {
            id: new_tree_id(),
            bufs: self.bufs.clone(),
            nodes: self.nodes.clone(),
            scopes: self.scopes.clone(),
            names: self.names.clone(),
            name_index: self.name_index.clone(),
            uris: self.uris.clone(),
            uri_index: self.uri_index.clone(),
            encoding: self.encoding,
            limits: self.limits.clone(),
        }
    }
}

const DOCUMENT: Ix = 0;
const XML_URI: UriId = 0;
const XMLNS_URI: UriId = 1;

impl Tree {
    /// Parses a complete XML document with the default [`Limits`].
    ///
    /// The bytes may be UTF-8, with or without a byte order mark, or UTF-16
    /// with a byte order mark; [`to_bytes`](Self::to_bytes) writes the same
    /// encoding back.
    pub fn parse(bytes: impl Into<Vec<u8>>) -> Result<Tree, Error> {
        Tree::parse_with_limits(bytes, &Limits::default())
    }

    /// Parses a complete XML document with the given limits. The tree keeps
    /// them: [`max_xml_nodes`](Limits::max_xml_nodes) counts every node it
    /// ever allocates, parsed, created, copied or from a fragment, and
    /// [`max_depth`](Limits::max_depth) bounds every edit that deepens it.
    /// An edit over a limit is refused with [`Error::Limit`] and changes
    /// nothing.
    pub fn parse_with_limits(bytes: impl Into<Vec<u8>>, limits: &Limits) -> Result<Tree, Error> {
        let (text, encoding) = parse::decode_bytes(bytes.into())?;
        let mut t = Tree {
            id: new_tree_id(),
            bufs: vec![],
            nodes: vec![],
            scopes: vec![Scope::new(None, vec![])],
            names: vec![],
            name_index: HashMap::new(),
            uris: vec![],
            uri_index: HashMap::new(),
            encoding,
            limits: limits.clone(),
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
        parse::parse(&mut t, text, Some(DOCUMENT), 0, false)?;
        Ok(t)
    }

    /// Parses an XML fragment, one or more sibling nodes, and returns them
    /// unattached, ready for [`append`](Self::append) or
    /// [`insert`](Self::insert). `namespaces` binds the prefixes the
    /// fragment uses, as `(prefix, namespace)` pairs, with an empty prefix
    /// for the default namespace; each prefix may appear once.
    ///
    /// The fragment's nodes are written as parsed wherever the tree binds
    /// the same prefixes to the same namespaces, and with the tree's own
    /// prefixes where it does not. The fragment's nodes count against the
    /// tree's [`Limits::max_xml_nodes`], and its nesting against
    /// [`Limits::max_depth`]; over either, nothing is added.
    pub fn parse_fragment(
        &mut self,
        xml: &str,
        namespaces: &[(&str, &str)],
    ) -> Result<Vec<NodeId>, Error> {
        let mut bindings: Vec<(Box<str>, Option<UriId>)> = vec![];
        for (i, &(p, uri)) in namespaces.iter().enumerate() {
            let bad = || Err(Error::Edit(format!("cannot bind prefix {p:?} to {uri:?}")));
            if !(p.is_empty() || chars::is_ncname(p))
                || p == "xmlns"
                || (p == "xml") != (uri == XML_NS)
                || uri.is_empty()
                || uri == XMLNS_NS
                || chars::check_chars(uri).is_err()
            {
                return bad();
            }
            if namespaces[..i].iter().any(|&(q, _)| q == p) {
                return Err(Error::Edit(format!("the prefix {p:?} is bound twice")));
            }
            let u = self.intern_uri(uri);
            bindings.push((p.into(), Some(u)));
        }
        let scope = self.push_scope(Scope::new(None, bindings));
        let (nodes, scopes, bufs) = (self.nodes.len(), self.scopes.len(), self.bufs.len());
        match parse::parse(self, xml.to_owned(), None, scope, true) {
            Ok(ids) => Ok(ids.into_iter().map(|i| self.handle(i)).collect()),
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
    pub(crate) fn is_changed(&self) -> bool {
        self.nodes[DOCUMENT as usize].flags != 0
    }

    // ---- reading ----------------------------------------------------------

    /// The document node: the parent of the prolog and the root element.
    pub fn document(&self) -> NodeId {
        self.handle(DOCUMENT)
    }

    /// The root element.
    pub fn root_element(&self) -> NodeId {
        // A parsed document always has one; the parser refuses others.
        let root = self.nodes[DOCUMENT as usize]
            .children
            .iter()
            .copied()
            .find(|&c| matches!(self.n(c).data, Data::Element(_)))
            .unwrap_or(DOCUMENT);
        self.handle(root)
    }

    /// What the node is.
    pub fn kind(&self, id: NodeId) -> Result<NodeKind, Error> {
        Ok(match self.node(id)?.data {
            Data::Document => NodeKind::Document,
            Data::Element(_) => NodeKind::Element,
            Data::Text(_) => NodeKind::Text,
            Data::Cdata => NodeKind::CData,
            Data::Comment => NodeKind::Comment,
            Data::Pi => NodeKind::ProcessingInstruction,
            Data::Decl => NodeKind::Declaration,
        })
    }

    /// True for an element.
    pub fn is_element(&self, id: NodeId) -> Result<bool, Error> {
        Ok(matches!(self.node(id)?.data, Data::Element(_)))
    }

    /// The name of an element, `None` for other nodes.
    pub fn name(&self, id: NodeId) -> Result<Option<Name<'_>>, Error> {
        Ok(match &self.node(id)?.data {
            Data::Element(e) => Some(self.name_of(e.name)),
            _ => None,
        })
    }

    /// True if the node is an element with this namespace and local name.
    pub fn is(&self, id: NodeId, namespace: Option<&str>, local: &str) -> Result<bool, Error> {
        Ok(self.is_ix(self.ix(id)?, namespace, local))
    }

    /// The parent, `None` for the document node and for unattached nodes.
    pub fn parent(&self, id: NodeId) -> Result<Option<NodeId>, Error> {
        Ok(self.node(id)?.parent.map(|p| self.handle(p)))
    }

    /// The children in document order.
    pub fn children(
        &self,
        id: NodeId,
    ) -> Result<impl DoubleEndedIterator<Item = NodeId> + ExactSizeIterator + '_, Error> {
        Ok(self.node(id)?.children.iter().map(|&c| self.handle(c)))
    }

    /// The child elements in document order.
    pub fn elements(&self, id: NodeId) -> Result<impl Iterator<Item = NodeId> + '_, Error> {
        Ok(self
            .node(id)?
            .children
            .iter()
            .copied()
            .filter(|&c| matches!(self.n(c).data, Data::Element(_)))
            .map(|c| self.handle(c)))
    }

    /// The first child element with this name.
    pub fn child(
        &self,
        id: NodeId,
        namespace: Option<&str>,
        local: &str,
    ) -> Result<Option<NodeId>, Error> {
        Ok(self
            .node(id)?
            .children
            .iter()
            .copied()
            .find(|&c| self.is_ix(c, namespace, local))
            .map(|c| self.handle(c)))
    }

    /// All nodes below `id` in document order, `id` itself excluded.
    pub fn descendants(&self, id: NodeId) -> Result<impl Iterator<Item = NodeId> + '_, Error> {
        let mut stack: Vec<Ix> = self.node(id)?.children.iter().rev().copied().collect();
        Ok(std::iter::from_fn(move || {
            let next = stack.pop()?;
            stack.extend(self.n(next).children.iter().rev().copied());
            Some(self.handle(next))
        }))
    }

    /// The parent, its parent and so on up to the document node.
    pub fn ancestors(&self, id: NodeId) -> Result<impl Iterator<Item = NodeId> + '_, Error> {
        let first = self.node(id)?.parent;
        Ok(std::iter::successors(first, move |&p| self.n(p).parent).map(|p| self.handle(p)))
    }

    /// The value of an attribute, found by namespace and local name.
    pub fn attribute(
        &self,
        id: NodeId,
        namespace: Option<&str>,
        local: &str,
    ) -> Result<Option<Cow<'_, str>>, Error> {
        let Data::Element(e) = &self.node(id)?.data else {
            return Ok(None);
        };
        Ok(match &e.attrs {
            Some(attrs) => attrs
                .iter()
                .find(|a| !self.is_decl(a.name) && self.name_matches(a.name, namespace, local))
                .map(|a| self.attr_value(a)),
            None => {
                let Some(open) = e.open else {
                    return Ok(None);
                };
                let tag = self.src(open);
                // Most lookups miss; a substring search is cheaper than a scan.
                if !tag.contains(local) {
                    return Ok(None);
                }
                parse::source_attrs(tag)
                    .find(|a| {
                        let (prefix, l) =
                            chars::split_qname(&tag[a.name.clone()]).unwrap_or(("", ""));
                        let decl = prefix == "xmlns" || (prefix.is_empty() && l == "xmlns");
                        !decl && l == local && self.attr_ns(e, prefix) == namespace
                    })
                    .map(|a| chars::decode(&tag[a.value], true))
            }
        })
    }

    /// The attributes of an element in source order, without namespace
    /// declarations. Empty for other nodes.
    pub fn attributes(&self, id: NodeId) -> Result<Vec<Attribute<'_>>, Error> {
        let Data::Element(e) = &self.node(id)?.data else {
            return Ok(vec![]);
        };
        Ok(match &e.attrs {
            Some(attrs) => attrs
                .iter()
                .filter(|a| !self.is_decl(a.name))
                .map(|a| Attribute {
                    name: self.name_of(a.name),
                    value: self.attr_value(a),
                })
                .collect(),
            None => {
                let Some(open) = e.open else {
                    return Ok(vec![]);
                };
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
        })
    }

    /// The text of a text or CDATA node, references decoded and line ends
    /// normalised. `None` for other nodes.
    pub fn text(&self, id: NodeId) -> Result<Option<Cow<'_, str>>, Error> {
        Ok(self.text_ix(self.ix(id)?))
    }

    /// The text of all text and CDATA nodes at and below `id`, in order.
    pub fn text_content(&self, id: NodeId) -> Result<String, Error> {
        let mut s = String::new();
        for d in std::iter::once(id).chain(self.descendants(id)?) {
            if let Some(t) = self.text(d)? {
                s.push_str(&t);
            }
        }
        Ok(s)
    }

    /// The byte range a node was parsed from, in the bytes given to
    /// [`parse`](Self::parse). `None` for nodes that were created or came
    /// from a fragment. It stays the same after edits. Only the tests use
    /// it.
    #[cfg(test)]
    pub(crate) fn source_range(&self, id: NodeId) -> Result<Option<Range<usize>>, Error> {
        let Some(s) = self.node(id)?.span else {
            return Ok(None);
        };
        if s.buf != 0 {
            return Ok(None);
        }
        let text = &self.bufs[0];
        let r = s.range();
        Ok(Some(match self.encoding {
            Encoding::Utf8 { bom } => {
                let o = if bom { 3 } else { 0 };
                r.start + o..r.end + o
            }
            Encoding::Utf16 { bom, .. } => {
                let o = if bom { 2 } else { 0 };
                let start = text[..r.start].encode_utf16().count() * 2 + o;
                start..start + text[r].encode_utf16().count() * 2
            }
        }))
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
        let i = self.push(Data::Element(Box::new(el)), None)?;
        Ok(self.handle(i))
    }

    /// Creates an unattached text node. Any text of characters XML allows
    /// can be written; the writer escapes what needs escaping.
    pub fn create_text(&mut self, text: &str) -> Result<NodeId, Error> {
        chars::check_chars(text).map_err(Error::Edit)?;
        let i = self.push(Data::Text(Some(text.into())), None)?;
        Ok(self.handle(i))
    }

    /// Replaces the text of a text or CDATA node, or replaces all children
    /// of an element by one text node. A CDATA section becomes a text node.
    pub fn set_text(&mut self, id: NodeId, text: &str) -> Result<(), Error> {
        let i = self.ix(id)?;
        chars::check_chars(text).map_err(Error::Edit)?;
        match self.n(i).data {
            Data::Text(_) | Data::Cdata => {
                if self.n(i).parent == Some(DOCUMENT) {
                    return Err(Error::Edit(
                        "text outside the root element cannot be changed".into(),
                    ));
                }
                let n = self.n_mut(i);
                n.data = Data::Text(Some(text.into()));
                n.span = None;
                self.touch(i, CHANGED);
                Ok(())
            }
            Data::Element(_) => {
                let t = self.push(Data::Text(Some(text.into())), None)?;
                for c in self.n(i).children.clone() {
                    self.unlink(c);
                }
                self.link(i, None, t);
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
        let i = self.ix(id)?;
        chars::check_chars(value).map_err(Error::Edit)?;
        if !matches!(self.n(i).data, Data::Element(_)) {
            return Err(Error::Edit("only elements have attributes".into()));
        }
        let name = self.checked_name(namespace, name)?;
        self.attrs_mut(i)?;
        let Tree { nodes, names, .. } = self;
        let Data::Element(e) = &mut nodes[i as usize].data else {
            return Err(Error::Edit("only elements have attributes".into()));
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
        self.touch(i, CHANGED);
        Ok(())
    }

    /// Removes an attribute. Returns whether the element had it.
    pub fn remove_attribute(
        &mut self,
        id: NodeId,
        namespace: Option<&str>,
        local: &str,
    ) -> Result<bool, Error> {
        let i = self.ix(id)?;
        if self.attribute(id, namespace, local)?.is_none() {
            return Ok(false);
        }
        self.attrs_mut(i)?;
        let Tree {
            nodes, names, uris, ..
        } = self;
        let Data::Element(e) = &mut nodes[i as usize].data else {
            return Ok(false);
        };
        let attrs = e.attrs.get_or_insert_with(Vec::new);
        attrs.retain(|a| {
            let n = &names[a.name as usize];
            !(n.ns != Some(XMLNS_URI)
                && *n.local == *local
                && n.ns.map(|u| &*uris[u as usize]) == namespace)
        });
        self.touch(i, CHANGED);
        Ok(true)
    }

    /// Renames an element, for example `w:t` to `w:delText`. Attributes and
    /// children stay.
    pub fn set_name(
        &mut self,
        id: NodeId,
        namespace: Option<&str>,
        name: &str,
    ) -> Result<(), Error> {
        let i = self.ix(id)?;
        let Data::Element(e) = &self.n(i).data else {
            return Err(Error::Edit("only elements have a name to set".into()));
        };
        if namespace.is_none()
            && let Some(scope) = e.scope.filter(|&s| Some(s) != e.ctx)
            && self.scopes[scope as usize].get("").flatten().is_some()
        {
            return Err(Error::Edit(
                "the element declares a default namespace, so its name cannot be in no namespace"
                    .into(),
            ));
        }
        let name = self.checked_name(namespace, name)?;
        self.attrs_mut(i)?;
        if let Data::Element(e) = &mut self.n_mut(i).data {
            e.name = name;
            e.renamed = true;
        }
        self.touch(i, CHANGED);
        Ok(())
    }

    /// Appends `child` as the last child of `parent`, moving it if it is
    /// attached elsewhere.
    pub fn append(&mut self, parent: NodeId, child: NodeId) -> Result<(), Error> {
        let (p, c) = self.check_insert(parent, child)?;
        self.unlink(c);
        self.link(p, None, c);
        Ok(())
    }

    /// Inserts `child` as the child at `index` of `parent`, moving it if it
    /// is attached elsewhere. The index counts the children after `child`
    /// has left its old place.
    pub fn insert(&mut self, parent: NodeId, index: usize, child: NodeId) -> Result<(), Error> {
        let (p, c) = self.check_insert(parent, child)?;
        let len = self.n(p).children.len() - usize::from(self.n(c).parent == Some(p));
        if index > len {
            return Err(Error::Edit(format!(
                "index {index} is past the end of the element's {len} children"
            )));
        }
        self.unlink(c);
        self.link(p, Some(index), c);
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

    /// Takes a node out of the tree. It stays in the tree's memory and can be
    /// attached again. The root element and the nodes around it cannot be
    /// detached.
    pub fn detach(&mut self, id: NodeId) -> Result<(), Error> {
        let i = self.ix(id)?;
        self.check_editable(i)?;
        self.unlink(i);
        Ok(())
    }

    /// Copies a node and everything below it. The copy is unattached and is
    /// written as the original was wherever it is attached in the same
    /// namespace scope. The document node and the XML declaration cannot be
    /// copied. The copy's nodes count against [`Limits::max_xml_nodes`]; a
    /// copy that does not fit is refused whole.
    pub fn deep_clone(&mut self, id: NodeId) -> Result<NodeId, Error> {
        let i = self.ix(id)?;
        if matches!(self.n(i).data, Data::Document | Data::Decl) {
            return Err(Error::Edit(
                "the document node and the XML declaration cannot be copied".into(),
            ));
        }
        let mut size = 0;
        let mut stack = vec![i];
        while let Some(n) = stack.pop() {
            size += 1;
            stack.extend_from_slice(&self.n(n).children);
        }
        self.room(size)?;
        let copy = |t: &mut Tree, src: Ix, parent: Option<Ix>| -> Result<Ix, Error> {
            let n = t.n(src);
            let node = Node {
                parent,
                children: vec![],
                data: n.data.clone(),
                span: n.span,
                flags: n.flags,
            };
            let id = t.push_raw(node)?;
            if let Some(p) = parent {
                t.n_mut(p).children.push(id);
            }
            Ok(id)
        };
        let top = copy(self, i, None)?;
        let mut stack = vec![(i, top)];
        while let Some((src, dst)) = stack.pop() {
            for c in self.n(src).children.clone() {
                let n = copy(self, c, Some(dst))?;
                stack.push((c, n));
            }
        }
        Ok(self.handle(top))
    }

    // ---- internals --------------------------------------------------------

    fn handle(&self, index: Ix) -> NodeId {
        NodeId {
            tree: self.id,
            index,
        }
    }

    /// The index of a handle, checked to belong to this tree.
    fn ix(&self, id: NodeId) -> Result<Ix, Error> {
        if id.tree != self.id || id.index as usize >= self.nodes.len() {
            return Err(Error::ForeignNode);
        }
        Ok(id.index)
    }

    fn node(&self, id: NodeId) -> Result<&Node, Error> {
        Ok(self.n(self.ix(id)?))
    }

    /// The node at an index that this tree handed out itself.
    fn n(&self, i: Ix) -> &Node {
        &self.nodes[i as usize]
    }

    fn n_mut(&mut self, i: Ix) -> &mut Node {
        &mut self.nodes[i as usize]
    }

    fn src(&self, s: Span) -> &str {
        &self.bufs[s.buf as usize][s.range()]
    }

    fn is_ix(&self, i: Ix, namespace: Option<&str>, local: &str) -> bool {
        match &self.n(i).data {
            Data::Element(e) => self.name_matches(e.name, namespace, local),
            _ => false,
        }
    }

    fn text_ix(&self, i: Ix) -> Option<Cow<'_, str>> {
        let n = self.n(i);
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

    /// Checks that `extra` more nodes fit the node limit. Every node the
    /// tree allocates, whatever made it, is checked here first.
    fn room(&self, extra: usize) -> Result<(), Error> {
        let max = self.limits.max_xml_nodes;
        // The document node is not counted.
        if (self.nodes.len() - 1).saturating_add(extra) > max {
            return Err(Error::Limit {
                what: format!("an XML part with more than {max} nodes"),
                field: "max_xml_nodes",
                limit: max as u64,
            });
        }
        Ok(())
    }

    /// Checks that putting `child` under `parent` keeps every element within
    /// the depth limit. Detached subtrees are bounded too, from their own
    /// top, so a subtree built apart cannot grow past the limit either.
    fn check_depth(&self, parent: Ix, child: Ix) -> Result<(), Error> {
        if !matches!(self.n(child).data, Data::Element(_)) {
            return Ok(());
        }
        let max = self.limits.max_depth;
        let mut above = 0usize;
        let mut cur = Some(parent);
        while let Some(a) = cur {
            if matches!(self.n(a).data, Data::Element(_)) {
                above += 1;
            }
            cur = self.n(a).parent;
        }
        let over = || Error::Limit {
            what: format!("XML nested more than {max} levels deep"),
            field: "max_depth",
            limit: max as u64,
        };
        if above >= max {
            return Err(over());
        }
        // Only elements nest; the deepest element below `child` decides.
        let mut stack = vec![(child, above + 1)];
        while let Some((n, depth)) = stack.pop() {
            if depth > max {
                return Err(over());
            }
            for &c in &self.n(n).children {
                if matches!(self.n(c).data, Data::Element(_)) {
                    stack.push((c, depth + 1));
                }
            }
        }
        Ok(())
    }

    fn push_raw(&mut self, node: Node) -> Result<Ix, Error> {
        self.room(1)?;
        let Ok(id) = Ix::try_from(self.nodes.len()) else {
            return Err(Error::Edit(
                "a tree cannot hold more than 4,294,967,295 nodes".into(),
            ));
        };
        self.nodes.push(node);
        Ok(id)
    }

    fn push(&mut self, data: Data, span: Option<Span>) -> Result<Ix, Error> {
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
    fn attrs_mut(&mut self, i: Ix) -> Result<&mut Vec<Attr>, Error> {
        let Data::Element(e) = &self.n(i).data else {
            return Err(Error::Edit("only elements have attributes".into()));
        };
        if e.attrs.is_none() {
            let (open, scope) = (e.open, e.scope);
            let list = self.source_attr_list(open, scope);
            if let Data::Element(e) = &mut self.n_mut(i).data {
                e.attrs = Some(list);
            }
        }
        match &mut self.n_mut(i).data {
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
        if let Some(uri) = namespace {
            chars::check_chars(uri).map_err(Error::Edit)?;
        }
        let ns = namespace.map(|u| self.intern_uri(u));
        Ok(self.intern_name(prefix, local, ns))
    }

    /// Nodes of the prolog and epilogue, the root element and the document
    /// node stay where they are.
    fn check_editable(&self, i: Ix) -> Result<(), Error> {
        if i == DOCUMENT || self.n(i).parent == Some(DOCUMENT) {
            return Err(Error::Edit(
                "the root element and the nodes outside it cannot be moved or removed".into(),
            ));
        }
        Ok(())
    }

    fn check_insert(&self, parent: NodeId, child: NodeId) -> Result<(Ix, Ix), Error> {
        let (p, c) = (self.ix(parent)?, self.ix(child)?);
        if !matches!(self.n(p).data, Data::Element(_)) {
            return Err(Error::Edit("only an element can take children".into()));
        }
        self.check_editable(c)?;
        if matches!(self.n(c).data, Data::Decl) {
            return Err(Error::Edit(
                "the XML declaration cannot go inside an element".into(),
            ));
        }
        let mut cur = Some(p);
        while let Some(a) = cur {
            if a == c {
                return Err(Error::Edit("a node cannot be inserted into itself".into()));
            }
            cur = self.n(a).parent;
        }
        self.check_depth(p, c)?;
        Ok((p, c))
    }

    fn insert_next_to(&mut self, anchor: NodeId, child: NodeId, after: usize) -> Result<(), Error> {
        let a = self.ix(anchor)?;
        self.ix(child)?;
        let Some(parent) = self.n(a).parent else {
            return Err(Error::Edit("the anchor is not attached".into()));
        };
        if anchor == child {
            return Err(Error::Edit(
                "a node cannot be inserted next to itself".into(),
            ));
        }
        let (p, c) = self.check_insert(self.handle(parent), child)?;
        self.unlink(c);
        let i = self.n(p).children.iter().position(|&k| k == a).unwrap_or(0);
        self.link(p, Some(i + after), c);
        Ok(())
    }

    fn unlink(&mut self, i: Ix) {
        if let Some(p) = self.n_mut(i).parent.take() {
            self.n_mut(p).children.retain(|&c| c != i);
            self.touch(p, DESCEND);
        }
    }

    fn link(&mut self, parent: Ix, index: Option<usize>, child: Ix) {
        self.n_mut(child).parent = Some(parent);
        let kids = &mut self.n_mut(parent).children;
        match index {
            Some(i) => kids.insert(i.min(kids.len()), child),
            None => kids.push(child),
        }
        self.touch(parent, DESCEND);
    }

    /// Flags a node and marks its ancestors as changed below. An attached
    /// ancestor that is already marked has marked ancestors, so the walk
    /// stops there.
    fn touch(&mut self, i: Ix, flag: u8) {
        self.n_mut(i).flags |= flag;
        let mut cur = self.n(i).parent;
        while let Some(c) = cur {
            if self.n(c).flags & DESCEND != 0 {
                break;
            }
            self.n_mut(c).flags |= DESCEND;
            cur = self.n(c).parent;
        }
    }
}
