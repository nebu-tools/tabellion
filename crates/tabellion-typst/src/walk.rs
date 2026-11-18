//! Evaluated Typst content -> the engine's content model.
//!
//! The walk sees content before realisation: markup elements as Typst
//! builds them, set rules as style chains, show rules as recipes. Set rules
//! for text (weight, style, fill, size) are read from the style chain. Show
//! rules are not run; each one is reported, because its effect would be
//! missing in Word.

use typst::foundations::Repr as _;
use nebu_docx::content::{Align, Block, Cell, Figure, FigureKind, Image, Inline, Marks, Table};
use nebu_docx::table_style::TableStyleDef;
use typst::foundations::{Content, StyleChain, StyledElem, Value};
use typst::text::{FontStyle, TextElem};
use typst::World;

use crate::world::FragmentWorld;
use crate::{Fragment, Options};

pub struct Walker<'a> {
    world: &'a FragmentWorld,
    opts: &'a Options,
    blocks: Vec<Block>,
    para: Vec<Inline>,
    warnings: Vec<String>,
    table_styles: Vec<TableStyleDef>,
    next_table_style: Option<String>,
}

fn field(c: &Content, name: &str) -> Option<Value> {
    c.get_by_name(name).ok()
}

fn field_in(c: &Content, name: &str, styles: StyleChain) -> Option<Value> {
    let id = c.elem().field_id(name)?;
    c.get(id, Some(styles)).ok()
}

fn content_field(c: &Content, name: &str) -> Option<Content> {
    match field(c, name)? {
        Value::Content(x) => Some(x),
        Value::Str(s) => Some(TextElem::packed(s.as_str())),
        Value::None => None,
        other => Some(TextElem::packed(other.repr())),
    }
}

impl<'a> Walker<'a> {
    pub fn new(world: &'a FragmentWorld, opts: &'a Options) -> Walker<'a> {
        Walker {
            world,
            opts,
            blocks: vec![],
            para: vec![],
            warnings: vec![],
            table_styles: vec![],
            next_table_style: None,
        }
    }

    fn sub(&self) -> Walker<'a> {
        Walker::new(self.world, self.opts)
    }

    fn absorb(&mut self, w: Walker<'a>) -> (Vec<Block>, Vec<Inline>) {
        self.warnings.extend(w.warnings);
        self.table_styles.extend(w.table_styles);
        (w.blocks, w.para)
    }

    fn warn(&mut self, c: &Content, what: &str) {
        let at = self.world.span_text(c.span()).map(|s| format!(" (`{}`)", s.chars().take(50).collect::<String>().replace('\n', " "))).unwrap_or_default();
        let msg = format!("{what}{at}");
        if !self.warnings.contains(&msg) {
            self.warnings.push(msg);
        }
    }

    pub fn run(&mut self, c: Content) {
        let lib = self.world.library();
        let styles = StyleChain::new(&lib.styles);
        self.node(&c, styles, &Marks::default());
        self.flush();
    }

    pub fn finish(self) -> Fragment {
        Fragment { blocks: self.blocks, table_styles: self.table_styles, warnings: self.warnings }
    }

    fn flush(&mut self) {
        // Leading and trailing spaces carry no meaning in a Word paragraph.
        while matches!(self.para.last(), Some(Inline::Text { text, .. }) if text.trim().is_empty()) {
            self.para.pop();
        }
        while matches!(self.para.first(), Some(Inline::Text { text, .. }) if text.trim().is_empty()) {
            self.para.remove(0);
        }
        if !self.para.is_empty() {
            let inlines = merge_text(std::mem::take(&mut self.para));
            self.blocks.push(Block::Paragraph { inlines, style: None, align: None });
        }
    }

    fn push_block(&mut self, b: Block) {
        self.flush();
        self.blocks.push(b);
    }

    fn text(&mut self, s: &str, styles: StyleChain, marks: &Marks) {
        let mut m = marks.clone();
        if styles.get(TextElem::weight).to_number() >= 600 {
            m.bold = true;
        }
        if styles.get(TextElem::style) != FontStyle::Normal {
            m.italic = true;
        }
        if let Some(hex) = crate::value::paint_hex(styles.get_ref(TextElem::fill)) {
            if hex != "000000" {
                m.color = Some(hex);
            }
        }
        let size = styles.resolve(TextElem::size).to_pt();
        if (size - 11.0).abs() > 0.01 {
            m.size_pt = Some(size as f32);
        }
        self.para.push(Inline::Text { text: s.to_string(), marks: m });
    }

    /// Inline content of `c` as a list of inlines.
    fn inlines_of(&mut self, c: &Content, styles: StyleChain, marks: &Marks) -> Vec<Inline> {
        let mut w = self.sub();
        w.node(c, styles, marks);
        let (blocks, mut para) = self.absorb(w);
        if !blocks.is_empty() {
            // A paragraph break inside inline-only content: keep the text.
            let mut out = vec![];
            for b in blocks {
                if let Block::Paragraph { inlines, .. } = b {
                    if !out.is_empty() {
                        out.push(Inline::LineBreak);
                    }
                    out.extend(inlines);
                }
            }
            out.append(&mut para);
            return merge_text(out);
        }
        trim(merge_text(para))
    }

    /// Block content of `c`.
    fn blocks_of(&mut self, c: &Content, styles: StyleChain) -> Vec<Block> {
        let mut w = self.sub();
        w.node(c, styles, &Marks::default());
        w.flush();
        let (blocks, _) = self.absorb(w);
        blocks
    }

    fn node(&mut self, c: &Content, styles: StyleChain, marks: &Marks) {
        let name = c.elem().name();
        match name {
            "sequence" => {
                if let Some(Value::Array(kids)) = field(c, "children") {
                    let kids: Vec<Content> = kids.into_iter().filter_map(|v| v.cast::<Content>().ok()).collect();
                    let mut i = 0;
                    while i < kids.len() {
                        let k = &kids[i];
                        if k.is::<typst::model::ListItem>() || k.is::<typst::model::EnumItem>() {
                            // Markup lists arrive as loose items; consecutive
                            // items of one kind form one list. Both kinds are
                            // named "item", so compare the element itself.
                            let ordered = k.is::<typst::model::EnumItem>();
                            let kind = k.elem();
                            let mut items = vec![];
                            while i < kids.len() {
                                let n = kids[i].elem().name();
                                if n == "space" || n == "parbreak" {
                                    if kids.get(i + 1).map(|x| x.elem()) == Some(kind) {
                                        i += 1;
                                        continue;
                                    }
                                    break;
                                }
                                if kids[i].elem() != kind {
                                    break;
                                }
                                let body = content_field(&kids[i], "body").unwrap_or_default();
                                items.push(self.blocks_of(&body, styles));
                                i += 1;
                            }
                            self.push_block(Block::List { ordered, items });
                            continue;
                        }
                        self.node(k, styles, marks);
                        i += 1;
                    }
                }
            }
            "styled" => {
                let Some(s) = c.to_packed::<StyledElem>() else { return };
                let chain = styles.chain(&s.styles);
                if s.styles.iter().any(|st| st.recipe().is_some()) {
                    self.warn(c, "show rule not applied in Word; its content is kept as written");
                }
                self.node(&s.child, chain, marks);
            }
            "parbreak" => self.flush(),
            "space" => self.para.push(Inline::text(" ")),
            "text" => {
                if let Some(Value::Str(s)) = field(c, "text") {
                    self.text(s.as_str(), styles, marks);
                }
            }
            "linebreak" => self.para.push(Inline::LineBreak),
            "smartquote" => {
                let double = matches!(field_in(c, "double", styles), Some(Value::Bool(true)));
                self.text(if double { "\u{201C}" } else { "\u{2019}" }, styles, marks);
            }
            "strong" | "emph" | "underline" | "strike" | "sub" | "super" | "smallcaps" | "highlight" => {
                let mut m = marks.clone();
                match name {
                    "strong" => m.bold = true,
                    "emph" => m.italic = !m.italic,
                    "underline" => m.underline = true,
                    "strike" => m.strike = true,
                    "sub" => m.sub = true,
                    "super" => m.sup = true,
                    "smallcaps" => m.small_caps = true,
                    _ => {
                        m.highlight = match field_in(c, "fill", styles) {
                            Some(Value::Color(col)) => Some(crate::value::color_hex(&col)),
                            _ => Some("FFFF00".into()),
                        }
                    }
                }
                if let Some(b) = content_field(c, "body") {
                    self.node(&b, styles, &m);
                }
            }
            "raw" => {
                let text = match field(c, "text") {
                    Some(Value::Str(s)) => s.to_string(),
                    _ => String::new(),
                };
                let block = matches!(field_in(c, "block", styles), Some(Value::Bool(true)));
                let m = Marks { code: true, ..marks.clone() };
                if block {
                    self.flush();
                    for line in text.lines() {
                        self.para.push(Inline::Text { text: line.to_string(), marks: m.clone() });
                        self.flush();
                    }
                } else {
                    self.para.push(Inline::Text { text, marks: m });
                }
            }
            "heading" => {
                let level = match field_in(c, "depth", styles).or_else(|| field_in(c, "level", styles)) {
                    Some(Value::Int(n)) => n.clamp(1, 9) as u8,
                    _ => 1,
                };
                let body = content_field(c, "body").unwrap_or_default();
                let inlines = self.inlines_of(&body, styles, &Marks::default());
                let label = c.label().map(|l| l.resolve().as_str().to_string());
                self.push_block(Block::Heading { level, inlines, label });
            }
            "list" | "enum" => {
                let ordered = name == "enum";
                let mut items = vec![];
                if let Some(Value::Array(kids)) = field(c, "children") {
                    for k in kids.into_iter().filter_map(|v| v.cast::<Content>().ok()) {
                        let body = content_field(&k, "body").unwrap_or_default();
                        items.push(self.blocks_of(&body, styles));
                    }
                }
                self.push_block(Block::List { ordered, items });
            }
            "table" => {
                let t = self.table(c, styles);
                self.push_block(Block::Table(t));
            }
            "figure" => self.figure(c, styles),
            "image" => match self.image(c, styles) {
                Some(img) => self.push_block(Block::Image(img)),
                None => self.render(c, "image could not be read; rendered by Typst instead"),
            },
            "footnote" => {
                let body = content_field(c, "body").unwrap_or_default();
                let blocks = self.blocks_of(&body, styles);
                self.para.push(Inline::Footnote { blocks });
            }
            "ref" => {
                if let Some(Value::Label(l)) = field(c, "target") {
                    self.para.push(Inline::Ref { target: l.resolve().as_str().to_string() });
                }
                if field(c, "supplement").is_some() {
                    self.warn(c, "reference supplement replaced by the caption label Word shows");
                }
            }
            "link" => {
                let url = match field(c, "dest") {
                    Some(Value::Str(s)) => Some(s.to_string()),
                    _ => None,
                };
                let body = content_field(c, "body").unwrap_or_default();
                let inlines = self.inlines_of(&body, styles, marks);
                match url {
                    Some(url) => self.para.push(Inline::Link { url, inlines }),
                    None => {
                        self.warn(c, "link to a document location kept as text");
                        self.para.extend(inlines);
                    }
                }
            }
            "equation" => {
                let body = content_field(c, "body").unwrap_or_default();
                let mut warn = vec![];
                let math = crate::math::convert(&body, &mut warn);
                for w in warn {
                    self.warn(c, &w);
                }
                if matches!(field_in(c, "block", styles), Some(Value::Bool(true))) {
                    let label = c.label().map(|l| l.resolve().as_str().to_string());
                    self.push_block(Block::Equation { math, label });
                } else {
                    self.para.push(Inline::Math { math });
                }
            }
            "metadata" => {
                if c.label().map(|l| l.resolve().as_str() == "tabellion-table-style").unwrap_or(false) {
                    if let Some(Value::Dict(d)) = field(c, "value") {
                        if let Some(v) = d.get("tabellion-table-style").ok() {
                            match serde_json::from_value::<TableStyleDef>(crate::value::to_json(v)) {
                                Ok(def) => {
                                    self.next_table_style = Some(def.name.clone());
                                    if !self.table_styles.iter().any(|t| t.name == def.name) {
                                        self.table_styles.push(def);
                                    }
                                }
                                Err(e) => self.warn(c, &format!("table style not understood: {e}")),
                            }
                        }
                    }
                }
            }
            "h" => self.para.push(Inline::text(" ")),
            "v" | "colbreak" => self.warn(c, "vertical spacing left to the paragraph styles"),
            "pagebreak" => self.push_block(Block::PageBreak),
            "box" | "block" | "pad" | "align" => {
                if name == "block" || name == "box" {
                    let styled = ["fill", "stroke", "inset", "width", "height", "radius"].iter().any(|f| field(c, f).is_some());
                    if styled {
                        self.render(c, &format!("{name} with a fill, border or size becomes a picture"));
                        return;
                    }
                }
                let body = content_field(c, "body");
                if let Some(b) = body {
                    if name == "align" {
                        let before = self.blocks.len();
                        self.node(&b, styles, marks);
                        self.flush();
                        let a = match field(c, "alignment").map(|v| v.repr().to_string()).as_deref() {
                            Some("center") | Some("center + horizon") => Some(Align::Center),
                            Some("right") | Some("end") => Some(Align::Right),
                            _ => None,
                        };
                        for blk in &mut self.blocks[before..] {
                            if let Block::Paragraph { align, .. } = blk {
                                *align = a.or(*align);
                            }
                        }
                    } else {
                        self.node(&b, styles, marks);
                    }
                }
            }
            "cite" => {
                let key = match field(c, "key") {
                    Some(Value::Label(l)) => l.resolve().as_str().to_string(),
                    _ => "?".into(),
                };
                self.warn(c, "citation written as a [@key] marker for the citation tool to resolve");
                self.para.push(Inline::text(format!("[@{key}]")));
            }
            "terms" => {
                if let Some(Value::Array(kids)) = field(c, "children") {
                    for k in kids.into_iter().filter_map(|v| v.cast::<Content>().ok()) {
                        let term = content_field(&k, "term").unwrap_or_default();
                        let desc = content_field(&k, "description").unwrap_or_default();
                        let mut inl = self.inlines_of(&term, styles, &Marks { bold: true, ..Default::default() });
                        inl.push(Inline::text(" "));
                        inl.extend(self.inlines_of(&desc, styles, &Marks::default()));
                        self.push_block(Block::Paragraph { inlines: inl, style: None, align: None });
                    }
                }
            }
            "quote" => {
                let body = content_field(c, "body").unwrap_or_default();
                let inlines = self.inlines_of(&body, styles, marks);
                self.push_block(Block::Paragraph { inlines, style: Some("Quote".into()), align: None });
            }
            "outline" => self.warn(c, "outline left out; insert a Word table of contents, which Word fills in"),
            "bibliography" => self.warn(c, "bibliography left out; the citation tool writes the reference list"),
            "place" | "hide" | "repeat" => self.warn(c, &format!("{name} has no Word equivalent and was left out")),
            _ => self.render(c, &format!("{name} has no native Word form; rendered by Typst as a picture")),
        }
    }

    fn figure(&mut self, c: &Content, styles: StyleChain) {
        let body = content_field(c, "body").unwrap_or_default();
        let blocks = self.blocks_of(&body, styles);
        let caption = match field(c, "caption") {
            Some(Value::Content(cap)) => {
                let b = content_field(&cap, "body").unwrap_or(cap);
                Some(self.inlines_of(&b, styles, &Marks::default()))
            }
            _ => None,
        };
        let kind = if blocks.iter().any(|b| matches!(b, Block::Table(_))) {
            FigureKind::Table
        } else if blocks.iter().all(|b| matches!(b, Block::Equation { .. })) && !blocks.is_empty() {
            FigureKind::Equation
        } else {
            FigureKind::Figure
        };
        let label = c.label().map(|l| l.resolve().as_str().to_string());
        self.push_block(Block::Figure(Figure { kind, body: blocks, caption, label }));
    }

    fn image(&mut self, c: &Content, styles: StyleChain) -> Option<Image> {
        let (name, data) = match field(c, "source")? {
            Value::Str(p) => {
                let path = p.to_string();
                let id = typst::syntax::FileId::new(typst::syntax::RootedPath::new(
                    typst::syntax::VirtualRoot::Project,
                    typst::syntax::VirtualPath::new(&path).ok()?,
                ));
                let bytes = self.world.file(id).ok()?;
                (path, bytes.to_vec())
            }
            Value::Bytes(b) => ("image.png".to_string(), b.to_vec()),
            _ => return None,
        };
        let lower = name.to_lowercase();
        if !(lower.ends_with(".png") || lower.ends_with(".jpg") || lower.ends_with(".jpeg") || lower.ends_with(".gif")) {
            return None;
        }
        let width = match field_in(c, "width", styles) {
            Some(Value::Length(l)) => (l.abs.to_pt() * 12700.0) as i64,
            Some(Value::Relative(r)) => (r.abs.abs.to_pt() * 12700.0) as i64,
            _ => 0,
        };
        let (pw, ph) = png_dims(&data).unwrap_or((4, 3));
        let height = if width > 0 { width * ph as i64 / pw.max(1) as i64 } else { 0 };
        let alt = match field(c, "alt") {
            Some(Value::Str(s)) => s.to_string(),
            _ => String::new(),
        };
        Some(Image { name: name.rsplit('/').next().unwrap_or(&name).to_string(), data, width, height, alt, fallback_png: None })
    }

    fn table(&mut self, c: &Content, styles: StyleChain) -> Table {
        let ncols = match field_in(c, "columns", styles) {
            Some(Value::Int(n)) => n.max(1) as usize,
            Some(Value::Array(a)) => a.len().max(1),
            _ => 1,
        };
        let widths: Vec<f32> = match field_in(c, "columns", styles) {
            Some(Value::Array(a)) => a
                .iter()
                .map(|v| match v {
                    Value::Fraction(f) => f.get() as f32,
                    Value::Length(l) => (l.abs.to_pt() / 72.0) as f32,
                    Value::Ratio(r) => r.get() as f32 * 6.0,
                    _ => 1.0,
                })
                .collect(),
            _ => vec![],
        };
        let table_fill = match field(c, "fill") {
            Some(Value::Color(col)) => Some(crate::value::color_hex(&col)),
            Some(Value::Func(_)) => {
                if self.next_table_style.is_none() {
                    self.warn(c, "a fill function is not carried over; use table.cell(fill: …) or a table style");
                }
                None
            }
            _ => None,
        };
        if matches!(field(c, "stroke"), Some(v) if !matches!(v, Value::None)) {
            self.warn(c, "table strokes are left to the Word table style");
        }
        let mut header_rows = 0;
        let mut cells: Vec<(Content, bool)> = vec![];
        if let Some(Value::Array(kids)) = field(c, "children") {
            for k in kids.into_iter().filter_map(|v| v.cast::<Content>().ok()) {
                match k.elem().name() {
                    "table.header" | "header" => {
                        if let Some(Value::Array(h)) = field(&k, "children") {
                            for x in h.into_iter().filter_map(|v| v.cast::<Content>().ok()) {
                                cells.push((x, true));
                            }
                        }
                    }
                    "table.hline" | "table.vline" | "hline" | "vline" => self.warn(&k, "table lines are left to the Word table style"),
                    "table.footer" | "footer" => {
                        if let Some(Value::Array(h)) = field(&k, "children") {
                            for x in h.into_iter().filter_map(|v| v.cast::<Content>().ok()) {
                                cells.push((x, false));
                            }
                        }
                    }
                    _ => cells.push((k, false)),
                }
            }
        }
        // Auto-placement as Typst does it: next free slot, row by row.
        let mut rows: Vec<Vec<Cell>> = vec![];
        let mut taken: std::collections::HashSet<(usize, usize)> = Default::default();
        let (mut r, mut col) = (0usize, 0usize);
        for (k, is_header) in cells {
            while taken.contains(&(r, col)) || col >= ncols {
                col += 1;
                if col >= ncols {
                    col = 0;
                    r += 1;
                }
            }
            let int = |n: &str| match field(&k, n) {
                Some(Value::Int(i)) => i.max(1) as usize,
                _ => 1,
            };
            let (colspan, rowspan, body) = if k.elem().name().ends_with("cell") {
                (int("colspan").min(ncols - col), int("rowspan"), content_field(&k, "body").unwrap_or_default())
            } else {
                (1, 1, k.clone())
            };
            if field(&k, "x").is_some() || field(&k, "y").is_some() {
                self.warn(&k, "explicit cell positions are placed in reading order");
            }
            let fill = match field(&k, "fill") {
                Some(Value::Color(col)) => Some(crate::value::color_hex(&col)),
                _ => table_fill.clone(),
            };
            let align = match field(&k, "align").map(|v| v.repr().to_string()).as_deref() {
                Some("right") | Some("end") => Some(Align::Right),
                Some("center") => Some(Align::Center),
                Some("left") | Some("start") => Some(Align::Left),
                _ => None,
            };
            for dr in 0..rowspan {
                for dc in 0..colspan {
                    taken.insert((r + dr, col + dc));
                }
            }
            while rows.len() <= r {
                rows.push(vec![]);
            }
            let blocks = self.blocks_of(&body, styles);
            rows[r].push(Cell { blocks, colspan, rowspan, fill, align });
            if is_header {
                header_rows = header_rows.max(r + rowspan);
            }
            col += colspan;
        }
        let style = self.next_table_style.take();
        Table { style, widths, header_rows, rows, align: vec![] }
    }

    /// Lays out the element with Typst and embeds it as a picture.
    fn render(&mut self, c: &Content, why: &str) {
        self.warn(c, why);
        let Some(src) = self.world.span_text(c.span()) else {
            self.warn(c, "element without source text could not be rendered and was left out");
            return;
        };
        let snippet = format!("#set page(width: auto, height: auto, margin: 2pt)\n{}{src}", if src.starts_with('#') { "" } else { "#" });
        match crate::world::render_png(&snippet, self.opts) {
            Ok((png, w_pt, h_pt)) => {
                let img = Image {
                    name: "typst.png".into(),
                    data: png,
                    width: (w_pt * 12700.0) as i64,
                    height: (h_pt * 12700.0) as i64,
                    alt: src.chars().take(200).collect(),
                    fallback_png: None,
                };
                self.push_block(Block::Image(img));
            }
            Err(e) => self.warn(c, &format!("rendering failed and the element was left out: {e}")),
        }
    }
}

fn png_dims(d: &[u8]) -> Option<(u32, u32)> {
    if d.len() < 24 || &d[1..4] != b"PNG" {
        return None;
    }
    let be = |i: usize| u32::from_be_bytes([d[i], d[i + 1], d[i + 2], d[i + 3]]);
    Some((be(16), be(20)))
}

fn trim(mut v: Vec<Inline>) -> Vec<Inline> {
    while matches!(v.last(), Some(Inline::Text { text, .. }) if text.trim().is_empty()) {
        v.pop();
    }
    while matches!(v.first(), Some(Inline::Text { text, .. }) if text.trim().is_empty()) {
        v.remove(0);
    }
    v
}

/// Joins neighbouring text with the same marks, as Typst splits text at
/// every space.
fn merge_text(v: Vec<Inline>) -> Vec<Inline> {
    let mut out: Vec<Inline> = vec![];
    for x in v {
        if let (Some(Inline::Text { text: a, marks: ma }), Inline::Text { text: b, marks: mb }) = (out.last_mut(), &x) {
            let space_only = b.trim().is_empty();
            if ma == mb || (space_only && !ma.underline && !ma.strike && ma.highlight.is_none()) {
                a.push_str(b);
                continue;
            }
        }
        out.push(x);
    }
    out
}
