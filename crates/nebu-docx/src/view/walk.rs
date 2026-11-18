//! The one walk over a story that every view uses.
//!
//! A story is the content of the body, a header or footer, a note, a
//! comment or a text box. [`walk`] goes through it in document order and
//! tells a [`Sink`] what it meets: block and inline structure, text, field
//! code, and objects such as drawings and note references. It keeps the
//! state that decides what a reader sees: the revisions around the current
//! point, the fields whose code is being read, and whether the current run
//! is hidden. [`State::visible`] is the one definition of visible text.
//!
//! The walk uses no recursion: it keeps its own stack, so a deeply nested
//! document costs memory in proportion to its size and no call stack.

use crate::Error;
use crate::ns;
use crate::xml::{NodeId, Tree};

use super::styles::Styles;
use super::{RevisionKind, Revisions};

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
    Math,
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
    /// Text of the current run: `w:t`, `w:delText`, tabs, breaks, hyphens
    /// and symbols. Whether a reader sees it is [`State::visible`].
    Text(&'a str),
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
    /// `w:fldSimple`, closed by its element rather than a `fldChar`.
    simple: bool,
    code: String,
}

/// The walk's state, which a sink may read at every event.
pub(crate) struct State {
    changes: Vec<Change>,
    fields: Vec<OpenField>,
    in_code: usize,
    next_field: usize,
    hidden: bool,
    pub paragraph: Option<NodeId>,
    pub run: Option<NodeId>,
    /// The style of the current paragraph, for hidden text.
    para_style: Option<String>,
}

impl State {
    /// True if a reader sees the current text under this view of the
    /// revisions: not in a field code, not hidden, and not removed by the
    /// revisions around it.
    pub fn visible(&self, view: Revisions) -> bool {
        self.in_code == 0 && !self.hidden && self.shown(view)
    }

    /// True if the revisions around the current point leave it in the
    /// document under this view.
    pub fn shown(&self, view: Revisions) -> bool {
        match view {
            Revisions::Current => true,
            Revisions::Accepted => !self
                .changes
                .iter()
                .any(|c| matches!(c.kind, RevisionKind::Deletion | RevisionKind::MoveFrom)),
            Revisions::Rejected => !self
                .changes
                .iter()
                .any(|c| matches!(c.kind, RevisionKind::Insertion | RevisionKind::MoveTo)),
        }
    }

    /// True inside the code of a field.
    pub fn in_code(&self) -> bool {
        self.in_code > 0
    }

    /// True if the current run is hidden text.
    pub fn hidden(&self) -> bool {
        self.hidden
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
            .map(|f| f.code.as_str())
            .unwrap_or_default()
    }

    /// Adds code text to the innermost field and to each enclosing field
    /// whose code holds it.
    fn add_code(&mut self, text: &str) {
        let mut it = self.fields.iter_mut().rev();
        if let Some(f) = it.next() {
            f.code.push_str(text);
        }
        for f in it {
            if f.phase != Phase::Code {
                break;
            }
            f.code.push_str(text);
        }
    }

    /// Adds text to the code of the fields enclosing the innermost one, as
    /// long as they are reading code.
    fn add_outer_code(&mut self, text: &str) {
        let n = self.fields.len();
        if n < 2 {
            return;
        }
        for f in self.fields[..n - 1].iter_mut().rev() {
            if f.phase != Phase::Code {
                break;
            }
            f.code.push_str(text);
        }
    }

    /// True if the innermost field is reading its code.
    fn innermost_in_code(&self) -> bool {
        self.fields.last().is_some_and(|f| f.phase == Phase::Code)
    }

    /// Numbers fields from this one on, so that walks of several stories
    /// number fields in one sequence.
    pub fn start_fields_at(&mut self, n: usize) {
        self.next_field = n;
    }

    fn new() -> State {
        State {
            changes: vec![],
            fields: vec![],
            in_code: 0,
            next_field: 0,
            hidden: false,
            paragraph: None,
            run: None,
            para_style: None,
        }
    }
}

/// Receives the events of a walk.
pub(crate) trait Sink {
    fn event(&mut self, state: &State, event: Event<'_>) -> Result<(), Error>;
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
    /// End a run.
    Run,
    /// End a paragraph.
    Paragraph,
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
    /// Neither report nor walk it.
    Skip,
    /// Walk only this child (a branch of `mc:AlternateContent`, the content
    /// of a content control).
    Only(Option<NodeId>, Kind),
}

/// Walks a story from its root element and returns the number of fields
/// it saw, counting from `first_field`.
pub(crate) fn walk(
    tree: &Tree,
    root: NodeId,
    styles: &Styles,
    first_field: usize,
    sink: &mut impl Sink,
) -> Result<usize, Error> {
    let mut st = State::new();
    st.start_fields_at(first_field);
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
        let mut role = role(tree, child, &st)?;
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
            Role::Container(kind) => Some(start(tree, &mut st, styles, kind, child, sink)?),
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
    while let Some(f) = st.fields.pop() {
        if f.phase == Phase::Code {
            st.in_code -= 1;
        }
        sink.event(
            &st,
            Event::FieldEnd {
                index: f.index,
                code: &f.code,
            },
        )?;
    }
    Ok(st.next_field)
}

fn leave(tree: &Tree, st: &mut State, f: &Frame, sink: &mut impl Sink) -> Result<(), Error> {
    match f.leave {
        Leave::Nothing => {}
        Leave::End(kind) => sink.event(st, Event::End(kind, f.node))?,
        Leave::Change => {
            sink.event(st, Event::End(Kind::Change, f.node))?;
            st.changes.pop();
        }
        Leave::SimpleField => {
            if let Some(i) = st.fields.iter().rposition(|o| o.simple) {
                sink.event(st, Event::End(Kind::SimpleField, f.node))?;
                let o = st.fields.remove(i);
                if o.phase == Phase::Code {
                    st.in_code -= 1;
                }
                sink.event(
                    st,
                    Event::FieldEnd {
                        index: o.index,
                        code: &o.code,
                    },
                )?;
            }
        }
        Leave::Run => {
            sink.event(st, Event::End(Kind::Run, f.node))?;
            st.run = None;
            st.hidden = false;
        }
        Leave::Paragraph => {
            // The paragraph mark: a change to it, then the end.
            if let Some((_, mark)) = paragraph_mark_change(tree, f.node)? {
                sink.event(st, Event::Object(Kind::MarkChange, mark))?;
            }
            sink.event(st, Event::End(Kind::Paragraph, f.node))?;
            st.paragraph = None;
            st.para_style = None;
        }
    }
    if f.block {
        sink.event(st, Event::BlockEnd(f.node))?;
    }
    Ok(())
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
    Ok(tree.is_element(n)? && !matches!(role(tree, n, &State::new())?, Role::Skip))
}

fn role(tree: &Tree, n: NodeId, st: &State) -> Result<Role, Error> {
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
            "oMath" | "oMathPara" => Role::Object(Kind::Math),
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
            if st.run.is_some() =>
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
fn choose(tree: &Tree, ac: NodeId) -> Result<Option<NodeId>, Error> {
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

fn start(
    tree: &Tree,
    st: &mut State,
    styles: &Styles,
    kind: Kind,
    n: NodeId,
    sink: &mut impl Sink,
) -> Result<Leave, Error> {
    Ok(match kind {
        Kind::Change => {
            let k = change_kind(tree, n)?.unwrap_or(RevisionKind::Insertion);
            st.changes.push(Change { kind: k, node: n });
            sink.event(st, Event::Start(Kind::Change, n))?;
            Leave::Change
        }
        Kind::SimpleField => {
            let index = st.next_field;
            st.next_field += 1;
            let code = tree
                .attribute(n, Some(W), "instr")?
                .unwrap_or_default()
                .into_owned();
            let nested = format!("{{{code}}}");
            st.fields.push(OpenField {
                index,
                phase: Phase::Result,
                simple: true,
                code,
            });
            st.add_outer_code(&nested);
            sink.event(st, Event::Start(Kind::SimpleField, n))?;
            Leave::SimpleField
        }
        Kind::Run => {
            st.run = Some(n);
            st.hidden = run_hidden(tree, n, styles, st.para_style.as_deref())?;
            sink.event(st, Event::Start(Kind::Run, n))?;
            if let Some(rpr) = tree.child(n, Some(W), "rPr")?
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
            sink.event(st, Event::Start(Kind::Row, n))?;
            if let Some(pr) = tree.child(n, Some(W), "trPr")? {
                for c in tree.elements(pr)? {
                    if change_kind(tree, c)?.is_some() {
                        sink.event(st, Event::Object(Kind::MarkChange, c))?;
                    } else if tree.is(c, Some(W), "trPrChange")? {
                        sink.event(st, Event::Object(Kind::PropertyChange, c))?;
                    }
                }
            }
            Leave::End(Kind::Row)
        }
        Kind::Cell => {
            sink.event(st, Event::Start(Kind::Cell, n))?;
            if let Some(pr) = tree.child(n, Some(W), "tcPr")? {
                for c in tree.elements(pr)? {
                    if change_kind(tree, c)?.is_some() {
                        sink.event(st, Event::Object(Kind::MarkChange, c))?;
                    } else if tree.is(c, Some(W), "tcPrChange")? {
                        sink.event(st, Event::Object(Kind::PropertyChange, c))?;
                    }
                }
            }
            Leave::End(Kind::Cell)
        }
        Kind::Table => {
            sink.event(st, Event::Start(Kind::Table, n))?;
            if let Some(pr) = tree.child(n, Some(W), "tblPr")?
                && let Some(c) = tree.child(pr, Some(W), "tblPrChange")?
            {
                sink.event(st, Event::Object(Kind::PropertyChange, c))?;
            }
            Leave::End(Kind::Table)
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
    st: &mut State,
    kind: Kind,
    n: NodeId,
    sink: &mut impl Sink,
) -> Result<(), Error> {
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
                    st.add_code(&s);
                    sink.event(st, Event::Code)?;
                } else {
                    sink.event(st, Event::Text(&s))?;
                }
            }
        }
        "instrText" | "delInstrText" => {
            let s = tree.text_content(n)?;
            // Word writes the result of a simple field nested in a code as
            // instrText inside the simple field.
            if st.fields.last().is_some_and(|f| f.simple) {
                sink.event(st, Event::Text(&s))?;
            } else {
                if !st.fields.is_empty() {
                    st.add_code(&s);
                }
                sink.event(st, Event::Code)?;
            }
        }
        "tab" | "ptab" => sink.event(st, Event::Text("\t"))?,
        "cr" => sink.event(st, Event::Text("\n"))?,
        "br" => {
            let t = tree.attribute(n, Some(W), "type")?;
            if matches!(t.as_deref(), Some("page") | Some("column")) {
                sink.event(st, Event::Object(Kind::Break, n))?;
            }
            sink.event(st, Event::Text("\n"))?;
        }
        "noBreakHyphen" => sink.event(st, Event::Text("\u{2011}"))?,
        "softHyphen" => sink.event(st, Event::Text("\u{AD}"))?,
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
                sink.event(st, Event::Text(c.encode_utf8(&mut b)))?;
            }
        }
        "fldChar" => match tree.attribute(n, Some(W), "fldCharType")?.as_deref() {
            Some("begin") => {
                let index = st.next_field;
                st.next_field += 1;
                if st.innermost_in_code() {
                    st.add_code("{");
                }
                st.fields.push(OpenField {
                    index,
                    phase: Phase::Code,
                    simple: false,
                    code: String::new(),
                });
                st.in_code += 1;
                sink.event(st, Event::FieldBegin)?;
            }
            Some("separate") => {
                if let Some(f) = st.fields.last_mut()
                    && !f.simple
                    && f.phase == Phase::Code
                {
                    f.phase = Phase::Result;
                    st.in_code -= 1;
                    sink.event(st, Event::FieldSeparate)?;
                }
            }
            Some("end") => {
                if let Some(f) = st.fields.pop_if(|f| !f.simple) {
                    if f.phase == Phase::Code {
                        st.in_code -= 1;
                    }
                    if st.innermost_in_code() {
                        st.add_code("}");
                    }
                    sink.event(
                        st,
                        Event::FieldEnd {
                            index: f.index,
                            code: &f.code,
                        },
                    )?;
                }
            }
            _ => {}
        },
        _ => sink.event(st, Event::Object(Kind::Opaque, n))?,
    }
    Ok(())
}

/// Whether a run is hidden: its own `w:vanish`, else its character style,
/// else the paragraph's style, else the document defaults.
fn run_hidden(tree: &Tree, r: NodeId, styles: &Styles, para: Option<&str>) -> Result<bool, Error> {
    let mut char_style = None;
    if let Some(rpr) = tree.child(r, Some(W), "rPr")? {
        if let Some(v) = tree.child(rpr, Some(W), "vanish")? {
            return super::props::on_off(tree, v);
        }
        if let Some(s) = tree.child(rpr, Some(W), "rStyle")? {
            char_style = tree.attribute(s, Some(W), "val")?.map(|v| v.into_owned());
        }
    }
    Ok(styles.hidden(char_style.as_deref(), para))
}
