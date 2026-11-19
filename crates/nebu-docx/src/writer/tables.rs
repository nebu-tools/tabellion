//! Tables: the grid from the spans, a real table style, header rows, and
//! cell formatting as direct formatting over the style.
//!
//! The layout is fixed: the grid columns and each cell have their width in
//! twentieths of a point, and the table is as wide as the text. Word then
//! draws the table as written and does not measure the content to size
//! the columns again, so the table looks the same in every reader and
//! after every save; Word's own `Tables.Add` writes a fixed layout too.

use crate::content::{Align, Block, CellBorders, Table, VerticalAlign, grid};
use crate::view::StyleKind;
use crate::{Document, Error};

use super::roles::{Role, border_xml};
use super::session::{Ctx, Session};
use super::xml::attr;

/// Word's default cell padding, left and right, in points.
const PADDING: f64 = 5.4;

impl Session<'_> {
    pub(crate) fn table(
        &mut self,
        doc: &mut Document,
        t: &Table,
        ctx: &Ctx,
        path: &str,
    ) -> Result<String, Error> {
        let g = grid(t).map_err(|e| Error::Invalid(format!("{path}: {e}")))?;
        let style = match &t.style {
            Some(name) => {
                let defs = self.opts.table_styles.clone();
                self.book
                    .table_style(doc, &defs, name)
                    .map_err(|e| match e {
                        Error::Invalid(m) => Error::Invalid(format!("{path}.style: {m}")),
                        e => e,
                    })?
            }
            None => match self.style_ref(doc, Role::Table)? {
                Some(id) => id,
                None => self
                    .book
                    .find(StyleKind::Table, "Table Grid")
                    .unwrap_or_default(),
            },
        };
        if !self.table_styles.contains(&style) {
            self.table_styles.push(style.clone());
        }
        let mut out = String::new();
        if let Some(c) = &t.caption {
            self.main_only(ctx, &format!("{path}.caption"), "a caption")?;
            let word = self.opts.captions.table.clone();
            out.push_str(&self.caption(doc, ctx, &word, c, t.label.as_deref(), true, path)?);
        }
        // Column widths in twips, summing to the width available.
        let total = (ctx.width * 20.0).round().max(720.0) as i64;
        let weights: Vec<f64> = match &t.widths {
            Some(w) => w.clone(),
            None => vec![1.0; g.columns],
        };
        let sum: f64 = weights.iter().sum();
        if !sum.is_finite() || sum <= 0.0 {
            return Err(Error::Invalid(format!(
                "{path}.widths: the widths add up to no finite total"
            )));
        }
        let mut cols: Vec<i64> = weights
            .iter()
            .map(|w| ((w / sum) * total as f64).floor() as i64)
            .collect();
        let used: i64 = cols.iter().sum();
        if let Some(last) = cols.last_mut() {
            *last += total - used;
        }
        let mut x = String::from("<w:tbl><w:tblPr>");
        if !style.is_empty() {
            x.push_str(&format!("<w:tblStyle w:val=\"{}\"/>", attr(&style)));
        }
        x.push_str(&format!(
            "<w:tblW w:w=\"{total}\" w:type=\"dxa\"/><w:tblLayout w:type=\"fixed\"/>"
        ));
        x.push_str(&format!(
            "<w:tblLook w:val=\"{:04X}\" w:firstRow=\"{}\" w:lastRow=\"0\" w:firstColumn=\"{}\" w:lastColumn=\"0\" w:noHBand=\"0\" w:noVBand=\"1\"/>",
            (if t.header_rows > 0 { 0x20 } else { 0 })
                | (if t.first_column { 0x80 } else { 0 })
                | 0x400,
            u8::from(t.header_rows > 0),
            u8::from(t.first_column)
        ));
        x.push_str("</w:tblPr><w:tblGrid>");
        for c in &cols {
            x.push_str(&format!("<w:gridCol w:w=\"{c}\"/>"));
        }
        x.push_str("</w:tblGrid>");
        for r in 0..t.rows.len() {
            crate::cost::tick(1);
            let id = self.para_id(ctx);
            x.push_str(&format!("<w:tr{id}>"));
            if r < t.header_rows {
                x.push_str("<w:trPr><w:tblHeader/></w:trPr>");
            }
            let mut col = 0;
            while col < g.columns {
                let (owner_row, k) = g.cover[r][col];
                let cell = &t.rows[owner_row].cells[k];
                let start = g.starts[owner_row][k];
                let span = cell.column_span;
                let width: i64 = cols[start..start + span].iter().sum();
                let mut pr = format!("<w:tcW w:w=\"{width}\" w:type=\"dxa\"/>");
                if span > 1 {
                    pr.push_str(&format!("<w:gridSpan w:val=\"{span}\"/>"));
                }
                if cell.row_span > 1 {
                    if owner_row == r {
                        pr.push_str("<w:vMerge w:val=\"restart\"/>");
                    } else {
                        pr.push_str("<w:vMerge/>");
                    }
                }
                if let Some(b) = &cell.borders {
                    pr.push_str(&cell_borders(b));
                }
                if let Some(s) = &cell.shading {
                    pr.push_str(&format!(
                        "<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"{}\"/>",
                        s.hex()
                    ));
                }
                if let Some(v) = cell.vertical_align {
                    pr.push_str(&format!(
                        "<w:vAlign w:val=\"{}\"/>",
                        match v {
                            VerticalAlign::Top => "top",
                            VerticalAlign::Center => "center",
                            VerticalAlign::Bottom => "bottom",
                        }
                    ));
                }
                x.push_str(&format!("<w:tc><w:tcPr>{pr}</w:tcPr>"));
                if owner_row == r {
                    let cctx = Ctx {
                        in_cell: true,
                        width: (width as f64 / 20.0 - 2.0 * PADDING).max(36.0),
                        text: Role::TableText,
                        align: cell.align.or(None::<Align>),
                        place: "a table cell",
                        ..ctx.clone()
                    };
                    let cp = format!("{path}.rows[{r}].cells[{k}].content");
                    let mut body = String::new();
                    for (i, b) in cell.content.iter().enumerate() {
                        body.push_str(&self.block(doc, b, &cctx, &format!("{cp}[{i}]"))?);
                    }
                    if !matches!(
                        cell.content.last(),
                        Some(
                            Block::Paragraph { .. }
                                | Block::Heading { .. }
                                | Block::List(_)
                                | Block::Image { .. }
                                | Block::Equation { .. }
                                | Block::Figure { .. }
                        )
                    ) {
                        // A cell ends with a paragraph.
                        let props = match self.style_ref(doc, Role::TableText)? {
                            Some(s) => {
                                vec![("pStyle", format!("<w:pStyle w:val=\"{}\"/>", attr(&s)))]
                            }
                            None => vec![],
                        };
                        body.push_str(&self.paragraph_xml(&cctx, props, ""));
                    }
                    x.push_str(&body);
                } else {
                    let p = self.paragraph_xml(ctx, vec![], "");
                    x.push_str(&p);
                }
                x.push_str("</w:tc>");
                col = start + span;
            }
            x.push_str("</w:tr>");
        }
        x.push_str("</w:tbl>");
        out.push_str(&x);
        Ok(out)
    }
}

fn cell_borders(b: &CellBorders) -> String {
    let mut x = String::from("<w:tcBorders>");
    for (side, s) in [
        ("top", &b.top),
        ("left", &b.left),
        ("bottom", &b.bottom),
        ("right", &b.right),
    ] {
        if let Some(s) = s {
            x.push_str(&border_xml(side, s));
        }
    }
    x.push_str("</w:tcBorders>");
    x
}
