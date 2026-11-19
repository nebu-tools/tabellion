//! Finds the parts and stories of a document, walks them once and puts the
//! views together.

use std::collections::{HashMap, VecDeque};

use crate::xml::{NodeId, Tree};
use crate::{Document, Error};

use super::collect::{Collector, NoteRef, Part, Place, Rel, merge_revisions, qualified};
use super::locate::Place as Wanted;
use super::numbering::Numbering;
use super::props::{self, child_val, on_off, parse_para_id};
use super::styles::Styles;
use super::text::{Pieces, Show, Span, roman};
use super::walk::{self, Shared, W};
use super::{
    Block, BlockId, BlockKind, Bookmark, Comment, ContentControl, Control, Field, HeaderFooter,
    Image, Inline, Locator, Margins, Note, NoteKind, NumberingRef, Paragraph, Revision, Section,
    StoryKind, Table, TextOptions,
};

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
    styles: Styles,
    main: String,
    parts: Parts,
    rels: HashMap<String, HashMap<String, Rel>>,
    content_types: HashMap<String, String>,
    /// The top-level blocks of the body with their ids and kinds, and
    /// whether their paragraph id is shared.
    blocks: Vec<(NodeId, BlockId, BlockKind, bool)>,
    ids: HashMap<NodeId, BlockId>,
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
                BlockKind::Paragraph => props::para_id(t, n)?.map(BlockId::Paragraph),
                BlockKind::Table => {
                    let mut id = None;
                    for d in t.descendants(n)? {
                        if t.is(d, Some(W), "p")? {
                            id = props::para_id(t, d)?.map(BlockId::Table);
                            break;
                        }
                    }
                    id
                }
                _ => None,
            };
            list.push((n, para, kind));
        }
        let number = |id: &BlockId| match id {
            BlockId::Paragraph(n) | BlockId::Table(n) => *n,
            BlockId::Index(_) => u32::MAX,
        };
        let mut seen: HashMap<u32, usize> = HashMap::new();
        for (_, id, _) in &list {
            if let Some(id) = id {
                *seen.entry(number(id)).or_default() += 1;
            }
        }
        for (i, (n, id, kind)) in list.into_iter().enumerate() {
            let (id, shared) = match id {
                Some(id) if seen.get(&number(&id)) == Some(&1) => (id, false),
                Some(_) => (BlockId::Index(i), true),
                None => (BlockId::Index(i), false),
            };
            self.ids.insert(n, id);
            self.blocks.push((n, id, kind, shared));
        }
        Ok(())
    }

    fn stories(&self) -> Result<Vec<Story>, Error> {
        let mut out = vec![];
        if let Some(body) = self.body()? {
            out.push(Story {
                kind: StoryKind::Main,
                part: self.main.clone(),
                root: body,
                id: None,
            });
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
        for (part, local, kind) in [
            (&self.parts.footnotes, "footnote", StoryKind::Footnote),
            (&self.parts.endnotes, "endnote", StoryKind::Endnote),
            (&self.parts.comments, "comment", StoryKind::Comment),
        ] {
            let Some(part) = part else { continue };
            let t = self.tree(part)?;
            for n in t.elements(t.root_element())? {
                if !t.is(n, Some(W), local)? {
                    continue;
                }
                // Separators and continuation notices are not notes.
                let kind_attr = t.attribute(n, Some(W), "type")?;
                if kind != StoryKind::Comment && kind_attr.is_some_and(|k| k != "normal") {
                    continue;
                }
                out.push(Story {
                    kind,
                    part: part.clone(),
                    root: n,
                    id: t.attribute(n, Some(W), "id")?.map(|v| v.into_owned()),
                });
            }
        }
        Ok(out)
    }

    /// Reads every view in one walk over the stories.
    pub fn read(self) -> Result<Views, Error> {
        let mut shared = Shared::default();
        let main_tree = self.tree(&self.main)?;
        let mut c = Collector::new(
            Place {
                tree: main_tree,
                story: StoryKind::Main,
                part: self.main.clone(),
                block: None,
                rels: &self.no_rels,
            },
            &self.ids,
            &self.content_types,
        );
        let mut walked: Vec<(Story, Span, Option<u32>)> = vec![];
        let mut box_spans: HashMap<NodeId, Span> = HashMap::new();
        let mut main_boxes: Vec<(Option<BlockId>, Span)> = vec![];
        for s in self.stories()? {
            let tree = self.tree(&s.part)?;
            let rels = self.rels_of(&s.part);
            let numbered = s.kind == StoryKind::Main;
            c.enter(tree, s.kind, &s.part, None, rels);
            let start = shared.pieces.len();
            walk::walk(tree, s.root, &self.styles, numbered, &mut shared, &mut c)?;
            c.close_all(shared.pieces.len());
            let para_id = c.last_para_id;
            walked.push((s.clone(), shared.pieces.span_from(start, 0), para_id));
            // Text boxes after the story that holds them, those inside text
            // boxes after those.
            let mut queue: VecDeque<_> = std::mem::take(&mut c.boxes).into();
            while let Some((b, block)) = queue.pop_front() {
                c.enter(tree, StoryKind::TextBox, &s.part, block, rels);
                let start = shared.pieces.len();
                // Word drops a note reference in a text box when it opens
                // the file, so such a reference takes no number.
                walk::walk(tree, b, &self.styles, false, &mut shared, &mut c)?;
                c.close_all(shared.pieces.len());
                let span = shared.pieces.span_from(start, 0);
                box_spans.insert(b, span);
                if s.kind == StoryKind::Main {
                    main_boxes.push((block, span));
                }
                queue.extend(std::mem::take(&mut c.boxes));
            }
        }
        let pieces = shared.pieces;
        let mut cut = Cut::new(&pieces, self.doc.limits().max_total_size);
        let (tables, table_map) = tables_from(std::mem::take(&mut c.tables), &pieces);
        let paragraphs = self.paragraphs_from(&mut c, &pieces, &box_spans, &table_map)?;
        let blocks = self.blocks_from(&c, &pieces, &paragraphs)?;
        let fields = std::mem::take(&mut c.fields)
            .into_iter()
            .flatten()
            .map(|(mut f, r)| {
                if let Some(r) = r {
                    f.result = cut.text(r)?;
                }
                Ok(f)
            })
            .collect::<Result<Vec<Field>, Error>>()?;
        let revisions = merge_revisions(std::mem::take(&mut c.revisions))
            .into_iter()
            .map(|(mut r, parts)| {
                for p in parts {
                    match p {
                        Part::Span(s) => r.text.push_str(&cut.render(s, Show::ALL)?),
                        Part::Mark => r.text.push('\n'),
                    }
                }
                Ok(r)
            })
            .collect::<Result<Vec<Revision>, Error>>()?;
        let bookmarks = std::mem::take(&mut c.bookmarks)
            .into_iter()
            .map(|(mut b, s)| {
                if let Some(s) = s {
                    b.text = cut.text(s)?;
                }
                Ok(b)
            })
            .collect::<Result<Vec<Bookmark>, Error>>()?;
        let controls = std::mem::take(&mut c.controls)
            .into_iter()
            .map(|(mut k, s)| {
                if let Some(s) = s {
                    k.text = cut.text(s)?;
                }
                Ok(k)
            })
            .collect::<Result<Vec<Control>, Error>>()?;
        let mut anchors = HashMap::new();
        for (id, s) in std::mem::take(&mut c.anchors) {
            anchors.insert(id, (s, cut.text(s)?));
        }
        let (comments, comment_spans) = self.comments(&walked, &c, &anchors, &pieces)?;
        let (notes, note_spans) = self.notes(&walked, &c.note_refs, &pieces);
        let headers: HashMap<String, Span> = walked
            .iter()
            .filter(|(s, _, _)| matches!(s.kind, StoryKind::Header | StoryKind::Footer))
            .map(|(s, span, _)| (s.part.to_ascii_lowercase(), *span))
            .collect();
        let sections = self.sections(&headers, &pieces)?;
        let block_spans = self
            .blocks
            .iter()
            .map(|(n, ..)| c.block_spans.get(n).copied())
            .collect();
        Ok(Views {
            blocks,
            paragraphs,
            styles: self.styles.clone(),
            tables,
            fields,
            revisions,
            comments,
            notes,
            images: std::mem::take(&mut c.images),
            bookmarks,
            controls,
            sections,
            numbering: self.numbering()?,
            text: Text {
                block_spans,
                main_boxes,
                headers,
                notes: note_spans,
                comments: comment_spans,
            },
            pieces,
        })
    }

    /// Every paragraph of every story with its content; the text of the
    /// text boxes a drawing holds goes into the drawing's item.
    fn paragraphs_from(
        &self,
        c: &mut Collector<'_>,
        pieces: &Pieces,
        box_spans: &HashMap<NodeId, Span>,
        table_map: &HashMap<usize, usize>,
    ) -> Result<Vec<Paragraph>, Error> {
        let raw = std::mem::take(&mut c.paragraphs);
        let mut out = Vec::with_capacity(raw.len());
        for (index, p) in raw.into_iter().enumerate() {
            let mut content = p.content;
            for (i, boxes) in std::mem::take(&mut content.boxes) {
                let mut all = String::new();
                for b in boxes {
                    if let Some(&s) = box_spans.get(&b) {
                        if !all.is_empty() {
                            all.push('\n');
                        }
                        all.push_str(&pieces.item_text(s, Show::VIEWS));
                    }
                }
                if let Some(Inline::Drawing { text, .. }) = content.items.get_mut(i) {
                    *text = Some(all);
                }
            }
            let t = self.tree(&p.location.part)?;
            let props = props::paragraph_props(t, p.node)?;
            let style = self.styles.paragraph_style(props.style.as_deref());
            out.push(Paragraph {
                index,
                style: props.style.clone().or_else(|| style.map(|s| s.id.clone())),
                style_name: style.map(|s| s.display_name.clone()).or(props.style),
                text: pieces.item_text(p.span, Show::VIEWS),
                cell: p.cell.map(|mut r| {
                    r.table = table_map.get(&r.table).copied().unwrap_or(r.table);
                    r
                }),
                location: p.location,
                content: content.items,
                node: p.node,
            });
        }
        Ok(out)
    }

    fn blocks_from(
        &self,
        c: &Collector<'_>,
        pieces: &Pieces,
        paragraphs: &[Paragraph],
    ) -> Result<Vec<Block>, Error> {
        let t = self.tree(&self.main)?;
        let by_node: HashMap<NodeId, usize> = paragraphs
            .iter()
            .enumerate()
            .filter(|(_, p)| p.location.story == StoryKind::Main && p.cell.is_none())
            .map(|(i, p)| (p.node, i))
            .collect();
        let mut out = Vec::with_capacity(self.blocks.len());
        for (index, &(n, id, kind, shared_para_id)) in self.blocks.iter().enumerate() {
            let text = c
                .block_spans
                .get(&n)
                .map(|&s| pieces.item_text(s, Show::VIEWS))
                .unwrap_or_default();
            let content = by_node
                .get(&n)
                .map(|&i| paragraphs[i].content.clone())
                .unwrap_or_default();
            let mut block = Block {
                id,
                shared_para_id,
                index,
                kind,
                style: None,
                style_name: None,
                level: None,
                numbering: None,
                text,
                element: None,
                control: None,
                content,
                node: n,
            };
            match kind {
                BlockKind::Paragraph => {
                    let p = props::paragraph_props(t, n)?;
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
                    if let Some(pr) = t.child(n, Some(W), "tblPr")? {
                        block.style = child_val(t, pr, "tblStyle")?;
                        block.style_name = block
                            .style
                            .as_deref()
                            .and_then(|s| self.styles.get(s))
                            .map(|s| s.display_name.clone());
                    }
                }
                BlockKind::Other | BlockKind::Bookmark => block.element = Some(qualified(t, n)?),
                BlockKind::ContentControl => {
                    let p = props::control_props(t, n)?;
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

    #[allow(clippy::type_complexity)]
    fn comments(
        &self,
        walked: &[(Story, Span, Option<u32>)],
        c: &Collector<'_>,
        anchors: &HashMap<String, (Span, String)>,
        pieces: &Pieces,
    ) -> Result<(Vec<Comment>, Vec<(Span, Option<Span>)>), Error> {
        let replies = self.replies()?;
        let mut out = vec![];
        for (s, span, para) in walked.iter().filter(|w| w.0.kind == StoryKind::Comment) {
            let t = self.tree(&s.part)?;
            let (author, date) = props::author_date(t, s.root)?;
            let id = s.id.clone().unwrap_or_default();
            let (parent, done) = para
                .and_then(|p| replies.get(&p).copied())
                .unwrap_or((None, false));
            let anchor = anchors.get(&id);
            out.push((
                *para,
                parent,
                Comment {
                    anchor: anchor.map(|a| a.1.clone()),
                    location: c.comment_places.get(&id).cloned(),
                    initials: t
                        .attribute(s.root, Some(W), "initials")?
                        .map(|v| v.into_owned()),
                    text: pieces.item_text(*span, Show::VIEWS),
                    id,
                    author,
                    date,
                    parent: None,
                    done,
                },
                (*span, anchor.map(|a| a.0)),
            ));
        }
        // The parent is named by paragraph id; give the comment id.
        let by_para: HashMap<u32, String> = out
            .iter()
            .filter_map(|(p, _, c, _)| p.map(|p| (p, c.id.clone())))
            .collect();
        let mut comments = vec![];
        let mut spans = vec![];
        for (_, parent, mut c, s) in out {
            c.parent = parent.and_then(|p| by_para.get(&p).cloned());
            comments.push(c);
            spans.push(s);
        }
        Ok((comments, spans))
    }

    /// From `commentsExtended`: a comment's paragraph id to the paragraph
    /// id of the comment it answers, and whether it is done.
    fn replies(&self) -> Result<HashMap<u32, (Option<u32>, bool)>, Error> {
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
            if let Some(p) = get("paraId")?.and_then(|p| parse_para_id(&p)) {
                let done = get("done")?.is_some_and(|d| d == "1" || d == "true");
                let parent = get("paraIdParent")?.and_then(|p| parse_para_id(&p));
                out.insert(p, (parent, done));
            }
        }
        Ok(out)
    }

    fn notes(
        &self,
        walked: &[(Story, Span, Option<u32>)],
        refs: &[NoteRef],
        pieces: &Pieces,
    ) -> (Vec<Note>, Vec<(Span, Option<String>)>) {
        // The first reference to each note, and its position.
        let mut first = HashMap::new();
        for (i, r) in refs.iter().enumerate() {
            first.entry((r.kind, r.id.as_str())).or_insert((i, r));
        }
        let mut out = vec![];
        for kind in [NoteKind::Footnote, NoteKind::Endnote] {
            let story = match kind {
                NoteKind::Footnote => StoryKind::Footnote,
                NoteKind::Endnote => StoryKind::Endnote,
            };
            let mut notes = vec![];
            for (k, (s, span, _)) in walked.iter().filter(|w| w.0.kind == story).enumerate() {
                let id = s.id.clone().unwrap_or_default();
                let r = first.get(&(kind, id.as_str()));
                // Referenced notes in the order of their references, then
                // the others in the order of the part.
                let order = r.map(|r| r.0).unwrap_or(usize::MAX / 2 + k);
                let mark = r.and_then(|r| r.1.mark.clone());
                let text = note_text(pieces, *span, mark.as_deref(), Show::VIEWS);
                notes.push((
                    order,
                    Note {
                        kind,
                        number: r.and_then(|r| r.1.number),
                        location: r.map(|r| r.1.location.clone()),
                        text,
                        id,
                    },
                    (*span, mark),
                ));
            }
            notes.sort_by_key(|n| n.0);
            out.extend(notes.into_iter().map(|n| (n.1, n.2)));
        }
        out.into_iter().unzip()
    }

    fn numbering(&self) -> Result<Numbering, Error> {
        super::numbering::read(
            self.parts
                .numbering
                .as_deref()
                .and_then(|p| self.doc.parsed(p)),
        )
    }

    fn sections(
        &self,
        headers: &HashMap<String, Span>,
        pieces: &Pieces,
    ) -> Result<Vec<Section>, Error> {
        let t = self.tree(&self.main)?;
        let mut ends: Vec<(NodeId, Option<BlockId>, Option<BlockId>)> = vec![];
        let mut first: Option<BlockId> = None;
        let mut last: Option<BlockId> = None;
        for &(n, id, kind, _) in &self.blocks {
            match kind {
                BlockKind::Section => {
                    ends.push((n, first.take(), last.take()));
                }
                _ => {
                    if first.is_none() {
                        first = Some(id);
                    }
                    last = Some(id);
                    if kind == BlockKind::Paragraph
                        && let Some(ppr) = t.child(n, Some(W), "pPr")?
                        && let Some(s) = t.child(ppr, Some(W), "sectPr")?
                    {
                        ends.push((s, first.take(), last.take()));
                    }
                }
            }
        }
        let rels = self.rels_of(&self.main);
        let mut out = vec![];
        let mut inherited: HashMap<(bool, &str), String> = HashMap::new();
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
            let mut headers_of = vec![];
            let mut footers_of = vec![];
            for header in [true, false] {
                for kind in ["default", "first", "even"] {
                    let (part, from_before) = match own.get(&(header, kind.to_string())) {
                        Some(p) => (p.clone(), false),
                        None => match inherited.get(&(header, kind)) {
                            Some(p) => (p.clone(), true),
                            None => continue,
                        },
                    };
                    inherited.insert((header, kind), part.clone());
                    let text = headers
                        .get(&part.to_ascii_lowercase())
                        .map(|&s| pieces.item_text(s, Show::VIEWS))
                        .unwrap_or_default();
                    let hf = HeaderFooter {
                        kind: kind.to_string(),
                        text,
                        part,
                        inherited: from_before,
                    };
                    if header {
                        headers_of.push(hf)
                    } else {
                        footers_of.push(hf)
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
                headers: headers_of,
                footers: footers_of,
            });
        }
        Ok(out)
    }
}

/// Text of ranges that may overlap, such as bookmarks and field results:
/// together they may hold no more than [`Limits::max_total_size`] bytes.
///
/// [`Limits::max_total_size`]: crate::Limits
struct Cut<'p> {
    pieces: &'p Pieces,
    budget: u64,
    used: u64,
}

impl<'p> Cut<'p> {
    fn new(pieces: &'p Pieces, budget: u64) -> Cut<'p> {
        Cut {
            pieces,
            budget,
            used: 0,
        }
    }

    fn render(&mut self, span: Span, show: Show) -> Result<String, Error> {
        let s = self.pieces.render(span, show);
        self.used = self.used.saturating_add(s.len() as u64);
        if self.used > self.budget {
            return Err(Error::Limit {
                what: format!(
                    "ranges such as bookmarks or fields that hold more than {} bytes of text together",
                    self.budget
                ),
                field: "max_total_size",
                limit: self.budget,
            });
        }
        Ok(s)
    }

    fn text(&mut self, span: Span) -> Result<String, Error> {
        let mut s = self.render(span, Show::VIEWS)?;
        let keep = s.trim_end_matches('\n').len();
        s.truncate(keep);
        Ok(s)
    }
}

fn tables_from(
    raw: Vec<Option<super::collect::TableSpans>>,
    pieces: &Pieces,
) -> (Vec<Table>, HashMap<usize, usize>) {
    let mut out: Vec<Table> = raw
        .into_iter()
        .flatten()
        .map(|t| {
            let mut table = t.table;
            for (cell, spans) in table.cells.iter_mut().zip(t.cells) {
                let texts: Vec<String> = spans
                    .into_iter()
                    .map(|s| pieces.item_text(s, Show::VIEWS))
                    .filter(|s| !s.is_empty())
                    .collect();
                cell.text = texts.join("\n");
            }
            let mut rows = vec![vec![Some(String::new()); table.column_count]; table.row_count];
            for c in &table.cells {
                for (r, row) in rows.iter_mut().enumerate().skip(c.row).take(c.row_span) {
                    for (k, slot) in row
                        .iter_mut()
                        .enumerate()
                        .skip(c.column)
                        .take(c.column_span)
                    {
                        *slot = (r == c.row && k == c.column).then(|| c.text.clone());
                    }
                }
            }
            table.rows = rows;
            table
        })
        .collect();
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
    (out, map)
}

/// A note's text without its own mark, if it has one, and without the
/// space after the mark.
fn note_text(pieces: &Pieces, span: Span, mark: Option<&str>, show: Show) -> String {
    let text = pieces.item_text(span, show);
    let text = match mark {
        Some(m) => text.trim_start().strip_prefix(m).unwrap_or(&text),
        None => &text,
    };
    text.trim_start().to_string()
}

/// What [`Views::text`] needs besides the views: the spans of the blocks,
/// text boxes, headers, notes and comments.
struct Text {
    block_spans: Vec<Option<Span>>,
    main_boxes: Vec<(Option<BlockId>, Span)>,
    headers: HashMap<String, Span>,
    notes: Vec<(Span, Option<String>)>,
    comments: Vec<(Span, Option<Span>)>,
}

/// Every view of a document, read in one walk over its stories.
///
/// The views hold what the walk found; each method returns it borrowed,
/// at no cost. [`Views::text`] renders the text anew for its options, at a
/// cost in proportion to the text of the stories it includes.
pub struct Views {
    blocks: Vec<Block>,
    paragraphs: Vec<Paragraph>,
    styles: Styles,
    tables: Vec<Table>,
    fields: Vec<Field>,
    revisions: Vec<Revision>,
    comments: Vec<Comment>,
    notes: Vec<Note>,
    images: Vec<Image>,
    bookmarks: Vec<Bookmark>,
    controls: Vec<Control>,
    sections: Vec<Section>,
    numbering: Numbering,
    text: Text,
    pieces: Pieces,
}

impl std::fmt::Debug for Views {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Views")
            .field("blocks", &self.blocks.len())
            .field("tables", &self.tables.len())
            .finish_non_exhaustive()
    }
}

impl Views {
    /// The top-level blocks of the body with ids, kinds, styles, levels,
    /// text and the content of paragraphs. No cost.
    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    /// Every paragraph of every story, with its location, the table cell
    /// that holds it, its style, text and content: the body and its
    /// tables and content controls, headers and footers, notes, comments,
    /// and text boxes after the story that holds them. No cost.
    pub fn paragraphs(&self) -> &[Paragraph] {
        &self.paragraphs
    }

    /// The styles. No cost.
    pub fn styles(&self) -> &Styles {
        &self.styles
    }

    /// All tables of all stories, nested ones included, in reading order.
    /// No cost.
    pub fn tables(&self) -> &[Table] {
        &self.tables
    }

    /// All fields of all stories, nested ones after the field that holds
    /// them. No cost.
    pub fn fields(&self) -> &[Field] {
        &self.fields
    }

    /// All tracked changes of all stories. No cost.
    pub fn revisions(&self) -> &[Revision] {
        &self.revisions
    }

    /// The comments, in the order of the comments part. No cost.
    pub fn comments(&self) -> &[Comment] {
        &self.comments
    }

    /// The pictures of all stories. No cost.
    pub fn images(&self) -> &[Image] {
        &self.images
    }

    /// The bookmarks of all stories. No cost.
    pub fn bookmarks(&self) -> &[Bookmark] {
        &self.bookmarks
    }

    /// The content controls of all stories, an enclosing control before
    /// those it holds. No cost.
    pub fn content_controls(&self) -> &[Control] {
        &self.controls
    }

    /// The footnotes, then the endnotes, without separators, each in the
    /// order of their references in the body. No cost.
    pub fn notes(&self) -> &[Note] {
        &self.notes
    }

    /// The sections with their page setup, headers and footers. No cost.
    pub fn sections(&self) -> &[Section] {
        &self.sections
    }

    /// The numbering definitions and instances. No cost.
    pub fn numbering(&self) -> &Numbering {
        &self.numbering
    }

    /// The one block a locator names. A locator that matches no block or
    /// several gives [`Error::Invalid`], which names the candidates' ids.
    /// Costs one pass over the blocks.
    pub fn locate(&self, locator: &Locator) -> Result<&Block, Error> {
        let blocks = &self.blocks;
        let mut found: Vec<usize> = match &locator.0 {
            Wanted::Block(id) => (0..blocks.len()).filter(|&i| blocks[i].id == *id).collect(),
            Wanted::Heading(text) => (0..blocks.len())
                .filter(|&i| blocks[i].level.is_some() && blocks[i].text.trim() == text)
                .collect(),
            Wanted::Placeholder(name) => {
                let mark = format!("{{{{{name}}}}}");
                (0..blocks.len())
                    .filter(|&i| {
                        blocks[i].kind == BlockKind::Paragraph && blocks[i].text.contains(&mark)
                    })
                    .collect()
            }
            Wanted::Text(s) => (0..blocks.len())
                .filter(|&i| blocks[i].text.contains(s.as_str()))
                .collect(),
            Wanted::Bookmark(name) => {
                let index: HashMap<BlockId, usize> =
                    blocks.iter().enumerate().map(|(i, b)| (b.id, i)).collect();
                self.bookmarks
                    .iter()
                    .filter(|b| b.name.eq_ignore_ascii_case(name))
                    .filter_map(|b| b.location.block.and_then(|id| index.get(&id).copied()))
                    .map(|mut i| {
                        // A bookmark between blocks names the block after it.
                        while i + 1 < blocks.len() && blocks[i].kind == BlockKind::Bookmark {
                            i += 1;
                        }
                        i
                    })
                    .collect()
            }
        };
        found.sort_unstable();
        found.dedup();
        match found.as_slice() {
            [one] => Ok(&blocks[*one]),
            [] => Err(Error::Invalid(format!("no block matches {locator}"))),
            many => {
                let ids: Vec<String> = many
                    .iter()
                    .take(10)
                    .map(|&i| blocks[i].id.to_string())
                    .collect();
                let more = if many.len() > 10 { ", …" } else { "" };
                Err(Error::Invalid(format!(
                    "{} blocks match {locator}: {}{more}",
                    many.len(),
                    ids.join(", ")
                )))
            }
        }
    }

    /// The text as a reader sees it: paragraphs as lines, tables as lines
    /// of tab-separated cells, with the options asked for. Costs one pass
    /// over the text of the stories it includes.
    pub fn text(&self, o: &TextOptions) -> String {
        let show = Show {
            revisions: o.revisions,
            note_marks: o.notes,
        };
        let pieces = &self.pieces;
        let t = &self.text;
        let mut out = String::new();
        let line = |out: &mut String, s: &str| {
            if !s.is_empty() {
                out.push_str(s.trim_end_matches('\n'));
                out.push('\n');
            }
        };
        let mut boxes_of: HashMap<BlockId, Vec<Span>> = HashMap::new();
        if o.text_boxes {
            for (b, s) in &t.main_boxes {
                if let Some(b) = b {
                    boxes_of.entry(*b).or_default().push(*s);
                }
            }
        }
        // A section's headers come before its first block and its footers
        // after its last: the first page's, then the others'.
        let mut before: HashMap<BlockId, Vec<String>> = HashMap::new();
        let mut after: HashMap<BlockId, Vec<String>> = HashMap::new();
        if o.headers_footers {
            for s in &self.sections {
                for (list, at, map) in [
                    (&s.headers, &s.first_block, &mut before),
                    (&s.footers, &s.last_block, &mut after),
                ] {
                    let Some(at) = at else { continue };
                    let kinds: &[&str] = if s.title_page {
                        &["first", "default"]
                    } else {
                        &["default"]
                    };
                    for k in kinds {
                        for h in list.iter().filter(|h| h.kind == *k) {
                            if let Some(&span) = t.headers.get(&h.part.to_ascii_lowercase()) {
                                map.entry(*at)
                                    .or_default()
                                    .push(pieces.item_text(span, show));
                            }
                        }
                    }
                }
            }
        }
        for (b, span) in self.blocks.iter().zip(&t.block_spans) {
            for s in before.get(&b.id).into_iter().flatten() {
                line(&mut out, s);
            }
            if let Some(span) = span {
                out.push_str(&pieces.render(*span, show));
            }
            for s in boxes_of.get(&b.id).into_iter().flatten() {
                line(&mut out, &pieces.render(*s, show));
            }
            for s in after.get(&b.id).into_iter().flatten() {
                line(&mut out, s);
            }
        }
        if o.notes {
            let numbered: Vec<(&Note, &(Span, Option<String>))> = self
                .notes
                .iter()
                .zip(&t.notes)
                .filter(|(n, _)| n.number.is_some())
                .collect();
            if !numbered.is_empty() {
                out.push('\n');
                for (n, (span, mark)) in numbered {
                    let num = n.number.unwrap_or_default();
                    let label = match n.kind {
                        NoteKind::Footnote => num.to_string(),
                        NoteKind::Endnote => roman(num),
                    };
                    let text = note_text(pieces, *span, mark.as_deref(), show);
                    line(&mut out, &format!("[{label}] {}", text.trim()));
                }
            }
        }
        if o.comments && !self.comments.is_empty() {
            out.push('\n');
            for (c, (span, anchor)) in self.comments.iter().zip(&t.comments) {
                let about = anchor
                    .map(|a| pieces.item_text(a, show))
                    .map(|a| format!(" on \u{201C}{}\u{201D}", a.trim()))
                    .unwrap_or_default();
                let verb = if c.parent.is_some() {
                    "Reply"
                } else {
                    "Comment"
                };
                let text = pieces.item_text(*span, show);
                line(
                    &mut out,
                    &format!("[{verb} by {}{about}] {}", c.author, text.trim()),
                );
            }
        }
        out
    }
}
