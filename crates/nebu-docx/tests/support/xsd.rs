//! A validator for the parts of a Word document, against the ECMA-376
//! transitional schemas in `tests/xsd/`.
//!
//! It knows the part of XML Schema those files use: global and local
//! elements and attributes, complex types with sequences, choices, `all`,
//! groups, `any` and occurrence bounds, extension of complex and simple
//! content, attribute groups, and simple types by restriction (enumeration,
//! pattern, length, bounds), list and union over the built-in types.
//! Content models become automata that the children run through, so an
//! element in the wrong place is found as Word's own validator finds it.
//!
//! Markup compatibility (ECMA-376 part 3) is applied first, as a consumer
//! that knows only these schemas applies it: elements and attributes of
//! namespaces that `mc:Ignorable` names are left out, and of
//! `mc:AlternateContent` the first `mc:Choice` whose required namespaces
//! the schemas define, else `mc:Fallback`, stands in its place.

#![allow(dead_code)]

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

const XSD: &str = "http://www.w3.org/2001/XMLSchema";
const XML: &str = "http://www.w3.org/XML/1998/namespace";
const MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";

type QName = (String, String);

#[derive(Debug, Clone)]
enum TypeRef {
    Named(QName),
    Anon(usize),
    /// No type: anything.
    Any,
}

#[derive(Debug, Clone)]
enum Particle {
    Element {
        name: QName,
        ty: TypeRef,
        min: u32,
        max: Option<u32>,
    },
    Ref {
        name: QName,
        min: u32,
        max: Option<u32>,
    },
    Seq(Vec<Particle>, u32, Option<u32>),
    Choice(Vec<Particle>, u32, Option<u32>),
    All(Vec<Particle>, u32, Option<u32>),
    Group(QName, u32, Option<u32>),
    Any {
        ns: AnyNs,
        lax: bool,
        min: u32,
        max: Option<u32>,
    },
}

#[derive(Debug, Clone)]
enum AnyNs {
    Any,
    Other(String),
    List(Vec<String>),
}

#[derive(Debug, Clone)]
enum AttrItem {
    Attr {
        name: QName,
        ty: TypeRef,
        required: bool,
    },
    Ref {
        name: QName,
        required: bool,
    },
    Group(QName),
}

#[derive(Debug, Clone, Default)]
struct Complex {
    base: Option<QName>,
    /// Extension of the base (true) or restriction.
    extends: bool,
    particle: Option<Particle>,
    attrs: Vec<AttrItem>,
    any_attr: bool,
    mixed: bool,
    /// The type of the text of simple content.
    text: Option<TypeRef>,
}

#[derive(Debug, Clone, Default)]
struct Facets {
    enums: Vec<String>,
    patterns: Vec<String>,
    length: Option<usize>,
    min_length: Option<usize>,
    max_length: Option<usize>,
    min_inc: Option<f64>,
    max_inc: Option<f64>,
    min_exc: Option<f64>,
    max_exc: Option<f64>,
}

#[derive(Debug, Clone)]
enum Simple {
    Builtin(String),
    Restrict(TypeRef, Facets),
    List(TypeRef),
    Union(Vec<TypeRef>),
}

#[derive(Debug, Clone)]
enum TypeDef {
    Complex(Complex),
    Simple(Simple),
}

/// The schemas, read once.
pub struct Schema {
    elements: HashMap<QName, TypeRef>,
    attributes: HashMap<QName, TypeRef>,
    types: HashMap<QName, usize>,
    defs: Vec<TypeDef>,
    groups: HashMap<QName, Particle>,
    attr_groups: HashMap<QName, Vec<AttrItem>>,
    namespaces: HashSet<String>,
    regex: std::sync::Mutex<HashMap<String, regex::Regex>>,
    automata: std::sync::Mutex<HashMap<usize, std::sync::Arc<Nfa>>>,
}

fn occurs(n: roxmltree::Node<'_, '_>) -> (u32, Option<u32>) {
    let min = n
        .attribute("minOccurs")
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);
    let max = match n.attribute("maxOccurs") {
        Some("unbounded") => None,
        Some(v) => Some(v.parse().unwrap_or(1)),
        None => Some(1),
    };
    (min, max)
}

fn is_xsd(n: roxmltree::Node<'_, '_>, local: &str) -> bool {
    n.is_element() && n.tag_name().namespace() == Some(XSD) && n.tag_name().name() == local
}

fn qref(n: roxmltree::Node<'_, '_>, v: &str) -> QName {
    let (p, l) = match v.split_once(':') {
        Some((p, l)) => (Some(p), l),
        None => (None, v),
    };
    let ns = if p == Some("xml") {
        XML
    } else {
        n.lookup_namespace_uri(p).unwrap_or_default()
    };
    (ns.to_string(), l.to_string())
}

struct Loader<'s> {
    s: &'s mut Schema,
    tns: String,
    qualified_attrs: bool,
}

impl Loader<'_> {
    fn anon(&mut self, d: TypeDef) -> TypeRef {
        self.s.defs.push(d);
        TypeRef::Anon(self.s.defs.len() - 1)
    }

    fn type_of(&mut self, n: roxmltree::Node<'_, '_>) -> TypeRef {
        if let Some(t) = n.attribute("type") {
            return TypeRef::Named(qref(n, t));
        }
        for c in n.children() {
            if is_xsd(c, "complexType") {
                let ct = self.complex(c);
                return self.anon(TypeDef::Complex(ct));
            }
            if is_xsd(c, "simpleType") {
                let st = self.simple(c);
                return self.anon(TypeDef::Simple(st));
            }
        }
        TypeRef::Any
    }

    fn particle(&mut self, n: roxmltree::Node<'_, '_>) -> Option<Particle> {
        let (min, max) = occurs(n);
        let kids = |l: &mut Self| -> Vec<Particle> {
            n.children()
                .filter(|c| c.is_element())
                .filter_map(|c| l.particle(c))
                .collect()
        };
        Some(match n.tag_name().name() {
            "element" => match n.attribute("ref") {
                Some(r) => Particle::Ref {
                    name: qref(n, r),
                    min,
                    max,
                },
                None => Particle::Element {
                    name: (self.tns.clone(), n.attribute("name")?.to_string()),
                    ty: self.type_of(n),
                    min,
                    max,
                },
            },
            "sequence" => Particle::Seq(kids(self), min, max),
            "choice" => Particle::Choice(kids(self), min, max),
            "all" => Particle::All(kids(self), min, max),
            "group" => Particle::Group(qref(n, n.attribute("ref")?), min, max),
            "any" => {
                let ns = match n.attribute("namespace").unwrap_or("##any") {
                    "##any" => AnyNs::Any,
                    "##other" => AnyNs::Other(self.tns.clone()),
                    list => AnyNs::List(
                        list.split_whitespace()
                            .map(|x| {
                                if x == "##targetNamespace" {
                                    self.tns.clone()
                                } else if x == "##local" {
                                    String::new()
                                } else {
                                    x.to_string()
                                }
                            })
                            .collect(),
                    ),
                };
                Particle::Any {
                    ns,
                    lax: n.attribute("processContents") != Some("strict"),
                    min,
                    max,
                }
            }
            _ => return None,
        })
    }

    fn attr_items(&mut self, n: roxmltree::Node<'_, '_>, out: &mut Vec<AttrItem>) -> bool {
        let mut any = false;
        for c in n.children() {
            if is_xsd(c, "attribute") {
                let required = c.attribute("use") == Some("required");
                if let Some(r) = c.attribute("ref") {
                    out.push(AttrItem::Ref {
                        name: qref(c, r),
                        required,
                    });
                } else if let Some(name) = c.attribute("name") {
                    let qualified = match c.attribute("form") {
                        Some(f) => f == "qualified",
                        None => self.qualified_attrs,
                    };
                    let ns = if qualified {
                        self.tns.clone()
                    } else {
                        String::new()
                    };
                    let ty = self.type_of(c);
                    out.push(AttrItem::Attr {
                        name: (ns, name.to_string()),
                        ty,
                        required,
                    });
                }
            } else if is_xsd(c, "attributeGroup") {
                if let Some(r) = c.attribute("ref") {
                    out.push(AttrItem::Group(qref(c, r)));
                }
            } else if is_xsd(c, "anyAttribute") {
                any = true;
            }
        }
        any
    }

    fn complex(&mut self, n: roxmltree::Node<'_, '_>) -> Complex {
        let mut ct = Complex {
            mixed: n.attribute("mixed") == Some("true"),
            ..Complex::default()
        };
        for c in n.children().filter(|c| c.is_element()) {
            match c.tag_name().name() {
                "sequence" | "choice" | "all" | "group" => ct.particle = self.particle(c),
                "complexContent" | "simpleContent" => {
                    let simple = c.tag_name().name() == "simpleContent";
                    if c.attribute("mixed") == Some("true") {
                        ct.mixed = true;
                    }
                    for d in c.children().filter(|d| d.is_element()) {
                        let extends = d.tag_name().name() == "extension";
                        let base = d.attribute("base").map(|b| qref(d, b));
                        if simple {
                            ct.text = base.map(TypeRef::Named);
                        } else {
                            ct.base = base;
                            ct.extends = extends;
                        }
                        for e in d.children().filter(|e| e.is_element()) {
                            if matches!(
                                e.tag_name().name(),
                                "sequence" | "choice" | "all" | "group"
                            ) {
                                ct.particle = self.particle(e);
                            }
                        }
                        ct.any_attr |= self.attr_items(d, &mut ct.attrs);
                    }
                }
                _ => {}
            }
        }
        ct.any_attr |= self.attr_items(n, &mut ct.attrs);
        ct
    }

    fn simple(&mut self, n: roxmltree::Node<'_, '_>) -> Simple {
        for c in n.children().filter(|c| c.is_element()) {
            match c.tag_name().name() {
                "restriction" => {
                    let base = match c.attribute("base") {
                        Some(b) => TypeRef::Named(qref(c, b)),
                        None => {
                            let inner = c.children().find(|d| is_xsd(*d, "simpleType"));
                            match inner {
                                Some(i) => {
                                    let st = self.simple(i);
                                    self.anon(TypeDef::Simple(st))
                                }
                                None => TypeRef::Any,
                            }
                        }
                    };
                    let mut f = Facets::default();
                    for d in c.children().filter(|d| d.is_element()) {
                        let v = d.attribute("value").unwrap_or_default();
                        let num = || v.parse::<f64>().ok();
                        match d.tag_name().name() {
                            "enumeration" => f.enums.push(v.to_string()),
                            "pattern" => f.patterns.push(v.to_string()),
                            "length" => f.length = v.parse().ok(),
                            "minLength" => f.min_length = v.parse().ok(),
                            "maxLength" => f.max_length = v.parse().ok(),
                            "minInclusive" => f.min_inc = num(),
                            "maxInclusive" => f.max_inc = num(),
                            "minExclusive" => f.min_exc = num(),
                            "maxExclusive" => f.max_exc = num(),
                            _ => {}
                        }
                    }
                    return Simple::Restrict(base, f);
                }
                "list" => {
                    let item = match c.attribute("itemType") {
                        Some(t) => TypeRef::Named(qref(c, t)),
                        None => match c.children().find(|d| is_xsd(*d, "simpleType")) {
                            Some(i) => {
                                let st = self.simple(i);
                                self.anon(TypeDef::Simple(st))
                            }
                            None => TypeRef::Any,
                        },
                    };
                    return Simple::List(item);
                }
                "union" => {
                    let mut members: Vec<TypeRef> = c
                        .attribute("memberTypes")
                        .unwrap_or_default()
                        .split_whitespace()
                        .map(|m| TypeRef::Named(qref(c, m)))
                        .collect();
                    for d in c.children().filter(|d| is_xsd(*d, "simpleType")) {
                        let st = self.simple(d);
                        members.push(self.anon(TypeDef::Simple(st)));
                    }
                    return Simple::Union(members);
                }
                _ => {}
            }
        }
        Simple::Builtin("string".into())
    }

    fn load(&mut self, doc: &roxmltree::Document<'_>) {
        let root = doc.root_element();
        for c in root.children().filter(|c| c.is_element()) {
            let name = c
                .attribute("name")
                .map(|n| (self.tns.clone(), n.to_string()));
            match c.tag_name().name() {
                "element" => {
                    if let Some(q) = name {
                        let t = self.type_of(c);
                        self.s.elements.insert(q, t);
                    }
                }
                "attribute" => {
                    if let Some(q) = name {
                        let t = self.type_of(c);
                        self.s.attributes.insert(q, t);
                    }
                }
                "complexType" => {
                    if let Some(q) = name {
                        let ct = self.complex(c);
                        self.s.defs.push(TypeDef::Complex(ct));
                        self.s.types.insert(q, self.s.defs.len() - 1);
                    }
                }
                "simpleType" => {
                    if let Some(q) = name {
                        let st = self.simple(c);
                        self.s.defs.push(TypeDef::Simple(st));
                        self.s.types.insert(q, self.s.defs.len() - 1);
                    }
                }
                "group" => {
                    if let Some(q) = name
                        && let Some(p) = c
                            .children()
                            .filter(|d| d.is_element())
                            .find_map(|d| self.particle(d))
                    {
                        self.s.groups.insert(q, p);
                    }
                }
                "attributeGroup" => {
                    if let Some(q) = name {
                        let mut items = vec![];
                        self.attr_items(c, &mut items);
                        self.s.attr_groups.insert(q, items);
                    }
                }
                _ => {}
            }
        }
    }
}

/// An automaton over the children of an element.
#[derive(Debug, Default)]
struct Nfa {
    /// For each state: epsilon moves, and moves on a term.
    eps: Vec<Vec<usize>>,
    moves: Vec<Vec<(usize, usize)>>,
    terms: Vec<Term>,
    start: usize,
    accept: usize,
}

#[derive(Debug, Clone)]
enum Term {
    Element(QName, TypeRef),
    Any(AnyNs, bool),
}

impl Nfa {
    fn state(&mut self) -> usize {
        self.eps.push(vec![]);
        self.moves.push(vec![]);
        self.eps.len() - 1
    }

    fn closure(&self, set: &mut Vec<bool>) {
        let mut stack: Vec<usize> = (0..set.len()).filter(|&i| set[i]).collect();
        while let Some(s) = stack.pop() {
            for &t in &self.eps[s] {
                if !set[t] {
                    set[t] = true;
                    stack.push(t);
                }
            }
        }
    }
}

const BUILTINS: &[&str] = &[
    "string",
    "token",
    "boolean",
    "int",
    "long",
    "short",
    "byte",
    "integer",
    "unsignedInt",
    "unsignedLong",
    "unsignedShort",
    "unsignedByte",
    "decimal",
    "double",
    "float",
    "hexBinary",
    "base64Binary",
    "dateTime",
    "anyURI",
    "NCName",
    "ID",
    "QName",
];

impl Schema {
    /// The schemas of `tests/xsd`.
    pub fn get() -> &'static Schema {
        static S: OnceLock<Schema> = OnceLock::new();
        S.get_or_init(|| {
            let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/xsd");
            let mut s = Schema {
                elements: HashMap::new(),
                attributes: HashMap::new(),
                types: HashMap::new(),
                defs: vec![],
                groups: HashMap::new(),
                attr_groups: HashMap::new(),
                namespaces: HashSet::new(),
                regex: Default::default(),
                automata: Default::default(),
            };
            for b in BUILTINS {
                s.defs.push(TypeDef::Simple(Simple::Builtin(b.to_string())));
                s.types
                    .insert((XSD.into(), b.to_string()), s.defs.len() - 1);
            }
            // xml:space
            s.defs.push(TypeDef::Simple(Simple::Restrict(
                TypeRef::Named((XSD.into(), "token".into())),
                Facets {
                    enums: vec!["default".into(), "preserve".into()],
                    ..Facets::default()
                },
            )));
            let space = TypeRef::Anon(s.defs.len() - 1);
            s.attributes.insert((XML.into(), "space".into()), space);
            s.attributes.insert(
                (XML.into(), "lang".into()),
                TypeRef::Named((XSD.into(), "string".into())),
            );
            let mut files: Vec<_> = std::fs::read_dir(&dir)
                .expect("tests/xsd")
                .map(|e| e.unwrap().path())
                .filter(|p| p.extension().is_some_and(|e| e == "xsd"))
                .collect();
            files.sort();
            for f in files {
                let text = std::fs::read_to_string(&f).unwrap();
                let doc = roxmltree::Document::parse(&text).unwrap();
                let root = doc.root_element();
                let tns = root
                    .attribute("targetNamespace")
                    .unwrap_or_default()
                    .to_string();
                s.namespaces.insert(tns.clone());
                let qualified_attrs = root.attribute("attributeFormDefault") == Some("qualified");
                let mut l = Loader {
                    s: &mut s,
                    tns,
                    qualified_attrs,
                };
                l.load(&doc);
            }
            s.namespaces.insert(XML.into());
            s
        })
    }

    fn def(&self, t: &TypeRef) -> Option<&TypeDef> {
        match t {
            TypeRef::Named(q) => self.types.get(q).map(|&i| &self.defs[i]),
            TypeRef::Anon(i) => self.defs.get(*i),
            TypeRef::Any => None,
        }
    }

    fn def_index(&self, t: &TypeRef) -> Option<usize> {
        match t {
            TypeRef::Named(q) => self.types.get(q).copied(),
            TypeRef::Anon(i) => Some(*i),
            TypeRef::Any => None,
        }
    }

    /// The content particle of a complex type with its bases.
    fn content(&self, ct: &Complex, out: &mut Vec<Particle>) {
        if ct.extends
            && let Some(b) = &ct.base
            && let Some(TypeDef::Complex(base)) = self.def(&TypeRef::Named(b.clone()))
        {
            self.content(base, out);
        }
        if let Some(p) = &ct.particle {
            out.push(p.clone());
        }
    }

    fn attrs_of(&self, ct: &Complex, out: &mut Vec<(QName, TypeRef, bool)>, any: &mut bool) {
        if let Some(b) = &ct.base
            && let Some(TypeDef::Complex(base)) = self.def(&TypeRef::Named(b.clone()))
        {
            self.attrs_of(base, out, any);
        }
        *any |= ct.any_attr;
        let mut stack: Vec<AttrItem> = ct.attrs.iter().rev().cloned().collect();
        let mut guard = 0;
        while let Some(a) = stack.pop() {
            guard += 1;
            if guard > 10_000 {
                break;
            }
            match a {
                AttrItem::Attr { name, ty, required } => out.push((name, ty, required)),
                AttrItem::Ref { name, required } => {
                    let ty = self.attributes.get(&name).cloned().unwrap_or(TypeRef::Any);
                    out.push((name, ty, required));
                }
                AttrItem::Group(g) => {
                    if let Some(items) = self.attr_groups.get(&g) {
                        stack.extend(items.iter().rev().cloned());
                    }
                }
            }
        }
    }

    fn build(&self, nfa: &mut Nfa, p: &Particle, from: usize, depth: usize) -> usize {
        if depth > 200 {
            return from;
        }
        let (min, max) = match p {
            Particle::Element { min, max, .. }
            | Particle::Ref { min, max, .. }
            | Particle::Seq(_, min, max)
            | Particle::Choice(_, min, max)
            | Particle::All(_, min, max)
            | Particle::Group(_, min, max)
            | Particle::Any { min, max, .. } => (*min, *max),
        };
        let mut at = from;
        for _ in 0..min {
            at = self.once(nfa, p, at, depth);
        }
        match max {
            None => {
                // Zero or more further times.
                let loop_start = nfa.state();
                nfa.eps[at].push(loop_start);
                let end = self.once(nfa, p, loop_start, depth);
                nfa.eps[end].push(loop_start);
                let out = nfa.state();
                nfa.eps[loop_start].push(out);
                out
            }
            Some(m) => {
                let out = nfa.state();
                for _ in min..m {
                    nfa.eps[at].push(out);
                    at = self.once(nfa, p, at, depth);
                }
                nfa.eps[at].push(out);
                out
            }
        }
    }

    fn once(&self, nfa: &mut Nfa, p: &Particle, from: usize, depth: usize) -> usize {
        match p {
            Particle::Element { name, ty, .. } => {
                let to = nfa.state();
                nfa.terms.push(Term::Element(name.clone(), ty.clone()));
                nfa.moves[from].push((nfa.terms.len() - 1, to));
                to
            }
            Particle::Ref { name, .. } => {
                let to = nfa.state();
                let ty = self.elements.get(name).cloned().unwrap_or(TypeRef::Any);
                nfa.terms.push(Term::Element(name.clone(), ty));
                nfa.moves[from].push((nfa.terms.len() - 1, to));
                to
            }
            Particle::Any { ns, lax, .. } => {
                let to = nfa.state();
                nfa.terms.push(Term::Any(ns.clone(), *lax));
                nfa.moves[from].push((nfa.terms.len() - 1, to));
                to
            }
            Particle::Seq(items, ..) => {
                let mut at = from;
                for i in items {
                    at = self.build(nfa, i, at, depth + 1);
                }
                at
            }
            Particle::Choice(items, ..) | Particle::All(items, ..) => {
                let out = nfa.state();
                if let Particle::All(..) = p {
                    // Any order: as a repeated choice, which accepts more
                    // than xsd:all; the schemas use it once.
                    let loop_start = nfa.state();
                    nfa.eps[from].push(loop_start);
                    for i in items {
                        let s = nfa.state();
                        nfa.eps[loop_start].push(s);
                        let e = self.build(nfa, i, s, depth + 1);
                        nfa.eps[e].push(loop_start);
                    }
                    nfa.eps[loop_start].push(out);
                    return out;
                }
                for i in items {
                    let s = nfa.state();
                    nfa.eps[from].push(s);
                    let e = self.build(nfa, i, s, depth + 1);
                    nfa.eps[e].push(out);
                }
                out
            }
            Particle::Group(g, ..) => match self.groups.get(g) {
                Some(inner) => self.build(nfa, &inner.clone(), from, depth + 1),
                None => from,
            },
        }
    }

    fn automaton(&self, def: usize, ct: &Complex) -> std::sync::Arc<Nfa> {
        if let Some(a) = self.automata.lock().unwrap().get(&def) {
            return a.clone();
        }
        let mut parts = vec![];
        self.content(ct, &mut parts);
        let mut nfa = Nfa::default();
        let start = nfa.state();
        let mut at = start;
        for p in &parts {
            at = self.build(&mut nfa, p, at, 0);
        }
        nfa.start = start;
        nfa.accept = at;
        let a = std::sync::Arc::new(nfa);
        self.automata.lock().unwrap().insert(def, a.clone());
        a
    }

    fn regex(&self, pattern: &str) -> regex::Regex {
        let mut m = self.regex.lock().unwrap();
        m.entry(pattern.to_string())
            .or_insert_with(|| regex::Regex::new(&format!("^(?:{pattern})$")).unwrap())
            .clone()
    }

    /// Whether a value is of a simple type; the reason if not.
    fn check_simple(&self, t: &TypeRef, v: &str, depth: usize) -> Result<(), String> {
        if depth > 64 {
            return Ok(());
        }
        let Some(def) = self.def(t) else {
            return Ok(());
        };
        let TypeDef::Simple(s) = def else {
            return Ok(());
        };
        match s {
            Simple::Builtin(b) => builtin(b, v),
            Simple::List(item) => {
                for x in v.split_whitespace() {
                    self.check_simple(item, x, depth + 1)?;
                }
                Ok(())
            }
            Simple::Union(members) => {
                if members
                    .iter()
                    .any(|m| self.check_simple(m, v, depth + 1).is_ok())
                {
                    Ok(())
                } else {
                    Err(format!("{v:?} is of none of the union's types"))
                }
            }
            Simple::Restrict(base, f) => {
                let trimmed = v.trim();
                self.check_simple(base, v, depth + 1)?;
                if !f.enums.is_empty() && !f.enums.iter().any(|e| e == v || e == trimmed) {
                    return Err(format!("{v:?} is not one of {:?}", f.enums));
                }
                for p in &f.patterns {
                    if !self.regex(p).is_match(v) {
                        return Err(format!("{v:?} does not match {p}"));
                    }
                }
                let hex = self.is_hex(base);
                let len = if hex { v.len() / 2 } else { v.chars().count() };
                if let Some(l) = f.length
                    && len != l
                {
                    return Err(format!("{v:?} is not {l} long"));
                }
                if let Some(l) = f.min_length
                    && len < l
                {
                    return Err(format!("{v:?} is shorter than {l}"));
                }
                if let Some(l) = f.max_length
                    && len > l
                {
                    return Err(format!("{v:?} is longer than {l}"));
                }
                if f.min_inc.is_some()
                    || f.max_inc.is_some()
                    || f.min_exc.is_some()
                    || f.max_exc.is_some()
                {
                    let n: f64 = match trimmed.parse() {
                        Ok(n) => n,
                        Err(_) => return Err(format!("{v:?} is not a number")),
                    };
                    if f.min_inc.is_some_and(|m| n < m)
                        || f.max_inc.is_some_and(|m| n > m)
                        || f.min_exc.is_some_and(|m| n <= m)
                        || f.max_exc.is_some_and(|m| n >= m)
                    {
                        return Err(format!("{v:?} is out of range"));
                    }
                }
                Ok(())
            }
        }
    }

    fn is_hex(&self, t: &TypeRef) -> bool {
        let mut cur = t.clone();
        for _ in 0..16 {
            match self.def(&cur) {
                Some(TypeDef::Simple(Simple::Builtin(b))) => return b == "hexBinary",
                Some(TypeDef::Simple(Simple::Restrict(b, _))) => cur = b.clone(),
                _ => return false,
            }
        }
        false
    }
}

fn builtin(b: &str, v: &str) -> Result<(), String> {
    let t = v.trim();
    let int = |lo: i128, hi: i128| -> Result<(), String> {
        match t.parse::<i128>() {
            Ok(n) if n >= lo && n <= hi => Ok(()),
            _ => Err(format!("{v:?} is not an {b}")),
        }
    };
    match b {
        "boolean" => match t {
            "true" | "false" | "1" | "0" => Ok(()),
            _ => Err(format!("{v:?} is not a boolean")),
        },
        "int" => int(i32::MIN as i128, i32::MAX as i128),
        "long" => int(i64::MIN as i128, i64::MAX as i128),
        "short" => int(i16::MIN as i128, i16::MAX as i128),
        "byte" => int(i8::MIN as i128, i8::MAX as i128),
        "integer" => int(i128::MIN, i128::MAX),
        "unsignedInt" => int(0, u32::MAX as i128),
        "unsignedLong" => int(0, u64::MAX as i128),
        "unsignedShort" => int(0, u16::MAX as i128),
        "unsignedByte" => int(0, u8::MAX as i128),
        "decimal" | "double" | "float" => {
            if t.parse::<f64>().is_ok() || matches!(t, "INF" | "-INF" | "NaN") {
                Ok(())
            } else {
                Err(format!("{v:?} is not a number"))
            }
        }
        "hexBinary" => {
            if t.len() % 2 == 0 && t.bytes().all(|c| c.is_ascii_hexdigit()) {
                Ok(())
            } else {
                Err(format!("{v:?} is not hexBinary"))
            }
        }
        _ => Ok(()),
    }
}

/// A child after markup compatibility: the element and the namespaces
/// that are ignorable where it stands.
fn children<'a, 'i>(
    n: roxmltree::Node<'a, 'i>,
    ignorable: &HashSet<String>,
    known: &HashSet<String>,
    out: &mut Vec<(roxmltree::Node<'a, 'i>, HashSet<String>)>,
) {
    for c in n.children().filter(|c| c.is_element()) {
        let ns = c.tag_name().namespace().unwrap_or_default();
        let mut ign = ignorable.clone();
        if let Some(list) = c.attribute((MC, "Ignorable")) {
            for p in list.split_whitespace() {
                if let Some(u) = c.lookup_namespace_uri(Some(p)) {
                    ign.insert(u.to_string());
                }
            }
        }
        if ns == MC && c.tag_name().name() == "AlternateContent" {
            let mut chosen = None;
            for b in c.children().filter(|b| b.is_element()) {
                match b.tag_name().name() {
                    "Choice" => {
                        let ok = b
                            .attribute("Requires")
                            .unwrap_or_default()
                            .split_whitespace()
                            .all(|p| {
                                b.lookup_namespace_uri(Some(p))
                                    .is_some_and(|u| known.contains(u))
                            });
                        if ok && chosen.is_none() {
                            chosen = Some(b);
                        }
                    }
                    "Fallback" => {
                        if chosen.is_none() {
                            chosen = Some(b);
                        }
                    }
                    _ => {}
                }
            }
            if let Some(b) = chosen {
                children(b, &ign, known, out);
            }
            continue;
        }
        if ign.contains(ns) && !known.contains(ns) {
            continue;
        }
        out.push((c, ign));
    }
}

fn shown(n: roxmltree::Node<'_, '_>) -> String {
    let t = n.tag_name();
    let p = t
        .namespace()
        .and_then(|u| n.lookup_prefix(u))
        .unwrap_or_default();
    if p.is_empty() {
        t.name().to_string()
    } else {
        format!("{p}:{}", t.name())
    }
}

struct Validator<'s> {
    s: &'s Schema,
    errors: Vec<String>,
}

impl Validator<'_> {
    fn err(&mut self, path: &str, what: String) {
        if self.errors.len() < 50 {
            self.errors.push(format!("{path}: {what}"));
        }
    }

    fn element(
        &mut self,
        n: roxmltree::Node<'_, '_>,
        ty: &TypeRef,
        ign: &HashSet<String>,
        path: &str,
        depth: usize,
    ) {
        if depth > 300 {
            return;
        }
        let s = self.s;
        let Some(def_i) = s.def_index(ty) else { return };
        let def = &s.defs[def_i];
        let ct = match def {
            TypeDef::Simple(_) => {
                let text: String = n.children().filter_map(|c| c.text()).collect();
                if n.children().any(|c| c.is_element()) {
                    self.err(path, "has elements, but its type is text".into());
                }
                if let Err(e) = s.check_simple(ty, &text, 0) {
                    self.err(path, e);
                }
                for a in n.attributes() {
                    let ns = a.namespace().unwrap_or_default();
                    if ns.is_empty() || ns == MC || ign.contains(ns) {
                        continue;
                    }
                    self.err(path, format!("attribute {} is not allowed", a.name()));
                }
                return;
            }
            TypeDef::Complex(ct) => ct,
        };
        // Attributes.
        let mut declared = vec![];
        let mut any_attr = false;
        s.attrs_of(ct, &mut declared, &mut any_attr);
        let mut seen = HashSet::new();
        for a in n.attributes() {
            let ns = a.namespace().unwrap_or_default().to_string();
            if ns == MC || (ign.contains(&ns) && !s.namespaces.contains(&ns)) {
                continue;
            }
            let q = (ns.clone(), a.name().to_string());
            match declared.iter().find(|(n, ..)| *n == q) {
                Some((_, t, _)) => {
                    seen.insert(q.clone());
                    if let Err(e) = s.check_simple(t, a.value(), 0) {
                        self.err(path, format!("attribute {}: {e}", a.name()));
                    }
                }
                None if any_attr => {}
                None => self.err(
                    path,
                    format!(
                        "attribute {}{} is not allowed",
                        if ns.is_empty() {
                            String::new()
                        } else {
                            format!("{{{ns}}}")
                        },
                        a.name()
                    ),
                ),
            }
        }
        for (q, _, required) in &declared {
            if *required && !seen.contains(q) {
                self.err(path, format!("attribute {} is missing", q.1));
            }
        }
        // Text.
        if let Some(t) = &ct.text {
            let text: String = n.children().filter_map(|c| c.text()).collect();
            if let Err(e) = s.check_simple(t, &text, 0) {
                self.err(path, e);
            }
            return;
        }
        if !ct.mixed
            && n.children()
                .any(|c| c.is_text() && !c.text().unwrap_or_default().trim().is_empty())
        {
            self.err(path, "has text, but its type holds only elements".into());
        }
        // Children through the automaton.
        let mut kids = vec![];
        children(n, ign, &s.namespaces, &mut kids);
        let nfa = s.automaton(def_i, ct);
        let mut cur = vec![false; nfa.eps.len()];
        cur[nfa.start] = true;
        nfa.closure(&mut cur);
        let mut counts: HashMap<String, usize> = HashMap::new();
        for (c, cign) in kids {
            let q = (
                c.tag_name().namespace().unwrap_or_default().to_string(),
                c.tag_name().name().to_string(),
            );
            let name = shown(c);
            let k = counts.entry(name.clone()).or_insert(0);
            *k += 1;
            let cpath = format!("{path}/{name}[{k}]");
            let mut next = vec![false; nfa.eps.len()];
            let mut matched: Option<Term> = None;
            for (st, on) in cur.iter().enumerate() {
                if !on {
                    continue;
                }
                for &(term, to) in &nfa.moves[st] {
                    let hit = match &nfa.terms[term] {
                        Term::Element(tq, _) => *tq == q,
                        Term::Any(ns, _) => match ns {
                            AnyNs::Any => true,
                            AnyNs::Other(t) => q.0 != *t && !q.0.is_empty(),
                            AnyNs::List(l) => l.contains(&q.0),
                        },
                    };
                    if hit {
                        next[to] = true;
                        // A declared element wins over a wildcard.
                        if matched.is_none() || matches!(nfa.terms[term], Term::Element(..)) {
                            matched = Some(nfa.terms[term].clone());
                        }
                    }
                }
            }
            if matched.is_none() {
                let mut expected: Vec<String> = vec![];
                for (st, on) in cur.iter().enumerate() {
                    if *on {
                        for &(term, _) in &nfa.moves[st] {
                            if let Term::Element(tq, _) = &nfa.terms[term] {
                                expected.push(tq.1.clone());
                            }
                        }
                    }
                }
                expected.sort();
                expected.dedup();
                expected.truncate(12);
                self.err(
                    &cpath,
                    format!("not allowed here; expected one of {expected:?}"),
                );
                // Carry on as if it were not there.
                continue;
            }
            nfa.closure(&mut next);
            cur = next;
            match matched {
                Some(Term::Element(_, t)) => self.element(c, &t, &cign, &cpath, depth + 1),
                Some(Term::Any(_, true)) => {
                    if let Some(t) = s.elements.get(&q).cloned() {
                        self.element(c, &t, &cign, &cpath, depth + 1);
                    }
                }
                _ => {}
            }
        }
        if !cur[nfa.accept] {
            let mut expected: Vec<String> = vec![];
            for (st, on) in cur.iter().enumerate() {
                if *on {
                    for &(term, _) in &nfa.moves[st] {
                        if let Term::Element(tq, _) = &nfa.terms[term] {
                            expected.push(tq.1.clone());
                        }
                    }
                }
            }
            expected.sort();
            expected.dedup();
            expected.truncate(12);
            self.err(
                path,
                format!("ends too early; it needs one of {expected:?}"),
            );
        }
    }
}

/// Validates one XML part. Returns the errors, at most 50.
pub fn validate(xml: &[u8]) -> Vec<String> {
    let s = Schema::get();
    let text = match std::str::from_utf8(xml) {
        Ok(t) => t.trim_start_matches('\u{FEFF}').to_string(),
        Err(_) => return vec!["not UTF-8".into()],
    };
    let doc = match roxmltree::Document::parse(&text) {
        Ok(d) => d,
        Err(e) => return vec![format!("not well-formed: {e}")],
    };
    let root = doc.root_element();
    let mut ign = HashSet::new();
    if let Some(list) = root.attribute((MC, "Ignorable")) {
        for p in list.split_whitespace() {
            if let Some(u) = root.lookup_namespace_uri(Some(p)) {
                ign.insert(u.to_string());
            }
        }
    }
    let q = (
        root.tag_name().namespace().unwrap_or_default().to_string(),
        root.tag_name().name().to_string(),
    );
    let Some(t) = s.elements.get(&q).cloned() else {
        return vec![format!(
            "{} is not an element the schemas declare",
            shown(root)
        )];
    };
    let mut v = Validator { s, errors: vec![] };
    let path = format!("/{}", shown(root));
    v.element(root, &t, &ign, &path, 0);
    v.errors
}

/// The parts of a document in WordprocessingML that the schemas cover.
pub fn wml_parts(entries: &[(String, Vec<u8>)]) -> Vec<(String, Vec<u8>)> {
    entries
        .iter()
        .filter(|(n, _)| {
            n.starts_with("word/")
                && n.ends_with(".xml")
                && !n.contains("/_rels/")
                && !n.contains("theme/")
                && !n.contains("glossary/")
                && !n.contains("commentsExtended")
                && !n.contains("commentsIds")
                && !n.contains("commentsExtensible")
                && !n.contains("people.xml")
                && !n.starts_with("word/media/")
        })
        .cloned()
        .collect()
}

/// Validates every WordprocessingML part of a .docx and returns the errors
/// with their parts.
pub fn validate_docx(bytes: &[u8]) -> Vec<String> {
    let mut out = vec![];
    for (name, xml) in wml_parts(&super::entries(bytes)) {
        for e in validate(&xml) {
            out.push(format!("{name}{e}"));
        }
    }
    out
}
