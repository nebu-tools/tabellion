//! Property tests: random legal XML with namespaces, references, comments,
//! CDATA and processing instructions, and random edits on it. roxmltree, an
//! independent parser, is the reference for what the XML means.

use std::collections::BTreeMap;

use nebu_docx::xml::{NodeId, NodeKind, Tree};
use proptest::prelude::*;

const URIS: [&str; 3] = ["urn:a", "urn:b", "urn:c"];
const PREFIXES: [&str; 4] = ["", "p", "q", "r"];
const LOCALS: [&str; 4] = ["a", "b", "c", "d"];
const ATTRS: [&str; 3] = ["x", "y", "z"];

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
    fn pick<'a>(&mut self, xs: &[&'a str]) -> &'a str {
        xs[self.below(xs.len())]
    }
}

fn text(r: &mut Rng, quote: Option<char>) -> String {
    let pieces = [
        "a",
        "bc",
        " ",
        "\n",
        "\r\n",
        "\t",
        "&amp;",
        "&lt;",
        "&gt;",
        "&#169;",
        "&#x1F426;",
        "é",
        "]",
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
            0..=2 => out.push_str(&text(r, None)),
            3 => out.push_str(&format!("<!--{}-->", ["", " c ", "-x", "a-b"][r.below(4)])),
            4 => out.push_str(&format!("<?t{}?>", ["", " data", " a?b"][r.below(3)])),
            5 => out.push_str(&format!(
                "<![CDATA[{}]]>",
                ["", "x < y & z", "]", "a]]b"][r.below(4)].replace("]]b", "] ]b")
            )),
            _ => element(r, &scope, depth + 1, out),
        }
    }
    out.push_str(&format!("</{qname}{}>", space(r)));
}

fn document(seed: u64) -> String {
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

// ---- a reference model of what the XML means -----------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum C {
    El {
        ns: Option<String>,
        local: String,
        attrs: BTreeMap<(Option<String>, String), String>,
        children: Vec<C>,
    },
    Text(String),
}

/// Merges adjacent text and drops empty text, as XML does not tell them apart.
fn tidy(children: Vec<C>) -> Vec<C> {
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

fn from_rox(n: roxmltree::Node<'_, '_>) -> C {
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

fn reference(xml: &str) -> C {
    let d = roxmltree::Document::parse(xml).unwrap_or_else(|e| panic!("{e}\n{xml}"));
    from_rox(d.root_element())
}

fn from_tree(t: &Tree, id: NodeId) -> C {
    let name = t.name(id).unwrap();
    C::El {
        ns: name.namespace.map(String::from),
        local: name.local.to_string(),
        attrs: t
            .attributes(id)
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
                .iter()
                .filter_map(|&c| match t.kind(c) {
                    NodeKind::Element => Some(from_tree(t, c)),
                    NodeKind::Text | NodeKind::CData => {
                        Some(C::Text(t.text(c).unwrap().into_owned()))
                    }
                    _ => None,
                })
                .collect(),
        ),
    }
}

/// The model as an arena, so that edits can address elements as the tree
/// does.
struct Model {
    els: Vec<MEl>,
}

struct MEl {
    ns: Option<String>,
    local: String,
    attrs: BTreeMap<(Option<String>, String), String>,
    children: Vec<MChild>,
    parent: Option<usize>,
}

enum MChild {
    El(usize),
    Text(String),
}

impl Model {
    fn add(&mut self, c: &C, parent: Option<usize>) -> usize {
        let C::El {
            ns,
            local,
            attrs,
            children,
        } = c
        else {
            unreachable!()
        };
        let id = self.els.len();
        self.els.push(MEl {
            ns: ns.clone(),
            local: local.clone(),
            attrs: attrs.clone(),
            children: vec![],
            parent,
        });
        for ch in children {
            let m = match ch {
                C::Text(t) => MChild::Text(t.clone()),
                el => MChild::El(self.add(el, Some(id))),
            };
            self.els[id].children.push(m);
        }
        id
    }

    fn canon(&self, id: usize) -> C {
        let e = &self.els[id];
        C::El {
            ns: e.ns.clone(),
            local: e.local.clone(),
            attrs: e.attrs.clone(),
            children: tidy(
                e.children
                    .iter()
                    .map(|c| match c {
                        MChild::El(i) => self.canon(*i),
                        MChild::Text(t) => C::Text(t.clone()),
                    })
                    .collect(),
            ),
        }
    }

    fn detach(&mut self, id: usize) {
        if let Some(p) = self.els[id].parent.take() {
            self.els[p]
                .children
                .retain(|c| !matches!(c, MChild::El(i) if *i == id));
        }
    }

    fn inside(&self, id: usize, ancestor: usize) -> bool {
        let mut cur = Some(id);
        while let Some(c) = cur {
            if c == ancestor {
                return true;
            }
            cur = self.els[c].parent;
        }
        false
    }
}

/// Elements of the tree and the model in document order, paired.
fn pairs(t: &Tree, m: &Model) -> Vec<(NodeId, usize)> {
    let mut out = vec![];
    let mut stack = vec![(t.root_element(), 0usize)];
    while let Some((n, i)) = stack.pop() {
        out.push((n, i));
        let tk: Vec<NodeId> = t.elements(n).collect();
        let mk: Vec<usize> = m.els[i]
            .children
            .iter()
            .filter_map(|c| {
                if let MChild::El(k) = c {
                    Some(*k)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(tk.len(), mk.len());
        for (a, b) in tk.into_iter().zip(mk).rev() {
            stack.push((a, b));
        }
    }
    out
}

#[derive(Debug, Clone)]
enum Edit {
    SetAttr(usize, usize, usize, String),
    RemoveAttr(usize, usize, usize),
    Append(usize, usize, usize),
    SetText(usize, String),
    Detach(usize),
    Move(usize, usize),
    Rename(usize, usize, usize),
}

fn edit_strategy() -> impl Strategy<Value = Edit> {
    let k = 0usize..64;
    let ns = 0usize..4;
    let value = "[a-z &<>\"'\t\n\u{e9}]{0,6}";
    prop_oneof![
        (k.clone(), ns.clone(), 0usize..3, value)
            .prop_map(|(k, n, l, v)| Edit::SetAttr(k, n, l, v)),
        (k.clone(), ns.clone(), 0usize..3).prop_map(|(k, n, l)| Edit::RemoveAttr(k, n, l)),
        (k.clone(), ns.clone(), 0usize..4).prop_map(|(k, n, l)| Edit::Append(k, n, l)),
        (k.clone(), value).prop_map(|(k, v)| Edit::SetText(k, v)),
        k.clone().prop_map(Edit::Detach),
        (k.clone(), k.clone()).prop_map(|(a, b)| Edit::Move(a, b)),
        (k, ns, 0usize..4).prop_map(|(k, n, l)| Edit::Rename(k, n, l)),
    ]
}

fn ns_of(i: usize) -> Option<&'static str> {
    URIS.get(i).copied()
}

fn qname(ns: Option<&str>, hint: &str, local: &str) -> String {
    if ns.is_some() {
        format!("{hint}:{local}")
    } else {
        local.to_string()
    }
}

/// Applies an edit to both. Returns the element whose source bytes contain
/// the whole change, if there is one.
fn apply(t: &mut Tree, m: &mut Model, edit: &Edit) -> Option<NodeId> {
    let ps = pairs(t, m);
    let pick = |k: usize| ps[k % ps.len()];
    match edit {
        Edit::SetAttr(k, n, l, v) => {
            let (node, i) = pick(*k);
            let ns = ns_of(*n);
            t.set_attribute(node, ns, &qname(ns, "s", ATTRS[*l]), v)
                .unwrap();
            m.els[i].attrs.insert(
                (ns.map(String::from), ATTRS[*l].to_string()),
                v.replace("\r\n", "\n"),
            );
            Some(node)
        }
        Edit::RemoveAttr(k, n, l) => {
            let (node, i) = pick(*k);
            let ns = ns_of(*n);
            let had = t.remove_attribute(node, ns, ATTRS[*l]);
            assert_eq!(
                had,
                m.els[i]
                    .attrs
                    .remove(&(ns.map(String::from), ATTRS[*l].to_string()))
                    .is_some()
            );
            Some(node)
        }
        Edit::Append(k, n, l) => {
            let (node, i) = pick(*k);
            let ns = ns_of(*n);
            let e = t.create_element(ns, &qname(ns, "p", LOCALS[*l])).unwrap();
            t.append(node, e).unwrap();
            let id = m.els.len();
            m.els.push(MEl {
                ns: ns.map(String::from),
                local: LOCALS[*l].to_string(),
                attrs: BTreeMap::new(),
                children: vec![],
                parent: Some(i),
            });
            m.els[i].children.push(MChild::El(id));
            Some(node)
        }
        Edit::SetText(k, v) => {
            let (node, i) = pick(*k);
            t.set_text(node, v).unwrap();
            for c in std::mem::take(&mut m.els[i].children) {
                if let MChild::El(e) = c {
                    m.els[e].parent = None;
                }
            }
            m.els[i].children.push(MChild::Text(v.clone()));
            Some(node)
        }
        Edit::Detach(k) => {
            let (node, i) = pick(*k);
            if i == 0 {
                assert!(t.detach(node).is_err());
                return None;
            }
            let parent = t.parent(node);
            t.detach(node).unwrap();
            m.detach(i);
            parent
        }
        Edit::Move(a, b) => {
            let ((na, ia), (nb, ib)) = (pick(*a), pick(*b));
            if ia == 0 || m.inside(ib, ia) {
                assert!(t.append(nb, na).is_err());
                return None;
            }
            t.append(nb, na).unwrap();
            m.detach(ia);
            m.els[ia].parent = Some(ib);
            m.els[ib].children.push(MChild::El(ia));
            None
        }
        Edit::Rename(k, n, l) => {
            let (node, i) = pick(*k);
            let ns = ns_of(*n);
            if t.set_name(node, ns, &qname(ns, "n", LOCALS[*l])).is_ok() {
                m.els[i].ns = ns.map(String::from);
                m.els[i].local = LOCALS[*l].to_string();
            }
            Some(node)
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 1000, ..ProptestConfig::default() })]

    #[test]
    fn unchanged_trees_write_their_source(seed in any::<u64>()) {
        let xml = document(seed);
        let t = Tree::parse(xml.clone()).unwrap_or_else(|e| panic!("{e}\n{xml}"));
        prop_assert_eq!(String::from_utf8(t.to_bytes()).unwrap(), xml.clone());
        prop_assert_eq!(from_tree(&t, t.root_element()), reference(&xml));
    }

    #[test]
    fn edits_mean_what_they_say(seed in any::<u64>(), edits in proptest::collection::vec(edit_strategy(), 1..8)) {
        let xml = document(seed);
        let mut t = Tree::parse(xml.clone()).unwrap();
        let mut m = Model { els: vec![] };
        m.add(&reference(&xml), None);
        for e in &edits {
            apply(&mut t, &mut m, e);
        }
        let out = String::from_utf8(t.to_bytes()).unwrap();
        prop_assert_eq!(reference(&out), m.canon(0), "{}\n=>\n{}", xml, out);
        // The output reads back and writes the same again.
        let again = Tree::parse(out.clone()).unwrap();
        prop_assert_eq!(String::from_utf8(again.to_bytes()).unwrap(), out);
    }

    #[test]
    fn an_edit_stays_inside_its_target(seed in any::<u64>(), edit in edit_strategy()) {
        let xml = document(seed);
        let mut t = Tree::parse(xml.clone()).unwrap();
        let mut m = Model { els: vec![] };
        m.add(&reference(&xml), None);
        if let Some(target) = apply(&mut t, &mut m, &edit) {
            let r = t.source_range(target).unwrap();
            let out = t.to_bytes();
            let src = xml.as_bytes();
            prop_assert!(out.starts_with(&src[..r.start]));
            prop_assert!(out.ends_with(&src[r.end..]));
        }
    }
}

#[test]
fn fragments_keep_their_meaning_under_any_bindings() {
    for seed in 0..300u64 {
        let xml = document(seed);
        let mut t = Tree::parse(xml.clone()).unwrap();
        let ps: Vec<NodeId> = std::iter::once(t.root_element())
            .chain(t.descendants(t.root_element()))
            .filter(|&n| t.is_element(n))
            .collect();
        let target = ps[seed as usize % ps.len()];
        let frag = t
            .parse_fragment(
                "<p:a p:x='1' y='2'><b>t</b><q:c/></p:a>",
                &[("p", "urn:a"), ("q", "urn:b"), ("", "urn:c")],
            )
            .unwrap();
        t.append(target, frag[0]).unwrap();
        let out = String::from_utf8(t.to_bytes()).unwrap();
        let d = roxmltree::Document::parse(&out).unwrap_or_else(|e| panic!("{e}\n{out}"));
        let a = d
            .descendants()
            .rfind(|n| n.tag_name().name() == "a" && n.attribute(("urn:a", "x")) == Some("1"))
            .unwrap();
        assert_eq!(a.tag_name().namespace(), Some("urn:a"));
        let kids: Vec<_> = a.children().filter(|n| n.is_element()).collect();
        assert_eq!(kids[0].tag_name().namespace(), Some("urn:c"), "{out}");
        assert_eq!(kids[1].tag_name().namespace(), Some("urn:b"));
    }
}
