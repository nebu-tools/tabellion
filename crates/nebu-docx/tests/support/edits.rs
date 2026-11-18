//! Random documents and random edit sequences, with the checks every edit
//! sequence must pass. Shared by the property tests and the `edit` fuzz
//! target, which includes this file by path, so it uses nothing but
//! `nebu_docx` and `roxmltree`.
//!
//! After any sequence of edits:
//!
//! - the written bytes parse again, and write back as the same bytes;
//! - the tree, the tree parsed from the output, and roxmltree reading the
//!   output see the same elements, namespaces, attributes and text;
//! - every element, comment, processing instruction and CDATA section that
//!   no edit touched is written as the bytes it was parsed from;
//! - the tree stays within the limits it was parsed with, and an edit the
//!   limits refuse leaves the tree as it was.
//!
//! Some runs use limits just above what the document needs, so that edits
//! reach them.

#![allow(dead_code)]

use std::cell::Cell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::Range;

use nebu_docx::xml::{NodeId, NodeKind, Tree, XML_NS};
use nebu_docx::{Error, Limits};

// ---- random documents -----------------------------------------------------

pub const URIS: [&str; 3] = ["urn:a", "urn:b", "urn:c"];
const PREFIXES: [&str; 4] = ["", "p", "q", "r"];
const LOCALS: [&str; 4] = ["a", "b", "c", "d"];
const ATTRS: [&str; 3] = ["x", "y", "z"];

pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    pub fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
    pub fn pick<'a>(&mut self, xs: &[&'a str]) -> &'a str {
        xs[self.below(xs.len())]
    }
}

/// Text or an attribute value (for `quote`), with references, raw `>` and
/// `]`, and every kind of line end.
fn text(r: &mut Rng, quote: Option<char>) -> String {
    let pieces = [
        "a",
        "bc",
        " ",
        "\n",
        "\r\n",
        "\r",
        "\t",
        "&amp;",
        "&lt;",
        "&gt;",
        ">",
        "&#169;",
        "&#x1F426;",
        "&#0065;",
        "é",
        "]",
        "]]",
        "'",
        "\"",
        "&quot;",
        "&apos;",
    ];
    let mut s = String::new();
    for _ in 0..r.below(5) {
        let p = r.pick(&pieces);
        if quote.is_some_and(|q| p.contains(q)) {
            continue;
        }
        s.push_str(p);
    }
    if quote.is_none() {
        s = s.replace("]]>", "]]&gt;");
    }
    s
}

fn space(r: &mut Rng) -> &'static str {
    ["", " ", "\n  ", "\t"][r.below(4)]
}

/// An element whose prefixes are all bound in `scope`.
fn element(r: &mut Rng, scope: &[(String, Option<&'static str>)], depth: usize, out: &mut String) {
    let mut scope = scope.to_vec();
    let mut decls = String::new();
    let mut declared: Vec<&str> = vec![];
    for _ in 0..r.below(3) {
        let p = r.pick(&PREFIXES);
        if declared.contains(&p) {
            continue;
        }
        declared.push(p);
        let uri = if p.is_empty() && r.chance(20) {
            None
        } else {
            Some(URIS[r.below(3)])
        };
        let name = if p.is_empty() {
            "xmlns".to_string()
        } else {
            format!("xmlns:{p}")
        };
        decls.push_str(&format!(" {name}=\"{}\"", uri.unwrap_or("")));
        scope.push((p.to_string(), uri));
    }
    fn bound(p: &str, scope: &[(String, Option<&'static str>)]) -> Option<&'static str> {
        scope
            .iter()
            .rev()
            .find(|(q, _)| q == p)
            .and_then(|(_, u)| *u)
    }
    let prefixed: Vec<String> = PREFIXES[1..]
        .iter()
        .filter(|p| bound(p, &scope).is_some())
        .map(|p| p.to_string())
        .collect();
    let prefix = if !prefixed.is_empty() && r.chance(60) {
        prefixed[r.below(prefixed.len())].clone()
    } else {
        String::new()
    };
    let local = r.pick(&LOCALS);
    let qname = if prefix.is_empty() {
        local.to_string()
    } else {
        format!("{prefix}:{local}")
    };
    out.push('<');
    out.push_str(&qname);
    out.push_str(&decls);
    let mut seen: Vec<(Option<&str>, &str)> = vec![];
    for _ in 0..r.below(4) {
        let a = r.pick(&ATTRS);
        let p = if !prefixed.is_empty() && r.chance(40) {
            prefixed[r.below(prefixed.len())].clone()
        } else {
            String::new()
        };
        let ns = if p.is_empty() {
            None
        } else {
            bound(&p, &scope)
        };
        if seen.contains(&(ns, a)) {
            continue;
        }
        seen.push((ns, a));
        let q = if r.chance(50) { '"' } else { '\'' };
        let name = if p.is_empty() {
            a.to_string()
        } else {
            format!("{p}:{a}")
        };
        out.push_str(&format!(
            " {name}{}={}{q}{}{q}",
            space(r),
            space(r),
            text(r, Some(q))
        ));
    }
    out.push_str(space(r).trim_start_matches('\t'));
    let n = if depth < 5 { r.below(5) } else { 0 };
    if n == 0 && r.chance(50) {
        out.push_str("/>");
        return;
    }
    out.push('>');
    for _ in 0..n {
        match r.below(10) {
            0..=2 => {
                // Keep adjacent pieces of text from forming "]]>".
                let t = text(r, None);
                if out.ends_with(']') && t.starts_with(['>', ']']) {
                    out.push(' ');
                }
                out.push_str(&t);
            }
            3 => out.push_str(&format!("<!--{}-->", ["", " c ", "-x", "a-b"][r.below(4)])),
            4 => out.push_str(&format!("<?t{}?>", ["", " data", " a?b"][r.below(3)])),
            5 => out.push_str(&format!(
                "<![CDATA[{}]]>",
                ["", "x < y & z", "]", "a] ]b", "\r\n"][r.below(5)]
            )),
            _ => element(r, &scope, depth + 1, out),
        }
    }
    out.push_str(&format!("</{qname}{}>", space(r)));
}

/// A random well-formed document with namespaces.
pub fn document(seed: u64) -> String {
    let mut r = Rng(seed);
    let mut s = String::new();
    if r.chance(50) {
        s.push_str(
            [
                "<?xml version=\"1.0\"?>",
                "<?xml version='1.0' encoding='UTF-8' standalone='yes'?>",
            ][r.below(2)],
        );
    }
    for _ in 0..r.below(3) {
        s.push_str(["\n", "<!-- prolog -->", "<?pi x?>", "\r\n"][r.below(4)]);
    }
    element(&mut r, &[], 0, &mut s);
    if r.chance(30) {
        s.push_str("\n<!-- end -->\n");
    }
    s
}

/// Documents whose text sits right at the boundaries the writer must watch.
pub const BOUNDARY_DOCUMENTS: [&str; 6] = [
    "<r>></r>",
    "<r>]]<x/>></r>",
    "<r>]<x/>]></r>",
    "<r>a\r<x/>\nb</r>",
    "<r xmlns:p='urn:a'><p:x p:k=']]>'/>]<y/>]>\r<z/>\n</r>",
    "<r xmlns='urn:a' xmlns:p='urn:b'><a><p:b/></a><c xmlns:p='urn:c'><p:b/></c></r>",
];

// ---- what a tree means ----------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum C {
    El {
        ns: Option<String>,
        local: String,
        attrs: BTreeMap<(Option<String>, String), String>,
        children: Vec<C>,
    },
    Text(String),
}

/// Merges adjacent text and drops empty text, as XML does not tell them apart.
pub fn tidy(children: Vec<C>) -> Vec<C> {
    let mut out: Vec<C> = vec![];
    for c in children {
        match (c, out.last_mut()) {
            (C::Text(t), _) if t.is_empty() => {}
            (C::Text(t), Some(C::Text(prev))) => prev.push_str(&t),
            (c, _) => out.push(c),
        }
    }
    out
}

pub fn from_rox(n: roxmltree::Node<'_, '_>) -> C {
    C::El {
        ns: n
            .tag_name()
            .namespace()
            .filter(|s| !s.is_empty())
            .map(String::from),
        local: n.tag_name().name().to_string(),
        attrs: n
            .attributes()
            .map(|a| {
                (
                    (
                        a.namespace().filter(|s| !s.is_empty()).map(String::from),
                        a.name().to_string(),
                    ),
                    a.value().to_string(),
                )
            })
            .collect(),
        children: tidy(
            n.children()
                .filter_map(|c| {
                    if c.is_element() {
                        Some(from_rox(c))
                    } else if c.is_text() {
                        Some(C::Text(c.text().unwrap_or("").to_string()))
                    } else {
                        None
                    }
                })
                .collect(),
        ),
    }
}

/// What roxmltree reads in `xml`.
///
/// roxmltree 0.21 keeps some carriage returns that are not followed by a
/// line feed (`"\r\r&amp;"` reads as `"\n\r&"`), so the line ends are
/// normalised first, as XML 1.0 section 2.11 has every processor do before
/// parsing. Character references to a carriage return are not line ends and
/// stay.
pub fn reference(xml: &str) -> C {
    let xml = &xml.replace("\r\n", "\n").replace('\r', "\n");
    let d = roxmltree::Document::parse(xml).unwrap_or_else(|e| panic!("{e}\n{xml:?}"));
    from_rox(d.root_element())
}

/// What the tree says about the element `id`.
pub fn from_tree(t: &Tree, id: NodeId) -> C {
    let name = t.name(id).unwrap().unwrap();
    C::El {
        ns: name.namespace.map(String::from),
        local: name.local.to_string(),
        attrs: t
            .attributes(id)
            .unwrap()
            .into_iter()
            .map(|a| {
                (
                    (a.name.namespace.map(String::from), a.name.local.to_string()),
                    a.value.into_owned(),
                )
            })
            .collect(),
        children: tidy(
            t.children(id)
                .unwrap()
                .filter_map(|c| match t.kind(c).unwrap() {
                    NodeKind::Element => Some(from_tree(t, c)),
                    NodeKind::Text | NodeKind::CData => {
                        Some(C::Text(t.text(c).unwrap().unwrap().into_owned()))
                    }
                    _ => None,
                })
                .collect(),
        ),
    }
}

// ---- random edits ---------------------------------------------------------

const NAMESPACES: [Option<&str>; 6] = [
    None,
    Some("urn:a"),
    Some("urn:b"),
    Some("urn:c"),
    Some(XML_NS),
    Some(""),
];
const NAMES: [&str; 13] = [
    "a", "p:a", "q:b", "c", "xml:lang", "1x", "a:b:c", "xmlns:z", "", "é:ü", "ns0:a", "s:x", "r:d",
];
const TEXTS: [&str; 20] = [
    "", "]", "]]", ">", "]>", "]]>", "\r", "\n", "\r\n", "&", "<", "a", "--", "?>", "\u{0}",
    "\u{FFFE}", "\t", "é", "&amp;", "x]]",
];
const FRAGMENTS: [&str; 15] = [
    "<p:a p:x='1' y='2'><b>t</b><q:c/></p:a>",
    ">",
    "]]",
    "]",
    "<a/>t<b/>",
    "<!--c-->",
    "<?p d?>",
    "<![CDATA[]]]]>",
    "\r",
    "\n",
    "<p:a xmlns:p='urn:c'><p:b p:k='v'/></p:a>",
    "<a xmlns=''><b/></a>",
    "<b xmlns='urn:a'><c/></b>",
    "&#13;&#x0000000A;",
    "x&#93;&#93;",
];
const BINDINGS: [&[(&str, &str)]; 7] = [
    &[],
    &[("p", "urn:a"), ("q", "urn:b"), ("", "urn:c")],
    &[("p", "urn:b")],
    &[("p", "urn:a"), ("p", "urn:b")],
    &[("", "urn:a")],
    &[("q", "urn:a"), ("p", "urn:a")],
    &[("p", "urn:c"), ("q", "urn:c"), ("", "urn:b")],
];

/// Reads choices from the bytes, then zeros.
struct Choices<'a>(&'a [u8]);

impl Choices<'_> {
    fn byte(&mut self) -> u8 {
        match self.0.split_first() {
            Some((&b, rest)) => {
                self.0 = rest;
                b
            }
            None => 0,
        }
    }
    fn pick<T: Copy>(&mut self, xs: &[T]) -> T {
        xs[self.byte() as usize % xs.len()]
    }
    fn done(&self) -> bool {
        self.0.is_empty()
    }
}

struct State {
    pool: Vec<NodeId>,
    touched: HashSet<NodeId>,
    foreign: NodeId,
}

impl State {
    fn node(&self, c: &mut Choices<'_>) -> NodeId {
        let b = c.byte();
        if b == 255 {
            return self.foreign;
        }
        let k = (b as usize) | ((c.byte() as usize) << 8);
        self.pool[k % self.pool.len()]
    }

    /// Marks a node and its ancestors as touched.
    fn up(&mut self, t: &Tree, n: NodeId) {
        self.touched.insert(n);
        self.touched.extend(t.ancestors(n).unwrap());
    }

    /// Marks a node and everything below it as touched.
    fn down(&mut self, t: &Tree, n: NodeId) {
        self.touched.insert(n);
        self.touched.extend(t.descendants(n).unwrap());
    }

    fn add(&mut self, t: &Tree, n: NodeId) {
        self.pool.push(n);
        self.pool.extend(t.descendants(n).unwrap());
    }
}

/// An edit either works or gives an error; a foreign handle gives
/// `ForeignNode`. `refused` notes an edit the limits refused.
fn checked<T>(refused: &Cell<bool>, r: Result<T, Error>, foreign: bool) -> Option<T> {
    if matches!(r, Err(Error::Limit { .. })) {
        refused.set(true);
    }
    match r {
        Ok(v) => {
            assert!(!foreign, "a foreign handle was accepted");
            Some(v)
        }
        Err(Error::ForeignNode) => {
            assert!(foreign, "a node of the tree was called foreign");
            None
        }
        Err(_) => None,
    }
}

/// Applies the edits that `ops` describes to `xml` and checks the result.
pub fn run(xml: &str, ops: &[u8]) {
    let Ok(t) = Tree::parse(xml) else { return };
    // One run in four uses limits a little above what the document needs.
    let (limits, ops) = match ops {
        [l, n, d, rest @ ..] if l % 4 == 0 => {
            let doc = t.document();
            let nodes = t.descendants(doc).unwrap().count();
            let limits = Limits {
                max_xml_nodes: nodes + *n as usize % 12,
                max_depth: (depth(&t, doc) + *d as usize % 3).max(1),
                ..Limits::default()
            };
            (limits, rest)
        }
        _ => (Limits::default(), ops),
    };
    // A document exactly at its limits opens.
    let mut t = Tree::parse_with_limits(xml, &limits).expect("a document within its limits");
    let other = Tree::parse("<f><g/></f>").unwrap();
    let doc = t.document();
    let mut s = State {
        pool: std::iter::once(doc)
            .chain(t.descendants(doc).unwrap())
            .collect(),
        touched: HashSet::new(),
        foreign: other.root_element(),
    };
    let original = node_ranges(&t, xml.as_bytes());
    let mut c = Choices(ops);
    let mut steps = 0;
    while !c.done() && steps < 64 {
        steps += 1;
        let op = c.byte() % 14;
        let before = t.to_bytes();
        let nodes_before = t.descendants(doc).unwrap().count();
        let refused = Cell::new(false);
        match op {
            0 => {
                let (ns, name) = (c.pick(&NAMESPACES), c.pick(&NAMES));
                // roxmltree 0.21 refuses elements in the xml namespace, which
                // the namespaces recommendation allows; leave them out.
                if ns == Some(XML_NS) {
                    continue;
                }
                if let Some(e) = checked(&refused, t.create_element(ns, name), false) {
                    s.add(&t, e);
                }
            }
            1 => {
                if let Some(x) = checked(&refused, t.create_text(c.pick(&TEXTS)), false) {
                    s.add(&t, x);
                }
            }
            2 => {
                let n = s.node(&mut c);
                let f = n == s.foreign;
                let v = c.pick(&TEXTS);
                if checked(&refused, t.set_text(n, v), f).is_some() {
                    s.up(&t, n);
                    let kids: Vec<NodeId> = t.children(n).unwrap().collect();
                    for k in kids {
                        s.add(&t, k);
                    }
                }
            }
            3 => {
                let n = s.node(&mut c);
                let f = n == s.foreign;
                let (ns, name, v) = (c.pick(&NAMESPACES), c.pick(&NAMES), c.pick(&TEXTS));
                if checked(&refused, t.set_attribute(n, ns, name, v), f).is_some() {
                    s.up(&t, n);
                }
            }
            4 => {
                let n = s.node(&mut c);
                let f = n == s.foreign;
                let (ns, local) = (c.pick(&NAMESPACES), c.pick(&["x", "y", "z", "k", "lang"]));
                if checked(&refused, t.remove_attribute(n, ns, local), f) == Some(true) {
                    s.up(&t, n);
                }
            }
            5 => {
                let n = s.node(&mut c);
                let f = n == s.foreign;
                let (ns, name) = (c.pick(&NAMESPACES), c.pick(&NAMES));
                if ns == Some(XML_NS) {
                    continue;
                }
                if checked(&refused, t.set_name(n, ns, name), f).is_some() {
                    s.up(&t, n);
                    s.down(&t, n);
                }
            }
            6..=9 => {
                let (a, b) = (s.node(&mut c), s.node(&mut c));
                let f = a == s.foreign || b == s.foreign;
                let index = c.byte() as usize % 4;
                let old = (b != s.foreign).then(|| t.parent(b).unwrap()).flatten();
                let r = match op {
                    6 => t.append(a, b),
                    7 => t.insert(a, index, b),
                    8 => t.insert_before(a, b),
                    _ => t.insert_after(a, b),
                };
                if checked(&refused, r, f).is_some() {
                    if let Some(p) = old {
                        s.up(&t, p);
                    }
                    s.up(&t, b);
                    s.down(&t, b);
                }
            }
            10 => {
                let n = s.node(&mut c);
                let f = n == s.foreign;
                let old = (!f).then(|| t.parent(n).unwrap()).flatten();
                if checked(&refused, t.detach(n), f).is_some() {
                    if let Some(p) = old {
                        s.up(&t, p);
                    }
                    s.down(&t, n);
                }
            }
            11 => {
                let n = s.node(&mut c);
                let f = n == s.foreign;
                if let Some(copy) = checked(&refused, t.deep_clone(n), f) {
                    s.add(&t, copy);
                }
            }
            _ => {
                let (x, b) = (c.pick(&FRAGMENTS), c.pick(&BINDINGS));
                if let Some(nodes) = checked(&refused, t.parse_fragment(x, b), false) {
                    for n in nodes {
                        s.add(&t, n);
                    }
                }
            }
        }
        if refused.get() {
            assert!(t.to_bytes() == before, "a refused edit changed the tree");
            assert_eq!(t.descendants(doc).unwrap().count(), nodes_before);
        }
    }
    within(&t, &s.pool, &limits);
    check(&t, xml, &original, &s.touched);
}

/// The elements on the longest path down from `n`, `n` included.
fn depth(t: &Tree, n: NodeId) -> usize {
    let own = usize::from(t.kind(n).unwrap() == NodeKind::Element);
    own + t
        .children(n)
        .unwrap()
        .map(|k| depth(t, k))
        .max()
        .unwrap_or(0)
}

/// Every node the run made is in `pool`; together they fit the node limit,
/// and no tree, attached or not, is deeper than the depth limit.
fn within(t: &Tree, pool: &[NodeId], limits: &Limits) {
    let all: HashSet<NodeId> = pool.iter().copied().collect();
    // The document node does not count.
    assert!(
        all.len() - 1 <= limits.max_xml_nodes,
        "{} nodes, more than the limit of {}",
        all.len() - 1,
        limits.max_xml_nodes
    );
    for &n in &all {
        if t.parent(n).unwrap().is_none() {
            assert!(
                depth(t, n) <= limits.max_depth,
                "a tree {} deep, more than the limit of {}",
                depth(t, n),
                limits.max_depth
            );
        }
    }
}

/// The byte range of each node of `t`, which must be freshly parsed from
/// `src`. A scanner of its own finds the ranges, so the check does not rest
/// on the parser's own record of them. Adjacent text reads as one node.
pub fn node_ranges(t: &Tree, src: &[u8]) -> HashMap<NodeId, Range<usize>> {
    let mut found: Vec<Range<usize>> = vec![];
    let mut open: Vec<usize> = vec![];
    let find = |from: usize, pat: &[u8]| {
        src[from..]
            .windows(pat.len())
            .position(|w| w == pat)
            .map(|p| from + p + pat.len())
            .expect("a well-formed document")
    };
    let mut i = if src.starts_with(b"\xEF\xBB\xBF") {
        3
    } else {
        0
    };
    while i < src.len() {
        if src[i] != b'<' {
            let end = src[i..]
                .iter()
                .position(|&c| c == b'<')
                .map_or(src.len(), |p| i + p);
            found.push(i..end);
            i = end;
        } else if src[i..].starts_with(b"<!--") {
            let end = find(i + 4, b"-->");
            found.push(i..end);
            i = end;
        } else if src[i..].starts_with(b"<![CDATA[") {
            let end = find(i + 9, b"]]>");
            found.push(i..end);
            i = end;
        } else if src[i..].starts_with(b"<?") {
            let end = find(i + 2, b"?>");
            found.push(i..end);
            i = end;
        } else if src[i..].starts_with(b"</") {
            let end = find(i + 2, b">");
            let k = open.pop().expect("an open element");
            found[k].end = end;
            i = end;
        } else {
            // A start tag: '>' inside a quoted value does not end it.
            let mut j = i + 1;
            let mut quote = None;
            while quote.is_some() || src[j] != b'>' {
                match (quote, src[j]) {
                    (None, q @ (b'"' | b'\'')) => quote = Some(q),
                    (Some(q), c) if c == q => quote = None,
                    _ => {}
                }
                j += 1;
            }
            found.push(i..j + 1);
            if src[j - 1] != b'/' {
                open.push(found.len() - 1);
            }
            i = j + 1;
        }
    }
    let nodes: Vec<NodeId> = t.descendants(t.document()).unwrap().collect();
    assert_eq!(
        nodes.len(),
        found.len(),
        "the scanner and the parser disagree"
    );
    nodes.into_iter().zip(found).collect()
}

/// The checks that follow any edit sequence.
fn check(t: &Tree, xml: &str, original: &HashMap<NodeId, Range<usize>>, touched: &HashSet<NodeId>) {
    let out = t.to_bytes();
    let text = String::from_utf8(out.clone()).expect("UTF-8 in, UTF-8 out");
    let again = Tree::parse(out.clone())
        .unwrap_or_else(|e| panic!("the output does not parse: {e}\n{xml:?}\n=>\n{text:?}"));
    assert!(again.to_bytes() == out, "the output does not write back");
    let meant = from_tree(t, t.root_element());
    assert_eq!(
        from_tree(&again, again.root_element()),
        meant,
        "the output parses as another tree\n{xml:?}\n=>\n{text:?}"
    );
    assert_eq!(
        reference(&text),
        meant,
        "roxmltree reads the output as another tree\n{xml:?}\n=>\n{text:?}"
    );

    // Untouched nodes keep their bytes. Text is left out: adjacent text
    // reads back as one node, so only the other nodes pair up.
    let marked = |t: &Tree| -> Vec<NodeId> {
        t.descendants(t.document())
            .unwrap()
            .filter(|&n| t.kind(n).unwrap() != NodeKind::Text)
            .collect()
    };
    let ranges = node_ranges(&again, &out);
    let (a, b) = (marked(t), marked(&again));
    assert_eq!(a.len(), b.len());
    for (&n, &m) in a.iter().zip(&b) {
        assert_eq!(t.kind(n).unwrap(), again.kind(m).unwrap());
        let Some(r) = original.get(&n).cloned() else {
            continue;
        };
        if touched.contains(&n) {
            continue;
        }
        let r2 = ranges[&m].clone();
        assert!(
            xml.as_bytes()[r.clone()] == out[r2],
            "an untouched node changed: {:?}\n{xml:?}\n=>\n{text:?}",
            &xml[r]
        );
    }
}
