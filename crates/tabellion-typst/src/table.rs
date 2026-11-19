//! `#table` and `#figure` calls as content [`Table`] blocks.
//!
//! A table names its column count or relative widths, then cells in
//! row-major order: plain content blocks, `table.cell` calls with
//! `colspan` and `rowspan`, and one `table.header` call whose cells
//! fill whole rows at the top. A `figure` around a table carries its
//! caption and label. Anything else about the calls is an error
//! naming the argument and the line where it stands: styling stays
//! with the document template.

use nebu_docx::content::{Align, Block, Cell, Inline, Marks, Row, Table};
use typst_syntax::{
    Span,
    ast::{self, AstNode},
};

use super::{Cx, Error, convert_flow, inline_markup, unsupported, unsupported_at};

/// Whether a call builds a table or a figure around one, or errors
/// naming why it does not.
pub(crate) fn is_tableish(call: ast::FuncCall) -> bool {
    match call.callee() {
        ast::Expr::Ident(id) => matches!(id.as_str(), "table" | "figure" | "grid"),
        ast::Expr::FuncCall(_) => false,
        _ => field_table(call).is_some(),
    }
}

/// A `table.` or `grid.` dotted name, if that is what the callee is.
fn field_table(call: ast::FuncCall) -> Option<(String, String)> {
    match call.callee() {
        ast::Expr::FieldAccess(access) => match access.target() {
            ast::Expr::Ident(target) => {
                let (target, field) = (target.as_str(), access.field().as_str());
                if matches!(target, "table" | "grid") {
                    return Some((target.to_string(), field.to_string()));
                }
                None
            }
            _ => None,
        },
        _ => None,
    }
}

/// A `#table`, `#figure` or dotted table call as a block. The label
/// stays empty for the caller, which reads a sibling `<label>`.
pub(crate) fn convert_tableish(cx: &Cx, call: ast::FuncCall) -> Result<Block, Error> {
    match call.callee() {
        ast::Expr::Ident(id) if id.as_str() == "table" => {
            Ok(Block::Table(convert_table(cx, call)?))
        }
        ast::Expr::Ident(id) if id.as_str() == "figure" => {
            Ok(Block::Table(convert_figure(cx, call)?))
        }
        ast::Expr::Ident(id) if id.as_str() == "grid" => {
            Err(unsupported(cx, ast::Expr::FuncCall(call), "`#grid`"))
        }
        _ => {
            let what = field_table(call)
                .map(|(target, field)| format!("`{target}.{field}` outside a table"))
                .unwrap_or_else(|| "a computed call".to_string());
            Err(unsupported(cx, ast::Expr::FuncCall(call), &what))
        }
    }
}

/// A cell as written: its blocks, span and alignment.
struct CellSpec {
    content: Vec<Block>,
    row_span: usize,
    column_span: usize,
    align: Option<Align>,
    span: Span,
}

/// A `#table` call as a content table.
fn convert_table(cx: &Cx, call: ast::FuncCall) -> Result<Table, Error> {
    let args: Vec<ast::Arg> = call.args().items().collect();
    let mut columns: Option<Columns> = None;
    let mut align: Option<Align> = None;
    let mut header: Vec<CellSpec> = Vec::new();
    let mut body: Vec<CellSpec> = Vec::new();
    for arg in args {
        match arg {
            ast::Arg::Named(named) => match named.name().as_str() {
                "columns" => {
                    columns = Some(read_columns(cx, named.expr())?);
                }
                "align" => {
                    align = Some(read_align(cx, named.expr())?);
                }
                name => {
                    return Err(unsupported_at(
                        cx,
                        named.span(),
                        &format!("a `{name}` argument"),
                    ));
                }
            },
            ast::Arg::Pos(expr) => match expr {
                ast::Expr::FuncCall(c) if is_header_call(c) => {
                    if !header.is_empty() {
                        return Err(unsupported(cx, expr, "a second table header"));
                    }
                    header = read_header(cx, c)?;
                }
                _ => body.push(read_cell(cx, expr, "in a table")?),
            },
            ast::Arg::Spread(_) => {
                return Err(unsupported(
                    cx,
                    ast::Expr::FuncCall(call),
                    "spread arguments",
                ));
            }
        }
    }
    lay_out(
        cx,
        call.span(),
        columns.unwrap_or(Columns::Count(1)),
        header,
        body,
        align,
    )
}

/// Column count or relative widths.
enum Columns {
    Count(usize),
    Widths(Vec<f64>),
}

/// A `columns:` argument.
fn read_columns(cx: &Cx, expr: ast::Expr) -> Result<Columns, Error> {
    match expr {
        ast::Expr::Int(n) => {
            let count = n.get();
            if count < 1 {
                return Err(unsupported(cx, expr, "a table without columns"));
            }
            Ok(Columns::Count(count as usize))
        }
        ast::Expr::Array(array) => {
            let mut widths = Vec::new();
            for item in array.items() {
                let ast::ArrayItem::Pos(entry) = item else {
                    return Err(unsupported(cx, expr, "spread columns"));
                };
                match entry {
                    ast::Expr::Numeric(n) => match n.get() {
                        (v, typst_syntax::ast::Unit::Fr) => widths.push(v),
                        _ => {
                            return Err(unsupported_at(
                                cx,
                                entry.span(),
                                "columns with absolute widths",
                            ));
                        }
                    },
                    ast::Expr::Auto(_) => {
                        return Err(unsupported_at(
                            cx,
                            entry.span(),
                            "columns with automatic widths",
                        ));
                    }
                    _ => {
                        return Err(unsupported_at(
                            cx,
                            entry.span(),
                            "columns with automatic widths",
                        ));
                    }
                }
            }
            if widths.is_empty() {
                return Err(unsupported(cx, expr, "a table without columns"));
            }
            Ok(Columns::Widths(widths))
        }
        ast::Expr::Auto(_) => Ok(Columns::Count(1)),
        _ => Err(unsupported(cx, expr, "columns without a count")),
    }
}

/// An `align:` argument or `table.cell` alignment.
fn read_align(cx: &Cx, expr: ast::Expr) -> Result<Align, Error> {
    match expr {
        ast::Expr::Ident(id) => match id.as_str() {
            "left" => Ok(Align::Left),
            "center" => Ok(Align::Center),
            "right" => Ok(Align::Right),
            "justify" => Ok(Align::Justify),
            v => Err(unsupported(cx, expr, &format!("{v} alignment"))),
        },
        _ => Err(unsupported(cx, expr, "a complex alignment")),
    }
}

/// Whether a call is `table.header`.
fn is_header_call(call: ast::FuncCall) -> bool {
    matches!(field_table(call), Some((target, field)) if target == "table" && field == "header")
}

/// A `table.header(...)` call: plain cells filling whole rows.
fn read_header(cx: &Cx, call: ast::FuncCall) -> Result<Vec<CellSpec>, Error> {
    let mut cells = Vec::new();
    for arg in call.args().items() {
        match arg {
            ast::Arg::Pos(expr) => {
                let cell = read_cell(cx, expr, "in a table header")?;
                if cell.row_span != 1 || cell.column_span != 1 {
                    return Err(unsupported(
                        cx,
                        ast::Expr::FuncCall(call),
                        "a spanned header cell",
                    ));
                }
                cells.push(cell);
            }
            ast::Arg::Named(named) if named.name().as_str() == "repeat" => {
                if !matches!(named.expr(), ast::Expr::Bool(b) if b.get()) {
                    return Err(unsupported_at(cx, named.span(), "a non-repeating header"));
                }
            }
            ast::Arg::Named(named) => {
                return Err(unsupported_at(
                    cx,
                    named.span(),
                    &format!("a `{}` header argument", named.name().as_str()),
                ));
            }
            ast::Arg::Spread(_) => {
                return Err(unsupported(
                    cx,
                    ast::Expr::FuncCall(call),
                    "spread header cells",
                ));
            }
        }
    }
    Ok(cells)
}

/// A cell argument: a `table.cell` call, a content block, or a scalar.
fn read_cell(cx: &Cx, expr: ast::Expr, ctx: &str) -> Result<CellSpec, Error> {
    // A block holding only a `table.cell` call is that cell.
    if let ast::Expr::ContentBlock(block) = expr {
        let inner: Vec<ast::Expr> = block
            .body()
            .exprs()
            .filter(|e| !matches!(e, ast::Expr::Space(_)))
            .collect();
        match inner.as_slice() {
            [ast::Expr::FuncCall(call)] if is_cell_call(*call) => {
                return read_spanned_cell(cx, *call);
            }
            [ast::Expr::FuncCall(call)] if is_header_call(*call) => {
                return Err(unsupported(cx, expr, "a table header inside a cell"));
            }
            _ => {}
        }
    }
    if let ast::Expr::FuncCall(call) = expr
        && is_cell_call(call)
    {
        return read_spanned_cell(cx, call);
    }
    Ok(CellSpec {
        content: cell_blocks(cx, expr, ctx)?,
        row_span: 1,
        column_span: 1,
        // Only an explicit cell alignment sticks: the table default
        // applies at layout, so argument order cannot matter.
        align: None,
        span: expr.span(),
    })
}

/// Whether a call is `table.cell`.
fn is_cell_call(call: ast::FuncCall) -> bool {
    matches!(field_table(call), Some((target, field)) if target == "table" && field == "cell")
}

/// A `table.cell` call: content with span and alignment.
fn read_spanned_cell(cx: &Cx, call: ast::FuncCall) -> Result<CellSpec, Error> {
    let mut body: Option<ast::Expr> = None;
    let mut row_span = 1;
    let mut column_span = 1;
    let mut cell_align = None;
    for arg in call.args().items() {
        match arg {
            ast::Arg::Pos(expr) => {
                if body.is_some() {
                    return Err(unsupported(cx, expr, "a cell with two bodies"));
                }
                body = Some(expr);
            }
            ast::Arg::Named(named) => match named.name().as_str() {
                "colspan" | "rowspan" => {
                    let ast::Expr::Int(n) = named.expr() else {
                        return Err(unsupported_at(cx, named.span(), "a computed span"));
                    };
                    if n.get() < 1 {
                        return Err(unsupported_at(
                            cx,
                            named.span(),
                            &format!("a {} of {}", named.name().as_str(), n.get()),
                        ));
                    }
                    if named.name().as_str() == "colspan" {
                        column_span = n.get() as usize;
                    } else {
                        row_span = n.get() as usize;
                    }
                }
                "align" => cell_align = Some(read_align(cx, named.expr())?),
                "x" | "y" => {
                    return Err(unsupported_at(cx, named.span(), "a positioned table cell"));
                }
                name => {
                    return Err(unsupported_at(
                        cx,
                        named.span(),
                        &format!("a `{name}` cell argument"),
                    ));
                }
            },
            ast::Arg::Spread(_) => {
                return Err(unsupported(
                    cx,
                    ast::Expr::FuncCall(call),
                    "spread cell arguments",
                ));
            }
        }
    }
    let Some(body) = body else {
        return Err(unsupported(
            cx,
            ast::Expr::FuncCall(call),
            "an empty table cell",
        ));
    };
    Ok(CellSpec {
        content: cell_blocks(cx, body, "in a table")?,
        row_span,
        column_span,
        align: cell_align,
        span: call.span(),
    })
}

/// A cell body as blocks: content, or scalar text.
fn cell_blocks(cx: &Cx, expr: ast::Expr, ctx: &str) -> Result<Vec<Block>, Error> {
    match expr {
        ast::Expr::ContentBlock(block) => {
            convert_flow(cx, &block.body().exprs().collect::<Vec<_>>(), true)
                .map(|(blocks, _)| blocks)
        }
        ast::Expr::Str(s) => Ok(vec![paragraph(vec![Inline::text(&s.get())])]),
        ast::Expr::Int(n) => Ok(vec![paragraph(vec![Inline::text(&n.get().to_string())])]),
        ast::Expr::Float(n) => Ok(vec![paragraph(vec![Inline::text(&n.get().to_string())])]),
        ast::Expr::Bool(b) => Ok(vec![paragraph(vec![Inline::text(&b.get().to_string())])]),
        _ => Err(unsupported(
            cx,
            expr,
            &format!("{} {ctx}", describeish(expr)),
        )),
    }
}

/// A single paragraph of inlines.
fn paragraph(content: Vec<Inline>) -> Block {
    Block::Paragraph {
        style: None,
        align: None,
        content,
    }
}

/// A short name for a cell-body error.
fn describeish(expr: ast::Expr) -> String {
    match expr {
        ast::Expr::FuncCall(call) => match call.callee() {
            ast::Expr::Ident(ident) => format!("`#{}`", ident.as_str()),
            _ => field_table(call)
                .map(|(target, field)| format!("`{target}.{field}`"))
                .unwrap_or_else(|| "a computed call".to_string()),
        },
        ast::Expr::Equation(_) | ast::Expr::Math(_) => "math".to_string(),
        ast::Expr::ListItem(_) | ast::Expr::EnumItem(_) => "a list".to_string(),
        _ => "this".to_string(),
    }
}

/// Grid positions a table may cover before it is too big to lay
/// out. Every row allocation below stays within this budget, checked
/// with checked arithmetic before it grows.
const GRID_BUDGET: usize = 1_000_000;

/// Cells in row-major order onto the grid. Header cells come first
/// and fill whole rows; anything ragged is an error at the table.
/// The occupancy grid stays within a budget no matter what the input
/// claims: positions are counted with checked arithmetic before any
/// row is allocated.
fn lay_out(
    cx: &Cx,
    span: Span,
    columns: Columns,
    header: Vec<CellSpec>,
    body: Vec<CellSpec>,
    align: Option<Align>,
) -> Result<Table, Error> {
    let (ncols, widths) = match columns {
        Columns::Count(n) => (n, None),
        Columns::Widths(w) => (w.len(), Some(w)),
    };
    if ncols < 1 {
        return Err(unsupported_at(cx, span, "a table without columns"));
    }
    if !header.is_empty() && !header.len().is_multiple_of(ncols) {
        return Err(unsupported_at(cx, span, "a partial header row"));
    }
    let header_rows = header.len() / ncols;
    // Grid occupancy: rows grow as spans reach into them. Cells are
    // placed in source order from a forward-only row-major cursor, as
    // Typst numbers automatic cells: a later cell never backfills an
    // earlier gap, so gaps stay ragged and the writer reads every row
    // in column order.
    let mut covered: Vec<Vec<bool>> = Vec::new();
    let mut rows: Vec<Row> = Vec::new();
    let mut cursor = (0usize, 0usize);
    let mut place = |spec: &CellSpec| -> Result<(), Error> {
        if spec.column_span > ncols {
            return Err(unsupported_at(cx, spec.span, "a cell wider than the table"));
        }
        // The search below always fits a new row at the latest, so the
        // grid can only grow by this span: count first, allocate after.
        let too_big = || unsupported_at(cx, spec.span, "a table too big to lay out");
        let end = covered
            .len()
            .checked_add(spec.row_span)
            .ok_or_else(&too_big)?;
        if end.checked_mul(ncols).ok_or_else(&too_big)? > GRID_BUDGET {
            return Err(too_big());
        }
        let mut r = cursor.0;
        loop {
            while covered.len() < r + spec.row_span {
                covered.push(vec![false; ncols]);
            }
            let mut c = if r == cursor.0 { cursor.1 } else { 0 };
            while c + spec.column_span <= ncols {
                if (r..r + spec.row_span)
                    .all(|rr| (c..c + spec.column_span).all(|cc| !covered[rr][cc]))
                {
                    for row in covered.iter_mut().skip(r).take(spec.row_span) {
                        for covered in row.iter_mut().skip(c).take(spec.column_span) {
                            *covered = true;
                        }
                    }
                    while rows.len() <= r {
                        rows.push(Row { cells: Vec::new() });
                    }
                    rows[r].cells.push(Cell {
                        content: spec.content.clone(),
                        row_span: spec.row_span,
                        column_span: spec.column_span,
                        align: spec.align.or(align),
                        vertical_align: None,
                        shading: None,
                        borders: None,
                    });
                    cursor = (r, c + spec.column_span);
                    return Ok(());
                }
                c += 1;
            }
            r += 1;
        }
    };
    for spec in header.iter().chain(body.iter()) {
        place(spec)?;
    }
    if rows.is_empty() {
        return Err(unsupported_at(cx, span, "an empty table"));
    }
    // A rowspan may cover trailing rows no cell begins in: list them
    // empty, so the grid the writer checks matches the covered one.
    while rows.len() < covered.len() {
        rows.push(Row { cells: Vec::new() });
    }
    if covered.iter().any(|row| row.iter().any(|c| !c)) {
        return Err(unsupported_at(cx, span, "a ragged table"));
    }
    Ok(Table {
        rows,
        header_rows,
        first_column: false,
        style: None,
        widths,
        caption: None,
        label: None,
    })
}

/// A `#figure` around a table: the caption and an optional label.
fn convert_figure(cx: &Cx, call: ast::FuncCall) -> Result<Table, Error> {
    let mut body: Option<ast::Expr> = None;
    let mut caption: Option<Vec<Inline>> = None;
    let mut label: Option<String> = None;
    for arg in call.args().items() {
        match arg {
            ast::Arg::Pos(expr) => {
                if body.is_some() {
                    return Err(unsupported(cx, expr, "a figure with two bodies"));
                }
                body = Some(expr);
            }
            ast::Arg::Named(named) if named.name().as_str() == "caption" => match named.expr() {
                ast::Expr::ContentBlock(block) => {
                    let mut content = Vec::new();
                    inline_markup(
                        cx,
                        block.body(),
                        &mut content,
                        Marks::default(),
                        "a caption",
                    )?;
                    caption = Some(content);
                }
                ast::Expr::Str(s) => {
                    caption = Some(vec![Inline::text(&s.get())]);
                }
                _ => {
                    return Err(unsupported_at(cx, named.span(), "a computed caption"));
                }
            },
            ast::Arg::Named(named) if named.name().as_str() == "label" => {
                if let ast::Expr::Label(l) = named.expr() {
                    label = Some(l.get().to_string());
                } else {
                    return Err(unsupported_at(cx, named.span(), "a computed label"));
                }
            }
            ast::Arg::Named(named) => {
                return Err(unsupported_at(
                    cx,
                    named.span(),
                    &format!("a `{}` figure argument", named.name().as_str()),
                ));
            }
            ast::Arg::Spread(_) => {
                return Err(unsupported(
                    cx,
                    ast::Expr::FuncCall(call),
                    "spread figure arguments",
                ));
            }
        }
    }
    let Some(body) = body else {
        return Err(unsupported(
            cx,
            ast::Expr::FuncCall(call),
            "an empty figure",
        ));
    };
    let ast::Expr::FuncCall(inner) = body else {
        return Err(unsupported(cx, body, "a figure around content"));
    };
    if !matches!(inner.callee(), ast::Expr::Ident(id) if id.as_str() == "table") {
        return Err(unsupported(
            cx,
            body,
            &format!("a figure around {}", describeish(body)),
        ));
    }
    let mut table = convert_table(cx, inner)?;
    table.caption = caption;
    if label.is_some() {
        table.label = label;
    }
    Ok(table)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Error, parse};

    fn table_of(source: &str) -> Table {
        let content = parse(source).unwrap();
        assert_eq!(content.blocks.len(), 1, "{source:?}");
        let Block::Table(table) = &content.blocks[0] else {
            panic!("a table: {source:?}");
        };
        table.clone()
    }

    fn cell_text(table: &Table, row: usize, cell: usize) -> String {
        let Block::Paragraph { content, .. } = &table.rows[row].cells[cell].content[0] else {
            panic!("a paragraph: {table:?}");
        };
        content
            .iter()
            .filter_map(|i| match i {
                Inline::Text { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn reads_a_plain_grid() {
        let table = table_of("#table(columns: 2, [a], [b], [c], [d])\n");
        assert_eq!(table.rows.len(), 2);
        assert_eq!(table.header_rows, 0);
        assert!(table.widths.is_none());
        assert_eq!(cell_text(&table, 0, 0), "a");
        assert_eq!(cell_text(&table, 0, 1), "b");
        assert_eq!(cell_text(&table, 1, 0), "c");
        assert_eq!(cell_text(&table, 1, 1), "d");
    }

    #[test]
    fn reads_header_widths_and_alignment() {
        let table = table_of(
            "#table(columns: (1fr, 2fr), align: center, table.header([H1], [H2]), [a], [b])\n",
        );
        assert_eq!(table.header_rows, 1);
        assert_eq!(table.widths, Some(vec![1.0, 2.0]));
        assert_eq!(cell_text(&table, 0, 0), "H1");
        for row in &table.rows {
            for cell in &row.cells {
                assert_eq!(cell.align, Some(Align::Center), "{cell:?}");
            }
        }
    }

    #[test]
    fn reads_spans() {
        let table = table_of("#table(columns: 2, table.cell(colspan: 2)[wide], [b], [c])\n");
        assert_eq!(table.rows.len(), 2);
        assert_eq!(table.rows[0].cells.len(), 1);
        assert_eq!(table.rows[0].cells[0].column_span, 2);
        assert_eq!(cell_text(&table, 0, 0), "wide");
    }

    #[test]
    fn lists_trailing_rows_covered_by_a_rowspan() {
        let table = table_of("#table(columns: 1, table.cell(rowspan: 2)[A])\n");
        assert_eq!(table.rows.len(), 2);
        assert!(table.rows[1].cells.is_empty());
        assert_eq!(cell_text(&table, 0, 0), "A");
    }

    #[test]
    fn reads_a_captioned_table() {
        let table = table_of("#figure(table(columns: 1, [a]), caption: [Sites]) <tab-sites>\n");
        assert_eq!(table.label.as_deref(), Some("tab-sites"));
        let caption = table.caption.clone().expect("a caption");
        assert_eq!(caption.len(), 1);
        assert!(matches!(
            &caption[0],
            Inline::Text { text, .. } if text == "Sites"
        ));
        assert_eq!(cell_text(&table, 0, 0), "a");
    }

    #[test]
    fn rejects_duplicate_arguments() {
        // The parser itself rejects a repeated named argument, so the
        // table default cannot depend on argument order.
        for source in [
            "#table(align: left, align: right, [a])\n",
            "#table(columns: 1, columns: 2, [a])\n",
        ] {
            let err = parse(source).unwrap_err();
            let Error::Syntax { line, message } = err else {
                panic!("a syntax error: {source:?}");
            };
            assert_eq!(line, 1);
            assert!(message.contains("duplicate argument"), "{message:?}");
        }
    }

    #[test]
    fn rejects_what_tables_cannot_hold() {
        for (source, line, what) in [
            ("#table(columns: 2, [a], [b], [c])\n", 1, "a ragged table"),
            ("#table(columns: 0, [a])\n", 1, "a table without columns"),
            (
                "#table(columns: 2, stroke: 1pt, [a], [b])\n",
                1,
                "a `stroke` argument",
            ),
            (
                "#table(columns: 2, table.header([H]))\n",
                1,
                "a partial header row",
            ),
            (
                "#table(columns: 2, table.cell(colspan: 3)[w], [a], [b])\n",
                1,
                "a cell wider than the table",
            ),
            (
                "#table(columns: 2, table.hline(), [a], [b])\n",
                1,
                "`table.hline` in a table",
            ),
            (
                "#table(columns: 9223372036854775807, [a])\n",
                1,
                "a table too big to lay out",
            ),
            (
                "#table(columns: 1, table.cell(rowspan: 999999999999)[A])\n",
                1,
                "a table too big to lay out",
            ),
            (
                "#table(columns: 4, [A], table.cell(rowspan: 2)[B], table.cell(colspan: 2)[C], table.cell(colspan: 2)[D], [E])\n",
                1,
                "a ragged table",
            ),
            ("#grid(columns: 2, [a])\n", 1, "`#grid`"),
            (
                "#figure(table(columns: 1, [a]), placement: auto)\n",
                1,
                "a `placement` figure argument",
            ),
            ("#figure(image(\"p.png\"))\n", 1, "a figure around `#image`"),
        ] {
            let err = parse(source).unwrap_err();
            assert_eq!(
                err,
                Error::Unsupported {
                    line,
                    what: what.to_string()
                },
                "{source:?}"
            );
        }
    }
}
