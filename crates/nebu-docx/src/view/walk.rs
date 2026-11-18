//! The one walk over a story that every view uses.
//!
//! A story is the content of the body, a header or footer, a note, a
//! comment or a text box. [`walk`] goes through it in document order,
//! records its visible text as [`Pieces`](super::text::Pieces) and tells a
//! [`Sink`] what it meets: block and inline structure, text, field code,
//! and objects such as drawings and note references. It keeps the state
//! that decides what a reader sees: the revisions around the current
//! point, the fields whose code is being read, the tables around it, and
//! whether the current run is hidden. Each piece carries that state, and
//! [`Pieces::render`](super::text::Pieces::render) decides from it.
//!
//! The walk uses no recursion: it keeps its own stack, so a deeply nested
//! document costs memory in proportion to its size and no call stack.

use crate::Error;
use crate::ns;
use crate::xml::{NodeId, Tree};

use super::RevisionKind;
use super::Revisions;
use super::props::child_val;
use super::styles::{Stated, Styles, Toggle, Toggles};
use super::text::{self, Piece, Pieces, What};

pub(crate) const W: &str = ns::W;
pub(crate) const MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";
pub(crate) const M: &str = "http://schemas.openxmlformats.org/officeDocument/2006/math";
pub(crate) const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
pub(crate) const W14: &str = "http://schemas.microsoft.com/office/word/2010/wordml";

/// Namespaces that the Word of today understands, for choosing a branch of
/// `mc:AlternateContent` as Word does.
const UNDERSTOOD: &[&str] = &[
    W,
    M,
    R,
    W14,
    "http://schemas.microsoft.com/office/word/2012/wordml",
    "http://schemas.microsoft.com/office/word/2015/wordml/symex",
    "http://schemas.microsoft.com/office/word/2016/wordml/cid",
    "http://schemas.microsoft.com/office/word/2018/wordml",
    "http://schemas.microsoft.com/office/word/2018/wordml/cex",
    "http://schemas.microsoft.com/office/word/2020/wordml/sdtdatahash",
    "http://schemas.microsoft.com/office/word/2023/wordml/word16du",
    "http://schemas.microsoft.com/office/word/2024/wordml/sdtformatlock",
    "http://schemas.microsoft.com/office/word/2010/wordprocessingShape",
    "http://schemas.microsoft.com/office/word/2010/wordprocessingGroup",
    "http://schemas.microsoft.com/office/word/2010/wordprocessingCanvas",
    "http://schemas.microsoft.com/office/word/2010/wordprocessingInk",
    "http://schemas.microsoft.com/office/word/2010/wordprocessingDrawing",
    "http://schemas.microsoft.com/office/drawing/2010/main",
    "http://schemas.microsoft.com/office/drawing/2014/chartex",
    "http://schemas.microsoft.com/office/drawing/2016/ink",
    "http://schemas.microsoft.com/office/drawing/2017/model3d",
    "http://schemas.openxmlformats.org/drawingml/2006/main",
    "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing",
    "http://schemas.openxmlformats.org/drawingml/2006/picture",
];

/// What an element is to the walk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Paragraph,
    Table,
    Row,
    Cell,
    Run,
    Hyperlink,
    ContentControl,
    SimpleField,
    /// A revision that holds content: `w:ins`, `w:del`, `w:moveFrom`,
    /// `w:moveTo`.
    Change,
    Drawing,
    NoteReference,
    CommentReference,
    BookmarkStart,
    BookmarkEnd,
    CommentStart,
    CommentEnd,
    /// A page or column break; the walk also gives a line end as text.
    Break,
    /// Section properties.
    Section,
    /// A property change (`w:rPrChange`, `w:pPrChange` and the like).
    PropertyChange,
    /// A paragraph mark, row or cell marked as inserted or deleted.
    MarkChange,
    /// An element no view knows. It is reported, and its content is not
    /// walked.
    Opaque,
}

/// What the walk tells its sink.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Event<'a> {
    /// A container begins. Its content follows, then [`Event::End`].
    Start(Kind, NodeId),
    End(Kind, NodeId),
    /// A point object with no content of interest.
    Object(Kind, NodeId),
    /// Text of the current run as the file holds it: `w:t`, `w:delText`,
    /// tabs, breaks as `\n`, hyphens and symbols, deleted and hidden text
    /// too. What a reader sees is decided by the pieces.
    Text(&'a str),
    /// An equation and its text.
    Math(&'a str),
    /// Text of a field code, from `w:instrText`, `w:delInstrText`, or
    /// `w:t` in a code. [`State::code`] holds the code so far.
    Code,
    /// A complex field begins (`fldChar` begin). The field's number is
    /// [`State::field`].
    FieldBegin,
    /// The code of the current field ends and its result begins.
    FieldSeparate,
    /// The current field ends.
    FieldEnd {
        /// Its number.
        index: usize,
        /// Its code, with nested fields as `{ code }`.
        code: &'a str,
    },
    /// A top-level block of the story begins or ends.
    BlockStart(NodeId),
    BlockEnd(NodeId),
}

/// An open revision around the current point.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Change {
    pub kind: RevisionKind,
    pub node: NodeId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Code,
    Result,
}

#[derive(Debug, Clone)]
struct OpenField {
    index: usize,
    phase: Phase,
    /// The code of a `w:fldSimple`. Its element ends it; a `fldChar`
    /// does not.
    simple: Option<String>,
    /// Where the code of a complex field starts and ends in
    /// [`State::codes`].
    code_start: usize,
    code_end: Option<usize>,
}

/// What lasts from one story to the next: the pieces, the count of fields
/// and the count of note references.
#[derive(Debug, Default)]
pub(crate) struct Shared {
    pub pieces: Pieces,
    next_field: usize,
    notes: [usize; 2],
}

/// The walk's state, which a sink may read at every event.
pub(crate) struct State<'s> {
    shared: &'s mut Shared,
    styles: &'s Styles,
    /// Note references in this story are numbered.
    numbered: bool,
    changes: Vec<Change>,
    /// Open revisions of each kind, row and cell marks included: inserted,
    /// deleted, moved away, moved here.
    open: [usize; 4],
    fields: Vec<OpenField>,
    /// Open simple fields among `fields`.
    simple: usize,
    /// The codes of the open complex fields. A field's code is a range of
    /// it, and a nested field's code lies inside the range of the code
    /// that holds it, so nesting costs no copies.
    codes: String,
    in_code: usize,
    toggles: Toggles,
    pub paragraph: Option<NodeId>,
    pub run: Option<NodeId>,
    /// The style of the current paragraph.
    para_style: Option<String>,
    /// The style of each open table, innermost last.
    tables: Vec<Option<String>>,
    /// For each open row: no cell has begun yet, and its `w:gridBefore`.
    rows: Vec<(bool, u16)>,
    /// The number of the last note reference, if it is numbered.
    note_number: Option<usize>,
}

fn slot(k: RevisionKind) -> Option<usize> {
    match k {
        RevisionKind::Insertion => Some(0),
        RevisionKind::Deletion => Some(1),
        RevisionKind::MoveFrom => Some(2),
        RevisionKind::MoveTo => Some(3),
        RevisionKind::Formatting => None,
    }
}

const BITS: [u8; 4] = [
    text::INSERTED,
    text::DELETED,
    text::MOVED_FROM,
    text::MOVED_TO,
];

impl<'s> State<'s> {
    /// True if the revisions around the current point leave it in the
    /// document under this view.
    pub fn shown(&self, view: Revisions) -> bool {
        match view {
            Revisions::Current => true,
            Revisions::Accepted => self.open[1] + self.open[2] == 0,
            Revisions::Rejected => self.open[0] + self.open[3] == 0,
        }
    }

    /// True inside the code of a field.
    pub fn in_code(&self) -> bool {
        self.in_code > 0
    }

    /// The toggle properties of the current run after the style cascade.
    pub fn toggles(&self) -> Toggles {
        self.toggles
    }

    /// The innermost revision around the current point.
    pub fn change(&self) -> Option<Change> {
        self.changes.last().copied()
    }

    /// The number of the innermost open field, counted in the order fields
    /// begin in the story.
    pub fn field(&self) -> Option<usize> {
        self.fields.last().map(|f| f.index)
    }

    /// The number of the field that encloses the innermost one.
    pub fn parent_field(&self) -> Option<usize> {
        let n = self.fields.len();
        (n >= 2).then(|| self.fields[n - 2].index)
    }

    /// The code of the innermost field so far.
    pub fn code(&self) -> &str {
        self.fields
            .last()
            .map(|f| self.code_of(f))
            .unwrap_or_default()
    }

    /// The number of pieces recorded so far: where a range that begins
    /// here begins and one that ends here ends.
    pub fn at(&self) -> usize {
        self.shared.pieces.len()
    }

    /// The number of tables around the current point.
    pub fn depth(&self) -> u16 {
        u16::try_from(self.tables.len()).unwrap_or(u16::MAX)
    }

    /// The number of field codes around the current point.
    pub fn code_depth(&self) -> u16 {
        u16::try_from(self.in_code).unwrap_or(u16::MAX)
    }

    /// The number Word shows for the last note reference, if it has one.
    pub fn note_number(&self) -> Option<usize> {
        self.note_number
    }

    fn code_of<'a>(&'a self, f: &'a OpenField) -> &'a str {
        match &f.simple {
            Some(c) => c,
            None => {
                let end = f.code_end.unwrap_or(self.codes.len());
                self.codes.get(f.code_start..end).unwrap_or_default()
            }
        }
    }

    /// A piece with the state of the current point.
    fn piece(&self, text: bool) -> Piece {
        let mut revisions = 0;
        for (n, bit) in self.open.iter().zip(BITS) {
            if *n > 0 {
                revisions |= bit;
            }
        }
        Piece {
            what: What::Break,
            revisions,
            hidden: text && self.toggles.has(Toggle::Hidden),
            code: self.code_depth(),
            depth: self.depth(),
        }
    }

    fn push(&mut self, what: What) {
        let p = self.piece(false);
        self.shared.pieces.push(what, p);
    }

    /// Records text of the current run and tells the sink.
    fn text(&mut self, s: &str, sink: &mut impl Sink) -> Result<(), Error> {
        let p = self.piece(true);
        self.shared.pieces.push_text(s, p);
        sink.event(self, Event::Text(s))
    }

    fn open_revision(&mut self, k: RevisionKind) {
        if let Some(i) = slot(k) {
            self.open[i] += 1;
        }
    }

    fn close_revision(&mut self, k: RevisionKind) {
        if let Some(i) = slot(k) {
            self.open[i] = self.open[i].saturating_sub(1);
        }
    }

    fn push_change(&mut self, c: Change) {
        self.open_revision(c.kind);
        self.changes.push(c);
    }

    fn pop_change(&mut self) {
        if let Some(c) = self.changes.pop() {
            self.close_revision(c.kind);
        }
    }

    /// Ends the innermost field and tells the sink.
    fn end_field(&mut self, sink: &mut impl Sink) -> Result<(), Error> {
        let Some(f) = self.fields.pop() else {
            return Ok(());
        };
        if f.phase == Phase::Code {
            self.in_code -= 1;
        }
        if f.simple.is_some() {
            self.simple -= 1;
        }
        sink.event(
            self,
            Event::FieldEnd {
                index: f.index,
                code: self.code_of(&f),
            },
        )?;
        if self.fields.is_empty() {
            self.codes.clear();
        } else if self.innermost_in_code() {
            self.codes.push('}');
        }
        Ok(())
    }

    /// True if the innermost field is reading its code.
    fn innermost_in_code(&self) -> bool {
        self.fields.last().is_some_and(|f| f.phase == Phase::Code)
    }

    fn table_style(&self) -> Option<&str> {
        self.tables.last().and_then(|t| t.as_deref())
    }
}

/// Receives the events of a walk.
pub(crate) trait Sink {
    fn event(&mut self, state: &State<'_>, event: Event<'_>) -> Result<(), Error>;
}

/// What to do when leaving an element.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Leave {
    Nothing,
    End(Kind),
    /// End a change and pop it.
    Change,
    /// End a simple field and pop it.
    SimpleField,
    Run,
    Paragraph,
    Table,
    /// End a row, and the revision of its mark.
    Row(Option<RevisionKind>),
    /// End a cell of this span, and the revision of its mark.
    Cell(u16, Option<RevisionKind>),
}

struct Frame {
    node: NodeId,
    next: usize,
    leave: Leave,
    /// A top-level block of the story.
    block: bool,
}

/// What an element of the story is.
enum Role {
    /// Walk its children; nothing to report.
    Transparent,
    /// Report it as a container and walk its children.
    Container(Kind),
    /// Report it as a point object; skip its content.
    Object(Kind),
    /// An equation.
    Math,
    /// Neither report nor walk it.
    Skip,
    /// Walk only this child (a branch of `mc:AlternateContent`, the content
    /// of a content control).
    Only(Option<NodeId>, Kind),
}

/// Walks a story from its root element. `numbered` says whether its note
/// references take numbers, as those of the body and its text boxes do.
pub(crate) fn walk(
    tree: &Tree,
    root: NodeId,
    styles: &Styles,
    numbered: bool,
    shared: &mut Shared,
    sink: &mut impl Sink,
) -> Result<(), Error> {
    let mut st = State {
        shared,
        styles,
        numbered,
        changes: vec![],
        open: [0; 4],
        fields: vec![],
        simple: 0,
        codes: String::new(),
        in_code: 0,
        toggles: Toggles::default(),
        paragraph: None,
        run: None,
        para_style: None,
        tables: vec![],
        rows: vec![],
        note_number: None,
    };
    let mut stack = vec![Frame {
        node: root,
        next: 0,
        leave: Leave::Nothing,
        block: false,
    }];
    while let Some(top) = stack.last_mut() {
        let Some(child) = tree.child_at(top.node, top.next) else {
            let f = stack.pop().expect("the loop holds a frame");
            leave(tree, &mut st, &f, sink)?;
            continue;
        };
        top.next += 1;
        let top_level = stack.len() == 1;
        match tree.kind(child)? {
            crate::xml::NodeKind::Element => {}
            _ => continue,
        }
        let mut role = role(tree, child, st.run.is_some())?;
        if let Role::Only(None, Kind::Opaque) = role {
            // No branch of mc:AlternateContent applies: report it whole.
            role = Role::Object(Kind::Opaque);
        }
        let skip = matches!(role, Role::Skip);
        if top_level && !skip {
            sink.event(&st, Event::BlockStart(child))?;
        }
        let leave_with = match role {
            Role::Skip => None,
            Role::Transparent => Some(Leave::Nothing),
            Role::Object(kind) => {
                object(tree, &mut st, kind, child, sink)?;
                None
            }
            Role::Math => {
                let t = math_text(tree, child)?;
                let p = st.piece(true);
                st.shared.pieces.push_text(&t, p);
                sink.event(&st, Event::Math(&t))?;
                None
            }
            Role::Container(kind) => Some(start(tree, &mut st, kind, child, sink)?),
            Role::Only(only, kind) => {
                if kind == Kind::ContentControl {
                    sink.event(&st, Event::Start(kind, child))?;
                }
                if let Some(o) = only {
                    stack.push(Frame {
                        node: child,
                        next: usize::MAX,
                        leave: if kind == Kind::ContentControl {
                            Leave::End(kind)
                        } else {
                            Leave::Nothing
                        },
                        block: top_level,
                    });
                    stack.push(Frame {
                        node: o,
                        next: 0,
                        leave: Leave::Nothing,
                        block: false,
                    });
                    continue;
                }
                if kind == Kind::ContentControl {
                    sink.event(&st, Event::End(kind, child))?;
                }
                None
            }
        };
        match leave_with {
            Some(leave) => stack.push(Frame {
                node: child,
                next: 0,
                leave,
                block: top_level,
            }),
            None => {
                if top_level && !skip {
                    sink.event(&st, Event::BlockEnd(child))?;
                }
            }
        }
    }
    // Fields left open at the end of the story end there.
    while !st.fields.is_empty() {
        st.end_field(sink)?;
    }
    Ok(())
}

fn leave(tree: &Tree, st: &mut State<'_>, f: &Frame, sink: &mut impl Sink) -> Result<(), Error> {
    match f.leave {
        Leave::Nothing => {}
        Leave::End(kind) => sink.event(st, Event::End(kind, f.node))?,
        Leave::Change => {
            sink.event(st, Event::End(Kind::Change, f.node))?;
            st.pop_change();
        }
        Leave::SimpleField => {
            if st.simple > 0 {
                sink.event(st, Event::End(Kind::SimpleField, f.node))?;
                // Complex fields that began inside the simple field and did
                // not end there end with it.
                while let Some(top) = st.fields.last() {
                    let simple = top.simple.is_some();
                    st.end_field(sink)?;
                    if simple {
                        break;
                    }
                }
            }
        }
        Leave::Run => {
            sink.event(st, Event::End(Kind::Run, f.node))?;
            st.run = None;
            st.toggles = Toggles::default();
        }
        Leave::Paragraph => {
            // The paragraph mark: a change to it, then its piece and the
            // end.
            let mark = paragraph_mark_change(tree, f.node)?;
            if let Some((_, m)) = mark {
                sink.event(st, Event::Object(Kind::MarkChange, m))?;
            }
            let mut p = st.piece(false);
            if let Some((k, _)) = mark
                && let Some(i) = slot(k)
            {
                p.revisions |= BITS[i];
            }
            p.hidden = mark_hidden(tree, f.node, st)?;
            st.shared.pieces.push(What::ParagraphEnd, p);
            sink.event(st, Event::End(Kind::Paragraph, f.node))?;
            st.paragraph = None;
            st.para_style = None;
        }
        Leave::Table => {
            sink.event(st, Event::End(Kind::Table, f.node))?;
            st.tables.pop();
        }
        Leave::Row(mark) => {
            st.push(What::RowEnd);
            sink.event(st, Event::End(Kind::Row, f.node))?;
            st.rows.pop();
            if let Some(k) = mark {
                st.close_revision(k);
            }
        }
        Leave::Cell(span, mark) => {
            sink.event(st, Event::End(Kind::Cell, f.node))?;
            st.push(What::CellEnd { span });
            if let Some(k) = mark {
                st.close_revision(k);
            }
        }
    }
    if f.block {
        sink.event(st, Event::BlockEnd(f.node))?;
    }
    Ok(())
}

/// Whether a paragraph's mark is hidden: its `w:pPr/w:rPr` through the
/// style cascade.
fn mark_hidden(tree: &Tree, p: NodeId, st: &State<'_>) -> Result<bool, Error> {
    let rpr = match tree.child(p, Some(W), "pPr")? {
        Some(ppr) => tree.child(ppr, Some(W), "rPr")?,
        None => None,
    };
    let character = match rpr {
        Some(r) => child_val(tree, r, "rStyle")?,
        None => None,
    };
    let t = st.styles.toggles(
        Stated::read(tree, rpr)?,
        character.as_deref(),
        st.para_style.as_deref(),
        st.table_style(),
    );
    Ok(t.has(Toggle::Hidden))
}

/// The revision of a paragraph mark: `w:pPr/w:rPr/w:ins` and the like.
pub(crate) fn paragraph_mark_change(
    tree: &Tree,
    p: NodeId,
) -> Result<Option<(RevisionKind, NodeId)>, Error> {
    let Some(ppr) = tree.child(p, Some(W), "pPr")? else {
        return Ok(None);
    };
    let Some(rpr) = tree.child(ppr, Some(W), "rPr")? else {
        return Ok(None);
    };
    for c in tree.elements(rpr)? {
        if let Some(k) = change_kind(tree, c)? {
            return Ok(Some((k, c)));
        }
    }
    Ok(None)
}

/// The kind of a revision element that holds content or marks a paragraph,
/// row or cell.
pub(crate) fn change_kind(tree: &Tree, n: NodeId) -> Result<Option<RevisionKind>, Error> {
    let Some(name) = tree.name(n)? else {
        return Ok(None);
    };
    if name.namespace != Some(W) {
        return Ok(None);
    }
    Ok(match name.local {
        "ins" | "cellIns" => Some(RevisionKind::Insertion),
        "del" | "cellDel" => Some(RevisionKind::Deletion),
        "moveFrom" => Some(RevisionKind::MoveFrom),
        "moveTo" => Some(RevisionKind::MoveTo),
        _ => None,
    })
}

/// True if an element of a story's top level is listed as a block.
pub(crate) fn listed(tree: &Tree, n: NodeId) -> Result<bool, Error> {
    Ok(tree.is_element(n)? && !matches!(role(tree, n, false)?, Role::Skip))
}

fn role(tree: &Tree, n: NodeId, in_run: bool) -> Result<Role, Error> {
    let Some(name) = tree.name(n)? else {
        return Ok(Role::Skip);
    };
    let ns = name.namespace.unwrap_or("");
    if ns == MC {
        return Ok(match name.local {
            "AlternateContent" => Role::Only(choose(tree, n)?, Kind::Opaque),
            _ => Role::Skip,
        });
    }
    if ns == M {
        return Ok(match name.local {
            "oMath" | "oMathPara" => Role::Math,
            _ => Role::Object(Kind::Opaque),
        });
    }
    if ns != W {
        return Ok(Role::Object(Kind::Opaque));
    }
    use Kind::*;
    Ok(match name.local {
        // Containers of blocks or inlines with nothing to report.
        "body" | "hdr" | "ftr" | "footnote" | "endnote" | "comment" | "txbxContent"
        | "customXml" | "smartTag" | "dir" | "bdo" | "docPartBody" | "ruby" | "rubyBase"
        | "sdtContent" => Role::Transparent,
        "p" => Role::Container(Paragraph),
        "tbl" => Role::Container(Table),
        "tr" => Role::Container(Row),
        "tc" => Role::Container(Cell),
        "r" => Role::Container(Run),
        "hyperlink" => Role::Container(Hyperlink),
        "fldSimple" => Role::Container(SimpleField),
        "ins" | "del" | "moveFrom" | "moveTo" => Role::Container(Change),
        "sdt" => Role::Only(tree.child(n, Some(W), "sdtContent")?, ContentControl),
        // Inside a run.
        "t" | "delText" | "instrText" | "delInstrText" | "tab" | "ptab" | "br" | "cr"
        | "noBreakHyphen" | "softHyphen" | "sym" | "fldChar"
            if in_run =>
        {
            Role::Object(Opaque)
        }
        "drawing" | "pict" | "object" => Role::Object(Drawing),
        "footnoteReference" | "endnoteReference" => Role::Object(NoteReference),
        "commentReference" => Role::Object(CommentReference),
        "bookmarkStart" => Role::Object(BookmarkStart),
        "bookmarkEnd" => Role::Object(BookmarkEnd),
        "commentRangeStart" => Role::Object(CommentStart),
        "commentRangeEnd" => Role::Object(CommentEnd),
        "sectPr" => Role::Object(Section),
        // Properties and marks with nothing a reader sees.
        "pPr"
        | "rPr"
        | "tblPr"
        | "trPr"
        | "tcPr"
        | "tblGrid"
        | "tblPrEx"
        | "sdtPr"
        | "sdtEndPr"
        | "proofErr"
        | "permStart"
        | "permEnd"
        | "lastRenderedPageBreak"
        | "footnoteRef"
        | "endnoteRef"
        | "annotationRef"
        | "separator"
        | "continuationSeparator"
        | "continuationNotice"
        | "customXmlPr"
        | "smartTagPr"
        | "rubyPr"
        | "rt"
        | "fldData"
        | "moveFromRangeStart"
        | "moveFromRangeEnd"
        | "moveToRangeStart"
        | "moveToRangeEnd"
        | "customXmlInsRangeStart"
        | "customXmlInsRangeEnd"
        | "customXmlDelRangeStart"
        | "customXmlDelRangeEnd"
        | "customXmlMoveFromRangeStart"
        | "customXmlMoveFromRangeEnd"
        | "customXmlMoveToRangeStart"
        | "customXmlMoveToRangeEnd"
        | "noProof" => Role::Skip,
        _ => Role::Object(Opaque),
    })
}

/// The branch of `mc:AlternateContent` that Word takes: the first
/// `mc:Choice` whose required namespaces it understands, else
/// `mc:Fallback`.
pub(crate) fn choose(tree: &Tree, ac: NodeId) -> Result<Option<NodeId>, Error> {
    let mut fallback = None;
    for c in tree.elements(ac)? {
        if tree.is(c, Some(MC), "Choice")? {
            let requires = tree
                .attribute(c, None, "Requires")?
                .unwrap_or_default()
                .into_owned();
            let mut ok = !requires.trim().is_empty();
            for prefix in requires.split_whitespace() {
                match tree.namespace_of_prefix(c, prefix)? {
                    Some(uri) if UNDERSTOOD.contains(&uri) => {}
                    _ => ok = false,
                }
            }
            if ok {
                return Ok(Some(c));
            }
        } else if fallback.is_none() && tree.is(c, Some(MC), "Fallback")? {
            fallback = Some(c);
        }
    }
    Ok(fallback)
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

fn int_val(tree: &Tree, parent: Option<NodeId>, local: &str) -> Result<usize, Error> {
    let Some(p) = parent else { return Ok(0) };
    Ok(child_val(tree, p, local)?
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0))
}

/// The revision mark among the children of a row's or cell's properties.
fn mark_of(tree: &Tree, pr: Option<NodeId>) -> Result<Option<RevisionKind>, Error> {
    let Some(pr) = pr else { return Ok(None) };
    for c in tree.elements(pr)? {
        if let Some(k) = change_kind(tree, c)? {
            return Ok(Some(k));
        }
    }
    Ok(None)
}

/// More grid positions per cell than this is not a table a person made.
pub(crate) const MAX_SPAN: usize = 1024;

fn start(
    tree: &Tree,
    st: &mut State<'_>,
    kind: Kind,
    n: NodeId,
    sink: &mut impl Sink,
) -> Result<Leave, Error> {
    Ok(match kind {
        Kind::Change => {
            let k = change_kind(tree, n)?.unwrap_or(RevisionKind::Insertion);
            st.push_change(Change { kind: k, node: n });
            sink.event(st, Event::Start(Kind::Change, n))?;
            Leave::Change
        }
        Kind::SimpleField => {
            let index = st.shared.next_field;
            st.shared.next_field += 1;
            let code = tree
                .attribute(n, Some(W), "instr")?
                .unwrap_or_default()
                .into_owned();
            // Nested in a code, it shows there as `{code}`; the brace that
            // closes it comes when it ends.
            if st.innermost_in_code() {
                st.codes.push('{');
                st.codes.push_str(&code);
            }
            st.fields.push(OpenField {
                index,
                phase: Phase::Result,
                simple: Some(code),
                code_start: 0,
                code_end: None,
            });
            st.simple += 1;
            sink.event(st, Event::Start(Kind::SimpleField, n))?;
            Leave::SimpleField
        }
        Kind::Run => {
            st.run = Some(n);
            let rpr = tree.child(n, Some(W), "rPr")?;
            let character = match rpr {
                Some(r) => child_val(tree, r, "rStyle")?,
                None => None,
            };
            st.toggles = st.styles.toggles(
                Stated::read(tree, rpr)?,
                character.as_deref(),
                st.para_style.as_deref(),
                st.table_style(),
            );
            sink.event(st, Event::Start(Kind::Run, n))?;
            if let Some(rpr) = rpr
                && let Some(c) = tree.child(rpr, Some(W), "rPrChange")?
            {
                sink.event(st, Event::Object(Kind::PropertyChange, c))?;
            }
            Leave::Run
        }
        Kind::Paragraph => {
            st.paragraph = Some(n);
            st.para_style = super::props::paragraph_style(tree, n)?;
            sink.event(st, Event::Start(Kind::Paragraph, n))?;
            if let Some(ppr) = tree.child(n, Some(W), "pPr")? {
                if let Some(c) = tree.child(ppr, Some(W), "pPrChange")? {
                    sink.event(st, Event::Object(Kind::PropertyChange, c))?;
                }
                if let Some(s) = tree.child(ppr, Some(W), "sectPr")? {
                    sink.event(st, Event::Object(Kind::Section, s))?;
                }
            }
            Leave::Paragraph
        }
        Kind::Row => {
            let pr = tree.child(n, Some(W), "trPr")?;
            let before = int_val(tree, pr, "gridBefore")?.min(MAX_SPAN) as u16;
            st.rows.push((true, before));
            sink.event(st, Event::Start(Kind::Row, n))?;
            if let Some(pr) = pr {
                for c in tree.elements(pr)? {
                    if change_kind(tree, c)?.is_some() {
                        sink.event(st, Event::Object(Kind::MarkChange, c))?;
                    } else if tree.is(c, Some(W), "trPrChange")? {
                        sink.event(st, Event::Object(Kind::PropertyChange, c))?;
                    }
                }
            }
            // A row inserted or deleted takes its content with it.
            let mark = mark_of(tree, pr)?;
            if let Some(k) = mark {
                st.open_revision(k);
            }
            Leave::Row(mark)
        }
        Kind::Cell => {
            let pr = tree.child(n, Some(W), "tcPr")?;
            let span = int_val(tree, pr, "gridSpan")?.clamp(1, MAX_SPAN) as u16;
            let tabs = match st.rows.last_mut() {
                Some((first @ true, before)) => {
                    *first = false;
                    *before
                }
                _ => 1,
            };
            let mark = mark_of(tree, pr)?;
            if let Some(k) = mark {
                st.open_revision(k);
            }
            st.push(What::CellStart { tabs });
            sink.event(st, Event::Start(Kind::Cell, n))?;
            if let Some(pr) = pr {
                for c in tree.elements(pr)? {
                    if change_kind(tree, c)?.is_some() {
                        sink.event(st, Event::Object(Kind::MarkChange, c))?;
                    } else if tree.is(c, Some(W), "tcPrChange")? {
                        sink.event(st, Event::Object(Kind::PropertyChange, c))?;
                    }
                }
            }
            Leave::Cell(span, mark)
        }
        Kind::Table => {
            sink.event(st, Event::Start(Kind::Table, n))?;
            let pr = tree.child(n, Some(W), "tblPr")?;
            st.tables.push(match pr {
                Some(p) => child_val(tree, p, "tblStyle")?,
                None => None,
            });
            if let Some(pr) = pr
                && let Some(c) = tree.child(pr, Some(W), "tblPrChange")?
            {
                sink.event(st, Event::Object(Kind::PropertyChange, c))?;
            }
            Leave::Table
        }
        other => {
            sink.event(st, Event::Start(other, n))?;
            Leave::End(other)
        }
    })
}

/// A point object. Run content arrives here as `Opaque` and is sorted out.
fn object(
    tree: &Tree,
    st: &mut State<'_>,
    kind: Kind,
    n: NodeId,
    sink: &mut impl Sink,
) -> Result<(), Error> {
    if kind == Kind::NoteReference {
        let custom = tree
            .attribute(n, Some(W), "customMarkFollows")?
            .is_some_and(|v| matches!(&*v, "1" | "true" | "on"));
        let endnote = tree.is(n, Some(W), "endnoteReference")?;
        // A reference in deleted text or a field code is not counted.
        st.note_number = if custom || !st.numbered || !st.shown(Revisions::Accepted) || st.in_code()
        {
            None
        } else {
            let c = &mut st.shared.notes[usize::from(endnote)];
            *c += 1;
            Some(*c)
        };
        if let Some(number) = st.note_number {
            let p = st.piece(true);
            st.shared.pieces.push(What::NoteMark { endnote, number }, p);
        }
        return sink.event(st, Event::Object(kind, n));
    }
    if kind != Kind::Opaque
        || st.run.is_none()
        || tree.name(n)?.map(|m| m.namespace) != Some(Some(W))
    {
        return sink.event(st, Event::Object(kind, n));
    }
    let local = tree.name(n)?.map(|m| m.local).unwrap_or_default();
    match local {
        "t" | "delText" => {
            let mut s = String::new();
            for c in tree.children(n)? {
                if let Some(t) = tree.text(c)? {
                    s.push_str(&t);
                }
            }
            if !s.is_empty() {
                // Text in a code is code; the result of a field nested in a
                // code is neither code nor visible.
                if st.innermost_in_code() {
                    st.codes.push_str(&s);
                    sink.event(st, Event::Code)?;
                } else {
                    st.text(&s, sink)?;
                }
            }
        }
        "instrText" | "delInstrText" => {
            let s = tree.text_content(n)?;
            // Word writes the result of a simple field nested in a code as
            // instrText inside the simple field.
            if st.fields.last().is_some_and(|f| f.simple.is_some()) {
                st.text(&s, sink)?;
            } else {
                if st.innermost_in_code() {
                    st.codes.push_str(&s);
                }
                sink.event(st, Event::Code)?;
            }
        }
        "tab" | "ptab" => st.text("\t", sink)?,
        "cr" | "br" => {
            let t = tree.attribute(n, Some(W), "type")?;
            if local == "br" && matches!(t.as_deref(), Some("page") | Some("column")) {
                sink.event(st, Event::Object(Kind::Break, n))?;
            }
            let p = st.piece(true);
            st.shared.pieces.push(What::Break, p);
            sink.event(st, Event::Text("\n"))?;
        }
        "noBreakHyphen" => st.text("\u{2011}", sink)?,
        "softHyphen" => st.text("\u{AD}", sink)?,
        "sym" => {
            // The character as the symbol font numbers it, often in the
            // private use area from U+F000; there is no general way to
            // know the Unicode character the glyph shows.
            let c = tree
                .attribute(n, Some(W), "char")?
                .and_then(|v| u32::from_str_radix(&v, 16).ok())
                .and_then(char::from_u32)
                .filter(|c| !c.is_control());
            if let Some(c) = c {
                let mut b = [0; 4];
                st.text(c.encode_utf8(&mut b), sink)?;
            }
        }
        "fldChar" => match tree.attribute(n, Some(W), "fldCharType")?.as_deref() {
            Some("begin") => {
                let index = st.shared.next_field;
                st.shared.next_field += 1;
                if st.innermost_in_code() {
                    st.codes.push('{');
                }
                st.fields.push(OpenField {
                    index,
                    phase: Phase::Code,
                    simple: None,
                    code_start: st.codes.len(),
                    code_end: None,
                });
                st.in_code += 1;
                sink.event(st, Event::FieldBegin)?;
            }
            Some("separate") => {
                let end = st.codes.len();
                if let Some(f) = st.fields.last_mut()
                    && f.simple.is_none()
                    && f.phase == Phase::Code
                {
                    f.phase = Phase::Result;
                    f.code_end = Some(end);
                    st.in_code -= 1;
                    sink.event(st, Event::FieldSeparate)?;
                }
            }
            Some("end") if st.fields.last().is_some_and(|f| f.simple.is_none()) => {
                st.end_field(sink)?;
            }
            _ => {}
        },
        _ => sink.event(st, Event::Object(Kind::Opaque, n))?,
    }
    Ok(())
}
