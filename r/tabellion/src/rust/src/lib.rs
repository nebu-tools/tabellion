//! R bindings. The R functions in R/tabellion.R are the interface; these are
//! the few entry points they call.

use extendr_api::prelude::*;
use extendr_api::Result;
use nebu_docx::content::{Align, Block, Cell, Figure, FigureKind, Image, Inline, Table};
use nebu_docx::document::{BlockKind, Locator};
use nebu_docx::{Document, Place, Revision, Roles, TableStyleDef, WriteOptions};

fn err<E: std::fmt::Display>(e: E) -> Error {
    Error::Other(e.to_string())
}

/// An open .docx.
#[extendr]
pub struct Docx {
    doc: Document,
    roles: Roles,
}

fn place(doc: &mut Document, at: &str, mode: &str) -> Result<Place> {
    if at == "end" {
        return Ok(Place::End);
    }
    let loc: Locator = at.parse().map_err(err)?;
    let b = doc.locate(&loc).map_err(err)?;
    let mode = if mode.is_empty() { if matches!(loc, Locator::Placeholder(_)) { "replace" } else { "after" } } else { mode };
    Ok(match mode {
        "before" => Place::Before(b.node),
        "after" => Place::After(b.node),
        "replace" => Place::Replace(b.node),
        m => return Err(err(format!("mode must be before, after or replace, not {m}"))),
    })
}

fn options(roles: &Roles, author: &str, styles: Vec<TableStyleDef>) -> WriteOptions {
    WriteOptions {
        roles: roles.clone(),
        track: (!author.is_empty()).then(|| Revision::now(author)),
        table_styles: styles,
    }
}

fn caption(caption: &str) -> Option<Vec<Inline>> {
    (!caption.is_empty()).then(|| vec![Inline::text(caption)])
}

fn label(label: &str) -> Option<String> {
    (!label.is_empty()).then(|| label.to_string())
}

#[extendr]
impl Docx {
    fn open(path: &str) -> Result<Self> {
        let mut doc = Document::open(path).map_err(err)?;
        let roles = Roles::detect(doc.styles().map_err(err)?);
        Ok(Docx { doc, roles })
    }

    fn save(&mut self, path: &str) -> Result<()> {
        self.doc.save(path).map_err(err)
    }

    /// Paragraph and table blocks as JSON.
    fn blocks_json(&mut self) -> Result<String> {
        let b = self.doc.blocks().map_err(err)?;
        serde_json::to_string(&b).map_err(err)
    }

    /// Every table as rows of cell text, as JSON.
    fn tables_json(&mut self) -> Result<String> {
        let blocks = self.doc.blocks().map_err(err)?;
        let t = self.doc.tree();
        let mut out = vec![];
        for b in blocks.iter().filter(|b| b.kind == BlockKind::Table) {
            let rows: Vec<Vec<String>> = t
                .elements(b.node)
                .filter(|&r| t.is(r, "w:tr"))
                .map(|r| t.elements(r).filter(|&c| t.is(c, "w:tc")).map(|c| nebu_docx::document::visible_text(t, c)).collect())
                .collect();
            out.push(serde_json::json!({ "block": b.id, "rows": rows }));
        }
        serde_json::to_string(&out).map_err(err)
    }

    /// A table from columns of text. `style` is a style name, `style_def` a
    /// table style definition as JSON; one of them may be empty.
    #[allow(clippy::too_many_arguments)]
    fn add_table(
        &mut self,
        header: Strings,
        columns: List,
        numeric: Logicals,
        style: &str,
        style_def: &str,
        at: &str,
        mode: &str,
        caption_text: &str,
        label_text: &str,
        author: &str,
    ) -> Result<()> {
        let cols: Vec<Vec<String>> = columns
            .values()
            .map(|c| c.as_string_vector().ok_or_else(|| err("each column must be character")))
            .collect::<Result<_>>()?;
        let n = cols.first().map_or(0, |c| c.len());
        let right: Vec<Option<Align>> = numeric.iter().map(|x| x.is_true().then_some(Align::Right)).collect();
        let mut rows = vec![header.iter().map(|h| Cell::text(h.as_str())).collect::<Vec<_>>()];
        for i in 0..n {
            rows.push(cols.iter().map(|c| Cell::text(c[i].as_str())).collect());
        }
        let mut defs = vec![];
        let style_name = if !style_def.is_empty() {
            let d: TableStyleDef = serde_json::from_str(style_def).map_err(|e| err(format!("table style: {e}")))?;
            let n = d.name.clone();
            defs.push(d);
            Some(n)
        } else if !style.is_empty() {
            Some(style.to_string())
        } else {
            None
        };
        let table = Block::Table(Table { style: style_name, widths: vec![], header_rows: 1, rows, align: right });
        let block = match caption(caption_text) {
            Some(c) => Block::Figure(Figure { kind: FigureKind::Table, body: vec![table], caption: Some(c), label: label(label_text) }),
            None => table,
        };
        let p = place(&mut self.doc, at, mode)?;
        let o = options(&self.roles, author, defs);
        self.doc.insert(p, &[block], &o).map_err(err)?;
        Ok(())
    }

    /// A picture from a PNG or JPEG file, `width_cm` wide (0: natural size).
    fn add_image(&mut self, path: &str, width_cm: f64, alt: &str, at: &str, mode: &str, caption_text: &str, label_text: &str, author: &str) -> Result<()> {
        let data = std::fs::read(path).map_err(err)?;
        let name = std::path::Path::new(path).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "image.png".into());
        let (pw, ph) = png_dims(&data).unwrap_or((4, 3));
        let width = (width_cm * 360000.0) as i64;
        let height = if width > 0 { width * ph as i64 / pw.max(1) as i64 } else { 0 };
        let img = Block::Image(Image { name, data, width, height, alt: alt.into(), fallback_png: None });
        let block = Block::Figure(Figure { kind: FigureKind::Figure, body: vec![img], caption: caption(caption_text), label: label(label_text) });
        let p = place(&mut self.doc, at, mode)?;
        let o = options(&self.roles, author, vec![]);
        self.doc.insert(p, &[block], &o).map_err(err)?;
        Ok(())
    }

    /// Typst source as Word content. Returns the warnings.
    fn add_typst(&mut self, source: &str, root: &str, at: &str, mode: &str, author: &str) -> Result<Strings> {
        let opts = tabellion_typst::Options { root: root.into(), ..Default::default() };
        let f = tabellion_typst::convert(source, &opts).map_err(err)?;
        let p = place(&mut self.doc, at, mode)?;
        let o = options(&self.roles, author, f.table_styles);
        self.doc.insert(p, &f.blocks, &o).map_err(err)?;
        Ok(f.warnings.iter().map(|s| s.as_str()).collect::<Strings>())
    }
}

fn png_dims(d: &[u8]) -> Option<(u32, u32)> {
    if d.len() < 24 || &d[1..4] != b"PNG" {
        return None;
    }
    let be = |i: usize| u32::from_be_bytes([d[i], d[i + 1], d[i + 2], d[i + 3]]);
    Some((be(16), be(20)))
}

/// A table style bound to `name` in Typst source, as JSON.
#[extendr]
fn typst_table_style_json(source: &str, name: &str, root: &str) -> Result<String> {
    let opts = tabellion_typst::Options { root: root.into(), ..Default::default() };
    let def = tabellion_typst::table_style(source, name, &opts).map_err(err)?;
    serde_json::to_string(&def).map_err(err)
}

extendr_module! {
    mod tabellion;
    impl Docx;
    fn typst_table_style_json;
}
