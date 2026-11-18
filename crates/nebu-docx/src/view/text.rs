//! Visible text: the pieces the walk records and the one function that
//! turns a range of them into a string.
//!
//! The walk ([`super::walk`]) records every story as a sequence of
//! [`Piece`]s: runs of text and the separators between them (paragraph
//! ends, cell and row boundaries, breaks), each tagged with what decides
//! whether a reader sees it. [`Pieces::render`] is the only place that
//! decides that. Every text a view gives (a block, a cell, a bookmark, a
//! note, a comment, a revision, a field result, the whole document) is a
//! [`Span`] of the pieces rendered by it.

use super::Revisions;

/// The revision marks around a piece, as bits.
pub(crate) const INSERTED: u8 = 1;
pub(crate) const DELETED: u8 = 2;
pub(crate) const MOVED_FROM: u8 = 4;
pub(crate) const MOVED_TO: u8 = 8;

/// What a piece is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum What {
    /// Text of a run, `start..end` of [`Pieces::text`]: `w:t`, tabs,
    /// symbols, hyphens, the letters of an equation.
    Text { start: usize, end: usize },
    /// A line, page or column break, or a carriage return.
    Break,
    /// The mark at the end of a paragraph.
    ParagraphEnd,
    /// A cell begins after `tabs` grid positions: 0 for the first cell of a
    /// row, more for a row with `w:gridBefore`.
    CellStart { tabs: u16 },
    /// A cell that covers `span` grid columns ends.
    CellEnd { span: u16 },
    /// A table row ends.
    RowEnd,
    /// The mark of a footnote or endnote reference with the number Word
    /// shows.
    NoteMark { endnote: bool, number: usize },
}

/// One piece of a story.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Piece {
    pub what: What,
    /// The revisions around it: [`INSERTED`], [`DELETED`], [`MOVED_FROM`],
    /// [`MOVED_TO`]. For a paragraph end, a row or a cell, the marks on it
    /// too.
    pub revisions: u8,
    /// Hidden text (`w:vanish` through the style cascade); for a paragraph
    /// end, a hidden paragraph mark.
    pub hidden: bool,
    /// The number of field codes it is in.
    pub code: u16,
    /// The number of tables it is in.
    pub depth: u16,
}

/// Every story of a document as pieces, in the order they were walked.
#[derive(Debug, Default, Clone)]
pub(crate) struct Pieces {
    text: String,
    items: Vec<Piece>,
}

/// A range of pieces and where it begins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Span {
    pub start: usize,
    pub end: usize,
    /// The number of tables around the point where the range begins.
    pub depth: u16,
    /// The number of field codes around the point where the range begins,
    /// for the result of a field nested in a code; 0 otherwise.
    pub code: u16,
    /// Only the pieces of the range's own level, for a cell without the
    /// tables nested in it.
    pub own_level: bool,
}

/// What a rendering shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Show {
    pub revisions: Revisions,
    /// Note references as `[1]`, endnotes as `[i]`.
    pub note_marks: bool,
}

impl Show {
    /// What the views show: revisions accepted, no note marks.
    pub const VIEWS: Show = Show {
        revisions: Revisions::Accepted,
        note_marks: false,
    };
    /// Everything the file holds, for the text of a revision.
    pub const ALL: Show = Show {
        revisions: Revisions::Current,
        note_marks: false,
    };
}

impl Pieces {
    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn push(&mut self, what: What, mut piece: Piece) {
        piece.what = what;
        self.items.push(piece);
    }

    pub fn push_text(&mut self, s: &str, piece: Piece) {
        if s.is_empty() {
            return;
        }
        let start = self.text.len();
        self.text.push_str(s);
        let end = self.text.len();
        self.push(What::Text { start, end }, piece);
    }

    /// A span from `start` to here.
    pub fn span_from(&self, start: usize, depth: u16) -> Span {
        Span {
            start,
            end: self.len(),
            depth,
            code: 0,
            own_level: false,
        }
    }

    /// The text a reader sees in a span.
    ///
    /// A piece is seen if the revisions view keeps it and, for text, breaks
    /// and note marks, if it is not hidden and not in a field code; a
    /// paragraph end is seen unless a revision removes it or its mark is
    /// hidden. A span is read from the outermost table level it touches:
    /// there a paragraph or a break ends a line, the cells of a table one
    /// level down are separated by tabs (one more for each grid column a
    /// cell spans and each column a row skips) and its rows end lines, and
    /// paragraphs and breaks inside one of its cells, and everything deeper,
    /// are joined by single spaces.
    pub fn render(&self, span: Span, show: Show) -> String {
        let items = self.items.get(span.start..span.end).unwrap_or_default();
        let base = items.iter().map(|p| p.depth).fold(span.depth, u16::min);
        let mut out = Out::default();
        for p in items {
            let level = p.depth - base;
            if span.own_level && level > 0 {
                continue;
            }
            if !self.seen(p, span, show) {
                continue;
            }
            match p.what {
                What::Text { start, end } => {
                    out.write(self.text.get(start..end).unwrap_or_default(), level);
                }
                What::NoteMark { endnote, number } => {
                    if show.note_marks {
                        let mark = if endnote {
                            format!("[{}]", roman(number))
                        } else {
                            format!("[{number}]")
                        };
                        out.write(&mark, level);
                    }
                }
                What::Break | What::ParagraphEnd => {
                    if level == 0 {
                        out.lines += 1;
                    } else if out.wrote {
                        out.space = true;
                    }
                }
                What::CellStart { tabs } => {
                    if level <= 1 {
                        out.space = false;
                        out.flush_lines();
                        out.wrote = false;
                        for _ in 0..tabs {
                            out.s.push('\t');
                        }
                    } else if tabs > 0 && out.wrote {
                        out.space = true;
                    }
                }
                What::CellEnd { span } => {
                    if level <= 1 {
                        out.lines = 0;
                        out.space = false;
                        for _ in 1..span {
                            out.s.push('\t');
                        }
                    }
                }
                What::RowEnd => {
                    if level <= 1 {
                        out.lines = 1;
                        out.space = false;
                        out.wrote = false;
                    } else if out.wrote {
                        out.space = true;
                    }
                }
            }
        }
        out.flush_lines();
        out.s
    }

    /// [`render`](Self::render) without the line ends at the end: the text
    /// of an item, such as a block, a cell or a bookmark.
    pub fn item_text(&self, span: Span, show: Show) -> String {
        let mut s = self.render(span, show);
        let keep = s.trim_end_matches('\n').len();
        s.truncate(keep);
        s
    }

    fn seen(&self, p: &Piece, span: Span, show: Show) -> bool {
        let kept = match show.revisions {
            Revisions::Current => true,
            Revisions::Accepted => p.revisions & (DELETED | MOVED_FROM) == 0,
            Revisions::Rejected => p.revisions & (INSERTED | MOVED_TO) == 0,
        };
        kept && match p.what {
            What::Text { .. } | What::Break | What::NoteMark { .. } => {
                !p.hidden && p.code <= span.code
            }
            What::ParagraphEnd => !p.hidden,
            What::CellStart { .. } | What::CellEnd { .. } | What::RowEnd => true,
        }
    }
}

/// A rendering in progress.
#[derive(Default)]
struct Out {
    s: String,
    /// Line ends owed at the outermost level, written when more follows
    /// and dropped where a cell ends, so that a cell's last paragraph ends
    /// no line.
    lines: usize,
    /// A space owed between two pieces of text in a cell, written only when
    /// more text follows in the cell.
    space: bool,
    /// Text was written since the current cell began.
    wrote: bool,
}

impl Out {
    fn flush_lines(&mut self) {
        for _ in 0..self.lines {
            self.s.push('\n');
        }
        self.lines = 0;
    }

    fn write(&mut self, text: &str, level: u16) {
        self.flush_lines();
        if self.space {
            self.s.push(' ');
            self.space = false;
        }
        if level > 0 && text.contains('\n') {
            self.s.push_str(&text.replace('\n', " "));
        } else {
            self.s.push_str(text);
        }
        self.wrote = true;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn piece(depth: u16) -> Piece {
        Piece {
            what: What::Break,
            revisions: 0,
            hidden: false,
            code: 0,
            depth,
        }
    }

    /// A table of one row, two cells: "a" and "b", "c" in two paragraphs.
    fn table() -> Pieces {
        let mut p = Pieces::default();
        p.push(What::CellStart { tabs: 0 }, piece(1));
        p.push_text("a", piece(1));
        p.push(What::ParagraphEnd, piece(1));
        p.push(What::CellEnd { span: 1 }, piece(1));
        p.push(What::CellStart { tabs: 1 }, piece(1));
        p.push_text("b", piece(1));
        p.push(What::ParagraphEnd, piece(1));
        p.push_text("c", piece(1));
        p.push(What::ParagraphEnd, piece(1));
        p.push(What::CellEnd { span: 1 }, piece(1));
        p.push(What::RowEnd, piece(1));
        p
    }

    #[test]
    fn a_span_is_read_from_its_outermost_level() {
        let p = table();
        let whole = p.span_from(0, 0);
        assert_eq!(p.render(whole, Show::VIEWS), "a\tb c\n");
        // The second cell alone: its paragraphs end lines.
        let cell = Span {
            start: 5,
            end: 9,
            depth: 1,
            code: 0,
            own_level: true,
        };
        assert_eq!(p.item_text(cell, Show::VIEWS), "b\nc");
        // Across the two cells, from inside the table: a tab between them.
        let across = Span {
            start: 1,
            end: 6,
            depth: 1,
            code: 0,
            own_level: false,
        };
        assert_eq!(p.item_text(across, Show::VIEWS), "a\tb");
    }

    #[test]
    fn roman_numerals() {
        assert_eq!(roman(1), "i");
        assert_eq!(roman(14), "xiv");
        assert_eq!(roman(0), "0");
    }
}
