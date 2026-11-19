//! Lists: a fresh numbering instance for each list, so that each starts
//! at 1, over the definition of the list style.
//!
//! Which definition, in this order:
//!
//! 1. The definition that the list role's style is numbered by, if it has
//!    a level for every depth the list reaches and is not a single-level
//!    definition (for a list with nested items). All items take that style
//!    and their depth as the level, in one instance for the whole list.
//! 2. For nested lists over single-level definitions, as Word's own
//!    `List Number` and `List Number 2` are: the style of each depth
//!    (`List Number 2` for the second) with its own definition, and an
//!    instance for each list at each depth.
//! 3. A definition the writer adds, with nine levels, under the role's
//!    style. The numbering part is created if the document has none.

use std::collections::HashMap;

use crate::content::List;
use crate::view::{Numbering, StyleKind};
use crate::xml::NodeId;
use crate::{Document, Error, Target, ns};

use super::roles::Role;
use super::session::{Ctx, Session};
use super::xml::attr;

const W: &str = ns::W;
const NUMBERING_TYPE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering";
const NUMBERING_CONTENT: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml";

pub(crate) struct Lists {
    view: Numbering,
    part: Option<String>,
    looked: bool,
    next_abstract: u64,
    next_num: u64,
    /// The definitions the writer added, numbered or bulleted.
    own: HashMap<bool, String>,
    /// Where new definitions and instances go: before the first instance,
    /// and before `w:numIdMacAtCleanup` or at the end.
    anchors: Option<(Option<NodeId>, Option<NodeId>)>,
}

/// How a list is numbered.
enum Plan {
    /// One instance over this definition; items take this style.
    Shared { def: String, style: Option<String> },
    /// For each depth, a style and its definition.
    PerLevel(Vec<(String, String)>),
}

impl Lists {
    pub fn new(view: Numbering) -> Lists {
        let max = |it: &mut dyn Iterator<Item = &String>| {
            it.filter_map(|s| s.parse::<u64>().ok())
                .max()
                .map_or(0, |m| m + 1)
        };
        let next_abstract = max(&mut view.definitions.iter().map(|d| &d.id));
        let next_num = max(&mut view.instances.iter().map(|d| &d.id)).max(1);
        Lists {
            view,
            part: None,
            looked: false,
            next_abstract,
            next_num,
            own: HashMap::new(),
            anchors: None,
        }
    }

    /// The definition behind a numbering instance, with its level count
    /// and whether it is single-level.
    fn definition_of(&self, num: &str) -> Option<(String, usize, bool)> {
        let inst = self.view.instances.iter().find(|i| i.id == num)?;
        let def = inst.definition.as_ref()?;
        let d = self.view.definitions.iter().find(|d| &d.id == def)?;
        Some((
            d.id.clone(),
            d.levels.len(),
            d.kind.as_deref() == Some("singleLevel"),
        ))
    }
}

fn depth_of(l: &List) -> usize {
    let mut max = 0;
    let mut stack = vec![(l, 0usize)];
    while let Some((l, d)) = stack.pop() {
        max = max.max(d);
        for it in &l.items {
            if let Some(sub) = &it.list {
                stack.push((sub, d + 1));
            }
        }
    }
    max
}

fn own_definition(id: &str, ordered: bool, nsid: u32) -> String {
    let mut x = format!(
        "<w:abstractNum w:abstractNumId=\"{id}\"><w:nsid w:val=\"{nsid:08X}\"/><w:multiLevelType w:val=\"hybridMultilevel\"/>"
    );
    for l in 0..9 {
        let (fmt, text) = if ordered {
            match l % 3 {
                0 => ("decimal", format!("%{}.", l + 1)),
                1 => ("lowerLetter", format!("%{}.", l + 1)),
                _ => ("lowerRoman", format!("%{}.", l + 1)),
            }
        } else {
            (
                "bullet",
                match l % 3 {
                    0 => "\u{2022}",
                    1 => "\u{25E6}",
                    _ => "\u{25AA}",
                }
                .to_string(),
            )
        };
        x.push_str(&format!(
            "<w:lvl w:ilvl=\"{l}\"><w:start w:val=\"1\"/><w:numFmt w:val=\"{fmt}\"/><w:lvlText w:val=\"{}\"/><w:lvlJc w:val=\"left\"/><w:pPr><w:ind w:left=\"{}\" w:hanging=\"360\"/></w:pPr></w:lvl>",
            attr(&text),
            720 * (l + 1)
        ));
    }
    x.push_str("</w:abstractNum>");
    x
}

impl Session<'_> {
    /// The numbering part, created if the document has none.
    fn numbering_part(&mut self, doc: &mut Document) -> Result<String, Error> {
        if !self.lists.looked {
            self.lists.looked = true;
            let main = self.main.clone();
            self.lists.part = doc
                .relationships(&main)?
                .into_iter()
                .find(|r| r.kind.ends_with("/numbering"))
                .and_then(|r| r.part);
        }
        if let Some(p) = &self.lists.part {
            return Ok(p.clone());
        }
        let main = self.main.clone();
        let name = super::ids::free_part(doc, "word/numbering", "xml");
        let xml = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n<w:numbering xmlns:w=\"{W}\"></w:numbering>"
        );
        doc.add_part(&name, NUMBERING_CONTENT, xml.into_bytes())?;
        self.rel(doc, &main, NUMBERING_TYPE, Target::Part(&name))?;
        self.lists.part = Some(name.clone());
        Ok(name)
    }

    /// Adds a definition or an instance where the schema puts it.
    fn add_numbering(
        &mut self,
        doc: &mut Document,
        xml: &str,
        definition: bool,
    ) -> Result<(), Error> {
        let part = self.numbering_part(doc)?;
        let t = doc.xml(&part)?;
        let root = t.root_element();
        if self.lists.anchors.is_none() {
            let mut first_num = None;
            let mut cleanup = None;
            for c in t.elements(root)? {
                if first_num.is_none() && t.is(c, Some(W), "num")? {
                    first_num = Some(c);
                }
                if t.is(c, Some(W), "numIdMacAtCleanup")? {
                    cleanup = Some(c);
                }
            }
            self.lists.anchors = Some((first_num, cleanup));
        }
        let (first_num, cleanup) = self.lists.anchors.unwrap_or((None, None));
        let nodes = t.parse_fragment(xml, &[("w", W)])?;
        for n in nodes {
            let at = if definition {
                first_num.or(cleanup)
            } else {
                cleanup
            };
            match at {
                Some(a) => t.insert_before(a, n)?,
                None => t.append(root, n)?,
            }
            if !definition && first_num.is_none() {
                // The first instance: definitions now go before it.
                self.lists.anchors = Some((Some(n), cleanup));
            }
        }
        Ok(())
    }

    fn own_definition(&mut self, doc: &mut Document, ordered: bool) -> Result<String, Error> {
        if let Some(id) = self.lists.own.get(&ordered) {
            return Ok(id.clone());
        }
        let id = self.lists.next_abstract.to_string();
        self.lists.next_abstract += 1;
        let nsid = self.ids.para();
        let xml = own_definition(&id, ordered, nsid);
        self.add_numbering(doc, &xml, true)?;
        self.lists.own.insert(ordered, id.clone());
        Ok(id)
    }

    /// A fresh instance over a definition, starting at 1 at this level.
    fn instance(
        &mut self,
        doc: &mut Document,
        def: &str,
        level: usize,
        ordered: bool,
    ) -> Result<String, Error> {
        let id = self.lists.next_num.to_string();
        self.lists.next_num += 1;
        let restart = if ordered {
            format!(
                "<w:lvlOverride w:ilvl=\"{level}\"><w:startOverride w:val=\"1\"/></w:lvlOverride>"
            )
        } else {
            String::new()
        };
        let xml = format!(
            "<w:num w:numId=\"{id}\"><w:abstractNumId w:val=\"{}\"/>{restart}</w:num>",
            attr(def)
        );
        self.add_numbering(doc, &xml, false)?;
        Ok(id)
    }

    fn plan(&mut self, doc: &mut Document, ordered: bool, depth: usize) -> Result<Plan, Error> {
        let role = if ordered {
            Role::ListNumber
        } else {
            Role::ListBullet
        };
        let style = self.style_ref(doc, role)?;
        let numbered_by = |s: &Session<'_>, id: &str| {
            s.book
                .styles()
                .get(id)
                .and_then(|st| st.numbering.as_ref())
                .and_then(|n| s.lists.definition_of(&n.id))
        };
        if let Some(sid) = &style
            && let Some((def, levels, single)) = numbered_by(self, sid)
            && levels > depth
            && (depth == 0 || !single)
        {
            return Ok(Plan::Shared {
                def,
                style: Some(sid.clone()),
            });
        }
        if depth > 0
            && let Some(base) = role.built_in()
        {
            let mut per = vec![];
            for d in 0..=depth {
                let name = if d == 0 {
                    base.clone()
                } else {
                    format!("{base} {}", d + 1)
                };
                let Some(id) = self.book.find(StyleKind::Paragraph, &name) else {
                    break;
                };
                let Some((def, _, _)) = numbered_by(self, &id) else {
                    break;
                };
                per.push((id, def));
            }
            if per.len() == depth + 1 {
                return Ok(Plan::PerLevel(per));
            }
        }
        let def = self.own_definition(doc, ordered)?;
        Ok(Plan::Shared { def, style })
    }

    pub(crate) fn list(
        &mut self,
        doc: &mut Document,
        list: &List,
        ctx: &Ctx,
        path: &str,
    ) -> Result<String, Error> {
        struct Frame<'l> {
            list: &'l List,
            depth: usize,
            next: usize,
            path: String,
            plan: usize,
            num: Option<String>,
        }
        // A list nested in a list of the other kind (bullets in a numbered
        // list) is numbered by a plan of its own.
        let mut plans: Vec<Plan> = vec![self.plan(doc, list.ordered, depth_of(list))?];
        let first = match &plans[0] {
            Plan::Shared { def, .. } => {
                let def = def.clone();
                Some(self.instance(doc, &def, 0, list.ordered)?)
            }
            Plan::PerLevel(_) => None,
        };
        let mut out = String::new();
        // Lists nest at most nine deep (checked), so a stack of frames.
        let mut stack = vec![Frame {
            list,
            depth: 0,
            next: 0,
            path: format!("{path}.items"),
            plan: 0,
            num: first,
        }];
        while let Some(f) = stack.pop() {
            let Some(item) = f.list.items.get(f.next) else {
                continue;
            };
            let (style, num, level) = match &plans[f.plan] {
                Plan::Shared { style, .. } => {
                    (style.clone(), f.num.clone().unwrap_or_default(), f.depth)
                }
                Plan::PerLevel(per) => {
                    let Some((style, def)) = per.get(f.depth).cloned() else {
                        return Err(Error::Invalid(format!(
                            "{}: no list style for this depth",
                            f.path
                        )));
                    };
                    let num = match &f.num {
                        Some(n) => n.clone(),
                        None => self.instance(doc, &def, 0, f.list.ordered)?,
                    };
                    (Some(style), num, 0)
                }
            };
            let mut props = vec![];
            if let Some(s) = &style {
                props.push(("pStyle", format!("<w:pStyle w:val=\"{}\"/>", attr(s))));
            }
            props.push((
                "numPr",
                format!(
                    "<w:numPr><w:ilvl w:val=\"{level}\"/><w:numId w:val=\"{}\"/></w:numPr>",
                    attr(&num)
                ),
            ));
            if let Some(a) = ctx.align {
                let v = match a {
                    crate::content::Align::Left => "left",
                    crate::content::Align::Center => "center",
                    crate::content::Align::Right => "right",
                    crate::content::Align::Justify => "both",
                };
                props.push(("jc", format!("<w:jc w:val=\"{v}\"/>")));
            }
            let ip = format!("{}[{}]", f.path, f.next);
            let inner = self.inlines(doc, &item.content, ctx, &format!("{ip}.content"), None)?;
            out.push_str(&self.paragraph_xml(ctx, props, &inner));
            // The rest of this list after the nested one.
            stack.push(Frame {
                next: f.next + 1,
                num: Some(num.clone()),
                path: f.path.clone(),
                ..f
            });
            if let Some(sub) = &item.list {
                let depth = f.depth + 1;
                let (plan, num) = if sub.ordered == f.list.ordered {
                    match &plans[f.plan] {
                        Plan::Shared { .. } => (f.plan, Some(num)),
                        Plan::PerLevel(_) => (f.plan, None),
                    }
                } else {
                    let p = self.plan(doc, sub.ordered, depth + depth_of(sub))?;
                    let num = match &p {
                        Plan::Shared { def, .. } => {
                            let def = def.clone();
                            Some(self.instance(doc, &def, depth, sub.ordered)?)
                        }
                        Plan::PerLevel(_) => None,
                    };
                    plans.push(p);
                    (plans.len() - 1, num)
                };
                stack.push(Frame {
                    list: sub,
                    depth,
                    next: 0,
                    path: format!("{ip}.list.items"),
                    plan,
                    num,
                });
            }
        }
        Ok(out)
    }
}
