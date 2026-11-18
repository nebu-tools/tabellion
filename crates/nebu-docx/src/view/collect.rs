//! The sinks that turn the events of a walk into views.
//!
//! Each sink keeps a [`Common`] that knows the story, the current
//! top-level block and the text boxes met so far, which the reader walks
//! after their story.

use std::collections::{HashMap, HashSet};

use crate::Error;
use crate::xml::{NodeId, Tree};

use super::props::{self, child_val};
use super::walk::{self, Event, Kind, M, R, Sink, State, W};
use super::{
    Bookmark, Cell, CellRef, Change, ContentControl, Control, DrawingKind, Field, Format, Image,
    Inline, Location, NoteKind, Revision, RevisionKind, RevisionTarget, Revisions, StoryKind,
    Table,
};

/// A relationship's target as the views need it.
#[derive(Debug, Clone)]
pub(crate) struct Rel {
    pub target: String,
    pub external: bool,
    pub part: Option<String>,
}

/// Text that ranges of the stories share. Bookmarks, comment anchors and
/// field results are ranges of it, cut out when they end, so that ranges
/// open at the same time cost no more than the text itself.
pub(crate) struct Spans {
    text: String,
    budget: u64,
    used: u64,
}

impl Spans {
    pub fn new(budget: u64) -> Spans {
        Spans {
            text: String::new(),
            budget,
            used: 0,
        }
    }

    pub fn push(&mut self, s: &str) {
        self.text.push_str(s);
    }

    pub fn pos(&self) -> usize {
        self.text.len()
    }

    /// The text from `start` to here. All cuts together may hold no more
    /// text than [`Limits::max_total_size`](crate::Limits).
    pub fn cut(&mut self, start: usize) -> Result<String, Error> {
        let s = self.text.get(start..).unwrap_or_default();
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
        Ok(s.to_string())
    }
}

/// What every sink tracks.
pub(crate) struct Common<'r> {
    pub tree: &'r Tree,
    pub story: StoryKind,
    pub part: String,
    pub block: Option<String>,
    /// Block ids of the main story.
    ids: &'r HashMap<NodeId, String>,
    /// Relationships of the story's part, by id.
    pub rels: &'r HashMap<String, Rel>,
    /// Text boxes met, with the block that holds them.
    pub boxes: Vec<(NodeId, Option<String>)>,
    /// Note references counted so far, footnotes and endnotes.
    notes: [usize; 2],
    /// The number of the last note reference, if it is numbered.
    pub note_number: Option<usize>,
}

impl<'r> Common<'r> {
    pub fn new(
        tree: &'r Tree,
        ids: &'r HashMap<NodeId, String>,
        rels: &'r HashMap<String, Rel>,
    ) -> Common<'r> {
        Common {
            tree,
            story: StoryKind::Main,
            part: String::new(),
            block: None,
            ids,
            rels,
            boxes: vec![],
            notes: [0, 0],
            note_number: None,
        }
    }

    /// Starts a story.
    pub fn enter(
        &mut self,
        tree: &'r Tree,
        story: StoryKind,
        part: &str,
        block: Option<String>,
        rels: &'r HashMap<String, Rel>,
    ) {
        self.tree = tree;
        self.story = story;
        self.part = part.to_string();
        self.block = block;
        self.rels = rels;
    }

    pub fn location(&self) -> Location {
        Location {
            story: self.story,
            part: self.part.clone(),
            block: self.block.clone(),
        }
    }

    fn track(&mut self, st: &State, ev: &Event<'_>) -> Result<(), Error> {
        match *ev {
            Event::BlockStart(n) if self.story == StoryKind::Main => {
                self.block = self.ids.get(&n).cloned();
            }
            Event::Object(Kind::Drawing, n) => {
                for b in props::drawing_props(self.tree, n)?.text_boxes {
                    self.boxes.push((b, self.block.clone()));
                }
            }
            Event::Object(Kind::NoteReference, n) => {
                let custom = self
                    .tree
                    .attribute(n, Some(W), "customMarkFollows")?
                    .is_some_and(|v| matches!(&*v, "1" | "true" | "on"));
                let k = usize::from(self.tree.is(n, Some(W), "endnoteReference")?);
                // A reference in deleted text or a field code is not counted.
                self.note_number = if custom || !st.shown(Revisions::Accepted) || st.in_code() {
                    None
                } else {
                    self.notes[k] += 1;
                    Some(self.notes[k])
                };
            }
            _ => {}
        }
        Ok(())
    }
}

/// Sinks that keep a [`Common`].
pub(crate) trait Collect<'r>: Sink {
    fn common(&mut self) -> &mut Common<'r>;
}

fn change_info(tree: &Tree, st: &State) -> Result<Option<Change>, Error> {
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

/// The text of an equation: its `m:t` elements.
fn math_text(tree: &Tree, n: NodeId) -> Result<String, Error> {
    let mut s = String::new();
    for d in tree.descendants(n)? {
        if tree.is(d, Some(M), "t")? {
            s.push_str(&tree.text_content(d)?);
        }
    }
    Ok(s)
}

// ---- text ------------------------------------------------------------------

/// Renders a story as plain text: paragraphs as lines, a table's rows as
/// lines of tab-separated cells, nested tables inline with spaces.
pub(crate) struct TextSink<'r> {
    pub c: Common<'r>,
    view: Revisions,
    note_marks: bool,
    pub out: String,
    containers: Vec<Kind>,
    tables: usize,
    first_cell: Vec<bool>,
    hidden_rows: Vec<bool>,
    spans: Vec<usize>,
    suppress: usize,
    mark_removed: bool,
    pending: Option<char>,
    /// The top-level blocks with the range of `out` they gave.
    pub blocks: Vec<(NodeId, usize, usize)>,
    block_start: usize,
}

impl<'r> TextSink<'r> {
    pub fn new(c: Common<'r>, view: Revisions, note_marks: bool) -> TextSink<'r> {
        TextSink {
            c,
            view,
            note_marks,
            out: String::new(),
            containers: vec![],
            tables: 0,
            first_cell: vec![],
            hidden_rows: vec![],
            spans: vec![],
            suppress: 0,
            mark_removed: false,
            pending: None,
            blocks: vec![],
            block_start: 0,
        }
    }

    fn push(&mut self, text: &str) {
        if let Some(p) = self.pending.take() {
            self.out.push(p);
        }
        if self.tables > 0 && text.contains('\n') {
            self.out.push_str(&text.replace('\n', " "));
        } else {
            self.out.push_str(text);
        }
    }

    fn sep(&self) -> char {
        if self.tables > 1 { ' ' } else { '\t' }
    }
}

impl<'r> Collect<'r> for TextSink<'r> {
    fn common(&mut self) -> &mut Common<'r> {
        &mut self.c
    }
}

impl Sink for TextSink<'_> {
    fn event(&mut self, st: &State, ev: Event<'_>) -> Result<(), Error> {
        self.c.track(st, &ev)?;
        let tree = self.c.tree;
        match ev {
            Event::BlockStart(_) => self.block_start = self.out.len(),
            Event::BlockEnd(n) => self.blocks.push((n, self.block_start, self.out.len())),
            Event::Start(k @ (Kind::Paragraph | Kind::Table | Kind::Row | Kind::Cell), n) => {
                self.containers.push(k);
                match k {
                    Kind::Paragraph => self.mark_removed = false,
                    Kind::Table => self.tables += 1,
                    Kind::Row => {
                        let before = int_val(tree, tree.child(n, Some(W), "trPr")?, "gridBefore")?;
                        self.first_cell.push(before == 0);
                        self.hidden_rows.push(false);
                        if self.suppress == 0 {
                            for _ in 0..before.min(64) {
                                let s = self.sep().to_string();
                                self.out.push_str(&s);
                            }
                        }
                    }
                    _ => {
                        self.pending = None;
                        let first = self
                            .first_cell
                            .last_mut()
                            .map(|b| std::mem::replace(b, false));
                        if self.suppress == 0 && first == Some(false) {
                            let s = self.sep().to_string();
                            self.out.push_str(&s);
                        }
                        let span = int_val(tree, tree.child(n, Some(W), "tcPr")?, "gridSpan")?;
                        self.spans.push(span.clamp(1, 64));
                    }
                }
            }
            Event::End(k @ (Kind::Paragraph | Kind::Table | Kind::Row | Kind::Cell), _) => {
                self.containers.pop();
                match k {
                    Kind::Paragraph => {
                        if self.suppress == 0 && !self.mark_removed {
                            if self.tables > 0 {
                                self.pending = Some(' ');
                            } else {
                                self.out.push('\n');
                            }
                        }
                    }
                    Kind::Table => self.tables = self.tables.saturating_sub(1),
                    Kind::Row => {
                        self.first_cell.pop();
                        if self.hidden_rows.pop() == Some(true) {
                            self.suppress -= 1;
                        } else if self.suppress == 0 {
                            self.pending = None;
                            self.out.push(if self.tables > 1 { ' ' } else { '\n' });
                        }
                    }
                    _ => {
                        self.pending = None;
                        let span = self.spans.pop().unwrap_or(1);
                        if self.suppress == 0 && self.tables == 1 {
                            for _ in 1..span {
                                self.out.push('\t');
                            }
                        }
                    }
                }
            }
            Event::Object(Kind::MarkChange, m) => match self.containers.last() {
                Some(Kind::Paragraph) => self.mark_removed = !mark_shown(tree, m, self.view)?,
                Some(Kind::Row) if !mark_shown(tree, m, self.view)? => {
                    if let Some(h) = self.hidden_rows.last_mut()
                        && !*h
                    {
                        *h = true;
                        self.suppress += 1;
                    }
                }
                _ => {}
            },
            Event::Text(t) if self.suppress == 0 && st.visible(self.view) => self.push(t),
            Event::Object(Kind::Math, n)
                if self.suppress == 0 && st.shown(self.view) && !st.in_code() =>
            {
                let t = math_text(tree, n)?;
                self.push(&t);
            }
            Event::Object(Kind::NoteReference, n)
                if self.note_marks && self.suppress == 0 && st.visible(self.view) =>
            {
                if let Some(num) = self.c.note_number {
                    let endnote = tree.is(n, Some(W), "endnoteReference")?;
                    let mark = if endnote {
                        format!("[{}]", roman(num))
                    } else {
                        format!("[{num}]")
                    };
                    self.push(&mark);
                }
            }
            _ => {}
        }
        Ok(())
    }
}

/// Lower-case roman numerals, as Word numbers endnotes.
pub(crate) fn roman(mut n: usize) -> String {
    const R: [(usize, &str); 13] = [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ];
    if n == 0 || n > 100_000 {
        return n.to_string();
    }
    let mut s = String::new();
    for (v, r) in R {
        while n >= v {
            s.push_str(r);
            n -= v;
        }
    }
    s
}

// ---- blocks ------------------------------------------------------------------

/// The content of top-level paragraphs, with the text of every block.
pub(crate) struct BlocksSink<'r> {
    pub text: TextSink<'r>,
    pub content: HashMap<NodeId, Vec<Inline>>,
    current: Option<(NodeId, Vec<Inline>)>,
    /// The run the last text item came from.
    last_run: Option<NodeId>,
    run_format: Format,
    bookmarks: HashMap<String, String>,
    started: HashSet<usize>,
    pictures: usize,
    /// Gives the text of a text box.
    box_text: &'r dyn Fn(NodeId) -> Result<String, Error>,
}

impl<'r> BlocksSink<'r> {
    pub fn new(
        c: Common<'r>,
        box_text: &'r dyn Fn(NodeId) -> Result<String, Error>,
    ) -> BlocksSink<'r> {
        BlocksSink {
            text: TextSink::new(c, Revisions::Accepted, false),
            content: HashMap::new(),
            current: None,
            last_run: None,
            run_format: Format::default(),
            bookmarks: HashMap::new(),
            started: HashSet::new(),
            pictures: 0,
            box_text,
        }
    }
}

impl<'r> Collect<'r> for BlocksSink<'r> {
    fn common(&mut self) -> &mut Common<'r> {
        &mut self.text.c
    }
}

impl Sink for BlocksSink<'_> {
    fn event(&mut self, st: &State, ev: Event<'_>) -> Result<(), Error> {
        self.text.event(st, ev)?;
        let tree = self.text.c.tree;
        if let Event::Object(Kind::Drawing, n) = ev
            && props::drawing_props(tree, n)?.kind == DrawingKind::Picture
        {
            self.pictures += 1;
        }
        match ev {
            Event::BlockStart(n) if tree.is(n, Some(W), "p")? => {
                self.current = Some((n, vec![]));
                return Ok(());
            }
            Event::BlockEnd(n) => {
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
        let Some((_, items)) = self.current.as_mut() else {
            return Ok(());
        };
        let rels = self.text.c.rels;
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
                    revision: change_info(tree, st)?,
                    hidden: st.hidden(),
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
                let name = tree
                    .attribute(n, Some(W), "name")?
                    .unwrap_or_default()
                    .into_owned();
                if let Some(id) = tree.attribute(n, Some(W), "id")? {
                    self.bookmarks.insert(id.into_owned(), name.clone());
                }
                Inline::BookmarkStart { name }
            }
            Event::Object(Kind::BookmarkEnd, n) => Inline::BookmarkEnd {
                name: tree
                    .attribute(n, Some(W), "id")?
                    .and_then(|id| self.bookmarks.get(&*id).cloned()),
            },
            Event::Object(
                k @ (Kind::CommentStart | Kind::CommentEnd | Kind::CommentReference),
                n,
            ) => {
                let comment = tree
                    .attribute(n, Some(W), "id")?
                    .unwrap_or_default()
                    .into_owned();
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
                id: tree
                    .attribute(n, Some(W), "id")?
                    .unwrap_or_default()
                    .into_owned(),
                number: self.text.c.note_number,
            },
            Event::Object(Kind::Drawing, n) => {
                let p = props::drawing_props(tree, n)?;
                let text = if p.text_boxes.is_empty() {
                    None
                } else {
                    let mut all = String::new();
                    for b in &p.text_boxes {
                        all.push_str(&(self.box_text)(*b)?);
                    }
                    Some(all.trim_end_matches('\n').to_string())
                };
                Inline::Drawing {
                    kind: p.kind,
                    placement: p.placement,
                    image: (p.kind == DrawingKind::Picture).then(|| self.pictures - 1),
                    alt: p.alt,
                    text,
                }
            }
            Event::Object(Kind::Math, n) => Inline::Math {
                text: math_text(tree, n)?,
            },
            Event::Object(Kind::Break, n) => Inline::Break {
                kind: tree
                    .attribute(n, Some(W), "type")?
                    .unwrap_or_default()
                    .into_owned(),
            },
            Event::Object(Kind::Opaque, n) => Inline::Opaque {
                element: qualified(tree, n)?,
            },
            _ => return Ok(()),
        };
        items.push(item);
        Ok(())
    }
}

/// An element's name as written.
pub(crate) fn qualified(tree: &Tree, n: NodeId) -> Result<String, Error> {
    Ok(match tree.name(n)? {
        Some(m) if m.prefix.is_empty() => m.local.to_string(),
        Some(m) => format!("{}:{}", m.prefix, m.local),
        None => String::new(),
    })
}

// ---- tables ------------------------------------------------------------------

struct TableBuild {
    table: Table,
    skip: bool,
    row: usize,
    row_hidden: bool,
    column: usize,
    width: usize,
    /// The cell that holds each grid column while a vertical merge goes on.
    vertical: HashMap<usize, usize>,
    current: Option<usize>,
}

/// More grid positions per cell than this is not a table a person made.
const MAX_SPAN: usize = 1024;

/// Tables on their grid.
pub(crate) struct TablesSink<'r> {
    pub c: Common<'r>,
    stack: Vec<TableBuild>,
    containers: Vec<(Kind, NodeId)>,
    pub tables: Vec<Option<Table>>,
}

impl<'r> TablesSink<'r> {
    pub fn new(c: Common<'r>) -> TablesSink<'r> {
        TablesSink {
            c,
            stack: vec![],
            containers: vec![],
            tables: vec![],
        }
    }

    fn top_cell(&mut self) -> Option<&mut Cell> {
        let t = self.stack.last_mut()?;
        if t.skip || t.row_hidden {
            return None;
        }
        let i = t.current?;
        t.table.cells.get_mut(i)
    }
}

impl<'r> Collect<'r> for TablesSink<'r> {
    fn common(&mut self) -> &mut Common<'r> {
        &mut self.c
    }
}

impl Sink for TablesSink<'_> {
    fn event(&mut self, st: &State, ev: Event<'_>) -> Result<(), Error> {
        self.c.track(st, &ev)?;
        let tree = self.c.tree;
        match ev {
            Event::Start(k, n) if matches!(k, Kind::Paragraph | Kind::Row | Kind::Cell) => {
                self.containers.push((k, n));
                if k == Kind::Row
                    && let Some(t) = self.stack.last_mut()
                {
                    t.row_hidden = false;
                    t.column =
                        int_val(tree, tree.child(n, Some(W), "trPr")?, "gridBefore")?.min(MAX_SPAN);
                }
                if k == Kind::Cell {
                    let pr = tree.child(n, Some(W), "tcPr")?;
                    let span = int_val(tree, pr, "gridSpan")?.clamp(1, MAX_SPAN);
                    let vmerge = match pr {
                        Some(p) => match tree.child(p, Some(W), "vMerge")? {
                            Some(v) => Some(
                                tree.attribute(v, Some(W), "val")?.as_deref() == Some("restart"),
                            ),
                            None => None,
                        },
                        None => None,
                    };
                    let hmerge_continue = match pr {
                        Some(p) => match tree.child(p, Some(W), "hMerge")? {
                            Some(v) => {
                                tree.attribute(v, Some(W), "val")?.as_deref() != Some("restart")
                            }
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
                    let col = t.column;
                    t.column = t.column.saturating_add(span);
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
                        last.column_span += span;
                        t.current = Some(cells.len() - 1);
                        return Ok(());
                    }
                    cells.push(Cell {
                        row: t.row,
                        column: col,
                        row_span: 1,
                        column_span: span,
                        text: String::new(),
                        tables: vec![],
                    });
                    let i = cells.len() - 1;
                    for c in col..col + span {
                        if vmerge == Some(true) {
                            t.vertical.insert(c, i);
                        } else {
                            t.vertical.remove(&c);
                        }
                    }
                    t.current = Some(i);
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
                        table: t.table.index,
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
                let id = if self.stack.is_empty() && self.c.story == StoryKind::Main {
                    self.c.ids.get(&n).cloned()
                } else {
                    None
                };
                self.stack.push(TableBuild {
                    table: Table {
                        index,
                        id,
                        location: self.c.location(),
                        parent,
                        style,
                        row_count: 0,
                        column_count: 0,
                        cells: vec![],
                        rows: vec![],
                    },
                    skip,
                    row: 0,
                    row_hidden: false,
                    column: 0,
                    width: grid.min(MAX_SPAN),
                    vertical: HashMap::new(),
                    current: None,
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
            Event::Text(text) if st.visible(Revisions::Accepted) => {
                if let Some(c) = self.top_cell() {
                    c.text.push_str(text);
                }
            }
            Event::Object(Kind::Math, n) if st.shown(Revisions::Accepted) && !st.in_code() => {
                let text = math_text(tree, n)?;
                if let Some(c) = self.top_cell() {
                    c.text.push_str(&text);
                }
            }
            Event::End(Kind::Paragraph, _) => {
                self.containers.pop();
                if st.shown(Revisions::Accepted)
                    && !st.in_code()
                    && let Some(c) = self.top_cell()
                {
                    c.text.push('\n');
                }
            }
            Event::End(Kind::Cell, _) => {
                self.containers.pop();
                if let Some(c) = self.top_cell()
                    && c.text.ends_with('\n')
                {
                    c.text.pop();
                }
                if let Some(t) = self.stack.last_mut() {
                    t.current = None;
                }
            }
            Event::End(Kind::Row, _) => {
                self.containers.pop();
                if let Some(t) = self.stack.last_mut()
                    && !t.row_hidden
                    && !t.skip
                {
                    t.row += 1;
                }
                if let Some(t) = self.stack.last_mut() {
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
                let i = table.index;
                self.tables[i] = Some(table);
            }
            _ => {}
        }
        Ok(())
    }
}

// ---- fields ------------------------------------------------------------------

/// Fields with code and result.
pub(crate) struct FieldsSink<'r> {
    pub c: Common<'r>,
    pub fields: Vec<Option<Field>>,
    /// Where each open field's result starts in `spans`.
    open: HashMap<usize, usize>,
    spans: Spans,
}

impl<'r> FieldsSink<'r> {
    pub fn new(c: Common<'r>, budget: u64) -> FieldsSink<'r> {
        FieldsSink {
            c,
            fields: vec![],
            open: HashMap::new(),
            spans: Spans::new(budget),
        }
    }

    fn begin(&mut self, st: &State, simple: bool) {
        let Some(index) = st.field() else { return };
        if self.fields.len() <= index {
            self.fields.resize(index + 1, None);
        }
        self.fields[index] = Some(Field {
            index,
            kind: String::new(),
            code: String::new(),
            result: String::new(),
            simple,
            parent: st.parent_field(),
            location: self.c.location(),
        });
        self.open.insert(index, self.spans.pos());
    }
}

impl<'r> Collect<'r> for FieldsSink<'r> {
    fn common(&mut self) -> &mut Common<'r> {
        &mut self.c
    }
}

impl Sink for FieldsSink<'_> {
    fn event(&mut self, st: &State, ev: Event<'_>) -> Result<(), Error> {
        self.c.track(st, &ev)?;
        match ev {
            Event::FieldBegin => self.begin(st, false),
            Event::FieldSeparate => {
                if let Some(i) = st.field() {
                    self.open.insert(i, self.spans.pos());
                }
            }
            Event::Start(Kind::SimpleField, _) => self.begin(st, true),
            Event::FieldEnd { index, code } => {
                let result = match self.open.remove(&index) {
                    Some(start) => self.spans.cut(start)?,
                    None => String::new(),
                };
                if let Some(Some(f)) = self.fields.get_mut(index) {
                    f.result = result;
                    f.code = code.trim().to_string();
                    f.kind = f
                        .code
                        .split_whitespace()
                        .next()
                        .unwrap_or_default()
                        .to_uppercase();
                }
            }
            Event::Text(t) if result_text(st) => self.spans.push(t),
            Event::Object(Kind::Math, n) if result_text(st) => {
                let t = math_text(self.c.tree, n)?;
                self.spans.push(&t);
            }
            Event::End(Kind::Paragraph, _) if result_text(st) && !st.in_code() => {
                self.spans.push("\n")
            }
            _ => {}
        }
        Ok(())
    }
}

/// Text that belongs to a field result: not hidden and not deleted. A
/// result inside the code of an outer field still belongs to its own field.
fn result_text(st: &State) -> bool {
    !st.hidden() && st.shown(Revisions::Accepted)
}

// ---- revisions ----------------------------------------------------------------

struct OpenRevision {
    seq: usize,
    until: NodeId,
    absorbed: bool,
    record: Option<(Revision, usize)>,
    /// A change of table properties covers the rows whose properties
    /// changed with it, as Word reports it; this is the whole table's text
    /// for a change that no row shares.
    whole: Option<String>,
}

/// Tracked changes with the text they affect.
pub(crate) struct RevisionsSink<'r> {
    pub c: Common<'r>,
    containers: Vec<(Kind, NodeId)>,
    open: Vec<OpenRevision>,
    /// Finished records: sequence, record, content marks at start and end.
    pub done: Vec<(usize, Revision, usize, usize)>,
    seq: usize,
    marks: usize,
    /// The current row has a property change.
    row_changed: bool,
}

impl<'r> RevisionsSink<'r> {
    pub fn new(c: Common<'r>) -> RevisionsSink<'r> {
        RevisionsSink {
            c,
            containers: vec![],
            open: vec![],
            done: vec![],
            seq: 0,
            marks: 0,
            row_changed: false,
        }
    }

    fn open(
        &mut self,
        node: NodeId,
        kind: RevisionKind,
        target: RevisionTarget,
        until: NodeId,
    ) -> Result<(), Error> {
        let (author, date) = props::author_date(self.c.tree, node)?;
        // Content inside a row or cell that is inserted or deleted as a
        // whole belongs to that revision, as Word reports it.
        // Property changes of rows and cells belong to the change of their
        // table's properties.
        let absorbed = self.open.iter().any(|o| {
            o.record.as_ref().is_some_and(|(r, _)| {
                r.kind == kind
                    && r.author == author
                    && if kind == RevisionKind::Formatting {
                        r.target == RevisionTarget::Table
                            && matches!(target, RevisionTarget::Row | RevisionTarget::Cell)
                    } else {
                        matches!(r.target, RevisionTarget::Row | RevisionTarget::Cell)
                    }
            })
        });
        let seq = self.seq;
        self.seq += 1;
        let record = (!absorbed).then(|| {
            (
                Revision {
                    index: 0,
                    kind,
                    target,
                    author,
                    date,
                    text: String::new(),
                    location: self.c.location(),
                },
                self.marks,
            )
        });
        let whole =
            (kind == RevisionKind::Formatting && target == RevisionTarget::Table).then(String::new);
        self.open.push(OpenRevision {
            seq,
            until,
            absorbed,
            record,
            whole,
        });
        Ok(())
    }

    fn close(&mut self, node: NodeId) {
        while let Some(i) = self.open.iter().rposition(|o| o.until == node) {
            let o = self.open.remove(i);
            if let (false, Some((mut r, start))) = (o.absorbed, o.record) {
                if let Some(whole) = o.whole
                    && r.text.is_empty()
                {
                    r.text = whole;
                }
                self.done.push((o.seq, r, start, self.marks));
            }
        }
    }

    fn add(&mut self, text: &str) {
        for o in &mut self.open {
            if let Some(w) = o.whole.as_mut() {
                w.push_str(text);
                if !self.row_changed {
                    continue;
                }
            }
            if let Some((r, _)) = o.record.as_mut() {
                r.text.push_str(text);
            }
        }
    }

    fn container(&self, kind: Kind) -> Option<NodeId> {
        self.containers
            .iter()
            .rev()
            .find(|(k, _)| *k == kind)
            .map(|&(_, n)| n)
    }
}

impl<'r> Collect<'r> for RevisionsSink<'r> {
    fn common(&mut self) -> &mut Common<'r> {
        &mut self.c
    }
}

impl Sink for RevisionsSink<'_> {
    fn event(&mut self, st: &State, ev: Event<'_>) -> Result<(), Error> {
        self.c.track(st, &ev)?;
        let tree = self.c.tree;
        match ev {
            Event::Start(k, n) => {
                if matches!(
                    k,
                    Kind::Paragraph | Kind::Row | Kind::Cell | Kind::Table | Kind::Run
                ) {
                    self.containers.push((k, n));
                }
                if k == Kind::Row {
                    self.row_changed = tree
                        .child(n, Some(W), "tblPrEx")?
                        .map(|x| tree.child(x, Some(W), "tblPrExChange"))
                        .transpose()?
                        .flatten()
                        .is_some();
                }
                if k == Kind::Change
                    && let Some(kind) = walk::change_kind(tree, n)?
                {
                    self.open(n, kind, RevisionTarget::Text, n)?;
                }
            }
            Event::End(k, n) => {
                if k == Kind::Paragraph && !st.in_code() {
                    self.add("\n");
                }
                if k == Kind::Cell {
                    self.add("\t");
                }
                self.close(n);
                if matches!(
                    k,
                    Kind::Paragraph | Kind::Row | Kind::Cell | Kind::Table | Kind::Run
                ) {
                    self.containers.pop();
                }
            }
            Event::Object(Kind::MarkChange, m) => {
                let Some(kind) = walk::change_kind(tree, m)? else {
                    return Ok(());
                };
                match self.containers.last().copied() {
                    Some((Kind::Paragraph, _)) => {
                        let (author, _) = props::author_date(tree, m)?;
                        // The mark joins an open revision of the same kind
                        // and author, as Word reports it.
                        let joined = self.open.iter().any(|o| {
                            o.record.as_ref().is_some_and(|(r, _)| {
                                matches!(r.target, RevisionTarget::Row | RevisionTarget::Cell)
                                    && r.kind == kind
                                    && r.author == author
                            })
                        });
                        if !joined {
                            self.open(m, kind, RevisionTarget::ParagraphMark, m)?;
                            if let Some(o) = self.open.last_mut()
                                && let Some((r, _)) = o.record.as_mut()
                            {
                                r.text.push('\n');
                            }
                            self.close(m);
                        }
                    }
                    Some((Kind::Row, row)) => self.open(m, kind, RevisionTarget::Row, row)?,
                    Some((Kind::Cell, cell)) => self.open(m, kind, RevisionTarget::Cell, cell)?,
                    _ => {}
                }
            }
            Event::Object(Kind::PropertyChange, c) => {
                let local = tree.name(c)?.map(|m| m.local).unwrap_or_default();
                let (target, until) = match local {
                    "rPrChange" => (RevisionTarget::Run, self.container(Kind::Run)),
                    "pPrChange" => (RevisionTarget::Paragraph, self.container(Kind::Paragraph)),
                    "tblPrChange" => (RevisionTarget::Table, self.container(Kind::Table)),
                    "trPrChange" => (RevisionTarget::Row, self.container(Kind::Row)),
                    "tcPrChange" => (RevisionTarget::Cell, self.container(Kind::Cell)),
                    _ => (RevisionTarget::Text, None),
                };
                if matches!(target, RevisionTarget::Row | RevisionTarget::Cell) {
                    self.row_changed = true;
                }
                if let Some(until) = until {
                    self.open(c, RevisionKind::Formatting, target, until)?;
                }
            }
            Event::Object(Kind::Section, s) => {
                if let Some(c) = tree.child(s, Some(W), "sectPrChange")? {
                    self.open(c, RevisionKind::Formatting, RevisionTarget::Section, c)?;
                    self.close(c);
                }
            }
            Event::Text(t) if !st.in_code() && !st.hidden() => {
                self.marks += 1;
                self.add(t);
            }
            _ => {}
        }
        Ok(())
    }
}

/// Joins revisions that Word reports as one: neighbours of the same kind,
/// author and date with no content between them.
pub(crate) fn merge_revisions(mut done: Vec<(usize, Revision, usize, usize)>) -> Vec<Revision> {
    done.sort_by_key(|d| d.0);
    let mut out: Vec<(Revision, usize)> = vec![];
    for (_, r, start, end) in done {
        if let Some((last, last_end)) = out.last_mut()
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
            && start == *last_end
        {
            last.text.push_str(&r.text);
            if r.target == RevisionTarget::Text {
                last.target = RevisionTarget::Text;
            }
            *last_end = end;
            continue;
        }
        out.push((r, end));
    }
    out.into_iter()
        .enumerate()
        .map(|(i, (mut r, _))| {
            r.index = i;
            r
        })
        .collect()
}

// ---- bookmarks, comment anchors, note references, images ---------------------

/// A reference to a note.
pub(crate) struct NoteRef {
    pub kind: NoteKind,
    pub id: String,
    pub number: Option<usize>,
    pub location: Location,
    /// The mark of a note that has its own.
    pub mark: Option<String>,
}

/// Bookmarks, the text comments are about, where comments and notes are
/// referenced, and pictures.
pub(crate) struct MarksSink<'r> {
    pub c: Common<'r>,
    pub bookmarks: Vec<Bookmark>,
    /// Open bookmarks by id: index and where their text starts.
    open_bookmarks: HashMap<String, (usize, usize)>,
    pub anchors: HashMap<String, String>,
    open_anchors: HashMap<String, usize>,
    spans: Spans,
    pub comment_places: HashMap<String, Location>,
    /// Note references: kind, id, number, place.
    pub note_refs: Vec<NoteRef>,
    pub images: Vec<Image>,
    pub controls: Vec<Control>,
    /// Open content controls: index and where their text starts.
    open_controls: Vec<(usize, usize)>,
    /// Content types of parts, filled by the reader.
    pub content_types: &'r HashMap<String, String>,
}

impl<'r> MarksSink<'r> {
    pub fn new(
        c: Common<'r>,
        content_types: &'r HashMap<String, String>,
        budget: u64,
    ) -> MarksSink<'r> {
        MarksSink {
            c,
            bookmarks: vec![],
            open_bookmarks: HashMap::new(),
            anchors: HashMap::new(),
            open_anchors: HashMap::new(),
            spans: Spans::new(budget),
            controls: vec![],
            open_controls: vec![],
            comment_places: HashMap::new(),
            note_refs: vec![],
            images: vec![],
            content_types,
        }
    }

    /// Ends the bookmarks and comment ranges still open, at the end of a
    /// story.
    pub fn close_all(&mut self) -> Result<(), Error> {
        for (_, (i, start)) in std::mem::take(&mut self.open_bookmarks) {
            self.bookmarks[i].text = self.spans.cut(start)?;
        }
        for (id, start) in std::mem::take(&mut self.open_anchors) {
            let t = self.spans.cut(start)?;
            self.anchors.insert(id, t);
        }
        while let Some((i, start)) = self.open_controls.pop() {
            self.controls[i].text = self.spans.cut(start)?;
        }
        Ok(())
    }
}

impl<'r> Collect<'r> for MarksSink<'r> {
    fn common(&mut self) -> &mut Common<'r> {
        &mut self.c
    }
}

impl Sink for MarksSink<'_> {
    fn event(&mut self, st: &State, ev: Event<'_>) -> Result<(), Error> {
        self.c.track(st, &ev)?;
        let tree = self.c.tree;
        let id = |n| -> Result<String, Error> {
            Ok(tree
                .attribute(n, Some(W), "id")?
                .unwrap_or_default()
                .into_owned())
        };
        match ev {
            Event::Object(Kind::BookmarkStart, n) => {
                let name = tree
                    .attribute(n, Some(W), "name")?
                    .unwrap_or_default()
                    .into_owned();
                let bid = id(n)?;
                self.open_bookmarks
                    .insert(bid.clone(), (self.bookmarks.len(), self.spans.pos()));
                self.bookmarks.push(Bookmark {
                    hidden: name.starts_with('_'),
                    name,
                    id: bid,
                    text: String::new(),
                    location: self.c.location(),
                });
            }
            Event::Object(Kind::BookmarkEnd, n) => {
                if let Some((i, start)) = self.open_bookmarks.remove(&id(n)?) {
                    self.bookmarks[i].text = self.spans.cut(start)?;
                }
            }
            Event::Object(Kind::CommentStart, n) => {
                self.open_anchors.insert(id(n)?, self.spans.pos());
            }
            Event::Object(Kind::CommentEnd, n) => {
                let cid = id(n)?;
                if let Some(start) = self.open_anchors.remove(&cid) {
                    let t = self.spans.cut(start)?;
                    self.anchors.insert(cid, t);
                }
            }
            Event::Start(Kind::ContentControl, n) => {
                let p = props::control_props(tree, n)?;
                self.open_controls
                    .push((self.controls.len(), self.spans.pos()));
                self.controls.push(Control {
                    control: ContentControl {
                        kind: p.kind,
                        tag: p.tag,
                        title: p.title,
                        placeholder: p.placeholder,
                    },
                    text: String::new(),
                    location: self.c.location(),
                });
            }
            Event::End(Kind::ContentControl, _) => {
                if let Some((i, start)) = self.open_controls.pop() {
                    let t = self.spans.cut(start)?;
                    self.controls[i].text = t.strip_suffix('\n').unwrap_or(&t).to_string();
                }
            }
            Event::Object(Kind::CommentReference, n) => {
                self.comment_places.insert(id(n)?, self.c.location());
            }
            Event::Object(Kind::NoteReference, n) => {
                let kind = if tree.is(n, Some(W), "endnoteReference")? {
                    NoteKind::Endnote
                } else {
                    NoteKind::Footnote
                };
                // A note with its own mark has the mark as text after the
                // reference, in the same run.
                let mut mark = None;
                if self.c.note_number.is_none()
                    && let Some(r) = st.run
                {
                    let mut m = String::new();
                    for c in tree.elements(r)? {
                        if tree.is(c, Some(W), "t")? {
                            m.push_str(&tree.text_content(c)?);
                        }
                    }
                    mark = Some(m).filter(|m| !m.is_empty());
                }
                self.note_refs.push(NoteRef {
                    kind,
                    id: id(n)?,
                    number: self.c.note_number,
                    location: self.c.location(),
                    mark,
                });
            }
            Event::Object(Kind::Drawing, n) => {
                let p = props::drawing_props(tree, n)?;
                if p.kind == DrawingKind::Picture {
                    let rel = p.embed.clone().or(p.link.clone());
                    let r = rel.as_ref().and_then(|id| self.c.rels.get(id));
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
                        location: self.c.location(),
                    });
                }
            }
            Event::Text(t) if st.visible(Revisions::Accepted) => self.spans.push(t),
            Event::Object(Kind::Math, n) if st.visible(Revisions::Accepted) => {
                let t = math_text(tree, n)?;
                self.spans.push(&t);
            }
            Event::End(Kind::Paragraph, _) if st.visible(Revisions::Accepted) => {
                self.spans.push("\n")
            }
            _ => {}
        }
        Ok(())
    }
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}
