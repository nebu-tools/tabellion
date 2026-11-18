//! What the views collect from the events of the walk.
//!
//! [`Collector`] is the one sink of the walk. It keeps the place (story,
//! part, block) and hands each event to the collector of each view. The
//! collectors keep no text: where a view gives text, they keep a
//! [`Span`] of the pieces the walk records, and the reader renders it
//! when the walk is done.

use std::collections::{HashMap, HashSet};

use crate::Error;
use crate::xml::{NodeId, Tree};

use super::props::{self, DrawingProps, child_val};
use super::text::Span;
use super::walk::{self, Event, Kind, R, Sink, State, W};
use super::{
    BlockId, Bookmark, Cell, CellRef, Change, ContentControl, Control, DrawingKind, Field, Format,
    Image, Inline, Location, NoteKind, Revision, RevisionKind, RevisionTarget, Revisions,
    StoryKind, Table,
};

/// A relationship's target as the views need it.
#[derive(Debug, Clone)]
pub(crate) struct Rel {
    pub target: String,
    pub external: bool,
    pub part: Option<String>,
}

/// Where the walk is.
pub(crate) struct Place<'r> {
    pub tree: &'r Tree,
    pub story: StoryKind,
    pub part: String,
    pub block: Option<BlockId>,
    /// Relationships of the story's part, by id.
    pub rels: &'r HashMap<String, Rel>,
}

impl Place<'_> {
    fn location(&self) -> Location {
        Location {
            story: self.story,
            part: self.part.clone(),
            block: self.block,
        }
    }
}

/// The content of a top-level paragraph of the body, with the drawings
/// whose text boxes give their text once those are read.
#[derive(Debug, Default)]
pub(crate) struct Content {
    pub items: Vec<Inline>,
    /// Items that are drawings, with their text boxes.
    pub boxes: Vec<(usize, Vec<NodeId>)>,
}

/// A table with the spans of its cells' text.
pub(crate) struct TableSpans {
    pub table: Table,
    /// For each cell, the spans of the cells it was merged from.
    pub cells: Vec<Vec<Span>>,
}

/// A part of the text of a revision.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Part {
    Span(Span),
    /// A paragraph mark, which Word gives as one line end.
    Mark,
}

/// A revision as collected: its order, the revision, the parts of its
/// text, and the count of visible text before and after it, which tells
/// whether two revisions have content between them.
pub(crate) struct RevisionSpans {
    pub seq: usize,
    pub revision: Revision,
    pub parts: Vec<Part>,
    pub start: usize,
    pub end: usize,
}

/// A reference to a note.
pub(crate) struct NoteRef {
    pub kind: NoteKind,
    pub id: String,
    pub number: Option<usize>,
    pub location: Location,
    /// The mark of a note that has its own.
    pub mark: Option<String>,
}

/// Everything the walk gives the views.
pub(crate) struct Collector<'r> {
    pub at: Place<'r>,
    /// Block ids of the main story.
    ids: &'r HashMap<NodeId, BlockId>,
    content_types: &'r HashMap<String, String>,
    /// Text boxes met in the current story, with the block that holds
    /// them.
    pub boxes: Vec<(NodeId, Option<BlockId>)>,
    /// The paragraph id of the last paragraph of the current story, for a
    /// comment.
    pub last_para_id: Option<u32>,

    // Blocks.
    pub block_spans: HashMap<NodeId, Span>,
    block_start: Option<(NodeId, usize)>,
    pub content: HashMap<NodeId, Content>,
    current: Option<(NodeId, Content)>,
    last_run: Option<NodeId>,
    run_format: Format,
    bookmark_names: HashMap<String, String>,
    started: HashSet<usize>,

    // Tables.
    stack: Vec<TableBuild>,
    containers: Vec<(Kind, NodeId)>,
    pub tables: Vec<Option<TableSpans>>,

    // Fields.
    pub fields: Vec<Option<(Field, Option<Span>)>>,
    results: HashMap<usize, (usize, u16, u16)>,

    // Revisions.
    open: Vec<OpenRevision>,
    pub revisions: Vec<RevisionSpans>,
    seq: usize,
    marks: usize,
    /// Open revision records of rows and cells: kind and author.
    row_records: Vec<(RevisionKind, String)>,
    /// Open records of table property changes: their authors.
    table_records: Vec<String>,
    /// The element whose start the walk reported last.
    last_start: Option<NodeId>,
    /// The current row has a property change, and where it began.
    row_changed: bool,
    row_start: usize,

    // Bookmarks, comment anchors, content controls, references, pictures.
    pub bookmarks: Vec<(Bookmark, Option<Span>)>,
    open_bookmarks: HashMap<String, (usize, usize, u16)>,
    pub anchors: HashMap<String, Span>,
    open_anchors: HashMap<String, (usize, u16)>,
    pub controls: Vec<(Control, Option<Span>)>,
    open_controls: Vec<(usize, usize, u16)>,
    pub comment_places: HashMap<String, Location>,
    pub note_refs: Vec<NoteRef>,
    /// The run of a note reference with its own mark, which follows it.
    mark_run: Option<NodeId>,
    pub images: Vec<Image>,
    pictures: usize,
}

struct TableBuild {
    index: usize,
    table: Table,
    spans: Vec<Vec<Span>>,
    skip: bool,
    row: usize,
    row_hidden: bool,
    column: usize,
    width: usize,
    /// The cell that holds each grid column while a vertical merge goes on.
    vertical: HashMap<usize, usize>,
    current: Option<usize>,
    cell_start: (usize, u16),
}

struct OpenRevision {
    seq: usize,
    until: NodeId,
    start: usize,
    depth: u16,
    record: Option<Revision>,
    /// A change of table properties covers the rows whose properties
    /// changed with it, as Word reports it, and the whole table if no row
    /// shares it.
    table: bool,
    parts: Vec<Part>,
    /// The kind and author it was counted under in `row_records`.
    row_record: bool,
    mark_start: usize,
}

impl<'r> Collector<'r> {
    pub fn new(
        at: Place<'r>,
        ids: &'r HashMap<NodeId, BlockId>,
        content_types: &'r HashMap<String, String>,
    ) -> Collector<'r> {
        Collector {
            at,
            ids,
            content_types,
            boxes: vec![],
            last_para_id: None,
            block_spans: HashMap::new(),
            block_start: None,
            content: HashMap::new(),
            current: None,
            last_run: None,
            run_format: Format::default(),
            bookmark_names: HashMap::new(),
            started: HashSet::new(),
            stack: vec![],
            containers: vec![],
            tables: vec![],
            fields: vec![],
            results: HashMap::new(),
            open: vec![],
            revisions: vec![],
            seq: 0,
            marks: 0,
            row_records: vec![],
            table_records: vec![],
            last_start: None,
            row_changed: false,
            row_start: 0,
            bookmarks: vec![],
            open_bookmarks: HashMap::new(),
            anchors: HashMap::new(),
            open_anchors: HashMap::new(),
            controls: vec![],
            open_controls: vec![],
            comment_places: HashMap::new(),
            note_refs: vec![],
            mark_run: None,
            images: vec![],
            pictures: 0,
        }
    }

    /// Starts a story.
    pub fn enter(
        &mut self,
        tree: &'r Tree,
        story: StoryKind,
        part: &str,
        block: Option<BlockId>,
        rels: &'r HashMap<String, Rel>,
    ) {
        self.at = Place {
            tree,
            story,
            part: part.to_string(),
            block,
            rels,
        };
        self.last_para_id = None;
    }

    /// Ends the bookmarks, comment ranges and content controls still open
    /// at the end of a story, at `end`.
    pub fn close_all(&mut self, end: usize) {
        for (_, (i, start, depth)) in std::mem::take(&mut self.open_bookmarks) {
            self.bookmarks[i].1 = Some(span(start, end, depth, 0));
        }
        for (id, (start, depth)) in std::mem::take(&mut self.open_anchors) {
            self.anchors.insert(id, span(start, end, depth, 0));
        }
        while let Some((i, start, depth)) = self.open_controls.pop() {
            self.controls[i].1 = Some(span(start, end, depth, 0));
        }
        self.mark_run = None;
    }
}

fn span(start: usize, end: usize, depth: u16, code: u16) -> Span {
    Span {
        start,
        end,
        depth,
        code,
        own_level: false,
    }
}

fn change_info(tree: &Tree, st: &State<'_>) -> Result<Option<Change>, Error> {
    let Some(c) = st.change() else {
        return Ok(None);
    };
    let (author, date) = props::author_date(tree, c.node)?;
    Ok(Some(Change {
        kind: c.kind,
        author,
        date,
    }))
}

/// True if a paragraph mark, row or cell revision leaves its content in
/// the document under this view.
fn mark_shown(tree: &Tree, mark: NodeId, view: Revisions) -> Result<bool, Error> {
    let kind = walk::change_kind(tree, mark)?;
    Ok(match (view, kind) {
        (Revisions::Current, _) | (_, None) => true,
        (Revisions::Accepted, Some(k)) => {
            !matches!(k, RevisionKind::Deletion | RevisionKind::MoveFrom)
        }
        (Revisions::Rejected, Some(k)) => {
            !matches!(k, RevisionKind::Insertion | RevisionKind::MoveTo)
        }
    })
}

fn int_val(tree: &Tree, parent: Option<NodeId>, local: &str) -> Result<usize, Error> {
    let Some(p) = parent else { return Ok(0) };
    Ok(child_val(tree, p, local)?
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0))
}

/// An element's name as written.
pub(crate) fn qualified(tree: &Tree, n: NodeId) -> Result<String, Error> {
    Ok(match tree.name(n)? {
        Some(m) if m.prefix.is_empty() => m.local.to_string(),
        Some(m) => format!("{}:{}", m.prefix, m.local),
        None => String::new(),
    })
}

fn attr(tree: &Tree, n: NodeId, local: &str) -> Result<String, Error> {
    Ok(tree
        .attribute(n, Some(W), local)?
        .unwrap_or_default()
        .into_owned())
}

impl Sink for Collector<'_> {
    fn event(&mut self, st: &State<'_>, ev: Event<'_>) -> Result<(), Error> {
        let tree = self.at.tree;
        match ev {
            Event::BlockStart(n) if self.at.story == StoryKind::Main => {
                self.at.block = self.ids.get(&n).copied();
            }
            Event::Start(Kind::Paragraph, p) if self.at.story == StoryKind::Comment => {
                self.last_para_id = props::para_id(tree, p)?.or(self.last_para_id);
            }
            _ => {}
        }
        // A drawing's properties are read once for every view.
        let drawing = match ev {
            Event::Object(Kind::Drawing, n) => {
                let p = props::drawing_props(tree, n)?;
                for b in &p.text_boxes {
                    self.boxes.push((*b, self.at.block));
                }
                if p.kind == DrawingKind::Picture {
                    self.pictures += 1;
                }
                Some(p)
            }
            _ => None,
        };
        if self.at.story == StoryKind::Main {
            self.blocks(st, ev, drawing.as_ref())?;
        }
        self.table(st, ev)?;
        self.field(st, ev);
        self.revision(st, ev)?;
        self.mark(st, ev, drawing)?;
        Ok(())
    }
}

impl Collector<'_> {
    // ---- blocks --------------------------------------------------------------

    fn blocks(
        &mut self,
        st: &State<'_>,
        ev: Event<'_>,
        drawing: Option<&DrawingProps>,
    ) -> Result<(), Error> {
        let tree = self.at.tree;
        match ev {
            Event::BlockStart(n) => {
                self.block_start = Some((n, st.at()));
                if tree.is(n, Some(W), "p")? {
                    self.current = Some((n, Content::default()));
                }
                return Ok(());
            }
            Event::BlockEnd(n) => {
                if let Some((b, start)) = self.block_start.take()
                    && b == n
                {
                    self.block_spans.insert(n, span(start, st.at(), 0, 0));
                }
                if let Some((p, c)) = self.current.take()
                    && p == n
                {
                    self.content.insert(p, c);
                }
                return Ok(());
            }
            Event::Start(Kind::Run, r) => {
                self.run_format = props::run_format(tree, r)?;
                return Ok(());
            }
            _ => {}
        }
        let Some((_, content)) = self.current.as_mut() else {
            return Ok(());
        };
        let items = &mut content.items;
        let rels = self.at.rels;
        let item = match ev {
            // Text inside a field code, such as the result of a field nested
            // in the code, is not content.
            Event::Text(_) if st.in_code() => return Ok(()),
            Event::Text(t) => {
                if let (Some(Inline::Text { text, .. }), true) = (
                    items.last_mut(),
                    self.last_run == st.run && st.run.is_some(),
                ) {
                    text.push_str(t);
                    return Ok(());
                }
                self.last_run = st.run;
                Inline::Text {
                    text: t.to_string(),
                    format: self.run_format.clone(),
                    toggles: st.toggles(),
                    revision: change_info(tree, st)?,
                }
            }
            Event::FieldSeparate if !st.in_code() => {
                let Some(f) = st.field() else { return Ok(()) };
                self.started.insert(f);
                Inline::FieldStart {
                    field: f,
                    code: st.code().trim().to_string(),
                }
            }
            Event::Start(Kind::SimpleField, _) if st.parent_field().is_none() || !st.in_code() => {
                let Some(f) = st.field() else { return Ok(()) };
                self.started.insert(f);
                Inline::FieldStart {
                    field: f,
                    code: st.code().trim().to_string(),
                }
            }
            Event::FieldEnd { index, code } => {
                if self.started.remove(&index) {
                    Inline::FieldEnd { field: index }
                } else if !st.in_code() {
                    items.push(Inline::FieldStart {
                        field: index,
                        code: code.trim().to_string(),
                    });
                    Inline::FieldEnd { field: index }
                } else {
                    return Ok(());
                }
            }
            Event::Start(Kind::Hyperlink, n) => {
                let target = tree
                    .attribute(n, Some(R), "id")?
                    .and_then(|id| rels.get(&*id).map(|r| r.target.clone()));
                let anchor = tree
                    .attribute(n, Some(W), "anchor")?
                    .map(|v| v.into_owned());
                Inline::HyperlinkStart { target, anchor }
            }
            Event::End(Kind::Hyperlink, _) => Inline::HyperlinkEnd,
            Event::Start(Kind::ContentControl, n) => {
                let p = props::control_props(tree, n)?;
                Inline::ContentControlStart {
                    kind: p.kind,
                    tag: p.tag,
                    title: p.title,
                    placeholder: p.placeholder,
                }
            }
            Event::End(Kind::ContentControl, _) => Inline::ContentControlEnd,
            Event::Object(Kind::BookmarkStart, n) => {
                let name = attr(tree, n, "name")?;
                self.bookmark_names
                    .insert(attr(tree, n, "id")?, name.clone());
                Inline::BookmarkStart { name }
            }
            Event::Object(Kind::BookmarkEnd, n) => Inline::BookmarkEnd {
                name: self.bookmark_names.get(&attr(tree, n, "id")?).cloned(),
            },
            Event::Object(
                k @ (Kind::CommentStart | Kind::CommentEnd | Kind::CommentReference),
                n,
            ) => {
                let comment = attr(tree, n, "id")?;
                match k {
                    Kind::CommentStart => Inline::CommentStart { comment },
                    Kind::CommentEnd => Inline::CommentEnd { comment },
                    _ => Inline::CommentReference { comment },
                }
            }
            Event::Object(Kind::NoteReference, n) => Inline::NoteReference {
                note: if tree.is(n, Some(W), "endnoteReference")? {
                    NoteKind::Endnote
                } else {
                    NoteKind::Footnote
                },
                id: attr(tree, n, "id")?,
                number: st.note_number(),
            },
            Event::Object(Kind::Drawing, _) => {
                let Some(p) = drawing else { return Ok(()) };
                if !p.text_boxes.is_empty() {
                    content.boxes.push((items.len(), p.text_boxes.clone()));
                }
                Inline::Drawing {
                    kind: p.kind,
                    placement: p.placement,
                    image: (p.kind == DrawingKind::Picture).then(|| self.pictures - 1),
                    alt: p.alt.clone(),
                    text: None,
                }
            }
            Event::Math(text) => Inline::Math {
                text: text.to_string(),
            },
            Event::Object(Kind::Break, n) => Inline::Break {
                kind: attr(tree, n, "type")?,
            },
            Event::Object(Kind::Opaque, n) => Inline::Opaque {
                element: qualified(tree, n)?,
            },
            _ => return Ok(()),
        };
        items.push(item);
        Ok(())
    }

    // ---- tables --------------------------------------------------------------

    fn table(&mut self, st: &State<'_>, ev: Event<'_>) -> Result<(), Error> {
        let tree = self.at.tree;
        match ev {
            Event::Start(k @ (Kind::Paragraph | Kind::Row | Kind::Cell), n) => {
                self.containers.push((k, n));
                if k == Kind::Row
                    && let Some(t) = self.stack.last_mut()
                {
                    t.row_hidden = false;
                    t.column = int_val(tree, tree.child(n, Some(W), "trPr")?, "gridBefore")?
                        .min(walk::MAX_SPAN);
                }
                if k == Kind::Cell {
                    self.start_cell(st, n)?;
                }
            }
            Event::Start(Kind::Table, n) => {
                let index = self.tables.len();
                self.tables.push(None);
                let skip = self.stack.last().is_some_and(|t| t.skip || t.row_hidden);
                let mut parent = None;
                if let Some(t) = self.stack.last_mut()
                    && let Some(i) = t.current
                    && !skip
                {
                    let cell = &mut t.table.cells[i];
                    cell.tables.push(index);
                    parent = Some(CellRef {
                        table: t.index,
                        row: cell.row,
                        column: cell.column,
                    });
                }
                let pr = tree.child(n, Some(W), "tblPr")?;
                let style = match pr {
                    Some(p) => child_val(tree, p, "tblStyle")?,
                    None => None,
                };
                let mut grid = 0;
                if let Some(g) = tree.child(n, Some(W), "tblGrid")? {
                    grid = tree.elements(g)?.count();
                }
                let id = if self.stack.is_empty() && self.at.story == StoryKind::Main {
                    self.ids.get(&n).copied()
                } else {
                    None
                };
                self.stack.push(TableBuild {
                    index,
                    table: Table {
                        index,
                        id,
                        location: self.at.location(),
                        parent,
                        style,
                        row_count: 0,
                        column_count: 0,
                        cells: vec![],
                        rows: vec![],
                    },
                    spans: vec![],
                    skip,
                    row: 0,
                    row_hidden: false,
                    column: 0,
                    width: grid.min(walk::MAX_SPAN),
                    vertical: HashMap::new(),
                    current: None,
                    cell_start: (0, 0),
                });
            }
            Event::Object(Kind::MarkChange, m) => {
                if let Some((Kind::Row, _)) = self.containers.last()
                    && !mark_shown(tree, m, Revisions::Accepted)?
                    && let Some(t) = self.stack.last_mut()
                {
                    t.row_hidden = true;
                }
            }
            Event::End(Kind::Paragraph, _) => {
                self.containers.pop();
            }
            Event::End(Kind::Cell, _) => {
                self.containers.pop();
                if let Some(t) = self.stack.last_mut()
                    && let Some(i) = t.current.take()
                    && !t.skip
                    && !t.row_hidden
                {
                    let (start, depth) = t.cell_start;
                    let mut s = span(start, st.at(), depth, 0);
                    s.own_level = true;
                    t.spans[i].push(s);
                }
            }
            Event::End(Kind::Row, _) => {
                self.containers.pop();
                if let Some(t) = self.stack.last_mut() {
                    if !t.row_hidden && !t.skip {
                        t.row += 1;
                    }
                    t.row_hidden = false;
                }
            }
            Event::End(Kind::Table, _) => {
                let Some(b) = self.stack.pop() else {
                    return Ok(());
                };
                if b.skip {
                    return Ok(());
                }
                let mut table = b.table;
                table.row_count = b.row;
                table.column_count = b.width;
                for c in &table.cells {
                    table.column_count = table.column_count.max(c.column + c.column_span);
                    table.row_count = table.row_count.max(c.row + c.row_span);
                }
                let slots = table.row_count.saturating_mul(table.column_count);
                if slots > (table.cells.len() * 16).max(1 << 16) {
                    return Err(Error::Unsupported(format!(
                        "a table of {} rows and {} columns with {} cells",
                        table.row_count,
                        table.column_count,
                        table.cells.len()
                    )));
                }
                self.tables[b.index] = Some(TableSpans {
                    table,
                    cells: b.spans,
                });
            }
            _ => {}
        }
        Ok(())
    }

    fn start_cell(&mut self, st: &State<'_>, n: NodeId) -> Result<(), Error> {
        let tree = self.at.tree;
        let pr = tree.child(n, Some(W), "tcPr")?;
        let span_of = int_val(tree, pr, "gridSpan")?.clamp(1, walk::MAX_SPAN);
        let vmerge = match pr {
            Some(p) => match tree.child(p, Some(W), "vMerge")? {
                Some(v) => Some(tree.attribute(v, Some(W), "val")?.as_deref() == Some("restart")),
                None => None,
            },
            None => None,
        };
        let hmerge_continue = match pr {
            Some(p) => match tree.child(p, Some(W), "hMerge")? {
                Some(v) => tree.attribute(v, Some(W), "val")?.as_deref() != Some("restart"),
                None => false,
            },
            None => false,
        };
        let Some(t) = self.stack.last_mut() else {
            return Ok(());
        };
        if t.skip || t.row_hidden {
            return Ok(());
        }
        t.cell_start = (st.at(), st.depth());
        let col = t.column;
        t.column = t.column.saturating_add(span_of);
        t.width = t.width.max(t.column);
        let cells = &mut t.table.cells;
        if vmerge == Some(false)
            && let Some(&i) = t.vertical.get(&col)
            && cells[i].row + cells[i].row_span == t.row
        {
            cells[i].row_span += 1;
            t.current = Some(i);
            return Ok(());
        }
        if hmerge_continue
            && let Some(last) = cells.last_mut()
            && last.row == t.row
            && last.column + last.column_span == col
        {
            last.column_span += span_of;
            t.current = Some(cells.len() - 1);
            return Ok(());
        }
        cells.push(Cell {
            row: t.row,
            column: col,
            row_span: 1,
            column_span: span_of,
            text: String::new(),
            tables: vec![],
        });
        t.spans.push(vec![]);
        let i = cells.len() - 1;
        for c in col..col + span_of {
            if vmerge == Some(true) {
                t.vertical.insert(c, i);
            } else {
                t.vertical.remove(&c);
            }
        }
        t.current = Some(i);
        Ok(())
    }

    // ---- fields --------------------------------------------------------------

    fn begin_field(&mut self, st: &State<'_>, simple: bool) {
        let Some(index) = st.field() else { return };
        if self.fields.len() <= index {
            self.fields.resize_with(index + 1, || None);
        }
        self.fields[index] = Some((
            Field {
                index,
                kind: String::new(),
                code: String::new(),
                result: String::new(),
                simple,
                parent: st.parent_field(),
                location: self.at.location(),
            },
            None,
        ));
        if simple {
            self.results
                .insert(index, (st.at(), st.depth(), st.code_depth()));
        }
    }

    fn field(&mut self, st: &State<'_>, ev: Event<'_>) {
        match ev {
            Event::FieldBegin => self.begin_field(st, false),
            Event::Start(Kind::SimpleField, _) => self.begin_field(st, true),
            Event::FieldSeparate => {
                if let Some(i) = st.field() {
                    self.results
                        .insert(i, (st.at(), st.depth(), st.code_depth()));
                }
            }
            Event::FieldEnd { index, code } => {
                let result = self
                    .results
                    .remove(&index)
                    .map(|(start, depth, code)| span(start, st.at(), depth, code));
                if let Some(Some((f, r))) = self.fields.get_mut(index) {
                    *r = result;
                    f.code = code.trim().to_string();
                    f.kind = f
                        .code
                        .split_whitespace()
                        .next()
                        .unwrap_or_default()
                        .to_uppercase();
                }
            }
            _ => {}
        }
    }

    // ---- revisions -----------------------------------------------------------

    fn open_revision(
        &mut self,
        st: &State<'_>,
        node: NodeId,
        kind: RevisionKind,
        target: RevisionTarget,
        until: NodeId,
    ) -> Result<(), Error> {
        let (author, date) = props::author_date(self.at.tree, node)?;
        // Content inside a row or cell that is inserted or deleted as a
        // whole belongs to that revision, as Word reports it. Property
        // changes of rows and cells belong to the change of their table's
        // properties.
        let absorbed = if kind == RevisionKind::Formatting {
            matches!(target, RevisionTarget::Row | RevisionTarget::Cell)
                && self.table_records.contains(&author)
        } else {
            self.row_records
                .iter()
                .any(|(k, a)| *k == kind && *a == author)
        };
        let row_record = !absorbed
            && kind != RevisionKind::Formatting
            && matches!(target, RevisionTarget::Row | RevisionTarget::Cell);
        if row_record {
            self.row_records.push((kind, author.clone()));
        }
        let table =
            !absorbed && kind == RevisionKind::Formatting && target == RevisionTarget::Table;
        if table {
            self.table_records.push(author.clone());
        }
        let seq = self.seq;
        self.seq += 1;
        self.open.push(OpenRevision {
            seq,
            until,
            start: st.at(),
            depth: st.depth(),
            record: (!absorbed).then(|| Revision {
                index: 0,
                kind,
                target,
                author,
                date,
                text: String::new(),
                location: self.at.location(),
            }),
            table,
            parts: vec![],
            row_record,
            mark_start: self.marks,
        });
        Ok(())
    }

    /// Ends the revisions that last until `node`. They were opened in the
    /// order of the elements they last until, so they are the last ones.
    fn close_revisions(&mut self, st: &State<'_>, node: NodeId) {
        while self.open.last().is_some_and(|o| o.until == node) {
            let Some(mut o) = self.open.pop() else { break };
            if o.row_record {
                self.row_records.pop();
            }
            if o.table {
                self.table_records.pop();
            }
            let Some(revision) = o.record else { continue };
            if !o.table || o.parts.is_empty() {
                o.parts = vec![Part::Span(span(o.start, st.at(), o.depth, 0))];
            }
            self.revisions.push(RevisionSpans {
                seq: o.seq,
                revision,
                parts: o.parts,
                start: o.mark_start,
                end: self.marks,
            });
        }
    }

    fn revision(&mut self, st: &State<'_>, ev: Event<'_>) -> Result<(), Error> {
        let tree = self.at.tree;
        match ev {
            Event::Start(k, n) => {
                // A property change follows the start of the element it
                // belongs to.
                self.last_start = Some(n);
                if k == Kind::Row {
                    self.row_changed = tree
                        .child(n, Some(W), "tblPrEx")?
                        .map(|x| tree.child(x, Some(W), "tblPrExChange"))
                        .transpose()?
                        .flatten()
                        .is_some();
                    self.row_start = st.at();
                }
                if k == Kind::Change
                    && let Some(kind) = walk::change_kind(tree, n)?
                {
                    self.open_revision(st, n, kind, RevisionTarget::Text, n)?;
                }
            }
            Event::End(k, n) => {
                if k == Kind::Row && self.row_changed {
                    // The row belongs to the open changes of its table.
                    let row = span(self.row_start, st.at(), st.depth(), 0);
                    for o in self
                        .open
                        .iter_mut()
                        .filter(|o| o.table && o.record.is_some())
                    {
                        crate::cost::tick(1);
                        o.parts.push(Part::Span(row));
                    }
                    self.row_changed = false;
                }
                self.close_revisions(st, n);
            }
            Event::Object(Kind::MarkChange, m) => {
                let Some(kind) = walk::change_kind(tree, m)? else {
                    return Ok(());
                };
                match self.containers.last().copied() {
                    Some((Kind::Paragraph, _)) => {
                        let (author, _) = props::author_date(tree, m)?;
                        // The mark joins an open revision of its row or cell
                        // of the same kind and author, as Word reports it.
                        let joined = self
                            .row_records
                            .iter()
                            .any(|(k, a)| *k == kind && *a == author);
                        if !joined {
                            self.open_revision(st, m, kind, RevisionTarget::ParagraphMark, m)?;
                            if let Some(o) = self.open.last_mut() {
                                o.parts.push(Part::Mark);
                                o.table = false;
                            }
                            if let Some(o) = self.open.pop()
                                && let Some(revision) = o.record
                            {
                                self.revisions.push(RevisionSpans {
                                    seq: o.seq,
                                    revision,
                                    parts: o.parts,
                                    start: self.marks,
                                    end: self.marks,
                                });
                            }
                        }
                    }
                    Some((Kind::Row, row)) => {
                        self.open_revision(st, m, kind, RevisionTarget::Row, row)?
                    }
                    Some((Kind::Cell, cell)) => {
                        self.open_revision(st, m, kind, RevisionTarget::Cell, cell)?
                    }
                    _ => {}
                }
            }
            Event::Object(Kind::PropertyChange, c) => {
                let local = tree.name(c)?.map(|m| m.local).unwrap_or_default();
                let (target, until) = match local {
                    "rPrChange" => (RevisionTarget::Run, self.last_start),
                    "pPrChange" => (RevisionTarget::Paragraph, self.last_start),
                    "tblPrChange" => (RevisionTarget::Table, self.last_start),
                    "trPrChange" => (RevisionTarget::Row, self.last_start),
                    "tcPrChange" => (RevisionTarget::Cell, self.last_start),
                    _ => (RevisionTarget::Text, None),
                };
                if matches!(target, RevisionTarget::Row | RevisionTarget::Cell) {
                    self.row_changed = true;
                }
                if let Some(until) = until {
                    self.open_revision(st, c, RevisionKind::Formatting, target, until)?;
                }
            }
            Event::Object(Kind::Section, s) => {
                if let Some(c) = tree.child(s, Some(W), "sectPrChange")? {
                    self.open_revision(
                        st,
                        c,
                        RevisionKind::Formatting,
                        RevisionTarget::Section,
                        c,
                    )?;
                    self.close_revisions(st, c);
                }
            }
            Event::Text(_) if !st.in_code() && !st.toggles().has(super::Toggle::Hidden) => {
                self.marks += 1;
            }
            _ => {}
        }
        Ok(())
    }

    // ---- bookmarks, comment anchors, references, pictures -------------------

    fn mark(
        &mut self,
        st: &State<'_>,
        ev: Event<'_>,
        drawing: Option<DrawingProps>,
    ) -> Result<(), Error> {
        let tree = self.at.tree;
        match ev {
            Event::Object(Kind::BookmarkStart, n) => {
                let name = attr(tree, n, "name")?;
                let id = attr(tree, n, "id")?;
                self.open_bookmarks
                    .insert(id.clone(), (self.bookmarks.len(), st.at(), st.depth()));
                self.bookmarks.push((
                    Bookmark {
                        hidden: name.starts_with('_'),
                        name,
                        id,
                        text: String::new(),
                        location: self.at.location(),
                    },
                    None,
                ));
            }
            Event::Object(Kind::BookmarkEnd, n) => {
                if let Some((i, start, depth)) = self.open_bookmarks.remove(&attr(tree, n, "id")?) {
                    self.bookmarks[i].1 = Some(span(start, st.at(), depth, 0));
                }
            }
            Event::Object(Kind::CommentStart, n) => {
                self.open_anchors
                    .insert(attr(tree, n, "id")?, (st.at(), st.depth()));
            }
            Event::Object(Kind::CommentEnd, n) => {
                let id = attr(tree, n, "id")?;
                if let Some((start, depth)) = self.open_anchors.remove(&id) {
                    self.anchors.insert(id, span(start, st.at(), depth, 0));
                }
            }
            Event::Start(Kind::ContentControl, n) => {
                let p = props::control_props(tree, n)?;
                self.open_controls
                    .push((self.controls.len(), st.at(), st.depth()));
                self.controls.push((
                    Control {
                        control: ContentControl {
                            kind: p.kind,
                            tag: p.tag,
                            title: p.title,
                            placeholder: p.placeholder,
                        },
                        text: String::new(),
                        location: self.at.location(),
                    },
                    None,
                ));
            }
            Event::End(Kind::ContentControl, _) => {
                if let Some((i, start, depth)) = self.open_controls.pop() {
                    self.controls[i].1 = Some(span(start, st.at(), depth, 0));
                }
            }
            Event::Object(Kind::CommentReference, n) => {
                self.comment_places
                    .insert(attr(tree, n, "id")?, self.at.location());
            }
            Event::Object(Kind::NoteReference, n) => {
                let kind = if tree.is(n, Some(W), "endnoteReference")? {
                    NoteKind::Endnote
                } else {
                    NoteKind::Footnote
                };
                let custom = tree
                    .attribute(n, Some(W), "customMarkFollows")?
                    .is_some_and(|v| matches!(&*v, "1" | "true" | "on"));
                // A note with its own mark has the mark as text after the
                // reference, in the same run.
                self.mark_run = if custom { st.run } else { None };
                self.note_refs.push(NoteRef {
                    kind,
                    id: attr(tree, n, "id")?,
                    number: st.note_number(),
                    location: self.at.location(),
                    mark: None,
                });
            }
            Event::Text(t) if self.mark_run.is_some() => {
                if st.run == self.mark_run
                    && let Some(r) = self.note_refs.last_mut()
                {
                    r.mark.get_or_insert_with(String::new).push_str(t);
                } else {
                    self.mark_run = None;
                }
            }
            Event::End(Kind::Run, _) => self.mark_run = None,
            Event::Object(Kind::Drawing, _) => {
                let Some(p) = drawing else { return Ok(()) };
                if p.kind == DrawingKind::Picture {
                    let rel = p.embed.clone().or(p.link.clone());
                    let r = rel.as_ref().and_then(|id| self.at.rels.get(id));
                    let part = r.and_then(|r| r.part.clone());
                    let link = r.filter(|r| r.external).map(|r| r.target.clone());
                    let content_type = part
                        .as_ref()
                        .and_then(|p| self.content_types.get(&p.to_ascii_lowercase()).cloned());
                    self.images.push(Image {
                        index: self.images.len(),
                        placement: p.placement,
                        relationship: rel,
                        part,
                        link,
                        content_type,
                        width: p.size.map(|s| round2(s.0)),
                        height: p.size.map(|s| round2(s.1)),
                        name: p.name,
                        title: p.title,
                        alt: p.alt,
                        location: self.at.location(),
                    });
                }
            }
            _ => {}
        }
        Ok(())
    }
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// Joins revisions that Word reports as one: neighbours of the same kind,
/// author and date with no content between them.
pub(crate) fn merge_revisions(mut done: Vec<RevisionSpans>) -> Vec<(Revision, Vec<Part>)> {
    done.sort_by_key(|d| d.seq);
    let mut out: Vec<(Revision, Vec<Part>, usize)> = vec![];
    for d in done {
        let r = d.revision;
        if let Some((last, parts, last_end)) = out.last_mut()
            && matches!(
                last.target,
                RevisionTarget::Text | RevisionTarget::ParagraphMark
            )
            && matches!(
                r.target,
                RevisionTarget::Text | RevisionTarget::ParagraphMark
            )
            && last.kind == r.kind
            && last.author == r.author
            && last.date == r.date
            && last.location == r.location
            && d.start == *last_end
        {
            parts.extend(d.parts);
            if r.target == RevisionTarget::Text {
                last.target = RevisionTarget::Text;
            }
            *last_end = d.end;
            continue;
        }
        out.push((r, d.parts, d.end));
    }
    out.into_iter()
        .enumerate()
        .map(|(i, (mut r, parts, _))| {
            r.index = i;
            (r, parts)
        })
        .collect()
}
