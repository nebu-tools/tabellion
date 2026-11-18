//! Property tests: random legal XML with namespaces, references, comments,
//! CDATA and processing instructions, and random edits on it. roxmltree, an
//! independent parser, is the reference for what the XML means.

mod support;

use std::collections::BTreeMap;

use nebu_docx::xml::{NodeId, Tree};
use proptest::prelude::*;
use support::edits::{BOUNDARY_DOCUMENTS, C, URIS, document, from_tree, reference, tidy};

const LOCALS: [&str; 4] = ["a", "b", "c", "d"];
const ATTRS: [&str; 3] = ["x", "y", "z"];

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
        let tk: Vec<NodeId> = t.elements(n).unwrap().collect();
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
            let had = t.remove_attribute(node, ns, ATTRS[*l]).unwrap();
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
            let parent = t.parent(node).unwrap();
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

    /// Any edit sequence, including ones that put text next to text, move
    /// nodes between parents that bind prefixes differently, and use handles
    /// of another tree, leaves a tree that writes well-formed XML meaning
    /// what the tree says, with untouched nodes as they were.
    #[test]
    fn any_edit_sequence_writes_what_the_tree_says(
        seed in any::<u64>(),
        boundary in any::<bool>(),
        ops in proptest::collection::vec(any::<u8>(), 0..200),
    ) {
        let xml = if boundary {
            BOUNDARY_DOCUMENTS[seed as usize % BOUNDARY_DOCUMENTS.len()].to_string()
        } else {
            document(seed)
        };
        support::edits::run(&xml, &ops);
    }

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
            let r = t.source_range(target).unwrap().unwrap();
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
            .chain(t.descendants(t.root_element()).unwrap())
            .filter(|&n| t.is_element(n).unwrap())
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
