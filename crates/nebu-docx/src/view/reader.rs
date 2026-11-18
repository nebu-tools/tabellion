//! Finds the parts and stories of a document, walks them with a sink and
//! puts the views together.

use std::collections::{HashMap, VecDeque};

use crate::xml::{NodeId, Tree};
use crate::{Document, Error};

use super::collect::{
    BlocksSink, Collect, Common, FieldsSink, MarksSink, Rel, RevisionsSink, TablesSink, TextSink,
    merge_revisions, qualified, roman,
};
use super::props::{self, child_val, on_off};
use super::styles::Styles;
use super::walk::{self, W};
use super::{
    Block, BlockKind, Comment, ContentControl, Field, HeaderFooter, Image, Locator, Margins, Note,
    NoteKind, Numbering, NumberingRef, Revision, Revisions, Section, StoryKind, Table, TextOptions,
};
use super::{Bookmark, Control};

const COMMENTS_EXTENDED: &str = "commentsExtended";

/// The parts a document's views read.
#[derive(Default)]
struct Parts {
    styles: Option<String>,
    numbering: Option<String>,
    footnotes: Option<String>,
    endnotes: Option<String>,
    comments: Option<String>,
    comments_extended: Option<String>,
    headers: Vec<String>,
    footers: Vec<String>,
}

/// A story to walk.
#[derive(Debug, Clone)]
struct Story {
    kind: StoryKind,
    part: String,
    root: NodeId,
    /// The note or comment id.
    id: Option<String>,
}

pub(crate) struct Reader<'d> {
    doc: &'d Document,
    pub styles: Styles,
    main: String,
    parts: Parts,
    rels: HashMap<String, HashMap<String, Rel>>,
    content_types: HashMap<String, String>,
    /// The top-level blocks of the body with their ids and kinds.
    blocks: Vec<(NodeId, String, BlockKind)>,
    ids: HashMap<NodeId, String>,
    no_rels: HashMap<String, Rel>,
}

/// The last segment of a relationship type.
fn rel_kind(kind: &str) -> &str {
    kind.rsplit('/').next().unwrap_or_default()
}

impl<'d> Reader<'d> {
    pub fn new(doc: &'d mut Document) -> Result<Reader<'d>, Error> {
        let main = doc.main_part().to_string();
        let main_rels = doc.relationships(&main)?;
        let mut parts = Parts::default();
        for r in &main_rels {
            let Some(p) = r.part.clone() else { continue };
            let slot = match rel_kind(&r.kind) {
                "styles" => &mut parts.styles,
                "numbering" => &mut parts.numbering,
                "footnotes" => &mut parts.footnotes,
                "endnotes" => &mut parts.endnotes,
                "comments" => &mut parts.comments,
                COMMENTS_EXTENDED => &mut parts.comments_extended,
                "header" => {
                    parts.headers.push(p);
                    continue;
                }
                "footer" => {
                    parts.footers.push(p);
                    continue;
                }
                _ => continue,
            };
            if slot.is_none() {
                *slot = Some(p);
            }
        }
        let mut story_parts = vec![main.clone()];
        story_parts.extend(parts.headers.iter().cloned());
        story_parts.extend(parts.footers.iter().cloned());
        story_parts.extend(
            [&parts.footnotes, &parts.endnotes, &parts.comments]
                .into_iter()
                .flatten()
                .cloned(),
        );
        let mut rels = HashMap::new();
        let mut content_types = HashMap::new();
        for p in &story_parts {
            doc.xml(p)?;
            let list = doc.relationships(p)?;
            let mut map = HashMap::new();
            for r in list {
                if let Some(target) = &r.part
                    && let Some(t) = doc.content_type(target)
                {
                    content_types.insert(target.to_ascii_lowercase(), t);
                }
                map.insert(
                    r.id.clone(),
                    Rel {
                        target: r.target,
                        external: r.external,
                        part: r.part,
                    },
                );
            }
            rels.insert(p.to_ascii_lowercase(), map);
        }
        for p in [&parts.styles, &parts.numbering, &parts.comments_extended]
            .into_iter()
            .flatten()
        {
            doc.xml(p)?;
        }
        let doc: &'d Document = doc;
        let styles = Styles::read(parts.styles.as_deref().and_then(|p| doc.parsed(p)))?;
        let mut reader = Reader {
            doc,
            styles,
            main,
            parts,
            rels,
            content_types,
            blocks: vec![],
            ids: HashMap::new(),
            no_rels: HashMap::new(),
        };
        reader.find_blocks()?;
        Ok(reader)
    }

    fn tree(&self, part: &str) -> Result<&'d Tree, Error> {
        self.doc
            .parsed(part)
            .ok_or_else(|| Error::MissingPart(part.to_string()))
    }

    fn rels_of(&self, part: &str) -> &HashMap<String, Rel> {
        self.rels
            .get(&part.to_ascii_lowercase())
            .unwrap_or(&self.no_rels)
    }

    fn body(&self) -> Result<Option<NodeId>, Error> {
        let t = self.tree(&self.main)?;
        t.child(t.root_element(), Some(W), "body")
    }

    /// Lists the top-level blocks and gives them ids. A `w14:paraId` that
    /// two blocks share gives neither a `p:` or `t:` id.
    fn find_blocks(&mut self) -> Result<(), Error> {
        let Some(body) = self.body()? else {
            return Ok(());
        };
        let t = self.tree(&self.main)?;
        let mut list = vec![];
        for n in t.children(body)? {
            if !walk::listed(t, n)? {
                continue;
            }
            let name = t.name(n)?.map(|m| (m.namespace == Some(W), m.local));
            let kind = match name {
                Some((true, "p")) => BlockKind::Paragraph,
                Some((true, "tbl")) => BlockKind::Table,
                Some((true, "sdt")) => BlockKind::ContentControl,
                Some((true, "sectPr")) => BlockKind::Section,
                Some((true, "bookmarkStart" | "bookmarkEnd")) => BlockKind::Bookmark,
                _ => BlockKind::Other,
            };
            let para = match kind {
                BlockKind::Paragraph => props::para_id(t, n)?.map(|id| format!("p:{id}")),
                BlockKind::Table => {
                    let mut id = None;
                    for d in t.descendants(n)? {
                        if t.is(d, Some(W), "p")? {
                            id = props::para_id(t, d)?.map(|id| format!("t:{id}"));
                            break;
                        }
                    }
                    id
                }
                _ => None,
            };
            list.push((n, para, kind));
        }
        let mut seen: HashMap<String, usize> = HashMap::new();
        for (_, id, _) in &list {
            if let Some(id) = id {
                *seen.entry(id[2..].to_string()).or_default() += 1;
            }
        }
        for (i, (n, id, kind)) in list.into_iter().enumerate() {
            let id = match id {
                Some(id) if seen.get(&id[2..]) == Some(&1) => id,
                _ => format!("#{i}"),
            };
            self.ids.insert(n, id.clone());
            self.blocks.push((n, id, kind));
        }
        Ok(())
    }

    fn stories(&self, main_only: bool) -> Result<Vec<Story>, Error> {
        let mut out = vec![];
        if let Some(body) = self.body()? {
            out.push(Story {
                kind: StoryKind::Main,
                part: self.main.clone(),
                root: body,
                id: None,
            });
        }
        if main_only {
            return Ok(out);
        }
        for (list, kind) in [
            (&self.parts.headers, StoryKind::Header),
            (&self.parts.footers, StoryKind::Footer),
        ] {
            for p in list {
                out.push(Story {
                    kind,
                    part: p.clone(),
                    root: self.tree(p)?.root_element(),
                    id: None,
                });
            }
        }
        out.extend(self.notes_stories(NoteKind::Footnote)?);
        out.extend(self.notes_stories(NoteKind::Endnote)?);
        out.extend(self.comment_stories()?);
        Ok(out)
    }

    fn notes_stories(&self, kind: NoteKind) -> Result<Vec<Story>, Error> {
        let (part, local, story) = match kind {
            NoteKind::Footnote => (&self.parts.footnotes, "footnote", StoryKind::Footnote),
            NoteKind::Endnote => (&self.parts.endnotes, "endnote", StoryKind::Endnote),
        };
        let Some(part) = part else { return Ok(vec![]) };
        let t = self.tree(part)?;
        let mut out = vec![];
        for n in t.elements(t.root_element())? {
            if !t.is(n, Some(W), local)? {
                continue;
            }
            let kind_attr = t.attribute(n, Some(W), "type")?;
            if kind_attr.is_some_and(|k| k != "normal") {
                continue;
            }
            out.push(Story {
                kind: story,
                part: part.clone(),
                root: n,
                id: t.attribute(n, Some(W), "id")?.map(|v| v.into_owned()),
            });
        }
        Ok(out)
    }

    fn comment_stories(&self) -> Result<Vec<Story>, Error> {
        let Some(part) = &self.parts.comments else {
            return Ok(vec![]);
        };
        let t = self.tree(part)?;
        let mut out = vec![];
        for n in t.elements(t.root_element())? {
            if t.is(n, Some(W), "comment")? {
                out.push(Story {
                    kind: StoryKind::Comment,
                    part: part.clone(),
                    root: n,
                    id: t.attribute(n, Some(W), "id")?.map(|v| v.into_owned()),
                });
            }
        }
        Ok(out)
    }

    /// The stamp of the document's trees now.
    pub fn stamp(&self) -> Vec<(u64, u64)> {
        self.doc.stamp()
    }

    /// The views read before, if any.
    pub fn cache(&self) -> Option<&'d super::Cache> {
        self.doc.views.as_deref()
    }

    /// The text the views may cut out of the stories, in bytes.
    fn budget(&self) -> u64 {
        self.doc.limits().max_total_size
    }

    fn common(&self) -> Result<Common<'_>, Error> {
        Ok(Common::new(
            self.tree(&self.main)?,
            &self.ids,
            &self.no_rels,
        ))
    }

    /// Walks stories with a sink, and after each story the text boxes it
    /// holds if `boxes` is set. Fields are numbered across all stories.
    fn run<'r, S: Collect<'r>>(
        &'r self,
        stories: &[Story],
        boxes: bool,
        sink: &mut S,
    ) -> Result<(), Error> {
        let mut fields = 0;
        for s in stories {
            let tree = self.tree(&s.part)?;
            let rels = self.rels_of(&s.part);
            sink.common().enter(tree, s.kind, &s.part, None, rels);
            fields = walk::walk(tree, s.root, &self.styles, fields, sink)?;
            if !boxes {
                continue;
            }
            let mut queue: VecDeque<_> = std::mem::take(&mut sink.common().boxes).into();
            while let Some((b, block)) = queue.pop_front() {
                sink.common()
                    .enter(tree, StoryKind::TextBox, &s.part, block, rels);
                fields = walk::walk(tree, b, &self.styles, fields, sink)?;
                queue.extend(std::mem::take(&mut sink.common().boxes));
            }
        }
        Ok(())
    }

    /// The text of one story, without its text boxes.
    fn story_text(&self, s: &Story, view: Revisions) -> Result<String, Error> {
        let mut sink = TextSink::new(self.common()?, view, false);
        self.run(std::slice::from_ref(s), false, &mut sink)?;
        Ok(sink.out.trim_end_matches('\n').to_string())
    }

    /// The text of a text box in the main part.
    fn box_text(&self, b: NodeId) -> Result<String, Error> {
        let story = Story {
            kind: StoryKind::TextBox,
            part: self.main.clone(),
            root: b,
            id: None,
        };
        self.story_text(&story, Revisions::Accepted)
    }

    /// Reads every view in one walk over the stories.
    pub fn read_all(&self) -> Result<Reading, Error> {
        let box_text = |b: NodeId| self.box_text(b);
        let mut all = All {
            blocks: BlocksSink::new(self.common()?, &box_text),
            tables: TablesSink::new(self.common()?),
            fields: FieldsSink::new(self.common()?, self.budget()),
            revisions: RevisionsSink::new(self.common()?),
            marks: MarksSink::new(self.common()?, &self.content_types, self.budget()),
        };
        let mut fields = 0;
        for s in self.stories(false)? {
            let tree = self.tree(&s.part)?;
            let rels = self.rels_of(&s.part);
            all.enter(tree, s.kind, &s.part, None, rels);
            fields = walk::walk(tree, s.root, &self.styles, fields, &mut all)?;
            let mut queue: VecDeque<_> = all.take_boxes().into();
            while let Some((b, block)) = queue.pop_front() {
                all.enter(tree, StoryKind::TextBox, &s.part, block, rels);
                fields = walk::walk(tree, b, &self.styles, fields, &mut all)?;
                queue.extend(all.take_boxes());
            }
            all.marks.close_all()?;
        }
        let All {
            blocks,
            tables,
            fields,
            revisions,
            marks,
        } = all;
        let blocks = self.blocks_from(blocks)?;
        let comments = self.comments(&marks)?;
        let notes = self.notes(&marks)?;
        Ok(Reading {
            blocks,
            styles: self.styles.clone(),
            tables: tables_from(tables),
            fields: fields.fields.into_iter().flatten().collect(),
            revisions: merge_revisions(revisions.done),
            comments,
            notes,
            images: marks.images,
            bookmarks: marks.bookmarks,
            controls: marks.controls,
            sections: self.sections()?,
            numbering: self.numbering()?,
        })
    }

    fn blocks_from(&self, mut sink: BlocksSink<'_>) -> Result<Vec<Block>, Error> {
        let ranges: HashMap<NodeId, (usize, usize)> = sink
            .text
            .blocks
            .iter()
            .map(|&(n, a, b)| (n, (a, b)))
            .collect();
        let t = self.tree(&self.main)?;
        let mut out = Vec::with_capacity(self.blocks.len());
        for (index, (n, id, kind)) in self.blocks.iter().enumerate() {
            let text = ranges
                .get(n)
                .map(|&(a, b)| sink.text.out[a..b].trim_end_matches('\n').to_string())
                .unwrap_or_default();
            let mut block = Block {
                id: id.clone(),
                index,
                kind: *kind,
                style: None,
                style_name: None,
                level: None,
                numbering: None,
                text,
                element: None,
                control: None,
                content: sink.content.remove(n).unwrap_or_default(),
                node: *n,
            };
            match kind {
                BlockKind::Paragraph => {
                    let p = props::paragraph_props(t, *n)?;
                    let style = self.styles.paragraph_style(p.style.as_deref());
                    block.style = p.style.clone().or_else(|| style.map(|s| s.id.clone()));
                    block.style_name = style.map(|s| s.display_name.clone()).or(p.style);
                    block.level = p
                        .outline
                        .filter(|&l| l < 9)
                        .map(|l| l + 1)
                        .or_else(|| style.and_then(|s| s.outline_level));
                    block.numbering = match p.numbering {
                        Some((id, level)) => Some(NumberingRef { id, level }),
                        None => style.and_then(|s| s.numbering.clone()),
                    }
                    .filter(|r| r.id != "0");
                }
                BlockKind::Table => {
                    if let Some(pr) = t.child(*n, Some(W), "tblPr")? {
                        block.style = child_val(t, pr, "tblStyle")?;
                        block.style_name = block
                            .style
                            .as_deref()
                            .and_then(|s| self.styles.get(s))
                            .map(|s| s.display_name.clone());
                    }
                }
                BlockKind::Other | BlockKind::Bookmark => block.element = Some(qualified(t, *n)?),
                BlockKind::ContentControl => {
                    let p = props::control_props(t, *n)?;
                    block.control = Some(ContentControl {
                        kind: p.kind,
                        tag: p.tag,
                        title: p.title,
                        placeholder: p.placeholder,
                    });
                }
                _ => {}
            }
            out.push(block);
        }
        Ok(out)
    }
}

/// Every view of a document, read in one walk.
pub(crate) struct Reading {
    pub blocks: Vec<Block>,
    pub styles: Styles,
    pub tables: Vec<Table>,
    pub fields: Vec<Field>,
    pub revisions: Vec<Revision>,
    pub comments: Vec<Comment>,
    pub notes: Vec<Note>,
    pub images: Vec<Image>,
    pub bookmarks: Vec<Bookmark>,
    pub controls: Vec<Control>,
    pub sections: Vec<Section>,
    pub numbering: Numbering,
}

impl Reading {
    pub fn locate(&self, locator: &Locator) -> Result<Block, Error> {
        let blocks = &self.blocks;
        let found: Vec<&Block> = match locator {
            Locator::Block(id) => blocks.iter().filter(|b| b.id == *id).collect(),
            Locator::Heading(text) => blocks
                .iter()
                .filter(|b| b.level.is_some() && b.text.trim() == text.trim())
                .collect(),
            Locator::Placeholder(name) => {
                let mark = format!("{{{{{name}}}}}");
                blocks
                    .iter()
                    .filter(|b| b.kind == BlockKind::Paragraph && b.text.contains(&mark))
                    .collect()
            }
            Locator::Text(s) => blocks
                .iter()
                .filter(|b| b.text.contains(s.as_str()))
                .collect(),
            Locator::Bookmark(name) => {
                let Some(b) = self.bookmarks.iter().find(|b| b.name == *name) else {
                    return Err(Error::Invalid(format!(
                        "the document has no bookmark {name:?}"
                    )));
                };
                match &b.location.block {
                    Some(id) => {
                        // A bookmark between blocks names the block after it.
                        let i = blocks.iter().position(|x| x.id == *id);
                        let i = i.map(|i| {
                            let mut j = i;
                            while j + 1 < blocks.len() && blocks[j].kind == BlockKind::Bookmark {
                                j += 1;
                            }
                            j
                        });
                        i.map(|i| vec![&blocks[i]]).unwrap_or_default()
                    }
                    None => vec![],
                }
            }
        };
        match found.as_slice() {
            [one] => Ok((*one).clone()),
            [] => Err(Error::Invalid(format!(
                "no block matches {}",
                describe(locator)
            ))),
            many => Err(Error::Invalid(format!(
                "{} blocks match {}: {}",
                many.len(),
                describe(locator),
                many.iter()
                    .take(5)
                    .map(|b| b.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))),
        }
    }
}

/// The sinks of every view, fed from one walk.
struct All<'r> {
    blocks: BlocksSink<'r>,
    tables: TablesSink<'r>,
    fields: FieldsSink<'r>,
    revisions: RevisionsSink<'r>,
    marks: MarksSink<'r>,
}

impl<'r> All<'r> {
    fn enter(
        &mut self,
        tree: &'r Tree,
        story: StoryKind,
        part: &str,
        block: Option<String>,
        rels: &'r HashMap<String, Rel>,
    ) {
        for c in [
            self.blocks.common(),
            self.tables.common(),
            self.fields.common(),
            self.revisions.common(),
            self.marks.common(),
        ] {
            c.enter(tree, story, part, block.clone(), rels);
        }
    }

    /// The text boxes met since the last call. Every sink sees them; one
    /// list is kept.
    fn take_boxes(&mut self) -> Vec<(NodeId, Option<String>)> {
        for c in [
            self.blocks.common(),
            self.tables.common(),
            self.fields.common(),
            self.revisions.common(),
        ] {
            c.boxes.clear();
        }
        std::mem::take(&mut self.marks.common().boxes)
    }
}

impl walk::Sink for All<'_> {
    fn event(&mut self, st: &walk::State, ev: walk::Event<'_>) -> Result<(), Error> {
        // Blocks are the top level of the body only.
        if self.marks.c.story == StoryKind::Main {
            self.blocks.event(st, ev)?;
        }
        self.tables.event(st, ev)?;
        self.fields.event(st, ev)?;
        self.revisions.event(st, ev)?;
        self.marks.event(st, ev)
    }
}

fn tables_from(sink: TablesSink<'_>) -> Vec<Table> {
    let mut out: Vec<Table> = sink.tables.into_iter().flatten().collect();
    // Indexes count skipped tables too; number the ones that remain.
    let map: HashMap<usize, usize> = out.iter().enumerate().map(|(i, t)| (t.index, i)).collect();
    for t in &mut out {
        t.index = map[&t.index];
        if let Some(p) = &mut t.parent {
            p.table = map.get(&p.table).copied().unwrap_or(p.table);
        }
        for c in &mut t.cells {
            c.tables = c
                .tables
                .iter()
                .filter_map(|i| map.get(i).copied())
                .collect();
        }
    }
    out
}

impl<'d> Reader<'d> {
    fn comments(&self, marks: &MarksSink<'_>) -> Result<Vec<Comment>, Error> {
        let replies = self.replies()?;
        let mut out = vec![];
        for s in self.comment_stories()? {
            let t = self.tree(&s.part)?;
            let (author, date) = props::author_date(t, s.root)?;
            let id = s.id.clone().unwrap_or_default();
            let mut para = None;
            for d in t.descendants(s.root)? {
                if t.is(d, Some(W), "p")? {
                    para = props::para_id(t, d)?.or(para);
                }
            }
            let (parent, done) = para
                .as_ref()
                .and_then(|p| replies.get(p).cloned())
                .unwrap_or((None, false));
            out.push((
                para,
                Comment {
                    anchor: marks.anchors.get(&id).cloned(),
                    location: marks.comment_places.get(&id).cloned(),
                    initials: t
                        .attribute(s.root, Some(W), "initials")?
                        .map(|v| v.into_owned()),
                    text: self.story_text(&s, Revisions::Accepted)?,
                    id,
                    author,
                    date,
                    parent: parent.clone(),
                    done,
                },
            ));
        }
        // The parent is named by paragraph id; give the comment id.
        let by_para: HashMap<String, String> = out
            .iter()
            .filter_map(|(p, c)| p.clone().map(|p| (p, c.id.clone())))
            .collect();
        Ok(out
            .into_iter()
            .map(|(_, mut c)| {
                c.parent = c.parent.and_then(|p| by_para.get(&p).cloned());
                c
            })
            .collect())
    }

    /// From `commentsExtended`: a comment's paragraph id to the paragraph
    /// id of the comment it answers, and whether it is done.
    fn replies(&self) -> Result<HashMap<String, (Option<String>, bool)>, Error> {
        let mut out = HashMap::new();
        let Some(part) = &self.parts.comments_extended else {
            return Ok(out);
        };
        let t = self.tree(part)?;
        for n in t.elements(t.root_element())? {
            let Some(name) = t.name(n)? else { continue };
            if name.local != "commentEx" {
                continue;
            }
            let ns = name.namespace;
            let get = |l| -> Result<Option<String>, Error> {
                Ok(t.attribute(n, ns, l)?.map(|v| v.into_owned()))
            };
            if let Some(p) = get("paraId")? {
                let done = get("done")?.is_some_and(|d| d == "1" || d == "true");
                out.insert(p, (get("paraIdParent")?, done));
            }
        }
        Ok(out)
    }

    fn notes(&self, marks: &MarksSink<'_>) -> Result<Vec<Note>, Error> {
        // The first reference to each note, and its position.
        let mut refs = HashMap::new();
        for (i, r) in marks.note_refs.iter().enumerate() {
            refs.entry((r.kind, r.id.clone())).or_insert((i, r));
        }
        let mut out = vec![];
        for kind in [NoteKind::Footnote, NoteKind::Endnote] {
            let mut notes = vec![];
            for (k, s) in self.notes_stories(kind)?.iter().enumerate() {
                let id = s.id.clone().unwrap_or_default();
                let r = refs.get(&(kind, id.clone()));
                // Referenced notes in the order of their references, then
                // the others in the order of the part.
                let order = r.map(|r| r.0).unwrap_or(usize::MAX / 2 + k);
                // The note's text without its own mark, if it has one, and
                // without the space after the mark.
                let text = self.story_text(s, Revisions::Accepted)?;
                let mark = r.and_then(|r| r.1.mark.as_deref());
                let text = match mark {
                    Some(m) => text.trim_start().strip_prefix(m).unwrap_or(&text),
                    None => &text,
                };
                notes.push((
                    order,
                    Note {
                        kind,
                        number: r.and_then(|r| r.1.number),
                        location: r.map(|r| r.1.location.clone()),
                        text: text.trim_start().to_string(),
                        id,
                    },
                ));
            }
            notes.sort_by_key(|n| n.0);
            out.extend(notes.into_iter().map(|n| n.1));
        }
        Ok(out)
    }

    fn numbering(&self) -> Result<Numbering, Error> {
        super::numbering::read(
            self.parts
                .numbering
                .as_deref()
                .and_then(|p| self.doc.parsed(p)),
        )
    }

    fn sections(&self) -> Result<Vec<Section>, Error> {
        let t = self.tree(&self.main)?;
        let mut ends: Vec<(NodeId, Option<String>, Option<String>)> = vec![];
        let mut first: Option<String> = None;
        let mut last: Option<String> = None;
        for (n, id, kind) in &self.blocks {
            match kind {
                BlockKind::Section => {
                    ends.push((*n, first.take(), last.take()));
                }
                _ => {
                    if first.is_none() {
                        first = Some(id.clone());
                    }
                    last = Some(id.clone());
                    if *kind == BlockKind::Paragraph
                        && let Some(ppr) = t.child(*n, Some(W), "pPr")?
                        && let Some(s) = t.child(ppr, Some(W), "sectPr")?
                    {
                        ends.push((s, first.take(), last.take()));
                    }
                }
            }
        }
        let rels = self.rels_of(&self.main);
        let mut out = vec![];
        let mut inherited: HashMap<(bool, String), String> = HashMap::new();
        // Many sections share a header; each part is read once.
        let mut texts = HashMap::new();
        for (index, (s, first_block, last_block)) in ends.into_iter().enumerate() {
            let twips = |el: Option<NodeId>, a: &str| -> Result<Option<f64>, Error> {
                let Some(el) = el else { return Ok(None) };
                Ok(t.attribute(el, Some(W), a)?
                    .and_then(|v| v.parse::<f64>().ok())
                    .map(|v| v / 20.0))
            };
            let size = t.child(s, Some(W), "pgSz")?;
            let mar = t.child(s, Some(W), "pgMar")?;
            let page_width = twips(size, "w")?;
            let margins = Margins {
                top: twips(mar, "top")?.unwrap_or(0.0),
                right: twips(mar, "right")?.unwrap_or(0.0),
                bottom: twips(mar, "bottom")?.unwrap_or(0.0),
                left: twips(mar, "left")?.unwrap_or(0.0),
                header: twips(mar, "header")?.unwrap_or(0.0),
                footer: twips(mar, "footer")?.unwrap_or(0.0),
                gutter: twips(mar, "gutter")?.unwrap_or(0.0),
            };
            let columns = match t.child(s, Some(W), "cols")? {
                Some(c) => t
                    .attribute(c, Some(W), "num")?
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(1),
                None => 1,
            };
            let title_page = match t.child(s, Some(W), "titlePg")? {
                Some(c) => on_off(t, c)?,
                None => false,
            };
            let mut own: HashMap<(bool, String), String> = HashMap::new();
            for r in t.elements(s)? {
                let header = t.is(r, Some(W), "headerReference")?;
                if !header && !t.is(r, Some(W), "footerReference")? {
                    continue;
                }
                let kind = t
                    .attribute(r, Some(W), "type")?
                    .map(|v| v.into_owned())
                    .unwrap_or_else(|| "default".into());
                let part = t
                    .attribute(r, Some(walk::R), "id")?
                    .and_then(|id| rels.get(&*id).and_then(|r| r.part.clone()));
                if let Some(p) = part {
                    own.insert((header, kind), p);
                }
            }
            let mut headers = vec![];
            let mut footers = vec![];
            for header in [true, false] {
                for kind in ["default", "first", "even"] {
                    let key = (header, kind.to_string());
                    let (part, from_before) = match own.get(&key) {
                        Some(p) => (p.clone(), false),
                        None => match inherited.get(&key) {
                            Some(p) => (p.clone(), true),
                            None => continue,
                        },
                    };
                    inherited.insert(key, part.clone());
                    let hf = HeaderFooter {
                        kind: kind.to_string(),
                        text: self.part_text(&part, header, Revisions::Accepted, &mut texts)?,
                        part,
                        inherited: from_before,
                    };
                    if header {
                        headers.push(hf)
                    } else {
                        footers.push(hf)
                    }
                }
            }
            out.push(Section {
                index,
                first_block,
                last_block,
                start: match child_val(t, s, "type")?.as_deref() {
                    Some("continuous") => "continuous",
                    Some("evenPage") => "even_page",
                    Some("oddPage") => "odd_page",
                    Some("nextColumn") => "next_column",
                    _ => "next_page",
                }
                .to_string(),
                page_width,
                page_height: twips(size, "h")?,
                orientation: match size {
                    Some(z)
                        if t.attribute(z, Some(W), "orient")?.as_deref() == Some("landscape") =>
                    {
                        "landscape".into()
                    }
                    _ => "portrait".into(),
                },
                text_width: page_width.map(|w| w - margins.left - margins.right - margins.gutter),
                margins,
                columns,
                title_page,
                headers,
                footers,
            });
        }
        Ok(out)
    }

    /// The text of a header or footer part, read once for each part.
    fn part_text(
        &self,
        part: &str,
        header: bool,
        view: Revisions,
        cache: &mut HashMap<String, String>,
    ) -> Result<String, Error> {
        if let Some(t) = cache.get(part) {
            return Ok(t.clone());
        }
        let story = Story {
            kind: if header {
                StoryKind::Header
            } else {
                StoryKind::Footer
            },
            root: self.tree(part)?.root_element(),
            part: part.to_string(),
            id: None,
        };
        let t = self.story_text(&story, view)?;
        cache.insert(part.to_string(), t.clone());
        Ok(t)
    }

    /// The text with the options asked for. Sections, notes and comments
    /// come from the reading.
    pub fn text(
        &self,
        o: &TextOptions,
        sections: &[Section],
        notes: &[Note],
        comments: &[Comment],
    ) -> Result<String, Error> {
        let mut sink = TextSink::new(self.common()?, o.revisions, o.notes);
        self.run(&self.stories(true)?, false, &mut sink)?;
        let boxes = std::mem::take(&mut sink.c.boxes);
        let sections = if o.headers_footers { sections } else { &[] };
        let mut out = String::new();
        let line = |out: &mut String, s: &str| {
            if !s.is_empty() {
                out.push_str(s.trim_end_matches('\n'));
                out.push('\n');
            }
        };
        let by_block: HashMap<NodeId, (usize, usize)> =
            sink.blocks.iter().map(|&(n, a, b)| (n, (a, b))).collect();
        let mut boxes_of: HashMap<&str, Vec<NodeId>> = HashMap::new();
        for (b, blk) in &boxes {
            if let Some(blk) = blk {
                boxes_of.entry(blk.as_str()).or_default().push(*b);
            }
        }
        // A section's headers come before its first block and its footers
        // after its last, in the view of revisions asked for.
        let mut before: HashMap<&str, Vec<String>> = HashMap::new();
        let mut after: HashMap<&str, Vec<String>> = HashMap::new();
        let mut cache = HashMap::new();
        for s in sections {
            for (header, list, at, map) in [
                (true, &s.headers, &s.first_block, &mut before),
                (false, &s.footers, &s.last_block, &mut after),
            ] {
                let Some(at) = at else { continue };
                // The first page's header and footer, then the others'.
                let kinds: &[&str] = if s.title_page {
                    &["first", "default"]
                } else {
                    &["default"]
                };
                for k in kinds {
                    for h in list.iter().filter(|h| h.kind == *k) {
                        let t = self.part_text(&h.part, header, o.revisions, &mut cache)?;
                        map.entry(at.as_str()).or_default().push(t);
                    }
                }
            }
        }
        for (n, id, _) in &self.blocks {
            for t in before.get(id.as_str()).into_iter().flatten() {
                line(&mut out, t);
            }
            if let Some(&(a, b)) = by_block.get(n) {
                out.push_str(&sink.out[a..b]);
            }
            if o.text_boxes {
                for b in boxes_of.get(id.as_str()).into_iter().flatten() {
                    let story = Story {
                        kind: StoryKind::TextBox,
                        part: self.main.clone(),
                        root: *b,
                        id: None,
                    };

                    let mut s = TextSink::new(self.common()?, o.revisions, o.notes);
                    self.run(std::slice::from_ref(&story), true, &mut s)?;
                    line(&mut out, &s.out);
                }
            }
            for t in after.get(id.as_str()).into_iter().flatten() {
                line(&mut out, t);
            }
        }
        if o.notes {
            let numbered: Vec<&Note> = notes.iter().filter(|n| n.number.is_some()).collect();
            if !numbered.is_empty() {
                out.push('\n');
                for n in numbered {
                    let num = n.number.unwrap_or_default();
                    let mark = match n.kind {
                        NoteKind::Footnote => num.to_string(),
                        NoteKind::Endnote => roman(num),
                    };
                    line(&mut out, &format!("[{mark}] {}", n.text.trim()));
                }
            }
        }
        if o.comments && !comments.is_empty() {
            out.push('\n');
            for c in comments {
                let about = c
                    .anchor
                    .as_deref()
                    .map(|a| format!(" on \u{201C}{}\u{201D}", a.trim()))
                    .unwrap_or_default();
                let verb = if c.parent.is_some() {
                    "Reply"
                } else {
                    "Comment"
                };
                line(
                    &mut out,
                    &format!("[{verb} by {}{about}] {}", c.author, c.text.trim()),
                );
            }
        }
        Ok(out)
    }
}

fn describe(l: &Locator) -> String {
    match l {
        Locator::Block(id) => format!("the block id {id}"),
        Locator::Bookmark(n) => format!("the bookmark {n:?}"),
        Locator::Heading(t) => format!("the heading {t:?}"),
        Locator::Placeholder(n) => format!("the placeholder {{{{{n}}}}}"),
        Locator::Text(t) => format!("the text {t:?}"),
    }
}
