//! Checks content before anything is written: limits, table grids,
//! labels, math, raw OOXML and the rules of nesting that hold wherever the
//! content goes. Rules that depend on the place (no footnote in a header)
//! are the writer's.
//!
//! Every error names the path of the offending item in the JSON form,
//! such as `blocks[2].rows[0].cells[1]`, whether the content came from
//! JSON or was built in code.

use std::collections::HashSet;

use super::read::ContentLimits;
use super::{Block, Border, Cell, Content, Image, Inline, List, Math, Source, Table};
use crate::xml::Tree;
use crate::{Error, Limits};

const W: &str = crate::ns::W;
const M: &str = "http://schemas.openxmlformats.org/officeDocument/2006/math";

/// The prefixes bound for raw OOXML and math.
pub(crate) const RAW_PREFIXES: &[(&str, &str)] = &[
    ("w", W),
    (
        "r",
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships",
    ),
    ("m", M),
    (
        "wp",
        "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing",
    ),
    ("a", "http://schemas.openxmlformats.org/drawingml/2006/main"),
    (
        "pic",
        "http://schemas.openxmlformats.org/drawingml/2006/picture",
    ),
    (
        "w14",
        "http://schemas.microsoft.com/office/word/2010/wordml",
    ),
    (
        "wps",
        "http://schemas.microsoft.com/office/word/2010/wordprocessingShape",
    ),
    (
        "mc",
        "http://schemas.openxmlformats.org/markup-compatibility/2006",
    ),
];

/// WordprocessingML elements that may stand where a block goes.
pub(crate) const BLOCK_RAW: &[&str] = &[
    "p",
    "tbl",
    "sdt",
    "customXml",
    "bookmarkStart",
    "bookmarkEnd",
    "permStart",
    "permEnd",
    "proofErr",
    "commentRangeStart",
    "commentRangeEnd",
    "moveFromRangeStart",
    "moveFromRangeEnd",
    "moveToRangeStart",
    "moveToRangeEnd",
];

/// WordprocessingML elements that may stand where an inline goes.
pub(crate) const INLINE_RAW: &[&str] = &[
    "r",
    "hyperlink",
    "fldSimple",
    "sdt",
    "customXml",
    "smartTag",
    "bookmarkStart",
    "bookmarkEnd",
    "commentRangeStart",
    "commentRangeEnd",
    "proofErr",
    "permStart",
    "permEnd",
    "ins",
    "del",
];

/// The elements of Office Math (ECMA-376 part 1, 22.1).
const MATH_ELEMENTS: &[&str] = &[
    "acc",
    "accPr",
    "aln",
    "alnScr",
    "argPr",
    "argSz",
    "bar",
    "barPr",
    "baseJc",
    "begChr",
    "borderBox",
    "borderBoxPr",
    "box",
    "boxPr",
    "brk",
    "brkBin",
    "brkBinSub",
    "cGp",
    "cGpRule",
    "chr",
    "count",
    "cSp",
    "ctrlPr",
    "d",
    "defJc",
    "deg",
    "degHide",
    "den",
    "diff",
    "dispDef",
    "dPr",
    "e",
    "endChr",
    "eqArr",
    "eqArrPr",
    "f",
    "fName",
    "fPr",
    "func",
    "funcPr",
    "groupChr",
    "groupChrPr",
    "grow",
    "hideBot",
    "hideLeft",
    "hideRight",
    "hideTop",
    "interSp",
    "intLim",
    "intraSp",
    "jc",
    "lim",
    "limLoc",
    "limLow",
    "limLowPr",
    "limUpp",
    "limUppPr",
    "lit",
    "lMargin",
    "m",
    "mathFont",
    "mathPr",
    "maxDist",
    "mc",
    "mcJc",
    "mcPr",
    "mcs",
    "mPr",
    "mr",
    "nary",
    "naryLim",
    "naryPr",
    "noBreak",
    "nor",
    "num",
    "objDist",
    "oMath",
    "oMathPara",
    "oMathParaPr",
    "opEmu",
    "phant",
    "phantPr",
    "plcHide",
    "pos",
    "postSp",
    "preSp",
    "r",
    "rad",
    "radPr",
    "rMargin",
    "rPr",
    "rSp",
    "rSpRule",
    "scr",
    "sepChr",
    "show",
    "shp",
    "smallFrac",
    "sPre",
    "sPrePr",
    "sSub",
    "sSubPr",
    "sSubSup",
    "sSubSupPr",
    "sSup",
    "sSupPr",
    "strikeBLTR",
    "strikeH",
    "strikeTLBR",
    "strikeV",
    "sty",
    "sub",
    "subHide",
    "sup",
    "supHide",
    "t",
    "transp",
    "type",
    "vertJc",
    "wrapIndent",
    "wrapRight",
    "zeroAsc",
    "zeroDesc",
    "zeroWid",
];

/// The longest label: Word allows bookmark names of 40 characters, and the
/// bookmark of a label is `_Ref_<label>`.
const MAX_LABEL: usize = 35;

/// A label: letters, digits, `_` and `-`, starting with a letter, at most
/// 35 characters.
pub(crate) fn check_label(label: &str) -> Result<(), String> {
    let ok = !label.is_empty()
        && label.chars().count() <= MAX_LABEL
        && label.chars().next().is_some_and(|c| c.is_alphabetic())
        && label
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-');
    if ok {
        Ok(())
    } else {
        Err(format!(
            "{label:?} is not a label; use up to {MAX_LABEL} letters, digits, '_' and '-', starting with a letter"
        ))
    }
}

/// A table on its grid: for each row, each cell's grid column, and the
/// number of columns.
pub(crate) struct Grid {
    pub columns: usize,
    /// For each row, the grid column of each of its cells.
    pub starts: Vec<Vec<usize>>,
    /// For each row and grid column: the row and index of the cell that
    /// covers it.
    pub cover: Vec<Vec<(usize, usize)>>,
}

/// Places the cells on the grid. A ragged table, an overlap or a span past
/// the end is an error that names the row and column, from 0.
pub(crate) fn grid(t: &Table) -> Result<Grid, String> {
    if t.rows.is_empty() {
        return Err("a table needs at least one row".into());
    }
    let n = t.rows.len();
    if t.header_rows > n {
        return Err(format!(
            "header_rows is {}, but the table has {n} rows",
            t.header_rows
        ));
    }
    // The grid positions covered from rows above, row by row.
    let mut cover: Vec<Vec<Option<(usize, usize)>>> = vec![vec![]; n];
    let mut starts = vec![];
    let mut columns: Option<usize> = None;
    for (r, row) in t.rows.iter().enumerate() {
        // A row may list no cells when cells from above cover all of it.
        if row.cells.is_empty() && cover[r].is_empty() {
            return Err(format!("row {r} has no cells"));
        }
        let mut col = 0;
        let mut list = vec![];
        for (k, c) in row.cells.iter().enumerate() {
            crate::cost::tick(1);
            if c.row_span == 0 || c.column_span == 0 {
                return Err(format!("row {r}, cell {k}: a span is at least 1"));
            }
            if r + c.row_span > n {
                return Err(format!(
                    "row {r}, cell {k}: it spans {} rows, past the end of the table",
                    c.row_span
                ));
            }
            while cover[r].get(col).copied().flatten().is_some() {
                col += 1;
            }
            for rr in r..r + c.row_span {
                let line = &mut cover[rr];
                if line.len() < col + c.column_span {
                    line.resize(col + c.column_span, None);
                }
                for (cc, slot) in line.iter_mut().enumerate().skip(col).take(c.column_span) {
                    crate::cost::tick(1);
                    if let Some((or, _)) = slot {
                        return Err(format!(
                            "row {rr}, column {cc}: a cell from row {r} overlaps a cell from row {or}"
                        ));
                    }
                    *slot = Some((r, k));
                }
            }
            list.push(col);
            col += c.column_span;
        }
        let width = cover[r].len();
        if cover[r].iter().any(Option::is_none) {
            let gap = cover[r].iter().position(Option::is_none).unwrap_or(0);
            return Err(format!("row {r}, column {gap}: no cell covers it"));
        }
        match columns {
            None => columns = Some(width),
            Some(w) if w != width => {
                return Err(format!(
                    "row {r}, column {}: the row covers {width} columns, row 0 covers {w}",
                    width.min(w)
                ));
            }
            _ => {}
        }
        starts.push(list);
    }
    let columns = columns.unwrap_or(0);
    if let Some(w) = &t.widths
        && w.len() != columns
    {
        return Err(format!(
            "widths gives {} columns, the table has {columns}",
            w.len()
        ));
    }
    let cover = cover
        .into_iter()
        .map(|l| l.into_iter().map(|s| s.unwrap_or((0, 0))).collect())
        .collect();
    Ok(Grid {
        columns,
        starts,
        cover,
    })
}

/// Checks that math is one `m:oMath` element of Office Math.
pub(crate) fn check_math(m: &Math) -> Result<(), String> {
    let xml = m.omml.trim();
    let (t, roots) = parse_raw(xml)?;
    let [root] = roots.as_slice() else {
        return Err("math is one m:oMath element".into());
    };
    let is = |n| t.is(n, Some(M), "oMath").unwrap_or(false);
    if !is(*root) {
        return Err("math is one m:oMath element".into());
    }
    let mut stack = vec![(*root, false)];
    while let Some((n, in_run)) = stack.pop() {
        let Ok(Some(name)) = t.name(n) else { continue };
        let ns = name.namespace;
        let local = name.local.to_string();
        let run_level = if ns == Some(M) {
            if !MATH_ELEMENTS.contains(&local.as_str()) {
                return Err(format!("m:{local} is not an element of Office Math"));
            }
            in_run || matches!(local.as_str(), "r" | "ctrlPr")
        } else if ns == Some(W) && in_run {
            true
        } else {
            return Err(format!(
                "{} is not an element of Office Math",
                qualified(&name.prefix, &local)
            ));
        };
        for c in t.elements(n).map_err(|e| e.to_string())? {
            stack.push((c, run_level));
        }
    }
    Ok(())
}

fn qualified(prefix: &str, local: &str) -> String {
    if prefix.is_empty() {
        local.to_string()
    } else {
        format!("{prefix}:{local}")
    }
}

/// Parses raw XML with the usual prefixes bound and returns the tree and
/// its top-level elements. Text between them may only be white space.
pub(crate) fn parse_raw(xml: &str) -> Result<(Tree, Vec<crate::xml::NodeId>), String> {
    let limits = Limits::default();
    let mut t = Tree::parse_with_limits("<x/>", &limits).map_err(|e| e.to_string())?;
    let nodes = t
        .parse_fragment(xml, RAW_PREFIXES)
        .map_err(|e| format!("the XML is not well-formed: {e}"))?;
    let mut roots = vec![];
    for n in nodes {
        match t.kind(n).map_err(|e| e.to_string())? {
            crate::xml::NodeKind::Element => roots.push(n),
            crate::xml::NodeKind::Text => {
                let s = t.text(n).map_err(|e| e.to_string())?.unwrap_or_default();
                if !s.trim().is_empty() {
                    return Err("text outside an element".into());
                }
            }
            crate::xml::NodeKind::Comment | crate::xml::NodeKind::ProcessingInstruction => {}
            _ => return Err("only elements, white space and comments".into()),
        }
    }
    Ok((t, roots))
}

/// Checks raw OOXML for a block or an inline position.
pub(crate) fn check_raw(xml: &str, block: bool) -> Result<(), String> {
    let (t, roots) = parse_raw(xml)?;
    if roots.is_empty() {
        return Err("raw OOXML needs at least one element".into());
    }
    let allowed = if block { BLOCK_RAW } else { INLINE_RAW };
    for r in roots {
        let Ok(Some(name)) = t.name(r) else { continue };
        let ok = (name.namespace == Some(W) && allowed.contains(&name.local))
            || (!block && name.namespace == Some(M) && name.local == "oMath")
            || (block && name.namespace == Some(M) && name.local == "oMathPara");
        if !ok {
            let local = name.local.to_string();
            return Err(format!(
                "{} cannot stand where {} goes; use {}",
                qualified(&name.prefix, &local),
                if block { "a block" } else { "an inline" },
                if block {
                    "w:p, w:tbl, w:sdt or another block-level element"
                } else {
                    "w:r, w:hyperlink, w:fldSimple, m:oMath or another run-level element"
                }
            ));
        }
    }
    Ok(())
}

fn check_text(s: &str) -> Result<(), String> {
    match s.chars().find(|&c| {
        !matches!(c, '\t' | '\n' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..)
    }) {
        Some(c) => Err(format!("U+{:04X} cannot stand in a document", c as u32)),
        None => Ok(()),
    }
}

struct Checker<'l> {
    limits: &'l ContentLimits,
    items: usize,
    labels: HashSet<String>,
}

#[derive(Clone, Copy)]
struct Within {
    depth: usize,
    note: bool,
    link: bool,
    cell: bool,
}

pub(super) fn content(c: &Content, limits: &ContentLimits) -> Result<(), Error> {
    let mut k = Checker {
        limits,
        items: 0,
        labels: HashSet::new(),
    };
    let top = Within {
        depth: 0,
        note: false,
        link: false,
        cell: false,
    };
    k.blocks(&c.blocks, "blocks", top)?;
    let mut names = HashSet::new();
    for (i, s) in c.table_styles.iter().enumerate() {
        let p = format!("table_styles[{i}]");
        if s.name.trim().is_empty() || check_text(&s.name).is_err() || s.name.contains('\n') {
            return Err(at(&p, "a table style needs a name of printable characters"));
        }
        if !names.insert(s.name.to_lowercase()) {
            return Err(at(&p, format!("a second table style named {:?}", s.name)));
        }
        if let Some(sz) = s.size
            && !(1.0..=1638.0).contains(&sz)
        {
            return Err(at(
                &p,
                format!("{sz} is not a font size; use 1 to 1638 points"),
            ));
        }
        let b = &s.borders;
        for (side, x) in [
            ("top", &b.top),
            ("bottom", &b.bottom),
            ("left", &b.left),
            ("right", &b.right),
            ("inside_h", &b.inside_h),
            ("inside_v", &b.inside_v),
        ] {
            if let Some(x) = x {
                border(x).map_err(|e| at(&format!("{p}.borders.{side}"), e))?;
            }
        }
        for (k, look) in [("header", &s.header), ("first_column", &s.first_column)] {
            if let Some(l) = look
                && let Some(x) = &l.border
            {
                border(x).map_err(|e| at(&format!("{p}.{k}.border"), e))?;
            }
        }
        if let Some(pad) = &s.padding {
            for v in [pad.top, pad.right, pad.bottom, pad.left] {
                if !(0.0..=100.0).contains(&v) {
                    return Err(at(
                        &format!("{p}.padding"),
                        format!("{v} is not a padding; use 0 to 100 points"),
                    ));
                }
            }
        }
    }
    Ok(())
}

fn at(path: &str, what: impl std::fmt::Display) -> Error {
    Error::Invalid(format!("{path}: {what}"))
}

fn border(b: &Border) -> Result<(), String> {
    if !(0.25..=6.0).contains(&b.width) {
        return Err(format!(
            "{} is not a border width; use 0.25 to 6 points",
            b.width
        ));
    }
    Ok(())
}

impl Checker<'_> {
    fn item(&mut self, p: &str) -> Result<(), Error> {
        self.items += 1;
        if self.items > self.limits.max_items {
            return Err(at(
                p,
                format!(
                    "the content has more than {} blocks, inlines, items, rows and cells; raise ContentLimits::max_items",
                    self.limits.max_items
                ),
            ));
        }
        Ok(())
    }

    fn deeper(&self, w: Within, p: &str) -> Result<Within, Error> {
        if w.depth >= self.limits.max_depth {
            return Err(at(
                p,
                format!(
                    "content nests deeper than {} levels; raise ContentLimits::max_depth",
                    self.limits.max_depth
                ),
            ));
        }
        Ok(Within {
            depth: w.depth + 1,
            ..w
        })
    }

    fn label(&mut self, l: &Option<String>, p: &str) -> Result<(), Error> {
        if let Some(l) = l {
            check_label(l).map_err(|e| at(&format!("{p}.label"), e))?;
            if !self.labels.insert(l.to_lowercase()) {
                return Err(at(
                    &format!("{p}.label"),
                    format!("the label {l:?} is given twice"),
                ));
            }
        }
        Ok(())
    }

    fn blocks(&mut self, list: &[Block], p: &str, w: Within) -> Result<(), Error> {
        for (i, b) in list.iter().enumerate() {
            self.block(b, &format!("{p}[{i}]"), w)?;
        }
        Ok(())
    }

    fn block(&mut self, b: &Block, p: &str, w: Within) -> Result<(), Error> {
        self.item(p)?;
        match b {
            Block::Paragraph { style, content, .. } => {
                if let Some(s) = style
                    && s.trim().is_empty()
                {
                    return Err(at(&format!("{p}.style"), "an empty style name"));
                }
                self.inlines(content, &format!("{p}.content"), w)
            }
            Block::Heading {
                level,
                content,
                label,
            } => {
                if !(1..=9).contains(level) {
                    return Err(at(
                        &format!("{p}.level"),
                        format!("{level} is not a heading level; use 1 to 9"),
                    ));
                }
                self.label(label, p)?;
                self.inlines(content, &format!("{p}.content"), w)
            }
            Block::List(l) => self.list(l, p, w),
            Block::Table(t) => self.table(t, p, w),
            Block::Figure {
                image,
                caption,
                label,
            } => {
                self.image(image, &format!("{p}.image"))?;
                self.label(label, p)?;
                self.inlines(caption, &format!("{p}.caption"), w)
            }
            Block::Image { image } => self.image(image, &format!("{p}.image")),
            Block::Equation { math, label } => {
                check_math(math).map_err(|e| at(&format!("{p}.math.omml"), e))?;
                self.label(label, p)
            }
            Block::PageBreak => {
                if w.cell || w.note {
                    return Err(at(
                        p,
                        format!(
                            "a page break cannot stand in a {}",
                            if w.cell { "table cell" } else { "footnote" }
                        ),
                    ));
                }
                Ok(())
            }
            Block::Raw { xml } => check_raw(xml, true).map_err(|e| at(&format!("{p}.xml"), e)),
        }
    }

    fn list(&mut self, l: &List, p: &str, w: Within) -> Result<(), Error> {
        if l.items.is_empty() {
            return Err(at(&format!("{p}.items"), "a list needs at least one item"));
        }
        for (i, it) in l.items.iter().enumerate() {
            let ip = format!("{p}.items[{i}]");
            self.item(&ip)?;
            self.inlines(&it.content, &format!("{ip}.content"), w)?;
            if let Some(sub) = &it.list {
                let lp = format!("{ip}.list");
                let d = self.deeper(w, &lp)?;
                if d.depth > 9 {
                    return Err(at(&lp, "a list nests at most 9 levels deep"));
                }
                self.list(sub, &lp, d)?;
            }
        }
        Ok(())
    }

    fn table(&mut self, t: &Table, p: &str, w: Within) -> Result<(), Error> {
        grid(t).map_err(|e| at(p, e))?;
        self.label(&t.label, p)?;
        if t.label.is_some() && t.caption.is_none() {
            return Err(at(
                &format!("{p}.label"),
                "a label needs a caption to number",
            ));
        }
        if let Some(s) = &t.style
            && s.trim().is_empty()
        {
            return Err(at(&format!("{p}.style"), "an empty style name"));
        }
        if let Some(c) = &t.caption {
            self.inlines(c, &format!("{p}.caption"), w)?;
        }
        for (r, row) in t.rows.iter().enumerate() {
            let rp = format!("{p}.rows[{r}]");
            self.item(&rp)?;
            for (k, c) in row.cells.iter().enumerate() {
                self.cell(c, &format!("{rp}.cells[{k}]"), w)?;
            }
        }
        Ok(())
    }

    fn cell(&mut self, c: &Cell, p: &str, w: Within) -> Result<(), Error> {
        self.item(p)?;
        if let Some(b) = &c.borders {
            for (side, x) in [
                ("top", &b.top),
                ("bottom", &b.bottom),
                ("left", &b.left),
                ("right", &b.right),
            ] {
                if let Some(x) = x {
                    border(x).map_err(|e| at(&format!("{p}.borders.{side}"), e))?;
                }
            }
        }
        let cp = format!("{p}.content");
        let mut d = self.deeper(w, &cp)?;
        d.cell = true;
        for (i, b) in c.content.iter().enumerate() {
            let bp = format!("{cp}[{i}]");
            if let Block::Heading { .. } | Block::Figure { .. } = b {
                // Allowed: Word puts headings and figures in cells.
            }
            self.block(b, &bp, d)?;
        }
        Ok(())
    }

    fn image(&mut self, i: &Image, p: &str) -> Result<(), Error> {
        for (k, v) in [("width", i.width), ("height", i.height)] {
            if let Some(v) = v
                && !(v > 0.0 && v <= 1584.0)
            {
                return Err(at(
                    &format!("{p}.{k}"),
                    format!("{v} is not a size; use above 0 up to 1584 points"),
                ));
            }
        }
        if let Source::Data(d) = &i.source
            && d.is_empty()
        {
            return Err(at(&format!("{p}.data"), "the picture has no bytes"));
        }
        for (k, v) in [("alt", &i.alt), ("title", &i.title)] {
            if let Some(v) = v {
                check_text(v).map_err(|e| at(&format!("{p}.{k}"), e))?;
            }
        }
        Ok(())
    }

    fn inlines(&mut self, list: &[Inline], p: &str, w: Within) -> Result<(), Error> {
        for (i, x) in list.iter().enumerate() {
            self.inline(x, &format!("{p}[{i}]"), w)?;
        }
        Ok(())
    }

    fn inline(&mut self, x: &Inline, p: &str, w: Within) -> Result<(), Error> {
        self.item(p)?;
        match x {
            Inline::Text { text, marks } => {
                check_text(text).map_err(|e| at(&format!("{p}.text"), e))?;
                if let Some(s) = marks.size
                    && !(1.0..=1638.0).contains(&s)
                {
                    return Err(at(
                        &format!("{p}.marks.size"),
                        format!("{s} is not a font size; use 1 to 1638 points"),
                    ));
                }
                if marks.superscript && marks.subscript {
                    return Err(at(
                        &format!("{p}.marks"),
                        "text is superscript or subscript, not both",
                    ));
                }
                Ok(())
            }
            Inline::Footnote { content } => {
                if w.note {
                    return Err(at(p, "a footnote cannot stand in a footnote"));
                }
                if w.link {
                    return Err(at(p, "a footnote cannot stand in a link"));
                }
                let cp = format!("{p}.content");
                let mut d = self.deeper(w, &cp)?;
                d.note = true;
                d.cell = false;
                if content.is_empty() {
                    return Err(at(&cp, "a footnote needs some content"));
                }
                self.blocks(content, &cp, d)
            }
            Inline::Reference { label } => {
                check_label(label).map_err(|e| at(&format!("{p}.label"), e))
            }
            Inline::Link { target, content } => {
                if w.link {
                    return Err(at(p, "a link cannot stand in a link"));
                }
                match target {
                    super::LinkTarget::Url(u) => {
                        if u.trim().is_empty() || check_text(u).is_err() || u.contains(['\n', '\t'])
                        {
                            return Err(at(
                                &format!("{p}.target.url"),
                                "an address of printable characters",
                            ));
                        }
                    }
                    super::LinkTarget::Bookmark(b) => {
                        if b.trim().is_empty() || check_text(b).is_err() || b.contains(['\n', '\t'])
                        {
                            return Err(at(
                                &format!("{p}.target.bookmark"),
                                "a bookmark name of printable characters",
                            ));
                        }
                    }
                    super::LinkTarget::Label(l) => {
                        check_label(l).map_err(|e| at(&format!("{p}.target.label"), e))?
                    }
                }
                let cp = format!("{p}.content");
                let mut d = self.deeper(w, &cp)?;
                d.link = true;
                self.inlines(content, &cp, d)
            }
            Inline::Math { math } => check_math(math).map_err(|e| at(&format!("{p}.math.omml"), e)),
            Inline::LineBreak => Ok(()),
            Inline::Field { code, result } => {
                if code.trim().is_empty() {
                    return Err(at(&format!("{p}.code"), "a field needs a code"));
                }
                check_text(code).map_err(|e| at(&format!("{p}.code"), e))?;
                check_text(result).map_err(|e| at(&format!("{p}.result"), e))
            }
            Inline::Image { image } => self.image(image, &format!("{p}.image")),
            Inline::Raw { xml } => check_raw(xml, false).map_err(|e| at(&format!("{p}.xml"), e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::{Cell, Row};

    fn table(rows: Vec<Vec<(usize, usize)>>) -> Table {
        Table {
            rows: rows
                .into_iter()
                .map(|r| Row {
                    cells: r
                        .into_iter()
                        .map(|(rs, cs)| Cell {
                            row_span: rs,
                            column_span: cs,
                            ..Cell::new(vec![])
                        })
                        .collect(),
                })
                .collect(),
            header_rows: 0,
            first_column: false,
            style: None,
            widths: None,
            caption: None,
            label: None,
        }
    }

    #[test]
    fn spans_place_cells_on_the_grid() {
        // A cell over two rows pushes the next row's cells right.
        let g = grid(&table(vec![vec![(2, 1), (1, 2)], vec![(1, 1), (1, 1)]])).unwrap();
        assert_eq!(g.columns, 3);
        assert_eq!(g.starts, vec![vec![0, 1], vec![1, 2]]);
    }

    #[test]
    fn ragged_and_overlapping_tables_name_row_and_column() {
        let e = grid(&table(vec![vec![(1, 1), (1, 1)], vec![(1, 1)]]))
            .err()
            .unwrap();
        assert!(e.contains("row 1"), "{e}");
        let e = grid(&table(vec![vec![(1, 1), (2, 1)], vec![(1, 2)]]))
            .err()
            .unwrap();
        assert!(e.contains("row 1, column 1"), "{e}");
        let e = grid(&table(vec![vec![(3, 1)], vec![]])).err().unwrap();
        assert!(e.contains("row 0"), "{e}");
    }

    #[test]
    fn math_is_office_math() {
        let ok = Math::omml("<m:oMath><m:r><m:t>x</m:t></m:r></m:oMath>");
        assert!(check_math(&ok).is_ok());
        let ok = Math::omml("<m:oMath><m:r><w:rPr><w:b/></w:rPr><m:t>x</m:t></m:r></m:oMath>");
        assert!(check_math(&ok).is_ok());
        for bad in [
            "<m:oMath><m:frog/></m:oMath>",
            "<w:p/>",
            "<m:oMath><w:p/></m:oMath>",
            "<m:oMath>",
            "<m:oMath/><m:oMath/>",
        ] {
            assert!(check_math(&Math::omml(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn raw_ooxml_must_fit_its_position() {
        assert!(check_raw("<w:p/>", true).is_ok());
        assert!(check_raw("<w:r><w:t>x</w:t></w:r>", false).is_ok());
        assert!(check_raw("<w:r/>", true).is_err());
        assert!(check_raw("<w:p/>", false).is_err());
        assert!(check_raw("text", true).is_err());
        assert!(check_raw("<w:p>", true).is_err());
        assert!(check_raw("", true).is_err());
    }
}
