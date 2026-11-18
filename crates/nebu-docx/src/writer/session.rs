//! One insertion: what the writer learns about the document first, the
//! state it keeps while it writes, and the order of the steps.

use std::collections::{HashMap, HashSet};

use crate::content::{Block, Inline, check_label};
use crate::view::{BlockKind, StoryKind};
use crate::xml::{NodeId, Tree};
use crate::{Document, Error, ns};

use super::ids::Ids;
use super::media::Media;
use super::numbering::Lists;
use super::place::{self, Anchor, Target};
use super::roles::{Role, StyleBook};
use super::{Inserted, Place, WriteOptions};

const W: &str = ns::W;
const W14: &str = "http://schemas.microsoft.com/office/word/2010/wordml";
const MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";

/// What a label names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LabelKind {
    /// A caption with this label word.
    Caption,
    Heading,
    Equation,
    /// A label the document had before: a `_Ref_` bookmark.
    Document,
}

#[derive(Debug, Clone)]
pub(crate) struct Label {
    pub name: String,
    pub bookmark: String,
    pub kind: LabelKind,
}

/// The context a block is written in.
#[derive(Debug, Clone)]
pub(crate) struct Ctx {
    /// The part the block goes into, which relationships start from.
    pub part: String,
    pub story: StoryKind,
    pub in_cell: bool,
    pub in_box: bool,
    /// The width available, in points.
    pub width: f64,
    /// Whether paragraphs get a `w14:paraId`.
    pub para_ids: bool,
    /// The role of plain paragraphs here.
    pub text: Role,
    /// The alignment of the cell the paragraphs are in.
    pub align: Option<crate::content::Align>,
    /// What the place is called in an error.
    pub place: &'static str,
}

/// The state of one insertion.
pub(crate) struct Session<'o> {
    pub opts: &'o WriteOptions,
    pub main: String,
    pub book: StyleBook,
    pub ids: Ids,
    pub labels: HashMap<String, Label>,
    /// Bookmark names of the document, in lower case.
    pub bookmarks: HashSet<String>,
    pub lists: Lists,
    pub media: Media,
    /// The relationships of each source part, read once: the next free
    /// number, and kind and target to id.
    pub rels: HashMap<String, (u64, HashMap<String, String>)>,
    pub notes_part: Option<String>,
    pub next_note: i64,
    /// Parts whose root declares `w14` as ignorable.
    pub para_parts: HashMap<String, bool>,
    /// Runs the next paragraph begins with: a footnote's mark.
    pub lead: Option<String>,
    /// The table styles tables got, by id.
    pub table_styles: Vec<String>,
}

/// The styles part of the document, if it has one.
pub(crate) fn styles_part(doc: &mut Document) -> Result<Option<String>, Error> {
    let main = doc.main_part().to_string();
    Ok(doc
        .relationships(&main)?
        .into_iter()
        .find(|r| r.kind.ends_with("/styles") && r.part.is_some())
        .and_then(|r| r.part))
}

/// True if the tree's root binds a prefix to `w14` and lists it in
/// `mc:Ignorable`, as files that Word saves do. Only then does the writer
/// give new paragraphs a `w14:paraId`, so that it adds no namespace a
/// reader that does not know it must refuse.
pub(crate) fn ignorable_w14(t: &Tree) -> Result<bool, Error> {
    let root = t.root_element();
    let Some(ign) = t.attribute(root, Some(MC), "Ignorable")? else {
        return Ok(false);
    };
    for p in ign.split_whitespace() {
        if t.namespace_of_prefix(root, p)? == Some(W14) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The labels the content defines, with their kinds, in order.
fn content_labels(blocks: &[Block], out: &mut Vec<(String, LabelKind)>) {
    // Explicit stacks: content nests as deep as its limits allow.
    let mut stack: Vec<&Block> = blocks.iter().rev().collect();
    let mut inl: Vec<&Inline> = vec![];
    while let Some(b) = stack.pop() {
        match b {
            Block::Heading { label, content, .. } => {
                if let Some(l) = label {
                    out.push((l.clone(), LabelKind::Heading));
                }
                inl.extend(content.iter());
            }
            Block::Table(t) => {
                if let Some(l) = &t.label {
                    out.push((l.clone(), LabelKind::Caption));
                }
                for r in t.rows.iter().rev() {
                    for c in r.cells.iter().rev() {
                        stack.extend(c.content.iter().rev());
                    }
                }
            }
            Block::Figure { label: Some(l), .. } => out.push((l.clone(), LabelKind::Caption)),
            Block::Equation { label: Some(l), .. } => out.push((l.clone(), LabelKind::Equation)),
            Block::Paragraph { content, .. } => inl.extend(content.iter()),
            _ => {}
        }
        while let Some(i) = inl.pop() {
            match i {
                Inline::Footnote { content } => stack.extend(content.iter().rev()),
                Inline::Link { content, .. } => inl.extend(content.iter()),
                _ => {}
            }
        }
    }
}

pub(crate) fn run(
    doc: &mut Document,
    items: &[(Place, &[Block])],
    opts: &WriteOptions,
) -> Result<Inserted, Error> {
    let stories = place::stories(doc)?;
    let styles_part = styles_part(doc)?;
    let main = doc.main_part().to_string();
    // What the views know, taken before the first change.
    let v = doc.views()?;
    let styles = v.styles().clone();
    let numbering = v.numbering().clone();
    let mut bookmarks = HashSet::new();
    let mut labels: HashMap<String, Label> = HashMap::new();
    for b in v.bookmarks() {
        bookmarks.insert(b.name.to_lowercase());
        if let Some(l) = b.name.strip_prefix("_Ref_")
            && b.location.story == StoryKind::Main
        {
            labels.insert(
                l.to_lowercase(),
                Label {
                    name: l.to_string(),
                    bookmark: b.name.clone(),
                    kind: LabelKind::Document,
                },
            );
        }
    }
    // The text width of the section of each top-level block of the body.
    let mut width_of: HashMap<NodeId, f64> = HashMap::new();
    let sections = v.sections().to_vec();
    let mut first_width = None;
    {
        let index: HashMap<_, _> = v
            .blocks()
            .iter()
            .enumerate()
            .map(|(i, b)| (b.id, i))
            .collect();
        let blocks = v.blocks();
        for s in &sections {
            let w = s.text_width.filter(|w| *w > 36.0);
            if first_width.is_none() {
                first_width = w;
            }
            let (Some(a), Some(z)) = (
                s.first_block.and_then(|b| index.get(&b)),
                s.last_block.and_then(|b| index.get(&b)),
            ) else {
                continue;
            };
            for b in blocks.iter().take(*z + 1).skip(*a) {
                if let Some(w) = w {
                    width_of.insert(b.node, w);
                }
            }
        }
        // The final section properties belong to the last section.
        if let Some(last) = blocks.last()
            && last.kind == BlockKind::Section
            && let Some(w) = sections.last().and_then(|s| s.text_width)
        {
            width_of.insert(last.node, w);
        }
    }
    let page_width = first_width.unwrap_or(451.3);
    let parts: Vec<String> = stories.iter().map(|(p, _)| p.clone()).collect();
    let ids = Ids::collect(doc, &parts, super::ids::seed(doc))?;
    let notes_part = stories
        .iter()
        .find(|(_, k)| *k == StoryKind::Footnote)
        .map(|(p, _)| p.clone());
    let mut s = Session {
        opts,
        main: main.clone(),
        book: StyleBook::new(styles, styles_part),
        ids,
        labels,
        bookmarks,
        lists: Lists::new(numbering),
        media: Media::default(),
        rels: HashMap::new(),
        notes_part,
        next_note: 0,
        para_parts: HashMap::new(),
        lead: None,
        table_styles: vec![],
    };
    // The labels of the content, before anything is written, so that a
    // reference may come before its target.
    let mut defined = vec![];
    for (_, blocks) in items {
        content_labels(blocks, &mut defined);
    }
    let mut given = HashSet::new();
    for (l, kind) in defined {
        check_label(&l).map_err(Error::Invalid)?;
        let key = l.to_lowercase();
        if !given.insert(key.clone()) {
            return Err(Error::Invalid(format!(
                "the label {l:?} is given twice in the content"
            )));
        }
        if s.labels.contains_key(&key) {
            return Err(Error::Invalid(format!(
                "the label {l:?} is already in the document (bookmark _Ref_{l})"
            )));
        }
        s.labels.insert(
            key,
            Label {
                bookmark: format!("_Ref_{l}"),
                name: l,
                kind,
            },
        );
    }
    // Resolve every place before the first change.
    let mut targets = vec![];
    for (p, _) in items {
        targets.push(place::resolve(doc, &stories, *p)?);
    }
    let mut out = Inserted::default();
    let mut pending: Vec<(&Target, Vec<NodeId>, bool)> = vec![];
    for (t, (_, blocks)) in targets.iter().zip(items) {
        let para_ids = match s.para_parts.get(&t.part) {
            Some(b) => *b,
            None => {
                let b = ignorable_w14(doc.xml(&t.part)?)?;
                s.para_parts.insert(t.part.clone(), b);
                b
            }
        };
        let width = t
            .cell_width
            .map(|w| (w - 10.8).max(36.0))
            .or_else(|| t.top.and_then(|n| width_of.get(&n).copied()))
            .unwrap_or(page_width);
        let ctx = Ctx {
            part: t.part.clone(),
            story: t.story,
            in_cell: t.in_cell,
            in_box: t.in_box,
            width,
            para_ids,
            text: if t.in_cell {
                Role::TableText
            } else {
                match t.story {
                    StoryKind::Footnote | StoryKind::Endnote => Role::FootnoteText,
                    _ => Role::Body,
                }
            },
            align: None,
            place: t.name(),
        };
        let mut xml = String::new();
        for (i, b) in blocks.iter().enumerate() {
            xml.push_str(&s.block(doc, b, &ctx, &format!("blocks[{i}]"))?);
        }
        let nodes = doc
            .xml(&t.part)?
            .parse_fragment(&xml, crate::content::RAW_PREFIXES)?;
        pending.push((t, nodes, para_ids));
    }
    put(doc, &pending, &mut s.ids)?;
    out.nodes = pending.into_iter().map(|(_, n, _)| n).collect();
    out.renumbered = super::fields::update(doc, &s, &out.nodes)?;
    out.roles = s.book.sources();
    out.styles_added = s.book.added_names.clone();
    out.table_styles = s.table_styles.clone();
    let mut labels: Vec<(String, String)> = s
        .labels
        .values()
        .filter(|l| l.kind != LabelKind::Document)
        .map(|l| (l.name.clone(), l.bookmark.clone()))
        .collect();
    labels.sort();
    out.labels = labels;
    doc.writes += 1;
    Ok(out)
}

/// Puts the parsed blocks at their places: in one pass over the children
/// of each container, so that many places cost no more than one. A
/// container that must end with a paragraph (a cell, a note, a header, a
/// text box) gets an empty one if the new blocks leave it without.
fn put(
    doc: &mut Document,
    pending: &[(&Target, Vec<NodeId>, bool)],
    ids: &mut Ids,
) -> Result<(), Error> {
    type Edits = HashMap<NodeId, (Vec<NodeId>, Vec<NodeId>, bool)>;
    type Group = ((String, NodeId), Edits, Vec<NodeId>, bool);
    let mut groups: Vec<Group> = vec![];
    let mut index: HashMap<(String, NodeId), usize> = HashMap::new();
    for (t, nodes, para_ids) in pending {
        let key = (t.part.clone(), t.container);
        let g = *index.entry(key.clone()).or_insert_with(|| {
            groups.push((key, HashMap::new(), vec![], *para_ids));
            groups.len() - 1
        });
        let (_, edits, appends, _) = &mut groups[g];
        match t.anchor {
            Anchor::Before(a) => edits.entry(a).or_default().0.extend(nodes),
            Anchor::After(a) => edits.entry(a).or_default().1.extend(nodes),
            Anchor::Replace(a) => {
                let e = edits.entry(a).or_default();
                if e.2 {
                    return Err(Error::Invalid("two places replace the same block".into()));
                }
                e.0.extend(nodes);
                e.2 = true;
            }
            Anchor::Append => appends.extend(nodes),
        }
    }
    for ((part, container), edits, appends, para_ids) in groups {
        let tree = doc.xml(&part)?;
        if !edits.is_empty() {
            tree.splice(container, &edits)?;
        }
        for n in appends {
            tree.append(container, n)?;
        }
        let needs_paragraph = !tree.is(container, Some(W), "body")?
            && !tree.is(container, Some(W), "sdtContent")?
            && !tree.is(container, Some(W), "customXml")?;
        if needs_paragraph {
            let last = tree.elements(container)?.last();
            let ends_in_p = match last {
                Some(l) => tree.is(l, Some(W), "p")?,
                None => false,
            };
            if !ends_in_p {
                let id = if para_ids {
                    format!(" w14:paraId=\"{:08X}\"", ids.para())
                } else {
                    String::new()
                };
                let p =
                    tree.parse_fragment(&format!("<w:p{id}/>"), crate::content::RAW_PREFIXES)?;
                for n in p {
                    tree.append(container, n)?;
                }
            }
        }
    }
    Ok(())
}
