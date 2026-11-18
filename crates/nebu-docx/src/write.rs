//! Content model -> WordprocessingML in the target document's styles.
//!
//! Which style plays which part (body text, caption, list, footnote) comes
//! from [`Roles`]; a pack fills them in, and [`Roles::detect`] finds Word's
//! built-in styles when there is no pack. Captions carry real SEQ fields and
//! cross-references real REF fields, so Word numbers and updates them.

use std::collections::HashMap;

use crate::content::{Align, Block, Cell, Figure, FigureKind, Image, Inline, Marks, Math, Table};
use crate::document::{Document, Revision};
use crate::package::{REL_FOOTNOTES, REL_HYPERLINK, REL_IMAGE};
use crate::styles::{StyleKind, Styles};
use crate::table_style::TableStyle;
use crate::xml::{esc, esc_attr, NodeId};
use crate::Error;

/// Paragraph and character styles by role, as style ids.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Roles {
    pub body: Option<String>,
    pub headings: Vec<String>,
    pub caption: Option<String>,
    pub list_bullet: Option<String>,
    pub list_number: Option<String>,
    pub footnote_text: Option<String>,
    pub footnote_reference: Option<String>,
    pub figure: Option<String>,
    pub table_text: Option<String>,
    pub equation: Option<String>,
    pub hyperlink: Option<String>,
    pub code: Option<String>,
    pub table_style: Option<String>,
    /// Caption words, e.g. "Figure", "Table". Also the SEQ identifiers.
    pub figure_word: String,
    pub table_word: String,
    pub equation_word: String,
    pub caption_separator: String,
    pub table_caption_above: bool,
}

impl Roles {
    /// Word's built-in styles, found by their stored English names.
    pub fn detect(s: &Styles) -> Roles {
        let p = |n: &str| s.resolve(n, Some(StyleKind::Paragraph)).map(|x| x.id.clone());
        let c = |n: &str| s.resolve(n, Some(StyleKind::Character)).map(|x| x.id.clone());
        Roles {
            body: p("Body Text").or_else(|| p("Normal")),
            headings: (1..=9).map_while(|i| p(&format!("heading {i}"))).collect(),
            caption: p("caption"),
            list_bullet: p("List Bullet"),
            list_number: p("List Number"),
            footnote_text: p("footnote text"),
            footnote_reference: c("footnote reference"),
            figure: None,
            table_text: None,
            equation: None,
            hyperlink: c("Hyperlink"),
            code: c("HTML Code"),
            table_style: s.resolve("Table Grid", Some(StyleKind::Table)).map(|x| x.id.clone()),
            figure_word: "Figure".into(),
            table_word: "Table".into(),
            equation_word: "Equation".into(),
            caption_separator: ": ".into(),
            table_caption_above: true,
        }
    }
}

/// Where new content goes, relative to a body block.
#[derive(Debug, Clone, Copy)]
pub enum Place {
    Before(NodeId),
    After(NodeId),
    /// The block goes away (tracked as a deletion when tracking).
    Replace(NodeId),
    /// Before the final section properties.
    End,
}

#[derive(Debug, Clone, Default)]
pub struct WriteOptions {
    pub roles: Roles,
    pub track: Option<Revision>,
    /// Table style definitions that `Table::style` may name.
    pub table_styles: Vec<crate::table_style::TableStyleDef>,
}

struct Writer<'a> {
    doc: &'a mut Document,
    o: &'a WriteOptions,
    labels: HashMap<String, String>,
    text_width: i64,
    footnotes: Vec<String>,
    next_footnote: i64,
    new_nums: Vec<String>,
    next_num: i64,
    images: usize,
    equation_labels: std::collections::HashSet<String>,
    equation_refs: std::collections::HashSet<String>,
}

impl Document {
    /// Writes `blocks` at `place` and returns the new body blocks. Fields are
    /// brought up to date afterwards.
    pub fn insert(&mut self, place: Place, blocks: &[Block], o: &WriteOptions) -> Result<Vec<NodeId>, Error> {
        if let Some(r) = &o.track {
            if let Place::Replace(n) = place {
                crate::revise::check_foreign(self.tree(), n, &r.author)?;
            }
        }
        for (p, uri) in [
            ("r", "http://schemas.openxmlformats.org/officeDocument/2006/relationships"),
            ("m", "http://schemas.openxmlformats.org/officeDocument/2006/math"),
            ("wp", "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"),
        ] {
            self.ensure_ns(p, uri)?;
        }
        let mut w = Writer::new(self, o)?;
        w.collect_labels(blocks);
        let mut xml = String::new();
        for b in blocks {
            w.block(b, &mut xml, 0)?;
        }
        w.finish()?;
        let t = self.tree();
        let nodes = t.parse_fragment(&xml).map_err(|e| Error::Xml("generated content".into(), e))?;
        let body = {
            let root = t.document_element();
            t.child(root, "w:body").unwrap()
        };
        match place {
            Place::Before(a) => {
                for &n in &nodes {
                    t.insert_before(a, n);
                }
            }
            Place::After(a) => {
                for &n in nodes.iter().rev() {
                    t.insert_after(a, n);
                }
            }
            Place::Replace(a) => {
                for &n in nodes.iter().rev() {
                    t.insert_after(a, n);
                }
                match &o.track {
                    None => t.detach(a),
                    Some(r) => {
                        let r = r.clone();
                        crate::revise::mark_deleted(self, a, &r)?;
                    }
                }
            }
            Place::End => {
                let sect = t.elements(body).filter(|&c| t.is(c, "w:sectPr")).last();
                for &n in &nodes {
                    match sect {
                        Some(s) => t.insert_before(s, n),
                        None => t.append(body, n),
                    }
                }
            }
        }
        crate::fields::update(self)?;
        Ok(nodes)
    }

    /// Declares a namespace prefix on the document element if it is missing.
    pub fn ensure_ns(&mut self, prefix: &str, uri: &str) -> Result<(), Error> {
        let t = self.tree();
        match t.ns_uri(prefix) {
            Some(u) if u == uri => Ok(()),
            Some(u) => Err(Error::Unsupported(format!("the document binds {prefix} to {u}, expected {uri}"))),
            None => {
                let root = t.document_element();
                t.set_attr(root, &format!("xmlns:{prefix}"), uri);
                Ok(())
            }
        }
    }

    /// Usable width between the margins of the last section, in twips.
    pub fn text_width(&mut self) -> i64 {
        let body = self.body();
        let t = self.tree();
        let sect = t.elements(body).filter(|&c| t.is(c, "w:sectPr")).last();
        let get = |n: Option<NodeId>, a: &str| n.and_then(|n| t.attr(n, a)).and_then(|v| v.parse::<i64>().ok());
        let pg = sect.and_then(|s| t.child(s, "w:pgSz"));
        let mar = sect.and_then(|s| t.child(s, "w:pgMar"));
        let w = get(pg, "w:w").unwrap_or(11906);
        w - get(mar, "w:left").unwrap_or(1440) - get(mar, "w:right").unwrap_or(1440)
    }
}

impl<'a> Writer<'a> {
    fn new(doc: &'a mut Document, o: &'a WriteOptions) -> Result<Writer<'a>, Error> {
        let text_width = doc.text_width();
        let mut next_footnote = 0;
        if doc.pkg.has("word/footnotes.xml") {
            let t = doc.pkg.xml("word/footnotes.xml")?;
            let root = t.document_element();
            for f in t.elements(root) {
                if let Some(n) = t.attr(f, "w:id").and_then(|v| v.parse::<i64>().ok()) {
                    next_footnote = next_footnote.max(n);
                }
            }
        }
        let mut next_num = 0;
        if doc.pkg.has("word/numbering.xml") {
            let t = doc.pkg.xml("word/numbering.xml")?;
            let root = t.document_element();
            for n in t.elements(root).filter(|&c| t.is(c, "w:num")) {
                if let Some(v) = t.attr(n, "w:numId").and_then(|v| v.parse::<i64>().ok()) {
                    next_num = next_num.max(v);
                }
            }
        }
        let images = doc.pkg.part_names().filter(|n| n.starts_with("word/media/")).count();
        Ok(Writer {
            doc,
            o,
            labels: HashMap::new(),
            text_width,
            footnotes: vec![],
            next_footnote,
            new_nums: vec![],
            next_num,
            images,
            equation_labels: Default::default(),
            equation_refs: Default::default(),
        })
    }

    fn collect_labels(&mut self, blocks: &[Block]) {
        for b in blocks {
            let label = match b {
                Block::Equation { label: Some(l), .. } => {
                    self.equation_refs.insert(l.clone());
                    Some(l.clone())
                }
                Block::Heading { label, .. } | Block::Equation { label, .. } => label.clone(),
                Block::Figure(f) => {
                    self.collect_labels(&f.body);
                    f.label.clone()
                }
                Block::List { items, .. } => {
                    for i in items {
                        self.collect_labels(i);
                    }
                    None
                }
                _ => None,
            };
            if let Some(l) = label {
                let name = self.doc.ids().map(|i| i.label_name(&l)).unwrap_or_else(|_| format!("_Ref{l}"));
                self.labels.insert(l, name);
            }
        }
    }

    fn id(&mut self) -> u64 {
        self.doc.ids().map(|i| i.next()).unwrap_or(1)
    }

    fn rev_attrs(&mut self) -> Option<String> {
        let r = self.o.track.clone()?;
        let id = self.id();
        Some(format!(" w:id=\"{id}\" w:author=\"{}\" w:date=\"{}\"", esc_attr(&r.author), r.date))
    }

    /// Wraps run-level content in w:ins when tracking.
    fn tracked(&mut self, runs: String) -> String {
        match self.rev_attrs() {
            Some(a) if !runs.is_empty() => format!("<w:ins{a}>{runs}</w:ins>"),
            _ => runs,
        }
    }

    fn mark_rpr(&mut self) -> String {
        match self.rev_attrs() {
            Some(a) => format!("<w:rPr><w:ins{a}/></w:rPr>"),
            None => String::new(),
        }
    }

    fn para(&mut self, style: Option<&str>, extra_ppr: &str, inner: &str, out: &mut String) {
        let mark = self.mark_rpr();
        out.push_str("<w:p><w:pPr>");
        if let Some(s) = style {
            out.push_str(&format!("<w:pStyle w:val=\"{}\"/>", esc_attr(s)));
        }
        out.push_str(extra_ppr);
        out.push_str(&mark);
        out.push_str("</w:pPr>");
        out.push_str(inner);
        out.push_str("</w:p>");
    }

    fn resolve_para_style(&mut self, s: &str) -> Option<String> {
        let roles = &self.o.roles;
        let by_role = match s {
            "body" => roles.body.clone(),
            "caption" => roles.caption.clone(),
            "figure" => roles.figure.clone(),
            "table_text" => roles.table_text.clone(),
            _ => None,
        };
        by_role.or_else(|| {
            self.doc.styles().ok()?.resolve(s, Some(StyleKind::Paragraph)).map(|x| x.id.clone())
        })
    }

    fn block(&mut self, b: &Block, out: &mut String, depth: usize) -> Result<(), Error> {
        match b {
            Block::Paragraph { inlines, style, align } => {
                let st = match style {
                    Some(s) => Some(self.resolve_para_style(s).ok_or_else(|| Error::Invalid(format!("unknown paragraph style {s:?}")))?),
                    None => self.o.roles.body.clone(),
                };
                let jc = align.map(jc).unwrap_or_default();
                let inner = self.inlines(inlines)?;
                self.para(st.as_deref(), &jc, &inner, out);
            }
            Block::Heading { level, inlines, label } => {
                let lvl = (*level).clamp(1, 9) as usize;
                let st = self.o.roles.headings.get(lvl - 1).or(self.o.roles.headings.last()).cloned();
                let mut inner = self.inlines(inlines)?;
                if let Some(l) = label {
                    inner = self.bookmark(l, &inner);
                }
                self.para(st.as_deref(), "<w:keepNext/>", &inner, out);
            }
            Block::List { ordered, items } => self.list(*ordered, items, out, depth)?,
            Block::Table(t) => self.table(t, out)?,
            Block::Figure(f) => self.figure(f, out)?,
            Block::Image(img) => {
                let inner = self.image(img)?;
                let st = self.o.roles.figure.clone();
                self.para(st.as_deref(), "<w:keepNext/><w:jc w:val=\"center\"/>", &inner, out);
            }
            Block::Equation { math, label } => {
                let st = self.o.roles.equation.clone().or(self.o.roles.body.clone());
                match label {
                    None => {
                        let m = format!("<m:oMathPara><m:oMath>{}</m:oMath></m:oMathPara>", omml(math));
                        let inner = self.tracked(m);
                        self.para(st.as_deref(), "", &inner, out);
                    }
                    Some(l) => {
                        // A numbered equation: centred formula, number at the
                        // right margin, the number "(n)" bookmarked for REF.
                        let w = self.text_width;
                        let tabs = format!("<w:tabs><w:tab w:val=\"center\" w:pos=\"{}\"/><w:tab w:val=\"right\" w:pos=\"{w}\"/></w:tabs>", w / 2);
                        let tab = self.tracked("<w:r><w:tab/></w:r>".into());
                        let m = self.tracked(format!("<m:oMath>{}</m:oMath>", omml(math)));
                        let open = self.tracked("<w:r><w:t>(</w:t></w:r>".into());
                        let word = seq_id(&self.o.roles.equation_word);
                        let seq = self.field(&format!(" SEQ {word} \\* ARABIC "), "1");
                        let close = self.tracked("<w:r><w:t>)</w:t></w:r>".into());
                        let num = self.bookmark(l, &format!("{open}{seq}{close}"));
                        let tab2 = self.tracked("<w:r><w:tab/></w:r>".into());
                        self.para(st.as_deref(), &tabs, &format!("{tab}{m}{tab2}{num}"), out);
                        self.equation_labels.insert(l.clone());
                    }
                }
            }
            Block::PageBreak => {
                let r = self.tracked("<w:r><w:br w:type=\"page\"/></w:r>".into());
                self.para(None, "", &r, out);
            }
            Block::Raw { xml } => out.push_str(xml),
        }
        Ok(())
    }

    fn bookmark(&mut self, label: &str, inner: &str) -> String {
        let name = self.labels.get(label).cloned().unwrap_or_else(|| label.to_string());
        let id = self.id();
        format!("<w:bookmarkStart w:id=\"{id}\" w:name=\"{}\"/>{inner}<w:bookmarkEnd w:id=\"{id}\"/>", esc_attr(&name))
    }

    fn list(&mut self, ordered: bool, items: &[Vec<Block>], out: &mut String, depth: usize) -> Result<(), Error> {
        let style = if ordered { self.o.roles.list_number.clone() } else { self.o.roles.list_bullet.clone() };
        let base_num = style.as_deref().and_then(|s| self.doc.styles().ok()?.num_id(s));
        // A numbered list starts at 1: its own w:num over the style's definition.
        let num = match (&base_num, ordered && depth == 0) {
            (Some(n), true) => Some(self.restart(n)?),
            (Some(n), false) => Some(n.clone()),
            (None, _) => None,
        };
        for item in items {
            let mut first = true;
            for b in item {
                match b {
                    Block::Paragraph { inlines, .. } if first => {
                        let inner = self.inlines(inlines)?;
                        let numpr = match &num {
                            Some(n) => format!("<w:numPr><w:ilvl w:val=\"{depth}\"/><w:numId w:val=\"{n}\"/></w:numPr>"),
                            None => String::new(),
                        };
                        self.para(style.as_deref(), &numpr, &inner, out);
                    }
                    Block::List { ordered, items } => self.list(*ordered, items, out, depth + 1)?,
                    other => self.block(other, out, depth + 1)?,
                }
                first = false;
            }
        }
        Ok(())
    }

    fn restart(&mut self, num: &str) -> Result<String, Error> {
        let t = self.doc.pkg.xml("word/numbering.xml")?;
        let root = t.document_element();
        let abs = t
            .elements(root)
            .find(|&c| t.is(c, "w:num") && t.attr(c, "w:numId").as_deref() == Some(num))
            .and_then(|n| t.child(n, "w:abstractNumId"))
            .and_then(|a| t.attr(a, "w:val"))
            .ok_or_else(|| Error::Invalid(format!("numbering {num} has no definition")))?;
        self.next_num += 1;
        let id = self.next_num;
        self.new_nums.push(format!(
            "<w:num w:numId=\"{id}\"><w:abstractNumId w:val=\"{abs}\"/><w:lvlOverride w:ilvl=\"0\"><w:startOverride w:val=\"1\"/></w:lvlOverride></w:num>"
        ));
        Ok(id.to_string())
    }

    fn figure(&mut self, f: &Figure, out: &mut String) -> Result<(), Error> {
        let (word, above) = match f.kind {
            FigureKind::Figure => (self.o.roles.figure_word.clone(), false),
            FigureKind::Table => (self.o.roles.table_word.clone(), self.o.roles.table_caption_above),
            FigureKind::Equation => (self.o.roles.equation_word.clone(), false),
        };
        let mut cap = String::new();
        if let Some(c) = &f.caption {
            let label_runs = self.tracked(format!("<w:r><w:t xml:space=\"preserve\">{} </w:t></w:r>", esc(&word)));
            let seq = self.field(&format!(" SEQ {} \\* ARABIC ", seq_id(&word)), "1");
            let mut num = format!("{label_runs}{seq}");
            if let Some(l) = &f.label {
                num = self.bookmark(l, &num);
            }
            let sep = self.tracked(format!("<w:r><w:t xml:space=\"preserve\">{}</w:t></w:r>", esc(&self.o.roles.caption_separator)));
            let text = self.inlines(c)?;
            let st = self.o.roles.caption.clone();
            let keep = if above { "<w:keepNext/>" } else { "" };
            self.para(st.as_deref(), keep, &format!("{num}{sep}{text}"), &mut cap);
        }
        if above {
            out.push_str(&cap);
        }
        for b in &f.body {
            self.block(b, out, 0)?;
        }
        if !above {
            out.push_str(&cap);
        } else if f.body.last().is_some_and(|b| matches!(b, Block::Table(_))) {
            // Word merges two adjacent tables; a paragraph keeps them apart.
            let st = self.o.roles.body.clone();
            self.para(st.as_deref(), "", "", out);
        }
        Ok(())
    }

    /// A complex field with a cached result.
    fn field(&mut self, code: &str, result: &str) -> String {
        let x = format!(
            "<w:r><w:fldChar w:fldCharType=\"begin\"/></w:r><w:r><w:instrText xml:space=\"preserve\">{}</w:instrText></w:r><w:r><w:fldChar w:fldCharType=\"separate\"/></w:r><w:r><w:t>{}</w:t></w:r><w:r><w:fldChar w:fldCharType=\"end\"/></w:r>",
            esc(code),
            esc(result)
        );
        self.tracked(x)
    }

    fn table(&mut self, tb: &Table, out: &mut String) -> Result<(), Error> {
        let grid = grid(&tb.rows)?;
        let ncol = grid.width;
        let fr: Vec<f32> = if tb.widths.len() == ncol { tb.widths.clone() } else { vec![1.0; ncol] };
        let total: f32 = fr.iter().sum();
        let widths: Vec<i64> = fr.iter().map(|f| (self.text_width as f32 * f / total).round() as i64).collect();
        let style = match &tb.style {
            Some(s) => {
                let def = self.o.table_styles.iter().find(|d| &d.name == s).cloned();
                Some(match def {
                    Some(d) => self.doc.ensure_table_style(&TableStyle::Defined(d))?,
                    None => self.doc.ensure_table_style(&TableStyle::Named(s.clone()))?,
                })
            }
            None => self.o.roles.table_style.clone(),
        };
        out.push_str("<w:tbl><w:tblPr>");
        if let Some(s) = &style {
            out.push_str(&format!("<w:tblStyle w:val=\"{}\"/>", esc_attr(s)));
        }
        out.push_str("<w:tblW w:w=\"5000\" w:type=\"pct\"/><w:tblLook w:val=\"04A0\" w:firstRow=\"1\" w:lastRow=\"0\" w:firstColumn=\"0\" w:lastColumn=\"0\" w:noHBand=\"0\" w:noVBand=\"1\"/></w:tblPr><w:tblGrid>");
        for w in &widths {
            out.push_str(&format!("<w:gridCol w:w=\"{w}\"/>"));
        }
        out.push_str("</w:tblGrid>");
        for (ri, row) in grid.rows.iter().enumerate() {
            let mut trpr = String::new();
            if ri < tb.header_rows {
                trpr.push_str("<w:tblHeader/>");
            }
            if let Some(a) = self.rev_attrs() {
                trpr.push_str(&format!("<w:ins{a}/>"));
            }
            out.push_str("<w:tr>");
            if !trpr.is_empty() {
                out.push_str(&format!("<w:trPr>{trpr}</w:trPr>"));
            }
            for gc in row {
                let w: i64 = widths[gc.col..gc.col + gc.span].iter().sum();
                out.push_str(&format!("<w:tc><w:tcPr><w:tcW w:w=\"{w}\" w:type=\"dxa\"/>"));
                if gc.span > 1 {
                    out.push_str(&format!("<w:gridSpan w:val=\"{}\"/>", gc.span));
                }
                match gc.merge {
                    Merge::None => {}
                    Merge::Restart => out.push_str("<w:vMerge w:val=\"restart\"/>"),
                    Merge::Continue => out.push_str("<w:vMerge/>"),
                }
                let cell = gc.cell.map(|(r, c)| &tb.rows[r][c]);
                if let Some(f) = cell.and_then(|c| c.fill.as_ref()) {
                    out.push_str(&format!("<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"{}\"/>", esc_attr(f)));
                }
                out.push_str("</w:tcPr>");
                let col_align = tb.align.get(gc.col).copied().flatten();
                match cell {
                    Some(c) if gc.merge != Merge::Continue && !c.blocks.is_empty() => {
                        let a = c.align.or(col_align);
                        for b in &c.blocks {
                            match (b, a) {
                                (Block::Paragraph { inlines, style, align: None }, Some(a)) => {
                                    let b2 = Block::Paragraph { inlines: inlines.clone(), style: style.clone(), align: Some(a) };
                                    self.cell_block(&b2, out)?;
                                }
                                _ => self.cell_block(b, out)?,
                            }
                        }
                    }
                    _ => {
                        let st = self.o.roles.table_text.clone();
                        self.para(st.as_deref(), "", "", out);
                    }
                }
                out.push_str("</w:tc>");
            }
            out.push_str("</w:tr>");
        }
        out.push_str("</w:tbl>");
        Ok(())
    }

    fn cell_block(&mut self, b: &Block, out: &mut String) -> Result<(), Error> {
        match b {
            Block::Paragraph { inlines, style: None, align } => {
                let st = self.o.roles.table_text.clone();
                let inner = self.inlines(inlines)?;
                self.para(st.as_deref(), &align.map(jc).unwrap_or_default(), &inner, out);
                Ok(())
            }
            other => self.block(other, out, 0),
        }
    }

    fn inlines(&mut self, xs: &[Inline]) -> Result<String, Error> {
        let mut s = String::new();
        for x in xs {
            let r = match x {
                Inline::Text { text, marks } => {
                    let rpr = self.rpr(marks);
                    self.tracked(text_run(&rpr, text))
                }
                Inline::LineBreak => self.tracked("<w:r><w:br/></w:r>".into()),
                Inline::Footnote { blocks } => self.footnote(blocks)?,
                Inline::Ref { target } => {
                    let bm = match self.labels.get(target) {
                        Some(b) => b.clone(),
                        None => {
                            let body = self.doc.body();
                            let known = crate::document::bookmarks(self.doc.tree(), body);
                            let by_label = crate::document::label_bookmark(target);
                            if known.contains_key(&by_label) {
                                by_label
                            } else if known.contains_key(target) {
                                target.clone()
                            } else {
                                return Err(Error::Invalid(format!("reference to unknown label {target:?}")));
                            }
                        }
                    };
                    let f = self.field(&format!(" REF {bm} \\h "), "?");
                    if self.equation_refs.contains(target) {
                        let word = self.tracked(format!("<w:r><w:t xml:space=\"preserve\">{} </w:t></w:r>", esc(&self.o.roles.equation_word)));
                        format!("{word}{f}")
                    } else {
                        f
                    }
                }
                Inline::Link { url, inlines } => {
                    let rid = self.doc.pkg.add_rel(&self.doc.main.clone(), REL_HYPERLINK, url, true)?;
                    let mut marks_inner = String::new();
                    for i in inlines {
                        if let Inline::Text { text, marks } = i {
                            let mut m = marks.clone();
                            if m.style.is_none() {
                                m.style = self.o.roles.hyperlink.clone();
                            }
                            let rpr = self.rpr(&m);
                            marks_inner.push_str(&text_run(&rpr, text));
                        }
                    }
                    let runs = self.tracked(marks_inner);
                    format!("<w:hyperlink r:id=\"{rid}\" w:history=\"1\">{runs}</w:hyperlink>")
                }
                Inline::Math { math } => self.tracked(format!("<m:oMath>{}</m:oMath>", omml(math))),
                Inline::Field { code, result } => self.field(code, result),
                Inline::Image(img) => self.image(img)?,
                Inline::Raw { xml } => xml.clone(),
            };
            s.push_str(&r);
        }
        Ok(s)
    }

    fn rpr(&mut self, m: &Marks) -> String {
        let mut r = String::new();
        let cstyle = m.style.as_ref().and_then(|s| {
            self.doc.styles().ok()?.resolve(s, Some(StyleKind::Character)).map(|x| x.id.clone()).or(Some(s.clone()))
        });
        let cstyle = cstyle.or_else(|| if m.code { self.o.roles.code.clone() } else { None });
        if let Some(s) = cstyle {
            r.push_str(&format!("<w:rStyle w:val=\"{}\"/>", esc_attr(&s)));
        }
        let font = m.font.clone().or_else(|| (m.code && self.o.roles.code.is_none()).then(|| "Consolas".to_string()));
        if let Some(f) = font {
            let f = esc_attr(&f);
            r.push_str(&format!("<w:rFonts w:ascii=\"{f}\" w:hAnsi=\"{f}\" w:cs=\"{f}\"/>"));
        }
        if m.bold {
            r.push_str("<w:b/><w:bCs/>");
        }
        if m.italic {
            r.push_str("<w:i/><w:iCs/>");
        }
        if m.small_caps {
            r.push_str("<w:smallCaps/>");
        }
        if m.strike {
            r.push_str("<w:strike/>");
        }
        if let Some(c) = &m.color {
            r.push_str(&format!("<w:color w:val=\"{}\"/>", esc_attr(c)));
        }
        if let Some(s) = m.size_pt {
            let hp = (s * 2.0).round() as i32;
            r.push_str(&format!("<w:sz w:val=\"{hp}\"/><w:szCs w:val=\"{hp}\"/>"));
        }
        if let Some(h) = &m.highlight {
            // w:highlight only knows named colours; shading takes any colour.
            r.push_str(&format!("<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"{}\"/>", esc_attr(h)));
        }
        if m.underline {
            r.push_str("<w:u w:val=\"single\"/>");
        }
        if m.sub {
            r.push_str("<w:vertAlign w:val=\"subscript\"/>");
        } else if m.sup {
            r.push_str("<w:vertAlign w:val=\"superscript\"/>");
        }
        if r.is_empty() { r } else { format!("<w:rPr>{r}</w:rPr>") }
    }

    fn footnote(&mut self, blocks: &[Block]) -> Result<String, Error> {
        self.next_footnote += 1;
        let id = self.next_footnote;
        let refst = self.o.roles.footnote_reference.clone();
        let rs = refst.as_deref().map(|s| format!("<w:rPr><w:rStyle w:val=\"{s}\"/></w:rPr>")).unwrap_or_else(|| "<w:rPr><w:vertAlign w:val=\"superscript\"/></w:rPr>".into());
        let text_st = self.o.roles.footnote_text.clone();
        let mut body = String::new();
        for (i, b) in blocks.iter().enumerate() {
            let inl = match b {
                Block::Paragraph { inlines, .. } => inlines.clone(),
                _ => vec![],
            };
            let mut inner = String::new();
            if i == 0 {
                inner.push_str(&format!("<w:r>{rs}<w:footnoteRef/></w:r><w:r><w:t xml:space=\"preserve\"> </w:t></w:r>"));
            }
            let runs = self.inlines(&inl)?;
            inner.push_str(&runs);
            body.push_str("<w:p><w:pPr>");
            if let Some(s) = &text_st {
                body.push_str(&format!("<w:pStyle w:val=\"{s}\"/>"));
            }
            body.push_str("</w:pPr>");
            body.push_str(&inner);
            body.push_str("</w:p>");
        }
        self.footnotes.push(format!("<w:footnote w:id=\"{id}\">{body}</w:footnote>"));
        Ok(self.tracked(format!("<w:r>{rs}<w:footnoteReference w:id=\"{id}\"/></w:r>")))
    }

    fn image(&mut self, img: &Image) -> Result<String, Error> {
        let ext = img.name.rsplit('.').next().unwrap_or("png").to_ascii_lowercase();
        let (ctype, ext) = match ext.as_str() {
            "png" => ("image/png", "png"),
            "jpg" | "jpeg" => ("image/jpeg", "jpeg"),
            "gif" => ("image/gif", "gif"),
            "emf" => ("image/x-emf", "emf"),
            other => return Err(Error::Unsupported(format!("image format {other}; use png, jpeg, gif or emf"))),
        };
        self.images += 1;
        let mut n = self.images;
        let mut part = format!("word/media/tabellion{n}.{ext}");
        while self.doc.pkg.has(&part) {
            n += 1;
            part = format!("word/media/tabellion{n}.{ext}");
        }
        self.doc.pkg.put(&part, img.data.clone());
        self.doc.pkg.ensure_content_type(None, Some(ext), ctype)?;
        let main = self.doc.main.clone();
        let rid = self.doc.pkg.add_rel(&main, REL_IMAGE, &part["word/".len()..], false)?;
        let (mut cx, mut cy) = (img.width, img.height);
        if cx <= 0 || cy <= 0 {
            let (pw, ph, dpi) = png_size(&img.data).unwrap_or((800, 600, 96.0));
            cx = (pw as f64 / dpi * 914400.0) as i64;
            cy = (ph as f64 / dpi * 914400.0) as i64;
        }
        let max = self.text_width * 635;
        if cx > max {
            cy = cy * max / cx;
            cx = max;
        }
        let did = self.id();
        let x = format!(
            "<w:r><w:drawing><wp:inline distT=\"0\" distB=\"0\" distL=\"0\" distR=\"0\"><wp:extent cx=\"{cx}\" cy=\"{cy}\"/><wp:docPr id=\"{did}\" name=\"Picture {did}\" descr=\"{alt}\"/><wp:cNvGraphicFramePr><a:graphicFrameLocks xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" noChangeAspect=\"1\"/></wp:cNvGraphicFramePr><a:graphic xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\"><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/picture\"><pic:pic xmlns:pic=\"http://schemas.openxmlformats.org/drawingml/2006/picture\"><pic:nvPicPr><pic:cNvPr id=\"{did}\" name=\"{name}\"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed=\"{rid}\"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r>",
            alt = esc_attr(&img.alt),
            name = esc_attr(&img.name),
        );
        Ok(self.tracked(x))
    }

    fn finish(&mut self) -> Result<(), Error> {
        if !self.footnotes.is_empty() {
            self.ensure_footnotes()?;
            let t = self.doc.pkg.xml("word/footnotes.xml")?;
            let root = t.document_element();
            let xml = self.footnotes.concat();
            for n in t.parse_fragment(&xml).map_err(|e| Error::Xml("footnotes".into(), e))? {
                t.append(root, n);
            }
        }
        if !self.new_nums.is_empty() {
            let t = self.doc.pkg.xml("word/numbering.xml")?;
            let root = t.document_element();
            let xml = self.new_nums.concat();
            let last = t.elements(root).filter(|&c| t.is(c, "w:num")).last();
            let nodes = t.parse_fragment(&xml).map_err(|e| Error::Xml("numbering".into(), e))?;
            for &n in nodes.iter().rev() {
                match last {
                    Some(l) => t.insert_after(l, n),
                    None => t.append(root, n),
                }
            }
        }
        Ok(())
    }

    fn ensure_footnotes(&mut self) -> Result<(), Error> {
        if self.doc.pkg.has("word/footnotes.xml") {
            return Ok(());
        }
        self.doc.pkg.put_xml("word/footnotes.xml", concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n",
            "<w:footnotes xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" xmlns:m=\"http://schemas.openxmlformats.org/officeDocument/2006/math\">",
            "<w:footnote w:type=\"separator\" w:id=\"-1\"><w:p><w:pPr><w:spacing w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/></w:pPr><w:r><w:separator/></w:r></w:p></w:footnote>",
            "<w:footnote w:type=\"continuationSeparator\" w:id=\"0\"><w:p><w:pPr><w:spacing w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/></w:pPr><w:r><w:continuationSeparator/></w:r></w:p></w:footnote>",
            "</w:footnotes>"))?;
        self.doc.pkg.ensure_content_type(Some("word/footnotes.xml"), None, "application/vnd.openxmlformats-officedocument.wordprocessingml.footnotes+xml")?;
        let main = self.doc.main.clone();
        self.doc.pkg.add_rel(&main, REL_FOOTNOTES, "footnotes.xml", false)?;
        self.next_footnote = self.next_footnote.max(0);
        Ok(())
    }
}

fn jc(a: Align) -> String {
    let v = match a {
        Align::Left => "left",
        Align::Center => "center",
        Align::Right => "right",
        Align::Justify => "both",
    };
    format!("<w:jc w:val=\"{v}\"/>")
}

fn seq_id(word: &str) -> String {
    word.chars().filter(|c| c.is_alphanumeric()).collect()
}

fn text_run(rpr: &str, text: &str) -> String {
    let mut s = format!("<w:r>{rpr}");
    let mut buf = String::new();
    let flush = |buf: &mut String, s: &mut String| {
        if !buf.is_empty() {
            s.push_str(&format!("<w:t xml:space=\"preserve\">{}</w:t>", esc(buf)));
            buf.clear();
        }
    };
    for ch in text.chars() {
        match ch {
            '\t' => {
                flush(&mut buf, &mut s);
                s.push_str("<w:tab/>");
            }
            '\n' => {
                flush(&mut buf, &mut s);
                s.push_str("<w:br/>");
            }
            '\u{2011}' => {
                flush(&mut buf, &mut s);
                s.push_str("<w:noBreakHyphen/>");
            }
            '\u{00AD}' => {
                flush(&mut buf, &mut s);
                s.push_str("<w:softHyphen/>");
            }
            c => buf.push(c),
        }
    }
    flush(&mut buf, &mut s);
    s.push_str("</w:r>");
    s
}

fn png_size(d: &[u8]) -> Option<(u32, u32, f64)> {
    if d.len() < 24 || &d[1..4] != b"PNG" {
        return None;
    }
    let be = |i: usize| u32::from_be_bytes([d[i], d[i + 1], d[i + 2], d[i + 3]]);
    let (w, h) = (be(16), be(20));
    let mut dpi = 96.0;
    if let Some(i) = d.windows(4).position(|x| x == b"pHYs") {
        let ppm = be(i + 4);
        if ppm > 0 && d.get(i + 12) == Some(&1) {
            dpi = ppm as f64 * 0.0254;
        }
    }
    Some((w, h, dpi))
}

// ---- table grid ---------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Merge {
    None,
    Restart,
    Continue,
}

struct GridCell {
    col: usize,
    span: usize,
    merge: Merge,
    /// Index into the source rows; None for a continuation.
    cell: Option<(usize, usize)>,
}

struct Grid {
    width: usize,
    rows: Vec<Vec<GridCell>>,
}

/// Places HTML-style rows on Word's grid. Word needs a cell where a row span
/// passes, so continuations become cells with `vMerge`.
fn grid(rows: &[Vec<Cell>]) -> Result<Grid, Error> {
    let mut out: Vec<Vec<GridCell>> = vec![];
    // Open row spans: (column, colspan, rows left).
    let mut open: Vec<(usize, usize, usize)> = vec![];
    let mut width = None;
    for (r, row) in rows.iter().enumerate() {
        let mut cells = vec![];
        let mut col = 0;
        let mut next_open = vec![];
        let mut it = row.iter().enumerate().peekable();
        loop {
            if let Some(&(c, span, left)) = open.iter().find(|o| o.0 == col) {
                cells.push(GridCell { col: c, span, merge: Merge::Continue, cell: None });
                if left > 1 {
                    next_open.push((c, span, left - 1));
                }
                col += span;
                continue;
            }
            let Some((ci, cell)) = it.next() else { break };
            let span = cell.colspan.max(1);
            let merge = if cell.rowspan > 1 { Merge::Restart } else { Merge::None };
            if cell.rowspan > 1 {
                next_open.push((col, span, cell.rowspan - 1));
            }
            cells.push(GridCell { col, span, merge, cell: Some((r, ci)) });
            col += span;
        }
        open = next_open;
        match width {
            None => width = Some(col),
            Some(w) if w != col => {
                return Err(Error::Invalid(format!("table row {} has {col} columns, the first row has {w}", r + 1)))
            }
            _ => {}
        }
        out.push(cells);
    }
    if !open.is_empty() {
        return Err(Error::Invalid("a row span reaches past the end of the table".into()));
    }
    Ok(Grid { width: width.unwrap_or(0), rows: out })
}

// ---- Office Math ----------------------------------------------------------------

pub fn omml(m: &Math) -> String {
    let arg = |tag: &str, x: &Math| format!("<m:{tag}>{}</m:{tag}>", omml(x));
    match m {
        Math::Row { items } => items.iter().map(omml).collect(),
        Math::Run { text, normal } => {
            let sty = if *normal { "<m:rPr><m:sty m:val=\"p\"/></m:rPr>" } else { "" };
            format!(
                "<m:r>{sty}<w:rPr><w:rFonts w:ascii=\"Cambria Math\" w:hAnsi=\"Cambria Math\"/></w:rPr><m:t xml:space=\"preserve\">{}</m:t></m:r>",
                esc(text)
            )
        }
        Math::Frac { num, den } => format!("<m:f>{}{}</m:f>", arg("num", num), arg("den", den)),
        Math::Scripts { base, sub, sup } => match (sub, sup) {
            (Some(b), Some(t)) => format!("<m:sSubSup>{}{}{}</m:sSubSup>", arg("e", base), arg("sub", b), arg("sup", t)),
            (Some(b), None) => format!("<m:sSub>{}{}</m:sSub>", arg("e", base), arg("sub", b)),
            (None, Some(t)) => format!("<m:sSup>{}{}</m:sSup>", arg("e", base), arg("sup", t)),
            (None, None) => omml(base),
        },
        Math::Root { index, radicand } => match index {
            Some(i) => format!("<m:rad>{}{}</m:rad>", arg("deg", i), arg("e", radicand)),
            None => format!("<m:rad><m:radPr><m:degHide m:val=\"1\"/></m:radPr><m:deg/>{}</m:rad>", arg("e", radicand)),
        },
        Math::Delim { open, close, body } => format!(
            "<m:d><m:dPr><m:begChr m:val=\"{}\"/><m:endChr m:val=\"{}\"/></m:dPr>{}</m:d>",
            esc_attr(open),
            esc_attr(close),
            arg("e", body)
        ),
        Math::Accent { accent, base } => {
            format!("<m:acc><m:accPr><m:chr m:val=\"{}\"/></m:accPr>{}</m:acc>", esc_attr(accent), arg("e", base))
        }
        Math::Nary { op, sub, sup, body } => {
            let hide = |x: &Option<Box<Math>>, tag: &str| if x.is_none() { format!("<m:{tag}Hide m:val=\"1\"/>") } else { String::new() };
            format!(
                "<m:nary><m:naryPr><m:chr m:val=\"{}\"/><m:limLoc m:val=\"undOvr\"/>{}{}</m:naryPr><m:sub>{}</m:sub><m:sup>{}</m:sup>{}</m:nary>",
                esc_attr(op),
                hide(sub, "sub"),
                hide(sup, "sup"),
                sub.as_deref().map(omml).unwrap_or_default(),
                sup.as_deref().map(omml).unwrap_or_default(),
                arg("e", body)
            )
        }
        Math::Matrix { rows } => {
            let mut s = String::from("<m:m>");
            for r in rows {
                s.push_str("<m:mr>");
                for c in r {
                    s.push_str(&arg("e", c));
                }
                s.push_str("</m:mr>");
            }
            s.push_str("</m:m>");
            s
        }
        Math::Image { .. } => String::new(),
    }
}
